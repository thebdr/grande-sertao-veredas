//! Encoding detection and streaming decode: UTF-8 (with or without a byte-order mark), UTF-16
//! LE and BE (a mark, or the zero-byte pattern of ASCII-heavy text), CP1252 as the fallback.
//!
//! Detection is lazy: the first bytes of a stream are held until there is enough signal (four
//! bytes, or a newline, or the end of the stream), then a decoder is committed for the file's
//! lifetime. A multi-byte sequence split across two reads is carried by the incremental decoder.

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

/// A streaming decoder with lazy detection. Bytes fed before detection commits are held;
/// afterwards they stream straight through.
pub(crate) struct TextDecoder {
    state: Option<(DetectedEncoding, Decoder)>,
    prebuf: Vec<u8>,
}

impl TextDecoder {
    /// An auto-detecting decoder for a stream read from its beginning, or from a live tail's
    /// attach point (no mark there; the heuristics carry it).
    pub fn new() -> Self {
        TextDecoder { state: None, prebuf: Vec::new() }
    }

    /// A decoder for a stream whose encoding is already known (a backfill seeking into the
    /// middle of a file it peeked earlier).
    pub fn with_encoding(enc: DetectedEncoding) -> Self {
        TextDecoder { state: Some((enc, enc.encoding().new_decoder_with_bom_removal())), prebuf: Vec::new() }
    }

    /// Feed raw bytes; decoded text is appended to `out`. May append nothing while detection
    /// still holds the first bytes.
    pub fn feed(&mut self, bytes: &[u8], out: &mut String) {
        if self.state.is_none() {
            self.prebuf.extend_from_slice(bytes);
            // Enough signal? A newline counts too: a complete tiny line must not wait for a
            // fourth byte.
            if self.prebuf.len() >= 4 || self.prebuf.contains(&b'\n') {
                self.commit(false, out);
            }
            return;
        }
        if let Some((_, decoder)) = self.state.as_mut() {
            decode_append(decoder, bytes, out, false);
        }
    }

    /// The end of the stream: decode whatever is still held (detecting from it when never
    /// committed) and finish a dangling partial sequence as U+FFFD. The decoder returns to the
    /// undetected state afterwards.
    pub fn finish(&mut self, out: &mut String) {
        if self.state.is_none() {
            if self.prebuf.is_empty() {
                return;
            }
            self.commit(true, out);
            self.state = None;
            return;
        }
        if let Some((_, mut decoder)) = self.state.take() {
            decode_append(&mut decoder, &[], out, true);
        }
    }

    /// Truncation or replacement: the next bytes are new content.
    pub fn reset(&mut self) {
        self.state = None;
        self.prebuf.clear();
    }

    fn commit(&mut self, last: bool, out: &mut String) {
        let enc = detect(&self.prebuf);
        let mut decoder = enc.encoding().new_decoder_with_bom_removal();
        let buffered = std::mem::take(&mut self.prebuf);
        decode_append(&mut decoder, &buffered, out, last);
        self.state = Some((enc, decoder));
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
    }
}
