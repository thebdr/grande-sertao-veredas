//! The listener against the bench's PROFINET devices (C-143): maquinar's own executable, run on a
//! bench file as a plant's devices on the far ends of two veth pairs in a private network, the
//! listener on the near ends. What the devices state of themselves (their names, addresses, ids,
//! ports) is what the listener finds, each on its interface; what they share by mistake (one
//! station name) is a conflict; Wireshark's dissectors read every frame kept as the listener
//! classed it; a device leaving says so on its port.

#![cfg(target_os = "linux")]

mod common;

use std::collections::BTreeSet;
use std::io::Read;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use veredas_aranha::fusion::{ConflictKind, Device};
use veredas_aranha::listen::{Listener, Options, Snapshot};
use veredas_aranha::netns;
use veredas_aranha::observe::{self, Class, Observed, Probe};
use veredas_aranha::profinet::dcp::{self, Filter, IpStatus, IpSuite, Service};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::Wire;
use veredas_aranha::wire::raw::{Options as RawOptions, RawLink};

/// Where the bench's executable is built: a target folder of its own under the one cargo gives
/// integration tests, so building it never replaces the executable the bench's own tests run from
/// the workspace's target folder (a build of one package resolves features apart from the
/// workspace's, and cargo puts its executable in place again).
fn bench_target() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("bench-device/target")
}

/// The bench's executable, built from this workspace's sources (offline, eight jobs) on the host,
/// before the private network is made; in the private network, the same path.
fn maquinar() -> PathBuf {
    let exe = bench_target().join("debug/veredas-maquinar");
    if std::env::var_os(netns::INNER).is_some() {
        return exe;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO"))
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", bench_target())
        .args(["build", "--offline", "-j", "8", "-p", "veredas-maquinar", "--bin", "veredas-maquinar", "--message-format=json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "the bench does not build:\n{}", String::from_utf8_lossy(&out.stderr));
    let reported = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "veredas-maquinar")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from));
    assert_eq!(reported, Some(exe.clone()), "cargo names the bench's executable");
    exe
}

/// Two devices of one station name, each on its own segment, as a plant commissioned twice.
const BENCH: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "first"
interface = "b0"
station = "bomba-1"
vendor_id = 42
device_id = 261
ip = { address = "192.168.0.10", mask = "255.255.255.0", gateway = "192.168.0.1" }
lldp_interval_ms = 200
seed = 1

[[profinet]]
name = "second"
interface = "b1"
station = "bomba-1"
vendor = "segunda bancada"
ip = { address = "192.168.0.11", mask = "255.255.255.0" }
lldp_interval_ms = 200
port = "port-002"
seed = 2
"#;

/// An interface's MAC, as the kernel gives it to a raw link (the private network's `/sys` is the
/// host's, its interfaces not there).
fn mac_of(iface: &str) -> Mac {
    RawLink::open_with(iface, RawOptions::default()).unwrap().mac()
}

fn link(iface: &str) -> Box<dyn Wire> {
    Box::new(RawLink::open_with(iface, RawOptions { ethertype: None, promiscuous: false, outgoing: false }).unwrap())
}

fn device(s: &Snapshot, mac: Mac) -> &Device {
    s.fusion.devices.get(&mac).unwrap_or_else(|| panic!("no device {mac}: {:?}", s.fusion.devices.keys().collect::<Vec<_>>()))
}

/// The DCP service a kept frame's observation states, and whether it answers.
fn dcp_of(l: &Listener, index: u64) -> (Service, bool) {
    let kept = l.frame(index).unwrap_or_else(|| panic!("frame #{index} is not kept"));
    let decoded = observe::decode(&kept.frame, index);
    match &decoded.observations[..] {
        [o] => match &o.observed {
            Observed::Dcp { service, service_type, .. } => (*service, service_type.is_response()),
            other => panic!("frame #{index} is not DCP: {other:?}"),
        },
        other => panic!("frame #{index}: {other:?}"),
    }
}

