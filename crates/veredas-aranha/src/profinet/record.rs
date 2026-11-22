//! The records a PNIO read returns that the scanner reads: I&M0, what a device says it is (index
//! 0xAFF0, block 0x0020), and the port's real data, what a port sees at its far end (index 0x802A,
//! block 0x020F): its own port id, its peers' port and chassis ids, MACs and line delays, its MAU
//! type, boundaries, link state and medium. The peers are what draws a device-to-device link
//! (`docs/aranha-plan.md`, "Traps": passive LLDP gives nodes, not edges).
//!
//! Both records are blocks of version 1.0, big-endian, read from a [`Block`] and written as one.
//! The port's data aligns its fields to four bytes from the block's start with zero padding.

use serde::{Deserialize, Serialize};

use super::mac::Mac;
use super::rpc::Block;

/// The record indexes.
pub const INDEX_IM0: u16 = 0xAFF0;
pub const INDEX_PORT_DATA_REAL: u16 = 0x802A;

/// The block types.
pub const IM0: u16 = 0x0020;
pub const PORT_DATA_REAL: u16 = 0x020F;

/// I&M0's fixed texts' lengths.
pub const ORDER_ID: usize = 20;
pub const SERIAL_NUMBER: usize = 16;

/// The software revision: a prefix letter (V, R, P, U or T) and three numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareRevision {
    pub prefix: u8,
    pub functional: u8,
    pub bugfix: u8,
    pub internal: u8,
}

/// I&M0: the device's identity as its maker wrote it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Im0 {
    pub vendor_id: u16,
    /// Twenty visible characters, padded with spaces as the device sends them.
    pub order_id: String,
    /// Sixteen visible characters, padded with spaces as the device sends them.
    pub serial_number: String,
    pub hardware_revision: u16,
    pub software_revision: SoftwareRevision,
    pub revision_counter: u16,
    pub profile_id: u16,
    pub profile_specific_type: u16,
    pub version: (u8, u8),
    /// Bit n: I&M n is supported (1 to 15); bit 0 is reserved.
    pub supported: u16,
}

/// A port's link: the port's state (bits 8 to 15) and the link's (bits 0 to 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkState {
    pub port: u8,
    pub link: u8,
}

impl LinkState {
    /// The link is up (1).
    pub fn up(self) -> bool {
        self.link == 1
    }
}

/// One peer a port sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub port_id: String,
    pub chassis_id: String,
    /// The delay word as it travels: bit 31 says whether bits 0 to 30 are the line delay (0) or the
    /// cable delay (1), in nanoseconds.
    pub line_delay: u32,
    pub mac: Mac,
}

impl Peer {
    /// Bit 31: the value is the cable delay, not the line delay.
    pub fn cable_delay(&self) -> bool {
        self.line_delay & 0x8000_0000 != 0
    }

    /// Bits 0 to 30: the delay in nanoseconds.
    pub fn delay_ns(&self) -> u32 {
        self.line_delay & 0x7FFF_FFFF
    }
}

