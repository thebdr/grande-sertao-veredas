//! The TOML document behind a file's two views (the spike's point 2, its Rust side): a form edit
//! changes the characters of one value and nothing else, the text view's edit is taken whole or
//! refused with where it fails, and the file's bytes come back as they were read.

use std::collections::BTreeSet;
use std::path::Path;

use oficina_spike_model::document::{Document, EditError, Field, Kind, KeyPath, LineEndings, OpenError, ParseError, PathError, Scalar, Segment};
use oficina_spike_model::formats::FORMATS;

const SAMPLE: &str = "\
# A channel to the sorter.
format = \"veredas-channel\"
version = 1

[link]
role = \"client\"   # dials the device
host = '127.0.0.1'
port = 0x13A4  # 5028
timeout_ms = 2000
keepalive = true
ratio = 1.5

# Framing below.
[[frame]]
name = \"announce\"
fields = [\"id\", \"length\"]  # in order
limits = { min = 4, max = 512 }
\"odd.key\" = \"kept\"
";

fn doc(text: &str) -> Document {
    Document::from_bytes(text.as_bytes()).expect("the sample opens")
}

fn path(s: &str) -> KeyPath {
    s.parse().expect("the path reads")
}

fn field(path: &str, value: Scalar, raw: &str, line: usize, comment: Option<&str>, stamp: bool) -> Field {
    Field { path: self::path(path), value, raw: raw.to_string(), line, comment: comment.map(str::to_string), stamp }
}

#[test]
fn the_form_lists_every_scalar_in_the_order_of_the_text() {
    use Scalar::*;
    let d = doc(SAMPLE);
    assert_eq!(
        d.fields(),
        vec![
            field("format", String("veredas-channel".into()), "\"veredas-channel\"", 2, None, true),
            field("version", Integer(1), "1", 3, None, true),
            field("link.role", String("client".into()), "\"client\"", 6, Some("dials the device"), false),
            field("link.host", String("127.0.0.1".into()), "'127.0.0.1'", 7, None, false),
            field("link.port", Integer(5028), "0x13A4", 8, Some("5028"), false),
            field("link.timeout_ms", Integer(2000), "2000", 9, None, false),
            field("link.keepalive", Boolean(true), "true", 10, None, false),
            field("link.ratio", Float(1.5), "1.5", 11, None, false),
            field("frame[0].name", String("announce".into()), "\"announce\"", 15, None, false),
            field("frame[0].fields[0]", String("id".into()), "\"id\"", 16, None, false),
            field("frame[0].fields[1]", String("length".into()), "\"length\"", 16, None, false),
            field("frame[0].limits.min", Integer(4), "4", 17, None, false),
            field("frame[0].limits.max", Integer(512), "512", 17, None, false),
            field("frame[0].\"odd.key\"", String("kept".into()), "\"kept\"", 18, None, false),
        ]
    );
    assert_eq!(d.text(), SAMPLE);
    assert_eq!(d.revision(), 0);
    assert_eq!(d.stamp().map(|s| (s.format, s.version)), Ok(("veredas-channel".to_string(), 1)));
}

#[test]
fn a_form_edit_changes_that_value_alone_in_the_old_value_s_style() {
    // (path, new value, the line before, the line after): every other byte of the file stays.
    let edits: Vec<(&str, Scalar, &str, &str)> = vec![
        ("link.role", Scalar::String("server".into()), "role = \"client\"   # dials the device", "role = \"server\"   # dials the device"),
        ("link.host", Scalar::String("10.0.0.7".into()), "host = '127.0.0.1'", "host = '10.0.0.7'"),
        ("link.host", Scalar::String("it's".into()), "host = '127.0.0.1'", "host = \"it's\""),
        ("link.port", Scalar::Integer(5029), "port = 0x13A4  # 5028", "port = 0x13A5  # 5028"),
        ("link.port", Scalar::Integer(-1), "port = 0x13A4  # 5028", "port = -1  # 5028"),
        ("link.timeout_ms", Scalar::Integer(250), "timeout_ms = 2000", "timeout_ms = 250"),
        ("link.keepalive", Scalar::Boolean(false), "keepalive = true", "keepalive = false"),
        ("link.ratio", Scalar::Float(2.0), "ratio = 1.5", "ratio = 2.0"),
        ("frame[0].fields[1]", Scalar::String("len".into()), "fields = [\"id\", \"length\"]  # in order", "fields = [\"id\", \"len\"]  # in order"),
        ("frame[0].limits.max", Scalar::Integer(1024), "limits = { min = 4, max = 512 }", "limits = { min = 4, max = 1024 }"),
        ("frame[0].\"odd.key\"", Scalar::String("still".into()), "\"odd.key\" = \"kept\"", "\"odd.key\" = \"still\""),
    ];
    for (p, value, before, after) in edits {
        let mut d = doc(SAMPLE);
        d.set(&path(p), value.clone()).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(d.text(), SAMPLE.replacen(before, after, 1), "{p}");
        assert_eq!(d.field(&path(p)).map(|f| f.value), Some(value), "{p} reads back");
        assert_eq!(d.revision(), 1, "{p}");
    }
}

#[test]
fn the_same_value_set_again_changes_nothing() {
    let mut d = doc(SAMPLE);
    d.set(&path("link.timeout_ms"), Scalar::Integer(2000)).expect("the same value");
    assert_eq!((d.text(), d.revision()), (SAMPLE, 0));
}

#[test]
fn a_form_edit_that_cannot_be_made_is_refused_by_name_and_changes_nothing() {
    let mut d = doc(SAMPLE);
    let refused = |d: &mut Document, p: &str, v: Scalar| d.set(&path(p), v).unwrap_err();
    assert_eq!(refused(&mut d, "format", Scalar::String("veredas-catalog".into())), EditError::Stamp(path("format")));
    assert_eq!(refused(&mut d, "version", Scalar::Integer(2)), EditError::Stamp(path("version")));
    assert_eq!(
        refused(&mut d, "link.port", Scalar::String("5028".into())),
        EditError::Kind { path: path("link.port"), found: Kind::Integer, sent: Kind::String }
    );
    assert_eq!(refused(&mut d, "link.ratio", Scalar::Integer(2)), EditError::Kind { path: path("link.ratio"), found: Kind::Float, sent: Kind::Integer });
    assert_eq!(refused(&mut d, "link", Scalar::Integer(1)), EditError::NotScalar(path("link")));
    assert_eq!(refused(&mut d, "frame[0].fields", Scalar::Integer(1)), EditError::NotScalar(path("frame[0].fields")));
    assert_eq!(refused(&mut d, "frame[0].limits", Scalar::Integer(1)), EditError::NotScalar(path("frame[0].limits")));
    assert_eq!(refused(&mut d, "link.speed", Scalar::Integer(1)), EditError::NoValue(path("link.speed")));
    assert_eq!(refused(&mut d, "frame[1].name", Scalar::String("x".into())), EditError::NoValue(path("frame[1].name")));
    assert_eq!(refused(&mut d, "link.port[0]", Scalar::Integer(1)), EditError::NoValue(path("link.port[0]")));
    assert_eq!((d.text(), d.revision()), (SAMPLE, 0));
    // The texts a view shows.
    assert_eq!(EditError::Stamp(path("format")).to_string(), "`format` is the file's stamp: its format and version are not the form's to change");
    assert_eq!(
        EditError::Kind { path: path("link.port"), found: Kind::Integer, sent: Kind::String }.to_string(),
        "`link.port` holds an integer, and the form sets a string: a value keeps its type"
    );
    assert_eq!(EditError::NotScalar(path("link")).to_string(), "`link` is no single value: a table or an array is edited as text");
    assert_eq!(EditError::NoValue(path("link.speed")).to_string(), "no value at `link.speed`");
    // A `format` that is not the top-level stamp is an ordinary value.
    let mut nested = doc("format = \"veredas-x\"\nversion = 1\n[t]\nformat = \"csv\"\n");
    nested.set(&path("t.format"), Scalar::String("json".into())).expect("a nested format");
    assert_eq!(nested.text(), "format = \"veredas-x\"\nversion = 1\n[t]\nformat = \"json\"\n");
}

#[test]
fn a_date_time_is_checked_before_it_is_written() {
    let mut d = doc("at = 1979-05-27T07:32:00Z # the first\n");
    assert_eq!(
        d.set(&path("at"), Scalar::Datetime("the 27th".into())).unwrap_err().to_string(),
        "`at`: `the 27th` is not a TOML date-time"
    );
    d.set(&path("at"), Scalar::Datetime("2026-11-22T10:00:00+01:00".into())).expect("a date-time");
    assert_eq!(d.text(), "at = 2026-11-22T10:00:00+01:00 # the first\n");
    assert_eq!(d.fields()[0].value, Scalar::Datetime("2026-11-22T10:00:00+01:00".into()));
}

#[test]
fn line_endings_and_a_byte_order_mark_come_back_as_they_were_read() {
    let crlf = format!("\u{feff}{}", SAMPLE.replace('\n', "\r\n"));
    let mut d = Document::from_bytes(crlf.as_bytes()).expect("CR LF and a mark open");
    assert_eq!((d.text(), d.endings(), d.has_bom()), (SAMPLE, LineEndings::Crlf, true));
    assert_eq!(d.to_bytes(), crlf.as_bytes());
    d.set(&path("link.port"), Scalar::Integer(5029)).expect("an edit");
    assert_eq!(String::from_utf8(d.to_bytes()).expect("UTF-8"), crlf.replacen("0x13A4", "0x13A5", 1));
    // Lines ended both ways come back with the ending more lines used, LF on a tie.
    let mixed = doc("a = 1\r\nb = 2\nc = 3\r\n");
    assert_eq!(mixed.endings(), LineEndings::Mixed { lf: 1, crlf: 2 });
    assert_eq!(mixed.to_bytes(), b"a = 1\r\nb = 2\r\nc = 3\r\n");
    let tie = doc("a = 1\r\nb = 2\n");
    assert_eq!(tie.endings(), LineEndings::Mixed { lf: 1, crlf: 1 });
    assert_eq!(tie.to_bytes(), b"a = 1\nb = 2\n");
    // One line and no ending, and an empty file, are LF and come back as they were.
    assert_eq!(doc("a = 1").to_bytes(), b"a = 1");
    assert_eq!((doc("").to_bytes(), doc("").endings()), (Vec::new(), LineEndings::Lf));
}

#[test]
fn bytes_that_are_not_utf8_are_refused_at_their_offset() {
    assert_eq!(Document::from_bytes(b"a = \"\xff\"\n").unwrap_err(), OpenError::NotUtf8 { offset: 5 });
    assert_eq!(Document::from_bytes(b"\xef\xbb\xbfa = \"\xff\"\n").unwrap_err(), OpenError::NotUtf8 { offset: 8 });
    assert_eq!(OpenError::NotUtf8 { offset: 5 }.to_string(), "not text in UTF-8: the byte at offset 5 begins no character");
}

#[test]
fn a_text_that_does_not_parse_is_refused_where_it_fails_and_the_document_stays() {
    let mut d = doc(SAMPLE);
    // The column counts UTF-16 code units, as both views do: `ã` is one, `😀` two.
    let accented = "format = \"veredas-ã\"\nversion = = 1\n";
    assert_eq!(
        d.set_text(accented).unwrap_err(),
        ParseError { message: "extra `=`, expected nothing".into(), line: 2, column: 11, offset: 32 }
    );
    let emoji = "a = \"😀\" x\n";
    let e = d.set_text(emoji).unwrap_err();
    assert_eq!(e, ParseError { message: "unexpected key or value, expected newline, `#`".into(), line: 1, column: 10, offset: 11 });
    assert_eq!(e.to_string(), "line 1, column 10: unexpected key or value, expected newline, `#`");
    assert_eq!((d.text(), d.revision()), (SAMPLE, 0));
    // Opening such bytes is refused the same way.
    assert_eq!(Document::from_bytes(emoji.as_bytes()).unwrap_err(), OpenError::Parse(e));
    // A text that parses is taken whole, CR LF read as LF; the same text again is no change.
    let edited = SAMPLE.replace("2000", "3000");
    d.set_text(&edited.replace('\n', "\r\n")).expect("a text that parses");
    assert_eq!((d.text(), d.revision()), (edited.as_str(), 1));
    d.set_text(&edited).expect("the same text");
    assert_eq!(d.revision(), 1);
    assert_eq!(d.field(&path("link.timeout_ms")).map(|f| f.value), Some(Scalar::Integer(3000)));
}

#[test]
fn a_key_path_is_written_as_toml_writes_keys_and_read_back() {
    let cases: Vec<(KeyPath, &str)> = vec![
        (KeyPath(vec![Segment::Key("log".into()), Segment::Index(0), Segment::Key("timestamp".into()), Segment::Key("pattern".into())]), "log[0].timestamp.pattern"),
        (KeyPath::keys(&["odd.key"]), "\"odd.key\""),
        (KeyPath::keys(&["a", "with space", "b"]), "a.\"with space\".b"),
        (KeyPath::keys(&["ação"]), "\"ação\""),
        (KeyPath::keys(&["quo\"te", "back\\slash"]), "\"quo\\\"te\".\"back\\\\slash\""),
        (KeyPath::keys(&["tab\there"]), "\"tab\\u0009here\""),
        (KeyPath::keys(&[""]), "\"\""),
        (KeyPath(vec![Segment::Key("m".into()), Segment::Index(2), Segment::Index(10)]), "m[2][10]"),
    ];
    for (p, text) in cases {
        assert_eq!(p.to_string(), text);
        assert_eq!(text.parse::<KeyPath>(), Ok(p), "{text}");
    }
    let refused = |s: &str| s.parse::<KeyPath>().unwrap_err();
    assert_eq!(refused(""), PathError { at: 0, why: "an empty path" });
    assert_eq!(refused("a."), PathError { at: 2, why: "a key after `.`" });
    assert_eq!(refused("a..b"), PathError { at: 2, why: "a bare key is ASCII letters, digits, `-` and `_`; quote any other" });
    assert_eq!(refused("a[x]"), PathError { at: 1, why: "an index is digits between `[` and `]`" });
    assert_eq!(refused("a[1"), PathError { at: 1, why: "an index is digits between `[` and `]`" });
    assert_eq!(refused("\"open"), PathError { at: 5, why: "a quoted key closes with `\"`" });
    assert_eq!(refused("a b"), PathError { at: 1, why: "`.` or `[` between the steps" });
    assert_eq!(refused("\"\\q\""), PathError { at: 1, why: "an escape is `\\\"`, `\\\\` or `\\uXXXX`" });
}

/// Every TOML text the suite holds to the stamp, as the laws read them (L7: every TOML file that
/// is not a cargo manifest, and every TOML block of a reference under `docs/`).
fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    leis::Suite::read(&root).expect("the repository reads").stamped
}

