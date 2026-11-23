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
use veredas_aranha::wire::{Frame, Wire, WireError, stamp};

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

/// tshark's reading of a capture file, in one run: each frame's stamp (as it prints it, an ISO
/// date in UTC), length on the wire, bytes kept and the bytes themselves; what it says on its error
/// stream; and whether it dissects the first frame as Ethernet.
struct Reading {
    rows: Vec<(String, String, String, String)>,
    err: String,
    ethernet: bool,
}

fn tshark_full(file: &std::path::Path) -> Reading {
    let out = Command::new(common::tshark()).args(["-n", "-r"]).arg(file).args(["-T", "ek", "-x"]).output().unwrap();
    let packets: Vec<serde_json::Value> =
        String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with("{\"index\"")).map(|l| serde_json::from_str(l).unwrap()).collect();
    let s = |x: &serde_json::Value| x.as_str().unwrap_or_default().to_string();
    let rows = packets
        .iter()
        .map(|v| {
            let frame = &v["layers"]["frame"];
            (s(&frame["frame_frame_time_epoch"]), s(&frame["frame_frame_len"]), s(&frame["frame_frame_cap_len"]), s(&v["layers"]["frame_raw"]))
        })
        .collect();
    let ethernet = packets.first().is_some_and(|v| v["layers"].get("eth").is_some());
    Reading { rows, err: String::from_utf8_lossy(&out.stderr).to_string(), ethernet }
}

fn tshark_reads(file: &std::path::Path) -> (Vec<(String, String, String, String)>, String) {
    let r = tshark_full(file);
    (r.rows, r.err)
}

