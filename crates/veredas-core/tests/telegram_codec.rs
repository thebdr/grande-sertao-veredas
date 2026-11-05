//! The wire codec (contract: the codec clause), from Veredas 1's `veredas-telegram` tests, plus
//! golden frames written byte by byte from the layout table in `src/telegram/codec.rs`.

use veredas_core::telegram::Endian;
use veredas_core::telegram::codec::*;

fn envelope() -> Envelope {
    Envelope {
        class: 0,
        source: 1,
        destination: 2,
        prg: 7,
    }
}

fn sample(segments: Vec<DataSegment>) -> Telegram {
    Telegram {
        envelope: envelope(),
        segments,
    }
}

#[test]
fn a_telegram_survives_a_round_trip_in_both_byte_orders() {
    for endian in [Endian::Big, Endian::Little] {
        let t = sample(vec![
            DataSegment {
                msg_id: 0x0102,
                data: b"POS=42".to_vec(),
            },
            DataSegment {
                msg_id: 9,
                data: vec![0xDE, 0xAD],
            },
        ]);
        let bytes = encode(&t, endian).expect("encodes");
        let (back, used) = decode(&bytes, endian).expect("decodes");
        assert_eq!(back, t, "{endian:?}");
        assert_eq!(used, bytes.len(), "{endian:?}");
        assert_eq!(bytes[0], SOM);
        assert_eq!(*bytes.last().unwrap(), EOM);
    }
}

#[test]
fn the_declared_lengths_describe_the_real_frame() {
    let t = sample(vec![DataSegment {
        msg_id: 1,
        data: b"ABCDE".to_vec(),
    }]);
    let bytes = encode(&t, Endian::Little).unwrap();
    let data_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let num_data = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    assert_eq!(num_data, 1);
    assert_eq!(data_len, SEGMENT_HEADER_LEN + 5, "header + body");
    assert_eq!(bytes.len(), ENVELOPE_LEN + data_len + TAIL_LEN);
    assert_eq!(bytes[bytes.len() - 2], 0, "CKS is unused, written zero");
}

#[test]
fn oversize_bodies_are_refused_rather_than_clamped_into_lies() {
    // The old clamp emitted a frame whose LENGTH understated the real
    // bytes — the receiver desynchronizes permanently. Refusing is
    // strictly better than sending a lie.
    let t = sample(vec![DataSegment {
        msg_id: 3,
        data: vec![0u8; u16::MAX as usize + 1],
    }]);
    assert!(matches!(
        encode(&t, Endian::Little),
        Err(EncodeError::SegmentTooLarge { msg_id: 3, .. })
    ));

    let many = sample(
        (0..3)
            .map(|i| DataSegment {
                msg_id: i,
                data: vec![0u8; 30_000],
            })
            .collect(),
    );
    assert!(matches!(
        encode(&many, Endian::Little),
        Err(EncodeError::FrameTooLarge { .. })
    ));
}

#[test]
fn a_keepalive_is_an_envelope_and_a_tail() {
    let ka = Telegram::keepalive(envelope());
    assert!(ka.is_keepalive());
    let bytes = encode(&ka, Endian::Little).unwrap();
    assert_eq!(bytes.len(), ENVELOPE_LEN + TAIL_LEN);
    let (back, used) = decode(&bytes, Endian::Little).unwrap();
    assert_eq!(back, ka);
    assert_eq!(used, bytes.len());
    assert!(back.is_keepalive());
}

#[test]
fn a_frame_split_across_packets_asks_for_more_rather_than_failing() {
    let t = sample(vec![DataSegment {
        msg_id: 3,
        data: b"hello".to_vec(),
    }]);
    let bytes = encode(&t, Endian::Little).unwrap();
    for cut in 1..bytes.len() {
        assert_eq!(
            decode(&bytes[..cut], Endian::Little),
            Err(DecodeError::NeedMore),
            "a {cut}-byte prefix should wait for the rest"
        );
    }
    assert!(decode(&bytes, Endian::Little).is_ok());
    assert_eq!(decode(&[], Endian::Little), Err(DecodeError::NeedMore));
}

