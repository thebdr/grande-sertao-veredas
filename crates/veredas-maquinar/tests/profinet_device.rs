//! The PROFINET device (contract: the PROFINET device clause): discovery and naming on a raw
//! interface, in a private network of the test's own (aranha's `netns::isolated`, a veth pair, no
//! privilege on the host). A controller on the pair's other end speaks aranha's codecs; every frame
//! the device sends is read back by Wireshark's dissectors, the second reading of the standard.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use veredas_aranha::netns;
use veredas_aranha::profinet::dcp::{self, BlockBody, DcpPdu, Filter, IpSuite, Service, ServiceType};
use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::lldp::{self, ChassisId, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::raw::{Options, RawLink};
use veredas_maquinar::profinet::*;

fn config(text: &str) -> ProfinetConfig {
    toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn link(iface: &str, ethertype: u16) -> RawLink {
    RawLink::open_with(iface, Options { ethertype: Some(ethertype), promiscuous: false, outgoing: false }).unwrap()
}

/// The next DCP PDU on `link` within `within`, with its frame's source and destination.
fn next_dcp(link: &RawLink, within: Duration) -> Option<(Mac, Mac, DcpPdu, Vec<u8>)> {
    let started = Instant::now();
    while started.elapsed() < within {
        let left = within.saturating_sub(started.elapsed());
        let r = link.recv(left).unwrap()?;
        let f = ether::parse(&r.bytes).unwrap();
        if f.ethertype == ether::ETHERTYPE_PROFINET {
            return Some((f.src, f.dst, dcp::decode(f.payload).unwrap(), r.bytes.clone()));
        }
    }
    None
}

const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nalias = \"port-001.switch-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nlldp_interval_ms = 200\nseed = 3\n";

#[test]
fn it_says_hello_and_answers_an_identify_it_matches() {
    netns::isolated("it_says_hello_and_answers_an_identify_it_matches", &[("a0", "b0")], || {
        let ctl = link("a0", ether::ETHERTYPE_PROFINET);
        let me = ctl.mac();
        let dev = start(&config(DEVICE)).unwrap();
        // The Hello, to the Hello group, saying who it is.
        let (src, dst, hello, _) = next_dcp(&ctl, Duration::from_secs(5)).expect("a Hello");
        assert_eq!((src, dst, hello.service), (dev.mac, Mac::DCP_HELLO_MULTICAST, Service::Hello));
        let id = dcp::DeviceIdentity::from_blocks(&hello.blocks);
        assert_eq!((id.name.as_deref(), id.vendor_id, id.device_id, id.mac), (Some("io-device-1"), Some(7), Some(9), Some(dev.mac)));
        // Identify: all, by its name, by its alias: answered, to the asker, with the request's xid.
        for (xid, filter) in [(10u32, Filter::All), (11, Filter::Name("io-device-1".into())), (12, Filter::Alias("port-001.switch-1".into()))] {
            ctl.send(&dcp::identify_request(me, xid, 1, &filter).unwrap()).unwrap();
            let (src, dst, answer, _) = next_dcp(&ctl, Duration::from_secs(5)).expect("an answer");
            assert_eq!((src, dst, answer.xid, answer.service, answer.service_type), (dev.mac, me, xid, Service::Identify, ServiceType::Success));
            let id = dcp::DeviceIdentity::from_blocks(&answer.blocks);
            assert_eq!(id.ip.map(|s| s.ip.to_string()), Some("192.168.0.10".into()));
        }
        // A name or an alias it does not have: no answer.
        for filter in [Filter::Name("io-device-2".into()), Filter::Alias("port-002.switch-1".into())] {
            ctl.send(&dcp::identify_request(me, 13, 1, &filter).unwrap()).unwrap();
            assert!(next_dcp(&ctl, Duration::from_millis(300)).is_none(), "{filter:?}");
        }
        let s = dev.status();
        assert_eq!((s.hellos, s.identify_answered, s.identify_passed), (1, 3, 2));
    });
}

#[test]
fn an_identify_is_answered_within_its_response_delay() {
    netns::isolated("an_identify_is_answered_within_its_response_delay", &[("a0", "b0")], || {
        let ctl = link("a0", ether::ETHERTYPE_PROFINET);
        let me = ctl.mac();
        let _dev = start(&config(&DEVICE.replace("seed = 3", "seed = 5\nhello = false"))).unwrap();
        // A factor of 50 spreads the answers over 0 to 490 ms; a factor of 1 answers at once.
        let mut waits = Vec::new();
        for xid in 0..12u32 {
            let asked = Instant::now();
            ctl.send(&dcp::identify_request(me, xid, 50, &Filter::All).unwrap()).unwrap();
            next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
            waits.push(asked.elapsed());
        }
        assert!(waits.iter().all(|w| *w < Duration::from_millis(600)), "{waits:?}");
        assert!(waits.iter().any(|w| *w > Duration::from_millis(50)), "spread, not all at once: {waits:?}");
        let asked = Instant::now();
        ctl.send(&dcp::identify_request(me, 99, 1, &Filter::All).unwrap()).unwrap();
        next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
        assert!(asked.elapsed() < Duration::from_millis(100), "{:?}", asked.elapsed());
    });
}

#[test]
fn a_set_names_addresses_and_blinks_the_device_and_each_block_is_answered() {
    netns::isolated("a_set_names_addresses_and_blinks_the_device_and_each_block_is_answered", &[("a0", "b0")], || {
        let ctl = link("a0", ether::ETHERTYPE_PROFINET);
        let me = ctl.mac();
        let dev = start(&config(&DEVICE.replace("station = \"io-device-1\"", "station = \"\"").replace("seed = 3", "hello = false"))).unwrap();
        // Unnamed: it answers no Identify by name, an empty one included.
        ctl.send(&dcp::identify_request(me, 1, 1, &Filter::Name(String::new())).unwrap()).unwrap();
        assert!(next_dcp(&ctl, Duration::from_millis(300)).is_none());
        let control = |pdu: &DcpPdu| dcp::control_responses(pdu);
        ctl.send(&dcp::set_name_request(dev.mac, me, 2, "conveyor-3", true).unwrap()).unwrap();
        let (_, dst, r, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        assert_eq!((dst, r.xid, r.service, control(&r)), (me, 2, Service::Set, vec![(2, 2, 0)]));
        assert_eq!(dev.station(), "conveyor-3");
        ctl.send(&dcp::identify_request(me, 3, 1, &Filter::Name("conveyor-3".into())).unwrap()).unwrap();
        assert!(next_dcp(&ctl, Duration::from_secs(5)).is_some(), "found by its new name");
        // A name the standard refuses: not set, the error said.
        // aranha's builders refuse such a name; a careless tool on the wire sends it all the same,
        // its bytes as the DCP layout has them: frame id, service Set, type request, xid, reserved,
        // the data's length, then the block (option 2, suboption 2, its length, the permanent
        // qualifier, the name).
        let name = b"Bad_Name";
        let mut pdu = vec![0xFE, 0xFD, 0x04, 0x00, 0, 0, 0, 4, 0, 0];
        pdu.extend_from_slice(&((4 + 2 + name.len()) as u16).to_be_bytes());
        pdu.extend_from_slice(&[0x02, 0x02]);
        pdu.extend_from_slice(&((2 + name.len()) as u16).to_be_bytes());
        pdu.extend_from_slice(&[0x00, 0x01]);
        pdu.extend_from_slice(name);
        ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
        let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        assert_eq!(control(&r), vec![(2, 2, error::SET_NOT_POSSIBLE)]);
        assert_eq!(dev.station(), "conveyor-3");
        let suite = IpSuite { ip: "10.0.0.5".parse().unwrap(), mask: "255.0.0.0".parse().unwrap(), gateway: "10.0.0.1".parse().unwrap() };
        ctl.send(&dcp::set_ip_request(dev.mac, me, 5, &suite, false).unwrap()).unwrap();
        let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        assert_eq!(control(&r), vec![(1, 2, 0)]);
        assert_eq!(dev.identity().ip, Some(suite));
        ctl.send(&dcp::signal_request(dev.mac, me, 6).unwrap()).unwrap();
        let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        assert_eq!(control(&r), vec![(5, 3, 0)]);
        assert_eq!(dev.status().blinks, 1);
        // A Get: the blocks asked for.
        ctl.send(&dcp::get_request(dev.mac, me, 7, &[(2, 2), (1, 2)]).unwrap()).unwrap();
        let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        assert_eq!((r.service, r.service_type), (Service::Get, ServiceType::Success));
        assert!(r.blocks.iter().any(|b| matches!(&b.body, BlockBody::NameOfStation { name } if name == "conveyor-3")));
        // Muted: nothing answered.
        dev.set_muted(true);
        ctl.send(&dcp::identify_request(me, 8, 1, &Filter::All).unwrap()).unwrap();
        assert!(next_dcp(&ctl, Duration::from_millis(300)).is_none());
    });
}

#[test]
fn lldp_names_the_station_and_the_port_and_says_when_it_leaves() {
    netns::isolated("lldp_names_the_station_and_the_port_and_says_when_it_leaves", &[("a0", "b0")], || {
        let ctl = link("a0", ether::ETHERTYPE_LLDP);
        let dev = start(&config(DEVICE)).unwrap();
        let mut seen = Vec::new();
        let started = Instant::now();
        while seen.len() < 3 && started.elapsed() < Duration::from_secs(5) {
            if let Some(r) = ctl.recv(Duration::from_secs(1)).unwrap() {
                let f = ether::parse(&r.bytes).unwrap();
                assert_eq!((f.src, f.dst), (dev.mac, Mac::LLDP_MULTICAST));
                seen.push((Instant::now(), lldp::decode(f.payload).unwrap()));
            }
        }
        assert_eq!(seen.len(), 3);
        let pdu = &seen[0].1;
        assert_eq!(pdu.chassis, ChassisId::Local { id: "io-device-1".into() });
        assert_eq!(pdu.port, PortId::Local { id: "port-001.io-device-1".into() });
        assert_eq!(pdu.pn.chassis_mac, Some(dev.mac));
        assert!(pdu.ttl > 0);
        let gap = seen[2].0 - seen[1].0;
        assert!(gap >= Duration::from_millis(150) && gap <= Duration::from_millis(600), "{gap:?}");
        dev.stop();
        let mut leaving = None;
        let started = Instant::now();
        while leaving.is_none() && started.elapsed() < Duration::from_secs(5) {
            if let Some(r) = ctl.recv(Duration::from_secs(1)).unwrap() {
                let p = lldp::decode(ether::parse(&r.bytes).unwrap().payload).unwrap();
                if p.ttl == 0 {
                    leaving = Some(p);
                }
            }
        }
        // The shutdown names the chassis and the port its frames named: a neighbour forgets those.
        let leaving = leaving.expect("a TTL of 0 when it stops");
        assert_eq!((&leaving.chassis, &leaving.port), (&pdu.chassis, &pdu.port));
    });
}

/// Every packet's layers as Wireshark's dissectors read them (`-T ek`), from a pcap of Ethernet
/// frames.
fn dissect(frames: &[Vec<u8>]) -> Vec<serde_json::Value> {
    let tshark = [std::env::var_os("VEREDAS_TSHARK").map(std::path::PathBuf::from), std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/opt/wireshark/tshark"))]
        .into_iter()
        .flatten()
        .chain([std::path::PathBuf::from("tshark")])
        .find(|c| std::process::Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false))
        .expect("the dissectors are this test's second reading of the standard: set VEREDAS_TSHARK or extract tshark into ~/.local/opt/wireshark");
    let mut pcap = Vec::new();
    pcap.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
    pcap.extend_from_slice(&2u16.to_le_bytes());
    pcap.extend_from_slice(&4u16.to_le_bytes());
    pcap.extend_from_slice(&[0; 8]);
    pcap.extend_from_slice(&65535u32.to_le_bytes());
    pcap.extend_from_slice(&1u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        pcap.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
        pcap.extend_from_slice(&0u32.to_le_bytes());
        pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
        pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
        pcap.extend_from_slice(f);
    }
    let file = std::env::temp_dir().join(format!("veredas-pn-dissect-{}.pcap", std::process::id()));
    std::fs::write(&file, pcap).unwrap();
    let out = std::process::Command::new(tshark).arg("-n").arg("-r").arg(&file).args(["-T", "ek"]).output().unwrap();
    std::fs::remove_file(&file).unwrap();
    assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with("{\"index\"")).map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["layers"].clone()).collect()
}

fn field(v: &serde_json::Value, key: &str, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, x) in map {
                if k == key {
                    match x {
                        serde_json::Value::Array(items) => out.extend(items.iter().map(|i| i.as_str().map(String::from).unwrap_or_else(|| i.to_string()))),
                        other => out.push(other.as_str().map(String::from).unwrap_or_else(|| other.to_string())),
                    }
                } else {
                    field(x, key, out);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|x| field(x, key, out)),
        _ => {}
    }
}

