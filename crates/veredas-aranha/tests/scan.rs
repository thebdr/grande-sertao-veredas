//! The scan file (contract: the scan file clause): a listening's counts, devices with every value
//! and its witnesses, conflicts and topology written as `veredas-scan` version 1 beside the
//! capture of its frames and read back whole; the header refused by core's reader, every key read
//! or the file refused; `docs/scan-schema.md`'s example is the file of the sample scan below.

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use veredas_aranha::listen::{Listener, Options};
use veredas_aranha::observe::{self, Observed};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpStatus, IpSuite};
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::scan::{Capture, FORMAT, Scan, ScanError, VERSION};
use veredas_aranha::topology::{self, Interface};
use veredas_aranha::wire::Frame;
use veredas_aranha::wire::pcap::{self, Replay};

const DEV: Mac = Mac([0x02, 0, 0, 0, 0, 0x01]);
const PORT: Mac = Mac([0x02, 0, 0, 0, 0x01, 0x01]);
const TWIN: Mac = Mac([0x02, 0, 0, 0, 0, 0x02]);
const T0: u64 = 1_764_000_000_000_000_000;

/// A device's Hello, its port's LLDP and its ARP, and a second device of its name: the frames of
/// the sample scan, a second apart.
fn frames() -> Vec<Frame> {
    let id = |ip| DeviceIdentity {
        name: Some("bomba-1".into()),
        ip: Some(IpSuite { ip, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
        ip_status: Some(IpStatus { set: 1, conflict: false }),
        vendor_id: Some(42),
        device_id: Some(261),
        ..Default::default()
    };
    let mut pdu = LldpPdu::new(ChassisId::Local { id: "bomba-1".into() }, PortId::Local { id: "port-001.bomba-1".into() }, 20);
    pdu.pn.chassis_mac = Some(DEV);
    let bytes = [
        dcp::hello(DEV, 1, &id(Ipv4Addr::new(192, 168, 0, 10))).unwrap(),
        lldp::frame(PORT, &pdu).unwrap(),
        observe::arp_frame(DEV, Ipv4Addr::new(192, 168, 0, 10), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap(),
        dcp::hello(TWIN, 1, &id(Ipv4Addr::new(192, 168, 0, 11))).unwrap(),
    ];
    bytes.into_iter().enumerate().map(|(i, b)| Frame::new(T0 + i as u64 * 1_000_000_000, "eth1", b)).collect()
}

/// The sample scan: its frames replayed through a listener as eth1, the topology of what it heard.
fn sample() -> Scan {
    let l = Listener::start(vec![Box::new(Replay::new("eth1", frames()))], Options::default()).unwrap();
    assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["eth1"].ended.is_some()));
    let s = l.stop();
    let t = topology::build(&s.fusion, &[Interface { name: "eth1".into(), mac: None }], &[]);
    Scan::new(&s.fusion, &s.stats, &t, T0 + 10_000_000_000, Some(Capture { file: "sample.pcap".into(), first_frame: 0, frames: 4 }))
}

/// The example `docs/scan-schema.md` gives: its one TOML block under "An example".
fn documented() -> String {
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scan-schema.md")).unwrap();
    let after = doc.split_once("## An example").expect("the schema has an example").1;
    let block = after.split_once("```toml\n").unwrap().1;
    block.split_once("```").unwrap().0.to_string()
}

#[test]
fn the_documented_example_is_the_sample_scan_s_file() {
    let scan = sample();
    let text = scan.to_toml().unwrap();
    assert_eq!(documented(), text, "docs/scan-schema.md's example is not the sample's file:\n{text}");
    assert!(text.starts_with(&format!("format = \"{FORMAT}\"\nversion = {VERSION}\n")));
    assert_eq!(Scan::read(&documented()).unwrap(), scan);
}

#[test]
fn a_scan_is_read_back_whole_and_its_witnesses_name_its_capture_s_frames() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scan");
    std::fs::create_dir_all(&dir).unwrap();
    let scan = sample();
    scan.write_file(&dir.join("sample.toml")).unwrap();
    pcap::write_file(&dir.join("sample.pcap"), &frames()).unwrap();
    let read = Scan::read_file(&dir.join("sample.toml")).unwrap();
    assert_eq!(read, scan);
    // What it found: two devices of one name, the conflict on record, DEV's port and its cable.
    let f = read.fusion();
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV, TWIN]);
    assert_eq!(read.conflicts, f.conflicts());
    assert_eq!(read.conflicts.len(), 1);
    assert_eq!(read.stats.heard, 4);
    assert!(read.topology.links.iter().any(|l| l.b.node == "if:eth1" && l.a.node == topology::device_id(DEV)));
    // Every name, chassis and ARP address witnessed by a record of the capture that says it.
    let capture = read.capture.clone().unwrap();
    let records = pcap::read_file(&dir.join(&capture.file), "eth1").unwrap().frames;
    assert_eq!(records.len() as u64, capture.frames);
    for d in &read.devices {
        let said = |frame: u64| observe::decode(&records[(frame - capture.first_frame) as usize], frame).observations;
        for c in &d.name.candidates {
            for w in [&c.first, &c.last] {
                assert!(said(w.frame).iter().any(|o| matches!(&o.observed, Observed::Dcp { identity, .. } if identity.name.as_ref() == Some(&c.value))), "{} {w:?}", c.value);
            }
        }
        for c in &d.arp_ip.candidates {
            assert!(said(c.first.frame).iter().any(|o| matches!(&o.observed, Observed::Arp { ip, .. } if *ip == c.value)));
        }
        for c in &d.chassis.candidates {
            assert!(said(c.first.frame).iter().any(|o| matches!(&o.observed, Observed::Lldp { pdu, .. } if pdu.chassis.key() == c.value)));
        }
    }
}

