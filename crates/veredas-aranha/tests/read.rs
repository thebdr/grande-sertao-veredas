//! The implicit read (contract: the implicit read clause): a record read with no connection, one
//! UDP datagram through the listener hearing the device's interface, from its MAC and address,
//! paced and kept; its answer the datagram the listener keeps from the device under the request's
//! activity; the ports' data read subslot by subslot until the device says one is not there, the
//! neighbours they report drawn by the topology; refused where it cannot be heard, sent or
//! answered. The datagrams are read by the dissectors too, and proven on the bench's devices.

mod common;

use std::net::Ipv4Addr;
use std::time::Duration;

use veredas_aranha::fusion::{Device, Fusion};
use veredas_aranha::ip::{self, Endpoint, IpError};
use veredas_aranha::listen::{ListenError, Listener, Options};
use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::read::{self, MAX_WAIT, ReadError, Target};
use veredas_aranha::wire::hub::Hub;
use veredas_aranha::wire::{Frame, Wire, WireError};

const HERE: Mac = Mac([0x02, 0, 0, 0, 0, 0xA0]);
const DEV: Mac = Mac([0x02, 0, 0, 0, 0, 0x01]);

fn a(last: u8) -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 0, last)
}

/// tshark's reading of `frames` with the IPv4 and UDP checksums checked, which it leaves unchecked
/// by default: one line per frame of the fields asked, tab-separated.
fn checked(frames: &[Vec<u8>], fields: &[&str]) -> Vec<Vec<String>> {
    let dir = std::env::temp_dir().join(format!("veredas-aranha-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("frames-{}.pcap", frames.len()));
    std::fs::write(&file, common::pcap(frames)).unwrap();
    let mut cmd = std::process::Command::new(common::tshark());
    cmd.args(["-n", "-o", "ip.check_checksum:TRUE", "-o", "udp.check_checksum:TRUE", "-r"]).arg(&file).args(["-T", "fields", "-E", "occurrence=a", "-E", "aggregator=,"]);
    for f in fields {
        cmd.args(["-e", f]);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_file(&file).unwrap();
    assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().map(|l| l.split('\t').map(String::from).collect()).collect()
}

#[test]
fn a_datagram_is_written_whole_and_read_back_only_when_whole() {
    let src = Endpoint { mac: HERE, ip: a(1), port: 49152 };
    let dst = Endpoint { mac: DEV, ip: a(10), port: 34964 };
    for len in [0usize, 1, 17, 18, 19, 300, ip::MAX_UDP_PAYLOAD] {
        let payload: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
        let f = ip::udp_frame(src, dst, 0x1234, &payload).unwrap();
        assert!(f.len() >= 60, "{len}: padded to the shortest frame");
        assert_eq!(f.len(), 60.max(14 + 20 + 8 + len));
        let d = ip::datagram(&f).unwrap();
        assert_eq!((d.src, d.dst, d.id, d.payload), (src, dst, 0x1234, payload), "{len}");
        // The dissectors' reading: addresses, ports, lengths, both checksums good.
        let fields = ["eth.src", "eth.dst", "ip.src", "ip.dst", "ip.id", "ip.flags.df", "ip.ttl", "ip.len", "ip.checksum.status", "udp.srcport", "udp.dstport", "udp.length", "udp.checksum.status"];
        let row = &checked(&[f], &fields)[0];
        let (ip_len, udp_len) = ((28 + len).to_string(), (8 + len).to_string());
        let want = ["02:00:00:00:00:a0", "02:00:00:00:00:01", "192.168.0.1", "192.168.0.10", "0x1234", "True", "64", &ip_len, "1", "49152", "34964", &udp_len, "1"];
        assert_eq!(row, &want, "{len}: {fields:?} (a checksum's status 1 is good)");
    }
    // One past what a frame carries unfragmented, and addresses no datagram goes from or to.
    assert_eq!(ip::udp_frame(src, dst, 0, &vec![0; ip::MAX_UDP_PAYLOAD + 1]), Err(IpError::TooLong(ip::MAX_UDP_PAYLOAD + 1)));
    for bad in [Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, Ipv4Addr::new(224, 0, 0, 1)] {
        assert_eq!(ip::udp_frame(Endpoint { ip: bad, ..src }, dst, 0, b"x"), Err(IpError::Address(bad)));
        assert_eq!(ip::udp_frame(src, Endpoint { ip: bad, ..dst }, 0, b"x"), Err(IpError::Address(bad)));
    }

    // Read only when whole: each damage none.
    let good = ip::udp_frame(src, dst, 7, b"hello, device").unwrap();
    let damaged = |at: usize, v: u8| {
        let mut f = good.clone();
        f[at] = v;
        f
    };
    let header_sum = {
        // A header changed with its checksum written again: only the change refuses it.
        |at: usize, v: u8| {
            let mut f = good.clone();
            f[at] = v;
            f[24] = 0;
            f[25] = 0;
            let mut acc: u32 = f[14..34].chunks(2).map(|p| u32::from(u16::from_be_bytes([p[0], p[1]]))).sum();
            while acc > 0xFFFF {
                acc = (acc & 0xFFFF) + (acc >> 16);
            }
            f[24..26].copy_from_slice(&(!(acc as u16)).to_be_bytes());
            f
        }
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("a header checksum wrong", damaged(25, good[25] ^ 1)),
        ("a payload byte changed (UDP's checksum)", damaged(14 + 28 + 2, b'X')),
        ("more fragments", header_sum(20, 0x20)),
        ("a fragment's offset", header_sum(21, 0x01)),
        ("another protocol (TCP)", header_sum(23, 6)),
        ("version 6", header_sum(14, 0x65)),
        ("a header of 16 bytes", header_sum(14, 0x44)),
        ("a total length past the frame", header_sum(17, 200)),
        ("a total length short of UDP's header", header_sum(17, 24)),
        ("a UDP length past the packet", damaged(14 + 20 + 5, 200)),
        ("a UDP length short of its header", damaged(14 + 20 + 5, 7)),
        ("ARP's type", damaged(13, 0x06)),
        ("cut short", good[..30].to_vec()),
    ];
    for (what, f) in &cases {
        assert_eq!(ip::datagram(f), None, "{what}");
    }
    // No UDP checksum (0) is a datagram; a tag before the type is read through.
    let mut unsummed = good.clone();
    unsummed[14 + 20 + 6] = 0;
    unsummed[14 + 20 + 7] = 0;
    assert_eq!(ip::datagram(&unsummed).unwrap().payload, b"hello, device");
    let mut tagged = good[..12].to_vec();
    tagged.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
    tagged.extend_from_slice(&good[12..]);
    assert_eq!(ip::datagram(&tagged).unwrap().payload, b"hello, device");
}

/// A wire that hears nothing, has no MAC, and never ends.
struct Quiet;

impl Wire for Quiet {
    fn name(&self) -> &str {
        "quiet0"
    }
    fn mac(&self) -> Option<Mac> {
        None
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout.min(Duration::from_millis(5)));
        Ok(None)
    }
}

/// A device as fusion holds it after a Hello heard on `iface`, stating `address` when given.
fn device_on(iface: &str, address: Option<Ipv4Addr>) -> Device {
    let identity = DeviceIdentity { name: Some("bomba-1".into()), ip: address.map(|ip| IpSuite { ip, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
    let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac: DEV, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
    let mut f = Fusion::new();
    f.apply(&o);
    f.devices.remove(&DEV).unwrap()
}

#[test]
fn a_read_is_refused_where_it_cannot_be_heard_sent_or_answered() {
    let hub = Hub::new();
    let l = Listener::start(vec![Box::new(hub.port("hub0", HERE)), Box::new(Quiet)], Options::default()).unwrap();
    let port = Target { slot: 0, subslot: 0x8001, index: 0x802A };
    let quick = read::Options { wait: Duration::from_millis(100) };
    let on = device_on("hub0", Some(a(10)));
    let refusal = |r: Result<read::Record, ReadError>| r.map(|_| ()).unwrap_err().to_string();
    // Where no listener hears, where it heard no MAC of its own, where the device was not heard.
    assert!(matches!(read::read(&l, "eth9", &on, port, &quick), Err(ReadError::Listen(ListenError::NotListening(i))) if i == "eth9"));
    assert_eq!(refusal(read::read(&l, "quiet0", &device_on("quiet0", Some(a(10))), port, &quick)), "\"quiet0\" has no MAC of its own (a capture replayed): nothing is read from it");
    assert_eq!(refusal(read::read(&l, "hub0", &device_on("eth1", Some(a(10))), port, &quick)), "02:00:00:00:00:01 was not heard on \"hub0\": it is not read there");
    // A device that states no address, or 0.0.0.0, has nothing to read it at.
    for none in [None, Some(Ipv4Addr::UNSPECIFIED)] {
        assert_eq!(refusal(read::read(&l, "hub0", &device_on("hub0", none), port, &quick)), "02:00:00:00:00:01 states no IPv4 address of its own (DCP's IP suite): there is nothing to read it at");
    }
    // An interface with no address of its own: a device could not answer it.
    assert_eq!(read::address_of("hub0"), None);
    assert_eq!(refusal(read::read(&l, "hub0", &on, port, &quick)), "\"hub0\" has no IPv4 address of its own: a device could not answer it");
    // A wait past the bound, before anything else.
    let long = read::Options { wait: MAX_WAIT + Duration::from_millis(1) };
    assert!(matches!(read::read(&l, "eth9", &on, port, &long), Err(ReadError::Wait(w)) if w == long.wait));
    // The bound itself is a wait taken: the next refusal is the interface's.
    assert!(matches!(read::read(&l, "eth9", &on, port, &read::Options { wait: MAX_WAIT }), Err(ReadError::Listen(ListenError::NotListening(_)))));
    // Nothing was sent by any refusal.
    assert_eq!(l.interface("hub0").unwrap().sent, 0);
    // Where the listener heard, and hears no more.
    let s = l.stop();
    assert_eq!(s.stats.interfaces["hub0"].sent, 0);
    let replay = veredas_aranha::wire::pcap::Replay::new("cap0", vec![]);
    let l = Listener::start(vec![Box::new(replay)], Options::default()).unwrap();
    assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["cap0"].ended.is_some()));
    assert!(matches!(read::read(&l, "cap0", &device_on("cap0", Some(a(10))), port, &quick), Err(ReadError::Ended { .. })));
    // The subslot past a device's last port: the status the standard gives it.
    assert!(read::is_no_subslot(veredas_aranha::profinet::rpc::PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0x00 }));
    for other in [(0xDE, 0x80, 0xB0), (0xDE, 0x80, 0xB4), (0xDF, 0x80, 0xB2), (0xDE, 0x81, 0xB2)] {
        assert!(!read::is_no_subslot(veredas_aranha::profinet::rpc::PnioStatus { code: other.0, decode: other.1, code1: other.2, code2: 0 }), "{other:x?}");
    }
}

#[cfg(target_os = "linux")]
mod bench;

/// Two devices on one segment whose switch forwards LLDP (a bridge told to), so each hears the
/// other's LLDP and keeps it as its port's neighbour; the second's port named `port-002`, as the
/// bench keeps its one port at subslot 0x8001 whatever its name.
#[cfg(target_os = "linux")]
const SEGMENT: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "first"
interface = "b0"
station = "bomba-1"
vendor_id = 42
device_id = 261
ip = { address = "192.168.0.10", mask = "255.255.255.0" }
lldp_interval_ms = 200
order_id = "VB-0042-261"
seed = 1

[[profinet]]
name = "second"
interface = "c0"
station = "valvula-1"
ip = { address = "192.168.0.11", mask = "255.255.255.0" }
lldp_interval_ms = 200
port = "port-002"
seed = 2
"#;

#[cfg(target_os = "linux")]
#[test]
fn the_bench_s_devices_report_each_other_by_their_port_data_read_through_the_listener() {
    use veredas_aranha::netns;
    use veredas_aranha::profinet::record::{Im0, Peer};
    use veredas_aranha::profinet::rpc::{self, PnioStatus};
    use veredas_aranha::topology::{self, Confidence, End, Interface};

    const FIRST: Mac = Mac([0x02, 0x00, 0x5E, 0xAD, 0x00, 0x01]);
    const SECOND: Mac = Mac([0x02, 0x00, 0x5E, 0xAD, 0x00, 0x02]);
    const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xAD, 0x00, 0xA0]);

    // Built on the host, before the private network is made.
    bench::maquinar();
    let pairs = [("a0", "ap"), ("b0", "bp"), ("c0", "cp")];
    netns::isolated("the_bench_s_devices_report_each_other_by_their_port_data_read_through_the_listener", &pairs, || {
        bench::pin(&[("a0", HERE), ("b0", FIRST), ("c0", SECOND)], &pairs);
        // One segment, its bridge forwarding LLDP's group (bit 14 of its mask) as a switch does not.
        netns::ip(&["link", "add", "br0", "type", "bridge", "group_fwd_mask", "0x4000"]);
        for port in ["ap", "bp", "cp"] {
            netns::ip(&["link", "set", port, "master", "br0"]);
        }
        netns::ip(&["link", "set", "br0", "up"]);
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        netns::wait_pairs(&[("a0", "b0"), ("a0", "c0"), ("b0", "c0")]);
        assert_eq!(read::address_of("a0"), Some(a(1)));
        assert_eq!(read::address_of("b0"), None);

        let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
        let run = bench::start("two-on-a-segment", SEGMENT);
        // Each device heard, its port's LLDP three times: each has heard the other's by then.
        let heard = |s: &veredas_aranha::listen::Snapshot| [FIRST, SECOND].iter().all(|m| s.fusion.devices.get(m).is_some_and(|d| d.ip.current().is_some() && d.ports.values().any(|p| p.frames >= 3)));
        assert!(l.until(Duration::from_secs(10), heard), "{:#?}", l.snapshot());
        let s = l.snapshot();
        let (first, second) = (&s.fusion.devices[&FIRST], &s.fusion.devices[&SECOND]);
        let quick = read::Options::default();

        // Each device's ports read, subslot 0x8001 on, until the next is not there.
        let ports_a = read::ports(&l, "a0", first, &quick).unwrap();
        let ports_b = read::ports(&l, "a0", second, &quick).unwrap();
        for (ports, own, peer) in [(&ports_a, "port-001", Peer { port_id: "port-002.valvula-1".into(), chassis_id: "valvula-1".into(), line_delay: 0, mac: SECOND }), (&ports_b, "port-002", Peer { port_id: "port-001.bomba-1".into(), chassis_id: "bomba-1".into(), line_delay: 0, mac: FIRST })] {
            assert_eq!(ports.read.len(), 1, "{ports:#?}");
            let r = &ports.read[0];
            assert_eq!((r.data.slot, r.data.subslot, r.data.own_port_id.as_str()), (0, 0x8001, own));
            assert_eq!(r.data.peers, [peer]);
            assert!(r.data.link_state.up());
            assert!(ports.end.is_some_and(|end| end > r.record.answer), "{ports:#?}");
        }
        // The neighbours they report, and the topology's confirmed link between them; the
        // interface hearing both ports is a segment of several, no cable of its own.
        let mut said = ports_a.neighbours.clone();
        said.extend(ports_b.neighbours.clone());
        assert_eq!(said.len(), 2);
        assert_eq!((said[0].device, said[0].port.as_str(), said[0].chassis.as_str(), said[0].peer_port.as_str(), said[0].peer_mac), (FIRST, "port-001", "valvula-1", "port-002.valvula-1", Some(SECOND)));
        assert_eq!((said[1].device, said[1].port.as_str(), said[1].chassis.as_str(), said[1].peer_port.as_str(), said[1].peer_mac), (SECOND, "port-002", "bomba-1", "port-001.bomba-1", Some(FIRST)));
        let t = topology::build(&s.fusion, &[Interface { name: "a0".into(), mac: Some(HERE) }], &said);
        let links: Vec<(End, End, Confidence)> = t.links.iter().map(|l| (l.a.clone(), l.b.clone(), l.confidence)).collect();
        let end = |m: Mac, p: &str| End { node: topology::device_id(m), port: p.into() };
        assert_eq!(links, [(end(FIRST, "port-001"), end(SECOND, "port-002"), Confidence::Confirmed)], "{t:#?}");

        // Every read: its request kept as sent from this interface's MAC and address, its answer
        // kept as heard from the device's; the dissectors read both as the read asked and answered.
        let reads = [(&ports_a.read[0], FIRST, (261u16, 42u16)), (&ports_b.read[0], SECOND, (0, 0))];
        for (port, device, (device_id, vendor_id)) in reads {
            let r = &port.record;
            let (req, ans) = (l.frame(r.request).unwrap(), l.frame(r.answer).unwrap());
            assert!(req.sent && !ans.sent);
            let (q, d) = (ip::datagram(&req.frame.bytes).unwrap(), ip::datagram(&ans.frame.bytes).unwrap());
            assert_eq!((q.src.mac, q.src.ip, q.dst.mac, q.dst.ip, q.dst.port), (HERE, a(1), device, r.address, 34964));
            assert_eq!((d.src, d.dst), (q.dst, q.src));
            assert!(r.sent_ns <= r.at_ns);
            let fields = ["ip.checksum.status", "udp.checksum.status", "dcerpc.pkt_type", "dcerpc.opnum", "pn_io.index", "pn_io.slot_nr", "pn_io.subslot_nr", "pn_io.own_port_id", "pn_io.peer_chassis_id", "pn_io.peer_port_id", "pn_io.peer_macadd", "dcerpc.obj_id", "pn_io.ar_uuid", "pn_io.record_data_length"];
            let rows = checked(&[req.frame.bytes.clone(), ans.frame.bytes.clone()], &fields);
            // The device's own object (its ids, instance 1), the nil AR: a read with no connection
            // of the port data at slot 0 subslot 0x8001, the most record one frame takes.
            let object = format!("dea00000-6c97-11d1-8271-0001{device_id:04x}{vendor_id:04x}");
            let nil = "00000000-0000-0000-0000-000000000000";
            assert_eq!(rows[0], ["1", "1", "0", "5", "0x802a", "0x0000", "0x8001", "", "", "", "", &object, nil, &read::MAX_RECORD.to_string()]);
            // The answer, as the dissectors read it, is what the read decoded.
            let p = &port.data.peers[0];
            let record_len = r.data.len().to_string();
            let mac = p.mac.to_string();
            assert_eq!(rows[1], ["1", "1", "2", "5", "0x802a", "0x0000,0x0000", "0x8001,0x8001", &port.data.own_port_id, &p.chassis_id, &p.port_id, &mac, &object, nil, &record_len]);
        }
        // Another record, the access point's I&M0, read the same way; an index the port lacks and a
        // subslot the device lacks answered with the status that says so.
        let im0 = read::read(&l, "a0", first, Target { slot: 0, subslot: 1, index: 0xAFF0 }, &quick).unwrap();
        let im0 = Im0::from_block(&rpc::read_blocks(&im0.data).0[0]).unwrap();
        assert_eq!((im0.vendor_id, im0.order_id.as_str()), (42, "VB-0042-261         "));
        let lacks = |target| match read::read(&l, "a0", first, target, &quick) {
            Err(ReadError::Status { device, status, .. }) if device == FIRST => status,
            other => panic!("{other:?}"),
        };
        assert_eq!(lacks(Target { slot: 0, subslot: 0x8001, index: 0xAFF0 }), PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB0, code2: 0 });
        assert_eq!(lacks(Target { slot: 0, subslot: 0x8002, index: 0x802A }), PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0 });

        let (status, out, err) = bench::end(run);
        assert!(status.success(), "the bench: {status}\n{out}\n{err}");
        assert!(out.contains("2 part(s) running"), "{out}");
    });
}

