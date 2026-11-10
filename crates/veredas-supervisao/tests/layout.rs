//! C-051: the 2D layout. Everything the page binds is served (the SVG as written, every bound
//! element's object in the binding and in the feed with what its way of showing needs), and the
//! page itself, run in a headless browser against the same session, draws it: every bound
//! element's state as a class with its mark and word beside it (a marker's below it, moving with
//! it), every level's fill to its height, every marker at its place along its path (the point
//! computed here from the drawing's own geometry), the objects not normal in the attention list
//! in order, the headline, and a selected object's value, sparkline and offered write, all read
//! back from the page's document against the feed; Veredas' green accent on the selection alone,
//! never a state, and no magenta anywhere in the bundle.

mod common;

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use common::browser::Browser;
use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, config, web};

/// The point at `f` (0 to 1) along the reference's `loop`: `M 80 40 H 320 A 70 70 0 0 1 320 180
/// H 80 A 70 70 0 0 1 80 40 Z`, two straights of 240 and two half turns of radius 70.
fn on_loop(f: f64) -> (f64, f64) {
    let arc = 70.0 * PI;
    let mut s = f * (480.0 + 2.0 * arc);
    if s <= 240.0 {
        return (80.0 + s, 40.0);
    }
    s -= 240.0;
    if s <= arc {
        let t = s / 70.0;
        return (320.0 + 70.0 * t.sin(), 110.0 - 70.0 * t.cos());
    }
    s -= arc;
    if s <= 240.0 {
        return (320.0 - s, 180.0);
    }
    s -= 240.0;
    let t = s / 70.0;
    (80.0 - 70.0 * t.sin(), 110.0 + 70.0 * t.cos())
}

/// What the page drew for every bound element, read from its document.
const DRAWN: &str = r#"(() => {
  const st = (el) => el ? ([...el.classList].find((c) => c.startsWith("sv-state-")) || "") : null;
  const out = {};
  for (const el of document.querySelectorAll("svg.plant [data-object][id]")) {
    const id = el.id;
    const marker = el.classList.contains("sv-show-move");
    const tag = document.querySelector(`.sv-overlay .sv-tag[data-for="${id}"]` + (marker ? `[data-key=""]` : `:not([data-key])`));
    const fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
    out[id] = {
      state: st(el), hidden: el.classList.contains("sv-hidden"), transform: el.getAttribute("transform"),
      tag: tag ? tag.textContent : null, tag_state: st(tag), tag_at: tag && marker ? [Number(tag.getAttribute("x")), Number(tag.getAttribute("y"))] : null,
      fill: fill ? [Number(fill.getAttribute("y")), Number(fill.getAttribute("height")), st(fill)] : null,
    };
  }
  return out;
})()"#;

/// What the page shows around the drawing: the headline, the attention list, the selection.
const AROUND: &str = r#"(() => ({
  headline: [...document.querySelectorAll(".headline .fig")].map((f) => [f.dataset.object, f.querySelector(".mono").textContent, f.querySelector(".mono").className]),
  attention_head: document.querySelector(".side .attention")?.previousElementSibling?.textContent ?? null,
  attention: [...document.querySelectorAll(".side ul.attention li")].map((li) => [li.dataset.id, li.querySelector(".mark").textContent, li.querySelector(".mark").className, li.querySelector(".grow").textContent]),
  selected: document.querySelector(".side .sel-title strong")?.textContent ?? null,
  badge: document.querySelector(".side .sel-title .badge")?.textContent ?? null,
  value: document.querySelector(".side .facts dd")?.textContent ?? null,
  spark: document.querySelectorAll(".side .spark-wrap svg path, .side .spark-wrap svg polyline").length,
  buttons: [...document.querySelectorAll(".side .actions button")].map((b) => [b.textContent, b.disabled]),
  gate: document.querySelector(".side .gate-note")?.textContent ?? null,
}))()"#;

