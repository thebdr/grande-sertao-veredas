//! LLDP with the organisation TLVs a PROFINET segment carries (contract: the LLDP clause). Exact
//! bytes both ways; every TLV it does not read kept with its bytes; damage named; what a TLV
//! cannot carry refused; total on any input; every frame read back by Wireshark's dissectors.

mod common;

use std::net::Ipv4Addr;

use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::lldp::{
    self, ChassisId, Delays, LldpError, LldpPdu, MacPhy, MgmtAddr, MrpPortStatus, OrgTlv, PnTlvs, PortId, PortStatus,
};
use veredas_aranha::profinet::mac::Mac;

const DEVICE: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x07]);
const PORT: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x08]);

/// What a PROFINET device's port says, every TLV this codec reads filled.
fn profinet_pdu() -> LldpPdu {
    LldpPdu {
        chassis: ChassisId::Local { id: "station-a".into() },
        port: PortId::Local { id: "port-002.station-a".into() },
        ttl: 20,
        port_desc: Some("port 2".into()),
        sys_name: Some("station-a".into()),
        sys_desc: Some("bench io module".into()),
        sys_caps: Some((0x0084, 0x0080)),
        mgmt_addrs: vec![MgmtAddr { subtype: 1, addr: vec![192, 168, 10, 11], if_subtype: 2, if_number: 1, oid: Vec::new() }],
        pn: PnTlvs {
            delays: Some(Delays { rx_local: 1, rx_remote: 2, tx_local: 3, tx_remote: 4, cable_local: 100 }),
            port_status: Some(PortStatus { rtc2: 0x0102, rtc3: 0x0002 }),
            mrp: Some(MrpPortStatus { domain_uuid: [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F], mrrt: 2 }),
            chassis_mac: Some(DEVICE),
        },
        port_vlan: Some(1),
        mac_phy: Some(MacPhy { autoneg: 3, pmd_capabilities: 0x6C00, mau_type: 16 }),
        unknown: Vec::new(),
        unknown_org: Vec::new(),
        end: Some(Vec::new()),
        trailing: Vec::new(),
    }
}

fn tlv(t: u8, v: &[u8]) -> Vec<u8> {
    let mut out = ((u16::from(t) << 9) | v.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(v);
    out
}

#[test]
fn the_smallest_pdu_is_its_three_mandatory_tlvs_and_the_end_to_the_byte() {
    let pdu = LldpPdu::new(ChassisId::Local { id: "station-a".into() }, PortId::Local { id: "port-001.station-a".into() }, 20);
    let f = lldp::frame(PORT, &pdu).unwrap();
    let e = ether::parse(&f).unwrap();
    assert_eq!((e.dst, e.src, e.ethertype), (Mac::LLDP_MULTICAST, PORT, 0x88CC));
    let mut want = vec![0x02, 0x0A, 7];
    want.extend_from_slice(b"station-a");
    want.extend_from_slice(&[0x04, 0x13, 7]);
    want.extend_from_slice(b"port-001.station-a");
    want.extend_from_slice(&[0x06, 0x02, 0, 20, 0, 0]);
    assert_eq!(lldp::encode(&pdu).unwrap(), want);
    assert_eq!(&e.payload[..want.len()], &want[..]);
    assert_eq!(f.len(), 60);
    assert_eq!(lldp::decode(e.payload).unwrap(), pdu);
}

#[test]
fn a_profinet_port_says_every_tlv_and_reads_back_whole() {
    let pdu = profinet_pdu();
    let f = lldp::frame(PORT, &pdu).unwrap();
    let back = lldp::decode(ether::parse(&f).unwrap().payload).unwrap();
    assert_eq!(back, pdu);
    assert_eq!(back.port.short(), "port-002");
    assert_eq!(back.chassis.key(), "local:station-a");
    assert_eq!(back.mgmt_addrs[0].ipv4(), Some(Ipv4Addr::new(192, 168, 10, 11)));
    let mrp = back.pn.mrp.as_ref().unwrap();
    assert_eq!((mrp.mrrt_name(), mrp.domain_hex().as_str()), ("up", "000102030405060708090a0b0c0d0e0f"));
    let names: Vec<&str> = (0..4).map(|m| MrpPortStatus { domain_uuid: [0; 16], mrrt: m }.mrrt_name()).collect();
    assert_eq!(names, ["off", "configured", "up", "unknown"]);
    assert!(!back.is_shutdown());
    assert_eq!(lldp::capability_names(0x0084), ["bridge", "station-only"]);
    assert_eq!(lldp::capability_names(0x07FF).len(), 11);
    assert!(lldp::capability_names(0xF800).is_empty());
    // The organisation TLVs to the byte: PROFINET's MRP port status and chassis MAC.
    let bytes = lldp::encode(&pdu).unwrap();
    let mrp_tlv = [&[0xFE, 0x16, 0x00, 0x0E, 0xCF, 4][..], &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F], &[0, 2]].concat();
    assert!(bytes.windows(mrp_tlv.len()).any(|w| w == mrp_tlv));
    let mac_tlv = [0xFE, 0x0A, 0x00, 0x0E, 0xCF, 5, 2, 0, 0, 0, 0, 7];
    assert!(bytes.windows(mac_tlv.len()).any(|w| w == mac_tlv));
    assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
}

#[test]
fn a_switch_port_reads_with_every_unread_tlv_kept_with_its_bytes() {
    let mut out = tlv(1, &[4, 0x02, 0x1c, 0x06, 0, 0, 1]);
    out.extend(tlv(2, b"\x05eth3"));
    out.extend(tlv(3, &[0, 120]));
    out.extend(tlv(9, &[1, 2, 3]));
    out.extend(tlv(127, &[0x00, 0x12, 0xBB, 7, 9, 9]));
    out.extend(tlv(127, &[0x00, 0x0E, 0xCF, 6, 1, 2, 3, 4]));
    out.extend(tlv(8, &[5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 3, 2, 0x2B, 0x06]));
    out.extend(tlv(0, &[]));
    out.extend_from_slice(&[0xEE; 7]);
    let pdu = lldp::decode(&out).unwrap();
    assert_eq!(pdu.chassis, ChassisId::Mac { mac: "02:1c:06:00:00:01".parse().unwrap() });
    assert_eq!(pdu.chassis.key(), "mac:02:1c:06:00:00:01");
    assert_eq!(pdu.port, PortId::IfName { name: "eth3".into() });
    assert_eq!((pdu.port.short(), pdu.ttl), ("eth3".to_string(), 120));
    assert_eq!(pdu.unknown, [(9, vec![1, 2, 3])]);
    assert_eq!(
        pdu.unknown_org,
        [OrgTlv { oui: [0x00, 0x12, 0xBB], subtype: 7, data: vec![9, 9] }, OrgTlv { oui: [0x00, 0x0E, 0xCF], subtype: 6, data: vec![1, 2, 3, 4] }]
    );
    assert_eq!(pdu.mgmt_addrs, [MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 1], if_subtype: 2, if_number: 3, oid: vec![0x2B, 0x06] }]);
    assert_eq!(pdu.pn, PnTlvs::default());
    // What it kept, it writes again: the PDU reads back the same.
    assert_eq!(lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap(), pdu);
}

