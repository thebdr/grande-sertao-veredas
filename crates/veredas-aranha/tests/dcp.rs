//! DCP, PROFINET's discovery and basic configuration protocol (contract: the DCP clause). Every
//! request and answer the identify and configure paths use, as exact bytes both ways; damage kept
//! with its reason; total on any input; every frame read back by Wireshark's dissectors.

mod common;

use std::net::Ipv4Addr;

use veredas_aranha::profinet::dcp::{
    self, Block, BlockBody, Damage, DcpError, DcpPdu, DeviceIdentity, Filter, IpStatus, IpSuite, NameError, Service, ServiceType,
};
use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::mac::Mac;

const DEV: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
const ME: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xAA]);

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

fn suite() -> IpSuite {
    IpSuite { ip: ip(192, 168, 10, 11), mask: ip(255, 255, 255, 0), gateway: ip(192, 168, 10, 1) }
}

/// Every field an answer can say, none of them a real vendor's.
fn full_identity() -> DeviceIdentity {
    DeviceIdentity {
        name: Some("station-a".into()),
        alias: Some("port-002.station-b".into()),
        ip: Some(suite()),
        ip_status: Some(IpStatus { set: 1, conflict: false }),
        vendor_id: Some(0xFEDC),
        device_id: Some(0x0123),
        vendor: Some("bench io module".into()),
        role: Some(0x01),
        instance: Some((0, 1)),
        oem: Some((0xFEDB, 0x0456)),
        options: vec![(1, 1), (1, 2), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (2, 7), (5, 3), (5, 4)],
        mac: Some(DEV),
        initiative: Some(1),
    }
}

/// The PROFINET payload of a frame, its envelope checked.
fn payload(frame: &[u8], dst: Mac, src: Mac) -> Vec<u8> {
    let e = ether::parse(frame).unwrap();
    assert_eq!((e.dst, e.src, e.vlan, e.ethertype), (dst, src, None, 0x8892));
    assert!(frame.len() >= 60);
    e.payload.to_vec()
}

fn padded(mut v: Vec<u8>) -> Vec<u8> {
    v.resize(v.len().max(46), 0);
    v
}

#[test]
fn an_identify_request_is_the_canonical_frame_for_each_filter() {
    let all = dcp::identify_request(ME, 0x1234_5678, 1, &Filter::All).unwrap();
    assert_eq!(payload(&all, Mac::DCP_MULTICAST, ME), padded(vec![0xFE, 0xFE, 5, 0, 0x12, 0x34, 0x56, 0x78, 0, 1, 0, 4, 0xFF, 0xFF, 0, 0]));

    // An odd name is followed by one pad byte its block length does not count.
    let by_name = dcp::identify_request(ME, 7, 0x0190, &Filter::Name("station-a".into())).unwrap();
    let mut want = vec![0xFE, 0xFE, 5, 0, 0, 0, 0, 7, 0x01, 0x90, 0, 14, 2, 2, 0, 9];
    want.extend_from_slice(b"station-a");
    want.push(0);
    assert_eq!(payload(&by_name, Mac::DCP_MULTICAST, ME), padded(want));

    let by_alias = dcp::identify_request(ME, 7, 1, &Filter::Alias("port-001.station-b".into())).unwrap();
    let mut want = vec![0xFE, 0xFE, 5, 0, 0, 0, 0, 7, 0, 1, 0, 22, 2, 6, 0, 18];
    want.extend_from_slice(b"port-001.station-b");
    assert_eq!(payload(&by_alias, Mac::DCP_MULTICAST, ME), padded(want));

    for (frame, body) in [
        (&all, BlockBody::AllSelector),
        (&by_name, BlockBody::NameOfStation { name: "station-a".into() }),
        (&by_alias, BlockBody::AliasName { name: "port-001.station-b".into() }),
    ] {
        let pdu = dcp::decode(ether::parse(frame).unwrap().payload).unwrap();
        assert_eq!((pdu.frame_id, pdu.service, pdu.service_type), (dcp::FRAME_ID_IDENTIFY_REQ, Service::Identify, ServiceType::Request));
        assert_eq!(pdu.blocks.len(), 1);
        assert_eq!(pdu.blocks[0].info, None, "a filter carries no BlockInfo");
        assert_eq!(pdu.blocks[0].body, body);
        assert!(pdu.trailing.is_empty());
    }
}

#[test]
fn an_identify_answer_says_every_identity_field_and_reads_back_whole() {
    let id = full_identity();
    let f = dcp::identify_response(ME, DEV, 0x0A0B_0C0D, &id).unwrap();
    let p = payload(&f, ME, DEV);
    assert_eq!(&p[..10], &[0xFE, 0xFF, 5, 1, 0x0A, 0x0B, 0x0C, 0x0D, 0, 0]);
    let pdu = dcp::decode(&p).unwrap();
    assert_eq!((pdu.service, pdu.service_type, pdu.xid, pdu.response_delay), (Service::Identify, ServiceType::Success, 0x0A0B_0C0D, 0));
    assert_eq!(DeviceIdentity::from_blocks(&pdu.blocks), id);
    let kinds: Vec<(u8, u8, Option<u16>)> = pdu.blocks.iter().map(|b| (b.option, b.suboption, b.info)).collect();
    assert_eq!(
        kinds,
        [
            (1, 1, Some(0)),
            (1, 2, Some(1)),
            (2, 1, Some(0)),
            (2, 2, Some(0)),
            (2, 3, Some(0)),
            (2, 4, Some(0)),
            (2, 5, Some(0)),
            (2, 6, Some(0)),
            (2, 7, Some(0)),
            (2, 8, Some(0)),
            (6, 1, Some(0))
        ]
    );
    assert!(pdu.trailing.is_empty());
    // The whole PDU re-encodes to the same bytes it was read from.
    let data_len = usize::from(u16::from_be_bytes([p[10], p[11]]));
    assert_eq!(pdu.encode().unwrap(), &p[..12 + data_len]);
    assert_eq!(dcp::role_names(0x01), ["io-device"]);
    assert_eq!(dcp::role_names(0x0F), ["io-device", "io-controller", "io-multidevice", "pn-supervisor"]);
    assert!(dcp::role_names(0xF0).is_empty());

    // The smallest answer, to the byte: an odd block (2 + 9) padded to even.
    let name_only = DeviceIdentity { name: Some("station-a".into()), ..Default::default() };
    let p = payload(&dcp::identify_response(ME, DEV, 1, &name_only).unwrap(), ME, DEV);
    let mut want = vec![0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0, 0, 16, 2, 2, 0, 11, 0, 0];
    want.extend_from_slice(b"station-a");
    want.push(0);
    assert_eq!(p, padded(want));
    // An identity that says nothing is an answer with no blocks; a vendor id alone says nothing.
    let half = DeviceIdentity { vendor_id: Some(1), ..Default::default() };
    assert!(half.blocks().is_empty());
    let p = payload(&dcp::identify_response(ME, DEV, 1, &DeviceIdentity::default()).unwrap(), ME, DEV);
    assert_eq!(p, padded(vec![0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0, 0, 0]));
    assert!(DeviceIdentity::from_blocks(&dcp::decode(&p).unwrap().blocks).is_empty());
}

#[test]
fn a_hello_goes_to_its_own_group_with_block_info_and_carries_the_address_conflict() {
    assert_eq!(Mac::DCP_HELLO_MULTICAST, Mac([0x01, 0x0E, 0xCF, 0x00, 0x00, 0x01]));
    assert_eq!(Mac::DCP_MULTICAST, Mac([0x01, 0x0E, 0xCF, 0x00, 0x00, 0x00]));
    let mut id = full_identity();
    for (info, status) in [
        (0x0000, IpStatus { set: 0, conflict: false }),
        (0x0001, IpStatus { set: 1, conflict: false }),
        (0x0002, IpStatus { set: 2, conflict: false }),
        (0x0080, IpStatus { set: 0, conflict: true }),
        (0x0081, IpStatus { set: 1, conflict: true }),
        (0x0082, IpStatus { set: 2, conflict: true }),
    ] {
        assert_eq!(IpStatus::from_info(info), status);
        assert_eq!(status.to_info(), info);
        id.ip_status = Some(status);
        let f = dcp::hello(DEV, 3, &id).unwrap();
        let pdu = dcp::decode(&payload(&f, Mac::DCP_HELLO_MULTICAST, DEV)).unwrap();
        assert_eq!((pdu.frame_id, pdu.service, pdu.service_type), (dcp::FRAME_ID_HELLO, Service::Hello, ServiceType::Request));
        let ipb = pdu.blocks.iter().find(|b| (b.option, b.suboption) == (1, 2)).unwrap();
        assert_eq!(ipb.info, Some(info));
        assert_eq!(DeviceIdentity::from_blocks(&pdu.blocks).ip_status, Some(status));
    }
    // An address with no status given is said to be set, with no conflict.
    id.ip_status = None;
    let pdu = dcp::decode(&payload(&dcp::hello(DEV, 3, &id).unwrap(), Mac::DCP_HELLO_MULTICAST, DEV)).unwrap();
    assert_eq!(DeviceIdentity::from_blocks(&pdu.blocks).ip_status, Some(IpStatus { set: 1, conflict: false }));
}

