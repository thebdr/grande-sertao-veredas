//! C-116: the PROFINET device's record read. The client is the kernel's own IP stack (an address on
//! the far end of the veth pair, a UDP socket): it resolves the device by ARP and checks every IPv4
//! and UDP checksum the device writes. The reads are aranha's codecs; tshark is the second reading.

#![cfg(target_os = "linux")]

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use veredas_aranha::netns;
use veredas_aranha::profinet::dcp::{self, IpSuite};
use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::record::{INDEX_IM0, INDEX_PORT_DATA_REAL, Im0, LinkState, Peer, PortDataReal, SoftwareRevision};
use veredas_aranha::profinet::rpc::{self, ByteOrder, PNIO_PORT, PnioStatus, ReadRequest, ReadResponse, ResponseArgs, RpcPdu, Uuid};
use veredas_aranha::wire::raw::{Options, RawLink};
use veredas_maquinar::profinet::ip;
use veredas_maquinar::profinet::*;

const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nport = \"port-002\"\norder_id = \"6ES7 bench\"\nserial_number = \"SN-0001\"\nhello = false\nlldp_interval_ms = 0\n";

fn config(text: &str) -> ProfinetConfig {
    toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

/// The far end's address, given to the kernel; its transmit checksums computed before they leave.
/// A veth leaves a UDP checksum to an offload that never happens, so a raw socket on the other end
/// (the device's) reads only its partial sum; a network card writes the whole one on the wire,
/// which is what the device checks.
fn host_address() {
    for args in [&["ip", "addr", "add", "192.168.0.1/24", "dev", "a0"][..], &["ethtool", "-K", "a0", "tx", "off"][..]] {
        let out = std::process::Command::new(args[0]).args(&args[1..]).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

fn socket() -> UdpSocket {
    let s = UdpSocket::bind("192.168.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(700))).unwrap();
    s
}

/// An implicit read of `index` at (slot 0, `subslot`) to `to`, as an engineering tool sends it;
/// the answer's status, header and record, or `None` when nothing comes back.
fn read(s: &UdpSocket, to: &str, seq: u16, subslot: u16, index: u16) -> Option<(PnioStatus, Option<ReadResponse>, Vec<u8>)> {
    let request = ReadRequest::implicit(seq, 0, subslot, index, 4096);
    let pdu = rpc::read_implicit(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([seq as u8; 16]), u32::from(seq), &request, 4096 + 64).unwrap();
    s.send_to(&pdu.encode().unwrap(), (to, PNIO_PORT)).unwrap();
    let mut buf = [0u8; 2048];
    let (n, from) = s.recv_from(&mut buf).ok()?;
    assert_eq!(from, SocketAddr::from((to.parse::<Ipv4Addr>().unwrap(), PNIO_PORT)), "answered from the device's address and port");
    let answer = RpcPdu::decode(&buf[..n]).unwrap();
    assert_eq!((answer.packet_type, answer.sequence, answer.activity), (rpc::PacketType::Response, u32::from(seq), Uuid([seq as u8; 16])));
    let args = ResponseArgs::decode(&answer.body, answer.byte_order()).unwrap();
    if args.data.is_empty() {
        return Some((args.status, None, Vec::new()));
    }
    let (header, record) = ReadResponse::split(&args.data).unwrap();
    Some((args.status, Some(header), record))
}

fn wait(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let begun = Instant::now();
    while begun.elapsed() < within {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn padded(s: &str, n: usize) -> String {
    format!("{s:<n$}")
}

#[test]
fn the_kernel_finds_the_device_by_arp_and_reads_its_i_m0() {
    netns::isolated("the_kernel_finds_the_device_by_arp_and_reads_its_i_m0", &[("a0", "b0")], || {
        host_address();
        let dev = start(&config(DEVICE)).unwrap();
        let s = socket();
        let (status, header, record) = read(&s, "192.168.0.10", 1, SUBSLOT_DAP, INDEX_IM0).expect("an answer");
        assert_eq!(status, PnioStatus::OK);
        let header = header.unwrap();
        assert_eq!((header.seq, header.slot, header.subslot, header.index, header.length as usize), (1, 0, SUBSLOT_DAP, INDEX_IM0, record.len()));
        let (blocks, rest) = rpc::read_blocks(&record);
        assert!(rest.is_empty());
        let im0 = Im0::from_block(&blocks[0]).unwrap();
        assert_eq!(
            im0,
            Im0 {
                vendor_id: 7,
                order_id: padded("6ES7 bench", 20),
                serial_number: padded("SN-0001", 16),
                hardware_revision: 1,
                software_revision: SoftwareRevision { prefix: b'V', functional: 1, bugfix: 0, internal: 0 },
                revision_counter: 0,
                profile_id: 0,
                profile_specific_type: 0,
                version: (1, 1),
                supported: 0,
            }
        );
        // The kernel resolved the address to the device's own MAC, by the device's ARP reply.
        let neigh = std::process::Command::new("ip").args(["neigh", "show", "192.168.0.10", "dev", "a0"]).output().unwrap();
        let neigh = String::from_utf8_lossy(&neigh.stdout).to_string();
        assert!(neigh.contains(&dev.mac.to_string()), "{neigh} against {}", dev.mac);
        let st = dev.status();
        assert!(st.arp_answered == 1 && (st.reads, st.reads_refused) == (1, 0), "{st:?}");
        // Without a serial number in the file, the MAC's twelve hex digits.
        drop(dev);
        let dev = start(&config(&DEVICE.replace("serial_number = \"SN-0001\"\n", ""))).unwrap();
        let (_, _, record) = read(&s, "192.168.0.10", 2, SUBSLOT_DAP, INDEX_IM0).expect("an answer");
        let im0 = Im0::from_block(&rpc::read_blocks(&record).0[0]).unwrap();
        let hex: String = dev.mac.0.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(im0.serial_number, padded(&hex, 16));
    });
}

#[test]
fn the_port_data_names_the_neighbour_its_lldp_heard_until_it_leaves() {
    netns::isolated("the_port_data_names_the_neighbour_its_lldp_heard_until_it_leaves", &[("a0", "b0")], || {
        host_address();
        let dev = start(&config(DEVICE)).unwrap();
        let raw = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_LLDP), promiscuous: false, outgoing: false }).unwrap();
        let s = socket();
        let port_data = |seq: u16| {
            let (status, _, record) = read(&s, "192.168.0.10", seq, SUBSLOT_PORT, INDEX_PORT_DATA_REAL).expect("an answer");
            assert_eq!(status, PnioStatus::OK);
            PortDataReal::from_block(&rpc::read_blocks(&record).0[0]).unwrap()
        };
        let alone = port_data(1);
        assert_eq!(
            alone,
            PortDataReal { slot: 0, subslot: SUBSLOT_PORT, own_port_id: "port-002".into(), peers: vec![], mau_type: 0x0010, domain_boundary: 0, multicast_boundary: 0, link_state: LinkState { port: 0, link: 1 }, media_type: 1, extra: vec![] }
        );
        // A switch's LLDP on the wire: its chassis and port become the port's peer.
        let switch = raw.mac();
        raw.send(&lldp::frame(switch, &LldpPdu::new(ChassisId::Local { id: "switch-1".into() }, PortId::Local { id: "port-003".into() }, 120)).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some()));
        let peer = Peer { port_id: "port-003".into(), chassis_id: "switch-1".into(), line_delay: 0, mac: switch };
        assert_eq!(port_data(2).peers, std::slice::from_ref(&peer));
        assert_eq!(dev.neighbour(), Some(peer));
        // A chassis given as a MAC is named by its address.
        raw.send(&lldp::frame(switch, &LldpPdu::new(ChassisId::Mac { mac: switch }, PortId::Local { id: "port-004".into() }, 120)).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some_and(|p| p.port_id == "port-004")));
        assert_eq!(port_data(3).peers[0].chassis_id, switch.to_string());
        // Its shutdown frame: no peer any more.
        raw.send(&lldp::frame(switch, &LldpPdu::new(ChassisId::Mac { mac: switch }, PortId::Local { id: "port-004".into() }, 0)).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(2), || dev.neighbour().is_none()));
        assert_eq!(port_data(4).peers, []);
    });
}

