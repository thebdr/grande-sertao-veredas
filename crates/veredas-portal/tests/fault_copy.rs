//! The store's window written to the drive at a fault (contract: the fault-copy clause, P-019): at
//! every fault record the controller source hands in, the session writes its store, the fault's
//! row in it, to a file of its own in the folder the embedding names, through its own connection.
//! Against the bench's plant, a store kept as a window, the copies read back by a plain connection.

mod common;

use std::path::Path;
use std::time::Duration;

use common::plant::*;
use veredas_core::state::{ControllerState, FaultReason, State};
use veredas_portal::session::{ControllerSourceConfig, FaultRecord, Session};

fn count(file: &Path, sql: &str) -> i64 {
    duckdb::Connection::open(file).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn fault(task: &str) -> FaultRecord {
    FaultRecord { fault_state: ControllerState::new(State::Fault(Some(FaultReason::Watchdog { task: task.into() }))), download: Some(3), outcome: None }
}

/// The files in a folder, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    out.sort();
    out
}

#[test]
fn at_a_fault_the_window_is_written_to_the_drive_with_the_fault_record() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let copies = dir.path().join("faults");
    let mut cfg = session_config("press", &log, &bench, port);
    cfg.store_window = Some(Duration::from_secs(1));
    cfg.controller = Some(ControllerSourceConfig { source_id: "cpu".into(), display_name: String::new() });
    cfg.fault_copy_dir = Some(copies.clone());
    cfg.tables.push(veredas_portal::store::TableDef { name: "fast".into(), sql: "SELECT * FROM plc_main WHERE speed > 1.5".into() });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let feed = session.controller_feed().unwrap();
    // The window full: polls let go, a second of them held.
    assert!(wait(Duration::from_secs(20), || live.counters.read().unwrap().evicted >= 20), "{:?}", live.counters.read().unwrap());
    assert!(!copies.exists(), "nothing written before a fault");
    feed.push_state(&fault("main").fault_state).unwrap();
    feed.push_fault(&fault("main")).unwrap();
    assert!(wait(Duration::from_secs(10), || live.counters.read().unwrap().fault_copies == 1), "{:?}", live.counters.read().unwrap());
    let written = files(&copies);
    assert_eq!(written.len(), 1, "{written:?}");
    assert!(written[0].starts_with("press-") && written[0].ends_with(".fault.duckdb"), "{written:?}");
    let first = copies.join(&written[0]);
    // Named for the session and the fault's time on the controller's clock, read off the copy.
    let index: i64 = count(&first, "SELECT index_ms FROM controller_faults");
    let at = chrono::DateTime::from_timestamp_millis(index).unwrap().naive_utc();
    assert_eq!(written[0], format!("press-{}.fault.duckdb", at.format("%Y%m%d-%H%M%S")));
    assert_eq!(live.counters.read().unwrap().last_fault_copy.as_deref(), Some(first.display().to_string().as_str()));
    // The copy: the fault's record, the state it came with, the window's polls (a second of
    // them, not every poll), the project's view.
    assert_eq!(count(&first, "SELECT count(*) FROM controller_faults WHERE cause = 501 AND download = 3 AND fault_text = 'FAULT watchdog(id=501, task=\"main\")'"), 1);
    assert_eq!(count(&first, "SELECT count(*) FROM controller_states WHERE state = 'FAULT'"), 1);
    let polled = count(&first, "SELECT count(*) FROM plc_main");
    let polls = live.counters.read().unwrap().polls as i64;
    assert!(polled > 0 && polled < polls, "the window's polls, {polled} of {polls}");
    assert_eq!(count(&first, "SELECT CAST(max(arrival_ms) - min(arrival_ms) <= 3000 AS BIGINT) FROM plc_main"), 1, "a window's stretch");
    assert_eq!(count(&first, "SELECT count(*) FROM fast"), count(&first, "SELECT count(*) FROM plc_main WHERE speed > 1.5"));
    // Two faults at once: two files, never one over the other.
    feed.push_fault(&fault("a")).unwrap();
    feed.push_fault(&fault("b")).unwrap();
    assert!(wait(Duration::from_secs(10), || live.counters.read().unwrap().fault_copies == 3));
    let written = files(&copies);
    assert_eq!(written.len(), 3, "{written:?}");
    // Each copy named for its fault's second, `.fault` for the first in a second and `.fault-1`
    // on for the next, in the order the faults came (read off the copy that holds all three); the
    // counters naming the newest.
    let whole = written.iter().map(|f| copies.join(f)).max_by_key(|f| count(f, "SELECT count(*) FROM controller_faults")).unwrap();
    let conn = duckdb::Connection::open(&whole).unwrap();
    let mut stmt = conn.prepare("SELECT index_ms FROM controller_faults ORDER BY seq").unwrap();
    let faults: Vec<i64> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    drop(stmt);
    drop(conn);
    assert_eq!(faults.len(), 3);
    let mut names: Vec<String> = Vec::new();
    for (i, ms) in faults.iter().enumerate() {
        let second = chrono::DateTime::from_timestamp_millis(*ms).unwrap().naive_utc().format("%Y%m%d-%H%M%S").to_string();
        let before = faults[..i].iter().filter(|m| chrono::DateTime::from_timestamp_millis(**m).unwrap().naive_utc().format("%Y%m%d-%H%M%S").to_string() == second).count();
        names.push(if before == 0 { format!("press-{second}.fault.duckdb") } else { format!("press-{second}.fault-{before}.duckdb") });
    }
    let mut want = names.clone();
    want.sort();
    assert_eq!(written, want, "the faults came at {faults:?}");
    assert!(names.iter().any(|n| n.contains(".fault-1.")), "a second fault in the same second: {names:?}");
    assert_eq!(live.counters.read().unwrap().last_fault_copy.as_deref(), Some(copies.join(&names[2]).display().to_string().as_str()));
    // The snapshot (C-044) carries the two counters, published within a refresh.
    assert!(
        wait(Duration::from_secs(2), || {
            let c = &veredas_portal::serve::snapshot(&live)["counters"];
            c["fault_copies"] == 3 && c["last_fault_copy"] == copies.join(&names[2]).display().to_string()
        }),
        "{}",
        veredas_portal::serve::snapshot(&live)["counters"]
    );
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert_eq!(count(&first, "SELECT count(*) FROM controller_faults"), 1, "a copy is never written again");
}

