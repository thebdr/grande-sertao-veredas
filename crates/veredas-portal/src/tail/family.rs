//! `FamilyTailSource`: a rotating file family followed with drain-then-handoff, optionally
//! backfilled from a start point in the past: the retired files replayed from the located line
//! onward, then a handoff to live tailing at an exact byte offset, no gap and no duplicate at
//! the seam. Zero lines lost, zero duplicated, order preserved.
//!
//! Three invariants shape this file, each born of a burst-rotation test where generations turn
//! over faster than the rescan interval:
//!
//! 1. **A family born after attach is all new content**, including generations that already
//!    retired by the time the family is first seen. Only a family that existed at attach has
//!    history to skip (and with a backfill start, history means content before the start).
//! 2. **Never start reading an adopted active file while older unconsumed retirees exist.**
//!    Every handoff loops resolve then consume until clean before the new active's first line
//!    is emitted, so cross-file order survives arbitrarily fast rotation.
//! 3. **A catch-up sweep skips every file some live cursor reads, matched by identity, and the
//!    consumed state survives retries.** The file just opened can be renamed into a fresh
//!    retiree while the sweep runs; reading it there and through the handle would emit it twice.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use same_file::Handle;

use veredas_core::{SourceError, SourceHandle, SourceId};

use crate::encoding::DetectedEncoding;
use crate::family::{ActiveMode, FamilyFile, FamilySpec, OrderKey};
use crate::source::{Emitter, LineEvent, Source, stop_flag};
use crate::tail::FileStart;
use crate::tail::backfill::locate_start;
use crate::tail::cursor::{PathStatus, ReadStep, TailedFile};
use crate::tail::single::{ATTACH_WAIT, attached, spawn_watcher, wait_tick};
use crate::timestamp::TimestampSpec;

/// Where a family tail starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FamilyStart {
    /// Read the active file from its beginning, then follow. Pre-existing retirees are history
    /// and are not read.
    ActiveBegin,
    /// Follow from the active file's current end.
    ActiveEnd,
    /// Backfill: replay the family from the first line whose stamp is at or after this point
    /// (retired files included, in order), then continue live with no gap and no duplicate at
    /// the seam. Requires a timestamp rule on the source.
    Backfill(NaiveDateTime),
}

#[derive(Clone, Debug)]
pub struct FamilyTailConfig {
    pub poll_interval: Duration,
    /// How often the folder is rescanned (rotation detection in newest-by-order mode).
    pub rescan_interval: Duration,
    /// See [`crate::tail::TailConfig::drain_grace`]: must exceed the writer's worst flush gap
    /// during a rotation, or its final lines are at risk.
    pub drain_grace: Duration,
    pub use_notify: Option<bool>,
    pub start: FamilyStart,
}

impl Default for FamilyTailConfig {
    fn default() -> Self {
        FamilyTailConfig {
            poll_interval: Duration::from_millis(100),
            rescan_interval: Duration::from_millis(250),
            drain_grace: Duration::from_millis(250),
            use_notify: None,
            start: FamilyStart::ActiveEnd,
        }
    }
}

pub struct FamilyTailSource {
    id: SourceId,
    name: String,
    spec: FamilySpec,
    cfg: FamilyTailConfig,
    ts: Option<TimestampSpec>,
}

impl FamilyTailSource {
    pub fn new(id: impl AsRef<str>, display_name: impl Into<String>, spec: FamilySpec, cfg: FamilyTailConfig) -> Self {
        FamilyTailSource { id: SourceId::new(id), name: display_name.into(), spec, cfg, ts: None }
    }

    /// Attach a per-source line-stamp rule; a parse failure leaves `parsed_ts` empty and the
    /// line flows ordered by `seq`. Required for a `Backfill` start.
    pub fn with_timestamps(mut self, spec: TimestampSpec) -> Self {
        self.ts = Some(spec);
        self
    }
}

