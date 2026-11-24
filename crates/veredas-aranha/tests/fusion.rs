//! Observations into devices (contract: the fusion clause): the MAC the identity, every value kept
//! with its witnesses, the value heard last current, ports and their leaving, every conflict on
//! record, and the same observations giving the same devices in whatever order they are applied.

mod common;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use veredas_aranha::fusion::{Conflict, ConflictKind, Fusion};
use veredas_aranha::observe::{self, Observation, Observed, Probe, Witness};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, Filter, IpStatus, IpSuite};
use veredas_aranha::profinet::lldp::{self, ChassisId, Delays, LldpPdu, MacPhy, MgmtAddr, MrpPortStatus, PortId, PortStatus};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::Frame;

const DEV: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
const DEV2: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x03]);
const PORT: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
const ME: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xAA]);

fn suite(last: u8) -> IpSuite {
    IpSuite { ip: Ipv4Addr::new(192, 168, 0, last), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }
}

fn identity(name: &str, last: u8) -> DeviceIdentity {
    DeviceIdentity {
        name: Some(name.into()),
        alias: Some(format!("port-001.{name}")),
        ip: Some(suite(last)),
        ip_status: Some(IpStatus { set: 1, conflict: false }),
        vendor_id: Some(0x1234),
        device_id: Some(0x0203),
        vendor: Some("bench device".into()),
        role: Some(1),
        instance: Some((0, 1)),
        ..Default::default()
    }
}

/// The observations a frame gives, kept at `at_ns` on `iface` as frame `index`.
fn heard(bytes: Vec<u8>, iface: &str, at_ns: u64, index: u64) -> Vec<Observation> {
    observe::decode(&Frame::new(at_ns, iface, bytes), index).observations
}

fn apply(f: &mut Fusion, obs: Vec<Observation>) {
    for o in &obs {
        f.apply(o);
    }
}

fn pn_lldp(name: &str, port: &str, ttl: u16) -> LldpPdu {
    let mut p = LldpPdu::new(ChassisId::Local { id: name.into() }, PortId::Local { id: format!("{port}.{name}") }, ttl);
    p.pn.chassis_mac = Some(DEV);
    p
}

#[test]
fn a_device_is_its_mac_and_holds_what_its_dcp_says() {
    let mut f = Fusion::new();
    apply(&mut f, heard(dcp::hello(DEV, 1, &identity("io-device-1", 10)).unwrap(), "eth1", 100, 0));
    apply(&mut f, heard(dcp::identify_response(ME, DEV, 2, &identity("io-device-1", 10)).unwrap(), "eth1", 200, 1));
    assert_eq!(f.devices.len(), 1);
    let d = &f.devices[&DEV];
    assert_eq!(d.mac, DEV);
    assert_eq!(d.name.current().map(String::as_str), Some("io-device-1"));
    assert_eq!(d.alias.current().map(String::as_str), Some("port-001.io-device-1"));
    assert_eq!(d.ip.current(), Some(&suite(10)));
    assert_eq!(d.ip_status.current(), Some(&IpStatus { set: 1, conflict: false }));
    assert_eq!(d.pn_id.current(), Some(&(0x1234, 0x0203)));
    assert_eq!(d.vendor.current().map(String::as_str), Some("bench device"));
    assert_eq!((d.role.current(), d.instance.current()), (Some(&1), Some(&(0, 1))));
    // Every value once, heard twice: its first and last witnesses and its count.
    let name = &d.name.candidates;
    assert_eq!(name.len(), 1);
    assert_eq!((name[0].count, name[0].first.at_ns, name[0].first.frame, name[0].last.at_ns, name[0].last.frame), (2, 100, 0, 200, 1));
    assert_eq!((name[0].first.probe, name[0].last.probe), (Probe::DcpHeard, Probe::DcpHeard));
    assert_eq!((d.first_ns, d.last_ns, d.frames), (100, 200, 2));
    assert_eq!(d.heard.iter().copied().collect::<Vec<_>>(), [Probe::DcpHeard]);
    assert_eq!(d.label(), "io-device-1");
    assert!(f.conflicts().is_empty());
    // A station that asks is on the segment too, and says nothing of itself.
    apply(&mut f, heard(dcp::identify_request(ME, 3, 1, &Filter::All).unwrap(), "eth1", 300, 2));
    let asker = &f.devices[&ME];
    assert_eq!(asker.heard.iter().copied().collect::<Vec<_>>(), [Probe::DcpRequestHeard]);
    assert!(asker.name.is_empty() && asker.ip.is_empty() && asker.pn_id.is_empty());
    assert_eq!((asker.label(), asker.frames), (ME.to_string(), 1));
}