#[test]
fn a_stream_that_lost_sync_can_be_recovered() {
    let t = sample(vec![DataSegment {
        msg_id: 1,
        data: b"X".to_vec(),
    }]);
    let good = encode(&t, Endian::Little).unwrap();
    let mut buf = vec![0x11, 0x22, 0x33];
    buf.extend_from_slice(&good);
    let Err(DecodeError::BadFrame { resync }) = decode(&buf, Endian::Little) else {
        panic!("garbage should not decode");
    };
    assert_eq!(resync, 3, "skip to the next SOM");
    let (back, _) = decode(&buf[resync..], Endian::Little).expect("recovered");
    assert_eq!(back, t);

    let mut broken = good.clone();
    let last = broken.len() - 1;
    broken[last] = 0x00;
    assert!(matches!(
        decode(&broken, Endian::Little),
        Err(DecodeError::BadFrame { .. })
    ));
}

#[test]
fn inconsistent_frames_are_refused_not_partially_believed() {
    // A segment claiming more than the data area holds.
    let t = sample(vec![DataSegment {
        msg_id: 1,
        data: b"1234".to_vec(),
    }]);
    let mut bytes = encode(&t, Endian::Little).unwrap();
    bytes[14] = 0xFF;
    bytes[15] = 0xFF;
    assert!(matches!(
        decode(&bytes, Endian::Little),
        Err(DecodeError::BadFrame { .. })
    ));

    // A data area with zero segments to describe it: those bytes
    // would vanish unexamined (and `is_keepalive` would lie).
    let ka = encode(&Telegram::keepalive(envelope()), Endian::Little).unwrap();
    let mut phantom = ka.clone();
    phantom[8] = 4; // LENGTH=4, NUM DATA=0
    phantom.splice(ENVELOPE_LEN..ENVELOPE_LEN, [0u8, 0, 0, 0]);
    assert!(matches!(
        decode(&phantom, Endian::Little),
        Err(DecodeError::BadFrame { .. })
    ));

    // Slack between the last segment and the declared end.
    let t = sample(vec![DataSegment {
        msg_id: 1,
        data: b"AB".to_vec(),
    }]);
    let mut slack = encode(&t, Endian::Little).unwrap();
    // Declare 2 extra bytes of data area and physically insert them
    // after the segment, keeping NUM DATA at 1.
    slack[8] = slack[8].wrapping_add(2);
    let insert_at = ENVELOPE_LEN + SEGMENT_HEADER_LEN + 2;
    slack.splice(insert_at..insert_at, [0xAA, 0xBB]);
    assert!(matches!(
        decode(&slack, Endian::Little),
        Err(DecodeError::BadFrame { .. })
    ));
}

#[test]
fn a_wrong_endian_setting_reads_as_bad_frames_not_phantom_data() {
    let t = sample(vec![DataSegment {
        msg_id: 1,
        data: b"payload".to_vec(),
    }]);
    let bytes = encode(&t, Endian::Little).unwrap();
    // Read big-endian, LENGTH says 2816 and NUM DATA 256: the one frame is a plausible prefix, so
    // it waits; repeated, the second segment header shows the lengths cannot add up, long before
    // the 2 832 declared bytes arrive. Never an invented segment.
    assert_eq!(decode(&bytes, Endian::Big), Err(DecodeError::NeedMore));
    assert_eq!(decode(&bytes.repeat(68), Endian::Big), Err(DecodeError::BadFrame { resync: 27 }));
    assert_eq!(decode(&bytes.repeat(149), Endian::Big), Err(DecodeError::BadFrame { resync: 27 }));
}

