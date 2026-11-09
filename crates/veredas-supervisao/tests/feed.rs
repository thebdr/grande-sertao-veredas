//! C-050: the object feed. Every object's value, state (a class with a mark and a word, by the
//! first rule that holds), staleness and marker place, exact: first on the fold itself with a
//! clock the test holds, then live, over the snapshot route and the stream, against a session fed
//! by the bench's OPC UA server and a log the test writes.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use veredas_supervisao::binding::Binding;
use veredas_supervisao::feed::{Fold, Item};
use veredas_supervisao::state::{Reading, StateClass};
use veredas_supervisao::{Host, config};

/// The example grown by what the reference's plant lacks: a keyed position, a text mode, a rule
/// over another series, and shorter freshness bounds for a test's clock.
fn grown(mut ex: Example) -> Example {
    ex.project.push_str("\n[[rule]]\nname = \"positions\"\npattern = \"POS item=(?P<item>\\\\w+) at=(?P<at>[0-9.]+)\"\nscope = \"sorter\"\ntypes = { at = \"float\" }\nkey = \"item\"\n\n[[rule]]\nname = \"modes\"\npattern = \"MODE (?P<mode>\\\\w+)\"\nscope = \"sorter\"\n");
    let extra = "\n[[object]]\nid = \"tracked\"\nlabel = \"Tracked\"\nvalue = { source = \"sorter\", series = \"at\" }\nshow = \"move\"\npath = \"loop\"\nstale_after_ms = 1500\n\n[[object]]\nid = \"tracked-a1\"\nlabel = \"A1\"\nvalue = { source = \"sorter\", series = \"at\", key = \"A1\" }\nshow = \"move\"\npath = \"loop\"\n\n[[object]]\nid = \"at-b2\"\nlabel = \"B2 at\"\nvalue = { source = \"sorter\", series = \"at\", key = \"B2\" }\nstates = [ { above = 40, state = \"warn\", word = \"far\" } ]\n\n[[object]]\nid = \"mode\"\nlabel = \"Mode\"\nvalue = { source = \"sorter\", series = \"mode\" }\nstates = [ { equals = \"jam\", state = \"warn\", word = \"jammed\" } ]\nnormal = \"running\"\n\n[[object]]\nid = \"outlet-9\"\nlabel = \"Outlet 9\"\nvalue = { source = \"plc\", series = \"fill1\" }\nstates = [ { equals = 2, series = \"infeed\", state = \"alarm\", word = \"infeed fault\" }, { at_least = 100, state = \"warn\", word = \"full\" } ]\n\n[[dashboard]]";
    ex.binding = ex.binding.replacen("\n[[dashboard]]", extra, 1).replace("stale_after_ms = 30000", "stale_after_ms = 1500");
    ex
}

fn binding_of(ex: &Example) -> Binding {
    let dir = tempfile::tempdir().unwrap();
    config::load_file(&write_example(dir.path(), ex)).unwrap().sessions.remove(0).binding
}

