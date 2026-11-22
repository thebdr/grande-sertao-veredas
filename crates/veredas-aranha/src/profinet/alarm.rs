//! The alarm frame of PROFINET IO: the acyclic real-time protocol (RTA) that carries a device's
//! alarms to its controller and the controller's acknowledgements back, on frame id 0xFC01 (high
//! priority, tagged priority 6) or 0xFE01 (low priority, tagged priority 5).
//!
//! ```text
//! FrameID u16 | AlarmDstEndpoint u16 | AlarmSrcEndpoint u16 | PDUType u8 (type, version) |
//!   AddFlags u8 (window size, TACK) | SendSeqNum u16 | AckSeqNum u16 | VarPartLen u16 | VarPart
//! ```
//!
//! A DATA PDU's var part is one block: an alarm notification (block type 0x0001 high, 0x0002 low)
//! or an alarm acknowledgement (0x8001, 0x8002), at block version 1.0. An ACK and a NACK carry
//! nothing; an ERR carries a PNIO status. The sequence numbers, the window and the retries are the
//! alarm connection's (the IO controller's and the device's state machines), not this codec's: it
//! carries them as numbers. A var part this codec does not read (a PDU type outside 1 to 4, a block
//! of another type, priority or version, a length that does not fit its kind) is kept as heard with
//! the reason, and written back as heard. Bytes after the var part are the frame's pad.

use serde::{Deserialize, Serialize};

use super::ether::{self, ETHERTYPE_PROFINET, EtherError, VlanTag};
use super::mac::Mac;

/// The frame ids of the two alarm connections.
pub const FRAME_ID_HIGH: u16 = 0xFC01;
pub const FRAME_ID_LOW: u16 = 0xFE01;

/// The RTA header's length after the frame id.
pub const HEADER: usize = 12;

/// The longest var part: what an Ethernet payload of 1500 bytes leaves after the frame id and the
/// header.
pub const MAX_VAR_PART: usize = ether::MAX_PAYLOAD - 2 - HEADER;

/// Which of the two alarm connections a frame travels on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    High,
    Low,
}

impl Priority {
    pub fn frame_id(self) -> u16 {
        match self {
            Priority::High => FRAME_ID_HIGH,
            Priority::Low => FRAME_ID_LOW,
        }
    }

    /// The priority whose frame id this is, if it is an alarm frame's.
    pub fn of_frame_id(frame_id: u16) -> Option<Priority> {
        match frame_id {
            FRAME_ID_HIGH => Some(Priority::High),
            FRAME_ID_LOW => Some(Priority::Low),
            _ => None,
        }
    }

    /// The tag the frame rides with: priority 6 for high, 5 for low, VLAN 0.
    pub fn tag(self) -> VlanTag {
        let priority = match self {
            Priority::High => 6,
            Priority::Low => 5,
        };
        VlanTag { priority, drop_eligible: false, id: 0 }
    }

    /// The block type of an alarm notification of this priority: 0x0001, 0x0002.
    pub fn notification_block(self) -> u16 {
        match self {
            Priority::High => 0x0001,
            Priority::Low => 0x0002,
        }
    }

    /// The block type of an alarm acknowledgement of this priority: 0x8001, 0x8002.
    pub fn ack_block(self) -> u16 {
        self.notification_block() | 0x8000
    }

    fn other(self) -> Priority {
        match self {
            Priority::High => Priority::Low,
            Priority::Low => Priority::High,
        }
    }
}

/// The PDU type's low nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PduType {
    /// 1: an alarm notification or acknowledgement.
    Data,
    /// 2: a negative acknowledgement of the transport.
    Nack,
    /// 3: the transport's acknowledgement of a DATA PDU.
    Ack,
    /// 4: the alarm connection's error, with its PNIO status.
    Err,
    /// Any other value of the nibble (0, 5 to 15), carried as heard.
    Other(u8),
}

impl PduType {
    fn from_nibble(n: u8) -> PduType {
        match n {
            1 => PduType::Data,
            2 => PduType::Nack,
            3 => PduType::Ack,
            4 => PduType::Err,
            n => PduType::Other(n),
        }
    }

