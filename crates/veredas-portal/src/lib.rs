//! veredas-portal: the gateway of Veredas 2.
//!
//! The one point where the suite exchanges data with the outside world (`docs/portal-plan.md`):
//! it reads the plant's logs, telegrams and OPC UA servers by configuration, stores every
//! protocol's data alike, and serves it, and its gated writes, to every other component.
//!
//! - [`encoding`]: text decoded from a plant's files, the encoding detected lazily.
//! - [`family`]: a rotating file family, resolved in rotation order from its names.
//! - [`timestamp`]: the stamp a line carries, located and parsed by a per-source rule.
//! - [`tail`]: one growing file, or a family through its rotations, followed with no line lost
//!   or duplicated; a start point in the past replayed and handed off live.
//! - [`source`]: the line-producing worker model: a merged, tagged stream of every source.
//! - [`capture`]: a line read into typed series by a regex rule.
//! - [`channel`]: a telegram channel to a device, by a channel file, a catalog and reply rules.
//!
//! Nothing here names a device, a vendor or a product: what a plant's files and telegrams mean
//! is configuration (`docs/questions.md` Q8). Reading a plant's file never blocks its writer.

pub mod capture;
pub mod channel;
pub mod encoding;
pub mod family;
pub mod source;
pub mod tail;
pub mod timestamp;

pub use capture::{CaptureError, CaptureOutcome, CaptureRule, named_groups};
pub use encoding::{DetectedEncoding, detect};
pub use family::{ActiveMode, FamilyFile, FamilySpec, OrderKey};
pub use source::{Disconnected, Emitter, LineEvent, Source, SourceSet, WorkerReport};
pub use tail::{
    FamilyStart, FamilyTailConfig, FamilyTailSource, FileStart, FileTailSource, TailConfig,
};
pub use timestamp::{TimestampExtractor, TimestampSpec};
pub use veredas_core::{
    Quality, Sample, SeriesRef, SeriesType, SourceError, SourceHandle, SourceId, Value,
    now_unix_ms, stop_flag,
};
