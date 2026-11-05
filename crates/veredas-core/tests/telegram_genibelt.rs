//! The GENIBELT typed catalog (contract: the catalog clause), from Veredas 1's `veredas-telegram`
//! tests: sizes, layouts and round trips per the vendor specification (`docs/questions.md` Q6).

use veredas_core::telegram::Endian;
use veredas_core::telegram::genibelt::*;

/// Round-trip a message through its own encode/decode.
fn round_trip(msg: &Message, endian: Endian) -> Message {
    let body = msg.encode(endian).expect("encodes");
    Message::decode(msg.msg_id(), &body, endian).expect("decodes")
}

#[test]
fn sizes_match_the_specification() {
    // Every MSG LENGTH the spec declares, in one place: a layout
    // typo shows up here instead of desynchronizing a real sorter.
    // (§4.x tables; the two variable-length messages are excluded.)
    let cases: Vec<(Message, usize)> = vec![
        (Message::System(System::default()), 2),
        (Message::Commands(Commands::default()), 10),
        (Message::CommandsEx(CommandsEx::default()), 15),
        (Message::Destination(Destination::default()), 46),
        (Message::Lamp(Lamp::default()), 14),
        (Message::CodingAnswer(CodingAnswer::default()), 375),
        (Message::EuBinState(EuBinState::default()), 6),
        (Message::Devices(Devices::default()), 12),
        (Message::BinLcdDisplay(BinLcdDisplay::default()), 82),
        (Message::OutputSignals(OutputSignals::default()), 34),
        (Message::CellReloadEx(CellReloadEx::default()), 249),
        (Message::IlBeltMaintenance(IlBeltMaintenance::default()), 8),
        (Message::Outcome(Outcome::default()), 17),
        (Message::SorterStatus(SorterStatus::default()), 5),
        (Message::SorterStatusEx(SorterStatusEx::default()), 6),
        (Message::BinPushButtons(BinPushButtons::default()), 10),
        (Message::BinStatus(BinStatus::default()), 7),
        (Message::Scanner(ScannerRequest::default()), 493),
        (Message::CodingRequest(CodingRequest::default()), 247),
        (Message::CodingResult(CodingResult::default()), 7),
        (Message::SorterLaps(SorterLaps::default()), 1),
        (Message::UpsStatus(UpsStatus::default()), 2),
        (Message::MscDevices(MscDevices::default()), 251),
        (Message::ParcelLoaded(ParcelLoaded::default()), 14),
        (Message::ParcelRecirculated(ParcelZoneEvent::default()), 13),
        (Message::ParcelOnCtrlZone(ParcelZoneEvent::default()), 13),
        (Message::InventoryBegin, 0),
        (Message::InventoryEnd, 0),
        (Message::SorterEmpty, 0),
        (Message::LampTest(LampTest::default()), 4),
        (
            Message::GenericDataExchange(GenericDataExchange::default()),
            506,
        ),
        (Message::ItemLost(ItemLost::default()), 251),
    ];
    for (msg, expected) in cases {
        let body = msg.encode(Endian::Little).expect("encodes");
        assert_eq!(
            body.len(),
            expected,
            "{} (id {}) should be {expected} bytes",
            msg.kind(),
            msg.msg_id()
        );
        // And a body of the right size decodes back to the same
        // message — the layout is self-consistent.
        let back = Message::decode(msg.msg_id(), &body, Endian::Little).expect("decodes");
        assert_eq!(back, msg, "{} did not round-trip", msg.kind());
    }
    // The one message the spec sizes as "506" is also the largest,
    // and it must still fit the 512-byte segment limit (§3.3).
    let largest = Message::GenericDataExchange(GenericDataExchange::default())
        .encode(Endian::Little)
        .unwrap()
        .len();
    assert_eq!(
        (largest, MAX_DATA_SEGMENT, MAX_PROTOCOL),
        (506, 512, 5 * 1024)
    );
}

#[test]
fn a_destination_carries_every_field_the_msc_reads() {
    // The message this whole chain exists to send.
    let msg = Message::Destination(Destination {
        object_id: 0xDEAD_BEEF,
        dest: 42,
        cell: 1234,
        used_cells: 0,
        flag: dest_flag::ASSIGNED,
        scn_id: 3,
        operator_id: 11,
        ramp_id: 0,
        dest_unload_offset: 5,
        oam_reject_anomaly: 0,
        num_alternative_dest: 2,
        alternative_dest: vec![50, 51, 0, 0, 0, 0, 0, 0, 0, 0],
        alternative_dest_unload_offset: vec![-1, 2, 0, 0, 0, 0, 0, 0, 0, 0],
    });
    for endian in [Endian::Little, Endian::Big] {
        assert_eq!(round_trip(&msg, endian), msg, "{endian:?}");
    }
    assert_eq!(msg.object_id(), Some(0xDEAD_BEEF), "the lifecycle key");
    assert_eq!(msg.dest(), Some(42));
    assert_eq!(
        dest_flag::name(dest_flag::FORCE_REASSIGNEMENT),
        "force reassignment"
    );

    // The two constructors the reply path uses.
    let assigned = Destination::assigned(5, 100, 7);
    assert_eq!(assigned.flag, dest_flag::ASSIGNED);
    assert_eq!(assigned.dest, 7);
    assert_eq!(
        Message::Destination(assigned)
            .encode(Endian::Little)
            .unwrap()
            .len(),
        46
    );
    // The no-decision answer: the parcel keeps circulating and is
    // asked about again, rather than being dumped somewhere wrong.
    let retry = Destination::force_reassignment(5, 100);
    assert_eq!(retry.flag, dest_flag::FORCE_REASSIGNEMENT);
    assert_eq!(retry.object_id, 5);
    assert_eq!(retry.cell, 100);

    // Little-endian is the spec's format (§3, "INTEL"): check the
    // first field lands LSB-first on the wire.
    let body = msg.encode(Endian::Little).unwrap();
    assert_eq!(&body[..4], &[0xEF, 0xBE, 0xAD, 0xDE]);
}

