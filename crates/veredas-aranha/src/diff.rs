//! The diff of a scan against a baseline (`docs/aranha-plan.md`, "Scope", scan files and the diff):
//! a baseline is a saved scan (`scan`, C-146), and the diff says what changed since, by the rules
//! the scan keeps ("Rules": it never guesses identity).
//!
//! - A device is its MAC: one in the scan only **appeared**, one in the baseline only **vanished**,
//!   whatever its name; a name is never a key, so a device that took another's name is that
//!   device renamed, never the other one moved.
//! - Of a device in both, what it states of itself now (each field's value heard last): its
//!   station name and LLDP system name **renamed**, its DCP IP suite, ARP address and LLDP
//!   management addresses **readdressed**, where both scans state the value and they differ (an
//!   empty name and 0.0.0.0 are values a device states); where one scan states it and the other
//!   does not, it is **one-sided**, neither a change nor silence: a scan that did not ask says
//!   nothing of what it was not told.
//! - A link is its two ends (node and port): one in the scan only was **added**, one in the
//!   baseline only **removed** (**rewired**, both together); its confidence and witnesses are
//!   evidence, not wiring, so a link reported then confirmed is no change. An end is its node's id,
//!   so a chassis node in one scan and a device in the other are two ends, never guessed one.
//! - A conflict is its kind, value and devices: one in the scan only is **new**, one in the
//!   baseline only **cleared**.
//! - An interface listened on by one scan only is said, so a device vanished with the interface it
//!   was heard on is read for what it is.
//!
//! Every list in a fixed order (devices by MAC, a device's changes by field, links by their ends,
//! conflicts by kind, value and devices, interfaces by name), so the same two scans give the same
//! diff whatever order they hold their conflicts and a conflict's devices in (their devices and
//! links come in the one order a scan holds them in, any other refused), and the diff of the two
//! the other way round is its mirror.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::fusion::{Conflict, Device};
use crate::profinet::mac::Mac;
use crate::scan::{Scan, ScanError};
use crate::topology::{End, Link};

/// A device appeared or vanished: its MAC, and what its own scan calls it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceRef {
    pub mac: Mac,
    pub label: String,
}

/// What a device states of itself that the diff compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Said {
    /// Its DCP station name.
    StationName,
    /// Its LLDP system name.
    SystemName,
    /// Its DCP IP suite: `address mask gateway`.
    IpSuite,
    /// The address its ARP gives.
    ArpAddress,
    /// Its LLDP management addresses, `, `-separated in order.
    ManagementAddresses,
}

impl Said {
    /// Renamed, else readdressed.
    pub fn is_name(self) -> bool {
        matches!(self, Said::StationName | Said::SystemName)
    }
}

/// A value of a device in both scans that both state, and that differs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Change {
    pub mac: Mac,
    pub what: Said,
    pub from: String,
    pub to: String,
}

/// A value of a device in both scans that one scan states and the other does not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OneSided {
    pub mac: Mac,
    pub what: Said,
    pub baseline: Option<String>,
    pub scan: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub appeared: Vec<DeviceRef>,
    pub vanished: Vec<DeviceRef>,
    pub renamed: Vec<Change>,
    pub readdressed: Vec<Change>,
    pub one_sided: Vec<OneSided>,
    /// Rewired: the links of the scan only, as the scan holds them.
    pub links_added: Vec<Link>,
    /// Rewired: the links of the baseline only, as the baseline holds them.
    pub links_removed: Vec<Link>,
    pub conflicts_new: Vec<Conflict>,
    pub conflicts_cleared: Vec<Conflict>,
    /// Interfaces the scan listened on and the baseline did not.
    pub interfaces_added: Vec<String>,
    /// Interfaces the baseline listened on and the scan did not.
    pub interfaces_removed: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("the baseline: {0}")]
    Baseline(ScanError),
    #[error("the scan: {0}")]
    Scan(ScanError),
}

impl Diff {
    /// Nothing changed, and nothing was stated by one scan alone.
    pub fn is_empty(&self) -> bool {
        *self == Diff::default()
    }

