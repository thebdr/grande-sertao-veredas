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

// One storage per test that runs a MainDevice: `cargo test` runs them in one process, and a
// storage splits once.
static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();
static PDU_STORAGE_BENCH: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();

const SEGMENT: &str = "interface = \"b0\"\ndevices = [\n\
  { name = \"coupler\", vendor_id = 0x0700, product_code = 1, revision = 1, serial = 11, product_name = \"bench coupler\" },\n\
  { name = \"out16\", vendor_id = 0x0701, product_code = 2, revision = 3, serial = 12, product_name = \"bench outputs\", outputs = [ { index = 0x7000, subindex = 1, bits = 8 }, { index = 0x7010, subindex = 1, bits = 8 } ] },\n\
  { name = \"in8\", vendor_id = 0x0702, product_code = 3, revision = 1, serial = 13, alias = 0x0102, product_name = \"bench inputs\", inputs = [ { index = 0x6000, subindex = 1, bits = 8 } ] },\n\
]\n";

/// A MainDevice on `a0`, its frames carried by a thread of its own.
fn maindevice(storage: &'static PduStorage<MAX_FRAMES, MAX_PDU_DATA>) -> Arc<MainDevice<'static>> {
    let (tx, rx, pdu_loop) = storage.try_split().expect("split once");
    let maindevice = Arc::new(MainDevice::new(
        pdu_loop,
        Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(5), pdu: Duration::from_millis(500), eeprom: Duration::from_millis(500), ..Default::default() },
        MainDeviceConfig::default(),
    ));
    std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
    maindevice
}

#[test]
fn a_maindevice_finds_the_segment_brings_it_to_op_and_exchanges_process_data() {
    netns::isolated("a_maindevice_finds_the_segment_brings_it_to_op_and_exchanges_process_data", &[("a0", "b0")], || {
        let segment = start(&config(SEGMENT)).unwrap();
        let maindevice = maindevice(&PDU_STORAGE);
        futures_lite::future::block_on(async {
            let group = maindevice.init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now).await.expect("init");
            // Found in order, each read from its EEPROM.
            let found: Vec<(String, u32, u32, u32, u32)> = group
                .iter(&maindevice)
                .map(|s| {
                    let id = s.identity();
                    (s.name().to_string(), id.vendor_id, id.product_id, id.revision, id.serial)
                })
                .collect();
            assert_eq!(
                found,
                [("bench coupler".to_string(), 0x0700, 1, 1, 11), ("bench outputs".into(), 0x0701, 2, 3, 12), ("bench inputs".into(), 0x0702, 3, 1, 13)]
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
        assert_eq!((r[0].0.clone(), r[0].1, r[1].1), (vec![0, 0, 0x5A], 1, 1), "LRD reads only, LWR writes only, each counted 1");
        assert_eq!(segment.outputs("out16").unwrap(), [0x56, 0x78]);
        // The EEPROM through its registers: a read command loads 8 bytes at a word address.
        ask(&[Dg(5, fp(0x1001, 0x0502), vec![0x00, 0x01, 0x08, 0x00, 0x00, 0x00])]);
        let (_, r) = ask(&[Dg(4, fp(0x1001, 0x0502), vec![0; 2]), Dg(4, fp(0x1001, 0x0508), vec![0; 8])]);
        assert_eq!(r[0].0[1] & 0x80, 0, "not busy");
        assert_eq!(r[1].0, [0x01, 0x07, 0, 0, 2, 0, 0, 0], "vendor 0x0701 and product 2, at words 8 to 11");
        // Distributed clocks: an ARMW of the system time, read by the first and written to the rest.
        let (_, r) = ask(&[Dg(13, ap(0, 0x0910), vec![0; 8])]);
        assert_eq!(r[0].1, 3);
        // A device faulted below SAFE-OP keeps its state, the error flagged and coded; the plant
        // side's names and sizes refused by name.
        assert_eq!(segment.al_state("coupler").unwrap(), (1, false));
        segment.fault("coupler", 0x001B).unwrap();
        assert_eq!(segment.al_state("coupler").unwrap(), (1, true), "never raised to SAFE-OP");
        assert_eq!(segment.al_state("out16").unwrap(), (2, false));
        segment.fault("out16", 0x001A).unwrap();
        assert_eq!(segment.al_state("out16").unwrap(), (2, true));
        let (_, r) = ask(&[Dg(4, fp(0x1001, 0x0134), vec![0; 2])]);
        assert_eq!(r[0].0, [0x1A, 0x00]);
        assert_eq!(segment.outputs("ghost").unwrap_err().to_string(), "no device `ghost`");
        assert_eq!(segment.set_inputs("in8", &[1, 2]).unwrap_err().to_string(), "device `in8` holds 1 byte(s) of inputs, not 2");
        assert_eq!(segment.al_state("ghost").unwrap_err().to_string(), "no device `ghost`");
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

#[test]
fn the_reference_s_segment_runs_from_its_bench_file_and_a_missing_interface_is_named() {
    netns::isolated("the_reference_s_segment_runs_from_its_bench_file_and_a_missing_interface_is_named", &[("a0", "b0")], || {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/bench-schema.md")).unwrap().replace("\r\n", "\n");
        let block = text.split("```toml\n").skip(1).map(|b| b[..b.find("```").unwrap()].to_string()).find(|b| b.contains("[[ethercat]]")).expect("the reference's EtherCAT example");
        let spec = veredas_maquinar::bench::load(&block, std::path::Path::new(".")).unwrap();
        assert_eq!(spec.plan(), ["ethercat `line`: on b0 with 3 device(s)"]);
        let plant = veredas_maquinar::bench::Bench::start(spec).unwrap();
        let line = plant.ethercat("line").expect("the part by its name");
        assert_eq!(line.interface(), "b0");
        let maindevice = maindevice(&PDU_STORAGE_BENCH);
        futures_lite::future::block_on(async {
            let group = maindevice.init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now).await.expect("init");
            let names: Vec<String> = group.iter(&maindevice).map(|s| s.name().to_string()).collect();
            assert_eq!(names, ["bench coupler", "bench outputs", "bench inputs"]);
            assert_eq!(group.iter(&maindevice).nth(2).unwrap().configured_address(), line.station("in8").unwrap());
            let group = group.into_op(&maindevice).await.expect("PRE-OP to OP");
            group.tx_rx(&maindevice).await.expect("tx/rx");
            assert_eq!(line.al_state("in8").unwrap(), (8, false));
        });
        // An interface the namespace lacks is named in the refusal.
        let missing = start(&config("interface = \"nowhere0\"\ndevices = [ { name = \"d\" } ]\n")).err().expect("no such interface");
        assert!(matches!(missing, EthercatError::Link(_)), "{missing}");
        assert_eq!(missing.to_string(), "the interface: no network interface is named \"nowhere0\"");
    });
}

#[test]
fn every_configuration_refusal_is_named_before_the_interface_opens() {
    let device = |extra: &str| format!("interface = \"b0\"\ndevices = [ {{ name = \"d\"{extra} }} ]\n");
    let entries = |n: usize, bits: u8| (0..n).map(|i| format!("{{ index = 0x7000, subindex = {}, bits = {bits} }}", i % 256)).collect::<Vec<_>>().join(", ");
    let cases: Vec<(String, String)> = vec![
        ("interface = \" \"\ndevices = [ { name = \"d\" } ]\n".into(), "ethercat.interface is empty".into()),
        ("interface = \"b0\"\n".into(), "ethercat.devices: a segment holds at least one device".into()),
        ("interface = \"b0\"\ndevices = [ { name = \"\" } ]\n".into(), "ethercat.devices[0]: a device's name is not empty and has no surrounding whitespace".into()),
        ("interface = \"b0\"\ndevices = [ { name = \"d \" } ]\n".into(), "ethercat.devices[0]: a device's name is not empty and has no surrounding whitespace".into()),
        ("interface = \"b0\"\ndevices = [ { name = \"d\" }, { name = \"d\" } ]\n".into(), "ethercat.devices[1]: two devices are named `d`".into()),
        (device(", product_name = \"\""), "ethercat.devices[0] (`d`).product_name: 1 to 255 ASCII characters".into()),
        (device(&format!(", product_name = \"{}\"", "x".repeat(256))), "ethercat.devices[0] (`d`).product_name: 1 to 255 ASCII characters".into()),
        (device(", product_name = \"bancada ç\""), "ethercat.devices[0] (`d`).product_name: 1 to 255 ASCII characters".into()),
        (device(&format!(", outputs = [ {} ]", entries(1, 0))), "ethercat.devices[0] (`d`).outputs[0]: 0 bit(s), not 1 to 64".into()),
        (device(&format!(", inputs = [ {} ]", entries(2, 65))), "ethercat.devices[0] (`d`).inputs[0]: 65 bit(s), not 1 to 64".into()),
        (device(&format!(", outputs = [ {} ]", entries(129, 64))), "ethercat.devices[0] (`d`).outputs: 1032 bytes, over 1024".into()),
        (device(&format!(", inputs = [ {} ]", entries(256, 1))), "ethercat.devices[0] (`d`).inputs: 256 entries, over 255".into()),
    ];
    for (text, message) in cases {
        let cfg = config(&text);
        assert_eq!(validate(&cfg).unwrap_err().to_string(), message, "{text}");
        // start refuses it the same way, before any interface is looked for.
        assert_eq!(start(&cfg).err().unwrap().to_string(), message);
    }
    // At the bounds: 255 entries, 1024 bytes, 64 bits, 255 characters.
    validate(&config(&device(&format!(", product_name = \"{}\", inputs = [ {} ], outputs = [ {} ]", "x".repeat(255), entries(255, 1), entries(128, 64))))).unwrap();
}

// ---- The ESC core alone, its clock handed in: exact to the nanosecond ----

use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity, PdoEntry};

/// Three devices whose local clocks started 0, 1 and 2 µs into the segment's time, at stations
/// 0x1000 to 0x1002, the last with alias 0x0102.
fn line() -> Vec<Esc> {
    (0..3u16)
        .map(|i| {
            let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: if i == 2 { 0x0102 } else { 0 } };
            let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
            e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
            e.mem[0x12..0x14].copy_from_slice(&id.alias.to_le_bytes());
            e.set_link(i < 2);
            e
        })
        .collect()
}

/// One frame's datagrams through the line at `now` nanoseconds: (data, counter, address) each.
fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
    let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
    esc::process(line, &mut d, now);
    d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
}

fn get(line: &[Esc], i: usize, at: u16, n: usize) -> Vec<u8> {
    line[i].read(at, n)
}

fn u32_at(line: &[Esc], i: usize, at: u16) -> u32 {
    u32::from_le_bytes(get(line, i, at, 4).try_into().unwrap())
}

#[test]
fn every_command_counts_and_addresses_as_the_standard_has_it() {
    let mut l = line();
    // Broadcast: a read ORs and counts every device, a write lands in each, a read-write counts
    // 3 a device and returns what they held, ORed.
    l[0].mem[0x0F80] = 0x01;
    l[2].mem[0x0F80] = 0x40;
    let r = pass(&mut l, 0, &[(7, 0x0F80 << 16, vec![0x80]), (8, 0x0F81 << 16, vec![0x5A])]);
    assert_eq!((r[0].0.clone(), r[0].1), (vec![0xC1], 3));
    assert_eq!(r[1].1, 3);
    assert!((0..3).all(|i| l[i].mem[0x0F81] == 0x5A));
    // A BRW: each device writes the data as it reaches it and ORs what it held into it.
    let r = pass(&mut l, 0, &[(9, 0x0F80 << 16, vec![0x22, 0x00])]);
    assert_eq!((r[0].0.clone(), r[0].1), (vec![0x63, 0x5A], 9));
    assert_eq!([l[0].read(0x0F80, 2), l[1].read(0x0F80, 2), l[2].read(0x0F80, 2)], [vec![0x22, 0x00], vec![0x23, 0x5A], vec![0x23, 0x5A]]);
    // Auto-increment: every device adds 1 to the position on the way, so the address comes back
    // three more than it left; a read-write counts 3 and returns the old bytes.
    let r = pass(&mut l, 0, &[(1, ap(1, 0x0F80), vec![0; 1]), (3, ap(2, 0x0F82), vec![0x77]), (2, ap(0, 0x0F83), vec![0x99])]);
    assert_eq!((r[0].1, r[0].2 as u16), (1, 0u16.wrapping_sub(1).wrapping_add(3)));
    assert_eq!((r[1].0.clone(), r[1].1, l[2].mem[0x0F82]), (vec![0x00], 3, 0x77));
    assert_eq!((r[2].1, l[0].mem[0x0F83], l[1].mem[0x0F83]), (1, 0x99, 0x00));
    // Configured: read, write, read-write; nobody at an address no device holds.
    let r = pass(&mut l, 0, &[(4, fp(0x1001, 0x0F82), vec![0]), (5, fp(0x1002, 0x0F84), vec![1]), (6, fp(0x1000, 0x0F84), vec![2]), (4, fp(0x4444, 0), vec![0])]);
    assert_eq!(r.iter().map(|x| x.1).collect::<Vec<_>>(), [1, 1, 3, 0]);
    // The type and kinds of a frame the segment does not answer: only type 1, commands.
    assert!(esc::parse(&[0x02, 0x20]).is_none());
}

/// An FMMU's sixteen bytes.
fn fmmu(logical: u32, len: u16, bits: (u8, u8), physical: u16, physical_bit: u8, kind: u8) -> Vec<u8> {
    let mut b = logical.to_le_bytes().to_vec();
    b.extend_from_slice(&len.to_le_bytes());
    b.extend_from_slice(&[bits.0, bits.1]);
    b.extend_from_slice(&physical.to_le_bytes());
    b.extend_from_slice(&[physical_bit, kind, 1, 0, 0, 0]);
    b
}

#[test]
fn fmmus_map_bit_by_bit_and_a_logical_write_counts_1_or_2_in_an_lrw() {
    let mut l = line();
    // Two devices, a 1-bit input each at physical bit 0 of 0x1000, mapped to bits 0 and 1 of one
    // logical byte: an LRD gathers both and leaves the byte's other bits as they came.
    l[0].mem[0x1000] = 0x01;
    l[1].mem[0x1000] = 0xFF;
    pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), fmmu(0, 1, (0, 0), 0x1000, 0, 1)), (5, fp(0x1001, 0x0600), fmmu(0, 1, (1, 1), 0x1000, 0, 1))]);
    let r = pass(&mut l, 0, &[(10, 0, vec![0xF0])]);
    assert_eq!((r[0].0.clone(), r[0].1), (vec![0xF3], 2));
    // A write of logical bit 1 lands on physical bit 0 alone, a byte holding 0xF0 becoming 0xF1,
    // and back to 0xF0 when the bit is 0: counted 1 a device.
    pass(&mut l, 0, &[(5, fp(0x1002, 0x0610), fmmu(0, 1, (1, 1), 0x1004, 0, 2))]);
    l[2].mem[0x1004] = 0xF0;
    let r = pass(&mut l, 0, &[(11, 0, vec![0x02])]);
    assert_eq!((l[2].mem[0x1004], r[0].1), (0xF1, 1));
    pass(&mut l, 0, &[(11, 0, vec![0xFD])]);
    assert_eq!(l[2].mem[0x1004], 0xF0);
    // A span across two logical bytes, bit 4 of the first to bit 3 of the second, onto a whole
    // physical byte.
    let mut l = line();
    l[0].mem[0x1000] = 0xAB;
    pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), fmmu(0x10, 2, (4, 3), 0x1000, 0, 1))]);
    let r = pass(&mut l, 0, &[(10, 0x10, vec![0, 0])]);
    assert_eq!((r[0].0.clone(), r[0].1), (vec![0xB0, 0x0A], 1));
    // An FMMU that reads and writes, under an LRW: the frame gets what the device held, the device
    // what the frame brought; counted 3.
    let mut l = line();
    l[1].mem[0x1100] = 0x11;
    pass(&mut l, 0, &[(5, fp(0x1001, 0x0600), fmmu(0x20, 1, (0, 7), 0x1100, 0, 3))]);
    let r = pass(&mut l, 0, &[(12, 0x20, vec![0xAA])]);
    assert_eq!((r[0].0.clone(), r[0].1, l[1].mem[0x1100]), (vec![0x11], 3, 0xAA));
    // An LWR through two devices counts 2, an LRW reading one and writing another 3.
    let mut l = line();
    pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), fmmu(0, 1, (0, 7), 0x1000, 0, 2)), (5, fp(0x1001, 0x0600), fmmu(1, 1, (0, 7), 0x1000, 0, 2)), (5, fp(0x1002, 0x0600), fmmu(2, 1, (0, 7), 0x1000, 0, 1))]);
    l[2].mem[0x1000] = 0x3C;
    let r = pass(&mut l, 0, &[(11, 0, vec![1, 2, 3]), (12, 0, vec![4, 5, 0])]);
    assert_eq!((r[0].1, r[1].1, r[1].0.clone()), (2, 5, vec![4, 5, 0x3C]));
    assert_eq!((l[0].mem[0x1000], l[1].mem[0x1000]), (4, 5));
}

