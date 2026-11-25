//! C-052: the dashboards. Every panel kind computed by the host from the session's store, exactly:
//! a log the test writes with stamps it chooses (on the store's axis, aligned on five minutes)
//! and the bench's fill levels, read back over the dashboard route with the window's right edge
//! named, so every bucket, breach, segment, median and bin is known before the test runs (a
//! downsample's very points, a state held from a day before, two objects on one timeline); then
//! the page itself, run in a headless browser, read back against the dashboard it fetched, every
//! mark's extent with it (a capped panel's note among them); and the cells read once each while
//! rows arrive.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::browser::Browser;
use common::*;
use serde_json::{Value, json};
use veredas_core::SeriesType;
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
id = "under"
label = "Under"
value = { source = "plc", series = "fill1" }
unit = "%"
show = "level"
range = [85, 120]

[[object]]
id = "dense"
label = "Dense"
value = { source = "sorter", series = "dv" }

[[object]]
id = "off"
label = "Off"
value = { source = "sorter", series = "travel" }
states = [ { above = 0, state = "disabled", word = "off" } ]

[[dashboard]]
name = "Numbers"

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
objects = ["outlet-*", "over", "under"]
ticks = [80, 90]

[[dashboard.panel]]
kind = "table"
table = "outlets"
columns = [ { name = "outlet" }, { name = "sorted", bar = true, states = [ { above = 2, state = "warn", word = "busy" } ] }, { name = "travel", decimals = 1, states = [ { above = 14, state = "warn", word = "slow" } ] } ]
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

[[dashboard.panel]]
kind = "trend"
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

/// The binding's aggregate words with their kinds, from the crate's own enum: an aggregate added
/// there stops this match compiling until it is listed.
fn every_aggregate() -> Vec<(&'static str, Aggregate)> {
    let all = [("mean", Aggregate::Mean), ("sum", Aggregate::Sum), ("count", Aggregate::Count), ("min", Aggregate::Min), ("max", Aggregate::Max), ("last", Aggregate::Last)];
    for (_, a) in all {
        match a {
            Aggregate::Mean | Aggregate::Sum | Aggregate::Count | Aggregate::Min | Aggregate::Max | Aggregate::Last => {}
        }
    }
    all.to_vec()
}

/// The vocabulary's states, from the crate's own enum, likewise.
fn every_state() -> Vec<&'static str> {
    use veredas_supervisao::state::StateClass as S;
    let all = [S::Normal, S::Near, S::Warn, S::Alarm, S::Disabled, S::Stale];
    for s in all {
        match s {
            S::Normal | S::Near | S::Warn | S::Alarm | S::Disabled | S::Stale => {}
        }
    }
    all.iter().map(|s| s.name()).collect()
}

/// A second dashboard writing what the first leaves to its defaults, and leaving what the first
/// writes: three columns, every panel a title and a span of its own, a bars panel per aggregate
/// word, and on every kind that takes one a window, a bucket or a limit other than its default; a
/// grid with an object in every state; levels without ticks; tables of every column (more rows
/// than the default limit), of a column relabelled sorted without `descending`, and of named
/// columns unsorted. Which keys each kind takes is the reader's (`every_key_written_both_ways`).
fn keys_dashboard() -> String {
    let mut out = String::from("\n[[dashboard]]\nname = \"Keys\"\ncolumns = 3\n");
    for (word, _) in every_aggregate() {
        out.push_str(&format!("\n[[dashboard.panel]]\nkind = \"bars\"\nobject = \"travel\"\naggregate = \"{word}\"\ntitle = \"{word}\"\nspan = 2\n"));
    }
    out.push_str(
        "\n[[dashboard.panel]]\nkind = \"trend\"\nobject = \"rate\"\nminutes = 20\ntitle = \"Rate, 20 min\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"table\"\ntable = \"outlets\"\nlimit = 2\ncolumns = [ { name = \"outlet\" }, { name = \"travel\", decimals = 1 } ]\nsort = { column = \"travel\", descending = true }\ntitle = \"Two outlets\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"timeline\"\nobjects = [\"line\"]\nminutes = 20\ntitle = \"Line, 20 min\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"value\"\nobject = \"rate\"\nminutes = 10\ntitle = \"Rate tile\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"cells\"\ntable = \"sorts\"\nkey = \"outlet\"\nvalue = \"travel\"\nspread = 5\nminutes = 20\nbucket_minutes = 10\ntitle = \"Cells, 20 min\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"grid\"\nobjects = [\"outlet-*\", \"low-rate\", \"rate-fresh\", \"off\"]\ntitle = \"Every state\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"bars\"\nobject = \"travel\"\nminutes = 20\nbucket_minutes = 10\ntitle = \"Bars, 20 min\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"levels\"\nobjects = [\"outlet-*\"]\ntitle = \"No ticks\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"table\"\ntable = \"dense\"\ntitle = \"Every column\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"table\"\ntable = \"sorts\"\ncolumns = [ { name = \"outlet\", label = \"Outlet no.\" }, { name = \"travel\" } ]\nsort = { column = \"outlet\" }\ntitle = \"By outlet\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"table\"\ntable = \"sorts\"\ncolumns = [ { name = \"travel\" }, { name = \"outlet\" } ]\ntitle = \"Unsorted\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"value\"\nobject = \"rate\"\ntarget = -10\ntitle = \"Against -10\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"value\"\nobject = \"rate\"\ntarget = 0\ntitle = \"Against 0\"\nspan = 2\n\
         \n[[dashboard.panel]]\nkind = \"table\"\ntable = \"notes\"\ncolumns = [ { name = \"word\", states = [ { equals = \"b\", state = \"warn\", word = \"bee\" } ] }, { name = \"n\" }, { name = \"ts\" }, { name = \"index_ms\" } ]\nsort = { column = \"word\" }\ntitle = \"Every type\"\nspan = 3\n\
         \n[[dashboard.panel]]\nkind = \"cells\"\ntable = \"notes\"\nkey = \"word\"\nvalue = \"n\"\nspread = 5\ntitle = \"Keyed by text\"\nspan = 3\n",
    );
    out
}

/// The keys a table of the binding takes, as the reader lists them refusing one it does not
/// (`unknown field `zz`, expected one of ...`).
fn reader_keys(err: &str) -> BTreeSet<String> {
    let at = err.find("expected ").unwrap_or_else(|| panic!("{err}"));
    err[at..].lines().next().unwrap().split('`').skip(1).step_by(2).map(String::from).collect()
}

/// The keys each kind and table requires: written everywhere, so no default to leave.
const REQUIRED: &[(&str, &[&str])] = &[
    ("dashboard", &["name", "panel"]),
    ("value", &["kind", "object"]),
    ("trend", &["kind", "object"]),
    ("bars", &["kind", "object"]),
    ("timeline", &["kind", "objects"]),
    ("grid", &["kind", "objects"]),
    ("levels", &["kind", "objects"]),
    ("table", &["kind", "table"]),
    ("cells", &["kind", "table", "key", "value", "spread"]),
    ("column", &["name"]),
    ("sort", &["column"]),
    // A limit is one of its two keys: each written somewhere, neither required.
    ("limit", &[]),
];

/// Every key the reader takes in a dashboard, in a panel of each kind, in a table's column and
/// sort and in a trend's limit, read from the reader's own refusal; and the test's binding read
/// as TOML: every key the reader takes and does not require is written somewhere in its table and
/// left to its default somewhere else (a limit's two keys each written), so a key added to the
/// reader is driven both ways here or the test fails.
fn every_key_written_both_ways(ex: &Example, dir: &std::path::Path) {
    let refusal = |binding: String| {
        let mut e = ex.clone();
        e.binding = binding;
        let sub = tempfile::tempdir_in(dir).unwrap();
        let path = write_example(sub.path(), &e);
        config::load_file(&path).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("the misspelled key was taken"))
    };
    let at = |from: &str, to: &str| {
        assert_eq!(ex.binding.matches(from).count() >= 1, true, "{from}");
        refusal(ex.binding.replacen(from, to, 1))
    };
    let mut listed: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    listed.insert("dashboard", reader_keys(&at("name = \"Numbers\"\n", "name = \"Numbers\"\nzz = 1\n")));
    for kind in ["value", "trend", "bars", "timeline", "grid", "levels", "table", "cells"] {
        let head = format!("[[dashboard.panel]]\nkind = \"{kind}\"\n");
        listed.insert(kind, reader_keys(&at(&head, &format!("{head}zz = 1\n"))));
    }
    listed.insert("column", reader_keys(&at("{ name = \"outlet\" }", "{ name = \"outlet\", zz = 1 }")));
    listed.insert("sort", reader_keys(&at("sort = { column = \"travel\", descending = true }", "sort = { column = \"travel\", descending = true, zz = 1 }")));
    listed.insert("limit", reader_keys(&at("limit = { below = 98 }", "limit = { below = 98, zz = 1 }")));
    assert_eq!(listed.keys().copied().collect::<BTreeSet<_>>(), REQUIRED.iter().map(|(t, _)| *t).collect::<BTreeSet<_>>());
    // Every table of each kind as the test writes it.
    let doc: toml::Table = toml::from_str(&ex.binding).unwrap();
    let mut tables: BTreeMap<&str, Vec<toml::Table>> = BTreeMap::new();
    for d in doc["dashboard"].as_array().unwrap() {
        let d = d.as_table().unwrap();
        tables.entry("dashboard").or_default().push(d.clone());
        for p in d["panel"].as_array().unwrap() {
            let p = p.as_table().unwrap();
            let kind = p["kind"].as_str().unwrap();
            let kind: &str = listed.keys().copied().find(|k| *k == kind).unwrap();
            tables.entry(kind).or_default().push(p.clone());
            for c in p.get("columns").and_then(|c| c.as_array()).into_iter().flatten() {
                tables.entry("column").or_default().push(c.as_table().unwrap().clone());
            }
            for (key, table) in [("sort", "sort"), ("limit", "limit")] {
                if let Some(t) = p.get(key).and_then(|t| t.as_table()) {
                    tables.entry(table).or_default().push(t.clone());
                }
            }
        }
    }
    for (table, required) in REQUIRED {
        let keys = &listed[table];
        assert!(required.iter().all(|k| keys.contains(*k)), "{table}: {keys:?}");
        let written = &tables[table];
        for key in keys.iter().filter(|k| !required.contains(&k.as_str())) {
            assert!(written.iter().any(|t| t.contains_key(key)), "`{key}` of a {table} never written");
            if *table != "limit" {
                assert!(written.iter().any(|t| !t.contains_key(key)), "`{key}` of a {table} never left to its default");
            }
        }
    }
}

