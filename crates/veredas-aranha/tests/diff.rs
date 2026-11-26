//! The diff of a scan against a baseline (contract: the diff clause): devices appeared and vanished
//! by their MAC, never by a name; what a device in both states of itself renamed or readdressed
//! where both scans state it, one-sided where one does; links added and removed by their ends;
//! conflicts new and cleared; interfaces listened on by one scan only; the same diff whatever order
//! the scans hold their lists in, and its mirror the other way round; proven on the bench's
//! devices, a baseline saved, the bench changed and scanned again.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use veredas_aranha::diff::{self, Change, DeviceRef, Diff, DiffError, OneSided, Said};
use veredas_aranha::fusion::{Conflict, ConflictKind, Fusion};
use veredas_aranha::listen::{Interface as Account, Stats};
use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
use veredas_aranha::profinet::lldp::{ChassisId, LldpPdu, MgmtAddr, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::scan::{Capture, Scan, ScanError};
use veredas_aranha::topology::{self, Confidence, End, Interface, Link, Neighbour};

const A: Mac = Mac([0x02, 0, 0, 0, 0, 0x0A]);
const B: Mac = Mac([0x02, 0, 0, 0, 0, 0x0B]);
const C: Mac = Mac([0x02, 0, 0, 0, 0, 0x0C]);
const D: Mac = Mac([0x02, 0, 0, 0, 0, 0x0D]);
const E: Mac = Mac([0x02, 0, 0, 0, 0, 0x0E]);
const T0: u64 = 1_764_000_000_000_000_000;

fn ip(last: u8) -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 0, last)
}

fn suite(address: Ipv4Addr) -> IpSuite {
    IpSuite { ip: address, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }
}

/// A suite of this prefix length and gateway.
fn suite_of(address: Ipv4Addr, prefix: u32, gateway: Ipv4Addr) -> IpSuite {
    IpSuite { ip: address, mask: Ipv4Addr::from(u32::MAX << (32 - prefix)), gateway }
}

fn witness(probe: Probe, iface: &str, frame: u64) -> Witness {
    Witness { probe, iface: iface.into(), at_ns: T0 + frame, frame }
}

