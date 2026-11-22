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
    for s in ["02:0e:cf:12:34:56", "02:0E:CF:12:34:56", "02-0e-cf-12-34-56", "020e.cf12.3456", "020E.CF12.3456"] {
        assert_eq!(s.parse::<Mac>(), Ok(m), "{s}");
    }
    assert_eq!(m.to_string(), "02:0e:cf:12:34:56");
    assert_eq!(format!("{m:?}"), "Mac(02:0e:cf:12:34:56)");
    assert_eq!(m.oui(), [0x02, 0x0e, 0xcf]);
    assert!(!m.is_multicast());
    assert!(m.is_local());
    assert!(!mac("00:0e:cf:12:34:56").is_local());
}

#[test]
fn a_mac_refuses_every_other_spelling_with_the_text_it_got() {
    for s in [
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
        "02:0e:cf:12:34:56:",
        "aa.bb.cc.dd.ee.ff",
        "aabb:ccdd:eeff",
        "aabb-ccdd-eeff",
        "aabb.ccdd.eef",
        "aabb.ccdd.eeff.0000",
        "éabb.ccdd.eeff",
        "02:0e:cf:12:34:٥٦",
    ] {
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
    assert_eq!(Mac::LLDP_MULTICAST.to_string(), "01:80:c2:00:00:0e");
    assert_eq!(Mac::BROADCAST.to_string(), "ff:ff:ff:ff:ff:ff");
    for m in [Mac::DCP_MULTICAST, Mac::LLDP_MULTICAST, Mac::BROADCAST, mac("03:00:00:00:00:00")] {
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
    let err = serde_json::from_str::<Mac>("\"02:0e\"").unwrap_err().to_string();
    assert!(err.starts_with("not a MAC address: \"02:0e\""), "{err}");
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
    assert_eq!(&f[12..20], &[0x81, 0x00, 0xC0, 0x00, 0x88, 0x92, 0x80, 0x00]);
    assert_eq!(f.len(), 60);
    let e = ether::parse(&f).unwrap();
    assert_eq!((e.vlan, e.ethertype, &e.payload[..2]), (Some(tag), 0x8892, &[0x80, 0x00][..]));

    let every = VlanTag { priority: 7, drop_eligible: true, id: 4095 };
    let f = ether::build_tagged(dst, src, Some(every), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0xFF, 0xFF]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(every));
    // The drop bit alone, and the priority's low bit alone, read back apart.
    let drop_only = VlanTag { priority: 0, drop_eligible: true, id: 5 };
    let f = ether::build_tagged(dst, src, Some(drop_only), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0x10, 0x05]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(drop_only));
    let low_priority = VlanTag { priority: 1, drop_eligible: false, id: 5 };
    let f = ether::build_tagged(dst, src, Some(low_priority), 0x0800, &[]).unwrap();
    assert_eq!(&f[14..16], &[0x20, 0x05]);
    assert_eq!(ether::parse(&f).unwrap().vlan, Some(low_priority));
}

#[test]
fn what_the_wire_cannot_carry_is_refused_never_cut() {
    let (dst, src) = (mac("02:00:00:00:00:01"), mac("02:00:00:00:00:02"));
    assert_eq!(ether::build(dst, src, 0x88CC, &[0; 1500]).unwrap().len(), 1514);
    assert_eq!(ether::build(dst, src, 0x88CC, &[0; 1501]), Err(EtherError::TooLong(1501)));
    let tag = |priority, id| Some(VlanTag { priority, drop_eligible: false, id });
    // Tagged, the payload's limit is the same: the frame grows by the tag.
    assert_eq!(ether::build_tagged(dst, src, tag(6, 0), 0x8892, &[0; 1500]).unwrap().len(), 1518);
    assert_eq!(ether::build_tagged(dst, src, tag(6, 0), 0x8892, &[0; 1501]), Err(EtherError::TooLong(1501)));
    // A type that is a length, or the tag's own, is no EtherType to write.
    for t in [0x0000u16, 0x0026, 0x05DC, 0x05FF, 0x8100] {
        assert_eq!(ether::build(dst, src, t, &[1, 2, 3]), Err(EtherError::NotEtherType(t)), "{t:#x}");
        assert_eq!(ether::build_tagged(dst, src, tag(0, 1), t, &[1, 2, 3]), Err(EtherError::NotEtherType(t)), "{t:#x}");
    }
    assert!(ether::build(dst, src, 0x0600, &[]).is_ok());
    assert_eq!(
        EtherError::NotEtherType(0x26).to_string(),
        "0x0026 is not an EtherType this envelope writes (below 0x0600 is an 802.3 length; 0x8100 is the tag's own)"
    );
    assert_eq!(ether::build_tagged(dst, src, tag(8, 0), 0x8892, &[]), Err(EtherError::BadPriority(8)));
    assert_eq!(ether::build_tagged(dst, src, tag(0, 4096), 0x8892, &[]), Err(EtherError::BadVlanId(4096)));
    assert_eq!(EtherError::TooLong(1501).to_string(), "a payload of 1501 bytes is past Ethernet's 1500");
}

#[test]
fn a_length_field_is_read_as_a_length_never_as_an_ethertype() {
    // A spanning-tree frame: 802.3, its length 0x0026, an LLC header (42 42 03) following.
    let mut stp = vec![0x01, 0x80, 0xC2, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0x00, 0x26, 0x42, 0x42, 0x03];
    stp.resize(60, 0);
    let e = ether::parse(&stp).unwrap();
    assert!(e.is_length());
    assert_eq!((e.ethertype, &e.payload[..3]), (0x0026, &[0x42, 0x42, 0x03][..]));
    let lldp = ether::build(Mac::LLDP_MULTICAST, mac("02:00:00:00:00:02"), 0x88CC, &[0, 0]).unwrap();
    assert!(!ether::parse(&lldp).unwrap().is_length());
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
    ];
    let read = common::dissect(&frames);
    for (i, p) in read.iter().enumerate() {
        assert_eq!(p.complaints(), Vec::<String>::new(), "frame {i}");
        assert_eq!(p.one("eth", "eth.src"), "02:0a:0b:0c:0d:0e", "frame {i}");
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
}
