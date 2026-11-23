//! PNIO over connectionless DCE/RPC: what a controller, a supervisor or this scanner says to a
//! PROFINET device over UDP (port 34964), and its answers.
//!
//! ```text
//! UDP payload: the RPC header (80 bytes) | the body (the header's length) | [trailing bytes]
//! the body of a PNIO request:  ArgsMaximum | ArgsLength | MaximumCount | Offset | ActualCount | data
//! the body of a PNIO response: PNIOStatus  | ArgsLength | MaximumCount | Offset | ActualCount | data
//! the data: PNIO blocks (BlockType u16 | BlockLength u16 | version high | version low | body)
//! ```
//!
//! The RPC header's integers, the first three fields of its UUIDs, the arguments' counts and a
//! response's PNIO status (a 32-bit value: error code, decode, code 1, code 2 from its most
//! significant byte) follow the header's data representation (`drep`: byte order big or little);
//! the blocks are big-endian always. This module reads and writes one datagram: a body split into
//! fragments is carried as each fragment's bytes with its number, and joined by the caller; the
//! retries, the activity's sequence and the ports the answers come from are the caller's.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub use super::alarm::PnioStatus;

/// The UDP port PNIO's RPC is addressed to (the endpoint mapper's).
pub const PNIO_PORT: u16 = 34964;

/// The connectionless header's length.
pub const HEADER: usize = 80;

/// The longest datagram UDP over IPv4 carries (65535 less the IPv4 and UDP headers).
pub const MAX_DATAGRAM: usize = 65_507;