#[test]
fn every_value_is_kept_with_its_witnesses_and_the_one_heard_last_is_current() {
    let mut f = Fusion::new();
    let w = |at_ns, frame| Witness { probe: Probe::DcpHeard, iface: "eth1".into(), at_ns, frame };
    let say = |name: &str, last: u8, at_ns, frame| Observation {
        witness: w(at_ns, frame),
        observed: Observed::Dcp { mac: DEV, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity: identity(name, last) },
    };
    // Renamed and readdressed, then the old name heard again later, then an older hearing applied
    // last (a capture merged out of order): time decides what is current, not the order applied.
    for o in [say("a", 10, 100, 0), say("b", 11, 200, 1), say("a", 10, 300, 2), say("b", 11, 150, 3)] {
        f.apply(&o);
    }
    let d = &f.devices[&DEV];
    assert_eq!(d.name.candidates.iter().map(|c| (c.value.as_str(), c.count, c.first.at_ns, c.last.at_ns)).collect::<Vec<_>>(), [("a", 2, 100, 300), ("b", 2, 150, 200)]);
    assert_eq!(d.name.current().map(String::as_str), Some("a"));
    assert_eq!(d.ip.current(), Some(&suite(10)));
    assert!(d.name.conflicted() && d.ip.conflicted());
    // Two hearings of one stamp: the later frame is the later.
    f.apply(&say("b", 11, 300, 4));
    assert_eq!(f.devices[&DEV].name.current().map(String::as_str), Some("b"));
    assert_eq!(
        f.conflicts(),
        [
            Conflict { kind: ConflictKind::NameChanged, value: "a, b".into(), macs: vec![DEV] },
            Conflict { kind: ConflictKind::IpChanged, value: "192.168.0.10, 192.168.0.11".into(), macs: vec![DEV] },
        ]
    );
    // Two hearings of one stamp, the earlier-heard value in the later frame: the frame decides.
    let mut tie = Fusion::new();
    for o in [say("a", 10, 100, 0), say("b", 11, 300, 2), say("a", 10, 300, 4)] {
        tie.apply(&o);
    }
    assert_eq!(tie.devices[&DEV].name.current().map(String::as_str), Some("a"));
    // The same observations in the same order give the same devices.
    let mut again = Fusion::new();
    for o in [say("a", 10, 100, 0), say("b", 11, 200, 1), say("a", 10, 300, 2), say("b", 11, 150, 3), say("b", 11, 300, 4)] {
        again.apply(&o);
    }
    assert_eq!(again, f);
}

