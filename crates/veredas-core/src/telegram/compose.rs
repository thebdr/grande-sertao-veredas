//! Turning named fields into a message body, by rules rather than code: the inverse of
//! [`crate::telegram::capture`].
//!
//! A channel sending some device's telegrams is configured, not programmed: a [`ComposeSpec`]
//! says how to write a body as a text template with `{name}` placeholders, binary fields at
//! byte offsets, and a fixed length padded with a byte. What the fields mean is the
//! configuration's business; this module knows no device. What compose writes, capture reads
//! back: the two share [`BinaryField`] and [`FieldType`], so one list of fields serves both
//! directions.
//!
//! Every write is bounded and fails closed: a value that does not fit its field, text longer
//! than its slot, a body longer than its declared length, or a value missing altogether is a
//! refusal, never a truncated or guessed body, because a body that lies about a field
//! desynchronises the peer for good.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::telegram::Endian;
use crate::telegram::capture::{BinaryField, FieldType};
use crate::telegram::framing::{HexError, parse_hex};

/// The largest body a spec may build: the framing's own ceiling, so a field placed at an
/// absurd offset is refused rather than allocated.
pub const MAX_BODY: usize = 16 * 1024 * 1024;

/// How to build a message body from named fields: the rules a channel is configured with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposeSpec {
    /// A text template: `{name}` is replaced by that value; `{{` and `}}` are literal braces.
    /// The body starts as the rendered text.
    #[serde(default)]
    pub template: Option<String>,
    /// Binary fields written at their offsets, the body grown with `pad` to reach them.
    #[serde(default)]
    pub fields: Vec<BinaryField>,
    /// A fixed body length: the body is padded with `pad` up to it, and refused beyond it.
    #[serde(default)]
    pub len: Option<usize>,
    /// The byte that fills gaps and the padding; 0 unless said otherwise.
    #[serde(default)]
    pub pad: u8,
}

/// Why a body could not be built.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    #[error("the template has an unclosed `{{` at character {0}")]
    UnclosedBrace(usize),
    #[error("the template has a stray `}}` at character {0}")]
    StrayBrace(usize),
    #[error("the template has an empty placeholder at character {0}")]
    EmptyPlaceholder(usize),
    #[error("the template has a `{{` inside the placeholder at character {0}")]
    BraceInPlaceholder(usize),
    /// A text or hex field without a length cannot be read back: capture reads its length.
    #[error("`{0}`: a text or hex field declares its length")]
    NoLength(String),
    /// Nothing says how to write this message: no template and no fields, while its payload
    /// pattern says it carries text.
    #[error("the message has no writing rules: no template and no fields")]
    NoRules,
    /// Two fields share bytes: the later would overwrite the earlier, which would then read
    /// back as something else.
    #[error("`{later}` overlaps `{earlier}` at byte {at}")]
    Overlap { earlier: String, later: String, at: usize },
    #[error("no value for `{0}`")]
    MissingValue(String),
    #[error("`{field}`: `{value}` is not a {ty}")]
    BadValue { field: String, value: String, ty: &'static str },
    #[error("`{field}`: {len} bytes of text do not fit {max}")]
    TextTooLong { field: String, len: usize, max: usize },
    #[error("`{field}`: {source}")]
    Hex { field: String, source: HexError },
    #[error("`{field}`: {len} hex bytes where {max} are declared")]
    HexLength { field: String, len: usize, max: usize },
    #[error("the body would be {len} bytes, over its {max}")]
    TooLong { len: usize, max: usize },
}

/// One piece of a compiled template.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Name(String),
}

