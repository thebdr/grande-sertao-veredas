//! Running the bench's PROFINET devices for a test (C-143): maquinar's own executable, built
//! from this workspace's sources into a target folder of its own and started on a bench file in
//! the test's private network, and the raw links the listener hears them on.

#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use veredas_aranha::netns;
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::Wire;
use veredas_aranha::wire::raw::{Options as RawOptions, RawLink};

/// Where the bench's executable is built: a target folder of its own under the one cargo gives
/// integration tests, so building it never replaces the executable the bench's own tests run from
/// the workspace's target folder (a build of one package resolves features apart from the
/// workspace's, and cargo puts its executable in place again).
pub fn bench_target() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("bench-device/target")
}

/// The bench's executable, built from this workspace's sources (offline, eight jobs) on the host,
/// before the private network is made; in the private network, the same path.
pub fn maquinar() -> PathBuf {
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

/// An interface's MAC, as the kernel gives it to a raw link (the private network's `/sys` is the
/// host's, its interfaces not there).
pub fn mac_of(iface: &str) -> Mac {
    RawLink::open_with(iface, RawOptions::default()).unwrap().mac()
}

pub fn link(iface: &str) -> Box<dyn Wire> {
    Box::new(RawLink::open_with(iface, RawOptions { ethertype: None, promiscuous: false, outgoing: false }).unwrap())
}

/// End the bench as its user does, closing its stdin; its exit status and what it printed.
pub fn end(mut bench: Child) -> (ExitStatus, String, String) {
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

/// The bench run on `file` (written under `name` to the folder cargo gives integration tests),
/// its stdin, stdout and stderr held by the test.
pub fn start(name: &str, file: &str) -> Child {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bench-device");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.toml"));
    std::fs::write(&path, file).unwrap();
    Command::new(maquinar()).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
}
