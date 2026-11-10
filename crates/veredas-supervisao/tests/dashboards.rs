//! C-052: the dashboards. Every panel kind computed by the host from the session's store, exactly:
//! a log the test writes with stamps it chooses (on the store's axis, aligned on five minutes)
//! and the bench's fill levels, read back over the dashboard route with the window's right edge
//! named, so every bucket, breach, segment, median and bin is known before the test runs (a
//! downsample's very points, a state held from a day before, two objects on one timeline); then
//! the page itself, run in a headless browser, read back against the dashboard it fetched, every
//! mark's extent with it (a capped panel's note among them); and the cells read once each while
//! rows arrive.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::browser::Browser;
use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, config};

const MIN: i64 = 60_000;

const BINDING: &str = r#"format = "veredas-binding"
version = 1
title = "Numbers"

[[object]]
id = "rate"
label = "Read rate"
value = { source = "sorter", series = "rate" }
unit = "%"
decimals = 1

[[object]]
id = "travel"
label = "Travel"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 15, state = "warn", word = "slow" } ]

[[object]]
id = "low-rate"
label = "Low rate"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { below = 99.5, state = "alarm", word = "under 99.5 %" } ]

[[object]]
id = "rate-fresh"
label = "Fresh rate"
value = { source = "sorter", series = "rate" }
unit = "%"
stale_after_ms = 1000

[[object]]
id = "line"
label = "Line"
value = { source = "sorter", series = "code" }
states = [ { equals = 0, state = "warn", word = "stopped" }, { equals = 2, state = "alarm", word = "fault" } ]
normal = "running"

[[object]]
id = "outlet-1"
label = "Outlet 1"
value = { source = "plc", series = "fill1" }
unit = "%"
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-2"
label = "Outlet 2"
value = { source = "plc", series = "fill2" }
unit = "%"
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-3"
label = "Outlet 3"
value = { source = "plc", series = "fill3" }
unit = "%"
show = "level"
range = [50, 150]
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "over"
label = "Over"
value = { source = "plc", series = "fill3" }
unit = "%"
show = "level"
range = [50, 85]

[[object]]
id = "dense"
label = "Dense"
value = { source = "sorter", series = "dv" }

[[dashboard]]
name = "Numbers"
columns = 4

[[dashboard.panel]]
kind = "value"
object = "rate"
target = 98

[[dashboard.panel]]
kind = "value"
object = "travel"

[[dashboard.panel]]
kind = "value"
object = "low-rate"

[[dashboard.panel]]
kind = "value"
object = "rate-fresh"

[[dashboard.panel]]
kind = "trend"
object = "rate"
limit = { below = 98 }

[[dashboard.panel]]
kind = "bars"
object = "travel"
target = 12

[[dashboard.panel]]
kind = "timeline"
objects = ["line", "travel"]

[[dashboard.panel]]
kind = "grid"
objects = ["outlet-*"]

[[dashboard.panel]]
kind = "levels"
objects = ["outlet-*", "over"]
ticks = [80, 90]

[[dashboard.panel]]
kind = "table"
table = "outlets"
columns = [ { name = "outlet" }, { name = "sorted", bar = true }, { name = "travel", decimals = 1, states = [ { above = 14, state = "warn", word = "slow" } ] } ]
sort = { column = "travel", descending = true }

[[dashboard.panel]]
kind = "cells"
table = "sorts"
key = "outlet"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "outlets"
columns = [ { name = "nothing" } ]

[[dashboard.panel]]
kind = "trend"
object = "travel"
limit = { above = 16 }

[[dashboard.panel]]
kind = "value"
object = "rate"
target = 100

[[dashboard.panel]]
kind = "trend"
object = "dense"
limit = { above = 50 }

[[dashboard.panel]]
kind = "value"
object = "dense"
"#;

/// The lines, `(minutes past the base, text)`: sorts with their outlet, travel and rate, and the
/// line's state code.
const LINES: &[(f64, &str)] = &[
    (0.0, "SORT outlet=1 travel=10.0 rate=99.0"),
    (0.0, "STATE code=1"),
    (2.0, "SORT outlet=2 travel=12.0 rate=97.0"),
    (4.0, "SORT outlet=1 travel=11.0 rate=98.5"),
    (6.0, "SORT outlet=2 travel=14.0 rate=96.5"),
    (6.0, "STATE code=0"),
    (10.0, "SORT outlet=3 travel=20.0 rate=99.5"),
    (10.0, "STATE code=0"),
    (14.0, "SORT outlet=1 travel=13.0 rate=99.0"),
    (14.0, "STATE code=2"),
    (20.0, "SORT outlet=2 travel=16.0 rate=98.0"),
    (29.0, "SORT outlet=1 travel=12.0 rate=97.5"),
    (29.0, "STATE code=1"),
    (30.5, "SORT outlet=3 travel=21.0 rate=99.2"),
];

fn panel(d: &Value, i: usize) -> &Value {
    assert!(d["panels"][i]["error"].is_null(), "panel {i}: {}", d["panels"][i]);
    &d["panels"][i]["data"]
}

