//! The listener against the bench's PROFINET devices (C-143): maquinar's own executable, run on a
//! bench file as a plant's devices on the far ends of two veth pairs in a private network, the
//! listener on the near ends. What the devices state of themselves (their names, addresses, ids,
//! ports) is what the listener finds, each on its interface; what they share by mistake (one
//! station name) is a conflict; Wireshark's dissectors read every frame kept as the listener
//! classed it; a device leaving says so on its port.

#![cfg(target_os = "linux")]

mod bench;
mod common;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use veredas_aranha::fusion::{ConflictKind, Device};
use veredas_aranha::listen::{Listener, Options, Snapshot};
use veredas_aranha::netns;
use veredas_aranha::observe::{self, Class, Observed, Probe};
use veredas_aranha::profinet::dcp::{self, Filter, IpStatus, IpSuite, Service};
use veredas_aranha::profinet::mac::Mac;
use bench::{link, mac_of};

const FIRST: Mac = Mac([0x02, 0, 0, 0, 0, 0xB0]);
const SECOND: Mac = Mac([0x02, 0, 0, 0, 0, 0xB1]);
const HERE0: Mac = Mac([0x02, 0, 0, 0, 0, 0xA0]);
const HERE1: Mac = Mac([0x02, 0, 0, 0, 0, 0xA1]);

/// What each device states, as the dissectors print it: vendor and device ids, address, mask,
/// gateway, vendor text, and its port's id.
struct Stated {
    ids: (&'static str, &'static str),
    ip: (&'static str, &'static str, &'static str),
    vendor: &'static str,
    port: &'static str,
}

const STATED: [Stated; 2] = [
    Stated { ids: ("42", "261"), ip: ("192.168.0.10", "255.255.255.0", "192.168.0.1"), vendor: "veredas bench device", port: "port-001.bomba-1" },
    Stated { ids: ("0", "0"), ip: ("192.168.0.11", "255.255.255.0", "0.0.0.0"), vendor: "segunda bancada", port: "port-002.bomba-1" },
];

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

#[test]
fn the_listener_finds_the_bench_devices_as_they_state_themselves_each_on_its_interface() {
    // Built on the host, before the private network is made.
    bench::maquinar();
    netns::isolated(
        "the_listener_finds_the_bench_devices_as_they_state_themselves_each_on_its_interface",
        &[("a0", "b0"), ("a1", "b1")],
        || {
            // The four MACs the test chose, set before anything opens an interface.
            let (first, second, here0, here1) = (FIRST, SECOND, HERE0, HERE1);
            bench::pin(&[("b0", first), ("b1", second), ("a0", here0), ("a1", here1)], &[("a0", "b0"), ("a1", "b1")]);
            assert_eq!([mac_of("b0"), mac_of("b1"), mac_of("a0"), mac_of("a1")], [first, second, here0, here1]);

            // Listening first: the Hellos the devices send as they start are heard.
            let l = Listener::start(vec![link("a0"), link("a1")], Options::default()).unwrap();
            let bench = bench::start("two-devices", BENCH);

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
            let (status, out, err) = bench::end(bench);
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
                let stated = &STATED[usize::from(k.frame.iface == "a1")];
                if k.sent {
                    assert_eq!(src, ours, "#{i}");
                    sent.entry(k.frame.iface.clone()).or_default().push(class);
                    // The Identify asks by the name, read so by the dissectors.
                    if class == Class::Dcp {
                        assert_eq!(d.one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "bomba-1", "#{i}");
                        assert!(d.values("pn_dcp", "pn_dcp.option").iter().all(|o| o != "255"), "#{i}: no all-selector");
                    }
                } else {
                    assert_eq!(src, theirs, "#{i} on {}", k.frame.iface);
                    *by_class.entry(class).or_default() += 1;
                    // What the device states, read by the dissectors against the bench file, not
                    // through the codecs the bench and the listener share.
                    if class == Class::Dcp {
                        let p = "pn_dcp";
                        assert_eq!(d.one(p, "pn_dcp.suboption_device_nameofstation"), "bomba-1", "#{i}");
                        assert_eq!((d.one(p, "pn_dcp.suboption_vendor_id"), d.one(p, "pn_dcp.suboption_device_id")), (stated.ids.0.to_string(), stated.ids.1.to_string()), "#{i}");
                        let ip = (d.one(p, "pn_dcp.suboption_ip_ip"), d.one(p, "pn_dcp.suboption_ip_subnetmask"), d.one(p, "pn_dcp.suboption_ip_standard_gateway"));
                        assert_eq!(ip, (stated.ip.0.to_string(), stated.ip.1.to_string(), stated.ip.2.to_string()), "#{i}");
                        assert_eq!((d.one(p, "pn_dcp.suboption_device_devicevendorvalue"), d.one(p, "pn_dcp.suboption_device_role")), (stated.vendor.to_string(), "1".to_string()), "#{i}");
                    }
                    if class == Class::Lldp {
                        let hex: Vec<String> = "bomba-1".bytes().map(|b| format!("{b:02x}")).collect();
                        assert_eq!((d.one("lldp", "lldp.chassis.subtype"), d.one("lldp", "lldp.chassis.id")), ("7".to_string(), hex.join(":")), "#{i}");
                        assert_eq!(d.one("lldp", "lldp.port.id"), stated.port, "#{i}");
                    }
                }
            }
            // (Each interface's worker sends on its own: the order holds within an interface.)
            let asked = vec![Class::Dcp, Class::Arp];
            assert_eq!(sent, std::collections::BTreeMap::from([("a0".to_string(), asked.clone()), ("a1".to_string(), asked)]));
            assert_eq!(by_class, s.stats.by_class);
            assert_eq!(s.stats.heard, frames.len() as u64 - 4);
            // Nothing of this machine's own and nothing damaged, the leaving frames among them.
            assert_eq!((s.stats.own, s.stats.damaged_frames, s.stats.damage.len()), (0, 0, 0), "{:?}", s.stats.damage);
        },
    );
}
