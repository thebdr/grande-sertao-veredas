//! C-049: the host. The sessions of a supervisão file started through portal and served side by
//! side on one address, each under its name with its own portal router nested unchanged; their
//! data kept apart; the page and its bundle served; the executable running a file until stdin
//! closes. Against two benches, over real sockets.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::browser::Browser;
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
/// (Every answer the host gives, a stream's head among them, a 404 and a 405 too, marked `nosniff`.)
fn answered(addr: SocketAddr, method: &str, path: &str) -> (u16, Option<Value>) {
    if method == "GET" && (path.ends_with("/events") || path.ends_with("/objects")) {
        let sse = Sse::open(addr, path);
        if sse.status != 404 {
            assert!(sse.headers.contains("x-content-type-options: nosniff"), "{method} {path}: {}", sse.headers);
            return (sse.status, None);
        }
    }
    let (status, head, body) = http(addr, method, path, (method == "POST").then_some("{}"));
    assert!(head.contains("x-content-type-options: nosniff"), "{method} {path}: {head}");
    (status, serde_json::from_str(&body).ok())
}

/// An answer's head as the host gives it (a stream's by its head alone), in lower case.
fn head_of(addr: SocketAddr, method: &str, path: &str) -> String {
    if method == "GET" && path.ends_with("/events") {
        return Sse::open(addr, path).headers;
    }
    http(addr, method, path, (method == "POST").then_some("{}")).1
}

/// The names a session's object stream sends, driven through the stream itself rather than read
/// from its source: its snapshot on connect, a fresh snapshot to a client that fell behind (two
/// events sent where it holds one), and every event the feed publishes passed on by its own name;
/// together the names the source reading finds.
#[test]
fn a_stream_sends_a_snapshot_on_connect_and_again_once_it_falls_behind() {
    use tokio_stream::StreamExt;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let got = rt.block_on(async {
        let (tx, rx) = tokio::sync::broadcast::channel::<(&'static str, std::sync::Arc<str>)>(1);
        tx.send(("objects", "first".into())).unwrap();
        tx.send(("session", "second".into())).unwrap();
        let mut s = Box::pin(veredas_supervisao::feed::stream(rx, || "the snapshot".to_string()));
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(tokio::time::timeout(Duration::from_secs(5), s.next()).await.unwrap().unwrap());
        }
        tx.send(("objects", "third".into())).unwrap();
        got.push(tokio::time::timeout(Duration::from_secs(5), s.next()).await.unwrap().unwrap());
        got
    });
    let got: Vec<(&str, &str)> = got.iter().map(|(n, d)| (n.as_str(), d.as_str())).collect();
    assert_eq!(got, [("snapshot", "the snapshot"), ("snapshot", "the snapshot"), ("session", "second"), ("objects", "third")]);
    let feed = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/feed.rs")).unwrap();
    let mut read: Vec<String> = feed.match_indices("((\"").map(|(i, _)| feed[i + 3..].split('"').next().unwrap().to_string()).collect();
    read.sort();
    read.dedup();
    let mut sent: Vec<&str> = got.iter().map(|(n, _)| *n).collect();
    sent.sort();
    sent.dedup();
    assert_eq!(sent, read, "the names the stream sends, the names the source reading finds");
}

