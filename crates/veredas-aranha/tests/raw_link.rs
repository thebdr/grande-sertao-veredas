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
use veredas_aranha::profinet::ether::VlanTag;
use veredas_aranha::wire::raw::{Options, RawError, RawLink, Received};

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
    netns::wait_pairs(&[("a0", "b0")]);
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
        // The time a frame was read, not the time the wait began: one sent 400 ms into a wait.
        let late = RawLink::open("a0", false).unwrap();
        let frame = identify.clone();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let sent = SystemTime::now();
            late.send(&frame).unwrap();
            sent
        });
        let got = heard(&b, ether::ETHERTYPE_PROFINET, false, Duration::from_secs(2)).expect("b0 missed the late frame");
        let sent = sender.join().unwrap();
        assert!(got.at >= sent && got.at <= SystemTime::now(), "read at {:?}, sent at {sent:?}", got.at);

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
fn a_tagged_frame_is_heard_with_its_tag_where_the_kernel_took_it_out() {
    netns::isolated("a_tagged_frame_is_heard_with_its_tag_where_the_kernel_took_it_out", &[("a0", "b0")], || {
        let (a, b) = pair();
        let cyclic = ether::build_tagged(B, A, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x8892, &[0x80, 0x00, 1, 2]).unwrap();
        let tagged = ether::build_tagged(B, A, Some(VlanTag { priority: 3, drop_eligible: true, id: 5 }), 0x88B5, &[0x5A; 1500]).unwrap();
        for f in [&cyclic, &tagged] {
            a.send(f).unwrap();
            let got = heard(&b, ether::parse(f).unwrap().ethertype, false, Duration::from_secs(2)).expect("nothing heard");
            assert_eq!(got.bytes, *f);
            assert_eq!(ether::parse(&got.bytes).unwrap().vlan, ether::parse(f).unwrap().vlan);
        }
        assert_eq!(tagged.len(), 1518);
    });
}