#[test]
fn the_dissectors_read_every_frame_the_device_sends_as_the_standard_has_it() {
    netns::isolated("the_dissectors_read_every_frame_the_device_sends_as_the_standard_has_it", &[("a0", "b0")], || {
        // Every frame on the controller's end: DCP and LLDP alike.
        let all = RawLink::open_with("a0", Options { ethertype: None, promiscuous: true, outgoing: false }).unwrap();
        let me = all.mac();
        let dev = start(&config(DEVICE)).unwrap();
        all.send(&dcp::identify_request(me, 21, 1, &Filter::All).unwrap()).unwrap();
        all.send(&dcp::set_name_request(dev.mac, me, 22, "conveyor-3", false).unwrap()).unwrap();
        all.send(&dcp::get_request(dev.mac, me, 23, &[(2, 2)]).unwrap()).unwrap();
        let mut frames = Vec::new();
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(900) {
            if let Some(r) = all.recv(Duration::from_millis(100)).unwrap()
                && ether::parse(&r.bytes).map(|f| f.src == dev.mac).unwrap_or(false)
            {
                frames.push(r.bytes);
            }
        }
        let packets = dissect(&frames);
        let mut complaints = Vec::new();
        for p in &packets {
            if !p["_ws_malformed"].is_null() {
                complaints.push(format!("malformed: {p}"));
            }
            let mut sev = Vec::new();
            field(p, "_ws_expert_severity", &mut sev);
            complaints.extend(sev.into_iter().filter(|s| s.parse::<u64>().unwrap_or(0) >= 6_291_456).map(|s| format!("{s}: {p}")));
        }
        assert_eq!(complaints, Vec::<String>::new());
        let all_of = |key: &str| {
            let mut out = Vec::new();
            packets.iter().for_each(|p| field(p, key, &mut out));
            out
        };
        // The Hello, the Identify's answer and the Get's answer name the station; the Set changed it.
        assert_eq!(all_of("pn_dcp_pn_dcp_suboption_device_nameofstation"), ["io-device-1", "io-device-1", "conveyor-3"]);
        assert_eq!(all_of("pn_dcp_pn_dcp_block_error"), ["0"], "the Set's one block, no error");
        assert!(all_of("pn_dcp_pn_dcp_xid").iter().any(|x| x == "21" || x == "0x00000015"), "{:?}", all_of("pn_dcp_pn_dcp_xid"));
        assert!(all_of("pn_dcp_pn_dcp_suboption_vendor_id").iter().all(|v| v == "7" || v == "0x0007"), "{:?}", all_of("pn_dcp_pn_dcp_suboption_vendor_id"));
        let ttls = all_of("lldp_lldp_time_to_live");
        assert!(!ttls.is_empty() && ttls.iter().all(|t| t != "0"), "{ttls:?}");
    });
}