#[test]
fn a_scanner_request_yields_the_parcel_physicals_the_sort_plan_needs() {
    let ispc = IspcMscData {
        ispc_id: 11,
        object_id: 777,
        length_mm: 400,
        width_mm: 300,
        height_mm: 250,
        volume: 30_000_000,
        weight_g: 2500,
        statistics: FsgStatisticData {
            subsystem_type: 2,
            zone_id: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut msc_data = vec![b'A'];
    msc_data.extend_from_slice(b"BARCODE-1;BARCODE-2");
    msc_data.resize(SZ_DATA_LEN, 0);

    let msg = Message::Scanner(ScannerRequest {
        scn_id: 1,
        operator_id: 11,
        cell: 500,
        used_cells: 1,
        object_id: 777,
        request_type: 1,
        request_anomaly: 0,
        cell_nif_position: 9000,
        msc_data,
        user_data: ispc.encode(SZ_DATA_LEN, Endian::Little),
    });
    let back = round_trip(&msg, Endian::Little);
    assert_eq!(back, msg);

    let Message::Scanner(req) = &back else {
        panic!("not a scanner request")
    };
    assert_eq!(req.read_status(), ReadStatus::Acknowledged);
    let parsed = req.ispc(Endian::Little);
    assert_eq!(parsed.length_mm, 400);
    assert_eq!(parsed.width_mm, 300);
    assert_eq!(parsed.height_mm, 250);
    assert_eq!(parsed.volume, 30_000_000);
    assert_eq!(parsed.weight_g, 2500);
    assert_eq!(parsed.object_id, 777);
    assert_eq!(parsed.statistics.zone_id, 4);
    assert_eq!(back.object_id(), Some(777));
}

#[test]
fn a_no_read_parcel_is_recognisable() {
    // The operationally critical case: the scanner never answered.
    let mut msc_data = vec![b'T'];
    msc_data.resize(SZ_DATA_LEN, 0);
    let req = ScannerRequest {
        object_id: 1,
        msc_data,
        user_data: vec![0; SZ_DATA_LEN],
        ..Default::default()
    };
    assert_eq!(req.read_status(), ReadStatus::Timeout);

    let mut virtual_scan = vec![0x01];
    virtual_scan.resize(SZ_DATA_LEN, 0);
    let req = ScannerRequest {
        msc_data: virtual_scan,
        ..req.clone()
    };
    assert_eq!(req.read_status(), ReadStatus::Virtual);
}

#[test]
fn outcomes_read_as_words_not_codes() {
    let msg = Message::Outcome(Outcome {
        object_id: 900,
        dest: 12,
        ret: outcome_return::REJECTED,
        anomaly: outcome_anomaly::CHUTE_FULL,
        cell: 77,
        unload_offset: -3,
        ..Default::default()
    });
    assert_eq!(round_trip(&msg, Endian::Little), msg);
    assert_eq!(outcome_return::name(outcome_return::REJECTED), "rejected");
    assert_eq!(
        outcome_anomaly::name(outcome_anomaly::CHUTE_FULL),
        "chute full"
    );
    assert_eq!(
        outcome_anomaly::name(outcome_anomaly::NO_DESTINATION),
        "no destination received"
    );
    assert_eq!(outcome_anomaly::name(105), "chute manager refusal");
    assert_eq!(msg.object_id(), Some(900));
    assert_eq!(msg.kind(), "outcome");
}

#[test]
fn bin_state_bits_decide_whether_a_chute_can_take_parcels() {
    // What the sort plan asks before offering a destination.
    assert!(bin_state::accepts_parcels(bin_state::RUNNING));
    assert!(bin_state::accepts_parcels(
        bin_state::RUNNING | bin_state::HALF_FULL
    ));
    assert!(!bin_state::accepts_parcels(bin_state::FULL));
    assert!(!bin_state::accepts_parcels(bin_state::JAM));
    assert!(!bin_state::accepts_parcels(
        bin_state::RUNNING | bin_state::BLOCKED_BY_OAM
    ));
    let names = bin_state::names(bin_state::FULL | bin_state::ALARM);
    assert!(
        names.contains(&"full") && names.contains(&"alarm"),
        "{names:?}"
    );
}

#[test]
fn text_fields_survive_their_fixed_width_blocks() {
    let msg = Message::OutputSignals(OutputSignals {
        class_id: 55,
        device_id: 7,
        output_name: "CONV_START".into(),
        output_state: 1,
        time_on_ms: 500,
        time_off_ms: 500,
    });
    let back = round_trip(&msg, Endian::Little);
    assert_eq!(back, msg, "ASCIIZ round-trips");

    let msg = Message::BinLcdDisplay(BinLcdDisplay {
        dest: 12,
        text: "CHUTE 12 — READY".into(),
    });
    for endian in [Endian::Little, Endian::Big] {
        assert_eq!(round_trip(&msg, endian), msg, "{endian:?} UNICODE");
    }
    // A name longer than its block is truncated, not overflowed.
    let long = Message::OutputSignals(OutputSignals {
        output_name: "X".repeat(40),
        ..Default::default()
    });
    let body = long.encode(Endian::Little).unwrap();
    assert_eq!(body.len(), 34);
}

#[test]
fn a_wrong_length_body_is_an_error_not_a_panic() {
    // These bytes arrive from a LAN: every malformed shape must be
    // a typed refusal, never a slice panic that kills the source.
    assert_eq!(
        Message::decode(ids::RTXTOOAM_OUTCOME, &[1, 2, 3], Endian::Little),
        Err(MsgError::WrongLength {
            msg_id: ids::RTXTOOAM_OUTCOME,
            expected: 17,
            actual: 3
        })
    );
    assert!(Message::decode(ids::RTXTOOAM_SCANNER, &[], Endian::Little).is_err());
    assert!(matches!(
        Message::decode(ids::RTXTOOAM_OUTCOME, &vec![0u8; 600], Endian::Little),
        Err(MsgError::TooLong { .. })
    ));
    // Truncated variable-length messages too.
    assert!(Message::decode(ids::OAMTORTX_CELL_RELOAD, &[0, 0], Endian::Little).is_err());
}

#[test]
fn an_unknown_id_is_kept_verbatim_rather_than_dropped() {
    // Custom plant messages exist (the spec says so). Recording
    // them losslessly is what makes the lifecycle store honest.
    let data = vec![1, 2, 3, 4, 5];
    let msg = Message::decode(49_999, &data, Endian::Little).unwrap();
    assert_eq!(
        msg,
        Message::Unknown {
            msg_id: 49_999,
            data: data.clone()
        }
    );
    assert_eq!(msg.encode(Endian::Little).unwrap(), data);
    assert_eq!(msg.kind(), "unknown");
    assert_eq!(msg.object_id(), None);
}

#[test]
fn variable_length_messages_carry_their_custom_tails() {
    let msg = Message::CellReload(CellReload {
        cell: 100,
        used_cells: 2,
        ispc_id: 11,
        object_id: 555,
        custom_data: vec![9; 40],
    });
    let body = msg.encode(Endian::Little).unwrap();
    assert_eq!(body.len(), 8 + 40, "the spec's 8 + X");
    assert_eq!(round_trip(&msg, Endian::Little), msg);
}

// ---------------------------------------------------------------------------
// Refute round 1 (C-011): the fixes, pinned at their edges.
// ---------------------------------------------------------------------------

#[test]
fn an_output_name_always_ends_in_nul() {
    // §4.1.1.7: 26 bytes on the wire, the last always 0, so 25 usable characters.
    for len in [25, 26, 40] {
        let msg = Message::OutputSignals(OutputSignals { output_name: "X".repeat(len), ..Default::default() });
        let body = msg.encode(Endian::Little).unwrap();
        assert_eq!(body.len(), 34);
        assert!(body[3..28].iter().all(|b| *b == b'X'), "{len}: {body:?}");
        assert_eq!(body[28], 0, "{len}: the block ends in NUL");
        let Message::OutputSignals(back) = Message::decode(msg.msg_id(), &body, Endian::Little).unwrap() else {
            panic!("an output signal decodes as one");
        };
        assert_eq!(back.output_name, "X".repeat(25));
    }
}

#[test]
fn every_named_bin_state_bit_reads_as_a_word() {
    assert_eq!(bin_state::BITS.len(), 30, "32 bits, two reserved");
    assert_eq!(
        bin_state::names(0xF000_0000),
        ["mapping", "emergency tristate", "alarm tristate", "warning tristate"]
    );
    assert_eq!(bin_state::names(bin_state::RUNNING | bin_state::MAPPING), ["running", "mapping"]);
    assert_eq!(bin_state::names(0x0C00_0000), Vec::<&str>::new(), "the reserved bits name nothing");
    // docs/questions.md Q7, the option taken: the TRISTATE bits do not stop a chute.
    assert!(bin_state::accepts_parcels(0xF000_0000));
    assert!(!bin_state::accepts_parcels(bin_state::FULL | bin_state::MAPPING));
}

#[test]
fn a_body_fits_a_512_byte_segment_with_its_header() {
    assert_eq!(MAX_BODY, 508);
    let reload = |custom: usize| {
        Message::CellReload(CellReload {
            cell: 1,
            used_cells: 1,
            ispc_id: 1,
            object_id: 1,
            custom_data: vec![0xAB; custom],
        })
    };
    assert_eq!(reload(500).encode(Endian::Little).unwrap().len(), 508);
    assert_eq!(
        reload(501).encode(Endian::Little),
        Err(MsgError::TooLong { msg_id: reload(0).msg_id(), len: 509 })
    );
    assert!(matches!(
        Message::decode(49_999, &[0; 508], Endian::Little),
        Ok(Message::Unknown { .. })
    ));
    assert_eq!(
        Message::decode(49_999, &[0; 509], Endian::Little),
        Err(MsgError::TooLong { msg_id: 49_999, len: 509 })
    );
    assert_eq!(
        MsgError::TooLong { msg_id: 7, len: 509 }.to_string(),
        "message 7 carries 509 bytes — over the 508-byte body a 512-byte segment holds"
    );
}

#[test]
fn a_utf16_block_never_ends_in_half_a_character() {
    // 39 units then a surrogate pair: the pair would need bytes 78..82 of an 80-byte block.
    let text = format!("{}\u{1F600}", "A".repeat(39));
    for endian in [Endian::Little, Endian::Big] {
        let block = utf16_block(&text, 80, endian);
        assert_eq!(block.len(), 80);
        assert_eq!(&block[78..], [0, 0], "no lone high surrogate");
        assert_eq!(utf16z(&block, endian), "A".repeat(39));
    }
    // With room, the pair goes out whole and comes back.
    let block = utf16_block("\u{1F600}", 80, Endian::Little);
    assert_eq!(utf16z(&block, Endian::Little), "\u{1F600}");
}

// ---------------------------------------------------------------------------
// Golden bodies (C-011, refute round 1; written by the critic from the specification's offset
// tables, §4.1, §4.2, §5, §6, §7, §8). Every message is built by hand with a writer of its own,
// so a layout that encode and decode get wrong the same way still fails here. Field values are
// distinct within each message, so two same-width fields swapped on both sides fail too. The
// word tables (§10.3, §10.6, §4.2.1.4) are pinned by number, not through the catalog's constants.
// ---------------------------------------------------------------------------

/// A little-endian body (§3, INTEL format), written independently of the catalog.
struct B(Vec<u8>);

impl B {
    fn new() -> B {
        B(Vec::new())
    }

    fn u8(mut self, v: u8) -> B {
        self.0.push(v);
        self
    }

    fn u16(mut self, v: u16) -> B {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn u32(mut self, v: u32) -> B {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn raw(mut self, v: &[u8]) -> B {
        self.0.extend_from_slice(v);
        self
    }

    fn done(self) -> Vec<u8> {
        self.0
    }
}

/// A position-dependent byte pattern: a block shifted by one byte does not compare equal.
fn pat(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)).collect()
}

/// UTF-16 units, little-endian, zero-padded to the LCD field's 80 bytes (§4.1.1.6).
fn utf16_le_80(units: &[u16]) -> Vec<u8> {
    let mut out: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    out.resize(80, 0);
    out
}

/// The hand-built body decodes to `expected`, and `expected` encodes back to the same bytes.
fn check(id: u16, body: Vec<u8>, expected: Message) {
    assert_eq!(expected.msg_id(), id, "{}", expected.kind());
    assert_eq!(Message::decode(id, &body, Endian::Little), Ok(expected.clone()), "decode {id}");
    assert_eq!(expected.encode(Endian::Little), Ok(body), "encode {id}");
}

#[test]
fn golden_bodies_external_unit_to_msc() {
    use ids::*;

    // §4.1.1.1
    check(OAMTORTX_SYSTEM, B::new().u16(0x0102).done(), Message::System(System { system: 0x0102 }));

    // §4.1.1.2
    check(
        OAMTORTX_COMMANDS,
        B::new().u8(1).u8(12).u8(0x23).u8(0x34).u16(0x0506).u32(0x0708_090A).done(),
        Message::Commands(Commands {
            command: 1,
            operator_id: 12,
            param0: 0x23,
            param1: 0x34,
            param2: 0x0506,
            param3: 0x0708_090A,
        }),
    );

    // §4.1.1.3
    check(
        OAMTORTX_COMMANDS_EX,
        B::new()
            .u8(1)
            .u8(12)
            .u8(0x23)
            .u8(0x34)
            .u16(0x0506)
            .u32(0x0708_090A)
            .u8(55)
            .u16(0x0C0D)
            .u8(2)
            .u8(63)
            .done(),
        Message::CommandsEx(CommandsEx {
            command: 1,
            operator_id: 12,
            param0: 0x23,
            param1: 0x34,
            param2: 0x0506,
            param3: 0x0708_090A,
            class_id: 55,
            element_id: 0x0C0D,
            status_alarm_warning: 2,
            bit_number: 63,
        }),
    );

    // §4.1.2.1: the ten alternates and their ten offsets are fixed on the wire.
    let alternates: Vec<u16> = (101..111).collect();
    let offsets: Vec<i8> = (1..11).map(|i: i8| -i).collect();
    let mut body = B::new()
        .u32(0x1122_3344)
        .u16(0x5566)
        .u16(0x7788)
        .u8(1)
        .u8(3)
        .u8(2)
        .u8(11)
        .u8(7)
        .u8(0xFB) // cDestUnloadOffset = -5
        .u8(9)
        .u8(2);
    for a in &alternates {
        body = body.u16(*a);
    }
    for o in &offsets {
        body = body.u8(*o as u8);
    }
    check(
        OAMTORTX_DESTINATION,
        body.done(),
        Message::Destination(Destination {
            object_id: 0x1122_3344,
            dest: 0x5566,
            cell: 0x7788,
            used_cells: 1,
            flag: 3,
            scn_id: 2,
            operator_id: 11,
            ramp_id: 7,
            dest_unload_offset: -5,
            oam_reject_anomaly: 9,
            num_alternative_dest: 2,
            alternative_dest: alternates,
            alternative_dest_unload_offset: offsets,
        }),
    );

    // §4.1.1.4: bInhibitUnload is a four-byte BOOL.
    check(
        OAMTORTX_LAMP,
        B::new().u16(0x0A0B).u16(3).u8(4).u32(1).u8(1).u16(1000).u16(2000).done(),
        Message::Lamp(Lamp {
            device_id: 0x0A0B,
            element: 3,
            command: 4,
            inhibit_unload: true,
            lamp_type: 1,
            time_on_ms: 1000,
            time_off_ms: 2000,
        }),
    );

    // §5.2.1.1
    check(
        OAMTORTX_CODINGANSWER,
        B::new().u8(12).u16(0x3344).raw(&pat(240, 1)).u32(1).raw(&pat(128, 3)).done(),
        Message::CodingAnswer(CodingAnswer {
            operator_id: 12,
            timeout_10ms: 0x3344,
            user_data: pat(240, 1),
            item_expected: true,
            message_to_show: pat(128, 3),
        }),
    );

    // §4.1.1.5
    check(
        OAMTORTX_EUBINSTATE,
        B::new().u16(0x0102).u32(0x0020_8000).done(),
        Message::EuBinState(EuBinState { dest: 0x0102, state: 0x0020_8000 }),
    );

    // §4.1.2.2
    check(
        OAMTORTX_DEVICES,
        B::new().u32(0x0102_0304).u8(5).u16(0x0607).u8(1).u32(0x0A0B_0C0D).done(),
        Message::Devices(Devices {
            device_type: 0x0102_0304,
            device_id: 5,
            cell: 0x0607,
            flag: 1,
            object_id: 0x0A0B_0C0D,
        }),
    );

    // §4.1.1.6: a non-ASCII unit in its little-endian order, then all 40 units used.
    check(
        OAMTORTX_EUIBINLCDDISPLAY,
        B::new().u16(12).raw(&utf16_le_80(&[0x41, 0x2014, 0x42])).done(),
        Message::BinLcdDisplay(BinLcdDisplay { dest: 12, text: "A\u{2014}B".into() }),
    );
    let forty: Vec<u16> = (0..40).map(|i| 0x30 + i % 10).collect();
    check(
        OAMTORTX_EUIBINLCDDISPLAY,
        B::new().u16(0x0102).raw(&utf16_le_80(&forty)).done(),
        Message::BinLcdDisplay(BinLcdDisplay { dest: 0x0102, text: "0123456789".repeat(4) }),
    );

    // §4.1.1.7: szOutputName is 26 bytes, ASCIIZ; 25 characters is the longest name.
    let mut name = b"CONV".to_vec();
    name.resize(26, 0);
    check(
        OAMTORTX_OUTPUTSIGNALS,
        B::new().u8(55).u16(0x0102).raw(&name).u8(4).u16(100).u16(200).done(),
        Message::OutputSignals(OutputSignals {
            class_id: 55,
            device_id: 0x0102,
            output_name: "CONV".into(),
            output_state: 4,
            time_on_ms: 100,
            time_off_ms: 200,
        }),
    );
    let mut longest = vec![b'X'; 25];
    longest.push(0);
    check(
        OAMTORTX_OUTPUTSIGNALS,
        B::new().u8(55).u16(7).raw(&longest).u8(1).u16(500).u16(600).done(),
        Message::OutputSignals(OutputSignals {
            class_id: 55,
            device_id: 7,
            output_name: "X".repeat(25),
            output_state: 1,
            time_on_ms: 500,
            time_off_ms: 600,
        }),
    );

    // §4.1.2.3: 8 + X.
    check(
        OAMTORTX_CELL_RELOAD,
        B::new().u16(0x0102).u8(3).u8(11).u32(0x0A0B_0C0D).raw(&[9, 8, 7]).done(),
        Message::CellReload(CellReload {
            cell: 0x0102,
            used_cells: 3,
            ispc_id: 11,
            object_id: 0x0A0B_0C0D,
            custom_data: vec![9, 8, 7],
        }),
    );

    // §4.1.2.4
    check(
        OAMTORTX_CELL_RELOAD_EX,
        B::new().u16(0x0102).u8(3).u8(11).u8(1).u32(0x0A0B_0C0D).raw(&pat(240, 5)).done(),
        Message::CellReloadEx(CellReloadEx {
            cell: 0x0102,
            used_cells: 3,
            ispc_id: 11,
            rescan_required: 1,
            object_id: 0x0A0B_0C0D,
            user_data: pat(240, 5),
        }),
    );

    // §8.1
    check(
        OAMTORTX_ILBELT_MAINTENANCE,
        B::new().u8(65).u16(32).u8(2).u8(3).u16(500).u8(1).done(),
        Message::IlBeltMaintenance(IlBeltMaintenance {
            class_id: 65,
            device_id: 32,
            belt_id: 2,
            command: 3,
            time_on_ms: 500,
            direction: 1,
        }),
    );
}

#[test]
fn golden_bodies_msc_to_external_unit() {
    use ids::*;

    // §4.2.1.1
    check(
        RTXTOOAM_OUTCOME,
        B::new()
            .u32(0x0102_0304)
            .u16(0x0C0D)
            .u8(0xFD) // cUnloadOffset = -3
            .u16(0x0E0F)
            .u8(2)
            .u16(0x1112)
            .u16(0x1314)
            .u16(2)
            .u8(11)
            .done(),
        Message::Outcome(Outcome {
            object_id: 0x0102_0304,
            dest: 0x0C0D,
            unload_offset: -3,
            cell: 0x0E0F,
            used_cell: 2,
            cell_nif_position: 0x1112,
            zone: 0x1314,
            ret: 2,
            anomaly: 11,
        }),
    );

    // §4.2.1.2
    check(
        RTXTOOAM_SORTERSTATUS,
        B::new().u8(1).u8(2).u8(3).u8(4).u8(0x21).done(),
        Message::SorterStatus(SorterStatus {
            mode: 1,
            moto: 2,
            transient: 3,
            stop_request: 4,
            enable_mode: 0x21,
        }),
    );

    // §4.2.1.3
    check(
        RTXTOOAM_SORTERSTATUS_EX,
        B::new().u8(1).u8(2).u8(3).u8(4).u8(0x21).u8(5).done(),
        Message::SorterStatusEx(SorterStatusEx {
            status: SorterStatus { mode: 1, moto: 2, transient: 3, stop_request: 4, enable_mode: 0x21 },
            working_mode: 5,
        }),
    );

    // §4.2.1.4
    check(
        RTXTOOAM_BINSTATUS,
        B::new().u16(0x0102).u8(9).u32(0x0000_0C01).done(),
        Message::BinStatus(BinStatus { dest: 0x0102, bin_type: 9, state: 0x0000_0C01 }),
    );

    // §4.2.1.5
    check(
        RTXTOOAM_BINPUSHBUTTONS,
        B::new().u32(7).u32(2).u16(0x0102).done(),
        Message::BinPushButtons(BinPushButtons { button_id: 7, state: 2, dest: 0x0102 }),
    );

    // §4.2.1.6
    check(RTXTOOAM_EVENT_SORTERLAPS, vec![200], Message::SorterLaps(SorterLaps { laps: 200 }));

    // §4.2.1.7: envelope-only messages carry an empty body.
    check(RTXTOOAM_SORTER_EMPTY, vec![], Message::SorterEmpty);
    check(RTXTOOAM_INVENTORY_BEGIN, vec![], Message::InventoryBegin);
    check(RTXTOOAM_INVENTORY_END, vec![], Message::InventoryEnd);

    // §4.2.1.8
    check(RTXTOOAM_EVENT_UPSSTATUS, B::new().u16(1).done(), Message::UpsStatus(UpsStatus { ups_status: 1 }));

    // §4.2.1.10 and §4.2.1.11: one layout, two ids.
    let zone_event = ParcelZoneEvent {
        object_id: 0x0102_0304,
        cell: 0x0506,
        cell_nif_position: 0x0708,
        zone_id: 0x090A,
        operator_id: 11,
        used_cells: 2,
        current_anomaly: 16,
    };
    let zone_body =
        B::new().u32(0x0102_0304).u16(0x0506).u16(0x0708).u16(0x090A).u8(11).u8(2).u8(16).done();
    check(RTXTOOAM_PARCEL_RECIRCULATED, zone_body.clone(), Message::ParcelRecirculated(zone_event.clone()));
    check(RTXTOOAM_PARCEL_ON_CTRLZONE, zone_body, Message::ParcelOnCtrlZone(zone_event));

    // §4.2.2.1
    check(
        RTXTOOAM_SCANNER,
        B::new()
            .u8(1)
            .u8(11)
            .u16(0x0102)
            .u8(2)
            .u32(0x0A0B_0C0D)
            .u8(3)
            .u8(16)
            .u16(0x1112)
            .raw(&pat(240, 9))
            .raw(&pat(240, 13))
            .done(),
        Message::Scanner(ScannerRequest {
            scn_id: 1,
            operator_id: 11,
            cell: 0x0102,
            used_cells: 2,
            object_id: 0x0A0B_0C0D,
            request_type: 3,
            request_anomaly: 16,
            cell_nif_position: 0x1112,
            msc_data: pat(240, 9),
            user_data: pat(240, 13),
        }),
    );

    // §4.2.2.2
    check(
        RTXTOOAM_MSCDEVICES,
        B::new().u32(0x0102_0304).u8(5).u16(0x0607).u32(0x0A0B_0C0D).raw(&pat(240, 4)).done(),
        Message::MscDevices(MscDevices {
            device_type: 0x0102_0304,
            device_id: 5,
            cell: 0x0607,
            object_id: 0x0A0B_0C0D,
            msc_data: pat(240, 4),
        }),
    );

    // §4.2.2.3
    check(
        RTXTOOAM_PARCEL_LOADED,
        B::new().u32(0x0102_0304).u16(0x0506).u16(0x0708).u16(0x090A).u8(11).u8(2).u8(3).u8(1).done(),
        Message::ParcelLoaded(ParcelLoaded {
            object_id: 0x0102_0304,
            cell: 0x0506,
            cell_nif_position: 0x0708,
            zone_id: 0x090A,
            operator_id: 11,
            used_cells: 2,
            loaded_type: 3,
            loaded_anomaly: 1,
        }),
    );

    // §5.1.1.1: bLoadItemAnyway is a four-byte BOOL.
    check(
        RTXTOOAM_CODINGREQUEST,
        B::new().u8(12).u16(0x3344).raw(&pat(240, 2)).u32(1).done(),
        Message::CodingRequest(CodingRequest {
            operator_id: 12,
            timeout_10ms: 0x3344,
            user_data: pat(240, 2),
            load_item_anyway: true,
        }),
    );

    // §5.1.1.2: bIsItemAssigned is a four-byte BOOL.
    check(
        RTXTOOAM_CODINGRESULT,
        B::new().u8(12).u32(1).u16(0x0102).done(),
        Message::CodingResult(CodingResult { operator_id: 12, item_assigned: true, cell: 0x0102 }),
    );
}

#[test]
fn golden_bodies_side_families() {
    use ids::*;

    // §4.2.1.9 (id §10.16): bSwitchON is a four-byte BOOL.
    check(CSPCTOEUI_LAMPTEST, B::new().u32(1).done(), Message::LampTest(LampTest { switch_on: true }));

    // §6.1 (id §10.18)
    check(
        GENERIC_DATAEXCHANGE,
        B::new().u16(0).u16(11).u16(2).u16(12).raw(&pat(498, 6)).done(),
        Message::GenericDataExchange(GenericDataExchange {
            sender_msg_id: 0,
            sender_who_id: 11,
            receiver_msg_id: 2,
            receiver_who_id: 12,
            data: pat(498, 6),
        }),
    );

    // §7.1 (id §10.21): eITEMLOST is a four-byte enum.
    check(
        CONVSYSTOOAM_ITEMLOST,
        B::new().u8(11).u8(3).u32(0x0102_0304).u8(9).u32(2).raw(&pat(240, 8)).done(),
        Message::ItemLost(ItemLost {
            operator_id: 11,
            conv_sys_id: 3,
            object_id: 0x0102_0304,
            lost_position: 9,
            reason: 2,
            custom_data: pat(240, 8),
        }),
    );
}

#[test]
fn golden_ispc_msc_data_and_fsg_statistics() {
    // §4.2.2.1: STRUCT_ISPC_MSC_DATA (44 fixed bytes), then STRUCT_FSG_STATISTIC_DATA (95),
    // then the plant-custom tail, filling the 240-byte szUserData.
    assert_eq!((IspcMscData::FIXED_LEN, FsgStatisticData::LEN), (44 + 95, 95));
    let body = B::new()
        .u8(11) // byISPC_ID
        .u16(0x0102) // wFirstUsedCell
        .u8(2) // byUsedCells
        .u8(0x55) // byUsedGaps
        .u8(0x66) // byLoadFlags
        .u16(0x0304) // wRejectReason
        .u32(0x0A0B_0C0D) // dwObjectID
        .u32(0x1A1B_1C1D) // dwISTrackingID
        .u16(400) // wLength
        .u16(300) // wWidth
        .u16(250) // wHeight
        .u32(30_000_000) // dwVolume
        .u16(2500) // wWeight
        .u16(7) // wHeadOffset
        .u16(42) // wDestination
        .u16(3) // wCoilID_Master
        .u32(0x00AB_CDEF) // dwAntennaMission_Master
        .u16(4) // wCoilID_Backup
        .u32(0x0012_3456) // dwAntennaMission_Backup
        .u8(2) // bySubsystemType
        .u8(3) // byGroupId
        .u8(4) // bySubsystemId
        .u8(5) // byZoneId
        .u8(6) // byScaleId
        .u8(7) // byVolumeId
        .u8(8) // byLFTW
        .u8(9) // byLFTV
        .u8(10) // byScaleOperativeMode
        .u8(11) // byVolumeOperativeMode
        .u32(0xDEAD_BEEF) // dwScaleError
        .u32(0xCAFE_F00D) // dwVolumeError
        .raw(&pat(35, 1)) // byArrayAlibiScale
        .raw(&pat(42, 2)) // byArrayAlibiVolume
        .raw(&pat(101, 3)) // csObjectData, the rest of the 240
        .done();
    assert_eq!(body.len(), 240);
    let expected = IspcMscData {
        ispc_id: 11,
        first_used_cell: 0x0102,
        used_cells: 2,
        used_gaps: 0x55,
        load_flags: 0x66,
        reject_reason: 0x0304,
        object_id: 0x0A0B_0C0D,
        tracking_id: 0x1A1B_1C1D,
        length_mm: 400,
        width_mm: 300,
        height_mm: 250,
        volume: 30_000_000,
        weight_g: 2500,
        head_offset: 7,
        destination: 42,
        coil_id_master: 3,
        antenna_mission_master: 0x00AB_CDEF,
        coil_id_backup: 4,
        antenna_mission_backup: 0x0012_3456,
        statistics: FsgStatisticData {
            subsystem_type: 2,
            group_id: 3,
            subsystem_id: 4,
            zone_id: 5,
            scale_id: 6,
            volume_id: 7,
            lftw: 8,
            lftv: 9,
            scale_operative_mode: 10,
            volume_operative_mode: 11,
            scale_error: 0xDEAD_BEEF,
            volume_error: 0xCAFE_F00D,
            alibi_scale: pat(35, 1),
            alibi_volume: pat(42, 2),
        },
        object_data: pat(101, 3),
    };
    assert_eq!(IspcMscData::decode(&body, Endian::Little), expected);
    assert_eq!(expected.encode(240, Endian::Little), body);
    // And reached the way the reply path reaches it, through the scanner request.
    let request = ScannerRequest { user_data: body, ..Default::default() };
    assert_eq!(request.ispc(Endian::Little), expected);
}

#[test]
fn outcome_returns_and_anomalies_are_the_spec_numbers_with_their_words() {
    use outcome_anomaly as a;
    use outcome_return as r;

    // §10.3, wReturn.
    assert_eq!(
        [
            r::SORTED,
            r::REJECTED,
            r::REASSIGNEMENT,
            r::UNLOAD,
            r::PARCELFOUND,
            r::PARCELLOST,
            r::TO_REJECT,
            r::DATA_ERROR,
            r::CELL_ACTION_BUILT,
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
    const RETURN_WORDS: [&str; 9] = [
        "sorted",
        "rejected",
        "reassignment",
        "unload",
        "parcel found",
        "parcel lost",
        "to reject",
        "data error",
        "cell action built",
    ];
    for v in 0..=u16::MAX {
        let expected = RETURN_WORDS.get(v as usize).copied().unwrap_or("unknown");
        assert_eq!(r::name(v), expected, "wReturn {v}");
    }

    // §10.3, byAnomaly: 0..=22 and the chute-manager block 100..=110. 23 and 111 are range
    // markers in the spec, not codes, so they read as unknown like every other number.
    assert_eq!(
        [
            a::NOTHING,
            a::ENABLE,
            a::CELL_FAULT,
            a::UNLOADS,
            a::CHUTE_FAULT,
            a::CHUTE_DISABLED,
            a::CHUTE_JAM,
            a::CHUTE_OAM_BLOCK,
            a::CHUTE_ON_TRACKING,
            a::CHUTE_EXTERNAL_BLOCK_SIGNAL,
            a::CHUTE_FLAP,
            a::CHUTE_FULL,
            a::NO_DESTINATION,
            a::UNEXPECTED,
            a::UNSORTED,
            a::NOTREAS,
            a::ONGAP,
            a::ISPC,
            a::CONCURRENT_UNLOADS,
            a::UNLOAD_RATE_LIMIT,
            a::SPECIAL_NOT_ALLOWED,
            a::NOT_DECENTERED,
            a::INFLIGHT_RATE_LIMIT,
            a::CHUTE_TIMEOUT,
            a::CHUTE_NAK_0,
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 100, 101]
    );
    const ANOMALY_WORDS: [&str; 23] = [
        "none",
        "sorter not ready",
        "cell fault",
        "unload write failed",
        "chute fault",
        "chute disabled",
        "chute jam",
        "chute blocked by the external unit",
        "chute under tracking",
        "chute blocked by an external signal",
        "chute flap not ready",
        "chute full",
        "no destination received",
        "unexpected object",
        "unsorted object",
        "never reassigned",
        "loaded between cells",
        "rejected by the induction station",
        "too many concurrent unloads",
        "unload rate limit",
        "special condition forbids sorting",
        "cell not de-centered",
        "in-flight rate limit",
    ];
    for v in 0..=u8::MAX {
        let expected = match v {
            0..=22 => ANOMALY_WORDS[v as usize],
            100 => "chute I/O timeout",
            101..=110 => "chute manager refusal",
            _ => "unknown",
        };
        assert_eq!(a::name(v), expected, "byAnomaly {v}");
    }
}

#[test]
fn destination_flags_are_the_spec_numbers_with_their_words() {
    use dest_flag as f;

    // §10.6
    assert_eq!(
        [
            f::SCANNER_ONCEAGAIN,
            f::FORCE_REASSIGNEMENT,
            f::WAIT_FOR_DESTINATION,
            f::ASSIGNED,
            f::ASSIGNED_ORDER_BY_DEST,
            f::PRIMARY_PLUS_ALTERNATE,
            f::ALTERNATE_IF_NO_PRIMARY,
        ],
        [0, 1, 2, 3, 4, 5, 6]
    );
    const FLAG_WORDS: [&str; 7] = [
        "scanner once again",
        "force reassignment",
        "wait for destination",
        "assigned",
        "assigned, ordered by destination",
        "primary plus alternate",
        "alternate if no primary",
    ];
    for v in 0..=u8::MAX {
        let expected = FLAG_WORDS.get(v as usize).copied().unwrap_or("unknown");
        assert_eq!(f::name(v), expected, "byFlag {v}");
    }
}

/// §4.2.1.4: every named dwState bit, in bit order, with its word. Bits 26 and 27 are reserved
/// by the spec and carry no word.
const BIN_STATE_BITS: [(u32, &str); 30] = [
    (0x0000_0001, "running"),
    (0x0000_0002, "emptying"),
    (0x0000_0004, "startup"),
    (0x0000_0008, "energy saving"),
    (0x0000_0010, "manual"),
    (0x0000_0020, "maintenance"),
    (0x0000_0040, "auto recovery"),
    (0x0000_0080, "emergency"),
    (0x0000_0100, "external fault"),
    (0x0000_0200, "speed ok"),
    (0x0000_0400, "jam"),
    (0x0000_0800, "full"),
    (0x0000_1000, "alarm"),
    (0x0000_2000, "warning"),
    (0x0000_4000, "half full"),
    (0x0000_8000, "booked"),
    (0x0001_0000, "prefilling"),
    (0x0002_0000, "completing"),
    (0x0004_0000, "completed"),
    (0x0008_0000, "waiting label"),
    (0x0010_0000, "disabled"),
    (0x0020_0000, "blocked by the external unit"),
    (0x0040_0000, "component fault"),
    (0x0080_0000, "blocked externally"),
    (0x0100_0000, "blocked by tracking"),
    (0x0200_0000, "operator present"),
    (0x1000_0000, "mapping"),
    (0x2000_0000, "emergency tristate"),
    (0x4000_0000, "alarm tristate"),
    (0x8000_0000, "warning tristate"),
];
const BIN_STATE_RESERVED: u32 = 0x0400_0000 | 0x0800_0000;

#[test]
fn bin_state_bits_are_the_spec_table_with_their_words() {
    use bin_state as s;

    // The constants, by value.
    let by_name = [
        s::RUNNING,
        s::EMPTYING,
        s::STARTUP,
        s::ENERGY_SAVING,
        s::MANUAL,
        s::MAINTENANCE,
        s::AUTORECOVERY,
        s::EMERGENCY,
        s::EXTERNAL_FAULT,
        s::SPEED_OK,
        s::JAM,
        s::FULL,
        s::ALARM,
        s::WARNING,
        s::HALF_FULL,
        s::BOOKED,
        s::PREFILLING,
        s::COMPLETING,
        s::COMPLETED,
        s::WAITING_LABEL,
        s::DISABLE,
        s::BLOCKED_BY_OAM,
        s::COMPONENT_FAULT,
        s::BLOCKED_BY_EXT,
        s::BLOCKED_BY_TRACKING,
        s::OPERATOR_PRESENCE,
        s::MAPPING,
        s::EMERGENCY_TRISTATE,
        s::ALARM_TRISTATE,
        s::WARNING_TRISTATE,
    ];
    assert_eq!(by_name, BIN_STATE_BITS.map(|(bit, _)| bit));

    // The table a UI builds its checkboxes from, exactly.
    assert_eq!(s::BITS.as_slice(), BIN_STATE_BITS.as_slice());

    // Each bit alone reads as its one word; every bit reads in bit order; reserved bits and an
    // empty state read as nothing.
    for (bit, word) in BIN_STATE_BITS {
        assert_eq!(s::names(bit), vec![word], "{bit:#010x}");
    }
    let every: Vec<&str> = BIN_STATE_BITS.iter().map(|(_, word)| *word).collect();
    assert_eq!(s::names(u32::MAX), every);
    assert!(s::names(BIN_STATE_RESERVED).is_empty());
    assert!(s::names(0).is_empty());

    // Whether a chute takes parcels, asked with the spec numbers rather than the constants, so a
    // swapped FULL/JAM fails here. The 26 original bits; the TRISTATE bits are Q7.
    const BLOCKING: [u32; 11] = [
        0x0000_0020, // maintenance
        0x0000_0080, // emergency
        0x0000_0100, // external fault
        0x0000_0400, // jam
        0x0000_0800, // full
        0x0000_1000, // alarm
        0x0010_0000, // disabled
        0x0020_0000, // blocked by the external unit
        0x0040_0000, // component fault
        0x0080_0000, // blocked externally
        0x0100_0000, // blocked by tracking
    ];
    for (bit, word) in BIN_STATE_BITS.iter().take(26) {
        assert_eq!(s::accepts_parcels(*bit), !BLOCKING.contains(bit), "{word}");
    }
}

#[test]
fn an_output_name_keeps_its_terminator() {
    // §4.1.1.7: the last of the 26 bytes is always zero, so a longer name is cut at 25.
    let mut name = vec![b'X'; 25];
    name.push(0);
    let body = B::new().u8(55).u16(7).raw(&name).u8(1).u16(500).u16(600).done();
    for long in [26, 40] {
        let msg = Message::OutputSignals(OutputSignals {
            class_id: 55,
            device_id: 7,
            output_name: "X".repeat(long),
            output_state: 1,
            time_on_ms: 500,
            time_off_ms: 600,
        });
        assert_eq!(msg.encode(Endian::Little), Ok(body.clone()), "{long} chars");
    }
}
