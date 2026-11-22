//! PNIO over connectionless DCE/RPC (contract: the RPC clause): the header and the arguments to the
//! byte in both byte orders, the UUIDs, the blocks, the record read's headers, damage named, and
//! every datagram read back by Wireshark's dissectors inside UDP and IPv4.

mod common;

use serde_json::Value;
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::rpc::{
    self, Block, ByteOrder, PacketType, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcError, RpcPdu, Uuid,
    UuidParseError, flags1, opnum,
};

const CONTROLLER: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0C]);
const DEVICE: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0D]);

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn activity() -> Uuid {
    "12345678-1234-5678-9abc-def012345678".parse().unwrap()
}

fn object() -> Uuid {
    Uuid::pnio_object(0x0001, 0x0203, 0x002A)
}

/// The implicit read of I&M0 at subslot 1, taking at most 4096 bytes of answer.
fn im0_read() -> ReadRequest {
    ReadRequest::implicit(1, 0, 1, 0xAFF0, 4032)
}

/// An I&M0 record as a device answers it (a block of its own).
fn im0_record() -> Vec<u8> {
    hex("0020 0038 0100 002A")
        .into_iter()
        .chain(b"6ES7 123-4AB56-7CD8 ".iter().copied())
        .chain(b"S C-ABCD12345678".iter().copied())
        .chain(hex("0003 56020100 0005 0000 0000 0101 001E"))
        .collect()
}

#[test]
fn a_uuid_is_its_text_and_travels_in_the_header_s_byte_order() {
    let u = activity();
    assert_eq!(u.0, hex("12345678 1234 5678 9abc def012345678")[..]);
    assert_eq!(u.to_string(), "12345678-1234-5678-9abc-def012345678");
    assert_eq!("12345678-1234-5678-9ABC-DEF012345678".parse::<Uuid>().unwrap(), u);
    assert_eq!(u.to_wire(ByteOrder::Big), u.0);
    assert_eq!(u.to_wire(ByteOrder::Little)[..], hex("78563412 3412 7856 9abc def012345678")[..]);
    assert_eq!(Uuid::from_wire(u.to_wire(ByteOrder::Little), ByteOrder::Little), u);
    assert_eq!(Uuid::from_wire(u.0, ByteOrder::Big), u);
    for bad in [
        "",
        "12345678123456789abcdef012345678",
        "12345678-1234-5678-9abc-def01234567",
        "12345678-1234-5678-9abc-def0123456789",
        "1234567-81234-5678-9abc-def012345678",
        "12345678-1234-5678-9abc-def01234567g",
        "12345678-1234-5678-9abc+def012345678",
        "{12345678-1234-5678-9abc-def012345678}",
        "+2345678-1234-5678-9abc-def012345678",
    ] {
        assert_eq!(bad.parse::<Uuid>(), Err(UuidParseError(bad.into())), "{bad:?}");
    }
    assert_eq!(
        UuidParseError("x".into()).to_string(),
        "\"x\" is not a UUID: 8, 4, 4, 4 and 12 hexadecimal digits joined by hyphens"
    );
    assert_eq!(serde_json::to_string(&u).unwrap(), "\"12345678-1234-5678-9abc-def012345678\"");
    assert_eq!(serde_json::from_str::<Uuid>("\"12345678-1234-5678-9abc-def012345678\"").unwrap(), u);
    assert!(serde_json::from_str::<Uuid>("\"12345678\"").is_err());

    assert_eq!(Uuid::PNIO_DEVICE.to_string(), "dea00001-6c97-11d1-8271-00a02442df7d");
    assert_eq!(Uuid::PNIO_CONTROLLER.to_string(), "dea00002-6c97-11d1-8271-00a02442df7d");
    assert_eq!(Uuid::PNIO_SUPERVISOR.to_string(), "dea00003-6c97-11d1-8271-00a02442df7d");
    assert_eq!(Uuid::PNIO_PARAMETER_SERVER.to_string(), "dea00004-6c97-11d1-8271-00a02442df7d");
    assert_eq!(Uuid::ENDPOINT_MAPPER.to_string(), "e1af8308-5d1f-11c9-91a4-08002b14a0fa");
    assert_eq!(Uuid::NIL.to_string(), "00000000-0000-0000-0000-000000000000");
    assert_eq!(object().to_string(), "dea00000-6c97-11d1-8271-00010203002a");
    assert_eq!(object().pnio_object_parts(), Some((0x0001, 0x0203, 0x002A)));
    assert_eq!(Uuid::pnio_object(0xFFFF, 0, 0x1234).pnio_object_parts(), Some((0xFFFF, 0, 0x1234)));
    assert_eq!(Uuid::PNIO_DEVICE.pnio_object_parts(), None);
    for near in ["dea00000-6c97-11d1-8272-00010203002a", "dea00000-6c97-11d1-8371-00010203002a", "dea00000-6c97-11d2-8271-00010203002a", "dea00001-6c97-11d1-8271-00010203002a"] {
        assert_eq!(near.parse::<Uuid>().unwrap().pnio_object_parts(), None, "{near}");
    }
    assert_eq!(activity().pnio_object_parts(), None);
    assert_eq!((rpc::PNIO_PORT, rpc::HEADER), (34964, 80));
}

