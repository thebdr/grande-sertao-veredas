//! An OPC UA `Variant` into a typed [`Value`], and bits carved out of words.
//!
//! An OPC server speaks some twenty wire types; the value model has four (`Int`, `Float`,
//! `Text`, `Time`). This module is the whole of that translation, free of any network so it is
//! testable without a server: the mapping is where a silent wrong number would hurt most.
//!
//! **Bits** are the second job. Controllers pack booleans into integer words: bit 0 of a UInt16
//! means "running", bits 4..6 are a 3-bit mode. Each field is configured as a series of its own,
//! so one polled tag can land as several columns.

use std::sync::Arc;

use opcua_types::Variant;

use crate::{SeriesType, Value};

/// Why one polled value could not become a typed sample. Every refusal but [`ValueError::Empty`]
/// names the wire type the server sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// The server returned no value at all.
    #[error("no value")]
    Empty,
    /// The wire type has no lossless mapping to the configured series type.
    #[error("{wire} value does not fit a {want} series")]
    Mismatch {
        /// The wire type the server sent.
        wire: &'static str,
        /// The series type the rule asked for.
        want: SeriesType,
    },
    /// Bits were configured on a value that is not a whole number.
    #[error("bit extraction needs an integer value, got {wire}")]
    NotIntegral {
        /// The wire type the server sent.
        wire: &'static str,
    },
    /// The bit field is too wide for an `Int`: its top bit is set at bit 63 of the field.
    #[error("bits {bit}..{end} of a {wire} value do not fit an int series", end = u16::from(*.bit) + u16::from(*.width))]
    FieldOverflow {
        /// The wire type the server sent.
        wire: &'static str,
        bit: u8,
        width: u8,
    },
}

/// The wire type's name, for messages the operator has to act on.
fn wire_name(v: &Variant) -> &'static str {
    match v {
        Variant::Empty => "empty",
        Variant::Boolean(_) => "Boolean",
        Variant::SByte(_) => "SByte",
        Variant::Byte(_) => "Byte",
        Variant::Int16(_) => "Int16",
        Variant::UInt16(_) => "UInt16",
        Variant::Int32(_) => "Int32",
        Variant::UInt32(_) => "UInt32",
        Variant::Int64(_) => "Int64",
        Variant::UInt64(_) => "UInt64",
        Variant::Float(_) => "Float",
        Variant::Double(_) => "Double",
        Variant::String(_) => "String",
        Variant::DateTime(_) => "DateTime",
        Variant::Guid(_) => "Guid",
        Variant::StatusCode(_) => "StatusCode",
        Variant::ByteString(_) => "ByteString",
        Variant::LocalizedText(_) => "LocalizedText",
        Variant::QualifiedName(_) => "QualifiedName",
        Variant::NodeId(_) => "NodeId",
        Variant::ExpandedNodeId(_) => "ExpandedNodeId",
        Variant::ExtensionObject(_) => "ExtensionObject",
        Variant::XmlElement(_) => "XmlElement",
        Variant::Variant(_) => "Variant",
        Variant::DataValue(_) => "DataValue",
        Variant::DiagnosticInfo(_) => "DiagnosticInfo",
        Variant::Array(_) => "Array",
        // No catch-all: a wire type the stack adds must be named here before this compiles.
    }
}

/// The value as a whole number, when it is one. `Boolean` counts (0/1): a server may expose a
/// single coil as a Boolean and it is still wanted as an `Int`. Floats do NOT: truncating 21.7 to
/// 21 is the quiet wrong a reading must never carry.
fn as_i64(v: &Variant) -> Option<i64> {
    match v {
        Variant::Boolean(b) => Some(i64::from(*b)),
        Variant::SByte(x) => Some(i64::from(*x)),
        Variant::Byte(x) => Some(i64::from(*x)),
        Variant::Int16(x) => Some(i64::from(*x)),
        Variant::UInt16(x) => Some(i64::from(*x)),
        Variant::Int32(x) => Some(i64::from(*x)),
        Variant::UInt32(x) => Some(i64::from(*x)),
        Variant::Int64(x) => Some(*x),
        // A UInt64 above i64::MAX has no lossless home: refuse rather than wrap.
        Variant::UInt64(x) => i64::try_from(*x).ok(),
        _ => None,
    }
}

