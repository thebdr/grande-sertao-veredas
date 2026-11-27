//! Core's OPC UA server (contract: the OPC UA server clause; `docs/core-plan.md`, "The OPC UA
//! server"), met by core's own OPC UA client over real OPC UA on loopback: the browser finds the
//! caller's folder and variables, the poll worker reads what the caller sets, its gated writes land
//! and come back to the caller in order, and the write guard, the access levels, the caller's
//! knobs, the bound port, the server's runtime and its stop do what the clause says.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use veredas_core::SeriesType;
use veredas_core::opc::browse::{BrowseEvent, Browser};
use veredas_core::opc::server::{
    Access, ClientWrite, Credentials, Folder, Server, ServerError, ServerSpec, SetError, VarType,
    VarValue, Variable, WRITES_KEPT, parse_value, render_value,
};
use veredas_core::opc::spec::{Auth, PollSpec, RuleSpec, TagSpec};
use veredas_core::opc::worker::{ConnState, OpcEvent, OpcWorker, TagRead};
use veredas_core::opc::{StatusCode, Variant, WriteGate, parse_node_id};

const NAMESPACE: &str = "urn:veredas:test:values";
const APP: &str = "urn:veredas:test";

fn var(id: &str, name: Option<&str>, ty: VarType, access: Access, value: Variant) -> Variable {
    Variable {
        id: id.into(),
        name: name.map(str::to_string),
        ty,
        access,
        value,
    }
}

/// Four variables, one of each type, and every access among them.
fn spec() -> ServerSpec {
    ServerSpec {
        bind: "127.0.0.1".into(),
        port: 0,
        application_uri: APP.into(),
        application_name: "veredas test".into(),
        namespace: NAMESPACE.into(),
        folder: Folder {
            id: "line".into(),
            name: "Line".into(),
        },
        user: None,
        max_sessions: None,
        max_nodes_per_read: None,
        variables: vec![
            var(
                "count",
                Some("Count"),
                VarType::Int,
                Access::ReadWrite,
                Variant::Int32(3),
            ),
            var(
                "level",
                None,
                VarType::Float,
                Access::Read,
                Variant::Double(1.5),
            ),
            var(
                "start",
                Some("Start"),
                VarType::Bool,
                Access::Write,
                Variant::Boolean(false),
            ),
            var(
                "batch",
                Some("Batch"),
                VarType::Text,
                Access::ReadWrite,
                Variant::from("B-1"),
            ),
        ],
    }
}

/// The same four variables, every one read-write, so a client reads each one's DataType and
/// writes each one.
fn writable_spec() -> ServerSpec {
    let mut s = spec();
    for v in &mut s.variables {
        v.access = Access::ReadWrite;
    }
    s
}

/// The stack's own client, for what core's worker never sends; configured as the worker's is.
fn raw_client() -> opcua_client::Client {
    opcua_client::ClientBuilder::new()
        .application_name("veredas test client")
        .application_uri("urn:veredas:test:client")
        .product_uri("urn:veredas:test:client")
        .create_sample_keypair(false)
        .trust_server_certs(true)
        .session_retry_limit(1)
        .client()
        .unwrap()
}

/// An anonymous session of the stack's own client on the server, connected.
async fn raw_session(
    client: &mut opcua_client::Client,
    server: &Server,
) -> (
    Arc<opcua_client::Session>,
    tokio::task::JoinHandle<StatusCode>,
) {
    use opcua_types::{MessageSecurityMode, UserTokenPolicy};
    let endpoint = (
        server.endpoint_url.as_str(),
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    );
    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, opcua_client::IdentityToken::Anonymous)
        .await
        .unwrap();
    let handle = event_loop.spawn();
    assert!(
        tokio::time::timeout(Duration::from_secs(15), session.wait_for_connection())
            .await
            .unwrap()
    );
    (session, handle)
}

/// OPC UA's Hello (Part 6, 7.1.2.3): "HEL", a final chunk, its size, protocol version 0, buffers
/// of 65536, no message or chunk limit, the endpoint's URL.
fn hello(url: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for v in [0u32, 65536, 65536, 0, 0] {
        body.extend_from_slice(&v.to_le_bytes());
    }
    body.extend_from_slice(&(url.len() as i32).to_le_bytes());
    body.extend_from_slice(url.as_bytes());
    let mut message = b"HELF".to_vec();
    message.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
    message.extend_from_slice(&body);
    message
}

fn block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

fn tag(server: &Server, id: &str, ty: SeriesType, writable: bool) -> TagSpec {
    let raw = server.node_id_str(id);
    TagSpec {
        node_id: parse_node_id(&raw).unwrap(),
        raw,
        series: id.into(),
        ty,
        writable,
        bits: vec![],
    }
}

/// A poll worker on the server, every variable a writable tag of its own (the server's access
/// levels, not the worker's whitelist, are what these tests meet), armed.
fn worker(server: &Server, auth: Auth) -> (OpcWorker, Receiver<OpcEvent>) {
    let tags = vec![
        tag(server, "count", SeriesType::Int, true),
        tag(server, "level", SeriesType::Float, true),
        tag(server, "start", SeriesType::Int, true),
        tag(server, "batch", SeriesType::Text, true),
    ];
    worker_of(server, auth, tags)
}

/// A poll worker on the server polling these tags, armed.
fn worker_of(server: &Server, auth: Auth, tags: Vec<TagSpec>) -> (OpcWorker, Receiver<OpcEvent>) {
    let spec = PollSpec {
        source_id: "plc".into(),
        display_name: "plc".into(),
        endpoint_url: server.endpoint_url.clone(),
        auth,
        poll: Duration::from_millis(50),
        rules: vec![RuleSpec {
            name: "line".into(),
            tags,
            store_on_change: false,
        }],
    };
    let (tx, rx) = channel();
    let w = veredas_core::opc::spawn(spec, tx, WriteGate::new(true)).unwrap();
    (w, rx)
}