#[test]
fn an_implicit_read_is_its_header_its_arguments_and_its_block_to_the_byte() {
    let pdu = rpc::read_implicit(ByteOrder::Little, object(), activity(), 7, &im0_read(), 4096).unwrap();
    let want = hex(
        "04 00 20 00 100000 00 \
         0000a0de 976c d111 8271 00010203002a \
         0100a0de 976c d111 8271 00a02442df7d \
         78563412 3412 7856 9abc def012345678 \
         00000000 01000000 07000000 0500 ffff ffff 5400 0000 00 00 \
         00100000 40000000 00100000 00000000 40000000 \
         0009 003c 0100 0001 00000000000000000000000000000000 00000000 0000 0001 0000 aff0 00000fc0 \
         00000000000000000000000000000000 0000000000000000",
    );
    assert_eq!(pdu.encode().unwrap(), want);
    assert_eq!(RpcPdu::decode(&want).unwrap(), pdu);
    assert_eq!(
        (pdu.packet_type, pdu.flags1, pdu.drep, pdu.interface, pdu.opnum, pdu.interface_version),
        (PacketType::Request, flags1::IDEMPOTENT, [0x10, 0, 0], Uuid::PNIO_DEVICE, opnum::READ_IMPLICIT, 1)
    );
    let args = RequestArgs::decode(&pdu.body, pdu.byte_order()).unwrap();
    assert_eq!(args, RequestArgs::new(4096, im0_read().block().encode().unwrap()));
    let (blocks, rest) = rpc::read_blocks(&args.data);
    assert_eq!((blocks.len(), rest.len()), (1, 0));
    assert_eq!(ReadRequest::from_block(&blocks[0]).unwrap(), im0_read());

    // Big-endian: the header's integers, the UUIDs' first fields and the counts turn; the block
    // does not.
    let big = rpc::read_implicit(ByteOrder::Big, object(), activity(), 7, &im0_read(), 4096).unwrap();
    let bytes = big.encode().unwrap();
    assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
    assert_eq!(&bytes[8..24], &object().0);
    assert_eq!(&bytes[60..80], &hex("00000001 00000007 0005 ffff ffff 0054 0000 00 00")[..]);
    assert_eq!(&bytes[80..100], &hex("00001000 00000040 00001000 00000000 00000040")[..]);
    assert_eq!(&bytes[100..], &want[100..]);
    assert_eq!(RpcPdu::decode(&bytes).unwrap(), big);
    assert_eq!(RequestArgs::decode(&big.body, ByteOrder::Big).unwrap(), args);
}

