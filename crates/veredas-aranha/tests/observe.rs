//! What a frame says (contract: the observations clause): each frame the listener keeps into the
//! observations it holds about one device each, with their witness, its class and its damage named;
//! every frame written by the codecs, tagged and untagged, and any bytes without a panic.

mod common;

use std::net::Ipv4Addr;

use veredas_aranha::observe::{self, Class, Observed, Probe, Witness};
use veredas_aranha::profinet::alarm::{AlarmSpecifier, Notification, Priority, RtaPdu};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, Filter, IpStatus, IpSuite, Service, ServiceType};
use veredas_aranha::profinet::ether::{self, VlanTag};
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::rt::{self, CyclicFrame, DataStatus};
use veredas_aranha::wire::hub::Hub;
use veredas_aranha::wire::pcap::{self, Replay};
use veredas_aranha::wire::{Frame, Wire};

const DEV: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
const PORT: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
const ME: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xAA]);

fn identity() -> DeviceIdentity {
    DeviceIdentity {
        name: Some("io-device-1".into()),
        ip: Some(IpSuite { ip: Ipv4Addr::new(192, 168, 0, 10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
        ip_status: Some(IpStatus { set: 1, conflict: false }),
        vendor_id: Some(0x1234),
        device_id: Some(0x0203),
        vendor: Some("bench device".into()),
        role: Some(1),
        mac: Some(DEV),
        ..Default::default()
    }
}

fn frame(bytes: Vec<u8>) -> Frame {
    Frame::new(1_700_000_000_123_456_789, "eth1", bytes)
}

fn witness(probe: Probe, index: u64) -> Witness {
    Witness { probe, iface: "eth1".into(), at_ns: 1_700_000_000_123_456_789, frame: index }
}

fn tagged(untagged: &[u8], tag: VlanTag) -> Vec<u8> {
    let e = ether::parse(untagged).unwrap();
    let payload = e.payload.to_vec();
    ether::build_tagged(e.dst, e.src, Some(tag), e.ethertype, &payload).unwrap()
}

#[test]
fn a_device_s_dcp_answer_and_hello_say_who_it_is_and_a_request_who_asks() {
    let id = identity();
    let answer = dcp::identify_response(ME, DEV, 7, &id).unwrap();
    let d = observe::decode(&frame(answer.clone()), 41);
    assert_eq!((d.class, d.damage.clone()), (Class::Dcp, vec![]));
    assert_eq!(d.observations.len(), 1);
    let o = &d.observations[0];
    assert_eq!(o.witness, witness(Probe::DcpHeard, 41));
    match &o.observed {
        Observed::Dcp { mac, service, service_type, identity } => {
            assert_eq!((*mac, *service, *service_type), (DEV, Service::Identify, ServiceType::Success));
            assert_eq!(*identity, DeviceIdentity::from_blocks(&dcp::decode(ether::parse(&answer).unwrap().payload).unwrap().blocks));
            assert_eq!((identity.name.as_deref(), identity.vendor_id, identity.ip.as_ref().map(|s| s.ip)), (Some("io-device-1"), Some(0x1234), Some(Ipv4Addr::new(192, 168, 0, 10))));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(o.observed.subject(), DEV);
    // Tagged (priority 6, VLAN 0, and VLAN 100), the same observation.
    for tag in [VlanTag { priority: 6, drop_eligible: false, id: 0 }, VlanTag { priority: 0, drop_eligible: false, id: 100 }] {
        assert_eq!(observe::decode(&frame(tagged(&answer, tag)), 41).observations, d.observations);
    }
    // A Hello describes the device too.
    let hello = observe::decode(&frame(dcp::hello(DEV, 3, &id).unwrap()), 2);
    assert_eq!(hello.observations[0].witness.probe, Probe::DcpHeard);
    assert!(matches!(&hello.observations[0].observed, Observed::Dcp { mac, service: Service::Hello, identity, .. } if *mac == DEV && identity.name.as_deref() == Some("io-device-1")));
    // Requests say who asks, and nothing of a device: Identify, Get, Set.
    let requests = [
        dcp::identify_request(ME, 1, 1, &Filter::All).unwrap(),
        dcp::identify_request(ME, 1, 1, &Filter::Name("io-device-1".into())).unwrap(),
        dcp::get_request(DEV, ME, 2, &[(2, 2)]).unwrap(),
        dcp::set_name_request(DEV, ME, 3, "io-device-2", true).unwrap(),
    ];
    for (i, r) in requests.iter().enumerate() {
        let d = observe::decode(&frame(r.clone()), i as u64);
        assert_eq!((d.class, d.damage.clone(), d.observations.len()), (Class::Dcp, vec![], 1), "{i}");
        assert_eq!(d.observations[0].witness.probe, Probe::DcpRequestHeard, "{i}");
        match &d.observations[0].observed {
            Observed::Dcp { mac, service_type: ServiceType::Request, identity, .. } => assert_eq!((*mac, identity.is_empty()), (ME, true), "{i}"),
            other => panic!("{i}: {other:?}"),
        }
    }
    // A Set answer is an answer: heard, its identity empty.
    let set_answer = observe::decode(&frame(dcp::set_response(ME, DEV, 3, &[(2, 2, 0)]).unwrap()), 9);
    assert_eq!(set_answer.observations[0].witness.probe, Probe::DcpHeard);
}

#[test]
fn what_a_dcp_frame_holds_damaged_is_named_and_a_group_source_says_nothing() {
    // A block that does not fit its kind: the rest observed, the damage named.
    let name = [2u8, 2, 0, 6, 0, 0, b'a', b'b', b'c', b'd'];
    let bad_ip = [1u8, 2, 0, 5, 0, 0, 10, 0, 0];
    let mut payload = vec![0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0];
    let blocks = [&name[..], &bad_ip[..], &[0]].concat();
    payload.extend_from_slice(&(blocks.len() as u16).to_be_bytes());
    payload.extend_from_slice(&blocks);
    let f = ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &payload).unwrap();
    let d = observe::decode(&frame(f), 0);
    assert_eq!(d.damage, ["DCP block 1/2: Length { expected: 12, got: 3 }"]);
    assert!(matches!(&d.observations[0].observed, Observed::Dcp { identity, .. } if identity.name.as_deref() == Some("abcd") && identity.ip.is_none()));
    // A PDU that is no PDU: damage, no observation.
    let mut over = payload.clone();
    over[11] = 0xFF;
    let d = observe::decode(&frame(ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &over).unwrap()), 0);
    assert_eq!((d.class, d.observations.len()), (Class::Dcp, 0));
    assert_eq!(d.damage.len(), 1);
    assert!(d.damage[0].starts_with("DCP: DCPDataLength 255 runs past"), "{:?}", d.damage);
    // From a group address: damage, nothing said of a device.
    let mut group = dcp::identify_response(ME, DEV, 7, &identity()).unwrap();
    group[6] = 0x03;
    let d = observe::decode(&frame(group), 0);
    assert_eq!((d.observations.len(), d.damage.clone()), (0, vec!["DCP from a group address, 03:00:00:00:00:01".to_string()]));
}

#[test]
fn an_lldp_frame_says_what_port_and_chassis_it_comes_from() {
    let mut pn = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001.io-device-1".into() }, 20);
    pn.pn.chassis_mac = Some(DEV);
    let switch = LldpPdu::new(ChassisId::Mac { mac: Mac([0x02, 0x1C, 0, 0, 0, 1]) }, PortId::IfName { name: "eth3".into() }, 120);
    let bare = LldpPdu::new(ChassisId::Local { id: "x".into() }, PortId::Local { id: "p".into() }, 0);
    // Both a chassis MAC TLV and a MAC chassis id, differing: the TLV names the device.
    let mut both = switch.clone();
    both.pn.chassis_mac = Some(DEV);
    let cases = [(pn.clone(), DEV), (switch.clone(), Mac([0x02, 0x1C, 0, 0, 0, 1])), (bare.clone(), PORT), (both, DEV)];
    for (i, (pdu, subject)) in cases.iter().enumerate() {
        let f = lldp::frame(PORT, pdu).unwrap();
        let d = observe::decode(&frame(f.clone()), i as u64);
        assert_eq!((d.class, d.damage.clone()), (Class::Lldp, vec![]), "{i}");
        assert_eq!(d.observations, [observe::Observation { witness: witness(Probe::LldpHeard, i as u64), observed: Observed::Lldp { src: PORT, pdu: lldp::decode(ether::parse(&f).unwrap().payload).unwrap() } }], "{i}");
        // The device: the chassis MAC TLV, else a MAC chassis id, else the port's MAC.
        assert_eq!(d.observations[0].observed.subject(), *subject, "{i}");
    }
    // A PDU that is no PDU (its chassis id unfit in its place): damage, no observation.
    let mut broken = lldp::frame(PORT, &pn).unwrap();
    broken[16] = 0x04;
    let d = observe::decode(&frame(broken), 0);
    assert_eq!((d.class, d.observations.len()), (Class::Lldp, 0));
    assert!(d.damage[0].starts_with("LLDP: "), "{:?}", d.damage);
    let mut group = lldp::frame(PORT, &pn).unwrap();
    group[6] = 0x01;
    let d = observe::decode(&frame(group), 0);
    assert_eq!((d.observations.len(), d.damage.clone()), (0, vec!["LLDP from a group address, 01:00:00:00:00:02".to_string()]));
}

#[test]
fn an_arp_sender_claims_its_address_and_a_probe_claims_none() {
    let ip = Ipv4Addr::new(192, 168, 0, 10);
    let other = Ipv4Addr::new(192, 168, 0, 1);
    for (f, reply) in [(observe::arp_frame(DEV, ip, Mac::BROADCAST, other, false).unwrap(), false), (observe::arp_frame(DEV, ip, ME, other, true).unwrap(), true)] {
        let d = observe::decode(&frame(f.clone()), 5);
        assert_eq!((d.class, d.damage.clone()), (Class::Arp, vec![]), "{reply}");
        assert_eq!(d.observations, [observe::Observation { witness: witness(Probe::ArpHeard, 5), observed: Observed::Arp { mac: DEV, ip } }]);
        // To the byte: Ethernet, IPv4, 6 and 4, the operation, sender and target.
        let p = ether::parse(&f).unwrap().payload.to_vec();
        assert_eq!(&p[..8], &[0, 1, 8, 0, 6, 4, 0, if reply { 2 } else { 1 }]);
        assert_eq!((&p[8..14], &p[14..18], &p[24..28]), (&DEV.0[..], &ip.octets()[..], &other.octets()[..]));
        assert_eq!(&p[18..24], if reply { &ME.0[..] } else { &[0; 6][..] });
    }
    // A probe (sender 0.0.0.0, RFC 5227) claims nothing, and is no damage; a gratuitous one claims.
    let probe = observe::decode(&frame(observe::arp_frame(DEV, Ipv4Addr::UNSPECIFIED, Mac::BROADCAST, ip, false).unwrap()), 0);
    assert_eq!((probe.observations.len(), probe.damage.len()), (0, 0));
    let gratuitous = observe::decode(&frame(observe::arp_frame(DEV, ip, Mac::BROADCAST, ip, false).unwrap()), 0);
    assert_eq!(gratuitous.observations.len(), 1);
    // Damage named: short, another hardware or protocol, an operation that is none of ARP's, a group
    // sender.
    let good = ether::parse(&observe::arp_frame(DEV, ip, Mac::BROADCAST, other, false).unwrap()).unwrap().payload[..28].to_vec();
    let with = |at: usize, v: u8| {
        let mut p = good.clone();
        p[at] = v;
        observe::decode(&frame(ether::build(Mac::BROADCAST, DEV, ether::ETHERTYPE_ARP, &p).unwrap()), 0)
    };
    let short = observe::decode(&frame(ether::build(Mac::BROADCAST, DEV, ether::ETHERTYPE_ARP, &good[..27]).unwrap()), 0);
    // (A frame of 27 bytes of payload is padded to 46 on the wire: the pad is read as the rest.)
    assert_eq!(short.damage, Vec::<String>::new());
    let cut = ether::build(Mac::BROADCAST, DEV, ether::ETHERTYPE_ARP, &good).unwrap()[..14 + 27].to_vec();
    assert_eq!(observe::decode(&frame(cut), 0).damage, ["an ARP packet of Ethernet and IPv4 is 28 bytes; this one is 27"]);
    assert_eq!(with(1, 6).damage, ["an ARP packet of hardware type 6, protocol 0x0800, address lengths 6 and 4: not Ethernet's and IPv4's"]);
    assert_eq!(with(5, 16).damage, ["an ARP packet of hardware type 1, protocol 0x0800, address lengths 6 and 16: not Ethernet's and IPv4's"]);
    assert_eq!(with(7, 3).damage, ["ARP operation 3 is neither a request (1) nor a reply (2)"]);
    assert_eq!(with(8, 0x03).damage, ["ARP from a group address, 03:00:00:00:00:01"]);
    for d in [with(1, 6), with(7, 3), with(8, 0x03)] {
        assert_eq!((d.class, d.observations.len()), (Class::Arp, 0));
    }
}

#[test]
fn every_other_frame_is_classed_and_counted_never_observed() {
    let cyclic = CyclicFrame { frame_id: 0x8001, data: vec![0; 40], cycle_counter: 1, data_status: DataStatus::GOOD, transfer_status: 0 };
    let notification = Notification { alarm_type: 1, api: 0, slot: 1, subslot: 1, module_ident: 1, submodule_ident: 1, specifier: AlarmSpecifier(0x0001), item: None };
    let cases: Vec<(Vec<u8>, Class)> = vec![
        (cyclic.frame(DEV, ME, Some(rt::CYCLIC_TAG)).unwrap(), Class::Cyclic),
        (CyclicFrame { frame_id: 0x0100, ..cyclic.clone() }.frame(DEV, ME, None).unwrap(), Class::Cyclic),
        (CyclicFrame { frame_id: 0xC000, ..cyclic.clone() }.frame(DEV, ME, None).unwrap(), Class::Cyclic),
        (RtaPdu::notification(Priority::High, 1, 2, 3, 4, notification.clone()).frame(ME, DEV, Some(Priority::High.tag())).unwrap(), Class::Alarm),
        (RtaPdu::notification(Priority::Low, 1, 2, 3, 4, notification).frame(ME, DEV, None).unwrap(), Class::Alarm),
        (ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &[0xFE, 0x02, 0, 0]).unwrap(), Class::ProfinetOther),
        (ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &[0xFC, 0x41]).unwrap(), Class::ProfinetOther),
        (ether::build(ME, DEV, 0x0800, &[0x45]).unwrap(), Class::Ipv4),
        (ether::build(ME, DEV, 0x86DD, &[0x60]).unwrap(), Class::Ipv6),
        (ether::build(Mac::BROADCAST, DEV, 0x88A4, &[0; 10]).unwrap(), Class::Ethercat),
        (ether::build(Mac::BROADCAST, DEV, 0x88AB, &[0; 10]).unwrap(), Class::Powerlink),
        (ether::build(Mac::BROADCAST, DEV, 0x88E3, &[0; 10]).unwrap(), Class::Mrp),
        (ether::build(Mac::BROADCAST, DEV, 0x88F7, &[0; 10]).unwrap(), Class::Ptp),
        (ether::build(Mac::BROADCAST, DEV, 0x1234, &[0; 10]).unwrap(), Class::Other(0x1234)),
    ];
    for (i, (f, class)) in cases.iter().enumerate() {
        let d = observe::decode(&frame(f.clone()), 0);
        assert_eq!((d.class, d.observations.len(), d.damage.len()), (*class, 0, 0), "{i}");
    }
    // An 802.3 length frame is classed by its length field.
    let mut llc = ether::build(Mac::BROADCAST, DEV, 0x0600, &[0; 10]).unwrap();
    llc[12..14].copy_from_slice(&0x0026u16.to_be_bytes());
    assert_eq!(observe::decode(&frame(llc), 0).class, Class::Other(0x0026));
    // A PROFINET frame too short for its id, and no Ethernet frame at all: damage named.
    let mut short = ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &[]).unwrap();
    short.truncate(15);
    let d = observe::decode(&frame(short), 0);
    assert_eq!((d.class, d.damage), (Class::ProfinetOther, vec!["a PROFINET frame too short for its frame id".to_string()]));
    let d = observe::decode(&frame(vec![0; 13]), 0);
    assert_eq!((d.class, d.damage), (Class::Malformed, vec!["an Ethernet frame is at least 14 bytes (18 with a VLAN tag); this one is 13".to_string()]));
    // Their names in a count, both ways.
    let names = [
        (Class::Dcp, "dcp"),
        (Class::Cyclic, "cyclic"),
        (Class::Alarm, "alarm"),
        (Class::ProfinetOther, "profinet-other"),
        (Class::Lldp, "lldp"),
        (Class::Arp, "arp"),
        (Class::Ipv4, "ipv4"),
        (Class::Ipv6, "ipv6"),
        (Class::Ethercat, "ethercat"),
        (Class::Powerlink, "powerlink"),
        (Class::Mrp, "mrp"),
        (Class::Ptp, "ptp"),
        (Class::Other(0x1234), "other:0x1234"),
        (Class::Other(0x0026), "other:0x0026"),
        (Class::Malformed, "malformed"),
    ];
    for (c, name) in names {
        assert_eq!(c.to_string(), name);
        assert_eq!(serde_json::to_string(&c).unwrap(), format!("\"{name}\""));
        assert_eq!(serde_json::from_str::<Class>(&format!("\"{name}\"")).unwrap(), c);
    }
    for bad in ["\"other:0x12\"", "\"other:1234\"", "\"Dcp\"", "\"other:0xzzzz\""] {
        assert!(serde_json::from_str::<Class>(bad).is_err(), "{bad}");
    }
}