fn next_poll(rx: &Receiver<OpcEvent>, within: Duration) -> Vec<TagRead> {
    let deadline = Instant::now() + within;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(OpcEvent::Poll { values, .. }) => return values,
            Ok(OpcEvent::Status { .. }) => continue,
            Err(RecvTimeoutError::Timeout) => panic!("no poll within {within:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the worker exited"),
        }
    }
}

/// The next status that is not `Connecting`.
fn next_state(rx: &Receiver<OpcEvent>, within: Duration) -> ConnState {
    let deadline = Instant::now() + within;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(OpcEvent::Status {
                state: ConnState::Connecting,
                ..
            }) => continue,
            Ok(OpcEvent::Status { state, .. }) => return state,
            Ok(OpcEvent::Poll { .. }) => continue,
            Err(e) => panic!("no status within {within:?}: {e:?}"),
        }
    }
}

fn wait_for(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    what()
}

fn write(w: &OpcWorker, server: &Server, writes: &[(&str, Variant)]) -> Vec<StatusCode> {
    let writes = writes
        .iter()
        .map(|(id, v)| (server.node_id_str(id), v.clone()))
        .collect();
    block(w.writer.write_tags(writes)).unwrap()
}

#[test]
fn the_callers_folder_and_variables_browse_and_poll_and_what_the_caller_sets_is_read() {
    let server = Server::start(&spec()).unwrap();
    // The browser finds the folder under Objects by the caller's browse name, and in it each
    // variable by its node id, display and browse name (the id when it has none), typed, in the browser's order (by display
    // name, byte for byte). A write-only variable browses untyped: async-opcua 0.19 checks a
    // node's read access for every attribute, its DataType included (`validate_node_read`), so
    // the browser's read of that type is refused.
    let browser = Browser::connect(&server.endpoint_url, Auth::Anonymous);
    let children = |parent: Option<String>| {
        browser.browse(parent.clone());
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match browser
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(BrowseEvent::Children { parent: p, nodes }) if p == parent => return nodes,
                Ok(BrowseEvent::Failed(e)) => panic!("{e:?}"),
                Ok(_) => continue,
                Err(e) => panic!("{e:?}"),
            }
        }
    };
    let root = children(None);
    let folder = root
        .iter()
        .find(|n| n.node_id == server.node_id_str("line"))
        .unwrap_or_else(|| panic!("{root:?}"));
    assert_eq!(
        (folder.browse_name.as_str(), folder.display_name.as_str()),
        ("Line", "Line")
    );
    assert!(folder.is_container && !folder.is_variable);
    let nodes = children(Some(folder.node_id.clone()));
    let seen: Vec<(String, String, String, Option<SeriesType>)> = nodes
        .iter()
        .map(|n| {
            (
                n.node_id.clone(),
                n.display_name.clone(),
                n.browse_name.clone(),
                n.series_type,
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            (
                server.node_id_str("batch"),
                "Batch".into(),
                "Batch".into(),
                Some(SeriesType::Text)
            ),
            (
                server.node_id_str("count"),
                "Count".into(),
                "Count".into(),
                Some(SeriesType::Int)
            ),
            (
                server.node_id_str("start"),
                "Start".into(),
                "Start".into(),
                None
            ),
            (
                server.node_id_str("level"),
                "level".into(),
                "level".into(),
                Some(SeriesType::Float)
            ),
        ]
    );
    assert!(nodes.iter().all(|n| n.is_variable));
    drop(browser);

    // A poll reads the initial values; what the caller sets is what the next polls read.
    let (w, rx) = worker(&server, Auth::Anonymous);
    let reads = next_poll(&rx, Duration::from_secs(15));
    assert_eq!(reads[0].value, Some(Variant::Int32(3)));
    assert_eq!(reads[1].value, Some(Variant::Double(1.5)));
    assert_eq!(reads[3].value, Some(Variant::from("B-1")));
    server.set("count", 41).unwrap();
    server.set("level", 2.25f64).unwrap();
    server.set("batch", "B-2").unwrap();
    assert!(wait_for(Duration::from_secs(5), || {
        let r = next_poll(&rx, Duration::from_secs(5));
        r[0].value == Some(Variant::Int32(41))
            && r[1].value == Some(Variant::Double(2.25))
            && r[3].value == Some(Variant::from("B-2"))
    }));
    // From the caller's side, in the spec's order, the write-only variable included; a set is
    // never handed back as a client's write.
    server.set("start", true).unwrap();
    assert_eq!(server.get("start"), Some(Variant::Boolean(true)));
    assert_eq!(server.get("nothing"), None);
    assert_eq!(
        server.snapshot(),
        vec![
            VarValue {
                id: "count".into(),
                value: Some(Variant::Int32(41))
            },
            VarValue {
                id: "level".into(),
                value: Some(Variant::Double(2.25))
            },
            VarValue {
                id: "start".into(),
                value: Some(Variant::Boolean(true))
            },
            VarValue {
                id: "batch".into(),
                value: Some(Variant::from("B-2"))
            },
        ]
    );
    assert_eq!(server.writes(), vec![]);
    assert_eq!(server.status().client_writes, 0);
    w.handle.join().unwrap();
}

