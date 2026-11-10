//! C-048: the supervisão file and the binding file, as `docs/binding-schema.md` states them: the
//! worked example loaded with every statement checked against what was read, the executable's
//! check on it, and every refusal by its text, each a one-line change to the example.

mod common;

use std::path::Path;

use common::{Example, example, write_example};
use veredas_core::SeriesType;
use veredas_supervisao::binding::{Aggregate, Limit, PanelKind, Show, Universe};
use veredas_supervisao::config::{self, ConfigError};
use veredas_supervisao::state::{Comparand, Compare, StateClass};

fn load(dir: &Path, ex: &Example) -> Result<config::HostConfig, ConfigError> {
    config::load_file(&write_example(dir, ex))
}

/// The reason a binding was refused for, the session and the path aside.
fn binding_refusal(ex: &Example) -> String {
    let dir = tempfile::tempdir().unwrap();
    match load(dir.path(), ex) {
        Err(ConfigError::Binding { session, reason, .. }) => {
            assert_eq!(session, "loop");
            reason
        }
        Err(other) => panic!("not a binding refusal: {other}"),
        Ok(_) => panic!("loaded"),
    }
}

fn host_refusal(ex: &Example) -> String {
    let dir = tempfile::tempdir().unwrap();
    match load(dir.path(), ex) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("loaded"),
    }
}

fn with_binding(f: impl FnOnce(&str) -> String) -> Example {
    let mut ex = example();
    let b = f(&ex.binding);
    assert_ne!(b, ex.binding, "the change applies");
    ex.binding = b;
    ex
}

#[test]
fn the_worked_example_loads_as_the_reference_states_it() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load(dir.path(), &example()).unwrap();
    assert_eq!(cfg.bind.to_string(), "127.0.0.1:8902");
    assert_eq!(cfg.sessions.len(), 1);
    let s = &cfg.sessions[0];
    assert_eq!(s.name, "loop");
    assert!(s.project_path.ends_with("loop/project.toml") && s.binding_path.ends_with("loop/binding.toml"));
    assert_eq!((s.project.name.as_str(), s.project.writes_armed), ("the loop", false));
    let b = &s.binding;
    assert_eq!(b.title, "The loop");
    let layout = b.layout.as_ref().unwrap();
    assert!(layout.file.ends_with("loop/layout.svg"));
    assert_eq!(layout.ids.keys().cloned().collect::<Vec<_>>(), ["infeed", "loop", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"]);
    let ids: Vec<&str> = b.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"]);
    // Two objects have no element in the layout: they show in the headline and the dashboard.
    let on_layout: Vec<&str> = ids.iter().copied().filter(|i| layout.ids.contains_key(*i)).collect();
    assert_eq!(on_layout, ["infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"]);
    assert_eq!(b.headline.iter().map(|i| b.objects[*i].id.as_str()).collect::<Vec<_>>(), ["speed", "read-rate"]);

    // Every object as written, the defaults filled.
    let speed = b.object("speed").unwrap();
    assert_eq!((speed.label.as_str(), speed.unit.as_str(), speed.decimals, speed.show, speed.range, speed.normal.as_str(), speed.stale_after_ms), ("Loop speed", "m/s", Some(1), Show::State, (0.0, 100.0), "running", None));
    assert_eq!((speed.value.source.as_str(), speed.value.series.as_str(), speed.value.key.as_deref(), speed.ty), ("plc", "speed", None, SeriesType::Float));
    assert_eq!(speed.rules.len(), 1);
    assert_eq!((&speed.rules[0].compare, speed.rules[0].class, speed.rules[0].word.as_str(), &speed.rules[0].series), (&Compare::Below(0.5), StateClass::Warn, "slow", &None));
    let rate = b.object("read-rate").unwrap();
    assert_eq!((rate.value.source.as_str(), rate.value.series.as_str(), rate.ty, rate.unit.as_str(), rate.stale_after_ms, rate.normal.as_str()), ("sorter", "rate", SeriesType::Float, "%", Some(30000), "ok"));
    assert_eq!((&rate.rules[0].compare, rate.rules[0].class, rate.rules[0].word.as_str()), (&Compare::Below(98.0), StateClass::Alarm, "under 98 %"));
    let infeed = b.object("infeed").unwrap();
    assert_eq!((infeed.ty, infeed.unit.as_str(), infeed.decimals, infeed.label.as_str()), (SeriesType::Int, "", None, "Infeed"));
    assert_eq!(infeed.rules.iter().map(|r| (r.compare.clone(), r.class, r.word.clone())).collect::<Vec<_>>(), [
        (Compare::Equals(Comparand::Num(0.0)), StateClass::Warn, "stopped".to_string()),
        (Compare::Equals(Comparand::Num(2.0)), StateClass::Alarm, "fault".to_string()),
    ]);
    for (i, series) in [(1, "fill1"), (2, "fill2"), (3, "fill3")] {
        let o = b.object(&format!("outlet-{i}")).unwrap();
        assert_eq!((o.label.clone(), o.value.series.as_str(), o.show, o.decimals, o.unit.as_str(), o.range), (format!("Outlet {i}"), series, Show::Level, Some(0), "%", (0.0, 100.0)));
        assert_eq!(o.rules.iter().map(|r| (r.compare.clone(), r.class, r.word.clone())).collect::<Vec<_>>(), [
            (Compare::AtLeast(100.0), StateClass::Warn, "full".to_string()),
            (Compare::AtLeast(80.0), StateClass::Near, "near full".to_string()),
        ]);
        assert_eq!(o.actions.len(), if i == 3 { 1 } else { 0 });
    }
    let action = &b.object("outlet-3").unwrap().actions[0];
    assert_eq!((action.label.as_str(), action.source.as_str(), action.tag.as_str(), action.value.as_str()), ("Release", "plc", "ns=2;s=release3", "1"));
    for p in ["parcel-1", "parcel-2"] {
        let o = b.object(p).unwrap();
        assert_eq!((o.show, o.path.as_deref(), o.range, o.rules.len(), o.normal.as_str()), (Show::Move, Some("loop"), (0.0, 100.0), 0, "ok"));
    }

    // The dashboard: eight panels, one of every kind, as written.
    assert_eq!(b.dashboards.len(), 1);
    let d = &b.dashboards[0];
    assert_eq!((d.name.as_str(), d.columns), ("Overview", 4));
    let kinds: Vec<&str> = d.panels.iter().map(|p| p.kind.name()).collect();
    assert_eq!(kinds, ["value", "trend", "bars", "timeline", "grid", "levels", "table", "cells"]);
    let idx = |id: &str| b.objects.iter().position(|o| o.id == id).unwrap();
    assert_eq!(d.panels[0].kind, PanelKind::Value { object: idx("read-rate"), minutes: 30, target: Some(99.0) });
    assert_eq!((d.panels[0].title.as_str(), d.panels[0].span), ("Read rate", 1));
    assert_eq!(d.panels[1].kind, PanelKind::Trend { object: idx("read-rate"), minutes: 60, limit: Some(Limit::Below(98.0)) });
    assert_eq!(d.panels[1].span, 2);
    assert_eq!(d.panels[2].kind, PanelKind::Bars { object: idx("speed"), minutes: 60, bucket_minutes: 5, aggregate: Aggregate::Mean, target: Some(1.5) });
    assert_eq!(d.panels[3].kind, PanelKind::Timeline { objects: vec![idx("infeed")], minutes: 60 });
    assert_eq!(d.panels[3].title, "State over time");
    let outlets = vec![idx("outlet-1"), idx("outlet-2"), idx("outlet-3")];
    assert_eq!(d.panels[4].kind, PanelKind::Grid { objects: outlets.clone() });
    assert_eq!(d.panels[5].kind, PanelKind::Levels { objects: outlets, ticks: vec![80.0, 90.0] });
    match &d.panels[6].kind {
        PanelKind::Table { table, columns, sort, limit } => {
            assert_eq!((table.as_str(), sort.clone(), *limit), ("outlets", Some(("sorted".to_string(), true)), 50));
            let cols: Vec<(&str, &str, bool, Option<u8>)> = columns.iter().map(|c| (c.name.as_str(), c.label.as_str(), c.bar, c.decimals)).collect();
            assert_eq!(cols, [("outlet", "outlet", false, None), ("sorted", "sorted", true, None), ("travel", "travel", false, Some(1))]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(d.panels[6].title, "outlets");
    assert_eq!(d.panels[7].kind, PanelKind::Cells { table: "sorts".into(), key: "outlet".into(), value: "travel".into(), minutes: 60, bucket_minutes: 5, spread: 10.0 });

    // What the project carries, as the binding sees it.
    let u = Universe::of(&s.project);
    assert_eq!(u.sources.keys().cloned().collect::<Vec<_>>(), ["plc", "sorter"]);
    let sorter = &u.sources["sorter"];
    assert_eq!(sorter.kind, "log");
    assert_eq!(sorter.series.iter().map(|(k, t)| (k.as_str(), *t)).collect::<Vec<_>>(), [("outlet", SeriesType::Int), ("rate", SeriesType::Float), ("travel", SeriesType::Float)]);
    let plc = &u.sources["plc"];
    assert_eq!(plc.kind, "opc");
    assert_eq!(plc.series.keys().cloned().collect::<Vec<_>>(), ["fill1", "fill2", "fill3", "infeed", "p1", "p2", "release3", "speed"]);
    assert_eq!((plc.series["infeed"], plc.series["speed"]), (SeriesType::Int, SeriesType::Float));
    assert_eq!(plc.writable.iter().cloned().collect::<Vec<_>>(), ["ns=2;s=release3"]);
    assert_eq!(u.tables.keys().cloned().collect::<Vec<_>>(), ["outlets", "plc_main", "sorts"]);
    let cols = |t: &str| u.tables[t].as_ref().map(|c| c.iter().map(|(n, ty)| (n.as_str(), *ty)).collect::<Vec<_>>());
    assert_eq!(cols("sorts").unwrap(), [("arrival_ms", SeriesType::Int), ("index_ms", SeriesType::Int), ("key", SeriesType::Text), ("outlet", SeriesType::Int), ("rate", SeriesType::Float), ("seq", SeriesType::Int), ("source", SeriesType::Text), ("travel", SeriesType::Float), ("ts", SeriesType::Time)]);
    assert!(cols("plc_main").unwrap().contains(&("fill3", SeriesType::Float)) && cols("plc_main").unwrap().contains(&("infeed", SeriesType::Int)));
    assert_eq!(cols("outlets"), None, "a view's columns are the engine's, known when the session runs");
}

#[test]
fn the_executable_checks_the_example_and_prints_its_plan() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_example(dir.path(), &example());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).arg("--check").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "supervisão on http://127.0.0.1:8902/\n\
         session `loop` at /s/loop/: \"The loop\", the project \"the loop\"\n  \
         8 object(s), 6 on the layout (layout.svg); 1 write(s) offered; headline: speed, read-rate\n  \
         dashboard `Overview`: 8 panel(s): value, trend, bars, timeline, grid, levels, table, cells\n  \
         writes off; portal's surface at /s/loop/portal/\n"
    );
    // A refused file is named with its reason and exits 2; a call of another shape is a usage error.
    let mut bad = example();
    bad.binding = bad.binding.replace("id = \"speed\"", "id = \"1speed\"");
    let path = write_example(dir.path(), &bad);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).arg("--check").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.trim_end().ends_with("object 0: the id `1speed` does not start with an ASCII letter"), "{err}");
    for args in [vec![], vec!["--check"], vec!["a.toml", "b.toml"], vec!["a.toml", "--chek"]] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim_end(), "usage: veredas-supervisao <supervisao.toml> [--check]");
    }
}

