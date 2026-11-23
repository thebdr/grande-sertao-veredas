//! The cyclic real-time frame of PROFINET IO (contract: the cyclic frame clause): exact bytes both
//! ways, the frame id ranges, the limits refused, the status bits named, every frame read back by
//! Wireshark's dissectors.

mod common;

use veredas_aranha::profinet::ether::{self, VlanTag};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::rt::{self, CyclicFrame, DataStatus, FrameClass, IoxStatus, RtError};

const CONTROLLER: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0C]);
const DEVICE: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0D]);

fn frame(frame_id: u16, data: Vec<u8>, ds: u8) -> CyclicFrame {
    CyclicFrame { frame_id, data, cycle_counter: 0x1234, data_status: DataStatus(ds), transfer_status: 0 }
}

#[test]
fn a_cyclic_frame_is_its_id_its_data_and_its_status_to_the_byte() {
    let data: Vec<u8> = (0..40).collect();
    let f = frame(0x8001, data.clone(), 0x35);
    let mut want = vec![0x80, 0x01];
    want.extend_from_slice(&data);
    want.extend_from_slice(&[0x12, 0x34, 0x35, 0x00]);
    assert_eq!(f.encode().unwrap(), want);
    assert_eq!(CyclicFrame::decode(&want).unwrap(), f);
    // On the wire: tagged priority 6, VLAN 0, as a cyclic frame rides; 64 bytes, no pad.
    let wire = f.frame(DEVICE, CONTROLLER, Some(rt::CYCLIC_TAG)).unwrap();
    assert_eq!(&wire[12..20], &[0x81, 0x00, 0xC0, 0x00, 0x88, 0x92, 0x80, 0x01]);
    assert_eq!(wire.len(), 64);
    let e = ether::parse(&wire).unwrap();
    assert_eq!((e.dst, e.src), (DEVICE, CONTROLLER));
    assert_eq!(&wire[..12], &[2, 0, 0, 0, 0, 0x0D, 2, 0, 0, 0, 0, 0x0C]);
    assert_eq!((e.vlan, CyclicFrame::decode(e.payload).unwrap()), (Some(rt::CYCLIC_TAG), f.clone()));
    // Untagged, 60 bytes exactly; with another tag, that tag; the addresses where they go.
    let untagged = f.frame(DEVICE, CONTROLLER, None).unwrap();
    assert_eq!(untagged.len(), 60);
    assert_eq!(&untagged[..14], &[2, 0, 0, 0, 0, 0x0D, 2, 0, 0, 0, 0, 0x0C, 0x88, 0x92]);
    let other = VlanTag { priority: 5, drop_eligible: true, id: 100 };
    let wire = f.frame(DEVICE, CONTROLLER, Some(other)).unwrap();
    assert_eq!(&wire[12..18], &[0x81, 0x00, 0xB0, 0x64, 0x88, 0x92]);
    let e = ether::parse(&wire).unwrap();
    assert_eq!((e.dst, e.src, e.vlan), (DEVICE, CONTROLLER, Some(other)));
    // The longest data.
    let long = frame(0x0100, vec![0xA5; 1440], 0x35);
    let back = CyclicFrame::decode(&long.encode().unwrap()).unwrap();
    assert_eq!(back, long);
    assert_eq!(long.frame(DEVICE, CONTROLLER, Some(rt::CYCLIC_TAG)).unwrap().len(), 1464);
}

#[test]
fn a_frame_id_is_read_by_its_range() {
    let cases = [
        (0x0000, FrameClass::Other),
        (0x00FF, FrameClass::Other),
        (0x0100, FrameClass::Rtc3),
        (0x0FFF, FrameClass::Rtc3),
        (0x1000, FrameClass::Other),
        (0x7FFF, FrameClass::Other),
        (0x8000, FrameClass::Rtc1),
        (0xBFFF, FrameClass::Rtc1),
        (0xC000, FrameClass::Rtc1Legacy),
        (0xFBFF, FrameClass::Rtc1Legacy),
        (0xFC00, FrameClass::Other),
        (0xFC01, FrameClass::AlarmHigh),
        (0xFC02, FrameClass::Other),
        (0xFDFF, FrameClass::Other),
        (0xFE00, FrameClass::Other),
        (0xFE01, FrameClass::AlarmLow),
        (0xFE02, FrameClass::Other),
        (0xFEFB, FrameClass::Other),
        (0xFEFC, FrameClass::Dcp),
        (0xFEFF, FrameClass::Dcp),
        (0xFF00, FrameClass::Other),
        (0xFFFF, FrameClass::Other),
    ];
    for (id, class) in cases {
        assert_eq!(FrameClass::of(id), class, "{id:#06x}");
        assert_eq!(FrameClass::of(id).is_cyclic(), matches!(class, FrameClass::Rtc3 | FrameClass::Rtc1 | FrameClass::Rtc1Legacy));
    }
}

