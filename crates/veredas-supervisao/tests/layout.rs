//! C-051: the 2D layout. Everything the page binds is served (the SVG as written, every bound
//! element's object in the binding and in the feed with what its way of showing needs), and the
//! page itself, run in a headless browser against the same session, draws it: every bound
//! element's state as a class with its mark and word beside it (a marker's below it, moving with
//! it, one per key of a keyed marker), every level's fill to its height inside its element, every
//! marker at its place along its path (the point computed here from the drawing's own geometry),
//! all of it visible and placed on screen where its element stands, moved and grouped elements
//! too, a level bound to a group filling its box, every fill and marker hit where it is drawn, a
//! key no longer moving dropped with its tag; the objects not normal in the attention list in order of weight, alarm to stale; the
//! headline; a selected object's value (a key's own for a keyed marker), sparkline (its own series
//! over its half hour) and offered write; all read back from the page's document against the feed.
//! Veredas' green accent marks the selection and no state, after every change; no magenta
//! anywhere in the bundle.

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

/// What the page drew for every bound element and every keyed marker's clone, read from its
/// document: classes, texts and transforms, the screen box of each element, its fill, tag and
/// label, and whether each is shown (displayed, visible, not transparent).
const DRAWN: &str = r#"(() => {
  const st = (el) => el ? ([...el.classList].find((c) => c.startsWith("sv-state-")) || "") : null;
  const rect = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
  const shown = (el) => { if (!el) return null; const c = getComputedStyle(el); return c.display !== "none" && c.visibility === "visible" && Number(c.opacity) > 0; };
  // Whether the thing is what is painted at its box's centre (clips, visibility and anything over
  // it count; a fill takes no pointer, so it is asked to for the probe).
  const hit = (el) => {
    if (!el || el.classList.contains("sv-hidden")) return null;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) return null;
    const prev = el.style.pointerEvents;
    el.style.pointerEvents = "visiblePainted";
    const at = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    el.style.pointerEvents = prev;
    return at === el || el.contains(at);
  };
  const tagOf = (id, key) => document.querySelector(`.sv-overlay .sv-tag[data-for="${id}"]` + (key === null ? `:not([data-key])` : `[data-key="${key}"]`));
  const out = {};
  for (const el of document.querySelectorAll("svg.plant [data-object][id]")) {
    const id = el.id;
    const marker = el.classList.contains("sv-show-move");
    const tag = tagOf(id, marker ? "" : null);
    const label = document.querySelector(`.sv-overlay .sv-label[data-for="${id}"]`);
    const fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
    out[id] = {
      state: st(el), hidden: el.classList.contains("sv-hidden"), transform: el.getAttribute("transform"), rect: rect(el), shown: shown(el), hit: marker ? hit(el) : null,
      tag_keys: [...document.querySelectorAll(`.sv-overlay .sv-tag[data-for="${id}"][data-key]`)].map((t) => t.dataset.key).sort(),
      tag: tag ? tag.textContent : null, tag_state: st(tag), tag_rect: rect(tag), tag_shown: shown(tag), label_rect: rect(label),
      fill: fill ? { rect: rect(fill), state: st(fill), shown: shown(fill), hit: hit(fill) } : null,
      clones: [...document.querySelectorAll(`svg.plant [data-object="${id}"][data-key]`)].map((c) => {
        const t = tagOf(id, c.dataset.key);
        return { key: c.dataset.key, state: st(c), transform: c.getAttribute("transform"), rect: rect(c), shown: shown(c), hit: hit(c), selected: c.classList.contains("sv-selected"), tag: t ? t.textContent : null, tag_state: st(t), tag_rect: rect(t) };
      }),
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

/// Every drawn thing whose fill or stroke is one of the accent's colours (the accent, its tint,
/// its soft and its outline, compared by red, green and blue whatever the alpha), the selection
/// apart: none may be a state's.
const ACCENT_ON_STATES: &str = r##"(() => {
  const root = getComputedStyle(document.documentElement);
  const hex = (h) => { h = h.trim().replace("#", ""); if (h.length === 3) h = [...h].map((c) => c + c).join(""); return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)).join(","); };
  const family = ["--accent", "--accent-tint", "--accent-soft", "--accent-outline"].map((v) => hex(root.getPropertyValue(v)));
  const rgb = (c) => { const m = c.match(/rgba?\(([^)]*)\)/); return m ? m[1].split(",").slice(0, 3).map((x) => Math.round(Number(x))).join(",") : null; };
  return [...document.querySelectorAll("svg.plant .sv-obj, svg.plant [data-key], svg.plant .sv-fill, .sv-overlay .sv-tag")]
    .filter((el) => !el.classList.contains("sv-selected"))
    .flatMap((el) => { const c = getComputedStyle(el); return [["fill", c.fill], ["stroke", c.stroke]].filter(([, v]) => family.includes(rgb(v))).map(([p, v]) => `${el.id || el.dataset.for || el.dataset.key} ${p} ${v}`); });
})()"##;