/// A UUID, held in the order its text spells it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    pub const NIL: Uuid = Uuid([0; 16]);
    /// The PNIO device interface: what a device serves (connect, release, read, write, control,
    /// read implicit).
    pub const PNIO_DEVICE: Uuid = Uuid::from_text_parts(0xDEA0_0001, 0x6C97, 0x11D1, [0x82, 0x71, 0x00, 0xA0, 0x24, 0x42, 0xDF, 0x7D]);
    /// The PNIO controller interface: what a controller serves (the device's application ready).
    pub const PNIO_CONTROLLER: Uuid = Uuid::from_text_parts(0xDEA0_0002, 0x6C97, 0x11D1, [0x82, 0x71, 0x00, 0xA0, 0x24, 0x42, 0xDF, 0x7D]);
    /// The PNIO supervisor interface.
    pub const PNIO_SUPERVISOR: Uuid = Uuid::from_text_parts(0xDEA0_0003, 0x6C97, 0x11D1, [0x82, 0x71, 0x00, 0xA0, 0x24, 0x42, 0xDF, 0x7D]);
    /// The PNIO parameter server interface.
    pub const PNIO_PARAMETER_SERVER: Uuid = Uuid::from_text_parts(0xDEA0_0004, 0x6C97, 0x11D1, [0x82, 0x71, 0x00, 0xA0, 0x24, 0x42, 0xDF, 0x7D]);
    /// The endpoint mapper (version 3).
    pub const ENDPOINT_MAPPER: Uuid = Uuid::from_text_parts(0xE1AF_8308, 0x5D1F, 0x11C9, [0x91, 0xA4, 0x08, 0x00, 0x2B, 0x14, 0xA0, 0xFA]);

    const fn from_text_parts(a: u32, b: u16, c: u16, d: [u8; 8]) -> Uuid {
        let a = a.to_be_bytes();
        let b = b.to_be_bytes();
        let c = c.to_be_bytes();
        Uuid([a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
    }

    /// The object UUID a PNIO device answers to: `DEA00000-6C97-11D1-8271-` then the instance, the
    /// device id and the vendor id, two bytes each.
    pub fn pnio_object(instance: u16, device_id: u16, vendor_id: u16) -> Uuid {
        let (i, d, v) = (instance.to_be_bytes(), device_id.to_be_bytes(), vendor_id.to_be_bytes());
        Uuid::from_text_parts(0xDEA0_0000, 0x6C97, 0x11D1, [0x82, 0x71, i[0], i[1], d[0], d[1], v[0], v[1]])
    }

    /// The instance, device id and vendor id of a PNIO object UUID; `None` for any other UUID.
    pub fn pnio_object_parts(self) -> Option<(u16, u16, u16)> {
        let b = self.0;
        (b[..10] == Uuid::pnio_object(0, 0, 0).0[..10]).then(|| {
            (u16::from_be_bytes([b[10], b[11]]), u16::from_be_bytes([b[12], b[13]]), u16::from_be_bytes([b[14], b[15]]))
        })
    }

    /// The 16 bytes as they travel in this byte order: the first three fields in it, the rest as
    /// they are.
    pub fn to_wire(self, order: ByteOrder) -> [u8; 16] {
        let mut b = self.0;
        if order == ByteOrder::Little {
            b[0..4].reverse();
            b[4..6].reverse();
            b[6..8].reverse();
        }
        b
    }

    pub fn from_wire(bytes: [u8; 16], order: ByteOrder) -> Uuid {
        // The swap is its own inverse.
        Uuid(Uuid(bytes).to_wire(order))
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({self})")
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0:?} is not a UUID: 8, 4, 4, 4 and 12 hexadecimal digits joined by hyphens")]
pub struct UuidParseError(pub String);

impl FromStr for Uuid {
    type Err = UuidParseError;

    fn from_str(s: &str) -> Result<Uuid, UuidParseError> {
        let bad = || UuidParseError(s.to_string());
        let groups: Vec<&str> = s.split('-').collect();
        let lens: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        if lens != [8, 4, 4, 4, 12] || !groups.iter().all(|g| g.bytes().all(|c| c.is_ascii_hexdigit())) {
            return Err(bad());
        }
        let hex: String = groups.concat();
        let mut out = [0u8; 16];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
        Ok(Uuid(out))
    }
}

impl Serialize for Uuid {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Uuid, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The byte order of the header's integers and the arguments' counts, from `drep`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ByteOrder {
    Big,
    Little,
}

impl ByteOrder {
    /// `drep[0]`'s high nibble: 0 big-endian, 1 little-endian (only its low bit is read, as
    /// Wireshark reads it).
    pub fn of_drep(drep: [u8; 3]) -> ByteOrder {
        if drep[0] & 0x10 != 0 { ByteOrder::Little } else { ByteOrder::Big }
    }

    /// The data representation of this order, with ASCII characters and IEEE floats.
    pub fn drep(self) -> [u8; 3] {
        match self {
            ByteOrder::Big => [0x00, 0, 0],
            ByteOrder::Little => [0x10, 0, 0],
        }
    }

    fn u16(self, b: &[u8], at: usize) -> u16 {
        let v = [b[at], b[at + 1]];
        match self {
            ByteOrder::Big => u16::from_be_bytes(v),
            ByteOrder::Little => u16::from_le_bytes(v),
        }
    }

    fn u32(self, b: &[u8], at: usize) -> u32 {
        let v = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        match self {
            ByteOrder::Big => u32::from_be_bytes(v),
            ByteOrder::Little => u32::from_le_bytes(v),
        }
    }

    fn put16(self, out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&match self {
            ByteOrder::Big => v.to_be_bytes(),
            ByteOrder::Little => v.to_le_bytes(),
        });
    }

    fn put32(self, out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&match self {
            ByteOrder::Big => v.to_be_bytes(),
            ByteOrder::Little => v.to_le_bytes(),
        });
    }
}

/// The connectionless packet types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PacketType {
    Request,
    Ping,
    Response,
    Fault,
    Working,
    Nocall,
    Reject,
    Ack,
    Cancel,
    Fack,
    CancelAck,
    /// Any other value (11 to 255), carried as heard.
    Other(u8),
}

impl PacketType {
    pub fn of(b: u8) -> PacketType {
        match b {
            0 => PacketType::Request,
            1 => PacketType::Ping,
            2 => PacketType::Response,
            3 => PacketType::Fault,
            4 => PacketType::Working,
            5 => PacketType::Nocall,
            6 => PacketType::Reject,
            7 => PacketType::Ack,
            8 => PacketType::Cancel,
            9 => PacketType::Fack,
            10 => PacketType::CancelAck,
            b => PacketType::Other(b),
        }
    }

    pub fn byte(self) -> u8 {
        match self {
            PacketType::Request => 0,
            PacketType::Ping => 1,
            PacketType::Response => 2,
            PacketType::Fault => 3,
            PacketType::Working => 4,
            PacketType::Nocall => 5,
            PacketType::Reject => 6,
            PacketType::Ack => 7,
            PacketType::Cancel => 8,
            PacketType::Fack => 9,
            PacketType::CancelAck => 10,
            PacketType::Other(b) => b,
        }
    }
}