#[test]
fn a_file_of_another_format_or_version_or_with_a_key_nobody_reads_is_refused() {
    let text = sample().to_toml().unwrap();
    let refused = |t: &str| Scan::read(t).unwrap_err().to_string();
    // The header, by core's reader and its texts, before the body is looked at.
    assert_eq!(refused(&text.replacen("format = \"veredas-scan\"", "format = \"veredas-bench\"", 1)), "`format` is `veredas-bench`, not `veredas-scan`");
    assert_eq!(refused(&text.replacen("version = 1\n", "version = 2\n", 1)), "version 2 of `veredas-scan` is not known here (this reader knows 1)");
    assert_eq!(refused(&text.replacen("format = \"veredas-scan\"\n", "", 1)), "no `format`: a scan file states `format` and `version`");
    assert!(refused("format = [").starts_with("not a scan file: "));
    // A key nobody reads, at the top, in a device and in a witness: refused, each named.
    let extra = text.replacen("tool = ", "colour = \"red\"\ntool = ", 1);
    assert_eq!(refused(&extra), "a scan file holds no key `colour`: refused, not ignored");
    let in_device = text.replacen("by_source = false\n", "by_source = false\nnickname = \"pump\"\n", 1);
    assert!(matches!(Scan::read(&in_device), Err(ScanError::Unread(k)) if k.len() == 1 && k[0].contains("nickname")));
    let in_witness = text.replacen("frame = 0\n", "frame = 0\nseen_by = \"me\"\n", 1);
    assert!(matches!(Scan::read(&in_witness), Err(ScanError::Unread(k)) if k[0].contains("seen_by")));
    // A value of the wrong kind, and a key missing, are no scan.
    assert!(refused(&text.replacen("taken_ns = ", "taken_ns = \"soon\"\n#", 1)).starts_with("not a scan file: "));
    assert!(refused(&text.replacen("frames = 4\n", "", 1)).starts_with("not a scan file: "));
    // A stamp past what TOML's integers hold cannot be written, said.
    let mut far = sample();
    far.taken_ns = u64::MAX;
    assert!(matches!(far.to_toml(), Err(ScanError::Write(_))));
}

