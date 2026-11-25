//! The scan file (contract: the scan file clause): a listening's counts, devices with every value
//! and its witnesses, conflicts and topology written as `veredas-scan` version 1 beside the
//! capture of its frames and read back whole; the header refused by core's reader, every key read
//! or the file refused; `docs/scan-schema.md`'s example is the file of the sample scan below.

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use veredas_aranha::listen::{Listener, Options};
use veredas_aranha::observe::{self, Observed};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpStatus, IpSuite};
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::scan::{Capture, FORMAT, Scan, ScanError, VERSION};
use veredas_aranha::topology::{self, Interface};
use veredas_aranha::wire::Frame;
use veredas_aranha::wire::pcap::{self, Replay};

const DEV: Mac = Mac([0x02, 0, 0, 0, 0, 0x01]);
const PORT: Mac = Mac([0x02, 0, 0, 0, 0x01, 0x01]);
const TWIN: Mac = Mac([0x02, 0, 0, 0, 0, 0x02]);
const T0: u64 = 1_764_000_000_000_000_000;

/// A device's Hello, its port's LLDP and its ARP, and a second device of its name: the frames of
/// the sample scan, a second apart.
fn frames() -> Vec<Frame> {
    let id = |ip| DeviceIdentity {
        name: Some("bomba-1".into()),
        ip: Some(IpSuite { ip, mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
        ip_status: Some(IpStatus { set: 1, conflict: false }),
        vendor_id: Some(42),
        device_id: Some(261),
        ..Default::default()
    };
    let mut pdu = LldpPdu::new(ChassisId::Local { id: "bomba-1".into() }, PortId::Local { id: "port-001.bomba-1".into() }, 20);
    pdu.pn.chassis_mac = Some(DEV);
    let bytes = [
        dcp::hello(DEV, 1, &id(Ipv4Addr::new(192, 168, 0, 10))).unwrap(),
        lldp::frame(PORT, &pdu).unwrap(),
        observe::arp_frame(DEV, Ipv4Addr::new(192, 168, 0, 10), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap(),
        dcp::hello(TWIN, 1, &id(Ipv4Addr::new(192, 168, 0, 11))).unwrap(),
    ];
    bytes.into_iter().enumerate().map(|(i, b)| Frame::new(T0 + i as u64 * 1_000_000_000, "eth1", b)).collect()
}

/// The sample scan: its frames replayed through a listener as eth1, the topology of what it heard.
fn sample() -> Scan {
    let l = Listener::start(vec![Box::new(Replay::new("eth1", frames()))], Options::default()).unwrap();
    assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["eth1"].ended.is_some()));
    let s = l.stop();
    let t = topology::build(&s.fusion, &[Interface { name: "eth1".into(), mac: None }], &[]);
    Scan::new(&s.fusion, &s.stats, &t, T0 + 10_000_000_000, Some(Capture { file: "sample.pcap".into(), first_frame: 0, frames: 4 }))
}

/// The example `docs/scan-schema.md` gives: its one TOML block under "An example".
fn documented() -> String {
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/scan-schema.md")).unwrap();
    let after = doc.split_once("## An example").expect("the schema has an example").1;
    let block = after.split_once("```toml\n").unwrap().1;
    block.split_once("```").unwrap().0.to_string()
}

#[test]
fn the_documented_example_is_the_sample_scan_s_file() {
    let scan = sample();
    let text = scan.to_toml().unwrap();
    assert_eq!(documented(), text, "docs/scan-schema.md's example is not the sample's file:\n{text}");
    assert!(text.starts_with(&format!("format = \"{FORMAT}\"\nversion = {VERSION}\n")));
    assert_eq!(Scan::read(&documented()).unwrap(), scan);
}

#[test]
fn a_scan_is_read_back_whole_and_its_witnesses_name_its_capture_s_frames() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scan");
    std::fs::create_dir_all(&dir).unwrap();
    let scan = sample();
    scan.write_file(&dir.join("sample.toml")).unwrap();
    pcap::write_file(&dir.join("sample.pcap"), &frames()).unwrap();
    let read = Scan::read_file(&dir.join("sample.toml")).unwrap();
    assert_eq!(read, scan);
    // What it found: two devices of one name, the conflict on record, DEV's port and its cable.
    let f = read.fusion();
    assert_eq!(f.devices.keys().copied().collect::<Vec<_>>(), [DEV, TWIN]);
    assert_eq!(read.conflicts, f.conflicts());
    assert_eq!(read.conflicts.len(), 1);
    assert_eq!(read.stats.heard, 4);
    assert!(read.topology.links.iter().any(|l| l.b.node == "if:eth1" && l.a.node == topology::device_id(DEV)));
    // Every name, chassis and ARP address witnessed by a record of the capture that says it.
    let capture = read.capture.clone().unwrap();
    let records = pcap::read_file(&dir.join(&capture.file), "eth1").unwrap().frames;
    assert_eq!(records.len() as u64, capture.frames);
    for d in &read.devices {
        let said = |frame: u64| observe::decode(&records[(frame - capture.first_frame) as usize], frame).observations;
        for c in &d.name.candidates {
            for w in [&c.first, &c.last] {
                assert!(said(w.frame).iter().any(|o| matches!(&o.observed, Observed::Dcp { identity, .. } if identity.name.as_ref() == Some(&c.value))), "{} {w:?}", c.value);
            }
        }
        for c in &d.arp_ip.candidates {
            assert!(said(c.first.frame).iter().any(|o| matches!(&o.observed, Observed::Arp { ip, .. } if *ip == c.value)));
        }
        for c in &d.chassis.candidates {
            assert!(said(c.first.frame).iter().any(|o| matches!(&o.observed, Observed::Lldp { pdu, .. } if pdu.chassis.key() == c.value)));
        }
    }
}