#[test]
fn a_tlv_that_says_what_is_already_said_or_does_not_fit_is_kept_unread() {
    let mut out = tlv(1, b"\x07first");
    out.extend(tlv(2, b"\x07port-001"));
    out.extend(tlv(3, &[0, 20]));
    out.extend(tlv(1, b"\x07second"));
    out.extend(tlv(3, &[0, 99]));
    out.extend(tlv(5, b"name"));
    out.extend(tlv(5, b"other"));
    out.extend(tlv(6, &[0xFF, 0xFE]));
    out.extend(tlv(7, &[0, 4]));
    out.extend(tlv(8, &[5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 3, 5]));
    out.extend(tlv(127, &[0x00, 0x0E, 0xCF]));
    out.extend(tlv(127, &[0x00, 0x0E, 0xCF, 4, 1, 2, 3]));
    out.extend(tlv(127, &[0x00, 0x0E, 0xCF, 5, 2, 0, 0, 0, 0, 1]));
    out.extend(tlv(127, &[0x00, 0x0E, 0xCF, 5, 2, 0, 0, 0, 0, 2]));
    out.extend(tlv(0, &[]));
    let pdu = lldp::decode(&out).unwrap();
    assert_eq!(pdu.chassis, ChassisId::Local { id: "first".into() });
    assert_eq!((pdu.ttl, pdu.sys_name.as_deref(), pdu.sys_desc.as_deref()), (20, Some("name"), None));
    assert_eq!(pdu.pn.chassis_mac, Some(Mac([2, 0, 0, 0, 0, 1])));
    assert_eq!(
        pdu.unknown,
        [
            (1, b"\x07second".to_vec()),
            (3, vec![0, 99]),
            (5, b"other".to_vec()),
            (6, vec![0xFF, 0xFE]),
            (7, vec![0, 4]),
            (8, vec![5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 3, 5]),
            (127, vec![0x00, 0x0E, 0xCF]),
        ]
    );
    assert_eq!(
        pdu.unknown_org,
        [OrgTlv { oui: [0x00, 0x0E, 0xCF], subtype: 4, data: vec![1, 2, 3] }, OrgTlv { oui: [0x00, 0x0E, 0xCF], subtype: 5, data: vec![2, 0, 0, 0, 0, 2] }]
    );
    // An id whose text is not UTF-8 is kept as bytes under its subtype.
    let mut out = tlv(1, &[7, 0xC3]);
    out.extend(tlv(2, &[5, b'a', 0xFF]));
    out.extend(tlv(3, &[0, 1]));
    let pdu = lldp::decode(&out).unwrap();
    assert_eq!(pdu.chassis, ChassisId::Other { subtype: 7, data: vec![0xC3] });
    assert_eq!(pdu.port, PortId::Other { subtype: 5, data: vec![b'a', 0xFF] });
    assert_eq!(pdu.port.short(), "other5:61ff");
    assert_eq!(lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap(), pdu);
}

#[test]
fn damage_and_missing_tlvs_are_errors_that_name_them() {
    assert_eq!(lldp::decode(&[1]), Err(LldpError::TooShort(1)));
    assert_eq!(lldp::decode(&[0x02, 0x07, 4, 0, 0]), Err(LldpError::Overrun { index: 0, tlv_type: 1, len: 7, left: 3 }));
    let mut out = tlv(1, b"\x07x");
    out.extend_from_slice(&[0x04, 0x09, 7]);
    assert_eq!(lldp::decode(&out), Err(LldpError::Overrun { index: 1, tlv_type: 2, len: 9, left: 1 }));
    let chassis = tlv(1, b"\x07x");
    let port = tlv(2, b"\x07p");
    let ttl = tlv(3, &[0, 9]);
    for (parts, missing) in [(vec![&chassis, &port], "time to live"), (vec![&chassis], "port id"), (vec![], "chassis id")] {
        let mut out: Vec<u8> = parts.into_iter().flatten().copied().collect();
        out.extend(tlv(0, &[]));
        assert_eq!(lldp::decode(&out), Err(LldpError::Missing(missing)), "{missing}");
    }
    // The first three in their order, as 802.1AB has a receiver check them: out of it, the PDU is
    // refused, naming the first TLV out of place (the dissector reads none of it).
    for (parts, index, tlv_type) in [
        (vec![&port, &chassis, &ttl], 1, 2),
        (vec![&ttl, &chassis, &port], 1, 3),
        (vec![&chassis, &ttl, &port], 2, 3),
        (vec![&chassis, &chassis, &port, &ttl], 2, 1),
        (vec![&chassis, &port, &tlv(5, b"n"), &ttl], 3, 5),
    ] {
        let mut out: Vec<u8> = parts.into_iter().flatten().copied().collect();
        out.extend(tlv(0, &[]));
        assert_eq!(lldp::decode(&out), Err(LldpError::Order { index, tlv_type }), "{index} {tlv_type}");
    }
    assert_eq!(
        LldpError::Order { index: 1, tlv_type: 2 }.to_string(),
        "TLV 1 is of type 2: the first three are the chassis id (1), the port id (2) and the time to live (3), in that order"
    );
    let heard = [port.clone(), chassis.clone(), ttl.clone(), tlv(0, &[])].concat();
    let mut f = vec![0x01, 0x80, 0xC2, 0, 0, 0x0E, 2, 0, 0, 0, 0, 8, 0x88, 0xCC];
    f.extend_from_slice(&heard);
    f.resize(60, 0);
    let read = common::dissect(&[f]);
    assert!(read[0].complaints().iter().any(|c| c.contains("Invalid TLV type (0x02), expected ChassisID type (0x01)")), "{:?}", read[0].complaints());
    // A TLV after the End TLV is the frame's pad, not the PDU's; a PDU with no End TLV ends at
    // its last whole TLV.
    let mut out = [chassis.clone(), port.clone(), ttl.clone(), tlv(0, &[]), tlv(5, b"late")].concat();
    assert_eq!(lldp::decode(&out).unwrap().sys_name, None);
    out.truncate(chassis.len() + port.len() + ttl.len());
    out.push(0x0A);
    let pdu = lldp::decode(&out).unwrap();
    // The lone byte after the last whole TLV is kept, and written back.
    assert_eq!((pdu.ttl, pdu.end.clone(), pdu.trailing.clone()), (9, None, vec![0x0A]));
    assert_eq!(lldp::encode(&pdu).unwrap(), out);
    let mut wrong = pdu.clone();
    wrong.end = Some(vec![]);
    assert_eq!(lldp::encode(&wrong), Err(LldpError::Trailing(1)));
    wrong.end = None;
    wrong.trailing = vec![1, 2];
    assert_eq!(lldp::encode(&wrong), Err(LldpError::Trailing(2)));
    assert_eq!(
        LldpError::Overrun { index: 0, tlv_type: 1, len: 7, left: 3 }.to_string(),
        "TLV 0 (type 1) says 7 bytes and 3 are left"
    );
    assert_eq!(LldpError::Missing("port id").to_string(), "the mandatory port id TLV is missing");
}