impl Source for FamilyTailSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn start(self: Box<Self>, tx: Sender<LineEvent>) -> Result<SourceHandle, SourceError> {
        let this = *self;
        if !this.spec.keyed() {
            return Err(SourceError::Spec(
                "the pattern has neither a `date` nor an `idx` group: a family is followed by its keys, and one without them has no rotation order to follow (a listing orders it by modification time, then name)".into(),
            ));
        }
        if matches!(this.cfg.start, FamilyStart::Backfill(_)) && this.ts.is_none() {
            return Err(SourceError::NotConfigured(
                "a Backfill start point requires a timestamp spec (with_timestamps)".into(),
            ));
        }
        let stop = stop_flag();
        let id = this.id.clone();
        let thread_stop = stop.clone();
        let ts_spec = this.ts.clone();
        let emitter = Emitter::with_timestamps(this.id.clone(), tx, this.ts);
        let (attached_tx, attached_rx) = channel::<()>();
        let join = std::thread::Builder::new()
            .name(format!("family:{id}"))
            .spawn(move || run_family(this.spec, this.cfg, ts_spec, emitter, thread_stop, attached_tx))
            .map_err(|e| SourceError::io(Path::new("<thread spawn>"), e))?;
        // The start returns once the worker has looked at its folder (listed it, or located its
        // backfill), so a line written after the start is never taken as history; bounded.
        let _ = attached_rx.recv_timeout(ATTACH_WAIT);
        Ok(SourceHandle::new(id, stop, join))
    }
}

/// How long a transient failure (an open, a resolve) is retried before the file or listing is
/// taken as truly gone.
const PATIENCE: Duration = Duration::from_millis(500);

/// Fully consume one retired or intermediate file: open from the start, drain to a stable end,
/// flush the final partial. A file that cannot be opened within `PATIENCE` is skipped: it
/// vanished mid-burst and nothing of it is readable.
fn read_full(path: &Path, grace: Duration, stop: &AtomicBool, emit: &mut dyn FnMut(String)) -> Result<(), SourceError> {
    read_range_full(path, 0, None, grace, stop, emit)
}

/// `read_full` from a byte offset with a known encoding: the backfill's replay primitive.
fn read_range_full(
    path: &Path,
    offset: u64,
    encoding: Option<DetectedEncoding>,
    grace: Duration,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(String),
) -> Result<(), SourceError> {
    let deadline = Instant::now() + PATIENCE;
    let mut cursor = loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match TailedFile::open_at(path, offset, encoding) {
            Ok(c) => break c,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return Ok(()),
        }
    };
    cursor.drain_stable(grace, stop, emit)?;
    cursor.flush_pending(emit);
    Ok(())
}

/// (key, name): the rotation-order rank. The modification time is deliberately left out: it only
/// breaks ties at the first resolve, never a live rotation decision.
fn rank(f: &FamilyFile) -> (OrderKey, &str) {
    (f.key, f.file_name.as_str())
}

fn member_names(files: &[FamilyFile]) -> HashSet<String> {
    files.iter().map(|f| f.file_name.clone()).collect()
}

/// Resolve with a little patience: a folder listing can fail transiently mid-rotation (over a
/// share above all). `None` means stopped, or out of patience.
fn retry_resolve(spec: &FamilySpec, stop: &AtomicBool) -> Option<Vec<FamilyFile>> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        match spec.resolve() {
            Ok(files) => return Some(files),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
}

/// Newest-by-order convergence: consume every member ranked above `consumed`, oldest first, until
/// the newest is opened as the new active. Each round either opens the newest or consumes it as
/// retired and resolves again: a rotation during the catch-up cannot outrun it. `None` means
/// stopped.
fn newest_catch_up(
    spec: &FamilySpec,
    cfg: &FamilyTailConfig,
    mut consumed: (OrderKey, String),
    stop: &AtomicBool,
    wake_rx: &Receiver<()>,
    emit: &mut dyn FnMut(String),
) -> Result<Option<(TailedFile, OrderKey, String)>, SourceError> {
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(files) = retry_resolve(spec, stop) else {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            wait_tick(wake_rx, cfg.poll_interval);
            continue;
        };
        let pending: Vec<FamilyFile> =
            files.iter().filter(|f| rank(f) > (consumed.0, consumed.1.as_str())).cloned().collect();
        if pending.is_empty() {
            // Nothing above the high-water mark right now; wait for the family to move (both
            // while attaching to a family being born and when everything newer vanished
            // mid-burst).
            wait_tick(wake_rx, cfg.poll_interval);
            continue;
        }
        for f in &pending[..pending.len() - 1] {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            read_full(&f.path, cfg.drain_grace, stop, emit)?;
        }
        let cand = &pending[pending.len() - 1];
        let deadline = Instant::now() + PATIENCE;
        let opened = loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match TailedFile::open(&cand.path, FileStart::Begin) {
                Ok(c) => break Some(c),
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break None,
            }
        };
        match opened {
            Some(c) => return Ok(Some((c, cand.key, cand.file_name.clone()))),
            None => {
                // Rotated away before it could be opened: consume what remains and resolve
                // everything above it again.
                read_full(&cand.path, cfg.drain_grace, stop, emit)?;
                consumed = (cand.key, cand.file_name.clone());
            }
        }
    }
}