#[test]
fn frames_through_the_seam_keep_their_witness_and_any_bytes_decode_without_a_panic() {
    // A capture replayed and a hub: each observation names the frame's interface, time and index.
    let frames = vec![
        Frame::new(1_000, "cap", dcp::hello(DEV, 1, &identity()).unwrap()),
        Frame::new(2_000, "cap", observe::arp_frame(DEV, Ipv4Addr::new(10, 0, 0, 1), Mac::BROADCAST, Ipv4Addr::new(10, 0, 0, 2), false).unwrap()),
    ];
    let bytes = pcap::write(&frames).unwrap();
    let mut replay = Replay::new("cap", pcap::read(&bytes, "eth7").unwrap().frames);
    let mut index = 0;
    while let Ok(Some(f)) = replay.recv(std::time::Duration::ZERO) {
        let d = observe::decode(&f, index);
        assert_eq!(d.observations[0].witness.iface, "eth7");
        assert_eq!(d.observations[0].witness.at_ns, frames[index as usize].at_ns);
        assert_eq!(d.observations[0].witness.frame, index);
        index += 1;
    }
    assert_eq!(index, 2);
    let hub = Hub::new();
    let mut dev = hub.port("dev0", DEV);
    let mut listener = hub.port("lan2", ME);
    hub.set_now(77);
    dev.send(&dcp::hello(DEV, 1, &identity()).unwrap()).unwrap();
    let heard = listener.recv(std::time::Duration::ZERO).unwrap().unwrap();
    assert_eq!(observe::decode(&heard, 9).observations[0].witness, Witness { probe: Probe::DcpHeard, iface: "lan2".into(), at_ns: 77, frame: 9 });

    // Noise: frames of every observed kind with a few bytes flipped and cut at random lengths, and a
    // quarter of it random bytes under seven types: a class, observations about unicast stations
    // only, damage named or none, never a panic.
    let mut pn = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001".into() }, 20);
    pn.pn.chassis_mac = Some(DEV);
    let seeds = [
        dcp::identify_response(ME, DEV, 7, &identity()).unwrap(),
        dcp::hello(DEV, 3, &identity()).unwrap(),
        dcp::identify_request(ME, 1, 1, &Filter::All).unwrap(),
        lldp::frame(PORT, &pn).unwrap(),
        observe::arp_frame(DEV, Ipv4Addr::new(10, 0, 0, 1), Mac::BROADCAST, Ipv4Addr::new(10, 0, 0, 2), false).unwrap(),
    ];
    let mut noise = common::Noise(0x0B5E);
    let types = [0x8892u16, 0x88CC, 0x0806, 0x0800, 0x8100, 0x88A4, 0x1234];
    let (mut observed, mut damaged) = (0, 0);
    for i in 0..30_000 {
        let f = if i % 4 == 0 {
            let len = usize::from(noise.next()) % 160;
            let mut f = noise.bytes(len);
            if f.len() >= 14 {
                let t = types[usize::from(noise.next()) % types.len()];
                f[12..14].copy_from_slice(&t.to_be_bytes());
            }
            f
        } else {
            let mut f = seeds[usize::from(noise.next()) % seeds.len()].clone();
            for _ in 0..noise.next() % 4 {
                let at = 14 + usize::from(noise.next()) % (f.len() - 14);
                f[at] = noise.next();
            }
            let keep = 14 + usize::from(noise.next()) % (f.len() - 13);
            f.truncate(keep.max(f.len() * usize::from(noise.next() % 2)));
            f
        };
        let d = observe::decode(&Frame::new(0, "n", f), 0);
        for o in &d.observations {
            assert!(!o.observed.subject().is_multicast(), "{o:?}");
        }
        observed += d.observations.len();
        damaged += usize::from(!d.damage.is_empty());
    }
    assert!(observed > 1_000 && damaged > 1_000, "{observed} observations, {damaged} damaged frames");
}