#[test]
fn the_documented_fresh_device_is_found_named_and_found_again_through_its_bench_file() {
    netns::isolated("the_documented_fresh_device_is_found_named_and_found_again_through_its_bench_file", &[("a0", "b0")], || {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/bench-schema.md")).unwrap().replace("\r\n", "\n");
        let block = text.split("```toml\n").skip(1).map(|b| b[..b.find("```").unwrap()].to_string()).find(|b| b.contains("[[profinet]]")).expect("the reference's PROFINET example");
        let ctl = link("a0", ether::ETHERTYPE_PROFINET);
        let me = ctl.mac();
        let spec = veredas_maquinar::bench::load(&block, std::path::Path::new(".")).unwrap();
        assert_eq!(spec.plan(), ["profinet `fresh`: on b0 as no station name"]);
        let plant = veredas_maquinar::bench::Bench::start(spec).unwrap();
        let dev = plant.profinet("fresh").unwrap();
        // Found by an Identify of every device: unnamed, its ids as the file gives them.
        loop {
            let (_, _, pdu, _) = next_dcp(&ctl, Duration::from_secs(5)).expect("its Hello");
            if pdu.service == Service::Hello {
                break;
            }
        }
        ctl.send(&dcp::identify_request(me, 1, 1, &Filter::All).unwrap()).unwrap();
        let (src, _, answer, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        let id = dcp::DeviceIdentity::from_blocks(&answer.blocks);
        assert_eq!((src, id.name.as_deref(), id.vendor_id, id.device_id), (dev.mac, Some(""), Some(7), Some(9)));
        // Named and addressed, then found by its name.
        ctl.send(&dcp::set_name_request(src, me, 2, "press-7", true).unwrap()).unwrap();
        next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        let suite = IpSuite { ip: "192.168.1.7".parse().unwrap(), mask: "255.255.255.0".parse().unwrap(), gateway: "0.0.0.0".parse().unwrap() };
        ctl.send(&dcp::set_ip_request(src, me, 3, &suite, true).unwrap()).unwrap();
        next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        ctl.send(&dcp::identify_request(me, 4, 1, &Filter::Name("press-7".into())).unwrap()).unwrap();
        let (_, _, answer, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        let id = dcp::DeviceIdentity::from_blocks(&answer.blocks);
        assert_eq!((id.name.as_deref(), id.ip), (Some("press-7"), Some(suite)));
        assert_eq!(id.ip_status.map(|s| (s.set, s.conflict)), Some((1, false)));
        // The plant side: renamed by hand, and an address conflict flagged, both in the next answer.
        dev.set_station("press-8").unwrap();
        dev.set_ip_conflict(true);
        assert!(dev.set_station("Press 8").is_err(), "a name the standard refuses");
        ctl.send(&dcp::identify_request(me, 5, 1, &Filter::All).unwrap()).unwrap();
        let (_, _, answer, _) = next_dcp(&ctl, Duration::from_secs(5)).unwrap();
        let id = dcp::DeviceIdentity::from_blocks(&answer.blocks);
        assert_eq!((id.name.as_deref(), id.ip_status.map(|s| s.conflict)), (Some("press-8"), Some(true)));
        let lines = plant.lines();
        assert!(lines[0].starts_with(&format!("profinet fresh: on b0 as `press-8` ({}), 3 identify answered, 2 set(s)", dev.mac)), "{lines:?}");
        plant.stop();
    });
}

/// C-112's first refute round: the critic's probes, kept as pins (LLDP of an unnamed device and a
/// bare shutdown frame, the frame ids, the mute, a Get of what it lacks, the name and signal
/// refusals, the filters, the spread, the interval, the MAC, the port and alias names).
mod round_1 {
    use std::time::{Duration, Instant};

    use veredas_aranha::netns;
    use veredas_aranha::profinet::dcp::{self, BlockBody, DcpPdu, Filter, IpSuite};
    use veredas_aranha::profinet::ether;
    use veredas_aranha::profinet::lldp::{self, ChassisId, PortId};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_maquinar::profinet::*;

    fn config(text: &str) -> ProfinetConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn link(iface: &str, ethertype: u16) -> RawLink {
        RawLink::open_with(iface, Options { ethertype: Some(ethertype), promiscuous: false, outgoing: false }).unwrap()
    }

    fn next_dcp(link: &RawLink, within: Duration) -> Option<(Mac, Mac, DcpPdu, Vec<u8>)> {
        let started = Instant::now();
        while started.elapsed() < within {
            let left = within.saturating_sub(started.elapsed());
            let r = link.recv(left).unwrap()?;
            let f = ether::parse(&r.bytes).unwrap();
            if f.ethertype == ether::ETHERTYPE_PROFINET {
                return Some((f.src, f.dst, dcp::decode(f.payload).unwrap(), r.bytes.clone()));
            }
        }
        None
    }

    /// What the critic wrote to its observation files, kept for a failing run's output.
    fn obs(name: &str, text: &str) {
        eprintln!("{name}: {text}");
    }

    const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nalias = \"port-001.switch-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nlldp_interval_ms = 200\nseed = 3\n";

    /// A Set request's bytes as a careless tool sends them: frame id, service 4, type 0, xid, then the
    /// given blocks (each: option, suboption, qualifier, data; padded when odd).
    fn raw_set(frame_id: u16, xid: u32, blocks: &[(u8, u8, u16, &[u8])]) -> Vec<u8> {
        let mut data = Vec::new();
        for (o, s, q, d) in blocks {
            data.extend_from_slice(&[*o, *s]);
            data.extend_from_slice(&((2 + d.len()) as u16).to_be_bytes());
            data.extend_from_slice(&q.to_be_bytes());
            data.extend_from_slice(d);
            if (2 + d.len()) % 2 == 1 {
                data.push(0);
            }
        }
        let mut pdu = frame_id.to_be_bytes().to_vec();
        pdu.extend_from_slice(&[0x04, 0x00]);
        pdu.extend_from_slice(&xid.to_be_bytes());
        pdu.extend_from_slice(&[0, 0]);
        pdu.extend_from_slice(&(data.len() as u16).to_be_bytes());
        pdu.extend_from_slice(&data);
        pdu
    }

    /// An Identify request's bytes under any frame id, with raw filter blocks (option, suboption, data).
    fn raw_identify(frame_id: u16, xid: u32, delay: u16, blocks: &[(u8, u8, &[u8])]) -> Vec<u8> {
        let mut data = Vec::new();
        for (o, s, d) in blocks {
            data.extend_from_slice(&[*o, *s]);
            data.extend_from_slice(&(d.len() as u16).to_be_bytes());
            data.extend_from_slice(d);
            if d.len() % 2 == 1 {
                data.push(0);
            }
        }
        let mut pdu = frame_id.to_be_bytes().to_vec();
        pdu.extend_from_slice(&[0x05, 0x00]);
        pdu.extend_from_slice(&xid.to_be_bytes());
        pdu.extend_from_slice(&delay.to_be_bytes());
        pdu.extend_from_slice(&(data.len() as u16).to_be_bytes());
        pdu.extend_from_slice(&data);
        pdu
    }

    // ---- Real gaps (probes against the unmutated device) -------------------------------------------

    #[test]
    fn an_unnamed_device_sends_lldp_its_mac_the_chassis_and_its_shutdown() {
        netns::isolated("round_1::an_unnamed_device_sends_lldp_its_mac_the_chassis_and_its_shutdown", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_LLDP);
            let dev = start(&config(&DEVICE.replace("station = \"io-device-1\"", "station = \"\""))).unwrap();
            let mut seen = Vec::new();
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(1500) {
                if let Some(r) = ctl.recv(Duration::from_millis(100)).unwrap() {
                    seen.push(lldp::decode(ether::parse(&r.bytes).unwrap().payload).unwrap());
                }
            }
            let sent = dev.status().lldp_sent;
            let mac = dev.mac;
            dev.stop();
            let mut shutdown = 0;
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(1000) {
                if let Some(r) = ctl.recv(Duration::from_millis(100)).unwrap()
                    && lldp::decode(ether::parse(&r.bytes).unwrap().payload).unwrap().ttl == 0
                {
                    shutdown += 1;
                }
            }
            obs("p1", &format!("lldp frames in 1.5 s at 200 ms: {}; status.lldp_sent {sent}; TTL-0 frames at stop: {shutdown}\n{seen:#?}", seen.len()));
            assert!(seen.len() >= 5 && shutdown == 1, "an unnamed device: {} LLDP frame(s) in 1.5 s, lldp_sent {sent}, {shutdown} shutdown frame(s)", seen.len());
            // Its chassis its MAC, its port the port alone (refute round 2).
            assert_eq!((&seen[0].chassis, &seen[0].port, seen[0].pn.chassis_mac), (&ChassisId::Mac { mac }, &PortId::Local { id: "port-001".into() }, Some(mac)));
        });
    }

    #[test]
    fn a_name_that_is_not_utf8_is_refused_0x05() {
        netns::isolated("round_1::a_name_that_is_not_utf8_is_refused_0x05", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let mut got = Vec::new();
            for (xid, name) in [(1u32, &b"st\xe4tion"[..]), (2, &b"Station"[..]), (3, &b"a"[..])] {
                let pdu = raw_set(dcp::FRAME_ID_GET_SET, xid, &[(2, 2, 1, name)]);
                ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
                got.push((String::from_utf8_lossy(name).to_string(), dcp::control_responses(&r), dev.station()));
            }
            obs("p2", &format!("{got:#?}"));
            assert_eq!(got[0].1, vec![(2, 2, error::SET_NOT_POSSIBLE)], "{got:?}");
        });
    }

    #[test]
    fn a_request_under_another_service_s_frame_id_is_dropped() {
        netns::isolated("round_1::a_request_under_another_service_s_frame_id_is_dropped", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let mut lines = Vec::new();
            // A Set under the Identify request's, the Identify answer's and the Hello's frame ids.
            for (i, fid) in [dcp::FRAME_ID_IDENTIFY_REQ, dcp::FRAME_ID_IDENTIFY_RES, dcp::FRAME_ID_HELLO].into_iter().enumerate() {
                let name = format!("renamed-{i}");
                let pdu = raw_set(fid, 40 + i as u32, &[(2, 2, 1, name.as_bytes())]);
                ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let r = next_dcp(&ctl, Duration::from_millis(300));
                lines.push(format!("Set under {fid:#06x}: answered {:?}, station now `{}`", r.map(|(_, _, p, _)| (p.frame_id, dcp::control_responses(&p))), dev.station()));
            }
            // An Identify request under the Get/Set and the Hello frame ids, to the Identify group.
            for fid in [dcp::FRAME_ID_GET_SET, dcp::FRAME_ID_HELLO, dcp::FRAME_ID_IDENTIFY_RES] {
                let pdu = raw_identify(fid, 50, 1, &[(0xFF, 0xFF, &[])]);
                ctl.send(&ether::build(Mac::DCP_MULTICAST, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let r = next_dcp(&ctl, Duration::from_millis(300));
                lines.push(format!("Identify under {fid:#06x}: answered {:?}", r.map(|(_, _, p, _)| (p.frame_id, p.xid))));
            }
            obs("p3", &lines.join("\n"));
            assert!(lines.iter().all(|l| l.contains("answered None")), "{lines:#?}");
        });
    }

    #[test]
    fn a_muted_device_sends_nothing_after_the_mute() {
        netns::isolated("round_1::a_muted_device_sends_nothing_after_the_mute", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            // Seed 3 kept: its spread at a factor of 200 is well past the 30 ms before the mute (with
            // the clock's seed, one draw in 65 came sooner: refute round 2).
            let dev = start(&config(&DEVICE.replace("seed = 3", "seed = 3\nhello = false"))).unwrap();
            ctl.send(&dcp::identify_request(me, 77, 200, &Filter::All).unwrap()).unwrap();
            std::thread::sleep(Duration::from_millis(30));
            let muted_at = Instant::now();
            dev.set_muted(true);
            let late = next_dcp(&ctl, Duration::from_millis(2500)).map(|(_, _, p, _)| (p.xid, muted_at.elapsed()));
            let s = dev.status();
            obs("p4", &format!("answer after the mute: {late:?}; identify_answered {}", s.identify_answered));
            assert!(late.is_none(), "an Identify answer left {:?} after the device was muted", late.map(|l| l.1));
            assert_eq!(s.identify_answered, 0, "an answer the mute dropped is not counted");
        });
    }

    #[test]
    fn a_get_of_what_it_lacks_is_answered_0x02_pair_by_pair() {
        netns::isolated("round_1::a_get_of_what_it_lacks_is_answered_0x02_pair_by_pair", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false").replace("alias = \"port-001.switch-1\"\n", ""))).unwrap();
            ctl.send(&dcp::get_request(dev.mac, me, 9, &[(2, 6), (9, 1)]).unwrap()).unwrap();
            let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
            obs("p5", &format!("{r:#?}"));
            assert_eq!(dcp::control_responses(&r), [(2, 6, error::SUBOPTION_UNSUPPORTED), (9, 1, error::SUBOPTION_UNSUPPORTED)], "each pair it lacks answered with a control response: {r:?}");
        });
    }

    #[test]
    fn an_alias_too_long_for_a_frame_is_refused() {
        let alias = "a".repeat(1450);
        let cfg = config(&DEVICE.replace("port-001.switch-1", &alias));
        let message = format!("profinet.alias `{alias}`: an alias is a port name, a dot and a station name (port-001.switch-1)");
        assert_eq!(validate(&cfg).err().map(|e| e.to_string()), Some(message.clone()));
        // Refused before any interface is looked for: no device starts to send no Hello.
        assert_eq!(start(&cfg).err().map(|e| e.to_string()), Some(message));
    }

    #[test]
    fn an_identify_by_its_name_and_device_id_is_answered() {
        netns::isolated("round_1::an_identify_by_its_name_and_device_id_is_answered", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let _dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let pdu = raw_identify(dcp::FRAME_ID_IDENTIFY_REQ, 60, 1, &[(2, 2, b"io-device-1"), (2, 3, &[0, 7, 0, 9])]);
            ctl.send(&ether::build(Mac::DCP_MULTICAST, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
            let r = next_dcp(&ctl, Duration::from_millis(500));
            obs("p7", &format!("name + device id filter: {:?}", r.as_ref().map(|(_, _, p, _)| p.xid)));
            assert!(r.is_some(), "an Identify by its name and its own device id is unanswered");
        });
    }

    /// Every frame the device sends over the paths the clause's tshark test never dissects: an unnamed
    /// device's Hello and answer, a Set refused, an IP Set, the signal, a Get of several blocks, LLDP
    /// after naming and the shutdown frame.
    #[test]
    fn the_dissectors_read_the_unnamed_device_the_refusals_the_get_and_the_shutdown() {
        netns::isolated("round_1::the_dissectors_read_the_unnamed_device_the_refusals_the_get_and_the_shutdown", &[("a0", "b0")], || {
            let all = RawLink::open_with("a0", Options { ethertype: None, promiscuous: true, outgoing: false }).unwrap();
            let me = all.mac();
            let dev = start(&config(&DEVICE.replace("station = \"io-device-1\"", "station = \"\""))).unwrap();
            let mut frames = Vec::new();
            let dmac = dev.mac;
            let grab = |ms: u64, frames: &mut Vec<Vec<u8>>| {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(ms) {
                    if let Some(r) = all.recv(Duration::from_millis(50)).unwrap()
                        && ether::parse(&r.bytes).map(|f| f.src == dmac).unwrap_or(false)
                    {
                        frames.push(r.bytes);
                    }
                }
            };
            all.send(&dcp::identify_request(me, 21, 1, &Filter::All).unwrap()).unwrap();
            grab(300, &mut frames);
            let bad = raw_set(dcp::FRAME_ID_GET_SET, 22, &[(2, 2, 1, b"Bad_Name")]);
            all.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &bad).unwrap()).unwrap();
            let multi = raw_set(dcp::FRAME_ID_GET_SET, 23, &[(3, 61, 1, b"x"), (2, 1, 1, b"v")]);
            all.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &multi).unwrap()).unwrap();
            all.send(&dcp::set_name_request(dev.mac, me, 24, "named-now", true).unwrap()).unwrap();
            let suite = IpSuite { ip: "10.0.0.5".parse().unwrap(), mask: "255.0.0.0".parse().unwrap(), gateway: "10.0.0.1".parse().unwrap() };
            all.send(&dcp::set_ip_request(dev.mac, me, 25, &suite, false).unwrap()).unwrap();
            all.send(&dcp::signal_request(dev.mac, me, 26).unwrap()).unwrap();
            all.send(&dcp::get_request(dev.mac, me, 27, &[(1, 1), (1, 2), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (2, 6)]).unwrap()).unwrap();
            grab(700, &mut frames);
            dev.stop();
            grab(400, &mut frames);
            let packets = dissect(&frames);
            let mut complaints = Vec::new();
            for p in &packets {
                if !p["_ws_malformed"].is_null() {
                    complaints.push(format!("malformed: {p}"));
                }
                let mut sev = Vec::new();
                field(p, "_ws_expert_severity", &mut sev);
                complaints.extend(sev.into_iter().filter(|s| s.parse::<u64>().unwrap_or(0) >= 4_194_304).map(|s| format!("{s}: {p}")));
            }
            let mut ttls = Vec::new();
            packets.iter().for_each(|p| field(p, "lldp_lldp_time_to_live", &mut ttls));
            let mut errs = Vec::new();
            packets.iter().for_each(|p| field(p, "pn_dcp_pn_dcp_block_error", &mut errs));
            obs("p8", &format!("{} frames; ttls {ttls:?}; block errors {errs:?}\ncomplaints (note and up): {complaints:#?}\n\n{}", frames.len(), packets.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")));
            assert_eq!(complaints, Vec::<String>::new());
            assert_eq!(ttls.last().map(String::as_str), Some("0"), "the shutdown frame read last: {ttls:?}");
        });
    }

    fn dissect(frames: &[Vec<u8>]) -> Vec<serde_json::Value> {
        let tshark = std::path::Path::new(&std::env::var_os("HOME").unwrap()).join(".local/opt/wireshark/tshark");
        let mut pcap = Vec::new();
        pcap.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
        pcap.extend_from_slice(&2u16.to_le_bytes());
        pcap.extend_from_slice(&4u16.to_le_bytes());
        pcap.extend_from_slice(&[0; 8]);
        pcap.extend_from_slice(&65535u32.to_le_bytes());
        pcap.extend_from_slice(&1u32.to_le_bytes());
        for (i, f) in frames.iter().enumerate() {
            pcap.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
            pcap.extend_from_slice(&0u32.to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(f);
        }
        let file = std::env::temp_dir().join(format!("veredas-pn-round1-{}.pcap", std::process::id()));
        std::fs::write(&file, pcap).unwrap();
        let out = std::process::Command::new(tshark).arg("-n").arg("-r").arg(&file).args(["-T", "ek"]).output().unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with("{\"index\"")).map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["layers"].clone()).collect()
    }

    fn field(v: &serde_json::Value, key: &str, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, x) in map {
                    if k == key {
                        match x {
                            serde_json::Value::Array(items) => out.extend(items.iter().map(|i| i.as_str().map(String::from).unwrap_or_else(|| i.to_string()))),
                            other => out.push(other.as_str().map(String::from).unwrap_or_else(|| other.to_string())),
                        }
                    } else {
                        field(x, key, out);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|x| field(x, key, out)),
            _ => {}
        }
    }

    // ---- Mutant catchers (each holds on the unmutated device; the named mutant fails it) -----------

    /// Catches CRITIC_M_DELAY: the delay follows the factor.
    #[test]
    fn the_delay_follows_the_factor() {
        netns::isolated("round_1::the_delay_follows_the_factor", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let _dev = start(&config(&DEVICE.replace("seed = 3", "seed = 5\nhello = false"))).unwrap();
            let mut at_once = Vec::new();
            for xid in 0..5u32 {
                let asked = Instant::now();
                ctl.send(&dcp::identify_request(me, xid, 1, &Filter::All).unwrap()).unwrap();
                next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
                at_once.push(asked.elapsed());
            }
            let mut spread = Vec::new();
            for xid in 10..22u32 {
                let asked = Instant::now();
                ctl.send(&dcp::identify_request(me, xid, 50, &Filter::All).unwrap()).unwrap();
                next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
                spread.push(asked.elapsed());
            }
            let (lo, hi) = (spread.iter().min().unwrap(), spread.iter().max().unwrap());
            obs("k1", &format!("factor 1: {at_once:?}\nfactor 50: {spread:?}"));
            let mut sorted = at_once.clone();
            sorted.sort();
            assert!(sorted[2] < Duration::from_millis(20), "factor 1, median {:?}", sorted[2]);
            assert!(*hi - *lo > Duration::from_millis(150) && *hi < Duration::from_millis(520), "factor 50: {spread:?}");
        });
    }

    /// Catches CRITIC_M_SETERR and CRITIC_M_FIRST: a Set of several blocks, each answered with its
    /// own code.
    #[test]
    fn a_set_of_several_blocks_answers_each_with_its_code() {
        netns::isolated("round_1::a_set_of_several_blocks_answers_each_with_its_code", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let ip = [10, 0, 0, 5, 255, 0, 0, 0, 10, 0, 0, 1];
            let pdu = raw_set(dcp::FRAME_ID_GET_SET, 30, &[(2, 2, 1, b"multi-1"), (1, 2, 0, &ip), (3, 61, 1, b"x"), (2, 1, 1, b"v")]);
            ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
            let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
            let got = dcp::control_responses(&r);
            obs("k2", &format!("{got:?}; station {}; ip {:?}", dev.station(), dev.identity().ip));
            assert_eq!(got, vec![(2, 2, 0), (1, 2, 0), (3, 61, error::OPTION_UNSUPPORTED), (2, 1, error::SUBOPTION_UNSUPPORTED)]);
            assert_eq!((dev.station().as_str(), dev.identity().ip.map(|s| s.ip.to_string())), ("multi-1", Some("10.0.0.5".to_string())));
        });
    }

    /// Catches CRITIC_M_GETALL and CRITIC_M_GETFID: a Get answers exactly what it was asked, under
    /// the Get/Set frame id; and CRITIC_M_COUNTS: the Get and the LLDP frames counted.
    #[test]
    fn a_get_answers_exactly_the_blocks_asked_and_counts() {
        netns::isolated("round_1::a_get_answers_exactly_the_blocks_asked_and_counts", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            ctl.send(&dcp::get_request(dev.mac, me, 7, &[(2, 2), (1, 2)]).unwrap()).unwrap();
            let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
            let codes: Vec<(u8, u8)> = r.blocks.iter().map(|b| (b.option, b.suboption)).collect();
            std::thread::sleep(Duration::from_millis(500));
            let s = dev.status();
            obs("k3", &format!("frame id {:#06x}; blocks {codes:?}; gets {}; lldp_sent {}", r.frame_id, s.gets, s.lldp_sent));
            assert_eq!(r.frame_id, dcp::FRAME_ID_GET_SET);
            assert_eq!(codes, vec![(2, 2), (1, 2)], "the pairs in the order asked");
            assert!(r.blocks.iter().any(|b| matches!(&b.body, BlockBody::IpParameter { suite } if suite.ip.to_string() == "192.168.0.10")));
            assert_eq!(s.gets, 1);
            assert!(s.lldp_sent >= 2, "lldp_sent {}", s.lldp_sent);
        });
    }

    /// Catches CRITIC_M_ADDR: a Set or a Get not addressed to it is dropped.
    #[test]
    fn a_set_not_addressed_to_it_is_dropped() {
        netns::isolated("round_1::a_set_not_addressed_to_it_is_dropped", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let other = Mac([0x02, 0, 0, 0, 0, 0x99]);
            let mut lines = Vec::new();
            for dst in [Mac::DCP_MULTICAST, other] {
                let pdu = raw_set(dcp::FRAME_ID_GET_SET, 31, &[(2, 2, 1, b"stolen")]);
                ctl.send(&ether::build(dst, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let r = next_dcp(&ctl, Duration::from_millis(300)).is_some();
                lines.push(format!("Set to {dst}: answered {r}, station `{}`", dev.station()));
            }
            ctl.send(&dcp::get_request(other, me, 32, &[(2, 2)]).unwrap()).unwrap();
            lines.push(format!("Get to {other}: answered {}", next_dcp(&ctl, Duration::from_millis(300)).is_some()));
            obs("k4", &lines.join("\n"));
            assert_eq!(dev.station(), "io-device-1", "{lines:?}");
            assert!(lines.iter().all(|l| l.contains("answered false")), "{lines:?}");
        });
    }

    /// Catches CRITIC_M_MAC: the device's MAC is its interface's.
    #[test]
    fn the_device_mac_is_the_interface_s() {
        netns::isolated("round_1::the_device_mac_is_the_interface_s", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let dev = start(&config(DEVICE)).unwrap();
            let shown = std::process::Command::new("ip").args(["-o", "link", "show", "b0"]).output().unwrap();
            let shown = String::from_utf8_lossy(&shown.stdout).to_string();
            let sys = shown.split("link/ether ").nth(1).unwrap().split(' ').next().unwrap().to_string();
            let (src, _, hello, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("a Hello");
            let id = dcp::DeviceIdentity::from_blocks(&hello.blocks);
            obs("k5", &format!("sysfs {sys}; handle {}; hello src {src}; mac block {:?}", dev.mac, id.mac));
            assert_eq!((dev.mac.to_string(), src.to_string()), (sys.clone(), sys));
        });
    }

    /// Catches CRITIC_M_LLDP2X: LLDP on its interval, measured over five gaps.
    #[test]
    fn lldp_comes_on_its_interval() {
        netns::isolated("round_1::lldp_comes_on_its_interval", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_LLDP);
            let _dev = start(&config(DEVICE)).unwrap();
            let mut at = Vec::new();
            let started = Instant::now();
            while at.len() < 7 && started.elapsed() < Duration::from_secs(5) {
                if ctl.recv(Duration::from_millis(500)).unwrap().is_some() {
                    at.push(Instant::now());
                }
            }
            let mean = (at[6] - at[1]) / 5;
            obs("k6", &format!("mean gap over 5: {mean:?}"));
            assert!(mean > Duration::from_millis(170) && mean < Duration::from_millis(260), "{mean:?}");
        });
    }

    /// Catches CRITIC_M_CONFLICT: a flagged conflict keeps the address set.
    #[test]
    fn a_flagged_conflict_keeps_the_address_set() {
        netns::isolated("round_1::a_flagged_conflict_keeps_the_address_set", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            dev.set_ip_conflict(true);
            ctl.send(&dcp::identify_request(me, 5, 1, &Filter::All).unwrap()).unwrap();
            let (_, _, a, _) = next_dcp(&ctl, Duration::from_secs(2)).unwrap();
            let id = dcp::DeviceIdentity::from_blocks(&a.blocks);
            obs("k7", &format!("{:?}", id.ip_status));
            assert_eq!(id.ip_status.map(|s| (s.set, s.conflict)), Some((1, true)));
        });
    }

    #[test]
    fn a_signal_that_is_not_the_flash_is_refused() {
        netns::isolated("round_1::a_signal_that_is_not_the_flash_is_refused", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("seed = 3", "hello = false"))).unwrap();
            let mut got = Vec::new();
            for (xid, q, v) in [(1u32, 0u16, 0x0000u16), (2, 0, 0xBEEF), (3, 1, 0x0100)] {
                let pdu = raw_set(dcp::FRAME_ID_GET_SET, xid, &[(5, 3, q, &v.to_be_bytes())]);
                ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let (_, _, r, _) = next_dcp(&ctl, Duration::from_secs(2)).expect("an answer");
                got.push((q, v, dcp::control_responses(&r), dev.status().blinks));
            }
            obs("p9", &format!("(qualifier, value, answer, blinks): {got:?}"));
            assert_eq!(got[1].3, 0, "a signal value of 0x0000 or 0xBEEF counted as the identification signal: {got:?}");
            assert_eq!(got.iter().map(|g| (g.2.clone(), g.3)).collect::<Vec<_>>(), [(vec![(5, 3, error::SET_NOT_POSSIBLE)], 0), (vec![(5, 3, error::SET_NOT_POSSIBLE)], 0), (vec![(5, 3, 0)], 1)]);
        });
    }

    #[test]
    fn a_port_or_an_alias_the_standard_does_not_name_is_refused() {
        let mut lines = Vec::new();
        for (key, value) in [("port", "port-"), ("port", "port-abc"), ("port", &format!("port-{}", "1".repeat(300))), ("alias", &"a".repeat(1450)), ("alias", "Not An Alias!")] {
            let mut cfg = ProfinetConfig::new("b0");
            cfg.station = "io-device-1".into();
            match key {
                "port" => cfg.port = value.to_string(),
                _ => cfg.alias = Some(value.to_string()),
            }
            lines.push(format!("{key} `{}`: {:?}", &value[..value.len().min(20)], validate(&cfg).err().map(|e| e.to_string())));
        }
        obs("p10", &lines.join("\n"));
        assert!(lines.iter().all(|l| !l.ends_with("None")), "{lines:#?}");
    }
}

