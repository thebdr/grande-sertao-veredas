//! The scan file (`docs/aranha-plan.md`, "Scope", scan files; `docs/scan-schema.md`): what a
//! listening found, written as a versioned TOML file (`veredas-scan`, law L7) beside the capture of
//! its frames, and read back whole. It holds the listener's counts (C-142), the devices with every
//! value and its witnesses (C-141), their conflicts, and the topology (C-145); a witness names a
//! frame by its number, and the capture beside it says which number its first record has, so every
//! value can be checked against the frame it came from. The header is read first by the suite's one
//! reader (core's `schema`): another format, an unknown version or a missing stamp is refused by
//! name; then the body, every key read or the file refused, naming each key nothing took.

use std::path::Path;

use serde::{Deserialize, Serialize};
use veredas_core::schema::{self, HeaderError};

use crate::fusion::{Conflict, Device, Field, Fusion};
use crate::listen::Stats;
use crate::topology::Topology;

/// The format every scan file states.
pub const FORMAT: &str = "veredas-scan";
/// The one version this reader knows.
pub const VERSION: u32 = 1;
const NOUN: &str = "a scan file";

/// The capture of a scan's frames, beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    /// Its file's name, in the scan file's folder.
    pub file: String,
    /// The number of its first record: a witness's frame `n` is its record `n - first_frame`.
    pub first_frame: u64,
    /// How many records it holds.
    pub frames: u64,
}

/// A scan, as its file holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    pub format: String,
    pub version: u32,
    /// What took it: `veredas-aranha <version>`.
    pub tool: String,
    /// When it was taken (written), in nanoseconds since the epoch, the frames' clock.
    pub taken_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
    pub stats: Stats,
    /// By MAC.
    pub devices: Vec<Device>,
    pub conflicts: Vec<Conflict>,
    pub topology: Topology,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{0}")]
    Header(#[from] HeaderError),
    #[error("not a scan file: {0}")]
    Toml(String),
    #[error("a scan file holds no key {}: refused, not ignored", .0.join(", "))]
    Unread(Vec<String>),
    /// Read, but not as the file holds it: an element past what its list takes, a value of
    /// another shape (a table written as a list, a word as a table), a repeat a set keeps once, a
    /// number past TOML's integers.
    #[error("a scan file's {0} is not what its reader keeps: refused, not read otherwise")]
    NotWhole(String),
    /// Read whole, but no scan: a device twice or out of order, a frame past its capture's.
    #[error("not a scan: {0}")]
    Invalid(String),
    #[error("a scan cannot be written: {0}")]
    Write(String),
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
}

impl Scan {
    /// A scan of what fusion holds, the listener's counts, the conflicts and the topology, taken at
    /// `taken_ns`, its frames in `capture` when one is written.
    pub fn new(fusion: &Fusion, stats: &Stats, topology: &Topology, taken_ns: u64, capture: Option<Capture>) -> Scan {
        Scan {
            format: FORMAT.to_string(),
            version: VERSION,
            tool: format!("veredas-aranha {}", env!("CARGO_PKG_VERSION")),
            taken_ns,
            capture,
            stats: stats.clone(),
            devices: fusion.devices.values().cloned().collect(),
            conflicts: fusion.conflicts(),
            topology: topology.clone(),
        }
    }

    /// The devices again, as fusion holds them.
    pub fn fusion(&self) -> Fusion {
        Fusion { devices: self.devices.iter().map(|d| (d.mac, d.clone())).collect() }
    }

    /// The file's text. Every number is one of TOML's integers (64 bits, signed): a scan holding
    /// one past them (a stamp from 2262 on) is refused, never written for another reader to refuse.
    pub fn to_toml(&self) -> Result<String, ScanError> {
        self.check().map_err(|e| ScanError::Write(e.to_string()))?;
        toml::Value::try_from(self).map_err(|e| ScanError::Write(e.to_string()))?;
        toml::to_string(self).map_err(|e| ScanError::Write(e.to_string()))
    }

    /// What makes it a scan beyond its keys: its devices by MAC, each once; its topology's links
    /// each stored lesser end first and by their ends, each pair of ends once (a link is its two
    /// ends, so one cable is one link); no witness, and no port's last leaving or other frame,
    /// naming a frame past its capture's last record (one before its first is a frame the listener
    /// let go, counted in its stats, which the capture does not hold).
    pub fn check(&self) -> Result<(), ScanError> {
        for pair in self.devices.windows(2) {
            if pair[0].mac >= pair[1].mac {
                let why = if pair[0].mac == pair[1].mac { "is in it twice" } else { "is out of the order by MAC" };
                return Err(ScanError::Invalid(format!("device {} {why}", pair[1].mac)));
            }
        }
        let ends = |l: &crate::topology::Link| format!("{} {} = {} {}", l.a.node, l.a.port, l.b.node, l.b.port);
        if let Some(l) = self.topology.links.iter().find(|l| l.a > l.b) {
            return Err(ScanError::Invalid(format!("link {} is stored greater end first", ends(l))));
        }
        for pair in self.topology.links.windows(2) {
            let (x, y) = ((&pair[0].a, &pair[0].b), (&pair[1].a, &pair[1].b));
            if x >= y {
                let why = if x == y { "is in it twice" } else { "is out of the order by its ends" };
                return Err(ScanError::Invalid(format!("link {} {why}", ends(&pair[1]))));
            }
        }
        if let Some(c) = &self.capture {
            let end = c.first_frame.saturating_add(c.frames);
            for d in &self.devices {
                if let Some(frame) = frames_of(d).find(|f| *f >= end) {
                    return Err(ScanError::Invalid(format!("device {} names frame {frame}, past its capture's last ({})", d.mac, end.saturating_sub(1))));
                }
            }
        }
        Ok(())
    }

