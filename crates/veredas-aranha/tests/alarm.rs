//! The alarm frame of PROFINET IO (contract: the alarm frame clause): exact bytes both ways, the
//! bits named, what is not read kept with its reason, what would read back as another refused,
//! every frame read back by Wireshark's dissectors.

mod common;

use serde_json::Value;
use veredas_aranha::profinet::alarm::{
    self, AddFlags, AlarmAck, AlarmError, AlarmItem, AlarmSpecifier, Body, Notification, PduType, PnioStatus, Priority, RtaPdu,
    Unread,
};
use veredas_aranha::profinet::ether::{self, VlanTag};
use veredas_aranha::profinet::mac::Mac;

const CONTROLLER: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0C]);
const DEVICE: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0D]);

/// A diagnosis alarm on slot 1, subslot 0x8001: sequence 5, channel, submodule and AR diagnosis.
fn diagnosis(item: Option<AlarmItem>) -> Notification {
    Notification {
        alarm_type: 1,
        api: 0,
        slot: 1,
        subslot: 0x8001,
        module_ident: 0x20,
        submodule_ident: 1,
        specifier: AlarmSpecifier(0xA805),
        item,
    }
}

/// A channel diagnosis: channel 0x8000, properties 0x0800 (appears), error type 1 (short circuit).
fn channel() -> AlarmItem {
    AlarmItem { usi: 0x8000, data: vec![0x80, 0x00, 0x08, 0x00, 0x00, 0x01] }
}

fn ack() -> AlarmAck {
    AlarmAck { alarm_type: 1, api: 0, slot: 1, subslot: 0x8001, specifier: AlarmSpecifier(0xA805), status: PnioStatus::OK }
}

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn pdu(pdu_type: PduType, add_flags: u8, body: Body) -> RtaPdu {
    RtaPdu { priority: Priority::High, dst_endpoint: 1, src_endpoint: 2, pdu_type, version: 1, add_flags: AddFlags(add_flags), send_seq: 7, ack_seq: 6, body }
}

#[test]
fn a_notification_is_its_header_and_its_block_to_the_byte() {
    let n = RtaPdu::notification(Priority::High, 0x0001, 0x0002, 0xFFFF, 0xFFFE, diagnosis(Some(channel())));
    let want = hex(
        "FC01 0001 0002 11 11 FFFF FFFE 0022 \
         0001 001E 0100 0001 00000000 0001 8001 00000020 00000001 A805 8000 800008000001",
    );
    assert_eq!(n.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), n);
    // On the wire: from the device to the controller, tagged priority 6, VLAN 0; 66 bytes, no pad.
    let wire = n.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap();
    assert_eq!(&wire[..20], &hex("02000000000C 02000000000D 8100 C000 8892 FC01")[..]);
    assert_eq!(wire.len(), 66);
    let e = ether::parse(&wire).unwrap();
    assert_eq!((e.dst, e.src, e.vlan), (CONTROLLER, DEVICE, Some(Priority::High.tag())));
    assert_eq!(RtaPdu::decode(e.payload).unwrap(), n);

    // Low priority: frame id 0xFE01, block type 0x0002, tag priority 5; no item, the block ends at
    // the specifier; the frame padded to 60 and the pad not kept.
    let low = RtaPdu::notification(Priority::Low, 0x0001, 0x0002, 0x0000, 0xFFFF, diagnosis(None));
    let want = hex("FE01 0001 0002 11 11 0000 FFFF 001A 0002 0016 0100 0001 00000000 0001 8001 00000020 00000001 A805");
    assert_eq!(low.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), low);
    let wire = low.frame(CONTROLLER, DEVICE, Some(Priority::Low.tag())).unwrap();
    assert_eq!(&wire[..20], &hex("02000000000C 02000000000D 8100 A000 8892 FE01")[..]);
    assert_eq!(wire.len(), 60);
    let e = ether::parse(&wire).unwrap();
    assert_eq!(e.payload.len(), 42);
    assert_eq!(RtaPdu::decode(e.payload).unwrap(), low);
    // An item with no data: its identifier alone.
    let bare = RtaPdu::notification(Priority::High, 1, 2, 0, 0, diagnosis(Some(AlarmItem { usi: 0x0042, data: vec![] })));
    let bytes = bare.encode().unwrap();
    assert_eq!(&bytes[12..20], &hex("001C 0001 0018 0100")[..]);
    assert_eq!(&bytes[bytes.len() - 2..], &[0x00, 0x42]);
    assert_eq!(RtaPdu::decode(&bytes).unwrap(), bare);
    // Untagged: 60 bytes exactly, the addresses where they go.
    let untagged = low.frame(CONTROLLER, DEVICE, None).unwrap();
    assert_eq!(untagged.len(), 60);
    assert_eq!(&untagged[..16], &hex("02000000000C 02000000000D 8892 FE01")[..]);
}

