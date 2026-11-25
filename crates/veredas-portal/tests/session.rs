//! The session (contract: the session clause): a project's sources of the three protocols run as
//! workers, every row into one store alike, the ingest keeping what a shell and the server read,
//! the master switch arming every write path at once, and a start refused by name before
//! anything spawns. Against the bench's plant: its OPC UA server and its machine over real
//! sockets, and a real log the test writes.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use common::plant::*;
use veredas_core::SeriesRef;
use veredas_core::opc::{Variant, WriteError};
use veredas_portal::SeriesType;
use veredas_portal::channel::SendError;
use veredas_portal::session::{ActiveConfig, HealthState, Live, LogKind, LogSourceConfig, Session, SessionConfig};
use veredas_portal::store::{RowsQuery, TableDef};

fn rows(live: &Live, table: &str) -> u64 {
    live.store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).map(|p| p.total).unwrap_or(0)
}

fn midnight() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

#[test]
fn the_three_protocols_run_into_one_store_alike() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..20);
    let mut cfg = session_config("plant", &log, &bench, port);
    // A start point in the past: the file is read whole. A view of the project over the rows.
    cfg.start = Some(midnight());
    cfg.tables.push(TableDef { name: "warm".into(), sql: "SELECT * FROM readings WHERE temp > 25".into() });
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert_eq!(
        live.sources.iter().map(|s| (s.id.as_str(), s.kind, s.has_timestamp)).collect::<Vec<_>>(),
        [("log", "log", true), ("plc", "opc", false), ("sorter", "channel", false)]
    );
    assert!(
        wait(Duration::from_secs(15), || rows(&live, "readings") == 20 && rows(&live, "plc_main") >= 3 && rows(&live, "announce") >= 1),
        "readings {} plc_main {} announce {}",
        rows(&live, "readings"),
        rows(&live, "plc_main"),
        rows(&live, "announce")
    );

    // The log's rows: the temperature typed, keyed by the item, indexed on the stamp.
    let temps = live.store.query_numeric(&SeriesRef::new("log", "temp"), None, i64::MIN, i64::MAX);
    assert_eq!(temps.len(), 20);
    assert_eq!((temps[0].index_ms, temps[0].value), (stamp_ms(0), temp(0)));
    assert_eq!((temps[19].index_ms, temps[19].value), (stamp_ms(19), temp(19)));
    assert_eq!(live.store.query_numeric(&SeriesRef::new("log", "temp"), Some("A1"), i64::MIN, i64::MAX).len(), 7, "items 1, 4, ..., 19");
    assert_eq!(rows(&live, "warm"), 9, "the view: temperatures over 25, lines 11 to 19");
    // The plant's rows: the drawn speed between its bounds, the word as configured.
    let speed = live.store.query_numeric(&SeriesRef::new("plc", "speed"), None, i64::MIN, i64::MAX);
    assert!(speed.len() >= 3 && speed.iter().all(|p| (1.0..=2.0).contains(&p.value)), "{speed:?}");
    let word = live.store.query_numeric(&SeriesRef::new("plc", "word"), None, i64::MIN, i64::MAX);
    assert!(!word.is_empty() && word.iter().all(|p| p.value == 5.0), "{word:?}");
    // The machine's rows: an announcement read by its catalog, in the message's table.
    let page = live.store.query_rows("announce", &RowsQuery { limit: 5, ..RowsQuery::default() }).unwrap();
    let col = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap_or_else(|| panic!("no column {name} in {:?}", page.columns));
    assert_eq!(page.rows[0][col("source")], serde_json::json!("sorter"));
    assert!(page.rows[0][col("item")].is_number(), "{:?}", page.rows[0]);

    // What the ingest keeps: the listing (never a store scan), the health, the vitals, the counters.
    assert!(wait(Duration::from_secs(5), || {
        let s = live.series.read().unwrap();
        s.get("log|temp").is_some_and(|e| e.count == 20) && s.contains_key("plc|speed") && s.contains_key("sorter|item")
    }));
    let t = live.series.read().unwrap()["log|temp"].clone();
    assert_eq!((t.ty, t.count, t.keys, t.keys_capped, t.last_index_ms), (SeriesType::Float, 20, 3, false, stamp_ms(19)));
    assert_eq!(t.last_value.parse::<f64>().ok(), Some(temp(19)));
    // A text series counts its samples as a numeric one does, though no frame carries them.
    let st = live.series.read().unwrap()["log|state"].clone();
    assert_eq!((st.ty, st.count, st.last_index_ms, st.last_value.as_str()), (SeriesType::Text, 20, stamp_ms(19), "run"));
    assert!(wait(Duration::from_secs(5), || {
        let h = live.health.read().unwrap();
        ["log", "plc", "sorter"].iter().all(|id| h.get(*id).is_some_and(|s| s.state == HealthState::Live))
    }), "{:?}", live.health.read().unwrap());
    let h = live.health.read().unwrap()["log"].clone();
    assert_eq!((h.count, h.last_ts), (20, Some(midnight() + chrono::Duration::seconds(10 * 3600 + 19))));
    assert!(live.vitals.read().unwrap().samples >= 40, "{:?}", live.vitals.read().unwrap());
    // The counters are a snapshot refreshed with the vitals, every 500 ms: the first refresh may
    // hold the first pass alone while the store already shows the polls that followed it.
    assert!(
        wait(Duration::from_secs(5), || {
            let c = live.counters.read().unwrap();
            c.lines == 20 && c.polls >= 3 && c.messages >= 1 && c.rows >= 24 && c.store_errors == 0
        }),
        "{:?}",
        live.counters.read().unwrap()
    );

    // The frames: every numeric sample since the last frame, for whoever listens, each with the
    // series' count with it (the cache's count once it holds the sample: 20 before these). A line
    // no rule reads among the ten appended moves no series, so the temperature's counts are the
    // series', not the lines read.
    let mut rx = live.subscribe();
    write_lines(&log, 20..25);
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(b"2026-07-19 10:00:24 a line no rule reads\n").unwrap();
    }
    write_lines(&log, 25..30);
    let log_temp = ("log".to_string(), "temp".to_string());
    let mut framed: Framed = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while (framed.get(&log_temp).map_or(0, Vec::len) < 10 || rows(&live, "readings") < 30) && Instant::now() < deadline {
        if !drain(&mut rx, &mut framed) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // Each with its key: the line's item.
    assert_eq!(framed.get(&log_temp), Some(&(20..30).map(|i| (stamp_ms(i), temp(i), i as u64 + 1, Some(format!("A{}", i % 3)))).collect::<Vec<_>>()));
    assert_eq!(rows(&live, "readings"), 30);

    // A stop ends every worker cleanly; the machine sees its peer go.
    let ended = session.stop();
    assert_eq!(ended.len(), 3);
    for (id, r) in ended {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert!(wait(Duration::from_secs(5), || bench.channel("machine").unwrap().status().peers.is_empty()));

    // After the stop's last refresh: every numeric series of the log and the server is framed
    // (the server's float and its integers alike; the machine announces before anyone listens, so
    // its item is checked by the cache against the store below), each series framed counts one up
    // from frame to frame, its last count the cache's, and a sample without a key frames none; the
    // cache holds each series' exact type, count, keys, last index and last value as the store
    // holds them; the text series counted 30.
    while drain(&mut rx, &mut framed) {}
    let cache = live.series.read().unwrap();
    for (src, ser) in [("log", "temp"), ("plc", "speed"), ("plc", "run"), ("plc", "word")] {
        assert!(framed.contains_key(&(src.to_string(), ser.to_string())), "{src}|{ser} framed: {:?}", framed.keys());
    }
    for ((src, ser), got) in &framed {
        let n: Vec<u64> = got.iter().map(|g| g.2).collect();
        assert!(n.windows(2).all(|w| w[1] == w[0] + 1), "{src}|{ser}: {n:?}");
        assert_eq!(n.last().copied(), Some(cache[&format!("{src}|{ser}")].count), "{src}|{ser}");
        if src != "log" {
            assert!(got.iter().all(|g| g.3.is_none()), "{src}|{ser} keyless: {got:?}");
        }
    }
    for (series, ty, keys) in [("log|temp", SeriesType::Float, 3), ("plc|speed", SeriesType::Float, 0), ("plc|word", SeriesType::Int, 0), ("sorter|item", SeriesType::Int, 0)] {
        let (src, ser) = series.split_once('|').unwrap();
        let stored = live.store.query_numeric(&SeriesRef::new(src, ser), None, i64::MIN, i64::MAX);
        let (e, last) = (&cache[series], stored.last().unwrap());
        assert_eq!((e.ty, e.count, e.keys, e.last_index_ms, e.last_value.parse::<f64>().ok()), (ty, stored.len() as u64, keys, last.index_ms, Some(last.value)), "{series}");
    }
    let st = &cache["log|state"];
    assert_eq!((st.ty, st.count, st.last_index_ms, st.last_value.as_str()), (SeriesType::Text, 30, stamp_ms(29), "run"));
}

/// Each series' samples as framed: stamp, value, count and key.
type Framed = BTreeMap<(String, String), Vec<(i64, f64, u64, Option<String>)>>;

/// Read one frame, if one waits, into each series' samples as framed.
fn drain(rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>, framed: &mut Framed) -> bool {
    match rx.try_recv() {
        Ok(frame) => {
            let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(v["type"], "data");
            for s in v["samples"].as_array().unwrap() {
                let key = (s["src"].as_str().unwrap().to_string(), s["ser"].as_str().unwrap().to_string());
                framed.entry(key).or_default().push((s["ms"].as_i64().unwrap(), s["v"].as_f64().unwrap(), s["n"].as_u64().unwrap(), s["k"].as_str().map(String::from)));
            }
            true
        }
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => false,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn a_start_is_refused_by_name_before_anything_spawns() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let refused = |cfg: SessionConfig| Session::start(cfg).err().expect("refused").to_string();

    assert_eq!(refused(SessionConfig::new("empty")), "the session has no source");
    let mut twice = SessionConfig::new("twice");
    twice.logs.push(log_source("log", &log));
    twice.logs.push(log_source("log", &log));
    assert_eq!(refused(twice), "two sources share the id `log`");
    let mut blank = SessionConfig::new("blank");
    blank.logs.push(log_source(" ", &log));
    assert_eq!(refused(blank), "a log has an empty id");
    let mut scope = SessionConfig::new("scope");
    scope.logs.push(log_source("log", &log));
    scope.rules.push(readings_rule(Some("nope")));
    assert_eq!(refused(scope), "capture rule `readings`: the scope `nope` is no log source of this session");
    let mut typed = SessionConfig::new("typed");
    typed.logs.push(log_source("log", &log));
    let mut rule = readings_rule(None);
    rule.types.insert("temp".into(), "number".into());
    typed.rules.push(rule);
    assert_eq!(refused(typed), "capture rule `readings`: group `temp`: unknown type `number` (int, float, text or time)");
    // A table name taken twice: a rule named like a catalog message of a channel. Refused before
    // the channel dials (nothing listens on the port, and nothing is asked to).
    let mut clash = SessionConfig::new("clash");
    clash.logs.push(log_source("log", &log));
    let mut rule = readings_rule(None);
    rule.name = "announce".into();
    clash.rules.push(rule);
    let nobody = common::ports::held_port();
    clash.channels.push(channel_config(nobody.port));
    assert_eq!(refused(clash), "two tables would be named `announce`: capture rule `announce` and message `announce` of channel `sorter`");
    // A start point in the past into a family without a timestamp rule: nothing to locate it by.
    let mut fam = SessionConfig::new("fam");
    fam.start = Some(midnight());
    fam.logs.push(LogSourceConfig {
        id: "fam".into(),
        display_name: String::new(),
        kind: LogKind::Family {
            dir: dir.path().to_string_lossy().into_owned(),
            pattern: r"plant_(?P<idx>\d+)\.log".into(),
            date_format: None,
            active: ActiveConfig::Newest,
            recursive: false,
        },
        timestamp: None,
    });
    assert_eq!(refused(fam), "log source `fam`: a start point in the past needs a timestamp rule, to find the line to start at");
    // A rule name outside the name shape, and a view named like a base table or another view.
    let mut named = SessionConfig::new("named");
    named.logs.push(log_source("log", &log));
    let mut rule = readings_rule(None);
    rule.name = "Bad Name".into();
    named.rules.push(rule);
    assert_eq!(refused(named), "capture rule `Bad Name`: the name must start with a lowercase letter or `_`");
    let mut viewed = SessionConfig::new("viewed");
    viewed.logs.push(log_source("log", &log));
    viewed.rules.push(readings_rule(None));
    viewed.tables.push(TableDef { name: "Readings".into(), sql: "SELECT 1".into() });
    assert_eq!(refused(viewed), "two tables would be named `Readings`: capture rule `readings` and table `Readings`");
    let mut views = SessionConfig::new("views");
    views.logs.push(log_source("log", &log));
    views.tables.push(TableDef { name: "v".into(), sql: "SELECT 1".into() });
    views.tables.push(TableDef { name: "v".into(), sql: "SELECT 2".into() });
    assert_eq!(refused(views), "two tables would be named `v`: table `v` and table `v`");
}

#[test]
fn the_master_switch_is_off_by_default_and_arms_every_write_path_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let port = machine_port(&bench);
    let log = dir.path().join("plant.log");
    std::fs::write(&log, "").unwrap();
    let session = Session::start(session_config("plant", &log, &bench, port)).unwrap();
    let live = session.live();
    assert!(!live.writes_armed());
    // Linked to both: a poll stored and an announcement read.
    assert!(wait(Duration::from_secs(15), || rows(&live, "plc_main") >= 1 && rows(&live, "announce") >= 1));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let plc = live.opc_handle("plc").unwrap();
    let sorter = live.channel_handle("sorter").unwrap();
    let server = bench.server("plc").unwrap();
    let word = node(&bench, "word");
    let write = |v: i32| rt.block_on(plc.write_tags(vec![(word.clone(), Variant::Int32(v))]));

    // Off: both paths refuse before anything reaches the plant.
    assert_eq!(write(9), Err(WriteError::Disarmed));
    assert_eq!(sorter.send(None, "command", stop_command()), Err(SendError::Disarmed));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(server.get("word"), Some(Variant::Int32(5)));
    assert_eq!(machine_status(&bench).registers["running"], 1);
    assert_eq!(machine_status(&bench).stats.commands, 0);

    // On, live: the word is written, the machine stops on the command.
    session.set_writes_armed(true);
    assert!(live.writes_armed());
    assert!(write(9).unwrap().iter().all(|s| s.is_good()));
    assert!(wait(Duration::from_secs(2), || server.get("word") == Some(Variant::Int32(9))));
    assert_eq!(sorter.send(None, "command", stop_command()), Ok(()));
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).registers["running"] == 0));
    // A tag no rule marks writable is refused whole, armed or not.
    let speed = node(&bench, "speed");
    assert_eq!(
        rt.block_on(plc.write_tags(vec![(speed.clone(), Variant::Double(1.0))])),
        Err(WriteError::NotWritable { source_id: "plc".into(), tag: speed })
    );

    // Off again, live: refused again, the word untouched.
    session.set_writes_armed(false);
    assert_eq!(write(1), Err(WriteError::Disarmed));
    assert_eq!(sorter.send(None, "command", stop_command()), Err(SendError::Disarmed));
    assert_eq!(server.get("word"), Some(Variant::Int32(9)));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn health_reads_waiting_then_live_then_idle_on_the_arrival_clock() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("quiet.log");
    std::fs::write(&log, "").unwrap();
    let mut cfg = SessionConfig::new("quiet");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    // Idle after 1.5 s of silence: longer than a refresh (every 500 ms), so the live state is
    // seen before it ages into idle.
    cfg.idle_after_ms = 1500;
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let state = || live.health.read().unwrap().get("log").map(|h| (h.state.clone(), h.count));
    // The windows are generous: under a loaded machine (the whole suite in parallel) a line can
    // take seconds to be read; the states and their order are what is asserted.
    assert!(wait(Duration::from_secs(10), || state() == Some((HealthState::Waiting, 0))), "{:?}", state());
    write_lines(&log, 0..1);
    assert!(wait(Duration::from_secs(10), || state() == Some((HealthState::Live, 1))), "{:?}", state());
    assert!(
        wait(Duration::from_secs(10), || matches!(state(), Some((HealthState::Idle { silence_ms }, 1)) if silence_ms >= 1500)),
        "{:?}",
        state()
    );
    // The last stamp read rides with it.
    assert_eq!(live.health.read().unwrap()["log"].last_ts, Some(midnight() + chrono::Duration::hours(10)));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// A stop stores every row the workers sent before it: the workers end first, then the pump
