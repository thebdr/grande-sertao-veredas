//! C-051: the 2D layout. Everything the page binds is served (the SVG as written, every bound
//! element's object in the binding and in the feed with what its way of showing needs), and the
//! page itself, run in a headless browser against the same session, draws it: every bound
//! element's state as a class with its mark and word beside it (a marker's below it, moving with
//! it, one per key of a keyed marker), every level's fill to its height inside its element, every
//! marker at its place along its path (the point computed here from the drawing's own geometry and
//! carried to the screen through the path's own transforms), all of it visible and placed on
//! screen where its element stands, moved and grouped elements and a moved path too, each label
//! shown just over its element, a level bound to a group filling its box, every fill and marker
//! hit where it is drawn, a key no longer moving dropped with its tag; the objects not normal in
//! the attention list in order of weight, alarm to stale, and an object leaving it once normal;
//! the headline, live; a selected object, chosen by a pointer at a point, alone in carrying the
//! selection, with its value (a key's own for a keyed marker), sparkline (its own series over its
//! half hour) and offered write; all read back from the page's document against the feed.
//! Veredas' green accent marks the selection and no state, in the drawing, the headline, the
//! attention list or a badge, nor any green of its hue, after every change; no magenta anywhere in
//! the bundle.

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
/// label, whether each is shown (displayed, visible, not transparent) and selected; and, as
/// `__loop`, the loop's own matrix to the screen.
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
      state: st(el), hidden: el.classList.contains("sv-hidden"), transform: el.getAttribute("transform"), rect: rect(el), shown: shown(el), hit: marker ? hit(el) : null, selected: el.classList.contains("sv-selected"),
      tag_keys: [...document.querySelectorAll(`.sv-overlay .sv-tag[data-for="${id}"][data-key]`)].map((t) => t.dataset.key).sort(),
      tag: tag ? tag.textContent : null, tag_state: st(tag), tag_rect: rect(tag), tag_shown: shown(tag), label_rect: rect(label), label_shown: shown(label),
      fill: fill ? { rect: rect(fill), state: st(fill), shown: shown(fill), hit: hit(fill) } : null,
      clones: [...document.querySelectorAll(`svg.plant [data-object="${id}"][data-key]`)].map((c) => {
        const t = tagOf(id, c.dataset.key);
        return { key: c.dataset.key, state: st(c), transform: c.getAttribute("transform"), rect: rect(c), shown: shown(c), hit: hit(c), selected: c.classList.contains("sv-selected"), tag: t ? t.textContent : null, tag_state: st(t), tag_rect: rect(t), tag_shown: shown(t) };
      }),
    };
  }
  const loop = document.getElementById("loop");
  const m = loop ? loop.getScreenCTM() : null;
  out.__loop = m ? [m.a, m.b, m.c, m.d, m.e, m.f] : null;
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

