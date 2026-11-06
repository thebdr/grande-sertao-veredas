//! The compose engine (contract: the compose clause): message bodies written from named fields
//! by configuration, the inverse of capture, and read back by it.

use std::collections::HashMap;

use veredas_core::telegram::Endian;
use veredas_core::telegram::capture::{BinaryField, Capture, CaptureSpec, FieldType};
use veredas_core::telegram::compose::*;
use veredas_core::telegram::framing::HexError;

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn field(name: &str, at: usize, len: usize, ty: FieldType) -> BinaryField {
    BinaryField { name: name.into(), at, len, ty }
}

fn compose(spec: &ComposeSpec) -> Compose {
    Compose::new(spec).unwrap()
}

#[test]
fn a_template_renders_its_placeholders_and_literal_braces() {
    let c = compose(&ComposeSpec { template: Some("ITEM={item};{{x}};D={dest}}}".into()), ..Default::default() });
    assert_eq!(c.names(), ["item", "dest"]);
    let body = c.render(&values(&[("item", "7"), ("dest", "3")]), Endian::Little).unwrap();
    assert_eq!(body, b"ITEM=7;{x};D=3}");
    // A missing value is a refusal, not an empty field; an extra value is ignored.
    assert_eq!(c.render(&values(&[("item", "7")]), Endian::Little), Err(ComposeError::MissingValue("dest".into())));
    assert_eq!(c.render(&values(&[("item", "7"), ("dest", ""), ("more", "x")]), Endian::Little).unwrap(), b"ITEM=7;{x};D=}");
    // Text that is not ASCII is written as UTF-8, and a name may repeat.
    let c = compose(&ComposeSpec { template: Some("{a}é{a}".into()), ..Default::default() });
    assert_eq!(c.names(), ["a"]);
    assert_eq!(c.render(&values(&[("a", "ç")]), Endian::Little).unwrap(), "çéç".as_bytes());
    // No template and no fields: an empty body.
    assert_eq!(compose(&ComposeSpec::default()).render(&HashMap::new(), Endian::Little).unwrap(), Vec::<u8>::new());
    assert!(compose(&ComposeSpec::default()).names().is_empty());
}