/// The tree's table of formats and the suite's TOML texts name the same formats: no stamped text
/// of the suite is shown without its component, and the table names no format the suite dropped.
#[test]
fn every_toml_text_of_the_suite_comes_back_byte_for_byte_and_covers_every_known_format() {
    let corpus = corpus();
    let mut formats = BTreeSet::new();
    for (place, text) in &corpus {
        let d = Document::from_bytes(text.as_bytes()).unwrap_or_else(|e| panic!("{place}: {e}"));
        assert_eq!(d.to_bytes(), text.as_bytes(), "{place}");
        if let Ok(s) = d.stamp() {
            formats.insert(s.format);
        }
    }
    let known: BTreeSet<String> = FORMATS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(known, formats, "the table of formats against the formats of the suite's TOML texts");
}

/// A value of the same type that differs from `v`.
fn other(v: &Scalar) -> Option<Scalar> {
    Some(match v {
        Scalar::String(s) => Scalar::String(format!("{s}x")),
        Scalar::Integer(n) => Scalar::Integer(n.checked_add(1).unwrap_or(n - 1)),
        Scalar::Float(x) if x.is_finite() => Scalar::Float(x + 0.5),
        Scalar::Float(_) => Scalar::Float(1.5),
        Scalar::Boolean(b) => Scalar::Boolean(!b),
        Scalar::Datetime(_) => return None,
    })
}

