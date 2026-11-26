//! The implicit read (`docs/aranha-plan.md`, "Scope", deep identity: the PROFINET implicit read,
//! never an application relation; phase 6: the port's neighbour data, where device-to-device links
//! come from; "Rules": it listens before it asks, every active step bounded and paced). A record of
//! one device is read with no connection: a connectionless DCE/RPC request (PNIO's operation 5)
//! in one UDP datagram to the device's port 34964, sent through a listener already hearing the
//! interface the device was heard on, from that interface's MAC and its IPv4 address on the
//! device's network (by the device's own mask: a device answers without a router only a source on
//! it), paced by its worker like any frame it sends. Its answer is the datagram that listener then
//! keeps from that device's MAC and address to the request's, under the request's activity: the
//! frame the record came from is on record, as every value's is. The interface's own address takes
//! in the answer (an open socket on the request's port, so this machine answers the device's ARP and
//! refuses nothing it sends back).
//!
//! The port data (index 0x802A) of a device's ports, subslot 0x8001 of slot 0 and on until the
//! device says a subslot is not there, gives the device's own word on each port's neighbours: the
//! reports the topology draws reported and confirmed links from (C-145), each port named as its
//! record names it.

use std::net::{Ipv4Addr, UdpSocket};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::fusion::Device;
use crate::ip::{self, Endpoint, IpError};
use crate::listen::{ListenError, Listener, Outcome};
use crate::profinet::mac::Mac;
use crate::profinet::record::{INDEX_PORT_DATA_REAL, PortDataReal};
use crate::profinet::rpc::{self, ByteOrder, PNIO_PORT, PacketType, PnioStatus, ReadRequest, ReadResponse, ResponseArgs, RpcError, RpcPdu, Uuid};
use crate::topology::Neighbour;

/// The longest an answer is waited for.
pub const MAX_WAIT: Duration = Duration::from_secs(10);
/// The most record data a read takes: what one frame's datagram holds past RPC's and the read's
/// headers.
pub const MAX_RECORD: u32 = (ip::MAX_UDP_PAYLOAD - rpc::HEADER - 20 - 64) as u32;
/// The subslot before a device's first port's (its interface's).
pub const SUBSLOT_PORTS: u16 = 0x8000;

/// How a read waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// How long past the request's going out its answer is waited for.
    pub wait: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options { wait: Duration::from_secs(1) }
    }
}

/// What a read asks of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub slot: u16,
    pub subslot: u16,
    pub index: u16,
}

/// A record read, and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub iface: String,
    pub device: Mac,
    pub address: Ipv4Addr,
    pub target: Target,
    /// The request as the listener kept it, and when it went.
    pub request: u64,
    pub sent_ns: u64,
    /// The answer as the listener kept it, and when it came.
    pub answer: u64,
    pub at_ns: u64,
    pub header: ReadResponse,
    /// The record's data, as long as the answer's header says.
    pub data: Vec<u8>,
}

/// One port's data, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRead {
    pub record: Record,
    pub data: PortDataReal,
}

/// A device's ports read: each one's data, and the neighbours they report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ports {
    /// By subslot, from 0x8001.
    pub read: Vec<PortRead>,
    /// One per peer of a port read, its port as the record names it.
    pub neighbours: Vec<Neighbour>,
    /// The answer (its kept frame) saying the next subslot is not there; none when `MAX_PORTS`
    /// were read and the device may hold more.
    pub end: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Listen(#[from] ListenError),
    #[error("{0:?} has no MAC of its own (a capture replayed): nothing is read from it")]
    NoMac(String),
    #[error("{iface:?} is heard no more ({reason}): nothing is read on it")]
    Ended { iface: String, reason: String },
    #[error("{0:?} has no IPv4 address of its own: a device could not answer it")]
    NoAddress(String),
    #[error("{iface:?} holds no IPv4 address on the network of {device} (mask {mask}): the device could not answer it")]
    OffNetwork { iface: String, device: Ipv4Addr, mask: Ipv4Addr },
    #[error("{device} was not heard on {iface:?}: it is not read there")]
    NotHeard { device: Mac, iface: String },
    #[error("{0} states no IPv4 address of its own (DCP's IP suite): there is nothing to read it at")]
    Unaddressed(Mac),
    #[error("a wait of {0:?} is past the {MAX_WAIT:?} a read's answer is waited for")]
    Wait(Duration),
    #[error("the read cannot be written: {0}")]
    Request(String),
    #[error("the answer's port cannot be opened on {address}: {source}")]
    Socket { address: Ipv4Addr, source: std::io::Error },
    #[error("{iface:?}'s wire refused the read: {reason}")]
    Refused { iface: String, reason: String },
    #[error("{iface:?} did not send the read within its turn, {waited:?}: it is withdrawn and never sent")]
    NotSent { iface: String, waited: Duration },
    #[error("{iface:?}'s wire was still taking the read past its turn, {waited:?}: it may yet go, and no answer to it is this read's")]
    Stalled { iface: String, waited: Duration },
    #[error("{device} did not answer the read within {waited:?} of its going")]
    NoAnswer { device: Mac, waited: Duration },
    #[error("frames after the read were let go past the listener's bound before they were read ({0}): its answer may be among them")]
    Lost(u64),
    #[error("{device} refused the read (frame {frame}): {kind:?}")]
    Rejected { device: Mac, frame: u64, kind: PacketType },
    #[error("{device} answered the read with PNIO status {status:?} (frame {frame})")]
    Status { device: Mac, frame: u64, status: PnioStatus },
    #[error("{device}'s answer (frame {frame}) is not the record asked: {why}")]
    Answer { device: Mac, frame: u64, why: String },
}

