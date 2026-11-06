//! The OPC UA server (contract: the OPC server clause), driven by the same client library portal
//! uses, raw and through core's own poll worker, so what passes here is what a consumer sees.
//! From Veredas 1's `tests/opcsim.rs`, plus reactions, credentials, limits, the relay and the
//! configuration's refusals.

use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use opcua_client::{Client, ClientBuilder, IdentityToken, Session};
use opcua_types::{
    AttributeId, EndpointDescription, MessageSecurityMode, NodeId, ReadValueId, StatusCode, TimestampsToReturn,
    UserTokenPolicy, Variant, WriteValue,
};
use veredas_core::opc::spec::{Auth, PollSpec, RuleSpec, TagSpec};
use veredas_core::opc::worker::{ConnState, OpcEvent};
use veredas_core::opc::{WriteGate, parse_node_id};
use veredas_core::SeriesType;
use veredas_maquinar::opc::*;

fn tag(id: &str, ty: TagType, access: Access, value: &str) -> TagConfig {
    TagConfig { id: id.into(), name: None, ty, access, value: value.into(), randomize: None }
}

/// Port 0: the OS picks, so tests run in parallel without fighting.
fn config(tags: Vec<TagConfig>) -> OpcConfig {
    OpcConfig { port: 0, tags, ..OpcConfig::default() }
}

fn client() -> Client {
    ClientBuilder::new()
        .application_name("maquinar test")
        .application_uri("urn:veredas:maquinar:test")
        .product_uri("urn:veredas:maquinar:test")
        .create_sample_keypair(false)
        .trust_server_certs(true)
        .session_retry_limit(1)
        .client()
        .expect("client builds")
}

fn endpoint(url: &str) -> EndpointDescription {
    (url, "None", MessageSecurityMode::None, UserTokenPolicy::anonymous()).into()
}

/// Connect, racing the event loop against the wait, so a rejected credential reports in
/// milliseconds instead of hanging.
async fn connect(url: &str, token: IdentityToken) -> Result<(Arc<Session>, tokio::task::JoinHandle<StatusCode>), String> {
    let mut client = client();
    let (session, event_loop) =
        client.connect_to_matching_endpoint(endpoint(url), token).await.map_err(|e| format!("connect: {e}"))?;
    let mut handle = event_loop.spawn();
    let settled = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::select! {
            status = &mut handle => Err(format!("event loop gave up: {status:?}")),
            ok = session.wait_for_connection() => {
                if ok { Ok(()) } else { Err("never connected".to_string()) }
            }
        }
    })
    .await
    .map_err(|_| "timed out connecting".to_string())?;
    settled.map(|()| (session, handle))
}

async fn anonymous(url: &str) -> (Arc<Session>, tokio::task::JoinHandle<StatusCode>) {
    connect(url, IdentityToken::Anonymous).await.unwrap_or_else(|e| panic!("{url}: {e}"))
}

fn read_id(node: &NodeId) -> ReadValueId {
    ReadValueId { node_id: node.clone(), attribute_id: AttributeId::Value as u32, ..Default::default() }
}

async fn read(session: &Session, node: &NodeId) -> (StatusCode, Option<Variant>) {
    let values = session.read(&[read_id(node)], TimestampsToReturn::Neither, 0.0).await.expect("read");
    let dv = values.into_iter().next().expect("one value");
    (dv.status.unwrap_or(StatusCode::Good), dv.value)
}

async fn write(session: &Session, node: &NodeId, value: Variant) -> StatusCode {
    let results = session
        .write(&[WriteValue { node_id: node.clone(), attribute_id: AttributeId::Value as u32, value: value.into(), ..Default::default() }])
        .await
        .expect("write call");
    results[0]
}

fn until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