/// One fixed-mode sweep: read every retiree not yet in `known`, oldest first. A file whose
/// identity matches a handle in `skip` is marked known without being read: a live cursor has
/// already streamed it (the drained old active) or is about to (the freshly opened new active,
/// which can be renamed into a retiree while this very sweep runs; invariant 3). Returns whether
/// anything fresh was found; loop until it reports false. `None` means stopped.
fn sweep_unknown_retirees(
    spec: &FamilySpec,
    known: &mut HashSet<String>,
    skip: &[&Handle],
    grace: Duration,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(String),
) -> Result<Option<bool>, SourceError> {
    let Some(files) = retry_resolve(spec, stop) else {
        return Ok(if stop.load(Ordering::Relaxed) { None } else { Some(false) });
    };
    let fresh: Vec<FamilyFile> = files.iter().filter(|f| !f.is_active && !known.contains(&f.file_name)).cloned().collect();
    if fresh.is_empty() {
        return Ok(Some(false));
    }
    for f in &fresh {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let cursor_owned = match Handle::from_path(&f.path) {
            Ok(h) => skip.iter().any(|identity| h == **identity),
            Err(_) => false,
        };
        if !cursor_owned {
            read_full(&f.path, grace, stop, emit)?;
        }
        known.insert(f.file_name.clone());
    }
    Ok(Some(true))
}

/// Adopt whatever sits at the fixed path as the new active, consuming any retirees that formed
/// meanwhile before its first line is read (invariant 2). The same identity reappearing means a
/// transient glitch: the old cursor is handed back untouched. `None` means stopped.
#[allow(clippy::too_many_arguments)]
fn fixed_adopt(
    spec: &FamilySpec,
    fixed_name: &str,
    cfg: &FamilyTailConfig,
    old: TailedFile,
    known: &mut HashSet<String>,
    stop: &AtomicBool,
    wake_rx: &Receiver<()>,
    emit: &mut dyn FnMut(String),
) -> Result<Option<TailedFile>, SourceError> {
    let fixed_path = spec.dir().join(fixed_name);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match Handle::from_path(&fixed_path) {
            Ok(h) if h == *old.identity() => return Ok(Some(old)),
            Ok(_) => match TailedFile::open(&fixed_path, FileStart::Begin) {
                Ok(new_cursor) => {
                    // The final pre-read sweep: anything that retired between the earlier sweeps
                    // and this open is older than the new active's content and goes first. Both
                    // live cursors' identities are skipped (invariant 3).
                    loop {
                        match sweep_unknown_retirees(
                            spec,
                            known,
                            &[old.identity(), new_cursor.identity()],
                            cfg.drain_grace,
                            stop,
                            emit,
                        )? {
                            None => return Ok(None),
                            Some(true) => continue,
                            Some(false) => break,
                        }
                    }
                    return Ok(Some(new_cursor));
                }
                Err(_) => wait_tick(wake_rx, cfg.poll_interval),
            },
            Err(_) => wait_tick(wake_rx, cfg.poll_interval),
        }
    }
}

/// The outcome of a backfill attach.
enum BackfillAttach {
    Attached { cursor: Box<TailedFile>, key: OrderKey, name: String, known: HashSet<String> },
    /// No family member on disk: fall through to a normal (born-mode) live attach; everything
    /// appearing later is newer than a past start.
    FamilyEmpty,
    Stopped,
}