/// Parse a template into its parts, refusing a brace that does not close, a stray close brace
/// or an empty placeholder, each named by the character it sits at (0-based).
fn parse_template(template: &str) -> Result<Vec<Part>, ComposeError> {
    let chars: Vec<char> = template.chars().collect();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                text.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                text.push('}');
                i += 2;
            }
            '{' => {
                let start = i;
                let close = chars[i + 1..].iter().position(|c| *c == '}').ok_or(ComposeError::UnclosedBrace(start))?;
                let raw: String = chars[i + 1..i + 1 + close].iter().collect();
                // A name is read trimmed, so `{ item }` names `item`.
                let name = raw.trim().to_string();
                if name.is_empty() {
                    return Err(ComposeError::EmptyPlaceholder(start));
                }
                if name.contains('{') {
                    return Err(ComposeError::BraceInPlaceholder(start));
                }
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Name(name));
                i += close + 2;
            }
            '}' => return Err(ComposeError::StrayBrace(i)),
            c => {
                text.push(c);
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

/// A compiled [`ComposeSpec`].
#[derive(Clone, Debug)]
pub struct Compose {
    template: Vec<Part>,
    fields: Vec<BinaryField>,
    len: Option<usize>,
    pad: u8,
}

/// A binary field's width in bytes, as capture reads it and compose writes it.
pub fn width(f: &BinaryField) -> usize {
    match f.ty {
        FieldType::U8 => 1,
        FieldType::U16 | FieldType::I16 => 2,
        FieldType::U32 | FieldType::I32 => 4,
        FieldType::Ascii | FieldType::Hex => f.len,
    }
}

/// Check a field list as both engines need it: a text or hex field with a length, and no two
/// fields sharing a byte.
pub fn check_fields(fields: &[BinaryField]) -> Result<(), ComposeError> {
    for (i, f) in fields.iter().enumerate() {
        if matches!(f.ty, FieldType::Ascii | FieldType::Hex) && f.len == 0 {
            return Err(ComposeError::NoLength(f.name.clone()));
        }
        let end = f.at.saturating_add(width(f));
        for e in &fields[..i] {
            let e_end = e.at.saturating_add(width(e));
            if f.at < e_end && e.at < end {
                return Err(ComposeError::Overlap { earlier: e.name.clone(), later: f.name.clone(), at: f.at.max(e.at) });
            }
        }
    }
    Ok(())
}

impl Compose {
    /// Compile a spec. A malformed template, a text or hex field without a length and two
    /// fields that overlap are refused here, not met on the first message.
    pub fn new(spec: &ComposeSpec) -> Result<Compose, ComposeError> {
        let template = match &spec.template {
            Some(t) => parse_template(t)?,
            None => Vec::new(),
        };
        check_fields(&spec.fields)?;
        // What the template writes whatever its values, and what the fields reach: neither may
        // exceed a fixed length, or no message could ever be written; and no length exceeds the
        // ceiling, which a body would be padded up to.
        if let Some(len) = spec.len {
            if len > MAX_BODY {
                return Err(ComposeError::TooLong { len, max: MAX_BODY });
            }
            let literal: usize = template.iter().map(|p| if let Part::Text(t) = p { t.len() } else { 0 }).sum();
            if literal > len {
                return Err(ComposeError::TooLong { len: literal, max: len });
            }
            for f in &spec.fields {
                let end = f.at.saturating_add(width(f));
                if end > len {
                    return Err(ComposeError::TooLong { len: end, max: len });
                }
            }
        }
        Ok(Compose { template, fields: spec.fields.clone(), len: spec.len, pad: spec.pad })
    }

    /// Whether anything is written at all: a template or at least one field.
    pub fn writes(&self) -> bool {
        !self.template.is_empty() || !self.fields.is_empty()
    }

    /// The names a body needs: every placeholder and every binary field, in order, each once.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.template {
            if let Part::Name(n) = p
                && !out.contains(n)
            {
                out.push(n.clone());
            }
        }
        for f in &self.fields {
            if !out.contains(&f.name) {
                out.push(f.name.clone());
            }
        }
        out
    }

    /// Build a body from `values`: the template rendered first, then each binary field written
    /// at its offset (the body grown with the pad byte as needed), then the padding to `len`.
    pub fn render(&self, values: &HashMap<String, String>, endian: Endian) -> Result<Vec<u8>, ComposeError> {
        let max = self.len.unwrap_or(MAX_BODY).min(MAX_BODY);
        let mut body: Vec<u8> = Vec::new();
        for part in &self.template {
            match part {
                Part::Text(t) => body.extend_from_slice(t.as_bytes()),
                Part::Name(n) => {
                    let v = values.get(n).ok_or_else(|| ComposeError::MissingValue(n.clone()))?;
                    body.extend_from_slice(v.as_bytes());
                }
            }
            if body.len() > max {
                return Err(ComposeError::TooLong { len: body.len(), max });
            }
        }
        for field in &self.fields {
            let value = values.get(&field.name).ok_or_else(|| ComposeError::MissingValue(field.name.clone()))?;
            let bytes = write_field(field, value, endian)?;
            // Bounded before any arithmetic grows the body: an offset past the ceiling is a
            // refusal, never an allocation.
            let end = match field.at.checked_add(bytes.len()) {
                Some(end) if end <= max => end,
                _ => return Err(ComposeError::TooLong { len: field.at.saturating_add(bytes.len()), max }),
            };
            if body.len() < end {
                body.resize(end, self.pad);
            }
            body[field.at..end].copy_from_slice(&bytes);
        }
        if let Some(len) = self.len {
            if body.len() > len {
                return Err(ComposeError::TooLong { len: body.len(), max: len });
            }
            body.resize(len, self.pad);
        }
        Ok(body)
    }
}