#[test]
fn two_sessions_are_served_side_by_side_each_under_its_name_with_its_portal() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    b.server("plc").unwrap().set("fill1", 65.0);
    let path = two_plants(dir.path(), &a, &b);
    // The plants polled once a second, not every 50 ms: a dashboard asked between two equal
    // computations (below) needs the store and the feed (each poll refreshes an object's arrival)
    // to hold still across the request, which a loaded machine's request outlasts at 50 ms.
    for s in ["loop", "hall"] {
        let p = dir.path().join(format!("{s}/project.toml"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("poll_ms = 50"), "{s}");
        std::fs::write(&p, text.replace("poll_ms = 50", "poll_ms = 1000")).unwrap();
    }
    // An object reading one key, so the view's `key` carries a value; its rules and a text
    // object's give the view every comparison a rule takes that the example leaves out (above,
    // at most, a text's equals), a rule over another series with the key it takes and one without,
    // the disabled state and a value shown, its project a text series for it.
    let binding = dir.path().join("loop/binding.toml");
    let text = std::fs::read_to_string(&binding).unwrap();
    std::fs::write(&binding, text.replacen("\n[[dashboard]]", "\n[[object]]\nid = \"keyed\"\nlabel = \"Keyed\"\nvalue = { source = \"plc\", series = \"fill1\", key = \"A1\" }\nunit = \"%\"\nstates = [ { equals = 42, state = \"near\", word = \"forty-two\" }, { below = 3, source = \"plc\", state = \"warn\", word = \"own\" }, { above = 90.5, state = \"near\" }, { at_most = 10, state = \"warn\", word = \"low\" }, { above = 1, source = \"plc\", series = \"fill2\", state = \"warn\", word = \"f2\" }, { above = 0.0625, source = \"plc\", series = \"fill2\", key = \"B7\", state = \"near\", word = \"b7\" }, { below = -5, state = \"disabled\" } ]\n\n[[object]]\nid = \"note\"\nlabel = \"Note\"\nvalue = { source = \"sorter\", series = \"word\" }\nshow = \"value\"\nunit = \"pcs\"\nstates = [ { equals = \"jam\", state = \"alarm\" }, { below = 0.5, source = \"plc\", series = \"speed\", state = \"warn\", word = \"slow\" } ]\n\n[[dashboard]]", 1)).unwrap();
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
    // Every route the routers declare asked with each method it does not declare for its path: a
    // 405, marked `nosniff` (`answered` holds every answer to it).
    let mut asked_405 = 0;
    for (routes, prefix) in [(&root, String::new()), (&own, "/s/loop".to_string()), (&portal, "/s/loop/portal".to_string())] {
        let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (m, p) in routes.iter() {
            by_path.entry(p.as_str()).or_default().push(m.as_str());
        }
        for (p, declared) in by_path {
            for m in ["GET", "POST", "PUT", "DELETE", "PATCH"].into_iter().filter(|m| !declared.contains(m)) {
                let at = if p == "/" && !prefix.is_empty() { format!("{prefix}/") } else { format!("{prefix}{p}") };
                assert_eq!(answered(addr, m, &at).0, 405, "{m} {at}");
                asked_405 += 1;
            }
        }
    }
    assert!(asked_405 >= 20, "the 405s asked: {asked_405}");

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
    let (mut series_seen, mut shows_seen, mut states_seen) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for (session, at) in [("loop", 0), ("hall", 1)] {
        let (_, view) = get(addr, &format!("/s/{session}/binding"));
        let objects = &want.sessions[at].binding.objects;
        // The words the view spells (a way of showing, a rule's state) are the ones the binding file
        // writes, read from the file itself (`state` where it writes no `show`), never from the
        // names the code gives them.
        let written: toml::Table = toml::from_str(&std::fs::read_to_string(dir.path().join(format!("{session}/binding.toml"))).unwrap()).unwrap();
        let written: BTreeMap<String, (String, Vec<String>)> = written["object"].as_array().unwrap().iter().map(|o| {
            let show = o.get("show").and_then(|s| s.as_str()).unwrap_or("state").to_string();
            let states = o.get("states").and_then(|s| s.as_array()).map(|a| a.iter().map(|r| r["state"].as_str().unwrap().to_string()).collect()).unwrap_or_default();
            (o["id"].as_str().unwrap().to_string(), (show, states))
        }).collect();
        let got = view["objects"].as_array().unwrap();
        assert_eq!(got.len(), objects.len(), "{session}");
        for (o, g) in objects.iter().zip(got) {
            let (show, states) = &written[&o.id];
            shows_seen.insert(show.clone());
            states_seen.extend(states.iter().cloned());
            let expected = json!({
                "id": o.id, "label": o.label, "unit": o.unit, "show": show, "range": [o.range.0, o.range.1], "path": o.path,
                "source": o.value.source, "series": o.value.series, "key": o.value.key, "normal": o.normal,
                "rules": o.rules.iter().zip(states).map(|(r, s)| json!({ "state": s, "word": r.word })).collect::<Vec<_>>(),
                "actions": o.actions.iter().map(|a| json!({ "label": a.label, "source": a.source, "tag": a.tag, "value": a.value })).collect::<Vec<_>>(),
            });
            let mut g = g.clone();
            assert_eq!(g["rules"].as_array().map(Vec::len), Some(o.rules.len()), "{session} {}", o.id);
            for (rule, r) in o.rules.iter().zip(g["rules"].as_array_mut().unwrap()) {
                let n = |x: f64| {
                    // Spelled here, not by the code's own `format_number`: every number of the
                    // fixture reads the same as Rust writes it.
                    let t = x.to_string();
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
                // Over another series than the object's own: that series named, its comparison bare.
                series_seen.insert(match &rule.series {
                    Some(s) if *s != o.value && s.key.is_some() => "another series, with its key",
                    Some(s) if *s != o.value => "another series",
                    Some(_) => "its own series, written out",
                    None => "its own series, left to it",
                });
                let when = match &rule.series {
                    Some(s) if *s != o.value => {
                        let bare = when.strip_suffix(&format!(" {}", o.unit)).filter(|_| !o.unit.is_empty()).unwrap_or(&when).to_string();
                        format!("{} · {}{} {bare}", s.source, s.series, s.key.as_deref().map(|k| format!(" · {k}")).unwrap_or_default())
                    }
                    _ => when,
                };
                assert_eq!(r["when"], json!(when), "{session} {}: {r}", o.id);
                r.as_object_mut().unwrap().remove("when");
            }
            assert_eq!(g, expected, "{session} {}", o.id);
        }
    }
    assert_eq!(compares_seen.into_iter().collect::<Vec<_>>(), { let mut e = every_compare.to_vec(); e.sort(); e }, "every comparison a rule takes, in a view");
    assert_eq!(series_seen.into_iter().collect::<Vec<_>>(), ["another series", "another series, with its key", "its own series, left to it", "its own series, written out"], "every series a rule reads, in a view");
    // Every comparison a rule takes (`Compare`, matched in full) and every form of another series
    // (a key of its own, not the object's; the object's key, inherited; no key) met on an object with
    // a unit, so a unit kept or dropped, or a key taken from the object, where the reference says
    // otherwise shows; and a number of four places among them (written in full, never to three).
    let kind = |c: &Compare| match c {
        Compare::Equals(Comparand::Num(_)) => "equals a number",
        Compare::Equals(Comparand::Text(_)) => "equals a text",
        Compare::Above(_) => "above",
        Compare::Below(_) => "below",
        Compare::AtLeast(_) => "at least",
        Compare::AtMost(_) => "at most",
    };
    let bound = &want.sessions[0].binding.objects;
    let with_unit: std::collections::BTreeSet<&str> = bound.iter().filter(|o| !o.unit.is_empty()).flat_map(|o| o.rules.iter().map(|r| kind(&r.compare))).collect();
    assert_eq!(with_unit.len(), every_compare.len(), "every comparison on an object with a unit: {with_unit:?}");
    let forms: std::collections::BTreeSet<&str> = bound
        .iter()
        .filter(|o| !o.unit.is_empty())
        .flat_map(|o| {
            o.rules.iter().filter_map(move |r| {
                r.series.as_ref().filter(|s| **s != o.value).map(|s| match &s.key {
                    None => "no key",
                    Some(k) if Some(k) == o.value.key.as_ref() => "the object's key",
                    Some(_) => "a key of its own",
                })
            })
        })
        .collect();
    assert_eq!(forms.into_iter().collect::<Vec<_>>(), ["a key of its own", "no key", "the object's key"], "every form of another series on an object with a unit");
    assert!(bound.iter().flat_map(|o| &o.rules).any(|r| matches!(r.compare, Compare::Above(x) if x == 0.0625)), "a number of four places, which the screen's three would write otherwise");
    // Every way of showing and every state a rule names, as the reader lists them refusing another,
    // met in a view.
    let reader_list = |from: &str, to: &str| -> std::collections::BTreeSet<String> {
        let d = tempfile::tempdir().unwrap();
        let mut ex = example();
        assert!(ex.binding.contains(from), "{from}");
        ex.binding = ex.binding.replacen(from, to, 1);
        let err = config::load_file(&write_example(d.path(), &ex)).err().expect("refused").to_string();
        let at = err.find("none of ").unwrap_or_else(|| panic!("{err}"));
        err[at..].split('`').skip(1).step_by(2).map(String::from).collect()
    };
    assert_eq!(shows_seen, reader_list("show = \"level\"", "show = \"nope\""), "every way of showing, in a view");
    assert_eq!(states_seen, reader_list("state = \"warn\"", "state = \"nope\""), "every state a rule names, in a view");
    assert_eq!(v["objects"][9]["show"], json!("value"));
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
            // And marked `nosniff` by the host, a stream's head too (portal's own answer is not).
            assert!(head_of(addr, m, &nested).contains("x-content-type-options: nosniff"), "{m} {nested}");
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
    // A check reads, checks and prints the plan, starting nothing: each way the executable takes one
    // (`main.rs`'s arms, the flag after the file and before it).
    let main_rs = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap();
    assert_eq!(main_rs.matches("c == \"--check\"").count(), 2, "the executable's ways of asking for a check, each driven here");
    for args in [vec![path.as_os_str().to_owned(), "--check".into()], vec!["--check".into(), path.as_os_str().to_owned()]] {
        let out = Command::new(env!("CARGO_BIN_EXE_veredas-supervisao")).args(&args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        // Both sessions planned, in the file's order, and nothing served.
        let plan = String::from_utf8_lossy(&out.stdout).to_string();
        let (at_loop, at_hall) = (plan.find("session `loop` at /s/loop/: \"The loop\""), plan.find("session `hall` at /s/hall/: \"The hall\""));
        assert!(at_loop.is_some() && at_hall.is_some() && at_loop < at_hall && !plan.contains("serving on"), "{args:?}: {plan}");
        assert!(!store.exists(), "{args:?}: a check started a session");
    }
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

/// A stop waits for what is still computed: a dashboard over a view that takes seconds to read,
/// asked a second before the stop, holds the session's live state (its store among it) until it
/// ends, and the stop returns only after it, nothing left holding that state and the store closed
/// (no write-ahead log left). The view is read once first, so the test knows the computation
/// outlasts the stop's start by more than the two seconds a bounded wait once gave it.
#[test]
fn a_stop_waits_for_a_dashboard_still_computed() {
    let dir = tempfile::tempdir().unwrap();
    let a = plant(&dir.path().join("bench-a"));
    let b = plant(&dir.path().join("bench-b"));
    let path = two_plants(dir.path(), &a, &b);
    // Read twice in each of two panels (its count, then its page), the store free between them:
    // about six seconds on the desk alone, several times that beside the whole suite.
    let n: u64 = 14_000;
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap().replace("store = \"ram\"", "store = { disk = \"loop.duckdb\" }");
    std::fs::write(&project, format!("{text}\n[[table]]\nname = \"slow\"\nsql = \"SELECT count(*) AS n FROM range({n}) a, range({n}) b WHERE (a.range * 31 + b.range) % 1000003 = 1\"\n")).unwrap();
    let binding = dir.path().join("loop/binding.toml");
    let text = std::fs::read_to_string(&binding).unwrap();
    let slow = text.matches("\n[[dashboard]]").count();
    std::fs::write(&binding, format!("{text}\n[[dashboard]]\nname = \"Slow\"\n\n[[dashboard.panel]]\nkind = \"table\"\ntable = \"slow\"\n\n[[dashboard.panel]]\nkind = \"table\"\ntable = \"slow\"\n")).unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    // Read once whole: how long the dashboard takes to compute here (waited for as long as a
    // loaded machine needs, beyond the twenty seconds the shared reader allows).
    let started = std::time::Instant::now();
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(300))).unwrap();
    // (HTTP/1.0: the body comes whole, never in chunks.)
    s.write_all(format!("GET /s/loop/dashboard/{slow} HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes()).unwrap();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut s, &mut raw).unwrap();
    let takes = started.elapsed();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a header block");
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let whole: Value = serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {head} {body}"));
    assert_eq!((status, whole["panels"][0]["error"].clone()), (200, Value::Null), "{whole}");
    assert!(takes > Duration::from_secs(4), "the view is read in {takes:?} here: too quick to outlast a stop's first two seconds");
    // Asked again, and the host stopped a second later while it is computed.
    let asking = std::thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("GET /s/loop/dashboard/{slow} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = std::io::Read::read_to_end(&mut s, &mut raw);
    });
    std::thread::sleep(Duration::from_secs(1));
    assert!(!asking.is_finished(), "the dashboard was answered within a second");
    let wal = dir.path().join("loop/loop.duckdb.wal");
    assert!(wal.exists(), "a write-ahead log while the session runs");
    // The test keeps no hold of its own on the session's live state, only a way to see whether
    // anything still does.
    let live = std::sync::Arc::downgrade(&host.session("loop").unwrap().live());
    let stopping = std::time::Instant::now();
    host.stop();
    let stopped = stopping.elapsed();
    assert!(live.upgrade().is_none(), "the session's live state still held {stopped:?} into the stop: a computation left running (the dashboard takes {takes:?})");
    let store = dir.path().join("loop/loop.duckdb");
    assert!(store.exists() && !wal.exists(), "the store was never closed: {:?}", std::fs::read_dir(dir.path().join("loop")).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>());
    let _ = asking.join();
}

/// An answer's status and headers, by their names in lower case, read off the wire.
fn headers_of(addr: SocketAddr, path: &str) -> (u16, BTreeMap<String, String>) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: {addr}\r\n\r\n").unwrap();
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let status: u16 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut out = BTreeMap::new();
    loop {
        line.clear();
        r.read_line(&mut line).unwrap();
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            out.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    (status, out)
}

/// Every kind of load and of script a page can be made to do, put straight into the document (past
/// the reader, as nothing the drawing may write could), aimed at `OTHER`: a picture by an SVG image,
/// an HTML image and a style's background; a script by its element, written inline and as a
/// handler; a sheet linked and one written inline importing another; a frame, an object, a font;
/// a fetch, a feed, a WebSocket, a beacon and a module imported. What runs says so in `__ran`.
/// Kept in a page from its first load: every refusal of its content policy, as `[directive, address]`
/// (the address the browser refused, or `inline`).
const WATCH: &str = "window.__refused = []; document.addEventListener('securitypolicyviolation', (e) => window.__refused.push([e.effectiveDirective, e.blockedURI]));";

/// Every kind of load and of script a page can be made to do, each aimed at its own address on the
/// other host (`fire(other)`), with the directive the policy refuses it by and the address it
/// refuses (`inline` for one written in the page): a picture by an SVG image, an HTML image, a
/// style's background and its cursor; a paint, a filter, a mask, a clip path and a marker from
/// another document, each in a `style` attribute as a drawing writes its paint; a script by its
/// element, written inline and as a handler; a sheet linked and one written inline importing
/// another; a frame, an object, a font; a fetch, a feed, a WebSocket, a beacon and a module
/// imported. Run in a page (its `svg.plant`) or in a blank document (an `svg.plant` made in it).
/// What runs says so in `__ran`.
const PAGE_KINDS: &str = r#"(() => {
  const svgNS = "http://www.w3.org/2000/svg";
  window.__ran = [];
  let plant = document.querySelector("svg.plant");
  if (!plant) { plant = document.createElementNS(svgNS, "svg"); plant.setAttribute("class", "plant"); plant.setAttribute("width", "40"); plant.setAttribute("height", "40"); document.body.append(plant); }
  const shape = (name, style) => { const e = document.createElementNS(svgNS, name); e.setAttribute("style", style); if (name === "path") e.setAttribute("d", "M 1 1 L 9 9"); else { e.setAttribute("width", "4"); e.setAttribute("height", "4"); } plant.append(e); };
  const kinds = [
    ["an SVG image", "img-src", "/svg-image.png", (o) => { const e = document.createElementNS(svgNS, "image"); e.setAttribute("href", o + "/svg-image.png"); e.setAttribute("width", "4"); e.setAttribute("height", "4"); plant.append(e); }],
    ["an HTML image", "img-src", "/img.png", (o) => { const e = new Image(); e.src = o + "/img.png"; document.body.append(e); }],
    ["a style's background", "img-src", "/bg.png", (o) => { const e = document.createElement("div"); e.setAttribute("style", `background-image: url(${o}/bg.png); width: 4px; height: 4px`); document.body.append(e); }],
    ["a style's cursor", "img-src", "/cursor.png", (o) => { const e = document.createElement("div"); e.setAttribute("style", `cursor: url(${o}/cursor.png), auto; width: 4px; height: 4px`); document.body.append(e); }],
    ["a paint from another document", "img-src", "/paint.svg", (o) => shape("rect", `fill: url(${o}/paint.svg#p)`)],
    ["a filter from another document", "img-src", "/filter.svg", (o) => shape("rect", `filter: url(${o}/filter.svg#f)`)],
    ["a mask from another document", "img-src", "/mask.svg", (o) => shape("rect", `mask: url(${o}/mask.svg#m)`)],
    ["a clip path from another document", "img-src", "/clip.svg", (o) => shape("rect", `clip-path: url(${o}/clip.svg#c)`)],
    ["a marker from another document", "img-src", "/marker.svg", (o) => shape("path", `marker-start: url(${o}/marker.svg#m); stroke: black`)],
    ["a script element", "script-src-elem", "/s.js", (o) => { const e = document.createElement("script"); e.src = o + "/s.js"; document.head.append(e); }],
    ["a script written inline", "script-src-elem", "inline", () => { const e = document.createElement("script"); e.textContent = "window.__ran.push('inline')"; document.head.append(e); }],
    ["a handler written in markup", "script-src-attr", "inline", () => { const e = document.createElement("button"); e.setAttribute("onclick", "window.__ran.push('handler')"); document.body.append(e); e.click(); }],
    ["a sheet linked", "style-src-elem", "/x.css", (o) => { const e = document.createElement("link"); e.rel = "stylesheet"; e.href = o + "/x.css"; document.head.append(e); }],
    ["a sheet written inline", "style-src-elem", "inline", (o) => { const e = document.createElement("style"); e.textContent = `@import url(${o}/imported.css); body { --probe: 1 }`; document.head.append(e); }],
    ["a frame", "frame-src", "/frame", (o) => { const e = document.createElement("iframe"); e.src = o + "/frame"; document.body.append(e); }],
    ["an object", "object-src", "/object", (o) => { const e = document.createElement("object"); e.data = o + "/object"; document.body.append(e); }],
    ["a font", "font-src", "/f.woff", async (o) => { try { await new FontFace("probe", `url(${o}/f.woff)`).load(); } catch {} }],
    ["a fetch", "connect-src", "/fetch", async (o) => { try { await fetch(o + "/fetch"); } catch {} }],
    ["a feed", "connect-src", "/feed", (o) => { try { new EventSource(o + "/feed"); } catch {} }],
    ["a WebSocket", "connect-src", "/socket", (o) => { try { new WebSocket(o.replace("http:", "ws:") + "/socket"); } catch {} }],
    ["a beacon", "connect-src", "/beacon", (o) => { try { navigator.sendBeacon(o + "/beacon", "x"); } catch {} }],
    ["a module imported", "script-src-elem", "/module.js", async (o) => { try { await import(o + "/module.js"); } catch {} }],
    ["a picture in a filter", "img-src", "/feimage.png", (o) => { const f = document.createElementNS(svgNS, "filter"); f.id = "probe-filter"; const i = document.createElementNS(svgNS, "feImage"); i.setAttribute("href", o + "/feimage.png"); f.append(i); plant.append(f); shape("rect", "filter: url(#probe-filter)"); }],
    ["a use of another document", "img-src", "/use.svg", (o) => { const e = document.createElementNS(svgNS, "use"); e.setAttribute("href", o + "/use.svg#u"); plant.append(e); }],
    ["a paint in a presentation attribute", "img-src", "/attr-paint.svg", (o) => { const e = document.createElementNS(svgNS, "rect"); e.setAttribute("width", "4"); e.setAttribute("height", "4"); e.setAttribute("fill", `url(${o}/attr-paint.svg#p)`); plant.append(e); }],
    ["a video", "media-src", "/video.mp4", (o) => { const e = document.createElement("video"); e.preload = "auto"; e.src = o + "/video.mp4"; document.body.append(e); }],
    ["an audio", "media-src", "/audio.mp3", (o) => { const e = document.createElement("audio"); e.preload = "auto"; e.src = o + "/audio.mp3"; document.body.append(e); }],
    ["a manifest", "manifest-src", "/manifest.json", (o) => { const e = document.createElement("link"); e.rel = "manifest"; e.href = o + "/manifest.json"; document.head.append(e); }],
    ["a prefetch hint", "default-src", "/prefetch", (o) => { const e = document.createElement("link"); e.rel = "prefetch"; e.href = o + "/prefetch"; document.head.append(e); }],
  ];
  window.__probes = Object.fromEntries(kinds.map(([n, , , f]) => [n, f]));
  return kinds.map(([n, d, a]) => [n, d, a]);
})()"#;

