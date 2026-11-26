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

/// The third refute round's cases (its critic's probes, taken in): the value heard last where a
/// lower frame carries a later stamp (two interfaces' workers), a suite at 0.0.0.0 whose mask or
/// gateway alone differs, management addresses of 0.0.0.0 alone, listenings as two workers make them
/// accepted and diffed alike in any order of a field's values, a tie refused in each field the diff
/// reads, a conflict's value told apart by case.
mod round_3 {
    use std::net::Ipv4Addr;

    use veredas_aranha::diff::{self, Change, OneSided, Said};
    use veredas_aranha::fusion::Fusion;
    use veredas_aranha::listen::{Interface as Account, Stats};
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::lldp::{ChassisId, LldpPdu, MgmtAddr, PortId};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::scan::Scan;
    use veredas_aranha::topology::{self, Interface, Neighbour};

    /// One hearing to come: its stamp and frame given when it is kept.
    type Hearing = Box<dyn Fn(u64, u64) -> Observation>;

    const A: Mac = Mac([0x02, 0, 0, 0, 0, 0x0A]);
    const T0: u64 = 1_764_000_000_000_000_000;

    fn ip(last: u8) -> Ipv4Addr { Ipv4Addr::new(192, 168, 0, last) }
    fn suite(address: Ipv4Addr) -> IpSuite { IpSuite { ip: address, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED } }
    fn w(probe: Probe, iface: &str, at: u64, frame: u64) -> Witness { Witness { probe, iface: iface.into(), at_ns: T0 + at, frame } }