    /// A change of what was heard: a device appeared, vanished, renamed or readdressed, a link
    /// added or removed, a conflict new or cleared. A value one scan alone states, and an
    /// interface one scan alone listened on, are said and are no change (`docs/aranha-plan.md`:
    /// neither a change nor silence).
    pub fn has_change(&self) -> bool {
        let Diff { appeared, vanished, renamed, readdressed, one_sided: _, links_added, links_removed, conflicts_new, conflicts_cleared, interfaces_added: _, interfaces_removed: _ } = self;
        !(appeared.is_empty() && vanished.is_empty() && renamed.is_empty() && readdressed.is_empty() && links_added.is_empty() && links_removed.is_empty() && conflicts_new.is_empty() && conflicts_cleared.is_empty())
    }
}

/// What `d` states of `what` now, as text (none when it states nothing of it).
fn said(d: &Device, what: Said) -> Option<String> {
    match what {
        Said::StationName => d.name.current().cloned(),
        Said::SystemName => d.sys_name.current().cloned(),
        Said::IpSuite => d.ip.current().map(|s| format!("{} {} {}", s.ip, s.mask, s.gateway)),
        Said::ArpAddress => d.arp_ip.current().map(|ip| ip.to_string()),
        Said::ManagementAddresses => d.mgmt_ips.current().map(|set| set.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join(", ")),
    }
}

const SAID: [Said; 5] = [Said::StationName, Said::SystemName, Said::IpSuite, Said::ArpAddress, Said::ManagementAddresses];

/// The diff of `scan` against `baseline`, both scans (`Scan::check`), or refused naming which is
/// not.
pub fn diff(baseline: &Scan, scan: &Scan) -> Result<Diff, DiffError> {
    baseline.check().map_err(DiffError::Baseline)?;
    scan.check().map_err(DiffError::Scan)?;
    let base: BTreeMap<Mac, &Device> = baseline.devices.iter().map(|d| (d.mac, d)).collect();
    let now: BTreeMap<Mac, &Device> = scan.devices.iter().map(|d| (d.mac, d)).collect();
    let mut out = Diff::default();
    for (mac, d) in &now {
        let Some(b) = base.get(mac) else {
            out.appeared.push(DeviceRef { mac: *mac, label: d.label() });
            continue;
        };
        for what in SAID {
            match (said(b, what), said(d, what)) {
                (Some(from), Some(to)) if from != to => {
                    let c = Change { mac: *mac, what, from, to };
                    if what.is_name() { out.renamed.push(c) } else { out.readdressed.push(c) }
                }
                (Some(_), Some(_)) | (None, None) => {}
                (baseline, scan) => out.one_sided.push(OneSided { mac: *mac, what, baseline, scan }),
            }
        }
    }
    for (mac, b) in &base {
        if !now.contains_key(mac) {
            out.vanished.push(DeviceRef { mac: *mac, label: b.label() });
        }
    }
    let ends = |l: &Link| (l.a.clone(), l.b.clone());
    let links = |s: &Scan| -> BTreeMap<(End, End), Link> { s.topology.links.iter().map(|l| (ends(l), l.clone())).collect() };
    let (bl, sl) = (links(baseline), links(scan));
    out.links_added = sl.iter().filter(|(k, _)| !bl.contains_key(*k)).map(|(_, l)| l.clone()).collect();
    out.links_removed = bl.iter().filter(|(k, _)| !sl.contains_key(*k)).map(|(_, l)| l.clone()).collect();
    let conflicts = |s: &Scan| -> BTreeSet<Conflict> {
        s.conflicts
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.macs.sort();
                c.macs.dedup();
                c
            })
            .collect()
    };
    let (bc, sc) = (conflicts(baseline), conflicts(scan));
    out.conflicts_new = sc.difference(&bc).cloned().collect();
    out.conflicts_cleared = bc.difference(&sc).cloned().collect();
    let (bi, si): (BTreeSet<&String>, BTreeSet<&String>) = (baseline.stats.interfaces.keys().collect(), scan.stats.interfaces.keys().collect());
    out.interfaces_added = si.difference(&bi).map(|s| s.to_string()).collect();
    out.interfaces_removed = bi.difference(&si).map(|s| s.to_string()).collect();
    Ok(out)
}