fn translate(t: &str) -> (f64, f64) {
    let inner = t.strip_prefix("translate(").and_then(|r| r.strip_suffix(')')).unwrap_or_else(|| panic!("{t}"));
    let mut it = inner.split_whitespace().map(|n| n.parse::<f64>().unwrap());
    (it.next().unwrap(), it.next().unwrap())
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Where the drawing differs from the feed (nothing once the page has caught up): each bound
/// element's class and tag against its object's state, mark and word; each level's fill against
/// its level in the element's box; each marker's place against the point on the path.
fn differences(drawn: &Value, feed: &Value, on_layout: &[(String, String)], boxes: &dyn Fn(&str) -> (f64, f64)) -> Vec<String> {
    let mut out = Vec::new();
    for (id, show) in on_layout {
        let d = &drawn[id];
        let v = object(feed, id);
        let state = format!("sv-state-{}", v["state"].as_str().unwrap());
        let said = if v["state"] == "normal" { String::new() } else { format!("{} {}", v["mark"].as_str().unwrap(), v["word"].as_str().unwrap()) };
        if d["state"] != json!(state) || d["tag"] != json!(said) || d["tag_state"] != json!(state) {
            out.push(format!("{id}: drawn {} {} {}, the feed says {state} {said:?}", d["state"], d["tag"], d["tag_state"]));
        }
        match show.as_str() {
            "level" => {
                let (y, h) = boxes(id);
                let f = v["level"].as_f64().unwrap();
                let fill = d["fill"].as_array().cloned().unwrap_or_default();
                let ok = fill.len() == 3 && near(fill[0].as_f64().unwrap(), y + h * (1.0 - f), 1e-6) && near(fill[1].as_f64().unwrap(), h * f, 1e-6) && fill[2] == json!(state);
                if !ok {
                    out.push(format!("{id}: fill {} for level {f} in a box at y {y} of {h}", d["fill"]));
                }
            }
            "move" => {
                let (x, y) = on_loop(v["place"].as_f64().unwrap());
                let ok = d["hidden"] == json!(false) && d["transform"].as_str().map(translate).is_some_and(|(tx, ty)| near(tx, x, 0.05) && near(ty, y, 0.05));
                let tag_ok = d["tag_at"].as_array().is_some_and(|a| near(a[0].as_f64().unwrap(), x, 0.05) && a[1].as_f64().unwrap() > y);
                if !ok || !tag_ok {
                    out.push(format!("{id}: at {} (tag at {}), the path's point is ({x:.2}, {y:.2})", d["transform"], d["tag_at"]));
                }
            }
            _ => {}
        }
    }
    out
}

/// Every colour the bundle spells in hex or `rgb()`, as (spelling, hue in degrees, saturation).
fn colours(text: &str) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let hsl = |r: f64, g: f64, bl: f64| {
        let (mx, mn) = (r.max(g).max(bl), r.min(g).min(bl));
        let l = (mx + mn) / 2.0;
        let d = mx - mn;
        if d == 0.0 {
            return (0.0, 0.0);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if mx == r { 60.0 * (((g - bl) / d) % 6.0) } else if mx == g { 60.0 * ((bl - r) / d + 2.0) } else { 60.0 * ((r - g) / d + 4.0) };
        ((h + 360.0) % 360.0, s)
    };
    for i in 0..b.len() {
        if b[i] == b'#' {
            let n = b[i + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
            let ends = b.get(i + 1 + n).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_' && *c != b'-');
            if ends && matches!(n, 3 | 4 | 6 | 8) {
                let hex = &text[i + 1..i + 1 + n];
                let ch = |k: usize| if n <= 4 { u8::from_str_radix(&hex[k..k + 1].repeat(2), 16).unwrap() } else { u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).unwrap() };
                let (h, s) = hsl(ch(0) as f64 / 255.0, ch(1) as f64 / 255.0, ch(2) as f64 / 255.0);
                out.push((format!("#{hex}"), h, s));
            }
        }
        if b[i..].starts_with(b"rgb(") || b[i..].starts_with(b"rgba(") {
            // Byte offsets of ASCII characters: always character boundaries.
            let open = i + b[i..].iter().position(|c| *c == b'(').unwrap();
            let Some(close) = b[open..].iter().position(|c| *c == b')').map(|p| open + p) else { continue };
            let inner = &text[open + 1..close];
            let n: Vec<f64> = inner.split(|c: char| c == ',' || c.is_whitespace() || c == '/').filter_map(|x| x.parse().ok()).collect();
            if n.len() >= 3 {
                let (h, s) = hsl(n[0] / 255.0, n[1] / 255.0, n[2] / 255.0);
                out.push((format!("rgb({inner})"), h, s));
            }
        }
    }
    out
}

#[test]
fn the_page_draws_the_layout_from_the_feed() {
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
    let on_layout: Vec<(String, String)> = b["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| svg.contains(&format!("id=\"{}\"", o["id"].as_str().unwrap())))
        .map(|o| (o["id"].as_str().unwrap().to_string(), o["show"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(on_layout.iter().map(|(i, s)| (i.as_str(), s.as_str())).collect::<Vec<_>>(), [
        ("infeed", "state"), ("outlet-1", "level"), ("outlet-2", "level"), ("outlet-3", "level"), ("parcel-1", "move"), ("parcel-2", "move"),
    ]);
    assert!(b["objects"].as_array().unwrap().iter().filter(|o| o["show"] == "move").all(|o| o["path"] == "loop" && svg.contains("<path id=\"loop\"")));
    // The outlets' boxes, as the drawing states them.
    let boxes = |id: &str| -> (f64, f64) {
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let el = doc.descendants().find(|n| n.attribute("id") == Some(id)).unwrap();
        (el.attribute("y").unwrap().parse().unwrap(), el.attribute("height").unwrap().parse().unwrap())
    };

    // The feed carries, for each, what its way of showing needs.
    assert!(wait(Duration::from_secs(20), || {
        let (_, s) = get(addr, "/s/loop/objects.json");
        object(&s, "parcel-1")["place"] == json!(0.25) && object(&s, "outlet-2")["level"] == json!(0.85) && object(&s, "infeed")["word"] == json!("running")
    }));

    // The page, run against the session, draws it: read back until it agrees with the feed.
    let mut page = Browser::open(&format!("http://{addr}/s/loop/"));
    let settle = |page: &mut Browser, what: &str| -> (Value, Value) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let feed = get(addr, "/s/loop/objects.json").1;
            let drawn = page.eval(DRAWN);
            let diff = differences(&drawn, &feed, &on_layout, &boxes);
            if diff.is_empty() {
                return (drawn, feed);
            }
            assert!(Instant::now() < deadline, "{what}: the page never drew the feed: {diff:#?}");
            std::thread::sleep(Duration::from_millis(150));
        }
    };
    let (drawn, _) = settle(&mut page, "first paint");
    // The anchors, spelled out: the states the reference's rules give the bench's values, the
    // fills, and the markers at a quarter and three quarters of the loop.
    let d = |id: &str, k: &str| drawn[id][k].clone();
    assert_eq!((d("infeed", "state"), d("infeed", "tag")), (json!("sv-state-normal"), json!("")));
    assert_eq!((d("outlet-1", "state"), d("outlet-2", "state"), d("outlet-3", "state")), (json!("sv-state-normal"), json!("sv-state-near"), json!("sv-state-warn")));
    assert_eq!((d("outlet-2", "tag"), d("outlet-3", "tag")), (json!("■ near full"), json!("■ full")));
    let fill = |id: &str| (d(id, "fill")[0].as_f64().unwrap(), d(id, "fill")[1].as_f64().unwrap());
    assert!(near(fill("outlet-1").1, 9.8, 1e-6) && near(fill("outlet-2").0, 8.2, 1e-6) && near(fill("outlet-2").1, 23.8, 1e-6) && near(fill("outlet-3").0, 4.0, 1e-6) && near(fill("outlet-3").1, 28.0, 1e-6), "{drawn}");
    let at = |id: &str| translate(d(id, "transform").as_str().unwrap());
    assert!(near(at("parcel-1").0, 309.96, 0.05) && near(at("parcel-1").1, 40.0, 0.05), "{:?}", at("parcel-1"));
    assert!(near(at("parcel-2").0, 90.04, 0.05) && near(at("parcel-2").1, 180.0, 0.05), "{:?}", at("parcel-2"));

    // Around it: the headline in its states, the objects not normal in order of weight.
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.attention.length === 3 && a.headline.length === 2 ? a : null; }})()"), Duration::from_secs(10));
    assert_eq!(around["headline"], json!([["speed", "1.5", "mono st-text-normal"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention_head"], json!("Needs attention · 3"));
    assert_eq!(around["attention"], json!([
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
    ]));

    // A click selects: the side panel holds its value, state, sparkline and offered write, gated
    // by portal's switch; the selection is drawn in Veredas' green (Q22).
    page.eval(r#"document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true }))"#);
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.selected === 'Outlet 3' && a.spark > 0 ? a : null; }})()"), Duration::from_secs(20));
    assert_eq!((around["badge"].clone(), around["value"].clone()), (json!("■ full"), json!("100 %")));
    assert_eq!(around["buttons"], json!([["Release", true]]), "offered, and off while the switch is off");
    assert_eq!(around["gate"], json!("Writes go through portal. Its write switch is off."));
    host.session("loop").unwrap().set_writes_armed(true);
    page.wait_for(r#"(() => { const b = document.querySelector(".side .actions button"); return b && !b.disabled; })()"#, Duration::from_secs(10));
    host.session("loop").unwrap().set_writes_armed(false);
    page.wait_for(r#"document.querySelector(".side .actions button").disabled"#, Duration::from_secs(10));
    let accent = page.eval(
        r#"(() => {
  const sel = document.getElementById("outlet-3");
  const green = "rgb(48, 209, 88)";
  const states = [...document.querySelectorAll("svg.plant .sv-obj:not(.sv-selected), svg.plant .sv-fill, .sv-overlay .sv-tag")]
    .filter((el) => { const c = getComputedStyle(el); return c.fill === green || c.stroke === green; }).map((el) => el.id || el.dataset.for);
  return { selected: sel.classList.contains("sv-selected"), stroke: getComputedStyle(sel).stroke, accent: getComputedStyle(sel).getPropertyValue("--accent").trim(), states };
})()"#,
    );
    assert_eq!(accent, json!({ "selected": true, "stroke": "rgb(48, 209, 88)", "accent": "#30d158", "states": [] }), "the accent marks the selection and never a state");

    // Moved by the plant, the marker moves along its path, its tag with it.
    bench.server("plc").unwrap().set("p1", 60.0);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "parcel-1")["place"] == json!(0.6)));
    let (drawn, _) = settle(&mut page, "the marker moved");
    let (x, y) = translate(drawn["parcel-1"]["transform"].as_str().unwrap());
    assert!(near(x, 228.02, 0.05) && near(y, 180.0, 0.05), "{x} {y}");

    // The plant away: every object stale, drawn so, a marker's mark and word below it.
    bench.stop();
    assert!(wait(Duration::from_secs(40), || {
        let s = get(addr, "/s/loop/objects.json").1;
        on_layout.iter().all(|(id, _)| object(&s, id)["state"] == json!("stale"))
    }));
    let (drawn, feed) = settle(&mut page, "the plant away");
    assert_eq!(object(&feed, "parcel-1")["state"], json!("stale"));
    assert_eq!(drawn["parcel-1"]["tag"], json!(format!("{} {}", object(&feed, "parcel-1")["mark"].as_str().unwrap(), object(&feed, "parcel-1")["word"].as_str().unwrap())));
    assert!(drawn["parcel-1"]["tag"].as_str().unwrap().starts_with("◌ "), "{}", drawn["parcel-1"]);
    drop(page);
    host.stop();

    // The bundle reads the session's binding, layout and feed, and styles every class.
    for needle in ["/binding", "/layout.svg", "/objects", "getPointAtLength", "Needs attention"] {
        assert!(web::APP_JS.contains(needle), "{needle}");
    }
    for class in ["normal", "near", "warn", "alarm", "disabled", "stale"] {
        assert!(web::APP_CSS.contains(&format!("sv-state-{class}")), "{class}");
    }
    // Veredas' one green accent (Q22), the desktop's: never the mockups' magenta, by name (the
    // mockups' six, the text tint on it among them) or by hue (no colour of the bundle between
    // purple and pink).
    assert!(web::APP_CSS.contains("--accent:#30d158") || web::APP_CSS.contains("--accent: #30d158"), "the accent is Veredas' green");
    for magenta in ["ac007c", "d94ba0", "3a0b2c", "6e1a54", "e86fb5", "e8b9d6"] {
        assert!(!web::APP_CSS.to_ascii_lowercase().contains(magenta) && !web::APP_JS.to_ascii_lowercase().contains(magenta), "{magenta}");
    }
    let found: Vec<(String, f64, f64)> = colours(web::APP_CSS).into_iter().chain(colours(web::APP_JS)).collect();
    assert!(found.iter().any(|(c, _, _)| c == "#30d158"), "{found:?}");
    let pink: Vec<&(String, f64, f64)> = found.iter().filter(|(_, h, s)| (280.0..=345.0).contains(h) && *s > 0.15).collect();
    assert_eq!(pink, Vec::<&(String, f64, f64)>::new());
    // The reader sees the mockups' colours in either spelling.
    for c in ["#ac007c", "#E8B9D6", "#d4b", "rgb(172, 0, 124)", "rgba(232,185,214,0.5)"] {
        assert!(colours(c).iter().any(|(_, h, s)| (280.0..=345.0).contains(h) && *s > 0.15), "{c}: {:?}", colours(c));
    }
    assert!(colours("#30d158 #sv-hatch url(#loop) #26262A").iter().all(|(_, h, _)| !(280.0..=345.0).contains(h)));
}
