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
    // Which rows: every poll once, and the polls the window still holds the record's own, every
    // column of them (the source, the key, the stamps and every tag's value).
    type Poll = (String, Option<String>, i64, i64, i64, Option<f64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>);
    let record: std::collections::BTreeMap<i64, Poll> = {
        let conn = duckdb::Connection::open(&file).unwrap();
        let mut stmt = conn.prepare("SELECT CAST(seq AS BIGINT), source, key, index_ms, epoch_ms(ts), arrival_ms, speed, run, word, word_cmd, cmd FROM plc_main").unwrap();
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?))))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(record.len() as u64, c.polls, "each poll once");
    let page = live.store.query_rows("plc_main", &RowsQuery { limit: 500, ..RowsQuery::default() }).unwrap();
    assert_eq!(page.columns.len(), 11, "the columns compared: {:?}", page.columns);
    let at = |name: &str| page.columns.iter().position(|col| col.name == name).unwrap();
    assert!(!page.rows.is_empty());
    for row in &page.rows {
        let seq = row[at("seq")].as_i64().unwrap();
        let (text, int) = (|n: &str| row[at(n)].as_str().map(str::to_string), |n: &str| row[at(n)].as_i64());
        let held: Poll = (text("source").unwrap(), text("key"), int("index_ms").unwrap(), int("ts").unwrap(), int("arrival_ms").unwrap(), row[at("speed")].as_f64(), int("run"), int("word"), int("word_cmd"), int("cmd"));
        assert_eq!(record.get(&seq), Some(&held), "the poll {seq} in the window and in the record");
    }
    // Every table of the record together holds exactly the rows the store took, the
    // announcements among them.
    let on_drive: u64 = {
        let conn = duckdb::Connection::open(&file).unwrap();
        let mut stmt = conn.prepare("SELECT table_name FROM duckdb_tables() WHERE NOT internal").unwrap();
        let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        tables.iter().map(|t| conn.query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get::<_, i64>(0)).unwrap() as u64).sum()
    };
    assert_eq!(on_drive, c.rows, "every row the store took on the drive");
    assert!(recorded(&file, "announce") >= 1);
    assert_eq!(recorded(&file, "fast"), {
        let conn = duckdb::Connection::open(&file).unwrap();
        conn.query_row("SELECT count(*) FROM plc_main WHERE speed > 1.5", [], |r| r.get::<_, i64>(0)).unwrap()
    }, "the project's view, in the record as a view");
    assert_eq!(veredas_portal::serve::snapshot(&live)["recording"]["recorded_rows"], json!(c.rows));
}

/// The record opened by another process: this test binary run again with the record's path, its
/// rows counted, or the refusal (in-process opens prove nothing here, the file's lock being the
/// process's own).
fn opened_elsewhere(file: &Path) -> Result<i64, String> {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["another_process_opens_the_record", "--exact", "--nocapture", "--test-threads=1"])
        .env("VEREDAS_RECORD_TO_OPEN", file)
        .output()
        .unwrap();
    // The answer follows the runner's own `test ... ` on its line.
    let text = String::from_utf8_lossy(&out.stdout);
    if let Some(at) = text.find("rows=") {
        return Ok(text[at + 5..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap());
    }
    Err(text.find("refused=").and_then(|at| text[at..].lines().next()).map(str::to_string).unwrap_or_else(|| format!("no answer: {text} {}", String::from_utf8_lossy(&out.stderr))))
}

/// The other process of [`opened_elsewhere`]; with no record named, nothing.
#[test]
fn another_process_opens_the_record() {
    let Some(path) = std::env::var_os("VEREDAS_RECORD_TO_OPEN") else { return };
    match duckdb::Connection::open(&path).and_then(|c| c.query_row("SELECT count(*) FROM readings", [], |r| r.get::<_, i64>(0))) {
        Ok(n) => println!("rows={n}"),
        Err(e) => println!("refused={e}"),
    }
}

/// A normal stop writes the batch left, folds the record's log, publishes the size the drive holds
/// and lets the record go: another process, refused it while the session runs, opens it after the
/// stop and reads every row. A record written every minute, so the stop's batch is the only one.
#[test]
fn a_normal_stop_writes_the_last_batch_folds_the_log_and_lets_the_record_go() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..3_000);
    let file = dir.path().join("stop.record.duckdb");
    let mut cfg = SessionConfig::new("stop");
    cfg.start = Some(midnight());
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.recording = Some(RecordingConfig { file: file.clone(), limit_bytes: 64 << 20, write_every: Duration::from_secs(60) });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(30), || rows_of(&live, "readings") == 3_000), "{}", rows_of(&live, "readings"));
    assert_eq!(status(&live).recorded_rows, 0, "nothing written before the stop");
    let held = opened_elsewhere(&file);
    assert!(held.as_ref().is_err_and(|e| e.starts_with("refused=")), "the record the session's while it runs: {held:?}");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let end = status(&live);
    assert_eq!((end.recorded_rows, end.unrecorded_rows, end.stopped_why.clone()), (3_000, 0, None), "{end:?}");
    let wal = std::path::PathBuf::from(format!("{}.wal", file.display()));
    let on_drive = std::fs::metadata(&file).unwrap().len() + std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert_eq!(end.written_bytes, on_drive, "the size published is the drive's");
    assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0, "the log folded into the file");
    assert_eq!(opened_elsewhere(&file), Ok(3_000), "the record any tool's once the session stopped");
}

