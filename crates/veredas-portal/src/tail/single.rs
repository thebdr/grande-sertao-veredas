//! `FileTailSource`: one growing file at a path followed `tail -F` style, surviving truncation,
//! replacement, and the file not existing yet. Poll-driven, with a filesystem watcher on top
//! for local paths.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use same_file::Handle;

use veredas_core::{SourceError, SourceHandle, SourceId};

use crate::source::{Emitter, LineEvent, Source, stop_flag};
use crate::tail::cursor::{PathStatus, ReadStep, TailedFile};
use crate::tail::{FileStart, TailConfig};
use crate::timestamp::TimestampSpec;

pub struct FileTailSource {
    id: SourceId,
    name: String,
    path: PathBuf,
    cfg: TailConfig,
    ts: Option<TimestampSpec>,
    history: Option<Arc<std::sync::atomic::AtomicU64>>,
}

impl FileTailSource {
    pub fn new(id: impl AsRef<str>, display_name: impl Into<String>, path: impl Into<PathBuf>, cfg: TailConfig) -> Self {
        FileTailSource { id: SourceId::new(id), name: display_name.into(), path: path.into(), cfg, ts: None, history: None }
    }

    /// Where the worker says its start's history read: the lines it had sent once it read the
    /// file to its end as it stood when the start opened it (at once for a start at its end, or
    /// a file not there), `u64::MAX` until then (docs/questions.md Q319).
    pub fn with_history_read(mut self, at: Arc<std::sync::atomic::AtomicU64>) -> Self {
        self.history = Some(at);
        self
    }

    /// Attach a per-source line-stamp rule; a parse failure leaves `parsed_ts` empty and the
    /// line flows ordered by `seq`.
    pub fn with_timestamps(mut self, spec: TimestampSpec) -> Self {
        self.ts = Some(spec);
        self
    }
}

impl Source for FileTailSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn start(self: Box<Self>, tx: Sender<LineEvent>) -> Result<SourceHandle, SourceError> {
        let this = *self;
        let stop = stop_flag();
        let id = this.id.clone();
        let thread_stop = stop.clone();
        let emitter = Emitter::with_timestamps(this.id.clone(), tx, this.ts);
        let (attached_tx, attached_rx) = channel::<()>();
        let join = std::thread::Builder::new()
            .name(format!("tail:{id}"))
            .spawn(move || run(this.path, this.cfg, emitter, thread_stop, attached_tx, this.history))
            .map_err(|e| SourceError::io(Path::new("<thread spawn>"), e))?;
        // The start returns once the worker has looked at its path: a file present is attached
        // where the start says, an absent one awaited, so a line written after the start is
        // never taken as history. Bounded: a path that cannot be looked at (a share away) is the
        // worker's business, not the caller's.
        let _ = attached_rx.recv_timeout(ATTACH_WAIT);
        Ok(SourceHandle::new(id, stop, join))
    }
}

/// How long a start waits for its worker to look at the path before returning anyway.
pub(crate) const ATTACH_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether an open failed because nothing is at the path (as against a file there that cannot
/// be opened for the moment).
pub(crate) fn is_not_found(e: &SourceError) -> bool {
    use std::io::ErrorKind::{InvalidFilename, NotADirectory, NotFound};
    // A folder on the path that is a file, or a name no file may have: nothing can be there.
    matches!(e, SourceError::Io { source, .. } if matches!(source.kind(), NotFound | NotADirectory | InvalidFilename))
}

/// Whether the file a look recorded the identity of is still in `path`'s folder, under another
/// name: renamed away, not deleted. An identity the look could not read, or a folder that cannot
/// be listed, cannot tell, and counts as moved (an unread history is never said read).
fn moved_within_folder(path: &Path, identity: Option<&crate::tail::cursor::Identity>) -> bool {
    let Some(id) = identity else { return true };
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().any(|e| crate::tail::cursor::identity_at(&e.path()).as_ref() == Some(id)),
        Err(_) => true,
    }
}