#[test]
fn every_panel_kind_is_computed_exactly_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    ex.project = ex.project.replace("writes_armed = false", "writes_armed = false\nidle_after_ms = 600000");
    ex.project.push_str("\n[[rule]]\nname = \"states\"\npattern = \"STATE code=(?P<code>\\\\d+)\"\nscope = \"sorter\"\ntypes = { code = \"int\" }\n");
    ex.project.push_str("\n[[rule]]\nname = \"dense\"\npattern = \"DENSE v=(?P<dv>[0-9.]+)\"\nscope = \"sorter\"\ntypes = { dv = \"float\" }\n");
    ex.binding = BINDING.to_string();
    let path = write_example(dir.path(), &ex);
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    let live = host.session("loop").unwrap().live();
    assert!(wait(Duration::from_secs(20), || live.health.read().unwrap().get("sorter").is_some_and(|h| h.count == 0)), "the log is attached");

    // The base: five minutes aligned, half an hour back; the window ends 31 minutes past it.
    let base = (store_now() / (5 * MIN)) * (5 * MIN) - 30 * MIN;
    let at = |m: f64| base + (m * MIN as f64) as i64;
    let appended = std::time::Instant::now();
    append(&log, &LINES.iter().map(|(m, l)| format!("{} {l}", stamp(at(*m)))).collect::<Vec<_>>());
    // And a dense series: 400 samples a second apart, five neighbours past its limit.
    let dense_at = |i: i64| base + i * 1000;
    let dense_v = |i: i64| if (200..205).contains(&i) { 60.0 } else { 10.0 };
    append(&log, &(0..400).map(|i| format!("{} DENSE v={}", stamp(dense_at(i)), dense_v(i))).collect::<Vec<_>>());
    let to = at(31.0);
    let rows = |t: &str| live.store.query_rows(t, &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || rows("sorts") == 9 && rows("states") == 5 && rows("dense") == 400), "the lines are stored");
    let get_d = || get(addr, &format!("/s/loop/dashboard/0?to={to}")).1;
    // The feed's view of the latest values, and the fills, before the tiles are read.
    assert!(wait(Duration::from_secs(20), || {
        let d = get_d();
        panel(&d, 0)["text"] == json!("99.2") && panel(&d, 7)["cells"][2]["text"] == json!("100")
    }), "{}", get_d());
    let d = get_d();
    assert_eq!((d["name"].clone(), d["columns"].clone(), d["to"].clone(), d["panels"].as_array().unwrap().len()), (json!("Numbers"), json!(4), json!(to), 16));
    let heads: Vec<(String, String, i64)> = d["panels"].as_array().unwrap().iter().map(|p| (p["kind"].as_str().unwrap().to_string(), p["title"].as_str().unwrap().to_string(), p["span"].as_i64().unwrap())).collect();
    assert_eq!(heads[..4].iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), ["Read rate", "Travel", "Low rate", "Fresh rate"]);
    assert_eq!(heads[4..].iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["trend", "bars", "timeline", "grid", "levels", "table", "cells", "table", "trend", "value", "trend", "value"]);
    // A dense trend: its total and breaches from every point of the window, its line downsampled
    // to the trend's points.
    let dn = panel(&d, 14);
    assert_eq!((dn["total"].clone(), dn["points"].as_array().unwrap().len()), (json!(400), veredas_supervisao::panel::TREND_POINTS));
    assert_eq!(dn["breaches"], json!((200..205).map(|i| json!([dense_at(i), 60.0])).collect::<Vec<_>>()), "every breach, the neighbours too");
    // Which points: the window's own, downsampled as portal downsamples (its ends and its peak kept).
    let raw: Vec<(i64, f64)> = (0..400).map(|i| (dense_at(i), dense_v(i))).collect();
    let line = veredas_portal::serve::lttb::downsample(&raw, veredas_supervisao::panel::TREND_POINTS);
    assert_eq!(dn["points"], json!(line.iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>()));
    assert_eq!((dn["points"][0].clone(), dn["points"][299].clone()), (json!([dense_at(0), 10.0]), json!([dense_at(399), 10.0])));
    assert!(dn["points"].as_array().unwrap().iter().any(|p| p[1] == json!(60.0)), "the peak kept");
    // A value tile over it: its last half hour (the samples from a minute past the base on) in
    // sixty points, its ends and its peak among them.
    let dt = panel(&d, 15);
    let half: Vec<(i64, f64)> = raw.iter().copied().filter(|(t, _)| *t >= to - 30 * MIN).collect();
    assert_eq!(half.len(), 340);
    let spark = veredas_portal::serve::lttb::downsample(&half, veredas_supervisao::panel::SPARK_POINTS);
    assert_eq!(dt["spark"], json!(spark.iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>()));
    assert_eq!((dt["spark"].as_array().unwrap().len(), dt["spark"][0].clone(), dt["spark"][59].clone()), (60, json!([dense_at(60), 10.0]), json!([dense_at(399), 10.0])));
    assert!(dt["spark"].as_array().unwrap().iter().any(|p| p[1] == json!(60.0)), "the peak kept");

    // Value tiles: normal with its target, warn, alarm; each with its sub-line and its sparkline
    // of the last 30 minutes (eight lines of the nine fall in it).
    let t = panel(&d, 0);
    assert_eq!((t["text"].clone(), t["unit"].clone(), t["state"].clone(), t["word"].clone(), t["mark"].clone(), t["stale"].clone(), t["sub"].clone(), t["target"].clone()), (json!("99.2"), json!("%"), json!("normal"), json!("ok"), json!("\u{25CF}"), json!(false), json!("+1% on the 98 target"), json!(98.0)));
    let spark: Vec<f64> = t["spark"].as_array().unwrap().iter().map(|p| p[1].as_f64().unwrap()).collect();
    assert_eq!(spark, [97.0, 98.5, 96.5, 99.5, 99.0, 98.0, 97.5, 99.2]);
    assert_eq!((t["spark"][0][0].clone(), t["from"].clone(), t["to"].clone()), (json!(at(2.0)), json!(to - 30 * MIN), json!(to)));
    let t = panel(&d, 1);
    assert_eq!((t["text"].clone(), t["state"].clone(), t["word"].clone(), t["sub"].clone()), (json!("21"), json!("warn"), json!("slow"), json!("above 15 s")));
    let t = panel(&d, 2);
    assert_eq!((t["text"].clone(), t["state"].clone(), t["word"].clone(), t["mark"].clone(), t["sub"].clone()), (json!("99.2"), json!("alarm"), json!("under 99.5 %"), json!("!"), json!("below 99.5 %")));

    // The trend: every point of the hour, the limit, the three points under it.
    let tr = panel(&d, 4);
    assert_eq!((tr["from"].clone(), tr["to"].clone(), tr["total"].clone(), tr["limit"].clone(), tr["unit"].clone()), (json!(to - 60 * MIN), json!(to), json!(9), json!({ "below": 98.0 }), json!("%")));
    assert_eq!(tr["points"].as_array().unwrap().iter().map(|p| p[1].as_f64().unwrap()).collect::<Vec<_>>(), [99.0, 97.0, 98.5, 96.5, 99.5, 99.0, 98.0, 97.5, 99.2]);
    assert_eq!(tr["breaches"], json!([[at(2.0), 97.0], [at(6.0), 96.5], [at(29.0), 97.5]]));

    // The bars: thirteen buckets of five minutes on the clock, the hour's first six empty, the
    // last running; the mean of each, its count, the target.
    let b = panel(&d, 5);
    assert_eq!((b["from"].clone(), b["to"].clone(), b["target"].clone(), b["unit"].clone()), (json!(base - 30 * MIN), json!(to), json!(12.0), json!("s")));
    let buckets = b["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 13);
    for (i, bk) in buckets.iter().enumerate() {
        assert_eq!((bk["start"].clone(), bk["end"].clone()), (json!(base + (i as i64 - 6) * 5 * MIN), json!(base + (i as i64 - 5) * 5 * MIN)), "{i}");
    }
    let got: Vec<(Value, Value, Value)> = buckets.iter().skip(6).map(|bk| (bk["value"].clone(), bk["n"].clone(), bk["running"].clone())).collect();
    assert_eq!(got, [
        (json!(11.0), json!(3), json!(false)),
        (json!(14.0), json!(1), json!(false)),
        (json!(16.5), json!(2), json!(false)),
        (Value::Null, json!(0), json!(false)),
        (json!(16.0), json!(1), json!(false)),
        (json!(12.0), json!(1), json!(false)),
        (json!(21.0), json!(1), json!(true)),
    ]);
    assert!(buckets.iter().take(6).all(|bk| bk["value"].is_null() && bk["n"] == json!(0)));

    // State over time: one run per change of state, the last until the window's edge.
    let tl = panel(&d, 6);
    assert_eq!((tl["from"].clone(), tl["to"].clone()), (json!(to - 60 * MIN), json!(to)));
    assert_eq!(tl["rows"][0]["segments"], json!([
        { "from": at(0.0), "to": at(6.0), "state": "normal", "mark": "\u{25CF}", "word": "running" },
        { "from": at(6.0), "to": at(14.0), "state": "warn", "mark": "\u{25A0}", "word": "stopped" },
        { "from": at(14.0), "to": at(29.0), "state": "alarm", "mark": "!", "word": "fault" },
        { "from": at(29.0), "to": to, "state": "normal", "mark": "\u{25CF}", "word": "running" },
    ]));
    assert_eq!((tl["rows"][0]["id"].clone(), tl["rows"][0]["label"].clone(), tl["rows"][0]["now"]["word"].clone()), (json!("line"), json!("Line"), json!("running")));
    // A second object on the same timeline, its own runs: the travel slow past 15 s.
    let ok = |a: f64, b: Value| json!({ "from": at(a), "to": b, "state": "normal", "mark": "\u{25CF}", "word": "ok" });
    let slow = |a: f64, b: Value| json!({ "from": at(a), "to": b, "state": "warn", "mark": "\u{25A0}", "word": "slow" });
    assert_eq!(tl["rows"][1]["segments"], json!([ok(0.0, json!(at(10.0))), slow(10.0, json!(at(14.0))), ok(14.0, json!(at(20.0))), slow(20.0, json!(at(29.0))), ok(29.0, json!(at(30.5))), slow(30.5, json!(to))]));
    assert_eq!((tl["rows"][1]["id"].clone(), tl["rows"][1]["now"]["word"].clone()), (json!("travel"), json!("slow")));

    // The grid and the levels: the objects as the feed reads them, each level its place in its
    // own range, the ticks placed on it.
    let cells: Vec<(Value, Value, Value)> = panel(&d, 7)["cells"].as_array().unwrap().iter().map(|c| (c["text"].clone(), c["state"].clone(), c["word"].clone())).collect();
    assert_eq!(cells, [(json!("35"), json!("normal"), json!("ok")), (json!("85"), json!("near"), json!("near full")), (json!("100"), json!("warn"), json!("full"))]);
    let levels: Vec<(Value, Value, Value)> = panel(&d, 8)["rows"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["fraction"].clone(), r["ticks"].clone())).collect();
    assert_eq!(levels, [
        (json!("outlet-1"), json!(0.35), json!([0.8, 0.9])),
        (json!("outlet-2"), json!(0.85), json!([0.8, 0.9])),
        (json!("outlet-3"), json!(0.5), json!([0.3, 0.4])),
        // A value and a tick past the range's end, held at its end.
        (json!("over"), json!(1.0), json!([30.0 / 35.0, 1.0])),
    ]);

    // The table: the view's rows sorted by the count, the count drawn as a bar against the
    // largest shown, the travel to one place with its state word.
    let tb = panel(&d, 9);
    assert_eq!((tb["total"].clone(), tb["columns"].clone()), (json!(3), json!([{ "name": "outlet", "label": "outlet", "bar": false }, { "name": "sorted", "label": "sorted", "bar": true }, { "name": "travel", "label": "travel", "bar": false }])));
    let shown: Vec<(Value, Value, Value, Value, Value)> = tb["rows"].as_array().unwrap().iter().map(|r| (r[0]["text"].clone(), r[1]["text"].clone(), r[1]["bar"].clone(), r[2]["text"].clone(), r[2]["word"].clone())).collect();
    // Sorted by travel, descending: an order the view's own (by outlet) does not give.
    assert_eq!(shown, [
        (json!("3"), json!("2"), json!(0.5), json!("20.5"), json!("slow")),
        (json!("2"), json!("3"), json!(0.75), json!("14.0"), json!("")),
        (json!("1"), json!("4"), json!(1.0), json!("11.5"), json!("")),
    ]);
    assert_eq!((tb["rows"][0][2]["state"].clone(), tb["rows"][0][2]["mark"].clone(), tb["rows"][2][2]["state"].clone()), (json!("warn"), json!("\u{25A0}"), json!("normal")));

    // Charts in cells: per outlet the median, σ and n over the hour, the median per bucket, and
    // the distribution in twenty bins from median − 5 to median + 5.
    let c = panel(&d, 10);
    assert_eq!((c["from"].clone(), c["to"].clone(), c["spread"].clone(), c["read"].clone(), c["total"].clone(), c["capped"].clone()), (json!(to - 60 * MIN), json!(to), json!(5.0), json!(9), json!(9), json!(false)));
    let bins = |hits: &[usize]| { let mut b = vec![0u64; 20]; for h in hits { b[*h] += 1; } json!(b) };
    assert_eq!(c["rows"], json!([
        { "key": "1", "n": 4, "median": 11.5, "sigma": 1.25f64.sqrt(), "spark": [[at(0.0), 10.5], [at(10.0), 13.0], [at(25.0), 12.0]], "bins": bins(&[7, 9, 11, 13]), "below": 0, "above": 0 },
        { "key": "2", "n": 3, "median": 14.0, "sigma": (8.0f64 / 3.0).sqrt(), "spark": [[at(0.0), 12.0], [at(5.0), 14.0], [at(20.0), 16.0]], "bins": bins(&[6, 10, 14]), "below": 0, "above": 0 },
        { "key": "3", "n": 2, "median": 20.5, "sigma": 0.5, "spark": [[at(10.0), 20.0], [at(30.0), 21.0]], "bins": bins(&[9, 11]), "below": 0, "above": 0 },
    ]));

    // A panel that cannot be computed says why in its place; the others stand.
    assert_eq!(d["panels"][11]["error"], json!("`outlets` has no column `nothing` (its columns: outlet, sorted, travel)"));
    assert!(d["panels"][11]["data"].is_null());
    // A limit above: a breach is a point past it.
    let up = panel(&d, 12);
    assert_eq!((up["limit"].clone(), up["unit"].clone(), up["total"].clone()), (json!({ "above": 16.0 }), json!("s"), json!(9)));
    assert_eq!(up["breaches"], json!([[at(10.0), 20.0], [at(30.5), 21.0]]), "16 is not past 16");
    // A value below its target says how far, signed.
    let t = panel(&d, 13);
    assert_eq!((t["text"].clone(), t["state"].clone(), t["sub"].clone()), (json!("99.2"), json!("normal"), json!("-1% on the 100 target")));

    // The fourth state: no line for longer than its bound, the tile stale with its last value
    // and how long ago it came.
    assert!(wait(Duration::from_secs(10), || panel(&get_d(), 3)["stale"] == json!(true)));
    let t = panel(&get_d(), 3).clone();
    assert_eq!((t["text"].clone(), t["state"].clone(), t["word"].clone(), t["mark"].clone()), (json!("99.2"), json!("stale"), json!("stale"), json!("\u{25CC}")));
    let sub = t["sub"].as_str().unwrap();
    let ago: u64 = sub.strip_prefix("last value ").and_then(|r| r.strip_suffix(" s ago")).unwrap_or_else(|| panic!("{sub}")).parse().unwrap();
    assert!(ago >= 1 && ago <= appended.elapsed().as_secs(), "{sub}, appended {:?} ago", appended.elapsed());
    // Other right edges, no new data: each window keeps what lies in it and nothing else.
    let late = at(67.0);
    let (_, d) = get(addr, &format!("/s/loop/dashboard/0?to={late}"));
    assert_eq!(panel(&d, 4)["total"], json!(5), "the trend from at(7): 10, 14, 20, 29, 30.5");
    // The state holding at the window's start (stopped since at(6)) opens the first run at it.
    assert_eq!(panel(&d, 6)["rows"][0]["segments"], json!([
        { "from": at(7.0), "to": at(14.0), "state": "warn", "mark": "\u{25A0}", "word": "stopped" },
        { "from": at(14.0), "to": at(29.0), "state": "alarm", "mark": "!", "word": "fault" },
        { "from": at(29.0), "to": late, "state": "normal", "mark": "\u{25CF}", "word": "running" },
    ]));
    // A window with no change in it is one run of the state that holds.
    let (_, d95) = get(addr, &format!("/s/loop/dashboard/0?to={}", at(95.0)));
    assert_eq!(panel(&d95, 6)["rows"][0]["segments"], json!([{ "from": at(35.0), "to": at(95.0), "state": "normal", "mark": "\u{25CF}", "word": "running" }]));
    let ns: Vec<(Value, Value)> = panel(&d, 10)["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!(ns, [(json!("1"), json!(2)), (json!("2"), json!(1)), (json!("3"), json!(2))]);
    let first = &panel(&d, 5)["buckets"][0];
    assert_eq!((first["start"].clone(), first["value"].clone(), first["n"].clone()), (json!(at(5.0)), json!(14.0), json!(1)), "the first bucket whole, from its clock start before the window's");
    let early = at(14.5);
    let (_, d) = get(addr, &format!("/s/loop/dashboard/0?to={early}"));
    assert_eq!(panel(&d, 4)["total"], json!(6), "the trend up to at(14.5)");
    assert_eq!((panel(&d, 10)["rows"][0]["key"].clone(), panel(&d, 10)["rows"][0]["n"].clone()), (json!("1"), json!(3)));
    let segs = panel(&d, 6)["rows"][0]["segments"].as_array().unwrap().clone();
    assert_eq!((segs.len(), segs[2]["to"].clone(), segs[2]["word"].clone()), (3, json!(early), json!("fault")), "the last run ends at the edge");
    // A state set hours before the window still holds at its start, within the day before: the
    // line running since at(29), a window opening 22 hours later is one running run; one opening
    // 25 hours later finds nothing within its day, and draws no run.
    let hours = |h: i64| at(29.0) + h * 60 * MIN + 60 * MIN;
    let (_, d22) = get(addr, &format!("/s/loop/dashboard/0?to={}", hours(22)));
    assert_eq!(panel(&d22, 6)["rows"][0]["segments"], json!([{ "from": hours(22) - 60 * MIN, "to": hours(22), "state": "normal", "mark": "\u{25CF}", "word": "running" }]));
    let (_, d25) = get(addr, &format!("/s/loop/dashboard/0?to={}", hours(25)));
    assert_eq!((panel(&d25, 6)["rows"][0]["segments"].clone(), panel(&d25, 6)["rows"][1]["segments"].clone()), (json!([]), json!([])), "past the day before, nothing");

    // The cells' cap: the newest rows kept, read in pages, said when it bit.
    let live = host.session("loop").unwrap().live();
    let capped = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 5).unwrap();
    assert_eq!((capped["read"].clone(), capped["total"].clone(), capped["capped"].clone()), (json!(5), json!(9), json!(true)));
    let kept: Vec<(Value, Value)> = capped["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!(kept, [(json!("1"), json!(2)), (json!("2"), json!(1)), (json!("3"), json!(2))], "the newest five: 30.5, 29, 20, 14, 10");
    let whole = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
    assert_eq!((whole["read"].clone(), whole["capped"].clone()), (json!(9), json!(false)), "five pages of two, the last short");
    // Buckets of fifteen minutes on the clock for the sparkline: outlet 1's medians per bucket.
    let quarter = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 15, 5.0, to, 50, 50_000).unwrap();
    let q = 15 * MIN;
    let mut per: std::collections::BTreeMap<i64, Vec<f64>> = std::collections::BTreeMap::new();
    for (m, v) in [(0.0, 10.0), (4.0, 11.0), (14.0, 13.0), (29.0, 12.0)] {
        per.entry(at(m).div_euclid(q) * q).or_default().push(v);
    }
    let want: Vec<Value> = per.into_iter().map(|(s, mut vs)| { vs.sort_by(|a, b| a.partial_cmp(b).unwrap()); json!([s, veredas_supervisao::panel::median(&vs)]) }).collect();
    assert_eq!(quarter["rows"][0]["spark"], json!(want), "outlet 1 by the quarter hour");

    // Without `to`, the window ends at the store's now.
    let before = store_now();
    let (status, now_d) = get(addr, "/s/loop/dashboard/0");
    let after = store_now();
    assert_eq!(status, 200);
    let edge = now_d["to"].as_i64().unwrap();
    assert!(before <= edge && edge <= after, "{before} <= {edge} <= {after}");

    // The page draws it: run in a headless browser, the dashboard the page fetched (kept as it
    // arrives) and the panels it drew, read in one moment, agree panel by panel.
    let mut page = Browser::open(&format!("http://{addr}/s/loop/#layout"));
    page.wait_for("document.querySelector('.tabs') !== null && document.querySelector('.tabs').children.length > 0", Duration::from_secs(20));
    page.eval(KEEP_DASHBOARD);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let (dash, drawn) = loop {
        let v = page.eval(DRAWN_PANELS);
        if !v.is_null() {
            let want = expected_panels(&v["dash"]);
            if close(&v["drawn"], &json!(want)) {
                break (v["dash"].clone(), v["drawn"].clone());
            }
            assert!(std::time::Instant::now() < deadline, "the page never drew its dashboard:\n drawn {:#}\n want {:#}", v["drawn"], json!(want));
        }
        assert!(std::time::Instant::now() < deadline, "the page never fetched its dashboard");
        std::thread::sleep(Duration::from_millis(200));
    };
    // What it drew, spelled out where the bench's values make it certain.
    assert_eq!(dash["panels"].as_array().unwrap().len(), 16);
    // The cells panel as the page's own fetch was handed it past its cap: the note drawn.
    // (The page reads the live window: its total is what that window held, the last sort in it
    // or not yet.)
    let total = dash["panels"][10]["data"]["total"].clone();
    assert!(drawn[10]["right"].as_str().unwrap().ends_with(&format!(" · newest 5 of {total} rows")) && (total == 8 || total == 9), "{} of {total}", drawn[10]["right"]);
    assert_eq!(drawn[6]["rows"].as_array().unwrap().len(), 2, "two rows on the timeline");
    assert_eq!((drawn[0]["big"].clone(), drawn[0]["box"].clone(), drawn[0]["badge"].clone()), (json!("99.2%"), json!("st-box-normal"), Value::Null));
    assert_eq!((drawn[2]["box"].clone(), drawn[2]["badge"].clone()), (json!("st-box-alarm"), json!("! UNDER 99.5 %")));
    assert_eq!(drawn[7]["cells"], json!([["Outlet 1", "35", "cell st-box-normal"], ["Outlet 2", "85", "cell st-box-near"], ["Outlet 3", "full", "cell st-box-warn"]]));
    assert_eq!(drawn[9]["rows"][0], json!([["3", "mono", null], ["2", "mono", "width:50.0%"], ["■ slow", "mono st-text-warn", null]]), "sorted by travel, descending");
    assert_eq!(drawn[11]["error"], json!("`outlets` has no column `nothing` (its columns: outlet, sorted, travel)"));
    drop(page);
    host.stop();
}

