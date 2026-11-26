//! IPv4 and UDP as the scanner writes and reads them on its raw link (`docs/aranha-plan.md`,
//! "Rules": the listener is the only sender on its interface, so a datagram the scanner sends goes
//! through it, paced and kept): one datagram in one Ethernet II frame, never a fragment, its
//! checksums written and checked. What is not a whole, well-formed datagram to the address and port
//! asked is none, never a guess at what a damaged one meant.

use std::net::Ipv4Addr;

use crate::profinet::ether::{self, ETHERTYPE_IPV4, MAX_PAYLOAD, MIN_FRAME};
use crate::profinet::mac::Mac;

/// IPv4's protocol number for UDP.
pub const UDP: u8 = 17;
/// The largest UDP payload one frame carries: 1500 less IPv4's 20 and UDP's 8.
pub const MAX_UDP_PAYLOAD: usize = MAX_PAYLOAD - 20 - 8;

/// One end of a datagram: its MAC, its address and its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub mac: Mac,
    pub ip: Ipv4Addr,
    pub port: u16,
}

/// A datagram read from a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datagram {
    pub src: Endpoint,
    pub dst: Endpoint,
    /// IPv4's identification.
    pub id: u16,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IpError {
    #[error("a datagram of {0} bytes is past the {MAX_UDP_PAYLOAD} one frame carries unfragmented")]
    TooLong(usize),
    #[error("{0} is no address a datagram goes from or to")]
    Address(Ipv4Addr),
}

fn sum(bytes: &[u8], mut acc: u32) -> u32 {
    for pair in bytes.chunks(2) {
        acc += u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
    }
    acc
}

fn fold(mut acc: u32) -> u16 {
    while acc > 0xFFFF {
        acc = (acc & 0xFFFF) + (acc >> 16);
    }
    !(acc as u16)
}

fn pseudo(src: Ipv4Addr, dst: Ipv4Addr, len: usize) -> u32 {
    sum(&src.octets(), sum(&dst.octets(), u32::from(UDP) + len as u32))
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// A frame carrying `payload` from `src` to `dst` as one UDP datagram over IPv4 (no options, don't
/// fragment, time to live 64), both checksums written; padded to the shortest frame.
pub fn udp_frame(src: Endpoint, dst: Endpoint, id: u16, payload: &[u8]) -> Result<Vec<u8>, IpError> {
    if payload.len() > MAX_UDP_PAYLOAD {
        return Err(IpError::TooLong(payload.len()));
    }
    for ip in [src.ip, dst.ip] {
        if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
            return Err(IpError::Address(ip));
        }
    }
    let len = 8 + payload.len();
    let mut udp = Vec::with_capacity(len);
    udp.extend_from_slice(&src.port.to_be_bytes());
    udp.extend_from_slice(&dst.port.to_be_bytes());
    udp.extend_from_slice(&(len as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(payload);
    // A computed 0 travels as 0xFFFF: 0 says no checksum.
    let c = match fold(sum(&udp, pseudo(src.ip, dst.ip, len))) {
        0 => 0xFFFF,
        c => c,
    };
    udp[6..8].copy_from_slice(&c.to_be_bytes());
    let mut ip = vec![0x45, 0];
    ip.extend_from_slice(&((20 + len) as u16).to_be_bytes());
    ip.extend_from_slice(&id.to_be_bytes());
    ip.extend_from_slice(&[0x40, 0, 64, UDP, 0, 0]);
    ip.extend_from_slice(&src.ip.octets());
    ip.extend_from_slice(&dst.ip.octets());
    let c = fold(sum(&ip, 0));
    ip[10..12].copy_from_slice(&c.to_be_bytes());
    let mut out = Vec::with_capacity(MIN_FRAME.max(14 + 20 + len));
    out.extend_from_slice(&dst.mac.0);
    out.extend_from_slice(&src.mac.0);
    out.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    out.extend(ip);
    out.extend(udp);
    if out.len() < MIN_FRAME {
        out.resize(MIN_FRAME, 0);
    }
    Ok(out)
}

/// The UDP datagram a frame carries, when it is one whole: Ethernet II (a tag allowed) with IPv4,
/// version 4, its header's checksum right, its length within the frame, not a fragment, protocol
/// UDP, its length within the IPv4 packet and its checksum right or absent. Anything else is none.
pub fn datagram(frame: &[u8]) -> Option<Datagram> {
    let e = ether::parse(frame).ok()?;
    if e.ethertype != ETHERTYPE_IPV4 {
        return None;
    }
    let p = e.payload;
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(p[0] & 0x0F) * 4;
    let total = usize::from(be16(p, 2));
    if ihl < 20 || total < ihl + 8 || total > p.len() || p[9] != UDP || fold(sum(&p[..ihl], 0)) != 0 {
        return None;
    }
    // More fragments, or an offset: a piece of a datagram, not one.
    if be16(p, 6) & 0x3FFF != 0 {
        return None;
    }
    let src = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
    let dst = Ipv4Addr::new(p[16], p[17], p[18], p[19]);
    let u = &p[ihl..total];
    let len = usize::from(be16(u, 4));
    if len < 8 || len > u.len() {
        return None;
    }
    let u = &u[..len];
    if be16(u, 6) != 0 && fold(sum(u, pseudo(src, dst, len))) != 0 {
        return None;
    }
    Some(Datagram {
        src: Endpoint { mac: e.src, ip: src, port: be16(u, 0) },
        dst: Endpoint { mac: e.dst, ip: dst, port: be16(u, 2) },
        id: be16(p, 4),
        payload: u[8..].to_vec(),
    })
}