/// C-112's second refute round: the critic's probes (`~/.cache/veredas/critic-c112-r2/probes`), kept
/// as pins: every Identify filter kind, a mute silencing Set and Get, LLDP following a rename, a Get
/// under another frame id and a Set that is no request dropped, damaged blocks of suboptions it
/// holds refused 0x05, the empty name, and the configuration's refusals by their text.
mod round_2 {
    use std::time::{Duration, Instant};

    use veredas_aranha::netns;
    use veredas_aranha::profinet::dcp::{self, DcpPdu, Filter, Service, ServiceType};
    use veredas_aranha::profinet::ether;
    use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_maquinar::profinet::*;

    fn config(text: &str) -> ProfinetConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn link(iface: &str, ethertype: u16) -> RawLink {
        RawLink::open_with(iface, Options { ethertype: Some(ethertype), promiscuous: false, outgoing: false }).unwrap()
    }

    fn next_dcp(link: &RawLink, within: Duration) -> Option<DcpPdu> {
        let started = Instant::now();
        while started.elapsed() < within {
            let r = link.recv(within.saturating_sub(started.elapsed())).unwrap()?;
            let f = ether::parse(&r.bytes).unwrap();
            if f.ethertype == ether::ETHERTYPE_PROFINET {
                return Some(dcp::decode(f.payload).unwrap());
            }
        }
        None
    }

