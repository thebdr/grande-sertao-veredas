//! The poll worker (C-022): its gated writes, then the worker against the demo plant over real
//! OPC UA: polling, mapping, writing, reading on demand, and what it does when the server is
//! absent, silent, killed, or slow. Veredas 1's unit and end-to-end tests, plus the guarantees
//! the port added (a request without a session answered at once, the gate re-checked before the
//! wire, a bounded connect, a refused zero interval, quality, both clocks).

use std::net::TcpListener;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use opcua_types::DataValue;
use veredas_core::opc::demo::{BULK_COUNT, DEMO_PASS, DEMO_USER, DemoPlant, DetachedPlant, Relay, SLOW_READ, tags};
use veredas_core::opc::spec::{Auth, BitSpec, PollSpec, RuleSpec, TagSpec};
use veredas_core::opc::worker::{
    CALL_TIMEOUT, CONNECT_TIMEOUT, ConnState, OpcEvent, OpcHandle, RETRY_MAX, RETRY_MIN, TagRead,
};
use veredas_core::opc::{
    Call, NodeId, OpcError, StatusCode, Variant, WriteError, WriteGate, map_poll, parse_node_id, retry_delays, tag_reads,
};
use veredas_core::{Quality, SeriesType, SourceError, Value, now_unix_ms};

fn tag(node: &str, series: &str, ty: SeriesType, writable: bool, bits: Vec<BitSpec>) -> TagSpec {
    TagSpec { node_id: parse_node_id(node).unwrap(), raw: node.to_string(), series: series.to_string(), ty, writable, bits }
}

fn one_rule(source: &str, url: &str, poll_ms: u64, tags: Vec<TagSpec>) -> PollSpec {
    PollSpec {
        source_id: source.into(),
        display_name: source.into(),
        endpoint_url: url.into(),
        auth: Auth::Anonymous,
        poll: Duration::from_millis(poll_ms),
        rules: vec![RuleSpec { name: "line1".into(), tags, store_on_change: false }],
    }
}

/// The demo plant's float, its packed word with two bit fields, and its writable command.
fn line1(url: &str, ns: impl Fn(&str) -> String, poll_ms: u64) -> PollSpec {
    one_rule(
        "plc",
        url,
        poll_ms,
        vec![
            tag(&ns(tags::TEMPERATURE), "temperature", SeriesType::Float, false, vec![]),
            tag(
                &ns(tags::STATUS_WORD),
                "status_word",
                SeriesType::Int,
                false,
                vec![
                    BitSpec { bit: 0, width: 1, series: "running".into() },
                    BitSpec { bit: 4, width: 3, series: "mode".into() },
                ],
            ),
            tag(&ns(tags::CMD_START), "cmd_start", SeriesType::Int, true, vec![]),
        ],
    )
}

fn spec_for(plant: &DemoPlant, poll_ms: u64) -> PollSpec {
    line1(&plant.endpoint_url, |t| plant.node_id_str(t), poll_ms)
}

/// The next `Poll`, skipping status chatter.
fn next_poll(rx: &Receiver<OpcEvent>, within: Duration) -> Vec<TagRead> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(OpcEvent::Poll { values, .. }) => return values,
            Ok(OpcEvent::Status { .. }) => continue,
            Err(RecvTimeoutError::Timeout) => panic!("no poll arrived within {within:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the worker exited"),
        }
    }
}

/// The next status, skipping polls.
fn next_status(rx: &Receiver<OpcEvent>, within: Duration) -> ConnState {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(OpcEvent::Status { state, .. }) => return state,
            Ok(OpcEvent::Poll { .. }) => continue,
            Err(e) => panic!("no status within {within:?}: {e:?}"),
        }
    }
}

// ---- the gates, on a worker that has stopped ------------------------------------------------

/// A writer whose worker has stopped: a real worker, spawned through the public path at an
/// endpoint nobody serves, then joined, so its job channel is closed. Anything that passes the
/// gates comes back `NotSent(WorkerGone)`; only a gate can answer otherwise.
fn orphan_handle(armed: bool) -> (OpcHandle, WriteGate) {
    let gate = WriteGate::new(armed);
    let spec = one_rule(
        "plc",
        "opc.tcp://127.0.0.1:1/x",
        500,
        vec![
            tag("ns=1;s=cmd", "cmd", SeriesType::Int, true, vec![]),
            tag("ns=1;s=temp", "temp", SeriesType::Float, false, vec![]),
        ],
    );
    let (events, _ignored) = channel();
    let worker = veredas_core::opc::spawn(spec, events, gate.clone()).unwrap();
    worker.handle.join().unwrap();
    (worker.writer, gate)
}

/// The master switch is checked FIRST, before the whitelist and before the channel: this
/// handle's worker is gone, so an ungated write could only come back `Transport`.
#[tokio::test]
async fn disarmed_refuses_before_the_wire() {
    let (handle, _gate) = orphan_handle(false);
    let refused = handle.write_tags(vec![("ns=1;s=cmd".into(), Variant::Int32(1))]).await.unwrap_err();
    assert_eq!(refused, WriteError::Disarmed);
    assert_eq!(refused.to_string(), "writes are turned off for this project");
    assert!(!handle.is_armed());
    // Disarmed outranks the whitelist: a read-only tag is refused as Disarmed too.
    let refused = handle.write_tags(vec![("ns=1;s=temp".into(), Variant::Double(1.0))]).await.unwrap_err();
    assert_eq!(refused, WriteError::Disarmed);
}

