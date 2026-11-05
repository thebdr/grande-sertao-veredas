//! The channel file (contract: the channel-schema clause): read, versioned, validated, and
//! documented by a worked example this test loads from `docs/channel-schema.md` itself.

use std::time::Duration;

use veredas_core::telegram::Endian;
use veredas_core::telegram::framing::{FieldRole, Frame};
use veredas_core::telegram::schema::*;

/// The first ```toml block of the schema's documentation.
fn documented_example() -> String {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/channel-schema.md");
    let text = std::fs::read_to_string(doc).unwrap();
    let start = text.find("```toml\n").or_else(|| text.find("```toml\r\n")).unwrap();
    let body = &text[start..];
    let body = &body[body.find('\n').unwrap() + 1..];
    body[..body.find("```").unwrap()].to_string()
}

/// A minimal valid file with `extra` appended inside `[framing]`'s neighbourhood.
fn file(framing: &str, rest: &str) -> String {
    format!("format = \"veredas-channel\"\nversion = 1\n[framing]\n{framing}\n{rest}")
}

const LP: &str = "envelope = [ { name = \"length\", size = 2, role = \"length\" } ]";

fn refused(text: &str) -> String {
    match load(text) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_documented_example_loads_and_frames_as_described() {
    let spec = load(&documented_example()).unwrap();
    let f = &spec.framing;
    assert_eq!((f.start.as_slice(), f.end.as_slice(), f.max_frame), (&[0xFD][..], &[0xFE][..], 65_536));
    let names: Vec<(&str, usize, FieldRole)> = f.envelope.iter().map(|x| (x.name.as_str(), x.size, x.role)).collect();
    assert_eq!(
        names,
        [
            ("class", 1, FieldRole::Opaque),
            ("source", 1, FieldRole::Opaque),
            ("destination", 1, FieldRole::Opaque),
            ("sequence", 1, FieldRole::Sequence),
            ("spare", 3, FieldRole::Opaque),
            ("length", 2, FieldRole::Length),
            ("count", 2, FieldRole::Count),
        ]
    );
    assert_eq!(f.envelope.iter().map(|x| x.value).collect::<Vec<_>>(), [0, 1, 2, 0, 0, 0, 0]);
    assert_eq!(f.segment.as_ref().unwrap().len(), 2);
    assert_eq!(f.tail.iter().map(|x| x.size).sum::<usize>(), 3);
    assert!(f.envelope.iter().all(|x| x.order == Endian::Little));
    assert_eq!(
        spec.sequence,
        Some(SequenceRules { connect: 0, first: 1, last: 255, skip_duplicates: true, keepalive_counts: true })
    );
    assert_eq!(spec.keepalive, KeepaliveRules { on_connect: true, idle: Duration::from_secs(10) });
    assert_eq!(
        spec.link,
        LinkRules { write_timeout: Duration::from_secs(2), outbox_limit_bytes: 16 * 1024 * 1024, max_peers: 8 }
    );
    // The keep-alive it describes: sixteen bytes, the sequence and node defaults in place.
    assert_eq!(
        f.encode(&Frame::default(), Some(0)).unwrap(),
        [0xFD, 0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFE]
    );
}

#[test]
fn defaults_fill_what_a_file_leaves_out() {
    let spec = load(&file(LP, "")).unwrap();
    assert_eq!((spec.framing.start.len(), spec.framing.end.len(), spec.framing.max_frame), (0, 0, 65_536));
    assert_eq!(spec.sequence, None);
    assert_eq!(spec.keepalive, KeepaliveRules { on_connect: true, idle: Duration::from_millis(10_000) });
    assert_eq!(spec.link.max_peers, 8);
    // An empty [sequence] takes every default.
    let spec = load(&file(
        "envelope = [ { name = \"s\", size = 2, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]",
        "[sequence]\n",
    ))
    .unwrap();
    assert_eq!(spec.sequence.unwrap(), SequenceRules { connect: 0, first: 1, last: 255, skip_duplicates: true, keepalive_counts: true });
}

#[test]
fn a_field_may_override_the_byte_order() {
    let spec = load(&file(
        "byte_order = \"big\"\nenvelope = [ { name = \"tag\", size = 2, byte_order = \"little\", value = 258 }, { name = \"length\", size = 2, role = \"length\" } ]",
        "",
    ))
    .unwrap();
    assert_eq!(spec.framing.envelope[0].order, Endian::Little);
    assert_eq!(spec.framing.envelope[1].order, Endian::Big);
    assert_eq!(spec.framing.encode(&Frame::default(), None).unwrap(), [0x02, 0x01, 0, 0]);
}

#[test]
fn a_file_states_its_format_and_a_version_this_reader_knows() {
    assert_eq!(refused("version = 1\n[framing]\nenvelope = []"), "no `format`: a channel file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-channel\"\n[framing]\nenvelope = []"), "no `version`: a channel file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-project\"\nversion = 1"), "`format` is `veredas-project`, not `veredas-channel`");
    assert_eq!(
        refused("format = \"veredas-channel\"\nversion = 2\n[framing]\nwhatever = true"),
        "version 2 of `veredas-channel` is not known here (this reader knows 1)"
    );
    assert!(refused("format = [").starts_with("not a channel file: "));
}

