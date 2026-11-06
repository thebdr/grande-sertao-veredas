//! The catalog file (contract: the catalog clause): read, versioned, validated, and documented by
//! a worked example this test loads from `docs/catalog-schema.md` itself, every number of its
//! prose and every key statement of its reference read from the text and held against the reader.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use veredas_core::telegram::Endian;
use veredas_core::telegram::capture::FieldType;
use veredas_core::telegram::catalog::*;
use veredas_core::telegram::compose::{ComposeError, MAX_BODY};
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

/// The number that follows `lead` in the prose.
fn number_after(prose: &str, lead: &str) -> usize {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = prose[at + lead.len()..].trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or_else(|_| panic!("no number after `{lead}`: {rest:.20}"))
}

/// The count that follows `lead` in the prose, as digits or as a word (`one` to `ten`).
fn count_after(prose: &str, lead: &str) -> usize {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let word = prose[at + lead.len()..].split_whitespace().next().unwrap_or("");
    let words = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    words
        .iter()
        .position(|w| *w == word)
        .or_else(|| word.parse().ok())
        .unwrap_or_else(|| panic!("no count after `{lead}`: `{word}`"))
}

/// A text as paragraphs, each on one line with its whitespace collapsed, the worked example's
/// TOML block left out (it is loaded, not quoted): what the reference and the copy this test
/// holds of it are compared as.
fn paragraphs(text: &str) -> Vec<String> {
    let mut text = text.replace("\r\n", "\n");
    if let Some(start) = text.find("```toml\n") {
        let end = start + text[start + 7..].find("```").unwrap() + 7 + 3;
        text.replace_range(start..end, "");
    }
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

/// The reference's tables: for each, the keys it lists with what it says of each (the text in
/// the parentheses right after the key).
fn documented_keys() -> BTreeMap<String, Vec<(String, String)>> {
    let text = doc();
    let keys = &text[text.find("## Keys").unwrap()..];
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut table = String::new();
    for line in keys.lines() {
        if let Some(rest) = line.strip_prefix("**") {
            table = rest[..rest.find("**").unwrap()].to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("- `") {
            let key = rest[..rest.find('`').unwrap()].to_string();
            let after = &rest[rest.find('`').unwrap() + 1..];
            let open = after.find('(').unwrap();
            let statement = after[open + 1..after.find(')').unwrap()].to_string();
            out.entry(table.clone()).or_default().push((key, statement));
        }
    }
    out
}

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn file(body: &str) -> String {
    format!("format = \"veredas-catalog\"\nversion = 1\nid_field = \"id\"\n{body}")
}

fn message(body: &str) -> String {
    file(&format!("[[message]]\nname = \"m\"\nid = 1\n{body}"))
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
    let prose = doc_prose();
    assert_eq!(catalog.id_field, "id");
    // The prose's numbers and words, read from the text.
    let order = quoted_after(&prose, "Bytes are");
    assert_eq!(order, "little-endian");
    assert_eq!(catalog.byte_order, Endian::Little);
    let announce_len = number_after(&prose, "a binary body of");
    let code_len = number_after(&prose, "a code of up to");
    let read_len = count_after(&prose, "a read flag as");
    let answer_len = number_after(&prose, "answers with a destination (");
    let outcome_fields = count_after(&prose, "and reads back into those");
    assert_eq!((announce_len, code_len, read_len, answer_len, outcome_fields), (32, 16, 1, 8, 3));
    assert!(prose.contains("`id_field` (required): the name of the frame field that carries a message's id: a segment header field in a layout with segments, an envelope field in one without."));
    let shape: Vec<(&str, u64)> = catalog.messages.iter().map(|m| (m.name.as_str(), m.id)).collect();
    assert_eq!(shape, [("announce", 21), ("answer", 22), ("outcome", 23)]);
    // The ids ride the channel file's worked example's segment `id` field: the prose says so,
    // and the channel schema's documentation names that field.
    assert!(prose.contains("The ids are carried in the `id` field of the channel file's worked example's segment header"));
    let channel = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/channel-schema.md")).unwrap();
    assert!(channel.contains("{ name = \"id\", size = 2 }"), "the channel example's segment id field is named `id`");

    // The announcement's shape: the prose's length, its fields where the prose puts them.
    let announce = catalog.by_name("announce").unwrap();
    assert_eq!(announce.compose_spec.len, Some(announce_len));
    let fields: Vec<(&str, usize, usize, FieldType)> =
        announce.compose_spec.fields.iter().map(|f| (f.name.as_str(), f.at, f.len, f.ty)).collect();
    assert_eq!(
        fields,
        [
            ("item", 0, 0, FieldType::U32),
            ("lane", 4, 0, FieldType::U8),
            ("read", 5, read_len, FieldType::Ascii),
            ("code", 8, code_len, FieldType::Ascii),
            ("weight", 24, 0, FieldType::U16),
        ]
    );
    assert_eq!(announce.capture_spec.fields, announce.compose_spec.fields, "one field list, both ways");
    assert_eq!(announce.names(), ["item", "lane", "read", "code", "weight"]);
    let v = values(&[("item", "7"), ("lane", "2"), ("read", "A"), ("code", "PKG7"), ("weight", "1250")]);
    let body = announce.write(&v, catalog.byte_order).unwrap();
    assert_eq!(body.len(), announce_len);
    assert_eq!(&body[..8], [7, 0, 0, 0, 2, b'A', 0, 0]);
    assert_eq!(&body[8..12], b"PKG7");
    assert_eq!(&body[24..26], [0xE2, 0x04]);
    assert_eq!(announce.read(&body, catalog.byte_order).unwrap(), v);

    // The answer's golden bytes, read from the prose.
    let golden = parse_hex(&quoted_after(&prose, "The `answer` for item 7 to destination 3 with flag 1 is")).unwrap();
    assert_eq!(golden, [7, 0, 0, 0, 3, 0, 1, 0]);
    assert_eq!(golden.len(), answer_len);
    let answer = catalog.by_name("answer").unwrap();
    assert_eq!(answer.compose_spec.len, Some(answer_len));
    let fields: Vec<(&str, usize, usize, FieldType)> = answer.compose_spec.fields.iter().map(|f| (f.name.as_str(), f.at, f.len, f.ty)).collect();
    assert_eq!(fields, [("item", 0, 0, FieldType::U32), ("dest", 4, 0, FieldType::U16), ("flag", 6, 0, FieldType::U8)]);
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
    assert_eq!(fields.len(), outcome_fields, "those three fields, as the prose counts them");
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
    assert!(catalog.decode(23, b"ITEM=1;DEST=2;RESULT=X and more").is_none(), "the pattern is anchored at the end");
    assert!(catalog.decode(23, b"xxITEM=1;DEST=2;RESULT=X").is_none(), "and at the start");
    assert!(catalog.decode(23, b"ITEM=;DEST=2;RESULT=X").is_none(), "an item needs its digits");
    assert_eq!(catalog.decode(23, b"ITEM=1;DEST=2;RESULT=x_9").unwrap().1["result"], "x_9", "a result is a word");
    assert!(prose.contains("a received body of another length is not this message and reads as nothing"));
    // A write missing a value is a refusal through the catalog too.
    let err = catalog.encode("answer", &values(&[("item", "7")])).unwrap().unwrap_err();
    assert_eq!(err.to_string(), "no value for `dest`");
    // A big-endian catalog writes the same answer the other way round: every binary field of
    // the body, as the reference says.
    assert!(prose.contains("`byte_order` (`\"little\"`): `\"little\"` or `\"big\"`, for every binary field of every body."));
    let big = load(&documented_example().replace("byte_order = \"little\"", "byte_order = \"big\"")).unwrap();
    assert_eq!(big.byte_order, Endian::Big);
    let (_, body) = big.encode("answer", &values(&[("item", "7"), ("dest", "3"), ("flag", "1")])).unwrap().unwrap();
    assert_eq!(body, [0, 0, 0, 7, 0, 3, 1, 0]);
    assert_eq!(big.decode(22, &body).unwrap().1, values(&[("item", "7"), ("dest", "3"), ("flag", "1")]));
    let (_, body) = big.encode("announce", &v).unwrap().unwrap();
    assert_eq!((&body[..4], &body[24..26]), (&[0, 0, 0, 7][..], &[0x04, 0xE2][..]), "both binary fields of the announcement, big-endian");
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    // The reference, paragraph by paragraph, is the copy below: a sentence changed, added or
    // dropped anywhere in it (a default stated a second time, a clause appended after a pinned
    // sentence) fails here, so every edit to the reference is an edit to this test, beside the
    // behaviour the sentence is held against.
    let actual = paragraphs(&doc());
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/catalog-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len(), "docs/catalog-schema.md has {} paragraphs, the copy {}", actual.len(), expected.len());
}

/// The keys of a table as the reader lists them in its refusal of a misspelled one.
fn reader_keys(text_with_bad_key: &str) -> Vec<String> {
    let err = refused(text_with_bad_key);
    let at = err.find("expected one of ").unwrap_or_else(|| panic!("{err}"));
    let rest = &err[at + "expected one of ".len()..];
    let rest = rest.lines().next().unwrap();
    rest.split(", ").map(|k| k.trim().trim_matches('`').to_string()).filter(|k| !k.is_empty()).collect()
}

#[test]
fn every_key_the_reference_lists_is_the_reader_s_and_states_what_the_reader_does() {
    let documented = documented_keys();
    assert_eq!(
        documented.keys().cloned().collect::<Vec<_>>(),
        ["A field", "A message", "A repeat", "The top level"],
        "the reference's four tables"
    );
    // Each table's documented keys are exactly the reader's.
    let sorted = |v: Vec<String>| {
        let mut v = v;
        v.sort();
        v
    };
    let names = |table: &str| sorted(documented[table].iter().map(|(k, _)| k.clone()).collect());
    assert_eq!(names("The top level"), sorted(reader_keys(&file("zz = 1\n"))));
    assert_eq!(names("A message"), sorted(reader_keys(&message("zz = 1\n"))));
    assert_eq!(names("A field"), sorted(reader_keys(&message("fields = [ { name = \"f\", at = 0, ty = \"U8\", zz = 1 } ]\n"))));
    assert_eq!(names("A repeat"), sorted(reader_keys(&message("repeat = [ { name = \"r\", pattern = \"x\", zz = 1 } ]\n"))));

    // Each key's statement against the reader: required, or loading with the stated default.
    let full_field = "{ name = \"f\", at = 0, ty = \"U8\", len = 0 }";
    let full_repeat = "{ name = \"r\", pattern = \"R=(\\\\w+)\", max = 16 }";
    let full_message = format!("[[message]]\nname = \"m\"\nid = 1\nlen = 8\npad = 0\nfields = [ {full_field} ]\ntemplate = \"T\"\npayload = \"T\"\nrepeat = [ {full_repeat} ]\n");
    let full_top = format!("format = \"veredas-catalog\"\nversion = 1\nid_field = \"id\"\nbyte_order = \"little\"\n{full_message}");
    assert!(load(&full_top).is_ok(), "{}", refused(&full_top));
    let without = |text: &str, line_starting: &str| -> String {
        let out: Vec<&str> = text.lines().filter(|l| !l.starts_with(line_starting)).collect();
        assert!(out.len() + 1 == text.lines().count(), "no line of {text} starts with {line_starting}");
        out.join("\n") + "\n"
    };
    let without_inline = |text: &str, line_prefix: &str, key: &str| -> String {
        // Drop `key = value` from the inline table on the line of a field or a repeat, whether
        // the key sits first (`{ key = v, `) or later (`, key = v`).
        let later = regex::Regex::new(&format!(r#", {key} = ("[^"]*"|\w+)"#)).unwrap();
        let first = regex::Regex::new(&format!(r#"\{{ {key} = ("[^"]*"|\w+), "#)).unwrap();
        let lines: Vec<String> = text
            .lines()
            .map(|l| {
                if l.starts_with(line_prefix) {
                    let out = later.replace(l, "").to_string();
                    let out = first.replace(&out, "{ ").to_string();
                    assert_ne!(out, l, "no `{key}` to drop in {l}");
                    out
                } else {
                    l.to_string()
                }
            })
            .collect();
        lines.join("\n") + "\n"
    };
    for (table, keys) in &documented {
        for (key, statement) in keys {
            let required = statement.starts_with("required");
            let text = match (table.as_str(), key.as_str()) {
                // No `[[message]]` at all: the header alone.
                ("The top level", "message") => full_top.lines().take(4).collect::<Vec<_>>().join("\n") + "\n",
                ("The top level", _) | ("A message", _) => without(&full_top, &format!("{key} =")),
                ("A field", _) => without_inline(&full_top, "fields =", key),
                ("A repeat", _) => without_inline(&full_top, "repeat =", key),
                _ => unreachable!(),
            };
            if required {
                let err = refused(&text);
                let expected = match key.as_str() {
                    "format" => "no `format`: a catalog file states `format` and `version`".to_string(),
                    "version" => "no `version`: a catalog file states `format` and `version`".to_string(),
                    _ => format!("missing field `{key}`"),
                };
                assert!(err.contains(&expected), "{table}.{key} is documented required ({statement}): {err}");
            } else {
                let catalog = load(&text).unwrap_or_else(|e| panic!("{table}.{key} documented as `{statement}` but refused when left out: {e}"));
                let m = catalog.by_name("m");
                // The stated default, as the reader holds it.
                let default = statement.split(';').next().unwrap().trim().trim_matches('`').to_string();
                match (table.as_str(), key.as_str()) {
                    ("The top level", "byte_order") => {
                        assert_eq!(default, "\"little\"");
                        assert_eq!(catalog.byte_order, Endian::Little);
                    }
                    ("The top level", "message") => {
                        assert_eq!(default, "none");
                        assert!(catalog.messages.is_empty());
                    }
                    ("A message", "len") => {
                        assert_eq!(default, "none");
                        assert_eq!(m.unwrap().compose_spec.len, None);
                    }
                    ("A message", "pad") => {
                        assert_eq!(default, "0");
                        assert_eq!(m.unwrap().compose_spec.pad, 0);
                    }
                    ("A message", "fields") => {
                        assert_eq!(default, "none");
                        assert!(m.unwrap().compose_spec.fields.is_empty() && m.unwrap().capture_spec.fields.is_empty());
                    }
                    ("A message", "template") => {
                        assert_eq!(default, "none");
                        assert_eq!(m.unwrap().compose_spec.template, None);
                    }
                    ("A message", "payload") => {
                        assert_eq!(default, "none");
                        assert_eq!(m.unwrap().capture_spec.payload, None);
                    }
                    ("A message", "repeat") => {
                        assert_eq!(default, "none");
                        assert!(m.unwrap().capture_spec.repeat.is_empty());
                    }
                    ("A field", "len") => {
                        assert_eq!(default, "0");
                        assert_eq!(m.unwrap().compose_spec.fields[0].len, 0);
                    }
                    ("A repeat", "max") => {
                        assert_eq!(default, "16");
                        assert_eq!(m.unwrap().capture_spec.repeat[0].max, 16);
                    }
                    other => panic!("{other:?}: a documented default this test does not know how to check"),
                }
            }
        }
    }
    // The reference's own rules about keys and names, and the constants it states.
    let prose = doc_prose();
    assert!(prose.contains("Each key states in parentheses that it is required, or the value it takes when left out."));
    assert!(prose.contains("A name (of a message, a field, a repeat) is exact: compared as written, with no surrounding whitespace."));
    assert!(prose.contains("`message` (none): the messages, as `[[message]]` tables; a catalog may be empty."));
    assert!(load(&file("")).unwrap().messages.is_empty());
    assert!(prose.contains("`format` (required): always `\"veredas-catalog\"`"));
    assert_eq!(FORMAT, "veredas-catalog");
    assert!(prose.contains("`version` (required): this reader knows `1`"));
    assert_eq!(VERSION, 1);
    // Every type name the reference lists loads; another spelling is refused.
    for ty in ["U8", "U16", "U32", "I16", "I32", "Ascii", "Hex"] {
        assert!(prose.contains(&format!("`{ty}`")), "{ty} is in the reference");
        load(&message(&format!("fields = [ {{ name = \"f\", at = 0, ty = \"{ty}\", len = 2 }} ]\n"))).unwrap_or_else(|e| panic!("{ty}: {e}"));
    }
    assert!(refused(&message("fields = [ { name = \"f\", at = 0, ty = \"u8\" } ]\n")).contains("u8"));
    assert_eq!(load(&file("byte_order = \"big\"\n")).unwrap().byte_order, Endian::Big);
    assert!(refused(&file("byte_order = \"middle\"\n")).contains("middle"));
    // A message with no rules reads an empty map and writes an empty body.
    let bare = load(&message("")).unwrap();
    let m = bare.by_name("m").unwrap();
    assert_eq!(m.write(&HashMap::new(), Endian::Little).unwrap(), Vec::<u8>::new());
    assert_eq!(m.read(b"anything", Endian::Little).unwrap(), HashMap::new(), "no rules read nothing and refuse nothing");
}

#[test]
fn the_header_is_refused_four_ways() {
    assert!(doc_prose().contains("A file of another format or an unknown version is refused, never guessed; migrating an older version is portal's (P-010)."));
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
    assert!(matches!(load("format = \"veredas-catalog\"\nversion = 0\nid_field = \"id\"\n"), Err(CatalogError::UnknownVersion(0))));
    assert!(refused("format = \"veredas-catalog \"\nversion = 1\nid_field = \"id\"\n").starts_with("`format` is `veredas-catalog `"));
    assert!(refused("format = \"veredas-catalog\"\nversion = 1.0\nid_field = \"id\"\n").starts_with("not a catalog file"));
}

#[test]
fn a_misspelled_key_in_any_table_is_refused() {
    for (text, key) in [
        (file("id_fild = \"id\"\n"), "id_fild"),
        (file("byte_orde = \"big\"\n"), "byte_orde"),
        (message("lenght = 8\n"), "lenght"),
        (message("fields = [ { name = \"f\", at = 0, ty = \"U8\", width = 1 } ]\n"), "width"),
        (message("repeat = [ { name = \"r\", pattern = \"x\", maximum = 1 } ]\n"), "maximum"),
        (file("[[messages]]\nname = \"m\"\nid = 1\n"), "messages"),
        (file("[[message]]\nname = \"m\"\n"), "id"),
        (file("[[message]]\nid = 1\n"), "name"),
        ("format = \"veredas-catalog\"\nversion = 1\n".to_string(), "id_field"),
        (message("fields = [ { name = \"f\", ty = \"U8\" } ]\n"), "at"),
        (message("fields = [ { name = \"f\", at = 0 } ]\n"), "ty"),
        (message("repeat = [ { pattern = \"x\" } ]\n"), "name"),
        (message("repeat = [ { name = \"r\" } ]\n"), "pattern"),
    ] {
        let err = refused(&text);
        assert!(err.contains(key), "{key}: {err}");
    }
}

#[test]
fn every_invalid_catalog_is_refused_with_its_place_named() {
    assert_eq!(refused(&file("").replace("id_field = \"id\"", "id_field = \" \"")), "id_field is empty: name the frame field that carries a message's id");
    assert_eq!(refused(&file("[[message]]\nname = \" \"\nid = 1\n")), "message[0]: the message has no name");
    assert_eq!(refused(&file("[[message]]\nname = \" m\"\nid = 1\n")), "message[0]: the name ` m` has surrounding whitespace");
    assert_eq!(refused(&file("[[message]]\nname = \"m\"\nid = 1\n[[message]]\nname = \"m\"\nid = 2\n")), "message[1] (`m`): two messages are named `m`");
    assert!(load(&file("[[message]]\nname = \"m\"\nid = 1\n[[message]]\nname = \"M\"\nid = 2\n")).is_ok(), "names are exact: a case apart is another name");
    assert_eq!(refused(&file("[[message]]\nname = \"a\"\nid = 1\n[[message]]\nname = \"b\"\nid = 1\n")), "message[1] (`b`): two messages have the id 1");
    assert_eq!(refused(&message("fields = [ { name = \"\", at = 0, ty = \"U8\" } ]\n")), "message[0] (`m`): a field has no name");
    assert_eq!(refused(&message("fields = [ { name = \" \", at = 0, ty = \"U8\" } ]\n")), "message[0] (`m`): a field has no name");
    assert_eq!(refused(&message("fields = [ { name = \"f \", at = 0, ty = \"U8\" } ]\n")), "message[0] (`m`): the name `f ` has surrounding whitespace");
    assert_eq!(
        refused(&message("fields = [ { name = \"f\", at = 0, ty = \"U8\" }, { name = \"f\", at = 1, ty = \"U8\" } ]\n")),
        "message[0] (`m`): two fields are named `f`"
    );
    // Past the body, for every type, with the field ending at the edge accepted.
    for (ty, width) in [("U8", 1), ("U16", 2), ("U32", 4), ("I16", 2), ("I32", 4), ("Ascii", 3), ("Hex", 3)] {
        let len = if width == 3 { ", len = 3" } else { "" };
        let edge = message(&format!("len = 8\nfields = [ {{ name = \"f\", at = {}, ty = \"{ty}\"{len} }} ]\n", 8 - width));
        assert!(load(&edge).is_ok(), "{ty} ending at the edge: {}", refused(&edge));
        let past = message(&format!("len = 8\nfields = [ {{ name = \"f\", at = {}, ty = \"{ty}\"{len} }} ]\n", 9 - width));
        assert_eq!(refused(&past), "message[0] (`m`).f: ends at byte 9, past the body's 8", "{ty}");
    }
    assert_eq!(
        refused(&message(&format!("fields = [ {{ name = \"f\", at = {MAX_BODY}, ty = \"U8\" }} ]\n"))),
        format!("message[0] (`m`).f: ends at byte {}, past the largest body", MAX_BODY + 1)
    );
    assert_eq!(
        refused(&message(&format!("fields = [ {{ name = \"f\", at = {}, ty = \"U8\" }} ]\n", usize::MAX))),
        "message[0] (`m`).f: the offset is beyond any body"
    );
    assert!(load(&message(&format!("fields = [ {{ name = \"f\", at = {}, ty = \"U8\" }} ]\n", MAX_BODY - 1))).is_ok());
    assert_eq!(refused(&message(&format!("len = {}\n", MAX_BODY + 1))), format!("message[0] (`m`): the body would be {} bytes, over its {MAX_BODY}", MAX_BODY + 1));
    // The rules compose and capture refuse, with the message named.
    let bad_regex = refused(&message("payload = \"(unclosed\"\n"));
    assert!(bad_regex.starts_with("message[0] (`m`): payload pattern `(unclosed` is invalid and is ignored: "), "{bad_regex}");
    let bad_repeat = refused(&message("repeat = [ { name = \"r\", pattern = \"[\" } ]\n"));
    assert!(bad_repeat.starts_with("message[0] (`m`): repeat pattern `[` is invalid and is ignored: "), "{bad_repeat}");
    assert_eq!(refused(&message("template = \"A={a\"\n")), "message[0] (`m`): the template has an unclosed `{` at character 2");
    assert_eq!(refused(&message("template = \"A={}\"\n")), "message[0] (`m`): the template has an empty placeholder at character 2");
    assert_eq!(refused(&message("len = 2\ntemplate = \"ABCDEF\"\n")), "message[0] (`m`): the body would be 6 bytes, over its 2");
    assert_eq!(refused(&message("len = 0\ntemplate = \"A\"\n")), "message[0] (`m`): the body would be 1 bytes, over its 0");
    assert_eq!(refused(&message("fields = [ { name = \"t\", at = 0, ty = \"Ascii\" } ]\n")), "message[0] (`m`): `t`: a text or hex field declares its length");
    assert_eq!(refused(&message("fields = [ { name = \"h\", at = 0, ty = \"Hex\" } ]\n")), "message[0] (`m`): `h`: a text or hex field declares its length");
    assert_eq!(
        refused(&message("fields = [ { name = \"a\", at = 0, ty = \"U16\" }, { name = \"b\", at = 1, ty = \"U8\" } ]\n")),
        "message[0] (`m`): `b` overlaps `a` at byte 1"
    );
    // Repeats and names that would shadow one another.
    assert_eq!(refused(&message("repeat = [ { name = \"r\", pattern = \"x\", max = 0 } ]\n")), "message[0] (`m`).repeat[0] (`r`): max is 0");
    assert_eq!(refused(&message("repeat = [ { name = \" r\", pattern = \"x\" } ]\n")), "message[0] (`m`).repeat[0]: the name ` r` has surrounding whitespace");
    assert_eq!(refused(&message("repeat = [ { name = \"r\", pattern = \"x\" }, { name = \"r\", pattern = \"y\" } ]\n")), "message[0] (`m`): two repeats are named `r`");
    assert_eq!(
        refused(&message("repeat = [ { name = \"a\", pattern = \"(x)\" } ]\nfields = [ { name = \"a1\", at = 0, ty = \"U8\" } ]\n")),
        "message[0] (`m`).a1: the repeat `a` reads a field of that name"
    );
    assert!(load(&message("repeat = [ { name = \"a\", pattern = \"(x)\", max = 2 } ]\nfields = [ { name = \"a3\", at = 0, ty = \"U8\" } ]\n")).is_ok(), "beyond the repeat's max");
    // Whether a name is a repeat's numbered name is decided by parsing it, never by listing
    // the repeat's names: a `max` in the millions loads at once, and the decision is the same.
    let started = Instant::now();
    let millions = "repeat = [ { name = \"a\", pattern = \"x\", max = 1000000 }, { name = \"b\", pattern = \"y\", max = 1000000 } ]\n";
    let big = load(&message(&format!("{millions}fields = [ {{ name = \"a1000001\", at = 0, ty = \"U8\" }} ]\n"))).unwrap();
    assert_eq!(big.by_name("m").unwrap().capture_spec.repeat[0].max, 1_000_000);
    assert_eq!(
        refused(&message(&format!("{millions}fields = [ {{ name = \"a1000000\", at = 0, ty = \"U8\" }} ]\n"))),
        "message[0] (`m`).a1000000: the repeat `a` reads a field of that name"
    );
    assert!(load(&message(&format!("{millions}fields = [ {{ name = \"a99999999999999999999\", at = 0, ty = \"U8\" }} ]\n"))).is_ok(), "beyond any max");
    assert!(started.elapsed() < Duration::from_secs(1), "three loads took {:?}", started.elapsed());
    // And at the top of a usize: a reader that quietly caps `max` fails here. A payload group
    // is judged the same way, at the repeat's max and one past it.
    let top = usize::MAX;
    let at_top = |field: &str| message(&format!("repeat = [ {{ name = \"a\", pattern = \"x\", max = {top} }} ]\nfields = [ {{ name = \"{field}\", at = 0, ty = \"U8\" }} ]\n"));
    assert_eq!(refused(&at_top(&format!("a{top}"))), format!("message[0] (`m`).a{top}: the repeat `a` reads a field of that name"));
    assert!(load(&at_top(&format!("a{}", top as u128 + 1))).is_ok(), "one past the top of a usize is no number");
    let group = |name: &str, max: usize| message(&format!("payload = \"(?P<{name}>x)\"\nrepeat = [ {{ name = \"a\", pattern = \"y\", max = {max} }} ]\n"));
    assert_eq!(refused(&group("a101", 101)), "message[0] (`m`).repeat[0] (`a`): the payload pattern reads a group named `a101`");
    assert!(load(&group("a102", 101)).is_ok(), "a group past the repeat's max is its own");
    assert_eq!(refused(&group(&format!("a{top}"), top)), format!("message[0] (`m`).repeat[0] (`a`): the payload pattern reads a group named `a{top}`"));
    // Two repeats meet when the longer name is the shorter followed by digits and the least
    // numbered name of the longer (`<long>1`) is within the shorter's max, in either order.
    assert_eq!(
        refused(&message("repeat = [ { name = \"a\", pattern = \"x\", max = 15 }, { name = \"a1\", pattern = \"y\", max = 3 } ]\n")),
        "message[0] (`m`).repeat[1] (`a1`): the repeat `a` reads `a11` too"
    );
    assert_eq!(
        refused(&message("repeat = [ { name = \"a1\", pattern = \"y\", max = 3 }, { name = \"a\", pattern = \"x\", max = 15 } ]\n")),
        "message[0] (`m`).repeat[1] (`a`): the repeat `a1` reads `a11` too"
    );
    assert!(load(&message("repeat = [ { name = \"a\", pattern = \"x\", max = 10 }, { name = \"a1\", pattern = \"y\", max = 3 } ]\n")).is_ok(), "a11 is past a's max");
    assert!(load(&message("repeat = [ { name = \"a\", pattern = \"x\" }, { name = \"a0\", pattern = \"y\" } ]\n")).is_ok(), "a01 has a leading zero: no name of a's");
    assert!(load(&message("repeat = [ { name = \"a\", pattern = \"x\" }, { name = \"ab\", pattern = \"y\" } ]\n")).is_ok(), "ab1 is no number of a's");
    assert_eq!(
        refused(&message("payload = \"(?P<x>\\\\d+)\"\nfields = [ { name = \"x\", at = 0, ty = \"U8\" } ]\n")),
        "message[0] (`m`).x: the payload pattern reads a group of that name"
    );
}

/// The backticked words of the sentence that follows `lead` in the prose, up to its full stop.
fn listed_after(prose: &str, lead: &str) -> Vec<String> {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = &prose[at + lead.len()..];
    let sentence = &rest[..rest.find(". ").unwrap_or(rest.len())];
    sentence.split('`').skip(1).step_by(2).map(String::from).collect()
}

/// The names a reader's refusal of an unknown variant lists after "expected".
fn variants_in(err: &str) -> Vec<String> {
    let at = err.find("expected").unwrap_or_else(|| panic!("{err}"));
    err[at..].split('`').skip(1).step_by(2).map(String::from).collect()
}

#[test]
fn every_rule_the_reference_states_is_the_reader_s() {
    // Each statement of the reference is quoted beside the behaviour that pins it, so neither
    // can drift from the other unnoticed.
    let prose = doc_prose();
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    // The sets the reference lists are the reader's own, no more and no fewer.
    let types = listed_after(&prose, "`ty` (required): ");
    assert_eq!(sorted(types), sorted(variants_in(&refused(&message("fields = [ { name = \"f\", at = 0, ty = \"u8\" } ]\n")))));
    let orders = listed_after(&prose, "`byte_order` (`\"little\"`): ");
    assert_eq!(sorted(orders.iter().map(|o| o.trim_matches('"').to_string()).collect()), sorted(variants_in(&refused(&file("byte_order = \"middle\"\n")))));
    // The ceiling the prose names is the engine's.
    let mib = number_after(&prose, "ending past the body or past");
    assert_eq!(mib * 1024 * 1024, MAX_BODY);
    assert!(prose.contains(&format!("past {mib} MiB")));

    // Names and ids are unique; a field's name is no reader's.
    assert!(prose.contains("`name` (required): what the rules and the bench call the message; unique."));
    assert!(prose.contains("`id` (required): the number the frame carries in `id_field`; unique."));
    assert!(refused(&file("[[message]]\nname = \"m\"\nid = 1\n[[message]]\nname = \"m\"\nid = 2\n")).contains("two messages are named"));
    assert!(refused(&file("[[message]]\nname = \"a\"\nid = 1\n[[message]]\nname = \"b\"\nid = 1\n")).contains("two messages have the id"));
    assert!(prose.contains("`name` (required): unique within the message, and never a name the payload's groups or a repeat's numbered names would read."));
    assert!(refused(&message("fields = [ { name = \"f\", at = 0, ty = \"U8\" }, { name = \"f\", at = 1, ty = \"U8\" } ]\n")).contains("two fields are named"));
    // The closing paragraph, whole: what writing needs and refuses, what reading yields when the
    // rules fail, and what loading refuses with its place named.
    assert!(prose.contains("Writing a body needs a value for every placeholder and every field, and refuses one that is missing or does not fit; reading a body that does not satisfy the rules yields nothing, never a partial map."));
    let both = load(&message("template = \"X={x}\"\nfields = [ { name = \"n\", at = 4, ty = \"U8\" } ]\npayload = \"^X=(?P<x>[a-z]+)\"\n")).unwrap();
    assert_eq!(both.encode("m", &values(&[("n", "1")])).unwrap().unwrap_err().to_string(), "no value for `x`");
    assert_eq!(both.encode("m", &values(&[("x", "a")])).unwrap().unwrap_err().to_string(), "no value for `n`");
    assert!(both.encode("m", &values(&[("x", "a"), ("n", "256")])).unwrap().unwrap_err().to_string().contains("is not a U8"));
    assert_eq!(both.decode(1, b"X=ab\x07").unwrap().1, values(&[("x", "ab"), ("n", "7")]));
    assert!(both.decode(1, b"X=9!\x07").is_none(), "the pattern fails: nothing, though the field would read");
    assert!(prose.contains(&format!(
        "A misspelled key in any table, an empty or repeated name, a repeated id, a field ending past the body or past {mib} MiB, a bad pattern or a bad template is refused with its place named."
    )));
    assert!(refused(&message("zz = 1\n")).contains("unknown field `zz`"));
    for text in [
        file("[[message]]\nname = \"\"\nid = 1\n"),
        file("[[message]]\nname = \"m\"\nid = 1\n[[message]]\nname = \"m\"\nid = 2\n"),
        file("[[message]]\nname = \"a\"\nid = 1\n[[message]]\nname = \"b\"\nid = 1\n"),
        message("len = 2\nfields = [ { name = \"f\", at = 1, ty = \"U16\" } ]\n"),
        message(&format!("fields = [ {{ name = \"f\", at = {MAX_BODY}, ty = \"U8\" }} ]\n")),
        message("payload = \"(\"\n"),
        message("repeat = [ { name = \"r\", pattern = \"(\" } ]\n"),
        message("template = \"{\"\n"),
    ] {
        assert!(refused(&text).starts_with("message["), "the place is named: {}", refused(&text));
    }
    // A text or hex field needs its length.
    assert!(prose.contains("`len` (`0`; required for an `Ascii` or `Hex` field): their byte count."));
    let len_rule = listed_after(&prose, "`len` (`0`; required for an ");
    assert_eq!(len_rule, ["Ascii", "Hex"]);
    for ty in &len_rule {
        assert!(refused(&message(&format!("fields = [ {{ name = \"f\", at = 0, ty = \"{ty}\" }} ]\n"))).contains("declares its length"), "{ty}");
    }
    // Fields never share a byte; a field written over the text overwrites its bytes.
    assert!(prose.contains("No two fields may share a byte."));
    assert!(refused(&message("fields = [ { name = \"a\", at = 0, ty = \"U16\" }, { name = \"b\", at = 1, ty = \"U8\" } ]\n")).contains("overlaps"));
    assert!(prose.contains("A field written over the text overwrites its bytes."));
    let over = load(&message("template = \"ABCD\"\nfields = [ { name = \"n\", at = 1, ty = \"U8\" } ]\n")).unwrap();
    assert_eq!(over.encode("m", &values(&[("n", "0")])).unwrap().unwrap().1, [b'A', 0, b'C', b'D']);
    // Padding, NUL, trailing whitespace, signs, hex, too long.
    assert!(prose.contains("Text is padded with NUL to it (a NUL inside the text refused) and refused when longer; hex must spell exactly that many bytes."));
    let text = load(&message("fields = [ { name = \"t\", at = 0, len = 4, ty = \"Ascii\" } ]\n")).unwrap();
    assert_eq!(text.encode("m", &values(&[("t", "AB")])).unwrap().unwrap().1, [b'A', b'B', 0, 0]);
    assert!(text.encode("m", &values(&[("t", "A\0B")])).unwrap().unwrap_err().to_string().contains("text without NUL"));
    assert!(text.encode("m", &values(&[("t", "ABCDE")])).unwrap().unwrap_err().to_string().contains("do not fit"));
    assert!(prose.contains("a text is read back without its trailing whitespace"));
    assert_eq!(text.decode(1, b"A  \0").unwrap().1["t"], "A");
    assert!(prose.contains("An integer is written from decimal text (a sign only where the type takes one) and refused when it does not fit"));
    let ints = load(&message("fields = [ { name = \"u\", at = 0, ty = \"U8\" }, { name = \"i\", at = 1, ty = \"I16\" } ]\n")).unwrap();
    assert!(ints.encode("m", &values(&[("u", "+1"), ("i", "1")])).unwrap().unwrap_err().to_string().contains("is not a U8"));
    assert!(ints.encode("m", &values(&[("u", "256"), ("i", "1")])).unwrap().unwrap_err().to_string().contains("is not a U8"));
    assert_eq!(ints.encode("m", &values(&[("u", "1"), ("i", "+1")])).unwrap().unwrap().1, [1, 1, 0]);
    // Fields at byte offsets, written and read alike.
    assert!(prose.contains("`fields` (none): binary fields at byte offsets, written and read alike."));
    assert!(prose.contains("`at` (required): the byte offset."));
    let (_, body) = ints.encode("m", &values(&[("u", "1"), ("i", "-2")])).unwrap().unwrap();
    assert_eq!(body, [1, 0xFE, 0xFF], "`u` at byte 0, `i` at byte 1");
    assert_eq!(ints.decode(1, &body).unwrap().1, values(&[("u", "1"), ("i", "-2")]));
    let hex = load(&message("fields = [ { name = \"h\", at = 0, len = 2, ty = \"Hex\" } ]\n")).unwrap();
    assert!(hex.encode("m", &values(&[("h", "DE")])).unwrap().unwrap_err().to_string().contains("1 hex bytes where 2 are declared"));
    assert!(prose.contains("`len` (none): a fixed body length. Written bodies are padded with `pad` up to it and refused beyond it"));
    let fixed = load(&message("len = 4\ntemplate = \"{t}\"\n")).unwrap();
    assert_eq!(fixed.encode("m", &values(&[("t", "AB")])).unwrap().unwrap().1, [b'A', b'B', 0, 0]);
    assert!(fixed.encode("m", &values(&[("t", "ABCDE")])).unwrap().unwrap_err().to_string().contains("over its 4"));
    assert!(prose.contains("a field that ends past it, or a template whose literal text is longer, is refused when the file loads"));
    assert!(refused(&message("len = 2\ntemplate = \"ABC\"\n")).contains("over its 2"));
    assert!(refused(&message("len = 2\nfields = [ { name = \"f\", at = 1, ty = \"U16\" } ]\n")).contains("past the body's 2"));
    assert!(prose.contains("Without it a body is as long as what was written, and read whatever its length."));
    let open = load(&message("template = \"{t}\"\npayload = \"(?P<t>.*)\"\n")).unwrap();
    assert_eq!(open.encode("m", &values(&[("t", "ABCDEFGH")])).unwrap().unwrap().1.len(), 8);
    assert_eq!(open.decode(1, b"x").unwrap().1["t"], "x");
    // A payload is a regex over the text, read into its named groups.
    assert!(prose.contains("`payload` (none): a regex over the body as text, read back into its named groups."));
    let groups = load(&message("payload = \"^(?P<k>[A-Z]+)=(?P<v>\\\\d+)$\"\n")).unwrap();
    assert_eq!(groups.decode(1, b"N=12").unwrap().1, values(&[("k", "N"), ("v", "12")]));
    // Placeholders: a text body written first, trimmed names, literal braces, the four refusals.
    assert!(prose.contains("`template` (none): a text body written first, `{name}` replaced by that value (the name read trimmed) and `{{` and `}}` standing for literal braces"));
    let trimmed = load(&message("template = \"{ t }{{\"\n")).unwrap();
    assert_eq!(trimmed.encode("m", &values(&[("t", "x")])).unwrap().unwrap().1, b"x{");
    assert!(prose.contains("an unclosed brace, a stray `}`, an empty `{}` or a `{` inside a placeholder is refused when the file loads"));
    for (template, what) in [("{a", "unclosed"), ("a}", "stray"), ("{}", "empty placeholder"), ("{a{b}", "inside the placeholder")] {
        assert!(refused(&message(&format!("template = \"{template}\"\n"))).contains(what), "{template}");
    }
    // Reading only: a payload or a repeat with nothing to write.
    assert!(prose.contains("A message with a payload or a repeat and neither a template nor a field can be read but never written."));
    let only_repeat = load(&message("repeat = [ { name = \"bc\", pattern = \"BC=([A-Z]+)\" } ]\n")).unwrap();
    assert_eq!(only_repeat.encode("m", &values(&[("bc1", "AAA")])).unwrap(), Err(ComposeError::NoRules));
    assert_eq!(only_repeat.decode(1, b"BC=AAA").unwrap().1["bc1"], "AAA");
    // Repeats: numbered from 1, group 1 else the whole match, the most read, 0 refused, no
    // numbered name another reader's.
    assert!(prose.contains("`repeat` (none): patterns captured repeatedly as `name1`, `name2`..., counted from 1; no numbered name may be a payload group's or another repeat's."));
    assert!(prose.contains("`name` (required): the prefix of the numbered names."));
    assert!(prose.contains("`pattern` (required): the regex captured again and again (group 1, else the whole match)."));
    assert!(prose.contains("`max` (`16`): the most occurrences read; `0` is refused."));
    let grouped = load(&message("repeat = [ { name = \"g\", pattern = \"G=([a-z]+)\", max = 2 }, { name = \"w\", pattern = \"W[0-9]\" } ]\n")).unwrap();
    let (_, fields) = grouped.decode(1, b"G=aa W1 G=bb G=cc W2").unwrap();
    assert_eq!((fields["g1"].as_str(), fields["g2"].as_str(), fields.get("g3"), fields["w1"].as_str(), fields["w2"].as_str()), ("aa", "bb", None, "W1", "W2"));
    assert!(!fields.contains_key("g0") && !fields.contains_key("w0"));
    assert!(refused(&message("repeat = [ { name = \"r\", pattern = \"x\", max = 0 } ]\n")).contains("max is 0"));
    assert_eq!(
        refused(&message("payload = \"(?P<bc1>Z+)\"\nrepeat = [ { name = \"bc\", pattern = \"BC=(\\\\w+)\" } ]\n")),
        "message[0] (`m`).repeat[0] (`bc`): the payload pattern reads a group named `bc1`"
    );
    assert_eq!(
        refused(&message("repeat = [ { name = \"a\", pattern = \"x\" }, { name = \"a1\", pattern = \"y\" } ]\n")),
        "message[0] (`m`).repeat[1] (`a1`): the repeat `a` reads `a11` too"
    );
    assert!(load(&message("repeat = [ { name = \"a\", pattern = \"x\", max = 1 }, { name = \"a1\", pattern = \"y\", max = 1 } ]\n")).is_ok(), "a11 is read by neither");
    assert!(prose.contains("never a name the payload's groups or a repeat's numbered names would read"));
    assert!(refused(&message("repeat = [ { name = \"a\", pattern = \"(x)\", max = 2 } ]\nfields = [ { name = \"a2\", at = 0, ty = \"U8\" } ]\n")).contains("reads a field of that name"));
    for ok in ["a0", "a3", "a01", "a+1"] {
        assert!(load(&message(&format!("repeat = [ {{ name = \"a\", pattern = \"(x)\", max = 2 }} ]\nfields = [ {{ name = \"{ok}\", at = 0, ty = \"U8\" }} ]\n"))).is_ok(), "{ok} is no numbered name");
    }
}

#[test]
fn what_a_message_writes_it_reads_and_what_it_cannot_write_it_says() {
    // The pad byte fills the gaps and the tail.
    assert!(doc_prose().contains("`pad` (`0`): the byte that fills the gaps between fields and the padding to `len`."));
    let padded = load(&message("len = 4\npad = 255\nfields = [ { name = \"n\", at = 1, ty = \"U8\" } ]\n")).unwrap();
    let (_, body) = padded.encode("m", &values(&[("n", "7")])).unwrap().unwrap();
    assert_eq!(body, [255, 7, 255, 255]);
    assert_eq!(padded.decode(1, &body).unwrap().1["n"], "7");
    // A message that only reads text cannot be written: its values would be dropped.
    let read_only = load(&message("payload = \"^X=(?P<x>\\\\d+)$\"\n")).unwrap();
    assert_eq!(read_only.encode("m", &values(&[("x", "5")])).unwrap(), Err(ComposeError::NoRules));
    assert_eq!(ComposeError::NoRules.to_string(), "the message has no writing rules: no template and no fields");
    assert_eq!(read_only.decode(1, b"X=5").unwrap().1["x"], "5");
    // A text body padded to a fixed length is read with its padding: a pattern anchored at the
    // end does not match it, as the reference says; one that allows for the NULs does.
    let anchored = load(&message("len = 6\ntemplate = \"X={x}\"\npayload = \"^X=(?P<x>\\\\d+)$\"\n")).unwrap();
    let (_, body) = anchored.encode("m", &values(&[("x", "5")])).unwrap().unwrap();
    assert_eq!(body, [b'X', b'=', b'5', 0, 0, 0]);
    assert!(anchored.decode(1, &body).is_none());
    let allowing = load(&message("len = 6\ntemplate = \"X={x}\"\npayload = \"^X=(?P<x>\\\\d+)\\\\x00*$\"\n")).unwrap();
    assert_eq!(allowing.decode(1, &body).unwrap().1["x"], "5");
    assert!(doc_prose().contains("A text body padded to `len` is read with its padding, so a pattern anchored at the end does not match it."));
    // A repeat and a field together, the field after the text.
    let both = load(&message("template = \"{codes}\"\nrepeat = [ { name = \"bc\", pattern = \"BC=(\\\\w+)\" } ]\nfields = [ { name = \"n\", at = 20, ty = \"U8\" } ]\n")).unwrap();
    let (_, body) = both.encode("m", &values(&[("codes", "BC=AAA BC=BBB"), ("n", "9")])).unwrap().unwrap();
    assert_eq!(body.len(), 21);
    let (_, fields) = both.decode(1, &body).unwrap();
    assert_eq!((fields["bc1"].as_str(), fields["bc2"].as_str(), fields["n"].as_str()), ("AAA", "BBB", "9"));
    assert!(!fields.contains_key("bc3"));
}

/// The reference, `docs/catalog-schema.md`, as this test holds it: its paragraphs, the worked
/// example's TOML block left out. Compared paragraph by paragraph, whitespace collapsed, by
/// `the_reference_is_the_text_this_test_holds`; every statement in it is held against the reader
/// in the tests above. An edit to the reference is an edit here, beside its pin.
const REFERENCE: &str = r#"
# The catalog file: `veredas-catalog`, version 1

A channel's messages are configured, not programmed (`docs/questions.md` Q8). The channel file
(`docs/channel-schema.md`) says how frames are framed and the link is held; the catalog file
says what the frames carry: each message by name, by the id its segment header (or envelope)
field carries, and by the rules its body is written with and read with.
`veredas_core::telegram::catalog::load` reads it into a `Catalog`; `telegram::compose` writes a
body from named fields and `telegram::capture` reads one back, from the same field list, so a
body a channel sends is a body the same catalog reads. Core knows no device: a device's catalog
is a file like the one below, kept with its channel file, outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a conversation of three messages

A controller announces an item (a binary body of 32 bytes: the item's number, its lane, a read
flag as one text character, a code of up to 16 characters, a weight), the unit it talks to
answers with a destination (8 bytes: the item, the destination, a flag), and the controller
reports the outcome as a line of text. The ids are carried in the `id` field of the channel
file's worked example's segment header. Bytes are `little-endian`. The `answer` for item 7 to
destination 3 with flag 1 is `07 00 00 00 03 00 01 00`; the `outcome` for item 7 at destination
3 with result `SORTED` is the text `ITEM=7;DEST=3;RESULT=SORTED`, and reads back into those three
fields. `crates/veredas-core/tests/telegram_catalog.rs` loads this very block and checks each of
these statements, and every key statement below, against the reader; it holds a copy of this
text, so an edit here is an edit there, beside the behaviour the sentence is held against.

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A
name (of a message, a field, a repeat) is exact: compared as written, with no surrounding
whitespace.

**The top level**.
- `format` (required): always `"veredas-catalog"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed; migrating an older version is portal's (P-010).
- `id_field` (required): the name of the frame field that carries a message's id: a segment
  header field in a layout with segments, an envelope field in one without.
- `byte_order` (`"little"`): `"little"` or `"big"`, for every binary field of every body.
- `message` (none): the messages, as `[[message]]` tables; a catalog may be empty.

**A message** (`[[message]]`).
- `name` (required): what the rules and the bench call the message; unique.
- `id` (required): the number the frame carries in `id_field`; unique.
- `len` (none): a fixed body length. Written bodies are padded with `pad` up to it and refused
  beyond it; a received body of another length is not this message and reads as nothing; a
  field that ends past it, or a template whose literal text is longer, is refused when the file
  loads. Without it a body is as long as what was written, and read whatever its length.
- `pad` (`0`): the byte that fills the gaps between fields and the padding to `len`.
- `fields` (none): binary fields at byte offsets, written and read alike.
- `template` (none): a text body written first, `{name}` replaced by that value (the name read
  trimmed) and `{{` and `}}` standing for literal braces; an unclosed brace, a stray `}`, an empty
  `{}` or a `{` inside a placeholder is refused when the file loads. A field written over the
  text overwrites its bytes.
- `payload` (none): a regex over the body as text, read back into its named groups. A message
  with a payload or a repeat and neither a template nor a field can be read but never written.
  A text body padded to `len` is read with its padding, so a pattern anchored at the end does
  not match it.
- `repeat` (none): patterns captured repeatedly as `name1`, `name2`..., counted from 1; no
  numbered name may be a payload group's or another repeat's.

**A field** (an entry of `fields`).
- `name` (required): unique within the message, and never a name the payload's groups or a
  repeat's numbered names would read.
- `at` (required): the byte offset.
- `ty` (required): `U8`, `U16`, `U32`, `I16`, `I32`, `Ascii` or `Hex`. An integer is written from
  decimal text (a sign only where the type takes one) and refused when it does not fit; a
  text is read back without its trailing whitespace.
- `len` (`0`; required for an `Ascii` or `Hex` field): their byte count. Text is padded with NUL
  to it (a NUL inside the text refused) and refused when longer; hex must spell exactly that
  many bytes. No two fields may share a byte.

**A repeat** (an entry of `repeat`).
- `name` (required): the prefix of the numbered names.
- `pattern` (required): the regex captured again and again (group 1, else the whole match).
- `max` (`16`): the most occurrences read; `0` is refused.

Writing a body needs a value for every placeholder and every field, and refuses one that is
missing or does not fit; reading a body that does not satisfy the rules yields nothing, never a
partial map. A misspelled key in any table, an empty or repeated name, a repeated id, a field
ending past the body or past 16 MiB, a bad pattern or a bad template is refused with its place
named.
"#;
