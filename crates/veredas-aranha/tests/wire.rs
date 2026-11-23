//! The wire seam (contract: the wire clause): the capture file read and written, checked against
//! tshark's own reader and writer; a capture replayed; the memory hub; and the raw link behind the
//! same seam, on a veth pair in a private network of the test's own.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use veredas_aranha::profinet::dcp::{self, Filter};
use veredas_aranha::profinet::ether::{self, VlanTag};
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::hub::{self, Hub};
use veredas_aranha::wire::pcap::{self, Capture, Damage, DamageKind, PcapError, Replay};
use veredas_aranha::wire::{Frame, Wire, WireError};

const A: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x0A, 0x01]);
const B: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x0B, 0x01]);
const C: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x0C, 0x01]);

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// A folder of this test's own under the one cargo gives integration tests (never `/tmp`).
fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wire").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Five frames of the kinds a scan keeps: a DCP Identify, an LLDP frame, a tagged frame, a frame
/// a capture cut (60 bytes kept of 1514), and the shortest, each at a stamp of its own.
fn frames() -> Vec<Frame> {
    let identify = dcp::identify_request(A, 0x0102_0304, 1, &Filter::All).unwrap();
    let lldp = lldp::frame(B, &LldpPdu::new(ChassisId::Local { id: "plc-1".into() }, PortId::Local { id: "port-001.plc-1".into() }, 20)).unwrap();
    let tagged = ether::build_tagged(C, A, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 }), 0x8892, &[0x80, 0x00, 1, 2, 3]).unwrap();
    let mut cut = ether::build(Mac::LLDP_MULTICAST, C, 0x88B5, &[0x55; 1500]).unwrap();
    cut.truncate(60);
    let shortest = hex("ffffffffffff 020000000a01 0800");
    vec![
        Frame::new(1_700_000_000_123_456_789, "eth1", identify),
        Frame::new(1_700_000_001_000_000_001, "eth1", lldp),
        Frame::new(1_700_000_001_999_999_999, "eth1", tagged),
        Frame { length: 1514, ..Frame::new(4_294_967_295_999_999_999, "eth1", cut) },
        Frame::new(0, "eth1", shortest),
    ]
}

/// tshark's reading of a capture file: each frame's stamp, length on the wire, bytes kept and the
/// bytes themselves, and what it says on its error stream.
fn tshark_reads(file: &std::path::Path) -> (Vec<(String, String, String, String)>, String) {
    let fields = Command::new(common::tshark())
        .args(["-n", "-r"])
        .arg(file)
        .args(["-T", "fields", "-E", "separator=,", "-e", "frame.time_epoch", "-e", "frame.len", "-e", "frame.cap_len"])
        .output()
        .unwrap();
    let raw = Command::new(common::tshark()).args(["-n", "-r"]).arg(file).args(["-T", "ek", "-x"]).output().unwrap();
    let raws: Vec<String> = String::from_utf8(raw.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("{\"index\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["layers"]["frame_raw"].as_str().unwrap_or_default().to_string()
        })
        .collect();
    let rows: Vec<(String, String, String, String)> = String::from_utf8(fields.stdout)
        .unwrap()
        .lines()
        .zip(&raws)
        .map(|(l, raw)| {
            let v: Vec<&str> = l.split(',').collect();
            (v[0].to_string(), v[1].to_string(), v[2].to_string(), raw.clone())
        })
        .collect();
    assert_eq!(rows.len(), raws.len(), "tshark's two readings differ in count");
    (rows, String::from_utf8_lossy(&fields.stderr).to_string())
}