#[test]
fn a_link_hears_one_ethertype_and_can_leave_out_what_this_host_sends() {
    netns::isolated("a_link_hears_one_ethertype_and_can_leave_out_what_this_host_sends", &[("a0", "b0")], || {
        let (a, _b) = pair();
        let only = |ethertype| RawLink::open_with("b0", Options { ethertype: Some(ethertype), ..Options::default() }).unwrap();
        let (ethercat, profinet) = (only(0x88A4), only(0x8892));
        let lldp_frame = lldp::frame(A, &LldpPdu::new(ChassisId::Local { id: "a".into() }, PortId::Local { id: "p".into() }, 20)).unwrap();
        let ec = ether::build(Mac::BROADCAST, A, 0x88A4, &[0x0D, 0x10, 0, 0]).unwrap();
        let rt = ether::build_tagged(B, A, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x8892, &[0x80, 0x00]).unwrap();
        for f in [&lldp_frame, &ec, &rt] {
            a.send(f).unwrap();
        }
        // Each hears its own type and nothing else; the tagged frame is matched by the type inside
        // its tag, and heard with the tag.
        assert_eq!(ethercat.recv(Duration::from_secs(2)).unwrap().map(|r| r.bytes), Some(ec.clone()));
        assert_eq!(ethercat.recv(Duration::from_millis(300)).unwrap(), None);
        assert_eq!(profinet.recv(Duration::from_secs(2)).unwrap().map(|r| r.bytes), Some(rt.clone()));
        assert_eq!(profinet.recv(Duration::from_millis(300)).unwrap(), None);

        // On the sending end: a watcher that takes outgoing frames sees a0's, one that leaves them
        // out sees none, and still hears what comes in.
        let sees = RawLink::open_with("a0", Options::default()).unwrap();
        let blind = RawLink::open_with("a0", Options { outgoing: false, ..Options::default() }).unwrap();
        a.send(&ec).unwrap();
        assert_eq!(heard(&sees, 0x88A4, true, Duration::from_secs(2)).map(|r| r.bytes), Some(ec.clone()));
        let start = Instant::now();
        assert_eq!(blind.recv(Duration::from_millis(300)).unwrap(), None);
        assert!(start.elapsed() >= Duration::from_millis(290), "{:?}", start.elapsed());
        let answer = ether::build(A, B, 0x88A4, &[0x0D, 0x10, 1, 0]).unwrap();
        _b.send(&answer).unwrap();
        assert_eq!(blind.recv(Duration::from_secs(2)).unwrap().map(|r| (r.bytes, r.outgoing)), Some((answer, false)));
        assert_eq!(Options::default(), Options { ethertype: None, promiscuous: false, outgoing: true });
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
    let names = [
        ("nope0", true),
        ("a-name-of-16-ch", true),
        ("", false),
        ("a-name-of-16-chr", false),
        ("a\0b", false),
        ("a/b", false),
        (".", false),
        ("..", false),
        ("a b", false),
        ("a:b", false),
        ("a\tb", false),
        ("a\nb", false),
        ("a\u{b}b", false),
        ("a\u{c}b", false),
        ("a\rb", false),
        // The kernel reads white space byte by byte: 0xA0 is one (the second byte of `à`).
        ("à0", false),
        ("a\u{a0}b", false),
    ];
    assert_eq!(names.iter().filter(|(_, missing)| !missing).count(), 15);
    for (name, missing) in names {
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
    if std::env::var(netns::INNER).is_ok() {
        // The child passes the call over: its body never runs.
        return;
    }
    let r = panic::catch_unwind(|| netns::isolated("a_body_that_never_ran_is_a_failure_not_a_pass", &[], || {}));
    let msg = message(r.unwrap_err());
    assert!(msg.starts_with("a_body_that_never_ran_is_a_failure_not_a_pass in its private network: the body never ran"), "{msg}");
}

#[test]
fn a_test_passes_its_own_name_and_makes_one_private_network() {
    // Another test's name would run that test's body, not this one's.
    let r = panic::catch_unwind(|| netns::isolated("frames_cross_a_veth_pair_whole_both_ways", &[], || panic!("not this body")));
    let msg = message(r.unwrap_err());
    assert_eq!(
        msg,
        "netns::isolated(\"frames_cross_a_veth_pair_whole_both_ways\") was called from the test Some(\"a_test_passes_its_own_name_and_makes_one_private_network\"): a test passes its own name"
    );
}

#[test]
fn a_second_private_network_in_one_test_is_refused() {
    netns::isolated("a_second_private_network_in_one_test_is_refused", &[], || {});
    let r = panic::catch_unwind(|| netns::isolated("a_second_private_network_in_one_test_is_refused", &[("x0", "y0")], || panic!("the second body")));
    assert_eq!(message(r.unwrap_err()), "a_second_private_network_in_one_test_is_refused asked for a second private network: one per test");
}

#[test]
fn where_the_namespace_cannot_be_made_the_failure_says_why() {
    netns::isolated("where_the_namespace_cannot_be_made_the_failure_says_why", &[], || {
        // Inside, as the namespace's root, forbid any further user namespace, then ask for one.
        std::fs::write("/proc/sys/user/max_user_namespaces", "0").unwrap();
        let why = netns::run_child("any_test").unwrap_err();
        assert!(why.starts_with("the private network for any_test could not be made: "), "{why}");
        assert!(why.contains("user.max_user_namespaces = 0"), "{why}");
        assert!(why.contains("kernel.apparmor_restrict_unprivileged_userns"), "{why}");
    });
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
        // The fixture waited for every end's carrier, and for a frame to cross each pair each way,
        // before the body (the first frame's test below runs the fixture twenty times).
        for n in ["p0", "q0", "p1", "q1"] {
            assert!(flags(n).split(',').any(|f| f == "LOWER_UP"), "{n}: {}", flags(n));
        }
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
        // IPv6 is off, so the links carry only what the test sends.
        for knob in ["all", "default", "p0", "q1", "lo"] {
            assert_eq!(std::fs::read_to_string(format!("/proc/sys/net/ipv6/conf/{knob}/disable_ipv6")).unwrap().trim(), "1", "{knob}");
        }
        // Loopback is not Ethernet: it would hand a host's own frames back as heard.
        assert!(matches!(RawLink::open("lo", false), Err(RawError::NotEthernet(n)) if n == "lo"));
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

#[test]
fn a_filtered_link_on_the_sending_end_hears_its_type_behind_any_tag() {
    netns::isolated("a_filtered_link_on_the_sending_end_hears_its_type_behind_any_tag", &[("a0", "b0")], || {
        let (a, b) = pair();
        let watcher = RawLink::open_with("a0", Options { ethertype: Some(0x8892), ..Options::default() }).unwrap();
        let far = RawLink::open_with("b0", Options { ethertype: Some(0x8892), ..Options::default() }).unwrap();
        let mut q = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x81, 0x00, 0xC0, 0x00, 0x88, 0x92, 0x80, 0x00];
        q.resize(60, 0);
        let mut ad = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xA8, 0x00, 0x05, 0x88, 0x92, 0x80, 0x00];
        ad.resize(60, 0);
        let mut double = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xA8, 0x00, 0x05, 0x81, 0x00, 0xC0, 0x00, 0x88, 0x92, 0x80, 0x00];
        double.resize(64, 0);
        let mut other = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x81, 0x00, 0xC0, 0x00, 0x88, 0xB5, 0x80, 0x00];
        other.resize(60, 0);
        let mut q_ad = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x81, 0x00, 0xC0, 0x00, 0x88, 0xA8, 0x00, 0x05, 0x88, 0x92, 0x80, 0x00];
        q_ad.resize(64, 0);
        let mut ad_ad = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xA8, 0x00, 0x05, 0x88, 0xA8, 0x00, 0x06, 0x88, 0x92, 0x80, 0x00];
        ad_ad.resize(64, 0);
        let mut q_q = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x81, 0x00, 0xC0, 0x00, 0x81, 0x00, 0x00, 0x07, 0x88, 0x92, 0x80, 0x00];
        q_q.resize(64, 0);
        // Three tags, and another type behind two tags in each order: kept by neither end.
        let mut three = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xA8, 0x00, 0x05, 0x81, 0x00, 0x00, 0x06, 0x81, 0x00, 0x00, 0x07, 0x88, 0x92, 0x80, 0x00];
        three.resize(64, 0);
        let others: Vec<Vec<u8>> = [[0x81, 0x00, 0x88, 0xA8], [0x88, 0xA8, 0x81, 0x00], [0x81, 0x00, 0x81, 0x00], [0x88, 0xA8, 0x88, 0xA8]]
            .iter()
            .map(|t| {
                let mut f = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, t[0], t[1], 0xC0, 0x00, t[2], t[3], 0x00, 0x07, 0x88, 0xB5, 0x80, 0x00];
                f.resize(64, 0);
                f
            })
            .collect();
        let all = RawLink::open("b0", false).unwrap();
        let mut sent = vec![q.clone(), ad.clone(), double.clone(), q_ad.clone(), ad_ad.clone(), q_q.clone(), other.clone(), three.clone()];
        sent.extend(others.iter().cloned());
        for f in &sent {
            a.send(f).unwrap();
        }
        // Every one of them crossed (a link with no filter hears the twelve, each whole).
        let mut crossed = Vec::new();
        while let Some(r) = all.recv(Duration::from_millis(300)).unwrap() {
            crossed.push(r.bytes);
        }
        assert_eq!(crossed, sent);
        for link in [&watcher, &far] {
            let mut got = Vec::new();
            while let Some(r) = link.recv(Duration::from_millis(300)).unwrap() {
                got.push(r.bytes);
            }
            assert_eq!(got, [q.clone(), ad.clone(), double.clone(), q_ad.clone(), ad_ad.clone(), q_q.clone()], "{}", link.iface());
        }
        // A tag's own type is no type to keep behind the tags.
        for t in [0x8100u16, 0x88A8] {
            let r = RawLink::open_with("a0", Options { ethertype: Some(t), ..Options::default() });
            assert!(matches!(r, Err(RawError::FilterType(x)) if x == t), "{r:?}");
        }
        // The kernel counts an 802.1ad tag in the 1514, an 802.1Q tag not.
        let mut ad_long = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xA8, 0x00, 0x05, 0x88, 0xB5];
        ad_long.resize(1518, 0);
        assert!(matches!(a.send(&ad_long), Err(RawError::TooLong(1518))));
        let mut q_long = ad_long.clone();
        q_long[12..14].copy_from_slice(&[0x81, 0x00]);
        assert!(a.send(&q_long).is_ok());
        drop(b);
    });
}

