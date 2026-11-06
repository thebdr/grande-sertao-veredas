//! The OPC UA client layer: polling plant tags, writing the few a configuration allows, browsing
//! the address space.
//!
//! A plant exposes OPC UA servers. A worker per configured source reads a batch of tags on an
//! interval and pushes them into a channel, the same worker-plus-channel shape every source has
//! ([`crate::source`]), so a consumer takes plant tags as it takes any other reading. The worker
//! also carries the write path (a command tag pulsed by an operator or by another process),
//! gated by a master switch and a per-tag whitelist, and a separate connection serves the
//! address-space browser.
//!
//! - [`spec`]: the compiled shape of a source (endpoint, identity, poll interval, rules, tags).
//! - [`value`]: an OPC `Variant` into a typed [`crate::Value`], and bits carved out of words.
//! - [`map`]: a poll into one row per rule, and the line an operator reads.
//! - [`browse`]: the address-space browser.
//! - [`worker`]: the poll worker, its reconnection and its gated writes.
//! - [`error`]: why a call to a server produced nothing, and why a node id did not parse.
//! - [`demo`]: an in-process OPC UA server shaped like a small plant, the fixture every test
//!   drives (feature `demo-server`, test-fixture plumbing; core's own tests switch it on).

#[cfg(feature = "demo-server")]
pub mod demo;

pub mod browse;
pub mod error;
pub mod map;
pub mod spec;
pub mod value;
pub mod worker;

pub use browse::{BrowseEvent, BrowsedNode, Browser};
pub use error::{Call, NodeIdError, OpcError};
pub use map::{DedupState, HEARTBEAT_POLLS, Mapped, RuleRow, map_poll, map_poll_dedup, render_line};
pub use spec::{Auth, BitSpec, PollSpec, RuleSpec, TagSpec, parse_node_id};
pub use value::{ValueError, bit_field, opc_to_value};
pub use worker::{
    ConnState, OpcEvent, OpcHandle, OpcWorker, TagRead, WriteError, WriteGate, retry_delays, run_on_own_runtime,
    spawn, tag_reads,
};

/// One character of a one-line rendering: a control character, or a Unicode line or paragraph
/// separator, reads as a space.
pub(crate) fn flat(c: char) -> char {
    if c.is_control() || c == '\u{2028}' || c == '\u{2029}' { ' ' } else { c }
}

/// Re-exported so callers can build values and node ids without depending on
/// `async-opcua-types` directly: the crate boundary stays one import wide.
pub use opcua_types::{NodeId, StatusCode, Variant};
