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
    // A file of its own: tests run as threads of one process under `cargo test`.
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file = dir.join(format!("frames-{}.pcap", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
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
            // (The links are open before the read starts, or its first request may go unheard.)
            let (mut b0, mut b1) = (bench::link("b0"), bench::link("b1"));
            std::thread::spawn(move || {
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
        // (From where the listener stood before the read: the peer's copy on a1 may be kept by a1's
        // worker before a0's, having handed the request to the wire, keeps it as sent.)
        let from = l.kept().1;
        let r = run(before, im0).unwrap();
        assert_eq!(r.data, b"mine");
        let heard = l.frames_from(from);
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
        let cut_why = |r: Result<read::Ports, Box<read::Cut>>| match r.map_err(|c| *c) {
            Err(read::Cut { subslot: 0x8001, error: ReadError::Answer { why, .. }, read }) if read.read.is_empty() => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(cut_why(read::ports(&l, "a0", &device, &quick)), "its port data is of slot 0 subslot 0x8005");
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
        assert_eq!(cut_why(read::ports(&l, "a0", &device, &quick)), "its port data is of slot 1 subslot 0x8001");
        // A walk cut by a read unanswered keeps what was read before it: the subslot, why, and the
        // first port with its neighbour.
        set(vec![(0x8001, vec![good(&port_data(0x8001, PEER1))])]);
        match read::ports(&l, "a0", &device, &quick).map_err(|c| *c) {
            Err(read::Cut { subslot: 0x8002, error: ReadError::NoAnswer { device: PEER, .. }, read }) => {
                assert_eq!((read.read.len(), read.neighbours.iter().map(|n| n.peer_mac).collect::<Vec<_>>(), read.end), (1, vec![Some(PEER1)], None));
            }
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
    #[cfg(target_os = "linux")]
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[cfg(target_os = "linux")]
    use std::sync::{Arc, Mutex};

    const SOURCE: Mac = Mac([0x02, 0x00, 0x5E, 0xB0, 0x00, 0xA0]);
    const DEVICE: Mac = Mac([0x02, 0x00, 0x5E, 0xB0, 0x00, 0x01]);

    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
    struct Slow {
        block: Duration,
        refuse: bool,
        dies: Option<std::time::Instant>,
        handed: Arc<AtomicUsize>,
    }

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
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
                let mut b0 = bench::link("b0");
                std::thread::spawn(move || {
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

/// The second refute round's cases (its critic's demonstrations, taken in): a walk cut after its
/// first port by any refusal keeping that port, a malformed first answer refused naming its frame, a
/// fragmented answer never taken whole, two reads at once each taking its own answer, sixteen ports
/// one at a time at the pace under activities of their own and the most record one frame takes, an
/// interface of two addresses, and frames let go after some were read.
#[cfg(target_os = "linux")]
mod round_2 {
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::bench;
    use veredas_aranha::fusion::{Device, Fusion};
    use veredas_aranha::ip::{self, Endpoint};
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::netns;
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::profinet::record::{LinkState, Peer, PortDataReal};
    use veredas_aranha::profinet::rpc::{self, PacketType, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcPdu, flags1};
    use veredas_aranha::read::{self, ReadError, Target};

    pub(super) const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xC0, 0x00, 0xA0]);
    pub(super) const DEV_A: Mac = Mac([0x02, 0x00, 0x5E, 0xC0, 0x00, 0x01]);
    const DEV_B: Mac = Mac([0x02, 0x00, 0x5E, 0xC0, 0x00, 0x02]);
    const PEER1: Mac = Mac([0x02, 0x00, 0x5E, 0xC0, 0x00, 0x09]);

    pub(super) fn a(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 0, last)
    }

    pub(super) fn device_of(iface: &str, mac: Mac, address: Ipv4Addr) -> Device {
        let identity = DeviceIdentity { name: Some("bomba-1".into()), ip: Some(IpSuite { ip: address, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
        let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
        let mut f = Fusion::new();
        f.apply(&o);
        f.devices.remove(&mac).unwrap()
    }

    /// One request a scripted device heard.
    #[derive(Clone)]
    pub(super) struct Req {
        pub(super) from: Endpoint,
        pub(super) to: Endpoint,
        pub(super) pdu: RpcPdu,
        pub(super) args: RequestArgs,
        pub(super) read: ReadRequest,
    }

    /// What a scripted device sends back: from, to, the UDP payload.
    pub(super) type Out = Vec<(Endpoint, Endpoint, Vec<u8>)>;
    pub(super) type Handler = Box<dyn FnMut(&Req) -> Out + Send>;

    pub(super) struct Scripted {
        stop: Arc<AtomicBool>,
        pub(super) icmp: Arc<AtomicUsize>,
        pub(super) seen: Arc<Mutex<Vec<Req>>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Scripted {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }

    /// A device on `link`: every PNIO request to one of `addresses` (port 34964) given to `handler`,
    /// what it returns sent; this machine's ICMP destination unreachable counted.
    pub(super) fn scripted(link: &str, addresses: Vec<Endpoint>, handler: Arc<Mutex<Handler>>) -> Scripted {
        let stop = Arc::new(AtomicBool::new(false));
        let icmp = Arc::new(AtomicUsize::new(0));
        let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
        let link = link.to_string();
        let join = {
            let (stop, icmp, seen) = (stop.clone(), icmp.clone(), seen.clone());
            // (Open before the read starts, or its first request may go unheard.)
            let mut b = bench::link(&link);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok(Some(f)) = b.recv(Duration::from_millis(5)) else { continue };
                    let x = &f.bytes;
                    if x.len() > 35 && x[12..14] == [0x08, 0x00] && x[23] == 1 && x[26..30] == [192, 168, 0, 1] && x[14 + usize::from(x[14] & 0x0F) * 4] == 3 {
                        icmp.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    let Some(d) = ip::datagram(x) else { continue };
                    if !addresses.contains(&d.dst) {
                        continue;
                    }
                    let Ok(pdu) = RpcPdu::decode(&d.payload) else { continue };
                    let args = RequestArgs::decode(&pdu.body, pdu.byte_order()).unwrap();
                    let read = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]).unwrap();
                    let req = Req { from: d.src, to: d.dst, pdu, args, read };
                    seen.lock().unwrap().push(req.clone());
                    let out = (handler.lock().unwrap())(&req);
                    for (from, to, payload) in out {
                        b.send(&ip::udp_frame(from, to, 1, &payload).unwrap()).unwrap();
                    }
                }
            })
        };
        Scripted { stop, icmp, seen, join: Some(join) }
    }

    /// The answer to `r` carrying `record`, its header saying `declared` bytes, with `status`.
    pub(super) fn answer(r: &Req, record: &[u8], status: PnioStatus) -> RpcPdu {
        let mut data = ReadResponse::answering(&r.read, record.len() as u32).block().encode().unwrap();
        data.extend_from_slice(record);
        RpcPdu::pnio_response(&r.pdu, 7, &ResponseArgs::new(status, r.args.args_maximum, data))
    }

    pub(super) fn back(r: &Req, pdu: &RpcPdu) -> (Endpoint, Endpoint, Vec<u8>) {
        (r.to, r.from, pdu.encode().unwrap())
    }

    fn port_data(subslot: u16, peer: Mac) -> Vec<u8> {
        PortDataReal {
            slot: 0,
            subslot,
            own_port_id: format!("port-{:03}", subslot.wrapping_sub(0x8000)),
            peers: vec![Peer { port_id: "port-009.sw".into(), chassis_id: "sw".into(), line_delay: 0, mac: peer }],
            mau_type: 0x10,
            domain_boundary: 0,
            multicast_boundary: 0,
            link_state: LinkState { port: 0, link: 1 },
            media_type: 1,
            extra: Vec::new(),
        }
        .block()
        .unwrap()
        .encode()
        .unwrap()
    }

    const NO_SUBSLOT: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0 };

    /// The frames kept from the device from `from` on whose RPC is a response or reject or fault.
    fn device_answers(l: &Listener, from: u64, mac: Mac) -> Vec<u64> {
        l.frames_from(from).into_iter().filter(|(_, k)| !k.sent && ip::datagram(&k.frame.bytes).is_some_and(|d| d.src.mac == mac && RpcPdu::decode(&d.payload).is_ok_and(|p| matches!(p.packet_type, PacketType::Response | PacketType::Reject | PacketType::Fault)))).map(|(i, _)| i).collect()
    }

    fn frame_of(e: &ReadError) -> Option<u64> {
        match e {
            ReadError::Answer { frame, .. } | ReadError::Status { frame, .. } | ReadError::Rejected { frame, .. } => Some(*frame),
            _ => None,
        }
    }

    /// The ports walk cut at its second subslot, whatever cuts it: the cut names 0x8002, its error the
    /// device's answer at 0x8002, and keeps the first port and its neighbour, no end.
    #[test]
    fn a_walk_cut_at_its_second_port_keeps_the_first_whatever_cuts_it() {
        netns::isolated("round_2::a_walk_cut_at_its_second_port_keeps_the_first_whatever_cuts_it", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            #[derive(Clone, Copy, Debug, PartialEq)]
            enum Cutter {
                OtherSubslot,
                NoBlock,
                NotPortData,
                Status,
                Reject,
            }
            let cutter = Arc::new(Mutex::new(Cutter::OtherSubslot));
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = {
                let cutter = cutter.clone();
                Box::new(move |r: &Req| {
                    if r.read.subslot == 0x8001 {
                        return vec![back(r, &answer(r, &port_data(0x8001, PEER1), PnioStatus::OK))];
                    }
                    if r.read.subslot != 0x8002 {
                        return vec![back(r, &answer(r, &[], NO_SUBSLOT))];
                    }
                    let pdu = match *cutter.lock().unwrap() {
                        Cutter::OtherSubslot => answer(r, &port_data(0x8005, PEER1), PnioStatus::OK),
                        Cutter::NoBlock => answer(r, &[], PnioStatus::OK),
                        Cutter::NotPortData => answer(r, &rpc::Block::new(0x0020, vec![0; 10]).encode().unwrap(), PnioStatus::OK),
                        Cutter::Status => answer(r, &[], PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB0, code2: 0 }),
                        Cutter::Reject => RpcPdu { packet_type: PacketType::Reject, ..answer(r, b"no", PnioStatus::OK) },
                    };
                    vec![back(r, &pdu)]
                })
            };
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let device = device_of("a0", DEV_A, a(10));
            let quick = read::Options { wait: Duration::from_millis(400) };
            let mut bad = Vec::new();
            for c in [Cutter::OtherSubslot, Cutter::NoBlock, Cutter::NotPortData, Cutter::Status, Cutter::Reject] {
                *cutter.lock().unwrap() = c;
                let from = l.kept().1;
                match read::ports(&l, "a0", &device, &quick).map_err(|c| *c) {
                    Err(cut) => {
                        let answers = device_answers(&l, from, DEV_A);
                        let kept: Vec<(u16, Option<Mac>)> = cut.read.read.iter().map(|p| p.data.subslot).zip(cut.read.neighbours.iter().map(|n| n.peer_mac)).collect();
                        let got = (cut.subslot, kept, cut.read.end, frame_of(&cut.error), cut.error.to_string());
                        let want_frame = answers.last().copied();
                        if (got.0, got.1.clone(), got.2, got.3) != (0x8002, vec![(0x8001, Some(PEER1))], None, want_frame) {
                            bad.push(format!("{c:?}: {got:?}, want (0x8002, [(0x8001, PEER1)], None, {want_frame:?})"));
                        }
                    }
                    Ok(p) => bad.push(format!("{c:?}: the walk ended well: {:?} read, end {:?}", p.read.iter().map(|r| r.data.subslot).collect::<Vec<_>>(), p.end)),
                }
            }
            assert!(bad.is_empty(), "{bad:#?}");
        });
    }

    /// An answer whose body is short of its arguments, or whose data is short of the read's header,
    /// or a record short of its header: refused, naming the device's frame, even with a good answer
    /// after it (the answer is the first datagram).
    #[test]
    fn a_short_body_is_refused_naming_its_frame() {
        netns::isolated("round_2::a_short_body_is_refused_naming_its_frame", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            #[derive(Clone, Copy, Debug, PartialEq)]
            enum Short {
                BodyShortOfArgs,
                DataShortOfHeader,
                RecordShortOfHeader,
            }
            let way = Arc::new(Mutex::new(Short::BodyShortOfArgs));
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = {
                let way = way.clone();
                Box::new(move |r: &Req| {
                    let good = answer(r, b"mine", PnioStatus::OK);
                    let first = match *way.lock().unwrap() {
                        Short::BodyShortOfArgs => {
                            let mut p = good.clone();
                            p.body.truncate(p.body.len() - 2);
                            p
                        }
                        Short::DataShortOfHeader => RpcPdu::pnio_response(&r.pdu, 7, &ResponseArgs::new(PnioStatus::OK, r.args.args_maximum, vec![0x80, 0x09, 0, 60, 1, 0])),
                        Short::RecordShortOfHeader => {
                            let mut data = ReadResponse::answering(&r.read, 9).block().encode().unwrap();
                            data.extend_from_slice(b"mine");
                            RpcPdu::pnio_response(&r.pdu, 7, &ResponseArgs::new(PnioStatus::OK, r.args.args_maximum, data))
                        }
                    };
                    vec![back(r, &first), back(r, &good)]
                })
            };
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let device = device_of("a0", DEV_A, a(10));
            let im0 = Target { slot: 0, subslot: 1, index: 0xAFF0 };
            let quick = read::Options { wait: Duration::from_millis(400) };
            let mut bad = Vec::new();
            for w in [Short::BodyShortOfArgs, Short::DataShortOfHeader, Short::RecordShortOfHeader] {
                *way.lock().unwrap() = w;
                let from = l.kept().1;
                let r = read::read(&l, "a0", &device, im0, &quick);
                std::thread::sleep(Duration::from_millis(100));
                let answers = device_answers(&l, from, DEV_A);
                match r {
                    Err(ReadError::Answer { frame, .. }) if Some(frame) == answers.first().copied() => {}
                    other => bad.push(format!("{w:?}: {other:?}, want an Answer refusal naming {:?}", answers.first())),
                }
            }
            assert!(bad.is_empty(), "{bad:#?}");
        });
    }

    /// A response in two fragments (as a device whose fragments are shorter than our datagram may
    /// send): refused, never taken as the whole record.
    #[test]
    fn a_fragmented_answer_is_never_taken_whole() {
        netns::isolated("round_2::a_fragmented_answer_is_never_taken_whole", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let reverse = Arc::new(AtomicBool::new(false));
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = {
                let reverse = reverse.clone();
                Box::new(move |r: &Req| {
                    let whole = answer(r, &[0x5A; 200], PnioStatus::OK);
                    let cut = 60;
                    let p0 = RpcPdu { flags1: flags1::FRAGMENT | flags1::NO_FACK, fragment: 0, body: whole.body[..cut].to_vec(), ..whole.clone() };
                    let p1 = RpcPdu { flags1: flags1::FRAGMENT | flags1::LAST_FRAGMENT | flags1::NO_FACK, fragment: 1, body: whole.body[cut..].to_vec(), ..whole.clone() };
                    if reverse.load(Ordering::SeqCst) { vec![back(r, &p1), back(r, &p0)] } else { vec![back(r, &p0), back(r, &p1)] }
                })
            };
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let device = device_of("a0", DEV_A, a(10));
            let im0 = Target { slot: 0, subslot: 1, index: 0xAFF0 };
            let quick = read::Options { wait: Duration::from_millis(400) };
            for rev in [false, true] {
                reverse.store(rev, Ordering::SeqCst);
                let r = read::read(&l, "a0", &device, im0, &quick);
                assert!(matches!(r, Err(ReadError::Answer { .. })), "reversed {rev}: {r:?}");
            }
        });
    }

    /// Two reads at once on one interface, of two devices and of one device twice: each takes its own
    /// answer though the answers come in the other order, from ports of their own, under activities
    /// of their own, and this machine sends no ICMP.
    #[test]
    fn two_reads_at_once_each_take_their_own() {
        netns::isolated("round_2::two_reads_at_once_each_take_their_own", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            for (ip, mac) in [("192.168.0.10", DEV_A), ("192.168.0.11", DEV_B)] {
                netns::ip(&["neigh", "add", ip, "lladdr", &mac.to_string(), "dev", "a0", "nud", "permanent"]);
            }
            let ea = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let eb = Endpoint { mac: DEV_B, ip: a(11), port: 34964 };
            // Holds every request until two have come, then answers them in the other order, each
            // with its device's MAC and its subslot.
            let handler: Handler = {
                let mut held: Vec<Req> = Vec::new();
                Box::new(move |r: &Req| {
                    held.push(r.clone());
                    if held.len() < 2 {
                        return vec![];
                    }
                    let out = held.iter().rev().map(|q| back(q, &answer(q, format!("{}:{:x}", q.to.mac, q.read.subslot).as_bytes(), PnioStatus::OK))).collect();
                    held.clear();
                    out
                })
            };
            // A raw link on b0 sends with any source MAC, so one thread plays both devices.
            let peer = scripted("b0", vec![ea, eb], Arc::new(Mutex::new(handler)));
            let l = Arc::new(Listener::start(vec![bench::link("a0")], Options::default()).unwrap());
            let quick = read::Options { wait: Duration::from_millis(1500) };
            let im0 = |sub: u16| Target { slot: 0, subslot: sub, index: 0xAFF0 };
            let pairs = [((DEV_A, a(10), 1u16), (DEV_B, a(11), 1u16)), ((DEV_A, a(10), 1u16), (DEV_A, a(10), 2u16))];
            for (x, y) in pairs {
                let go = |(mac, ip, sub): (Mac, Ipv4Addr, u16)| {
                    let l = l.clone();
                    std::thread::spawn(move || read::read(&l, "a0", &device_of("a0", mac, ip), im0(sub), &quick))
                };
                let (hx, hy) = (go(x), go(y));
                let (rx, ry) = (hx.join().unwrap(), hy.join().unwrap());
                assert_eq!(rx.unwrap().data, format!("{}:{:x}", x.0, x.2).into_bytes());
                assert_eq!(ry.unwrap().data, format!("{}:{:x}", y.0, y.2).into_bytes());
            }
            std::thread::sleep(Duration::from_millis(300));
            let seen = peer.seen.lock().unwrap().clone();
            let ports: std::collections::BTreeSet<u16> = seen.iter().map(|r| r.from.port).collect();
            let acts: std::collections::BTreeSet<[u8; 16]> = seen.iter().map(|r| r.pdu.activity.0).collect();
            assert_eq!((seen.len(), ports.len(), acts.len()), (4, 4, 4), "requests, their ports, their activities");
            assert_eq!(peer.icmp.load(Ordering::SeqCst), 0);
        });
    }

    /// Sixteen ports read one at a time at the listener's pace: each request after the answer before
    /// it and at least a gap after the request before it; each under an activity of its own; the
    /// request asks the most record one frame takes (1308 bytes, the arguments' maximum 1372), and a
    /// record of 1308 bytes comes back in one frame, read whole.
    #[test]
    fn sixteen_ports_one_at_a_time_and_the_most_one_frame_takes() {
        netns::isolated("round_2::sixteen_ports_one_at_a_time_and_the_most_one_frame_takes", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = Box::new(move |r: &Req| {
                if r.read.index == 0xAFF0 {
                    // As much record as the request takes, cut to what its arguments leave.
                    let room = r.read.length.min(r.args.args_maximum.saturating_sub(64)) as usize;
                    return vec![back(r, &answer(r, &vec![0xA5; room], PnioStatus::OK))];
                }
                vec![back(r, &answer(r, &port_data(r.read.subslot, PEER1), PnioStatus::OK))]
            });
            let peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let device = device_of("a0", DEV_A, a(10));
            let quick = read::Options { wait: Duration::from_millis(400) };
            let all = read::ports(&l, "a0", &device, &quick).unwrap();
            assert_eq!(all.read.len(), 16);
            let gap = Duration::from_secs(1) / l.options().rate;
            for w in all.read.windows(2) {
                let (p, q) = (&w[0].record, &w[1].record);
                assert!(q.request > p.answer, "one at a time: request {} after answer {}", q.request, p.answer);
                let d = Duration::from_nanos(q.sent_ns - p.sent_ns);
                assert!(d + Duration::from_millis(2) >= gap, "paced: {d:?} between two requests, the gap {gap:?}");
            }
            let seen = peer.seen.lock().unwrap().clone();
            let acts: std::collections::BTreeSet<[u8; 16]> = seen.iter().map(|r| r.pdu.activity.0).collect();
            assert_eq!(acts.len(), 16, "an activity of its own for each request");
            // The most record one frame takes.
            let r = read::read(&l, "a0", &device, Target { slot: 0, subslot: 1, index: 0xAFF0 }, &quick).unwrap();
            let last = peer.seen.lock().unwrap().last().cloned().unwrap();
            assert_eq!((last.read.length, last.args.args_maximum), (1308, 1372), "the record asked and the arguments' maximum");
            assert_eq!(r.data.len(), 1308);
            assert_eq!(l.frame(r.answer).unwrap().frame.bytes.len(), 1514, "one whole frame");
        });
    }

    /// An interface holding two IPv4 addresses, the device's network's first: the read goes from it
    /// and is answered there (round 3 lists the others first).
    #[test]
    fn an_interface_of_two_addresses() {
        netns::isolated("round_2::an_interface_of_two_addresses", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            netns::ip(&["addr", "add", "10.9.9.1/24", "dev", "a0"]);
            assert_eq!(read::address_of("a0"), Some(a(1)));
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = Box::new(move |r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]);
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_of("a0", DEV_A, a(10)), Target { slot: 0, subslot: 1, index: 0xAFF0 }, &read::Options { wait: Duration::from_millis(400) }).unwrap();
            assert_eq!(r.data, b"mine");
        });
    }

    /// Frames let go past the listener's bound after the read has read some of those kept after the
    /// request: still refused as lost, its answer may be among them (a working packet read, then a
    /// burst past the bound, then the answer).
    #[test]
    fn frames_let_go_after_some_were_read_are_lost() {
        netns::isolated("round_2::frames_let_go_after_some_were_read_are_lost", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV_A)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let me = Endpoint { mac: DEV_A, ip: a(10), port: 34964 };
            let handler: Handler = Box::new(move |r: &Req| {
                let working = RpcPdu { packet_type: PacketType::Working, ..answer(r, b"wait", PnioStatus::OK) };
                let mut out = vec![back(r, &working)];
                // (The pause is the peer's: its sends go after the handler returns, so it is done
                // here by sending the working packet first, on a link of its own.)
                let mut b = bench::link("b0");
                b.send(&ip::udp_frame(out[0].0, out[0].1, 1, &out[0].2).unwrap()).unwrap();
                std::thread::sleep(Duration::from_millis(200));
                out.clear();
                for i in 0..40u16 {
                    out.push((me, Endpoint { port: 9, ..r.from }, format!("junk{i}").into_bytes()));
                }
                out.push(back(r, &answer(r, b"late", PnioStatus::OK)));
                out
            });
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 8, ..Options::default() }).unwrap();
            let r = read::read(&small, "a0", &device_of("a0", DEV_A, a(10)), Target { slot: 0, subslot: 1, index: 0xAFF0 }, &read::Options { wait: Duration::from_millis(800) });
            assert!(matches!(r, Err(ReadError::Lost(n)) if n > 0), "{r:?}");
        });
    }
}

