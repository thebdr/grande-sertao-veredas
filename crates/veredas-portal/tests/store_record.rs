//! Recording to the drive (contract: the recording clause, P-019): every row the store takes also
//! written, in batches, to a database file of the session's own, until the file reaches its size
//! limit; then the recording stops and nothing is deleted. Against the bench's plant and a log the
//! test writes, the record read back by a plain connection.

mod common;

use std::path::Path;
use std::time::Duration;

use chrono::NaiveDate;
use common::plant::*;
use serde_json::json;
use veredas_portal::session::{Live, RecordingConfig, RecordingStatus, Session, SessionConfig};
use veredas_portal::store::RowsQuery;

fn rows_of(live: &Live, table: &str) -> u64 {
    live.store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).map(|p| p.total).unwrap_or(0)
}

fn status(live: &Live) -> RecordingStatus {
    live.recording_status.read().unwrap().clone().expect("a recording")
}

/// A table's rows in a record, read by a plain connection once the session is gone.
fn recorded(file: &Path, table: &str) -> i64 {
    let conn = duckdb::Connection::open(file).unwrap();
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get(0)).unwrap()
}

fn midnight() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

/// Against the bench's plant, a store kept as a window of one second and a record written every
/// second: the polls and the announcements the window lets go are on the drive, every one, the
/// record a database any tool reads, with the project's views in it.
#[test]
fn the_record_keeps_on_the_drive_what_the_window_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let file = dir.path().join("records").join("plant.record.duckdb");
    let mut cfg = session_config("plant", &log, &bench, port);
    cfg.store_window = Some(Duration::from_secs(1));
    cfg.recording = Some(RecordingConfig { file: file.clone(), limit_bytes: 64 << 20, write_every: Duration::from_secs(1) });
    cfg.tables.push(veredas_portal::store::TableDef { name: "fast".into(), sql: "SELECT * FROM plc_main WHERE speed > 1.5".into() });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(file.exists(), "the record's folder created and its file opened at the start");
    assert!(
        wait(Duration::from_secs(20), || {
            let c = live.counters.read().unwrap();
            c.polls >= 60 && c.messages >= 1 && c.evicted >= 20 && status(&live).recorded_rows >= 40
        }),
        "{:?} {:?}",
        live.counters.read().unwrap(),
        status(&live)
    );
    let snap = veredas_portal::serve::snapshot(&live);
    assert_eq!(snap["recording"]["file"], json!(file.display().to_string()));
    assert_eq!(snap["recording"]["limit_bytes"], json!(64u64 << 20));
    assert_eq!(snap["recording"]["stopped_why"], json!(null));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    // The stop wrote what was left: every row the store took is on the drive.
    let c = live.counters.read().unwrap().clone();
    let st = status(&live);
    assert_eq!((st.recorded_rows, st.unrecorded_rows, st.stopped_why.clone()), (c.rows, 0, None), "{st:?}");
    assert!(st.written_bytes > 0 && st.written_bytes < st.limit_bytes, "{st:?}");
    assert!(rows_of(&live, "plc_main") < c.polls, "the window let polls go");
    assert_eq!(recorded(&file, "plc_main") as u64, c.polls, "every poll on the drive");
    assert!(recorded(&file, "announce") >= 1);
    assert_eq!(recorded(&file, "fast"), {
        let conn = duckdb::Connection::open(&file).unwrap();
        conn.query_row("SELECT count(*) FROM plc_main WHERE speed > 1.5", [], |r| r.get::<_, i64>(0)).unwrap()
    }, "the project's view, in the record as a view");
    assert_eq!(veredas_portal::serve::snapshot(&live)["recording"]["recorded_rows"], json!(c.rows));
}

/// At its size limit a recording stops: what it wrote stays, nothing is deleted, the rows after
/// are counted and never written, the store goes on taking them, and the session says why. A
/// record already standing at its file refuses the session by name, as a store does.
#[test]
fn at_its_size_limit_a_recording_stops_and_nothing_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..20_000);
    let file = dir.path().join("small.record.duckdb");
    let mut cfg = SessionConfig::new("small");
    cfg.start = Some(midnight());
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.recording = Some(RecordingConfig { file: file.clone(), limit_bytes: 200 * 1024, write_every: Duration::from_secs(1) });
    let session = Session::start(cfg.clone()).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(30), || status(&live).stopped_why.is_some()), "{:?}", status(&live));
    let at_stop = status(&live);
    assert!(at_stop.stopped_why.as_deref().unwrap().starts_with("the record reached its size limit ("), "{at_stop:?}");
    assert!(at_stop.stopped_why.as_deref().unwrap().ends_with(&format!(" of {} bytes)", 200 * 1024)), "{at_stop:?}");
    assert!(at_stop.written_bytes >= at_stop.limit_bytes, "{at_stop:?}");
    assert!(at_stop.recorded_rows > 0);
    // Fifty lines after the stop: stored, counted unrecorded, never written.
    write_lines(&log, 20_000..20_050);
    assert!(wait(Duration::from_secs(10), || rows_of(&live, "readings") == 20_050), "{}", rows_of(&live, "readings"));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let end = status(&live);
    assert_eq!(end.recorded_rows, at_stop.recorded_rows, "nothing written after the stop");
    assert_eq!(end.recorded_rows + end.unrecorded_rows, 20_050, "{end:?}");
    assert_eq!(end.stopped_why, at_stop.stopped_why);
    assert_eq!(recorded(&file, "readings") as u64, end.recorded_rows, "what was written stays");
    // A second session onto the same record file is refused by name, the record untouched.
    let before = std::fs::metadata(&file).unwrap().len();
    let e = Session::start(cfg).err().expect("refused").to_string();
    assert!(e.contains("a database already stands there") && e.contains("small.record.duckdb"), "{e}");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), before);
}
