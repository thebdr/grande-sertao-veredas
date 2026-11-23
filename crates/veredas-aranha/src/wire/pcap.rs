//! The capture file: classic pcap, read and written, and a capture replayed behind the seam.
//!
//! Read as tshark 4.6.6 reads it (measured, `docs/aranha-plan.md`, "Traps"): a 24-byte file header
//! (the magic number says the byte order, the stamps' resolution and the record header's length;
//! major version 2 or later; the link type Ethernet), then a record header per frame (seconds, the
//! fraction, the bytes kept and the length on the wire, in that order from version 2.4; before
//! 2.3 the two lengths stand the other way round, and at 2.3 they are taken so that the bytes kept
//! are the smaller) and its bytes. The nanosecond magic counts in nanoseconds from version 2.4 on,
//! in microseconds before it; AIX's format (version 2.2, the microsecond magic, link type 6) is
//! Ethernet stamped in nanoseconds; the modified format's records carry eight bytes more in their
//! header, passed over. The link type is Ethernet when its low 16 bits are 1 and its reserved bits
//! (16 to 25) are clear; the bits above (a check sequence's flag and length) are kept, and the
//! frames kept as recorded. The snapshot length is not held against the records, a fraction past
//! a second is carried into the next, a length on the wire under the bytes kept is kept as stated,
//! and a capture whose last record is cut short (one still being written) gives the frames before
//! it with the damage named. pcapng is not read.
//!
//! Where it departs from tshark, on purpose: a microsecond fraction past 4 294 967 is carried
//! whole (tshark's product wraps at 32 bits), and a length on the wire past 2 147 483 647 is kept
//! as stated (tshark shows 2 147 483 647).
//!
//! Written: nanosecond stamps, little-endian, version 2.4, link type Ethernet, snapshot length
//! `SNAPLEN`, each frame's bytes and its length on the wire as it holds them.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Frame, Wire, WireError, sendable};
use crate::profinet::mac::Mac;

/// The magic number of a file stamped in microseconds, as written in its own byte order.
pub const MAGIC_MICROS: u32 = 0xA1B2_C3D4;
/// The magic number of a file stamped in nanoseconds (from version 2.4).
pub const MAGIC_NANOS: u32 = 0xA1B2_3C4D;
/// The magic number of the modified format: microseconds, records with eight bytes more of header.
pub const MAGIC_MODIFIED: u32 = 0xA1B2_CD34;
/// pcapng's first block, which this reader names when it refuses one.
const PCAPNG: u32 = 0x0A0D_0D0A;
/// Link type 1: Ethernet.
pub const LINKTYPE_ETHERNET: u32 = 1;
/// The link type's reserved bits, which must be clear.
pub const LINKTYPE_RESERVED: u32 = 0x03FF_0000;
/// The most bytes a record holds, read or written (Wireshark's own bound for Ethernet).
pub const SNAPLEN: u32 = 262_144;
/// The file header's length, and a record header's (the modified format's is `RECORD_HEADER + 8`).
pub const FILE_HEADER: usize = 24;
pub const RECORD_HEADER: usize = 16;

/// A capture read whole: its header's values, its frames in order, and the damage that ended it
/// early, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub big_endian: bool,
    /// The magic number, as read in the file's byte order.
    pub magic: u32,
    /// Whether the stamps were read in nanoseconds (by the magic, the version and the AIX format).
    pub nanos: bool,
    pub version: (u16, u16),
    pub snaplen: u32,
    /// The link type field whole: Ethernet in its low 16 bits; the bits above the reserved ones
    /// (a check sequence's flag and length) are kept, and the frames kept as recorded.
    pub linktype: u32,
    pub frames: Vec<Frame>,
    pub damage: Option<Damage>,
}

/// Why reading stopped before the end of the file: the record (counted from 0) and the byte it
/// starts at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Damage {
    pub record: usize,
    pub offset: usize,
    pub kind: DamageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DamageKind {
    /// The file ends inside a record header of `need` bytes.
    HeaderCut { have: usize, need: usize },
    /// The file ends inside a record's bytes.
    DataCut { declared: u32, have: usize },
    /// A record says it keeps more than `SNAPLEN` bytes.
    TooLong { declared: u32 },
}

