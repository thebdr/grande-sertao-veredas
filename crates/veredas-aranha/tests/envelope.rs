//! The envelope every PROFINET frame travels in, and the identity of everything a scan sees:
//! Ethernet II with at most one 802.1Q tag, and the MAC address (contract: the envelope clause).
//! Exact bytes both ways, the limits refused rather than cut, and every frame read back by
//! Wireshark's dissectors as a second reading.

mod common;

use veredas_aranha::profinet::ether::{self, EtherError, EtherFrame, VlanTag};
use veredas_aranha::profinet::mac::{Mac, MacParseError};

fn mac(s: &str) -> Mac {
    s.parse().unwrap()
}

#[test]
fn a_mac_reads_its_three_spellings_in_either_case_and_prints_one() {
    let m = Mac([0x02, 0x0e, 0xcf, 0x12, 0x34, 0x56]);
    for s in ["02:0e:cf:12:34:56", "02:0E:CF:12:34:56", "02-0e-cf-12-34-56", "020e.cf12.3456", "020E.CF12.3456", "02:0E:cf:12:34:56", "020E.cf12.3456"] {
        assert_eq!(s.parse::<Mac>(), Ok(m), "{s}");
    }
    assert_eq!(m.to_string(), "02:0e:cf:12:34:56");
    // Every hex letter, in upper case and in lower.
    assert_eq!("AB:CD:EF:AB:CD:EF".parse::<Mac>(), Ok(Mac([0xAB, 0xCD, 0xEF, 0xAB, 0xCD, 0xEF])));
    assert_eq!("abcd.efAB.CDEF".parse::<Mac>(), Ok(Mac([0xAB, 0xCD, 0xEF, 0xAB, 0xCD, 0xEF])));
    assert_eq!(format!("{m:?}"), "Mac(02:0e:cf:12:34:56)");
    assert_eq!(m.oui(), [0x02, 0x0e, 0xcf]);
    assert!(!m.is_multicast());
    assert!(m.is_local());
    assert!(!mac("00:0e:cf:12:34:56").is_local());
    // The locally administered bit is bit 1 of the first octet, whatever the others.
    for (s, local) in [("06:00:00:00:00:01", true), ("0a:00:00:00:00:01", true), ("03:00:00:00:00:01", true), ("01:00:00:00:00:01", false), ("04:00:00:00:00:01", false)] {
        assert_eq!(mac(s).is_local(), local, "{s}");
    }
    assert!(Mac::BROADCAST.is_local() && !Mac::DCP_MULTICAST.is_local());
    for (first, local) in [(0x80u8, false), (0xF0, false), (0x40, false), (0x10, false), (0xFE, true), (0xFD, false)] {
        assert_eq!(Mac([first, 0, 0, 0, 0, 0]).is_local(), local, "{first:#04x}");
    }
}

/// Texts that are none of the three spellings.
const REFUSED: [&str; 25] = [
        "",
        "02:0e:cf:12:34",
        "02:0e:cf:12:34:56:78",
        "020ecf123456",
        "02:0e-cf:12:34:56",
        "02:0e:cf:12:34:5",
        "02:0e:cf:12:34:567",
        "2:0e:cf:12:34:56",
        "+2:0e:cf:12:34:56",
        "02:0e:cf:12:34:5g",
        " 02:0e:cf:12:34:56",
        "02:0e:cf:12:34:56 ",
        "02:0e:cf:12:34:56\n",
        "\t02:0e:cf:12:34:56",
        "02:0e:cf:12:34:56:",
        "aa.bb.cc.dd.ee.ff",
        "aabb:ccdd:eeff",
        "aabb-ccdd-eeff",
        "aabb.ccdd.eef",
        "aabb.ccdd.eeff.0000",
        "éabb.ccdd.eeff",
        "02:0e:cf:12:34:٥٦",
        // Non-ASCII groups exactly as wide, in bytes, as a group: refused by the digit check.
        "aéb.ccdd.eeff",
        "€a.ccdd.eeff",
        "02:0e:cf:12:34:é",
];

