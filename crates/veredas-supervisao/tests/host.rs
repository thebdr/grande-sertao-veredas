//! C-049: the host. The sessions of a supervisão file started through portal and served side by
//! side on one address, each under its name with its own portal router nested unchanged; their
//! data kept apart; the page and its bundle served; the executable running a file until stdin
//! closes. Against two benches, over real sockets.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use veredas_maquinar::bench::Bench;
use veredas_supervisao::state::{Comparand, Compare};
use veredas_supervisao::{Host, HostError, config, panel, web};

fn objects(addr: SocketAddr, name: &str) -> Value {
    let (status, v) = get(addr, &format!("/s/{name}/objects.json"));
    assert_eq!(status, 200);
    v
}

/// The routes a router's source declares, `(method, path)` in order, a path parameter read as `0`.
fn declared_routes(file: &str) -> Vec<(String, String)> {
    routes_in(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap(), file)
}

/// The host's routes as its source declares them, split where it does: those at the root
/// (declared before its loop over the sessions) and each session's own (declared in it).
fn host_routes() -> (Vec<(String, String)>, Vec<(String, String)>) {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs")).unwrap();
    let at = text.find("for app in apps").expect("the host's loop over its sessions");
    (routes_in(&text[..at], "src/host.rs"), routes_in(&text[at..], "src/host.rs"))
}

/// Every key `now_ms` or `arrival_now_ms` (the two clocks' now, which move between two answers)
/// taken out, however deep.
fn without_clocks(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("now_ms");
            m.remove("arrival_now_ms");
            m.values_mut().for_each(without_clocks);
        }
        Value::Array(a) => a.iter_mut().for_each(without_clocks),
        _ => {}
    }
}

/// A route's whole answer, to be compared with another's: its status and its body read as JSON
/// (as text when it is none), a stream by its head and its first event (its name and data read
/// as JSON), the clocks taken out.
fn whole_answer(addr: SocketAddr, method: &str, path: &str) -> (u16, Value) {
    if method == "GET" && path.ends_with("/events") {
        let mut sse = Sse::open(addr, path);
        let first = sse.next(Duration::from_secs(10)).map(|(n, d)| json!([n, serde_json::from_str::<Value>(&d).unwrap_or(Value::String(d))]));
        let mut v = json!({ "type": sse.headers.contains("content-type: text/event-stream"), "first": first });
        without_clocks(&mut v);
        return (sse.status, v);
    }
    let (status, _, body) = http(addr, method, path, (method == "POST").then_some("{}"));
    let mut v = serde_json::from_str(&body).unwrap_or(Value::String(body));
    without_clocks(&mut v);
    (status, v)
}

fn routes_in(text: &str, file: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(".route(\"") {
        let after = &rest[i + ".route(\"".len()..];
        let end = after.find('"').unwrap();
        let tail = after[end + 1..].trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        let method = ["get", "post", "put", "delete", "patch"].into_iter().find(|m| tail.starts_with(&format!("{m}("))).unwrap_or_else(|| panic!("{file}: a route's method: {}", &tail[..tail.len().min(40)]));
        let mut path = String::new();
        let mut inside = false;
        for c in after[..end].chars() {
            match c {
                '{' => inside = true,
                '}' => {
                    inside = false;
                    path.push('0');
                }
                c if !inside => path.push(c),
                _ => {}
            }
        }
        out.push((method.to_ascii_uppercase(), path));
        rest = &after[end..];
    }
    out
}

/// A route's status and, unless it is a stream, its body read as JSON: a stream (an object feed,
/// portal's events) answered by its head alone, a post sent an empty object.
fn answered(addr: SocketAddr, method: &str, path: &str) -> (u16, Option<Value>) {
    if method == "GET" && (path.ends_with("/events") || path.ends_with("/objects")) {
        let sse = Sse::open(addr, path);
        if sse.status != 404 {
            return (sse.status, None);
        }
    }
    let (status, _, body) = http(addr, method, path, (method == "POST").then_some("{}"));
    (status, serde_json::from_str(&body).ok())
}

