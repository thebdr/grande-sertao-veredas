//! The controller a portal is embedded in, as a source (contract: the controller-state clause,
//! P-019): its state changes and its fault records handed in process through the session's feed,
//! stored like any other data (rows, series, frames, health), the state's text and parts by core's
//! one writer and table (C-070). Beside the bench's plant, read by the same session.

mod common;

use std::time::Duration;

use common::plant::*;
use serde_json::{Value as Json, json};
use veredas_core::state::{CommandSource, ControllerState, FaultReason, State, StartupPhase, StartupReason, StopReason};
use veredas_portal::session::{ControllerSourceConfig, FaultRecord, FeedClosed, HealthState, Live, Session, SessionConfig, CONTROLLER_FAULTS, CONTROLLER_STATES};
use veredas_portal::store::RowsQuery;

/// A table's rows, oldest first, each as its column names to its cells.
fn rows(live: &Live, table: &str) -> Vec<serde_json::Map<String, Json>> {
    let page = live.store.query_rows(table, &RowsQuery { limit: 100, ..RowsQuery::default() }).unwrap();
    let mut out: Vec<serde_json::Map<String, Json>> =
        page.rows.iter().map(|r| page.columns.iter().zip(r.iter()).map(|(c, v)| (c.name.clone(), v.clone())).collect()).collect();
    out.reverse();
    out
}

fn controller() -> ControllerSourceConfig {
    ControllerSourceConfig { source_id: "cpu".into(), display_name: "the controller".into() }
}