#[test]
fn what_a_tlv_cannot_carry_is_refused_never_cut() {
    let base = || LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    let length = |what, min, max, len| Err(LldpError::Length { what, min, max, len });
    // Ids of 1 to 255 bytes, texts of up to 255 (802.1AB).
    for (id, ok) in [(0, false), (1, true), (255, true), (256, false)] {
        let mut pdu = base();
        pdu.chassis = ChassisId::Local { id: "c".repeat(id) };
        assert_eq!(lldp::encode(&pdu).is_ok(), ok, "chassis {id}");
        if !ok {
            assert_eq!(lldp::encode(&pdu), length("a chassis id", 1, 255, id));
        }
        let mut pdu = base();
        pdu.port = PortId::Local { id: "q".repeat(id) };
        assert_eq!(lldp::encode(&pdu).is_ok(), ok, "port {id}");
        if !ok {
            assert_eq!(lldp::encode(&pdu), length("a port id", 1, 255, id));
        }
    }
    let mut pdu = base();
    pdu.chassis = ChassisId::Network { data: vec![] };
    assert_eq!(lldp::encode(&pdu), length("a chassis id", 1, 255, 0));
    for (what, set) in [
        ("a port description", (|p: &mut LldpPdu, s| p.port_desc = Some(s)) as fn(&mut LldpPdu, String)),
        ("a system name", |p, s| p.sys_name = Some(s)),
        ("a system description", |p, s| p.sys_desc = Some(s)),
    ] {
        let mut pdu = base();
        set(&mut pdu, "d".repeat(255));
        assert!(lldp::encode(&pdu).is_ok(), "{what}");
        set(&mut pdu, "d".repeat(256));
        assert_eq!(lldp::encode(&pdu), length(what, 0, 255, 256));
    }
    // Any value past 511 bytes, and a kept TLV of type 0 or past 127.
    let mut pdu = base();
    pdu.unknown = vec![(9, vec![0; 511])];
    assert!(lldp::encode(&pdu).is_ok());
    pdu.unknown = vec![(9, vec![0; 512])];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::TlvTooLong { tlv_type: 9, len: 512 }));
    pdu.unknown_org = vec![OrgTlv { oui: [0, 0x12, 0xBB], subtype: 1, data: vec![0; 508] }];
    pdu.unknown = vec![];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::TlvTooLong { tlv_type: 127, len: 512 }));
    let mut pdu = base();
    pdu.unknown = vec![(128, vec![])];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::BadType(128)));
    pdu.unknown = vec![(9, vec![1, 2, 3]), (0, vec![5, 5]), (10, vec![4])];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::BadType(0)));
    // A management address: 1 to 31 bytes, its family's length where the family has one, an OID
    // of up to 255 bytes.
    for (subtype, len, ok) in [(5, 0, false), (5, 1, true), (5, 31, true), (5, 32, false), (1, 4, true), (1, 1, false), (2, 16, true), (2, 4, false), (6, 6, true), (6, 7, false)] {
        let mut pdu = base();
        pdu.mgmt_addrs = vec![MgmtAddr { subtype, addr: vec![1; len], if_subtype: 1, if_number: 0, oid: Vec::new() }];
        assert_eq!(lldp::encode(&pdu).is_ok(), ok, "{subtype} {len}");
        let want = match (ok, len) {
            (true, _) => continue,
            (false, 0 | 32) => length("a management address", 1, 31, len),
            (false, _) => Err(LldpError::BadAddress { subtype, want: [0, 4, 16, 0, 0, 0, 6][usize::from(subtype)], len }),
        };
        assert_eq!(lldp::encode(&pdu), want, "{subtype} {len}");
    }
    let mut pdu = base();
    pdu.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![1; 4], if_subtype: 1, if_number: 0, oid: vec![0; 128] }];
    assert!(lldp::encode(&pdu).is_ok());
    pdu.mgmt_addrs[0].oid.push(0);
    assert_eq!(lldp::encode(&pdu), length("an OID", 0, 128, 129));
    // An id of the MAC kind is six bytes; one of the network-address kind is its family's length.
    let id_form = |what, subtype, len| Err(LldpError::IdForm { what, subtype, len });
    for (chassis, want) in [
        (ChassisId::Other { subtype: 4, data: vec![1; 5] }, id_form("a chassis id", 4, 5)),
        (ChassisId::Other { subtype: 4, data: vec![1; 7] }, id_form("a chassis id", 4, 7)),
        (ChassisId::Network { data: vec![1, 10] }, id_form("a chassis id", 5, 2)),
        (ChassisId::Network { data: vec![1] }, id_form("a chassis id", 5, 1)),
        (ChassisId::Network { data: vec![2, 1, 2, 3, 4] }, id_form("a chassis id", 5, 5)),
        (ChassisId::Network { data: vec![1, 10, 0, 0, 1] }, Ok(())),
        (ChassisId::Network { data: vec![9, 1, 2] }, Ok(())),
    ] {
        let mut pdu = base();
        pdu.chassis = chassis.clone();
        assert_eq!(lldp::encode(&pdu).map(|_| ()), want, "{chassis:?}");
    }
    for (port, want) in [
        (PortId::Other { subtype: 3, data: vec![1; 5] }, id_form("a port id", 3, 5)),
        (PortId::Other { subtype: 4, data: vec![1, 10] }, id_form("a port id", 4, 2)),
        (PortId::Other { subtype: 4, data: vec![1, 10, 0, 0, 1] }, Ok(())),
    ] {
        let mut pdu = base();
        pdu.port = port.clone();
        assert_eq!(lldp::encode(&pdu).map(|_| ()), want, "{port:?}");
    }
    // And a PDU past the frame.
    let mut pdu = base();
    pdu.unknown = vec![(9, vec![0; 500]), (10, vec![0; 500]), (11, vec![0; 500])];
    assert_eq!(lldp::frame(PORT, &pdu), Err(LldpError::Ether(ether::EtherError::TooLong(1520))));
    assert_eq!(
        LldpError::Length { what: "a chassis id", min: 1, max: 255, len: 256 }.to_string(),
        "a chassis id is 1 to 255 bytes; this one is 256"
    );
}

#[test]
fn what_the_standard_does_not_let_a_tlv_say_is_read_as_unread() {
    let ttl = tlv(3, &[0, 20]);
    // An id of no bytes, or of 256, is no id: kept unread, and the PDU then has none.
    for (t, v, missing) in [(1u8, vec![7u8], "chassis id"), (1, [vec![7], vec![b'c'; 256]].concat(), "chassis id"), (2, vec![7], "port id")] {
        let out = if t == 1 { [tlv(1, &v), tlv(2, b"\x07p"), ttl.clone()].concat() } else { [tlv(1, b"\x07c"), tlv(2, &v), ttl.clone()].concat() };
        assert_eq!(lldp::decode(&out), Err(LldpError::Missing(missing)), "{t} {}", v.len());
    }
    let mut out = [tlv(1, b"\x07c"), tlv(2, b"\x07p"), ttl.clone()].concat();
    out.extend(tlv(1, &[7]));
    out.extend(tlv(5, &[b'n'; 256]));
    out.extend(tlv(8, &[2, 1, 10, 2, 0, 0, 0, 1, 0]));
    out.extend(tlv(8, &[5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 1, 0]));
    let pdu = lldp::decode(&out).unwrap();
    assert_eq!(pdu.sys_name, None);
    assert_eq!(pdu.mgmt_addrs.len(), 1);
    assert_eq!(pdu.unknown, [(1, vec![7]), (5, vec![b'n'; 256]), (8, vec![2, 1, 10, 2, 0, 0, 0, 1, 0])]);
    assert_eq!(lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap(), pdu);
}