/// A port's real data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDataReal {
    pub slot: u16,
    pub subslot: u16,
    pub own_port_id: String,
    pub peers: Vec<Peer>,
    pub mau_type: u16,
    pub domain_boundary: u32,
    pub multicast_boundary: u32,
    pub link_state: LinkState,
    /// 0 unknown, 1 copper, 2 fibre, 3 radio.
    pub media_type: u32,
    /// Bytes after the medium, kept as heard (a later version's fields).
    pub extra: Vec<u8>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecordError {
    #[error("{what} is block {want:#06x} version 1.0; this block is {got:#06x} version {high}.{low}")]
    Kind { what: &'static str, want: u16, got: u16, high: u8, low: u8 },
    #[error("{what} is {want} bytes after its version; this block holds {got}")]
    Length { what: &'static str, want: usize, got: usize },
    #[error("the port's data ends before its {0}")]
    CutShort(&'static str),
    #[error("the port's data has padding that is not zero before its {0}")]
    Padding(&'static str),
    #[error("the port's data holds at most 255 peers; this one has {0}")]
    Peers(usize),
    #[error("the {field} is {want} visible characters (space to tilde); {got:?} is not")]
    Text { field: &'static str, want: String, got: String },
}

/// Visible characters only, space to tilde.
fn visible(b: &[u8]) -> Option<String> {
    b.iter().all(|c| (0x20..=0x7E).contains(c)).then(|| b.iter().map(|&c| c as char).collect())
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

const IM0_BODY: usize = 54;

impl Im0 {
    pub fn from_block(b: &Block) -> Result<Im0, RecordError> {
        if b.block_type != IM0 || b.version != (1, 0) {
            return Err(RecordError::Kind { what: "I&M0", want: IM0, got: b.block_type, high: b.version.0, low: b.version.1 });
        }
        let x = &b.body;
        if x.len() != IM0_BODY {
            return Err(RecordError::Length { what: "I&M0", want: IM0_BODY, got: x.len() });
        }
        let text = |field: &'static str, at: usize, n: usize| {
            visible(&x[at..at + n]).ok_or_else(|| RecordError::Text { field, want: n.to_string(), got: String::from_utf8_lossy(&x[at..at + n]).into() })
        };
        let prefix = x[40];
        if !(0x20..=0x7E).contains(&prefix) {
            return Err(RecordError::Text { field: "software revision's prefix", want: "1".into(), got: String::from_utf8_lossy(&[prefix]).into() });
        }
        Ok(Im0 {
            vendor_id: be16(x, 0),
            order_id: text("order id", 2, ORDER_ID)?,
            serial_number: text("serial number", 22, SERIAL_NUMBER)?,
            hardware_revision: be16(x, 38),
            software_revision: SoftwareRevision { prefix, functional: x[41], bugfix: x[42], internal: x[43] },
            revision_counter: be16(x, 44),
            profile_id: be16(x, 46),
            profile_specific_type: be16(x, 48),
            version: (x[50], x[51]),
            supported: be16(x, 52),
        })
    }

    /// The block. An order id that is not 20 visible characters, a serial number that is not 16,
    /// or a prefix that is not one visible character, is refused (pad a shorter text with spaces).
    pub fn block(&self) -> Result<Block, RecordError> {
        let check = |field: &'static str, s: &str, n: usize| {
            if s.len() == n && visible(s.as_bytes()).is_some() {
                Ok(())
            } else {
                Err(RecordError::Text { field, want: n.to_string(), got: s.to_string() })
            }
        };
        check("order id", &self.order_id, ORDER_ID)?;
        check("serial number", &self.serial_number, SERIAL_NUMBER)?;
        let r = self.software_revision;
        if !(0x20..=0x7E).contains(&r.prefix) {
            return Err(RecordError::Text { field: "software revision's prefix", want: "1".into(), got: String::from_utf8_lossy(&[r.prefix]).into() });
        }
        let mut x = Vec::with_capacity(IM0_BODY);
        x.extend_from_slice(&self.vendor_id.to_be_bytes());
        x.extend_from_slice(self.order_id.as_bytes());
        x.extend_from_slice(self.serial_number.as_bytes());
        x.extend_from_slice(&self.hardware_revision.to_be_bytes());
        x.extend_from_slice(&[r.prefix, r.functional, r.bugfix, r.internal]);
        x.extend_from_slice(&self.revision_counter.to_be_bytes());
        x.extend_from_slice(&self.profile_id.to_be_bytes());
        x.extend_from_slice(&self.profile_specific_type.to_be_bytes());
        x.extend_from_slice(&[self.version.0, self.version.1]);
        x.extend_from_slice(&self.supported.to_be_bytes());
        Ok(Block::new(IM0, x))
    }

    /// I&M `n` (1 to 15) is supported; `false` for 0 and past 15.
    pub fn supports(&self, n: u8) -> bool {
        (1..=15).contains(&n) && self.supported & (1 << n) != 0
    }
}

/// A reader over the port data's body, which knows its offset from the block's start (the body
/// starts 6 bytes in: type, length, version).
struct Reader<'a> {
    x: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], RecordError> {
        let b = self.x.get(self.at..self.at + n).ok_or(RecordError::CutShort(what))?;
        self.at += n;
        Ok(b)
    }

    fn u8(&mut self, what: &'static str) -> Result<u8, RecordError> {
        Ok(self.take(1, what)?[0])
    }

    fn u16(&mut self, what: &'static str) -> Result<u16, RecordError> {
        Ok(be16(self.take(2, what)?, 0))
    }

    fn u32(&mut self, what: &'static str) -> Result<u32, RecordError> {
        Ok(be32(self.take(4, what)?, 0))
    }

    fn pad(&mut self, n: usize, what: &'static str) -> Result<(), RecordError> {
        if self.take(n, what)?.iter().any(|&b| b != 0) {
            return Err(RecordError::Padding(what));
        }
        Ok(())
    }

    /// Zero padding to the next multiple of four from the block's start.
    fn align(&mut self, what: &'static str) -> Result<(), RecordError> {
        self.pad((4 - (6 + self.at) % 4) % 4, what)
    }

    fn text(&mut self, field: &'static str) -> Result<String, RecordError> {
        let n = self.u8(field)? as usize;
        let b = self.take(n, field)?;
        visible(b).ok_or_else(|| RecordError::Text { field, want: "0 to 255".into(), got: String::from_utf8_lossy(b).into() })
    }
}

fn align(x: &mut Vec<u8>) {
    while !(6 + x.len()).is_multiple_of(4) {
        x.push(0);
    }
}

fn put_text(x: &mut Vec<u8>, field: &'static str, s: &str) -> Result<(), RecordError> {
    if s.len() > 255 || visible(s.as_bytes()).is_none() {
        return Err(RecordError::Text { field, want: "0 to 255".into(), got: s.to_string() });
    }
    x.push(s.len() as u8);
    x.extend_from_slice(s.as_bytes());
    Ok(())
}

impl PortDataReal {
    pub fn from_block(b: &Block) -> Result<PortDataReal, RecordError> {
        if b.block_type != PORT_DATA_REAL || b.version != (1, 0) {
            return Err(RecordError::Kind { what: "the port's data", want: PORT_DATA_REAL, got: b.block_type, high: b.version.0, low: b.version.1 });
        }
        let mut r = Reader { x: &b.body, at: 0 };
        r.pad(2, "slot")?;
        let slot = r.u16("slot")?;
        let subslot = r.u16("subslot")?;
        let own_port_id = r.text("own port id")?;
        let n = r.u8("number of peers")?;
        r.align("first peer")?;
        let mut peers = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let port_id = r.text("peer's port id")?;
            let chassis_id = r.text("peer's chassis id")?;
            r.align("peer's line delay")?;
            let line_delay = r.u32("peer's line delay")?;
            let m = r.take(6, "peer's MAC")?;
            let mac = Mac([m[0], m[1], m[2], m[3], m[4], m[5]]);
            r.align("next peer")?;
            peers.push(Peer { port_id, chassis_id, line_delay, mac });
        }
        let mau_type = r.u16("MAU type")?;
        r.align("domain boundary")?;
        let domain_boundary = r.u32("domain boundary")?;
        let multicast_boundary = r.u32("multicast boundary")?;
        let port = r.u8("link state")?;
        let link = r.u8("link state")?;
        r.align("media type")?;
        let media_type = r.u32("media type")?;
        Ok(PortDataReal {
            slot,
            subslot,
            own_port_id,
            peers,
            mau_type,
            domain_boundary,
            multicast_boundary,
            link_state: LinkState { port, link },
            media_type,
            extra: b.body[r.at..].to_vec(),
        })
    }

    /// The block, its padding zero. A text past 255 bytes or not visible characters, or more than
    /// 255 peers, is refused.
    pub fn block(&self) -> Result<Block, RecordError> {
        let mut x = vec![0, 0];
        x.extend_from_slice(&self.slot.to_be_bytes());
        x.extend_from_slice(&self.subslot.to_be_bytes());
        put_text(&mut x, "own port id", &self.own_port_id)?;
        let n = u8::try_from(self.peers.len()).map_err(|_| RecordError::Peers(self.peers.len()))?;
        x.push(n);
        align(&mut x);
        for p in &self.peers {
            put_text(&mut x, "peer's port id", &p.port_id)?;
            put_text(&mut x, "peer's chassis id", &p.chassis_id)?;
            align(&mut x);
            x.extend_from_slice(&p.line_delay.to_be_bytes());
            x.extend_from_slice(&p.mac.0);
            align(&mut x);
        }
        x.extend_from_slice(&self.mau_type.to_be_bytes());
        align(&mut x);
        x.extend_from_slice(&self.domain_boundary.to_be_bytes());
        x.extend_from_slice(&self.multicast_boundary.to_be_bytes());
        x.extend_from_slice(&[self.link_state.port, self.link_state.link]);
        align(&mut x);
        x.extend_from_slice(&self.media_type.to_be_bytes());
        x.extend_from_slice(&self.extra);
        Ok(Block::new(PORT_DATA_REAL, x))
    }
}
