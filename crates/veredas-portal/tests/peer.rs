//! Portal to portal, the reading side (C-095): a portal serving a session over the bench's plant,
//! read by another through `peer::PeerClient` over a real socket, as Oficina's portal reads the
//! one embedded in a controller; and peers played by the test for the bounds a reader keeps (a
//! peer that never answers, a stream gone silent, an answer too large or not in the shape).

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use common::plant::*;
use common::ports::held_port;
use serde_json::json;
use veredas_portal::peer::*;
use veredas_portal::session::{Live, Session, SessionConfig};
use veredas_portal::store::{RowsQuery, TableDef};

struct Server {
    _rt: tokio::runtime::Runtime,
    addr: SocketAddr,
}

fn serve(live: Arc<Live>) -> Server {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(veredas_portal::serve::serve(listener, live));
    Server { _rt: rt, addr }
}

/// The reader's own runtime, apart from the served portal's, as two processes would be.
fn reader() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

fn midnight() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

fn total(live: &Live, table: &str) -> u64 {
    live.store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).map(|p| p.total).unwrap_or(0)
}

/// A peer played by the test: every connection's request read whole (its head kept), then
/// answered by `answer`.
fn fake_peer(answer: impl Fn(&mut TcpStream) + Send + Sync + 'static) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heads);
    std::thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(mut s) = s else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            seen.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
            answer(&mut s);
        }
    });
    (addr, heads)
}

