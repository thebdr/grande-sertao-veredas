//! C-052: the dashboards. Every panel kind computed by the host from the session's store, exactly:
//! a log the test writes with stamps it chooses (on the store's axis, aligned on five minutes)
//! and the bench's fill levels, read back over the dashboard route with the window's right edge
//! named, so every bucket, breach, segment, median and bin is known before the test runs; then
//! the page itself, run in a headless browser, read back against the dashboard it fetched.

mod common;

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
    let to = at(31.0);
    let rows = |t: &str| live.store.query_rows(t, &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || rows("sorts") == 9 && rows("states") == 5), "the lines are stored");
    let get_d = || get(addr, &format!("/s/loop/dashboard/0?to={to}")).1;
    // The feed's view of the latest values, and the fills, before the tiles are read.
    assert!(wait(Duration::from_secs(20), || {
        let d = get_d();
        panel(&d, 0)["text"] == json!("99.2") && panel(&d, 7)["cells"][2]["text"] == json!("100")
    }), "{}", get_d());
    let d = get_d();
    assert_eq!((d["name"].clone(), d["columns"].clone(), d["to"].clone(), d["panels"].as_array().unwrap().len()), (json!("Numbers"), json!(4), json!(to), 14));
    let heads: Vec<(String, String, i64)> = d["panels"].as_array().unwrap().iter().map(|p| (p["kind"].as_str().unwrap().to_string(), p["title"].as_str().unwrap().to_string(), p["span"].as_i64().unwrap())).collect();
    assert_eq!(heads[..4].iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), ["Read rate", "Travel", "Low rate", "Fresh rate"]);
    assert_eq!(heads[4..].iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["trend", "bars", "timeline", "grid", "levels", "table", "cells", "table", "trend", "value"]);

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
        (json!("outlet-3"), json!(0.5), json!([0.3, 0.4])),
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
    assert_eq!(panel(&d, 6)["rows"][0]["segments"], json!([
        { "from": at(10.0), "to": at(14.0), "state": "warn", "mark": "\u{25A0}", "word": "stopped" },
        { "from": at(14.0), "to": at(29.0), "state": "alarm", "mark": "!", "word": "fault" },
        { "from": at(29.0), "to": late, "state": "normal", "mark": "\u{25CF}", "word": "running" },
    ]));
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

    // The cells' cap: the newest rows kept, read in pages, said when it bit.
    let live = host.session("loop").unwrap().live();
    let capped = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 5).unwrap();
    assert_eq!((capped["read"].clone(), capped["total"].clone(), capped["capped"].clone()), (json!(5), json!(9), json!(true)));
    let kept: Vec<(Value, Value)> = capped["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!(kept, [(json!("1"), json!(2)), (json!("2"), json!(1)), (json!("3"), json!(2))], "the newest five: 30.5, 29, 20, 14, 10");
    let whole = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
    assert_eq!((whole["read"].clone(), whole["capped"].clone()), (json!(9), json!(false)), "five pages of two, the last short");

    // Without `to`, the window ends at the store's now.
    let (status, now_d) = get(addr, "/s/loop/dashboard/0");
    assert_eq!(status, 200);
    assert!((now_d["to"].as_i64().unwrap() - store_now()).abs() < 60_000);

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
            if v["drawn"] == json!(want) {
                break (v["dash"].clone(), v["drawn"].clone());
            }
            assert!(std::time::Instant::now() < deadline, "the page never drew its dashboard:\n drawn {:#}\n want {:#}", v["drawn"], json!(want));
        }
        assert!(std::time::Instant::now() < deadline, "the page never fetched its dashboard");
        std::thread::sleep(Duration::from_millis(200));
    };
    // What it drew, spelled out where the bench's values make it certain.
    assert_eq!(dash["panels"].as_array().unwrap().len(), 14);
    assert_eq!((drawn[0]["big"].clone(), drawn[0]["box"].clone(), drawn[0]["badge"].clone()), (json!("99.2%"), json!("st-box-normal"), Value::Null));
    assert_eq!((drawn[2]["box"].clone(), drawn[2]["badge"].clone()), (json!("st-box-alarm"), json!("! UNDER 99.5 %")));
    assert_eq!(drawn[7]["cells"], json!([["Outlet 1", "35", "cell st-box-normal"], ["Outlet 2", "85", "cell st-box-near"], ["Outlet 3", "full", "cell st-box-warn"]]));
    assert_eq!(drawn[9]["rows"][0], json!([["3", "mono", null], ["2", "mono", "width:50.0%"], ["■ slow", "mono st-text-warn", null]]), "sorted by travel, descending");
    assert_eq!(drawn[11]["error"], json!("`outlets` has no column `nothing` (its columns: outlet, sorted, travel)"));
    drop(page);
    host.stop();
}