/// Armed, but the tag is not marked writable: refused by the whitelist, before the channel.
#[tokio::test]
async fn armed_still_refuses_a_read_only_tag() {
    let (handle, _gate) = orphan_handle(true);
    let refused = handle.write_tags(vec![("ns=1;s=temp".into(), Variant::Double(1.0))]).await.unwrap_err();
    assert_eq!(refused, WriteError::NotWritable { source_id: "plc".into(), tag: "ns=1;s=temp".into() });
    assert_eq!(refused.to_string(), "`plc` / `ns=1;s=temp` is not a writable tag in this session");
}

/// An unknown tag and a read-only one are refused identically: a caller cannot map the
/// address space by watching which error comes back.
#[tokio::test]
async fn unknown_and_read_only_are_indistinguishable() {
    let (handle, _gate) = orphan_handle(true);
    let unknown = handle.write_tags(vec![("ns=1;s=nothing".into(), Variant::Int32(1))]).await.unwrap_err();
    assert_eq!(unknown, WriteError::NotWritable { source_id: "plc".into(), tag: "ns=1;s=nothing".into() });
}

/// A batch is all-or-nothing at the gate: one tag not permitted refuses the whole call, in
/// either position, and the permitted tag in it never reaches the channel either.
#[tokio::test]
async fn one_bad_tag_refuses_the_whole_batch() {
    let (handle, _gate) = orphan_handle(true);
    for batch in [
        vec![("ns=1;s=cmd".to_string(), Variant::Int32(1)), ("ns=1;s=temp".to_string(), Variant::Double(2.0))],
        vec![("ns=1;s=temp".to_string(), Variant::Double(2.0)), ("ns=1;s=cmd".to_string(), Variant::Int32(1))],
    ] {
        let refused = handle.write_tags(batch).await.unwrap_err();
        assert_eq!(refused, WriteError::NotWritable { source_id: "plc".into(), tag: "ns=1;s=temp".into() });
    }
}

/// Armed and writable: the gate lets it through, and the only thing left to fail is the absent
/// worker. The control for the tests above: without it they would pass on a gate that refused
/// everything.
#[tokio::test]
async fn a_permitted_write_reaches_the_channel() {
    let (handle, _gate) = orphan_handle(true);
    let err = handle.write_tags(vec![(" ns=1;s=cmd ".into(), Variant::Int32(1))]).await.unwrap_err();
    assert_eq!(err, WriteError::NotSent(OpcError::WorkerGone));
    assert_eq!(err.to_string(), "the OPC worker is not running");
    // An empty batch passes the gates too.
    assert_eq!(handle.write_tags(vec![]).await, Err(WriteError::NotSent(OpcError::WorkerGone)));
    assert_eq!(handle.read(vec![NodeId::new(1, "temp")]).await, Err(OpcError::WorkerGone));
}

/// Disarming is live: the same handle that just passed is refused after the shared gate flips.
#[tokio::test]
async fn the_gate_flips_live() {
    let (handle, gate) = orphan_handle(true);
    let write = || handle.write_tags(vec![("ns=1;s=cmd".into(), Variant::Int32(1))]);
    assert_eq!(write().await, Err(WriteError::NotSent(OpcError::WorkerGone)));
    gate.set(false);
    assert_eq!(write().await, Err(WriteError::Disarmed));
    assert!(!handle.is_armed());
    gate.set(true);
    assert_eq!(write().await, Err(WriteError::NotSent(OpcError::WorkerGone)));
    // A clone of the gate is the same switch.
    gate.clone().set(false);
    assert_eq!(write().await, Err(WriteError::Disarmed));
}

#[test]
fn writable_type_answers_only_for_writable_tags() {
    let (handle, _gate) = orphan_handle(true);
    assert_eq!(handle.writable_type("ns=1;s=cmd"), Some(SeriesType::Int));
    assert_eq!(handle.writable_type("ns=1;s=temp"), None);
    assert_eq!(handle.writable_type("ns=1;s=nothing"), None);
    assert_eq!(handle.writable_type("  ns=1;s=cmd  "), Some(SeriesType::Int));
    assert_eq!(handle.spec().source_id, "plc");
    assert_eq!(handle.source_id, "plc");
}

#[test]
fn a_zero_poll_interval_is_refused_at_spawn() {
    let mut spec = one_rule("plc", "opc.tcp://127.0.0.1:1/", 0, vec![]);
    spec.poll = Duration::ZERO;
    let (tx, _rx) = channel();
    let err = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).err().expect("refused");
    assert!(matches!(err, SourceError::NotConfigured(_)), "{err:?}");
    assert_eq!(err.to_string(), "source not configured: OPC source `plc`: the poll interval is zero");
}

#[test]
fn a_reading_is_good_only_with_a_value_and_a_good_status() {
    let node = NodeId::new(1, "t");
    let read = |value: Option<Variant>, status: StatusCode| TagRead { node_id: node.clone(), value, status };
    assert_eq!(read(Some(Variant::Int32(1)), StatusCode::Good).quality(), Quality::Good);
    assert!(read(Some(Variant::Int32(1)), StatusCode::Good).is_good());
    // A good status that carried nothing is no reading.
    assert_eq!(read(None, StatusCode::Good).quality(), Quality::Bad);
    assert!(!read(None, StatusCode::Good).is_good());
    assert_eq!(read(None, StatusCode::UncertainLastUsableValue).quality(), Quality::Uncertain);
    assert_eq!(read(None, StatusCode::BadNodeIdUnknown).quality(), Quality::Bad);
    // A value beside a bad or uncertain status is not trusted either.
    assert_eq!(read(Some(Variant::Int32(1)), StatusCode::BadNoCommunication).quality(), Quality::Bad);
    assert_eq!(read(Some(Variant::Int32(1)), StatusCode::UncertainInitialValue).quality(), Quality::Uncertain);
}