#[test]
fn a_read_of_what_it_lacks_is_refused_with_the_status_and_no_record() {
    netns::isolated("a_read_of_what_it_lacks_is_refused_with_the_status_and_no_record", &[("a0", "b0")], || {
        host_address();
        let dev = start(&config(DEVICE)).unwrap();
        let s = socket();
        let refused = |seq: u16, subslot: u16, index: u16| {
            let (status, header, record) = read(&s, "192.168.0.10", seq, subslot, index).expect("an answer");
            (status.to_bytes(), header.map(|h| (h.index, h.length)), record.len())
        };
        assert_eq!(refused(1, SUBSLOT_DAP, 0x1234), ([0xDE, 0x80, 0xB0, 0x00], Some((0x1234, 0)), 0), "an index it lacks");
        assert_eq!(refused(2, SUBSLOT_PORT, INDEX_IM0), ([0xDE, 0x80, 0xB2, 0x00], Some((INDEX_IM0, 0)), 0), "I&M0 at the port");
        assert_eq!(refused(3, SUBSLOT_DAP, INDEX_PORT_DATA_REAL), ([0xDE, 0x80, 0xB2, 0x00], Some((INDEX_PORT_DATA_REAL, 0)), 0), "the port's data at the access point");
        assert_eq!(refused(4, 0x8002, INDEX_PORT_DATA_REAL), ([0xDE, 0x80, 0xB2, 0x00], Some((INDEX_PORT_DATA_REAL, 0)), 0), "a port it lacks");
        assert_eq!((ip::INVALID_INDEX.to_bytes(), ip::INVALID_SLOT.to_bytes()), ([0xDE, 0x80, 0xB0, 0x00], [0xDE, 0x80, 0xB2, 0x00]));
        let st = dev.status();
        assert_eq!((st.reads, st.reads_refused), (0, 4));
    });
}