#[test]
fn a_misspelled_key_is_refused_wherever_it_is() {
    for (framing, rest, key) in [
        (format!("{LP}\nmax_frames = 10"), String::new(), "max_frames"),
        ("envelope = [ { name = \"length\", size = 2, rol = \"length\" } ]".to_string(), String::new(), "rol"),
        (LP.to_string(), "[keepalive]\nidle = 5".to_string(), "idle"),
        (LP.to_string(), "[link]\nwrite_timeout = 5".to_string(), "write_timeout"),
        (LP.to_string(), "[linc]\nmax_peers = 1".to_string(), "linc"),
    ] {
        let e = refused(&file(&framing, &rest));
        assert!(e.starts_with("not a channel file: ") && e.contains(key), "{key}: {e}");
    }
    let e = refused(&file("envelope = [ { name = \"length\", size = 2, role = \"size\" } ]", ""));
    assert!(e.contains("size") && e.starts_with("not a channel file: "), "{e}");
}

#[test]
fn an_impossible_layout_is_refused_with_its_place_named() {
    let env = |fields: &str| format!("envelope = [ {fields} ]");
    let l = "{ name = \"l\", size = 2, role = \"length\" }";
    let cases: Vec<(String, &str, &str)> = vec![
        (env(&format!("{{ name = \"x\", size = 0 }}, {l}")), "", "envelope.x: size 0 is not 1 to 8 bytes"),
        (env(&format!("{{ name = \"x\", size = 9 }}, {l}")), "", "envelope.x: size 9 is not 1 to 8 bytes"),
        (env(&format!("{{ name = \"x\", size = 1 }}, {{ name = \"x\", size = 1 }}, {l}")), "", "envelope: two fields are named `x`"),
        (env(&format!("{{ name = \"  \", size = 1 }}, {l}")), "", "envelope: a field has no name"),
        (env(&format!("{l}, {{ name = \"m\", size = 2, role = \"length\" }}")), "", "envelope: two fields share the role of `m`"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\", value = 3 }}, {l}")), "", "envelope.s: a computed field takes no `value`"),
        (env(&format!("{{ name = \"x\", size = 1, value = 256 }}, {l}")), "", "envelope.x: value 256 does not fit 1 byte(s)"),
        (env("{ name = \"x\", size = 1 }"), "", "envelope: without segments the envelope needs a `length` field"),
        (env(&format!("{l}, {{ name = \"c\", size = 1, role = \"count\" }}")), "", "envelope: a `count` field needs segments"),
        (format!("{}\nsegment = [ {{ name = \"id\", size = 1 }} ]", env(l)), "", "segment: a segment header needs a `length` field"),
        (format!("{}\nsegment = [ {{ name = \"n\", size = 1, role = \"length\" }} ]", env("{ name = \"x\", size = 1 }")), "", "envelope: segments need a `length` or a `count` field to end"),
        (format!("{}\nsegment = []", env(l)), "", "segment: an empty segment header; leave `segment` out instead"),
        (format!("{}\nsegment = [ {{ name = \"c\", size = 1, role = \"count\" }} ]", env(l)), "", "segment.c: this part takes no such role"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 1, role = \"sequence\" }} ]", env(l)), "", "tail.t: this part takes no such role"),
        (format!("{}\ntail = [ {{ name = \"l\", size = 1 }} ]", env(l)), "", "tail.l: the envelope already has a field named `l`"),
        (format!("start = \"\"\n{}", env(l)), "", "framing.start: an empty delimiter; leave it out instead"),
        (format!("end = \"FG\"\n{}", env(l)), "", "framing.end: `FG` is not a hex byte"),
        (format!("max_frame = 3\nstart = \"FD\"\nend = \"FE\"\n{}", env(l)), "", "framing.max_frame: 3 is less than the 4 fixed bytes"),
        (env(l), "[sequence]\n", "sequence: rules for a sequence the envelope has no `sequence` field for"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nfirst = 9\nlast = 3", "sequence: first 9 .. last 3 (connect 0) does not fit a 1-byte field in order"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nlast = 256", "sequence: first 1 .. last 256 (connect 0) does not fit a 1-byte field in order"),
        (env(l), "[link]\nmax_peers = 0", "link.max_peers: at least one"),
    ];
    for (framing, rest, expected) in cases {
        assert_eq!(refused(&file(&framing, rest)), expected, "{framing} {rest}");
    }
}