#[test]
fn a_mac_refuses_every_other_spelling_with_the_text_it_got() {
    for s in REFUSED {
        assert_eq!(s.parse::<Mac>(), Err(MacParseError(s.to_string())), "{s:?}");
    }
    assert_eq!(
        "x".parse::<Mac>().unwrap_err().to_string(),
        "not a MAC address: \"x\" (six pairs of hex digits joined by ':' or '-', or three groups of four joined by '.')"
    );
}

#[test]
fn group_addresses_are_never_one_device() {
    assert_eq!(Mac::DCP_MULTICAST.to_string(), "01:0e:cf:00:00:00");
    assert_eq!(Mac::DCP_HELLO_MULTICAST.to_string(), "01:0e:cf:00:00:01");
    assert_eq!(Mac::LLDP_MULTICAST.to_string(), "01:80:c2:00:00:0e");
    assert_eq!(Mac::BROADCAST.to_string(), "ff:ff:ff:ff:ff:ff");
    for m in [Mac::DCP_MULTICAST, Mac::DCP_HELLO_MULTICAST, Mac::LLDP_MULTICAST, Mac::BROADCAST, mac("03:00:00:00:00:00")] {
        assert!(m.is_multicast(), "{m}");
    }
    assert!(!mac("fe:ff:ff:ff:ff:ff").is_multicast());
    assert_eq!(Mac::from_slice(&[1, 2, 3, 4, 5]), None);
    assert_eq!(Mac::from_slice(&[1, 2, 3, 4, 5, 6, 7]), Some(Mac([1, 2, 3, 4, 5, 6])));
}

#[test]
fn a_mac_serialises_as_its_printed_form() {
    let m = mac("02:0e:cf:12:34:56");
    assert_eq!(serde_json::to_string(&m).unwrap(), "\"02:0e:cf:12:34:56\"");
    assert_eq!(serde_json::from_str::<Mac>("\"02-0E-CF-12-34-56\"").unwrap(), m);
    // Read from an owned value, from a reader, and through an escape: the text, however it came.
    assert_eq!(serde_json::from_value::<Mac>(serde_json::json!("02:0e:cf:12:34:56")).unwrap(), m);
    assert_eq!(serde_json::from_reader::<_, Mac>(&b"\"02:0e:cf:12:34:56\""[..]).unwrap(), m);
    assert_eq!(serde_json::from_str::<Mac>("\"02\\u003a0e:cf:12:34:56\"").unwrap(), m);
    let err = serde_json::from_str::<Mac>("\"02:0e\"").unwrap_err().to_string();
    assert!(err.starts_with("not a MAC address: \"02:0e\""), "{err}");
    // Read through serde, every refused text is refused as it was given, untrimmed.
    for text in REFUSED {
        let err = serde_json::from_str::<Mac>(&serde_json::to_string(text).unwrap()).unwrap_err().to_string();
        assert!(err.starts_with(&MacParseError(text.to_string()).to_string()), "{text:?}: {err}");
    }
}

#[test]
fn an_untagged_frame_is_built_padded_and_read_back_exactly() {
    let (dst, src) = (mac("02:00:00:00:00:01"), mac("02:00:00:00:00:02"));
    let f = ether::build(dst, src, 0x8892, &[0xFE, 0xFE, 5]).unwrap();
    let mut want = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2, 0x88, 0x92, 0xFE, 0xFE, 5];
    want.resize(60, 0);
    assert_eq!(f, want);
    let e = ether::parse(&f).unwrap();
    assert_eq!(e, EtherFrame { dst, src, vlan: None, ethertype: 0x8892, payload: &want[14..] });
    // A frame already 60 bytes or longer is not padded further.
    let long = ether::build(dst, src, 0x88CC, &[7; 100]).unwrap();
    assert_eq!(long.len(), 114);
    assert_eq!(ether::parse(&long).unwrap().payload, &[7; 100][..]);
}