/// drains the channels dry, cap or no cap, and publishes a last refresh, so a reader still
/// holding the session sees the final counters and every source ended. The backlog is made
/// deterministic by holding the health lock the pump's refresh takes while the stop begins.
#[test]
fn a_stop_stores_every_row_the_workers_sent_and_publishes_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("big.log");
    write_lines(&log, 0..60_000);
    let mut cfg = SessionConfig::new("big");
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    cfg.start = Some(midnight());
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(10), || live.health.read().unwrap().contains_key("log")), "the first refresh");
    // The pump's next refresh waits on this lock, the inner lock in hand, while the tail reads
    // the file into the channel; the stop that begins meanwhile waits for the inner lock too.
    let held = live.health.read().unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let stopper = std::thread::spawn(move || session.stop());
    std::thread::sleep(Duration::from_millis(300));
    drop(held);
    for (id, r) in stopper.join().unwrap() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let counters = live.counters.read().unwrap().clone();
    assert_eq!(rows(&live, "readings"), 60_000, "{counters:?}");
    assert_eq!((counters.lines, counters.rows), (60_000, 60_000), "the counters final after the stop");
    assert_eq!(live.health.read().unwrap()["log"].state, HealthState::Ended, "a stopped source reads as ended");
}

/// A worker that failed stays failed after the stop: the stop's last refresh keeps each worker's
/// outcome, so a reader still holding the session reads the failure's text, never a clean end.
#[test]
fn a_failed_worker_still_reads_as_failed_after_the_stop() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app_notadate.log"), "x\n").unwrap();
    let mut cfg = SessionConfig::new("failing");
    cfg.logs.push(LogSourceConfig {
        id: "fam".into(),
        display_name: "a family on which no key parses".into(),
        kind: LogKind::Family {
            dir: dir.path().to_string_lossy().into_owned(),
            pattern: r"app_(?P<date>\w+)\.log".into(),
            date_format: Some("%Y%m%d".into()),
            active: ActiveConfig::Newest,
            recursive: false,
        },
        timestamp: None,
    });
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let failed = |live: &Live| live.health.read().unwrap().get("fam").map(|h| matches!(h.state, HealthState::Failed(_))).unwrap_or(false);
    assert!(wait(Duration::from_secs(10), || failed(&live)), "{:?}", live.health.read().unwrap());
    let text = match &live.health.read().unwrap()["fam"].state {
        HealthState::Failed(t) => t.clone(),
        other => panic!("{other:?}"),
    };
    assert!(text.contains("key groups parse on no member"), "{text}");
    let ended = session.stop();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].1.as_ref().map_err(|e| e.to_string()), Err(text.clone()), "the stop returns the failure");
    assert_eq!(live.health.read().unwrap()["fam"].state, HealthState::Failed(text), "and the health keeps it after the stop");
}

