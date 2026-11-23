//! The records a read returns (contract: the records clause): I&M0 and the port's real data to the
//! byte both ways, what does not fit refused by name, and both read by Wireshark's dissectors in a
//! read answer.

mod common;

use serde_json::Value;
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::record::{self, Im0, LinkState, Peer, PortDataReal, RecordError, SoftwareRevision};
use veredas_aranha::profinet::rpc::{self, Block, ByteOrder, PnioStatus, ReadRequest, Uuid};

const CONTROLLER: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0C]);
const DEVICE: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0D]);

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn im0() -> Im0 {
    Im0 {
        vendor_id: 0x1234,
        order_id: "6ES7 151-3BA23-0AB0 ".into(),
        serial_number: "  S C-ABCD12    ".into(),
        hardware_revision: 0x0103,
        software_revision: SoftwareRevision { prefix: b'V', functional: 2, bugfix: 4, internal: 9 },
        revision_counter: 0x0205,
        profile_id: 0xF600,
        profile_specific_type: 0x0304,
        version: (1, 3),
        supported: 0xF01E,
    }
}

fn peer(port_id: &str, chassis_id: &str, line_delay: u32, last: u8) -> Peer {
    Peer { port_id: port_id.into(), chassis_id: chassis_id.into(), line_delay, mac: Mac([0x02, 0x1A, 0x2B, 0x3C, 0x4D, last]) }
}

fn port(peers: Vec<Peer>) -> PortDataReal {
    PortDataReal {
        slot: 0x0102,
        subslot: 0x8001,
        own_port_id: "port-001".into(),
        peers,
        mau_type: 0x0010,
        domain_boundary: 0,
        multicast_boundary: 0,
        link_state: LinkState { port: 5, link: 1 },
        media_type: 1,
        extra: vec![],
    }
}