/// The accent check once every transition has run: a fill or stroke sliding from the selection's
/// green is not a state drawn in it.
fn accent_on_states(page: &mut Browser) -> Value {
    page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running")"#, Duration::from_secs(5));
    page.eval(ACCENT_ON_STATES)
}

fn translate(t: &str) -> (f64, f64) {
    let inner = t.strip_prefix("translate(").and_then(|r| r.strip_suffix(')')).unwrap_or_else(|| panic!("{t}"));
    let mut it = inner.split_whitespace().map(|n| n.parse::<f64>().unwrap());
    (it.next().unwrap(), it.next().unwrap())
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// A screen box as (left, top, right, bottom, centre x).
fn edges(r: &Value) -> Option<(f64, f64, f64, f64, f64)> {
    let a = r.as_array()?;
    let (x, y, w, h) = (a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?, a[3].as_f64()?);
    Some((x, y, x + w, y + h, x + w / 2.0))
}

/// Where a tag stands against the box it marks: centred under it, just below it.
fn under(tag: &Value, el: &Value) -> bool {
    match (edges(tag), edges(el)) {
        (Some((_, tt, _, _, tc)), Some((_, _, _, eb, ec))) => near(tc, ec, 3.0) && tt >= eb - 1.0 && tt - eb <= 40.0,
        _ => false,
    }
}

/// Where the drawing differs from the feed (nothing once the page has caught up): each bound
/// element's class and tag against its object's state, mark and word; each tag and label placed
/// on screen at its element and shown; each level's fill inside its element to its level; each
/// marker at the point on the path, its tag under it; a keyed marker's clones, one per key.
fn differences(drawn: &Value, feed: &Value, on_layout: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (id, show) in on_layout {
        let d = &drawn[id];
        let v = object(feed, id);
        let state = format!("sv-state-{}", v["state"].as_str().unwrap());
        let said = if v["state"] == "normal" { String::new() } else { format!("{} {}", v["mark"].as_str().unwrap(), v["word"].as_str().unwrap()) };
        let keyed = !v["items"].as_array().unwrap().is_empty();
        // A marker with no place is hidden and has no tag: the attention list carries it.
        let placed = show != "move" || !v["place"].is_null();
        if !keyed && placed && (d["state"] != json!(state) || d["tag"] != json!(said) || d["tag_state"] != json!(state)) {
            out.push(format!("{id}: drawn {} {} {}, the feed says {state} {said:?}", d["state"], d["tag"], d["tag_state"]));
        }
        if !keyed && !placed && (d["state"] != json!(state) || !d["tag"].is_null() || d["tag_keys"] != json!([])) {
            out.push(format!("{id}: no place, drawn {} with a tag {} and tags for {} over {} clones", d["state"], d["tag"], d["tag_keys"], d["clones"].as_array().map_or(0, |c| c.len())));
        }
        if show != "move" {
            if d["shown"] != json!(true) {
                out.push(format!("{id}: not shown"));
            }
            if !said.is_empty() && !(under(&d["tag_rect"], &d["rect"]) && d["tag_shown"] == json!(true)) {
                out.push(format!("{id}: its tag at {} is not shown under it at {}", d["tag_rect"], d["rect"]));
            }
            let label_over = matches!((edges(&d["label_rect"]), edges(&d["rect"])), (Some((_, _, _, lb, lc)), Some((_, et, _, _, ec))) if near(lc, ec, 3.0) && lb <= et + 2.0);
            if !label_over {
                out.push(format!("{id}: its label at {} is not over it at {}", d["label_rect"], d["rect"]));
            }
        }
        match show.as_str() {
            "level" => {
                let f = v["level"].as_f64().unwrap();
                let fill = &d["fill"];
                let ok = match (edges(&fill["rect"]), edges(&d["rect"])) {
                    (Some((fl, ft, fr, fb, _)), Some((el, et, er, eb, _))) => near(fl, el, 1.0) && near(fr, er, 1.0) && near(fb, eb, 1.0) && near(fb - ft, f * (eb - et), 1.0),
                    _ => false,
                };
                let painted = fill["hit"] == json!(true) || edges(&fill["rect"]).is_some_and(|(_, ft, _, fb, _)| fb - ft < 2.0);
                if !ok || fill["state"] != json!(state) || (f > 0.0 && (fill["shown"] != json!(true) || !painted)) {
                    out.push(format!("{id}: fill {fill} for level {f} in {}", d["rect"]));
                }
            }
            "move" if !keyed && v["place"].is_null() => {
                if d["hidden"] != json!(true) {
                    out.push(format!("{id}: drawn though it has no place"));
                }
            }
            "move" if !keyed => {
                let (x, y) = on_loop(v["place"].as_f64().unwrap());
                let ok = d["hidden"] == json!(false) && d["shown"] == json!(true) && d["hit"] == json!(true) && d["tag_keys"] == json!([""]) && d["transform"].as_str().map(translate).is_some_and(|(tx, ty)| near(tx, x, 0.05) && near(ty, y, 0.05));
                let tag_ok = said.is_empty() || (under(&d["tag_rect"], &d["rect"]) && d["tag_shown"] == json!(true));
                if !ok || !tag_ok {
                    out.push(format!("{id}: at {} (its tag {} at {}), the path's point is ({x:.2}, {y:.2})", d["transform"], d["tag"], d["tag_rect"]));
                }
            }
            "move" => {
                // One clone per key at its own place, in the object's state, its tag under it.
                let items = v["items"].as_array().unwrap();
                let clones = d["clones"].as_array().unwrap();
                let mut keys: Vec<&str> = items.iter().map(|i| i["key"].as_str().unwrap()).collect();
                keys.sort();
                if clones.len() != items.len() || d["tag_keys"] != json!(keys) {
                    out.push(format!("{id}: {} clones and tags for {} for the keys {keys:?}", clones.len(), d["tag_keys"]));
                }
                for it in items {
                    let key = it["key"].as_str().unwrap();
                    let (x, y) = on_loop(it["place"].as_f64().unwrap());
                    let ok = clones.iter().find(|c| c["key"] == json!(key)).is_some_and(|c| {
                        c["state"] == json!(state) && c["shown"] == json!(true) && c["hit"] == json!(true) && c["transform"].as_str().map(translate).is_some_and(|(tx, ty)| near(tx, x, 0.05) && near(ty, y, 0.05)) && c["tag"] == json!(said) && (said.is_empty() || under(&c["tag_rect"], &c["rect"]))
                    });
                    if !ok {
                        out.push(format!("{id} {key}: drawn {:?}, the path's point is ({x:.2}, {y:.2})", clones.iter().find(|c| c["key"] == json!(key))));
                    }
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

/// The reference's example grown for this test: a keyed marker over the log's positions (a key
/// forgotten 6 s after its last), a disabled state for outlet 1 and its element a group, outlet 2
/// moved by its own transform and outlet 3 inside a moved group.
fn grown(mut ex: Example) -> Example {
    ex.project.push_str("\n[[rule]]\nname = \"positions\"\npattern = 'POS item=(?P<item>\\w+) at=(?P<at>[0-9.]+)'\nscope = \"sorter\"\ntypes = { at = \"float\" }\nkey = \"item\"\n");
    ex.binding = ex
        .binding
        .replacen("states = [ { at_least = 100, state = \"warn\", word = \"full\" }", "states = [ { at_most = 0, state = \"disabled\", word = \"off\" }, { at_least = 100, state = \"warn\", word = \"full\" }", 1)
        .replacen("\n[[dashboard]]", "\n[[object]]\nid = \"tracked\"\nlabel = \"Tracked\"\nvalue = { source = \"sorter\", series = \"at\" }\nshow = \"move\"\npath = \"loop\"\nstale_after_ms = 6000\n\n[[dashboard]]", 1);
    ex.svg = ex
        .svg
        .replace("<rect id=\"outlet-1\" x=\"130\" y=\"4\" width=\"24\" height=\"28\"/>", "<g id=\"outlet-1\"><rect x=\"130\" y=\"4\" width=\"24\" height=\"28\"/></g>")
        .replace("<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\"/>", "<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\" transform=\"translate(-20 110)\"/>")
        .replace("<rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/>", "<g transform=\"translate(0 100)\"><rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/></g>")
        .replace("<circle id=\"parcel-2\" r=\"5\"/>", "<circle id=\"parcel-2\" r=\"5\"/>\n  <circle id=\"tracked\" r=\"4\"/>");
    ex
}

#[test]
fn the_page_draws_the_layout_from_the_feed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let ex = grown(live_example(&bench, false));
    assert!(ex.svg.contains("translate(-20 110)") && ex.svg.contains("<g transform") && ex.svg.contains("<g id=\"outlet-1\">") && ex.svg.contains("id=\"tracked\""), "{}", ex.svg);
    let path = write_example(dir.path(), &ex);
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();

    // The drawing as written, its elements carrying the objects' ids.
    let (status, headers, svg) = http(addr, "GET", "/s/loop/layout.svg", None);
    assert_eq!((status, svg.replace("\r\n", "\n")), (200, ex.svg.clone()));
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
        ("infeed", "state"), ("outlet-1", "level"), ("outlet-2", "level"), ("outlet-3", "level"), ("parcel-1", "move"), ("parcel-2", "move"), ("tracked", "move"),
    ]);

    // The feed carries, for each, what its way of showing needs.
    assert!(wait(Duration::from_secs(20), || {
        let (_, s) = get(addr, "/s/loop/objects.json");
        object(&s, "parcel-1")["place"] == json!(0.25) && object(&s, "outlet-2")["level"] == json!(0.85) && object(&s, "infeed")["word"] == json!("running")
    }));

    // The page, run against the session, draws it: read back until it agrees with the feed, and
    // after each change the accent marks no state.
    let mut page = Browser::watched(&format!("http://{addr}/s/loop/"));
    let settle = |page: &mut Browser, what: &str| -> (Value, Value) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let feed = get(addr, "/s/loop/objects.json");
            let drawn = page.eval(DRAWN);
            let diff = differences(&drawn, &feed.1, &on_layout);
            if diff.is_empty() {
                let accent = accent_on_states(page);
                assert_eq!(accent, json!([]), "{what}: a state drawn in the accent");
                return (drawn, feed.1);
            }
            assert!(Instant::now() < deadline, "{what}: the page never drew the feed: {diff:#?}");
            std::thread::sleep(Duration::from_millis(150));
        }
    };
    let (drawn, _) = settle(&mut page, "first paint");
    // The anchors, spelled out: the states the rules give the bench's values, and the markers at
    // a quarter and three quarters of the loop.
    let d = |id: &str, k: &str| drawn[id][k].clone();
    assert_eq!((d("infeed", "state"), d("infeed", "tag")), (json!("sv-state-normal"), json!("")));
    assert_eq!((d("outlet-1", "state"), d("outlet-2", "state"), d("outlet-3", "state")), (json!("sv-state-normal"), json!("sv-state-near"), json!("sv-state-warn")));
    assert_eq!((d("outlet-2", "tag"), d("outlet-3", "tag")), (json!("■ near full"), json!("■ full")));
    let at = |id: &str| translate(d(id, "transform").as_str().unwrap());
    assert!(near(at("parcel-1").0, 309.96, 0.05) && near(at("parcel-1").1, 40.0, 0.05), "{:?}", at("parcel-1"));
    assert!(near(at("parcel-2").0, 90.04, 0.05) && near(at("parcel-2").1, 180.0, 0.05), "{:?}", at("parcel-2"));
    assert_eq!((d("tracked", "hidden"), d("tracked", "clones")), (json!(true), json!([])), "no position yet");

    // Around it: the headline in its states, the objects not normal in order of weight.
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.attention.length === 4 && a.headline.length === 2 ? a : null; }})()"), Duration::from_secs(10));
    assert_eq!(around["headline"], json!([["speed", "1.5", "mono st-text-normal"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention_head"], json!("Needs attention · 4"));
    assert_eq!(around["attention"], json!([
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
    ]));

    // A click selects: the side panel holds its value, state, sparkline (its own series over its
    // last half hour) and offered write, gated by portal's switch; the selection is drawn in
    // Veredas' green (Q22).
    page.eval(r#"document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true }))"#);
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.selected === 'Outlet 3' && a.spark > 0 ? a : null; }})()"), Duration::from_secs(20));
    assert_eq!((around["badge"].clone(), around["value"].clone()), (json!("■ full"), json!("100 %")));
    assert_eq!(around["buttons"], json!([["Release", true]]), "offered, and off while the switch is off");
    assert_eq!(around["gate"], json!("Writes go through portal. Its write switch is off."));
    let sparks: Vec<String> = page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?")).collect();
    let query = |u: &str, k: &str| u.split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(String::from);
    assert!(
        sparks.iter().any(|u| query(u, "source").as_deref() == Some("plc") && query(u, "series").as_deref() == Some("fill3") && query(u, "to").zip(query(u, "from")).is_some_and(|(t, f)| t.parse::<i64>().unwrap() - f.parse::<i64>().unwrap() == 1_800_000)),
        "the sparkline reads outlet 3's own series over half an hour: {sparks:?}"
    );
    host.session("loop").unwrap().set_writes_armed(true);
    page.wait_for(r#"(() => { const b = document.querySelector(".side .actions button"); return b && !b.disabled; })()"#, Duration::from_secs(10));
    host.session("loop").unwrap().set_writes_armed(false);
    page.wait_for(r#"document.querySelector(".side .actions button").disabled"#, Duration::from_secs(10));
    let accent = page.eval(
        r#"(() => {
  const sel = document.getElementById("outlet-3");
  return { selected: sel.classList.contains("sv-selected"), stroke: getComputedStyle(sel).stroke, accent: getComputedStyle(sel).getPropertyValue("--accent").trim() };
})()"#,
    );
    assert_eq!(accent, json!({ "selected": true, "stroke": "rgb(48, 209, 88)", "accent": "#30d158" }), "the accent marks the selection");
    assert_eq!(accent_on_states(&mut page), json!([]), "and never a state");

    // Every state the vocabulary has, the attention list in order of weight: the infeed in fault
    // (alarm), outlet 1 empty (disabled).
    bench.server("plc").unwrap().set("infeed", 2);
    bench.server("plc").unwrap().set("fill1", 0.0);
    assert!(wait(Duration::from_secs(20), || {
        let s = get(addr, "/s/loop/objects.json").1;
        object(&s, "infeed")["state"] == json!("alarm") && object(&s, "outlet-1")["state"] == json!("disabled")
    }));
    settle(&mut page, "an alarm and a disabled object");
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.attention.length === 6 ? a : null; }})()"), Duration::from_secs(10));
    assert_eq!(around["attention"], json!([
        ["infeed", "!", "mark st-alarm", "Infeed fault"],
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["outlet-1", "▨", "mark st-disabled", "Outlet 1 off"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
    ]));

    // A keyed marker: one per key, each at its own place, its tag under it; one selected by a
    // click, alone, its own value in the side panel.
    let now = store_now();
    append(&log, &[format!("{} POS item=A1 at=10", stamp(now)), format!("{} POS item=B2 at=50", stamp(now))]);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "tracked")["items"].as_array().unwrap().len() == 2));
    let (drawn, _) = settle(&mut page, "the keyed marker");
    let keys: Vec<Value> = drawn["tracked"]["clones"].as_array().unwrap().iter().map(|c| c["key"].clone()).collect();
    assert_eq!((drawn["tracked"]["hidden"].clone(), keys.len()), (json!(true), 2), "{keys:?}");
    page.eval(r#"document.querySelector('svg.plant [data-object="tracked"][data-key="A1"]').dispatchEvent(new MouseEvent("click", { bubbles: true }))"#);
    let around = page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.selected === 'Tracked · A1' ? a : null; }})()"), Duration::from_secs(10));
    assert_eq!(around["value"], json!("10"), "A1's own value, not the latest of any key");
    let picked: Vec<(Value, Value)> = page.eval(DRAWN)["tracked"]["clones"].as_array().unwrap().iter().map(|c| (c["key"].clone(), c["selected"].clone())).collect();
    assert!(picked.contains(&(json!("A1"), json!(true))) && picked.contains(&(json!("B2"), json!(false))), "{picked:?}");
    assert_eq!(accent_on_states(&mut page), json!([]));
    // A1 moves on, B2 says nothing more: B2 is dropped with its tag once its 6 s pass, A1 drawn at
    // its new place, still selected, its new value beside.
    // Two and a half seconds after the first two, so A1 outlives B2 by as much.
    std::thread::sleep(Duration::from_millis(2500));
    append(&log, &[format!("{} POS item=A1 at=30", stamp(store_now()))]);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "tracked")["items"] == json!([{ "key": "A1", "place": 0.3, "text": "30" }])));
    let (drawn, _) = settle(&mut page, "a key forgotten");
    let left: Vec<(Value, Value)> = drawn["tracked"]["clones"].as_array().unwrap().iter().map(|c| (c["key"].clone(), c["selected"].clone())).collect();
    assert_eq!((left, drawn["tracked"]["tag_keys"].clone()), (vec![(json!("A1"), json!(true))], json!(["A1"])));
    page.wait_for(&format!("(() => {{ const a = {AROUND}; return a.selected === 'Tracked · A1' && a.value === '30'; }})()"), Duration::from_secs(10));

    // Moved by the plant, the marker moves along its path, its tag with it.
    bench.server("plc").unwrap().set("p1", 60.0);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "parcel-1")["place"] == json!(0.6)));
    let (drawn, _) = settle(&mut page, "the marker moved");
    let (x, y) = translate(drawn["parcel-1"]["transform"].as_str().unwrap());
    assert!(near(x, 228.02, 0.05) && near(y, 180.0, 0.05), "{x} {y}");

    // The plant away: every object of its source stale, drawn so, a marker's mark and word below it.
    bench.stop();
    assert!(wait(Duration::from_secs(40), || {
        let s = get(addr, "/s/loop/objects.json").1;
        on_layout.iter().filter(|(id, _)| id != "tracked").all(|(id, _)| object(&s, id)["state"] == json!("stale"))
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