/// A Hello: the name and the IP suite a device states.
fn hello(dev: Mac, name: Option<&str>, address: Option<IpSuite>, frame: u64) -> Observation {
    let identity = DeviceIdentity { name: name.map(String::from), ip: address, ..Default::default() };
    Observation { witness: witness(Probe::DcpHeard, "eth9", frame), observed: Observed::Dcp { mac: dev, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
}

/// A port's LLDP heard on `iface`: its chassis the device's MAC, its system name and management
/// addresses.
fn lldp(dev: Mac, port: u8, iface: &str, sys_name: Option<&str>, mgmt: &[Ipv4Addr], frame: u64) -> Observation {
    let mut pdu = LldpPdu::new(ChassisId::Mac { mac: dev }, PortId::Local { id: format!("port-{port:03}") }, 20);
    pdu.pn.chassis_mac = Some(dev);
    pdu.sys_name = sys_name.map(String::from);
    pdu.mgmt_addrs = mgmt.iter().map(|a| MgmtAddr { subtype: 1, addr: a.octets().to_vec(), if_subtype: 1, if_number: 0, oid: vec![] }).collect();
    Observation { witness: witness(Probe::LldpHeard, iface, frame), observed: Observed::Lldp { src: Mac([0x02, 0, 0, 0, dev.0[5], port]), pdu } }
}

fn arp(dev: Mac, address: Ipv4Addr, frame: u64) -> Observation {
    Observation { witness: witness(Probe::ArpHeard, "eth9", frame), observed: Observed::Arp { mac: dev, ip: address } }
}

/// A scan of these observations, listened on these interfaces, with these neighbour reports.
fn scan_of(obs: &[Observation], interfaces: &[&str], neighbours: &[Neighbour]) -> Scan {
    let mut fusion = Fusion::new();
    for o in obs {
        fusion.apply(o);
    }
    let stats = Stats { interfaces: interfaces.iter().map(|i| (i.to_string(), Account::default())).collect(), ..Stats::default() };
    let ifaces: Vec<Interface> = interfaces.iter().map(|i| Interface { name: i.to_string(), mac: None }).collect();
    let t = topology::build(&fusion, &ifaces, neighbours);
    Scan::new(&fusion, &stats, &t, T0 + 1_000_000_000, None)
}

fn neighbour(device: Mac, port: u8, peer: Mac, peer_port: u8) -> Neighbour {
    Neighbour { device, port: format!("port-{port:03}"), chassis: format!("mac:{peer}"), peer_port: format!("port-{peer_port:03}"), peer_mac: Some(peer), witnesses: 1 }
}

fn end(node: &str, port: &str) -> End {
    End { node: node.into(), port: port.into() }
}

fn ends(links: &[Link]) -> Vec<(End, End, Confidence)> {
    links.iter().map(|l| (l.a.clone(), l.b.clone(), l.confidence)).collect()
}

fn change(mac: Mac, what: Said, from: &str, to: &str) -> Change {
    Change { mac, what, from: from.into(), to: to.into() }
}

/// The baseline of the hand-checked case: five devices, C and B sharing an address, A heard on eth1
/// through its port 1, B on eth2, A reporting its port 3's neighbour B; listened on eth1 to eth3.
fn baseline() -> Scan {
    scan_of(
        &[
            hello(A, Some("bomba-1"), Some(suite(ip(10))), 1),
            lldp(A, 1, "eth1", Some("bomba-1"), &[ip(10)], 2),
            arp(A, ip(10), 3),
            hello(B, Some(""), Some(suite(ip(11))), 4),
            lldp(B, 1, "eth2", None, &[], 5),
            hello(C, Some("c-1"), Some(suite(ip(11))), 6),
            hello(E, Some("e-1"), Some(suite(ip(14))), 7),
        ],
        &["eth1", "eth2", "eth3"],
        &[neighbour(A, 3, B, 3)],
    )
}

/// The same plant later: A renamed in the scan's course and readdressed, its ARP not heard, its
/// cable on eth1 moved to its port 2; B named; C gone; D new with A's old name and A's new address;
/// E without an address; both ends of A's port 3 reporting each other; listened on eth1, eth2, eth4.
fn later() -> Scan {
    scan_of(
        &[
            hello(A, Some("bomba-1"), Some(suite(ip(20))), 10),
            hello(A, Some("bomba-2"), Some(suite(ip(20))), 11),
            lldp(A, 2, "eth1", Some("bomba-1"), &[ip(21), ip(20)], 12),
            hello(B, Some("valvula-1"), Some(suite(ip(11))), 13),
            lldp(B, 1, "eth2", None, &[], 14),
            lldp(B, 1, "eth2", None, &[], 15),
            hello(D, Some("bomba-1"), Some(suite(ip(20))), 16),
            hello(E, Some("e-1"), Some(suite(Ipv4Addr::UNSPECIFIED)), 17),
        ],
        &["eth1", "eth2", "eth4"],
        &[neighbour(A, 3, B, 3), neighbour(B, 3, A, 3)],
    )
}

#[test]
fn each_change_is_found_by_the_mac_and_only_where_both_scans_state_it() {
    let d = diff::diff(&baseline(), &later()).unwrap();
    // By the MAC: D took A's old name and A's new address, and is D appeared, never A moved.
    assert_eq!(d.appeared, [DeviceRef { mac: D, label: "bomba-1".into() }]);
    assert_eq!(d.vanished, [DeviceRef { mac: C, label: "c-1".into() }]);
    // What each states now, its value heard last; an empty name is a name.
    assert_eq!(d.renamed, [change(A, Said::StationName, "bomba-1", "bomba-2"), change(B, Said::StationName, "", "valvula-1")]);
    assert_eq!(
        d.readdressed,
        [
            change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.20 255.255.255.0 0.0.0.0"),
            change(A, Said::ManagementAddresses, "192.168.0.10", "192.168.0.20, 192.168.0.21"),
            change(E, Said::IpSuite, "192.168.0.14 255.255.255.0 0.0.0.0", "0.0.0.0 255.255.255.0 0.0.0.0"),
        ]
    );
    // A's ARP was not heard this time: said, neither changed nor silent.
    assert_eq!(d.one_sided, [OneSided { mac: A, what: Said::ArpAddress, baseline: Some("192.168.0.10".into()), scan: None }]);
    // A's cable moved to its port 2; B's cable on eth2, heard more often, and A's port 3, reported
    // then confirmed, are the same cables.
    let a = topology::device_id(A);
    let b = topology::device_id(B);
    assert_eq!(ends(&d.links_removed), [(end(&a, "port-001"), end("if:eth1", ""), Confidence::Heard)]);
    assert_eq!(ends(&d.links_added), [(end(&a, "port-002"), end("if:eth1", ""), Confidence::Heard)]);
    assert!(later().topology.links.iter().any(|l| (l.a.clone(), l.b.clone(), l.confidence) == (end(&a, "port-003"), end(&b, "port-003"), Confidence::Confirmed)));
    assert!(baseline().topology.links.iter().any(|l| (l.a.clone(), l.b.clone(), l.confidence) == (end(&a, "port-003"), end(&b, "port-003"), Confidence::Reported)));
    // A named itself twice in the scan, and shares its address with D; C's address with B is gone.
    let conflict = |kind, value: &str, macs: &[Mac]| Conflict { kind, value: value.into(), macs: macs.to_vec() };
    assert_eq!(d.conflicts_new, [conflict(ConflictKind::DuplicateIp, "192.168.0.20", &[A, D]), conflict(ConflictKind::NameChanged, "bomba-1, bomba-2", &[A])]);
    assert_eq!(d.conflicts_cleared, [conflict(ConflictKind::DuplicateIp, "192.168.0.11", &[B, C])]);
    assert_eq!((d.interfaces_added, d.interfaces_removed), (vec!["eth4".to_string()], vec!["eth3".to_string()]));
}

#[test]
fn a_scan_against_itself_is_no_change_and_the_other_way_round_is_the_mirror() {
    let (b, l) = (baseline(), later());
    assert!(diff::diff(&b, &b).unwrap().is_empty());
    assert!(diff::diff(&l, &l).unwrap().is_empty());
    assert!(!diff::diff(&b, &l).unwrap().is_empty());
    assert_eq!(diff::diff(&l, &b).unwrap(), mirror(diff::diff(&b, &l).unwrap()));
}

/// What the diff of the same two scans the other way round must be.
fn mirror(d: Diff) -> Diff {
    let back = |c: Vec<Change>| c.into_iter().map(|c| Change { mac: c.mac, what: c.what, from: c.to, to: c.from }).collect();
    Diff {
        appeared: d.vanished,
        vanished: d.appeared,
        renamed: back(d.renamed),
        readdressed: back(d.readdressed),
        one_sided: d.one_sided.into_iter().map(|o| OneSided { mac: o.mac, what: o.what, baseline: o.scan, scan: o.baseline }).collect(),
        links_added: d.links_removed,
        links_removed: d.links_added,
        conflicts_new: d.conflicts_cleared,
        conflicts_cleared: d.conflicts_new,
        interfaces_added: d.interfaces_removed,
        interfaces_removed: d.interfaces_added,
    }
}

/// A small generator, so the sweep is the same every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const NAMES: [Option<&str>; 4] = [None, Some(""), Some("bomba-1"), Some("bomba-2")];

/// A random plant of up to five devices from a pool of six, each stating some of its values.
fn random_scan(rng: &mut Rng) -> Scan {
    let mut obs = Vec::new();
    let mut frame = 0;
    let mut next = || {
        frame += 1;
        frame
    };
    let pool: Vec<Mac> = (1..=6).map(|i| Mac([0x02, 0, 0, 0, 0, i])).collect();
    let mut neighbours = Vec::new();
    // Up to four hearings of each kind, so a value heard x, y then x, or x, x then y, is common;
    // suites whose mask or gateway alone differ; system names alike but for their case; a
    // management address of 0.0.0.0; reports from and to two ports.
    let suites = [None, Some(suite(ip(10))), Some(suite(ip(11))), Some(suite(Ipv4Addr::UNSPECIFIED)), Some(suite_of(ip(10), 16, Ipv4Addr::UNSPECIFIED)), Some(suite_of(ip(10), 24, ip(1)))];
    for mac in &pool {
        if rng.below(3) == 0 {
            continue;
        }
        for _ in 0..rng.below(5) {
            let name = NAMES[rng.below(4) as usize];
            obs.push(hello(*mac, name, suites[rng.below(6) as usize].clone(), next()));
        }
        for _ in 0..rng.below(4) {
            obs.push(arp(*mac, ip(10 + rng.below(2) as u8), next()));
        }
        for _ in 0..rng.below(5) {
            let iface = ["eth1", "eth2", "eth3"][rng.below(3) as usize];
            let sys = [None, Some("sw-1"), Some("Sw-1"), Some("sw-2")][rng.below(4) as usize];
            let mgmt: Vec<Ipv4Addr> = (0..rng.below(3)).map(|k| if k == 0 && rng.below(3) == 0 { Ipv4Addr::UNSPECIFIED } else { ip(10 + k as u8) }).collect();
            obs.push(lldp(*mac, 1 + rng.below(2) as u8, iface, sys, &mgmt, next()));
        }
        if rng.below(3) == 0 {
            let peer = pool[rng.below(6) as usize];
            neighbours.push(neighbour(*mac, 3 + rng.below(2) as u8, peer, 3 + rng.below(2) as u8));
        }
    }
    let interfaces: Vec<&str> = ["eth1", "eth2", "eth3", "eth4"].into_iter().filter(|_| rng.below(4) != 0).collect();
    scan_of(&obs, &interfaces, &neighbours)
}

/// The same scan, its conflicts (and each conflict's devices) in another order: neither is ordered
/// by what makes a scan (its devices and its links are: C-146).
fn shuffled(mut s: Scan, rng: &mut Rng) -> Scan {
    for _ in 0..8 {
        let n = s.conflicts.len();
        if n > 1 {
            s.conflicts.swap(rng.below(n as u64) as usize, rng.below(n as u64) as usize);
        }
    }
    for c in &mut s.conflicts {
        c.macs.reverse();
    }
    s
}

/// Every value the diff says, against the clause's words read from the two scans directly.
fn check_against_the_words(b: &Scan, s: &Scan, d: &Diff) {
    let by_mac = |x: &Scan| -> BTreeMap<Mac, String> { x.devices.iter().map(|d| (d.mac, d.label())).collect() };
    let (bm, sm) = (by_mac(b), by_mac(s));
    let appeared: Vec<DeviceRef> = sm.iter().filter(|(m, _)| !bm.contains_key(m)).map(|(m, l)| DeviceRef { mac: *m, label: l.clone() }).collect();
    let vanished: Vec<DeviceRef> = bm.iter().filter(|(m, _)| !sm.contains_key(m)).map(|(m, l)| DeviceRef { mac: *m, label: l.clone() }).collect();
    assert_eq!((&d.appeared, &d.vanished), (&appeared, &vanished));
    let (mut renamed, mut readdressed, mut one_sided) = (Vec::new(), Vec::new(), Vec::new());
    for sd in &s.devices {
        let Some(bd) = b.devices.iter().find(|x| x.mac == sd.mac) else { continue };
        let values = |x: &veredas_aranha::fusion::Device| -> [(Said, Option<String>); 5] {
            [
                (Said::StationName, x.name.current().cloned()),
                (Said::SystemName, x.sys_name.current().cloned()),
                (Said::IpSuite, x.ip.current().map(|s| format!("{} {} {}", s.ip, s.mask, s.gateway))),
                (Said::ArpAddress, x.arp_ip.current().map(|a| a.to_string())),
                (Said::ManagementAddresses, x.mgmt_ips.current().map(|m| m.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "))),
            ]
        };
        for ((what, was), (_, is)) in values(bd).into_iter().zip(values(sd)) {
            match (was, is) {
                (Some(f), Some(t)) if f == t => {}
                (Some(f), Some(t)) => {
                    let c = change(sd.mac, what, &f, &t);
                    if matches!(what, Said::StationName | Said::SystemName) { renamed.push(c) } else { readdressed.push(c) }
                }
                (None, None) => {}
                (baseline, scan) => one_sided.push(OneSided { mac: sd.mac, what, baseline, scan }),
            }
        }
    }
    assert_eq!((&d.renamed, &d.readdressed, &d.one_sided), (&renamed, &readdressed, &one_sided));
    let key = |l: &Link| (l.a.clone(), l.b.clone());
    let mut added: Vec<Link> = s.topology.links.iter().filter(|l| !b.topology.links.iter().any(|x| key(x) == key(l))).cloned().collect();
    let mut removed: Vec<Link> = b.topology.links.iter().filter(|l| !s.topology.links.iter().any(|x| key(x) == key(l))).cloned().collect();
    added.sort_by_key(key);
    removed.sort_by_key(key);
    assert_eq!((&d.links_added, &d.links_removed), (&added, &removed));
    let norm = |c: &Conflict| {
        let mut c = c.clone();
        c.macs.sort();
        c
    };
    let mut new: Vec<Conflict> = s.conflicts.iter().map(norm).filter(|c| !b.conflicts.iter().map(norm).any(|x| x == *c)).collect();
    let mut cleared: Vec<Conflict> = b.conflicts.iter().map(norm).filter(|c| !s.conflicts.iter().map(norm).any(|x| x == *c)).collect();
    new.sort();
    cleared.sort();
    assert_eq!((&d.conflicts_new, &d.conflicts_cleared), (&new, &cleared));
    let added: Vec<String> = s.stats.interfaces.keys().filter(|k| !b.stats.interfaces.contains_key(*k)).cloned().collect();
    let removed: Vec<String> = b.stats.interfaces.keys().filter(|k| !s.stats.interfaces.contains_key(*k)).cloned().collect();
    assert_eq!((&d.interfaces_added, &d.interfaces_removed), (&added, &removed));
}

#[test]
fn every_pair_of_scans_gives_the_diff_the_words_say_in_any_order_and_mirrored() {
    let mut rng = Rng(0x5EED_D1FF);
    let mut changes = [0usize; 6];
    for _ in 0..3000 {
        let (b, s) = (random_scan(&mut rng), random_scan(&mut rng));
        let d = diff::diff(&b, &s).unwrap();
        check_against_the_words(&b, &s, &d);
        assert_eq!(diff::diff(&shuffled(b.clone(), &mut rng), &shuffled(s.clone(), &mut rng)).unwrap(), d);
        // Links in another order make no scan: refused, never a diff of whatever order they hold.
        if s.topology.links.len() > 1 {
            let mut reordered = s.clone();
            reordered.topology.links.reverse();
            assert!(matches!(diff::diff(&b, &reordered), Err(DiffError::Scan(ScanError::Invalid(_)))));
        }
        assert_eq!(diff::diff(&s, &b).unwrap(), mirror(d.clone()));
        assert!(diff::diff(&b, &b).unwrap().is_empty());
        for (i, n) in [d.appeared.len(), d.renamed.len(), d.readdressed.len(), d.one_sided.len(), d.links_added.len(), d.conflicts_new.len()].into_iter().enumerate() {
            changes[i] += n;
        }
    }
    // The sweep reached every kind of change, many times.
    assert!(changes.iter().all(|n| *n > 100), "{changes:?}");
}

#[test]
fn a_baseline_or_a_scan_that_is_no_scan_is_refused_by_name() {
    let mut twice = baseline();
    twice.devices.insert(1, twice.devices[0].clone());
    match diff::diff(&twice, &later()) {
        Err(DiffError::Baseline(ScanError::Invalid(why))) => assert!(why.contains("twice"), "{why}"),
        other => panic!("{other:?}"),
    }
    let mut past = later();
    past.capture = Some(Capture { file: "later.pcap".into(), first_frame: 0, frames: 5 });
    match diff::diff(&baseline(), &past) {
        Err(e @ DiffError::Scan(ScanError::Invalid(_))) => assert!(e.to_string().starts_with("the scan: not a scan: device "), "{e}"),
        other => panic!("{other:?}"),
    }
}

/// The first refute round's cases: values heard more than twice, a suite's mask or gateway alone,
/// a link's far port, an address of 0.0.0.0 among others, a name's case; links a scan cannot hold.
#[test]
fn what_a_device_states_now_is_its_value_heard_last_each_part_of_it_compared() {
    let one = |obs: &[Observation]| scan_of(obs, &["eth1"], &[]);
    let station = |d: &Diff| d.renamed.iter().filter(|c| c.what == Said::StationName).map(|c| (c.from.clone(), c.to.clone())).collect::<Vec<_>>();
    let base = one(&[hello(A, Some("bomba-1"), None, 1)]);
    // Renamed and back within the scan: what it states now is its old name, no rename (the
    // name change within the scan is a conflict of its own).
    let back = one(&[hello(A, Some("bomba-1"), None, 1), hello(A, Some("bomba-2"), None, 2), hello(A, Some("bomba-1"), None, 3)]);
    let d = diff::diff(&base, &back).unwrap();
    assert_eq!(station(&d), Vec::<(String, String)>::new());
    assert_eq!(d.conflicts_new.iter().map(|c| c.kind).collect::<Vec<_>>(), [ConflictKind::NameChanged]);
    // Heard twice by its old name, then once by the new one: renamed, the most heard aside.
    let renamed = one(&[hello(A, Some("bomba-1"), None, 1), hello(A, Some("bomba-1"), None, 2), hello(A, Some("bomba-2"), None, 3)]);
    assert_eq!(station(&diff::diff(&base, &renamed).unwrap()), [("bomba-1".to_string(), "bomba-2".to_string())]);
    // The same for the ARP address, the system name, the management addresses and the IP suite.
    let b = one(&[arp(A, ip(10), 1), lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 2), hello(A, None, Some(suite(ip(10))), 3)]);
    let s = one(&[
        arp(A, ip(11), 1),
        arp(A, ip(10), 2),
        arp(A, ip(11), 3),
        lldp(A, 1, "eth1", Some("sw-2"), &[ip(11)], 4),
        lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 5),
        lldp(A, 1, "eth1", Some("sw-2"), &[ip(11)], 6),
        hello(A, None, Some(suite(ip(11))), 7),
        hello(A, None, Some(suite(ip(10))), 8),
        hello(A, None, Some(suite(ip(11))), 9),
    ]);
    let d = diff::diff(&b, &s).unwrap();
    assert_eq!(d.renamed, [change(A, Said::SystemName, "sw-1", "sw-2")]);
    assert_eq!(
        d.readdressed,
        [
            change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.11 255.255.255.0 0.0.0.0"),
            change(A, Said::ArpAddress, "192.168.0.10", "192.168.0.11"),
            change(A, Said::ManagementAddresses, "192.168.0.10", "192.168.0.11"),
        ]
    );
    // A mask alone, a gateway alone: readdressed.
    let mask = one(&[hello(A, None, Some(suite_of(ip(10), 16, Ipv4Addr::UNSPECIFIED)), 1)]);
    let gateway = one(&[hello(A, None, Some(suite_of(ip(10), 24, ip(1))), 1)]);
    let plain = one(&[hello(A, None, Some(suite(ip(10))), 1)]);
    assert_eq!(diff::diff(&plain, &mask).unwrap().readdressed, [change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.10 255.255.0.0 0.0.0.0")]);
    assert_eq!(diff::diff(&plain, &gateway).unwrap().readdressed, [change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.10 255.255.255.0 192.168.0.1")]);
    // 0.0.0.0 among the management addresses is one of them; a name differing in case alone is
    // another name.
    let zero = one(&[lldp(A, 1, "eth1", Some("Sw-1"), &[Ipv4Addr::UNSPECIFIED, ip(10)], 1)]);
    let ten = one(&[lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 1)]);
    let d = diff::diff(&zero, &ten).unwrap();
    assert_eq!(d.renamed, [change(A, Said::SystemName, "Sw-1", "sw-1")]);
    assert_eq!(d.readdressed, [change(A, Said::ManagementAddresses, "0.0.0.0, 192.168.0.10", "192.168.0.10")]);
    // A link's far port changed: the link removed and another added.
    let fused = [hello(A, Some("a"), None, 1), hello(B, Some("b"), None, 2)];
    let to3 = scan_of(&fused, &["eth1"], &[neighbour(A, 3, B, 3)]);
    let to4 = scan_of(&fused, &["eth1"], &[neighbour(A, 3, B, 4)]);
    let d = diff::diff(&to3, &to4).unwrap();
    let (a, b) = (topology::device_id(A), topology::device_id(B));
    assert_eq!(ends(&d.links_removed), [(end(&a, "port-003"), end(&b, "port-003"), Confidence::Reported)]);
    assert_eq!(ends(&d.links_added), [(end(&a, "port-003"), end(&b, "port-004"), Confidence::Reported)]);
    // One cable stored the other way round, or twice: no scan, refused, never a rewiring.
    let mut flipped = to3.clone();
    let l = &mut flipped.topology.links[0];
    std::mem::swap(&mut l.a, &mut l.b);
    assert!(matches!(diff::diff(&to3, &flipped), Err(DiffError::Scan(ScanError::Invalid(why))) if why.ends_with("is stored greater end first")));
    let mut twice = to3.clone();
    let again = Link { confidence: Confidence::Confirmed, witnesses: 2, ..twice.topology.links[0].clone() };
    twice.topology.links.push(again);
    assert!(matches!(diff::diff(&twice, &to3), Err(DiffError::Baseline(ScanError::Invalid(why))) if why.ends_with("is in it twice")));
}

/// The second refute round's cases.
#[test]
fn a_station_name_s_case_a_stamp_shared_by_two_frames_every_conflict_kind_and_an_ended_account() {
    use veredas_aranha::profinet::dcp::IpStatus;
    let one = |obs: &[Observation]| scan_of(obs, &["eth1"], &[]);
    let at = |dev: Mac, name: &str, address: Option<IpSuite>, conflict: Option<bool>, stamp: u64, frame: u64| {
        let identity = DeviceIdentity { name: Some(name.into()), ip: address, ip_status: conflict.map(|c| IpStatus { set: 1, conflict: c }), ..Default::default() };
        Observation { witness: Witness { probe: Probe::DcpHeard, iface: "eth9".into(), at_ns: T0 + stamp, frame }, observed: Observed::Dcp { mac: dev, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
    };
    // A station name differing in case alone is another name.
    let d = diff::diff(&one(&[at(A, "bomba-1", None, None, 1, 1)]), &one(&[at(A, "Bomba-1", None, None, 1, 1)])).unwrap();
    assert_eq!(d.renamed, [change(A, Said::StationName, "bomba-1", "Bomba-1")]);
    // Two frames at one stamp: the later frame is heard last.
    let base = one(&[at(A, "bomba-1", Some(suite(ip(10))), None, 1, 1)]);
    let later = one(&[at(A, "bomba-2", Some(suite(ip(20))), None, 1, 1), at(A, "bomba-1", Some(suite(ip(10))), None, 2, 2), at(A, "bomba-1", Some(suite(ip(10))), None, 5, 3), at(A, "bomba-2", Some(suite(ip(20))), None, 5, 4)]);
    let d = diff::diff(&base, &later).unwrap();
    assert_eq!(d.renamed, [change(A, Said::StationName, "bomba-1", "bomba-2")]);
    assert_eq!(d.readdressed, [change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.20 255.255.255.0 0.0.0.0")]);
    let back = one(&[at(A, "bomba-2", Some(suite(ip(20))), None, 1, 1), at(A, "bomba-1", Some(suite(ip(10))), None, 2, 2), at(A, "bomba-2", Some(suite(ip(20))), None, 5, 3), at(A, "bomba-1", Some(suite(ip(10))), None, 5, 4)]);
    assert_eq!((diff::diff(&base, &back).unwrap().renamed, diff::diff(&base, &back).unwrap().readdressed), (vec![], vec![]));
    // A device flagging its own address in conflict: new, and cleared the other way round; at
    // 0.0.0.0 its value is empty and it is new all the same.
    let calm = one(&[at(A, "a", Some(suite(ip(10))), Some(false), 1, 1)]);
    let flagged = one(&[at(A, "a", Some(suite(ip(10))), Some(true), 1, 1)]);
    let reports = |value: &str| Conflict { kind: ConflictKind::ReportsIpConflict, value: value.into(), macs: vec![A] };
    assert_eq!(diff::diff(&calm, &flagged).unwrap().conflicts_new, [reports("192.168.0.10")]);
    assert_eq!(diff::diff(&flagged, &calm).unwrap().conflicts_cleared, [reports("192.168.0.10")]);
    let unaddressed = one(&[at(A, "a", Some(suite(Ipv4Addr::UNSPECIFIED)), Some(true), 1, 1)]);
    assert_eq!(diff::diff(&calm, &unaddressed).unwrap().conflicts_new, [reports("")]);
    // An interface whose account says its capture was read to the end was listened on all the
    // same: no interface added or removed against one still live.
    let mut replayed = calm.clone();
    replayed.stats.interfaces.get_mut("eth1").unwrap().ended = Some("its capture is read to the end".into());
    let d = diff::diff(&replayed, &calm).unwrap();
    assert_eq!((d.interfaces_added.len(), d.interfaces_removed.len()), (0, 0));
    // Two values of one field last heard in one frame: no scan, refused, never a diff of the
    // order they are listed in.
    let mut tie = later.clone();
    let mut again = tie.devices[0].name.candidates[0].clone();
    again.value = "bomba-3".into();
    again.last = tie.devices[0].name.candidates[1].last.clone();
    tie.devices[0].name.candidates.push(again);
    assert!(matches!(diff::diff(&base, &tie), Err(DiffError::Scan(ScanError::Invalid(why))) if why.contains("'s name holds two values last heard in frame")));
}

#[cfg(target_os = "linux")]
mod bench;

/// The bench of the baseline: two devices of one station name, each on its own segment.
#[cfg(target_os = "linux")]
const BEFORE: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "first"
interface = "b0"
station = "bomba-1"
ip = { address = "192.168.0.10", mask = "255.255.255.0" }
lldp_interval_ms = 200
seed = 1

[[profinet]]
name = "second"
interface = "b1"
station = "bomba-1"
ip = { address = "192.168.0.11", mask = "255.255.255.0" }
lldp_interval_ms = 200
seed = 2
"#;

/// The same bench later: the first device renamed, readdressed and its cable on its port 2; the
/// second gone, a third in its place taking the first's new address.
#[cfg(target_os = "linux")]
const AFTER: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "first"
interface = "b0"
station = "bomba-2"
ip = { address = "192.168.0.20", mask = "255.255.255.0" }
lldp_interval_ms = 200
port = "port-002"
seed = 1

[[profinet]]
name = "third"
interface = "b1"
station = "valvula-1"
ip = { address = "192.168.0.20", mask = "255.255.255.0" }
lldp_interval_ms = 200
seed = 3
"#;

#[cfg(target_os = "linux")]
const FIRST: Mac = Mac([0x02, 0x00, 0x5E, 0xD1, 0xF0, 0x01]);
#[cfg(target_os = "linux")]
const SECOND: Mac = Mac([0x02, 0x00, 0x5E, 0xD1, 0xF0, 0x02]);
#[cfg(target_os = "linux")]
const THIRD: Mac = Mac([0x02, 0x00, 0x5E, 0xD1, 0xF0, 0x03]);
#[cfg(target_os = "linux")]
const HERE0: Mac = Mac([0x02, 0x00, 0x5E, 0xD1, 0xA0, 0x00]);
#[cfg(target_os = "linux")]
const HERE1: Mac = Mac([0x02, 0x00, 0x5E, 0xD1, 0xA0, 0x01]);

/// A scan as an engineer takes one: listening on both interfaces before the bench starts, an
/// Identify of every device on each, the topology of what was heard while the devices still run,
/// written to its file and read back.
#[cfg(target_os = "linux")]
fn scan_the_bench(name: &str, file: &str, devices: [Mac; 2]) -> Scan {
    use std::time::Duration;
    use veredas_aranha::ask::{self, Identify};
    use veredas_aranha::listen::{Listener, Options};

    let l = Listener::start(vec![bench::link("a0"), bench::link("a1")], Options::default()).unwrap();
    let run = bench::start(name, file);
    let heard = |s: &veredas_aranha::listen::Snapshot| devices.iter().all(|m| s.fusion.devices.get(m).is_some_and(|d| d.heard.contains(&Probe::DcpHeard) && d.heard.contains(&Probe::LldpHeard)));
    assert!(l.until(Duration::from_secs(10), heard), "{:#?}", l.snapshot());
    for iface in ["a0", "a1"] {
        let asked = ask::identify(&l, iface, &Identify { filter: dcp::Filter::All, response_delay: 1, wait: Duration::from_millis(500) }).unwrap();
        assert_eq!(asked.answers.len(), 1, "{iface}: {asked:?}");
    }
    let s = l.stop();
    let (status, out, err) = bench::end(run);
    assert!(status.success(), "the bench: {status}\n{out}\n{err}");
    let interfaces: Vec<Interface> = s.stats.interfaces.iter().map(|(name, i)| Interface { name: name.clone(), mac: i.mac }).collect();
    let t = topology::build(&s.fusion, &interfaces, &[]);
    let scan = Scan::new(&s.fusion, &s.stats, &t, veredas_aranha::wire::stamp(std::time::SystemTime::now()), None);
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("diff-{name}.toml"));
    scan.write_file(&path).unwrap();
    let read = Scan::read_file(&path).unwrap();
    assert_eq!(read, scan);
    read
}

#[cfg(target_os = "linux")]
#[test]
fn a_baseline_of_the_bench_and_the_bench_changed_since_are_told_apart() {
    use veredas_aranha::netns;

    // Built on the host, before the private network is made.
    bench::maquinar();
    netns::isolated("a_baseline_of_the_bench_and_the_bench_changed_since_are_told_apart", &[("a0", "b0"), ("a1", "b1")], || {
        bench::pin(&[("b0", FIRST), ("b1", SECOND), ("a0", HERE0), ("a1", HERE1)], &[("a0", "b0"), ("a1", "b1")]);
        let before = scan_the_bench("diff-before", BEFORE, [FIRST, SECOND]);
        // The second device unplugged, a third in its place.
        bench::pin(&[("b1", THIRD)], &[("a1", "b1")]);
        let after = scan_the_bench("diff-after", AFTER, [FIRST, THIRD]);

        let d = diff::diff(&before, &after).unwrap();
        assert_eq!(d.appeared, [DeviceRef { mac: THIRD, label: "valvula-1".into() }]);
        assert_eq!(d.vanished, [DeviceRef { mac: SECOND, label: "bomba-1".into() }]);
        assert_eq!(d.renamed, [change(FIRST, Said::StationName, "bomba-1", "bomba-2")]);
        assert_eq!(d.readdressed, [change(FIRST, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.20 255.255.255.0 0.0.0.0")]);
        assert_eq!(d.one_sided, []);
        let (first, second, third) = (topology::device_id(FIRST), topology::device_id(SECOND), topology::device_id(THIRD));
        let heard = |a: &str, port: &str, iface: &str| (end(a, port), end(iface, ""), Confidence::Heard);
        assert_eq!(ends(&d.links_removed), [heard(&first, "port-001", "if:a0"), heard(&second, "port-001", "if:a1")]);
        assert_eq!(ends(&d.links_added), [heard(&first, "port-002", "if:a0"), heard(&third, "port-001", "if:a1")]);
        let mut both = vec![FIRST, SECOND];
        both.sort();
        assert_eq!(d.conflicts_cleared, [Conflict { kind: ConflictKind::DuplicateName, value: "bomba-1".into(), macs: both }]);
        let mut both = vec![FIRST, THIRD];
        both.sort();
        assert_eq!(d.conflicts_new, [Conflict { kind: ConflictKind::DuplicateIp, value: "192.168.0.20".into(), macs: both }]);
        assert_eq!((d.interfaces_added.len(), d.interfaces_removed.len()), (0, 0));
        // The bench's devices state no system name and no management address in their LLDP.
        for s in [&before, &after] {
            assert!(s.devices.iter().all(|d| d.sys_name.is_empty() && d.mgmt_ips.is_empty()), "{:?}", s.devices);
        }
    });
}