/// What a scripted peer answers a read with, in order.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
enum Reply {
    /// The answer: `record`, its header saying `declared` bytes (the record's own length when
    /// none), of the index asked unless another is given, with this status.
    Answer { record: Vec<u8>, declared: Option<u32>, index: Option<u16>, status: veredas_aranha::profinet::rpc::PnioStatus },
    /// The same answer, but from another MAC, address or port, to another port, under another
    /// activity, or as another packet type.
    FromMac(Mac, Vec<u8>),
    FromIp(Ipv4Addr, Vec<u8>),
    FromPort(u16, Vec<u8>),
    ToPort(u16, Vec<u8>),
    Activity(Vec<u8>),
    Kind(veredas_aranha::profinet::rpc::PacketType, Vec<u8>),
    /// The answer, the same in every byte, on the peer's other interface.
    Elsewhere(Vec<u8>),
    /// Frames of no read, as many.
    Junk(usize),
    /// A pause, in milliseconds, before the next reply.
    Pause(u64),
    /// An answer whose header names this slot and subslot, the index asked.
    Header { slot: u16, subslot: u16 },
}

#[cfg(target_os = "linux")]
fn good(record: &[u8]) -> Reply {
    Reply::Answer { record: record.to_vec(), declared: None, index: None, status: veredas_aranha::profinet::rpc::PnioStatus::OK }
}

