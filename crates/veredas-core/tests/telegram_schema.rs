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

/// LP's one field, to sit beside others in an envelope.
const LEN: &str = "{ name = \"length\", size = 2, role = \"length\" }";

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
    // The data frame it describes: one segment `AB`, with the id the prose names, sequence 7,
    // byte for byte (refute round 7: the id is read from the prose, not assumed).
    let prose = doc_prose();
    let id_text = prose.split("one segment `AB` with id `0x").nth(1).and_then(|s| s.split('`').next()).unwrap();
    let id = u64::from_str_radix(id_text, 16).unwrap();
    assert_eq!(id, 0x0102);
    let ab = Frame {
        fields: BTreeMap::new(),
        segments: vec![Segment { fields: BTreeMap::from([("id".to_string(), id)]), body: b"AB".to_vec() }],
    };
    let data = f.encode(&ab, Some(7)).unwrap();
    assert_eq!(
        data,
        [0xFD, 0, 1, 2, 7, 0, 0, 0, 6, 0, 1, 0, 2, 1, 2, 0, 0x41, 0x42, 0, 0, 0, 0xFE]
    );
    // And the prose says exactly these numbers.
    for phrase in ["an 11-byte envelope after it", "a 3-byte tail", "A keep-alive is therefore 16 bytes"] {
        assert!(prose.contains(phrase), "the documentation says `{phrase}`");
    }
    let quoted = prose.split("sequence 7 is `").nth(1).and_then(|s| s.split('`').next()).unwrap();
    assert_eq!(parse_hex(quoted).unwrap(), data);
    // Every other number of the prose, derived from the loaded file (refute round 6).
    let words = ["", "one", "two", "three"];
    let seg = f.segment.as_ref().unwrap();
    let seq = spec.sequence.unwrap();
    let field = |name: &str| f.envelope.iter().chain(&f.tail).find(|x| x.name == name).unwrap().size;
    assert_eq!((field("class"), field("source"), field("destination"), field("sequence"), field("check")), (1, 1, 1, 1, 1));
    for phrase in [
        format!("a class byte, source and destination nodes, a sequence number, {} spare bytes", words[field("spare")]),
        format!("segments of a {}-byte id, a {}-byte length and the body", seg[0].size, seg[1].size),
        format!("({} spare bytes, a check byte)", words[field("tail_spare")]),
        format!("numbers {} to {} after a {} on connect", seq.first, seq.last, seq.connect),
        format!("a keep-alive probe after {} s of silence", spec.keepalive.idle.as_secs()),
    ] {
        assert!(prose.contains(&phrase), "the documentation says `{phrase}`");
    }
}

/// What the documentation's key reference gives in parentheses after "`key` (", up to the first
/// comma or closing parenthesis.
fn documented(prose: &str, key: &str) -> String {
    let after = prose.split(&format!("`{key}` (")).nth(1).unwrap_or_else(|| panic!("`{key}` is documented"));
    after.split([',', ')']).next().unwrap().trim().to_string()
}