/// A scan with every field it can hold set: a device that said everything DCP and LLDP can say,
/// its port's PROFINET TLVs and its leaving, its ARP; a neighbour named by chassis alone and one
/// confirmed; an MRP ring; counts with damage, an interface that ended; a capture.
fn full() -> Scan {
    use veredas_aranha::fusion::Fusion;
    use veredas_aranha::listen::{Interface as Account, Stats};
    use veredas_aranha::observe::{Class, Observation, Probe, Witness};
    use veredas_aranha::profinet::lldp::{Delays, MacPhy, MgmtAddr, MrpPortStatus, PortStatus};
    use veredas_aranha::topology::Neighbour;
    let w = |probe, at_ns: u64, frame| Witness { probe, iface: "eth1".into(), at_ns: T0 + at_ns, frame };
    let identity = DeviceIdentity {
        name: Some("bomba-1".into()),
        alias: Some("port-001.switch-1".into()),
        ip: Some(IpSuite { ip: Ipv4Addr::new(192, 168, 0, 10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::new(192, 168, 0, 1) }),
        ip_status: Some(IpStatus { set: 1, conflict: true }),
        vendor_id: Some(42),
        device_id: Some(261),
        vendor: Some("bench io module".into()),
        role: Some(1),
        instance: Some((0, 1)),
        ..Default::default()
    };
    let mut pdu = LldpPdu::new(ChassisId::Local { id: "bomba-1".into() }, PortId::Local { id: "port-001.bomba-1".into() }, 20);
    pdu.pn.chassis_mac = Some(DEV);
    pdu.sys_name = Some("bomba-1".into());
    pdu.sys_desc = Some("bench".into());
    pdu.sys_caps = Some((0x0080, 0x0080));
    pdu.port_desc = Some("first port".into());
    pdu.port_vlan = Some(7);
    pdu.pn.port_status = Some(PortStatus { rtc2: 1, rtc3: 0 });
    pdu.pn.delays = Some(Delays { rx_local: 1, rx_remote: 2, tx_local: 3, tx_remote: 4, cable_local: 5 });
    pdu.pn.mrp = Some(MrpPortStatus { domain_uuid: [7; 16], mrrt: 2 });
    pdu.mac_phy = Some(MacPhy { autoneg: 3, pmd_capabilities: 0x6C00, mau_type: 16 });
    pdu.mgmt_addrs = vec![MgmtAddr { subtype: 1, addr: vec![10, 0, 0, 9], if_subtype: 2, if_number: 1, oid: vec![] }];
    let leaving = LldpPdu::new(ChassisId::Local { id: "bomba-1".into() }, PortId::Local { id: "port-001.bomba-1".into() }, 0);
    let obs = [
        Observation { witness: w(Probe::DcpHeard, 0, 0), observed: Observed::Dcp { mac: DEV, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } },
        Observation { witness: w(Probe::LldpHeard, 1, 1), observed: Observed::Lldp { src: PORT, pdu } },
        Observation { witness: w(Probe::ArpHeard, 2, 2), observed: Observed::Arp { mac: DEV, ip: Ipv4Addr::new(192, 168, 0, 10) } },
        Observation { witness: w(Probe::LldpHeard, 3, 3), observed: Observed::Lldp { src: PORT, pdu: leaving } },
        Observation { witness: w(Probe::DcpRequestHeard, 4, 4), observed: Observed::Dcp { mac: TWIN, service: dcp::Service::Identify, service_type: dcp::ServiceType::Request, identity: DeviceIdentity::default() } },
    ];
    let mut f = Fusion::new();
    for o in &obs {
        f.apply(o);
    }
    let stats = Stats {
        heard: 6,
        let_go: 1,
        by_class: [(Class::Dcp, 2), (Class::Lldp, 2), (Class::Arp, 1), (Class::Other(0x88B6), 1)].into_iter().collect(),
        observations: 5,
        own: 1,
        damage: ["#5 eth1: DCP: 3 bytes after its last block make none".to_string()].into_iter().collect(),
        damaged_frames: 1,
        interfaces: [("eth1".to_string(), Account { mac: Some(Mac([2, 0, 0, 0, 0, 0xE1])), heard: 6, sent: 1, refused: 1, queued: 0, unsent: 1, withdrawn: 1, ended: Some("its capture is read to the end".into()) })].into_iter().collect(),
        first_ns: Some(T0),
        last_ns: Some(T0 + 5),
    };
    let neighbours = [
        Neighbour { device: DEV, port: "port-002".into(), chassis: "dev-q".into(), peer_port: "port-001".into(), peer_mac: None, witnesses: 2 },
        Neighbour { device: DEV, port: "port-003".into(), chassis: "x".into(), peer_port: "port-009".into(), peer_mac: Some(TWIN), witnesses: 1 },
        Neighbour { device: TWIN, port: "port-009".into(), chassis: "bomba-1".into(), peer_port: "port-003".into(), peer_mac: Some(PORT), witnesses: 1 },
    ];
    let t = topology::build(&f, &[Interface { name: "eth1".into(), mac: Some(Mac([2, 0, 0, 0, 0, 0xE1])) }], &neighbours);
    Scan::new(&f, &stats, &t, T0 + 10, Some(Capture { file: "full.pcap".into(), first_frame: 0, frames: 6 }))
}

#[test]
fn a_full_scan_reads_back_equal_and_the_schema_names_every_key_it_holds() {
    let scan = full();
    let text = scan.to_toml().unwrap();
    assert_eq!(Scan::read(&text).unwrap(), scan);
    let d = &scan.devices[0];
    assert!(!d.alias.is_empty() && !d.sys_caps.is_empty() && !d.mgmt_ips.is_empty() && d.ports["port-001"].left_ns.is_some());
    assert!(!scan.topology.rings.is_empty() && !scan.topology.notes.is_empty());
    assert!(scan.topology.nodes.iter().any(|n| n.kind == topology::NodeKind::Chassis));
    assert!(scan.topology.links.iter().any(|l| l.confidence == topology::Confidence::Confirmed));
    // Every key the file holds is named in the schema's "The keys", in backticks; the keys of a
    // map by name (an interface's, a port's, a frame class, what a port was on each interface)
    // are its data, not its keys.
    fn keys(v: &toml::Value, by_name: bool, out: &mut std::collections::BTreeSet<String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, x) in t {
                    if !by_name {
                        out.insert(k.clone());
                    }
                    keys(x, matches!(k.as_str(), "interfaces" | "ports" | "by_class" | "on"), out);
                }
            }
            toml::Value::Array(a) => a.iter().for_each(|x| keys(x, false, out)),
            _ => {}
        }
    }
    let mut held = std::collections::BTreeSet::new();
    keys(&text.parse::<toml::Table>().map(toml::Value::Table).unwrap(), false, &mut held);
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scan-schema.md")).unwrap();
    let section = doc.split_once("## The keys").unwrap().1.split_once("## An example").unwrap().0;
    let unnamed: Vec<&String> = held.iter().filter(|k| !section.contains(&format!("`{k}`"))).collect();
    assert_eq!(unnamed, Vec::<&String>::new(), "keys the schema does not name");
}