#[test]
fn the_end_tlv_is_kept_as_heard_with_its_bytes_or_its_absence() {
    let head = [tlv(1, b"\x07a"), tlv(2, b"\x07p"), tlv(3, &[0, 20])].concat();
    // An End with a value: the value kept, written back.
    let with_value = [head.clone(), tlv(0, &[9, 9, 9])].concat();
    let pdu = lldp::decode(&with_value).unwrap();
    assert_eq!((pdu.end.clone(), pdu.unknown.clone()), (Some(vec![9, 9, 9]), vec![]));
    assert_eq!(lldp::encode(&pdu).unwrap(), with_value);
    // No End: none written back, so a full frame's PDU frames again.
    let mut full = head.clone();
    full.extend(tlv(9, &[0; 509]));
    full.extend(tlv(10, &[0; 509]));
    full.extend(tlv(11, &[0; 1500 - 12 - 2 * 511 - 2]));
    assert_eq!(full.len(), 1500);
    let pdu = lldp::decode(&full).unwrap();
    assert_eq!(pdu.end, None);
    assert_eq!(lldp::encode(&pdu).unwrap(), full);
    assert_eq!(lldp::frame(PORT, &pdu).unwrap().len(), 1514);
    // The standard's End is an empty one.
    assert_eq!(&lldp::encode(&LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 1)).unwrap()[12..], &[0, 0]);
}

#[test]
fn a_sender_leaving_says_a_time_to_live_of_zero() {
    let pdu = LldpPdu::new(ChassisId::Local { id: "station-a".into() }, PortId::Local { id: "port-001".into() }, 0);
    let back = lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap();
    assert!(back.is_shutdown());
    assert_eq!(back.port.short(), "port-001");
}

/// A PDU's TLVs as (type, value) up to its End or its last whole TLV, sorted, and its End's value.
type Tlvs = (Vec<(u8, Vec<u8>)>, Option<Vec<u8>>);
fn tlvs(p: &[u8]) -> Tlvs {
    let (mut out, mut off) = (Vec::new(), 0);
    while off + 2 <= p.len() {
        let head = u16::from_be_bytes([p[off], p[off + 1]]);
        let (t, len) = ((head >> 9) as u8, usize::from(head & 0x1FF));
        if off + 2 + len > p.len() {
            break;
        }
        let v = p[off + 2..off + 2 + len].to_vec();
        off += 2 + len;
        if t == 0 {
            out.sort();
            return (out, Some(v));
        }
        out.push((t, v));
    }
    out.sort();
    (out, None)
}

#[test]
fn every_tlv_the_codec_reads_kept_when_it_repeats_or_its_length_is_off_by_one() {
    let head = [tlv(1, b"\x07a"), tlv(2, b"\x07p")].concat();
    let org = |oui: [u8; 3], sub: u8, d: Vec<u8>| tlv(127, &[&oui[..], &[sub], &d].concat());
    let pn = [0x00, 0x0E, 0xCF];
    let ieee1 = [0x00, 0x80, 0xC2];
    let ieee3 = [0x00, 0x12, 0x0F];
    // (the TLV of the right size, the same type one byte short, one byte long)
    type Make = Box<dyn Fn(usize) -> Vec<u8>>;
    let cases: Vec<(&str, Make)> = vec![
        ("ttl", Box::new(|n| tlv(3, &vec![0x14; n]))),
        ("capabilities", Box::new(|n| tlv(7, &vec![0x01; n]))),
        ("delays", Box::new(move |n| org(pn, 1, vec![1; n]))),
        ("port status", Box::new(move |n| org(pn, 2, vec![1; n]))),
        ("mrp", Box::new(move |n| org(pn, 4, vec![1; n]))),
        ("chassis mac", Box::new(move |n| org(pn, 5, vec![2; n]))),
        ("port vlan", Box::new(move |n| org(ieee1, 1, vec![1; n]))),
        ("mac/phy", Box::new(move |n| org(ieee3, 1, vec![1; n]))),
    ];
    let sizes = [("ttl", 2), ("capabilities", 4), ("delays", 20), ("port status", 4), ("mrp", 18), ("chassis mac", 6), ("port vlan", 2), ("mac/phy", 5)];
    let read: [fn(&LldpPdu) -> bool; 8] = [
        |_| true,
        |p| p.sys_caps.is_some(),
        |p| p.pn.delays.is_some(),
        |p| p.pn.port_status.is_some(),
        |p| p.pn.mrp.is_some(),
        |p| p.pn.chassis_mac.is_some(),
        |p| p.port_vlan.is_some(),
        |p| p.mac_phy.is_some(),
    ];
    for (((name, make), (_, size)), is_read) in cases.iter().zip(sizes).zip(read) {
        // Alone: of its size read, one byte short or long kept unread (the time to live then
        // missing), and written back.
        let ttl_tlv = if *name == "ttl" { vec![] } else { tlv(3, &[0, 20]) };
        let right = lldp::decode(&[head.clone(), ttl_tlv.clone(), make(size), tlv(0, &[])].concat()).unwrap();
        assert!(is_read(&right), "{name}");
        assert_eq!(right.unknown.len() + right.unknown_org.len(), 0, "{name}");
        for n in [size - 1, size + 1] {
            let p = [head.clone(), ttl_tlv.clone(), make(n), tlv(0, &[])].concat();
            if *name == "ttl" {
                assert_eq!(lldp::decode(&p), Err(LldpError::Missing("time to live")), "{n}");
                continue;
            }
            let pdu = lldp::decode(&p).unwrap();
            assert!(!is_read(&pdu), "{name} of {n} bytes read: {pdu:?}");
            assert_eq!(pdu.unknown.len() + pdu.unknown_org.len(), 1, "{name} {n}");
            assert_eq!(tlvs(&lldp::encode(&pdu).unwrap()), tlvs(&p), "{name} {n}");
        }
        let ttl = if *name == "ttl" { vec![] } else { tlv(3, &[0, 20]) };
        for body in [
            vec![make(size), make(size)],
            vec![make(size), make(size - 1)],
            vec![make(size), make(size + 1)],
        ] {
            let mut p = [head.clone(), ttl.clone()].concat();
            for b in &body {
                p.extend_from_slice(b);
            }
            p.extend(tlv(0, &[]));
            let pdu = lldp::decode(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
            let written = lldp::encode(&pdu).unwrap();
            assert_eq!(tlvs(&written), tlvs(&p), "{name}: the second {} kept", body[1].len());
            let kept = pdu.unknown.len() + pdu.unknown_org.len();
            assert_eq!(kept, 1, "{name}: {pdu:?}");
        }
    }
    // A management address with a byte after its OID, and a MAC id of seven bytes: kept unread.
    let p = [head.clone(), tlv(3, &[0, 20]), tlv(8, &[5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 1, 1, 0x2B, 0xFF]), tlv(0, &[])].concat();
    let pdu = lldp::decode(&p).unwrap();
    assert_eq!((pdu.mgmt_addrs.len(), pdu.unknown.clone()), (0, vec![(8, vec![5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 1, 1, 0x2B, 0xFF])]));
    let p = [tlv(1, &[4, 2, 0, 0, 0, 0, 1, 9]), head.clone(), tlv(3, &[0, 20]), tlv(0, &[])].concat();
    assert_eq!(lldp::decode(&p), Err(LldpError::Order { index: 2, tlv_type: 1 }));
    assert_eq!(lldp::decode(&[tlv(1, &[4, 2, 0, 0, 0, 0, 1, 9]), tlv(2, b"\x07p"), tlv(3, &[0, 1])].concat()), Err(LldpError::Missing("chassis id")));
    // An OID of 129 bytes is past what the TLV carries (802.1AB: 167 bytes in all): kept unread.
    let mut m = vec![5, 1, 10, 0, 0, 1, 2, 0, 0, 0, 1, 129];
    m.extend_from_slice(&[0x2B; 129]);
    let p = [head, tlv(3, &[0, 20]), tlv(8, &m), tlv(0, &[])].concat();
    let pdu = lldp::decode(&p).unwrap();
    assert_eq!((pdu.mgmt_addrs.len(), pdu.unknown.len()), (0, 1));
}

#[test]
fn the_dissectors_read_text_as_ascii_and_stop_at_a_nul() {
    // The codec writes the text it is given; the dissector reads ASCII as it is (control
    // characters too), stops at a NUL (warning of what follows), and shows each byte past ASCII as
    // a replacement character.
    let mut nul = LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    nul.sys_desc = Some("a\0b".into());
    let mut accented = LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "porta-ü".into() }, 20);
    accented.sys_name = Some("estação".into());
    let mut control = LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    control.sys_name = Some("a\u{1}b\tc\u{7f}d".into());
    let read = common::dissect(&[lldp::frame(PORT, &nul).unwrap(), lldp::frame(PORT, &accented).unwrap(), lldp::frame(PORT, &control).unwrap()]);
    // Control characters are ASCII: read as they are, with no complaint.
    assert_eq!(read[2].one("lldp", "lldp.tlv.system.name"), "a\u{1}b\tc\u{7f}d");
    assert_eq!(read[2].complaints(), Vec::<String>::new());
    assert_eq!(read[0].one("lldp", "lldp.tlv.system.desc"), "a");
    assert_eq!(read[0].complaints().len(), 1, "{:?}", read[0].complaints());
    assert_eq!(lldp::decode(&lldp::encode(&nul).unwrap()).unwrap().sys_desc.as_deref(), Some("a\0b"));
    assert_eq!(read[1].complaints(), Vec::<String>::new());
    // One replacement character per byte that is not ASCII.
    assert_eq!(read[1].one("lldp", "lldp.tlv.system.name"), "esta\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}o");
    assert_eq!(read[1].one("lldp", "lldp.port.id"), "porta-\u{FFFD}\u{FFFD}");
    assert_eq!(lldp::decode(&lldp::encode(&accented).unwrap()).unwrap(), accented);
}

