//! Observations into devices (contract: the fusion clause): the MAC the identity, every value kept
//! with its witnesses, the value heard last current, ports and their leaving, every conflict on
//! record, and the same observations giving the same devices.

use std::net::Ipv4Addr;

use veredas_aranha::fusion::{Conflict, ConflictKind, Fusion};
use veredas_aranha::observe::{self, Observation, Observed, Probe, Witness};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, Filter, IpStatus, IpSuite};
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, MgmtAddr, PortId, PortStatus};
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
    assert_eq!(d.mgmt_ips.iter().copied().collect::<Vec<_>>(), [Ipv4Addr::new(10, 0, 0, 9)]);
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
