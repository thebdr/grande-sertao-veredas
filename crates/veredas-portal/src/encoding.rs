//! Encoding detection and streaming decode: UTF-8 (with or without a byte-order mark), UTF-16
//! LE and BE (a mark, or the zero-byte pattern of ASCII-heavy text), CP1252 as the fallback.
//!
//! Detection is lazy, and decided by the first bytes that tell: a byte-order mark or the zero
//! pattern of UTF-16 within the first bytes, else the first byte beyond ASCII, which says UTF-8
//! (a valid sequence) or CP1252 (anything else). Plain ASCII tells nothing (UTF-8 and CP1252
//! agree on it byte for byte), so a file that opens with ASCII lines streams them through and
//! stays undecided until its first accented character, however far in; the encoding is then
//! committed for the file's lifetime. A multi-byte sequence split across two reads is carried
//! by the incremental decoder.

use std::fmt;

use encoding_rs::{CoderResult, Decoder, Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// The encodings a plant's text files come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectedEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Cp1252,
}

impl DetectedEncoding {
    pub fn encoding(self) -> &'static Encoding {
        match self {
            DetectedEncoding::Utf8 => UTF_8,
            DetectedEncoding::Utf16Le => UTF_16LE,
            DetectedEncoding::Utf16Be => UTF_16BE,
            DetectedEncoding::Cp1252 => WINDOWS_1252,
        }
    }

    /// The width of a code unit: two bytes in UTF-16, one otherwise.
    pub fn unit_width(self) -> usize {
        match self {
            DetectedEncoding::Utf16Le | DetectedEncoding::Utf16Be => 2,
            DetectedEncoding::Utf8 | DetectedEncoding::Cp1252 => 1,
        }
    }

    /// The bytes of a line feed in this encoding.
    pub fn newline(self) -> &'static [u8] {
        match self {
            DetectedEncoding::Utf16Le => &[0x0A, 0x00],
            DetectedEncoding::Utf16Be => &[0x00, 0x0A],
            DetectedEncoding::Utf8 | DetectedEncoding::Cp1252 => &[0x0A],
        }
    }
}

impl fmt::Display for DetectedEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DetectedEncoding::Utf8 => "UTF-8",
            DetectedEncoding::Utf16Le => "UTF-16 LE",
            DetectedEncoding::Utf16Be => "UTF-16 BE",
            DetectedEncoding::Cp1252 => "CP1252",
        })
    }
}

/// Sniff a sample taken from the start of the stream (a byte-order mark counts only there).
/// Plain ASCII reads as UTF-8 here; [`TextDecoder`] knows it has not decided yet.
pub fn detect(sample: &[u8]) -> DetectedEncoding {
    // Marks first: the one unambiguous signal.
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return DetectedEncoding::Utf8;
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return DetectedEncoding::Utf16Le;
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return DetectedEncoding::Utf16Be;
    }
    // The UTF-16 zero pattern before UTF-8 validity: ASCII-heavy UTF-16 is valid UTF-8 with
    // NULs in it and would read as UTF-8 otherwise.
    if let Some(enc) = looks_like_utf16(sample) {
        return enc;
    }
    // Valid UTF-8, or a valid prefix cut inside a sequence, is UTF-8. Pure ASCII lands here too,
    // where UTF-8 and CP1252 agree byte for byte.
    match std::str::from_utf8(sample) {
        Ok(_) => DetectedEncoding::Utf8,
        Err(e) if e.error_len().is_none() => DetectedEncoding::Utf8,
        Err(_) => DetectedEncoding::Cp1252,
    }
}

/// The length of the byte-order mark `head` starts with: three for UTF-8, two for UTF-16, none
/// otherwise.
pub fn mark_len(head: &[u8]) -> usize {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        2
    } else {
        0
    }
}

/// Whether every byte is plain ASCII text (no NUL, nothing beyond 0x7F): the bytes UTF-8 and
/// CP1252 agree on, which tell nothing about the encoding.
pub fn is_plain_ascii(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| (1..0x80).contains(b))
}

