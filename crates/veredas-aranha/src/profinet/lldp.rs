//! LLDP (IEEE 802.1AB) with the organisation TLVs a PROFINET segment carries: PROFINET's own
//! (OUI 00-0E-CF: measured delays, port status, MRP port status, chassis MAC), IEEE 802.1's port
//! VLAN id and IEEE 802.3's MAC/PHY status.
//!
//! ```text
//! EtherType 0x88CC, dst 01:80:C2:00:00:0E | TLV ... | End TLV
//! TLV: type (7 bits) | length (9 bits) | value
//! ```
//!
//! LLDP is link-local: no bridge forwards it, so a frame heard on an interface names the device
//! at the far end of that cable (or, on a mirror port, one of the mirrored port's neighbours).
//! This module only reads and writes the frames; what a frame means for the topology is the
//! scanner's.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use super::ether::{self, ETHERTYPE_LLDP, EtherError};
use super::mac::Mac;

pub const OUI_PROFINET: [u8; 3] = [0x00, 0x0E, 0xCF];
pub const OUI_IEEE_802_1: [u8; 3] = [0x00, 0x80, 0xC2];
pub const OUI_IEEE_802_3: [u8; 3] = [0x00, 0x12, 0x0F];

/// The longest value a TLV's 9-bit length can say.
pub const MAX_TLV_VALUE: usize = 511;
/// The longest chassis id, port id, port description, system name or description (802.1AB).
pub const MAX_TEXT: usize = 255;

/// Does an id (subtype, then data) have the form its subtype gives it: a MAC of six bytes, a
/// network address of its family's length (a family byte, then the address)?
fn id_fits(mac_subtype: u8, network_subtype: u8, v: &[u8]) -> bool {
    let data = &v[1..];
    if v[0] == mac_subtype {
        data.len() == 6
    } else if v[0] == network_subtype {
        data.len() >= 2 && family_len(data[0]).is_none_or(|n| n == data.len() - 1)
    } else {
        true
    }
}

/// The longest OID a management address carries (802.1AB: the TLV is at most 167 bytes).
pub const MAX_OID: usize = 128;