#[test]
fn a_tagged_frame_carries_its_priority_drop_bit_and_vlan() {
    let (dst, src) = (mac("02:00:00:00:00:01"), mac("02:00:00:00:00:02"));
    let tag = VlanTag { priority: 6, drop_eligible: false, id: 0 };
    let f = ether::build_tagged(dst, src, Some(tag), 0x8892, &[0x80, 0x00]).unwrap();
    let mut want = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2, 0x81, 0x00, 0xC0, 0x00, 0x88, 0x92, 0x80, 0x00];
    want.resize(60, 0);
    assert_eq!(f, want, "the whole frame, the pad's zeros included");
    assert_eq!(ether::parse(&f).unwrap(), EtherFrame { dst, src, vlan: Some(tag), ethertype: 0x8892, payload: &want[18..] });
    let e = ether::parse(&f).unwrap();
    assert_eq!((e.vlan, e.ethertype, &e.payload[..2]), (Some(tag), 0x8892, &[0x80, 0x00][..]));

    // Every bit set but the reserved VLAN id's last: 4094 written; 4095 read, never written.
    let every = VlanTag { priority: 7, drop_eligible: true, id: 4094 };
    let f = ether::build_tagged(dst, src, Some(every), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0xFF, 0xFE]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(every));
    let reserved = VlanTag { id: 4095, ..every };
    assert_eq!(ether::build_tagged(dst, src, Some(reserved), 0x0800, &[]), Err(EtherError::BadVlanId(4095)));
    let mut heard = f.clone();
    heard[15] = 0xFF;
    assert_eq!(ether::parse(&heard).unwrap().vlan, Some(reserved));
    // The drop bit alone, and the priority's low bit alone, read back apart.
    let drop_only = VlanTag { priority: 0, drop_eligible: true, id: 5 };
    let f = ether::build_tagged(dst, src, Some(drop_only), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0x10, 0x05]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(drop_only));
    // A tag of nothing (priority 0, VLAN 0) is a tag all the same, written and read.
    let nothing = VlanTag { priority: 0, drop_eligible: false, id: 0 };
    let f = ether::build_tagged(dst, src, Some(nothing), 0x8892, &[0x80, 0x00]).unwrap();
    assert_eq!(&f[12..18], &[0x81, 0x00, 0x00, 0x00, 0x88, 0x92]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(nothing));
    let low_priority = VlanTag { priority: 1, drop_eligible: false, id: 5 };
    let f = ether::build_tagged(dst, src, Some(low_priority), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0x20, 0x05]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(low_priority));
}