/// A fault accepted while the session stops has its copy: the feed closes before the last drain,
/// and the last passes copy what they store. Every push that returned Ok is a file.
#[test]
fn a_fault_accepted_while_the_session_stops_has_its_copy() {
    for round in 0..3u64 {
        let dir = tempfile::tempdir().unwrap();
        let copies = dir.path().join("faults");
        let mut cfg = veredas_portal::session::SessionConfig::new("race");
        cfg.controller = Some(ControllerSourceConfig { source_id: "cpu".into(), display_name: String::new() });
        cfg.fault_copy_dir = Some(copies.clone());
        let session = Session::start(cfg).unwrap();
        let feed = session.controller_feed().unwrap();
        let pusher = std::thread::spawn(move || {
            let (mut accepted, mut refused) = (0u64, 0u64);
            while refused < 5 {
                match feed.push_fault(&fault("main")) {
                    Ok(()) => accepted += 1,
                    Err(_) => refused += 1,
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            accepted
        });
        std::thread::sleep(Duration::from_millis(30 + round * 11));
        for (id, r) in session.stop() {
            r.unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let accepted = pusher.join().unwrap();
        assert!(accepted > 0);
        assert_eq!(files(&copies).len() as u64, accepted, "round {round}: every accepted fault a copy");
    }
}

#[test]
fn no_folder_no_copy_and_a_folder_that_cannot_be_made_is_an_error_with_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = veredas_portal::session::SessionConfig::new("quiet");
    cfg.logs.push(log_source("log", &log));
    cfg.controller = Some(ControllerSourceConfig { source_id: "cpu".into(), display_name: String::new() });
    let session = Session::start(cfg.clone()).unwrap();
    session.controller_feed().unwrap().push_fault(&fault("main")).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(5), || live.counters.read().unwrap().rows == 1));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(live.counters.read().unwrap().fault_copies, 0, "no folder, no copy");
    drop(session);
    // A file where the folder should be: the copy fails, the session goes on and says why.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "a file").unwrap();
    cfg.fault_copy_dir = Some(blocked.join("faults"));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    session.controller_feed().unwrap().push_fault(&fault("main")).unwrap();
    assert!(wait(Duration::from_secs(5), || live.counters.read().unwrap().store_errors == 1), "{:?}", live.counters.read().unwrap());
    let c = live.counters.read().unwrap().clone();
    assert_eq!(c.fault_copies, 0);
    assert!(c.last_error.as_deref().unwrap().starts_with(&format!("store: the copy at a fault: create {}", blocked.join("faults").display())), "{c:?}");
    assert_eq!(c.rows, 1, "the fault stored all the same");
}

/// The copy is written through the session's own connection: a reader holding the store's query
/// connection (an export into a pipe nobody reads) holds no copy back. Unix: the pipe is a FIFO.
#[cfg(unix)]
#[test]
fn a_reader_holding_the_store_holds_no_copy_back() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..10);
    let copies = dir.path().join("faults");
    let mut cfg = veredas_portal::session::SessionConfig::new("held");
    cfg.start = chrono::NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0);
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.controller = Some(ControllerSourceConfig { source_id: "cpu".into(), display_name: String::new() });
    cfg.fault_copy_dir = Some(copies.clone());
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(10), || live.counters.read().unwrap().rows == 10));
    let pipe = dir.path().join("pipe.csv");
    assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
    let (reader, path) = (std::sync::Arc::clone(&live), pipe.clone());
    let export = std::thread::spawn(move || reader.store.export_table_csv("readings", &path));
    std::thread::sleep(Duration::from_millis(300));
    session.controller_feed().unwrap().push_fault(&fault("main")).unwrap();
    let copied = wait(Duration::from_secs(5), || live.counters.read().unwrap().fault_copies == 1);
    let held = !export.is_finished();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::fs::File::open(&pipe).unwrap(), &mut text).unwrap();
    export.join().unwrap().unwrap();
    assert!(held, "the export held the query connection");
    assert!(copied, "the copy written while a reader held the query connection: {:?}", live.counters.read().unwrap());
    assert_eq!(count(&copies.join(&files(&copies)[0]), "SELECT count(*) FROM readings"), 10);
}
