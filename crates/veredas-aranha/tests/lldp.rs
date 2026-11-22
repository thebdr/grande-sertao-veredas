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
            port_status: Some(PortStatus { rtc2: 0x0000, rtc3: 0x0002 }),
            mrp: Some(MrpPortStatus { domain_uuid: [0xAB; 16], mrrt: 2 }),
            chassis_mac: Some(DEVICE),
        },
        port_vlan: Some(1),
        mac_phy: Some(MacPhy { autoneg: 3, pmd_capabilities: 0x6C00, mau_type: 16 }),
        unknown: Vec::new(),
        unknown_org: Vec::new(),
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
    assert_eq!((mrp.mrrt_name(), mrp.domain_hex().as_str()), ("up", "abababababababababababababababab"));
    let names: Vec<&str> = (0..4).map(|m| MrpPortStatus { domain_uuid: [0; 16], mrrt: m }.mrrt_name()).collect();
    assert_eq!(names, ["off", "configured", "up", "unknown"]);
    assert!(!back.is_shutdown());
    assert_eq!(lldp::capability_names(0x0084), ["bridge", "station-only"]);
    assert_eq!(lldp::capability_names(0x07FF).len(), 11);
    assert!(lldp::capability_names(0xF800).is_empty());
    // The organisation TLVs to the byte: PROFINET's MRP port status and chassis MAC.
    let bytes = lldp::encode(&pdu).unwrap();
    let mrp_tlv = [&[0xFE, 0x16, 0x00, 0x0E, 0xCF, 4][..], &[0xAB; 16], &[0, 2]].concat();
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
    out.extend(tlv(1, b"\x07second"));
    out.extend(tlv(3, &[0, 20]));
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
    for (parts, missing) in [
        (vec![&port, &ttl], "chassis id"),
        (vec![&chassis, &ttl], "port id"),
        (vec![&chassis, &port], "time to live"),
        (vec![], "chassis id"),
    ] {
        let mut out: Vec<u8> = parts.into_iter().flatten().copied().collect();
        out.extend(tlv(0, &[]));
        assert_eq!(lldp::decode(&out), Err(LldpError::Missing(missing)), "{missing}");
    }
    // A TLV after the End TLV is the frame's pad, not the PDU's; a PDU with no End TLV ends at
    // its last whole TLV.
    let mut out = [chassis.clone(), port.clone(), ttl.clone(), tlv(0, &[]), tlv(5, b"late")].concat();
    assert_eq!(lldp::decode(&out).unwrap().sys_name, None);
    out.truncate(chassis.len() + port.len() + ttl.len());
    out.push(0x0A);
    assert_eq!(lldp::decode(&out).unwrap().ttl, 9);
    assert_eq!(
        LldpError::Overrun { index: 0, tlv_type: 1, len: 7, left: 3 }.to_string(),
        "TLV 0 (type 1) says 7 bytes and 3 are left"
    );
    assert_eq!(LldpError::Missing("port id").to_string(), "the mandatory port id TLV is missing");
}

#[test]
fn what_a_tlv_cannot_carry_is_refused_never_cut() {
    let base = || LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20);
    let mut pdu = base();
    pdu.sys_desc = Some("d".repeat(511));
    assert!(lldp::encode(&pdu).is_ok());
    pdu.sys_desc = Some("d".repeat(512));
    assert_eq!(lldp::encode(&pdu), Err(LldpError::TlvTooLong { tlv_type: 6, len: 512 }));
    let mut pdu = base();
    pdu.chassis = ChassisId::Local { id: "c".repeat(511) };
    assert_eq!(lldp::encode(&pdu), Err(LldpError::TlvTooLong { tlv_type: 1, len: 512 }));
    let mut pdu = base();
    pdu.unknown = vec![(128, vec![])];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::BadType(128)));
    for (len, ok) in [(0, false), (1, true), (31, true), (32, false)] {
        let mut pdu = base();
        pdu.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![1; len], if_subtype: 1, if_number: 0, oid: Vec::new() }];
        assert_eq!(lldp::encode(&pdu).is_ok(), ok, "{len}");
        if !ok {
            assert_eq!(lldp::encode(&pdu), Err(LldpError::BadAddress(len)));
        }
    }
    let mut pdu = base();
    pdu.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![1; 4], if_subtype: 1, if_number: 0, oid: vec![0; 256] }];
    assert_eq!(lldp::encode(&pdu), Err(LldpError::TlvTooLong { tlv_type: 8, len: 256 }));
    let mut pdu = base();
    pdu.sys_desc = Some("d".repeat(500));
    pdu.port_desc = Some("d".repeat(500));
    pdu.sys_name = Some("d".repeat(500));
    assert_eq!(lldp::frame(PORT, &pdu), Err(LldpError::Ether(ether::EtherError::TooLong(1520))));
}

#[test]
fn a_sender_leaving_says_a_time_to_live_of_zero() {
    let pdu = LldpPdu::new(ChassisId::Local { id: "station-a".into() }, PortId::Local { id: "port-001".into() }, 0);
    let back = lldp::decode(&lldp::encode(&pdu).unwrap()).unwrap();
    assert!(back.is_shutdown());
    assert_eq!(back.port.short(), "port-001");
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
            let len = usize::from(noise.next() % 24);
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
                let p1 = lldp::decode(&lldp::encode(&p0).unwrap()).unwrap();
                assert_eq!(p0, p1, "{:?}", &s[..cut]);
            }
        }
    }
    assert!(decoded > 20_000, "{decoded}");
}

#[test]
fn the_dissectors_read_every_frame_as_it_was_written() {
    let pdu = profinet_pdu();
    let mut switch = LldpPdu::new(ChassisId::Mac { mac: "02:1c:06:00:00:01".parse().unwrap() }, PortId::IfName { name: "eth3".into() }, 120);
    switch.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 1], if_subtype: 3, if_number: 7, oid: vec![0x2B, 0x06, 0x01] }];
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
    assert_eq!(p.one(l, "lldp.profinet.rtc2_port_status"), "0");
    assert_eq!(p.one(l, "lldp.profinet.rtc3_port_status"), "2");
    assert_eq!(p.one(l, "lldp.profinet.mrp_domain_uuid"), "abababab-abab-abab-abab-abababababab");
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
    assert_eq!(p.one(l, "lldp.mgn.interface.number"), "7");
    assert_eq!(p.one(l, "lldp.mgn.obj.len"), "3");
    assert_eq!(p.one(l, "lldp.profinet.mrrt_port_status"), "0");

    let p = &read[2];
    assert_eq!(p.one(l, "lldp.time_to_live"), "0");
    assert_eq!(p.one(l, "lldp.port.subtype"), "3");
    assert_eq!(p.one(l, "lldp.port.id.mac"), "02:00:00:00:00:08");
}
