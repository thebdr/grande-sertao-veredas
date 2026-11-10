//! C-049: the host. The sessions of a supervisão file started through portal and served side by
//! side on one address, each under its name with its own portal router nested unchanged; their
//! data kept apart; the page and its bundle served; the executable running a file until stdin
//! closes. Against two benches, over real sockets.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, HostError, config, web};

fn objects(addr: SocketAddr, name: &str) -> Value {
    let (status, v) = get(addr, &format!("/s/{name}/objects.json"));
    assert_eq!(status, 200);
    v
}

#[test]
fn two_sessions_are_served_side_by_side_each_under_its_name_with_its_portal() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    b.server("plc").unwrap().set("fill1", 65.0);
    let path = two_plants(dir.path(), &a, &b);
    let cfg = config::load_file(&path).unwrap();
    let host = Host::start(cfg).unwrap();
    let addr = host.addr();
    assert_ne!(addr.port(), 0);

    // The page and its bundle, compiled in.
    let (status, headers, body) = http(addr, "GET", "/", None);
    assert_eq!((status, body.as_str()), (200, web::INDEX_HTML));
    assert!(headers.contains("content-type: text/html; charset=utf-8"), "{headers}");
    for page in ["/s/loop/", "/s/loop", "/s/hall/", "/s/hall"] {
        let (status, headers, body) = http(addr, "GET", page, None);
        assert_eq!((status, body.as_str()), (200, web::INDEX_HTML), "{page}");
        assert!(headers.contains("content-type: text/html; charset=utf-8"), "{page}: {headers}");
    }
    let (status, headers, body) = http(addr, "GET", "/assets/app.js", None);
    assert_eq!((status, body.as_str()), (200, web::APP_JS));
    assert!(headers.contains("content-type: text/javascript; charset=utf-8"));
    let (status, headers, body) = http(addr, "GET", "/assets/app.css", None);
    assert_eq!((status, body.as_str()), (200, web::APP_CSS));
    assert!(headers.contains("content-type: text/css; charset=utf-8"));

    // The sessions, in the file's order.
    assert_eq!(get(addr, "/api/sessions"), (200, json!({ "sessions": [
        { "name": "loop", "title": "The loop", "path": "/s/loop/" },
        { "name": "hall", "title": "The hall", "path": "/s/hall/" },
    ] })));
    // Nothing else is served, and an unknown session is no session.
    for p in ["/s/nope/", "/s/nope/binding", "/api/nope", "/s/loop/nope", "/s/loop/portal/api/nope"] {
        assert_eq!(get(addr, p), (404, json!({ "error": format!("nothing is served at {p}") })), "{p}");
    }

    // Each session's binding, layout and portal are its own.
    let (status, v) = get(addr, "/s/loop/binding");
    assert_eq!(status, 200);
    assert_eq!((v["name"].clone(), v["title"].clone(), v["layout"].clone(), v["headline"].clone()), (json!("loop"), json!("The loop"), json!("layout.svg"), json!(["speed", "read-rate"])));
    assert_eq!(v["objects"].as_array().unwrap().len(), 8);
    assert_eq!(v["objects"][5], json!({
        "id": "outlet-3", "label": "Outlet 3", "unit": "%", "show": "level", "range": [0.0, 100.0], "path": null,
        "source": "plc", "series": "fill3", "key": null, "normal": "ok",
        "rules": [ { "state": "warn", "word": "full", "when": "at least 100 %" }, { "state": "near", "word": "near full", "when": "at least 80 %" } ],
        "actions": [ { "label": "Release", "source": "plc", "tag": format!("ns={};s=release3", a.server("plc").unwrap().namespace), "value": "1" } ],
    }));
    assert_eq!(v["dashboards"], json!([{ "name": "Overview", "columns": 4, "panels": [
        { "kind": "value", "title": "Read rate", "span": 1 }, { "kind": "trend", "title": "Read rate", "span": 2 },
        { "kind": "bars", "title": "Loop speed", "span": 1 }, { "kind": "timeline", "title": "State over time", "span": 1 },
        { "kind": "grid", "title": "States", "span": 1 }, { "kind": "levels", "title": "Levels", "span": 1 },
        { "kind": "table", "title": "outlets", "span": 1 }, { "kind": "cells", "title": "sorts", "span": 1 },
    ] }]));
    let (_, v) = get(addr, "/s/hall/binding");
    assert_eq!((v["name"].clone(), v["title"].clone(), v["layout"].clone(), v["objects"].as_array().unwrap().len()), (json!("hall"), json!("The hall"), Value::Null, 6));
    let (status, headers, body) = http(addr, "GET", "/s/loop/layout.svg", None);
    assert_eq!((status, body.replace("\r\n", "\n")), (200, example().svg));
    assert!(headers.contains("content-type: image/svg+xml"), "{headers}");
    assert_eq!(get(addr, "/s/hall/layout.svg"), (404, json!({ "error": "session `hall` has no layout" })));
    let (status, s) = get(addr, "/s/loop/portal/api/session");
    assert_eq!((status, s["session_name"].clone(), s["running"].clone(), s["writes_armed"].clone()), (200, json!("the loop"), json!(true), json!(false)));
    let (_, s) = get(addr, "/s/hall/portal/api/session");
    assert_eq!(s["session_name"], json!("the hall"));
    // Portal's own live stream, nested as it is: its hello names the session it belongs to.
    for (name, title) in [("loop", "the loop"), ("hall", "the hall")] {
        let mut sse = Sse::open(addr, &format!("/s/{name}/portal/events"));
        assert_eq!(sse.status, 200);
        assert!(sse.headers.contains("content-type: text/event-stream"), "{}", sse.headers);
        let (event, data) = sse.next(Duration::from_secs(10)).unwrap();
        let hello: Value = serde_json::from_str(&data).unwrap();
        assert_eq!((event.as_str(), hello["session_name"].clone(), hello["running"].clone()), ("hello", json!(title), json!(true)));
    }

    // The plants' values stay in their own sessions, on the snapshot routes and on each session's
    // own stream: each stream opens with its session's snapshot and then carries its own plant's
    // changes alone.
    assert!(wait(Duration::from_secs(20), || object(&objects(addr, "loop"), "outlet-1")["value"] == json!(35.0) && object(&objects(addr, "hall"), "outlet-1")["value"] == json!(65.0)));
    let mut streams = Vec::new();
    for (name, fill) in [("loop", 35.0), ("hall", 65.0)] {
        let mut sse = Sse::open(addr, &format!("/s/{name}/objects"));
        assert_eq!(sse.status, 200);
        assert!(sse.headers.contains("content-type: text/event-stream"), "{}", sse.headers);
        let (event, data) = sse.next(Duration::from_secs(10)).unwrap();
        let first: Value = serde_json::from_str(&data).unwrap();
        assert_eq!((event.as_str(), object(&first, "outlet-1")["value"].clone()), ("snapshot", json!(fill)), "{name}");
        assert_eq!(first["objects"].as_array().unwrap().len(), if name == "loop" { 8 } else { 6 });
        streams.push(sse);
    }
    let fill_of = |v: &Value| v["objects"].as_array().and_then(|os| os.iter().find(|o| o["id"] == "outlet-1")).map(|o| o["value"].clone());
    a.server("plc").unwrap().set("fill1", 40.0);
    assert!(streams[0].until(Duration::from_secs(20), |n, v| n == "objects" && fill_of(v) == Some(json!(40.0))).is_some(), "loop's change on loop's stream");
    b.server("plc").unwrap().set("fill1", 70.0);
    let hall_events = streams[1].until(Duration::from_secs(20), |n, v| n == "objects" && fill_of(v) == Some(json!(70.0))).expect("hall's change on hall's stream");
    assert!(hall_events.iter().all(|(_, v)| fill_of(v) != Some(json!(40.0))), "loop's change never on hall's stream: {hall_events:?}");
    assert_eq!((object(&objects(addr, "loop"), "outlet-1")["value"].clone(), object(&objects(addr, "hall"), "outlet-1")["value"].clone()), (json!(40.0), json!(70.0)));
    assert!(objects(addr, "hall")["objects"].as_array().unwrap().iter().all(|o| !o["id"].as_str().unwrap().starts_with("parcel")));
    let (_, s) = get(addr, &format!("/s/hall/portal/api/opc/read?source=plc&tag={}", enc(&format!("ns={};s=fill1", b.server("plc").unwrap().namespace))));
    assert_eq!(s["value"], json!("70"));

    // The dashboards are computed per session, each from its own plant (the levels panel's
    // outlet 1: 0.4 and 0.7); one that is not there is named.
    let level_of = |d: &Value| d["panels"][5]["data"]["rows"].as_array().and_then(|rs| rs.iter().find(|r| r["id"] == "outlet-1")).map(|r| r["fraction"].clone());
    let (status, d) = get(addr, "/s/loop/dashboard/0");
    assert_eq!((status, d["name"].clone(), d["panels"].as_array().unwrap().len(), level_of(&d)), (200, json!("Overview"), 8, Some(json!(0.4))));
    let (status, d) = get(addr, "/s/hall/dashboard/0");
    assert_eq!((status, d["name"].clone(), level_of(&d)), (200, json!("Overview"), Some(json!(0.7))));
    assert_eq!(get(addr, "/s/loop/dashboard/1"), (404, json!({ "error": "session `loop` has no dashboard 1" })));
    assert_eq!(get(addr, "/s/loop/dashboard/x"), (404, json!({ "error": "`x` is no dashboard of session `loop`" })));

    // Stopped with streams open (an object stream, portal's own), every session ends, the stop
    // returns at once, both streams end and the address is let go.
    let mut events = Sse::open(addr, "/s/hall/portal/events");
    assert!(events.next(Duration::from_secs(10)).is_some());
    let stopping = std::time::Instant::now();
    let ends = host.stop();
    // At once: under 3 s (a stop with nothing open takes a quarter of one; blocking tasks get 2 s).
    assert!(stopping.elapsed() < Duration::from_secs(3), "the stop waited on open streams: {:?}", stopping.elapsed());
    assert!(streams[0].ends_within(Duration::from_secs(5)) && events.ends_within(Duration::from_secs(5)), "the open streams end with the stop");
    assert_eq!(ends.iter().map(|(n, e)| (n.as_str(), e.len())).collect::<Vec<_>>(), [("loop", 2), ("hall", 2)]);
    assert!(ends.iter().all(|(_, e)| e.iter().all(|(_, r)| r.is_ok())), "{:?}", ends.iter().map(|(n, e)| (n, e.iter().map(|(i, r)| (i.to_string(), r.is_ok())).collect::<Vec<_>>())).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(5), || TcpStream::connect(addr).is_err()), "the address is free again");
}