impl std::fmt::Display for Damage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (r, o) = (self.record, self.offset);
        match &self.kind {
            DamageKind::HeaderCut { have, need } => write!(f, "record {r} at byte {o} is cut short in its header: {have} of {need} bytes"),
            DamageKind::DataCut { declared, have } => write!(f, "record {r} at byte {o} is cut short: {have} of its {declared} bytes"),
            DamageKind::TooLong { declared } => write!(f, "record {r} at byte {o} says it keeps {declared} bytes, past the {SNAPLEN} a record holds"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PcapError {
    #[error("a capture file begins with a {FILE_HEADER}-byte header; this one has {0} bytes")]
    TooShort(usize),
    #[error("this is pcapng, which is not read: write it as classic pcap (`editcap -F pcap`)")]
    Pcapng,
    #[error("magic number {0:#010x} is not classic pcap's (a1b2c3d4, a1b23c4d or a1b2cd34, in either byte order)")]
    Magic(u32),
    #[error("pcap version {0}.{1} is not read: major version 2 or later")]
    Version(u16, u16),
    #[error("link type {0:#x} is not Ethernet (1)")]
    NotEthernet(u32),
    #[error("link type {0:#x} has reserved bits set (bits 16 to 25)")]
    Reserved(u32),
    #[error("a frame stamped {0} ns is past what a capture's 32-bit seconds count (2106)")]
    Stamp(u64),
    #[error("a frame of {0} bytes is past the {SNAPLEN} a record holds")]
    FrameTooLong(usize),
    #[error("a frame's length on the wire, {0}, is past what a record states (32 bits)")]
    LengthTooLong(usize),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
}

/// Read a capture from its bytes, each frame said heard on `iface`.
pub fn read(bytes: &[u8], iface: &str) -> Result<Capture, PcapError> {
    if bytes.len() < FILE_HEADER {
        return Err(PcapError::TooShort(bytes.len()));
    }
    let head = [bytes[0], bytes[1], bytes[2], bytes[3]];
    let known = |m: u32| [MAGIC_MICROS, MAGIC_NANOS, MAGIC_MODIFIED].contains(&m);
    let (big_endian, magic) = match (u32::from_le_bytes(head), u32::from_be_bytes(head)) {
        (m, _) if known(m) => (false, m),
        (_, m) if known(m) => (true, m),
        (PCAPNG, _) => return Err(PcapError::Pcapng),
        (_, m) => return Err(PcapError::Magic(m)),
    };
    let u16_at = |at: usize| {
        let b = [bytes[at], bytes[at + 1]];
        if big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let u32_at = |at: usize| {
        let b = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    };
    let version = (u16_at(4), u16_at(6));
    if version.0 < 2 {
        return Err(PcapError::Version(version.0, version.1));
    }
    let snaplen = u32_at(16);
    let linktype = u32_at(20);
    let aix = version == (2, 2) && magic == MAGIC_MICROS && linktype == 6;
    if !aix {
        if linktype & LINKTYPE_RESERVED != 0 {
            return Err(PcapError::Reserved(linktype));
        }
        if linktype & 0xFFFF != LINKTYPE_ETHERNET {
            return Err(PcapError::NotEthernet(linktype));
        }
    }
    let nanos = aix || (magic == MAGIC_NANOS && version >= (2, 4));
    let per_fraction: u64 = if nanos { 1 } else { 1000 };
    let header = if magic == MAGIC_MODIFIED { RECORD_HEADER + 8 } else { RECORD_HEADER };
    let mut frames = Vec::new();
    let mut damage = None;
    let mut at = FILE_HEADER;
    while at < bytes.len() {
        let record = frames.len();
        let have = bytes.len() - at;
        if have < header {
            damage = Some(Damage { record, offset: at, kind: DamageKind::HeaderCut { have, need: header } });
            break;
        }
        let (secs, fraction, first, second) = (u32_at(at), u32_at(at + 4), u32_at(at + 8), u32_at(at + 12));
        // Before 2.3 the length on the wire comes first; at 2.3 either way, the smaller kept.
        let swapped = version < (2, 3) || (version == (2, 3) && first > second);
        let (kept, length) = if swapped { (second, first) } else { (first, second) };
        if kept > SNAPLEN {
            damage = Some(Damage { record, offset: at, kind: DamageKind::TooLong { declared: kept } });
            break;
        }
        let body = at + header;
        if bytes.len() - body < kept as usize {
            damage = Some(Damage { record, offset: at, kind: DamageKind::DataCut { declared: kept, have: bytes.len() - body } });
            break;
        }
        frames.push(Frame {
            at_ns: u64::from(secs) * 1_000_000_000 + u64::from(fraction) * per_fraction,
            iface: iface.to_string(),
            bytes: bytes[body..body + kept as usize].to_vec(),
            length: length as usize,
        });
        at = body + kept as usize;
    }
    Ok(Capture { big_endian, magic, nanos, version, snaplen, linktype, frames, damage })
}

/// Read a capture file, each frame said heard on `iface`.
pub fn read_file(path: &Path, iface: &str) -> Result<Capture, PcapError> {
    let bytes = std::fs::read(path).map_err(|source| PcapError::Io { path: path.to_path_buf(), source })?;
    read(&bytes, iface)
}

/// Write frames as a capture: nanosecond stamps, little-endian, version 2.4, Ethernet.
pub fn write(frames: &[Frame]) -> Result<Vec<u8>, PcapError> {
    let mut out = Vec::with_capacity(FILE_HEADER + frames.iter().map(|f| RECORD_HEADER + f.bytes.len()).sum::<usize>());
    out.extend_from_slice(&MAGIC_NANOS.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    // The zone and the accuracy, always 0.
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&SNAPLEN.to_le_bytes());
    out.extend_from_slice(&LINKTYPE_ETHERNET.to_le_bytes());
    for f in frames {
        let secs = u32::try_from(f.at_ns / 1_000_000_000).map_err(|_| PcapError::Stamp(f.at_ns))?;
        let kept = u32::try_from(f.bytes.len()).ok().filter(|k| *k <= SNAPLEN).ok_or(PcapError::FrameTooLong(f.bytes.len()))?;
        let length = u32::try_from(f.length).map_err(|_| PcapError::LengthTooLong(f.length))?;
        out.extend_from_slice(&secs.to_le_bytes());
        out.extend_from_slice(&((f.at_ns % 1_000_000_000) as u32).to_le_bytes());
        out.extend_from_slice(&kept.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&f.bytes);
    }
    Ok(out)
}

/// Write frames as a capture file.
pub fn write_file(path: &Path, frames: &[Frame]) -> Result<(), PcapError> {
    let bytes = write(frames)?;
    std::fs::write(path, bytes).map_err(|source| PcapError::Io { path: path.to_path_buf(), source })
}

/// A capture replayed behind the seam: its frames in order, at once whatever the timeout, then
/// `WireError::Ended`. What is sent is kept, not put anywhere, so a test sees what a step would
/// have sent.
#[derive(Debug)]
pub struct Replay {
    name: String,
    frames: VecDeque<Frame>,
    sent: Vec<Vec<u8>>,
}

impl Replay {
    pub fn new(name: impl Into<String>, frames: Vec<Frame>) -> Replay {
        Replay { name: name.into(), frames: frames.into(), sent: Vec::new() }
    }

    /// The frames not yet handed back.
    pub fn remaining(&self) -> usize {
        self.frames.len()
    }

    /// What was sent, in order.
    pub fn sent(&self) -> &[Vec<u8>] {
        &self.sent
    }
}

impl Wire for Replay {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac(&self) -> Option<Mac> {
        None
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        sendable(frame)?;
        self.sent.push(frame.to_vec());
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Option<Frame>, WireError> {
        self.frames.pop_front().map(Some).ok_or_else(|| WireError::Ended(self.name.clone()))
    }
}