#[test]
fn what_the_wire_cannot_carry_is_refused_never_cut() {
    let (dst, src) = (mac("02:00:00:00:00:01"), mac("02:00:00:00:00:02"));
    let f = ether::build(dst, src, 0x88CC, &[0; 1500]).unwrap();
    assert_eq!(f.len(), 1514);
    assert_eq!(ether::parse(&f).unwrap().payload, &[0; 1500][..]);
    assert_eq!(ether::build(dst, src, 0x88CC, &[0; 1501]), Err(EtherError::TooLong(1501)));
    let tag = |priority, id| Some(VlanTag { priority, drop_eligible: false, id });
    // Tagged, the payload's limit is the same: the frame grows by the tag.
    let f = ether::build_tagged(dst, src, tag(6, 0), 0x8892, &[0x5A; 1500]).unwrap();
    assert_eq!(f.len(), 1518);
    assert_eq!(ether::parse(&f).unwrap().payload, &[0x5A; 1500][..]);
    // What is heard past 1500 is read whole, never refused: two tags, the inner one read as the
    // payload's first bytes (this envelope reads one tag), 1522 bytes in all.
    let mut double = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2, 0x81, 0x00, 0xC0, 0x05, 0x81, 0x00, 0x00, 0x07, 0x88, 0x92];
    double.resize(1522, 0x33);
    let e = ether::parse(&double).unwrap();
    assert_eq!((e.vlan, e.ethertype, e.payload.len()), (tag(6, 5), 0x8100, 1504));
    assert_eq!(e.payload, &double[18..]);
    let big = vec![0x44; 9000];
    let e = ether::parse(&big).unwrap();
    assert_eq!((e.payload.len(), e.payload), (8986, &big[14..]));
    assert_eq!(ether::build_tagged(dst, src, tag(6, 0), 0x8892, &[0; 1501]), Err(EtherError::TooLong(1501)));
    // A type that is a length, nothing (0x05DD to 0x05FF), or one readers take for a tag, is no
    // EtherType to write.
    for t in [0x0000u16, 0x0026, 0x05DC, 0x05DD, 0x05FF, 0x8100, 0x88A8, 0x9100] {
        assert_eq!(ether::build(dst, src, t, &[1, 2, 3]), Err(EtherError::NotEtherType(t)), "{t:#x}");
        assert_eq!(ether::build_tagged(dst, src, tag(0, 1), t, &[1, 2, 3]), Err(EtherError::NotEtherType(t)), "{t:#x}");
    }
    assert!(ether::build(dst, src, 0x0600, &[]).is_ok());
    assert_eq!(
        EtherError::NotEtherType(0x26).to_string(),
        "0x0026 is not an EtherType this envelope writes (below 0x0600 is an 802.3 length or nothing; 0x8100, 0x88A8 and 0x9100 are read as tags)"
    );
    assert_eq!(ether::TAG_TYPES, [0x8100, 0x88A8, 0x9100]);
    assert!(ether::build(dst, src, 0x88A9, &[]).is_ok() && ether::build(dst, src, 0x9101, &[]).is_ok());
    assert_eq!(ether::build_tagged(dst, src, tag(8, 0), 0x8892, &[]), Err(EtherError::BadPriority(8)));
    assert_eq!(EtherError::BadPriority(8).to_string(), "a VLAN priority is 0 to 7, not 8");
    // A source is one station: a group address there is refused, tagged or not (Wireshark warns of
    // it, after IEEE 802.3).
    for group in [Mac::BROADCAST, Mac::DCP_MULTICAST, Mac([0x03, 0, 0, 0, 0, 2])] {
        assert_eq!(ether::build(dst, group, 0x8892, &[1]), Err(EtherError::GroupSource(group)), "{group}");
        assert_eq!(ether::build_tagged(dst, group, tag(6, 0), 0x8892, &[1]), Err(EtherError::GroupSource(group)), "{group}");
    }
    assert_eq!(
        EtherError::GroupSource(Mac::BROADCAST).to_string(),
        "a frame's source is one station; ff:ff:ff:ff:ff:ff is a group address (IEEE 802.3, 3.2.3)"
    );
    // Heard, such a frame is read as it is, for the scanner to see.
    let mut heard = ether::build(dst, src, 0x8892, &[1]).unwrap();
    heard[6] = 0xFF;
    assert_eq!(ether::parse(&heard).unwrap().src, Mac([0xFF, 0, 0, 0, 0, 2]));
    assert_eq!(ether::build_tagged(dst, src, tag(0, 4096), 0x8892, &[]), Err(EtherError::BadVlanId(4096)));
    assert_eq!(EtherError::BadVlanId(4095).to_string(), "a VLAN id written is 0 to 4094 (4095 is reserved), not 4095");
    assert_eq!(EtherError::TooLong(1501).to_string(), "a payload of 1501 bytes is past Ethernet's 1500");
}

