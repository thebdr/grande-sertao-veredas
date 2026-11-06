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
//! - [`rng`]: a deterministic PCG32, for weighted draws and seeded fixtures.
//! - [`opc`]: the OPC UA client layer (poll, gated write, browse), and the demo plant its tests
//!   drive (feature `demo-server`).
//!
//! Nothing here names its first caller: each type says what it means on the wire or in the
//! plant, as if a second consumer already existed. Nothing here names a device or a vendor
//! either: what a device's telegrams mean is configuration, not core code (the plan's decision
//! of 2026-11-05).

pub mod opc;
pub mod rng;
pub mod source;
pub mod telegram;
pub mod types;

pub use source::{SourceError, SourceHandle, stop_flag};
pub use types::{Quality, Sample, SeriesRef, SeriesType, SourceId, Value, now_unix_ms};