#[test]
fn hex_bodies_parse_and_previews_render_both_ways() {
    assert_eq!(parse_hex("00 1A FF").unwrap(), vec![0x00, 0x1A, 0xFF]);
    assert_eq!(parse_hex("001aff").unwrap(), vec![0x00, 0x1A, 0xFF]);
    assert_eq!(parse_hex("  ").unwrap(), Vec::<u8>::new());
    assert_eq!(parse_hex("ABC"), Err(HexError::OddDigits));
    assert_eq!(parse_hex("ZZ"), Err(HexError::NotHex("ZZ".into())));

    assert_eq!(preview(b"POS=42"), "POS=42");
    assert_eq!(preview(&[0x00, 0xFF]), "00 FF");
    assert!(preview(&[b'x'; 100]).ends_with('…'));

    let dump = hex_dump(b"ABC\x00DEF");
    assert!(dump.starts_with("0000  41 42 43 00 44 45 46"), "{dump}");
    assert!(dump.trim_end().ends_with("ABC.DEF"), "{dump}");
}

/// The frame of `sample([seg(0x0102, "AB")])` in one byte order, field by field.
fn golden(little: bool) -> Vec<u8> {
    let u16b = |v: u16| {
        if little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    };
    let mut f = vec![0xFD, 0, 1, 2, 7, 0, 0, 0]; // SOM CLASS SOURCE DEST PRG SPARE*3
    f.extend(u16b(6)); // LENGTH: segment header (4) + body (2)
    f.extend(u16b(1)); // NUM DATA
    f.extend(u16b(0x0102)); // MSG ID
    f.extend(u16b(2)); // MSG LENGTH
    f.extend(b"AB"); // MSG DATA
    f.extend([0, 0, 0, 0xFE]); // SPARE*2 CKS EOM
    f
}

#[test]
fn the_frame_is_the_spec_layout_byte_for_byte() {
    let t = sample(vec![DataSegment {
        msg_id: 0x0102,
        data: b"AB".to_vec(),
    }]);
    assert_eq!(
        encode(&t, Endian::Little).unwrap(),
        [
            0xFD, 0, 1, 2, 7, 0, 0, 0, 6, 0, 1, 0, 0x02, 0x01, 2, 0, b'A', b'B', 0, 0, 0, 0xFE
        ]
    );
    assert_eq!(encode(&t, Endian::Little).unwrap(), golden(true));
    assert_eq!(encode(&t, Endian::Big).unwrap(), golden(false));
    // And the golden bytes decode to the telegram, consuming exactly the frame.
    for little in [true, false] {
        let endian = if little { Endian::Little } else { Endian::Big };
        let mut stream = golden(little);
        stream.extend(golden(little)); // a second frame behind it is left alone
        assert_eq!(decode(&stream, endian), Ok((t.clone(), 22)));
    }
    // Little-endian is the default.
    assert_eq!(Endian::default(), Endian::Little);
    assert_eq!(
        (Endian::Little.label(), Endian::Big.label()),
        ("little-endian", "big-endian")
    );
}

#[test]
fn a_keepalive_is_sixteen_bytes_and_the_envelope_fields_round_trip() {
    let ka = Telegram::keepalive(Envelope {
        class: 0xAA,
        source: 0x10,
        destination: 0x20,
        prg: 0,
    });
    let bytes = encode(&ka, Endian::Little).unwrap();
    assert_eq!(
        bytes,
        [
            0xFD, 0xAA, 0x10, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFE
        ]
    );
    assert_eq!(
        decode(&bytes, Endian::Big),
        Ok((ka, 16)),
        "zero lengths read the same both ways"
    );
}