#[test]
fn a_controller_s_states_and_faults_are_stored_like_any_other_data() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = session_config("plant", &log, &bench, port);
    cfg.controller = Some(controller());
    cfg.idle_after_ms = 200;
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let mut frames = live.subscribe();
    let feed = session.controller_feed().expect("a feed for the controller source");
    assert_eq!(live.sources.iter().find(|s| s.id == "cpu").map(|s| (s.kind, s.display_name.as_str())), Some(("controller", "the controller")));
    assert!(wait(Duration::from_secs(5), || live.health.read().unwrap().get("cpu").is_some_and(|h| h.state == HealthState::Waiting)));

    // A start, a run, a fault: the states as the controller holds them.
    let states = [
        ControllerState::new(State::Stop(Some(StopReason::NoProgram))),
        ControllerState::new(State::Startup { reason: Some(StartupReason::Warm), phase: Some(StartupPhase::Devices { up: 3, of: 5 }) }),
        ControllerState::new(State::Run),
        ControllerState::new(State::Fault(Some(FaultReason::Watchdog { task: "main".into() }))),
        ControllerState::new(State::Stop(Some(StopReason::Command { source: CommandSource::Oficina }))),
    ];
    for s in &states {
        feed.push_state(s).unwrap();
    }
    let record = FaultRecord { fault_state: states[3].clone(), download: Some(7), outcome: Some("no handler".into()) };
    feed.push_fault(&record).unwrap();
    assert!(wait(Duration::from_secs(10), || rows(&live, CONTROLLER_STATES).len() == 5 && rows(&live, CONTROLLER_FAULTS).len() == 1));

    // Every state a row: its text by core's one writer, its word, its reason by number and word,
    // its health; a state with no reason holds none.
    let held = rows(&live, CONTROLLER_STATES);
    let parts: Vec<(Json, Json, Json, Json, Json)> =
        held.iter().map(|r| (r["state_text"].clone(), r["state"].clone(), r["reason"].clone(), r["reason_word"].clone(), r["health"].clone())).collect();
    assert_eq!(
        parts,
        vec![
            (json!("STOP no_program(id=104)"), json!("STOP"), json!(104), json!("no_program"), json!("function check")),
            (json!("STARTUP warm(id=201) devices(up=3, of=5)"), json!("STARTUP"), json!(201), json!("warm"), json!("function check")),
            (json!("RUN"), json!("RUN"), Json::Null, Json::Null, json!("normal")),
            (json!("FAULT watchdog(id=501, task=\"main\")"), json!("FAULT"), json!(501), json!("watchdog"), json!("failure")),
            (json!("STOP command(id=101, source=\"oficina\")"), json!("STOP"), json!(101), json!("command"), json!("function check")),
        ]
    );
    for (r, s) in held.iter().zip(states.iter()) {
        assert_eq!(r["state_text"], json!(s.to_string()), "the text is core's writer's");
        assert_eq!((r["source"].clone(), r["key"].clone()), (json!("cpu"), Json::Null));
    }
    let seqs: Vec<u64> = held.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    // The fault record a row of its own.
    let fault = &rows(&live, CONTROLLER_FAULTS)[0];
    assert_eq!(
        (fault["fault_text"].clone(), fault["cause"].clone(), fault["cause_word"].clone(), fault["download"].clone(), fault["outcome"].clone(), fault["seq"].clone()),
        (json!("FAULT watchdog(id=501, task=\"main\")"), json!(501), json!("watchdog"), json!(7), json!("no handler"), json!(6))
    );

    // Series like any other: listed, counted, the last value the last state's.
    assert!(wait(Duration::from_secs(5), || live.series.read().unwrap().get("cpu|state").is_some_and(|s| s.count == 5)));
    let listing = live.series.read().unwrap().clone();
    assert_eq!(listing["cpu|state"].last_value, "STOP");
    assert_eq!((listing["cpu|reason"].count, listing["cpu|reason"].last_value.as_str()), (4, "101"), "RUN carries no reason");
    assert_eq!(listing["cpu|cause"].last_value, "501");
    // Framed like any other numeric sample.
    let mut reasons = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        let v: Json = serde_json::from_str(&frame).unwrap();
        for s in v["samples"].as_array().unwrap() {
            if s["src"] == "cpu" && s["ser"] == "reason" {
                reasons.push(s["v"].as_f64().unwrap() as i64);
            }
        }
    }
    assert_eq!(reasons, vec![104, 201, 501, 101]);
    // Its health: live once a state came, and never idle, a state standing until it changes.
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(wait(Duration::from_secs(2), || live.health.read().unwrap()["cpu"].state == HealthState::Live), "{:?}", live.health.read().unwrap()["cpu"]);
    assert_eq!(live.health.read().unwrap()["cpu"].count, 6);
    // The plant beside it, read by the same session.
    assert!(wait(Duration::from_secs(10), || live.counters.read().unwrap().polls >= 1));

    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert_eq!(live.health.read().unwrap()["cpu"].state, HealthState::Ended);
    assert_eq!(feed.push_state(&states[2]), Err(FeedClosed), "a push after the stop is refused");
}

#[test]
fn a_controller_source_is_checked_like_any_other() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    // Alone: a session of one controller source starts.
    let mut cfg = SessionConfig::new("alone");
    cfg.controller = Some(controller());
    let session = Session::start(cfg).unwrap();
    session.controller_feed().unwrap().push_state(&ControllerState::new(State::Run)).unwrap();
    assert!(wait(Duration::from_secs(5), || rows(&session.live(), CONTROLLER_STATES).len() == 1));
    drop(session);
    // No controller, no feed.
    let mut cfg = SessionConfig::new("none");
    cfg.logs.push(log_source("log", &log));
    assert!(Session::start(cfg).unwrap().controller_feed().is_none());
    // Its id taken by a log, and its tables' names taken by a rule: refused by name.
    let mut cfg = SessionConfig::new("clash");
    cfg.logs.push(log_source("cpu", &log));
    cfg.controller = Some(controller());
    assert_eq!(Session::start(cfg).err().unwrap().to_string(), "two sources share the id `cpu`");
    let mut cfg = SessionConfig::new("table");
    cfg.logs.push(log_source("log", &log));
    let mut rule = readings_rule(None);
    rule.name = CONTROLLER_FAULTS.into();
    cfg.rules.push(rule);
    cfg.controller = Some(controller());
    assert_eq!(
        Session::start(cfg).err().unwrap().to_string(),
        "two tables would be named `controller_faults`: capture rule `controller_faults` and the faults of controller `cpu`"
    );
}