#[test]
fn an_lldp_frame_describes_its_device_s_port_and_its_leaving() {
    let mut f = Fusion::new();
    let mut pdu = pn_lldp("io-device-1", "port-001", 20);
    pdu.sys_name = Some("io-device-1".into());
    pdu.sys_desc = Some("bench".into());
    pdu.sys_caps = Some((0x0080, 0x0080));
    pdu.port_desc = Some("first port".into());
    pdu.pn.port_status = Some(PortStatus { rtc2: 1, rtc3: 0 });
    pdu.port_vlan = Some(7);
    pdu.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 9], if_subtype: 2, if_number: 1, oid: vec![] }];
    apply(&mut f, heard(lldp::frame(PORT, &pdu).unwrap(), "eth1", 100, 0));
    let d = &f.devices[&DEV];
    assert_eq!(d.chassis.current().map(String::as_str), Some("local:io-device-1"));
    assert_eq!((d.sys_name.current().map(String::as_str), d.sys_desc.current().map(String::as_str), d.sys_caps.current()), (Some("io-device-1"), Some("bench"), Some(&(0x0080, 0x0080))));
    assert_eq!(d.mgmt_ips.current(), Some(&BTreeSet::from([Ipv4Addr::new(10, 0, 0, 9)])));
    let p = &d.ports["port-001"];
    assert_eq!((p.id.as_str(), p.port_mac.current(), p.desc.current().map(String::as_str)), ("port-001", Some(&PORT), Some("first port")));
    assert_eq!((p.pn_status.current(), p.pvid.current(), p.left_ns), (Some(&PortStatus { rtc2: 1, rtc3: 0 }), Some(&7), None));
    assert_eq!((p.heard_on.iter().cloned().collect::<Vec<_>>(), p.frames, p.first_ns, p.last_ns), (vec!["eth1".to_string()], 1, 100, 100));
    assert_eq!(d.heard.iter().copied().collect::<Vec<_>>(), [Probe::LldpHeard]);
    // Called by its LLDP system name until its station name is heard, then by that.
    let mut named = Fusion::new();
    let mut sys = pn_lldp("io-device-1", "port-001", 20);
    sys.sys_name = Some("plc-hall-2".into());
    apply(&mut named, heard(lldp::frame(PORT, &sys).unwrap(), "eth1", 1, 0));
    assert_eq!(named.devices[&DEV].label(), "plc-hall-2");
    apply(&mut named, heard(dcp::hello(DEV, 1, &identity("io-device-1", 10)).unwrap(), "eth1", 2, 1));
    assert_eq!(named.devices[&DEV].label(), "io-device-1");
    // Heard on a second interface too; then leaving (TTL 0), an older frame not bringing it back,
    // a later one doing so.
    apply(&mut f, heard(lldp::frame(PORT, &pdu).unwrap(), "eth2", 150, 1));
    // (A shutdown PDU holds the mandatory three alone, no chassis MAC: it is the device whose port
    // its source already is, by that MAC.)
    let leaving = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001.io-device-1".into() }, 0);
    apply(&mut f, heard(lldp::frame(PORT, &leaving).unwrap(), "eth1", 200, 2));
    assert_eq!(f.devices.len(), 1, "{:?}", f.devices.keys().collect::<Vec<_>>());
    assert_eq!(f.devices[&DEV].ports["port-001"].left_ns, Some(200));
    assert_eq!(f.port_owner(PORT), Some(DEV));
    apply(&mut f, heard(lldp::frame(PORT, &pdu).unwrap(), "eth1", 180, 3));
    assert_eq!(f.devices[&DEV].ports["port-001"].left_ns, Some(200));
    apply(&mut f, heard(lldp::frame(PORT, &pdu).unwrap(), "eth1", 250, 4));
    let p = &f.devices[&DEV].ports["port-001"];
    assert_eq!((p.left_ns, p.frames, p.heard_on.len()), (None, 5, 2));
    // A second port of the device, its short name apart from the station's suffix.
    apply(&mut f, heard(lldp::frame(Mac([0x02, 0, 0, 0, 0, 0x04]), &pn_lldp("io-device-1", "port-002", 20)).unwrap(), "eth1", 300, 5));
    assert_eq!(f.devices[&DEV].ports.keys().cloned().collect::<Vec<_>>(), ["port-001", "port-002"]);
    // A name is never a key: an LLDP PDU whose local chassis id is a station name known from DCP,
    // with no chassis MAC, is its port's MAC's, not that station's.
    apply(&mut f, heard(dcp::hello(DEV2, 1, &identity("io-device-2", 20)).unwrap(), "eth1", 400, 6));
    let nameless = LldpPdu::new(ChassisId::Local { id: "io-device-2".into() }, PortId::Local { id: "port-001.io-device-2".into() }, 20);
    let port2 = Mac([0x02, 0, 0, 0, 0, 0x05]);
    apply(&mut f, heard(lldp::frame(port2, &nameless).unwrap(), "eth1", 410, 7));
    assert!(f.devices[&DEV2].ports.is_empty());
    assert_eq!(f.devices[&port2].chassis.current().map(String::as_str), Some("local:io-device-2"));
    // A port MAC two devices claim belongs to neither: a chassis-less PDU from it is its own.
    let mut twice = Fusion::new();
    let mut other = pn_lldp("io-device-9", "port-001", 20);
    other.pn.chassis_mac = Some(DEV2);
    apply(&mut twice, heard(lldp::frame(PORT, &pn_lldp("io-device-1", "port-001", 20)).unwrap(), "eth1", 1, 0));
    apply(&mut twice, heard(lldp::frame(PORT, &other).unwrap(), "eth1", 2, 1));
    assert_eq!(twice.port_owner(PORT), None);
    apply(&mut twice, heard(lldp::frame(PORT, &leaving).unwrap(), "eth1", 3, 2));
    assert_eq!(twice.devices[&PORT].ports["port-001"].left_ns, Some(3));
}