#[test]
fn an_alarm_ack_and_the_transport_s_pdus_to_the_byte() {
    let a = RtaPdu::alarm_ack(Priority::High, 0x0002, 0x0001, 0x0000, 0xFFFF, ack());
    let want = hex("FC01 0002 0001 11 11 0000 FFFF 0016 8001 0012 0100 0001 00000000 0001 8001 A805 00000000");
    assert_eq!(a.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), a);
    let low = RtaPdu::alarm_ack(Priority::Low, 2, 1, 0, 0xFFFF, AlarmAck { status: PnioStatus { code: 0xDA, decode: 0x81, code1: 0x3C, code2: 0x01 }, ..ack() });
    let bytes = low.encode().unwrap();
    assert_eq!(&bytes[..2], &[0xFE, 0x01]);
    assert_eq!(&bytes[14..16], &[0x80, 0x02]);
    assert_eq!(&bytes[bytes.len() - 4..], &[0xDA, 0x81, 0x3C, 0x01]);
    assert_eq!(RtaPdu::decode(&bytes).unwrap(), low);

    let t = RtaPdu::transport_ack(Priority::High, 0x0001, 0x0002, 0xFFFF, 0x0000);
    let want = hex("FC01 0001 0002 13 01 FFFF 0000 0000");
    assert_eq!(t.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), t);
    assert_eq!(t.body, Body::Empty);

    let err = RtaPdu::error(Priority::Low, 0x0001, 0x0002, 0xFFFF, 0xFFFE, PnioStatus { code: 0xCF, decode: 0x81, code1: 0xFD, code2: 0x05 });
    let want = hex("FE01 0001 0002 14 01 FFFF FFFE 0004 CF81FD05");
    assert_eq!(err.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), err);

    let nack = pdu(PduType::Nack, 0x01, Body::Empty);
    let want = hex("FC01 0001 0002 12 01 0007 0006 0000");
    assert_eq!(nack.encode().unwrap(), want);
    assert_eq!(RtaPdu::decode(&want).unwrap(), nack);
    // The pad of a short frame is not kept.
    let mut padded = want.clone();
    padded.extend_from_slice(&[0xEE; 30]);
    assert_eq!(RtaPdu::decode(&padded).unwrap(), nack);
}

#[test]
fn the_bits_are_named() {
    assert_eq!((Priority::High.frame_id(), Priority::Low.frame_id()), (0xFC01, 0xFE01));
    assert_eq!((alarm::FRAME_ID_HIGH, alarm::FRAME_ID_LOW), (0xFC01, 0xFE01));
    assert_eq!((Priority::of_frame_id(0xFC01), Priority::of_frame_id(0xFE01)), (Some(Priority::High), Some(Priority::Low)));
    for other in [0x0000, 0x8001, 0xFC02, 0xFC41, 0xFE02, 0xFE41, 0xFEFE, 0xFFFF] {
        assert_eq!(Priority::of_frame_id(other), None, "{other:#06x}");
    }
    assert_eq!(Priority::High.tag(), VlanTag { priority: 6, drop_eligible: false, id: 0 });
    assert_eq!(Priority::Low.tag(), VlanTag { priority: 5, drop_eligible: false, id: 0 });
    assert_eq!((Priority::High.notification_block(), Priority::Low.notification_block()), (0x0001, 0x0002));
    assert_eq!((Priority::High.ack_block(), Priority::Low.ack_block()), (0x8001, 0x8002));
    assert_eq!((alarm::HEADER, alarm::MAX_VAR_PART), (12, 1486));

    assert_eq!((AddFlags::new(true), AddFlags::new(false)), (AddFlags(0x11), AddFlags(0x01)));
    for (byte, window, tack) in [(0x01, 1, false), (0x11, 1, true), (0x10, 0, true), (0x0F, 15, false), (0xE1, 1, false), (0xF1, 1, true)] {
        assert_eq!((AddFlags(byte).window_size(), AddFlags(byte).tack()), (window, tack), "{byte:#04x}");
    }

    let s = |v: u16| {
        let a = AlarmSpecifier(v);
        (a.sequence(), a.channel_diagnosis(), a.manufacturer_diagnosis(), a.submodule_diagnosis(), a.ar_diagnosis())
    };
    assert_eq!(s(0x0000), (0, false, false, false, false));
    assert_eq!(s(0x07FF), (2047, false, false, false, false));
    assert_eq!(s(0x0800), (0, true, false, false, false));
    assert_eq!(s(0x1000), (0, false, true, false, false));
    assert_eq!(s(0x2000), (0, false, false, true, false));
    assert_eq!(s(0x4000), (0, false, false, false, false));
    assert_eq!(s(0x8000), (0, false, false, false, true));
    assert_eq!(s(0xA805), (5, true, false, true, true));

    assert_eq!(PnioStatus::OK.to_bytes(), [0, 0, 0, 0]);
    let st = PnioStatus::from_bytes([0xDA, 0x81, 0x3C, 0x01]);
    assert_eq!((st.code, st.decode, st.code1, st.code2), (0xDA, 0x81, 0x3C, 0x01));
    assert_eq!(st.to_bytes(), [0xDA, 0x81, 0x3C, 0x01]);

    for (n, t) in [(1, PduType::Data), (2, PduType::Nack), (3, PduType::Ack), (4, PduType::Err), (0, PduType::Other(0)), (5, PduType::Other(5)), (15, PduType::Other(15))] {
        assert_eq!(t.nibble(), n);
        let bytes = [&[0xFC, 0x01, 0, 1, 0, 2, 0x10 | n, 1, 0, 0, 0, 0, 0, 0][..]].concat();
        assert_eq!(RtaPdu::decode(&bytes).unwrap().pdu_type, t, "{n}");
    }
}