#[test]
fn a_get_request_is_bare_pairs_and_its_answer_carries_block_info() {
    let f = dcp::get_request(DEV, ME, 9, &[(2, 2), (1, 2)]).unwrap();
    let p = payload(&f, DEV, ME);
    assert_eq!(p, padded(vec![0xFE, 0xFD, 3, 0, 0, 0, 0, 9, 0, 0, 0, 4, 2, 2, 1, 2]));
    let pdu = dcp::decode(&p).unwrap();
    let pairs: Vec<(u8, u8, Option<u16>, BlockBody)> = pdu.blocks.iter().map(|b| (b.option, b.suboption, b.info, b.body.clone())).collect();
    assert_eq!(pairs, [(2, 2, None, BlockBody::Requested), (1, 2, None, BlockBody::Requested)]);
    // An odd last byte is kept, not dropped.
    let odd = [0xFE, 0xFD, 3, 0, 0, 0, 0, 9, 0, 0, 0, 3, 2, 2, 1];
    let pdu = dcp::decode(&odd).unwrap();
    assert_eq!((pdu.blocks.len(), pdu.trailing.clone()), (1, vec![1]));
    assert_eq!(pdu.encode().unwrap(), odd);

    // The answer: the name with its BlockInfo, and a control response for what it lacks.
    let answer = DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Get,
        service_type: ServiceType::Success,
        xid: 9,
        response_delay: 0,
        blocks: vec![
            Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "station-a".into() } },
            Block { option: 5, suboption: 4, info: None, body: BlockBody::ControlResponse { option: 1, suboption: 2, error: 3 } },
        ],
        trailing: Vec::new(),
    };
    let bytes = answer.encode().unwrap();
    let mut want = vec![0xFE, 0xFD, 3, 1, 0, 0, 0, 9, 0, 0, 0, 24, 2, 2, 0, 11, 0, 0];
    want.extend_from_slice(b"station-a");
    want.extend_from_slice(&[0, 5, 4, 0, 3, 1, 2, 3, 0]);
    assert_eq!(bytes, want);
    assert_eq!(dcp::decode(&bytes).unwrap(), answer);
    assert_eq!(dcp::control_responses(&answer), [(1, 2, 3)]);
}

#[test]
fn a_set_is_written_to_one_device_with_its_qualifier() {
    let f = dcp::set_name_request(DEV, ME, 0x0102_0304, "station-b", true).unwrap();
    let mut want = vec![0xFE, 0xFD, 4, 0, 1, 2, 3, 4, 0, 0, 0, 16, 2, 2, 0, 11, 0, 1];
    want.extend_from_slice(b"station-b");
    want.push(0);
    assert_eq!(payload(&f, DEV, ME), padded(want));

    let f = dcp::set_name_request(DEV, ME, 1, "s1", false).unwrap();
    let pdu = dcp::decode(&payload(&f, DEV, ME)).unwrap();
    assert_eq!(pdu.blocks, [Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "s1".into() } }]);

    let f = dcp::set_ip_request(DEV, ME, 2, &suite(), true).unwrap();
    let want = vec![0xFE, 0xFD, 4, 0, 0, 0, 0, 2, 0, 0, 0, 18, 1, 2, 0, 14, 0, 1, 192, 168, 10, 11, 255, 255, 255, 0, 192, 168, 10, 1];
    assert_eq!(payload(&f, DEV, ME), padded(want));

    let f = dcp::signal_request(DEV, ME, 3).unwrap();
    let want = vec![0xFE, 0xFD, 4, 0, 0, 0, 0, 3, 0, 0, 0, 8, 5, 3, 0, 4, 0, 0, 1, 0];
    assert_eq!(payload(&f, DEV, ME), padded(want));
    let pdu = dcp::decode(&payload(&f, DEV, ME)).unwrap();
    assert_eq!(pdu.blocks[0].body, BlockBody::ControlSignal { value: dcp::SIGNAL_FLASH });
}

#[test]
fn a_set_to_a_group_address_is_refused_by_the_codec() {
    for target in [Mac::DCP_MULTICAST, Mac::BROADCAST, Mac::LLDP_MULTICAST, Mac([0x03, 0, 0, 0, 0, 0])] {
        assert_eq!(dcp::set_name_request(target, ME, 1, "station-a", true), Err(DcpError::GroupTarget(target)));
        assert_eq!(dcp::set_ip_request(target, ME, 1, &suite(), true), Err(DcpError::GroupTarget(target)));
        assert_eq!(dcp::signal_request(target, ME, 1), Err(DcpError::GroupTarget(target)));
    }
    assert_eq!(
        DcpError::GroupTarget(Mac::BROADCAST).to_string(),
        "a DCP Set names one device by its MAC; ff:ff:ff:ff:ff:ff is a group address"
    );
    // A name that breaks the rules is refused before anything is written.
    assert_eq!(dcp::set_name_request(DEV, ME, 1, "Station", true), Err(DcpError::Name(NameError::Character('S'))));
}

/// A decoded PDU written again: a Set request through `frame` to one device (the one way it is
/// written), anything else through `encode`; the PROFINET payload either way.
fn rewrite(pdu: &DcpPdu) -> Result<Vec<u8>, DcpError> {
    if pdu.is_set_request() || pdu.is_hello() {
        let dst = if pdu.is_hello() { Mac::DCP_HELLO_MULTICAST } else { DEV };
        let f = pdu.frame(dst, ME)?;
        let p = ether::parse(&f).unwrap().payload;
        let len = 12 + usize::from(u16::from_be_bytes([p[10], p[11]]));
        Ok(p[..len].to_vec())
    } else {
        pdu.encode()
    }
}

/// A Set request holding these blocks, as a caller could build or decode one by hand.
fn set_pdu(blocks: Vec<Block>) -> DcpPdu {
    DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Set,
        service_type: ServiceType::Request,
        xid: 1,
        response_delay: 0,
        blocks,
        trailing: Vec::new(),
    }
}

#[test]
fn a_set_built_or_decoded_by_hand_is_gated_as_the_helpers_are() {
    let name = |n: &str, q: Option<u16>| Block { option: 2, suboption: 2, info: q, body: BlockBody::NameOfStation { name: n.into() } };
    let ip = |q| Block { option: 1, suboption: 2, info: q, body: BlockBody::IpParameter { suite: suite() } };
    let signal = |q| Block { option: 5, suboption: 3, info: q, body: BlockBody::ControlSignal { value: dcp::SIGNAL_FLASH } };
    // What may be written: to one device, a good name, an IP suite and the signal, each with a
    // qualifier it may carry.
    for blocks in [vec![name("station-a", Some(1))], vec![name("station-a", Some(0)), ip(Some(1))], vec![ip(Some(0)), signal(Some(0))]] {
        let pdu = set_pdu(blocks);
        assert!(pdu.frame(DEV, ME).is_ok(), "{pdu:?}");
        for group in [Mac::BROADCAST, Mac::DCP_MULTICAST, Mac::DCP_HELLO_MULTICAST, Mac([0x03, 0, 0, 0, 0, 9])] {
            assert_eq!(pdu.frame(group, ME), Err(DcpError::GroupTarget(group)));
        }
    }
    // What a Set can do beyond them is not expressible: a reset to factory (5, 6), a factory
    // reset (5, 5), a transaction's start and end, a reset as unread bytes, a name given as unread
    // bytes, a damaged block, another kind of block.
    let reset = Block { option: 5, suboption: 6, info: Some(2), body: BlockBody::ResetToFactory };
    let control = |suboption, body| Block { option: 5, suboption, info: Some(0), body };
    let cases = vec![
        (reset.clone(), DcpError::SetBlock { option: 5, suboption: 6, body: "reset-to-factory" }),
        (control(5, BlockBody::FactoryReset), DcpError::SetBlock { option: 5, suboption: 5, body: "factory-reset" }),
        (control(1, BlockBody::StartTransaction), DcpError::SetBlock { option: 5, suboption: 1, body: "start-transaction" }),
        (control(2, BlockBody::EndTransaction), DcpError::SetBlock { option: 5, suboption: 2, body: "end-transaction" }),
        (Block { option: 5, suboption: 6, info: Some(2), body: BlockBody::Unknown { data: vec![] } }, DcpError::SetBlock { option: 5, suboption: 6, body: "unknown" }),
        (Block { option: 5, suboption: 5, info: Some(0), body: BlockBody::Unknown { data: vec![] } }, DcpError::SetBlock { option: 5, suboption: 5, body: "unknown" }),
        (Block { option: 2, suboption: 2, info: Some(1), body: BlockBody::Unknown { data: b"station-a".to_vec() } }, DcpError::SetBlock { option: 2, suboption: 2, body: "unknown" }),
        (Block { option: 2, suboption: 2, info: Some(1), body: BlockBody::Damaged { data: vec![0xFF], damage: Damage::Text } }, DcpError::SetBlock { option: 2, suboption: 2, body: "damaged" }),
        (Block { option: 2, suboption: 1, info: Some(1), body: BlockBody::DeviceVendor { value: "v".into() } }, DcpError::SetBlock { option: 2, suboption: 1, body: "device-vendor" }),
        (Block { option: 2, suboption: 6, info: Some(1), body: BlockBody::NameOfStation { name: "station-a".into() } }, DcpError::SetBlock { option: 2, suboption: 6, body: "name-of-station" }),
        (name("Port-001", Some(1)), DcpError::Name(NameError::Character('P'))),
        (name("port-001", Some(1)), DcpError::Name(NameError::PortName)),
        (name("station-a", Some(2)), DcpError::Qualifier { option: 2, suboption: 2, qualifier: Some(2), allowed: "0 or 1" }),
        (name("station-a", None), DcpError::Qualifier { option: 2, suboption: 2, qualifier: None, allowed: "0 or 1" }),
        (ip(Some(3)), DcpError::Qualifier { option: 1, suboption: 2, qualifier: Some(3), allowed: "0 or 1" }),
        (signal(Some(1)), DcpError::Qualifier { option: 5, suboption: 3, qualifier: Some(1), allowed: "0" }),
    ];
    assert_eq!(cases.len(), 16);
    for (block, err) in cases {
        let pdu = set_pdu(vec![name("station-a", Some(1)), block.clone()]);
        assert_eq!(pdu.encode(), Err(DcpError::NeedsTarget("Set request")), "{block:?}");
        assert_eq!(pdu.frame(DEV, ME), Err(err), "{block:?}");
    }
    // A Set read from the wire (here a factory reset with a broken name, broadcast) decodes, so
    // the scanner can see it, and cannot be written again, to one device or to all.
    let heard = [0xFE, 0xFD, 4, 0, 0, 0, 0, 1, 0, 0, 0, 20, 2, 2, 0, 10, 0, 1, b'P', b'o', b'r', b't', b'-', b'0', b'0', b'1', 5, 6, 0, 2, 0, 2];
    let pdu = dcp::decode(&heard).unwrap();
    assert!(pdu.is_set_request());
    assert_eq!(pdu.blocks[1], Block { option: 5, suboption: 6, info: Some(2), body: BlockBody::ResetToFactory });
    assert_eq!(pdu.frame(DEV, ME), Err(DcpError::Name(NameError::Character('P'))));
    assert_eq!(pdu.encode(), Err(DcpError::NeedsTarget("Set request")));
    assert_eq!(pdu.frame(Mac::BROADCAST, ME), Err(DcpError::GroupTarget(Mac::BROADCAST)));
    let renamed = DcpPdu { blocks: vec![name("station-a", Some(1)), pdu.blocks[1].clone()], ..pdu };
    assert_eq!(renamed.frame(DEV, ME), Err(DcpError::SetBlock { option: 5, suboption: 6, body: "reset-to-factory" }));
    // A signal heard with more than its value decodes damaged, and is not written again.
    let long_signal = [0xFE, 0xFD, 4, 0, 0, 0, 0, 1, 0, 0, 0, 10, 5, 3, 0, 6, 0, 0, 1, 0, 0xDE, 0xAD];
    let pdu = dcp::decode(&long_signal).unwrap();
    assert_eq!(pdu.blocks[0].body, BlockBody::Damaged { data: vec![1, 0, 0xDE, 0xAD], damage: Damage::Length { expected: 2, got: 4 } });
    assert_eq!(pdu.frame(DEV, ME), Err(DcpError::SetBlock { option: 5, suboption: 3, body: "damaged" }));
    // A Set answer is not a request: it is written to whoever asked, its blocks unread by the gate.
    let answer = DcpPdu { service_type: ServiceType::Success, ..set_pdu(vec![reset]) };
    assert!(answer.frame(ME, DEV).is_ok());
    assert_eq!(
        DcpError::SetBlock { option: 5, suboption: 6, body: "unknown" }.to_string(),
        "a Set writes a station name, an IP suite or the identification signal, never option 5 suboption 6 (unknown)"
    );
}