/// Three claims of the clause the other tests left unasserted: the series cache caps its keys at
/// 64 and flags the cap; a server away and a peer away read as retrying with the reason; a slow
/// listener loses frames while the plant is never slowed (the frames keep counting).
#[test]
fn the_key_cap_the_retrying_states_and_a_slow_listener_s_lag_are_as_the_clause_says() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let log = dir.path().join("keys.log");
    let mut text = String::new();
    for i in 0..70 {
        text.push_str(&format!("2026-07-19 10:00:00 temp=1.0 item=K{i} state=run\n"));
    }
    std::fs::write(&log, text).unwrap();
    // The plant is gone before the session starts: the server and the peer are away. The session
    // dials two ports held bound and listening for nobody, never the ones the stopped bench freed,
    // which another test's server or machine could take and answer on.
    let (opc_away, peer_away) = (common::ports::held_port(), common::ports::held_port());
    let mut cfg = session_config("away", &log, &bench, peer_away.port);
    cfg.opc[0].endpoint_url = format!("opc.tcp://127.0.0.1:{}/", opc_away.port);
    cfg.start = Some(midnight());
    bench.stop();
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let mut frames = live.subscribe();
    assert!(wait(Duration::from_secs(10), || live.series.read().unwrap().get("log|temp").is_some_and(|e| e.count == 70)), "{:?}", live.series.read().unwrap().get("log|temp"));
    let temp = live.series.read().unwrap()["log|temp"].clone();
    assert_eq!((temp.keys, temp.keys_capped), (64, true), "seventy keys: the cache holds 64 and says so");
    // Both dials are refused at once; the bound is the worker's connect timeout and a margin, for
    // a hang.
    assert!(
        wait(veredas_core::opc::worker::CONNECT_TIMEOUT + Duration::from_secs(20), || {
            let h = live.health.read().unwrap();
            matches!(h.get("plc").map(|s| &s.state), Some(HealthState::Retrying(_))) && matches!(h.get("sorter").map(|s| &s.state), Some(HealthState::Retrying(_)))
        }),
        "{:?}",
        live.health.read().unwrap()
    );
    let h = live.health.read().unwrap().clone();
    assert!(matches!(&h["plc"].state, HealthState::Retrying(why) if !why.is_empty()), "{:?}", h["plc"].state);
    assert!(matches!(&h["sorter"].state, HealthState::Retrying(why) if why.contains("connecting")), "{:?}", h["sorter"].state);
    // A listener that never reads: once more than the channel's 256 frames went out, it lags,
    // and the plant's lines kept flowing meanwhile. The plant is away here, so every frame is a
    // line: some 260 writes at 55 ms, 15 s on a quiet machine; the bound is for a hang, not a
    // pace (a loaded machine took over a minute once, with 49 frames in).
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut i = 70u32;
    while live.counters.read().unwrap().frames <= 260 && Instant::now() < deadline {
        write_lines(&log, i..i + 1);
        i += 1;
        std::thread::sleep(Duration::from_millis(55));
    }
    // The last line written is still on its way when the loop ends: the counters are read once
    // it has arrived (a published snapshot is read by waiting for what is asserted of it).
    let expected = 70 + (i - 70) as u64;
    let deadline = Instant::now() + Duration::from_secs(10);
    while live.counters.read().unwrap().lines < expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let counters = live.counters.read().unwrap().clone();
    assert!(counters.frames > 260, "{counters:?}");
    assert!(matches!(frames.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) if n > 0), "a listener that never read lags");
    assert_eq!(counters.lines, expected, "every line reached the store meanwhile");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// The pump never waits on a reader, measured against its own pace in the same session: its