/// The address length a management address of this family has: IPv4, IPv6, an 802 MAC.
fn family_len(subtype: u8) -> Option<usize> {
    match subtype {
        1 => Some(4),
        2 => Some(16),
        6 => Some(6),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ChassisId {
    /// Subtype 4.
    Mac { mac: Mac },
    /// Subtype 7: a PROFINET device puts its NameOfStation here.
    Local { id: String },
    /// Subtype 6.
    IfName { name: String },
    /// Subtype 5: an address family byte and the address.
    Network { data: Vec<u8> },
    /// Subtypes 1 (chassis component), 2 (interface alias), 3 (port component) and any other.
    Other { subtype: u8, data: Vec<u8> },
}

impl ChassisId {
    /// A stable text key, for a graph node: what a neighbour table names a chassis by.
    pub fn key(&self) -> String {
        match self {
            ChassisId::Mac { mac } => format!("mac:{mac}"),
            ChassisId::Local { id } => format!("local:{id}"),
            ChassisId::IfName { name } => format!("ifname:{name}"),
            ChassisId::Network { data } => format!("net:{}", hex(data)),
            ChassisId::Other { subtype, data } => format!("other{subtype}:{}", hex(data)),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let (subtype, data) = match self {
            ChassisId::Mac { mac } => (4, mac.0.to_vec()),
            ChassisId::Network { data } => (5, data.clone()),
            ChassisId::IfName { name } => (6, name.as_bytes().to_vec()),
            ChassisId::Local { id } => (7, id.as_bytes().to_vec()),
            ChassisId::Other { subtype, data } => (*subtype, data.clone()),
        };
        [vec![subtype], data].concat()
    }

    fn decode(v: &[u8]) -> ChassisId {
        let data = &v[1..];
        match (v[0], text(data)) {
            (4, _) if data.len() == 6 => ChassisId::Mac { mac: Mac([data[0], data[1], data[2], data[3], data[4], data[5]]) },
            (5, _) => ChassisId::Network { data: data.to_vec() },
            (6, Some(name)) => ChassisId::IfName { name },
            (7, Some(id)) => ChassisId::Local { id },
            (s, _) => ChassisId::Other { subtype: s, data: data.to_vec() },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PortId {
    /// Subtype 3.
    Mac { mac: Mac },
    /// Subtype 5.
    IfName { name: String },
    /// Subtype 7: PROFINET writes `port-001`, or `port-001.<station name>`.
    Local { id: String },
    /// Subtype 1.
    IfAlias { name: String },
    /// Subtypes 2 (port component), 4 (network address), 6 (agent circuit id) and any other.
    Other { subtype: u8, data: Vec<u8> },
}

impl PortId {
    /// The port's short name: a PROFINET `port-001.station` loses its station suffix; anything
    /// else is shown as it is.
    pub fn short(&self) -> String {
        match self {
            PortId::Mac { mac } => mac.to_string(),
            PortId::IfName { name } | PortId::IfAlias { name } => name.clone(),
            PortId::Local { id } => match id.split_once('.') {
                Some((port, _)) if port.starts_with("port-") => port.to_string(),
                _ => id.clone(),
            },
            PortId::Other { subtype, data } => format!("other{subtype}:{}", hex(data)),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let (subtype, data) = match self {
            PortId::IfAlias { name } => (1, name.as_bytes().to_vec()),
            PortId::Mac { mac } => (3, mac.0.to_vec()),
            PortId::IfName { name } => (5, name.as_bytes().to_vec()),
            PortId::Local { id } => (7, id.as_bytes().to_vec()),
            PortId::Other { subtype, data } => (*subtype, data.clone()),
        };
        [vec![subtype], data].concat()
    }

    fn decode(v: &[u8]) -> PortId {
        let data = &v[1..];
        match (v[0], text(data)) {
            (1, Some(name)) => PortId::IfAlias { name },
            (3, _) if data.len() == 6 => PortId::Mac { mac: Mac([data[0], data[1], data[2], data[3], data[4], data[5]]) },
            (5, Some(name)) => PortId::IfName { name },
            (7, Some(id)) => PortId::Local { id },
            (s, _) => PortId::Other { subtype: s, data: data.to_vec() },
        }
    }
}

/// A management address TLV: where the device answers management protocols.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MgmtAddr {
    /// The address family: 1 IPv4, 2 IPv6, 6 an 802 MAC.
    pub subtype: u8,
    pub addr: Vec<u8>,
    /// How the interface is numbered: 1 unknown, 2 ifIndex, 3 system port number.
    pub if_subtype: u8,
    pub if_number: u32,
    /// The object identifier of the hardware component, often empty.
    pub oid: Vec<u8>,
}

impl MgmtAddr {
    pub fn ipv4(&self) -> Option<Ipv4Addr> {
        (self.subtype == 1 && self.addr.len() == 4).then(|| Ipv4Addr::new(self.addr[0], self.addr[1], self.addr[2], self.addr[3]))
    }
}

/// PROFINET's MRP port status (organisation subtype 4): the ring's domain and the port's status
/// of real-time media redundancy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrpPortStatus {
    pub domain_uuid: [u8; 16],
    /// MRRT's port status: 0 off, 1 configured, 2 up. Not the state of the ring port.
    pub mrrt: u16,
}

impl MrpPortStatus {
    pub fn domain_hex(&self) -> String {
        hex(&self.domain_uuid)
    }

    pub fn mrrt_name(&self) -> &'static str {
        match self.mrrt {
            0 => "off",
            1 => "configured",
            2 => "up",
            _ => "unknown",
        }
    }
}

/// PROFINET's measured delay values (organisation subtype 1), in nanoseconds, 0 for unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delays {
    pub rx_local: u32,
    pub rx_remote: u32,
    pub tx_local: u32,
    pub tx_remote: u32,
    pub cable_local: u32,
}

/// PROFINET's port status (organisation subtype 2): the RT class 2 and RT class 3 port status
/// words, as they travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortStatus {
    pub rtc2: u16,
    pub rtc3: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PnTlvs {
    pub delays: Option<Delays>,
    pub port_status: Option<PortStatus>,
    pub mrp: Option<MrpPortStatus>,
    /// Organisation subtype 5: the device's interface MAC, where the frame's source is the port's.
    pub chassis_mac: Option<Mac>,
}

/// IEEE 802.3's MAC/PHY configuration and status (organisation subtype 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacPhy {
    pub autoneg: u8,
    pub pmd_capabilities: u16,
    pub mau_type: u16,
}

/// An organisation TLV this codec does not read, its bytes kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgTlv {
    pub oui: [u8; 3],
    pub subtype: u8,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LldpPdu {
    pub chassis: ChassisId,
    pub port: PortId,
    /// Seconds the receiver keeps this information; 0 says the sender is leaving.
    pub ttl: u16,
    pub port_desc: Option<String>,
    pub sys_name: Option<String>,
    pub sys_desc: Option<String>,
    /// (capabilities, enabled): bit 2 bridge, bit 4 router, bit 7 station only (802.1AB).
    pub sys_caps: Option<(u16, u16)>,
    pub mgmt_addrs: Vec<MgmtAddr>,
    pub pn: PnTlvs,
    pub port_vlan: Option<u16>,
    pub mac_phy: Option<MacPhy>,
    /// TLVs of a type this codec does not read (or too short for theirs): (type, value), kept.
    pub unknown: Vec<(u8, Vec<u8>)>,
    pub unknown_org: Vec<OrgTlv>,
    /// The End TLV's value: `Some` of nothing as the standard writes it, `Some` of what a sender
    /// put in it (kept as heard), or `None` when the PDU had no End TLV and ended at its last
    /// whole TLV.
    pub end: Option<Vec<u8>>,
    /// A PDU with no End: the byte after its last whole TLV, too short for a TLV's header, kept.
    pub trailing: Vec<u8>,
}

impl LldpPdu {
    /// A PDU with the three mandatory TLVs and nothing else.
    pub fn new(chassis: ChassisId, port: PortId, ttl: u16) -> LldpPdu {
        LldpPdu {
            chassis,
            port,
            ttl,
            port_desc: None,
            sys_name: None,
            sys_desc: None,
            sys_caps: None,
            mgmt_addrs: Vec::new(),
            pn: PnTlvs::default(),
            port_vlan: None,
            mac_phy: None,
            unknown: Vec::new(),
            unknown_org: Vec::new(),
            end: Some(Vec::new()),
            trailing: Vec::new(),
        }
    }

    /// The sender is leaving: forget what it said.
    pub fn is_shutdown(&self) -> bool {
        self.ttl == 0
    }
}

/// The system capability bits, named (802.1AB).
pub fn capability_names(bits: u16) -> Vec<&'static str> {
    let names = [
        "other", "repeater", "bridge", "wlan-access-point", "router", "telephone", "docsis-cable-device", "station-only", "c-vlan",
        "s-vlan", "tpmr",
    ];
    names.iter().enumerate().filter(|(i, _)| bits & (1 << i) != 0).map(|(_, n)| *n).collect()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LldpError {
    #[error("an LLDP PDU is at least 2 bytes; this one is {0}")]
    TooShort(usize),
    #[error("TLV {index} (type {tlv_type}) says {len} bytes and {left} are left")]
    Overrun { index: usize, tlv_type: u8, len: usize, left: usize },
    #[error("the mandatory {0} TLV is missing")]
    Missing(&'static str),
    #[error("a TLV value holds at most 511 bytes; type {tlv_type} has {len}")]
    TlvTooLong { tlv_type: u8, len: usize },
    #[error("a PDU's trailing bytes are the one byte after its last TLV when it has no End; {0} cannot be written")]
    Trailing(usize),
    #[error("a TLV kept unread is of type 1 to 127 (0 is the End TLV), not {0}")]
    BadType(u8),
    #[error("{what} is {min} to {max} bytes; this one is {len}")]
    Length { what: &'static str, min: usize, max: usize, len: usize },
    #[error("a management address of family {subtype} is {want} bytes; this one is {len}")]
    BadAddress { subtype: u8, want: usize, len: usize },
    #[error("{what} of subtype {subtype} is a MAC of 6 bytes or a network address of its family's length; this one is {len} bytes")]
    IdForm { what: &'static str, subtype: u8, len: usize },
    #[error("a PDU with no End is framed only when it fills the frame ({min} bytes); this one is {len}, and the pad would be read as more of it")]
    NoEndInPad { len: usize, min: usize },
    #[error("{0}")]
    Ether(#[from] EtherError),
}

/// Decode an LLDP PDU (the bytes after the EtherType). It ends at the End TLV or at the last
/// whole TLV; the three mandatory TLVs (chassis id, port id, time to live) must be there.
pub fn decode(payload: &[u8]) -> Result<LldpPdu, LldpError> {
    if payload.len() < 2 {
        return Err(LldpError::TooShort(payload.len()));
    }
    let (mut chassis, mut port, mut ttl) = (None, None, None);
    let mut pdu = LldpPdu::new(ChassisId::Other { subtype: 0, data: Vec::new() }, PortId::Other { subtype: 0, data: Vec::new() }, 0);
    let mut off = 0usize;
    let mut index = 0usize;
    pdu.end = None;
    while off + 2 <= payload.len() {
        let head = u16::from_be_bytes([payload[off], payload[off + 1]]);
        let tlv_type = (head >> 9) as u8;
        let len = usize::from(head & 0x1FF);
        off += 2;
        let left = payload.len() - off;
        if len > left {
            return Err(LldpError::Overrun { index, tlv_type, len, left });
        }
        let v = &payload[off..off + len];
        off += len;
        index += 1;
        // A TLV that says what is already said (a second chassis id, a second system name) or
        // that does not fit its type is kept among the unread, never dropped.
        let kept = match tlv_type {
            0 => {
                pdu.end = Some(v.to_vec());
                break;
            }
            1 if chassis.is_none() && (2..=256).contains(&v.len()) && id_fits(4, 5, v) => {
                chassis = Some(ChassisId::decode(v));
                true
            }
            2 if port.is_none() && (2..=256).contains(&v.len()) && id_fits(3, 4, v) => {
                port = Some(PortId::decode(v));
                true
            }
            3 if ttl.is_none() && v.len() == 2 => {
                ttl = Some(u16::from_be_bytes([v[0], v[1]]));
                true
            }
            4 => set_text(&mut pdu.port_desc, v),
            5 => set_text(&mut pdu.sys_name, v),
            6 => set_text(&mut pdu.sys_desc, v),
            7 if pdu.sys_caps.is_none() && v.len() == 4 => {
                pdu.sys_caps = Some((u16::from_be_bytes([v[0], v[1]]), u16::from_be_bytes([v[2], v[3]])));
                true
            }
            8 => match decode_mgmt(v) {
                Some(m) => {
                    pdu.mgmt_addrs.push(m);
                    true
                }
                None => false,
            },
            127 if v.len() >= 4 => {
                decode_org(&mut pdu, [v[0], v[1], v[2]], v[3], &v[4..]);
                true
            }
            _ => false,
        };
        if !kept {
            pdu.unknown.push((tlv_type, v.to_vec()));
        }
    }
    if pdu.end.is_none() {
        pdu.trailing = payload[off..].to_vec();
    }
    pdu.chassis = chassis.ok_or(LldpError::Missing("chassis id"))?;
    pdu.port = port.ok_or(LldpError::Missing("port id"))?;
    pdu.ttl = ttl.ok_or(LldpError::Missing("time to live"))?;
    Ok(pdu)
}

/// Fill an empty text field from UTF-8 bytes; `false` (keep the TLV unread) when the field is
/// already said, the bytes are not text, or there are more than 802.1AB's 255.
fn set_text(field: &mut Option<String>, v: &[u8]) -> bool {
    match (field.is_none() && v.len() <= MAX_TEXT, text(v)) {
        (true, Some(t)) => {
            *field = Some(t);
            true
        }
        _ => false,
    }
}

fn decode_org(pdu: &mut LldpPdu, oui: [u8; 3], subtype: u8, d: &[u8]) {
    let u16_at = |i: usize| u16::from_be_bytes([d[i], d[i + 1]]);
    let u32_at = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    match (oui, subtype) {
        (OUI_PROFINET, 1) if pdu.pn.delays.is_none() && d.len() == 20 => {
            pdu.pn.delays = Some(Delays { rx_local: u32_at(0), rx_remote: u32_at(4), tx_local: u32_at(8), tx_remote: u32_at(12), cable_local: u32_at(16) })
        }
        (OUI_PROFINET, 2) if pdu.pn.port_status.is_none() && d.len() == 4 => pdu.pn.port_status = Some(PortStatus { rtc2: u16_at(0), rtc3: u16_at(2) }),
        (OUI_PROFINET, 4) if pdu.pn.mrp.is_none() && d.len() == 18 => {
            let mut domain_uuid = [0u8; 16];
            domain_uuid.copy_from_slice(&d[..16]);
            pdu.pn.mrp = Some(MrpPortStatus { domain_uuid, mrrt: u16_at(16) });
        }
        (OUI_PROFINET, 5) if pdu.pn.chassis_mac.is_none() && d.len() == 6 => pdu.pn.chassis_mac = Mac::from_slice(d),
        (OUI_IEEE_802_1, 1) if pdu.port_vlan.is_none() && d.len() == 2 => pdu.port_vlan = Some(u16_at(0)),
        (OUI_IEEE_802_3, 1) if pdu.mac_phy.is_none() && d.len() == 5 => pdu.mac_phy = Some(MacPhy { autoneg: d[0], pmd_capabilities: u16_at(1), mau_type: u16_at(3) }),
        _ => pdu.unknown_org.push(OrgTlv { oui, subtype, data: d.to_vec() }),
    }
}

/// address string length (1 + address), family, address, interface numbering subtype, interface
/// number, OID length, OID.
fn decode_mgmt(v: &[u8]) -> Option<MgmtAddr> {
    let alen = usize::from(*v.first()?);
    if !(2..=32).contains(&alen) || v.len() < 1 + alen + 6 {
        return None;
    }
    let rest = &v[1 + alen..];
    let oid_len = usize::from(rest[5]);
    if rest.len() != 6 + oid_len || oid_len > MAX_OID || family_len(v[1]).is_some_and(|n| n != alen - 1) {
        return None;
    }
    Some(MgmtAddr {
        subtype: v[1],
        addr: v[2..1 + alen].to_vec(),
        if_subtype: rest[0],
        if_number: u32::from_be_bytes([rest[1], rest[2], rest[3], rest[4]]),
        oid: rest[6..].to_vec(),
    })
}

/// The bytes as text when they are UTF-8; `None` keeps them as bytes elsewhere.
fn text(b: &[u8]) -> Option<String> {
    std::str::from_utf8(b).ok().map(str::to_string)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn push_tlv(out: &mut Vec<u8>, tlv_type: u8, value: &[u8]) -> Result<(), LldpError> {
    if tlv_type > 127 {
        return Err(LldpError::BadType(tlv_type));
    }
    if value.len() > MAX_TLV_VALUE {
        return Err(LldpError::TlvTooLong { tlv_type, len: value.len() });
    }
    let head = (u16::from(tlv_type) << 9) | value.len() as u16;
    out.extend_from_slice(&head.to_be_bytes());
    out.extend_from_slice(value);
    Ok(())
}

fn push_org(out: &mut Vec<u8>, oui: [u8; 3], subtype: u8, data: &[u8]) -> Result<(), LldpError> {
    push_tlv(out, 127, &[&oui[..], &[subtype], data].concat())
}

fn check_len(what: &'static str, min: usize, max: usize, len: usize) -> Result<(), LldpError> {
    if (min..=max).contains(&len) { Ok(()) } else { Err(LldpError::Length { what, min, max, len }) }
}

/// Encode a PDU (no Ethernet header): the mandatory TLVs first, in the standard's order, then
/// the optional ones, the organisation TLVs, those kept unread, and the End TLV as the PDU had it
/// (none when it had none). What a TLV cannot carry is refused, never cut: a chassis or port id
/// outside 1 to 255 bytes, or a MAC id not of 6 bytes or a network-address id not of its family's
/// length; a description or system name past 255; a management address outside 1 to 31 bytes or
/// not its family's length; an OID past 128; any value past 511; a kept TLV of type 0 or past 127.
pub fn encode(pdu: &LldpPdu) -> Result<Vec<u8>, LldpError> {
    let mut out = Vec::new();
    let chassis = pdu.chassis.encode();
    check_len("a chassis id", 1, MAX_TEXT, chassis.len() - 1)?;
    if !id_fits(4, 5, &chassis) {
        return Err(LldpError::IdForm { what: "a chassis id", subtype: chassis[0], len: chassis.len() - 1 });
    }
    let port = pdu.port.encode();
    check_len("a port id", 1, MAX_TEXT, port.len() - 1)?;
    if !id_fits(3, 4, &port) {
        return Err(LldpError::IdForm { what: "a port id", subtype: port[0], len: port.len() - 1 });
    }
    push_tlv(&mut out, 1, &chassis)?;
    push_tlv(&mut out, 2, &port)?;
    push_tlv(&mut out, 3, &pdu.ttl.to_be_bytes())?;
    for (t, what, s) in [(4, "a port description", &pdu.port_desc), (5, "a system name", &pdu.sys_name), (6, "a system description", &pdu.sys_desc)] {
        if let Some(s) = s {
            check_len(what, 0, MAX_TEXT, s.len())?;
            push_tlv(&mut out, t, s.as_bytes())?;
        }
    }
    if let Some((c, e)) = pdu.sys_caps {
        push_tlv(&mut out, 7, &[c.to_be_bytes(), e.to_be_bytes()].concat())?;
    }
    for m in &pdu.mgmt_addrs {
        check_len("a management address", 1, 31, m.addr.len())?;
        if let Some(want) = family_len(m.subtype).filter(|&n| n != m.addr.len()) {
            return Err(LldpError::BadAddress { subtype: m.subtype, want, len: m.addr.len() });
        }
        check_len("an OID", 0, MAX_OID, m.oid.len())?;
        let mut v = vec![(m.addr.len() + 1) as u8, m.subtype];
        v.extend_from_slice(&m.addr);
        v.push(m.if_subtype);
        v.extend_from_slice(&m.if_number.to_be_bytes());
        v.push(m.oid.len() as u8);
        v.extend_from_slice(&m.oid);
        push_tlv(&mut out, 8, &v)?;
    }
    if let Some(d) = pdu.pn.delays {
        let v: Vec<u8> = [d.rx_local, d.rx_remote, d.tx_local, d.tx_remote, d.cable_local].iter().flat_map(|x| x.to_be_bytes()).collect();
        push_org(&mut out, OUI_PROFINET, 1, &v)?;
    }
    if let Some(s) = pdu.pn.port_status {
        push_org(&mut out, OUI_PROFINET, 2, &[s.rtc2.to_be_bytes(), s.rtc3.to_be_bytes()].concat())?;
    }
    if let Some(m) = &pdu.pn.mrp {
        push_org(&mut out, OUI_PROFINET, 4, &[&m.domain_uuid[..], &m.mrrt.to_be_bytes()].concat())?;
    }
    if let Some(mac) = pdu.pn.chassis_mac {
        push_org(&mut out, OUI_PROFINET, 5, &mac.0)?;
    }
    if let Some(pvid) = pdu.port_vlan {
        push_org(&mut out, OUI_IEEE_802_1, 1, &pvid.to_be_bytes())?;
    }
    if let Some(m) = &pdu.mac_phy {
        push_org(&mut out, OUI_IEEE_802_3, 1, &[&[m.autoneg][..], &m.pmd_capabilities.to_be_bytes(), &m.mau_type.to_be_bytes()].concat())?;
    }
    for o in &pdu.unknown_org {
        push_org(&mut out, o.oui, o.subtype, &o.data)?;
    }
    for (t, v) in &pdu.unknown {
        if *t == 0 {
            return Err(LldpError::BadType(0));
        }
        push_tlv(&mut out, *t, v)?;
    }
    match &pdu.end {
        Some(end) if pdu.trailing.is_empty() => push_tlv(&mut out, 0, end)?,
        Some(_) => return Err(LldpError::Trailing(pdu.trailing.len())),
        None if pdu.trailing.len() > 1 => return Err(LldpError::Trailing(pdu.trailing.len())),
        None => out.extend_from_slice(&pdu.trailing),
    }
    Ok(out)
}

/// A whole LLDP frame from `src`: on a PROFINET device, the port's own MAC (the device's MAC
/// rides in the chassis MAC TLV). A PDU with no End that the frame would pad is refused: the pad
/// would be read after its last TLV as more of it (an End, a byte, or with a trailing byte a TLV
/// header running past the frame).
pub fn frame(src: Mac, pdu: &LldpPdu) -> Result<Vec<u8>, LldpError> {
    let bytes = encode(pdu)?;
    let room = ether::MIN_FRAME - 14;
    if pdu.end.is_none() && bytes.len() < room {
        return Err(LldpError::NoEndInPad { len: bytes.len(), min: room });
    }
    Ok(ether::build(Mac::LLDP_MULTICAST, src, ETHERTYPE_LLDP, &bytes)?)
}