#[test]
fn a_portal_reads_another_s_snapshot_listing_windows_tables_and_tags() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..40);
    let mut cfg = session_config("plant", &log, &bench, port);
    cfg.start = Some(midnight());
    cfg.tables.push(TableDef { name: "warm".into(), sql: "SELECT * FROM readings WHERE temp > 30".into() });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let peer = PeerClient::new(server.addr.to_string());
    assert_eq!(peer.peer_addr(), server.addr.to_string());
    let rt = reader();
    assert!(
        wait(Duration::from_secs(15), || total(&live, "readings") == 40 && total(&live, "plc_main") >= 3 && total(&live, "announce") >= 1),
        "readings {} plc_main {} announce {}",
        total(&live, "readings"),
        total(&live, "plc_main"),
        total(&live, "announce")
    );
    // The peer's caches catch up within a refresh: read until every source is live and listed.
    assert!(
        wait(Duration::from_secs(5), || rt.block_on(async {
            let (Ok(s), Ok(series)) = (peer.snapshot().await, peer.series().await) else { return false };
            let listed = |source: &str, name: &str| series.iter().any(|e| e.source == source && e.series == name);
            ["log", "plc", "sorter"].iter().all(|id| s.as_served["health"][id]["state"] == "live")
                && s.as_served["counters"]["lines"] == 40
                && listed("plc", "speed")
                && listed("sorter", "item")
        })),
        "{:?}",
        rt.block_on(peer.snapshot())
    );

    rt.block_on(async {
        // The snapshot, typed: what runs, its clocks, its switch and its sources.
        let s = peer.snapshot().await.unwrap();
        assert_eq!((s.session_name.as_str(), s.is_running, s.writes_armed, s.start_ms, s.window_ms), ("plant", true, false, Some(midnight().and_utc().timestamp_millis()), None));
        assert_eq!(
            s.sources.iter().map(|x| (x.id.as_str(), x.kind.as_str(), x.has_timestamp)).collect::<Vec<_>>(),
            [("log", "log", true), ("plc", "opc", false), ("sorter", "channel", false)]
        );
        assert!(s.started_ms > 0 && s.store_now_ms > 0 && s.arrival_now_ms >= s.started_ms, "{s:?}");
        // Each clock the one the peer named: the store's axis and the arrival, the latter this
        // machine's Unix clock within a few seconds.
        assert_eq!((s.store_now_ms, s.arrival_now_ms), (s.as_served["now_ms"].as_i64().unwrap(), s.as_served["arrival_now_ms"].as_i64().unwrap()));
        assert!((veredas_core::now_unix_ms() - s.arrival_now_ms).abs() < 5_000, "{s:?}");

        // The listing: the log's temperature typed, counted, keyed and last, sorted as the peer sorts.
        let series = peer.series().await.unwrap();
        let listed = series.iter().find(|e| e.source == "log" && e.series == "temp").unwrap();
        assert_eq!((listed.ty.as_str(), listed.count, listed.keys, listed.keys_capped, listed.last_ms), ("float", 40, 3, false, Some(stamp_ms(39))));
        let order: Vec<(&str, &str)> = series.iter().map(|e| (e.source.as_str(), e.series.as_str())).collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted);

        // A window downsampled to ten with its endpoints, a narrower one whole, a key, the budget.
        let w = peer.series_window(&SeriesWindow { source: "log", series: "temp", points: Some(10), ..Default::default() }).await.unwrap();
        assert_eq!((w.total, w.downsampled, w.points.len()), (40, true, 10));
        assert_eq!((w.points[0], w.points[9]), ((stamp_ms(0), Some(temp(0))), (stamp_ms(39), Some(temp(39)))));
        let w = peer
            .series_window(&SeriesWindow { source: "log", series: "temp", from_ms: Some(stamp_ms(10)), to_ms: Some(stamp_ms(19)), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(w.points, (10..20).map(|i| (stamp_ms(i), Some(temp(i)))).collect::<Vec<_>>());
        assert!(!w.downsampled);
        let w = peer.series_window(&SeriesWindow { source: "log", series: "temp", key: Some("A1"), ..Default::default() }).await.unwrap();
        assert_eq!(w.total, 13, "items 1, 4, ..., 37");
        let w = peer.series_window(&SeriesWindow { source: "log", series: "temp", points: Some(3), ..Default::default() }).await.unwrap();
        assert_eq!(w.points.len(), 10, "the peer clamps the budget to ten");
        let w = peer.series_window(&SeriesWindow { source: "log", series: "nothing", ..Default::default() }).await.unwrap();
        assert_eq!((w.total, w.points.len()), (0, 0));

        // The catalog, a page, a view's page, a column charted, a table that is not there.
        let tables = peer.tables().await.unwrap();
        let kinds: Vec<(&str, &str)> = tables.iter().map(|t| (t.name.as_str(), t.kind.as_str())).collect();
        for want in [("readings", "base"), ("plc_main", "base"), ("announce", "base"), ("samples", "samples"), ("warm", "view")] {
            assert!(kinds.contains(&want), "{want:?} not in {kinds:?}");
        }
        let readings = tables.iter().find(|t| t.name == "readings").unwrap();
        assert_eq!(readings.rows, Some(40));
        assert!(readings.columns.iter().any(|c| c.name == "temp" && c.ty == "DOUBLE"), "{readings:?}");
        assert_eq!(tables.iter().find(|t| t.name == "warm").unwrap().rows, None);
        let page = peer.table_page(&TablePage { table: "readings", limit: Some(5), ..Default::default() }).await.unwrap();
        assert_eq!((page.total, page.truncated, page.rows.len()), (40, true, 5));
        let temp_at = page.columns.iter().position(|c| c.name == "temp").unwrap();
        assert_eq!(page.rows[0][temp_at], json!(temp(39)), "newest first");
        let page = peer.table_page(&TablePage { table: "readings", limit: Some(5), offset: Some(35), ..Default::default() }).await.unwrap();
        assert_eq!((page.truncated, page.rows.len(), page.rows[4][temp_at].clone()), (false, 5, json!(temp(0))));
        let page = peer.table_page(&TablePage { table: "readings", from_ms: Some(stamp_ms(10)), to_ms: Some(stamp_ms(19)), limit: Some(100), ..Default::default() }).await.unwrap();
        assert_eq!((page.total, page.truncated, page.rows[0][temp_at].clone(), page.rows[9][temp_at].clone()), (10, false, json!(temp(19)), json!(temp(10))), "a window of the time column, newest first");
        let page = peer.table_page(&TablePage { table: "warm", limit: Some(100), ..Default::default() }).await.unwrap();
        assert_eq!((page.total, page.truncated), (19, false));
        let chart = peer.table_chart(&TableChart { table: "readings", y: "temp", points: Some(10), ..Default::default() }).await.unwrap();
        assert_eq!((chart.total, chart.points.len(), chart.points[9]), (40, 10, (stamp_ms(39), Some(temp(39)))));
        let chart = peer.table_chart(&TableChart { table: "readings", y: "temp", from_ms: Some(stamp_ms(10)), to_ms: Some(stamp_ms(19)), ..Default::default() }).await.unwrap();
        assert_eq!(chart.points, (10..20).map(|i| (stamp_ms(i), Some(temp(i)))).collect::<Vec<_>>(), "a chart's window");
        let chart = peer.table_chart(&TableChart { table: "readings", y: "temp", key: Some("A1"), ..Default::default() }).await.unwrap();
        assert_eq!(chart.total, 13, "a chart's key: items 1, 4, ..., 37");
        match peer.table_page(&TablePage { table: "nope", ..Default::default() }).await {
            Err(PeerError::Status { code, text }) => assert_eq!((code, text.as_str()), (404, "unknown table `nope`")),
            other => panic!("{other:?}"),
        }
        match peer.table_chart(&TableChart { table: "readings", y: "ghost", ..Default::default() }).await {
            Err(PeerError::Status { code, .. }) => assert!(code >= 400, "{code}"),
            other => panic!("{other:?}"),
        }

        // A tag read on demand through the peer (its node id carries `=` and `;`), and one that
        // is not in its session, refused by the peer's own text.
        let word = node(&bench, "word");
        let read = peer.tag_read("plc", &word).await.unwrap();
        assert_eq!((read.source.as_str(), read.tag.as_str(), read.good, read.value.as_deref()), ("plc", word.as_str(), true, Some("5")));
        let ghost = node(&bench, "ghost");
        match peer.tag_read("plc", &ghost).await {
            Err(PeerError::Status { code, text }) => assert_eq!((code, text), (404, format!("`plc` / `{ghost}` is not a tag in this session"))),
            other => panic!("{other:?}"),
        }
    });
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    // Stopped, the peer says so.
    let s = rt.block_on(peer.snapshot()).unwrap();
    assert!(!s.is_running);
}