/// passes over three seconds while a reader holds the store's query connection (a CSV export into
/// a named pipe nobody reads, which blocks for as long as the pipe stays shut), and while four
/// readers take that connection in short grabs (a page each, back to back: the server under a
/// polling client), are each at least half of its passes over three quiet seconds, and the lines
/// written during the hold are stored before the pipe is read. Before, the pump read its memory
/// figure through that connection: no pass and no row while the export blocked, a pass a second
/// under a polling client. Unix: the pipe is a FIFO.
#[cfg(unix)]
#[test]
fn a_reader_holding_the_store_never_holds_the_pump() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("plant.log");
    write_lines(&log, 0..40);
    let mut cfg = SessionConfig::new("held");
    cfg.start = Some(midnight());
    cfg.logs.push(log_source("log", &log));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(10), || live.counters.read().unwrap().rows == 40), "{:?}", live.counters.read().unwrap());
    // The passes over a span, as the published counters show them.
    let passes_over = |span: Duration| {
        let started = live.counters.read().unwrap().passes;
        std::thread::sleep(span);
        live.counters.read().unwrap().passes - started
    };
    let quiet = passes_over(Duration::from_secs(3));
    assert!(quiet >= 20, "the pump's own pace: {quiet} passes in three seconds");

    // Held: the export blocks on the pipe with the query connection taken.
    let pipe = dir.path().join("pipe.csv");
    assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
    let (reader, path) = (Arc::clone(&live), pipe.clone());
    let export = std::thread::spawn(move || reader.store.export_table_csv("readings", &path));
    std::thread::sleep(Duration::from_millis(300));
    write_lines(&log, 40..45);
    let held_passes = passes_over(Duration::from_secs(3));
    let counters = live.counters.read().unwrap().clone();
    let held = !export.is_finished();
    // The pipe read before anything is asserted, so a pump stuck behind the export is set free
    // and the session stops: the export ends with the rows it saw, a header and the first forty.
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::fs::File::open(&pipe).unwrap(), &mut text).unwrap();
    export.join().unwrap().unwrap();
    assert!(held, "the export held the query connection for the whole span");
    assert!(held_passes * 2 >= quiet, "the pump behind the export: {held_passes} passes against {quiet} quiet");
    assert_eq!((counters.lines, counters.rows), (45, 45), "the lines written during the hold stored during it");
    assert_eq!(text.lines().count(), 41, "{text}");
    assert!(text.lines().next().unwrap().starts_with("source,key,index_ms,ts,arrival_ms,seq,temp,state"), "{text}");

    // Grabbed: four readers taking the query connection back to back.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (live, stop) = (Arc::clone(&live), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut pages = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    rows(&live, "readings");
                    pages += 1;
                }
                pages
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let grabbed_passes = passes_over(Duration::from_secs(3));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let pages: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
    assert!(pages >= 40, "the readers kept the connection busy: {pages} pages");
    assert!(grabbed_passes * 2 >= quiet, "the pump behind the readers: {grabbed_passes} passes against {quiet} quiet");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// A line of the history tests, stamped `i` seconds after 10:00 on the plant's day (hours past