/// The third refute round's cases: the wait's bound, the walk's end, the datagram reader's offsets
/// and short header, frames let go before and after the request, the wait counted from the
/// request's going, and the interface's address on the device's network.
mod round_3 {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use veredas_aranha::fusion::{Device, Fusion};
    use veredas_aranha::ip::{self, Endpoint};
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::profinet::rpc::PnioStatus;
    use veredas_aranha::read::{self, ReadError, Target};
    use veredas_aranha::wire::hub::Hub;

    #[cfg(target_os = "linux")]
    use super::bench;
    #[cfg(target_os = "linux")]
    use veredas_aranha::profinet::rpc::{PacketType, flags1};
    #[cfg(target_os = "linux")]
    use super::round_2::{Handler, Req, answer, back, scripted};
    #[cfg(target_os = "linux")]
    use std::sync::{Arc, Mutex};
    #[cfg(target_os = "linux")]
    use veredas_aranha::netns;

    const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xD0, 0x00, 0xA0]);
    const DEV: Mac = Mac([0x02, 0x00, 0x5E, 0xD0, 0x00, 0x01]);
    const MASK24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
    const IM0: Target = Target { slot: 0, subslot: 1, index: 0xAFF0 };

    fn a(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 0, last)
    }

    /// A device heard on `iface` by its DCP Hello, at `address` with `mask`.
    fn device_with(iface: &str, mac: Mac, address: Ipv4Addr, mask: Ipv4Addr) -> Device {
        let identity = DeviceIdentity { name: Some("bomba-1".into()), ip: Some(IpSuite { ip: address, mask, gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
        let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
        let mut f = Fusion::new();
        f.apply(&o);
        f.devices.remove(&mac).unwrap()
    }

    /// A header's checksum computed again.
    fn resum(header: &mut [u8]) {
        header[10..12].copy_from_slice(&[0, 0]);
        let mut acc: u32 = header.chunks(2).map(|p| u32::from(u16::from_be_bytes([p[0], p[1]]))).sum();
        while acc > 0xFFFF {
            acc = (acc & 0xFFFF) + (acc >> 16);
        }
        header[10..12].copy_from_slice(&(!(acc as u16)).to_be_bytes());
    }

    /// A frame of no protocol the read knows, to this machine.
    #[cfg(target_os = "linux")]
    fn junk(i: u16) -> Vec<u8> {
        let mut f = HERE.0.to_vec();
        f.extend_from_slice(&DEV.0);
        f.extend_from_slice(&[0x88, 0xB5]);
        f.extend_from_slice(&i.to_be_bytes());
        f.resize(60, 0);
        f
    }

    /// "or for a wait past 10 s": the bound is ten seconds, whatever the constant says.
    #[test]
    fn the_wait_s_bound_is_ten_seconds() {
        assert_eq!(read::MAX_WAIT, Duration::from_secs(10));
        let hub = Hub::new();
        let l = Listener::start(vec![Box::new(hub.port("hub0", HERE))], Options::default()).unwrap();
        let on = device_with("hub0", DEV, a(10), MASK24);
        let past = read::Options { wait: Duration::from_secs(10) + Duration::from_millis(1) };
        assert!(matches!(read::read(&l, "hub0", &on, IM0, &past), Err(ReadError::Wait(w)) if w == past.wait));
        let at = read::Options { wait: Duration::from_secs(10) };
        assert!(matches!(read::read(&l, "hub0", &on, IM0, &at), Err(ReadError::NoAddress(_))));
    }

    /// "until the device answers that a subslot is not there (IODReadRes, PNIORW, 0xB2)": its error
    /// code 2 is no part of it.
    #[test]
    fn the_walk_ends_at_0xb2_whatever_its_error_code_2() {
        for code2 in [0u8, 1, 0x05, 0xFF] {
            assert!(read::is_no_subslot(PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2 }), "{code2:#x}");
        }
    }

    /// "not a fragment": each bit of the offset alone, and more fragments alone, is a fragment;
    /// don't fragment alone is none.
    #[test]
    fn every_bit_of_the_offset_is_a_fragment() {
        let good = ip::udp_frame(Endpoint { mac: HERE, ip: a(1), port: 49152 }, Endpoint { mac: DEV, ip: a(10), port: 34964 }, 7, b"hello, device").unwrap();
        let with = |flags_offset: u16| {
            let mut f = good.clone();
            f[20..22].copy_from_slice(&flags_offset.to_be_bytes());
            resum(&mut f[14..34]);
            f
        };
        for bit in 0..14 {
            assert_eq!(ip::datagram(&with(1 << bit)), None, "flags and offset {:#06x}", 1u16 << bit);
        }
        assert!(ip::datagram(&with(0x4000)).is_some());
    }

    /// A header that says 16 bytes, its checksum right over those 16: none (a header is at least
    /// 20), though what follows reads as UDP.
    #[test]
    fn a_16_byte_header_with_its_own_checksum_is_none() {
        let payload = b"0123456789abcdef";
        let total = 16 + 8 + payload.len();
        let mut h = vec![0x44, 0, 0, 0, 0, 7, 0x40, 0, 64, 17, 0, 0, 192, 168, 0, 1];
        h[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        resum(&mut h);
        let mut f = DEV.0.to_vec();
        f.extend_from_slice(&HERE.0);
        f.extend_from_slice(&[0x08, 0x00]);
        f.extend_from_slice(&h);
        // "UDP": the ports a 20-byte header's destination would hold, its length, no checksum.
        f.extend_from_slice(&[0xC0, 0xA8, 0x00, 0x0A]);
        f.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(payload);
        f.resize(f.len().max(60), 0);
        assert_eq!(ip::datagram(&f), None);
    }

    /// A device on `link` answering every read with `record`.
    #[cfg(target_os = "linux")]
    fn answering(link: &str, me: Endpoint, record: &'static [u8]) -> super::round_2::Scripted {
        let handler: Handler = Box::new(move |r: &Req| vec![back(r, &answer(r, record, PnioStatus::OK))]);
        scripted(link, vec![me], Arc::new(Mutex::new(handler)))
    }

    /// Frames let go before the request (a listener long past its bound) are no cause: the read
    /// takes its answer.
    #[cfg(target_os = "linux")]
    #[test]
    fn frames_let_go_before_the_request_do_not_refuse_it() {
        netns::isolated("round_3::frames_let_go_before_the_request_do_not_refuse_it", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let _peer = answering("b0", Endpoint { mac: DEV, ip: a(10), port: 34964 }, b"mine");
            let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 8, ..Options::default() }).unwrap();
            let mut b = bench::link("b0");
            for i in 0..40 {
                b.send(&junk(i)).unwrap();
            }
            assert!(small.until(Duration::from_secs(3), |s| s.stats.let_go >= 30), "{:?}", small.snapshot().stats);
            let r = read::read(&small, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(400) });
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// One frame let go after the request before it was read: refused, one lost. (The read may
    /// read the first of the device's two frames in the microseconds before the second comes: then
    /// nothing it had not read was let go, and it takes the answer.)
    #[cfg(target_os = "linux")]
    #[test]
    fn one_frame_let_go_after_the_request_refuses_it() {
        netns::isolated("round_3::one_frame_let_go_after_the_request_refuses_it", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let handler: Handler = Box::new(move |r: &Req| {
                std::thread::sleep(Duration::from_millis(100));
                let j = |i: u16| (me, Endpoint { port: 9, ..r.from }, format!("junk{i}").into_bytes());
                vec![j(0), j(1), back(r, &answer(r, b"late", PnioStatus::OK))]
            });
            let _peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
            let mut got = Vec::new();
            for _ in 0..5 {
                let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 2, ..Options::default() }).unwrap();
                got.push(read::read(&small, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(400) }).map(|r| r.data));
                small.stop();
            }
            assert!(got.iter().all(|r| matches!(r, Err(ReadError::Lost(1))) || r.as_ref().is_ok_and(|d| d == b"late")), "{got:#?}");
            assert!(got.iter().any(|r| matches!(r, Err(ReadError::Lost(1)))), "{got:#?}");
        });
    }

    /// The request itself let go (the bound one frame) is no frame after it: the answer is read.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_request_let_go_is_no_frame_after_it() {
        netns::isolated("round_3::the_request_let_go_is_no_frame_after_it", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let _peer = answering("b0", Endpoint { mac: DEV, ip: a(10), port: 34964 }, b"mine");
            let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 1, ..Options::default() }).unwrap();
            let r = read::read(&small, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(400) });
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// "no answer within the wait": an answer a few milliseconds after its request went, behind a
    /// queue that takes longer than the wait, is within it.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_wait_counts_from_the_request_s_going() {
        netns::isolated("round_3::the_wait_counts_from_the_request_s_going", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let _peer = answering("b0", Endpoint { mac: DEV, ip: a(10), port: 34964 }, b"mine");
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            // Twenty frames ahead at the default twenty a second: the request goes about a second
            // after the call.
            for i in 0..20 {
                let mut f = junk(i);
                f[..6].copy_from_slice(&DEV.0);
                f[6..12].copy_from_slice(&HERE.0);
                l.send("a0", f).unwrap();
            }
            let start = std::time::Instant::now();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(400) });
            let took = start.elapsed();
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()), "{took:?}");
            assert!(took >= Duration::from_millis(900), "{took:?}");
        });
    }

    /// The read goes from the interface's address on the device's network by the device's own mask
    /// (a device answers without a router only a source there; the first such listed), whatever
    /// the system lists first: an office network's address before two of the device's, a
    /// link-local one beside it, or the device's wider mask taking in an address the interface's
    /// own mask leaves out. The device here answers only such a source.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_read_goes_from_the_interface_s_address_on_the_device_s_network() {
        netns::isolated("round_3::the_read_goes_from_the_interface_s_address_on_the_device_s_network", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let wide = Ipv4Addr::new(255, 255, 0, 0);
            let arrangements: [(&[&[&str]], Ipv4Addr, Ipv4Addr); 3] = [
                (&[&["10.9.9.1/24"], &["192.168.0.1/24"], &["192.168.0.2/24"]], MASK24, a(1)),
                (&[&["192.168.0.1/24"], &["169.254.7.7/16", "scope", "link"]], MASK24, a(1)),
                (&[&["10.9.9.1/24"], &["192.168.77.1/24"]], wide, Ipv4Addr::new(192, 168, 77, 1)),
            ];
            for (addresses, mask, from) in arrangements {
                netns::ip(&["addr", "flush", "dev", "a0"]);
                for address in addresses {
                    let mut args = vec!["addr", "add"];
                    args.extend_from_slice(address);
                    args.extend_from_slice(&["dev", "a0"]);
                    netns::ip(&args);
                }
                assert_ne!(read::address_of("a0"), Some(from), "the source is not the first listed: {addresses:?}");
                assert_eq!(read::address_for("a0", me.ip, mask), Some(from));
                let on_network = move |ip: Ipv4Addr| u32::from(ip) & u32::from(mask) == u32::from(me.ip) & u32::from(mask);
                let handler: Handler = Box::new(move |r: &Req| if on_network(r.from.ip) { vec![back(r, &answer(r, b"mine", PnioStatus::OK))] } else { vec![] });
                let peer = scripted("b0", vec![me], Arc::new(Mutex::new(handler)));
                let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
                let r = read::read(&l, "a0", &device_with("a0", DEV, me.ip, mask), IM0, &read::Options { wait: Duration::from_millis(400) });
                let sources: Vec<Ipv4Addr> = peer.seen.lock().unwrap().iter().map(|q| q.from.ip).collect();
                assert_eq!(sources, [from], "{addresses:?}");
                assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()), "{addresses:?}");
                l.stop();
            }
        });
    }

    /// The request asks for its answer: a request (not maybe, not broadcast), idempotent, as a read
    /// is.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_request_asks_for_its_answer() {
        netns::isolated("round_3::the_request_asks_for_its_answer", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            netns::ip(&["addr", "add", "192.168.0.1/24", "dev", "a0"]);
            let peer = answering("b0", Endpoint { mac: DEV, ip: a(10), port: 34964 }, b"mine");
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(400) });
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            let seen = peer.seen.lock().unwrap();
            let flags: Vec<(PacketType, u8)> = seen.iter().map(|q| (q.pdu.packet_type, q.pdu.flags1)).collect();
            assert_eq!(flags, [(PacketType::Request, flags1::IDEMPOTENT)]);
        });
    }

    /// An interface holding addresses, none on the device's network by the device's own mask: the
    /// read refused, nothing sent, even where the interface's mask would take the device in.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_interface_with_no_address_on_the_device_s_network_is_refused() {
        netns::isolated("round_3::an_interface_with_no_address_on_the_device_s_network_is_refused", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            let device = Ipv4Addr::new(192, 168, 5, 10);
            for address in ["10.9.9.1/24", "192.168.0.1/16"] {
                netns::ip(&["addr", "flush", "dev", "a0"]);
                netns::ip(&["addr", "add", address, "dev", "a0"]);
                let peer = answering("b0", Endpoint { mac: DEV, ip: device, port: 34964 }, b"mine");
                let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
                let r = read::read(&l, "a0", &device_with("a0", DEV, device, MASK24), IM0, &read::Options { wait: Duration::from_millis(400) });
                assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Err("\"a0\" holds no IPv4 address on the network of 192.168.5.10 (mask 255.255.255.0): the device could not answer it".to_string()), "{address}");
                assert_eq!(l.interface("a0").unwrap().sent, 0, "{address}");
                assert!(peer.seen.lock().unwrap().is_empty(), "{address}");
                l.stop();
            }
        });
    }
}