#[test]
fn a_clients_writes_land_and_are_handed_to_the_caller_in_the_order_they_landed() {
    let server = Server::start(&spec()).unwrap();
    let (w, rx) = worker(&server, Auth::Anonymous);
    next_poll(&rx, Duration::from_secs(15));
    // One call of three writes, then two calls: Good each, and handed in that order.
    let statuses = write(
        &w,
        &server,
        &[
            ("count", Variant::Int32(5)),
            ("batch", Variant::from("x")),
            ("count", Variant::Int32(7)),
        ],
    );
    assert_eq!(statuses, vec![StatusCode::Good; 3]);
    assert_eq!(
        write(&w, &server, &[("start", Variant::Boolean(true))]),
        vec![StatusCode::Good]
    );
    assert_eq!(
        write(&w, &server, &[("count", Variant::Int32(8))]),
        vec![StatusCode::Good]
    );
    let mut got = Vec::new();
    assert!(wait_for(Duration::from_secs(5), || {
        got.extend(server.writes());
        got.len() >= 5
    }));
    let w_ = |id: &str, value: Variant| ClientWrite {
        id: id.into(),
        value,
    };
    assert_eq!(
        got,
        vec![
            w_("count", Variant::Int32(5)),
            w_("batch", Variant::from("x")),
            w_("count", Variant::Int32(7)),
            w_("start", Variant::Boolean(true)),
            w_("count", Variant::Int32(8)),
        ]
    );
    // Landed: the caller's side and the next poll read them.
    assert_eq!(server.get("count"), Some(Variant::Int32(8)));
    assert_eq!(server.get("batch"), Some(Variant::from("x")));
    assert_eq!(server.get("start"), Some(Variant::Boolean(true)));
    assert!(wait_for(Duration::from_secs(5), || next_poll(
        &rx,
        Duration::from_secs(5)
    )[0]
    .value
        == Some(Variant::Int32(8))));
    let status = server.status();
    assert_eq!(
        (
            status.client_writes,
            status.writes_dropped,
            status.type_mismatches
        ),
        (5, 0, 0)
    );
    // A set between two client writes is not among them.
    server.set("count", 100).unwrap();
    write(&w, &server, &[("count", Variant::Int32(9))]);
    assert_eq!(
        server.next_write(Duration::from_secs(5)),
        Some(w_("count", Variant::Int32(9)))
    );
    assert_eq!(server.next_write(Duration::from_millis(200)), None);
    w.handle.join().unwrap();
}

/// A write answered `Good` has landed before the server serves the client's next request: the
/// stack runs each Write and each Read in a task of its own on the server's one thread, the
/// guard's landing task is spawned while the Write's runs, and a request read after the Write's
/// answer has left is spawned after it, so the landing runs first. So the client's next read,
/// made at once with no wait, sees each of twenty values, and the caller's get after that read
/// sees it too. (The caller's get at the instant of the answer is not ordered: the session's own
/// task may already be due, woken by another request, and send the answer before the landing
/// runs. What the caller is handed has landed, the next test holds.)
#[test]
fn a_write_answered_good_has_landed_before_the_clients_next_request() {
    let server = Server::start(&spec()).unwrap();
    let (w, rx) = worker_of(
        &server,
        Auth::Anonymous,
        vec![tag(&server, "count", SeriesType::Int, true)],
    );
    next_poll(&rx, Duration::from_secs(15));
    let node = parse_node_id(&server.node_id_str("count")).unwrap();
    for i in 100..120 {
        assert_eq!(
            write(&w, &server, &[("count", Variant::Int32(i))]),
            vec![StatusCode::Good]
        );
        let read = block(w.writer.read(vec![node.clone()])).unwrap();
        assert_eq!(read[0].value, Some(Variant::Int32(i)), "the client's read");
        assert_eq!(
            server.get("count"),
            Some(Variant::Int32(i)),
            "the caller's get"
        );
    }
    w.handle.join().unwrap();
}

/// A write handed to the caller has landed and is counted: through a burst of 4000 rising
/// values, the caller takes each write as it comes, and at once its get reads that value or a
/// later one, never an earlier one, and the status counts at least as many writes as it has taken.
#[test]
fn a_write_handed_to_the_caller_has_landed_already() {
    let server = Server::start(&spec()).unwrap();
    let (w, rx) = worker_of(
        &server,
        Auth::Anonymous,
        vec![tag(&server, "count", SeriesType::Int, true)],
    );
    next_poll(&rx, Duration::from_secs(15));
    let total = 4000;
    let (earlier, taken) = std::thread::scope(|scope| {
        let server = &server;
        let caller = scope.spawn(move || {
            let (mut earlier, mut taken) = (Vec::new(), 0);
            let mut behind = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(30);
            while taken < total && Instant::now() < deadline {
                for cw in server.writes() {
                    let Variant::Int32(v) = cw.value else {
                        panic!("{cw:?}")
                    };
                    match server.get("count") {
                        Some(Variant::Int32(now)) if now >= v => {}
                        other => earlier.push((v, other)),
                    }
                    taken += 1;
                    let counted = server.status().client_writes;
                    if counted < taken as u64 {
                        behind.push((taken, counted));
                    }
                }
            }
            assert_eq!(
                behind,
                Vec::<(i32, u64)>::new(),
                "taken and not yet counted"
            );
            (earlier, taken)
        });
        let batch: Vec<(String, Variant)> = (10..10 + total)
            .map(|i| (server.node_id_str("count"), Variant::Int32(i)))
            .collect();
        for chunk in batch.chunks(800) {
            assert!(
                block(w.writer.write_tags(chunk.to_vec()))
                    .unwrap()
                    .iter()
                    .all(|s| *s == StatusCode::Good)
            );
        }
        caller.join().unwrap()
    });
    assert_eq!(
        (taken, earlier.len()),
        (total, 0),
        "{:?}",
        &earlier[..earlier.len().min(5)]
    );
    w.handle.join().unwrap();
}