    fn next_lldp(link: &RawLink, within: Duration) -> Option<LldpPdu> {
        let started = Instant::now();
        while started.elapsed() < within {
            let r = link.recv(within.saturating_sub(started.elapsed())).unwrap()?;
            let f = ether::parse(&r.bytes).unwrap();
            if f.ethertype == ether::ETHERTYPE_LLDP {
                return Some(lldp::decode(f.payload).unwrap());
            }
        }
        None
    }

    /// The next LLDP frame sent after `settle`, what was queued before it passed over.
    fn fresh_lldp(link: &RawLink, settle: Duration) -> LldpPdu {
        std::thread::sleep(settle);
        while link.recv(Duration::from_millis(1)).unwrap().is_some() {}
        next_lldp(link, Duration::from_secs(2)).expect("an LLDP frame")
    }

    const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nalias = \"port-001.switch-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nlldp_interval_ms = 200\nseed = 3\nhello = false\n";

    /// A DCP PDU of any frame id, service and type, its blocks raw (the qualifier or info word
    /// included as given), each padded when odd.
    fn raw_pdu(frame_id: u16, service: u8, service_type: u8, xid: u32, delay: u16, blocks: &[(u8, u8, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (o, s, d) in blocks {
            data.extend_from_slice(&[*o, *s]);
            data.extend_from_slice(&(d.len() as u16).to_be_bytes());
            data.extend_from_slice(d);
            if d.len() % 2 == 1 {
                data.push(0);
            }
        }
        let mut pdu = frame_id.to_be_bytes().to_vec();
        pdu.extend_from_slice(&[service, service_type]);
        pdu.extend_from_slice(&xid.to_be_bytes());
        pdu.extend_from_slice(&delay.to_be_bytes());
        pdu.extend_from_slice(&(data.len() as u16).to_be_bytes());
        pdu.extend_from_slice(&data);
        pdu
    }

    /// A block raw: option, suboption, its bytes.
    type Raw = (u8, u8, Vec<u8>);

    fn with_q(q: u16, d: &[u8]) -> Vec<u8> {
        let mut v = q.to_be_bytes().to_vec();
        v.extend_from_slice(d);
        v
    }

    #[test]
    fn every_identify_filter_kind_is_matched_as_the_clause_says() {
        netns::isolated("round_2::every_identify_filter_kind_is_matched_as_the_clause_says", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            let ip = [192, 168, 0, 10, 255, 255, 255, 0, 0, 0, 0, 0].to_vec();
            let other_ip = [192, 168, 0, 11, 255, 255, 255, 0, 0, 0, 0, 0].to_vec();
            let cases: Vec<(&str, Vec<Raw>, bool)> = vec![
                ("vendor", vec![(2, 1, b"veredas bench device".to_vec())], true),
                ("another vendor", vec![(2, 1, b"another vendor".to_vec())], false),
                ("its role, io-device", vec![(2, 4, vec![0x01, 0x00])], true),
                ("a role it lacks, controller", vec![(2, 4, vec![0x02, 0x00])], false),
                ("its ip suite", vec![(1, 2, ip)], true),
                ("another ip suite", vec![(1, 2, other_ip)], false),
                ("its mac", vec![(1, 1, dev.mac.0.to_vec())], true),
                ("another mac", vec![(1, 1, vec![2, 0, 0, 0, 0, 0x55])], false),
                ("an instance (another kind)", vec![(2, 7, vec![0, 1])], false),
                ("an oem id (another kind)", vec![(2, 8, vec![0, 7, 0, 9])], false),
                ("all and an instance", vec![(0xFF, 0xFF, vec![]), (2, 7, vec![0, 1])], false),
            ];
            let mut got = Vec::new();
            for (i, (what, blocks, _)) in cases.iter().enumerate() {
                let pdu = raw_pdu(dcp::FRAME_ID_IDENTIFY_REQ, 5, 0, 100 + i as u32, 1, blocks);
                ctl.send(&ether::build(Mac::DCP_MULTICAST, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                got.push((*what, next_dcp(&ctl, Duration::from_millis(500)).map(|p| p.xid)));
            }
            let want: Vec<(&str, Option<u32>)> = cases.iter().enumerate().map(|(i, (w, _, yes))| (*w, yes.then_some(100 + i as u32))).collect();
            assert_eq!(got, want);
        });
    }

    #[test]
    fn a_mute_silences_set_and_get_too() {
        netns::isolated("round_2::a_mute_silences_set_and_get_too", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            dev.set_muted(true);
            ctl.send(&dcp::set_name_request(dev.mac, me, 2, "while-muted", true).unwrap()).unwrap();
            let set = next_dcp(&ctl, Duration::from_millis(300)).map(|p| p.xid);
            ctl.send(&dcp::get_request(dev.mac, me, 3, &[(2, 2)]).unwrap()).unwrap();
            let get = next_dcp(&ctl, Duration::from_millis(300)).map(|p| p.xid);
            assert_eq!((set, dev.station().as_str(), get), (None, "io-device-1", None));
        });
    }

    #[test]
    fn lldp_follows_a_set_of_its_name_and_a_rename_by_hand() {
        netns::isolated("round_2::lldp_follows_a_set_of_its_name_and_a_rename_by_hand", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let lctl = link("a0", ether::ETHERTYPE_LLDP);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            ctl.send(&dcp::set_name_request(dev.mac, me, 2, "renamed-1", true).unwrap()).unwrap();
            next_dcp(&ctl, Duration::from_secs(2)).expect("the Set's answer");
            let p = fresh_lldp(&lctl, Duration::from_millis(250));
            assert_eq!((p.chassis, p.port), (ChassisId::Local { id: "renamed-1".into() }, PortId::Local { id: "port-001.renamed-1".into() }));
            dev.set_station("by-hand").unwrap();
            let q = fresh_lldp(&lctl, Duration::from_millis(250));
            assert_eq!((q.chassis, q.port), (ChassisId::Local { id: "by-hand".into() }, PortId::Local { id: "port-001.by-hand".into() }));
        });
    }

    #[test]
    fn a_get_under_another_frame_id_and_a_set_that_is_no_request_are_dropped() {
        netns::isolated("round_2::a_get_under_another_frame_id_and_a_set_that_is_no_request_are_dropped", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            // A Get of the name under the Identify request's frame id, addressed to it.
            let get = raw_pdu(dcp::FRAME_ID_IDENTIFY_REQ, 3, 0, 9, 0, &[(2, 2, vec![])]);
            ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &get).unwrap()).unwrap();
            let g = next_dcp(&ctl, Duration::from_millis(300)).map(|p| p.xid);
            // A Set of service type 1 (a response), under the Set's own frame id.
            let set = raw_pdu(dcp::FRAME_ID_GET_SET, 4, 1, 10, 0, &[(2, 2, b"hijacked".to_vec())]);
            ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &set).unwrap()).unwrap();
            let s = next_dcp(&ctl, Duration::from_millis(300)).map(|p| p.xid);
            assert_eq!((g, s, dev.station().as_str()), (None, None, "io-device-1"));
        });
    }

    #[test]
    fn a_damaged_block_of_a_suboption_it_holds_is_refused_0x05_and_changes_nothing() {
        netns::isolated("round_2::a_damaged_block_of_a_suboption_it_holds_is_refused_0x05_and_changes_nothing", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            let cases: Vec<(Vec<Raw>, (u8, u8, u8))> = vec![
                // An IP suite of 8 bytes, and one with no qualifier word.
                (vec![(1, 2, with_q(1, &[10, 0, 0, 5, 255, 0, 0, 0]))], (1, 2, 5)),
                (vec![(1, 2, vec![0])], (1, 2, 5)),
                // A signal of 3 bytes.
                (vec![(5, 3, with_q(0, &[1, 0, 0]))], (5, 3, 5)),
                // A name that is not text (round 1's).
                (vec![(2, 2, with_q(1, b"st\xe4tion"))], (2, 2, 5)),
            ];
            for (i, (blocks, want)) in cases.iter().enumerate() {
                let pdu = raw_pdu(dcp::FRAME_ID_GET_SET, 4, 0, 200 + i as u32, 0, blocks);
                ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
                let r = next_dcp(&ctl, Duration::from_millis(500)).map(|p| dcp::control_responses(&p));
                assert_eq!(r, Some(vec![*want]), "{blocks:02X?}");
            }
            assert_eq!((dev.station().as_str(), dev.identity().ip.map(|s| s.ip.to_string())), ("io-device-1", Some("192.168.0.10".to_string())));
            assert_eq!(dev.status().blinks, 0);
        });
    }

    #[test]
    fn an_empty_name_unnames_the_device() {
        netns::isolated("round_2::an_empty_name_unnames_the_device", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let lctl = link("a0", ether::ETHERTYPE_LLDP);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            // An empty NameOfStation, as an engineering tool clears a name: taken.
            let pdu = raw_pdu(dcp::FRAME_ID_GET_SET, 4, 0, 1, 0, &[(2, 2, with_q(1, b""))]);
            ctl.send(&ether::build(dev.mac, me, ether::ETHERTYPE_PROFINET, &pdu).unwrap()).unwrap();
            let r = next_dcp(&ctl, Duration::from_millis(500)).map(|p| dcp::control_responses(&p));
            assert_eq!((r, dev.station().as_str()), (Some(vec![(2, 2, 0)]), ""));
            // Found no more by its old name; its LLDP chassis its MAC now.
            ctl.send(&dcp::identify_request(me, 300, 1, &Filter::Name("io-device-1".into())).unwrap()).unwrap();
            assert_eq!(next_dcp(&ctl, Duration::from_millis(300)).map(|p| p.xid), None);
            let l = fresh_lldp(&lctl, Duration::from_millis(250));
            assert_eq!((l.chassis, l.port), (ChassisId::Mac { mac: dev.mac }, PortId::Local { id: "port-001".into() }));
        });
    }

    #[test]
    fn once_a_mute_returns_no_answer_leaves_and_every_answer_counted_was_sent() {
        // Answers falling due all around the mute (two hundred Identify requests at a factor of 2,
        // so one pass of the device's loop sends many; the mute at a different moment each round):
        // the count read when the mute returns is every answer the controller ever receives, so
        // none left once it had returned (the orchestrator's run at 081ffa70 saw one leave 94 µs
        // after the call began; the bare flag let answers out here within ten rounds, three runs
        // of three).
        const N: u32 = 200;
        const F: u16 = 2;
        const W: u32 = 12;
        netns::isolated("round_2::once_a_mute_returns_no_answer_leaves_and_every_answer_counted_was_sent", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            let (mut received, mut before) = (0u64, 0u64);
            let mut cut = 0;
            for round in 0..100u32 {
                dev.set_muted(false);
                for i in 0..N {
                    ctl.send(&dcp::identify_request(me, round * 1000 + i, F, &Filter::All).unwrap()).unwrap();
                }
                std::thread::sleep(Duration::from_micros(u64::from(round % W) * 1_000));
                dev.set_muted(true);
                let counted = dev.status().identify_answered;
                while let Some(p) = next_dcp(&ctl, Duration::from_millis(60)) {
                    if p.service == Service::Identify && p.service_type == ServiceType::Success {
                        received += 1;
                    }
                }
                assert_eq!(received, counted, "round {round}: answers received against those counted when the mute returned");
                if counted - before < u64::from(N) {
                    cut += 1;
                }
                before = counted;
            }
            // The mute fell inside a burst in most rounds: it dropped answers due after it.
            assert!(cut >= 40, "the mute fell inside a burst in {cut} round(s) of 100");
            assert_eq!(dev.status().identify_answered, received);
        });
    }

    #[test]
    fn an_alias_s_halves_the_port_range_a_long_lldp_interval_and_a_misspelled_key_are_refused() {
        let alias = |a: &str| format!("profinet.alias `{a}`: an alias is a port name, a dot and a station name (port-001.switch-1)");
        let port = |p: &str| format!("profinet.port `{p}`: a port is named port-xyz or port-xyz-abcde (xyz 001 to 255, abcde five digits)");
        let cases: Vec<(&str, &str, String)> = vec![
            ("alias", "x.switch-1", alias("x.switch-1")),
            ("alias", "port-001.Bad_Name", alias("port-001.Bad_Name")),
            ("alias", "port-001.", alias("port-001.")),
            ("port", "port-000", port("port-000")),
            ("port", "port-256", port("port-256")),
            ("lldp", "3600001", "profinet.lldp_interval_ms: 3600001, over 3600000 (an hour, 802.1AB's longest)".into()),
        ];
        for (key, value, want) in cases {
            let mut cfg = ProfinetConfig::new("b0");
            cfg.station = "io-device-1".into();
            match key {
                "port" => cfg.port = value.to_string(),
                "lldp" => cfg.lldp_interval_ms = value.parse().unwrap(),
                _ => cfg.alias = Some(value.to_string()),
            }
            assert_eq!(validate(&cfg).err().map(|e| e.to_string()), Some(want), "{key} {value}");
        }
        // The edges taken: port-255, an hour.
        let mut cfg = ProfinetConfig::new("b0");
        cfg.port = "port-255".into();
        cfg.lldp_interval_ms = 3_600_000;
        assert!(validate(&cfg).is_ok());
        // A misspelled key, read straight into the configuration.
        let e = toml::from_str::<ProfinetConfig>("interface = \"b0\"\nstaton = \"io-device-1\"\n").unwrap_err().to_string();
        assert!(e.contains("unknown field `staton`"), "{e}");
    }
}