#[test]
fn the_clock_latches_takes_its_offset_and_compares_in_sign_and_magnitude() {
    let mut l = line();
    // A broadcast write to port 0's receive time latches every device's ports at its local time,
    // port 1 40 ns later, the data written not kept; the processing unit's 64-bit time with them.
    let t = 10_000_000u64;
    let r = pass(&mut l, t, &[(8, 0x0900 << 16, vec![0xFF; 4])]);
    assert_eq!(r[0].1, 3);
    for (i, origin) in [0u64, 1_000, 2_000].iter().enumerate() {
        assert_eq!(u32_at(&l, i, 0x0900), (t - origin) as u32, "device {i} port 0");
        assert_eq!(u32_at(&l, i, 0x0904), (t - origin + 40) as u32, "device {i} port 1");
        assert_eq!(u64::from_le_bytes(get(&l, i, 0x0918, 8).try_into().unwrap()), t - origin);
    }
    // The first device given an offset of 1 ms: an ARMW reads its system time and hands it to the
    // others, each keeping its difference, behind by its own origin and the offset (bit 31).
    pass(&mut l, t, &[(5, fp(0x1000, 0x0920), 1_000_000u64.to_le_bytes().to_vec())]);
    let t2 = 20_000_000u64;
    let r = pass(&mut l, t2, &[(13, ap(0, 0x0910), vec![0; 8])]);
    assert_eq!((u64::from_le_bytes(r[0].0.clone().try_into().unwrap()), r[0].1), (t2 + 1_000_000, 3));
    assert_eq!(u32_at(&l, 1, 0x092C), 0x8000_0000 | 1_001_000);
    assert_eq!(u32_at(&l, 2, 0x092C), 0x8000_0000 | 1_002_000);
    assert_eq!(u32_at(&l, 0, 0x092C), 0, "the reference itself compares nothing");
    // The third device ahead by its own offset of 5 ms: bit 31 clear. An FRMW by the first's
    // station counts 3 too.
    pass(&mut l, t2, &[(5, fp(0x1002, 0x0920), 5_000_000u64.to_le_bytes().to_vec())]);
    let t3 = 30_000_000u64;
    let r = pass(&mut l, t3, &[(14, fp(0x1000, 0x0910), vec![0; 8])]);
    assert_eq!((u64::from_le_bytes(r[0].0.clone().try_into().unwrap()), r[0].1), (t3 + 1_000_000, 3));
    assert_eq!(u32_at(&l, 2, 0x092C), 3_998_000);
    // A write of the low 32 bits compares those, and the system time is read live, never as
    // written; the difference is EtherCAT's to read, not to write.
    let mine = (t3 - 1_000) as u32;
    pass(&mut l, t3, &[(5, fp(0x1001, 0x0910), (mine + 500).to_le_bytes().to_vec()), (5, fp(0x1001, 0x092C), vec![0; 4])]);
    assert_eq!(u32_at(&l, 1, 0x092C), 0x8000_0000 | 500);
    let r = pass(&mut l, t3 + 7, &[(4, fp(0x1001, 0x0910), vec![0; 8]), (4, fp(0x1002, 0x0910), vec![0; 8])]);
    assert_eq!(u64::from_le_bytes(r[0].0.clone().try_into().unwrap()), t3 + 7 - 1_000);
    assert_eq!(u64::from_le_bytes(r[1].0.clone().try_into().unwrap()), t3 + 7 - 2_000 + 5_000_000);
    // A difference past 31 bits is held at the most it can say.
    let mut l = line();
    pass(&mut l, 0, &[(5, fp(0x1000, 0x0920), (1u64 << 40).to_le_bytes().to_vec())]);
    pass(&mut l, 0, &[(13, ap(0, 0x0910), vec![0; 8])]);
    assert_eq!(u32_at(&l, 1, 0x092C), 0xFFFF_FFFF);
}

#[test]
fn registers_ethercat_only_reads_keep_their_values_and_the_ram_is_the_8_kb_stated() {
    let mut l = line();
    pass(&mut l, 5_000, &[(8, 0x0900 << 16, vec![0; 4])]);
    let before: Vec<Vec<u8>> = [(0x0000u16, 10usize), (0x0012, 2), (0x0110, 2), (0x0130, 2), (0x0134, 2), (0x0805, 1), (0x0807, 1), (0x083D, 1), (0x0904, 4), (0x0908, 8), (0x0918, 8), (0x092C, 4)].iter().map(|(a, n)| get(&l, 2, *a, *n)).collect();
    let writes: Vec<(u8, u32, Vec<u8>)> = [(0x0000u16, 10usize), (0x0012, 2), (0x0110, 2), (0x0130, 2), (0x0134, 2), (0x0805, 1), (0x0807, 1), (0x083D, 1), (0x0904, 4), (0x0908, 8), (0x0918, 8), (0x092C, 4)]
        .iter()
        .map(|(a, n)| (5u8, fp(0x1002, *a), vec![0xEE; *n]))
        .collect();
    let r = pass(&mut l, 9_000, &writes);
    assert!(r.iter().all(|x| x.1 == 1), "each write counted");
    let after: Vec<Vec<u8>> = [(0x0000u16, 10usize), (0x0012, 2), (0x0110, 2), (0x0130, 2), (0x0134, 2), (0x0805, 1), (0x0807, 1), (0x083D, 1), (0x0904, 4), (0x0908, 8), (0x0918, 8), (0x092C, 4)].iter().map(|(a, n)| get(&l, 2, *a, *n)).collect();
    assert_eq!(after, before);
    assert_eq!((before[0][0], before[1].clone(), before[3][0]), (0x11, vec![0x02, 0x01], 0x01));
    // The station address and the offset are written.
    pass(&mut l, 9_000, &[(5, fp(0x1002, 0x0010), vec![0x34, 0x12]), (5, fp(0x1234, 0x0920), vec![9; 8])]);
    assert_eq!((l[2].station(), get(&l, 2, 0x0920, 8)), (0x1234, vec![9; 8]));
    // 8 KB of process RAM, as register 0x0006 states: 0x1000 to 0x2FFF, kept.
    let r = pass(&mut l, 0, &[(4, fp(0x1234, 0x0006), vec![0]), (5, fp(0x1234, 0x2100), vec![0xAB]), (5, fp(0x1234, 0x2FFE), vec![0xCD, 0xEF])]);
    assert_eq!(r[0].0, [8]);
    let r = pass(&mut l, 0, &[(4, fp(0x1234, 0x2100), vec![0]), (4, fp(0x1234, 0x2FFE), vec![0, 0])]);
    assert_eq!((r[0].0.clone(), r[1].0.clone()), (vec![0xAB], vec![0xCD, 0xEF]));
}