#[test]
fn a_binding_refuses_what_it_cannot_light_up_by_its_place() {
    let cases: Vec<(Box<dyn Fn(&str) -> String>, &str)> = vec![
        (Box::new(|b| b.replace("format = \"veredas-binding\"", "format = \"veredas-bindings\"")), "`format` is `veredas-bindings`, not `veredas-binding`"),
        (Box::new(|b| b.replace("version = 1", "version = 2")), "version 2 of `veredas-binding` is not known here (this reader knows 1)"),
        (Box::new(|b| b.replace("id = \"speed\"", "id = \"1speed\"")), "object 0: the id `1speed` does not start with an ASCII letter"),
        (Box::new(|b| b.replace("id = \"speed\"", "id = \"sp eed\"")), "object 0: the id `sp eed` holds ` `: an id is ASCII letters, digits, `-`, `_` and `.`"),
        (Box::new(|b| b.replace("id = \"speed\"", &format!("id = \"s{}\"", "x".repeat(64)))), "object 0: the id `sxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx` is longer than 64"),
        (Box::new(|b| b.replace("id = \"read-rate\"", "id = \"Speed\"").replace("headline = [\"speed\", \"read-rate\"]", "headline = [\"speed\"]").replace("object = \"read-rate\"", "object = \"speed\"")), "object `Speed`: the id is taken by `speed` (ids differ by more than their case)"),
        (Box::new(|b| b.replace("value = { source = \"plc\", series = \"speed\" }", "value = { source = \"plcx\", series = \"speed\" }")), "object `speed`, value: `plcx` is no source of the project (its sources: `plc`, `sorter`)"),
        (Box::new(|b| b.replace("value = { source = \"plc\", series = \"speed\" }", "value = { source = \"plc\", series = \"speeed\" }")), "object `speed`, value: the opc source `plc` carries no series `speeed` (it carries: `fill1`, `fill2`, `fill3`, `infeed`, `p1`, `p2`, `release3`, `speed`)"),
        (Box::new(|b| b.replace("value = { source = \"plc\", series = \"speed\" }", "value = { source = \"plc\", series = \"speed\", key = \"\" }")), "object `speed`, value: the key is empty"),
        (Box::new(|b| b.replace("show = \"level\"\nstates = [ { at_least = 100, state = \"warn\", word = \"full\" }, { at_least = 80, state = \"near\", word = \"near full\" } ]\n\n[[object]]\nid = \"outlet-2\"", "show = \"gauge\"\n\n[[object]]\nid = \"outlet-2\"")), "object `outlet-1`: `show` is `gauge`, none of `state`, `level`, `value`, `move`"),
        (Box::new(|b| b.replace("label = \"Infeed\"", "label = \"Infeed\"\npath = \"loop\"")), "object `infeed`: `path` is for a marker (`show = \"move\"`)"),
        (Box::new(|b| b.replacen("show = \"move\"\npath = \"loop\"", "show = \"move\"", 1)), "object `parcel-1`: a marker needs the `path` it follows"),
        (Box::new(|b| b.replacen("path = \"loop\"", "path = \"lop\"", 1)), "object `parcel-1`: the path `lop` is no element of the layout"),
        (Box::new(|b| b.replacen("path = \"loop\"", "path = \"loop\"\nrange = [100, 0]", 1)), "object `parcel-1`: the range is empty (100 is not below 0)"),
        (Box::new(|b| b.replacen("path = \"loop\"", "path = \"loop\"\nrange = [0, 50, 100]", 1)), "object `parcel-1`: `range` takes a low and a high, not 3 values"),
        (Box::new(|b| b.replacen("path = \"loop\"", "path = \"loop\"\ndecimals = 7", 1)), "object `parcel-1`: `decimals` is 7, past 6"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ state = \"warn\", word = \"slow\" }")), "object `speed`, states[0]: a rule needs one comparison (`equals`, `above`, `below`, `at_least` or `at_most`)"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ below = 0.5, above = 3, state = \"warn\", word = \"slow\" }")), "object `speed`, states[0]: a rule takes one comparison, this one has 2"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ below = 0.5, state = \"full\", word = \"slow\" }")), "object `speed`, states[0]: the state `full` is none of `near`, `warn`, `alarm`, `disabled`"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ below = 0.5, state = \"normal\" }")), "object `speed`, states[0]: the state `normal` is none of `near`, `warn`, `alarm`, `disabled`"),
        (Box::new(|b| b.replace("{ equals = 0, state = \"warn\", word = \"stopped\" }", "{ equals = true, state = \"warn\", word = \"stopped\" }")), "object `infeed`, states[0]: `equals` takes a number or a string, not boolean"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ below = nan, state = \"warn\" }")), "object `speed`, states[0]: `below` is not a finite number"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ equals = 0, series = \"infeed\", source = \"plcx\", state = \"warn\" }")), "object `speed`, states[0]: `plcx` is no source of the project (its sources: `plc`, `sorter`)"),
        (Box::new(|b| b.replace("{ below = 0.5, state = \"warn\", word = \"slow\" }", "{ equals = 0, source = \"plc\", state = \"warn\" }")), "object `speed`, states[0]: a rule over another series names its `series`"),
        (Box::new(|b| b.replace("{ label = \"Release\", source = \"plc\"", "{ label = \"Release\", source = \"sorter\"")), "object `outlet-3`, actions[0]: `sorter` is no OPC source of the project"),
        (Box::new(|b| b.replace("tag = \"ns=2;s=release3\"", "tag = \"ns=2;s=speed\"")), "object `outlet-3`, actions[0]: `ns=2;s=speed` is no tag of `plc` a rule marks writable (its writable tags: `ns=2;s=release3`)"),
        (Box::new(|b| b.replace("tag = \"ns=2;s=release3\"", "tag = \"release3\"")), "object `outlet-3`, actions[0]: `release3` is no tag of `plc` a rule marks writable (its writable tags: `ns=2;s=release3`)"),
        (Box::new(|b| b.replace("label = \"Release\"", "label = \" \"")), "object `outlet-3`, actions[0]: the label is empty"),
        (Box::new(|b| b.replace("stale_after_ms = 30000", "stale_after_ms = 0")), "object `read-rate`: `stale_after_ms` is 0"),
        (Box::new(|b| b.replace("headline = [\"speed\", \"read-rate\"]", "headline = [\"sped\"]")), "headline: `sped` names no object"),
        (Box::new(|b| b.replace("kind = \"grid\"\nobjects = [\"outlet-*\"]", "kind = \"grid\"\nobjects = [\"chute-*\"]")), "dashboard `Overview`, panel[4] (grid): `chute-*` names no object"),
        (Box::new(|b| b.replace("kind = \"grid\"\nobjects = [\"outlet-*\"]", "kind = \"grid\"\nobjects = []")), "dashboard `Overview`, panel[4] (grid): `objects` names no object"),
        (Box::new(|b| b.replace("kind = \"value\"", "kind = \"gauge\"")), "dashboard `Overview`, panel[0] (gauge): the kind `gauge` is none of `value`, `trend`, `bars`, `timeline`, `grid`, `levels`, `table`, `cells`"),
        (Box::new(|b| b.replace("limit = { below = 98.0 }\nspan = 2", "limit = { below = 98.0 }\nspan = 2\nbucket_minutes = 5")), "dashboard `Overview`, panel[1] (trend): unknown field `bucket_minutes`, expected one of `kind`, `title`, `span`, `object`, `minutes`, `limit`"),
        (Box::new(|b| b.replace("limit = { below = 98.0 }\nspan = 2", "limit = { below = 98.0 }\nspan = 5")), "dashboard `Overview`, panel[1] (trend): `span` is 5, not 1 to the dashboard's 4 columns"),
        (Box::new(|b| b.replace("limit = { below = 98.0 }", "limit = { below = 98.0, above = 99.0 }")), "dashboard `Overview`, panel[1] (trend): a limit is `{ above = x }` or `{ below = x }`"),
        (Box::new(|b| b.replace("bucket_minutes = 5\naggregate = \"mean\"", "bucket_minutes = 5\naggregate = \"median\"")), "dashboard `Overview`, panel[2] (bars): `aggregate` is `median`, none of `mean`, `sum`, `count`, `min`, `max`, `last`"),
        (Box::new(|b| b.replace("table = \"outlets\"", "table = \"outletz\"")), "dashboard `Overview`, panel[6] (table): `outletz` is no table of the project"),
        (Box::new(|b| b.replace("sort = { column = \"sorted\", descending = true }", "sort = { column = \"travelled\", descending = true }")), "dashboard `Overview`, panel[6] (table): the sort column `travelled` is none of the panel's columns"),
        (Box::new(|b| b.replace("spread = 10.0", "spread = 0.0")), "dashboard `Overview`, panel[7] (cells): `spread` is not a number above 0"),
        (Box::new(|b| b.replace("columns = 4", "columns = 13")), "dashboard `Overview`: `columns` is 13, not 1 to 12"),
        (Box::new(|b| b.replace("[[dashboard]]\nname = \"Overview\"", "[[dashboard]]\nname = \"Overview\"\n\n[[dashboard]]\nname = \"Overview\"")), "dashboard `Overview`: the name is taken"),
    ];
    for (i, (change, expected)) in cases.iter().enumerate() {
        let got = binding_refusal(&with_binding(|b| change(b)));
        assert_eq!(got, *expected, "case {i}");
    }
    // A misspelled key anywhere is the parser's refusal, with its place.
    for (from, to, key) in [("unit = \"m/s\"", "units = \"m/s\"", "units"), ("{ below = 0.5, state", "{ bellow = 0.5, state", "bellow"), ("title = \"The loop\"", "titel = \"The loop\"", "titel")] {
        let got = binding_refusal(&with_binding(|b| b.replace(from, to)));
        assert!(got.starts_with("not a binding file: TOML parse error at line ") && got.contains(&format!("unknown field `{key}`")), "{got}");
    }
}