#[test]
fn a_read_answer_is_its_status_its_header_and_the_record() {
    let request = rpc::read_implicit(ByteOrder::Little, object(), activity(), 7, &im0_read(), 4096).unwrap();
    let answer = rpc::read_answer(&request, &im0_read(), PnioStatus::OK, &im0_record(), 0x0102_0304).unwrap();
    let bytes = answer.encode().unwrap();
    assert_eq!(&bytes[..8], &hex("04 02 0a 00 100000 00")[..]);
    assert_eq!(&bytes[8..56], &request.encode().unwrap()[8..56]);
    assert_eq!(&bytes[56..80], &hex("04030201 01000000 07000000 0500 ffff ffff 9000 0000 00 00")[..]);
    assert_eq!(&bytes[80..100], &hex("00000000 7c000000 00100000 00000000 7c000000")[..]);
    assert_eq!(
        &bytes[100..164],
        &hex("8009 003c 0100 0001 00000000000000000000000000000000 00000000 0000 0001 0000 aff0 0000003c 0000 0000 0000000000000000000000000000000000000000")[..]
    );
    assert_eq!(&bytes[164..], &im0_record()[..]);
    assert_eq!(bytes.len(), 80 + 20 + 64 + 60);
    let back = RpcPdu::decode(&bytes).unwrap();
    assert_eq!(back, answer);
    assert_eq!((back.packet_type, back.flags1, back.server_boot), (PacketType::Response, flags1::LAST_FRAGMENT | flags1::NO_FACK, 0x0102_0304));
    let args = ResponseArgs::decode(&back.body, back.byte_order()).unwrap();
    assert_eq!(args, ResponseArgs::new(PnioStatus::OK, 4096, [ReadResponse::answering(&im0_read(), 60).block().encode().unwrap(), im0_record()].concat()));
    let (header, record) = ReadResponse::split(&args.data).unwrap();
    assert_eq!(header, ReadResponse { seq: 1, ar: Uuid::NIL, api: 0, slot: 0, subslot: 1, index: 0xAFF0, length: 60, add1: 0, add2: 0 });
    assert_eq!(record, im0_record());

    // A refusal: the status in its bytes, a header and no record.
    let refused = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB0, code2: 0x00 };
    let no = rpc::read_answer(&request, &im0_read(), refused, &[], 0).unwrap();
    let b = no.encode().unwrap();
    assert_eq!(&b[80..84], &[0xDE, 0x80, 0xB0, 0x00]);
    let args = ResponseArgs::decode(&no.body, ByteOrder::Little).unwrap();
    assert_eq!((args.status, args.data.len()), (refused, 64));
    assert_eq!(ReadResponse::split(&args.data).unwrap().1, Vec::<u8>::new());
    // Big-endian, answered in the request's order.
    let big = rpc::read_implicit(ByteOrder::Big, object(), activity(), 7, &im0_read(), 4096).unwrap();
    let answer = rpc::read_answer(&big, &im0_read(), PnioStatus::OK, &im0_record(), 0).unwrap();
    assert_eq!(answer.drep, [0, 0, 0]);
    assert_eq!(&answer.encode().unwrap()[84..88], &[0, 0, 0, 0x7C]);
}