#[test]
fn the_al_state_machine_holds_a_refusal_until_it_is_acknowledged() {
    let mut l = line();
    let state = |l: &[Esc]| (l[1].mem[0x0130], u16::from_le_bytes([l[1].mem[0x0134], l[1].mem[0x0135]]));
    let ask = |l: &mut [Esc], control: u8| pass(l, 0, &[(5, fp(0x1001, 0x0120), vec![control, 0])]);
    ask(&mut l, 0x08);
    assert_eq!(state(&l), (0x11, 0x0011), "INIT to OP refused");
    ask(&mut l, 0x02);
    assert_eq!(state(&l), (0x11, 0x0011), "a request without the acknowledge is not taken");
    ask(&mut l, 0x15);
    assert_eq!(state(&l), (0x11, 0x0012), "a state that does not exist, acknowledged: refused as unknown");
    ask(&mut l, 0x12);
    assert_eq!(state(&l), (0x02, 0), "PRE-OP, acknowledged");
    for (control, now) in [(0x04, 0x04), (0x08, 0x08), (0x02, 0x02), (0x04, 0x04), (0x08, 0x08), (0x01, 0x01)] {
        ask(&mut l, control);
        assert_eq!(state(&l), (now, 0));
    }
    // INIT clears an error without the acknowledge.
    ask(&mut l, 0x04);
    assert_eq!(state(&l), (0x11, 0x0011));
    ask(&mut l, 0x01);
    assert_eq!(state(&l), (0x01, 0));
}

#[test]
fn the_alias_addresses_a_device_once_the_master_enables_it() {
    let mut l = line();
    let r = pass(&mut l, 0, &[(4, fp(0x0102, 0x0010), vec![0; 2])]);
    assert_eq!(r[0].1, 0, "not by its alias while DL control's bit 24 is clear");
    pass(&mut l, 0, &[(5, fp(0x1002, 0x0100), vec![0, 0, 0, 1])]);
    let r = pass(&mut l, 0, &[(4, fp(0x0102, 0x0010), vec![0; 2]), (4, fp(0x1002, 0x0010), vec![0; 2])]);
    assert_eq!((r[0].0.clone(), r[0].1, r[1].1), (vec![0x02, 0x10], 1, 1), "by its alias and by its address");
}

#[test]
fn the_eeprom_states_its_port_and_its_size() {
    let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0 };
    let general = |w: &[u16]| {
        let mut at = 0x40;
        while w[at] != 30 {
            at += 2 + w[at + 1] as usize;
        }
        w[at + 2..at + 2 + w[at + 1] as usize].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
    };
    let small = esc::sii_image(&id, &[], &[]);
    let g = general(&small);
    assert_eq!((g[4], g[0x10], g[0x11]), (0, 0x11, 0), "ports 0 and 1 MII, at the physical-port bytes");
    assert_eq!(small[0x3E], 15, "16 Kbit");
    let many: Vec<PdoEntry> = (0..255).map(|i| PdoEntry { index: 0x7000, subindex: i as u8, bits: 32 }).collect();
    let big = esc::sii_image(&id, &many, &many);
    let kbit = big[0x3E] as usize + 1;
    assert!(kbit.is_power_of_two() && kbit * 128 >= big.len() * 2 && kbit * 64 < big.len() * 2, "{kbit} Kbit for {} bytes", big.len() * 2);
}

/// C-113's second refute round: the critic's probes, kept as pins (a frame of another type, the
/// state machine's refusals from every state and BOOT never taken, the physical start bit, the
/// EEPROM's bytes other masters read, every SyncManager's read-only bytes, the clock's difference
/// ahead held, a broadcast read of the system time, a broadcast's position field).
mod round_2 {
    use std::time::{Duration, Instant};

    use veredas_aranha::netns;
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity, PdoEntry};
    use veredas_maquinar::ethercat::*;

    const SEGMENT: &str = "interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\" } ]\n";

    fn line() -> Vec<Esc> {
        (0..3u16)
            .map(|i| {
                let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: 0 };
                let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.set_link(i < 2);
                e
            })
            .collect()
    }

    fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
        let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    fn ap(position: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(0u16.wrapping_sub(position))
    }

    fn fmmu(logical: u32, len: u16, bits: (u8, u8), physical: u16, physical_bit: u8, kind: u8) -> Vec<u8> {
        let mut b = logical.to_le_bytes().to_vec();
        b.extend_from_slice(&len.to_le_bytes());
        b.extend_from_slice(&[bits.0, bits.1]);
        b.extend_from_slice(&physical.to_le_bytes());
        b.extend_from_slice(&[physical_bit, kind, 1, 0, 0, 0]);
        b
    }

    /// P1. "each frame of commands (type 1; any other unanswered, counted unreadable)": a frame of
    /// type 4 carrying a well-formed APRD goes unanswered and is counted unreadable.
    #[test]
    fn a_frame_of_another_type_is_unanswered_and_counted_unreadable() {
        netns::isolated("round_2::a_frame_of_another_type_is_unanswered_and_counted_unreadable", &[("a0", "b0")], || {
            use veredas_aranha::wire::raw::{Options, RawLink};
            let cfg: EthercatConfig = toml::from_str(SEGMENT).unwrap();
            let segment = start(&cfg).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            let me = [0x10u8; 6];
            let frame = |kind: u16| {
                let mut body = vec![1u8, 0]; // APRD, idx 0
                body.extend_from_slice(&ap(0, 0x0000).to_le_bytes());
                body.extend_from_slice(&1u16.to_le_bytes());
                body.extend_from_slice(&[0, 0, 0, 0, 0]);
                let mut f = vec![0xFF; 6];
                f.extend_from_slice(&me);
                f.extend_from_slice(&0x88A4u16.to_be_bytes());
                f.extend_from_slice(&((body.len() as u16) | (kind << 12)).to_le_bytes());
                f.extend(body);
                f.resize(60, 0);
                f
            };
            let back = |link: &RawLink| {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(1500) {
                    if let Some(r) = link.recv(Duration::from_millis(100)).unwrap()
                        && r.bytes.len() > 12
                        && r.bytes[6..12] != me
                    {
                        return Some(r.bytes);
                    }
                }
                None
            };
            link.send(&frame(4)).unwrap();
            let got = back(&link);
            let s = segment.status();
            assert!(got.is_none(), "a frame of type 4 was answered: {got:02x?}");
            assert_eq!((s.frames, s.unreadable), (0, 1), "{s:?}");
            link.send(&frame(1)).unwrap();
            assert!(back(&link).is_some(), "type 1 answered");
            // The core itself: a whole payload of type 2 is not read; the same as type 1 is.
            let mut payload = (12u16 | (2 << 12)).to_le_bytes().to_vec();
            payload.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert!(esc::parse(&payload).is_none());
            payload[1] = (payload[1] & 0x0F) | 0x10;
            assert!(esc::parse(&payload).is_some(), "the same, type 1");
        });
    }

    /// P2. "a transition it refuses flagged with code 0x0011": ETG.1000.6's state machine refuses
    /// PRE-OP to OP, and BOOT is reached from INIT alone and left to INIT alone.
    #[test]
    fn every_transition_etg_refuses_is_refused() {
        let state = |l: &[Esc]| (l[1].mem[0x0130], u16::from_le_bytes([l[1].mem[0x0134], l[1].mem[0x0135]]));
        let ask = |l: &mut [Esc], control: u8| {
            pass(l, 0, &[(5, fp(0x1001, 0x0120), vec![control, 0])]);
        };
        let to = |l: &mut [Esc], path: &[u8]| {
            for c in path {
                ask(l, *c);
            }
        };
        // PRE-OP to OP
        let mut l = line();
        to(&mut l, &[0x01, 0x02]);
        ask(&mut l, 0x08);
        assert_eq!(state(&l), (0x12, 0x0011), "PRE-OP to OP");
        // PRE-OP, SAFE-OP and OP to BOOT
        for (path, from) in [(&[0x01u8, 0x02][..], 0x02u8), (&[0x01, 0x02, 0x04][..], 0x04), (&[0x01, 0x02, 0x04, 0x08][..], 0x08)] {
            let mut l = line();
            to(&mut l, path);
            ask(&mut l, 0x03);
            assert_eq!(state(&l), (from | 0x10, 0x0011), "{from:#x} to BOOT");
        }
        // INIT to BOOT refused: the EEPROM states no bootstrap mailbox (0x0013, bootstrap not
        // supported); INIT clears it.
        let mut l = line();
        ask(&mut l, 0x13);
        assert_eq!(state(&l), (0x11, 0x0013), "INIT to BOOT");
        ask(&mut l, 0x01);
        assert_eq!(state(&l), (0x01, 0));
    }

    /// P3. "logical through its FMMUs bit by bit": the physical start bit (0x060A) moves the mapped
    /// bit within its physical byte, for a read and for a write.
    #[test]
    fn the_physical_start_bit_is_honoured() {
        let mut l = line();
        l[0].mem[0x1000] = 0x08;
        pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), fmmu(0, 1, (0, 0), 0x1000, 3, 1)), (5, fp(0x1000, 0x0610), fmmu(1, 1, (0, 0), 0x1004, 5, 2))]);
        let r = pass(&mut l, 0, &[(10, 0, vec![0x00])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![0x01], 1), "logical bit 0 read from physical bit 3");
        pass(&mut l, 0, &[(11, 1, vec![0x01])]);
        assert_eq!(l[0].mem[0x1004], 0x20, "logical bit 8 written to physical bit 5");
    }

    /// P4. "its SII EEPROM ... (identity, name, General ..., SyncManager ... categories built from the
    /// file)": the bytes other masters than ethercrab read. ETG.2010: word 4 the alias; General byte 3
    /// the NameIdx (IgH's name; ethercrab reads the OrderIdx); a SyncManager's control byte bits 2-3
    /// the direction (01 written by the master: outputs; 00 read: inputs), which IgH takes a PDO's
    /// direction from.
    #[test]
    fn the_sii_bytes_other_masters_read() {
        let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0x0102 };
        let e = |bits| vec![PdoEntry { index: 0x7000, subindex: 1, bits }];
        let w = esc::sii_image(&id, &e(8), &e(8));
        let category = |kind: u16| {
            let mut at = 0x40;
            while w[at] != kind {
                assert_ne!(w[at], 0xFFFF, "category {kind}");
                at += 2 + w[at + 1] as usize;
            }
            w[at + 2..at + 2 + w[at + 1] as usize].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
        };
        assert_eq!(w[4], 0x0102, "the alias in the EEPROM's word 4");
        let g = category(30);
        assert_eq!((g[2], g[3]), (1, 1), "OrderIdx and NameIdx both the name");
        let sm = category(41);
        assert_eq!((sm[16 + 4] & 0x0C, sm[16 + 7]), (0x04, 3), "SM2: written by the master, outputs");
        assert_eq!((sm[24 + 4] & 0x0C, sm[24 + 7]), (0x00, 4), "SM3: read by the master, inputs");
    }

    /// P5. "each SyncManager's status and PDI control" kept: all eight, not three of sixteen.
    #[test]
    fn every_syncmanager_s_status_and_pdi_control_is_kept() {
        let mut l = line();
        for n in 0..8u16 {
            for at in [0x0805 + 8 * n, 0x0807 + 8 * n] {
                pass(&mut l, 0, &[(5, fp(0x1001, at), vec![0xEE])]);
                assert_eq!(l[1].mem[at as usize], 0, "{at:#06x}");
            }
        }
    }

    /// P6. "held to 31 bits" both ways: a device ahead by more than 2^31 ns reads 0x7FFFFFFF.
    #[test]
    fn a_difference_ahead_over_31_bits_is_held_at_its_most() {
        let mut l = line();
        pass(&mut l, 0, &[(5, fp(0x1002, 0x0920), (1u64 << 40).to_le_bytes().to_vec())]);
        pass(&mut l, 0, &[(13, ap(0, 0x0910), vec![0; 8])]);
        assert_eq!(u32::from_le_bytes(l[2].read(0x092C, 4).try_into().unwrap()), 0x7FFF_FFFF);
    }

    /// P7. "system time live": read live by a broadcast read as by a configured one.
    #[test]
    fn a_brd_reads_the_system_time_live() {
        let mut l = line();
        let r = pass(&mut l[..1], 5_000_000, &[(7, 0x0910 << 16, vec![0; 8])]);
        assert_eq!(u64::from_le_bytes(r[0].0.clone().try_into().unwrap()), 5_000_000);
    }

    /// P8. Outside the clause's words: the ESC datasheet's addressing table says each device
    /// increments the position field of a broadcast too ("not used for addressing").
    #[test]
    fn a_broadcast_s_position_field_comes_back_incremented() {
        let mut l = line();
        let r = pass(&mut l, 0, &[(7, 0, vec![0]), (8, 0x0F80 << 16, vec![0])]);
        assert_eq!((r[0].2 as u16, r[1].2 as u16), (3, 3));
        let r = pass(&mut l, 0, &[(9, 0x0F80 << 16, vec![0])]);
        assert_eq!(r[0].2 as u16, 3, "a BRW's too");
    }
}

