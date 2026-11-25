//! The topology (contract: the topology clause): devices, this machine's interfaces and the chassis
//! neighbours name as nodes; a heard link only where one port is heard on an interface; reported
//! and confirmed links from the devices' own words on their neighbours, found by MAC, else by the
//! chassis a device announces, never by a name; MRP rings by their domain, closed when their
//! links make one cycle; components; a note for what is not drawn.

mod common;

use veredas_aranha::fusion::Fusion;
use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity};
use veredas_aranha::profinet::lldp::{ChassisId, LldpPdu, MrpPortStatus, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::topology::{self, Confidence, End, Interface, Neighbour, NodeKind, Topology};

const A: Mac = Mac([0x02, 0, 0, 0, 0, 0x0A]);
const B: Mac = Mac([0x02, 0, 0, 0, 0, 0x0B]);
const C: Mac = Mac([0x02, 0, 0, 0, 0, 0x0C]);
const ETH1: Mac = Mac([0x02, 0, 0, 0, 0, 0xE1]);

/// A port's MAC: the device's last byte and the port's number.
fn port_mac(dev: Mac, port: u8) -> Mac {
    Mac([0x02, 0, 0, 0, dev.0[5], port])
}

/// A PROFINET device's LLDP from one of its ports, its chassis its station name and its MAC.
#[allow(clippy::too_many_arguments)]
fn lldp(dev: Mac, name: &str, port: u8, iface: &str, at: u64, frame: u64, ttl: u16, mrp: Option<[u8; 16]>) -> Observation {
    let mut pdu = LldpPdu::new(ChassisId::Local { id: name.into() }, PortId::Local { id: format!("port-{port:03}.{name}") }, ttl);
    pdu.pn.chassis_mac = Some(dev);
    pdu.sys_name = Some(name.into());
    pdu.pn.mrp = mrp.map(|domain_uuid| MrpPortStatus { domain_uuid, mrrt: 0 });
    Observation { witness: Witness { probe: Probe::LldpHeard, iface: iface.into(), at_ns: at, frame }, observed: Observed::Lldp { src: port_mac(dev, port), pdu } }
}

fn hello(dev: Mac, name: &str, frame: u64) -> Observation {
    let identity = DeviceIdentity { name: Some(name.into()), ..Default::default() };
    Observation { witness: Witness { probe: Probe::DcpHeard, iface: "eth9".into(), at_ns: frame, frame }, observed: Observed::Dcp { mac: dev, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
}

fn fused(obs: &[Observation]) -> Fusion {
    let mut f = Fusion::new();
    for o in obs {
        f.apply(o);
    }
    f
}

fn said(device: Mac, port: u8, chassis: &str, peer_port: u8, peer_mac: Option<Mac>) -> Neighbour {
    Neighbour { device, port: format!("port-{port:03}"), chassis: chassis.into(), peer_port: format!("port-{peer_port:03}"), peer_mac, witnesses: 1 }
}

fn end(mac: Mac, port: u8) -> End {
    End { node: topology::device_id(mac), port: format!("port-{port:03}") }
}

fn links(t: &Topology) -> Vec<(End, End, Confidence, u64)> {
    t.links.iter().map(|l| (l.a.clone(), l.b.clone(), l.confidence, l.witnesses)).collect()
}

/// A link between these two ends, whichever is the lesser, as the topology stores it.
fn link(x: End, y: End, c: Confidence, w: u64) -> (End, End, Confidence, u64) {
    if x <= y { (x, y, c, w) } else { (y, x, c, w) }
}

type NodeRow = (String, NodeKind, String, Option<Mac>, Vec<String>);

#[test]
fn one_port_heard_on_an_interface_is_its_cable_and_several_are_none() {
    // A heard on eth1 three times; B and C both on eth2; C's other port on eth3, then leaving.
    let f = fused(&[
        lldp(A, "dev-a", 1, "eth1", 1, 0, 20, None),
        lldp(A, "dev-a", 1, "eth1", 2, 1, 20, None),
        lldp(A, "dev-a", 1, "eth1", 3, 2, 20, None),
        lldp(B, "dev-b", 1, "eth2", 4, 3, 20, None),
        lldp(C, "dev-c", 1, "eth2", 5, 4, 20, None),
        lldp(C, "dev-c", 2, "eth3", 6, 5, 20, None),
        lldp(C, "dev-c", 2, "eth3", 7, 6, 0, None),
    ]);
    let t = topology::build(&f, &[Interface { name: "eth1".into(), mac: Some(ETH1) }, Interface { name: "eth4".into(), mac: None }], &[]);
    // Nodes: the devices by MAC with their labels and ports, then every interface named or heard.
    let nodes: Vec<NodeRow> = t.nodes.iter().map(|n| (n.id.clone(), n.kind, n.label.clone(), n.mac, n.ports.clone())).collect();
    let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        nodes,
        [
            (topology::device_id(A), NodeKind::Device, "dev-a".to_string(), Some(A), p(&["port-001"])),
            (topology::device_id(B), NodeKind::Device, "dev-b".to_string(), Some(B), p(&["port-001"])),
            (topology::device_id(C), NodeKind::Device, "dev-c".to_string(), Some(C), p(&["port-001", "port-002"])),
            ("if:eth1".into(), NodeKind::Interface, "eth1".into(), Some(ETH1), vec![]),
            ("if:eth2".into(), NodeKind::Interface, "eth2".into(), None, vec![]),
            ("if:eth3".into(), NodeKind::Interface, "eth3".into(), None, vec![]),
            ("if:eth4".into(), NodeKind::Interface, "eth4".into(), None, vec![]),
        ]
    );
    // One link: eth1's cable to A's port, witnessed by its three frames.
    assert_eq!(links(&t), [(end(A, 1), End { node: "if:eth1".into(), port: String::new() }, Confidence::Heard, 3)]);
    assert_eq!(
        t.notes,
        [
            "dev-c's port-002 said it was leaving: no cable is drawn to it".to_string(),
            format!("eth2 hears LLDP from 2 ports ({} port-001, {} port-001): a mirror port or a hub, no cable is drawn from it", topology::device_id(B), topology::device_id(C)),
        ]
    );
    // Components: A with eth1, then everyone alone, in order.
    assert_eq!(t.components[0], [topology::device_id(A), "if:eth1".to_string()]);
    assert_eq!(t.components.len(), 6);
    assert!(t.components[1..].iter().all(|c| c.len() == 1));
}

#[test]
fn a_device_s_word_on_its_neighbour_is_reported_and_both_words_confirmed() {
    let f = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, None), lldp(B, "dev-b", 1, "eth2", 2, 1, 20, None), lldp(C, "dev-c", 1, "eth3", 3, 2, 20, None)]);
    // A's port 2 names B's port 1 by its MAC; B names A back (its port id with the station's
    // suffix); C names A's port 3 by chassis alone, twice.
    let t = topology::build(
        &f,
        &[],
        &[
            said(A, 2, "dev-b", 1, Some(port_mac(B, 1))),
            Neighbour { peer_port: "port-002.dev-a".into(), ..said(B, 1, "dev-a", 2, Some(port_mac(A, 2))) },
            Neighbour { witnesses: 2, ..said(C, 2, "dev-a", 3, None) },
        ],
    );
    let ls = links(&t);
    assert!(ls.contains(&(end(A, 2), end(B, 1), Confidence::Confirmed, 2)), "{ls:?}");
    assert!(ls.contains(&(end(A, 3), end(C, 2), Confidence::Reported, 2)), "{ls:?}");
    assert_eq!(ls.iter().filter(|l| l.2 != Confidence::Heard).count(), 2);
    assert_eq!(t.components[0].len(), 6, "{:?}", t.components);
    // The order the neighbours come in changes nothing.
    let again = topology::build(
        &f,
        &[],
        &[
            Neighbour { witnesses: 2, ..said(C, 2, "dev-a", 3, None) },
            Neighbour { peer_port: "port-002.dev-a".into(), ..said(B, 1, "dev-a", 2, Some(port_mac(A, 2))) },
            said(A, 2, "dev-b", 1, Some(port_mac(B, 1))),
        ],
    );
    assert_eq!(again, t);
}