#[test]
fn an_arp_sender_s_address_is_a_second_witness() {
    let mut f = Fusion::new();
    apply(&mut f, heard(dcp::hello(DEV, 1, &identity("io-device-1", 10)).unwrap(), "eth1", 100, 0));
    apply(&mut f, heard(observe::arp_frame(DEV, Ipv4Addr::new(192, 168, 0, 10), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap(), "eth1", 200, 1));
    let d = &f.devices[&DEV];
    assert_eq!(d.arp_ip.current(), Some(&Ipv4Addr::new(192, 168, 0, 10)));
    assert_eq!(d.heard.iter().copied().collect::<Vec<_>>(), [Probe::DcpHeard, Probe::ArpHeard]);
    assert_eq!(d.addresses().into_iter().collect::<Vec<_>>(), [Ipv4Addr::new(192, 168, 0, 10)]);
    assert!(f.conflicts().is_empty(), "one device's own two witnesses of one address agree");
    // An address of 0.0.0.0 is none.
    let mut unset = Fusion::new();
    apply(&mut unset, heard(dcp::hello(DEV, 1, &DeviceIdentity { ip: Some(IpSuite { ip: Ipv4Addr::UNSPECIFIED, ..suite(0) }), ..identity("a", 0) }).unwrap(), "eth1", 1, 0));
    apply(&mut unset, heard(dcp::hello(DEV2, 1, &DeviceIdentity { ip: Some(IpSuite { ip: Ipv4Addr::UNSPECIFIED, ..suite(0) }), ..identity("b", 0) }).unwrap(), "eth1", 2, 1));
    assert!(unset.devices[&DEV].addresses().is_empty());
    assert!(unset.conflicts().is_empty());
}

#[test]
fn every_conflict_is_on_record_once_ordered_by_kind_and_value() {
    let mut f = Fusion::new();
    // Two devices naming themselves alike; a third holding by ARP the address one holds by DCP; a
    // device flagging its own address conflict.
    let mut conflicted = identity("io-device-3", 30);
    conflicted.ip_status = Some(IpStatus { set: 1, conflict: true });
    let third = Mac([0x02, 0, 0, 0, 0, 0x07]);
    apply(&mut f, heard(dcp::hello(DEV, 1, &identity("io-device-1", 10)).unwrap(), "eth1", 100, 0));
    apply(&mut f, heard(dcp::hello(DEV2, 1, &identity("io-device-1", 20)).unwrap(), "eth1", 110, 1));
    apply(&mut f, heard(observe::arp_frame(third, Ipv4Addr::new(192, 168, 0, 10), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap(), "eth1", 120, 2));
    apply(&mut f, heard(dcp::hello(Mac([0x02, 0, 0, 0, 0, 0x09]), 1, &conflicted).unwrap(), "eth1", 130, 3));
    assert_eq!(
        f.conflicts(),
        [
            Conflict { kind: ConflictKind::DuplicateName, value: "io-device-1".into(), macs: vec![DEV, DEV2] },
            Conflict { kind: ConflictKind::DuplicateIp, value: "192.168.0.10".into(), macs: vec![DEV, third] },
            Conflict { kind: ConflictKind::ReportsIpConflict, value: "192.168.0.30".into(), macs: vec![Mac([0x02, 0, 0, 0, 0, 0x09])] },
        ]
    );
    // An empty name is no name to share.
    let mut blank = Fusion::new();
    apply(&mut blank, heard(dcp::hello(DEV, 1, &DeviceIdentity { name: Some(String::new()), ..identity("x", 41) }).unwrap(), "eth1", 1, 0));
    apply(&mut blank, heard(dcp::hello(DEV2, 1, &DeviceIdentity { name: Some(String::new()), ..identity("x", 42) }).unwrap(), "eth1", 2, 1));
    assert!(blank.conflicts().is_empty(), "{:?}", blank.conflicts());
    // The kinds' names as a scan names them.
    let names: Vec<String> = [ConflictKind::DuplicateName, ConflictKind::DuplicateIp, ConflictKind::NameChanged, ConflictKind::IpChanged, ConflictKind::ReportsIpConflict]
        .iter()
        .map(|k| serde_json::to_string(k).unwrap())
        .collect();
    assert_eq!(names, ["\"duplicate-name\"", "\"duplicate-ip\"", "\"name-changed\"", "\"ip-changed\"", "\"reports-ip-conflict\""]);
}

fn seen(probe: Probe, iface: &str, at_ns: u64, frame: u64) -> Witness {
    Witness { probe, iface: iface.into(), at_ns, frame }
}

fn dcp_at(mac: Mac, identity: DeviceIdentity, at_ns: u64, frame: u64) -> Observation {
    Observation { witness: seen(Probe::DcpHeard, "eth1", at_ns, frame), observed: Observed::Dcp { mac, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
}

fn lldp_at(src: Mac, pdu: &LldpPdu, iface: &str, at_ns: u64, frame: u64) -> Observation {
    Observation { witness: seen(Probe::LldpHeard, iface, at_ns, frame), observed: Observed::Lldp { src, pdu: pdu.clone() } }
}

fn arp_at(mac: Mac, ip: Ipv4Addr, at_ns: u64, frame: u64) -> Observation {
    Observation { witness: seen(Probe::ArpHeard, "eth1", at_ns, frame), observed: Observed::Arp { mac, ip } }
}

fn fused<'a>(obs: impl IntoIterator<Item = &'a Observation>) -> Fusion {
    let mut f = Fusion::new();
    for o in obs {
        f.apply(o);
    }
    f
}

fn ten(last: u8) -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, last)
}

fn mgmt(ip: Ipv4Addr) -> MgmtAddr {
    MgmtAddr { subtype: 1, addr: ip.octets().to_vec(), if_subtype: 2, if_number: 1, oid: vec![] }
}

fn shutdown(name: &str, port: &str) -> LldpPdu {
    LldpPdu::new(ChassisId::Local { id: name.into() }, PortId::Local { id: format!("{port}.{name}") }, 0)
}

fn at_address(name: &str, ip: Ipv4Addr) -> DeviceIdentity {
    DeviceIdentity { ip: Some(IpSuite { ip, ..suite(0) }), ..identity(name, 0) }
}

#[test]
fn what_is_fused_is_read_by_time_whatever_order_it_is_applied_in() {
    let mut rich = pn_lldp("io-device-1", "port-001", 20);
    rich.pn.delays = Some(Delays { rx_local: 1, rx_remote: 2, tx_local: 3, tx_remote: 4, cable_local: 5 });
    rich.pn.mrp = Some(MrpPortStatus { domain_uuid: [7; 16], mrrt: 2 });
    rich.mac_phy = Some(MacPhy { autoneg: 3, pmd_capabilities: 0x6C00, mau_type: 16 });
    rich.mgmt_addrs = vec![mgmt(ten(9))];
    let mut moved = rich.clone();
    moved.mgmt_addrs = vec![mgmt(ten(10))];
    let dev3 = Mac([0x02, 0, 0, 0, 0, 0x06]);
    let mut nameless = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001.io-device-1".into() }, 20);
    nameless.mgmt_addrs = vec![mgmt(ten(10))];
    let obs = vec![
        // The port's leaving heard before anything names its device: a port not yet known.
        lldp_at(PORT, &shutdown("io-device-1", "port-001"), "eth1", 50, 0),
        dcp_at(DEV, identity("a", 10), 100, 1),
        lldp_at(PORT, &rich, "eth2", 110, 2),
        dcp_at(DEV2, at_address("io-device-2", ten(9)), 120, 3),
        dcp_at(DEV, identity("b", 11), 200, 4),
        arp_at(DEV, Ipv4Addr::new(192, 168, 0, 11), 210, 5),
        lldp_at(PORT, &moved, "eth1", 220, 6),
        dcp_at(dev3, at_address("io-device-3", ten(10)), 230, 7),
        lldp_at(PORT, &shutdown("io-device-1", "port-001"), "eth1", 300, 8),
        // Then a PDU naming no MAC for its chassis that is no leaving, on a third interface,
        // with a management address.
        lldp_at(PORT, &nameless, "eth3", 320, 9),
    ];
    let by_time = fused(&obs);
    let d = &by_time.devices[&DEV];
    assert_eq!(by_time.devices.keys().copied().collect::<Vec<_>>(), [DEV, DEV2, dev3], "the port's MAC is no device");
    assert!(!d.by_source);
    assert_eq!(d.name.candidates.iter().map(|c| (c.value.as_str(), c.first.frame, c.count)).collect::<Vec<_>>(), [("a", 1, 1), ("b", 4, 1)]);
    assert_eq!((d.first_ns, d.last_ns, d.frames), (50, 320, 8));
    let three = BTreeSet::from(["eth1".to_string(), "eth2".to_string(), "eth3".to_string()]);
    assert_eq!(d.heard_on, three);
    assert_eq!(d.heard, BTreeSet::from([Probe::DcpHeard, Probe::LldpHeard, Probe::ArpHeard]));
    let p = &d.ports["port-001"];
    assert_eq!((p.first_ns, p.last_ns, p.frames, p.left_ns), (50, 320, 5, None));
    assert_eq!((p.leaving, p.spoke), (Some((300, 8)), Some((320, 9))));
    assert_eq!(p.heard_on, three);
    assert_eq!(p.delays.current(), rich.pn.delays.as_ref());
    assert_eq!(p.mrp.current(), rich.pn.mrp.as_ref());
    assert_eq!(p.mau_type.current(), Some(&16));
    assert_eq!(d.chassis.candidates.iter().map(|c| (c.value.as_str(), c.count)).collect::<Vec<_>>(), [("local:io-device-1", 5)]);
    // The management addresses a field like any: each PDU's, with its witnesses; the current one
    // the PDU heard last, the moved-from address no longer held.
    let m: Vec<_> = d.mgmt_ips.candidates.iter().map(|c| (c.value.iter().copied().collect::<Vec<_>>(), c.first.frame, c.last.frame, c.count)).collect();
    assert_eq!(m, [(vec![ten(9)], 2, 2, 1), (vec![ten(10)], 6, 9, 2)]);
    assert_eq!(d.addresses(), BTreeSet::from([ten(10), Ipv4Addr::new(192, 168, 0, 11)]));
    assert_eq!(
        by_time.conflicts(),
        [
            Conflict { kind: ConflictKind::DuplicateIp, value: "10.0.0.10".into(), macs: vec![DEV, dev3] },
            Conflict { kind: ConflictKind::NameChanged, value: "a, b".into(), macs: vec![DEV] },
            Conflict { kind: ConflictKind::IpChanged, value: "192.168.0.10, 192.168.0.11".into(), macs: vec![DEV] },
        ]
    );
    // Reversed, and shuffled twenty ways: the same devices, to the witness.
    let mut orders = vec![obs.iter().rev().cloned().collect::<Vec<_>>()];
    let mut noise = common::Noise(141);
    for _ in 0..20 {
        let mut o = obs.clone();
        for i in (1..o.len()).rev() {
            o.swap(i, usize::from(noise.next()) % (i + 1));
        }
        orders.push(o);
    }
    for o in &orders {
        let order: Vec<u64> = o.iter().map(|x| x.witness.frame).collect();
        assert_eq!(fused(o), by_time, "applied in the order {order:?}");
    }
}

#[test]
fn a_port_s_leaving_is_its_last_frame_by_time() {
    let back = pn_lldp("io-device-1", "port-001", 20);
    let gone = shutdown("io-device-1", "port-001");
    let left = |obs: &[Observation]| fused(obs).devices[&DEV].ports["port-001"].left_ns;
    let first = lldp_at(PORT, &back, "eth1", 100, 0);
    // A leaving and a return, applied in both orders: the later by time decides.
    for (leave, ret, expect) in [((200, 1), (250, 2), None), ((200, 1), (150, 2), Some(200)), ((200, 5), (200, 6), None), ((200, 6), (200, 5), Some(200))] {
        let l = lldp_at(PORT, &gone, "eth1", leave.0, leave.1);
        let r = lldp_at(PORT, &back, "eth1", ret.0, ret.1);
        assert_eq!(left(&[first.clone(), l.clone(), r.clone()]), expect, "{leave:?} then {ret:?}");
        assert_eq!(left(&[first.clone(), r, l]), expect, "{ret:?} then {leave:?}");
    }
    // Two leavings, the later applied first: the later stands.
    assert_eq!(left(&[first.clone(), lldp_at(PORT, &gone, "eth1", 300, 1), lldp_at(PORT, &gone, "eth1", 200, 2)]), Some(300));
    // A leaving heard before the port's device is known is taken in by it, and the leavings after
    // go to it too: no device of the port's MAC.
    let early = fused(&[lldp_at(PORT, &gone, "eth1", 50, 0)]);
    assert_eq!((early.devices[&PORT].by_source, early.port_owner(PORT)), (true, None));
    let mut f = fused(&[lldp_at(PORT, &gone, "eth1", 50, 0), first.clone()]);
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV]);
    assert_eq!(f.devices[&DEV].ports["port-001"].left_ns, None);
    f.apply(&lldp_at(PORT, &gone, "eth1", 200, 2));
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV]);
    assert_eq!(f.devices[&DEV].ports["port-001"].left_ns, Some(200));
    // A port is its device's by any MAC its frames came from, not only the last.
    let moved_port = Mac([0x02, 0, 0, 0, 0, 0x08]);
    let f = fused(&[first.clone(), lldp_at(moved_port, &back, "eth1", 150, 1), lldp_at(PORT, &gone, "eth1", 200, 2)]);
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV]);
    assert_eq!(f.devices[&DEV].ports["port-001"].left_ns, Some(200));
    // A MAC two devices' ports share is no one's, and a PDU naming one of them later takes in
    // nothing: what that MAC said stays its own.
    let mut other = pn_lldp("io-device-9", "port-001", 20);
    other.pn.chassis_mac = Some(DEV2);
    let f = fused(&[first.clone(), lldp_at(PORT, &other, "eth1", 110, 1), lldp_at(PORT, &gone, "eth1", 200, 2), lldp_at(PORT, &back, "eth1", 300, 3)]);
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV, PORT, DEV2]);
    assert_eq!(f.devices[&PORT].ports["port-001"].left_ns, Some(200));
    // A MAC that speaks for itself (DCP) is a device, not a port waiting for its own: a PDU from
    // it naming another device takes in nothing of what it said.
    let f = fused(&[lldp_at(PORT, &gone, "eth1", 50, 0), dcp_at(PORT, identity("io-device-7", 70), 60, 1), lldp_at(PORT, &back, "eth1", 100, 2)]);
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV, PORT]);
    assert!(!f.devices[&PORT].by_source);
    assert_eq!((f.devices[&PORT].label(), f.devices[&PORT].ports["port-001"].left_ns), ("io-device-7".to_string(), Some(50)));
}