#[test]
fn what_is_not_read_is_kept_with_its_reason() {
    let header = |t: u8, var: &[u8], fid: u16| {
        let mut b = fid.to_be_bytes().to_vec();
        b.extend_from_slice(&[0, 1, 0, 2, 0x10 | t, 0x11, 0, 7, 0, 6]);
        b.extend_from_slice(&(var.len() as u16).to_be_bytes());
        b.extend_from_slice(var);
        b
    };
    let good = RtaPdu::notification(Priority::High, 1, 2, 7, 6, diagnosis(None)).encode().unwrap();
    let var = good[14..].to_vec();
    let with = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut v = var.clone();
        f(&mut v);
        v
    };
    let ack_var = RtaPdu::alarm_ack(Priority::High, 1, 2, 7, 6, ack()).encode().unwrap()[14..].to_vec();
    let cases: Vec<(u8, Vec<u8>, u16, Unread)> = vec![
        (1, vec![], 0xFC01, Unread::Shape),
        (2, vec![0], 0xFC01, Unread::Shape),
        (3, vec![1, 2, 3, 4], 0xFC01, Unread::Shape),
        (4, vec![0xCF, 0x81, 0xFD], 0xFC01, Unread::Shape),
        (4, vec![0xCF, 0x81, 0xFD, 0x05, 0x00], 0xFC01, Unread::Shape),
        (4, vec![], 0xFC01, Unread::Shape),
        (0, var.clone(), 0xFC01, Unread::Type),
        (5, vec![1, 2], 0xFC01, Unread::Type),
        (15, vec![], 0xFC01, Unread::Type),
        (1, vec![0, 1, 0, 0, 1], 0xFC01, Unread::Length),
        (1, with(&|v| v[1] = 0x09), 0xFC01, Unread::Block(0x0009)),
        // A notification's fields under the acknowledgement's type: too long for it.
        (1, with(&|v| v[0] = 0x80), 0xFC01, Unread::Length),
        (1, with(&|v| v[0] = 0x40), 0xFC01, Unread::Block(0x4001)),
        (1, with(&|v| v[1] = 0x02), 0xFC01, Unread::Priority(0x0002)),
        (1, var.clone(), 0xFE01, Unread::Priority(0x0001)),
        (1, ack_var.clone(), 0xFE01, Unread::Priority(0x8001)),
        (1, with(&|v| v[4] = 2), 0xFC01, Unread::Version(2, 0)),
        (1, with(&|v| v[5] = 1), 0xFC01, Unread::Version(1, 1)),
        (1, with(&|v| v[4] = 0), 0xFC01, Unread::Version(0, 0)),
        (1, with(&|v| v[3] += 1), 0xFC01, Unread::Length),
        (1, with(&|v| v[3] -= 1), 0xFC01, Unread::Length),
        (1, with(&|v| v.truncate(v.len() - 1)), 0xFC01, Unread::Length),
        (1, with(&|v| {
            v.push(0x80);
            v[3] += 1;
        }), 0xFC01, Unread::Length),
        (1, with(&|v| {
            v.truncate(v.len() - 2);
            v[3] -= 2;
        }), 0xFC01, Unread::Length),
        (1, {
            let mut v = ack_var.clone();
            v.push(0);
            v[3] += 1;
            v
        }, 0xFC01, Unread::Length),
        (1, {
            let mut v = ack_var.clone();
            v.pop();
            v[3] -= 1;
            v
        }, 0xFC01, Unread::Length),
    ];
    for (t, v, fid, why) in &cases {
        let bytes = header(*t, v, *fid);
        let got = RtaPdu::decode(&bytes).unwrap();
        assert_eq!(got.body, Body::Unread { bytes: v.clone(), why: why.clone() }, "type {t}, {v:02x?}");
        // Written back as heard.
        assert_eq!(got.encode().unwrap(), bytes, "type {t}, {v:02x?}");
    }
    assert_eq!(cases.len(), 26);
    // The reasons' words.
    assert_eq!(Unread::Type.text(), "a PDU type this codec does not read");
    assert_eq!(Unread::Shape.text(), "a var part that does not fit its PDU type");
    assert_eq!(Unread::Block(9).text(), "block type 0x0009 is neither an alarm notification nor an acknowledgement");
    assert_eq!(Unread::Priority(2).text(), "block type 0x0002 is of the other priority than its frame");
    assert_eq!(Unread::Version(2, 0).text(), "block version 2.0 is not 1.0");
    assert_eq!(Unread::Length.text(), "a block whose length does not fit");

    // A damaged header is an error naming it.
    assert_eq!(RtaPdu::decode(&good[..13]), Err(AlarmError::TooShort(13)));
    assert_eq!(RtaPdu::decode(&[]), Err(AlarmError::TooShort(0)));
    let mut not = good.clone();
    not[1] = 0x02;
    assert_eq!(RtaPdu::decode(&not), Err(AlarmError::NotAlarm(0xFC02)));
    assert_eq!(RtaPdu::decode(&good[..good.len() - 1]), Err(AlarmError::Overrun { declared: 26, available: 25 }));
    assert_eq!(AlarmError::TooShort(13).to_string(), "an RTA PDU is a frame id and 12 header bytes; this one is 13 bytes");
    assert_eq!(AlarmError::NotAlarm(0xFC02).to_string(), "frame id 0xfc02 is not an alarm frame's");
    // An alarm frame with PROFINET security is named as one, not as damage (tshark reads it as a
    // secured alarm); its neighbours are not alarm frames.
    for (id, secured) in [(0xFC41u16, true), (0xFE41, true), (0xFC40, false), (0xFC42, false), (0xFE40, false), (0xFE42, false)] {
        let mut f = good.clone();
        f[..2].copy_from_slice(&id.to_be_bytes());
        let want = if secured { AlarmError::Secured(id) } else { AlarmError::NotAlarm(id) };
        assert_eq!(RtaPdu::decode(&f), Err(want), "{id:#06x}");
    }
    assert_eq!((alarm::FRAME_ID_HIGH_SECURED, alarm::FRAME_ID_LOW_SECURED), (0xFC41, 0xFE41));
    assert_eq!(AlarmError::Secured(0xFC41).to_string(), "frame id 0xfc41 is an alarm frame with PROFINET security, which this codec does not read");
    assert_eq!(AlarmError::Overrun { declared: 26, available: 25 }.to_string(), "the var part is said to be 26 bytes and 25 follow the header");
    // A var part past what a frame carries is refused on reading as on writing, typed or not,
    // whether its bytes are there or not.
    for t in [5u8, 1] {
        let mut b = header(t, &[0xAB; 1487], 0xFC01);
        if t == 1 {
            b[14..20].copy_from_slice(&[0x00, 0x01, 0x05, 0xCB, 1, 0]);
        }
        assert_eq!(RtaPdu::decode(&b), Err(AlarmError::TooLong(1487)), "type {t}");
    }
    let mut big = header(5, &[], 0xFC01);
    big[12..14].copy_from_slice(&[0xFF, 0xFF]);
    assert_eq!(RtaPdu::decode(&big), Err(AlarmError::TooLong(65_535)));
    let longest = header(5, &[0xAB; 1486], 0xFE01);
    assert_eq!(RtaPdu::decode(&longest).unwrap().encode().unwrap(), longest);

    // Every cut of every written PDU decodes or names its damage, never a panic.
    let mut noise = common::Noise(0xA1A2);
    let written: Vec<Vec<u8>> = vec![
        good.clone(),
        RtaPdu::notification(Priority::Low, 1, 2, 7, 6, diagnosis(Some(channel()))).encode().unwrap(),
        RtaPdu::alarm_ack(Priority::High, 1, 2, 7, 6, ack()).encode().unwrap(),
        RtaPdu::error(Priority::High, 1, 2, 7, 6, PnioStatus::OK).encode().unwrap(),
    ];
    for w in &written {
        for cut in 0..=w.len() {
            let _ = RtaPdu::decode(&w[..cut]);
        }
    }
    // Twenty thousand noisy PDUs, their var part length restated: each decodes, and writes back to
    // its own bytes without the pad.
    let mut read = 0;
    for i in 0..20_000 {
        let len = (noise.next() as usize) % 64;
        let mut b = header(noise.next() % 6, &noise.bytes(len), if i % 2 == 0 { 0xFC01 } else { 0xFE01 });
        if i % 3 == 0 && len >= 6 {
            // A block header that fits, so the typed paths are reached.
            b[14..20].copy_from_slice(&[if i % 4 == 0 { 0x80 } else { 0x00 }, if i % 5 == 0 { 2 } else { 1 }, 0, (len - 4) as u8, 1, 0]);
            b[6] = 0x11;
        }
        let pad_len = (noise.next() % 8) as usize;
        let pad = noise.bytes(pad_len);
        let mut heard = b.clone();
        heard.extend_from_slice(&pad);
        let p = RtaPdu::decode(&heard).unwrap();
        if !matches!(p.body, Body::Unread { .. }) {
            read += 1;
        }
        assert_eq!(p.encode().unwrap(), b, "{heard:02x?}");
    }
    assert!(read > 500, "the noise reached the typed bodies {read} times");
    // Long var parts, past what a frame carries among them: each decoded one is written back, each
    // other refused by its length.
    let (mut kept, mut refused) = (0, 0);
    for i in 0..2_000 {
        let len = (usize::from(noise.next()) << 4 | usize::from(noise.next() & 0x0F)) % 2_200;
        let mut b = header(noise.next() % 6, &noise.bytes(len), 0xFC01);
        if i % 2 == 0 && len >= 6 {
            b[14..20].copy_from_slice(&[0x00, 0x01, ((len - 4) >> 8) as u8, (len - 4) as u8, 1, 0]);
        }
        match RtaPdu::decode(&b) {
            Ok(p) => {
                kept += 1;
                assert_eq!(p.encode().unwrap(), b);
            }
            Err(e) => {
                refused += 1;
                assert_eq!((e, len > 1486), (AlarmError::TooLong(len), true));
            }
        }
    }
    assert!(kept > 500 && refused > 500, "{kept} kept, {refused} refused");
}