impl From<RpcError> for ReadError {
    fn from(e: RpcError) -> ReadError {
        ReadError::Request(e.to_string())
    }
}

impl From<IpError> for ReadError {
    fn from(e: IpError) -> ReadError {
        ReadError::Request(e.to_string())
    }
}

/// An activity this process has not used: a count from a start drawn from the clock and the
/// process, so two scanners on one segment do not share one.
fn next_activity() -> Uuid {
    static BASE: OnceLock<u64> = OnceLock::new();
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let base = *BASE.get_or_init(|| {
        let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        d.as_nanos() as u64 ^ u64::from(std::process::id()).rotate_left(32)
    });
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&base.to_be_bytes());
    b[8..12].copy_from_slice(&n.to_be_bytes());
    // A random UUID's version (4) and variant (10), as DCE/RPC's activities are.
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    Uuid(b)
}

/// The IPv4 addresses an interface holds, in the order the system lists them.
#[cfg(target_os = "linux")]
pub fn addresses_of(iface: &str) -> Vec<Ipv4Addr> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a list the system owns until freeifaddrs.
    if unsafe { libc::getifaddrs(&raw mut list) } != 0 {
        return Vec::new();
    }
    let v4 = |sa: *const libc::sockaddr| -> Option<Ipv4Addr> {
        // SAFETY: a non-null sockaddr of the list, its family saying its shape; an AF_INET one is
        // a sockaddr_in.
        (!sa.is_null() && i32::from(unsafe { (*sa).sa_family }) == libc::AF_INET).then(|| Ipv4Addr::from(u32::from_be(unsafe { (*sa.cast::<libc::sockaddr_in>()).sin_addr.s_addr })))
    };
    let mut found = Vec::new();
    let mut at = list;
    while !at.is_null() {
        // SAFETY: `at` is an element of the list getifaddrs returned, not yet freed.
        let a = unsafe { &*at };
        // SAFETY: a non-null `ifa_name` is a NUL-terminated string of the list.
        let name = (!a.ifa_name.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(a.ifa_name) });
        if name.is_some_and(|n| n.to_bytes() == iface.as_bytes())
            && let Some(address) = v4(a.ifa_addr)
        {
            found.push(address);
        }
        at = a.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(list) };
    found
}

#[cfg(not(target_os = "linux"))]
pub fn addresses_of(_iface: &str) -> Vec<Ipv4Addr> {
    Vec::new()
}

/// The first IPv4 address an interface holds, if any.
pub fn address_of(iface: &str) -> Option<Ipv4Addr> {
    addresses_of(iface).first().copied()
}

/// The interface's address on the network of a device at `device` with `mask`, by the device's own
/// mask (the first such the system lists): a device answers without a router only a source on its
/// network; none when the interface holds none there.
pub fn address_for(iface: &str, device: Ipv4Addr, mask: Ipv4Addr) -> Option<Ipv4Addr> {
    let m = u32::from(mask);
    addresses_of(iface).into_iter().find(|a| u32::from(*a) & m == u32::from(device) & m)
}

/// The object a read is addressed to: the device's own (its vendor and device ids, its instance,
/// 1 when it states none), else the nil one.
fn object_of(device: &Device) -> Uuid {
    match device.pn_id.current() {
        Some((vendor, id)) => {
            let instance = device.instance.current().map_or(1, |(high, low)| u16::from_be_bytes([*high, *low]));
            Uuid::pnio_object(instance, *id, *vendor)
        }
        None => Uuid::NIL,
    }
}