#[test]
fn conflicts_are_read_from_what_each_device_says_now() {
    let names = |f: &Fusion| f.conflicts().into_iter().map(|c| (c.kind, c.value)).collect::<Vec<_>>();
    // A device that moved holds its new address alone: no duplicate with the one that took its
    // old one, by DCP or by ARP.
    let dcp_moved = [dcp_at(DEV, identity("a", 10), 100, 0), dcp_at(DEV, identity("a", 11), 200, 1), dcp_at(DEV2, identity("b", 10), 300, 2)];
    assert_eq!(names(&fused(&dcp_moved)), [(ConflictKind::IpChanged, "192.168.0.10, 192.168.0.11".to_string())]);
    let arp_moved = [arp_at(DEV, Ipv4Addr::new(192, 168, 0, 10), 100, 0), arp_at(DEV, Ipv4Addr::new(192, 168, 0, 11), 200, 1), arp_at(DEV2, Ipv4Addr::new(192, 168, 0, 10), 300, 2)];
    assert_eq!(names(&fused(&arp_moved)), []);
    // A device renamed from the name another now holds: only its renaming.
    let renamed = [dcp_at(DEV, identity("b", 10), 100, 0), dcp_at(DEV, identity("a", 10), 200, 1), dcp_at(DEV2, identity("b", 20), 300, 2)];
    assert_eq!(names(&fused(&renamed)), [(ConflictKind::NameChanged, "b, a".to_string())]);
    // ...its names in the order heard, though applied the other way.
    assert_eq!(names(&fused(renamed.iter().rev())), [(ConflictKind::NameChanged, "b, a".to_string())]);
    // A device that flagged its address in conflict and stopped: none; one that flags it on its
    // second address: that address.
    let flag = |last, conflict| DeviceIdentity { ip_status: Some(IpStatus { set: 1, conflict }), ..identity("a", last) };
    assert_eq!(names(&fused(&[dcp_at(DEV, flag(10, true), 100, 0), dcp_at(DEV, flag(10, false), 200, 1)])), []);
    assert_eq!(
        names(&fused(&[dcp_at(DEV, flag(10, false), 100, 0), dcp_at(DEV, flag(11, true), 200, 1)])),
        [(ConflictKind::IpChanged, "192.168.0.10, 192.168.0.11".to_string()), (ConflictKind::ReportsIpConflict, "192.168.0.11".to_string())]
    );
    // One address held two ways is no change of address: a mask, a gateway; nor is 0.0.0.0, nor
    // an empty name a change of name.
    let wide = DeviceIdentity { ip: Some(IpSuite { mask: Ipv4Addr::new(255, 255, 0, 0), ..suite(10) }), ..identity("a", 10) };
    let routed = DeviceIdentity { ip: Some(IpSuite { gateway: Ipv4Addr::new(192, 168, 0, 1), ..suite(10) }), ..identity("a", 10) };
    assert_eq!(names(&fused(&[dcp_at(DEV, identity("a", 10), 100, 0), dcp_at(DEV, wide, 200, 1), dcp_at(DEV, routed, 300, 2)])), []);
    let unset = DeviceIdentity { name: Some(String::new()), ..at_address("x", Ipv4Addr::UNSPECIFIED) };
    assert_eq!(names(&fused(&[dcp_at(DEV, unset.clone(), 100, 0), dcp_at(DEV, identity("a", 10), 200, 1)])), []);
    assert_eq!(names(&fused(&[dcp_at(DEV, identity("a", 10), 100, 0), dcp_at(DEV, unset, 200, 1), dcp_at(DEV, identity("b", 10), 300, 2)])), [(ConflictKind::NameChanged, "a, b".to_string())]);
    // A management address is an address: held by another's DCP, a duplicate.
    let mut pdu = pn_lldp("io-device-1", "port-001", 20);
    pdu.mgmt_addrs = vec![mgmt(ten(9))];
    let f = fused(&[lldp_at(PORT, &pdu, "eth1", 100, 0), dcp_at(DEV2, at_address("b", ten(9)), 200, 1)]);
    assert_eq!(f.conflicts(), [Conflict { kind: ConflictKind::DuplicateIp, value: "10.0.0.9".into(), macs: vec![DEV, DEV2] }]);
}

