//! The emulated EtherCAT segment (contract: the EtherCAT segment clause): SubDevices answering on a
//! raw interface in a private network of the test's own (aranha's `netns::isolated`, a veth pair),
//! brought up by an EtherCAT MainDevice the bench did not write (ethercrab, the crate core's master
//! stands on): found, their EEPROMs read, taken to OP, their process data exchanged both ways.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use ethercrab::std::ethercat_now;
use ethercrab::{MainDevice, MainDeviceConfig, PduStorage, Timeouts};
use veredas_aranha::netns;
use veredas_maquinar::ethercat::*;

fn config(text: &str) -> EthercatConfig {
    toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

const MAX_SUBDEVICES: usize = 8;
const MAX_PDU_DATA: usize = PduStorage::element_size(1100);
const MAX_FRAMES: usize = 16;
const PDI_LEN: usize = 64;

static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();

const SEGMENT: &str = "interface = \"b0\"\ndevices = [\n\
  { name = \"coupler\", product_code = 1, revision = 1, serial = 11, product_name = \"bench coupler\" },\n\
  { name = \"out16\", product_code = 2, revision = 3, serial = 12, product_name = \"bench outputs\", outputs = [ { index = 0x7000, subindex = 1, bits = 8 }, { index = 0x7010, subindex = 1, bits = 8 } ] },\n\
  { name = \"in8\", product_code = 3, revision = 1, serial = 13, alias = 0x0102, product_name = \"bench inputs\", inputs = [ { index = 0x6000, subindex = 1, bits = 8 } ] },\n\
]\n";

/// A MainDevice on `a0`, its frames carried by a thread of its own.
fn maindevice() -> Arc<MainDevice<'static>> {
    let (tx, rx, pdu_loop) = PDU_STORAGE.try_split().expect("split once");
    let maindevice = Arc::new(MainDevice::new(
        pdu_loop,
        Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(5), pdu: Duration::from_millis(500), ..Default::default() },
        MainDeviceConfig::default(),
    ));
    std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
    maindevice
}

#[test]
fn a_maindevice_finds_the_segment_brings_it_to_op_and_exchanges_process_data() {
    netns::isolated("a_maindevice_finds_the_segment_brings_it_to_op_and_exchanges_process_data", &[("a0", "b0")], || {
        let segment = start(&config(SEGMENT)).unwrap();
        let maindevice = maindevice();
        futures_lite::future::block_on(async {
            let group = maindevice.init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now).await.expect("init");
            // Found in order, each read from its EEPROM.
            let found: Vec<(String, u32, u32, u32)> = group
                .iter(&maindevice)
                .map(|s| {
                    let id = s.identity();
                    (s.name().to_string(), id.product_id, id.revision, id.serial)
                })
                .collect();
            assert_eq!(
                found,
                [("bench coupler".to_string(), 1, 1, 11), ("bench outputs".into(), 2, 3, 12), ("bench inputs".into(), 3, 1, 13)]
            );
            // Each given a configured address, which the segment now holds.
            let addresses: Vec<u16> = group.iter(&maindevice).map(|s| s.configured_address()).collect();
            assert_eq!(addresses, [segment.station("coupler").unwrap(), segment.station("out16").unwrap(), segment.station("in8").unwrap()]);
            assert_eq!(segment.al_state("out16").unwrap(), (2, false), "PRE-OP after init");
            let group = group.into_op(&maindevice).await.expect("PRE-OP to OP");
            for d in ["coupler", "out16", "in8"] {
                assert_eq!(segment.al_state(d).unwrap(), (8, false), "{d} in OP");
            }
            // Process data: outputs written by the master land in the device; inputs set on the
            // plant side reach the master.
            segment.set_inputs("in8", &[0xA5]).unwrap();
            let started = Instant::now();
            let mut seen = None;
            while started.elapsed() < Duration::from_secs(5) {
                group.tx_rx(&maindevice).await.expect("tx/rx");
                let out16 = group.iter(&maindevice).nth(1).unwrap();
                out16.outputs_raw_mut().copy_from_slice(&[0x12, 0x34]);
                let in8 = group.iter(&maindevice).nth(2).unwrap();
                if in8.inputs_raw()[0] == 0xA5 && segment.outputs("out16").unwrap() == [0x12, 0x34] {
                    seen = Some(started.elapsed());
                    break;
                }
            }
            assert!(seen.is_some(), "outputs {:?}, inputs on the master's side not 0xA5", segment.outputs("out16").unwrap());
            // A device faulted on the plant side: the master reads its error flag and its code (the
            // MainDevice reports them as the device's error); the others stay in OP.
            segment.fault("in8", 0x001B).unwrap();
            let e = group.iter(&maindevice).nth(2).unwrap().status().await.expect_err("the device's error");
            assert!(matches!(e, ethercrab::error::Error::SubDevice(ethercrab::AlStatusCode::SyncManagerWatchdog)), "{e:?}");
            assert_eq!(segment.al_state("in8").unwrap(), (4, true), "SAFE-OP with the error flag");
            assert_eq!(segment.al_state("out16").unwrap(), (8, false));
            // A silent segment: the master's exchange fails; spoken again, it goes through.
            segment.set_silent(true);
            assert!(group.tx_rx(&maindevice).await.is_err(), "no answer from a silent segment");
            segment.set_silent(false);
            group.tx_rx(&maindevice).await.expect("answered again");
        });
        assert!(segment.status().frames > 10);
    });
}

