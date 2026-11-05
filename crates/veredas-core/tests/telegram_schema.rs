//! The channel file (contract: the channel-schema clause): read, versioned, validated, and
//! documented by a worked example this test loads from `docs/channel-schema.md` itself.

use std::collections::BTreeMap;
use std::time::Duration;

use veredas_core::telegram::Endian;
use veredas_core::telegram::framing::{FieldRole, Frame, Segment, parse_hex};
use veredas_core::telegram::schema::*;

/// The schema's documentation, as text.
fn doc() -> String {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/channel-schema.md");
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
    let shape = |fields: &[veredas_core::telegram::framing::Field]| -> Vec<(String, usize, FieldRole)> {
        fields.iter().map(|x| (x.name.clone(), x.size, x.role)).collect()
    };
    let named = |v: &[(&str, usize, FieldRole)]| -> Vec<(String, usize, FieldRole)> {
        v.iter().map(|(n, s, r)| (n.to_string(), *s, *r)).collect()
    };
    assert_eq!(
        shape(&f.envelope),
        named(&[
            ("class", 1, FieldRole::Opaque),
            ("source", 1, FieldRole::Opaque),
            ("destination", 1, FieldRole::Opaque),
            ("sequence", 1, FieldRole::Sequence),
            ("spare", 3, FieldRole::Opaque),
            ("length", 2, FieldRole::Length),
            ("count", 2, FieldRole::Count),
        ])
    );
    assert_eq!(
        shape(f.segment.as_ref().unwrap()),
        named(&[("id", 2, FieldRole::Opaque), ("length", 2, FieldRole::Length)])
    );
    assert_eq!(shape(&f.tail), named(&[("tail_spare", 2, FieldRole::Opaque), ("check", 1, FieldRole::Opaque)]));
    assert_eq!(f.envelope.iter().map(|x| x.value).collect::<Vec<_>>(), [0, 1, 2, 0, 0, 0, 0]);
    let all = || f.envelope.iter().chain(f.segment.as_ref().unwrap()).chain(&f.tail);
    assert!(all().all(|x| x.order == Endian::Little));
    // The sums the prose states: an 11-byte envelope after the start byte, a 3-byte tail before
    // the end byte.
    assert_eq!(f.envelope.iter().map(|x| x.size).sum::<usize>(), 11);
    assert_eq!(f.tail.iter().map(|x| x.size).sum::<usize>(), 3);
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
    let keepalive = f.encode(&Frame::default(), Some(0)).unwrap();
    assert_eq!(keepalive, [0xFD, 0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFE]);
    // The data frame it describes: one segment `AB`, id 0x0102, sequence 7, byte for byte.
    let ab = Frame {
        fields: BTreeMap::new(),
        segments: vec![Segment { fields: BTreeMap::from([("id".to_string(), 0x0102)]), body: b"AB".to_vec() }],
    };
    let data = f.encode(&ab, Some(7)).unwrap();
    assert_eq!(
        data,
        [0xFD, 0, 1, 2, 7, 0, 0, 0, 6, 0, 1, 0, 2, 1, 2, 0, 0x41, 0x42, 0, 0, 0, 0xFE]
    );
    // And the prose says exactly these numbers.
    let prose = doc_prose();
    for phrase in ["an 11-byte envelope after it", "a 3-byte tail", "A keep-alive is therefore 16 bytes"] {
        assert!(prose.contains(phrase), "the documentation says `{phrase}`");
    }
    let quoted = prose.split("sequence 7 is `").nth(1).and_then(|s| s.split('`').next()).unwrap();
    assert_eq!(parse_hex(quoted).unwrap(), data);
}