/// C-113's third refute round: the critic's probes (`~/.cache/veredas/critic-c113-r3/probes`), kept
/// as pins: a 32-bit system time compared across its wrap, by a write and by an FRMW; a write and an
/// FMMU running past the RAM's end keeping the bytes inside it; an EEPROM write or reload done with
/// no effect; the IRQ field never set; the EEPROM's FMMU usage and PDO entries; a silent master
/// leaving its devices in OP; a fault in SAFE-OP keeping SAFE-OP.
mod round_3 {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ethercrab::std::ethercat_now;
    use ethercrab::{MainDevice, MainDeviceConfig, PduStorage, Timeouts};
    use veredas_aranha::netns;
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity, PdoEntry};
    use veredas_maquinar::ethercat::*;

    fn line() -> Vec<Esc> {
        (0..3u16)
            .map(|i| {
                let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: 0 };
                let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.set_link(i < 2);
                e
            })
            .collect()
    }

    fn dgs(dgs: &[(u8, u32, Vec<u8>)]) -> Vec<Datagram> {
        dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect()
    }

    fn pass(line: &mut [Esc], now: u64, d: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
        let mut d = dgs(d);
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    fn diff(l: &[Esc], i: usize) -> u32 {
        u32::from_le_bytes(l[i].read(0x092C, 4).try_into().unwrap())
    }

    fn offset(l: &mut [Esc], station: u16, v: u64) {
        pass(l, 0, &[(5, fp(station, 0x0920), v.to_le_bytes().to_vec())]);
    }

    #[test]
    fn a_low_32_bit_system_time_is_compared_across_its_wrap() {
        // The device at 0x1_0000_0100, the master's low half 0xFFFF_FF00: ahead by 512 across the wrap.
        let now = 10_000u64;
        let mut l = line();
        offset(&mut l, 0x1000, 0x1_0000_0100 - now);
        pass(&mut l, now, &[(5, fp(0x1000, 0x0910), 0xFFFF_FF00u32.to_le_bytes().to_vec())]);
        assert_eq!(diff(&l, 0), 0x0000_0200);
        // The device at 0xFFFF_FF00, the master's 0x0000_0100: behind by 512.
        let mut l = line();
        offset(&mut l, 0x1000, 0xFFFF_FF00 - now);
        pass(&mut l, now, &[(5, fp(0x1000, 0x0910), 0x0000_0100u32.to_le_bytes().to_vec())]);
        assert_eq!(diff(&l, 0), 0x8000_0200);
        // Halfway round and past it, held to 31 bits the short way.
        let mut l = line();
        offset(&mut l, 0x1000, 0x8000_0000 - now);
        pass(&mut l, now, &[(5, fp(0x1000, 0x0910), 0u32.to_le_bytes().to_vec())]);
        assert_eq!(diff(&l, 0), 0xFFFF_FFFF, "2^31 behind or ahead: the most behind there is");
    }

    #[test]
    fn an_frmw_of_32_bits_compares_across_the_wrap_as_one_of_64_does() {
        // The reference at 0xFFFF_FF80, the next device 256 ns ahead of it across the wrap, the
        // last 256 ns behind.
        let now = 100_000u64;
        let set = |l: &mut [Esc]| {
            offset(l, 0x1000, 0xFFFF_FF80 - now);
            offset(l, 0x1001, 0x1_0000_0080 - (now - 1_000));
            offset(l, 0x1002, 0xFFFF_FE80 - (now - 2_000));
        };
        for bytes in [8, 4] {
            let mut l = line();
            set(&mut l);
            let r = pass(&mut l, now, &[(14, fp(0x1000, 0x0910), vec![0; bytes])]);
            assert_eq!((r[0].1, &r[0].0[..4]), (3, &0xFFFF_FF80u32.to_le_bytes()[..]), "{bytes} bytes");
            assert_eq!((diff(&l, 1), diff(&l, 2)), (0x0000_0100, 0x8000_0100), "{bytes} bytes");
        }
    }

    #[test]
    fn a_write_running_past_the_ram_s_end_keeps_the_bytes_inside_it() {
        let mut l = line();
        let r = pass(&mut l, 0, &[(5, fp(0x1000, 0x2FFE), vec![1, 2, 3, 4])]);
        assert_eq!((r[0].1, l[0].read(0x2FFE, 2)), (1, vec![1, 2]));
        // Read back the same way: the bytes inside, zeros past the end.
        let r = pass(&mut l, 0, &[(4, fp(0x1000, 0x2FFE), vec![0; 4])]);
        assert_eq!(r[0].0, [1, 2, 0, 0]);
    }

    #[test]
    fn an_fmmu_running_past_the_ram_s_end_writes_the_byte_inside_it() {
        let mut l = line();
        // FMMU 0: logical 0, 2 bytes, bits 0 to 7, physical 0x2FFF, write.
        let mut f = 0u32.to_le_bytes().to_vec();
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&[0, 7]);
        f.extend_from_slice(&0x2FFFu16.to_le_bytes());
        f.extend_from_slice(&[0, 2, 1, 0, 0, 0]);
        pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), f)]);
        let r = pass(&mut l, 0, &[(11, 0, vec![0xAB, 0xCD])]);
        assert_eq!((r[0].1, l[0].read(0x2FFF, 1)), (1, vec![0xAB]));
    }

    #[test]
    fn an_eeprom_write_or_reload_is_answered_as_done_with_no_effect() {
        let mut l = line();
        let read8 = |l: &mut [Esc]| {
            pass(l, 0, &[(5, fp(0x1001, 0x0502), vec![0x00, 0x01, 0x08, 0x00, 0x00, 0x00])]);
            pass(l, 0, &[(4, fp(0x1001, 0x0502), vec![0; 2]), (4, fp(0x1001, 0x0508), vec![0; 8])])
        };
        let before = read8(&mut l);
        // A write of word 8 (write enable, write command, the address, the data), then a reload.
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0502), vec![0x01, 0x02, 0x08, 0x00, 0x00, 0x00, 0x34, 0x12])]);
        let r = pass(&mut l, 0, &[(4, fp(0x1001, 0x0502), vec![0; 2])]);
        assert_eq!(u16::from_le_bytes([r[0].0[0], r[0].0[1]]) & 0xF800, 0, "write: done, not busy, no error");
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0502), vec![0x00, 0x04])]);
        let r = pass(&mut l, 0, &[(4, fp(0x1001, 0x0502), vec![0; 2])]);
        assert_eq!(u16::from_le_bytes([r[0].0[0], r[0].0[1]]) & 0xF800, 0, "reload: done, not busy, no error");
        assert_eq!(read8(&mut l), before, "the EEPROM's words unchanged");
    }

    #[test]
    fn the_irq_field_is_never_set() {
        let mut l = line();
        let mut d = dgs(&[(7, 0, vec![0]), (4, fp(0x1001, 0x0130), vec![0; 2]), (12, 0, vec![0])]);
        d[1].irq = 0x1234;
        esc::process(&mut l, &mut d, 0);
        assert_eq!(d.iter().map(|x| x.irq).collect::<Vec<_>>(), [0, 0x1234, 0], "as each went");
    }

    #[test]
    fn the_eeprom_s_fmmu_usage_and_pdo_entries_are_the_file_s() {
        let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0 };
        let w = esc::sii_image(&id, &[PdoEntry { index: 0x7010, subindex: 3, bits: 8 }], &[PdoEntry { index: 0x6020, subindex: 5, bits: 16 }]);
        let category = |kind: u16| {
            let mut at = 0x40;
            while w[at] != kind {
                assert_ne!(w[at], 0xFFFF, "category {kind}");
                at += 2 + w[at + 1] as usize;
            }
            w[at + 2..at + 2 + w[at + 1] as usize].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
        };
        assert_eq!(&category(40)[..2], [1, 2], "FMMU0 outputs, FMMU1 inputs");
        let tx = category(50);
        assert_eq!((u16::from_le_bytes([tx[0], tx[1]]), tx[2], tx[3]), (0x1A00, 1, 3), "TxPDO 0x1A00: one entry, SyncManager 3");
        assert_eq!((u16::from_le_bytes([tx[8], tx[9]]), tx[10], tx[13]), (0x6020, 5, 16), "the input entry");
        let rx = category(51);
        assert_eq!((u16::from_le_bytes([rx[0], rx[1]]), rx[2], rx[3]), (0x1600, 1, 2), "RxPDO 0x1600: one entry, SyncManager 2");
        assert_eq!((u16::from_le_bytes([rx[8], rx[9]]), rx[10], rx[13]), (0x7010, 3, 8), "the output entry");
    }

    const SEGMENT: &str = "interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\", outputs = [ { index = 0x7000, subindex = 1, bits = 8 } ] } ]\n";
    const MAX_PDU_DATA: usize = PduStorage::element_size(1100);
    static PDU_STORAGE: PduStorage<16, MAX_PDU_DATA> = PduStorage::new();

    #[test]
    fn a_silent_master_leaves_its_devices_in_op() {
        // No SyncManager watchdog: 300 ms without process data (a real ESC's default watchdog is
        // 100 ms), every device still in OP and its exchange answered.
        netns::isolated("round_3::a_silent_master_leaves_its_devices_in_op", &[("a0", "b0")], || {
            let segment = start(&toml::from_str(SEGMENT).unwrap()).unwrap();
            let (tx, rx, pdu_loop) = PDU_STORAGE.try_split().expect("split once");
            let maindevice = Arc::new(MainDevice::new(
                pdu_loop,
                Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(5), pdu: Duration::from_millis(500), eeprom: Duration::from_millis(500), ..Default::default() },
                MainDeviceConfig::default(),
            ));
            std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
            futures_lite::future::block_on(async {
                let group = maindevice.init_single_group::<8, 16>(ethercat_now).await.expect("init");
                let group = group.into_op(&maindevice).await.expect("OP");
                for _ in 0..20 {
                    group.tx_rx(&maindevice).await.expect("tx/rx");
                }
                std::thread::sleep(Duration::from_millis(300));
                assert_eq!((segment.al_state("d0").unwrap(), segment.al_state("d1").unwrap()), ((8, false), (8, false)), "still in OP");
                let r = group.tx_rx(&maindevice).await.expect("tx/rx after the silence");
                assert!(r.all_op(), "{r:?}");
            });
        });
    }

    #[test]
    fn a_device_faulted_in_safe_op_keeps_safe_op_flagged() {
        netns::isolated("round_3::a_device_faulted_in_safe_op_keeps_safe_op_flagged", &[("a0", "b0")], || {
            use veredas_aranha::wire::raw::{Options, RawLink};
            let segment = start(&toml::from_str(SEGMENT).unwrap()).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            let me = [0x10u8; 6];
            // One APWR, answered before the next is sent.
            let send = |cmd: u8, address: u32, data: &[u8]| {
                let mut body = vec![cmd, 0];
                body.extend_from_slice(&address.to_le_bytes());
                body.extend_from_slice(&(data.len() as u16).to_le_bytes());
                body.extend_from_slice(&[0, 0]);
                body.extend_from_slice(data);
                body.extend_from_slice(&[0, 0]);
                let mut f = vec![0xFF; 6];
                f.extend_from_slice(&me);
                f.extend_from_slice(&0x88A4u16.to_be_bytes());
                f.extend_from_slice(&((body.len() as u16) | 0x1000).to_le_bytes());
                f.extend(body);
                f.resize(60, 0);
                link.send(&f).unwrap();
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(2) {
                    if let Some(r) = link.recv(Duration::from_millis(100)).unwrap()
                        && r.bytes.len() > 12
                        && r.bytes[6..12] != me
                    {
                        return;
                    }
                }
                panic!("no answer");
            };
            // d1 (position 1) to PRE-OP, then SAFE-OP; faulted there, it stays, flagged.
            let ap1 = (0x0120u32 << 16) | u32::from(0u16.wrapping_sub(1));
            send(2, ap1, &[0x02, 0]);
            send(2, ap1, &[0x04, 0]);
            assert_eq!(segment.al_state("d1").unwrap(), (4, false));
            segment.fault("d1", 0x001A).unwrap();
            assert_eq!(segment.al_state("d1").unwrap(), (4, true));
        });
    }
}

