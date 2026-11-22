//! Ethernet II with at most one 802.1Q tag: the envelope every PROFINET frame travels in.
//!
//! ```text
//! dst (6) | src (6) | [0x8100 | PCP:3 DEI:1 VID:12] | EtherType (2) | payload | [pad to 60]
//! ```
//!
//! A frame shorter than 60 bytes (without its check sequence) is padded on the wire, and a capture
//! may or may not hold the pad: the payload handed back runs to the end of the bytes given, and each
//! protocol above bounds itself by its own length field.
//!
//! The two bytes after the addresses (and after a tag) are an EtherType from 0x0600 up; below it
//! they are an IEEE 802.3 length, an LLC header following (a spanning-tree frame is one). `parse`
//! hands the field back as it is and `EtherFrame::is_length` tells the two apart; `build` writes
//! only an EtherType, and never the tag's own type as a frame's type.

use serde::{Deserialize, Serialize};

use super::mac::Mac;

pub const ETHERTYPE_VLAN: u16 = 0x8100;
pub const ETHERTYPE_PROFINET: u16 = 0x8892;
pub const ETHERTYPE_LLDP: u16 = 0x88CC;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_IPV6: u16 = 0x86DD;
/// Listened to and counted, never sent to: an active frame on these buses disturbs their master.
pub const ETHERTYPE_ETHERCAT: u16 = 0x88A4;
pub const ETHERTYPE_POWERLINK: u16 = 0x88AB;
pub const ETHERTYPE_MRP: u16 = 0x88E3;
pub const ETHERTYPE_PTP: u16 = 0x88F7;

/// The shortest frame on the wire, without its check sequence.
pub const MIN_FRAME: usize = 60;
/// The longest payload, tagged or not (a tag adds its four bytes to the frame).
pub const MAX_PAYLOAD: usize = 1500;
/// The lowest value that is an EtherType; below it the field is an 802.3 length.
pub const MIN_ETHERTYPE: u16 = 0x0600;

/// An 802.1Q tag: the priority (0 to 7; PROFINET's cyclic frames ride at 6), the drop-eligible
/// bit, and the VLAN id (0 means "priority only, no VLAN").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlanTag {
    pub priority: u8,
    pub drop_eligible: bool,
    pub id: u16,
}

impl VlanTag {
    fn tci(self) -> u16 {
        (u16::from(self.priority & 7) << 13) | (u16::from(self.drop_eligible) << 12) | (self.id & 0x0FFF)
    }

    fn from_tci(tci: u16) -> VlanTag {
        VlanTag { priority: (tci >> 13) as u8, drop_eligible: tci & 0x1000 != 0, id: tci & 0x0FFF }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EtherFrame<'a> {
    pub dst: Mac,
    pub src: Mac,
    pub vlan: Option<VlanTag>,
    /// The type field as it travels: an EtherType, or an 802.3 length (`is_length`).
    pub ethertype: u16,
    pub payload: &'a [u8],
}

impl EtherFrame<'_> {
    /// The type field is an IEEE 802.3 length (below 0x0600), not an EtherType: an LLC header
    /// follows.
    pub fn is_length(&self) -> bool {
        self.ethertype < MIN_ETHERTYPE
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum EtherError {
    #[error("an Ethernet frame is at least 14 bytes (18 with a VLAN tag); this one is {0}")]
    TooShort(usize),
    #[error("a payload of {0} bytes is past Ethernet's 1500")]
    TooLong(usize),
    #[error("a VLAN priority is 0 to 7, not {0}")]
    BadPriority(u8),
    #[error("a VLAN id is 0 to 4095, not {0}")]
    BadVlanId(u16),
    #[error("{0:#06x} is not an EtherType this envelope writes (below 0x0600 is an 802.3 length; 0x8100 is the tag's own)")]
    NotEtherType(u16),
}

/// Open the envelope. The payload is everything after the EtherType, pad included.
pub fn parse(bytes: &[u8]) -> Result<EtherFrame<'_>, EtherError> {
    if bytes.len() < 14 {
        return Err(EtherError::TooShort(bytes.len()));
    }
    let dst = Mac::from_slice(&bytes[0..6]).ok_or(EtherError::TooShort(bytes.len()))?;
    let src = Mac::from_slice(&bytes[6..12]).ok_or(EtherError::TooShort(bytes.len()))?;
    let mut ethertype = u16::from_be_bytes([bytes[12], bytes[13]]);
    let mut off = 14;
    let mut vlan = None;
    if ethertype == ETHERTYPE_VLAN {
        if bytes.len() < 18 {
            return Err(EtherError::TooShort(bytes.len()));
        }
        vlan = Some(VlanTag::from_tci(u16::from_be_bytes([bytes[14], bytes[15]])));
        ethertype = u16::from_be_bytes([bytes[16], bytes[17]]);
        off = 18;
    }
    Ok(EtherFrame { dst, src, vlan, ethertype, payload: &bytes[off..] })
}

/// An untagged frame, padded to 60 bytes.
pub fn build(dst: Mac, src: Mac, ethertype: u16, payload: &[u8]) -> Result<Vec<u8>, EtherError> {
    build_tagged(dst, src, None, ethertype, payload)
}

/// A frame with an optional 802.1Q tag, padded to 60 bytes. A payload past 1500 bytes, a
/// priority past 7, a VLAN id past 4095, or a type that is not an EtherType (an 802.3 length below
/// 0x0600, or the tag's own 0x8100) is refused, never cut or misread.
pub fn build_tagged(
    dst: Mac,
    src: Mac,
    vlan: Option<VlanTag>,
    ethertype: u16,
    payload: &[u8],
) -> Result<Vec<u8>, EtherError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(EtherError::TooLong(payload.len()));
    }
    if ethertype < MIN_ETHERTYPE || ethertype == ETHERTYPE_VLAN {
        return Err(EtherError::NotEtherType(ethertype));
    }
    let mut out = Vec::with_capacity(18 + payload.len().max(46));
    out.extend_from_slice(&dst.0);
    out.extend_from_slice(&src.0);
    if let Some(tag) = vlan {
        if tag.priority > 7 {
            return Err(EtherError::BadPriority(tag.priority));
        }
        if tag.id > 0x0FFF {
            return Err(EtherError::BadVlanId(tag.id));
        }
        out.extend_from_slice(&ETHERTYPE_VLAN.to_be_bytes());
        out.extend_from_slice(&tag.tci().to_be_bytes());
    }
    out.extend_from_slice(&ethertype.to_be_bytes());
    out.extend_from_slice(payload);
    out.resize(out.len().max(MIN_FRAME), 0);
    Ok(out)
}