/// ASCII-heavy UTF-16 shows a stark zero pattern: little-endian zeroes the odd (high) bytes,
/// big-endian the even ones. Demands a strong signal on one side and near silence on the other.
fn looks_like_utf16(sample: &[u8]) -> Option<DetectedEncoding> {
    if sample.len() < 4 {
        return None;
    }
    let n = sample.len().min(1024) & !1;
    let pairs = n / 2;
    let mut even_zero = 0usize;
    let mut odd_zero = 0usize;
    for i in (0..n).step_by(2) {
        if sample[i] == 0 {
            even_zero += 1;
        }
        if sample[i + 1] == 0 {
            odd_zero += 1;
        }
    }
    if odd_zero * 10 >= pairs * 4 && even_zero * 10 <= pairs {
        Some(DetectedEncoding::Utf16Le)
    } else if even_zero * 10 >= pairs * 4 && odd_zero * 10 <= pairs {
        Some(DetectedEncoding::Utf16Be)
    } else {
        None
    }
}

/// How many bytes, or which byte, a fresh stream waits for before it decides.
const FRESH_SAMPLE: usize = 4;

enum State {
    /// The first bytes of a stream, held until there are enough to tell (a mark or the UTF-16
    /// pattern shows within the first few).
    Fresh(Vec<u8>),
    /// The stream opened with plain ASCII: UTF-8 or CP1252, undecided. ASCII streams through;
    /// the buffer holds the bytes from the first one beyond ASCII until there are enough to
    /// tell a UTF-8 sequence from a CP1252 byte.
    Ascii(Vec<u8>),
    Committed(DetectedEncoding, Decoder),
}

/// A streaming decoder with lazy detection. Bytes fed before detection decides are held (or,
/// being plain ASCII, streamed through); afterwards they stream straight through the committed
/// decoder.
pub(crate) struct TextDecoder {
    state: State,
}

impl TextDecoder {
    /// An auto-detecting decoder for a stream read from its beginning.
    pub fn new() -> Self {
        TextDecoder { state: State::Fresh(Vec::new()) }
    }

    /// A decoder for a stream whose encoding is already known (a backfill seeking into the
    /// middle of a file it peeked earlier).
    pub fn with_encoding(enc: DetectedEncoding) -> Self {
        TextDecoder { state: State::Committed(enc, enc.encoding().new_decoder_with_bom_removal()) }
    }

    /// A decoder for a stream joined past its start, whose head was read: a head of plain ASCII
    /// leaves UTF-8 and CP1252 undecided; anything else decides.
    pub fn from_head(head: &[u8]) -> Self {
        if head.is_empty() || is_plain_ascii(head) {
            TextDecoder { state: State::Ascii(Vec::new()) }
        } else {
            let enc = detect(head);
            TextDecoder { state: State::Committed(enc, enc.encoding().new_decoder_without_bom_handling()) }
        }
    }

    /// The encoding this stream's head decided on, when it has.
    pub fn encoding(&self) -> Option<DetectedEncoding> {
        match &self.state {
            State::Committed(enc, _) => Some(*enc),
            _ => None,
        }
    }

    /// Feed raw bytes; decoded text is appended to `out`. May append nothing while detection
    /// still holds the first bytes.
    pub fn feed(&mut self, bytes: &[u8], out: &mut String) {
        match &mut self.state {
            State::Fresh(buf) => {
                buf.extend_from_slice(bytes);
                // Enough signal? A newline counts too: a complete tiny line must not wait for a
                // fourth byte.
                if buf.len() >= FRESH_SAMPLE || buf.contains(&b'\n') {
                    self.decide_fresh(false, out);
                }
            }
            State::Ascii(held) => {
                let mut rest = bytes;
                if held.is_empty() {
                    // Plain ASCII streams through; the first byte beyond it starts the buffer.
                    let split = rest.iter().position(|b| !(1..0x80).contains(b)).unwrap_or(rest.len());
                    out.push_str(std::str::from_utf8(&rest[..split]).unwrap_or(""));
                    rest = &rest[split..];
                }
                if !rest.is_empty() {
                    held.extend_from_slice(rest);
                    if held.len() >= FRESH_SAMPLE || held.contains(&b'\n') {
                        self.decide_ascii(false, out);
                    }
                }
            }
            State::Committed(_, decoder) => decode_append(decoder, bytes, out, false),
        }
    }

    /// The end of the stream: decode whatever is still held (deciding from it when never
    /// decided) and finish a dangling partial sequence as U+FFFD. The decoder is fresh
    /// afterwards.
    pub fn finish(&mut self, out: &mut String) {
        match &mut self.state {
            State::Fresh(buf) => {
                if !buf.is_empty() {
                    self.decide_fresh(true, out);
                }
            }
            State::Ascii(held) => {
                if !held.is_empty() {
                    self.decide_ascii(true, out);
                }
            }
            State::Committed(_, decoder) => decode_append(decoder, &[], out, true),
        }
        self.state = State::Fresh(Vec::new());
    }