#[test]
fn i_m0_is_its_fields_to_the_byte() {
    let b = im0().block().unwrap();
    let want = hex("0020 0038 0100 1234")
        .into_iter()
        .chain(b"6ES7 151-3BA23-0AB0   S C-ABCD12    ".iter().copied())
        .chain(hex("0103 56020409 0205 F600 0304 0103 F01E"))
        .collect::<Vec<u8>>();
    assert_eq!(b.encode().unwrap(), want);
    assert_eq!(Im0::from_block(&b).unwrap(), im0());
    let (blocks, rest) = rpc::read_blocks(&want);
    assert_eq!((blocks.len(), rest.len()), (1, 0));
    assert_eq!(Im0::from_block(&blocks[0]).unwrap(), im0());
    // The version's two bytes in their order.
    let v = Im0 { version: (2, 1), ..im0() };
    let bytes = v.block().unwrap().encode().unwrap();
    assert_eq!(&bytes[56..58], &[2, 1]);
    assert_eq!(Im0::from_block(&rpc::read_blocks(&bytes).0[0]).unwrap().version, (2, 1));
    // I&M1 to I&M4 supported (bits 1 to 4); bit 0 and past 15 name nothing.
    let i = im0();
    assert_eq!((1..=15).filter(|&n| i.supports(n)).collect::<Vec<u8>>(), [1, 2, 3, 4, 12, 13, 14, 15]);
    let read = Im0::from_block(&rpc::read_blocks(&want).0[0]).unwrap();
    assert!(read.supports(12) && !read.supports(5));
    // The texts kept as sent, their spaces at either end with them.
    assert_eq!((read.order_id.as_str(), read.serial_number.as_str()), ("6ES7 151-3BA23-0AB0 ", "  S C-ABCD12    "));
    // A tilde, the last visible character, in each text, read and written.
    let tildes = Im0 { order_id: "~ES7 151-3BA23-0AB0~".into(), serial_number: "~ C-ABCD12     ~".into(), ..im0() };
    assert_eq!(Im0::from_block(&rpc::read_blocks(&tildes.block().unwrap().encode().unwrap()).0[0]).unwrap(), tildes);
    let all = Im0 { supported: 0xFFFF, ..im0() };
    assert_eq!((all.supports(0), all.supports(15), all.supports(16)), (false, true, false));
    assert_eq!((record::INDEX_IM0, record::IM0, record::ORDER_ID, record::SERIAL_NUMBER), (0xAFF0, 0x0020, 20, 16));

    // Written only with texts of their length, visible characters; read only from its own block.
    let text = |field: &'static str, want: &str, got: &str| Err(RecordError::Text { field, want: want.into(), got: got.into() });
    assert_eq!(Im0 { order_id: "6ES7 123-4AB56-7CD8".into(), ..im0() }.block(), text("order id", "20", "6ES7 123-4AB56-7CD8"));
    assert_eq!(Im0 { order_id: "6ES7 123-4AB56-7CD8  ".into(), ..im0() }.block(), text("order id", "20", "6ES7 123-4AB56-7CD8  "));
    assert_eq!(Im0 { order_id: "6ES7 123-4AB56-7CD8\t".into(), ..im0() }.block(), text("order id", "20", "6ES7 123-4AB56-7CD8\t"));
    assert_eq!(Im0 { order_id: "6ES7 123-4AB56-7CDé".into(), ..im0() }.block(), text("order id", "20", "6ES7 123-4AB56-7CDé"));
    assert_eq!(Im0 { serial_number: "S C-ABCD1234567".into(), ..im0() }.block(), text("serial number", "16", "S C-ABCD1234567"));
    let r = SoftwareRevision { prefix: 0x7F, ..im0().software_revision };
    assert_eq!(Im0 { software_revision: r, ..im0() }.block(), text("software revision's prefix", "1", "\u{7f}"));
    let mut bad = want.clone();
    bad[8] = 0x7F;
    let (blocks, _) = rpc::read_blocks(&bad);
    assert_eq!(Im0::from_block(&blocks[0]), Err(RecordError::Text { field: "order id", want: "20".into(), got: "\u{7f}ES7 151-3BA23-0AB0 ".into() }));
    let mut bad = want.clone();
    bad[30] = 0x00;
    assert_eq!(Im0::from_block(&rpc::read_blocks(&bad).0[0]), Err(RecordError::Text { field: "serial number", want: "16".into(), got: "  \u{0} C-ABCD12    ".into() }));
    let mut bad = want.clone();
    bad[46] = 0x19;
    let err = Im0::from_block(&rpc::read_blocks(&bad).0[0]).unwrap_err();
    assert_eq!(err, RecordError::Text { field: "software revision's prefix", want: "1".into(), got: "\u{19}".into() });
    assert_eq!(err.to_string(), "the software revision's prefix is 1 visible characters (space to tilde); \"\\u{19}\" is not");
    // The prefix's bounds both ways: space taken, under it and past tilde refused, read or written;
    // and a text byte just under space refused both ways.
    let space = Im0 { software_revision: SoftwareRevision { prefix: b' ', ..im0().software_revision }, ..im0() };
    assert_eq!(Im0::from_block(&rpc::read_blocks(&space.block().unwrap().encode().unwrap()).0[0]).unwrap(), space);
    for p in [0x1F, 0x7F] {
        let r = SoftwareRevision { prefix: p, ..im0().software_revision };
        assert!(matches!(Im0 { software_revision: r, ..im0() }.block(), Err(RecordError::Text { field: "software revision's prefix", .. })), "{p:#x}");
        let mut bad = want.clone();
        bad[46] = p;
        assert!(matches!(Im0::from_block(&rpc::read_blocks(&bad).0[0]), Err(RecordError::Text { field: "software revision's prefix", .. })), "{p:#x}");
    }
    let mut bad = want.clone();
    bad[10] = 0x1F;
    assert!(matches!(Im0::from_block(&rpc::read_blocks(&bad).0[0]), Err(RecordError::Text { field: "order id", .. })));
    assert!(matches!(Im0 { serial_number: "\u{1f} C-ABCD12      ".into(), ..im0() }.block(), Err(RecordError::Text { field: "serial number", .. })));
    // Tilde is the last visible character: read and written, in every text and the prefix.
    let tilde = Im0 { software_revision: SoftwareRevision { prefix: b'~', ..im0().software_revision }, ..im0() };
    assert_eq!(Im0::from_block(&rpc::read_blocks(&tilde.block().unwrap().encode().unwrap()).0[0]).unwrap(), tilde);
    // Another version or type, by either byte.
    for (t, v) in [(0x0020, (2, 0)), (0x0020, (0, 0)), (0x0120, (1, 0)), (0x8020, (1, 0))] {
        assert_eq!(Im0::from_block(&Block { block_type: t, version: v, ..b.clone() }), Err(RecordError::Kind { what: "I&M0", want: 0x0020, got: t, high: v.0, low: v.1 }), "{t:#x} {v:?}");
    }
    assert_eq!(
        Im0::from_block(&Block { version: (1, 1), ..b.clone() }),
        Err(RecordError::Kind { what: "I&M0", want: 0x0020, got: 0x0020, high: 1, low: 1 })
    );
    assert_eq!(Im0::from_block(&Block { block_type: 0x0021, ..b.clone() }), Err(RecordError::Kind { what: "I&M0", want: 0x0020, got: 0x0021, high: 1, low: 0 }));
    let mut long = b.clone();
    long.body.push(0);
    assert_eq!(Im0::from_block(&long), Err(RecordError::Length { what: "I&M0", want: 54, got: 55 }));
    let mut short = b.clone();
    short.body.pop();
    assert_eq!(Im0::from_block(&short), Err(RecordError::Length { what: "I&M0", want: 54, got: 53 }));
    assert_eq!(
        RecordError::Kind { what: "I&M0", want: 0x0020, got: 0x0021, high: 1, low: 0 }.to_string(),
        "I&M0 is block 0x0020 version 1.0; this block is 0x0021 version 1.0"
    );
    assert_eq!(RecordError::Length { what: "I&M0", want: 54, got: 53 }.to_string(), "I&M0 is 54 bytes after its version; this block holds 53");
    assert_eq!(
        RecordError::Text { field: "order id", want: "20".into(), got: "x".into() }.to_string(),
        "the order id is 20 visible characters (space to tilde); \"x\" is not"
    );
}