/// tshark rewriting a capture in another format (`pcap`, `nsecpcap`, `pcapng`).
fn tshark_writes(from: &std::path::Path, to: &std::path::Path, format: &str) {
    let out = Command::new(common::tshark()).args(["-n", "-r"]).arg(from).arg("-w").arg(to).args(["-F", format]).output().unwrap();
    assert!(out.status.success(), "tshark -F {format}: {}", String::from_utf8_lossy(&out.stderr));
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn a_capture_is_written_to_the_byte_and_read_back_whole() {
    let frames = frames();
    let bytes = pcap::write(&frames).unwrap();
    // The file header: nanosecond magic, little-endian, 2.4, zone and accuracy 0, 262 144, Ethernet.
    assert_eq!(&bytes[..24], &hex("4d3cb2a1 0200 0400 00000000 00000000 00000400 01000000")[..]);
    // The first record: seconds, nanoseconds, bytes kept, length on the wire, the bytes.
    let first = &frames[0];
    assert_eq!(&bytes[24..40], &hex(&format!("{:08x} {:08x} {:08x} {:08x}", 1_700_000_000u32.swap_bytes(), 123_456_789u32.swap_bytes(), (first.bytes.len() as u32).swap_bytes(), (first.bytes.len() as u32).swap_bytes()))[..]);
    assert_eq!(&bytes[40..40 + first.bytes.len()], &first.bytes[..]);
    // The cut frame states 60 kept of 1514; the last second a capture counts, to the nanosecond.
    let at = 24 + frames[..3].iter().map(|f| 16 + f.bytes.len()).sum::<usize>();
    assert_eq!(&bytes[at..at + 16], &hex("ffffffff ffc99a3b 3c000000 ea050000")[..]);
    assert_eq!(bytes.len(), 24 + frames.iter().map(|f| 16 + f.bytes.len()).sum::<usize>());

    let back = pcap::read(&bytes, "b0").unwrap();
    assert_eq!(
        back,
        Capture {
            big_endian: false,
            nanos: true,
            version: (2, 4),
            snaplen: pcap::SNAPLEN,
            linktype: 1,
            frames: frames.iter().map(|f| Frame { iface: "b0".into(), ..f.clone() }).collect(),
            damage: None,
        }
    );
    assert!(back.frames[3].is_cut() && !back.frames[0].is_cut());
    // An empty capture is its header alone.
    let empty = pcap::write(&[]).unwrap();
    assert_eq!(empty.len(), 24);
    assert_eq!(pcap::read(&empty, "b0").unwrap().frames, vec![]);
    // A file written and read back by path.
    let file = folder("by_path").join("scan.pcap");
    pcap::write_file(&file, &frames).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(pcap::read_file(&file, "b0").unwrap(), back);
    let missing = pcap::read_file(&file.with_file_name("none.pcap"), "b0").unwrap_err();
    assert!(matches!(missing, PcapError::Io { .. }) && missing.to_string().contains("none.pcap"), "{missing}");
}

#[test]
fn tshark_reads_every_capture_written_as_it_was_written() {
    let dir = folder("tshark_reads");
    let frames = frames();
    let file = dir.join("written.pcap");
    pcap::write_file(&file, &frames).unwrap();
    let (rows, err) = tshark_reads(&file);
    assert_eq!(err, "", "tshark complained of the file");
    assert_eq!(rows.len(), frames.len());
    for (f, (time, len, cap, raw)) in frames.iter().zip(&rows) {
        assert_eq!(time, &format!("{}.{:09}", f.at_ns / 1_000_000_000, f.at_ns % 1_000_000_000));
        assert_eq!((len, cap), (&f.length.to_string(), &f.bytes.len().to_string()));
        assert_eq!(raw, &to_hex(&f.bytes));
    }
}

#[test]
fn captures_tshark_writes_are_read_in_either_resolution_and_pcapng_is_refused() {
    let dir = folder("tshark_writes");
    // tshark writes seconds as a signed number (to 2038), though it reads them to 2106: the frame
    // in the last second a capture counts moves to the last second tshark writes.
    let last = 2_147_483_647_999_999_999;
    let frames: Vec<Frame> = frames().into_iter().map(|f| Frame { at_ns: f.at_ns.min(last), ..f }).collect();
    assert_eq!(frames[3].at_ns, last);
    let ours = dir.join("ours.pcap");
    pcap::write_file(&ours, &frames).unwrap();

    // Nanosecond pcap from tshark: the frames whole.
    let ns = dir.join("ns.pcap");
    tshark_writes(&ours, &ns, "nsecpcap");
    let read = pcap::read_file(&ns, "eth1").unwrap();
    assert!(!read.big_endian && read.nanos && read.damage.is_none());
    assert_eq!(read.frames, frames);
    // Microsecond pcap: each stamp to its microsecond, the rest whole.
    let us = dir.join("us.pcap");
    tshark_writes(&ours, &us, "pcap");
    let read = pcap::read_file(&us, "eth1").unwrap();
    assert!(!read.nanos);
    assert_eq!(read.frames, frames.iter().map(|f| Frame { at_ns: f.at_ns / 1000 * 1000, ..f.clone() }).collect::<Vec<_>>());
    assert_eq!(read.frames[0].at_ns, 1_700_000_000_123_456_000);
    // pcapng is refused by name, with the way out.
    let ng = dir.join("ng.pcapng");
    tshark_writes(&ours, &ng, "pcapng");
    let err = pcap::read_file(&ng, "eth1").unwrap_err();
    assert!(matches!(err, PcapError::Pcapng));
    assert_eq!(err.to_string(), "this is pcapng, which is not read: write it as classic pcap (`editcap -F pcap`)");
}

/// A capture written by hand: the header's fields as given, in the byte order given, then records.
fn by_hand(big: bool, magic: u32, version: (u16, u16), linktype: u32, records: &[(u32, u32, &[u8], u32)]) -> Vec<u8> {
    let w32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let w16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut out = Vec::new();
    out.extend_from_slice(&w32(magic));
    out.extend_from_slice(&w16(version.0));
    out.extend_from_slice(&w16(version.1));
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&w32(96));
    out.extend_from_slice(&w32(linktype));
    for (secs, fraction, bytes, length) in records {
        out.extend_from_slice(&w32(*secs));
        out.extend_from_slice(&w32(*fraction));
        out.extend_from_slice(&w32(bytes.len() as u32));
        out.extend_from_slice(&w32(*length));
        out.extend_from_slice(bytes);
    }
    out
}

