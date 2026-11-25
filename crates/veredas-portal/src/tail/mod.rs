//! File tailing: one growing file, and a rotating file family followed with drain-then-handoff.
//!
//! Polling is the primary mechanism: it is the only reliable one over a network share, and a
//! local path gets a filesystem watcher on top, so an append shows faster than the poll
//! interval. Every handle on a plant's file is a plain `File::open` (share-all on Windows) and
//! never a memory map, so reading never blocks the owner from writing, rotating or truncating.

mod backfill;
mod cursor;
// The tail's read size, so the grep decodes a file in the same slices (C-094).
pub(crate) use cursor::READ_CHUNK;
mod family;
mod pathkind;
mod single;

/// A log's start read in (docs/questions.md Q319): the number of lines its worker had sent once
/// it reached the end of every file as it stood when the start opened it, `u64::MAX` before. A
/// session holds one per log (`with_history_read`) and says the log's history read once that
/// many of its lines are in the store. Set once: the first moment counts.
pub(crate) fn mark_history_read(at: &Option<std::sync::Arc<std::sync::atomic::AtomicU64>>, sent: u64) {
    if let Some(at) = at {
        let _ = at.compare_exchange(u64::MAX, sent, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The mark of a log whose start's history cannot be whole (a member there and left unread): no
/// count of lines reaches it, so the session never says the history read. Set once, as a count.
pub(crate) const HISTORY_LOST: u64 = u64::MAX - 1;

pub(crate) fn mark_history_lost(at: &Option<std::sync::Arc<std::sync::atomic::AtomicU64>>) {
    mark_history_read(at, HISTORY_LOST);
}

#[doc(hidden)]
pub use backfill::{line_table, plan_for, plan_for_with};
pub use family::{FamilyStart, FamilyTailConfig, FamilyTailSource};
pub use pathkind::{PathKind, classify};
pub use single::FileTailSource;

use std::time::Duration;

/// Where a newly opened tail begins reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStart {
    /// Read the file from its first byte.
    Begin,
    /// Start at the current end: only lines appended after the open are emitted. If the file
    /// ends mid-line, the fragment's completion is skipped too: a tail never emits the torn half
    /// of a line it did not see start.
    End,
}

/// The knobs of one tailed file.
#[derive(Clone, Debug)]
pub struct TailConfig {
    /// How often to poll for growth or rotation when no watcher event arrives.
    pub poll_interval: Duration,
    /// How long a retiring or replaced file's end must hold still before it counts as drained.
    /// Must exceed the writer's worst flush gap during a rotation, or its final lines are at
    /// risk.
    pub drain_grace: Duration,
    /// `Some(true/false)` forces the watcher on or off; `None` is automatic (on for local paths,
    /// off for a share, where polling is the only mechanism).
    pub use_notify: Option<bool>,
    /// The start position for the first open. A reopen after a replacement always reads the
    /// new file from its beginning.
    pub start: FileStart,
}

impl Default for TailConfig {
    fn default() -> Self {
        TailConfig {
            poll_interval: Duration::from_millis(100),
            drain_grace: Duration::from_millis(250),
            use_notify: None,
            start: FileStart::End,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// The first moment counts: a mark after it, the lines sent since, is new content and moves
    /// nothing (C-096, "set once").
    #[test]
    fn the_history_read_is_set_once() {
        let at = Some(Arc::new(AtomicU64::new(u64::MAX)));
        super::mark_history_read(&at, 5);
        super::mark_history_read(&at, 9);
        super::mark_history_read(&at, 0);
        assert_eq!(at.as_ref().unwrap().load(Ordering::Relaxed), 5);
        super::mark_history_read(&None, 3);
    }
}