#[test]
fn the_guard_refuses_another_wire_type_and_counts_it_and_the_access_levels_hold() {
    let server = Server::start(&spec()).unwrap();
    let (w, rx) = worker(&server, Auth::Anonymous);
    next_poll(&rx, Duration::from_secs(15));
    // A Double into the Int32, an Int32 into the bool, a bool into the text: each refused
    // `BadTypeMismatch`, nothing landed, nothing handed, each counted, the last kept.
    let statuses = write(
        &w,
        &server,
        &[
            ("count", Variant::Double(1.5)),
            ("start", Variant::Int32(1)),
            ("batch", Variant::Boolean(true)),
        ],
    );
    assert_eq!(statuses, vec![StatusCode::BadTypeMismatch; 3]);
    assert_eq!(server.get("count"), Some(Variant::Int32(3)));
    assert_eq!(server.get("start"), Some(Variant::Boolean(false)));
    assert_eq!(server.get("batch"), Some(Variant::from("B-1")));
    let status = server.status();
    assert_eq!((status.type_mismatches, status.client_writes), (3, 0));
    assert_eq!(
        status.last_mismatch,
        Some(("batch".into(), "Boolean(true)".into()))
    );
    // The caller's set of another type is refused, counted with them, and changes nothing.
    assert_eq!(
        server.set("level", 2),
        Err(SetError::TypeMismatch {
            id: "level".into(),
            ty: "float",
            value: "Int32(2)".into()
        })
    );
    assert_eq!(
        server.set("nothing", 2),
        Err(SetError::Unknown("nothing".into()))
    );
    assert_eq!(server.get("level"), Some(Variant::Double(1.5)));
    assert_eq!(server.status().type_mismatches, 4);
    assert_eq!(
        server.status().last_mismatch,
        Some(("level".into(), "Int32(2)".into()))
    );
    // Read-only: a client's write is refused by the stack, nothing lands or is handed.
    let statuses = write(&w, &server, &[("level", Variant::Double(9.0))]);
    assert!(statuses[0].is_bad(), "{statuses:?}");
    assert_eq!(server.get("level"), Some(Variant::Double(1.5)));
    // Write-only: a client reads nothing back, its write lands and is handed.
    let reads = next_poll(&rx, Duration::from_secs(5));
    assert!(
        reads[2].status.is_bad() && reads[2].value.is_none(),
        "{:?}",
        reads[2]
    );
    assert_eq!(
        write(&w, &server, &[("start", Variant::Boolean(true))]),
        vec![StatusCode::Good]
    );
    assert_eq!(
        server.next_write(Duration::from_secs(5)),
        Some(ClientWrite {
            id: "start".into(),
            value: Variant::Boolean(true)
        })
    );
    assert_eq!(server.get("start"), Some(Variant::Boolean(true)));
    assert_eq!(server.writes(), vec![]);
    w.handle.join().unwrap();
}

/// What core's worker never sends, sent through the stack's own client: a write with an index
/// range, refused `BadIndexRangeInvalid` by the guard; one carrying no value, which the stack
/// answers `BadTypeMismatch` before the guard sees it, uncounted; and one carrying the Null
/// variant, which the stack lets through and the guard refuses as another wire type, counted.
/// Nothing of them lands or is handed; a right write in the same call lands.
#[test]
fn a_write_with_an_index_range_or_no_value_is_refused_and_nothing_lands() {
    use opcua_types::{AttributeId, DataValue, NumericRange, WriteValue};
    let server = Server::start(&spec()).unwrap();
    let results = block(async {
        let mut client = raw_client();
        let (session, handle) = raw_session(&mut client, &server).await;
        let write = |id: &str, value: DataValue, range: NumericRange| WriteValue {
            node_id: server.node_id(id),
            attribute_id: AttributeId::Value as u32,
            index_range: range,
            value,
        };
        let results = session
            .write(&[
                write(
                    "count",
                    DataValue::value_only(Variant::Int32(4)),
                    NumericRange::Index(0),
                ),
                write(
                    "batch",
                    DataValue::value_only(Variant::from("y")),
                    NumericRange::Range(0, 1),
                ),
                write("count", DataValue::null(), NumericRange::None),
                write(
                    "count",
                    DataValue::value_only(Variant::Empty),
                    NumericRange::None,
                ),
                write(
                    "batch",
                    DataValue::value_only(Variant::from("z")),
                    NumericRange::None,
                ),
            ])
            .await
            .unwrap();
        let _ = session.disconnect().await;
        handle.abort();
        results
    });
    assert_eq!(
        results,
        vec![
            StatusCode::BadIndexRangeInvalid,
            StatusCode::BadIndexRangeInvalid,
            StatusCode::BadTypeMismatch,
            StatusCode::BadTypeMismatch,
            StatusCode::Good,
        ]
    );
    assert_eq!(
        server.next_write(Duration::from_secs(5)),
        Some(ClientWrite {
            id: "batch".into(),
            value: Variant::from("z")
        })
    );
    assert_eq!(server.next_write(Duration::from_millis(200)), None);
    assert_eq!(server.get("count"), Some(Variant::Int32(3)));
    assert_eq!(server.get("batch"), Some(Variant::from("z")));
    let status = server.status();
    assert_eq!((status.type_mismatches, status.client_writes), (1, 1));
    assert_eq!(status.last_mismatch, Some(("count".into(), "Empty".into())));
}