#[test]
fn either_byte_order_either_resolution_and_every_header_field_are_read() {
    let dir = folder("by_hand");
    let f = frames();
    let (a, b) = (&f[0].bytes[..], &f[1].bytes[..]);
    for big in [false, true] {
        for (magic, nanos, per) in [(pcap::MAGIC_MICROS, false, 1000u64), (pcap::MAGIC_NANOS, true, 1)] {
            let bytes = by_hand(big, magic, (2, 4), 1, &[(7, 123_456, a, a.len() as u32), (8, 999_999, b, 1514)]);
            let read = pcap::read(&bytes, "p1").unwrap();
            assert_eq!((read.big_endian, read.nanos, read.version, read.snaplen, read.linktype), (big, nanos, (2, 4), 96, 1));
            assert_eq!(read.frames, vec![Frame::new(7_000_000_000 + 123_456 * per, "p1", a.to_vec()), Frame { length: 1514, ..Frame::new(8_000_000_000 + 999_999 * per, "p1", b.to_vec()) }]);
            // tshark reads the same stamps, lengths and bytes from the same file.
            let file = dir.join(format!("{big}-{nanos}.pcap"));
            std::fs::write(&file, &bytes).unwrap();
            let (rows, _) = tshark_reads(&file);
            let want: Vec<_> = read.frames.iter().map(|f| (format!("{}.{:09}", f.at_ns / 1_000_000_000, f.at_ns % 1_000_000_000), f.length.to_string(), f.bytes.len().to_string(), to_hex(&f.bytes))).collect();
            assert_eq!(rows, want, "big {big}, nanos {nanos}");
        }
    }
    // The snapshot length is not held against the records (tshark reads a 60-byte frame under 20).
    let mut small = by_hand(false, pcap::MAGIC_NANOS, (2, 4), 1, &[(1, 0, a, a.len() as u32)]);
    small[16..20].copy_from_slice(&20u32.to_le_bytes());
    let read = pcap::read(&small, "p1").unwrap();
    assert_eq!((read.snaplen, read.frames.len(), read.frames[0].bytes.len()), (20, 1, a.len()));
    // Versions 2.3 and 3.0 are read, 1.0 and 0.9 refused (tshark: "major version 1 unsupported").
    for v in [(2, 3), (3, 0), (2, 0xFFFF)] {
        assert_eq!(pcap::read(&by_hand(true, pcap::MAGIC_MICROS, v, 1, &[(1, 0, a, 60)]), "p1").unwrap().version, v);
    }
    for v in [(1, 0), (0, 9), (1, 0xFFFF)] {
        let err = pcap::read(&by_hand(false, pcap::MAGIC_MICROS, v, 1, &[]), "p1").unwrap_err();
        assert!(matches!(err, PcapError::Version(x, y) if (x, y) == v), "{err}");
    }
    assert_eq!(pcap::read(&by_hand(false, pcap::MAGIC_MICROS, (1, 0), 1, &[]), "p1").unwrap_err().to_string(), "pcap version 1.0 is not read: major version 2 or later");
    // Ethernet with a check sequence's bits set (four bytes each) is Ethernet; the frames are kept
    // as recorded, the check sequence with them, and the field whole.
    let fcs = 0x1000_0001 | (2 << 29);
    let with_fcs = [a, &[0xAA, 0xBB, 0xCC, 0xDD]].concat();
    let read = pcap::read(&by_hand(false, pcap::MAGIC_MICROS, (2, 4), fcs, &[(1, 0, &with_fcs, 64)]), "p1").unwrap();
    assert_eq!((read.linktype, &read.frames[0].bytes), (fcs, &with_fcs));
    // Another link type, a reserved bit, another magic and a file shorter than its header are
    // refused by name.
    for lt in [0, 101, 105, 0x0100_0001, 0x0800_0001] {
        let err = pcap::read(&by_hand(false, pcap::MAGIC_MICROS, (2, 4), lt, &[]), "p1").unwrap_err();
        assert!(matches!(err, PcapError::NotEthernet(x) if x == lt), "{lt:#x}: {err}");
    }
    assert_eq!(pcap::read(&by_hand(true, pcap::MAGIC_MICROS, (2, 4), 101, &[]), "p1").unwrap_err().to_string(), "link type 0x65 is not Ethernet (1)");
    for magic in [0xA1B2_CD34, 0xD4C3_B2A0, 0, 0x0A0D_0D0B] {
        let err = pcap::read(&by_hand(false, magic, (2, 4), 1, &[]), "p1").unwrap_err();
        assert!(matches!(err, PcapError::Magic(_)), "{magic:#x}: {err}");
    }
    assert_eq!(
        pcap::read(&by_hand(true, 0xA1B2_CD34, (2, 4), 1, &[]), "p1").unwrap_err().to_string(),
        "magic number 0xa1b2cd34 is not classic pcap's (a1b2c3d4 or a1b23c4d, in either byte order)"
    );
    for len in [0, 4, 23] {
        let err = pcap::read(&by_hand(false, pcap::MAGIC_MICROS, (2, 4), 1, &[])[..len], "p1").unwrap_err();
        assert!(matches!(err, PcapError::TooShort(x) if x == len), "{err}");
    }
    assert_eq!(pcap::read(&[0; 23], "p1").unwrap_err().to_string(), "a capture file begins with a 24-byte header; this one has 23 bytes");
}

