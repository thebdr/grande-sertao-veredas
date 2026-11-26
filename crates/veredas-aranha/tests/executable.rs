//! The executable (contract: the executable clause): `veredas-aranha listen` on interfaces for a
//! while, asking and reading as told, writing the scan and its capture; `identify`; `diff` of two
//! scan files, its exit status diff(1)'s; every usage it cannot follow refused with status 2 and
//! its reason. Proven on the bench's devices in a private network, the bench changed between two
//! scans.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::scan::Scan;

fn aranha(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).args(args).output().unwrap()
}

/// The executable started, its output read when it ends: the listener up before the bench starts,
/// so the devices' Hellos are heard.
#[cfg(target_os = "linux")]
fn started(args: &[&str]) -> std::process::Child {
    use std::process::Stdio;
    let child = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    child
}

/// The executable run with its stdout or its stderr a pipe whose reader is gone before it writes
/// anything: every write fails (a broken pipe), which must change neither what it does nor its
/// exit status.
fn closed(args: &[&std::ffi::OsStr], stdout: bool) -> Output {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_veredas-aranha"));
    cmd.args(args);
    if stdout {
        cmd.stdout(writer).stderr(std::process::Stdio::piped());
    } else {
        cmd.stderr(writer).stdout(std::process::Stdio::piped());
    }
    cmd.output().unwrap()
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
}

/// A path that is not UTF-8, as a system may give one.
#[cfg(unix)]
fn not_utf8(tail: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    let mut b = vec![0xFF, b'-'];
    b.extend_from_slice(tail.as_bytes());
    std::ffi::OsString::from_vec(b)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("executable").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_usage_it_cannot_follow_is_refused_with_its_reason() {
    let version = aranha(&["--version"]);
    assert_eq!((version.status.code(), text(&version.stdout)), (Some(0), format!("veredas-aranha {}\n", env!("CARGO_PKG_VERSION"))));
    let help = aranha(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(text(&help.stdout).starts_with("usage:\n  veredas-aranha listen <interface>..."), "{}", text(&help.stdout));
    let d = dir("refusals");
    let scan = d.join("x.toml");
    let s = scan.to_str().unwrap();
    for (args, says) in [
        (vec![], "usage:"),
        (vec!["frob"], "no command \"frob\""),
        (vec!["listen", "--out", s], "listen needs an interface"),
        (vec!["listen", "eth0"], "listen writes its scan to --out <scan.toml>"),
        (vec!["listen", "eth0", "--for", "0", "--out", s], "--for takes 1 to 3600 seconds"),
        (vec!["listen", "eth0", "--for", "3601", "--out", s], "--for takes 1 to 3600 seconds"),
        (vec!["listen", "eth0", "--for", "x", "--out", s], "--for takes whole seconds"),
        (vec!["listen", "eth0", "eth0", "--out", s], "eth0 is named twice"),
        (vec!["listen", "eth0", "--out", "x.json"], "x.json: a scan file is .toml"),
        (vec!["listen", "eth0", "--loud", "--out", s], "listen takes no --loud"),
        (vec!["listen", "eth0", "--out"], "--out needs a value"),
        (vec!["identify"], "identify needs an interface"),
        (vec!["identify", "eth0", "eth1"], "identify asks on one interface"),
        (vec!["identify", "eth0", "--wait", "soon"], "--wait takes milliseconds"),
        // An option given twice is refused, never the second taken over the first.
        (vec!["listen", "eth0", "--for", "1", "--for", "2", "--out", s], "--for is given twice"),
        (vec!["listen", "eth0", "--out", s, "--out", s], "--out is given twice"),
        (vec!["listen", "eth0", "--identify", "--identify", "--out", s], "--identify is given twice"),
        (vec!["listen", "eth0", "--ports", "--ports", "--out", s], "--ports is given twice"),
        (vec!["identify", "eth0", "--name", "a", "--alias", "b"], "--name or --alias is given twice"),
        (vec!["identify", "eth0", "--name", "a", "--name", "a"], "--name or --alias is given twice"),
        (vec!["identify", "eth0", "--wait", "1", "--wait", "1"], "--wait is given twice"),
        (vec!["diff", s], "diff takes two scan files"),
        (vec!["diff", s, s], "x.toml: "),
    ] {
        let out = aranha(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with("veredas-aranha: ") && text(&out.stderr).contains(says), "{args:?}: {}", text(&out.stderr));
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert!(!scan.exists());

    // Arguments as the system gives them: a path need not be UTF-8 (read, and named in the
    // refusal), anything else that is not is refused by name; never a panic.
    #[cfg(unix)]
    {
        let odd = d.join(not_utf8("baseline.toml"));
        let out = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).arg("diff").arg(&odd).arg(&scan).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with(&format!("veredas-aranha: {}: ", odd.display())), "{}", text(&out.stderr));
        let out = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).arg("listen").arg(not_utf8("eth")).arg("--out").arg(&scan).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with("veredas-aranha: an argument ") && text(&out.stderr).contains("is not UTF-8"), "{}", text(&out.stderr));
        let out = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).arg(not_utf8("listen")).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with("veredas-aranha: no command "), "{}", text(&out.stderr));
    }
    // A stdout or a stderr closed changes no exit status.
    for (args, stdout, code) in [(vec!["--help"], true, 0), (vec!["--version"], true, 0), (vec!["frob"], false, 2), (vec!["diff", s, s], false, 2)] {
        let out = closed(&args.iter().map(|a| os(a)).collect::<Vec<_>>(), stdout);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}{}", text(&out.stdout), text(&out.stderr));
        assert!(!text(&out.stderr).contains("panicked") && !text(&out.stdout).contains("panicked"), "{args:?}");
    }
}

