//! The framing engine: frames encoded and decoded by a layout that is configuration.
//!
//! Core holds no frame layout of its own (`docs/questions.md` Q9). A [`Framing`], compiled from
//! a channel configuration ([`crate::telegram::schema`]), says where a frame starts and ends
//! (optional delimiter bytes), what its envelope, segment headers and tail hold (fields by
//! size, byte order and role) and how large a frame may be. The engine reads and writes any
//! such layout:
//!
//! - an envelope field's **role** is `sequence` (stamped by the channel), `length` (the size of
//!   the data area: every segment, headers included), `count` (the number of segments), or
//!   `opaque` (carried by name, its value from the frame or the field's configured default);
//! - a segment header field's role is `length` (its body's size) or `opaque`;
//! - a tail field is `opaque`.
//!
//! A frame with no segments is a keep-alive. Encoding refuses a value its field cannot hold,
//! never clamping; decoding returns the frame and the bytes it used, asks for more only while
//! the delimiter, envelope and whole segment headers that have arrived could still be a valid
//! frame, and refuses a frame whose lengths disagree as soon as they show it, saying how far
//! to skip to the next start delimiter.

use std::collections::BTreeMap;

use crate::telegram::Endian;

/// What a field means to the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldRole {
    /// Carried by name, not interpreted.
    #[default]
    Opaque,
    /// The frame's sequence number (envelope only), stamped by the channel.
    Sequence,
    /// The size of what follows: the data area (envelope) or the body (segment header).
    Length,
    /// The number of segments (envelope only).
    Count,
}

/// One field of a layout, its offset computed from the fields before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// 1 to 8 bytes.
    pub size: usize,
    pub role: FieldRole,
    pub order: Endian,
    /// The value an opaque field takes when the frame does not name it.
    pub value: u64,
}

impl Field {
    fn max(&self) -> u64 {
        if self.size >= 8 { u64::MAX } else { (1u64 << (8 * self.size)) - 1 }
    }

    fn read(&self, bytes: &[u8]) -> u64 {
        let mut v = 0u64;
        match self.order {
            Endian::Big => bytes.iter().for_each(|b| v = (v << 8) | u64::from(*b)),
            Endian::Little => bytes.iter().rev().for_each(|b| v = (v << 8) | u64::from(*b)),
        }
        v
    }

    fn write(&self, v: u64, out: &mut Vec<u8>) {
        let le = v.to_le_bytes();
        let bytes = &le[..self.size];
        match self.order {
            Endian::Little => out.extend_from_slice(bytes),
            Endian::Big => out.extend(bytes.iter().rev()),
        }
    }
}

/// A compiled layout. Build one through [`crate::telegram::schema::load`], which validates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Framing {
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub envelope: Vec<Field>,
    /// The segment header; `None` means the data area is one body, and a non-empty body is the
    /// frame's one segment.
    pub segment: Option<Vec<Field>>,
    pub tail: Vec<Field>,
    /// The largest frame, delimiters included, encoded or accepted.
    pub max_frame: usize,
}

/// A decoded or to-be-encoded frame: its envelope and tail fields by name, and its segments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub fields: BTreeMap<String, u64>,
    pub segments: Vec<Segment>,
}

/// One segment: its header fields by name, and its body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub fields: BTreeMap<String, u64>,
    pub body: Vec<u8>,
}

impl Frame {
    /// A frame with no segments: the idle heartbeat and link probe.
    pub fn is_keepalive(&self) -> bool {
        self.segments.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// A value does not fit its field: refused, never clamped (a frame whose lengths lie
    /// desynchronises the peer for good).
    #[error("{field} = {value} does not fit its {size}-byte field")]
    DoesNotFit { field: String, value: u64, size: usize },
    #[error("frame would be {len} bytes — larger than the {max}-byte limit")]
    FrameTooLarge { len: usize, max: usize },
    /// A layout without segment headers carries one body at most.
    #[error("{0} segments for a layout that carries one body")]
    TooManySegments(usize),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Not enough bytes yet: a frame split across packets is normal. Wait for more.
    #[error("an incomplete frame: waiting for more bytes")]
    NeedMore,
    /// The framing is wrong; `resync` bytes are to be skipped, to the next start delimiter (or
    /// one byte, for a layout without one). Callers count these: a climbing count is the
    /// visible symptom of a wrong byte order.
    #[error("not a frame: skip {resync} byte(s) to the next start")]
    BadFrame { resync: usize },
}

fn width(fields: &[Field]) -> usize {
    fields.iter().map(|f| f.size).sum()
}

fn role(fields: &[Field], role: FieldRole) -> Option<&Field> {
    fields.iter().find(|f| f.role == role)
}

impl Framing {
    /// The envelope field stamped with the sequence number, if the layout has one.
    pub fn sequence_field(&self) -> Option<&Field> {
        role(&self.envelope, FieldRole::Sequence)
    }