/// The fourth refute round's cases: an address under a label or an interface's altname the
/// interface's own, the device's own address never the source, a device's mask of nothing or
/// everything, the port held open on the request's address, a fault or reject by its activity, frames
/// read then let go, a cut walk's neighbours, a port with no peer, the walk ended at its first
/// subslot, and the datagram's checksums, lengths and tags.
mod round_4 {
    use std::net::Ipv4Addr;
    #[cfg(target_os = "linux")]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[cfg(target_os = "linux")]
    use std::sync::{Arc, Mutex};
    #[cfg(target_os = "linux")]
    use std::time::Duration;

    #[cfg(target_os = "linux")]
    use super::bench;
    #[cfg(target_os = "linux")]
    use veredas_aranha::fusion::{Device, Fusion};
    use veredas_aranha::ip::{self, Endpoint};
    #[cfg(target_os = "linux")]
    use veredas_aranha::listen::{Listener, Options};
    #[cfg(target_os = "linux")]
    use veredas_aranha::netns;
    #[cfg(target_os = "linux")]
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    #[cfg(target_os = "linux")]
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::mac::Mac;
    #[cfg(target_os = "linux")]
    use veredas_aranha::profinet::record::{LinkState, Peer, PortDataReal};
    #[cfg(target_os = "linux")]
    use veredas_aranha::profinet::rpc::{self, PacketType, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcPdu, Uuid};
    #[cfg(target_os = "linux")]
    use veredas_aranha::read::{self, ReadError, Target};