#[test]
fn every_panel_kind_is_computed_exactly_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    ex.project = ex.project.replace("writes_armed = false", "writes_armed = false\nidle_after_ms = 600000");
    ex.project.push_str("\n[[rule]]\nname = \"states\"\npattern = \"STATE code=(?P<code>\\\\d+)\"\nscope = \"sorter\"\ntypes = { code = \"int\" }\n");
    ex.project.push_str("\n[[rule]]\nname = \"dense\"\npattern = \"DENSE v=(?P<dv>[0-9.]+)\"\nscope = \"sorter\"\ntypes = { dv = \"float\" }\n");
    // Notes: a word (text) and a number (float), each stored with its stamp (a time and an int).
    ex.project.push_str("\n[[rule]]\nname = \"notes\"\npattern = 'NOTE (?P<word>\\w+) n=(?P<n>[0-9.]+)'\nscope = \"sorter\"\ntypes = { word = \"text\", n = \"float\" }\n");
    ex.binding = format!("{BINDING}{}", keys_dashboard());
    every_key_written_both_ways(&ex, dir.path());
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
    // The notes, their words out of order.
    append(&log, &[format!("{} NOTE c n=3", stamp(at(1.0))), format!("{} NOTE a n=1", stamp(at(3.0))), format!("{} NOTE b n=2", stamp(at(5.0))), format!("{} NOTE a n=1.5", stamp(at(7.0)))]);
    let to = at(31.0);
    let rows = |t: &str| live.store.query_rows(t, &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || rows("sorts") == 9 && rows("states") == 5 && rows("dense") == 400 && rows("notes") == 4), "the lines are stored");
    let get_d = || get(addr, &format!("/s/loop/dashboard/0?to={to}")).1;
    // The feed's view of the latest values, and the fills, before the tiles are read.
    assert!(wait(Duration::from_secs(20), || {
        let d = get_d();
        panel(&d, 0)["text"] == json!("99.2") && panel(&d, 7)["cells"][2]["text"] == json!("100")
    }), "{}", get_d());
    let d = get_d();
    assert_eq!((d["name"].clone(), d["columns"].clone(), d["to"].clone(), d["panels"].as_array().unwrap().len()), (json!("Numbers"), json!(4), json!(to), 17));
    let heads: Vec<(String, String, i64)> = d["panels"].as_array().unwrap().iter().map(|p| (p["kind"].as_str().unwrap().to_string(), p["title"].as_str().unwrap().to_string(), p["span"].as_i64().unwrap())).collect();
    assert_eq!(heads[..4].iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), ["Read rate", "Travel", "Low rate", "Fresh rate"]);
    assert_eq!(heads[4..].iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["trend", "bars", "timeline", "grid", "levels", "table", "cells", "table", "trend", "value", "trend", "value", "trend"]);
    // Every title and span left to its default: the object's label for a panel of one object, the
    // table's name for a table's, else the kind's own words; one column each.
    assert_eq!(heads.iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), [
        "Read rate", "Travel", "Low rate", "Fresh rate", "Read rate", "Travel", "State over time", "States", "Levels", "outlets", "sorts", "outlets", "Travel", "Read rate", "Dense", "Dense", "Dense",
    ]);
    assert!(heads.iter().all(|h| h.2 == 1), "{heads:?}");
    // A trend with no limit: no breaches, no limit, its every point counted (the 400, not the line's
    // 300).
    let free = panel(&d, 16);
    assert_eq!((free["total"].clone(), free["limit"].clone(), free["breaches"].clone(), free["points"].as_array().unwrap().len()), (json!(400), Value::Null, json!([]), veredas_supervisao::panel::TREND_POINTS));
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
    assert_eq!(tr["limit_text"], json!("98"), "the limit in full, as the binding writes it");

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
    assert_eq!(panel(&d, 7)["not_normal"], json!(2), "near and warn: two not normal");
    let levels: Vec<(Value, Value, Value)> = panel(&d, 8)["rows"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["fraction"].clone(), r["ticks"].clone())).collect();
    assert_eq!(levels, [
        (json!("outlet-1"), json!(0.35), json!([0.8, 0.9])),
        (json!("outlet-2"), json!(0.85), json!([0.8, 0.9])),
        (json!("outlet-3"), json!(0.5), json!([0.3, 0.4])),
        // A value and a tick past the range's end, held at its end; and before its start, held at
        // its start.
        (json!("over"), json!(1.0), json!([30.0 / 35.0, 1.0])),
        (json!("under"), json!(0.0), json!([0.0, 5.0 / 35.0])),
    ]);

    // The table: the view's rows sorted by the count, the count drawn as a bar against the
    // largest shown, the travel to one place with its state word.
    let tb = panel(&d, 9);
    assert_eq!((tb["total"].clone(), tb["columns"].clone()), (json!(3), json!([{ "name": "outlet", "label": "outlet", "bar": false }, { "name": "sorted", "label": "sorted", "bar": true }, { "name": "travel", "label": "travel", "bar": false }])));
    // A column with a bar and rules both: each cell its bar and its state and word.
    let shown: Vec<(Value, Value, Value, Value, Value, Value)> = tb["rows"].as_array().unwrap().iter().map(|r| (r[0]["text"].clone(), r[1]["text"].clone(), r[1]["bar"].clone(), r[1]["word"].clone(), r[2]["text"].clone(), r[2]["word"].clone())).collect();
    // Sorted by travel, descending: an order the view's own (by outlet) does not give.
    assert_eq!(shown, [
        (json!("3"), json!("2"), json!(0.5), json!(""), json!("20.5"), json!("slow")),
        (json!("2"), json!("3"), json!(0.75), json!("busy"), json!("14.0"), json!("")),
        (json!("1"), json!("4"), json!(1.0), json!("busy"), json!("11.5"), json!("")),
    ]);
    assert_eq!((tb["rows"][1][1]["state"].clone(), tb["rows"][0][1]["state"].clone()), (json!("warn"), json!("normal")));
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
    assert_eq!(up["limit_text"], json!("16"));
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
    // The second dashboard, every key written other than its default and read through the route:
    // a bars panel per aggregate word, each bucket of five minutes holding the travel's samples
    // aggregated as its word says (an empty one counted 0, else empty).
    let (_, k) = get(addr, &format!("/s/loop/dashboard/1?to={to}"));
    assert_eq!((k["name"].clone(), k["columns"].clone(), k["panels"].as_array().unwrap().len()), (json!("Keys"), json!(3), 21));
    // Every title and span as written.
    let written: Vec<(Value, Value)> = k["panels"].as_array().unwrap().iter().map(|p| (p["title"].clone(), p["span"].clone())).collect();
    let mut want: Vec<(Value, Value)> = every_aggregate().iter().map(|(w, _)| (json!(w), json!(2))).collect();
    for (t, s) in [("Rate, 20 min", 3), ("Two outlets", 2), ("Line, 20 min", 3), ("Rate tile", 2), ("Cells, 20 min", 3), ("Every state", 2), ("Bars, 20 min", 3), ("No ticks", 2), ("Every column", 3), ("By outlet", 2), ("Unsorted", 3), ("Against -10", 2), ("Against 0", 2), ("Every type", 3), ("Keyed by text", 3)] {
        want.push((json!(t), json!(s)));
    }
    assert_eq!(written, want);
    // No target written: none drawn.
    assert!((0..6).chain([12]).all(|i| panel(&k, i)["target"].is_null()));
    // Levels without ticks: none on any row.
    let lv: Vec<(Value, Value)> = panel(&k, 13)["rows"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["ticks"].clone())).collect();
    assert_eq!(lv, [(json!("outlet-1"), json!([])), (json!("outlet-2"), json!([])), (json!("outlet-3"), json!([]))]);
    // A table naming no columns: every column of the table, each labelled by its name and drawn
    // without a bar, its rows the store's own page of the default fifty, newest first, unsorted.
    let page = live.store.query_rows("dense", &veredas_portal::store::RowsQuery { limit: 50, ..Default::default() }).unwrap();
    let every = panel(&k, 14);
    assert_eq!(every["columns"], json!(page.columns.iter().map(|c| json!({ "name": c.name, "label": c.name, "bar": false })).collect::<Vec<_>>()));
    assert_eq!((every["total"].clone(), every["rows"].as_array().unwrap().len()), (json!(400), 50));
    let values: Vec<Vec<Value>> = every["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|c| c["value"].clone()).collect()).collect();
    assert_eq!(values, page.rows, "the store's page as it gave it");
    // A column relabelled, sorted by a column with no `descending`: ascending, the newest first
    // among equals (the store's order kept).
    let by = panel(&k, 15);
    assert_eq!(by["columns"], json!([{ "name": "outlet", "label": "Outlet no.", "bar": false }, { "name": "travel", "label": "travel", "bar": false }]));
    let rows: Vec<(Value, Value)> = by["rows"].as_array().unwrap().iter().map(|r| (r[0]["value"].clone(), r[1]["value"].clone())).collect();
    let pair = |o: i64, t: f64| (json!(o), json!(t));
    assert_eq!(rows, [pair(1, 12.0), pair(1, 13.0), pair(1, 11.0), pair(1, 10.0), pair(2, 16.0), pair(2, 14.0), pair(2, 12.0), pair(3, 21.0), pair(3, 20.0)]);
    // Named columns, no sort: the store's order, newest first.
    let un: Vec<(Value, Value)> = panel(&k, 16)["rows"].as_array().unwrap().iter().map(|r| (r[1]["value"].clone(), r[0]["value"].clone())).collect();
    assert_eq!(un, [pair(3, 21.0), pair(1, 12.0), pair(2, 16.0), pair(1, 13.0), pair(3, 20.0), pair(2, 14.0), pair(1, 11.0), pair(2, 12.0), pair(1, 10.0)]);
    // A tile's target of either sign: how far the value is, in percent of the target's size,
    // signed by the side it is on (99.2 is above -10); a target of 0 says nothing.
    assert_eq!((panel(&k, 17)["sub"].clone(), panel(&k, 17)["target"].clone()), (json!("+1092% on the -10 target"), json!(-10.0)));
    assert_eq!((panel(&k, 18)["sub"].clone(), panel(&k, 18)["target"].clone()), (json!(""), json!(0.0)));
    // Every type a column holds (core's `SeriesType`, matched in full, each a column of the notes):
    // each cell's text as the store holds the value (a number as shown, a text and a time as they
    // are), sorted by the words (out of order in the store), a rule comparing a word.
    let column_of = |ty: SeriesType| match ty {
        SeriesType::Int => "index_ms",
        SeriesType::Float => "n",
        SeriesType::Text => "word",
        SeriesType::Time => "ts",
    };
    let every_type = [SeriesType::Int, SeriesType::Float, SeriesType::Text, SeriesType::Time];
    let notes = panel(&k, 19);
    let heads: Vec<&str> = notes["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(every_type.iter().all(|ty| heads.contains(&column_of(*ty))), "{heads:?}");
    let store_notes = live.store.query_rows("notes", &veredas_portal::store::RowsQuery { limit: 10, ..Default::default() }).unwrap();
    let col = |name: &str| store_notes.columns.iter().position(|c| c.name == name).unwrap();
    let ts_of = |ms: i64| store_notes.rows.iter().find(|r| r[col("index_ms")] == json!(ms)).map(|r| r[col("ts")].clone()).unwrap();
    let texts: Vec<Vec<Value>> = notes["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|c| c["text"].clone()).collect()).collect();
    let want_texts: Vec<Vec<Value>> = [("a", "1.5", 7.0), ("a", "1", 3.0), ("b", "2", 5.0), ("c", "3", 1.0)]
        .iter()
        .map(|(w, n, m)| vec![json!(w), json!(n), json!(chrono::DateTime::from_timestamp_millis(ts_of(at(*m)).as_i64().unwrap()).unwrap().format("%Y-%m-%d %H:%M:%S").to_string()), json!(at(*m).to_string())])
        .collect();
    assert_eq!(texts, want_texts, "every type's text, sorted by the words");
    assert!(texts.iter().all(|r| r[2].as_str().is_some_and(|s| !s.is_empty() && s.parse::<f64>().is_err())), "a time as its text: {texts:?}");
    let words: Vec<(Value, Value)> = notes["rows"].as_array().unwrap().iter().map(|r| (r[0]["state"].clone(), r[0]["word"].clone())).collect();
    assert_eq!(words, [(json!("normal"), json!("")), (json!("normal"), json!("")), (json!("warn"), json!("bee")), (json!("normal"), json!(""))], "a word's rule");
    // Charts in cells keyed by a text column.
    let keyed: Vec<(Value, Value, Value)> = panel(&k, 20)["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone(), r["median"].clone())).collect();
    assert_eq!(keyed, [(json!("a"), json!(2), json!(1.25)), (json!("b"), json!(1), json!(2.0)), (json!("c"), json!(1), json!(3.0))]);
    let in_bucket: [&[f64]; 7] = [&[10.0, 12.0, 11.0], &[14.0], &[20.0, 13.0], &[], &[16.0], &[12.0], &[21.0]];
    for (i, (word, agg)) in every_aggregate().into_iter().enumerate() {
        let b = &panel(&k, i)["buckets"];
        let got: Vec<(Value, Value)> = b.as_array().unwrap().iter().skip(6).map(|bk| (bk["value"].clone(), bk["n"].clone())).collect();
        let want: Vec<(Value, Value)> = in_bucket
            .iter()
            .map(|vs| {
                let v = match (agg, vs.is_empty()) {
                    (Aggregate::Count, _) => Some(vs.len() as f64),
                    (_, true) => None,
                    (Aggregate::Mean, _) => Some(vs.iter().sum::<f64>() / vs.len() as f64),
                    (Aggregate::Sum, _) => Some(vs.iter().sum()),
                    (Aggregate::Min, _) => Some(vs.iter().cloned().fold(f64::MAX, f64::min)),
                    (Aggregate::Max, _) => Some(vs.iter().cloned().fold(f64::MIN, f64::max)),
                    (Aggregate::Last, _) => vs.last().copied(),
                };
                (json!(v), json!(vs.len()))
            })
            .collect();
        assert_eq!(got, want, "{word}");
        assert_eq!(k["panels"][i]["title"], json!(word));
    }
    // A trend of twenty minutes: the rate's points from at(11) on.
    let tr = panel(&k, 6);
    assert_eq!((tr["from"].clone(), tr["total"].clone()), (json!(to - 20 * MIN), json!(4)));
    assert_eq!(tr["points"].as_array().unwrap().iter().map(|p| p[1].clone()).collect::<Vec<_>>(), [json!(99.0), json!(98.0), json!(97.5), json!(99.2)]);
    // A table of two rows (the view keeps no order, so which two the limit reads is the store's),
    // sorted between them by travel, descending.
    let tv: Vec<f64> = panel(&k, 7)["rows"].as_array().unwrap().iter().map(|r| r[1]["value"].as_f64().unwrap()).collect();
    assert!(tv.len() == 2 && tv[0] >= tv[1], "{tv:?}");
    // A timeline of twenty minutes: the line stopped at its start (since at(6)), then in fault, then running.
    assert_eq!(panel(&k, 8)["rows"][0]["segments"].as_array().unwrap().iter().map(|g| (g["from"].clone(), g["state"].clone())).collect::<Vec<_>>(), [
        (json!(to - 20 * MIN), json!("warn")),
        (json!(at(14.0)), json!("alarm")),
        (json!(at(29.0)), json!("normal")),
    ]);
    // A tile's sparkline of ten minutes: the rate's last two.
    let tile = panel(&k, 9);
    assert_eq!((tile["from"].clone(), tile["spark"].as_array().unwrap().iter().map(|p| p[1].clone()).collect::<Vec<_>>()), (json!(to - 10 * MIN), vec![json!(97.5), json!(99.2)]));
    // Charts in cells over twenty minutes, the sparkline by ten: per outlet its sorts from at(11).
    let cl = panel(&k, 10);
    let ten = 10 * MIN;
    let rows: Vec<(Value, Value, Value)> = cl["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone(), r["spark"].clone())).collect();
    let spark = |pts: &[(f64, f64)]| {
        let mut per: std::collections::BTreeMap<i64, Vec<f64>> = std::collections::BTreeMap::new();
        for (m, v) in pts {
            per.entry(at(*m).div_euclid(ten) * ten).or_default().push(*v);
        }
        json!(per.into_iter().map(|(s, mut vs)| { vs.sort_by(|a, b| a.partial_cmp(b).unwrap()); json!([s, veredas_supervisao::panel::median(&vs)]) }).collect::<Vec<_>>())
    };
    assert_eq!(cl["from"], json!(to - 20 * MIN));
    assert_eq!(rows, [
        (json!("1"), json!(2), spark(&[(14.0, 13.0), (29.0, 12.0)])),
        (json!("2"), json!(1), spark(&[(20.0, 16.0)])),
        (json!("3"), json!(1), spark(&[(30.5, 21.0)])),
    ]);
    // A grid with an object in every state: every one but the normal counted not normal.
    let gr = panel(&k, 11);
    let states: Vec<Value> = gr["cells"].as_array().unwrap().iter().map(|c| c["state"].clone()).collect();
    assert_eq!(states, ["normal", "near", "warn", "alarm", "stale", "disabled"].map(|s| json!(s)));
    let mut seen: Vec<&str> = states.iter().map(|s| s.as_str().unwrap()).collect();
    seen.sort();
    let mut all = every_state();
    all.sort();
    assert_eq!(seen, all, "every state of the vocabulary in the grid");
    assert_eq!(gr["not_normal"], json!(5));
    // Bars of twenty minutes in buckets of ten on the clock: each bucket ten minutes wide, the
    // window's start inside the first, its end inside the last, each the mean of its samples.
    let bars = panel(&k, 12)["buckets"].as_array().unwrap().clone();
    assert!(bars.iter().all(|b| b["end"].as_i64().unwrap() - b["start"].as_i64().unwrap() == ten), "{bars:?}");
    assert!(bars[0]["start"].as_i64().unwrap() <= to - 20 * MIN && to - 20 * MIN < bars[0]["end"].as_i64().unwrap());
    assert!(bars.last().unwrap()["start"].as_i64().unwrap() < to && to <= bars.last().unwrap()["end"].as_i64().unwrap());
    let samples = [(0.0, 10.0), (2.0, 12.0), (4.0, 11.0), (6.0, 14.0), (10.0, 20.0), (14.0, 13.0), (20.0, 16.0), (29.0, 12.0), (30.5, 21.0)];
    for b in &bars {
        let (s0, s1) = (b["start"].as_i64().unwrap(), b["end"].as_i64().unwrap());
        let vs: Vec<f64> = samples.iter().filter(|(m, _)| (s0..s1).contains(&at(*m))).map(|(_, v)| *v).collect();
        let want = if vs.is_empty() { Value::Null } else { json!(vs.iter().sum::<f64>() / vs.len() as f64) };
        assert_eq!((b["value"].clone(), b["n"].clone()), (want, json!(vs.len())), "{b}");
    }

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
    // A sort stamped now (outlet 2, as the others leave every drawn figure as it stands), so the
    // running bucket of the page's window holds a value and is drawn marked whenever in its five
    // minutes the page reads. The page is open, so the fetch follows the sort at once; within a
    // minute of a bucket's end (the sort's storing, the fetch and the drawing all fit in it), the
    // next bucket is waited for.
    let into = store_now() % (5 * MIN);
    if into > 5 * MIN - 60_000 {
        std::thread::sleep(Duration::from_millis((5 * MIN - into + 500) as u64));
    }
    let sorts = || live.store.query_rows("sorts", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    let had = sorts();
    append(&log, &[format!("{} SORT outlet=2 travel=12.0 rate=99.2", stamp(store_now()))]);
    assert!(wait(Duration::from_secs(20), || sorts() == had + 1), "{}", sorts());
    page.eval(KEEP_DASHBOARD);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let (dash, drawn) = loop {
        let v = page.eval(&drawn_panels());
        if !v.is_null() {
            let want = expected_panels(&v["dash"]);
            if close(&v["drawn"], &json!(want)) && v["columns"] == v["dash"]["columns"] && v["marks"] == json!([]) {
                break (v["dash"].clone(), v["drawn"].clone());
            }
            assert!(std::time::Instant::now() < deadline, "the page never drew its dashboard:\n drawn {:#}\n want {:#}\n marks {}", v["drawn"], json!(want), v["marks"]);
        }
        assert!(std::time::Instant::now() < deadline, "the page never fetched its dashboard");
        std::thread::sleep(Duration::from_millis(200));
    };
    // What it drew, spelled out where the bench's values make it certain.
    assert_eq!(dash["panels"].as_array().unwrap().len(), 17);
    assert_eq!((drawn[7]["right"].clone(), drawn[16]["right"].clone()), (json!("3 · 2 not normal"), json!("400 points")));
    // The cells panel as the page's own fetch was handed it past its cap: the note drawn.
    // (The page reads the live window: its total is what that window held, the last sort in it
    // or not yet.)
    let total = dash["panels"][10]["data"]["total"].clone();
    assert!(drawn[10]["right"].as_str().unwrap().ends_with(&format!(" · newest 5 of {total} rows")) && (total == 9 || total == 10), "{} of {total}", drawn[10]["right"]);
    // The running bucket drawn, and marked: one bar of the page's bars carries it.
    let running = drawn[5]["bars"].as_array().unwrap().iter().filter(|b| b[4] == json!(true)).count();
    assert_eq!(running, 1, "the running bucket drawn marked: {}", drawn[5]["bars"]);
    assert_eq!(drawn[6]["rows"].as_array().unwrap().len(), 2, "two rows on the timeline");
    assert_eq!((drawn[0]["big"].clone(), drawn[0]["box"].clone(), drawn[0]["badge"].clone()), (json!("99.2%"), json!("st-box-normal"), Value::Null));
    assert_eq!((drawn[2]["box"].clone(), drawn[2]["badge"].clone()), (json!("st-box-alarm"), json!("! under 99.5 %")));
    assert_eq!(drawn[7]["cells"], json!([["Outlet 1", "35", "cell st-box-normal", "cell-v mono st-text-normal", "Outlet 1: ok"], ["Outlet 2", "85", "cell st-box-near", "cell-v mono st-text-near", "Outlet 2: near full"], ["Outlet 3", "full", "cell st-box-warn", "cell-v mono st-text-warn", "Outlet 3: full"]]));
    assert_eq!(drawn[9]["rows"][0], json!([["3", "mono", null], ["2", "mono st-text-normal", "width:50.0%"], ["■ slow", "mono st-text-warn", null]]), "sorted by travel, descending");
    assert!(drawn[9]["rows"].as_array().unwrap().iter().any(|r| r[1][0].as_str().is_some_and(|t| t.ends_with(" ■ busy")) && r[1][1] == json!("mono st-text-warn") && r[1][2].as_str().is_some_and(|w| w.starts_with("width:"))), "a bar with its state and word: {}", drawn[9]["rows"]);
    assert_eq!(drawn[11]["error"], json!("`outlets` has no column `nothing` (its columns: outlet, sorted, travel)"));
    // An answer for a dashboard no longer shown is dropped, a dashboard or a refusal: the second
    // dashboard asked and its answer held back while the first is asked again, the first stays
    // drawn, its tab on, read once the page has done with the held answer.
    const FIRST_DRAWN: &str = r#"[[...document.querySelectorAll(".dash-grid > .panel .panel-title")].map((t) => t.textContent), document.querySelector(".subtabs button.on").textContent, document.querySelector(".dash-grid > p.error")?.textContent ?? null]"#;
    for refused in [false, true] {
        page.eval(&format!("window.__hold = '/dashboard/1'; window.__holdError = {refused}; location.hash = 'dashboards/1'; true"));
        page.wait_for("window.__held === false || null", Duration::from_secs(10));
        // While the second is asked, its tab on, the grid holds none of the first's panels: it says
        // the second is asked.
        let asking = page.eval(r#"(() => { const p = document.querySelector(".dash-grid > p.asking"); return [document.querySelectorAll(".dash-grid > .panel").length, p?.textContent ?? null, document.querySelector(".subtabs button.on").textContent, p ? window.__shownMark(p) : null, p ? window.__textAsWritten(p) : null]; })()"#);
        assert_eq!(asking, json!([0, "asking for Keys", "Keys", true, []]), "the first dashboard's panels under the second's tab, or what it says there not shown as written");
        page.eval("location.hash = 'dashboards/0'; true");
        page.wait_for("window.__held === true", Duration::from_secs(10));
        let after = page.eval(FIRST_DRAWN);
        assert_eq!((after[0].as_array().unwrap().len(), after[0][0].clone(), after[1].clone(), after[2].clone()), (17, json!("Read rate"), json!("Numbers"), Value::Null), "the held {} drew over the first: {after}", if refused { "refusal" } else { "answer" });
    }
    // A refusal on time is said in the dashboard's place, and the next refresh draws it again.
    page.eval("location.hash = 'layout'; true");
    page.wait_for("document.querySelector('.dash-grid') ? null : true", Duration::from_secs(10));
    page.eval("window.__failNext = true; location.hash = 'dashboards/0'; true");
    let said = page.wait_for(r#"(() => { const p = document.querySelector(".dash-grid > p.error"); return p ? [p.textContent, window.__shownMark(p), window.__textAsWritten(p)] : null; })()"#, Duration::from_secs(10));
    assert_eq!(said, json!(["Error: probe failure", true, []]), "the refusal said in the dashboard's place, shown as written");
    page.wait_for(r#"document.querySelectorAll(".dash-grid > .panel").length === 17 || null"#, Duration::from_secs(15));
    // The five-second refresh waits for an answer still out rather than overtake it: the first
    // dashboard asked and its answer held back six and a half seconds, the tick at five asks
    // nothing; once the page has read the answer, the next tick asks again.
    page.eval("location.hash = 'layout'; true");
    page.wait_for("document.querySelector('.dash-grid') ? null : true", Duration::from_secs(10));
    page.eval("window.__asks = 0; window.__hold = '/dashboard/0'; window.__holdMs = 6500; location.hash = 'dashboards/0'; true");
    page.wait_for("window.__held === false || null", Duration::from_secs(10));
    page.wait_for("window.__held === true || null", Duration::from_secs(15));
    assert_eq!(page.eval("window.__asks"), json!(1), "a refresh overtook the answer still out");
    page.wait_for("window.__asks >= 2 || null", Duration::from_secs(10));
    // An answer that never comes (held a minute) is given up on at ten seconds and said in the
    // dashboard's place; the next tick asks again and draws it.
    page.eval("location.hash = 'layout'; true");
    page.wait_for("document.querySelector('.dash-grid') ? null : true", Duration::from_secs(10));
    // (What it says is recorded as it is drawn: the tick that asks again can draw over it at once.)
    page.eval(r#"(() => { window.__said = []; window.__saidAt = null; new MutationObserver(() => { const e = document.querySelector(".dash-grid > p.error"); if (e && !window.__said.some((s) => s[0] === e.textContent)) { window.__saidAt = window.__saidAt ?? performance.now(); window.__said.push([e.textContent, window.__shownMark(e), window.__textAsWritten(e)]); } }).observe(document.body, { childList: true, subtree: true, characterData: true }); return true; })()"#);
    page.eval("window.__askAt = []; window.__hold = '/dashboard/0'; window.__holdMs = 60000; location.hash = 'dashboards/0'; true");
    let gave_up = page.wait_for("window.__said.length ? window.__said : null", Duration::from_secs(20));
    assert_eq!(gave_up, json!([["Error: no answer within 10 s", true, []]]), "given up on, said in the dashboard's place, shown as written");
    // Given up on at ten seconds (`ASK_MS`), from the ask to the words, within half a second.
    let after = page.eval("window.__saidAt - window.__askAt[0]").as_f64().unwrap();
    page.eval("window.__askAt = null; true");
    assert!((9500.0..=10_500.0).contains(&after), "given up on {after} ms after the ask");
    page.wait_for(r#"document.querySelectorAll(".dash-grid > .panel").length === 17 || null"#, Duration::from_secs(15));
    // The refresh every five seconds: over sixteen seconds and a half, each ask of the dashboard
    // shown five seconds after the last, within half a second.
    page.eval("window.__askAt = []; true");
    std::thread::sleep(Duration::from_millis(16_500));
    let asked: Vec<f64> = page.eval("window.__askAt").as_array().unwrap().iter().map(|t| t.as_f64().unwrap()).collect();
    assert!(asked.len() >= 3 && asked.windows(2).all(|w| (4500.0..=5600.0).contains(&(w[1] - w[0]))), "asked every five seconds: {asked:?}");
    page.eval("window.__askAt = null; true");
    // The second dashboard (every key written, every aggregate, a grid in every state) and then a
    // probe built from the first's panels: every state of the vocabulary on every surface (a tile,
    // its badge, sub-line and sparkline; a grid cell and its value; a level's fill and figure; a
    // timeline's run and its word; a table's word) and windows of one point and of none (a tile's
    // sparkline, a trend, a cell's sparkline), handed to the page's own fetch. Each drawn as the
    // page received it.
    let drawn_as = |page: &mut Browser, name: &str, hash: &str, probe: Option<&Value>| {
        if let Some(p) = probe {
            page.eval(&format!("window.__probe = {p}; true"));
        }
        page.eval(&format!("location.hash = '{hash}'; true"));
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        // The last drawing of it read, for the failure: a probe is handed over once, so the next
        // refresh draws the dashboard again in its place.
        let mut last: Option<String> = None;
        loop {
            let v = page.eval(&drawn_panels());
            if v["dash"]["name"] == json!(name) {
                let want = expected_panels(&v["dash"]);
                if close(&v["drawn"], &json!(want)) && v["columns"] == v["dash"]["columns"] && v["marks"] == json!([]) {
                    return v;
                }
                last = Some(format!("never drawn as received:\n drawn {:#}\n want {:#}\n marks {}", v["drawn"], json!(want), v["marks"]));
            }
            assert!(std::time::Instant::now() < deadline, "{name}: {}", last.as_deref().unwrap_or("never fetched"));
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    page.eval("location.hash = 'layout'; true");
    drawn_as(&mut page, "Keys", "dashboards/1", None);
    page.eval("location.hash = 'layout'; true");
    let probe = probe_dashboard(&dash);
    let v = drawn_as(&mut page, "Probe", "dashboards/0", Some(&probe));
    let kinds: Vec<&str> = v["drawn"].as_array().unwrap().iter().map(|p| p["kind"].as_str().unwrap()).collect();
    // The probe's panels by state (each with its twin of one point), its 30 cases, and the dashboard's own panels again with every
    // string spaced (and a normal tile, a normal level's row, a refused panel).
    assert_eq!(kinds.len(), every_state().len() * 2 + 30 + dash["panels"].as_array().unwrap().len() + 3, "{kinds:?}");
    // Every place a dashboard writes a state's class, each state painted otherwise than every other
    // there, none in the accent's family or its band of hue but the neutral series (Q22).
    let painted = page.eval(&format!("{STATE_PAINT}({})", serde_json::to_string(&every_state()).unwrap()));
    assert_eq!(painted[1], json!([]), "a state painted as another, or in the accent: {}", painted[0]);
    // The places, each object kind that draws a state: a table's cell, a tile (its badge, figure,
    // sub-line and sparkline inside it), a grid cell (its value inside it), a level's figure and its
    // fill, a timeline's run and its word now.
    assert_eq!(
        painted[0],
        json!(["TR > mono st-text-*", "dash-grid > panel tile st-box-*", "grid-cells > cell st-box-*", "lv-row > lv-v mono st-text-*", "lv-track > lv-fill st-fill-*", "statebar > st-*", "tl-row > tl-now st-text-*"]),
        "the places a dashboard draws a state on"
    );
    // Bars of every sign drawn from 0: a bucket below it among the others, and every bucket and
    // the target below it, each bar inside the chart, the axis labelled at both ends and at 0.
    let n = kinds.len();
    for (p, sign) in [(&v["drawn"][n - 2], "mixed"), (&v["drawn"][n - 1], "under")] {
        let bars = p["bars"].as_array().unwrap();
        assert!(!bars.is_empty() && bars.iter().all(|b| b[1].as_f64().unwrap() >= 10.0 && b[1].as_f64().unwrap() + b[3].as_f64().unwrap() <= 158.0 + 0.05), "{sign}: {bars:?}");
        let labels: Vec<&str> = p["axis"].as_array().unwrap().iter().take(3).map(|a| a[0].as_str().unwrap()).collect();
        assert!(labels[0] != "0" || sign == "under", "{sign}: {labels:?}");
        assert!(labels[1].starts_with('-') && (sign == "under" || labels[2] == "0"), "{sign}: {labels:?}");
    }
    // Each input of a value axis its end: the target the top, then the bottom; nothing but 0, the
    // axis 0 to 1; a trend of no point and no limit 0 to 1; one of a single value that value less
    // and more one, each padded.
    let label = |i: usize, k: usize| v["drawn"][i]["axis"][k][0].as_str().unwrap().to_string();
    let target = |i: usize| probe["panels"][i]["data"]["target"].as_f64().unwrap();
    assert_eq!((label(n - 7, 0), label(n - 7, 1)), (num(&json!(target(n - 7) * 1.1), 1), "0".to_string()), "the target the top");
    assert!(target(n - 7) > top_of(&probe["panels"][n - 7]), "past every bucket");
    assert_eq!(label(n - 6, 1), num(&json!(target(n - 6) * 1.1), 1), "the target the bottom");
    assert_eq!((label(n - 5, 0), label(n - 5, 1)), ("1".to_string(), "0".to_string()), "nothing but 0");
    assert_eq!((0..4).map(|k| label(n - 4, k)).collect::<Vec<_>>(), ["0", "0.3", "0.7", "1"], "no point and no limit");
    assert_eq!((0..4).map(|k| label(n - 3, k)).collect::<Vec<_>>(), ["5.8", "6.6", "7.4", "8.2"], "one value, 7");
    drop(page);
    host.stop();
}

/// Every place under the dashboards a state's class is written (`st-<state>`, `st-<part>-<state>`),
/// read from the document as it stands, by its parent and its class's family: an object's outermost
/// element carrying one (its parts' state classes swapped with it, so it stands in one state whole),
/// every one of them read as the page drew it in its own state, and a probe per state put beside the
/// one of them with the most parts (a badge, a dot), read by what paints it and every part inside
/// it (every property of the browser's computed style whose value holds a colour by the browser's own
/// colour syntax, each where it paints: an SVG shape's fill and stroke; a box's background, shadows,
/// each side of its border and its outline; a text's colour, its glyphs' fill and stroke, its
/// decoration and its shadow, its first letter and first line read alike; a filter's colours; a
/// property holding a colour that no rule reads refused by name; each colour resolved, none of no
/// alpha, nor a shadow of no length: a tile's badge, figure, sub-line and sparkline, a cell's
/// text); every state painted
/// otherwise than every other, and none in the accent's four colours (by red, green and blue) or any
/// colour of its band of hue (220° to 290°, saturation above 0.2), but the neutral series the
/// accent draws (Q22): a sparkline's normal and a table's inline bar.
const STATE_PAINT: &str = r##"((STATES) => {
  const STATE = new RegExp(`^(st-(?:[a-z]+-)?)(${STATES.join("|")})$`);
  const family = (cls) => cls.split(/\s+/).map((c) => c.replace(STATE, "$1*")).join(" ");
  // What paints a part, read from every property of the browser's computed style whose value holds a
  // colour, each counted where it paints: an SVG shape or text its fill and stroke; a box (an HTML
  // element, an outermost SVG) its background, its shadow, each side of its border with a width and
  // a style, its outline with both, its filters; a text of its own its colour, its glyphs' fill and
  // their stroke with a width, its decoration with a line, its emphasis with a style, its shadow. A
  // colour SVG keeps for a gradient's stop or a filter's flood and light, a touch's highlight and the
  // logical sides (the physical ones read) paint nothing here, nor a custom property (it paints
  // through a property that names it, read); any other property holding a colour is refused by name
  // until a rule reads it. A column's rule and a row's (the gaps' decorations) with a style and a
  // width.
  const DRAWS = new Set(["rect", "circle", "ellipse", "path", "line", "polyline", "polygon", "text", "tspan"]);
  const ownText = (e) => [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() !== "");
  const html = (e) => !(e instanceof SVGElement);
  const box = (e) => html(e) || (e instanceof SVGSVGElement && !e.ownerSVGElement);
  const shape = (e) => e instanceof SVGElement && DRAWS.has(e.tagName.toLowerCase());
  const side = (c, s) => parseFloat(c.getPropertyValue(`border-${s}-width`)) > 0 && !["none", "hidden"].includes(c.getPropertyValue(`border-${s}-style`));
  const PAINTS = {
    "color": (e) => html(e) && ownText(e),
    "-webkit-text-fill-color": (e) => html(e) && ownText(e),
    "-webkit-text-stroke-color": (e, c) => html(e) && ownText(e) && parseFloat(c.getPropertyValue("-webkit-text-stroke-width")) > 0,
    "text-decoration-color": (e, c) => ownText(e) && c.textDecorationLine !== "none",
    "text-emphasis-color": (e, c) => ownText(e) && c.getPropertyValue("text-emphasis-style") !== "none",
    "text-shadow": (e) => ownText(e),
    "background-color": box,
    "background-image": box,
    "box-shadow": box,
    "border-top-color": (e, c) => box(e) && side(c, "top"),
    "border-right-color": (e, c) => box(e) && side(c, "right"),
    "border-bottom-color": (e, c) => box(e) && side(c, "bottom"),
    "border-left-color": (e, c) => box(e) && side(c, "left"),
    "outline-color": (e, c) => box(e) && c.outlineStyle !== "none" && parseFloat(c.outlineWidth) > 0,
    "column-rule-color": (e, c) => box(e) && c.columnRuleStyle !== "none" && parseFloat(c.columnRuleWidth) > 0,
    "row-rule-color": (e, c) => box(e) && c.getPropertyValue("row-rule-style") !== "none" && parseFloat(c.getPropertyValue("row-rule-width")) > 0,
    "fill": shape,
    "stroke": shape,
    "filter": () => true,
    "backdrop-filter": box,
    "caret-color": (e) => e.isContentEditable || e.matches("input, textarea"),
    "accent-color": (e) => e.matches("input, progress, meter"),
    "scrollbar-color": box,
  };
  const INERT = new Set(["stop-color", "flood-color", "lighting-color", "-webkit-tap-highlight-color", "border-block-start-color", "border-block-end-color", "border-inline-start-color", "border-inline-end-color"]);
  const root = getComputedStyle(document.documentElement);
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const rgb = (c) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = "rgba(0, 0, 0, 0)"; ctx.fillStyle = c; ctx.fillRect(0, 0, 1, 1); return [...ctx.getImageData(0, 0, 1, 1).data]; };
  // The colours a value holds, by the browser's own colour syntax: every function in it the browser
  // takes for a colour (`CSS.supports`: rgb, hsl, hwb, lab, lch, oklab, oklch, `color()`,
  // `color-mix()` and whatever else it parses as one), a function that is none (a gradient, a
  // filter's drop shadow) read for the colours inside it, and every hex. (A computed value names no
  // colour by a word: the browser writes each as a function.)
  const isColour = new Map();
  const colours = (v) => {
    const out = [];
    for (let i = 0; i < v.length; ) {
      const f = /^[a-z-]+\(/i.exec(v.slice(i, i + 40));
      if (f) {
        let depth = 0, j = i;
        for (; j < v.length; j++) { if (v[j] === "(") depth++; else if (v[j] === ")" && --depth === 0) break; }
        const fn = v.slice(i, j + 1);
        if (!isColour.has(fn)) isColour.set(fn, CSS.supports("color", fn));
        if (isColour.get(fn)) out.push(fn);
        else out.push(...colours(v.slice(i + f[0].length, j)));
        i = j + 1;
        continue;
      }
      const h = /^#[0-9a-f]{3,8}\b/i.exec(v.slice(i, i + 10));
      if (h) { out.push(h[0]); i += h[0].length; continue; }
      i++;
    }
    return out;
  };
  // What a counted value paints, its colours resolved (so one colour written two ways is one
  // paint): none where every colour it holds is of no alpha, nor a shadow of no length (no offset,
  // blur or spread) or of no alpha, each shadow of a list read alone.
  const SHADOWS = new Set(["box-shadow", "text-shadow"]);
  const resolved = (v) => colours(v).reduce((s, tok) => { const [r, g, b, a] = rgb(tok); return s.split(tok).join(`rgba(${r}, ${g}, ${b}, ${+(a / 255).toFixed(3)})`); }, v);
  const painting = (q, v) => {
    const cols = colours(v);
    if (cols.length && cols.every((tok) => rgb(tok)[3] === 0)) return null;
    if (!SHADOWS.has(q)) return resolved(v);
    const kept = [];
    let depth = 0, from = 0;
    for (let i = 0; i <= v.length; i++) {
      if (v[i] === "(") depth++;
      else if (v[i] === ")") depth--;
      if (i === v.length || (v[i] === "," && depth === 0)) {
        const one = v.slice(from, i).trim();
        from = i + 1;
        const lengths = (one.replace(/\([^)]*\)/g, "").match(/-?[\d.]+px/g) || []).map(parseFloat);
        if (colours(one).some((tok) => rgb(tok)[3] > 0) && lengths.some((l) => l !== 0)) kept.push(resolved(one));
      }
    }
    return kept.length ? kept.join(", ") : null;
  };
  const unread = new Set();
  // A part read with its first letter and its first line, where it holds a text of its own (each
  // painted by the same rules as the part).
  const paint = (e, pseudo = null) => {
    const c = getComputedStyle(e, pseudo);
    const got = [];
    for (const q of c) {
      if (q.startsWith("--") || INERT.has(q)) continue;
      const v = c.getPropertyValue(q);
      if (!colours(v).length) continue;
      if (!(q in PAINTS)) unread.add(q);
      else if (PAINTS[q](e, c)) { const p = painting(q, v); if (p !== null) got.push([pseudo ? `${pseudo} ${q}` : q, p]); }
    }
    return got;
  };
  const read = (e) => [...paint(e), ...(html(e) && ownText(e) ? [...paint(e, "::first-letter"), ...paint(e, "::first-line")] : [])];
  const parts = (el) => [el, ...el.querySelectorAll("*")];
  const sig = (el) => parts(el).map((e) => read(e).map(([q, v]) => `${q} ${v}`).join(" | ")).join(" // ");
  const familyRgb = ["--accent", "--accent-tint", "--accent-soft", "--accent-outline"].map((v) => rgb(root.getPropertyValue(v).trim()).slice(0, 3).join(","));
  const ofAccent = (tok) => {
    const p = rgb(tok);
    if (p[3] === 0) return false;
    if (familyRgb.includes(p.slice(0, 3).join(","))) return true;
    const [r, g, b] = p.slice(0, 3).map((x) => x / 255), mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn, l = (mx + mn) / 2;
    if (d === 0) return false;
    const s = d / (1 - Math.abs(2 * l - 1)), h = mx === r ? 60 * (((g - b) / d) % 6) : mx === g ? 60 * ((b - r) / d + 2) : 60 * ((r - g) / d + 4);
    const hue = (h + 360) % 360;
    return hue >= 220 && hue <= 290 && s > 0.2;
  };
  // A place: an object's outermost element whose class names a state (a tile, a grid cell, a
  // timeline's run, a level's fill and its figure, a table's cell), its parts inside it swapped with
  // it, every state class they carry, so the object stands in one state whole.
  const stateful = (e) => (e.getAttribute("class") || "").split(/\s+/).some((c) => STATE.test(c));
  const swap = (root, s) => { for (const e of [root, ...root.querySelectorAll("[class]")]) if (stateful(e)) e.setAttribute("class", family(e.getAttribute("class")).replaceAll("*", s)); };
  const places = new Map();
  for (const el of document.querySelectorAll(".dashboards [class]")) {
    if (!stateful(el)) continue;
    let up = el.parentElement;
    while (up && !up.matches(".dashboards") && !stateful(up)) up = up.parentElement;
    if (up && !up.matches(".dashboards")) continue;
    const where = `${el.parentElement?.getAttribute("class") ?? el.parentElement?.tagName} > ${family(el.getAttribute("class"))}`;
    places.set(where, [...(places.get(where) ?? []), el]);
  }
  const out = [];
  // The accent on every part each place's objects draw, each as the page drew it in its own state (a
  // part the page draws only in some states, or only for some data, among them: a tile's badge, a
  // one-point sparkline's dot).
  const accentIn = (p, s, where) => {
    for (const e of parts(p)) {
      if (s === "normal" && e.closest(".st-line-normal") && p.contains(e.closest(".st-line-normal"))) continue;
      if (e.matches(".inbar-fill")) continue;
      for (const [q, v] of read(e)) for (const tok of colours(v)) if (ofAccent(tok)) out.push(`${where} in ${s}: its ${e.tagName.toLowerCase()}.${e.getAttribute("class") ?? ""}'s ${q} ${tok}, of the accent`);
    }
  };
  const ownState = (el) => (el.getAttribute("class") || "").split(/\s+/).map((c) => c.match(STATE)?.[2]).find((s) => s) ?? null;
  for (const [where, els] of places) for (const el of els) accentIn(el, ownState(el), `${where} as drawn`);
  // Each place's probe the object of it with the most parts the page drew (a badge, a dot), every
  // state painted on it in turn.
  for (const [where, els] of places) {
    const el = els.reduce((a, b) => (parts(b).length > parts(a).length ? b : a));
    const seen = new Map();
    for (const s of STATES) {
      const p = el.cloneNode(true);
      swap(p, s);
      el.after(p);
      const v = sig(p);
      // The neutral series are the accent's (Q22): a normal sparkline's line, a table's inline bar,
      // whose fill shows a value's share in every state, its cell's text the state.
      accentIn(p, s, where);
      p.remove();
      if (seen.has(v)) out.push(`${where} in ${s} painted as in ${seen.get(v)}: ${v.slice(0, 160)}`);
      else seen.set(v, s);
    }
  }
  for (const q of unread) out.push(`a colour the census reads no rule for: ${q}`);
  return [[...places.keys()].sort(), out];
})"##;

/// Kept in the page: the dashboard JSON as the page's own fetch receives it, then the view opened.
/// The cells panel is handed over as a read past its cap (five of its nine rows), so the page's
/// note on it is drawn; the rest as it came.
const KEEP_DASHBOARD: &str = r#"(() => {
  const orig = window.fetch;
  window.fetch = async (...a) => {
    const dash = String(a[0]).includes("/dashboard/");
    // Every dashboard asked, counted.
    if (dash) window.__asks = (window.__asks ?? 0) + 1;
    // And when, while `__askAt` is kept.
    if (dash && window.__askAt) window.__askAt.push(performance.now());
    // A dashboard the test hands over in place of the next one asked (the probes), taken as it is
    // asked, so an answer already on its way never carries it.
    const probe = dash ? window.__probe : null;
    if (probe) window.__probe = null;
    // An answer the test holds back (the address it names) for `__holdMs` (1.5 s unless set), to come after the next one, refused
    // then while `__holdError` is set; `__held` set once the page has read it and done with it
    // (the page's handling runs on the read's own resolution, before the next task).
    const hold = dash && window.__hold && String(a[0]).includes(window.__hold);
    const holdError = hold && window.__holdError;
    const holdMs = window.__holdMs ?? 1500;
    if (hold) { window.__hold = null; window.__holdError = false; window.__held = false; window.__holdMs = null; }
    // The next dashboard asked answered at once with portal's kind of refusal, while `__failNext`.
    const failNow = dash && window.__failNext;
    if (failNow) window.__failNext = false;
    const r = await orig(...a);
    if (!dash) return r;
    if (failNow) return new Response(JSON.stringify({ error: "probe failure" }), { status: 500, headers: { "content-type": "application/json" } });
    let j = await r.json();
    if (probe) j = probe;
    if (hold) {
      await new Promise((ok) => setTimeout(ok, holdMs));
      const late = holdError ? new Response(JSON.stringify({ error: "probe failure, late" }), { status: 500, headers: { "content-type": "application/json" } }) : new Response(JSON.stringify(j), { status: r.status, headers: r.headers });
      const read = late.json.bind(late);
      late.json = async () => { const v = await read(); setTimeout(() => { window.__held = true; }, 0); return v; };
      return late;
    }
    for (const p of j.panels) if (p.kind === "cells" && p.data) { p.data.capped = true; p.data.read = 5; }
    window.__dash = j;
    return new Response(JSON.stringify(j), { status: r.status, headers: r.headers });
  };
  location.hash = "dashboards/0";
  return true;
})()"#;

/// The dashboard the page fetched last and what it drew of every panel, read in one moment: the
/// texts and classes, the charts' geometry (every vertex, dot, bar, run and line) and their axes'
/// labels, the grid's columns as laid out, and every mark a panel paints that is not shown (each
/// element with text of its own, SVG shape, or box painted by its own background or border, error
/// panels included: shown is a box, inside every ancestor that hides its overflow up to the one
/// that scrolls, visible, displayed through every ancestor, and painted with some alpha once every
/// opacity is multiplied with its paint's; a mask, a filter, a font or a cover that takes no
/// pointer is not read, the limit the clause states).
/// `DRAWN_PANELS` with the views' shared census of text as written, and of what hides a mark, in its
/// place.
fn drawn_panels() -> String {
    DRAWN_PANELS.replacen("/*AS_WRITTEN*/", &format!("{AS_WRITTEN_FN}\n{HIDDEN_BY_FN}"), 1)
}

const DRAWN_PANELS: &str = r#"(() => {
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const alpha = (p) => { if (!p || p === "none") return 0; if (p.startsWith("url(")) return referenced(p) ? 1 : 0; ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = "rgba(0, 0, 0, 0)"; ctx.fillStyle = p; ctx.fillRect(0, 0, 1, 1); return ctx.getImageData(0, 0, 1, 1).data[3] / 255; };
  // What a panel paints: every element with text of its own, every SVG shape, every box painted by
  // its own background or border (the panel's own box among them); the hover tip, shown on hover,
  // and an SVG title, a tooltip, apart.
  const SHAPES = new Set(["path", "circle", "rect", "line", "polyline", "polygon", "ellipse", "text"]);
  const ownText = (el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() !== "");
  const border = (c) => Math.max(...["Top", "Right", "Bottom", "Left"].map((s) => parseFloat(c[`border${s}Width`]) > 0 && c[`border${s}Style`] !== "none" ? alpha(c[`border${s}Color`]) : 0));
  // A box the page writes empty (a level or a bar at zero: its inline width 0 %) paints nothing.
  const empty = (el) => !(el instanceof SVGElement) && /(^|;)\s*width:\s*0(\.0+)?%/.test(el.getAttribute("style") ?? "");
  const paints = (el) => {
    if (el.closest(".tip")) return false;
    if (el instanceof SVGElement) return SHAPES.has(el.tagName.toLowerCase());
    if (ownText(el)) return true;
    if (empty(el)) return false;
    const c = getComputedStyle(el);
    return alpha(c.backgroundColor) > 0 || c.backgroundImage !== "none" || border(c) > 0;
  };
  // Clipped: any part of it outside an ancestor that hides its overflow (a label cut at a chart's
  // edge is cut), below the nearest one that scrolls (what scrolls into view shows).
  const clipped = (el, b) => {
    for (let e = el.parentElement; e; e = e.parentElement) {
      const s = getComputedStyle(e);
      const o = [s.overflowX, s.overflowY];
      if (o.some((v) => v === "auto" || v === "scroll")) return false;
      if (o.some((v) => v === "hidden" || v === "clip")) {
        const a = e.getBoundingClientRect();
        if (b.left < a.left - 0.5 || b.right > a.right + 0.5 || b.top < a.top - 0.5 || b.bottom > a.bottom + 0.5) return true;
      }
    }
    return false;
  };
  // A box of no area shows nothing: an HTML element (text, a background, a border) needs both its
  // width and its height; an SVG line or a level text may lie along one of them.
  const shown = (el) => {
    const b = el.getBoundingClientRect();
    if (el instanceof SVGElement ? !(b.width > 0 || b.height > 0) : !(b.width > 0 && b.height > 0)) return false;
    if (clipped(el, b)) return false;
    if (!el.checkVisibility({ contentVisibilityAuto: true, opacityProperty: true, visibilityProperty: true })) return false;
    if (hiddenBy(el)) return false;
    const c = getComputedStyle(el);
    if (c.visibility !== "visible") return false;
    let o = 1;
    for (let e = el; e; e = e.parentElement) { const s = getComputedStyle(e); if (s.display === "none") return false; o *= Number(s.opacity); }
    const paint = el instanceof SVGElement
      ? Math.max(alpha(c.fill) * Number(c.fillOpacity), parseFloat(c.strokeWidth) > 0 && strokeDrawn(c) ? alpha(c.stroke) * Number(c.strokeOpacity) : 0)
      : ownText(el) ? Math.max(alpha(c.webkitTextFillColor || c.color), parseFloat(c.webkitTextStrokeWidth) > 0 ? alpha(c.webkitTextStrokeColor) : 0) : Math.max(alpha(c.backgroundColor), c.backgroundImage !== "none" ? 1 : 0, border(c));
    return o * paint > 0.05;
  };
  const hidden = (p) => [p, ...p.querySelectorAll("*")].filter(paints).filter((el) => !shown(el)).map((el) => `${el.tagName}.${el.getAttribute("class") ?? ""} "${el.textContent.trim().slice(0, 30)}"`);
  // Every element a reading takes, kept: the census below asks every text and shape a panel shows
  // to be one of them, so a mark the page adds (or draws twice) is a mark this test does not read.
  const seen = new Set();
  const see = (el) => { if (el) seen.add(el); return el; };
  // Every hover text a reading takes (an element's `title`), kept for the census alike.
  const titled = new Set();
  const attr = (el, a) => { if (!el) return null; titled.add(el); return el.getAttribute(a); };
  const pts = (path) => path ? (see(path).getAttribute("d").match(/[ML][^ML]*/g) || []).map((s) => s.slice(1).trim().split(/\s+/).map(Number)) : [];
  const num = (el, a) => el ? Number(see(el).getAttribute(a)) : null;
  const txt = (el) => el ? see(el).textContent : null;
  // A text read whole: the element and every element inside it whose words it holds.
  const txtAll = (el) => { if (!el) return null; for (const e of [el, ...el.querySelectorAll("*")]) see(e); return el.textContent; };
  const dot = (svg) => { const c = svg ? svg.querySelector("circle.point") : null; return c ? [num(c, "cx"), num(c, "cy")] : null; };
  const axis = (svg) => [...svg.querySelectorAll("text.axis")].map((t) => [txt(t), num(t, "x"), num(t, "y")]);
  // Text drawn as the page wrote it: the views' shared census (`AS_WRITTEN_FN`).
  /*AS_WRITTEN*/
  window.__shownMark = shown;
  window.__textAsWritten = asWritten;
  const d = window.__dash;
  if (!d) return null;
  const drawn = [...document.querySelectorAll(".dash-grid > .panel")].map((p) => {
    const r = { kind: p.dataset.kind, title: txt(p.querySelector(".panel-title")), span: getComputedStyle(p).gridColumnStart };
    r.hidden = hidden(p);
    const err = p.querySelector(":scope > p.error");
    if (err) { r.error = txt(err); r.right = txt(p.querySelector(".panel-bind")); return r; }
    switch (r.kind) {
      case "value": {
        r.box = [...p.classList].find((c) => c.startsWith("st-box-")) ?? null;
        r.big = txtAll(p.querySelector(".big")); r.big_cls = p.querySelector(".big").className;
        r.id = txt(p.querySelector(".panel-bind > .mono"));
        r.badge = txt(p.querySelector(".panel-head .badge")); r.sub = txt(p.querySelector(".sub"));
        r.badge_cls = p.querySelector(".panel-head .badge")?.className ?? null; r.sub_cls = p.querySelector(".sub").className;
        const sp = p.querySelector("svg.spark");
        r.spark = pts(sp.querySelector("path.line")); r.spark_cls = sp.getAttribute("class"); r.spark_dot = dot(sp); break;
      }
      case "trend": {
        const svg = p.querySelector("svg.trend");
        r.line = pts(svg.querySelector("path.line"));
        const pt = svg.querySelector("circle.point");
        r.point = pt ? [num(pt, "cx"), num(pt, "cy"), num(pt, "r")] : null;
        r.breaches = [...svg.querySelectorAll("circle.breach")].map((c) => [num(c, "cx"), num(c, "cy"), num(c, "r")]);
        const lim = svg.querySelector("line.limit");
        r.limit = lim ? [num(lim, "x1"), num(lim, "x2"), num(lim, "y1"), num(lim, "y2")] : null;
        r.axis = axis(svg);
        r.right = txt(p.querySelector(".panel-bind")); break;
      }
      case "bars": {
        const svg = p.querySelector("svg.bars");
        r.bars = [...svg.querySelectorAll("rect.bar")].map((b) => [num(b, "x"), num(b, "y"), num(b, "width"), num(b, "height"), b.classList.contains("running"), txt(b.querySelector("title"))]);
        const tg = svg.querySelector("line.limit");
        r.target = tg ? [num(tg, "x1"), num(tg, "x2"), num(tg, "y1"), num(tg, "y2")] : null; r.right = txt(p.querySelector(".panel-bind"));
        r.axis = axis(svg); break;
      }
      case "timeline":
        r.rows = [...p.querySelectorAll(".tl-row")].map((x) => [txt(x.querySelector(".tl-label")), txt(x.querySelector(".tl-now")), x.querySelector(".tl-now").className,
          [...x.querySelectorAll("svg.statebar rect:not(.st-none)")].map((g) => [g.getAttribute("class"), num(g, "x"), num(g, "width"), num(g, "y"), num(g, "height"), txt(g.querySelector("title"))]),
          ((g) => g ? [num(g, "x"), num(g, "width"), num(g, "y"), num(g, "height")] : null)(x.querySelector("svg.statebar rect.st-none"))]);
        r.axis = [...p.querySelectorAll(".tl-axis span")].map(txt); break;
      case "grid":
        r.cells = [...p.querySelectorAll(".grid-cells .cell")].map((c) => [see(c) && txt(c.querySelector(".cell-id")), txt(c.querySelector(".cell-v")), c.className, c.querySelector(".cell-v").className, attr(c, "title")]);
        r.right = txt(p.querySelector(".panel-bind")); break;
      case "levels":
        r.rows = [...p.querySelectorAll(".lv-row")].map((x) => [see(x.querySelector(".lv-track")) && txt(x.querySelector(".lv-label")), see(x.querySelector(".lv-fill")).getAttribute("style"), x.querySelector(".lv-fill").className, [...x.querySelectorAll(".lv-tick")].map((t) => see(t).getAttribute("style")), txt(x.querySelector(".lv-v")), x.querySelector(".lv-v").className]); break;
      case "table":
        r.head = [...p.querySelectorAll("thead th")].map(txt);
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => [...tr.children].map((td) => [txtAll(td), td.className, td.querySelector(".inbar-fill")?.getAttribute("style") ?? null]));
        r.right = txt(p.querySelector(".panel-bind")); break;
      case "cells":
        r.head = [...p.querySelectorAll("thead th")].map(txt);
        r.rows = [...p.querySelectorAll("tbody tr")].map((tr) => { const td = [...tr.children]; return [txt(td[0]), txt(td[1]), see(td[2]) && pts(td[2].querySelector("path.line")), see(td[3]) && pts(td[3].querySelector("path.line")), txt(td[4]), txt(td[5]), dot(td[2].querySelector("svg"))]; });
        r.right = txt(p.querySelector(".panel-bind")); break;
    }
    return r;
  });
  // Every mark a chart paints read: each SVG shape one of the kinds its chart draws, and those the
  // readings above leave held to the frame they stand in (a sparkline's dot by its size, a trend's
  // grid lines at its value labels, a distribution's area its line closed down to the base and its
  // median at the middle).
  const KINDS = {
    spark: ["path.line", "circle.point"],
    trend: ["line.grid", "text.axis", "line.limit", "path.line", "circle.point", "circle.breach", "circle.tipdot", "text.tiptext"],
    bars: ["text.axis", "rect.bar", "rect.bar running", "line.limit"],
    dist: ["path.area", "path.line", "line.median"],
  };
  const marks = [];
  // The boxes the readings take the first of, one of each: a level's track and its fill, a cell's bar.
  for (const x of document.querySelectorAll(".dash-grid .lv-row")) { const t = x.querySelectorAll(".lv-track").length, f = x.querySelectorAll(".lv-fill").length; if (t !== 1 || f !== 1) marks.push(`levels: a row of ${t} tracks and ${f} fills`); }
  for (const td of document.querySelectorAll(".dash-grid td")) if (td.querySelectorAll(".inbar").length > 1 || td.querySelectorAll(".inbar-fill").length > 1) marks.push("table: a cell of more than one bar");
  for (const svg of document.querySelectorAll(".dash-grid svg")) {
    const kind = ["spark", "trend", "bars", "statebar", "dist"].find((k) => svg.classList.contains(k));
    if (!kind) { marks.push(`an svg of no chart this test reads: ${svg.getAttribute("class")}`); continue; }
    for (const el of svg.querySelectorAll("*")) {
      const tag = el.tagName.toLowerCase();
      if (!SHAPES.has(tag)) continue;
      const k = `${tag}.${el.getAttribute("class") ?? ""}`;
      if (kind === "statebar" ? !/^rect\.st-[a-z]+$/.test(k) : !KINDS[kind].includes(k)) marks.push(`${kind}: a mark this test does not read, ${k}`);
    }
    const vb = svg.viewBox.baseVal;
    if (kind === "spark") for (const c of svg.querySelectorAll("circle.point")) if (num(c, "r") !== 2) marks.push(`spark: its dot of radius ${num(c, "r")}`);
    if (kind === "trend") {
      const grids = [...svg.querySelectorAll("line.grid")].map(see);
      const labels = [...svg.querySelectorAll("text.axis")].filter((t) => t.getAttribute("text-anchor") === "end" && num(t, "y") < vb.height - 6);
      if (grids.length !== labels.length) marks.push(`trend: ${grids.length} grid lines for ${labels.length} value labels`);
      grids.forEach((g, i) => {
        const l = labels[i];
        if (!l || Math.abs(num(g, "y1") - (num(l, "y") - 4)) > 0.01 || num(g, "y1") !== num(g, "y2") || Math.abs(num(g, "x1") - (num(l, "x") + 6)) > 0.01 || Math.abs(num(g, "x2") - (vb.width - 10)) > 0.01) marks.push(`trend: grid line ${i} off its label`);
      });
    }
    if (kind === "dist") {
      const line = svg.querySelector("path.line"), area = see(svg.querySelector("path.area")), med = see(svg.querySelector("line.median"));
      const n = pts(line).length, W = vb.width, H = vb.height, bw = W / n;
      if (!area || area.getAttribute("d") !== `${line.getAttribute("d")} L${W - bw / 2} ${H - 1} L${bw / 2} ${H - 1} Z`) marks.push(`dist: its area not its line closed to the base: ${area?.getAttribute("d")}`);
      if (!med || num(med, "x1") !== W / 2 || num(med, "x2") !== W / 2 || num(med, "y1") !== 0 || num(med, "y2") !== H) marks.push("dist: its median not at the middle");
    }
  }
  // The census: every mark a panel paints (a text, an SVG shape, a box painted by its background or
  // its border) is one a reading above took, one to one (a second of a kind the readings take the
  // first of is unread); every hover text (an SVG
  // title, a `title` attribute) one a reading took; no text the stylesheet generates (before or
  // after an element, a list item's marker) and none it draws otherwise than written (its case
  // changed, its letters masked).
  for (const p of document.querySelectorAll(".dash-grid > .panel")) {
    for (const el of [p, ...p.querySelectorAll("*")]) {
      const at = `${p.dataset.kind}: ${el.tagName}.${el.getAttribute("class") ?? ""}`;
      const shape = el instanceof SVGElement && SHAPES.has(el.tagName.toLowerCase());
      if (el !== p && paints(el) && !el.closest(".tip, title") && shown(el) && !seen.has(el)) {
        marks.push(`${p.dataset.kind}: a mark this test does not read, ${el.tagName}.${el.getAttribute("class") ?? ""} "${el.textContent.trim().slice(0, 30)}"`);
      }
      if (el instanceof SVGElement && el.tagName.toLowerCase() === "title" && !seen.has(el)) marks.push(`${at}: a hover text this test does not read, "${el.textContent.trim().slice(0, 30)}"`);
      if (el.hasAttribute("title") && !titled.has(el)) marks.push(`${at}: a hover text this test does not read, "${el.getAttribute("title").slice(0, 30)}"`);
      for (const said of asWritten(el)) marks.push(`${at}: ${said}`);
    }
  }
  // Where the browser draws each mark is where the page wrote it: no transform on a shape or on any
  // element between it and its chart (nor the chart itself), and the geometry CSS can set on a shape
  // (a rect's, a circle's, an ellipse's, a path's) its attribute's; an HTML box the page places by
  // its inline width or left, by its own percent of what holds it, and moved by no transform.
  const GEOMETRY = { rect: ["x", "y", "width", "height", "rx", "ry"], circle: ["cx", "cy", "r"], ellipse: ["cx", "cy", "rx", "ry"], path: ["d"] };
  const numbers = (s) => (String(s).match(/-?\d*\.?\d+(e[-+]?\d+)?/gi) || []).map(Number);
  const moved = (e) => { const s = getComputedStyle(e); return ["transform", "translate", "rotate", "scale"].some((k) => s[k] && s[k] !== "none") || (s.offsetPath ?? "none") !== "none"; };
  for (const el of document.querySelectorAll(".dash-grid svg *, .dash-grid svg")) {
    const tag = el.tagName.toLowerCase();
    if (!SHAPES.has(tag) && tag !== "svg" && tag !== "g") continue;
    if (moved(el)) marks.push(`${tag}.${el.getAttribute("class") ?? ""}: moved by a transform`);
    for (const a of GEOMETRY[tag] ?? []) {
      if (!el.hasAttribute(a)) continue;
      const css = getComputedStyle(el)[a];
      const want = numbers(el.getAttribute(a)), got = numbers(css);
      if (want.length !== got.length || want.some((w, i) => Math.abs(w - got[i]) > 1e-3)) marks.push(`${tag}.${el.getAttribute("class") ?? ""}: its ${a} drawn ${css}, written ${el.getAttribute(a)}`);
    }
  }
  for (const el of document.querySelectorAll(".dash-grid [style]")) {
    if (el.matches(".dash-grid > .panel")) continue;
    if (moved(el)) marks.push(`${el.className}: moved by a transform`);
    const c = getComputedStyle(el);
    for (const prop of ["width", "left"]) {
      const inline = el.style[prop];
      if (!inline || !inline.endsWith("%")) continue;
      const holder = c.position === "absolute" ? el.offsetParent : el.parentElement;
      const hs = getComputedStyle(holder);
      const room = c.position === "absolute" ? holder.clientWidth : holder.clientWidth - parseFloat(hs.paddingLeft) - parseFloat(hs.paddingRight);
      const want = (parseFloat(inline) / 100) * room, got = parseFloat(c[prop]);
      if (Math.abs(want - got) > 0.5) marks.push(`${el.className}: its ${prop} drawn ${c[prop]}, written ${inline} of ${room}px`);
    }
  }
  // Where and in what order the browser draws each text and row, read from its boxes: every text of
  // a chart at its anchor (its start, middle or end at its x, carried by the chart's own matrix) and
  // on its baseline; a table's heading above its rows, each row below the one before, its cells left
  // to right; a timeline's and a level's rows each below the one before; a grid's cells in reading
  // order.
  const box = (e) => e.getBoundingClientRect();
  for (const t of document.querySelectorAll(".dash-grid svg text")) {
    if (t.closest(".tip") || !t.hasAttribute("x") || !t.textContent.trim()) continue;
    const X = new DOMPoint(Number(t.getAttribute("x")), Number(t.getAttribute("y"))).matrixTransform(t.getScreenCTM());
    const b = box(t), anchor = t.getAttribute("text-anchor") ?? "start";
    const edge = anchor === "end" ? b.right : anchor === "middle" ? (b.left + b.right) / 2 : b.left;
    if (Math.abs(edge - X.x) > 1.5) marks.push(`a chart's text ${JSON.stringify(t.textContent)} drawn with its ${anchor} at ${edge.toFixed(1)}, written at ${X.x.toFixed(1)}`);
    if (!(b.height > 0 && X.y <= b.bottom + 0.5 && (X.y - b.top) / b.height > 0.55)) marks.push(`a chart's text ${JSON.stringify(t.textContent)} off its baseline: [${b.top.toFixed(1)}, ${b.bottom.toFixed(1)}] for ${X.y.toFixed(1)}`);
  }
  const below = (els, what) => { for (let i = 1; i < els.length; i++) if (box(els[i]).top < box(els[i - 1]).bottom - 0.5) marks.push(`${what} ${i} drawn above the one before`); };
  for (const p of document.querySelectorAll(".dash-grid > .panel")) {
    for (const tb of p.querySelectorAll("table")) {
      below([...tb.querySelectorAll("thead tr, tbody tr")], `${p.dataset.kind}: the table's row`);
      for (const tr of tb.querySelectorAll("tr")) { const c = [...tr.children]; for (let i = 1; i < c.length; i++) if (box(c[i]).left < box(c[i - 1]).right - 0.5) marks.push(`${p.dataset.kind}: a cell ${i} drawn left of the one before`); }
    }
    below([...p.querySelectorAll(":scope > .tl-row")], `${p.dataset.kind}: the timeline's row`);
    below([...p.querySelectorAll(":scope > .lv-row")], `${p.dataset.kind}: the level's row`);
    const cells = [...p.querySelectorAll(".grid-cells > .cell")];
    for (let i = 1; i < cells.length; i++) { const a = box(cells[i - 1]), b = box(cells[i]); if (!(b.top >= a.bottom - 0.5 || (Math.abs(b.top - a.top) < 0.5 && b.left >= a.right - 0.5))) marks.push(`grid: a cell ${i} drawn before the one before`); }
  }
  // The dashboards' tabs (the binding's names for them, "its name on the screen"): each shown and
  // drawn as written.
  for (const b of document.querySelectorAll(".subtabs button")) {
    if (!shown(b)) marks.push(`a dashboard's tab not shown: ${JSON.stringify(b.textContent)}`);
    for (const said of asWritten(b)) marks.push(`a dashboard's tab ${JSON.stringify(b.textContent)}: ${said}`);
  }
  // The grid's columns as laid out: one track per column.
  const columns = getComputedStyle(document.querySelector(".dash-grid")).gridTemplateColumns.split(" ").filter(Boolean).length;
  return { dash: d, drawn, columns, marks };
})()"#;

/// A number to `places` as the page's `toFixed` writes it: the exact value of the double rounded,
/// a tie away from zero (Rust's own formatting rounds a tie to even: 1.25 is "1.2" there and "1.3"
/// in the page).
fn to_fixed(x: f64, places: usize) -> String {
    let exact = format!("{:.40}", x.abs());
    let (int, frac) = exact.split_once('.').unwrap();
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes().take(places)).map(|b| b - b'0').collect();
    if frac.as_bytes()[places] >= b'5' {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let s: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let (whole, part) = s.split_at(s.len() - places);
    let body = if places == 0 { whole.to_string() } else { format!("{whole}.{part}") };
    if x < 0.0 && body.bytes().any(|b| b != b'0' && b != b'.') { format!("-{body}") } else { body }
}

/// A number as the page writes it: to `places`, trailing zeros and a bare point dropped.
fn num(v: &Value, places: usize) -> String {
    match v.as_f64() {
        Some(x) if x.is_finite() => {
            let t = to_fixed(x, places);
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

/// A dashboard built from the first one's panels as the page received it: a value tile in every
/// state of the vocabulary, tiles with a sparkline of one point and of none, a tile with no value
/// (its figure a dash), trends of one point
/// and of none, a grid, levels, a timeline (one run in every state, each row's word in one) and a
/// table (a word in every state) each across every state, and a cells panel whose first row's
/// sparkline has one point.
fn probe_dashboard(d: &Value) -> Value {
    let states = every_state();
    let p = |i: usize| d["panels"][i].clone();
    // Each state's texts given white space of their own (two spaces, a leading and a trailing one, a
    // tab, a line break), so a text drawn in any state is read so on every surface that draws one.
    let word = |s: &str| json!(format!("  {s}\tword\n+ "));
    let mut panels = Vec::new();
    for s in &states {
        let mut t = p(0);
        t["data"]["state"] = json!(s);
        t["data"]["word"] = word(s);
        t["data"]["mark"] = json!("!");
        t["data"]["sub"] = json!(format!("  {s}\tsub\n+ "));
        t["data"]["text"] = json!(format!("  {s}\t1\n+ "));
        spaced(&mut t["title"]);
        spaced(&mut t["data"]);
        // And its twin of one point, its sparkline a dot, drawn in every state.
        let mut dot = t.clone();
        dot["data"]["spark"] = json!([t["data"]["spark"][0].clone()]);
        panels.push(t);
        panels.push(dot);
    }
    let mut one = p(0);
    one["data"]["spark"] = json!([one["data"]["spark"][0].clone()]);
    panels.push(one);
    let mut none = p(0);
    none["data"]["spark"] = json!([]);
    panels.push(none);
    let mut blank = p(0);
    blank["data"]["text"] = Value::Null;
    panels.push(blank);
    let mut one = p(4);
    one["data"]["points"] = json!([one["data"]["points"][0].clone()]);
    one["data"]["breaches"] = json!([]);
    // A limit of four places, drawn as the host writes it, never rounded by the page.
    one["data"]["limit"] = json!({ "below": 97.0625 });
    one["data"]["limit_text"] = json!("97.0625");
    panels.push(one);
    let mut none = p(4);
    none["data"]["points"] = json!([]);
    none["data"]["breaches"] = json!([]);
    none["data"]["total"] = json!(0);
    panels.push(none);
    // A cell and a level with no value: the host's own form (stale, "no value", no text, a level no
    // fraction) and a normal one with no text, so every fallback of the page the host can reach is
    // drawn.
    let mut g = p(7);
    let c0 = g["data"]["cells"][0].clone();
    let mut cells: Vec<Value> = states.iter().map(|s| { let mut c = c0.clone(); c["state"] = json!(s); c["word"] = word(s); c["label"] = json!(format!("  Cell {s}\t+\n+ ")); c["text"] = json!(format!("  {s}\t2\n+ ")); c }).collect();
    for (label, state, word) in [("Cell gone", "stale", "no value"), ("Cell blank", "normal", "ok")] {
        let mut c = c0.clone();
        (c["label"], c["state"], c["word"], c["text"]) = (json!(label), json!(state), json!(word), Value::Null);
        cells.push(c);
    }
    g["data"]["not_normal"] = json!(cells.iter().filter(|c| c["state"] != "normal").count());
    g["data"]["cells"] = json!(cells);
    spaced(&mut g["title"]);
    spaced(&mut g["data"]);
    panels.push(g);
    let mut l = p(8);
    let r0 = l["data"]["rows"][0].clone();
    let mut rows: Vec<Value> = states.iter().map(|s| { let mut r = r0.clone(); r["state"] = json!(s); r["word"] = word(s); r["label"] = json!(format!("  Level {s}\t+\n+ ")); r["text"] = json!(format!("  {s}\t3\n+ ")); r }).collect();
    for (label, state, word) in [("Level gone", "stale", "no value"), ("Level blank", "normal", "ok")] {
        let mut r = r0.clone();
        (r["label"], r["state"], r["word"], r["text"], r["fraction"]) = (json!(label), json!(state), json!(word), Value::Null, Value::Null);
        rows.push(r);
    }
    l["data"]["rows"] = json!(rows);
    spaced(&mut l["title"]);
    spaced(&mut l["data"]);
    panels.push(l);
    let mut tl = p(6);
    let (from, to) = (tl["data"]["from"].as_i64().unwrap(), tl["data"]["to"].as_i64().unwrap());
    let step = (to - from) / states.len() as i64;
    let mut row = tl["data"]["rows"][0].clone();
    row["segments"] = json!(states.iter().enumerate().map(|(i, s)| json!({ "from": from + i as i64 * step, "to": from + (i as i64 + 1) * step, "state": s, "mark": "!", "word": word(s) })).collect::<Vec<_>>());
    tl["data"]["rows"] = json!(states.iter().map(|s| { let mut r = row.clone(); r["label"] = json!(format!("  Line {s}\t+\n+ ")); r["now"]["state"] = json!(s); r["now"]["word"] = word(s); r["now"]["mark"] = json!("!"); r }).collect::<Vec<_>>());
    spaced(&mut tl["title"]);
    spaced(&mut tl["data"]);
    panels.push(tl);
    let mut tb = p(9);
    let row0 = tb["data"]["rows"][0].clone();
    // Every state on every cell that draws one: the rules' column and the bar's with rules.
    tb["data"]["rows"] = json!(states.iter().map(|s| { let mut r = row0.clone(); for k in [1, 2] { r[k]["state"] = json!(s); r[k]["word"] = word(s); r[k]["mark"] = json!("!"); } r }).collect::<Vec<_>>());
    spaced(&mut tb["title"]);
    spaced(&mut tb["data"]);
    panels.push(tb);
    // Where no word holds, the page draws a text alone: a cell of a column with rules none of which
    // holds (plain and with a bar: the host's word and mark empty), and a level's value with no unit;
    // each text spaced, its empty word, mark and unit left empty.
    let mut none = p(9);
    let mut row = none["data"]["rows"][0].clone();
    for k in [1, 2] {
        (row[k]["state"], row[k]["word"], row[k]["mark"], row[k]["text"]) = (json!("normal"), json!(""), json!(""), json!(format!("cell {k}")));
    }
    none["data"]["rows"] = json!([row]);
    spaced(&mut none["data"]);
    panels.push(none);
    let mut bare = p(8);
    let mut r = bare["data"]["rows"][0].clone();
    (r["state"], r["unit"], r["text"]) = (json!("normal"), json!(""), json!("7"));
    bare["data"]["rows"] = json!([r]);
    spaced(&mut bare["data"]);
    panels.push(bare);
    let mut cl = p(10);
    cl["data"]["rows"][0]["spark"] = json!([cl["data"]["rows"][0]["spark"][0].clone()]);
    // A spread of four places, drawn as the host writes it, never rounded by the page; and rows left
    // out (no key or no value), said.
    cl["data"]["spread"] = json!(0.0005);
    cl["data"]["spread_text"] = json!("0.0005");
    cl["data"]["left_out"] = json!(2);
    panels.push(cl);
    // A rule's word written with a symbol and a unit's case (`over 3σ in mA`), drawn on the badge as
    // written (Q28); trends of values of seven digits and more and of large negatives, and bars of
    // large values, every value label with room at the chart's left edge (`leftFor`).
    let mut worded = p(0);
    (worded["data"]["state"], worded["data"]["word"], worded["data"]["mark"]) = (json!("warn"), json!("over 3σ in mA"), json!("■"));
    panels.push(worded);
    // Texts whose white space is their own (two spaces, a leading and a trailing one, a tab, a line
    // break), on every kind that draws a text of the binding's or the store's: a tile's title, value,
    // badge word and sub-line; a trend's title; a timeline row's label and word; a grid cell's label
    // and value; a level's label and value; a table's heading and cell; a cells key. Each drawn as written.
    let mut tile = p(0);
    tile["title"] = json!("Fill  A ");
    (tile["data"]["state"], tile["data"]["word"], tile["data"]["mark"], tile["data"]["text"], tile["data"]["sub"]) = (json!("warn"), json!("  over  limit"), json!("!"), json!(" E01\t2"), json!("first\nsecond"));
    panels.push(tile);
    let mut tr = p(4);
    tr["title"] = json!("Trend\ttwo");
    panels.push(tr);
    let mut tl = p(6);
    let mut row = tl["data"]["rows"][0].clone();
    (row["label"], row["now"]["state"], row["now"]["mark"], row["now"]["word"]) = (json!("Line  one"), json!("warn"), json!("!"), json!("  over  it"));
    tl["data"]["rows"] = json!([row]);
    panels.push(tl);
    let mut g = p(7);
    let mut c = g["data"]["cells"][0].clone();
    (c["label"], c["text"]) = (json!("Cell  two\t"), json!(" 5  6"));
    g["data"]["not_normal"] = json!(if c["state"] == "normal" { 0 } else { 1 });
    g["data"]["cells"] = json!([c]);
    panels.push(g);
    let mut l = p(8);
    let mut r = l["data"]["rows"][0].clone();
    (r["label"], r["text"]) = (json!("Level\tthree"), json!("  7"));
    l["data"]["rows"] = json!([r]);
    panels.push(l);
    let mut tb = p(9);
    tb["data"]["rows"][0][0]["text"] = json!("  a  b");
    tb["data"]["columns"][0]["label"] = json!("Out  let");
    panels.push(tb);
    let mut cl = p(10);
    cl["data"]["rows"][0]["key"] = json!(" k  1");
    panels.push(cl);
    // And every text a panel draws, by the panels' own data rather than a list of fields: each of
    // the dashboard's panels again, a tile and a level's row also normal with a unit (a tile's id
    // and a level's value and unit drawn then), every string the host hands it, its title too,
    // given white space of its own (two spaces, a leading and a trailing one, a tab, a line break),
    // the two words that name a kind or a state apart (the page's classes); and a refused panel,
    // its reason so. Each drawn as written and as received.
    fn spaced(v: &mut Value) {
        match v {
            // An empty string left empty: the page's own branch for none taken as the host sends it.
            Value::String(s) if !s.is_empty() => *s = format!("  {s}\t+\n+ "),
            Value::Array(a) => a.iter_mut().for_each(spaced),
            Value::Object(o) => o.iter_mut().filter(|(k, _)| *k != "kind" && *k != "state").for_each(|(_, x)| spaced(x)),
            _ => {}
        }
    }
    let mut every: Vec<Value> = d["panels"].as_array().unwrap().clone();
    let mut tile = p(0);
    (tile["data"]["state"], tile["data"]["unit"]) = (json!("normal"), json!("%"));
    every.push(tile);
    let mut l = p(8);
    (l["data"]["rows"][0]["state"], l["data"]["rows"][0]["unit"]) = (json!("normal"), json!("%"));
    every.push(l);
    let mut refused = p(0);
    refused["error"] = json!("no column `rate`");
    every.push(refused);
    for mut q in every {
        spaced(&mut q["title"]);
        spaced(&mut q["data"]);
        if q.get("error").is_some_and(|e| !e.is_null()) {
            spaced(&mut q["error"]);
        }
        panels.push(q);
    }
    let at = p(4)["data"]["points"].as_array().unwrap().clone();
    for vals in [[123456.7, 131234.5, 127000.25], [-1234.5, -1500.25, -1100.0]] {
        let mut t = p(4);
        t["data"]["total"] = json!(3);
        (t["data"]["points"], t["data"]["breaches"], t["data"]["limit"], t["data"]["limit_text"]) = (json!(vals.iter().enumerate().map(|(i, v)| json!([at[i][0], v])).collect::<Vec<_>>()), json!([]), Value::Null, Value::Null);
        panels.push(t);
    }
    let mut big = p(5);
    for b in big["data"]["buckets"].as_array_mut().unwrap() {
        if let Some(v) = b["value"].as_f64() {
            b["value"] = json!(v * 10000.0);
        }
    }
    (big["data"]["target"], big["data"]["target_text"]) = (Value::Null, Value::Null);
    panels.push(big);
    // Each input of a chart's value axis its end at least once: bars whose target lies past every
    // bucket (above them, then below 0 under them), bars all at 0 with no target (the axis 0 to 1),
    // a trend of no point and no limit (0 to 1), and one whose points are all one value.
    for target in [(top_of(&p(5)) * 2.0).ceil() + 0.25, -7.75] {
        let mut b = p(5);
        (b["data"]["target"], b["data"]["target_text"]) = (json!(target), json!(format!("{target}")));
        panels.push(b);
    }
    let mut zero = p(5);
    for b in zero["data"]["buckets"].as_array_mut().unwrap() {
        if !b["value"].is_null() {
            b["value"] = json!(0.0);
        }
    }
    (zero["data"]["target"], zero["data"]["target_text"]) = (Value::Null, Value::Null);
    panels.push(zero);
    let ends = p(4)["data"]["points"].as_array().unwrap().clone();
    for points in [json!([]), json!([[ends[0][0], 7.0], [ends[ends.len() - 1][0], 7.0]])] {
        let mut t = p(4);
        t["data"]["total"] = json!(points.as_array().unwrap().len());
        (t["data"]["points"], t["data"]["breaches"], t["data"]["limit"], t["data"]["limit_text"]) = (points, json!([]), Value::Null, Value::Null);
        panels.push(t);
    }
    // Bars of every sign: a bucket below 0 among the others; and every bucket and the target below 0.
    let mut mixed = p(5);
    let first = mixed["data"]["buckets"].as_array().unwrap().iter().position(|b| !b["value"].is_null()).unwrap();
    // Two places, so a bar's hover text is read to its two.
    mixed["data"]["buckets"][first]["value"] = json!(-5.25);
    panels.push(mixed);
    let mut under = p(5);
    for b in under["data"]["buckets"].as_array_mut().unwrap() {
        if let Some(v) = b["value"].as_f64() {
            b["value"] = json!(-v);
        }
    }
    under["data"]["target"] = json!(-12.0625);
    under["data"]["target_text"] = json!("-12.0625");
    panels.push(under);
    json!({ "name": "Probe", "columns": d["columns"], "to": d["to"], "panels": panels })
}

/// A time on the store's clock as the page writes it (`clock(t, false)`): hours and minutes.
fn clock(t: f64) -> String {
    chrono::DateTime::from_timestamp_millis(t as i64).unwrap().format("%H:%M").to_string()
}

/// A time on the store's clock as the page writes it with its seconds (`clock(t)`).
fn clock_s(t: f64) -> String {
    chrono::DateTime::from_timestamp_millis(t as i64).unwrap().format("%H:%M:%S").to_string()
}

/// A chart's left edge as the page sizes it (`leftFor`): room for its longest value label, ten
/// units of mono type each glyph at most two thirds of that wide, then 6; never less than 44.
fn left_for(labels: &[String]) -> f64 {
    (44.0f64).max((12.0 + 6.6 * labels.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64).ceil())
}

/// The highest value of a bars panel's buckets.
fn top_of(p: &Value) -> f64 {
    p["data"]["buckets"].as_array().unwrap().iter().filter_map(|b| b["value"].as_f64()).fold(f64::MIN, f64::max)
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
            // Every mark it paints shown: none hidden.
            let mut r = json!({ "kind": p["kind"], "title": p["title"], "span": format!("span {}", p["span"]), "hidden": [] });
            if let Some(e) = p.get("error").filter(|e| !e.is_null()) {
                r["error"] = e.clone();
                r["right"] = p["kind"].clone();
                return r;
            }
            let x = &p["data"];
            match p["kind"].as_str().unwrap() {
                "value" => {
                    r["box"] = json!(format!("st-box-{}", s(&x["state"])));
                    r["big"] = json!(format!("{}{}", x["text"].as_str().unwrap_or("\u{2014}"), s(&x["unit"])));
                    r["big_cls"] = json!(format!("big mono st-text-{}", s(&x["state"])));
                    r["badge"] = if x["state"] == "normal" { Value::Null } else { json!(format!("{} {}", s(&x["mark"]), s(&x["word"]))) };
                    // Its object's id beside the title while it is normal, the badge in its place else.
                    r["id"] = if x["state"] == "normal" { x["id"].clone() } else { Value::Null };
                    r["sub"] = json!(s(&x["sub"]));
                    r["spark"] = json!(spark_at(&pairs_of(&x["spark"]), 260.0, 30.0));
                    // The state on every part of the tile: its badge, its sub-line and its sparkline.
                    let st = s(&x["state"]);
                    r["badge_cls"] = if st == "normal" { Value::Null } else { json!(format!("badge st-{st}")) };
                    r["sub_cls"] = json!(format!("sub st-text-{st}"));
                    r["spark_cls"] = json!(format!("spark st-line-{st}"));
                    // A window of one point a dot in the middle, as the trend draws one.
                    r["spark_dot"] = if pairs_of(&x["spark"]).len() == 1 { json!([130.0, 15.0]) } else { Value::Null };
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
                    // The plot's left edge room for the longest value label (`leftFor`).
                    let left = left_for(&(0..=3).map(|i| num(&json!(lo + (hi - lo) * i as f64 / 3.0), 1)).collect::<Vec<_>>());
                    let px = scale(x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap(), left, 590.0);
                    let py = scale(lo, hi, 178.0, 10.0);
                    r["line"] = json!(if points.len() > 1 { points.iter().map(|(t, v)| json!([px(*t), py(*v)])).collect::<Vec<_>>() } else { Vec::new() });
                    r["point"] = if points.len() == 1 { json!([px(points[0].0), py(points[0].1), 3.0]) } else { Value::Null };
                    // Its axes: four values a third apart, each beside its height, and five times a
                    // quarter apart on the store's clock, the last "now".
                    let (from, to) = (x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap());
                    let mut axis: Vec<Value> = (0..=3).map(|i| { let v = lo + (hi - lo) * i as f64 / 3.0; json!([num(&json!(v), 1), left - 6.0, py(v) + 4.0]) }).collect();
                    axis.extend((0..=4).map(|i| { let t = from + (to - from) * i as f64 / 4.0; json!([if i == 4 { "now".to_string() } else { clock(t) }, px(t), 194.0]) }));
                    r["axis"] = json!(axis);
                    // A breach a dot of radius 3; the limit a line across the plot at its value.
                    r["breaches"] = json!(pairs_of(&x["breaches"]).iter().map(|(t, v)| json!([px(*t), py(*v), 3.0])).collect::<Vec<_>>());
                    r["limit"] = json!(limit.map(|l| json!([left, 590.0, py(l), py(l)])));
                    r["right"] = match limit {
                        None => json!(format!("{} points", x["total"])),
                        Some(_) => {
                            // The limit as the host wrote it, in full (its `limit_text`).
                            let word = if x["limit"].get("above").is_some() { "max" } else { "min" };
                            json!(format!("{word} {}{unit} · {n} breach{}", x["limit_text"].as_str().unwrap(), if n == 1 { "" } else { "es" }))
                        }
                    };
                }
                "bars" => {
                    // 600 by 180, from x 44, from y 158 up to 10, from the lowest to the highest of
                    // 0, the values and the target, a tenth past an end that is not 0; a bar per
                    // bucket with a value, in its slot, from 0 up to its value or down to it.
                    let unit = if s(&x["unit"]).is_empty() { String::new() } else { format!(" {}", s(&x["unit"])) };
                    let buckets = x["buckets"].as_array().unwrap();
                    let target = x["target"].as_f64();
                    let mut vals: Vec<f64> = buckets.iter().map(|b| b["value"].as_f64().unwrap_or(0.0)).collect();
                    vals.extend(target);
                    vals.push(0.0);
                    let top = vals.iter().cloned().fold(f64::MIN, f64::max);
                    let bottom = vals.iter().cloned().fold(f64::MAX, f64::min);
                    let hi = if top > 0.0 { top * 1.1 } else if bottom < 0.0 { 0.0 } else { 1.0 };
                    let lo = if bottom < 0.0 { bottom * 1.1 } else { 0.0 };
                    let py = scale(lo, hi, 158.0, 10.0);
                    // The plot's left edge room for the longest value label (`leftFor`).
                    let label = |v: f64| if v == 0.0 { "0".to_string() } else { num(&json!(v), 1) };
                    let left = left_for(&[label(hi), label(lo), "0".to_string()]);
                    let bw = (590.0 - left) / (buckets.len().max(1) as f64);
                    r["bars"] = json!(buckets.iter().enumerate().filter(|(_, b)| !b["value"].is_null()).map(|(i, b)| {
                        let v = b["value"].as_f64().unwrap();
                        // Its hover text: the bucket's start, its value to two places and its count.
                        let title = format!("{} \u{b7} {} (n {})", clock(b["start"].as_f64().unwrap()), num(&b["value"], 2), b["n"]);
                        json!([left + i as f64 * bw + 2.0, py(v).min(py(0.0)), bw - 4.0, (py(0.0) - py(v)).abs(), b["running"], title])
                    }).collect::<Vec<_>>());
                    r["target"] = json!(target.map(|t| json!([left, 590.0, py(t), py(t)])));
                    // Its axis: the top value and the bottom one (0 written so), 0 at its height
                    // between them, a bucket's start on the store's clock every sixth of them, and
                    // the target's own words.
                    let mut axis = vec![json!([label(hi), left - 6.0, py(hi) + 10.0]), json!([label(lo), left - 6.0, 158.0])];
                    if lo < 0.0 && hi > 0.0 {
                        axis.push(json!(["0", left - 6.0, py(0.0) + 4.0]));
                    }
                    let every = (buckets.len() as f64 / 6.0).ceil() as usize;
                    for (i, b) in buckets.iter().enumerate() {
                        if every > 0 && i % every == 0 {
                            axis.push(json!([clock(b["start"].as_f64().unwrap()), left + i as f64 * bw + 2.0, 174.0]));
                        }
                    }
                    if let Some(t) = target {
                        axis.push(json!([format!("target {}", x["target_text"].as_str().unwrap()), 590.0, py(t) - 4.0]));
                    }
                    r["axis"] = json!(axis);
                    r["right"] = if x["target"].is_null() { Value::Null } else { json!(format!("target {}{unit}", x["target_text"].as_str().unwrap())) };
                }
                "timeline" => {
                    // 400 wide: a run from its start to its end in the window, clipped to it.
                    let px = scale(x["from"].as_f64().unwrap(), x["to"].as_f64().unwrap(), 0.0, 400.0);
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let segs: Vec<Value> = row["segments"].as_array().unwrap().iter().filter_map(|g| {
                            let (x0, x1) = (px(g["from"].as_f64().unwrap()).max(0.0), px(g["to"].as_f64().unwrap()).min(400.0));
                            // Its hover text: its word and its times to the second, as the host gave them.
                            let title = format!("{} {}\u{2013}{}", s(&g["word"]), clock_s(g["from"].as_f64().unwrap()), clock_s(g["to"].as_f64().unwrap()));
                            (x1 > x0).then(|| json!([format!("st-{}", s(&g["state"])), x0, x1 - x0, 0.0, 14.0, title]))
                        }).collect();
                        // Under the runs, the rail of no state the whole window across.
                        json!([row["label"], format!("{} {}", s(&row["now"]["mark"]), s(&row["now"]["word"])), format!("tl-now st-text-{}", s(&row["now"]["state"])), segs, [0.0, 400.0, 0.0, 14.0]])
                    }).collect::<Vec<_>>());
                    r["axis"] = json!([clock(x["from"].as_f64().unwrap()), "now"]);
                }
                "grid" => {
                    r["cells"] = json!(x["cells"].as_array().unwrap().iter().map(|c| {
                        let v = if quiet(&c["state"]) { c["text"].as_str().unwrap_or("\u{2014}").to_string() } else { s(&c["word"]) };
                        // Its hover text: its label and its word.
                        json!([c["label"], v, format!("cell st-box-{}", s(&c["state"])), format!("cell-v mono st-text-{}", s(&c["state"])), format!("{}: {}", s(&c["label"]), s(&c["word"]))])
                    }).collect::<Vec<_>>());
                    r["right"] = json!(format!("{} · {} not normal", x["cells"].as_array().unwrap().len(), x["not_normal"]));
                }
                "levels" => {
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let unit = if s(&row["unit"]).is_empty() { String::new() } else { format!(" {}", s(&row["unit"])) };
                        let v = if quiet(&row["state"]) { format!("{}{unit}", row["text"].as_str().unwrap_or("\u{2014}")) } else { s(&row["word"]) };
                        let ticks: Vec<String> = row["ticks"].as_array().unwrap().iter().map(|t| format!("left:{:.1}%", t.as_f64().unwrap() * 100.0)).collect();
                        json!([row["label"], format!("width:{:.1}%", row["fraction"].as_f64().unwrap_or(0.0) * 100.0), format!("lv-fill st-fill-{}", s(&row["state"])), ticks, v, format!("lv-v mono st-text-{}", s(&row["state"]))])
                    }).collect::<Vec<_>>());
                }
                "table" => {
                    r["head"] = json!(x["columns"].as_array().unwrap().iter().map(|c| c["label"].clone()).collect::<Vec<_>>());
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(|c| {
                        if let Some(bar) = c.get("bar").and_then(Value::as_f64) {
                            // A bar with rules keeps its state: the class, and the mark and word
                            // after the value when one holds.
                            let (t, cls) = match c.get("state").filter(|v| !v.is_null()) {
                                Some(st) => (if s(&c["word"]).is_empty() { s(&c["text"]) } else { format!("{} {} {}", s(&c["text"]), s(&c["mark"]), s(&c["word"])) }, format!("mono st-text-{}", st.as_str().unwrap())),
                                None => (s(&c["text"]), "mono".to_string()),
                            };
                            json!([t, cls, format!("width:{:.1}%", bar * 100.0)])
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
                    // The page's own words typed in capitals, σ small (Q28).
                    r["head"] = json!(["KEY", "MEDIAN", "MEDIAN, WINDOW", "DISTRIBUTION", "σ", "N"]);
                    // A distribution 160 by 26: bin i at its middle across, its count up from y 25
                    // to 2 against the largest (one at least).
                    r["rows"] = json!(x["rows"].as_array().unwrap().iter().map(|row| {
                        let bins: Vec<f64> = row["bins"].as_array().unwrap().iter().map(|b| b.as_f64().unwrap()).collect();
                        let bw = 160.0 / bins.len() as f64;
                        let py = scale(0.0, bins.iter().cloned().fold(1.0, f64::max), 25.0, 2.0);
                        let dist: Vec<Value> = bins.iter().enumerate().map(|(i, c)| json!([i as f64 * bw + bw / 2.0, py(*c)])).collect();
                        let dot = if pairs_of(&row["spark"]).len() == 1 { json!([60.0, 12.0]) } else { Value::Null };
                        json!([row["key"], num(&row["median"], 1), spark_at(&pairs_of(&row["spark"]), 120.0, 24.0), dist, num(&row["sigma"], 1), row["n"].to_string(), dot])
                    }).collect::<Vec<_>>());
                    let minutes = ((x["to"].as_f64().unwrap() - x["from"].as_f64().unwrap()) / 60_000.0).round();
                    let cap = if x["capped"] == json!(true) { format!(" · newest {} of {} rows", x["read"], x["total"]) } else { String::new() };
                    // The spread as the host wrote it (in full), and the rows left out, counted.
                    let left = match x["left_out"].as_u64() {
                        Some(n) if n > 0 => format!(" · {n} with no key or no value"),
                        _ => String::new(),
                    };
                    r["right"] = json!(format!("last {minutes} min · distribution ±{} around each median, same scale on every row{cap}{left}", x["spread_text"].as_str().unwrap()));
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

/// A dashboard of what a panel meets: rules written over the object's own series, a band of two
/// rules of one class, a rule over another series, a tile normal with no target and one with no
/// value, an object bound to one key on every chart, a table's columns in every pairing of `bar`
/// and `states` over values of every sign, a view's time of every kind portal's store writes as
/// milliseconds, a view fanning one stored row into several, float keys apart past three places,
/// cells a view cannot give (no order, no such column), a tile in every state a rule names and
/// every comparison (one over another series' key), a quarter hour's timeline of one key opening
/// on that key's state, a view of every date and time type the engine lists, one of a type its
/// driver cannot read, and integers past 2^53 as keys and cells.
const RULES: &str = r#"format = "veredas-binding"
version = 1
title = "Rules"

[[object]]
id = "own-source"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 15, source = "sorter", state = "warn", word = "slow" } ]

[[object]]
id = "own-written"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 15, source = "sorter", series = "travel", state = "warn", word = "slow" } ]

[[object]]
id = "band"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { above = 100, state = "alarm" }, { below = 99.5, state = "alarm" } ]

[[object]]
id = "other"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { above = 15, source = "sorter", series = "travel", state = "warn", word = "long" } ]

[[object]]
id = "plain"
value = { source = "sorter", series = "rate" }
unit = "%"

[[object]]
id = "empty"
value = { source = "sorter", series = "never" }

[[object]]
id = "keyed"
value = { source = "sorter", series = "at", key = "A1" }
states = [ { above = 30, state = "warn", word = "far" } ]

[[object]]
id = "two-rules"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 15, state = "warn", word = "slow" }, { above = 22, state = "alarm", word = "late" } ]