#[test]
fn defaults_fill_what_a_file_leaves_out() {
    let spec = load(&file(LP, "")).unwrap();
    let f = &spec.framing;
    assert_eq!((f.start.len(), f.end.len(), f.max_frame), (0, 0, DEFAULT_MAX_FRAME));
    assert_eq!(DEFAULT_MAX_FRAME, 65_536);
    assert_eq!(f.segment, None);
    assert!(f.tail.is_empty());
    assert_eq!(f.envelope.len(), 1);
    assert_eq!((f.envelope[0].order, f.envelope[0].value), (Endian::Little, 0));
    assert_eq!(spec.sequence, None);
    assert_eq!(spec.keepalive, KeepaliveRules { on_connect: true, idle: Duration::from_millis(10_000) });
    assert_eq!(
        spec.link,
        LinkRules { write_timeout: Duration::from_millis(2_000), outbox_limit_bytes: 16 * 1024 * 1024, max_peers: 8 }
    );
    // An empty [sequence] takes every default.
    let spec = load(&file(
        "envelope = [ { name = \"s\", size = 2, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]",
        "[sequence]\n",
    ))
    .unwrap();
    assert_eq!(spec.sequence.unwrap(), SequenceRules { connect: 0, first: 1, last: 255, skip_duplicates: true, keepalive_counts: true });
}

#[test]
fn every_limit_is_accepted_at_its_edge() {
    // The frame limit at its ceiling, the outbox exactly one largest frame, a timeout of 1 ms,
    // a cycle of one number that does not skip.
    let spec = load(&file(&format!("max_frame = {MAX_MAX_FRAME}\n{LP}"), "")).unwrap();
    assert_eq!((spec.framing.max_frame, MAX_MAX_FRAME), (16 * 1024 * 1024, 16 * 1024 * 1024));
    let spec = load(&file(&format!("max_frame = 2048\n{LP}"), "[link]\noutbox_limit_bytes = 2048\nwrite_timeout_ms = 1")).unwrap();
    assert_eq!((spec.link.outbox_limit_bytes, spec.link.write_timeout), (2048, Duration::from_millis(1)));
    let one = "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]";
    let spec = load(&file(one, "[sequence]\nfirst = 5\nlast = 5\nskip_duplicates = false\nconnect = 4")).unwrap();
    assert_eq!(spec.sequence.map(|s| (s.first, s.last, s.skip_duplicates)), Some((5, 5, false)));
    // `connect` just outside the cycle, on either side (Q11).
    for (connect, first, last) in [(0, 1, 2), (3, 1, 2), (255, 0, 254)] {
        let rules = format!("[sequence]\nconnect = {connect}\nfirst = {first}\nlast = {last}");
        let s = load(&file(one, &rules)).unwrap().sequence.unwrap();
        assert_eq!((s.connect, s.first, s.last), (connect, first, last));
    }
    // The frame limit at its floor, exactly the fixed bytes (start, a 2-byte length, end), and
    // one peer (refute round 3).
    let spec = load(&file(&format!("max_frame = 4\nstart = \"FD\"\nend = \"FE\"\n{LP}"), "")).unwrap();
    assert_eq!(spec.framing.max_frame, 4);
    assert_eq!(spec.framing.encode(&Frame::default(), None).unwrap(), [0xFD, 0, 0, 0xFE]);
    assert_eq!(load(&file(LP, "[link]\nmax_peers = 1")).unwrap().link.max_peers, 1);
}