#[test]
fn a_station_name_follows_the_standards_rules_and_is_never_rewritten() {
    let long_label = "a".repeat(63);
    let longer_label = "a".repeat(64);
    let at_limit = format!("{}.{}.{}.{}", "a".repeat(60), "b".repeat(60), "c".repeat(60), "d".repeat(57));
    let past_limit = format!("{at_limit}x");
    assert_eq!(at_limit.len(), 240);
    let cases: Vec<(&str, Result<(), NameError>)> = vec![
        ("station-a", Ok(())),
        ("a", Ok(())),
        ("plc-1.line-2.plant", Ok(())),
        ("0", Ok(())),
        ("port-1", Ok(())),
        ("port-0012", Ok(())),
        ("port-001-1234", Ok(())),
        ("x.port-001", Ok(())),
        ("port-abc", Ok(())),
        ("port-abc-defgh", Ok(())),
        ("port-abc-12345", Ok(())),
        ("1.2.3", Ok(())),
        ("1.2.3.4.5", Ok(())),
        ("1.2.3.a", Ok(())),
        ("1234.2.3.4", Ok(())),
        (&long_label, Ok(())),
        (&at_limit, Ok(())),
        ("", Err(NameError::Length(0))),
        (&past_limit, Err(NameError::Length(241))),
        ("Station", Err(NameError::Character('S'))),
        ("station a", Err(NameError::Character(' '))),
        ("station_a", Err(NameError::Character('_'))),
        ("estação", Err(NameError::Character('ç'))),
        ("a..b", Err(NameError::EmptyLabel(1))),
        (".a", Err(NameError::EmptyLabel(0))),
        ("a.", Err(NameError::EmptyLabel(1))),
        (&longer_label, Err(NameError::LabelTooLong { index: 0, len: 64 })),
        ("-a", Err(NameError::HyphenAtEdge(0))),
        ("a.b-", Err(NameError::HyphenAtEdge(1))),
        ("port-001", Err(NameError::PortName)),
        ("port-001-12345", Err(NameError::PortName)),
        ("port-001.plant", Err(NameError::PortName)),
        ("1.2.3.4", Err(NameError::AddressForm)),
        ("192.168.0.999", Err(NameError::AddressForm)),
    ];
    assert_eq!(cases.len(), 34);
    for (name, want) in cases {
        assert_eq!(dcp::check_station_name(name), want, "{name:?}");
    }
    assert_eq!(NameError::PortName.to_string(), "the first label has the form of a port name (port-xyz or port-xyz-abcde)");
}

#[test]
fn a_set_answer_is_one_three_byte_control_response_per_block() {
    let f = dcp::set_response(ME, DEV, 0x0102_0304, &[(2, 2, 0), (1, 2, 6)]).unwrap();
    let want = vec![0xFE, 0xFD, 4, 1, 1, 2, 3, 4, 0, 0, 0, 16, 5, 4, 0, 3, 2, 2, 0, 0, 5, 4, 0, 3, 1, 2, 6, 0];
    assert_eq!(payload(&f, ME, DEV), padded(want));
    let pdu = dcp::decode(&payload(&f, ME, DEV)).unwrap();
    assert_eq!(pdu.blocks.iter().map(|b| b.info).collect::<Vec<_>>(), [None, None]);
    assert_eq!(dcp::control_responses(&pdu), [(2, 2, 0), (1, 2, 6)]);
    let words: Vec<&str> = (0..=7).map(dcp::error_text).collect();
    assert_eq!(
        words,
        [
            "ok",
            "option not supported",
            "suboption not supported or no data set available",
            "suboption not set",
            "resource error",
            "set not possible for local reasons",
            "in operation, set not possible",
            "unknown error code"
        ]
    );
}

/// An answer's payload holding `blocks` (already written), for the damage cases.
fn answer_with(blocks: &[u8]) -> Vec<u8> {
    let mut p = vec![0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0];
    p.extend_from_slice(&(blocks.len() as u16).to_be_bytes());
    p.extend_from_slice(blocks);
    p
}

#[test]
fn a_damaged_block_is_kept_with_its_reason_and_the_next_still_decodes() {
    let name = [2, 2, 0, 4, 0, 0, b'o', b'k'];
    let cases: Vec<(Vec<u8>, BlockBody)> = vec![
        (vec![1, 1, 0, 6, 0, 0, 1, 2, 3, 4], BlockBody::Damaged { data: vec![1, 2, 3, 4], damage: Damage::Length { expected: 6, got: 4 } }),
        (vec![1, 2, 0, 4, 0, 0, 1, 2], BlockBody::Damaged { data: vec![1, 2], damage: Damage::Length { expected: 12, got: 2 } }),
        (vec![1, 3, 0, 14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], BlockBody::Damaged { data: vec![0; 12], damage: Damage::Length { expected: 28, got: 12 } }),
        (vec![2, 3, 0, 4, 0, 0, 0, 42], BlockBody::Damaged { data: vec![0, 42], damage: Damage::Length { expected: 4, got: 2 } }),
        (vec![2, 4, 0, 3, 0, 0, 1, 0], BlockBody::Damaged { data: vec![1], damage: Damage::Length { expected: 2, got: 1 } }),
        (vec![2, 5, 0, 5, 0, 0, 1, 2, 3, 0], BlockBody::Damaged { data: vec![1, 2, 3], damage: Damage::Length { expected: 4, got: 3 } }),
        (vec![2, 7, 0, 5, 0, 0, 1, 2, 3, 0], BlockBody::Damaged { data: vec![1, 2, 3], damage: Damage::Length { expected: 2, got: 3 } }),
        (vec![2, 8, 0, 2, 0, 0], BlockBody::Damaged { data: vec![], damage: Damage::Length { expected: 4, got: 0 } }),
        (vec![2, 10, 0, 3, 0, 0, 1, 0], BlockBody::Damaged { data: vec![1], damage: Damage::Length { expected: 2, got: 1 } }),
        (vec![5, 3, 0, 4, 0, 0, 1, 0], BlockBody::ControlSignal { value: 0x0100 }),
        (vec![5, 3, 0, 6, 0, 0, 1, 0, 0xDE, 0xAD], BlockBody::Damaged { data: vec![1, 0, 0xDE, 0xAD], damage: Damage::Length { expected: 2, got: 4 } }),
        (vec![5, 5, 0, 4, 0, 1, 2, 2], BlockBody::Damaged { data: vec![2, 2], damage: Damage::Length { expected: 0, got: 2 } }),
        (vec![0xFF, 0xFF, 0, 2, 1, 2], BlockBody::Damaged { data: vec![1, 2], damage: Damage::Length { expected: 0, got: 2 } }),
        (vec![5, 4, 0, 4, 2, 2, 0, 9], BlockBody::Damaged { data: vec![2, 2, 0, 9], damage: Damage::Length { expected: 3, got: 4 } }),
        (vec![6, 1, 0, 3, 0, 0, 1, 0], BlockBody::Damaged { data: vec![1], damage: Damage::Length { expected: 2, got: 1 } }),
        (vec![2, 2, 0, 1, 7, 0], BlockBody::Damaged { data: vec![7], damage: Damage::NoInfo }),
        (vec![2, 2, 0, 0], BlockBody::Damaged { data: vec![], damage: Damage::NoInfo }),
        (vec![2, 5, 0, 0], BlockBody::Damaged { data: vec![], damage: Damage::NoInfo }),
        (vec![2, 4, 0, 4, 0, 0, 1, 0xFF], BlockBody::DeviceRole { role: 1, reserved: 0xFF }),
        (vec![9, 9, 0, 3, 0, 0, 1, 0], BlockBody::Unknown { data: vec![1] }),
        (vec![2, 2, 0, 4, 0, 0, 0xFF, 0xFE], BlockBody::Damaged { data: vec![0xFF, 0xFE], damage: Damage::Text }),
        (vec![2, 1, 0, 3, 0, 0, 0xC3, 0], BlockBody::Damaged { data: vec![0xC3], damage: Damage::Text }),
        (vec![2, 6, 0, 5, 0, 0, b'a', 0xE2, 0x82, 0], BlockBody::Damaged { data: vec![b'a', 0xE2, 0x82], damage: Damage::Text }),
    ];
    let damaged = cases.iter().filter(|(_, b)| matches!(b, BlockBody::Damaged { .. })).count();
    assert_eq!((damaged, cases.len()), (20, 23), "twenty damaged blocks, three good ones of the same shapes");
    for (block, body) in cases {
        let mut blocks = block.clone();
        blocks.extend_from_slice(&name);
        let pdu = dcp::decode(&answer_with(&blocks)).unwrap();
        assert_eq!(pdu.blocks.len(), 2, "{block:?}");
        assert_eq!(pdu.blocks[0].body, body, "{block:?}");
        assert_eq!(pdu.blocks[1].body, BlockBody::NameOfStation { name: "ok".into() }, "{block:?}");
        assert!(pdu.trailing.is_empty(), "{block:?}");
    }

    // A block running past the data stops the decoding there, what came before kept.
    let mut blocks = name.to_vec();
    blocks.extend_from_slice(&[2, 1, 0, 40, 0, 0, b'x']);
    let pdu = dcp::decode(&answer_with(&blocks)).unwrap();
    assert_eq!(pdu.blocks[0].body, BlockBody::NameOfStation { name: "ok".into() });
    assert_eq!(
        pdu.blocks[1],
        Block { option: 2, suboption: 1, info: None, body: BlockBody::Damaged { data: vec![0, 0, b'x'], damage: Damage::Overrun { declared: 40, left: 3 } } }
    );
    // Fewer than a block header's four bytes at the end are kept as trailing bytes.
    let mut blocks = name.to_vec();
    blocks.extend_from_slice(&[2, 2, 0]);
    let pdu = dcp::decode(&answer_with(&blocks)).unwrap();
    assert_eq!((pdu.blocks.len(), pdu.trailing.clone()), (1, vec![2, 2, 0]));
    // One and two bytes too, each kept as it came, never dropped unseen.
    for tail in [vec![0x5A], vec![0x5A, 0xA5]] {
        let mut blocks = name.to_vec();
        blocks.extend_from_slice(&tail);
        let pdu = dcp::decode(&answer_with(&blocks)).unwrap();
        assert_eq!((pdu.blocks.len(), pdu.trailing.clone()), (1, tail.clone()));
        assert_eq!(pdu.encode().unwrap(), answer_with(&blocks));
    }
    // Bytes past the data length (the frame's pad) are not the PDU's.
    let mut p = answer_with(&name);
    p.extend_from_slice(&[0xAB; 30]);
    let pdu = dcp::decode(&p).unwrap();
    assert_eq!((pdu.blocks.len(), pdu.trailing.len()), (1, 0));
}

