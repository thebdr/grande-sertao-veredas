//! EtherNet/IP's wire, as the adapter reads and writes it: the encapsulation header, the common
//! packet format's items, a CIP request's path, the Forward_Open and Forward_Close bodies, and the
//! Class 1 frame. Every multi-byte field is little-endian, as the protocol has it, except the
//! socket address items, which carry the network's byte order.

/// The encapsulation header's length.
pub const HEADER: usize = 24;

/// Encapsulation commands.
pub mod command {
    pub const NOP: u16 = 0x0000;
    pub const LIST_SERVICES: u16 = 0x0004;
    pub const LIST_IDENTITY: u16 = 0x0063;
    pub const LIST_INTERFACES: u16 = 0x0064;
    pub const REGISTER_SESSION: u16 = 0x0065;
    pub const UNREGISTER_SESSION: u16 = 0x0066;
    pub const SEND_RR_DATA: u16 = 0x006F;
    pub const SEND_UNIT_DATA: u16 = 0x0070;
}

/// Encapsulation status codes.
pub mod encap_status {
    pub const SUCCESS: u32 = 0x0000;
    pub const INVALID_COMMAND: u32 = 0x0001;
    pub const INSUFFICIENT_MEMORY: u32 = 0x0002;
    pub const INCORRECT_DATA: u32 = 0x0003;
    pub const INVALID_SESSION: u32 = 0x0064;
    pub const INVALID_LENGTH: u32 = 0x0065;
    pub const UNSUPPORTED_PROTOCOL: u32 = 0x0069;
}

/// Common packet format item types.
pub mod item {
    pub const NULL_ADDRESS: u16 = 0x0000;
    pub const LIST_IDENTITY: u16 = 0x000C;
    pub const CONNECTED_ADDRESS: u16 = 0x00A1;
    pub const CONNECTED_DATA: u16 = 0x00B1;
    pub const UNCONNECTED_DATA: u16 = 0x00B2;
    pub const LIST_SERVICES: u16 = 0x0100;
    pub const SOCKADDR_O_TO_T: u16 = 0x8000;
    pub const SOCKADDR_T_TO_O: u16 = 0x8001;
    pub const SEQUENCED_ADDRESS: u16 = 0x8002;
}

/// CIP general status codes.
pub mod general {
    pub const SUCCESS: u8 = 0x00;
    pub const CONNECTION_FAILURE: u8 = 0x01;
    pub const RESOURCE_UNAVAILABLE: u8 = 0x02;
    pub const PATH_SEGMENT_ERROR: u8 = 0x04;
    pub const PATH_DESTINATION_UNKNOWN: u8 = 0x05;
    pub const SERVICE_NOT_SUPPORTED: u8 = 0x08;
    pub const INVALID_ATTRIBUTE_VALUE: u8 = 0x09;
    pub const OBJECT_STATE_CONFLICT: u8 = 0x0C;
    pub const ATTRIBUTE_NOT_SETTABLE: u8 = 0x0E;
    pub const NOT_ENOUGH_DATA: u8 = 0x13;
    pub const ATTRIBUTE_NOT_SUPPORTED: u8 = 0x14;
    pub const TOO_MUCH_DATA: u8 = 0x15;
    pub const OBJECT_DOES_NOT_EXIST: u8 = 0x16;
    pub const INVALID_PARAMETER: u8 = 0x20;
}

/// Connection Manager extended status codes (with general status 01).
pub mod extended {
    pub const CONNECTION_IN_USE: u16 = 0x0100;
    pub const TRANSPORT_NOT_SUPPORTED: u16 = 0x0103;
    pub const OWNERSHIP_CONFLICT: u16 = 0x0106;
    pub const CONNECTION_NOT_FOUND: u16 = 0x0107;
    pub const INVALID_CONNECTION_TYPE: u16 = 0x0108;
    pub const INVALID_CONNECTION_SIZE: u16 = 0x0109;
    pub const RPI_NOT_SUPPORTED: u16 = 0x0111;
    pub const NO_MORE_CONNECTIONS: u16 = 0x0113;
    pub const VENDOR_OR_PRODUCT_MISMATCH: u16 = 0x0114;
    pub const DEVICE_TYPE_MISMATCH: u16 = 0x0115;
    pub const REVISION_MISMATCH: u16 = 0x0116;
    pub const INVALID_CONFIGURATION_SIZE: u16 = 0x0126;
    pub const INVALID_O_TO_T_SIZE: u16 = 0x0127;
    pub const INVALID_T_TO_O_SIZE: u16 = 0x0128;
    pub const INVALID_CONFIGURATION_PATH: u16 = 0x0129;
    pub const INVALID_CONSUMING_PATH: u16 = 0x012A;
    pub const INVALID_PRODUCING_PATH: u16 = 0x012B;
    pub const INVALID_SEGMENT_IN_PATH: u16 = 0x0315;
}