#[test]
fn a_portal_follows_another_s_stream_its_lag_its_end_and_a_fresh_hello() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("stream.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = SessionConfig::new("stream");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let peer = PeerClient::new(server.addr.to_string());
    let rt = reader();
    let mut events = rt.block_on(peer.event_stream()).unwrap();

    // The hello first, then the samples the plant writes, with their exact stamps and values.
    match rt.block_on(events.next_event()).unwrap() {
        PeerEvent::Hello(s) => assert_eq!((s.session_name.as_str(), s.is_running), ("stream", true)),
        other => panic!("{other:?}"),
    }
    write_lines(&log, 0..5);
    let mut got = Vec::new();
    while got.len() < 5 {
        match rt.block_on(events.next_event()).unwrap() {
            PeerEvent::Data(samples) => got.extend(samples.into_iter().filter(|s| s.source == "log" && s.series == "temp").map(|s| (s.index_ms, s.value, s.count, s.key))),
            other => panic!("{other:?}"),
        }
    }
    // Each sample keyed by the line's item, as the rule keys it.
    assert_eq!(got, (0..5).map(|i| (stamp_ms(i), Some(temp(i)), i as u64 + 1, Some(format!("A{}", i % 3)))).collect::<Vec<_>>());

    // A reader that does not read lags: more frames than the peer's channel holds, each larger
    // than the socket buffers absorb, pushed before it reads one. The frames it then reads and
    // the counts its lag events carry are the four hundred sent, the marker after them read.
    let big = format!("{{\"type\":\"data\",\"samples\":[],\"pad\":\"{}\"}}", "x".repeat(65_536));
    for _ in 0..400 {
        let _ = live.frames.send(Arc::from(big.as_str()));
    }
    let marker = json!({ "type": "data", "samples": [{ "src": "marker", "ser": "m", "k": null, "ms": 1, "v": 1.0, "n": 1 }] }).to_string();
    let _ = live.frames.send(Arc::from(marker.as_str()));
    let (mut pads, mut dropped, mut lags) = (0u64, 0u64, 0);
    loop {
        match rt.block_on(events.next_event()).unwrap() {
            PeerEvent::Data(s) if s.is_empty() => pads += 1,
            PeerEvent::Data(s) if s[0].source == "marker" => break,
            PeerEvent::Lag(n) => {
                lags += 1;
                dropped += n;
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(lags >= 1, "a lag event; {pads} frames read");
    assert_eq!(pads + dropped, 400, "{pads} read, {dropped} dropped in {lags} lag events");

    // The serving portal gone, the stream ends, and stays ended.
    drop(server);
    let end = rt.block_on(events.next_event());
    assert!(matches!(end, Err(PeerError::Closed) | Err(PeerError::Link { .. })), "{end:?}");
    assert!(matches!(rt.block_on(events.next_event()), Err(PeerError::Closed)));
    // Served again, a fresh stream opens with its hello.
    let server = serve(Arc::clone(&live));
    let peer = PeerClient::new(server.addr.to_string());
    let mut events = rt.block_on(peer.event_stream()).unwrap();
    assert!(matches!(rt.block_on(events.next_event()), Ok(PeerEvent::Hello(s)) if s.session_name == "stream"));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The request as the peer reads it: the path and its query, every value percent-encoded, the
/// peer's address as the host, JSON asked for.
#[test]
fn the_request_names_the_peer_and_encodes_its_values() {
    let (addr, heads) = fake_peer(|s| {
        let body = r#"{"points":[[1,2.5]],"total":1,"downsampled":false}"#;
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    let peer = PeerClient::new(addr.to_string());
    let w = reader()
        .block_on(peer.series_window(&SeriesWindow { source: "plc main", series: "ns=2;s=a&b", key: Some("ç"), from_ms: Some(-5), to_ms: None, points: Some(10) }))
        .unwrap();
    assert_eq!(w, PeerWindow { points: vec![(1, Some(2.5))], total: 1, downsampled: false });
    let head = heads.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.starts_with("get /api/series/data?source=plc%20main&series=ns%3d2%3bs%3da%26b&key=%c3%a7&from=-5&points=10 http/1.1\r\n"), "{head}");
    assert!(head.contains(&format!("\r\nhost: {addr}\r\n")) && head.contains("\r\naccept: application/json\r\n") && head.contains("\r\nconnection: close\r\n"), "{head}");
}

/// An answer read by HTTP/1.1's framing as a peer may send it: chunks split anywhere, an extension
/// on every size line and a trailer after the last; no length and no chunks, to the connection's
/// end; and refused when the framing breaks, by what broke.
#[test]
fn an_answer_is_read_by_its_framing() {
    let rt = reader();
    let body = r#"{"points":[[1,2.5],[2,3.5]],"total":2,"downsampled":false}"#;
    let want = PeerWindow { points: vec![(1, Some(2.5)), (2, Some(3.5))], total: 2, downsampled: false };
    let w = SeriesWindow { source: "s", series: "t", ..Default::default() };
    let (addr, _) = fake_peer(move |s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n");
        for c in body.as_bytes().chunks(7) {
            let _ = write!(s, "{:x};ext=1\r\n", c.len());
            let _ = s.flush();
            std::thread::sleep(Duration::from_millis(2));
            let _ = s.write_all(c);
            let _ = s.write_all(b"\r\n");
        }
        let _ = s.write_all(b"0\r\nX-Trailer: 1\r\n\r\n");
    });
    assert_eq!(rt.block_on(PeerClient::new(addr.to_string()).series_window(&w)).unwrap(), want);
    let (addr, _) = fake_peer(move |s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\n\r\n{body}");
    });
    assert_eq!(rt.block_on(PeerClient::new(addr.to_string()).series_window(&w)).unwrap(), want);
    let broken: [(&'static str, &str); 3] = [
        ("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n", "a chunk's size `zz`"),
        ("SMTP ready\r\n\r\n", "the status line `SMTP ready`"),
        ("HTTP/1.1 200 OK\r\nContent-Length: ten\r\n\r\n", "a content length `ten`"),
    ];
    for (answer, why) in broken {
        let (addr, _) = fake_peer(move |s| {
            let _ = s.write_all(answer.as_bytes());
        });
        match rt.block_on(PeerClient::new(addr.to_string()).series_window(&w)) {
            Err(PeerError::Shape(text)) => assert_eq!(text, why),
            other => panic!("{answer:?}: {other:?}"),
        }
    }
    let (addr, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"points\"");
    });
    match rt.block_on(PeerClient::new(addr.to_string()).series_window(&w)) {
        Err(PeerError::Link { text, .. }) => assert_eq!(text, "the connection closed 91 bytes short of the answer's end"),
        other => panic!("{other:?}"),
    }
    let (addr, _) = fake_peer(|s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nX-Long: {}\r\n\r\n", "a".repeat(HEAD_LIMIT + 10));
    });
    assert!(matches!(rt.block_on(PeerClient::new(addr.to_string()).series_window(&w)), Err(PeerError::TooLarge { limit: HEAD_LIMIT })));
}

/// What a reader keeps from a peer: an answer within its time, a stream that speaks, an answer of
/// a bounded size in the shape asked, a refusal in the peer's own words; a peer not there named.
#[test]
fn a_reader_is_bounded_in_time_size_and_shape() {
    let rt = reader();
    // Nobody listening: the address named.
    let held = held_port();
    let addr = format!("127.0.0.1:{}", held.port);
    match rt.block_on(PeerClient::new(addr.clone()).snapshot()) {
        Err(e @ PeerError::Connect { .. }) => assert!(e.to_string().contains(&addr), "{e}"),
        other => panic!("{other:?}"),
    }
    // A peer that takes the request and never answers: the call's time, not longer.
    let (addr, _) = fake_peer(|_| std::thread::sleep(Duration::from_secs(20)));
    let started = Instant::now();
    let r = rt.block_on(PeerClient::new(addr.to_string()).with_timeout(Duration::from_millis(500)).series());
    assert!(matches!(&r, Err(e @ PeerError::Timeout { ms: 500, .. }) if e.to_string().contains(&addr.to_string())), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    // A stream that opens and then says nothing, not even a comment: dead after its silence.
    let (addr, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n: hi\n\r\n");
        std::thread::sleep(Duration::from_secs(20));
    });
    let mut events = rt.block_on(PeerClient::new(addr.to_string()).event_stream()).unwrap().with_silence(Duration::from_millis(800));
    let started = Instant::now();
    let r = rt.block_on(events.next_event());
    assert!(matches!(r, Err(PeerError::Silent { ms: 800 })), "{r:?}");
    assert!(started.elapsed() >= Duration::from_millis(700) && started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(matches!(rt.block_on(events.next_event()), Err(PeerError::Closed)));
    // An answer past the limit, refused before it is whole.
    let (addr, _) = fake_peer(|s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", ANSWER_LIMIT + 1);
        let block = vec![b' '; 1 << 20];
        for _ in 0..=(ANSWER_LIMIT >> 20) {
            if s.write_all(&block).is_err() {
                return;
            }
        }
    });
    let r = rt.block_on(PeerClient::new(addr.to_string()).tables());
    assert!(matches!(r, Err(PeerError::TooLarge { limit: ANSWER_LIMIT })), "{r:?}");
    // Not in the shape: no JSON, or JSON without what was asked.
    for (body, want) in [("not json", "/api/series"), ("{\"tables\": 3}", "no `series`")] {
        let (addr, _) = fake_peer(move |s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        });
        match rt.block_on(PeerClient::new(addr.to_string()).series()) {
            Err(PeerError::Shape(text)) => assert!(text.contains(want), "{text}"),
            other => panic!("{other:?}"),
        }
    }
    // A refusal that is not the peer's JSON form is still its words, with its status; a stream
    // answered as JSON is no stream.
    let (addr, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\nbusy");
    });
    match rt.block_on(PeerClient::new(addr.to_string()).snapshot()) {
        Err(PeerError::Status { code, text }) => assert_eq!((code, text.as_str()), (503, "busy")),
        other => panic!("{other:?}"),
    }
    let (addr, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
    });
    assert!(matches!(rt.block_on(PeerClient::new(addr.to_string()).event_stream()), Err(PeerError::Shape(_))));
}