/// Kept in the page: the dashboard JSON as the page's own fetch receives it, then the view opened.
/// The cells panel is handed over as a read past its cap (five of its nine rows), so the page's
/// note on it is drawn; the rest as it came.
const KEEP_DASHBOARD: &str = r#"(() => {
  const orig = window.fetch;
  window.fetch = async (...a) => {
    const r = await orig(...a);
    if (!String(a[0]).includes("/dashboard/")) return r;
    const j = await r.json();
    for (const p of j.panels) if (p.kind === "cells" && p.data) { p.data.capped = true; p.data.read = 5; }
    window.__dash = j;
    return new Response(JSON.stringify(j), { status: r.status, headers: r.headers });
  };
  location.hash = "dashboards/0";
  return true;
})()"#;

/// The dashboard the page fetched last and what it drew of every panel, read in one moment: the
/// texts and classes, and the charts' geometry (every vertex, dot, bar, run and line).
const DRAWN_PANELS: &str = r#"(() => {
  const d = window.__dash;
  if (!d) return null;
  const pts = (path) => path ? (path.getAttribute("d").match(/[ML][^ML]*/g) || []).map((s) => s.slice(1).trim().split(/\s+/).map(Number)) : [];
  const num = (el, a) => el ? Number(el.getAttribute(a)) : null;
  const txt = (el) => el ? el.textContent : null;
  const drawn = [...document.querySelectorAll(".dash-grid > .panel")].map((p) => {
    const r = { kind: p.dataset.kind, title: txt(p.querySelector(".panel-title")), span: getComputedStyle(p).gridColumnStart };
    const err = p.querySelector(":scope > p.error");
    if (err) { r.error = err.textContent; return r; }
    switch (r.kind) {
      case "value":
        r.box = [...p.classList].find((c) => c.startsWith("st-box-")) ?? null;
        r.big = txt(p.querySelector(".big")); r.big_cls = p.querySelector(".big").className;
        r.badge = txt(p.querySelector(".panel-head .badge")); r.sub = txt(p.querySelector(".sub"));
        r.spark = pts(p.querySelector("svg path.line")); break;
      case "trend": {
        const svg = p.querySelector("svg.trend");
        r.line = pts(svg.querySelector("path.line"));
        r.breaches = [...svg.querySelectorAll("circle.breach")].map((c) => [num(c, "cx"), num(c, "cy"), num(c, "r")]);
        const lim = svg.querySelector("line.limit");
        r.limit = lim ? [num(lim, "x1"), num(lim, "x2"), num(lim, "y1"), num(lim, "y2")] : null;
        r.right = txt(p.querySelector(".panel-bind")); break;
      }
      case "bars": {
        const svg = p.querySelector("svg.bars");
        r.bars = [...svg.querySelectorAll("rect.bar")].map((b) => [num(b, "x"), num(b, "y"), num(b, "width"), num(b, "height"), b.classList.contains("running")]);
        const tg = svg.querySelector("line.limit");
        r.target = tg ? [num(tg, "x1"), num(tg, "x2"), num(tg, "y1"), num(tg, "y2")] : null; r.right = txt(p.querySelector(".panel-bind")); break;
      }
      case "timeline":
        r.rows = [...p.querySelectorAll(".tl-row")].map((x) => [txt(x.querySelector(".tl-label")), txt(x.querySelector(".tl-now")), x.querySelector(".tl-now").className,
          [...x.querySelectorAll("svg.statebar rect:not(.st-none)")].map((g) => [g.getAttribute("class"), num(g, "x"), num(g, "width"), num(g, "y"), num(g, "height")])]); break;
      case "grid":
        r.cells = [...p.querySelectorAll(".grid-cells .cell")].map((c) => [txt(c.querySelector(".cell-id")), txt(c.querySelector(".cell-v")), c.className]); break;
      case "levels":
        r.rows = [...p.querySelectorAll(".lv-row")].map((x) => [txt(x.querySelector(".lv-label")), x.querySelector(".lv-fill").getAttribute("style"), x.querySelector(".lv-fill").className, [...x.querySelectorAll(".lv-tick")].map((t) => t.getAttribute("style")), txt(x.querySelector(".lv-v"))]); break;
      case "table":
        r.head = [...p.querySelectorAll("thead th")].map(txt);
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => [...tr.children].map((td) => [td.textContent, td.className, td.querySelector(".inbar-fill")?.getAttribute("style") ?? null]));
        r.right = txt(p.querySelector(".panel-bind")); break;
      case "cells":
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => { const td = [...tr.children]; return [txt(td[0]), txt(td[1]), pts(td[2].querySelector("path.line")), pts(td[3].querySelector("path.line")), txt(td[4]), txt(td[5])]; });
        r.right = txt(p.querySelector(".panel-bind")); break;
    }
    return r;
  });
  return { dash: d, drawn };
})()"#;

