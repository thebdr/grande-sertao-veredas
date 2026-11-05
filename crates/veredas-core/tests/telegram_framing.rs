//! The framing engine (contract: the framing clause), on synthetic layouts read from channel
//! files: the segmented envelope (a start byte, an 11-byte envelope with a sequence number, a
//! data-area length and a segment count, segments of id, length and body, a 3-byte tail, an end
//! byte: the layout Veredas 1's telegram codec hard-coded, now a configuration), a bare
//! length-prefixed layout, and a delimited layout counted by segments. Veredas 1's codec tests
//! come along against the first, with golden frames written byte by byte.

use std::collections::BTreeMap;

use veredas_core::telegram::framing::*;
use veredas_core::telegram::schema::load;

/// The segmented envelope, as a channel file, in one byte order.
fn envelope_layout(order: &str, max_frame: Option<usize>) -> Framing {
    let max = max_frame.map(|m| format!("max_frame = {m}\n")).unwrap_or_default();
    let text = format!(
        r#"
format = "veredas-channel"
version = 1

[framing]
byte_order = "{order}"
start = "FD"
end = "FE"
{max}envelope = [
  {{ name = "class", size = 1 }},
  {{ name = "source", size = 1, value = 1 }},
  {{ name = "destination", size = 1, value = 2 }},
  {{ name = "sequence", size = 1, role = "sequence" }},
  {{ name = "spare", size = 3 }},
  {{ name = "length", size = 2, role = "length" }},
  {{ name = "count", size = 2, role = "count" }},
]
segment = [ {{ name = "id", size = 2 }}, {{ name = "length", size = 2, role = "length" }} ]
tail = [ {{ name = "tail_spare", size = 2 }}, {{ name = "check", size = 1 }} ]

[sequence]
"#
    );
    load(&text).unwrap().framing
}

fn le() -> Framing {
    envelope_layout("little", None)
}

fn seg(id: u64, body: &[u8]) -> Segment {
    Segment { fields: BTreeMap::from([("id".to_string(), id)]), body: body.to_vec() }
}

/// What a decoded frame of the envelope layout holds for the given envelope and segments.
fn decoded(seq: u64, segments: Vec<Segment>) -> Frame {
    let data: usize = segments.iter().map(|s| 4 + s.body.len()).sum();
    let mut fields = BTreeMap::new();
    for (k, v) in [
        ("class", 0),
        ("source", 1),
        ("destination", 2),
        ("sequence", seq),
        ("spare", 0),
        ("length", data as u64),
        ("count", segments.len() as u64),
        ("tail_spare", 0),
        ("check", 0),
    ] {
        fields.insert(k.to_string(), v);
    }
    let segments = segments
        .into_iter()
        .map(|mut s| {
            s.fields.insert("length".into(), s.body.len() as u64);
            s
        })
        .collect();
    Frame { fields, segments }
}

fn frame(segments: Vec<Segment>) -> Frame {
    Frame { fields: BTreeMap::new(), segments }
}

/// The frame of `[seg(0x0102, "AB")]` with sequence 7, field by field.
fn golden(little: bool) -> Vec<u8> {
    let u16b = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut f = vec![0xFD, 0, 1, 2, 7, 0, 0, 0]; // start class source destination sequence spare*3
    f.extend(u16b(6)); // length: segment header (4) + body (2)
    f.extend(u16b(1)); // count
    f.extend(u16b(0x0102)); // id
    f.extend(u16b(2)); // segment length
    f.extend(b"AB");
    f.extend([0, 0, 0, 0xFE]); // tail_spare*2 check end
    f
}

#[test]
fn the_frame_is_the_configured_layout_byte_for_byte() {
    let t = frame(vec![seg(0x0102, b"AB")]);
    assert_eq!(
        le().encode(&t, Some(7)).unwrap(),
        [0xFD, 0, 1, 2, 7, 0, 0, 0, 6, 0, 1, 0, 0x02, 0x01, 2, 0, b'A', b'B', 0, 0, 0, 0xFE]
    );
    assert_eq!(le().encode(&t, Some(7)).unwrap(), golden(true));
    let be = envelope_layout("big", None);
    assert_eq!(be.encode(&t, Some(7)).unwrap(), golden(false));
    for (little, layout) in [(true, le()), (false, be)] {
        let mut stream = golden(little);
        stream.extend(golden(little)); // a second frame behind it is left alone
        assert_eq!(layout.decode(&stream), Ok((decoded(7, vec![seg(0x0102, b"AB")]), 22)));
    }
}

