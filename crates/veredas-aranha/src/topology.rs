//! The topology (`docs/aranha-plan.md`, "Scope", the topology; "Rules": it never guesses identity;
//! "Traps": passive LLDP gives nodes, not edges): the devices fusion holds (C-141), this machine's
//! interfaces and the chassis a neighbour names that no frame has identified, as nodes; the links
//! between them with the confidence their evidence earns; MRP rings; connected components; and a
//! note for everything it declines to draw.
//!
//! - **Heard**: exactly one device's port was heard on one of this machine's interfaces (LLDP is
//!   link-local, so that is the far end of the interface's cable). An interface hearing several
//!   ports is a mirror port or a hub, and no cable is drawn from it; a port that said it was
//!   leaving is no one's cable.
//! - **Reported**: a device's own word on its port's neighbour (its port data, read by the implicit
//!   read), the neighbour found by its MAC (a device's, or the one device whose port it is), else
//!   by the chassis id it announces in its own LLDP when exactly one device does. A name is never
//!   a key: a station name heard by DCP joins nothing. A chassis that two devices announce, or
//!   none, is a chassis node of its own, said ambiguous or not yet identified.
//! - **Confirmed**: both ends report each other.
//!
//! A ring is an MRP domain and the devices with a port in it (whatever MRRT's port status says:
//! the domain is the membership); it is closed when the links among its members make one cycle.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::fusion::Fusion;
use crate::profinet::mac::Mac;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Device,
    /// One of this machine's interfaces.
    Interface,
    /// A chassis a neighbour names that no frame has identified, or that two devices announce.
    Chassis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// `dev:<mac>`, `if:<name>`, `chassis:<key>`.
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub mac: Option<Mac>,
    /// Its ports by short name.
    pub ports: Vec<String>,
    /// The MRP domains (hex) a port of it is in.
    pub rings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    Heard,
    Reported,
    Confirmed,
}

/// A link's end: a node and its port (empty for one of this machine's interfaces).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct End {
    pub node: String,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The lesser end first, so one cable is one link whichever end spoke.
    pub a: End,
    pub b: End,
    pub confidence: Confidence,
    /// Frames heard (a heard link), or reports (a reported or confirmed one).
    pub witnesses: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    pub domain: String,
    pub members: Vec<String>,
    pub closed: bool,
}

/// One of this machine's interfaces, as the listener names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    pub mac: Option<Mac>,
}

/// A device's own word on its port's neighbour: its port data's peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbour {
    /// The device that said it, and its port (short name).
    pub device: Mac,
    pub port: String,
    /// The peer's chassis id and port id, as its port data gives them.
    pub chassis: String,
    pub peer_port: String,
    /// The peer's port MAC, when its port data gives one (all zeros is none).
    pub peer_mac: Option<Mac>,
    /// How many reads said it.
    pub witnesses: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    /// Devices by MAC, then interfaces by name, then chassis by key.
    pub nodes: Vec<Node>,
    /// By their ends.
    pub links: Vec<Link>,
    /// By domain.
    pub rings: Vec<Ring>,
    /// Node ids, largest component first, each sorted.
    pub components: Vec<Vec<String>>,
    pub notes: Vec<String>,
}

pub fn device_id(mac: Mac) -> String {
    format!("dev:{mac}")
}

pub fn interface_id(name: &str) -> String {
    format!("if:{name}")
}

pub fn chassis_id(key: &str) -> String {
    format!("chassis:{key}")
}

/// A port id as a port's short name: `port-001.station` is `port-001`.
fn short(port: &str) -> String {
    match port.split_once('.') {
        Some((p, _)) if p.starts_with("port-") => p.to_string(),
        _ => port.to_string(),
    }
}