// ---- against the demo plant ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_worker_polls_and_its_values_map_to_typed_rows() {
    let plant = DemoPlant::start().await.expect("demo plant");
    plant.set(tags::TEMPERATURE, 42.5f64);
    plant.set(tags::STATUS_WORD, 0b0101_0001u16);

    let spec = spec_for(&plant, 100);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    assert_eq!(next_status(&rx, Duration::from_secs(5)), ConnState::Connecting);
    assert_eq!(next_status(&rx, Duration::from_secs(15)), ConnState::Connected);

    // The first poll carries every node once, in poll order, good.
    let before = now_unix_ms();
    let (index_ms, arrival_ms, reads) = match rx.recv_timeout(Duration::from_secs(15)).expect("a poll") {
        OpcEvent::Poll { source_id, index_ms, arrival_ms, values } => {
            assert_eq!(source_id, "plc");
            (index_ms, arrival_ms, values)
        }
        OpcEvent::Status { state, .. } => panic!("unexpected status {state:?}"),
    };
    let after = now_unix_ms();
    assert!(arrival_ms >= before - 1_000 && arrival_ms <= after, "{before} <= {arrival_ms} <= {after}");
    // The plant axis is local wall clock: it sits the UTC offset away from arrival.
    let offset_ms = i64::from(chrono::Local::now().offset().local_minus_utc()) * 1_000;
    assert!((index_ms - arrival_ms - offset_ms).abs() < 2_000, "index {index_ms}, arrival {arrival_ms}, offset {offset_ms}");
    assert_eq!(reads.iter().map(|r| r.node_id.clone()).collect::<Vec<_>>(), spec.poll_nodes());
    assert!(reads.iter().all(|r| r.is_good() && r.quality() == Quality::Good), "{reads:?}");

    let mapped = map_poll(&spec, &reads, 1_000, 0);
    assert!(mapped.failures.is_empty(), "{:?}", mapped.failures);
    assert_eq!(
        mapped.rows[0].values,
        vec![
            Some(Value::Float(42.5)),
            Some(Value::Int(0b0101_0001)),
            Some(Value::Int(1)), // bit 0: running
            Some(Value::Int(5)), // bits 4..6: mode
            Some(Value::Int(0)), // cmd_start
        ]
    );

    // It keeps polling, and picks up a change the plant makes mid-run.
    plant.set(tags::TEMPERATURE, 43.5f64);
    let mut saw_new = false;
    for _ in 0..40 {
        let reads = next_poll(&rx, Duration::from_secs(15));
        if map_poll(&spec, &reads, 0, 0).rows[0].values[0] == Some(Value::Float(43.5)) {
            saw_new = true;
            break;
        }
    }
    assert!(saw_new, "the worker never reported the changed value");

    // A stop while connected is prompt.
    let t0 = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_reaches_the_plant_through_the_running_worker() {
    // The poll loop keeps running while a write is served on the SAME session.
    let plant = DemoPlant::start().await.expect("demo plant");
    let mut spec = spec_for(&plant, 100);
    spec.rules[0].tags[0].writable = true; // the temperature, read-only on the plant
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(true)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));

    let cmd_start = plant.node_id_str(tags::CMD_START);
    let statuses = worker.writer.write_tags(vec![(cmd_start.clone(), Variant::Int32(1))]).await.expect("write");
    assert_eq!(statuses, vec![StatusCode::Good]);
    // Server-side truth.
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));

    // The poll loop survived the write and reports the new value.
    let mut saw = false;
    for _ in 0..40 {
        let reads = next_poll(&rx, Duration::from_secs(15));
        if map_poll(&spec, &reads, 0, 0).rows[0].values[4] == Some(Value::Int(1)) {
            saw = true;
            break;
        }
    }
    assert!(saw, "the poll never reflected the written value");

    // The other half of a pulse.
    worker.writer.write_tags(vec![(cmd_start, Variant::Int32(0))]).await.expect("write");
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)));

    // A server that takes the call but refuses one write answers per write, in order, not as a
    // refusal: the temperature is writable in this spec's whitelist and read-only on the plant.
    plant.set(tags::TEMPERATURE, 20.0f64);
    let temperature = plant.node_id_str(tags::TEMPERATURE);
    let statuses = worker
        .writer
        .write_tags(vec![(temperature, Variant::Double(99.0)), (plant.node_id_str(tags::CMD_START), Variant::Int32(0))])
        .await
        .expect("the call itself succeeds");
    assert_eq!(statuses.len(), 2);
    assert!(statuses[0].is_bad(), "{statuses:?}");
    assert_eq!(statuses[1], StatusCode::Good);
    assert_eq!(plant.get(tags::TEMPERATURE), Some(Variant::Double(20.0)));

    // An on-demand read, of any node, on the same session, positionally.
    plant.set(tags::SETPOINT, 12.25f64);
    let reads = worker
        .writer
        .read(vec![
            parse_node_id(&plant.node_id_str(tags::SETPOINT)).unwrap(),
            NodeId::new(9, "no.such.tag"),
            parse_node_id(&plant.node_id_str(tags::BATCH_ID)).unwrap(),
        ])
        .await
        .expect("read");
    assert_eq!(reads.len(), 3);
    assert_eq!(reads[0].value, Some(Variant::Double(12.25)));
    assert_eq!(reads[0].quality(), Quality::Good);
    assert_eq!((reads[1].value.clone(), reads[1].status), (None, StatusCode::BadNodeIdUnknown));
    assert_eq!(reads[1].quality(), Quality::Bad);
    assert_eq!(reads[2].value, Some(Variant::from("B-000")));

    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