#[test]
fn a_frame_survives_a_round_trip_in_both_byte_orders() {
    for layout in [le(), envelope_layout("big", None)] {
        let t = frame(vec![seg(0x0102, b"POS=42"), seg(9, &[0xDE, 0xAD])]);
        let bytes = layout.encode(&t, Some(200)).unwrap();
        let (back, used) = layout.decode(&bytes).unwrap();
        assert_eq!(back, decoded(200, vec![seg(0x0102, b"POS=42"), seg(9, &[0xDE, 0xAD])]));
        assert_eq!(used, bytes.len());
        assert_eq!(layout.sequence_of(&back), Some(200));
    }
}

#[test]
fn opaque_fields_take_the_frame_value_or_their_default_and_the_sequence_is_stamped() {
    let mut t = frame(vec![]);
    t.fields.insert("class".into(), 0xAA);
    t.fields.insert("source".into(), 0x10);
    t.fields.insert("check".into(), 0x33);
    t.fields.insert("sequence".into(), 9);
    let bytes = le().encode(&t, None).unwrap();
    // destination falls back to its configured 2; the frame's own sequence 9 is used.
    assert_eq!(bytes, [0xFD, 0xAA, 0x10, 2, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x33, 0xFE]);
    // A stamp given at encode wins over the frame's.
    assert_eq!(le().encode(&t, Some(4)).unwrap()[4], 4);
    // A keep-alive is the envelope and the tail: sixteen bytes, zero lengths read the same both ways.
    let (back, used) = envelope_layout("big", None).decode(&bytes).unwrap();
    assert_eq!(used, 16);
    assert!(back.is_keepalive());
    assert_eq!((back.fields["class"], back.fields["check"]), (0xAA, 0x33));
}

#[test]
fn a_value_that_does_not_fit_its_field_is_refused_never_clamped() {
    // A body MSG LENGTH cannot describe.
    assert_eq!(
        le().encode(&frame(vec![seg(3, &vec![0; 65_536])]), None),
        Err(EncodeError::DoesNotFit { field: "length".into(), value: 65_536, size: 2 })
    );
    // A data area LENGTH cannot describe (the frame limit raised so it is the field that refuses).
    let roomy = envelope_layout("little", Some(1 << 20));
    let many = frame((0..3).map(|i| seg(i, &vec![0; 30_000])).collect());
    assert_eq!(
        roomy.encode(&many, None),
        Err(EncodeError::DoesNotFit { field: "length".into(), value: 90_012, size: 2 })
    );
    // An opaque value or a sequence too wide for its byte.
    let mut t = frame(vec![]);
    t.fields.insert("class".into(), 256);
    assert_eq!(le().encode(&t, None), Err(EncodeError::DoesNotFit { field: "class".into(), value: 256, size: 1 }));
    assert_eq!(
        le().encode(&frame(vec![]), Some(256)),
        Err(EncodeError::DoesNotFit { field: "sequence".into(), value: 256, size: 1 })
    );
    assert_eq!(
        EncodeError::DoesNotFit { field: "length".into(), value: 65_536, size: 2 }.to_string(),
        "length = 65536 does not fit its 2-byte field"
    );
}

#[test]
fn the_frame_limit_sits_at_its_exact_edge_both_ways() {
    // 16 fixed bytes (start, envelope, segment header, tail, end): the largest body fills the rest.
    let body = 65_536 - 16 - 4;
    let big = frame(vec![seg(9, &vec![0x5A; body])]);
    let bytes = le().encode(&big, None).unwrap();
    assert_eq!(bytes.len(), 65_536);
    assert_eq!(le().decode(&bytes).map(|(_, used)| used), Ok(65_536));
    assert_eq!(
        le().encode(&frame(vec![seg(9, &vec![0; body + 1])]), None),
        Err(EncodeError::FrameTooLarge { len: 65_537, max: 65_536 })
    );
    // On decode: a LENGTH of 65 520 can still be a valid frame and waits; 65 521 cannot and is
    // refused at once, its resync past the buffer.
    let head = |len: u16| {
        let mut h = vec![0xFD, 0, 0, 0, 0, 0, 0, 0];
        h.extend(len.to_le_bytes());
        h.extend([1, 0]);
        h
    };
    assert_eq!(le().decode(&head(65_520)), Err(DecodeError::NeedMore));
    assert_eq!(le().decode(&head(65_521)), Err(DecodeError::BadFrame { resync: 12 }));
}