/// One datagram for a hand-built frame: command, address, data.
struct Dg(u8, u32, Vec<u8>);

/// A frame of `datagrams` from `src` (the test's end), broadcast as a master sends it.
fn frame(src: [u8; 6], datagrams: &[Dg]) -> Vec<u8> {
    let mut body = Vec::new();
    for (i, Dg(cmd, address, data)) in datagrams.iter().enumerate() {
        body.push(*cmd);
        body.push(i as u8);
        body.extend_from_slice(&address.to_le_bytes());
        let more = if i + 1 < datagrams.len() { 0x8000 } else { 0 };
        body.extend_from_slice(&((data.len() as u16) | more).to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(data);
        body.extend_from_slice(&[0, 0]);
    }
    let mut f = vec![0xFF; 6];
    f.extend_from_slice(&src);
    f.extend_from_slice(&0x88A4u16.to_be_bytes());
    f.extend_from_slice(&((body.len() as u16) | 0x1000).to_le_bytes());
    f.extend(body);
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

/// A returned frame: its source, and each datagram's data and working counter.
type Returned = ([u8; 6], Vec<(Vec<u8>, u16)>);

/// The datagrams of a returned frame: (data, working counter) each, and the frame's source.
fn answer(link: &veredas_aranha::wire::raw::RawLink, me: [u8; 6]) -> Option<Returned> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        let r = link.recv(Duration::from_millis(200)).unwrap()?;
        let f = r.bytes;
        if f.len() < 16 || f[12..14] != [0x88, 0xA4] || f[6..12] == me {
            continue;
        }
        let src: [u8; 6] = f[6..12].try_into().unwrap();
        let len = (u16::from_le_bytes([f[14], f[15]]) & 0x07FF) as usize;
        let body = &f[16..16 + len];
        let mut at = 0;
        let mut out = Vec::new();
        loop {
            let lf = u16::from_le_bytes([body[at + 6], body[at + 7]]);
            let n = (lf & 0x07FF) as usize;
            out.push((body[at + 10..at + 10 + n].to_vec(), u16::from_le_bytes([body[at + 10 + n], body[at + 11 + n]])));
            at += 12 + n;
            if lf & 0x8000 == 0 {
                break;
            }
        }
        return Some((src, out));
    }
    None
}

fn ap(position: u16, ado: u16) -> u32 {
    (u32::from(ado) << 16) | u32::from(0u16.wrapping_sub(position))
}

fn fp(station: u16, ado: u16) -> u32 {
    (u32::from(ado) << 16) | u32::from(station)
}