/// Read `target` of `device` on `iface` through `listener`, and wait for its answer.
pub fn read(listener: &Listener, iface: &str, device: &Device, target: Target, options: &Options) -> Result<Record, ReadError> {
    if options.wait > MAX_WAIT {
        return Err(ReadError::Wait(options.wait));
    }
    let account = listener.interface(iface).ok_or_else(|| ListenError::NotListening(iface.to_string()))?;
    if let Some(reason) = account.ended {
        return Err(ReadError::Ended { iface: iface.to_string(), reason });
    }
    let mac = account.mac.ok_or_else(|| ReadError::NoMac(iface.to_string()))?;
    if !device.heard_on.contains(iface) {
        return Err(ReadError::NotHeard { device: device.mac, iface: iface.to_string() });
    }
    let suite = device.ip.current().filter(|s| !s.ip.is_unspecified()).ok_or(ReadError::Unaddressed(device.mac))?;
    let address = suite.ip;
    let here = match address_for(iface, address, suite.mask) {
        Some(here) => here,
        None if address_of(iface).is_some() => return Err(ReadError::OffNetwork { iface: iface.to_string(), device: address, mask: suite.mask }),
        None => return Err(ReadError::NoAddress(iface.to_string())),
    };
    let socket = UdpSocket::bind((here, 0)).map_err(|source| ReadError::Socket { address: here, source })?;
    let port = socket.local_addr().map_err(|source| ReadError::Socket { address: here, source })?.port();

    let activity = next_activity();
    let request = ReadRequest::implicit(0, target.slot, target.subslot, target.index, MAX_RECORD);
    let pdu = rpc::read_implicit(ByteOrder::Little, object_of(device), activity, 0, &request, MAX_RECORD + 64)?;
    let id = u16::from_be_bytes([activity.0[10], activity.0[11]]);
    let frame = ip::udp_frame(Endpoint { mac, ip: here, port }, Endpoint { mac: device.mac, ip: address, port: PNIO_PORT }, id, &pdu.encode()?)?;
    let tracked = listener.send_tracked(iface, frame)?;

    // Its turn: the frames queued ahead of it at the interface's pace, and a second more; past it,
    // taken back if it still waits.
    let gap = Duration::from_secs(1) / listener.options().rate;
    let waited = gap.saturating_mul(u32::try_from(tracked.ahead + 1).unwrap_or(u32::MAX)).saturating_add(Duration::from_secs(1));
    let told = match tracked.told.recv_timeout(waited) {
        Ok(outcome) => Some(outcome),
        Err(_) if listener.withdraw(iface, tracked.id) => return Err(ReadError::NotSent { iface: iface.to_string(), waited }),
        Err(_) => tracked.told.try_recv().ok(),
    };
    let (index, sent_ns) = match told {
        Some(Outcome::Sent { index, at_ns }) => (index, at_ns),
        Some(Outcome::Refused(reason)) => return Err(ReadError::Refused { iface: iface.to_string(), reason }),
        Some(Outcome::Unsent(reason)) => return Err(ReadError::Ended { iface: iface.to_string(), reason }),
        None => return Err(ReadError::Stalled { iface: iface.to_string(), waited }),
    };

    // The answer: the first datagram kept after the request from the device to the request's
    // address and port under its activity.
    let deadline = Instant::now() + options.wait;
    let poll = listener.options().poll.max(Duration::from_millis(1));
    let mut next = index + 1;
    let _ = socket.set_nonblocking(true);
    loop {
        let (lost, frames) = listener.read_from(next);
        if lost > 0 {
            return Err(ReadError::Lost(lost));
        }
        for (i, k) in frames {
            next = i + 1;
            if k.sent || k.frame.iface != iface || k.frame.is_cut() {
                continue;
            }
            let Some(d) = ip::datagram(&k.frame.bytes) else { continue };
            if d.src != (Endpoint { mac: device.mac, ip: address, port: PNIO_PORT }) || d.dst != (Endpoint { mac, ip: here, port }) {
                continue;
            }
            let Ok(answer) = RpcPdu::decode(&d.payload) else { continue };
            if answer.activity != activity || answer.interface != Uuid::PNIO_DEVICE {
                continue;
            }
            match answer.packet_type {
                PacketType::Response => {}
                PacketType::Fault | PacketType::Reject => return Err(ReadError::Rejected { device: device.mac, frame: i, kind: answer.packet_type }),
                // Working, a no-call, an acknowledgement: not the answer yet.
                _ => continue,
            }
            let bad = |why: String| ReadError::Answer { device: device.mac, frame: i, why };
            let args = ResponseArgs::decode(&answer.body, answer.byte_order()).map_err(|e| bad(e.to_string()))?;
            if args.status != PnioStatus::OK {
                return Err(ReadError::Status { device: device.mac, frame: i, status: args.status });
            }
            let (header, data) = ReadResponse::split(&args.data).map_err(|e| bad(e.to_string()))?;
            if (header.slot, header.subslot, header.index) != (target.slot, target.subslot, target.index) {
                return Err(bad(format!("it answers slot {} subslot {:#06x} index {:#06x}", header.slot, header.subslot, header.index)));
            }
            let len = usize::try_from(header.length).unwrap_or(usize::MAX);
            if data.len() < len {
                return Err(bad(format!("its header says {len} bytes of record and {} follow", data.len())));
            }
            let mut sink = [0u8; 2048];
            while socket.recv(&mut sink).is_ok() {}
            return Ok(Record {
                iface: iface.to_string(),
                device: device.mac,
                address,
                target,
                request: index,
                sent_ns,
                answer: i,
                at_ns: k.frame.at_ns,
                header,
                data: data[..len].to_vec(),
            });
        }
        if Instant::now() >= deadline {
            return Err(ReadError::NoAnswer { device: device.mac, waited: options.wait });
        }
        std::thread::sleep(poll.min(deadline.saturating_duration_since(Instant::now())));
    }
}

