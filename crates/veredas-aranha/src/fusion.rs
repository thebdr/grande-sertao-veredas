//! Observations into devices (`docs/aranha-plan.md`, "Scope", fusion; "Rules": it never guesses
//! identity). The MAC an observation names is the device; an address is a value of it and a name
//! is never a key. Every field keeps every value it was given, each with the first and last witness
//! and how often, so a disagreement is a conflict on record, never a silent overwrite; its current
//! value is the one heard last. An empty station name is no name, and 0.0.0.0 no address.
//!
//! What is fused is read by the frames' times, never by the order they were applied in: the same
//! observations in any order give the same devices (but for a MAC that is both a device of its own
//! and another device's port, and one two devices' ports share: [`Fusion::apply`]), so a replay of
//! a capture reads as the listening did, and a capture merged out of order reads as one heard in
//! order.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::observe::{Observation, Observed, Probe, Witness};
use crate::profinet::dcp::{IpStatus, IpSuite};
use crate::profinet::lldp::{ChassisId, Delays, LldpPdu, MrpPortStatus, PortStatus};
use crate::profinet::mac::Mac;

/// One value a field was given, and by whom: the first and the last witness, and how many.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate<T> {
    pub value: T,
    pub first: Witness,
    pub last: Witness,
    pub count: u64,
}

