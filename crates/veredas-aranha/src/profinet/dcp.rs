//! DCP, the Discovery and basic Configuration Protocol of PROFINET: the Layer-2 protocol by which
//! a device says who it is (Identify, Hello) and is named and addressed (Set).
//!
//! ```text
//! EtherType 0x8892 | FrameID u16 | ServiceID u8 | ServiceType u8 | Xid u32 |
//!   ResponseDelay u16 | DCPDataLength u16 | blocks
//! block: Option u8 | Suboption u8 | DCPBlockLength u16 | [BlockInfo or BlockQualifier u16] | data | [pad]
//! ```
//!
//! Which blocks carry the word after the length, as the standard and Wireshark's dissector read
//! it: an Identify request's blocks are filters and carry none; the blocks of an Identify or Get
//! answer and of a Hello carry a BlockInfo (for the IP block, whether the address is set and
//! whether the device saw a conflict); a Set request's blocks carry a BlockQualifier (keep the
//! value for good, or only until power-off); a control response (option 5, suboption 4) carries
//! none: it is the option and suboption answered and an error code, three bytes. A Get request
//! is no blocks at all, only option and suboption pairs. A block of odd length is followed by one
//! pad byte its length does not count.
//!
//! Decoding is total: a block too short for its kind is kept with its bytes and the reason, and
//! the blocks around it still decode; only a damaged header is an error.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use super::ether::{self, ETHERTYPE_PROFINET, EtherError};
use super::mac::Mac;

pub const FRAME_ID_HELLO: u16 = 0xFEFC;
pub const FRAME_ID_GET_SET: u16 = 0xFEFD;
pub const FRAME_ID_IDENTIFY_REQ: u16 = 0xFEFE;
pub const FRAME_ID_IDENTIFY_RES: u16 = 0xFEFF;

/// The BlockQualifier of a Set: keep the value through a power cycle, or only until then.
pub const QUALIFIER_TEMPORARY: u16 = 0x0000;
pub const QUALIFIER_PERMANENT: u16 = 0x0001;

/// The value of the identification signal (option 5, suboption 3): flash the device's light.
pub const SIGNAL_FLASH: u16 = 0x0100;

/// Is this frame id one of DCP's four?
pub fn is_dcp_frame_id(id: u16) -> bool {
    (FRAME_ID_HELLO..=FRAME_ID_IDENTIFY_RES).contains(&id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    Get,
    Set,
    Identify,
    Hello,
    Other(u8),
}

impl Service {
    pub fn from_u8(v: u8) -> Service {
        match v {
            3 => Service::Get,
            4 => Service::Set,
            5 => Service::Identify,
            6 => Service::Hello,
            o => Service::Other(o),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Service::Get => 3,
            Service::Set => 4,
            Service::Identify => 5,
            Service::Hello => 6,
            Service::Other(o) => o,
        }
    }
}

/// A request, a successful answer, or an answer that the service is not supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceType {
    Request,
    Success,
    Unsupported,
    Other(u8),
}

impl ServiceType {
    pub fn from_u8(v: u8) -> ServiceType {
        match v {
            0 => ServiceType::Request,
            1 => ServiceType::Success,
            5 => ServiceType::Unsupported,
            o => ServiceType::Other(o),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            ServiceType::Request => 0,
            ServiceType::Success => 1,
            ServiceType::Unsupported => 5,
            ServiceType::Other(o) => o,
        }
    }

    /// The selection bit: an answer, whatever its outcome.
    pub fn is_response(self) -> bool {
        self.to_u8() & 1 == 1
    }
}

/// The IP block's BlockInfo, read: whether the address is set (0 not set, 1 set, 2 set by DHCP)
/// and whether the device itself detected an address conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IpStatus {
    pub set: u8,
    pub conflict: bool,
}

impl IpStatus {
    pub fn from_info(info: u16) -> IpStatus {
        IpStatus { set: (info & 0x7F) as u8, conflict: info & 0x80 != 0 }
    }