#[test]
fn a_malformed_template_is_refused_where_it_is_compiled() {
    let bad = |t: &str| Compose::new(&ComposeSpec { template: Some(t.into()), ..Default::default() }).err().unwrap();
    assert_eq!(bad("a{b"), ComposeError::UnclosedBrace(1));
    assert_eq!(bad("a}b"), ComposeError::StrayBrace(1));
    assert_eq!(bad("ab{}"), ComposeError::EmptyPlaceholder(2));
    assert_eq!(bad("ab{ }"), ComposeError::EmptyPlaceholder(2));
    assert_eq!(bad("{a}{"), ComposeError::UnclosedBrace(3));
    assert_eq!(bad("x{a{b}"), ComposeError::BraceInPlaceholder(1));
    assert_eq!(bad("é}"), ComposeError::StrayBrace(1), "counted in characters, not bytes");
    assert_eq!(bad("a{b").to_string(), "the template has an unclosed `{` at character 1");
    assert_eq!(bad("a}b").to_string(), "the template has a stray `}` at character 1");
    assert_eq!(bad("ab{}").to_string(), "the template has an empty placeholder at character 2");
    assert_eq!(bad("x{a{b}").to_string(), "the template has a `{` inside the placeholder at character 1");
    // Doubled braces are text, wherever they sit.
    assert!(Compose::new(&ComposeSpec { template: Some("{{}}}}{{".into()), ..Default::default() }).is_ok());
    // A placeholder's name is read trimmed.
    let c = compose(&ComposeSpec { template: Some("{ item }".into()), ..Default::default() });
    assert_eq!(c.names(), ["item"]);
    assert_eq!(c.render(&values(&[("item", "7")]), Endian::Little).unwrap(), b"7");
    // A text or hex field without a length, and two fields sharing a byte, are refused where
    // the spec compiles: neither would read back as written.
    let spec = |fields: Vec<BinaryField>| Compose::new(&ComposeSpec { fields, ..Default::default() }).err().unwrap();
    assert_eq!(spec(vec![field("t", 0, 0, FieldType::Ascii)]), ComposeError::NoLength("t".into()));
    assert_eq!(spec(vec![field("h", 2, 0, FieldType::Hex)]), ComposeError::NoLength("h".into()));
    assert_eq!(ComposeError::NoLength("t".into()).to_string(), "`t`: a text or hex field declares its length");
    assert_eq!(
        spec(vec![field("a", 0, 0, FieldType::U16), field("b", 1, 0, FieldType::U8)]),
        ComposeError::Overlap { earlier: "a".into(), later: "b".into(), at: 1 }
    );
    assert_eq!(
        spec(vec![field("a", 4, 0, FieldType::U8), field("b", 0, 8, FieldType::Ascii)]),
        ComposeError::Overlap { earlier: "a".into(), later: "b".into(), at: 4 }
    );
    assert_eq!(ComposeError::Overlap { earlier: "a".into(), later: "b".into(), at: 1 }.to_string(), "`b` overlaps `a` at byte 1");
    assert!(Compose::new(&ComposeSpec { fields: vec![field("a", 0, 0, FieldType::U16), field("b", 2, 0, FieldType::U8)], ..Default::default() }).is_ok(), "adjacent is fine");
    // A template whose literal text, or a field, cannot fit a fixed length is refused at once.
    let fixed = |template: Option<&str>, fields: Vec<BinaryField>, len: usize| {
        Compose::new(&ComposeSpec { template: template.map(String::from), fields, len: Some(len), pad: 0 }).err()
    };
    assert_eq!(fixed(Some("ABCDEF"), vec![], 2), Some(ComposeError::TooLong { len: 6, max: 2 }));
    assert_eq!(fixed(Some("A{x}"), vec![], 1), None, "a placeholder may be empty");
    assert_eq!(fixed(Some("A"), vec![], 0), Some(ComposeError::TooLong { len: 1, max: 0 }));
    assert_eq!(fixed(None, vec![field("n", 7, 0, FieldType::U16)], 8), Some(ComposeError::TooLong { len: 9, max: 8 }));
    assert_eq!(fixed(None, vec![field("n", 6, 0, FieldType::U16)], 8), None);
}

#[test]
fn binary_fields_are_written_at_their_offsets_in_both_byte_orders() {
    let spec = ComposeSpec {
        fields: vec![
            field("parcel", 0, 0, FieldType::U32),
            field("lane", 4, 0, FieldType::U8),
            field("offset", 5, 0, FieldType::I16),
            field("code", 7, 4, FieldType::Ascii),
            field("raw", 11, 2, FieldType::Hex),
            field("big", 13, 0, FieldType::I32),
        ],
        ..Default::default()
    };
    let c = compose(&spec);
    assert_eq!(c.names(), ["parcel", "lane", "offset", "code", "raw", "big"]);
    let v = values(&[("parcel", "258"), ("lane", "7"), ("offset", "-2"), ("code", "AB"), ("raw", "DE AD"), ("big", "-1")]);
    let le = c.render(&v, Endian::Little).unwrap();
    assert_eq!(le, [2, 1, 0, 0, 7, 0xFE, 0xFF, b'A', b'B', 0, 0, 0xDE, 0xAD, 0xFF, 0xFF, 0xFF, 0xFF]);
    let be = c.render(&v, Endian::Big).unwrap();
    assert_eq!(be, [0, 0, 1, 2, 7, 0xFF, 0xFE, b'A', b'B', 0, 0, 0xDE, 0xAD, 0xFF, 0xFF, 0xFF, 0xFF]);
    // Capture reads either back, field for field.
    let capture = Capture::new(&CaptureSpec { fields: spec.fields.clone(), ..Default::default() });
    for (body, endian) in [(&le, Endian::Little), (&be, Endian::Big)] {
        let back = capture.extract(body, endian).unwrap();
        assert_eq!(back["parcel"], "258");
        assert_eq!(back["lane"], "7");
        assert_eq!(back["offset"], "-2");
        assert_eq!(back["code"], "AB");
        assert_eq!(back["raw"], "DEAD");
        assert_eq!(back["big"], "-1");
    }
}