// ---------------------------------------------------------------------------------------------
// Serving
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_client_reads_the_tags_the_bench_configured() {
    let mut named = tag("line1.temp", TagType::Float, Access::Read, "21.5");
    named.name = Some("Temperature".into());
    let server = start(&config(vec![
        named,
        tag("line1.count", TagType::Int, Access::Read, "-7"),
        tag("line1.on", TagType::Bool, Access::Read, "yes"),
        tag("line1.batch", TagType::Text, Access::Read, "B-001"),
        tag("line1.blank", TagType::Text, Access::Read, ""),
        tag("line1.zero", TagType::Int, Access::Read, ""),
    ]))
    .expect("the server starts");
    assert_eq!(server.endpoint_url, format!("opc.tcp://127.0.0.1:{}/", server.port));
    assert_ne!(server.port, 0, "the port the OS gave is advertised");
    assert_eq!(server.node_id_str("line1.temp"), format!("ns={};s=line1.temp", server.namespace));
    assert!(server.namespace >= 2, "the bench's namespace is not the application uri's: {}", server.namespace);
    assert_ne!(NAMESPACE, "urn:veredas:maquinar");

    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;
    let expect = [
        ("line1.temp", Variant::Double(21.5)),
        ("line1.count", Variant::Int32(-7)),
        ("line1.on", Variant::Boolean(true)),
        ("line1.batch", Variant::from("B-001")),
        ("line1.blank", Variant::from("")),
        ("line1.zero", Variant::Int32(0)),
    ];
    for (id, value) in &expect {
        let (status, got) = read(&session, &NodeId::new(ns, *id)).await;
        assert_eq!(status, StatusCode::Good, "reading {}", server.node_id_str(id));
        assert_eq!(got.as_ref(), Some(value), "{id}");
        assert_eq!(server.get(id), Some(value.clone()), "server-side {id}");
    }
    // The browse and display names: the name given, else the id.
    let names = session
        .read(
            &[
                ReadValueId { node_id: NodeId::new(ns, "line1.temp"), attribute_id: AttributeId::DisplayName as u32, ..Default::default() },
                ReadValueId { node_id: NodeId::new(ns, "line1.count"), attribute_id: AttributeId::BrowseName as u32, ..Default::default() },
            ],
            TimestampsToReturn::Neither,
            0.0,
        )
        .await
        .unwrap();
    assert!(format!("{:?}", names[0].value).contains("Temperature"), "{:?}", names[0]);
    assert!(format!("{:?}", names[1].value).contains("line1.count"), "{:?}", names[1]);
    // An unknown node is refused, and the snapshot lists every configured tag in order.
    let (status, _) = read(&session, &NodeId::new(ns, "nope")).await;
    assert_eq!(status, StatusCode::BadNodeIdUnknown);
    assert_eq!(server.get("nope"), None);
    let snapshot = server.snapshot();
    assert_eq!(snapshot.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), expect.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    assert_eq!(snapshot[0].value, Some(Variant::Double(21.5)));
    assert_eq!(render_value(&snapshot[1].value), "-7");
    assert_eq!(render_value(&None), "-");
    assert_eq!(render_value(&Some(Variant::Empty)), "(empty)");
    assert_eq!(render_value(&Some(Variant::from("B-001"))), "B-001");

    // A value set from the plant side is what the client reads next.
    server.set("line1.temp", 30.25f64);
    until("the set to land", Duration::from_secs(2), || server.get("line1.temp") == Some(Variant::Double(30.25)));
    let (status, got) = read(&session, &NodeId::new(ns, "line1.temp")).await;
    assert_eq!((status, got), (StatusCode::Good, Some(Variant::Double(30.25))));
    assert_eq!(server.status(), OpcStatus::default(), "nothing drawn, nothing fired");
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn randomized_tags_move_and_stay_inside_their_range() {
    let mut float = tag("t", TagType::Float, Access::Read, "0");
    float.randomize = Some(Randomize { low: 18.0, high: 24.0, decimals: 1, interval_ms: 30 });
    let mut int = tag("i", TagType::Int, Access::Read, "0");
    int.randomize = Some(Randomize { low: 3.0, high: 5.0, decimals: 0, interval_ms: 30 });
    let mut flag = tag("b", TagType::Bool, Access::Read, "0");
    flag.randomize = Some(Randomize { low: 0.0, high: 0.0, decimals: 0, interval_ms: 30 });
    let still = tag("s", TagType::Float, Access::Read, "1.5");
    let server = start(&config(vec![float, int, flag, still])).expect("starts");

    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;
    let mut floats: Vec<f64> = Vec::new();
    let mut ints: Vec<i32> = Vec::new();
    let mut bools: Vec<bool> = Vec::new();
    for _ in 0..30 {
        match read(&session, &NodeId::new(ns, "t")).await {
            (StatusCode::Good, Some(Variant::Double(v))) => {
                assert!((18.0..=24.0).contains(&v), "{v} escaped the range");
                assert_eq!((v * 10.0).round() / 10.0, v, "one decimal: {v}");
                floats.push(v);
            }
            other => panic!("expected a double, got {other:?}"),
        }
        match read(&session, &NodeId::new(ns, "i")).await {
            (StatusCode::Good, Some(Variant::Int32(v))) => {
                assert!((3..=5).contains(&v), "{v} escaped the range");
                ints.push(v);
            }
            other => panic!("expected an int, got {other:?}"),
        }
        match read(&session, &NodeId::new(ns, "b")).await {
            (StatusCode::Good, Some(Variant::Boolean(v))) => bools.push(v),
            other => panic!("expected a bool, got {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    // A simulator whose "randomized" value never changes is not simulating anything.
    assert!(floats.iter().any(|v| (*v - floats[0]).abs() > f64::EPSILON), "the float never moved: {floats:?}");
    assert!(ints.iter().any(|v| *v != ints[0]), "the int never moved: {ints:?}");
    assert!(bools.iter().any(|v| *v != bools[0]), "the bool never flipped: {bools:?}");
    // A tag without a draw stays where it was, and the draws are counted.
    assert_eq!(server.get("s"), Some(Variant::Double(1.5)));
    let draws = server.status().draws;
    assert!(draws >= 60, "{draws}");
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_land_in_the_address_space_and_the_access_is_honoured() {
    let server = start(&config(vec![
        tag("cmd", TagType::Int, Access::ReadWrite, "0"),
        tag("ro", TagType::Int, Access::Read, "0"),
        tag("wo", TagType::Int, Access::Write, "0"),
    ]))
    .expect("starts");
    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;

    assert_eq!(write(&session, &NodeId::new(ns, "cmd"), Variant::Int32(7)).await, StatusCode::Good, "a writable tag accepts");
    // A Good status only proves the call was accepted; the server side is the ground truth.
    assert_eq!(server.get("cmd"), Some(Variant::Int32(7)));
    let (status, got) = read(&session, &NodeId::new(ns, "cmd")).await;
    assert_eq!((status, got), (StatusCode::Good, Some(Variant::Int32(7))));

    let refused = write(&session, &NodeId::new(ns, "ro"), Variant::Int32(1)).await;
    assert!(refused.is_bad(), "a read-only tag must refuse, got {refused:?}");
    assert_eq!(server.get("ro"), Some(Variant::Int32(0)), "and holds its value");

    // Write-only: a read is refused, a write lands, the server side still shows it.
    let (status, _) = read(&session, &NodeId::new(ns, "wo")).await;
    assert!(status.is_bad(), "a write-only tag should refuse a read, got {status:?}");
    assert_eq!(write(&session, &NodeId::new(ns, "wo"), Variant::Int32(3)).await, StatusCode::Good);
    assert_eq!(server.get("wo"), Some(Variant::Int32(3)));
    server.set("wo", Variant::Int32(4));
    until("the set", Duration::from_secs(2), || server.get("wo") == Some(Variant::Int32(4)));
    assert_eq!(Access::Write.label(), "write");
    assert_eq!(Access::ReadWrite.label(), "read/write");
    assert_eq!(Access::Read.label(), "read");
    server.stop();
}

// ---------------------------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_reaction_answers_a_write_with_a_timed_sequence() {
    // A command handshake as a plant plays it: a client writes 100 into a trigger, sees 99
    // (running) at once and 1 (executed) a little later.
    let mut cfg = config(vec![
        tag("trigger", TagType::Int, Access::ReadWrite, "0"),
        tag("echo", TagType::Text, Access::ReadWrite, ""),
        tag("mirror", TagType::Text, Access::Read, ""),
    ]);
    cfg.reactions = vec![
        Reaction {
            on_write: "trigger".into(),
            equals: Some("100".into()),
            tag: None,
            then: vec![Step { after_ms: 0, set: "99".into() }, Step { after_ms: 300, set: "1".into() }],
        },
        // Any change of `echo` is mirrored into another tag.
        Reaction { on_write: "echo".into(), equals: None, tag: Some("mirror".into()), then: vec![Step { after_ms: 0, set: "changed".into() }] },
    ];
    let server = start(&cfg).expect("starts");
    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;
    let trigger = NodeId::new(ns, "trigger");

    // A write of another value does nothing.
    assert_eq!(write(&session, &trigger, Variant::Int32(5)).await, StatusCode::Good);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.get("trigger"), Some(Variant::Int32(5)));
    assert_eq!(server.status().reactions_fired, 0);

    let written = Instant::now();
    assert_eq!(write(&session, &trigger, Variant::Int32(100)).await, StatusCode::Good);
    until("99", Duration::from_secs(2), || server.get("trigger") == Some(Variant::Int32(99)));
    let running_after = written.elapsed();
    assert!(running_after < Duration::from_millis(200), "99 within a few ticks, not {running_after:?}");
    until("1", Duration::from_secs(2), || server.get("trigger") == Some(Variant::Int32(1)));
    let done_after = written.elapsed();
    assert!(done_after >= Duration::from_millis(300) && done_after < Duration::from_millis(700), "1 after about 300 ms, not {done_after:?}");
    assert_eq!(server.status().reactions_fired, 1);

    // The client sees the sequence too, and the reaction fires again on the next 100.
    let (status, got) = read(&session, &trigger).await;
    assert_eq!((status, got), (StatusCode::Good, Some(Variant::Int32(1))));
    assert_eq!(write(&session, &trigger, Variant::Int32(100)).await, StatusCode::Good);
    until("1 again", Duration::from_secs(2), || server.get("trigger") == Some(Variant::Int32(1)));
    assert_eq!(server.status().reactions_fired, 2);

    // Any change: writing text into `echo` sets `mirror`; writing the same text again does not.
    assert_eq!(server.get("mirror"), Some(Variant::from("")));
    assert_eq!(write(&session, &NodeId::new(ns, "echo"), Variant::from("hello")).await, StatusCode::Good);
    until("the mirror", Duration::from_secs(2), || server.get("mirror") == Some(Variant::from("changed")));
    assert_eq!(server.status().reactions_fired, 3);
    server.set("mirror", "");
    until("the reset", Duration::from_secs(2), || server.get("mirror") == Some(Variant::from("")));
    assert_eq!(write(&session, &NodeId::new(ns, "echo"), Variant::from("hello")).await, StatusCode::Good);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.get("mirror"), Some(Variant::from("")), "the same value is no change");
    assert_eq!(server.status().reactions_fired, 3);
    // A plant-side set is a change too.
    server.set("echo", "bye");
    until("the mirror again", Duration::from_secs(2), || server.get("mirror") == Some(Variant::from("changed")));
    assert_eq!(server.status().reactions_fired, 4);
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draw_or_another_reaction_s_step_is_not_a_change() {
    // Only what a client wrote or the plant side set is a change: a watched tag that is drawn
    // every tick fires nothing, and a step of one reaction into another's watched tag fires
    // nothing either.
    let mut drawn = tag("drawn", TagType::Int, Access::Read, "0");
    drawn.randomize = Some(Randomize { low: 0.0, high: 1000.0, decimals: 0, interval_ms: 20 });
    let mut cfg = config(vec![
        drawn,
        tag("d_out", TagType::Int, Access::Read, "0"),
        tag("a", TagType::Int, Access::ReadWrite, "0"),
        tag("b", TagType::Int, Access::Read, "0"),
        tag("c", TagType::Int, Access::Read, "0"),
    ]);
    cfg.reactions = vec![
        Reaction { on_write: "drawn".into(), equals: None, tag: Some("d_out".into()), then: vec![Step { after_ms: 0, set: "1".into() }] },
        Reaction { on_write: "a".into(), equals: Some("1".into()), tag: Some("b".into()), then: vec![Step { after_ms: 0, set: "9".into() }] },
        Reaction { on_write: "b".into(), equals: None, tag: Some("c".into()), then: vec![Step { after_ms: 0, set: "1".into() }] },
    ];
    let server = start(&cfg).expect("starts");
    until("draws", Duration::from_secs(2), || server.status().draws >= 10);
    assert_eq!(server.get("d_out"), Some(Variant::Int32(0)), "a draw is not a change");
    server.set("a", Variant::Int32(1));
    until("b", Duration::from_secs(2), || server.get("b") == Some(Variant::Int32(9)));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.get("c"), Some(Variant::Int32(0)), "a step of another reaction is not a change");
    assert_eq!(server.status().reactions_fired, 1);
    // A client's write into `b` is one.
    let (session, _loop) = anonymous(&server.endpoint_url).await;
    assert!(write(&session, &NodeId::new(server.namespace, "b"), Variant::Int32(2)).await.is_bad(), "b is read-only to a client");
    server.set("b", Variant::Int32(2));
    until("c", Duration::from_secs(2), || server.get("c") == Some(Variant::Int32(1)));
    assert_eq!(server.status().reactions_fired, 2);
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_of_the_wrong_wire_type_is_put_back_and_counted() {
    // The stack answers a scalar of another type with Good and stores it (a real controller
    // would refuse it); the bench undoes it on its next tick, counts it where a consumer's
    // test can see it, and never reads it as a value: a reaction on the tag does not fire.
    let mut cfg = config(vec![
        tag("f", TagType::Float, Access::ReadWrite, "1.5"),
        tag("i", TagType::Int, Access::ReadWrite, "7"),
        tag("b", TagType::Bool, Access::ReadWrite, "true"),
        tag("t", TagType::Text, Access::ReadWrite, "x"),
        tag("out", TagType::Int, Access::Read, "0"),
    ]);
    cfg.reactions = vec![Reaction { on_write: "f".into(), equals: Some("99".into()), tag: Some("out".into()), then: vec![Step { after_ms: 0, set: "1".into() }] }];
    let server = start(&cfg).expect("starts");
    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;
    let wrong = [
        ("f", Variant::Int32(99), Some(Variant::Double(1.5))),
        ("i", Variant::from("hello"), Some(Variant::Int32(7))),
        ("b", Variant::Double(3.5), Some(Variant::Boolean(true))),
        ("t", Variant::Int32(1), Some(Variant::from("x"))),
        ("i", Variant::Empty, Some(Variant::Int32(7))),
    ];
    for (n, (id, value, kept)) in wrong.iter().enumerate() {
        let status = write(&session, &NodeId::new(ns, *id), value.clone()).await;
        assert!(status.is_good(), "the stack takes {value:?} into {id}: {status:?}");
        until("the value put back", Duration::from_secs(2), || server.get(id) == *kept && server.status().type_mismatches as usize > n);
        let (st, got) = read(&session, &NodeId::new(ns, *id)).await;
        assert_eq!((st, got), (StatusCode::Good, kept.clone()), "{id} reads its own type again");
        assert_eq!(server.status().last_mismatch.as_ref().map(|m| m.0.as_str()), Some(*id));
    }
    assert_eq!(server.status().type_mismatches, 5);
    assert_eq!((server.get("out"), server.status().reactions_fired), (Some(Variant::Int32(0)), 0), "an Int32 99 is no Double 99");
    // The right type fires it, and a plant-side set of the wrong type is counted and dropped.
    assert_eq!(write(&session, &NodeId::new(ns, "f"), Variant::Double(99.0)).await, StatusCode::Good);
    until("fired", Duration::from_secs(2), || server.get("out") == Some(Variant::Int32(1)));
    server.set("i", Variant::Double(2.0));
    until("the set counted", Duration::from_secs(2), || server.status().type_mismatches == 6);
    assert_eq!(server.get("i"), Some(Variant::Int32(7)));
    assert_eq!(server.status().last_mismatch, Some(("i".into(), "Double(2.0)".into())));
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn int_draws_take_only_the_whole_numbers_inside_the_range() {
    let mut int = tag("i", TagType::Int, Access::Read, "0");
    int.randomize = Some(Randomize { low: 3.5, high: 5.5, decimals: 0, interval_ms: 20 });
    let mut edge = tag("e", TagType::Int, Access::Read, "0");
    edge.randomize = Some(Randomize { low: -2147483648.0, high: -2147483647.0, decimals: 0, interval_ms: 20 });
    let server = start(&config(vec![int, edge])).expect("starts");
    let mut seen = std::collections::BTreeSet::new();
    let mut edges = std::collections::BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(Variant::Int32(v)) = server.get("i") {
            seen.insert(v);
        }
        if let Some(Variant::Int32(v)) = server.get("e") {
            edges.insert(v);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    seen.remove(&0);
    assert!(!seen.is_empty() && seen.iter().all(|v| (4..=5).contains(v)), "3.5..=5.5 holds 4 and 5 only: {seen:?}");
    assert!(seen.len() == 2, "both drawn in two seconds: {seen:?}");
    edges.remove(&0);
    assert!(edges.iter().all(|v| *v == i32::MIN || *v == i32::MIN + 1), "{edges:?}");
    server.stop();
}

// ---------------------------------------------------------------------------------------------
// Through core's own client layer
// ---------------------------------------------------------------------------------------------

#[test]
fn the_consumer_s_poll_worker_polls_and_writes_through_the_bench() {
    let mut float = tag("line1.temperature", TagType::Float, Access::Read, "21.5");
    float.randomize = Some(Randomize { low: 18.0, high: 24.0, decimals: 1, interval_ms: 50 });
    let server = start(&config(vec![
        float,
        tag("line1.status", TagType::Int, Access::Read, "19"),
        tag("line1.cmd.start", TagType::Int, Access::ReadWrite, "0"),
    ]))
    .expect("starts");
    let node = |id: &str| server.node_id_str(id);
    let t = |id: &str, series: &str, ty: SeriesType, writable: bool| TagSpec {
        node_id: parse_node_id(&node(id)).unwrap(),
        raw: node(id),
        series: series.into(),
        ty,
        writable,
        bits: vec![],
    };
    let spec = PollSpec {
        source_id: "plc".into(),
        display_name: "plc".into(),
        endpoint_url: server.endpoint_url.clone(),
        auth: Auth::Anonymous,
        poll: Duration::from_millis(100),
        rules: vec![RuleSpec {
            name: "line1".into(),
            tags: vec![
                t("line1.temperature", "temperature", SeriesType::Float, false),
                t("line1.status", "status", SeriesType::Int, false),
                t("line1.cmd.start", "cmd_start", SeriesType::Int, true),
            ],
            store_on_change: false,
        }],
    };
    let (tx, rx) = channel();
    let worker = veredas_core::opc::spawn(spec, tx, WriteGate::new(true)).expect("the worker spawns");

    // Connected, then polls with every tag good, in node order.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut connected = false;
    let mut polls = Vec::new();
    while Instant::now() < deadline && polls.len() < 5 {
        match rx.recv_timeout(Duration::from_secs(15)).expect("an event") {
            OpcEvent::Status { state: ConnState::Connected, .. } => connected = true,
            OpcEvent::Status { .. } => {}
            OpcEvent::Poll { values, .. } => polls.push(values),
        }
    }
    assert!(connected);
    assert_eq!(polls.len(), 5);
    for values in &polls {
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|v| v.is_good()), "{values:?}");
        assert!(matches!(values[0].value, Some(Variant::Double(v)) if (18.0..=24.0).contains(&v)), "{:?}", values[0]);
        assert_eq!(values[1].value, Some(Variant::Int32(19)));
    }
    let temps: Vec<f64> = polls.iter().filter_map(|p| p[0].value.as_ref()).map(|v| if let Variant::Double(v) = v { *v } else { f64::NAN }).collect();
    assert!(temps.iter().any(|v| (*v - temps[0]).abs() > f64::EPSILON), "the draws reach the consumer: {temps:?}");

    // A write through the consumer's gated path lands server-side.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let statuses = rt.block_on(worker.writer.write_tags(vec![(node("line1.cmd.start"), Variant::Int32(1))])).expect("the write goes");
    assert_eq!(statuses, vec![StatusCode::Good]);
    until("the pulse to land", Duration::from_secs(2), || server.get("line1.cmd.start") == Some(Variant::Int32(1)));

    // The bench vanishing is noticed: the worker reports a retry, not silence.
    drop(worker.writer);
    server.stop();
    let retrying = Instant::now() + Duration::from_secs(20);
    let mut seen_retry = false;
    while Instant::now() < retrying {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(OpcEvent::Status { state: ConnState::Retrying { .. }, .. }) => {
                seen_retry = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(seen_retry, "the worker noticed the bench going away");
    worker.handle.join().expect("the worker ends cleanly");
}

// ---------------------------------------------------------------------------------------------
// Credentials and limits
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_configured_user_is_accepted_and_a_wrong_password_refused() {
    let mut cfg = config(vec![tag("x", TagType::Int, Access::Read, "1")]);
    cfg.user = Some(Credentials { name: "operator".into(), password: "hunter2".into() });
    assert!(!format!("{cfg:?}").contains("hunter2"), "the password shows in no debug form: {cfg:?}");
    let server = start(&cfg).expect("starts");
    let ns = server.namespace;

    let (session, _loop) = connect(&server.endpoint_url, IdentityToken::UserName("operator".into(), "hunter2".into()))
        .await
        .expect("the configured user connects");
    assert_eq!(read(&session, &NodeId::new(ns, "x")).await, (StatusCode::Good, Some(Variant::Int32(1))));
    let (anon, _loop2) = anonymous(&server.endpoint_url).await;
    assert_eq!(read(&anon, &NodeId::new(ns, "x")).await, (StatusCode::Good, Some(Variant::Int32(1))), "anonymous still works");

    let started = Instant::now();
    let wrong = connect(&server.endpoint_url, IdentityToken::UserName("operator".into(), "wrong".into())).await;
    assert!(wrong.is_err(), "a wrong password must not yield a session");
    assert!(started.elapsed() < Duration::from_secs(10), "refused promptly, not after {:?}", started.elapsed());
    let stranger = connect(&server.endpoint_url, IdentityToken::UserName("nobody".into(), "hunter2".into())).await;
    assert!(stranger.is_err(), "an unknown user must not yield a session");
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_read_cap_and_the_session_cap_are_the_bench_s_knobs() {
    let mut cfg = config((0..5).map(|i| tag(&format!("v{i}"), TagType::Int, Access::Read, &i.to_string())).collect());
    cfg.max_nodes_per_read = Some(3);
    cfg.max_sessions = Some(1);
    let server = start(&cfg).expect("starts");
    let ns = server.namespace;
    let (session, _loop) = anonymous(&server.endpoint_url).await;

    let ids: Vec<ReadValueId> = (0..4).map(|i| read_id(&NodeId::new(ns, format!("v{i}")))).collect();
    let err = session.read(&ids, TimestampsToReturn::Neither, 0.0).await.expect_err("four nodes over a cap of three");
    assert!(err.to_string().contains("BadTooManyOperations"), "{err}");
    let three = session.read(&ids[..3], TimestampsToReturn::Neither, 0.0).await.expect("three nodes at the cap");
    assert_eq!(three.iter().map(|dv| dv.value.clone()).collect::<Vec<_>>(), vec![Some(Variant::Int32(0)), Some(Variant::Int32(1)), Some(Variant::Int32(2))]);

    // One session held: a second is refused.
    let second = connect(&server.endpoint_url, IdentityToken::Anonymous).await;
    assert!(second.is_err(), "a second session over a cap of one must be refused");
    server.stop();
}

// ---------------------------------------------------------------------------------------------
// The relay, the port and the configuration
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_makes_the_link_go_silent_and_delivers_what_it_held_when_thawed() {
    let server = start(&config(vec![tag("x", TagType::Int, Access::Read, "5")])).expect("starts");
    let relay = server.relay().expect("a relay for a server on 127.0.0.1");
    assert_eq!(relay.endpoint_url, format!("opc.tcp://127.0.0.2:{}/", server.port));
    let ns = server.namespace;
    let (session, _loop) = anonymous(&relay.endpoint_url).await;
    assert_eq!(read(&session, &NodeId::new(ns, "x")).await, (StatusCode::Good, Some(Variant::Int32(5))));

    relay.freeze();
    let silent = tokio::time::timeout(Duration::from_secs(1), session.read(&[read_id(&NodeId::new(ns, "x"))], TimestampsToReturn::Neither, 0.0)).await;
    assert!(silent.is_err(), "a frozen link answers nothing: {silent:?}");
    relay.thaw();
    // The read held in the relay is delivered: the next read answers, and the server still
    // serves directly.
    let after = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(v) = session.read(&[read_id(&NodeId::new(ns, "x"))], TimestampsToReturn::Neither, 0.0).await
                && v[0].value == Some(Variant::Int32(5))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(after.is_ok(), "the link recovers once thawed");
    drop(relay);

    let mut elsewhere = config(vec![]);
    elsewhere.bind = "localhost".into();
    let other = start(&elsewhere).expect("starts on localhost");
    match other.relay() {
        Err(OpcError::NoRelay(bind)) => assert_eq!(bind, "localhost"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("a relay for a server not on 127.0.0.1"),
    }
    server.stop();
}

#[test]
fn a_port_already_in_use_fails_with_its_address_rather_than_hanging() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port to squat on");
    let port = listener.local_addr().unwrap().port();
    let cfg = OpcConfig { port, tags: vec![tag("x", TagType::Float, Access::Read, "0")], ..OpcConfig::default() };
    let started = Instant::now();
    match start(&cfg) {
        Err(OpcError::Bind { addr, source }) => {
            assert_eq!(addr, format!("127.0.0.1:{port}"));
            assert!(!source.to_string().is_empty());
            assert!(OpcError::Bind { addr, source }.to_string().starts_with(&format!("bind 127.0.0.1:{port}: ")));
        }
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn stopping_frees_the_port_for_the_next_start() {
    // Start, stop, start on the same port: what pressing the button twice does, and the thing a
    // soft cancel (which leaves sessions alive) would break.
    let first = start(&config(vec![tag("x", TagType::Float, Access::Read, "0")])).expect("first start");
    let port = first.port;
    first.stop();
    let cfg = OpcConfig { port, tags: vec![tag("x", TagType::Float, Access::Read, "0")], ..OpcConfig::default() };
    let second = start(&cfg).unwrap_or_else(|e| panic!("restart on {port}: {e}"));
    assert_eq!(second.port, port);
    drop(second);
    let third = start(&cfg).unwrap_or_else(|e| panic!("restart after a drop on {port}: {e}"));
    assert_eq!(third.port, port);
}

#[test]
fn values_parse_per_type_and_nonsense_is_refused() {
    assert_eq!(parse_value(TagType::Int, " 42 "), Ok(Variant::Int32(42)));
    assert_eq!(parse_value(TagType::Int, "-2147483648"), Ok(Variant::Int32(i32::MIN)));
    assert_eq!(parse_value(TagType::Int, ""), Ok(Variant::Int32(0)));
    assert_eq!(parse_value(TagType::Int, "4.5"), Err("`4.5` is not an int".into()));
    assert_eq!(parse_value(TagType::Int, "2147483648"), Err("`2147483648` is not an int".into()));
    assert_eq!(parse_value(TagType::Float, "21.5"), Ok(Variant::Double(21.5)));
    assert_eq!(parse_value(TagType::Float, "1e3"), Ok(Variant::Double(1000.0)));
    assert_eq!(parse_value(TagType::Float, ""), Ok(Variant::Double(0.0)));
    assert_eq!(parse_value(TagType::Float, "x"), Err("`x` is not a float".into()));
    assert_eq!(parse_value(TagType::Float, "NaN"), Err("`NaN` is not a finite float".into()));
    assert_eq!(parse_value(TagType::Float, "inf"), Err("`inf` is not a finite float".into()));
    for yes in ["true", "1", "on", "YES", " True "] {
        assert_eq!(parse_value(TagType::Bool, yes), Ok(Variant::Boolean(true)), "{yes}");
    }
    for no in ["false", "0", "off", "No", ""] {
        assert_eq!(parse_value(TagType::Bool, no), Ok(Variant::Boolean(false)), "{no}");
    }
    assert_eq!(parse_value(TagType::Bool, "2"), Err("`2` is not a bool".into()));
    assert_eq!(parse_value(TagType::Text, " B-1 "), Ok(Variant::from(" B-1 ")), "text is kept as typed");
    assert_eq!(TagType::Int.label(), "int");
    assert_eq!(TagType::Text.label(), "text");
}

#[test]
fn a_bad_configuration_is_refused_by_name_before_anything_starts() {
    let ok = |tags: Vec<TagConfig>, reactions: Vec<Reaction>| OpcConfig { port: 0, tags, reactions, ..OpcConfig::default() };
    let refused = |cfg: &OpcConfig| match start(cfg) {
        Err(OpcError::Config(m)) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        refused(&ok(vec![tag("a", TagType::Int, Access::Read, "1"), tag("a", TagType::Float, Access::Read, "1")], vec![])),
        "opc.tags[1] (`a`): two tags share this id"
    );
    assert_eq!(refused(&ok(vec![tag("  ", TagType::Int, Access::Read, "1")], vec![])), "opc.tags[0]: the tag has no id");
    assert_eq!(refused(&ok(vec![tag("a", TagType::Int, Access::Read, "one")], vec![])), "opc.tags[0] (`a`).value: `one` is not an int");
    assert_eq!(refused(&ok(vec![tag("a", TagType::Bool, Access::Read, "maybe")], vec![])), "opc.tags[0] (`a`).value: `maybe` is not a bool");
    let mut text = tag("t", TagType::Text, Access::Read, "x");
    text.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![text], vec![])), "opc.tags[0] (`t`).randomize: text has no range to draw from");
    let mut zero = tag("z", TagType::Int, Access::Read, "0");
    zero.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 0 });
    assert_eq!(refused(&ok(vec![zero], vec![])), "opc.tags[0] (`z`).randomize.interval_ms is 0");
    let mut wide = tag("w", TagType::Float, Access::Read, "0");
    wide.randomize = Some(Randomize { low: f64::NEG_INFINITY, high: 1.0, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![wide], vec![])), "opc.tags[0] (`w`).randomize: the range is not finite");
    let mut empty = tag("f", TagType::Float, Access::Read, "0");
    empty.randomize = Some(Randomize { low: 24.0, high: 18.0, decimals: 1, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![empty], vec![])), "opc.tags[0] (`f`).randomize: the range is empty (24 > 18)");
    let mut empty_int = tag("i", TagType::Int, Access::Read, "0");
    empty_int.randomize = Some(Randomize { low: 5.0, high: 3.0, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![empty_int], vec![])), "opc.tags[0] (`i`).randomize: the range is empty (5 > 3)");
    let mut no_whole = tag("i", TagType::Int, Access::Read, "0");
    no_whole.randomize = Some(Randomize { low: 3.2, high: 3.8, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![no_whole], vec![])), "opc.tags[0] (`i`).randomize: no whole number lies in 3.2..=3.8");
    let mut too_big = tag("i", TagType::Int, Access::Read, "0");
    too_big.randomize = Some(Randomize { low: 3e9, high: 4e9, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![too_big], vec![])), "opc.tags[0] (`i`).randomize: 3000000000..=4000000000 is outside an int");
    let mut too_low = tag("i", TagType::Int, Access::Read, "0");
    too_low.randomize = Some(Randomize { low: -3e9, high: 0.0, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![too_low], vec![])), "opc.tags[0] (`i`).randomize: -3000000000..=0 is outside an int");
    let mut one_whole = tag("i", TagType::Int, Access::Read, "0");
    one_whole.randomize = Some(Randomize { low: 2.5, high: 3.5, decimals: 0, interval_ms: 10 });
    assert!(validate(&ok(vec![one_whole], vec![])).is_ok(), "3 lies in 2.5..=3.5");
    let mut flip = tag("b", TagType::Bool, Access::Read, "0");
    flip.randomize = Some(Randomize { low: 0.0, high: 0.0, decimals: 0, interval_ms: 10 });
    assert!(validate(&ok(vec![flip], vec![])).is_ok(), "a bool's range is unread, but must not be empty");
    let mut flip_empty = tag("b", TagType::Bool, Access::Read, "0");
    flip_empty.randomize = Some(Randomize { low: 1.0, high: 0.0, decimals: 0, interval_ms: 10 });
    assert_eq!(refused(&ok(vec![flip_empty], vec![])), "opc.tags[0] (`b`).randomize: the range is empty (1 > 0)");
    let mut blank = ok(vec![], vec![]);
    blank.bind = " ".into();
    assert_eq!(refused(&blank), "opc.bind is empty");
    let mut nobody = ok(vec![], vec![]);
    nobody.user = Some(Credentials { name: "".into(), password: "x".into() });
    assert_eq!(refused(&nobody), "opc.user.name is empty");
    let mut capless = ok(vec![], vec![]);
    capless.max_nodes_per_read = Some(0);
    assert_eq!(refused(&capless), "opc.max_nodes_per_read is 0: no Read could carry a node");
    let mut sessionless = ok(vec![], vec![]);
    sessionless.max_sessions = Some(0);
    assert_eq!(refused(&sessionless), "opc.max_sessions is 0: no client could connect");
    let reaction = |on: &str, equals: Option<&str>, target: Option<&str>, then: Vec<(u64, &str)>| Reaction {
        on_write: on.into(),
        equals: equals.map(String::from),
        tag: target.map(String::from),
        then: then.into_iter().map(|(after_ms, set)| Step { after_ms, set: set.into() }).collect(),
    };
    let one = || vec![tag("a", TagType::Int, Access::ReadWrite, "0"), tag("s", TagType::Text, Access::Read, "")];
    assert_eq!(refused(&ok(one(), vec![reaction("nope", None, None, vec![(0, "1")])])), "opc.reactions[0].on_write: no tag `nope`");
    assert_eq!(refused(&ok(one(), vec![reaction("a", None, Some("nope"), vec![(0, "1")])])), "opc.reactions[0].tag: no tag `nope`");
    assert_eq!(refused(&ok(one(), vec![reaction("a", Some("x"), None, vec![(0, "1")])])), "opc.reactions[0].equals: `x` is not an int");
    assert_eq!(refused(&ok(one(), vec![reaction("a", None, None, vec![])])), "opc.reactions[0].then is empty: a reaction sets something");
    assert_eq!(refused(&ok(one(), vec![reaction("a", None, None, vec![(0, "1"), (5, "x")])])), "opc.reactions[0].then[1].set: `x` is not an int");
    // The target's type decides a step's value, the watched tag's the trigger.
    assert!(validate(&ok(one(), vec![reaction("a", Some("1"), Some("s"), vec![(0, "any text")])])).is_ok());
    assert_eq!(validate(&ok(one(), vec![reaction("a", Some("1"), Some("s"), vec![(0, "x")]), reaction("s", None, Some("a"), vec![(0, "2")])])).ok(), Some(2));
    // Nothing started on a refusal: the port is still free afterwards.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut bad = ok(vec![tag("a", TagType::Int, Access::Read, "one")], vec![]);
    bad.port = port;
    refused(&bad);
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port was never taken");
}