#[test]
fn a_binding_is_checked_against_its_project_and_its_layout() {
    // A key group carries no series.
    let mut ex = example();
    ex.project = ex.project.replace("types = { outlet = \"int\", travel = \"float\", rate = \"float\" }", "types = { outlet = \"int\", travel = \"float\", rate = \"float\" }\nkey = \"outlet\"");
    ex.binding = ex.binding.replace("value = { source = \"sorter\", series = \"rate\" }", "value = { source = \"sorter\", series = \"outlet\" }");
    assert_eq!(binding_refusal(&ex), "object `read-rate`, value: the log source `sorter` carries no series `outlet` (it carries: `rate`, `travel`)");
    // A group with no type is text, and a level needs a number.
    let mut ex = example();
    ex.project = ex.project.replace("types = { outlet = \"int\", travel = \"float\", rate = \"float\" }", "types = { outlet = \"int\", travel = \"float\" }");
    ex.binding = ex.binding.replace("value = { source = \"plc\", series = \"fill1\" }", "value = { source = \"sorter\", series = \"rate\" }");
    assert_eq!(binding_refusal(&ex), "object `outlet-1`: a level needs a numeric series, and `rate` is text");
    // The same text series on a trend: the panel charts numbers.
    let mut ex = example();
    ex.project = ex.project.replace("types = { outlet = \"int\", travel = \"float\", rate = \"float\" }", "types = { outlet = \"int\", travel = \"float\" }");
    assert_eq!(binding_refusal(&ex), "dashboard `Overview`, panel[1] (trend): `read-rate` reads `rate`, which is text, and the panel charts numbers");
    // The layout: an element no marker can follow, an object's id on two elements, a document
    // that is not an SVG, a script, a file not there.
    let mut ex = example();
    ex.svg = ex.svg.replace("</svg>", "  <g id=\"group\"/>\n</svg>");
    ex.binding = ex.binding.replacen("path = \"loop\"", "path = \"group\"", 1);
    assert_eq!(binding_refusal(&ex), "object `parcel-1`: the path `group` is a <g>, which no marker can follow (one of path, line, polyline, polygon, rect, circle, ellipse)");
    let mut ex = example();
    ex.svg = ex.svg.replace("</svg>", "  <rect id=\"infeed\"/>\n</svg>");
    assert_eq!(binding_refusal(&ex), "object `infeed`: the layout carries the id on 2 elements");
    let mut ex = example();
    ex.svg = ex.svg.replace("</svg>", "  <rect id=\"loop\"/>\n</svg>");
    assert_eq!(binding_refusal(&ex), "object `parcel-1`: the path `loop` is carried by 2 elements of the layout");
    let mut ex = example();
    ex.svg = "<html/>".into();
    let got = binding_refusal(&ex);
    assert!(got.starts_with("the layout: ") && got.ends_with("layout.svg is not an SVG document: its root is <html>"), "{got}");
    let mut ex = example();
    ex.svg = ex.svg.replace("</svg>", "<script>alert(1)</script></svg>");
    assert!(binding_refusal(&ex).ends_with("layout.svg holds a <script> element: a layout is a drawing, with no script and no foreign content"));
    let mut ex = example();
    ex.svg = ex.svg.replace("<rect id=\"infeed\"", "<rect onmouseover=\"x()\" id=\"infeed\"");
    assert!(binding_refusal(&ex).ends_with("layout.svg holds an event handler (`onmouseover` on <rect>): a layout is a drawing, with no script"));
    let ex = with_binding(|b| b.replace("layout = \"layout.svg\"", "layout = \"missing.svg\""));
    let got = binding_refusal(&ex);
    assert!(got.starts_with("the layout: cannot read ") && got.contains("missing.svg"), "{got}");
    let ex = with_binding(|b| b.replace("layout = \"layout.svg\"", "layout = \" \""));
    assert_eq!(binding_refusal(&ex), "`layout` is empty");
    // Without a layout, a marker has nothing to follow; the rest stands.
    let ex = with_binding(|b| b.replace("layout = \"layout.svg\"\n", ""));
    assert_eq!(binding_refusal(&ex), "object `parcel-1`: a marker follows an element of the layout, and the binding has no `layout`");
    // The title defaults to the project's name.
    let dir = tempfile::tempdir().unwrap();
    let ex = with_binding(|b| b.replace("title = \"The loop\"\n", ""));
    assert_eq!(load(dir.path(), &ex).unwrap().sessions[0].binding.title, "the loop");
}

#[test]
fn a_supervisao_file_refuses_what_it_cannot_serve_by_its_place() {
    let cases: Vec<(Box<dyn Fn(&str) -> String>, &str)> = vec![
        (Box::new(|h| h.replace("format = \"veredas-supervisao\"", "format = \"veredas-supervisor\"")), "`format` is `veredas-supervisor`, not `veredas-supervisao`"),
        (Box::new(|h| h.replace("version = 1", "version = 3")), "version 3 of `veredas-supervisao` is not known here (this reader knows 1)"),
        (Box::new(|h| h.replace("format = \"veredas-supervisao\"\n", "")), "no `format`: a supervisão file states `format` and `version`"),
        (Box::new(|h| h.replace("bind = \"127.0.0.1:8902\"", "bind = \"localhost\"")), "`bind` is `localhost`, not an address and a port (`127.0.0.1:8902`)"),
        (Box::new(|h| h[..h.find("[[session]]").unwrap()].to_string()), "no session: a supervisão file names at least one `[[session]]`"),
        (Box::new(|h| h.replace("name = \"loop\"", "name = \"Loop\"")), "session 0: the name `Loop` does not start with a lowercase ASCII letter or a digit"),
        (Box::new(|h| h.replace("name = \"loop\"", "name = \"lo_op\"")), "session 0: the name `lo_op` holds `_`: a name is lowercase ASCII letters, digits and `-`"),
        (Box::new(|h| h.replace("name = \"loop\"", &format!("name = \"{}\"", "l".repeat(33)))), "session 0: the name `lllllllllllllllllllllllllllllllll` is longer than 32"),
        (Box::new(|h| format!("{h}\n[[session]]\nname = \"loop\"\nproject = \"loop/project.toml\"\nbinding = \"loop/binding.toml\"\n")), "session `loop`: the name is taken"),
        (Box::new(|h| h.replace("binding = \"loop/binding.toml\"", "binding = \"\"")), "session `loop`: `project` and `binding` name files"),
    ];
    for (i, (change, expected)) in cases.iter().enumerate() {
        let mut ex = example();
        ex.host = change(&ex.host);
        assert_eq!(host_refusal(&ex), *expected, "case {i}");
    }
    let mut ex = example();
    ex.host = ex.host.replace("bind = \"127.0.0.1:8902\"", "bindd = \"127.0.0.1:8902\"");
    let got = host_refusal(&ex);
    assert!(got.starts_with("not a supervisão file: TOML parse error at line ") && got.contains("unknown field `bindd`"), "{got}");
    // The bind's default is the loopback (Q20).
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.host = ex.host.replace("bind = \"127.0.0.1:8902\"\n", "");
    assert_eq!(load(dir.path(), &ex).unwrap().bind.to_string(), "127.0.0.1:8902");
    // A project that is not there, or does not load, is refused with portal's reason.
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.host = ex.host.replace("project = \"loop/project.toml\"", "project = \"loop/nope.toml\"");
    match load(dir.path(), &ex) {
        Err(ConfigError::Project { session, path, reason }) => {
            assert_eq!(session, "loop");
            assert!(path.ends_with("loop/nope.toml"));
            assert!(reason.starts_with("the project file: cannot read "), "{reason}");
        }
        other => panic!("{:?}", other.err()),
    }
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.project = ex.project.replace("format = \"veredas-project\"", "format = \"veredas-projekt\"");
    match load(dir.path(), &ex) {
        Err(ConfigError::Project { reason, .. }) => assert_eq!(reason, "`format` is `veredas-projekt`, not `veredas-project`"),
        other => panic!("{:?}", other.err()),
    }
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.host = ex.host.replace("binding = \"loop/binding.toml\"", "binding = \"loop/nope.toml\"");
    let got = host_refusal_in(dir.path(), &ex);
    assert!(got.starts_with("session `loop`: the binding ") && got.contains("nope.toml: cannot read it: "), "{got}");
}

fn host_refusal_in(dir: &Path, ex: &Example) -> String {
    match load(dir, ex) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("loaded"),
    }
}

// ---- the reference, held here ----

/// A text's paragraphs, each with its whitespace folded: what the reference says, whatever its
/// line breaks.
fn paragraphs(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    let actual = paragraphs(&common::doc("binding-schema.md"));
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/binding-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len(), "docs/binding-schema.md has {} paragraphs, the copy {}", actual.len(), expected.len());
}

/// The keys a reader takes in a table, from its refusal of a misspelled key there.
fn reader_keys(err: &str) -> std::collections::BTreeSet<String> {
    let at = err.find("expected ").unwrap_or_else(|| panic!("{err}"));
    let line = err[at..].lines().next().unwrap();
    line.split('`').skip(1).step_by(2).map(String::from).collect()
}

/// The keys a section of the reference lists: every backticked word that opens a bullet, or
/// follows one before its parenthesis (`a`, `b` (...)), and the braces of an inline table.
fn listed_keys(section: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for line in section.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("- ") else { continue };
        let head = rest.split(" (").next().unwrap_or("");
        for k in head.split(',') {
            let k = k.trim();
            if let Some(k) = k.strip_prefix('`').and_then(|k| k.strip_suffix('`')) {
                out.insert(k.to_string());
            }
        }
    }
    out
}

/// The text of the reference between two headings or bold labels.
fn section<'a>(doc: &'a str, from: &str, to: &str) -> &'a str {
    let a = doc.find(from).unwrap_or_else(|| panic!("{from}"));
    let b = doc[a + from.len()..].find(to).map(|b| a + from.len() + b).unwrap_or(doc.len());
    &doc[a..b]
}