/// Kept in the page: the dashboard JSON as the page's own fetch receives it, then the view opened.
const KEEP_DASHBOARD: &str = r#"(() => {
  const orig = window.fetch;
  window.fetch = async (...a) => {
    const r = await orig(...a);
    if (String(a[0]).includes("/dashboard/")) r.clone().json().then((j) => { window.__dash = j; });
    return r;
  };
  location.hash = "dashboards/0";
  return true;
})()"#;

/// The dashboard the page fetched last and what it drew of every panel, read in one moment.
const DRAWN_PANELS: &str = r#"(() => {
  const d = window.__dash;
  if (!d) return null;
  const verts = (path) => path ? (path.getAttribute("d").match(/[ML]/g) || []).length : 0;
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
        r.spark = verts(p.querySelector("svg path.line")); break;
      case "trend":
        r.line = verts(p.querySelector("svg.trend path.line")); r.breaches = p.querySelectorAll("svg.trend circle.breach").length;
        r.limit = p.querySelector("svg.trend line.limit") !== null; r.right = txt(p.querySelector(".panel-bind")); break;
      case "bars":
        r.bars = [...p.querySelectorAll("svg.bars rect.bar")].map((b) => b.classList.contains("running"));
        r.target = p.querySelector("svg.bars line.limit") !== null; r.right = txt(p.querySelector(".panel-bind")); break;
      case "timeline":
        r.rows = [...p.querySelectorAll(".tl-row")].map((x) => [txt(x.querySelector(".tl-label")), txt(x.querySelector(".tl-now")), x.querySelector(".tl-now").className, [...x.querySelectorAll("svg.statebar rect:not(.st-none)")].map((g) => g.getAttribute("class"))]); break;
      case "grid":
        r.cells = [...p.querySelectorAll(".grid-cells .cell")].map((c) => [txt(c.querySelector(".cell-id")), txt(c.querySelector(".cell-v")), c.className]); break;
      case "levels":
        r.rows = [...p.querySelectorAll(".lv-row")].map((x) => [txt(x.querySelector(".lv-label")), x.querySelector(".lv-fill").getAttribute("style"), x.querySelector(".lv-fill").className, [...x.querySelectorAll(".lv-tick")].map((t) => t.getAttribute("style")), txt(x.querySelector(".lv-v"))]); break;
      case "table":
        r.head = [...p.querySelectorAll("thead th")].map(txt);
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => [...tr.children].map((td) => [td.textContent, td.className, td.querySelector(".inbar-fill")?.getAttribute("style") ?? null]));
        r.right = txt(p.querySelector(".panel-bind")); break;
      case "cells":
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => { const td = [...tr.children]; return [txt(td[0]), txt(td[1]), verts(td[2].querySelector("path.line")), verts(td[3].querySelector("path.line")), txt(td[4]), txt(td[5])]; });
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

/// What the page must draw of each panel of a dashboard, in the reader's terms.
fn expected_panels(d: &Value) -> Vec<Value> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let lines = |n: usize| if n >= 2 { n } else { 0 };
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
                    r["spark"] = json!(lines(x["spark"].as_array().map_or(0, |a| a.len())));
                }
                "trend" => {
                    let n = x["breaches"].as_array().unwrap().len();
                    let unit = if s(&x["unit"]).is_empty() { String::new() } else { format!(" {}", s(&x["unit"])) };
                    r["line"] = json!(lines(x["points"].as_array().unwrap().len()));
                    r["breaches"] = json!(n);
                    r["limit"] = json!(!x["limit"].is_null());
                    r["right"] = if x["limit"].is_null() {
                        json!(format!("{} points", x["total"]))
                    } else {
                        let (word, at) = if x["limit"].get("above").is_some() { ("max", &x["limit"]["above"]) } else { ("min", &x["limit"]["below"]) };
                        json!(format!("{word} {}{unit} · {n} breach{}", num(at, 2), if n == 1 { "" } else { "es" }))
                    };
                }
                "bars" => {
                    let unit = if s(&x["unit"]).is_empty() { String::new() } else { format!(" {}", s(&x["unit"])) };
                    r["bars"] = json!(x["buckets"].as_array().unwrap().iter().filter(|b| !b["value"].is_null()).map(|b| b["running"].clone()).collect::<Vec<_>>());
                    r["target"] = json!(!x["target"].is_null());
                    r["right"] = if x["target"].is_null() { Value::Null } else { json!(format!("target {}{unit}", num(&x["target"], 2))) };
                }
                "timeline" => {
                    let (from, to) = (x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap());
                    let at = |v: f64| (v - from) / (to - from) * 400.0;
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let segs: Vec<String> = row["segments"].as_array().unwrap().iter()
                            .filter(|g| at(g["to"].as_f64().unwrap()).min(400.0) > at(g["from"].as_f64().unwrap()).max(0.0))
                            .map(|g| format!("st-{}", s(&g["state"]))).collect();
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
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let spark = row["spark"].as_array().unwrap().iter().filter(|p| !p[1].is_null()).count();
                        json!([row["key"], num(&row["median"], 1), lines(spark), row["bins"].as_array().unwrap().len(), num(&row["sigma"], 1), row["n"].to_string()])
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
