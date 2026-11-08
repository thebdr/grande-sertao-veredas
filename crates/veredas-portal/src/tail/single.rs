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
}

impl FileTailSource {
    pub fn new(id: impl AsRef<str>, display_name: impl Into<String>, path: impl Into<PathBuf>, cfg: TailConfig) -> Self {
        FileTailSource { id: SourceId::new(id), name: display_name.into(), path: path.into(), cfg, ts: None }
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
            .spawn(move || run(this.path, this.cfg, emitter, thread_stop, attached_tx))
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
    matches!(e, SourceError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
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

fn run(path: PathBuf, cfg: TailConfig, mut emitter: Emitter, stop: Arc<AtomicBool>, attached_tx: Sender<()>) -> Result<(), SourceError> {
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
    let mut emit = |line: String| {
        if emitter.emit(line).is_err() {
            disconnected.set(true);
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
    // Attributes are readable through a hold that refuses every read; an identity that is not read matches nothing.
    let mut held: Option<(u64, Option<crate::tail::cursor::Identity>)> = None;
    match TailedFile::open(&path, cfg.start) {
        Ok(tf) => {
            active = Some(tf);
            first_open = false;
        }
        Err(e) if is_not_found(&e) => first_open = false,
        Err(_) => held = std::fs::metadata(&path).ok().map(|m| (m.len(), crate::tail::cursor::identity_at(&path))),
    }
    attached(&mut Some(attached_tx));
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
                match opened {
                    Ok(tf) => {
                        first_open = false;
                        active = Some(tf);
                        continue; // read at once, without burning a tick
                    }
                    // Absent at attach: whatever appears later is new content. Any other failure
                    // (held, a passing error) keeps the configured start for the open that succeeds.
                    Err(e) if is_not_found(&e) => first_open = false,
                    Err(_) => {}
                }
            }
            Some(tf) => {
                match tf.read_new(&mut emit)? {
                    ReadStep::Truncated => {
                        tf.handle_truncation(&mut emit)?;
                        continue; // re-read the fresh content now
                    }
                    ReadStep::Progress => {}
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