#[test]
fn a_capture_cut_short_gives_the_frames_before_its_damage_as_tshark_does() {
    let dir = folder("cut");
    let frames = frames();
    let whole = pcap::write(&frames).unwrap();
    let starts: Vec<usize> = frames.iter().scan(24, |at, f| {
        let s = *at;
        *at += 16 + f.bytes.len();
        Some(s)
    }).collect();

    // Cut inside the third record's header, then inside its bytes: two frames, the damage named;
    // tshark reads the same two and says the file was cut short.
    let header_cut = &whole[..starts[2] + 9];
    let read = pcap::read(header_cut, "b0").unwrap();
    assert_eq!(read.frames, pcap::read(&whole, "b0").unwrap().frames[..2]);
    assert_eq!(read.damage, Some(Damage { record: 2, offset: starts[2], kind: DamageKind::HeaderCut { have: 9 } }));
    assert_eq!(read.damage.unwrap().to_string(), format!("record 2 at byte {} is cut short in its header: 9 of 16 bytes", starts[2]));
    let data_cut = &whole[..starts[2] + 16 + 5];
    let read = pcap::read(data_cut, "b0").unwrap();
    assert_eq!(read.frames.len(), 2);
    let declared = frames[2].bytes.len() as u32;
    assert_eq!(read.damage, Some(Damage { record: 2, offset: starts[2], kind: DamageKind::DataCut { declared, have: 5 } }));
    assert_eq!(read.damage.unwrap().to_string(), format!("record 2 at byte {} is cut short: 5 of its {declared} bytes", starts[2]));
    for (name, bytes) in [("header", header_cut), ("data", data_cut)] {
        let file = dir.join(format!("{name}.pcap"));
        std::fs::write(&file, bytes).unwrap();
        let (rows, err) = tshark_reads(&file);
        assert_eq!(rows.len(), 2, "{name}");
        assert!(err.contains("cut short in the middle of a packet"), "{name}: {err}");
    }

    // A record keeping 262 144 bytes is read; one keeping a byte more ends the capture there.
    let big = vec![0x5A; pcap::SNAPLEN as usize];
    let at_limit = pcap::write(&[Frame::new(5, "x", big.clone())]).unwrap();
    assert_eq!(pcap::read(&at_limit, "x").unwrap().frames[0].bytes, big);
    let mut past = pcap::write(&frames[..1]).unwrap();
    past.extend_from_slice(&pcap::write(&[Frame::new(5, "x", vec![0; 100])]).unwrap()[24..]);
    let second = 24 + 16 + frames[0].bytes.len();
    past[second + 8..second + 12].copy_from_slice(&(pcap::SNAPLEN + 1).to_le_bytes());
    let read = pcap::read(&past, "x").unwrap();
    assert_eq!(read.frames.len(), 1);
    assert_eq!(read.damage, Some(Damage { record: 1, offset: second, kind: DamageKind::TooLong { declared: pcap::SNAPLEN + 1 } }));
    assert_eq!(read.damage.unwrap().to_string(), format!("record 1 at byte {second} says it keeps 262145 bytes, past the 262144 a record holds"));

    // Every cut of a written capture reads, its frames a prefix of the whole, its damage named
    // unless it falls between records.
    let all = pcap::read(&whole, "b0").unwrap().frames;
    for n in 24..whole.len() {
        let read = pcap::read(&whole[..n], "b0").unwrap();
        assert_eq!(read.frames[..], all[..read.frames.len()], "cut at {n}");
        assert_eq!(read.damage.is_none(), starts.contains(&n), "cut at {n}");
    }
    // Noise after a good header reads or names its damage, never a panic.
    let mut noise = common::Noise(0x5EED);
    for _ in 0..20_000 {
        let len = 24 + usize::from(noise.next()) * 3;
        let mut bytes: Vec<u8> = (0..len).map(|_| noise.next()).collect();
        bytes[..24].copy_from_slice(&whole[..24]);
        let read = pcap::read(&bytes, "n").unwrap();
        let back = pcap::write(&read.frames).unwrap();
        assert_eq!(pcap::read(&back, "n").unwrap().frames, read.frames);
    }
}