#[test]
fn a_short_pdu_with_no_end_keeps_its_absence_through_encode_not_through_a_padded_frame() {
    let p = [tlv(1, b"\x07a"), tlv(2, b"\x07p"), tlv(3, &[0, 20])].concat();
    let pdu = lldp::decode(&p).unwrap();
    assert_eq!(pdu.end, None);
    assert_eq!(lldp::encode(&pdu).unwrap(), p);
    // A frame would pad it, and the pad would read as more of it (an End, a byte, or after a
    // trailing byte a TLV running past the frame): refused, up to 45 bytes; at 46 the frame needs
    // no pad, and the absence survives.
    assert_eq!(lldp::frame(PORT, &pdu), Err(LldpError::NoEndInPad { len: 12, min: 46 }));
    for t in [0x0B, 0x0A, 0x02, 0x00] {
        let odd = LldpPdu { trailing: vec![t], ..pdu.clone() };
        assert_eq!(lldp::encode(&odd).unwrap(), [&p[..], &[t]].concat());
        assert_eq!(lldp::frame(PORT, &odd), Err(LldpError::NoEndInPad { len: 13, min: 46 }), "{t:#04x}");
    }
    let sized = |n: usize| LldpPdu { sys_desc: Some("d".repeat(n)), ..pdu.clone() };
    assert_eq!(lldp::encode(&sized(31)).unwrap().len(), 45);
    assert_eq!(lldp::frame(PORT, &sized(31)), Err(LldpError::NoEndInPad { len: 45, min: 46 }));
    let f = lldp::frame(PORT, &sized(32)).unwrap();
    assert_eq!(f.len(), 60);
    assert_eq!(lldp::decode(ether::parse(&f).unwrap().payload).unwrap(), sized(32));
    let with_end = LldpPdu { end: Some(vec![]), ..pdu.clone() };
    assert_eq!(lldp::frame(PORT, &with_end).unwrap().len(), 60);
    assert_eq!(
        LldpError::NoEndInPad { len: 12, min: 46 }.to_string(),
        "a PDU with no End is framed only when it fills the frame (46 bytes); this one is 12, and the pad would be read as more of it"
    );
}

#[test]
fn any_bytes_decode_without_a_panic_and_rewrite_to_themselves() {
    let mut seeds = vec![lldp::encode(&profinet_pdu()).unwrap()];
    let mut noise = common::Noise(0x11D9);
    for _ in 0..20_000 {
        // A run of TLV headers of plausible types and lengths, then bytes.
        let mut p = Vec::new();
        for t in [1u8, 2, 3] {
            // Mostly the shape a mandatory TLV has, sometimes not.
            let len = match (t, noise.next() % 8) {
                (_, 0) => usize::from(noise.next() % 12),
                (3, _) => 2,
                _ => 1 + usize::from(noise.next() % 11),
            };
            p.extend(tlv(t, &noise.bytes(len)));
        }
        let n = usize::from(noise.next() % 6);
        for _ in 0..n {
            let t = [4u8, 5, 6, 7, 8, 127, 127, 127, 9, 1][usize::from(noise.next()) % 10];
            let len = usize::from(noise.next() % 28);
            let mut v = noise.bytes(len);
            if t == 127 && v.len() >= 4 {
                let oui = [[0x00, 0x0E, 0xCF], [0x00, 0x80, 0xC2], [0x00, 0x12, 0x0F]][usize::from(noise.next()) % 3];
                v[..3].copy_from_slice(&oui);
                v[3] = noise.next() % 7;
            }
            p.extend(tlv(t, &v));
        }
        let extra = usize::from(noise.next() % 4);
        p.extend(noise.bytes(extra));
        seeds.push(p);
    }
    let mut decoded = 0;
    for s in &seeds {
        for cut in 0..=s.len() {
            if let Ok(p0) = lldp::decode(&s[..cut]) {
                decoded += 1;
                let written = lldp::encode(&p0).unwrap();
                let p1 = lldp::decode(&written).unwrap();
                assert_eq!(p0, p1, "{:?}", &s[..cut]);
                // Every TLV heard is written again, the same bytes, whatever their order; the End
                // as it was.
                assert_eq!(tlvs(&s[..cut]), tlvs(&written), "{:?}", &s[..cut]);
            }
        }
    }
    assert!(decoded > 20_000, "{decoded}");
}

