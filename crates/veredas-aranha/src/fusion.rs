//! Observations into devices (`docs/aranha-plan.md`, "Scope", fusion; "Rules": it never guesses
//! identity). The MAC an observation names is the device; an address is a value of it and a name
//! is never a key. Every field keeps every value it was given, each with the first and last witness
//! and how often, so a disagreement is a conflict on record, never a silent overwrite; its current
//! value is the one heard last.
//!
//! Applying the same observations in the same order gives the same devices: a replay of a capture
//! reads as the listening did.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::observe::{Observation, Observed, Probe, Witness};
use crate::profinet::dcp::{IpStatus, IpSuite};
use crate::profinet::lldp::{Delays, MrpPortStatus, PortStatus};
use crate::profinet::mac::Mac;

/// One value a field was given, and by whom: the first and the last witness, and how many.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate<T> {
    pub value: T,
    pub first: Witness,
    pub last: Witness,
    pub count: u64,
}

/// A field with its provenance: its candidates in the order first heard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field<T> {
    pub candidates: Vec<Candidate<T>>,
}

impl<T> Default for Field<T> {
    fn default() -> Self {
        Field { candidates: Vec::new() }
    }
}

/// A witness's place in time: its stamp, then its frame's index (two frames of one stamp in the
/// order kept).
fn when(w: &Witness) -> (u64, u64) {
    (w.at_ns, w.frame)
}

impl<T: PartialEq + Clone> Field<T> {
    /// Record `value` as `w` heard it.
    pub fn observe(&mut self, value: T, w: &Witness) {
        match self.candidates.iter_mut().find(|c| c.value == value) {
            Some(c) => {
                c.count += 1;
                if when(w) >= when(&c.last) {
                    c.last = w.clone();
                }
                if when(w) < when(&c.first) {
                    c.first = w.clone();
                }
            }
            None => self.candidates.push(Candidate { value, first: w.clone(), last: w.clone(), count: 1 }),
        }
    }

    /// The value heard last.
    pub fn current(&self) -> Option<&T> {
        self.candidates.iter().max_by_key(|c| when(&c.last)).map(|c| &c.value)
    }

    /// More than one value on record.
    pub fn conflicted(&self) -> bool {
        self.candidates.len() > 1
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// One port of a device, as its own LLDP frames describe it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// Its short name (`port-001`; a PROFINET port's station suffix left out).
    pub id: String,
    /// The MAC its frames come from.
    pub port_mac: Field<Mac>,
    pub desc: Field<String>,
    pub pn_status: Field<PortStatus>,
    pub delays: Field<Delays>,
    pub mrp: Field<MrpPortStatus>,
    pub pvid: Field<u16>,
    pub mau_type: Field<u16>,
    /// The interfaces its frames were heard on: LLDP is link-local, so a port heard on one of this
    /// machine's interfaces is cabled to it.
    pub heard_on: BTreeSet<String>,
    /// When it said it was leaving (a time to live of 0), unless it spoke again after.
    pub left_ns: Option<u64>,
    pub first_ns: u64,
    pub last_ns: u64,
    pub frames: u64,
}

/// One device: a MAC and everything said of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub mac: Mac,
    /// DCP's NameOfStation.
    pub name: Field<String>,
    pub alias: Field<String>,
    /// DCP's IP suite, and the device's own word on it.
    pub ip: Field<IpSuite>,
    pub ip_status: Field<IpStatus>,
    /// DCP's vendor and device ids.
    pub pn_id: Field<(u16, u16)>,
    pub vendor: Field<String>,
    pub role: Field<u8>,
    pub instance: Field<(u8, u8)>,
    /// LLDP's chassis id, as a key (`mac:...`, `local:...`): what a neighbour table names it by.
    pub chassis: Field<String>,
    pub sys_name: Field<String>,
    pub sys_desc: Field<String>,
    pub sys_caps: Field<(u16, u16)>,
    /// LLDP's management addresses (IPv4).
    pub mgmt_ips: BTreeSet<Ipv4Addr>,
    /// The addresses its ARP claimed: a second, independent witness of its address.
    pub arp_ip: Field<Ipv4Addr>,
    pub ports: BTreeMap<String, Port>,
    /// What it was heard doing: answering DCP, asking DCP, LLDP, ARP.
    pub heard: BTreeSet<Probe>,
    pub heard_on: BTreeSet<String>,
    pub first_ns: u64,
    pub last_ns: u64,
    pub frames: u64,
}