#[test]
fn only_what_dcp_defines_is_observed_and_a_request_is_known_by_its_selection_bit() {
    // Every service and service type byte, on a request's shape and an answer's: a service DCP does
    // not define, or a service type that is none of 0, 1 and 5, says nothing and is named; a request
    // (service type 0, Hello aside) is its asker with nothing said of it; the rest is heard.
    let request = ether::parse(&dcp::identify_request(ME, 1, 1, &Filter::Name("io-device-1".into())).unwrap()).unwrap().payload.to_vec();
    let answer = ether::parse(&dcp::identify_response(ME, DEV, 7, &identity()).unwrap()).unwrap().payload.to_vec();
    let mut heard_with_identity = 0;
    for shape in [&request, &answer] {
        for service in 0..=255u8 {
            for service_type in 0..=255u8 {
                let mut p = shape.clone();
                p[2] = service;
                p[3] = service_type;
                let d = observe::decode(&frame(ether::build(DEV, ME, ether::ETHERTYPE_PROFINET, &p).unwrap()), 0);
                assert_eq!(d.class, Class::Dcp);
                if !(3..=6).contains(&service) {
                    assert_eq!(d.observations.len(), 0, "{service} {service_type}");
                    assert_eq!(d.damage, [format!("DCP service {service} is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)")]);
                    continue;
                }
                if ![0, 1, 5].contains(&service_type) {
                    assert_eq!(d.observations.len(), 0, "{service} {service_type}");
                    assert_eq!(d.damage, [format!("DCP service type {service_type} is none of DCP's (0 request, 1 success, 5 unsupported)")]);
                    continue;
                }
                assert_eq!(d.observations.len(), 1, "{service} {service_type}: {:?}", d.damage);
                let asking = service_type == 0 && service != 6;
                let o = &d.observations[0];
                assert_eq!(o.witness.probe, if asking { Probe::DcpRequestHeard } else { Probe::DcpHeard }, "{service} {service_type}");
                if let Observed::Dcp { identity, .. } = &o.observed {
                    if asking {
                        assert!(identity.is_empty(), "{service} {service_type}");
                    } else if !identity.is_empty() {
                        heard_with_identity += 1;
                    }
                }
            }
        }
    }
    assert!(heard_with_identity > 0);
    // The critic's four: a name filter under service type 4 and a Set under 2 (requests DCP does not
    // define, tshark leaves undecoded), service 7, and a frame id with nothing after it but zeros.
    for (hex_frame, damage) in [
        ("010ecf0000000200000000aa8892fefe050400000001000100100202000b696f2d6465766963652d31", "DCP service type 4 is none of DCP's (0 request, 1 success, 5 unsupported)"),
        ("0200000000010200000000aa8892fefd040200000003000000120202000d0001696f2d6465766963652d32", "DCP service type 2 is none of DCP's (0 request, 1 success, 5 unsupported)"),
        ("010ecf0000000200000000aa8892fefe070100000001000100100202000b696f2d6465766963652d31", "DCP service 7 is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)"),
        ("010ecf0000000200000000aa8892fefe", "DCP service 0 is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)"),
    ] {
        let mut bytes: Vec<u8> = (0..hex_frame.len()).step_by(2).map(|i| u8::from_str_radix(&hex_frame[i..i + 2], 16).unwrap()).collect();
        bytes.resize(60, 0);
        let d = observe::decode(&frame(bytes), 0);
        assert_eq!((d.observations.len(), d.damage), (0, vec![damage.to_string()]), "{hex_frame}");
    }
    // Bytes after the last block are named; the PDU still says what its blocks say.
    let mut p = answer.clone();
    let n = u16::from_be_bytes([p[10], p[11]]) + 3;
    p[10..12].copy_from_slice(&n.to_be_bytes());
    p.extend_from_slice(&[1, 2, 3]);
    let d = observe::decode(&frame(ether::build(ME, DEV, ether::ETHERTYPE_PROFINET, &p).unwrap()), 0);
    assert_eq!(d.damage, ["DCP: 3 bytes after its last block make none"]);
    assert_eq!(d.observations.len(), 1);
}