#[test]
fn every_integer_type_is_held_to_its_range_and_its_text_form() {
    let one = |ty: FieldType, text: &str| {
        let c = compose(&ComposeSpec { fields: vec![field("f", 0, 0, ty)], ..Default::default() });
        c.render(&values(&[("f", text)]), Endian::Little)
    };
    let bad = |_ty: FieldType, text: &str, label: &'static str| ComposeError::BadValue { field: "f".into(), value: text.into(), ty: label };
    assert_eq!(one(FieldType::U8, "255").unwrap(), [255]);
    assert_eq!(one(FieldType::U8, " 0 ").unwrap(), [0]);
    assert_eq!(one(FieldType::U8, "007").unwrap(), [7], "leading zeros are decimal text");
    // A sign only where the type takes one: an unsigned type refuses `+` and `-`, `-0` too.
    assert_eq!(one(FieldType::U8, "+7"), Err(bad(FieldType::U8, "+7", "U8")));
    assert_eq!(one(FieldType::U8, "-0"), Err(bad(FieldType::U8, "-0", "U8")));
    assert_eq!(one(FieldType::U16, "+1"), Err(bad(FieldType::U16, "+1", "U16")));
    assert_eq!(one(FieldType::U32, "+1"), Err(bad(FieldType::U32, "+1", "U32")));
    assert_eq!(one(FieldType::I16, "+7").unwrap(), [7, 0]);
    assert_eq!(one(FieldType::I32, "-0").unwrap(), [0, 0, 0, 0]);
    assert_eq!(one(FieldType::U8, "256"), Err(bad(FieldType::U8, "256", "U8")));
    assert_eq!(one(FieldType::U8, "-1"), Err(bad(FieldType::U8, "-1", "U8")));
    assert_eq!(one(FieldType::U8, "1_0"), Err(bad(FieldType::U8, "1_0", "U8")));
    assert_eq!(one(FieldType::U8, "1.0"), Err(bad(FieldType::U8, "1.0", "U8")));
    assert_eq!(one(FieldType::U8, "0x10"), Err(bad(FieldType::U8, "0x10", "U8")));
    assert_eq!(one(FieldType::U8, ""), Err(bad(FieldType::U8, "", "U8")));
    assert_eq!(one(FieldType::U16, "65535").unwrap(), [0xFF, 0xFF]);
    assert_eq!(one(FieldType::U16, "65536"), Err(bad(FieldType::U16, "65536", "U16")));
    assert_eq!(one(FieldType::U32, "4294967295").unwrap(), [0xFF; 4]);
    assert_eq!(one(FieldType::U32, "4294967296"), Err(bad(FieldType::U32, "4294967296", "U32")));
    assert_eq!(one(FieldType::U32, "99999999999999999999"), Err(bad(FieldType::U32, "99999999999999999999", "U32")));
    assert_eq!(one(FieldType::I16, "-32768").unwrap(), [0x00, 0x80]);
    assert_eq!(one(FieldType::I16, "32767").unwrap(), [0xFF, 0x7F]);
    assert_eq!(one(FieldType::I16, "32768"), Err(bad(FieldType::I16, "32768", "I16")));
    assert_eq!(one(FieldType::I16, "-32769"), Err(bad(FieldType::I16, "-32769", "I16")));
    assert_eq!(one(FieldType::I32, "-2147483648").unwrap(), [0, 0, 0, 0x80]);
    assert_eq!(one(FieldType::I32, "2147483648"), Err(bad(FieldType::I32, "2147483648", "I32")));
    assert_eq!(bad(FieldType::I32, "x", "I32").to_string(), "`f`: `x` is not a I32");
}