#[cfg(target_os = "linux")]
mod bench;

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
hello = false
seed = 2
"#;

#[cfg(target_os = "linux")]
const AFTER: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "first"
interface = "b0"
station = "bomba-2"
alias = "port-007.switch-9"
vendor_id = 42
device_id = 261
ip = { address = "192.168.0.20", mask = "255.255.255.0" }
lldp_interval_ms = 200
port = "port-002"
seed = 1
"#;

#[cfg(target_os = "linux")]
#[test]
fn a_scan_written_by_the_executable_and_the_diff_of_two_say_what_the_bench_did() {
    use veredas_aranha::netns;
    use veredas_aranha::wire::pcap;

    const FIRST: Mac = Mac([0x02, 0x00, 0x5E, 0xAF, 0x00, 0x01]);
    const SECOND: Mac = Mac([0x02, 0x00, 0x5E, 0xAF, 0x00, 0x02]);
    const HERE0: Mac = Mac([0x02, 0x00, 0x5E, 0xAF, 0x00, 0xA0]);
    const HERE1: Mac = Mac([0x02, 0x00, 0x5E, 0xAF, 0x00, 0xA1]);
    bench::maquinar();
    let d = dir("bench");
    let (before, after) = (d.join("before.toml"), d.join("after.toml"));
    let pairs = [("a0", "b0"), ("a1", "b1")];
    netns::isolated("a_scan_written_by_the_executable_and_the_diff_of_two_say_what_the_bench_did", &pairs, || {
        bench::pin(&[("a0", HERE0), ("b0", FIRST), ("a1", HERE1), ("b1", SECOND)], &pairs);
        // a1 holds no address of its own: the second device's ports cannot be read there.
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        // An interface that is not there is named; nothing is written; an hour is a time taken.
        let nowhere = d.join("nowhere.toml");
        let out = aranha(&["listen", "nosuch0", "--for", "1", "--out", nowhere.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(2));
        assert!(text(&out.stderr).starts_with("veredas-aranha: nosuch0: "), "{}", text(&out.stderr));
        assert!(!nowhere.exists() && !d.join("nowhere.pcap").exists());
        let out = aranha(&["listen", "nosuch0", "--for", "3600", "--out", nowhere.to_str().unwrap()]);
        assert!(text(&out.stderr).starts_with("veredas-aranha: nosuch0: "), "{}", text(&out.stderr));
        // A folder whose name is not UTF-8 is a folder like any: the interface is refused.
        #[cfg(unix)]
        {
            let odd = d.join(not_utf8("folder"));
            std::fs::create_dir_all(&odd).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).args(["listen", "nosuch0", "--out"]).arg(odd.join("x.toml")).output().unwrap();
            assert!(text(&out.stderr).starts_with("veredas-aranha: nosuch0: "), "{}", text(&out.stderr));
        }

        // The first scan: both devices, the second's name found by asking alone (it says no
        // Hello), each one's ports read.
        let t0 = veredas_aranha::wire::stamp(std::time::SystemTime::now());
        let clock = std::time::Instant::now();
        let listening = started(&["listen", "a0", "a1", "--for", "3", "--identify", "--ports", "--out", before.to_str().unwrap()]);
        let run = bench::start("executable-before", BEFORE);
        let out = listening.wait_with_output().unwrap();
        // Three seconds, the Identify inside them, the reads after: not a second more.
        let took = clock.elapsed();
        assert!(took >= std::time::Duration::from_secs(3) && took < std::time::Duration::from_millis(4500), "{took:?}");
        let (status, o, e) = bench::end(run);
        assert!(status.success(), "the bench: {o}\n{e}");
        assert_eq!(out.status.code(), Some(0), "{}\n{}", text(&out.stdout), text(&out.stderr));
        let said = text(&out.stdout);
        assert!(said.contains(&format!("{FIRST} \"bomba-1\" 192.168.0.10 on a0\n")) && said.contains(&format!("{SECOND} \"bomba-1\" 192.168.0.11 on a1\n")), "{said}");
        assert!(said.contains(&format!("conflict duplicate-name \"bomba-1\": {FIRST}, {SECOND}\n")), "{said}");
        assert!(said.ends_with(&format!("2 device(s), 2 link(s), {} frame(s) heard; {} and {}\n", Scan::read_file(&before).unwrap().stats.heard, before.display(), d.join("before.pcap").display())), "{said}");
        assert_eq!(
            text(&out.stderr),
            format!("a0: 1 device(s) answered the Identify\na1: 1 device(s) answered the Identify\n{SECOND} \"bomba-1\": subslot 0x8001 not read: \"a1\" has no IPv4 address of its own: a device could not answer it\n")
        );

        // The file: read back whole, its capture beside it holding every frame its witnesses name,
        // the Identify and the reads among what was sent.
        let scan = Scan::read_file(&before).unwrap();
        assert_eq!(scan.devices.iter().map(|d| d.mac).collect::<Vec<_>>(), [FIRST, SECOND]);
        let capture = scan.capture.clone().unwrap();
        assert_eq!(capture.file, "before.pcap");
        let frames = pcap::read_file(&d.join(&capture.file), "a0").unwrap().frames;
        assert_eq!(frames.len() as u64, capture.frames);
        // An Identify on each, and the first device's two reads on a0.
        assert_eq!((scan.stats.interfaces["a0"].sent, scan.stats.interfaces["a1"].sent), (3, 1), "{:?}", scan.stats.interfaces);
        // The Identify on both interfaces at once, a second in.
        let asked: Vec<u64> = frames
            .iter()
            .filter(|f| veredas_aranha::profinet::ether::parse(&f.bytes).is_ok_and(|e| (e.src == HERE0 || e.src == HERE1) && e.ethertype == veredas_aranha::profinet::ether::ETHERTYPE_PROFINET && e.payload.get(..2) == Some(&[0xFE, 0xFE][..])))
            .map(|f| f.at_ns - t0)
            .collect();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(asked.iter().all(|t| (900_000_000..1_700_000_000).contains(t)), "{asked:?}");
        assert!(asked[0].abs_diff(asked[1]) < 200_000_000, "{asked:?}");
        // Every name's witnesses are records of the capture that say that name.
        for dev in &scan.devices {
            for c in &dev.name.candidates {
                for w in [&c.first, &c.last] {
                    let record = &frames[(w.frame - capture.first_frame) as usize];
                    let said = veredas_aranha::observe::decode(record, w.frame).observations;
                    assert!(said.iter().any(|o| matches!(&o.observed, veredas_aranha::observe::Observed::Dcp { identity, .. } if identity.name.as_ref() == Some(&c.value))), "{} {w:?}", c.value);
                }
            }
        }
        let reads = frames.iter().filter(|f| veredas_aranha::ip::datagram(&f.bytes).is_some_and(|d| d.dst.port == 34964)).count();
        assert_eq!(reads, 2, "the first device's port 0x8001 read, and 0x8002 refused");

        // The second, the bench changed: the first renamed, readdressed and its cable on port 2,
        // the second gone; and an identify by name.
        let listening = started(&["listen", "a0", "a1", "--for", "2", "--out", after.to_str().unwrap()]);
        let run = bench::start("executable-after", AFTER);
        let out = listening.wait_with_output().unwrap();
        let clock = std::time::Instant::now();
        let named = aranha(&["identify", "a0", "--name", "bomba-2", "--wait", "300"]);
        let quick = clock.elapsed();
        let aliased = aranha(&["identify", "a0", "--alias", "port-007.switch-9", "--wait", "300"]);
        let nobody = aranha(&["identify", "a0", "--name", "bomba-1", "--wait", "300"]);
        let (status, o, e) = bench::end(run);
        assert!(status.success(), "the bench: {o}\n{e}");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        assert_eq!(named.status.code(), Some(0), "{}", text(&named.stderr));
        // By name and by alias, waited 300 ms: the answer, its ids as the device states them.
        let answer = format!("{FIRST} \"bomba-2\" 192.168.0.20 ids 0x002a/0x0105 (\"bomba-2\")\n1 answer(s) on a0, 0 other(s), 0 lost\n");
        assert_eq!(text(&named.stdout), answer);
        assert_eq!((aliased.status.code(), text(&aliased.stdout)), (Some(0), answer.clone()));
        assert!(quick < std::time::Duration::from_millis(900), "{quick:?}");
        // Listening with its stderr closed: the Identify's count goes nowhere, the scan is written
        // all the same and the status is 0.
        let quiet = d.join("quiet.toml");
        let out = closed(&[os("listen"), os("a0"), os("--for"), os("1"), os("--identify"), os("--out"), quiet.as_os_str()], false);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
        assert!(Scan::read_file(&quiet).is_ok() && d.join("quiet.pcap").exists());
        // A name no device holds now: no answer.
        assert_eq!((nobody.status.code(), text(&nobody.stdout)), (Some(0), "0 answer(s) on a0, 0 other(s), 0 lost\n".to_string()));
        // A write failing past the checks (a folder where the capture's own file is first
        // written): refused, the scan and capture already there byte for byte as they were, and
        // nothing left under another name.
        let (old_scan, old_capture) = (d.join("collide.toml"), d.join("collide.pcap"));
        std::fs::write(&old_scan, "the baseline").unwrap();
        std::fs::write(&old_capture, "its frames").unwrap();
        std::fs::create_dir_all(d.join(".collide.pcap.partial")).unwrap();
        let out = aranha(&["listen", "a0", "--for", "1", "--out", old_scan.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with(&format!("veredas-aranha: {}: ", old_capture.display())), "{}", text(&out.stderr));
        assert_eq!((std::fs::read(&old_scan).unwrap(), std::fs::read(&old_capture).unwrap()), (b"the baseline".to_vec(), b"its frames".to_vec()));
        assert!(!d.join(".collide.toml.partial").exists());
        // Without --for: ten seconds.
        let clock = std::time::Instant::now();
        let ten = aranha(&["listen", "a0", "--out", d.join("ten.toml").to_str().unwrap()]);
        assert_eq!(ten.status.code(), Some(0), "{}", text(&ten.stderr));
        let took = clock.elapsed();
        assert!(took >= std::time::Duration::from_secs(10) && took < std::time::Duration::from_millis(11500), "{took:?}");
    });

    // The diff, outside the private network: the files are all it needs.
    let out = aranha(&["diff", before.to_str().unwrap(), after.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let first = format!("dev:{FIRST}");
    let second = format!("dev:{SECOND}");
    assert_eq!(
        text(&out.stdout),
        [
            format!("vanished {SECOND} \"bomba-1\""),
            format!("renamed {FIRST} station name: \"bomba-1\" -> \"bomba-2\""),
            format!("readdressed {FIRST} ip suite: \"192.168.0.10 255.255.255.0 0.0.0.0\" -> \"192.168.0.20 255.255.255.0 0.0.0.0\""),
            format!("link added \"{first}\" \"port-002\" = \"if:a0\" (heard)"),
            format!("link removed \"{first}\" \"port-001\" = \"if:a0\" (heard)"),
            format!("link removed \"{second}\" \"port-001\" = \"if:a1\" (heard)"),
            format!("conflict cleared duplicate-name \"bomba-1\": {FIRST}, {SECOND}"),
            String::new(),
        ]
        .join("\n")
    );
    // The other way round: the mirror, a line a change.
    let back = aranha(&["diff", after.to_str().unwrap(), before.to_str().unwrap()]);
    assert_eq!(back.status.code(), Some(1));
    assert_eq!(
        text(&back.stdout),
        [
            format!("appeared {SECOND} \"bomba-1\""),
            format!("renamed {FIRST} station name: \"bomba-2\" -> \"bomba-1\""),
            format!("readdressed {FIRST} ip suite: \"192.168.0.20 255.255.255.0 0.0.0.0\" -> \"192.168.0.10 255.255.255.0 0.0.0.0\""),
            format!("link added \"{first}\" \"port-001\" = \"if:a0\" (heard)"),
            format!("link added \"{second}\" \"port-001\" = \"if:a1\" (heard)"),
            format!("link removed \"{first}\" \"port-002\" = \"if:a0\" (heard)"),
            format!("conflict new duplicate-name \"bomba-1\": {FIRST}, {SECOND}"),
            String::new(),
        ]
        .join("\n")
    );
    // A scan listened on a0 alone against one on both: the interface removed is said.
    let quiet = aranha(&["diff", before.to_str().unwrap(), d.join("quiet.toml").to_str().unwrap()]);
    assert!(text(&quiet.stdout).lines().any(|l| l == "interface removed a1"), "{}", text(&quiet.stdout));
    let same = aranha(&["diff", before.to_str().unwrap(), before.to_str().unwrap()]);
    assert_eq!((same.status.code(), text(&same.stdout)), (Some(0), "no change\n".to_string()));
    // A reader that stops reading (`diff a b | head -1`) changes neither status.
    assert_eq!(closed(&[os("diff"), before.as_os_str(), after.as_os_str()], true).status.code(), Some(1));
    assert_eq!(closed(&[os("diff"), before.as_os_str(), before.as_os_str()], true).status.code(), Some(0));
}

/// Scans made with the library, written and diffed by the executable: every kind of line, in the
/// diff's order, the texts quoted so that a port or a chassis holding a space, an equals sign or a
/// line break is one line, read back for what it says.
#[test]
fn the_diff_prints_every_kind_of_change_a_line_each_its_texts_quoted() {
    use std::net::Ipv4Addr;
    use veredas_aranha::fusion::Fusion;
    use veredas_aranha::listen::{Interface as Account, Stats};
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::lldp::{ChassisId, LldpPdu, PortId};
    use veredas_aranha::topology::{self, Interface, Neighbour};
    let mac = |b: u8| Mac([0x02, 0, 0, 0, 0, b]);
    let (a, b, c, dd, e) = (mac(0x0A), mac(0x0B), mac(0x0C), mac(0x0D), mac(0x0E));
    let at = |frame: u64, iface: &str, probe| Witness { probe, iface: iface.into(), at_ns: 1_000 + frame, frame };
    let hello = |m: Mac, name: &str, ip: Option<Ipv4Addr>, frame| {
        let identity = DeviceIdentity { name: Some(name.into()), ip: ip.map(|ip| IpSuite { ip, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
        Observation { witness: at(frame, "eth1", Probe::DcpHeard), observed: Observed::Dcp { mac: m, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } }
    };
    let lldp = |m: Mac, port: &str, frame| {
        let mut pdu = LldpPdu::new(ChassisId::Mac { mac: m }, PortId::Local { id: port.into() }, 20);
        pdu.pn.chassis_mac = Some(m);
        Observation { witness: at(frame, "eth1", Probe::LldpHeard), observed: Observed::Lldp { src: Mac([0x02, 0, 0, 0, m.0[5], 1]), pdu } }
    };
    let arp = |m: Mac, ip, frame| Observation { witness: at(frame, "eth1", Probe::ArpHeard), observed: Observed::Arp { mac: m, ip } };
    let scan = |obs: &[Observation], ifaces: &[&str], neighbours: &[Neighbour]| {
        let mut f = Fusion::new();
        for o in obs {
            f.apply(o);
        }
        let stats = Stats { interfaces: ifaces.iter().map(|i| (i.to_string(), Account::default())).collect(), ..Stats::default() };
        let list: Vec<Interface> = ifaces.iter().map(|i| Interface { name: i.to_string(), mac: None }).collect();
        Scan::new(&f, &stats, &topology::build(&f, &list, neighbours), 2_000, None)
    };
    let ten = Ipv4Addr::new(192, 168, 0, 10);
    let baseline = scan(&[hello(a, "a", Some(ten), 1), lldp(a, "port-001", 2), arp(a, ten, 3), hello(b, "b", None, 4), hello(c, "dup", None, 5), hello(dd, "dup", None, 6)], &["eth1", "eth2"], &[]);
    let odd_port = "port 1 = \"x\"\nno change";
    let chassis = "sw 1 = x";
    let word = Neighbour { device: a, port: "port-003".into(), chassis: chassis.into(), peer_port: "port-009".into(), peer_mac: None, witnesses: 1 };
    let later = scan(&[hello(a, "a", Some(ten), 1), lldp(a, odd_port, 2), hello(c, "c", None, 5), hello(dd, "dup", None, 6), hello(e, "e", Some(ten), 7)], &["eth1", "eth3"], &[word]);
    let d = dir("kinds");
    let (bf, lf) = (d.join("baseline.toml"), d.join("later.toml"));
    baseline.write_file(&bf).unwrap();
    later.write_file(&lf).unwrap();
    let out = aranha(&["diff", bf.to_str().unwrap(), lf.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let dev = |m: Mac| format!("\"dev:{m}\"");
    let want = [
        format!("appeared {e} \"e\""),
        format!("vanished {b} \"b\""),
        format!("renamed {c} station name: \"dup\" -> \"c\""),
        format!("one-sided {a} arp address: baseline \"192.168.0.10\", scan none"),
        format!("link added \"chassis:sw 1 = x\" \"port-009\" = {} \"port-003\" (reported)", dev(a)),
        format!("link added {} {:?} = \"if:eth1\" (heard)", dev(a), odd_port),
        format!("link removed {} \"port-001\" = \"if:eth1\" (heard)", dev(a)),
        format!("conflict new duplicate-ip \"192.168.0.10\": {a}, {e}"),
        format!("conflict cleared duplicate-name \"dup\": {c}, {dd}"),
        "interface added eth3".to_string(),
        "interface removed eth2".to_string(),
    ];
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines, want.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(!lines.contains(&"no change"));
}

/// The scan and its capture are checked before anything is opened, and a scan that cannot be
/// written replaces nothing: a folder where the scan or the capture would be, a scan or a capture
/// that may not be written, a scan file's name that is not UTF-8.
#[cfg(unix)]
#[test]
fn a_scan_that_cannot_be_written_is_refused_before_anything_is_opened_or_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("unwritable");
    let refused = |out: &Path, says: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_veredas-aranha")).args(["listen", "nosuch0", "--for", "1", "--out"]).arg(out).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        let e = text(&o.stderr);
        assert!(e.starts_with("veredas-aranha: ") && e.contains(says) && !e.contains("nosuch0"), "{e}");
    };
    // A folder where the scan would be, or its capture.
    std::fs::create_dir_all(d.join("folder.toml")).unwrap();
    refused(&d.join("folder.toml"), "there is something there that is no file");
    std::fs::create_dir_all(d.join("held.pcap")).unwrap();
    refused(&d.join("held.toml"), "held.pcap: there is something there that is no file");
    // An earlier scan, read only: refused, it and its capture byte for byte as they were.
    let (kept, capture) = (d.join("kept.toml"), d.join("kept.pcap"));
    std::fs::write(&kept, "the baseline").unwrap();
    std::fs::write(&capture, "its frames").unwrap();
    std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o444)).unwrap();
    refused(&kept, "kept.toml: ");
    assert_eq!((std::fs::read(&kept).unwrap(), std::fs::read(&capture).unwrap()), (b"the baseline".to_vec(), b"its frames".to_vec()));
    std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&capture, std::fs::Permissions::from_mode(0o444)).unwrap();
    refused(&kept, "kept.pcap: ");
    assert_eq!((std::fs::read(&kept).unwrap(), std::fs::read(&capture).unwrap()), (b"the baseline".to_vec(), b"its frames".to_vec()));
    std::fs::set_permissions(&capture, std::fs::Permissions::from_mode(0o644)).unwrap();
    // A file name that is not UTF-8: the scan could not name its capture.
    let odd = d.join(not_utf8("odd.toml"));
    refused(&odd, "a scan file's name is UTF-8");
    assert!(!odd.exists() && !odd.with_extension("pcap").exists());
    // Nothing left behind under another name.
    let left: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains("partial")).collect();
    assert_eq!(left, Vec::<String>::new());
}

/// A segment whose switch passes LLDP (a bridge told to), both of this machine's interfaces on it:
/// each device heard on both, the two devices' reports read and drawn as one confirmed link, a
/// device stating no address heard but never read.
#[cfg(target_os = "linux")]
const SEGMENT: &str = r#"format = "veredas-bench"
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
interface = "c0"
station = "valvula-1"
ip = { address = "192.168.0.11", mask = "255.255.255.0" }
lldp_interval_ms = 200
port = "port-002"
seed = 2

[[profinet]]
name = "third"
interface = "d0"
station = "sensor-1"
lldp_interval_ms = 0
seed = 3
"#;

#[cfg(target_os = "linux")]
#[test]
fn a_scan_with_the_ports_read_draws_the_devices_own_word_on_their_cables() {
    use veredas_aranha::netns;
    use veredas_aranha::topology::{Confidence, End};
    const FIRST: Mac = Mac([0x02, 0x00, 0x5E, 0xB1, 0x00, 0x01]);
    const SECOND: Mac = Mac([0x02, 0x00, 0x5E, 0xB1, 0x00, 0x02]);
    const THIRD: Mac = Mac([0x02, 0x00, 0x5E, 0xB1, 0x00, 0x03]);
    const HERE0: Mac = Mac([0x02, 0x00, 0x5E, 0xB1, 0x00, 0xA0]);
    const HERE1: Mac = Mac([0x02, 0x00, 0x5E, 0xB1, 0x00, 0xA1]);
    bench::maquinar();
    let d = dir("segment");
    let seg = d.join("segment.toml");
    let pairs = [("a0", "ap"), ("a1", "aq"), ("b0", "bp"), ("c0", "cp"), ("d0", "dp")];
    netns::isolated("a_scan_with_the_ports_read_draws_the_devices_own_word_on_their_cables", &pairs, || {
        bench::pin(&[("a0", HERE0), ("a1", HERE1), ("b0", FIRST), ("c0", SECOND), ("d0", THIRD)], &pairs);
        netns::ip(&["link", "add", "br0", "type", "bridge", "group_fwd_mask", "0x4000"]);
        for port in ["ap", "aq", "bp", "cp", "dp"] {
            netns::ip(&["link", "set", port, "master", "br0"]);
        }
        netns::ip(&["link", "set", "br0", "up"]);
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        netns::wait_pairs(&[("a0", "b0"), ("a0", "c0"), ("a1", "d0"), ("b0", "c0")]);
        let listening = started(&["listen", "a0", "a1", "--for", "3", "--identify", "--ports", "--out", seg.to_str().unwrap()]);
        let run = bench::start("executable-segment", SEGMENT);
        let out = listening.wait_with_output().unwrap();
        let (status, o, e) = bench::end(run);
        assert!(status.success(), "the bench: {o}\n{e}");
        assert_eq!(out.status.code(), Some(0), "{}\n{}", text(&out.stdout), text(&out.stderr));
        // The device stating no address is never read (no line for it), the two others are.
        assert_eq!(text(&out.stderr), "a0: 3 device(s) answered the Identify\na1: 3 device(s) answered the Identify\n");
        let said = text(&out.stdout);
        for line in [format!("{FIRST} \"bomba-1\" 192.168.0.10 on a0,a1\n"), format!("{SECOND} \"valvula-1\" 192.168.0.11 on a0,a1\n"), format!("{THIRD} \"sensor-1\" 0.0.0.0 on a0,a1\n")] {
            assert!(said.contains(&line), "{line}: {said}");
        }
        let scan = Scan::read_file(&seg).unwrap();
        let end = |m: Mac, p: &str| End { node: veredas_aranha::topology::device_id(m), port: p.into() };
        let links: Vec<(End, End, Confidence)> = scan.topology.links.iter().map(|l| (l.a.clone(), l.b.clone(), l.confidence)).collect();
        assert_eq!(links, [(end(FIRST, "port-001"), end(SECOND, "port-002"), Confidence::Confirmed)], "{:#?}", scan.topology);
    });
}

/// A device whose second port's read goes unanswered: the walk is cut, said on stderr, and what
/// was read before it (its first port's neighbour) is drawn all the same.
#[cfg(target_os = "linux")]
#[test]
fn a_device_whose_walk_is_cut_keeps_what_was_read_of_it() {
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use veredas_aranha::ip::{self, Endpoint};
    use veredas_aranha::netns;
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
    use veredas_aranha::profinet::record::{LinkState, Peer, PortDataReal};
    use veredas_aranha::profinet::rpc::{self, PnioStatus, ReadRequest, RequestArgs, RpcPdu};
    use veredas_aranha::topology::{Confidence, End};
    const DEVICE: Mac = Mac([0x02, 0x00, 0x5E, 0xB2, 0x00, 0x01]);
    const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xB2, 0x00, 0xA0]);
    let d = dir("cut");
    let out_file = d.join("cut.toml");
    let pairs = [("a0", "b0")];
    netns::isolated("a_device_whose_walk_is_cut_keeps_what_was_read_of_it", &pairs, || {
        bench::pin(&[("a0", HERE), ("b0", DEVICE)], &pairs);
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        let stop = Arc::new(AtomicBool::new(false));
        let peer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut b0 = bench::link("b0");
                let me = Endpoint { mac: DEVICE, ip: Ipv4Addr::new(192, 168, 0, 10), port: 34964 };
                let identity = DeviceIdentity { name: Some("dev-s".into()), ip: Some(IpSuite { ip: me.ip, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
                let mut pdu = LldpPdu::new(ChassisId::Local { id: "dev-s".into() }, PortId::Local { id: "port-001.dev-s".into() }, 20);
                pdu.pn.chassis_mac = Some(DEVICE);
                let mut last = std::time::Instant::now() - Duration::from_secs(1);
                while !stop.load(Ordering::SeqCst) {
                    if last.elapsed() >= Duration::from_millis(200) {
                        b0.send(&dcp::hello(DEVICE, 1, &identity).unwrap()).unwrap();
                        b0.send(&lldp::frame(DEVICE, &pdu).unwrap()).unwrap();
                        last = std::time::Instant::now();
                    }
                    let Ok(Some(f)) = b0.recv(Duration::from_millis(10)) else { continue };
                    let Some(dg) = ip::datagram(&f.bytes) else { continue };
                    if dg.dst != me {
                        continue;
                    }
                    let req = RpcPdu::decode(&dg.payload).unwrap();
                    let args = RequestArgs::decode(&req.body, req.byte_order()).unwrap();
                    let read = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]).unwrap();
                    // Port 1 answered, with its neighbour; port 2 never.
                    if read.subslot != 0x8001 {
                        continue;
                    }
                    let port = PortDataReal {
                        slot: 0,
                        subslot: 0x8001,
                        own_port_id: "port-001".into(),
                        peers: vec![Peer { port_id: "port-009.sw-9".into(), chassis_id: "sw-9".into(), line_delay: 0, mac: Mac([0; 6]) }],
                        mau_type: 0x10,
                        domain_boundary: 0,
                        multicast_boundary: 0,
                        link_state: LinkState { port: 0, link: 1 },
                        media_type: 1,
                        extra: Vec::new(),
                    };
                    let answer = rpc::read_answer(&req, &read, PnioStatus::OK, &port.block().unwrap().encode().unwrap(), 7).unwrap();
                    b0.send(&ip::udp_frame(me, dg.src, 1, &answer.encode().unwrap()).unwrap()).unwrap();
                }
            })
        };
        let out = aranha(&["listen", "a0", "--for", "2", "--ports", "--out", out_file.to_str().unwrap()]);
        stop.store(true, Ordering::SeqCst);
        peer.join().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}\n{}", text(&out.stdout), text(&out.stderr));
        assert_eq!(text(&out.stderr), format!("{DEVICE} \"dev-s\": subslot 0x8002 not read: {DEVICE} did not answer the read within 1s of its going\n"));
        let scan = Scan::read_file(&out_file).unwrap();
        let links: Vec<(End, End, Confidence)> = scan.topology.links.iter().filter(|l| l.confidence != Confidence::Heard).map(|l| (l.a.clone(), l.b.clone(), l.confidence)).collect();
        let chassis = End { node: veredas_aranha::topology::chassis_id("sw-9"), port: "port-009".into() };
        assert_eq!(links, [(chassis, End { node: veredas_aranha::topology::device_id(DEVICE), port: "port-001".into() }, Confidence::Reported)]);
    });
}