    pub fn nibble(self) -> u8 {
        match self {
            PduType::Data => 1,
            PduType::Nack => 2,
            PduType::Ack => 3,
            PduType::Err => 4,
            PduType::Other(n) => n,
        }
    }
}

/// The AddFlags byte as it travels: the window size (bits 0 to 3), TACK (bit 4), and three reserved
/// bits kept as heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddFlags(pub u8);

impl AddFlags {
    /// Window size 1, as the standard has it, and TACK as given.
    pub fn new(tack: bool) -> AddFlags {
        AddFlags(0x01 | if tack { 0x10 } else { 0 })
    }

    pub fn window_size(self) -> u8 {
        self.0 & 0x0F
    }

    /// Bit 4: the receiver is to acknowledge at once.
    pub fn tack(self) -> bool {
        self.0 & 0x10 != 0
    }
}

/// The alarm specifier, with its bits named: the alarm's sequence number (bits 0 to 10) and four
/// diagnosis flags (bits 11, 12, 13 and 15; bit 14 is reserved).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmSpecifier(pub u16);

impl AlarmSpecifier {
    /// Bits 0 to 10: 0 to 2047.
    pub fn sequence(self) -> u16 {
        self.0 & 0x07FF
    }

    /// Bit 11: the channel has a diagnosis.
    pub fn channel_diagnosis(self) -> bool {
        self.0 & 0x0800 != 0
    }

    /// Bit 12: a manufacturer's diagnosis.
    pub fn manufacturer_diagnosis(self) -> bool {
        self.0 & 0x1000 != 0
    }

    /// Bit 13: the submodule has a diagnosis.
    pub fn submodule_diagnosis(self) -> bool {
        self.0 & 0x2000 != 0
    }

    /// Bit 15: the AR has a diagnosis.
    pub fn ar_diagnosis(self) -> bool {
        self.0 & 0x8000 != 0
    }
}

/// A PNIO status: error code, error decode, error code 1 and error code 2, each a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PnioStatus {
    pub code: u8,
    pub decode: u8,
    pub code1: u8,
    pub code2: u8,
}

impl PnioStatus {
    /// All four bytes zero.
    pub const OK: PnioStatus = PnioStatus { code: 0, decode: 0, code1: 0, code2: 0 };

    pub fn from_bytes(b: [u8; 4]) -> PnioStatus {
        PnioStatus { code: b[0], decode: b[1], code1: b[2], code2: b[3] }
    }

    pub fn to_bytes(self) -> [u8; 4] {
        [self.code, self.decode, self.code1, self.code2]
    }
}

/// What follows an alarm notification's specifier: the first user structure identifier, and every
/// byte after it as data (a channel diagnosis, a manufacturer's data: the device's to define). A
/// maintenance item (0x8100) is followed by the item it qualifies: its 12 bytes, then that item's
/// identifier and data, all in `data`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmItem {
    pub usi: u16,
    pub data: Vec<u8>,
}

/// An alarm notification: which submodule raised which alarm, with its item when it has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub alarm_type: u16,
    pub api: u32,
    pub slot: u16,
    pub subslot: u16,
    pub module_ident: u32,
    pub submodule_ident: u32,
    pub specifier: AlarmSpecifier,
    pub item: Option<AlarmItem>,
}

/// An alarm acknowledgement: the alarm it answers, and how.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmAck {
    pub alarm_type: u16,
    pub api: u32,
    pub slot: u16,
    pub subslot: u16,
    pub specifier: AlarmSpecifier,
    pub status: PnioStatus,
}

/// Why a var part was kept as bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unread {
    /// A PDU type outside 1 to 4.
    Type,
    /// An ACK or NACK carrying bytes, an ERR whose status is not four bytes, a DATA PDU with no
    /// block.
    Shape,
    /// A block of a type that is neither a notification nor an acknowledgement.
    Block(u16),
    /// A notification or acknowledgement of the other priority than the frame's.
    Priority(u16),
    /// A block version other than 1.0.
    Version(u8, u8),
    /// A block whose length is not the var part's, or that is too short for its kind.
    Length,
}

