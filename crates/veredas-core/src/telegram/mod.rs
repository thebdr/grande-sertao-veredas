//! The TCP telegram layer: the sorting chain's wire, spoken by one copy of the code.
//!
//! Whoever plays the External Unit (portal, facing a real sort controller) and whoever plays
//! the Master Sort Control (maquinar's simulated controller) speak through these modules, so
//! the codec, the typed catalog and the decision engine cannot drift apart between them.
//!
//! - [`codec`]: the SOM/envelope/segments/tail/EOM framing.
//! - [`genibelt`]: every typed GENIBELT message, decoded and encoded.
//! - [`channel`]: one TCP link spoken correctly (PRG, duplicate skip, keep-alive probe,
//!   reconnect, a peer that stops reading dropped rather than wedging the link).
//! - [`capture`]: binary fields of a message body read into named values.
//! - [`sortplan`]: how a parcel becomes a destination.
//! - [`decision_client`]: asking a decision service where a parcel goes, telling it what
//!   happened.
//!
//! The vendor specification is machine-local at `docs/vendor/genibelt-protocol.txt`
//! (gitignored, proprietary; `docs/questions.md` Q6); code comments cite its section numbers.
//! The binding facts: little-endian ("INTEL format", §3), the MSC is always the TCP client
//! (§3), PRG cycles 1..255 with receiver-side duplicate skip (§3.1), a 10 s receive-timeout
//! keep-alive probe (§3.2), data segments of at most 512 bytes and frames of at most 5 KB
//! (§3.3).

pub mod capture;
pub mod channel;
pub mod codec;
pub mod decision_client;
pub mod genibelt;
pub mod sortplan;

pub use channel::{Channel, ChannelConfig, ChannelEvent, PeerId, Role};
pub use codec::{DataSegment, DecodeError, Envelope, Telegram};
pub use genibelt::Message;
pub use sortplan::{Decision, Parcel, SortPlan};

use serde::{Deserialize, Serialize};

/// Byte order of the 16/32-bit wire fields.
///
/// GENIBELT fixes it to little-endian (§3, "INTEL data format"), the default. The switch
/// survives for generic channels speaking other vendors' framings.
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