#[test]
fn a_wait_lasts_its_timeout_through_signals_skipped_frames_and_under_a_millisecond() {
    netns::isolated("a_wait_lasts_its_timeout_through_signals_skipped_frames_and_under_a_millisecond", &[("a0", "b0")], || {
        let (a, _b) = pair();
        // A signal in the middle of the wait does not end it.
        extern "C" fn nothing(_: libc::c_int) {}
        // SAFETY: a handler that does nothing, installed without SA_RESTART so the wait is interrupted.
        unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = nothing as *const () as usize;
            libc::sigaction(libc::SIGUSR1, &act, std::ptr::null_mut());
        }
        let me = unsafe { libc::pthread_self() };
        let kicker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            // SAFETY: `me` is the waiting thread, alive until the wait ends.
            unsafe { libc::pthread_kill(me, libc::SIGUSR1) };
        });
        let start = Instant::now();
        assert_eq!(a.recv(Duration::from_millis(300)).unwrap(), None);
        let waited = start.elapsed();
        kicker.join().unwrap();
        assert!(waited >= Duration::from_millis(295) && waited < Duration::from_millis(800), "{waited:?}");
        // A signal late in the wait does not start it again: it ends at its timeout, not a
        // timeout after the signal.
        let kicker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(280));
            // SAFETY: as above.
            unsafe { libc::pthread_kill(me, libc::SIGUSR1) };
        });
        let start = Instant::now();
        assert_eq!(a.recv(Duration::from_millis(300)).unwrap(), None);
        let waited = start.elapsed();
        kicker.join().unwrap();
        assert!(waited >= Duration::from_millis(295) && waited < Duration::from_millis(480), "{waited:?}");

        // A link that leaves out what this host sends waits its timeout under a stream of them.
        let blind = RawLink::open_with("a0", Options { outgoing: false, ..Options::default() }).unwrap();
        let sender = RawLink::open("a0", false).unwrap();
        let stream = std::thread::spawn(move || {
            let f = ether::build(B, A, 0x88B5, &[1; 46]).unwrap();
            let end = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < end {
                let _ = sender.send(&f);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        std::thread::sleep(Duration::from_millis(20));
        let start = Instant::now();
        assert_eq!(blind.recv(Duration::from_millis(300)).unwrap(), None);
        let waited = start.elapsed();
        stream.join().unwrap();
        assert!(waited >= Duration::from_millis(295) && waited < Duration::from_millis(600), "{waited:?}");

        // Under a millisecond, to the microsecond, on a link nothing reaches.
        let idle = RawLink::open_with("a0", Options { ethertype: Some(0x9999), ..Options::default() }).unwrap();
        // Each wait at least what it asks; the shortest of twenty under what a wait rounded up to
        // the millisecond would take (measured: 100 µs takes about 110 µs, rounded about 1030; 1500
        // µs about 1530, rounded about 2070).
        for (asked, under) in [(Duration::from_micros(100), Duration::from_micros(900)), (Duration::from_micros(1500), Duration::from_micros(1900))] {
            let mut waits = Vec::new();
            for _ in 0..20 {
                let start = Instant::now();
                assert_eq!(idle.recv(asked).unwrap(), None);
                waits.push(start.elapsed());
            }
            assert!(waits.iter().all(|w| *w >= asked), "{asked:?}: {waits:?}");
            assert!(waits.iter().min().unwrap() < &under, "{asked:?}: {waits:?}");
        }
    });
}

