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
        (vec!["diff", s], "diff takes two scan files"),
        (vec!["diff", s, s], "x.toml: "),
    ] {
        let out = aranha(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out.stderr));
        assert!(text(&out.stderr).starts_with("veredas-aranha: ") && text(&out.stderr).contains(says), "{args:?}: {}", text(&out.stderr));
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert!(!scan.exists());
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
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        netns::ip(&["addr", "add", "192.168.0.2/24", "dev", "a1"]);
        // An interface that is not there is named; nothing is written.
        let nowhere = d.join("nowhere.toml");
        let out = aranha(&["listen", "nosuch0", "--for", "1", "--out", nowhere.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(2));
        assert!(text(&out.stderr).starts_with("veredas-aranha: nosuch0: "), "{}", text(&out.stderr));
        assert!(!nowhere.exists() && !d.join("nowhere.pcap").exists());

        // The first scan: both devices, the second's name found by asking alone (it says no
        // Hello), each one's ports read.
        let listening = started(&["listen", "a0", "a1", "--for", "3", "--identify", "--ports", "--out", before.to_str().unwrap()]);
        let run = bench::start("executable-before", BEFORE);
        let out = listening.wait_with_output().unwrap();
        let (status, o, e) = bench::end(run);
        assert!(status.success(), "the bench: {o}\n{e}");
        assert_eq!(out.status.code(), Some(0), "{}\n{}", text(&out.stdout), text(&out.stderr));
        let said = text(&out.stdout);
        assert!(said.contains(&format!("{FIRST} \"bomba-1\" 192.168.0.10 on a0\n")) && said.contains(&format!("{SECOND} \"bomba-1\" 192.168.0.11 on a1\n")), "{said}");
        assert!(said.contains(&format!("conflict duplicate-name \"bomba-1\": {FIRST}, {SECOND}\n")), "{said}");
        assert!(said.ends_with(&format!("2 device(s), 2 link(s), {} frame(s) heard; {} and {}\n", Scan::read_file(&before).unwrap().stats.heard, before.display(), d.join("before.pcap").display())), "{said}");
        assert_eq!(text(&out.stderr), "a0: 1 device(s) answered the Identify\na1: 1 device(s) answered the Identify\n");

        // The file: read back whole, its capture beside it holding every frame its witnesses name,
        // the Identify and the reads among what was sent.
        let scan = Scan::read_file(&before).unwrap();
        assert_eq!(scan.devices.iter().map(|d| d.mac).collect::<Vec<_>>(), [FIRST, SECOND]);
        let capture = scan.capture.clone().unwrap();
        assert_eq!(capture.file, "before.pcap");
        let frames = pcap::read_file(&d.join(&capture.file), "a0").unwrap().frames;
        assert_eq!(frames.len() as u64, capture.frames);
        assert!(scan.stats.interfaces.values().all(|i| i.sent >= 2), "an Identify and at least a read sent on each: {:?}", scan.stats.interfaces);
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
        assert_eq!(reads, 4, "each device's port 0x8001 read, and 0x8002 refused");

        // The second, the bench changed: the first renamed, readdressed and its cable on port 2,
        // the second gone; and an identify by name.
        let listening = started(&["listen", "a0", "a1", "--for", "2", "--out", after.to_str().unwrap()]);
        let run = bench::start("executable-after", AFTER);
        let out = listening.wait_with_output().unwrap();
        let named = aranha(&["identify", "a0", "--name", "bomba-2", "--wait", "300"]);
        let nobody = aranha(&["identify", "a0", "--name", "bomba-1", "--wait", "300"]);
        let (status, o, e) = bench::end(run);
        assert!(status.success(), "the bench: {o}\n{e}");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        assert_eq!(named.status.code(), Some(0), "{}", text(&named.stderr));
        let lines: Vec<String> = text(&named.stdout).lines().map(String::from).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with(&format!("{FIRST} \"bomba-2\" 192.168.0.20 ids ")), "{lines:?}");
        assert_eq!(lines[1], "1 answer(s) on a0, 0 other(s), 0 lost");
        // A name no device holds now: no answer.
        assert_eq!((nobody.status.code(), text(&nobody.stdout)), (Some(0), "0 answer(s) on a0, 0 other(s), 0 lost\n".to_string()));
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
            format!("link added {first} port-002 = if:a0 (heard)"),
            format!("link removed {first} port-001 = if:a0 (heard)"),
            format!("link removed {second} port-001 = if:a1 (heard)"),
            format!("conflict cleared duplicate-name \"bomba-1\": {FIRST}, {SECOND}"),
            String::new(),
        ]
        .join("\n")
    );
    let same = aranha(&["diff", before.to_str().unwrap(), before.to_str().unwrap()]);
    assert_eq!((same.status.code(), text(&same.stdout)), (Some(0), "no change\n".to_string()));
}