#[test]
fn a_length_field_is_read_as_a_length_never_as_an_ethertype() {
    assert_eq!((ether::MAX_LENGTH, ether::MIN_ETHERTYPE), (0x05DC, 0x0600));
    // A spanning-tree frame: 802.3, its length 0x0026, an LLC header (42 42 03) following.
    let mut stp = vec![0x01, 0x80, 0xC2, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0x00, 0x26, 0x42, 0x42, 0x03];
    stp.resize(60, 0);
    let e = ether::parse(&stp).unwrap();
    assert!(e.is_length());
    assert_eq!((e.ethertype, e.payload), (0x0026, &stp[14..]));
    assert_eq!(e.payload.len(), 46, "the payload runs to the end, its pad included");
    let lldp = ether::build(Mac::LLDP_MULTICAST, mac("02:00:00:00:00:02"), 0x88CC, &[0, 0]).unwrap();
    assert!(!ether::parse(&lldp).unwrap().is_length());
    // At the boundaries, tagged and not: up to 0x05DC a length, from 0x0600 an EtherType, between
    // them neither; the payload whole, to the end.
    for (t, length) in [(0x05FFu16, false), (0x05DD, false), (0x05DC, true), (0x0000, true), (0x0600, false), (0x0800, false), (0x86DD, false)] {
        let ethertype = t >= 0x0600;
        let mut untagged = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2];
        untagged.extend_from_slice(&t.to_be_bytes());
        untagged.resize(60, 0);
        let mut tagged = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2, 0x81, 0x00, 0x00, 0x05];
        tagged.extend_from_slice(&t.to_be_bytes());
        tagged.resize(60, 0);
        for f in [&untagged, &tagged] {
            let e = ether::parse(f).unwrap();
            assert_eq!((e.ethertype, e.is_length(), e.is_ethertype()), (t, length, ethertype), "{t:#06x} {:?}", e.vlan);
            assert_eq!(e.payload, &f[if e.vlan.is_some() { 18 } else { 14 }..], "{t:#06x}");
        }
    }
}

#[test]
fn a_short_frame_is_an_error_that_says_its_length() {
    assert_eq!(ether::parse(&[0; 13]), Err(EtherError::TooShort(13)));
    assert!(ether::parse(&[0; 14]).is_ok());
    let mut tagged = vec![0; 12];
    tagged.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
    assert_eq!(ether::parse(&tagged), Err(EtherError::TooShort(16)));
    tagged.push(0x08);
    assert_eq!(ether::parse(&tagged), Err(EtherError::TooShort(17)));
    tagged.push(0x00);
    let e = ether::parse(&tagged).unwrap();
    assert_eq!((e.vlan.map(|v| v.id), e.ethertype, e.payload.len()), (Some(5), 0x0800, 0));
    assert_eq!(EtherError::TooShort(13).to_string(), "an Ethernet frame is at least 14 bytes (18 with a VLAN tag); this one is 13");
}

