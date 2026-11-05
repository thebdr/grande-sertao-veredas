//! The framing engine (contract: the framing clause), on synthetic layouts read from channel
//! files: the segmented envelope (a start byte, a 12-byte envelope with a sequence number, a
//! data-area length and a segment count, segments of id, length and body, a 4-byte tail, an end
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