/// A number as the page writes it: to `places`, trailing zeros and a bare point dropped.
fn num(v: &Value, places: usize) -> String {
    match v.as_f64() {
        Some(x) if x.is_finite() => {
            let t = format!("{x:.places$}");
            let t = if t.contains('.') { t.trim_end_matches('0').trim_end_matches('.').to_string() } else { t };
            if t == "-0" { "0".to_string() } else { t }
        }
        _ => "\u{2014}".to_string(),
    }
}

/// The charts' frame as the reference for the page states it (`web/src/charts.ts`): a value from
/// `[d0, d1]` placed on `[r0, r1]`, a span of nothing read as one.
fn scale(d0: f64, d1: f64, r0: f64, r1: f64) -> impl Fn(f64) -> f64 {
    let span = if d1 - d0 == 0.0 { 1.0 } else { d1 - d0 };
    move |v| r0 + (v - d0) / span * (r1 - r0)
}

/// The values' extent padded by 8 % each way; one value alone widened by one each way.
fn extent(vals: &[f64]) -> (f64, f64) {
    if vals.is_empty() {
        return (0.0, 1.0);
    }
    let (mut lo, mut hi) = vals.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    if lo == hi {
        lo -= 1.0;
        hi += 1.0;
    }
    let p = (hi - lo) * 0.08;
    (lo - p, hi + p)
}