#[test]
fn a_send_never_waits_and_a_full_queue_is_named() {
    netns::isolated("a_send_never_waits_and_a_full_queue_is_named", &[("a0", "b0")], || {
        let (a, _b) = pair();
        // Two slow queues on a0, 8 kbit/s: a shallow one (the queue drops: ENOBUFS), and a deep
        // one, where the socket's own send buffer fills, the place a send would wait.
        for latency in ["1ms", "1000s"] {
            let _ = Command::new("tc").args(["qdisc", "del", "dev", "a0", "root"]).output();
            let out = Command::new("tc").args(["qdisc", "add", "dev", "a0", "root", "tbf", "rate", "8kbit", "burst", "1600", "latency", latency]).output().unwrap();
            assert!(out.status.success(), "tc: {}", String::from_utf8_lossy(&out.stderr));
            let f = ether::build(B, A, 0x88B5, &[0; 1500]).unwrap();
            let mut full = None;
            for i in 0..5000 {
                let start = Instant::now();
                let r = a.send(&f);
                assert!(start.elapsed() < Duration::from_millis(50), "{latency}: send {i} took {:?}", start.elapsed());
                if let Err(e) = r {
                    full = Some((i, e));
                    break;
                }
            }
            let (i, e) = full.unwrap_or_else(|| panic!("{latency}: the queue never filled"));
            assert!(i > 0, "{latency}: the first send failed: {e:?}");
            assert!(matches!(&e, RawError::QueueFull(n) if n == "a0"), "{latency}: {e:?}");
        }
        let e = RawError::QueueFull("a0".into());
        assert_eq!(e.to_string(), "the send queue of \"a0\" is full: the frame is not sent, and the link did not wait");
        drop(a);
    });
}