#[test]
fn every_command_reads_writes_and_counts_as_the_standard_has_it() {
    netns::isolated("every_command_reads_writes_and_counts_as_the_standard_has_it", &[("a0", "b0")], || {
        use veredas_aranha::wire::raw::{Options, RawLink};
        let segment = start(&config(SEGMENT)).unwrap();
        let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
        // A master's own address, its locally administered bit clear (the veth's is set), so the
        // bit on the way back is the segment's doing.
        let me = [0x10u8; 6];
        let ask = |dgs: &[Dg]| {
            link.send(&frame(me, dgs)).unwrap();
            answer(&link, me).expect("the frame came back")
        };
        // A broadcast read counts every device; the frame comes back with the source's locally
        // administered bit set.
        let (src, r) = ask(&[Dg(7, 0, vec![0; 1])]);
        assert_eq!((r[0].1, r[0].0[0]), (3, 0x11));
        assert_eq!(src, [0x12, 0x10, 0x10, 0x10, 0x10, 0x10]);
        // Auto-increment writes give each device its address, by position.
        let (_, r) = ask(&[Dg(2, ap(0, 0x10), vec![0x00, 0x10]), Dg(2, ap(1, 0x10), vec![0x01, 0x10]), Dg(2, ap(2, 0x10), vec![0x02, 0x10]), Dg(2, ap(3, 0x10), vec![0x03, 0x10])]);
        assert_eq!(r.iter().map(|x| x.1).collect::<Vec<_>>(), [1, 1, 1, 0], "a position past the last device: no one");
        assert_eq!((segment.station("coupler").unwrap(), segment.station("out16").unwrap(), segment.station("in8").unwrap()), (0x1000, 0x1001, 0x1002));
        // Configured-address reads, a read-write's counter 3, an address no device holds 0.
        let (_, r) = ask(&[Dg(4, fp(0x1002, 0x12), vec![0; 2]), Dg(4, fp(0x7777, 0x12), vec![0; 2]), Dg(6, fp(0x1001, 0x0E00), vec![0xAB, 0xCD])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![0x02, 0x01], 1), "in8's alias");
        assert_eq!(r[1].1, 0);
        assert_eq!((r[2].0.clone(), r[2].1), (vec![0, 0], 3), "the old bytes back, the new ones written");
        // The AL state machine: INIT to OP at once is refused, its error flagged and coded; PRE-OP
        // is taken.
        let (_, r) = ask(&[Dg(5, fp(0x1001, 0x0120), vec![0x08, 0x00])]);
        assert_eq!(r[0].1, 1);
        let (_, r) = ask(&[Dg(4, fp(0x1001, 0x0130), vec![0; 6])]);
        assert_eq!((r[0].0[0], u16::from_le_bytes([r[0].0[4], r[0].0[5]])), (0x11, 0x0011), "INIT with the error flag, invalid state change");
        assert_eq!(segment.al_state("out16").unwrap(), (1, true));
        let (_, _) = ask(&[Dg(5, fp(0x1001, 0x0120), vec![0x12, 0x00])]);
        assert_eq!(segment.al_state("out16").unwrap(), (2, false), "PRE-OP, the error acknowledged");
        // Logical: out16's outputs mapped at 0..2 (written), in8's input at 2 (read); one LRW
        // writes the outputs and reads the input, counted 2 for the write and 1 for the read.
        let fmmu = |logical: u32, len: u16, physical: u16, kind: u8| {
            let mut b = logical.to_le_bytes().to_vec();
            b.extend_from_slice(&len.to_le_bytes());
            b.extend_from_slice(&[0, 7]);
            b.extend_from_slice(&physical.to_le_bytes());
            b.extend_from_slice(&[0, kind, 1, 0, 0, 0]);
            b
        };
        ask(&[Dg(5, fp(0x1001, 0x0600), fmmu(0, 2, 0x1000, 2)), Dg(5, fp(0x1002, 0x0600), fmmu(2, 1, 0x1002, 1))]);
        segment.set_inputs("in8", &[0x5A]).unwrap();
        let (_, r) = ask(&[Dg(12, 0, vec![0x12, 0x34, 0x00])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![0x12, 0x34, 0x5A], 3));
        assert_eq!(segment.outputs("out16").unwrap(), [0x12, 0x34]);
        let (_, r) = ask(&[Dg(10, 0, vec![0; 3]), Dg(11, 0, vec![0x56, 0x78, 0xFF])]);
        assert_eq!((r[0].0.clone(), r[0].1, r[1].1), (vec![0, 0, 0x5A], 1, 2), "LRD reads only, LWR writes only");
        assert_eq!(segment.outputs("out16").unwrap(), [0x56, 0x78]);
        // The EEPROM through its registers: a read command loads 8 bytes at a word address.
        ask(&[Dg(5, fp(0x1001, 0x0502), vec![0x00, 0x01, 0x08, 0x00, 0x00, 0x00])]);
        let (_, r) = ask(&[Dg(4, fp(0x1001, 0x0502), vec![0; 2]), Dg(4, fp(0x1001, 0x0508), vec![0; 8])]);
        assert_eq!(r[0].0[1] & 0x80, 0, "not busy");
        assert_eq!(r[1].0, [0, 0, 0, 0, 2, 0, 0, 0], "vendor 0 and product 2, at words 8 to 11");
        // Distributed clocks: an ARMW of the system time, read by the first and written to the rest.
        let (_, r) = ask(&[Dg(13, ap(0, 0x0910), vec![0; 8])]);
        assert_eq!(r[0].1, 3);
        // Silent: nothing comes back; then it answers again.
        segment.set_silent(true);
        link.send(&frame(me, &[Dg(7, 0, vec![0])])).unwrap();
        assert!(answer(&link, me).is_none());
        segment.set_silent(false);
        let (_, r) = ask(&[Dg(7, 0, vec![0])]);
        assert_eq!(r[0].1, 3);
        let s = segment.status();
        assert!(s.frames >= 12 && s.dropped == 1, "{s:?}");
    });
}