    const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xE4, 0x00, 0xA0]);
    const DEV: Mac = Mac([0x02, 0x00, 0x5E, 0xE4, 0x00, 0x01]);
    #[cfg(target_os = "linux")]
    const PEER1: Mac = Mac([0x02, 0x00, 0x5E, 0xE4, 0x00, 0x09]);
    #[cfg(target_os = "linux")]
    const MASK24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
    #[cfg(target_os = "linux")]
    const IM0: Target = Target { slot: 0, subslot: 1, index: 0xAFF0 };
    #[cfg(target_os = "linux")]
    const NO_SUBSLOT: PnioStatus = PnioStatus { code: 0xDE, decode: 0x80, code1: 0xB2, code2: 0 };

    fn a(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 0, last)
    }

    #[cfg(target_os = "linux")]
    fn device_with(iface: &str, mac: Mac, address: Ipv4Addr, mask: Ipv4Addr) -> Device {
        let identity = DeviceIdentity { name: Some("bomba-1".into()), ip: Some(IpSuite { ip: address, mask, gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
        let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
        let mut f = Fusion::new();
        f.apply(&o);
        f.devices.remove(&mac).unwrap()
    }

    #[cfg(target_os = "linux")]
    #[derive(Clone)]
    struct Req {
        from: Endpoint,
        to: Endpoint,
        pdu: RpcPdu,
        args: RequestArgs,
        read: ReadRequest,
    }

    #[cfg(target_os = "linux")]
    /// What the scripted device sends: whole frames, with a pause (ms) before each.
    type Handler = Box<dyn FnMut(&Req) -> Vec<(u64, Vec<u8>)> + Send>;

    #[cfg(target_os = "linux")]
    struct Scripted {
        stop: Arc<AtomicBool>,
        seen: Arc<Mutex<Vec<Req>>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    #[cfg(target_os = "linux")]
    impl Drop for Scripted {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }

    #[cfg(target_os = "linux")]
    /// A device on `link` answering PNIO requests to any of `addresses` (any address when empty).
    fn scripted(link: &str, addresses: Vec<Endpoint>, mut handler: Handler) -> Scripted {
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
        let mut b = bench::link(link);
        let join = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok(Some(f)) = b.recv(Duration::from_millis(5)) else { continue };
                    let Some(d) = ip::datagram(&f.bytes) else { continue };
                    if d.dst.port != 34964 || (!addresses.is_empty() && !addresses.contains(&d.dst)) {
                        continue;
                    }
                    let Ok(pdu) = RpcPdu::decode(&d.payload) else { continue };
                    let Ok(args) = RequestArgs::decode(&pdu.body, pdu.byte_order()) else { continue };
                    let Ok(read) = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]) else { continue };
                    let req = Req { from: d.src, to: d.dst, pdu, args, read };
                    seen.lock().unwrap().push(req.clone());
                    for (pause, frame) in handler(&req) {
                        if pause > 0 {
                            std::thread::sleep(Duration::from_millis(pause));
                        }
                        b.send(&frame).unwrap();
                    }
                }
            })
        };
        Scripted { stop, seen, join: Some(join) }
    }

    #[cfg(target_os = "linux")]
    fn answer(r: &Req, record: &[u8], status: PnioStatus) -> RpcPdu {
        let mut data = ReadResponse::answering(&r.read, record.len() as u32).block().encode().unwrap();
        data.extend_from_slice(record);
        RpcPdu::pnio_response(&r.pdu, 7, &ResponseArgs::new(status, r.args.args_maximum, data))
    }

    #[cfg(target_os = "linux")]
    fn frame(from: Endpoint, to: Endpoint, pdu: &RpcPdu) -> Vec<u8> {
        ip::udp_frame(from, to, 1, &pdu.encode().unwrap()).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn back(r: &Req, pdu: &RpcPdu) -> (u64, Vec<u8>) {
        (0, frame(r.to, r.from, pdu))
    }

    #[cfg(target_os = "linux")]
    fn port_data(subslot: u16, peers: Vec<Peer>) -> Vec<u8> {
        PortDataReal {
            slot: 0,
            subslot,
            own_port_id: format!("port-{:03}", subslot.wrapping_sub(0x8000)),
            peers,
            mau_type: 0x10,
            domain_boundary: 0,
            multicast_boundary: 0,
            link_state: LinkState { port: 0, link: 1 },
            media_type: 1,
            extra: Vec::new(),
        }
        .block()
        .unwrap()
        .encode()
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn peer(mac: Mac) -> Vec<Peer> {
        vec![Peer { port_id: "port-009.sw".into(), chassis_id: "sw".into(), line_delay: 0, mac }]
    }

    #[cfg(target_os = "linux")]
    fn quick() -> read::Options {
        read::Options { wait: Duration::from_millis(400) }
    }

    #[cfg(target_os = "linux")]
    fn setup(addrs: &[&[&str]]) {
        bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
        for address in addrs {
            let mut args = vec!["addr", "add"];
            args.extend_from_slice(address);
            args.extend_from_slice(&["dev", "a0"]);
            netns::ip(&args);
        }
    }

    #[cfg(target_os = "linux")]
    fn ip_out(args: &[&str]) -> String {
        let out = std::process::Command::new("ip").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    // ---------------------------------------------------------------------------------------------
    // The address choice.

    /// An interface holding the device's network's address under a label (`ip addr add ... label
    /// a0:plant`, as an `eth0:1` alias is): the interface holds it, the system lists it (as
    /// `a0:plant`), and the clause's read goes from it; the code does not see it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_labelled_address_is_the_interface_s() {
        netns::isolated("round_4::a_labelled_address_is_the_interface_s", &[("a0", "b0")], || {
            setup(&[&["10.9.9.1/24"], &["192.168.0.1/24", "label", "a0:plant"]]);
            let shown = ip_out(&["-4", "addr", "show", "dev", "a0"]);
            assert!(shown.contains("192.168.0.1/24") && shown.contains("a0:plant"), "{shown}");
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            assert_eq!(read::addresses_of("a0"), [Ipv4Addr::new(10, 9, 9, 1), a(1)]);
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            let sources: Vec<Ipv4Addr> = peer.seen.lock().unwrap().iter().map(|q| q.from.ip).collect();
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources, [a(1)]);
        });
    }

    /// The same with the labelled address the interface's only one: the code says it holds none.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_labelled_address_alone_is_the_interface_s() {
        netns::isolated("round_4::a_labelled_address_alone_is_the_interface_s", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24", "label", "a0:plant"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// A listener opened on the interface's alternative name (`ip link property add dev a0 altname
    /// plant0`): the wire opens by it, the interface holds the device's network's address, and the
    /// read is refused as holding none.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_altname_s_address_is_the_interface_s() {
        netns::isolated("round_4::an_altname_s_address_is_the_interface_s", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            netns::ip(&["link", "property", "add", "dev", "a0", "altname", "plant0"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("plant0")], Options::default()).unwrap();
            let r = read::read(&l, "plant0", &device_with("plant0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(read::addresses_of("plant0"), [a(1)]);
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// A device stating mask 0.0.0.0: by its own mask every source is on its network, so the read goes
    /// from the interface's first address (here an office one), and the device answers it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_device_mask_of_zero_takes_the_first_address() {
        netns::isolated("round_4::a_device_mask_of_zero_takes_the_first_address", &[("a0", "b0")], || {
            setup(&[&["10.9.9.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), Ipv4Addr::UNSPECIFIED), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            let sources: Vec<Ipv4Addr> = peer.seen.lock().unwrap().iter().map(|q| q.from.ip).collect();
            assert_eq!(sources, [Ipv4Addr::new(10, 9, 9, 1)]);
        });
    }

    /// A device stating mask 255.255.255.255 beside an interface on its /24: no address of the
    /// interface is on its network (its own alone), refused, nothing sent.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_device_host_mask_is_off_network() {
        netns::isolated("round_4::a_device_host_mask_is_off_network", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), Ipv4Addr::BROADCAST), IM0, &quick());
            assert!(matches!(r, Err(ReadError::OffNetwork { .. })), "{r:?}");
            assert_eq!(l.interface("a0").unwrap().sent, 0);
        });
    }

    /// The device's own address is never the request's source (the device could not answer
    /// itself): an interface holding it beside another on its network reads from the other; holding
    /// it alone, refused by name, nothing sent.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_device_s_own_address_is_never_the_source() {
        netns::isolated("round_4::the_device_s_own_address_is_never_the_source", &[("a0", "b0")], || {
            setup(&[&["192.168.0.10/24"], &["192.168.0.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(peer.seen.lock().unwrap().iter().map(|q| q.from.ip).collect::<Vec<_>>(), [a(1)]);
            l.stop();
            netns::ip(&["addr", "del", "192.168.0.1/24", "dev", "a0"]);
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Err("\"a0\" holds 192.168.0.10, the device's own address, and no other on its network: the device could not answer it".to_string()));
            assert_eq!(l.interface("a0").unwrap().sent, 0);
        });
    }

    /// The port held open is held on the request's own address: where the interface lists an office
    /// address first, the answer to the device-network address and port reaches an open port, and this
    /// machine sends the device no ICMP port unreachable.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_port_is_open_on_the_request_s_address() {
        netns::isolated("round_4::the_port_is_open_on_the_request_s_address", &[("a0", "b0")], || {
            setup(&[&["10.9.9.1/24"], &["192.168.0.1/24"]]);
            // The kernel may answer the device (an ICMP error) without asking for its MAC first.
            netns::ip(&["neigh", "add", "192.168.0.10", "lladdr", &DEV.to_string(), "dev", "a0", "nud", "permanent"]);
            let icmp = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let mut b = bench::link("b0");
            let t = {
                let (icmp, stop) = (icmp.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        let Ok(Some(f)) = b.recv(Duration::from_millis(5)) else { continue };
                        let x = &f.bytes;
                        if x.len() > 35 && x[12..14] == [0x08, 0x00] && x[23] == 1 && x[14 + usize::from(x[14] & 0x0F) * 4] == 3 {
                            icmp.fetch_add(1, Ordering::SeqCst);
                            continue;
                        }
                        let Some(d) = ip::datagram(x) else { continue };
                        if d.dst != me {
                            continue;
                        }
                        let pdu = RpcPdu::decode(&d.payload).unwrap();
                        let args = RequestArgs::decode(&pdu.body, pdu.byte_order()).unwrap();
                        let read = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]).unwrap();
                        let req = Req { from: d.src, to: d.dst, pdu, args, read };
                        b.send(&back(&req, &answer(&req, b"mine", PnioStatus::OK)).1).unwrap();
                    }
                })
            };
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            for _ in 0..3 {
                let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
                assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            }
            std::thread::sleep(Duration::from_millis(300));
            stop.store(true, Ordering::SeqCst);
            t.join().unwrap();
            assert_eq!(icmp.load(Ordering::SeqCst), 0, "the answer reached an open port");
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The answer's match.

    /// A fault or a reject from the device to the request's port under another activity is no answer
    /// of this read: passed over, its own answer taken.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_fault_or_reject_of_another_activity_is_passed_over() {
        netns::isolated("round_4::a_fault_or_reject_of_another_activity_is_passed_over", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted(
                "b0",
                vec![me],
                Box::new(|r: &Req| {
                    let good = answer(r, b"mine", PnioStatus::OK);
                    vec![
                        back(r, &RpcPdu { packet_type: PacketType::Fault, activity: Uuid([9; 16]), ..good.clone() }),
                        back(r, &RpcPdu { packet_type: PacketType::Reject, activity: Uuid([8; 16]), ..good.clone() }),
                        back(r, &good),
                    ]
                }),
            );
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// A reject (or a response) from the device under the request's activity whose RPC header names
    /// another interface than PNIO's device one: the clause's match names no interface UUID.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_reject_under_the_activity_is_its_answer_whatever_interface_it_names() {
        netns::isolated("round_4::a_reject_under_the_activity_is_its_answer_whatever_interface_it_names", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &RpcPdu { packet_type: PacketType::Reject, interface: Uuid::NIL, body: vec![0x01, 0, 0, 0x1c], ..answer(r, b"", PnioStatus::OK) })]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert!(matches!(r, Err(ReadError::Rejected { kind: PacketType::Reject, .. })), "{r:?}");
        });
    }

    /// A response under the request's activity naming another interface than PNIO's device one is
    /// passed over; the answer naming it is taken.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_response_naming_another_interface_is_passed_over() {
        netns::isolated("round_4::a_response_naming_another_interface_is_passed_over", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted("b0", vec![me], Box::new(|r: &Req| vec![back(r, &RpcPdu { interface: Uuid::NIL, ..answer(r, b"not this", PnioStatus::OK) }), back(r, &answer(r, b"mine", PnioStatus::OK))]));
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    /// Datagrams the read has read (passed over) and then let go past a small bound are no loss: the
    /// answer after them is read.
    #[cfg(target_os = "linux")]
    #[test]
    fn frames_read_then_let_go_are_no_loss() {
        netns::isolated("round_4::frames_read_then_let_go_are_no_loss", &[("a0", "b0")], || {
            setup(&[&["192.168.0.1/24"]]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let _peer = scripted(
                "b0",
                vec![me],
                Box::new(move |r: &Req| {
                    let mut out: Vec<(u64, Vec<u8>)> = (0..6u16).map(|i| (80, ip::udp_frame(me, Endpoint { port: 9, ..r.from }, i, format!("junk{i}").as_bytes()).unwrap())).collect();
                    out.push((80, frame(r.to, r.from, &answer(r, b"mine", PnioStatus::OK))));
                    out
                }),
            );
            let small = Listener::start(vec![bench::link("a0")], Options { max_frames: 3, ..Options::default() }).unwrap();
            let r = read::read(&small, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &read::Options { wait: Duration::from_millis(1500) });
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The walk.

    #[cfg(target_os = "linux")]
    fn walk_device(handler: Handler) -> (Scripted, Listener, Device) {
        setup(&[&["192.168.0.1/24"]]);
        let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
        let peer = scripted("b0", vec![me], handler);
        let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
        (peer, l, device_with("a0", DEV, a(10), MASK24))
    }

    /// A walk cut at its second subslot by port data of another subslot (holding a peer): the cut
    /// keeps the first port and its neighbour alone, never the refused record's.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_cut_port_s_neighbours_are_not_kept() {
        netns::isolated("round_4::a_cut_port_s_neighbours_are_not_kept", &[("a0", "b0")], || {
            let other = Mac([0x02, 0x00, 0x5E, 0xE4, 0x00, 0x77]);
            let (_p, l, device) = walk_device(Box::new(move |r: &Req| match r.read.subslot {
                0x8001 => vec![back(r, &answer(r, &port_data(0x8001, peer(PEER1)), PnioStatus::OK))],
                0x8002 => vec![back(r, &answer(r, &port_data(0x8005, peer(other)), PnioStatus::OK))],
                _ => vec![back(r, &answer(r, &[], NO_SUBSLOT))],
            }));
            let cut = *read::ports(&l, "a0", &device, &quick()).unwrap_err();
            assert_eq!(cut.subslot, 0x8002);
            assert_eq!(cut.read.read.len(), 1);
            assert_eq!(cut.read.neighbours.iter().map(|n| n.peer_mac).collect::<Vec<_>>(), [Some(PEER1)]);
        });
    }

    /// The same at the first subslot (port data of slot 1): no port and no neighbour kept.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_cut_first_port_keeps_no_neighbour() {
        netns::isolated("round_4::a_cut_first_port_keeps_no_neighbour", &[("a0", "b0")], || {
            let (_p, l, device) = walk_device(Box::new(move |r: &Req| {
                let mut pd = PortDataReal::from_block(&rpc::read_blocks(&port_data(r.read.subslot, peer(PEER1))).0[0]).unwrap();
                pd.slot = 1;
                vec![back(r, &answer(r, &pd.block().unwrap().encode().unwrap(), PnioStatus::OK))]
            }));
            let cut = *read::ports(&l, "a0", &device, &quick()).unwrap_err();
            assert_eq!((cut.subslot, cut.read.read.len(), cut.read.neighbours.len(), cut.read.end), (0x8001, 0, 0, None));
        });
    }

    /// A device that answers its first port subslot is not there: the walk ends there, no port, no
    /// neighbour, the end that answer.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_walk_ends_at_its_first_subslot() {
        netns::isolated("round_4::the_walk_ends_at_its_first_subslot", &[("a0", "b0")], || {
            let (p, l, device) = walk_device(Box::new(|r: &Req| vec![back(r, &answer(r, &[], NO_SUBSLOT))]));
            let ports = read::ports(&l, "a0", &device, &quick()).map_err(|c| format!("{:?}", c.error)).unwrap();
            assert_eq!((ports.read.len(), ports.neighbours.len()), (0, 0));
            let end = ports.end.unwrap();
            let k = l.frame(end).unwrap();
            assert!(!k.sent && ip::datagram(&k.frame.bytes).unwrap().src.mac == DEV);
            assert_eq!(p.seen.lock().unwrap().len(), 1);
        });
    }

    /// A port with no peer (nothing cabled, or LLDP off) is a port read like any: the walk goes on,
    /// the next port's neighbour given, the port without one giving none.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_port_with_no_peer_is_read_and_the_walk_goes_on() {
        netns::isolated("round_4::a_port_with_no_peer_is_read_and_the_walk_goes_on", &[("a0", "b0")], || {
            let (_p, l, device) = walk_device(Box::new(|r: &Req| match r.read.subslot {
                0x8001 => vec![back(r, &answer(r, &port_data(0x8001, vec![]), PnioStatus::OK))],
                0x8002 => vec![back(r, &answer(r, &port_data(0x8002, peer(PEER1)), PnioStatus::OK))],
                _ => vec![back(r, &answer(r, &[], NO_SUBSLOT))],
            }));
            let ports = read::ports(&l, "a0", &device, &quick()).map_err(|c| format!("{:?}", c.error)).unwrap();
            assert_eq!(ports.read.iter().map(|p| (p.data.subslot, p.data.peers.len())).collect::<Vec<_>>(), [(0x8001, 0), (0x8002, 1)]);
            assert_eq!(ports.neighbours.iter().map(|n| (n.port.as_str(), n.peer_mac)).collect::<Vec<_>>(), [("port-002", Some(PEER1))]);
            assert!(ports.end.is_some());
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The datagram.

    fn good_frame() -> Vec<u8> {
        ip::udp_frame(Endpoint { mac: HERE, ip: a(1), port: 49152 }, Endpoint { mac: DEV, ip: a(10), port: 34964 }, 7, b"hello, device").unwrap()
    }

    fn resum(header: &mut [u8]) {
        header[10..12].copy_from_slice(&[0, 0]);
        let mut acc: u32 = header.chunks(2).map(|p| u32::from(u16::from_be_bytes([p[0], p[1]]))).sum();
        while acc > 0xFFFF {
            acc = (acc & 0xFFFF) + (acc >> 16);
        }
        header[10..12].copy_from_slice(&(!(acc as u16)).to_be_bytes());
    }

    /// A UDP checksum field of 0xFFFF that is wrong (0xFFFF is a computed zero, never "none"): none.
    #[test]
    fn a_wrong_checksum_of_ffff_is_none() {
        let mut f = good_frame();
        assert_ne!(&f[40..42], &[0xFF, 0xFF]);
        f[40] = 0xFF;
        f[41] = 0xFF;
        assert_eq!(ip::datagram(&f), None);
    }

    /// An IPv4 packet longer than its UDP datagram, the datagram's checksum right (over its own length
    /// and pseudo header): read; that checksum wrong: none.
    #[test]
    fn a_datagram_shorter_than_its_packet_is_checked_over_its_own_length() {
        let payload = b"hello, device";
        let g = good_frame();
        let mut f = g[..14 + 20 + 8 + payload.len()].to_vec();
        f.extend_from_slice(b"tail");
        let total = (20 + 8 + payload.len() + 4) as u16;
        f[16..18].copy_from_slice(&total.to_be_bytes());
        resum(&mut f[14..34]);
        f.resize(f.len().max(60), 0);
        // The checksum as written for the datagram alone is kept.
        let d = ip::datagram(&f).unwrap();
        assert_eq!(d.payload, payload);
        let mut wrong = f.clone();
        wrong[41] ^= 0x01;
        assert_eq!(ip::datagram(&wrong), None);
    }

    /// A total length past the frame by exactly one, its header's checksum right: none, no panic.
    #[test]
    fn a_total_one_past_the_frame_is_none() {
        let payload = [0x11u8; 40];
        let g = ip::udp_frame(Endpoint { mac: HERE, ip: a(1), port: 49152 }, Endpoint { mac: DEV, ip: a(10), port: 34964 }, 7, &payload).unwrap();
        assert_eq!(g.len(), 14 + 20 + 8 + 40);
        let mut f = g.clone();
        let total = (20 + 8 + 40 + 1) as u16;
        f[16..18].copy_from_slice(&total.to_be_bytes());
        resum(&mut f[14..34]);
        assert_eq!(ip::datagram(&f), None);
        // A UDP length of exactly its header: an empty datagram.
        let e = ip::udp_frame(Endpoint { mac: HERE, ip: a(1), port: 49152 }, Endpoint { mac: DEV, ip: a(10), port: 34964 }, 7, &[]).unwrap();
        assert_eq!(u16::from_be_bytes([e[38], e[39]]), 8);
        assert_eq!(ip::datagram(&e).unwrap().payload, Vec::<u8>::new());
        // A tag with its priority bits set (and the CFI), read through; two tags, none.
        let mut tagged = g[..12].to_vec();
        tagged.extend_from_slice(&[0x81, 0x00, 0xF0, 0x05]);
        tagged.extend_from_slice(&g[12..]);
        assert_eq!(ip::datagram(&tagged).unwrap().payload, payload);
        let mut two = g[..12].to_vec();
        two.extend_from_slice(&[0x81, 0x00, 0x00, 0x05, 0x81, 0x00, 0x00, 0x06]);
        two.extend_from_slice(&g[12..]);
        assert_eq!(ip::datagram(&two), None);
    }
}

/// The fifth refute round's cases: a fault or reject from anywhere but the device's MAC, address
/// and port to the request's, or heard on another interface, passed over; the kernel's list of
/// addresses whole and in its order past many reads, a point-to-point address its own end, the
/// device's address held twice, a /31; and the addresses by the index the listener's link was
/// opened on: a rename, an alternative name moved or dropped since, an interface removed, and a
/// list the system cannot give, refused by its own reason with nothing sent.
#[cfg(target_os = "linux")]
mod round_5 {
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::bench;
    use veredas_aranha::fusion::{Device, Fusion};
    use veredas_aranha::ip::{self, Endpoint};
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::netns;
    use veredas_aranha::observe::{Observation, Observed, Probe, Witness};
    use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::profinet::rpc::{self, PacketType, PnioStatus, ReadRequest, ReadResponse, RequestArgs, ResponseArgs, RpcPdu};
    use veredas_aranha::read::{self, ReadError, Target};
    use veredas_aranha::wire::Wire;
    use veredas_aranha::wire::raw::{Options as RawOptions, RawLink};

    const HERE: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0xA0]);
    const HERE1: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0xA1]);
    const HERE2: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0xA2]);
    const DEV: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0x01]);
    const DEV1: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0x02]);
    const DEV2: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0x03]);
    const OTHER: Mac = Mac([0x02, 0x00, 0x5E, 0xE5, 0x00, 0x77]);
    const MASK24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
    const IM0: Target = Target { slot: 0, subslot: 1, index: 0xAFF0 };

    fn a(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 0, last)
    }

    fn device_with(iface: &str, mac: Mac, address: Ipv4Addr, mask: Ipv4Addr) -> Device {
        let identity = DeviceIdentity { name: Some("bomba-1".into()), ip: Some(IpSuite { ip: address, mask, gateway: Ipv4Addr::UNSPECIFIED }), ..Default::default() };
        let o = Observation { witness: Witness { probe: Probe::DcpHeard, iface: iface.into(), at_ns: 1, frame: 0 }, observed: Observed::Dcp { mac, service: dcp::Service::Hello, service_type: dcp::ServiceType::Request, identity } };
        let mut f = Fusion::new();
        f.apply(&o);
        f.devices.remove(&mac).unwrap()
    }

    #[derive(Clone)]
    struct Req {
        from: Endpoint,
        to: Endpoint,
        pdu: RpcPdu,
        args: RequestArgs,
        read: ReadRequest,
    }

    /// What the scripted device sends for a request: (the link it goes out on, the frame); a link
    /// of `PAUSE` is a pause of 150 ms instead.
    const PAUSE: usize = usize::MAX;
    type Handler = Box<dyn FnMut(&Req) -> Vec<(usize, Vec<u8>)> + Send>;

    struct Scripted {
        stop: Arc<AtomicBool>,
        seen: Arc<Mutex<Vec<Req>>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Scripted {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }

    /// A device hearing requests on `links[0]` to `me`, answering on any of `links`; the links
    /// open before its thread starts.
    fn scripted(links: &[&str], me: Endpoint, mut handler: Handler) -> Scripted {
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
        let mut ls: Vec<Box<dyn Wire>> = links.iter().map(|l| bench::link(l)).collect();
        let join = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok(Some(f)) = ls[0].recv(Duration::from_millis(5)) else { continue };
                    let Some(d) = ip::datagram(&f.bytes) else { continue };
                    if d.dst != me {
                        continue;
                    }
                    let Ok(pdu) = RpcPdu::decode(&d.payload) else { continue };
                    let Ok(args) = RequestArgs::decode(&pdu.body, pdu.byte_order()) else { continue };
                    let Ok(read) = ReadRequest::from_block(&rpc::read_blocks(&args.data).0[0]) else { continue };
                    let req = Req { from: d.src, to: d.dst, pdu, args, read };
                    seen.lock().unwrap().push(req.clone());
                    for (link, frame) in handler(&req) {
                        if link == PAUSE {
                            std::thread::sleep(Duration::from_millis(150));
                            continue;
                        }
                        ls[link].send(&frame).unwrap();
                    }
                }
            })
        };
        Scripted { stop, seen, join: Some(join) }
    }

    fn answer(r: &Req, record: &[u8], status: PnioStatus) -> RpcPdu {
        let mut data = ReadResponse::answering(&r.read, record.len() as u32).block().encode().unwrap();
        data.extend_from_slice(record);
        RpcPdu::pnio_response(&r.pdu, 7, &ResponseArgs::new(status, r.args.args_maximum, data))
    }

    fn frame(from: Endpoint, to: Endpoint, pdu: &RpcPdu) -> Vec<u8> {
        ip::udp_frame(from, to, 1, &pdu.encode().unwrap()).unwrap()
    }

    /// A device answering every request with its own record, from where it was asked.
    fn mine() -> Handler {
        Box::new(move |r: &Req| vec![(0, frame(r.to, r.from, &answer(r, b"mine", PnioStatus::OK)))])
    }

    fn quick() -> read::Options {
        read::Options { wait: Duration::from_millis(600) }
    }

    fn sources(p: &Scripted) -> Vec<Ipv4Addr> {
        p.seen.lock().unwrap().iter().map(|q| q.from.ip).collect()
    }

    fn add(dev: &str, spec: &[&str]) {
        let mut args = vec!["addr", "add"];
        args.extend_from_slice(spec);
        args.extend_from_slice(&["dev", dev]);
        netns::ip(&args);
    }

    fn ip_out(args: &[&str]) -> String {
        let out = std::process::Command::new("ip").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// The local IPv4 addresses `ip -4 -o addr show dev <dev>` lists, in its order.
    fn ip_show(dev: &str) -> Vec<Ipv4Addr> {
        ip_out(&["-4", "-o", "addr", "show", "dev", dev])
            .lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                w.by_ref().find(|t| *t == "inet")?;
                w.next()?.split('/').next()?.parse().ok()
            })
            .collect()
    }

    /// The system's index of an interface, as `ip -o link show` says it (the private network's
    /// own: `/sys` is the host's).
    fn index_of(dev: &str) -> u32 {
        let out = ip_out(&["-o", "link", "show", "dev", dev]);
        out.split(':').next().unwrap().trim().parse().unwrap_or_else(|_| panic!("{dev}: {out:?}"))
    }

    // ---------------------------------------------------------------------------------------------
    // The answer's match.

    /// A fault and a reject under the request's activity, each from another MAC, address or port,
    /// to another port or address, and on another interface (heard promiscuously, as on a mirror
    /// port, kept before the rest): all passed over, the device's own answer read after them.
    #[test]
    fn a_fault_or_reject_from_anywhere_else_is_passed_over() {
        netns::isolated("round_5::a_fault_or_reject_from_anywhere_else_is_passed_over", &[("a0", "b0"), ("a1", "b1")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV), ("a1", HERE1), ("b1", DEV1)], &[("a0", "b0"), ("a1", "b1")]);
            add("a0", &["192.168.0.1/24"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(
                &["b0", "b1"],
                me,
                Box::new(move |r: &Req| {
                    let good = answer(r, b"mine", PnioStatus::OK);
                    let fault = RpcPdu { packet_type: PacketType::Fault, ..good.clone() };
                    let reject = RpcPdu { packet_type: PacketType::Reject, ..good.clone() };
                    // On the other interface first, then a pause: a1's worker keeps them before
                    // a0's keeps what follows.
                    let mut out = vec![(1, frame(me, r.from, &fault)), (1, frame(me, r.from, &reject)), (PAUSE, Vec::new())];
                    for bad in [&fault, &reject] {
                        out.push((0, frame(Endpoint { mac: OTHER, ..me }, r.from, bad)));
                        out.push((0, frame(Endpoint { ip: a(99), ..me }, r.from, bad)));
                        out.push((0, frame(Endpoint { port: 34965, ..me }, r.from, bad)));
                        out.push((0, frame(me, Endpoint { port: r.from.port.wrapping_add(1), ..r.from }, bad)));
                        out.push((0, frame(me, Endpoint { ip: a(2), ..r.from }, bad)));
                    }
                    out.push((0, frame(me, r.from, &good)));
                    out
                }),
            );
            let mirror = RawLink::open_with("a1", RawOptions { ethertype: None, promiscuous: true, outgoing: false }).unwrap();
            let l = Listener::start(vec![bench::link("a0"), Box::new(mirror)], Options::default()).unwrap();
            let from = l.kept().1;
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.as_ref().map(|r| r.data.clone()).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            let r = r.unwrap();
            // The decoys were kept before the answer: the read passed over them.
            let act = peer.seen.lock().unwrap()[0].pdu.activity;
            let decoys: Vec<(u64, String)> = l
                .frames_from(from)
                .into_iter()
                .filter(|(i, k)| *i < r.answer && !k.sent && ip::datagram(&k.frame.bytes).is_some_and(|d| RpcPdu::decode(&d.payload).is_ok_and(|p| p.activity == act && matches!(p.packet_type, PacketType::Fault | PacketType::Reject))))
                .map(|(i, k)| (i, k.frame.iface.clone()))
                .collect();
            assert_eq!(decoys.iter().filter(|(_, i)| i == "a1").count(), 2, "{decoys:?}");
            assert_eq!(decoys.iter().filter(|(_, i)| i == "a0").count(), 10, "{decoys:?}");
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The kernel's list of addresses.

    /// The addresses are the system's list, in `ip -4 addr show`'s order, past a dump split
    /// across many reads (400 addresses on `lo` and 406 on the interface): a secondary, a
    /// labelled, a deprecated and a point-to-point address (its own end, never its peer's) among
    /// them; the read goes from the device's network's address listed last, past every split.
    #[test]
    fn the_addresses_are_the_system_s_list_in_its_order() {
        netns::isolated("round_5::the_addresses_are_the_system_s_list_in_its_order", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            let mut batch = String::new();
            for i in 0..400u32 {
                batch.push_str(&format!("addr add 127.{}.{}.{}/32 dev lo\n", 1 + i / 62500, (i / 250) % 250, i % 250 + 1));
            }
            batch.push_str("addr add 10.9.9.1/24 dev a0\naddr add 10.9.9.2/24 dev a0\naddr add 10.9.9.3/24 dev a0 label a0:x\n");
            batch.push_str("addr add 172.16.0.1 peer 172.16.0.2/32 dev a0\naddr add 10.7.0.1/16 dev a0 preferred_lft 0\n");
            for i in 0..400u32 {
                batch.push_str(&format!("addr add 10.200.{}.{}/32 dev a0\n", i / 250, i % 250 + 1));
            }
            batch.push_str("addr add 192.168.0.1/24 dev a0\n");
            let file = std::env::temp_dir().join(format!("aranha-read-round5-batch-{}", std::process::id()));
            std::fs::write(&file, batch).unwrap();
            netns::ip(&["-batch", file.to_str().unwrap()]);
            let _ = std::fs::remove_file(&file);
            let shown = ip_show("a0");
            assert_eq!(shown.len(), 406);
            assert!(shown.contains(&Ipv4Addr::new(172, 16, 0, 1)) && !shown.contains(&Ipv4Addr::new(172, 16, 0, 2)));
            assert!(shown.contains(&Ipv4Addr::new(10, 7, 0, 1)), "the deprecated address is listed");
            assert_eq!(read::addresses_of("a0"), shown);
            assert_eq!(read::addresses_at(index_of("a0")).unwrap(), shown);
            assert_eq!(read::addresses_of("lo").len(), 401);
            assert_eq!(read::addresses_of("lo"), ip_show("lo"));
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources(&peer), [a(1)]);
        });
    }

    /// A point-to-point address (`192.168.0.1 peer 192.168.0.2/32`): the interface holds .1 (the
    /// dump's local end), never its peer .2; the read goes from .1.
    #[test]
    fn a_point_to_point_address_is_its_own_end() {
        netns::isolated("round_5::a_point_to_point_address_is_its_own_end", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.1", "peer", "192.168.0.2/32"]);
            assert_eq!(ip_show("a0"), [a(1)]);
            assert_eq!(read::addresses_of("a0"), [a(1)]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources(&peer), [a(1)]);
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The address chosen.

    /// The device's own address held twice (two masks) beside an office one: refused by name,
    /// nothing sent; a /31 the device states: the interface's other end of it, past a /24 listed
    /// first.
    #[test]
    fn the_device_s_address_twice_and_a_slash_31() {
        netns::isolated("round_5::the_device_s_address_twice_and_a_slash_31", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.10/24"]);
            add("a0", &["192.168.0.10/16"]);
            add("a0", &["10.9.9.1/24"]);
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert!(matches!(r, Err(ReadError::OwnAddress { .. })), "{r:?}");
            assert_eq!(l.interface("a0").unwrap().sent, 0);
            l.stop();
            netns::ip(&["addr", "flush", "dev", "a0"]);
            add("a0", &["192.168.0.1/24"]);
            add("a0", &["192.168.0.11/31"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), Ipv4Addr::new(255, 255, 255, 254)), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources(&peer), [a(11)]);
        });
    }

    // ---------------------------------------------------------------------------------------------
    // The interface's addresses by the index its link was opened on.

    /// The interface the listener hears renamed (`a0` to `x0`, while up) and another given its old name: the
    /// listener says the index it opened, the read takes that interface's addresses, not the
    /// other's that now bears the name, and goes from them.
    #[test]
    fn a_rename_since_keeps_the_interface_s_addresses() {
        netns::isolated("round_5::a_rename_since_keeps_the_interface_s_addresses", &[("a0", "b0"), ("c0", "d0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV), ("c0", HERE2), ("d0", DEV2)], &[("a0", "b0"), ("c0", "d0")]);
            add("a0", &["192.168.0.1/24"]);
            add("c0", &["192.168.0.5/24"]);
            let opened = index_of("a0");
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            assert_eq!(l.index("a0"), Some(opened));
            assert_eq!(l.index("c0"), None, "a name it does not listen on");
            // Renamed while up (Linux renames a live interface since 6.2): no link goes down.
            netns::ip(&["link", "set", "dev", "a0", "name", "x0"]);
            netns::ip(&["link", "set", "dev", "c0", "name", "a0"]);
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(index_of("x0"), opened);
            assert_eq!(read::addresses_of("a0"), [a(5)], "the name now names the other");
            assert_eq!(l.index("a0"), Some(opened));
            assert_eq!(l.interface("a0").unwrap().ended, None, "the listener still hears x0");
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources(&peer), [a(1)], "the heard interface's own address, beside its MAC {HERE}");
        });
    }

    /// A listener opened on an alternative name, the name then moved to another interface (no link
    /// goes down: the listener hears on): the read takes the heard interface's addresses, none on
    /// the device's network, and is refused by name with nothing sent, not sent from the other
    /// interface's.
    #[test]
    fn an_alternative_name_moved_since_keeps_the_interface_s_addresses() {
        netns::isolated("round_5::an_alternative_name_moved_since_keeps_the_interface_s_addresses", &[("a0", "b0"), ("c0", "d0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV), ("c0", HERE2), ("d0", DEV2)], &[("a0", "b0"), ("c0", "d0")]);
            add("a0", &["10.9.9.1/24"]);
            add("c0", &["192.168.0.1/24"]);
            netns::ip(&["link", "property", "add", "dev", "a0", "altname", "plant0"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("plant0")], Options::default()).unwrap();
            assert_eq!(l.index("plant0"), Some(index_of("a0")));
            netns::ip(&["link", "property", "del", "dev", "a0", "altname", "plant0"]);
            netns::ip(&["link", "property", "add", "dev", "c0", "altname", "plant0"]);
            assert_eq!(read::addresses_of("plant0"), [a(1)], "the name now names c0");
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(l.interface("plant0").unwrap().ended, None, "the listener still hears a0");
            let r = read::read(&l, "plant0", &device_with("plant0", DEV, a(10), MASK24), IM0, &quick());
            match r {
                Err(ReadError::OffNetwork { iface, device, mask }) => assert_eq!((iface.as_str(), device, mask), ("plant0", a(10), MASK24)),
                other => panic!("{other:?}"),
            }
            assert_eq!(l.interface("plant0").unwrap().sent, 0);
            // The name's interface emptied: the refusal is still by the heard one's addresses.
            netns::ip(&["addr", "flush", "dev", "c0"]);
            let r = read::read(&l, "plant0", &device_with("plant0", DEV, a(10), MASK24), IM0, &quick());
            assert!(matches!(r, Err(ReadError::OffNetwork { .. })), "{r:?}");
            assert_eq!(l.interface("plant0").unwrap().sent, 0);
            std::thread::sleep(Duration::from_millis(200));
            assert!(sources(&peer).is_empty());
        });
    }

    /// A listener opened on an alternative name, the name then dropped: the interface it hears
    /// still holds its address, and the read goes from it.
    #[test]
    fn an_alternative_name_dropped_since_keeps_the_interface_s_addresses() {
        netns::isolated("round_5::an_alternative_name_dropped_since_keeps_the_interface_s_addresses", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.1/24"]);
            netns::ip(&["link", "property", "add", "dev", "a0", "altname", "plant0"]);
            let me = Endpoint { mac: DEV, ip: a(10), port: 34964 };
            let peer = scripted(&["b0"], me, mine());
            let l = Listener::start(vec![bench::link("plant0")], Options::default()).unwrap();
            netns::ip(&["link", "property", "del", "dev", "a0", "altname", "plant0"]);
            assert!(read::addresses_of("plant0").is_empty(), "the name names nothing now");
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(l.interface("plant0").unwrap().ended, None, "the listener still hears a0");
            let r = read::read(&l, "plant0", &device_with("plant0", DEV, a(10), MASK24), IM0, &quick());
            assert_eq!(r.map(|r| r.data).map_err(|e| e.to_string()), Ok(b"mine".to_vec()));
            assert_eq!(sources(&peer), [a(1)]);
        });
    }

    /// The interface the listener hears removed: refused at once (its index holds no address now)
    /// and, once its worker has heard the end, as ended.
    #[test]
    fn an_interface_removed_is_refused() {
        netns::isolated("round_5::an_interface_removed_is_refused", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.1/24"]);
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            netns::ip(&["link", "del", "a0"]);
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert!(matches!(r, Err(ReadError::NoAddress(_) | ReadError::Ended { .. })), "{r:?}");
            std::thread::sleep(Duration::from_millis(300));
            let r = read::read(&l, "a0", &device_with("a0", DEV, a(10), MASK24), IM0, &quick());
            assert!(matches!(r, Err(ReadError::Ended { .. })), "{r:?}");
            assert_eq!(l.interface("a0").unwrap().sent, 0);
        });
    }

    /// The interface's list of addresses the system cannot give (this process may open no more
    /// descriptors): refused by that reason, with nothing sent, never said to hold no address.
    #[test]
    fn a_list_of_addresses_the_system_cannot_give_is_refused_by_its_reason() {
        netns::isolated("round_5::a_list_of_addresses_the_system_cannot_give_is_refused_by_its_reason", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.1/24"]);
            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let device = device_with("a0", DEV, a(10), MASK24);
            let mut old = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            // SAFETY: getrlimit and setrlimit on valid structs; this child process runs this test
            // alone (`netns::isolated`).
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut old) };
            let none = libc::rlimit { rlim_cur: 0, rlim_max: old.rlim_max };
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &none) };
            let r = read::read(&l, "a0", &device, IM0, &quick());
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &old) };
            match &r {
                Err(e @ ReadError::Addresses { iface, source }) => {
                    assert_eq!((iface.as_str(), source.raw_os_error()), ("a0", Some(libc::EMFILE)));
                    assert!(e.to_string().starts_with("\"a0\"'s IPv4 addresses cannot be read ("), "{e}");
                    assert!(e.to_string().ends_with("): nothing is sent on it"), "{e}");
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(l.interface("a0").unwrap().sent, 0);
            // With descriptors again the same read is no longer refused on its addresses.
            assert_eq!(read::addresses_heard(&l, "a0").unwrap(), [a(1)]);
        });
    }

    /// A wire with no index of its own (a hub's port) named as an interface of the system's: its
    /// addresses are that name's, and a list the system cannot give is refused by its reason there
    /// too, nothing sent.
    #[test]
    fn a_wire_without_an_index_takes_its_name_s_addresses_and_their_failure() {
        netns::isolated("round_5::a_wire_without_an_index_takes_its_name_s_addresses_and_their_failure", &[("a0", "b0")], || {
            bench::pin(&[("a0", HERE), ("b0", DEV)], &[("a0", "b0")]);
            add("a0", &["192.168.0.1/24"]);
            let hub = veredas_aranha::wire::hub::Hub::new();
            let l = Listener::start(vec![Box::new(hub.port("a0", HERE))], Options::default()).unwrap();
            assert_eq!(l.index("a0"), None);
            assert_eq!(read::addresses_heard(&l, "a0").unwrap(), [a(1)]);
            let device = device_with("a0", DEV, a(10), MASK24);
            let mut old = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            // SAFETY: as above; this child process runs this test alone.
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut old) };
            let none = libc::rlimit { rlim_cur: 0, rlim_max: old.rlim_max };
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &none) };
            let r = read::read(&l, "a0", &device, IM0, &quick());
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &old) };
            match &r {
                Err(ReadError::Addresses { iface, source }) => assert_eq!((iface.as_str(), source.raw_os_error()), ("a0", Some(libc::EMFILE))),
                other => panic!("{other:?}"),
            }
            assert_eq!(l.interface("a0").unwrap().sent, 0);
        });
    }
}
