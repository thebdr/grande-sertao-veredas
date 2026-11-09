//! C-051: the 2D layout, the part a machine can hold: everything the page binds is served (the
//! SVG as written, every bound element's object in the binding and in the feed with what its way
//! of showing needs) and the page's bundle reads the routes and draws the vocabulary. That the
//! page shows it is verified in the browser against a bench-fed session (`.zen/evidence/C-051.md`).

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, config, web};

#[test]
fn everything_the_layout_binds_is_served_and_the_page_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let path = write_example(dir.path(), &live_example(&bench, false));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();

    // The drawing as written, its elements carrying the objects' ids.
    let (status, headers, svg) = http(addr, "GET", "/s/loop/layout.svg", None);
    assert_eq!((status, svg.replace("\r\n", "\n")), (200, example().svg));
    assert!(headers.contains("content-type: image/svg+xml") && headers.contains("cache-control: no-cache"), "{headers}");
    let (_, b) = get(addr, "/s/loop/binding");
    let on_layout: Vec<&Value> = b["objects"].as_array().unwrap().iter().filter(|o| svg.contains(&format!("id=\"{}\"", o["id"].as_str().unwrap()))).collect();
    assert_eq!(on_layout.iter().map(|o| (o["id"].as_str().unwrap(), o["show"].as_str().unwrap())).collect::<Vec<_>>(), [
        ("infeed", "state"), ("outlet-1", "level"), ("outlet-2", "level"), ("outlet-3", "level"), ("parcel-1", "move"), ("parcel-2", "move"),
    ]);
    assert!(on_layout.iter().filter(|o| o["show"] == "move").all(|o| o["path"] == "loop" && svg.contains("<path id=\"loop\"")));

    // The feed carries, for each, what its way of showing needs: a state with a mark and a word,
    // a level's fill, a marker's place; moved by the plant, the place moves.
    assert!(wait(Duration::from_secs(20), || {
        let (_, s) = get(addr, "/s/loop/objects.json");
        object(&s, "parcel-1")["place"] == json!(0.25) && object(&s, "outlet-2")["level"] == json!(0.85) && object(&s, "infeed")["word"] == json!("running")
    }));
    let (_, s) = get(addr, "/s/loop/objects.json");
    for o in &on_layout {
        let v = object(&s, o["id"].as_str().unwrap());
        assert!(v["state"].is_string() && v["mark"].is_string() && v["word"].is_string(), "{v}");
        match o["show"].as_str().unwrap() {
            "level" => assert!(v["level"].is_f64() && v["place"].is_null(), "{v}"),
            "move" => assert!(v["place"].is_f64() && v["level"].is_null(), "{v}"),
            _ => assert!(v["level"].is_null() && v["place"].is_null(), "{v}"),
        }
    }
    bench.server("plc").unwrap().set("p1", 60.0);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "parcel-1")["place"] == json!(0.6)));

    // The page's bundle reads the session's binding, layout and feed, and draws the vocabulary:
    // every class, its mark, the selection.
    for needle in ["/binding", "/layout.svg", "/objects", "sv-state-", "sv-selected", "sv-fill", "getPointAtLength", "Needs attention"] {
        assert!(web::APP_JS.contains(needle) || web::APP_CSS.contains(needle), "{needle}");
    }
    for class in ["normal", "near", "warn", "alarm", "disabled", "stale"] {
        assert!(web::APP_CSS.contains(&format!("sv-state-{class}")), "{class}");
    }
    assert!(web::APP_CSS.contains("--accent:#ac007c") || web::APP_CSS.contains("--accent: #ac007c"), "the accent is the web magenta");
    host.stop();
}