/// C-112's third refute round: the critic's probes (`~/.cache/veredas/critic-c112-r3/probes`), kept
/// as pins: the strict mute by the kernel's own send stamps, a mute lifted at once still dropping
/// what waited, the shutdown after a rename naming what its frames named, a Get too large for a
/// frame, the frame ids of a Hello, an Identify answer and a Set answer, a vendor not ASCII refused.
mod round_3 {
    use std::time::{Duration, Instant, SystemTime};

    use veredas_aranha::netns;
    use veredas_aranha::profinet::dcp::{self, DcpPdu, Filter, Service, ServiceType};
    use veredas_aranha::profinet::ether;
    use veredas_aranha::profinet::lldp::{self, LldpPdu};
    use veredas_aranha::profinet::mac::Mac;
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_maquinar::profinet::*;

    fn config(text: &str) -> ProfinetConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn link(iface: &str, ethertype: u16) -> RawLink {
        RawLink::open_with(iface, Options { ethertype: Some(ethertype), promiscuous: false, outgoing: false }).unwrap()
    }

    fn next_dcp(link: &RawLink, within: Duration) -> Option<DcpPdu> {
        let started = Instant::now();
        while started.elapsed() < within {
            let r = link.recv(within.saturating_sub(started.elapsed())).unwrap()?;
            let f = ether::parse(&r.bytes).unwrap();
            if f.ethertype == ether::ETHERTYPE_PROFINET {
                return Some(dcp::decode(f.payload).unwrap());
            }
        }
        None
    }

