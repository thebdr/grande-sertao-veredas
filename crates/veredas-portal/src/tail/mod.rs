//! File tailing: one growing file, and a rotating file family followed with drain-then-handoff.
//!
//! Polling is the primary mechanism: it is the only reliable one over a network share, and a
//! local path gets a filesystem watcher on top, so an append shows faster than the poll
//! interval. Every handle on a plant's file is a plain `File::open` (share-all on Windows) and
//! never a memory map, so reading never blocks the owner from writing, rotating or truncating.

mod backfill;
mod cursor;
mod family;
mod pathkind;
mod single;

#[doc(hidden)]
pub use backfill::{line_table, plan_for};
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