#[test]
fn the_fold_reads_every_object_exactly() {
    let b = binding_of(&grown(example()));
    let o = |id: &str| b.object(id).unwrap();
    let mut f = Fold::new(&b);

    // Nothing arrived: no value, read as stale.
    let v = f.view(o("outlet-1"), true, 0);
    assert_eq!((v.value, v.text, v.state, v.word.as_str(), v.stale, v.since_ms, v.level), (None, None, StateClass::Stale, "no value", true, None, None));

    // A level: its fill by the value's place in the range, clamped; its state by the first rule.
    for (x, text, state, word, level) in [
        (35.0, "35", StateClass::Normal, "ok", 0.35),
        (80.0, "80", StateClass::Near, "near full", 0.8),
        (85.25, "85", StateClass::Near, "near full", 0.8525),
        (100.0, "100", StateClass::Warn, "full", 1.0),
        (120.0, "120", StateClass::Warn, "full", 1.0),
        (-5.0, "-5", StateClass::Normal, "ok", 0.0),
    ] {
        f.sample("plc", "fill1", None, 1_000, x, 5_000);
        let v = f.view(o("outlet-1"), true, 9_000);
        assert_eq!((v.value, v.text.as_deref(), v.state, v.word.as_str(), v.stale, v.since_ms, v.level), (Some(Reading::Num(x)), Some(text), state, word, false, Some(5_000), Some(level)), "{x}");
    }
    // An older sample never replaces a newer one; an equal index does.
    f.sample("plc", "fill1", None, 999, 50.0, 6_000);
    assert_eq!(f.view(o("outlet-1"), true, 9_000).value, Some(Reading::Num(-5.0)));
    f.sample("plc", "fill1", None, 1_000, 60.0, 6_000);
    assert_eq!(f.view(o("outlet-1"), true, 9_000).value, Some(Reading::Num(60.0)));
    // Its source not live: stale, the last value kept.
    let v = f.view(o("outlet-1"), false, 9_000);
    assert_eq!((v.value, v.text.as_deref(), v.state, v.word.as_str(), v.stale), (Some(Reading::Num(60.0)), Some("60"), StateClass::Stale, "stale", true));

    // A code read by equals; anything no rule names is normal, with the object's word.
    for (x, state, word) in [(0.0, StateClass::Warn, "stopped"), (2.0, StateClass::Alarm, "fault"), (1.0, StateClass::Normal, "running"), (1.5, StateClass::Normal, "running")] {
        f.sample("plc", "infeed", None, 2_000, x, 2_000);
        let v = f.view(o("infeed"), true, 2_000);
        assert_eq!((v.state, v.word.as_str()), (state, word), "{x}");
    }
    // A rule over another series: the outlet reads alarm while the infeed reads 2, whatever its fill.
    f.sample("plc", "infeed", None, 3_000, 2.0, 3_000);
    f.sample("plc", "fill1", None, 3_000, 100.0, 3_000);
    assert_eq!((f.view(o("outlet-9"), true, 3_000).state, f.view(o("outlet-9"), true, 3_000).word), (StateClass::Alarm, "infeed fault".to_string()));
    f.sample("plc", "infeed", None, 3_001, 1.0, 3_001);
    assert_eq!((f.view(o("outlet-9"), true, 3_001).state, f.view(o("outlet-9"), true, 3_001).word), (StateClass::Warn, "full".to_string()));

    // Decimals as the object says; the rule below.
    f.sample("plc", "speed", None, 1, 0.44, 1);
    let v = f.view(o("speed"), true, 1);
    assert_eq!((v.text.as_deref(), v.state, v.word.as_str()), (Some("0.4"), StateClass::Warn, "slow"));
    f.sample("plc", "speed", None, 2, 1.5, 2);
    assert_eq!((f.view(o("speed"), true, 2).text.as_deref(), f.view(o("speed"), true, 2).word.as_str()), (Some("1.5"), "running"));

    // A freshness bound: fresh at the bound, stale past it, though the source is live.
    f.sample("sorter", "rate", None, 10, 97.5, 10_000);
    let v = f.view(o("read-rate"), true, 11_500);
    assert_eq!((v.state, v.word.as_str(), v.text.as_deref(), v.stale), (StateClass::Alarm, "under 98 %", Some("97.5"), false));
    let v = f.view(o("read-rate"), true, 11_501);
    assert_eq!((v.state, v.word.as_str(), v.text.as_deref(), v.stale), (StateClass::Stale, "stale", Some("97.5"), true));

    // A marker by its place along the path, from a data frame as the session broadcasts it; a
    // series no object reads, and a frame that is no frame, change nothing.
    f.frame(r#"{"type":"data","samples":[{"src":"plc","ser":"p1","k":null,"ms":5,"v":25.0},{"src":"plc","ser":"zz","k":null,"ms":5,"v":1.0}]}"#, 100);
    f.frame("not json", 100);
    f.frame(r#"{"type":"data","samples":[{"src":"plc","ser":"p1","ms":6}]}"#, 100);
    let v = f.view(o("parcel-1"), true, 100);
    assert_eq!((v.place, v.level, v.items.len(), v.state), (Some(0.25), None, 0, StateClass::Normal));
    f.frame(r#"{"type":"data","samples":[{"src":"plc","ser":"p1","k":null,"ms":7,"v":130.0}]}"#, 100);
    assert_eq!(f.view(o("parcel-1"), true, 100).place, Some(1.0));
    assert_eq!(f.view(o("parcel-2"), true, 100).place, None, "no value, no place");

    // Markers per key: every key with its place, forgotten past the bound.
    f.frame(r#"{"type":"data","samples":[{"src":"sorter","ser":"at","k":"B2","ms":1,"v":50.0},{"src":"sorter","ser":"at","k":"A1","ms":1,"v":10.0}]}"#, 20_000);
    let v = f.view(o("tracked"), true, 20_000);
    assert_eq!((v.place, v.items.clone()), (None, vec![Item { key: "A1".into(), place: 0.1 }, Item { key: "B2".into(), place: 0.5 }]));
    f.sample("sorter", "at", Some("A1"), 2, 20.0, 21_000);
    assert_eq!(f.view(o("tracked"), true, 21_501).items, vec![Item { key: "A1".into(), place: 0.2 }], "B2 last moved 1501 ms ago");
    assert_eq!(f.view(o("tracked"), true, 22_501).items, vec![], "both forgotten");
    // A key's older sample never replaces its newer one.
    f.sample("sorter", "at", Some("A1"), 1, 90.0, 22_600);
    assert_eq!(f.view(o("tracked"), true, 22_600).items, vec![], "an older sample is not a new arrival");
    // Bound to one key, a marker is one place, that key's; a value is that key's, whatever the
    // other keys did since, and never forgotten while its source is live.
    let v = f.view(o("tracked-a1"), true, 90_000);
    assert_eq!((v.place, v.items.clone(), v.text.as_deref()), (Some(0.2), vec![], Some("20")));
    let v = f.view(o("at-b2"), true, 90_000);
    assert_eq!((v.text.as_deref(), v.state, v.word.as_str(), v.stale), (Some("50"), StateClass::Warn, "far", false));
    f.sample("sorter", "at", Some("C3"), 9, 99.0, 90_000);
    assert_eq!((f.view(o("at-b2"), true, 90_000).text.as_deref(), f.view(o("tracked-a1"), true, 90_000).place), (Some("50"), Some(0.2)));
    assert_eq!(f.view(o("tracked"), true, 90_000).items, vec![Item { key: "C3".into(), place: 0.99 }]);

    // Text arrives through the ingest's cache: compared as text, exactly; a newer cached value of
    // a numeric series heals a missed frame; an older one is ignored.
    f.cached("sorter", "mode", 5, "jam", 30_000);
    let v = f.view(o("mode"), true, 30_000);
    assert_eq!((v.value, v.text.as_deref(), v.state, v.word.as_str()), (Some(Reading::Text("jam".into())), Some("jam"), StateClass::Warn, "jammed"));
    f.cached("sorter", "mode", 6, "Jam", 30_001);
    assert_eq!(f.view(o("mode"), true, 30_001).state, StateClass::Normal);
    f.cached("plc", "speed", 3, "1.75", 30_002);
    assert_eq!(f.view(o("speed"), true, 30_002).value, Some(Reading::Num(1.75)));
    f.cached("plc", "speed", 2, "9", 30_003);
    assert_eq!(f.view(o("speed"), true, 30_003).value, Some(Reading::Num(1.75)));

    // Away, by the source's health: waiting is away only once the source has delivered (portal
    // reads an OPC source reconnecting, and a channel whose peer left, as waiting).
    use veredas_portal::session::HealthState;
    use veredas_supervisao::feed::away;
    for (state, before_first, after_first) in [
        (HealthState::Live, false, false),
        (HealthState::Waiting, false, true),
        (HealthState::Idle { silence_ms: 25_000 }, true, true),
        (HealthState::Retrying("refused".into()), true, true),
        (HealthState::Ended, true, true),
        (HealthState::Failed("gone".into()), true, true),
    ] {
        assert_eq!((away(&state, 0), away(&state, 1)), (before_first, after_first), "{state:?}");
    }

    // The page's form of a view.
    f.sample("plc", "fill2", None, 1, 85.0, 40_000);
    assert_eq!(f.view(o("outlet-2"), true, 40_000).to_json(), json!({
        "id": "outlet-2", "value": 85.0, "text": "85", "state": "near", "mark": "\u{25A0}", "word": "near full", "stale": false,
        "since_ms": 40_000, "level": 0.85, "place": null, "items": [],
    }));
    assert_eq!(f.view(o("tracked"), true, 100_000).to_json()["items"], json!([]));
    assert_eq!(f.view(o("mode"), true, 30_001).to_json()["value"], json!("Jam"));
    assert_eq!([StateClass::Normal, StateClass::Near, StateClass::Warn, StateClass::Alarm, StateClass::Disabled, StateClass::Stale].map(|c| c.mark()), ["\u{25CF}", "\u{25A0}", "\u{25A0}", "!", "\u{25A8}", "\u{25CC}"]);
}

fn snapshot(addr: std::net::SocketAddr) -> Value {
    let (status, v) = get(addr, "/s/loop/objects.json");
    assert_eq!(status, 200);
    v
}

#[test]
fn the_feed_follows_the_plant_live_over_the_snapshot_and_the_stream() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let path = write_example(dir.path(), &grown(live_example(&bench, false)));
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();

    // The plant as the bench holds it, every object as the rules read it.
    assert!(wait(Duration::from_secs(20), || {
        let s = snapshot(addr);
        object(&s, "parcel-2")["place"] == json!(0.75) && object(&s, "outlet-3")["text"] == json!("100") && object(&s, "outlet-1")["state"] == json!("normal") && s["session"]["health"]["plc"]["state"] == json!("live")
    }), "{}", snapshot(addr));
    let s = snapshot(addr);
    let pick = |id: &str| { let o = object(&s, id); (o["text"].clone(), o["state"].clone(), o["word"].clone(), o["mark"].clone(), o["stale"].clone()) };
    assert_eq!(pick("outlet-1"), (json!("35"), json!("normal"), json!("ok"), json!("\u{25CF}"), json!(false)));
    assert_eq!(pick("outlet-2"), (json!("85"), json!("near"), json!("near full"), json!("\u{25A0}"), json!(false)));
    assert_eq!(pick("outlet-3"), (json!("100"), json!("warn"), json!("full"), json!("\u{25A0}"), json!(false)));
    assert_eq!(pick("infeed"), (json!("1"), json!("normal"), json!("running"), json!("\u{25CF}"), json!(false)));
    assert_eq!(pick("speed"), (json!("1.5"), json!("normal"), json!("running"), json!("\u{25CF}"), json!(false)));
    assert_eq!(pick("read-rate"), (Value::Null, json!("stale"), json!("no value"), json!("\u{25CC}"), json!(true)));
    assert_eq!((object(&s, "outlet-1")["level"].clone(), object(&s, "outlet-3")["level"].clone(), object(&s, "parcel-1")["place"].clone()), (json!(0.35), json!(1.0), json!(0.25)));
    assert_eq!((s["session"]["running"].clone(), s["session"]["writes_armed"].clone(), s["session"]["health"]["plc"]["state"].clone()), (json!(true), json!(false), json!("live")));
    assert!(s["session"]["now_ms"].as_i64().unwrap() > 0);

    // The stream: a snapshot first, then what changes, and the session's status every second.
    let mut sse = Sse::open(addr, "/s/loop/objects");
    assert_eq!(sse.status, 200);
    assert!(sse.headers.contains("content-type: text/event-stream"), "{}", sse.headers);
    let (name, data) = sse.next(Duration::from_secs(10)).unwrap();
    assert_eq!(name, "snapshot");
    let first: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(first["objects"].as_array().unwrap().len(), 13);
    assert_eq!(object(&first, "outlet-2")["word"], json!("near full"));
    let got = sse.until(Duration::from_secs(5), |n, _| n == "session").expect("a status within the second");
    let (_, st) = got.last().unwrap();
    assert_eq!((st["running"].clone(), st["writes_armed"].clone(), st["health"]["plc"]["state"].clone(), st["health"]["plc"]["detail"].clone()), (json!(true), json!(false), json!("live"), Value::Null));
    assert!((st["now_ms"].as_i64().unwrap() - store_now()).abs() < 5_000, "the store's now: {st}");
    assert!(st["health"].get("sorter").is_some(), "every source's health: {st}");
    // Armed in process, the next status says so.
    host.session("loop").unwrap().set_writes_armed(true);
    assert!(sse.until(Duration::from_secs(5), |n, v| n == "session" && v["writes_armed"] == json!(true)).is_some(), "the switch rides the stream");
    host.session("loop").unwrap().set_writes_armed(false);
    bench.server("plc").unwrap().set("infeed", 2);
    let got = sse.until(Duration::from_secs(20), |n, v| n == "objects" && v["objects"].as_array().unwrap().iter().any(|o| o["id"] == "infeed" && o["state"] == "alarm")).expect("the fault arrives as a change");
    let (_, last) = got.last().unwrap();
    let infeed = last["objects"].as_array().unwrap().iter().find(|o| o["id"] == "infeed").unwrap();
    assert_eq!((infeed["word"].clone(), infeed["mark"].clone(), infeed["text"].clone()), (json!("fault"), json!("!"), json!("2")));
    // Only what changed rides a change: the fill did not move.
    assert!(got.iter().filter(|(n, _)| n == "objects").all(|(_, v)| v["objects"].as_array().unwrap().iter().all(|o| o["id"] != "outlet-2")));

    // The log: a rate under its rule, two keyed markers, a text mode.
    let now = store_now();
    append(&log, &[
        format!("{} SORT outlet=1 travel=12.5 rate=97.5", stamp(now)),
        format!("{} POS item=A1 at=10", stamp(now)),
        format!("{} POS item=B2 at=50", stamp(now)),
        format!("{} MODE jam", stamp(now)),
    ]);
    assert!(wait(Duration::from_secs(20), || {
        let s = snapshot(addr);
        object(&s, "mode")["word"] == json!("jammed") && object(&s, "tracked")["items"].as_array().unwrap().len() == 2 && object(&s, "read-rate")["state"] == json!("alarm")
    }), "{}", snapshot(addr));
    let s = snapshot(addr);
    assert_eq!(object(&s, "tracked")["items"], json!([{ "key": "A1", "place": 0.1 }, { "key": "B2", "place": 0.5 }]));
    assert_eq!((object(&s, "read-rate")["text"].clone(), object(&s, "read-rate")["word"].clone()), (json!("97.5"), json!("under 98 %")));
    assert_eq!((object(&s, "mode")["text"].clone(), object(&s, "mode")["state"].clone()), (json!("jam"), json!("warn")));
    // No line for longer than the bound: the rate goes stale with its value kept, while its
    // source still reads live; the markers are forgotten.
    assert!(wait(Duration::from_secs(10), || {
        let s = snapshot(addr);
        object(&s, "read-rate")["state"] == json!("stale") && object(&s, "tracked")["items"] == json!([])
    }), "{}", snapshot(addr));
    let s = snapshot(addr);
    assert_eq!((object(&s, "read-rate")["text"].clone(), object(&s, "read-rate")["word"].clone(), s["session"]["health"]["sorter"]["state"].clone()), (json!("97.5"), json!("stale"), json!("live")));

    // The plant away: every object of its source stale, its last value kept.
    bench.stop();
    assert!(wait(Duration::from_secs(30), || object(&snapshot(addr), "outlet-1")["stale"] == json!(true)), "{}", snapshot(addr));
    let s = snapshot(addr);
    assert_eq!((object(&s, "outlet-1")["text"].clone(), object(&s, "outlet-1")["word"].clone(), object(&s, "outlet-1")["state"].clone()), (json!("35"), json!("stale"), json!("stale")));
    assert_ne!(s["session"]["health"]["plc"]["state"], json!("live"));
    // And it stays stale while the source tries again, retrying or waiting on a reconnect.
    let mut states = std::collections::BTreeSet::new();
    for _ in 0..16 {
        std::thread::sleep(Duration::from_millis(250));
        let s = snapshot(addr);
        assert_eq!(object(&s, "outlet-1")["stale"], json!(true), "{s}");
        states.insert(s["session"]["health"]["plc"]["state"].as_str().unwrap().to_string());
    }
    assert!(states.iter().all(|x| x == "retrying" || x == "waiting"), "{states:?}");
    host.stop();
}

// ---- the vocabulary alone: numbers as shown, rules over their kind of value (src/state.rs) ----

use veredas_supervisao::state::{Comparand, Compare, format_number};

#[test]
fn numbers_read_as_they_are_shown() {
    assert_eq!(format_number(35.0, None), "35");
    assert_eq!(format_number(2.4, None), "2.4");
    assert_eq!(format_number(1.23456, None), "1.235");
    assert_eq!(format_number(1.5, Some(0)), "2");
    assert_eq!(format_number(97.94, Some(1)), "97.9");
    assert_eq!(format_number(-0.0004, None), "0");
    assert_eq!(format_number(-0.04, Some(1)), "0.0");
    assert_eq!(format_number(-2.5, None), "-2.5");
    assert_eq!(format_number(8640.0, None), "8640");
}

#[test]
fn a_rule_holds_only_over_its_kind_of_value() {
    assert!(Compare::Equals(Comparand::Num(2.0)).holds(&Reading::Num(2.0)));
    assert!(!Compare::Equals(Comparand::Num(2.0)).holds(&Reading::Text("2".into())));
    assert!(Compare::Equals(Comparand::Text("run".into())).holds(&Reading::Text("run".into())));
    assert!(!Compare::Equals(Comparand::Text("run".into())).holds(&Reading::Text("Run".into())));
    assert!(Compare::AtLeast(100.0).holds(&Reading::Num(100.0)));
    assert!(!Compare::Above(100.0).holds(&Reading::Num(100.0)));
    assert!(Compare::Below(98.0).holds(&Reading::Num(97.9)));
    assert!(Compare::AtMost(0.0).holds(&Reading::Num(0.0)));
    assert!(!Compare::Below(98.0).holds(&Reading::Text("97".into())));
    assert_eq!(Compare::Below(98.0).describe("%"), "below 98 %");
    assert_eq!(Compare::Above(2.0).describe(""), "above 2");
}