#[test]
fn every_prefix_of_a_valid_frame_waits() {
    let t = frame(vec![seg(1, b"ONE"), seg(2, b"")]);
    let bytes = le().encode(&t, Some(3)).unwrap();
    for cut in 0..bytes.len() {
        assert_eq!(le().decode(&bytes[..cut]), Err(DecodeError::NeedMore), "cut {cut}");
    }
    assert_eq!(le().decode(&bytes).map(|(_, used)| used), Ok(bytes.len()));
}

#[test]
fn a_bad_frame_says_exactly_how_far_to_skip() {
    let good = golden(true);
    let mut buf = vec![0x01, 0x02];
    buf.extend(&good);
    assert_eq!(le().decode(&buf), Err(DecodeError::BadFrame { resync: 2 }));
    assert_eq!(le().decode(&[0x00, 0x01, 0x02]), Err(DecodeError::BadFrame { resync: 3 }));
    // A broken end delimiter: skip past this start to the next one.
    let mut broken = good.clone();
    let last = broken.len() - 1;
    broken[last] = 0x00;
    broken.extend(&good);
    assert_eq!(le().decode(&broken), Err(DecodeError::BadFrame { resync: 22 }));
    assert_eq!(
        DecodeError::BadFrame { resync: 3 }.to_string(),
        "not a frame: skip 3 byte(s) to the next start"
    );
}

#[test]
fn lengths_that_disagree_are_refused_once_the_header_that_shows_it_is_whole() {
    let mut two_good = golden(true);
    two_good.extend(golden(true));
    // LENGTH 60 000, one segment of 2 bytes: refused now, not after 60 KB more.
    let mut buf = vec![0xFD, 0, 0, 0, 0, 0, 0, 0, 0x60, 0xEA, 1, 0, 1, 0, 2, 0];
    buf.extend(&two_good);
    assert_eq!(le().decode(&buf), Err(DecodeError::BadFrame { resync: 16 }));
    assert!(le().decode(&buf[16..]).is_ok());
    // A frame whose body holds a start byte, its end broken: the resync lands on that start,
    // whose bytes read as a 61 440-byte frame with an empty segment, refused at once.
    let t = frame(vec![seg(1, &[0xFD, 0, 0, 0, 0, 0, 0, 0, 0, 0xF0, 1, 0])]);
    let mut stream = le().encode(&t, None).unwrap();
    let last = stream.len() - 1;
    stream[last] = 0;
    stream.extend(&two_good);
    assert_eq!(le().decode(&stream), Err(DecodeError::BadFrame { resync: 16 }));
    assert_eq!(le().decode(&stream[16..]), Err(DecodeError::BadFrame { resync: 16 }));
    assert!(le().decode(&stream[32..]).is_ok());
    // Three segment headers cannot fit an 8-byte data area.
    assert_eq!(le().decode(&[0xFD, 0, 0, 0, 0, 0, 0, 0, 8, 0, 3, 0]), Err(DecodeError::BadFrame { resync: 12 }));
    // A data area with no segment to describe it.
    let mut phantom = vec![0xFD, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFE];
    assert_eq!(le().decode(&phantom), Err(DecodeError::BadFrame { resync: 20 }));
    phantom.truncate(12);
    assert_eq!(le().decode(&phantom), Err(DecodeError::BadFrame { resync: 12 }));
    // Slack after the last segment.
    let mut slack = golden(true);
    slack[8] += 2;
    slack.splice(18..18, [0xAA, 0xBB]);
    assert_eq!(le().decode(&slack), Err(DecodeError::BadFrame { resync: 24 }));
}