/// The gates, proven where it matters: against a REAL server, from an in-process caller holding
/// the handle. The assertion that carries the test is the plant-side value: not merely "an
/// error came back", but "the plant never moved".
#[tokio::test(flavor = "multi_thread")]
async fn a_disarmed_master_switch_blocks_an_in_process_write() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let spec = spec_for(&plant, 100);
    let (tx, rx) = channel();
    let gate = WriteGate::new(false);
    let worker = veredas_core::opc::spawn(spec.clone(), tx, gate.clone()).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));

    let cmd_start = plant.node_id_str(tags::CMD_START);
    plant.set(tags::CMD_START, 0i32);

    // Disarmed: refused, and the plant is untouched.
    let refused = worker.writer.write_tags(vec![(cmd_start.clone(), Variant::Int32(1))]).await.unwrap_err();
    assert_eq!(refused, WriteError::Disarmed);
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)), "the write must not have reached the plant");

    // Armed live (no new session, no new handle) and the same call lands.
    gate.set(true);
    let statuses = worker.writer.write_tags(vec![(cmd_start.clone(), Variant::Int32(1))]).await.expect("armed");
    assert_eq!(statuses, vec![StatusCode::Good]);
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));

    // Disarmed again mid-session: stops at once, and the value the armed write left is neither
    // rolled back nor written again.
    gate.set(false);
    let refused = worker.writer.write_tags(vec![(cmd_start, Variant::Int32(0))]).await.unwrap_err();
    assert_eq!(refused, WriteError::Disarmed);
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));

    // A tag the spec does not mark writable is refused even while armed, though the server
    // would take it (the setpoint is writable on the plant), and the plant does not move.
    gate.set(true);
    let mut readonly = spec.clone();
    readonly.rules[0].tags.push(tag(&plant.node_id_str(tags::SETPOINT), "setpoint", SeriesType::Float, false, vec![]));
    let (tx2, _rx2) = channel();
    let other = veredas_core::opc::spawn(readonly, tx2, gate.clone()).expect("worker spawns");
    plant.set(tags::SETPOINT, 5.0f64);
    let refused = other.writer.write_tags(vec![(plant.node_id_str(tags::SETPOINT), Variant::Double(99.0))]).await.unwrap_err();
    assert!(matches!(refused, WriteError::NotWritable { .. }));
    assert_eq!(plant.get(tags::SETPOINT), Some(Variant::Double(5.0)));

    other.handle.join().expect("clean shutdown");
    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

/// A write that passed the handle's check and waits in the worker's queue meets the switch
/// again before the wire: disarmed meanwhile, it is refused and the plant never moves. The
/// demo plant's slow tag holds the worker for SLOW_READ so the write is surely queued; the
/// control half proves the same queued write lands when the switch stays armed.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_write_meets_the_switch_again_before_the_wire() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let spec = spec_for(&plant, 60_000);
    let (tx, rx) = channel();
    let gate = WriteGate::new(true);
    let worker = veredas_core::opc::spawn(spec, tx, gate.clone()).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    let slow = parse_node_id(&plant.node_id_str(tags::SLOW)).unwrap();
    let cmd = plant.node_id_str(tags::CMD_START);

    for disarm in [true, false] {
        plant.set(tags::CMD_START, 0i32);
        gate.set(true);
        let reader = worker.writer.clone();
        let slow_id = slow.clone();
        let slow_read = tokio::spawn(async move {
            let t0 = Instant::now();
            (reader.read(vec![slow_id]).await, t0.elapsed())
        });
        tokio::time::sleep(Duration::from_millis(100)).await; // the worker is inside the slow read
        let writer = worker.writer.clone();
        let target = cmd.clone();
        let write = tokio::spawn(async move {
            let t0 = Instant::now();
            (writer.write_tags(vec![(target, Variant::Int32(1))]).await, t0.elapsed())
        });
        tokio::time::sleep(Duration::from_millis(100)).await; // the write passed the handle's check
        if disarm {
            gate.set(false);
        }
        let (written, waited) = write.await.unwrap();
        // It waited behind the slow read: the answer came from the worker, not the handle.
        assert!(waited >= Duration::from_millis(150), "{waited:?}");
        let (read, took) = slow_read.await.unwrap();
        assert_eq!(read.unwrap()[0].value, Some(Variant::Int32(0)));
        assert!(took >= SLOW_READ, "{took:?}");
        if disarm {
            assert_eq!(written, Err(WriteError::Disarmed));
            assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)), "the queued write reached the plant");
        } else {
            assert_eq!(written, Ok(vec![StatusCode::Good]));
            assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));
        }
    }

    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_bad_node_does_not_stop_the_poll() {
    // A mistyped node id costs that column, not the whole source.
    let plant = DemoPlant::start().await.expect("demo plant");
    plant.set(tags::TEMPERATURE, 12.5f64);
    let mut spec = spec_for(&plant, 100);
    spec.rules[0].tags.push(tag("ns=9;s=no.such.tag", "ghost", SeriesType::Float, false, vec![]));

    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    let reads = next_poll(&rx, Duration::from_secs(15));
    assert_eq!(reads[3].status, StatusCode::BadNodeIdUnknown);
    assert_eq!(reads[3].quality(), Quality::Bad);

    let mapped = map_poll(&spec, &reads, 0, 0);
    // The good columns are intact; the bad one is a hole, REPORTED rather than swallowed.
    assert_eq!(mapped.rows[0].values[0], Some(Value::Float(12.5)));
    assert_eq!(mapped.rows[0].values.last(), Some(&None));
    assert_eq!(mapped.failures, vec![("ghost".to_string(), "BadNodeIdUnknown".to_string())]);
    // And the poll goes on.
    next_poll(&rx, Duration::from_secs(15));

    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

#[test]
fn an_unreachable_server_retries_instead_of_dying() {
    // The plant is off when the source starts: the worker stays alive and keeps saying why.
    let spec = one_rule("ghost", "opc.tcp://127.0.0.1:1/", 100, vec![tag("ns=1;s=t", "temp", SeriesType::Float, false, vec![])]);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");

    // Connecting, then Retrying with a growing backoff, again and again.
    let mut waits = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while waits.len() < 3 && Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(OpcEvent::Status { state: ConnState::Retrying { error, in_ms }, source_id }) => {
                assert_eq!(source_id, "ghost");
                assert!(matches!(error, OpcError::Connect { .. }), "a refused connection: {error:?}");
                assert!(!error.to_string().is_empty(), "a retry says why");
                waits.push(in_ms);
            }
            Ok(OpcEvent::Status { state: ConnState::Connecting, .. }) => {}
            Ok(other) => panic!("unexpected {other:?}"),
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => panic!("the worker exited instead of retrying"),
        }
    }
    assert_eq!(waits, vec![500, 1_000, 2_000], "the backoff doubles from half a second");
    assert!(!worker.handle.is_finished(), "the worker is still alive");

    // And it stops promptly when asked, mid-backoff.
    let stopped = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(stopped.elapsed() < Duration::from_secs(1), "{:?}", stopped.elapsed());
}