#[test]
fn the_size_limits_sit_exactly_at_the_wire_fields() {
    let seg = |n: usize| DataSegment {
        msg_id: 9,
        data: vec![0x5A; n],
    };
    // u16::MAX body: the segment fits MSG LENGTH, but the data area (65 539) does not fit LENGTH.
    assert_eq!(
        encode(&sample(vec![seg(u16::MAX as usize)]), Endian::Little),
        Err(EncodeError::FrameTooLarge {
            len: ENVELOPE_LEN + 4 + 65_535 + TAIL_LEN
        })
    );
    assert_eq!(
        encode(&sample(vec![seg(65_536)]), Endian::Little),
        Err(EncodeError::SegmentTooLarge {
            msg_id: 9,
            len: 65_536
        })
    );
    // The largest frame MAX_FRAME admits encodes and decodes.
    let body = MAX_FRAME - ENVELOPE_LEN - TAIL_LEN - SEGMENT_HEADER_LEN;
    let big = sample(vec![seg(body)]);
    let bytes = encode(&big, Endian::Little).unwrap();
    assert_eq!(bytes.len(), MAX_FRAME);
    assert_eq!(decode(&bytes, Endian::Little), Ok((big, MAX_FRAME)));
    // One byte more is refused, with the size it would have had.
    assert_eq!(
        encode(&sample(vec![seg(body + 1)]), Endian::Little),
        Err(EncodeError::FrameTooLarge { len: MAX_FRAME + 1 })
    );
    assert_eq!(
        EncodeError::SegmentTooLarge {
            msg_id: 9,
            len: 65_536
        }
        .to_string(),
        "segment (msg id 9) is 65536 bytes — larger than the wire format can describe"
    );
    assert_eq!(
        EncodeError::FrameTooLarge { len: 70_000 }.to_string(),
        "frame would be 70000 bytes — larger than the wire format allows"
    );
}

#[test]
fn a_bad_frame_says_exactly_how_far_to_skip() {
    let good = golden(true);
    // No SOM at the start: skip to the next SOM, or past everything when there is none.
    let mut buf = vec![0x01, 0x02];
    buf.extend(&good);
    assert_eq!(
        decode(&buf, Endian::Little),
        Err(DecodeError::BadFrame { resync: 2 })
    );
    assert_eq!(
        decode(&[0x00, 0x01, 0x02], Endian::Little),
        Err(DecodeError::BadFrame { resync: 3 })
    );
    // A broken EOM: skip past this SOM to the next one (here, the start of the second frame).
    let mut broken = good.clone();
    let last = broken.len() - 1;
    broken[last] = 0x00;
    broken.extend(&good);
    assert_eq!(
        decode(&broken, Endian::Little),
        Err(DecodeError::BadFrame { resync: 22 })
    );
    // A LENGTH past MAX_FRAME is refused at once rather than waited for.
    let mut huge = vec![0xFD, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 1, 0];
    assert_eq!(
        decode(&huge, Endian::Little),
        Err(DecodeError::BadFrame { resync: 12 })
    );
    huge.push(0xFD);
    assert_eq!(
        decode(&huge, Endian::Little),
        Err(DecodeError::BadFrame { resync: 12 })
    );
    // A frame that is only a SOM so far waits.
    assert_eq!(decode(&[0xFD], Endian::Little), Err(DecodeError::NeedMore));
    assert_eq!(
        DecodeError::BadFrame { resync: 3 }.to_string(),
        "not a telegram frame: skip 3 byte(s) to the next start"
    );
}

#[test]
fn hex_is_two_ascii_digits_per_byte_and_nothing_else() {
    assert_eq!(parse_hex("").unwrap(), Vec::<u8>::new());
    assert_eq!(
        parse_hex("fd 00\t7f\nFE").unwrap(),
        vec![0xFD, 0x00, 0x7F, 0xFE]
    );
    // A sign is not a digit, though `from_str_radix` would take it (Veredas 1 read "+F" as 0x0F).
    assert_eq!(parse_hex("+F"), Err(HexError::NotHex("+F".into())));
    assert_eq!(parse_hex("-1"), Err(HexError::NotHex("-1".into())));
    // Digits are characters: a two-byte character is one (odd) digit, never half a byte.
    assert_eq!(parse_hex("é"), Err(HexError::OddDigits));
    assert_eq!(parse_hex("éé"), Err(HexError::NotHex("éé".into())));
    assert_eq!(parse_hex("0g"), Err(HexError::NotHex("0g".into())));
    assert_eq!(
        HexError::NotHex("0g".into()).to_string(),
        "`0g` is not a hex byte"
    );
    assert_eq!(
        HexError::OddDigits.to_string(),
        "hex needs an even number of digits"
    );
}