#[test]
fn a_body_not_read_as_the_file_holds_it_is_refused() {
    let text = sample().to_toml().unwrap();
    let refused = |t: &str| Scan::read(t).map(|_| ()).unwrap_err().to_string();
    // An element past what its list takes; a table written as a list; a word written as a table;
    // a repeat a set keeps once: each refused, its place named.
    assert_eq!(refused(&text.replacen("value = [42, 261]", "value = [42, 261, 999]", 1)), "a scan file's `devices.0.pn_id.0.value` is not what its reader keeps: refused, not read otherwise");
    let capture = "[capture]\nfile = \"sample.pcap\"\nfirst_frame = 0\nframes = 4\n\n";
    assert!(text.contains(capture));
    let as_list = text.replacen(capture, "", 1).replacen("taken_ns = ", "capture = [\"sample.pcap\", 0, 4]\ntaken_ns = ", 1);
    assert_eq!(refused(&as_list), "a scan file's `capture` is not what its reader keeps: refused, not read otherwise");
    let longer = text.replacen(capture, "", 1).replacen("taken_ns = ", "capture = [\"sample.pcap\", 0, 4, \"extra\"]\ntaken_ns = ", 1);
    assert!(refused(&longer).contains("`capture`"), "{}", refused(&longer));
    assert!(refused(&text.replacen("probe = \"dcp-heard\"", "probe = { dcp-heard = {} }", 1)).ends_with("is not what its reader keeps: refused, not read otherwise"));
    assert!(refused(&text.replacen("heard_on = [\"eth1\"]", "heard_on = [\"eth1\", \"eth1\"]", 1)).ends_with("is not what its reader keeps: refused, not read otherwise"));
    // A device twice: read whole, but no scan.
    let first = text.split("[[devices]]").nth(1).unwrap();
    let twice = format!("{text}\n[[devices]]{first}");
    assert_eq!(refused(&twice), "not a scan: device 02:00:00:00:00:01 is out of the order by MAC");
    // A device repeated next to itself: refused when written.
    let mut doubled = sample();
    doubled.devices.insert(1, doubled.devices[0].clone());
    assert!(matches!(doubled.to_toml(), Err(ScanError::Write(m)) if m == "not a scan: device 02:00:00:00:00:01 is in it twice"));
    // A port's last frame edited past the capture, its witnesses inside it: no scan.
    let full_text = full().to_toml().unwrap();
    let spoke = full_text.lines().find(|l| l.starts_with("spoke = [")).unwrap().to_string();
    let edited = full_text.replacen(&spoke, &format!("{}, 99]", spoke.rsplit_once(',').unwrap().0), 1);
    assert_ne!(edited, full_text);
    assert_eq!(refused(&edited), "not a scan: device 02:00:00:00:00:01 names frame 99, past its capture's last (5)");
    // A number past TOML's integers, read where the toml crate would take it, and written in any
    // slot.
    let past = refused(&text.replacen("heard = 4\n", "heard = 18446744073709551615\n", 1));
    assert!(past.starts_with("not a scan file: TOML parse error"), "{past}");
    let mut far = sample();
    far.devices[0].frames = u64::MAX;
    assert!(matches!(far.to_toml(), Err(ScanError::Write(_))));
    far = sample();
    far.stats.heard = u64::MAX;
    assert!(matches!(far.to_toml(), Err(ScanError::Write(_))));
}