#[test]
fn it_answers_only_a_read_at_its_own_address_and_port_and_follows_a_dcp_set_of_its_address() {
    netns::isolated("it_answers_only_a_read_at_its_own_address_and_port_and_follows_a_dcp_set_of_its_address", &[("a0", "b0")], || {
        host_address();
        let dev = start(&config(DEVICE)).unwrap();
        let s = socket();
        // Another operation of the device interface (a Connect, opnum 0) is not answered.
        let connect = RpcPdu::pnio_request(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([9; 16]), 1, rpc::opnum::CONNECT, &rpc::RequestArgs::new(1024, vec![]));
        s.send_to(&connect.encode().unwrap(), ("192.168.0.10", PNIO_PORT)).unwrap();
        let mut buf = [0u8; 2048];
        assert!(s.recv_from(&mut buf).is_err(), "a Connect is not this slice's");
        // Nor a read to another port, nor to another address of the network.
        let request = rpc::read_implicit(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([8; 16]), 2, &ReadRequest::implicit(2, 0, 1, INDEX_IM0, 4096), 4160).unwrap().encode().unwrap();
        s.send_to(&request, ("192.168.0.10", PNIO_PORT + 1)).unwrap();
        assert!(s.recv_from(&mut buf).is_err());
        assert!(read(&s, "192.168.0.11", 3, SUBSLOT_DAP, INDEX_IM0).is_none(), "no device at .11");
        // Its address set by DCP: answered at the new one, no longer at the old.
        let dcp_link = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_PROFINET), promiscuous: false, outgoing: false }).unwrap();
        let suite = IpSuite { ip: "192.168.0.20".parse().unwrap(), mask: "255.255.255.0".parse().unwrap(), gateway: Ipv4Addr::UNSPECIFIED };
        dcp_link.send(&dcp::set_ip_request(dev.mac, dcp_link.mac(), 4, &suite, false).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(2), || dev.identity().ip.map(|s| s.ip) == Some("192.168.0.20".parse().unwrap())));
        assert!(read(&s, "192.168.0.20", 5, SUBSLOT_DAP, INDEX_IM0).is_some_and(|(st, _, _)| st == PnioStatus::OK));
        assert!(read(&s, "192.168.0.10", 6, SUBSLOT_DAP, INDEX_IM0).is_none(), "the old address is no longer its");
        // A device without an address answers neither ARP nor a read.
        drop(dev);
        let bare = start(&config(&DEVICE.replace("ip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\n", ""))).unwrap();
        assert!(read(&s, "192.168.0.10", 7, SUBSLOT_DAP, INDEX_IM0).is_none());
        assert_eq!(bare.status().arp_answered, 0);
    });
}

#[test]
fn a_frame_it_must_not_take_is_not_answered() {
    let mac = Mac([0x02, 0, 0, 0, 0, 0x10]);
    let me = Ipv4Addr::new(192, 168, 0, 10);
    let peer = (Mac([0x02, 0, 0, 0, 0, 0x01]), Ipv4Addr::new(192, 168, 0, 1), 50000);
    let good = ip::udp_frame((mac, me, PNIO_PORT), peer, 7, b"hello");
    assert_eq!(ip::udp_to(&good, me, PNIO_PORT), Some((peer.0, peer.1, peer.2, b"hello".to_vec())));
    // A fragment, a damaged header checksum, a damaged UDP checksum, another protocol: refused.
    let mut fragment = good.clone();
    fragment[14 + 6] |= 0x20;
    assert_eq!(ip::udp_to(&fragment, me, PNIO_PORT), None);
    let mut header = good.clone();
    header[14 + 10] ^= 0xFF;
    assert_eq!(ip::udp_to(&header, me, PNIO_PORT), None);
    let mut udp = good.clone();
    udp[14 + 20 + 8] ^= 0x01;
    assert_eq!(ip::udp_to(&udp, me, PNIO_PORT), None);
    // A UDP checksum of 0 means none was computed: taken.
    let mut none = good.clone();
    none[14 + 20 + 6] = 0;
    none[14 + 20 + 7] = 0;
    assert!(ip::udp_to(&none, me, PNIO_PORT).is_some());
    let mut tcp = good.clone();
    tcp[14 + 9] = 6;
    assert_eq!(ip::udp_to(&tcp, me, PNIO_PORT), None);
    assert_eq!(ip::udp_to(&good, Ipv4Addr::UNSPECIFIED, PNIO_PORT), None);
    // ARP: a request for its address is answered from its MAC to the asker; any other is not.
    let mut ask = vec![0xFF; 6];
    ask.extend_from_slice(&peer.0.0);
    ask.extend_from_slice(&[0x08, 0x06, 0, 1, 0x08, 0, 6, 4, 0, 1]);
    ask.extend_from_slice(&peer.0.0);
    ask.extend_from_slice(&peer.1.octets());
    ask.extend_from_slice(&[0; 6]);
    ask.extend_from_slice(&me.octets());
    let reply = ip::arp_reply(&ask, mac, me).unwrap();
    let mut expected = [&peer.0.0[..], &mac.0, &[0x08, 0x06, 0, 1, 0x08, 0, 6, 4, 0, 2], &mac.0, &me.octets(), &peer.0.0, &peer.1.octets()].concat();
    expected.resize(60, 0);
    assert_eq!(reply, expected, "the whole reply, byte for byte");
    assert_eq!(ip::arp_reply(&ask, mac, Ipv4Addr::new(192, 168, 0, 11)), None);
    let mut answer_op = ask.clone();
    answer_op[21] = 2;
    assert_eq!(ip::arp_reply(&answer_op, mac, me), None, "a reply is not answered");
}

