//! The device's own IP side on its raw link, as a device with no host stack answers: ARP for the
//! address it holds, IPv4 and UDP in and out, and the connectionless DCE/RPC record read on
//! aranha's codecs (`profinet::rpc`, `profinet::record`). Pure: frames in, frames out.

use std::net::Ipv4Addr;

use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::rpc::{self, PacketType, PnioStatus, ReadRequest, RpcPdu};

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

/// The status of a read of an index the device does not hold: IODReadRes, PNIORW, "invalid
/// index" (IEC 61158-6-10, the error codes of a record read).
pub const INVALID_INDEX: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB0, code2: 0x00 };
/// A slot or subslot the device does not have.
pub const INVALID_SLOT: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0x00 };
/// An API the device does not have ("invalid area/API").
pub const INVALID_API: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB4, code2: 0x00 };
/// A record the device holds but could not write out (an application's read error).
pub const READ_ERROR: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xA0, code2: 0x00 };

/// The read response header's block, before its record: what ArgsMaximum must leave room for.
const READ_HEADER: u32 = 64;

fn mac_at(f: &[u8], at: usize) -> Mac {
    let mut m = [0u8; 6];
    m.copy_from_slice(&f[at..at + 6]);
    Mac(m)
}

fn be16(f: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([f[at], f[at + 1]])
}

/// The answer to an ARP request for `ip`, broadcast or to `mac`: a reply from `mac`, padded to the
/// least frame. `None` for any other frame.
pub fn arp_reply(frame: &[u8], mac: Mac, ip: Ipv4Addr) -> Option<Vec<u8>> {
    if frame.len() < 42 || be16(frame, 12) != ETHERTYPE_ARP {
        return None;
    }
    // Broadcast or to its own MAC: a frame to another station is not its (a card drops it).
    let dst = mac_at(frame, 0);
    if dst != mac && dst != Mac::BROADCAST {
        return None;
    }
    let a = &frame[14..];
    // Ethernet and IPv4, 6 and 4 bytes, a request.
    if be16(a, 0) != 1 || be16(a, 2) != ETHERTYPE_IPV4 || a[4] != 6 || a[5] != 4 || be16(a, 6) != 1 {
        return None;
    }
    if a[24..28] != ip.octets() || ip.is_unspecified() {
        return None;
    }
    let (sha, spa) = (mac_at(a, 8), &a[14..18]);
    let mut out = Vec::with_capacity(60);
    out.extend_from_slice(&sha.0);
    out.extend_from_slice(&mac.0);
    out.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
    out.extend_from_slice(&[0, 1, 0x08, 0x00, 6, 4, 0, 2]);
    out.extend_from_slice(&mac.0);
    out.extend_from_slice(&ip.octets());
    out.extend_from_slice(&sha.0);
    out.extend_from_slice(spa);
    out.resize(60, 0);
    Some(out)
}

/// The ones'-complement sum of 16-bit words, folded.
fn sum(bytes: &[u8], mut acc: u32) -> u32 {
    for c in bytes.chunks(2) {
        acc += u32::from(u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]));
    }
    acc
}

fn fold(mut acc: u32) -> u16 {
    while acc > 0xFFFF {
        acc = (acc & 0xFFFF) + (acc >> 16);
    }
    !(acc as u16)
}

/// The UDP checksum over its pseudo-header; 0 travels as 0xFFFF.
fn udp_checksum(src: Ipv4Addr, dst: Ipv4Addr, udp: &[u8]) -> u16 {
    let mut acc = sum(&src.octets(), 0);
    acc = sum(&dst.octets(), acc);
    acc += 17 + udp.len() as u32;
    let c = fold(sum(udp, acc));
    if c == 0 { 0xFFFF } else { c }
}

/// A UDP datagram an IPv4 frame carries to `ip`:`port`: the sender's MAC, address and port and the
/// payload. `None` for a frame of another kind, a fragment, another address or port, or a header
/// or UDP checksum that does not hold.
pub fn udp_to(frame: &[u8], ip: Ipv4Addr, port: u16) -> Option<(Mac, Ipv4Addr, u16, Vec<u8>)> {
    if frame.len() < 14 + 20 + 8 || be16(frame, 12) != ETHERTYPE_IPV4 || ip.is_unspecified() {
        return None;
    }
    let p = &frame[14..];
    let ihl = usize::from(p[0] & 0x0F) * 4;
    let total = usize::from(be16(p, 2));
    if p[0] >> 4 != 4 || ihl < 20 || total < ihl + 8 || total > p.len() || p[9] != 17 || p[16..20] != ip.octets() {
        return None;
    }
    // Neither more fragments nor an offset: a fragment is not answered.
    if be16(p, 6) & 0x3FFF != 0 || fold(sum(&p[..ihl], 0)) != 0 {
        return None;
    }
    let src = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
    let u = &p[ihl..total];
    let len = usize::from(be16(u, 4));
    if be16(u, 2) != port || len < 8 || len > u.len() {
        return None;
    }
    let u = &u[..len];
    if be16(u, 6) != 0 && fold(sum(u, sum(&src.octets(), sum(&ip.octets(), 17 + len as u32)))) != 0 {
        return None;
    }
    Some((mac_at(frame, 6), src, be16(u, 0), u[8..].to_vec()))
}