#[test]
fn a_set_answer_carries_no_block_info_whatever_its_blocks() {
    // A Set answer holding a name block (not a control response): no info word is taken from it.
    let p = [0xFE, 0xFD, 4, 1, 0, 0, 0, 1, 0, 0, 0, 8, 2, 2, 0, 4, b'a', b'b', b'c', b'd'];
    let pdu = dcp::decode(&p).unwrap();
    assert_eq!(pdu.blocks, [Block { option: 2, suboption: 2, info: None, body: BlockBody::NameOfStation { name: "abcd".into() } }]);
    assert_eq!(pdu.encode().unwrap(), p);
    let mut frame = vec![2, 0, 0, 0, 0, 0xAA, 2, 0, 0, 0, 0, 1, 0x88, 0x92];
    frame.extend_from_slice(&p);
    frame.resize(60, 0);
    let read = common::dissect(&[frame]);
    assert_eq!(read[0].complaints(), Vec::<String>::new());
    assert_eq!(read[0].one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "abcd");
    assert!(read[0].values("pn_dcp", "pn_dcp.block_info").is_empty());
}

#[test]
fn a_decoded_pdu_is_written_back_to_its_own_bytes_save_its_pads() {
    // Every frame this codec writes, decoded and written again, is the same bytes.
    let frames = [
        dcp::identify_request(ME, 1, 1, &Filter::Name("station-a".into())).unwrap(),
        dcp::identify_response(ME, DEV, 2, &full_identity()).unwrap(),
        dcp::hello(DEV, 3, &full_identity()).unwrap(),
        dcp::get_request(DEV, ME, 4, &[(2, 2), (1, 2), (2, 3)]).unwrap(),
        dcp::set_name_request(DEV, ME, 5, "station-b", true).unwrap(),
        dcp::set_ip_request(DEV, ME, 6, &suite(), false).unwrap(),
        dcp::signal_request(DEV, ME, 7).unwrap(),
        dcp::set_response(ME, DEV, 8, &[(2, 2, 0), (1, 2, 6)]).unwrap(),
    ];
    for f in &frames {
        let p = ether::parse(f).unwrap().payload;
        let len = 12 + usize::from(u16::from_be_bytes([p[10], p[11]]));
        assert_eq!(rewrite(&dcp::decode(p).unwrap()).unwrap(), &p[..len]);
    }
    // A device's reserved role byte is kept as heard.
    let role = answer_with(&[2, 4, 0, 4, 0, 0, 1, 0xFF]);
    assert_eq!(dcp::decode(&role).unwrap().encode().unwrap(), role);
    // The two bytes a decoded PDU does not keep, and what it writes in their place: a pad byte's
    // value (the standard has it 0) and the pad a last odd block went without.
    let odd = answer_with(&[2, 2, 0, 3, 0, 0, b'a', 0xAB, 2, 2, 0, 3, 0, 0, b'b']);
    let back = dcp::decode(&odd).unwrap().encode().unwrap();
    let mut want = odd.clone();
    want[19] = 0;
    want.push(0);
    want[11] = 16;
    assert_eq!(back, want);
    assert_eq!(dcp::decode(&back).unwrap(), dcp::decode(&odd).unwrap());
}

