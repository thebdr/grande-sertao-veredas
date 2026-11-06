//! An OPC `Variant` into a typed `Value`, and bits carved out of words (C-019). Veredas 1's
//! tests, plus every wire type at its edges, the refusals' exact text, and bits read at the wire
//! word's own width.

use std::sync::Arc;

use chrono::NaiveDate;
use opcua_types::{DateTime, LocalizedText, QualifiedName};
use veredas_core::opc::value::{ValueError, bit_field, opc_to_value};
use veredas_core::opc::{StatusCode, Variant};
use veredas_core::{SeriesType, Value};

fn int(v: Variant) -> Result<Value, ValueError> {
    opc_to_value(&v, SeriesType::Int)
}

fn float(v: Variant) -> Result<Value, ValueError> {
    opc_to_value(&v, SeriesType::Float)
}

#[test]
fn integers_widen_but_floats_never_narrow() {
    assert_eq!(int(Variant::Int16(7)).unwrap(), Value::Int(7));
    // An int tag is welcome in a float series.
    assert_eq!(float(Variant::UInt16(7)).unwrap(), Value::Float(7.0));
    // The reverse would have to truncate: refuse, loudly.
    let err = int(Variant::Double(21.7)).unwrap_err();
    assert_eq!(err, ValueError::Mismatch { wire: "Double", want: SeriesType::Int });
    assert_eq!(err.to_string(), "Double value does not fit a int series");
    assert_eq!(int(Variant::Float(2.0)).unwrap_err(), ValueError::Mismatch { wire: "Float", want: SeriesType::Int });
}

#[test]
fn every_integer_wire_type_reaches_an_int_series_exactly_at_its_edges() {
    let cases: Vec<(Variant, i64)> = vec![
        (Variant::SByte(i8::MIN), i64::from(i8::MIN)),
        (Variant::SByte(i8::MAX), i64::from(i8::MAX)),
        (Variant::Byte(u8::MAX), i64::from(u8::MAX)),
        (Variant::Int16(i16::MIN), i64::from(i16::MIN)),
        (Variant::UInt16(u16::MAX), i64::from(u16::MAX)),
        (Variant::Int32(i32::MIN), i64::from(i32::MIN)),
        (Variant::UInt32(u32::MAX), i64::from(u32::MAX)),
        (Variant::Int64(i64::MIN), i64::MIN),
        (Variant::Int64(i64::MAX), i64::MAX),
        (Variant::UInt64(i64::MAX as u64), i64::MAX),
        (Variant::UInt64(0), 0),
    ];
    for (v, want) in cases {
        assert_eq!(int(v.clone()).unwrap(), Value::Int(want), "{v:?}");
        assert_eq!(float(v.clone()).unwrap(), Value::Float(want as f64), "{v:?}");
    }
}

#[test]
fn booleans_are_integers_and_empty_is_an_error_for_every_series() {
    assert_eq!(int(Variant::Boolean(true)).unwrap(), Value::Int(1));
    assert_eq!(int(Variant::Boolean(false)).unwrap(), Value::Int(0));
    assert_eq!(float(Variant::Boolean(false)).unwrap(), Value::Float(0.0));
    for want in [SeriesType::Int, SeriesType::Float, SeriesType::Text, SeriesType::Time] {
        assert_eq!(opc_to_value(&Variant::Empty, want), Err(ValueError::Empty), "{want}");
    }
    assert_eq!(ValueError::Empty.to_string(), "no value");
}

#[test]
fn floats_keep_their_value_in_a_float_series() {
    assert_eq!(float(Variant::Double(21.7)).unwrap(), Value::Float(21.7));
    // An f32 widens exactly: the f32 nearest 0.1, not 0.1.
    assert_eq!(float(Variant::Float(0.1)).unwrap(), Value::Float(f64::from(0.1f32)));
    assert_eq!(float(Variant::Double(f64::MAX)).unwrap(), Value::Float(f64::MAX));
    assert_eq!(float(Variant::Double(-0.0)).unwrap(), Value::Float(-0.0));
    let nan = float(Variant::Double(f64::NAN)).unwrap();
    assert!(matches!(nan, Value::Float(x) if x.is_nan()));
}