#[test]
fn a_key_nobody_reads_is_named_where_it_is() {
    let text = sample().to_toml().unwrap();
    let unread = |t: &str| match Scan::read(t) {
        Err(ScanError::Unread(k)) => k,
        other => panic!("{other:?}"),
    };
    assert_eq!(unread(&text.replacen("[devices.name.first]\n", "[devices.name.first]\nseen_by = \"me\"\n", 1)), ["`devices.0.name.0.first.seen_by`"]);
    assert_eq!(unread(&text.replacen("[devices.ports.port-001]\n", "[devices.ports.port-001]\ncolour = \"red\"\n", 1)), ["`devices.0.ports.port-001.colour`"]);
    assert_eq!(unread(&text.replacen("[stats.interfaces.eth1]\n", "[stats.interfaces.eth1]\nspeed = 100\n", 1)), ["`stats.interfaces.eth1.speed`"]);
    // Two at once, both named.
    let two = text.replacen("tool = ", "colour = \"red\"\nshade = \"dark\"\ntool = ", 1);
    assert_eq!(unread(&two), ["`colour`", "`shade`"]);
    // The header's other refusals, in core's texts.
    let refused = |t: &str| Scan::read(t).map(|_| ()).unwrap_err().to_string();
    assert_eq!(refused(&text.replacen("version = 1\n", "", 1)), "no `version`: a scan file states `format` and `version`");
    assert_eq!(refused(&text.replacen("format = \"veredas-scan\"", "format = 1", 1)), "not a scan file: `format` is not a string");
    assert_eq!(refused(&text.replacen("version = 1\n", "version = \"1\"\n", 1)), "not a scan file: `version` is not a whole number from 0 to 4294967295");
}

#[test]
fn witnesses_before_a_bounded_capture_are_frames_let_go_and_none_lies_past_it() {
    // A listening that kept two of its four frames: the capture holds those, the earlier witnesses
    // name frames the listener let go, and every witness from the capture's first on names the
    // record that says it.
    let l = Listener::start(vec![Box::new(Replay::new("eth1", frames()))], Options { max_frames: 2, ..Options::default() }).unwrap();
    assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["eth1"].ended.is_some()));
    let kept = l.frames();
    let (first, next) = l.kept();
    let s = l.stop();
    assert_eq!((first, next, s.stats.let_go), (2, 4, 2));
    let t = topology::build(&s.fusion, &[], &[]);
    let scan = Scan::new(&s.fusion, &s.stats, &t, T0 + 10, Some(Capture { file: "kept.pcap".into(), first_frame: first, frames: next - first }));
    let read = Scan::read(&scan.to_toml().unwrap()).unwrap();
    let records: Vec<Frame> = kept.into_iter().map(|k| k.frame).collect();
    let mut inside = 0;
    for d in &read.devices {
        for c in &d.name.candidates {
            for w in [&c.first, &c.last] {
                if w.frame >= first {
                    inside += 1;
                    let said = observe::decode(&records[(w.frame - first) as usize], w.frame).observations;
                    assert!(said.iter().any(|o| matches!(&o.observed, Observed::Dcp { identity, .. } if identity.name.as_ref() == Some(&c.value))));
                }
            }
        }
    }
    assert!(inside > 0);
    // A capture that ends before a witness: refused, written or read.
    let short = Scan { capture: Some(Capture { file: "kept.pcap".into(), first_frame: first, frames: 1 }), ..scan.clone() };
    assert!(matches!(short.to_toml(), Err(ScanError::Write(m)) if m.contains("past its capture's last (2)")), "{:?}", short.to_toml());
    let text = scan.to_toml().unwrap().replacen("frames = 2\n", "frames = 1\n", 1);
    assert!(Scan::read(&text).unwrap_err().to_string().starts_with("not a scan: device 02:00:00:00:00:02 names frame 3, past its capture's last (2)"), "{}", Scan::read(&text).unwrap_err());
}

/// A link is its two ends, so a scan stores each lesser end first, by their ends, each pair once:
/// otherwise one cable could be two links, or a diff depend on the order a list holds (C-147's
/// first refute round). Refused when written and when read.
#[test]
fn links_stored_otherwise_than_lesser_end_first_and_each_pair_once_are_no_scan() {
    let scan = full();
    assert!(scan.topology.links.len() >= 2);
    let written = |s: &Scan| toml::to_string(s).unwrap();
    let refused = |s: Scan, says: &str| {
        let e = s.to_toml().unwrap_err().to_string();
        assert!(e.starts_with("a scan cannot be written: not a scan: link ") && e.ends_with(says), "{e}");
        let e = Scan::read(&written(&s)).unwrap_err().to_string();
        assert!(e.starts_with("not a scan: link ") && e.ends_with(says), "{e}");
    };
    let mut flipped = scan.clone();
    let l = &mut flipped.topology.links[0];
    std::mem::swap(&mut l.a, &mut l.b);
    refused(flipped, "is stored greater end first");
    let mut twice = scan.clone();
    let again = twice.topology.links[0].clone();
    twice.topology.links.insert(1, again);
    refused(twice, "is in it twice");
    // Twice by its ends alone, its confidence and witnesses aside.
    let mut twice = scan.clone();
    let mut again = twice.topology.links[0].clone();
    again.witnesses += 1;
    twice.topology.links.insert(1, again);
    refused(twice, "is in it twice");
    let mut swapped = scan.clone();
    swapped.topology.links.swap(0, 1);
    refused(swapped, "is out of the order by its ends");
    // A link of one end twice (a device's word on its own port) is lesser end first.
    let mut own = scan.clone();
    let l = own.topology.links[0].clone();
    own.topology.links[0].b = l.a.clone();
    if own.topology.links.windows(2).all(|w| (&w[0].a, &w[0].b) < (&w[1].a, &w[1].b)) {
        assert!(own.to_toml().is_ok());
    }
}

