//! The wire codec: SOM/envelope/data-segments/tail/EOM framing.
//!
//! Spec: `docs/vendor/genibelt-protocol.txt` §2 (message format) and §3
//! (protocol issues). Layout (§2.2–2.4):
//!
//! ```text
//! ENVELOPE (12 bytes)
//!   0  1  SOM              0xFD
//!   1  1  CLASS            unused
//!   2  1  SOURCE NODE
//!   3  1  DESTINATION NODE
//!   4  1  PRG NUMBER       progressive counter (§3.1: cycles 1..255,
//!                          0 only on the connect keep-alive; the
//!                          RECEIVER skips duplicate-PRG frames)
//!   5  3  SPARE
//!   8  2  LENGTH           data area size, bytes
//!  10  2  NUM DATA         number of data segments
//! DATA SEGMENT (× NUM DATA)
//!   +0 2  MSG ID
//!   +2 2  MSG LENGTH
//!   +4 n  MSG DATA
//! TAIL (4 bytes)
//!   +0 2  SPARE
//!   +2 1  CKS              unused (§2.4) — written 0, never verified
//!   +3 1  EOM              0xFE
//! ```
//!
//! A **keep-alive** is envelope + tail only (§2.5); it doubles as the
//! 10-second receive-timeout link probe (§3.2).
//!
//! Hardening (the adversarial round): `encode` returns `Result` and
//! refuses anything the u16 length fields cannot describe — the old
//! clamp silently produced a frame whose LENGTH lied, which permanently
//! desynchronizes the peer. `decode` refuses inconsistent frames
//! (declared lengths not matching the segment walk, a data area with
//! zero segments) instead of letting bytes vanish unexamined.

use crate::telegram::Endian;

pub const SOM: u8 = 0xFD;
pub const EOM: u8 = 0xFE;
pub const ENVELOPE_LEN: usize = 12;
pub const TAIL_LEN: usize = 4;
pub const SEGMENT_HEADER_LEN: usize = 4;

