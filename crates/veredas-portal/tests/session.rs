//! The session (contract: the session clause): a project's sources of the three protocols run as
//! workers, every row into one store alike, the ingest keeping what a shell and the server read,
//! the master switch arming every write path at once, and a start refused by name before
//! anything spawns. Against the bench's plant: its OPC UA server and its machine over real
//! sockets, and a real log the test writes.

mod common;

use std::collections::BTreeMap;
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
    let port = free_port();
    let bench = plant(dir.path(), port);
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
    let mut framed: BTreeMap<(String, String), Vec<(i64, f64, u64)>> = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while (framed.get(&log_temp).map_or(0, Vec::len) < 10 || rows(&live, "readings") < 30) && Instant::now() < deadline {
        if !drain(&mut rx, &mut framed) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert_eq!(framed.get(&log_temp), Some(&(20..30).map(|i| (stamp_ms(i), temp(i), i as u64 + 1)).collect::<Vec<_>>()));
    assert_eq!(rows(&live, "readings"), 30);

    // A stop ends every worker cleanly; the machine sees its peer go.
    let ended = session.stop();
    assert_eq!(ended.len(), 3);
    for (id, r) in ended {
        r.unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert!(wait(Duration::from_secs(5), || bench.channel("machine").unwrap().status().peers.is_empty()));

    // After the stop's last refresh: every series framed, from every source, counts one up from
    // frame to frame, and its last count is the cache's; the cache holds each series' exact type,
    // count, keys, last index and last value as the store holds them; the text series counted 30.
    while drain(&mut rx, &mut framed) {}
    let cache = live.series.read().unwrap();
    assert!(framed.keys().any(|(src, _)| src == "plc"), "{:?}", framed.keys());
    for ((src, ser), got) in &framed {
        let n: Vec<u64> = got.iter().map(|g| g.2).collect();
        assert!(n.windows(2).all(|w| w[1] == w[0] + 1), "{src}|{ser}: {n:?}");
        assert_eq!(n.last().copied(), Some(cache[&format!("{src}|{ser}")].count), "{src}|{ser}");
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

/// Read one frame, if one waits, into each series' samples as framed: stamp, value and count.
fn drain(rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>, framed: &mut BTreeMap<(String, String), Vec<(i64, f64, u64)>>) -> bool {
    match rx.try_recv() {
        Ok(frame) => {
            let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(v["type"], "data");
            for s in v["samples"].as_array().unwrap() {
                let key = (s["src"].as_str().unwrap().to_string(), s["ser"].as_str().unwrap().to_string());
                framed.entry(key).or_default().push((s["ms"].as_i64().unwrap(), s["v"].as_f64().unwrap(), s["n"].as_u64().unwrap()));
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
    clash.channels.push(channel_config(free_port()));
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
    let port = free_port();
    let bench = plant(dir.path(), port);
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
    let port = free_port();
    let bench = plant(dir.path(), port);
    let log = dir.path().join("keys.log");
    let mut text = String::new();
    for i in 0..70 {
        text.push_str(&format!("2026-07-19 10:00:00 temp=1.0 item=K{i} state=run\n"));
    }
    std::fs::write(&log, text).unwrap();
    let mut cfg = session_config("away", &log, &bench, port);
    cfg.start = Some(midnight());
    // The plant is gone before the session starts: the server and the peer are away.
    bench.stop();
    let session = Session::start(cfg).unwrap();
    let live = session.live();
    let mut frames = live.subscribe();
    assert!(wait(Duration::from_secs(10), || live.series.read().unwrap().get("log|temp").is_some_and(|e| e.count == 70)), "{:?}", live.series.read().unwrap().get("log|temp"));
    let temp = live.series.read().unwrap()["log|temp"].clone();
    assert_eq!((temp.keys, temp.keys_capped), (64, true), "seventy keys: the cache holds 64 and says so");
    // A connection attempt may take the worker's whole connect timeout (10 s) before it reads as
    // retrying (a port freed by the bench can be taken by another test's server, which then never
    // answers this client's handshake): the bound is that timeout and a margin, for a hang.
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