/// Replay the family from the located start point, then return a cursor positioned for a
/// seamless live continuation. The planned active may rotate while earlier files replay; both
/// modes converge on the true active without losing the seek offset owed to the planned file's
/// remains.
#[allow(clippy::too_many_arguments)] // the attach has this many inputs; a struct would name them once more
fn attach_backfill(
    spec: &FamilySpec,
    cfg: &FamilyTailConfig,
    ts: Option<&TimestampSpec>,
    start_at: NaiveDateTime,
    stop: &AtomicBool,
    wake_rx: &Receiver<()>,
    emit: &mut dyn FnMut(String),
    attached_slot: &mut Option<Sender<()>>,
) -> Result<BackfillAttach, SourceError> {
    let Some(ts) = ts else {
        return Err(SourceError::NotConfigured("a Backfill start point requires a timestamp spec".into()));
    };
    // Located (or found empty), the folder has been looked at: the start may return while the
    // replay runs; the planned active is opened at its offset, so nothing written meanwhile is
    // missed.
    let located = locate_start(spec, ts, start_at);
    attached(attached_slot);
    let Some(plan) = located? else {
        return Ok(BackfillAttach::FamilyEmpty);
    };

    // Everything older than the plan is pre-start history; a rotation during the backfill must
    // not "discover" it (see `BackfillPlan::skipped`).
    let mut consumed: HashSet<String> = plan.skipped.iter().cloned().collect();

    // Replay everything before the planned active, in order, at full speed.
    let n = plan.files.len();
    for (i, f) in plan.files[..n - 1].iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            return Ok(BackfillAttach::Stopped);
        }
        let (off, enc) = if i == 0 { (plan.first_offset, plan.first_encoding) } else { (0, None) };
        read_range_full(&f.path, off, enc, cfg.drain_grace, stop, emit)?;
        consumed.insert(f.name.clone());
    }

    let planned = &plan.files[n - 1];
    // The seek information still owed to the oldest unread part of the planned active (its
    // remains inherit it when it rotates before it is opened).
    let mut pending: Option<(u64, Option<DetectedEncoding>)> =
        Some(if n == 1 { (plan.first_offset, plan.first_encoding) } else { (0, None) });

    match spec.active_mode() {
        ActiveMode::NewestByOrder => {
            // Names are stable in this mode: opening the planned active by name never hits the
            // wrong file. A rotation after this open is the live loop's ordinary business.
            let (off, enc) = pending.take().unwrap_or((0, None));
            let deadline = Instant::now() + PATIENCE;
            let opened = loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(BackfillAttach::Stopped);
                }
                match TailedFile::open_at(&planned.path, off, enc) {
                    Ok(c) => break Some(c),
                    Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break None,
                }
            };
            match opened {
                Some(cursor) => {
                    Ok(BackfillAttach::Attached { cursor: Box::new(cursor), key: planned.key, name: planned.name.clone(), known: consumed })
                }
                None => {
                    // Vanished before it was reached: read any remnant, then converge on the
                    // newest by rank.
                    read_range_full(&planned.path, off, enc, cfg.drain_grace, stop, emit)?;
                    match newest_catch_up(spec, cfg, (planned.key, planned.name.clone()), stop, wake_rx, emit)? {
                        None => Ok(BackfillAttach::Stopped),
                        Some((c, k, nm)) => Ok(BackfillAttach::Attached { cursor: Box::new(c), key: k, name: nm, known: consumed }),
                    }
                }
            }
        }
        ActiveMode::Fixed(fixed_name) => {
            let fixed_path = spec.dir().join(fixed_name);
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(BackfillAttach::Stopped);
                }
                let Some(files) = retry_resolve(spec, stop) else {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(BackfillAttach::Stopped);
                    }
                    wait_tick(wake_rx, cfg.poll_interval);
                    continue;
                };
                let fresh: Vec<FamilyFile> =
                    files.iter().filter(|f| !f.is_active && !consumed.contains(&f.file_name)).cloned().collect();
                if !fresh.is_empty() {
                    // The planned active rotated during the backfill: its remains are the oldest
                    // unconsumed retiree and inherit the seek offset; younger ones are read whole.
                    for f in &fresh {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(BackfillAttach::Stopped);
                        }
                        let (off, enc) = pending.take().unwrap_or((0, None));
                        read_range_full(&f.path, off, enc, cfg.drain_grace, stop, emit)?;
                        consumed.insert(f.file_name.clone());
                    }
                    continue;
                }
                // Clean: adopt the fixed path (the offset applies only when the planned active
                // never rotated).
                let (off, enc) = pending.unwrap_or((0, None));
                match TailedFile::open_at(&fixed_path, off, enc) {
                    Ok(cursor) => {
                        // Invariant 2: check that nothing retired between the clean sweep and
                        // this open. Nothing was read from the cursor yet, so dropping it is free.
                        let clean = match retry_resolve(spec, stop) {
                            Some(fs) => fs.iter().all(|f| f.is_active || consumed.contains(&f.file_name)),
                            None => {
                                if stop.load(Ordering::Relaxed) {
                                    return Ok(BackfillAttach::Stopped);
                                }
                                true // the listing is out of patience: proceed
                            }
                        };
                        if clean {
                            return Ok(BackfillAttach::Attached {
                                cursor: Box::new(cursor),
                                key: OrderKey::NONE,
                                name: fixed_name.clone(),
                                known: consumed,
                            });
                        }
                        // Fell behind: drop the unread cursor; the loop sweeps again (the offset
                        // still owed to the remains).
                    }
                    Err(_) => wait_tick(wake_rx, cfg.poll_interval),
                }
            }
        }
    }
}