/// The most port subslots read of one device.
pub const MAX_PORTS: u16 = 16;

/// The status a device answers a read of a slot or subslot it does not have with: IODReadRes,
/// PNIORW, "access: invalid slot/subslot" (error code 1 0xB2).
pub fn is_no_subslot(status: PnioStatus) -> bool {
    (status.code, status.decode, status.code1) == (0xDE, 0x80, 0xB2)
}

/// A walk of a device's ports ended by a read refused or unanswered: the subslot, why, and the
/// ports read before it, kept.
#[derive(Debug)]
pub struct Cut {
    pub subslot: u16,
    pub error: ReadError,
    pub read: Ports,
}

/// Read the port data of `device`'s ports, one read at a time: subslot 0x8001 of slot 0 and on,
/// until the device answers that a subslot is not there (every port, not only those whose LLDP was
/// heard: a port facing another device is never heard here), at most `MAX_PORTS`; each port named
/// as its record names it, and the neighbours they report. Any other refusal, or no answer, ends
/// it, naming the subslot, with what was read before it.
pub fn ports(listener: &Listener, iface: &str, device: &Device, options: &Options) -> Result<Ports, Box<Cut>> {
    let mut out = Ports::default();
    for subslot in (SUBSLOT_PORTS + 1)..=(SUBSLOT_PORTS + MAX_PORTS) {
        let target = Target { slot: 0, subslot, index: INDEX_PORT_DATA_REAL };
        let record = match read(listener, iface, device, target, options) {
            Ok(r) => r,
            Err(ReadError::Status { status, frame, .. }) if is_no_subslot(status) => {
                out.end = Some(frame);
                return Ok(out);
            }
            Err(error) => return Err(Box::new(Cut { subslot, error, read: out })),
        };
        let frame = record.answer;
        let bad = |why: String| ReadError::Answer { device: device.mac, frame, why };
        let data = match rpc::read_blocks(&record.data).0.first() {
            None => Err(bad("it holds no block".into())),
            Some(block) => match PortDataReal::from_block(block) {
                Err(e) => Err(bad(e.to_string())),
                Ok(d) if (d.slot, d.subslot) != (0, subslot) => Err(bad(format!("its port data is of slot {} subslot {:#06x}", d.slot, d.subslot))),
                Ok(d) => Ok(d),
            },
        };
        let data = match data {
            Ok(d) => d,
            Err(error) => return Err(Box::new(Cut { subslot, error, read: out })),
        };
        let own = crate::topology::short(&data.own_port_id);
        for peer in &data.peers {
            out.neighbours.push(Neighbour {
                device: device.mac,
                port: own.clone(),
                chassis: peer.chassis_id.clone(),
                peer_port: peer.port_id.clone(),
                peer_mac: (peer.mac != Mac([0; 6])).then_some(peer.mac),
                witnesses: 1,
            });
        }
        out.read.push(PortRead { record, data });
    }
    Ok(out)
}