#[test]
fn the_dissectors_read_the_envelope_as_it_was_written() {
    let (dst, src) = (mac("02:00:00:00:00:01"), mac("02:0a:0b:0c:0d:0e"));
    let frames = vec![
        ether::build(dst, src, 0x88B5, &[1, 2, 3]).unwrap(),
        ether::build_tagged(Mac::BROADCAST, src, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x88B5, &[9]).unwrap(),
        ether::build_tagged(dst, src, Some(VlanTag { priority: 3, drop_eligible: true, id: 1234 }), 0x88B5, &[0; 1496]).unwrap(),
        ether::build_tagged(dst, src, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x88B5, &[0; 1500]).unwrap(),
        ether::build(Mac::LLDP_MULTICAST, src, 0x88CC, &[0x02, 0x02, 0x07, 0x61, 0x04, 0x02, 0x07, 0x70, 0x06, 0x02, 0x00, 0x14, 0, 0]).unwrap(),
        // PROFINET's EtherType, tagged as a cyclic frame rides and untagged as DCP does.
        ether::build_tagged(dst, src, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x8892, &[0x80, 0x00]).unwrap(),
        ether::build(Mac::DCP_MULTICAST, src, 0x8892, &[0xFE, 0xFE, 5, 0, 0, 0, 0, 1, 0, 1, 0, 4, 0xFF, 0xFF, 0, 0]).unwrap(),
        // LLDP's, tagged.
        ether::build_tagged(Mac::LLDP_MULTICAST, src, Some(VlanTag { priority: 7, drop_eligible: false, id: 9 }), 0x88CC, &[0x02, 0x02, 0x07, 0x61, 0x04, 0x02, 0x07, 0x70, 0x06, 0x02, 0x00, 0x14, 0, 0]).unwrap(),
    ];
    let read = common::dissect(&frames);
    // Every field on every frame where it applies, against what the frame was built with.
    for (i, (p, f)) in read.iter().zip(&frames).enumerate() {
        let e = ether::parse(f).unwrap();
        assert_eq!(p.complaints(), Vec::<String>::new(), "frame {i}");
        assert_eq!(p.one("eth", "eth.src"), e.src.to_string(), "frame {i}");
        assert_eq!(p.one("eth", "eth.dst"), e.dst.to_string(), "frame {i}");
        match e.vlan {
            None => {
                assert!(!p.has("vlan"), "frame {i}");
                assert_eq!(p.one("eth", "eth.type"), e.ethertype.to_string(), "frame {i}");
            }
            Some(tag) => {
                assert_eq!(p.one("eth", "eth.type"), "33024", "frame {i}");
                assert_eq!(p.one("vlan", "vlan.priority"), tag.priority.to_string(), "frame {i}");
                assert_eq!(p.one("vlan", "vlan.dei"), tag.drop_eligible.to_string(), "frame {i}");
                assert_eq!(p.one("vlan", "vlan.id"), tag.id.to_string(), "frame {i}");
                assert_eq!(p.one("vlan", "vlan.etype"), e.ethertype.to_string(), "frame {i}");
            }
        }
    }
    assert_eq!(read[0].one("eth", "eth.dst"), "02:00:00:00:00:01");
    assert_eq!(read[0].one("eth", "eth.type"), "34997");
    assert!(!read[0].has("vlan"));
    assert_eq!(read[1].one("eth", "eth.dst"), "ff:ff:ff:ff:ff:ff");
    assert_eq!(read[1].one("vlan", "vlan.priority"), "6");
    assert_eq!(read[1].one("vlan", "vlan.id"), "0");
    assert_eq!(read[1].one("vlan", "vlan.etype"), "34997");
    assert_eq!(read[2].one("vlan", "vlan.priority"), "3");
    assert_eq!(read[2].one("vlan", "vlan.dei"), "true");
    assert_eq!(read[2].one("vlan", "vlan.id"), "1234");
    assert_eq!(read[2].one("frame", "frame.len"), "1514");
    assert_eq!(read[3].one("vlan", "vlan.etype"), "34997");
    assert_eq!(read[3].one("frame", "frame.len"), "1518");
    assert_eq!(read[4].one("eth", "eth.type"), "35020");
    assert!(read[4].has("lldp"));
    assert_eq!(read[5].one("vlan", "vlan.priority"), "6");
    assert_eq!(read[5].one("vlan", "vlan.etype"), "34962");
    assert_eq!(read[5].one("pn_rt", "pn_rt.frame_id"), "32768");
    assert_eq!(read[6].one("eth", "eth.type"), "34962");
    assert_eq!(read[6].one("pn_dcp", "pn_dcp.service_id"), "5");
    assert_eq!((read[7].one("vlan", "vlan.priority"), read[7].one("vlan", "vlan.id"), read[7].one("vlan", "vlan.etype")), ("7".into(), "9".into(), "35020".into()));
    assert!(read[7].has("lldp"));
}

#[test]
fn a_tag_of_another_type_is_read_as_the_frame_s_type() {
    // One 802.1Q tag (0x8100) is read as the tag; an 802.1ad tag (0x88A8) or one of 0x9100 is not
    // (Wireshark reads both as tags): its type is the frame's type, its tag field the payload's
    // first two bytes. They are what this envelope never writes as a frame's type.
    for tpid in [0x88A8u16, 0x9100] {
        let mut f = vec![2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2];
        f.extend_from_slice(&tpid.to_be_bytes());
        f.extend_from_slice(&[0xC0, 0x05, 0x88, 0x92, 0xFE, 0xFE]);
        f.resize(60, 0);
        let e = ether::parse(&f).unwrap();
        assert_eq!((e.vlan, e.ethertype, &e.payload[..4]), (None, tpid, &[0xC0, 0x05, 0x88, 0x92][..]), "{tpid:#06x}");
        assert!(e.is_ethertype());
    }
}