[[object]]
id = "t-near"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { at_least = 25, state = "near", word = "close" } ]

[[object]]
id = "t-off"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { at_most = 25, state = "disabled", word = "off" } ]

[[object]]
id = "t-equal"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { equals = 99, state = "alarm", word = "stuck" } ]

[[object]]
id = "t-word"
value = { source = "sorter", series = "mode" }
states = [ { equals = "hold", state = "warn", word = "held" } ]

[[object]]
id = "t-keyed"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { above = 15, source = "sorter", series = "at", key = "A1", state = "warn", word = "far" } ]

[[object]]
id = "t-tiny"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 0.0001, state = "warn", word = "moving" } ]

[[object]]
id = "t-half"
value = { source = "sorter", series = "rate" }
unit = "%"
states = [ { below = 99.0005, state = "near", word = "under" } ]

[[object]]
id = "k-source"
value = { source = "sorter", series = "at", key = "A1" }
states = [ { above = 30, source = "sorter", state = "warn", word = "far" } ]

[[object]]
id = "k-series"
value = { source = "sorter", series = "at", key = "A1" }
states = [ { above = 30, source = "sorter", series = "at", state = "warn", word = "far" } ]

[[object]]
id = "k-key"
value = { source = "sorter", series = "at", key = "A1" }
states = [ { above = 30, source = "sorter", series = "at", key = "A1", state = "warn", word = "far" } ]