/// A value that is no finite number (NaN, an infinity) is served as `null`: read as none, the
/// stream going on and the window whole, the finite samples beside it kept.
#[test]
fn a_value_that_is_no_finite_number_is_read_as_none_and_the_stream_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("faults.log");
    std::fs::write(&log, "").unwrap();
    let mut rule = readings_rule(None);
    rule.pattern = r"temp=(?P<temp>\S+) item=(?P<item>\w+) state=(?P<state>\w+)".into();
    let mut cfg = SessionConfig::new("faults");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(rule);
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let server = serve(Arc::clone(&live));
    let peer = PeerClient::new(server.addr.to_string());
    let rt = reader();
    let mut events = rt.block_on(peer.event_stream()).unwrap();
    assert!(matches!(rt.block_on(events.next_event()), Ok(PeerEvent::Hello(_))));
    let values = ["20.5", "NaN", "21.0", "inf", "22.0"];
    let text: String = values.iter().enumerate().map(|(i, v)| format!("2026-07-19 10:00:{i:02} temp={v} item=A1 state=run\n")).collect();
    std::fs::write(&log, text).unwrap();
    let mut got = Vec::new();
    while got.len() < 5 {
        match rt.block_on(events.next_event()) {
            Ok(PeerEvent::Data(samples)) => got.extend(samples.into_iter().filter(|s| s.series == "temp").map(|s| s.value)),
            other => panic!("{other:?} after {got:?}"),
        }
    }
    assert_eq!(got, vec![Some(20.5), None, Some(21.0), None, Some(22.0)]);
    // The stream goes on: a finite sample after them.
    std::io::Write::write_all(&mut std::fs::OpenOptions::new().append(true).open(&log).unwrap(), b"2026-07-19 10:00:09 temp=23.5 item=A1 state=run\n").unwrap();
    match rt.block_on(events.next_event()) {
        Ok(PeerEvent::Data(samples)) => assert_eq!(samples.iter().map(|s| s.value).collect::<Vec<_>>(), vec![Some(23.5)]),
        other => panic!("{other:?}"),
    }
    let w = rt.block_on(peer.series_window(&SeriesWindow { source: "log", series: "temp", ..Default::default() })).unwrap();
    assert_eq!(w.points.iter().map(|(_, v)| *v).collect::<Vec<_>>(), vec![Some(20.5), None, Some(21.0), None, Some(22.0), Some(23.5)]);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The defaults: a peer that never answers holds a call ten seconds, a stream silent fifteen,