#[test]
fn what_a_capture_holds_as_heard_is_kept_as_tshark_keeps_it() {
    let dir = folder("as_heard");
    let a = frames().remove(0).bytes;
    // A length on the wire under the bytes kept: kept as stated (tshark: "Frame length is less
    // than captured length").
    let under = by_hand(false, pcap::MAGIC_NANOS, (2, 4), 1, &[(1, 5, &a, 20)]);
    let read = pcap::read(&under, "p").unwrap();
    assert_eq!((read.frames[0].length, read.frames[0].bytes.len()), (20, a.len()));
    assert_eq!(pcap::read(&pcap::write(&read.frames).unwrap(), "p").unwrap().frames, read.frames);
    // A fraction past a second is carried into the next, at either resolution (tshark shows
    // "1.(1500000000 nanoseconds)" for both).
    for (magic, fraction) in [(pcap::MAGIC_MICROS, 1_500_000), (pcap::MAGIC_NANOS, 1_500_000_000)] {
        let read = pcap::read(&by_hand(false, magic, (2, 4), 1, &[(1, fraction, &a, 60)]), "p").unwrap();
        assert_eq!(read.frames[0].at_ns, 2_500_000_000);
    }
    // A record keeping no bytes, last in its file, is a frame of none (tshark reads it too).
    let empty_last = by_hand(false, pcap::MAGIC_NANOS, (2, 4), 1, &[(1, 0, &a, 60), (2, 0, &[], 0)]);
    let read = pcap::read(&empty_last, "p").unwrap();
    assert_eq!((read.frames.len(), read.frames[1].bytes.len(), read.damage), (2, 0, None));
    let file = dir.join("empty_last.pcap");
    std::fs::write(&file, &empty_last).unwrap();
    assert_eq!(tshark_reads(&file).0.len(), 2);
    let file = dir.join("under.pcap");
    std::fs::write(&file, &under).unwrap();
    let (rows, _) = tshark_reads(&file);
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_str()), ("20", a.len().to_string().as_str()));
}