[[object]]
id = "word-only"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 22, state = "warn", word = "late" }, { above = 15, state = "warn", word = "slow" } ]

[[object]]
id = "class-only"
value = { source = "sorter", series = "travel" }
unit = "s"
states = [ { above = 22, state = "alarm", word = "slow" }, { above = 15, state = "warn", word = "slow" } ]

[[object]]
id = "odd"
value = { source = "sorter", series = "ov" }

[[dashboard]]
name = "Rules"

[[dashboard.panel]]
kind = "timeline"
objects = ["own-source", "own-written", "keyed", "two-rules", "word-only", "class-only"]

[[dashboard.panel]]
kind = "value"
object = "band"

[[dashboard.panel]]
kind = "value"
object = "other"

[[dashboard.panel]]
kind = "value"
object = "plain"

[[dashboard.panel]]
kind = "value"
object = "empty"

[[dashboard.panel]]
kind = "value"
object = "keyed"

[[dashboard.panel]]
kind = "trend"
object = "keyed"

[[dashboard.panel]]
kind = "bars"
object = "keyed"
aggregate = "max"

[[dashboard.panel]]
kind = "table"
table = "signed"
columns = [ { name = "plain" }, { name = "bar", bar = true }, { name = "ruled", states = [ { below = 0, state = "alarm", word = "under" } ] }, { name = "both", bar = true, states = [ { below = 0, state = "alarm", word = "under" } ] }, { name = "twice", states = [ { below = 0, state = "alarm", word = "under" }, { below = 8, state = "warn", word = "low" } ] } ]