#[test]
fn a_frame_past_the_buffer_is_refused_never_handed_back_cut() {
    netns::isolated("a_frame_past_the_buffer_is_refused_never_handed_back_cut", &[("a0", "b0")], || {
        for iface in ["a0", "b0"] {
            netns::ip(&["link", "set", iface, "mtu", "65535"]);
        }
        netns::wait_pairs(&[("a0", "b0")]);
        let a = RawLink::open("a0", false).unwrap();
        let plain = RawLink::open("b0", false).unwrap();
        let filtered = RawLink::open_with("b0", Options { ethertype: Some(0x88B5), ..Options::default() }).unwrap();
        // Untagged and tagged, the buffer's 65 536 bytes are heard whole (a tag the kernel took out
        // counted), and a byte more is refused, its whole length named.
        let frame = |len: usize, tagged: bool| {
            let mut f = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1];
            if tagged {
                f.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
            }
            f.extend_from_slice(&[0x88, 0xB5]);
            f.resize(len, 0x5A);
            f
        };
        for (len, tagged) in [(65_536, false), (65_536, true), (65_537, false), (65_537, true), (65_549, false)] {
            let f = frame(len, tagged);
            a.send(&f).unwrap();
            for link in [&plain, &filtered] {
                let r = link.recv(Duration::from_secs(2));
                if len <= 65_536 {
                    assert_eq!(r.unwrap().map(|r| r.bytes), Some(f.clone()), "{len} {tagged}");
                } else {
                    assert!(matches!(r, Err(RawError::TooLongHeard(n)) if n == len), "{len} {tagged}: {r:?}");
                }
            }
        }
        // A link that leaves out what this host sends passes over its over-long frames too, and
        // waits its timeout.
        let blind = RawLink::open_with("a0", Options { outgoing: false, ..Options::default() }).unwrap();
        a.send(&frame(65_537, false)).unwrap();
        let start = Instant::now();
        assert_eq!(blind.recv(Duration::from_millis(300)).unwrap(), None);
        assert!(start.elapsed() >= Duration::from_millis(295), "{:?}", start.elapsed());
        assert_eq!(
            RawError::TooLongHeard(65_537).to_string(),
            "a frame of 65537 bytes was heard, past the link's 65536-byte buffer: it is not handed back"
        );
    });
}

#[test]
fn a_wait_of_no_end_hands_back_the_next_frame() {
    netns::isolated("a_wait_of_no_end_hands_back_the_next_frame", &[("a0", "b0")], || {
        let (a, b) = pair();
        let f = ether::build(B, A, 0x88B5, &[7; 46]).unwrap();
        a.send(&f).unwrap();
        assert_eq!(b.recv(Duration::MAX).unwrap().map(|r| r.bytes), Some(f.clone()));
        // Nothing there yet: it waits, for the frame sent 300 ms into the wait.
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                a.send(&f).unwrap();
            });
            let start = Instant::now();
            assert_eq!(b.recv(Duration::MAX).unwrap().map(|r| r.bytes), Some(f.clone()));
            assert!(start.elapsed() >= Duration::from_millis(290), "{:?}", start.elapsed());
        });
    });
}

#[test]
fn a_name_the_kernel_gives_an_interface_is_opened() {
    netns::isolated("a_name_the_kernel_gives_an_interface_is_opened", &[], || {
        // Unicode's white space that is not the kernel's: the kernel names an interface so.
        for name in ["a\u{2003}b", "a\u{85}b"] {
            netns::ip(&["link", "add", name, "type", "veth", "peer", "name", &format!("{}x", &name[..1])]);
            netns::ip(&["link", "set", name, "up"]);
            assert!(RawLink::open(name, false).is_ok(), "{name:?}");
            netns::ip(&["link", "del", name]);
        }
    });
}