#[test]
fn the_dissectors_read_every_frame_as_it_was_written() {
    let pdu = profinet_pdu();
    let mut switch = LldpPdu::new(ChassisId::Mac { mac: "02:1c:06:00:00:01".parse().unwrap() }, PortId::IfName { name: "eth3".into() }, 120);
    let fe80_1 = [0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1].to_vec();
    switch.mgmt_addrs = vec![
        MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 1], if_subtype: 3, if_number: 7, oid: vec![0x2B, 0x06, 0x01] },
        MgmtAddr { subtype: 2, addr: fe80_1, if_subtype: 2, if_number: 0x0102_0304, oid: vec![] },
    ];
    switch.unknown_org = vec![OrgTlv { oui: [0x00, 0x0E, 0xCF], subtype: 4, data: vec![0; 18] }];
    let leaving = LldpPdu::new(ChassisId::Local { id: "station-b".into() }, PortId::Mac { mac: PORT }, 0);
    let read = common::dissect(&[lldp::frame(PORT, &pdu).unwrap(), lldp::frame(PORT, &switch).unwrap(), lldp::frame(PORT, &leaving).unwrap()]);
    let l = "lldp";
    for (i, p) in read.iter().enumerate() {
        assert_eq!(p.complaints(), Vec::<String>::new(), "frame {i}");
        assert_eq!(p.one("eth", "eth.dst"), "01:80:c2:00:00:0e");
    }
    let p = &read[0];
    assert_eq!(p.one(l, "lldp.chassis.subtype"), "7");
    assert_eq!(p.one(l, "lldp.chassis.id"), "73:74:61:74:69:6f:6e:2d:61");
    assert_eq!(p.one(l, "lldp.port.subtype"), "7");
    assert_eq!(p.one(l, "lldp.port.id"), "port-002.station-a");
    assert_eq!(p.one(l, "lldp.time_to_live"), "20");
    assert_eq!(p.one(l, "lldp.port.desc"), "port 2");
    assert_eq!(p.one(l, "lldp.tlv.system.name"), "station-a");
    assert_eq!(p.one(l, "lldp.tlv.system.desc"), "bench io module");
    assert_eq!(p.one(l, "lldp.tlv.system_cap"), "132");
    assert_eq!(p.one(l, "lldp.tlv.enable_system_cap"), "128");
    assert_eq!(p.one(l, "lldp.mgn.addr.ip4"), "192.168.10.11");
    assert_eq!(p.one(l, "lldp.mgn.interface.subtype"), "2");
    assert_eq!(p.one(l, "lldp.mgn.interface.number"), "1");
    assert_eq!(p.one(l, "lldp.profinet.port_rx_delay_local"), "1");
    assert_eq!(p.one(l, "lldp.profinet.port_rx_delay_remote"), "2");
    assert_eq!(p.one(l, "lldp.profinet.port_tx_delay_local"), "3");
    assert_eq!(p.one(l, "lldp.profinet.port_tx_delay_remote"), "4");
    assert_eq!(p.one(l, "lldp.profinet.cable_delay_local"), "100");
    assert_eq!(p.one(l, "lldp.profinet.rtc2_port_status"), "258");
    assert_eq!(p.one(l, "lldp.profinet.rtc3_port_status"), "2");
    assert_eq!(p.one(l, "lldp.profinet.mrp_domain_uuid"), "00010203-0405-0607-0809-0a0b0c0d0e0f");
    assert_eq!(p.one(l, "lldp.profinet.mrrt_port_status"), "2");
    assert_eq!(p.one(l, "lldp.profinet.cm_mac_add"), "02:00:00:00:00:07");
    assert_eq!(p.one(l, "lldp.ieee.802_1.port_vlan.id"), "1");
    assert_eq!(p.one(l, "lldp.ieee.802_3.mac_phy_auto_neg_status"), "3");
    assert_eq!(p.one(l, "lldp.ieee.802_3.pmd_auto_neg_advertised_caps"), "27648");
    assert_eq!(p.one(l, "lldp.ieee.802_3.pmd_mau_type"), "16");
    assert_eq!(p.values(l, "lldp.tlv.type"), ["1", "2", "3", "4", "5", "6", "7", "8", "127", "127", "127", "127", "127", "127", "0"]);

    let p = &read[1];
    assert_eq!(p.one(l, "lldp.chassis.subtype"), "4");
    assert_eq!(p.one(l, "lldp.chassis.id.mac"), "02:1c:06:00:00:01");
    assert_eq!(p.one(l, "lldp.port.subtype"), "5");
    assert_eq!(p.one(l, "lldp.port.id"), "eth3");
    assert_eq!(p.one(l, "lldp.time_to_live"), "120");
    assert_eq!(p.one(l, "lldp.mgn.addr.ip4"), "10.0.0.1");
    assert_eq!(p.one(l, "lldp.mgn.addr.ip6"), "fe80::1");
    assert_eq!(p.values(l, "lldp.mgn.interface.subtype"), ["3", "2"]);
    assert_eq!(p.values(l, "lldp.mgn.interface.number"), ["7", "16909060"]);
    assert_eq!(p.values(l, "lldp.mgn.obj.len"), ["3", "0"]);
    assert_eq!(p.one(l, "lldp.mgn.obj.id"), "1.3.6.1");
    assert_eq!(p.one(l, "lldp.profinet.mrrt_port_status"), "0");

    let p = &read[2];
    assert_eq!(p.one(l, "lldp.time_to_live"), "0");
    assert_eq!(p.one(l, "lldp.port.subtype"), "3");
    assert_eq!(p.one(l, "lldp.port.id.mac"), "02:00:00:00:00:08");
}

#[test]
fn what_the_writer_takes_at_each_bound_the_reader_reads_back() {
    let base = || LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    let mut cases = Vec::new();
    let mut p = base();
    p.chassis = ChassisId::Local { id: "c".repeat(255) };
    cases.push(p);
    let mut p = base();
    p.port = PortId::Local { id: "q".repeat(255) };
    cases.push(p);
    let mut p = base();
    p.chassis = ChassisId::Network { data: vec![9, 1] };
    cases.push(p);
    let mut p = base();
    (p.port_desc, p.sys_name, p.sys_desc) = (Some("d".repeat(255)), Some("n".repeat(255)), Some("s".repeat(255)));
    cases.push(p);
    for (subtype, len, oid) in [(5u8, 1usize, 0usize), (5, 31, 0), (1, 4, 128)] {
        let mut p = base();
        p.mgmt_addrs = vec![MgmtAddr { subtype, addr: vec![1; len], if_subtype: 2, if_number: 1, oid: vec![0x2B; oid] }];
        cases.push(p);
    }
    for pdu in cases {
        let bytes = lldp::encode(&pdu).unwrap();
        assert_eq!(lldp::decode(&bytes).unwrap(), pdu, "{pdu:?}");
    }
    // And past each bound, what the writer refuses the reader keeps unread.
    let long_port = [tlv(1, b"\x07c"), tlv(2, &[&[7u8][..], &[b'q'; 256]].concat()), tlv(3, &[0, 20])].concat();
    assert_eq!(lldp::decode(&long_port), Err(LldpError::Missing("port id")));
    let ok = [tlv(1, b"\x07c"), tlv(2, b"\x07p"), tlv(3, &[0, 20])].concat();
    for address_string in [1u8, 33] {
        let mut m = vec![address_string, 1];
        m.extend_from_slice(&vec![10; usize::from(address_string).saturating_sub(1)]);
        m.extend_from_slice(&[2, 0, 0, 0, 1, 0]);
        let pdu = lldp::decode(&[&ok[..], &tlv(8, &m)].concat()).unwrap();
        assert_eq!((pdu.mgmt_addrs.len(), pdu.unknown.clone()), (0, vec![(8, m.clone())]), "{address_string}");
    }
}