    /// Truncation or replacement: the next bytes are new content.
    pub fn reset(&mut self) {
        self.state = State::Fresh(Vec::new());
    }

    /// The first bytes have told what they can: a mark, the UTF-16 pattern or a byte beyond
    /// ASCII decides; plain ASCII streams through and leaves UTF-8 and CP1252 open.
    fn decide_fresh(&mut self, last: bool, out: &mut String) {
        let State::Fresh(buf) = std::mem::replace(&mut self.state, State::Fresh(Vec::new())) else { return };
        if is_plain_ascii(&buf) {
            out.push_str(std::str::from_utf8(&buf).unwrap_or(""));
            self.state = State::Ascii(Vec::new());
            if last {
                self.state = State::Fresh(Vec::new());
            }
            return;
        }
        let enc = detect(&buf);
        let mut decoder = enc.encoding().new_decoder_with_bom_removal();
        decode_append(&mut decoder, &buf, out, last);
        self.state = State::Committed(enc, decoder);
    }

    /// The first bytes beyond ASCII of an ASCII-opened stream: a valid UTF-8 sequence commits
    /// UTF-8, anything else CP1252 (the UTF-16 pattern cannot show here: an ASCII head rules
    /// it out).
    fn decide_ascii(&mut self, last: bool, out: &mut String) {
        let State::Ascii(held) = std::mem::replace(&mut self.state, State::Fresh(Vec::new())) else { return };
        let enc = match std::str::from_utf8(&held) {
            Ok(_) => DetectedEncoding::Utf8,
            // A valid prefix cut short is a sequence still arriving, or torn at the end of the
            // stream: UTF-8 either way, and the decoder finishes the torn one as U+FFFD. A byte no
            // sequence starts is CP1252.
            Err(e) if e.error_len().is_none() => DetectedEncoding::Utf8,
            Err(_) => DetectedEncoding::Cp1252,
        };
        let mut decoder = enc.encoding().new_decoder_without_bom_handling();
        decode_append(&mut decoder, &held, out, last);
        self.state = State::Committed(enc, decoder);
    }
}