#[test]
fn the_dissectors_read_the_returned_frame_as_the_standard_has_it() {
    netns::isolated("the_dissectors_read_the_returned_frame_as_the_standard_has_it", &[("a0", "b0")], || {
        use veredas_aranha::wire::raw::{Options, RawLink};
        let _segment = start(&config(SEGMENT)).unwrap();
        let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
        let me = [0x10u8; 6];
        let sent = frame(me, &[Dg(7, 0, vec![0; 2]), Dg(2, ap(1, 0x10), vec![0x01, 0x10]), Dg(4, fp(0x1001, 0x0130), vec![0; 2])]);
        link.send(&sent).unwrap();
        let started = Instant::now();
        let back = loop {
            assert!(started.elapsed() < Duration::from_secs(2), "the frame came back");
            if let Some(r) = link.recv(Duration::from_millis(200)).unwrap()
                && r.bytes.len() > 12
                && r.bytes[6..12] != me
            {
                break r.bytes;
            }
        };
        let tshark = [std::env::var_os("VEREDAS_TSHARK").map(std::path::PathBuf::from), std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/opt/wireshark/tshark"))]
            .into_iter()
            .flatten()
            .find(|c| std::process::Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false))
            .expect("the dissectors are this test's second reading: set VEREDAS_TSHARK or extract tshark into ~/.local/opt/wireshark");
        let mut pcap = 0xa1b2_c3d4u32.to_le_bytes().to_vec();
        pcap.extend_from_slice(&[2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 1, 0, 0, 0]);
        for (i, f) in [sent, back].iter().enumerate() {
            pcap.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
            pcap.extend_from_slice(&0u32.to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
            pcap.extend_from_slice(f);
        }
        let file = std::env::temp_dir().join(format!("veredas-ecat-{}.pcap", std::process::id()));
        std::fs::write(&file, pcap).unwrap();
        let out = std::process::Command::new(tshark).arg("-n").arg("-r").arg(&file).args(["-T", "fields", "-e", "ecat.cmd", "-e", "ecat.cnt", "-e", "_ws.malformed", "-e", "_ws.expert.severity", "-E", "occurrence=a", "-E", "aggregator=,"]).output().unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
        let lines: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect();
        // The frame as sent, every counter 0; the frame back, BRD counted by the three devices, the
        // APWR by the second, the FPRD by the device of that address; no complaint either way.
        assert_eq!(lines, ["0x07,0x02,0x04\t0,0,0\t\t", "0x07,0x02,0x04\t3,1,1\t\t"]);
    });
}