#[cfg(target_os = "linux")]
#[test]
fn a_read_takes_its_own_answer_alone_and_refuses_what_is_not_the_record_asked() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use veredas_aranha::netns;
    use veredas_aranha::profinet::record::{LinkState, Peer, PortDataReal};
    use veredas_aranha::profinet::rpc::{self, PacketType, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcPdu, Uuid};

    const PEER: Mac = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0x01]);
    const PEER1: Mac = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0x02]);
    const HERE0: Mac = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0xA0]);
    const HERE1: Mac = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0xA1]);
    let pairs = [("a0", "b0"), ("a1", "b1")];
    netns::isolated("a_read_takes_its_own_answer_alone_and_refuses_what_is_not_the_record_asked", &pairs, || {
        bench::pin(&[("a0", HERE0), ("b0", PEER), ("a1", HERE1), ("b1", PEER1)], &pairs);
        netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
        // The replies by the subslot a read asks for.
        let script: Arc<Mutex<std::collections::BTreeMap<u16, Vec<Reply>>>> = Arc::new(Mutex::new(Default::default()));
        let stop = Arc::new(AtomicBool::new(false));
        // The peer: on each read to its address, the replies scripted for it.
        let peer = {
            let (script, stop) = (script.clone(), stop.clone());
            std::thread::spawn(move || {
                let (mut b0, mut b1) = (bench::link("b0"), bench::link("b1"));
                let me = Endpoint { mac: PEER, ip: a(10), port: 34964 };
                while !stop.load(Ordering::SeqCst) {
                    let Ok(Some(f)) = b0.recv(Duration::from_millis(10)) else { continue };
                    let Some(d) = ip::datagram(&f.bytes) else { continue };
                    if d.dst != me {
                        continue;
                    }
                    let req = RpcPdu::decode(&d.payload).unwrap();
                    let args = RequestArgs::decode(&req.body, req.byte_order()).unwrap();
                    let read = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]).unwrap();
                    let answer = |record: &[u8], declared: Option<u32>, index: Option<u16>, status: PnioStatus| {
                        let mut header = ReadResponse::answering(&read, declared.unwrap_or(record.len() as u32));
                        header.index = index.unwrap_or(read.index);
                        let mut data = header.block().encode().unwrap();
                        data.extend_from_slice(record);
                        RpcPdu::pnio_response(&req, 7, &ResponseArgs::new(status, args.args_maximum, data))
                    };
                    let send = |link: &mut Box<dyn Wire>, from: Endpoint, to: Endpoint, pdu: &RpcPdu| link.send(&ip::udp_frame(from, to, 1, &pdu.encode().unwrap()).unwrap()).unwrap();
                    let replies = script.lock().unwrap().get(&read.subslot).cloned().unwrap_or_default();
                    for reply in replies {
                        let ok = |r: &[u8]| answer(r, None, None, PnioStatus::OK);
                        match reply {
                            Reply::Answer { record, declared, index, status } => send(&mut b0, me, d.src, &answer(&record, declared, index, status)),
                            Reply::FromMac(mac, r) => send(&mut b0, Endpoint { mac, ..me }, d.src, &ok(&r)),
                            Reply::FromIp(ip, r) => send(&mut b0, Endpoint { ip, ..me }, d.src, &ok(&r)),
                            Reply::FromPort(port, r) => send(&mut b0, Endpoint { port, ..me }, d.src, &ok(&r)),
                            Reply::ToPort(port, r) => send(&mut b0, me, Endpoint { port, ..d.src }, &ok(&r)),
                            Reply::Activity(r) => send(&mut b0, me, d.src, &RpcPdu { activity: Uuid([9; 16]), ..ok(&r) }),
                            Reply::Kind(kind, r) => send(&mut b0, me, d.src, &RpcPdu { packet_type: kind, ..ok(&r) }),
                            // Its own answer to the byte, but on the other interface.
                            Reply::Elsewhere(r) => send(&mut b1, me, d.src, &ok(&r)),
                            Reply::Pause(ms) => std::thread::sleep(Duration::from_millis(ms)),
                            Reply::Header { slot, subslot } => {
                                let mut header = ReadResponse::answering(&read, 1);
                                (header.slot, header.subslot) = (slot, subslot);
                                let mut data = header.block().encode().unwrap();
                                data.push(b'x');
                                send(&mut b0, me, d.src, &RpcPdu::pnio_response(&req, 7, &ResponseArgs::new(PnioStatus::OK, args.args_maximum, data)));
                            }
                            Reply::Junk(n) => {
                                for i in 0..n {
                                    b0.send(&ip::udp_frame(me, Endpoint { port: 9, ..d.src }, i as u16, b"junk").unwrap()).unwrap();
                                }
                            }
                        }
                    }
                }
            })
        };
        // The second interface promiscuous, as on a mirror port: it hears what is addressed to the
        // first.
        let mirror = veredas_aranha::wire::raw::RawLink::open_with("a1", veredas_aranha::wire::raw::Options { ethertype: None, promiscuous: true, outgoing: false }).unwrap();
        let l = Listener::start(vec![bench::link("a0"), Box::new(mirror)], Options::default()).unwrap();
        let mut device = device_on("a0", Some(a(10)));
        device.mac = PEER;
        let im0 = Target { slot: 0, subslot: 1, index: 0xAFF0 };
        let quick = read::Options { wait: Duration::from_millis(400) };
        let set = |by_subslot: Vec<(u16, Vec<Reply>)>| *script.lock().unwrap() = by_subslot.into_iter().collect();
        let run = |replies: Vec<Reply>, target: Target| {
            set(vec![(target.subslot, replies)]);
            read::read(&l, "a0", &device, target, &quick)
        };

        // Its own answer alone: from the device's MAC, address and port, to the request's address
        // and port, under its activity, a response, on its interface; a datagram unlike it in any
        // one of those is passed over, and the answer read is the last one sent.
        let other = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0x77]);
        let before = vec![
            // Its own answer to the byte, on the other interface, and kept before the rest.
            Reply::Elsewhere(b"elsewhere".to_vec()),
            Reply::Pause(150),
            Reply::FromMac(other, b"mac".to_vec()),
            Reply::FromIp(a(99), b"address".to_vec()),
            Reply::FromPort(34965, b"port".to_vec()),
            Reply::ToPort(9, b"to".to_vec()),
            Reply::Activity(b"activity".to_vec()),
            Reply::Kind(PacketType::Working, b"working".to_vec()),
            good(b"mine"),
        ];
        let r = run(before, im0).unwrap();
        assert_eq!(r.data, b"mine");
        let heard = l.frames_from(r.request + 1);
        let mine: Vec<u64> = heard.iter().filter(|(_, k)| !k.sent && ip::datagram(&k.frame.bytes).is_some_and(|d| d.payload.windows(4).any(|w| w == b"mine"))).map(|(i, _)| *i).collect();
        assert_eq!(mine, [r.answer]);
        let carrying = |tag: &[u8]| heard.iter().filter(|(_, k)| ip::datagram(&k.frame.bytes).is_some_and(|d| d.payload.windows(tag.len()).any(|w| w == tag))).map(|(i, k)| (*i, k.frame.iface.clone())).collect::<Vec<_>>();
        let elsewhere = carrying(b"elsewhere");
        assert_eq!(elsewhere.iter().map(|(_, i)| i.as_str()).collect::<Vec<_>>(), ["a1"], "the answer elsewhere was heard, on a1 alone");
        assert!(elsewhere[0].0 < r.answer, "and kept before the one read");
        // A fault or a reject is a refusal, named.
        // The frames from the device kept from `from` on, by what they are: the frame a refusal
        // names is the device's offending datagram, never the request.
        let from_device = |from: u64, what: &dyn Fn(&RpcPdu) -> bool| -> Vec<u64> {
            l.frames_from(from)
                .into_iter()
                .filter(|(_, k)| !k.sent)
                .filter(|(_, k)| ip::datagram(&k.frame.bytes).is_some_and(|d| d.src.mac == PEER && RpcPdu::decode(&d.payload).is_ok_and(|p| what(&p))))
                .map(|(i, _)| i)
                .collect()
        };
        for kind in [PacketType::Fault, PacketType::Reject] {
            let from = l.kept().1;
            match run(vec![Reply::Kind(kind, b"no".to_vec())], im0) {
                Err(ReadError::Rejected { device: PEER, kind: k, frame }) if k == kind => assert_eq!(vec![frame], from_device(from, &|p| p.packet_type == kind)),
                other => panic!("{other:?}"),
            }
        }
        // Another record's header, or a record shorter than its header says: refused, named.
        let refusal = |r: Result<read::Record, ReadError>| match r {
            Err(ReadError::Answer { device: PEER, why, .. }) => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(refusal(run(vec![Reply::Answer { record: b"x".to_vec(), declared: None, index: Some(0xAFF1), status: PnioStatus::OK }], im0)), "it answers slot 0 subslot 0x0001 index 0xaff1");
        assert_eq!(refusal(run(vec![Reply::Header { slot: 0, subslot: 2 }], im0)), "it answers slot 0 subslot 0x0002 index 0xaff0");
        assert_eq!(refusal(run(vec![Reply::Header { slot: 1, subslot: 1 }], im0)), "it answers slot 1 subslot 0x0001 index 0xaff0");
        let from = l.kept().1;
        match run(vec![Reply::Header { slot: 0, subslot: 2 }], im0) {
            Err(ReadError::Answer { frame, .. }) => assert_eq!(vec![frame], from_device(from, &|p| p.packet_type == PacketType::Response)),
            other => panic!("{other:?}"),
        }
        assert_eq!(refusal(run(vec![Reply::Answer { record: b"xy".to_vec(), declared: Some(3), index: None, status: PnioStatus::OK }], im0)), "its header says 3 bytes of record and 2 follow");
        // A record longer than its header says is read to its header's length.
        assert_eq!(run(vec![Reply::Answer { record: b"abcdef".to_vec(), declared: Some(4), index: None, status: PnioStatus::OK }], im0).unwrap().data, b"abcd");
        // No answer within the wait.
        assert!(matches!(run(vec![], im0), Err(ReadError::NoAnswer { device: PEER, waited }) if waited == quick.wait));

        // A port's data of another subslot than asked is refused; a peer of an all-zero MAC has none.
        let port_data_of = |slot: u16, subslot: u16, peers: Vec<Peer>| {
            let p = PortDataReal {
                slot,
                subslot,
                own_port_id: format!("port-{:03}", subslot - 0x8000),
                peers,
                mau_type: 0x10,
                domain_boundary: 0,
                multicast_boundary: 0,
                link_state: LinkState { port: 0, link: 1 },
                media_type: 1,
                extra: Vec::new(),
            };
            p.block().unwrap().encode().unwrap()
        };
        let peer_of = |mac: Mac| Peer { port_id: "port-009.sw".into(), chassis_id: "sw".into(), line_delay: 0, mac };
        let port_data = |subslot: u16, peer_mac: Mac| port_data_of(0, subslot, vec![peer_of(peer_mac)]);
        let lacks = || Reply::Answer { record: Vec::new(), declared: None, index: None, status: PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0 } };
        set(vec![(0x8001, vec![good(&port_data(0x8005, PEER1))])]);
        match read::ports(&l, "a0", &device, &quick) {
            Err((0x8001, ReadError::Answer { why, .. })) => assert_eq!(why, "its port data is of slot 0 subslot 0x8005"),
            other => panic!("{other:?}"),
        }
        set(vec![(0x8001, vec![good(&port_data(0x8001, Mac([0; 6])))]), (0x8002, vec![good(&port_data(0x8002, PEER1))]), (0x8003, vec![lacks()])]);
        let from = l.kept().1;
        let ports = read::ports(&l, "a0", &device, &quick).unwrap();
        assert_eq!(ports.read.iter().map(|r| r.data.subslot).collect::<Vec<_>>(), [0x8001, 0x8002]);
        assert_eq!(ports.neighbours.iter().map(|n| n.peer_mac).collect::<Vec<_>>(), [None, Some(PEER1)]);
        // The end is the device's answer saying the subslot is not there.
        let answers = from_device(from, &|p| p.packet_type == PacketType::Response);
        assert_eq!((answers.len(), ports.end), (3, answers.last().copied()));
        // Port data of another slot at the subslot asked is refused; two peers are two neighbours.
        set(vec![(0x8001, vec![good(&port_data_of(1, 0x8001, vec![peer_of(PEER1)]))])]);
        match read::ports(&l, "a0", &device, &quick) {
            Err((0x8001, ReadError::Answer { why, .. })) => assert_eq!(why, "its port data is of slot 1 subslot 0x8001"),
            other => panic!("{other:?}"),
        }
        let other = Mac([0x02, 0x00, 0x5E, 0xAE, 0x00, 0x09]);
        set(vec![(0x8001, vec![good(&port_data_of(0, 0x8001, vec![peer_of(PEER1), peer_of(other)]))]), (0x8002, vec![lacks()])]);
        let two = read::ports(&l, "a0", &device, &quick).unwrap();
        assert_eq!(two.neighbours.iter().map(|n| (n.port.as_str(), n.peer_mac)).collect::<Vec<_>>(), [("port-001", Some(PEER1)), ("port-001", Some(other))]);
        // A device answering every subslot: read to the sixteenth, 0x8010, and no further, no end.
        set((0x8001..=0x8011).map(|sub| (sub, vec![good(&port_data(sub, PEER1))])).collect());
        let from = l.kept().1;
        let all = read::ports(&l, "a0", &device, &quick).unwrap();
        assert_eq!((all.read.len(), all.read.last().map(|r| r.data.subslot), all.end), (16, Some(0x8010), None));
        let asked = l.frames_from(from).into_iter().filter(|(_, k)| k.sent && ip::datagram(&k.frame.bytes).is_some_and(|d| d.dst.port == 34964)).count();
        assert_eq!(asked, 16);

        // Frames let go past the listener's bound after the request, before its answer was read:
        // its answer may be among them, and the read says so.
        let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 8, ..Options::default() }).unwrap();
        set(vec![(1, vec![Reply::Junk(40), good(b"late")])]);
        assert!(matches!(read::read(&small, "a0", &device, im0, &quick), Err(ReadError::Lost(n)) if n > 0));
        small.stop();
        stop.store(true, Ordering::SeqCst);
        peer.join().unwrap();
        l.stop();
    });
}