/// The first flags byte's bits.
pub mod flags1 {
    pub const LAST_FRAGMENT: u8 = 0x02;
    pub const FRAGMENT: u8 = 0x04;
    pub const NO_FACK: u8 = 0x08;
    pub const MAYBE: u8 = 0x10;
    pub const IDEMPOTENT: u8 = 0x20;
    pub const BROADCAST: u8 = 0x40;
}

/// The second flags byte's one named bit.
pub mod flags2 {
    pub const CANCEL_PENDING: u8 = 0x02;
}

/// PNIO's operations on the device interface.
pub mod opnum {
    pub const CONNECT: u16 = 0;
    pub const RELEASE: u16 = 1;
    pub const READ: u16 = 2;
    pub const WRITE: u16 = 3;
    pub const CONTROL: u16 = 4;
    pub const READ_IMPLICIT: u16 = 5;
}

/// One connectionless RPC datagram.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcPdu {
    pub packet_type: PacketType,
    pub flags1: u8,
    pub flags2: u8,
    pub drep: [u8; 3],
    /// The fragment's serial number (its high and low bytes travel apart).
    pub serial: u16,
    pub object: Uuid,
    pub interface: Uuid,
    pub activity: Uuid,
    pub server_boot: u32,
    pub interface_version: u32,
    pub sequence: u32,
    pub opnum: u16,
    pub interface_hint: u16,
    pub activity_hint: u16,
    pub fragment: u16,
    pub auth_proto: u8,
    pub body: Vec<u8>,
    /// Bytes of the datagram after the body, kept as heard.
    pub trailing: Vec<u8>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RpcError {
    #[error("a connectionless RPC header is {HEADER} bytes; this datagram is {0}")]
    TooShort(usize),
    #[error("RPC version {0} is not the connectionless protocol's (4)")]
    NotConnectionless(u8),
    #[error("the body is said to be {declared} bytes and {available} follow the header")]
    Overrun { declared: usize, available: usize },
    #[error("a datagram of {0} bytes is past what UDP over IPv4 carries ({MAX_DATAGRAM})")]
    TooLong(usize),
    #[error("PNIO's arguments are {want} bytes before their data; this body is {len}")]
    ArgsTooShort { want: usize, len: usize },
    #[error("the arguments' data is said to be {declared} bytes and {available} follow")]
    ArgsOverrun { declared: usize, available: usize },
    #[error("a block's body of {0} bytes is past what its length counts (65533)")]
    BlockTooLong(usize),
    #[error("{0}")]
    Unfit(String),
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn uuid_at(b: &[u8], at: usize) -> [u8; 16] {
    let mut u = [0u8; 16];
    u.copy_from_slice(&b[at..at + 16]);
    u
}

impl RpcPdu {
    pub fn byte_order(&self) -> ByteOrder {
        ByteOrder::of_drep(self.drep)
    }

    /// Read one datagram (the UDP payload). A header shorter than 80 bytes, a datagram past what
    /// UDP over IPv4 carries, a version other than 4, or a body past the datagram is an error
    /// naming it; bytes after the body are kept.
    pub fn decode(d: &[u8]) -> Result<RpcPdu, RpcError> {
        if d.len() < HEADER {
            return Err(RpcError::TooShort(d.len()));
        }
        if d.len() > MAX_DATAGRAM {
            return Err(RpcError::TooLong(d.len()));
        }
        if d[0] != 4 {
            return Err(RpcError::NotConnectionless(d[0]));
        }
        let drep = [d[4], d[5], d[6]];
        let o = ByteOrder::of_drep(drep);
        let declared = o.u16(d, 74) as usize;
        let available = d.len() - HEADER;
        if declared > available {
            return Err(RpcError::Overrun { declared, available });
        }
        Ok(RpcPdu {
            packet_type: PacketType::of(d[1]),
            flags1: d[2],
            flags2: d[3],
            drep,
            serial: u16::from_be_bytes([d[7], d[79]]),
            object: Uuid::from_wire(uuid_at(d, 8), o),
            interface: Uuid::from_wire(uuid_at(d, 24), o),
            activity: Uuid::from_wire(uuid_at(d, 40), o),
            server_boot: o.u32(d, 56),
            interface_version: o.u32(d, 60),
            sequence: o.u32(d, 64),
            opnum: o.u16(d, 68),
            interface_hint: o.u16(d, 70),
            activity_hint: o.u16(d, 72),
            fragment: o.u16(d, 76),
            auth_proto: d[78],
            body: d[HEADER..HEADER + declared].to_vec(),
            trailing: d[HEADER + declared..].to_vec(),
        })
    }

    /// The datagram: header, body, trailing bytes. A datagram past what UDP over IPv4 carries
    /// (65 507 bytes) is refused.
    pub fn encode(&self) -> Result<Vec<u8>, RpcError> {
        let total = HEADER + self.body.len() + self.trailing.len();
        if total > MAX_DATAGRAM {
            return Err(RpcError::TooLong(total));
        }
        let o = self.byte_order();
        let mut out = Vec::with_capacity(HEADER + self.body.len() + self.trailing.len());
        out.extend_from_slice(&[4, self.packet_type.byte(), self.flags1, self.flags2]);
        out.extend_from_slice(&self.drep);
        out.push((self.serial >> 8) as u8);
        out.extend_from_slice(&self.object.to_wire(o));
        out.extend_from_slice(&self.interface.to_wire(o));
        out.extend_from_slice(&self.activity.to_wire(o));
        o.put32(&mut out, self.server_boot);
        o.put32(&mut out, self.interface_version);
        o.put32(&mut out, self.sequence);
        o.put16(&mut out, self.opnum);
        o.put16(&mut out, self.interface_hint);
        o.put16(&mut out, self.activity_hint);
        o.put16(&mut out, self.body.len() as u16);
        o.put16(&mut out, self.fragment);
        out.push(self.auth_proto);
        out.push(self.serial as u8);
        out.extend_from_slice(&self.body);
        out.extend_from_slice(&self.trailing);
        Ok(out)
    }

    /// A PNIO request to a device: the device interface version 1, idempotent, no hints, fragment
    /// 0, serial 0, server boot 0, in the byte order given.
    pub fn pnio_request(order: ByteOrder, object: Uuid, activity: Uuid, sequence: u32, opnum: u16, args: &RequestArgs) -> RpcPdu {
        RpcPdu {
            packet_type: PacketType::Request,
            flags1: flags1::IDEMPOTENT,
            flags2: 0,
            drep: order.drep(),
            serial: 0,
            object,
            interface: Uuid::PNIO_DEVICE,
            activity,
            server_boot: 0,
            interface_version: 1,
            sequence,
            opnum,
            interface_hint: 0xFFFF,
            activity_hint: 0xFFFF,
            fragment: 0,
            auth_proto: 0,
            body: args.encode(order),
            trailing: Vec::new(),
        }
    }

    /// The device's answer to `request`: a response, last fragment, no fragment acknowledgement
    /// wanted, the request's object, interface, activity, sequence and operation, in the
    /// request's byte order.
    pub fn pnio_response(request: &RpcPdu, server_boot: u32, args: &ResponseArgs) -> RpcPdu {
        RpcPdu {
            packet_type: PacketType::Response,
            flags1: flags1::LAST_FRAGMENT | flags1::NO_FACK,
            flags2: 0,
            drep: request.drep,
            serial: 0,
            object: request.object,
            interface: request.interface,
            activity: request.activity,
            server_boot,
            interface_version: request.interface_version,
            sequence: request.sequence,
            opnum: request.opnum,
            interface_hint: 0xFFFF,
            activity_hint: 0xFFFF,
            fragment: 0,
            auth_proto: 0,
            body: args.encode(request.byte_order()),
            trailing: Vec::new(),
        }
    }
}

/// A PNIO request's arguments: the largest answer the caller takes, and the data (its blocks).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestArgs {
    pub args_maximum: u32,
    pub args_length: u32,
    pub max_count: u32,
    pub offset: u32,
    /// The data, as many bytes as its actual count says (the count is written as its length).
    pub data: Vec<u8>,
    /// Bytes of the body after the data, kept as heard.
    pub trailing: Vec<u8>,
}

