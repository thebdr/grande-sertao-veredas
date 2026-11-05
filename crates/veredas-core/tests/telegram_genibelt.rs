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