fn keys(names: &[&str]) -> std::collections::BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_key_the_reference_lists_is_the_reader_s() {
    let doc = common::doc("binding-schema.md");
    let host_refusal = |text: &str| -> String {
        let dir = tempfile::tempdir().unwrap();
        let mut ex = example();
        ex.host = text.to_string();
        load(dir.path(), &ex).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("loaded: {text}"))
    };
    let ex = example();
    // The supervisão file's two tables.
    let host_top = section(&doc, "## The supervisão file: keys", "**A session**");
    assert_eq!(listed_keys(host_top), keys(&["format", "version", "bind", "session"]));
    assert_eq!(reader_keys(&host_refusal(&ex.host.replace("bind = ", "zz = 1\nbind = "))), listed_keys(host_top));
    let session = section(&doc, "**A session**", "## The binding file: keys");
    assert_eq!(listed_keys(session), keys(&["name", "project", "binding"]));
    assert_eq!(reader_keys(&host_refusal(&ex.host.replace("name = \"loop\"", "name = \"loop\"\nzz = 1"))), listed_keys(session));
    // The binding file's tables, each with a misspelled key placed in it.
    let walk: Vec<(&str, &str, Box<dyn Fn(&str) -> String>, Vec<&str>)> = vec![
        ("**The top level.**", "**An object**", Box::new(|b: &str| b.replace("title = ", "zz = 1\ntitle = ")), vec!["format", "version", "title", "layout", "headline", "object", "dashboard"]),
        ("**An object**", "**A state rule**", Box::new(|b: &str| b.replacen("label = \"Loop speed\"", "label = \"Loop speed\"\nzz = 1", 1)), vec!["id", "label", "value", "unit", "decimals", "show", "range", "path", "states", "normal", "stale_after_ms", "actions"]),
        ("**A state rule**", "**An action**", Box::new(|b: &str| b.replace("{ below = 0.5, state", "{ below = 0.5, zz = 1, state")), vec!["equals", "above", "below", "at_least", "at_most", "state", "word", "source", "series", "key"]),
        ("**An action**", "**A dashboard**", Box::new(|b: &str| b.replace("{ label = \"Release\",", "{ label = \"Release\", zz = 1,")), vec!["label", "source", "tag", "value"]),
        ("**A dashboard**", "**A panel**", Box::new(|b: &str| b.replace("columns = 4", "columns = 4\nzz = 1")), vec!["name", "columns", "panel"]),
    ];
    for (from, to, place, expected) in &walk {
        let listed = listed_keys(section(&doc, from, to));
        // The object's `value` lists its own keys beneath it; they are the series table's.
        let listed: std::collections::BTreeSet<String> = listed.into_iter().filter(|k| !(from.starts_with("**An object") && ["source", "series", "key"].contains(&k.as_str()))).collect();
        assert_eq!(listed, keys(expected), "{from}");
        let err = binding_refusal(&with_binding(|b| place(b)));
        assert_eq!(reader_keys(&err), listed, "{from}: {err}");
    }
    // The bound series, beneath an object's `value`.
    let err = binding_refusal(&with_binding(|b| b.replace("value = { source = \"plc\", series = \"speed\" }", "value = { source = \"plc\", series = \"speed\", zz = 1 }")));
    assert_eq!(reader_keys(&err), keys(&["source", "series", "key"]));
    // Every panel kind: the three every panel has, and the ones its bullet lists.
    let panels = section(&doc, "**A panel**", "## Refusals");
    let common_keys = ["kind", "title", "span"];
    for (kind, place) in [("value", "object = \"read-rate\"\ntarget = 99.0"), ("trend", "limit = { below = 98.0 }"), ("bars", "aggregate = \"mean\""), ("timeline", "objects = [\"infeed\"]"), ("grid", "kind = \"grid\""), ("levels", "ticks = [80, 90]"), ("table", "table = \"outlets\""), ("cells", "spread = 10.0")] {
        let bullet = panels.lines().find(|l| l.starts_with(&format!("- `{kind}`:"))).unwrap_or_else(|| panic!("{kind}"));
        // The bullet runs on in its indented lines.
        let start = panels.find(bullet).unwrap();
        let text: String = panels[start..].lines().take_while(|l| l.starts_with(&format!("- `{kind}`")) || l.starts_with("  ")).collect::<Vec<_>>().join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut listed: std::collections::BTreeSet<String> = common_keys.iter().map(|s| s.to_string()).collect();
        let pieces: Vec<&str> = text.split('`').collect();
        for i in (1..pieces.len()).step_by(2) {
            // A key: a plain word followed by its parenthesis (an inline table's braces are not one).
            if pieces[i].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && pieces.get(i + 1).is_some_and(|after| after.starts_with(" (")) {
                listed.insert(pieces[i].to_string());
            }
        }
        let err = binding_refusal(&with_binding(|b| b.replacen(place, &format!("{place}\nzz = 1"), 1)));
        assert!(err.contains(&format!("({kind}): unknown field `zz`")), "{kind}: {err}");
        assert_eq!(reader_keys(&err), listed, "{kind}: {err}");
    }
    // A table panel's column, its sort and a trend's limit.
    let folded = panels.split_whitespace().collect::<Vec<_>>().join(" ");
    let err = binding_refusal(&with_binding(|b| b.replace("{ name = \"outlet\" }", "{ name = \"outlet\", zz = 1 }")));
    assert_eq!(reader_keys(&err), keys(&["name", "label", "bar", "decimals", "states"]));
    assert!(folded.contains("each `{ name, label, bar, decimals, states }`"));
    let err = binding_refusal(&with_binding(|b| b.replace("sort = { column = \"sorted\", descending = true }", "sort = { column = \"sorted\", descending = true, zz = 1 }")));
    assert_eq!(reader_keys(&err), keys(&["column", "descending"]));
    assert!(folded.contains("`{ column, descending }`"));
    let err = binding_refusal(&with_binding(|b| b.replace("limit = { below = 98.0 }", "limit = { below = 98.0, zz = 1 }")));
    assert_eq!(reader_keys(&err), keys(&["above", "below"]));
    assert!(folded.contains("`{ above = x }` or `{ below = x }`"));
}