/// One frame says one value of a field: a field holding two values last heard in one frame is no
/// scan (the value heard last would be the list's order), written or read (C-147's second refute
/// round); and a link between two ports of one device is stored lesser end first like any.
#[test]
fn two_values_of_a_field_from_one_frame_and_a_device_s_own_link_flipped_are_no_scan() {
    use veredas_aranha::topology::{Confidence, End, Link};
    let scan = full();
    let written = |s: &Scan| toml::to_string(s).unwrap();
    let mut tie = scan.clone();
    let mut again = tie.devices[0].name.candidates[0].clone();
    again.value = format!("{}-other", again.value);
    let frame = again.last.frame;
    tie.devices[0].name.candidates.push(again);
    let says = format!("device {}'s name holds two values last heard in frame {frame}", tie.devices[0].mac);
    assert_eq!(tie.to_toml().unwrap_err().to_string(), format!("a scan cannot be written: not a scan: {says}"));
    assert_eq!(Scan::read(&written(&tie)).unwrap_err().to_string(), format!("not a scan: {says}"));
    // A port's field too.
    let mut tie = scan.clone();
    let port = tie.devices[0].ports.values_mut().next().unwrap();
    let mut again = port.port_mac.candidates[0].clone();
    again.value = veredas_aranha::profinet::mac::Mac([0x02, 0, 0, 0, 0x99, 0x99]);
    let (id, frame) = (port.id.clone(), again.last.frame);
    port.port_mac.candidates.push(again);
    assert_eq!(tie.check().unwrap_err().to_string(), format!("not a scan: device {}'s port {id}'s port_mac holds two values last heard in frame {frame}", tie.devices[0].mac));
    // A device's word on its own port 3 as its port 4's neighbour, stored port 4 first: refused.
    let dev = veredas_aranha::topology::device_id(scan.devices[0].mac);
    let end = |p: &str| End { node: dev.clone(), port: p.into() };
    let mut own = scan.clone();
    own.topology.links = vec![Link { a: end("port-004"), b: end("port-003"), confidence: Confidence::Reported, witnesses: 1 }];
    assert!(own.check().unwrap_err().to_string().ends_with("is stored greater end first"));
    own.topology.links = vec![Link { a: end("port-003"), b: end("port-004"), confidence: Confidence::Reported, witnesses: 1 }];
    assert!(own.check().is_ok());
}

/// A port's last frame on an interface is a frame the scan names like any (C-147's third refute
/// round): past its capture's last, no scan.
#[test]
fn a_port_s_last_frame_on_an_interface_past_the_capture_is_no_scan() {
    let mut scan = sample();
    assert!(scan.check().is_ok());
    let dev = scan.devices.iter().position(|d| !d.ports.is_empty()).unwrap();
    let mac = scan.devices[dev].mac;
    let port = scan.devices[dev].ports.values_mut().next().unwrap();
    port.on.values_mut().next().unwrap().last.1 = 900;
    assert_eq!(scan.check().unwrap_err().to_string(), format!("not a scan: device {mac} names frame 900, past its capture's last (3)"));
}

/// The second refute round's cases: another stamp and a class its reader does not take back
/// refused when written; a misspelt key named, not only the key it stood for; each of the 21 fields
/// holding two values last heard in one frame refused, and each one's first and last witness, and a
/// port's leaving, at the capture's last record taken and one past it refused, written and read
/// alike; a MAC in another spelling and a set out of its order refused where they sit; the devices
/// giving the listener's fusion again.
mod round_2 {
    use super::{DEV, T0, frames, full, sample};
    use std::time::Duration;
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::observe::Class;
    use veredas_aranha::scan::{Capture, Scan, ScanError};
    use veredas_aranha::topology::{self, Interface};
    use veredas_aranha::wire::pcap::Replay;

