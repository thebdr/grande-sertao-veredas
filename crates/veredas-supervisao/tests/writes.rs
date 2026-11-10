//! C-054: writes, gated. The binding offers a write; the page posts it to the session's portal
//! write route through supervisão's address, where portal's switch answers: refused 403 while
//! the switch is off with nothing reaching the plant, landing at the bench once the project arms
//! it (or a shell arms it in process). Supervisão has no write path of its own: no route of its
//! own takes a POST.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;
use veredas_maquinar::opc::render_value;
use veredas_supervisao::{Host, config};

fn release(bench: &veredas_maquinar::bench::Bench) -> String {
    render_value(&bench.server("plc").unwrap().get("release3"))
}

#[test]
fn a_write_is_refused_on_the_wire_while_the_switch_is_off_and_lands_once_it_is_armed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let ns = bench.server("plc").unwrap().namespace;
    let path = write_example(dir.path(), &live_example(&bench, false));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();

    // The write the binding offers, as the page reads it, and the switch the feed shows.
    let (_, b) = get(addr, "/s/loop/binding");
    let action = b["objects"].as_array().unwrap().iter().find(|o| o["id"] == "outlet-3").unwrap()["actions"][0].clone();
    assert_eq!(action, json!({ "label": "Release", "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "1" }));
    assert!(wait(Duration::from_secs(20), || { let s = get(addr, "/s/loop/objects.json").1; s["session"]["writes_armed"] == json!(false) && s["session"]["health"]["plc"]["state"] == json!("live") }));
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
    assert!(wait(Duration::from_secs(10), || release(&bench) == "1"), "{}", release(&bench));
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
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for word in ["write_tags", "opc_handle", "channel_handle", "set_writes_armed", "WriteGate", "/api/opc/write", "/api/channels/send"] {
            assert!(!text.contains(word), "{} names `{word}`", path.display());
        }
    }
    let bundle = veredas_supervisao::web::APP_JS;
    assert_eq!(bundle.matches("/api/opc/write").count(), 1, "the page posts to one route");
    assert!(bundle.contains("/portal/api/opc/write") && !bundle.contains("/api/channels/send"), "the route is portal's own");
    assert_eq!(bundle.matches("method:\"POST\"").count() + bundle.matches("method: \"POST\"").count(), 1, "one POST in the page");

    // Supervisão's own routes take no write.
    for p in ["/", "/api/sessions", "/s/loop/", "/s/loop/binding", "/s/loop/objects", "/s/loop/objects.json", "/s/loop/dashboard/0", "/s/loop/layout.svg"] {
        let (status, _, _) = http(addr, "POST", p, Some(&body.to_string()));
        assert_eq!(status, 405, "{p}");
    }
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