/// Every state drawn in a green of the accent's: the drawing's fills and strokes (the selection
/// apart, which the selection check pins to the one selected), the headline's figures, the
/// attention list's marks and the side panel's badges by their colour and background. A green is
/// one of the accent's four colours (the accent, its tint, its soft and its outline, by red, green
/// and blue whatever the alpha) or any colour of the accent's band of hue (100° to 170°) with some
/// saturation (above 0.2): a shade one unit off the accent is still the accent to the eye. A fill or
/// stroke painted by a pattern or a gradient (`url(#id)`) is read through the colours it paints
/// with: the fills, strokes and stop colours of the element it names and everything inside it.
/// Every colour is read as the browser paints it (a 1×1 canvas filled with it, its bytes read
/// back), whatever syntax the browser reports it in (`rgb()`, `color(srgb ...)` from a
/// `color-mix()`, `oklch()`, `lab()`). The drawing's shapes are read inside the bound elements too
/// (a group's own fill is not what paints), anything inside the selection apart; and every selected
/// element, a keyed marker's clone among them, must stroke in the accent itself.
const ACCENT_ON_STATES: &str = r##"(() => {
  const root = getComputedStyle(document.documentElement);
  const hex = (h) => { h = h.trim().replace("#", ""); if (h.length === 3) h = [...h].map((c) => c + c).join(""); return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)).join(","); };
  const family = ["--accent", "--accent-tint", "--accent-soft", "--accent-outline"].map((v) => hex(root.getPropertyValue(v)));
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const parts = (c) => {
    const sentinel = "rgba(1, 2, 3, 0.5)";
    ctx.fillStyle = sentinel;
    const before = ctx.fillStyle;
    ctx.fillStyle = c;
    if (ctx.fillStyle === before && c.replace(/\s/g, "") !== sentinel.replace(/\s/g, "")) return null;
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    return [d[0], d[1], d[2], d[3] / 255];
  };
  const green = (c) => {
    const p = parts(c);
    if (!p || p[3] === 0) return false;
    if (family.includes(p.slice(0, 3).map(Math.round).join(","))) return true;
    const [r, g, b] = p.slice(0, 3).map((x) => x / 255);
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn, l = (mx + mn) / 2;
    if (d === 0) return false;
    const s = d / (1 - Math.abs(2 * l - 1));
    const h = mx === r ? 60 * (((g - b) / d) % 6) : mx === g ? 60 * ((b - r) / d + 2) : 60 * ((r - g) / d + 4);
    const hue = (h + 360) % 360;
    return hue >= 100 && hue <= 170 && s > 0.2;
  };
  const painted = (v) => {
    const m = v.match(/url\(["']?#([^"')]+)["']?\)/);
    if (!m) return green(v) ? [v] : [];
    const ref = document.getElementById(m[1]);
    if (!ref) return [];
    return [ref, ...ref.querySelectorAll("*")].flatMap((d) => { const c = getComputedStyle(d); return [c.fill, c.stroke, c.stopColor].filter(green); }).map((g) => `${v} paints ${g}`);
  };
  const name = (el) => el.id || el.dataset.for || el.dataset.key || `${el.tagName} in ${el.closest("[id]")?.id}`;
  const drawn = [...document.querySelectorAll("svg.plant .sv-obj, svg.plant .sv-obj *, svg.plant [data-key], svg.plant [data-key] *, svg.plant .sv-fill, .sv-overlay .sv-tag")]
    .filter((el) => !el.closest(".sv-selected"))
    .flatMap((el) => { const c = getComputedStyle(el); return [["fill", c.fill], ["stroke", c.stroke]].flatMap(([p, v]) => painted(v).map((g) => `${name(el)} ${p} ${g}`)); });
  const around = [...document.querySelectorAll(".headline .fig .mono, .side ul.attention .mark, .side .badge")]
    .flatMap((el) => { const c = getComputedStyle(el); return [["color", c.color], ["background", c.backgroundColor]].filter(([, v]) => green(v)).map(([p, v]) => `${el.className} ${JSON.stringify(el.textContent)} ${p} ${v}`); });
  const accent = hex(root.getPropertyValue("--accent"));
  const unmarked = [...document.querySelectorAll("svg.plant .sv-selected")]
    .filter((el) => { const p = parts(getComputedStyle(el).stroke); return !p || p[3] === 0 || p.slice(0, 3).join(",") !== accent; })
    .map((el) => `${el.dataset.key !== undefined ? `${el.dataset.object} ${el.dataset.key}` : el.id} selected, stroked ${getComputedStyle(el).stroke}`);
  return drawn.concat(around, unmarked);
})()"##;

/// What carries the selection in the drawing: an element by its id, a keyed marker's clone by its
/// object's id and its key.
const SELECTED: &str = r#"[...document.querySelectorAll("svg.plant .sv-selected")].map((e) => e.dataset.key !== undefined ? `${e.dataset.object} ${e.dataset.key}` : e.id).sort()"#;

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

/// A screen box's centre.
fn centre(r: &Value) -> Option<(f64, f64)> {
    edges(r).map(|(l, t, rr, b, _)| ((l + rr) / 2.0, (t + b) / 2.0))
}

/// A point of the loop's own coordinates on the screen, through the loop's matrix (`__loop`).
fn on_screen(m: &Value, (x, y): (f64, f64)) -> Option<(f64, f64)> {
    let m: Vec<f64> = m.as_array()?.iter().filter_map(Value::as_f64).collect();
    (m.len() == 6).then(|| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]))
}