/// The body the next test runs twenty times: the first frame sent from the end raised first, at
/// once after the fixture, is heard.
#[test]
fn the_first_frame_after_the_fixture_crosses() {
    netns::isolated("the_first_frame_after_the_fixture_crosses", &[("a0", "b0")], || {
        let b = RawLink::open_with("b0", Options { ethertype: Some(0x88B5), ..Options::default() }).unwrap();
        let a = RawLink::open("a0", false).unwrap();
        let f = ether::build(Mac::BROADCAST, a.mac(), 0x88B5, &[1; 46]).unwrap();
        a.send(&f).unwrap();
        assert_eq!(b.recv(Duration::from_secs(1)).unwrap().map(|r| r.bytes), Some(f));
        // The fixture's probes are gone before the body: a link opened now hears none.
        let all = RawLink::open("b0", false).unwrap();
        assert_eq!(all.recv(Duration::from_millis(100)).unwrap(), None);
    });
}

#[test]
fn the_first_frame_after_the_fixture_crosses_in_every_run() {
    for run in 0..20 {
        let out = netns::run_child("the_first_frame_after_the_fixture_crosses").unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.lines().any(|l| l == "veredas-netns: the body ran in the private network: the_first_frame_after_the_fixture_crosses"),
            "run {run}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn a_pair_that_carries_nothing_fails_the_fixture_by_name() {
    netns::isolated("a_pair_that_carries_nothing_fails_the_fixture_by_name", &[("a0", "b0")], || {
        // A pair whose one end drops everything it is given (a queue of no room): the wait fails,
        // naming the way.
        let out = Command::new("tc").args(["qdisc", "add", "dev", "a0", "root", "pfifo", "limit", "0"]).output().unwrap();
        assert!(out.status.success(), "tc: {}", String::from_utf8_lossy(&out.stderr));
        let start = Instant::now();
        let err = panic::catch_unwind(|| netns::wait_pairs(&[("a0", "b0")])).unwrap_err();
        let m = message(err);
        assert!(m.starts_with("no frame crossed from a0 to b0 in 5 s ("), "{m}");
        assert!(start.elapsed() >= Duration::from_secs(5) && start.elapsed() < Duration::from_secs(8), "{:?}", start.elapsed());
        assert_eq!(netns::PROBE, 0x88B6);
    });
}

#[test]
fn a_pair_dead_the_other_way_fails_the_fixture_by_name() {
    netns::isolated("a_pair_dead_the_other_way_fails_the_fixture_by_name", &[("a0", "b0")], || {
        let out = Command::new("tc").args(["qdisc", "add", "dev", "b0", "root", "pfifo", "limit", "0"]).output().unwrap();
        assert!(out.status.success(), "tc: {}", String::from_utf8_lossy(&out.stderr));
        let start = Instant::now();
        let m = message(panic::catch_unwind(|| netns::wait_pairs(&[("a0", "b0")])).unwrap_err());
        assert!(m.starts_with("no frame crossed from b0 to a0 in 5 s ("), "{m}");
        assert!(start.elapsed() >= Duration::from_secs(5) && start.elapsed() < Duration::from_secs(8), "{:?}", start.elapsed());
    });
}

#[test]
fn a_link_that_leaves_out_this_host_s_frames_hears_the_wire_behind_a_burst_of_them() {
    netns::isolated("a_link_that_leaves_out_this_host_s_frames_hears_the_wire_behind_a_burst_of_them", &[("a0", "b0")], || {
        let (_a, b) = pair();
        let blind = RawLink::open_with("a0", Options { outgoing: false, ..Options::default() }).unwrap();
        let sender = RawLink::open("a0", false).unwrap();
        // Three thousand of this host's frames while the link is not read: none fills its buffer.
        let f = ether::build(B, A, 0x88B5, &[1; 46]).unwrap();
        for _ in 0..3000 {
            let _ = sender.send(&f);
        }
        std::thread::sleep(Duration::from_millis(50));
        let answer = ether::build(A, B, 0x88B7, &[2; 46]).unwrap();
        b.send(&answer).unwrap();
        let mut heard = Vec::new();
        while let Some(r) = blind.recv(Duration::from_millis(500)).unwrap() {
            heard.push(r.bytes);
        }
        assert_eq!(heard, [answer]);
    });
}

#[test]
fn a_link_opened_in_a_flood_hears_only_its_interface_and_its_type() {
    netns::isolated("a_link_opened_in_a_flood_hears_only_its_interface_and_its_type", &[("a0", "b0"), ("a1", "b1")], || {
        let (_a, _b) = pair();
        let other = Mac([0x02, 0x00, 0x00, 0x00, 0x0A, 0x02]);
        netns::ip(&["link", "set", "a1", "down"]);
        netns::ip(&["link", "set", "a1", "address", &other.to_string()]);
        netns::ip(&["link", "set", "a1", "up"]);
        netns::wait_pairs(&[("a1", "b1")]);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let floods: Vec<_> = [("a0", A), ("a1", other)]
            .into_iter()
            .map(|(iface, src)| {
                let stop = stop.clone();
                let link = RawLink::open(iface, false).unwrap();
                std::thread::spawn(move || {
                    let f = ether::build(Mac::BROADCAST, src, 0x88B5, &[3; 46]).unwrap();
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = link.send(&f);
                    }
                })
            })
            .collect();
        let (mut opens, mut wrong_type, mut wrong_interface) = (0, 0, 0);
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            let filtered = RawLink::open_with("b0", Options { ethertype: Some(0x8892), ..Options::default() }).unwrap();
            while let Some(r) = filtered.recv(Duration::ZERO).unwrap() {
                if ether::parse(&r.bytes).unwrap().ethertype != 0x8892 {
                    wrong_type += 1;
                }
            }
            let plain = RawLink::open("b0", false).unwrap();
            for _ in 0..3 {
                if let Some(r) = plain.recv(Duration::ZERO).unwrap()
                    && ether::parse(&r.bytes).unwrap().src == other
                {
                    wrong_interface += 1;
                }
            }
            opens += 1;
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        floods.into_iter().for_each(|t| t.join().unwrap());
        assert!(opens > 20, "{opens}");
        assert_eq!((wrong_type, wrong_interface), (0, 0), "in {opens} opens");
    });
}