#[test]
fn the_configuration_reads_from_toml_with_its_defaults_and_refuses_a_misspelled_key() {
    let text = r#"
[[tags]]
id = "line1.temperature"
name = "Temperature"
type = "float"
value = "21.5"
randomize = { low = 18, high = 24, decimals = 1 }

[[tags]]
id = "line1.cmd.start"
type = "int"
access = "read-write"

[[reactions]]
on_write = "line1.cmd.start"
equals = "100"
then = [ { set = "99" }, { after_ms = 300, set = "1" } ]
"#;
    let cfg: OpcConfig = toml::from_str(text).unwrap();
    assert_eq!(cfg.bind, "127.0.0.1");
    assert_eq!(cfg.port, 4855);
    assert_eq!(cfg.user, None);
    assert_eq!((cfg.max_nodes_per_read, cfg.max_sessions), (None, None));
    assert_eq!(cfg.tags.len(), 2);
    assert_eq!(cfg.tags[0].name.as_deref(), Some("Temperature"));
    assert_eq!(cfg.tags[0].access, Access::Read);
    assert_eq!(cfg.tags[0].randomize, Some(Randomize { low: 18.0, high: 24.0, decimals: 1, interval_ms: 1000 }));
    assert_eq!(cfg.tags[1], TagConfig { id: "line1.cmd.start".into(), name: None, ty: TagType::Int, access: Access::ReadWrite, value: "".into(), randomize: None });
    assert_eq!(cfg.reactions[0].then, vec![Step { after_ms: 0, set: "99".into() }, Step { after_ms: 300, set: "1".into() }]);
    assert_eq!(cfg.reactions[0].tag, None);
    let full: OpcConfig = toml::from_str(&format!("bind = \"10.0.0.5\"\nport = 4840\nuser = {{ name = \"op\", password = \"pw\" }}\nmax_nodes_per_read = 800\nmax_sessions = 2\n{text}")).unwrap();
    assert_eq!((full.bind.as_str(), full.port, full.max_nodes_per_read, full.max_sessions), ("10.0.0.5", 4840, Some(800), Some(2)));
    assert_eq!(full.user.as_ref().map(|u| u.name.as_str()), Some("op"));
    assert_eq!(toml::from_str::<OpcConfig>("").unwrap(), OpcConfig::default());

    for (bad, key) in [
        (text.replace("value = \"21.5\"", "valeu = \"21.5\""), "valeu"),
        (text.replace("type = \"float\"", "type = \"double\""), "double"),
        (text.replace("access = \"read-write\"", "access = \"readwrite\""), "readwrite"),
        (text.replace("decimals = 1", "decimal = 1"), "decimal"),
        (text.replace("after_ms = 300", "after = 300"), "after"),
        (text.replace("on_write", "on"), "on"),
        (format!("{text}bindd = \"x\"\n"), "bindd"),
        (format!("{text}user = {{ name = \"op\", pass = \"pw\" }}\n"), "pass"),
        (text.replace("id = \"line1.cmd.start\"\n", ""), "id"),
    ] {
        let err = toml::from_str::<OpcConfig>(&bad).unwrap_err().to_string();
        assert!(err.contains(key), "{key}: {err}");
    }
}