    pub fn to_info(self) -> u16 {
        (u16::from(self.set) & 0x7F) | if self.conflict { 0x80 } else { 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpSuite {
    pub ip: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
}

/// The device role bits (option 2, suboption 4), named.
pub fn role_names(role: u8) -> Vec<&'static str> {
    let names = [(0x01, "io-device"), (0x02, "io-controller"), (0x04, "io-multidevice"), (0x08, "pn-supervisor")];
    names.iter().filter(|(bit, _)| role & bit != 0).map(|(_, n)| *n).collect()
}

/// Why a block's bytes could not be read as its kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Damage {
    /// Its kind has a fixed size, and this block has another.
    Length { expected: usize, got: usize },
    /// Its length runs past the bytes the PDU holds; decoding stopped here.
    Overrun { declared: usize, left: usize },
    /// It should carry a BlockInfo or BlockQualifier word and holds a single byte.
    NoInfo,
    /// Its kind is text, and its bytes are not UTF-8.
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BlockBody {
    /// Option 1, suboption 1.
    MacAddress { mac: Mac },
    /// Option 1, suboption 2.
    IpParameter { suite: IpSuite },
    /// Option 1, suboption 3: the suite and four name servers.
    IpFullSuite { suite: IpSuite, dns: [Ipv4Addr; 4] },
    /// Option 2, suboption 1: the vendor's text for the device's type.
    DeviceVendor { value: String },
    /// Option 2, suboption 2.
    NameOfStation { name: String },
    /// Option 2, suboption 3.
    DeviceId { vendor_id: u16, device_id: u16 },
    /// Option 2, suboption 4: the role bits and a reserved byte.
    DeviceRole { role: u8 },
    /// Option 2, suboption 5: the option and suboption pairs the device supports.
    DeviceOptions { options: Vec<(u8, u8)> },
    /// Option 2, suboption 6.
    AliasName { name: String },
    /// Option 2, suboption 7.
    DeviceInstance { high: u8, low: u8 },
    /// Option 2, suboption 8.
    OemDeviceId { vendor_id: u16, device_id: u16 },
    /// Option 2, suboption 10: which remote service interfaces the device offers.
    RsiProperties { value: u16 },
    /// Option 5, suboption 3: the identification signal.
    ControlSignal { value: u16 },
    /// Option 5, suboption 4: the answer to one block of a Set or Get: which option and
    /// suboption, and the error (0 is success).
    ControlResponse { option: u8, suboption: u8, error: u8 },
    /// Option 6, suboption 1.
    DeviceInitiative { value: u16 },
    /// Option 255, suboption 255: the Identify-All filter, no data.
    AllSelector,
    /// One option and suboption pair of a Get request, which carries no length and no data.
    Requested,
    /// An option and suboption this codec does not read, its bytes kept.
    Unknown { data: Vec<u8> },
    /// A block of a known kind whose bytes do not fit it, kept with the reason.
    Damaged { data: Vec<u8>, damage: Damage },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub option: u8,
    pub suboption: u8,
    /// The BlockInfo (answers, Hello) or BlockQualifier (Set requests); `None` where the block
    /// carries neither (filters, control responses, Get request pairs, a block too short for it).
    pub info: Option<u16>,
    pub body: BlockBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcpPdu {
    pub frame_id: u16,
    pub service: Service,
    pub service_type: ServiceType,
    pub xid: u32,
    /// Requests: the response delay factor (answers spread over it, in 10 ms steps);
    /// answers: reserved, 0.
    pub response_delay: u16,
    pub blocks: Vec<Block>,
    /// Bytes inside the data length that make no whole block (fewer than a block's four header
    /// bytes, or a Get request's odd last byte): kept, never dropped unseen.
    pub trailing: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DcpError {
    #[error("a DCP PDU is at least 12 bytes; this one is {0}")]
    TooShort(usize),
    #[error("frame id {0:#06x} is not DCP's")]
    NotDcp(u16),
    #[error("DCPDataLength {declared} runs past the {available} bytes present")]
    LengthOverrun { declared: usize, available: usize },
    #[error("a DCP block holds at most 65535 bytes; option {option} suboption {suboption} has {len}")]
    BlockTooLong { option: u8, suboption: u8, len: usize },
    #[error("the blocks make {0} bytes, past what one frame carries")]
    PduTooLong(usize),
    #[error("{0}")]
    Ether(#[from] EtherError),
}

/// Does this block carry a BlockInfo or BlockQualifier word, in a PDU of this service and type?
fn carries_info(service: Service, service_type: ServiceType, option: u8, suboption: u8) -> bool {
    if (option, suboption) == (5, 4) || (option, suboption) == (0xFF, 0xFF) {
        return false;
    }
    !(service == Service::Identify && service_type == ServiceType::Request)
}

/// Decode a DCP PDU from the PROFINET payload: the bytes after the EtherType, frame id included.
pub fn decode(payload: &[u8]) -> Result<DcpPdu, DcpError> {
    if payload.len() < 12 {
        return Err(DcpError::TooShort(payload.len()));
    }
    let frame_id = u16::from_be_bytes([payload[0], payload[1]]);
    if !is_dcp_frame_id(frame_id) {
        return Err(DcpError::NotDcp(frame_id));
    }
    let service = Service::from_u8(payload[2]);
    let service_type = ServiceType::from_u8(payload[3]);
    let xid = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let response_delay = u16::from_be_bytes([payload[8], payload[9]]);
    let data_len = usize::from(u16::from_be_bytes([payload[10], payload[11]]));
    let available = payload.len() - 12;
    if data_len > available {
        return Err(DcpError::LengthOverrun { declared: data_len, available });
    }
    let data = &payload[12..12 + data_len];
    let mut pdu = DcpPdu { frame_id, service, service_type, xid, response_delay, blocks: Vec::new(), trailing: Vec::new() };

    if service == Service::Get && service_type == ServiceType::Request {
        let (pairs, rest) = data.as_chunks::<2>();
        for &[option, suboption] in pairs {
            pdu.blocks.push(Block { option, suboption, info: None, body: BlockBody::Requested });
        }
        pdu.trailing = rest.to_vec();
        return Ok(pdu);
    }

    let mut off = 0usize;
    while off + 4 <= data.len() {
        let option = data[off];
        let suboption = data[off + 1];
        let len = usize::from(u16::from_be_bytes([data[off + 2], data[off + 3]]));
        off += 4;
        let left = data.len() - off;
        if len > left {
            let body = BlockBody::Damaged { data: data[off..].to_vec(), damage: Damage::Overrun { declared: len, left } };
            pdu.blocks.push(Block { option, suboption, info: None, body });
            off = data.len();
            break;
        }
        let mut bytes = &data[off..off + len];
        let mut info = None;
        if carries_info(service, service_type, option, suboption) {
            match bytes.len() {
                0 => {}
                1 => {
                    let body = BlockBody::Damaged { data: bytes.to_vec(), damage: Damage::NoInfo };
                    pdu.blocks.push(Block { option, suboption, info: None, body });
                    off += len + (len & 1);
                    continue;
                }
                _ => {
                    info = Some(u16::from_be_bytes([bytes[0], bytes[1]]));
                    bytes = &bytes[2..];
                }
            }
        }
        pdu.blocks.push(Block { option, suboption, info, body: decode_body(option, suboption, bytes) });
        off += len + (len & 1);
    }
    if off < data.len() {
        pdu.trailing = data[off..].to_vec();
    }
    Ok(pdu)
}

fn ipv4(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}


fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn decode_body(option: u8, suboption: u8, b: &[u8]) -> BlockBody {
    let exact = |n: usize| (b.len() != n).then(|| BlockBody::Damaged { data: b.to_vec(), damage: Damage::Length { expected: n, got: b.len() } });
    let suite = |b: &[u8]| IpSuite { ip: ipv4(&b[0..4]), mask: ipv4(&b[4..8]), gateway: ipv4(&b[8..12]) };
    let text = |make: fn(String) -> BlockBody| match std::str::from_utf8(b) {
        Ok(t) => make(t.to_string()),
        Err(_) => BlockBody::Damaged { data: b.to_vec(), damage: Damage::Text },
    };
    match (option, suboption) {
        (1, 1) => exact(6).unwrap_or_else(|| BlockBody::MacAddress { mac: Mac([b[0], b[1], b[2], b[3], b[4], b[5]]) }),
        (1, 2) => exact(12).unwrap_or_else(|| BlockBody::IpParameter { suite: suite(b) }),
        (1, 3) => exact(28).unwrap_or_else(|| BlockBody::IpFullSuite {
            suite: suite(b),
            dns: [ipv4(&b[12..16]), ipv4(&b[16..20]), ipv4(&b[20..24]), ipv4(&b[24..28])],
        }),
        (2, 1) => text(|value| BlockBody::DeviceVendor { value }),
        (2, 2) => text(|name| BlockBody::NameOfStation { name }),
        (2, 3) => exact(4).unwrap_or_else(|| BlockBody::DeviceId { vendor_id: u16_at(b, 0), device_id: u16_at(b, 2) }),
        (2, 4) => exact(2).unwrap_or_else(|| BlockBody::DeviceRole { role: b[0] }),
        (2, 5) => match b.len() % 2 {
            0 => BlockBody::DeviceOptions { options: b.as_chunks::<2>().0.iter().map(|&[o, s]| (o, s)).collect() },
            _ => BlockBody::Damaged { data: b.to_vec(), damage: Damage::Length { expected: b.len() + 1, got: b.len() } },
        },
        (2, 6) => text(|name| BlockBody::AliasName { name }),
        (2, 7) => exact(2).unwrap_or_else(|| BlockBody::DeviceInstance { high: b[0], low: b[1] }),
        (2, 8) => exact(4).unwrap_or_else(|| BlockBody::OemDeviceId { vendor_id: u16_at(b, 0), device_id: u16_at(b, 2) }),
        (2, 10) => exact(2).unwrap_or_else(|| BlockBody::RsiProperties { value: u16_at(b, 0) }),
        (5, 3) => exact(2).unwrap_or_else(|| BlockBody::ControlSignal { value: u16_at(b, 0) }),
        (5, 4) => exact(3).unwrap_or_else(|| BlockBody::ControlResponse { option: b[0], suboption: b[1], error: b[2] }),
        (6, 1) => exact(2).unwrap_or_else(|| BlockBody::DeviceInitiative { value: u16_at(b, 0) }),
        (0xFF, 0xFF) => exact(0).unwrap_or(BlockBody::AllSelector),
        _ => BlockBody::Unknown { data: b.to_vec() },
    }
}

impl BlockBody {
    /// The data bytes this body is written as, after the length and any info word.
    fn data(&self) -> Vec<u8> {
        let suite = |s: &IpSuite| [s.ip.octets(), s.mask.octets(), s.gateway.octets()].concat();
        match self {
            BlockBody::MacAddress { mac } => mac.0.to_vec(),
            BlockBody::IpParameter { suite: s } => suite(s),
            BlockBody::IpFullSuite { suite: s, dns } => {
                let mut v = suite(s);
                for d in dns {
                    v.extend_from_slice(&d.octets());
                }
                v
            }
            BlockBody::DeviceVendor { value } => value.as_bytes().to_vec(),
            BlockBody::NameOfStation { name } | BlockBody::AliasName { name } => name.as_bytes().to_vec(),
            BlockBody::DeviceId { vendor_id, device_id } | BlockBody::OemDeviceId { vendor_id, device_id } => {
                [vendor_id.to_be_bytes(), device_id.to_be_bytes()].concat()
            }
            BlockBody::DeviceRole { role } => vec![*role, 0],
            BlockBody::DeviceOptions { options } => options.iter().flat_map(|(o, s)| [*o, *s]).collect(),
            BlockBody::DeviceInstance { high, low } => vec![*high, *low],
            BlockBody::RsiProperties { value } | BlockBody::ControlSignal { value } | BlockBody::DeviceInitiative { value } => {
                value.to_be_bytes().to_vec()
            }
            BlockBody::ControlResponse { option, suboption, error } => vec![*option, *suboption, *error],
            BlockBody::AllSelector | BlockBody::Requested => Vec::new(),
            BlockBody::Unknown { data } | BlockBody::Damaged { data, .. } => data.clone(),
        }
    }
}

/// Append one block: header, info word when given, data, and the pad byte an odd length needs.
fn push_block(out: &mut Vec<u8>, block: &Block) -> Result<(), DcpError> {
    let data = block.body.data();
    let len = data.len() + if block.info.is_some() { 2 } else { 0 };
    let len16 = u16::try_from(len).map_err(|_| DcpError::BlockTooLong { option: block.option, suboption: block.suboption, len })?;
    out.push(block.option);
    out.push(block.suboption);
    out.extend_from_slice(&len16.to_be_bytes());
    if let Some(i) = block.info {
        out.extend_from_slice(&i.to_be_bytes());
    }
    out.extend_from_slice(&data);
    if len & 1 == 1 {
        out.push(0);
    }
    Ok(())
}

impl DcpPdu {
    /// The PROFINET payload (frame id onward). A Get request is written as its pairs; every
    /// other PDU as its blocks, each with its info word when it has one. A block's declared
    /// length is always that of what it holds: a `Damaged` overrun is written as the bytes it
    /// kept, so it decodes as a whole block.
    pub fn encode(&self) -> Result<Vec<u8>, DcpError> {
        let mut blocks = Vec::new();
        if self.service == Service::Get && self.service_type == ServiceType::Request {
            for b in &self.blocks {
                blocks.push(b.option);
                blocks.push(b.suboption);
            }
        } else {
            for b in &self.blocks {
                push_block(&mut blocks, b)?;
            }
        }
        blocks.extend_from_slice(&self.trailing);
        let data_len = u16::try_from(blocks.len()).map_err(|_| DcpError::PduTooLong(blocks.len()))?;
        let mut out = Vec::with_capacity(12 + blocks.len());
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.push(self.service.to_u8());
        out.push(self.service_type.to_u8());
        out.extend_from_slice(&self.xid.to_be_bytes());
        out.extend_from_slice(&self.response_delay.to_be_bytes());
        out.extend_from_slice(&data_len.to_be_bytes());
        out.extend_from_slice(&blocks);
        Ok(out)
    }

    /// The whole Ethernet frame, untagged, padded to 60 bytes.
    pub fn frame(&self, dst: Mac, src: Mac) -> Result<Vec<u8>, DcpError> {
        Ok(ether::build(dst, src, ETHERTYPE_PROFINET, &self.encode()?)?)
    }

    fn request(frame_id: u16, service: Service, xid: u32, response_delay: u16, blocks: Vec<Block>) -> DcpPdu {
        DcpPdu { frame_id, service, service_type: ServiceType::Request, xid, response_delay, blocks, trailing: Vec::new() }
    }
}

/// What one DCP PDU says about the device that sent it: the fields fusion cares about, read out
/// of the blocks. A field the PDU does not carry is `None` (or empty).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub name: Option<String>,
    pub alias: Option<String>,
    pub ip: Option<IpSuite>,
    pub ip_status: Option<IpStatus>,
    pub vendor_id: Option<u16>,
    pub device_id: Option<u16>,
    pub vendor: Option<String>,
    pub role: Option<u8>,
    pub instance: Option<(u8, u8)>,
    pub oem: Option<(u16, u16)>,
    pub options: Vec<(u8, u8)>,
    /// The MAC the device states for itself (option 1, suboption 1).
    pub mac: Option<Mac>,
    pub initiative: Option<u16>,
}

impl DeviceIdentity {
    pub fn from_blocks(blocks: &[Block]) -> DeviceIdentity {
        let mut id = DeviceIdentity::default();
        for b in blocks {
            match &b.body {
                BlockBody::NameOfStation { name } => id.name = Some(name.clone()),
                BlockBody::AliasName { name } => id.alias = Some(name.clone()),
                BlockBody::IpParameter { suite } | BlockBody::IpFullSuite { suite, .. } => {
                    id.ip = Some(suite.clone());
                    id.ip_status = b.info.map(IpStatus::from_info);
                }
                BlockBody::DeviceId { vendor_id, device_id } => {
                    id.vendor_id = Some(*vendor_id);
                    id.device_id = Some(*device_id);
                }
                BlockBody::OemDeviceId { vendor_id, device_id } => id.oem = Some((*vendor_id, *device_id)),
                BlockBody::DeviceVendor { value } => id.vendor = Some(value.clone()),
                BlockBody::DeviceRole { role } => id.role = Some(*role),
                BlockBody::DeviceInstance { high, low } => id.instance = Some((*high, *low)),
                BlockBody::DeviceOptions { options } => id.options = options.clone(),
                BlockBody::MacAddress { mac } => id.mac = Some(*mac),
                BlockBody::DeviceInitiative { value } => id.initiative = Some(*value),
                _ => {}
            }
        }
        id
    }

    pub fn is_empty(&self) -> bool {
        *self == DeviceIdentity::default()
    }

    /// The blocks of an answer or a Hello saying this identity, each with its BlockInfo (0, or
    /// the IP status for the IP block: set and no conflict when none is given). A vendor id
    /// without a device id (or the reverse) says nothing, as the block needs both.
    pub fn blocks(&self) -> Vec<Block> {
        let described = |option, suboption, body| Block { option, suboption, info: Some(0), body };
        let mut out = Vec::new();
        if let Some(mac) = self.mac {
            out.push(described(1, 1, BlockBody::MacAddress { mac }));
        }
        if let Some(suite) = &self.ip {
            let info = self.ip_status.unwrap_or(IpStatus { set: 1, conflict: false }).to_info();
            out.push(Block { option: 1, suboption: 2, info: Some(info), body: BlockBody::IpParameter { suite: suite.clone() } });
        }
        if let Some(value) = &self.vendor {
            out.push(described(2, 1, BlockBody::DeviceVendor { value: value.clone() }));
        }
        if let Some(name) = &self.name {
            out.push(described(2, 2, BlockBody::NameOfStation { name: name.clone() }));
        }
        if let (Some(vendor_id), Some(device_id)) = (self.vendor_id, self.device_id) {
            out.push(described(2, 3, BlockBody::DeviceId { vendor_id, device_id }));
        }
        if let Some(role) = self.role {
            out.push(described(2, 4, BlockBody::DeviceRole { role }));
        }
        if !self.options.is_empty() {
            out.push(described(2, 5, BlockBody::DeviceOptions { options: self.options.clone() }));
        }
        if let Some(name) = &self.alias {
            out.push(described(2, 6, BlockBody::AliasName { name: name.clone() }));
        }
        if let Some((high, low)) = self.instance {
            out.push(described(2, 7, BlockBody::DeviceInstance { high, low }));
        }
        if let Some((vendor_id, device_id)) = self.oem {
            out.push(described(2, 8, BlockBody::OemDeviceId { vendor_id, device_id }));
        }
        if let Some(value) = self.initiative {
            out.push(described(6, 1, BlockBody::DeviceInitiative { value }));
        }
        out
    }
}

/// The filter of an Identify request: every device, or the one with this station name or alias.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum Filter {
    All,
    Name(String),
    Alias(String),
}

/// An Identify request, multicast to every DCP device on the segment. `response_delay` is the
/// factor the devices spread their answers over (10 ms steps; 1 answers at once).
pub fn identify_request(src: Mac, xid: u32, response_delay: u16, filter: &Filter) -> Result<Vec<u8>, DcpError> {
    let block = match filter {
        Filter::All => Block { option: 0xFF, suboption: 0xFF, info: None, body: BlockBody::AllSelector },
        Filter::Name(n) => Block { option: 2, suboption: 2, info: None, body: BlockBody::NameOfStation { name: n.clone() } },
        Filter::Alias(n) => Block { option: 2, suboption: 6, info: None, body: BlockBody::AliasName { name: n.clone() } },
    };
    DcpPdu::request(FRAME_ID_IDENTIFY_REQ, Service::Identify, xid, response_delay, vec![block]).frame(Mac::DCP_MULTICAST, src)
}

/// An Identify answer, unicast to whoever asked: what a device (or the bench's emulation of one)
/// says of itself.
pub fn identify_response(dst: Mac, src: Mac, xid: u32, id: &DeviceIdentity) -> Result<Vec<u8>, DcpError> {
    let pdu = DcpPdu {
        frame_id: FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid,
        response_delay: 0,
        blocks: id.blocks(),
        trailing: Vec::new(),
    };
    pdu.frame(dst, src)
}

/// A Hello: a device announcing itself, multicast, as it starts.
pub fn hello(src: Mac, xid: u32, id: &DeviceIdentity) -> Result<Vec<u8>, DcpError> {
    DcpPdu::request(FRAME_ID_HELLO, Service::Hello, xid, 0, id.blocks()).frame(Mac::DCP_MULTICAST, src)
}

/// A Get request for these option and suboption pairs, to one device.
pub fn get_request(dst: Mac, src: Mac, xid: u32, pairs: &[(u8, u8)]) -> Result<Vec<u8>, DcpError> {
    let blocks = pairs.iter().map(|&(option, suboption)| Block { option, suboption, info: None, body: BlockBody::Requested }).collect();
    DcpPdu::request(FRAME_ID_GET_SET, Service::Get, xid, 0, blocks).frame(dst, src)
}

// ---- Set: the configuring writes. ------------------------------------------------------------

/// Why a NameOfStation breaks the standard's rules for one.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("a station name is 1 to 240 characters; this one is {0}")]
    Length(usize),
    #[error("label {0} is empty (two dots together, or a dot at an end)")]
    EmptyLabel(usize),
    #[error("label {index} has {len} characters; a label has at most 63")]
    LabelTooLong { index: usize, len: usize },
    #[error("{0:?} is not a letter a to z, a digit or '-'")]
    Character(char),
    #[error("label {0} starts or ends with '-'")]
    HyphenAtEdge(usize),
    #[error("the first label has the form of a port name (port-xyz or port-xyz-abcde)")]
    PortName,
    #[error("the name has the form of an IP address (n.n.n.n)")]
    AddressForm,
}

/// The standard's rules for a NameOfStation: 1 to 240 characters; labels joined by '.', each 1
/// to 63 of `a`-`z`, `0`-`9` and `-`, never starting or ending with `-`; the first label not of
/// the form `port-xyz` or `port-xyz-abcde` (digits); the whole not of the form `n.n.n.n`. A name
/// that breaks them is refused, never rewritten.
pub fn check_station_name(name: &str) -> Result<(), NameError> {
    let len = name.chars().count();
    if len == 0 || len > 240 {
        return Err(NameError::Length(len));
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '.')) {
        return Err(NameError::Character(c));
    }
    let labels: Vec<&str> = name.split('.').collect();
    for (i, l) in labels.iter().enumerate() {
        if l.is_empty() {
            return Err(NameError::EmptyLabel(i));
        }
        if l.len() > 63 {
            return Err(NameError::LabelTooLong { index: i, len: l.len() });
        }
        if l.starts_with('-') || l.ends_with('-') {
            return Err(NameError::HyphenAtEdge(i));
        }
    }
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|c| c.is_ascii_digit());
    let first = labels[0];
    if let Some(rest) = first.strip_prefix("port-") {
        let parts: Vec<&str> = rest.split('-').collect();
        let port = match parts.as_slice() {
            [p] => digits(p, 3),
            [p, q] => digits(p, 3) && digits(q, 5),
            _ => false,
        };
        if port {
            return Err(NameError::PortName);
        }
    }
    if labels.len() == 4 && labels.iter().all(|l| (1..=3).contains(&l.len()) && l.bytes().all(|c| c.is_ascii_digit())) {
        return Err(NameError::AddressForm);
    }
    Ok(())
}