#[test]
fn the_remaining_headers_check_holds_on_both_sides_of_its_boundary() {
    // LENGTH 8, two segments, the first 1 byte long: 3 bytes left for a 4-byte header.
    let short = [0xFD, 0, 0, 0, 0, 0, 0, 0, 8, 0, 2, 0, 1, 0, 1, 0];
    assert_eq!(le().decode(&short), Err(DecodeError::BadFrame { resync: 16 }));
    // The first segment empty: exactly 4 bytes left, a valid frame could still follow.
    let mut exact = short;
    exact[14] = 0;
    assert_eq!(le().decode(&exact), Err(DecodeError::NeedMore));
}

#[test]
fn a_wrong_byte_order_reads_as_bad_frames_not_phantom_data() {
    let bytes = le().encode(&frame(vec![seg(1, b"payload")]), Some(7)).unwrap();
    let be = envelope_layout("big", None);
    // Read big-endian, LENGTH says 2816 and the count 256: one frame is a plausible prefix and
    // waits; repeated, the second segment header shows the lengths cannot add up.
    assert_eq!(be.decode(&bytes), Err(DecodeError::NeedMore));
    assert_eq!(be.decode(&bytes.repeat(68)), Err(DecodeError::BadFrame { resync: 27 }));
}

/// A bare layout: a big-endian two-byte length, then the body; no delimiter, no segments.
fn length_prefixed() -> Framing {
    load(
        r#"
format = "veredas-channel"
version = 1
[framing]
byte_order = "big"
max_frame = 1024
envelope = [ { name = "length", size = 2, role = "length" } ]
"#,
    )
    .unwrap()
    .framing
}

#[test]
fn a_length_prefixed_layout_carries_one_body() {
    let lp = length_prefixed();
    let one = frame(vec![Segment { fields: BTreeMap::new(), body: b"HELLO".to_vec() }]);
    let bytes = lp.encode(&one, None).unwrap();
    assert_eq!(bytes, [0, 5, b'H', b'E', b'L', b'L', b'O']);
    let (back, used) = lp.decode(&bytes).unwrap();
    assert_eq!((back.segments.len(), back.segments[0].body.as_slice(), used), (1, &b"HELLO"[..], 7));
    // An empty body is a keep-alive.
    let ka = lp.encode(&Frame::default(), None).unwrap();
    assert_eq!(ka, [0, 0]);
    assert!(lp.decode(&ka).unwrap().0.is_keepalive());
    // Two bodies do not fit a layout that carries one.
    assert_eq!(lp.encode(&frame(vec![Segment::default(), Segment::default()]), None), Err(EncodeError::TooManySegments(2)));
    // Past the frame limit, with no start byte to look for, one byte is skipped.
    assert_eq!(lp.decode(&[0xFF, 0xFF, 0]), Err(DecodeError::BadFrame { resync: 1 }));
    assert_eq!(lp.decode(&[0]), Err(DecodeError::NeedMore));
}

/// A delimited layout counted by segments, with no data-area length.
fn counted() -> Framing {
    load(
        r#"
format = "veredas-channel"
version = 1
[framing]
start = "02"
end = "03"
envelope = [ { name = "count", size = 1, role = "count" } ]
segment = [ { name = "length", size = 1, role = "length" } ]
"#,
    )
    .unwrap()
    .framing
}

#[test]
fn a_layout_counted_by_segments_ends_where_its_segments_do() {
    let c = counted();
    let t = frame(vec![Segment { fields: BTreeMap::new(), body: b"AB".to_vec() }, Segment { fields: BTreeMap::new(), body: b"C".to_vec() }]);
    let bytes = c.encode(&t, None).unwrap();
    assert_eq!(bytes, [0x02, 2, 2, b'A', b'B', 1, b'C', 0x03]);
    let (back, used) = c.decode(&bytes).unwrap();
    assert_eq!(used, 8);
    assert_eq!(back.segments.iter().map(|s| s.body.clone()).collect::<Vec<_>>(), [b"AB".to_vec(), b"C".to_vec()]);
    for cut in 0..bytes.len() {
        assert_eq!(c.decode(&bytes[..cut]), Err(DecodeError::NeedMore), "cut {cut}");
    }
    // The end delimiter missing: skip to the next start byte (one segment, so no field value
    // in between reads as a start).
    let one = c.encode(&frame(vec![Segment { fields: BTreeMap::new(), body: b"ABC".to_vec() }]), None).unwrap();
    assert_eq!(one, [0x02, 1, 3, b'A', b'B', b'C', 0x03]);
    let mut bad = one.clone();
    bad[6] = 0x04;
    bad.extend(&one);
    assert_eq!(c.decode(&bad), Err(DecodeError::BadFrame { resync: 7 }));
    // A layout whose start byte can also be a value resyncs to the first such byte.
    let mut ambiguous = bytes.clone();
    ambiguous[7] = 0x04;
    assert_eq!(c.decode(&ambiguous), Err(DecodeError::BadFrame { resync: 1 }));
}