/// C-113's fourth refute round: the critic's probes (`~/.cache/veredas/critic-c113-r4/probes`), kept
/// as pins: a working counter by device, not by FMMU; an FMMU's activation; a datagram starting or
/// ending inside an FMMU, and ethercrab's split image at the 1024-byte bound; the downward
/// transitions and an acknowledge in the device's own state; the system time compared when its
/// first byte is written.
mod round_4 {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ethercrab::std::ethercat_now;
    use ethercrab::{MainDevice, MainDeviceConfig, PduStorage, Timeouts};
    use veredas_aranha::netns;
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity};
    use veredas_maquinar::ethercat::*;

    fn line() -> Vec<Esc> {
        (0..3u16)
            .map(|i| {
                let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: 0 };
                let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.set_link(i < 2);
                e
            })
            .collect()
    }

    fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
        let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    #[allow(clippy::too_many_arguments)]
    fn fmmu(logical: u32, len: u16, bits: (u8, u8), physical: u16, physical_bit: u8, kind: u8, enable: u8) -> Vec<u8> {
        let mut b = logical.to_le_bytes().to_vec();
        b.extend_from_slice(&len.to_le_bytes());
        b.extend_from_slice(&[bits.0, bits.1]);
        b.extend_from_slice(&physical.to_le_bytes());
        b.extend_from_slice(&[physical_bit, kind, enable, 0, 0, 0]);
        b
    }

    /// P1. "the working counter the standard gives each command ... logical through its FMMUs ... a
    /// read counted 1, a write 1, ... a logical write 1 and an LRW's write 2": counted a device, not an
    /// FMMU (ETG.1000.4; the ESC datasheet: incremented once if at least one byte was read/written).
    /// A device whose outputs sit in two FMMUs (two output SyncManagers, or a split image) counts once.
    #[test]
    fn a_device_with_two_fmmus_of_one_direction_counts_once() {
        let mut l = line();
        pass(
            &mut l,
            0,
            &[
                (5, fp(0x1000, 0x0600), fmmu(0, 1, (0, 7), 0x1000, 0, 2, 1)),
                (5, fp(0x1000, 0x0610), fmmu(1, 1, (0, 7), 0x1001, 0, 2, 1)),
                (5, fp(0x1000, 0x0620), fmmu(2, 1, (0, 7), 0x1100, 0, 1, 1)),
                (5, fp(0x1000, 0x0630), fmmu(3, 1, (0, 7), 0x1101, 0, 1, 1)),
            ],
        );
        l[0].mem[0x1100] = 0x5A;
        l[0].mem[0x1101] = 0xA5;
        let r = pass(&mut l, 0, &[(11, 0, vec![0x11, 0x22, 0, 0]), (10, 0, vec![0; 4]), (12, 0, vec![0x33, 0x44, 0, 0])]);
        assert_eq!(r[0].1, 1, "LWR through two write FMMUs of one device: 1");
        assert_eq!((r[1].0.clone(), r[1].1), (vec![0, 0, 0x5A, 0xA5], 1), "LRD through two read FMMUs of one device: 1");
        assert_eq!((r[2].0.clone(), r[2].1), (vec![0x33, 0x44, 0x5A, 0xA5], 3), "LRW: 2 for the write and 1 for the read, once each");
        assert_eq!(l[0].read(0x1000, 2), [0x33, 0x44]);
    }

    /// P2. "logical through its FMMUs": an FMMU maps only while activated (0x060C bit 0). A master that
    /// deactivates an FMMU by its activation byte alone leaves its other bytes set.
    #[test]
    fn a_deactivated_fmmu_maps_nothing() {
        let mut l = line();
        l[0].mem[0x1100] = 0x5A;
        pass(&mut l, 0, &[(5, fp(0x1000, 0x0600), fmmu(0, 1, (0, 7), 0x1000, 0, 2, 0)), (5, fp(0x1000, 0x0610), fmmu(1, 1, (0, 7), 0x1100, 0, 1, 0))]);
        let r = pass(&mut l, 0, &[(12, 0, vec![0x77, 0x00])]);
        assert_eq!((r[0].0.clone(), r[0].1, l[0].mem[0x1000]), (vec![0x77, 0x00], 0, 0x00), "never activated: nothing read, written or counted");
        // Activated, then deactivated by its activation byte alone.
        pass(&mut l, 0, &[(5, fp(0x1000, 0x060C), vec![1]), (5, fp(0x1000, 0x061C), vec![1])]);
        let r = pass(&mut l, 0, &[(12, 0, vec![0x77, 0x00])]);
        assert_eq!((r[0].0.clone(), r[0].1, l[0].mem[0x1000]), (vec![0x77, 0x5A], 3, 0x77), "activated");
        pass(&mut l, 0, &[(5, fp(0x1000, 0x060C), vec![0]), (5, fp(0x1000, 0x061C), vec![0])]);
        let r = pass(&mut l, 0, &[(12, 0, vec![0x99, 0x00])]);
        assert_eq!((r[0].0.clone(), r[0].1, l[0].mem[0x1000]), (vec![0x99, 0x00], 0, 0x77), "deactivated again");
    }

    /// P3. "logical through its FMMUs bit by bit": a datagram that starts inside an FMMU's span (a
    /// master splitting a process image too long for one frame, as ethercrab does) maps the FMMU's
    /// later bytes, not its first ones.
    #[test]
    fn a_datagram_starting_or_ending_inside_an_fmmu_maps_its_bytes_there() {
        let mut l = line();
        l[0].mem[0x1000..0x1004].copy_from_slice(&[1, 2, 3, 4]);
        l[0].mem[0x1200] = 0xAB;
        pass(
            &mut l,
            0,
            &[
                (5, fp(0x1000, 0x0600), fmmu(0x100, 4, (0, 7), 0x1000, 0, 1, 1)),
                (5, fp(0x1000, 0x0610), fmmu(0x200, 4, (0, 7), 0x1100, 0, 2, 1)),
                (5, fp(0x1000, 0x0620), fmmu(0x300, 2, (4, 3), 0x1200, 0, 1, 1)),
            ],
        );
        let r = pass(&mut l, 0, &[(10, 0x102, vec![0; 2])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![3, 4], 1), "LRD of the FMMU's bytes 2 and 3");
        pass(&mut l, 0, &[(11, 0x202, vec![0xAA, 0xBB])]);
        assert_eq!(l[0].read(0x1100, 4), [0, 0, 0xAA, 0xBB], "LWR onto the FMMU's bytes 2 and 3");
        // Bit by bit: logical bits 0x300.4 to 0x301.3 onto physical 0x1200 bits 0 to 7; a read of
        // logical byte 0x301 alone gets physical bits 4 to 7 (0xA) in its bits 0 to 3.
        let r = pass(&mut l, 0, &[(10, 0x301, vec![0])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![0x0A], 1));
        // A datagram that ends inside an FMMU (the first of the split): its first two bytes alone.
        let r = pass(&mut l, 0, &[(10, 0x100, vec![0; 2])]);
        assert_eq!((r[0].0.clone(), r[0].1), (vec![1, 2], 1), "LRD of the FMMU's bytes 0 and 1");
        pass(&mut l, 0, &[(11, 0x200, vec![0xCC, 0xDD])]);
        assert_eq!(l[0].read(0x1100, 4), [0xCC, 0xDD, 0xAA, 0xBB], "LWR onto the FMMU's bytes 0 and 1");
    }

    /// P4. "its AL state machine (a transition it refuses flagged ...)": the transitions ETG.1000.6
    /// allows downward are taken: OP to SAFE-OP (SOEM's shutdown, ethercrab's `into_safe_op`), SAFE-OP
    /// to PRE-OP.
    #[test]
    fn op_to_safe_op_and_safe_op_to_pre_op_are_taken() {
        let mut l = line();
        let state = |l: &[Esc]| (l[1].mem[0x0130], u16::from_le_bytes([l[1].mem[0x0134], l[1].mem[0x0135]]));
        for c in [0x02u8, 0x04, 0x08] {
            pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![c, 0])]);
        }
        assert_eq!(state(&l), (0x08, 0));
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![0x04, 0])]);
        assert_eq!(state(&l), (0x04, 0), "OP to SAFE-OP");
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![0x02, 0])]);
        assert_eq!(state(&l), (0x02, 0), "SAFE-OP to PRE-OP");
    }

    /// P5. "held until acknowledged": a device the plant faulted (OP to SAFE-OP, flagged, coded) is
    /// acknowledged in its own state, as a master recovers it (SAFE-OP with the acknowledge, 0x14), and
    /// taken back to OP; likewise in PRE-OP. Through the wire and the handle.
    #[test]
    fn a_faulted_device_is_acknowledged_in_its_state_and_back_in_op() {
        netns::isolated("round_4::a_faulted_device_is_acknowledged_in_its_state_and_back_in_op", &[("a0", "b0")], || {
            use veredas_aranha::wire::raw::{Options, RawLink};
            let segment = start(&toml::from_str("interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\" } ]\n").unwrap()).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            let me = [0x10u8; 6];
            let send = |cmd: u8, address: u32, data: &[u8]| {
                let mut body = vec![cmd, 0];
                body.extend_from_slice(&address.to_le_bytes());
                body.extend_from_slice(&(data.len() as u16).to_le_bytes());
                body.extend_from_slice(&[0, 0]);
                body.extend_from_slice(data);
                body.extend_from_slice(&[0, 0]);
                let mut f = vec![0xFF; 6];
                f.extend_from_slice(&me);
                f.extend_from_slice(&0x88A4u16.to_be_bytes());
                f.extend_from_slice(&((body.len() as u16) | 0x1000).to_le_bytes());
                f.extend(body);
                f.resize(60, 0);
                link.send(&f).unwrap();
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(2) {
                    if let Some(r) = link.recv(Duration::from_millis(100)).unwrap()
                        && r.bytes.len() > 12
                        && r.bytes[6..12] != me
                    {
                        return;
                    }
                }
                panic!("no answer");
            };
            let ap1 = (0x0120u32 << 16) | u32::from(0u16.wrapping_sub(1));
            for c in [0x02u8, 0x04, 0x08] {
                send(2, ap1, &[c, 0]);
            }
            assert_eq!(segment.al_state("d1").unwrap(), (8, false));
            segment.fault("d1", 0x001B).unwrap();
            assert_eq!(segment.al_state("d1").unwrap(), (4, true));
            send(2, ap1, &[0x14, 0]);
            assert_eq!(segment.al_state("d1").unwrap(), (4, false), "acknowledged in SAFE-OP");
            send(2, ap1, &[0x08, 0]);
            assert_eq!(segment.al_state("d1").unwrap(), (8, false), "back in OP");
            // Faulted in PRE-OP, acknowledged there.
            send(2, ap1, &[0x02, 0]);
            segment.fault("d1", 0x001A).unwrap();
            assert_eq!(segment.al_state("d1").unwrap(), (2, true));
            send(2, ap1, &[0x12, 0]);
            assert_eq!(segment.al_state("d1").unwrap(), (2, false), "acknowledged in PRE-OP");
        });
    }

    const BIG_PDU: usize = PduStorage::element_size(1100);
    static PDU_STORAGE_BIG: PduStorage<16, BIG_PDU> = PduStorage::new();

    /// P3, end to end: a device at the bound the clause accepts (1024 bytes each way) brought to OP by
    /// ethercrab, whose process image of 2048 bytes it sends in more than one LRW, the second starting
    /// inside the device's FMMU: every output byte reaches the device and every input byte the master.
    #[test]
    fn a_device_of_1024_bytes_each_way_exchanges_every_byte_with_ethercrab() {
        netns::isolated("round_4::a_device_of_1024_bytes_each_way_exchanges_every_byte_with_ethercrab", &[("a0", "b0")], || {
            let entries = (0..128).map(|i| format!("{{ index = 0x7000, subindex = {}, bits = 64 }}", i + 1)).collect::<Vec<_>>().join(", ");
            let ientries = entries.replace("0x7000", "0x6000");
            let text = format!("interface = \"b0\"\ndevices = [ {{ name = \"big\", product_name = \"big\", outputs = [ {entries} ], inputs = [ {ientries} ] }} ]\n");
            let segment = start(&toml::from_str(&text).unwrap()).unwrap();
            let (tx, rx, pdu_loop) = PDU_STORAGE_BIG.try_split().expect("split once");
            let maindevice = Arc::new(MainDevice::new(
                pdu_loop,
                Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(5), pdu: Duration::from_millis(500), eeprom: Duration::from_millis(500), ..Default::default() },
                MainDeviceConfig::default(),
            ));
            std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
            let inputs: Vec<u8> = (0..1024u32).map(|i| (i * 7 + 3) as u8).collect();
            let outputs: Vec<u8> = (0..1024u32).map(|i| (i * 13 + 5) as u8 ^ (i >> 8) as u8).collect();
            futures_lite::future::block_on(async {
                let group = maindevice.init_single_group::<4, 4096>(ethercat_now).await.expect("init");
                let group = group.into_op(&maindevice).await.expect("OP");
                segment.set_inputs("big", &inputs).unwrap();
                for _ in 0..5 {
                    {
                        let d = group.iter(&maindevice).next().unwrap();
                        d.outputs_raw_mut().copy_from_slice(&outputs);
                    }
                    group.tx_rx(&maindevice).await.expect("tx/rx");
                }
                let d = group.iter(&maindevice).next().unwrap();
                let seen_in = d.inputs_raw().to_vec();
                let bad_in = (0..1024).filter(|&i| seen_in[i] != inputs[i]).count();
                let got_out = segment.outputs("big").unwrap();
                let bad_out = (0..1024).filter(|&i| got_out[i] != outputs[i]).count();
                assert_eq!((bad_in, bad_out), (0, 0), "input bytes wrong at the master, output bytes wrong in the device");
            });
        });
    }

    /// P6. The ESC datasheet (0x0910): a written system time is compared "if at least the
    /// first byte (0x0910) was written". A write of the high half alone (0x0914) compares nothing.
    #[test]
    fn a_write_of_the_high_half_alone_compares_nothing() {
        let mut l = line();
        pass(&mut l, 5_000, &[(5, fp(0x1001, 0x0914), vec![0, 0, 0, 0])]);
        assert_eq!(l[1].read(0x092C, 4), [0, 0, 0, 0]);
    }

    /// P8. An FRMW addressed to the second device: the first writes, as the standard has it.
    #[test]
    fn an_frmw_addressed_to_the_second_device_is_written_by_the_first() {
        let mut l = line();
        let r = pass(&mut l, 50_000, &[(14, fp(0x1001, 0x0910), vec![0; 8])]);
        assert_eq!(r[0].1, 3);
        assert_eq!(u64::from_le_bytes(r[0].0.clone().try_into().unwrap()), 49_000);
    }

    /// P6b. A write from 0x0910 running past 0x0917 writes the system time's first byte:
    /// the datasheet compares it.
    #[test]
    fn a_write_of_the_system_time_running_on_is_compared() {
        let mut l = line();
        pass(&mut l, 5_000, &[(5, fp(0x1001, 0x0910), vec![0; 12])]);
        assert_eq!(l[1].read(0x092C, 4), 4_000u32.to_le_bytes());
    }

    static PDU_STORAGE_DOWN: PduStorage<16, BIG_PDU> = PduStorage::new();

    /// P4, end to end: ethercrab's own way down, OP to SAFE-OP to PRE-OP (`into_safe_op`, `into_pre_op`).
    #[test]
    fn ethercrab_takes_the_segment_down_from_op() {
        netns::isolated("round_4::ethercrab_takes_the_segment_down_from_op", &[("a0", "b0")], || {
            let segment = start(&toml::from_str("interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\", outputs = [ { index = 0x7000, subindex = 1, bits = 8 } ] } ]\n").unwrap()).unwrap();
            let (tx, rx, pdu_loop) = PDU_STORAGE_DOWN.try_split().expect("split once");
            let maindevice = Arc::new(MainDevice::new(
                pdu_loop,
                Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(2), pdu: Duration::from_millis(500), eeprom: Duration::from_millis(500), ..Default::default() },
                MainDeviceConfig::default(),
            ));
            std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
            futures_lite::future::block_on(async {
                let group = maindevice.init_single_group::<4, 16>(ethercat_now).await.expect("init");
                let group = group.into_op(&maindevice).await.expect("OP");
                let group = group.into_safe_op(&maindevice).await.expect("OP to SAFE-OP");
                assert_eq!(segment.al_state("d1").unwrap(), (4, false));
                let _group = group.into_pre_op(&maindevice).await.expect("SAFE-OP to PRE-OP");
                assert_eq!(segment.al_state("d1").unwrap(), (2, false));
            });
        });
    }
}