#[test]
fn text_accepts_anything_rendering_text_as_text_and_numbers_as_numbers() {
    let text = |v: Variant| opc_to_value(&v, SeriesType::Text).unwrap();
    assert_eq!(text(Variant::from("B-042")), Value::Text(Arc::from("B-042")));
    // A numeric tag mapped to a text series renders its number, not a debug blob.
    assert_eq!(text(Variant::Int32(42)), Value::Text(Arc::from("42")));
    assert_eq!(text(Variant::Double(21.5)), Value::Text(Arc::from("21.5")));
    assert_eq!(text(Variant::Boolean(true)), Value::Text(Arc::from("true")));
    assert_eq!(text(Variant::from(LocalizedText::new("en", "Running"))), Value::Text(Arc::from("Running")));
    assert_eq!(text(Variant::from(QualifiedName::new(2, "Motor"))), Value::Text(Arc::from("Motor")));
    assert_eq!(text(Variant::from(StatusCode::BadNodeIdUnknown)), Value::Text(Arc::from("BadNodeIdUnknown")));
}

#[test]
fn a_datetime_reaches_a_time_series_as_naive_utc_and_nothing_else_does() {
    let at = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap().and_hms_milli_opt(13, 45, 7, 250).unwrap();
    let dt = DateTime::from(at.and_utc());
    assert_eq!(opc_to_value(&Variant::from(dt), SeriesType::Time).unwrap(), Value::Time(at));
    for (v, wire) in [(Variant::Int64(0), "Int64"), (Variant::from("2026-11-06"), "String"), (Variant::Double(1.0), "Double")] {
        assert_eq!(
            opc_to_value(&v, SeriesType::Time),
            Err(ValueError::Mismatch { wire, want: SeriesType::Time })
        );
    }
    // A time is no number.
    assert_eq!(int(Variant::from(dt)), Err(ValueError::Mismatch { wire: "DateTime", want: SeriesType::Int }));
    assert_eq!(float(Variant::from(dt)), Err(ValueError::Mismatch { wire: "DateTime", want: SeriesType::Float }));
}

#[test]
fn a_u64_beyond_i64_refuses_rather_than_wrapping() {
    let big = Variant::UInt64(i64::MAX as u64 + 1);
    assert_eq!(int(big.clone()), Err(ValueError::Mismatch { wire: "UInt64", want: SeriesType::Int }));
    assert_eq!(int(Variant::UInt64(u64::MAX)), Err(ValueError::Mismatch { wire: "UInt64", want: SeriesType::Int }));
    // As a float it is representable, rounded like any float.
    assert_eq!(float(Variant::UInt64(u64::MAX)).unwrap(), Value::Float(u64::MAX as f64));
    assert_eq!(float(big).unwrap(), Value::Float(9_223_372_036_854_775_808.0));
}

#[test]
fn text_is_no_number() {
    assert_eq!(int(Variant::from("42")), Err(ValueError::Mismatch { wire: "String", want: SeriesType::Int }));
    assert_eq!(float(Variant::from("4.2")), Err(ValueError::Mismatch { wire: "String", want: SeriesType::Float }));
}

#[test]
fn single_bits_read_off_a_status_word() {
    // 0b0000_0000_0101_0011: bits 0, 1, 4, 6 set.
    let w = Variant::UInt16(0b0000_0000_0101_0011);
    for (b, want) in [(0, 1), (1, 1), (2, 0), (3, 0), (4, 1), (5, 0), (6, 1), (7, 0), (15, 0)] {
        assert_eq!(bit_field(&w, b, 1).unwrap(), Value::Int(want), "bit {b}");
    }
    // Past the top of a 16-bit word: zero, not an error (configured for a wider word).
    assert_eq!(bit_field(&w, 16, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&w, 20, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&w, 63, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&w, 64, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&w, 255, 8).unwrap(), Value::Int(0));
    // A Boolean is a one-bit word.
    assert_eq!(bit_field(&Variant::Boolean(true), 0, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&Variant::Boolean(true), 1, 1).unwrap(), Value::Int(0));
}