#[test]
fn a_damaged_header_is_an_error_that_names_it() {
    assert_eq!(dcp::decode(&[0xFE, 0xFF, 5, 1]), Err(DcpError::TooShort(4)));
    assert_eq!(dcp::decode(&[]), Err(DcpError::TooShort(0)));
    for id in [0x8000u16, 0xFEFB, 0xFF00, 0x0000] {
        let mut p = id.to_be_bytes().to_vec();
        p.extend_from_slice(&[5, 1, 0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(dcp::decode(&p), Err(DcpError::NotDcp(id)));
    }
    assert_eq!(dcp::decode(&[0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0]), Err(DcpError::LengthOverrun { declared: 9, available: 2 }));
    assert_eq!(DcpError::NotDcp(0x8000).to_string(), "frame id 0x8000 is not DCP's");
    assert_eq!(DcpError::LengthOverrun { declared: 9, available: 2 }.to_string(), "DCPDataLength 9 runs past the 2 bytes present");
    assert!(dcp::is_dcp_frame_id(0xFEFC) && dcp::is_dcp_frame_id(0xFEFF));
    assert!(!dcp::is_dcp_frame_id(0xFEFB) && !dcp::is_dcp_frame_id(0xFF00));
}

#[test]
fn what_one_frame_cannot_carry_is_refused_never_cut() {
    let long = DeviceIdentity { vendor: Some("v".repeat(1500)), ..Default::default() };
    assert_eq!(dcp::identify_response(ME, DEV, 1, &long), Err(DcpError::PduTooLong(1518)));
    let huge = DcpPdu {
        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks: vec![Block { option: 2, suboption: 1, info: Some(0), body: BlockBody::DeviceVendor { value: "v".repeat(65_534) } }],
        trailing: Vec::new(),
    };
    assert_eq!(huge.encode(), Err(DcpError::BlockTooLong { option: 2, suboption: 1, len: 65_536 }));
    let over = DcpPdu {
        blocks: vec![Block { option: 2, suboption: 1, info: Some(0), body: BlockBody::DeviceVendor { value: "v".repeat(40_000) } }; 2],
        ..huge.clone()
    };
    assert_eq!(over.encode(), Err(DcpError::PduTooLong(80_024)));
    // What one frame carries, to the byte: 1500 written, past it refused.
    let at = |n: usize, trailing: usize| DcpPdu {
        blocks: vec![Block { option: 2, suboption: 1, info: Some(0), body: BlockBody::DeviceVendor { value: "v".repeat(n) } }],
        trailing: vec![0; trailing],
        ..huge.clone()
    };
    assert_eq!(at(1482, 0).encode().unwrap().len(), 1500);
    assert_eq!(at(1482, 1).encode(), Err(DcpError::PduTooLong(1501)));
    assert_eq!(at(1482, 0).frame(ME, DEV).unwrap().len(), 1514);
    assert_eq!(DcpError::PduTooLong(1501).to_string(), "the PDU makes 1501 bytes, past the 1500 one frame carries");
}

#[test]
fn any_bytes_decode_without_a_panic_and_rewrite_to_themselves() {
    let mut seeds = vec![
        payload(&dcp::identify_response(ME, DEV, 1, &full_identity()).unwrap(), ME, DEV),
        payload(&dcp::identify_request(ME, 1, 1, &Filter::Name("station-a".into())).unwrap(), Mac::DCP_MULTICAST, ME),
        payload(&dcp::set_response(ME, DEV, 1, &[(2, 2, 0)]).unwrap(), ME, DEV),
        payload(&dcp::get_request(DEV, ME, 1, &[(2, 2)]).unwrap(), DEV, ME),
    ];
    let mut noise = common::Noise(0x5EED);
    for _ in 0..20_000 {
        let len = usize::from(noise.next()) % 80;
        let mut p = noise.bytes(len);
        if p.len() >= 4 {
            p[0] = 0xFE;
            p[1] = 0xFC + noise.next() % 4;
            p[2] = 3 + noise.next() % 4;
            p[3] = [0, 1, 5][usize::from(noise.next()) % 3];
        }
        if p.len() >= 12 {
            let n = (p.len() - 12) as u16;
            p[10..12].copy_from_slice(&n.to_be_bytes());
        }
        seeds.push(p);
    }
    let (mut decoded, mut overruns, mut gated) = (0, 0, 0);
    for s in &seeds {
        for cut in 0..=s.len() {
            // The data length restated for the cut, so every cut reaches the block decoder.
            let mut c = s[..cut].to_vec();
            if c.len() >= 12 {
                let n = (c.len() - 12) as u16;
                c[10..12].copy_from_slice(&n.to_be_bytes());
            }
            if let Ok(p0) = dcp::decode(&c) {
                decoded += 1;
                // A Set request outside what may be written decodes and is never written again;
                // a PDU holding a damaged block is kept to be seen and never written.
                let bytes = match rewrite(&p0) {
                    Ok(b) => b,
                    Err(e) => {
                        let damaged = p0.blocks.iter().any(|b| matches!(b.body, BlockBody::Damaged { .. }));
                        if p0.is_set_request() {
                            assert!(
                                matches!(e, DcpError::SetBlock { .. } | DcpError::Qualifier { .. } | DcpError::Name(_) | DcpError::Signal(_) | DcpError::SetTrailing(_)),
                                "{e:?}"
                            );
                            gated += 1;
                        } else {
                            assert!(damaged && matches!(e, DcpError::Damaged { .. }), "{e:?} {c:?}");
                            overruns += 1;
                        }
                        continue;
                    }
                };
                // Everything else reads back as it was.
                assert_eq!(dcp::decode(&bytes).unwrap(), p0, "{c:?}");
            }
        }
    }
    assert!(decoded > 100_000 && overruns > 1_000 && gated > 1_000, "{decoded} decoded, {overruns} damaged and refused, {gated} Sets gated");
}

#[test]
fn the_dissectors_read_every_frame_as_it_was_written() {
    let id = full_identity();
    let frames = vec![
        dcp::identify_request(ME, 0x1234_5678, 1, &Filter::All).unwrap(),
        dcp::identify_request(ME, 2, 0x0190, &Filter::Name("station-a".into())).unwrap(),
        dcp::identify_request(ME, 3, 1, &Filter::Alias("port-001.station-b".into())).unwrap(),
        dcp::identify_response(ME, DEV, 0x0A0B_0C0D, &id).unwrap(),
        dcp::hello(DEV, 5, &DeviceIdentity { ip_status: Some(IpStatus { set: 1, conflict: true }), ..id.clone() }).unwrap(),
        dcp::get_request(DEV, ME, 6, &[(2, 2), (1, 2)]).unwrap(),
        dcp::set_name_request(DEV, ME, 7, "station-b", true).unwrap(),
        dcp::set_ip_request(DEV, ME, 8, &suite(), false).unwrap(),
        dcp::signal_request(DEV, ME, 9).unwrap(),
        dcp::set_response(ME, DEV, 10, &[(2, 2, 0), (1, 2, 6)]).unwrap(),
        DcpPdu {
            frame_id: dcp::FRAME_ID_GET_SET,
            service: Service::Get,
            service_type: ServiceType::Success,
            xid: 11,
            response_delay: 0,
            blocks: vec![
                Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "station-a".into() } },
                Block { option: 1, suboption: 2, info: Some(1), body: BlockBody::IpParameter { suite: suite() } },
                Block { option: 5, suboption: 4, info: None, body: BlockBody::ControlResponse { option: 2, suboption: 3, error: 3 } },
            ],
            trailing: Vec::new(),
        }
        .frame(ME, DEV)
        .unwrap(),
    ];
    let read = common::dissect(&frames);
    let dsts: Vec<String> = read.iter().map(|p| p.one("eth", "eth.dst")).collect();
    let (group, hello, dev, me) = ("01:0e:cf:00:00:00", "01:0e:cf:00:00:01", "02:00:00:00:00:01", "02:00:00:00:00:aa");
    assert_eq!(dsts, [group, group, group, me, hello, dev, dev, dev, dev, me, me]);
    for (i, p) in read.iter().enumerate() {
        assert_eq!(p.complaints(), Vec::<String>::new(), "frame {i}");
        assert!(p.has("pn_dcp"), "frame {i}: {}", p.layers);
    }
    let d = "pn_dcp";
    let xids: Vec<String> = read.iter().map(|p| p.one(d, "pn_dcp.xid")).collect();
    assert_eq!(xids, ["305419896", "2", "3", "168496141", "5", "6", "7", "8", "9", "10", "11"]);
    let services: Vec<String> = read.iter().map(|p| p.one(d, "pn_dcp.service_id")).collect();
    assert_eq!(services, ["5", "5", "5", "5", "6", "3", "4", "4", "4", "4", "3"]);
    let types: Vec<String> = read.iter().map(|p| p.one(d, "pn_dcp.service_type")).collect();
    assert_eq!(types, ["0", "0", "0", "1", "0", "0", "0", "0", "0", "1", "1"]);

    // The identify requests: the delay factor and the filters.
    assert_eq!(read[0].one(d, "pn_dcp.response_delay"), "1");
    assert_eq!(read[0].one(d, "pn_dcp.option"), "255");
    assert_eq!(read[1].one(d, "pn_dcp.response_delay"), "400");
    assert_eq!(read[1].one(d, "pn_dcp.suboption_device_nameofstation"), "station-a");
    assert_eq!(read[2].one(d, "pn_dcp.suboption_device_aliasname"), "port-001.station-b");
    for p in &read[..3] {
        assert!(p.values(d, "pn_dcp.block_info").is_empty(), "a filter carries no BlockInfo");
    }

    // The answer and the Hello: every identity field, read by the dissector.
    for (p, ip_info) in [(&read[3], "1"), (&read[4], "129")] {
        assert_eq!(p.one(d, "pn_dcp.suboption_device_nameofstation"), "station-a");
        assert_eq!(p.one(d, "pn_dcp.suboption_device_aliasname"), "port-002.station-b");
        assert_eq!(p.one(d, "pn_dcp.suboption_device_devicevendorvalue"), "bench io module");
        assert_eq!(p.one(d, "pn_dcp.suboption_vendor_id"), "65244");
        assert_eq!(p.one(d, "pn_dcp.suboption_device_id"), "291");
        assert_eq!(p.one(d, "pn_dcp.suboption_device_role"), "1");
        assert_eq!(p.values(d, "pn_dcp.suboption_device_instance"), ["0", "1"]);
        assert_eq!(p.one(d, "pn_dcp.suboption_device_oem_ven_id"), "65243");
        assert_eq!(p.one(d, "pn_dcp.suboption_device_oem_dev_id"), "1110");
        assert_eq!(p.one(d, "pn_dcp.suboption_ip_mac_address"), "02:00:00:00:00:01");
        assert_eq!(p.one(d, "pn_dcp.suboption_ip_ip"), "192.168.10.11");
        assert_eq!(p.one(d, "pn_dcp.suboption_ip_subnetmask"), "255.255.255.0");
        assert_eq!(p.one(d, "pn_dcp.suboption_ip_standard_gateway"), "192.168.10.1");
        assert_eq!(p.one(d, "pn_dcp.suboption_ip_block_info"), ip_info);
        assert_eq!(p.one(d, "pn_dcp.deviceinitiative_value"), "1");
    }
    assert_eq!(read[4].values(d, "pn_dcp.ip_conflict").len(), 1, "the dissector sees the conflict bit");
    assert!(read[3].values(d, "pn_dcp.ip_conflict").is_empty());

    // The Set requests: their qualifiers and values; the answer: its control responses.
    assert_eq!(read[6].one(d, "pn_dcp.block_qualifier"), "1");
    assert_eq!(read[6].one(d, "pn_dcp.suboption_device_nameofstation"), "station-b");
    assert_eq!(read[7].one(d, "pn_dcp.block_qualifier"), "0");
    assert_eq!(read[7].one(d, "pn_dcp.suboption_ip_ip"), "192.168.10.11");
    assert_eq!(read[8].one(d, "pn_dcp.suboption_control_signal_value"), "256");
    assert_eq!(read[9].values(d, "pn_dcp.block_error"), ["0", "6"]);
    // The Get answer: the name with its BlockInfo, the IP block's, and a control response's error.
    assert_eq!(read[10].one(d, "pn_dcp.suboption_device_nameofstation"), "station-a");
    assert_eq!(read[10].one(d, "pn_dcp.block_info"), "0");
    assert_eq!(read[10].one(d, "pn_dcp.suboption_ip_block_info"), "1");
    assert_eq!(read[10].one(d, "pn_dcp.suboption_ip_ip"), "192.168.10.11");
    assert_eq!(read[10].values(d, "pn_dcp.block_error"), ["3"]);
}

#[test]
fn the_dissectors_tell_a_wrong_reading_apart() {
    // Veredas 1 wrote a control response with a BlockInfo word, five bytes; it read its own
    // frames back and agreed with itself. The dissector reads that frame's error as 2, not 0.
    let wrong = DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Set,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks: vec![Block { option: 5, suboption: 4, info: Some(0), body: BlockBody::ControlResponse { option: 2, suboption: 2, error: 0 } }],
        trailing: Vec::new(),
    };
    let right = dcp::set_response(ME, DEV, 1, &[(2, 2, 0)]).unwrap();
    // This codec will not write it; written by hand, the dissector reads it wrongly.
    assert_eq!(wrong.frame(ME, DEV), Err(DcpError::InfoWord { option: 5, suboption: 4, carried: true }));
    let mut five = vec![2, 0, 0, 0, 0, 0xAA, 2, 0, 0, 0, 0, 1, 0x88, 0x92, 0xFE, 0xFD, 4, 1, 0, 0, 0, 1, 0, 0, 0, 10, 5, 4, 0, 5, 0, 0, 2, 2, 0, 0];
    five.resize(60, 0);
    let read = common::dissect(&[five.clone(), right]);
    assert_eq!(read[0].values("pn_dcp", "pn_dcp.block_error"), ["2"]);
    assert_eq!(read[1].values("pn_dcp", "pn_dcp.block_error"), ["0"]);
    // And this codec reads the five-byte form as damaged, not as a success.
    let pdu = dcp::decode(ether::parse(&five).unwrap().payload).unwrap();
    assert_eq!(pdu.blocks[0].body, BlockBody::Damaged { data: vec![0, 0, 2, 2, 0], damage: Damage::Length { expected: 3, got: 5 } });
    // And will not write it again: as decoded, it is a damaged block.
    assert!(matches!(pdu.frame(ME, DEV), Err(DcpError::Damaged { option: 5, suboption: 4, .. })));
    assert!(matches!(DcpPdu { service: Service::Get, ..pdu.clone() }.encode(), Err(DcpError::Damaged { option: 5, suboption: 4, .. })));
}

