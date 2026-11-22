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
    netns::wait_up(&["a0", "b0"]);
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
    ];
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
        for f in [&q, &ad, &double, &other] {
            a.send(f).unwrap();
        }
        for link in [&watcher, &far] {
            let mut got = Vec::new();
            while let Some(r) = link.recv(Duration::from_millis(300)).unwrap() {
                got.push(r.bytes);
            }
            assert_eq!(got, [q.clone(), ad.clone(), double.clone()], "{}", link.iface());
        }
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
        let start = Instant::now();
        assert_eq!(idle.recv(Duration::from_micros(900)).unwrap(), None);
        let waited = start.elapsed();
        assert!(waited >= Duration::from_micros(900) && waited < Duration::from_millis(50), "{waited:?}");
    });
}

#[test]
fn a_send_never_waits_and_a_full_queue_is_named() {
    netns::isolated("a_send_never_waits_and_a_full_queue_is_named", &[("a0", "b0")], || {
        let (a, _b) = pair();
        // A slow queue on a0: 8 kbit/s, so it fills at once.
        let out = Command::new("tc").args(["qdisc", "add", "dev", "a0", "root", "tbf", "rate", "8kbit", "burst", "1600", "latency", "1ms"]).output().unwrap();
        assert!(out.status.success(), "tc: {}", String::from_utf8_lossy(&out.stderr));
        let f = ether::build(B, A, 0x88B5, &[0; 1500]).unwrap();
        let mut full = None;
        for i in 0..2000 {
            let start = Instant::now();
            let r = a.send(&f);
            assert!(start.elapsed() < Duration::from_millis(50), "send {i} took {:?}", start.elapsed());
            if let Err(e) = r {
                full = Some((i, e));
                break;
            }
        }
        let (i, e) = full.expect("the queue never filled");
        assert!(i > 0, "the first send failed: {e:?}");
        assert!(matches!(&e, RawError::QueueFull(n) if n == "a0"), "{e:?}");
        assert_eq!(e.to_string(), "the send queue of \"a0\" is full: the frame is not sent, and the link did not wait");
    });
}

#[test]
fn a_frame_past_the_buffer_is_refused_never_handed_back_cut() {
    netns::isolated("a_frame_past_the_buffer_is_refused_never_handed_back_cut", &[("a0", "b0")], || {
        for iface in ["a0", "b0"] {
            netns::ip(&["link", "set", iface, "mtu", "65535"]);
        }
        netns::wait_up(&["a0", "b0"]);
        let a = RawLink::open("a0", false).unwrap();
        let plain = RawLink::open("b0", false).unwrap();
        let filtered = RawLink::open_with("b0", Options { ethertype: Some(0x88B5), ..Options::default() }).unwrap();
        let mut f = vec![2, 0, 0, 0, 0x0B, 1, 2, 0, 0, 0, 0x0A, 1, 0x88, 0xB5];
        f.resize(14 + 65_535, 0x5A);
        a.send(&f).unwrap();
        for link in [&plain, &filtered] {
            let r = link.recv(Duration::from_secs(2));
            assert!(matches!(r, Err(RawError::TooLong(65_549))), "{r:?}");
        }
    });
}