/// CIP services the adapter answers.
pub mod service {
    pub const GET_ATTRIBUTES_ALL: u8 = 0x01;
    pub const RESET: u8 = 0x05;
    pub const GET_ATTRIBUTE_SINGLE: u8 = 0x0E;
    pub const SET_ATTRIBUTE_SINGLE: u8 = 0x10;
    pub const FORWARD_CLOSE: u8 = 0x4E;
    pub const UNCONNECTED_SEND: u8 = 0x52;
    pub const FORWARD_OPEN: u8 = 0x54;
    pub const LARGE_FORWARD_OPEN: u8 = 0x5B;
}

/// CIP classes the adapter holds.
pub mod class {
    pub const IDENTITY: u16 = 0x01;
    pub const MESSAGE_ROUTER: u16 = 0x02;
    pub const ASSEMBLY: u16 = 0x04;
    pub const CONNECTION_MANAGER: u16 = 0x06;
}

/// An encapsulation header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub command: u16,
    pub length: u16,
    pub session: u32,
    pub status: u32,
    pub context: [u8; 8],
    pub options: u32,
}

impl Header {
    pub fn parse(b: &[u8; HEADER]) -> Header {
        Header {
            command: le16(b, 0),
            length: le16(b, 2),
            session: le32(b, 4),
            status: le32(b, 8),
            context: b[12..20].try_into().unwrap_or([0; 8]),
            options: le32(b, 20),
        }
    }

    /// The reply to this header: its command, session and context, the given status and body.
    pub fn reply(&self, session: u32, status: u32, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + body.len());
        out.extend_from_slice(&self.command.to_le_bytes());
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&session.to_le_bytes());
        out.extend_from_slice(&status.to_le_bytes());
        out.extend_from_slice(&self.context);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(body);
        out
    }
}

pub fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// One item of the common packet format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: u16,
    pub data: Vec<u8>,
}

/// The items of a common packet format, from its item count on; `None` when the count or a length
/// runs past the bytes.
pub fn parse_items(b: &[u8]) -> Option<Vec<Item>> {
    if b.len() < 2 {
        return None;
    }
    let count = le16(b, 0) as usize;
    let mut at = 2;
    let mut items = Vec::with_capacity(count.min(8));
    for _ in 0..count {
        if at + 4 > b.len() {
            return None;
        }
        let kind = le16(b, at);
        let len = le16(b, at + 2) as usize;
        at += 4;
        if at + len > b.len() {
            return None;
        }
        items.push(Item { kind, data: b[at..at + len].to_vec() });
        at += len;
    }
    if at != b.len() {
        return None;
    }
    Some(items)
}

/// Items written out, the count first.
pub fn write_items(items: &[Item]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for i in items {
        out.extend_from_slice(&i.kind.to_le_bytes());
        out.extend_from_slice(&(i.data.len() as u16).to_le_bytes());
        out.extend_from_slice(&i.data);
    }
    out
}

/// A socket address item's body: family, port and address in the network's byte order, eight
/// zeros.
pub fn sockaddr(addr: std::net::SocketAddrV4) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&addr.port().to_be_bytes());
    out.extend_from_slice(&addr.ip().octets());
    out.extend_from_slice(&[0; 8]);
    out
}

/// A socket address item read back; `None` when it is not sixteen bytes of family 2.
pub fn read_sockaddr(b: &[u8]) -> Option<std::net::SocketAddrV4> {
    if b.len() != 16 || u16::from_be_bytes([b[0], b[1]]) != 2 {
        return None;
    }
    let port = u16::from_be_bytes([b[2], b[3]]);
    Some(std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(b[4], b[5], b[6], b[7]), port))
}

/// What a request's logical path names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Path {
    pub class: Option<u16>,
    pub instance: Option<u16>,
    pub attribute: Option<u16>,
    /// Connection points, in order (Forward_Open: the consumed one first, then the produced).
    pub points: Vec<u16>,
    /// An electronic key segment: vendor, device type, product code, major revision (its top bit
    /// the compatibility bit), minor revision.
    pub key: Option<(u16, u16, u16, u8, u8)>,
    /// A simple data segment's bytes (configuration data).
    pub data: Option<Vec<u8>>,
    /// Port segments (a route), as `(port, link)`.
    pub ports: Vec<(u16, Vec<u8>)>,
    /// Where the first segment that is not logical (a key, data or a port) begins, in bytes: what
    /// a request's own path may not hold.
    pub first_other: Option<usize>,
}