#[test]
fn the_rest_of_the_refusals_name_their_place() {
    let cases: Vec<(Box<dyn Fn(&str) -> String>, &str)> = vec![
        (Box::new(|b| b.replace("id = \"speed\"", "id = \"\"").replace("headline = [\"speed\", \"read-rate\"]", "headline = [\"read-rate\"]").replace("object = \"speed\"", "object = \"read-rate\"")), "object 0: the id is empty"),
        (Box::new(|b| b.replacen("path = \"loop\"", "path = \"loop\"\nrange = [nan, 1]", 1)), "object `parcel-1`: the range's low is not a finite number"),
        (Box::new(|b| b.replace("[[dashboard.panel]]\nkind = \"value\"", "[[dashboard.panel]]\nkinds = \"value\"")), "dashboard `Overview`, panel[0]: a panel names its `kind`"),
        (Box::new(|b| b.replace("kind = \"trend\"\nobject = \"read-rate\"\nminutes = 60", "kind = \"trend\"\nobject = \"read-rate\"\nminutes = 0")), "dashboard `Overview`, panel[1] (trend): `minutes` is 0"),
        (Box::new(|b| b.replace("bucket_minutes = 5\naggregate", "bucket_minutes = 0\naggregate")), "dashboard `Overview`, panel[2] (bars): `bucket_minutes` is 0"),
        (Box::new(|b| b.replace("spread = 10.0", "spread = 10.0\nbucket_minutes = 0")), "dashboard `Overview`, panel[7] (cells): `bucket_minutes` is 0"),
        (Box::new(|b| b.replace("target = 99.0", "target = nan")), "dashboard `Overview`, panel[0] (value): `target` is not a finite number"),
        (Box::new(|b| b.replace("limit = { below = 98.0 }", "limit = { below = inf }")), "dashboard `Overview`, panel[1] (trend): the limit is not a finite number"),
        (Box::new(|b| b.replace("ticks = [80, 90]", "ticks = [80, nan]")), "dashboard `Overview`, panel[5] (levels): a tick is not a finite number"),
        (Box::new(|b| b.replace("sort = { column = \"sorted\", descending = true }", "sort = { column = \"sorted\", descending = true }\nlimit = 0")), "dashboard `Overview`, panel[6] (table): `limit` is 0, not 1 to 1000"),
        (Box::new(|b| b.replace("sort = { column = \"sorted\", descending = true }", "sort = { column = \"sorted\", descending = true }\nlimit = 1001")), "dashboard `Overview`, panel[6] (table): `limit` is 1001, not 1 to 1000"),
        (Box::new(|b| b.replace("{ name = \"travel\", decimals = 1 }", "{ name = \"travel\", decimals = 7 }")), "dashboard `Overview`, panel[6] (table), columns[2]: `decimals` is 7, past 6"),
        (Box::new(|b| b.replace("{ name = \"outlet\" }", "{ name = \" \" }")), "dashboard `Overview`, panel[6] (table), columns[0]: the name is empty"),
        (Box::new(|b| b.replace("{ name = \"outlet\" }", "{ name = \"outlet\", states = [ { equals = 1, series = \"rate\", state = \"warn\" } ] }")), "dashboard `Overview`, panel[6] (table), columns[0], states[0]: a column's rule compares the column's own value"),
        (Box::new(|b| b.replace("{ name = \"outlet\" }", "{ name = \"outlet\", states = [ { equals = 1, state = \"full\" } ] }")), "dashboard `Overview`, panel[6] (table), columns[0], states[0]: the state `full` is none of `near`, `warn`, `alarm`, `disabled`"),
        (Box::new(|b| b.replace("[[dashboard]]\nname = \"Overview\"", "[[dashboard]]\nname = \" \"")), "dashboard 0: the name is empty"),
        (Box::new(|b| b.replace("columns = 4", "columns = 0")), "dashboard `Overview`: `columns` is 0, not 1 to 12"),
        (Box::new(|b| b.replace("limit = { below = 98.0 }\nspan = 2", "limit = { below = 98.0 }\nspan = 0")), "dashboard `Overview`, panel[1] (trend): `span` is 0, not 1 to the dashboard's 4 columns"),
    ];
    for (i, (change, expected)) in cases.iter().enumerate() {
        assert_eq!(binding_refusal(&with_binding(|b| change(b))), *expected, "case {i}");
    }
    // A timeline and levels read numbers, as a trend does.
    let mut ex = example();
    ex.project = ex.project.replace("types = { outlet = \"int\", travel = \"float\", rate = \"float\" }", "types = { outlet = \"int\", travel = \"float\" }");
    ex.binding = ex.binding.replace("kind = \"trend\"\nobject = \"read-rate\"", "kind = \"trend\"\nobject = \"speed\"").replace("objects = [\"infeed\"]", "objects = [\"read-rate\"]").replace("kind = \"value\"\nobject = \"read-rate\"", "kind = \"value\"\nobject = \"speed\"");
    assert_eq!(binding_refusal(&ex), "dashboard `Overview`, panel[3] (timeline): `read-rate` reads `rate`, which is text, and the panel charts numbers");
    let mut ex = example();
    ex.project = ex.project.replace("types = { outlet = \"int\", travel = \"float\", rate = \"float\" }", "types = { outlet = \"int\", travel = \"float\" }");
    ex.binding = ex.binding.replace("kind = \"trend\"\nobject = \"read-rate\"", "kind = \"trend\"\nobject = \"speed\"").replace("kind = \"levels\"\nobjects = [\"outlet-*\"]", "kind = \"levels\"\nobjects = [\"read-rate\"]");
    assert_eq!(binding_refusal(&ex), "dashboard `Overview`, panel[5] (levels): `read-rate` reads `rate`, which is text, and the panel charts numbers");
    // A pattern of a star alone names every object, in the binding's order.
    let dir = tempfile::tempdir().unwrap();
    let ex = with_binding(|b| b.replace("kind = \"grid\"\nobjects = [\"outlet-*\"]", "kind = \"grid\"\nobjects = [\"*\", \"speed\"]"));
    let b = load(dir.path(), &ex).unwrap().sessions.remove(0).binding;
    match &b.dashboards[0].panels[4].kind {
        PanelKind::Grid { objects } => assert_eq!(objects.iter().map(|i| b.objects[*i].id.as_str()).collect::<Vec<_>>(), ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2", "speed"]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_edges_the_types_and_every_kind_of_source_and_column() {
    // At the limits: a 32-letter session name and a 64-character id load; a range of one point
    // and a spread past every number are refused.
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    let long = "l".repeat(32);
    ex.host = ex.host.replace("name = \"loop\"", &format!("name = \"{long}\""));
    ex.binding = ex.binding.replace("id = \"speed\"", &format!("id = \"s{}\"", "x".repeat(63))).replace("headline = [\"speed\", \"read-rate\"]", "headline = [\"read-rate\"]").replace("object = \"speed\"", "object = \"read-rate\"");
    let cfg = load(dir.path(), &ex).unwrap();
    assert_eq!((cfg.sessions[0].name.len(), cfg.sessions[0].binding.objects[0].id.len()), (32, 64));
    assert_eq!(binding_refusal(&with_binding(|b| b.replacen("path = \"loop\"", "path = \"loop\"\nrange = [5, 5]", 1))), "object `parcel-1`: the range is empty (5 is not below 5)");
    assert_eq!(binding_refusal(&with_binding(|b| b.replace("spread = 10.0", "spread = inf"))), "dashboard `Overview`, panel[7] (cells): `spread` is not a number above 0");

    // Types read as portal reads them, in any case: a tag typed TEXT is text, a group FLOAT a float.
    let mut ex = example();
    ex.project = ex.project.replace("series = \"fill1\", type = \"float\"", "series = \"fill1\", type = \"TEXT\"");
    assert_eq!(binding_refusal(&ex), "object `outlet-1`: a level needs a numeric series, and `fill1` is text");
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.project = ex.project.replace("rate = \"float\" }", "rate = \" FLOAT \" }");
    let u = Universe::of(&load(dir.path(), &ex).unwrap().sessions[0].project);
    assert_eq!(u.sources["sorter"].series["rate"], SeriesType::Float);

    // Two logs: a rule scoped to one carries nothing to the other; an unscoped rule reads both.
    let mut ex = example();
    ex.project = ex.project.replacen("[[rule]]", "[[log]]\nid = \"other\"\nfile = \"other.log\"\n\n[[rule]]\nname = \"anywhere\"\npattern = \"LEN (?P<len>\\\\d+)\"\ntypes = { len = \"int\" }\n\n[[rule]]", 1);
    let dir = tempfile::tempdir().unwrap();
    let u = Universe::of(&load(dir.path(), &ex).unwrap().sessions[0].project);
    assert_eq!(u.sources["other"].series.keys().cloned().collect::<Vec<_>>(), ["len"]);
    assert_eq!(u.sources["sorter"].series.keys().cloned().collect::<Vec<_>>(), ["len", "outlet", "rate", "travel"]);
    let mut bad = ex.clone();
    bad.binding = bad.binding.replace("value = { source = \"sorter\", series = \"rate\" }", "value = { source = \"other\", series = \"rate\" }");
    assert_eq!(binding_refusal(&bad), "object `read-rate`, value: the log source `other` carries no series `rate` (it carries: `len`)");

    // A tag's bits: each bit a series, an int, of its source.
    let mut ex = example();
    ex.project = ex.project.replace("{ node_id = \"ns=2;s=infeed\", series = \"infeed\", type = \"int\" },", "{ node_id = \"ns=2;s=infeed\", series = \"infeed\", type = \"int\", bits = [ { bit = 0, series = \"infeed_on\" }, { bit = 1, width = 2, series = \"infeed_mode\" } ] },");
    ex.binding = ex.binding.replace("value = { source = \"plc\", series = \"infeed\" }", "value = { source = \"plc\", series = \"infeed_mode\" }");
    let dir = tempfile::tempdir().unwrap();
    let cfg = load(dir.path(), &ex).unwrap();
    let u = Universe::of(&cfg.sessions[0].project);
    assert_eq!((u.sources["plc"].series["infeed_on"], u.sources["plc"].series["infeed_mode"]), (SeriesType::Int, SeriesType::Int));
    assert_eq!(cfg.sessions[0].binding.object("infeed").unwrap().ty, SeriesType::Int);

    // A channel: its messages' fields are its series, typed by the catalog; each message a table.
    let dir = tempfile::tempdir().unwrap();
    let mut ex = example();
    ex.project.push_str("\n[[channel]]\nsource_id = \"line\"\nrole = \"connect\"\nhost = \"127.0.0.1\"\nport = 5020\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n");
    ex.binding = ex.binding.replace("value = { source = \"plc\", series = \"speed\" }", "value = { source = \"line\", series = \"weight\" }").replace("table = \"outlets\"", "table = \"announce\"").replace("columns = [ { name = \"outlet\" }, { name = \"sorted\", bar = true }, { name = \"travel\", decimals = 1 } ]\nsort = { column = \"sorted\", descending = true }", "columns = [ { name = \"item\" }, { name = \"code\" } ]");
    write_example(dir.path(), &ex);
    std::fs::write(dir.path().join("loop/channel.toml"), common::blocks("channel-schema.md", "toml")[0].clone()).unwrap();
    std::fs::write(dir.path().join("loop/catalog.toml"), common::blocks("controller-schema.md", "toml")[0].clone()).unwrap();
    let cfg = config::load_file(&dir.path().join("supervisao.toml")).unwrap_or_else(|e| panic!("{e}"));
    let u = Universe::of(&cfg.sessions[0].project);
    let line = &u.sources["line"];
    // A field several messages carry is numeric only where all of them type it so: `item` is a
    // number in two messages and text in the outcome's payload, `code` text then a number.
    assert_eq!((line.kind, line.series["weight"], line.series["lane"], line.series["code"], line.series["item"], line.series["result"]), ("channel", SeriesType::Int, SeriesType::Int, SeriesType::Text, SeriesType::Text, SeriesType::Text));
    let mut item = example();
    item.project = ex.project.clone();
    item.binding = ex.binding.replacen("show = \"level\"", "show = \"value\"", 0).replace("value = { source = \"plc\", series = \"fill1\" }
unit = \"%\"
decimals = 0
show = \"level\"", "value = { source = \"line\", series = \"item\" }
unit = \"%\"
decimals = 0
show = \"level\"");
    let d2 = tempfile::tempdir().unwrap();
    write_example(d2.path(), &item);
    std::fs::write(d2.path().join("loop/channel.toml"), common::blocks("channel-schema.md", "toml")[0].clone()).unwrap();
    std::fs::write(d2.path().join("loop/catalog.toml"), common::blocks("controller-schema.md", "toml")[0].clone()).unwrap();
    match config::load_file(&d2.path().join("supervisao.toml")) {
        Err(ConfigError::Binding { reason, .. }) => assert_eq!(reason, "object `outlet-1`: a level needs a numeric series, and `item` is text"),
        other => panic!("{:?}", other.err()),
    }
    assert!(u.tables.contains_key("announce") && u.tables["announce"].as_ref().unwrap().contains_key("lane"));
    assert_eq!(cfg.sessions[0].binding.object("speed").unwrap().ty, SeriesType::Int);

    // A panel's columns against its table's, where the project states them; a view's when the
    // session runs.
    let cases: Vec<(Box<dyn Fn(&str) -> String>, &str)> = vec![
        (Box::new(|b| b.replace("{ name = \"outlet\" }", "{ name = \"outlett\" }").replace("table = \"outlets\"", "table = \"sorts\"").replace("{ name = \"sorted\", bar = true }", "{ name = \"travel\", bar = true }").replace("{ name = \"travel\", decimals = 1 }", "{ name = \"rate\", decimals = 1 }").replace("sort = { column = \"sorted\", descending = true }", "sort = { column = \"travel\", descending = true }")),
         "dashboard `Overview`, panel[6] (table), columns[0]: `sorts` has no column `outlett` (its columns: `arrival_ms`, `index_ms`, `key`, `outlet`, `rate`, `seq`, `source`, `travel`, `ts`)"),
        (Box::new(|b| b.replace("table = \"outlets\"\ncolumns = [ { name = \"outlet\" }, { name = \"sorted\", bar = true }, { name = \"travel\", decimals = 1 } ]\nsort = { column = \"sorted\", descending = true }", "table = \"sorts\"\nsort = { column = \"sorted\", descending = true }")),
         "dashboard `Overview`, panel[6] (table): the sort column `sorted` is no column of `sorts` (its columns: `arrival_ms`, `index_ms`, `key`, `outlet`, `rate`, `seq`, `source`, `travel`, `ts`)"),
        (Box::new(|b| b.replace("key = \"outlet\"\nvalue = \"travel\"", "key = \"nokey\"\nvalue = \"travel\"")), "dashboard `Overview`, panel[7] (cells): the key `nokey` is no column of `sorts` (its columns: `arrival_ms`, `index_ms`, `key`, `outlet`, `rate`, `seq`, `source`, `travel`, `ts`)"),
        (Box::new(|b| b.replace("key = \"outlet\"\nvalue = \"travel\"", "key = \"outlet\"\nvalue = \"nope\"")), "dashboard `Overview`, panel[7] (cells): the value `nope` is no column of `sorts` (its columns: `arrival_ms`, `index_ms`, `key`, `outlet`, `rate`, `seq`, `source`, `travel`, `ts`)"),
        (Box::new(|b| b.replace("key = \"outlet\"\nvalue = \"travel\"", "key = \"outlet\"\nvalue = \"source\"")), "dashboard `Overview`, panel[7] (cells): the value `source` of `sorts` is not numeric"),
    ];
    for (i, (change, expected)) in cases.iter().enumerate() {
        assert_eq!(binding_refusal(&with_binding(|b| change(b))), *expected, "case {i}");
    }
    // A view's columns pass the check, to be judged by the engine when the panel is computed.
    let dir = tempfile::tempdir().unwrap();
    assert!(load(dir.path(), &with_binding(|b| b.replace("{ name = \"outlet\" }", "{ name = \"whatever\" }"))).is_ok());
}

/// The reference, `docs/binding-schema.md`, as this test holds it: an edit there is an edit here.
const REFERENCE: &str = r###"# The supervisão file and the binding file: `veredas-supervisao` and `veredas-binding`, version 1

Supervisão (`docs/supervisao-plan.md`) runs one or more plants' portal sessions and shows each
through its objects. Two plain-text files say everything it does. The **supervisão file** names
the address it serves and the sessions it runs, each a portal project file
(`docs/project-schema.md`) and a binding file. The **binding file** names a session's objects by
ID, binds each to a series the session carries, says how its value reads as a state (a mark and a
word) and how the layout shows it, names the writes it offers, points to the SVG layout whose
elements carry the same IDs, and lists the dashboards and their panels. No plant is named
anywhere: a plant's supervisão is a folder of files like the ones below, kept outside the suite's
code, and nothing of a plant's engineering exports is read (`docs/questions.md` Q18).

`veredas_supervisao::config::load` reads the supervisão file, every project and binding file it
points at, and every layout a binding points at, and checks each against the others, so a
supervisão file that loads is a host every session of which can start; the executable
`veredas-supervisao <supervisao.toml>` starts it (`--check` reads, checks and prints the plan
without starting anything).

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed, the header read
by core's one reader (`veredas_core::schema`).

## The worked example: a loop with three outlets, two parcels and an infeed

A controller serves the loop's speed, two parcels' places along it, three outlets' fill levels, an
infeed's state and an outlet's release command over OPC UA; the sorter's log records every sort
with its outlet, its travel time and the read rate. The project below reads both; the binding
shows the infeed, the outlets and the parcels on the layout, the speed and the read rate in the
top bar, and one dashboard with a panel of every kind. `crates/veredas-supervisao/tests/files.rs`
writes the four blocks beside each other under the names the files give (`supervisao.toml`,
`loop/project.toml`, `loop/binding.toml`, `loop/layout.svg`), loads them, checks every statement
below against what it read, and runs the executable's check on them; it holds a copy of this text,
so an edit here is an edit there.

The supervisão file:

```toml
format = "veredas-supervisao"
version = 1
bind = "127.0.0.1:8902"

[[session]]
name = "loop"
project = "loop/project.toml"
binding = "loop/binding.toml"
```

The session's project, a portal project file like any other (`docs/project-schema.md`):

```toml
format = "veredas-project"
version = 1
name = "the loop"

[session]
store = "ram"
writes_armed = false

[[log]]
id = "sorter"
file = "sorts.log"
timestamp = { pattern = "^(?P<ts>\\S+ \\S+)", formats = ["%Y-%m-%d %H:%M:%S%.3f"] }

[[rule]]
name = "sorts"
pattern = "SORT outlet=(?P<outlet>\\d+) travel=(?P<travel>[0-9.]+) rate=(?P<rate>[0-9.]+)"
scope = "sorter"
types = { outlet = "int", travel = "float", rate = "float" }

[[opc]]
source_id = "plc"
endpoint_url = "opc.tcp://127.0.0.1:4855/"
poll_ms = 200
rules = [
  { name = "plc_main", tags = [
    { node_id = "ns=2;s=speed", series = "speed", type = "float" },
    { node_id = "ns=2;s=p1", series = "p1", type = "float" },
    { node_id = "ns=2;s=p2", series = "p2", type = "float" },
    { node_id = "ns=2;s=fill1", series = "fill1", type = "float" },
    { node_id = "ns=2;s=fill2", series = "fill2", type = "float" },
    { node_id = "ns=2;s=fill3", series = "fill3", type = "float" },
    { node_id = "ns=2;s=infeed", series = "infeed", type = "int" },
    { node_id = "ns=2;s=release3", series = "release3", type = "int", writable = true },
  ] },
]

[[table]]
name = "outlets"
sql = "SELECT outlet, count(*) AS sorted, avg(travel) AS travel FROM sorts GROUP BY outlet"
```

The binding file:

```toml
format = "veredas-binding"
version = 1
title = "The loop"
layout = "layout.svg"
headline = ["speed", "read-rate"]

[[object]]
id = "speed"
label = "Loop speed"
value = { source = "plc", series = "speed" }
unit = "m/s"
decimals = 1
states = [ { below = 0.5, state = "warn", word = "slow" } ]
normal = "running"

[[object]]
id = "read-rate"
label = "Read rate"
value = { source = "sorter", series = "rate" }
unit = "%"
decimals = 1
states = [ { below = 98.0, state = "alarm", word = "under 98 %" } ]
stale_after_ms = 30000

[[object]]
id = "infeed"
label = "Infeed"
value = { source = "plc", series = "infeed" }
states = [
  { equals = 0, state = "warn", word = "stopped" },
  { equals = 2, state = "alarm", word = "fault" },
]
normal = "running"

[[object]]
id = "outlet-1"
label = "Outlet 1"
value = { source = "plc", series = "fill1" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-2"
label = "Outlet 2"
value = { source = "plc", series = "fill2" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-3"
label = "Outlet 3"
value = { source = "plc", series = "fill3" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]
actions = [ { label = "Release", source = "plc", tag = "ns=2;s=release3", value = "1" } ]

[[object]]
id = "parcel-1"
label = "Parcel 1"
value = { source = "plc", series = "p1" }
show = "move"
path = "loop"

[[object]]
id = "parcel-2"
label = "Parcel 2"
value = { source = "plc", series = "p2" }
show = "move"
path = "loop"

[[dashboard]]
name = "Overview"
columns = 4

[[dashboard.panel]]
kind = "value"
object = "read-rate"
target = 99.0

[[dashboard.panel]]
kind = "trend"
object = "read-rate"
minutes = 60
limit = { below = 98.0 }
span = 2

[[dashboard.panel]]
kind = "bars"
object = "speed"
minutes = 60
bucket_minutes = 5
aggregate = "mean"
target = 1.5

[[dashboard.panel]]
kind = "timeline"
objects = ["infeed"]
minutes = 60

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
columns = [ { name = "outlet" }, { name = "sorted", bar = true }, { name = "travel", decimals = 1 } ]
sort = { column = "sorted", descending = true }

[[dashboard.panel]]
kind = "cells"
table = "sorts"
key = "outlet"
value = "travel"
minutes = 60
spread = 10.0
```

The layout the binding points at, an SVG whose elements carry the objects' IDs:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 220">
  <path id="loop" d="M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z" fill="none" stroke="#26262A" stroke-width="14"/>
  <rect id="infeed" x="20" y="96" width="40" height="28"/>
  <rect id="outlet-1" x="130" y="4" width="24" height="28"/>
  <rect id="outlet-2" x="190" y="4" width="24" height="28"/>
  <rect id="outlet-3" x="250" y="4" width="24" height="28"/>
  <circle id="parcel-1" r="5"/>
  <circle id="parcel-2" r="5"/>
</svg>
```

Loaded, this host serves one session, `loop`, at `/s/loop/` on `127.0.0.1:8902`. Its binding has
eight objects, two of which (`speed`, `read-rate`) have no element in the layout and show in the
headline and the dashboard alone; one dashboard of eight panels; one write, `Release`, offered on
`outlet-3`, posting `1` to the tag `ns=2;s=release3` of `plc` through portal's surface, refused there
while the project leaves its writes off.

## The supervisão file: keys

Each key states in parentheses that it is required, or the value it takes when left out. A path is
relative to the file's folder.

- `format` (required): always `"veredas-supervisao"`.
- `version` (required): this reader knows `1`.
- `bind` (`"127.0.0.1:8902"`): the address and port the host serves, a port of `0` letting the
  system choose. Supervisão has no accounts (Q20): whoever reaches the address sees every session,
  and posts to a session's write routes, which portal refuses unless the project armed its writes;
  an address beyond the loopback opens that to every machine that reaches it.
- `session` (required, at least one): `[[session]]` tables, served in the file's order.

**A session** (`[[session]]`):
- `name` (required): the session's path segment, `/s/<name>/`: a lowercase ASCII letter or digit,
  then lowercase letters, digits and `-`, at most 32; no two sessions share one.
- `project` (required): the path of the session's portal project file, loaded as portal loads it
  (its refusals are portal's, named with the session). The project's own `[server]` address is not
  bound: the session's portal surface is served at `/s/<name>/portal/`.
- `binding` (required): the path of the session's binding file.

## The binding file: keys

A path is relative to the binding file's folder.

**The top level.**
- `format` (required): always `"veredas-binding"`.
- `version` (required): this reader knows `1`.
- `title` (the project's name): the session's name on the screen.
- `layout` (none): the path of the SVG layout. Without one the session has dashboards and no
  layout view.
- `headline` (none): object IDs shown in the top bar, in order.
- `object` (none): `[[object]]` tables.
- `dashboard` (none): `[[dashboard]]` tables.

**An object** (`[[object]]`): something on the plant with a value.
- `id` (required): an ASCII letter, then ASCII letters, digits, `-`, `_` and `.`, at most 64; no two
  objects share one, whatever the case. The layout's element with this `id` shows the object.
- `label` (`id`): the object's name on the screen.
- `value` (required): the series it reads, an inline table:
  - `source` (required): a source of the session's project (a log, an OPC source, a channel).
  - `series` (required): a series that source carries: a capture rule's group (not its key group)
    for a log the rule reads, an OPC tag's or bit's series for an OPC source, a message's field for
    a channel; typed as portal types it (a type's word in any case), and, where several rules or
    messages carry it, numeric only where every one of them types it so.
  - `key` (none): the one key read, for a numeric series whose samples carry keys (a text or a
    time series has no value per key to read); absent, the latest sample whatever its key, or, for
    a marker, one marker per key.
- `unit` (`""`): shown after the value.
- `decimals` (none): the places a number is shown with, 0 to 6; absent, as it reads, at most three
  places, trailing zeros dropped.
- `show` (`"state"`): how the layout shows the object: `state` (its element takes the state's
  class, and its mark and word when not normal), `level` (as `state`, and its element fills from
  the bottom by the value's place in `range`, a shape or a text within its outline, a group its
  box), `value` (as `state`, and the value written over its element) or `move` (its element is a
  marker placed along `path` by the value's place in `range`). A level and a marker need a numeric
  series.
- `range` (`[0, 100]`): the low and the high of a level or a marker, low below high, both finite.
- `path` (required for `move`, refused otherwise): the `id` of the layout element the marker
  follows, a `path`, `line`, `polyline`, `polygon`, `rect`, `circle` or `ellipse`.
- `states` (none): the rules, in order; the first that holds names the state; none holding, the
  state is normal.
- `normal` (`"ok"`): the word of the normal state.
- `stale_after_ms` (none): a freshness bound above 0: with it, the object is stale when no sample of its
  series arrived within that many milliseconds, even while its source is live; for a marker per
  key, a key with no sample within it is no longer shown (absent, after 10000 ms).
- `actions` (none): the writes the object offers, as inline tables.

**A state rule** (an entry of `states`): exactly one comparison, a state and a word.
- `equals`, `above`, `below`, `at_least`, `at_most` (one, required): the value compared; `equals`
  takes a number (compared as numbers) or a string (compared as text, exactly), the others a
  finite number.
- `state` (required): `near`, `warn`, `alarm` or `disabled`; normal is the absence of a rule, and
  stale is no rule's: it is judged on the arrival clock.
- `word` (the state's name): the word shown with the mark.
- `source`, `series`, `key` (the object's): another series the rule compares instead of the
  object's value, by the same rules as `value`'s keys.

The states read with a mark and a word everywhere: normal ●, near ■, warn ■, alarm !, disabled ▨,
stale ◌, and the page's own selected ◆. Stale overrides the rules: an object with no value yet, whose source is away (idle past the
project's window, retrying, ended or failed, or waiting again once it has delivered: an OPC source
reconnecting, a channel whose peer left), or whose value is older than its `stale_after_ms`, is
stale, a last value kept and greyed.

**An action** (an entry of `actions`): a write the side panel offers, posted to portal's write
surface (`POST /s/<name>/portal/api/opc/write`), never written by supervisão itself.
- `label` (required): the button's text.
- `source` (required): an OPC source of the project.
- `tag` (required): the tag's node id as the project's OPC rule writes it (portal names a tag by it), a tag the rule marks writable.
- `value` (required): the value written, as text, coerced by portal to the tag's type.

The page shows an action gated (the warn colour) and disabled while the session's switch is off;
portal answers a post 403 while it is off, whoever sends it.

**A dashboard** (`[[dashboard]]`):
- `name` (required): its name on the screen, unique in the file.
- `columns` (`4`): the grid's columns, 1 to 12.
- `panel` (none): `[[dashboard.panel]]` tables, laid out in order.

**A panel** (`[[dashboard.panel]]`): every panel has a `kind` (required), a `title` (the object's
label for a panel of one object, the table's name for a table's, else `"State over time"`,
`"States"` or `"Levels"`) and a `span` (`1`: the columns it takes, 1 to the
dashboard's `columns`). An entry of `objects` ending in `*` names every object whose ID starts with
what precedes it, in the binding's order, and must name at least one. A window is the last
`minutes` up to now on the store's time axis; `minutes` and `bucket_minutes` are above 0. A
trend, bars, a timeline and levels chart numbers: their objects read numeric series. A column a
panel names is checked against its table's where the project states them (a capture rule's, an OPC
rule's, a message's, each beside `source`, `key`, `index_ms`, `ts`, `arrival_ms` and `seq`); a
view's are the engine's, judged when the panel is computed.
- `value`: a tile with the object's value, unit, state and a sparkline. `object` (required),
  `minutes` (`30`: the sparkline's window), `target` (none: the tile says how far the value is from
  it, in percent).
- `trend`: the object's series over a window with a dashed limit and its breaches marked.
  `object` (required), `minutes` (`60`), `limit` (none): `{ above = x }` or `{ below = x }`; a
  breach is a point past it.
- `bars`: the object's series in buckets of `bucket_minutes` aligned on the clock, the bucket now
  running marked, against a dashed `target`. `object` (required), `minutes` (`60`),
  `bucket_minutes` (`5`), `aggregate` (`"mean"`: `mean`, `sum`, `count`, `min`, `max` or `last`),
  `target` (none).
- `timeline`: per object one bar of its state over the window, each state lasting until the next
  sample, the one holding at the window's start (its last sample within the day before) drawn from
  the start. `objects` (required), `minutes` (`60`). The objects' series must be numeric.
- `grid`: one small cell per object with its value and state. `objects` (required).
- `levels`: one horizontal bar per object, its value's place in the object's `range`, with ticks.
  `objects` (required), `ticks` (none: values in the objects' units).
- `table`: rows of a table of the session, newest first. `table` (required: a capture rule, an OPC
  rule, a channel message or a view of the project), `columns` (none: every column), each
  `{ name, label, bar, decimals, states }` (`name` required; `bar` true draws the value as a bar
  against the column's largest in the page; `states` rules as an object's), `sort` (none:
  `{ column, descending }`, applied to the rows shown, a column of the panel's, or of the table's
  when the panel names none), `limit` (`50`, 1 to 1000). A column's `decimals` are 0 to 6; its
  rules compare its own value.
- `cells`: charts in cells, one row per value of a key column of a table over a window: the
  median, a sparkline of the median per `bucket_minutes`, the distribution from median − `spread`
  to median + `spread` in 20 bins, σ and n. `table` (required), `key` (required), `value`
  (required: a numeric column), `minutes` (`60`), `bucket_minutes` (`5`), `spread` (required,
  above 0). It reads at most the newest 50 000 rows of the window, and says so when it did.

## Refusals

A misspelled key anywhere; a header of another format or version; a session name of another shape
or twice; a project or binding file that cannot be read or does not load (with portal's reason
for a project); an object ID of another shape or twice whatever the case; a source the project
lacks; a series its source does not carry (with the series it does carry); a level or a marker on
a series that is not numeric; places past 6; a freshness bound of 0; a `path` on an object that
does not move, a marker without one, or one naming no element of the layout or an element no
marker can follow; a `range` empty or not finite; a rule with no comparison or two, a state outside
the four, a comparison that is not a finite number; an action whose source is no OPC source of the
project or whose tag no rule marks writable, or whose label is empty; a headline or panel naming no
object, an `objects` pattern matching none; an empty dashboard name, or one twice; columns outside 1
to 12; a panel without a kind, of an unknown kind, or with a key its kind does not take; a span
past the columns; `minutes` or `bucket_minutes` of 0; a target, a limit or a tick that is not a
finite number, a limit naming both sides, an unknown aggregate; a trend, bars, a timeline or levels
over a series that is not numeric; a table the project lacks; a table panel's limit outside 1 to
1000, a column with an empty name, places past 6 or a rule over another series, a sort column the
panel or its table lacks; a column, key or value the table lacks where the project states its
columns, a value that is not numeric; a `spread` not above 0; a layout that cannot be read, is not
an SVG document (its root an `svg` in the SVG namespace), or holds a script, foreign content (an
element outside the SVG namespace), an event handler or a script link (a `javascript:` URL in any
attribute, read as a browser reads a URL): each is refused when the file loads, with its place
named, before any session starts.
"###;

// ---- the layout alone (src/svg.rs) ----

use std::path::Path as SvgPath;
use veredas_supervisao::svg::parse;

#[test]
fn a_layout_is_an_svg_document_with_no_script() {
    let p = SvgPath::new("l.svg");
    let ok = parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"a\"><path id=\"loop\" d=\"M0 0\"/></g><rect id=\"a\"/></svg>", p).unwrap();
    assert_eq!(ok.element("loop"), Some("path"));
    assert_eq!(ok.element("a"), None, "two elements carry it");
    assert_eq!(ok.ids["a"].1, 2);
    assert_eq!(parse("<html/>", p).unwrap_err(), "l.svg is not an SVG document: its root is <html>");
    assert!(parse("<svg>", p).unwrap_err().starts_with("l.svg is not an XML document: "));
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><script>x()</script></svg>", p).unwrap_err(), "l.svg holds a <script> element: a layout is a drawing, with no script and no foreign content");
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect onclick=\"x()\"/></svg>", p).unwrap_err(), "l.svg holds an event handler (`onclick` on <rect>): a layout is a drawing, with no script");
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect ONLOAD=\"x()\"/></svg>", p).unwrap_err(), "l.svg holds an event handler (`ONLOAD` on <rect>): a layout is a drawing, with no script");
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect one=\"1\"/></svg>", p).is_err(), "any on* attribute");
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect o=\"1\" on=\"1\"/></svg>", p).is_ok(), "on alone names no event");
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><foreignObject/></svg>", p).is_err());
    // A name with a letter past ASCII is read, never cut inside it.
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect o\u{e9}=\"1\" \u{e9}t\u{e9}=\"2\"/></svg>", p).is_ok());
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect on\u{e9}=\"1\"/></svg>", p).is_err(), "on followed by anything is an event's");
    // The root in the SVG namespace, or no drawing a browser lays out.
    assert_eq!(parse("<svg/>", p).unwrap_err(), "l.svg is not an SVG document: its root is not in the SVG namespace (http://www.w3.org/2000/svg)");
    assert_eq!(parse("<svg xmlns=\"urn:not-svg\"/>", p).unwrap_err(), "l.svg is not an SVG document: its root is not in the SVG namespace (http://www.w3.org/2000/svg)");
    assert!(parse("<s:svg xmlns:s=\"http://www.w3.org/2000/svg\"><s:rect id=\"r\"/></s:svg>", p).unwrap().element("r") == Some("rect"), "a prefix is a spelling");
    // A link that runs script, in either attribute's spelling, whatever its case and spacing.
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><a href=\" JavaScript:alert(1)\"><rect/></a></svg>", p).unwrap_err(), "l.svg holds a script link (`JavaScript:alert(1)` on <a>): a layout is a drawing, with no script");
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><a xlink:href=\"javascript:x()\"/></svg>", p).is_err());
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><a href=\"#loop\"><rect/></a></svg>", p).is_ok(), "a link within the drawing is a drawing's");
    // As a browser reads a URL: a tab, a line break or a carriage return inside the scheme is
    // dropped, leading controls trimmed; in any attribute, an animation's `to` among them.
    for (spelled, shown) in [("java&#9;script:alert(1)", "java\tscript:alert(1)"), ("java&#10;script:alert(1)", "java\nscript:alert(1)"), ("java&#13;script:alert(1)", "java\rscript:alert(1)"), ("&#9; javascript:alert(1)", "javascript:alert(1)")] {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><a href=\"{spelled}\"><rect/></a></svg>");
        assert_eq!(parse(&svg, p).unwrap_err(), format!("l.svg holds a script link (`{}` on <a>): a layout is a drawing, with no script", shown.trim()), "{spelled}");
    }
    assert!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><a xlink:href=\"java&#9;script:x()\"/></svg>", p).is_err());
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><a><set attributeName=\"href\" to=\"javascript:alert(1)\"/></a></svg>", p).unwrap_err(), "l.svg holds a script link (`javascript:alert(1)` on <set>): a layout is a drawing, with no script");
    // The root's own handler, the classic one.
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"x()\"/>", p).unwrap_err(), "l.svg holds an event handler (`onload` on <svg>): a layout is a drawing, with no script");
    // Foreign content of any other form: an element outside the SVG namespace, an XHTML frame, a
    // `path` of another namespace a marker would otherwise follow.
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><iframe xmlns=\"http://www.w3.org/1999/xhtml\" srcdoc=\"x\"/></svg>", p).unwrap_err(), "l.svg holds a <iframe> element of the namespace `http://www.w3.org/1999/xhtml`: a layout is a drawing in the SVG namespace, with no foreign content");
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:h=\"http://www.w3.org/1999/xhtml\"><h:object data=\"x\"/></svg>", p).unwrap_err(), "l.svg holds a <object> element of the namespace `http://www.w3.org/1999/xhtml`: a layout is a drawing in the SVG namespace, with no foreign content");
    assert_eq!(parse("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"urn:not-svg\"><x:path id=\"loop\"/></svg>", p).unwrap_err(), "l.svg holds a <path> element of the namespace `urn:not-svg`: a layout is a drawing in the SVG namespace, with no foreign content");
    assert_eq!(parse("<s:svg xmlns:s=\"http://www.w3.org/2000/svg\"><rect/></s:svg>", p).unwrap_err(), "l.svg holds a <rect> element of the namespace `none`: a layout is a drawing in the SVG namespace, with no foreign content");
    // A document type, as drawing tools export it, is read.
    assert!(parse("<?xml version=\"1.0\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"x\"/></svg>", p).unwrap().element("x") == Some("rect"));
}