#[test]
fn a_damaged_datagram_is_an_error_that_names_it_and_trailing_bytes_are_kept() {
    let good = rpc::read_implicit(ByteOrder::Little, object(), activity(), 7, &im0_read(), 4096).unwrap().encode().unwrap();
    assert_eq!(RpcPdu::decode(&good[..79]), Err(RpcError::TooShort(79)));
    assert_eq!(RpcPdu::decode(&[]), Err(RpcError::TooShort(0)));
    let mut v5 = good.clone();
    v5[0] = 5;
    assert_eq!(RpcPdu::decode(&v5), Err(RpcError::NotConnectionless(5)));
    assert_eq!(RpcPdu::decode(&good[..good.len() - 1]), Err(RpcError::Overrun { declared: 84, available: 83 }));
    let mut longer = good.clone();
    longer.extend_from_slice(&[0xAA, 0xBB]);
    let pdu = RpcPdu::decode(&longer).unwrap();
    assert_eq!((pdu.body.len(), pdu.trailing.clone()), (84, vec![0xAA, 0xBB]));
    assert_eq!(pdu.encode().unwrap(), longer);
    assert_eq!(RpcError::TooShort(79).to_string(), "a connectionless RPC header is 80 bytes; this datagram is 79");
    assert_eq!(RpcError::NotConnectionless(5).to_string(), "RPC version 5 is not the connectionless protocol's (4)");
    assert_eq!(RpcError::Overrun { declared: 84, available: 83 }.to_string(), "the body is said to be 84 bytes and 83 follow the header");

    // The serial number's two bytes travel apart; every packet type is carried.
    let mut odd = good.clone();
    odd[7] = 0x12;
    odd[79] = 0x34;
    odd[1] = 11;
    let pdu = RpcPdu::decode(&odd).unwrap();
    assert_eq!((pdu.serial, pdu.packet_type), (0x1234, PacketType::Other(11)));
    assert_eq!(pdu.encode().unwrap(), odd);
    for b in 0..=255u8 {
        assert_eq!(PacketType::of(b).byte(), b);
    }
    let named = [
        PacketType::Request,
        PacketType::Ping,
        PacketType::Response,
        PacketType::Fault,
        PacketType::Working,
        PacketType::Nocall,
        PacketType::Reject,
        PacketType::Ack,
        PacketType::Cancel,
        PacketType::Fack,
        PacketType::CancelAck,
    ];
    for (i, t) in named.iter().enumerate() {
        assert_eq!(PacketType::of(i as u8), *t);
    }
    assert_eq!(
        (flags1::LAST_FRAGMENT, flags1::FRAGMENT, flags1::NO_FACK, flags1::MAYBE, flags1::IDEMPOTENT, flags1::BROADCAST, rpc::flags2::CANCEL_PENDING),
        (0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x02)
    );
    assert_eq!(
        (opnum::CONNECT, opnum::RELEASE, opnum::READ, opnum::WRITE, opnum::CONTROL, opnum::READ_IMPLICIT),
        (0, 1, 2, 3, 4, 5)
    );
    // drep: only bit 4 of its first byte says the order; the rest is carried.
    assert_eq!(ByteOrder::of_drep([0x10, 0, 0]), ByteOrder::Little);
    assert_eq!(ByteOrder::of_drep([0x11, 0x01, 0x02]), ByteOrder::Little);
    assert_eq!(ByteOrder::of_drep([0x00, 0, 0]), ByteOrder::Big);
    assert_eq!(ByteOrder::of_drep([0xE0, 0, 0]), ByteOrder::Big);
    assert_eq!((ByteOrder::Big.drep(), ByteOrder::Little.drep()), ([0, 0, 0], [0x10, 0, 0]));

    // A body past what the length counts is refused; 65535 is written.
    let mut pdu = RpcPdu::decode(&good).unwrap();
    pdu.body = vec![0; 65_536];
    assert_eq!(pdu.encode(), Err(RpcError::BodyTooLong(65_536)));
    pdu.body = vec![0; 65_535];
    let bytes = pdu.encode().unwrap();
    assert_eq!(RpcPdu::decode(&bytes).unwrap(), pdu);
    assert_eq!(RpcError::BodyTooLong(65_536).to_string(), "a body of 65536 bytes is past what the header's length counts (65535)");

    // Every cut of a good datagram, and twenty thousand noisy ones, decode or name their damage
    // without a panic; each decoded one writes back to its own bytes.
    for cut in 0..=good.len() {
        if let Ok(p) = RpcPdu::decode(&good[..cut]) {
            assert_eq!(p.encode().unwrap(), &good[..cut]);
        }
    }
    let mut noise = common::Noise(0x5EED);
    let mut decoded = 0;
    for i in 0..20_000 {
        let len = 70 + (noise.next() as usize) % 60;
        let mut d = noise.bytes(len);
        if i % 2 == 0 {
            d[0] = 4;
        }
        if d.len() >= 80 && i % 3 == 0 {
            let body = (d.len() - 80 - (noise.next() as usize % 3).min(d.len() - 80)) as u16;
            let at = 74;
            let (a, b) = if ByteOrder::of_drep([d[4], d[5], d[6]]) == ByteOrder::Little { (body as u8, (body >> 8) as u8) } else { ((body >> 8) as u8, body as u8) };
            d[at] = a;
            d[at + 1] = b;
        }
        if let Ok(p) = RpcPdu::decode(&d) {
            decoded += 1;
            assert_eq!(p.encode().unwrap(), d);
            for o in [ByteOrder::Big, ByteOrder::Little] {
                if let Ok(a) = RequestArgs::decode(&p.body, o) {
                    assert_eq!(a.encode(o), p.body);
                }
                if let Ok(a) = ResponseArgs::decode(&p.body, o) {
                    assert_eq!(a.encode(o), p.body);
                }
            }
        }
    }
    assert!(decoded > 2_000, "{decoded} noisy datagrams decoded");
}