#[test]
fn a_cut_frame_says_nothing_and_a_group_chassis_is_no_device() {
    // The critic's LLDP frame cut by its capture at 55 of 120 bytes, its chassis MAC TLV left out:
    // named, nothing observed, from bytes and through a capture file.
    let mut pn = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001.io-device-1".into() }, 20);
    pn.port_desc = Some("port description".into());
    pn.sys_name = Some("a-long-system-name-for-the-test".into());
    pn.pn.chassis_mac = Some(DEV);
    let whole = lldp::frame(PORT, &pn).unwrap();
    assert_eq!(observe::decode(&frame(whole.clone()), 0).observations[0].observed.subject(), DEV);
    for cut in [55usize, 56, 73, whole.len() - 1] {
        let f = Frame { length: whole.len(), ..frame(whole[..cut].to_vec()) };
        let d = observe::decode(&f, 0);
        assert_eq!((d.class, d.observations.len()), (Class::Lldp, 0), "{cut}");
        assert_eq!(d.damage, [format!("the capture kept {cut} of the frame's {} bytes", whole.len())]);
        let read = pcap::read(&pcap::write(&[f]).unwrap(), "x").unwrap();
        assert_eq!(observe::decode(&read.frames[0], 0).observations.len(), 0);
    }
    // A cut DCP frame and a cut cyclic frame keep their classes.
    let dcp_frame = dcp::hello(DEV, 1, &identity()).unwrap();
    assert_eq!(observe::decode(&Frame { length: 200, ..frame(dcp_frame) }, 0).class, Class::Dcp);
    let cyclic = CyclicFrame { frame_id: 0x8001, data: vec![0; 40], cycle_counter: 1, data_status: DataStatus::GOOD, transfer_status: 0 }.frame(DEV, ME, None).unwrap();
    assert_eq!(observe::decode(&Frame { length: 100, ..frame(cyclic[..40].to_vec()) }, 0).class, Class::Cyclic);
    // A group address named for the chassis (the chassis MAC TLV, or a MAC chassis id) is never one
    // device: named, nothing observed.
    let mut tlv = pn.clone();
    tlv.pn.chassis_mac = Some(Mac([0x01, 0x00, 0x5E, 0, 0, 1]));
    let by_id = LldpPdu::new(ChassisId::Mac { mac: Mac([0x01, 0x80, 0xC2, 0, 0, 0x0E]) }, PortId::Local { id: "p".into() }, 20);
    for (pdu, mac) in [(tlv, "01:00:5e:00:00:01"), (by_id, "01:80:c2:00:00:0e")] {
        let d = observe::decode(&frame(lldp::frame(PORT, &pdu).unwrap()), 0);
        assert_eq!((d.observations.len(), d.damage), (0, vec![format!("LLDP names a group address for its chassis, {mac}")]));
    }
    // An answer under two tags is classed by the second tag's type, and says nothing.
    let answer = dcp::identify_response(ME, DEV, 7, &identity()).unwrap();
    let inner = tagged(&answer, VlanTag { priority: 6, drop_eligible: false, id: 0 });
    let e = ether::parse(&inner).unwrap();
    let double = ether::build_tagged(e.dst, e.src, Some(VlanTag { priority: 0, drop_eligible: false, id: 5 }), 0x8100, &inner[14..]).unwrap_or_else(|_| {
        let mut f = inner[..12].to_vec();
        f.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
        f.extend_from_slice(&inner[12..]);
        f
    });
    let d = observe::decode(&frame(double), 0);
    assert_eq!((d.class, d.observations.len()), (Class::Other(0x8100), 0));
}