/// Tell the start that the worker has looked at its path, once.
pub(crate) fn attached(slot: &mut Option<Sender<()>>) {
    if let Some(tx) = slot.take() {
        let _ = tx.send(());
    }
}

/// Watch the file's parent folder; any event wakes the poll loop early. Failing to set the
/// watcher up is not an error: polling covers it.
pub(crate) fn spawn_watcher(dir: &Path, wake: Sender<()>, recursive: bool) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |_res| {
        let _ = wake.send(());
    })
    .ok()?;
    let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    watcher.watch(dir, mode).ok()?;
    Some(watcher)
}

/// Sleep until the next poll tick or a filesystem event, coalescing bursts.
pub(crate) fn wait_tick(wake: &Receiver<()>, interval: std::time::Duration) {
    if wake.recv_timeout(interval).is_ok() {
        while wake.try_recv().is_ok() {}
    }
}

fn run(path: PathBuf, cfg: TailConfig, mut emitter: Emitter, stop: Arc<AtomicBool>, attached_tx: Sender<()>, history: Option<Arc<std::sync::atomic::AtomicU64>>) -> Result<(), SourceError> {
    let (wake_tx, wake_rx) = channel::<()>();
    // Automatic mode arms the watcher for local paths alone: directory notifications are
    // unreliable over a share, where polling stays the one mechanism.
    let notify_wanted = cfg.use_notify.unwrap_or_else(|| crate::tail::pathkind::classify(&path).notify_useful());
    // The loop keeps a sender of its own: with none alive the wake channel reads as
    // disconnected and the wait returns at once, every tick, and the loop would spin where
    // the watcher could not be set up (a parent folder not there yet, a path that cannot be
    // watched).
    let _wake_keep = wake_tx.clone();

    let disconnected = Cell::new(false);
    let sent = Cell::new(0u64);
    let mut emit = |line: String| {
        if emitter.emit(line).is_err() {
            disconnected.set(true);
        } else {
            sent.set(sent.get() + 1);
        }
    };

    // The first look, before the loop and before the start returns: present, the file is
    // attached where the start says; absent, whatever appears later is new content; present but
    // not openable for the moment (a writer holding it, a share's passing error), awaited at the
    // configured start, never read whole as if new. The watcher is armed after the signal, so a
    // slow arm never eats the bound the caller waits on; the poll covers the gap.
    let mut active: Option<TailedFile> = None;
    let mut first_open = true;
    // A file present but held at the first look: its length and identity then, so the open that
    // succeeds can tell the same file (read from where it stood) from a replacement (read whole).
    // An identity that is not read matches nothing; a length that is not read is zero.
    let mut held: Option<(u64, Option<crate::tail::cursor::Identity>)> = None;
    match TailedFile::open(&path, cfg.start) {
        Ok(tf) => {
            active = Some(tf);
            first_open = false;
        }
        Err(e) if is_not_found(&e) => first_open = false,
        Err(_) => {
            // A look that reads nothing at all (the folder denying its listing, a share's passing
            // error) records a length of zero and no identity: whatever opens is read whole.
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            held = Some((len, crate::tail::cursor::identity_at(&path)));
        }
    }
    attached(&mut Some(attached_tx));
    // A start at the end skips the history, and a file not there at the first look has none:
    // read in at once (a file held at the first look and started at its end is read from where
    // it stood once it opens, new content). Read from its start, present, its history is read
    // with its first read; held, with the first read of the open that succeeds.
    if cfg.start == FileStart::End || (active.is_none() && held.is_none()) {
        crate::tail::mark_history_read(&history, 0);
    }
    // Read from its start, the file's history ends at its length when the start looked at it:
    // the read-in must reach it, or the file was cut in place (a copy-and-truncate rotation) and
    // the history is lost.
    // A held file's bytes cannot be read at the look: its length alone is kept.
    let mut history_to: Option<crate::tail::cursor::Reach> = None;
    if cfg.start == FileStart::Begin {
        if let Some(tf) = active.as_mut() {
            history_to = Some(tf.reach_now()?);
        } else if let Some((len, _)) = &held {
            history_to = Some(crate::tail::cursor::Reach { len: *len, head: Vec::new(), tail: Vec::new() });
        }
    }
    let _watcher = if notify_wanted { path.parent().and_then(|dir| spawn_watcher(dir, wake_tx, false)) } else { None };

    loop {
        if disconnected.get() {
            return Ok(()); // the consumer is gone: a clean end
        }
        if stop.load(Ordering::Relaxed) {
            // A final sweep, so a stop right after a write loses nothing.
            if let Some(tf) = active.as_mut() {
                let _ = tf.read_new(&mut emit);
                tf.flush_pending(&mut emit);
            }
            return Ok(());
        }

        match active.as_mut() {
            None => {
                let opened = match (first_open, cfg.start, &held) {
                    (true, FileStart::End, Some((len, identity))) => TailedFile::open_held(&path, *len, identity.as_ref()),
                    (true, start, _) => TailedFile::open(&path, start),
                    (false, _, _) => TailedFile::open(&path, FileStart::Begin),
                };
                // A file held at the first look and read from its start holds the start's history.
                let history_held = first_open && cfg.start == FileStart::Begin;
                match opened {
                    Ok(tf) => {
                        // The same file is read whole now; another one at the path (the held one
                        // renamed away, which this tail does not follow) leaves that history
                        // unread for good, as does one whose identity the look could not read.
                        if history_held
                            && let Some((_, identity)) = &held
                            && !identity.as_ref().is_some_and(|id| tf.has_identity(id))
                        {
                            crate::tail::mark_history_lost(&history);
                        }
                        first_open = false;
                        active = Some(tf);
                        continue; // read at once, without burning a tick
                    }
                    // Absent at attach: whatever appears later is new content. Any other failure
                    // (held, a passing error) keeps the configured start for the open that succeeds.
                    Err(e) if is_not_found(&e) => {
                        // Gone while it held the start's history: deleted, nothing of it is left
                        // to read; still in its folder under another name, unread for good.
                        if history_held && let Some((_, identity)) = &held {
                            if moved_within_folder(&path, identity.as_ref()) {
                                crate::tail::mark_history_lost(&history);
                            } else {
                                crate::tail::mark_history_read(&history, sent.get());
                            }
                        }
                        first_open = false;
                    }
                    Err(_) => {}
                }
            }
            Some(tf) => {
                #[cfg(test)]
                if history_to.is_some() {
                    crate::tail::family::before_open(&path);
                }
                match tf.read_new(&mut emit)? {
                    ReadStep::Truncated => {
                        if history_to.take().is_some() {
                            crate::tail::mark_history_lost(&history);
                        }
                        tf.handle_truncation(&mut emit)?;
                        continue; // re-read the fresh content now
                    }
                    ReadStep::Progress => {}
                }
                // Read to its end as it stood: the start's history sent, if the read reached the
                // length it had when the start looked; short of it, cut in place, lost.
                match history_to.take() {
                    Some(to) if !tf.reached(&to) => crate::tail::mark_history_lost(&history),
                    _ => crate::tail::mark_history_read(&history, sent.get()),
                }

                match tf.check_path() {
                    PathStatus::Same => {}
                    PathStatus::Replaced | PathStatus::Missing => {
                        // Whatever happens next, first take every byte still reachable through
                        // the old handle.
                        tf.drain_stable(cfg.drain_grace, &stop, &mut emit)?;
                        match Handle::from_path(&path) {
                            Ok(h) if h == *tf.identity() => {
                                // A transient glitch: the same file is back. Keep the held
                                // partial line; its completion may still arrive.
                            }
                            _ => {
                                // Truly replaced or gone: the old file never grows again, so the
                                // held partial is a final (torn) line. Then start over on
                                // whatever appears at the path.
                                tf.flush_pending(&mut emit);
                                active = None;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        wait_tick(&wake_rx, cfg.poll_interval);
    }
}