/// A PNIO response's arguments: the status (a 32-bit value in the header's byte order), and the
/// data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseArgs {
    pub status: PnioStatus,
    pub args_length: u32,
    pub max_count: u32,
    pub offset: u32,
    /// The data, as many bytes as its actual count says (the count is written as its length).
    pub data: Vec<u8>,
    pub trailing: Vec<u8>,
}

const ARGS: usize = 20;

fn args_data(body: &[u8], o: ByteOrder) -> Result<(Vec<u8>, Vec<u8>), RpcError> {
    if body.len() < ARGS {
        return Err(RpcError::ArgsTooShort { want: ARGS, len: body.len() });
    }
    let declared = o.u32(body, 16) as usize;
    let available = body.len() - ARGS;
    if declared > available {
        return Err(RpcError::ArgsOverrun { declared, available });
    }
    Ok((body[ARGS..ARGS + declared].to_vec(), body[ARGS + declared..].to_vec()))
}

impl RequestArgs {
    /// The arguments of `data`, as a controller writes them: its length counted twice, the
    /// largest answer as the maximum count, offset 0.
    pub fn new(args_maximum: u32, data: Vec<u8>) -> RequestArgs {
        let n = data.len() as u32;
        RequestArgs { args_maximum, args_length: n, max_count: args_maximum, offset: 0, data, trailing: Vec::new() }
    }

