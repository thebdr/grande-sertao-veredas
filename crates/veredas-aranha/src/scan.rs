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

use crate::fusion::{Conflict, Device, Fusion};
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
        toml::Value::try_from(self).map_err(|e| ScanError::Write(e.to_string()))?;
        toml::to_string(self).map_err(|e| ScanError::Write(e.to_string()))
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