    /// The 21 fields of the full scan's device and its port, by number: the name the check gives
    /// each, and whether it is a port's.
    const FIELDS: [(&str, bool); 21] = [
        ("name", false),
        ("alias", false),
        ("ip", false),
        ("ip_status", false),
        ("pn_id", false),
        ("vendor", false),
        ("role", false),
        ("instance", false),
        ("chassis", false),
        ("sys_name", false),
        ("sys_desc", false),
        ("sys_caps", false),
        ("mgmt_ips", false),
        ("arp_ip", false),
        ("port_mac", true),
        ("desc", true),
        ("pn_status", true),
        ("delays", true),
        ("mrp", true),
        ("pvid", true),
        ("mau_type", true),
    ];

    /// Field `$i` of the scan's first device (its first port's for a port's), as `$f`, to `$body`.
    macro_rules! field {
        ($scan:expr, $i:expr, |$f:ident| $body:expr) => {{
            let d = &mut $scan.devices[0];
            let p = d.ports.values_mut().next().unwrap();
            match $i {
                0 => { let $f = &mut d.name; $body }
                1 => { let $f = &mut d.alias; $body }
                2 => { let $f = &mut d.ip; $body }
                3 => { let $f = &mut d.ip_status; $body }
                4 => { let $f = &mut d.pn_id; $body }
                5 => { let $f = &mut d.vendor; $body }
                6 => { let $f = &mut d.role; $body }
                7 => { let $f = &mut d.instance; $body }
                8 => { let $f = &mut d.chassis; $body }
                9 => { let $f = &mut d.sys_name; $body }
                10 => { let $f = &mut d.sys_desc; $body }
                11 => { let $f = &mut d.sys_caps; $body }
                12 => { let $f = &mut d.mgmt_ips; $body }
                13 => { let $f = &mut d.arp_ip; $body }
                14 => { let $f = &mut p.port_mac; $body }
                15 => { let $f = &mut p.desc; $body }
                16 => { let $f = &mut p.pn_status; $body }
                17 => { let $f = &mut p.delays; $body }
                18 => { let $f = &mut p.mrp; $body }
                19 => { let $f = &mut p.pvid; $body }
                _ => { let $f = &mut p.mau_type; $body }
            }
        }};
    }

    /// What the check says of a scan, written (to_toml) and read (its text as serde writes it).
    fn refusals(s: &Scan) -> (Option<String>, Option<String>) {
        let written = s.to_toml().err().map(|e| match e {
            ScanError::Write(why) => why,
            other => format!("not a write refusal: {other}"),
        });
        let read = Scan::read(&toml::to_string(s).unwrap()).err().map(|e| e.to_string());
        (written, read)
    }

    /// Another stamp, and a count by a type whose class reads back as another, are refused when
    /// written, never written for the reader to refuse.
    #[test]
    fn another_stamp_or_a_class_read_back_otherwise_is_refused_when_written() {
        for (what, change, says) in [
            ("version 2", (|s: &mut Scan| s.version = 2) as fn(&mut Scan), "its stamp is `veredas-scan` version 2, not `veredas-scan` version 1"),
            ("version 0", |s: &mut Scan| s.version = 0, "its stamp is `veredas-scan` version 0, not `veredas-scan` version 1"),
            ("another format", |s: &mut Scan| s.format = "veredas-bench".into(), "its stamp is `veredas-bench` version 1, not `veredas-scan` version 1"),
            ("LLDP's type as another", |s: &mut Scan| _ = s.stats.by_class.insert(Class::Other(0x88cc), 1), "its count of frames `other:0x88cc` names a type that has a class of its own"),
        ] {
            let mut s = sample();
            change(&mut s);
            assert_eq!(s.to_toml().map(|_| ()).map_err(|e| e.to_string()), Err(format!("a scan cannot be written: not a scan: {says}")), "{what}");
        }
        // A type with no class of its own is counted by it, written and read back.
        let mut s = sample();
        s.stats.by_class.insert(Class::Other(0x88b6), 1);
        assert_eq!(Scan::read(&s.to_toml().unwrap()).unwrap(), s);
    }

    /// A misspelt key is named, where it stood for a key the reader misses as much as beside one;
    /// a key inside the capture by its place in the file.
    #[test]
    fn a_misspelt_key_is_named_not_only_the_key_it_stood_for() {
        let text = sample().to_toml().unwrap();
        for (from, to, named) in [
            ("tool = ", "Tool = ", "`Tool`"),
            ("first_frame = 0", "firstframe = 0", "`capture.firstframe`"),
            ("\nframe = 0\n", "\nfrme = 0\n", "`devices.0.name.0.first.frme`"),
            ("first_frame = 0\n", "first_frame = 0\ncolour = \"red\"\n", "`capture.colour`"),
        ] {
            let edited = text.replacen(from, to, 1);
            assert_ne!(edited, text, "{to}");
            match Scan::read(&edited) {
                Err(ScanError::Unread(k)) => assert_eq!(k, [named], "{to}"),
                other => panic!("{to}: {other:?}"),
            }
        }
    }