#[test]
fn short_frames_and_tags_of_nothing_cross_whole() {
    netns::isolated("short_frames_and_tags_of_nothing_cross_whole", &[("a0", "b0")], || {
        let (a, b) = pair();
        let filtered = RawLink::open_with("b0", Options { ethertype: Some(0x8892), ..Options::default() }).unwrap();
        // The shortest frame a link sends is the header alone; one under 60 crosses as it is.
        for len in [14, 59] {
            let mut f = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xB5];
            f.resize(len, 0x33);
            a.send(&f).unwrap();
            assert_eq!(heard(&b, 0x88B5, false, Duration::from_secs(2)).map(|r| r.bytes), Some(f), "{len}");
        }
        // Tags of priority 0 and VLAN 0 (a TCI of zero), 802.1Q, 802.1ad and two of them, put back
        // whole by a plain link and a filtered one.
        let tagged = |tags: &[u8]| {
            let mut f = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1];
            f.extend_from_slice(tags);
            f.extend_from_slice(&[0x88, 0x92, 0x80, 0x00]);
            f.resize(64, 0x33);
            f
        };
        let frames = [tagged(&[0x81, 0x00, 0x00, 0x00]), tagged(&[0x88, 0xA8, 0x00, 0x00]), tagged(&[0x81, 0x00, 0x00, 0x00, 0x81, 0x00, 0x00, 0x00])];
        for f in &frames {
            a.send(f).unwrap();
        }
        for link in [&b, &filtered] {
            let mut got = Vec::new();
            while let Some(r) = link.recv(Duration::from_millis(300)).unwrap() {
                if r.bytes.len() == 64 {
                    got.push(r.bytes);
                }
            }
            assert_eq!(got, frames, "{}", link.iface());
        }
    });
}

