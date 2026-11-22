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
        assert!(leaving.is_some(), "a TTL of 0 when it stops");
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