/// C-113's fifth refute round: the critic's probes (`~/.cache/veredas/critic-c113-r5/probes`), kept
/// as pins: INIT from every state and clearing an error there (a plant fault too), the EEPROM's
/// mailbox words 0, a write covering 0x0910 from before it compared, outputs read in SAFE-OP and INIT.
mod round_5 {
    use std::time::{Duration, Instant};

    use veredas_aranha::netns;
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity};
    use veredas_maquinar::ethercat::*;

    fn line() -> Vec<Esc> {
        (0..3u16)
            .map(|i| {
                let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: 0 };
                let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.set_link(i < 2);
                e
            })
            .collect()
    }

    fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
        let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    fn diff(l: &[Esc], i: usize) -> u32 {
        u32::from_le_bytes(l[i].read(0x092C, 4).try_into().unwrap())
    }

    /// "a written system time, by ARMW, FRMW or a plain write covering its first byte (0x0910, as
    /// the ESC datasheet has it), compared": a write that starts before 0x0910 and covers it (a master
    /// writing the DC block in one datagram) is compared, the system time taken from the bytes at 0x0910.
    #[test]
    fn a_write_starting_before_0x0910_and_covering_it_is_compared() {
        // Device 1 (origin 1 µs) at 5 µs: local and system time 4000. A 12-byte write from 0x090C:
        // 4 bytes of port 3's receive time (kept), then 8 bytes of system time, 10 000.
        let mut l = line();
        let mut data = vec![0xEE; 4];
        data.extend_from_slice(&10_000u64.to_le_bytes());
        let r = pass(&mut l, 5_000, &[(5, fp(0x1001, 0x090C), data)]);
        assert_eq!(r[0].1, 1);
        assert_eq!(diff(&l, 1), 0x8000_0000 | 6_000, "compared over 64 bits, 6 µs behind");
    }

    /// "over the low 32 bits when none of the high four is written": an 8-byte write from 0x090C
    /// writes 0x0910..0x0913 alone, so the comparison is over 32 bits whatever the datagram's length.
    #[test]
    fn a_write_from_0x090c_of_8_bytes_compares_the_low_32_bits() {
        // Device 1's system time 2^32 + 4000 (offset 2^32): the master's low half 3000 is 1000 behind it.
        let mut l = line();
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0920), (1u64 << 32).to_le_bytes().to_vec())]);
        let mut data = vec![0xEE; 4];
        data.extend_from_slice(&3_000u32.to_le_bytes());
        pass(&mut l, 5_000, &[(5, fp(0x1001, 0x090C), data)]);
        assert_eq!(diff(&l, 1), 1_000);
    }

    /// The DC block blanked by one broadcast write from 0x0900 (latch, then system time): every
    /// device latches its receive times and compares a system time of 0.
    #[test]
    fn a_broadcast_blank_of_the_dc_block_latches_and_compares() {
        let mut l = line();
        let r = pass(&mut l, 50_000, &[(8, 0x0900 << 16, vec![0; 0x18])]);
        assert_eq!(r[0].1, 3);
        for (i, origin) in [0u32, 1_000, 2_000].iter().enumerate() {
            assert_eq!(u32::from_le_bytes(l[i].read(0x0900, 4).try_into().unwrap()), 50_000 - origin, "device {i} latched");
            assert_eq!(diff(&l, i), 50_000 - origin, "device {i} compared with 0: ahead by its own time");
        }
    }

    /// "held until acknowledged, INIT clearing it": INIT clears an error from every state the
    /// device can hold one in, and is taken from every state (ETG.1000.6: any state to INIT).
    #[test]
    fn init_clears_an_error_from_every_state_and_is_taken_from_each() {
        let state = |l: &[Esc]| (l[1].mem[0x0130], u16::from_le_bytes([l[1].mem[0x0134], l[1].mem[0x0135]]));
        let ask = |l: &mut [Esc], control: u8| {
            pass(l, 0, &[(5, fp(0x1001, 0x0120), vec![control, 0])]);
        };
        // Plain: PRE-OP to INIT, SAFE-OP to INIT.
        for path in [&[0x02u8][..], &[0x02, 0x04][..]] {
            let mut l = line();
            for c in path {
                ask(&mut l, *c);
            }
            ask(&mut l, 0x01);
            assert_eq!(state(&l), (0x01, 0), "{path:?} then INIT");
        }
        // With an error: PRE-OP (PRE-OP to OP refused), SAFE-OP and OP (to BOOT refused); INIT without
        // the acknowledge clears it.
        for (path, refused, held) in [(&[0x02u8][..], 0x08u8, 0x12u8), (&[0x02, 0x04][..], 0x03, 0x14), (&[0x02, 0x04, 0x08][..], 0x03, 0x18)] {
            let mut l = line();
            for c in path {
                ask(&mut l, *c);
            }
            ask(&mut l, refused);
            assert_eq!(state(&l), (held, 0x0011));
            ask(&mut l, 0x01);
            assert_eq!(state(&l), (0x01, 0), "INIT from {held:#x}");
        }
    }

    fn wire_send(link: &veredas_aranha::wire::raw::RawLink, me: [u8; 6], cmd: u8, address: u32, data: &[u8]) {
        let mut body = vec![cmd, 0];
        body.extend_from_slice(&address.to_le_bytes());
        body.extend_from_slice(&(data.len() as u16).to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(data);
        body.extend_from_slice(&[0, 0]);
        let mut f = vec![0xFF; 6];
        f.extend_from_slice(&me);
        f.extend_from_slice(&0x88A4u16.to_be_bytes());
        f.extend_from_slice(&((body.len() as u16) | 0x1000).to_le_bytes());
        f.extend(body);
        if f.len() < 60 {
            f.resize(60, 0);
        }
        link.send(&f).unwrap();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if let Some(r) = link.recv(Duration::from_millis(100)).unwrap()
                && r.bytes.len() > 12
                && r.bytes[6..12] != me
            {
                return;
            }
        }
        panic!("no answer");
    }

    /// The plant's fault recovered by INIT, as a master that restarts a faulted device does: OP,
    /// faulted to SAFE-OP with its code, INIT requested without the acknowledge: INIT, no error.
    #[test]
    fn a_plant_fault_is_cleared_by_init() {
        netns::isolated("round_5::a_plant_fault_is_cleared_by_init", &[("a0", "b0")], || {
            use veredas_aranha::wire::raw::{Options, RawLink};
            let segment = start(&toml::from_str("interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\" } ]\n").unwrap()).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            let me = [0x10u8; 6];
            let ap1 = (0x0120u32 << 16) | u32::from(0u16.wrapping_sub(1));
            for c in [0x02u8, 0x04, 0x08] {
                wire_send(&link, me, 2, ap1, &[c, 0]);
            }
            assert_eq!(segment.al_state("d1").unwrap(), (8, false));
            segment.fault("d1", 0x001B).unwrap();
            assert_eq!(segment.al_state("d1").unwrap(), (4, true));
            wire_send(&link, me, 2, ap1, &[0x01, 0]);
            assert_eq!(segment.al_state("d1").unwrap(), (1, false), "INIT clears the plant's error");
        });
    }

    /// "its SII EEPROM ... no mailbox": the EEPROM's bootstrap and standard mailbox words (ETG.2010
    /// Table 2, words 0x14 to 0x1B) and its mailbox protocols (word 0x1C) are all 0, so no master (IgH
    /// and TwinCAT read them) looks for CoE.
    #[test]
    fn the_eeprom_states_no_mailbox() {
        let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0 };
        let e = vec![esc::PdoEntry { index: 0x7000, subindex: 1, bits: 8 }];
        let w = esc::sii_image(&id, &e, &e);
        assert_eq!(&w[0x14..=0x1C], &[0u16; 9], "no bootstrap mailbox, no standard mailbox, no protocol");
    }

    /// "The plant side reads the outputs a master wrote whatever the AL state": in SAFE-OP and in
    /// INIT as in PRE-OP and OP.
    #[test]
    fn the_outputs_are_read_whatever_the_al_state() {
        netns::isolated("round_5::the_outputs_are_read_whatever_the_al_state", &[("a0", "b0")], || {
            use veredas_aranha::wire::raw::{Options, RawLink};
            let segment = start(&toml::from_str("interface = \"b0\"\ndevices = [ { name = \"d0\", outputs = [ { index = 0x7000, subindex = 1, bits = 8 } ] } ]\n").unwrap()).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            let me = [0x10u8; 6];
            let ap0 = |ado: u16| u32::from(ado) << 16;
            // FMMU 0: logical 0, 1 byte, onto 0x1000, written by the master, activated.
            let mut f = 0u32.to_le_bytes().to_vec();
            f.extend_from_slice(&1u16.to_le_bytes());
            f.extend_from_slice(&[0, 7]);
            f.extend_from_slice(&0x1000u16.to_le_bytes());
            f.extend_from_slice(&[0, 2, 1, 0, 0, 0]);
            wire_send(&link, me, 2, ap0(0x0600), &f);
            for c in [0x02u8, 0x04] {
                wire_send(&link, me, 2, ap0(0x0120), &[c, 0]);
            }
            assert_eq!(segment.al_state("d0").unwrap(), (4, false));
            wire_send(&link, me, 11, 0, &[0xAB]);
            assert_eq!(segment.outputs("d0").unwrap(), [0xAB], "SAFE-OP");
            wire_send(&link, me, 2, ap0(0x0120), &[0x01, 0]);
            assert_eq!(segment.al_state("d0").unwrap(), (1, false));
            wire_send(&link, me, 11, 0, &[0xCD]);
            assert_eq!(segment.outputs("d0").unwrap(), [0xCD], "INIT");
        });
    }
}