#[test]
fn a_tag_the_kernel_holds_on_the_sending_end_is_put_back() {
    netns::isolated("a_tag_the_kernel_holds_on_the_sending_end_is_put_back", &[("a0", "b0"), ("c0", "d0")], || {
        // A bridge that tags on its way out of a0 (VLAN 5) what comes in untagged on c0: the frame
        // leaves a0 with its tag held by the kernel beside it, not in its bytes.
        let bridge = |args: &[&str]| {
            let out = Command::new("bridge").args(args).output().unwrap();
            assert!(out.status.success(), "bridge {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        netns::ip(&["link", "add", "br0", "type", "bridge", "vlan_filtering", "1"]);
        netns::ip(&["link", "set", "a0", "master", "br0"]);
        netns::ip(&["link", "set", "c0", "master", "br0"]);
        netns::ip(&["link", "set", "br0", "up"]);
        bridge(&["vlan", "add", "dev", "a0", "vid", "5"]);
        bridge(&["vlan", "add", "dev", "c0", "vid", "5", "pvid", "untagged"]);
        netns::wait_up(&["a0", "c0"]);
        let watcher = RawLink::open("a0", false).unwrap();
        let filtered = RawLink::open_with("a0", Options { ethertype: Some(0x8892), ..Options::default() }).unwrap();
        let d = RawLink::open("d0", false).unwrap();
        let sent = ether::build(Mac::BROADCAST, d.mac(), 0x8892, &[0x80, 0x00]).unwrap();
        for _ in 0..3 {
            d.send(&sent).unwrap();
        }
        let mut want = sent[..12].to_vec();
        want.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
        want.extend_from_slice(&sent[12..]);
        for link in [&watcher, &filtered] {
            let mut got = Vec::new();
            while let Some(r) = link.recv(Duration::from_millis(300)).unwrap() {
                if r.outgoing && ether::parse(&r.bytes).unwrap().src == d.mac() {
                    got.push(r.bytes);
                }
            }
            assert_eq!(got, vec![want.clone(); 3], "{}", link.iface());
        }
    });
}

#[test]
fn the_lookup_names_what_failed_and_an_alternative_name_follows_the_primary_rule() {
    netns::isolated("the_lookup_names_what_failed_and_an_alternative_name_follows_the_primary_rule", &[("a0", "b0")], || {
        netns::wait_pairs(&[("a0", "b0")]);
        let a0 = RawLink::open("a0", false).unwrap().index();
        // An alternative name that keeps the primary names' rule opens its interface; one the
        // kernel gives that does not (a space, a colon, past 15 bytes) is refused by that rule.
        for alt in ["alt7", "a b", "a:b", "a-very-long-alternative-name"] {
            netns::ip(&["link", "property", "add", "dev", "a0", "altname", alt]);
        }
        assert_eq!(RawLink::open("alt7", false).unwrap().index(), a0);
        for alt in ["a b", "a:b", "a-very-long-alternative-name"] {
            assert!(matches!(RawLink::open(alt, false), Err(RawError::BadName(n)) if n == alt), "{alt:?}");
        }

        // With no descriptor left for the lookup's own socket, an interface that is there is not
        // said to be missing: the lookup's failure is named.
        let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: a valid rlimit to fill and then set.
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut lim) };
        let low = libc::rlimit { rlim_cur: 64, rlim_max: lim.rlim_max };
        // SAFETY: as above.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const low) }, 0);
        let mut held = Vec::new();
        loop {
            // SAFETY: duplicating standard input; each kept to be closed below.
            let fd = unsafe { libc::dup(0) };
            if fd < 0 {
                break;
            }
            held.push(fd);
        }
        let err = RawLink::open("a0", false).unwrap_err();
        for fd in held {
            // SAFETY: each fd was opened above and is closed once.
            unsafe { libc::close(fd) };
        }
        // SAFETY: as above.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) };
        // (glibc's lookup reports its own socket failing as ENOENT, measured; never ENODEV.)
        assert!(matches!(&err, RawError::Io { op: "look up the interface", iface, source } if iface == "a0" && source.raw_os_error() != Some(libc::ENODEV)), "{err}");
        assert!(err.to_string().starts_with("look up the interface on \"a0\": "), "{err}");
        // With descriptors again, a name no interface has is said to be missing.
        assert!(matches!(RawLink::open("zz9", false), Err(RawError::NoSuchInterface(n)) if n == "zz9"));
        assert!(RawLink::open("a0", false).is_ok());
    });
}