    fn hello(dev: Mac, name: Option<&str>, address: Option<IpSuite>, iface: &str, at: u64, frame: u64) -> Observation {
        let identity = DeviceIdentity { name: name.map(String::from), ip: address, ..Default::default() };
        Observation { witness: w(Probe::DcpHeard, iface, at, frame), observed: Observed::Dcp { mac: dev, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
    }
    fn lldp(dev: Mac, port: u8, iface: &str, sys_name: Option<&str>, mgmt: &[Ipv4Addr], at: u64, frame: u64) -> Observation {
        let mut pdu = LldpPdu::new(ChassisId::Mac { mac: dev }, PortId::Local { id: format!("port-{port:03}") }, 20);
        pdu.pn.chassis_mac = Some(dev);
        pdu.sys_name = sys_name.map(String::from);
        pdu.mgmt_addrs = mgmt.iter().map(|a| MgmtAddr { subtype: 1, addr: a.octets().to_vec(), if_subtype: 1, if_number: 0, oid: vec![] }).collect();
        Observation { witness: w(Probe::LldpHeard, iface, at, frame), observed: Observed::Lldp { src: Mac([0x02, 0, 0, 0, dev.0[5], port]), pdu } }
    }
    /// An LLDP PDU from a port's MAC naming no MAC for its chassis (a port known before its device).
    fn lldp_bare(port_mac: Mac, port: u8, iface: &str, sys_name: Option<&str>, at: u64, frame: u64) -> Observation {
        let mut pdu = LldpPdu::new(ChassisId::Local { id: "x".into() }, PortId::Local { id: format!("port-{port:03}") }, 20);
        pdu.sys_name = sys_name.map(String::from);
        Observation { witness: w(Probe::LldpHeard, iface, at, frame), observed: Observed::Lldp { src: port_mac, pdu } }
    }
    fn arp(dev: Mac, address: Ipv4Addr, iface: &str, at: u64, frame: u64) -> Observation {
        Observation { witness: w(Probe::ArpHeard, iface, at, frame), observed: Observed::Arp { mac: dev, ip: address } }
    }
    fn scan_of(obs: &[Observation], interfaces: &[&str], neighbours: &[Neighbour]) -> Scan {
        let mut fusion = Fusion::new();
        for o in obs { fusion.apply(o); }
        let stats = Stats { interfaces: interfaces.iter().map(|i| (i.to_string(), Account::default())).collect(), ..Stats::default() };
        let ifaces: Vec<Interface> = interfaces.iter().map(|i| Interface { name: i.to_string(), mac: None }).collect();
        let t = topology::build(&fusion, &ifaces, neighbours);
        Scan::new(&fusion, &stats, &t, T0 + 1_000_000_000, None)
    }
    fn change(mac: Mac, what: Said, from: &str, to: &str) -> Change { Change { mac, what, from: from.into(), to: to.into() } }
    fn back_and_forth(path: &str, s: &Scan) -> Scan {
        let p = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(path);
        s.write_file(&p).unwrap();
        Scan::read_file(&p).unwrap()
    }

    /// Two workers keep frames in the order each reaches the lock, their stamps the wire's: a frame
    /// of a lower index can carry a later stamp (a device heard on two interfaces, two captures
    /// replayed at once). Fusion's value heard last is the later stamp's; the diff must say so.
    #[test]
    fn q01_a_later_stamp_on_a_lower_frame_is_the_value_heard_last() {
        let base = scan_of(&[hello(A, Some("bomba-1"), Some(suite(ip(10))), "eth1", 1, 1), arp(A, ip(10), "eth1", 2, 2), lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 3, 3)], &["eth1", "eth2"], &[]);
        // In index order (as the listener applies them): the later-stamped values come first.
        let now = scan_of(
            &[
                hello(A, Some("bomba-2"), Some(suite(ip(20))), "eth1", 200, 5),
                hello(A, Some("bomba-1"), Some(suite(ip(10))), "eth2", 100, 6),
                arp(A, ip(20), "eth1", 201, 7),
                arp(A, ip(10), "eth2", 101, 8),
                lldp(A, 1, "eth1", Some("sw-2"), &[ip(20)], 202, 9),
                lldp(A, 2, "eth2", Some("sw-1"), &[ip(10)], 102, 10),
            ],
            &["eth1", "eth2"],
            &[],
        );
        let a = &now.devices[0];
        assert_eq!(a.name.current().map(String::as_str), Some("bomba-2"), "fusion's value heard last");
        let d = diff::diff(&base, &now).unwrap();
        assert_eq!(d.renamed, [change(A, Said::StationName, "bomba-1", "bomba-2"), change(A, Said::SystemName, "sw-1", "sw-2")]);
        assert_eq!(
            d.readdressed,
            [
                change(A, Said::IpSuite, "192.168.0.10 255.255.255.0 0.0.0.0", "192.168.0.20 255.255.255.0 0.0.0.0"),
                change(A, Said::ArpAddress, "192.168.0.10", "192.168.0.20"),
                change(A, Said::ManagementAddresses, "192.168.0.10", "192.168.0.20"),
            ]
        );
        // Read back from its file, the same.
        let read = back_and_forth("diff-round-3-q01.toml", &now);
        assert_eq!(diff::diff(&base, &read).unwrap(), d);
    }

    /// An IP suite at 0.0.0.0 whose mask (or gateway) alone differs: a value, readdressed.
    #[test]
    fn q02_a_suite_at_zero_whose_mask_alone_differs_is_readdressed() {
        let z = |mask: [u8; 4], gw: [u8; 4]| IpSuite { ip: Ipv4Addr::UNSPECIFIED, mask: mask.into(), gateway: gw.into() };
        let b = scan_of(&[hello(A, None, Some(z([255, 255, 255, 0], [0, 0, 0, 0])), "eth1", 1, 1)], &["eth1"], &[]);
        let s = scan_of(&[hello(A, None, Some(z([0, 0, 0, 0], [0, 0, 0, 0])), "eth1", 1, 1)], &["eth1"], &[]);
        let g = scan_of(&[hello(A, None, Some(z([255, 255, 255, 0], [192, 168, 0, 1])), "eth1", 1, 1)], &["eth1"], &[]);
        assert_eq!(diff::diff(&b, &s).unwrap().readdressed, [change(A, Said::IpSuite, "0.0.0.0 255.255.255.0 0.0.0.0", "0.0.0.0 0.0.0.0 0.0.0.0")]);
        assert_eq!(diff::diff(&b, &g).unwrap().readdressed, [change(A, Said::IpSuite, "0.0.0.0 255.255.255.0 0.0.0.0", "0.0.0.0 255.255.255.0 192.168.0.1")]);
    }

    /// A management address set of 0.0.0.0 alone: a value.
    #[test]
    fn q03_management_addresses_of_zero_alone_are_a_value() {
        let zero = scan_of(&[lldp(A, 1, "eth1", None, &[Ipv4Addr::UNSPECIFIED], 1, 1)], &["eth1"], &[]);
        let ten = scan_of(&[lldp(A, 1, "eth1", None, &[ip(10)], 1, 1)], &["eth1"], &[]);
        let none = scan_of(&[lldp(A, 1, "eth1", None, &[], 1, 1)], &["eth1"], &[]);
        assert_eq!(diff::diff(&zero, &ten).unwrap().readdressed, [change(A, Said::ManagementAddresses, "0.0.0.0", "192.168.0.10")]);
        assert_eq!(diff::diff(&none, &zero).unwrap().one_sided, [OneSided { mac: A, what: Said::ManagementAddresses, baseline: None, scan: Some("0.0.0.0".into()) }]);
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
        fn below(&mut self, n: u64) -> u64 { self.next() % n }
    }

    /// A listening as two workers make one: frames numbered in the order kept, stamps per interface
    /// (a worker's stamps rise, the two interleave in any order), ports known before their device.
    fn listening(rng: &mut Rng) -> Scan {
        let pool: Vec<Mac> = (1..=4).map(|i| Mac([0x02, 0, 0, 0, 0, i])).collect();
        let names = [None, Some(""), Some("bomba-1"), Some("bomba-2")];
        let suites = [None, Some(suite(ip(10))), Some(suite(ip(11))), Some(suite(Ipv4Addr::UNSPECIFIED)), Some(IpSuite { ip: Ipv4Addr::UNSPECIFIED, mask: Ipv4Addr::UNSPECIFIED, gateway: Ipv4Addr::UNSPECIFIED })];
        // Each interface's own stream, its stamps rising.
        let mut streams: [Vec<Hearing>; 2] = [Vec::new(), Vec::new()];
        for mac in &pool {
            if rng.below(4) == 0 { continue; }
            let n = rng.below(7);
            for _ in 0..n {
                let iface = rng.below(2) as usize;
                let name = if iface == 0 { "eth1" } else { "eth2" };
                let mac = *mac;
                let f: Hearing = match rng.below(4) {
                    0 => {
                        let (nm, su) = (names[rng.below(4) as usize], suites[rng.below(5) as usize].clone());
                        Box::new(move |at, fr| hello(mac, nm, su.clone(), name, at, fr))
                    }
                    1 => {
                        let a = ip(10 + rng.below(2) as u8);
                        Box::new(move |at, fr| arp(mac, a, name, at, fr))
                    }
                    2 => {
                        let (port, sys) = (1 + rng.below(2) as u8, [None, Some("sw-1"), Some("sw-2")][rng.below(3) as usize]);
                        let mg: Vec<Ipv4Addr> = (0..rng.below(3)).map(|k| if k == 0 && rng.below(2) == 0 { Ipv4Addr::UNSPECIFIED } else { ip(10 + k as u8) }).collect();
                        Box::new(move |at, fr| lldp(mac, port, name, sys, &mg, at, fr))
                    }
                    _ => {
                        // A port's PDU naming no chassis MAC: kept at the port's MAC until its device's comes.
                        let port = 1 + rng.below(2) as u8;
                        let sys = [None, Some("sw-1"), Some("sw-3")][rng.below(3) as usize];
                        let src = Mac([0x02, 0, 0, 0, mac.0[5], port]);
                        Box::new(move |at, fr| lldp_bare(src, port, name, sys, at, fr))
                    }
                };
                streams[iface].push(f);
            }
        }
        // Stamps: each interface's rise from its own start; the two merged in a random interleaving.
        let mut at = [rng.below(50), rng.below(50)];
        let mut idx = [0usize, 0usize];
        let mut obs = Vec::new();
        let mut frame = 0u64;
        while idx[0] < streams[0].len() || idx[1] < streams[1].len() {
            let i = if idx[0] == streams[0].len() { 1 } else if idx[1] == streams[1].len() { 0 } else { rng.below(2) as usize };
            frame += 1;
            at[i] += 1 + rng.below(20);
            obs.push((streams[i][idx[i]])(at[i], frame));
            idx[i] += 1;
        }
        scan_of(&obs, &["eth1", "eth2"], &[])
    }

    /// Every listening such as two workers make is a scan (the new rule refuses none), and its diff
    /// against another is the same with each field's values in any order, written and read back.
    #[test]
    fn q04_listenings_are_scans_and_the_diff_ignores_the_order_of_a_field_s_values() {
        let mut rng = Rng(0xC147_0003);
        let mut disagree = 0;
        for k in 0..3000 {
            let (b, s) = (listening(&mut rng), listening(&mut rng));
            b.check().unwrap_or_else(|e| panic!("{k}: a listening refused: {e}"));
            s.check().unwrap_or_else(|e| panic!("{k}: a listening refused: {e}"));
            let d = diff::diff(&b, &s).unwrap();
            // Each field's candidates reversed (out of the "first heard first" order).
            let mut r = s.clone();
            for dv in &mut r.devices {
                dv.name.candidates.reverse();
                dv.sys_name.candidates.reverse();
                dv.ip.candidates.reverse();
                dv.arp_ip.candidates.reverse();
                dv.mgmt_ips.candidates.reverse();
            }
            assert_eq!(diff::diff(&b, &r).unwrap(), d, "{k}");
            if k % 10 == 0 {
                let read = back_and_forth("diff-round-3-q04.toml", &r);
                assert_eq!(diff::diff(&b, &read).unwrap(), d, "{k}");
            }
            // How often index order and stamp order disagree on the value heard last.
            for dv in &s.devices {
                let by_frame = dv.name.candidates.iter().max_by_key(|c| c.last.frame).map(|c| &c.value);
                if by_frame != dv.name.current() { disagree += 1; }
            }
        }
        assert!(disagree > 100, "{disagree}");
    }



    /// Each of the diff's own fields holding two values last heard in one frame: refused, never a
    /// diff of the order its list holds (the second round pinned the station name alone).
    #[test]
    fn q07_a_tie_in_each_field_the_diff_reads_is_refused() {
        let s = scan_of(
            &[hello(A, Some("bomba-1"), Some(suite(ip(10))), "eth1", 1, 1), arp(A, ip(10), "eth1", 2, 2), lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 3, 3)],
            &["eth1"],
            &[],
        );
        let base = s.clone();
        let mut seen = Vec::new();
        // The system name.
        let mut t = s.clone();
        let mut c = t.devices[0].sys_name.candidates[0].clone();
        c.value = "sw-9".into();
        t.devices[0].sys_name.candidates.insert(0, c);
        seen.push(("sys_name", diff::diff(&base, &t).map(|d| d.renamed)));
        // The IP suite.
        let mut t = s.clone();
        let mut c = t.devices[0].ip.candidates[0].clone();
        c.value = suite(ip(99));
        t.devices[0].ip.candidates.insert(0, c);
        seen.push(("ip", diff::diff(&base, &t).map(|d| d.renamed)));
        // The ARP address.
        let mut t = s.clone();
        let mut c = t.devices[0].arp_ip.candidates[0].clone();
        c.value = ip(99);
        t.devices[0].arp_ip.candidates.insert(0, c);
        seen.push(("arp_ip", diff::diff(&base, &t).map(|d| d.renamed)));
        // The management addresses.
        let mut t = s.clone();
        let mut c = t.devices[0].mgmt_ips.candidates[0].clone();
        c.value = [ip(99)].into_iter().collect();
        t.devices[0].mgmt_ips.candidates.insert(0, c);
        seen.push(("mgmt_ips", diff::diff(&base, &t).map(|d| d.renamed)));
        for (field, got) in &seen {
            match got {
                Err(e) => assert!(e.to_string().contains(&format!("'s {field} holds two values last heard in frame")), "{field}: {e}"),
                Ok(d) => panic!("{field}: a tie diffed: {d:?}"),
            }
        }
    }