/// A field's type as a message names it.
fn label(ty: FieldType) -> &'static str {
    match ty {
        FieldType::U8 => "U8",
        FieldType::U16 => "U16",
        FieldType::U32 => "U32",
        FieldType::I16 => "I16",
        FieldType::I32 => "I32",
        FieldType::Ascii => "Ascii",
        FieldType::Hex => "Hex",
    }
}

/// An integer field's bytes, the value parsed as decimal text (a sign allowed only where the
/// type takes one: an unsigned type refuses `+` and `-`) and held to the type's range.
fn integer(field: &BinaryField, value: &str, endian: Endian) -> Result<Vec<u8>, ComposeError> {
    let bad = || ComposeError::BadValue { field: field.name.clone(), value: value.to_string(), ty: label(field.ty) };
    let text = value.trim();
    if matches!(field.ty, FieldType::U8 | FieldType::U16 | FieldType::U32) && text.starts_with(['+', '-']) {
        return Err(bad());
    }
    let n: i64 = text.parse().map_err(|_| bad())?;
    let (bytes, width): (u64, usize) = match field.ty {
        FieldType::U8 => (u8::try_from(n).map_err(|_| bad())? as u64, 1),
        FieldType::U16 => (u16::try_from(n).map_err(|_| bad())? as u64, 2),
        FieldType::U32 => (u32::try_from(n).map_err(|_| bad())? as u64, 4),
        FieldType::I16 => (i16::try_from(n).map_err(|_| bad())? as u16 as u64, 2),
        FieldType::I32 => (i32::try_from(n).map_err(|_| bad())? as u32 as u64, 4),
        FieldType::Ascii | FieldType::Hex => unreachable!("text fields are written apart"),
    };
    let le = bytes.to_le_bytes();
    Ok(match endian {
        Endian::Little => le[..width].to_vec(),
        Endian::Big => le[..width].iter().rev().copied().collect(),
    })
}

/// One field's bytes.
fn write_field(field: &BinaryField, value: &str, endian: Endian) -> Result<Vec<u8>, ComposeError> {
    match field.ty {
        FieldType::Ascii => {
            let bytes = value.as_bytes();
            // A NUL ends the text when it is read back: the rest would be lost.
            if bytes.contains(&0) {
                return Err(ComposeError::BadValue { field: field.name.clone(), value: value.to_string(), ty: "text without NUL" });
            }
            if bytes.len() > field.len {
                return Err(ComposeError::TextTooLong { field: field.name.clone(), len: bytes.len(), max: field.len });
            }
            let mut out = bytes.to_vec();
            out.resize(field.len, 0);
            Ok(out)
        }
        FieldType::Hex => {
            let bytes = parse_hex(value).map_err(|source| ComposeError::Hex { field: field.name.clone(), source })?;
            if bytes.len() != field.len {
                return Err(ComposeError::HexLength { field: field.name.clone(), len: bytes.len(), max: field.len });
            }
            Ok(bytes)
        }
        _ => integer(field, value, endian),
    }
}
