//! The topology (`docs/aranha-plan.md`, "Scope", the topology; "Rules": it never guesses identity;
//! "Traps": passive LLDP gives nodes, not edges): the devices fusion holds (C-141), this machine's
//! interfaces and the chassis a neighbour names that no frame has identified, as nodes; the links
//! between them with the confidence their evidence earns; MRP rings; connected components; and a
//! note for everything it declines to draw.
//!
//! - **Heard**: exactly one device's port was heard on one of this machine's interfaces since it
//!   last said it was leaving, and on no other since (LLDP is link-local, so that is the far end of
//!   the interface's cable). An interface hearing several such ports is a mirror port or a hub, and
//!   no cable is drawn from it; a port that said it was leaving is no one's cable, and what it was
//!   heard on before is no longer its own.
//! - **Reported**: a device's own word on its port's neighbour (its port data, read by the implicit
//!   read), the neighbour found by its MAC (a device's, or the one device whose port it is), else
//!   by the chassis id it announces in its own LLDP, its text whatever the subtype, as a port data
//!   writes it (its peer chassis is a bare text: a MAC as its address, a network address as its
//!   text, a name as itself), when exactly one device does. A name is never a key: a station
//!   name heard by DCP joins nothing, and an empty chassis id is none. A chassis that two devices
//!   announce, or none, is a chassis node of its own, said ambiguous or not yet identified.
//! - **Confirmed**: both ends report each other.
//!
//! A ring is an MRP domain and the devices with a port in it (whatever MRRT's port status says:
//! the domain is the membership); it is closed when the links among its members make one cycle
//! and none of them is gone (every member with a port in it that has not said it is leaving).

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
    /// `dev:<mac>`, `if:<name>`, `chassis:<its text>`.
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
    /// Frames heard on its interface (a heard link), or reports (a reported or confirmed one).
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
    /// Devices by MAC, then interfaces by name, then chassis by text.
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

pub fn chassis_id(text: &str) -> String {
    format!("chassis:{text}")
}