impl Unread {
    pub fn text(&self) -> String {
        match self {
            Unread::Type => "a PDU type this codec does not read".into(),
            Unread::Shape => "a var part that does not fit its PDU type".into(),
            Unread::Block(t) => format!("block type {t:#06x} is neither an alarm notification nor an acknowledgement"),
            Unread::Priority(t) => format!("block type {t:#06x} is of the other priority than its frame"),
            Unread::Version(h, l) => format!("block version {h}.{l} is not 1.0"),
            Unread::Length => "a block whose length does not fit".into(),
        }
    }
}

/// The var part, read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Body {
    /// No var part: an ACK or a NACK.
    Empty,
    Notification(Notification),
    Ack(AlarmAck),
    /// An ERR PDU's status.
    Error(PnioStatus),
    /// The var part as heard, and why it was not read.
    Unread { bytes: Vec<u8>, why: Unread },
}

/// One RTA PDU.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtaPdu {
    pub priority: Priority,
    /// The receiver's alarm reference.
    pub dst_endpoint: u16,
    /// The sender's alarm reference.
    pub src_endpoint: u16,
    pub pdu_type: PduType,
    /// The PDU type's high nibble: 1.
    pub version: u8,
    pub add_flags: AddFlags,
    pub send_seq: u16,
    pub ack_seq: u16,
    pub body: Body,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum AlarmError {
    #[error("an RTA PDU is a frame id and {HEADER} header bytes; this one is {0} bytes")]
    TooShort(usize),
    #[error("frame id {0:#06x} is not an alarm frame's")]
    NotAlarm(u16),
    #[error("the var part is said to be {declared} bytes and {available} follow the header")]
    Overrun { declared: usize, available: usize },
    #[error("the {what} is a nibble; {value} is not")]
    Nibble { what: &'static str, value: u8 },
    #[error("a var part of {0} bytes is past what a frame carries ({MAX_VAR_PART})")]
    TooLong(usize),
    #[error("this PDU would be read back as another: {0}")]
    NotItself(String),
    #[error("{0}")]
    Ether(#[from] EtherError),
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A block's header: type, length (counting from the version), version 1.0.
const BLOCK_HEADER: usize = 6;
/// A notification's fields after the block header, before its item.
const NOTIFICATION: usize = 20;
/// An acknowledgement's fields after the block header.
const ACK: usize = 16;

impl RtaPdu {
    /// Read an RTA PDU from the PROFINET payload (the bytes after the EtherType, without the frame
    /// check sequence). A damaged header (cut short, a frame id not an alarm's, a var part past the
    /// bytes or past what a frame carries) is an error naming it; a var part this codec does not read
    /// is kept with the reason; bytes after the var part are the frame's pad and are not kept.
    pub fn decode(payload: &[u8]) -> Result<RtaPdu, AlarmError> {
        if payload.len() < 2 + HEADER {
            return Err(AlarmError::TooShort(payload.len()));
        }
        let frame_id = be16(payload, 0);
        let priority = Priority::of_frame_id(frame_id).ok_or(AlarmError::NotAlarm(frame_id))?;
        let h = &payload[2..];
        let declared = be16(h, 10) as usize;
        if declared > MAX_VAR_PART {
            return Err(AlarmError::TooLong(declared));
        }
        let available = h.len() - HEADER;
        if declared > available {
            return Err(AlarmError::Overrun { declared, available });
        }
        let pdu_type = PduType::from_nibble(h[4] & 0x0F);
        let var = &h[HEADER..HEADER + declared];
        Ok(RtaPdu {
            priority,
            dst_endpoint: be16(h, 0),
            src_endpoint: be16(h, 2),
            pdu_type,
            version: h[4] >> 4,
            add_flags: AddFlags(h[5]),
            send_seq: be16(h, 6),
            ack_seq: be16(h, 8),
            body: read_body(priority, pdu_type, var),
        })
    }

    /// The PROFINET payload: frame id, header, var part, with no pad (the Ethernet frame pads).
    /// A type or version past a nibble, a var part past what a frame carries, and a PDU that would
    /// be read back as another (a typed body under a PDU type that does not carry it, bytes kept as
    /// unread that would now be read, or with another reason) are refused.
    pub fn encode(&self) -> Result<Vec<u8>, AlarmError> {
        let type_nibble = self.pdu_type.nibble();
        if type_nibble > 0x0F {
            return Err(AlarmError::Nibble { what: "PDU type", value: type_nibble });
        }
        if self.version > 0x0F {
            return Err(AlarmError::Nibble { what: "PDU version", value: self.version });
        }
        let var = write_body(self.priority, &self.body);
        if var.len() > MAX_VAR_PART {
            return Err(AlarmError::TooLong(var.len()));
        }
        let mut out = Vec::with_capacity(2 + HEADER + var.len());
        out.extend_from_slice(&self.priority.frame_id().to_be_bytes());
        out.extend_from_slice(&self.dst_endpoint.to_be_bytes());
        out.extend_from_slice(&self.src_endpoint.to_be_bytes());
        out.push((self.version << 4) | type_nibble);
        out.push(self.add_flags.0);
        out.extend_from_slice(&self.send_seq.to_be_bytes());
        out.extend_from_slice(&self.ack_seq.to_be_bytes());
        out.extend_from_slice(&(var.len() as u16).to_be_bytes());
        out.extend_from_slice(&var);
        let back = RtaPdu::decode(&out)?;
        if &back != self {
            return Err(AlarmError::NotItself(format!("{back:?}")));
        }
        Ok(out)
    }

    /// The whole Ethernet frame, with the tag given (the priority's own is [`Priority::tag`]) or
    /// untagged.
    pub fn frame(&self, dst: Mac, src: Mac, tag: Option<VlanTag>) -> Result<Vec<u8>, AlarmError> {
        Ok(ether::build_tagged(dst, src, tag, ETHERTYPE_PROFINET, &self.encode()?)?)
    }

    /// An alarm notification as the device sends it: a DATA PDU, window 1, TACK set.
    pub fn notification(priority: Priority, dst_endpoint: u16, src_endpoint: u16, send_seq: u16, ack_seq: u16, n: Notification) -> RtaPdu {
        RtaPdu::data(priority, dst_endpoint, src_endpoint, send_seq, ack_seq, Body::Notification(n))
    }

    /// An alarm acknowledgement as the controller sends it: a DATA PDU, window 1, TACK set.
    pub fn alarm_ack(priority: Priority, dst_endpoint: u16, src_endpoint: u16, send_seq: u16, ack_seq: u16, a: AlarmAck) -> RtaPdu {
        RtaPdu::data(priority, dst_endpoint, src_endpoint, send_seq, ack_seq, Body::Ack(a))
    }

    fn data(priority: Priority, dst_endpoint: u16, src_endpoint: u16, send_seq: u16, ack_seq: u16, body: Body) -> RtaPdu {
        RtaPdu { priority, dst_endpoint, src_endpoint, pdu_type: PduType::Data, version: 1, add_flags: AddFlags::new(true), send_seq, ack_seq, body }
    }

    /// The transport's acknowledgement of a DATA PDU: an ACK, window 1, no TACK, nothing carried.
    pub fn transport_ack(priority: Priority, dst_endpoint: u16, src_endpoint: u16, send_seq: u16, ack_seq: u16) -> RtaPdu {
        RtaPdu {
            priority,
            dst_endpoint,
            src_endpoint,
            pdu_type: PduType::Ack,
            version: 1,
            add_flags: AddFlags::new(false),
            send_seq,
            ack_seq,
            body: Body::Empty,
        }
    }

    /// The alarm connection's error: an ERR with its status, window 1, no TACK.
    pub fn error(priority: Priority, dst_endpoint: u16, src_endpoint: u16, send_seq: u16, ack_seq: u16, status: PnioStatus) -> RtaPdu {
        RtaPdu {
            priority,
            dst_endpoint,
            src_endpoint,
            pdu_type: PduType::Err,
            version: 1,
            add_flags: AddFlags::new(false),
            send_seq,
            ack_seq,
            body: Body::Error(status),
        }
    }
}

fn unread(var: &[u8], why: Unread) -> Body {
    Body::Unread { bytes: var.to_vec(), why }
}

fn read_body(priority: Priority, pdu_type: PduType, var: &[u8]) -> Body {
    match pdu_type {
        PduType::Ack | PduType::Nack if var.is_empty() => Body::Empty,
        PduType::Ack | PduType::Nack => unread(var, Unread::Shape),
        PduType::Err => match <[u8; 4]>::try_from(var) {
            Ok(b) => Body::Error(PnioStatus::from_bytes(b)),
            Err(_) => unread(var, Unread::Shape),
        },
        PduType::Data => read_block(priority, var),
        PduType::Other(_) => unread(var, Unread::Type),
    }
}

fn read_block(priority: Priority, var: &[u8]) -> Body {
    if var.is_empty() {
        return unread(var, Unread::Shape);
    }
    if var.len() < BLOCK_HEADER {
        return unread(var, Unread::Length);
    }
    let block_type = be16(var, 0);
    let other = priority.other();
    if block_type == other.notification_block() || block_type == other.ack_block() {
        return unread(var, Unread::Priority(block_type));
    }
    let notification = block_type == priority.notification_block();
    if !notification && block_type != priority.ack_block() {
        return unread(var, Unread::Block(block_type));
    }
    if (var[4], var[5]) != (1, 0) {
        return unread(var, Unread::Version(var[4], var[5]));
    }
    if be16(var, 2) as usize != var.len() - 4 {
        return unread(var, Unread::Length);
    }
    let b = &var[BLOCK_HEADER..];
    if notification {
        if b.len() < NOTIFICATION || b.len() == NOTIFICATION + 1 {
            return unread(var, Unread::Length);
        }
        let item = (b.len() > NOTIFICATION).then(|| AlarmItem { usi: be16(b, NOTIFICATION), data: b[NOTIFICATION + 2..].to_vec() });
        Body::Notification(Notification {
            alarm_type: be16(b, 0),
            api: be32(b, 2),
            slot: be16(b, 6),
            subslot: be16(b, 8),
            module_ident: be32(b, 10),
            submodule_ident: be32(b, 14),
            specifier: AlarmSpecifier(be16(b, 18)),
            item,
        })
    } else {
        if b.len() != ACK {
            return unread(var, Unread::Length);
        }
        Body::Ack(AlarmAck {
            alarm_type: be16(b, 0),
            api: be32(b, 2),
            slot: be16(b, 6),
            subslot: be16(b, 8),
            specifier: AlarmSpecifier(be16(b, 10)),
            status: PnioStatus::from_bytes([b[12], b[13], b[14], b[15]]),
        })
    }
}

fn block(block_type: u16, fields: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BLOCK_HEADER + fields.len());
    out.extend_from_slice(&block_type.to_be_bytes());
    // Counted from the version: what follows the type and the length. Past 65535 the var part is
    // past a frame too, and `encode` refuses it before this length is read.
    out.extend_from_slice(&((fields.len() + 2).min(u16::MAX as usize) as u16).to_be_bytes());
    out.extend_from_slice(&[1, 0]);
    out.extend_from_slice(fields);
    out
}

fn write_body(priority: Priority, body: &Body) -> Vec<u8> {
    match body {
        Body::Empty => Vec::new(),
        Body::Notification(n) => {
            let mut f = Vec::with_capacity(NOTIFICATION + 2);
            f.extend_from_slice(&n.alarm_type.to_be_bytes());
            f.extend_from_slice(&n.api.to_be_bytes());
            f.extend_from_slice(&n.slot.to_be_bytes());
            f.extend_from_slice(&n.subslot.to_be_bytes());
            f.extend_from_slice(&n.module_ident.to_be_bytes());
            f.extend_from_slice(&n.submodule_ident.to_be_bytes());
            f.extend_from_slice(&n.specifier.0.to_be_bytes());
            if let Some(item) = &n.item {
                f.extend_from_slice(&item.usi.to_be_bytes());
                f.extend_from_slice(&item.data);
            }
            block(priority.notification_block(), &f)
        }
        Body::Ack(a) => {
            let mut f = Vec::with_capacity(ACK);
            f.extend_from_slice(&a.alarm_type.to_be_bytes());
            f.extend_from_slice(&a.api.to_be_bytes());
            f.extend_from_slice(&a.slot.to_be_bytes());
            f.extend_from_slice(&a.subslot.to_be_bytes());
            f.extend_from_slice(&a.specifier.0.to_be_bytes());
            f.extend_from_slice(&a.status.to_bytes());
            block(priority.ack_block(), &f)
        }
        Body::Error(s) => s.to_bytes().to_vec(),
        Body::Unread { bytes, .. } => bytes.clone(),
    }
}