[[dashboard.panel]]
kind = "table"
table = "days"
columns = [ { name = "ts" }, { name = "day" }, { name = "at" } ]

[[dashboard.panel]]
kind = "cells"
table = "fanned"
key = "zone"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "keyed"
key = "k"
value = "n"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "unordered"
key = "outlet"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "fanned"
key = "nokey"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "doubled"
key = "outlet"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "value"
object = "t-near"

[[dashboard.panel]]
kind = "value"
object = "t-off"

[[dashboard.panel]]
kind = "value"
object = "t-equal"

[[dashboard.panel]]
kind = "value"
object = "t-word"

[[dashboard.panel]]
kind = "value"
object = "t-keyed"

[[dashboard.panel]]
kind = "timeline"
objects = ["keyed"]
minutes = 15

[[dashboard.panel]]
kind = "table"
table = "clocks"

[[dashboard.panel]]
kind = "table"
table = "nanos"

[[dashboard.panel]]
kind = "cells"
table = "big"
key = "k"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "big"
columns = [ { name = "k" }, { name = "u", decimals = 2 } ]

[[dashboard.panel]]
kind = "table"
table = "close"
columns = [ { name = "k" } ]
sort = { column = "k" }

[[dashboard.panel]]
kind = "cells"
table = "near"
key = "k"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "wide"
columns = [ { name = "h" }, { name = "d" } ]