/// The folder is a FolderType organized by the Objects folder, as the stack's own browse shows it.
/// Each type is its wire type: the DataType a client reads of each variable is Int32 (i=6),
/// Double (i=11), Boolean (i=1) and String (i=12). A neighbouring type is refused like any
/// other: an Int64, a UInt32 and an Int16 into the Int32, a Float into the Double, a Byte into
/// the Boolean, each `BadTypeMismatch`, counted, nothing landed or handed; then a right write of
/// each lands. And discovery shows the caller's application: its URI, its name and the endpoint.
#[test]
fn each_type_is_its_wire_type_a_neighbour_is_refused_and_discovery_shows_the_callers_application() {
    use opcua_types::{
        AttributeId, DataTypeId, DataValue, NodeId, NumericRange, QualifiedName, ReadValueId,
        TimestampsToReturn, WriteValue,
    };
    let server = Server::start(&writable_spec()).unwrap();
    let ids = ["count", "level", "start", "batch"];
    let (folder, namespaces, types, refused, after_refused, right) = block(async {
        use opcua_types::{
            BrowseDescription, BrowseDirection, BrowseResultMask, ObjectId, ReferenceTypeId,
        };
        let mut client = raw_client();
        let (session, handle) = raw_session(&mut client, &server).await;
        // The folder as the stack shows it under Objects: its reference and its type.
        let objects = BrowseDescription {
            node_id: ObjectId::ObjectsFolder.into(),
            browse_direction: BrowseDirection::Forward,
            reference_type_id: ReferenceTypeId::HierarchicalReferences.into(),
            include_subtypes: true,
            node_class_mask: 0,
            result_mask: BrowseResultMask::All as u32,
        };
        let browsed = session.browse(&[objects], 0, None).await.unwrap();
        let folder: Vec<(NodeId, NodeId, String, String)> = browsed[0]
            .references
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.node_id.node_id == server.node_id("line"))
            .map(|r| {
                (
                    r.reference_type_id,
                    r.type_definition.node_id,
                    r.browse_name.name.as_ref().to_string(),
                    r.display_name.text.as_ref().to_string(),
                )
            })
            .collect();
        // The server's namespaces, as a client reads them: the data namespace at the index the
        // server gives, the application's URI the server's own.
        let namespaces = session
            .read(
                &[ReadValueId {
                    node_id: opcua_types::VariableId::Server_NamespaceArray.into(),
                    attribute_id: AttributeId::Value as u32,
                    index_range: NumericRange::None,
                    data_encoding: QualifiedName::null(),
                }],
                TimestampsToReturn::Neither,
                0.0,
            )
            .await
            .unwrap();
        let namespaces: Vec<String> = match &namespaces[0].value {
            Some(Variant::Array(a)) => a.values.iter().map(|v| format!("{v}")).collect(),
            other => panic!("the namespace array: {other:?}"),
        };
        let reads: Vec<ReadValueId> = ids
            .iter()
            .map(|id| ReadValueId {
                node_id: server.node_id(id),
                attribute_id: AttributeId::DataType as u32,
                index_range: NumericRange::None,
                data_encoding: QualifiedName::null(),
            })
            .collect();
        let types = session
            .read(&reads, TimestampsToReturn::Neither, 0.0)
            .await
            .unwrap();
        let write = |id: &str, v: Variant| WriteValue {
            node_id: server.node_id(id),
            attribute_id: AttributeId::Value as u32,
            index_range: NumericRange::None,
            value: DataValue::value_only(v),
        };
        let refused = session
            .write(&[
                write("count", Variant::Int64(7)),
                write("count", Variant::UInt32(7)),
                write("count", Variant::Int16(7)),
                write("level", Variant::Float(2.5)),
                write("start", Variant::Byte(1)),
            ])
            .await
            .unwrap();
        // Nothing of them landed: the next read, which the server serves after them, finds the
        // initial values.
        let values: Vec<ReadValueId> = ids
            .iter()
            .map(|id| ReadValueId {
                node_id: server.node_id(id),
                attribute_id: AttributeId::Value as u32,
                index_range: NumericRange::None,
                data_encoding: QualifiedName::null(),
            })
            .collect();
        let after_refused: Vec<Option<Variant>> = session
            .read(&values, TimestampsToReturn::Neither, 0.0)
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.value)
            .collect();
        let right = session
            .write(&[
                write("count", Variant::Int32(7)),
                write("level", Variant::Double(2.5)),
                write("start", Variant::Boolean(true)),
                write("batch", Variant::from("B-7")),
            ])
            .await
            .unwrap();
        let _ = session.disconnect().await;
        handle.abort();
        (folder, namespaces, types, refused, after_refused, right)
    });
    assert_eq!(
        folder,
        vec![(
            NodeId::from(opcua_types::ReferenceTypeId::Organizes),
            NodeId::from(opcua_types::ObjectTypeId::FolderType),
            "Line".to_string(),
            "Line".to_string()
        )],
        "the folder, organized by Objects, a FolderType, named as the caller named it"
    );
    assert_eq!(
        namespaces
            .get(server.namespace as usize)
            .map(String::as_str),
        Some(NAMESPACE),
        "{namespaces:?}"
    );
    assert_eq!(
        namespaces.get(1).map(String::as_str),
        Some(APP),
        "{namespaces:?}"
    );
    assert_eq!(
        after_refused,
        vec![
            Some(Variant::Int32(3)),
            Some(Variant::Double(1.5)),
            Some(Variant::Boolean(false)),
            Some(Variant::from("B-1")),
        ]
    );
    let types: Vec<Option<Variant>> = types.into_iter().map(|d| d.value).collect();
    let want: Vec<Option<Variant>> = [
        DataTypeId::Int32,
        DataTypeId::Double,
        DataTypeId::Boolean,
        DataTypeId::String,
    ]
    .into_iter()
    .map(|t| Some(Variant::from(NodeId::from(t))))
    .collect();
    assert_eq!(types, want);
    assert_eq!(refused, vec![StatusCode::BadTypeMismatch; 5]);
    assert_eq!(right, vec![StatusCode::Good; 4]);
    let mut handed = Vec::new();
    while let Some(w) = server.next_write(Duration::from_secs(5)) {
        handed.push(w);
        if handed.len() == 4 {
            break;
        }
    }
    let w_ = |id: &str, value: Variant| ClientWrite {
        id: id.into(),
        value,
    };
    assert_eq!(
        handed,
        vec![
            w_("count", Variant::Int32(7)),
            w_("level", Variant::Double(2.5)),
            w_("start", Variant::Boolean(true)),
            w_("batch", Variant::from("B-7")),
        ]
    );
    assert_eq!(server.next_write(Duration::from_millis(200)), None);
    // Counted by the time they are handed.
    let status = server.status();
    assert_eq!((status.type_mismatches, status.client_writes), (5, 4));
    assert_eq!(
        status.last_mismatch,
        Some(("start".into(), "Byte(1)".into()))
    );

    // Discovery, as a client finds a server: every endpoint the server's own URL, the
    // application the caller's.
    let endpoints = block(async {
        raw_client()
            .get_server_endpoints_from_url(server.endpoint_url.as_str())
            .await
            .unwrap()
    });
    assert!(!endpoints.is_empty());
    for e in &endpoints {
        assert_eq!(e.endpoint_url.as_ref(), server.endpoint_url);
        assert_eq!(e.server.application_uri.as_ref(), APP);
        assert_eq!(e.server.application_name.text.as_ref(), "veredas test");
    }
}