#[test]
fn a_session_that_does_not_start_is_named_and_a_bind_taken_is_refused_first() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    // The second session's store is pinned to a file that already stands: its session refuses
    // to open it, so the host does not start.
    let hall = dir.path().join("hall/project.toml");
    let text = std::fs::read_to_string(&hall).unwrap().replace("store = \"ram\"", "store = { disk = \"taken.duckdb\" }");
    std::fs::write(&hall, text).unwrap();
    std::fs::write(dir.path().join("hall/taken.duckdb"), b"not a database").unwrap();
    let cfg = config::load_file(&path).unwrap();
    match Host::start(cfg) {
        Err(HostError::Session { name, reason }) => {
            assert_eq!(name, "hall");
            assert!(reason.contains("taken.duckdb: a database already stands there"), "{reason}");
        }
        other => panic!("{:?}", other.err()),
    }
    // An address taken is refused before any session starts: the same file, whose second
    // session would not start, answers with the address, never with the session.
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("bind = \"127.0.0.1:0\"", &format!("bind = \"{taken}\""));
    std::fs::write(&path, text).unwrap();
    let cfg = config::load_file(&path).unwrap();
    match Host::start(cfg) {
        Err(e @ HostError::Bind { .. }) => assert!(e.to_string().starts_with(&format!("cannot bind {taken}: ")), "{e}"),
        other => panic!("{:?}", other.err()),
    }
    // The executable says the same on stderr and exits 2.
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with(&format!("cannot bind {taken}: ")), "{err}");
    assert!(out.stdout.is_empty(), "nothing served, nothing announced");
    // Freed, the second session's own failure is the executable's answer.
    drop(held);
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("session `hall` did not start: ") && err.contains("taken.duckdb: a database already stands there"), "{err}");
}

