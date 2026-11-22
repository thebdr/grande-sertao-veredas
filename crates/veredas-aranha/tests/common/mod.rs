//! What the codec tests share: a classic pcap writer, and the second reading of the standard,
//! Wireshark's dissectors (`tshark`), which read back every frame an encoder writes.
//!
//! Where tshark is: `VEREDAS_TSHARK` when set; else `~/.local/opt/wireshark/tshark` (extracted
//! from Ubuntu's archive into the home folder, no root, `docs/aranha-plan.md`, "Rules"); else
//! `tshark` on the `PATH`. When none runs, a test that needs it fails and says so by name: never a
//! skip, never a pass.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A classic pcap file (microsecond stamps, link type Ethernet) holding `frames`, one second
/// apart from 1 700 000 000.
pub fn pcap(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&65535u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        out.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// The tshark this machine has, or a failure that names what is missing.
pub fn tshark() -> PathBuf {
    let candidates: Vec<PathBuf> = [
        std::env::var_os("VEREDAS_TSHARK").map(PathBuf::from),
        std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/opt/wireshark/tshark")),
        Some(PathBuf::from("tshark")),
    ]
    .into_iter()
    .flatten()
    .collect();
    for c in &candidates {
        if Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            return c.clone();
        }
    }
    panic!(
        "the dissectors are this test's second reading of the standard, and no tshark runs here: tried {candidates:?}. \
         Set VEREDAS_TSHARK, extract Ubuntu's tshark packages into ~/.local/opt/wireshark (docs/aranha-plan.md), or install tshark."
    );
}

/// One packet as tshark's dissectors read it: its layers, by protocol (`-T ek`).
pub struct Dissected {
    pub layers: Value,
}

impl Dissected {
    /// Every value of `field` (Wireshark's dotted name, `pn_dcp.xid`) in protocol `proto`, in
    /// the order the frame holds them; empty when the dissector found none.
    pub fn values(&self, proto: &str, field: &str) -> Vec<String> {
        let key = format!("{proto}_{}", field.replace('.', "_"));
        let mut out = Vec::new();
        collect(&self.layers[proto], &key, &mut out);
        out
    }

    /// The one value of `field`, failing when there is none or more than one.
    pub fn one(&self, proto: &str, field: &str) -> String {
        let v = self.values(proto, field);
        assert_eq!(v.len(), 1, "{proto} {field}: {v:?} in {}", self.layers);
        v[0].clone()
    }

    pub fn has(&self, proto: &str) -> bool {
        !self.layers[proto].is_null()
    }

    /// What the dissectors complained of: a malformed packet, or expert information at warning
    /// or error severity, anywhere in the packet.
    pub fn complaints(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.layers["_ws_malformed"].is_null() {
            out.push(format!("malformed: {}", self.layers["_ws_malformed"]));
        }
        expert(&self.layers, &mut out);
        out
    }
}

fn collect(v: &Value, key: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if k == key {
                    match x {
                        Value::Array(items) => out.extend(items.iter().map(scalar)),
                        other => out.push(scalar(other)),
                    }
                } else {
                    collect(x, key, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|x| collect(x, key, out)),
        _ => {}
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Expert entries at warning (6291456) or error (8388608) severity.
fn expert(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if k.ends_with("_ws_expert_severity") {
                    let sev: u64 = scalar(x).parse().unwrap_or(0);
                    if sev >= 6_291_456 {
                        out.push(format!("expert at severity {sev}: {v}"));
                    }
                }
                expert(x, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|x| expert(x, out)),
        _ => {}
    }
}

/// Read `frames` through tshark: one `Dissected` per frame, in order.
pub fn dissect(frames: &[Vec<u8>]) -> Vec<Dissected> {
    let tshark = tshark();
    let dir = std::env::temp_dir().join(format!("veredas-aranha-dissect-{}-{}", std::process::id(), unique()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("frames.pcap");
    std::fs::write(&file, pcap(frames)).unwrap();
    let out = Command::new(&tshark).arg("-n").arg("-r").arg(&file).args(["-T", "ek"]).output().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
    let packets: Vec<Dissected> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("{\"index\""))
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            Dissected { layers: v["layers"].clone() }
        })
        .collect();
    assert_eq!(packets.len(), frames.len(), "tshark read {} packets of {}", packets.len(), frames.len());
    packets
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// A small deterministic generator (a 64-bit LCG), for the byte noise a decoder must survive.
pub struct Noise(pub u64);

impl Noise {
    pub fn next(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u8
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next()).collect()
    }
}