#[test]
fn a_set_spelled_any_way_is_a_set_and_is_gated() {
    let bad = vec![
        Block { option: 2, suboption: 2, info: Some(1), body: BlockBody::NameOfStation { name: "Port-001".into() } },
        Block { option: 5, suboption: 6, info: Some(2), body: BlockBody::Unknown { data: vec![] } },
    ];
    let good = vec![Block { option: 2, suboption: 2, info: Some(1), body: BlockBody::NameOfStation { name: "station-a".into() } }];
    // Service 4 with the selection bit clear is a Set request on the wire, however the enum says it.
    // Whatever DCP frame id it carries: the dissector reads the service, not the frame id.
    for frame_id in 0xFEFC..=0xFEFF {
        for (service, service_type) in [
            (Service::Set, ServiceType::Request),
            (Service::Other(4), ServiceType::Request),
            (Service::Set, ServiceType::Other(0)),
            (Service::Other(4), ServiceType::Other(2)),
            (Service::Set, ServiceType::Other(4)),
        ] {
            let pdu = DcpPdu { frame_id, service, service_type, ..set_pdu(bad.clone()) };
            assert!(pdu.is_set_request(), "{frame_id:#x} {service:?} {service_type:?}");
            assert_eq!(pdu.frame(Mac::BROADCAST, ME), Err(DcpError::GroupTarget(Mac::BROADCAST)), "{frame_id:#x}");
            assert_eq!(pdu.frame(DEV, ME), Err(DcpError::Name(NameError::Character('P'))), "{frame_id:#x}");
            assert_eq!(pdu.encode(), Err(DcpError::NeedsTarget("Set request")), "{frame_id:#x}");
            let ok = DcpPdu { frame_id, service, service_type, ..set_pdu(good.clone()) };
            assert!(ok.frame(DEV, ME).is_ok(), "{frame_id:#x}");
            assert_eq!(ok.frame(Mac::DCP_MULTICAST, ME), Err(DcpError::GroupTarget(Mac::DCP_MULTICAST)), "{frame_id:#x}");
        }
    }
    // Answers are not requests: service 4 with the selection bit set.
    assert!(!DcpPdu { service_type: ServiceType::Other(3), ..set_pdu(vec![]) }.is_set_request());
    // A PDU read from a file is gated alike.
    let json = r#"{"frame_id":65277,"service":{"other":4},"service_type":"request","xid":1,"response_delay":0,"blocks":[{"option":5,"suboption":6,"info":2,"body":{"kind":"unknown","data":[]}}],"trailing":[]}"#;
    let pdu: DcpPdu = serde_json::from_str(json).unwrap();
    assert_eq!(pdu.frame(Mac::BROADCAST, ME), Err(DcpError::GroupTarget(Mac::BROADCAST)));
    assert_eq!(pdu.frame(DEV, ME), Err(DcpError::SetBlock { option: 5, suboption: 6, body: "unknown" }));
    // Trailing bytes in a Set request are refused: a block could hide in them.
    let hidden = DcpPdu { trailing: vec![5, 6, 0, 2, 0, 2], ..set_pdu(good.clone()) };
    assert_eq!(hidden.frame(DEV, ME), Err(DcpError::SetTrailing(6)));
    let only = DcpPdu { trailing: vec![5, 6, 0, 2, 0, 2], ..set_pdu(vec![]) };
    assert_eq!(only.frame(DEV, ME), Err(DcpError::SetTrailing(6)));
    // The signal's one value.
    let signal = |value| set_pdu(vec![Block { option: 5, suboption: 3, info: Some(0), body: BlockBody::ControlSignal { value } }]);
    assert!(signal(dcp::SIGNAL_FLASH).frame(DEV, ME).is_ok());
    for v in [0xFFFF, 0x0000, 0x00FF, 0x0101, 0x0200] {
        assert_eq!(signal(v).frame(DEV, ME), Err(DcpError::Signal(v)), "{v:#06x}");
    }
    // No trailing byte at all: one, two or three, which make no block, are refused too.
    for n in 1..=3 {
        let t = DcpPdu { trailing: vec![7; n], ..set_pdu(good.clone()) };
        assert_eq!(t.frame(DEV, ME), Err(DcpError::SetTrailing(n)), "{n}");
    }
    assert_eq!(DcpError::NeedsTarget("Set request").to_string(), "a Set request is written only with where it goes (`DcpPdu::frame`)");
}

#[test]
fn a_block_is_written_with_its_info_word_exactly_where_the_pdu_carries_one() {
    let identify_answer = |blocks| DcpPdu {
        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks,
        trailing: Vec::new(),
    };
    let name = |info| Block { option: 2, suboption: 2, info, body: BlockBody::NameOfStation { name: "station-a".into() } };
    assert!(identify_answer(vec![name(Some(0))]).encode().is_ok());
    assert_eq!(identify_answer(vec![name(None)]).encode(), Err(DcpError::InfoWord { option: 2, suboption: 2, carried: false }));
    // A filter carries none, a Set answer's blocks none, a control response none anywhere.
    let filter = DcpPdu { frame_id: dcp::FRAME_ID_IDENTIFY_REQ, service_type: ServiceType::Request, ..identify_answer(vec![name(Some(0))]) };
    assert_eq!(filter.encode(), Err(DcpError::InfoWord { option: 2, suboption: 2, carried: true }));
    let set_answer = DcpPdu { frame_id: dcp::FRAME_ID_GET_SET, service: Service::Set, ..identify_answer(vec![name(Some(0))]) };
    assert_eq!(set_answer.encode(), Err(DcpError::InfoWord { option: 2, suboption: 2, carried: true }));
    let control = Block { option: 5, suboption: 4, info: Some(0), body: BlockBody::ControlResponse { option: 2, suboption: 2, error: 0 } };
    assert_eq!(identify_answer(vec![control]).encode(), Err(DcpError::InfoWord { option: 5, suboption: 4, carried: true }));
    // A damaged block is kept to be seen, never written, whatever its word.
    let damaged = Block { option: 2, suboption: 2, info: None, body: BlockBody::Damaged { data: vec![7], damage: Damage::NoInfo } };
    assert_eq!(identify_answer(vec![damaged]).encode(), Err(DcpError::Damaged { option: 2, suboption: 2, damage: Damage::NoInfo }));
    // A typed block only under its own option and suboption; a Get request's pair only in a Get
    // request, and a Get request nothing else.
    let moved = Block { option: 2, suboption: 1, info: Some(0), body: BlockBody::NameOfStation { name: "station-a".into() } };
    assert_eq!(identify_answer(vec![moved]).encode(), Err(DcpError::Misplaced { option: 2, suboption: 1, body: "name-of-station" }));
    let pair = Block { option: 2, suboption: 2, info: None, body: BlockBody::Requested };
    assert_eq!(identify_answer(vec![pair.clone()]).encode(), Err(DcpError::Misplaced { option: 2, suboption: 2, body: "requested" }));
    let get = DcpPdu { frame_id: dcp::FRAME_ID_GET_SET, service: Service::Get, service_type: ServiceType::Request, ..identify_answer(vec![pair, name(None)]) };
    assert_eq!(get.encode(), Err(DcpError::Misplaced { option: 2, suboption: 2, body: "name-of-station" }));
}

#[test]
fn a_hello_is_written_to_its_group_alone_and_read_as_the_dissector_reads_it() {
    let hello = DcpPdu {
        frame_id: dcp::FRAME_ID_HELLO,
        service: Service::Hello,
        service_type: ServiceType::Request,
        xid: 1,
        response_delay: 0,
        blocks: full_identity().blocks(),
        trailing: Vec::new(),
    };
    assert!(hello.frame(Mac::DCP_HELLO_MULTICAST, DEV).is_ok());
    for dst in [Mac::DCP_MULTICAST, Mac::BROADCAST, ME] {
        assert_eq!(hello.frame(dst, DEV), Err(DcpError::HelloTarget(dst)));
    }
    // A Hello answer (or "unsupported") carries no BlockInfo: its name is the whole block.
    for t in [1u8, 5] {
        let p = [0xFE, 0xFC, 6, t, 0, 0, 0, 1, 0, 0, 0, 8, 2, 2, 0, 4, b'a', b'b', b'c', b'd'];
        let pdu = dcp::decode(&p).unwrap();
        assert_eq!(pdu.blocks, [Block { option: 2, suboption: 2, info: None, body: BlockBody::NameOfStation { name: "abcd".into() } }]);
        let mut frame = vec![1, 0x0E, 0xCF, 0, 0, 1, 2, 0, 0, 0, 0, 1, 0x88, 0x92];
        frame.extend_from_slice(&p);
        frame.resize(60, 0);
        let read = common::dissect(&[frame]);
        assert_eq!(read[0].complaints(), Vec::<String>::new());
        assert_eq!(read[0].one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "abcd");
    }
    // A manufacturer's own option carries no BlockInfo either: an empty one, then a good name.
    let p = answer_with(&[0x80, 1, 0, 0, 2, 2, 0, 4, 0, 0, b'o', b'k']);
    let pdu = dcp::decode(&p).unwrap();
    assert_eq!(pdu.blocks[0], Block { option: 0x80, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![] } });
    assert_eq!(pdu.blocks[1].body, BlockBody::NameOfStation { name: "ok".into() });
    assert_eq!(pdu.encode().unwrap(), p);
}

/// One body of every kind, at its own code.
fn every_body() -> Vec<(u8, u8, BlockBody)> {
    vec![
        (1, 1, BlockBody::MacAddress { mac: DEV }),
        (1, 2, BlockBody::IpParameter { suite: suite() }),
        (1, 3, BlockBody::IpFullSuite { suite: suite(), dns: [ip(8, 8, 8, 8), ip(1, 1, 1, 1), ip(0, 0, 0, 0), ip(0, 0, 0, 0)] }),
        (2, 1, BlockBody::DeviceVendor { value: "v".into() }),
        (2, 2, BlockBody::NameOfStation { name: "station-a".into() }),
        (2, 3, BlockBody::DeviceId { vendor_id: 1, device_id: 2 }),
        (2, 4, BlockBody::DeviceRole { role: 1, reserved: 0 }),
        (2, 5, BlockBody::DeviceOptions { options: vec![(2, 2)] }),
        (2, 6, BlockBody::AliasName { name: "port-001.station-b".into() }),
        (2, 7, BlockBody::DeviceInstance { high: 0, low: 1 }),
        (2, 8, BlockBody::OemDeviceId { vendor_id: 1, device_id: 2 }),
        (2, 10, BlockBody::RsiProperties { value: 1 }),
        (5, 3, BlockBody::ControlSignal { value: dcp::SIGNAL_FLASH }),
        (5, 4, BlockBody::ControlResponse { option: 2, suboption: 2, error: 0 }),
        (6, 1, BlockBody::DeviceInitiative { value: 1 }),
        (0xFF, 0xFF, BlockBody::AllSelector),
        (5, 6, BlockBody::Unknown { data: vec![] }),
        (5, 1, BlockBody::StartTransaction),
        (5, 2, BlockBody::EndTransaction),
        (5, 5, BlockBody::FactoryReset),
        (5, 6, BlockBody::ResetToFactory),
        (2, 2, BlockBody::Requested),
        (2, 2, BlockBody::Damaged { data: vec![0xFF], damage: Damage::Text }),
    ]
}

