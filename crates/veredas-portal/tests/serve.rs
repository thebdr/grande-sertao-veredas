//! The server (contract: the server clause): over a running session on a real socket, with a raw
//! TCP client (what reaches a browser): the live stream's hello and data frames, the session
//! snapshot, the series listing and a window downsampled to a budget, the tables and a page of
//! rows, a tag read on demand, and the write surface refused by default on the wire, armed in
//! process alone, and whitelisted. Against the bench's plant.

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use common::plant::*;
use serde_json::{Value, json};
use veredas_core::opc::Variant;
use veredas_portal::session::{Live, Session, SessionConfig};
use veredas_portal::store::{RowsQuery, TableDef};

struct Server {
    _rt: tokio::runtime::Runtime,
    addr: SocketAddr,
}

/// The server on an ephemeral port, on a runtime of its own.
fn serve(live: Arc<Live>) -> Server {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(veredas_portal::serve::serve(listener, live));
    Server { _rt: rt, addr }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&after[..n]);
        rest = &after[n + 2..];
    }
    out
}

/// One request over raw TCP, the connection closed by the server: the status and the body.
fn http(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a header block");
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(body) } else { body.to_string() };
    (status, body)
}

fn get(addr: SocketAddr, path: &str) -> (u16, Value) {
    let (status, body) = http(addr, "GET", path, None);
    (status, serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}")))
}

fn post(addr: SocketAddr, path: &str, body: &Value) -> (u16, Value) {
    let (status, text) = http(addr, "POST", path, Some(&body.to_string()));
    (status, serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}: {text}")))
}

/// A query value with the characters a node id carries escaped.
fn enc(s: &str) -> String {
    s.replace('%', "%25").replace('=', "%3D").replace(';', "%3B").replace('&', "%26").replace(' ', "%20")
}

fn midnight() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

fn total(live: &Live, table: &str) -> u64 {
    live.store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).map(|p| p.total).unwrap_or(0)
}