#[test]
fn the_port_s_data_is_its_fields_aligned_to_the_byte() {
    let one = port(vec![peer("port-002", "switch-a", 10, 0x99)]);
    let want = hex(
        "020F 0048 0100 0000 0102 8001 08 706f72742d303031 01 0000 \
         08 706f72742d303032 08 7377697463682d61 0000 0000000a 021a2b3c4d99 0000 \
         0010 0000 00000000 00000000 0501 0000 00000001",
    );
    let b = one.block().unwrap();
    assert_eq!(b.encode().unwrap(), want);
    assert_eq!(PortDataReal::from_block(&b).unwrap(), one);
    // No peer: the medium 44 bytes into the block.
    let none = port(vec![]);
    let want = hex("020F 0028 0100 0000 0102 8001 08 706f72742d303031 00 0000 0010 0000 00000000 00000000 0501 0000 00000001");
    assert_eq!(none.block().unwrap().encode().unwrap(), want);
    assert_eq!(PortDataReal::from_block(&rpc::read_blocks(&want).0[0]).unwrap(), none);
    // Two peers of odd lengths, one with a cable delay; boundaries, link down, fibre; bytes after
    // the medium kept.
    let two = PortDataReal {
        subslot: 0x8002,
        own_port_id: "port-002".into(),
        peers: vec![peer("port-001", "dev~b", 0x8001_2345, 0x11), peer("port-003.x", "s~", 0x0001_0007, 0x22)],
        mau_type: 0x011E,
        domain_boundary: 0x0100_0003,
        multicast_boundary: 0x8000_0010,
        link_state: LinkState { port: 2, link: 2 },
        media_type: 0x0100_0002,
        extra: vec![0, 1, 0, 0],
        ..port(vec![])
    };
    let bytes = two.block().unwrap().encode().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    assert_eq!(&bytes[24..37], &hex("08 706f72742d303031 05 646576")[..]);
    assert_eq!(&bytes[37..52], &hex("7e62 00 80012345 021a2b3c4d11 0000")[..]);
    assert_eq!(&bytes[52..80], &hex("0a 706f72742d3030332e78 02 737e 0000 00010007 021a2b3c4d22 0000")[..]);
    assert_eq!(&bytes[80..], &hex("011E 0000 01000003 80000010 0202 0000 01000002 00010000")[..]);
    assert_eq!(&bytes[8..12], &hex("0102 8002")[..]);
    assert_eq!(PortDataReal::from_block(&rpc::read_blocks(&bytes).0[0]).unwrap(), two);
    let (cable, line) = (&two.peers[0], &two.peers[1]);
    assert_eq!((cable.cable_delay(), cable.delay_ns(), line.cable_delay(), line.delay_ns()), (true, 0x0001_2345, false, 0x0001_0007));
    assert_eq!((LinkState { port: 5, link: 1 }.up(), LinkState { port: 5, link: 2 }.up()), (true, false));
    assert_eq!((0..=8).filter(|&link| LinkState { port: 5, link }.up()).collect::<Vec<u8>>(), [1]);
    assert_eq!((record::INDEX_PORT_DATA_REAL, record::PORT_DATA_REAL), (0x802A, 0x020F));
    // The longest texts and the most peers a block holds are written and read back.
    let long = "p".repeat(255);
    let full = PortDataReal { own_port_id: long.clone(), peers: (0..100).map(|i| peer(&long, "c", i, i as u8)).collect(), ..port(vec![]) };
    let b = full.block().unwrap();
    assert_eq!(PortDataReal::from_block(&rpc::read_blocks(&b.encode().unwrap()).0[0]).unwrap(), full);
    let many = PortDataReal { peers: (0..255).map(|i| peer("p", "c", i, i as u8)).collect(), ..port(vec![]) };
    assert_eq!(PortDataReal::from_block(&many.block().unwrap()).unwrap(), many);
}