fn decode_append(decoder: &mut Decoder, bytes: &[u8], out: &mut String, last: bool) {
    let mut src = bytes;
    loop {
        if let Some(needed) = decoder.max_utf8_buffer_length(src.len()) {
            out.reserve(needed);
        }
        let (result, read, _) = decoder.decode_to_string(src, out, last);
        src = &src[read..];
        match result {
            CoderResult::InputEmpty => break,
            CoderResult::OutputFull => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_detection() {
        assert_eq!(detect(&[0xEF, 0xBB, 0xBF, b'h', b'i']), DetectedEncoding::Utf8);
        assert_eq!(detect(&[0xFF, 0xFE, b'h', 0]), DetectedEncoding::Utf16Le);
        assert_eq!(detect(&[0xFE, 0xFF, 0, b'h']), DetectedEncoding::Utf16Be);
    }

    #[test]
    fn heuristics_without_bom() {
        assert_eq!(detect(b"plain ascii line\r\n"), DetectedEncoding::Utf8);
        assert_eq!(detect("héllo wörld".as_bytes()), DetectedEncoding::Utf8);
        assert_eq!(detect(&[b'h', 0, b'e', 0, b'l', 0, b'l', 0, b'o', 0]), DetectedEncoding::Utf16Le);
        assert_eq!(detect(&[0, b'h', 0, b'e', 0, b'l', 0, b'l', 0, b'o']), DetectedEncoding::Utf16Be);
        // A lone 0xB0 (the degree sign) is invalid UTF-8: CP1252.
        assert_eq!(detect(b"Temp 25\xB0C"), DetectedEncoding::Cp1252);
        // A valid prefix cut inside a sequence stays UTF-8 (C3 A9 cut at C3).
        assert_eq!(detect(&[b'a', b'b', b'c', 0xC3]), DetectedEncoding::Utf8);
        assert!(is_plain_ascii(b"plain\r\n") && !is_plain_ascii(b"a\0b") && !is_plain_ascii(b"25\xB0"));
    }

    #[test]
    fn streaming_split_code_units() {
        let mut d = TextDecoder::new();
        let mut out = String::new();
        let bytes: Vec<u8> = [0xFF, 0xFE, b'h', 0, 0xE9, 0, b'!', 0].to_vec();
        d.feed(&bytes[..3], &mut out);
        d.feed(&bytes[3..5], &mut out);
        d.feed(&bytes[5..], &mut out);
        d.finish(&mut out);
        assert_eq!(out, "hé!");
    }

    #[test]
    fn tiny_line_is_not_held() {
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"a\n", &mut out);
        assert_eq!(out, "a\n");
    }

    #[test]
    fn finish_flushes_an_undetected_prefix() {
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"ab", &mut out);
        assert_eq!(out, "");
        d.finish(&mut out);
        assert_eq!(out, "ab");
    }

    #[test]
    fn a_known_encoding_skips_detection() {
        let mut d = TextDecoder::with_encoding(DetectedEncoding::Cp1252);
        let mut out = String::new();
        d.feed(b"25\xB0C", &mut out);
        assert_eq!(out, "25°C");
        assert_eq!(d.encoding(), Some(DetectedEncoding::Cp1252));
    }

    #[test]
    fn an_ascii_opening_decides_nothing_until_the_first_byte_beyond_ascii() {
        // CP1252 after lines of ASCII.
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"header line\nmore ascii\n", &mut out);
        assert_eq!(out, "header line\nmore ascii\n");
        assert_eq!(d.encoding(), None, "ASCII tells nothing");
        d.feed(b"Temp 25\xB0C\n", &mut out);
        assert_eq!(out, "header line\nmore ascii\nTemp 25°C\n");
        assert_eq!(d.encoding(), Some(DetectedEncoding::Cp1252));
        // UTF-8 after lines of ASCII, the sequence split across feeds.
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"header\ncaf\xC3", &mut out);
        assert_eq!(out, "header\ncaf", "the high byte is held, the ASCII before it streamed");
        d.feed("\u{A9} ready\n".as_bytes().get(1..).unwrap_or(b""), &mut out);
        d.feed(b"\xA9 ready\n", &mut out);
        assert!(out.starts_with("header\ncaf"), "{out}");
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"header\ncaf\xC3", &mut out);
        d.feed(b"\xA9 ready\n", &mut out);
        assert_eq!(out, "header\ncafé ready\n");
        assert_eq!(d.encoding(), Some(DetectedEncoding::Utf8));
        // A byte no UTF-8 sequence starts, at the very end: CP1252, torn or not.
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"ascii\n\xB0", &mut out);
        d.finish(&mut out);
        assert_eq!(out, "ascii\n°");
        // A UTF-8 lead the stream ends on, in the ASCII state: torn, so U+FFFD, never a CP1252
        // character invented from it.
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"ascii\ncaf\xC3", &mut out);
        assert_eq!(out, "ascii\ncaf");
        d.finish(&mut out);
        assert_eq!(out, "ascii\ncaf\u{FFFD}");
        let mut d = TextDecoder::new();
        let mut out = String::new();
        d.feed(b"ascii\n", &mut out);
        d.feed(b"\xE2\x82", &mut out);
        d.finish(&mut out);
        assert_eq!(out, "ascii\n\u{FFFD}");
        assert_eq!(
            (mark_len(&[0xEF, 0xBB, 0xBF, b'h']), mark_len(&[0xFF, 0xFE]), mark_len(&[0xFE, 0xFF, 0]), mark_len(b"hi"), mark_len(&[0xFF])),
            (3, 2, 2, 0, 0)
        );
        assert_eq!((DetectedEncoding::Utf16Be.unit_width(), DetectedEncoding::Cp1252.unit_width()), (2, 1));
    }

    #[test]
    fn a_head_decides_or_leaves_ascii_open() {
        let mut d = TextDecoder::from_head(b"2026-07-19 10:00:00 seed 0\n");
        assert_eq!(d.encoding(), None);
        let mut out = String::new();
        d.feed(b"owner 25\xB0C\n", &mut out);
        assert_eq!(out, "owner 25°C\n");
        let d = TextDecoder::from_head(&[0xFF, 0xFE, b'o', 0, b'l', 0]);
        assert_eq!(d.encoding(), Some(DetectedEncoding::Utf16Le));
        let mut d = TextDecoder::from_head(b"na\xEFve\n");
        assert_eq!(d.encoding(), Some(DetectedEncoding::Cp1252));
        let mut out = String::new();
        d.feed(b"\xB0", &mut out);
        assert_eq!(out, "°");
        assert_eq!(TextDecoder::from_head(b"").encoding(), None);
        assert_eq!(DetectedEncoding::Utf16Le.newline(), &[0x0A, 0x00]);
        assert_eq!(DetectedEncoding::Utf16Be.newline(), &[0x00, 0x0A]);
        assert_eq!(DetectedEncoding::Cp1252.newline(), &[0x0A]);
    }
}