#[test]
fn what_is_not_a_cyclic_frame_is_refused_both_ways() {
    for id in [0x0000u16, 0x00FF, 0x1000, 0x7FFF, 0xFC01, 0xFE01, 0xFEFE, 0xFFFF] {
        assert_eq!(frame(id, vec![0; 40], 0x35).encode(), Err(RtError::NotCyclic(id)), "{id:#06x}");
        let mut p = id.to_be_bytes().to_vec();
        p.extend_from_slice(&[0; 44]);
        assert_eq!(CyclicFrame::decode(&p), Err(RtError::NotCyclic(id)), "{id:#06x}");
    }
    for len in [0, 39, 1441] {
        assert_eq!(frame(0x8000, vec![0; len], 0x35).encode(), Err(RtError::DataLength(len)));
    }
    let mut short = vec![0x80, 0x00];
    short.extend_from_slice(&[0; 43]);
    assert_eq!(CyclicFrame::decode(&short), Err(RtError::DataLength(39)));
    assert_eq!(CyclicFrame::decode(&[0x80]), Err(RtError::DataLength(0)));
    assert_eq!(CyclicFrame::decode(&[0x80, 0x00, 1, 2]), Err(RtError::DataLength(0)));
    // A frame id and the APDU status with no data between them: no data is no frame.
    assert_eq!(CyclicFrame::decode(&[0x80, 0x00, 0x12, 0x34, 0x35, 0x00]), Err(RtError::DataLength(0)));
    let mut long = vec![0x80, 0x00];
    long.extend_from_slice(&[0; 1445]);
    assert_eq!(CyclicFrame::decode(&long), Err(RtError::DataLength(1441)));
    assert_eq!(RtError::DataLength(39).to_string(), "a cyclic frame's data is 40 to 1440 bytes; this one is 39");
    assert_eq!(RtError::NotCyclic(0xFC01).to_string(), "frame id 0xfc01 is not a cyclic frame's");
    // Any bytes decode or are refused, never a panic.
    let mut noise = common::Noise(0xC1C);
    for _ in 0..20_000 {
        let n = (usize::from(noise.next()) << 8 | usize::from(noise.next())) % 1600;
        let p = noise.bytes(n);
        if let Ok(f) = CyclicFrame::decode(&p) {
            assert_eq!(f.encode().unwrap(), p);
        }
    }
}

#[test]
fn the_status_bits_are_named() {
    assert_eq!(DataStatus::GOOD, DataStatus(0x35));
    let g = DataStatus::GOOD;
    assert_eq!((g.primary(), g.redundancy(), g.valid(), g.running(), g.station_ok(), g.ignore()), (true, false, true, true, true, false));
    let names = |d: DataStatus| [d.primary(), d.redundancy(), d.valid(), d.running(), d.station_ok(), d.ignore()];
    for (bit, at) in [(0u8, 0usize), (1, 1), (2, 2), (4, 3), (5, 4), (7, 5)] {
        let mut want = [false; 6];
        want[at] = true;
        assert_eq!(names(DataStatus(1 << bit)), want, "bit {bit}");
    }
    assert_eq!(names(DataStatus(0x48)), [false; 6], "bits 3 and 6 are reserved");
    assert!(IoxStatus::GOOD.good() && !IoxStatus::BAD.good());
    assert_eq!((IoxStatus::GOOD, IoxStatus::BAD), (IoxStatus(0x80), IoxStatus(0x00)));
    // Each bit alone: good at 7, who decided at 6 and 5, extension at 0; 1 to 4 reserved.
    let iox = |b: u8| (IoxStatus(b).good(), IoxStatus(b).instance(), IoxStatus(b).extension());
    for (b, want) in [
        (0x80, (true, 0, false)),
        (0x60, (false, 3, false)),
        (0x40, (false, 2, false)),
        (0x20, (false, 1, false)),
        (0x01, (false, 0, true)),
        (0x1E, (false, 0, false)),
        (0xE1, (true, 3, true)),
    ] {
        assert_eq!(iox(b), want, "{b:#04x}");
    }
}