/// and a real portal's idle stream, kept alive by its comments every five, stays open past both.
#[test]
fn the_defaults_hold_a_dead_link_ten_seconds_and_keep_an_idle_stream_open() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("idle.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = SessionConfig::new("idle");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let server = serve(session.live());
    let (dead, _) = fake_peer(|_| std::thread::sleep(Duration::from_secs(30)));
    let (mute, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n: hi\n\r\n");
        std::thread::sleep(Duration::from_secs(30));
    });
    let rt = reader();
    rt.block_on(async {
        let call = async {
            let started = Instant::now();
            let r = PeerClient::new(dead.to_string()).series().await;
            (r, started.elapsed())
        };
        let silent = async {
            let mut events = PeerClient::new(mute.to_string()).event_stream().await.unwrap();
            let started = Instant::now();
            (events.next_event().await, started.elapsed())
        };
        let idle = async {
            let mut events = PeerClient::new(server.addr.to_string()).event_stream().await.unwrap();
            assert!(matches!(events.next_event().await, Ok(PeerEvent::Hello(_))));
            tokio::time::timeout(Duration::from_secs(17), events.next_event()).await
        };
        let ((r, took), (silence, waited), idle) = tokio::join!(call, silent, idle);
        assert!(matches!(r, Err(PeerError::Timeout { ms: 10_000, .. })), "{r:?}");
        assert!(took >= Duration::from_millis(9_900) && took < Duration::from_secs(14), "{took:?}");
        assert!(matches!(silence, Err(PeerError::Silent { ms: 15_000 })), "{silence:?}");
        assert!(waited >= Duration::from_millis(14_900) && waited < Duration::from_secs(19), "{waited:?}");
        assert!(idle.is_err(), "the idle stream still open after 17 s, nothing come: {idle:?}");
    });
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The size bounds whatever the framing: an answer past its limit in chunks or to the
/// connection's end, a head of headers past its limit together; and a snapshot missing a key the
/// interface always writes, refused.
#[test]
fn every_bound_holds_whatever_the_framing() {
    let rt = reader();
    let big = |chunked: bool| {
        fake_peer(move |s| {
            let head: &[u8] = if chunked { b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n" } else { b"HTTP/1.1 200 OK\r\n\r\n" };
            let _ = s.write_all(head);
            let block = vec![b' '; 1 << 20];
            for _ in 0..=(ANSWER_LIMIT >> 20) {
                let sent = if chunked { write!(s, "{:x}\r\n", block.len()).and_then(|()| s.write_all(&block)).and_then(|()| s.write_all(b"\r\n")) } else { s.write_all(&block) };
                if sent.is_err() {
                    return;
                }
            }
        })
        .0
    };
    for chunked in [true, false] {
        let r = rt.block_on(PeerClient::new(big(chunked).to_string()).tables());
        assert!(matches!(r, Err(PeerError::TooLarge { limit: ANSWER_LIMIT })), "chunked {chunked}: {r:?}");
    }
    let (addr, _) = fake_peer(|s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nX-One: {}\r\nX-Two: {}\r\n\r\n", "a".repeat(40 << 10), "b".repeat(40 << 10));
    });
    assert!(matches!(rt.block_on(PeerClient::new(addr.to_string()).tables()), Err(PeerError::TooLarge { limit: HEAD_LIMIT })));
    let (addr, _) = fake_peer(|s| {
        let body = r#"{"session_name":"x","running":true,"started_ms":1,"start_ms":null,"now_ms":2,"arrival_now_ms":3,"writes_armed":false,"sources":[]}"#;
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    match rt.block_on(PeerClient::new(addr.to_string()).snapshot()) {
        Err(PeerError::Shape(text)) => assert!(text.contains("missing field `window_ms`"), "{text}"),
        other => panic!("{other:?}"),
    }
}

/// A stream read through a caller's timeouts loses nothing: a chunk's line end held back, then
/// the next chunk's size line cut in two, each across several dropped calls; the events come in
/// order, then the end, never a refusal of the peer's framing.
#[test]
fn a_stream_read_through_a_caller_s_timeouts_loses_nothing() {
    let event = |ms: i64| format!("event: data\ndata: {{\"samples\":[{{\"src\":\"s\",\"ser\":\"t\",\"k\":null,\"ms\":{ms},\"v\":1.5,\"n\":{ms}}}]}}\n\n");
    let (one, two) = (event(1), event(2));
    let (addr, _) = fake_peer(move |s| {
        let size = format!("{:x}", two.len());
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{one}", one.len());
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(700));
        let _ = write!(s, "\r\n{}", &size[..1]);
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(700));
        let _ = write!(s, "{}\r\n{two}\r\n0\r\n\r\n", &size[1..]);
    });
    let rt = reader();
    let mut events = rt.block_on(PeerClient::new(addr.to_string()).event_stream()).unwrap();
    let (mut got, mut dropped) = (Vec::new(), 0);
    let end = loop {
        match rt.block_on(async { tokio::time::timeout(Duration::from_millis(300), events.next_event()).await }) {
            Err(_) => dropped += 1,
            Ok(Ok(PeerEvent::Data(samples))) => got.extend(samples.iter().map(|s| s.index_ms)),
            Ok(other) => break other,
        }
        assert!(dropped < 40, "the stream never came: {got:?}");
    };
    assert_eq!(got, vec![1, 2]);
    assert!(dropped >= 2, "calls dropped while bytes were held back: {dropped}");
    assert!(matches!(end, Err(PeerError::Closed)), "{end:?}");
}