#[test]
fn two_sessions_are_served_side_by_side_each_under_its_name_with_its_portal() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    b.server("plc").unwrap().set("fill1", 65.0);
    let path = two_plants(dir.path(), &a, &b);
    // An object reading one key, so the view's `key` carries a value; its rules and a text
    // object's give the view every comparison a rule takes that the example leaves out (above,
    // at most, a text's equals), its project a text series for it.
    let binding = dir.path().join("loop/binding.toml");
    let text = std::fs::read_to_string(&binding).unwrap();
    std::fs::write(&binding, text.replacen("\n[[dashboard]]", "\n[[object]]\nid = \"keyed\"\nlabel = \"Keyed\"\nvalue = { source = \"plc\", series = \"fill1\", key = \"A1\" }\nstates = [ { above = 90.5, state = \"near\" }, { at_most = 10, state = \"warn\", word = \"low\" } ]\n\n[[object]]\nid = \"note\"\nlabel = \"Note\"\nvalue = { source = \"sorter\", series = \"word\" }\nstates = [ { equals = \"jam\", state = \"alarm\" } ]\n\n[[dashboard]]", 1)).unwrap();
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap();
    std::fs::write(&project, format!("{text}\n[[rule]]\nname = \"notes\"\npattern = 'NOTE (?P<word>\\w+)'\nscope = \"sorter\"\ntypes = {{ word = \"text\" }}\n")).unwrap();
    let want = config::load_file(&path).unwrap();
    let cfg = config::load_file(&path).unwrap();
    // Loop's log there from the start (hall's stays absent), so its sorts are read as they come.
    std::fs::write(dir.path().join("loop/sorts.log"), "").unwrap();
    let host = Host::start(cfg).unwrap();
    let addr = host.addr();
    assert_ne!(addr.port(), 0);
    // Hall's switch armed, loop's not: every status either session serves tells them apart.
    host.session("hall").unwrap().set_writes_armed(true);

    // The page and its bundle, compiled in.
    let (status, headers, body) = http(addr, "GET", "/", None);
    assert_eq!((status, body.as_str()), (200, web::INDEX_HTML));
    assert!(headers.contains("content-type: text/html; charset=utf-8"), "{headers}");
    for page in ["/s/loop/", "/s/loop", "/s/hall/", "/s/hall"] {
        let (status, headers, body) = http(addr, "GET", page, None);
        assert_eq!((status, body.as_str()), (200, web::INDEX_HTML), "{page}");
        assert!(headers.contains("content-type: text/html; charset=utf-8"), "{page}: {headers}");
    }
    let (status, headers, body) = http(addr, "GET", "/assets/app.js", None);
    assert_eq!((status, body.as_str()), (200, web::APP_JS));
    assert!(headers.contains("content-type: text/javascript; charset=utf-8"));
    let (status, headers, body) = http(addr, "GET", "/assets/app.css", None);
    assert_eq!((status, body.as_str()), (200, web::APP_CSS));
    assert!(headers.contains("content-type: text/css; charset=utf-8"));

    // The sessions, in the file's order.
    assert_eq!(get(addr, "/api/sessions"), (200, json!({ "sessions": [
        { "name": "loop", "title": "The loop", "path": "/s/loop/" },
        { "name": "hall", "title": "The hall", "path": "/s/hall/" },
    ] })));
    // Nothing else is served, and an unknown session is no session.
    for p in ["/s/nope/", "/s/nope/binding", "/api/nope", "/s/loop/nope", "/s/loop/portal/api/nope"] {
        assert_eq!(get(addr, p), (404, json!({ "error": format!("nothing is served at {p}") })), "{p}");
    }
    // Every route the routers declare, read from their own source so none is left out: each of
    // portal's answers under each session's `/portal` and nowhere else (not at the root, not under
    // a `/portal` of no session, not under a session without its `/portal`); each of a session's
    // own answers under the session and nowhere else.
    // Read as literal `.route("…")` declarations: every other way a router takes a route is absent
    // from both sources, and every `.route(` but the host's two per-session page routes (checked by
    // their paths above) names its path as a literal.
    for (file, computed, nests) in [("src/host.rs", vec![".route(&base, get(index))", ".route(&format!(\"{base}/\"), get(index))"], vec![".nest(\"/portal\", ", ".nest(&base, session)"]), ("../veredas-portal/src/serve.rs", vec![], vec![])] {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap();
        for other in ["route_service(", "nest_service(", ".merge(", "fallback_service("] {
            assert!(!text.contains(other), "{file} takes a route by `{other}`, which the lists here do not read");
        }
        let at = |pat: &str| text.match_indices(pat).map(|(i, _)| text[i..].split_once(')').map(|(h, _)| format!("{h})")).unwrap()).collect::<Vec<_>>();
        let not_literal: Vec<String> = at(".route(").into_iter().filter(|r| !r.starts_with(".route(\"")).collect();
        assert_eq!(not_literal, computed.iter().map(|c| c.split_once(')').map(|(h, _)| format!("{h})")).unwrap()).collect::<Vec<_>>(), "{file}");
        assert_eq!(text.matches(".nest(").count(), nests.len(), "{file}");
        assert!(nests.iter().all(|n| text.contains(n)), "{file}");
    }
    let portal = declared_routes("../veredas-portal/src/serve.rs");
    assert!(["/events", "/api/session", "/api/opc/write"].iter().all(|p| portal.iter().any(|(_, q)| q == p)) && portal.iter().any(|(m, _)| m == "POST"), "{portal:?}");
    let (root, own) = host_routes();
    assert_eq!(root.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(), ["/", "/assets/app.js", "/assets/app.css", "/api/sessions"]);
    assert!(["/binding", "/objects", "/dashboard/0"].iter().all(|p| own.iter().any(|(_, q)| q == p)), "{own:?}");
    // The root's own routes (its page at `/` aside, which every session serves as its page too)
    // nowhere but at the root.
    for (m, p) in root.iter().filter(|(_, p)| p != "/") {
        assert_ne!(answered(addr, m, p).0, 404, "{m} {p}");
        for at in [format!("/s/loop{p}"), format!("/portal{p}"), format!("/s/loop/portal{p}"), format!("/s/nope{p}")] {
            assert_eq!(answered(addr, m, &at), (404, Some(json!({ "error": format!("nothing is served at {at}") }))), "{m} {at}");
        }
    }
    for (m, p) in &portal {
        for name in ["loop", "hall"] {
            let at = format!("/s/{name}/portal{p}");
            assert_ne!(answered(addr, m, &at).0, 404, "{m} {at}");
        }
        for at in [p.clone(), format!("/portal{p}"), format!("/s/loop{p}"), format!("/s/hall{p}"), format!("/s/nope/portal{p}")] {
            assert_eq!(answered(addr, m, &at), (404, Some(json!({ "error": format!("nothing is served at {at}") }))), "{m} {at}");
        }
    }
    for (m, p) in &own {
        let at = format!("/s/loop{p}");
        assert_ne!(answered(addr, m, &at).0, 404, "{m} {at}");
        for at in [p.clone(), format!("/portal{p}"), format!("/s/loop/portal{p}"), format!("/s/nope{p}")] {
            assert_eq!(answered(addr, m, &at), (404, Some(json!({ "error": format!("nothing is served at {at}") }))), "{m} {at}");
        }
    }

    // Each session's binding, layout and portal are its own.
    let (status, v) = get(addr, "/s/loop/binding");
    assert_eq!(status, 200);
    assert_eq!((v["name"].clone(), v["title"].clone(), v["layout"].clone(), v["headline"].clone()), (json!("loop"), json!("The loop"), json!("layout.svg"), json!(["speed", "read-rate"])));
    // Every object of each session's view, every field the page reads, as its binding holds it (the
    // rules by their state, word and comparison as the test writes it; outlet 3's spelled out in
    // full below). The comparisons matched in full (one the reader adds fails to compile here), and
    // every one of them met in a view.
    let every_compare = ["equals a number", "equals a text", "above", "below", "at least", "at most"];
    let mut compares_seen = std::collections::BTreeSet::new();
    for (session, at) in [("loop", 0), ("hall", 1)] {
        let (_, view) = get(addr, &format!("/s/{session}/binding"));
        let objects = &want.sessions[at].binding.objects;
        let got = view["objects"].as_array().unwrap();
        assert_eq!(got.len(), objects.len(), "{session}");
        for (o, g) in objects.iter().zip(got) {
            let expected = json!({
                "id": o.id, "label": o.label, "unit": o.unit, "show": o.show.name(), "range": [o.range.0, o.range.1], "path": o.path,
                "source": o.value.source, "series": o.value.series, "key": o.value.key, "normal": o.normal,
                "rules": o.rules.iter().map(|r| json!({ "state": r.class.name(), "word": r.word })).collect::<Vec<_>>(),
                "actions": o.actions.iter().map(|a| json!({ "label": a.label, "source": a.source, "tag": a.tag, "value": a.value })).collect::<Vec<_>>(),
            });
            let mut g = g.clone();
            assert_eq!(g["rules"].as_array().map(Vec::len), Some(o.rules.len()), "{session} {}", o.id);
            for (rule, r) in o.rules.iter().zip(g["rules"].as_array_mut().unwrap()) {
                let n = |x: f64| {
                    let t = veredas_supervisao::state::format_number(x, None);
                    if o.unit.is_empty() { t } else { format!("{t} {}", o.unit) }
                };
                let (kind, when) = match &rule.compare {
                    Compare::Equals(Comparand::Num(x)) => ("equals a number", format!("equals {}", n(*x))),
                    Compare::Equals(Comparand::Text(x)) => ("equals a text", format!("is {x}")),
                    Compare::Above(x) => ("above", format!("above {}", n(*x))),
                    Compare::Below(x) => ("below", format!("below {}", n(*x))),
                    Compare::AtLeast(x) => ("at least", format!("at least {}", n(*x))),
                    Compare::AtMost(x) => ("at most", format!("at most {}", n(*x))),
                };
                compares_seen.insert(kind);
                assert_eq!(r["when"], json!(when), "{session} {}: {r}", o.id);
                r.as_object_mut().unwrap().remove("when");
            }
            assert_eq!(g, expected, "{session} {}", o.id);
        }
    }
    assert_eq!(compares_seen.into_iter().collect::<Vec<_>>(), { let mut e = every_compare.to_vec(); e.sort(); e }, "every comparison a rule takes, in a view");
    assert_eq!(v["objects"].as_array().unwrap().len(), 10);
    // The class has values to see: a marker's path and an object's key among them.
    assert!(v["objects"].as_array().unwrap().iter().any(|o| o["path"] == json!("loop")) && v["objects"].as_array().unwrap().iter().any(|o| o["key"] == json!("A1")), "{v}");
    assert_eq!(v["objects"][5], json!({
        "id": "outlet-3", "label": "Outlet 3", "unit": "%", "show": "level", "range": [0.0, 100.0], "path": null,
        "source": "plc", "series": "fill3", "key": null, "normal": "ok",
        "rules": [ { "state": "warn", "word": "full", "when": "at least 100 %" }, { "state": "near", "word": "near full", "when": "at least 80 %" } ],
        "actions": [ { "label": "Release", "source": "plc", "tag": format!("ns={};s=release3", a.server("plc").unwrap().namespace), "value": "1" } ],
    }));
    assert_eq!(v["dashboards"], json!([{ "name": "Overview", "columns": 4, "panels": [
        { "kind": "value", "title": "Read rate", "span": 1 }, { "kind": "trend", "title": "Read rate", "span": 2 },
        { "kind": "bars", "title": "Loop speed", "span": 1 }, { "kind": "timeline", "title": "State over time", "span": 1 },
        { "kind": "grid", "title": "States", "span": 1 }, { "kind": "levels", "title": "Levels", "span": 1 },
        { "kind": "table", "title": "outlets", "span": 1 }, { "kind": "cells", "title": "sorts", "span": 1 },
    ] }]));
    let (_, v) = get(addr, "/s/hall/binding");
    assert_eq!((v["name"].clone(), v["title"].clone(), v["layout"].clone(), v["objects"].as_array().unwrap().len()), (json!("hall"), json!("The hall"), Value::Null, 6));
    let (status, headers, body) = http(addr, "GET", "/s/loop/layout.svg", None);
    assert_eq!((status, body.replace("\r\n", "\n")), (200, example().svg));
    assert!(headers.contains("content-type: image/svg+xml"), "{headers}");
    assert_eq!(get(addr, "/s/hall/layout.svg"), (404, json!({ "error": "session `hall` has no layout" })));
    let (status, s) = get(addr, "/s/loop/portal/api/session");
    assert_eq!((status, s["session_name"].clone(), s["running"].clone(), s["writes_armed"].clone()), (200, json!("the loop"), json!(true), json!(false)));
    let (_, s) = get(addr, "/s/hall/portal/api/session");
    assert_eq!((s["session_name"].clone(), s["writes_armed"].clone()), (json!("the hall"), json!(true)));
    // Portal's own live stream, nested as it is: its hello names the session it belongs to.
    for (name, title) in [("loop", "the loop"), ("hall", "the hall")] {
        let mut sse = Sse::open(addr, &format!("/s/{name}/portal/events"));
        assert_eq!(sse.status, 200);
        assert!(sse.headers.contains("content-type: text/event-stream"), "{}", sse.headers);
        let (event, data) = sse.next(Duration::from_secs(10)).unwrap();
        let hello: Value = serde_json::from_str(&data).unwrap();
        assert_eq!((event.as_str(), hello["session_name"].clone(), hello["running"].clone()), ("hello", json!(title), json!(true)));
    }
    // Portal's router nested unchanged: portal's own router over each session's live state, served
    // on a listener of this test, answers every route portal declares as the session's nested one
    // does (status and body, a stream's head and first event, the clocks' now aside; asked again
    // while a value moves between the two answers), its posts among them.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    for name in ["loop", "hall"] {
        let live = host.session(name).unwrap().live();
        let own_portal = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let at = l.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(l, veredas_portal::serve::router(live)).await.unwrap() });
            at
        });
        for (m, p) in &portal {
            let nested = format!("/s/{name}/portal{p}");
            let mut last = None;
            let same = (0..20).any(|_| {
                let pair = (whole_answer(addr, m, &nested), whole_answer(own_portal, m, p));
                if pair.0 == pair.1 {
                    return true;
                }
                last = Some(pair);
                std::thread::sleep(Duration::from_millis(150));
                false
            });
            assert!(same, "{m} {nested} answers otherwise than portal's own: {last:?}");
        }
    }
    // Portal's own routers let go of the sessions' state before the sessions stop.
    drop(rt);

    // The plants' values stay in their own sessions, on the snapshot routes and on each session's
    // own stream: each stream opens with its session's snapshot and then carries its own plant's
    // changes alone.
    assert!(wait(Duration::from_secs(20), || object(&objects(addr, "loop"), "outlet-1")["value"] == json!(35.0) && object(&objects(addr, "hall"), "outlet-1")["value"] == json!(65.0)));
    let mut streams = Vec::new();
    for (name, fill) in [("loop", 35.0), ("hall", 65.0)] {
        let mut sse = Sse::open(addr, &format!("/s/{name}/objects"));
        assert_eq!(sse.status, 200);
        assert!(sse.headers.contains("content-type: text/event-stream"), "{}", sse.headers);
        let (event, data) = sse.next(Duration::from_secs(10)).unwrap();
        let first: Value = serde_json::from_str(&data).unwrap();
        assert_eq!((event.as_str(), object(&first, "outlet-1")["value"].clone()), ("snapshot", json!(fill)), "{name}");
        assert_eq!(first["objects"].as_array().unwrap().len(), if name == "loop" { 10 } else { 6 });
        assert_eq!(first["session"]["writes_armed"], json!(name == "hall"), "{name}'s snapshot carries its own switch");
        assert_eq!(objects(addr, name)["session"]["writes_armed"], json!(name == "hall"), "{name}'s objects.json carries its own switch");
        streams.push(sse);
    }
    let fill_of = |v: &Value| v["objects"].as_array().and_then(|os| os.iter().find(|o| o["id"] == "outlet-1")).map(|o| o["value"].clone());
    a.server("plc").unwrap().set("fill1", 40.0);
    assert!(streams[0].until(Duration::from_secs(20), |n, v| n == "objects" && fill_of(v) == Some(json!(40.0))).is_some(), "loop's change on loop's stream");
    b.server("plc").unwrap().set("fill1", 70.0);
    let hall_events = streams[1].until(Duration::from_secs(20), |n, v| n == "objects" && fill_of(v) == Some(json!(70.0))).expect("hall's change on hall's stream");
    assert!(hall_events.iter().all(|(_, v)| fill_of(v) != Some(json!(40.0))), "loop's change never on hall's stream: {hall_events:?}");
    // And the other way: loop's stream, read on for two seconds and a half past hall's change
    // (a leak would have been sent to it before hall's own stream delivered the change), never
    // carries it.
    let reading = std::time::Instant::now();
    let mut loop_after: Vec<(String, Value)> = Vec::new();
    while reading.elapsed() < Duration::from_millis(2500) {
        if let Some((n, d)) = streams[0].next(Duration::from_millis(2500).saturating_sub(reading.elapsed())) {
            loop_after.push((n, serde_json::from_str(&d).unwrap_or(Value::Null)));
        }
    }
    assert!(loop_after.iter().any(|(n, _)| n == "session"), "loop's stream read on: {loop_after:?}");
    assert!(loop_after.iter().all(|(_, v)| fill_of(v) != Some(json!(70.0))), "hall's change never on loop's stream: {loop_after:?}");
    // Hall's stream read on likewise. Every event the feed sends (its names read from its own
    // source, so none is left out) told apart between the sessions: a snapshot and an objects
    // event by a plant's value, a status by the switch (hall's armed, loop's not).
    let reading = std::time::Instant::now();
    let mut hall_after: Vec<(String, Value)> = Vec::new();
    while reading.elapsed() < Duration::from_millis(2500) {
        if let Some((n, d)) = streams[1].next(Duration::from_millis(2500).saturating_sub(reading.elapsed())) {
            hall_after.push((n, serde_json::from_str(&d).unwrap_or(Value::Null)));
        }
    }
    let feed = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/feed.rs")).unwrap();
    // Read as literal names: every event sent or opened with names it as one.
    assert_eq!(feed.matches(".send(").count(), feed.matches(".send((\"").count(), "an event sent under a name no literal gives");
    assert_eq!(feed.matches("once(").count(), feed.matches("once((\"").count(), "an event opened under a name no literal gives");
    let mut sent: Vec<String> = feed.match_indices("((\"").map(|(i, _)| feed[i + 3..].split('"').next().unwrap().to_string()).collect();
    sent.sort();
    sent.dedup();
    assert_eq!(sent, ["objects", "session", "snapshot"], "the events the feed sends, each checked here");
    for (session, after, armed) in [("loop", &loop_after, false), ("hall", &hall_after, true)] {
        let statuses: Vec<&Value> = after.iter().filter(|(n, _)| n == "session").map(|(_, v)| v).collect();
        assert!(!statuses.is_empty() && statuses.iter().all(|s| s["writes_armed"] == json!(armed)), "{session}'s statuses carry its own switch: {statuses:?}");
        assert!(after.iter().filter(|(n, _)| n == "snapshot").all(|(_, v)| v["session"]["writes_armed"] == json!(armed)), "{session}");
    }
    assert_eq!((object(&objects(addr, "loop"), "outlet-1")["value"].clone(), object(&objects(addr, "hall"), "outlet-1")["value"].clone()), (json!(40.0), json!(70.0)));
    assert!(objects(addr, "hall")["objects"].as_array().unwrap().iter().all(|o| !o["id"].as_str().unwrap().starts_with("parcel")));
    let (_, s) = get(addr, &format!("/s/hall/portal/api/opc/read?source=plc&tag={}", enc(&format!("ns={};s=fill1", b.server("plc").unwrap().namespace))));
    assert_eq!(s["value"], json!("70"));

    // The dashboards are computed per session, each from its own plant and its own store: the
    // plants set apart on every series a panel reads (the speed and the infeed on hall's plant, the
    // sorts in loop's log alone, the fills already), every panel of the dashboard (one of every kind
    // the reader knows, from its own refusal) differs between the two over the same window; the
    // levels' outlet 1 spelled out (0.4 and 0.7). One that is not there is named.
    b.server("plc").unwrap().set("speed", 2.5);
    b.server("plc").unwrap().set("infeed", 2);
    let sorts_log = dir.path().join("loop/sorts.log");
    append(&sorts_log, &[1, 2, 3].map(|o| format!("{} SORT outlet={o} travel={}.0 rate=9{o}.5", stamp(store_now()), 10 + o)));
    let loop_live = host.session("loop").unwrap().live();
    let stored = || loop_live.store.query_rows("sorts", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || stored() == 3), "loop's sorts stored: {}", stored());
    assert!(wait(Duration::from_secs(20), || { let h = objects(addr, "hall"); object(&h, "speed")["value"].as_f64() == Some(2.5) && object(&h, "infeed")["value"].as_f64() == Some(2.0) }), "hall's plant moved: speed {} infeed {}", object(&objects(addr, "hall"), "speed"), object(&objects(addr, "hall"), "infeed"));
    let known: std::collections::BTreeSet<String> = {
        let d = tempfile::tempdir().unwrap();
        let mut ex = example();
        ex.binding = ex.binding.replacen("kind = \"value\"", "kind = \"nope\"", 1);
        let err = config::load_file(&write_example(d.path(), &ex)).err().expect("an unknown kind refused").to_string();
        let at = err.find("is none of ").unwrap_or_else(|| panic!("{err}"));
        err[at..].split('`').skip(1).step_by(2).map(String::from).collect()
    };
    let to = store_now() + 1_000;
    let (status, ld) = get(addr, &format!("/s/loop/dashboard/0?to={to}"));
    let (status_h, hd) = get(addr, &format!("/s/hall/dashboard/0?to={to}"));
    assert_eq!((status, status_h, ld["name"].clone(), hd["name"].clone()), (200, 200, json!("Overview"), json!("Overview")));
    let kinds: Vec<&str> = ld["panels"].as_array().unwrap().iter().map(|p| p["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds.iter().map(|k| k.to_string()).collect::<std::collections::BTreeSet<_>>(), known, "every kind on the dashboard");
    assert_eq!(kinds.len(), known.len());
    for (i, kind) in kinds.iter().enumerate() {
        let (l, h) = (&ld["panels"][i], &hd["panels"][i]);
        assert_eq!((h["kind"].as_str(), l["error"].clone(), h["error"].clone()), (Some(*kind), Value::Null, Value::Null), "panel {i}");
        assert_ne!(l["data"], h["data"], "panel {i} ({kind}) read the same in both sessions: {}", l["data"]);
    }
    let level_of = |d: &Value| d["panels"][5]["data"]["rows"].as_array().and_then(|rs| rs.iter().find(|r| r["id"] == "outlet-1")).map(|r| r["fraction"].clone());
    assert_eq!((level_of(&ld), level_of(&hd)), (Some(json!(0.4)), Some(json!(0.7))));
    // Each session's dashboard is the one computed in process over its own store, its own binding
    // and its own feed's snapshot, whole: a panel read from the other session's store (whose every
    // panel differs, above) is told apart. Asked between two equal computations, so the store, the
    // snapshot and the clock's now held still across the request.
    let unix_ms = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    for (session, at) in [("loop", 0), ("hall", 1)] {
        let live = host.session(session).unwrap().live();
        let compute = || panel::dashboard(&live, &want.sessions[at].binding, &host.snapshot(session).unwrap(), 0, to, unix_ms()).unwrap();
        let mut quiet = None;
        for _ in 0..100 {
            let before = compute();
            let (status, served) = get(addr, &format!("/s/{session}/dashboard/0?to={to}"));
            assert_eq!(status, 200);
            if compute() == before {
                quiet = Some((served, before));
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let (served, own) = quiet.unwrap_or_else(|| panic!("{session}'s dashboard never held still"));
        // Through the text a client reads (a float parsed from text may sit a ULP off its value).
        assert_eq!(served, as_served(&own), "{session}'s dashboard, as computed over its own store");
    }
    assert_eq!(get(addr, "/s/loop/dashboard/1"), (404, json!({ "error": "session `loop` has no dashboard 1" })));
    assert_eq!(get(addr, "/s/loop/dashboard/x"), (404, json!({ "error": "`x` is no dashboard of session `loop`" })));

    // A write carrying its content through the nested router lands in its own session alone:
    // posted to armed hall, its plant reads it back; loop's switch, off, refuses the same post and
    // loop's plant keeps its value. (Portal's routes are compared above on one request each; the
    // writes' whole class is C-054's.)
    let release = |session: &str, bench: &Bench| {
        let tag = format!("ns={};s=release3", bench.server("plc").unwrap().namespace);
        get(addr, &format!("/s/{session}/portal/api/opc/read?source=plc&tag={}", enc(&tag))).1["value"].clone()
    };
    let write = |bench: &Bench| json!({ "source": "plc", "tag": format!("ns={};s=release3", bench.server("plc").unwrap().namespace), "value": "1" });
    assert_eq!((release("hall", &b), release("loop", &a)), (json!("0"), json!("0")));
    let (status, v) = post(addr, "/s/hall/portal/api/opc/write", &write(&b));
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(10), || release("hall", &b) == json!("1")), "hall's plant: {}", release("hall", &b));
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &write(&a)), (403, json!({ "error": "writes are turned off for this project" })));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release("loop", &a), json!("0"), "loop's plant kept its value");

    // Stopped with streams open (an object stream, portal's own), every session ends, the stop
    // returns at once, both streams end and the address is let go.
    let mut events = Sse::open(addr, "/s/hall/portal/events");
    assert!(events.next(Duration::from_secs(10)).is_some());
    let stopping = std::time::Instant::now();
    let ends = host.stop();
    // At once: under 3 s (a stop with nothing open takes a quarter of one; blocking tasks get 2 s).
    assert!(stopping.elapsed() < Duration::from_secs(3), "the stop waited on open streams: {:?}", stopping.elapsed());
    assert!(streams[0].ends_within(Duration::from_secs(5)) && events.ends_within(Duration::from_secs(5)), "the open streams end with the stop");
    assert_eq!(ends.iter().map(|(n, e)| (n.as_str(), e.len())).collect::<Vec<_>>(), [("loop", 2), ("hall", 2)]);
    assert!(ends.iter().all(|(_, e)| e.iter().all(|(_, r)| r.is_ok())), "{:?}", ends.iter().map(|(n, e)| (n, e.iter().map(|(i, r)| (i.to_string(), r.is_ok())).collect::<Vec<_>>())).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(5), || TcpStream::connect(addr).is_err()), "the address is free again");
}