#[test]
fn every_value_of_every_toml_text_of_the_suite_edits_alone() {
    let mut edits = 0;
    for (place, text) in corpus() {
        let d = Document::from_bytes(text.as_bytes()).unwrap_or_else(|e| panic!("{place}: {e}"));
        let before = d.fields();
        for f in before.iter().filter(|f| !f.stamp) {
            let Some(new) = other(&f.value) else { continue };
            let mut e = d.clone();
            e.set(&f.path, new.clone()).unwrap_or_else(|err| panic!("{place} `{}`: {err}", f.path));
            edits += 1;
            let after = e.fields();
            assert_eq!(after.len(), before.len(), "{place} `{}`", f.path);
            for (b, a) in before.iter().zip(&after) {
                assert_eq!((&a.path, &a.comment), (&b.path, &b.comment), "{place}: editing `{}` moved a value or a comment", f.path);
                if b.path == f.path {
                    assert_eq!(a.value, new, "{place} `{}` reads back", f.path);
                } else {
                    assert_eq!(a.raw, b.raw, "{place}: editing `{}` changed another value", f.path);
                }
            }
            if !f.raw.contains('\n') {
                let changed: Vec<usize> = text.lines().zip(e.text().lines()).enumerate().filter(|(_, (x, y))| x != y).map(|(i, _)| i + 1).collect();
                assert_eq!(text.lines().count(), e.text().lines().count(), "{place} `{}`", f.path);
                assert_eq!(changed, vec![f.line], "{place}: editing `{}` changed other lines", f.path);
            }
        }
    }
    assert!(edits > 0, "the corpus holds values to edit");
    eprintln!("{edits} edits, each alone");
}