#[test]
fn packed_multi_bit_fields() {
    // bits 4..6 = 0b101 = 5.
    let w = Variant::UInt16(0b0000_0000_0101_0000);
    assert_eq!(bit_field(&w, 4, 3).unwrap(), Value::Int(5));
    // Width 0 is treated as 1 rather than yielding a constant 0.
    assert_eq!(bit_field(&w, 4, 0).unwrap(), Value::Int(1));
    // A full-width read returns the whole word.
    assert_eq!(bit_field(&Variant::UInt16(0xBEEF), 0, 16).unwrap(), Value::Int(0xBEEF));
    // A field running past the word's top keeps only the word's bits.
    assert_eq!(bit_field(&Variant::UInt16(0xBEEF), 12, 8).unwrap(), Value::Int(0xB));
    // Widths past 64 read as 64.
    assert_eq!(bit_field(&Variant::UInt32(0xDEAD_BEEF), 0, 200).unwrap(), Value::Int(0xDEAD_BEEF));
}

#[test]
fn negative_words_expose_exactly_their_own_twos_complement_bits() {
    // -1 as Int16 is sixteen ones, and nothing above them.
    let w = Variant::Int16(-1);
    assert_eq!(bit_field(&w, 0, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&w, 15, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&w, 16, 1).unwrap(), Value::Int(0), "no sign extension past the word");
    assert_eq!(bit_field(&w, 0, 32).unwrap(), Value::Int(0xFFFF));
    // Int16::MIN is bit 15 alone.
    let w = Variant::Int16(i16::MIN);
    assert_eq!(bit_field(&w, 15, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&w, 0, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&w, 0, 16).unwrap(), Value::Int(0x8000));
    // The same at every signed width.
    assert_eq!(bit_field(&Variant::SByte(-1), 0, 16).unwrap(), Value::Int(0xFF));
    assert_eq!(bit_field(&Variant::SByte(-128), 7, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&Variant::Int32(-1), 0, 40).unwrap(), Value::Int(0xFFFF_FFFF));
    assert_eq!(bit_field(&Variant::Int32(-1), 32, 1).unwrap(), Value::Int(0));
    assert_eq!(bit_field(&Variant::Int64(-1), 63, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&Variant::Int64(-1), 1, 63).unwrap(), Value::Int(i64::MAX));
}

#[test]
fn a_u64_word_has_all_its_bits_and_a_field_too_wide_for_an_int_is_refused() {
    let top = Variant::UInt64(u64::MAX);
    assert_eq!(bit_field(&top, 63, 1).unwrap(), Value::Int(1));
    assert_eq!(bit_field(&top, 60, 4).unwrap(), Value::Int(0xF));
    assert_eq!(bit_field(&top, 1, 63).unwrap(), Value::Int(i64::MAX));
    // All 64 bits with the top one set has no lossless Int: refused, never wrapped to -1.
    let err = bit_field(&top, 0, 64).unwrap_err();
    assert_eq!(err, ValueError::FieldOverflow { wire: "UInt64", bit: 0, width: 64 });
    assert_eq!(err.to_string(), "bits 0..64 of a UInt64 value do not fit an int series");
    assert_eq!(
        bit_field(&Variant::Int64(-1), 0, 64),
        Err(ValueError::FieldOverflow { wire: "Int64", bit: 0, width: 64 })
    );
    // Without the top bit, the whole word fits.
    assert_eq!(bit_field(&Variant::UInt64(i64::MAX as u64), 0, 64).unwrap(), Value::Int(i64::MAX));
    assert_eq!(bit_field(&Variant::Int64(i64::MIN), 0, 100), Err(ValueError::FieldOverflow { wire: "Int64", bit: 0, width: 64 }));
}

#[test]
fn bits_refuse_non_integers() {
    let err = bit_field(&Variant::Double(1.5), 0, 1).unwrap_err();
    assert_eq!(err, ValueError::NotIntegral { wire: "Double" });
    assert_eq!(err.to_string(), "bit extraction needs an integer value, got Double");
    assert_eq!(bit_field(&Variant::Float(1.0), 0, 1), Err(ValueError::NotIntegral { wire: "Float" }));
    assert_eq!(bit_field(&Variant::Empty, 0, 1), Err(ValueError::Empty));
    // A string tag cannot have bits either, even one that reads as a number.
    assert_eq!(bit_field(&Variant::from("3"), 0, 1), Err(ValueError::NotIntegral { wire: "String" }));
}
