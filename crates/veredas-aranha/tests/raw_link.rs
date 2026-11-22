//! The raw link and the private network a test makes for it (contract: the raw link clause).
//! Real frames on a real veth pair, in a user and network namespace of the test's own, with no
//! privilege on the host; and the refusals named where the privilege or the interface is missing.

#![cfg(target_os = "linux")]

use std::panic::{self, AssertUnwindSafe};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use veredas_aranha::netns;
use veredas_aranha::profinet::dcp::{self, Filter};
use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::raw::{RawError, RawLink, Received};

const A: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x0A, 0x01]);
const B: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x0B, 0x01]);

/// The next frame of this EtherType and direction the link hears within `within`.
fn heard(link: &RawLink, ethertype: u16, outgoing: bool, within: Duration) -> Option<Received> {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if let Some(r) = link.recv(end.saturating_duration_since(Instant::now())).unwrap() {
            let e = ether::parse(&r.bytes).unwrap();
            if e.ethertype == ethertype && r.outgoing == outgoing {
                return Some(r);
            }
        }
    }
    None
}

/// The two ends of the pair, each with its own MAC set first.
fn pair() -> (RawLink, RawLink) {
    for (iface, mac) in [("a0", A), ("b0", B)] {
        netns::ip(&["link", "set", iface, "down"]);
        netns::ip(&["link", "set", iface, "address", &mac.to_string()]);
        netns::ip(&["link", "set", iface, "up"]);
    }
    (RawLink::open("a0", false).unwrap(), RawLink::open("b0", false).unwrap())
}

fn flags(iface: &str) -> String {
    let out = Command::new("ip").args(["-o", "link", "show", iface]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.split('<').nth(1).and_then(|s| s.split('>').next()).unwrap_or_default().to_string()
}

/// The interface's promiscuity and all-multicast counts, as the kernel keeps them (`ip -d`): a
/// socket's membership counts there, not among the flags a person sets.
fn counts(iface: &str) -> (u32, u32) {
    let out = Command::new("ip").args(["-d", "-o", "link", "show", iface]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let after = |key: &str| -> u32 {
        let words: Vec<&str> = text.split_whitespace().collect();
        let i = words.iter().position(|w| *w == key).unwrap_or_else(|| panic!("{key} not in {text}"));
        words[i + 1].parse().unwrap()
    };
    (after("promiscuity"), after("allmulti"))
}

#[test]
fn frames_cross_a_veth_pair_whole_both_ways() {
    netns::isolated("frames_cross_a_veth_pair_whole_both_ways", &[("a0", "b0")], || {
        let (a, b) = pair();
        assert_eq!((a.iface(), a.mac(), b.mac()), ("a0", A, B));
        assert!(a.index() > 0 && b.index() > 0 && a.index() != b.index());

        // A DCP Identify-All from a0, heard whole on b0, and on a0 as its own outgoing frame.
        let before = SystemTime::now();
        let identify = dcp::identify_request(a.mac(), 0x0102_0304, 1, &Filter::All).unwrap();
        a.send(&identify).unwrap();
        let got = heard(&b, ether::ETHERTYPE_PROFINET, false, Duration::from_secs(2)).expect("b0 heard nothing");
        assert_eq!(got.bytes, identify);
        assert!(got.at >= before && got.at <= SystemTime::now(), "{:?}", got.at);
        // Another link on a0 sees the frame leave, marked outgoing; the sender does not hear its
        // own frame back (the kernel skips the socket that sent it).
        let watcher = RawLink::open("a0", false).unwrap();
        a.send(&identify).unwrap();
        let own = heard(&watcher, ether::ETHERTYPE_PROFINET, true, Duration::from_secs(2)).expect("a0's watcher saw nothing leave");
        assert_eq!(own.bytes, identify);
        assert!(heard(&a, ether::ETHERTYPE_PROFINET, true, Duration::from_millis(300)).is_none());
        assert_eq!(heard(&b, ether::ETHERTYPE_PROFINET, false, Duration::from_secs(2)).expect("b0 missed the second").bytes, identify);
        let pdu = dcp::decode(ether::parse(&got.bytes).unwrap().payload).unwrap();
        assert_eq!(pdu.xid, 0x0102_0304);

        // The other way: an LLDP frame to its group address, and an answer unicast to a0.
        let pdu = LldpPdu::new(ChassisId::Local { id: "station-b".into() }, PortId::Local { id: "port-001".into() }, 20);
        let announce = lldp::frame(b.mac(), &pdu).unwrap();
        b.send(&announce).unwrap();
        let got = heard(&a, ether::ETHERTYPE_LLDP, false, Duration::from_secs(2)).expect("a0 heard no LLDP");
        assert_eq!(got.bytes, announce);
        assert_eq!(lldp::decode(ether::parse(&got.bytes).unwrap().payload).unwrap(), pdu);
        let answer = dcp::set_response(a.mac(), b.mac(), 7, &[(2, 2, 0)]).unwrap();
        b.send(&answer).unwrap();
        assert_eq!(heard(&a, ether::ETHERTYPE_PROFINET, false, Duration::from_secs(2)).expect("no answer").bytes, answer);

        // The longest frame the pair carries crosses whole; one byte more is refused by name, as
        // is a frame shorter than its header.
        let longest = ether::build(B, A, 0x88B5, &[0x5A; 1500]).unwrap();
        a.send(&longest).unwrap();
        assert_eq!(heard(&b, 0x88B5, false, Duration::from_secs(2)).expect("no long frame").bytes, longest);
        let mut over = longest.clone();
        over.push(0);
        assert!(matches!(a.send(&over), Err(RawError::TooLong(1515))), "{:?}", a.send(&over));
        assert!(matches!(a.send(&[0; 13]), Err(RawError::TooShort(13))));

        // An idle link answers None at its timeout, neither sooner nor long after.
        while b.recv(Duration::from_millis(50)).unwrap().is_some() {}
        let start = Instant::now();
        assert_eq!(b.recv(Duration::from_millis(300)).unwrap(), None);
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(290) && waited < Duration::from_millis(1500), "{waited:?}");
    });
}

#[test]
fn the_link_joins_every_group_while_open_and_leaves_the_interface_as_it_was() {
    netns::isolated("the_link_joins_every_group_while_open_and_leaves_the_interface_as_it_was", &[("a0", "b0")], || {
        assert_eq!(counts("b0"), (0, 0));
        let plain = RawLink::open("b0", false).unwrap();
        assert_eq!(counts("b0"), (0, 1));
        let mirror = RawLink::open("b0", true).unwrap();
        assert_eq!(counts("b0"), (1, 2));
        drop(mirror);
        assert_eq!(counts("b0"), (0, 1));
        drop(plain);
        assert_eq!(counts("b0"), (0, 0));
        assert_eq!(counts("a0"), (0, 0), "the other end untouched");
    });
}

#[test]
fn a_missing_interface_and_a_bad_name_are_refused_by_name() {
    // Asked on the host: naming an interface needs no privilege.
    for (name, missing) in [("nope0", true), ("", false), ("a-name-of-16-ch", true), ("a-name-of-16-chr", false), ("a\0b", false)] {
        let err = RawLink::open(name, false).unwrap_err();
        match (missing, &err) {
            (true, RawError::NoSuchInterface(n)) | (false, RawError::BadName(n)) => assert_eq!(n, name),
            _ => panic!("{name:?}: {err:?}"),
        }
    }
    assert_eq!(RawError::NoSuchInterface("nope0".into()).to_string(), "no network interface is named \"nope0\"");
}

/// Whether this process holds CAP_NET_RAW (bit 13 of its effective set).
fn holds_net_raw() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let eff = status.lines().find_map(|l| l.strip_prefix("CapEff:")).unwrap().trim().to_string();
    u64::from_str_radix(&eff, 16).unwrap() & (1 << 13) != 0
}