#[test]
fn what_would_read_back_as_another_is_refused() {
    let not_itself = |p: RtaPdu| matches!(p.encode(), Err(AlarmError::NotItself(_)));
    // A typed body under a PDU type that does not carry it.
    assert!(not_itself(pdu(PduType::Data, 0x11, Body::Empty)));
    assert!(not_itself(pdu(PduType::Data, 0x11, Body::Error(PnioStatus::OK))));
    assert!(not_itself(pdu(PduType::Ack, 0x01, Body::Notification(diagnosis(None)))));
    assert!(not_itself(pdu(PduType::Ack, 0x01, Body::Error(PnioStatus::OK))));
    assert!(not_itself(pdu(PduType::Nack, 0x01, Body::Ack(ack()))));
    assert!(not_itself(pdu(PduType::Err, 0x01, Body::Empty)));
    assert!(not_itself(pdu(PduType::Err, 0x01, Body::Notification(diagnosis(None)))));
    assert!(not_itself(pdu(PduType::Other(5), 0x01, Body::Empty)));
    assert!(not_itself(pdu(PduType::Other(5), 0x01, Body::Error(PnioStatus::OK))));
    // A type spelled as another's nibble.
    assert!(not_itself(pdu(PduType::Other(1), 0x11, Body::Notification(diagnosis(None)))));
    assert!(not_itself(pdu(PduType::Other(3), 0x01, Body::Empty)));
    // Bytes kept as unread that would now be read, or read with another reason.
    let var = RtaPdu::notification(Priority::High, 1, 2, 7, 6, diagnosis(None)).encode().unwrap()[14..].to_vec();
    assert!(not_itself(pdu(PduType::Data, 0x11, Body::Unread { bytes: var.clone(), why: Unread::Length })));
    assert!(not_itself(pdu(PduType::Err, 0x01, Body::Unread { bytes: vec![1, 2, 3, 4], why: Unread::Shape })));
    assert!(not_itself(pdu(PduType::Ack, 0x01, Body::Unread { bytes: vec![], why: Unread::Shape })));
    assert!(not_itself(pdu(PduType::Ack, 0x01, Body::Unread { bytes: vec![1], why: Unread::Type })));
    assert!(not_itself(pdu(PduType::Data, 0x11, Body::Unread { bytes: vec![0, 9, 0, 2, 1, 0], why: Unread::Block(0x000A) })));
    // The same bytes with their own reason are written as heard.
    let kept = pdu(PduType::Data, 0x11, Body::Unread { bytes: vec![0, 9, 0, 2, 1, 0], why: Unread::Block(0x0009) });
    assert_eq!(RtaPdu::decode(&kept.encode().unwrap()).unwrap(), kept);
    // An item that would end the block at the specifier is no item.
    let one = pdu(PduType::Data, 0x11, Body::Notification(diagnosis(Some(AlarmItem { usi: 0x8000, data: vec![] }))));
    assert_eq!(RtaPdu::decode(&one.encode().unwrap()).unwrap(), one);

    // Nibbles.
    assert_eq!(pdu(PduType::Other(16), 0x01, Body::Empty).encode(), Err(AlarmError::Nibble { what: "PDU type", value: 16 }));
    assert_eq!(RtaPdu { version: 16, ..pdu(PduType::Ack, 0x01, Body::Empty) }.encode(), Err(AlarmError::Nibble { what: "PDU version", value: 16 }));
    assert_eq!(AlarmError::Nibble { what: "PDU type", value: 16 }.to_string(), "the PDU type is a nibble; 16 is not");
    // Versions 0 and 15 of the PDU are carried as heard.
    for v in [0, 2, 15] {
        let p = RtaPdu { version: v, ..pdu(PduType::Ack, 0x01, Body::Empty) };
        assert_eq!(RtaPdu::decode(&p.encode().unwrap()).unwrap(), p);
    }

    // The longest var part a frame carries, and one more.
    let item = |n: usize| diagnosis(Some(AlarmItem { usi: 0x0001, data: vec![0x5A; n] }));
    let longest = pdu(PduType::Data, 0x11, Body::Notification(item(1486 - 28)));
    let bytes = longest.encode().unwrap();
    assert_eq!(bytes.len(), 1500);
    assert_eq!(RtaPdu::decode(&bytes).unwrap(), longest);
    assert_eq!(longest.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap().len(), 1518);
    assert_eq!(pdu(PduType::Data, 0x11, Body::Notification(item(1487 - 28))).encode(), Err(AlarmError::TooLong(1487)));
    assert_eq!(pdu(PduType::Data, 0x11, Body::Notification(item(70_000))).encode(), Err(AlarmError::TooLong(70_028)));
    assert_eq!(pdu(PduType::Other(9), 0x01, Body::Unread { bytes: vec![0; 1487], why: Unread::Type }).encode(), Err(AlarmError::TooLong(1487)));
    assert_eq!(AlarmError::TooLong(1487).to_string(), "a var part of 1487 bytes is past what a frame carries (1486)");

    // Read from JSON, gated alike.
    let json = serde_json::to_string(&pdu(PduType::Data, 0x11, Body::Notification(diagnosis(None)))).unwrap();
    let back: RtaPdu = serde_json::from_str(&json).unwrap();
    assert!(back.encode().is_ok());
    let wrong: RtaPdu = serde_json::from_str(&json.replace("\"data\"", "\"ack\"")).unwrap();
    assert_eq!(wrong.pdu_type, PduType::Ack);
    assert!(matches!(wrong.encode(), Err(AlarmError::NotItself(_))));
}