#[test]
fn previews_and_dumps_render_exactly() {
    assert_eq!(preview(b""), "");
    assert_eq!(preview(b"a\tb"), "a\tb");
    assert_eq!(preview(&[b'x'; 48]), "x".repeat(48));
    assert_eq!(preview(&[b'x'; 49]), format!("{}…", "x".repeat(48)));
    assert_eq!(
        preview(&(0u8..16).collect::<Vec<_>>()),
        "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F"
    );
    assert_eq!(
        preview(&(0u8..17).collect::<Vec<_>>()),
        "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F …"
    );
    assert_eq!(hex_dump(b""), "");
    let dump = hex_dump(b"0123456789abcdef\x7f");
    assert_eq!(
        dump,
        format!(
            "0000  30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66  0123456789abcdef\n0010  7F{}  .\n",
            " ".repeat(45)
        )
    );
}

#[test]
fn lengths_that_disagree_are_refused_as_soon_as_the_disagreement_is_visible() {
    let mut two_good = golden(true);
    two_good.extend(golden(true));

    // LENGTH 60 000 and NUM DATA 1, the segment declaring 2 bytes: one segment cannot fill the
    // area. Refused now, not after 60 KB more, so the real frames behind it are not held.
    let mut buf = vec![0xFD, 0, 0, 0, 0, 0, 0, 0, 0x60, 0xEA, 1, 0, 1, 0, 2, 0];
    buf.extend(&two_good);
    assert_eq!(decode(&buf, Endian::Little), Err(DecodeError::BadFrame { resync: 16 }));
    assert_eq!(decode(&buf[16..], Endian::Little), Ok((sample(vec![DataSegment { msg_id: 0x0102, data: b"AB".to_vec() }]), 22)));

    // A controller restarting mid-frame: a frame whose body holds a SOM, its EOM broken. The
    // resync lands on that SOM, whose bytes read as a 61 440-byte frame with a zero-length
    // segment: refused at once, and the next skip reaches the real frames.
    let t = Telegram {
        envelope: Envelope::default(),
        segments: vec![DataSegment { msg_id: 1, data: vec![0xFD, 0, 0, 0, 0, 0, 0, 0, 0, 0xF0, 1, 0] }],
    };
    let mut stream = encode(&t, Endian::Little).unwrap();
    let last = stream.len() - 1;
    stream[last] = 0;
    stream.extend(&two_good);
    assert_eq!(stream.len(), 76);
    assert_eq!(decode(&stream, Endian::Little), Err(DecodeError::BadFrame { resync: 16 }));
    assert_eq!(decode(&stream[16..], Endian::Little), Err(DecodeError::BadFrame { resync: 16 }));
    assert!(decode(&stream[32..], Endian::Little).is_ok());

    // Three segments cannot fit their headers in an 8-byte data area.
    let short = [0xFD, 0, 0, 0, 0, 0, 0, 0, 8, 0, 3, 0];
    assert_eq!(decode(&short, Endian::Little), Err(DecodeError::BadFrame { resync: 12 }));

    // A valid two-segment frame still waits on every prefix and decodes whole.
    let t = sample(vec![
        DataSegment { msg_id: 1, data: b"ONE".to_vec() },
        DataSegment { msg_id: 2, data: vec![] },
    ]);
    let bytes = encode(&t, Endian::Little).unwrap();
    for cut in 1..bytes.len() {
        assert_eq!(decode(&bytes[..cut], Endian::Little), Err(DecodeError::NeedMore), "cut {cut}");
    }
    assert_eq!(decode(&bytes, Endian::Little), Ok((t, bytes.len())));
}

#[test]
fn the_remaining_headers_check_holds_on_both_sides_of_its_boundary() {
    // LENGTH 8, NUM DATA 2, the first segment 1 byte long: 3 bytes left for a 4-byte header.
    let short = [0xFD, 0, 0, 0, 0, 0, 0, 0, 8, 0, 2, 0, 1, 0, 1, 0];
    assert_eq!(decode(&short, Endian::Little), Err(DecodeError::BadFrame { resync: 16 }));
    // The first segment empty: exactly 4 bytes left, a valid frame could still follow.
    let mut exact = short;
    exact[14] = 0;
    assert_eq!(decode(&exact, Endian::Little), Err(DecodeError::NeedMore));
}
