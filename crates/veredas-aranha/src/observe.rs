//! A frame into observations (`docs/aranha-plan.md`, "Scope", listening): one thing one frame says
//! about one device, with its witness (what was heard, on which interface, when, which kept frame);
//! the frame's class, for the count of what the wire carries; and the damage it holds, named. Frames
//! are kept verbatim by the listener and decoded here on demand, so a decoder fixed later reads the
//! same capture again.
//!
//! What is observed, passively: a DCP answer or Hello (the device describing itself), a DCP request
//! (a station asking: a controller, an engineering tool, a scanner), an LLDP frame (a port
//! describing itself and its chassis), an ARP sender (a MAC claiming an address). Everything else
//! is classed and counted, never observed: cyclic and alarm frames, the other EtherTypes. Decoding
//! is total: any bytes give a class, observations or none, and damage named, without a panic.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::profinet::dcp::{self, BlockBody, DeviceIdentity, Service, ServiceType};
use crate::profinet::ether::{self, ETHERTYPE_ARP, ETHERTYPE_ETHERCAT, ETHERTYPE_IPV4, ETHERTYPE_IPV6, ETHERTYPE_LLDP, ETHERTYPE_MRP, ETHERTYPE_POWERLINK, ETHERTYPE_PROFINET, ETHERTYPE_PTP};
use crate::profinet::lldp::{self, ChassisId, LldpPdu};
use crate::profinet::mac::Mac;
use crate::profinet::rt::FrameClass;
use crate::wire::Frame;

/// What kind of hearing a witness is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Probe {
    /// A DCP answer or Hello: a device describing itself (to us or to another asker: the wire
    /// does not say).
    DcpHeard,
    /// A DCP request: a station asking (Identify, Get, Set).
    DcpRequestHeard,
    /// An LLDP frame: a port describing itself.
    LldpHeard,
    /// An ARP request or reply: its sender claiming an address.
    ArpHeard,
}

/// Where an observation comes from: the hearing, the interface, the time (nanoseconds since the
/// Unix epoch) and the kept frame (its index in the listener's or the capture's numbering).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    pub probe: Probe,
    pub iface: String,
    pub at_ns: u64,
    pub frame: u64,
}

/// What one frame says about one device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Observed {
    /// A DCP PDU from `mac`: an answer or a Hello with the identity its blocks give, or a request
    /// (its identity then empty).
    Dcp { mac: Mac, service: Service, service_type: ServiceType, identity: DeviceIdentity },
    /// An LLDP PDU from `src`, the port's own MAC on a PROFINET device.
    Lldp { src: Mac, pdu: LldpPdu },
    /// An ARP sender: `mac` claims `ip`.
    Arp { mac: Mac, ip: Ipv4Addr },
}

impl Observed {
    /// The MAC the frame names for the device it is about: a DCP PDU's source; an LLDP PDU's
    /// chassis MAC (PROFINET's TLV), else its chassis id when that is a MAC, else its source (the
    /// port's MAC); an ARP sender's address.
    pub fn subject(&self) -> Mac {
        match self {
            Observed::Dcp { mac, .. } | Observed::Arp { mac, .. } => *mac,
            Observed::Lldp { src, pdu } => pdu.pn.chassis_mac.or(match &pdu.chassis {
                ChassisId::Mac { mac } => Some(*mac),
                _ => None,
            })
            .unwrap_or(*src),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub witness: Witness,
    pub observed: Observed,
}

/// What a frame is, for the count of what the wire carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// PROFINET's DCP (frame ids 0xFEFC to 0xFEFF).
    Dcp,
    /// PROFINET's cyclic data (RT class 1 and 3, the legacy range).
    Cyclic,
    /// PROFINET's alarms (0xFC01, 0xFE01).
    Alarm,
    /// Any other PROFINET frame id, or a PROFINET frame too short for one.
    ProfinetOther,
    Lldp,
    Arp,
    Ipv4,
    Ipv6,
    /// EtherCAT and POWERLINK: heard and counted, never sent to (the plan).
    Ethercat,
    Powerlink,
    Mrp,
    Ptp,
    /// Any other EtherType, or an 802.3 length (a value under 0x0600).
    Other(u16),
    /// Not an Ethernet frame (shorter than its header).
    Malformed,
}

impl Class {
    pub fn of_ethertype(ethertype: u16) -> Class {
        match ethertype {
            ETHERTYPE_LLDP => Class::Lldp,
            ETHERTYPE_ARP => Class::Arp,
            ETHERTYPE_IPV4 => Class::Ipv4,
            ETHERTYPE_IPV6 => Class::Ipv6,
            ETHERTYPE_ETHERCAT => Class::Ethercat,
            ETHERTYPE_POWERLINK => Class::Powerlink,
            ETHERTYPE_MRP => Class::Mrp,
            ETHERTYPE_PTP => Class::Ptp,
            ETHERTYPE_PROFINET => Class::ProfinetOther,
            other => Class::Other(other),
        }
    }
}

impl std::fmt::Display for Class {
    /// Its name in a count: `dcp`, `cyclic`, `alarm`, `profinet-other`, `lldp`, `arp`, `ipv4`,
    /// `ipv6`, `ethercat`, `powerlink`, `mrp`, `ptp`, `other:0x1234`, `malformed`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Class::Dcp => "dcp",
            Class::Cyclic => "cyclic",
            Class::Alarm => "alarm",
            Class::ProfinetOther => "profinet-other",
            Class::Lldp => "lldp",
            Class::Arp => "arp",
            Class::Ipv4 => "ipv4",
            Class::Ipv6 => "ipv6",
            Class::Ethercat => "ethercat",
            Class::Powerlink => "powerlink",
            Class::Mrp => "mrp",
            Class::Ptp => "ptp",
            Class::Other(t) => return write!(f, "other:{t:#06x}"),
            Class::Malformed => "malformed",
        };
        f.write_str(name)
    }
}