/// tshark on many files, six at a time (each run starts the dissectors anew).
fn tshark_all(files: &[PathBuf]) -> Vec<Reading> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<Reading>>> = Mutex::new((0..files.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= files.len() {
                        break;
                    }
                    let r = tshark_full(&files[i]);
                    out.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(Option::unwrap).collect()
}

/// Write each `(name, bytes)` under `dir`, and read them all with tshark.
fn tshark_files(dir: &std::path::Path, files: &[(String, Vec<u8>)]) -> Vec<Reading> {
    let paths: Vec<PathBuf> = files
        .iter()
        .map(|(name, bytes)| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .collect();
    tshark_all(&paths)
}

/// A stamp as tshark's ek output prints it: `1970-01-01T00:00:01.000005000Z`.
fn iso(at_ns: u64) -> String {
    let (secs, ns) = (at_ns / 1_000_000_000, at_ns % 1_000_000_000);
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since the epoch to a civil date (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ns:09}Z", rest / 3600, rest / 60 % 60, rest % 60)
}

/// Our reading of a capture in tshark's terms.
fn as_tshark(frames: &[Frame]) -> Vec<(String, String, String, String)> {
    frames.iter().map(|f| (iso(f.at_ns), f.length.to_string(), f.bytes.len().to_string(), to_hex(&f.bytes))).collect()
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
            magic: pcap::MAGIC_NANOS,
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
    assert_eq!(rows, as_tshark(&frames));
    assert_eq!(rows[3].0, "2106-02-07T06:28:15.999999999Z");
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

/// A capture written by hand: the header's fields as given, in the byte order given, then records
/// of seconds, fraction, the two length fields in the order given, and bytes; the modified
/// format's records carry eight bytes more of header (an interface index, a protocol, a type).
fn raw_file(big: bool, magic: u32, version: (u16, u16), linktype: u32, records: &[(u32, u32, u32, u32, &[u8])]) -> Vec<u8> {
    let w32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let w16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut out = Vec::new();
    out.extend_from_slice(&w32(magic));
    out.extend_from_slice(&w16(version.0));
    out.extend_from_slice(&w16(version.1));
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&w32(96));
    out.extend_from_slice(&w32(linktype));
    for (secs, fraction, first, second, bytes) in records {
        out.extend_from_slice(&w32(*secs));
        out.extend_from_slice(&w32(*fraction));
        out.extend_from_slice(&w32(*first));
        out.extend_from_slice(&w32(*second));
        if magic == pcap::MAGIC_MODIFIED {
            out.extend_from_slice(&[0, 0, 0, 2, 0x08, 0x00, 0x00, 0x00]);
        }
        out.extend_from_slice(bytes);
    }
    out
}

/// The same, each record's bytes kept first and its length on the wire second (version 2.4's
/// order).
fn by_hand(big: bool, magic: u32, version: (u16, u16), linktype: u32, records: &[(u32, u32, &[u8], u32)]) -> Vec<u8> {
    let records: Vec<_> = records.iter().map(|(s, f, b, l)| (*s, *f, b.len() as u32, *l, *b)).collect();
    raw_file(big, magic, version, linktype, &records)
}

/// Write `bytes` as `name` under `dir` and read it with tshark.
fn tshark_on(dir: &std::path::Path, name: &str, bytes: &[u8]) -> (Vec<(String, String, String, String)>, String) {
    let file = dir.join(name);
    std::fs::write(&file, bytes).unwrap();
    tshark_reads(&file)
}

#[test]
fn every_version_magic_byte_order_and_record_layout_is_read_as_tshark_reads_it() {
    let dir = folder("matrix");
    let f = frames();
    let (a, b) = (&f[0].bytes[..], &f[2].bytes[..]);
    let mut cases = Vec::new();
    for version in [(2, 0), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (3, 0)] {
        for magic in [pcap::MAGIC_MICROS, pcap::MAGIC_NANOS, pcap::MAGIC_MODIFIED] {
            for big in [false, true] {
                if big && ![(2, 2), (2, 3), (2, 4)].contains(&version) {
                    continue;
                }
                // Two frames cut by their capture and a whole one between, the cut ones written in
                // version 2.4's order (kept first) or the older one (on the wire first).
                for old in [false, true] {
                    let lengths = |kept: usize, wire: u32| if old { (wire, kept as u32) } else { (kept as u32, wire) };
                    let (l1, l2, l3) = (lengths(a.len(), 1514), lengths(b.len(), b.len() as u32), lengths(a.len(), 1500));
                    let records = [(1, 1, l1.0, l1.1, a), (2, 999_999, l2.0, l2.1, b), (3, 5, l3.0, l3.1, a)];
                    cases.push((version, magic, big, old, raw_file(big, magic, version, 1, &records)));
                }
            }
        }
    }
    assert_eq!(cases.len(), 60);
    let files: Vec<(String, Vec<u8>)> = cases.iter().enumerate().map(|(i, c)| (format!("{i}.pcap"), c.4.clone())).collect();
    let readings = tshark_files(&dir, &files);
    for ((version, magic, big, old, bytes), tshark) in cases.iter().zip(&readings) {
        let (version, magic, big, old) = (*version, *magic, *big, *old);
        let read = pcap::read(bytes, "m").unwrap();
        let what = format!("{version:?} {magic:#x} big {big} old {old}");
        assert_eq!(tshark.rows, as_tshark(&read.frames), "{what}");
        assert_eq!(read.damage.is_some(), !tshark.err.is_empty(), "{what}: {:?} / {}", read.damage, tshark.err);
        // What each layout is, by version: the old one before 2.3, either at 2.3, the new one
        // after; stamps in nanoseconds for the nanosecond magic from 2.4.
        let readable = if version < (2, 3) { old } else if version == (2, 3) { true } else { !old };
        assert_eq!(read.frames.len() == 3 && read.damage.is_none(), readable, "{what}");
        if readable {
            assert_eq!(read.frames.iter().map(|f| (f.bytes.len(), f.length)).collect::<Vec<_>>(), [(a.len(), 1514), (b.len(), b.len()), (a.len(), 1500)], "{what}");
        }
        let nanos = magic == pcap::MAGIC_NANOS && version >= (2, 4);
        assert_eq!((read.magic, read.nanos, read.big_endian), (magic, nanos, big), "{what}");
        if readable {
            assert_eq!(read.frames[0].at_ns, 1_000_000_000 + if nanos { 1 } else { 1000 }, "{what}");
        }
    }
}

#[test]
fn the_link_type_and_the_aix_format_are_read_as_tshark_reads_them() {
    let dir = folder("link");
    let a = frames().remove(0).bytes;
    let fcs = [&a[..], &[0xA8, 0x24, 0x72, 0xD3]].concat();
    let file = |big: bool, magic: u32, version: (u16, u16), lt: u32, frame: &[u8]| by_hand(big, magic, version, lt, &[(1, 5, frame, frame.len() as u32)]);
    // Ethernet whatever bits 26 to 31 say (a check sequence's flag and length), the frame kept as
    // recorded, the field whole: tshark reads each as Ethernet, with no complaint.
    let ethernet = [0x0000_0001u32, 0x0400_0001, 0x0800_0001, 0x0C00_0001, 0x1000_0001, 0x2400_0001, 0x4400_0001, 0x5000_0001, 0xF000_0001, 0xFC00_0001];
    // A reserved bit (16 to 25), refused as tshark refuses it; another link type, none of which
    // tshark reads as Ethernet.
    let reserved = [0x0001_0001u32, 0x0100_0001, 0x0200_0001, 0x03FF_0001];
    let other = [0u32, 6, 101, 105, 0x0101, 0x8001];
    // AIX's format (2.2, the microsecond magic, type 6) in either byte order, and four near it.
    let not_aix = [(pcap::MAGIC_NANOS, (2, 2), 6), (pcap::MAGIC_MICROS, (2, 4), 6), (pcap::MAGIC_MICROS, (2, 2), 9), (pcap::MAGIC_MODIFIED, (2, 2), 6)];
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for lt in ethernet {
        files.push((format!("e{lt:x}.pcap"), file(false, pcap::MAGIC_MICROS, (2, 4), lt, if lt & 0x0400_0000 != 0 { &fcs } else { &a })));
    }
    for lt in reserved.iter().chain(&other) {
        files.push((format!("r{lt:x}.pcap"), file(false, pcap::MAGIC_MICROS, (2, 4), *lt, &a)));
    }
    for big in [false, true] {
        files.push((format!("aix-{big}.pcap"), file(big, pcap::MAGIC_MICROS, (2, 2), 6, &a)));
    }
    for (i, (magic, version, lt)) in not_aix.iter().enumerate() {
        files.push((format!("not-aix-{i}.pcap"), file(false, *magic, *version, *lt, &a)));
    }
    let readings = tshark_files(&dir, &files);
    let mut k = 0;
    for lt in ethernet {
        let read = pcap::read(&files[k].1, "l").unwrap();
        let frame = if lt & 0x0400_0000 != 0 { &fcs } else { &a };
        assert_eq!((read.linktype, &read.frames[0].bytes), (lt, frame), "{lt:#x}");
        assert_eq!((&readings[k].rows, readings[k].err.as_str(), readings[k].ethernet), (&as_tshark(&read.frames), "", true), "{lt:#x}");
        k += 1;
    }
    for lt in reserved {
        let err = pcap::read(&files[k].1, "l").unwrap_err();
        assert!(matches!(err, PcapError::Reserved(x) if x == lt), "{lt:#x}: {err}");
        assert!(readings[k].err.contains("reserved field not zero"), "{lt:#x}: {}", readings[k].err);
        k += 1;
    }
    assert_eq!(PcapError::Reserved(0x0100_0001).to_string(), "link type 0x1000001 has reserved bits set (bits 16 to 25)");
    for lt in other {
        let err = pcap::read(&files[k].1, "l").unwrap_err();
        assert!(matches!(err, PcapError::NotEthernet(x) if x == lt), "{lt:#x}: {err}");
        assert!(!readings[k].ethernet, "{lt:#x}");
        k += 1;
    }
    assert_eq!(pcap::read(&by_hand(true, pcap::MAGIC_MICROS, (2, 4), 101, &[]), "l").unwrap_err().to_string(), "link type 0x65 is not Ethernet (1)");
    for big in [false, true] {
        let read = pcap::read(&files[k].1, "l").unwrap();
        assert_eq!((read.nanos, read.linktype, read.frames[0].at_ns), (true, 6, 1_000_000_005), "{big}");
        assert_eq!((&readings[k].rows, readings[k].err.as_str(), readings[k].ethernet), (&as_tshark(&read.frames), "", true), "{big}");
        k += 1;
    }
    for (magic, version, lt) in not_aix {
        assert!(matches!(pcap::read(&files[k].1, "l"), Err(PcapError::NotEthernet(x)) if x == lt), "{magic:#x} {version:?} {lt}");
        assert!(!readings[k].ethernet, "{magic:#x} {version:?} {lt}");
        k += 1;
    }
    assert_eq!(k, files.len());
}

#[test]
fn where_the_reader_departs_from_tshark_it_keeps_what_the_file_states() {
    let dir = folder("departures");
    let a = frames().remove(0).bytes;
    // A fraction past a second is carried into the next, at either resolution; tshark keeps it as
    // an invalid fraction (printing "1.1500000000") and wraps it past 2^31 nanoseconds. A fraction
    // under a second reads alike.
    let fractions = [(pcap::MAGIC_MICROS, 1_000_000u32), (pcap::MAGIC_MICROS, 2_147_484), (pcap::MAGIC_NANOS, 1_500_000_000), (pcap::MAGIC_NANOS, u32::MAX)];
    let alike = [(pcap::MAGIC_MICROS, 999_999u32), (pcap::MAGIC_NANOS, 999_999_999)];
    let files: Vec<(String, Vec<u8>)> = fractions.iter().chain(&alike).enumerate().map(|(i, (m, fr))| (format!("f{i}.pcap"), by_hand(false, *m, (2, 4), 1, &[(1, *fr, &a, 60)]))).collect();
    let readings = tshark_files(&dir, &files);
    for (i, (magic, fraction)) in fractions.iter().chain(&alike).enumerate() {
        let read = pcap::read(&files[i].1, "d").unwrap();
        let per = if *magic == pcap::MAGIC_NANOS { 1 } else { 1000 };
        assert_eq!(read.frames[0].at_ns, 1_000_000_000 + u64::from(*fraction) * per, "{fraction}");
        assert_eq!(readings[i].rows == as_tshark(&read.frames), i >= fractions.len(), "{fraction}: {:?}", readings[i].rows);
    }
    // A length on the wire past 2 147 483 647 is kept as stated; tshark shows 2 147 483 647.
    let lengths = [0x7FFF_FFFFu32, 0x8000_0000, u32::MAX];
    let bytes = by_hand(false, pcap::MAGIC_NANOS, (2, 4), 1, &[(1, 0, &a, lengths[0]), (1, 0, &a, lengths[1]), (1, 0, &a, lengths[2])]);
    let read = pcap::read(&bytes, "d").unwrap();
    assert_eq!(read.frames.iter().map(|f| f.length).collect::<Vec<_>>(), lengths.map(|l| l as usize));
    let (rows, _) = tshark_on(&dir, "lengths.pcap", &bytes);
    assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["2147483647"; 3]);
    // The same capture written by us states the same lengths again.
    assert_eq!(pcap::read(&pcap::write(&read.frames).unwrap(), "d").unwrap().frames, read.frames);
}

#[test]
fn a_stamp_is_nanoseconds_since_the_epoch_saturating_at_either_end() {
    use std::time::SystemTime;
    let epoch = SystemTime::UNIX_EPOCH;
    assert_eq!(stamp(epoch), 0);
    assert_eq!(stamp(epoch + Duration::new(1, 500_000_000)), 1_500_000_000);
    assert_eq!(stamp(epoch + Duration::new(1_700_000_000, 123_456_789)), 1_700_000_000_123_456_789);
    assert_eq!(stamp(epoch - Duration::from_nanos(1)), 0);
    assert_eq!(stamp(epoch - Duration::from_secs(86_400)), 0);
    assert_eq!(stamp(epoch + Duration::new(18_446_744_073, 709_551_615)), u64::MAX);
    assert_eq!(stamp(epoch + Duration::new(18_446_744_073, 709_551_614)), u64::MAX - 1);
    assert_eq!(stamp(epoch + Duration::new(18_446_744_073, 709_551_616)), u64::MAX);
    assert_eq!(stamp(epoch + Duration::from_secs(20_000_000_000)), u64::MAX);
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
            assert_eq!(rows, as_tshark(&read.frames), "big {big}, nanos {nanos}");
        }
    }
    // The snapshot length is not held against the records (tshark reads a 60-byte frame under 20).
    let mut small = by_hand(false, pcap::MAGIC_NANOS, (2, 4), 1, &[(1, 0, a, a.len() as u32)]);
    small[16..20].copy_from_slice(&20u32.to_le_bytes());
    let read = pcap::read(&small, "p1").unwrap();
    assert_eq!((read.snaplen, read.frames.len(), read.frames[0].bytes.len()), (20, 1, a.len()));
    // Major versions 2 and later are read (every minor), 1 and 0 refused (tshark: "major version 1
    // unsupported").
    for v in [(2, 0xFFFF), (3, 3), (0xFFFF, 0)] {
        assert_eq!(pcap::read(&by_hand(true, pcap::MAGIC_MICROS, v, 1, &[(1, 0, a, 60)]), "p1").unwrap().version, v);
    }
    for v in [(1, 0), (0, 9), (1, 0xFFFF)] {
        let err = pcap::read(&by_hand(false, pcap::MAGIC_MICROS, v, 1, &[]), "p1").unwrap_err();
        assert!(matches!(err, PcapError::Version(x, y) if (x, y) == v), "{err}");
    }
    assert_eq!(pcap::read(&by_hand(false, pcap::MAGIC_MICROS, (1, 0), 1, &[]), "p1").unwrap_err().to_string(), "pcap version 1.0 is not read: major version 2 or later");
    // Another magic number is refused by name.
    for magic in [0xD4C3_B2A0, 0, 0x0A0D_0D0B, 0xA1B2_3C4E, 0x34CD_B2A0] {
        let err = pcap::read(&by_hand(false, magic, (2, 4), 1, &[]), "p1").unwrap_err();
        assert!(matches!(err, PcapError::Magic(_)), "{magic:#x}: {err}");
    }
    assert_eq!(
        pcap::read(&by_hand(true, 0xA1B2_3C4E, (2, 4), 1, &[]), "p1").unwrap_err().to_string(),
        "magic number 0xa1b23c4e is not classic pcap's (a1b2c3d4, a1b23c4d or a1b2cd34, in either byte order)"
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
    assert_eq!(read.damage, Some(Damage { record: 2, offset: starts[2], kind: DamageKind::HeaderCut { have: 9, need: 16 } }));
    assert_eq!(read.damage.unwrap().to_string(), format!("record 2 at byte {} is cut short in its header: 9 of 16 bytes", starts[2]));
    // A modified capture's record header is 24 bytes: cut at 20 of them.
    let modified = raw_file(false, pcap::MAGIC_MODIFIED, (2, 4), 1, &[(1, 0, 60, 60, &frames[0].bytes[..60]), (2, 0, 60, 60, &frames[0].bytes[..60])]);
    let cut_modified = &modified[..24 + 24 + 60 + 20];
    let read = pcap::read(cut_modified, "b0").unwrap();
    assert_eq!(read.frames.len(), 1);
    assert_eq!(read.damage, Some(Damage { record: 1, offset: 108, kind: DamageKind::HeaderCut { have: 20, need: 24 } }));
    assert_eq!(read.damage.unwrap().to_string(), "record 1 at byte 108 is cut short in its header: 20 of 24 bytes");
    let (rows, err) = tshark_on(&dir, "modified-cut.pcap", cut_modified);
    assert_eq!((rows.len(), err.contains("cut short")), (1, true), "{err}");
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
    assert!(started.elapsed() < Duration::from_secs(1), "a capture never waits: {:?}", started.elapsed());
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
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(50) && waited < Duration::from_millis(500), "{waited:?}");
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
    use veredas_aranha::wire::raw::{Options, RawError, RawLink};
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
        // This host's frames every 20 ms for two seconds: passing them over keeps one deadline,
        // the wait ending empty at its timeout, not when they stop.
        std::thread::scope(|s| {
            s.spawn(|| {
                let until = Instant::now() + Duration::from_secs(2);
                while Instant::now() < until {
                    let _ = other_on_b.send(&mine);
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
            std::thread::sleep(Duration::from_millis(100));
            let started = Instant::now();
            assert_eq!(b.recv(Duration::from_millis(300)).unwrap(), None);
            let waited = started.elapsed();
            assert!(waited >= Duration::from_millis(300) && waited < Duration::from_millis(1500), "{waited:?}");
        });
        // Lengths are checked before the socket: the seam's errors, not the link's.
        assert!(matches!(a.send(&[0; 13]), Err(WireError::TooShort(13))));
        assert!(matches!(a.send(&vec![0; 65_537]), Err(WireError::TooLong(65_537))));
        // A frame too long for the link's MTU is the link's own error, carried.
        let err = a.send(&vec![0; 9000]).unwrap_err();
        assert!(matches!(err, WireError::Raw(RawError::TooLong(9000))), "{err}");
        assert_eq!(err.to_string(), "a frame of 9000 bytes is past what the link sends");
    });
}