#[test]
fn text_and_hex_fields_are_held_to_their_lengths() {
    let text = |len: usize, value: &str| {
        let c = compose(&ComposeSpec { fields: vec![field("t", 0, len, FieldType::Ascii)], ..Default::default() });
        c.render(&values(&[("t", value)]), Endian::Little)
    };
    assert_eq!(text(4, "AB").unwrap(), [b'A', b'B', 0, 0], "padded with NUL");
    assert_eq!(text(4, "ABCD").unwrap(), b"ABCD");
    assert_eq!(text(4, ""), Ok(vec![0; 4]));
    assert_eq!(text(4, "ABCDE"), Err(ComposeError::TextTooLong { field: "t".into(), len: 5, max: 4 }));
    assert_eq!(text(2, "é"), Ok(vec![0xC3, 0xA9]), "counted in bytes");
    assert_eq!(text(1, "é"), Err(ComposeError::TextTooLong { field: "t".into(), len: 2, max: 1 }));
    assert_eq!(text(4, "A\0B"), Err(ComposeError::BadValue { field: "t".into(), value: "A\0B".into(), ty: "text without NUL" }), "a NUL would end the text read back");
    assert_eq!(ComposeError::TextTooLong { field: "t".into(), len: 5, max: 4 }.to_string(), "`t`: 5 bytes of text do not fit 4");
    // Capture reads a padded text back without the padding, and without trailing whitespace:
    // the one way a written text differs when read.
    let capture = Capture::new(&CaptureSpec { fields: vec![field("t", 0, 4, FieldType::Ascii)], ..Default::default() });
    assert_eq!(capture.extract(&text(4, "AB").unwrap(), Endian::Little).unwrap()["t"], "AB");
    assert_eq!(capture.extract(&text(4, "A  ").unwrap(), Endian::Little).unwrap()["t"], "A");
    assert_eq!(capture.extract(&text(4, " A").unwrap(), Endian::Little).unwrap()["t"], " A", "leading whitespace stays");

    let hex = |len: usize, value: &str| {
        let c = compose(&ComposeSpec { fields: vec![field("h", 0, len, FieldType::Hex)], ..Default::default() });
        c.render(&values(&[("h", value)]), Endian::Little)
    };
    assert_eq!(hex(2, "DE AD").unwrap(), [0xDE, 0xAD]);
    assert_eq!(hex(2, "dead").unwrap(), [0xDE, 0xAD]);
    assert_eq!(hex(2, "DE"), Err(ComposeError::HexLength { field: "h".into(), len: 1, max: 2 }));
    assert_eq!(hex(2, "DE AD BE"), Err(ComposeError::HexLength { field: "h".into(), len: 3, max: 2 }));
    assert_eq!(hex(3, "DE AD BE").unwrap(), [0xDE, 0xAD, 0xBE]);
    assert_eq!(hex(2, "DEA"), Err(ComposeError::Hex { field: "h".into(), source: HexError::OddDigits }));
    assert_eq!(hex(2, "ZZ"), Err(ComposeError::Hex { field: "h".into(), source: HexError::NotHex("ZZ".into()) }));
    assert_eq!(hex(2, "DEA").unwrap_err().to_string(), "`h`: hex needs an even number of digits");
    assert_eq!(ComposeError::HexLength { field: "h".into(), len: 1, max: 2 }.to_string(), "`h`: 1 hex bytes where 2 are declared");
    let capture = Capture::new(&CaptureSpec { fields: vec![field("h", 0, 2, FieldType::Hex)], ..Default::default() });
    assert_eq!(capture.extract(&hex(2, "dead").unwrap(), Endian::Little).unwrap()["h"], "DEAD");
}