#[test]
fn exactly_three_shapes_of_block_pass_the_set_gate() {
    let allowed = |option: u8, suboption: u8, body: &BlockBody| match (option, suboption, body) {
        (2, 2, BlockBody::NameOfStation { .. }) | (1, 2, BlockBody::IpParameter { .. }) => Some(&[0u16, 1][..]),
        (5, 3, BlockBody::ControlSignal { .. }) => Some(&[0u16][..]),
        _ => None,
    };
    let mut written = 0;
    for (code_o, code_s, body) in every_body() {
        let mut codes = vec![(code_o, code_s), (2, 2), (1, 2), (5, 3)];
        codes.dedup_by(|a, b| a == b);
        let codes: Vec<(u8, u8)> = codes.iter().enumerate().filter(|(i, c)| !codes[..*i].contains(c)).map(|(_, c)| *c).collect();
        for (option, suboption) in codes {
            for qualifier in [None, Some(0), Some(1), Some(2), Some(3)] {
                let pdu = set_pdu(vec![Block { option, suboption, info: qualifier, body: body.clone() }]);
                let got = pdu.frame(DEV, ME).map(|_| ());
                let want = match allowed(option, suboption, &body) {
                    Some(ok) if qualifier.is_some_and(|q| ok.contains(&q)) => Ok(()),
                    Some(ok) => Err(DcpError::Qualifier { option, suboption, qualifier, allowed: if ok.len() == 2 { "0 or 1" } else { "0" } }),
                    None => Err(DcpError::SetBlock { option, suboption, body: body.kind() }),
                };
                written += usize::from(got.is_ok());
                assert_eq!(got, want, "{option},{suboption} {qualifier:?} {body:?}");
            }
        }
    }
    assert_eq!(written, 5, "a good name and an IP suite at 0 and 1, the signal at 0");
}

#[test]
fn bytes_as_given_are_written_only_where_no_typed_body_is_read() {
    let answer = |blocks| DcpPdu {
        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks,
        trailing: Vec::new(),
    };
    // Unread bytes under a typed code are refused: Veredas 1's five-byte control response cannot
    // be written as bytes either.
    let five = DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Set,
        ..answer(vec![Block { option: 5, suboption: 4, info: None, body: BlockBody::Unknown { data: vec![0, 0, 2, 2, 0] } }])
    };
    assert_eq!(five.frame(ME, DEV), Err(DcpError::Misplaced { option: 5, suboption: 4, body: "unknown" }));
    let mac = Block { option: 1, suboption: 1, info: Some(0), body: BlockBody::Unknown { data: vec![1, 2, 3] } };
    assert_eq!(answer(vec![mac]).encode(), Err(DcpError::Misplaced { option: 1, suboption: 1, body: "unknown" }));
    // A damaged block of any kind, with its word or without, in any PDU, is refused: each reader
    // recovers from damage its own way (the dissector reads a too-long block's extra bytes as
    // blocks of their own, and a too-short one swallows the next).
    let damaged = |info, damage| Block { option: 2, suboption: 2, info, body: BlockBody::Damaged { data: vec![0xFF, 0xFE, 0], damage } };
    let kinds = [Damage::NoInfo, Damage::Text, Damage::Length { expected: 4, got: 3 }, Damage::Overrun { declared: 40, left: 3 }];
    for damage in kinds {
        for info in [None, Some(0)] {
            let want = Err(DcpError::Damaged { option: 2, suboption: 2, damage: damage.clone() });
            assert_eq!(answer(vec![damaged(info, damage.clone())]).encode(), want, "{damage:?} {info:?}");
            let filter = DcpPdu { frame_id: dcp::FRAME_ID_IDENTIFY_REQ, service_type: ServiceType::Request, ..answer(vec![damaged(info, damage.clone())]) };
            assert_eq!(filter.encode(), want);
            let hello = DcpPdu { frame_id: dcp::FRAME_ID_HELLO, service: Service::Hello, service_type: ServiceType::Request, ..answer(vec![damaged(info, damage.clone())]) };
            assert_eq!(hello.frame(Mac::DCP_HELLO_MULTICAST, DEV), want);
        }
    }
    assert_eq!(
        DcpError::Damaged { option: 2, suboption: 2, damage: Damage::NoInfo }.to_string(),
        "block 2/2 is damaged (NoInfo): a damaged block is kept to be seen, never written"
    );
    // Elsewhere bytes as given are written as given.
    let other = Block { option: 2, suboption: 11, info: Some(0), body: BlockBody::Unknown { data: vec![1, 2] } };
    assert!(answer(vec![other]).encode().is_ok());
    // Under every typed code, unread bytes are refused, empty or not.
    // The twenty typed codes, listed here apart from the codec's own list, and held to it.
    let typed: [(u8, u8); 20] =
        [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (2, 6), (2, 7), (2, 8), (2, 10), (5, 1), (5, 2), (5, 3), (5, 4), (5, 5), (5, 6), (6, 1), (0xFF, 0xFF)];
    assert_eq!(dcp::TYPED, typed);
    for (option, suboption) in typed {
        for data in [vec![], vec![0x5A; 7]] {
            let b = Block { option, suboption, info: Some(0), body: BlockBody::Unknown { data } };
            assert_eq!(answer(vec![b]).encode(), Err(DcpError::Misplaced { option, suboption, body: "unknown" }), "{option},{suboption}");
        }
    }
    assert_eq!(dcp::TYPED.len(), 20);

    // What would be read back as another PDU is refused: trailing bytes that make a block or a
    // pair.
    let not_itself = |r: Result<Vec<u8>, DcpError>| matches!(r, Err(DcpError::NotItself(_)));
    let name = Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "ok".into() } };
    assert!(not_itself(DcpPdu { trailing: vec![2, 2, 0, 0], ..answer(vec![name.clone()]) }.encode()));
    assert!(DcpPdu { trailing: vec![2, 2, 0], ..answer(vec![name.clone()]) }.encode().is_ok());
    let hello = DcpPdu {
        frame_id: dcp::FRAME_ID_HELLO,
        service: Service::Hello,
        service_type: ServiceType::Request,
        trailing: [&[2, 2, 0, 4][..], b"evil", &[5, 4, 0, 5, 0, 0, 2, 2, 0, 0]].concat(),
        ..answer(vec![name.clone()])
    };
    assert!(matches!(hello.frame(Mac::DCP_HELLO_MULTICAST, DEV), Err(DcpError::NotItself(_))));
    let get = |trailing| DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Get,
        service_type: ServiceType::Request,
        trailing,
        ..answer(vec![Block { option: 2, suboption: 2, info: None, body: BlockBody::Requested }])
    };
    assert!(not_itself(get(vec![2, 2, 1, 2]).encode()));
    assert!(not_itself(get(vec![2, 2]).encode()));
    assert_eq!(get(vec![2]).encode().unwrap(), [0xFE, 0xFD, 3, 0, 0, 0, 0, 1, 0, 0, 0, 3, 2, 2, 2]);
    assert!(DcpError::NotItself("x".into()).to_string().starts_with("this PDU would be read back as another: "));
    // A Get request's pair carries no word; a frame id that is not DCP's is refused.
    let get = DcpPdu {
        frame_id: dcp::FRAME_ID_GET_SET,
        service: Service::Get,
        service_type: ServiceType::Request,
        ..answer(vec![Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::Requested }])
    };
    assert_eq!(get.encode(), Err(DcpError::Misplaced { option: 2, suboption: 2, body: "requested" }));
    assert_eq!(DcpPdu { frame_id: 0x8000, ..answer(vec![]) }.encode(), Err(DcpError::NotDcp(0x8000)));
}

#[test]
fn a_hello_spelled_any_way_goes_to_its_group_alone() {
    for frame_id in 0xFEFC..=0xFEFF {
        for (service, service_type) in [
            (Service::Hello, ServiceType::Request),
            (Service::Other(6), ServiceType::Request),
            (Service::Hello, ServiceType::Success),
            (Service::Hello, ServiceType::Other(5)),
            (Service::Other(6), ServiceType::Other(2)),
        ] {
            let pdu = DcpPdu { frame_id, service, service_type, xid: 1, response_delay: 0, blocks: vec![], trailing: Vec::new() };
            assert!(pdu.is_hello());
            assert_eq!(pdu.encode(), Err(DcpError::NeedsTarget("Hello")), "{frame_id:#x}");
            for dst in [Mac::DCP_MULTICAST, Mac::BROADCAST, ME] {
                assert_eq!(pdu.frame(dst, DEV), Err(DcpError::HelloTarget(dst)), "{frame_id:#x}");
            }
            assert!(pdu.frame(Mac::DCP_HELLO_MULTICAST, DEV).is_ok(), "{frame_id:#x}");
        }
    }
}

/// A frame from DEV to ME holding this PROFINET payload.
fn wire(p: &[u8]) -> Vec<u8> {
    let mut f = vec![2, 0, 0, 0, 0, 0xAA, 2, 0, 0, 0, 0, 1, 0x88, 0x92];
    f.extend_from_slice(p);
    f.resize(f.len().max(60), 0);
    f
}