#[test]
fn every_arp_rule_and_every_class_name_is_held() {
    let ip = Ipv4Addr::new(192, 168, 0, 10);
    let good = observe::arp_frame(DEV, ip, Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap();
    // An ARP frame as a host captures its own, unpadded (42 bytes): observed.
    let unpadded = good[..42].to_vec();
    let d = observe::decode(&frame(unpadded), 0);
    assert_eq!((d.observations.len(), d.damage.len()), (1, 0));
    let with = |at: usize, v: u8| {
        let mut f = good.clone();
        f[14 + at] = v;
        observe::decode(&frame(f), 0)
    };
    assert_eq!(with(7, 0).damage, ["ARP operation 0 is neither a request (1) nor a reply (2)"]);
    let mut v6 = good.clone();
    v6[16..18].copy_from_slice(&0x86DDu16.to_be_bytes());
    assert_eq!(observe::decode(&frame(v6), 0).damage, ["an ARP packet of hardware type 1, protocol 0x86dd, address lengths 6 and 4: not Ethernet's and IPv4's"]);
    assert_eq!(with(4, 8).damage, ["an ARP packet of hardware type 1, protocol 0x0800, address lengths 8 and 4: not Ethernet's and IPv4's"]);
    for d in [with(7, 0), with(4, 8)] {
        assert_eq!(d.observations.len(), 0);
    }
    // A class name is a class's own name exactly; an unknown type's only when it has no class.
    for bad in ["\"other:0x+123\"", "\"other:0x12AB\"", "\"other:0x0800\"", "\"other:0x88cc\"", "\"other:0x8892\"", "\"other:0x86dd\""] {
        assert!(serde_json::from_str::<Class>(bad).is_err(), "{bad}");
    }
    assert_eq!(serde_json::from_str::<Class>("\"other:0x8100\"").unwrap(), Class::Other(0x8100));
}