#[test]
fn the_snapshot_the_listing_the_windows_and_the_tables_over_a_running_session() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = plant(dir.path(), port);
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..40);
    let mut cfg = session_config("plant", &log, &bench, port);
    cfg.start = Some(midnight());
    cfg.tables.push(TableDef { name: "warm".into(), sql: "SELECT * FROM readings WHERE temp > 30".into() });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let addr = server.addr;
    assert!(wait(Duration::from_secs(15), || total(&live, "readings") == 40 && total(&live, "plc_main") >= 3 && total(&live, "announce") >= 1));

    // The snapshot, from the caches: what runs, its sources, the two clocks, the switch.
    let (status, v) = get(addr, "/api/session");
    assert_eq!(status, 200);
    assert_eq!((v["running"].clone(), v["session_name"].clone(), v["writes_armed"].clone()), (json!(true), json!("plant"), json!(false)));
    assert_eq!(
        v["sources"].as_array().unwrap().iter().map(|s| (s["id"].as_str().unwrap(), s["kind"].as_str().unwrap(), s["has_timestamp"].as_bool().unwrap())).collect::<Vec<_>>(),
        [("log", "log", true), ("plc", "opc", false), ("sorter", "channel", false)]
    );
    assert_eq!(v["start_ms"], json!(midnight().and_utc().timestamp_millis()));
    assert!(v["now_ms"].is_i64() && v["arrival_now_ms"].is_i64() && v["started_ms"].is_i64(), "{v}");
    // The health, the vitals, the counters and the listing catch up within a refresh: every
    // snapshot the test reads next is waited for here, the channel's series included (its first
    // message may land after the refresh that showed the log's lines).
    assert!(
        wait(Duration::from_secs(5), || {
            let (_, v) = get(addr, "/api/session");
            let (_, s) = get(addr, "/api/series");
            let listed = |source: &str, series: &str| {
                s["series"].as_array().is_some_and(|a| a.iter().any(|e| e["source"] == source && e["series"] == series))
            };
            ["log", "plc", "sorter"].iter().all(|id| v["health"][id]["state"] == "live")
                && v["samples"].as_u64().unwrap_or(0) >= 40
                && v["counters"]["lines"] == 40
                && listed("plc", "speed")
                && listed("sorter", "item")
        }),
        "{}\n{}",
        get(addr, "/api/session").1,
        get(addr, "/api/series").1
    );
    let (_, v) = get(addr, "/api/session");
    assert_eq!(v["health"]["log"]["count"], 20 + 20);
    assert_eq!(v["health"]["log"]["last_ts"], json!(stamp_ms(39)));

    // The listing: from the ingest's cache, typed and counted, sorted by source then series.
    let (_, v) = get(addr, "/api/series");
    let series = v["series"].as_array().unwrap();
    let t = series.iter().find(|s| s["source"] == "log" && s["series"] == "temp").unwrap_or_else(|| panic!("{v}"));
    assert_eq!((t["type"].as_str(), t["count"].as_u64(), t["keys"].as_u64(), t["keys_capped"].as_bool(), t["last_ms"].as_i64()), (Some("float"), Some(40), Some(3), Some(false), Some(stamp_ms(39))));
    assert!(series.iter().any(|s| s["source"] == "plc" && s["series"] == "speed" && s["type"] == "float"), "{v}");
    assert!(series.iter().any(|s| s["source"] == "sorter" && s["series"] == "item"), "{v}");
    let order: Vec<(String, String)> = series.iter().map(|s| (s["source"].as_str().unwrap().to_string(), s["series"].as_str().unwrap().to_string())).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);

    // A window downsampled to a budget: the endpoints survive, the total is the raw count.
    let (_, v) = get(addr, "/api/series/data?source=log&series=temp&points=10");
    assert_eq!((v["total"].clone(), v["downsampled"].clone()), (json!(40), json!(true)));
    let pts = v["points"].as_array().unwrap();
    assert_eq!(pts.len(), 10);
    assert_eq!((pts[0].clone(), pts[9].clone()), (json!([stamp_ms(0), temp(0)]), json!([stamp_ms(39), temp(39)])));
    // A narrower window, inclusive at both ends, under the budget: passed through whole.
    let (_, v) = get(addr, &format!("/api/series/data?source=log&series=temp&from={}&to={}", stamp_ms(10), stamp_ms(19)));
    assert_eq!((v["total"].clone(), v["downsampled"].clone(), v["points"].as_array().unwrap().len()), (json!(10), json!(false), 10));
    assert_eq!(v["points"][0], json!([stamp_ms(10), temp(10)]));
    // A key narrows it to one item.
    let (_, v) = get(addr, "/api/series/data?source=log&series=temp&key=A1");
    assert_eq!(v["total"], 13, "items 1, 4, ..., 37");

    // The tables: every base table, the samples table and the project's view.
    let (_, v) = get(addr, "/api/tables");
    let names: Vec<(&str, &str)> = v["tables"].as_array().unwrap().iter().map(|t| (t["name"].as_str().unwrap(), t["kind"].as_str().unwrap())).collect();
    for want in [("readings", "base"), ("plc_main", "base"), ("announce", "base"), ("samples", "samples"), ("warm", "view")] {
        assert!(names.contains(&want), "{want:?} not in {names:?}");
    }
    let readings = v["tables"].as_array().unwrap().iter().find(|t| t["name"] == "readings").unwrap();
    assert_eq!(readings["rows"], 40);
    assert!(readings["columns"].as_array().unwrap().iter().any(|c| c["name"] == "temp"), "{readings}");
    // A page of typed rows, a view, a column charted, and a table that is not there.
    let (_, v) = get(addr, "/api/table/data?name=readings&limit=5&offset=0");
    assert_eq!((v["total"].clone(), v["truncated"].clone(), v["rows"].as_array().unwrap().len()), (json!(40), json!(true), 5));
    assert!(v["schema"].as_array().unwrap().iter().any(|c| c["name"] == "temp"), "{v}");
    let (_, v) = get(addr, "/api/table/data?name=warm&limit=100");
    assert_eq!((v["total"].clone(), v["truncated"].clone()), (json!(19), json!(false)));
    let (_, v) = get(addr, "/api/table/data?name=readings&y=temp&points=10");
    assert_eq!((v["total"].clone(), v["points"].as_array().unwrap().len()), (json!(40), 10));
    assert_eq!(get(addr, "/api/table/data?name=nope"), (404, json!({ "error": "unknown table `nope`" })));

    // A tag read on demand, and one that is not in the session.
    let (status, v) = get(addr, &format!("/api/opc/read?source=plc&tag={}", enc(&node(&bench, "word"))));
    assert_eq!((status, v["ok"].clone(), v["value"].clone()), (200, json!(true), json!("5")));
    let ghost = node(&bench, "ghost");
    assert_eq!(
        get(addr, &format!("/api/opc/read?source=plc&tag={}", enc(&ghost))),
        (404, json!({ "error": format!("`plc` / `{ghost}` is not a tag in this session") }))
    );
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn the_write_surface_is_refused_by_default_on_the_wire_and_whitelisted_when_armed() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = plant(dir.path(), port);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let session = Session::start(session_config("plant", &log, &bench, port)).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let addr = server.addr;
    assert!(wait(Duration::from_secs(15), || total(&live, "plc_main") >= 1 && total(&live, "announce") >= 1));
    let plc = bench.server("plc").unwrap();
    let word = node(&bench, "word");
    let write = |tag: &str, value: &str| post(addr, "/api/opc/write", &json!({ "source": "plc", "tag": tag, "value": value }));
    let send = |message: &str, fields: Value| post(addr, "/api/channels/send", &json!({ "source": "sorter", "message": message, "fields": fields }));

    // Off by default: both paths answer 403 on the wire, and nothing reaches the plant.
    let off = json!({ "error": "writes are turned off for this project" });
    assert_eq!(write(&word, "9"), (403, off.clone()));
    assert_eq!(send("command", json!({ "code": "0" })), (403, off.clone()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(plc.get("word"), Some(Variant::Int32(5)));
    assert_eq!(machine_status(&bench).registers["running"], 1);
    assert_eq!(machine_status(&bench).stats.commands, 0);

    // Armed in process (no endpoint does this): the snapshot says so.
    session.set_writes_armed(true);
    assert_eq!(get(addr, "/api/session").1["writes_armed"], true);
    // The whitelist: a polled tag no rule marks writable, an unknown source, a value that does
    // not fit the type; nothing reaches the plant.
    let speed = node(&bench, "speed");
    assert_eq!(write(&speed, "1.0"), (403, json!({ "error": format!("`plc` / `{speed}` is not a writable tag in this session") })));
    assert_eq!(
        post(addr, "/api/opc/write", &json!({ "source": "ghost", "tag": "x", "value": "1" })),
        (403, json!({ "error": "`ghost` / `x` is not a writable tag in this session" }))
    );
    assert_eq!(write(&word, "abc"), (400, json!({ "error": "`abc` is not a whole number" })));
    assert_eq!(plc.get("word"), Some(Variant::Int32(5)));
    // The write: the server's word changes.
    let (status, v) = write(&word, "9");
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(2), || plc.get("word") == Some(Variant::Int32(9))));
    // The send: an unknown message and a missing value are the catalog's refusals, 400; the
    // command the catalog writes stops the machine.
    assert_eq!(send("nope", json!({})), (400, json!({ "error": "the catalog has no message `nope`" })));
    let (status, v) = send("command", json!({}));
    assert!(status == 400 && v["error"].as_str().unwrap().contains("code"), "{status} {v}");
    assert_eq!(send("command", json!({ "code": "0" })), (200, json!({ "ok": true })));
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).registers["running"] == 0));

    // Disarmed again, live: 403 again, the word keeps its value.
    session.set_writes_armed(false);
    assert_eq!(write(&word, "1"), (403, off.clone()));
    assert_eq!(send("command", json!({ "code": "0" })), (403, off));
    assert_eq!(plc.get("word"), Some(Variant::Int32(9)));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn the_live_stream_says_hello_then_carries_every_numeric_sample() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("stream.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = SessionConfig::new("stream");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let addr = server.addr;

    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(format!("GET /events HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut written = false;
    loop {
        let text = String::from_utf8_lossy(&buf).into_owned();
        // The hello proves the client is subscribed; only then are frames guaranteed to reach it.
        if !written && text.contains("event: hello") {
            assert!(text.contains("200 OK") && text.contains("text/event-stream"), "{text}");
            assert!(text.contains("\"session_name\":\"stream\"") && text.contains("\"running\":true"), "{text}");
            write_lines(&log, 0..5);
            written = true;
        }
        if written && text.matches("\"ser\":\"temp\"").count() >= 5 {
            let mut got = Vec::new();
            for line in text.lines().filter_map(|l| l.strip_prefix("data: ")) {
                let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
                if v["type"] != "data" {
                    continue;
                }
                for smp in v["samples"].as_array().unwrap() {
                    if smp["src"] == "log" && smp["ser"] == "temp" {
                        got.push((smp["ms"].as_i64().unwrap(), smp["v"].as_f64().unwrap()));
                    }
                }
            }
            assert_eq!(got, (0..5).map(|i| (stamp_ms(i), temp(i))).collect::<Vec<_>>());
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the stream:\n{text}");
        let n = s.read(&mut chunk).unwrap_or_else(|e| {
            // Diagnostics for a stall: what the ingest saw, and the file as the folder lists it
            // and as it reads back.
            let by_path = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
            let back = std::fs::read_to_string(&log).map(|t| t.lines().count()).unwrap_or(0);
            let c = live.counters.read().unwrap().clone();
            let health = live.health.read().unwrap().get("log").map(|h| (h.state.clone(), h.count));
            panic!(
                "{e}; lines {} rows {} frames {} receivers {} passes {} health {health:?}; the file by path {by_path} bytes, {back} lines read back; received so far:\n{text}",
                c.lines,
                total(&live, "readings"),
                c.frames,
                live.frames.receiver_count(),
                c.passes
            )
        });
        assert!(n > 0, "the server closed the stream early");
        buf.extend_from_slice(&chunk[..n]);
    }
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The write surface's other answers, the budget's clamp, the stream's lag and keep-alive, and
/// the session's end as the snapshot shows it: disarmed, a body that does not parse is refused
/// by the switch before anything else; armed, an unknown channel is 403 by name, a peer not there
/// 503, a server away 503, the body's shape 400 in the error form; the budget clamped and
/// defaulted; a client that never reads lags once and the stream goes on, a comment every five
/// seconds holding an idle one; after the stop the snapshot says the session no longer runs.
#[test]
fn the_write_surface_s_other_answers_the_budget_the_lag_and_the_end() {
    use veredas_core::opc::{OpcError, WriteError};
    use veredas_portal::serve::{budget, refused_code};
    assert_eq!((budget(None), budget(Some(0)), budget(Some(9)), budget(Some(5001)), budget(Some(usize::MAX))), (1000, 10, 10, 5000, 5000));
    assert_eq!(refused_code(&WriteError::Unconfirmed(OpcError::WorkerDropped)), 504, "a write that may have reached the plant");
    assert_eq!(refused_code(&WriteError::Disarmed), 403);

    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = plant(dir.path(), port);
    let word = node(&bench, "word");
    let log = dir.path().join("rest.log");
    write_lines(&log, 0..40);
    // The channel dials a port nobody listens on: a peer never there.
    let mut cfg = session_config("rest", &log, &bench, free_port());
    cfg.start = Some(midnight());
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let addr = server.addr;
    assert!(wait(Duration::from_secs(10), || {
        let (_, v) = get(addr, "/api/session");
        v["health"]["log"]["state"] == "live" && v["health"]["plc"]["state"] == "live" && v["counters"]["lines"] == 40
    }), "{}", get(addr, "/api/session").1);

    // Disarmed: the switch answers before the body is even parsed.
    let off = json!({ "error": "writes are turned off for this project" });
    assert_eq!(post(addr, "/api/opc/write", &json!({ "source": "plc", "tag": word, "value": 9 })), (403, off.clone()));
    assert_eq!(post(addr, "/api/channels/send", &json!({ "source": "sorter" })), (403, off.clone()));
    let (st, text) = http(addr, "POST", "/api/opc/write", Some("not json"));
    assert_eq!((st, serde_json::from_str::<Value>(&text).unwrap()), (403, off.clone()));

    session.set_writes_armed(true);
    // Armed: a body that is not the shape is 400 in the error form.
    let (st, body) = post(addr, "/api/opc/write", &json!({ "source": "plc", "tag": word, "value": 9 }));
    assert_eq!(st, 400, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("the body is not a tag write"), "{body}");
    let (st, body) = post(addr, "/api/channels/send", &json!({ "source": "sorter" }));
    assert_eq!(st, 400, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("the body is not a message send"), "{body}");
    // An unknown channel is 403 by name, as an unknown OPC source is; a peer not there is 503.
    assert_eq!(
        post(addr, "/api/channels/send", &json!({ "source": "ghost", "message": "command", "fields": { "code": "0" } })),
        (403, json!({ "error": "`ghost` is no channel of this session" }))
    );
    assert_eq!(
        post(addr, "/api/channels/send", &json!({ "source": "sorter", "message": "command", "fields": { "code": "0" } })),
        (503, json!({ "error": "no peer is connected" }))
    );
    // The budget over the socket: below the minimum, ten points.
    let (st, v) = get(addr, &format!("/api/series/data?source=log&series=temp&from=0&to={}&points=3", i64::MAX));
    assert_eq!(st, 200, "{v}");
    assert_eq!((v["points"].as_array().unwrap().len(), v["downsampled"].as_bool()), (10, Some(true)), "{v}");

    // The stream: a client that never reads lags once more than the channel holds, and the
    // stream goes on; an idle connection carries a comment every five seconds.
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(format!("GET /events HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let mut read_until = |s: &mut TcpStream, buf: &mut Vec<u8>, needle: &str, within: Duration| -> String {
        let deadline = Instant::now() + within;
        loop {
            let text = String::from_utf8_lossy(buf).into_owned();
            if text.contains(needle) {
                return text;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {needle:?}:\n{}", &text[text.len().saturating_sub(2000)..]);
            match s.read(&mut chunk) {
                Ok(0) => panic!("the stream closed waiting for {needle:?}"),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => panic!("{e}"),
            }
        }
    };
    read_until(&mut s, &mut buf, "event: hello", Duration::from_secs(15));
    let big = format!("{{\"type\":\"data\",\"samples\":[],\"pad\":\"{}\"}}", "x".repeat(65_536));
    for _ in 0..400 {
        let _ = live.frames.send(Arc::from(big.as_str()));
    }
    let _ = live.frames.send(Arc::from("{\"type\":\"data\",\"samples\":[],\"marker\":true}"));
    let text = read_until(&mut s, &mut buf, "\"marker\":true", Duration::from_secs(30));
    assert_eq!(text.matches("event: lag").count(), 1, "one lag event for the frames dropped");
    assert!(text.contains("data: frames dropped"), "{}", &text[text.len().saturating_sub(500)..]);
    assert!(text.find("event: lag").unwrap() < text.find("\"marker\":true").unwrap(), "the stream goes on after the lag");
    // The plant stopped first: the OPC source polls it every 50 ms and its frames never let the
    // stream fall idle; retrying, it sends none, and the keep-alive shows.
    bench.stop();
    assert!(wait(Duration::from_secs(10), || get(addr, "/api/session").1["health"]["plc"]["state"] == "retrying"), "{}", get(addr, "/api/session").1);
    let before = buf.len();
    let deadline = Instant::now() + Duration::from_secs(12);
    let saw_comment = loop {
        let fresh = String::from_utf8_lossy(&buf[before..]).into_owned();
        if fresh.lines().any(|l| l.starts_with(':')) {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        match s.read(&mut chunk) {
            Ok(0) => break false,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
    };
    assert!(saw_comment, "a keep-alive comment on an idle stream within twelve seconds; got {:?}", String::from_utf8_lossy(&buf[before..]));

    // The server away: a write the plant never saw.
    assert_eq!(post(addr, "/api/opc/write", &json!({ "source": "plc", "tag": word, "value": "7" })), (503, json!({ "error": "not connected to the server" })));

    // The end: the snapshot says the session no longer runs.
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let (_, v) = get(addr, "/api/session");
    assert_eq!(v["running"], false, "{v}");
    assert_eq!(v["health"]["log"]["state"], "ended", "{v}");
}