#[test]
fn the_dissectors_read_every_frame_as_it_was_written() {
    let data: Vec<u8> = (0..40).collect();
    let frames = [
        frame(0x8001, data.clone(), 0x35).frame(DEVICE, CONTROLLER, Some(rt::CYCLIC_TAG)).unwrap(),
        frame(0x0100, data.clone(), 0x00).frame(DEVICE, CONTROLLER, Some(rt::CYCLIC_TAG)).unwrap(),
        CyclicFrame { frame_id: 0xC000, data: vec![0x80; 1440], cycle_counter: 0xFFFF, data_status: DataStatus(0x86), transfer_status: 0x00 }
            .frame(CONTROLLER, DEVICE, None)
            .unwrap(),
        frame(0x9000, data, 0x34).frame(DEVICE, CONTROLLER, Some(VlanTag { priority: 6, drop_eligible: false, id: 0 })).unwrap(),
    ];
    let read = common::dissect(&frames);
    let r = "pn_rt";
    for (i, (p, f)) in read.iter().zip(&frames).enumerate() {
        assert_eq!(p.complaints(), Vec::<String>::new(), "frame {i}");
        let e = ether::parse(f).unwrap();
        assert_eq!((p.one("eth", "eth.dst"), p.one("eth", "eth.src")), (e.dst.to_string(), e.src.to_string()), "frame {i}");
    }
    // The third was written from the device to the controller, the others the other way.
    assert_eq!(read[2].one("eth", "eth.dst"), CONTROLLER.to_string());
    let ids: Vec<String> = read.iter().map(|p| p.one(r, "pn_rt.frame_id")).collect();
    assert_eq!(ids, ["32769", "256", "49152", "36864"]);
    let counters: Vec<String> = read.iter().map(|p| p.one(r, "pn_rt.cycle_counter")).collect();
    assert_eq!(counters, ["4660", "4660", "65535", "4660"]);
    let ds: Vec<String> = read.iter().map(|p| p.one(r, "pn_rt.ds")).collect();
    assert_eq!(ds, ["53", "0", "134", "52"]);
    let bits = |p: &common::Dissected| -> Vec<String> {
        ["pn_rt.ds_primary", "pn_rt.ds_redundancy", "pn_rt.ds_valid", "pn_rt.ds_operate", "pn_rt.ds_ok", "pn_rt.ds_ignore"]
            .iter()
            .map(|f| p.one(r, f))
            .collect()
    };
    let as_str = |v: [&str; 6]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(bits(&read[0]), as_str(["1", "false", "1", "1", "1", "0"]));
    assert_eq!(bits(&read[1]), as_str(["0", "false", "0", "0", "0", "0"]));
    assert_eq!(bits(&read[2]), as_str(["0", "true", "1", "0", "0", "1"]));
    assert_eq!(bits(&read[3]), as_str(["0", "false", "1", "1", "1", "0"]));
    let ts: Vec<String> = read.iter().map(|p| p.one(r, "pn_rt.transfer_status")).collect();
    assert_eq!(ts, ["0", "0", "0", "0"]);

    // Statuses that set each bit apart from the others, the dissector's reading of each bit held
    // to this codec's accessor; and a frame with another tag.
    let statuses = [0x15u8, 0x25, 0x06, 0x84, 0x35, 0x01];
    let frames: Vec<Vec<u8>> = statuses
        .iter()
        .map(|&ds| frame(0xC001, (0..40).collect(), ds).frame(DEVICE, CONTROLLER, Some(VlanTag { priority: 5, drop_eligible: false, id: 100 })).unwrap())
        .collect();
    let read = common::dissect(&frames);
    let flag = |s: String| s == "1" || s == "true";
    for (p, &ds) in read.iter().zip(&statuses) {
        assert_eq!(p.complaints(), Vec::<String>::new(), "{ds:#04x}");
        let d = DataStatus(ds);
        let theirs: Vec<bool> = ["pn_rt.ds_primary", "pn_rt.ds_redundancy", "pn_rt.ds_valid", "pn_rt.ds_operate", "pn_rt.ds_ok", "pn_rt.ds_ignore"]
            .iter()
            .map(|f| flag(p.one(r, f)))
            .collect();
        assert_eq!(theirs, [d.primary(), d.redundancy(), d.valid(), d.running(), d.station_ok(), d.ignore()], "{ds:#04x}");
        assert_eq!((p.one("vlan", "vlan.priority"), p.one("vlan", "vlan.id")), ("5".into(), "100".into()));
    }
}