#[test]
fn the_arguments_name_their_damage_and_keep_what_follows() {
    for o in [ByteOrder::Big, ByteOrder::Little] {
        let args = RequestArgs::new(4096, vec![1, 2, 3]);
        let body = args.encode(o);
        assert_eq!(body.len(), 23);
        assert_eq!(RequestArgs::decode(&body, o).unwrap(), args);
        assert_eq!(RequestArgs::decode(&body[..19], o), Err(RpcError::ArgsTooShort { want: 20, len: 19 }));
        assert_eq!(RequestArgs::decode(&body[..22], o), Err(RpcError::ArgsOverrun { declared: 3, available: 2 }));
        let mut more = body.clone();
        more.extend_from_slice(&[9, 9]);
        let read = RequestArgs::decode(&more, o).unwrap();
        assert_eq!((read.data.clone(), read.trailing.clone()), (vec![1, 2, 3], vec![9, 9]));
        assert_eq!(read.encode(o), more);
        let res = ResponseArgs::new(PnioStatus { code: 0xDE, decode: 0x81, code1: 0x01, code2: 0x02 }, 512, vec![7; 5]);
        let body = res.encode(o);
        assert_eq!(&body[..4], &[0xDE, 0x81, 0x01, 0x02]);
        assert_eq!(ResponseArgs::decode(&body, o).unwrap(), res);
        assert_eq!(ResponseArgs::decode(&body[..24], o), Err(RpcError::ArgsOverrun { declared: 5, available: 4 }));
        assert_eq!(ResponseArgs::decode(&body[..3], o), Err(RpcError::ArgsTooShort { want: 20, len: 3 }));
    }
    // Hand-written counts are kept, but for the actual count, which is the data's length.
    let odd = RequestArgs { args_maximum: 1, args_length: 2, max_count: 3, offset: 4, data: vec![5], trailing: vec![] };
    assert_eq!(odd.encode(ByteOrder::Big), hex("00000001 00000002 00000003 00000004 00000001 05"));
    assert_eq!(RpcError::ArgsTooShort { want: 20, len: 3 }.to_string(), "PNIO's arguments are 20 bytes before their data; this body is 3");
    assert_eq!(RpcError::ArgsOverrun { declared: 5, available: 4 }.to_string(), "the arguments' data is said to be 5 bytes and 4 follow");
}

#[test]
fn blocks_are_read_in_order_and_what_is_not_a_whole_block_is_kept() {
    let a = Block::new(0x0020, vec![1, 2, 3]);
    let b = Block { block_type: 0x8009, version: (1, 1), body: vec![] };
    let data = [a.encode().unwrap(), b.encode().unwrap()].concat();
    assert_eq!(data, hex("0020 0005 0100 010203 8009 0002 0101"));
    assert_eq!(rpc::read_blocks(&data), (vec![a.clone(), b.clone()], vec![]));
    for (tail, kept) in [
        (hex("0020 00"), hex("0020 00")),
        (hex("0020 0009 0100 0102"), hex("0020 0009 0100 0102")),
        (hex("0020 0001 01"), hex("0020 0001 01")),
        (hex("0020 0000"), hex("0020 0000")),
        (hex("00"), hex("00")),
    ] {
        let all = [data.clone(), tail].concat();
        assert_eq!(rpc::read_blocks(&all), (vec![a.clone(), b.clone()], kept));
    }
    assert_eq!(rpc::read_blocks(&[]), (vec![], vec![]));
    assert_eq!(Block::new(1, vec![0; 65_533]).encode().unwrap().len(), 65_539);
    assert_eq!(Block::new(1, vec![0; 65_534]).encode(), Err(RpcError::BlockTooLong(65_534)));
    assert_eq!(RpcError::BlockTooLong(65_534).to_string(), "a block's body of 65534 bytes is past what its length counts (65533)");
}