#[test]
fn every_rule_is_read_from_the_file_and_every_way_to_end_is_accepted() {
    // Keep-alive and sequence keys away from their defaults are read, not assumed (refute
    // round 4).
    let spec = load(&file(LP, "[keepalive]\non_connect = false\nidle_ms = 0")).unwrap();
    assert_eq!(spec.keepalive, KeepaliveRules { on_connect: false, idle: Duration::ZERO });
    let spec = load(&file(LP, "[keepalive]\nidle_ms = 1234")).unwrap();
    assert_eq!(spec.keepalive, KeepaliveRules { on_connect: true, idle: Duration::from_millis(1234) });
    let seq = "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]";
    let spec = load(&file(seq, "[sequence]\nkeepalive_counts = false")).unwrap();
    assert_eq!(
        spec.sequence,
        Some(SequenceRules { connect: 0, first: 1, last: 255, skip_duplicates: true, keepalive_counts: false })
    );
    // A segmented layout ends by its data-area length, by its segment count, or by both.
    let header = "segment = [ { name = \"n\", size = 1, role = \"length\" } ]";
    for envelope in [
        "envelope = [ { name = \"l\", size = 2, role = \"length\" } ]",
        "envelope = [ { name = \"c\", size = 1, role = \"count\" } ]",
        "envelope = [ { name = \"l\", size = 2, role = \"length\" }, { name = \"c\", size = 1, role = \"count\" } ]",
    ] {
        let f = load(&file(&format!("{envelope}\n{header}"), "")).unwrap().framing;
        assert_eq!(f.segment.map(|s| s.len()), Some(1), "{envelope}");
    }
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
        (
            "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]".to_string(),
            "[sequence]\nfirts = 1".to_string(),
            "firts",
        ),
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
        // Every role a part does not take; a count given a value; a default too wide for a
        // wider field (round 4).
        (format!("{}\nsegment = [ {{ name = \"s\", size = 1, role = \"sequence\" }}, {{ name = \"n\", size = 1, role = \"length\" }} ]", env(l)), "", "segment.s: this part takes no such role"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 1, role = \"length\" }} ]", env(l)), "", "tail.t: this part takes no such role"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 1, role = \"count\" }} ]", env(l)), "", "tail.t: this part takes no such role"),
        (format!("{}\nsegment = [ {{ name = \"n\", size = 1, role = \"length\" }} ]", env(&format!("{l}, {{ name = \"c\", size = 1, role = \"count\", value = 0 }}"))), "", "envelope.c: a computed field takes no `value`"),
        (env(&format!("{{ name = \"x\", size = 2, value = 65536 }}, {l}")), "", "envelope.x: value 65536 does not fit 2 byte(s)"),
        // The tail counts among the fixed bytes; a `connect` too wide for its field (round 3).
        (format!("max_frame = 4\n{}\ntail = [ {{ name = \"t\", size = 3 }} ]", env(l)), "", "framing.max_frame: 4 is less than the 5 fixed bytes"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nconnect = 256", "sequence: first 1 .. last 255 (connect 256) does not fit a 1-byte field in order"),
        (env(l), "[sequence]\n", "sequence: rules for a sequence the envelope has no `sequence` field for"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nfirst = 9\nlast = 3", "sequence: first 9 .. last 3 (connect 0) does not fit a 1-byte field in order"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nlast = 256", "sequence: first 1 .. last 256 (connect 0) does not fit a 1-byte field in order"),
        (env(l), "[link]\nmax_peers = 0", "link.max_peers: at least one"),
        // Refute round 1 (C-017): a sequence field states its rules; a computed field states no
        // value, not even 0; the frame limit has a ceiling; a cycle of one cannot skip repeats;
        // a link setting of zero, or an outbox smaller than one frame, is refused.
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "", "envelope.s: a sequence field needs `[sequence]` rules (an empty `[sequence]` takes the defaults)"),
        (env("{ name = \"l\", size = 2, role = \"length\", value = 0 }"), "", "envelope.l: a computed field takes no `value`"),
        (format!("max_frame = 16777217\n{}", env(l)), "", "framing.max_frame: 16777217 is over the 16777216-byte ceiling"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nfirst = 5\nlast = 5", "sequence: a cycle of one number cannot skip duplicates (every frame would repeat)"),
        (env(l), "[link]\nwrite_timeout_ms = 0", "link.write_timeout_ms: at least 1"),
        (format!("max_frame = 2048\n{}", env(l)), "[link]\noutbox_limit_bytes = 2047", "link.outbox_limit_bytes: 2047 cannot hold one largest frame (2048)"),
        // Refute round 2 (C-017): `connect` inside the cycle (Q11), at each end of it.
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nfirst = 1\nlast = 2\nconnect = 2", "sequence: connect 2 lies inside the cycle 1 .. 2 (the connect keep-alive is told by its number)"),
        (env(&format!("{{ name = \"s\", size = 1, role = \"sequence\" }}, {l}")), "[sequence]\nfirst = 0\nlast = 1", "sequence: connect 0 lies inside the cycle 0 .. 1 (the connect keep-alive is told by its number)"),
    ];
    for (framing, rest, expected) in cases {
        assert_eq!(refused(&file(&framing, rest)), expected, "{framing} {rest}");
    }
}
