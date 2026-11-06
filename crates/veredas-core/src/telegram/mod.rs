//! The TCP telegram layer: a hardened socket, and the rules a device's telegrams are framed and
//! read by.
//!
//! Content-agnostic: no device, vendor or message catalog is named here, and no frame layout
//! is built in. A channel is configured by a plain-text, versioned channel file (as an OPC rule
//! maps tags and a log rule captures fields); this layer moves the bytes reliably and reads them
//! by those rules (`docs/questions.md` Q8, Q9; `docs/channel-schema.md`).
//!
//! - [`schema`]: the channel file: its framing, sequence, keep-alive and link rules, versioned,
//!   read and validated into a [`schema::ChannelSpec`].
//! - [`framing`]: the framing engine: frames encoded and decoded by a configured layout
//!   (delimiters, envelope, segment headers and tail as fields with roles), in any byte order.
//! - [`channel`]: one TCP link held correctly in either role, by the channel file's rules.
//! - [`capture`]: message bodies read into named fields by a [`capture::CaptureSpec`].
//! - [`compose`]: message bodies written from named fields by a [`compose::ComposeSpec`], the
//!   inverse of capture.
//! - [`catalog`]: the catalog file: a channel's messages by name, id and body rules, read and
//!   written by the two engines above.

pub mod capture;
pub mod catalog;
pub mod channel;
pub mod compose;
pub mod framing;
pub mod schema;

pub use catalog::{Catalog, CatalogError, Message};
pub use channel::{Channel, ChannelConfig, ChannelEvent, PeerId, Role};
pub use compose::{Compose, ComposeError, ComposeSpec};
pub use framing::{DecodeError, Frame, Framing, Segment};
pub use schema::{ChannelSpec, SchemaError};

use serde::{Deserialize, Serialize};

/// Byte order of a multi-byte field. Little-endian by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endian {
    Big,
    #[default]
    Little,
}

impl Endian {
    pub fn label(self) -> &'static str {
        match self {
            Endian::Big => "big-endian",
            Endian::Little => "little-endian",
        }
    }
}