/// A field with its provenance: its candidates in the order first heard (by their first witness's
/// time, whatever order they were applied in).
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
        self.merge(Candidate { value, first: w.clone(), last: w.clone(), count: 1 });
    }

    /// Take in a candidate: its count added, its witnesses widened, the candidates kept in the
    /// order first heard.
    fn merge(&mut self, n: Candidate<T>) {
        match self.candidates.iter_mut().find(|c| c.value == n.value) {
            Some(c) => {
                c.count += n.count;
                if when(&n.last) >= when(&c.last) {
                    c.last = n.last;
                }
                if when(&n.first) < when(&c.first) {
                    c.first = n.first;
                }
            }
            None => self.candidates.push(n),
        }
        self.candidates.sort_by_key(|c| when(&c.first));
    }

    /// Take in every candidate of another field.
    fn absorb(&mut self, other: Field<T>) {
        for c in other.candidates {
            self.merge(c);
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
    /// When it said it was leaving (a time to live of 0), unless a later frame came: its last
    /// leaving frame against its last other frame, by stamp then index.
    pub left_ns: Option<u64>,
    /// The place (stamp, index) of its last frame saying it was leaving, and of its last other.
    pub leaving: Option<(u64, u64)>,
    pub spoke: Option<(u64, u64)>,
    pub first_ns: u64,
    pub last_ns: u64,
    pub frames: u64,
}

impl Port {
    /// A frame at `at`, leaving or not.
    fn said(&mut self, at: (u64, u64), leaving: bool) {
        let slot = if leaving { &mut self.leaving } else { &mut self.spoke };
        if slot.is_none_or(|t| at > t) {
            *slot = Some(at);
        }
        self.settle();
    }

    fn settle(&mut self) {
        self.left_ns = match (self.leaving, self.spoke) {
            (Some(l), spoke) if spoke.is_none_or(|s| l > s) => Some(l.0),
            _ => None,
        };
    }

    /// Take in what another record of this port holds.
    fn absorb(&mut self, o: Port) {
        self.port_mac.absorb(o.port_mac);
        self.desc.absorb(o.desc);
        self.pn_status.absorb(o.pn_status);
        self.delays.absorb(o.delays);
        self.mrp.absorb(o.mrp);
        self.pvid.absorb(o.pvid);
        self.mau_type.absorb(o.mau_type);
        self.heard_on.extend(o.heard_on);
        self.leaving = self.leaving.max(o.leaving);
        self.spoke = self.spoke.max(o.spoke);
        self.settle();
        self.first_ns = self.first_ns.min(o.first_ns);
        self.last_ns = self.last_ns.max(o.last_ns);
        self.frames += o.frames;
    }
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
    /// LLDP's management addresses (IPv4), as each PDU that gives one states them.
    pub mgmt_ips: Field<BTreeSet<Ipv4Addr>>,
    /// The addresses its ARP claimed: a second, independent witness of its address.
    pub arp_ip: Field<Ipv4Addr>,
    pub ports: BTreeMap<String, Port>,
    /// What it was heard doing: answering DCP, asking DCP, LLDP, ARP.
    pub heard: BTreeSet<Probe>,
    pub heard_on: BTreeSet<String>,
    pub first_ns: u64,
    pub last_ns: u64,
    pub frames: u64,
    /// Nothing is said of it but LLDP PDUs from its own MAC naming no MAC for their chassis: a
    /// port whose device is not known yet, taken into that device once a PDU from the same MAC
    /// names it.
    pub by_source: bool,
}

impl Device {
    fn new(mac: Mac, at_ns: u64, by_source: bool) -> Device {
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
            mgmt_ips: Field::default(),
            arp_ip: Field::default(),
            ports: BTreeMap::new(),
            heard: BTreeSet::new(),
            heard_on: BTreeSet::new(),
            first_ns: at_ns,
            last_ns: at_ns,
            frames: 0,
            by_source,
        }
    }

    /// What to call it: its station name, else its LLDP system name, else its MAC (an empty name
    /// is none).
    pub fn label(&self) -> String {
        let named = |f: &Field<String>| f.current().filter(|n| !n.is_empty()).cloned();
        named(&self.name).or_else(|| named(&self.sys_name)).unwrap_or_else(|| self.mac.to_string())
    }

    /// The addresses it holds now: its current DCP address, ARP address and management addresses,
    /// 0.0.0.0 left out.
    pub fn addresses(&self) -> BTreeSet<Ipv4Addr> {
        let mut s: BTreeSet<Ipv4Addr> = self.mgmt_ips.current().cloned().unwrap_or_default();
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

    /// Take in what another record of this device holds (a port of it known before it was).
    fn absorb(&mut self, o: Device) {
        self.name.absorb(o.name);
        self.alias.absorb(o.alias);
        self.ip.absorb(o.ip);
        self.ip_status.absorb(o.ip_status);
        self.pn_id.absorb(o.pn_id);
        self.vendor.absorb(o.vendor);
        self.role.absorb(o.role);
        self.instance.absorb(o.instance);
        self.chassis.absorb(o.chassis);
        self.sys_name.absorb(o.sys_name);
        self.sys_desc.absorb(o.sys_desc);
        self.sys_caps.absorb(o.sys_caps);
        self.mgmt_ips.absorb(o.mgmt_ips);
        self.arp_ip.absorb(o.arp_ip);
        for (id, p) in o.ports {
            match self.ports.get_mut(&id) {
                Some(mine) => mine.absorb(p),
                None => {
                    self.ports.insert(id, p);
                }
            }
        }
        self.heard.extend(o.heard);
        self.heard_on.extend(o.heard_on);
        self.first_ns = self.first_ns.min(o.first_ns);
        self.last_ns = self.last_ns.max(o.last_ns);
        self.frames += o.frames;
    }
}

/// An LLDP PDU names its chassis by a MAC: PROFINET's chassis MAC, or a chassis id of subtype MAC.
fn names_chassis_mac(pdu: &LldpPdu) -> bool {
    pdu.pn.chassis_mac.is_some() || matches!(pdu.chassis, ChassisId::Mac { .. })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    /// Two devices name themselves alike.
    DuplicateName,
    /// Two devices hold one address.
    DuplicateIp,
    /// One device has named itself two ways (an empty name none).
    NameChanged,
    /// One device has held two DCP addresses (0.0.0.0 none, mask and gateway aside).
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

    /// The device one of whose ports a MAC is (any of its LLDP frames came from it), when exactly
    /// one is. A MAC's own device is not its owner, so a port not yet known ([`Device::by_source`],
    /// kept at its own MAC, its frames' only source) is no device's.
    pub fn port_owner(&self, port_mac: Mac) -> Option<Mac> {
        let mut owners = self
            .devices
            .values()
            .filter(|d| d.mac != port_mac && d.ports.values().any(|p| p.port_mac.candidates.iter().any(|c| c.value == port_mac)))
            .map(|d| d.mac);
        let first = owners.next()?;
        owners.next().is_none().then_some(first)
    }

    /// Apply one observation to the device its subject names. An LLDP PDU that names no MAC for
    /// its chassis (a shutdown PDU holds the mandatory three alone) is the device's whose port its
    /// source is, when exactly one device's is, else its source's: kept there as a port not yet
    /// known ([`Device::by_source`]) until a PDU from that MAC names its device, which then takes
    /// it in. So the order applied changes nothing, but where a MAC is a device of its own (it
    /// speaks DCP or ARP, or a PDU names it as its chassis) before a PDU names it another device's
    /// port: what it said by LLDP before then stays its own.
    pub fn apply(&mut self, o: &Observation) {
        let w = &o.witness;
        let (mac, sourced) = match &o.observed {
            Observed::Lldp { src, pdu } if !names_chassis_mac(pdu) => {
                let mac = self.port_owner(*src).unwrap_or(*src);
                (mac, mac == *src)
            }
            other => (other.subject(), false),
        };
        let d = self.devices.entry(mac).or_insert_with(|| Device::new(mac, w.at_ns, sourced));
        if !sourced {
            d.by_source = false;
        }
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
                let mgmt: BTreeSet<Ipv4Addr> = pdu.mgmt_addrs.iter().filter_map(|m| m.ipv4()).collect();
                if !mgmt.is_empty() {
                    d.mgmt_ips.observe(mgmt, w);
                }
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
                port.said(when(w), pdu.ttl == 0);
            }
            Observed::Arp { ip, .. } => d.arp_ip.observe(*ip, w),
        }
        // A PDU naming its device from a MAC kept as a port not yet known: that device takes it in.
        if let Observed::Lldp { src, pdu } = &o.observed
            && names_chassis_mac(pdu)
            && mac != *src
            && self.devices.get(src).is_some_and(|d| d.by_source)
            && self.port_owner(*src) == Some(mac)
            && let Some(port) = self.devices.remove(src)
            && let Some(owner) = self.devices.get_mut(&mac)
        {
            owner.absorb(port);
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
            let names: Vec<&str> = d.name.candidates.iter().map(|c| c.value.as_str()).filter(|n| !n.is_empty()).collect();
            if names.len() > 1 {
                out.push(Conflict { kind: ConflictKind::NameChanged, value: names.join(", "), macs: vec![d.mac] });
            }
            let mut ips: Vec<Ipv4Addr> = Vec::new();
            for ip in d.ip.candidates.iter().map(|c| c.value.ip).filter(|ip| !ip.is_unspecified()) {
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
            if ips.len() > 1 {
                let ips: Vec<String> = ips.iter().map(Ipv4Addr::to_string).collect();
                out.push(Conflict { kind: ConflictKind::IpChanged, value: ips.join(", "), macs: vec![d.mac] });
            }
            if d.ip_status.current().is_some_and(|s| s.conflict) {
                let value = d.ip.current().map(|s| s.ip).filter(|ip| !ip.is_unspecified()).map(|ip| ip.to_string()).unwrap_or_default();
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