[[dashboard.panel]]
kind = "cells"
table = "wide"
key = "hk"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "timed"
key = "ts"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "timed"
key = "day"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "ends"
sort = { column = "n" }

[[dashboard.panel]]
kind = "table"
table = "codes"
sort = { column = "code" }

[[dashboard.panel]]
kind = "table"
table = "huge"
columns = [ { name = "h", bar = true, decimals = 2, states = [ { above = 10, state = "alarm", word = "big" } ] } ]
sort = { column = "h" }

[[dashboard.panel]]
kind = "cells"
table = "huge"
key = "outlet"
value = "h"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "worded"
key = "outlet"
value = "word"
spread = 5

[[dashboard.panel]]
kind = "value"
object = "t-tiny"

[[dashboard.panel]]
kind = "value"
object = "t-half"

[[dashboard.panel]]
kind = "value"
object = "plain"
target = 0.0005

[[dashboard.panel]]
kind = "trend"
object = "keyed"
limit = { above = 0.004 }

[[dashboard.panel]]
kind = "bars"
object = "keyed"
aggregate = "max"
target = 0.0005

[[dashboard.panel]]
kind = "cells"
table = "digits"
key = "code"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "digits"
key = "n5"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "cells"
table = "nulls"
key = "k"
value = "v"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "flags"
columns = [ { name = "hot", bar = true, states = [ { equals = 1, state = "warn", word = "hot" } ] } ]
sort = { column = "hot" }

[[dashboard.panel]]
kind = "table"
table = "edges"
columns = [ { name = "h" } ]

[[dashboard.panel]]
kind = "cells"
table = "edges"
key = "h"
value = "travel"
spread = 5

[[dashboard.panel]]
kind = "table"
table = "far"

[[dashboard.panel]]
kind = "cells"
table = "sorts"
key = "outlet"
value = "travel"
spread = 0.0005

[[dashboard.panel]]
kind = "timeline"
objects = ["keyed", "k-source", "k-series", "k-key"]

[[dashboard.panel]]
kind = "value"
object = "k-source"

[[dashboard.panel]]
kind = "value"
object = "k-series"

[[dashboard.panel]]
kind = "value"
object = "k-key"

[[dashboard.panel]]
kind = "table"
table = "places"
columns = [ { name = "small", decimals = 0 }, { name = "k", decimals = 0 }, { name = "u", decimals = 0 }, { name = "h", decimals = 0 }, { name = "small", decimals = 6 }, { name = "k", decimals = 6 }, { name = "u", decimals = 6 }, { name = "h", decimals = 6 } ]

[[dashboard.panel]]
kind = "trend"
object = "odd"

[[dashboard.panel]]
kind = "value"
object = "odd"

[[dashboard.panel]]
kind = "bars"
object = "odd"
aggregate = "mean"

[[dashboard.panel]]
kind = "table"
table = "nonfinite"
columns = [ { name = "p", bar = true, states = [ { above = 100, state = "alarm", word = "over" } ] }, { name = "q" } ]
"#;

/// The project's rules and views for `RULES`: a keyed numeric series, float keys, a series never
/// written, a text series, a view fanning sorts out by zone, one of every time portal writes as
/// milliseconds, one of values of every sign, one with no order; the engine's own list of its date
/// and time types, a view of each, one of the type its driver cannot read, one of integers past 2^53.
const RULES_PROJECT: &str = r#"
[[rule]]
name = "positions"
pattern = 'POS item=(?P<item>\w+) at=(?P<at>[0-9.]+)'
scope = "sorter"
types = { at = "float" }
key = "item"

[[rule]]
name = "keyed"
pattern = 'KEYED k=(?P<k>[0-9.]+) n=(?P<n>[0-9.]+)'
scope = "sorter"
types = { k = "float", n = "float" }

[[rule]]
name = "nevers"
pattern = 'NEVER v=(?P<never>[0-9.]+)'
scope = "sorter"
types = { never = "float" }

[[rule]]
name = "modes"
pattern = 'MODE m=(?P<mode>\w+)'
scope = "sorter"
types = { mode = "text" }

[[rule]]
name = "odds"
pattern = 'ODD v=(?P<ov>\S+)'
scope = "sorter"
types = { ov = "float" }

[[table]]
name = "datetimes"
sql = "SELECT DISTINCT logical_type AS t FROM duckdb_types() WHERE type_category = 'DATETIME'"

[[table]]
name = "clocks"
sql = "SELECT index_ms, CAST(ts AS TIMESTAMP_S) AS s, CAST(ts AS TIMESTAMP_MS) AS ms, ts AS us, CAST(ts AS TIMESTAMP_NS) AS ns, CAST(ts AS TIMESTAMPTZ) AS tz, CAST(ts AS DATE) AS day, CAST(ts AS TIME) AS hour, CAST(CAST(ts AS TIME) AS TIMETZ) AS hourtz FROM sorts"

[[table]]
name = "nanos"
sql = "SELECT index_ms, CAST(CAST(ts AS VARCHAR)[12:] AS TIME_NS) AS at FROM sorts"

[[table]]
name = "close"
sql = "SELECT index_ms, CASE outlet WHEN 1 THEN 9007199254740993 ELSE 9007199254740992 END AS k FROM sorts"

[[table]]
name = "near"
sql = "SELECT index_ms, source, seq, CASE outlet WHEN 1 THEN 9999999999999999 ELSE 10000000000000000 END AS k, travel FROM sorts"

[[table]]
name = "wide"
sql = "SELECT index_ms, source, seq, CAST('18446744073709551616' AS HUGEINT) + outlet AS h, CAST(CAST('18446744073709551616' AS HUGEINT) + outlet AS DECIMAL(38,0)) AS d, CASE outlet WHEN 1 THEN CAST(1 AS HUGEINT) ELSE CAST('18446744073709551617' AS HUGEINT) END AS hk, travel FROM sorts"

[[table]]
name = "timed"
sql = "SELECT index_ms, source, seq, ts, CAST(ts AS DATE) AS day, travel FROM sorts"

[[table]]
name = "ends"
sql = "SELECT 0 AS n, CAST(-128 AS TINYINT) AS ti, CAST(-32768 AS SMALLINT) AS si, CAST(-2147483648 AS INTEGER) AS i, CAST(-9223372036854775808 AS BIGINT) AS bi, CAST('-170141183460469231731687303715884105728' AS HUGEINT) AS hi, CAST(0 AS UTINYINT) AS uti, CAST(0 AS USMALLINT) AS usi, CAST(0 AS UINTEGER) AS ui, CAST(0 AS UBIGINT) AS ubi, CAST('-99999999999999999999999999999999999999' AS DECIMAL(38,0)) AS d38 UNION ALL SELECT 1, CAST(127 AS TINYINT), CAST(32767 AS SMALLINT), CAST(2147483647 AS INTEGER), CAST(9223372036854775807 AS BIGINT), CAST('170141183460469231731687303715884105727' AS HUGEINT), CAST(255 AS UTINYINT), CAST(65535 AS USMALLINT), CAST(4294967295 AS UINTEGER), CAST(18446744073709551615 AS UBIGINT), CAST('99999999999999999999999999999999999999' AS DECIMAL(38,0))"

[[table]]
name = "ends_text"
sql = "SELECT n, CAST(ti AS VARCHAR) AS ti, CAST(si AS VARCHAR) AS si, CAST(i AS VARCHAR) AS i, CAST(bi AS VARCHAR) AS bi, CAST(hi AS VARCHAR) AS hi, CAST(uti AS VARCHAR) AS uti, CAST(usi AS VARCHAR) AS usi, CAST(ui AS VARCHAR) AS ui, CAST(ubi AS VARCHAR) AS ubi, CAST(d38 AS VARCHAR) AS d38 FROM ends"

[[table]]
name = "numerics"
sql = "SELECT DISTINCT logical_type AS t FROM duckdb_types() WHERE type_category = 'NUMERIC'"

[[table]]
name = "big"
sql = "SELECT index_ms, source, seq, CASE WHEN outlet = 1 THEN 9007199254740993 ELSE -9007199254740995 END AS k, CAST(18446744073709551615 AS UBIGINT) - CAST(outlet AS UBIGINT) AS u, travel FROM sorts"

[[table]]
name = "zones"
sql = "SELECT * FROM (VALUES (1, 'a'), (1, 'b'), (2, 'c')) AS z(outlet, zone)"

[[table]]
name = "fanned"
sql = "SELECT s.index_ms, s.source, s.seq, s.outlet, s.travel, z.zone FROM sorts s JOIN zones z ON z.outlet = s.outlet"

[[table]]
name = "days"
sql = "SELECT index_ms, ts, CAST(ts AS DATE) AS day, CAST(ts AS TIME) AS at FROM sorts"

[[table]]
name = "signed"
sql = "SELECT index_ms, travel - 15 AS plain, travel - 15 AS bar, travel - 15 AS ruled, travel - 15 AS both, travel - 15 AS twice FROM sorts"

[[table]]
name = "unordered"
sql = "SELECT outlet, travel FROM sorts"

[[table]]
name = "doubled"
sql = "SELECT index_ms, source, seq, outlet, travel FROM sorts UNION ALL SELECT index_ms, source, seq, outlet, travel FROM sorts"

[[table]]
name = "codes"
sql = "SELECT * FROM (VALUES ('100000000000000000000'), ('99999999999999999999'), ('5'), ('9000000000000000000')) AS c(code)"

[[table]]
name = "huge"
sql = "SELECT index_ms, source, seq, outlet, CASE outlet WHEN 1 THEN CAST('18446744073709551617' AS HUGEINT) ELSE CAST(5 AS HUGEINT) END AS h, travel FROM sorts"

[[table]]
name = "worded"
sql = "SELECT index_ms, source, seq, outlet, CAST(travel AS VARCHAR) AS word FROM sorts"

[[table]]
name = "keyclocks"
sql = "SELECT index_ms, source, seq, travel, CAST(ts AS TIMESTAMP_S) AS s, CAST(ts AS TIMESTAMP_MS) AS ms, ts AS us, CAST(ts AS TIMESTAMP_NS) AS ns, CAST(ts AS TIMESTAMPTZ) AS tz, CAST(ts AS DATE) AS day, CAST(ts AS TIME) AS hour, CAST(CAST(ts AS TIME) AS TIMETZ) AS hourtz FROM sorts"

[[table]]
name = "others"
sql = "SELECT DISTINCT logical_type AS t FROM duckdb_types() WHERE type_category IS DISTINCT FROM 'NUMERIC'"

[[table]]
name = "digits"
sql = "SELECT index_ms, source, seq, CAST(outlet * 5 AS VARCHAR) AS code, outlet * 5 AS n5, travel FROM sorts"

[[table]]
name = "nulls"
sql = "SELECT index_ms, source, seq, CASE WHEN travel = 0 THEN NULL ELSE outlet END AS k, CASE WHEN travel = 20 THEN NULL ELSE travel END AS v FROM sorts"

[[table]]
name = "flags"
sql = "SELECT index_ms, outlet, travel > 15 AS hot FROM sorts"

[[table]]
name = "edges"
sql = "SELECT index_ms, source, seq, CASE outlet WHEN 2 THEN CAST('24:00:00' AS TIME) ELSE CAST('00:00:00' AS TIME) END AS h, travel FROM sorts"

[[table]]
name = "far"
sql = "SELECT CAST('infinity' AS TIMESTAMP) AS a, CAST('-infinity' AS TIMESTAMP) AS b, CAST('infinity' AS DATE) AS c, CAST('-infinity' AS DATE) AS d, CAST('294000-01-01 00:00:00' AS TIMESTAMP) AS e"

[[table]]
name = "keynums"
sql = "SELECT index_ms, source, seq, travel, CAST(CASE travel WHEN 0 THEN -1 WHEN 20 THEN -2 ELSE 10 END AS TINYINT) AS ti, CAST(CASE travel WHEN 0 THEN -1 WHEN 20 THEN -2 ELSE 10 END AS SMALLINT) AS si, CAST(CASE travel WHEN 0 THEN -1 WHEN 20 THEN -2 ELSE 10 END AS INTEGER) AS i, CAST(CASE travel WHEN 0 THEN -1 WHEN 20 THEN -2 ELSE 10 END AS BIGINT) AS bi, CASE travel WHEN 0 THEN CAST('-18446744073709551617' AS HUGEINT) WHEN 20 THEN CAST('-18446744073709551616' AS HUGEINT) ELSE CAST('18446744073709551617' AS HUGEINT) END AS hi, CASE travel WHEN 0 THEN CAST(5 AS HUGEINT) WHEN 20 THEN CAST('18446744073709551617' AS HUGEINT) ELSE CAST(10 AS HUGEINT) END AS hi5, CAST(CASE travel WHEN 0 THEN 5 WHEN 20 THEN 10 ELSE 100 END AS UTINYINT) AS uti, CAST(CASE travel WHEN 0 THEN 5 WHEN 20 THEN 10 ELSE 100 END AS USMALLINT) AS usi, CAST(CASE travel WHEN 0 THEN 5 WHEN 20 THEN 10 ELSE 100 END AS UINTEGER) AS ui, CASE travel WHEN 0 THEN CAST(5 AS UBIGINT) WHEN 20 THEN CAST(10 AS UBIGINT) ELSE CAST('18446744073709551615' AS UBIGINT) END AS ubi, CAST(CASE travel WHEN 0 THEN 5 WHEN 20 THEN 10 ELSE 100 END AS UHUGEINT) AS uhi, CAST(CASE travel WHEN 0 THEN -0.5 WHEN 20 THEN -1.5 ELSE 10.5 END AS FLOAT) AS f, CAST(CASE travel WHEN 0 THEN -0.5 WHEN 20 THEN -1.5 ELSE 10.5 END AS DOUBLE) AS d, CAST(CASE travel WHEN 0 THEN -0.5 WHEN 20 THEN -1.5 ELSE 10.5 END AS DECIMAL(4,1)) AS de, CASE travel WHEN 0 THEN CAST(5 AS DECIMAL(38,0)) WHEN 20 THEN CAST('18446744073709551617' AS DECIMAL(38,0)) ELSE CAST(10 AS DECIMAL(38,0)) END AS de38, travel > 15 AS b FROM sorts"

[[table]]
name = "places"
sql = "SELECT index_ms, CASE travel WHEN 20 THEN -5 ELSE 5 END AS small, CASE travel WHEN 20 THEN -9007199254740995 ELSE 9007199254740993 END AS k, CAST('18446744073709551615' AS UBIGINT) - CAST(travel AS UBIGINT) AS u, CASE travel WHEN 20 THEN CAST('-18446744073709551617' AS HUGEINT) ELSE CAST('18446744073709551617' AS HUGEINT) END AS h FROM sorts"

[[table]]
name = "keytimes"
sql = "SELECT index_ms, source, seq, travel, CASE travel WHEN 0 THEN DATE '2026-01-01' WHEN 20 THEN DATE '10000-01-01' ELSE DATE 'infinity' END AS day, CASE travel WHEN 0 THEN TIMESTAMP '2026-01-01 00:00:00' WHEN 20 THEN TIMESTAMP '10000-01-01 00:00:00' ELSE TIMESTAMP '-infinity' END AS ts, CAST(CASE travel WHEN 0 THEN '2026-01-01' WHEN 20 THEN '10000-01-01' ELSE '2027-01-01' END AS TIMESTAMP_S) AS s, CAST(CASE travel WHEN 0 THEN '2026-01-01' WHEN 20 THEN '10000-01-01' ELSE '2027-01-01' END AS TIMESTAMP_MS) AS ms, CAST(CASE travel WHEN 0 THEN '2026-01-01' WHEN 20 THEN '10000-01-01' ELSE '2027-01-01' END AS TIMESTAMPTZ) AS tz FROM sorts"

[[table]]
name = "enums"
sql = "SELECT index_ms, source, seq, travel, CAST(CASE travel WHEN 0 THEN 'low' WHEN 20 THEN 'mid' ELSE 'high' END AS ENUM('low', 'mid', 'high')) AS lv FROM sorts"

[[table]]
name = "nonfinite"
sql = "SELECT CAST('inf' AS DOUBLE) AS p, CAST('nan' AS DOUBLE) AS q UNION ALL SELECT 5.0, 1.0"

[[table]]
name = "zeros"
sql = "SELECT index_ms, source, seq, travel, CASE travel WHEN 0 THEN CAST('-0.0' AS DOUBLE) ELSE CAST('0.0' AS DOUBLE) END AS k, CASE travel WHEN 0 THEN CAST('-0.0' AS FLOAT) ELSE CAST('0.0' AS FLOAT) END AS kf FROM sorts"

[[table]]
name = "extremes"
sql = "SELECT index_ms, source, seq, 'k' AS key, CAST('1.7976931348623157e308' AS DOUBLE) AS top, CASE travel WHEN 0 THEN CAST('1e155' AS DOUBLE) ELSE CAST('-1e155' AS DOUBLE) END AS wide FROM sorts"

[[table]]
name = "singles"
sql = "SELECT index_ms, source, seq, travel, CAST(CASE travel WHEN 0 THEN 0.1 WHEN 20 THEN 0.2 ELSE 0.3 END AS FLOAT) AS f FROM sorts"
"#;