#[test]
fn the_info_word_is_where_the_dissector_reads_one_for_each_family_of_block() {
    let name = [2, 2, 0, 6, 0, 0, b'o', b'k', b'o', b'k'];
    let with = |first: &[u8]| answer_with(&[first, &name[..]].concat());
    // An IP option outside its three suboptions, a control option outside its six, the
    // all-selector's option with another suboption, and manufacturers' options: no word, the whole
    // block unread bytes.
    let cases: Vec<(Vec<u8>, Block)> = vec![
        (vec![1, 9, 0, 4, 0, 0, 1, 2], Block { option: 1, suboption: 9, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![1, 0, 0, 4, 0, 0, 1, 2], Block { option: 1, suboption: 0, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![1, 4, 0, 4, 0, 0, 1, 2], Block { option: 1, suboption: 4, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![1, 8, 0, 4, 0, 0, 1, 2], Block { option: 1, suboption: 8, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![5, 0, 0, 4, 0, 0, 1, 2], Block { option: 5, suboption: 0, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![5, 7, 0, 4, 0, 0, 1, 2], Block { option: 5, suboption: 7, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![5, 9, 0, 4, 0, 1, 0, 1], Block { option: 5, suboption: 9, info: None, body: BlockBody::Unknown { data: vec![0, 1, 0, 1] } }),
        (vec![5, 9, 0, 0], Block { option: 5, suboption: 9, info: None, body: BlockBody::Unknown { data: vec![] } }),
        (vec![0xFF, 1, 0, 0], Block { option: 0xFF, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![] } }),
        (vec![0x81, 1, 0, 4, 0, 0, 1, 2], Block { option: 0x81, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![0xC0, 1, 0, 4, 0, 0, 1, 2], Block { option: 0xC0, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![0xFD, 1, 0, 4, 0, 0, 1, 2], Block { option: 0xFD, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
        (vec![0xFE, 1, 0, 4, 0, 0, 1, 2], Block { option: 0xFE, suboption: 1, info: None, body: BlockBody::Unknown { data: vec![0, 0, 1, 2] } }),
    ];
    let mut frames = Vec::new();
    for (first, block) in &cases {
        let p = with(first);
        let pdu = dcp::decode(&p).unwrap();
        assert_eq!(pdu.blocks[0], *block, "{first:?}");
        assert_eq!(pdu.blocks[1].body, BlockBody::NameOfStation { name: "okok".into() });
        assert_eq!(pdu.encode().unwrap(), p);
        frames.push(wire(&p));
    }
    // A control block carries a qualifier wherever it is: here a signal in a Set answer.
    let set_answer = [0xFE, 0xFD, 4, 1, 0, 0, 0, 1, 0, 0, 0, 8, 5, 3, 0, 4, 0, 0, 1, 0];
    let pdu = dcp::decode(&set_answer).unwrap();
    assert_eq!(pdu.blocks, [Block { option: 5, suboption: 3, info: Some(0), body: BlockBody::ControlSignal { value: 0x0100 } }]);
    assert_eq!(pdu.encode().unwrap(), set_answer);
    frames.push(wire(&set_answer));
    let read = common::dissect(&frames);
    for (p, (first, _)) in read.iter().zip(&cases) {
        // The dissector reads the name after it, and a BlockInfo for the name alone (under the
        // general field and, for the IP option, under the IP's own, which stays empty).
        assert_eq!(p.one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "okok", "{first:?}");
        assert_eq!(p.values("pn_dcp", "pn_dcp.block_info"), ["0"], "{first:?}");
        assert!(p.values("pn_dcp", "pn_dcp.suboption_ip_block_info").is_empty(), "{first:?}");
        assert!(p.values("pn_dcp", "pn_dcp.block_qualifier").is_empty(), "{first:?}");
    }
    assert_eq!(read[cases.len()].one("pn_dcp", "pn_dcp.block_qualifier"), "0");
    assert_eq!(read[cases.len()].one("pn_dcp", "pn_dcp.suboption_control_signal_value"), "256");
    assert_eq!(cases.len(), 13);

    // An "unsupported" answer (type 5) carries a BlockInfo as a successful one does: an Identify
    // answer and a Get answer, by this codec and by tshark.
    let mut unsupported = Vec::new();
    for (frame_id, service) in [(0xFEFFu16, 5u8), (0xFEFD, 3)] {
        let mut p = vec![(frame_id >> 8) as u8, frame_id as u8, service, 5, 0, 0, 0, 1, 0, 0];
        p.extend_from_slice(&(name.len() as u16).to_be_bytes());
        p.extend_from_slice(&name);
        let pdu = dcp::decode(&p).unwrap();
        assert_eq!(pdu.blocks, [Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "okok".into() } }], "{service}");
        assert_eq!(pdu.encode().unwrap(), p);
        unsupported.push(wire(&p));
    }
    for (i, p) in common::dissect(&unsupported).iter().enumerate() {
        assert_eq!((p.one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), p.values("pn_dcp", "pn_dcp.block_info")), ("okok".to_string(), vec!["0".to_string()]), "{i}");
    }

    // The four control blocks that are their qualifier alone, typed, in an answer: the dissector
    // reads each qualifier (a reset's under its own field) and the name after them.
    let control = |suboption, info, body| Block { option: 5, suboption, info: Some(info), body };
    let blocks = vec![
        control(1, 0, BlockBody::StartTransaction),
        control(2, 0, BlockBody::EndTransaction),
        control(5, 1, BlockBody::FactoryReset),
        control(6, 2, BlockBody::ResetToFactory),
        Block { option: 2, suboption: 2, info: Some(0), body: BlockBody::NameOfStation { name: "okok".into() } },
    ];
    let pdu = DcpPdu {
        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks: blocks.clone(),
        trailing: Vec::new(),
    };
    let p = pdu.encode().unwrap();
    assert_eq!(&p[12..36], &[5, 1, 0, 2, 0, 0, 5, 2, 0, 2, 0, 0, 5, 5, 0, 2, 0, 1, 5, 6, 0, 2, 0, 2]);
    assert_eq!(dcp::decode(&p).unwrap().blocks, blocks);
    // A Hello holding a reset with more than its qualifier, as heard: this codec keeps it damaged
    // beside the name before it, and will not write it again; the dissector reads its qualifier
    // and then its extra bytes as a block of their own (a second name), which is why.
    let mut heard = vec![0xFE, 0xFC, 6, 0, 0, 0, 0, 1, 0, 0, 0, 32, 2, 2, 0, 11, 0, 0];
    heard.extend_from_slice(b"station-a\0");
    heard.extend_from_slice(&[5, 5, 0, 12, 0, 0, 2, 2, 0, 6, 0, 0]);
    heard.extend_from_slice(b"evil");
    let hello = dcp::decode(&heard).unwrap();
    assert_eq!(hello.blocks.len(), 2);
    assert_eq!(hello.blocks[1], control(5, 0, BlockBody::Damaged { data: [&[2, 2, 0, 6, 0, 0][..], b"evil"].concat(), damage: Damage::Length { expected: 0, got: 10 } }));
    assert!(matches!(hello.frame(Mac::DCP_HELLO_MULTICAST, DEV), Err(DcpError::Damaged { option: 5, suboption: 5, .. })));
    let mut f = vec![1, 0x0E, 0xCF, 0, 0, 1, 2, 0, 0, 0, 0, 0xAA, 0x88, 0x92];
    f.extend_from_slice(&heard);
    f.resize(60, 0);
    let read = common::dissect(&[wire(&p), f]);
    assert_eq!(read[0].values("pn_dcp", "pn_dcp.suboption_control"), ["1", "2", "5", "6"]);
    assert_eq!(read[0].values("pn_dcp", "pn_dcp.block_qualifier"), ["0", "0"]);
    assert_eq!(read[0].values("pn_dcp", "pn_dcp.block_qualifier_reset"), ["1", "2"]);
    assert_eq!(read[0].one("pn_dcp", "pn_dcp.suboption_device_nameofstation"), "okok");
    assert_eq!(read[0].complaints(), Vec::<String>::new());
    assert_eq!(read[1].values("pn_dcp", "pn_dcp.suboption_device_nameofstation"), ["station-a", "evil"]);
}

#[test]
fn the_device_options_and_the_full_ip_suite_are_written_and_read_in_their_order() {
    let full = Block {
        option: 1,
        suboption: 3,
        info: Some(1),
        body: BlockBody::IpFullSuite { suite: suite(), dns: [ip(10, 0, 0, 53), ip(10, 0, 0, 54), ip(10, 0, 0, 55), ip(10, 0, 0, 56)] },
    };
    let options = Block { option: 2, suboption: 5, info: Some(0), body: BlockBody::DeviceOptions { options: vec![(1, 3), (6, 1)] } };
    let pdu = DcpPdu {
        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
        service: Service::Identify,
        service_type: ServiceType::Success,
        xid: 1,
        response_delay: 0,
        blocks: vec![options, full],
        trailing: Vec::new(),
    };
    let p = pdu.encode().unwrap();
    let mut want = vec![0xFE, 0xFF, 5, 1, 0, 0, 0, 1, 0, 0, 0, 44, 2, 5, 0, 6, 0, 0, 1, 3, 6, 1, 1, 3, 0, 30, 0, 1];
    want.extend_from_slice(&[192, 168, 10, 11, 255, 255, 255, 0, 192, 168, 10, 1, 10, 0, 0, 53, 10, 0, 0, 54, 10, 0, 0, 55, 10, 0, 0, 56]);
    assert_eq!(p, want);
    assert_eq!(dcp::decode(&p).unwrap(), pdu);
    let id = DeviceIdentity::from_blocks(&pdu.blocks);
    assert_eq!((id.ip, id.ip_status, id.options), (Some(suite()), Some(IpStatus { set: 1, conflict: false }), vec![(1, 3), (6, 1)]));
    let read = common::dissect(&[wire(&p)]);
    // The options listed after the block's own: 1 then 6, as written (swapped they would read 3, 1).
    assert_eq!(read[0].values("pn_dcp", "pn_dcp.option"), ["2", "1", "6", "1"]);
    // The dissector reads the four name servers after the suite under its own address, mask and
    // gateway fields, in their order: 53, 54, then 55 and 56.
    let d = "pn_dcp";
    assert_eq!(read[0].values(d, "pn_dcp.suboption_ip_ip"), ["192.168.10.11", "10.0.0.53"]);
    assert_eq!(read[0].values(d, "pn_dcp.suboption_ip_subnetmask"), ["255.255.255.0", "10.0.0.54"]);
    assert_eq!(read[0].values(d, "pn_dcp.suboption_ip_standard_gateway"), ["192.168.10.1", "10.0.0.55", "10.0.0.56"]);
    // The options' BlockInfo, then the full suite's (set, under the general field for this one).
    assert_eq!(read[0].values(d, "pn_dcp.block_info"), ["0", "1"]);
    assert_eq!(read[0].complaints(), Vec::<String>::new());
}