/// A port id as a port's short name: `port-001.station` is `port-001`.
pub(crate) fn short(port: &str) -> String {
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
    // The members with a port in the domain that has not said it is leaving: a member gone leaves
    // its ring open, whatever links are still reported.
    let mut present: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // Interface name -> the (device, port) pairs heard on it, with their frames.
    let mut heard: BTreeMap<String, BTreeMap<(String, String), u64>> = BTreeMap::new();
    for d in fusion.devices.values() {
        let id = device_id(d.mac);
        let mut rings = BTreeSet::new();
        for p in d.ports.values() {
            // The domain its port states last, leaving or not: membership is the ring's
            // configuration, and a member gone leaves the ring open.
            if let Some(m) = p.mrp.current() {
                rings.insert(m.domain_hex());
                domains.entry(m.domain_hex()).or_default().insert(id.clone());
                if p.left_ns.is_none() {
                    present.entry(m.domain_hex()).or_default().insert(id.clone());
                }
            }
            if p.left_ns.is_some() {
                note(&mut t.notes, format!("{id} {} said it was leaving: no cable is drawn to it", p.id));
                continue;
            }
            // Only where it was heard since it last said it was leaving: a port that left one
            // interface and came back on another has moved, not joined the two.
            for iface in p.heard_since_leaving() {
                let frames = p.on.get(&iface).map_or(0, |on| on.frames);
                heard.entry(iface).or_default().insert((id.clone(), p.id.clone()), frames);
            }
        }
        t.nodes.push(Node { id, kind: NodeKind::Device, label: d.label(), mac: Some(d.mac), ports: d.ports.keys().cloned().collect(), rings: rings.into_iter().collect() });
    }
    // An interface named twice keeps the first MAC it is given.
    let mut ifaces: BTreeMap<String, Option<Mac>> = BTreeMap::new();
    for i in interfaces {
        let mac = ifaces.entry(i.name.clone()).or_insert(i.mac);
        if mac.is_none() {
            *mac = i.mac;
        }
    }
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
            note(&mut t.notes, format!("{node} {port} is heard on {} interfaces ({}): a hub or a mirror joins them, no cable is drawn to it", on.len(), on.join(", ")));
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
            note(&mut t.notes, format!("{iface} hears LLDP from {} ports ({}): a mirror port or a hub, no cable is drawn from it", senders.len(), list.join(", ")));
        }
    }

    // Reported and confirmed, from the devices' own words on their neighbours.
    // A chassis by its text, whatever subtype its LLDP gives it, as a port data writes it (a port
    // data's peer chassis is a bare text), and a chassis announced as a MAC by that MAC too, so a
    // word that is a MAC in another spelling finds it. An empty one is no chassis.
    let mut by_text: BTreeMap<String, BTreeSet<Mac>> = BTreeMap::new();
    let mut by_mac: BTreeMap<Mac, BTreeSet<Mac>> = BTreeMap::new();
    for d in fusion.devices.values() {
        for c in &d.chassis.candidates {
            let text = chassis_text(&c.value);
            if !text.is_empty() {
                by_text.entry(text).or_default().insert(d.mac);
            }
            if let Some(m) = c.value.strip_prefix("mac:").and_then(|m| m.parse::<Mac>().ok()) {
                by_mac.entry(m).or_default().insert(d.mac);
            }
        }
    }
    // The devices a word names: those announcing it as itself, and, when it is a MAC in any of its
    // spellings, those announcing that MAC; two of them resolve to neither.
    let claimants = |word: &str| -> BTreeSet<Mac> {
        let mut out = by_text.get(word).cloned().unwrap_or_default();
        if let Ok(m) = word.parse::<Mac>() {
            out.extend(by_mac.get(&m).into_iter().flatten().copied());
        }
        out
    };
    // A chassis node by the word, a MAC by its address (one chassis, however it is spelled).
    let node_text = |word: &str| word.parse::<Mac>().map_or_else(|_| word.to_string(), |m| m.to_string());
    let mut chassis_nodes: BTreeMap<String, BTreeSet<Mac>> = BTreeMap::new();
    let mut said: BTreeMap<(End, End), (u64, u64)> = BTreeMap::new();
    let mut ordered: Vec<&Neighbour> = neighbours.iter().collect();
    ordered.sort_by(|x, y| (x.device, &x.port, &x.chassis, &x.peer_port, x.peer_mac).cmp(&(y.device, &y.port, &y.chassis, &y.peer_port, y.peer_mac)));
    for n in ordered {
        // A word said by no read is none; one from a device nothing heard has no end to draw from.
        if n.witnesses == 0 {
            continue;
        }
        if !fusion.devices.contains_key(&n.device) {
            note(&mut t.notes, format!("{} {} reports a neighbour, but nothing heard that device: not drawn", device_id(n.device), short(&n.port)));
            continue;
        }
        let local = End { node: device_id(n.device), port: short(&n.port) };
        let by_peer_mac = n.peer_mac.filter(|m| m.0 != [0; 6]).and_then(|m| if fusion.devices.contains_key(&m) { Some(m) } else { fusion.port_owner(m) });
        let text = n.chassis.as_str();
        let claimed = if text.is_empty() { BTreeSet::new() } else { claimants(text) };
        let by_chassis = (claimed.len() == 1).then(|| claimed.iter().next().copied()).flatten();
        let remote = match (by_peer_mac, by_chassis) {
            (Some(m), _) => {
                let others: Vec<String> = claimed.iter().filter(|d| **d != m).map(|d| device_id(*d)).collect();
                if !others.is_empty() {
                    let verb = if others.len() == 1 { "announces" } else { "announce" };
                    note(&mut t.notes, format!("{} {} names chassis {text:?}, which {} {verb}, from the port MAC of {}: the MAC is taken", device_id(n.device), local.port, others.join(", "), device_id(m)));
                }
                device_id(m)
            }
            (None, Some(c)) => device_id(c),
            (None, None) if text.is_empty() => {
                let mac = match n.peer_mac.filter(|m| m.0 != [0; 6]) {
                    Some(m) => format!("a MAC that is no single device's ({m})"),
                    None => "no MAC".to_string(),
                };
                note(&mut t.notes, format!("{} {} reports a neighbour with no chassis and {mac}: not drawn", device_id(n.device), local.port));
                continue;
            }
            (None, None) => {
                let key = node_text(text);
                chassis_nodes.entry(key.clone()).or_default().extend(claimed);
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
    for (text, announcing) in &chassis_nodes {
        let announcers: Vec<String> = announcing.iter().map(|m| device_id(*m)).collect();
        let label = if announcers.len() > 1 {
            note(&mut t.notes, format!("a neighbour names chassis {text:?}, which {} devices announce ({}): its links go to that chassis, not to a guess", announcers.len(), announcers.join(", ")));
            format!("{text} (ambiguous: {} devices)", announcers.len())
        } else {
            format!("{text} (not yet identified)")
        };
        t.nodes.push(Node { id: chassis_id(text), kind: NodeKind::Chassis, label, mac: None, ports: Vec::new(), rings: Vec::new() });
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
        let all_there = present.get(&domain) == Some(&members);
        t.rings.push(Ring { domain, members: members.into_iter().collect(), closed: one_cycle && all_there });
    }
    t
}

/// A note, said once.
fn note(notes: &mut Vec<String>, n: String) {
    if !notes.contains(&n) {
        notes.push(n);
    }
}

/// The most a port data's text holds, in visible characters (the bench's device cuts there too).
const MAX_ID: usize = 255;

/// Bytes as a port data writes a name: visible ASCII as itself, a backslash and every other byte
/// as `\xHH` (so an escape reads one way), cut at a whole character or escape to `MAX_ID`.
fn rendered(data: &[u8]) -> String {
    let mut out = String::new();
    for &b in data {
        let piece = if (0x20..=0x7E).contains(&b) && b != b'\\' { (b as char).to_string() } else { format!("\\x{b:02x}") };
        if out.len() + piece.len() > MAX_ID {
            break;
        }
        out.push_str(&piece);
    }
    out
}

/// Bytes as hex, cut at a whole byte to `MAX_ID`.
fn hex_of(data: &[u8]) -> String {
    data.iter().take(MAX_ID / 2).map(|b| format!("{b:02x}")).collect()
}

/// A chassis key's text, as a port data writes the chassis it names (its peer chassis is a bare
/// text, as the bench's device writes it): a name (`local:`, `ifname:`, and the text subtypes 1, 2,
/// 3, 6 and 7) as itself, rendered; a MAC as its address; an IPv4 or IPv6 network address as its
/// text, another family as hex; another subtype's bytes as their text when they are visible ASCII,
/// else as hex.
fn chassis_text(key: &str) -> String {
    let Some((kind, rest)) = key.split_once(':') else { return key.to_string() };
    let bytes = || -> Option<Vec<u8>> {
        if rest.len() % 2 != 0 {
            return None;
        }
        (0..rest.len()).step_by(2).map(|i| u8::from_str_radix(rest.get(i..i + 2)?, 16).ok()).collect()
    };
    match kind {
        "local" | "ifname" => rendered(rest.as_bytes()),
        "mac" => rest.to_string(),
        "net" => match bytes().as_deref() {
            Some([1, a, b, c, d]) => std::net::Ipv4Addr::new(*a, *b, *c, *d).to_string(),
            Some([2, r @ ..]) if r.len() == 16 => std::net::Ipv6Addr::from(<[u8; 16]>::try_from(r).unwrap_or([0; 16])).to_string(),
            Some(b) => hex_of(b),
            None => rest.to_string(),
        },
        "other1" | "other2" | "other3" | "other6" | "other7" => bytes().map_or_else(|| rest.to_string(), |b| rendered(&b)),
        other if other.starts_with("other") => match bytes() {
            Some(b) if !b.is_empty() && b.iter().all(|c| (0x20..=0x7E).contains(c)) => rendered(&b),
            Some(b) => hex_of(&b),
            None => rest.to_string(),
        },
        _ => key.to_string(),
    }
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