#[test]
fn every_rule_type_row_and_key_a_panel_meets_reads_as_the_reference_says() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    ex.project.push_str(RULES_PROJECT);
    ex.binding = RULES.to_string();
    let path = write_example(dir.path(), &ex);
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let cfg = config::load_file(&path).unwrap();
    let binding = cfg.sessions[0].binding.clone();
    let host = Host::start(cfg).unwrap();
    let addr = host.addr();
    let live = host.session("loop").unwrap().live();
    assert!(wait(Duration::from_secs(20), || live.health.read().unwrap().get("sorter").is_some_and(|h| h.count == 0)), "the log is attached");
    // Ten minutes apart, within the tiles' half hour: travel 0, 20, 25 (a run normal, then slow);
    // A1 at 10 and 20 with B2 at 50 between; two float keys apart in their fourth place.
    let base = (store_now() / MIN) * MIN - 25 * MIN;
    let at = |m: f64| base + (m * MIN as f64) as i64;
    append(&log, &[
        format!("{} SORT outlet=1 travel=0.0 rate=99.0", stamp(at(0.0))),
        format!("{} POS item=A1 at=10", stamp(at(1.0))),
        format!("{} POS item=B2 at=50", stamp(at(2.0))),
        format!("{} POS item=A1 at=20", stamp(at(3.0))),
        format!("{} KEYED k=1.0001 n=1", stamp(at(4.0))),
        format!("{} KEYED k=1.0004 n=2", stamp(at(5.0))),
        format!("{} POS item=B2 at=60", stamp(at(6.0))),
        format!("{} MODE m=hold", stamp(at(7.0))),
        format!("{} SORT outlet=2 travel=20.0 rate=99.0", stamp(at(10.0))),
        format!("{} SORT outlet=1 travel=25.0 rate=99.0", stamp(at(20.0))),
        format!("{} ODD v=97", stamp(at(21.0))),
        format!("{} ODD v=NaN", stamp(at(22.0))),
        format!("{} ODD v=99", stamp(at(23.0))),
        format!("{} ODD v=inf", stamp(at(24.0))),
        format!("{} ODD v=98", stamp(at(25.0))),
    ]);
    let rows = |t: &str| live.store.query_rows(t, &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || rows("sorts") == 3 && rows("positions") == 4 && rows("keyed") == 2 && rows("modes") == 1 && rows("odds") == 5), "the lines are stored");
    let to = at(26.0);
    let get_d = || get(addr, &format!("/s/loop/dashboard/0?to={to}")).1;
    assert!(wait(Duration::from_secs(20), || { let d = get_d(); panel(&d, 2)["state"] == json!("warn") && panel(&d, 5)["text"] == json!("20") && panel(&d, 18)["state"] == json!("warn") && panel(&d, 19)["state"] == json!("warn") }), "{}", get_d());
    let d = get_d();

    // Every panel computed but the four said in their places (three refusals, one panic).
    for (i, p) in d["panels"].as_array().unwrap().iter().enumerate() {
        assert_eq!(p["error"].is_null(), ![12, 13, 22, 35].contains(&i), "panel {i}: {p}");
    }
    // A rule written over the object's own series (its source alone, or its source and series) is
    // the object's own on a timeline: normal to the slow sort, slow from it to the window's end.
    let tl = panel(&d, 0);
    for (i, id) in ["own-source", "own-written"].into_iter().enumerate() {
        let runs: Vec<(Value, Value, Value)> = tl["rows"][i]["segments"].as_array().unwrap().iter().map(|s| (s["from"].clone(), s["state"].clone(), s["word"].clone())).collect();
        assert_eq!(runs, [(json!(at(0.0)), json!("normal"), json!("ok")), (json!(at(10.0)), json!("warn"), json!("slow"))], "{id}");
        assert_eq!(tl["rows"][i]["now"]["state"], json!("warn"), "{id}");
    }
    // The tiles' sub-lines, by every branch: the rule that holds, of two of one class and word (the
    // feed's own pick); a rule over another series, that series named, its comparison bare; a
    // normal tile with no target, nothing; a tile with no value.
    let sub = |i: usize| (panel(&d, i)["state"].clone(), panel(&d, i)["sub"].clone());
    assert_eq!(sub(1), (json!("alarm"), json!("below 99.5 %")));
    assert_eq!(sub(2), (json!("warn"), json!("sorter · travel above 15")));
    assert_eq!(sub(3), (json!("normal"), json!("")));
    assert_eq!((panel(&d, 4)["text"].clone(), panel(&d, 4)["sub"].clone()), (Value::Null, json!("no value")));
    // An object bound to one key: its tile, its trend, its bars and its timeline read that key's
    // samples alone (B2's 50 in none of them).
    let a1 = json!([[at(1.0), 10.0], [at(3.0), 20.0]]);
    assert_eq!((panel(&d, 5)["spark"].clone(), panel(&d, 6)["points"].clone()), (a1.clone(), a1));
    // Each bucket's max over A1's samples alone, wherever the clock puts their buckets.
    let bucket = 5 * MIN;
    let mut want: BTreeMap<i64, f64> = BTreeMap::new();
    for (t, v) in [(at(1.0), 10.0), (at(3.0), 20.0)] {
        let e = want.entry(t.div_euclid(bucket) * bucket).or_insert(v);
        *e = e.max(v);
    }
    let maxes: Vec<(i64, f64)> = panel(&d, 7)["buckets"].as_array().unwrap().iter().filter_map(|b| Some((b["start"].as_i64()?, b["value"].as_f64()?))).collect();
    assert_eq!(maxes, want.into_iter().collect::<Vec<_>>(), "the bars' max over A1 alone");
    let keyed_runs: Vec<Value> = tl["rows"][2]["segments"].as_array().unwrap().iter().map(|s| s["state"].clone()).collect();
    assert_eq!(keyed_runs, [json!("normal")], "the timeline over A1 alone, never far");
    // A table's columns in every pairing of a bar and rules, over values of every sign (-15, 5, 10,
    // newest first): a bar is the value's magnitude against the column's largest; a state on a
    // column with rules alone, with or without its bar.
    let tb = panel(&d, 8);
    let cell = |r: usize, c: usize| tb["rows"][r][c].clone();
    let fractions: Vec<Value> = (0..3).map(|r| cell(r, 1)["bar"].clone()).collect();
    assert_eq!(json!(fractions), as_served(&json!([10.0 / 15.0, 5.0 / 15.0, 1.0])));
    assert_eq!((0..3).map(|r| cell(r, 3)["bar"].clone()).collect::<Vec<_>>(), fractions);
    for r in 0..3 {
        assert!(cell(r, 0).get("bar").is_none() && cell(r, 0).get("state").is_none() && cell(r, 1).get("state").is_none() && cell(r, 2).get("bar").is_none(), "row {r}: {}", tb["rows"][r]);
    }
    assert_eq!((0..3).map(|r| (cell(r, 2)["state"].clone(), cell(r, 3)["state"].clone())).collect::<Vec<_>>(), [(json!("normal"), json!("normal")), (json!("normal"), json!("normal")), (json!("alarm"), json!("alarm"))]);
    // Every kind of time portal's store writes as milliseconds (a timestamp, a date, a time of day),
    // each written as the store's clock reads it.
    let days = panel(&d, 9);
    let clock = |ms: i64, f: &str| chrono::DateTime::from_timestamp_millis(ms).unwrap().format(f).to_string();
    let texts: Vec<Vec<Value>> = days["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|c| c["text"].clone()).collect()).collect();
    let want: Vec<Vec<Value>> = [20.0, 10.0, 0.0].iter().map(|m| vec![json!(clock(at(*m), "%Y-%m-%d %H:%M:%S")), json!(clock(at(*m), "%Y-%m-%d")), json!(clock(at(*m), "%H:%M:%S"))]).collect();
    assert_eq!(texts, want);
    // A view fanning a stored row into several: every row read (outlet 1's two zones each), nothing
    // capped.
    let fan = panel(&d, 10);
    let per: Vec<(Value, Value)> = fan["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!((per, fan["read"].clone(), fan["total"].clone(), fan["capped"].clone()), (vec![(json!("a"), json!(2)), (json!("b"), json!(2)), (json!("c"), json!(1))], json!(5), json!(5), json!(false)));
    // Float keys apart past three places, each its own row.
    let keys: Vec<(Value, Value)> = panel(&d, 11)["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!(keys, [(json!("1.0001"), json!(1)), (json!("1.0004"), json!(1))]);
    // What cells cannot read, said in the panel's place through the route: a view with no order,
    // a key the view lacks.
    assert_eq!(d["panels"][12]["error"], json!("`unordered` has no `index_ms`, `source`, `seq` (cells read each row once by `index_ms`, `source` and `seq`; its columns: outlet, travel)"));
    // (The limit stated: a view's rows alike in every column are one row, and the window not
    // capped for it.)
    let doubled = panel(&d, 14);
    let per: Vec<(Value, Value)> = doubled["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    assert_eq!((per, doubled["read"].clone(), doubled["total"].clone(), doubled["capped"].clone()), (vec![(json!("1"), json!(2)), (json!("2"), json!(1))], json!(3), json!(6), json!(false)));
    assert_eq!(d["panels"][13]["error"], json!("`fanned` has no column `nokey` (its columns: index_ms, source, seq, outlet, travel, zone)"));

    // Every state a rule names and every comparison a rule writes, each said on a tile: its
    // sub-line the rule that named the state (the feed's pick: the band's second), written here
    // from the binding's own rule, in the object's unit, or over another series (keyed or not)
    // that series named and its comparison bare. The crate's enums are matched in full, so a
    // state or a comparison added there fails here until a tile says it.
    use veredas_supervisao::binding::{PanelKind, Rule};
    use veredas_supervisao::state::{Comparand, Compare, StateClass};
    let picked: BTreeMap<usize, usize> = [(1, 1), (2, 0), (15, 0), (16, 0), (17, 0), (18, 0), (19, 0), (36, 0), (37, 0)].into_iter().collect();
    let (mut states, mut comparisons) = (BTreeSet::new(), BTreeSet::new());
    for (i, p) in binding.dashboards[0].panels.iter().enumerate() {
        let PanelKind::Value { object, .. } = &p.kind else { continue };
        let o = &binding.objects[*object];
        let tile = panel(&d, i);
        if tile["state"] == json!("normal") || tile["state"] == json!("stale") {
            assert!(!picked.contains_key(&i), "panel {i}: {tile}");
            continue;
        }
        let r: &Rule = &o.rules[picked[&i]];
        let other = r.series.as_ref().filter(|s| **s != o.value);
        let n = |x: f64| if other.is_none() && !o.unit.is_empty() { format!("{x} {}", o.unit) } else { format!("{x}") };
        let (kind, cmp) = match &r.compare {
            Compare::Equals(Comparand::Num(x)) => ("equals a number", format!("equals {}", n(*x))),
            Compare::Equals(Comparand::Text(t)) => ("equals a word", format!("is {t}")),
            Compare::Above(x) => ("above", format!("above {}", n(*x))),
            Compare::Below(x) => ("below", format!("below {}", n(*x))),
            Compare::AtLeast(x) => ("at least", format!("at least {}", n(*x))),
            Compare::AtMost(x) => ("at most", format!("at most {}", n(*x))),
        };
        let said = match other {
            Some(s) => format!("{} · {}{} {cmp}", s.source, s.series, s.key.as_ref().map(|k| format!(" · {k}")).unwrap_or_default()),
            None => cmp,
        };
        assert_eq!((tile["state"].clone(), tile["sub"].clone()), (json!(r.class.name()), json!(said)), "panel {i}");
        states.insert(r.class.name());
        comparisons.insert(kind);
    }
    let ruled: BTreeSet<&str> = every_state().into_iter().filter(|s| StateClass::of_rule(s).is_some()).collect();
    assert_eq!(states, ruled, "every state a rule names, said on a tile");
    assert_eq!(comparisons, ["above", "at least", "at most", "below", "equals a number", "equals a word"].into_iter().collect(), "every comparison, said on a tile");
    // The state holding at a window's start read from the object's own key: A1's timeline over the
    // last quarter hour opens on A1's 20 (normal), not on B2's 60 stored after it (far).
    let runs: Vec<(Value, Value, Value)> = panel(&d, 20)["rows"][0]["segments"].as_array().unwrap().iter().map(|s| (s["from"].clone(), s["state"].clone(), s["word"].clone())).collect();
    assert_eq!(runs, [(json!(at(11.0)), json!("normal"), json!("ok"))], "the quarter hour of A1 alone");
    // Every date and time type the engine lists (its own catalog, less the interval: a span, which
    // portal writes as text), each written as the store's clock reads its value: a type the engine
    // adds fails here until the view holds it. The engine's time of day in nanoseconds is the
    // limit: its driver cannot read it, and its panel says so in its own place.
    let listed = live.store.query_rows("datetimes", &veredas_portal::store::RowsQuery { limit: 100, ..Default::default() }).unwrap();
    let engine: BTreeSet<String> = listed.rows.iter().map(|r| r[0].as_str().unwrap().to_string()).filter(|t| t != "INTERVAL").collect();
    let held = live.store.query_rows("clocks", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).unwrap();
    let types: Vec<String> = held.columns.iter().skip(1).map(|c| c.data_type.clone()).collect();
    assert!(engine.contains("TIME_NS"), "the limit is the engine's own type: {engine:?}");
    assert_eq!(types.iter().cloned().chain(["TIME_NS".to_string()]).collect::<BTreeSet<_>>(), engine, "the views hold every date and time type the engine lists");
    let read_as = |t: &str| match t {
        "TIMESTAMP_S" | "TIMESTAMP_MS" | "TIMESTAMP" | "TIMESTAMP_NS" | "TIMESTAMP WITH TIME ZONE" => "%Y-%m-%d %H:%M:%S",
        "DATE" => "%Y-%m-%d",
        "TIME" | "TIME WITH TIME ZONE" => "%H:%M:%S",
        other => panic!("{other}: a type this test has no clock for"),
    };
    let clocks = panel(&d, 21);
    assert_eq!(clocks["rows"].as_array().unwrap().len(), 3);
    for row in clocks["rows"].as_array().unwrap() {
        for (c, t) in row.as_array().unwrap().iter().skip(1).zip(&types) {
            assert_eq!(c["text"], json!(clock(c["value"].as_i64().unwrap(), read_as(t))), "{t}: {row}");
        }
    }
    // Q130 (ruled 2026-11-25, "Read it as microseconds"): portal is to read a time of day in
    // nanoseconds as a TIME, its last three digits dropped. Until its change lands the panel says it
    // could not be computed (its text held to that prefix alone); after it, the panel reads each
    // time of day as the store's clock reads a TIME. Either is taken for the push between.
    // DECISION PENDING: the error's branch goes once portal's cast is on main.
    match d["panels"][22]["error"].as_str() {
        Some(panicked) => assert!(panicked.starts_with("the panel could not be computed: "), "{panicked}"),
        None => {
            let nanos = panel(&d, 22);
            assert!(!nanos["rows"].as_array().unwrap().is_empty(), "{nanos}");
            for row in nanos["rows"].as_array().unwrap() {
                let at = &row.as_array().unwrap()[1];
                assert_eq!(at["text"], json!(clock(at["value"].as_i64().unwrap(), "%H:%M:%S")), "a time of day in nanoseconds read as a TIME: {row}");
            }
        }
    }
    // The store answers again after it: the dashboard read once more, the same.
    let again = get_d();
    assert_eq!((again["panels"][21].clone(), again["panels"][22]["error"].clone()), (d["panels"][21].clone(), d["panels"][22]["error"].clone()));
    // Integers a double cannot hold (past 2^53), as keys and as cells, in their own digits: 2^53 + 1
    // and -(2^53 + 3) as keys; the largest unsigned less the outlet, to two places.
    let keys: BTreeMap<String, Value> = panel(&d, 23)["rows"].as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap().to_string(), r["n"].clone())).collect();
    assert_eq!(keys, [("-9007199254740995".to_string(), json!(1)), ("9007199254740993".to_string(), json!(2))].into_iter().collect());
    let big: Vec<Vec<Value>> = panel(&d, 24)["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|c| c["text"].clone()).collect()).collect();
    let want: Vec<Vec<Value>> = [["9007199254740993", "18446744073709551614.00"], ["-9007199254740995", "18446744073709551613.00"], ["9007199254740993", "18446744073709551614.00"]].iter().map(|r| r.iter().map(|t| json!(t)).collect()).collect();
    assert_eq!(big, want);
    // Capped is said only when the cap stopped the read with a row of the window left: the doubled
    // view's six rows are three, read whole under a cap of three or four and not capped, capped
    // under two; the sorts' three alike.
    for (table, cap, read, capped) in [("doubled", 2, 2, true), ("doubled", 3, 3, false), ("doubled", 4, 3, false), ("sorts", 2, 2, true), ("sorts", 3, 3, false)] {
        let c = veredas_supervisao::panel::cells(&live, table, "outlet", "travel", 60, 5, 5.0, to, 2, cap).unwrap();
        assert_eq!((c["read"].clone(), c["capped"].clone()), (json!(read), json!(capped)), "{table} under {cap}");
    }
    // The rules' order where the panels apply rules themselves: the first that holds names the state
    // (a timeline's run and a table's cell), never a later one that holds too.
    let runs: Vec<(Value, Value, Value)> = tl["rows"][3]["segments"].as_array().unwrap().iter().map(|s| (s["from"].clone(), s["state"].clone(), s["word"].clone())).collect();
    assert_eq!(runs, [(json!(at(0.0)), json!("normal"), json!("ok")), (json!(at(10.0)), json!("warn"), json!("slow"))], "25 s holds both rules: the first names it");
    // A run is one state, its class and its word: two samples apart in the word alone are two runs,
    // and two apart in the class alone.
    for (row, id, last) in [(4, "word-only", ("warn", "late")), (5, "class-only", ("alarm", "slow"))] {
        let runs: Vec<(Value, Value, Value)> = tl["rows"][row]["segments"].as_array().unwrap().iter().map(|s| (s["from"].clone(), s["state"].clone(), s["word"].clone())).collect();
        assert_eq!(runs, [(json!(at(0.0)), json!("normal"), json!("ok")), (json!(at(10.0)), json!("warn"), json!("slow")), (json!(at(20.0)), json!(last.0), json!(last.1))], "{id}");
    }
    let twice: Vec<(Value, Value)> = (0..3).map(|r| (cell(r, 4)["state"].clone(), cell(r, 4)["word"].clone())).collect();
    assert_eq!(twice[1..], [(json!("warn"), json!("low")), (json!("alarm"), json!("under"))], "-15 holds both rules: the first names it");
    assert_eq!(twice[0].0, json!("normal"));
    // Whole numbers in their exact order: integers past 2^53 sorted as themselves, and as cells keys.
    let texts = |i: usize| -> Vec<Vec<Value>> { panel(&d, i)["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|c| c["text"].clone()).collect()).collect() };
    assert_eq!(texts(25), [[json!("9007199254740992")], [json!("9007199254740993")], [json!("9007199254740993")]], "sorted exactly");
    let keyed = |i: usize| -> Vec<(Value, Value)> { panel(&d, i)["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect() };
    assert_eq!(keyed(26), [(json!("9999999999999999"), json!(2)), (json!("10000000000000000"), json!(1))], "keys in their exact order");
    // Integers past 64 bits (a HUGEINT, a whole DECIMAL of 38 digits) in their own digits, never
    // wrapped, as cells and as keys.
    assert_eq!(texts(27), [[json!("18446744073709551617"), json!("18446744073709551617")], [json!("18446744073709551618"), json!("18446744073709551618")], [json!("18446744073709551617"), json!("18446744073709551617")]]);
    assert_eq!(keyed(28), [(json!("1"), json!(2)), (json!("18446744073709551617"), json!(1))]);
    // A key of a time kind written as the store's clock reads it (a timestamp, a date).
    let mut by_stamp: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_day: BTreeMap<String, u64> = BTreeMap::new();
    for m in [0.0, 10.0, 20.0] {
        *by_stamp.entry(clock(at(m), "%Y-%m-%d %H:%M:%S")).or_default() += 1;
        *by_day.entry(clock(at(m), "%Y-%m-%d")).or_default() += 1;
    }
    let as_rows = |m: BTreeMap<String, u64>| m.into_iter().map(|(k, n)| (json!(k), json!(n))).collect::<Vec<_>>();
    assert_eq!((keyed(29), keyed(30)), (as_rows(by_stamp), as_rows(by_day)), "time keys by the clock");
    // Every whole-number type the engine lists, at both ends of its range, written as the engine
    // itself writes it (its own cast to text); the rest of its NUMERIC types the stated limits (a
    // FLOAT and a DOUBLE to three places, a DECIMAL through the double nearest it, a UHUGEINT the
    // driver hands over as a double).
    let listed = live.store.query_rows("numerics", &veredas_portal::store::RowsQuery { limit: 100, ..Default::default() }).unwrap();
    let numeric: BTreeSet<String> = listed.rows.iter().map(|r| r[0].as_str().unwrap().to_string()).collect();
    let whole_types: BTreeSet<String> = ["TINYINT", "SMALLINT", "INTEGER", "BIGINT", "HUGEINT", "UTINYINT", "USMALLINT", "UINTEGER", "UBIGINT"].iter().map(|s| s.to_string()).collect();
    let limits: BTreeSet<String> = ["UHUGEINT", "FLOAT", "DOUBLE", "DECIMAL"].iter().map(|s| s.to_string()).collect();
    assert_eq!(numeric, whole_types.union(&limits).cloned().collect::<BTreeSet<_>>(), "the engine's NUMERIC types, each driven or a stated limit");
    let mut said = live.store.query_rows("ends_text", &veredas_portal::store::RowsQuery { limit: 10, ..Default::default() }).unwrap().rows;
    said.sort_by_key(|r| r[0].as_i64().unwrap());
    let ends_types: BTreeSet<String> = live.store.query_rows("ends", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).unwrap().columns.iter().skip(1).filter(|c| !c.data_type.starts_with("DECIMAL")).map(|c| c.data_type.clone()).collect();
    assert_eq!(ends_types, whole_types, "every whole-number type at its ends");
    let want: Vec<Vec<Value>> = said.iter().map(|r| r.iter().map(|c| json!(c.as_str().map(String::from).unwrap_or_else(|| c.to_string()))).collect::<Vec<Value>>()).collect();
    assert_eq!(texts(31), want, "each written as the engine writes it");
    // A column read by its engine type, every type the engine lists: each NUMERIC one holds numbers
    // (a decimal of any precision and scale among them), each of every other category does not.
    use veredas_supervisao::panel::numeric as holds_numbers;
    let others: BTreeSet<String> = live.store.query_rows("others", &veredas_portal::store::RowsQuery { limit: 1000, ..Default::default() }).unwrap().rows.iter().map(|r| r[0].as_str().unwrap().to_string()).collect();
    assert!(others.contains("VARCHAR") && others.contains("BOOLEAN") && others.contains("DATE"), "{others:?}");
    assert!(numeric.iter().all(|t| holds_numbers(t)) && holds_numbers("DECIMAL(38,0)") && holds_numbers("DECIMAL(4,1)"), "{numeric:?}");
    let not: Vec<&String> = others.iter().filter(|t| holds_numbers(t)).collect();
    assert_eq!(not, Vec::<&String>::new(), "a type of another category read as numbers");
    let as_texts = |rows: &[&[&str]]| rows.iter().map(|r| r.iter().map(|t| json!(t)).collect::<Vec<_>>()).collect::<Vec<_>>();
    // A text column's digits sorted as text (a code is no number), whatever their length.
    assert_eq!(texts(32), as_texts(&[&["100000000000000000000"], &["5"], &["9000000000000000000"], &["99999999999999999999"]]), "a text column in text's order");
    // A numeric column holding an integer past 64 bits (portal's digits) beside a small one: sorted
    // as the integers they are, its places written as zeros, its bar against it as the largest, its
    // rule holding over it (through the double nearest it).
    let hg: Vec<Vec<Value>> = panel(&d, 33)["rows"].as_array().unwrap().iter().map(|r| vec![r[0]["text"].clone(), r[0]["bar"].clone(), r[0]["state"].clone(), r[0]["word"].clone()]).collect();
    let small = as_served(&json!(5.0 / 18446744073709551617.0_f64));
    let big = vec![json!("18446744073709551617.00"), json!(1.0), json!("alarm"), json!("big")];
    assert_eq!(hg, vec![vec![json!("5.00"), small, json!("normal"), json!("")], big.clone(), big], "an integer past 64 bits in a numeric column");
    // Charted in cells, never dropped: outlet 1's two values past 64 bits and outlet 2's 5.
    let huge = panel(&d, 34);
    assert_eq!((keyed(34), huge["read"].clone(), huge["rows"][0]["median"].clone()), (vec![(json!("1"), json!(2)), (json!("2"), json!(1))], json!(3), as_served(&json!(18446744073709551617.0_f64))));
    // A value column that is no number, refused in the panel's place by its engine type.
    assert_eq!(d["panels"][35]["error"], json!("`worded`'s `word` is VARCHAR, not a number: cells chart a numeric column"));
    // A target and a limit of more places than the screen's three, each written in full (the
    // tiles' thresholds of four places are read with the others above).
    assert_eq!(panel(&d, 38)["sub"], json!("+19799900% on the 0.0005 target"));
    assert_eq!((panel(&d, 39)["limit"].clone(), panel(&d, 39)["limit_text"].clone()), (json!({ "above": 0.004 }), json!("0.004")));
    assert_eq!((panel(&d, 40)["target"].clone(), panel(&d, 40)["target_text"].clone()), (json!(0.0005), json!("0.0005")));
    // A cells key of every date and time type the engine lists (the driver's TIME_NS apart, the
    // limit), each written as the store's clock reads it, never its milliseconds.
    let kc = live.store.query_rows("keyclocks", &veredas_portal::store::RowsQuery { limit: 100, ..Default::default() }).unwrap();
    let key_types: Vec<(String, String)> = kc.columns.iter().skip(4).map(|c| (c.name.clone(), c.data_type.clone())).collect();
    assert_eq!(key_types.iter().map(|(_, t)| t.clone()).chain(["TIME_NS".to_string()]).collect::<BTreeSet<_>>(), engine, "a key of every date and time type");
    for (i, (name, t)) in key_types.iter().enumerate() {
        let c = veredas_supervisao::panel::cells(&live, "keyclocks", name, "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
        let got: BTreeMap<String, u64> = c["rows"].as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap().to_string(), r["n"].as_u64().unwrap())).collect();
        let mut want: BTreeMap<String, u64> = BTreeMap::new();
        for r in &kc.rows {
            *want.entry(clock(r[4 + i].as_i64().unwrap(), read_as(t))).or_default() += 1;
        }
        assert_eq!(got, want, "keyed by {name}, a {t}");
    }
    // Cells keys in their column's own order, by its engine type: a text column's digits as text
    // ("10" before "5"), the same numbers in a numeric column as numbers.
    assert_eq!(keyed(41), [(json!("10"), json!(1)), (json!("5"), json!(2))], "a text key's digits in text's order");
    assert_eq!(keyed(42), [(json!("5"), json!(2)), (json!("10"), json!(1))], "a numeric key in numbers' order");
    // A row with no key and one with no value charted in no row, and counted.
    let nulls = panel(&d, 43);
    assert_eq!((keyed(43), nulls["read"].clone(), nulls["left_out"].clone()), (vec![(json!("1"), json!(1))], json!(3), json!(2)), "{nulls}");
    for i in [10, 11, 14, 23, 26, 28, 29, 30, 34, 41, 42, 46] {
        assert_eq!(panel(&d, i)["left_out"], json!(0), "panel {i}: nothing left out");
    }
    // A boolean read as 1 or 0 by its rules, its bar and its sort, written true or false: false
    // first, then the two trues, each warned and at the bar's full length.
    let flags: Vec<Vec<Value>> = panel(&d, 44)["rows"].as_array().unwrap().iter().map(|r| vec![r[0]["text"].clone(), r[0]["bar"].clone(), r[0]["state"].clone(), r[0]["word"].clone()]).collect();
    let hot = vec![json!("true"), json!(1.0), json!("warn"), json!("hot")];
    assert_eq!(flags, vec![vec![json!("false"), json!(0.0), json!("normal"), json!("")], hot.clone(), hot], "a boolean column");
    // The day's end, which the engine holds apart from midnight, written as itself, in a table and as
    // a key of its own after midnight's.
    assert_eq!(texts(45), [[json!("00:00:00")], [json!("24:00:00")], [json!("00:00:00")]], "24:00:00 as itself");
    assert_eq!(keyed(46), [(json!("00:00:00"), json!(2)), (json!("24:00:00"), json!(1))], "24:00:00 a key of its own");
    // Times past the clock's reach (the engine's infinities, a year past chrono's) written as the
    // milliseconds the store holds.
    let far = panel(&d, 47);
    let far_row = far["rows"][0].as_array().unwrap();
    assert_eq!(far_row.len(), 5, "{far}");
    for c in far_row {
        let ms = c["value"].as_i64().unwrap_or_else(|| panic!("a far time as milliseconds: {c}"));
        assert!(chrono::DateTime::from_timestamp_millis(ms).is_none(), "{ms}: past the clock's reach");
        assert_eq!(c["text"], json!(ms.to_string()), "a far time written as its milliseconds: {c}");
    }
    // A spread of four places, written in full for the page to draw as written.
    assert_eq!((panel(&d, 48)["spread"].clone(), panel(&d, 48)["spread_text"].clone(), panel(&d, 10)["spread_text"].clone()), (json!(0.0005), json!("0.0005"), json!("5")));
    // A keyed object's rule written out in each form (its source alone, its source and series, all
    // three) is the object's own, key and all: on a timeline beside the object's rule left bare, the
    // same runs; on a tile, the same reading, never another series named.
    let ktl = panel(&d, 49);
    let runs = |i: usize| ktl["rows"][i]["segments"].clone();
    for (i, id) in ["k-source", "k-series", "k-key"].into_iter().enumerate() {
        assert_eq!((runs(i + 1), ktl["rows"][i + 1]["now"]["state"].clone()), (runs(0), ktl["rows"][0]["now"]["state"].clone()), "{id} on the timeline");
        let (tile, own) = (panel(&d, 50 + i), panel(&d, 5));
        for k in ["state", "text", "sub", "spark", "word"] {
            assert_eq!(tile[k], own[k], "{id}'s tile, its {k}");
        }
    }
    // Cells keys in their column's own order by its engine type: a key column of every type the
    // engine lists as NUMERIC (its own catalog) and a boolean, each column's values such that text's
    // order is not the column's (a sign, a count of digits, a value past 64 bits either way), the
    // boolean's false before its true. A NUMERIC type the engine adds fails here until a column
    // holds it.
    let kn = live.store.query_rows("keynums", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).unwrap();
    let kn_types: Vec<(String, String)> = kn.columns.iter().skip(4).map(|c| (c.name.clone(), c.data_type.clone())).collect();
    let held: BTreeSet<String> = kn_types.iter().map(|(_, t)| t.split('(').next().unwrap().to_string()).filter(|t| t != "BOOLEAN").collect();
    assert_eq!(held, numeric, "a key column of every NUMERIC type the engine lists");
    assert!(kn_types.iter().any(|(_, t)| t == "BOOLEAN"), "{kn_types:?}");
    let (signed, unsigned, halves, past) = (vec!["-2", "-1", "10"], vec!["5", "10", "100"], vec!["-1.5", "-0.5", "10.5"], vec!["5", "10", "18446744073709551617"]);
    let want_keys: BTreeMap<&str, Vec<&str>> = [
        ("ti", signed.clone()), ("si", signed.clone()), ("i", signed.clone()), ("bi", signed),
        ("hi", vec!["-18446744073709551617", "-18446744073709551616", "18446744073709551617"]), ("hi5", past.clone()),
        ("uti", unsigned.clone()), ("usi", unsigned.clone()), ("ui", unsigned.clone()), ("ubi", vec!["5", "10", "18446744073709551615"]), ("uhi", unsigned),
        ("f", halves.clone()), ("d", halves.clone()), ("de", halves), ("de38", past),
        ("b", vec!["false", "true"]),
    ]
    .into_iter()
    .collect();
    assert_eq!(kn_types.iter().map(|(n, _)| n.as_str()).collect::<BTreeSet<_>>(), want_keys.keys().cloned().collect::<BTreeSet<_>>(), "every key column read");
    for (name, t) in &kn_types {
        let c = veredas_supervisao::panel::cells(&live, "keynums", name, "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
        let got: Vec<(String, u64)> = c["rows"].as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap().to_string(), r["n"].as_u64().unwrap())).collect();
        let want: Vec<(String, u64)> = want_keys[name.as_str()].iter().map(|k| (k.to_string(), if *k == "true" { 2 } else { 1 })).collect();
        assert_eq!((got, c["left_out"].clone()), (want.clone(), json!(0)), "keyed by {name}, a {t}");
        if t != "BOOLEAN" {
            let keys: Vec<&String> = want.iter().map(|(k, _)| k).collect();
            let mut as_text = keys.clone();
            as_text.sort();
            assert_ne!(as_text, keys, "{name}: values whose order text's would keep tell nothing");
        }
    }
    // Places at the reader's bounds, 0 and 6: a small integer, one a double cannot hold, one past 64
    // bits (portal's digits) and the largest unsigned less the travel, each with no point under 0 and
    // six zeros under 6; newest first.
    let want: Vec<Vec<Value>> = [25u64, 20, 0]
        .iter()
        .map(|t| {
            let neg = *t == 20;
            let row = [if neg { "-5" } else { "5" }.to_string(), if neg { "-9007199254740995" } else { "9007199254740993" }.to_string(), (u64::MAX - t).to_string(), if neg { "-18446744073709551617" } else { "18446744073709551617" }.to_string()];
            row.iter().map(|s| json!(s)).chain(row.iter().map(|s| json!(format!("{s}.000000")))).collect()
        })
        .collect();
    assert_eq!(texts(53), want, "places at 0 and at 6");
    // A sample that is no finite number (NaN, an infinity: a plant can send one and portal keeps it)
    // is no value: left out of a trend's points and its total, a tile's sparkline and every bucket
    // alike (a bucket of none of them empty), never drawn as 0.
    let finite = [(at(21.0), 97.0), (at(23.0), 99.0), (at(25.0), 98.0)];
    let pairs = json!(finite.iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>());
    assert_eq!((panel(&d, 54)["points"].clone(), panel(&d, 54)["total"].clone(), panel(&d, 55)["spark"].clone()), (pairs.clone(), json!(3), pairs), "the finite samples alone");
    let mut per: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for (t, v) in finite {
        per.entry(t.div_euclid(5 * MIN) * 5 * MIN).or_default().push(v);
    }
    let want: Vec<(i64, u64, f64)> = per.into_iter().map(|(s, vs)| (s, vs.len() as u64, vs.iter().sum::<f64>() / vs.len() as f64)).collect();
    let got: Vec<(i64, u64, f64)> = panel(&d, 56)["buckets"].as_array().unwrap().iter().filter(|b| !b["value"].is_null()).map(|b| (b["start"].as_i64().unwrap(), b["n"].as_u64().unwrap(), b["value"].as_f64().unwrap())).collect();
    assert_eq!(got, want, "each bucket's mean of its finite samples, counted");
    assert!(panel(&d, 56)["buckets"].as_array().unwrap().iter().all(|b| b["value"].is_null() == (b["n"] == json!(0))), "a bucket empty only of finite samples");
    // (A view's double that is no finite number: portal writes it as null (C-041's conversion), so
    // its cell is blank and its rules and bar read no value; the limit, pinned.)
    let nf: Vec<Vec<Value>> = panel(&d, 57)["rows"].as_array().unwrap().iter().map(|r| vec![r[0]["text"].clone(), r[0]["state"].clone(), r[1]["text"].clone()]).collect();
    assert_eq!(nf, vec![vec![json!(""), json!("normal"), json!("")], vec![json!("5"), json!("normal"), json!("1")]], "non-finite doubles as portal writes them");
    // Cells keys of a time kind in the clock's order, never their text's: a date and a timestamp past
    // the year 9999 (written with a sign) and the engine's infinities (written as their milliseconds)
    // beside dates of this century.
    let kt = live.store.query_rows("keytimes", &veredas_portal::store::RowsQuery { limit: 10, ..Default::default() }).unwrap();
    for (i, col) in kt.columns.iter().enumerate().skip(4) {
        let fmt = if col.data_type == "DATE" { "%Y-%m-%d" } else { "%Y-%m-%d %H:%M:%S" };
        let mut raw: Vec<i64> = kt.rows.iter().map(|r| r[i].as_i64().unwrap_or_else(|| panic!("{}: {}", col.name, r[i]))).collect();
        raw.sort();
        raw.dedup();
        let want: Vec<String> = raw.iter().map(|ms| chrono::DateTime::from_timestamp_millis(*ms).map(|t| t.format(fmt).to_string()).unwrap_or(ms.to_string())).collect();
        let c = veredas_supervisao::panel::cells(&live, "keytimes", &col.name, "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
        let got: Vec<String> = c["rows"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(got, want, "keyed by {}, a {}, in the clock's order", col.name, col.data_type);
        let mut as_text = want.clone();
        as_text.sort();
        assert_ne!(as_text, want, "{}: values whose order text's would keep tell nothing", col.name);
    }
    // An ENUM key in text's order, as any type but a number, a time or a boolean (its labels' own
    // order is not read: the stated rule).
    let en = veredas_supervisao::panel::cells(&live, "enums", "lv", "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
    let keys: Vec<&str> = en["rows"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["high", "low", "mid"], "an ENUM's keys as text");
    // A key of negative zero is the key 0, a double's and a single's alike (one value; the crate's
    // number writers write no "-0"), its rows one row of the cells.
    for k in ["k", "kf"] {
        let z = veredas_supervisao::panel::cells(&live, "zeros", k, "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
        let keys: Vec<(Value, Value)> = z["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
        assert_eq!(keys, vec![(json!("0"), json!(3))], "{k}: -0 and 0 one key");
    }
    // A key of the engine's single-precision FLOAT by that float's own text, never the double portal
    // hands over (0.1 as 0.10000000149011612).
    let si = veredas_supervisao::panel::cells(&live, "singles", "f", "travel", 60, 5, 5.0, to, 50, 1000).unwrap();
    let keys: Vec<&str> = si["rows"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["0.1", "0.2", "0.3"], "a FLOAT's keys as written");
    // Finite samples whose sum or squares pass a double's reach: their mean, median and σ all the same
    // (the largest double twice, a "no reading" some controllers write; ±1e155, whose squares pass
    // it), through the route's cells and in a mean's bucket.
    let m = f64::MAX;
    let top = veredas_supervisao::panel::cells(&live, "extremes", "key", "top", 60, 5, 5.0, to, 50, 1000).unwrap();
    assert_eq!((top["rows"][0]["median"].clone(), top["rows"][0]["sigma"].clone(), top["rows"][0]["n"].clone()), (json!(m), json!(0.0), json!(3)), "{top}");
    let wide = veredas_supervisao::panel::cells(&live, "extremes", "key", "wide", 60, 5, 5.0, to, 50, 1000).unwrap();
    let s = wide["rows"][0]["sigma"].as_f64().unwrap_or_else(|| panic!("{wide}"));
    assert!((s / 9.428090415820634e154 - 1.0).abs() < 1e-12, "σ of 1e155 once and -1e155 twice: {s}");
    use veredas_supervisao::panel::{buckets, mean, median, sigma};
    assert_eq!((mean(&[m, m]), median(&[m, m]), sigma(&[m, m]), sigma(&[m, -m]), median(&[-m, m])), (Some(m), Some(m), Some(0.0), Some(m), Some(0.0)));
    let b = buckets(&[(0, m), (1, m)], 0, 299_999, 300_000, veredas_supervisao::binding::Aggregate::Mean);
    assert_eq!((b[0]["value"].clone(), b[0]["n"].clone()), (json!(m), json!(2)), "a mean's bucket of the largest double twice");
    host.stop();
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
    // Two logs feeding the sorts (the rule's scope dropped): their sequences overlap from 0, so a row
    // is known by its source and its sequence together.
    let scoped = "scope = \"sorter\"\ntypes = { outlet";
    assert!(ex.project.contains(scoped));
    ex.project = ex.project.replace(scoped, "types = { outlet");
    ex.project.push_str("\n[[log]]\nid = \"second\"\nfile = \"second.log\"\ntimestamp = { pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }\n");
    // Views as the cells table: one with the three columns rows are read once by, two without.
    ex.project.push_str("\n[[table]]\nname = \"whole\"\nsql = \"SELECT index_ms, source, seq, outlet, travel FROM sorts\"\n");
    ex.project.push_str("\n[[table]]\nname = \"unsourced\"\nsql = \"SELECT index_ms, outlet, travel FROM sorts\"\n");
    ex.project.push_str("\n[[table]]\nname = \"unstamped\"\nsql = \"SELECT ts, source, seq, outlet, travel FROM sorts\"\n");
    let path = write_example(dir.path(), &ex);
    let log = dir.path().join("loop/sorts.log");
    let second = dir.path().join("loop/second.log");
    std::fs::write(&log, "").unwrap();
    std::fs::write(&second, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let live = host.session("loop").unwrap().live();
    assert!(
        wait(Duration::from_secs(20), || {
            let h = live.health.read().unwrap();
            h.get("sorter").is_some_and(|h| h.count == 0) && h.get("second").is_some_and(|h| h.count == 0)
        }),
        "the logs are attached"
    );
    let rows = || live.store.query_rows("sorts", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    // Three hundred sorts over the last half hour, a hundred per outlet; and ninety from the second
    // log, outlet 10, between them.
    let now = store_now();
    append(&log, &(0..300).map(|i| format!("{} SORT outlet={} travel={} rate=99.0", stamp(now - 30 * MIN + i * 5_000), 1 + i % 3, 10.0 + (i % 7) as f64)).collect::<Vec<_>>());
    append(&second, &(0..90).map(|i| format!("{} SORT outlet=10 travel=12.0 rate=99.0", stamp(now - 30 * MIN + i * 20_000 + 1_250))).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || rows() == 390), "{}", rows());
    let sources = live.store.query_rows("whole", &veredas_portal::store::RowsQuery { limit: 1000, ..Default::default() }).unwrap();
    let si = sources.columns.iter().position(|c| c.name == "source").unwrap();
    let mut by: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in &sources.rows {
        *by.entry(r[si].as_str().unwrap().to_string()).or_default() += 1;
    }
    assert_eq!(by, [("second".to_string(), 90), ("sorter".to_string(), 300)].into_iter().collect(), "both logs in the one table");
    // A view without them is refused, naming what it lacks.
    let refused = |t: &str| veredas_supervisao::panel::cells(&live, t, "outlet", "travel", 60, 5, 5.0, now, 2, 50_000).unwrap_err();
    assert_eq!(refused("unsourced"), "`unsourced` has no `source`, `seq` (cells read each row once by `index_ms`, `source` and `seq`; its columns: index_ms, outlet, travel)");
    assert_eq!(refused("unstamped"), "`unstamped` has no `index_ms` (cells read each row once by `index_ms`, `source` and `seq`; its columns: ts, source, seq, outlet, travel)");
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
        // The base table and the view that carries its three columns, read alike.
        for table in ["sorts", "whole"] {
            let before = rows();
            let c = veredas_supervisao::panel::cells(&live, table, "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
            grew += rows() - before;
            reads += 1;
            let ns: Vec<(Value, Value)> = c["rows"].as_array().unwrap().iter().filter(|r| r["key"] != "9").map(|r| (r["key"].clone(), r["n"].clone())).collect();
            assert_eq!(ns, [(json!("1"), json!(100)), (json!("2"), json!(100)), (json!("3"), json!(100)), (json!("10"), json!(90))], "each sort once, {table}, read {reads}: {c}");
        }
    }
    writer.join().unwrap();
    assert!(grew > 0, "rows arrived while the cells were read ({reads} reads)");
    // Rows sharing a stamp, a page's end falling among them (four at one stamp between two others,
    // read two rows a page): each row read once, those already taken at the bound's stamp skipped.
    let tie = now - 25 * MIN + 2_500;
    let sort = |ms: i64, outlet: u32| format!("{} SORT outlet={outlet} travel=10.0 rate=99.0", stamp(ms));
    append(&log, &[sort(tie - 500, 4), sort(tie, 4), sort(tie, 4), sort(tie, 5), sort(tie, 6), sort(tie + 500, 6)]);
    assert!(wait(Duration::from_secs(20), || rows() == 546), "{}", rows());
    let c = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
    let ns: Vec<(Value, Value)> = c["rows"].as_array().unwrap().iter().map(|r| (r["key"].clone(), r["n"].clone())).collect();
    let want: Vec<(Value, Value)> = [("1", 100), ("2", 100), ("3", 100), ("4", 3), ("5", 1), ("6", 2), ("9", 150), ("10", 90)].into_iter().map(|(k, n)| (json!(k), json!(n))).collect();
    assert_eq!(ns, want, "each row once across tied stamps: {c}");
    assert_eq!((c["read"].clone(), c["total"].clone(), c["capped"].clone()), (json!(546), json!(546), json!(false)));
    // Rows arriving at the very stamp a page ended on: two hundred of outlet 7 at the window's
    // newest stamp, so every read pages through them first (a hundred pages of two), then a hundred
    // and fifty of outlet 8 at that stamp, ten milliseconds apart, while the cells are read again
    // and again. The store sorts the newcomers ahead of the rows read there, and still every row
    // read is read once: outlet 7 exactly two hundred, no key past the rows stored for it.
    let at = now + 5 * MIN;
    append(&log, &(0..200).map(|_| sort(at, 7)).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || rows() == 746), "{}", rows());
    let (stop, appended) = (Arc::new(AtomicBool::new(false)), Arc::new(std::sync::atomic::AtomicU64::new(0)));
    let writer = std::thread::spawn({
        let (stop, appended, log, line) = (Arc::clone(&stop), Arc::clone(&appended), log.clone(), sort(at, 8));
        move || {
            for _ in 0..150 {
                append(&log, std::slice::from_ref(&line));
                appended.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(10));
            }
            stop.store(true, Ordering::Relaxed);
        }
    });
    let stored = [("1", 100), ("2", 100), ("3", 100), ("4", 3), ("5", 1), ("6", 2), ("7", 200), ("9", 150), ("10", 90)];
    let (mut reads, mut grew) = (0, 0);
    while !stop.load(Ordering::Relaxed) || reads < 2 {
        let before = rows();
        let c = veredas_supervisao::panel::cells(&live, "sorts", "outlet", "travel", 60, 5, 5.0, to, 2, 50_000).unwrap();
        grew += rows() - before;
        reads += 1;
        for r in c["rows"].as_array().unwrap() {
            let (k, n) = (r["key"].as_str().unwrap(), r["n"].as_u64().unwrap());
            let most = if k == "8" { appended.load(Ordering::Relaxed) } else { stored.iter().find(|(s, _)| *s == k).map(|(_, m)| *m).unwrap_or_else(|| panic!("key {k}: {c}")) };
            assert!(n <= most, "key {k} counted {n}, {most} stored, read {reads}: {c}");
            if k == "7" {
                assert_eq!(n, 200, "outlet 7 once each, read {reads}: {c}");
            }
        }
    }
    writer.join().unwrap();
    assert!(grew > 0, "rows arrived at the paged stamp while the cells were read ({reads} reads)");
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
    // The limit the clause states: a width that divides no day counts from the epoch, so a week
    // starts on a Thursday (the epoch's day) and fifty minutes start off the hour.
    let week = 7 * day;
    let weekly = buckets(&pts, at(7, 17), at(9, 30), week, Aggregate::Count);
    let monday_or_thursday = chrono::DateTime::from_timestamp_millis(weekly[0]["start"].as_i64().unwrap()).unwrap().format("%A").to_string();
    assert_eq!(monday_or_thursday, "Thursday");
    let fifty = buckets(&pts, at(7, 17), at(9, 30), 50 * 60_000, Aggregate::Count);
    assert_eq!(fifty[0]["start"].as_i64().unwrap() % hour != 0, true, "fifty minutes from the epoch, off the hour: {}", fifty[0]["start"]);
}

#[test]
fn a_table_sorts_numbers_as_numbers_with_nulls_last() {
    use veredas_supervisao::panel::sort_rows;
    let rows = || vec![vec![json!(10.5)], vec![Value::Null], vec![json!(9.5)], vec![json!(20.5)], vec![json!(100)]];
    let column = |r: &[Vec<Value>]| r.iter().map(|x| x[0].clone()).collect::<Vec<_>>();
    let mut down = rows();
    sort_rows(&mut down, 0, true, true);
    assert_eq!(column(&down), [json!(100), json!(20.5), json!(10.5), json!(9.5), Value::Null], "numbers, not their text; the null last");
    let mut up = rows();
    sort_rows(&mut up, 0, false, true);
    assert_eq!(column(&up), [json!(9.5), json!(10.5), json!(20.5), json!(100), Value::Null]);
    let mut words = vec![vec![json!("b")], vec![Value::Null], vec![json!("a")]];
    sort_rows(&mut words, 0, true, false);
    assert_eq!(column(&words), [json!("b"), json!("a"), Value::Null]);
    // Digits in a text column are text (a code is no number), whatever their length; in a numeric
    // column the digits portal writes for an integer past 64 bits are that integer, beside numbers.
    let digits = || vec![vec![json!("100000000000000000000")], vec![json!("99999999999999999999")], vec![json!("5")], vec![json!("9000000000000000000")]];
    let mut text = digits();
    sort_rows(&mut text, 0, false, false);
    assert_eq!(column(&text), [json!("100000000000000000000"), json!("5"), json!("9000000000000000000"), json!("99999999999999999999")]);
    let mut huge = vec![vec![json!("18446744073709551617")], vec![json!(5)], vec![json!("-18446744073709551617")], vec![json!("18446744073709551616")]];
    sort_rows(&mut huge, 0, false, true);
    assert_eq!(column(&huge), [json!("-18446744073709551617"), json!(5), json!("18446744073709551616"), json!("18446744073709551617")]);
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