impl Device {
    fn new(mac: Mac, at_ns: u64) -> Device {
        Device {
            mac,
            name: Field::default(),
            alias: Field::default(),
            ip: Field::default(),
            ip_status: Field::default(),
            pn_id: Field::default(),
            vendor: Field::default(),
            role: Field::default(),
            instance: Field::default(),
            chassis: Field::default(),
            sys_name: Field::default(),
            sys_desc: Field::default(),
            sys_caps: Field::default(),
            mgmt_ips: BTreeSet::new(),
            arp_ip: Field::default(),
            ports: BTreeMap::new(),
            heard: BTreeSet::new(),
            heard_on: BTreeSet::new(),
            first_ns: at_ns,
            last_ns: at_ns,
            frames: 0,
        }
    }

    /// What to call it: its station name, else its LLDP system name, else its MAC.
    pub fn label(&self) -> String {
        self.name.current().or_else(|| self.sys_name.current()).cloned().unwrap_or_else(|| self.mac.to_string())
    }

    /// The addresses it holds now: its DCP address, its ARP address and its management addresses,
    /// 0.0.0.0 left out.
    pub fn addresses(&self) -> BTreeSet<Ipv4Addr> {
        let mut s: BTreeSet<Ipv4Addr> = self.mgmt_ips.clone();
        s.extend(self.ip.current().map(|suite| suite.ip));
        s.extend(self.arp_ip.current().copied());
        s.remove(&Ipv4Addr::UNSPECIFIED);
        s
    }

    fn touch(&mut self, w: &Witness) {
        self.first_ns = self.first_ns.min(w.at_ns);
        self.last_ns = self.last_ns.max(w.at_ns);
        self.frames += 1;
        self.heard.insert(w.probe);
        self.heard_on.insert(w.iface.clone());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    /// Two devices name themselves alike.
    DuplicateName,
    /// Two devices hold one address.
    DuplicateIp,
    /// One device has named itself two ways.
    NameChanged,
    /// One device has held two DCP addresses.
    IpChanged,
    /// A device flags its own address as in conflict (DCP's BlockInfo).
    ReportsIpConflict,
}

/// A disagreement on record: its kind, the value it is about, and the devices.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub value: String,
    pub macs: Vec<Mac>,
}

/// The devices heard, by MAC.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fusion {
    pub devices: BTreeMap<Mac, Device>,
}

impl Fusion {
    pub fn new() -> Fusion {
        Fusion::default()
    }

    /// The device one of whose ports a MAC is (its LLDP frames come from it), when exactly one is.
    pub fn port_owner(&self, port_mac: Mac) -> Option<Mac> {
        let mut owners = self.devices.values().filter(|d| d.ports.values().any(|p| p.port_mac.current() == Some(&port_mac))).map(|d| d.mac);
        let first = owners.next()?;
        owners.next().is_none().then_some(first)
    }