/// The first refute round's cases (its critic's demonstrations, taken in): the read's turn, the
/// port held open, the device's object, an answer in the other byte order or to another address
/// or MAC, a checksum of zero, an IPv4 header of 24 bytes, a UDP length short of its packet, and
/// the datagram reader's totality.
mod round_1 {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const SOURCE: Mac = Mac([0x02, 0x00, 0x5E, 0xB0, 0x00, 0xA0]);
    const DEVICE: Mac = Mac([0x02, 0x00, 0x5E, 0xB0, 0x00, 0x01]);

    fn device_of(iface: &str, ids: Option<(u16, u16)>, instance: Option<(u8, u8)>) -> Device {
        let identity = DeviceIdentity {
            name: Some("bomba-1".into()),
            ip: Some(IpSuite { ip: a(10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
            vendor_id: ids.map(|i| i.0),
            device_id: ids.map(|i| i.1),
            instance,
            ..Default::default()
        };
        let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac: DEVICE, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
        let mut f = Fusion::new();
        f.apply(&o);
        f.devices.remove(&DEVICE).unwrap()
    }

    /// A datagram whose UDP checksum computes to 0 carries 0xFFFF (0 says none), and the
    /// dissectors read it good.
    #[test]
    fn a_checksum_of_zero_travels_as_ffff() {
        let f = ip::udp_frame(Endpoint { mac: SOURCE, ip: a(1), port: 49152 }, Endpoint { mac: DEVICE, ip: a(10), port: 34964 }, 0, &[0x35, 0xE9]).unwrap();
        assert_eq!(&f[40..42], &[0xFF, 0xFF]);
        assert_eq!(checked(std::slice::from_ref(&f), &["udp.checksum.status"])[0], ["1"]);
        assert_eq!(ip::datagram(&f).unwrap().payload, [0x35, 0xE9]);
    }

    /// The header's checksum of `ip` written again.
    fn resum(ip: &mut [u8]) {
        ip[10] = 0;
        ip[11] = 0;
        let mut acc: u32 = ip.chunks(2).map(|p| u32::from(u16::from_be_bytes([p[0], p[1]]))).sum();
        while acc > 0xFFFF {
            acc = (acc & 0xFFFF) + (acc >> 16);
        }
        ip[10..12].copy_from_slice(&(!(acc as u16)).to_be_bytes());
    }

    #[test]
    fn a_header_of_24_bytes_is_read_through_and_a_datagram_to_its_own_length() {
        let payload = b"hello, device, a longer payload";
        let good = ip::udp_frame(Endpoint { mac: SOURCE, ip: a(1), port: 49152 }, Endpoint { mac: DEVICE, ip: a(10), port: 34964 }, 7, payload).unwrap();
        // One option word: the header 24 bytes, its total 4 longer, its checksum written again.
        let mut iph = good[14..34].to_vec();
        iph[0] = 0x46;
        let total = u16::from_be_bytes([iph[2], iph[3]]) + 4;
        iph[2..4].copy_from_slice(&total.to_be_bytes());
        iph.extend_from_slice(&[1, 1, 1, 0]);
        resum(&mut iph);
        let mut opt = good[..14].to_vec();
        opt.extend_from_slice(&iph);
        opt.extend_from_slice(&good[34..14 + 20 + 8 + payload.len()]);
        let d = ip::datagram(&opt).unwrap();
        assert_eq!((d.payload.as_slice(), d.src.port, d.dst.port), (&payload[..], 49152, 34964));
        assert_eq!(checked(&[opt], &["ip.hdr_len", "udp.checksum.status", "udp.length"])[0], ["24", "1", &(8 + payload.len()).to_string()]);
        // An IPv4 packet longer than its UDP datagram: the datagram read to its own length (the UDP
        // checksum then absent, as the bytes past it are no part of it).
        let mut long = good[..14 + 20 + 8 + payload.len()].to_vec();
        long.extend_from_slice(b"tail");
        let total = (20 + 8 + payload.len() + 4) as u16;
        long[16..18].copy_from_slice(&total.to_be_bytes());
        long[40] = 0;
        long[41] = 0;
        let mut iph = long[14..34].to_vec();
        resum(&mut iph);
        long[14..34].copy_from_slice(&iph);
        assert_eq!(ip::datagram(&long).unwrap().payload, payload);
    }

    /// Every cut of a good frame (tagged or not, with an option word), and every byte of its first
    /// sixty set to every value, read without a panic, each datagram read within its frame.
    #[test]
    fn the_datagram_reader_is_total() {
        let good = ip::udp_frame(Endpoint { mac: SOURCE, ip: a(1), port: 49152 }, Endpoint { mac: DEVICE, ip: a(10), port: 34964 }, 7, b"hello, device, a longer payload").unwrap();
        let mut tagged = good[..12].to_vec();
        tagged.extend_from_slice(&[0x81, 0x00, 0x00, 0x05]);
        tagged.extend_from_slice(&good[12..]);
        let mut iph = good[14..34].to_vec();
        iph[0] = 0x46;
        let total = u16::from_be_bytes([iph[2], iph[3]]) + 4;
        iph[2..4].copy_from_slice(&total.to_be_bytes());
        iph.extend_from_slice(&[1, 1, 1, 0]);
        resum(&mut iph);
        let mut opt = good[..14].to_vec();
        opt.extend_from_slice(&iph);
        opt.extend_from_slice(&good[34..]);
        for f in [good, tagged, opt] {
            for cut in 0..=f.len() {
                let _ = ip::datagram(&f[..cut]);
            }
            for at in 0..f.len().min(60) {
                for v in 0..=255u8 {
                    let mut g = f.clone();
                    g[at] = v;
                    if let Some(d) = ip::datagram(&g) {
                        assert!(d.payload.len() + 14 + 20 + 8 <= g.len() + 4, "{at} {v}");
                    }
                }
            }
        }
    }

    /// A wire named as the private network's addressed interface: it takes `block` to send each
    /// frame, refuses them when told to, ends (an error on its next read) once `dies` has passed,
    /// and hears nothing. It counts what it is handed.
    struct Slow {
        block: Duration,
        refuse: bool,
        dies: Option<std::time::Instant>,
        handed: Arc<AtomicUsize>,
    }

    impl Wire for Slow {
        fn name(&self) -> &str {
            "a0"
        }
        fn mac(&self) -> Option<Mac> {
            Some(SOURCE)
        }
        fn send(&mut self, f: &[u8]) -> Result<(), WireError> {
            self.handed.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.block);
            if self.refuse { Err(WireError::TooShort(f.len())) } else { Ok(()) }
        }
        fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
            if self.dies.is_some_and(|d| std::time::Instant::now() >= d) {
                return Err(WireError::Ended("a0".into()));
            }
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok(None)
        }
    }

    fn slow(block: Duration, refuse: bool, dies: Option<Duration>) -> (Box<dyn Wire>, Arc<AtomicUsize>) {
        let handed = Arc::new(AtomicUsize::new(0));
        (Box::new(Slow { block, refuse, dies: dies.map(|d| std::time::Instant::now() + d), handed: handed.clone() }), handed)
    }

    /// Sent through the listener in its turn: a read behind a frame its wire stalls on is taken
    /// back and never sent; one its wire stalls on itself is said stalled; one refused, refused; one
    /// whose worker ends while it waits, ended; one behind thirty frames sent in its turn.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_read_goes_in_its_turn_or_is_taken_back_never_sent() {
        use veredas_aranha::netns;
        netns::isolated("round_1::a_read_goes_in_its_turn_or_is_taken_back_never_sent", &[("a0", "b0")], || {
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let device = device_of("a0", None, None);
            let im0 = Target { slot: 0, subslot: 1, index: 0xAFF0 };
            let quick = read::Options { wait: Duration::from_millis(200) };

            // Behind a frame the wire takes 3 s to send: withdrawn past its turn, never handed over.
            let (wire, handed) = slow(Duration::from_secs(3), false, None);
            let l = Listener::start(vec![wire], Options::default()).unwrap();
            l.send("a0", vec![0u8; 60]).unwrap();
            assert!(l.until(Duration::from_secs(2), |s| s.stats.interfaces["a0"].queued == 0));
            let start = std::time::Instant::now();
            assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::NotSent { .. })));
            assert!(start.elapsed() < Duration::from_millis(2500), "{:?}", start.elapsed());
            std::thread::sleep(Duration::from_millis(6500).saturating_sub(start.elapsed()));
            let i = l.interface("a0").unwrap();
            assert_eq!((i.sent, i.withdrawn, handed.load(Ordering::SeqCst)), (1, 1, 1), "the read never went");
            drop(l);