/// The kinds a drawing opened alone can be made to do (an SVG document, no body), each aimed at its
/// own address: a picture by an SVG image and by an HTML image in a foreign object; a paint, a
/// filter, a mask, a clip path and a marker from another document in a `style` attribute; a sheet
/// importing another; a script written inline and one linked; a handler; a fetch.
const DRAWING_KINDS: &str = r#"(() => {
  const svgNS = "http://www.w3.org/2000/svg";
  window.__ran = [];
  const root = document.documentElement;
  const shape = (name, style) => { const e = document.createElementNS(svgNS, name); e.setAttribute("style", style); if (name === "path") e.setAttribute("d", "M 1 1 L 9 9"); else { e.setAttribute("width", "4"); e.setAttribute("height", "4"); } root.append(e); };
  const kinds = [
    ["an SVG image", "/drawing-image.png", (o) => { const e = document.createElementNS(svgNS, "image"); e.setAttribute("href", o + "/drawing-image.png"); e.setAttribute("width", "4"); e.setAttribute("height", "4"); root.append(e); }],
    ["an HTML image in a foreign object", "/drawing-img.png", (o) => { const f = document.createElementNS(svgNS, "foreignObject"); f.setAttribute("width", "4"); f.setAttribute("height", "4"); const i = document.createElementNS("http://www.w3.org/1999/xhtml", "img"); i.setAttribute("src", o + "/drawing-img.png"); f.append(i); root.append(f); }],
    ["a paint from another document", "/drawing-paint.svg", (o) => shape("rect", `fill: url(${o}/drawing-paint.svg#p)`)],
    ["a filter from another document", "/drawing-filter.svg", (o) => shape("rect", `filter: url(${o}/drawing-filter.svg#f)`)],
    ["a mask from another document", "/drawing-mask.svg", (o) => shape("rect", `mask: url(${o}/drawing-mask.svg#m)`)],
    ["a clip path from another document", "/drawing-clip.svg", (o) => shape("rect", `clip-path: url(${o}/drawing-clip.svg#c)`)],
    ["a marker from another document", "/drawing-marker.svg", (o) => shape("path", `marker-start: url(${o}/drawing-marker.svg#m); stroke: black`)],
    ["a sheet importing another", "/drawing.css", (o) => { const e = document.createElementNS(svgNS, "style"); e.textContent = `@import url(${o}/drawing.css);`; root.append(e); }],
    ["a script written inline", "inline", () => { const e = document.createElementNS(svgNS, "script"); e.textContent = "window.__ran.push('script')"; root.append(e); }],
    ["a script linked", "/drawing.js", (o) => { const e = document.createElementNS(svgNS, "script"); e.setAttribute("href", o + "/drawing.js"); root.append(e); }],
    ["a handler", "inline", () => { const e = document.createElementNS(svgNS, "rect"); e.setAttribute("onclick", "window.__ran.push('handler')"); root.append(e); e.dispatchEvent(new MouseEvent("click")); }],
    ["a fetch", "/drawing-fetch", async (o) => { try { await fetch(o + "/drawing-fetch"); } catch {} }],
    ["a picture in a filter", "/drawing-feimage.png", (o) => { const f = document.createElementNS(svgNS, "filter"); f.id = "probe-filter"; const i = document.createElementNS(svgNS, "feImage"); i.setAttribute("href", o + "/drawing-feimage.png"); f.append(i); root.append(f); shape("rect", "filter: url(#probe-filter)"); }],
    ["a use of another document", "/drawing-use.svg", (o) => { const e = document.createElementNS(svgNS, "use"); e.setAttribute("href", o + "/drawing-use.svg#u"); root.append(e); }],
    ["a paint in a presentation attribute", "/drawing-attr-paint.svg", (o) => { const e = document.createElementNS(svgNS, "rect"); e.setAttribute("width", "4"); e.setAttribute("height", "4"); e.setAttribute("fill", `url(${o}/drawing-attr-paint.svg#p)`); root.append(e); }],
  ];
  window.__probes = Object.fromEntries(kinds.map(([n, , f]) => [n, f]));
  return kinds.map(([n, a]) => [n, a]);
})()"#;