#[test]
fn a_silent_server_is_given_up_on_within_the_connect_timeout_and_a_stop_is_prompt() {
    // A peer that accepts TCP and never speaks OPC UA must not hold the worker.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("opc.tcp://{}/", silent.local_addr().unwrap());
    let spec = one_rule("mute", &url, 100, vec![tag("ns=1;s=t", "temp", SeriesType::Float, false, vec![])]);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    assert_eq!(next_status(&rx, Duration::from_secs(5)), ConnState::Connecting);
    let t0 = Instant::now();
    let state = next_status(&rx, CONNECT_TIMEOUT + Duration::from_secs(5));
    let took = t0.elapsed();
    assert_eq!(state, ConnState::Retrying { error: OpcError::ConnectTimedOut(10), in_ms: 500 });
    assert!(took >= CONNECT_TIMEOUT - Duration::from_millis(500) && took <= CONNECT_TIMEOUT + Duration::from_secs(2), "{took:?}");
    worker.handle.join().expect("clean shutdown");

    // A stop in the middle of a connection attempt does not wait it out.
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");
    assert_eq!(next_status(&rx, Duration::from_secs(5)), ConnState::Connecting);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
    drop(silent);
}

/// A write or read asked for while there is no session is answered at once, and a write never
/// fires later when the plant returns: a command pulse requested during an outage must not
/// land minutes afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_without_a_session_is_refused_at_once_and_never_fires_later() {
    let plant = DetachedPlant::start(0).expect("demo plant");
    let port = plant.port();
    let spec = line1(&plant.endpoint_url, |t| plant.node_id_str(t), 100);
    let cmd = plant.node_id_str(tags::CMD_START);
    plant.kill();

    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(true)).expect("worker spawns");
    assert_eq!(next_status(&rx, Duration::from_secs(5)), ConnState::Connecting);
    assert!(matches!(next_status(&rx, Duration::from_secs(15)), ConnState::Retrying { .. }));

    for _ in 0..3 {
        let t0 = Instant::now();
        let refused = worker.writer.write_tags(vec![(cmd.clone(), Variant::Int32(1))]).await;
        assert_eq!(refused, Err(WriteError::NotSent(OpcError::NotConnected)));
        assert_eq!(worker.writer.read(vec![NodeId::new(1, "x")]).await, Err(OpcError::NotConnected));
        assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(OpcError::NotConnected.to_string(), "not connected to the server");

    // The plant returns on the same endpoint; the worker finds it, and the refused writes are
    // gone, not queued.
    let plant = DetachedPlant::start(port).expect("demo plant restarts");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "the worker never reconnected");
        if let Ok(OpcEvent::Poll { values, .. }) = rx.recv_timeout(Duration::from_secs(1))
            && values.iter().all(|r| r.is_good())
        {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)), "a refused write fired after the outage");
    // A write now lands.
    assert_eq!(worker.writer.write_tags(vec![(cmd, Variant::Int32(1))]).await, Ok(vec![StatusCode::Good]));
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(1)));

    worker.handle.join().expect("clean shutdown");
    plant.kill();
}

#[test]
fn the_worker_reconnects_after_the_server_comes_back() {
    // A DETACHED plant, because killing it must really kill it: cancelling a server leaves
    // established sessions alive, and an earlier version of this test passed without a single
    // reconnect.
    let plant = DetachedPlant::start(0).expect("demo plant");
    let port = plant.port();
    plant.set(tags::TEMPERATURE, 1.0f64);
    let spec = one_rule(
        "plc",
        &plant.endpoint_url,
        100,
        vec![tag(&plant.node_id_str(tags::TEMPERATURE), "temperature", SeriesType::Float, false, vec![])],
    );
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    // The baseline poll is GOOD, or the rest proves nothing.
    let baseline = next_poll(&rx, Duration::from_secs(15));
    assert!(baseline.iter().all(|r| r.is_good()), "{baseline:?}");

    // The plant goes away: thread joined, runtime dropped, sockets closed.
    plant.kill();

    // The worker must NOTICE, before the server is back: on Windows a second bind to a
    // still-live port can succeed, so "restarted and read a new value" alone proves nothing.
    let mut noticed = false;
    let notice_by = Instant::now() + Duration::from_secs(45);
    while Instant::now() < notice_by {
        let left = notice_by.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(OpcEvent::Status { state: ConnState::Retrying { .. } | ConnState::Connecting, .. }) => {
                noticed = true;
                break;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => panic!("the worker exited"),
        }
    }
    assert!(noticed, "the worker never reported losing the connection");

    // The plant comes back on the same endpoint with a new value, and the worker finds it.
    let plant = DetachedPlant::start(port).expect("demo plant restarts");
    plant.set(tags::TEMPERATURE, 99.0f64);
    let mut recovered = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.min(Duration::from_secs(20))) {
            Ok(OpcEvent::Poll { values, .. }) => {
                if map_poll(&spec, &values, 0, 0).rows[0].values[0] == Some(Value::Float(99.0)) {
                    recovered = true;
                    break;
                }
            }
            Ok(OpcEvent::Status { .. }) => {}
            Err(_) => break,
        }
    }
    assert!(recovered, "the worker never reconnected to the restarted server");

    worker.handle.join().expect("clean shutdown");
    plant.kill();
}