#[test]
fn a_session_that_does_not_start_is_named_and_a_bind_taken_is_refused_first() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    // The second session's store is pinned to a file that already stands: its session refuses
    // to open it, so the host does not start.
    let hall = dir.path().join("hall/project.toml");
    let text = std::fs::read_to_string(&hall).unwrap().replace("store = \"ram\"", "store = { disk = \"taken.duckdb\" }");
    std::fs::write(&hall, text).unwrap();
    std::fs::write(dir.path().join("hall/taken.duckdb"), b"not a database").unwrap();
    let cfg = config::load_file(&path).unwrap();
    match Host::start(cfg) {
        Err(HostError::Session { name, reason }) => {
            assert_eq!(name, "hall");
            assert!(reason.contains("taken.duckdb: a database already stands there"), "{reason}");
        }
        other => panic!("{:?}", other.err()),
    }
    // An address taken is refused before any session starts: the same file, whose second
    // session would not start, answers with the address, never with the session.
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("bind = \"127.0.0.1:0\"", &format!("bind = \"{taken}\""));
    std::fs::write(&path, text).unwrap();
    let cfg = config::load_file(&path).unwrap();
    match Host::start(cfg) {
        Err(e @ HostError::Bind { .. }) => assert!(e.to_string().starts_with(&format!("cannot bind {taken}: ")), "{e}"),
        other => panic!("{:?}", other.err()),
    }
    // The executable says the same on stderr and exits 2.
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with(&format!("cannot bind {taken}: ")), "{err}");
    assert!(out.stdout.is_empty(), "nothing served, nothing announced");
    // Freed, the second session's own failure is the executable's answer.
    drop(held);
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("session `hall` did not start: ") && err.contains("taken.duckdb: a database already stands there"), "{err}");
}