#[test]
fn what_does_not_fit_the_port_s_data_is_refused_by_name() {
    let one = port(vec![peer("port-002", "switch-a", 10, 0x99)]);
    let text = |field: &'static str, got: &str| Err(RecordError::Text { field, want: "0 to 255".into(), got: got.into() });
    let long = "p".repeat(256);
    assert_eq!(PortDataReal { own_port_id: long.clone(), ..one.clone() }.block(), text("own port id", &long));
    assert_eq!(port(vec![peer(&long, "c", 0, 0)]).block(), text("peer's port id", &long));
    assert_eq!(port(vec![peer("p", "c\u{7f}", 0, 0)]).block(), text("peer's chassis id", "c\u{7f}"));
    assert_eq!(port(vec![peer("pórt", "c", 0, 0)]).block(), text("peer's port id", "pórt"));
    assert_eq!(port((0..256).map(|i| peer("p", "c", i, 0)).collect()).block(), Err(RecordError::Peers(256)));
    assert_eq!(RecordError::Peers(256).to_string(), "the port's data holds at most 255 peers; this one has 256");
    // Tilde, the last visible character, in each text, read and written.
    let tilde = PortDataReal { own_port_id: "port~1".into(), peers: vec![peer("~", "a~b", 1, 1)], ..port(vec![]) };
    assert_eq!(PortDataReal::from_block(&tilde.block().unwrap()).unwrap(), tilde);
    // Texts read as sent, a chassis id's spaces at either end with it; a byte just under space
    // refused both ways.
    let spaced = port(vec![peer("port-002", " switch a ", 1, 1)]);
    assert_eq!(PortDataReal::from_block(&spaced.block().unwrap()).unwrap().peers[0].chassis_id, " switch a ");
    assert!(matches!(port(vec![peer("p", "c\u{1f}", 0, 0)]).block(), Err(RecordError::Text { field: "peer's chassis id", .. })));
    let mut body = one.block().unwrap().body;
    body[7] = 0x1F;
    assert!(matches!(PortDataReal::from_block(&Block::new(record::PORT_DATA_REAL, body)), Err(RecordError::Text { field: "own port id", .. })));
    // A peer's MAC, six different bytes in their order.
    let mac = PortDataReal::from_block(&one.block().unwrap()).unwrap().peers[0].mac;
    assert_eq!(mac, Mac([0x02, 0x1A, 0x2B, 0x3C, 0x4D, 0x99]));
    let b = one.block().unwrap();
    assert_eq!(
        PortDataReal::from_block(&Block { version: (1, 1), ..b.clone() }),
        Err(RecordError::Kind { what: "the port's data", want: 0x020F, got: 0x020F, high: 1, low: 1 })
    );
    assert!(matches!(PortDataReal::from_block(&Block { block_type: 0x0240, ..b.clone() }), Err(RecordError::Kind { got: 0x0240, .. })));
    for (t, v) in [(0x020F, (2, 0)), (0x020F, (0, 0)), (0x010F, (1, 0)), (0x820F, (1, 0))] {
        assert_eq!(
            PortDataReal::from_block(&Block { block_type: t, version: v, ..b.clone() }),
            Err(RecordError::Kind { what: "the port's data", want: 0x020F, got: t, high: v.0, low: v.1 }),
            "{t:#x} {v:?}"
        );
    }
    // Each cut of the body ends before a named field.
    let mut names = Vec::new();
    for cut in 0..b.body.len() {
        let short = Block { body: b.body[..cut].to_vec(), ..b.clone() };
        match PortDataReal::from_block(&short) {
            Err(RecordError::CutShort(what)) => names.push(what),
            other => panic!("cut at {cut}: {other:?}"),
        }
    }
    names.dedup();
    assert_eq!(
        names,
        [
            "slot",
            "subslot",
            "own port id",
            "number of peers",
            "first peer",
            "peer's port id",
            "peer's chassis id",
            "peer's line delay",
            "peer's MAC",
            "next peer",
            "MAU type",
            "domain boundary",
            "multicast boundary",
            "link state",
            "media type"
        ]
    );
    // Padding that is not zero, each place.
    let pads = [(0, "slot"), (1, "slot"), (16, "first peer"), (17, "first peer"), (36, "peer's line delay"), (48, "next peer"), (52, "domain boundary"), (64, "media type")];
    for (at, what) in pads {
        let mut body = b.body.clone();
        assert_eq!(body[at], 0, "{at}");
        body[at] = 1;
        assert_eq!(PortDataReal::from_block(&Block { body, ..b.clone() }), Err(RecordError::Padding(what)), "{at}");
    }
    // Text that is not visible characters.
    let mut body = b.body.clone();
    body[7] = 0x80;
    assert_eq!(PortDataReal::from_block(&Block { body, ..b.clone() }), Err(RecordError::Text { field: "own port id", want: "0 to 255".into(), got: "\u{fffd}ort-001".into() }));
    assert_eq!(RecordError::CutShort("MAU type").to_string(), "the port's data ends before its MAU type");
    assert_eq!(RecordError::Padding("slot").to_string(), "the port's data has padding that is not zero before its slot");
}