#[test]
fn the_dissectors_read_the_arp_and_the_record_read_as_the_standard_has_it() {
    netns::isolated("the_dissectors_read_the_arp_and_the_record_read_as_the_standard_has_it", &[("a0", "b0")], || {
        host_address();
        let all = RawLink::open_with("a0", Options { ethertype: None, promiscuous: true, outgoing: true }).unwrap();
        let _dev = start(&config(DEVICE)).unwrap();
        let s = socket();
        read(&s, "192.168.0.10", 1, SUBSLOT_DAP, INDEX_IM0).expect("an answer");
        read(&s, "192.168.0.10", 2, SUBSLOT_PORT, INDEX_PORT_DATA_REAL).expect("an answer");
        read(&s, "192.168.0.10", 3, SUBSLOT_DAP, 0x1234).expect("an answer");
        let mut frames = Vec::new();
        while let Some(r) = all.recv(Duration::from_millis(200)).unwrap() {
            let t = u16::from_be_bytes([r.bytes[12], r.bytes[13]]);
            if t == ip::ETHERTYPE_ARP || t == ip::ETHERTYPE_IPV4 {
                frames.push(r.bytes);
            }
        }
        let tshark = std::path::Path::new(&std::env::var_os("HOME").unwrap()).join(".local/opt/wireshark/tshark");
        let mut pcap = 0xa1b2_c3d4u32.to_le_bytes().to_vec();
        pcap.extend_from_slice(&[2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 1, 0, 0, 0]);
        for (i, f) in frames.iter().enumerate() {
            pcap.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
            pcap.extend_from_slice(&0u32.to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(f);
        }
        let file = std::env::temp_dir().join(format!("veredas-pn-records-{}.pcap", std::process::id()));
        std::fs::write(&file, pcap).unwrap();
        let fields = ["arp.opcode", "udp.checksum.status", "ip.checksum.status", "pn_io.index", "pn_io.error_code", "_ws.malformed", "_ws.expert.severity", "pn_io.error_decode"];
        let mut cmd = std::process::Command::new(&tshark);
        cmd.args(["-n", "-o", "udp.check_checksum:TRUE", "-o", "ip.check_checksum:TRUE", "-r"]).arg(&file).args(["-T", "fields", "-E", "occurrence=a", "-E", "aggregator=,"]);
        for f in fields {
            cmd.args(["-e", f]);
        }
        let out = cmd.output().unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
        let lines: Vec<Vec<String>> = String::from_utf8(out.stdout).unwrap().lines().map(|l| l.split('\t').map(String::from).collect()).collect();
        // The ARP request and the device's reply.
        assert!(lines.iter().any(|l| l[0] == "2"), "an ARP reply: {lines:?}");
        // Three reads and three answers, every checksum good, the indexes as asked, the refusal's
        // status as sent; no frame malformed or warned of.
        let reads: Vec<&Vec<String>> = lines.iter().filter(|l| !l[3].is_empty()).collect();
        assert_eq!(reads.len(), 6, "{lines:?}");
        // Wireshark's checksum status: 1 good (0 bad, 2 not checked).
        assert!(reads.iter().all(|l| l[1] == "1" && l[2] == "1"), "checksums good (1): {lines:?}");
        let indexes: Vec<&str> = reads.iter().map(|l| l[3].as_str()).collect();
        assert_eq!(indexes, ["0xaff0", "0xaff0", "0x802a", "0x802a", "0x1234", "0x1234"], "{lines:?}");
        assert_eq!((reads[5][4].as_str(), reads[5][7].as_str()), ("0xde", "0x80"), "the refusal: IODReadRes, PNIORW: {:?}", reads[5]);
        assert!(reads[1][4] == "0x00" || reads[1][4] == "0", "the I&M0 answer's error code 0: {:?}", reads[1]);
        assert!(lines.iter().all(|l| l[5].is_empty() && l[6].split(',').all(|s| s.is_empty() || s.parse::<u64>().unwrap_or(0) < 4_194_304)), "{lines:?}");
    });
}

#[test]
fn an_order_id_or_a_serial_number_i_m0_cannot_carry_is_refused_before_anything_opens() {
    let refused = |key: &str, value: &str| {
        let mut cfg = ProfinetConfig::new("b0");
        match key {
            "order_id" => cfg.order_id = value.into(),
            _ => cfg.serial_number = value.into(),
        }
        validate(&cfg).err().map(|e| e.to_string())
    };
    assert_eq!(refused("order_id", ""), Some("profinet.order_id ``: 1 to 20 visible characters".into()));
    assert_eq!(refused("order_id", &"o".repeat(21)), Some(format!("profinet.order_id `{}`: 1 to 20 visible characters", "o".repeat(21))));
    assert_eq!(refused("order_id", "bancada ç"), Some("profinet.order_id `bancada ç`: 1 to 20 visible characters".into()));
    assert_eq!(refused("serial_number", &"1".repeat(17)), Some(format!("profinet.serial_number `{}`: up to 16 visible characters", "1".repeat(17))));
    assert_eq!(refused("serial_number", "a\tb"), Some("profinet.serial_number `a\tb`: up to 16 visible characters".into()));
    assert_eq!((refused("order_id", &"o".repeat(20)), refused("serial_number", &"1".repeat(16)), refused("serial_number", "")), (None, None, None));
}

/// C-116's first refute round: the critic's probes, kept as pins (ids of every subtype as text,
/// past ASCII and long; another station's shutdown; the cut; another slot and API; another
/// operation, interface and packet type; the mute; the activity; ArgsMaximum; the last fragment,
/// the unspecified address and a computed-zero checksum).
mod round_1 {
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    use std::time::{Duration, Instant};

    use veredas_aranha::netns;
    use veredas_aranha::profinet::ether;
    use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::profinet::record::{INDEX_IM0, INDEX_PORT_DATA_REAL, PortDataReal};
    use veredas_aranha::profinet::rpc::{self, ByteOrder, PNIO_PORT, PnioStatus, ReadRequest, ReadResponse, ResponseArgs, RpcPdu, Uuid};
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_maquinar::profinet::ip;
    use veredas_maquinar::profinet::*;

    macro_rules! note {
        ($($t:tt)*) => {{
            eprintln!($($t)*);
        }};
    }

    const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nport = \"port-002\"\norder_id = \"6ES7 bench\"\nserial_number = \"SN-0001\"\nhello = false\nlldp_interval_ms = 0\n";

    fn config(text: &str) -> ProfinetConfig {
        toml::from_str(text).unwrap()
    }

    fn host_address() {
        for args in [&["ip", "addr", "add", "192.168.0.1/24", "dev", "a0"][..], &["ethtool", "-K", "a0", "tx", "off"][..]] {
            let out = std::process::Command::new(args[0]).args(&args[1..]).output().unwrap();
            assert!(out.status.success());
        }
    }

    fn socket() -> UdpSocket {
        let s = UdpSocket::bind("192.168.0.1:0").unwrap();
        s.set_read_timeout(Some(Duration::from_millis(700))).unwrap();
        s
    }

    fn send_pdu(s: &UdpSocket, pdu: &RpcPdu) -> Option<(RpcPdu, ResponseArgs)> {
        s.send_to(&pdu.encode().unwrap(), ("192.168.0.10", PNIO_PORT)).unwrap();
        let mut buf = [0u8; 4096];
        let (n, from) = s.recv_from(&mut buf).ok()?;
        assert_eq!(from, SocketAddr::from((Ipv4Addr::new(192, 168, 0, 10), PNIO_PORT)));
        let answer = RpcPdu::decode(&buf[..n]).unwrap();
        let args = ResponseArgs::decode(&answer.body, answer.byte_order()).unwrap();
        Some((answer, args))
    }

    fn read_req(s: &UdpSocket, req: &ReadRequest, args_max: u32, activity: Uuid) -> Option<(RpcPdu, ResponseArgs)> {
        let pdu = rpc::read_implicit(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), activity, u32::from(req.seq), req, args_max).unwrap();
        send_pdu(s, &pdu)
    }

    fn wait(within: Duration, mut done: impl FnMut() -> bool) -> bool {
        let begun = Instant::now();
        while begun.elapsed() < within {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    fn port_data(s: &UdpSocket, seq: u16) -> (PnioStatus, Vec<u8>) {
        let (_, args) = read_req(s, &ReadRequest::implicit(seq, 0, SUBSLOT_PORT, INDEX_PORT_DATA_REAL, 4096), 4160, Uuid([seq as u8; 16])).expect("an answer");
        if args.data.is_empty() {
            return (args.status, vec![]);
        }
        let (_, record) = ReadResponse::split(&args.data).unwrap();
        (args.status, record)
    }

    /// P1: a switch whose LLDP port id is an interface name (subtype 5, what most managed switches send).
    #[test]
    fn port_and_chassis_ids_of_every_subtype_read_as_text() {
        netns::isolated("round_1::port_and_chassis_ids_of_every_subtype_read_as_text", &[("a0", "b0")], || {
            host_address();
            let dev = start(&config(DEVICE)).unwrap();
            let raw = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_LLDP), promiscuous: false, outgoing: false }).unwrap();
            let s = socket();
            let sw = raw.mac();
            raw.send(&lldp::frame(sw, &LldpPdu::new(ChassisId::Mac { mac: sw }, PortId::IfName { name: "Gi1/0/3".into() }, 120)).unwrap()).unwrap();
            assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some()));
            let (st, rec) = port_data(&s, 1);
            let pd = PortDataReal::from_block(&rpc::read_blocks(&rec).0[0]).unwrap();
            note!("PROBE p1: status {:?} peer port id {:?}", st.to_bytes(), pd.peers[0].port_id);
            assert_eq!(pd.peers[0].port_id, "Gi1/0/3", "the port id as text");
            // A port named by its MAC, then a chassis by its interface name.
            raw.send(&lldp::frame(sw, &LldpPdu::new(ChassisId::IfName { name: "sw-core".into() }, PortId::Mac { mac: sw }, 120)).unwrap()).unwrap();
            assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some_and(|p| p.chassis_id == "sw-core")));
            let (_, rec) = port_data(&s, 2);
            let pd = PortDataReal::from_block(&rpc::read_blocks(&rec).0[0]).unwrap();
            assert_eq!((pd.peers[0].chassis_id.as_str(), pd.peers[0].port_id.clone()), ("sw-core", sw.to_string()));
        });
    }

    /// P2: a neighbour whose local chassis id is UTF-8 text outside ASCII (LLDP allows it).
    #[test]
    fn a_chassis_past_ascii_is_written_and_the_port_data_answered() {
        netns::isolated("round_1::a_chassis_past_ascii_is_written_and_the_port_data_answered", &[("a0", "b0")], || {
            host_address();
            let dev = start(&config(DEVICE)).unwrap();
            let raw = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_LLDP), promiscuous: false, outgoing: false }).unwrap();
            let s = socket();
            let sw = raw.mac();
            raw.send(&lldp::frame(sw, &LldpPdu::new(ChassisId::Local { id: "schalter-ü".into() }, PortId::Local { id: "port-003".into() }, 120)).unwrap()).unwrap();
            assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some()));
            let (st, rec) = port_data(&s, 1);
            let status = dev.status();
            note!("PROBE p2: status {:02X?} record {} bytes; reads {} refused {}", st.to_bytes(), rec.len(), status.reads, status.reads_refused);
            assert_eq!(st, PnioStatus::OK, "the port's data is a record the device holds");
            let pd = PortDataReal::from_block(&rpc::read_blocks(&rec).0[0]).unwrap();
            assert_eq!(pd.peers[0].chassis_id, "schalter-\\xc3\\xbc", "the bytes past ASCII written as hex");
            assert_eq!((status.reads, status.reads_refused), (1, 0));
        });
    }

    /// P3: neighbour A heard; then another station's (B's) shutdown frame.
    #[test]
    fn another_station_s_shutdown_leaves_the_neighbour() {
        netns::isolated("round_1::another_station_s_shutdown_leaves_the_neighbour", &[("a0", "b0")], || {
            host_address();
            let dev = start(&config(DEVICE)).unwrap();
            let raw = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_LLDP), promiscuous: false, outgoing: false }).unwrap();
            let a = raw.mac();
            raw.send(&lldp::frame(a, &LldpPdu::new(ChassisId::Local { id: "switch-1".into() }, PortId::Local { id: "port-003".into() }, 120)).unwrap()).unwrap();
            assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some()));
            let b = Mac([0x02, 0xBB, 0, 0, 0, 0x01]);
            raw.send(&lldp::frame(b, &LldpPdu::new(ChassisId::Local { id: "switch-2".into() }, PortId::Local { id: "port-009".into() }, 0)).unwrap()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            note!("PROBE p3: neighbour after B's TTL-0: {:?}", dev.neighbour());
            assert!(dev.neighbour().is_some_and(|p| p.chassis_id == "switch-1"), "A stays until A's own TTL-0 frame");
        });
    }

    /// P4: the record cut to the length asked (a passing probe the official test lacks).
    #[test]
    fn the_record_is_cut_to_the_length_asked() {
        netns::isolated("round_1::the_record_is_cut_to_the_length_asked", &[("a0", "b0")], || {
            host_address();
            let _dev = start(&config(DEVICE)).unwrap();
            let s = socket();
            let (_, full) = read_req(&s, &ReadRequest::implicit(1, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 4160, Uuid([1; 16])).unwrap();
            let (_, full_rec) = ReadResponse::split(&full.data).unwrap();
            let (_, cut) = read_req(&s, &ReadRequest::implicit(2, 0, SUBSLOT_DAP, INDEX_IM0, 10), 4160, Uuid([2; 16])).unwrap();
            let (h, rec) = ReadResponse::split(&cut.data).unwrap();
            note!("PROBE p4: full {} bytes; cut header length {} record {} bytes", full_rec.len(), h.length, rec.len());
            assert_eq!((h.length, rec.as_slice()), (10, &full_rec[..10]));
            let (_, zero) = read_req(&s, &ReadRequest::implicit(3, 0, SUBSLOT_DAP, INDEX_IM0, 0), 4160, Uuid([3; 16])).unwrap();
            let (h, rec) = ReadResponse::split(&zero.data).unwrap();
            assert_eq!((h.length, rec.len()), (0, 0));
        });
    }

    /// P5: I&M0 at slot 1 (the clause: "at slot 0 subslot 1"); P6: at API 1.
    #[test]
    fn another_slot_or_api_is_refused() {
        netns::isolated("round_1::another_slot_or_api_is_refused", &[("a0", "b0")], || {
            host_address();
            let _dev = start(&config(DEVICE)).unwrap();
            let s = socket();
            let (_, at_slot1) = read_req(&s, &ReadRequest::implicit(1, 1, SUBSLOT_DAP, INDEX_IM0, 4096), 4160, Uuid([1; 16])).unwrap();
            let mut api1 = ReadRequest::implicit(2, 0, SUBSLOT_DAP, INDEX_IM0, 4096);
            api1.api = 1;
            let (_, at_api1) = read_req(&s, &api1, 4160, Uuid([2; 16])).unwrap();
            note!("PROBE p5: slot 1 {:02X?} ({} data bytes); api 1 {:02X?} ({} data bytes)", at_slot1.status.to_bytes(), at_slot1.data.len(), at_api1.status.to_bytes(), at_api1.data.len());
            assert_eq!(at_slot1.status.to_bytes(), [0xDE, 0x80, 0xB2, 0x00], "I&M0 at slot 1 refused");
            assert_eq!(at_api1.status.to_bytes(), [0xDE, 0x80, 0xB4, 0x00], "I&M0 at API 1 refused: invalid area/API");
        });
    }

    /// P7: a Read (opnum 2, under an AR the device does not have) carrying a read request block.
    #[test]
    fn another_operation_interface_or_packet_type_is_not_answered() {
        netns::isolated("round_1::another_operation_interface_or_packet_type_is_not_answered", &[("a0", "b0")], || {
            host_address();
            let _dev = start(&config(DEVICE)).unwrap();
            let s = socket();
            let mut req = ReadRequest::implicit(1, 0, SUBSLOT_DAP, INDEX_IM0, 4096);
            req.ar = Uuid([0x11; 16]);
            let data = req.block().unwrap().encode().unwrap();
            let pdu = RpcPdu::pnio_request(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([7; 16]), 1, rpc::opnum::READ, &rpc::RequestArgs::new(4160, data.clone()));
            let a = send_pdu(&s, &pdu);
            note!("PROBE p7: opnum 2 answered: {:?}", a.as_ref().map(|(_, x)| x.status.to_bytes()));
            assert!(a.is_none(), "another operation is not answered");
            // The same read block under the controller interface.
            let mut pdu = rpc::read_implicit(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([8; 16]), 2, &ReadRequest::implicit(2, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 4160).unwrap();
            pdu.interface = Uuid::PNIO_CONTROLLER;
            let b = send_pdu(&s, &pdu);
            note!("PROBE p7: controller interface answered: {:?}", b.as_ref().map(|(_, x)| x.status.to_bytes()));
            assert!(b.is_none(), "another interface is not answered");
            // A packet of another type (a Response) carrying the read.
            let mut pdu = rpc::read_implicit(ByteOrder::Little, Uuid::pnio_object(1, 9, 7), Uuid([9; 16]), 3, &ReadRequest::implicit(3, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 4160).unwrap();
            pdu.packet_type = rpc::PacketType::Response;
            let c = send_pdu(&s, &pdu);
            note!("PROBE p7: a Response answered: {:?}", c.as_ref().map(|(_, x)| x.status.to_bytes()));
            assert!(c.is_none(), "a packet that is not a request is not answered");
        });
    }

    /// P8: a muted device (C-112: "a muted device silent"; the handle: "Answer nothing").
    #[test]
    fn a_muted_device_still_answers_its_record_read() {
        netns::isolated("round_1::a_muted_device_still_answers_its_record_read", &[("a0", "b0")], || {
            host_address();
            let dev = start(&config(DEVICE)).unwrap();
            dev.set_muted(true);
            let s = socket();
            let a = read_req(&s, &ReadRequest::implicit(1, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 4160, Uuid([1; 16]));
            // Its DCP alone is muted: ARP and the record read go on.
            assert_eq!(a.map(|(_, x)| x.status), Some(PnioStatus::OK), "{:?}", dev.status());
        });
    }

    /// P9: an activity UUID whose first three fields are not palindromes, as a tool makes one.
    #[test]
    fn an_asymmetric_activity_comes_back_in_either_byte_order() {
        netns::isolated("round_1::an_asymmetric_activity_comes_back_in_either_byte_order", &[("a0", "b0")], || {
            host_address();
            let _dev = start(&config(DEVICE)).unwrap();
            let s = socket();
            let act = Uuid([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10]);
            let (answer, _) = read_req(&s, &ReadRequest::implicit(1, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 4160, act).unwrap();
            note!("PROBE p9: activity {} answered {}; server boot {}", act, answer.activity, answer.server_boot);
            assert_eq!(answer.activity, act);
            // Big-endian too.
            let pdu = rpc::read_implicit(ByteOrder::Big, Uuid::pnio_object(1, 9, 7), act, 2, &ReadRequest::implicit(2, 0, SUBSLOT_DAP, 0x1234, 4096), 4160).unwrap();
            let (answer, args) = send_pdu(&s, &pdu).unwrap();
            note!("PROBE p9: big-endian: drep {:?} activity {} status {:02X?}", answer.drep, answer.activity, args.status.to_bytes());
            assert_eq!((answer.activity, answer.sequence, args.status.to_bytes()), (act, 2, [0xDE, 0x80, 0xB0, 0x00]));
        });
    }

    /// P10: args_maximum below the answer (64), the record length 4096.
    #[test]
    fn a_small_args_maximum_cuts_the_record() {
        netns::isolated("round_1::a_small_args_maximum_cuts_the_record", &[("a0", "b0")], || {
            host_address();
            let _dev = start(&config(DEVICE)).unwrap();
            let s = socket();
            let (_, args) = read_req(&s, &ReadRequest::implicit(1, 0, SUBSLOT_DAP, INDEX_IM0, 4096), 64, Uuid([1; 16])).unwrap();
            // 64 bytes of arguments hold the answer's header and nothing more: the record cut to 0.
            let (header, record) = ReadResponse::split(&args.data).unwrap();
            assert_eq!((args.status, args.max_count, args.data.len(), header.length, record.len()), (PnioStatus::OK, 64, 64, 0, 0));
        });
    }

    /// P11..P13: frames built by hand, the parser alone.
    #[test]
    fn a_last_fragment_the_unspecified_address_and_a_zero_checksum() {
        let mac = Mac([0x02, 0, 0, 0, 0, 0x10]);
        let me = Ipv4Addr::new(192, 168, 0, 10);
        let peer = (Mac([0x02, 0, 0, 0, 0, 0x01]), Ipv4Addr::new(192, 168, 0, 1), 50000);
        let good = ip::udp_frame((mac, me, PNIO_PORT), peer, 7, b"hello");
        // The last fragment of a datagram: MF clear, offset 1 (8 bytes); its header checksum rewritten.
        let mut last = good.clone();
        last[14 + 6] = 0x00;
        last[14 + 7] = 0x01;
        refix_ip(&mut last);
        let r_last = ip::udp_to(&last, me, PNIO_PORT);
        // A datagram addressed to 0.0.0.0, to a device that holds no address.
        let zero = ip::udp_frame((mac, Ipv4Addr::UNSPECIFIED, PNIO_PORT), peer, 7, b"hello");
        let r_zero = ip::udp_to(&zero, Ipv4Addr::UNSPECIFIED, PNIO_PORT);
        // An ARP request for 0.0.0.0, to a device that holds no address.
        let mut ask = vec![0xFF; 6];
        ask.extend_from_slice(&peer.0.0);
        ask.extend_from_slice(&[0x08, 0x06, 0, 1, 0x08, 0, 6, 4, 0, 1]);
        ask.extend_from_slice(&peer.0.0);
        ask.extend_from_slice(&peer.1.octets());
        ask.extend_from_slice(&[0; 6]);
        ask.extend_from_slice(&[0, 0, 0, 0]);
        let r_arp0 = ip::arp_reply(&ask, mac, Ipv4Addr::UNSPECIFIED);
        // The ARP reply's fixed bytes and its padding.
        let mut ask_me = ask.clone();
        ask_me[38..42].copy_from_slice(&me.octets());
        let reply = ip::arp_reply(&ask_me, mac, me).unwrap();
        // A payload whose UDP checksum sums to 0: it must travel as 0xFFFF (RFC 768).
        let mut found = None;
        for x in 0..=0xFFFFu16 {
            let p = [b'h', b'i', (x >> 8) as u8, x as u8];
            let f = ip::udp_frame((mac, me, PNIO_PORT), peer, 7, &p);
            let c = u16::from_be_bytes([f[14 + 20 + 6], f[14 + 20 + 7]]);
            if c == 0xFFFF || c == 0 {
                found = Some((x, c));
                break;
            }
        }
        note!("PROBE p11: last fragment taken: {}; 0.0.0.0 datagram taken: {}; ARP for 0.0.0.0 answered: {}; reply[12..20] {:02X?} padding zero: {}; zero-sum checksum written {:?}",
            r_last.is_some(), r_zero.is_some(), r_arp0.is_some(), &reply[12..20], reply[42..].iter().all(|&b| b == 0), found);
        assert!(r_last.is_none(), "a last fragment is a fragment");
        assert!(r_zero.is_none(), "no address, no datagram");
        assert!(r_arp0.is_none(), "no address, no ARP");
        assert_eq!(&reply[12..20], &[0x08, 0x06, 0, 1, 0x08, 0, 6, 4]);
        assert!(reply[42..].iter().all(|&b| b == 0));
        assert_eq!(found.map(|f| f.1), Some(0xFFFF), "a computed 0 travels as 0xFFFF");
    }

    fn refix_ip(f: &mut [u8]) {
        f[14 + 10] = 0;
        f[14 + 11] = 0;
        let mut acc: u32 = 0;
        for c in f[14..34].chunks(2) {
            acc += u32::from(u16::from_be_bytes([c[0], c[1]]));
        }
        while acc > 0xFFFF {
            acc = (acc & 0xFFFF) + (acc >> 16);
        }
        let c = !(acc as u16);
        f[14 + 10..14 + 12].copy_from_slice(&c.to_be_bytes());
    }

    /// P2b: a chassis id of another subtype (1, chassis component) with a long payload.
    #[test]
    fn a_long_chassis_of_another_subtype_is_written_as_hex() {
        netns::isolated("round_1::a_long_chassis_of_another_subtype_is_written_as_hex", &[("a0", "b0")], || {
            host_address();
            let dev = start(&config(DEVICE)).unwrap();
            let raw = RawLink::open_with("a0", Options { ethertype: Some(ether::ETHERTYPE_LLDP), promiscuous: false, outgoing: false }).unwrap();
            let s = socket();
            let sw = raw.mac();
            raw.send(&lldp::frame(sw, &LldpPdu::new(ChassisId::Other { subtype: 1, data: vec![0xAB; 100] }, PortId::Local { id: "port-003".into() }, 120)).unwrap()).unwrap();
            assert!(wait(Duration::from_secs(2), || dev.neighbour().is_some()));
            let (st, rec) = port_data(&s, 1);
            assert_eq!(st, PnioStatus::OK);
            let pd = PortDataReal::from_block(&rpc::read_blocks(&rec).0[0]).unwrap();
            assert_eq!(pd.peers[0].chassis_id, "ab".repeat(100), "a chassis no text names, as hex");
        });
    }
}