#[test]
fn a_file_of_another_format_or_version_or_with_a_key_nobody_reads_is_refused() {
    let text = sample().to_toml().unwrap();
    let refused = |t: &str| Scan::read(t).unwrap_err().to_string();
    // The header, by core's reader and its texts, before the body is looked at.
    assert_eq!(refused(&text.replacen("format = \"veredas-scan\"", "format = \"veredas-bench\"", 1)), "`format` is `veredas-bench`, not `veredas-scan`");
    assert_eq!(refused(&text.replacen("version = 1\n", "version = 2\n", 1)), "version 2 of `veredas-scan` is not known here (this reader knows 1)");
    assert_eq!(refused(&text.replacen("format = \"veredas-scan\"\n", "", 1)), "no `format`: a scan file states `format` and `version`");
    assert!(refused("format = [").starts_with("not a scan file: "));
    // A key nobody reads, at the top, in a device and in a witness: refused, each named.
    let extra = text.replacen("tool = ", "colour = \"red\"\ntool = ", 1);
    assert_eq!(refused(&extra), "a scan file holds no key `colour`: refused, not ignored");
    let in_device = text.replacen("by_source = false\n", "by_source = false\nnickname = \"pump\"\n", 1);
    assert!(matches!(Scan::read(&in_device), Err(ScanError::Unread(k)) if k.len() == 1 && k[0].contains("nickname")));
    let in_witness = text.replacen("frame = 0\n", "frame = 0\nseen_by = \"me\"\n", 1);
    assert!(matches!(Scan::read(&in_witness), Err(ScanError::Unread(k)) if k[0].contains("seen_by")));
    // A value of the wrong kind, and a key missing, are no scan.
    assert!(refused(&text.replacen("taken_ns = ", "taken_ns = \"soon\"\n#", 1)).starts_with("not a scan file: "));
    assert!(refused(&text.replacen("frames = 4\n", "", 1)).starts_with("not a scan file: "));
    // A stamp past what TOML's integers hold cannot be written, said.
    let mut far = sample();
    far.taken_ns = u64::MAX;
    assert!(matches!(far.to_toml(), Err(ScanError::Write(_))));
}