/// The other host: a listener counting every connection made to it, keeping the first line each one
/// asks (empty for one opened bare, asking nothing), until `stop`.
struct OtherHost {
    port: u16,
    asked: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl OtherHost {
    fn start() -> OtherHost {
        let other = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = other.local_addr().unwrap().port();
        other.set_nonblocking(true).unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = std::thread::spawn({
            let (asked, stop) = (std::sync::Arc::clone(&asked), std::sync::Arc::clone(&stop));
            move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    match other.accept() {
                        Ok((s, _)) => {
                            let asked = std::sync::Arc::clone(&asked);
                            // Each connection read apart, so a slow one never holds the next back.
                            std::thread::spawn(move || {
                                s.set_nonblocking(false).ok();
                                s.set_read_timeout(Some(Duration::from_millis(500))).ok();
                                let mut first = String::new();
                                let _ = BufReader::new(s).read_line(&mut first);
                                asked.lock().unwrap().push(first.trim_end().to_string());
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
            }
        });
        OtherHost { port, asked, stop, watcher: Some(watcher) }
    }

    fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// What was asked since `from` (a count of connections), the bare connections apart.
    fn requests_since(&self, from: usize) -> Vec<String> {
        self.asked.lock().unwrap()[from..].iter().filter(|l| !l.is_empty()).cloned().collect()
    }

    fn count(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

impl Drop for OtherHost {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(w) = self.watcher.take() {
            let _ = w.join();
        }
    }
}

/// A server on this machine answering every request with `body` and no policy at all (an SVG
/// drawing: the control for the drawing served alone; or a page of another origin), until dropped.
struct Plain {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Plain {
    fn serve(body: String) -> Plain {
        Plain::serve_as(body, "image/svg+xml")
    }

    fn serve_as(body: String, kind: &'static str) -> Plain {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        l.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = std::thread::spawn({
            let stop = std::sync::Arc::clone(&stop);
            move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    match l.accept() {
                        Ok((mut s, _)) => {
                            s.set_nonblocking(false).ok();
                            s.set_read_timeout(Some(Duration::from_millis(500))).ok();
                            let mut r = BufReader::new(s.try_clone().unwrap());
                            let mut line = String::new();
                            while r.read_line(&mut line).is_ok_and(|n| n > 2) {
                                line.clear();
                            }
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
            }
        });
        Plain { port, stop, server: Some(server) }
    }
}

impl Drop for Plain {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(s) = self.server.take() {
            let _ = s.join();
        }
    }
}

/// Every view a session's page lists in its tabs, shown in turn and drawn (a table and a series
/// picked, the first of each list), its page refusing nothing under its policy.
fn every_view_refuses_nothing(page: &mut Browser, session: &str) {
    let views: Vec<String> = page.eval(r#"[...document.querySelectorAll(".tabs [data-view]")].map((a) => a.dataset.view)"#).as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert!(views.len() >= 5 && views.contains(&"layout".to_string()), "{session}: {views:?}");
    for view in &views {
        page.eval(&format!("location.hash = {}; true", serde_json::to_string(view).unwrap()));
        page.wait_for("document.querySelector('main') && document.querySelector('main').children.length > 0 ? true : null", Duration::from_secs(20));
        if view == "tables" || view == "series" {
            page.wait_for("document.querySelector('.list button[data-table], .list button[data-series]') ? true : null", Duration::from_secs(20));
            page.eval("document.querySelector('.list button[data-table], .list button[data-series]').click(); true");
            page.wait_for("['page', 'chart', 'waiting', 'error'].includes(document.querySelector('.list-main')?.dataset.screen) ? true : null", Duration::from_secs(20));
        }
        std::thread::sleep(Duration::from_millis(1200));
        assert_eq!(page.eval("window.__refused"), json!([]), "{session}: the {view} view refused nothing");
    }
}

/// A browser watched from its first load at `url` (`WATCH` registered before it), returned once that
/// document has loaded, waited for through the protocol's page events (`navigate_loaded`).
fn watched_from_first_load(url: &str) -> Browser {
    let mut p = Browser::open("about:blank");
    p.call("Page.enable", json!({}));
    p.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": WATCH }));
    p.navigate_loaded(url, Duration::from_secs(20));
    p
}

/// The host's content policy (Q23), word for word on the page and on the drawing's own route, and
/// every answer the host gives `nosniff`; and in the browser, proven as a layer of its own, past
/// the reader. Every screen of the page as built refuses nothing under it: the sessions' list at
/// `/`, and of a session every view its own tabs list, each drawn (the layout with an embedded
/// picture in its drawing, a dashboard, a table and a series picked, the alarms) and an object
/// selected with its sparkline. Then every kind of load and of script (`PAGE_KINDS`), each alone:
/// in a blank document with no policy it asks the other host (a listener standing for it) or runs,
/// so the probe is live; put into the mounted page it asks nothing and runs nothing, refused by its
/// own address under its directive. A kind the browser asks nothing for even with no policy (some
/// of the references to another document) is named, and asks nothing under the policy either. The
/// drawing opened alone at its own route is sandboxed (an origin of no one's): each of its kinds
/// (`DRAWING_KINDS`) live in a drawing with no policy (a `data:` one) asks and runs nothing there.
/// A connection opened bare, asking nothing (the browser's own preconnect for a frame, before its
/// policy refuses the frame), is the stated limit.
#[test]
fn the_page_loads_and_runs_its_own_files_alone_whatever_its_drawing_points_to() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (plant(&dir.path().join("bench-a")), plant(&dir.path().join("bench-b")));
    let path = two_plants(dir.path(), &a, &b);
    // The other host, which counts (a connection made from here is seen, asking nothing).
    let other = OtherHost::start();
    // A picture embedded in loop's drawing, beside the objects; and Inkscape's script beside a mesh
    // gradient as the root's last child (Q27, decided: the reader drops it and loads the drawing),
    // its code asking the other host, so a run of it anywhere is seen.
    let svg_path = dir.path().join("loop/layout.svg");
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    assert!(svg.contains("</svg>"));
    let polyfill = format!("  <script id=\"mesh_polyfill\" type=\"text/javascript\"><![CDATA[\n// Use Canvas to render a mesh gradient, passing the rendering to an image via a data stream.\nfetch(\"{}/polyfill-ran\");\n]]></script>\n", other.address());
    std::fs::write(&svg_path, svg.replacen("</svg>", &format!("  <image id=\"pic\" x=\"2\" y=\"2\" width=\"8\" height=\"8\" href=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==\"/>\n{polyfill}</svg>"), 1)).unwrap();
    let polyfill_ran = |from: usize| other.requests_since(from).iter().any(|l| l.contains("/polyfill-ran"));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    const PAGE: &str = "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
    const DRAWING: &str = "default-src 'none'; style-src-attr 'unsafe-inline'; img-src data:; sandbox; frame-ancestors 'none'";
    for p in ["/", "/s/loop", "/s/loop/", "/s/hall/"] {
        let (status, h) = headers_of(addr, p);
        assert_eq!((status, h.get("content-security-policy").map(String::as_str), h.get("x-content-type-options").map(String::as_str)), (200, Some(PAGE), Some("nosniff")), "{p}");
    }
    let (status, h) = headers_of(addr, "/s/loop/layout.svg");
    assert_eq!((status, h.get("content-security-policy").map(String::as_str), h.get("x-content-type-options").map(String::as_str), h.get("content-type").map(String::as_str)), (200, Some(DRAWING), Some("nosniff"), Some("image/svg+xml")), "the drawing's own route");
    for p in ["/assets/app.js", "/assets/app.css", "/api/sessions", "/s/loop/binding", "/s/loop/objects.json", "/s/loop/dashboard/0", "/s/loop/portal/api/tables", "/nothing/here"] {
        let (_, h) = headers_of(addr, p);
        assert_eq!(h.get("x-content-type-options").map(String::as_str), Some("nosniff"), "{p}");
        assert!(!h.contains_key("content-security-policy"), "{p}: a policy on an answer no browser renders as the page");
    }

    drop(TcpStream::connect(("127.0.0.1", other.port)).unwrap());
    assert!(wait(Duration::from_secs(5), || other.count() == 1), "the listener counts");
    let other_js = serde_json::to_string(&other.address()).unwrap();

    // Every screen of the page as built, refusing nothing: the sessions' list, then each view the
    // page's own tabs list, drawn, and an object selected.
    let mut list = watched_from_first_load(&format!("http://{addr}/"));
    list.wait_for("document.querySelectorAll('.session-card').length === 2 || null", Duration::from_secs(20));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(list.eval("window.__refused"), json!([]), "the sessions' list refused nothing");
    drop(list);
    let mut page = watched_from_first_load(&format!("http://{addr}/s/loop/"));
    page.wait_for(r#"document.getElementById("pic") && document.getElementById("outlet-1")?.classList.contains("sv-obj") ? true : null"#, Duration::from_secs(20));
    every_view_refuses_nothing(&mut page, "loop");
    // And every other session the host lists, its own page and every view (hall has no layout, its
    // layout view the page's words for that).
    let (_, sessions) = get(addr, "/api/sessions");
    for s in sessions["sessions"].as_array().unwrap().iter().filter(|s| s["name"] != "loop") {
        let name = s["name"].as_str().unwrap();
        let mut other_page = watched_from_first_load(&format!("http://{addr}/s/{name}/"));
        other_page.wait_for("document.querySelectorAll('.tabs [data-view]').length > 0 || null", Duration::from_secs(20));
        every_view_refuses_nothing(&mut other_page, name);
    }
    page.eval("location.hash = 'layout'; true");
    page.wait_for(r#"document.getElementById("outlet-1")?.classList.contains("sv-obj") ? true : null"#, Duration::from_secs(20));
    page.eval(r#"document.getElementById("outlet-1").dispatchEvent(new MouseEvent("click", { bubbles: true })); true"#);
    page.wait_for("document.querySelector('.side .spark-wrap')?.children.length ? true : null", Duration::from_secs(20));
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(page.eval("window.__refused"), json!([]), "an object selected, its sparkline drawn, refused nothing");
    // Every object of every session selected in turn (by its element where the layout draws it, else
    // by its row in the attention list), its side panel built (the writes it offers, their gate and
    // their answers' place among them), refusing nothing.
    let (mut selected, mut offering) = (BTreeMap::new(), 0);
    for name in ["loop", "hall"] {
        let (_, view) = get(addr, &format!("/s/{name}/binding"));
        let mut side = watched_from_first_load(&format!("http://{addr}/s/{name}/"));
        side.wait_for("document.querySelector('.side .attention') && document.querySelectorAll('.tabs [data-view]').length > 0 ? true : null", Duration::from_secs(20));
        if name == "loop" {
            side.wait_for(r#"document.getElementById("outlet-1")?.classList.contains("sv-obj") ? true : null"#, Duration::from_secs(20));
        }
        std::thread::sleep(Duration::from_millis(1500));
        for o in view["objects"].as_array().unwrap() {
            let id = serde_json::to_string(o["id"].as_str().unwrap()).unwrap();
            let how = side.eval(&format!("(() => {{ const el = document.getElementById({id}); if (el && el.classList.contains('sv-obj')) {{ el.dispatchEvent(new MouseEvent('click', {{ bubbles: true }})); return 'its element'; }} const li = [...document.querySelectorAll('.side ul.attention li')].find((l) => l.dataset.id === {id}); if (li) {{ li.click(); return 'its row'; }} return null; }})()"));
            let Some(how) = how.as_str().map(String::from) else { continue };
            side.wait_for(&format!("document.querySelector('.side .sel-title')?.dataset.id === {id} || null"), Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(700));
            assert_eq!(side.eval("window.__refused"), json!([]), "{name}: {id} selected by {how}, refused nothing");
            if !o["actions"].as_array().unwrap().is_empty() {
                assert!(side.eval("document.querySelectorAll('.side .actions button').length").as_u64().unwrap() > 0, "{name}: {id}'s writes offered");
                offering += 1;
            }
            *selected.entry(name).or_insert(0) += 1;
        }
        drop(side);
    }
    assert!(selected.get("loop").is_some_and(|n| *n >= 5) && selected.contains_key("hall") && offering >= 2, "objects selected: {selected:?}, offering writes: {offering}");

    // Each kind alone in a blank document with no policy: live (it asks the other host, or runs).
    let mut blank = Browser::open("about:blank");
    let kinds: Vec<(String, String, String)> = blank.eval(PAGE_KINDS).as_array().unwrap().iter().map(|k| (k[0].as_str().unwrap().to_string(), k[1].as_str().unwrap().to_string(), k[2].as_str().unwrap().to_string())).collect();
    let mut live = std::collections::BTreeSet::new();
    let mut silent = Vec::new();
    for (kind, _, address) in &kinds {
        let (from, ran) = (other.count(), blank.eval("window.__ran.length").as_u64().unwrap());
        blank.eval(&format!("window.__probes[{}]({other_js})", serde_json::to_string(kind).unwrap()));
        std::thread::sleep(Duration::from_millis(700));
        let asked = other.requests_since(from);
        let ran_now = blank.eval("window.__ran.length").as_u64().unwrap() > ran;
        if (address != "inline" && asked.iter().any(|l| l.contains(address.as_str()))) || (address == "inline" && (ran_now || !asked.is_empty())) {
            live.insert(kind.clone());
        } else {
            silent.push(kind.clone());
        }
    }
    drop(blank);
    // Every kind live but the references to another document a browser may never fetch; those the
    // browser asks nothing for even with no policy are named (and held below to asking nothing).
    let may_be_silent = ["a style's cursor", "a paint from another document", "a filter from another document", "a mask from another document", "a clip path from another document", "a marker from another document", "a use of another document", "a manifest"];
    assert!(silent.iter().all(|k| may_be_silent.contains(&k.as_str())), "a probe that tried nothing with no policy: {silent:?}");
    eprintln!("kinds the browser asks nothing for even with no policy: {silent:?}");

    // Then each kind alone in the mounted page: asking nothing, running nothing, refused by its own
    // address under its directive where it is live.
    page.call("Runtime.evaluate", json!({ "expression": PAGE_KINDS, "returnByValue": true }));
    for (kind, directive, address) in &kinds {
        let (from, refused_before) = (other.count(), page.eval("window.__refused.length").as_u64().unwrap() as usize);
        page.eval(&format!("window.__probes[{}]({other_js})", serde_json::to_string(kind).unwrap()));
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(other.requests_since(from), Vec::<String>::new(), "{kind}: asked of the other host");
        assert_eq!(page.eval("window.__ran"), json!([]), "{kind}: ran");
        let refused: Vec<(String, String)> = page.eval("window.__refused").as_array().unwrap()[refused_before..].iter().map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_str().unwrap().to_string())).collect();
        if live.contains(kind) {
            // Its own address, or the other host's origin alone where the browser strips a path from
            // what it reports (a frame's navigation): the kind ran alone, so what it refused is its own.
            let own = refused.iter().any(|(d, u)| d == directive && (if address == "inline" { u == "inline" } else { u.ends_with(address.as_str()) || *u == other.address() }));
            assert!(own, "{kind}: refused by its own address ({address}) under {directive}: {refused:?}");
        }
    }

    // The drawing opened alone, at its own route: an origin of no one's (the sandbox). Each of its
    // kinds live in the same drawing served with no policy from this machine (it asks the other host
    // or runs), and in the drawing alone asking nothing and running nothing.
    // Inkscape's script in the drawing ran nowhere in the page, on any of its screens (the page drops
    // every script before it draws); served with no policy, it runs (the probe live).
    assert!(!polyfill_ran(0), "Inkscape's script beside the mesh ran in the page");
    let plain_from = other.count();
    let plain = Plain::serve(std::fs::read_to_string(&svg_path).unwrap());
    let mut bare = Browser::open("about:blank");
    bare.call("Page.enable", json!({}));
    bare.navigate_loaded(&format!("http://127.0.0.1:{}/layout.svg", plain.port), Duration::from_secs(20));
    bare.wait_for("document.documentElement && document.documentElement.tagName === 'svg' ? true : null", Duration::from_secs(20));
    assert!(wait(Duration::from_secs(10), || polyfill_ran(plain_from)), "Inkscape's script live in the drawing served with no policy");
    let drawing_kinds: Vec<(String, String)> = bare.eval(DRAWING_KINDS).as_array().unwrap().iter().map(|k| (k[0].as_str().unwrap().to_string(), k[1].as_str().unwrap().to_string())).collect();
    let mut drawing_live = std::collections::BTreeSet::new();
    let mut drawing_silent = Vec::new();
    for (kind, address) in &drawing_kinds {
        let (from, ran) = (other.count(), bare.eval("window.__ran.length").as_u64().unwrap());
        bare.eval(&format!("window.__probes[{}]({other_js})", serde_json::to_string(kind).unwrap()));
        std::thread::sleep(Duration::from_millis(700));
        let asked = other.requests_since(from);
        let ran_now = bare.eval("window.__ran.length").as_u64().unwrap() > ran;
        if (address != "inline" && asked.iter().any(|l| l.contains(address.as_str()))) || (address == "inline" && ran_now) {
            drawing_live.insert(kind.clone());
        } else {
            drawing_silent.push(kind.clone());
        }
    }
    drop(bare);
    drop(plain);
    assert!(drawing_silent.iter().all(|k| k.contains("another document")), "a probe that tried nothing in a drawing with no policy: {drawing_silent:?}");
    eprintln!("kinds a drawing with no policy asks nothing for: {drawing_silent:?}");
    // (Waited for through the protocol's page events, never by evaluating into it while it commits:
    // a sandboxed document can then offer no context, CI's red on 114bcc8.)
    let drawing = format!("http://{addr}/s/loop/layout.svg");
    let alone_from = other.count();
    let mut alone = Browser::open("about:blank");
    alone.call("Page.enable", json!({}));
    alone.navigate_loaded(&drawing, Duration::from_secs(20));
    alone.wait_for("document.documentElement && document.documentElement.tagName === 'svg' && document.getElementById('outlet-1') ? true : null", Duration::from_secs(20));
    assert_eq!(alone.eval("window.origin"), json!("null"), "the drawing alone, in a sandbox of no origin");
    alone.eval(DRAWING_KINDS);
    for (kind, _) in &drawing_kinds {
        let from = other.count();
        alone.eval(&format!("window.__probes[{}]({other_js})", serde_json::to_string(kind).unwrap()));
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(other.requests_since(from), Vec::<String>::new(), "{kind}: asked of the other host from the drawing alone");
        assert_eq!(alone.eval("window.__ran"), json!([]), "{kind}: ran in the drawing alone");
    }
    assert!(drawing_live.len() >= 6, "the drawing's kinds live with no policy: {drawing_live:?}");
    // Opened alone ten times more, from a blank page each time: the first evaluate after its load
    // runs in the sandboxed drawing every time.
    for i in 0..10 {
        alone.navigate_loaded("about:blank", Duration::from_secs(20));
        alone.navigate_loaded(&drawing, Duration::from_secs(20));
        assert_eq!(alone.eval("window.origin"), json!("null"), "open {i}");
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(!polyfill_ran(alone_from), "Inkscape's script ran in the drawing alone");
    drop(alone);

    // Framed by another site (the operator's ruling of 2026-11-15, both policies' `frame-ancestors`):
    // a page of another origin framing the page and the drawing's route sees each refused, by the
    // browser's own report of the directive (one per frame, naming the host's origin, as the browser
    // reports a frame), and no document of either in its frames; a file of the host's that carries
    // no policy, framed beside them, shown (the control).
    let page_url = format!("http://{addr}/s/loop/");
    let control = format!("http://{addr}/assets/app.css");
    let framer = Plain::serve_as(format!("<!doctype html><title>framer</title><iframe src=\"{page_url}\"></iframe><iframe src=\"{drawing}\"></iframe><iframe src=\"{control}\"></iframe>"), "text/html; charset=utf-8");
    let mut outer = Browser::open("about:blank");
    outer.call("Page.enable", json!({}));
    outer.call("Log.enable", json!({}));
    outer.navigate_loaded(&format!("http://127.0.0.1:{}/", framer.port), Duration::from_secs(20));
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let tree = outer.call("Page.getFrameTree", json!({}));
        let kids: Vec<String> = tree["frameTree"]["childFrames"].as_array().map(|c| c.iter().map(|f| f["frame"]["url"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
        let said: Vec<String> = outer.events_of("Log.entryAdded").iter().map(|e| e["params"]["entry"]["text"].as_str().unwrap_or("").to_string()).collect();
        let refused = said.iter().filter(|t| t.contains("frame-ancestors 'none'") && t.contains(&format!("'http://{addr}/'"))).count();
        if kids.contains(&control) && refused >= 2 {
            std::thread::sleep(Duration::from_millis(300));
            let said = outer.events_of("Log.entryAdded").iter().filter(|e| e["params"]["entry"]["text"].as_str().is_some_and(|t| t.contains("frame-ancestors"))).count();
            assert_eq!(said, 2, "a refusal for each of the two framed documents with the directive, none for the control");
            assert!(!kids.contains(&page_url) && !kids.contains(&drawing), "a refused frame holding its document: {kids:?}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the page and the drawing never refused in another site's frames, or the control never shown: {kids:?} {said:?}");
        std::thread::sleep(Duration::from_millis(150));
    }
    drop(outer);
    drop(framer);
    drop(page);
    drop(other);
    host.stop();
}