#[test]
fn hex_is_two_ascii_digits_per_byte_and_nothing_else() {
    assert_eq!(parse_hex("").unwrap(), Vec::<u8>::new());
    assert_eq!(parse_hex("fd 00\t7f\nFE").unwrap(), vec![0xFD, 0x00, 0x7F, 0xFE]);
    assert_eq!(parse_hex("ABC"), Err(HexError::OddDigits));
    assert_eq!(parse_hex("+F"), Err(HexError::NotHex("+F".into())));
    assert_eq!(parse_hex("é"), Err(HexError::OddDigits));
    assert_eq!(parse_hex("éé"), Err(HexError::NotHex("éé".into())));
    assert_eq!(HexError::NotHex("0g".into()).to_string(), "`0g` is not a hex byte");
}

#[test]
fn previews_and_dumps_render_exactly() {
    assert_eq!(preview(b""), "");
    assert_eq!(preview(&[b'x'; 48]), "x".repeat(48));
    assert_eq!(preview(&[b'x'; 49]), format!("{}…", "x".repeat(48)));
    assert_eq!(
        preview(&(0u8..17).collect::<Vec<_>>()),
        "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F …"
    );
    assert_eq!(hex_dump(b""), "");
    assert_eq!(
        hex_dump(b"0123456789abcdef\x7f"),
        format!(
            "0000  30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66  0123456789abcdef\n0010  7F{}  .\n",
            " ".repeat(45)
        )
    );
}

// ---------------------------------------------------------------------------
// Refute round 4 (C-010): a field may be eight bytes wide, so every length and count a peer
// declares is bounded by the frame limit before any arithmetic, and a declared count or length
// that can never fit is refused at once, never waited for. Each limit is pinned on both sides.
// ---------------------------------------------------------------------------

/// A channel file's framing, from its `[framing]` body.
fn framing_of(body: &str) -> Framing {
    load(&format!("format = \"veredas-channel\"\nversion = 1\n[framing]\n{body}")).unwrap().framing
}

#[test]
fn an_eight_byte_envelope_length_is_bounded_before_it_is_added() {
    let lp = framing_of("max_frame = 1024\nenvelope = [ { name = \"length\", size = 8, role = \"length\" } ]\n");
    // u64::MAX would wrap `8 + length`: refused, one byte skipped (no start delimiter).
    let mut hostile = vec![0xFF; 8];
    hostile.extend([1, 2, 3]);
    assert_eq!(lp.decode(&hostile), Err(DecodeError::BadFrame { resync: 1 }));
    assert_eq!(lp.decode(&(1u64 << 63).to_le_bytes()), Err(DecodeError::BadFrame { resync: 1 }));
    // 8 + 1016 is exactly the limit and waits; 1017 is one over and is refused.
    assert_eq!(lp.decode(&1016u64.to_le_bytes()), Err(DecodeError::NeedMore));
    assert_eq!(lp.decode(&1017u64.to_le_bytes()), Err(DecodeError::BadFrame { resync: 1 }));
    let mut whole = 1016u64.to_le_bytes().to_vec();
    whole.extend(vec![0x5A; 1016]);
    assert_eq!(lp.decode(&whole).map(|(f, used)| (f.segments[0].body.len(), used)), Ok((1016, 1024)));
}

#[test]
fn an_eight_byte_segment_length_is_bounded_before_it_is_added() {
    let c = framing_of(
        "max_frame = 1024\nenvelope = [ { name = \"count\", size = 1, role = \"count\" } ]\n\
         segment = [ { name = \"length\", size = 8, role = \"length\" } ]\n",
    );
    for hostile in [u64::MAX, 1 << 63, 1025] {
        let mut buf = vec![1];
        buf.extend(hostile.to_le_bytes());
        assert_eq!(c.decode(&buf), Err(DecodeError::BadFrame { resync: 1 }), "{hostile}");
    }
    // 1 + 8 + 1015 is exactly the limit and waits; 1016 is one over.
    let head = |len: u64| {
        let mut b = vec![1];
        b.extend(len.to_le_bytes());
        b
    };
    assert_eq!(c.decode(&head(1015)), Err(DecodeError::NeedMore));
    assert_eq!(c.decode(&head(1016)), Err(DecodeError::BadFrame { resync: 1 }));
}