/// A source of more tags than one Read may carry: the demo server refuses a Read of more than
/// its cap whole (pinned in opc_capabilities.rs), so a poll of all BULK_COUNT variables arrives
/// only if the worker chunks, and every value on its own column only if the chunks reassemble
/// in order (each variable holds its own index).
#[tokio::test(flavor = "multi_thread")]
async fn a_poll_bigger_than_one_read_reassembles_in_order() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let bulk: Vec<TagSpec> = (0..BULK_COUNT)
        .map(|i| tag(&plant.node_id_str(&format!("bulk.v{i:04}")), &format!("v{i:04}"), SeriesType::Int, false, vec![]))
        .collect();
    let spec = one_rule("bulk", &plant.endpoint_url, 200, bulk);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");

    let values = next_poll(&rx, Duration::from_secs(20));
    assert_eq!(values.len(), BULK_COUNT);
    for (i, v) in values.iter().enumerate() {
        assert!(v.status.is_good(), "v{i:04}: {:?}", v.status);
        assert_eq!(v.value, Some(Variant::Int32(i as i32)), "v{i:04} carries another column's value");
    }
    // An on-demand read of the same nodes is chunked the same way.
    let reads = worker.writer.read(spec.poll_nodes()).await.expect("read");
    assert_eq!(reads, values);

    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_identity_reaches_the_server() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let mut spec = spec_for(&plant, 100);
    spec.auth = Auth::UserPass { username: DEMO_USER.into(), password: DEMO_PASS.into() };
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    assert!(next_poll(&rx, Duration::from_secs(15)).iter().all(|r| r.is_good()));
    worker.handle.join().expect("clean shutdown");

    // A wrong password never yields a session: the worker retries, saying why, and never
    // reports itself connected.
    spec.auth = Auth::UserPass { username: DEMO_USER.into(), password: "wrong".into() };
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");
    let mut retries = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    while retries < 2 && Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(OpcEvent::Status { state: ConnState::Retrying { error, .. }, .. }) => {
                assert!(!matches!(error, OpcError::ConnectTimedOut(_)), "refused, not timed out: {error:?}");
                retries += 1;
            }
            Ok(OpcEvent::Status { state: ConnState::Connecting, .. }) => {}
            Ok(other) => panic!("a wrong password connected: {other:?}"),
            Err(e) => panic!("no retry: {e:?}"),
        }
    }
    assert_eq!(retries, 2);
    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_worker_ends_when_its_consumer_is_gone() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let (tx, rx) = channel();
    let mut worker = veredas_core::opc::spawn(spec_for(&plant, 50), tx, WriteGate::new(false)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    drop(rx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !worker.handle.is_finished() {
        assert!(Instant::now() < deadline, "the worker outlived its consumer");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(matches!(worker.handle.poll_outcome(), Some(Ok(()))));
    // Its handle now reports the worker gone.
    assert_eq!(worker.writer.read(vec![NodeId::new(1, "x")]).await, Err(OpcError::WorkerGone));
    plant.stop().await;
}

// ---- added in refute round 1 -----------------------------------------------------------------

#[test]
fn a_reads_answer_becomes_one_tag_read_per_node_in_order() {
    let nodes: Vec<NodeId> = (0..6).map(|i| NodeId::new(1, i as u32)).collect();
    let dv = |value: Option<Variant>, status: Option<StatusCode>| DataValue { value, status, ..DataValue::default() };
    // One answer short; one with no status; a value beside a bad and an uncertain status; an
    // empty variant beside a good one.
    let answer = vec![
        dv(Some(Variant::Int32(1)), None),
        dv(Some(Variant::Int32(2)), Some(StatusCode::Good)),
        dv(Some(Variant::Int32(3)), Some(StatusCode::BadOutOfService)),
        dv(Some(Variant::Int32(4)), Some(StatusCode::UncertainLastUsableValue)),
        dv(Some(Variant::Empty), Some(StatusCode::Good)),
    ];
    let reads = tag_reads(&nodes, &answer);
    let got: Vec<(NodeId, Option<Variant>, StatusCode, Quality)> =
        reads.iter().map(|r| (r.node_id.clone(), r.value.clone(), r.status, r.quality())).collect();
    assert_eq!(
        got,
        vec![
            (nodes[0].clone(), Some(Variant::Int32(1)), StatusCode::Good, Quality::Good),
            (nodes[1].clone(), Some(Variant::Int32(2)), StatusCode::Good, Quality::Good),
            (nodes[2].clone(), None, StatusCode::BadOutOfService, Quality::Bad),
            (nodes[3].clone(), None, StatusCode::UncertainLastUsableValue, Quality::Uncertain),
            (nodes[4].clone(), None, StatusCode::Good, Quality::Bad),
            (nodes[5].clone(), None, StatusCode::BadNoData, Quality::Bad),
        ]
    );
    // An answer longer than the request is cut to it; an empty request reads nothing.
    let mut long = answer.clone();
    long.push(dv(Some(Variant::Int32(9)), None));
    long.push(dv(Some(Variant::Int32(9)), None));
    assert_eq!(tag_reads(&nodes[..2], &long).len(), 2);
    assert_eq!(tag_reads(&[], &answer), Vec::new());
}

#[test]
fn the_backoff_doubles_from_half_a_second_and_holds_at_thirty() {
    let ms: Vec<u128> = retry_delays().take(9).map(|d| d.as_millis()).collect();
    assert_eq!(ms, vec![500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]);
    assert_eq!((RETRY_MIN, RETRY_MAX), (Duration::from_millis(500), Duration::from_secs(30)));
}

/// The waits a worker reports in its Retrying statuses, until `n` have come.
fn retry_waits(rx: &Receiver<OpcEvent>, n: usize, within: Duration) -> Vec<(OpcError, u64)> {
    let deadline = Instant::now() + within;
    let mut out = Vec::new();
    while out.len() < n {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(OpcEvent::Status { state: ConnState::Retrying { error, in_ms }, .. }) => out.push((error, in_ms)),
            Ok(_) => {}
            Err(e) => panic!("only {out:?} within {within:?}: {e:?}"),
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn the_backoff_starts_over_after_a_connection() {
    let plant = DetachedPlant::start(0).expect("demo plant");
    let port = plant.port();
    let spec = line1(&plant.endpoint_url, |t| plant.node_id_str(t), 100);
    plant.kill();

    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");
    let waits: Vec<u64> = retry_waits(&rx, 3, Duration::from_secs(30)).into_iter().map(|(_, ms)| ms).collect();
    assert_eq!(waits, vec![500, 1_000, 2_000]);

    let plant = DetachedPlant::start(port).expect("demo plant restarts");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "never reconnected");
        if let Ok(OpcEvent::Status { state: ConnState::Connected, .. }) = rx.recv_timeout(Duration::from_secs(1)) {
            break;
        }
    }
    plant.kill();
    // The first wait after a lost session is the shortest again.
    let (_, first) = retry_waits(&rx, 1, Duration::from_secs(30))[0].clone();
    assert_eq!(first, 500);
    worker.handle.join().expect("clean shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_that_never_answers_ends_the_session_and_a_stop_mid_poll_is_prompt() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let relay = Relay::start(plant.port()).expect("relay");
    let spec = line1(&relay.endpoint_url, |t| plant.node_id_str(t), 100);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(false)).expect("worker spawns");
    assert!(next_poll(&rx, Duration::from_secs(15)).iter().all(|r| r.is_good()), "the relay carries the session");

    // The link stalls mid-run: the read in flight never answers, and the worker drops the
    // session at the call timeout rather than waiting on it forever.
    relay.freeze();
    let t0 = Instant::now();
    let (error, in_ms) = retry_waits(&rx, 1, CALL_TIMEOUT + Duration::from_secs(5))[0].clone();
    assert!(
        matches!(error, OpcError::TimedOut(Call::Read) | OpcError::Failed { call: Call::Read, .. }),
        "a read failure, not the end of the session itself: {error:?}"
    );
    assert!(t0.elapsed() <= CALL_TIMEOUT + Duration::from_secs(2), "{:?}", t0.elapsed());
    assert_eq!(in_ms, 500);
    worker.handle.join().expect("clean shutdown");

    // A stop while a poll hangs on the stalled link does not wait for it, or for a reply to
    // the CloseSession.
    relay.thaw();
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    relay.freeze();
    std::thread::sleep(Duration::from_millis(500));
    let t0 = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(t0.elapsed() < Duration::from_millis(400), "{:?}", t0.elapsed());
    drop(relay);
    plant.stop().await;
}

/// A write whose call never answers is not "refused": it may still reach the plant. The worker
/// says so, drops the session, and a stop with a write in flight says so too.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_without_an_answer_is_unconfirmed_and_ends_the_session() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let relay = Relay::start(plant.port()).expect("relay");
    let spec = line1(&relay.endpoint_url, |t| plant.node_id_str(t), 60_000);
    let cmd = plant.node_id_str(tags::CMD_START);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec.clone(), tx, WriteGate::new(true)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));

    relay.freeze();
    let t0 = Instant::now();
    let written = worker.writer.write_tags(vec![(cmd.clone(), Variant::Int32(1))]).await;
    assert_eq!(written, Err(WriteError::Unconfirmed(OpcError::TimedOut(Call::Write))));
    assert!(t0.elapsed() >= CALL_TIMEOUT && t0.elapsed() < CALL_TIMEOUT + Duration::from_secs(2), "{:?}", t0.elapsed());
    assert_eq!(written.unwrap_err().to_string(), "the write may have reached the plant: write timed out");
    let (error, _) = retry_waits(&rx, 1, Duration::from_secs(5))[0].clone();
    assert_eq!(error, OpcError::TimedOut(Call::Write));
    worker.handle.join().expect("clean shutdown");

    // Stopped with a write in flight on the stalled link: prompt, and the write unconfirmed.
    relay.thaw();
    plant.set(tags::CMD_START, 0i32);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(true)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    relay.freeze();
    let writer = worker.writer.clone();
    let target = cmd.clone();
    let write = tokio::spawn(async move { writer.write_tags(vec![(target, Variant::Int32(1))]).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t0 = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(t0.elapsed() < Duration::from_millis(400), "{:?}", t0.elapsed());
    assert_eq!(write.await.unwrap(), Err(WriteError::Unconfirmed(OpcError::WorkerDropped)));
    drop(relay);
    plant.stop().await;
}

/// A write whose caller gave up on it (its own timeout dropped the future) before the worker
/// reached it is never sent: the operator who pressed again must not get two pulses later.
/// The control: the same queued write, still awaited, lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_its_caller_gave_up_on_is_never_sent() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec_for(&plant, 60_000), tx, WriteGate::new(true)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    let slow = parse_node_id(&plant.node_id_str(tags::SLOW)).unwrap();
    let cmd = plant.node_id_str(tags::CMD_START);

    for give_up in [true, false] {
        plant.set(tags::CMD_START, 0i32);
        let reader = worker.writer.clone();
        let slow_id = slow.clone();
        let read = tokio::spawn(async move { reader.read(vec![slow_id]).await });
        tokio::time::sleep(Duration::from_millis(100)).await; // the worker is inside the slow read
        let write = worker.writer.write_tags(vec![(cmd.clone(), Variant::Int32(1))]);
        if give_up {
            assert!(tokio::time::timeout(Duration::from_millis(50), write).await.is_err(), "it waits behind the read");
        } else {
            assert_eq!(write.await, Ok(vec![StatusCode::Good]));
        }
        read.await.unwrap().expect("the slow read");
        tokio::time::sleep(Duration::from_millis(300)).await; // the queue has drained
        let want = if give_up { 0 } else { 1 };
        assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(want)), "gave up: {give_up}");
    }

    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}