/// The example with a keyed text series in its log and an object reading it.
fn with_modes() -> Example {
    let mut ex = example();
    ex.project.push_str(MODES_RULE);
    ex.binding = ex.binding.replacen("\n[[dashboard]]", "\n[[object]]\nid = \"mode\"\nlabel = \"Mode\"\nvalue = { source = \"sorter\", series = \"mode\" }\n\n[[dashboard]]", 1);
    ex
}

#[test]
fn the_refusals_of_a_kind_of_series_a_panel_and_a_number_name_their_place() {
    let cases: Vec<(Box<dyn Fn(&str) -> String>, &str)> = vec![
        (Box::new(|b| b.replace("value = { source = \"plc\", series = \"p1\" }", "value = { source = \"sorter\", series = \"mode\" }")), "object `parcel-1`: a marker needs a numeric series, and `mode` is text"),
        (Box::new(|b| format!("{b}\n[[dashboard.panel]]\nkind = \"bars\"\nobject = \"mode\"\n")), "dashboard `Overview`, panel[8] (bars): `mode` reads `mode`, which is text, and the panel charts numbers"),
        (Box::new(|b| format!("{b}\n[[dashboard.panel]]\nkind = \"cells\"\ntable = \"nothere\"\nkey = \"outlet\"\nvalue = \"travel\"\nspread = 5\n")), "dashboard `Overview`, panel[8] (cells): `nothere` is no table of the project"),
        (Box::new(|b| format!("{b}\n[[dashboard.panel]]\nkind = \"value\"\n")), "dashboard `Overview`, panel[8] (value): missing field `object`"),
        (Box::new(|b| b.replacen("show = \"level\"", "show = \"level\"\nrange = [0, inf]", 1)), "object `outlet-1`: the range's high is not a finite number"),
        (Box::new(|b| format!("{b}\n[[dashboard.panel]]\nkind = \"bars\"\nobject = \"outlet-1\"\ntarget = nan\n")), "dashboard `Overview`, panel[8] (bars): `target` is not a finite number"),
        (Box::new(|b| b.replacen("{ equals = 0, state = \"warn\", word = \"stopped\" }", "{ equals = nan, state = \"warn\", word = \"stopped\" }", 1)), "object `infeed`, states[0]: `equals` is not a finite number"),
        // A key of a text series: portal frames numbers alone and keeps no text per key, so it
        // would never light up; for an object's value and for a rule alike.
        (Box::new(|b| b.replace("value = { source = \"sorter\", series = \"mode\" }", "value = { source = \"sorter\", series = \"mode\", key = \"A1\" }")), "object `mode`, value: a `key` reads one key of a numeric series, and `mode` is text"),
        (Box::new(|b| b.replacen("{ equals = 0, state = \"warn\", word = \"stopped\" }", "{ equals = \"jam\", source = \"sorter\", series = \"mode\", key = \"A1\", state = \"warn\", word = \"stopped\" }", 1)), "object `infeed`, states[0]: a `key` reads one key of a numeric series, and `mode` is text"),
    ];
    for (f, expected) in cases {
        let mut ex = with_modes();
        let changed = f(&ex.binding);
        assert_ne!(changed, ex.binding, "the change applies: {expected}");
        ex.binding = changed;
        assert_eq!(binding_refusal(&ex), expected);
    }
}

