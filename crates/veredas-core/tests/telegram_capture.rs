//! The binary capture layer (contract: the capture clause), from Veredas 1's `veredas-telegram`
//! tests.

use std::collections::HashMap;

use veredas_core::telegram::Endian;
use veredas_core::telegram::capture::*;
use veredas_core::telegram::genibelt::{IspcMscData, SZ_DATA_LEN, ScannerRequest};
use veredas_core::telegram::sortplan::Physical;

fn scanner_with(msc: &[u8], ispc: IspcMscData) -> ScannerRequest {
    let mut msc_data = msc.to_vec();
    msc_data.resize(SZ_DATA_LEN, 0);
    ScannerRequest {
        scn_id: 1,
        operator_id: 11,
        cell: 500,
        used_cells: 1,
        object_id: 4242,
        request_type: 1,
        request_anomaly: 0,
        cell_nif_position: 900,
        msc_data,
        user_data: ispc.encode(SZ_DATA_LEN, Endian::Little),
    }
}

#[test]
fn a_scanner_request_becomes_the_fields_a_plan_matches_on() {
    let req = scanner_with(
        b"ABC-111;ABC-222;ABC-333",
        IspcMscData {
            ispc_id: 21,
            tracking_id: 777,
            destination: 55,
            load_flags: 3,
            length_mm: 400,
            width_mm: 300,
            height_mm: 250,
            weight_g: 2500,
            volume: 30_000_000,
            ..Default::default()
        },
    );
    let parcel = scanner_parcel(&req, Endian::Little, &BarcodeFormat::default());

    assert_eq!(parcel.object_id, 4242);
    assert_eq!(parcel.field("id"), Some("4242"));
    assert_eq!(parcel.field("id2"), Some("777"));
    assert_eq!(parcel.field("id3"), Some("55"));
    assert_eq!(parcel.field("id4"), Some("3"));
    assert_eq!(parcel.field("infeed"), Some("21"), "from the ISPC struct");
    assert_eq!(parcel.field("scanner"), Some("1"));
    assert_eq!(parcel.field("cell"), Some("500"));
    // The FIRST byte is the read status (§4.2.2.1), not data — here
    // the 'A' of "ABC-111" is consumed as Acknowledged, so the
    // first barcode reads "BC-111". That is the spec's layout, and
    // seeing it stated is the point: a plant whose scanner data
    // starts straight into a barcode will look one character short
    // until its BarcodeFormat is set up for that device.
    assert_eq!(parcel.field("read_status"), Some("ok"));
    assert_eq!(parcel.field("barcode1"), Some("BC-111"));
    assert_eq!(parcel.field("barcode2"), Some("ABC-222"));
    assert_eq!(parcel.field("barcode3"), Some("ABC-333"));
    assert_eq!(parcel.field("barcode4"), None);

    assert_eq!(
        parcel.physical,
        Physical {
            width_mm: Some(300),
            length_mm: Some(400),
            height_mm: Some(250),
            weight_g: Some(2500),
            volume: Some(30_000_000),
        }
    );
}

#[test]
fn unmeasured_dimensions_stay_unknown_rather_than_becoming_zero() {
    // Zero in these fields means "not measured", and a zero limit
    // check would exclude every destination.
    let req = scanner_with(b"A", IspcMscData::default());
    let parcel = scanner_parcel(&req, Endian::Little, &BarcodeFormat::default());
    assert_eq!(parcel.physical, Physical::default());
    assert!(parcel.physical.weight_g.is_none());
}

#[test]
fn a_no_read_parcel_captures_no_barcodes_but_says_why() {
    // 'T' = the scanner never answered. The plan's (empty) pattern
    // catches exactly this.
    let mut req = scanner_with(b"T", IspcMscData::default());
    req.msc_data[0] = b'T';
    let parcel = scanner_parcel(&req, Endian::Little, &BarcodeFormat::default());
    assert_eq!(parcel.field("read_status"), Some("timeout"));
    assert_eq!(parcel.field("barcode1"), None);
    assert!(parcel.fields_with_prefix("barcode").next().is_none());
}

#[test]
fn barcode_splitting_is_configurable_and_bounded() {
    let format = BarcodeFormat {
        separators: "|".into(),
        min_len: 3,
        max_barcodes: 2,
    };
    let mut data = vec![b'A'];
    data.extend_from_slice(b"AA|LONGCODE|XX|ANOTHER");
    assert_eq!(format.split(&data), vec!["LONGCODE", "ANOTHER"]);

    // The 16-barcode plant, at the default cap.
    let many: Vec<String> = (1..=20).map(|i| format!("CODE{i:02}")).collect();
    let mut data = vec![b'A'];
    data.extend_from_slice(many.join(";").as_bytes());
    data.resize(SZ_DATA_LEN, 0);
    let split = BarcodeFormat::default().split(&data);
    assert_eq!(split.len(), 16);
    assert_eq!(split[0], "CODE01");
    assert_eq!(split[15], "CODE16");
}

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
