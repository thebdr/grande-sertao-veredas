//! The store kept as a window (contract: the window clause, P-019): the rows that arrived within
//! a span, the older let go every second through the ingest's connection, nothing on the drive.
//! On a real in-memory store, then a session over a log the test writes and over the bench's
//! plant (its OPC UA server and its machine over real sockets).

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use common::plant::*;
use serde_json::json;
use veredas_portal::session::{Live, Session, SessionConfig};
use veredas_portal::store::{BaseTableSpec, DuckStore, Residency, RowsQuery, TableDef, WideRow};
use veredas_portal::{Sample, SeriesRef, SeriesType, SourceId, Value};

fn row(source: &str, arrival_ms: i64, seq: u64, v: f64) -> WideRow {
    WideRow { source: SourceId::new(source), key: None, index_ms: 1_000_000 - arrival_ms, arrival_ms, seq, values: vec![Some(Value::Float(v))] }
}

fn total(store: &DuckStore, table: &str) -> u64 {
    store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).unwrap().total
}

/// The arrivals a table holds, oldest first.
fn arrivals(store: &DuckStore, table: &str) -> Vec<i64> {
    let page = store.query_rows(table, &RowsQuery { limit: 2_000, ..RowsQuery::default() }).unwrap();
    let at = page.columns.iter().position(|c| c.name == "arrival_ms").unwrap();
    let mut out: Vec<i64> = page.rows.iter().map(|r| r[at].as_i64().unwrap()).collect();
    out.sort();
    out
}

#[test]
fn the_store_lets_go_of_every_row_that_arrived_before_the_cutoff_and_keeps_the_rest() {
    let spec = |name: &str| BaseTableSpec { name: name.into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false };
    let store = DuckStore::open(&Residency::Ram, &[spec("a"), spec("b")], &[TableDef { name: "both".into(), sql: "SELECT arrival_ms FROM a UNION ALL SELECT arrival_ms FROM b".into() }]).unwrap();
    assert_eq!(store.evict_before(5_000).unwrap(), 0, "an empty store lets nothing go");
    // Stamps run backwards against arrival: the window follows the arrival clock alone.
    store.append_wide(0, &[row("s", 1_000, 1, 1.0), row("s", 2_000, 2, 2.0), row("s", 3_000, 3, 3.0)]).unwrap();
    store.append_wide(1, &[row("t", 1_500, 1, 4.0), row("t", 2_500, 2, 5.0)]).unwrap();
    let sample = |series: &str, ms: i64| Sample { source: SourceId::new("u"), series: Arc::from(series), key: None, ts: None, seq: 1, arrival_unix_ms: ms, value: Value::Float(6.0) };
    store.append_samples(&[sample("x", 1_000), sample("y", 3_000)]).unwrap();
    // The cutoff is exclusive: a row that arrived at it stays.
    assert_eq!(store.evict_before(2_000).unwrap(), 3, "a's 1 000, b's 1 500 and the sample at 1 000");
    assert_eq!((arrivals(&store, "a"), arrivals(&store, "b")), (vec![2_000, 3_000], vec![2_500]));
    assert_eq!(total(&store, "samples"), 1);
    assert_eq!(total(&store, "both"), 3, "a view reads the window");
    let points = store.query_numeric(&SeriesRef::new("s", "v"), None, i64::MIN, i64::MAX);
    assert_eq!(points.iter().map(|p| p.value).collect::<Vec<_>>(), vec![3.0, 2.0], "the narrow surface reads the window, by stamp");
    assert_eq!(store.evict_before(2_000).unwrap(), 0, "a second pass at the same cutoff lets nothing go");
    assert_eq!(store.evict_before(2_501).unwrap(), 2, "a's 2 000 and b's 2 500");
    assert_eq!(store.evict_before(i64::MAX).unwrap(), 2, "everything left: a's 3 000 and the sample at 3 000");
    assert_eq!((total(&store, "a"), total(&store, "b"), total(&store, "samples")), (0, 0, 0));
    // The store goes on after its window emptied it.
    store.append_wide(0, &[row("s", 9_000, 4, 9.0)]).unwrap();
    assert_eq!(arrivals(&store, "a"), vec![9_000]);
}

fn midnight() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

/// A line the readings rule captures, stamped on `day` of July 2026 at ten plus `i` seconds.
fn line_on(day: u32, i: u32) -> String {
    format!("2026-07-{day:02} 10:00:{:02} temp={}.5 item=A{i} state=run\n", i, 100 + i)
}

fn rows_of(live: &Live, table: &str) -> u64 {
    live.store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).map(|p| p.total).unwrap_or(0)
}

