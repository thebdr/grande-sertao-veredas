//! veredas-core: the foundation of Veredas 2.
//!
//! Every type and protocol layer more than one component needs, as a library that depends on
//! no other member, so the suite speaks one wire and one value model (`docs/core-plan.md`).
//!
//! - [`types`]: the value model, samples on their two clocks, source and series identity,
//!   quality.
//! - [`source`]: the stop/join handle every worker thread returns, and its error.
//! - [`telegram`]: the TCP telegram layer (the envelope framing, a hardened channel, a capture
//!   rules engine), content-agnostic.
//! - [`fieldbus`]: the fieldbus masters, the functions that drive a plant's devices over a
//!   standard fieldbus: a Modbus/TCP client first.
//! - [`rng`]: a deterministic PCG32, for weighted draws and seeded fixtures.
//! - [`opc`]: the OPC UA client layer (poll, gated write, browse), and the demo plant its tests
//!   drive (feature `demo-server`).
//! - [`state`]: a controller's state as structured data, and its one line of text with one
//!   writer and one reader, the table of reason numbers and words beside it.
//! - [`schema`]: the one header reader every plain-text file of the suite opens with.
//!
//! Features: `net`, on by default, holds `types`, `source`, `telegram` and `opc` and what they
//! stand on; without it (`default-features = false`) core is `schema`, `rng` and `state`, on
//! serde, toml and thiserror alone. `demo-server` adds the OPC UA demo plant, `modbus` and `enip`
//! the fieldbus masters; none of the three is on by default.
//!
//! Nothing here names its first caller: each type says what it means on the wire or in the
//! plant, as if a second consumer already existed. Nothing here names a device or a vendor
//! either: what a device's telegrams mean is configuration, not core code (the plan's decision
//! of 2026-11-05).

pub mod fieldbus;
#[cfg(feature = "net")]
pub mod opc;
pub mod rng;
pub mod schema;
#[cfg(feature = "net")]
pub mod source;
pub mod state;
#[cfg(feature = "net")]
pub mod telegram;
#[cfg(feature = "net")]
pub mod types;

#[cfg(feature = "net")]
pub use source::{SourceError, SourceHandle, stop_flag};
#[cfg(feature = "net")]
pub use types::{Quality, Sample, SeriesRef, SeriesType, SourceId, Value, now_unix_ms};