/// the hour, so tens of thousands stay stamped in order).
fn history_line(i: u32) -> String {
    format!("2026-07-19 {:02}:{:02}:{:02} temp={:.1} item=A{} state=run\n", 10 + i / 3600, (i / 60) % 60, i % 60, 20.0 + f64::from(i % 50) * 0.5, i % 3)
}

fn write_history(path: &std::path::Path, range: std::ops::Range<u32>) {
    let body: String = range.map(history_line).collect();
    std::fs::write(path, body).unwrap();
}

/// Waits until `id` says its start's history read, checking at every look that it never says
/// so before every one of `history` rows is in the store; returns the rows then.
fn rows_when_history_read(live: &Live, id: &str, history: u64) -> u64 {
    let started = Instant::now();
    loop {
        let said = live.health.read().unwrap().get(id).is_some_and(|h| h.history_read);
        let stored = rows(live, "readings");
        if said {
            assert_eq!(stored, history, "`{id}` said its history read with {stored} of {history} rows stored");
            return stored;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "`{id}` never said its history read: {stored} of {history} rows stored");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A log says when its start's history is in the store (docs/questions.md Q319): a family read
/// from a start in the past, five of its eight members empty in a row (the case a replay's guess
/// by stillness lost), says so only once every line written before the start is a row; a single
/// file read from its start alike; a log started at its end says so at once, its history skipped,
/// no row of it stored; a source with no history to read (a log not there) reads as read.
#[test]
fn a_log_says_when_its_start_s_history_is_in_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let fam = dir.path().join("fam");
    std::fs::create_dir_all(&fam).unwrap();
    write_history(&fam.join("plant_1.log"), 0..20_000);
    for i in 2..=6 {
        std::fs::write(fam.join(format!("plant_{i}.log")), "").unwrap();
    }
    write_history(&fam.join("plant_7.log"), 20_000..40_000);
    write_history(&fam.join("plant_8.log"), 40_000..60_000);
    let stamps = log_source("x", &fam).timestamp;
    let mut cfg = SessionConfig::new("history");
    cfg.start = Some(midnight());
    cfg.logs.push(LogSourceConfig {
        id: "fam".into(),
        display_name: String::new(),
        kind: LogKind::Family { dir: fam.to_string_lossy().into_owned(), pattern: r"plant_(?P<idx>\d+)\.log".into(), date_format: None, active: ActiveConfig::Newest, recursive: false },
        timestamp: stamps.clone(),
    });
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    assert_eq!(rows_when_history_read(&session.live(), "fam", 60_000), 60_000);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // A single file read from its start.
    let file = dir.path().join("one.log");
    write_history(&file, 0..40_000);
    let mut cfg = SessionConfig::new("one");
    cfg.start = Some(midnight());
    cfg.logs.push(log_source("one", &file));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    assert_eq!(rows_when_history_read(&session.live(), "one", 40_000), 40_000);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // Started at its end: read at once, nothing of its history stored; and a log not there.
    let mut cfg = SessionConfig::new("live");
    cfg.logs.push(log_source("one", &file));
    cfg.logs.push(log_source("none", &dir.path().join("not_there.log")));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(
        wait(Duration::from_secs(10), || {
            let h = live.health.read().unwrap();
            ["one", "none"].iter().all(|id| h.get(*id).is_some_and(|s| s.history_read))
        }),
        "{:?}",
        live.health.read().unwrap()
    );
    assert_eq!(rows(&live, "readings"), 0, "the history skipped");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

fn family(id: &str, dir: &std::path::Path, pattern: &str, active: ActiveConfig) -> LogSourceConfig {
    LogSourceConfig {
        id: id.into(),
        display_name: String::new(),
        kind: LogKind::Family { dir: dir.to_string_lossy().into_owned(), pattern: pattern.into(), date_format: None, active, recursive: false },
        timestamp: log_source("x", dir).timestamp,
    }
}

/// C-096's other paths (its first refute round): a fixed-name family read from a start in the
/// past whose fixed active is not there says its history read once every retiree is stored; a
/// file held at the first look and started at its end, and a family started at its end whose
/// fixed active is not there, say so at once, nothing to read; a family folder not there and one
/// born empty after the start say so at once, members appearing after it new content; and once
/// said, lines that arrive after it never take it back.
#[test]
fn a_log_s_history_read_on_every_path_and_set_once() {
    let dir = tempfile::tempdir().unwrap();
    let history = |live: &Live, id: &str| live.health.read().unwrap().get(id).is_some_and(|h| h.history_read);
    // A fixed family, its active not there, read from a start in the past.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write_history(&fixed.join("done_1.log"), 0..100);
    write_history(&fixed.join("done_2.log"), 100..200);
    let mut cfg = SessionConfig::new("fixed");
    cfg.start = Some(midnight());
    cfg.logs.push(family("fixed", &fixed, r"done_(?P<idx>\d+)\.log", ActiveConfig::Fixed("current.log".into())));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    assert_eq!(rows_when_history_read(&session.live(), "fixed", 200), 200);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // Started at their end: a file held at the first look (no read allowed), a family whose fixed
    // active is not there; a family folder not there; a folder empty at the start.
    let held = dir.path().join("held.log");
    write_history(&held, 0..10);
    #[cfg(unix)]
    std::fs::set_permissions(&held, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let born = dir.path().join("born");
    std::fs::create_dir_all(&born).unwrap();
    // A family whose active (its newest) nobody may read.
    let shut = dir.path().join("shut");
    std::fs::create_dir_all(&shut).unwrap();
    write_history(&shut.join("plant_1.log"), 0..10);
    write_history(&shut.join("plant_2.log"), 10..20);
    #[cfg(unix)]
    std::fs::set_permissions(shut.join("plant_2.log"), std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let mut cfg = SessionConfig::new("at-end");
    cfg.logs.push(family("shut", &shut, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.logs.push(log_source("held", &held));
    cfg.logs.push(family("waiting", &fixed, r"done_(?P<idx>\d+)\.log", ActiveConfig::Fixed("current.log".into())));
    cfg.logs.push(family("absent", &dir.path().join("no_such_folder"), r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.logs.push(family("born", &born, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let ids = ["held", "shut", "waiting", "absent", "born"];
    assert!(wait(Duration::from_secs(10), || ids.iter().all(|id| history(&live, id))), "{:?}", live.health.read().unwrap());
    assert_eq!(rows(&live, "readings"), 0, "nothing of a history read");
    // A member born after the start is new content, the flag kept; and lines arriving after the
    // flag never take it back, at any look while they are stored.
    write_history(&born.join("plant_1.log"), 0..5_000);
    let started = Instant::now();
    while rows(&live, "readings") < 5_000 {
        assert!(history(&live, "born"), "said once, kept: {} rows stored", rows(&live, "readings"));
        assert!(started.elapsed() < Duration::from_secs(30), "the born member never stored");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ids.iter().all(|id| history(&live, id)));
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    #[cfg(unix)]
    for p in [held, shut.join("plant_2.log")] {
        std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    }
}

/// C-096's second refute round. A member of a family there and not readable while its backfill
/// reaches it leaves the history unread for good: never said read (500 of 600 lines stored, the
/// 100 of the member lost). A fixed-name family whose fixed active is there and not readable holds
/// history: not said read until it opens and is stored. A family read from a start in the past
/// whose folder is not there fails its worker, never saying read; a file not there read from a
/// start in the past says read at once, as does one under a folder that is a regular file. A
/// member deleted before the backfill reaches it leaves
/// nothing unread: said read, the newest one (the active it would attach to) too.
#[cfg(unix)]
#[test]
fn a_history_left_unread_is_never_said_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let history = |live: &Live, id: &str| live.health.read().unwrap().get(id).map(|h| (h.history_read, h.state.clone()));
    let shut = |p: &std::path::Path, mode: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    // A family of six members, the fifth made unreadable once the start has located its point
    // (the start returns once the locate has peeked every member) and while the first's 20 000
    // lines replay: the replay reaches it shut. A chmod landing late fails the test, 20 500 rows.
    let fam = dir.path().join("fam");
    std::fs::create_dir_all(&fam).unwrap();
    write_history(&fam.join("plant_1.log"), 0..20_000);
    for m in 2..=6u32 {
        write_history(&fam.join(format!("plant_{m}.log")), 20_000 + (m - 2) * 100..20_000 + (m - 1) * 100);
    }
    let mut cfg = SessionConfig::new("member");
    cfg.start = Some(midnight());
    cfg.logs.push(family("fam", &fam, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    shut(&fam.join("plant_5.log"), 0o000);
    let live = session.live();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) && rows(&live, "readings") < 20_400 {
        assert_ne!(history(&live, "fam").map(|h| h.0), Some(true), "{} of 20 500 rows stored", rows(&live, "readings"));
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        assert_ne!(history(&live, "fam").map(|h| h.0), Some(true), "{} of 20 500 rows stored", rows(&live, "readings"));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!((history(&live, "fam").map(|h| h.0), rows(&live, "readings")), (Some(false), 20_400), "the unreadable member's lines lost");
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    shut(&fam.join("plant_5.log"), 0o644);

    // The same family, the fifth member deleted instead: gone, nothing of it left to read, the
    // history said read with the other 20 400 lines.
    let gone = dir.path().join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    for entry in std::fs::read_dir(&fam).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), gone.join(entry.file_name())).unwrap();
    }
    let mut cfg = SessionConfig::new("gone");
    cfg.start = Some(midnight());
    cfg.logs.push(family("gone", &gone, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    std::fs::remove_file(gone.join("plant_5.log")).unwrap();
    assert_eq!(rows_when_history_read(&session.live(), "gone", 20_400), 20_400);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // The newest member deleted, the active the backfill would attach to: every member before it
    // read, the history said read with their 20 100 lines, whatever is born newer being new.
    let newest = dir.path().join("newest");
    std::fs::create_dir_all(&newest).unwrap();
    write_history(&newest.join("plant_1.log"), 0..20_000);
    write_history(&newest.join("plant_2.log"), 20_000..20_100);
    write_history(&newest.join("plant_3.log"), 20_100..20_200);
    let mut cfg = SessionConfig::new("newest");
    cfg.start = Some(midnight());
    cfg.logs.push(family("newest", &newest, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    std::fs::remove_file(newest.join("plant_3.log")).unwrap();
    assert_eq!(rows_when_history_read(&session.live(), "newest", 20_100), 20_100);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // A fixed family whose active holds history and is made unreadable: waited for.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write_history(&fixed.join("done_1.log"), 0..20_000);
    for m in 2..=4u32 {
        write_history(&fixed.join(format!("done_{m}.log")), 20_000 + (m - 2) * 100..20_000 + (m - 1) * 100);
    }
    write_history(&fixed.join("current.log"), 20_300..20_400);
    let mut cfg = SessionConfig::new("fixed-held");
    cfg.start = Some(midnight());
    cfg.logs.push(family("fixed", &fixed, r"done_(?P<idx>\d+)\.log", ActiveConfig::Fixed("current.log".into())));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    shut(&fixed.join("current.log"), 0o000);
    let live = session.live();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) && rows(&live, "readings") < 20_300 {
        assert_ne!(history(&live, "fixed").map(|h| h.0), Some(true), "{} of 20 400 rows stored", rows(&live, "readings"));
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!((history(&live, "fixed").map(|h| h.0), rows(&live, "readings")), (Some(false), 20_300), "the active's history not yet read");
    shut(&fixed.join("current.log"), 0o644);
    assert_eq!(rows_when_history_read(&live, "fixed", 20_400), 20_400);
    for (id, r) in session.stop() {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    // From a start in the past: a family folder not there fails, a file not there is read at once.
    let mut cfg = SessionConfig::new("absent-past");
    cfg.start = Some(midnight());
    cfg.logs.push(family("absent", &dir.path().join("no_such_folder"), r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest));
    cfg.logs.push(log_source("none", &dir.path().join("not_there.log")));
    // A folder on the path that is a regular file: nothing can be there.
    std::fs::write(dir.path().join("a_file"), "x").unwrap();
    cfg.logs.push(log_source("under_a_file", &dir.path().join("a_file").join("app.log")));
    cfg.rules.push(readings_rule(None));
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    assert!(wait(Duration::from_secs(10), || history(&live, "none").is_some_and(|h| h.0)), "{:?}", live.health.read().unwrap());
    assert!(wait(Duration::from_secs(10), || history(&live, "under_a_file").is_some_and(|h| h.0)), "{:?}", live.health.read().unwrap());
    assert!(wait(Duration::from_secs(10), || history(&live, "absent").is_some_and(|h| matches!(h.1, HealthState::Failed(_)))), "{:?}", live.health.read().unwrap());
    assert_eq!(history(&live, "absent").map(|h| h.0), Some(false));
    let _ = session.stop();
}

/// C-096's third refute round. A line is a line once ended (C-035): the history said read holds
/// every ended line of the start, a last line its writer has not ended held until its ending
/// comes and stored then, a file's and a family's active alike; never waited for, so a file that
/// ends unended still says read.
#[test]
fn a_history_said_read_holds_every_ended_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("one.log");
    write_history(&file, 0..100);
    let unended = history_line(100);
    let append = |path: &std::path::Path, text: &str| {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
    };
    append(&file, unended.trim_end());
    let fam = dir.path().join("fam");
    std::fs::create_dir_all(&fam).unwrap();
    write_history(&fam.join("plant_1.log"), 0..100);
    write_history(&fam.join("plant_2.log"), 100..150);
    append(&fam.join("plant_2.log"), history_line(150).trim_end());
    for (id, path, log, ended) in [
        ("one", file.clone(), log_source("one", &file), 100u64),
        ("fam", fam.join("plant_2.log"), family("fam", &fam, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest), 150),
    ] {
        let mut cfg = SessionConfig::new(id);
        cfg.start = Some(midnight());
        cfg.logs.push(log);
        cfg.rules.push(readings_rule(None));
        let session = Session::start(cfg).unwrap();
        let live = session.live();
        assert_eq!(rows_when_history_read(&live, id, ended), ended);
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(rows(&live, "readings"), ended, "`{id}`: the unended line held");
        append(&path, "\n");
        assert!(wait(Duration::from_secs(10), || rows(&live, "readings") == ended + 1), "`{id}`: {} rows", rows(&live, "readings"));
        for (id, r) in session.stop() {
            r.unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }
}

/// C-096's third refute round. A family started at its end whose active is there and not
/// readable at the first look (write alone allowed) is read from where it stood once it opens,
/// as a file is: the lines written to it meanwhile read, its history skipped. One that retires
/// unopened, two newer members born meanwhile, is read from where it stood once it opens,
/// awaited till then, the generation between whole and the new active whole, held in turn; a
/// fixed name's held active retired with no active after it for a while alike; the newer active,
/// held in turn, retiring before it opens, read whole once it does; and a generation between held
/// past every patience, awaited and read whole.
#[cfg(unix)]
#[test]
fn a_held_active_started_at_its_end_is_read_from_where_it_stood() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mode = |p: &std::path::Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    let append = |path: &std::path::Path, range: std::ops::Range<u32>| {
        use std::io::Write;
        let body: String = range.map(history_line).collect();
        std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(body.as_bytes()).unwrap();
    };
    let same = dir.path().join("same");
    std::fs::create_dir_all(&same).unwrap();
    write_history(&same.join("plant_1.log"), 0..10);
    write_history(&same.join("plant_2.log"), 10..20);
    mode(&same.join("plant_2.log"), 0o200);
    let retired = dir.path().join("retired");
    std::fs::create_dir_all(&retired).unwrap();
    write_history(&retired.join("plant_1.log"), 0..10);
    mode(&retired.join("plant_1.log"), 0o200);
    let file = dir.path().join("held.log");
    write_history(&file, 0..10);
    mode(&file, 0o200);
    // A fixed name's held active, renamed into a retiree with no active for a while.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write_history(&fixed.join("done_1.log"), 0..10);
    write_history(&fixed.join("current.log"), 10..20);
    mode(&fixed.join("current.log"), 0o200);
    // The newer active held in turn, retiring before it opens; a younger generation held.
    let newer = dir.path().join("newer");
    let younger = dir.path().join("younger");
    for d in [&newer, &younger] {
        std::fs::create_dir_all(d).unwrap();
        write_history(&d.join("plant_1.log"), 0..10);
        mode(&d.join("plant_1.log"), 0o200);
    }
    for (id, log) in [
        ("same", family("same", &same, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest)),
        ("retired", family("retired", &retired, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest)),
        ("file", log_source("file", &file)),
        ("fixed", family("fixed", &fixed, r"done_(?P<idx>\d+)\.log", ActiveConfig::Fixed("current.log".into()))),
        ("newer", family("newer", &newer, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest)),
        ("younger", family("younger", &younger, r"plant_(?P<idx>\d+)\.log", ActiveConfig::Newest)),
    ] {
        let mut cfg = SessionConfig::new(id);
        cfg.logs.push(log);
        cfg.rules.push(readings_rule(None));
        // The start returns once the held active is looked at, not at its five-second bound.
        let begun = Instant::now();
        let session = Session::start(cfg).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(3), "`{id}`: the start waited {:?}", begun.elapsed());
        let live = session.live();
        assert!(wait(Duration::from_secs(10), || live.health.read().unwrap().get(id).is_some_and(|h| h.history_read)), "`{id}`");
        let expected = match id {
            "same" => {
                append(&same.join("plant_2.log"), 20..70);
                mode(&same.join("plant_2.log"), 0o644);
                50
            }
            "retired" => {
                append(&retired.join("plant_1.log"), 10..40);
                write_history(&retired.join("plant_2.log"), 40..60);
                write_history(&retired.join("plant_3.log"), 60..65);
                mode(&retired.join("plant_3.log"), 0o200);
                std::thread::sleep(Duration::from_millis(1500));
                assert_eq!(rows(&live, "readings"), 0, "the retired held active awaited");
                mode(&retired.join("plant_1.log"), 0o644);
                // The new active held in turn: born after the start, read whole once it opens.
                assert!(wait(Duration::from_secs(10), || rows(&live, "readings") == 50), "{} rows", rows(&live, "readings"));
                std::thread::sleep(Duration::from_millis(1000));
                mode(&retired.join("plant_3.log"), 0o644);
                55
            }
            "file" => {
                append(&file, 10..60);
                mode(&file, 0o644);
                50
            }
            "fixed" => {
                append(&fixed.join("current.log"), 20..50);
                std::fs::rename(fixed.join("current.log"), fixed.join("done_2.log")).unwrap();
                std::thread::sleep(Duration::from_millis(1000));
                write_history(&fixed.join("current.log"), 50..55);
                std::thread::sleep(Duration::from_millis(1000));
                assert_eq!(rows(&live, "readings"), 0, "the retired held active awaited");
                mode(&fixed.join("done_2.log"), 0o644);
                35
            }
            "newer" => {
                append(&newer.join("plant_1.log"), 10..40);
                write_history(&newer.join("plant_2.log"), 40..60);
                write_history(&newer.join("plant_3.log"), 60..65);
                mode(&newer.join("plant_3.log"), 0o200);
                std::thread::sleep(Duration::from_millis(1500));
                mode(&newer.join("plant_1.log"), 0o644);
                assert!(wait(Duration::from_secs(10), || rows(&live, "readings") == 50), "{} rows", rows(&live, "readings"));
                // The newer active, held, retires before it opens: read whole once it does.
                write_history(&newer.join("plant_4.log"), 65..70);
                std::thread::sleep(Duration::from_millis(1500));
                assert_eq!(rows(&live, "readings"), 50, "the newer active awaited");
                mode(&newer.join("plant_3.log"), 0o644);
                60
            }
            _ => {
                append(&younger.join("plant_1.log"), 10..40);
                write_history(&younger.join("plant_2.log"), 40..60);
                mode(&younger.join("plant_2.log"), 0o200);
                write_history(&younger.join("plant_3.log"), 60..65);
                std::thread::sleep(Duration::from_millis(1500));
                mode(&younger.join("plant_1.log"), 0o644);
                assert!(wait(Duration::from_secs(10), || rows(&live, "readings") == 30), "{} rows", rows(&live, "readings"));
                // The generation between, held past every patience: awaited, then read whole.
                std::thread::sleep(Duration::from_millis(2500));
                assert_eq!(rows(&live, "readings"), 30, "the held generation awaited");
                mode(&younger.join("plant_2.log"), 0o644);
                55
            }
        };
        assert!(wait(Duration::from_secs(10), || rows(&live, "readings") == expected), "`{id}`: {} rows", rows(&live, "readings"));
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(rows(&live, "readings"), expected, "`{id}`: nothing of the history read");
        for (id, r) in session.stop() {
            r.unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }
}

/// C-096's fourth refute round. A file held at the first look (nobody may read it) and read from
/// a start in the past holds the start's history: renamed away and another file made at its path,
/// that history is unread for good, never said read, the new file read whole; renamed away with
/// nothing at the path for a while, the same (found in its folder under its new name); deleted,
/// nothing of it is left to read, said read with no row.
#[cfg(unix)]
#[test]
fn a_file_held_from_a_past_start_says_read_only_of_its_own_history() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for case in ["replaced", "renamed", "deleted"] {
        let file = dir.path().join(format!("{case}.log"));
        write_history(&file, 0..100);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let mut cfg = SessionConfig::new(case);
        cfg.start = Some(midnight());
        cfg.logs.push(log_source(case, &file));
        cfg.rules.push(readings_rule(None));
        let session = Session::start(cfg).unwrap();
        let live = session.live();
        let said = |live: &Live| live.health.read().unwrap().get(case).is_some_and(|h| h.history_read);
        let away = dir.path().join(format!("{case}.log.1"));
        match case {
            "replaced" => {
                std::fs::rename(&file, &away).unwrap();
                write_history(&file, 100..110);
            }
            "renamed" => {
                std::fs::rename(&file, &away).unwrap();
                std::thread::sleep(Duration::from_millis(1500));
                assert!(!said(&live), "renamed away: never said read");
                write_history(&file, 100..110);
            }
            _ => {
                std::fs::remove_file(&file).unwrap();
                assert!(wait(Duration::from_secs(10), || said(&live)), "deleted: said read");
                assert_eq!(rows(&live, "readings"), 0);
            }
        }
        if case != "deleted" {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                assert!(!said(&live), "`{case}`: never said read, {} rows stored", rows(&live, "readings"));
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(rows(&live, "readings"), 10, "`{case}`: the new file read whole");
            std::fs::set_permissions(&away, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        for (id, r) in session.stop() {
            r.unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }
}