#[test]
fn a_read_header_is_read_only_from_its_own_block() {
    let req = ReadRequest { seq: 0x0102, ar: activity(), api: 0x0304_0506, slot: 7, subslot: 0x8001, index: 0x802A, length: 99, target_ar: object() };
    let block = req.block();
    assert_eq!((block.block_type, block.version, block.body.len()), (rpc::READ_REQUEST, (1, 0), 58));
    assert_eq!(ReadRequest::from_block(&block).unwrap(), req);
    let res = ReadResponse { seq: 2, ar: activity(), api: 1, slot: 2, subslot: 3, index: 4, length: 5, add1: 6, add2: 7 };
    let rblock = res.block();
    assert_eq!((rblock.block_type, rblock.version, rblock.body.len()), (rpc::READ_RESPONSE, (1, 0), 58));
    assert_eq!(ReadResponse::from_block(&rblock).unwrap(), res);
    // Another type, version or length, or padding that is not zero: not a read header.
    let unfit = |b: Block| ReadRequest::from_block(&b).is_err() && ReadResponse::from_block(&b).is_err();
    assert!(unfit(Block { block_type: 0x0008, ..block.clone() }));
    assert!(ReadResponse::from_block(&block).is_err());
    assert!(ReadRequest::from_block(&rblock).is_err());
    for version in [(1, 1), (2, 0), (0, 0)] {
        assert!(ReadRequest::from_block(&Block { version, ..block.clone() }).is_err(), "{version:?}");
        assert!(ReadResponse::from_block(&Block { version, ..rblock.clone() }).is_err(), "{version:?}");
    }
    let mut short = block.clone();
    short.body.pop();
    assert!(ReadRequest::from_block(&short).is_err());
    let mut long = rblock.clone();
    long.body.push(0);
    assert!(ReadResponse::from_block(&long).is_err());
    for at in [26, 27, 50, 57] {
        let mut b = block.clone();
        b.body[at] = 1;
        assert!(ReadRequest::from_block(&b).is_err(), "{at}");
    }
    for at in [26, 27, 38, 57] {
        let mut b = rblock.clone();
        b.body[at] = 1;
        assert!(ReadResponse::from_block(&b).is_err(), "{at}");
    }
    assert_eq!(
        ReadRequest::from_block(&Block::new(0x0008, vec![])).unwrap_err().to_string(),
        "a read request is block 0x0009 version 1.0 of 60 bytes with zero padding; this block is 0x0008 version 1.0 of 2 bytes"
    );
    assert_eq!(
        ReadResponse::split(&[0; 63]).unwrap_err().to_string(),
        "a read answer's data begins with its 64-byte header; this one is 63 bytes"
    );
    assert!(ReadResponse::split(&block.encode().unwrap()).is_err());
}

fn checksum(b: &[u8]) -> u16 {
    let mut s: u32 = b.chunks(2).map(|c| u32::from(u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))).sum();
    while s > 0xFFFF {
        s = (s >> 16) + (s & 0xFFFF);
    }
    !(s as u16)
}

