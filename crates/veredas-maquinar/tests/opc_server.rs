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
    TagConfig { id: id.into(), name: None, ty, access, value: value.into(), randomize: None, ramp: None }
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
    // The initial value holds until the first draw: read once every randomized tag has drawn.
    until("the first draws", Duration::from_secs(5), || server.status().draws >= 3);
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
async fn draws_come_at_their_interval() {
    // A draw is due an interval after the last was due, never after the tick that served it:
    // a 100 ms tag draws 20 times in 2 s on either platform (the tick adds nothing to the
    // interval; a due time taken from the tick's clock gave 16), and a 20 ms tag draws on every
    // tick. The tick is 20 ms as asked of the runtime's timer, which lands it at the host's
    // timer quantum, held at a millisecond while the server runs (`timer::FineTimer`;
    // `docs/questions.md` Q13, decided): 20 ms on both platforms, where it was about 31 on
    // Windows before. The 20 ms count is a bound against the quantum, not a pace: the tick's
    // `MissedTickBehavior::Delay` turns a late wake-up into a longer period, and a loaded host
    // wakes late (a shared CI runner drew 81), so the floor sits where a 31 ms tick (64 draws)
    // cannot reach it and the ceiling where two draws a tick would.
    let mut slow = tag("slow", TagType::Int, Access::Read, "0");
    slow.randomize = Some(Randomize { low: 0.0, high: 9.0, decimals: 0, interval_ms: 100 });
    let server = start(&config(vec![slow])).expect("starts");
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let draws = server.status().draws;
    server.stop();
    assert!((19..=21).contains(&draws), "a 100 ms tag drew {draws} times in 2 s");

    let mut fast = tag("fast", TagType::Int, Access::Read, "0");
    fast.randomize = Some(Randomize { low: 0.0, high: 9.0, decimals: 0, interval_ms: 20 });
    let server = start(&config(vec![fast])).expect("starts");
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let draws = server.status().draws;
    server.stop();
    assert!((70..=102).contains(&draws), "a 20 ms tag drew {draws} times in 2 s: under 70 is a 31 ms tick (64), the host's quantum not held at a millisecond, not a loaded host's late wake-ups (81 on a shared CI runner); over 102 is two draws a tick");
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
async fn a_write_of_the_wrong_wire_type_is_refused_and_counted() {
    // The stack's own validation would take a scalar of another type into any tag; the bench
    // answers it as a real controller does, `BadTypeMismatch`, stores nothing, counts it where a
    // consumer's test can see its mistake, and no reaction sees it. A write of the right type
    // lands, and a right one followed at once by a wrong one keeps the right one.
    let mut cfg = config(vec![
        tag("f", TagType::Float, Access::ReadWrite, "1.5"),
        tag("i", TagType::Int, Access::ReadWrite, "7"),
        tag("b", TagType::Bool, Access::ReadWrite, "true"),
        tag("t", TagType::Text, Access::ReadWrite, "x"),
        tag("w", TagType::Int, Access::Write, "0"),
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
        ("w", Variant::Float(1.0), Some(Variant::Int32(0))),
        ("f", Variant::Float(1.0), Some(Variant::Double(1.5))),
    ];
    for (n, (id, value, kept)) in wrong.iter().enumerate() {
        let status = write(&session, &NodeId::new(ns, *id), value.clone()).await;
        assert_eq!(status, StatusCode::BadTypeMismatch, "{value:?} into {id}");
        // Plainly, at once: nothing was stored.
        assert_eq!(server.get(id), *kept, "{id} after {value:?}");
        assert_eq!(server.snapshot().iter().find(|t| t.id == *id).unwrap().value, *kept);
        assert_eq!(server.status().type_mismatches as usize, n + 1);
        assert_eq!(server.status().last_mismatch.as_ref().map(|m| m.0.as_str()), Some(*id));
    }
    assert_eq!(server.status().last_mismatch, Some(("f".into(), "Float(1.0)".into())));
    assert_eq!((server.get("out"), server.status().reactions_fired), (Some(Variant::Int32(0)), 0), "an Int32 99 is no Double 99");
    // A right write lands, and a right one followed at once by a wrong one keeps the right one.
    assert_eq!(write(&session, &NodeId::new(ns, "f"), Variant::Double(5.0)).await, StatusCode::Good);
    assert_eq!(write(&session, &NodeId::new(ns, "f"), Variant::Int32(1)).await, StatusCode::BadTypeMismatch);
    assert_eq!(server.get("f"), Some(Variant::Double(5.0)));
    let (st, got) = read(&session, &NodeId::new(ns, "f")).await;
    assert_eq!((st, got), (StatusCode::Good, Some(Variant::Double(5.0))));
    // The right type fires the reaction, and a plant-side set of the wrong type is counted and
    // dropped.
    assert_eq!(write(&session, &NodeId::new(ns, "f"), Variant::Double(99.0)).await, StatusCode::Good);
    until("fired", Duration::from_secs(2), || server.get("out") == Some(Variant::Int32(1)));
    server.set("i", Variant::Double(2.0));
    until("the set counted", Duration::from_secs(2), || server.status().type_mismatches == 9);
    assert_eq!(server.get("i"), Some(Variant::Int32(7)));
    assert_eq!(server.status().last_mismatch, Some(("i".into(), "Double(2.0)".into())));
    // A ranged write is refused; a text written as text lands.
    let ranged = session
        .write(&[WriteValue { node_id: NodeId::new(ns, "t"), attribute_id: AttributeId::Value as u32, index_range: opcua_types::NumericRange::Index(0), value: Variant::from("y").into() }])
        .await
        .unwrap();
    assert_eq!(ranged[0], StatusCode::BadIndexRangeInvalid);
    assert_eq!(write(&session, &NodeId::new(ns, "t"), Variant::from("y")).await, StatusCode::Good);
    assert_eq!(server.get("t"), Some(Variant::from("y")));
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
fn a_ramp_steps_by_its_step_and_wraps_by_the_range_s_width() {
    let r = Ramp { low: 0.0, high: 100.0, step: 30.0, decimals: None, interval_ms: 100 };
    let mut v = 0.0;
    let mut seen = Vec::new();
    for _ in 0..12 {
        v = ramp_next(v, &r);
        seen.push(v);
    }
    assert_eq!(seen, [30.0, 60.0, 90.0, 20.0, 50.0, 80.0, 10.0, 40.0, 70.0, 100.0, 30.0, 60.0], "the high is reached, then wrapped past");
    assert_eq!(ramp_next(100.0, &Ramp { step: 100.0, ..r.clone() }), 100.0, "a step of the whole width comes back to where it was");
    assert_eq!(ramp_next(150.0, &r), 0.0, "outside the range: the low");
    assert_eq!(ramp_next(-0.5, &r), 0.0);
    assert_eq!(ramp_next(f64::NAN, &r), 0.0);
    let tenths = Ramp { low: 0.0, high: 1.0, step: 0.1, decimals: Some(1), interval_ms: 100 };
    let mut v = 0.0;
    for _ in 0..10 {
        v = ramp_next(v, &tenths);
    }
    assert_eq!(v, 1.0, "rounded each step, ten tenths make one");
    assert_eq!(ramp_next(v, &tenths), 0.1);
    // With places the ramp counts in units of its last place: 0..=0.3 by 0.1 reaches its high and
    // never steps back (float sums wrapped it early: 0.2 + 0.1 is past 0.3 by its last bit).
    let short = Ramp { low: 0.0, high: 0.3, step: 0.1, decimals: Some(1), interval_ms: 100 };
    let mut v = 0.0;
    let seen: Vec<f64> = (0..5).map(|_| { v = ramp_next(v, &short); v }).collect();
    assert_eq!(seen, [0.1, 0.2, 0.3, 0.1, 0.2]);
    assert_eq!(ramp_next(0.25, &short), 0.1, "a value between places is counted from its nearest place (0.3)");
    // Every high and step at one place from 0.1 to 10, from 0 and from a low of -1, against the
    // same ramp counted in whole tenths here.
    for lo in [0i64, -10] {
        for hi in 1..=100i64 {
            for st in 1..=hi - lo {
                let r = Ramp { low: lo as f64 / 10.0, high: hi as f64 / 10.0, step: st as f64 / 10.0, decimals: Some(1), interval_ms: 100 };
                let (mut v, mut n) = (r.low, lo);
                for k in 0..(2 * (hi - lo) / st + 3) {
                    v = ramp_next(v, &r);
                    n += st;
                    while n > hi {
                        n -= hi - lo;
                    }
                    assert_eq!(v, n as f64 / 10.0, "{}..={} by {}, step {k}", r.low, r.high, r.step);
                }
            }
        }
    }
    // At the edge of what fifteen places count exactly (2^51 units, a range within ±2.25), every
    // unit step up to the high is the next whole count of units.
    let edge = Ramp { low: 2.24999999999999, high: 2.25, step: 1e-15, decimals: Some(15), interval_ms: 100 };
    let mut v = edge.low;
    for k in 1..=10i64 {
        v = ramp_next(v, &edge);
        assert_eq!(v, (2_249_999_999_999_990i64 + k) as f64 / 1e15, "step {k}");
    }
    assert_eq!(v, 2.25, "the high, exactly");
    assert_eq!(ramp_next(v, &edge), (2_249_999_999_999_991i64) as f64 / 1e15, "wrapped by the width");
    let ints = Ramp { low: -5.0, high: 5.0, step: 4.0, decimals: None, interval_ms: 100 };
    assert_eq!([ramp_next(-5.0, &ints), ramp_next(3.0, &ints), ramp_next(5.0, &ints)], [-1.0, -3.0, -1.0]);
    // With places too, a value outside the range starts again at the low.
    let one = Ramp { low: 0.0, high: 1.0, step: 0.3, decimals: Some(1), interval_ms: 100 };
    assert_eq!([ramp_next(5.0, &one), ramp_next(-0.5, &one), ramp_next(f64::NAN, &one)], [0.0, 0.0, 0.0]);
    // Without places, an int's ramp: its whole numbers summed exactly (a float's names its places).
    let whole = Ramp { low: -7.0, high: 6.0, step: 5.0, decimals: None, interval_ms: 100 };
    let mut v = whole.low;
    for k in 0..60 {
        let next = ramp_next(v, &whole);
        assert!((whole.low..=whole.high).contains(&next) && next.fract() == 0.0 && (next == v + 5.0 || next == v + 5.0 - 13.0), "step {k}: {v} then {next}");
        v = next;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stall_skips_the_draws_and_steps_it_missed() {
    // The service loop held: a 500 ms ramp and a 100 ms draw miss what fell in it and skip it,
    // keeping their phase; catching it up would step and draw once a tick after.
    let mut ramped = tag("ramped", TagType::Int, Access::Read, "0");
    ramped.ramp = Some(Ramp { low: 0.0, high: 1000.0, step: 1.0, decimals: None, interval_ms: 500 });
    let mut drawn = tag("drawn", TagType::Int, Access::Read, "0");
    drawn.randomize = Some(Randomize { low: 0.0, high: 9.0, decimals: 0, interval_ms: 100 });
    let server = start(&config(vec![ramped, drawn])).expect("starts");
    let read = |s: &OpcHandle| match s.get("ramped") {
        Some(Variant::Int32(x)) => x,
        other => panic!("{other:?}"),
    };
    // Begun just after a step of a 500 ms ramp (seen within a tick or two of it), a stall of
    // 1200 ms ends some 250 ms before the ramp's next point on its old phase; a schedule restarted
    // from the tick that served the step it owed would put that point a whole interval after.
    let before = server.status();
    let v0 = read(&server);
    let deadline = Instant::now() + Duration::from_secs(3);
    while read(&server) == v0 {
        assert!(Instant::now() < deadline, "the ramp never stepped");
    }
    let stepped = read(&server);
    let t = Instant::now();
    server.stall(Duration::from_millis(1200));
    let resumed_at = Instant::now();
    assert!(resumed_at - t >= Duration::from_millis(1200), "the loop was held: {:?}", resumed_at - t);
    // After it: the one step it owed, then the next on the old phase.
    let mut changes: Vec<Duration> = Vec::new();
    let mut last = stepped;
    while changes.len() < 2 && resumed_at.elapsed() < Duration::from_millis(1500) {
        let v = read(&server);
        if v != last {
            changes.push(resumed_at.elapsed());
            last = v;
        }
    }
    let after = server.status();
    server.stop();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes[0] < Duration::from_millis(100), "the step it owed, at once: {changes:?}");
    assert!(changes[1] > Duration::from_millis(150) && changes[1] < Duration::from_millis(400), "the next on the old phase, some 250 ms after (500 had the schedule restarted): {changes:?}");
    assert!(after.ramp_steps - before.ramp_steps <= 3 && after.draws - before.draws <= 12, "nothing caught up (the draws: some 5 before, none in, 4 after; 12 more caught up): {before:?} / {after:?}");
}

#[test]
fn a_draw_or_a_step_is_next_due_an_interval_after_it_was_due() {
    let t0 = tokio::time::Instant::now();
    let ms = Duration::from_millis;
    assert_eq!(next_due(t0, ms(100), t0 + ms(20)), t0 + ms(100), "an interval after it was due, not after the tick that served it");
    assert_eq!(next_due(t0, ms(100), t0 + ms(399)), t0 + ms(400));
    assert_eq!(next_due(t0, ms(100), t0 + ms(350)), t0 + ms(400), "a stall skips what it missed and keeps the phase");
    assert_eq!(next_due(t0, ms(100), t0 + ms(400)), t0 + ms(500), "never due at or before the tick that served it");
}

/// The tag's distinct values read on the server side every millisecond, until `n` came.
fn distinct(server: &OpcHandle, id: &str, n: usize, within: Duration) -> Vec<f64> {
    let mut seen: Vec<f64> = Vec::new();
    let started = Instant::now();
    while seen.len() < n && started.elapsed() < within {
        let v = match server.get(id) {
            Some(Variant::Int32(x)) => x as f64,
            Some(Variant::Double(x)) => x,
            _ => continue,
        };
        if seen.last() != Some(&v) {
            seen.push(v);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    seen
}

/// Once the tag has just stepped, so what follows lands early in its interval and is read before
/// the next step.
fn just_stepped(server: &OpcHandle, id: &str) {
    let seen = distinct(server, id, 2, Duration::from_secs(5));
    assert_eq!(seen.len(), 2, "{id} never stepped: {seen:?}");
}

/// The value the tag holds after `x` appears in it: the ramp's next from `x`.
fn next_after(server: &OpcHandle, id: &str, x: f64) -> f64 {
    let seen = distinct(server, id, 6, Duration::from_secs(5));
    let at = seen.iter().position(|v| *v == x).unwrap_or_else(|| panic!("{id} never held {x}: {seen:?}"));
    *seen.get(at + 1).unwrap_or_else(|| panic!("{id} never stepped from {x}: {seen:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ramped_tag_steps_on_the_wire_and_moves_on_from_what_it_holds() {
    let pos_r = Ramp { low: 0.0, high: 100.0, step: 30.0, decimals: Some(0), interval_ms: 100 };
    let level_r = Ramp { low: 0.0, high: 10.0, step: 3.0, decimals: None, interval_ms: 100 };
    let mut pos = tag("pos", TagType::Float, Access::ReadWrite, "0");
    pos.ramp = Some(pos_r.clone());
    let mut level = tag("level", TagType::Int, Access::Read, "8");
    level.ramp = Some(level_r.clone());
    let tenths_r = Ramp { low: 0.0, high: 1.0, step: 0.3, decimals: Some(1), interval_ms: 100 };
    let mut tenths = tag("tenths", TagType::Float, Access::Read, "0");
    tenths.ramp = Some(tenths_r.clone());
    let mut cfg = config(vec![
        pos,
        level,
        tenths,
        tag("still", TagType::Float, Access::Read, "1.5"),
        tag("go", TagType::Int, Access::ReadWrite, "0"),
        tag("fired", TagType::Int, Access::Read, "0"),
    ]);
    // A client's press sets the ramped level (a reaction's step); a reaction watching the ramped
    // level would fire on any change of it.
    cfg.reactions = vec![
        Reaction { on_write: "go".into(), equals: Some("1".into()), tag: Some("level".into()), then: vec![Step { after_ms: 0, set: "0".into() }] },
        Reaction { on_write: "level".into(), equals: None, tag: Some("fired".into()), then: vec![Step { after_ms: 0, set: "1".into() }] },
    ];
    let server = start(&cfg).expect("starts");
    let (session, _loop) = anonymous(&server.endpoint_url).await;
    let ns = server.namespace;
    let cycle = |r: &Ramp, from: f64, n: usize| { let mut v = from; (0..n).map(|_| { v = ramp_next(v, r); v }).collect::<Vec<f64>>() };

    // Over the wire, until twelve distinct values came: each the ramp's next of the one before it
    // (a value read twice is the same step; a read never skips more than the steps a stalled
    // reader missed, each one a step of the ramp).
    let mut values: Vec<f64> = Vec::new();
    let started = Instant::now();
    while values.len() < 12 && started.elapsed() < Duration::from_secs(20) {
        if let (StatusCode::Good, Some(Variant::Double(v))) = read(&session, &NodeId::new(ns, "pos")).await
            && values.last() != Some(&v)
        {
            values.push(v);
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(values.len() >= 12, "{values:?}");
    for w in values.windows(2) {
        assert!((1..=4).any(|k| cycle(&pos_r, w[0], k)[k - 1] == w[1]), "{} after {}: {values:?}", w[1], w[0]);
    }
    assert!(values.iter().all(|v| (0.0..=100.0).contains(v) && v % 10.0 == 0.0), "{values:?}");
    // On the server side, every step seen: each distinct value of the int ramp exactly the ramp's
    // next of the one before.
    let levels = distinct(&server, "level", 8, Duration::from_secs(5));
    assert_eq!(levels.len(), 8, "{levels:?}");
    for w in levels.windows(2) {
        assert_eq!(w[1], ramp_next(w[0], &level_r), "{levels:?}");
    }
    // And a float ramp at its places, wrapping within the values read: 0.3, 0.6, 0.9, 0.2, ...
    let seen = distinct(&server, "tenths", 12, Duration::from_secs(5));
    assert_eq!(seen.len(), 12, "{seen:?}");
    for w in seen.windows(2) {
        assert_eq!(w[1], ramp_next(w[0], &tenths_r), "{seen:?}");
    }
    assert!(seen.windows(2).any(|w| w[1] < w[0]), "it wrapped: {seen:?}");
    assert_eq!(read(&session, &NodeId::new(ns, "still")).await, (StatusCode::Good, Some(Variant::Double(1.5))));

    // What the tag holds moves the ramp on: a client's write, a plant-side set, a reaction's step.
    just_stepped(&server, "pos");
    assert_eq!(write(&session, &NodeId::new(ns, "pos"), Variant::Double(55.0)).await, StatusCode::Good);
    assert_eq!(next_after(&server, "pos", 55.0), 85.0, "from a client's write");
    just_stepped(&server, "pos");
    server.set("pos", 25.0);
    assert_eq!(next_after(&server, "pos", 25.0), 55.0, "from a plant-side set");
    just_stepped(&server, "level");
    assert_eq!(write(&session, &NodeId::new(ns, "go"), Variant::Int32(1)).await, StatusCode::Good);
    // The press sets 0, a value the level's own cycle never takes (8, 1, 4, 7, 10, 3, ...).
    assert_eq!(next_after(&server, "level", 0.0), 3.0, "from a reaction's step");
    // Outside the range: the low, exactly.
    just_stepped(&server, "pos");
    server.set("pos", 250.0);
    assert_eq!(next_after(&server, "pos", 250.0), 0.0, "outside the range, the ramp starts at its low");
    // A ramp's step is no change: the reaction watching the level never fired.
    assert_eq!((server.get("fired"), server.status().reactions_fired), (Some(Variant::Int32(0)), 1), "only the press fired");
    // While a plant-side set of the ramped level is one: the watching reaction is alive.
    server.set("level", 5);
    until("the set fires the watcher", Duration::from_secs(2), || server.get("fired") == Some(Variant::Int32(1)));
    assert_eq!(server.status().reactions_fired, 2);
    assert!(server.status().ramp_steps >= 16, "{:?}", server.status());
    assert_eq!(server.status().draws, 0, "a step is no draw");
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn ramps_come_at_their_interval_counted_apart_from_draws() {
    // As a draw is (`draws_come_at_their_interval`): due an interval after the last was due, the
    // tick adding nothing; a 100 ms ramp steps 20 times in 2 s, a 20 ms ramp on every tick (the
    // same bounds, against the timer's quantum and not a pace). Beside a 100 ms draw, each counts
    // its own.
    let mut ramped = tag("ramped", TagType::Int, Access::Read, "0");
    ramped.ramp = Some(Ramp { low: 0.0, high: 1000.0, step: 1.0, decimals: None, interval_ms: 100 });
    let mut floated = tag("floated", TagType::Float, Access::Read, "0");
    floated.ramp = Some(Ramp { low: 0.0, high: 1.0, step: 0.3, decimals: Some(1), interval_ms: 100 });
    let mut drawn = tag("drawn", TagType::Int, Access::Read, "0");
    drawn.randomize = Some(Randomize { low: 0.0, high: 9.0, decimals: 0, interval_ms: 100 });
    let server = start(&config(vec![ramped, floated, drawn])).expect("starts");
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let st = server.status();
    server.stop();
    assert!((38..=42).contains(&st.ramp_steps) && (19..=21).contains(&st.draws), "an int and a float 100 ms ramp and a 100 ms draw in 2 s: {st:?}");

    let mut fast = tag("fast", TagType::Int, Access::Read, "0");
    fast.ramp = Some(Ramp { low: 0.0, high: 1000.0, step: 1.0, decimals: None, interval_ms: 20 });
    let server = start(&config(vec![fast])).expect("starts");
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let st = server.status();
    server.stop();
    assert!((70..=102).contains(&st.ramp_steps) && st.draws == 0, "a 20 ms ramp in 2 s: {st:?}");
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
    text.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![text], vec![])), "opc.tags[0] (`t`).randomize: text has no range to draw from");
    let mut zero = tag("z", TagType::Int, Access::Read, "0");
    zero.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 0 });
    assert_eq!(refused(&ok(vec![zero], vec![])), "opc.tags[0] (`z`).randomize.interval_ms is 0");
    let mut short = tag("s", TagType::Int, Access::Read, "0");
    short.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 19 });
    assert_eq!(refused(&ok(vec![short], vec![])), "opc.tags[0] (`s`).randomize.interval_ms 19 is shorter than the server's tick of 20 ms");
    let ramp = |low: f64, high: f64, step: f64, decimals: Option<u8>, interval_ms: u64| Some(Ramp { low, high, step, decimals, interval_ms });
    let ramped = |ty: TagType, value: &str, r: Option<Ramp>| { let mut t = tag("r", ty, Access::Read, value); t.ramp = r; t };
    let mut both = ramped(TagType::Float, "0", ramp(0.0, 1.0, 0.5, None, 20));
    both.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![both], vec![])), "opc.tags[0] (`r`): a tag takes a draw or a ramp, not both");
    for (t, r, expected) in [
        (TagType::Bool, ramp(0.0, 1.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: a bool has no range to step through"),
        (TagType::Text, ramp(0.0, 1.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: a text has no range to step through"),
        (TagType::Float, ramp(0.0, f64::INFINITY, 1.0, None, 20), "opc.tags[0] (`r`).ramp: the range and the step are not all finite"),
        (TagType::Float, ramp(f64::NEG_INFINITY, 10.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: the range and the step are not all finite"),
        (TagType::Float, ramp(-1.7e308, 1.7e308, 1e308, None, 20), "opc.tags[0] (`r`).ramp: the range's width (-1.7e308..=1.7e308) is past what a float holds"),
        (TagType::Float, ramp(0.0, 1.0, 0.04, Some(1), 20), "opc.tags[0] (`r`).ramp.step 0.04 is not a whole number of the ramp's places (1)"),
        (TagType::Float, ramp(0.04, 1.0, 0.1, Some(1), 20), "opc.tags[0] (`r`).ramp.low 0.04 is not a whole number of the ramp's places (1)"),
        (TagType::Float, ramp(0.0, 1.25, 0.1, Some(1), 20), "opc.tags[0] (`r`).ramp.high 1.25 is not a whole number of the ramp's places (1)"),
        (TagType::Float, ramp(0.0, 2.5, 0.5, Some(0), 20), "opc.tags[0] (`r`).ramp.high 2.5 is not a whole number of the ramp's places (0)"),
        (TagType::Float, ramp(0.0, 1e6, 0.1, Some(15), 20), "opc.tags[0] (`r`).ramp.high 1000000 at 15 places is past what a float counts exactly"),
        (TagType::Float, ramp(0.0, 2.26, 0.01, Some(15), 20), "opc.tags[0] (`r`).ramp.high 2.26 at 15 places is past what a float counts exactly"),
        (TagType::Float, ramp(4.0, 5.0, 1e-15, Some(15), 20), "opc.tags[0] (`r`).ramp.low 4 at 15 places is past what a float counts exactly"),
        (TagType::Float, ramp(99999999.9, 100000000.06, 0.1, Some(1), 20), "opc.tags[0] (`r`).ramp.high 100000000.06 is not a whole number of the ramp's places (1)"),
        (TagType::Float, ramp(0.0, 1.0, 0.07, Some(1), 20), "opc.tags[0] (`r`).ramp.step 0.07 is not a whole number of the ramp's places (1)"),
        (TagType::Float, ramp(0.1, 0.3, 0.3, Some(1), 20), "opc.tags[0] (`r`).ramp: the step 0.3 is not above 0 and at most the range's width 0.2"),
        (TagType::Float, ramp(0.0, 10.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: a float ramp counts its steps in its places: name them (`decimals`)"),
        (TagType::Float, ramp(1e16, 1e16 + 2.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: a float ramp counts its steps in its places: name them (`decimals`)"),
        (TagType::Int, ramp(-3e9, 10.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: an int steps by whole numbers inside an int (-3000000000..=10 by 1)"),
        (TagType::Float, ramp(0.0, 10.0, f64::NAN, None, 20), "opc.tags[0] (`r`).ramp: the range and the step are not all finite"),
        (TagType::Float, ramp(10.0, 10.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: the range is empty (10 is not below 10)"),
        (TagType::Float, ramp(0.0, 10.0, 0.0, None, 20), "opc.tags[0] (`r`).ramp: the step 0 is not above 0 and at most the range's width 10"),
        (TagType::Float, ramp(0.0, 10.0, -1.0, None, 20), "opc.tags[0] (`r`).ramp: the step -1 is not above 0 and at most the range's width 10"),
        (TagType::Float, ramp(0.0, 10.0, 10.5, Some(1), 20), "opc.tags[0] (`r`).ramp: the step 10.5 is not above 0 and at most the range's width 10"),
        (TagType::Int, ramp(0.0, 10.0, 1.5, None, 20), "opc.tags[0] (`r`).ramp: an int steps by whole numbers inside an int (0..=10 by 1.5)"),
        (TagType::Int, ramp(0.5, 10.0, 1.0, None, 20), "opc.tags[0] (`r`).ramp: an int steps by whole numbers inside an int (0.5..=10 by 1)"),
        (TagType::Int, ramp(0.0, 3e9, 1.0, None, 20), "opc.tags[0] (`r`).ramp: an int steps by whole numbers inside an int (0..=3000000000 by 1)"),
        (TagType::Int, ramp(0.0, 10.0, 1.0, Some(1), 20), "opc.tags[0] (`r`).ramp.decimals: an int has no places"),
        (TagType::Float, ramp(0.0, 10.0, 1.0, Some(16), 20), "opc.tags[0] (`r`).ramp.decimals 16 is past 15"),
        (TagType::Float, ramp(0.0, 10.0, 1.0, Some(1), 0), "opc.tags[0] (`r`).ramp.interval_ms is 0"),
        (TagType::Float, ramp(0.0, 10.0, 1.0, Some(1), 19), "opc.tags[0] (`r`).ramp.interval_ms 19 is shorter than the server's tick of 20 ms"),
    ] {
        assert_eq!(refused(&ok(vec![ramped(t, "0", r)], vec![])), expected);
    }
    assert_eq!(refused(&ok(vec![ramped(TagType::Float, "11", ramp(0.0, 10.0, 1.0, Some(1), 20))], vec![])), "opc.tags[0] (`r`).ramp: the initial value 11 lies outside 0..=10");
    assert_eq!(refused(&ok(vec![ramped(TagType::Int, "-1", ramp(0.0, 10.0, 1.0, None, 20))], vec![])), "opc.tags[0] (`r`).ramp: the initial value -1 lies outside 0..=10");
    assert!(validate(&ok(vec![ramped(TagType::Float, "", ramp(0.0, 1.0, 1.0, Some(15), 20))], vec![])).is_ok(), "an empty value is the zero, a step of the whole width, fifteen places (a range of 1 counts exactly at them)");
    assert!(validate(&ok(vec![ramped(TagType::Int, "10", ramp(-5.0, 10.0, 15.0, None, 20))], vec![])).is_ok());
    assert!(validate(&ok(vec![ramped(TagType::Float, "0", ramp(0.0, 2.25, 0.01, Some(15), 20))], vec![])).is_ok(), "2.25 at fifteen places counts exactly");
    assert!(validate(&ok(vec![ramped(TagType::Float, "0", ramp(-2.2, 0.3, 0.1, Some(1), 20))], vec![])).is_ok(), "-2.2, 0.3 and 0.1 at one place");
    // At one and two places, every range from -0.6 to 0.6 (-6 to 6 at one): a step of its whole
    // width accepted, one unit more refused, counted in units (0.07 * 100 is 7.000000000000001).
    for (d, f, n) in [(1u8, 10.0, 60i64), (2, 100.0, 60)] {
        for lo in -n..=n {
            for hi in (lo + 1)..=n {
                let (low, high) = (lo as f64 / f, hi as f64 / f);
                let whole = ramp(low, high, (hi - lo) as f64 / f, Some(d), 20);
                assert!(validate(&ok(vec![ramped(TagType::Float, &low.to_string(), whole)], vec![])).is_ok(), "{low}..={high} by its width at {d}");
                let past = ramp(low, high, (hi - lo + 1) as f64 / f, Some(d), 20);
                assert!(validate(&ok(vec![ramped(TagType::Float, &low.to_string(), past)], vec![])).is_err(), "{low}..={high} by a unit past its width at {d}");
            }
        }
    }
    // A step of the whole width at places, counted in units where the floats fall a hair short
    // (0.3 - 0.1 is 0.19999999999999998): accepted, and it comes back to where it was.
    for (low, high, step) in [(0.1, 0.3, 0.2), (0.4, 0.7, 0.3), (100000000.0, 100000000.1, 0.1)] {
        let r = Ramp { low, high, step, decimals: Some(1), interval_ms: 20 };
        assert!(validate(&ok(vec![ramped(TagType::Float, &low.to_string(), Some(r.clone()))], vec![])).is_ok(), "{low}..={high} by {step}");
        assert_eq!((ramp_next(high, &r), ramp_next(ramp_next(high, &r), &r)), (high, high), "{low}..={high} by {step}");
    }
    let mut at_tick = tag("k", TagType::Int, Access::Read, "0");
    at_tick.randomize = Some(Randomize { low: 0.0, high: 1.0, decimals: 0, interval_ms: 20 });
    assert!(validate(&ok(vec![at_tick], vec![])).is_ok(), "an interval of one tick is served on every tick");
    let mut wide = tag("w", TagType::Float, Access::Read, "0");
    wide.randomize = Some(Randomize { low: f64::NEG_INFINITY, high: 1.0, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![wide], vec![])), "opc.tags[0] (`w`).randomize: the range is not finite");
    let mut empty = tag("f", TagType::Float, Access::Read, "0");
    empty.randomize = Some(Randomize { low: 24.0, high: 18.0, decimals: 1, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![empty], vec![])), "opc.tags[0] (`f`).randomize: the range is empty (24 > 18)");
    let mut empty_int = tag("i", TagType::Int, Access::Read, "0");
    empty_int.randomize = Some(Randomize { low: 5.0, high: 3.0, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![empty_int], vec![])), "opc.tags[0] (`i`).randomize: the range is empty (5 > 3)");
    let mut no_whole = tag("i", TagType::Int, Access::Read, "0");
    no_whole.randomize = Some(Randomize { low: 3.2, high: 3.8, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![no_whole], vec![])), "opc.tags[0] (`i`).randomize: no whole number lies in 3.2..=3.8");
    let mut too_big = tag("i", TagType::Int, Access::Read, "0");
    too_big.randomize = Some(Randomize { low: 3e9, high: 4e9, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![too_big], vec![])), "opc.tags[0] (`i`).randomize: 3000000000..=4000000000 is outside an int");
    let mut too_low = tag("i", TagType::Int, Access::Read, "0");
    too_low.randomize = Some(Randomize { low: -3e9, high: 0.0, decimals: 0, interval_ms: 20 });
    assert_eq!(refused(&ok(vec![too_low], vec![])), "opc.tags[0] (`i`).randomize: -3000000000..=0 is outside an int");
    let mut one_whole = tag("i", TagType::Int, Access::Read, "0");
    one_whole.randomize = Some(Randomize { low: 2.5, high: 3.5, decimals: 0, interval_ms: 20 });
    assert!(validate(&ok(vec![one_whole], vec![])).is_ok(), "3 lies in 2.5..=3.5");
    let mut flip = tag("b", TagType::Bool, Access::Read, "0");
    flip.randomize = Some(Randomize { low: 0.0, high: 0.0, decimals: 0, interval_ms: 20 });
    assert!(validate(&ok(vec![flip], vec![])).is_ok(), "a bool's range is unread, but must not be empty");
    let mut flip_empty = tag("b", TagType::Bool, Access::Read, "0");
    flip_empty.randomize = Some(Randomize { low: 1.0, high: 0.0, decimals: 0, interval_ms: 20 });
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
    assert_eq!(cfg.tags[1], TagConfig { id: "line1.cmd.start".into(), name: None, ty: TagType::Int, access: Access::ReadWrite, value: "".into(), randomize: None, ramp: None });
    assert_eq!(cfg.reactions[0].then, vec![Step { after_ms: 0, set: "99".into() }, Step { after_ms: 300, set: "1".into() }]);
    assert_eq!(cfg.reactions[0].tag, None);
    let full: OpcConfig = toml::from_str(&format!("bind = \"10.0.0.5\"\nport = 4840\nuser = {{ name = \"op\", password = \"pw\" }}\nmax_nodes_per_read = 800\nmax_sessions = 2\n{text}")).unwrap();
    assert_eq!((full.bind.as_str(), full.port, full.max_nodes_per_read, full.max_sessions), ("10.0.0.5", 4840, Some(800), Some(2)));
    assert_eq!(full.user.as_ref().map(|u| u.name.as_str()), Some("op"));
    assert_eq!(toml::from_str::<OpcConfig>("").unwrap(), OpcConfig::default());

    let ramped: OpcConfig = toml::from_str("[[tags]]\nid = \"p\"\ntype = \"float\"\nramp = { low = 0, high = 100 }\n\n[[tags]]\nid = \"q\"\ntype = \"int\"\nramp = { low = 0, high = 10, step = 3, interval_ms = 200 }\n").unwrap();
    assert_eq!(ramped.tags[0].ramp, Some(Ramp { low: 0.0, high: 100.0, step: 1.0, decimals: None, interval_ms: 1000 }));
    assert_eq!(ramped.tags[1].ramp, Some(Ramp { low: 0.0, high: 10.0, step: 3.0, decimals: None, interval_ms: 200 }));
    assert!(toml::from_str::<OpcConfig>("[[tags]]\nid = \"p\"\ntype = \"float\"\nramp = { low = 0, high = 100, stepp = 2 }\n").unwrap_err().to_string().contains("stepp"));
    assert!(toml::from_str::<OpcConfig>("[[tags]]\nid = \"p\"\ntype = \"float\"\nramp = { high = 100 }\n").unwrap_err().to_string().contains("low"));
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