/// Why a Set is refused before it is written.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SetError {
    #[error("a DCP Set names one device by its MAC; {0} is a group address")]
    GroupTarget(Mac),
    #[error("{0}")]
    Name(#[from] NameError),
    #[error("{0}")]
    Dcp(#[from] DcpError),
}

fn set_frame(dst: Mac, src: Mac, xid: u32, block: Block) -> Result<Vec<u8>, SetError> {
    if dst.is_multicast() {
        return Err(SetError::GroupTarget(dst));
    }
    Ok(DcpPdu::request(FRAME_ID_GET_SET, Service::Set, xid, 0, vec![block]).frame(dst, src)?)
}

fn qualifier(permanent: bool) -> u16 {
    if permanent { QUALIFIER_PERMANENT } else { QUALIFIER_TEMPORARY }
}

/// Set a device's NameOfStation. Refused for a group target or a name that breaks the rules.
pub fn set_name_request(dst: Mac, src: Mac, xid: u32, name: &str, permanent: bool) -> Result<Vec<u8>, SetError> {
    check_station_name(name)?;
    let body = BlockBody::NameOfStation { name: name.to_string() };
    set_frame(dst, src, xid, Block { option: 2, suboption: 2, info: Some(qualifier(permanent)), body })
}

/// Set a device's IP suite. Refused for a group target.
pub fn set_ip_request(dst: Mac, src: Mac, xid: u32, suite: &IpSuite, permanent: bool) -> Result<Vec<u8>, SetError> {
    let body = BlockBody::IpParameter { suite: suite.clone() };
    set_frame(dst, src, xid, Block { option: 1, suboption: 2, info: Some(qualifier(permanent)), body })
}

/// Ask a device to flash its light, so a person sees which box answers to a MAC. Momentary: its
/// qualifier is 0. Refused for a group target.
pub fn signal_request(dst: Mac, src: Mac, xid: u32) -> Result<Vec<u8>, SetError> {
    let body = BlockBody::ControlSignal { value: SIGNAL_FLASH };
    set_frame(dst, src, xid, Block { option: 5, suboption: 3, info: Some(QUALIFIER_TEMPORARY), body })
}

/// The answer to a Set: one control response per block set, `(option, suboption, error)`.
pub fn set_response(dst: Mac, src: Mac, xid: u32, answers: &[(u8, u8, u8)]) -> Result<Vec<u8>, DcpError> {
    let blocks = answers
        .iter()
        .map(|&(option, suboption, error)| Block { option: 5, suboption: 4, info: None, body: BlockBody::ControlResponse { option, suboption, error } })
        .collect();
    let pdu = DcpPdu { frame_id: FRAME_ID_GET_SET, service: Service::Set, service_type: ServiceType::Success, xid, response_delay: 0, blocks, trailing: Vec::new() };
    pdu.frame(dst, src)
}

/// The control responses an answer carries, `(option, suboption, error)`; error 0 is success.
pub fn control_responses(pdu: &DcpPdu) -> Vec<(u8, u8, u8)> {
    pdu.blocks
        .iter()
        .filter_map(|b| match &b.body {
            BlockBody::ControlResponse { option, suboption, error } => Some((*option, *suboption, *error)),
            _ => None,
        })
        .collect()
}

/// A control response's error code in words, as the standard names them.
pub fn error_text(code: u8) -> &'static str {
    match code {
        0 => "ok",
        1 => "option not supported",
        2 => "suboption not supported or no data set available",
        3 => "suboption not set",
        4 => "resource error",
        5 => "set not possible for local reasons",
        6 => "in operation, set not possible",
        _ => "unknown error code",
    }
}