#[test]
fn a_neighbour_is_found_by_its_mac_or_announced_chassis_never_by_a_name() {
    // A announces chassis dev-a; B and C both announce dev-x; D is named dev-d by DCP alone.
    let d = Mac([0x02, 0, 0, 0, 0, 0x0D]);
    let f = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, None), lldp(B, "dev-x", 1, "eth2", 2, 1, 20, None), lldp(C, "dev-x", 1, "eth3", 3, 2, 20, None), hello(d, "dev-d", 3)]);
    let t = topology::build(
        &f,
        &[],
        &[
            // A's port 2: chassis dev-x, which two devices announce.
            said(A, 2, "dev-x", 1, None),
            // A's port 3: chassis dev-d, a name DCP gave, which no LLDP announces.
            said(A, 3, "dev-d", 1, None),
            // A's port 4: chassis dev-x again, with C's port MAC: the MAC decides.
            said(A, 4, "dev-x", 1, Some(port_mac(C, 1))),
            // A's port 5: an all-zero MAC is none, and an unknown chassis.
            said(A, 5, "dev-q", 7, Some(Mac([0; 6]))),
            // A's port 6: the MAC of a device itself (D's, from DCP).
            said(A, 6, "whatever", 1, Some(d)),
        ],
    );
    let ls = links(&t);
    let chassis = |k: &str, port: u8| End { node: topology::chassis_id(&format!("local:{k}")), port: format!("port-{port:03}") };
    assert!(ls.contains(&link(end(A, 2), chassis("dev-x", 1), Confidence::Reported, 1)), "{ls:?}");
    assert!(ls.contains(&link(end(A, 3), chassis("dev-d", 1), Confidence::Reported, 1)), "{ls:?}");
    assert!(ls.contains(&(end(A, 4), end(C, 1), Confidence::Reported, 1)), "{ls:?}");
    assert!(ls.contains(&link(end(A, 5), chassis("dev-q", 7), Confidence::Reported, 1)), "{ls:?}");
    assert!(ls.contains(&(end(A, 6), End { node: topology::device_id(d), port: "port-001".into() }, Confidence::Reported, 1)), "{ls:?}");
    let ch: Vec<(String, NodeKind, String)> = t.nodes.iter().filter(|n| n.kind == NodeKind::Chassis).map(|n| (n.id.clone(), n.kind, n.label.clone())).collect();
    assert_eq!(
        ch,
        [
            ("chassis:local:dev-d".to_string(), NodeKind::Chassis, "dev-d (not yet identified)".to_string()),
            ("chassis:local:dev-q".to_string(), NodeKind::Chassis, "dev-q (not yet identified)".to_string()),
            ("chassis:local:dev-x".to_string(), NodeKind::Chassis, "dev-x (ambiguous: 2 devices)".to_string()),
        ]
    );
    assert!(t.notes.contains(&"a neighbour names chassis \"dev-x\", which 2 devices announce: its links go to that chassis, not to a guess".to_string()), "{:?}", t.notes);
    // Links stored in order of their ends, the heard and the reported together.
    assert!(t.links.windows(2).all(|w| (&w[0].a, &w[0].b) < (&w[1].a, &w[1].b)), "{ls:?}");
    // An all-zero MAC is none even where a broken frame made a device of it.
    let zero = Mac([0; 6]);
    let mut broken = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, None)]);
    broken.apply(&Observation { witness: Witness { probe: Probe::DcpHeard, iface: "eth1".into(), at_ns: 2, frame: 1 }, observed: Observed::Dcp { mac: zero, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity: DeviceIdentity::default() } });
    let tz = topology::build(&broken, &[], &[said(A, 2, "dev-a", 1, Some(zero))]);
    assert!(links(&tz).contains(&(end(A, 1), end(A, 2), Confidence::Reported, 1)), "{:?}", links(&tz));
    // The MAC against a chassis another single device announces: the MAC is taken, and said.
    let t = topology::build(&f, &[], &[said(B, 2, "dev-a", 1, Some(port_mac(C, 1)))]);
    assert!(links(&t).contains(&(end(B, 2), end(C, 1), Confidence::Reported, 1)));
    assert!(t.notes.iter().any(|n| n.contains("names chassis \"dev-a\", which another device announces") && n.contains("the MAC is taken")), "{:?}", t.notes);
}

