//! The catalog file (contract: the catalog clause): read, versioned, validated, and documented by
//! a worked example this test loads from `docs/catalog-schema.md` itself.

use std::collections::HashMap;

use veredas_core::telegram::Endian;
use veredas_core::telegram::capture::FieldType;
use veredas_core::telegram::catalog::*;
use veredas_core::telegram::compose::MAX_BODY;
use veredas_core::telegram::framing::parse_hex;

/// The schema's documentation, as text.
fn doc() -> String {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/catalog-schema.md");
    std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n")
}

/// The first ```toml block of the schema's documentation.
fn documented_example() -> String {
    let text = doc();
    let start = text.find("```toml\n").unwrap();
    let body = &text[start..];
    let body = &body[body.find('\n').unwrap() + 1..];
    body[..body.find("```").unwrap()].to_string()
}

/// The documentation's prose, its lines joined, so a phrase wrapped across lines still reads.
fn doc_prose() -> String {
    doc().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The backticked text that follows `lead` in the prose.
fn quoted_after(prose: &str, lead: &str) -> String {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = &prose[at + lead.len()..];
    let rest = &rest[rest.find('`').unwrap() + 1..];
    rest[..rest.find('`').unwrap()].to_string()
}

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn file(body: &str) -> String {
    format!("format = \"veredas-catalog\"\nversion = 1\nid_field = \"id\"\n{body}")
}

fn refused(text: &str) -> String {
    match load(text) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_documented_example_loads_and_converses_as_described() {
    let catalog = load(&documented_example()).unwrap();
    assert_eq!(catalog.id_field, "id");
    assert_eq!(catalog.byte_order, Endian::Little);
    let shape: Vec<(&str, u64)> = catalog.messages.iter().map(|m| (m.name.as_str(), m.id)).collect();
    assert_eq!(shape, [("announce", 21), ("answer", 22), ("outcome", 23)]);
    let prose = doc_prose();
    // The ids are the channel file's worked example's segment ids: the prose says so, and the
    // channel schema's documentation carries that example with its id field.
    assert!(prose.contains("the segment ids of the channel file's worked example"));
    let channel = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/channel-schema.md")).unwrap();
    assert!(channel.contains("{ name = \"id\", size = 2 }"), "the channel example's segment id field is named `id`");

    // The announcement's shape: 32 bytes, its fields where the prose puts them.
    let announce = catalog.by_name("announce").unwrap();
    assert_eq!(announce.compose_spec.len, Some(32));
    let fields: Vec<(&str, usize, usize, FieldType)> =
        announce.compose_spec.fields.iter().map(|f| (f.name.as_str(), f.at, f.len, f.ty)).collect();
    assert_eq!(
        fields,
        [
            ("item", 0, 0, FieldType::U32),
            ("lane", 4, 0, FieldType::U8),
            ("read", 5, 1, FieldType::Ascii),
            ("code", 8, 16, FieldType::Ascii),
            ("weight", 24, 0, FieldType::U16),
        ]
    );
    assert_eq!(announce.capture_spec.fields, announce.compose_spec.fields, "one field list, both ways");
    assert_eq!(announce.names(), ["item", "lane", "read", "code", "weight"]);
    let v = values(&[("item", "7"), ("lane", "2"), ("read", "A"), ("code", "PKG7"), ("weight", "1250")]);
    let body = announce.write(&v, catalog.byte_order).unwrap();
    assert_eq!(body.len(), 32);
    assert_eq!(&body[..8], [7, 0, 0, 0, 2, b'A', 0, 0]);
    assert_eq!(&body[8..12], b"PKG7");
    assert_eq!(&body[24..26], [0xE2, 0x04]);
    assert_eq!(announce.read(&body, catalog.byte_order).unwrap(), v);

    // The answer's golden bytes, read from the prose.
    let golden = parse_hex(&quoted_after(&prose, "The `answer` for item 7 to destination 3 with flag 1 is")).unwrap();
    assert_eq!(golden, [7, 0, 0, 0, 3, 0, 1, 0]);
    let (id, body) = catalog.encode("answer", &values(&[("item", "7"), ("dest", "3"), ("flag", "1")])).unwrap().unwrap();
    assert_eq!((id, body.as_slice()), (22, golden.as_slice()));
    let (message, fields) = catalog.decode(22, &golden).unwrap();
    assert_eq!(message.name, "answer");
    assert_eq!(fields, values(&[("item", "7"), ("dest", "3"), ("flag", "1")]));

    // The outcome's text, read from the prose.
    let text = quoted_after(&prose, "with result `SORTED` is the text");
    assert_eq!(text, "ITEM=7;DEST=3;RESULT=SORTED");
    let (id, body) = catalog.encode("outcome", &values(&[("item", "7"), ("dest", "3"), ("result", "SORTED")])).unwrap().unwrap();
    assert_eq!((id, String::from_utf8(body.clone()).unwrap()), (23, text));
    let (message, fields) = catalog.decode(23, &body).unwrap();
    assert_eq!(message.name, "outcome");
    assert_eq!(fields, values(&[("item", "7"), ("dest", "3"), ("result", "SORTED")]));
    assert_eq!(catalog.by_name("outcome").unwrap().compose_spec.len, None, "as long as what was written");

    // What the catalog does not hold, or a body its rules refuse, is nothing.
    assert!(catalog.by_name("nope").is_none());
    assert!(catalog.by_id(24).is_none());
    assert!(catalog.encode("nope", &HashMap::new()).is_none());
    assert!(catalog.decode(24, &golden).is_none());
    assert!(catalog.decode(22, &golden[..7]).is_none(), "a short body, though its fields would read");
    let mut long = golden.clone();
    long.push(0);
    assert!(catalog.decode(22, &long).is_none(), "a long body is not this message");
    assert!(catalog.decode(23, b"nonsense").is_none());
    assert!(catalog.decode(23, b"ITEM=1;DEST=2;RESULT=X and more").is_none(), "the pattern is anchored");
    assert!(prose.contains("a received body of another length is not this message and reads as nothing"));
    // A write missing a value is a refusal through the catalog too.
    let err = catalog.encode("answer", &values(&[("item", "7")])).unwrap().unwrap_err();
    assert_eq!(err.to_string(), "no value for `dest`");
    // A big-endian catalog writes the same answer the other way round.
    let big = load(&documented_example().replace("byte_order = \"little\"", "byte_order = \"big\"")).unwrap();
    assert_eq!(big.byte_order, Endian::Big);
    let (_, body) = big.encode("answer", &values(&[("item", "7"), ("dest", "3"), ("flag", "1")])).unwrap().unwrap();
    assert_eq!(body, [0, 0, 0, 7, 0, 3, 1, 0]);
    assert_eq!(big.decode(22, &body).unwrap().1["item"], "7");
}

#[test]
fn every_default_and_every_key_the_reference_states() {
    let prose = doc_prose();
    // The reference's statements about the header and the defaults, each checked against the
    // reader.
    assert!(prose.contains("`format` (required: always `\"veredas-catalog\"`)"));
    assert_eq!(FORMAT, "veredas-catalog");
    assert!(prose.contains("`version` (required: this reader knows `1`)"));
    assert_eq!(VERSION, 1);
    assert!(prose.contains("`byte_order` (`\"little\"`)"));
    let minimal = load(&file("")).unwrap();
    assert_eq!(minimal.byte_order, Endian::Little);
    assert!(minimal.messages.is_empty(), "a catalog may be empty");
    assert!(prose.contains("`pad` (0)"));
    assert!(prose.contains("`len` (none)"));
    let one = load(&file("[[message]]\nname = \"m\"\nid = 1\n")).unwrap();
    let m = one.by_name("m").unwrap();
    assert_eq!((m.compose_spec.len, m.compose_spec.pad), (None, 0));
    assert_eq!(m.compose_spec.template, None);
    assert_eq!(m.capture_spec.payload, None);
    assert!(m.capture_spec.repeat.is_empty() && m.compose_spec.fields.is_empty());
    assert_eq!(m.write(&HashMap::new(), Endian::Little).unwrap(), Vec::<u8>::new());
    assert_eq!(m.read(b"anything", Endian::Little).unwrap(), HashMap::new(), "no rules read nothing and refuse nothing");
    assert!(prose.contains("`max` (16)"));
    let r = load(&file("[[message]]\nname = \"m\"\nid = 1\nrepeat = [ { name = \"bc\", pattern = \"BC=(\\\\w+)\" } ]\n")).unwrap();
    assert_eq!(r.by_name("m").unwrap().capture_spec.repeat[0].max, 16);
    assert!(prose.contains("`len` (0; the byte count of an `Ascii` or `Hex` field"));
    // Every type name the reference lists loads; another spelling is refused.
    for ty in ["U8", "U16", "U32", "I16", "I32", "Ascii", "Hex"] {
        assert!(prose.contains(&format!("`{ty}`")), "{ty} is in the reference");
        load(&file(&format!("[[message]]\nname = \"m\"\nid = 1\nfields = [ {{ name = \"f\", at = 0, ty = \"{ty}\", len = 2 }} ]\n")))
            .unwrap_or_else(|e| panic!("{ty}: {e}"));
    }
    assert!(refused(&file("[[message]]\nname = \"m\"\nid = 1\nfields = [ { name = \"f\", at = 0, ty = \"u8\" } ]\n")).contains("u8"));
    // A big byte order loads, another word does not.
    assert_eq!(load(&file("byte_order = \"big\"\n")).unwrap().byte_order, Endian::Big);
    assert!(refused(&file("byte_order = \"middle\"\n")).contains("middle"));
}

#[test]
fn the_header_is_refused_four_ways() {
    assert_eq!(refused("id_field = \"id\"\n"), "no `format`: a catalog file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-catalog\"\nid_field = \"id\"\n"), "no `version`: a catalog file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-channel\"\nversion = 1\nid_field = \"id\"\n"), "`format` is `veredas-channel`, not `veredas-catalog`");
    assert_eq!(refused("format = \"veredas-catalog\"\nversion = 2\nid_field = \"id\"\n"), "version 2 of `veredas-catalog` is not known here (this reader knows 1)");
    // The header is judged before the body: a version 2 file with keys this reader does not
    // know is an unknown version, not a misspelling.
    assert!(refused("format = \"veredas-catalog\"\nversion = 2\nid_field = \"id\"\nnew_key = 1\n").starts_with("version 2"));
    assert!(refused("not toml at all [").starts_with("not a catalog file: "));
    assert!(matches!(load("format = \"x\"\nversion = 1\n"), Err(CatalogError::WrongFormat(f)) if f == "x"));
    assert!(matches!(load("format = \"veredas-catalog\"\nversion = 3\n"), Err(CatalogError::UnknownVersion(3))));
}