#[test]
fn every_key_reads_back_as_written_and_every_default_fills() {
    let mut ex = with_modes();
    ex.binding = KEYS.to_string();
    let dir = tempfile::tempdir().unwrap();
    let cfg = load(dir.path(), &ex).unwrap();
    let b = &cfg.sessions[0].binding;
    use veredas_supervisao::binding::{Column, Rule, SeriesBind};
    let bind = |source: &str, series: &str, key: Option<&str>| SeriesBind { source: source.into(), series: series.into(), key: key.map(String::from) };
    // An object: its key, its range, how it shows, its label defaulting to its id; its rules each
    // compared as written, a word defaulting to its state's name.
    let a = b.object("a").unwrap();
    assert_eq!((a.label.as_str(), &a.value, a.range, a.show), ("a", &bind("plc", "fill1", Some("A1")), (10.0, 20.0), Show::Value));
    assert_eq!(a.rules, vec![
        Rule { compare: Compare::Above(5.0), class: StateClass::Warn, word: "warn".into(), series: None, ty: None },
        Rule { compare: Compare::AtMost(1.0), class: StateClass::Alarm, word: "low".into(), series: None, ty: None },
    ]);
    // Rules over another series: an int one, and a text one compared exactly as written.
    let o = b.object("b").unwrap();
    assert_eq!(o.label, "Bee");
    assert_eq!(o.rules, vec![
        Rule { compare: Compare::Equals(Comparand::Num(2.0)), class: StateClass::Alarm, word: "fault".into(), series: Some(bind("plc", "infeed", None)), ty: Some(SeriesType::Int) },
        Rule { compare: Compare::Equals(Comparand::Text("Jam".into())), class: StateClass::Near, word: "jam".into(), series: Some(bind("sorter", "mode", None)), ty: Some(SeriesType::Text) },
    ]);
    // A dashboard's columns default to 4; every panel's keys as written, its defaults filled.
    let d = &b.dashboards[0];
    assert_eq!(d.columns, 4);
    let kinds: Vec<&PanelKind> = d.panels.iter().map(|p| &p.kind).collect();
    assert_eq!(d.panels[0].title, "Ay");
    assert_eq!(kinds[0], &PanelKind::Value { object: 0, minutes: 30, target: None });
    assert_eq!(kinds[1], &PanelKind::Trend { object: 0, minutes: 60, limit: Some(Limit::Above(3.0)) });
    assert_eq!(kinds[2], &PanelKind::Bars { object: 0, minutes: 60, bucket_minutes: 5, aggregate: Aggregate::Sum, target: None });
    assert_eq!(kinds[3], &PanelKind::Timeline { objects: vec![0], minutes: 60 });
    assert_eq!(kinds[4], &PanelKind::Table {
        table: "outlets".into(),
        columns: vec![Column { name: "outlet".into(), label: "Outlet".into(), bar: false, decimals: None, rules: vec![Rule { compare: Compare::Above(2.0), class: StateClass::Warn, word: "far".into(), series: None, ty: None }] }],
        sort: None,
        limit: 10,
    });
    // Every aggregate word, its kind.
    for (word, agg) in [("mean", Aggregate::Mean), ("sum", Aggregate::Sum), ("count", Aggregate::Count), ("min", Aggregate::Min), ("max", Aggregate::Max), ("last", Aggregate::Last)] {
        let mut e = with_modes();
        e.binding = KEYS.replace("aggregate = \"sum\"", &format!("aggregate = \"{word}\""));
        let dir = tempfile::tempdir().unwrap();
        let cfg = load(dir.path(), &e).unwrap();
        assert!(matches!(&cfg.sessions[0].binding.dashboards[0].panels[2].kind, PanelKind::Bars { aggregate, .. } if *aggregate == agg), "{word}");
    }
}