#[test]
fn the_body_is_padded_to_its_length_and_refused_beyond_it() {
    let spec = ComposeSpec {
        template: Some("AB".into()),
        fields: vec![field("n", 4, 0, FieldType::U16)],
        len: Some(8),
        pad: 0x20,
    };
    let c = compose(&spec);
    let body = c.render(&values(&[("n", "1")]), Endian::Little).unwrap();
    assert_eq!(body, [b'A', b'B', 0x20, 0x20, 1, 0, 0x20, 0x20], "the gap and the padding take the pad byte");
    // A field over the literal text overwrites it; over a placeholder's value it is refused,
    // since that value would not read back.
    let c = compose(&ComposeSpec { template: Some("ABCD".into()), fields: vec![field("n", 1, 0, FieldType::U8)], len: None, pad: 0 });
    assert_eq!(c.render(&values(&[("n", "0")]), Endian::Little).unwrap(), [b'A', 0, b'C', b'D']);
    let c = compose(&ComposeSpec { template: Some("{a}".into()), fields: vec![field("b", 0, 0, FieldType::U8)], len: None, pad: 0 });
    assert_eq!(c.render(&values(&[("a", "Z"), ("b", "65")]), Endian::Little), Err(ComposeError::Overlap { earlier: "a".into(), later: "b".into(), at: 0 }));
    let c = compose(&ComposeSpec { template: Some("X={a};".into()), fields: vec![field("b", 3, 0, FieldType::U16)], len: None, pad: 0 });
    assert_eq!(c.render(&values(&[("a", "12"), ("b", "7")]), Endian::Little), Err(ComposeError::Overlap { earlier: "a".into(), later: "b".into(), at: 3 }));
    assert_eq!(c.render(&values(&[("a", "1"), ("b", "7")]), Endian::Little).unwrap(), [b'X', b'=', b'1', 7, 0], "a shorter value leaves the field clear");
    // Over the length by what the values bring: refused whole, never truncated.
    let c = compose(&ComposeSpec { template: Some("{a}{b}".into()), len: Some(8), ..Default::default() });
    assert_eq!(c.render(&values(&[("a", "ABCDE"), ("b", "FGHI")]), Endian::Little), Err(ComposeError::TooLong { len: 9, max: 8 }));
    assert_eq!(c.render(&values(&[("a", "ABCDE"), ("b", "FGH")]), Endian::Little).unwrap(), b"ABCDEFGH");
    let c = compose(&ComposeSpec { fields: vec![field("n", 6, 0, FieldType::U16)], len: Some(8), ..Default::default() });
    assert_eq!(c.render(&values(&[("n", "1")]), Endian::Little).unwrap(), [0, 0, 0, 0, 0, 0, 1, 0], "at the edge");
    // A length of zero: an empty body, and anything written refused.
    let c = compose(&ComposeSpec { len: Some(0), ..Default::default() });
    assert_eq!(c.render(&HashMap::new(), Endian::Little).unwrap(), Vec::<u8>::new());
    let c = compose(&ComposeSpec { template: Some("{x}".into()), len: Some(0), ..Default::default() });
    assert_eq!(c.render(&values(&[("x", "x")]), Endian::Little), Err(ComposeError::TooLong { len: 1, max: 0 }));
    assert_eq!(ComposeError::TooLong { len: 9, max: 8 }.to_string(), "the body would be 9 bytes, over its 8");
    assert!(!compose(&ComposeSpec::default()).writes() && compose(&ComposeSpec { template: Some("x".into()), ..Default::default() }).writes());
}

#[test]
fn an_offset_past_the_ceiling_is_refused_not_allocated() {
    assert_eq!(MAX_BODY, 16 * 1024 * 1024);
    // A field past the ceiling is refused where the spec compiles, with or without a length;
    // one at its last byte writes a body of exactly the ceiling.
    let spec_at = |at: usize| Compose::new(&ComposeSpec { fields: vec![field("n", at, 0, FieldType::U8)], ..Default::default() });
    assert_eq!(spec_at(MAX_BODY).err(), Some(ComposeError::TooLong { len: MAX_BODY + 1, max: MAX_BODY }));
    assert_eq!(spec_at(usize::MAX).err(), Some(ComposeError::TooLong { len: usize::MAX, max: MAX_BODY }), "no wrap");
    assert_eq!(spec_at(MAX_BODY - 1).unwrap().render(&values(&[("n", "1")]), Endian::Little).map(|b| b.len()), Ok(MAX_BODY), "the last byte of the largest body");
    // Two fields at the top of the address space are not read as apart by a wrapped sum.
    let top = Compose::new(&ComposeSpec { fields: vec![field("a", usize::MAX - 1, 4, FieldType::Ascii), field("b", usize::MAX, 0, FieldType::U8)], ..Default::default() });
    assert_eq!(top.err(), Some(ComposeError::TooLong { len: usize::MAX, max: MAX_BODY }));
    // A template of literal text past the ceiling, with no length, refused at compile.
    let long_literal = "x".repeat(MAX_BODY + 1);
    assert_eq!(Compose::new(&ComposeSpec { template: Some(long_literal), ..Default::default() }).err(), Some(ComposeError::TooLong { len: MAX_BODY + 1, max: MAX_BODY }));
    // A declared length over the ceiling is refused where the spec compiles: a body would be
    // padded up to it.
    assert_eq!(Compose::new(&ComposeSpec { len: Some(MAX_BODY + 1), ..Default::default() }).err(), Some(ComposeError::TooLong { len: MAX_BODY + 1, max: MAX_BODY }));
    assert_eq!(Compose::new(&ComposeSpec { len: Some(usize::MAX), ..Default::default() }).err(), Some(ComposeError::TooLong { len: usize::MAX, max: MAX_BODY }));
    let c = compose(&ComposeSpec { fields: vec![field("n", MAX_BODY - 1, 0, FieldType::U8)], len: Some(MAX_BODY), ..Default::default() });
    assert_eq!(c.render(&values(&[("n", "1")]), Endian::Little).map(|b| b.len()), Ok(MAX_BODY), "a body of exactly the ceiling");
    // Text past the ceiling too.
    let c = compose(&ComposeSpec { template: Some("{t}".into()), ..Default::default() });
    let long = "x".repeat(MAX_BODY + 1);
    assert_eq!(c.render(&values(&[("t", &long)]), Endian::Little), Err(ComposeError::TooLong { len: MAX_BODY + 1, max: MAX_BODY }));
}

