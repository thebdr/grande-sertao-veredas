//! Turning a message body into named fields, by rules rather than code.
//!
//! A channel speaking some device's telegrams is configured, not programmed: a
//! [`CaptureSpec`] says how to read a body as an ASCII regex with named groups,
//! binary fields at byte offsets, and `repeat` patterns that capture every
//! occurrence as `name1`, `name2`, … (sixteen barcodes without a
//! sixteen-group regex). What the fields mean is the configuration's
//! business; this module knows no device.
//!
//! Every extraction is bounds-checked. These bytes arrive from a LAN: a
//! short field is a capture that does not match, never a slice panic.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::telegram::Endian;

/// Text up to the first NUL, without trailing whitespace: how a
/// fixed-width, NUL-padded text field reads.
fn asciiz(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim_end().to_string()
}

/// How to capture fields from a message body: the rules a channel is configured with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSpec {
    /// A regex over the body as text; its named groups become fields.
    #[serde(default)]
    pub payload: Option<String>,
    /// Binary fields at fixed byte offsets.
    #[serde(default)]
    pub fields: Vec<BinaryField>,
    /// Patterns captured repeatedly: every occurrence becomes
    /// `name1`, `name2`, … — sixteen barcodes without sixteen groups.
    #[serde(default)]
    pub repeat: Vec<RepeatSpec>,
}

/// One binary field: where it is, how long, and how to read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryField {
    pub name: String,
    pub at: usize,
    #[serde(default)]
    pub len: usize,
    pub ty: FieldType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    U8,
    U16,
    U32,
    I16,
    I32,
    /// Trimmed text, stopping at a NUL.
    Ascii,
    /// Uppercase hex, no separators.
    Hex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatSpec {
    pub name: String,
    pub pattern: String,
    /// Hard cap on occurrences — an attacker-controlled body must not
    /// make the capture loop forever.
    #[serde(default = "default_max_repeat")]
    pub max: usize,
}

fn default_max_repeat() -> usize {
    16
}

/// A compiled [`CaptureSpec`].
pub struct Capture {
    payload: Option<regex::Regex>,
    fields: Vec<BinaryField>,
    repeat: Vec<(String, regex::Regex, usize)>,
    pub warnings: Vec<String>,
}

impl Capture {
    /// Compile a spec. A bad regex is a warning that drops one pattern,
    /// never a refusal: a half-valid configuration still reads what it can.
    pub fn new(spec: &CaptureSpec) -> Capture {
        let mut warnings = Vec::new();
        let payload = spec
            .payload
            .as_ref()
            .and_then(|p| match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    warnings.push(format!(
                        "payload pattern `{p}` is invalid and is ignored: {e}"
                    ));
                    None
                }
            });
        let mut repeat = Vec::new();
        for spec in &spec.repeat {
            match regex::Regex::new(&spec.pattern) {
                Ok(re) => repeat.push((spec.name.clone(), re, spec.max.max(1))),
                Err(e) => warnings.push(format!(
                    "repeat pattern `{}` is invalid and is ignored: {e}",
                    spec.pattern
                )),
            }
        }
        Capture {
            payload,
            fields: spec.fields.clone(),
            repeat,
            warnings,
        }
    }

    /// Extract fields from a body.
    ///
    /// Returns `None` when the spec cannot be satisfied — the payload
    /// regex did not match, or a binary field reaches past the body. A
    /// malformed request must not earn a well-formed reply, so "does not
    /// match" is a refusal, not an empty map.
    pub fn extract(&self, data: &[u8], endian: Endian) -> Option<HashMap<String, String>> {
        let mut out: HashMap<String, String> = HashMap::new();

        if let Some(re) = &self.payload {
            let text = String::from_utf8_lossy(data);
            let caps = re.captures(&text)?;
            for name in re.capture_names().flatten() {
                if let Some(m) = caps.name(name) {
                    out.insert(name.to_string(), m.as_str().to_string());
                }
            }
        }

        for (name, re, max) in &self.repeat {
            let text = String::from_utf8_lossy(data);
            for (i, caps) in re.captures_iter(&text).take(*max).enumerate() {
                // Group 1 if the pattern has one, else the whole match.
                let value = caps
                    .get(1)
                    .or_else(|| caps.get(0))
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                out.insert(format!("{name}{}", i + 1), value);
            }
        }

        // Binary fields last: they WIN a name collision with the regex
        // path (a fixed offset is more precise than a text match), and a
        // field that does not fit fails the whole extraction.
        for field in &self.fields {
            let value = read_field(data, field, endian)?;
            out.insert(field.name.clone(), value);
        }

        Some(out)
    }
}

fn read_field(data: &[u8], field: &BinaryField, endian: Endian) -> Option<String> {
    let width = match field.ty {
        FieldType::U8 => 1,
        FieldType::U16 | FieldType::I16 => 2,
        FieldType::U32 | FieldType::I32 => 4,
        FieldType::Ascii | FieldType::Hex => field.len,
    };
    // Checked: a body too short for a declared field means the rule does
    // not match, not a panic.
    let bytes = data.get(field.at..field.at.checked_add(width)?)?;
    let u16v = |b: &[u8]| match endian {
        Endian::Big => u16::from_be_bytes([b[0], b[1]]),
        Endian::Little => u16::from_le_bytes([b[0], b[1]]),
    };
    let u32v = |b: &[u8]| match endian {
        Endian::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        Endian::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    };
    Some(match field.ty {
        FieldType::U8 => bytes[0].to_string(),
        FieldType::U16 => u16v(bytes).to_string(),
        FieldType::I16 => (u16v(bytes) as i16).to_string(),
        FieldType::U32 => u32v(bytes).to_string(),
        FieldType::I32 => (u32v(bytes) as i32).to_string(),
        FieldType::Ascii => asciiz(bytes),
        FieldType::Hex => bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(""),
    })
}
