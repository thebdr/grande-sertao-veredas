//! Turning a frame into the fields a decision is made from.
//!
//! Two paths, because two kinds of traffic exist:
//!
//! - **GENIBELT** ([`scanner_parcel`]): the typed path. A scanner
//!   request already carries the ids, the infeed and the physicals in
//!   known places — no operator regex needed. Only the **barcodes** are
//!   scanner-device-dependent (the spec calls `szMSCData` "Message
//!   received by the Scanner Device" and stops there), so how they are
//!   split out is configurable, with the common shape as the default.
//! - **Generic** ([`CaptureSpec`]): for other vendors' framings — an
//!   ASCII regex with named groups, binary fields at byte offsets, and
//!   `repeat` patterns that capture every occurrence as `name1`,
//!   `name2`, … (sixteen barcodes without a sixteen-group regex).
//!
//! Every extraction is bounds-checked. These bytes arrive from a LAN: a
//! short field is a capture that does not match, never a slice panic.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::telegram::Endian;
use crate::telegram::genibelt::{self, ScannerRequest};
use crate::telegram::sortplan::{Parcel, Physical};

/// How to pull barcodes out of a scanner device's `szMSCData`.
///
/// The first byte is the read status (`T`/`A`/SOH/`!` — §4.2.2.1) and is
/// skipped; the rest is device-specific text. Defaults match the usual
/// shape: printable ASCII, one barcode per separator run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarcodeFormat {
    /// Characters that separate barcodes.
    #[serde(default = "default_separators")]
    pub separators: String,
    /// Ignore anything shorter than this (stray control bytes, a lone
    /// status letter) — 0 keeps everything.
    #[serde(default = "default_min_len")]
    pub min_len: usize,
    /// Cap on how many are captured; the spec's plants go to 16.
    #[serde(default = "default_max_barcodes")]
    pub max_barcodes: usize,
}

fn default_separators() -> String {
    ";,|\t\r\n".to_string()
}

fn default_min_len() -> usize {
    1
}

fn default_max_barcodes() -> usize {
    16
}

impl Default for BarcodeFormat {
    fn default() -> BarcodeFormat {
        BarcodeFormat {
            separators: default_separators(),
            min_len: default_min_len(),
            max_barcodes: default_max_barcodes(),
        }
    }
}

impl BarcodeFormat {
    /// Split a `szMSCData` block into barcodes.
    pub fn split(&self, msc_data: &[u8]) -> Vec<String> {
        // Skip the status byte, stop at the block's NUL padding.
        let body = msc_data.get(1..).unwrap_or(&[]);
        let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
        let text = String::from_utf8_lossy(&body[..end]);
        text.split(|c| self.separators.contains(c))
            .map(str::trim)
            .filter(|s| s.len() >= self.min_len.max(1))
            .take(self.max_barcodes)
            .map(str::to_string)
            .collect()
    }
}

/// Build the [`Parcel`] a sort plan decides on from a scanner request.
///
/// The field names are the ones a plan's `routing.csv` matches on:
///
/// | field | source |
/// |---|---|
/// | `id` | `dwObjectID` — the MSC's identity for the item |
/// | `id2` | the induction line's tracking id |
/// | `id3` | the destination the induction line proposed |
/// | `id4` | reserved; the induction load flags |
/// | `infeed` | `byISPC_ID` — the induction point |
/// | `operator`, `scanner`, `cell`, `zone` | the request's own fields |
/// | `read_status` | `ok` / `timeout` / `virtual` / `no handshake` |
/// | `barcode1`…`barcodeN` | split from `szMSCData` |
pub fn scanner_parcel(req: &ScannerRequest, endian: Endian, barcodes: &BarcodeFormat) -> Parcel {
    let ispc = req.ispc(endian);
    let mut fields: HashMap<String, String> = HashMap::new();

    let mut put = |k: &str, v: String| {
        fields.insert(k.to_string(), v);
    };
    put("id", req.object_id.to_string());
    put("id2", ispc.tracking_id.to_string());
    put("id3", ispc.destination.to_string());
    put("id4", ispc.load_flags.to_string());
    // The induction point: from the ISPC struct when it is populated,
    // else the request's operator id (which encodes the loading zone,
    // §5).
    let infeed = if ispc.ispc_id != 0 {
        ispc.ispc_id
    } else {
        req.operator_id
    };
    put("infeed", infeed.to_string());
    put("operator", req.operator_id.to_string());
    put("scanner", req.scn_id.to_string());
    put("cell", req.cell.to_string());
    put("used_cells", req.used_cells.to_string());
    put("request_type", req.request_type.to_string());
    put("anomaly", req.request_anomaly.to_string());
    put(
        "read_status",
        match req.read_status() {
            genibelt::ReadStatus::Acknowledged => "ok".into(),
            genibelt::ReadStatus::Timeout => "timeout".into(),
            genibelt::ReadStatus::Virtual => "virtual".into(),
            genibelt::ReadStatus::NoHandshake => "no handshake".into(),
            genibelt::ReadStatus::Other(b) => format!("other({b})"),
        },
    );
    for (i, code) in barcodes.split(&req.msc_data).into_iter().enumerate() {
        fields.insert(format!("barcode{}", i + 1), code);
    }

    // Zero means "not measured" in these fields, and an unmeasured
    // dimension must never exclude a destination.
    let some_if_set = |v: u32| (v != 0).then_some(v);
    Parcel {
        object_id: req.object_id,
        fields,
        physical: Physical {
            width_mm: some_if_set(ispc.width_mm as u32),
            length_mm: some_if_set(ispc.length_mm as u32),
            height_mm: some_if_set(ispc.height_mm as u32),
            weight_g: some_if_set(ispc.weight_g as u32),
            volume: some_if_set(ispc.volume),
        },
    }
}

// ---------------------------------------------------------------------------
// The generic path
// ---------------------------------------------------------------------------

/// How to capture fields from a non-GENIBELT message body.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// never a refusal — the same rule the sort plan follows.
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
        FieldType::Ascii => genibelt::asciiz(bytes),
        FieldType::Hex => bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(""),
    })
}