#[test]
fn every_kind_of_id_reads_from_its_bytes_with_its_key() {
    let read = |chassis: &[u8], port: &[u8]| lldp::decode(&[tlv(1, chassis), tlv(2, port), tlv(3, &[0, 20])].concat()).unwrap();
    let pdu = read(&[5, 1, 10, 0, 0, 1], &[1, b'u', b'p', b'l', b'i', b'n', b'k']);
    assert_eq!(pdu.chassis, ChassisId::Network { data: vec![1, 10, 0, 0, 1] });
    assert_eq!(pdu.chassis.key(), "net:010a000001");
    assert_eq!(pdu.port, PortId::IfAlias { name: "uplink".into() });
    assert_eq!(pdu.port.short(), "uplink");
    let pdu = read(&[6, b'e', b't', b'h', b'0'], &[3, 2, 0, 0, 0, 0, 9]);
    assert_eq!(pdu.chassis, ChassisId::IfName { name: "eth0".into() });
    assert_eq!(pdu.chassis.key(), "ifname:eth0");
    assert_eq!(pdu.port, PortId::Mac { mac: Mac([2, 0, 0, 0, 0, 9]) });
    assert_eq!(pdu.port.short(), "02:00:00:00:00:09");
    let pdu = read(&[1, b'x'], &[2, b'y']);
    assert_eq!((pdu.chassis.key(), pdu.port.short()), ("other1:78".to_string(), "other2:79".to_string()));
}

#[test]
fn the_dissector_reads_an_oid_as_ber_and_no_end_past_what_it_does_not_read() {
    let base = || LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    let with_oid = |oid: Vec<u8>| {
        let mut p = base();
        p.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 1], if_subtype: 2, if_number: 1, oid }];
        p
    };
    let mut end_bytes = base();
    end_bytes.end = Some(vec![1, 2, 3]);
    let mut kept = base();
    kept.unknown = vec![(9, vec![1, 2, 3])];
    let frames: Vec<Vec<u8>> = [with_oid(vec![0x2B, 0x06, 0x01]), with_oid(vec![0xFF]), end_bytes.clone(), kept.clone()]
        .iter()
        .map(|p| lldp::frame(PORT, p).unwrap())
        .collect();
    let read = common::dissect(&frames);
    let l = "lldp";
    // The OID is bytes to this codec; the dissector reads them as a BER object identifier, and
    // shows bytes that are not one as it reads them.
    assert_eq!(read[0].one(l, "lldp.mgn.obj.id"), "1.3.6.1");
    assert_eq!(read[1].one(l, "lldp.mgn.obj.id"), "0");
    assert_eq!(read[1].complaints(), Vec::<String>::new());
    // An End holding bytes, which this codec keeps as heard, the dissector calls malformed; a TLV
    // of a reserved type ends its reading, so it reads neither that TLV nor the End after it.
    let c = read[2].complaints();
    assert_eq!(c.len(), 1, "{c:?}");
    assert!(c[0].contains("Invalid Length (3) for Type (End of LLDPDU), expected (0)"), "{c:?}");
    assert_eq!(read[3].values(l, "lldp.tlv.type"), ["1", "2", "3"]);
    assert_eq!(lldp::decode(&lldp::encode(&end_bytes).unwrap()).unwrap(), end_bytes);
    assert_eq!(lldp::decode(&lldp::encode(&kept).unwrap()).unwrap(), kept);
}

#[test]
fn an_id_or_address_not_of_its_family_s_length_is_refused_and_kept_unread() {
    let base = || LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    // A network-address id is its family byte and an address of its family's length: an IPv4 id of
    // five bytes or of three, and a family byte alone, are refused, chassis and port alike.
    for data in [vec![1u8, 10, 0, 0, 1, 9], vec![1, 10, 0, 0], vec![9], vec![2, 0xFE, 0x80]] {
        let len = data.len();
        let pdu = LldpPdu { chassis: ChassisId::Network { data: data.clone() }, ..base() };
        assert_eq!(lldp::encode(&pdu), Err(LldpError::IdForm { what: "a chassis id", subtype: 5, len }), "{data:?}");
        let pdu = LldpPdu { port: PortId::Other { subtype: 4, data: data.clone() }, ..base() };
        assert_eq!(lldp::encode(&pdu), Err(LldpError::IdForm { what: "a port id", subtype: 4, len }), "{data:?}");
        // Heard, the same bytes are no id: kept unread, and the PDU has none.
        let chassis = [tlv(1, &[&[5u8][..], &data].concat()), tlv(2, b"\x07p"), tlv(3, &[0, 20])].concat();
        assert_eq!(lldp::decode(&chassis), Err(LldpError::Missing("chassis id")), "{data:?}");
        let port = [tlv(1, b"\x07a"), tlv(2, &[&[4u8][..], &data].concat()), tlv(3, &[0, 20])].concat();
        assert_eq!(lldp::decode(&port), Err(LldpError::Missing("port id")), "{data:?}");
    }
    // Of its family's length, read.
    let pdu = LldpPdu { chassis: ChassisId::Network { data: vec![1, 10, 0, 0, 1] }, port: PortId::Other { subtype: 4, data: vec![1, 10, 0, 0, 2] }, ..base() };
    assert_eq!(lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap(), pdu);

    // A management address heard of another length than its family's, or an address string
    // outside 2 to 32 bytes whatever the family: kept unread.
    let ok = [tlv(1, b"\x07c"), tlv(2, b"\x07p"), tlv(3, &[0, 20])].concat();
    let mgmt = |family: u8, addr: &[u8]| [&[addr.len() as u8 + 1, family][..], addr, &[2, 0, 0, 0, 1, 0]].concat();
    for m in [mgmt(1, &[10, 0, 0]), mgmt(1, &[10, 0, 0, 1, 9]), mgmt(1, &[10]), mgmt(2, &[0xFE; 15]), mgmt(6, &[2; 7]), mgmt(5, &[]), mgmt(5, &[7; 32])] {
        let pdu = lldp::decode(&[&ok[..], &tlv(8, &m)].concat()).unwrap();
        assert_eq!((pdu.mgmt_addrs.len(), pdu.unknown.clone()), (0, vec![(8, m.clone())]), "{m:?}");
        assert_eq!(lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap(), pdu);
    }
    // At each family's length, and of an unknown family from 1 to 31 bytes, read.
    for m in [mgmt(1, &[10, 0, 0, 1]), mgmt(2, &[0xFE; 16]), mgmt(6, &[2; 6]), mgmt(5, &[7]), mgmt(5, &[7; 31])] {
        let pdu = lldp::decode(&[&ok[..], &tlv(8, &m)].concat()).unwrap();
        assert_eq!((pdu.mgmt_addrs.len(), pdu.unknown.len()), (1, 0), "{m:?}");
    }

    // Two management addresses, IPv4 then IPv6: both read, both written back to the byte.
    let both = [&ok[..], &tlv(8, &mgmt(1, &[10, 0, 0, 1])), &tlv(8, &mgmt(2, &[0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])), &tlv(0, &[])].concat();
    let pdu = lldp::decode(&both).unwrap();
    assert_eq!(pdu.mgmt_addrs.iter().map(|m| (m.subtype, m.addr.len())).collect::<Vec<_>>(), [(1, 4), (2, 16)]);
    assert_eq!(lldp::encode(&pdu).unwrap(), both);
}