/// End the bench as its user does, closing its stdin; its exit status and what it printed.
fn end(mut bench: Child) -> (std::process::ExitStatus, String, String) {
    drop(bench.stdin.take());
    let start = Instant::now();
    let status = loop {
        if let Some(s) = bench.try_wait().unwrap() {
            break s;
        }
        if start.elapsed() > Duration::from_secs(10) {
            let _ = bench.kill();
            panic!("the bench did not stop within 10 s of its stdin closing");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    let mut err = String::new();
    bench.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    bench.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    (status, out, err)
}

#[test]
fn the_listener_finds_the_bench_devices_as_they_state_themselves_each_on_its_interface() {
    let exe = maquinar();
    netns::isolated(
        "the_listener_finds_the_bench_devices_as_they_state_themselves_each_on_its_interface",
        &[("a0", "b0"), ("a1", "b1")],
        || {
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bench-device");
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("two-devices.toml");
            std::fs::write(&file, BENCH).unwrap();
            let (first, second) = (mac_of("b0"), mac_of("b1"));
            let (here0, here1) = (mac_of("a0"), mac_of("a1"));

            // Listening first: the Hellos the devices send as they start are heard.
            let l = Listener::start(vec![link("a0"), link("a1")], Options::default()).unwrap();
            let bench = Command::new(&exe).arg(&file).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();

            // Each device's Hello and LLDP, heard on its interface.
            let heard = |s: &Snapshot| {
                [first, second].iter().all(|m| {
                    s.fusion.devices.get(m).is_some_and(|d| d.heard.contains(&Probe::DcpHeard) && d.heard.contains(&Probe::LldpHeard))
                })
            };
            assert!(l.until(Duration::from_secs(10), heard), "{:#?}", l.snapshot());
            let s = l.snapshot();
            for (mac, iface) in [(first, "a0"), (second, "a1")] {
                let d = device(&s, mac);
                let (service, answers) = dcp_of(&l, d.name.candidates[0].first.frame);
                assert_eq!((service, answers), (Service::Hello, false), "{mac}: the first name heard is the Hello's");
                assert_eq!(d.heard_on, BTreeSet::from([iface.to_string()]), "{mac}");
            }

            // Asked by name on both interfaces, and for their addresses by ARP.
            // (Each interface asks from an address of its own: the dissectors read both segments'
            // frames as one capture, where one address from two MACs is a duplicate.)
            let here = Ipv4Addr::new(192, 168, 0, 1);
            let asks = [("a0", here0, Ipv4Addr::new(192, 168, 0, 2), Ipv4Addr::new(192, 168, 0, 10)), ("a1", here1, Ipv4Addr::new(192, 168, 0, 3), Ipv4Addr::new(192, 168, 0, 11))];
            for (iface, mac, from, ip) in asks {
                l.send(iface, dcp::identify_request(mac, 0x5EED, 1, &Filter::Name("bomba-1".into())).unwrap()).unwrap();
                l.send(iface, observe::arp_frame(mac, from, Mac::BROADCAST, ip, false).unwrap()).unwrap();
            }
            let answered = |s: &Snapshot| {
                [first, second].iter().all(|m| s.fusion.devices.get(m).is_some_and(|d| d.arp_ip.current().is_some() && d.name.candidates[0].count >= 2))
            };
            assert!(l.until(Duration::from_secs(10), answered), "{:#?}", l.snapshot());
            let s = l.snapshot();

            // What each device states of itself.
            let a = device(&s, first);
            let b = device(&s, second);
            for d in [a, b] {
                assert_eq!(d.label(), "bomba-1");
                assert_eq!(d.name.current(), Some(&"bomba-1".to_string()));
                assert!(!d.name.conflicted(), "{:?}", d.name);
                assert_eq!(d.role.current(), Some(&0x01), "an IO device");
                assert_eq!(d.ip_status.current(), Some(&IpStatus { set: 1, conflict: false }));
                assert_eq!(d.heard, BTreeSet::from([Probe::DcpHeard, Probe::LldpHeard, Probe::ArpHeard]));
                // The name's last witness is the Identify answer to the request sent.
                assert_eq!(dcp_of(&l, d.name.candidates[0].last.frame), (Service::Identify, true));
            }
            assert_eq!(a.ip.current(), Some(&IpSuite { ip: Ipv4Addr::new(192, 168, 0, 10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: here }));
            assert_eq!(b.ip.current(), Some(&IpSuite { ip: Ipv4Addr::new(192, 168, 0, 11), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }));
            assert_eq!(a.arp_ip.current(), Some(&Ipv4Addr::new(192, 168, 0, 10)));
            assert_eq!(b.arp_ip.current(), Some(&Ipv4Addr::new(192, 168, 0, 11)));
            assert_eq!(a.addresses(), BTreeSet::from([Ipv4Addr::new(192, 168, 0, 10)]));
            assert_eq!(a.pn_id.current(), Some(&(42, 261)));
            assert_eq!(b.pn_id.current(), Some(&(0, 0)));
            assert_eq!(a.vendor.current(), Some(&"veredas bench device".to_string()));
            assert_eq!(b.vendor.current(), Some(&"segunda bancada".to_string()));
            assert_eq!(a.ports.keys().collect::<Vec<_>>(), ["port-001"]);
            assert_eq!(b.ports.keys().collect::<Vec<_>>(), ["port-002"]);
            for (d, iface) in [(a, "a0"), (b, "a1")] {
                let port = d.ports.values().next().unwrap();
                assert_eq!(port.heard_on, BTreeSet::from([iface.to_string()]));
                assert_eq!(port.left_ns, None);
                assert_eq!(d.chassis.current(), Some(&"local:bomba-1".to_string()));
            }

            // One name on two devices: the conflict, and no other.
            let conflicts: Vec<_> = s.conflicts.iter().map(|c| (c.kind, c.value.as_str(), c.macs.clone())).collect();
            let mut both = vec![first, second];
            both.sort();
            assert_eq!(conflicts, [(ConflictKind::DuplicateName, "bomba-1", both)]);

            // The interfaces' accounts: two frames sent on each, nothing refused, nothing of this
            // machine's own heard, nothing damaged.
            for (iface, mac) in [("a0", here0), ("a1", here1)] {
                let i = &s.stats.interfaces[iface];
                assert_eq!((i.mac, i.sent, i.refused, i.queued, i.ended.clone()), (Some(mac), 2, 0, 0, None), "{iface}");
            }
            assert_eq!((s.stats.own, s.stats.damaged_frames, s.stats.damage.len()), (0, 0, 0), "{:?}", s.stats.damage);

            // The bench stopped as its user stops it: each device leaving says so on its port.
            let (status, out, err) = end(bench);
            assert!(status.success(), "the bench: {status}\n{out}\n{err}");
            assert!(out.contains("2 part(s) running"), "{out}");
            let left = |s: &Snapshot| [first, second].iter().all(|m| s.fusion.devices.get(m).is_some_and(|d| d.ports.values().all(|p| p.left_ns.is_some())));
            assert!(l.until(Duration::from_secs(5), left), "{:#?}", l.snapshot());
            let frames = l.frames();
            let s = l.stop();

            // Every frame kept, read by Wireshark's dissectors as the listener classed it, none
            // they complain of: the four sent from this machine's interfaces, each kept on its
            // own, the rest heard from the device on that interface's segment, counted by class
            // as the listener counted them.
            assert_eq!((s.kept.0, s.kept.1), (0, frames.len() as u64));
            let dissected = common::dissect(&frames.iter().map(|k| k.frame.bytes.clone()).collect::<Vec<_>>());
            let mut by_class = std::collections::BTreeMap::<Class, u64>::new();
            let mut sent = std::collections::BTreeMap::<String, Vec<Class>>::new();
            for (i, (k, d)) in frames.iter().zip(&dissected).enumerate() {
                assert_eq!(d.complaints(), Vec::<String>::new(), "#{i}");
                let class = observe::decode(&k.frame, i as u64).class;
                let proto = match class {
                    Class::Dcp => "pn_dcp",
                    Class::Lldp => "lldp",
                    Class::Arp => "arp",
                    other => panic!("#{i}: {other} on a segment of the bench's devices"),
                };
                assert!(d.has(proto), "#{i} {class}: {}", d.layers);
                let src = Mac::from_slice(&k.frame.bytes[6..12]).unwrap();
                let (ours, theirs) = if k.frame.iface == "a0" { (here0, first) } else { (here1, second) };
                if k.sent {
                    assert_eq!(src, ours, "#{i}");
                    sent.entry(k.frame.iface.clone()).or_default().push(class);
                } else {
                    assert_eq!(src, theirs, "#{i} on {}", k.frame.iface);
                    *by_class.entry(class).or_default() += 1;
                    if class == Class::Dcp {
                        assert_eq!(d.one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "bomba-1", "#{i}");
                    }
                }
            }
            // (Each interface's worker sends on its own: the order holds within an interface.)
            let asked = vec![Class::Dcp, Class::Arp];
            assert_eq!(sent, std::collections::BTreeMap::from([("a0".to_string(), asked.clone()), ("a1".to_string(), asked)]));
            assert_eq!(by_class, s.stats.by_class);
            assert_eq!(s.stats.heard, frames.len() as u64 - 4);
        },
    );
}