/// A binding writing the keys the worked example leaves to their defaults, and the defaults it
/// writes left out.
const KEYS: &str = r#"format = "veredas-binding"
version = 1
title = "Keys"

[[object]]
id = "a"
value = { source = "plc", series = "fill1", key = "A1" }
range = [10, 20]
show = "value"
states = [ { above = 5, state = "warn" }, { at_most = 1, state = "alarm", word = "low" } ]

[[object]]
id = "b"
label = "Bee"
value = { source = "plc", series = "speed" }
states = [ { equals = 2, series = "infeed", state = "alarm", word = "fault" }, { equals = "Jam", source = "sorter", series = "mode", state = "near", word = "jam" } ]

[[dashboard]]
name = "Keys"

[[dashboard.panel]]
kind = "value"
object = "a"
title = "Ay"

[[dashboard.panel]]
kind = "trend"
object = "a"
limit = { above = 3 }

[[dashboard.panel]]
kind = "bars"
object = "a"
aggregate = "sum"

[[dashboard.panel]]
kind = "timeline"
objects = ["a"]

[[dashboard.panel]]
kind = "table"
table = "outlets"
limit = 10
columns = [ { name = "outlet", label = "Outlet", states = [ { above = 2, state = "warn", word = "far" } ] } ]
"#;

/// A keyed text series for the example's log: a mode per item.
const MODES_RULE: &str = r#"
[[rule]]
name = "modes"
pattern = 'MODE (?P<mode>\w+) at (?P<item>\w+)'
scope = "sorter"
key = "item"
"#;
