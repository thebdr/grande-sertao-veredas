//! The capture rules engine (contract: the capture clause): message bodies read into named
//! fields by configuration, from Veredas 1's `veredas-telegram` tests (its scanner-request path
//! is device-specific and stayed out of core).

use std::collections::HashMap;

use veredas_core::telegram::Endian;
use veredas_core::telegram::capture::*;

#[test]
fn generic_regex_capture_names_its_groups() {
    let capture = Capture::new(&CaptureSpec {
        payload: Some(r"SRC=(?P<infeed>\d+);ID=(?P<id>\w+)".into()),
        ..Default::default()
    });
    assert!(capture.warnings.is_empty());
    let out = capture
        .extract(b"SRC=2;ID=ABC123", Endian::Little)
        .expect("matches");
    assert_eq!(out.get("infeed").map(String::as_str), Some("2"));
    assert_eq!(out.get("id").map(String::as_str), Some("ABC123"));

    // A body that does not match is a REFUSAL, not an empty map: a
    // malformed request must not earn a well-formed reply.
    assert_eq!(capture.extract(b"nothing here", Endian::Little), None);
}

#[test]
fn repeat_patterns_capture_every_occurrence_in_order() {
    let capture = Capture::new(&CaptureSpec {
        repeat: vec![RepeatSpec {
            name: "barcode".into(),
            pattern: r"BC=(\w+)".into(),
            max: 16,
        }],
        ..Default::default()
    });
    let out = capture
        .extract(b"BC=AAA BC=BBB BC=CCC", Endian::Little)
        .expect("extracts");
    assert_eq!(out.get("barcode1").map(String::as_str), Some("AAA"));
    assert_eq!(out.get("barcode2").map(String::as_str), Some("BBB"));
    assert_eq!(out.get("barcode3").map(String::as_str), Some("CCC"));
    assert_eq!(out.get("barcode4"), None);

    // Zero occurrences is legal (nothing captured), and the cap
    // holds against a flood.
    let out = capture
        .extract(b"no codes", Endian::Little)
        .expect("extracts");
    assert!(out.is_empty());
    let many = "BC=X ".repeat(100);
    let out = capture.extract(many.as_bytes(), Endian::Little).unwrap();
    assert_eq!(out.len(), 16, "capped at max");
}

#[test]
fn binary_fields_read_at_offsets_in_both_byte_orders() {
    let spec = CaptureSpec {
        fields: vec![
            BinaryField {
                name: "parcel".into(),
                at: 0,
                len: 0,
                ty: FieldType::U32,
            },
            BinaryField {
                name: "lane".into(),
                at: 4,
                len: 0,
                ty: FieldType::U8,
            },
            BinaryField {
                name: "offset".into(),
                at: 5,
                len: 0,
                ty: FieldType::I16,
            },
            BinaryField {
                name: "code".into(),
                at: 7,
                len: 8,
                ty: FieldType::Ascii,
            },
        ],
        ..Default::default()
    };
    let capture = Capture::new(&spec);

    let mut body = Vec::new();
    body.extend_from_slice(&1234u32.to_le_bytes());
    body.push(3);
    body.extend_from_slice(&(-5i16).to_le_bytes());
    body.extend_from_slice(b"ABC\0\0\0\0\0");
    let out = capture.extract(&body, Endian::Little).expect("extracts");
    assert_eq!(out.get("parcel").map(String::as_str), Some("1234"));
    assert_eq!(out.get("lane").map(String::as_str), Some("3"));
    assert_eq!(out.get("offset").map(String::as_str), Some("-5"));
    assert_eq!(out.get("code").map(String::as_str), Some("ABC"));

    // The same bytes read big-endian give different (wrong) numbers,
    // which is why the channel's endian setting is one switch.
    let out = capture.extract(&body, Endian::Big).expect("extracts");
    assert_ne!(out.get("parcel").map(String::as_str), Some("1234"));
}

#[test]
fn a_body_too_short_for_a_field_does_not_match_and_does_not_panic() {
    // Remotely triggerable with one short frame: it must be a
    // refusal, never a slice panic that kills the source.
    let capture = Capture::new(&CaptureSpec {
        fields: vec![BinaryField {
            name: "x".into(),
            at: 10,
            len: 0,
            ty: FieldType::U32,
        }],
        ..Default::default()
    });
    assert_eq!(capture.extract(b"short", Endian::Little), None);
    assert_eq!(capture.extract(&[], Endian::Little), None);

    // An offset that would overflow the addition is refused too.
    let capture = Capture::new(&CaptureSpec {
        fields: vec![BinaryField {
            name: "x".into(),
            at: usize::MAX,
            len: 4,
            ty: FieldType::Ascii,
        }],
        ..Default::default()
    });
    assert_eq!(capture.extract(b"whatever", Endian::Little), None);
}