fn checksum(b: &[u8]) -> u16 {
    let mut s: u32 = b.chunks(2).map(|c| u32::from(u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))).sum();
    while s > 0xFFFF {
        s = (s >> 16) + (s & 0xFFFF);
    }
    !(s as u16)
}

/// The datagram in UDP in IPv4 in Ethernet.
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
    [&dst.0.0[..], &src.0.0[..], &[0x08, 0x00], &ip, &u].concat()
}

fn field(d: &common::Dissected, key: &str) -> Vec<String> {
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
    walk(&d.layers["pn_io_device"], &format!("pn_io_pn_io_{key}"), &mut out);
    out
}

#[test]
fn the_dissectors_read_both_records_in_a_read_answer() {
    let me = (CONTROLLER, [10, 0, 0, 1], 50_000);
    let dev = (DEVICE, [10, 0, 0, 2], rpc::PNIO_PORT);
    let object = Uuid::pnio_object(1, 0x0203, 0x002A);
    let activity: Uuid = "12345678-1234-5678-9abc-def012345678".parse().unwrap();
    let two = PortDataReal {
        subslot: 0x8002,
        own_port_id: "port-002".into(),
        peers: vec![peer("port-001", "dev~b", 0x8001_2345, 0x11), peer("port-003.x", "s~", 0x0001_0007, 0x22)],
        mau_type: 0x011E,
        domain_boundary: 0x0100_0003,
        multicast_boundary: 0x8000_0010,
        link_state: LinkState { port: 2, link: 2 },
        media_type: 0x0100_0002,
        extra: vec![],
        ..port(vec![])
    };
    let records: Vec<(ReadRequest, Vec<u8>)> = vec![
        (ReadRequest::implicit(1, 0, 1, record::INDEX_IM0, 4032), im0().block().unwrap().encode().unwrap()),
        (ReadRequest::implicit(2, 7, 0x8001, record::INDEX_PORT_DATA_REAL, 4032), port(vec![]).block().unwrap().encode().unwrap()),
        (ReadRequest::implicit(3, 7, 0x8001, record::INDEX_PORT_DATA_REAL, 4032), port(vec![peer("port-002", "switch-a", 10, 0x99)]).block().unwrap().encode().unwrap()),
        (ReadRequest::implicit(4, 7, 0x8002, record::INDEX_PORT_DATA_REAL, 4032), two.block().unwrap().encode().unwrap()),
    ];
    let mut frames = Vec::new();
    for (i, (read, rec)) in records.iter().enumerate() {
        let request = rpc::read_implicit(ByteOrder::Little, object, activity, i as u32, read, 4096).unwrap();
        let answer = rpc::read_answer(&request, read, PnioStatus::OK, rec, 0).unwrap();
        frames.push(udp(dev, me, &request.encode().unwrap()));
        frames.push(udp(me, dev, &answer.encode().unwrap()));
    }
    let d = common::dissect(&frames);
    for (i, p) in d.iter().enumerate() {
        assert!(p.complaints().is_empty(), "frame {i}: {:?}", p.complaints());
    }
    let im = &d[1];
    let i = im0();
    let one = |d: &common::Dissected, k: &str| {
        let v = field(d, k);
        assert_eq!(v.len(), 1, "{k}: {v:?}");
        v[0].clone()
    };
    assert_eq!(field(im, "block_type"), ["32777", "32"]);
    assert_eq!((one(im, "vendor_id_high"), one(im, "vendor_id_low")), ("18".into(), "52".into()));
    assert_eq!(one(im, "order_id"), i.order_id);
    assert_eq!(one(im, "im_serial_number"), i.serial_number);
    assert_eq!(one(im, "im_hardware_revision"), "259");
    assert_eq!(one(im, "im_revision_prefix"), "'V'");
    assert_eq!(one(im, "im_sw_revision_functional_enhancement"), "2");
    assert_eq!(one(im, "im_revision_bugfix"), "4");
    assert_eq!(one(im, "im_sw_revision_internal_change"), "9");
    assert_eq!(one(im, "im_revision_counter"), "517");
    assert_eq!((one(im, "im_profile_id"), one(im, "im_profile_specific_type")), ("62976".into(), "772".into()));
    assert_eq!((one(im, "im_version_major"), one(im, "im_version_minor")), ("1".into(), "3".into()));
    assert_eq!(one(im, "im_supported"), "61470");

    let ports = [port(vec![]), port(vec![peer("port-002", "switch-a", 10, 0x99)]), two];
    for (k, pd) in ports.iter().enumerate() {
        let p = &d[3 + 2 * k];
        assert_eq!(field(p, "block_type"), ["32777", "527"], "port {k}");
        assert_eq!(field(p, "subslot_nr"), [pd.subslot.to_string(), pd.subslot.to_string()], "port {k}");
        // The read header's slot is the request's (7), the record's its own (258).
        assert_eq!(field(p, "slot_nr"), ["7".to_string(), pd.slot.to_string()], "port {k}");
        assert_eq!(one(p, "own_port_id"), pd.own_port_id, "port {k}");
        assert_eq!(one(p, "length_own_port_id"), pd.own_port_id.len().to_string(), "port {k}");
        assert_eq!(one(p, "number_of_peers"), pd.peers.len().to_string(), "port {k}");
        let each = |f: fn(&Peer) -> String| pd.peers.iter().map(f).collect::<Vec<String>>();
        assert_eq!(field(p, "peer_port_id"), each(|x| x.port_id.clone()), "port {k}");
        assert_eq!(field(p, "length_peer_port_id"), each(|x| x.port_id.len().to_string()), "port {k}");
        assert_eq!(field(p, "peer_chassis_id"), each(|x| x.chassis_id.clone()), "port {k}");
        assert_eq!(field(p, "length_peer_chassis_id"), each(|x| x.chassis_id.len().to_string()), "port {k}");
        assert_eq!(field(p, "line_delay"), each(|x| x.line_delay.to_string()), "port {k}");
        assert_eq!(field(p, "line_delay_format_indicator"), each(|x| u8::from(x.cable_delay()).to_string()), "port {k}");
        let line: Vec<String> = pd.peers.iter().filter(|x| !x.cable_delay()).map(|x| x.delay_ns().to_string()).collect();
        let cable: Vec<String> = pd.peers.iter().filter(|x| x.cable_delay()).map(|x| x.delay_ns().to_string()).collect();
        assert_eq!((field(p, "line_delay_value"), field(p, "cable_delay_value")), (line, cable), "port {k}");
        assert_eq!(field(p, "peer_macadd"), each(|x| x.mac.to_string()), "port {k}");
        assert_eq!(one(p, "mau_type"), pd.mau_type.to_string(), "port {k}");
        assert_eq!(one(p, "domain_boundary"), pd.domain_boundary.to_string(), "port {k}");
        assert_eq!(one(p, "multicast_boundary"), pd.multicast_boundary.to_string(), "port {k}");
        assert_eq!((one(p, "link_state_port"), one(p, "link_state_link")), (pd.link_state.port.to_string(), pd.link_state.link.to_string()), "port {k}");
        assert_eq!(one(p, "media_type"), pd.media_type.to_string(), "port {k}");
    }
}
