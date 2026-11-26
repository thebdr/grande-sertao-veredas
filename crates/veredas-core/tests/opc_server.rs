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
    // variable by its name (the id when it has none), typed, in the browser's order (by display
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
        .find(|n| n.browse_name.ends_with("Line"))
        .unwrap_or_else(|| panic!("{root:?}"));
    assert_eq!(folder.node_id, server.node_id_str("line"));
    assert!(folder.is_container && !folder.is_variable);
    let nodes = children(Some(folder.node_id.clone()));
    let seen: Vec<(String, String, Option<SeriesType>)> = nodes
        .iter()
        .map(|n| (n.node_id.clone(), n.display_name.clone(), n.series_type))
        .collect();
    assert_eq!(
        seen,
        vec![
            (
                server.node_id_str("batch"),
                "Batch".into(),
                Some(SeriesType::Text)
            ),
            (
                server.node_id_str("count"),
                "Count".into(),
                Some(SeriesType::Int)
            ),
            (server.node_id_str("start"), "Start".into(), None),
            (
                server.node_id_str("level"),
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
/// range, refused `BadIndexRangeInvalid` by the guard; and one carrying no value, which the stack
/// answers `BadTypeMismatch` before the guard sees it. Nothing lands, nothing is handed, nothing
/// is counted; a right write in the same call lands.
#[test]
fn a_write_with_an_index_range_or_no_value_is_refused_and_nothing_lands() {
    use opcua_client::{ClientBuilder, IdentityToken};
    use opcua_types::{
        AttributeId, DataValue, MessageSecurityMode, NumericRange, UserTokenPolicy, WriteValue,
    };
    let server = Server::start(&spec()).unwrap();
    let results = block(async {
        let mut client = ClientBuilder::new()
            .application_name("veredas test")
            .application_uri("urn:veredas:test:client")
            .product_uri("urn:veredas:test:client")
            .create_sample_keypair(false)
            .trust_server_certs(true)
            .session_retry_limit(1)
            .client()
            .unwrap();
        let endpoint = (
            server.endpoint_url.as_str(),
            "None",
            MessageSecurityMode::None,
            UserTokenPolicy::anonymous(),
        );
        let (session, event_loop) = client
            .connect_to_matching_endpoint(endpoint, IdentityToken::Anonymous)
            .await
            .unwrap();
        let handle = event_loop.spawn();
        assert!(
            tokio::time::timeout(Duration::from_secs(15), session.wait_for_connection())
                .await
                .unwrap()
        );
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
    assert_eq!((status.type_mismatches, status.client_writes), (0, 1));
    assert_eq!(status.last_mismatch, None);
}

#[test]
fn a_spec_that_cannot_be_served_is_refused_by_name_before_anything_binds() {
    let refused = |s: &ServerSpec| Server::start(s).unwrap_err().to_string();
    let mut s = spec();
    s.namespace = APP.into();
    assert_eq!(
        refused(&s),
        "the data namespace urn:veredas:test is the application's URI; give it a URI of its own"
    );
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
    let mut s = spec();
    s.variables[0].id = " count".into();
    assert_eq!(
        refused(&s),
        "variable ` count`: an id is not empty and has no spaces around it"
    );
    let mut s = spec();
    s.folder.id = String::new();
    assert_eq!(refused(&s), "variable ``: the folder's id is empty");
    // A port already taken: the bind's own error, naming the address.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut s = spec();
    s.port = taken.local_addr().unwrap().port();
    match Server::start(&s) {
        Err(ServerError::Bind { addr, source }) => {
            assert_eq!(addr, format!("127.0.0.1:{}", s.port));
            assert_eq!(source.kind(), std::io::ErrorKind::AddrInUse);
        }
        other => panic!("{other:?}"),
    }
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
    let (w, rx) = worker(&server, Auth::Anonymous);
    assert_eq!(
        next_state(&rx, Duration::from_secs(15)),
        ConnState::Connected
    );
    next_poll(&rx, Duration::from_secs(5));
    server.stop();
    // The port is free at once, and the client has lost its server.
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is free after stop");
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
    let (second, rx2) = worker(&server, Auth::Anonymous);
    assert!(matches!(
        next_state(&rx2, Duration::from_secs(15)),
        ConnState::Retrying { .. }
    ));
    second.handle.join().unwrap();
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
    let status = server.status();
    assert_eq!(status.writes_dropped, 4500 - WRITES_KEPT as u64);
    let kept = server.writes();
    assert_eq!(kept.len(), WRITES_KEPT);
    assert_eq!(
        kept[0],
        ClientWrite {
            id: "count".into(),
            value: Variant::Int32(0)
        }
    );
    assert_eq!(server.get("count"), Some(Variant::Int32(899)));
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
        (VarType::Float, "2.5", Ok(Variant::Double(2.5))),
        (VarType::Float, "", Ok(Variant::Double(0.0))),
        (VarType::Float, "inf", Err("`inf` is not a finite float")),
        (VarType::Float, "NaN", Err("`NaN` is not a finite float")),
        (VarType::Float, "x", Err("`x` is not a float")),
        (VarType::Bool, "ON", Ok(Variant::Boolean(true))),
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
    assert_eq!(
        [VarType::Int, VarType::Float, VarType::Bool, VarType::Text].map(VarType::label),
        ["int", "float", "bool", "text"]
    );
    assert_eq!(
        [Access::Read, Access::Write, Access::ReadWrite].map(Access::label),
        ["read", "write", "read/write"]
    );
}