#[test]
fn writing_refuses_what_a_capture_cannot_hold() {
    let a = frames().remove(0).bytes;
    let last = 4_294_967_295_999_999_999;
    assert!(pcap::write(&[Frame::new(last, "x", a.clone())]).is_ok());
    let err = pcap::write(&[Frame::new(last + 1, "x", a.clone())]).unwrap_err();
    assert!(matches!(err, PcapError::Stamp(s) if s == last + 1));
    assert_eq!(err.to_string(), "a frame stamped 4294967296000000000 ns is past what a capture's 32-bit seconds count (2106)");
    assert!(matches!(pcap::write(&[Frame::new(u64::MAX, "x", a.clone())]).unwrap_err(), PcapError::Stamp(u64::MAX)));
    let err = pcap::write(&[Frame::new(1, "x", vec![0; pcap::SNAPLEN as usize + 1])]).unwrap_err();
    assert!(matches!(err, PcapError::FrameTooLong(262_145)));
    assert_eq!(err.to_string(), "a frame of 262145 bytes is past the 262144 a record holds");
    let err = pcap::write(&[Frame { length: u32::MAX as usize + 1, ..Frame::new(1, "x", a.clone()) }]).unwrap_err();
    assert!(matches!(err, PcapError::LengthTooLong(n) if n == u32::MAX as usize + 1));
    assert_eq!(err.to_string(), "a frame's length on the wire, 4294967296, is past what a record states (32 bits)");
    assert!(pcap::write(&[Frame { length: u32::MAX as usize, ..Frame::new(1, "x", a) }]).is_ok());
}

#[test]
fn a_replayed_capture_hands_its_frames_in_order_then_ends_and_keeps_what_is_sent() {
    let frames = frames();
    let mut replay = Replay::new("scan.pcap", frames.clone());
    assert_eq!((replay.name(), replay.mac(), replay.remaining()), ("scan.pcap", None, 5));
    let identify = dcp::identify_request(A, 9, 1, &Filter::All).unwrap();
    replay.send(&identify).unwrap();
    assert!(matches!(replay.send(&[0; 13]), Err(WireError::TooShort(13))));
    assert!(matches!(replay.send(&vec![0; 65_537]), Err(WireError::TooLong(65_537))));
    replay.send(&vec![0; 65_536]).unwrap();
    let started = Instant::now();
    for f in &frames {
        assert_eq!(replay.recv(Duration::from_secs(60)).unwrap().as_ref(), Some(f));
    }
    assert!(started.elapsed() < Duration::from_secs(30), "a capture never waits");
    assert_eq!(replay.remaining(), 0);
    let end = replay.recv(Duration::ZERO).unwrap_err();
    assert!(matches!(&end, WireError::Ended(n) if n == "scan.pcap"));
    assert_eq!(end.to_string(), "scan.pcap has no frame left: its capture is read to the end");
    assert_eq!(replay.sent(), &[identify, vec![0; 65_536]]);
    assert_eq!(WireError::TooShort(13).to_string(), "a frame of 13 bytes is shorter than an Ethernet header");
    assert_eq!(WireError::TooLong(65_537).to_string(), "a frame of 65537 bytes is past the 65536 a wire carries");
}