#[test]
fn nothing_starts_before_the_address_is_held_nor_under_check() {
    // The first session keeps its store on disk: a session started leaves its file.
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap().replace("store = \"ram\"", "store = { disk = \"loop.duckdb\" }");
    std::fs::write(&project, text).unwrap();
    let store = dir.path().join("loop/loop.duckdb");
    // A check reads, checks and prints the plan, starting nothing.
    let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).arg("--check").stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    // Both sessions planned, in the file's order.
    let plan = String::from_utf8_lossy(&out.stdout).to_string();
    let (at_loop, at_hall) = (plan.find("session `loop` at /s/loop/: \"The loop\""), plan.find("session `hall` at /s/hall/: \"The hall\""));
    assert!(at_loop.is_some() && at_hall.is_some() && at_loop < at_hall, "{plan}");
    assert!(!store.exists(), "a check started a session");
    // An address taken is refused before the first session starts.
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("bind = \"127.0.0.1:0\"", &format!("bind = \"{taken}\""));
    std::fs::write(&path, text).unwrap();
    match Host::start(config::load_file(&path).unwrap()) {
        Err(e @ HostError::Bind { .. }) => assert!(e.to_string().starts_with(&format!("cannot bind {taken}: ")), "{e}"),
        other => panic!("{:?}", other.err()),
    }
    assert!(!store.exists(), "a session started before the address was held");
    drop(held);
}