/// The rows the stop's last passes take are recorded too: a log of 200 000 lines read from its
/// start, a record written every minute, the session stopped as soon as the store holds a row, so
/// nearly every row reaches the store in the stop's own drain; every one of them is on the drive
/// and counted recorded, none unrecorded.
#[test]
fn the_rows_the_stop_s_last_passes_take_are_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..200_000);
    let file = dir.path().join("drain.record.duckdb");
    let mut cfg = SessionConfig::new("drain");
    cfg.start = Some(midnight());
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.recording = Some(RecordingConfig { file: file.clone(), limit_bytes: 1 << 30, write_every: Duration::from_secs(60) });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(30), || rows_of(&live, "readings") > 0), "no row stored");
    let before = rows_of(&live, "readings");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let c = live.counters.read().unwrap().clone();
    let st = status(&live);
    assert!(c.rows > before, "the stop's last passes took rows: {before} before, {} after", c.rows);
    assert_eq!((st.recorded_rows, st.unrecorded_rows, st.stopped_why.clone()), (c.rows, 0, None), "{st:?}");
    assert_eq!(recorded(&file, "readings") as u64, c.rows, "every row the store took on the drive");
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
    assert_eq!(at_stop.stopped_why, Some(format!("the record reached its size limit ({} of {} bytes)", at_stop.written_bytes, 200 * 1024)), "{at_stop:?}");
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
    // Which rows: the log's first lines, each once, with their temperatures.
    let n = end.recorded_rows;
    let (distinct, first, last, low, high, sum): (i64, i64, i64, f64, f64, f64) = duckdb::Connection::open(&file)
        .unwrap()
        .query_row("SELECT count(DISTINCT seq), CAST(min(seq) AS BIGINT), CAST(max(seq) AS BIGINT), min(temp), max(temp), sum(temp) FROM readings", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .unwrap();
    let want: f64 = (0..n as u32).map(temp).sum();
    assert_eq!((distinct as u64, first, last as u64, low, high, sum), (n, 0, n - 1, temp(0), temp(n as u32 - 1), want), "lines 0 to {}", n - 1);
    // And each line's source, key and state, as the rule read them (`item=A0`, `A1`, `A2` in turn).
    let whole: i64 = duckdb::Connection::open(&file)
        .unwrap()
        .query_row("SELECT count(*) FROM readings WHERE source = 'log' AND key = 'A' || CAST(seq % 3 AS VARCHAR) AND state = 'run'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(whole as u64, n, "every line's source, key and state");
    // The snapshot carries the recording whole: its file, its limit, its size, the rows recorded
    // and not, and why it stopped.
    assert_eq!(
        veredas_portal::serve::snapshot(&live)["recording"],
        json!({
            "file": file.display().to_string(),
            "limit_bytes": 200 * 1024,
            "written_bytes": end.written_bytes,
            "recorded_rows": end.recorded_rows,
            "unrecorded_rows": end.unrecorded_rows,
            "stopped_why": end.stopped_why,
        })
    );
    assert!(end.unrecorded_rows >= 50 && end.written_bytes >= end.limit_bytes, "{end:?}");
    // Beside a store on the drive a record is refused by name before anything opens: that store
    // already holds every row.
    let mut on_disk = cfg.clone();
    let disk = dir.path().join("kept.duckdb");
    on_disk.residency = veredas_portal::store::Residency::Disk { path: disk.clone(), memory_limit_mb: 0 };
    on_disk.recording = Some(RecordingConfig { file: dir.path().join("other.record.duckdb"), limit_bytes: 1 << 20, write_every: Duration::from_secs(1) });
    let e = Session::start(on_disk).err().expect("refused").to_string();
    assert_eq!(e, format!("a record is for a store in memory: this one is on disk at {}, where every row already goes", disk.display()));
    assert!(!disk.exists() && !dir.path().join("other.record.duckdb").exists());
    // A second session onto the same record file is refused by name, the record untouched.
    let before = std::fs::metadata(&file).unwrap().len();
    let e = Session::start(cfg).err().expect("refused").to_string();
    assert!(e.contains("a database already stands there") && e.contains("small.record.duckdb"), "{e}");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), before);
}