#[test]
fn an_eight_byte_count_is_bounded_before_it_is_multiplied() {
    let c = framing_of(
        "max_frame = 1024\nenvelope = [ { name = \"count\", size = 8, role = \"count\" } ]\n\
         segment = [ { name = \"length\", size = 2, role = \"length\" } ]\n",
    );
    // 2^63 two-byte headers would wrap `count * 2`; u64::MAX would wrap any sum.
    for hostile in [1u64 << 63, u64::MAX, 1025] {
        assert_eq!(c.decode(&hostile.to_le_bytes()), Err(DecodeError::BadFrame { resync: 1 }), "{hostile}");
    }
    // 8 + 508 headers of 2 bytes is exactly the limit and waits; 509 headers cannot fit.
    assert_eq!(c.decode(&508u64.to_le_bytes()), Err(DecodeError::NeedMore));
    assert_eq!(c.decode(&509u64.to_le_bytes()), Err(DecodeError::BadFrame { resync: 1 }));
}

/// A delimited layout counted by two-byte segment counts, its frames at most 1024 bytes.
fn counted_small() -> Framing {
    framing_of(
        "start = \"02\"\nend = \"03\"\nmax_frame = 1024\n\
         envelope = [ { name = \"count\", size = 2, role = \"count\" } ]\n\
         segment = [ { name = \"length\", size = 2, role = \"length\" } ]\n",
    )
}

#[test]
fn a_count_whose_headers_cannot_fit_is_refused_at_once() {
    let c = counted_small();
    // 3000 segments: refused from the three bytes that declare them, the resync past the buffer
    // (no start byte after the first), not after a wait for 6 KB that could never be a frame.
    assert_eq!(c.decode(&[0x02, 0xB8, 0x0B]), Err(DecodeError::BadFrame { resync: 3 }));
    // 1 + 2 + 510 headers of 2 + 1 is exactly 1024 and waits; 511 headers is two over.
    assert_eq!(c.decode(&[0x02, 0xFE, 0x01]), Err(DecodeError::NeedMore));
    assert_eq!(c.decode(&[0x02, 0xFF, 0x01]), Err(DecodeError::BadFrame { resync: 3 }));
}

#[test]
fn a_segment_that_leaves_no_room_for_the_headers_still_to_come_is_refused_at_once() {
    let c = counted_small();
    // Three segments, the first 1014 bytes: 3 + (2 + 1014) + 2 + 2 + 1 is exactly 1024 and waits.
    assert_eq!(c.decode(&[0x02, 3, 0, 0xF6, 0x03]), Err(DecodeError::NeedMore));
    // The first 1015 bytes: the two headers still to come no longer fit, refused before its body.
    assert_eq!(c.decode(&[0x02, 3, 0, 0xF7, 0x03]), Err(DecodeError::BadFrame { resync: 5 }));
    // The frame at the edge is whole and decodes; one byte more is refused on encode.
    let at_edge = frame(vec![
        Segment { fields: BTreeMap::new(), body: vec![0x5A; 1014] },
        Segment::default(),
        Segment::default(),
    ]);
    let bytes = c.encode(&at_edge, None).unwrap();
    assert_eq!(bytes.len(), 1024);
    assert_eq!(c.decode(&bytes).map(|(f, used)| (f.segments.len(), used)), Ok((3, 1024)));
    let mut over = at_edge;
    over.segments[0].body.push(0x5A);
    assert_eq!(c.encode(&over, None), Err(EncodeError::FrameTooLarge { len: 1025, max: 1024 }));
}

// ---------------------------------------------------------------------------
// Refute round 5 (C-010): with both a data-area length and a segment count, the data area
// must be fillable as well as reachable: `count` segments, each at most its header and the
// largest body its length field can declare, bound it from above.
// ---------------------------------------------------------------------------