/// The datagram in UDP in IPv4 in Ethernet, from `src` to `dst`.
fn udp(dst: (Mac, [u8; 4], u16), src: (Mac, [u8; 4], u16), datagram: &[u8]) -> Vec<u8> {
    let mut u = Vec::new();
    u.extend_from_slice(&src.2.to_be_bytes());
    u.extend_from_slice(&dst.2.to_be_bytes());
    u.extend_from_slice(&((8 + datagram.len()) as u16).to_be_bytes());
    u.extend_from_slice(&[0, 0]);
    u.extend_from_slice(datagram);
    let mut ip = vec![0x45, 0];
    ip.extend_from_slice(&((20 + u.len()) as u16).to_be_bytes());
    ip.extend_from_slice(&[0, 1, 0, 0, 64, 17, 0, 0]);
    ip.extend_from_slice(&src.1);
    ip.extend_from_slice(&dst.1);
    let c = checksum(&ip);
    ip[10..12].copy_from_slice(&c.to_be_bytes());
    let mut f = Vec::new();
    f.extend_from_slice(&dst.0.0);
    f.extend_from_slice(&src.0.0);
    f.extend_from_slice(&[0x08, 0x00]);
    f.extend_from_slice(&ip);
    f.extend_from_slice(&u);
    f
}