/// The temperatures the readings table holds, ascending.
fn temps(live: &Live) -> Vec<f64> {
    let mut out: Vec<f64> = live.store.query_numeric(&SeriesRef::new("log", "temp"), None, i64::MIN, i64::MAX).iter().map(|p| p.value).collect();
    out.sort_by(f64::total_cmp);
    out
}

/// A session over a log kept as a window of three seconds: what arrived first goes first,
/// whatever its lines' stamps say; a line stamped a day earlier but arriving later stays; the
/// series listing still counts every sample seen (its counts order the frames), the snapshot says
/// the span and the rows let go.
#[test]
fn a_session_kept_as_a_window_lets_go_of_what_arrived_first_whatever_its_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    let first: String = (0..10).map(|i| line_on(19, i)).collect();
    std::fs::write(&log, first).unwrap();
    let mut cfg = SessionConfig::new("window");
    cfg.start = Some(midnight());
    cfg.store_window = Some(Duration::from_secs(3));
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert_eq!(live.window_ms, Some(3_000));
    assert!(wait(Duration::from_secs(10), || rows_of(&live, "readings") == 10), "{:?}", live.counters.read().unwrap());
    let stored = Instant::now();
    let batch1: Vec<f64> = (0..10).map(|i| 100.0 + i as f64 + 0.5).collect();
    assert_eq!(temps(&live), batch1);
    // Two seconds on, ten lines stamped a day earlier arrive: the window keeps them past the first.
    while stored.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let second: String = (20..30).map(|i| line_on(18, i)).collect();
    std::io::Write::write_all(&mut std::fs::OpenOptions::new().append(true).open(&log).unwrap(), second.as_bytes()).unwrap();
    let batch2: Vec<f64> = (20..30).map(|i| 100.0 + i as f64 + 0.5).collect();
    assert!(
        wait(Duration::from_secs(20), || temps(&live) == batch2),
        "the first batch gone, the second whole: {:?} {:?}",
        temps(&live),
        live.counters.read().unwrap()
    );
    assert!(wait(Duration::from_secs(2), || live.counters.read().unwrap().evicted == 10), "{:?}", live.counters.read().unwrap());
    // The listing counts what was seen, the store holds what the window keeps.
    assert!(wait(Duration::from_secs(2), || live.series.read().unwrap().get("log|temp").is_some_and(|s| s.count == 20)));
    // Then the second goes too, and the snapshot says so.
    assert!(wait(Duration::from_secs(20), || rows_of(&live, "readings") == 0), "{:?}", temps(&live));
    assert!(wait(Duration::from_secs(2), || live.counters.read().unwrap().evicted == 20), "{:?}", live.counters.read().unwrap());
    let snap = veredas_portal::serve::snapshot(&live);
    assert_eq!((snap["window_ms"].clone(), snap["counters"]["evicted"].clone(), snap["counters"]["lines"].clone()), (json!(3000), json!(20), json!(20)));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// Against the bench's plant, a window of one second: the server polled every 50 ms and the
/// machine's announcements, every protocol's rows alike, hold no row older than the span and the
/// window's period (and a margin for a loaded machine) behind the newest, while the polls go on
/// and the rows they made are let go; without a window nothing goes.
#[test]
fn every_protocol_s_rows_are_kept_as_a_window_against_the_bench() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = session_config("window", &log, &bench, port);
    cfg.store_window = Some(Duration::from_secs(1));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(
        wait(Duration::from_secs(20), || {
            let c = live.counters.read().unwrap();
            c.polls >= 80 && c.messages >= 1 && c.evicted >= 20
        }),
        "{:?}",
        live.counters.read().unwrap()
    );
    let polls = live.counters.read().unwrap().polls;
    for table in ["plc_main", "announce"] {
        let held = arrivals(&live.store, table);
        if let (Some(first), Some(last)) = (held.first(), held.last()) {
            assert!(last - first <= 1_000 + 1_000 + 1_500, "{table} holds {} ms of arrivals: {held:?}", last - first);
        }
    }
    assert!(rows_of(&live, "plc_main") < polls, "the window let polls go: {} rows held of {polls}", rows_of(&live, "plc_main"));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    // The same plant, no window: every poll stays.
    let session = Session::start(session_config("all", &log, &bench, port)).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(20), || live.counters.read().unwrap().polls >= 40), "{:?}", live.counters.read().unwrap());
    std::thread::sleep(Duration::from_millis(1_500));
    let c = live.counters.read().unwrap().clone();
    assert_eq!(c.evicted, 0);
    assert_eq!(live.window_ms, None);
    assert_eq!(veredas_portal::serve::snapshot(&live)["window_ms"], json!(null));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert_eq!(rows_of(&live, "plc_main"), live.counters.read().unwrap().polls, "every poll a row, none let go");
}