/// A sparkline's vertices: time across `[1, w - 1]`, the values' extent up `[h - 2, 2]`.
fn spark_at(pts: &[(f64, f64)], w: f64, h: f64) -> Vec<Value> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let x = scale(pts[0].0, pts[pts.len() - 1].0, 1.0, w - 1.0);
    let (lo, hi) = extent(&pts.iter().map(|p| p.1).collect::<Vec<_>>());
    let y = scale(lo, hi, h - 2.0, 2.0);
    pts.iter().map(|(t, v)| json!([x(*t), y(*v)])).collect()
}

fn pairs_of(v: &Value) -> Vec<(f64, f64)> {
    v.as_array().map(|a| a.iter().filter(|p| !p[1].is_null()).map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect()).unwrap_or_default()
}

/// What the page must draw of each panel of a dashboard, in the reader's terms, its geometry as
/// the chart's frame places the panel's numbers.
fn expected_panels(d: &Value) -> Vec<Value> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let quiet = |st: &Value| st == "normal" || st == "near";
    d["panels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let mut r = json!({ "kind": p["kind"], "title": p["title"], "span": format!("span {}", p["span"]) });
            if let Some(e) = p.get("error").filter(|e| !e.is_null()) {
                r["error"] = e.clone();
                return r;
            }
            let x = &p["data"];
            match p["kind"].as_str().unwrap() {
                "value" => {
                    r["box"] = json!(format!("st-box-{}", s(&x["state"])));
                    r["big"] = json!(format!("{}{}", x["text"].as_str().unwrap_or("\u{2014}"), s(&x["unit"])));
                    r["big_cls"] = json!(format!("big mono st-text-{}", s(&x["state"])));
                    r["badge"] = if x["state"] == "normal" { Value::Null } else { json!(format!("{} {}", s(&x["mark"]), s(&x["word"]).to_uppercase())) };
                    r["sub"] = json!(s(&x["sub"]));
                    r["spark"] = json!(spark_at(&pairs_of(&x["spark"]), 260.0, 30.0));
                }
                "trend" => {
                    // 600 by 200, the plot from x 44 to 590 and from y 178 up to 10; the values'
                    // extent with the limit in it.
                    let n = x["breaches"].as_array().unwrap().len();
                    let unit = if s(&x["unit"]).is_empty() { String::new() } else { format!(" {}", s(&x["unit"])) };
                    let limit = if x["limit"].is_null() { None } else { x["limit"].get("above").or(x["limit"].get("below")).and_then(Value::as_f64) };
                    let points = pairs_of(&x["points"]);
                    let mut vals: Vec<f64> = points.iter().map(|p| p.1).collect();
                    vals.extend(limit);
                    let (lo, hi) = extent(&vals);
                    let px = scale(x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap(), 44.0, 590.0);
                    let py = scale(lo, hi, 178.0, 10.0);
                    r["line"] = json!(if points.len() > 1 { points.iter().map(|(t, v)| json!([px(*t), py(*v)])).collect::<Vec<_>>() } else { Vec::new() });
                    // A breach a dot of radius 3; the limit a line across the plot at its value.
                    r["breaches"] = json!(pairs_of(&x["breaches"]).iter().map(|(t, v)| json!([px(*t), py(*v), 3.0])).collect::<Vec<_>>());
                    r["limit"] = json!(limit.map(|l| json!([44.0, 590.0, py(l), py(l)])));
                    r["right"] = match limit {
                        None => json!(format!("{} points", x["total"])),
                        Some(_) => {
                            let (word, at) = if x["limit"].get("above").is_some() { ("max", &x["limit"]["above"]) } else { ("min", &x["limit"]["below"]) };
                            json!(format!("{word} {}{unit} · {n} breach{}", num(at, 2), if n == 1 { "" } else { "es" }))
                        }
                    };
                }
                "bars" => {
                    // 600 by 180, from x 44, from y 158 up to 10, the highest of the values, the
                    // target and 0 a tenth below the top; a bar per bucket with a value, in its slot.
                    let unit = if s(&x["unit"]).is_empty() { String::new() } else { format!(" {}", s(&x["unit"])) };
                    let buckets = x["buckets"].as_array().unwrap();
                    let target = x["target"].as_f64();
                    let mut vals: Vec<f64> = buckets.iter().map(|b| b["value"].as_f64().unwrap_or(0.0)).collect();
                    vals.extend(target);
                    vals.push(0.0);
                    let top = vals.iter().cloned().fold(f64::MIN, f64::max) * 1.1;
                    let hi = if top == 0.0 { 1.0 } else { top };
                    let py = scale(0.0, hi, 158.0, 10.0);
                    let bw = 546.0 / (buckets.len().max(1) as f64);
                    r["bars"] = json!(buckets.iter().enumerate().filter(|(_, b)| !b["value"].is_null()).map(|(i, b)| {
                        let v = b["value"].as_f64().unwrap();
                        json!([44.0 + i as f64 * bw + 2.0, py(v), bw - 4.0, (py(0.0) - py(v)).max(0.0), b["running"]])
                    }).collect::<Vec<_>>());
                    r["target"] = json!(target.map(|t| json!([44.0, 590.0, py(t), py(t)])));
                    r["right"] = if x["target"].is_null() { Value::Null } else { json!(format!("target {}{unit}", num(&x["target"], 2))) };
                }
                "timeline" => {
                    // 400 wide: a run from its start to its end in the window, clipped to it.
                    let px = scale(x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap(), 0.0, 400.0);
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let segs: Vec<Value> = row["segments"].as_array().unwrap().iter().filter_map(|g| {
                            let (x0, x1) = (px(g["from"].as_f64().unwrap()).max(0.0), px(g["to"].as_f64().unwrap()).min(400.0));
                            (x1 > x0).then(|| json!([format!("st-{}", s(&g["state"])), x0, x1 - x0, 0.0, 14.0]))
                        }).collect();
                        json!([row["label"], format!("{} {}", s(&row["now"]["mark"]), s(&row["now"]["word"])), format!("tl-now st-text-{}", s(&row["now"]["state"])), segs])
                    }).collect::<Vec<_>>());
                }
                "grid" => {
                    r["cells"] = json!(x["cells"].as_array().unwrap().iter().map(|c| {
                        let v = if quiet(&c["state"]) { c["text"].as_str().unwrap_or("\u{2014}").to_string() } else { s(&c["word"]) };
                        json!([c["label"], v, format!("cell st-box-{}", s(&c["state"]))])
                    }).collect::<Vec<_>>());
                }
                "levels" => {
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let unit = if s(&row["unit"]).is_empty() { String::new() } else { format!(" {}", s(&row["unit"])) };
                        let v = if quiet(&row["state"]) { format!("{}{unit}", row["text"].as_str().unwrap_or("\u{2014}")) } else { s(&row["word"]) };
                        let ticks: Vec<String> = row["ticks"].as_array().unwrap().iter().map(|t| format!("left:{:.1}%", t.as_f64().unwrap() * 100.0)).collect();
                        json!([row["label"], format!("width:{:.1}%", row["fraction"].as_f64().unwrap_or(0.0) * 100.0), format!("lv-fill st-fill-{}", s(&row["state"])), ticks, v])
                    }).collect::<Vec<_>>());
                }
                "table" => {
                    r["head"] = json!(x["columns"].as_array().unwrap().iter().map(|c| c["label"].clone()).collect::<Vec<_>>());
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(|c| {
                        if let Some(bar) = c.get("bar").and_then(Value::as_f64) {
                            json!([c["text"], "mono", format!("width:{:.1}%", bar * 100.0)])
                        } else if let Some(st) = c.get("state").filter(|v| !v.is_null()) {
                            let t = if s(&c["word"]).is_empty() { s(&c["text"]) } else { format!("{} {}", s(&c["mark"]), s(&c["word"])) };
                            json!([t, format!("mono st-text-{}", st.as_str().unwrap()), null])
                        } else {
                            json!([c["text"], "mono", null])
                        }
                    }).collect::<Vec<_>>()).collect::<Vec<_>>());
                    r["right"] = json!(format!("{} rows", x["total"]));
                }
                "cells" => {
                    // A distribution 160 by 26: bin i at its middle across, its count up from y 25
                    // to 2 against the largest (one at least).
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let bins: Vec<f64> = row["bins"].as_array().unwrap().iter().map(|b| b.as_f64().unwrap()).collect();
                        let bw = 160.0 / bins.len() as f64;
                        let py = scale(0.0, bins.iter().cloned().fold(1.0, f64::max), 25.0, 2.0);
                        let dist: Vec<Value> = bins.iter().enumerate().map(|(i, c)| json!([i as f64 * bw + bw / 2.0, py(*c)])).collect();
                        json!([row["key"], num(&row["median"], 1), spark_at(&pairs_of(&row["spark"]), 120.0, 24.0), dist, num(&row["sigma"], 1), row["n"].to_string()])
                    }).collect::<Vec<_>>());
                    let minutes = ((x["to"].as_f64().unwrap() - x["from"].as_f64().unwrap()) / 60_000.0).round();
                    let cap = if x["capped"] == json!(true) { format!(" · newest {} of {} rows", x["read"], x["total"]) } else { String::new() };
                    r["right"] = json!(format!("last {minutes} min · distribution ±{} around each median, same scale on every row{cap}", num(&x["spread"], 2)));
                }
                _ => {}
            }
            r
        })
        .collect()
}