/// An Ethernet frame carrying `payload` in UDP from `src` to `dst`, its IPv4 header's
/// identification `id`, time to live 64, both checksums computed.
pub fn udp_frame(dst: (Mac, Ipv4Addr, u16), src: (Mac, Ipv4Addr, u16), id: u16, payload: &[u8]) -> Vec<u8> {
    let mut udp = Vec::with_capacity(8 + payload.len());
    udp.extend_from_slice(&src.2.to_be_bytes());
    udp.extend_from_slice(&dst.2.to_be_bytes());
    udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(payload);
    let c = udp_checksum(src.1, dst.1, &udp);
    udp[6..8].copy_from_slice(&c.to_be_bytes());
    let mut ip = vec![0x45, 0];
    ip.extend_from_slice(&((20 + udp.len()) as u16).to_be_bytes());
    ip.extend_from_slice(&id.to_be_bytes());
    ip.extend_from_slice(&[0x40, 0, 64, 17, 0, 0]);
    ip.extend_from_slice(&src.1.octets());
    ip.extend_from_slice(&dst.1.octets());
    let c = fold(sum(&ip, 0));
    ip[10..12].copy_from_slice(&c.to_be_bytes());
    let mut out = Vec::with_capacity(14 + ip.len() + udp.len());
    out.extend_from_slice(&dst.0.0);
    out.extend_from_slice(&src.0.0);
    out.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    out.extend(ip);
    out.extend(udp);
    if out.len() < 60 {
        out.resize(60, 0);
    }
    out
}

/// What a record read is answered with: the record's bytes, or the status that refuses it.
pub type Records<'a> = &'a dyn Fn(&ReadRequest) -> Result<Vec<u8>, PnioStatus>;

/// The answer to a connectionless DCE/RPC datagram for the device: an implicit read of a record
/// answered with the record or the status that refuses it. `None` for what the device does not
/// answer: not a request, another interface, another operation, a fragment of several, a read
/// block it cannot decode.
pub fn answer(datagram: &[u8], records: Records, server_boot: u32) -> Option<(ReadRequest, Vec<u8>)> {
    let request = RpcPdu::decode(datagram).ok()?;
    if request.packet_type != PacketType::Request || request.interface != rpc::Uuid::PNIO_DEVICE || request.opnum != rpc::opnum::READ_IMPLICIT {
        return None;
    }
    // A request whole in its datagram: not a fragment, or one that is both its first and last.
    // The device puts no request back together from several.
    let fragmented = request.flags1 & rpc::flags1::FRAGMENT != 0;
    if fragmented && (request.flags1 & rpc::flags1::LAST_FRAGMENT == 0 || request.fragment != 0) {
        return None;
    }
    let args = rpc::RequestArgs::decode(&request.body, request.byte_order()).ok()?;
    let (blocks, _) = rpc::read_blocks(&args.data);
    let read = ReadRequest::from_block(blocks.first()?).ok()?;
    // The record cut to the length asked, and to what the request's ArgsMaximum leaves after the
    // answer's header.
    // An ArgsMaximum that cannot hold the answer's header is a read the device cannot write out:
    // refused with no data at all, so the answer stays within what the caller allows.
    if args.args_maximum < READ_HEADER {
        let pdu = RpcPdu::pnio_response(&request, server_boot, &rpc::ResponseArgs::new(READ_ERROR, args.args_maximum, Vec::new()));
        return Some((read, pdu.encode().ok()?));
    }
    let room = read.length.min(args.args_maximum.saturating_sub(READ_HEADER)) as usize;
    let (status, record) = match records(&read) {
        Ok(r) => (PnioStatus::OK, r.into_iter().take(room).collect::<Vec<u8>>()),
        Err(s) => (s, Vec::new()),
    };
    let pdu = rpc::read_answer(&request, &read, status, &record, server_boot).ok()?;
    Some((read, pdu.encode().ok()?))
}