    /// Read a scan file's text: its header first, then its body whole, every key taken.
    pub fn read(text: &str) -> Result<Scan, ScanError> {
        schema::check(text, NOUN, FORMAT, VERSION)?;
        let de = toml::Deserializer::parse(text).map_err(|e| ScanError::Toml(e.to_string()))?;
        let mut unread = Vec::new();
        let scan: Scan = serde_ignored::deserialize(de, |path| unread.push(format!("`{path}`"))).map_err(|e| ScanError::Toml(e.to_string()))?;
        if !unread.is_empty() {
            return Err(ScanError::Unread(unread));
        }
        // Whole: what was read is what the file holds, written back as TOML's own values.
        let held: toml::Table = text.parse().map_err(|e: toml::de::Error| ScanError::Toml(e.to_string()))?;
        let kept = toml::Value::try_from(&scan).map_err(|e| ScanError::NotWhole(format!("value ({e})")))?;
        if let Some(at) = first_difference(&toml::Value::Table(held), &kept, String::new()) {
            return Err(ScanError::NotWhole(at));
        }
        scan.check()?;
        Ok(scan)
    }

    pub fn write_file(&self, path: &Path) -> Result<(), ScanError> {
        let text = self.to_toml()?;
        std::fs::write(path, text).map_err(|source| ScanError::Io { path: path.display().to_string(), source })
    }

    pub fn read_file(path: &Path) -> Result<Scan, ScanError> {
        let text = std::fs::read_to_string(path).map_err(|source| ScanError::Io { path: path.display().to_string(), source })?;
        Scan::read(&text)
    }
}

/// Every frame number a device names: its fields' witnesses and its ports' last leaving and
/// other frames.
fn frames_of(d: &Device) -> impl Iterator<Item = u64> + '_ {
    fn of<T>(f: &Field<T>) -> impl Iterator<Item = u64> + '_ {
        f.candidates.iter().flat_map(|c| [c.first.frame, c.last.frame])
    }
    let own = of(&d.name)
        .chain(of(&d.alias))
        .chain(of(&d.ip))
        .chain(of(&d.ip_status))
        .chain(of(&d.pn_id))
        .chain(of(&d.vendor))
        .chain(of(&d.role))
        .chain(of(&d.instance))
        .chain(of(&d.chassis))
        .chain(of(&d.sys_name))
        .chain(of(&d.sys_desc))
        .chain(of(&d.sys_caps))
        .chain(of(&d.mgmt_ips))
        .chain(of(&d.arp_ip));
    let ports = d.ports.values().flat_map(|p| {
        of(&p.port_mac)
            .chain(of(&p.desc))
            .chain(of(&p.pn_status))
            .chain(of(&p.delays))
            .chain(of(&p.mrp))
            .chain(of(&p.pvid))
            .chain(of(&p.mau_type))
            .chain(p.leaving.into_iter().chain(p.spoke).map(|(_, frame)| frame))
    });
    own.chain(ports)
}

/// The first place, as a key path in backticks, where what the file holds and what was kept of it
/// differ.
fn first_difference(held: &toml::Value, kept: &toml::Value, at: String) -> Option<String> {
    let here = |at: &str| if at.is_empty() { "body".to_string() } else { format!("`{at}`") };
    let key = |at: &str, k: &str| if at.is_empty() { k.to_string() } else { format!("{at}.{k}") };
    match (held, kept) {
        (toml::Value::Table(h), toml::Value::Table(k)) => {
            for name in h.keys().chain(k.keys().filter(|n| !h.contains_key(*n))) {
                match (h.get(name), k.get(name)) {
                    (Some(a), Some(b)) => {
                        if let Some(d) = first_difference(a, b, key(&at, name)) {
                            return Some(d);
                        }
                    }
                    _ => return Some(here(&key(&at, name))),
                }
            }
            None
        }
        (toml::Value::Array(h), toml::Value::Array(k)) => {
            if h.len() != k.len() {
                return Some(here(&at));
            }
            h.iter().zip(k).enumerate().find_map(|(i, (a, b))| first_difference(a, b, key(&at, &i.to_string())))
        }
        (a, b) if a == b => None,
        _ => Some(here(&at)),
    }
}