    /// Read a request's body in the header's byte order.
    pub fn decode(body: &[u8], order: ByteOrder) -> Result<RequestArgs, RpcError> {
        let (data, trailing) = args_data(body, order)?;
        Ok(RequestArgs {
            args_maximum: order.u32(body, 0),
            args_length: order.u32(body, 4),
            max_count: order.u32(body, 8),
            offset: order.u32(body, 12),
            data,
            trailing,
        })
    }

    /// The body; the actual count is the data's length.
    pub fn encode(&self, order: ByteOrder) -> Vec<u8> {
        let mut out = Vec::with_capacity(ARGS + self.data.len() + self.trailing.len());
        for v in [self.args_maximum, self.args_length, self.max_count, self.offset, self.data.len() as u32] {
            order.put32(&mut out, v);
        }
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&self.trailing);
        out
    }
}

impl ResponseArgs {
    /// The arguments of `data` with `status`, as a device writes them: its length counted twice,
    /// the request's largest answer as the maximum count, offset 0.
    pub fn new(status: PnioStatus, max_count: u32, data: Vec<u8>) -> ResponseArgs {
        let n = data.len() as u32;
        ResponseArgs { status, args_length: n, max_count, offset: 0, data, trailing: Vec::new() }
    }

    pub fn decode(body: &[u8], order: ByteOrder) -> Result<ResponseArgs, RpcError> {
        let (data, trailing) = args_data(body, order)?;
        Ok(ResponseArgs {
            status: PnioStatus::from_bytes(order.u32(body, 0).to_be_bytes()),
            args_length: order.u32(body, 4),
            max_count: order.u32(body, 8),
            offset: order.u32(body, 12),
            data,
            trailing,
        })
    }

    /// The body; the actual count is the data's length.
    pub fn encode(&self, order: ByteOrder) -> Vec<u8> {
        let mut out = Vec::with_capacity(ARGS + self.data.len() + self.trailing.len());
        order.put32(&mut out, u32::from_be_bytes(self.status.to_bytes()));
        for v in [self.args_length, self.max_count, self.offset, self.data.len() as u32] {
            order.put32(&mut out, v);
        }
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&self.trailing);
        out
    }
}

/// One PNIO block: its type, version and body (what follows the version; its length is the
/// body's plus the two version bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub block_type: u16,
    pub version: (u8, u8),
    pub body: Vec<u8>,
}

/// The blocks a data holds, in order, and the bytes from the first that is not a whole block (a
/// header cut short, a length past the data, a length under 2), kept.
pub fn read_blocks(data: &[u8]) -> (Vec<Block>, Vec<u8>) {
    let mut blocks = Vec::new();
    let mut at = 0;
    while data.len() - at >= 6 {
        let len = be16(data, at + 2) as usize;
        if len < 2 || at + 4 + len > data.len() {
            break;
        }
        blocks.push(Block { block_type: be16(data, at), version: (data[at + 4], data[at + 5]), body: data[at + 6..at + 4 + len].to_vec() });
        at += 4 + len;
    }
    (blocks, data[at..].to_vec())
}

impl Block {
    /// A block of version 1.0.
    pub fn new(block_type: u16, body: Vec<u8>) -> Block {
        Block { block_type, version: (1, 0), body }
    }