/// Whether what the page drew is what it must draw: texts and classes exactly, coordinates within
/// the page's own rounding (a path's to a tenth).
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= 0.051,
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| close(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w))),
        _ => a == b,
    }
}

#[test]
fn the_cells_read_each_row_once_while_rows_arrive() {
    // Rows stored between two page reads shift an offset by one: one row read twice, another
    // missed. Paged by stamp, every row the window held when the read began is read once, however
    // many arrive at its newest end meanwhile.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    ex.project = ex.project.replace("writes_armed = false", "writes_armed = false\nidle_after_ms = 600000");
    let path = write_example(dir.path(), &ex);
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let live = host.session("loop").unwrap().live();
    assert!(wait(Duration::from_secs(20), || live.health.read().unwrap().get("sorter").is_some_and(|h| h.count == 0)), "the log is attached");
    let rows = || live.store.query_rows("sorts", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    // Three hundred sorts over the last half hour, a hundred per outlet.
    let now = store_now();
    append(&log, &(0..300).map(|i| format!("{} SORT outlet={} travel={} rate=99.0", stamp(now - 30 * MIN + i * 5_000), 1 + i % 3, 10.0 + (i % 7) as f64)).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || rows() == 300), "{}", rows());
    // Then a hundred and fifty sorts arriving at the newest end, ten milliseconds apart, while the
    // cells are read two rows a page, again and again until the last has arrived.
    let stop = Arc::new(AtomicBool::new(false));
    let writer = std::thread::spawn({
        let (stop, log) = (Arc::clone(&stop), log.clone());
        move || {
            for _ in 0..150 {
                append(&log, &[format!("{} SORT outlet=9 travel=10.0 rate=99.0", stamp(store_now()))]);
                std::thread::sleep(Duration::from_millis(10));
            }
            stop.store(true, Ordering::Relaxed);
        }
    });
    let to = store_now() + 10 * MIN;
    let (mut grew, mut reads) = (0, 0);
    while !stop.load(Ordering::Relaxed) || reads < 2 {
        let before = rows();
        let c = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
        grew += rows() - before;
        reads += 1;
        let ns: Vec<(Value, Value)> = c["rows"].as_array().unwrap().iter().filter(|r| r["key"] != "9").map(|r| (r["key"].clone(), r["n"].clone())).collect();
        assert_eq!(ns, [(json!("1"), json!(100)), (json!("2"), json!(100)), (json!("3"), json!(100))], "each sort once, read {reads}: {c}");
    }
    writer.join().unwrap();
    assert!(grew > 0, "rows arrived while the cells were read ({reads} reads)");
    host.stop();
}