/// Every value of `key` (tshark's flattened name) anywhere in `layer`.
fn field(d: &common::Dissected, layer: &str, key: &str) -> Vec<String> {
    fn walk(v: &Value, key: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == key {
                        match x {
                            Value::String(s) => out.push(s.clone()),
                            Value::Array(a) => out.extend(a.iter().map(|y| y.as_str().map(str::to_string).unwrap_or_else(|| y.to_string()))),
                            other => out.push(other.to_string()),
                        }
                    } else {
                        walk(x, key, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&d.layers[layer], key, &mut out);
    out
}

#[test]
fn the_dissectors_read_every_datagram_as_it_was_written() {
    let me = (CONTROLLER, [10, 0, 0, 1], 50_000);
    let dev = (DEVICE, [10, 0, 0, 2], rpc::PNIO_PORT);
    let port = ReadRequest::implicit(2, 0, 0x8001, 0x802A, 4032);
    let le = rpc::read_implicit(ByteOrder::Little, object(), activity(), 7, &im0_read(), 4096).unwrap();
    let le_answer = rpc::read_answer(&le, &im0_read(), PnioStatus::OK, &im0_record(), 3).unwrap();
    let be = rpc::read_implicit(ByteOrder::Big, Uuid::pnio_object(1, 0x0A01, 0x002A), activity(), 8, &port, 4096).unwrap();
    let be_answer = rpc::read_answer(&be, &port, PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB0, code2: 0x00 }, &[], 3).unwrap();
    let datagrams = [&le, &le_answer, &be, &be_answer];
    let frames: Vec<Vec<u8>> = datagrams
        .iter()
        .enumerate()
        .map(|(i, p)| if i % 2 == 0 { udp(dev, me, &p.encode().unwrap()) } else { udp(me, dev, &p.encode().unwrap()) })
        .collect();
    let d = common::dissect(&frames);
    let reads = [(&im0_read(), 60u32), (&im0_read(), 60), (&port, 0), (&port, 0)];
    for (i, ((p, d), (read, record))) in datagrams.iter().zip(&d).zip(reads).enumerate() {
        assert!(d.complaints().is_empty(), "datagram {i}: {:?}", d.complaints());
        let rpc_one = |k: &str| {
            let v = field(d, "dcerpc", &format!("dcerpc_dcerpc_{k}"));
            assert_eq!(v.len(), 1, "datagram {i} {k}: {v:?}");
            v[0].clone()
        };
        assert_eq!(rpc_one("ver"), "4");
        assert_eq!(rpc_one("pkt_type"), p.packet_type.byte().to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_flags1"), p.flags1.to_string(), "datagram {i}");
        assert_eq!(rpc_one("drep_byteorder"), if p.byte_order() == ByteOrder::Little { "1" } else { "0" }, "datagram {i}");
        assert_eq!(rpc_one("obj_id"), p.object.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_if_id"), p.interface.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_act_id"), p.activity.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_if_ver"), p.interface_version.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_seqnum"), p.sequence.to_string(), "datagram {i}");
        assert_eq!(rpc_one("opnum"), p.opnum.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_ihint"), p.interface_hint.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_ahint"), p.activity_hint.to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_frag_len"), p.body.len().to_string(), "datagram {i}");
        assert_eq!(rpc_one("dg_frag_num"), p.fragment.to_string(), "datagram {i}");
        assert_eq!((rpc_one("dg_serial_hi"), rpc_one("dg_serial_lo")), ((p.serial >> 8).to_string(), (p.serial & 0xFF).to_string()), "datagram {i}");
        let io = |k: &str| {
            let v = field(d, "pn_io_device", &format!("pn_io_pn_io_{k}"));
            assert_eq!(v.len(), 1, "datagram {i} {k}: {v:?}");
            v[0].clone()
        };
        let response = p.packet_type == PacketType::Response;
        let (data_len, max) = if response {
            let a = ResponseArgs::decode(&p.body, p.byte_order()).unwrap();
            let s = field(d, "pn_io_device", "pn_rt_pn_io_error_code");
            assert_eq!(s, [a.status.code.to_string()], "datagram {i}");
            assert_eq!(field(d, "pn_io_device", "pn_rt_pn_io_error_decode"), [a.status.decode.to_string()], "datagram {i}");
            (a.data.len(), a.max_count)
        } else {
            let a = RequestArgs::decode(&p.body, p.byte_order()).unwrap();
            assert_eq!(io("args_max"), a.args_maximum.to_string(), "datagram {i}");
            (a.data.len(), a.max_count)
        };
        assert_eq!(io("args_len"), data_len.to_string(), "datagram {i}");
        assert_eq!(io("array_max_count"), max.to_string(), "datagram {i}");
        assert_eq!(io("array_offset"), "0", "datagram {i}");
        assert_eq!(io("array_act_count"), data_len.to_string(), "datagram {i}");
        // The read header: the first block.
        let types = field(d, "pn_io_device", "pn_io_pn_io_block_type");
        assert_eq!(types[0], if response { "32777" } else { "9" }, "datagram {i}");
        let first = |k: &str| field(d, "pn_io_device", &format!("pn_io_pn_io_{k}"))[0].clone();
        assert_eq!(first("block_length"), "60", "datagram {i}");
        assert_eq!((first("block_version_high"), first("block_version_low")), ("1".into(), "0".into()), "datagram {i}");
        assert_eq!(first("seq_number"), read.seq.to_string(), "datagram {i}");
        assert_eq!(io("ar_uuid"), read.ar.to_string(), "datagram {i}");
        assert_eq!(first("api"), read.api.to_string(), "datagram {i}");
        assert_eq!(first("slot_nr"), read.slot.to_string(), "datagram {i}");
        assert_eq!(first("subslot_nr"), read.subslot.to_string(), "datagram {i}");
        assert_eq!(io("index"), read.index.to_string(), "datagram {i}");
        if response {
            assert_eq!(io("record_data_length"), record.to_string(), "datagram {i}");
            assert_eq!((io("add_val1"), io("add_val2")), ("0".into(), "0".into()), "datagram {i}");
        } else {
            assert_eq!(io("record_data_length"), read.length.to_string(), "datagram {i}");
            assert_eq!(io("target_ar_uuid"), read.target_ar.to_string(), "datagram {i}");
        }
    }
    // The I&M0 record after the header is a block of its own; the refusal's status bytes are read
    // as the record read's codes.
    assert_eq!(field(&d[1], "pn_io_device", "pn_io_pn_io_block_type"), ["32777", "32"]);
    assert_eq!(field(&d[1], "pn_io_device", "pn_io_pn_io_order_id"), ["6ES7 123-4AB56-7CD8 "]);
    assert_eq!(field(&d[3], "pn_io_device", "pn_rt_pn_io_error_code1_pniorw"), ["176"]);
    // The answer is paired with its request by the dissector.
    assert_eq!(field(&d[1], "dcerpc", "dcerpc_dcerpc_request_in"), ["1"]);
    assert_eq!(field(&d[3], "dcerpc", "dcerpc_dcerpc_request_in"), ["3"]);
}