/// Structural ceiling on a frame — what the u16 fields can express, kept
/// well under it so a corrupt LENGTH cannot make a receiver wait
/// forever. The GENIBELT-specific §3.3 bounds (512-byte segments, 5 KB
/// frames) are enforced by the typed layer on top, not here: generic
/// channels may speak other vendors' framings.
pub const MAX_FRAME: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Envelope {
    pub class: u8,
    pub source: u8,
    pub destination: u8,
    pub prg: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataSegment {
    pub msg_id: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Telegram {
    pub envelope: Envelope,
    pub segments: Vec<DataSegment>,
}

impl Telegram {
    /// The idle heartbeat / link probe: envelope + tail, no data (§2.5).
    pub fn keepalive(envelope: Envelope) -> Telegram {
        Telegram {
            envelope,
            segments: Vec::new(),
        }
    }

    pub fn is_keepalive(&self) -> bool {
        self.segments.is_empty()
    }
}

fn put_u16(out: &mut Vec<u8>, v: u16, endian: Endian) {
    match endian {
        Endian::Big => out.extend_from_slice(&v.to_be_bytes()),
        Endian::Little => out.extend_from_slice(&v.to_le_bytes()),
    }
}

fn get_u16(buf: &[u8], at: usize, endian: Endian) -> u16 {
    let bytes = [buf[at], buf[at + 1]];
    match endian {
        Endian::Big => u16::from_be_bytes(bytes),
        Endian::Little => u16::from_le_bytes(bytes),
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// A segment's body exceeds what MSG LENGTH (u16) can say.
    #[error("segment (msg id {msg_id}) is {len} bytes — larger than the wire format can describe")]
    SegmentTooLarge { msg_id: u16, len: usize },
    /// The data area exceeds what LENGTH (u16) can say, or the whole
    /// frame exceeds [`MAX_FRAME`].
    #[error("frame would be {len} bytes — larger than the wire format allows")]
    FrameTooLarge { len: usize },
}

/// Serialize a telegram. LENGTH counts every data segment including its
/// own 4-byte header — the size of the whole data area.
///
/// Refuses (rather than clamps) oversize bodies: a frame whose declared
/// lengths lie about its real size desynchronizes the receiving
/// controller permanently, which is strictly worse than not sending.
pub fn encode(telegram: &Telegram, endian: Endian) -> Result<Vec<u8>, EncodeError> {
    let mut data_len: usize = 0;
    for segment in &telegram.segments {
        if segment.data.len() > u16::MAX as usize {
            return Err(EncodeError::SegmentTooLarge {
                msg_id: segment.msg_id,
                len: segment.data.len(),
            });
        }
        data_len += SEGMENT_HEADER_LEN + segment.data.len();
    }
    let total = ENVELOPE_LEN + data_len + TAIL_LEN;
    if data_len > u16::MAX as usize || total > MAX_FRAME {
        return Err(EncodeError::FrameTooLarge { len: total });
    }

    let mut out = Vec::with_capacity(total);
    out.push(SOM);
    out.push(telegram.envelope.class);
    out.push(telegram.envelope.source);
    out.push(telegram.envelope.destination);
    out.push(telegram.envelope.prg);
    out.extend_from_slice(&[0, 0, 0]); // SPARE
    put_u16(&mut out, data_len as u16, endian);
    put_u16(&mut out, telegram.segments.len() as u16, endian);
    for segment in &telegram.segments {
        put_u16(&mut out, segment.msg_id, endian);
        put_u16(&mut out, segment.data.len() as u16, endian);
        out.extend_from_slice(&segment.data);
    }
    out.extend_from_slice(&[0, 0]); // SPARE
    out.push(0); // CKS — "not used" (§2.4)
    out.push(EOM);
    Ok(out)
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Not enough bytes yet — TCP delivers a stream, and a frame split
    /// across packets is normal, not an error. Wait for more.
    #[error("an incomplete frame: waiting for more bytes")]
    NeedMore,
    /// The framing is wrong (no SOM, bad EOM, inconsistent lengths).
    /// `resync` says how many bytes to discard to reach the next
    /// plausible start. Callers COUNT these: a steadily climbing resync
    /// counter is the visible symptom of a wrong endian switch.
    #[error("not a telegram frame: skip {resync} byte(s) to the next start")]
    BadFrame { resync: usize },
}

/// Why a typed-in hex body did not parse.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    #[error("hex needs an even number of digits")]
    OddDigits,
    #[error("`{0}` is not a hex byte")]
    NotHex(String),
}

/// Decode the first telegram in `buf`, returning it and how many bytes
/// it consumed.
///
/// On a framing error the caller is told how far to skip: a stream that
/// lost sync (a controller restarting mid-frame) must be recoverable by
/// scanning to the next SOM, or the link stays dead until reconnected.
pub fn decode(buf: &[u8], endian: Endian) -> Result<(Telegram, usize), DecodeError> {
    if buf.is_empty() {
        return Err(DecodeError::NeedMore);
    }
    let bad = |from: usize| DecodeError::BadFrame {
        resync: next_som(&buf[from..])
            .map(|i| i + from)
            .unwrap_or(buf.len()),
    };
    if buf[0] != SOM {
        return Err(bad(1));
    }
    if buf.len() < ENVELOPE_LEN {
        return Err(DecodeError::NeedMore);
    }
    let data_len = get_u16(buf, 8, endian) as usize;
    let num_data = get_u16(buf, 10, endian) as usize;
    let total = ENVELOPE_LEN + data_len + TAIL_LEN;
    if total > MAX_FRAME {
        return Err(bad(1));
    }
    // A data area with no segments to describe it: bytes would vanish
    // unexamined. A real peer never sends it; refuse rather than skip.
    if num_data == 0 && data_len != 0 {
        return Err(bad(1));
    }
    if buf.len() < total {
        return Err(DecodeError::NeedMore);
    }
    if buf[total - 1] != EOM {
        return Err(bad(1));
    }

    let mut segments = Vec::with_capacity(num_data.min(64));
    let mut at = ENVELOPE_LEN;
    let data_end = ENVELOPE_LEN + data_len;
    for _ in 0..num_data {
        if at + SEGMENT_HEADER_LEN > data_end {
            return Err(bad(1));
        }
        let msg_id = get_u16(buf, at, endian);
        let msg_len = get_u16(buf, at + 2, endian) as usize;
        let start = at + SEGMENT_HEADER_LEN;
        if start + msg_len > data_end {
            return Err(bad(1));
        }
        segments.push(DataSegment {
            msg_id,
            data: buf[start..start + msg_len].to_vec(),
        });
        at = start + msg_len;
    }
    // The segment walk must land exactly on the declared end — slack in
    // the data area means the lengths disagree with each other.
    if at != data_end {
        return Err(bad(1));
    }

    Ok((
        Telegram {
            envelope: Envelope {
                class: buf[1],
                source: buf[2],
                destination: buf[3],
                prg: buf[4],
            },
            segments,
        },
        total,
    ))
}

fn next_som(buf: &[u8]) -> Option<usize> {
    buf.iter().position(|b| *b == SOM)
}

/// Parse whitespace-separated hex (`"00 1A FF"`, `"001AFF"`) into bytes —
/// how a binary message body is typed in a GUI.
///
/// Digits are counted as characters, so text that is not ASCII is refused by
/// the pair it sits in, never split inside a character.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, HexError> {
    let digits: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return Err(HexError::OddDigits);
    }
    let mut out = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let s: String = pair.iter().collect();
        // `from_str_radix` would take a leading `+`: only two hex digits are a byte.
        if !pair.iter().all(char::is_ascii_hexdigit) {
            return Err(HexError::NotHex(s));
        }
        out.push(u8::from_str_radix(&s, 16).map_err(|_| HexError::NotHex(s.clone()))?);
    }
    Ok(out)
}

/// A short, human-readable preview of a message body for monitors:
/// printable ASCII as text, anything else as hex.
pub fn preview(data: &[u8]) -> String {
    const MAX: usize = 48;
    let printable = data.iter().all(|b| (0x20..0x7F).contains(b) || *b == b'\t');
    if printable {
        let text = String::from_utf8_lossy(data).to_string();
        if text.chars().count() > MAX {
            format!("{}…", text.chars().take(MAX).collect::<String>())
        } else {
            text
        }
    } else {
        let hex: Vec<String> = data.iter().take(16).map(|b| format!("{b:02X}")).collect();
        let ellipsis = if data.len() > 16 { " …" } else { "" };
        format!("{}{}", hex.join(" "), ellipsis)
    }
}

/// The full two-column dump a debugging view renders: offset · hex ·
/// ASCII, 16 bytes per row — the "translatable in app" form the parcel
/// lifecycle store shows for stored raw frames.
pub fn hex_dump(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4);
    for (row, chunk) in data.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|b| {
                if (0x20..0x7F).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!(
            "{:04X}  {:<47}  {}\n",
            row * 16,
            hex.join(" "),
            ascii
        ));
    }
    out
}