#[test]
fn nothing_starts_before_the_address_is_held_nor_under_check() {
    // The first session keeps its store on disk: a session started leaves its file.
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap().replace("store = \"ram\"", "store = { disk = \"loop.duckdb\" }");
    std::fs::write(&project, text).unwrap();
    let store = dir.path().join("loop/loop.duckdb");
    // A check reads, checks and prints the plan, starting nothing.
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).arg("--check").stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("session `loop`"), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(!store.exists(), "a check started a session");
    // An address taken is refused before the first session starts.
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("bind = \"127.0.0.1:0\"", &format!("bind = \"{taken}\""));
    std::fs::write(&path, text).unwrap();
    match Host::start(config::load_file(&path).unwrap()) {
        Err(e @ HostError::Bind { .. }) => assert!(e.to_string().starts_with(&format!("cannot bind {taken}: ")), "{e}"),
        other => panic!("{:?}", other.err()),
    }
    assert!(!store.exists(), "a session started before the address was held");
    drop(held);
}

#[test]
fn the_executable_serves_a_file_until_stdin_closes() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    let mut child = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    out.read_line(&mut first).unwrap();
    let url = first.trim().strip_prefix("serving on http://").and_then(|r| r.strip_suffix('/')).unwrap_or_else(|| panic!("{first}")).to_string();
    let addr: SocketAddr = url.parse().unwrap();
    let mut second = String::new();
    out.read_line(&mut second).unwrap();
    assert_eq!(second.trim(), format!("session `loop` at http://{addr}/s/loop/"));
    let mut third = String::new();
    out.read_line(&mut third).unwrap();
    assert_eq!(third.trim(), format!("session `hall` at http://{addr}/s/hall/"));
    let mut fourth = String::new();
    out.read_line(&mut fourth).unwrap();
    assert_eq!(fourth.trim(), "running; close stdin to stop");
    assert_eq!(get(addr, "/api/sessions").1["sessions"][1]["name"], json!("hall"));
    let (_, s) = get(addr, "/s/loop/portal/api/session");
    assert_eq!(s["session_name"], json!("the loop"));
    // A line on stdin is not a close, in any encoding (one in Latin-1 among them): it keeps serving.
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"hello\n").unwrap();
    stdin.write_all(b"n\xe3o\n").unwrap();
    stdin.flush().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(child.try_wait().unwrap().is_none(), "a line stopped it");
    assert_eq!(get(addr, "/api/sessions").0, 200);
    // Closed with a stream open (a browser on the page), it stops at once, the stream ended.
    let mut sse = Sse::open(addr, "/s/loop/objects");
    assert!(sse.next(Duration::from_secs(10)).is_some());
    drop(stdin);
    let closed = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(closed.elapsed() < Duration::from_secs(3), "it waited on an open stream: {:?}", closed.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status}");
    assert!(sse.ends_within(Duration::from_secs(5)), "the open stream ended");
    assert!(TcpStream::connect(addr).is_err(), "nothing serves after the stop");
}