#[test]
fn an_lldp_pdu_naming_its_source_for_its_chassis_is_its_source_s_device() {
    // PORT a port of DEV by DEV's PDU; then a PDU from PORT naming PORT itself as its chassis, by
    // PROFINET's chassis MAC and by a chassis id of subtype MAC, or by the latter alone.
    let mut own = LldpPdu::new(ChassisId::Mac { mac: PORT }, PortId::Local { id: "port-009".into() }, 20);
    own.pn.chassis_mac = Some(PORT);
    let mut bare = own.clone();
    bare.pn.chassis_mac = None;
    for pdu in [own, bare] {
        let f = fused(&[lldp_at(PORT, &pn_lldp("io-device-1", "port-001", 20), "eth1", 100, 0), lldp_at(PORT, &pdu, "eth1", 200, 1)]);
        assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV, PORT]);
        assert_eq!(f.devices[&DEV].ports.keys().cloned().collect::<Vec<_>>(), ["port-001"]);
        assert_eq!(f.devices[&DEV].chassis.candidates.len(), 1);
        assert_eq!(f.devices[&PORT].ports.keys().cloned().collect::<Vec<_>>(), ["port-009"]);
        assert_eq!(f.devices[&PORT].chassis.current().map(String::as_str), Some("mac:02:00:00:00:00:02"));
        assert!(!f.devices[&PORT].by_source);
    }
}

#[test]
fn an_empty_station_name_is_no_label() {
    let mut sys = pn_lldp("io-device-1", "port-001", 20);
    sys.sys_name = Some("plc-hall-2".into());
    let blank = DeviceIdentity { name: Some(String::new()), ..identity("x", 10) };
    let f = fused(&[lldp_at(PORT, &sys, "eth1", 100, 0), dcp_at(DEV, blank.clone(), 200, 1)]);
    assert_eq!(f.devices[&DEV].label(), "plc-hall-2");
    assert_eq!(fused(&[dcp_at(DEV, blank, 100, 0)]).devices[&DEV].label(), DEV.to_string());
}