// ---- the computations alone, at their edges (src/panel.rs) ----

use veredas_supervisao::binding::Aggregate;
use veredas_supervisao::panel::{buckets, distribution, median, sigma};

#[test]
fn buckets_align_on_the_clock_and_mark_the_running_one() {
    let b = 300_000;
    // A window from 07:02:00 to 07:13:00 (in ms past an hour) holds the buckets 07:00, 07:05, 07:10.
    let (from, to) = (2 * 60_000, 13 * 60_000);
    let pts = vec![(60_000, 1.0), (200_000, 3.0), (400_000, 5.0), (700_000, 7.0), (780_000, 9.0)];
    let out = buckets(&pts, from, to, b, Aggregate::Mean);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], json!({ "start": 0, "end": 300_000, "value": 2.0, "n": 2, "running": false }));
    assert_eq!(out[1], json!({ "start": 300_000, "end": 600_000, "value": 5.0, "n": 1, "running": false }));
    assert_eq!(out[2], json!({ "start": 600_000, "end": 900_000, "value": 8.0, "n": 2, "running": true }));
    assert_eq!(buckets(&[], from, to, b, Aggregate::Count)[0]["value"], json!(0.0));
    assert_eq!(buckets(&[], from, to, b, Aggregate::Sum)[0]["value"], Value::Null);
    assert_eq!(buckets(&pts, from, to, b, Aggregate::Last)[2]["value"], json!(9.0));
    assert_eq!(buckets(&pts, from, to, b, Aggregate::Max)[0]["value"], json!(3.0));
    assert_eq!(buckets(&pts, from, to, b, Aggregate::Min)[2]["value"], json!(7.0));
    assert_eq!(buckets(&pts, from, to, b, Aggregate::Sum)[2]["value"], json!(16.0));
    // A bucket ending exactly at `to` is not running; the next one starts there and is.
    let edge = buckets(&pts, from, 600_000, b, Aggregate::Count);
    assert_eq!(edge.len(), 3);
    assert_eq!((edge[1]["running"].clone(), edge[2]["running"].clone(), edge[2]["start"].clone()), (json!(false), json!(true), json!(600_000)));
    // Hours and days align on the clock too, from a window off the five-minute grid: a window
    // from 07:17 opens its hourly buckets at 07:00 and its daily one at midnight.
    let day = 86_400_000;
    let hour = 3_600_000;
    let at = |h: i64, m: i64| 20 * day + h * hour + m * 60_000;
    let pts = vec![(at(7, 20), 1.0), (at(7, 50), 3.0), (at(8, 10), 5.0), (at(9, 59), 7.0)];
    let hourly = buckets(&pts, at(7, 17), at(9, 30), hour, Aggregate::Mean);
    let starts: Vec<Value> = hourly.iter().map(|b| b["start"].clone()).collect();
    assert_eq!(starts, [json!(at(7, 0)), json!(at(8, 0)), json!(at(9, 0))]);
    assert_eq!((hourly[0]["value"].clone(), hourly[1]["value"].clone(), hourly[2]["running"].clone()), (json!(2.0), json!(5.0), json!(true)));
    let daily = buckets(&pts, at(7, 17), at(9, 30), day, Aggregate::Count);
    assert_eq!((daily.len(), daily[0]["start"].clone(), daily[0]["value"].clone()), (1, json!(20 * day), json!(4.0)));
}