#[test]
fn what_compose_writes_capture_reads_back_through_a_template_and_a_payload() {
    // The text round trip: a template writes, a regex reads.
    let c = compose(&ComposeSpec { template: Some("ITEM={item};DEST={dest};RESULT={result}".into()), ..Default::default() });
    let capture = Capture::new(&CaptureSpec {
        payload: Some(r"^ITEM=(?P<item>\d+);DEST=(?P<dest>\d+);RESULT=(?P<result>\w+)$".into()),
        ..Default::default()
    });
    let v = values(&[("item", "7"), ("dest", "3"), ("result", "SORTED")]);
    let body = c.render(&v, Endian::Little).unwrap();
    assert_eq!(body, b"ITEM=7;DEST=3;RESULT=SORTED");
    assert_eq!(capture.extract(&body, Endian::Little).unwrap(), v);
    // A value that breaks the pattern writes fine and does not read back: the reader's rules
    // decide what a valid body is, and the writer never guesses for it.
    let v = values(&[("item", "7"), ("dest", "x"), ("result", "SORTED")]);
    let body = c.render(&v, Endian::Little).unwrap();
    assert_eq!(capture.extract(&body, Endian::Little), None);
}

#[test]
fn a_spec_reads_from_toml_with_its_defaults_and_refuses_a_misspelled_key() {
    let spec: ComposeSpec = toml::from_str(
        r#"
template = "ITEM={item}"
fields = [ { name = "item", at = 0, ty = "U32" }, { name = "code", at = 4, len = 8, ty = "Ascii" } ]
"#,
    )
    .unwrap();
    assert_eq!(spec.template.as_deref(), Some("ITEM={item}"));
    assert_eq!(spec.fields, vec![field("item", 0, 0, FieldType::U32), field("code", 4, 8, FieldType::Ascii)]);
    assert_eq!((spec.len, spec.pad), (None, 0));
    let full: ComposeSpec = toml::from_str("len = 16\npad = 32\n").unwrap();
    assert_eq!((full.len, full.pad, full.template, full.fields.len()), (Some(16), 32, None, 0));
    assert_eq!(toml::from_str::<ComposeSpec>("").unwrap(), ComposeSpec::default());
    for (bad, key) in [
        ("templat = \"x\"", "templat"),
        ("fields = [ { name = \"a\", at = 0, ty = \"U8\", length = 1 } ]", "length"),
        ("fields = [ { name = \"a\", at = 0, ty = \"u8\" } ]", "u8"),
        ("len = -1", "-1"),
        ("pad = 256", "256"),
    ] {
        let err = toml::from_str::<ComposeSpec>(bad).unwrap_err().to_string();
        assert!(err.contains(key), "{key}: {err}");
    }
}