/// Build the topology of what fusion holds, this machine's interfaces and the neighbours devices
/// reported. The same inputs give the same topology, whatever order the neighbours come in.
pub fn build(fusion: &Fusion, interfaces: &[Interface], neighbours: &[Neighbour]) -> Topology {
    let mut t = Topology::default();
    let mut domains: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // Interface name -> the (device, port) pairs heard on it, with their frames.
    let mut heard: BTreeMap<String, BTreeMap<(String, String), u64>> = BTreeMap::new();
    for d in fusion.devices.values() {
        let id = device_id(d.mac);
        let mut rings = BTreeSet::new();
        for p in d.ports.values() {
            if let Some(m) = p.mrp.current() {
                rings.insert(m.domain_hex());
                domains.entry(m.domain_hex()).or_default().insert(id.clone());
            }
            if p.left_ns.is_some() {
                t.notes.push(format!("{}'s {} said it was leaving: no cable is drawn to it", d.label(), p.id));
                continue;
            }
            for iface in &p.heard_on {
                heard.entry(iface.clone()).or_default().insert((id.clone(), p.id.clone()), p.frames);
            }
        }
        t.nodes.push(Node { id, kind: NodeKind::Device, label: d.label(), mac: Some(d.mac), ports: d.ports.keys().cloned().collect(), rings: rings.into_iter().collect() });
    }
    let mut ifaces: BTreeMap<String, Option<Mac>> = interfaces.iter().map(|i| (i.name.clone(), i.mac)).collect();
    for d in fusion.devices.values() {
        for iface in d.heard_on.iter().chain(d.ports.values().flat_map(|p| p.heard_on.iter())) {
            ifaces.entry(iface.clone()).or_insert(None);
        }
    }
    for (name, mac) in &ifaces {
        t.nodes.push(Node { id: interface_id(name), kind: NodeKind::Interface, label: name.clone(), mac: *mac, ports: Vec::new(), rings: Vec::new() });
    }

    // Heard: one port on an interface is its cable; several, a mirror or a hub. A port heard on
    // several interfaces is no one's cable (LLDP stops at the far end of one): a hub or a mirror
    // joins them.
    let mut ifaces_of: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (iface, senders) in &heard {
        for key in senders.keys() {
            ifaces_of.entry(key.clone()).or_default().insert(iface.clone());
        }
    }
    for ((node, port), on) in &ifaces_of {
        if on.len() > 1 {
            let on: Vec<&str> = on.iter().map(String::as_str).collect();
            t.notes.push(format!("{node} {port} is heard on {} interfaces ({}): a hub or a mirror joins them, no cable is drawn to it", on.len(), on.join(", ")));
        }
    }
    for (iface, senders) in &heard {
        if senders.len() == 1 {
            let ((node, port), frames) = senders.iter().next().map(|(k, f)| (k.clone(), *f)).unwrap_or_default();
            if ifaces_of.get(&(node.clone(), port.clone())).is_some_and(|on| on.len() == 1) {
                push(&mut t.links, End { node: interface_id(iface), port: String::new() }, End { node, port }, Confidence::Heard, frames);
            }
        } else {
            let list: Vec<String> = senders.keys().map(|(n, p)| format!("{n} {p}")).collect();
            t.notes.push(format!("{iface} hears LLDP from {} ports ({}): a mirror port or a hub, no cable is drawn from it", senders.len(), list.join(", ")));
        }
    }

    // Reported and confirmed, from the devices' own words on their neighbours.
    let mut claims: BTreeMap<String, BTreeSet<Mac>> = BTreeMap::new();
    for d in fusion.devices.values() {
        for c in &d.chassis.candidates {
            claims.entry(c.value.clone()).or_default().insert(d.mac);
        }
    }
    let mut chassis_nodes: BTreeMap<String, String> = BTreeMap::new();
    let mut said: BTreeMap<(End, End), (u64, u64)> = BTreeMap::new();
    let mut ordered: Vec<&Neighbour> = neighbours.iter().collect();
    ordered.sort_by(|x, y| (x.device, &x.port, &x.chassis, &x.peer_port, x.peer_mac).cmp(&(y.device, &y.port, &y.chassis, &y.peer_port, y.peer_mac)));
    for n in ordered {
        // A word said by no read is none; one from a device nothing heard has no end to draw from.
        if n.witnesses == 0 {
            continue;
        }
        if !fusion.devices.contains_key(&n.device) {
            t.notes.push(format!("{} {} reports a neighbour, but nothing heard that device: not drawn", device_id(n.device), short(&n.port)));
            continue;
        }
        let local = End { node: device_id(n.device), port: short(&n.port) };
        let by_mac = n.peer_mac.filter(|m| m.0 != [0; 6]).and_then(|m| if fusion.devices.contains_key(&m) { Some(m) } else { fusion.port_owner(m) });
        let key = format!("local:{}", n.chassis);
        let by_chassis = claims.get(&key).filter(|c| c.len() == 1).and_then(|c| c.iter().next().copied());
        let remote = match (by_mac, by_chassis) {
            (Some(m), _) => {
                if claims.get(&key).is_some_and(|c| c.iter().any(|d| *d != m)) {
                    t.notes.push(format!("{} {} names chassis {:?}, which another device announces, from the port MAC of {}: the MAC is taken", device_id(n.device), local.port, n.chassis, device_id(m)));
                }
                device_id(m)
            }
            (None, Some(c)) => device_id(c),
            (None, None) => {
                let claimants = claims.get(&key).map_or(0, BTreeSet::len);
                chassis_nodes.entry(key.clone()).or_insert_with(|| {
                    if claimants > 1 {
                        format!("{} (ambiguous: {claimants} devices)", n.chassis)
                    } else {
                        format!("{} (not yet identified)", n.chassis)
                    }
                });
                chassis_id(&key)
            }
        };
        let remote = End { node: remote, port: short(&n.peer_port) };
        let from_a = local <= remote;
        let (a, b) = if from_a { (local, remote) } else { (remote, local) };
        let e = said.entry((a, b)).or_default();
        if from_a {
            e.0 += n.witnesses;
        } else {
            e.1 += n.witnesses;
        }
    }
    for (key, label) in &chassis_nodes {
        let claimants = claims.get(key).map_or(0, BTreeSet::len);
        if claimants > 1 {
            t.notes.push(format!("a neighbour names chassis {:?}, which {claimants} devices announce: its links go to that chassis, not to a guess", &key["local:".len()..]));
        }
        t.nodes.push(Node { id: chassis_id(key), kind: NodeKind::Chassis, label: label.clone(), mac: None, ports: Vec::new(), rings: Vec::new() });
    }
    for ((a, b), (wa, wb)) in said {
        let confidence = if wa > 0 && wb > 0 { Confidence::Confirmed } else { Confidence::Reported };
        push(&mut t.links, a, b, confidence, wa + wb);
    }
    t.links.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));

    // Components: the nodes the links join.
    let ids: Vec<String> = t.nodes.iter().map(|n| n.id.clone()).collect();
    let index: BTreeMap<&str, usize> = ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    for l in &t.links {
        if let (Some(&a), Some(&b)) = (index.get(l.a.node.as_str()), index.get(l.b.node.as_str())) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut comps: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        let r = root(&mut parent, i);
        comps.entry(r).or_default().push(id.clone());
    }
    let mut comps: Vec<Vec<String>> = comps.into_values().map(|mut c| {
        c.sort();
        c
    }).collect();
    comps.sort_by(|x, y| y.len().cmp(&x.len()).then_with(|| x.cmp(y)));
    t.components = comps;

    // Rings: closed when the links among the members make one cycle through them all.
    for (domain, members) in domains {
        let among: Vec<&Link> = t.links.iter().filter(|l| members.contains(&l.a.node) && members.contains(&l.b.node) && l.a.node != l.b.node).collect();
        let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
        for l in &among {
            *degree.entry(l.a.node.as_str()).or_default() += 1;
            *degree.entry(l.b.node.as_str()).or_default() += 1;
        }
        let one_cycle = members.len() >= 3 && among.len() == members.len() && members.iter().all(|m| degree.get(m.as_str()) == Some(&2)) && connected(&members, &among);
        t.rings.push(Ring { domain, members: members.into_iter().collect(), closed: one_cycle });
    }
    t
}

fn push(links: &mut Vec<Link>, a: End, b: End, confidence: Confidence, witnesses: u64) {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    links.push(Link { a, b, confidence, witnesses });
}

fn root(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut c = i;
    while parent[c] != r {
        let next = parent[c];
        parent[c] = r;
        c = next;
    }
    r
}

/// Whether the links join every member into one piece (two triangles are not one ring).
fn connected(members: &BTreeSet<String>, links: &[&Link]) -> bool {
    let Some(first) = members.iter().next() else { return false };
    let mut seen: BTreeSet<&str> = BTreeSet::from([first.as_str()]);
    let mut grew = true;
    while grew {
        grew = false;
        for l in links {
            let (a, b) = (l.a.node.as_str(), l.b.node.as_str());
            if seen.contains(a) && seen.insert(b) || seen.contains(b) && seen.insert(a) {
                grew = true;
            }
        }
    }
    seen.len() == members.len()
}