    /// Apply one observation to the device its subject names; an LLDP PDU that names no MAC for
    /// its chassis (a shutdown PDU holds the mandatory three alone) to the device its source is
    /// already a port of, when one is, by that MAC.
    pub fn apply(&mut self, o: &Observation) {
        let w = &o.witness;
        let mac = match &o.observed {
            Observed::Lldp { src, .. } if o.observed.subject() == *src => self.port_owner(*src).unwrap_or(*src),
            other => other.subject(),
        };
        let d = self.devices.entry(mac).or_insert_with(|| Device::new(mac, w.at_ns));
        d.touch(w);
        match &o.observed {
            // (A request's identity is empty: the asker is on record, nothing of it is said.)
            Observed::Dcp { identity, .. } => {
                if let Some(v) = &identity.name {
                    d.name.observe(v.clone(), w);
                }
                if let Some(v) = &identity.alias {
                    d.alias.observe(v.clone(), w);
                }
                if let Some(v) = &identity.ip {
                    d.ip.observe(v.clone(), w);
                }
                if let Some(v) = identity.ip_status {
                    d.ip_status.observe(v, w);
                }
                if let (Some(vendor), Some(device)) = (identity.vendor_id, identity.device_id) {
                    d.pn_id.observe((vendor, device), w);
                }
                if let Some(v) = &identity.vendor {
                    d.vendor.observe(v.clone(), w);
                }
                if let Some(v) = identity.role {
                    d.role.observe(v, w);
                }
                if let Some(v) = identity.instance {
                    d.instance.observe(v, w);
                }
            }
            Observed::Lldp { src, pdu } => {
                d.chassis.observe(pdu.chassis.key(), w);
                if let Some(v) = &pdu.sys_name {
                    d.sys_name.observe(v.clone(), w);
                }
                if let Some(v) = &pdu.sys_desc {
                    d.sys_desc.observe(v.clone(), w);
                }
                if let Some(v) = pdu.sys_caps {
                    d.sys_caps.observe(v, w);
                }
                d.mgmt_ips.extend(pdu.mgmt_addrs.iter().filter_map(|m| m.ipv4()));
                let id = pdu.port.short();
                let port = d.ports.entry(id.clone()).or_insert_with(|| Port { id, first_ns: w.at_ns, last_ns: w.at_ns, ..Port::default() });
                port.first_ns = port.first_ns.min(w.at_ns);
                port.last_ns = port.last_ns.max(w.at_ns);
                port.frames += 1;
                port.heard_on.insert(w.iface.clone());
                port.port_mac.observe(*src, w);
                if let Some(v) = &pdu.port_desc {
                    port.desc.observe(v.clone(), w);
                }
                if let Some(v) = pdu.pn.port_status {
                    port.pn_status.observe(v, w);
                }
                if let Some(v) = pdu.pn.delays {
                    port.delays.observe(v, w);
                }
                if let Some(v) = &pdu.pn.mrp {
                    port.mrp.observe(v.clone(), w);
                }
                if let Some(v) = pdu.port_vlan {
                    port.pvid.observe(v, w);
                }
                if let Some(m) = &pdu.mac_phy {
                    port.mau_type.observe(m.mau_type, w);
                }
                // Leaving, or back: whichever was said last.
                if pdu.ttl == 0 {
                    if port.left_ns.is_none_or(|t| w.at_ns >= t) {
                        port.left_ns = Some(w.at_ns);
                    }
                } else if port.left_ns.is_some_and(|t| w.at_ns > t) {
                    port.left_ns = None;
                }
            }
            Observed::Arp { ip, .. } => d.arp_ip.observe(*ip, w),
        }
    }

    /// The conflicts on record, each once, ordered by kind and value.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut out = Vec::new();
        let mut by_name: BTreeMap<&str, Vec<Mac>> = BTreeMap::new();
        let mut by_ip: BTreeMap<Ipv4Addr, Vec<Mac>> = BTreeMap::new();
        for d in self.devices.values() {
            if let Some(n) = d.name.current().filter(|n| !n.is_empty()) {
                by_name.entry(n).or_default().push(d.mac);
            }
            for ip in d.addresses() {
                by_ip.entry(ip).or_default().push(d.mac);
            }
            if d.name.conflicted() {
                let names: Vec<&str> = d.name.candidates.iter().map(|c| c.value.as_str()).collect();
                out.push(Conflict { kind: ConflictKind::NameChanged, value: names.join(", "), macs: vec![d.mac] });
            }
            if d.ip.conflicted() {
                let ips: Vec<String> = d.ip.candidates.iter().map(|c| c.value.ip.to_string()).collect();
                out.push(Conflict { kind: ConflictKind::IpChanged, value: ips.join(", "), macs: vec![d.mac] });
            }
            if d.ip_status.current().is_some_and(|s| s.conflict) {
                let value = d.ip.current().map(|s| s.ip.to_string()).unwrap_or_default();
                out.push(Conflict { kind: ConflictKind::ReportsIpConflict, value, macs: vec![d.mac] });
            }
        }
        for (name, macs) in by_name.into_iter().filter(|(_, m)| m.len() > 1) {
            out.push(Conflict { kind: ConflictKind::DuplicateName, value: name.to_string(), macs });
        }
        for (ip, macs) in by_ip.into_iter().filter(|(_, m)| m.len() > 1) {
            out.push(Conflict { kind: ConflictKind::DuplicateIp, value: ip.to_string(), macs });
        }
        out.sort();
        out
    }
}