#[test]
fn a_table_sorts_numbers_as_numbers_with_nulls_last() {
    use veredas_supervisao::panel::sort_rows;
    let rows = || vec![vec![json!(10.5)], vec![Value::Null], vec![json!(9.5)], vec![json!(20.5)], vec![json!(100)]];
    let column = |r: &[Vec<Value>]| r.iter().map(|x| x[0].clone()).collect::<Vec<_>>();
    let mut down = rows();
    sort_rows(&mut down, 0, true);
    assert_eq!(column(&down), [json!(100), json!(20.5), json!(10.5), json!(9.5), Value::Null], "numbers, not their text; the null last");
    let mut up = rows();
    sort_rows(&mut up, 0, false);
    assert_eq!(column(&up), [json!(9.5), json!(10.5), json!(20.5), json!(100), Value::Null]);
    let mut words = vec![vec![json!("b")], vec![Value::Null], vec![json!("a")]];
    sort_rows(&mut words, 0, true);
    assert_eq!(column(&words), [json!("b"), json!("a"), Value::Null]);
}

#[test]
fn medians_sigmas_and_distributions_are_exact() {
    assert_eq!(median(&[1.0, 2.0, 9.0]), Some(2.0));
    assert_eq!(median(&[1.0, 2.0, 3.0, 9.0]), Some(2.5));
    assert_eq!(median(&[]), None);
    assert_eq!(sigma(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]), Some(2.0));
    let (bins, below, above) = distribution(&[-1.0, 0.0, 0.5, 5.0, 9.99, 10.0, 10.5], 5.0, 5.0, 10);
    assert_eq!((below, above), (1, 1));
    assert_eq!(bins, vec![2, 0, 0, 0, 0, 1, 0, 0, 0, 2]);
}
