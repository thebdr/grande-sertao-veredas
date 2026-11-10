//! C-054: writes, gated. The binding offers a write; the page posts it to the session's portal
//! write route through supervisão's address, where portal's switch answers: refused 403 while
//! the switch is off with nothing reaching the plant, landing at the bench once the project arms
//! it (or a shell arms it in process). What the page sends is the binding's action, the one
//! pressed: its value and its source, asked and posted as the binding says. Supervisão has no
//! write path of its own: no route of its own takes a POST, no source of its names a write handle,
//! the switch or its start position.

mod common;

use std::time::Duration;

use common::browser::Browser;
use common::*;
use serde_json::json;
use veredas_maquinar::bench::Bench;
use veredas_maquinar::opc::render_value;
use veredas_supervisao::{Host, config};

fn release(bench: &Bench) -> String {
    render_value(&bench.server("plc").unwrap().get("release3"))
}

fn hold(bench: &Bench) -> String {
    render_value(&bench.server("aux").unwrap().get("hold3"))
}

/// A second controller, `aux`, with a tag of its own to write.
const AUX: &str = "\n[[opc]]\nname = \"aux\"\nport = 0\ntags = [\n  { id = \"hold3\", type = \"int\", value = \"0\", access = \"read-write\" },\n]\n";

/// The example with outlet 3 offering two writes no test meets by accident: a release of 7 to the
/// controller it reads, and a hold of 3 to `aux`, a source it does not read.
fn with_aux(mut ex: Example, bench: &Bench) -> Example {
    let aux = bench.server("aux").unwrap();
    ex.project.push_str(&format!(
        "\n[[opc]]\nsource_id = \"aux\"\nendpoint_url = \"{}\"\npoll_ms = 50\nrules = [\n  {{ name = \"aux_main\", tags = [\n    {{ node_id = \"ns={};s=hold3\", series = \"hold3\", type = \"int\", writable = true }},\n  ] }},\n]\n",
        aux.endpoint_url, aux.namespace
    ));
    let plc = bench.server("plc").unwrap().namespace;
    let one = format!("actions = [ {{ label = \"Release\", source = \"plc\", tag = \"ns={plc};s=release3\", value = \"1\" }} ]");
    assert!(ex.binding.contains(&one), "{}", ex.binding);
    let two = format!("actions = [ {{ label = \"Release\", source = \"plc\", tag = \"ns={plc};s=release3\", value = \"7\" }}, {{ label = \"Hold\", source = \"aux\", tag = \"ns={};s=hold3\", value = \"3\" }} ]", aux.namespace);
    ex.binding = ex.binding.replacen(&one, &two, 1);
    ex
}