    /// A conflict's value differing in case alone is another conflict: two devices sharing a name
    /// renamed alike in case, the duplicate cleared and another new.
    #[test]
    fn q08_a_conflict_whose_value_differs_in_case_alone_is_cleared_and_new() {
        use veredas_aranha::fusion::{Conflict, ConflictKind};
        let bm = Mac([0x02, 0, 0, 0, 0, 0x0B]);
        let b = scan_of(&[hello(A, Some("Bomba-1"), None, "eth1", 1, 1), hello(bm, Some("Bomba-1"), None, "eth1", 2, 2)], &["eth1"], &[]);
        let s = scan_of(&[hello(A, Some("bomba-1"), None, "eth1", 1, 1), hello(bm, Some("bomba-1"), None, "eth1", 2, 2)], &["eth1"], &[]);
        let d = diff::diff(&b, &s).unwrap();
        let dup = |v: &str| Conflict { kind: ConflictKind::DuplicateName, value: v.into(), macs: vec![A, bm] };
        assert_eq!(d.conflicts_cleared, [dup("Bomba-1")]);
        assert_eq!(d.conflicts_new, [dup("bomba-1")]);
    }

    /// Whatever the check lets through in the diff's own fields, one diff whatever the order of a
    /// field's values: a tie in each field, its list reversed.
    #[test]
    fn q09_a_tie_the_check_lets_through_gives_one_diff_in_either_order() {
        let s = scan_of(
            &[hello(A, Some("bomba-1"), Some(suite(ip(10))), "eth1", 1, 1), arp(A, ip(10), "eth1", 2, 2), lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 3, 3)],
            &["eth1"],
            &[],
        );
        let base = scan_of(&[hello(A, Some("bomba-1"), Some(suite(ip(10))), "eth1", 1, 1), arp(A, ip(10), "eth1", 2, 2), lldp(A, 1, "eth1", Some("sw-1"), &[ip(10)], 3, 3)], &["eth1"], &[]);
        let mut t = s.clone();
        let mut c = t.devices[0].ip.candidates[0].clone();
        c.value = suite(ip(99));
        t.devices[0].ip.candidates.push(c);
        let mut r = t.clone();
        r.devices[0].ip.candidates.reverse();
        if let (Ok(x), Ok(y)) = (diff::diff(&base, &t), diff::diff(&base, &r)) {
            assert_eq!(x, y, "one scan, two diffs by the order of its IP suite's values");
        }
    }
}