/// Why a path could not be read: the byte offset of the segment that is not understood.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadSegment(pub usize);

/// Read a padded EPATH: logical class, instance, attribute and connection point segments (8 and 16
/// bit), an electronic key, a simple data segment and port segments; anything else is a bad
/// segment at its offset.
pub fn parse_path(b: &[u8]) -> Result<Path, BadSegment> {
    let mut p = Path::default();
    let mut at = 0;
    while at < b.len() {
        let seg = b[at];
        let need = |n: usize| if at + n <= b.len() { Ok(()) } else { Err(BadSegment(at)) };
        match seg {
            // Logical segments: type in bits 4..2, format in bits 1..0 (0: 8 bit, 1: 16 bit, padded).
            0x20 | 0x24 | 0x2C | 0x30 => {
                need(2)?;
                let v = b[at + 1] as u16;
                set_logical(&mut p, seg & 0x1C, v);
                at += 2;
            }
            0x21 | 0x25 | 0x2D | 0x31 => {
                need(4)?;
                let v = le16(b, at + 2);
                set_logical(&mut p, seg & 0x1C, v);
                at += 4;
            }
            // The electronic key: format 4, ten bytes in all.
            0x34 => {
                need(10)?;
                p.first_other.get_or_insert(at);
                if b[at + 1] != 0x04 {
                    return Err(BadSegment(at));
                }
                p.key = Some((le16(b, at + 2), le16(b, at + 4), le16(b, at + 6), b[at + 8], b[at + 9]));
                at += 10;
            }
            // A simple data segment: its length in words, then the words.
            0x80 => {
                need(2)?;
                p.first_other.get_or_insert(at);
                let words = b[at + 1] as usize;
                need(2 + 2 * words)?;
                p.data = Some(b[at + 2..at + 2 + 2 * words].to_vec());
                at += 2 + 2 * words;
            }
            // A port segment, its link address one byte, padded to a word.
            s if s & 0xE0 == 0x00 && s & 0x10 == 0 && s & 0x0F != 0x0F => {
                need(2)?;
                p.first_other.get_or_insert(at);
                p.ports.push(((s & 0x0F) as u16, vec![b[at + 1]]));
                at += 2;
            }
            // A port segment with an extended link address: its size, the address, a pad byte.
            s if s & 0xE0 == 0x00 && s & 0x10 != 0 && s & 0x0F != 0x0F => {
                need(2)?;
                p.first_other.get_or_insert(at);
                let size = b[at + 1] as usize;
                need(2 + size)?;
                p.ports.push(((s & 0x0F) as u16, b[at + 2..at + 2 + size].to_vec()));
                at += 2 + size + (size % 2);
            }
            _ => return Err(BadSegment(at)),
        }
    }
    Ok(p)
}

fn set_logical(p: &mut Path, kind: u8, v: u16) {
    match kind {
        0x00 => p.class = Some(v),
        0x04 => p.instance = Some(v),
        0x0C => p.points.push(v),
        _ => p.attribute = Some(v),
    }
}

/// A CIP request: its service, its path's bytes, and its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub service: u8,
    pub path: Vec<u8>,
    pub data: Vec<u8>,
}

/// A request read from a message router frame; `None` when the path's size runs past it.
pub fn parse_request(b: &[u8]) -> Option<Request> {
    if b.len() < 2 {
        return None;
    }
    let words = b[1] as usize;
    if 2 + 2 * words > b.len() {
        return None;
    }
    Some(Request { service: b[0], path: b[2..2 + 2 * words].to_vec(), data: b[2 + 2 * words..].to_vec() })
}