/// Every Rust file under `dir`, however deep.
fn rust_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn a_write_is_refused_on_the_wire_while_the_switch_is_off_and_lands_once_it_is_armed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant_with(&dir.path().join("bench"), AUX);
    let ns = bench.server("plc").unwrap().namespace;
    let aux_ns = bench.server("aux").unwrap().namespace;
    let path = write_example(dir.path(), &with_aux(live_example(&bench, false), &bench));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();

    // The writes the binding offers, as the page reads them, and the switch the feed shows.
    let (_, b) = get(addr, "/s/loop/binding");
    let actions = b["objects"].as_array().unwrap().iter().find(|o| o["id"] == "outlet-3").unwrap()["actions"].clone();
    assert_eq!(actions, json!([
        { "label": "Release", "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "7" },
        { "label": "Hold", "source": "aux", "tag": format!("ns={aux_ns};s=hold3"), "value": "3" },
    ]));
    let action = actions[0].clone();
    assert!(wait(Duration::from_secs(20), || {
        let s = get(addr, "/s/loop/objects.json").1;
        s["session"]["writes_armed"] == json!(false) && s["session"]["health"]["plc"]["state"] == json!("live") && s["session"]["health"]["aux"]["state"] == json!("live")
    }));
    let body = json!({ "source": action["source"], "tag": action["tag"], "value": action["value"] });

    // Off: refused by the switch, on the wire, through supervisão's address; nothing at the plant.
    assert_eq!(release(&bench), "0");
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body), (403, json!({ "error": "writes are turned off for this project" })));
    assert_eq!(post(addr, "/s/loop/portal/api/channels/send", &json!({ "source": "plc", "message": "x" })), (403, json!({ "error": "writes are turned off for this project" })));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0", "nothing reached the plant");

    // Armed in process by the shell that runs the session: the same post lands, seen by the
    // plant; the feed shows the switch.
    host.session("loop").unwrap().set_writes_armed(true);
    assert!(wait(Duration::from_secs(10), || get(addr, "/s/loop/objects.json").1["session"]["writes_armed"] == json!(true)));
    let (status, v) = post(addr, "/s/loop/portal/api/opc/write", &body);
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(10), || release(&bench) == "7"), "{}", release(&bench));
    // Armed, the whitelist still holds: a tag no rule marks writable is refused by name.
    let speed = format!("ns={ns};s=speed");
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &json!({ "source": "plc", "tag": speed, "value": "9" })), (403, json!({ "error": format!("`plc` / `{speed}` is not a writable tag in this session") })));
    // Disarmed again: refused again, the plant keeps what it had.
    host.session("loop").unwrap().set_writes_armed(false);
    bench.server("plc").unwrap().set("release3", 0);
    assert!(wait(Duration::from_secs(10), || release(&bench) == "0"));
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body).0, 403);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0");

    // Supervisão's code reaches no write handle and no switch: the library and the executable
    // name none of portal's write paths, and the page posts to portal's route alone.
    // Nor does it decide the switch's start position (portal keeps it crate-private), start a
    // worker of its own, or hide code from this reading (no file pulled in by path, no build
    // script).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("build.rs").exists() && !std::fs::read_to_string(root.join("Cargo.toml")).unwrap().contains("build ="), "no build script writes code this reading cannot see");
    let files = rust_files(&root.join("src"));
    assert!(files.len() >= 8, "{files:?}");
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        for word in [
            "write_tags", "opc_handle", "channel_handle", "set_writes_armed", "WriteGate", ".gate", "live.opc", "live.channels", "/api/opc/write", "/api/channels/send",
            "writes_armed =", "writes_armed |=", "writes_armed:", "channel::spawn", "worker::spawn", "include!", "#[path",
        ] {
            assert!(!text.contains(word), "{} names `{word}`", path.display());
        }
    }
    let bundle = veredas_supervisao::web::APP_JS;
    assert_eq!(bundle.matches("/api/opc/write").count(), 1, "the page posts to one route");
    assert!(bundle.contains("/portal/api/opc/write") && !bundle.contains("/api/channels/send"), "the route is portal's own");
    assert_eq!(bundle.matches("method:\"POST\"").count() + bundle.matches("method: \"POST\"").count(), 1, "one POST in the page");

    // Supervisão's own routes take no write.
    for p in ["/", "/assets/app.js", "/assets/app.css", "/api/sessions", "/s/loop", "/s/loop/", "/s/loop/binding", "/s/loop/objects", "/s/loop/objects.json", "/s/loop/dashboard/0", "/s/loop/layout.svg"] {
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let (status, _, _) = http(addr, method, p, Some(&body.to_string()));
            assert_eq!(status, 405, "{method} {p}");
        }
        // Nor does a read move the switch (the object stream never ends; it is read elsewhere).
        if p != "/s/loop/objects" {
            let _ = http(addr, "GET", p, None);
        }
    }
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(get(addr, "/s/loop/objects.json").1["session"]["writes_armed"], json!(false), "no route of supervisão's armed the switch");
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body).0, 403);

    // The page itself, run in a headless browser: the offered write is a button, disabled while
    // the switch is off; pressed anyway (the page's own handler, the button re-enabled by hand),
    // its post is refused on the wire and the plant keeps its value; enabled once the switch is
    // armed, pressed and confirmed, it lands at the plant; asked and not confirmed, nothing is
    // sent; the other action pressed, its own value lands at its own controller and the first
    // tag keeps what it had; disarmed, disabled again. Every question asked and every body posted
    // is the binding's action, the one pressed.
    let mut page = Browser::watched(&format!("http://{addr}/s/loop/"));
    page.wait_for(r#"document.getElementById("outlet-3") !== null && document.getElementById("outlet-3").classList.contains("sv-obj")"#, Duration::from_secs(20));
    page.eval(r#"(() => { window.asked = []; window.answer = true; window.confirm = (m) => { window.asked.push(m); return window.answer; }; return true; })()"#);
    page.eval(r#"document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true }))"#);
    const BUTTON: &str = r#"(() => { const b = document.querySelector('.side .actions button[data-action="Release"]'); return b ? { disabled: b.disabled, gate: document.querySelector(".side .gate-note").textContent, note: document.querySelector(".side .write-note").textContent } : null; })()"#;
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.gate !== '' ? b : null; }})()"), Duration::from_secs(20));
    assert_eq!(b, json!({ "disabled": true, "gate": "Writes go through portal. Its write switch is off.", "note": "" }));
    page.eval(r#"(() => { window.answer = true; const b = document.querySelector('.side .actions button[data-action="Release"]'); b.click(); return b.disabled; })()"#);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(page.eval(BUTTON)["note"], json!(""), "a disabled button sends nothing");
    page.eval(r#"(() => { const b = document.querySelector('.side .actions button[data-action="Release"]'); b.disabled = false; b.click(); return true; })()"#);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.note !== '' ? b : null; }})()"), Duration::from_secs(10));
    assert_eq!(b["note"], json!("Release: refused, 403: writes are turned off for this project"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0", "nothing reached the plant");
    host.session("loop").unwrap().set_writes_armed(true);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && !b.disabled ? b : null; }})()"), Duration::from_secs(10));
    assert_eq!(b["gate"], json!("Writes go through portal. Its write switch is on."));
    page.eval(r#"(() => { window.answer = false; document.querySelector('.side .actions button[data-action="Release"]').click(); return true; })()"#);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((page.eval(BUTTON)["note"].clone(), release(&bench)), (json!("Release: refused, 403: writes are turned off for this project"), "0".to_string()), "not confirmed, nothing sent");
    page.eval(r#"(() => { window.answer = true; document.querySelector('.side .actions button[data-action="Release"]').click(); return true; })()"#);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.note.startsWith('Release: written') ? b : null; }})()"), Duration::from_secs(10));
    assert!(wait(Duration::from_secs(10), || release(&bench) == "7"), "the page's write landed: {b}");
    assert_eq!(hold(&bench), "0", "and nothing else");
    bench.server("plc").unwrap().set("release3", 0);
    assert!(wait(Duration::from_secs(10), || release(&bench) == "0"));
    page.eval(r#"(() => { window.answer = true; document.querySelector('.side .actions button[data-action="Hold"]').click(); return true; })()"#);
    page.wait_for(r#"document.querySelector(".side .write-note").textContent.startsWith("Hold: written")"#, Duration::from_secs(10));
    assert!(wait(Duration::from_secs(10), || hold(&bench) == "3"), "the hold landed at its own controller: {}", hold(&bench));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(release(&bench), "0", "the hold wrote nothing to the release");
    let (release_asked, hold_asked) = (format!("Write 7 to ns={ns};s=release3 of plc?"), format!("Write 3 to ns={aux_ns};s=hold3 of aux?"));
    assert_eq!(page.eval("window.asked"), json!([release_asked, release_asked, release_asked, hold_asked]), "the refused press, the one not confirmed, the one that landed, the hold");
    let sent: Vec<(String, serde_json::Value)> = page.posts().into_iter().map(|(u, b)| (u, serde_json::from_str(&b).unwrap())).collect();
    let to = format!("http://{addr}/s/loop/portal/api/opc/write");
    let (release_body, hold_body) = (json!({ "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "7" }), json!({ "source": "aux", "tag": format!("ns={aux_ns};s=hold3"), "value": "3" }));
    assert_eq!(sent, vec![(to.clone(), release_body.clone()), (to.clone(), release_body), (to, hold_body)], "the refused post, the release, the hold: each the binding's action");
    host.session("loop").unwrap().set_writes_armed(false);
    page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.disabled && b.gate.endsWith('is off.') ? b : null; }})()"), Duration::from_secs(10));
    // Every request the page made for a write went to portal's route under the session.
    let posts = page.eval(r#"performance.getEntriesByType("resource").filter((e) => e.name.includes("/write")).map((e) => new URL(e.name).pathname)"#);
    assert_eq!(posts, json!(["/s/loop/portal/api/opc/write", "/s/loop/portal/api/opc/write", "/s/loop/portal/api/opc/write"]), "the refused post, the release and the hold");
    drop(page);
    host.stop();

    // Armed by the project: the post lands from the start.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let path = write_example(dir.path(), &live_example(&bench, true));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    // Written once the source is connected: before that, the plant never saw it (503).
    assert!(wait(Duration::from_secs(20), || { let s = get(addr, "/s/loop/objects.json").1; s["session"]["writes_armed"] == json!(true) && s["session"]["health"]["plc"]["state"] == json!("live") }));
    let ns = bench.server("plc").unwrap().namespace;
    let (status, v) = post(addr, "/s/loop/portal/api/opc/write", &json!({ "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "1" }));
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(10), || release(&bench) == "1"));
    host.stop();
}