#[test]
fn every_capability_bit_is_named_as_the_dissector_names_it() {
    let names = ["other", "repeater", "bridge", "wlan-access-point", "router", "telephone", "docsis-cable-device", "station-only", "c-vlan", "s-vlan", "tpmr"];
    let fields = ["other", "repeater", "bridge", "wlan_access_pt", "router", "telephone", "docsis_cable_device", "station_only", "cvlan", "svlan", "tpmr"];
    assert_eq!(lldp::capability_names(0x07FF), names);
    assert_eq!(lldp::capability_names(0xF800), Vec::<&str>::new());
    let frames: Vec<Vec<u8>> = (0..11)
        .map(|i| {
            let mut p = LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
            p.sys_caps = Some((1 << i, 1 << i));
            lldp::frame(PORT, &p).unwrap()
        })
        .collect();
    let read = common::dissect(&frames);
    for (i, d) in read.iter().enumerate() {
        assert_eq!(lldp::capability_names(1 << i), [names[i]]);
        for (j, f) in fields.iter().enumerate() {
            let v = d.values("lldp", &format!("lldp.tlv.system_cap.{f}"));
            let set = v.iter().any(|x| x == "true" || x == "1");
            assert_eq!(set, i == j, "bit {i}, field {f}: {v:?}");
        }
    }
    // The short port name drops a station suffix only from a PROFINET port.
    assert_eq!(PortId::Local { id: "port-003.station-c".into() }.short(), "port-003");
    assert_eq!(PortId::Local { id: "eth.1".into() }.short(), "eth.1");
    assert_eq!(PortId::Local { id: "port-003".into() }.short(), "port-003");
}

#[test]
fn ids_of_any_bytes_read_as_their_kind_and_the_dissector_reads_addresses_as_it_does() {
    let read = |chassis: &[u8], port: &[u8]| lldp::decode(&[tlv(1, chassis), tlv(2, port), tlv(3, &[0, 20])].concat()).unwrap();
    // A MAC id of bytes past ASCII, chassis and port; network ids of IPv4 and IPv6.
    let pdu = read(&[4, 0x00, 0x0E, 0xCF, 0x12, 0x34, 0x56], &[3, 0x00, 0x1B, 0x1B, 0xAA, 0xBB, 0xCC]);
    assert_eq!(pdu.chassis, ChassisId::Mac { mac: Mac([0x00, 0x0E, 0xCF, 0x12, 0x34, 0x56]) });
    assert_eq!(pdu.chassis.key(), "mac:00:0e:cf:12:34:56");
    assert_eq!(pdu.port, PortId::Mac { mac: Mac([0x00, 0x1B, 0x1B, 0xAA, 0xBB, 0xCC]) });
    let pdu = read(&[5, 1, 192, 168, 0, 1], &[7, b'p']);
    assert_eq!((pdu.chassis.clone(), pdu.chassis.key()), (ChassisId::Network { data: vec![1, 192, 168, 0, 1] }, "net:01c0a80001".to_string()));
    let fe80: Vec<u8> = [&[2u8, 0xFE, 0x80][..], &[0; 13], &[1]].concat();
    let pdu = read(&[&[5u8][..], &fe80].concat(), &[7, b'p']);
    assert_eq!(pdu.chassis, ChassisId::Network { data: fe80.clone() });

    // Through tshark: network ids of IPv4 and IPv6, chassis and port, read as addresses; a port id
    // of an 802 address family, which this codec takes as a network id of its family's length,
    // the dissector reads as text after the family byte, stopping at its first NUL and warning of
    // the rest.
    let base = |chassis: ChassisId, port: PortId| LldpPdu::new(chassis, port, 20);
    let frames: Vec<Vec<u8>> = [
        base(ChassisId::Network { data: vec![1, 192, 168, 0, 1] }, PortId::Other { subtype: 4, data: vec![1, 10, 0, 0, 2] }),
        base(ChassisId::Network { data: fe80.clone() }, PortId::Other { subtype: 4, data: fe80.clone() }),
        base(ChassisId::Local { id: "a".into() }, PortId::Other { subtype: 4, data: vec![6, 2, 0, 0, 0, 0, 1] }),
    ]
    .iter()
    .map(|p| lldp::frame(PORT, p).unwrap())
    .collect();
    let d = common::dissect(&frames);
    assert_eq!((d[0].one("lldp", "lldp.chassis.id.ip4"), d[0].one("lldp", "lldp.port.id.ip4")), ("192.168.0.1".to_string(), "10.0.0.2".to_string()));
    assert_eq!((d[1].one("lldp", "lldp.chassis.id.ip6"), d[1].one("lldp", "lldp.port.id.ip6")), ("fe80::1".to_string(), "fe80::1".to_string()));
    assert_eq!((d[0].complaints(), d[1].complaints()), (Vec::<String>::new(), Vec::<String>::new()));
    assert_eq!(d[2].one("lldp", "lldp.port.id"), "\u{2}");
    assert!(d[2].complaints().iter().any(|c| c.contains("Trailing stray characters")), "{:?}", d[2].complaints());

    // The kept TLVs are written with their bytes, the organisation TLVs first, then the others,
    // each in the order heard; a reader that stops at one it cannot read (a malformed organisation
    // TLV here) reads no further.
    let heard = [tlv(1, b"\x07a"), tlv(2, b"\x07p"), tlv(3, &[0, 20]), tlv(5, b"n"), tlv(5, b"m"), tlv(127, &[0x00, 0x0E, 0xCF, 2, 1, 2, 3]), tlv(9, b"x"), tlv(0, &[])].concat();
    let pdu = lldp::decode(&heard).unwrap();
    let written = lldp::encode(&pdu).unwrap();
    assert_eq!(&written[12..], &[tlv(5, b"n"), tlv(127, &[0x00, 0x0E, 0xCF, 2, 1, 2, 3]), tlv(5, b"m"), tlv(9, b"x"), tlv(0, &[])].concat()[..]);
    let d = common::dissect(&[lldp::frame(PORT, &pdu).unwrap()]);
    assert_eq!(d[0].values("lldp", "lldp.tlv.system.name"), ["n"]);

    // The short port name drops all of a dotted station name.
    assert_eq!(PortId::Local { id: "port-001.io.cell-2".into() }.short(), "port-001");
}