#[test]
fn a_misspelled_key_in_any_table_is_refused() {
    for (text, key) in [
        (file("id_fild = \"id\"\n"), "id_fild"),
        (file("byte_orde = \"big\"\n"), "byte_orde"),
        (file("[[message]]\nname = \"m\"\nid = 1\nlenght = 8\n"), "lenght"),
        (file("[[message]]\nname = \"m\"\nid = 1\nfields = [ { name = \"f\", at = 0, ty = \"U8\", width = 1 } ]\n"), "width"),
        (file("[[message]]\nname = \"m\"\nid = 1\nrepeat = [ { name = \"r\", pattern = \"x\", maximum = 1 } ]\n"), "maximum"),
        (file("[[messages]]\nname = \"m\"\nid = 1\n"), "messages"),
        (file("[[message]]\nname = \"m\"\n"), "id"),
        (file("[[message]]\nid = 1\n"), "name"),
        ("format = \"veredas-catalog\"\nversion = 1\n".to_string(), "id_field"),
    ] {
        let err = refused(&text);
        assert!(err.contains(key), "{key}: {err}");
    }
}

#[test]
fn every_invalid_catalog_is_refused_with_its_place_named() {
    assert_eq!(refused(&file("").replace("id_field = \"id\"", "id_field = \" \"")), "id_field is empty: name the frame field that carries a message's id");
    assert_eq!(refused(&file("[[message]]\nname = \" \"\nid = 1\n")), "message[0]: the message has no name");
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\n[[message]]\nname = \"m\"\nid = 2\n")),
        "message[1] (`m`): two messages are named `m`"
    );
    assert_eq!(
        refused(&file("[[message]]\nname = \"a\"\nid = 1\n[[message]]\nname = \"b\"\nid = 1\n")),
        "message[1] (`b`): two messages have the id 1"
    );
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\nfields = [ { name = \"\", at = 0, ty = \"U8\" } ]\n")),
        "message[0] (`m`): a field has no name"
    );
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\nfields = [ { name = \"f\", at = 0, ty = \"U8\" }, { name = \"f\", at = 1, ty = \"U8\" } ]\n")),
        "message[0] (`m`): two fields are named `f`"
    );
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\nlen = 8\nfields = [ { name = \"f\", at = 6, ty = \"U32\" } ]\n")),
        "message[0] (`m`).f: ends at byte 10, past the body's 8"
    );
    assert!(load(&file("[[message]]\nname = \"m\"\nid = 1\nlen = 8\nfields = [ { name = \"f\", at = 4, ty = \"U32\" } ]\n")).is_ok(), "at the edge");
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\nlen = 8\nfields = [ { name = \"t\", at = 4, len = 5, ty = \"Ascii\" } ]\n")),
        "message[0] (`m`).t: ends at byte 9, past the body's 8"
    );
    assert_eq!(
        refused(&file(&format!("[[message]]\nname = \"m\"\nid = 1\nfields = [ {{ name = \"f\", at = {MAX_BODY}, ty = \"U8\" }} ]\n"))),
        format!("message[0] (`m`).f: ends at byte {}, past the largest body", MAX_BODY + 1)
    );
    assert_eq!(
        refused(&file(&format!("[[message]]\nname = \"m\"\nid = 1\nfields = [ {{ name = \"f\", at = {}, ty = \"U8\" }} ]\n", usize::MAX))),
        "message[0] (`m`).f: the offset is beyond any body"
    );
    assert!(load(&file(&format!("[[message]]\nname = \"m\"\nid = 1\nfields = [ {{ name = \"f\", at = {}, ty = \"U8\" }} ]\n", MAX_BODY - 1))).is_ok());
    let bad_regex = refused(&file("[[message]]\nname = \"m\"\nid = 1\npayload = \"(unclosed\"\n"));
    assert!(bad_regex.starts_with("message[0] (`m`): payload pattern `(unclosed` is invalid and is ignored: "), "{bad_regex}");
    let bad_repeat = refused(&file("[[message]]\nname = \"m\"\nid = 1\nrepeat = [ { name = \"r\", pattern = \"[\" } ]\n"));
    assert!(bad_repeat.starts_with("message[0] (`m`): repeat pattern `[` is invalid and is ignored: "), "{bad_repeat}");
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\ntemplate = \"A={a\"\n")),
        "message[0] (`m`): the template has an unclosed `{` at character 2"
    );
    assert_eq!(
        refused(&file("[[message]]\nname = \"m\"\nid = 1\ntemplate = \"A={}\"\n")),
        "message[0] (`m`): the template has an empty placeholder at character 2"
    );
}