    pub fn encode(&self) -> Result<Vec<u8>, RpcError> {
        if self.body.len() > u16::MAX as usize - 2 {
            return Err(RpcError::BlockTooLong(self.body.len()));
        }
        let mut out = Vec::with_capacity(6 + self.body.len());
        out.extend_from_slice(&self.block_type.to_be_bytes());
        out.extend_from_slice(&(self.body.len() as u16 + 2).to_be_bytes());
        out.extend_from_slice(&[self.version.0, self.version.1]);
        out.extend_from_slice(&self.body);
        Ok(out)
    }
}

/// The block types of the record read.
pub const READ_REQUEST: u16 = 0x0009;
pub const READ_RESPONSE: u16 = 0x8009;

/// What a read asks for: which record of which subslot, and at most how many bytes. An implicit
/// read (no connection) carries the nil AR UUID and may name a target AR; a read under a connection
/// carries its AR's UUID, and the target AR's place is then padding (written zero, read only when
/// zero, so `target_ar` is nil).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    pub seq: u16,
    pub ar: Uuid,
    pub api: u32,
    pub slot: u16,
    pub subslot: u16,
    pub index: u16,
    /// The most record data the caller takes.
    pub length: u32,
    pub target_ar: Uuid,
}

/// The read's answer header; the record data follows it, `length` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResponse {
    pub seq: u16,
    pub ar: Uuid,
    pub api: u32,
    pub slot: u16,
    pub subslot: u16,
    pub index: u16,
    /// The record data's length.
    pub length: u32,
    pub add1: u16,
    pub add2: u16,
}

/// The read headers' body length: 58 bytes after the version.
const READ_BODY: usize = 58;

fn unfit(what: &str, b: &Block) -> RpcError {
    RpcError::Unfit(format!(
        "{what} is block {:#06x} version 1.0 of 60 bytes with zero padding; this block is {:#06x} version {}.{} of {} bytes",
        if what == "a read request" { READ_REQUEST } else { READ_RESPONSE },
        b.block_type,
        b.version.0,
        b.version.1,
        b.body.len() + 2
    ))
}

impl ReadRequest {
    /// The record read with no connection, of `index` at a subslot of API 0.
    pub fn implicit(seq: u16, slot: u16, subslot: u16, index: u16, length: u32) -> ReadRequest {
        ReadRequest { seq, ar: Uuid::NIL, api: 0, slot, subslot, index, length, target_ar: Uuid::NIL }
    }

    /// The block. A target AR under an AR that is not nil (where its place is padding) is refused.
    pub fn block(&self) -> Result<Block, RpcError> {
        if self.ar != Uuid::NIL && self.target_ar != Uuid::NIL {
            return Err(RpcError::Unfit(format!("a target AR travels only under the nil AR (an implicit read); this read's AR is {}", self.ar)));
        }
        let mut b = Vec::with_capacity(READ_BODY);
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(&self.ar.0);
        b.extend_from_slice(&self.api.to_be_bytes());
        b.extend_from_slice(&self.slot.to_be_bytes());
        b.extend_from_slice(&self.subslot.to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&self.index.to_be_bytes());
        b.extend_from_slice(&self.length.to_be_bytes());
        b.extend_from_slice(&self.target_ar.0);
        b.extend_from_slice(&[0; 8]);
        Ok(Block::new(READ_REQUEST, b))
    }

    /// The read request a block holds: type 0x0009, version 1.0, 60 bytes, its padding zero (the
    /// target AR's place among it under an AR that is not nil).
    pub fn from_block(b: &Block) -> Result<ReadRequest, RpcError> {
        let x = &b.body;
        let explicit = x.len() == READ_BODY && x[2..18] != [0; 16];
        if b.block_type != READ_REQUEST
            || b.version != (1, 0)
            || x.len() != READ_BODY
            || x[26..28] != [0, 0]
            || x[50..58] != [0; 8]
            || (explicit && x[34..50] != [0; 16])
        {
            return Err(unfit("a read request", b));
        }
        Ok(ReadRequest {
            seq: be16(x, 0),
            ar: Uuid(uuid_at(x, 2)),
            api: be32(x, 18),
            slot: be16(x, 22),
            subslot: be16(x, 24),
            index: be16(x, 28),
            length: be32(x, 30),
            target_ar: Uuid(uuid_at(x, 34)),
        })
    }
}