#[test]
fn the_hub_carries_each_frame_to_every_other_port_stamped_with_its_clock_and_the_hearer() {
    let hub = Hub::new();
    let mut a = hub.port("eth0", A);
    let mut b = hub.port("eth0", B);
    let mut c = hub.port("lan2", C);
    assert_eq!((a.name(), b.name(), c.name()), ("eth0", "eth0", "lan2"));
    assert_eq!((a.mac(), b.mac(), c.mac()), (Some(A), Some(B), Some(C)));
    assert_eq!((hub.ports(), hub.now()), (3, 0));
    let identify = dcp::identify_request(A, 1, 1, &Filter::All).unwrap();
    hub.set_now(1_700_000_000_000_000_042);
    a.send(&identify).unwrap();
    hub.set_now(1_700_000_000_000_000_043);
    c.send(&identify[..14]).unwrap();
    let z = Duration::ZERO;
    assert_eq!(b.recv(z).unwrap(), Some(Frame::new(1_700_000_000_000_000_042, "eth0", identify.clone())));
    assert_eq!(b.recv(z).unwrap(), Some(Frame::new(1_700_000_000_000_000_043, "eth0", identify[..14].to_vec())));
    assert_eq!(c.recv(z).unwrap(), Some(Frame::new(1_700_000_000_000_000_042, "lan2", identify.clone())));
    assert_eq!(a.recv(z).unwrap(), Some(Frame::new(1_700_000_000_000_000_043, "eth0", identify[..14].to_vec())));
    for port in [&mut a, &mut b, &mut c] {
        assert_eq!(port.recv(z).unwrap(), None, "a port never hears its own frames");
    }
    // A clone is the same segment; a port dropped leaves it.
    let same = hub.clone();
    drop(c);
    assert_eq!(same.ports(), 2);
    a.send(&identify).unwrap();
    assert!(b.recv(z).unwrap().is_some());
    let mut d = same.port("eth9", C);
    a.send(&identify).unwrap();
    assert_eq!(d.recv(z).unwrap().map(|f| f.iface), Some("eth9".to_string()));
    assert_eq!(b.recv(z).unwrap().map(|f| f.iface), Some("eth0".to_string()));
    // Lengths as every wire checks them, refused before anyone hears.
    assert!(matches!(a.send(&[0; 13]), Err(WireError::TooShort(13))));
    assert!(matches!(a.send(&vec![0; 65_537]), Err(WireError::TooLong(65_537))));
    assert_eq!(b.recv(z).unwrap(), None);
    a.send(&vec![0; 65_536]).unwrap();
    assert_eq!(b.recv(z).unwrap().map(|f| f.bytes.len()), Some(65_536));
    // A port that does not read keeps its last QUEUE frames and counts the rest let go.
    let _ = d.recv(z);
    for i in 0..hub::QUEUE + 3 {
        a.send(&[&[i as u8, (i >> 8) as u8][..], &[0; 12]].concat()).unwrap();
    }
    assert_eq!((b.dropped(), d.dropped()), (3, 3));
    assert_eq!(b.recv(z).unwrap().map(|f| (f.bytes[0], f.bytes[1])), Some((3, 0)));
    let mut kept = 1;
    while b.recv(z).unwrap().is_some() {
        kept += 1;
    }
    assert_eq!(kept, hub::QUEUE);
}