#[test]
fn an_mrp_ring_is_its_domain_s_members_closed_when_their_links_make_one_cycle() {
    let ring = Some([7; 16]);
    let other = Some([9; 16]);
    let f = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, ring), lldp(B, "dev-b", 1, "eth2", 2, 1, 20, ring), lldp(C, "dev-c", 1, "eth3", 3, 2, 20, ring)]);
    let triangle = [said(A, 2, "dev-b", 3, Some(port_mac(B, 3))), said(B, 2, "dev-c", 3, Some(port_mac(C, 3))), said(C, 2, "dev-a", 3, Some(port_mac(A, 3)))];
    let t = topology::build(&f, &[], &triangle);
    assert_eq!(t.rings.len(), 1);
    let r = &t.rings[0];
    assert_eq!((r.domain.as_str(), r.closed), ("07".repeat(16).as_str(), true));
    assert_eq!(r.members, [A, B, C].map(topology::device_id));
    assert!(t.nodes.iter().filter(|n| n.kind == NodeKind::Device).all(|n| n.rings == ["07".repeat(16)]));
    // A link missing: open; MRRT off (0) is a member all the same.
    let t = topology::build(&f, &[], &triangle[..2]);
    assert!(!t.rings[0].closed);
    // Six members as two triangles: every member of degree two and as many links as members, but
    // not one cycle.
    let six: Vec<Mac> = (0x21..0x27).map(|b| Mac([0x02, 0, 0, 0, 0, b])).collect();
    let f6 = fused(&six.iter().enumerate().map(|(i, m)| lldp(*m, &format!("dev-{i}"), 1, &format!("eth{i}"), i as u64, i as u64, 20, ring)).collect::<Vec<_>>());
    let mut two = Vec::new();
    for (x, y) in [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)] {
        two.push(said(six[x], 2, &format!("dev-{y}"), 3, None));
    }
    let t = topology::build(&f6, &[], &two);
    assert_eq!(t.links.iter().filter(|l| l.confidence == Confidence::Reported).count(), 6);
    assert_eq!((t.rings[0].members.len(), t.rings[0].closed), (6, false));
    // A triangle with a tail: as many links as members, joined, but not one cycle.
    let d = Mac([0x02, 0, 0, 0, 0, 0x0D]);
    let f4 = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, ring), lldp(B, "dev-b", 1, "eth2", 2, 1, 20, ring), lldp(C, "dev-c", 1, "eth3", 3, 2, 20, ring), lldp(d, "dev-d", 1, "eth4", 4, 3, 20, ring)]);
    let mut tail = triangle.to_vec();
    tail.push(said(d, 2, "dev-a", 4, None));
    let t = topology::build(&f4, &[], &tail);
    assert_eq!((t.rings[0].members.len(), t.rings[0].closed), (4, false));
    // Components largest first: a device alone before nothing larger.
    let lone = fused(&[hello(A, "dev-a", 0), lldp(B, "dev-b", 1, "eth1", 1, 1, 20, None)]);
    let t = topology::build(&lone, &[], &[]);
    assert_eq!(t.components, [vec![topology::device_id(B), "if:eth1".to_string()], vec![topology::device_id(A)]]);
    // Two members are no ring to close; another domain is another ring.
    let f2 = fused(&[lldp(A, "dev-a", 1, "eth1", 1, 0, 20, ring), lldp(B, "dev-b", 1, "eth2", 2, 1, 20, ring), lldp(C, "dev-c", 1, "eth3", 3, 2, 20, other)]);
    let t = topology::build(&f2, &[], &[said(A, 2, "dev-b", 3, Some(port_mac(B, 3))), said(B, 2, "dev-a", 3, Some(port_mac(A, 3)))]);
    assert_eq!(t.rings.iter().map(|r| (r.members.len(), r.closed)).collect::<Vec<_>>(), [(2, false), (1, false)]);
}