/// Every value of `key` (tshark's flattened name) in the `pn_io` layer, wherever its dissector put
/// it: the PNIO status fields are named `pn_rt_pn_io_*` there.
fn pn_io(d: &common::Dissected, key: &str) -> Vec<String> {
    fn walk(v: &Value, key: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == key {
                        match x {
                            Value::String(s) => out.push(s.clone()),
                            Value::Array(a) => out.extend(a.iter().map(|y| y.as_str().map(str::to_string).unwrap_or_else(|| y.to_string()))),
                            other => out.push(other.to_string()),
                        }
                    } else {
                        walk(x, key, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&d.layers["pn_io"], key, &mut out);
    out
}

#[test]
fn the_dissectors_read_every_frame_as_it_was_written() {
    let low_ack = AlarmAck { status: PnioStatus { code: 0xDA, decode: 0x81, code1: 0x3C, code2: 0x01 }, ..ack() };
    let pdus = [
        RtaPdu::notification(Priority::High, 0x0001, 0x0002, 0xFFFF, 0xFFFE, diagnosis(Some(channel()))),
        RtaPdu::notification(Priority::Low, 0x0003, 0x0004, 0x0000, 0x7FFF, Notification { alarm_type: 0x0E, api: 0x12345678, slot: 0, subslot: 0x8002, module_ident: 0xABCDEF01, submodule_ident: 0x00010002, specifier: AlarmSpecifier(0x17FF), item: None }),
        RtaPdu::alarm_ack(Priority::High, 0x0002, 0x0001, 0x0000, 0xFFFF, ack()),
        RtaPdu::alarm_ack(Priority::Low, 0x0004, 0x0003, 0x0001, 0x0000, low_ack),
        RtaPdu::transport_ack(Priority::High, 0x0001, 0x0002, 0xFFFF, 0x0000),
        RtaPdu::error(Priority::Low, 0x0001, 0x0002, 0xFFFF, 0xFFFE, PnioStatus { code: 0xCF, decode: 0x81, code1: 0xFD, code2: 0x05 }),
        pdu(PduType::Nack, 0x01, Body::Empty),
    ];
    let frames: Vec<Vec<u8>> = pdus
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (dst, src) = if i % 2 == 0 { (CONTROLLER, DEVICE) } else { (DEVICE, CONTROLLER) };
            p.frame(dst, src, Some(p.priority.tag())).unwrap()
        })
        .collect();
    let d = common::dissect(&frames);
    for (i, (p, d)) in pdus.iter().zip(&d).enumerate() {
        assert!(d.complaints().is_empty(), "frame {i}: {:?}", d.complaints());
        let (dst, src) = if i % 2 == 0 { (CONTROLLER, DEVICE) } else { (DEVICE, CONTROLLER) };
        assert_eq!((d.one("eth", "eth.dst"), d.one("eth", "eth.src")), (dst.to_string().to_lowercase(), src.to_string().to_lowercase()), "frame {i}");
        assert_eq!(d.one("vlan", "vlan.priority"), p.priority.tag().priority.to_string(), "frame {i}");
        assert_eq!(d.one("pn_rt", "pn_rt.frame_id"), p.priority.frame_id().to_string(), "frame {i}");
        let one = |k: &str| {
            let v = pn_io(d, k);
            assert_eq!(v.len(), 1, "frame {i} {k}: {v:?}");
            v[0].clone()
        };
        assert_eq!(one("pn_io_pn_io_alarm_dst_endpoint"), p.dst_endpoint.to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_alarm_src_endpoint"), p.src_endpoint.to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_pdu_type_type"), p.pdu_type.nibble().to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_pdu_type_version"), p.version.to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_window_size"), p.add_flags.window_size().to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_tack"), (p.add_flags.tack() as u8).to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_send_seq_num"), p.send_seq.to_string(), "frame {i}");
        assert_eq!(one("pn_io_pn_io_ack_seq_num"), p.ack_seq.to_string(), "frame {i}");
        let var = p.encode().unwrap().len() - 14;
        assert_eq!(one("pn_io_pn_io_var_part_len"), var.to_string(), "frame {i}");
        let (block, fields): (Option<u16>, Vec<(&str, String)>) = match &p.body {
            Body::Notification(n) => (
                Some(p.priority.notification_block()),
                vec![
                    ("alarm_type", n.alarm_type.to_string()),
                    ("api", n.api.to_string()),
                    ("slot_nr", n.slot.to_string()),
                    ("subslot_nr", n.subslot.to_string()),
                    ("module_ident_number", n.module_ident.to_string()),
                    ("submodule_ident_number", n.submodule_ident.to_string()),
                    ("alarm_specifier_sequence", n.specifier.sequence().to_string()),
                    ("alarm_specifier_channel", (n.specifier.channel_diagnosis() as u8).to_string()),
                    ("alarm_specifier_manufacturer", (n.specifier.manufacturer_diagnosis() as u8).to_string()),
                    ("alarm_specifier_submodule", (n.specifier.submodule_diagnosis() as u8).to_string()),
                    ("alarm_specifier_ardiagnosis", (n.specifier.ar_diagnosis() as u8).to_string()),
                ],
            ),
            Body::Ack(a) => (
                Some(p.priority.ack_block()),
                vec![
                    ("alarm_type", a.alarm_type.to_string()),
                    ("api", a.api.to_string()),
                    ("slot_nr", a.slot.to_string()),
                    ("subslot_nr", a.subslot.to_string()),
                    ("alarm_specifier_sequence", a.specifier.sequence().to_string()),
                    ("alarm_specifier_channel", (a.specifier.channel_diagnosis() as u8).to_string()),
                    ("alarm_specifier_manufacturer", (a.specifier.manufacturer_diagnosis() as u8).to_string()),
                    ("alarm_specifier_submodule", (a.specifier.submodule_diagnosis() as u8).to_string()),
                    ("alarm_specifier_ardiagnosis", (a.specifier.ar_diagnosis() as u8).to_string()),
                ],
            ),
            _ => (None, vec![]),
        };
        match block {
            Some(t) => {
                assert_eq!(one("pn_io_pn_io_block_type"), t.to_string(), "frame {i}");
                assert_eq!(one("pn_io_pn_io_block_length"), (var - 4).to_string(), "frame {i}");
                assert_eq!((one("pn_io_pn_io_block_version_high"), one("pn_io_pn_io_block_version_low")), ("1".into(), "0".into()), "frame {i}");
                for (k, want) in fields {
                    assert_eq!(one(&format!("pn_io_pn_io_{k}")), want, "frame {i} {k}");
                }
            }
            None => assert!(pn_io(d, "pn_io_pn_io_block_type").is_empty(), "frame {i}"),
        }
        let status = match &p.body {
            Body::Ack(a) => Some(a.status),
            Body::Error(s) => Some(*s),
            _ => None,
        };
        match status {
            Some(s) => {
                assert_eq!(one("pn_rt_pn_io_error_code"), s.code.to_string(), "frame {i}");
                assert_eq!(one("pn_rt_pn_io_error_decode"), s.decode.to_string(), "frame {i}");
            }
            None => assert!(pn_io(d, "pn_rt_pn_io_error_code").is_empty(), "frame {i}"),
        }
    }
    // The item: a channel diagnosis read field by field; a notification without one has none.
    assert_eq!(pn_io(&d[0], "pn_io_pn_io_user_structure_identifier"), ["32768"]);
    assert_eq!(pn_io(&d[0], "pn_io_pn_io_channel_number"), ["32768"]);
    assert_eq!(pn_io(&d[0], "pn_io_pn_io_channel_properties"), ["2048"]);
    assert_eq!(pn_io(&d[0], "pn_io_pn_io_channel_error_type"), ["1"]);
    assert!(pn_io(&d[1], "pn_io_pn_io_user_structure_identifier").is_empty());
    // The statuses' second bytes, under the names the dissector gives them by their first.
    assert_eq!(pn_io(&d[2], "pn_rt_pn_io_error_code1"), ["0"]);
    assert_eq!(pn_io(&d[2], "pn_rt_pn_io_error_code2"), ["0"]);
    assert_eq!(pn_io(&d[3], "pn_rt_pn_io_error_code1_pnio"), ["60"]);
    assert_eq!(pn_io(&d[3], "pn_rt_pn_io_error_code2_pnio_60"), ["1"]);
    assert_eq!(pn_io(&d[5], "pn_rt_pn_io_error_code1_pnio"), ["253"]);
    assert_eq!(pn_io(&d[5], "pn_rt_pn_io_error_code2_pnio_253"), ["5"]);

    // A maintenance item and the item it qualifies: this codec names the first identifier and keeps
    // the rest as its data; the dissector reads both identifiers, with no complaint.
    let maintenance = AlarmItem {
        usi: 0x8100,
        data: [hex("0F00 0008 0100 0000 00000001"), hex("8002"), hex("8000 0800 0001 0000 00000000")].concat(),
    };
    let two = RtaPdu::notification(Priority::High, 1, 2, 3, 4, diagnosis(Some(maintenance.clone())));
    let back = RtaPdu::decode(&two.encode().unwrap()).unwrap();
    assert_eq!(back, two);
    // What the dissector complains of, as written: an item whose data does not fit its identifier's
    // structure (a channel diagnosis with none), and three bodies kept unread and written as heard:
    // a DATA PDU with no block, a PDU type it does not know, a block of version 2.0.
    let bare = RtaPdu::notification(Priority::High, 1, 2, 3, 4, diagnosis(Some(AlarmItem { usi: 0x8000, data: vec![] })));
    let unread = |pdu_type, bytes: Vec<u8>, why| pdu(pdu_type, 0x11, Body::Unread { bytes, why });
    let mut v2 = RtaPdu::notification(Priority::High, 1, 2, 3, 4, diagnosis(None)).encode().unwrap()[14..].to_vec();
    v2[4] = 2;
    let odd = [
        two.clone(),
        bare,
        unread(PduType::Data, vec![], Unread::Shape),
        unread(PduType::Other(5), vec![], Unread::Type),
        unread(PduType::Data, v2, Unread::Version(2, 0)),
    ];
    let frames: Vec<Vec<u8>> = odd.iter().map(|p| p.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap()).collect();
    let d = common::dissect(&frames);
    assert_eq!(pn_io(&d[0], "pn_io_pn_io_user_structure_identifier"), ["33024", "32770"]);
    assert_eq!(d[0].complaints(), Vec::<String>::new());

    // Several diagnoses under one identifier: this codec keeps them as the item's data; the
    // dissector reads one diagnosis per identifier and the next one's channel number as an
    // identifier, silent, or malformed when that channel number is 0x8000.
    let several = [
        (AlarmItem { usi: 0x8000, data: hex("0001 0800 0001  0002 0800 0002") }, vec!["32768", "2"], false),
        (AlarmItem { usi: 0x8002, data: hex("0001 0800 0001 0000 0000 0000  0002 0800 0002 0000 0000 0000") }, vec!["32770", "2"], false),
        (AlarmItem { usi: 0x8000, data: hex("0001 0800 0001  8000 0800 0002") }, vec!["32768", "32768"], true),
    ];
    let frames: Vec<Vec<u8>> = several
        .iter()
        .map(|(item, _, _)| {
            let n = RtaPdu::notification(Priority::High, 1, 2, 3, 4, diagnosis(Some(item.clone())));
            let back = RtaPdu::decode(&n.encode().unwrap()).unwrap();
            assert_eq!(back, n);
            n.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap()
        })
        .collect();
    for (p, (item, usis, malformed)) in common::dissect(&frames).iter().zip(&several) {
        assert_eq!(pn_io(p, "pn_io_pn_io_user_structure_identifier"), *usis, "{item:?}");
        assert_eq!(p.complaints().iter().any(|c| c.contains("malformed")), *malformed, "{item:?}: {:?}", p.complaints());
        if !malformed {
            assert_eq!(p.complaints(), Vec::<String>::new(), "{item:?}");
        }
    }
    let complaint = |i: usize, what: &str| {
        let c = d[i].complaints();
        assert!(c.iter().any(|x| x.contains(what)), "frame {i}: {c:?}");
    };
    complaint(1, "malformed");
    complaint(2, "length >= 0");
    complaint(3, "malformed");
    complaint(4, "Block version 2.0 not implemented");
    // An item under an identifier the dissector names and does not decode (iParameter here) draws
    // its "Undecoded Data", whatever the item holds.
    let iparameter = RtaPdu::notification(Priority::High, 1, 2, 7, 6, diagnosis(Some(AlarmItem { usi: 0x8201, data: vec![0; 16] })));
    let d = common::dissect(&[iparameter.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap()]);
    assert!(d[0].complaints().iter().any(|c| c.contains("Undecoded Data, 16 bytes")), "{:?}", d[0].complaints());

    // A status the dissector does not know draws its complaint: an ErrorDecode it has no table for,
    // an ErrorCode1 it does not know under PNIO's. And a var part this codec keeps unread is not
    // always one the dissector complains of: a notification of the other priority than its frame
    // it reads as a notification, silent. The dissector is no oracle for what is kept unread.
    let unknown_decode = RtaPdu::error(Priority::Low, 1, 2, 3, 4, PnioStatus { code: 0xCF, decode: 0x82, code1: 0, code2: 0 });
    let unknown_code1 = RtaPdu::error(Priority::High, 1, 2, 3, 4, PnioStatus { code: 0xCF, decode: 0x81, code1: 0, code2: 0 });
    let other_priority = {
        let var = RtaPdu::notification(Priority::Low, 1, 2, 7, 6, diagnosis(None)).encode().unwrap()[14..].to_vec();
        pdu(PduType::Data, 0x11, Body::Unread { bytes: var, why: Unread::Priority(0x0002) })
    };
    assert_eq!(RtaPdu::decode(&other_priority.encode().unwrap()).unwrap(), other_priority);
    let frames: Vec<Vec<u8>> = [&unknown_decode, &unknown_code1, &other_priority].iter().map(|p| p.frame(CONTROLLER, DEVICE, Some(p.priority.tag())).unwrap()).collect();
    let d = common::dissect(&frames);
    assert!(d[0].complaints().iter().any(|c| c.contains("Unknown ErrorDecode 0x82")), "{:?}", d[0].complaints());
    assert!(d[1].complaints().iter().any(|c| c.contains("Unknown ErrorCode1 0x0")), "{:?}", d[1].complaints());
    assert_eq!(d[2].complaints(), Vec::<String>::new());
    assert_eq!(pn_io(&d[2], "pn_io_pn_io_block_type"), ["2"]);

    // A seeded batch of typed frames, statuses the dissector knows: each read field for field, with
    // no complaint (seven hand-picked frames are not the whole of the claim).
    let mut noise = common::Noise(0xA1A3);
    let mut batch = Vec::new();
    for i in 0..300u32 {
        let priority = if noise.next() & 1 == 0 { Priority::High } else { Priority::Low };
        let (dst, src, send, ack) = (u16::from(noise.next()) << 8 | u16::from(noise.next()), u16::from(noise.next()), u16::from(noise.next()) << 7, u16::from(noise.next()));
        let n = Notification {
            alarm_type: u16::from(noise.next() % 0x20),
            api: u32::from(noise.next()) << 16 | u32::from(noise.next()),
            slot: u16::from(noise.next()),
            subslot: 0x8000 | u16::from(noise.next()),
            module_ident: u32::from(noise.next()) << 24 | u32::from(noise.next()),
            submodule_ident: u32::from(noise.next()),
            specifier: AlarmSpecifier(u16::from(noise.next()) << 8 & 0xBF00 | u16::from(noise.next())),
            item: match i % 3 {
                0 => None,
                1 => {
                    let usi = u16::from(noise.next() & 0x7F) << 8 | u16::from(noise.next());
                    let len = usize::from(noise.next() % 40);
                    Some(AlarmItem { usi, data: noise.bytes(len) })
                }
                _ => Some(channel()),
            },
        };
        let p = match i % 4 {
            0 | 1 => RtaPdu::notification(priority, dst, src, send, ack, n),
            2 => RtaPdu::alarm_ack(priority, dst, src, send, ack, AlarmAck { alarm_type: n.alarm_type, api: n.api, slot: n.slot, subslot: n.subslot, specifier: n.specifier, status: PnioStatus::OK }),
            _ => RtaPdu::transport_ack(priority, dst, src, send, ack),
        };
        batch.push(p);
    }
    let frames: Vec<Vec<u8>> = batch.iter().map(|p| p.frame(CONTROLLER, DEVICE, Some(p.priority.tag())).unwrap()).collect();
    let d = common::dissect(&frames);
    for (i, (p, d)) in batch.iter().zip(&d).enumerate() {
        assert_eq!(d.complaints(), Vec::<String>::new(), "frame {i}: {p:?}");
        let one = |k: &str| pn_io(d, k);
        assert_eq!(one("pn_io_pn_io_alarm_dst_endpoint"), [p.dst_endpoint.to_string()], "frame {i}");
        assert_eq!(one("pn_io_pn_io_alarm_src_endpoint"), [p.src_endpoint.to_string()], "frame {i}");
        assert_eq!(one("pn_io_pn_io_send_seq_num"), [p.send_seq.to_string()], "frame {i}");
        assert_eq!(one("pn_io_pn_io_ack_seq_num"), [p.ack_seq.to_string()], "frame {i}");
        // Every block field: the block's type, length and version, the alarm's place, the
        // specifier's sequence and its four bits, the idents and the item's identifier, the
        // acknowledgement's status.
        let flag = |b: bool| u8::from(b).to_string();
        let var = p.encode().unwrap().len() - 14;
        let block = |want_type: u16, alarm_type: u16, api: u32, slot: u16, subslot: u16, s: AlarmSpecifier| {
            assert_eq!(one("pn_io_pn_io_block_type"), [want_type.to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_block_length"), [(var - 4).to_string()], "frame {i}");
            assert_eq!((one("pn_io_pn_io_block_version_high"), one("pn_io_pn_io_block_version_low")), (vec!["1".to_string()], vec!["0".to_string()]), "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_type"), [alarm_type.to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_api"), [api.to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_slot_nr"), [slot.to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_subslot_nr"), [subslot.to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_specifier_sequence"), [s.sequence().to_string()], "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_specifier_channel"), [flag(s.channel_diagnosis())], "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_specifier_manufacturer"), [flag(s.manufacturer_diagnosis())], "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_specifier_submodule"), [flag(s.submodule_diagnosis())], "frame {i}");
            assert_eq!(one("pn_io_pn_io_alarm_specifier_ardiagnosis"), [flag(s.ar_diagnosis())], "frame {i}");
        };
        match &p.body {
            Body::Notification(n) => {
                block(p.priority.notification_block(), n.alarm_type, n.api, n.slot, n.subslot, n.specifier);
                assert_eq!(one("pn_io_pn_io_module_ident_number"), [n.module_ident.to_string()], "frame {i}");
                assert_eq!(one("pn_io_pn_io_submodule_ident_number"), [n.submodule_ident.to_string()], "frame {i}");
                let usi: Vec<String> = n.item.iter().map(|it| it.usi.to_string()).collect();
                assert_eq!(one("pn_io_pn_io_user_structure_identifier"), usi, "frame {i}");
            }
            Body::Ack(a) => {
                block(p.priority.ack_block(), a.alarm_type, a.api, a.slot, a.subslot, a.specifier);
                let st = a.status;
                assert_eq!(
                    [one("pn_rt_pn_io_error_code"), one("pn_rt_pn_io_error_decode"), one("pn_rt_pn_io_error_code1"), one("pn_rt_pn_io_error_code2")],
                    [[st.code], [st.decode], [st.code1], [st.code2]].map(|b| vec![b[0].to_string()]),
                    "frame {i}"
                );
            }
            _ => assert!(one("pn_io_pn_io_block_type").is_empty(), "frame {i}"),
        }
    }

    // The dissector reads TACK as the whole high nibble of AddFlags, reserved bits with it: with
    // reserved bits set, this codec's TACK (bit 4) and the dissector's differ, as written.
    let reserved = [pdu(PduType::Ack, 0xF1, Body::Empty), pdu(PduType::Ack, 0xE1, Body::Empty)];
    let frames: Vec<Vec<u8>> = reserved.iter().map(|p| p.frame(CONTROLLER, DEVICE, Some(Priority::High.tag())).unwrap()).collect();
    let d = common::dissect(&frames);
    assert_eq!((pn_io(&d[0], "pn_io_pn_io_tack"), reserved[0].add_flags.tack()), (vec!["15".to_string()], true));
    assert_eq!((pn_io(&d[1], "pn_io_pn_io_tack"), reserved[1].add_flags.tack()), (vec!["14".to_string()], false));
}