            // Its own frame stalled on: said stalled, past its turn and not before.
            let (wire, _) = slow(Duration::from_millis(2500), false, None);
            let l = Listener::start(vec![wire], Options::default()).unwrap();
            let start = std::time::Instant::now();
            assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::Stalled { .. })));
            assert!(start.elapsed() >= Duration::from_millis(1050), "{:?}", start.elapsed());
            drop(l);

            // Refused by its wire.
            let (wire, _) = slow(Duration::ZERO, true, None);
            let l = Listener::start(vec![wire], Options::default()).unwrap();
            assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::Refused { .. })));
            drop(l);

            // Its worker ending while it waits behind a frame: ended, never stalled.
            let (wire, _) = slow(Duration::from_millis(600), false, Some(Duration::from_millis(300)));
            let l = Listener::start(vec![wire], Options::default()).unwrap();
            l.send("a0", vec![0u8; 60]).unwrap();
            assert!(l.until(Duration::from_secs(2), |s| s.stats.interfaces["a0"].queued == 0));
            assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::Ended { .. })));
            drop(l);

            // Behind thirty frames at twenty a second: its turn is theirs and a second more, so it
            // goes (no answer: nothing hears it).
            let (wire, handed) = slow(Duration::ZERO, false, None);
            let l = Listener::start(vec![wire], Options::default()).unwrap();
            for _ in 0..30 {
                l.send("a0", vec![0u8; 60]).unwrap();
            }
            assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::NoAnswer { .. })));
            assert_eq!((l.interface("a0").unwrap().sent, handed.load(Ordering::SeqCst)), (31, 31));
        });
    }

    /// The answer reaches the port the read holds open, so this machine sends the device no ICMP
    /// port unreachable; the object is the device's own, its instance as stated; an answer in the
    /// other byte order NDR allows is read the same; one to another address or MAC of this machine's
    /// port is passed over.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_read_holds_its_port_open_and_takes_its_answer_whole() {
        use veredas_aranha::netns;
        use veredas_aranha::profinet::rpc::{self, ByteOrder, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcPdu, Uuid};
        let pairs = [("a0", "b0")];
        netns::isolated("round_1::a_read_holds_its_port_open_and_takes_its_answer_whole", &pairs, || {
            bench::pin(&[("a0", SOURCE), ("b0", DEVICE)], &pairs);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            // The kernel may answer the device (an ICMP error) without asking for its MAC first.
            netns::ip(&["neigh", "add", "192.168.0.10", "lladdr", &DEVICE.to_string(), "dev", "a0", "nud", "permanent"]);
            #[derive(Clone, Copy, PartialEq, Debug)]
            enum Way {
                Good,
                BigEndian,
                ToOtherIp,
                ToOtherMac,
            }
            let way = Arc::new(Mutex::new(Way::Good));
            let icmp = Arc::new(AtomicUsize::new(0));
            let objects: Arc<Mutex<Vec<Uuid>>> = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let peer = {
                let (way, icmp, objects, stop) = (way.clone(), icmp.clone(), objects.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut b0 = bench::link("b0");
                    let me = Endpoint { mac: DEVICE, ip: a(10), port: 34964 };
                    while !stop.load(Ordering::SeqCst) {
                        let Ok(Some(f)) = b0.recv(Duration::from_millis(10)) else { continue };
                        let b = &f.bytes;
                        // ICMP (protocol 1) from this machine's address, destination unreachable (3).
                        if b.len() > 35 && b[12..14] == [0x08, 0x00] && b[23] == 1 && b[26..30] == [192, 168, 0, 1] && b[14 + usize::from(b[14] & 0x0F) * 4] == 3 {
                            icmp.fetch_add(1, Ordering::SeqCst);
                            continue;
                        }
                        let Some(d) = ip::datagram(b) else { continue };
                        if d.dst != me {
                            continue;
                        }
                        let req = RpcPdu::decode(&d.payload).unwrap();
                        objects.lock().unwrap().push(req.object);
                        let args = RequestArgs::decode(&req.body, req.byte_order()).unwrap();
                        let read = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]).unwrap();
                        let w = *way.lock().unwrap();
                        let mut data = ReadResponse::answering(&read, 4).block().encode().unwrap();
                        data.extend_from_slice(b"mine");
                        let answer = ResponseArgs::new(PnioStatus::OK, args.args_maximum, data);
                        let mut pdu = RpcPdu::pnio_response(&req, 7, &answer);
                        if w == Way::BigEndian {
                            pdu.drep = ByteOrder::Big.drep();
                            pdu.body = answer.encode(ByteOrder::Big);
                        }
                        let to = match w {
                            Way::ToOtherIp => Endpoint { ip: a(2), ..d.src },
                            Way::ToOtherMac => Endpoint { mac: Mac([0x02, 0, 0, 0, 0, 0x77]), ..d.src },
                            _ => d.src,
                        };
                        b0.send(&ip::udp_frame(me, to, 1, &pdu.encode().unwrap()).unwrap()).unwrap();
                    }
                })
            };
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let im0 = Target { slot: 0, subslot: 1, index: 0xAFF0 };
            let quick = read::Options { wait: Duration::from_millis(400) };
            let set = |w: Way| *way.lock().unwrap() = w;
            // Ids (42, 261) and instance (0x01, 0x02) stated: the object carries them.
            let device = device_of("a0", Some((42, 261)), Some((0x01, 0x02)));
            set(Way::Good);
            assert_eq!(read::read(&l, "a0", &device, im0, &quick).unwrap().data, b"mine");
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(objects.lock().unwrap().last().copied(), Some(Uuid::pnio_object(0x0102, 261, 42)));
            assert_eq!(icmp.load(Ordering::SeqCst), 0, "the answer reached an open port");
            // The other byte order.
            set(Way::BigEndian);
            assert_eq!(read::read(&l, "a0", &device, im0, &quick).unwrap().data, b"mine");
            // To another address or another MAC of this machine's port: passed over.
            for w in [Way::ToOtherIp, Way::ToOtherMac] {
                set(w);
                assert!(matches!(read::read(&l, "a0", &device, im0, &quick), Err(ReadError::NoAnswer { .. })), "{w:?}");
            }
            stop.store(true, Ordering::SeqCst);
            peer.join().unwrap();
            l.stop();
        });
    }
}