#[test]
fn a_hub_port_waits_for_a_frame_no_longer_than_its_timeout() {
    let hub = Hub::new();
    let mut a = hub.port("a", A);
    let mut b = hub.port("b", B);
    let started = Instant::now();
    assert_eq!(b.recv(Duration::from_millis(50)).unwrap(), None);
    assert!(started.elapsed() >= Duration::from_millis(50));
    // A frame sent while it waits ends the wait.
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        a.send(&[0x11; 60]).unwrap();
        a
    });
    let started = Instant::now();
    let f = b.recv(Duration::from_secs(20)).unwrap();
    let waited = started.elapsed();
    assert_eq!(f.map(|f| f.bytes), Some(vec![0x11; 60]));
    assert!(waited >= Duration::from_millis(90) && waited < Duration::from_secs(15), "{waited:?}");
    let mut a = sender.join().unwrap();
    // A timeout past what the clock counts waits for a frame all the same.
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        a.send(&[0x22; 60]).unwrap();
    });
    assert_eq!(b.recv(Duration::MAX).unwrap().map(|f| f.bytes[0]), Some(0x22));
    sender.join().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn the_raw_link_is_a_wire_and_never_hands_back_a_frame_this_host_sent() {
    use veredas_aranha::netns;
    use veredas_aranha::wire::raw::{Options, RawLink};
    use veredas_aranha::wire::stamp;
    netns::isolated("the_raw_link_is_a_wire_and_never_hands_back_a_frame_this_host_sent", &[("a0", "b0")], || {
        for (iface, mac) in [("a0", A), ("b0", B)] {
            netns::ip(&["link", "set", iface, "down"]);
            netns::ip(&["link", "set", iface, "address", &mac.to_string()]);
            netns::ip(&["link", "set", iface, "up"]);
        }
        netns::wait_pairs(&[("a0", "b0")]);
        let mut a: Box<dyn Wire> = Box::new(RawLink::open("a0", false).unwrap());
        // b0 opened to hear this host's own frames: the seam passes them over all the same.
        let mut b: Box<dyn Wire> = Box::new(RawLink::open_with("b0", Options { ethertype: Some(0x8892), promiscuous: false, outgoing: true }).unwrap());
        let other_on_b = RawLink::open("b0", false).unwrap();
        assert_eq!((a.name(), a.mac(), b.name(), b.mac()), ("a0", Some(A), "b0", Some(B)));

        let mine = dcp::identify_request(B, 0x0B0B, 1, &Filter::All).unwrap();
        other_on_b.send(&mine).unwrap();
        let before = stamp(std::time::SystemTime::now());
        let theirs = dcp::identify_request(A, 0x0A0A, 1, &Filter::All).unwrap();
        a.send(&theirs).unwrap();
        let f = b.recv(Duration::from_secs(5)).unwrap().expect("a0's frame on b0");
        let after = stamp(std::time::SystemTime::now());
        assert_eq!((f.iface.as_str(), &f.bytes, f.length), ("b0", &theirs, theirs.len()));
        assert!(before <= f.at_ns && f.at_ns <= after, "{before} <= {} <= {after}", f.at_ns);
        // Only this host's frame is left on b0: the wait ends empty, at its timeout.
        other_on_b.send(&mine).unwrap();
        let started = Instant::now();
        assert_eq!(b.recv(Duration::from_millis(300)).unwrap(), None);
        assert!(started.elapsed() >= Duration::from_millis(300));
        // Lengths are checked before the socket: the seam's errors, not the link's.
        assert!(matches!(a.send(&[0; 13]), Err(WireError::TooShort(13))));
        assert!(matches!(a.send(&vec![0; 65_537]), Err(WireError::TooLong(65_537))));
        // A frame too long for the link's MTU is the link's own error, carried.
        let err = a.send(&vec![0; 9000]).unwrap_err();
        assert!(matches!(err, WireError::Raw(_)), "{err}");
    });
}