/// The drive written seldom: against the bench's plant, polled every 50 ms, a record written every
/// two seconds takes its rows in batches at least that far apart, never on every pass.
#[test]
fn a_record_is_written_once_a_period_whatever_the_poll_rate() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = session_config("seldom", &log, &bench, port);
    let record = dir.path().join("seldom.record.duckdb");
    cfg.recording = Some(RecordingConfig { file: record.clone(), limit_bytes: 64 << 20, write_every: Duration::from_secs(2) });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(20), || live.counters.read().unwrap().polls >= 20));
    // Looks every 20 ms until four writes are seen: the rows recorded change at writes alone (the
    // status is published at each), every write two seconds after the one before, a pass's
    // lateness and a margin for a loaded machine aside, never sooner. And the drive itself, the
    // record's file and its log (their sizes and times), changes at those writes alone: as often,
    // as far apart, never between two.
    let on_drive = |f: &Path| -> Vec<(u64, Option<std::time::SystemTime>)> {
        [f.to_path_buf(), std::path::PathBuf::from(format!("{}.wal", f.display()))]
            .iter()
            .map(|p| std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None)))
            .collect()
    };
    let started = std::time::Instant::now();
    let mut last = status(&live).recorded_rows;
    let mut drive = on_drive(&record);
    let mut changes: Vec<std::time::Duration> = Vec::new();
    let mut drive_changes: Vec<std::time::Duration> = Vec::new();
    while changes.len() < 4 && started.elapsed() < Duration::from_secs(12) {
        let held = on_drive(&record);
        if held != drive {
            drive_changes.push(started.elapsed());
            drive = held;
        }
        let now = status(&live).recorded_rows;
        if now != last {
            changes.push(started.elapsed());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // A batch is one commit, which the drive may show in more than one step under load (its
    // bytes, then its time): steps closer than 300 ms are one write, and a write is done within
    // 500 ms. A write the first look met midway (its bytes down, its status not yet published)
    // counts on the status alone.
    let mut writes: Vec<(std::time::Duration, std::time::Duration)> = Vec::new();
    for t in &drive_changes {
        match writes.last_mut() {
            Some((_, end)) if *t - *end < Duration::from_millis(300) => *end = *t,
            _ => writes.push((*t, *t)),
        }
    }
    assert!(
        writes.len() == changes.len() || writes.len() + 1 == changes.len(),
        "the drive written {} times for {} published writes: {drive_changes:?} against {changes:?}",
        writes.len(),
        changes.len()
    );
    for (start, end) in &writes {
        assert!(*end - *start <= Duration::from_millis(500), "a write on the drive from {start:?} to {end:?}: {drive_changes:?}");
    }
    for pair in writes.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!(gap >= Duration::from_millis(1_800) && gap <= Duration::from_millis(2_600), "the drive written twice {gap:?} apart: {drive_changes:?}");
    }
    assert_eq!(changes.len(), 4, "four writes at one every two seconds: {changes:?}");
    for pair in changes.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap >= Duration::from_millis(1_800) && gap <= Duration::from_millis(2_600), "two writes {gap:?} apart: {changes:?}");
    }
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The limit stops the recording at the first write that reaches it: written in batches under it,
/// every size published before the stop is under the limit and the one at the stop at or past
/// it; once closed, the size published is the one the drive holds, the log folded into the file.
#[test]
fn the_limit_stops_the_recording_at_the_first_write_that_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let file = dir.path().join("chunks.record.duckdb");
    let mut cfg = SessionConfig::new("chunks");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.recording = Some(RecordingConfig { file: file.clone(), limit_bytes: 400 * 1024, write_every: Duration::from_millis(200) });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let mut sizes: Vec<(u64, bool)> = Vec::new();
    let mut i = 0u32;
    let started = std::time::Instant::now();
    while status(&live).stopped_why.is_none() && started.elapsed() < Duration::from_secs(60) {
        write_lines(&log, i..i + 500);
        i += 500;
        let until = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < until {
            let st = status(&live);
            if sizes.last() != Some(&(st.written_bytes, st.stopped_why.is_some())) {
                sizes.push((st.written_bytes, st.stopped_why.is_some()));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let at_stop = status(&live);
    assert_eq!(at_stop.stopped_why, Some(format!("the record reached its size limit ({} of {} bytes)", at_stop.written_bytes, 400 * 1024)), "{at_stop:?}");
    // The size the stop names is the drive's once the write that reached the limit landed, the
    // session still running (a stopped record takes no write, so the drive holds still): the
    // write measured after its rows, so the stop falls at the first write past the limit.
    let wal = std::path::PathBuf::from(format!("{}.wal", file.display()));
    let drive = || std::fs::metadata(&file).unwrap().len() + std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert_eq!(at_stop.written_bytes, drive(), "the size at the stop the drive's, before the session stops");
    let under: Vec<u64> = sizes.iter().filter(|(_, stopped)| !stopped).map(|(b, _)| *b).collect();
    assert!(under.iter().filter(|b| **b > 0).count() >= 2, "several writes under the limit first: {sizes:?}");
    assert!(under.iter().all(|b| *b < at_stop.limit_bytes), "every size published before the stop under the limit: {sizes:?}");
    assert!(at_stop.written_bytes >= at_stop.limit_bytes, "{at_stop:?}");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let end = status(&live);
    assert_eq!(end.written_bytes, drive(), "the size published is the drive's");
    assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0, "the log folded into the file");
    assert_eq!(recorded(&file, "readings") as u64, end.recorded_rows);
}