/// The stream's opening is bounded by the call's time, and its refusal is the peer's own words;
/// every key the interface always writes, missing, is refused by name, in a data event as in each
/// typed answer.
#[test]
fn the_stream_s_opening_is_bounded_and_every_key_is_required() {
    let rt = reader();
    let (dead, _) = fake_peer(|_| std::thread::sleep(Duration::from_secs(20)));
    let started = Instant::now();
    let r = rt.block_on(PeerClient::new(dead.to_string()).with_timeout(Duration::from_millis(500)).event_stream());
    assert!(matches!(&r, Err(e @ PeerError::Timeout { ms: 500, .. }) if e.to_string().contains(&dead.to_string())), "{:?}", r.as_ref().err());
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    let (busy, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\n{\"error\":\"busy\"}");
    });
    match rt.block_on(PeerClient::new(busy.to_string()).event_stream()) {
        Err(PeerError::Status { code, text }) => assert_eq!((code, text.as_str()), (503, "busy")),
        other => panic!("{:?}", other.err()),
    }
    // A data event's sample without its value, or its key.
    for (sample, field) in [(r#"{"src":"s","ser":"t","k":null,"ms":1,"n":1}"#, "v"), (r#"{"src":"s","ser":"t","ms":1,"v":1.5,"n":1}"#, "k")] {
        let body = format!("event: data\ndata: {{\"samples\":[{sample}]}}\n\n");
        let (addr, _) = fake_peer(move |s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        });
        let mut events = rt.block_on(PeerClient::new(addr.to_string()).event_stream()).unwrap();
        match rt.block_on(events.next_event()) {
            Err(PeerError::Shape(text)) => assert!(text.contains(&format!("missing field `{field}`")), "{text}"),
            other => panic!("{field}: {other:?}"),
        }
    }
    // Each typed answer without one of its always-written keys.
    let answer = |body: &'static str| {
        fake_peer(move |s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        })
        .0
        .to_string()
    };
    let shape = |r: Result<(), PeerError>, field: &str| match r {
        Err(PeerError::Shape(text)) => assert!(text.contains(&format!("missing field `{field}`")), "{text}"),
        other => panic!("{field}: {other:?}"),
    };
    let listing = answer(r#"{"series":[{"source":"s","series":"t","type":"float","count":1,"keys":0,"keys_capped":false,"last_value":"1"}]}"#);
    shape(rt.block_on(PeerClient::new(listing).series()).map(drop), "last_ms");
    let catalog = answer(r#"{"tables":[{"name":"t","kind":"base","columns":[]}]}"#);
    shape(rt.block_on(PeerClient::new(catalog).tables()).map(drop), "rows");
    let tag = answer(r#"{"source":"plc","tag":"x","ok":true,"status":"Good"}"#);
    shape(rt.block_on(PeerClient::new(tag).tag_read("plc", "x")).map(drop), "value");
}

/// A stream's silence runs from the last byte it gave, across every call: read through 300 ms
/// timeouts, a peer that goes quiet is `Silent` about its silence after its last byte, then
/// closed; and a chunked stream's trailers held back across dropped calls still end it cleanly.
#[test]
fn a_silence_and_the_trailers_hold_across_dropped_calls() {
    let rt = reader();
    let read_through = |events: &mut PeerEvents| -> (Vec<i64>, Result<PeerEvent, PeerError>, usize) {
        let (mut got, mut dropped) = (Vec::new(), 0);
        let end = loop {
            match rt.block_on(async { tokio::time::timeout(Duration::from_millis(300), events.next_event()).await }) {
                Err(_) => dropped += 1,
                Ok(Ok(PeerEvent::Data(samples))) => got.extend(samples.iter().map(|s| s.index_ms)),
                Ok(other) => break other,
            }
            assert!(dropped < 60, "no end after {dropped} dropped calls: {got:?}");
        };
        (got, end, dropped)
    };
    // A peer that says one thing and goes quiet, the link left open.
    let (quiet, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n: hi\n\r\n");
        std::thread::sleep(Duration::from_secs(20));
    });
    let mut events = rt.block_on(PeerClient::new(quiet.to_string()).event_stream()).unwrap().with_silence(Duration::from_millis(800));
    let started = Instant::now();
    let (got, end, dropped) = read_through(&mut events);
    let took = started.elapsed();
    assert!(matches!(end, Err(PeerError::Silent { ms: 800 })), "{end:?}");
    assert!(got.is_empty() && dropped >= 2, "{got:?} {dropped}");
    assert!(took >= Duration::from_millis(700) && took < Duration::from_millis(1_600), "silent after {took:?}");
    assert!(matches!(rt.block_on(events.next_event()), Err(PeerError::Closed)));
    // The last chunk's trailers, their blank line held back 700 ms.
    let event = "event: data\ndata: {\"samples\":[{\"src\":\"s\",\"ser\":\"t\",\"k\":null,\"ms\":1,\"v\":1.5,\"n\":1}]}\n\n";
    let (trailing, _) = fake_peer(move |s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n0\r\nX-Trailer: 1\r\n", event.len());
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(700));
        let _ = s.write_all(b"\r\n");
    });
    let mut events = rt.block_on(PeerClient::new(trailing.to_string()).event_stream()).unwrap();
    let (got, end, dropped) = read_through(&mut events);
    assert_eq!(got, vec![1]);
    assert!(dropped >= 1, "a call dropped while the trailers were read");
    assert!(matches!(end, Err(PeerError::Closed)), "{end:?}");
    // A snapshot without `start_ms`, a key the interface always writes: refused by its name.
    let (addr, _) = fake_peer(|s| {
        let body = r#"{"session_name":"x","running":true,"started_ms":1,"window_ms":null,"now_ms":2,"arrival_now_ms":3,"writes_armed":false,"sources":[]}"#;
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    match rt.block_on(PeerClient::new(addr.to_string()).snapshot()) {
        Err(PeerError::Shape(text)) => assert!(text.contains("missing field `start_ms`"), "{text}"),
        other => panic!("{other:?}"),
    }
}

/// Round 4's four: a call dropped inside a chunk's data loses none of it; a chunked answer cut
/// short is a broken link, before its last chunk, inside a chunk and between a stream's chunks; a
/// body that stalls after its head is a timeout naming the address; and the snapshot's clocks,
/// window and switch read as served, each a value of its own.
#[test]
fn a_chunk_split_across_calls_a_cut_answer_a_stalled_body_and_every_snapshot_field() {
    let rt = reader();
    let event = "event: data\ndata: {\"samples\":[{\"src\":\"s\",\"ser\":\"t\",\"k\":null,\"ms\":1,\"v\":1.5,\"n\":1}]}\n\n";
    // A data event cut inside its chunk, its second half 700 ms later, read through 300 ms timeouts.
    let (split, _) = fake_peer(move |s| {
        let half = event.len() / 2;
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}", event.len(), &event[..half]);
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(700));
        let _ = write!(s, "{}\r\n0\r\n\r\n", &event[half..]);
    });
    let mut events = rt.block_on(PeerClient::new(split.to_string()).event_stream()).unwrap();
    let (mut got, mut dropped) = (Vec::new(), 0);
    let end = loop {
        match rt.block_on(async { tokio::time::timeout(Duration::from_millis(300), events.next_event()).await }) {
            Err(_) => dropped += 1,
            Ok(Ok(PeerEvent::Data(samples))) => got.extend(samples.iter().map(|s| s.index_ms)),
            Ok(other) => break other,
        }
        assert!(dropped < 40, "the stream never came: {got:?}");
    };
    assert_eq!(got, vec![1]);
    assert!(dropped >= 1, "a call dropped inside the chunk's data: {dropped}");
    assert!(matches!(end, Err(PeerError::Closed)), "{end:?}");
    // A chunked answer cut short: without its last chunk, and inside a chunk.
    let window = r#"{"points":[[1,2.5]],"total":1,"downsampled":false}"#;
    let cut = |tail: String| {
        fake_peer(move |s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{tail}");
        })
        .0
        .to_string()
    };
    let w = SeriesWindow { source: "s", series: "t", ..Default::default() };
    for (tail, said) in [(format!("{:x}\r\n{window}\r\n", window.len()), "closed before the answer's end"), (format!("40\r\n{}", &window[..9]), "closed 55 bytes short")] {
        match rt.block_on(PeerClient::new(cut(tail.clone())).series_window(&w)) {
            Err(PeerError::Link { text, .. }) => assert!(text.contains(said), "{tail:?}: {text}"),
            other => panic!("{tail:?}: {other:?}"),
        }
    }
    // A stream cut between its chunks: its event, then a broken link, not a clean end.
    let (cut_stream, _) = fake_peer(move |s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n", event.len());
    });
    let mut events = rt.block_on(PeerClient::new(cut_stream.to_string()).event_stream()).unwrap();
    assert!(matches!(rt.block_on(events.next_event()), Ok(PeerEvent::Data(_))));
    let end = rt.block_on(events.next_event());
    assert!(matches!(end, Err(PeerError::Link { .. })), "{end:?}");
    // A body that stalls after its head, the link left open.
    let (stalled, _) = fake_peer(|s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"series\"");
        let _ = s.flush();
        std::thread::sleep(Duration::from_secs(20));
    });
    let started = Instant::now();
    let r = rt.block_on(PeerClient::new(stalled.to_string()).with_timeout(Duration::from_millis(500)).series());
    assert!(matches!(&r, Err(e @ PeerError::Timeout { ms: 500, .. }) if e.to_string().contains(&stalled.to_string())), "{:?}", r.as_ref().err());
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    // Every clock, the window and the switch as served, none a default.
    let (full, _) = fake_peer(|s| {
        let body = r#"{"session_name":"x","running":true,"started_ms":111,"start_ms":222,"window_ms":60000,"now_ms":333,"arrival_now_ms":444,"writes_armed":true,"sources":[]}"#;
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    let s = rt.block_on(PeerClient::new(full.to_string()).snapshot()).unwrap();
    assert_eq!((s.started_ms, s.start_ms, s.window_ms, s.store_now_ms, s.arrival_now_ms, s.writes_armed), (111, Some(222), Some(60_000), 333, 444, true));
}

/// Round 5's three: an event of another kind, and an unnamed one, skipped on a stream that goes
/// on; a sample's key read as served; and a chunk longer than its size line says refused by its
/// shape, never read as an answer.
#[test]
fn an_unknown_event_is_skipped_a_key_read_and_a_chunk_overrun_refused() {
    let rt = reader();
    let sample = |ms: i64, key: &str| format!("{{\"samples\":[{{\"src\":\"s\",\"ser\":\"t\",\"k\":{key},\"ms\":{ms},\"v\":1.5,\"n\":{ms}}}]}}");
    let body = format!(
        "event: data\ndata: {}\n\nevent: fault\ndata: {{\"why\":\"a newer peer's kind\"}}\n\ndata: unnamed\n\nevent: data\ndata: {}\n\n",
        sample(1, "\"A1\""),
        sample(2, "null")
    );
    let (addr, _) = fake_peer(move |s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    let mut events = rt.block_on(PeerClient::new(addr.to_string()).event_stream()).unwrap();
    let mut got = Vec::new();
    let end = loop {
        match rt.block_on(events.next_event()) {
            Ok(PeerEvent::Data(samples)) => got.extend(samples.into_iter().map(|s| (s.index_ms, s.key))),
            other => break other,
        }
    };
    assert_eq!(got, vec![(1, Some("A1".to_string())), (2, None)]);
    assert!(matches!(end, Err(PeerError::Closed)), "{end:?}");
    // The size line the window's own length: the bytes after it overrun the chunk.
    let window = r#"{"points":[[1,2.5]],"total":1,"downsampled":false}"#;
    let (addr, _) = fake_peer(move |s| {
        let _ = write!(s, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{window}junk\r\n0\r\n\r\n", window.len());
    });
    match rt.block_on(PeerClient::new(addr.to_string()).series_window(&SeriesWindow { source: "s", series: "t", ..Default::default() })) {
        Err(PeerError::Shape(text)) => assert_eq!(text, "a chunk not ended by its line end"),
        other => panic!("{other:?}"),
    }
}