impl Serialize for Class {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Class {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Class, D::Error> {
        let text = String::deserialize(d)?;
        let named = [
            Class::Dcp,
            Class::Cyclic,
            Class::Alarm,
            Class::ProfinetOther,
            Class::Lldp,
            Class::Arp,
            Class::Ipv4,
            Class::Ipv6,
            Class::Ethercat,
            Class::Powerlink,
            Class::Mrp,
            Class::Ptp,
            Class::Malformed,
        ];
        if let Some(c) = named.iter().find(|c| c.to_string() == text) {
            return Ok(*c);
        }
        // Another type's name, exactly as written (four lower-case digits), and only a type with no
        // class of its own.
        text.strip_prefix("other:0x")
            .filter(|h| h.len() == 4 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .and_then(|h| u16::from_str_radix(h, 16).ok())
            .filter(|t| Class::of_ethertype(*t) == Class::Other(*t))
            .map(Class::Other)
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is no frame class")))
    }
}

/// Everything one frame gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub class: Class,
    pub observations: Vec<Observation>,
    /// The damage it holds, each named (a DCP block that does not fit its kind, an LLDP PDU that
    /// is no PDU, an ARP that is not Ethernet's and IPv4's): never fatal, never dropped unseen.
    pub damage: Vec<String>,
}

/// Decode one kept frame, `index` its number among the kept frames.
pub fn decode(frame: &Frame, index: u64) -> Decoded {
    let mut out = Decoded { class: Class::Malformed, observations: Vec::new(), damage: Vec::new() };
    let e = match ether::parse(&frame.bytes) {
        Ok(e) => e,
        Err(err) => {
            out.damage.push(err.to_string());
            return out;
        }
    };
    out.class = Class::of_ethertype(e.ethertype);
    // A frame a capture cut is classed and named, and says nothing: what it would say may be what
    // the cut left out (an LLDP PDU's chassis MAC TLV, say).
    if frame.is_cut() {
        out.damage.push(format!("the capture kept {} of the frame's {} bytes", frame.bytes.len(), frame.length));
        if e.ethertype == ETHERTYPE_PROFINET {
            out.class = e.payload.get(..2).map_or(Class::ProfinetOther, |b| profinet_class(u16::from_be_bytes([b[0], b[1]])));
        }
        return out;
    }
    let witness = |probe| Witness { probe, iface: frame.iface.clone(), at_ns: frame.at_ns, frame: index };
    match e.ethertype {
        ETHERTYPE_PROFINET => {
            let Some(id) = e.payload.get(..2).map(|b| u16::from_be_bytes([b[0], b[1]])) else {
                out.damage.push("a PROFINET frame too short for its frame id".to_string());
                return out;
            };
            out.class = profinet_class(id);
            if out.class != Class::Dcp {
                return out;
            }
            let pdu = match dcp::decode(e.payload) {
                Ok(pdu) => pdu,
                Err(err) => {
                    out.damage.push(format!("DCP: {err}"));
                    return out;
                }
            };
            // A service or a service type DCP does not define says nothing (tshark leaves it
            // undecoded).
            if matches!(pdu.service, Service::Other(_)) {
                out.damage.push(format!("DCP service {} is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)", pdu.service.to_u8()));
                return out;
            }
            if !matches!(pdu.service_type.to_u8(), 0 | 1 | 5) {
                out.damage.push(format!("DCP service type {} is none of DCP's (0 request, 1 success, 5 unsupported)", pdu.service_type.to_u8()));
                return out;
            }
            for b in &pdu.blocks {
                if let BlockBody::Damaged { damage, .. } = &b.body {
                    out.damage.push(format!("DCP block {}/{}: {damage:?}", b.option, b.suboption));
                }
            }
            if !pdu.trailing.is_empty() {
                out.damage.push(format!("DCP: {} bytes after its last block make none", pdu.trailing.len()));
            }
            if e.src.is_multicast() {
                out.damage.push(format!("DCP from a group address, {}", e.src));
                return out;
            }
            // A request by its selection bit (as the codec reads one), a Hello aside: the station
            // describing itself.
            let asking = !pdu.service_type.is_response() && pdu.service != Service::Hello;
            let identity = if asking { DeviceIdentity::default() } else { DeviceIdentity::from_blocks(&pdu.blocks) };
            let probe = if asking { Probe::DcpRequestHeard } else { Probe::DcpHeard };
            out.observations.push(Observation {
                witness: witness(probe),
                observed: Observed::Dcp { mac: e.src, service: pdu.service, service_type: pdu.service_type, identity },
            });
        }
        ETHERTYPE_LLDP => {
            let pdu = match lldp::decode(e.payload) {
                Ok(pdu) => pdu,
                Err(err) => {
                    out.damage.push(format!("LLDP: {err}"));
                    return out;
                }
            };
            if e.src.is_multicast() {
                out.damage.push(format!("LLDP from a group address, {}", e.src));
                return out;
            }
            let observed = Observed::Lldp { src: e.src, pdu };
            // A group address is never one device: a chassis named by one says nothing.
            if observed.subject().is_multicast() {
                out.damage.push(format!("LLDP names a group address for its chassis, {}", observed.subject()));
                return out;
            }
            out.observations.push(Observation { witness: witness(Probe::LldpHeard), observed });
        }
        ETHERTYPE_ARP => match arp_sender(e.payload) {
            Ok(Some((mac, ip))) => out.observations.push(Observation { witness: witness(Probe::ArpHeard), observed: Observed::Arp { mac, ip } }),
            Ok(None) => {}
            Err(why) => out.damage.push(why),
        },
        _ => {}
    }
    out
}

/// A PROFINET frame's class, by its frame id's range.
fn profinet_class(id: u16) -> Class {
    match FrameClass::of(id) {
        FrameClass::Rtc3 | FrameClass::Rtc1 | FrameClass::Rtc1Legacy => Class::Cyclic,
        FrameClass::AlarmHigh | FrameClass::AlarmLow => Class::Alarm,
        FrameClass::Dcp => Class::Dcp,
        FrameClass::Other => Class::ProfinetOther,
    }
}

/// An ARP packet's sender, when it claims an address: hardware type Ethernet (1), protocol IPv4
/// (0x0800), addresses of 6 and 4 bytes, a request (1) or a reply (2). A sender of 0.0.0.0 (a probe,
/// RFC 5227) claims none; a group address as sender is damage.
fn arp_sender(p: &[u8]) -> Result<Option<(Mac, Ipv4Addr)>, String> {
    if p.len() < 28 {
        return Err(format!("an ARP packet of Ethernet and IPv4 is 28 bytes; this one is {}", p.len()));
    }
    let (htype, ptype, oper) = (u16::from_be_bytes([p[0], p[1]]), u16::from_be_bytes([p[2], p[3]]), u16::from_be_bytes([p[6], p[7]]));
    if (htype, ptype, p[4], p[5]) != (1, ETHERTYPE_IPV4, 6, 4) {
        return Err(format!("an ARP packet of hardware type {htype}, protocol {ptype:#06x}, address lengths {} and {}: not Ethernet's and IPv4's", p[4], p[5]));
    }
    if !matches!(oper, 1 | 2) {
        return Err(format!("ARP operation {oper} is neither a request (1) nor a reply (2)"));
    }
    let mac = Mac([p[8], p[9], p[10], p[11], p[12], p[13]]);
    let ip = Ipv4Addr::new(p[14], p[15], p[16], p[17]);
    if mac.is_multicast() {
        return Err(format!("ARP from a group address, {mac}"));
    }
    Ok((!ip.is_unspecified()).then_some((mac, ip)))
}

/// An ARP request or reply frame, for fixtures and, later, the asking phase: `src` at `src_ip` to
/// `dst` (broadcast for a request) about `dst_ip`.
pub fn arp_frame(src: Mac, src_ip: Ipv4Addr, dst: Mac, dst_ip: Ipv4Addr, reply: bool) -> Result<Vec<u8>, ether::EtherError> {
    let mut p = vec![0, 1, 8, 0, 6, 4, 0, if reply { 2 } else { 1 }];
    p.extend_from_slice(&src.0);
    p.extend_from_slice(&src_ip.octets());
    p.extend_from_slice(&if reply { dst.0 } else { [0; 6] });
    p.extend_from_slice(&dst_ip.octets());
    ether::build(dst, src, ETHERTYPE_ARP, &p)
}