#[test]
fn a_spec_that_cannot_be_served_is_refused_by_name_before_anything_binds() {
    // Every refused spec asks for a port already taken: a refusal that came after the bind would
    // be the bind's error instead.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken_port = taken.local_addr().unwrap().port();
    let refused = |s: &ServerSpec| {
        let mut s = s.clone();
        s.port = taken_port;
        let e = Server::start(&s).unwrap_err();
        assert!(
            matches!(
                e,
                ServerError::Variable { .. }
                    | ServerError::NamespaceIsApplication(_)
                    | ServerError::NamespaceIsOpcUa(_)
                    | ServerError::User { .. }
            ),
            "{e:?}"
        );
        e.to_string()
    };
    let mut s = spec();
    s.namespace = APP.into();
    assert_eq!(
        refused(&s),
        "the data namespace urn:veredas:test is the application's URI; give it a URI of its own"
    );
    let mut s = spec();
    s.namespace = "http://opcfoundation.org/UA/".into();
    assert_eq!(
        refused(&s),
        "the data namespace http://opcfoundation.org/UA/ is OPC UA's own; give it a URI of its own"
    );
    for (name, why) in [
        ("", "the name is empty"),
        (
            "ANONYMOUS",
            "the anonymous token's id, which the stack reserves",
        ),
    ] {
        let mut s = spec();
        s.user = Some(Credentials {
            name: name.into(),
            password: "pw".into(),
        });
        assert_eq!(refused(&s), format!("user `{name}`: {why}"));
    }
    let mut s = spec();
    s.variables.push(var(
        "count",
        None,
        VarType::Int,
        Access::Read,
        Variant::Int32(0),
    ));
    assert_eq!(refused(&s), "variable `count`: given twice");
    let mut s = spec();
    s.variables[1].value = Variant::Int32(1);
    assert_eq!(
        refused(&s),
        "variable `level`: its initial value Int32(1) is not float"
    );
    let mut s = spec();
    s.variables[0].id = "line".into();
    assert_eq!(refused(&s), "variable `line`: the folder's id");
    for id in [" count", "count ", "", " "] {
        let mut s = spec();
        s.variables[0].id = id.into();
        assert_eq!(
            refused(&s),
            format!("variable `{id}`: an id is not empty and has no spaces around it")
        );
    }
    let mut s = spec();
    s.folder.id = String::new();
    assert_eq!(refused(&s), "variable ``: the folder's id is empty");
    // A good spec on the port already taken: the bind's own error, naming the address.
    let mut s = spec();
    s.port = taken_port;
    match Server::start(&s) {
        Err(ServerError::Bind { addr, source }) => {
            assert_eq!(addr, format!("127.0.0.1:{}", s.port));
            assert_eq!(source.kind(), std::io::ErrorKind::AddrInUse);
        }
        other => panic!("{other:?}"),
    }
    // A space inside an id is no space around it: such a variable is served, read by a client.
    let mut s = spec();
    s.variables[0].id = "count now".into();
    let server = Server::start(&s).unwrap();
    assert_eq!(server.get("count now"), Some(Variant::Int32(3)));
    assert_eq!(
        server.node_id_str("count now"),
        format!("ns={};s=count now", server.namespace)
    );
    let read = block(async {
        use opcua_types::{
            AttributeId, NumericRange, QualifiedName, ReadValueId, TimestampsToReturn,
        };
        let mut client = raw_client();
        let (session, handle) = raw_session(&mut client, &server).await;
        let read = session
            .read(
                &[ReadValueId {
                    node_id: server.node_id("count now"),
                    attribute_id: AttributeId::Value as u32,
                    index_range: NumericRange::None,
                    data_encoding: QualifiedName::null(),
                }],
                TimestampsToReturn::Neither,
                0.0,
            )
            .await
            .unwrap();
        let _ = session.disconnect().await;
        handle.abort();
        read
    });
    assert_eq!(read[0].value, Some(Variant::Int32(3)));
}

