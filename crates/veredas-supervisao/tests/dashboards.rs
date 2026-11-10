//! C-052: the dashboards. Every panel kind computed by the host from the session's store, exactly:
//! a log the test writes with stamps it chooses (on the store's axis, aligned on five minutes)
//! and the bench's fill levels, read back over the dashboard route with the window's right edge
//! named, so every bucket, breach, segment, median and bin is known before the test runs.

mod common;

use std::time::Duration;

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
range = [0, 200]
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

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
objects = ["line"]

[[dashboard.panel]]
kind = "grid"
objects = ["outlet-*"]

[[dashboard.panel]]
kind = "levels"
objects = ["outlet-*"]
ticks = [80, 90]

[[dashboard.panel]]
kind = "table"
table = "outlets"
columns = [ { name = "outlet" }, { name = "sorted", bar = true }, { name = "travel", decimals = 1, states = [ { above = 14, state = "warn", word = "slow" } ] } ]
sort = { column = "sorted", descending = true }

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
limit = { above = 15 }
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
    append(&log, &LINES.iter().map(|(m, l)| format!("{} {l}", stamp(at(*m)))).collect::<Vec<_>>());
    let to = at(31.0);
    let rows = |t: &str| live.store.query_rows(t, &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || rows("sorts") == 9 && rows("states") == 4), "the lines are stored");
    let get_d = || get(addr, &format!("/s/loop/dashboard/0?to={to}")).1;
    // The feed's view of the latest values, and the fills, before the tiles are read.
    assert!(wait(Duration::from_secs(20), || {
        let d = get_d();
        panel(&d, 0)["text"] == json!("99.2") && panel(&d, 7)["cells"][2]["text"] == json!("100")
    }), "{}", get_d());
    let d = get_d();
    assert_eq!((d["name"].clone(), d["columns"].clone(), d["to"].clone(), d["panels"].as_array().unwrap().len()), (json!("Numbers"), json!(4), json!(to), 13));
    let heads: Vec<(String, String, i64)> = d["panels"].as_array().unwrap().iter().map(|p| (p["kind"].as_str().unwrap().to_string(), p["title"].as_str().unwrap().to_string(), p["span"].as_i64().unwrap())).collect();
    assert_eq!(heads[..4].iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), ["Read rate", "Travel", "Low rate", "Fresh rate"]);
    assert_eq!(heads[4..].iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["trend", "bars", "timeline", "grid", "levels", "table", "cells", "table", "trend"]);

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

    // The grid and the levels: the objects as the feed reads them, each level its place in its
    // own range, the ticks placed on it.
    let cells: Vec<(Value, Value, Value)> = panel(&d, 7)["cells"].as_array().unwrap().iter().map(|c| (c["text"].clone(), c["state"].clone(), c["word"].clone())).collect();
    assert_eq!(cells, [(json!("35"), json!("normal"), json!("ok")), (json!("85"), json!("near"), json!("near full")), (json!("100"), json!("warn"), json!("full"))]);
    let levels: Vec<(Value, Value, Value)> = panel(&d, 8)["rows"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["fraction"].clone(), r["ticks"].clone())).collect();
    assert_eq!(levels, [
        (json!("outlet-1"), json!(0.35), json!([0.8, 0.9])),
        (json!("outlet-2"), json!(0.85), json!([0.8, 0.9])),
        (json!("outlet-3"), json!(0.5), json!([0.4, 0.45])),
    ]);

    // The table: the view's rows sorted by the count, the count drawn as a bar against the
    // largest shown, the travel to one place with its state word.
    let tb = panel(&d, 9);
    assert_eq!((tb["total"].clone(), tb["columns"].clone()), (json!(3), json!([{ "name": "outlet", "label": "outlet", "bar": false }, { "name": "sorted", "label": "sorted", "bar": true }, { "name": "travel", "label": "travel", "bar": false }])));
    let shown: Vec<(Value, Value, Value, Value, Value)> = tb["rows"].as_array().unwrap().iter().map(|r| (r[0]["text"].clone(), r[1]["text"].clone(), r[1]["bar"].clone(), r[2]["text"].clone(), r[2]["word"].clone())).collect();
    assert_eq!(shown, [
        (json!("1"), json!("4"), json!(1.0), json!("11.5"), json!("")),
        (json!("2"), json!("3"), json!(0.75), json!("14.0"), json!("")),
        (json!("3"), json!("2"), json!(0.5), json!("20.5"), json!("slow")),
    ]);
    assert_eq!((tb["rows"][2][2]["state"].clone(), tb["rows"][2][2]["mark"].clone(), tb["rows"][0][2]["state"].clone()), (json!("warn"), json!("\u{25A0}"), json!("normal")));

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
    assert_eq!((up["limit"].clone(), up["unit"].clone(), up["total"].clone()), (json!({ "above": 15.0 }), json!("s"), json!(9)));
    assert_eq!(up["breaches"], json!([[at(10.0), 20.0], [at(20.0), 16.0], [at(30.5), 21.0]]));

    // The fourth state: no line for longer than its bound, the tile stale with its last value
    // and how long ago it came.
    assert!(wait(Duration::from_secs(10), || panel(&get_d(), 3)["stale"] == json!(true)));
    let t = panel(&get_d(), 3).clone();
    assert_eq!((t["text"].clone(), t["state"].clone(), t["word"].clone(), t["mark"].clone()), (json!("99.2"), json!("stale"), json!("stale"), json!("\u{25CC}")));
    let sub = t["sub"].as_str().unwrap();
    assert!(sub.starts_with("last value ") && sub.ends_with(" s ago") && sub["last value ".len()..sub.len() - " s ago".len()].parse::<u64>().unwrap() >= 1, "{sub}");
    // Without `to`, the window ends at the store's now.
    let (status, now_d) = get(addr, "/s/loop/dashboard/0");
    assert_eq!(status, 200);
    assert!((now_d["to"].as_i64().unwrap() - store_now()).abs() < 60_000);
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