#[test]
fn a_stop_while_a_host_name_resolves_is_prompt() {
    let spec = one_rule("named", "opc.tcp://veredas-probe-no-such-host-zz/", 100, vec![]);
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(false)).expect("worker spawns");
    assert_eq!(next_status(&rx, Duration::from_secs(5)), ConnState::Connecting);
    std::thread::sleep(Duration::from_millis(50));
    let t0 = Instant::now();
    worker.handle.join().expect("clean shutdown");
    assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
}

/// The worker's thread runs on `run_on_own_runtime`, which never waits for a blocking task left
/// on its runtime (a host-name lookup is one, on the blocking pool). The lookup test above only
/// bites where a failed lookup is slow (about 1.2 s on Windows; a Linux resolver may answer at
/// once); this one bites on any machine, with a blocking task that is certainly still running.
#[test]
fn a_blocking_task_left_on_the_runtime_does_not_hold_the_thread() {
    let t0 = Instant::now();
    let out = veredas_core::opc::run_on_own_runtime(async {
        let (started, running) = tokio::sync::oneshot::channel();
        let _lookup = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            std::thread::sleep(Duration::from_secs(3));
        });
        running.await.expect("the blocking task runs");
        7
    })
    .expect("a runtime");
    assert_eq!(out, 7);
    assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
}

