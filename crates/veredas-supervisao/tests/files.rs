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
    assert_eq!(u.tables.iter().cloned().collect::<Vec<_>>(), ["outlets", "plc_main", "sorts"]);
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