#[test]
fn the_key_reference_states_what_the_reader_does() {
    // Each default, limit and range the documentation states, against the reader (refute round
    // 6: the documentation could drift with the suite green).
    let prose = doc_prose();
    let spec = load(&file(LP, "")).unwrap();
    let seq = load(&file(
        "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"l\", size = 2, role = \"length\" } ]",
        "[sequence]\n",
    ))
    .unwrap()
    .sequence
    .unwrap();
    let mib = |s: String| s.strip_suffix(" MiB").map(|n| n.parse::<usize>().unwrap() * 1024 * 1024).unwrap();
    assert_eq!(documented(&prose, "connect"), seq.connect.to_string());
    assert_eq!(documented(&prose, "first"), seq.first.to_string());
    assert_eq!(documented(&prose, "last"), seq.last.to_string());
    assert_eq!(documented(&prose, "skip_duplicates"), seq.skip_duplicates.to_string());
    assert_eq!(documented(&prose, "keepalive_counts"), seq.keepalive_counts.to_string());
    assert_eq!(documented(&prose, "on_connect"), spec.keepalive.on_connect.to_string());
    assert_eq!(documented(&prose, "idle_ms"), spec.keepalive.idle.as_millis().to_string());
    assert_eq!(documented(&prose, "write_timeout_ms"), spec.link.write_timeout.as_millis().to_string());
    assert_eq!(mib(documented(&prose, "outbox_limit_bytes")), spec.link.outbox_limit_bytes);
    assert_eq!(documented(&prose, "max_peers"), spec.link.max_peers.to_string());
    // The frame limit: its default and its ceiling.
    let max_frame = prose.split("`max_frame` (").nth(1).unwrap();
    let default = max_frame.split(',').next().unwrap();
    let ceiling = max_frame.split("at most ").nth(1).unwrap().split(':').next().unwrap();
    assert_eq!((default.parse::<usize>().unwrap(), ceiling.parse::<usize>().unwrap()), (spec.framing.max_frame, MAX_MAX_FRAME));
    // The default role (its entry among the roles), and the field sizes. Each other default is
    // compared key by key in `every_key_is_required_or_takes_its_documented_default`.
    assert!(prose.contains("`\"opaque\"` (default)"));
    let opaque = load(&file(&format!("envelope = [ {{ name = \"x\", size = 1 }}, {LEN} ]"), "")).unwrap();
    assert_eq!(opaque.framing.envelope[0].role, FieldRole::Opaque);
    let sizes = prose.split("`size` (required; 1 to ").nth(1).unwrap().split(" bytes)").next().unwrap().parse::<usize>().unwrap();
    let sized = |n: usize| load(&file(&format!("envelope = [ {{ name = \"x\", size = {n} }}, {LEN} ]"), ""));
    assert!(sized(sizes).is_ok() && sized(1).is_ok() && sized(sizes + 1).is_err() && sized(0).is_err());
    // The version this reader knows, and the format it states.
    let known = prose.split("this reader knows `").nth(1).unwrap().split('`').next().unwrap();
    assert_eq!(known.parse::<u32>().unwrap(), VERSION);
    let format = prose.split("required: always `\"").nth(1).unwrap().split("\"`").next().unwrap();
    assert_eq!(format, FORMAT);
    // Each floor the key reference states, at its edge (refute round 7).
    let floor = |key: &str| {
        let entry = prose.split(&format!("`{key}` (")).nth(1).unwrap().split(')').next().unwrap();
        entry.split(", at least ").nth(1).unwrap_or_else(|| panic!("`{key}` states a floor")).to_string()
    };
    let n: u64 = floor("write_timeout_ms").parse().unwrap();
    assert!(load(&file(LP, &format!("[link]\nwrite_timeout_ms = {}", n - 1))).is_err());
    assert_eq!(load(&file(LP, &format!("[link]\nwrite_timeout_ms = {n}"))).unwrap().link.write_timeout, Duration::from_millis(n));
    let n: usize = floor("max_peers").parse().unwrap();
    assert!(load(&file(LP, &format!("[link]\nmax_peers = {}", n - 1))).is_err());
    assert_eq!(load(&file(LP, &format!("[link]\nmax_peers = {n}"))).unwrap().link.max_peers, n);
    assert_eq!(floor("outbox_limit_bytes"), "one `max_frame`");
    let framing = format!("max_frame = 2048\n{LP}");
    assert!(load(&file(&framing, "[link]\noutbox_limit_bytes = 2047")).is_err());
    assert_eq!(load(&file(&framing, "[link]\noutbox_limit_bytes = 2048")).unwrap().link.outbox_limit_bytes, 2048);
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
fn the_framing_byte_order_reaches_every_part_and_every_width_reaches_its_top() {
    // `byte_order` holds for every field that does not say otherwise: segment header and tail
    // included, byte for byte (refute round 5).
    let big = load(&file(
        "byte_order = \"big\"\n\
         envelope = [ { name = \"l\", size = 2, role = \"length\" }, { name = \"c\", size = 2, role = \"count\" } ]\n\
         segment = [ { name = \"id\", size = 2 }, { name = \"n\", size = 2, role = \"length\" } ]\n\
         tail = [ { name = \"t\", size = 2, value = 258 } ]",
        "",
    ))
    .unwrap()
    .framing;
    let parts = big.envelope.iter().chain(big.segment.as_ref().unwrap()).chain(&big.tail);
    assert!(parts.into_iter().all(|x| x.order == Endian::Big));
    let ab = Frame {
        fields: BTreeMap::new(),
        segments: vec![Segment { fields: BTreeMap::from([("id".to_string(), 0x0102)]), body: b"AB".to_vec() }],
    };
    assert_eq!(big.encode(&ab, None).unwrap(), [0, 6, 0, 1, 1, 2, 0, 2, 0x41, 0x42, 1, 2]);
    // An 8-byte opaque field holds u64::MAX; an 8-byte sequence takes its rules; the widest
    // value of a 1-byte field loads; a 2-byte sequence cycles to 65535.
    let l = "{ name = \"l\", size = 2, role = \"length\" }";
    let wide = load(&file(&format!("envelope = [ {{ name = \"x\", size = 8, value = 18446744073709551615 }}, {l} ]"), "")).unwrap();
    assert_eq!((wide.framing.envelope[0].size, wide.framing.envelope[0].value), (8, u64::MAX));
    assert_eq!(wide.framing.encode(&Frame::default(), None).unwrap(), [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
    let s8 = load(&file(&format!("envelope = [ {{ name = \"s\", size = 8, role = \"sequence\" }}, {l} ]"), "[sequence]\n")).unwrap();
    assert_eq!((s8.framing.envelope[0].size, s8.sequence.map(|s| s.last)), (8, Some(255)));
    let top = load(&file(&format!("envelope = [ {{ name = \"x\", size = 1, value = 255 }}, {l} ]"), "")).unwrap();
    assert_eq!(top.framing.envelope[0].value, 255);
    let s2 = load(&file(&format!("envelope = [ {{ name = \"s\", size = 2, role = \"sequence\" }}, {l} ]"), "[sequence]\nlast = 65535")).unwrap();
    assert_eq!(s2.sequence.map(|s| (s.first, s.last)), Some((1, 65535)));
    // A field's own byte order overrides the framing's in the segment header and the tail too,
    // and a segment's opaque field takes a default value (refute round 6).
    let mixed = load(&file(
        "byte_order = \"big\"\n\
         envelope = [ { name = \"l\", size = 2, role = \"length\" } ]\n\
         segment = [ { name = \"id\", size = 2, byte_order = \"little\", value = 7 }, { name = \"n\", size = 2, role = \"length\" } ]\n\
         tail = [ { name = \"t\", size = 2, byte_order = \"little\", value = 258 } ]",
        "",
    ))
    .unwrap()
    .framing;
    let header = mixed.segment.as_ref().unwrap();
    assert_eq!(
        (header[0].order, header[0].value, header[1].order, mixed.tail[0].order),
        (Endian::Little, 7, Endian::Big, Endian::Little)
    );
    // Multi-byte delimiters, each written as hex bytes.
    let wide = load(&file(&format!("start = \"02 03\"\nend = \"03 04\"\n{LP}"), "")).unwrap().framing;
    assert_eq!(wide.encode(&Frame::default(), None).unwrap(), [0x02, 0x03, 0, 0, 0x03, 0x04]);
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
    // Matched exactly: no version below the known one, no trimming, no folding of case.
    assert_eq!(
        refused("format = \"veredas-channel\"\nversion = 0\n[framing]\nenvelope = []"),
        "version 0 of `veredas-channel` is not known here (this reader knows 1)"
    );
    assert_eq!(refused("format = \" veredas-channel\"\nversion = 1"), "`format` is ` veredas-channel`, not `veredas-channel`");
    assert_eq!(refused("format = \"Veredas-Channel\"\nversion = 1"), "`format` is `Veredas-Channel`, not `veredas-channel`");
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
        (format!("end = \"\"\n{}", env(l)), "", "framing.end: an empty delimiter; leave it out instead"),
        // Each field check in the segment header and the tail, not only the envelope (round 6).
        (format!("{}\nsegment = [ {{ name = \"a\", size = 1 }}, {{ name = \"a\", size = 1, role = \"length\" }} ]", env(l)), "", "segment: two fields are named `a`"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 1 }}, {{ name = \"t\", size = 1 }} ]", env(l)), "", "tail: two fields are named `t`"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 9 }} ]", env(l)), "", "tail.t: size 9 is not 1 to 8 bytes"),
        (format!("{}\nsegment = [ {{ name = \"id\", size = 0 }}, {{ name = \"n\", size = 1, role = \"length\" }} ]", env(l)), "", "segment.id: size 0 is not 1 to 8 bytes"),
        (format!("{}\nsegment = [ {{ name = \"n\", size = 1, role = \"length\" }}, {{ name = \"m\", size = 1, role = \"length\" }} ]", env(l)), "", "segment: two fields share the role of `m`"),
        (format!("{}\nsegment = [ {{ name = \"n\", size = 1, role = \"length\", value = 3 }} ]", env(l)), "", "segment.n: a computed field takes no `value`"),
        (format!("{}\ntail = [ {{ name = \"t\", size = 1, value = 256 }} ]", env(l)), "", "tail.t: value 256 does not fit 1 byte(s)"),
        (format!("{}\nsegment = [ {{ name = \"id\", size = 1, value = 256 }}, {{ name = \"n\", size = 1, role = \"length\" }} ]", env(l)), "", "segment.id: value 256 does not fit 1 byte(s)"),
        (format!("{}\ntail = [ {{ name = \" \", size = 1 }} ]", env(l)), "", "tail: a field has no name"),
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

/// The names the reader accepts at one place: the backticked names after "expected" in its
/// refusal of an unknown key or variant there, sorted.
fn accepted(text: &str) -> Vec<String> {
    let e = refused(text);
    let line = e.lines().find(|l| l.contains("unknown field") || l.contains("unknown variant")).unwrap();
    let mut names: Vec<String> = line.split("expected ").nth(1).unwrap().split('`').skip(1).step_by(2).map(str::to_string).collect();
    names.sort();
    names
}

/// One block of the key reference: the text after its bold marker, up to the next bold text or
/// the closing paragraph ("Every key is checked").
fn block(prose: &str, marker: &str) -> String {
    let after = prose.split(marker).nth(1).unwrap_or_else(|| panic!("{marker} is documented"));
    after.split("**").next().unwrap().split("Every key is checked").next().unwrap().to_string()
}

/// The keys a block names: backticked lowercase names outside parentheses, each followed
/// directly by " (", ":" or ",", sorted.
fn named_keys(block: &str) -> Vec<String> {
    let chars: Vec<char> = block.chars().collect();
    let (mut out, mut depth, mut i) = (Vec::new(), 0i32, 0);
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '`' if depth == 0 => {
                if let Some(len) = chars[i + 1..].iter().position(|&c| c == '`') {
                    let name: String = chars[i + 1..i + 1 + len].iter().collect();
                    let next: String = chars[i + 2 + len..].iter().take(2).collect();
                    let follows = next.starts_with(" (") || next.starts_with(':') || next.starts_with(',');
                    if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') && follows {
                        out.push(name);
                    }
                    i += len + 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.sort();
    out
}

/// The quoted names in a text: every backticked `"name"`, without its quotes.
fn quoted(text: &str) -> Vec<String> {
    text.split('`').skip(1).step_by(2).filter_map(|t| t.strip_prefix('"')?.strip_suffix('"')).map(str::to_string).collect()
}

#[test]
fn the_key_reference_names_what_the_reader_accepts() {
    // Every section, key, role, byte order, part, flag and example the key reference names,
    // against what the reader accepts (refute round 8: a name could drift with the suite green).
    let prose = doc_prose();
    let keys = prose.split("## Keys").nth(1).unwrap().to_string();
    let seq_env = "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"length\", size = 2, role = \"length\" } ]";
    // The sections, and the header keys beside them.
    let sections: Vec<String> = keys.split("**`[").skip(1).map(|s| s.split("]`**").next().unwrap().to_string()).collect();
    let mut top = named_keys(&block(&keys, "**The header.**"));
    top.extend(sections.iter().cloned());
    top.sort();
    assert_eq!(top, accepted(&format!("zz = 1\n{}", file(LP, ""))));
    // Each table's keys.
    assert_eq!(named_keys(&block(&keys, "**`[framing]`**")), accepted(&file(&format!("zz = 1\n{LP}"), "")));
    assert_eq!(
        named_keys(&block(&keys, "**A field.**")),
        accepted(&file("envelope = [ { name = \"length\", size = 2, role = \"length\", zz = 1 } ]", ""))
    );
    assert_eq!(named_keys(&block(&keys, "**`[sequence]`**")), accepted(&file(seq_env, "[sequence]\nzz = 1")));
    assert_eq!(named_keys(&block(&keys, "**`[keepalive]`**")), accepted(&file(LP, "[keepalive]\nzz = 1")));
    assert_eq!(named_keys(&block(&keys, "**`[link]`**")), accepted(&file(LP, "[link]\nzz = 1")));
    // The roles, and the byte orders.
    let roles_block = block(&keys, "**Roles.**");
    let mut roles = quoted(&roles_block);
    roles.sort();
    assert_eq!(roles, accepted(&file("envelope = [ { name = \"length\", size = 2, role = \"zz\" } ]", "")));
    let framing_block = block(&keys, "**`[framing]`**");
    let order_entry = framing_block.split("`byte_order` (").nth(1).unwrap().split(" - ").next().unwrap();
    let mut orders = quoted(order_entry);
    orders.sort();
    orders.dedup();
    assert_eq!(orders, accepted(&file(&format!("byte_order = \"zz\"\n{LP}"), "")));
    // The parts each role is documented for: its annotation names them; with none, the parts
    // its entry names; with no part named, every part. The reader takes it exactly there.
    let base_envelope = "{ name = \"l\", size = 2, role = \"length\" }, { name = \"c\", size = 1, role = \"count\" }, { name = \"s\", size = 1, role = \"sequence\" }";
    let parts = ["envelope", "segment", "tail"];
    for role in &roles {
        let entry = roles_block.split(&format!("`\"{role}\"`")).nth(1).unwrap().split(" - ").next().unwrap();
        let documented: Vec<&str> = match entry.strip_prefix(" (").and_then(|a| a.split(')').next()) {
            Some(note) if note != "default" => note.split(", ").collect(),
            _ => {
                let named: Vec<&str> = parts.iter().copied().filter(|p| entry.contains(&format!("in the {p}")) || entry.contains(&format!("in a {p}"))).collect();
                if named.is_empty() { parts.to_vec() } else { named }
            }
        };
        let x = format!("{{ name = \"x\", size = 1, role = \"{role}\" }}");
        let takes: Vec<&str> = parts
            .iter()
            .copied()
            .filter(|part| {
                let (envelope, segment, tail) = match *part {
                    "envelope" => (format!("{base_envelope}, {x}"), "{ name = \"n\", size = 1, role = \"length\" }".to_string(), String::new()),
                    "segment" => (base_envelope.to_string(), format!("{{ name = \"n\", size = 1, role = \"length\" }}, {x}"), String::new()),
                    _ => (base_envelope.to_string(), "{ name = \"n\", size = 1, role = \"length\" }".to_string(), x.clone()),
                };
                let text = file(&format!("envelope = [ {envelope} ]\nsegment = [ {segment} ]\ntail = [ {tail} ]"), "[sequence]\n");
                load(&text).err().is_none_or(|e| !e.to_string().contains(&format!("{part}.x: this part takes no such role")))
            })
            .collect();
        assert_eq!(takes, documented, "the role `{role}`");
    }
    // Each section flagged required is refused when absent; each flagged optional loads absent
    // (`[sequence]` is required only with a sequence field).
    for section in &sections {
        let flag = block(&keys, &format!("**`[{section}]`**"));
        let flag = flag.trim_start().strip_prefix('(').unwrap().split(')').next().unwrap();
        let without = if section == "framing" { "format = \"veredas-channel\"\nversion = 1\n".to_string() } else { file(LP, "") };
        match flag {
            "required" => assert!(load(&without).is_err(), "[{section}] is required"),
            "optional" => assert!(load(&without).is_ok(), "[{section}] is optional"),
            f if f.starts_with("required with an envelope field of role `\"sequence\"`") => {
                assert!(load(&without).is_ok() && load(&file(seq_env, "")).is_err() && load(&file(seq_env, "[sequence]")).is_ok());
            }
            other => panic!("[{section}] is flagged `{other}`"),
        }
    }
    // The delimiter examples load as delimiters, and the frame limit counts them.
    let delimiter_entry = framing_block.split("`start`, `end` (").nth(1).unwrap().split(" - ").next().unwrap();
    let examples = quoted(delimiter_entry);
    assert_eq!(examples.len(), 2);
    for hex in &examples {
        assert!(load(&file(&format!("start = \"{hex}\"\nend = \"{hex}\"\n{LP}"), "")).is_ok(), "the example `{hex}`");
    }
    let limit_entry = framing_block.split("`max_frame` (").nth(1).unwrap().split(" - ").next().unwrap();
    assert!(limit_entry.contains("): the largest frame in bytes, delimiters included."));
    assert!(load(&file(&format!("max_frame = 4\nstart = \"FD\"\nend = \"FE\"\n{LP}"), "")).is_ok());
    assert!(load(&file(&format!("max_frame = 3\nstart = \"FD\"\nend = \"FE\"\n{LP}"), "")).is_err());
}

/// What a key's entry states in parentheses, its first clause (up to a comma or semicolon):
/// "required...", or the value the key takes when left out. A key listed before another
/// (`start`, `end` (none)) shares that one's parentheses.
fn status(block: &str, key: &str) -> String {
    let needle = format!("`{key}`");
    let mut from = 0;
    while let Some(at) = block[from..].find(&needle) {
        let start = from + at;
        let depth = block[..start].matches('(').count() as i64 - block[..start].matches(')').count() as i64;
        let mut rest = &block[start + needle.len()..];
        while let Some(r) = rest.strip_prefix(", `") {
            rest = r.split_once('`').map_or(r, |(_, after)| after);
        }
        if depth == 0
            && let Some(inner) = rest.strip_prefix(" (")
        {
            let mut level = 1;
            let close = inner
                .char_indices()
                .find(|&(_, c)| {
                    level += match c {
                        '(' => 1,
                        ')' => -1,
                        _ => 0,
                    };
                    level == 0
                })
                .map(|(i, _)| i)
                .unwrap();
            return inner[..close].split([',', ';']).next().unwrap().trim().to_string();
        }
        from = start + needle.len();
    }
    panic!("`{key}` states neither that it is required nor its default");
}

#[test]
fn every_key_is_required_or_takes_its_documented_default() {
    // For every key the reader knows, in every table: a file without it is refused when the key
    // reference says it is required, and otherwise loads with exactly the value the reference
    // states (refute round 9: `tail` and `value` stated none).
    let prose = doc_prose();
    let keys = prose.split("## Keys").nth(1).unwrap().to_string();
    let seq_env = "envelope = [ { name = \"s\", size = 1, role = \"sequence\" }, { name = \"length\", size = 2, role = \"length\" } ]";
    let order = |o: Endian| if o == Endian::Little { "`\"little\"`".to_string() } else { "`\"big\"`".to_string() };
    let present = |b: bool| if b { "some".to_string() } else { "none".to_string() };
    let mib = 1024 * 1024;
    // Each table: its reference block, every key stated, the file built from the statements
    // kept, and what the loaded spec says for a key left out (None: nothing to compare).
    type Build = Box<dyn Fn(&[&str]) -> String>;
    type Render = Box<dyn Fn(&str, &ChannelSpec) -> Option<String>>;
    type Table = (&'static str, String, Vec<(&'static str, String)>, Build, Render);
    let tables: Vec<Table> = vec![
        (
            "top",
            block(&keys, "**The header.**"),
            vec![
                ("format", "format = \"veredas-channel\"".into()),
                ("version", "version = 1".into()),
                ("framing", format!("[framing]\n{seq_env}")),
                ("sequence", "[sequence]".into()),
                ("keepalive", "[keepalive]".into()),
                ("link", "[link]".into()),
            ],
            Box::new(|kept| kept.join("\n")),
            Box::new(|_, _| None),
        ),
        (
            "framing",
            block(&keys, "**`[framing]`**"),
            vec![
                ("byte_order", "byte_order = \"big\"".into()),
                ("start", "start = \"01\"".into()),
                ("end", "end = \"02\"".into()),
                ("max_frame", "max_frame = 2048".into()),
                ("envelope", "envelope = [ { name = \"l\", size = 2, role = \"length\" } ]".into()),
                ("segment", "segment = [ { name = \"n\", size = 1, role = \"length\" } ]".into()),
                ("tail", "tail = [ { name = \"t\", size = 1 } ]".into()),
            ],
            Box::new(|kept| file(&kept.join("\n"), "")),
            Box::new(move |key, spec| {
                let f = &spec.framing;
                Some(match key {
                    "byte_order" => order(f.envelope[0].order),
                    "start" => present(!f.start.is_empty()),
                    "end" => present(!f.end.is_empty()),
                    "max_frame" => f.max_frame.to_string(),
                    "segment" => present(f.segment.is_some()),
                    "tail" => present(!f.tail.is_empty()),
                    other => panic!("no default to render for `{other}`"),
                })
            }),
        ),
        (
            "field",
            block(&keys, "**A field.**"),
            vec![
                ("name", "name = \"x\"".into()),
                ("size", "size = 1".into()),
                ("role", "role = \"opaque\"".into()),
                ("byte_order", "byte_order = \"little\"".into()),
                ("value", "value = 3".into()),
            ],
            Box::new(|kept| file(&format!("byte_order = \"big\"\nenvelope = [ {{ {} }}, {LEN} ]", kept.join(", ")), "")),
            Box::new(move |key, spec| {
                let (x, plain) = (&spec.framing.envelope[0], &spec.framing.envelope[1]);
                Some(match key {
                    "role" => format!("`\"{}\"`", format!("{:?}", x.role).to_lowercase()),
                    "byte_order" if x.order == plain.order => "the framing's".to_string(),
                    "byte_order" => order(x.order),
                    "value" => x.value.to_string(),
                    other => panic!("no default to render for `{other}`"),
                })
            }),
        ),
        (
            "sequence",
            block(&keys, "**`[sequence]`**"),
            vec![
                ("connect", "connect = 0".into()),
                ("first", "first = 2".into()),
                ("last", "last = 9".into()),
                ("skip_duplicates", "skip_duplicates = false".into()),
                ("keepalive_counts", "keepalive_counts = false".into()),
            ],
            Box::new(move |kept| file(seq_env, &format!("[sequence]\n{}", kept.join("\n")))),
            Box::new(|key, spec| {
                let s = spec.sequence.as_ref().unwrap();
                Some(match key {
                    "connect" => s.connect.to_string(),
                    "first" => s.first.to_string(),
                    "last" => s.last.to_string(),
                    "skip_duplicates" => s.skip_duplicates.to_string(),
                    "keepalive_counts" => s.keepalive_counts.to_string(),
                    other => panic!("no default to render for `{other}`"),
                })
            }),
        ),
        (
            "keepalive",
            block(&keys, "**`[keepalive]`**"),
            vec![("on_connect", "on_connect = false".into()), ("idle_ms", "idle_ms = 5".into())],
            Box::new(|kept| file(LP, &format!("[keepalive]\n{}", kept.join("\n")))),
            Box::new(|key, spec| {
                Some(match key {
                    "on_connect" => spec.keepalive.on_connect.to_string(),
                    "idle_ms" => spec.keepalive.idle.as_millis().to_string(),
                    other => panic!("no default to render for `{other}`"),
                })
            }),
        ),
        (
            "link",
            block(&keys, "**`[link]`**"),
            vec![
                ("write_timeout_ms", "write_timeout_ms = 5".into()),
                ("outbox_limit_bytes", "outbox_limit_bytes = 70000".into()),
                ("max_peers", "max_peers = 2".into()),
            ],
            Box::new(|kept| file(LP, &format!("[link]\n{}", kept.join("\n")))),
            Box::new(move |key, spec| {
                let l = &spec.link;
                Some(match key {
                    "write_timeout_ms" => l.write_timeout.as_millis().to_string(),
                    "outbox_limit_bytes" if l.outbox_limit_bytes % mib == 0 => format!("{} MiB", l.outbox_limit_bytes / mib),
                    "outbox_limit_bytes" => l.outbox_limit_bytes.to_string(),
                    "max_peers" => l.max_peers.to_string(),
                    other => panic!("no default to render for `{other}`"),
                })
            }),
        ),
    ];
    let reader_keys = |table: &str| match table {
        "top" => accepted(&format!("zz = 1\n{}", file(LP, ""))),
        "framing" => accepted(&file(&format!("zz = 1\n{LP}"), "")),
        "field" => accepted(&file("envelope = [ { name = \"length\", size = 2, role = \"length\", zz = 1 } ]", "")),
        "sequence" => accepted(&file(seq_env, "[sequence]\nzz = 1")),
        "keepalive" => accepted(&file(LP, "[keepalive]\nzz = 1")),
        _ => accepted(&file(LP, "[link]\nzz = 1")),
    };
    let mut checked = 0;
    for (table, doc, statements, build, render) in &tables {
        let all: Vec<&str> = statements.iter().map(|(_, s)| s.as_str()).collect();
        assert!(load(&build(&all)).is_ok(), "{table}: the file stating every key loads");
        let mut stated: Vec<String> = statements.iter().map(|(k, _)| k.to_string()).collect();
        stated.sort();
        assert_eq!(stated, reader_keys(table), "{table}: every key the reader knows is stated here");
        for (key, _) in statements {
            let documented = if *table == "top" && !["format", "version"].contains(key) {
                let flag = block(&keys, &format!("**`[{key}]`**"));
                flag.trim_start().strip_prefix('(').unwrap().split([')', ',']).next().unwrap().to_string()
            } else {
                status(doc, key)
            };
            let kept: Vec<&str> = statements.iter().filter(|(k, _)| k != key).map(|(_, s)| s.as_str()).collect();
            let without = load(&build(&kept));
            if documented.starts_with("required") {
                assert!(without.is_err(), "{table}.{key} is documented required, yet loads without");
            } else {
                let spec = without.unwrap_or_else(|e| panic!("{table}.{key} is documented `{documented}`, yet: {e}"));
                if let Some(value) = render(key, &spec) {
                    assert_eq!(value, documented, "{table}.{key}: the default the reference states");
                }
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 28);
}