/// C-113's sixth refute round: the critic's probes (`~/.cache/veredas/critic-c113-r6/probes`), kept
/// as pins: the features register a master reads (and ethercrab's 64-bit clock from it), a VLAN-
/// tagged frame of commands answered with its tag, a name padded in front refused.
mod round_6 {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ethercrab::std::ethercat_now;
    use ethercrab::{DcSupport, MainDevice, MainDeviceConfig, PduStorage, Timeouts};
    use veredas_aranha::netns;
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_maquinar::ethercat::esc::{self, Esc, Identity};
    use veredas_maquinar::ethercat::*;

    const ME: [u8; 6] = [0x10; 6];

    const SEGMENT: &str = "interface = \"b0\"\ndevices = [ { name = \"d0\" }, { name = \"d1\" }, { name = \"d2\", inputs = [ { index = 0x6000, subindex = 1, bits = 32 } ] } ]\n";

    /// The body (EtherCAT header and datagrams) of a frame of commands.
    fn ecat(datagrams: &[(u8, u32, Vec<u8>, u16)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (i, (cmd, address, data, flags)) in datagrams.iter().enumerate() {
            body.push(*cmd);
            body.push(i as u8);
            body.extend_from_slice(&address.to_le_bytes());
            let more = if i + 1 < datagrams.len() { 0x8000 } else { 0 };
            body.extend_from_slice(&((data.len() as u16) | more | flags).to_le_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(data);
            body.extend_from_slice(&[0, 0]);
        }
        let mut out = ((body.len() as u16) | 0x1000).to_le_bytes().to_vec();
        out.extend(body);
        out
    }

    fn eth(tag: Option<u16>, ethertype: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0xFF; 6];
        f.extend_from_slice(&ME);
        if let Some(tci) = tag {
            f.extend_from_slice(&0x8100u16.to_be_bytes());
            f.extend_from_slice(&tci.to_be_bytes());
        }
        f.extend_from_slice(&ethertype.to_be_bytes());
        f.extend_from_slice(payload);
        if f.len() < 60 {
            f.resize(60, 0);
        }
        f
    }

    /// The next frame not from ME within `wait`.
    fn back(link: &RawLink, wait: Duration) -> Option<Vec<u8>> {
        let started = Instant::now();
        while started.elapsed() < wait {
            if let Some(r) = link.recv(Duration::from_millis(100)).unwrap()
                && r.bytes.len() > 12
                && r.bytes[6..12] != ME
            {
                return Some(r.bytes);
            }
        }
        None
    }

    /// (data, wkc, flags) of each datagram of an EtherCAT payload (header first).
    fn datagrams(p: &[u8]) -> Vec<(Vec<u8>, u16, u16)> {
        let len = (u16::from_le_bytes([p[0], p[1]]) & 0x07FF) as usize;
        let body = &p[2..2 + len];
        let mut at = 0;
        let mut out = Vec::new();
        loop {
            let lf = u16::from_le_bytes([body[at + 6], body[at + 7]]);
            let n = (lf & 0x07FF) as usize;
            out.push((body[at + 10..at + 10 + n].to_vec(), u16::from_le_bytes([body[at + 10 + n], body[at + 11 + n]]), lf & 0xF800));
            at += 12 + n;
            if lf & 0x8000 == 0 {
                break;
            }
        }
        out
    }


    #[test]
    fn the_features_register_states_what_the_esc_does() {
        let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0 };
        let e = Esc::new(esc::sii_image(&id, &[], &[]), 0);
        let f = e.get16(0x0008);
        assert_eq!(f & 0x0001, 0, "FMMU bit-oriented (the clause: logical through its FMMUs bit by bit): {f:#06x}");
        assert_eq!(f & 0x0004, 0x0004, "distributed clocks available (the clause: its distributed clock): {f:#06x}");
        assert_eq!(f & 0x0008, 0x0008, "64-bit system time: {f:#06x}");
        assert_eq!(f & 0x0600, 0, "LRW and the read-write commands supported (the clause counts them): {f:#06x}");
        // And no bit more (round 7: 0x09FC, a fixed FMMU and SyncManager configuration, and 0x01FE
        // passed the bits above).
        assert_eq!(f, 0x01FC);
    }

    const PDU_DATA: usize = PduStorage::element_size(1100);
    static PDU_ROUND_6: PduStorage<16, PDU_DATA> = PduStorage::new();

    /// End to end: the MainDevice sees each device's 64-bit DC (ethercrab's `dc_support`, from 0x0008),
    /// so it configures the segment's clocks; a segment telling it there is none passes the clause's test.
    #[test]
    fn ethercrab_sees_a_64_bit_dc_in_every_device() {
        netns::isolated("round_6::ethercrab_sees_a_64_bit_dc_in_every_device", &[("a0", "b0")], || {
            let _segment = start(&toml::from_str(SEGMENT).unwrap()).unwrap();
            let (tx, rx, pdu_loop) = PDU_ROUND_6.try_split().expect("split once");
            let maindevice = Arc::new(MainDevice::new(
                pdu_loop,
                Timeouts { wait_loop_delay: Duration::from_millis(2), state_transition: Duration::from_secs(5), pdu: Duration::from_millis(500), eeprom: Duration::from_millis(500), ..Default::default() },
                MainDeviceConfig { dc_static_sync_iterations: 100, ..Default::default() },
            ));
            std::thread::spawn(move || futures_lite::future::block_on(ethercrab::std::tx_rx_task("a0", tx, rx).expect("the raw socket")));
            futures_lite::future::block_on(async {
                let group = maindevice.init_single_group::<8, 64>(ethercat_now).await.expect("init");
                let dc: Vec<DcSupport> = group.iter(&maindevice).map(|s| s.dc_support()).collect();
                assert_eq!(dc, [DcSupport::Bits64; 3]);
            });
        });
    }

    #[test]
    fn a_vlan_tagged_frame_of_commands_is_answered_with_its_tag() {
        netns::isolated("round_6::a_vlan_tagged_frame_of_commands_is_answered_with_its_tag", &[("a0", "b0")], || {
            let segment = start(&toml::from_str(SEGMENT).unwrap()).unwrap();
            let link = RawLink::open_with("a0", Options { ethertype: Some(0x88A4), promiscuous: true, outgoing: false }).unwrap();
            // Untagged first: answered, BRD counted 3.
            link.send(&eth(None, 0x88A4, &ecat(&[(7, 0, vec![0; 1], 0)]))).unwrap();
            let f = back(&link, Duration::from_secs(2)).expect("the untagged frame back");
            assert_eq!(datagrams(&f[14..])[0].1, 3);
            // Tagged, VID 5 priority 6, and a priority tag (VID 0).
            let mut seen = Vec::new();
            for tci in [0xC005u16, 0xC000] {
                link.send(&eth(Some(tci), 0x88A4, &ecat(&[(7, 0, vec![0; 1], 0)]))).unwrap();
                let got = back(&link, Duration::from_millis(1500));
                seen.push((tci, got.as_ref().map(|f| (f[12..16].to_vec(), u16::from_be_bytes([f[16], f[17]]), datagrams(&f[18..])[0].1))));
            }
            let s = segment.status();
            assert_eq!(
                seen,
                [(0xC005, Some((vec![0x81, 0x00, 0xC0, 0x05], 0x88A4, 3))), (0xC000, Some((vec![0x81, 0x00, 0xC0, 0x00], 0x88A4, 3)))],
                "a tagged frame of commands back with its tag and counted by the three devices; status {s:?}"
            );
        });
    }

    #[test]
    fn a_name_padded_in_front_is_refused() {
        let cfg: EthercatConfig = toml::from_str("interface = \"b0\"\ndevices = [ { name = \" d\" } ]\n").unwrap();
        assert_eq!(validate(&cfg).unwrap_err().to_string(), "ethercat.devices[0]: a device's name is not empty and has no surrounding whitespace");
        assert_eq!(start(&cfg).err().unwrap().to_string(), "ethercat.devices[0]: a device's name is not empty and has no surrounding whitespace");
    }
}