    /// Each of the 21 fields holding two values last heard in one frame: refused, naming the field
    /// and the frame, written and read alike.
    #[test]
    fn each_field_holding_two_values_last_heard_in_one_frame_is_no_scan() {
        let s = full();
        let port = s.devices[0].ports.values().next().unwrap().id.clone();
        for (i, (name, of_port)) in FIELDS.iter().enumerate() {
            let mut x = s.clone();
            let frame = field!(x, i, |f| {
                let c = f.candidates.first().cloned().unwrap_or_else(|| panic!("{name}: the full scan holds no value of it"));
                let frame = c.last.frame;
                f.candidates.push(c);
                frame
            });
            let whose = if *of_port { format!("port {port}'s {name}") } else { name.to_string() };
            let says = format!("not a scan: device {DEV}'s {whose} holds two values last heard in frame {frame}");
            assert_eq!(refusals(&x), (Some(says.clone()), Some(says)), "{name}");
        }
    }

    /// Each field's first and last witness, and a port's leaving, at the capture's last record:
    /// taken; one past it: refused, naming the device and the frame, written and read alike.
    #[test]
    fn each_witness_and_a_leaving_at_the_capture_s_last_taken_and_one_past_refused() {
        let s = full();
        let c = s.capture.clone().unwrap();
        let last = c.first_frame + c.frames - 1;
        let past = format!("not a scan: device {DEV} names frame {}, past its capture's last ({last})", last + 1);
        for (i, (name, _)) in FIELDS.iter().enumerate() {
            for which in ["first", "last"] {
                for (frame, refused) in [(last, None), (last + 1, Some(past.clone()))] {
                    let mut x = s.clone();
                    field!(x, i, |f| {
                        let c = &mut f.candidates[0];
                        if which == "first" { c.first.frame = frame } else { c.last.frame = frame }
                    });
                    assert_eq!(refusals(&x), (refused.clone(), refused), "{name}'s {which} at {frame}");
                }
            }
        }
        for (frame, refused) in [(last, None), (last + 1, Some(past.clone()))] {
            let mut x = s.clone();
            x.devices[0].ports.values_mut().next().unwrap().leaving = Some((T0, frame));
            assert_eq!(refusals(&x), (refused.clone(), refused), "a leaving at {frame}");
        }
    }

    /// A MAC in another spelling, and a set out of its order, are refused where they sit.
    #[test]
    fn a_mac_spelt_otherwise_and_a_set_out_of_order_are_refused_where_they_sit() {
        let text = sample().to_toml().unwrap();
        for (from, to, at) in [
            ("mac = \"02:00:00:00:00:01\"", "mac = \"02-00-00-00-00-01\"", "`devices.0.mac`"),
            ("macs = [\"02:00:00:00:00:01\"", "macs = [\"02-00-00-00-00-01\"", "`conflicts.0.macs.0`"),
            ("heard = [\"dcp-heard\", \"lldp-heard\", \"arp-heard\"]", "heard = [\"lldp-heard\", \"dcp-heard\", \"arp-heard\"]", "`devices.0.heard.0`"),
        ] {
            let edited = text.replacen(from, to, 1);
            assert_ne!(edited, text, "{to}");
            assert_eq!(Scan::read(&edited).map(|_| ()).map_err(|e| e.to_string()), Err(format!("a scan file's {at} is not what its reader keeps: refused, not read otherwise")), "{to}");
        }
        let full_text = full().to_toml().unwrap();
        let upper = full_text.replacen("mac = \"02:00:00:00:00:e1\"", "mac = \"02:00:00:00:00:E1\"", 1);
        assert_ne!(upper, full_text);
        assert_eq!(Scan::read(&upper).map(|_| ()).map_err(|e| e.to_string()), Err("a scan file's `stats.interfaces.eth1.mac` is not what its reader keeps: refused, not read otherwise".to_string()));
    }

    /// A scan's devices give the listener's fusion again, every port and value with them, written
    /// and read back.
    #[test]
    fn a_scan_s_devices_give_the_listener_s_fusion_again() {
        let l = Listener::start(vec![Box::new(Replay::new("eth1", frames()))], Options::default()).unwrap();
        assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["eth1"].ended.is_some()));
        let s = l.stop();
        let t = topology::build(&s.fusion, &[Interface { name: "eth1".into(), mac: None }], &[]);
        let scan = Scan::new(&s.fusion, &s.stats, &t, T0 + 10_000_000_000, Some(Capture { file: "sample.pcap".into(), first_frame: 0, frames: 4 }));
        assert_eq!(Scan::read(&scan.to_toml().unwrap()).unwrap().fusion(), s.fusion);
        assert!(s.fusion.devices.values().any(|d| !d.ports.is_empty()), "a device with a port among them");
    }
}