#[test]
fn without_the_privilege_the_link_is_refused_naming_it() {
    assert!(!holds_net_raw(), "this test shows the refusal, so it runs without CAP_NET_RAW; this process holds it");
    let err = RawLink::open("lo", false).unwrap_err();
    assert!(matches!(&err, RawError::Privilege { iface, .. } if iface == "lo"), "{err:?}");
    let text = err.to_string();
    assert!(text.starts_with("opening a raw link on \"lo\" needs CAP_NET_RAW in its network namespace"), "{text}");
    assert!(text.ends_with("`setcap cap_net_raw+ep` on the executable, or run it in a namespace of its own"), "{text}");
}

fn message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

#[test]
fn a_body_that_never_ran_is_a_failure_not_a_pass() {
    let r = panic::catch_unwind(|| netns::isolated("no_test_has_this_name", &[], || {}));
    let msg = message(r.unwrap_err());
    assert!(msg.starts_with("no_test_has_this_name in its private network: the body never ran"), "{msg}");
}

#[test]
fn a_failing_body_fails_its_test_with_the_childs_words() {
    let r = panic::catch_unwind(AssertUnwindSafe(|| {
        netns::isolated("a_failing_body_fails_its_test_with_the_childs_words", &[], || panic!("deliberately red inside"))
    }));
    if std::env::var(netns::INNER).is_ok() {
        // The child: its failure is what the parent reads.
        panic::resume_unwind(r.unwrap_err());
    }
    let msg = message(r.unwrap_err());
    assert!(msg.contains("in its private network: the body failed"), "{msg}");
    assert!(msg.contains("deliberately red inside"), "{msg}");
}

#[test]
fn the_private_network_holds_only_what_the_test_made() {
    netns::isolated("the_private_network_holds_only_what_the_test_made", &[("p0", "q0"), ("p1", "q1")], || {
        let out = Command::new("ip").args(["-o", "link", "show"]).output().unwrap();
        let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split(": ").nth(1).map(|n| n.split('@').next().unwrap_or(n).to_string()))
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, ["lo", "p0", "p1", "q0", "q1"], "{names:?}");
        for n in ["lo", "p0", "q0", "p1", "q1"] {
            assert!(flags(n).contains("UP"), "{n}: {}", flags(n));
        }
        // Root here, and only here.
        // SAFETY: geteuid cannot fail.
        assert_eq!(unsafe { libc_geteuid() }, 0);
    });
    // And the host has none of them (this runs on the host alone: the child exits after its body).
    let out = Command::new("ip").args(["-o", "link", "show"]).output().unwrap();
    let host = String::from_utf8_lossy(&out.stdout);
    assert!(!host.contains(" p0@") && !host.contains(" q1@"), "{host}");
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