/// A request still queued when the worker stops was never sent, and is told so.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_still_queued_at_a_stop_is_not_sent() {
    let plant = DemoPlant::start().await.expect("demo plant");
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec_for(&plant, 60_000), tx, WriteGate::new(true)).expect("worker spawns");
    next_poll(&rx, Duration::from_secs(15));
    plant.set(tags::CMD_START, 0i32);
    let slow = parse_node_id(&plant.node_id_str(tags::SLOW)).unwrap();

    let reader = worker.writer.clone();
    let read = tokio::spawn(async move { reader.read(vec![slow]).await });
    tokio::time::sleep(Duration::from_millis(100)).await; // the worker is inside the slow read
    let writer = worker.writer.clone();
    let cmd = plant.node_id_str(tags::CMD_START);
    let write = tokio::spawn(async move { writer.write_tags(vec![(cmd, Variant::Int32(1))]).await });
    tokio::time::sleep(Duration::from_millis(100)).await; // the write waits in the queue
    worker.handle.join().expect("clean shutdown");

    assert_eq!(read.await.unwrap(), Err(OpcError::WorkerDropped), "the read was in flight");
    assert_eq!(write.await.unwrap(), Err(WriteError::NotSent(OpcError::WorkerGone)), "the write never left the queue");
    tokio::time::sleep(SLOW_READ).await;
    assert_eq!(plant.get(tags::CMD_START), Some(Variant::Int32(0)));
    plant.stop().await;
}

/// The plant axis is the local wall clock, the unix arrival its UTC: apart by the offset. On a
/// machine set to UTC the two coincide and nothing would be proven, so there (on Unix) the test
/// runs itself again in a zone five and a half hours east.
#[tokio::test(flavor = "multi_thread")]
async fn the_plant_axis_is_local_time_and_arrival_is_utc() {
    let offset_ms = i64::from(chrono::Local::now().offset().local_minus_utc()) * 1_000;
    if offset_ms == 0 {
        assert!(std::env::var_os("VEREDAS_TZ_CHILD").is_none(), "TZ was not honoured: the offset is still 0");
        #[cfg(unix)]
        {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["the_plant_axis_is_local_time_and_arrival_is_utc", "--exact", "--nocapture"])
                .env("TZ", "Asia/Kolkata")
                .env("VEREDAS_TZ_CHILD", "1")
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success() && text.contains("1 passed"), "{text}");
            return;
        }
        #[cfg(not(unix))]
        panic!("this machine is on UTC: set a zone off UTC to check the plant axis");
    }
    let plant = DemoPlant::start().await.expect("demo plant");
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec_for(&plant, 100), tx, WriteGate::new(false)).expect("worker spawns");
    let deadline = Instant::now() + Duration::from_secs(15);
    let (index_ms, arrival_ms, before, after) = loop {
        assert!(Instant::now() < deadline, "no poll");
        let before = now_unix_ms();
        if let Ok(OpcEvent::Poll { index_ms, arrival_ms, .. }) = rx.recv_timeout(Duration::from_secs(1)) {
            break (index_ms, arrival_ms, before, now_unix_ms());
        }
    };
    assert!(arrival_ms <= after && arrival_ms >= before - 1_000, "{before} <= {arrival_ms} <= {after}");
    assert!((index_ms - arrival_ms - offset_ms).abs() < 1_000, "index {index_ms}, arrival {arrival_ms}, offset {offset_ms}");
    worker.handle.join().expect("clean shutdown");
    plant.stop().await;
}