#[test]
fn it_advertises_the_port_it_bound_and_a_stop_frees_the_port_and_drops_its_clients() {
    let server = Server::start(&spec()).unwrap();
    let port = server.port;
    assert_ne!(port, 0);
    assert_eq!(server.endpoint_url, format!("opc.tcp://127.0.0.1:{port}/"));
    assert_eq!(
        server.node_id_str("count"),
        format!("ns={};s=count", server.namespace)
    );
    assert_eq!(
        format!("{server:?}"),
        format!(
            "Server {{ endpoint_url: \"opc.tcp://127.0.0.1:{port}/\", namespace: {}, port: {port} }}",
            server.namespace
        )
    );
    // Bound on the host it was given alone: another loopback address finds nothing there.
    let elsewhere = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.2:{port}").parse().unwrap(),
        Duration::from_secs(2),
    );
    assert_eq!(
        elsewhere.map(|_| ()).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::ConnectionRefused)
    );
    let (w, rx) = worker(&server, Auth::Anonymous);
    assert_eq!(
        next_state(&rx, Duration::from_secs(15)),
        ConnState::Connected
    );
    next_poll(&rx, Duration::from_secs(5));
    // A connection the server has accepted and answered, beside the worker's session: OPC UA's
    // Hello, written by hand, and the server's Acknowledge read back.
    let mut plain = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::io::Write::write_all(&mut plain, &hello(&server.endpoint_url)).unwrap();
    plain
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut ack = [0u8; 28];
    std::io::Read::read_exact(&mut plain, &mut ack).unwrap();
    assert_eq!(&ack[..4], b"ACKF");
    assert_eq!(u32::from_le_bytes([ack[4], ack[5], ack[6], ack[7]]), 28);
    server.stop();
    // The port is free at once, the plain connection closed by the server before the stop
    // returned (its end of file at once, not a wait), and the client has lost its server.
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is free after stop");
    plain
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut buf = [0u8; 64];
    match std::io::Read::read(&mut plain, &mut buf) {
        Ok(0) => {}
        other => panic!("the accepted connection not closed by the stop: {other:?}"),
    }
    assert!(matches!(
        next_state(&rx, Duration::from_secs(15)),
        ConnState::Retrying { .. }
    ));
    w.handle.join().unwrap();
    // A drop stops it too.
    let server = Server::start(&spec()).unwrap();
    let port = server.port;
    drop(server);
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is free after a drop");
}

#[test]
fn a_user_by_name_and_password_is_accepted_and_the_password_never_shows() {
    let mut s = spec();
    s.user = Some(Credentials {
        name: "operator".into(),
        password: "hunter2".into(),
    });
    assert!(!format!("{s:?}").contains("hunter2"), "{s:?}");
    assert_eq!(
        format!("{:?}", s.user.as_ref().unwrap()),
        "Credentials { name: \"operator\", password: \"<hidden>\" }"
    );
    let server = Server::start(&s).unwrap();
    let (w, rx) = worker(
        &server,
        Auth::UserPass {
            username: "operator".into(),
            password: "hunter2".into(),
        },
    );
    assert_eq!(
        next_poll(&rx, Duration::from_secs(15))[0].value,
        Some(Variant::Int32(3))
    );
    w.handle.join().unwrap();
    let (w, rx) = worker(
        &server,
        Auth::UserPass {
            username: "operator".into(),
            password: "wrong".into(),
        },
    );
    assert!(matches!(
        next_state(&rx, Duration::from_secs(15)),
        ConnState::Retrying { .. }
    ));
    w.handle.join().unwrap();
    // Anonymous is accepted beside the user.
    let (w, rx) = worker(&server, Auth::Anonymous);
    next_poll(&rx, Duration::from_secs(15));
    w.handle.join().unwrap();
}

#[test]
fn the_callers_limits_on_one_read_and_on_sessions_hold() {
    let mut s = spec();
    s.max_nodes_per_read = Some(2);
    let server = Server::start(&s).unwrap();
    // The worker polls two tags, within the limit: a poll of four would be refused whole and
    // the worker would drop its session before the reads below.
    let two_tags = vec![
        tag(&server, "count", SeriesType::Int, false),
        tag(&server, "level", SeriesType::Float, false),
    ];
    let (w, rx) = worker_of(&server, Auth::Anonymous, two_tags);
    assert_eq!(
        next_poll(&rx, Duration::from_secs(15))[0].value,
        Some(Variant::Int32(3))
    );
    let node = |id: &str| parse_node_id(&server.node_id_str(id)).unwrap();
    let two = block(w.writer.read(vec![node("count"), node("level")])).unwrap();
    assert_eq!(two[0].value, Some(Variant::Int32(3)));
    let three = block(
        w.writer
            .read(vec![node("count"), node("level"), node("batch")]),
    );
    match three {
        Err(e) => assert!(e.to_string().contains("BadTooManyOperations"), "{e}"),
        Ok(reads) => panic!("a read of three past a limit of two answered {reads:?}"),
    }
    w.handle.join().unwrap();
    drop(server);

    let mut s = spec();
    s.max_sessions = Some(1);
    let server = Server::start(&s).unwrap();
    let (first, rx1) = worker(&server, Auth::Anonymous);
    next_poll(&rx1, Duration::from_secs(15));
    while rx1.try_recv().is_ok() {}
    let (second, rx2) = worker(&server, Auth::Anonymous);
    assert!(matches!(
        next_state(&rx2, Duration::from_secs(15)),
        ConnState::Retrying { .. }
    ));
    second.handle.join().unwrap();
    // The first's session untouched meanwhile: no status of it at all, its polls going on; and
    // a poll made after the refusal, the ones waiting from before it taken first.
    let statuses = rx1
        .try_iter()
        .filter(|e| matches!(e, OpcEvent::Status { .. }))
        .count();
    assert_eq!(statuses, 0, "the first client's session moved");
    next_poll(&rx1, Duration::from_secs(5));
    first.handle.join().unwrap();
}