fn run_family(
    spec: FamilySpec,
    cfg: FamilyTailConfig,
    ts: Option<TimestampSpec>,
    mut emitter: Emitter,
    stop: Arc<AtomicBool>,
    attached_tx: Sender<()>,
) -> Result<(), SourceError> {
    let mut attached_slot = Some(attached_tx);
    let (wake_tx, wake_rx) = channel::<()>();
    // Automatic mode arms the watcher for local paths alone: directory notifications are
    // unreliable over a share, where polling stays the one mechanism.
    let notify_wanted =
        cfg.use_notify.unwrap_or_else(|| crate::tail::pathkind::classify(spec.dir()).notify_useful());
    // The loop's own sender keeps the wake channel connected when no watcher could be set up
    // (the folder not there yet): a disconnected channel would return the wait at once and
    // spin the loop.
    let _wake_keep = wake_tx.clone();
    let _watcher = if notify_wanted { spawn_watcher(spec.dir(), wake_tx, spec.recursive()) } else { None };

    let disconnected = Cell::new(false);
    let mut emit = |line: String| {
        if emitter.emit(line).is_err() {
            disconnected.set(true);
        }
    };

    // ---- attach ------------------------------------------------------------------------------
    // Decided at the very first resolve: a family with members on disk has history to skip; an
    // empty or unreachable folder means everything that ever appears is new content
    // (invariant 1).
    //
    // `known` lives outside the attach loop on purpose: an open that lands in the rename-create
    // gap retries the loop, and the sweep's progress must survive the retry or already-emitted
    // generations replay (invariant 3).
    let mut known: HashSet<String> = HashSet::new();
    let mut preexisted: Option<bool> = None;

    // A backfill start replays history from the located point first; the cursor it returns IS
    // the seam: the same handle, the exact offset, live from here.
    let mut backfilled: Option<(TailedFile, OrderKey, String)> = None;
    if let FamilyStart::Backfill(start_at) = cfg.start {
        match attach_backfill(&spec, &cfg, ts.as_ref(), start_at, &stop, &wake_rx, &mut emit, &mut attached_slot)? {
            BackfillAttach::Stopped => return Ok(()),
            BackfillAttach::FamilyEmpty => preexisted = Some(false),
            BackfillAttach::Attached { cursor, key, name, known: consumed } => {
                known = consumed;
                backfilled = Some((*cursor, key, name));
            }
        }
    }

    let attach = match backfilled {
        Some(seam) => seam,
        None => loop {
            if stop.load(Ordering::Relaxed) || disconnected.get() {
                return Ok(());
            }
            let files = spec.resolve().ok();
            if preexisted.is_none() {
                preexisted = Some(files.as_ref().map(|f| !f.is_empty()).unwrap_or(false));
            }
            let born = preexisted == Some(false);
            let Some(files) = files else {
                // Looked at, not there: the start may return; whatever appears is new content.
                attached(&mut attached_slot);
                wait_tick(&wake_rx, cfg.poll_interval);
                continue;
            };

            if born {
                // Born: nothing to attach to, everything that appears is new content.
                attached(&mut attached_slot);
                match spec.active_mode() {
                    ActiveMode::NewestByOrder => {
                        if files.is_empty() {
                            wait_tick(&wake_rx, cfg.poll_interval);
                            continue;
                        }
                        // The generic convergence reads every generation from the family's first
                        // file onward and opens the newest: no birth-window generation slips past.
                        let sentinel = (OrderKey::NONE, String::new());
                        match newest_catch_up(&spec, &cfg, sentinel, &stop, &wake_rx, &mut emit)? {
                            None => return Ok(()),
                            Some((c, k, n)) => break (c, k, n),
                        }
                    }
                    ActiveMode::Fixed(fixed_name) => {
                        // Replay the birth-window retirees in order...
                        loop {
                            match sweep_unknown_retirees(&spec, &mut known, &[], cfg.drain_grace, &stop, &mut emit)? {
                                None => return Ok(()),
                                Some(true) => continue,
                                Some(false) => break,
                            }
                        }
                        // ...then follow the active once it exists.
                        match TailedFile::open(&spec.dir().join(fixed_name), FileStart::Begin) {
                            Ok(cursor) => {
                                // A pre-read sweep, for the same reason as `fixed_adopt`: the
                                // cursor just opened may already be renamed into a retiree
                                // (invariant 3).
                                loop {
                                    match sweep_unknown_retirees(
                                        &spec,
                                        &mut known,
                                        &[cursor.identity()],
                                        cfg.drain_grace,
                                        &stop,
                                        &mut emit,
                                    )? {
                                        None => return Ok(()),
                                        Some(true) => continue,
                                        Some(false) => break,
                                    }
                                }
                                break (cursor, OrderKey::NONE, fixed_name.clone());
                            }
                            Err(_) => {
                                wait_tick(&wake_rx, cfg.poll_interval);
                                continue;
                            }
                        }
                    }
                }
            } else {
                // The family predates the session: retirees are history (a backfill's domain),
                // only the active file is followed, honouring the configured start.
                // A family on which no key parses has no rotation order to follow: refused, like
                // a keyless pattern, rather than followed by name. A fixed-name active carries no
                // key by design and does not count.
                let fixed = matches!(spec.active_mode(), ActiveMode::Fixed(_));
                let mut keyed = files.iter().filter(|f| !(fixed && f.is_active)).peekable();
                if keyed.peek().is_some() && keyed.all(|f| f.key == OrderKey::NONE) {
                    return Err(SourceError::Spec(
                        "the pattern's key groups parse on no member of the family: a family is followed by its keys, and these name none (check the date format against the names)".into(),
                    ));
                }
                let Some(active) = files.iter().find(|f| f.is_active) else {
                    // Listed, with no active to attach to yet (a fixed name not born): the start
                    // may return. The retirees listed are history, and from here the family is
                    // followed as one born after the start, so a generation that retires before
                    // the active is opened is read, never taken for history.
                    attached(&mut attached_slot);
                    known = member_names(&files);
                    preexisted = Some(false);
                    wait_tick(&wake_rx, cfg.poll_interval);
                    continue;
                };
                let start = match cfg.start {
                    FamilyStart::ActiveEnd => FileStart::End,
                    // A backfill reaches here only through the family-empty fallthrough: whatever
                    // appears now is newer than the (past) start point, so read it whole.
                    FamilyStart::ActiveBegin | FamilyStart::Backfill(_) => FileStart::Begin,
                };
                match TailedFile::open(&active.path, start) {
                    Ok(c) => {
                        known = member_names(&files);
                        break (c, active.key, active.file_name.clone());
                    }
                    Err(_) => {
                        // Rotated between the resolve and the open; the next round realigns.
                        wait_tick(&wake_rx, cfg.poll_interval);
                        continue;
                    }
                }
            }
        },
    };
    // Attached (the active open at its end or its beginning): the start may return.
    attached(&mut attached_slot);
    let (mut cursor, mut current_key, mut current_name) = attach;

    // ---- live loop ---------------------------------------------------------------------------
    let mut next_rescan = Instant::now();
    loop {
        if disconnected.get() {
            return Ok(());
        }
        if stop.load(Ordering::Relaxed) {
            let _ = cursor.read_new(&mut emit);
            cursor.flush_pending(&mut emit);
            return Ok(());
        }

        match cursor.read_new(&mut emit)? {
            ReadStep::Truncated => {
                // An in-place truncation of the active file: the degenerate single-file case.
                cursor.handle_truncation(&mut emit)?;
                continue;
            }
            ReadStep::Progress => {}
        }

        match spec.active_mode() {
            ActiveMode::NewestByOrder => {
                // The active renamed or deleted underneath is a rotation smell in this mode too:
                // rescan at once.
                if cursor.check_path() != PathStatus::Same {
                    next_rescan = Instant::now();
                }
                if Instant::now() >= next_rescan {
                    next_rescan = Instant::now() + cfg.rescan_interval;
                    let newer_exists = match spec.resolve() {
                        Ok(files) => files.iter().any(|f| rank(f) > (current_key, current_name.as_str())),
                        Err(_) => false,
                    };
                    if newer_exists {
                        // Drain the retiring file dry; once its length holds still, the held
                        // partial is final.
                        cursor.drain_stable(cfg.drain_grace, &stop, &mut emit)?;
                        cursor.flush_pending(&mut emit);
                        let consumed = (current_key, current_name.clone());
                        match newest_catch_up(&spec, &cfg, consumed, &stop, &wake_rx, &mut emit)? {
                            None => return Ok(()),
                            Some((c, k, n)) => {
                                cursor = c;
                                current_key = k;
                                current_name = n;
                            }
                        }
                        // Another generation may already be waiting.
                        next_rescan = Instant::now();
                        continue;
                    }
                }
            }
            ActiveMode::Fixed(fixed_name) => match cursor.check_path() {
                PathStatus::Same => {
                    if Instant::now() >= next_rescan {
                        next_rescan = Instant::now() + cfg.rescan_interval;
                        // Members appearing without the fixed path changing were dropped in from
                        // outside (not a rotation); a live tail does not replay them. The path is
                        // checked again after the listing: a burst of rotations between the check
                        // above and the listing would show its generations as dropped in, and they
                        // would be taken as known without ever being read.
                        if let Ok(files) = spec.resolve()
                            && cursor.check_path() == PathStatus::Same
                        {
                            known = member_names(&files);
                        }
                    }
                }
                PathStatus::Replaced | PathStatus::Missing => {
                    // The handle still reads the renamed-away file: drain it dry, then the
                    // partial is final. (Emitting it now keeps it ordered before any missed
                    // generation; the cost is a split line in the rare false positive of a
                    // transient share glitch.)
                    cursor.drain_stable(cfg.drain_grace, &stop, &mut emit)?;
                    cursor.flush_pending(&mut emit);

                    // Consume the generations that retired while nobody looked, the cursor's own
                    // recognised by identity, not position (a glitched earlier listing must not
                    // desync this).
                    loop {
                        match sweep_unknown_retirees(
                            &spec,
                            &mut known,
                            &[cursor.identity()],
                            cfg.drain_grace,
                            &stop,
                            &mut emit,
                        )? {
                            None => return Ok(()),
                            Some(true) => continue,
                            Some(false) => break,
                        }
                    }

                    match fixed_adopt(&spec, fixed_name, &cfg, cursor, &mut known, &stop, &wake_rx, &mut emit)? {
                        None => return Ok(()),
                        Some(c) => cursor = c,
                    }
                    continue;
                }
            },
        }

        wait_tick(&wake_rx, cfg.poll_interval);
    }
}