#[test]
fn the_executable_serves_a_file_until_stdin_closes() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    // The first session keeps its store on disk: a stop closes it (its write-ahead log folded in
    // and gone), a process that ended without stopping its sessions leaves the log behind.
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap().replace("store = \"ram\"", "store = { disk = \"loop.duckdb\" }");
    std::fs::write(&project, text).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    out.read_line(&mut first).unwrap();
    let url = first.trim().strip_prefix("serving on http://").and_then(|r| r.strip_suffix('/')).unwrap_or_else(|| panic!("{first}")).to_string();
    let addr: SocketAddr = url.parse().unwrap();
    let mut second = String::new();
    out.read_line(&mut second).unwrap();
    assert_eq!(second.trim(), format!("session `loop` at http://{addr}/s/loop/"));
    let mut third = String::new();
    out.read_line(&mut third).unwrap();
    assert_eq!(third.trim(), format!("session `hall` at http://{addr}/s/hall/"));
    let mut fourth = String::new();
    out.read_line(&mut fourth).unwrap();
    assert_eq!(fourth.trim(), "running; close stdin to stop");
    assert_eq!(get(addr, "/api/sessions").1["sessions"][1]["name"], json!("hall"));
    let (_, s) = get(addr, "/s/loop/portal/api/session");
    assert_eq!(s["session_name"], json!("the loop"));
    // A line on stdin is not a close, in any encoding (one in Latin-1 among them), nor an empty
    // one (a bare Enter, with or without its carriage return): it keeps serving.
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"hello\n").unwrap();
    stdin.write_all(b"n\xe3o\n").unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.write_all(b"\r\n").unwrap();
    stdin.flush().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(child.try_wait().unwrap().is_none(), "a line stopped it");
    assert_eq!(get(addr, "/api/sessions").0, 200);
    // Closed with a stream open (a browser on the page), it stops at once, the stream ended.
    let mut sse = Sse::open(addr, "/s/loop/objects");
    assert!(sse.next(Duration::from_secs(10)).is_some());
    drop(stdin);
    let closed = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(closed.elapsed() < Duration::from_secs(3), "it waited on an open stream: {:?}", closed.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status}");
    assert!(sse.ends_within(Duration::from_secs(5)), "the open stream ended");
    assert!(TcpStream::connect(addr).is_err(), "nothing serves after the stop");
    // Its sessions stopped, not dropped with the process: the store closed.
    let store = dir.path().join("loop/loop.duckdb");
    assert!(store.exists() && !dir.path().join("loop/loop.duckdb.wal").exists(), "the store was never closed: {:?}", std::fs::read_dir(dir.path().join("loop")).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>());
}