#[test]
fn a_binary_field_wins_a_name_collision_with_the_regex() {
    // A fixed offset is more precise than a text match; the winner
    // must be predictable rather than map-order chance.
    let capture = Capture::new(&CaptureSpec {
        payload: Some(r"ID=(?P<id>\w+)".into()),
        fields: vec![BinaryField {
            name: "id".into(),
            at: 0,
            len: 2,
            ty: FieldType::Ascii,
        }],
        ..Default::default()
    });
    let out = capture.extract(b"XY ID=TEXT", Endian::Little).unwrap();
    assert_eq!(out.get("id").map(String::as_str), Some("XY"));
}

#[test]
fn a_bad_pattern_is_a_warning_that_drops_one_capture() {
    let capture = Capture::new(&CaptureSpec {
        payload: Some("([".into()),
        repeat: vec![RepeatSpec {
            name: "b".into(),
            pattern: "(?P<".into(),
            max: 4,
        }],
        ..Default::default()
    });
    assert_eq!(capture.warnings.len(), 2, "{:?}", capture.warnings);
    // With both patterns dropped the spec captures nothing, but it
    // still extracts (an empty map) rather than refusing everything.
    assert_eq!(
        capture.extract(b"anything", Endian::Little),
        Some(HashMap::new())
    );
}

// ---------------------------------------------------------------------------
// Refute round 1 (C-013): each claim pinned with exact values.
// ---------------------------------------------------------------------------

fn one_field(name: &str, at: usize, len: usize, ty: FieldType) -> Capture {
    Capture::new(&CaptureSpec {
        fields: vec![BinaryField { name: name.into(), at, len, ty }],
        ..Default::default()
    })
}

fn read_one(at: usize, len: usize, ty: FieldType, body: &[u8], endian: Endian) -> Option<String> {
    one_field("v", at, len, ty).extract(body, endian).map(|m| m["v"].clone())
}

#[test]
fn every_field_type_reads_its_exact_value_in_both_orders() {
    let body = [0x12, 0x34, 0x56, 0x78, 0xAB, 0xCD];
    let both = |at, len, ty| {
        (
            read_one(at, len, ty, &body, Endian::Little).unwrap(),
            read_one(at, len, ty, &body, Endian::Big).unwrap(),
        )
    };
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(both(0, 0, FieldType::U8), s("18", "18"));
    assert_eq!(both(0, 0, FieldType::U16), s("13330", "4660"));
    assert_eq!(both(4, 0, FieldType::I16), s("-12885", "-21555"));
    assert_eq!(both(0, 0, FieldType::U32), s("2018915346", "305419896"));
    assert_eq!(both(2, 0, FieldType::I32), s("-844400554", "1450748877"));
    assert_eq!(both(0, 6, FieldType::Hex), s("12345678ABCD", "12345678ABCD"));
}

#[test]
fn a_field_running_past_the_end_does_not_match() {
    let body = b"12345";
    for (at, len, ty) in [
        (2, 0, FieldType::U32),
        (3, 0, FieldType::U32),
        (4, 0, FieldType::U16),
        (5, 0, FieldType::I16),
        (5, 0, FieldType::U8),
        (3, 4, FieldType::Ascii),
        (0, usize::MAX, FieldType::Hex),
        (usize::MAX - 2, 0, FieldType::U32),
    ] {
        assert_eq!(read_one(at, len, ty, body, Endian::Little), None, "{ty:?} at {at} len {len}");
    }
    // A field that ends exactly at the body's end matches.
    assert_eq!(read_one(1, 0, FieldType::U32, body, Endian::Little).as_deref(), Some("892613426"));
    assert_eq!(read_one(4, 0, FieldType::U8, body, Endian::Little).as_deref(), Some("53"));
}

#[test]
fn a_repeat_stops_at_its_own_cap() {
    let capped = |max| {
        let c = Capture::new(&CaptureSpec {
            repeat: vec![RepeatSpec { name: "b".into(), pattern: r"BC=(\w+)".into(), max }],
            ..Default::default()
        });
        let mut keys: Vec<String> =
            c.extract("BC=X ".repeat(20).as_bytes(), Endian::Little).unwrap().into_keys().collect();
        keys.sort();
        keys
    };
    assert_eq!(capped(3), ["b1", "b2", "b3"]);
    assert_eq!(capped(0), ["b1"], "a cap of 0 still captures one");
}

#[test]
fn a_bad_pattern_drops_only_its_own_capture() {
    let capture = Capture::new(&CaptureSpec {
        payload: Some("([".into()),
        fields: vec![BinaryField { name: "f".into(), at: 0, len: 0, ty: FieldType::U8 }],
        repeat: vec![
            RepeatSpec { name: "bad".into(), pattern: "(?P<".into(), max: 4 },
            RepeatSpec { name: "good".into(), pattern: r"G=(\w+)".into(), max: 4 },
        ],
    });
    assert_eq!(capture.warnings.len(), 2, "{:?}", capture.warnings);
    let out = capture.extract(b"G=ONE G=TWO", Endian::Little).unwrap();
    let expected: HashMap<String, String> =
        [("good1", "ONE"), ("good2", "TWO"), ("f", "71")].map(|(k, v)| (k.into(), v.into())).into();
    assert_eq!(out, expected);
}