/// Delimited, a two-byte data-area length and a one-byte count; segments of a one-byte length
/// and the body, so one segment takes at most 256 bytes.
fn narrow_segments() -> Framing {
    framing_of(
        "start = \"02\"\nend = \"03\"\nmax_frame = 1024\n\
         envelope = [ { name = \"length\", size = 2, role = \"length\" }, { name = \"count\", size = 1, role = \"count\" } ]\n\
         segment = [ { name = \"length\", size = 1, role = \"length\" } ]\n",
    )
}

#[test]
fn a_data_area_its_segments_can_never_fill_is_refused_at_the_envelope() {
    let n = narrow_segments();
    let real = [0x02, 0x02, 0x00, 0x01, 0x01, 0x41, 0x03];
    assert_eq!(n.decode(&real).map(|(_, used)| used), Ok(7));
    // Two segments fill at most 512 bytes: 512 waits, 513 is refused from the envelope.
    assert_eq!(n.decode(&[0x02, 0x00, 0x02, 0x02]), Err(DecodeError::NeedMore));
    assert_eq!(n.decode(&[0x02, 0x01, 0x02, 0x02]), Err(DecodeError::BadFrame { resync: 2 }));
    // LENGTH 600 with two segments: refused at once, never holding the real frames behind it.
    let mut stream = vec![0x02, 0x58, 0x02, 0x02, 0xFF];
    for _ in 0..10 {
        stream.extend(real);
    }
    assert_eq!(stream.len(), 75);
    assert_eq!(n.decode(&stream), Err(DecodeError::BadFrame { resync: 2 }));
    // No segments and a data area: nothing can fill it.
    assert_eq!(n.decode(&[0x02, 0x01, 0x00, 0x00]), Err(DecodeError::BadFrame { resync: 4 }));
    // The 512-byte data area is reachable: two 255-byte bodies, a 517-byte frame, decoded whole.
    let full = frame(vec![
        Segment { fields: BTreeMap::new(), body: vec![0x41; 255] },
        Segment { fields: BTreeMap::new(), body: vec![0x41; 255] },
    ]);
    let bytes = n.encode(&full, None).unwrap();
    assert_eq!((bytes.len(), &bytes[..4]), (517, &[0x02, 0x00, 0x02, 0x02][..]));
    assert_eq!(n.decode(&bytes).map(|(f, used)| (f.segments.len(), used)), Ok((2, 517)));
}

#[test]
fn a_data_area_the_segments_still_to_come_cannot_fill_is_refused_at_the_header() {
    let n = narrow_segments();
    // LENGTH 300, two segments: after a first body of b bytes, 300 - 1 - b bytes are left for
    // one segment of at most 256. b = 43 leaves exactly 256 and waits; b = 42 leaves 257.
    assert_eq!(n.decode(&[0x02, 0x2C, 0x01, 0x02, 43]), Err(DecodeError::NeedMore));
    assert_eq!(n.decode(&[0x02, 0x2C, 0x01, 0x02, 42]), Err(DecodeError::BadFrame { resync: 3 }));
    // The last segment must end the data area exactly, as before.
    assert_eq!(n.decode(&[0x02, 0x05, 0x00, 0x01, 0x03]), Err(DecodeError::BadFrame { resync: 5 }));
}

#[test]
fn a_framing_built_by_hand_past_the_ceiling_waits_and_never_wraps() {
    // The channel file caps `max_frame` at 16 MiB; a `Framing` built in code may not. Its sums
    // saturate: an 8-byte length of u64::MAX waits for bytes that never come, and never panics.
    let mut hand = framing_of("max_frame = 1024\nenvelope = [ { name = \"length\", size = 8, role = \"length\" } ]\n");
    hand.max_frame = usize::MAX;
    assert_eq!(hand.decode(&[0xFF; 8]), Err(DecodeError::NeedMore));
    let mut counted = framing_of(
        "max_frame = 1024\nenvelope = [ { name = \"count\", size = 8, role = \"count\" } ]\n\
         segment = [ { name = \"length\", size = 8, role = \"length\" } ]\n",
    );
    counted.max_frame = usize::MAX;
    let mut buf = u64::MAX.to_le_bytes().to_vec();
    buf.extend(u64::MAX.to_le_bytes());
    assert!(counted.decode(&buf).is_err());
}