    const DEVICE: &str = "interface = \"b0\"\nstation = \"io-device-1\"\nalias = \"port-001.switch-1\"\nvendor_id = 7\ndevice_id = 9\nip = { address = \"192.168.0.10\", mask = \"255.255.255.0\" }\nlldp_interval_ms = 200\nseed = 3\nhello = false\n";

    /// A packet socket on the device's own end with the kernel's send stamp (SO_TIMESTAMPNS): a frame
    /// the device sends is stamped as it is handed to the interface (dev_queue_xmit_nit), inside the
    /// device's `send` call.
    struct Tap {
        fd: i32,
    }

    impl Tap {
        fn open(iface: &str) -> Tap {
            unsafe {
                let all = (libc::ETH_P_ALL as u16).to_be();
                let fd = libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, i32::from(all));
                assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
                let name = std::ffi::CString::new(iface).unwrap();
                let index = libc::if_nametoindex(name.as_ptr());
                assert!(index > 0);
                let mut addr: libc::sockaddr_ll = std::mem::zeroed();
                addr.sll_family = libc::AF_PACKET as u16;
                addr.sll_protocol = all;
                addr.sll_ifindex = index as i32;
                let rc = libc::bind(fd, (&raw const addr).cast(), std::mem::size_of::<libc::sockaddr_ll>() as u32);
                assert!(rc == 0, "bind: {}", std::io::Error::last_os_error());
                let on: libc::c_int = 1;
                let rc = libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_TIMESTAMPNS, (&raw const on).cast(), 4);
                assert!(rc == 0, "SO_TIMESTAMPNS: {}", std::io::Error::last_os_error());
                let size: libc::c_int = 8 << 20;
                libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, (&raw const size).cast(), 4);
                Tap { fd }
            }
        }

        /// The next frame within `timeout`: its bytes, the kernel's stamp, its packet type.
        fn recv(&self, timeout: Duration) -> Option<(Vec<u8>, SystemTime, u8)> {
            unsafe {
                let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
                let rc = libc::poll(&raw mut pfd, 1, timeout.as_millis() as i32);
                if rc <= 0 {
                    return None;
                }
                let mut buf = vec![0u8; 2048];
                let mut from: libc::sockaddr_ll = std::mem::zeroed();
                let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
                let mut control = [0u64; 32];
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_name = (&raw mut from).cast();
                msg.msg_namelen = std::mem::size_of::<libc::sockaddr_ll>() as u32;
                msg.msg_iov = &raw mut iov;
                msg.msg_iovlen = 1;
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen = std::mem::size_of_val(&control);
                let n = libc::recvmsg(self.fd, &raw mut msg, libc::MSG_DONTWAIT);
                if n < 0 {
                    return None;
                }
                buf.truncate(n as usize);
                let mut stamp = None;
                let mut cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
                while !cmsg.is_null() {
                    if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_TIMESTAMPNS {
                        let ts: libc::timespec = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast());
                        stamp = Some(SystemTime::UNIX_EPOCH + Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32));
                    }
                    cmsg = libc::CMSG_NXTHDR(&raw const msg, cmsg);
                }
                Some((buf, stamp.expect("a kernel stamp"), from.sll_pkttype))
            }
        }
    }

    /// DCP answers the device sent (stamp, frame id, service, type), read from the tap until it is
    /// quiet for `quiet`.
    fn drain_answers(tap: &Tap, dev: Mac, quiet: Duration) -> Vec<(SystemTime, u16, Service, ServiceType)> {
        let mut out = Vec::new();
        while let Some((bytes, at, kind)) = tap.recv(quiet) {
            if kind != libc::PACKET_OUTGOING {
                continue;
            }
            let Ok(f) = ether::parse(&bytes) else { continue };
            if f.src != dev || f.ethertype != ether::ETHERTYPE_PROFINET {
                continue;
            }
            if let Ok(p) = dcp::decode(f.payload) {
                out.push((at, p.frame_id, p.service, p.service_type));
            }
        }
        out
    }

    #[test]
    fn no_dcp_frame_is_handed_to_the_interface_after_the_mute_returns() {
        // Bursts of 200 Identify requests (factor 1), Sets of the signal and Gets, the mute at a
        // different moment each round: every DCP answer is stamped by the kernel as it is handed
        // to the interface, and none carries a stamp past the moment `set_muted(true)` returned.
        netns::isolated("round_3::no_dcp_frame_is_handed_to_the_interface_after_the_mute_returns", &[("a0", "b0")], || {
            let tap = Tap::open("b0");
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            drain_answers(&tap, dev.mac, Duration::from_millis(50));
            let (mut late, mut total, mut cut) = (Vec::new(), 0usize, 0u32);
            for round in 0..150u32 {
                let kind = ["identify", "set", "get"][round as usize % 3];
                dev.set_muted(false);
                for i in 0..200u32 {
                    let f = match kind {
                        "identify" => dcp::identify_request(me, round * 1000 + i, 1, &Filter::All).unwrap(),
                        "set" => dcp::signal_request(dev.mac, me, round * 1000 + i).unwrap(),
                        _ => dcp::get_request(dev.mac, me, round * 1000 + i, &[(2, 2)]).unwrap(),
                    };
                    ctl.send(&f).unwrap();
                }
                std::thread::sleep(Duration::from_micros(u64::from(round % 12) * 100));
                dev.set_muted(true);
                let returned = SystemTime::now();
                let answers = drain_answers(&tap, dev.mac, Duration::from_millis(60));
                while ctl.recv(Duration::from_millis(1)).unwrap().is_some() {}
                total += answers.len();
                if answers.len() < 200 {
                    cut += 1;
                }
                for (at, _, service, _) in answers {
                    if at > returned {
                        late.push(format!("round {round} ({kind}): {service:?} {:?} after the mute returned", at.duration_since(returned).unwrap()));
                    }
                }
            }
            assert!(total > 0 && cut > 0, "no burst cut by the mute");
            assert_eq!(late, Vec::<String>::new());
        });
    }

    #[test]
    fn a_mute_lifted_at_once_still_drops_the_answer_that_waited() {
        netns::isolated("round_3::a_mute_lifted_at_once_still_drops_the_answer_that_waited", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            for round in 0..5u32 {
                ctl.send(&dcp::identify_request(me, 500 + round, 200, &Filter::All).unwrap()).unwrap();
                std::thread::sleep(Duration::from_millis(2));
                dev.set_muted(true);
                dev.set_muted(false);
                assert_eq!(next_dcp(&ctl, Duration::from_millis(2100)).map(|p| p.xid), None, "round {round}");
            }
            assert_eq!(dev.status().identify_answered, 0);
            // An Identify after the mute was lifted is answered as ever.
            ctl.send(&dcp::identify_request(me, 600, 1, &Filter::All).unwrap()).unwrap();
            assert_eq!(next_dcp(&ctl, Duration::from_millis(500)).map(|p| p.xid), Some(600));
        });
    }

    fn lldp_until(link: &RawLink, within: Duration, mut stop: impl FnMut(&LldpPdu) -> bool) -> Vec<LldpPdu> {
        let mut seen = Vec::new();
        let started = Instant::now();
        while started.elapsed() < within {
            if let Some(r) = link.recv(Duration::from_millis(50)).unwrap() {
                let f = ether::parse(&r.bytes).unwrap();
                if f.ethertype == ether::ETHERTYPE_LLDP {
                    let p = lldp::decode(f.payload).unwrap();
                    let done = stop(&p);
                    seen.push(p);
                    if done {
                        break;
                    }
                }
            }
        }
        seen
    }

    #[test]
    fn the_shutdown_after_a_rename_names_what_its_frames_named() {
        netns::isolated("round_3::the_shutdown_after_a_rename_names_what_its_frames_named", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let lctl = link("a0", ether::ETHERTYPE_LLDP);
            let me = ctl.mac();
            // The reference's interval: a rename and a stop within it.
            let dev = start(&config(&DEVICE.replace("lldp_interval_ms = 200", "lldp_interval_ms = 1000"))).unwrap();
            let first = lldp_until(&lctl, Duration::from_secs(2), |_| true);
            ctl.send(&dcp::set_name_request(dev.mac, me, 2, "renamed-1", true).unwrap()).unwrap();
            assert!(next_dcp(&ctl, Duration::from_millis(500)).is_some(), "the Set's answer");
            dev.stop();
            let rest = lldp_until(&lctl, Duration::from_secs(2), |p| p.ttl == 0);
            let shutdown = rest.iter().find(|p| p.ttl == 0).expect("a shutdown");
            let named: Vec<_> = first.iter().chain(rest.iter()).filter(|p| p.ttl > 0).map(|p| (&p.chassis, &p.port)).collect();
            assert_eq!((&shutdown.chassis, &shutdown.port), (&first[0].chassis, &first[0].port));
            assert!(named.contains(&(&shutdown.chassis, &shutdown.port)), "{named:?}");
        });
    }

    #[test]
    fn a_get_whose_answer_would_not_fit_a_frame_is_neither_answered_nor_counted() {
        netns::isolated("round_3::a_get_whose_answer_would_not_fit_a_frame_is_neither_answered_nor_counted", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(DEVICE)).unwrap();
            let pairs: Vec<(u8, u8)> = (0..200u8).map(|i| (0x80, i)).collect();
            ctl.send(&dcp::get_request(dev.mac, me, 9, &pairs).unwrap()).unwrap();
            assert!(next_dcp(&ctl, Duration::from_millis(500)).is_none());
            assert_eq!(dev.status().gets, 0);
            ctl.send(&dcp::get_request(dev.mac, me, 10, &[(2, 2)]).unwrap()).unwrap();
            assert_eq!(next_dcp(&ctl, Duration::from_millis(500)).map(|p| p.xid), Some(10));
            assert_eq!(dev.status().gets, 1);
        });
    }

    #[test]
    fn a_hello_an_identify_answer_and_a_set_answer_go_under_their_frame_ids() {
        netns::isolated("round_3::a_hello_an_identify_answer_and_a_set_answer_go_under_their_frame_ids", &[("a0", "b0")], || {
            let ctl = link("a0", ether::ETHERTYPE_PROFINET);
            let me = ctl.mac();
            let dev = start(&config(&DEVICE.replace("hello = false\n", ""))).unwrap();
            let hello = next_dcp(&ctl, Duration::from_millis(500)).unwrap();
            assert_eq!((hello.service, hello.frame_id), (Service::Hello, 0xFEFC));
            ctl.send(&dcp::identify_request(me, 1, 1, &Filter::All).unwrap()).unwrap();
            let answer = next_dcp(&ctl, Duration::from_millis(500)).unwrap();
            assert_eq!((answer.service, answer.service_type, answer.frame_id), (Service::Identify, ServiceType::Success, dcp::FRAME_ID_IDENTIFY_RES));
            ctl.send(&dcp::signal_request(dev.mac, me, 2).unwrap()).unwrap();
            let set = next_dcp(&ctl, Duration::from_millis(500)).unwrap();
            assert_eq!((set.service, set.frame_id), (Service::Set, dcp::FRAME_ID_GET_SET));
        });
    }

    #[test]
    fn a_vendor_text_dcp_cannot_carry_is_refused_before_anything_opens() {
        for vendor in ["Fábrica de bancada", "", &"v".repeat(256), "tab\there"] {
            let mut cfg = ProfinetConfig::new("b0");
            cfg.vendor = vendor.to_string();
            assert_eq!(validate(&cfg).err().map(|e| e.to_string()), Some(format!("profinet.vendor `{vendor}`: 1 to 255 visible ASCII characters")), "{vendor:?}");
        }
        let mut cfg = ProfinetConfig::new("b0");
        cfg.vendor = "v".repeat(255);
        assert!(validate(&cfg).is_ok());
    }
}