    /// The sequence number a decoded frame carries.
    pub fn sequence_of(&self, frame: &Frame) -> Option<u64> {
        self.sequence_field().and_then(|f| frame.fields.get(&f.name).copied())
    }

    /// Serialize a frame, stamping `sequence` into the sequence field when given (else the
    /// frame's own value, else 0) and computing every length and count.
    pub fn encode(&self, frame: &Frame, sequence: Option<u64>) -> Result<Vec<u8>, EncodeError> {
        let mut data = Vec::new();
        match &self.segment {
            Some(header) => {
                for seg in &frame.segments {
                    for f in header {
                        let v = match f.role {
                            FieldRole::Length => seg.body.len() as u64,
                            _ => seg.fields.get(&f.name).copied().unwrap_or(f.value),
                        };
                        put(f, v, &mut data)?;
                    }
                    data.extend_from_slice(&seg.body);
                }
            }
            None => match frame.segments.as_slice() {
                [] => {}
                [one] => data.extend_from_slice(&one.body),
                many => return Err(EncodeError::TooManySegments(many.len())),
            },
        }
        let total = self.start.len()
            + width(&self.envelope)
            + data.len()
            + width(&self.tail)
            + self.end.len();
        if total > self.max_frame {
            return Err(EncodeError::FrameTooLarge { len: total, max: self.max_frame });
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&self.start);
        for f in &self.envelope {
            let v = match f.role {
                FieldRole::Length => data.len() as u64,
                FieldRole::Count => frame.segments.len() as u64,
                FieldRole::Sequence => sequence
                    .or_else(|| frame.fields.get(&f.name).copied())
                    .unwrap_or(0),
                FieldRole::Opaque => frame.fields.get(&f.name).copied().unwrap_or(f.value),
            };
            put(f, v, &mut out)?;
        }
        out.extend_from_slice(&data);
        for f in &self.tail {
            put(f, frame.fields.get(&f.name).copied().unwrap_or(f.value), &mut out)?;
        }
        out.extend_from_slice(&self.end);
        Ok(out)
    }