/// The value as a real number, when it is one. An integer beyond 2^53 rounds to the nearest
/// float, as any float series does.
fn as_f64(v: &Variant) -> Option<f64> {
    match v {
        Variant::Float(x) => Some(f64::from(*x)),
        Variant::Double(x) => Some(*x),
        Variant::UInt64(x) => Some(*x as f64),
        other => as_i64(other).map(|i| i as f64),
    }
}

/// The integer's bits as the server holds them, at the wire type's own width: a negative Int16
/// is its sixteen two's-complement bits, with nothing set above bit 15.
fn wire_bits(v: &Variant) -> Option<u64> {
    match v {
        Variant::Boolean(b) => Some(u64::from(*b)),
        Variant::SByte(x) => Some(u64::from(*x as u8)),
        Variant::Byte(x) => Some(u64::from(*x)),
        Variant::Int16(x) => Some(u64::from(*x as u16)),
        Variant::UInt16(x) => Some(u64::from(*x)),
        Variant::Int32(x) => Some(u64::from(*x as u32)),
        Variant::UInt32(x) => Some(u64::from(*x)),
        Variant::Int64(x) => Some(*x as u64),
        Variant::UInt64(x) => Some(*x),
        _ => None,
    }
}

/// Map one polled `Variant` onto the series type its rule declares.
///
/// Widening is allowed (an `Int16` tag into a `Float` series); narrowing is not (a `Double` into
/// an `Int` series is refused rather than truncated). Anything renders into a `Text` series:
/// that is the escape hatch for a wire type the value model does not have.
pub fn opc_to_value(v: &Variant, want: SeriesType) -> Result<Value, ValueError> {
    if matches!(v, Variant::Empty) {
        return Err(ValueError::Empty);
    }
    let mismatch = || ValueError::Mismatch { wire: wire_name(v), want };
    match want {
        SeriesType::Int => as_i64(v).map(Value::Int).ok_or_else(mismatch),
        SeriesType::Float => as_f64(v).map(Value::Float).ok_or_else(mismatch),
        SeriesType::Text => Ok(Value::Text(Arc::from(render_text(v).as_str()))),
        SeriesType::Time => match v {
            Variant::DateTime(dt) => Ok(Value::Time(dt.as_chrono().naive_utc())),
            _ => Err(mismatch()),
        },
    }
}

/// A `Text` rendering for any wire type: strings and localized text as their text, a qualified
/// name as its name, a status code as its name, the numbers, booleans, ids, guids and times by
/// the stack's own text form; a type with none (an array, a structure) in its debug form.
fn render_text(v: &Variant) -> String {
    match v {
        Variant::String(s) => s.as_ref().to_string(),
        Variant::LocalizedText(t) => t.text.as_ref().to_string(),
        Variant::QualifiedName(q) => q.name.as_ref().to_string(),
        Variant::StatusCode(c) => c.to_string(),
        other => other.to_string(),
    }
}

/// Extract `width` bits starting at `bit` from an integer-valued tag, as an `Int`.
///
/// `width` 1 gives the 0/1 flag; wider takes a packed field (bits 4..6 as a 3-bit mode is
/// `bit 4, width 3`); 0 reads as 1, more than 64 as 64. Bits are read from the value's own
/// two's-complement pattern at its wire width, so a negative Int16 exposes exactly the sixteen
/// bits the controller set, and bits past the word's top read 0 (as does any `bit` from 64 up).
/// A field whose value does not fit an `Int` (64 bits with the top one set) is refused, never
/// wrapped negative.
pub fn bit_field(v: &Variant, bit: u8, width: u8) -> Result<Value, ValueError> {
    if matches!(v, Variant::Empty) {
        return Err(ValueError::Empty);
    }
    let raw = wire_bits(v).ok_or_else(|| ValueError::NotIntegral { wire: wire_name(v) })?;
    let width = width.clamp(1, 64);
    if bit >= 64 {
        return Ok(Value::Int(0));
    }
    // Saturate the mask at the word edge: `1u64 << 64` would overflow.
    let mask: u64 = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
    let field = (raw >> bit) & mask;
    i64::try_from(field)
        .map(Value::Int)
        .map_err(|_| ValueError::FieldOverflow { wire: wire_name(v), bit, width })
}