/// A reply frame: the service with its reply bit, a reserved byte, the general status, the
/// additional status words, the data.
pub fn reply(service: u8, general: u8, additional: &[u16], data: &[u8]) -> Vec<u8> {
    let mut out = vec![service | 0x80, 0, general, additional.len() as u8];
    for w in additional {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

/// A network connection parameters word (16 bit, Forward_Open) or double word (32 bit,
/// Large_Forward_Open), read into its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetParams {
    pub size: u16,
    pub variable: bool,
    pub priority: u8,
    /// 0 null, 1 multicast, 2 point to point.
    pub kind: u8,
    pub redundant_owner: bool,
}

impl NetParams {
    pub fn from_word(w: u16) -> NetParams {
        NetParams {
            size: w & 0x01FF,
            variable: w & 0x0200 != 0,
            priority: ((w >> 10) & 0x03) as u8,
            kind: ((w >> 13) & 0x03) as u8,
            redundant_owner: w & 0x8000 != 0,
        }
    }

    pub fn from_dword(d: u32) -> NetParams {
        NetParams {
            size: (d & 0xFFFF) as u16,
            variable: d & 0x0200_0000 != 0,
            priority: ((d >> 26) & 0x03) as u8,
            kind: ((d >> 29) & 0x03) as u8,
            redundant_owner: d & 0x8000_0000 != 0,
        }
    }
}

/// A Forward_Open (or Large_Forward_Open) request's body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardOpen {
    pub priority_tick: u8,
    pub timeout_ticks: u8,
    pub o_to_t_id: u32,
    pub t_to_o_id: u32,
    pub serial: u16,
    pub vendor: u16,
    pub originator_serial: u32,
    pub timeout_multiplier: u8,
    pub o_to_t_rpi: u32,
    pub o_to_t: NetParams,
    pub t_to_o_rpi: u32,
    pub t_to_o: NetParams,
    pub transport: u8,
    pub path: Vec<u8>,
}

/// Read a Forward_Open's body (`large` for a Large_Forward_Open's 32-bit parameters); `None` when it
/// is short or its path's size does not match the bytes left.
pub fn parse_forward_open(b: &[u8], large: bool) -> Option<ForwardOpen> {
    let fixed = if large { 40 } else { 36 };
    if b.len() < fixed {
        return None;
    }
    let (o_to_t, t_to_o_rpi_at, t_to_o, after) = if large {
        // RPI 22..26, parameters 26..30, RPI 30..34, parameters 34..38, transport 38, path size 39.
        (NetParams::from_dword(le32(b, 26)), 30, NetParams::from_dword(le32(b, 34)), 38)
    } else {
        // RPI 22..26, parameters 26..28, RPI 28..32, parameters 32..34, transport 34, path size 35.
        (NetParams::from_word(le16(b, 26)), 28, NetParams::from_word(le16(b, 32)), 34)
    };
    if b.len() < after + 2 {
        return None;
    }
    let transport = b[after];
    let words = b[after + 1] as usize;
    if b.len() != after + 2 + 2 * words {
        return None;
    }
    Some(ForwardOpen {
        priority_tick: b[0],
        timeout_ticks: b[1],
        o_to_t_id: le32(b, 2),
        t_to_o_id: le32(b, 6),
        serial: le16(b, 10),
        vendor: le16(b, 12),
        originator_serial: le32(b, 14),
        timeout_multiplier: b[18],
        o_to_t_rpi: le32(b, 22),
        o_to_t,
        t_to_o_rpi: le32(b, t_to_o_rpi_at),
        t_to_o,
        transport,
        path: b[after + 2..].to_vec(),
    })
}

/// A Forward_Close request's body: who closes, and the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardClose {
    pub serial: u16,
    pub vendor: u16,
    pub originator_serial: u32,
    pub path: Vec<u8>,
}

/// Read a Forward_Close's body; `None` when it is short or its path's size is wrong.
pub fn parse_forward_close(b: &[u8]) -> Option<ForwardClose> {
    if b.len() < 12 {
        return None;
    }
    let words = b[10] as usize;
    if b.len() != 12 + 2 * words {
        return None;
    }
    Some(ForwardClose { serial: le16(b, 2), vendor: le16(b, 4), originator_serial: le32(b, 6), path: b[12..].to_vec() })
}

/// A Class 1 frame: the sequenced address item (the connection, the frame's sequence), then the
/// connected data item (the 16-bit sequence count, the run/idle header when the connection carries
/// one, the data).
pub fn class1_frame(connection: u32, sequence: u32, count: u16, run: Option<bool>, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(6 + data.len());
    payload.extend_from_slice(&count.to_le_bytes());
    if let Some(run) = run {
        payload.extend_from_slice(&(run as u32).to_le_bytes());
    }
    payload.extend_from_slice(data);
    let mut address = Vec::with_capacity(8);
    address.extend_from_slice(&connection.to_le_bytes());
    address.extend_from_slice(&sequence.to_le_bytes());
    write_items(&[Item { kind: item::SEQUENCED_ADDRESS, data: address }, Item { kind: item::CONNECTED_DATA, data: payload }])
}

/// A Class 1 frame read: the connection, the sequence, the payload of its connected data item.
pub fn read_class1(b: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let items = parse_items(b)?;
    let address = items.iter().find(|i| i.kind == item::SEQUENCED_ADDRESS && i.data.len() == 8)?;
    let data = items.iter().find(|i| i.kind == item::CONNECTED_DATA)?;
    Some((le32(&address.data, 0), le32(&address.data, 4), data.data.clone()))
}