#[test]
fn a_task_on_the_servers_runtime_runs_on_its_thread_and_ends_with_the_server() {
    struct Ended(Arc<AtomicBool>);
    impl Drop for Ended {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let server = Server::start(&spec()).unwrap();
    let ended = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let guard = Ended(Arc::clone(&ended));
    server.runtime().spawn(async move {
        let _guard = guard;
        let _ = tx.send(std::thread::current().name().map(str::to_string));
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Some("veredas-opc-server".into())
    );
    assert!(!ended.load(Ordering::SeqCst));
    server.stop();
    assert!(ended.load(Ordering::SeqCst), "the task outlived the server");
}

#[test]
fn writes_past_those_kept_land_and_are_counted_not_kept() {
    let server = Server::start(&spec()).unwrap();
    let (w, rx) = worker(&server, Auth::Anonymous);
    next_poll(&rx, Duration::from_secs(15));
    let batch: Vec<(&str, Variant)> = (0..900).map(|i| ("count", Variant::Int32(i))).collect();
    for _ in 0..5 {
        assert!(
            write(&w, &server, &batch)
                .iter()
                .all(|s| *s == StatusCode::Good)
        );
    }
    assert!(wait_for(Duration::from_secs(10), || server
        .status()
        .client_writes
        == 4500));
    assert_eq!(WRITES_KEPT, 4096);
    let status = server.status();
    assert_eq!(status.writes_dropped, 404);
    let kept = server.writes();
    assert_eq!(kept.len(), 4096);
    assert!(
        kept.iter()
            .enumerate()
            .all(|(i, w)| w.value == Variant::Int32((i % 900) as i32)),
        "kept in the order they landed"
    );
    assert_eq!(
        kept[0],
        ClientWrite {
            id: "count".into(),
            value: Variant::Int32(0)
        }
    );
    assert_eq!(server.get("count"), Some(Variant::Int32(899)));
    // The cap is on writes waiting: once the caller has taken them, the next is kept again.
    assert_eq!(
        write(&w, &server, &[("count", Variant::Int32(-1))]),
        vec![StatusCode::Good]
    );
    assert_eq!(
        server.next_write(Duration::from_secs(5)),
        Some(ClientWrite {
            id: "count".into(),
            value: Variant::Int32(-1)
        })
    );
    assert_eq!(server.status().writes_dropped, 404);
    w.handle.join().unwrap();
}

#[test]
fn values_parse_from_text_by_type_and_render_as_a_monitor_shows_them() {
    let cases: &[(VarType, &str, Result<Variant, &str>)] = &[
        (VarType::Int, "42", Ok(Variant::Int32(42))),
        (VarType::Int, " -7 ", Ok(Variant::Int32(-7))),
        (VarType::Int, "", Ok(Variant::Int32(0))),
        (VarType::Int, "1.5", Err("`1.5` is not an int")),
        (
            VarType::Int,
            "2147483648",
            Err("`2147483648` is not an int"),
        ),
        (VarType::Int, "2147483647", Ok(Variant::Int32(i32::MAX))),
        (VarType::Int, "-2147483648", Ok(Variant::Int32(i32::MIN))),
        (VarType::Float, "2.5", Ok(Variant::Double(2.5))),
        (VarType::Float, "0.1", Ok(Variant::Double(0.1))),
        (VarType::Float, "1e300", Ok(Variant::Double(1e300))),
        (VarType::Float, "", Ok(Variant::Double(0.0))),
        (VarType::Float, "inf", Err("`inf` is not a finite float")),
        (VarType::Float, "NaN", Err("`NaN` is not a finite float")),
        (VarType::Float, "x", Err("`x` is not a float")),
        (VarType::Bool, "ON", Ok(Variant::Boolean(true))),
        (VarType::Bool, "true", Ok(Variant::Boolean(true))),
        (VarType::Bool, "TRUE", Ok(Variant::Boolean(true))),
        (VarType::Bool, "Yes", Ok(Variant::Boolean(true))),
        (VarType::Bool, "On", Ok(Variant::Boolean(true))),
        (VarType::Bool, "false", Ok(Variant::Boolean(false))),
        (VarType::Bool, "False", Ok(Variant::Boolean(false))),
        (VarType::Bool, "0", Ok(Variant::Boolean(false))),
        (VarType::Bool, "No", Ok(Variant::Boolean(false))),
        (VarType::Bool, "OFF", Ok(Variant::Boolean(false))),
        (VarType::Bool, " no ", Ok(Variant::Boolean(false))),
        (VarType::Bool, "2", Err("`2` is not a bool")),
        (VarType::Bool, "yes", Ok(Variant::Boolean(true))),
        (VarType::Bool, "1", Ok(Variant::Boolean(true))),
        (VarType::Bool, "off", Ok(Variant::Boolean(false))),
        (VarType::Bool, "", Ok(Variant::Boolean(false))),
        (VarType::Bool, "maybe", Err("`maybe` is not a bool")),
        (VarType::Text, " a b ", Ok(Variant::from(" a b "))),
        (VarType::Text, "", Ok(Variant::from(""))),
    ];
    for (ty, text, want) in cases {
        let got = parse_value(*ty, text);
        assert_eq!(got, want.clone().map_err(str::to_string), "{ty:?} {text:?}");
        if let Ok(v) = &got {
            assert!(ty.holds(v));
        }
    }
    assert_eq!(render_value(&None), "-");
    assert_eq!(render_value(&Some(Variant::Double(2.5))), "2.5");
    assert_eq!(render_value(&Some(Variant::Int32(-3))), "-3");
    assert_eq!(render_value(&Some(Variant::Boolean(true))), "true");
    assert_eq!(render_value(&Some(Variant::from("B-1"))), "B-1");
    assert_eq!(render_value(&Some(Variant::Empty)), "(empty)");
    assert_eq!(render_value(&Some(Variant::Float(2.5))), "2.5");
    assert_eq!(render_value(&Some(Variant::Int64(5))), "5");
    assert_eq!(render_value(&Some(Variant::UInt32(6))), "6");
    assert_eq!(render_value(&Some(Variant::Byte(1))), "Byte(1)");
    assert_eq!(
        [VarType::Int, VarType::Float, VarType::Bool, VarType::Text].map(VarType::label),
        ["int", "float", "bool", "text"]
    );
    assert_eq!(
        [Access::Read, Access::Write, Access::ReadWrite].map(Access::label),
        ["read", "write", "read/write"]
    );
}