/// Whether a marker stands on screen where the loop's point at `place` is.
fn at_place(drawn: &Value, rect: &Value, place: f64) -> bool {
    match (centre(rect), on_screen(&drawn["__loop"], on_loop(place))) {
        (Some((cx, cy)), Some((px, py))) => near(cx, px, 1.0) && near(cy, py, 1.0),
        _ => false,
    }
}

/// A press and a release of the pointer's left button at a screen box's centre, as a person
/// clicks: whatever is painted there takes it.
fn click_at(page: &mut Browser, r: &Value) {
    let (x, y) = centre(r).unwrap_or_else(|| panic!("no box to click: {r}"));
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        page.call("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }));
    }
}

/// Where the drawing differs from the feed (nothing once the page has caught up): each bound
/// element's class and tag against its object's state, mark and word; each tag and label placed
/// on screen at its element and shown; each level's fill inside its element to its level; each
/// marker on screen at the path's point, its tag under it; a keyed marker's clones, one per key,
/// each with its tag.
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
            // Its label shown, centred just over it: its foot at most 40 px above the element's top.
            let label_over = matches!((edges(&d["label_rect"]), edges(&d["rect"])), (Some((_, _, _, lb, lc)), Some((_, et, _, _, ec))) if near(lc, ec, 3.0) && lb <= et + 2.0 && et - lb <= 40.0);
            if !label_over || d["label_shown"] != json!(true) {
                out.push(format!("{id}: its label at {} (shown {}) is not just over it at {}", d["label_rect"], d["label_shown"], d["rect"]));
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
                let place = v["place"].as_f64().unwrap();
                let ok = d["hidden"] == json!(false) && d["shown"] == json!(true) && d["hit"] == json!(true) && d["tag_keys"] == json!([""]) && at_place(drawn, &d["rect"], place);
                let tag_ok = said.is_empty() || (under(&d["tag_rect"], &d["rect"]) && d["tag_shown"] == json!(true));
                if !ok || !tag_ok {
                    out.push(format!("{id}: at {:?} on screen (its tag {} at {}), the path's point is {:?}", centre(&d["rect"]), d["tag"], d["tag_rect"], on_screen(&drawn["__loop"], on_loop(place))));
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
                    let place = it["place"].as_f64().unwrap();
                    let ok = clones.iter().find(|c| c["key"] == json!(key)).is_some_and(|c| {
                        c["state"] == json!(state)
                            && c["shown"] == json!(true)
                            && c["hit"] == json!(true)
                            && at_place(drawn, &c["rect"], place)
                            && c["tag"] == json!(said)
                            && c["tag_state"] == json!(state)
                            && (said.is_empty() || (under(&c["tag_rect"], &c["rect"]) && c["tag_shown"] == json!(true)))
                    });
                    if !ok {
                        out.push(format!("{id} {key}: drawn {:?}, the path's point is {:?}", clones.iter().find(|c| c["key"] == json!(key)), on_screen(&drawn["__loop"], on_loop(place))));
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
/// forgotten 6 s after its last, warned past 40), a disabled state for outlet 1 and its element a
/// group, outlet 2 moved by its own transform, outlet 3 inside a moved group, and the loop itself
/// drawn 30 down inside a moved group, its markers outside it.
fn grown(mut ex: Example) -> Example {
    ex.project.push_str("\n[[rule]]\nname = \"positions\"\npattern = 'POS item=(?P<item>\\w+) at=(?P<at>[0-9.]+)'\nscope = \"sorter\"\ntypes = { at = \"float\" }\nkey = \"item\"\n");
    ex.binding = ex
        .binding
        .replacen("states = [ { at_least = 100, state = \"warn\", word = \"full\" }", "states = [ { at_most = 0, state = \"disabled\", word = \"off\" }, { at_least = 100, state = \"warn\", word = \"full\" }", 1)
        .replacen(
            "\n[[dashboard]]",
            "\n[[object]]\nid = \"tracked\"\nlabel = \"Tracked\"\nvalue = { source = \"sorter\", series = \"at\" }\nshow = \"move\"\npath = \"loop\"\nstale_after_ms = 6000\nstates = [ { at_least = 40, state = \"warn\", word = \"far\" } ]\n\n[[dashboard]]",
            1,
        );
    ex.svg = ex
        .svg
        .replace("<path id=\"loop\" d=\"M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z\" fill=\"none\" stroke=\"#26262A\" stroke-width=\"14\"/>", "<g transform=\"translate(0 30)\"><path id=\"loop\" d=\"M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z\" fill=\"none\" stroke=\"#26262A\" stroke-width=\"14\"/></g>")
        .replace("<rect id=\"outlet-1\" x=\"130\" y=\"4\" width=\"24\" height=\"28\"/>", "<g id=\"outlet-1\"><rect x=\"130\" y=\"4\" width=\"24\" height=\"28\"/></g>")
        .replace("<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\"/>", "<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\" transform=\"translate(-20 110)\"/>")
        .replace("<rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/>", "<g transform=\"translate(0 100)\"><rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/></g>")
        // A keyed marker inside a moved group, as a drawing's layers put one: its clones stand at
        // their places whatever the group's transform.
        .replace("<circle id=\"parcel-2\" r=\"5\"/>", "<circle id=\"parcel-2\" r=\"5\"/>\n  <g transform=\"translate(15 -10)\"><circle id=\"tracked\" r=\"4\"/></g>");
    ex
}

#[test]
fn the_page_draws_the_layout_from_the_feed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let ex = grown(live_example(&bench, false));
    for grew in ["<g transform=\"translate(0 30)\"><path id=\"loop\"", "translate(-20 110)", "<g transform=\"translate(0 100)\"><rect id=\"outlet-3\"", "<g id=\"outlet-1\">", "<g transform=\"translate(15 -10)\"><circle id=\"tracked\""] {
        assert!(ex.svg.contains(grew), "{grew}: {}", ex.svg);
    }
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

    // The page, run against the session, draws it: read back until it agrees with the feed and
    // the selection is where it was put (on that element alone), and after each change the accent
    // marks no state.
    let mut page = Browser::watched(&format!("http://{addr}/s/loop/"));
    let settle = |page: &mut Browser, what: &str, selection: &[&str]| -> (Value, Value) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let feed = get(addr, "/s/loop/objects.json");
            let drawn = page.eval(DRAWN);
            let mut diff = differences(&drawn, &feed.1, &on_layout);
            let selected = page.eval(SELECTED);
            if selected != json!(selection) {
                diff.push(format!("the selection carried by {selected}, not by {selection:?} alone"));
            }
            if diff.is_empty() {
                let accent = accent_on_states(page);
                assert_eq!(accent, json!([]), "{what}: a state drawn in the accent");
                return (drawn, feed.1);
            }
            assert!(Instant::now() < deadline, "{what}: the page never drew the feed: {diff:#?}");
            std::thread::sleep(Duration::from_millis(150));
        }
    };
    let around_when = |page: &mut Browser, cond: &str| page.wait_for(&format!("(() => {{ const a = {AROUND}; return ({cond}) ? a : null; }})()"), Duration::from_secs(10));
    let (drawn, _) = settle(&mut page, "first paint", &[]);
    // The anchors, spelled out: the states the rules give the bench's values, and the markers at
    // a quarter and three quarters of the loop (drawn 30 down, so their own places are too).
    let d = |id: &str, k: &str| drawn[id][k].clone();
    assert_eq!((d("infeed", "state"), d("infeed", "tag")), (json!("sv-state-normal"), json!("")));
    assert_eq!((d("outlet-1", "state"), d("outlet-2", "state"), d("outlet-3", "state")), (json!("sv-state-normal"), json!("sv-state-near"), json!("sv-state-warn")));
    assert_eq!((d("outlet-2", "tag"), d("outlet-3", "tag")), (json!("■ near full"), json!("■ full")));
    let at = |id: &str| translate(d(id, "transform").as_str().unwrap());
    assert!(near(at("parcel-1").0, 309.96, 0.05) && near(at("parcel-1").1, 70.0, 0.05), "{:?}", at("parcel-1"));
    assert!(near(at("parcel-2").0, 90.04, 0.05) && near(at("parcel-2").1, 210.0, 0.05), "{:?}", at("parcel-2"));
    assert_eq!((d("tracked", "hidden"), d("tracked", "clones")), (json!(true), json!([])), "no position yet");

    // Around it: the headline in its states, the objects not normal in order of weight.
    let around = around_when(&mut page, "a.attention.length === 4 && a.headline.length === 2");
    assert_eq!(around["headline"], json!([["speed", "1.5", "mono st-text-normal"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention_head"], json!("Needs attention · 4"));
    let quiet = json!([
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
    ]);
    assert_eq!(around["attention"], quiet);

    // A click of the pointer on outlet 3 (full, so its fill covers it) selects it: the side panel
    // holds its value, state, sparkline (its own series over its last half hour) and offered
    // write, gated by portal's switch; the selection is drawn in Veredas' green (Q22).
    click_at(&mut page, &drawn["outlet-3"]["rect"]);
    let around = around_when(&mut page, "a.selected === 'Outlet 3' && a.spark > 0");
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
    settle(&mut page, "outlet 3 selected", &["outlet-3"]);

    // Every state the vocabulary has, the attention list in order of weight and the headline
    // live: the infeed in fault (alarm), the loop slow (warn), outlet 1 empty (disabled).
    let plc = bench.server("plc").unwrap();
    plc.set("infeed", 2);
    plc.set("fill1", 0.0);
    plc.set("speed", 0.2);
    assert!(wait(Duration::from_secs(20), || {
        let s = get(addr, "/s/loop/objects.json").1;
        object(&s, "infeed")["state"] == json!("alarm") && object(&s, "outlet-1")["state"] == json!("disabled") && object(&s, "speed")["state"] == json!("warn")
    }));
    settle(&mut page, "an alarm, a warning and a disabled object", &["outlet-3"]);
    let around = around_when(&mut page, "a.attention.length === 7 && a.headline[0][1] === '0.2'");
    assert_eq!(around["headline"], json!([["speed", "0.2", "mono st-text-warn"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention"], json!([
        ["infeed", "!", "mark st-alarm", "Infeed fault"],
        ["speed", "■", "mark st-warn", "Loop speed slow"],
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["outlet-1", "▨", "mark st-disabled", "Outlet 1 off"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
    ]));
    // Back to normal, each leaves the list, and the headline follows.
    plc.set("infeed", 1);
    plc.set("fill1", 35.0);
    plc.set("speed", 1.5);
    assert!(wait(Duration::from_secs(20), || {
        let s = get(addr, "/s/loop/objects.json").1;
        ["infeed", "outlet-1", "speed"].iter().all(|id| object(&s, id)["state"] == json!("normal"))
    }));
    settle(&mut page, "back to normal", &["outlet-3"]);
    let around = around_when(&mut page, "a.attention.length === 4 && a.headline[0][1] === '1.5'");
    assert_eq!((around["attention"].clone(), around["headline"][0].clone()), (quiet.clone(), json!(["speed", "1.5", "mono st-text-normal"])));

    // A keyed marker: one per key, each at its own place in the object's state (warned, B2's 50
    // the latest), its mark and word under it; one selected by a click of the pointer, alone, its
    // own value in the side panel.
    let now = store_now();
    append(&log, &[format!("{} POS item=A1 at=10", stamp(now)), format!("{} POS item=B2 at=50", stamp(now))]);
    assert!(wait(Duration::from_secs(20), || {
        let t = object(&get(addr, "/s/loop/objects.json").1, "tracked").clone();
        t["items"].as_array().unwrap().len() == 2 && t["state"] == json!("warn")
    }));
    let (drawn, _) = settle(&mut page, "the keyed marker", &["outlet-3"]);
    let tags: Vec<(Value, Value)> = drawn["tracked"]["clones"].as_array().unwrap().iter().map(|c| (c["key"].clone(), c["tag"].clone())).collect();
    assert_eq!((drawn["tracked"]["hidden"].clone(), tags.len()), (json!(true), 2), "{tags:?}");
    assert!(tags.iter().all(|(_, t)| *t == json!("■ far")), "{tags:?}");
    let around = around_when(&mut page, "a.attention.length === 4 && a.attention[1][0] === 'tracked'");
    assert_eq!(around["attention"][1], json!(["tracked", "■", "mark st-warn", "Tracked far"]));
    let a1 = drawn["tracked"]["clones"].as_array().unwrap().iter().find(|c| c["key"] == json!("A1")).unwrap()["rect"].clone();
    click_at(&mut page, &a1);
    let around = around_when(&mut page, "a.selected === 'Tracked · A1'");
    assert_eq!(around["value"], json!("10"), "A1's own value, not the latest of any key");
    settle(&mut page, "A1 selected", &["tracked A1"]);
    // A1 moves on (to 30, so the marker is normal again), B2 says nothing more: B2 is dropped with
    // its tag once its 6 s pass, A1 drawn at its new place, still selected, its new value beside,
    // and the marker leaves the attention list. Two and a half seconds after the first two, so A1
    // outlives B2 by as much.
    std::thread::sleep(Duration::from_millis(2500));
    append(&log, &[format!("{} POS item=A1 at=30", stamp(store_now()))]);
    assert!(wait(Duration::from_secs(20), || {
        let t = object(&get(addr, "/s/loop/objects.json").1, "tracked").clone();
        t["items"] == json!([{ "key": "A1", "place": 0.3, "text": "30" }]) && t["state"] == json!("normal")
    }));
    let (drawn, _) = settle(&mut page, "a key forgotten", &["tracked A1"]);
    let left: Vec<(Value, Value)> = drawn["tracked"]["clones"].as_array().unwrap().iter().map(|c| (c["key"].clone(), c["selected"].clone())).collect();
    assert_eq!((left, drawn["tracked"]["tag_keys"].clone()), (vec![(json!("A1"), json!(true))], json!(["A1"])));
    let around = around_when(&mut page, "a.selected === 'Tracked · A1' && a.value === '30' && a.attention.length === 3");
    assert_eq!(around["attention"], json!([quiet[0], quiet[1], quiet[2]]), "the marker normal again, off the list");

    // Moved by the plant, the marker moves along its path, its tag with it.
    plc.set("p1", 60.0);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "parcel-1")["place"] == json!(0.6)));
    let (drawn, _) = settle(&mut page, "the marker moved", &["tracked A1"]);
    let (x, y) = translate(drawn["parcel-1"]["transform"].as_str().unwrap());
    assert!(near(x, 228.02, 0.05) && near(y, 210.0, 0.05), "{x} {y}");
    // Another click moves the selection, which no element keeps.
    click_at(&mut page, &drawn["outlet-3"]["rect"]);
    around_when(&mut page, "a.selected === 'Outlet 3'");
    settle(&mut page, "outlet 3 selected again", &["outlet-3"]);

    // The plant away: every object of its source stale, drawn so, a marker's mark and word below
    // it, the headline's figure greyed with its last value.
    bench.stop();
    assert!(wait(Duration::from_secs(40), || {
        let s = get(addr, "/s/loop/objects.json").1;
        on_layout.iter().filter(|(id, _)| id != "tracked").all(|(id, _)| object(&s, id)["state"] == json!("stale"))
    }));
    let (drawn, feed) = settle(&mut page, "the plant away", &["outlet-3"]);
    assert_eq!(object(&feed, "parcel-1")["state"], json!("stale"));
    assert_eq!(drawn["parcel-1"]["tag"], json!(format!("{} {}", object(&feed, "parcel-1")["mark"].as_str().unwrap(), object(&feed, "parcel-1")["word"].as_str().unwrap())));
    assert!(drawn["parcel-1"]["tag"].as_str().unwrap().starts_with("◌ "), "{}", drawn["parcel-1"]);
    let around = around_when(&mut page, "a.headline[0][2].includes('stale')");
    assert_eq!(around["headline"], json!([["speed", "1.5", "mono st-text-stale"], ["read-rate", "—", "mono st-text-stale"]]));
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