    /// Decode the first frame in `buf`, returning it and how many bytes it used.
    pub fn decode(&self, buf: &[u8]) -> Result<(Frame, usize), DecodeError> {
        let bad = |from: usize| DecodeError::BadFrame {
            resync: match self.start.first() {
                Some(first) => buf[from.min(buf.len())..]
                    .iter()
                    .position(|b| b == first)
                    .map(|i| i + from)
                    .unwrap_or(buf.len()),
                None => from.min(buf.len()).max(1),
            },
        };
        if buf.is_empty() {
            return Err(DecodeError::NeedMore);
        }
        // The start delimiter, possibly still arriving.
        let s = self.start.len();
        let have = buf.len().min(s);
        if buf[..have] != self.start[..have] {
            return Err(bad(1));
        }
        let env_end = s + width(&self.envelope);
        if buf.len() < env_end {
            return Err(DecodeError::NeedMore);
        }
        let mut fields = BTreeMap::new();
        let mut at = s;
        for f in &self.envelope {
            fields.insert(f.name.clone(), f.read(&buf[at..at + f.size]));
            at += f.size;
        }
        let tail_len = width(&self.tail) + self.end.len();
        // Every length and count read is bounded by the frame limit before any arithmetic: a
        // field may be 8 bytes wide, and a hostile value must be refused, never wrapped (C-010,
        // refute round 4).
        let max = self.max_frame as u64;
        let length = role(&self.envelope, FieldRole::Length).map(|f| fields[&f.name]);
        let count = role(&self.envelope, FieldRole::Count).map(|f| fields[&f.name]);
        if length.is_some_and(|l| l > max) || count.is_some_and(|c| c > max) {
            return Err(bad(1));
        }
        let (length, count) = (length.map(|l| l as usize), count.map(|c| c as usize));
        let data_end = length.map(|l| env_end + l);
        if let Some(de) = data_end
            && de + tail_len > self.max_frame
        {
            return Err(bad(1));
        }

        // The segment headers, walked as they arrive.
        let mut bodies: Vec<(BTreeMap<String, u64>, usize, usize)> = Vec::new();
        let data_end = match &self.segment {
            None => match data_end {
                Some(de) => de,
                None => return Err(bad(1)), // schema validation forbids this layout
            },
            Some(header) => {
                let hdr = width(header);
                if let (Some(c), Some(l)) = (count, length)
                    && (c * hdr > l || (c == 0 && l != 0))
                {
                    return Err(bad(1));
                }
                // A count whose headers alone cannot fit the frame limit can never end.
                if let Some(c) = count
                    && env_end + c * hdr + tail_len > self.max_frame
                {
                    return Err(bad(1));
                }
                let mut i = 0;
                loop {
                    let done = match (count, data_end) {
                        (Some(c), _) => i == c,
                        (None, Some(de)) => at == de,
                        (None, None) => return Err(bad(1)),
                    };
                    if done {
                        break;
                    }
                    if let Some(de) = data_end
                        && at + hdr > de
                    {
                        return Err(bad(1));
                    }
                    if at + hdr > buf.len() {
                        return Err(DecodeError::NeedMore);
                    }
                    let mut seg_fields = BTreeMap::new();
                    let mut h = at;
                    let mut body_len = 0u64;
                    for f in header {
                        let v = f.read(&buf[h..h + f.size]);
                        if f.role == FieldRole::Length {
                            body_len = v;
                        }
                        seg_fields.insert(f.name.clone(), v);
                        h += f.size;
                    }
                    if body_len > max {
                        return Err(bad(1));
                    }
                    let end = h + body_len as usize;
                    // The segments still to come need their headers too.
                    let still = count.map(|c| (c - i - 1) * hdr).unwrap_or(0);
                    if end + still + tail_len > self.max_frame {
                        return Err(bad(1));
                    }
                    if let Some(de) = data_end {
                        let remaining = count.map(|c| c - i - 1);
                        if end > de
                            || remaining.is_some_and(|r| r * hdr > de - end)
                            || (remaining == Some(0) && end != de)
                        {
                            return Err(bad(1));
                        }
                    }
                    bodies.push((seg_fields, h, end));
                    at = end;
                    i += 1;
                }
                at
            }
        };

        let total = data_end + tail_len;
        if buf.len() < total {
            return Err(DecodeError::NeedMore);
        }
        let mut t = data_end;
        for f in &self.tail {
            fields.insert(f.name.clone(), f.read(&buf[t..t + f.size]));
            t += f.size;
        }
        if buf[t..total] != self.end[..] {
            return Err(bad(1));
        }
        let segments = match &self.segment {
            Some(_) => bodies
                .into_iter()
                .map(|(fields, a, b)| Segment { fields, body: buf[a..b].to_vec() })
                .collect(),
            None if data_end > env_end => {
                vec![Segment { fields: BTreeMap::new(), body: buf[env_end..data_end].to_vec() }]
            }
            None => Vec::new(),
        };
        Ok((Frame { fields, segments }, total))
    }
}

fn put(f: &Field, v: u64, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    if v > f.max() {
        return Err(EncodeError::DoesNotFit { field: f.name.clone(), value: v, size: f.size });
    }
    f.write(v, out);
    Ok(())
}

/// Why a typed-in hex text did not parse.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    #[error("hex needs an even number of digits")]
    OddDigits,
    #[error("`{0}` is not a hex byte")]
    NotHex(String),
}

/// Parse whitespace-separated hex (`"00 1A FF"`, `"001AFF"`) into bytes — how a binary body or a
/// delimiter is typed in a configuration or a GUI.
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
/// ASCII, 16 bytes per row: how a stored raw frame is shown to be read
/// back by an operator.
pub fn hex_dump(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4);
    for (row, chunk) in data.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|b| if (0x20..0x7F).contains(b) { *b as char } else { '.' })
            .collect();
        out.push_str(&format!("{:04X}  {:<47}  {}\n", row * 16, hex.join(" "), ascii));
    }
    out
}