/// C-113's seventh refute round: the critic's probes (`~/.cache/veredas/critic-c113-r7/probes`),
/// kept as pins: every configuration bound refused both ways; a read-multiple-write counted 1 a
/// device, the reader and each writer; the EEPROM's SyncManager starts, lengths and enables; the SII
/// interface's 8-byte reads stated before any command.
mod round_7 {
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity, PdoEntry};
    use veredas_maquinar::ethercat::*;

    /// `n` devices in a line, each at station 0x1000 + its place, the last with its port 1 open.
    fn line(n: u16) -> Vec<Esc> {
        (0..n)
            .map(|i| {
                let id = Identity { vendor_id: 7, product_code: u32::from(i), revision: 1, serial: 0, name: format!("d{i}"), alias: 0 };
                let mut e = Esc::new(esc::sii_image(&id, &[], &[]), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.set_link(i + 1 < n);
                e
            })
            .collect()
    }

    /// One frame's datagrams through the line: each one's data and working counter.
    fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16)> {
        let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    fn ap(position: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(0u16.wrapping_sub(position))
    }

    #[test]
    fn every_bound_is_refused_both_ways() {
        let device = |extra: &str| format!("interface = \"b0\"\ndevices = [ {{ name = \"d\"{extra} }} ]\n");
        let entries = |n: usize, bits: u8| (0..n).map(|i| format!("{{ index = 0x7000, subindex = {}, bits = {bits} }}", i % 256)).collect::<Vec<_>>().join(", ");
        let cases = [
            (device(&format!(", inputs = [ {} ]", entries(129, 64))), "ethercat.devices[0] (`d`).inputs: 1032 bytes, over 1024"),
            (device(&format!(", outputs = [ {} ]", entries(256, 1))), "ethercat.devices[0] (`d`).outputs: 256 entries, over 255"),
            (device(&format!(", inputs = [ {} ]", entries(1, 0))), "ethercat.devices[0] (`d`).inputs[0]: 0 bit(s), not 1 to 64"),
            (device(&format!(", outputs = [ {} ]", entries(1, 65))), "ethercat.devices[0] (`d`).outputs[0]: 65 bit(s), not 1 to 64"),
        ];
        for (text, want) in &cases {
            let cfg: EthercatConfig = toml::from_str(text).unwrap();
            assert_eq!(validate(&cfg).err().map(|e| e.to_string()).as_deref(), Some(*want));
        }
    }

    /// An ARMW (13) and an FRMW (14) on two devices, and each addressed to no device: the reader
    /// counted 1 and the writer 1 (2), and with no reader both devices writing (2).
    #[test]
    fn a_read_multiple_write_counts_each_device_once() {
        let mut l = line(2);
        let r = pass(&mut l, 10_000, &[(13, ap(0, 0x0910), vec![0; 8]), (14, fp(0x1000, 0x0910), vec![0; 8]), (13, ap(5, 0x0910), vec![0; 8]), (14, fp(0x7777, 0x0910), vec![0; 8])]);
        assert_eq!(r.iter().map(|x| x.1).collect::<Vec<_>>(), [2, 2, 2, 2]);
    }

    /// SM2 at 0x1000 of the outputs' 3 bytes, SM3 where the inputs begin, of their 5, both enabled.
    #[test]
    fn the_eeprom_s_syncmanagers_have_the_file_s_lengths() {
        let id = Identity { vendor_id: 1, product_code: 2, revision: 3, serial: 4, name: "x".into(), alias: 0 };
        let w = esc::sii_image(&id, &[PdoEntry { index: 0x7000, subindex: 1, bits: 24 }], &[PdoEntry { index: 0x6000, subindex: 1, bits: 40 }]);
        let mut at = 0x40;
        while w[at] != 41 {
            assert_ne!(w[at], 0xFFFF, "a SyncManager category");
            at += 2 + w[at + 1] as usize;
        }
        let sm: Vec<u8> = w[at + 2..at + 2 + w[at + 1] as usize].iter().flat_map(|x| x.to_le_bytes()).collect();
        let word = |o: usize| u16::from_le_bytes([sm[o], sm[o + 1]]);
        assert_eq!((word(16), word(18), sm[22], word(24), word(26), sm[30]), (0x1000, 3, 1, esc::inputs_at(3), 5, 1));
    }

    /// The SII control register says 8-byte reads (bit 6) from power-up, as SOEM reads it before
    /// its first command to choose 4- or 8-byte reads (it read 0 until one).
    #[test]
    fn the_sii_interface_states_8_byte_reads_before_any_command() {
        let l = line(1);
        assert_eq!(l[0].get16(0x0502), 0x0040);
    }
}

/// C-113's eighth refute round: the critic's probes (`~/.cache/veredas/critic-c113-r8/probes`),
/// kept as pins: the SII control register's 8-byte reads after a command too; a request of the
/// state held taken without error, SOEM's broadcast OP among them; every auto-increment command
/// back three more; the alias addressing every configured-address command; an unknown state
/// flagged without an error held.
mod round_8 {
    use veredas_maquinar::ethercat::esc::{self, Datagram, Esc, Identity, PdoEntry};

    fn line(n: u16) -> Vec<Esc> {
        (0..n)
            .map(|i| {
                let id = Identity { vendor_id: 0x0700 + u32::from(i), product_code: u32::from(i) + 1, revision: 1, serial: 0, name: format!("d{i}"), alias: if i == 2 { 0x0102 } else { 0 } };
                let e8 = vec![PdoEntry { index: 0x7000, subindex: 1, bits: 8 }];
                let mut e = Esc::new(esc::sii_image(&id, &e8, &e8), u64::from(i) * 1_000);
                e.mem[0x10..0x12].copy_from_slice(&(0x1000 + i).to_le_bytes());
                e.mem[0x12..0x14].copy_from_slice(&id.alias.to_le_bytes());
                e.set_link(i + 1 < n);
                e
            })
            .collect()
    }

    fn pass(line: &mut [Esc], now: u64, dgs: &[(u8, u32, Vec<u8>)]) -> Vec<(Vec<u8>, u16, u32)> {
        let mut d: Vec<Datagram> = dgs.iter().map(|(cmd, address, data)| Datagram { cmd: *cmd, idx: 0, address: *address, circulating: false, more: false, irq: 0, data: data.clone(), wkc: 0 }).collect();
        esc::process(line, &mut d, now);
        d.into_iter().map(|x| (x.data, x.wkc, x.address)).collect()
    }

    fn fp(station: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(station)
    }

    fn ap(position: u16, ado: u16) -> u32 {
        (u32::from(ado) << 16) | u32::from(0u16.wrapping_sub(position))
    }

    fn u16_of(b: &[u8]) -> u16 {
        u16::from_le_bytes([b[0], b[1]])
    }

    /// "its SII EEPROM read through the registers, 8 bytes at a time as its control register says":
    /// after a read command (ethercrab's 6-byte request, IgH's 4-byte one with 0x80 in the low byte) the
    /// control register still says 8-byte reads, no command pending, no error: 0x0040.
    #[test]
    fn the_sii_control_register_says_8_byte_reads_after_a_command_too() {
        let mut l = line(3);
        // ethercrab / SOEM: control (read), address word 8, high word 0.
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0502), vec![0x00, 0x01, 0x08, 0x00, 0x00, 0x00])]);
        let r = pass(&mut l, 0, &[(4, fp(0x1001, 0x0502), vec![0; 2]), (4, fp(0x1001, 0x0508), vec![0; 8])]);
        let after_ethercrab = u16_of(&r[0].0);
        // IgH: 0x80 (two address octets), 0x01 (read), the word.
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0502), vec![0x80, 0x01, 0x0A, 0x00])]);
        let r2 = pass(&mut l, 0, &[(4, fp(0x1001, 0x0502), vec![0; 10])]);
        let after_igh = u16_of(&r2[0].0);
        assert_eq!((after_ethercrab, after_igh), (0x0040, 0x0040));
        assert_eq!(&r2[0].0[6..10], &[0x02, 0x00, 0x00, 0x00], "product code 2 at word 0x0A");
    }


    /// "a transition it refuses flagged with code 0x0011": a request of the state the device is
    /// already in is no transition (ETG.1000.6), so it is never flagged; SOEM's `ec_writestate(0)`
    /// broadcasts OP to every device, some already in OP, while one comes back from SAFE-OP.
    #[test]
    fn a_request_of_the_state_held_is_taken_without_error() {
        let st = |l: &[Esc], i: usize| (l[i].mem[0x0130], u16::from_le_bytes([l[i].mem[0x0134], l[i].mem[0x0135]]));
        let mut seen = Vec::new();
        for (path, again) in [(&[0x02u8][..], 0x02u8), (&[0x02, 0x04][..], 0x04), (&[0x02, 0x04, 0x08][..], 0x08)] {
            let mut l = line(3);
            for c in path {
                pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![*c, 0])]);
            }
            pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![again, 0])]);
            seen.push(st(&l, 1));
        }
        // SOEM's broadcast: every device in OP but d1 back in SAFE-OP; BWR OP to all (one byte).
        let mut l = line(3);
        for c in [0x02u8, 0x04, 0x08] {
            pass(&mut l, 0, &[(8, u32::from(0x0120u16) << 16, vec![c])]);
        }
        pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![0x04, 0])]);
        let r = pass(&mut l, 0, &[(8, u32::from(0x0120u16) << 16, vec![0x08])]);
        let all: Vec<(u8, u16)> = (0..3).map(|i| st(&l, i)).collect();
        assert_eq!(seen, [(0x02, 0), (0x04, 0), (0x08, 0)]);
        assert_eq!((all, r[0].1), (vec![(0x08, 0), (0x08, 0), (0x08, 0)], 3), "each in OP, the broadcast counted by the three");
    }


    /// "auto-increment by position, the address back three more than sent": every auto-increment
    /// command, not the APRD alone.
    #[test]
    fn every_auto_increment_command_comes_back_three_more() {
        let mut l = line(3);
        let dgs = [(1u8, ap(1, 0x0F80), vec![0]), (2, ap(1, 0x0F80), vec![1]), (3, ap(2, 0x0F81), vec![2]), (13, ap(0, 0x0910), vec![0; 8])];
        let r = pass(&mut l, 1_000_000, &dgs);
        let back: Vec<u16> = r.iter().map(|x| x.2 as u16).collect();
        let want: Vec<u16> = dgs.iter().map(|d| (d.1 as u16).wrapping_add(3)).collect();
        assert_eq!(back, want);
    }


    /// "configured address, or the alias once DL control's bit 24 is set": every configured-address
    /// command (FPWR, FPRW, FRMW too: the ESC datasheet, DL control bit 24), not the FPRD alone.
    #[test]
    fn the_alias_addresses_every_configured_command() {
        let mut l = line(3);
        pass(&mut l, 0, &[(5, fp(0x1002, 0x0100), vec![0, 0, 0, 1])]);
        let r = pass(&mut l, 5_000, &[(5, fp(0x0102, 0x0F80), vec![0xAB]), (6, fp(0x0102, 0x0F80), vec![0xCD]), (14, fp(0x0102, 0x0910), vec![0; 8])]);
        let got = (r[0].1, r[1].1, r[1].0.clone(), l[2].mem[0x0F80], r[2].1, u64::from_le_bytes(r[2].0.clone().try_into().unwrap()));
        // FRMW by d2's alias: d2 reads (its system time at 5 us is 3000), d0 and d1 write: 3.
        assert_eq!(got, (1, 3, vec![0xAB], 0xCD, 3, 3_000));
    }


    /// "an unknown state 0x0012": a request of a state that does not exist, from a device holding no
    /// error (PRE-OP asked 0x05, OP asked 0x0F), is flagged 0x0012 as one under an acknowledge is.
    #[test]
    fn an_unknown_state_is_flagged_without_an_error_held() {
        let st = |l: &[Esc]| (l[1].mem[0x0130], u16::from_le_bytes([l[1].mem[0x0134], l[1].mem[0x0135]]));
        let mut seen = Vec::new();
        for (path, unknown) in [(&[0x02u8][..], 0x05u8), (&[0x02, 0x04, 0x08][..], 0x0F)] {
            let mut l = line(3);
            for c in path {
                pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![*c, 0])]);
            }
            pass(&mut l, 0, &[(5, fp(0x1001, 0x0120), vec![unknown, 0])]);
            seen.push(st(&l));
        }
        assert_eq!(seen, [(0x12, 0x0012), (0x18, 0x0012)]);
    }
}