/// A change of what was heard is every kind but two: a value one scan alone states, and an
/// interface one scan alone listened on, are said and are no change (the plan: neither a change
/// nor silence), so a diff holding only those is no change, though it is not empty.
#[test]
fn one_sided_values_and_interfaces_alone_are_no_change() {
    let m = Mac([0x02, 0, 0, 0, 0, 0x0A]);
    let device = DeviceRef { mac: m, label: "a".into() };
    let change = Change { mac: m, what: Said::StationName, from: "a".into(), to: "b".into() };
    let one_sided = OneSided { mac: m, what: Said::ArpAddress, baseline: Some("192.168.0.10".into()), scan: None };
    let end = |node: &str, port: &str| End { node: node.into(), port: port.into() };
    let link = Link { a: end("dev:02:00:00:00:00:0a", "port-001"), b: end("if:eth1", ""), confidence: Confidence::Heard, witnesses: 1 };
    let conflict = Conflict { kind: ConflictKind::DuplicateName, value: "a".into(), macs: vec![m] };
    assert!(!Diff::default().has_change() && Diff::default().is_empty());
    let quiet = [
        Diff { one_sided: vec![one_sided.clone()], ..Diff::default() },
        Diff { interfaces_added: vec!["eth2".into()], ..Diff::default() },
        Diff { interfaces_removed: vec!["eth2".into()], ..Diff::default() },
        Diff { one_sided: vec![one_sided], interfaces_added: vec!["eth3".into()], interfaces_removed: vec!["eth2".into()], ..Diff::default() },
    ];
    for d in &quiet {
        assert!(!d.has_change() && !d.is_empty(), "{d:?}");
    }
    let changes = [
        Diff { appeared: vec![device.clone()], ..Diff::default() },
        Diff { vanished: vec![device], ..Diff::default() },
        Diff { renamed: vec![change.clone()], ..Diff::default() },
        Diff { readdressed: vec![Change { what: Said::IpSuite, ..change }], ..Diff::default() },
        Diff { links_added: vec![link.clone()], ..Diff::default() },
        Diff { links_removed: vec![link], ..Diff::default() },
        Diff { conflicts_new: vec![conflict.clone()], ..Diff::default() },
        Diff { conflicts_cleared: vec![conflict], ..Diff::default() },
    ];
    for d in &changes {
        assert!(d.has_change(), "{d:?}");
        // With one-sided values and interfaces beside it, still a change.
        let beside = Diff { interfaces_added: vec!["eth2".into()], ..d.clone() };
        assert!(beside.has_change(), "{beside:?}");
    }
}