#[test]
fn a_binary_field_wins_over_a_repeat_too() {
    let capture = Capture::new(&CaptureSpec {
        fields: vec![BinaryField { name: "id1".into(), at: 0, len: 2, ty: FieldType::Ascii }],
        repeat: vec![RepeatSpec { name: "id".into(), pattern: r"ID=(\w+)".into(), max: 4 }],
        ..Default::default()
    });
    let out = capture.extract(b"XY ID=TEXT", Endian::Little).unwrap();
    assert_eq!(out.get("id1").map(String::as_str), Some("XY"));
}

#[test]
fn a_repeat_without_a_group_captures_the_whole_match() {
    // Group 1 when the pattern has one, else the whole match: `\d{6}` is how a barcode repeat
    // is most naturally written, and blanks here would read as no barcode at all.
    let capture = Capture::new(&CaptureSpec {
        repeat: vec![RepeatSpec { name: "barcode".into(), pattern: r"\d{6}".into(), max: 16 }],
        ..Default::default()
    });
    let out = capture.extract(b"x 123456 y 654321 z", Endian::Little).unwrap();
    let expected: HashMap<String, String> =
        [("barcode1", "123456"), ("barcode2", "654321")].map(|(k, v)| (k.into(), v.into())).into();
    assert_eq!(out, expected);
}

#[test]
fn a_capture_spec_is_read_from_configuration_text() {
    // How a channel's capture rules are written: TOML, optional keys left out, misspellings refused.
    let spec: CaptureSpec = toml::from_str(
        r#"
payload = 'ID=(?P<id>\w+)'  # a literal string: no escapes, as a regex wants
fields = [ { name = "n", at = 0, ty = "U16" } ]
repeat = [ { name = "bc", pattern = "BC=([A-Z]+)" } ]
"#,
    )
    .unwrap();
    assert_eq!(spec.repeat[0].max, 16, "a repeat without `max` takes the default cap");
    assert_eq!(spec.fields[0].len, 0);
    let literal = CaptureSpec {
        payload: Some(r"ID=(?P<id>\w+)".into()),
        fields: vec![BinaryField { name: "n".into(), at: 0, len: 0, ty: FieldType::U16 }],
        repeat: vec![RepeatSpec { name: "bc".into(), pattern: "BC=([A-Z]+)".into(), max: 16 }],
    };
    assert_eq!(spec, literal);
    let body = format!("ID=X1 {}", "BC=AB ".repeat(40));
    let out = Capture::new(&spec).extract(body.as_bytes(), Endian::Little).unwrap();
    assert_eq!(out, Capture::new(&literal).extract(body.as_bytes(), Endian::Little).unwrap());
    assert_eq!(out.len(), 1 + 1 + 16, "id, n, and 16 barcodes");
    // An empty spec is a spec; a misspelled key or type is refused.
    assert_eq!(toml::from_str::<CaptureSpec>("").unwrap(), CaptureSpec::default());
    assert!(toml::from_str::<CaptureSpec>("repeat = [ { name = \"b\", pattern = \"x\", maximum = 3 } ]").is_err());
    assert!(toml::from_str::<CaptureSpec>("fields = [ { name = \"n\", at = 0, ty = \"u16\" } ]").is_err());
    assert!(toml::from_str::<CaptureSpec>("payloads = \"x\"").is_err());
}

#[test]
fn every_field_type_and_every_table_reads_from_configuration_exactly() {
    // The seven type names, as Veredas 1 spelled them (docs/questions.md Q10).
    for (name, ty) in [
        ("U8", FieldType::U8),
        ("U16", FieldType::U16),
        ("U32", FieldType::U32),
        ("I16", FieldType::I16),
        ("I32", FieldType::I32),
        ("Ascii", FieldType::Ascii),
        ("Hex", FieldType::Hex),
    ] {
        let text = format!("fields = [ {{ name = \"f\", at = 0, len = 2, ty = \"{name}\" }} ]");
        assert_eq!(toml::from_str::<CaptureSpec>(&text).unwrap().fields[0].ty, ty, "{name}");
    }
    for wrong in ["u8", "ASCII", "ascii", "HEX", "hex", "U64"] {
        let text = format!("fields = [ {{ name = \"f\", at = 0, ty = \"{wrong}\" }} ]");
        assert!(toml::from_str::<CaptureSpec>(&text).is_err(), "{wrong}");
    }
    // A misspelled key is refused in each of the three tables.
    for text in [
        "payloads = \"x\"",
        "fields = [ { name = \"c\", at = 0, length = 4, ty = \"Ascii\" } ]",
        "repeat = [ { name = \"b\", pattern = \"x\", maximum = 3 } ]",
    ] {
        let e = toml::from_str::<CaptureSpec>(text).unwrap_err().to_string();
        assert!(e.contains("unknown field"), "{text}: {e}");
    }
}
