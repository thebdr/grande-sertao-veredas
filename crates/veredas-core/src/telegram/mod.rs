//! The TCP telegram layer: a hardened socket and the rules a device's telegrams are read by.
//!
//! Content-agnostic: no device, vendor or message catalog is named here. What a telegram means
//! is configuration (plain text, versioned, in a project folder, as an OPC rule maps tags and a
//! log rule captures fields); this layer moves the bytes reliably and reads them by rules.
//!
//! - [`codec`]: the envelope framing (start byte, envelope with sequence number, data area
//!   length and segment count, segments of id, length and body, tail, end byte), in either
//!   byte order.
//! - [`channel`]: one TCP link held correctly in either role: sequence stamping and duplicate
//!   skip, a keep-alive probe on silence, reconnection, bounded reads, a peer that stops
//!   reading dropped rather than wedging the link.
//! - [`capture`]: message bodies read into named fields by a [`capture::CaptureSpec`].
//!
//! The framing's shape is fixed in code today; making it configuration too is
//! `docs/questions.md` Q9.

pub mod capture;
pub mod channel;
pub mod codec;

pub use channel::{Channel, ChannelConfig, ChannelEvent, PeerId, Role};
pub use codec::{DataSegment, DecodeError, Envelope, Telegram};

use serde::{Deserialize, Serialize};

/// Byte order of the 16/32-bit wire fields. Little-endian by default.
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