impl ReadResponse {
    pub fn block(&self) -> Block {
        let mut b = Vec::with_capacity(READ_BODY);
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(&self.ar.0);
        b.extend_from_slice(&self.api.to_be_bytes());
        b.extend_from_slice(&self.slot.to_be_bytes());
        b.extend_from_slice(&self.subslot.to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&self.index.to_be_bytes());
        b.extend_from_slice(&self.length.to_be_bytes());
        b.extend_from_slice(&self.add1.to_be_bytes());
        b.extend_from_slice(&self.add2.to_be_bytes());
        b.extend_from_slice(&[0; 20]);
        Block::new(READ_RESPONSE, b)
    }

    /// The read response a block holds: type 0x8009, version 1.0, 60 bytes, its padding zero.
    pub fn from_block(b: &Block) -> Result<ReadResponse, RpcError> {
        let x = &b.body;
        if b.block_type != READ_RESPONSE || b.version != (1, 0) || x.len() != READ_BODY || x[26..28] != [0, 0] || x[38..58] != [0; 20] {
            return Err(unfit("a read response", b));
        }
        Ok(ReadResponse {
            seq: be16(x, 0),
            ar: Uuid(uuid_at(x, 2)),
            api: be32(x, 18),
            slot: be16(x, 22),
            subslot: be16(x, 24),
            index: be16(x, 28),
            length: be32(x, 30),
            add1: be16(x, 34),
            add2: be16(x, 36),
        })
    }

    /// The answer to `request` carrying `record` (the caller cuts it to the request's length).
    pub fn answering(request: &ReadRequest, record_len: u32) -> ReadResponse {
        ReadResponse {
            seq: request.seq,
            ar: request.ar,
            api: request.api,
            slot: request.slot,
            subslot: request.subslot,
            index: request.index,
            length: record_len,
            add1: 0,
            add2: 0,
        }
    }

    /// A read answer's data: its header, then the record. The header is the first block; the
    /// record is what follows it, as bytes (a record is often a block of its own, sometimes not).
    pub fn split(data: &[u8]) -> Result<(ReadResponse, Vec<u8>), RpcError> {
        const HEAD: usize = 4 + 2 + READ_BODY;
        if data.len() < HEAD {
            return Err(RpcError::Unfit(format!("a read answer's data begins with its 64-byte header; this one is {} bytes", data.len())));
        }
        let declared = be16(data, 2);
        if usize::from(declared) != 2 + READ_BODY {
            return Err(RpcError::Unfit(format!(
                "a read answer's data begins with its header, a block of length 60; this one begins with block {:#06x} of length {declared}",
                be16(data, 0)
            )));
        }
        let first = Block { block_type: be16(data, 0), version: (data[4], data[5]), body: data[6..HEAD].to_vec() };
        let header = ReadResponse::from_block(&first)?;
        Ok((header, data[HEAD..].to_vec()))
    }
}

/// The datagram of an implicit read (operation 5) of `request`, in the byte order given;
/// `args_maximum` is the largest answer taken, the request's record length what is left of it
/// after the answer's header. A request under an AR that is not nil is refused: that is a read
/// under a connection (operation 2).
pub fn read_implicit(order: ByteOrder, object: Uuid, activity: Uuid, sequence: u32, request: &ReadRequest, args_maximum: u32) -> Result<RpcPdu, RpcError> {
    if request.ar != Uuid::NIL {
        return Err(RpcError::Unfit(format!("an implicit read carries the nil AR; this one carries {}", request.ar)));
    }
    let data = request.block()?.encode()?;
    Ok(RpcPdu::pnio_request(order, object, activity, sequence, opnum::READ_IMPLICIT, &RequestArgs::new(args_maximum, data)))
}

/// The device's answer to a read: the response header for `record`, then `record`, with `status`.
pub fn read_answer(request: &RpcPdu, read: &ReadRequest, status: PnioStatus, record: &[u8], server_boot: u32) -> Result<RpcPdu, RpcError> {
    let mut data = ReadResponse::answering(read, record.len() as u32).block().encode()?;
    data.extend_from_slice(record);
    let max = RequestArgs::decode(&request.body, request.byte_order()).map(|a| a.args_maximum).unwrap_or(data.len() as u32);
    Ok(RpcPdu::pnio_response(request, server_boot, &ResponseArgs::new(status, max, data)))
}
