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
  // A paint's alpha as the browser paints it (a 1×1 canvas), a pattern or a gradient read as opaque.
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const alpha = (paint) => {
    if (!paint || paint === "none") return 0;
    if (paint.startsWith("url(")) return 1;
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = "rgba(0, 0, 0, 0)";
    ctx.fillStyle = paint;
    ctx.fillRect(0, 0, 1, 1);
    return ctx.getImageData(0, 0, 1, 1).data[3] / 255;
  };
  // Shown: displayed through every ancestor, visible, and painted with some alpha once its opacity
  // and every ancestor's, its paint's own alpha and its fill or stroke opacity are multiplied (a
  // group by the shapes inside it). A mask, a clip, a filter or a font that hides it is not read
  // (the clause states the limit).
  const SHAPES = "rect, circle, ellipse, path, polygon, polyline, line, text";
  const shown = (el) => {
    if (!el) return null;
    if (getComputedStyle(el).visibility !== "visible") return false;
    let op = 1;
    for (let a = el; a; a = a.parentElement) { const s = getComputedStyle(a); if (s.display === "none") return false; op *= Number(s.opacity); }
    const own = (x) => { const s = getComputedStyle(x); return Math.max(alpha(s.fill) * Number(s.fillOpacity), parseFloat(s.strokeWidth) > 0 ? alpha(s.stroke) * Number(s.strokeOpacity) : 0); };
    const shapes = el.matches(SHAPES) ? [el] : [...el.querySelectorAll(SHAPES)];
    return op * Math.max(0, ...shapes.map(own)) > 0.05;
  };
  // A level's fill shown by its fill alone: an outline is no level.
  const filled = (el) => {
    if (!el || getComputedStyle(el).visibility !== "visible") return el ? false : null;
    let op = 1;
    for (let a = el; a; a = a.parentElement) { const s = getComputedStyle(a); if (s.display === "none") return false; op *= Number(s.opacity); }
    const s = getComputedStyle(el);
    return op * alpha(s.fill) * Number(s.fillOpacity) > 0.05;
  };
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
  // A shape's stroke on the screen as the page draws it unselected (the selection's own stroke
  // apart): its width carried through its matrix to the screen.
  const strokePx = (el) => {
    const c = el.getScreenCTM();
    if (!c || !(el instanceof SVGGeometryElement)) return 0;
    const was = el.classList.contains("sv-selected");
    el.classList.remove("sv-selected");
    const w = parseFloat(getComputedStyle(el).strokeWidth) || 0;
    if (was) el.classList.add("sv-selected");
    return w * Math.sqrt(Math.abs(c.a * c.d - c.b * c.c));
  };
  const out = {};
  for (const el of document.querySelectorAll("svg.plant [data-object][id]")) {
    const id = el.id;
    const marker = el.classList.contains("sv-show-move");
    const tag = tagOf(id, marker ? "" : null);
    const label = document.querySelector(`.sv-overlay .sv-label[data-for="${id}"]`);
    const fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
    const value = document.querySelector(`.sv-overlay .sv-value[data-for="${id}"]`);
    out[id] = {
      kind: el instanceof SVGGeometryElement ? "shape" : el instanceof SVGTextElement ? "text" : "group",
      value: value ? [value.textContent, shown(value), rect(value)] : null,
      stroke_px: strokePx(el),
      state: st(el), hidden: el.classList.contains("sv-hidden"), transform: el.getAttribute("transform"), rect: rect(el), shown: shown(el), hit: marker ? hit(el) : null, selected: el.classList.contains("sv-selected"),
      tag_keys: [...document.querySelectorAll(`.sv-overlay .sv-tag[data-for="${id}"][data-key]`)].map((t) => t.dataset.key).sort(),
      tag: tag ? tag.textContent : null, tag_state: st(tag), tag_rect: rect(tag), tag_shown: shown(tag), label_rect: rect(label), label_shown: shown(label),
      fill: fill ? { rect: rect(fill), state: st(fill), shown: filled(fill), hit: hit(fill) } : null,
      clones: [...document.querySelectorAll(`svg.plant [data-object="${id}"][data-key]`)].map((c) => {
        const t = tagOf(id, c.dataset.key);
        return { key: c.dataset.key, state: st(c), transform: c.getAttribute("transform"), rect: rect(c), shown: shown(c), hit: hit(c), selected: c.classList.contains("sv-selected"), tag: t ? t.textContent : null, tag_state: st(t), tag_rect: rect(t), tag_shown: shown(t) };
      }),
    };
  }
  const loop = document.getElementById("loop");
  const m = loop ? loop.getScreenCTM() : null;
  out.__loop = m ? [m.a, m.b, m.c, m.d, m.e, m.f] : null;
  // Every path a marker can follow, its own matrix to the screen.
  out.__ctm = Object.fromEntries([...document.querySelectorAll("svg.plant [id]")].filter((e) => e.id === "loop" || e.id.startsWith("track-")).map((e) => { const c = e.getScreenCTM(); return [e.id, c ? [c.a, c.b, c.c, c.d, c.e, c.f] : null]; }));
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

/// The colour readers the accent checks share. A green is one of the accent's four colours (the
/// accent, its tint, its soft and its outline, by red, green and blue whatever the alpha) or any
/// colour of the accent's band of hue (100° to 170°) with some saturation (above 0.2): a shade one
/// unit off the accent is still the accent to the eye. Every colour is read as the browser paints it
/// (a 1×1 canvas filled with it, its bytes read back), whatever syntax the browser reports it in
/// (`rgb()`, `color(srgb ...)` from a `color-mix()`, `oklch()`, `lab()`). A paint (`fill`, `stroke`)
/// follows the paint grammar: a colour, or a `url(...)` with what follows it, the colour the
/// browser paints when the reference is no paint server; a reference within the page is read
/// through the colours it paints with (the fills, strokes and stop colours of the element it names
/// and everything inside it). `greens` reads every property the browser computes for an element,
/// from the browser's own list so none is left out (its custom properties apart): a paint as above,
/// any other value through every colour in it (a border, an outline, a shadow, a decoration, a
/// background image).
const COLOUR_READERS: &str = r##"
  const root = getComputedStyle(document.documentElement);
  const hex = (h) => { h = h.trim().replace("#", ""); if (h.length === 3) h = [...h].map((c) => c + c).join(""); return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)).join(","); };
  const family = ["--accent", "--accent-tint", "--accent-soft", "--accent-outline"].map((v) => hex(root.getPropertyValue(v)));
  const accent = hex(root.getPropertyValue("--accent"));
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
    const m = v.match(/^url\((["']?)(.*?)\1\)\s*(.*)$/);
    if (!m) return green(v) ? [v] : [];
    const fallback = m[3] && green(m[3]) ? [`${v} falls back to ${m[3]}`] : [];
    // A reference within the page names an element by its id; one to another document paints the
    // fallback alone.
    const at = m[2].indexOf("#");
    const local = at === 0 || (at > 0 && m[2].slice(0, at) === location.href.split("#")[0]);
    const ref = local ? document.getElementById(decodeURIComponent(m[2].slice(at + 1))) : null;
    if (!ref) return fallback;
    return fallback.concat([ref, ...ref.querySelectorAll("*")].flatMap((d) => { const c = getComputedStyle(d); return [c.fill, c.stroke, c.stopColor].filter(green); }).map((g) => `${v} paints ${g}`));
  };
  const TOKENS = /(rgba?\([^)]*\)|hsla?\([^)]*\)|hwb\([^)]*\)|(?:ok)?l(?:ab|ch)\([^)]*\)|color\([^)]*\)|#[0-9a-f]{3,8}\b)/gi;
  const greens = (el) => {
    const c = getComputedStyle(el);
    const out = [];
    for (let i = 0; i < c.length; i++) {
      const p = c[i];
      if (p.startsWith("--")) continue;
      const v = c.getPropertyValue(p);
      if (p === "fill" || p === "stroke") out.push(...painted(v).map((g) => `${p} ${g}`));
      else for (const t of v.match(TOKENS) || []) if (green(t)) out.push(`${p} ${t}`);
    }
    return out;
  };
"##;

/// Every state drawn in a green: the drawing's elements and everything inside them (the selection
/// apart, which the selection check pins to the one selected), its fills, tags, values and labels,
/// the headline's figures, the attention list's marks and the side panel's badges, each by every
/// property it has. The attention list's rows: the accent's own mark (`on`) on the selected
/// object's row alone, and no other row painted in a green by any property. Every selected element
/// marked on every shape it draws (a group's shapes inside it, their own stroke written or not):
/// stroked in the accent itself, a pixel wide or more, at half its alpha or more once every opacity
/// is multiplied, shown.
const ACCENT_ON_STATES: &str = r##"
  const name = (el) => el.id || el.dataset.for || el.dataset.key || `${el.tagName} in ${el.closest("[id]")?.id}`;
  const drawn = [...document.querySelectorAll("svg.plant .sv-obj, svg.plant .sv-obj *, svg.plant [data-key], svg.plant [data-key] *, svg.plant .sv-fill, .sv-overlay .sv-tag, .sv-overlay .sv-value, .sv-overlay .sv-label")]
    .filter((el) => !el.closest(".sv-selected"))
    .flatMap((el) => greens(el).map((g) => `${name(el)} ${g}`));
  // Every element outside the drawing whose class names a state (`st-`, `sv-state-`), wherever the
  // page writes one: the headline, the attention list, the badges, the alarm strip, the legend, the
  // Alarms view.
  const stateful = (el) => /(^|\s)(st-|sv-state-)/.test(el.getAttribute("class") || "");
  const around = [...document.querySelectorAll("[class]")].filter((el) => stateful(el) && !el.closest("svg.plant"))
    .flatMap((el) => greens(el).map((g) => `${el.getAttribute("class")} ${JSON.stringify(el.textContent)} ${g}`));
  // The side panel's own selection: every row's `on` mark by it, and nothing drawn selected but it.
  const chosen = document.querySelector(".side .sel-title")?.dataset.id ?? null;
  const strays = [...document.querySelectorAll("svg.plant .sv-selected")].map((e) => e.dataset.object ?? e.id).filter((id) => id !== chosen).map((id) => `${id} drawn selected while the side panel shows ${chosen}`);
  const rows = strays.concat([...document.querySelectorAll(".side ul.attention li")].flatMap((li) => {
    const on = li.classList.contains("on");
    const out = on !== (li.dataset.id === chosen) ? [`attention row ${li.dataset.id} on: ${on}, the side panel showing ${chosen}`] : [];
    return on ? out : out.concat(greens(li).map((g) => `attention row ${li.dataset.id} ${g}`));
  }));
  const GEOMETRY = "rect, circle, ellipse, path, polygon, polyline, line, text";
  const strokeAlpha = (s) => { let op = 1; for (let a = s; a; a = a.parentElement) op *= Number(getComputedStyle(a).opacity); const c = getComputedStyle(s), p = parts(c.stroke); return p ? op * p[3] * Number(c.strokeOpacity) : 0; };
  const unmarked = [...document.querySelectorAll("svg.plant .sv-selected")].flatMap((el) => {
    const who = el.dataset.key !== undefined ? `${el.dataset.object} ${el.dataset.key}` : el.id;
    const shapes = el.matches(GEOMETRY) ? [el] : [...el.querySelectorAll(GEOMETRY)];
    if (shapes.length === 0) return [`${who} selected, with no shape to stroke`];
    return shapes
      .filter((s) => { const c = getComputedStyle(s), p = parts(c.stroke); return !p || p.slice(0, 3).join(",") !== accent || !(parseFloat(c.strokeWidth) >= 1) || !(strokeAlpha(s) >= 0.5) || c.visibility !== "visible" || s.closest(".sv-hidden") !== null; })
      .map((s) => { const c = getComputedStyle(s); return `${who}${s === el ? "" : ` (its ${s.tagName})`} selected, stroked ${c.stroke} ${c.strokeWidth} at ${c.strokeOpacity}, alpha ${strokeAlpha(s)}`; });
  });
  return drawn.concat(around, rows, unmarked);
"##;

/// Every state the vocabulary has on every surface it is drawn on, by a probe element of each put
/// where the page puts its own (so every rule of the stylesheet that could reach one does): an
/// element and a group of every way of showing, a level's fill and a tag in the drawing, a figure
/// of the headline, a mark of the attention list and a badge; each read by `greens`, then removed.
/// The states and the ways of showing come from the crate's own enums (`STATES`, `KINDS`).
const PROBES: &str = r##"
  const svgNS = "http://www.w3.org/2000/svg";
  const plant = document.querySelector("svg.plant"), overlay = plant.querySelector(".sv-overlay");
  const made = [];
  const paints = [];
  const put = (parent, el, what) => { parent.append(el); made.push([el, what]); };
  const shape = (cls) => { const r = document.createElementNS(svgNS, "rect"); r.setAttribute("class", cls); r.setAttribute("width", "4"); r.setAttribute("height", "4"); return r; };
  for (const s of STATES) {
    for (const k of KINDS) {
      put(plant, shape(`sv-obj sv-show-${k} sv-state-${s}`), `a ${k} element in ${s}`);
      const g = document.createElementNS(svgNS, "g");
      g.setAttribute("class", `sv-obj sv-show-${k} sv-state-${s}`);
      const own = shape("");
      own.setAttribute("fill", "#123456");
      own.setAttribute("stroke", "#654321");
      own.setAttribute("stroke-width", "7");
      own.setAttribute("stroke-dasharray", "9 1");
      g.append(own);
      put(plant, g, `a ${k} group in ${s}`);
      paints.push([`${k} element`, s, () => made.find(([, w]) => w === `a ${k} element in ${s}`)[0], () => own]);
    }
    put(plant, shape(`sv-fill sv-state-${s}`), `a fill in ${s}`);
    const tag = document.createElementNS(svgNS, "text");
    tag.setAttribute("class", `sv-tag sv-state-${s}`);
    tag.textContent = "x";
    put(overlay, tag, `a tag in ${s}`);
    const fig = document.createElement("span");
    fig.className = "fig";
    fig.innerHTML = `<span class="mono st-text-${s}">1</span>`;
    put(document.querySelector(".headline"), fig, `the headline in ${s}`);
    const ul = document.createElement("ul");
    ul.className = "attention";
    ul.innerHTML = `<li><span class="mark st-${s}">!</span></li>`;
    put(document.querySelector(".side"), ul, `an attention row in ${s}`);
    const badge = document.createElement("span");
    badge.className = `badge st-${s}`;
    badge.textContent = "x";
    put(document.querySelector(".side"), badge, `a badge in ${s}`);
  }
  const out = made.flatMap(([el, what]) => [el, ...el.querySelectorAll("*")].flatMap((x) => greens(x).map((g) => `${what}: ${x === el ? "" : `its ${x.tagName} `}${g}`)));
  // Painted in its state: a group's shape (its own fill and stroke written) as a plain one is, and
  // on every surface each state painted otherwise than every other (a tag's normal apart: it is
  // empty).
  const sig = (el, props) => { const c = getComputedStyle(el); return props.map((q) => c.getPropertyValue(q)).join(" | "); };
  const SHAPE = ["fill", "stroke", "stroke-width", "stroke-dasharray"];
  for (const [what, s, plain, own] of paints) {
    if (sig(plain(), SHAPE) !== sig(own(), SHAPE)) out.push(`a ${what}'s group in ${s}: its shape painted ${sig(own(), SHAPE)}, a plain one ${sig(plain(), SHAPE)}`);
  }
  const surfaces = [
    ...KINDS.map((k) => [`a ${k} element`, (s) => `a ${k} element in ${s}`, SHAPE, STATES]),
    ["a fill", (s) => `a fill in ${s}`, SHAPE, STATES],
    ["a tag", (s) => `a tag in ${s}`, ["fill"], STATES.filter((s) => s !== "normal")],
    ["the headline", (s) => `the headline in ${s}`, ["color"], STATES],
    ["an attention mark", (s) => `an attention row in ${s}`, ["color"], STATES],
    ["a badge", (s) => `a badge in ${s}`, ["color", "background-color"], STATES],
  ];
  for (const [surface, named, props, states] of surfaces) {
    const seen = new Map();
    for (const s of states) {
      const entry = made.find(([, w]) => w === named(s));
      if (!entry) { out.push(`no probe for ${surface} in ${s}`); continue; }
      const el = entry[0].querySelector(".mono, .mark") ?? entry[0];
      const v = sig(el, props);
      if (seen.has(v)) out.push(`${surface} in ${s} painted as in ${seen.get(v)}: ${v}`);
      else seen.set(v, s);
    }
  }
  // Every way the drawing writes paint (presentation attributes, a `style` attribute) on a bound
  // element, and on every element a bound group may hold (the reader's own list of what it draws on,
  // its text apart: `DRAWN`) inside a group that writes its own, a group's rect within a held group
  // too: each painted as a plain element of its state; and, selected, each stroked as a plain
  // selected one.
  const OWN = { fill: "#123456", stroke: "#654321", "stroke-width": "7", "stroke-dasharray": "9 1", "fill-opacity": "0.3", "stroke-opacity": "0.4" };
  const write = (el, how) => {
    if (how === "attributes") for (const [k, v] of Object.entries(OWN)) el.setAttribute(k, v);
    else el.setAttribute("style", Object.entries(OWN).map(([k, v]) => `${k}: ${v}`).join("; "));
    return el;
  };
  const PAINT = ["fill", "stroke", "stroke-width", "stroke-dasharray", "fill-opacity", "stroke-opacity"];
  const selBase = {};
  const SEL = ["stroke", "stroke-width", "stroke-dasharray", "stroke-opacity"];
  const extra = [];
  const texts = [];
  const nest = (cls, how) => {
    const g = document.createElementNS(svgNS, "g");
    g.setAttribute("class", cls);
    const mid = write(document.createElementNS(svgNS, "g"), how);
    g.append(mid);
    const inner = [["the group between", mid]];
    // A text of the group's, writing its own paint: kept (read apart, against a free one).
    const text = write(document.createElementNS(svgNS, "text"), how);
    const span = document.createElementNS(svgNS, "tspan");
    span.setAttribute("fill", "#abcdef");
    span.setAttribute("stroke", "#fedcba");
    const run = document.createElementNS(svgNS, "tspan");
    run.setAttribute("fill", "#13579b");
    run.setAttribute("stroke", "#b97531");
    span.append(run);
    text.append(span);
    mid.append(text);
    texts.push([cls, how, text]);
    for (const name of HELD) {
      const e = write(document.createElementNS(svgNS, name), how);
      mid.append(e);
      inner.push([name, e]);
      if (name === "g") {
        const r = document.createElementNS(svgNS, "rect");
        e.append(r);
        inner.push(["a rect in a held g", r]);
      }
    }
    plant.append(g);
    extra.push(g);
    return inner;
  };
  for (const how of ["attributes", "style"]) {
    for (const k of KINDS) {
      for (const s of [...STATES.map((st) => [st, false]), ...STATES.map((st) => [st, true])]) {
        const [state, selected] = s;
        const cls = `sv-obj sv-show-${k} sv-state-${state}${selected ? " sv-selected" : ""}`;
        const props = selected ? SEL : PAINT;
        const plain = shape(cls);
        plant.append(plain);
        extra.push(plain);
        const want = sig(plain, props);
        const own = write(shape(cls), how);
        plant.append(own);
        extra.push(own);
        const what = `a ${k} element in ${state}${selected ? ", selected," : ""} its paint written as ${how}`;
        if (sig(own, props) !== want) out.push(`${what}: ${sig(own, props)}, a plain one ${want}`);
        // The selection the same in every state: as on a normal element selected.
        if (selected && state === STATES[0]) selBase[k] = want;
        else if (selected && want !== selBase[k]) out.push(`a ${k} element selected in ${state}: ${want}, selected in ${STATES[0]} ${selBase[k]}`);
        // A bound text's span writing its own paint takes the text's state, and its selection.
        const tx = document.createElementNS(svgNS, "text");
        tx.setAttribute("class", cls);
        const span = write(document.createElementNS(svgNS, "tspan"), how);
        // And a run inside it writing its own, as Inkscape writes a styled run inside a line's span.
        const run = write(document.createElementNS(svgNS, "tspan"), how);
        span.append(run);
        tx.append(span);
        plant.append(tx);
        extra.push(tx);
        if (sig(span, props) !== want) out.push(`${what}, a bound text's span: ${sig(span, props)}, a plain one ${want}`);
        if (sig(run, props) !== want) out.push(`${what}, a bound text's run inside its span: ${sig(run, props)}, a plain one ${want}`);
        for (const [name, e] of nest(cls, how)) if (sig(e, props) !== want) out.push(`${what}, its group's ${name}: ${sig(e, props)}, a plain one ${want}`);
      }
    }
  }
  // Every group's text keeps the paint it writes, in every state and selected: as a free text.
  for (const [cls, how, text] of texts) {
    const free = write(document.createElementNS(svgNS, "text"), how);
    const span = document.createElementNS(svgNS, "tspan");
    span.setAttribute("fill", "#abcdef");
    span.setAttribute("stroke", "#fedcba");
    const run = document.createElementNS(svgNS, "tspan");
    run.setAttribute("fill", "#13579b");
    run.setAttribute("stroke", "#b97531");
    span.append(run);
    free.append(span);
    plant.append(free);
    extra.push(free);
    if (sig(text, PAINT) !== sig(free, PAINT)) out.push(`a text in a group ${cls}, its paint written as ${how}: ${sig(text, PAINT)}, a free one ${sig(free, PAINT)}`);
    // Its span writing its own paint keeps it, as a free text's does, and the run inside the span.
    if (sig(text.firstElementChild, PAINT) !== sig(span, PAINT)) out.push(`a span of a text in a group ${cls}: ${sig(text.firstElementChild, PAINT)}, a free one's ${sig(span, PAINT)}`);
    if (sig(text.firstElementChild.firstElementChild, PAINT) !== sig(run, PAINT)) out.push(`a run in a span of a text in a group ${cls}: ${sig(text.firstElementChild.firstElementChild, PAINT)}, a free one's ${sig(run, PAINT)}`);
  }
  // Every way of showing in every state shown: displayed, visible and painted with some alpha once
  // every opacity is multiplied (an element by its fill or its stroke, a group by its shape); a fill
  // and a tag by their fill alone, neither stroked (a state's outline is a bound element's alone).
  const cv = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const alphaOf = (paint) => { if (!paint || paint === "none") return 0; if (paint.startsWith("url(")) return 1; cv.clearRect(0, 0, 1, 1); cv.fillStyle = "rgba(0, 0, 0, 0)"; cv.fillStyle = paint; cv.fillRect(0, 0, 1, 1); return cv.getImageData(0, 0, 1, 1).data[3] / 255; };
  const opac = (el) => { let op = 1; for (let a = el; a; a = a.parentElement) { const c = getComputedStyle(a); if (c.display === "none") return 0; op *= Number(c.opacity); } return getComputedStyle(el).visibility === "visible" ? op : 0; };
  const byFill = (el) => { const c = getComputedStyle(el); return opac(el) * alphaOf(c.fill) * Number(c.fillOpacity); };
  const byStroke = (el) => { const c = getComputedStyle(el); return parseFloat(c.strokeWidth) > 0 ? opac(el) * alphaOf(c.stroke) * Number(c.strokeOpacity) : 0; };
  for (const s of STATES) {
    for (const k of KINDS) {
      const el = made.find(([, w]) => w === `a ${k} element in ${s}`)[0];
      if (Math.max(byFill(el), byStroke(el)) <= 0.05) out.push(`a ${k} element in ${s} not shown`);
      const shape = made.find(([, w]) => w === `a ${k} group in ${s}`)[0].firstElementChild;
      if (Math.max(byFill(shape), byStroke(shape)) <= 0.05) out.push(`a ${k} group's shape in ${s} not shown`);
    }
    for (const what of ["a fill", "a tag"]) {
      const el = made.find(([, w]) => w === `${what} in ${s}`)[0];
      if (byFill(el) <= 0.05) out.push(`${what} in ${s} not shown by its fill`);
      if (getComputedStyle(el).stroke !== "none") out.push(`${what} in ${s} stroked ${getComputedStyle(el).stroke}`);
    }
  }
  for (const e of extra) e.remove();
  for (const [el] of made) el.remove();
  return out;
"##;

/// Every place outside the drawing where the page writes a state's class (`st-`, `sv-state-`), read
/// from the document as it stands (the headline, the attention list, the legend, the alarm strip,
/// the rules' badges, the Alarms view: whatever is there now), by its parent and its class's
/// family: a probe per state put beside the first element found there, its state word swapped for
/// each, and every state painted otherwise than every other.
const PLACES: &str = r##"(() => {
  const STATE = new RegExp(`^(st-(?:[a-z]+-)?|sv-state-)(${STATES.join("|")})$`);
  const family = (cls) => cls.split(/\s+/).map((c) => c.replace(STATE, "$1*")).join(" ");
  const sig = (el) => { const c = getComputedStyle(el); return ["color", "background-color", "fill", "stroke", "border-top-color"].map((q) => c.getPropertyValue(q)).join(" | "); };
  const places = new Map();
  for (const el of document.querySelectorAll("[class]")) {
    if (el.closest("svg.plant")) continue;
    const cls = el.getAttribute("class") || "";
    const fam = family(cls);
    if (fam === cls) continue;
    const where = `${el.parentElement?.getAttribute("class") ?? el.parentElement?.tagName} > ${fam}`;
    if (!places.has(where)) places.set(where, el);
  }
  const out = [];
  for (const [where, el] of places) {
    const seen = new Map();
    for (const s of STATES) {
      const p = el.cloneNode(true);
      p.setAttribute("class", family(el.getAttribute("class")).replaceAll("*", s));
      el.after(p);
      const v = sig(p);
      p.remove();
      if (seen.has(v)) out.push(`${where} in ${s} painted as in ${seen.get(v)}: ${v}`);
      else seen.set(v, s);
    }
  }
  return [[...places.keys()].sort(), out];
})()"##;

/// The places a state's class is written outside the drawing, and each told apart in every state.
fn places_told_apart(page: &mut Browser) -> Value {
    page.eval(&format!("(() => {{ const STATES = {};\nreturn {PLACES};\n}})()", serde_json::to_string(&every_state()).unwrap()))
}

/// What carries the selection in the drawing: an element by its id, a keyed marker's clone by its
/// object's id and its key.
const SELECTED: &str = r#"[...document.querySelectorAll("svg.plant .sv-selected")].map((e) => e.dataset.key !== undefined ? `${e.dataset.object} ${e.dataset.key}` : e.id).sort()"#;

/// The accent check once every transition has run: a fill or stroke sliding from the selection's
/// green is not a state drawn in it.
fn accent_on_states(page: &mut Browser) -> Value {
    page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running")"#, Duration::from_secs(5));
    page.eval(&format!("(() => {{\n{COLOUR_READERS}\n{ACCENT_ON_STATES}\n}})()"))
}

/// The probes of every state on every surface (`PROBES`), the states and the ways of showing taken
/// from the crate's own enums.
fn probe_states(page: &mut Browser) -> Value {
    let list = |names: Vec<&str>| serde_json::to_string(&names).unwrap();
    // What a bound group may hold: the reader's own list of what the page draws on, its text apart
    // (a text keeps the paint it writes).
    let held: Vec<&str> = veredas_supervisao::svg::DRAWN.into_iter().filter(|n| *n != "text").collect();
    page.eval(&format!("(() => {{\n{COLOUR_READERS}\nconst STATES = {};\nconst KINDS = {};\nconst HELD = {};\n{PROBES}\n}})()", list(every_state()), list(every_show()), list(held)))
}

/// The vocabulary's states, from the crate's own enum: a state added there stops this match
/// compiling until it is listed.
fn every_state() -> Vec<&'static str> {
    use veredas_supervisao::state::StateClass as S;
    let all = [S::Normal, S::Near, S::Warn, S::Alarm, S::Disabled, S::Stale];
    for s in all {
        match s {
            S::Normal | S::Near | S::Warn | S::Alarm | S::Disabled | S::Stale => {}
        }
    }
    all.iter().map(|s| s.name()).collect()
}

/// The binding's ways of showing, from the crate's own enum, likewise.
fn every_show() -> Vec<&'static str> {
    use veredas_supervisao::binding::Show;
    let all = [Show::State, Show::Level, Show::Value, Show::Move];
    for s in all {
        match s {
            Show::State | Show::Level | Show::Value | Show::Move => {}
        }
    }
    all.iter().map(|s| s.name()).collect()
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

/// The point at `f` (0 to 1) along a run of segments, closed back to its start or not.
fn along(points: &[(f64, f64)], closed: bool, f: f64) -> (f64, f64) {
    let mut pts = points.to_vec();
    if closed {
        pts.push(points[0]);
    }
    let lens: Vec<f64> = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).collect();
    let mut s = f * lens.iter().sum::<f64>();
    for (w, l) in pts.windows(2).zip(&lens) {
        if s <= *l {
            let k = if *l > 0.0 { s / l } else { 0.0 };
            return (w[0].0 + k * (w[1].0 - w[0].0), w[0].1 + k * (w[1].1 - w[0].1));
        }
        s -= l;
    }
    *pts.last().unwrap()
}

/// The point at `f` along an ellipse's outline from its right end, clockwise on the screen, by
/// its arc length (as `getPointAtLength` measures it).
fn on_ellipse(cx: f64, cy: f64, rx: f64, ry: f64, f: f64) -> (f64, f64) {
    let n = 20_000;
    let at = |i: usize| { let a = std::f64::consts::TAU * i as f64 / n as f64; (cx + rx * a.cos(), cy + ry * a.sin()) };
    let pts: Vec<(f64, f64)> = (0..n).map(at).collect();
    along(&pts, true, f)
}

/// The point at `f` along each path the fixture's markers follow, from the drawing's own geometry
/// (`GROWN_SVG`): the loop, and one of every other element a marker can follow.
fn on_path(path: &str, f: f64) -> (f64, f64) {
    match path {
        "loop" => on_loop(f),
        "track-line" => along(&[(90.0, 195.0), (130.0, 195.0)], false, f),
        "track-polyline" => along(&[(140.0, 160.0), (160.0, 160.0), (160.0, 180.0)], false, f),
        "track-polygon" => along(&[(170.0, 160.0), (190.0, 160.0), (190.0, 180.0), (170.0, 180.0)], true, f),
        "track-rect" => along(&[(200.0, 160.0), (220.0, 160.0), (220.0, 180.0), (200.0, 180.0)], true, f),
        "track-circle" => on_ellipse(245.0, 175.0, 10.0, 10.0, f),
        "track-ellipse" => on_ellipse(280.0, 175.0, 15.0, 8.0, f),
        "track-own" => along(&[(300.0, 200.0), (340.0, 200.0)], false, f),
        "track-g6" | "track-own6" => along(&[(0.0, 0.0), (30.0, 0.0)], false, f),
        other => panic!("a path this test cannot follow: {other}"),
    }
}

/// Whether a marker stands on screen where its path's point at `place` is, carried to the screen
/// through the path's own matrix.
fn at_place(drawn: &Value, rect: &Value, path: &str, place: f64) -> bool {
    match (centre(rect), on_screen(&drawn["__ctm"][path], on_path(path, place))) {
        (Some((cx, cy)), Some((px, py))) => near(cx, px, 1.0) && near(cy, py, 1.0),
        _ => false,
    }
}

/// A click on the drawing's background, where no element is drawn.
fn deselect(page: &mut Browser) {
    let at = page.eval(r#"(() => { const svg = document.querySelector("svg.plant"), r = svg.getBoundingClientRect(); for (const [x, y] of [[r.left + 3, r.top + 3], [r.right - 3, r.top + 3], [r.left + 3, r.bottom - 3]]) if (document.elementFromPoint(x, y) === svg) return [x - 1, y - 1, 2, 2]; return null; })()"#);
    assert!(!at.is_null(), "no background to click");
    click_at(page, &at);
}

/// A click on each attention row in turn, each settled: the object picked, drawn selected on what
/// it draws (from `drawn`: its element when shown, every clone of a keyed marker, else nothing).
fn pick_every_row(page: &mut Browser, drawn: &Value, settle: &dyn Fn(&mut Browser, &str, &[&str]) -> (Value, Value)) {
    let ids: Vec<String> = page.eval(r#"[...document.querySelectorAll(".side ul.attention li")].map((li) => li.dataset.id)"#).as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert!(!ids.is_empty());
    for id in ids {
        let row = settled_row(page, &id);
        click_at(page, &row);
        page.wait_for(&format!(r#"document.querySelector(".side .sel-title")?.dataset.id === "{id}" || null"#), Duration::from_secs(10));
        // The selection's sparkline holds its height whether it has loaded or not (read emptied),
        // so its loading moves nothing beneath it, the attention rows among them.
        let held = page.eval(r#"(() => { const w = document.querySelector(".side .spark-wrap"); const kids = [...w.childNodes]; w.replaceChildren(); const h = w.getBoundingClientRect().height; w.replaceChildren(...kids); return [h, Math.max(0, ...kids.map((k) => k.getBoundingClientRect?.().height ?? 0))]; })()"#);
        assert!(held[0].as_f64() == Some(44.0) && held[1].as_f64().unwrap() <= 44.0, "{id}: the sparkline's place {held}");
        let d = &drawn[id.as_str()];
        let mut want: Vec<String> = if d.is_null() {
            Vec::new()
        } else if !d["clones"].as_array().unwrap().is_empty() {
            d["clones"].as_array().unwrap().iter().map(|c| format!("{id} {}", c["key"].as_str().unwrap())).collect()
        } else if d["hidden"] == json!(true) {
            Vec::new()
        } else {
            vec![id.clone()]
        };
        want.sort();
        let want: Vec<&str> = want.iter().map(String::as_str).collect();
        settle(page, &format!("{id} picked from its row"), &want);
    }
}

/// An attention row's box once the side stands still, as a person sees it before clicking: the
/// row itself under its centre, at the same place over three reads a fifth of a second apart (the
/// list reorders when a state changes; the selection's sparkline holds its height while it loads,
/// so it moves nothing).
fn settled_row(page: &mut Browser, id: &str) -> Value {
    let read = format!(r#"(() => {{ const li = document.querySelector('.side ul.attention li[data-id="{id}"]'); if (!li) return null; const r = li.getBoundingClientRect(); const e = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return e && e.closest("li") === li ? [r.x, r.y, r.width, r.height] : null; }})()"#);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut seen: Vec<Value> = Vec::new();
    loop {
        let now = page.eval(&read);
        seen.push(now.clone());
        if seen.len() >= 3 && !now.is_null() && seen[seen.len() - 3..].iter().all(|r| *r == now) {
            return now;
        }
        assert!(std::time::Instant::now() < deadline, "the row of {id} never stood still under the pointer: {seen:?}");
        std::thread::sleep(Duration::from_millis(200));
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
fn differences(drawn: &Value, feed: &Value, on_layout: &[(String, String)], units: &std::collections::BTreeMap<String, String>, paths: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for (id, show) in on_layout {
        let path = paths.get(id).map(String::as_str).unwrap_or("loop");
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
            "value" => {
                // Its value over it, with its unit, shown, centred across it and standing within
                // its height.
                let unit = units.get(id).map(String::as_str).unwrap_or("");
                let text = v["text"].as_str().map(|t| if unit.is_empty() { t.to_string() } else { format!("{t} {unit}") }).unwrap_or_default();
                let over = matches!((centre(&d["value"][2]), edges(&d["rect"])), (Some((vx, vy)), Some((_, et, _, eb, ec))) if near(vx, ec, 3.0) && et <= vy && vy <= eb);
                if d["value"][0] != json!(text) || d["value"][1] != json!(true) || !over {
                    out.push(format!("{id}: its value drawn {}, the feed says {text:?}, over it at {}", d["value"], d["rect"]));
                }
            }
            "level" => {
                let f = v["level"].as_f64().unwrap();
                let fill = &d["fill"];
                // An element with no width or no height (a line) filled over its stroke: its box
                // widened to the stroke's width on the screen.
                let sw = d["stroke_px"].as_f64().unwrap_or(0.0);
                let widened = edges(&d["rect"]).map(|(el, et, er, eb, _)| {
                    let (mut l, mut t, mut r, mut b) = (el, et, er, eb);
                    if r - l < 0.5 && sw > r - l {
                        let c = (l + r) / 2.0;
                        (l, r) = (c - sw / 2.0, c + sw / 2.0);
                    }
                    if b - t < 0.5 && sw > b - t {
                        let c = (t + b) / 2.0;
                        (t, b) = (c - sw / 2.0, c + sw / 2.0);
                    }
                    (l, t, r, b)
                });
                let ok = match (edges(&fill["rect"]), widened) {
                    (Some((fl, ft, fr, fb, _)), Some((el, et, er, eb))) => near(fl, el, 1.0) && near(fr, er, 1.0) && near(fb, eb, 1.0) && near(fb - ft, f * (eb - et), 1.0),
                    _ => false,
                };
                let painted = fill["hit"] == json!(true) || d["kind"] == json!("text") || edges(&fill["rect"]).is_some_and(|(_, ft, _, fb, _)| fb - ft < 2.0);
                if !ok || fill["state"] != json!(state) || (f > 0.0 && (fill["shown"] != json!(true) || !painted)) {
                    out.push(format!("{id}: fill {fill} for level {f} in {}", d["rect"]));
                }
            }
            "move" if !keyed && v["place"].is_null() => {
                if d["hidden"] != json!(true) || d["shown"] != json!(false) {
                    out.push(format!("{id}: drawn though it has no place"));
                }
            }
            "move" if !keyed => {
                let place = v["place"].as_f64().unwrap();
                let ok = d["hidden"] == json!(false) && d["shown"] == json!(true) && d["hit"] == json!(true) && d["tag_keys"] == json!([""]) && at_place(drawn, &d["rect"], path, place);
                let tag_ok = said.is_empty() || (under(&d["tag_rect"], &d["rect"]) && d["tag_shown"] == json!(true));
                if !ok || !tag_ok {
                    out.push(format!("{id}: at {:?} on screen (its tag {} at {}), the path's point is {:?}", centre(&d["rect"]), d["tag"], d["tag_rect"], on_screen(&drawn["__ctm"][path], on_path(path, place))));
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
                // Its own element hidden and not shown: the clones are what is drawn.
                if d["hidden"] != json!(true) || d["shown"] != json!(false) {
                    out.push(format!("{id}: its own element drawn beside its clones"));
                }
                for it in items {
                    let key = it["key"].as_str().unwrap();
                    let place = it["place"].as_f64().unwrap();
                    let ok = clones.iter().find(|c| c["key"] == json!(key)).is_some_and(|c| {
                        c["state"] == json!(state)
                            && c["shown"] == json!(true)
                            && c["hit"] == json!(true)
                            && at_place(drawn, &c["rect"], path, place)
                            && c["tag"] == json!(said)
                            && c["tag_state"] == json!(state)
                            && (said.is_empty() || (under(&c["tag_rect"], &c["rect"]) && c["tag_shown"] == json!(true)))
                    });
                    if !ok {
                        out.push(format!("{id} {key}: drawn {:?}, the path's point is {:?}", clones.iter().find(|c| c["key"] == json!(key)), on_screen(&drawn["__ctm"][path], on_path(path, place))));
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
/// group whose shape writes its own stroke, the speed shown by its value, outlet 2 moved by its own transform, outlet 3 inside a moved group, and the loop itself
/// drawn 30 down inside a moved group, its markers outside it.
fn grown(mut ex: Example) -> Example {
    ex.project.push_str("\n[[rule]]\nname = \"positions\"\npattern = 'POS item=(?P<item>\\w+) at=(?P<at>[0-9.]+)'\nscope = \"sorter\"\ntypes = { at = \"float\" }\nkey = \"item\"\n");
    ex.binding = ex
        .binding
        .replacen("states = [ { at_least = 100, state = \"warn\", word = \"full\" }", "states = [ { at_most = 0, state = \"disabled\", word = \"off\" }, { at_least = 100, state = \"warn\", word = \"full\" }", 1)
        .replacen("label = \"Loop speed\"\n", "label = \"Loop speed\"\nshow = \"value\"\n", 1)
        .replacen(
            "\n[[dashboard]]",
            "\n[[object]]\nid = \"tracked\"\nlabel = \"Tracked\"\nvalue = { source = \"sorter\", series = \"at\" }\nshow = \"move\"\npath = \"loop\"\nstale_after_ms = 6000\nstates = [ { at_least = 40, state = \"warn\", word = \"far\" } ]\n\n[[dashboard]]",
            1,
        );
    ex.svg = ex
        .svg
        .replace("<path id=\"loop\" d=\"M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z\" fill=\"none\" stroke=\"#26262A\" stroke-width=\"14\"/>", "<g transform=\"translate(0 30)\"><path id=\"loop\" d=\"M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z\" fill=\"none\" stroke=\"#26262A\" stroke-width=\"14\"/></g>")
        .replace("<rect id=\"outlet-1\" x=\"130\" y=\"4\" width=\"24\" height=\"28\"/>", "<g id=\"outlet-1\"><rect x=\"130\" y=\"4\" width=\"24\" height=\"28\" stroke=\"#3a3a40\"/></g>")
        // The loop's speed shown by its value, inside the loop.
        .replace("<rect id=\"infeed\" x=\"20\" y=\"96\" width=\"40\" height=\"28\"/>", "<rect id=\"infeed\" x=\"20\" y=\"96\" width=\"40\" height=\"28\"/>\n  <rect id=\"speed\" x=\"110\" y=\"120\" width=\"50\" height=\"20\"/>")
        .replace("<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\"/>", "<rect id=\"outlet-2\" x=\"190\" y=\"4\" width=\"24\" height=\"28\" transform=\"translate(-20 110)\"/>")
        .replace("<rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/>", "<g transform=\"translate(0 100)\"><rect id=\"outlet-3\" x=\"250\" y=\"4\" width=\"24\" height=\"28\"/></g>")
        // A keyed marker inside a moved group, as a drawing's layers put one: its clones stand at
        // their places whatever the group's transform. Its element writes `display:inline`, as
        // Inkscape writes on an object hidden and shown again: the page hides it all the same.
        .replace("<circle id=\"parcel-2\" r=\"5\"/>", "<circle id=\"parcel-2\" r=\"5\"/>\n  <g transform=\"translate(15 -10) scale(2)\"><circle id=\"tracked\" r=\"4\" style=\"display:inline\"/></g>")
        .replace("viewBox=\"0 0 400 220\"", "viewBox=\"0 0 400 300\"")
        .replace("</svg>", GROWN_SVG);
    ex.binding = ex.binding.replacen("\n[[dashboard]]", GROWN_OBJECTS, 1).replacen("\n[[dashboard]]", MORE_OBJECTS, 1);
    ex
}

/// The drawing grown further: a level bound to a circle inside a turned group (filled within its
/// outline, up the screen) and one bound to a skewed text (within its glyphs); a level on every
/// other element a level can be bound to (a path, a line with no area, a polyline, a polygon, an
/// ellipse); a track of every element a marker can follow besides the loop's path (the polyline's
/// inside a skewed group), each with a marker on it (the rect's marker scaled by its own transform,
/// the circle's by its own matrix).
const GROWN_SVG: &str = r##"  <g transform="rotate(90 300 110)"><circle id="ring" cx="300" cy="110" r="12"/></g>
  <text id="word" x="200" y="100" font-size="14" transform="skewX(10)">■■■</text>
  <path id="lv-path" d="M 200 4 h 16 v 28 h -16 Z"/>
  <line id="lv-line" x1="240" y1="4" x2="240" y2="32" stroke="#3a3a40" stroke-width="10"/>
  <polyline id="lv-polyline" points="260,4 276,4 276,32 260,32"/>
  <polygon id="lv-polygon" points="290,4 306,4 306,32 290,32"/>
  <ellipse id="lv-ellipse" cx="330" cy="18" rx="9" ry="14"/>
  <line id="track-line" x1="90" y1="195" x2="130" y2="195" stroke="#26262A"/>
  <g transform="skewY(5)"><polyline id="track-polyline" points="140,160 160,160 160,180" fill="none" stroke="#26262A"/></g>
  <polygon id="track-polygon" points="170,160 190,160 190,180 170,180" fill="none" stroke="#26262A"/>
  <rect id="track-rect" x="200" y="160" width="20" height="20" fill="none" stroke="#26262A"/>
  <circle id="track-circle" cx="245" cy="175" r="10" fill="none" stroke="#26262A"/>
  <ellipse id="track-ellipse" cx="280" cy="175" rx="15" ry="8" fill="none" stroke="#26262A"/>
  <circle id="m-line" r="3"/>
  <circle id="m-polyline" r="3"/>
  <circle id="m-polygon" r="3"/>
  <circle id="m-rect" r="3" transform="scale(2)"/>
  <circle id="m-circle" r="3" transform="matrix(1.5 0 0 1.5 0 0)"/>
  <circle id="m-ellipse" r="3"/>
  <line id="track-own" x1="300" y1="200" x2="340" y2="200" stroke="#26262A" transform="translate(0 -10)"/>
  <circle id="o-translate" cx="50" cy="40" r="3" transform="translate(5 -3)"/>
  <circle id="o-scale" cx="50" cy="40" r="3" transform="scale(1.5)"/>
  <circle id="o-rotate" cx="50" cy="40" r="3" transform="rotate(30)"/>
  <circle id="o-skewx" cx="50" cy="40" r="3" transform="skewX(20)"/>
  <circle id="o-skewy" cx="50" cy="40" r="3" transform="skewY(20)"/>
  <circle id="o-matrix" cx="50" cy="40" r="3" transform="matrix(1 0.2 0 1 3 1)"/>
  <circle id="o-list" cx="50" cy="40" r="3" transform="translate(4 0) scale(2)"/>
  <line id="lv-hline" x1="350" y1="10" x2="380" y2="10" stroke="#3a3a40" stroke-width="10"/>
  <path id="lv-hpath" d="M 350 30 H 380" stroke="#3a3a40" stroke-width="8"/>
  <path id="lv-vpath" d="M 392 4 V 32" stroke="#3a3a40" stroke-width="8"/>
  <g transform="translate(40 250) rotate(8) scale(1.2) skewX(6) skewY(4) matrix(1 0 0 1 0 0)">
    <line id="track-g6" x1="0" y1="0" x2="30" y2="0" stroke="#26262A"/>
    <circle id="m-g6" r="2"/>
    <rect id="lv-g6" x="0" y="8" width="10" height="12"/>
    <rect id="v-g6" x="36" y="6" width="26" height="14"/>
  </g>
  <g transform="translate(120 248) scale(2)"><line id="lv-scaled" x1="0" y1="0" x2="0" y2="14" stroke="#3a3a40" stroke-width="6"/></g>
  <rect id="lv-own6" x="0" y="0" width="10" height="12" transform="translate(160 250) rotate(8) scale(1.2) skewX(6) skewY(4) matrix(1 0 0 1 0 0)"/>
  <path id="lv-ring" d="M 320,215 H 350 V 245 H 320 Z M 324,219 H 336 V 229 H 324 Z" style="fill:#cccccc;fill-rule:evenodd;clip-rule:nonzero;stroke:#000000;stroke-width:0.264583"/>
  <path id="lv-ring-nz" d="M 360,215 H 390 V 245 H 360 Z M 364,219 H 376 V 229 H 364 Z" style="fill:#cccccc;stroke:#000000;stroke-width:0.264583"/>
  <rect id="v-own6" x="0" y="0" width="26" height="14" transform="translate(200 250) rotate(8) scale(1.2) skewX(6) skewY(4) matrix(1 0 0 1 0 0)"/>
  <line id="track-own6" x1="0" y1="0" x2="30" y2="0" stroke="#26262A" transform="translate(260 262) rotate(8) scale(1.2) skewX(6) skewY(4) matrix(1 0 0 1 0 0)"/>
  <circle id="m-own6" r="2"/>
</svg>"##;

/// The levels on every other element a level can be bound to (and on a line and a straight path with
/// no width and with no height); the markers whose own transform is each function a transform takes,
/// and a list of two, each drawn off its own origin (one on a path that writes its own transform);
/// and an object bound to one key of the keyed marker's series (on no element: its row and its side
/// panel alone).
const MORE_OBJECTS: &str = "\n[[object]]\nid = \"lv-path\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-line\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-polyline\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-polygon\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-ellipse\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-hline\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-hpath\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-vpath\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"o-translate\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-line\"\n\n[[object]]\nid = \"o-scale\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-polyline\"\n\n[[object]]\nid = \"o-rotate\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-polygon\"\n\n[[object]]\nid = \"o-skewx\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-rect\"\n\n[[object]]\nid = \"o-skewy\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-circle\"\n\n[[object]]\nid = \"o-matrix\"\nvalue = { source = \"plc\", series = \"p2\" }\nshow = \"move\"\npath = \"track-ellipse\"\n\n[[object]]\nid = \"o-list\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-own\"\n\n[[object]]\nid = \"lv-g6\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-scaled\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"v-g6\"\nvalue = { source = \"plc\", series = \"speed\" }\nshow = \"value\"\n\n[[object]]\nid = \"m-g6\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-g6\"\n\n[[object]]\nid = \"lv-own6\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"v-own6\"\nvalue = { source = \"plc\", series = \"speed\" }\nshow = \"value\"\n\n[[object]]\nid = \"m-own6\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-own6\"\n\n[[object]]\nid = \"lv-ring\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"lv-ring-nz\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"keyed\"\nlabel = \"Keyed\"\nvalue = { source = \"sorter\", series = \"at\", key = \"A1\" }\n\n[[dashboard]]";

const GROWN_OBJECTS: &str = "\n[[object]]\nid = \"ring\"\nlabel = \"Ring\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"word\"\nlabel = \"Word\"\nvalue = { source = \"plc\", series = \"fill2\" }\nshow = \"level\"\n\n[[object]]\nid = \"m-line\"\nlabel = \"On a line\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-line\"\n\n[[object]]\nid = \"m-polyline\"\nlabel = \"On a polyline\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-polyline\"\n\n[[object]]\nid = \"m-polygon\"\nlabel = \"On a polygon\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-polygon\"\n\n[[object]]\nid = \"m-rect\"\nlabel = \"On a rect\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-rect\"\n\n[[object]]\nid = \"m-circle\"\nlabel = \"On a circle\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-circle\"\n\n[[object]]\nid = \"m-ellipse\"\nlabel = \"On an ellipse\"\nvalue = { source = \"plc\", series = \"p1\" }\nshow = \"move\"\npath = \"track-ellipse\"\n\n[[dashboard]]";

/// Every level's fill against its element's outline: a shape's or a text's fill clipped by the
/// element itself (its clip's `use` naming the element, on the fill or a group holding it), and for
/// a shape, on a grid of 11 by 11 points over the fill's box, each the browser reads inside the
/// shape's fill (by its own fill rule: a ring with a hole, `evenodd` and `nonzero`) taking the fill
/// and each outside it not (a point a pixel off the outline, read either way, is no verdict); a
/// group's fill, and a shape's with no area (a line), its box, never clipped by the element itself:
/// with no area, held to its runs' bands (a clip of rects alone, `sv-runs-…`), a group with an
/// area unclipped. Each level's element named by its tag.
const LEVEL_OUTLINES: &str = r#"(() => [...document.querySelectorAll("svg.plant .sv-show-level[data-object]")].flatMap((el) => {
  const id = el.dataset.object;
  const kind = el instanceof SVGGeometryElement ? "shape" : el instanceof SVGTextElement ? "text" : "group";
  const fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
  const clip = fill.closest("[clip-path]")?.getAttribute("clip-path") ?? null;
  const out = [];
  const box = el.getBBox();
  if (kind === "group" || el instanceof SVGLineElement || box.width === 0 || box.height === 0) {
    const ref = clip && clip.match(/url\(["']?#([^"')]+)/);
    const cp = ref ? document.getElementById(ref[1]) : null;
    const bandsOnly = !!cp && cp.id === `sv-runs-${id}` && cp.children.length > 0 && [...cp.children].every((c) => c.tagName === "rect");
    if (el.classList.contains("sv-noarea") ? !bandsOnly : clip) out.push(`${id}: a ${kind === "group" ? "group's" : "line's"} fill clipped otherwise than by its runs (${clip})`);
    return out.concat([`tag ${el.tagName}`]);
  }
  const ref = clip && clip.match(/url\(["']?#([^"')]+)/);
  const use = ref ? document.getElementById(ref[1])?.querySelector("use") : null;
  if (!use || use.getAttribute("href") !== `#${id}`) out.push(`${id}: its fill not clipped by its ${kind} (${clip})`);
  if (kind === "shape") {
    const r = fill.getBoundingClientRect(), m = el.getScreenCTM().inverse();
    const inside = (x, y) => el.isPointInFill(new DOMPoint(x, y).matrixTransform(m));
    const prev = fill.style.pointerEvents;
    fill.style.pointerEvents = "visiblePainted";
    const outside = [], missing = [];
    for (let i = 0; i <= 10; i++) for (let j = 0; j <= 10; j++) {
      const x = r.left + 1 + ((r.width - 2) * i) / 10, y = r.top + 1 + ((r.height - 2) * j) / 10;
      const at = inside(x, y);
      if ([[1, 0], [-1, 0], [0, 1], [0, -1]].some(([dx, dy]) => inside(x + dx, y + dy) !== at)) continue;
      const hit = document.elementFromPoint(x, y);
      if (!at && hit === fill) outside.push(`(${x.toFixed(1)}, ${y.toFixed(1)})`);
      if (at && hit !== fill && !hit?.closest(".sv-overlay")) missing.push(`(${x.toFixed(1)}, ${y.toFixed(1)}) ${hit?.id || hit?.tagName}`);
    }
    fill.style.pointerEvents = prev;
    if (outside.length) out.push(`${id}: its fill painted outside its outline at ${outside.join(" ")}`);
    if (missing.length) out.push(`${id}: its fill missing inside its outline at ${missing.join(" ")}`);
  }
  return out.concat([`tag ${el.tagName}`]);
}))()"#;

/// Everything the page writes beside the drawing's elements and shows (each label, mark and word,
/// value: the overlay's own children, a census) against the drawing on the screen: inside the svg's
/// own box, none cut by its frame; and the drawing's own frame (its viewBox as written, given) inside
/// the view's. With the count of what was read.
const INSIDE_FRAME: &str = r#"((drawn) => {
  const svg = document.querySelector("svg.plant");
  const s = svg.getBoundingClientRect();
  const said = [...svg.querySelectorAll(".sv-overlay > *")].filter((t) => t.textContent && getComputedStyle(t).display !== "none");
  const cut = said.flatMap((t) => {
    const r = t.getBoundingClientRect();
    return r.left < s.left - 0.5 || r.top < s.top - 0.5 || r.right > s.right + 0.5 || r.bottom > s.bottom + 0.5 ? [`${t.getAttribute("class")} ${t.dataset.for} "${t.textContent}": [${r.left.toFixed(1)}, ${r.top.toFixed(1)}, ${r.right.toFixed(1)}, ${r.bottom.toFixed(1)}] beyond [${s.left.toFixed(1)}, ${s.top.toFixed(1)}, ${s.right.toFixed(1)}, ${s.bottom.toFixed(1)}]`] : [];
  });
  // Nor covered: at its middle and inside each corner, the topmost thing on the screen is the
  // drawing (or what lies in it), never anything of the page's over it.
  const covered = said.flatMap((t) => {
    const r = t.getBoundingClientRect();
    const over = [[(r.left + r.right) / 2, (r.top + r.bottom) / 2], [r.left + 1, r.top + 1], [r.right - 1, r.top + 1], [r.left + 1, r.bottom - 1], [r.right - 1, r.bottom - 1]].map(([px, py]) => document.elementFromPoint(px, py)).filter((e) => !e || !(e === svg || svg.contains(e)));
    return over.length ? [`${t.getAttribute("class")} ${t.dataset.for} "${t.textContent}" covered by ${over.map((e) => (e ? `${e.tagName.toLowerCase()}.${e.className}` : "nothing")).join(", ")}`] : [];
  });
  const v = svg.viewBox.baseVal;
  const [x, y, w, h] = drawn;
  const kept = v.x <= x + 1e-3 && v.y <= y + 1e-3 && v.x + v.width >= x + w - 1e-3 && v.y + v.height >= y + h - 1e-3;
  return [cut.concat(covered), kept, said.length, [v.x, v.y, v.width, v.height]];
})"#;

/// The page's own controls beside the drawing (every element of the plant's pane outside the svg
/// and around none of it: the layers and their buttons, the legend and its words, the zoom and its
/// buttons), each box clear of the svg's: none lies over the drawing. With the count read.
const CONTROLS_CLEAR: &str = r#"(() => {
  const svg = document.querySelector("svg.plant"), s = svg.getBoundingClientRect();
  const mine = [...svg.closest(".plant-pane").querySelectorAll("*")].filter((e) => !(e instanceof SVGElement) && !e.contains(svg));
  const over = mine.map((e) => [e, e.getBoundingClientRect()]).filter(([, r]) => r.width > 0 && r.height > 0 && r.left < s.right && r.right > s.left && r.top < s.bottom && r.bottom > s.top).map(([e, r]) => `${e.tagName.toLowerCase()}.${e.className} "${e.textContent.trim().slice(0, 20)}" [${r.left.toFixed(1)}, ${r.top.toFixed(1)}, ${r.right.toFixed(1)}, ${r.bottom.toFixed(1)}]`);
  return [mine.length, over, [s.left, s.top, s.right, s.bottom]];
})()"#;

fn assert_controls_clear(page: &mut Browser, what: &str) {
    let got = page.eval(CONTROLS_CLEAR);
    assert!(got[0].as_u64().unwrap() >= 14 && got[1] == json!([]), "{what}: the page's own controls over the drawing: {got}");
}

/// Every marker placed by its element's box as the browser gives it now (no slide running), and every
/// key's clone of a keyed one by its own: its
/// box's centre, carried to the overlay through its own matrices, on its path's point at the place
/// the feed gives, and its mark and word centred across it and below it; each within the hundredth
/// the page writes a place to. With the count read, and of clones.
const MARKERS_NOW: &str = r#"(async () => {
  const at = location.pathname.replace(/\/$/, "");
  const [feed, binding] = await Promise.all([fetch(`${at}/objects.json`).then((r) => r.json()), fetch(`${at}/binding`).then((r) => r.json())]);
  const svg = document.querySelector("svg.plant"), overlay = svg.querySelector(".sv-overlay"), O = overlay.getCTM().inverse();
  const out = [];
  let read = 0, clones = 0;
  const off = (id, what, got, want) => { read++; if (Math.abs(got - want) > 0.011) out.push(`${id}: its ${what} at ${got}, the box says ${want}`); };
  for (const def of binding.objects.filter((o) => o.show === "move")) {
    const el = document.getElementById(def.id), o = feed.objects.find((x) => x.id === def.id);
    if (!el || !o) continue;
    const p = document.getElementById(def.path);
    // One marker (its element, the key "") or every key's clone, each at its own place.
    const each = o.items.length ? o.items.map((it) => [`${def.id} ${it.key}`, svg.querySelector(`[data-object="${def.id}"][data-key="${CSS.escape(it.key)}"]`), it.key, it.place]) : o.place === null ? [] : [[def.id, el, "", o.place]];
    for (const [who, node, key, place] of each) {
      if (!node) { out.push(`${who}: no clone drawn`); continue; }
      if (key !== "") clones++;
      const pt = p.getPointAtLength(place * p.getTotalLength());
      const q = new DOMPoint(pt.x, pt.y).matrixTransform(O.multiply(p.getCTM()));
      const b = node.getBBox(), m = O.multiply(node.getCTM());
      const pts = [[b.x, b.y], [b.x + b.width, b.y], [b.x, b.y + b.height], [b.x + b.width, b.y + b.height]].map(([x, y]) => new DOMPoint(x, y).matrixTransform(m));
      const c = new DOMPoint(b.x + b.width / 2, b.y + b.height / 2).matrixTransform(m);
      off(who, "centre across", c.x, q.x);
      off(who, "centre down", c.y, q.y);
      const t = overlay.querySelector(`.sv-tag[data-for="${def.id}"][data-key="${CSS.escape(key)}"]`);
      if (t && t.textContent) {
        const fs = Number(t.getAttribute("font-size")) / 0.8;
        off(who, "tag across", Number(t.getAttribute("x")), (Math.min(...pts.map((p) => p.x)) + Math.max(...pts.map((p) => p.x))) / 2);
        off(who, "tag's height", Number(t.getAttribute("y")), Math.max(...pts.map((p) => p.y)) + fs * 0.9);
      }
    }
  }
  return [read, out.slice(0, 20), clones];
})()"#;

/// What the page placed for every bound element but a marker against the element's box as the
/// browser gives it now, carried to the overlay (its corners through the matrices between them): the
/// label and the mark and word centred across it, the value too, and a level's fill (one with an
/// area) across it and resting on its foot, each within a thousandth of a unit. With the count read.
const PLACED_NOW: &str = r#"(() => {
  const svg = document.querySelector("svg.plant"), overlay = svg.querySelector(".sv-overlay");
  const out = [];
  let read = 0;
  for (const el of svg.querySelectorAll("[data-object][id]")) {
    if (el.classList.contains("sv-show-move")) continue;
    const id = el.id, b = el.getBBox(), m = overlay.getCTM().inverse().multiply(el.getCTM());
    const pts = [[b.x, b.y], [b.x + b.width, b.y], [b.x, b.y + b.height], [b.x + b.width, b.y + b.height]].map(([x, y]) => new DOMPoint(x, y).matrixTransform(m));
    const x0 = Math.min(...pts.map((p) => p.x)), x1 = Math.max(...pts.map((p) => p.x)), y0 = Math.min(...pts.map((p) => p.y)), y1 = Math.max(...pts.map((p) => p.y));
    const mid = (x0 + x1) / 2;
    const off = (what, got, want) => { read++; if (Math.abs(got - want) > 1e-3) out.push(`${id}: its ${what} at ${got}, the box says ${want}`); };
    // Each text across at the box's middle, and up and down by its own step from the box, in the
    // page's own font steps (a label 0.85 of the page's size, a mark and word 0.8, a value 1).
    const steps = { label: [0.85, (fs) => y0 - fs * 0.35], tag: [0.8, (fs) => y1 + fs * 1.05], value: [1, (fs) => (y0 + y1) / 2 + fs * 0.35] };
    for (const k of ["label", "tag", "value"]) {
      const t = overlay.querySelector(`.sv-${k}[data-for="${id}"]:not([data-key])`);
      if (!t) continue;
      off(k, Number(t.getAttribute("x")), mid);
      const [share, at] = steps[k];
      off(`${k}'s height`, Number(t.getAttribute("y")), at(Number(t.getAttribute("font-size")) / share));
    }
    const fill = svg.querySelector(`.sv-fill[data-for="${id}"]`);
    if (fill && !el.classList.contains("sv-noarea")) {
      off("fill's left", Number(fill.getAttribute("x")), x0);
      off("fill's width", Number(fill.getAttribute("width")), x1 - x0);
      off("fill's foot", Number(fill.getAttribute("y")) + Number(fill.getAttribute("height")), y1);
    }
  }
  return [read, out.slice(0, 20)];
})()"#;

/// What the page writes beside the drawing, read at the view's fit: none of it cut by the drawing's
/// frame, the frame itself kept whole, and at least `least` things read.
fn assert_inside_frame(page: &mut Browser, drawn: [f64; 4], least: u64, what: &str) {
    let got = page.eval(&format!("{INSIDE_FRAME}({})", serde_json::to_string(&drawn).unwrap()));
    assert!(got[0] == json!([]) && got[1] == json!(true) && got[2].as_u64().unwrap() >= least, "{what}: what the page writes beside the drawing cut by its frame, or the frame not kept whole: {got}");
}

#[test]
fn the_page_draws_the_layout_from_the_feed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let ex = grown(live_example(&bench, false));
    for grew in ["<g transform=\"translate(0 30)\"><path id=\"loop\"", "translate(-20 110)", "<g transform=\"translate(0 100)\"><rect id=\"outlet-3\"", "<g id=\"outlet-1\"><rect x=\"130\" y=\"4\" width=\"24\" height=\"28\" stroke=", "<rect id=\"speed\"", "<g transform=\"translate(15 -10) scale(2)\"><circle id=\"tracked\""] {
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
        ("speed", "value"), ("infeed", "state"), ("outlet-1", "level"), ("outlet-2", "level"), ("outlet-3", "level"), ("parcel-1", "move"), ("parcel-2", "move"), ("tracked", "move"),
        ("ring", "level"), ("word", "level"), ("m-line", "move"), ("m-polyline", "move"), ("m-polygon", "move"), ("m-rect", "move"), ("m-circle", "move"), ("m-ellipse", "move"),
        ("lv-path", "level"), ("lv-line", "level"), ("lv-polyline", "level"), ("lv-polygon", "level"), ("lv-ellipse", "level"),
        ("lv-hline", "level"), ("lv-hpath", "level"), ("lv-vpath", "level"),
        ("o-translate", "move"), ("o-scale", "move"), ("o-rotate", "move"), ("o-skewx", "move"), ("o-skewy", "move"), ("o-matrix", "move"), ("o-list", "move"),
        ("lv-g6", "level"), ("lv-scaled", "level"), ("v-g6", "value"), ("m-g6", "move"), ("lv-own6", "level"), ("v-own6", "value"), ("m-own6", "move"),
        ("lv-ring", "level"), ("lv-ring-nz", "level"),
    ]);
    let shows: std::collections::BTreeSet<&str> = on_layout.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(shows, every_show().into_iter().collect(), "every way of showing the binding knows is on the layout");
    // Every element a marker can follow (the reader's own list) followed by a marker here.
    let paths: std::collections::BTreeMap<String, String> = b["objects"].as_array().unwrap().iter().filter_map(|o| Some((o["id"].as_str()?.to_string(), o["path"].as_str()?.to_string()))).collect();
    let followed: std::collections::BTreeSet<&str> = paths.values().map(|p| { let at = svg.find(&format!(" id=\"{p}\"")).unwrap(); let open = svg[..at].rfind('<').unwrap(); &svg[open + 1..at] }).collect();
    assert_eq!(followed, veredas_supervisao::svg::FOLLOWABLE.into_iter().collect(), "a marker on every element the reader follows");
    let units: std::collections::BTreeMap<String, String> = b["objects"].as_array().unwrap().iter().map(|o| (o["id"].as_str().unwrap().to_string(), o["unit"].as_str().unwrap_or("").to_string())).collect();

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
            let mut diff = differences(&drawn, &feed.1, &on_layout, &units, &paths);
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
    // Outlet 1's label over it at the drawing's top edge among them.
    page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 300.0], 20, "first paint");
    assert_eq!(probe_states(&mut page), json!([]), "a state drawn in the accent on some surface");
    let first_places = places_told_apart(&mut page);
    assert_eq!(first_places[1], json!([]), "a place painting two states alike: {}", first_places[0]);
    assert!(first_places[0].as_array().unwrap().iter().any(|p| p.as_str().unwrap().starts_with("legend")), "the legend among the places: {}", first_places[0]);
    // Opened on another view, the layout behind it while the feed's snapshot is painted: once
    // shown, every marker stands at its place (nothing is placed off screen, everything is when
    // the layout is shown).
    let mut behind = Browser::open(&format!("http://{addr}/s/loop/#dashboards"));
    behind.wait_for("(() => { const f = document.querySelector('.headline .fig .mono'); return f !== null && f.textContent === '1.5' && document.querySelector('.dash-grid .panel') !== null; })()", Duration::from_secs(20));
    behind.eval("location.hash = 'layout'; true");
    settle(&mut behind, "shown after another view", &[]);
    drop(behind);
    // The anchors, spelled out: the states the rules give the bench's values, and the markers at
    // a quarter and three quarters of the loop (drawn 30 down, so their own places are too).
    let d = |id: &str, k: &str| drawn[id][k].clone();
    assert_eq!((d("infeed", "state"), d("infeed", "tag")), (json!("sv-state-normal"), json!("")));
    assert_eq!((d("outlet-1", "state"), d("outlet-2", "state"), d("outlet-3", "state")), (json!("sv-state-normal"), json!("sv-state-near"), json!("sv-state-warn")));
    assert_eq!((d("outlet-2", "tag"), d("outlet-3", "tag")), (json!("■ near full"), json!("■ full")));
    let at = |id: &str| translate(d(id, "transform").as_str().unwrap());
    assert!(near(at("parcel-1").0, 309.96, 0.05) && near(at("parcel-1").1, 70.0, 0.05), "{:?}", at("parcel-1"));
    assert!(near(at("parcel-2").0, 90.04, 0.05) && near(at("parcel-2").1, 210.0, 0.05), "{:?}", at("parcel-2"));
    assert_eq!((d("tracked", "hidden"), d("tracked", "shown"), d("tracked", "clones")), (json!(true), json!(false), json!([])), "no position yet");
    // Every function a transform can be written with (the browser's own list of them) somewhere in
    // the drawing: on an element of its own, a marker's, a level's, a group around a marker, a level
    // or a path.
    let grammar = page.eval(&format!(
        r#"(() => {{ const doc = new DOMParser().parseFromString({}, "image/svg+xml"); const used = new Set(); for (const el of doc.querySelectorAll("*")) {{ const l = el.transform?.baseVal; if (l) for (let i = 0; i < l.numberOfItems; i++) used.add(l.getItem(i).type); }} const all = Object.keys(SVGTransform).filter((k) => k.startsWith("SVG_TRANSFORM_") && k !== "SVG_TRANSFORM_UNKNOWN"); return [all.length, all.filter((k) => !used.has(SVGTransform[k]))]; }})()"#,
        serde_json::to_string(&ex.svg).unwrap()
    ));
    assert_eq!(grammar, json!([6, []]), "every transform function in the drawing");
    // Each function a transform takes (the browser's list) as a marker's own, on a marker drawn off
    // its own origin, and one such marker's own transform a list of two; and every place a transform
    // stands, an element's own and a group's around it, for a bound element, a marker and the path a
    // marker follows.
    let ids = |show: bool| serde_json::to_string(&on_layout.iter().filter(|(_, s)| (s == "move") == show).map(|(i, _)| i.as_str()).collect::<Vec<_>>()).unwrap();
    let followed_paths = serde_json::to_string(&paths.values().collect::<std::collections::BTreeSet<_>>()).unwrap();
    let stands = page.eval(&format!(
        r#"(() => {{ const doc = new DOMParser().parseFromString({}, "image/svg+xml"); const MARKERS = {}, BOUND = {}, PATHS = {};
  const types = new Set(); let list = false;
  for (const id of MARKERS) {{ const live = document.getElementById(id); const was = live.classList.contains("sv-hidden"); live.classList.remove("sv-hidden"); const b = live.getBBox(); if (was) live.classList.add("sv-hidden"); const off = Math.abs(b.x + b.width / 2) > 1 || Math.abs(b.y + b.height / 2) > 1; const l = doc.getElementById(id).transform.baseVal; if (off) {{ for (let i = 0; i < l.numberOfItems; i++) types.add(l.getItem(i).type); if (l.numberOfItems > 1) list = true; }} }}
  const all = Object.keys(SVGTransform).filter((k) => k.startsWith("SVG_TRANSFORM_") && k !== "SVG_TRANSFORM_UNKNOWN");
  const seen = {{}};
  const add = (cell, l) => {{ for (let i = 0; i < (l?.numberOfItems ?? 0); i++) (seen[cell] ??= new Set()).add(l.getItem(i).type); }};
  for (const [role, list] of [["bound", BOUND], ["marker", MARKERS], ["path", PATHS]]) for (const id of list) {{ const el = doc.getElementById(id); add(role + " own", el.transform?.baseVal); for (let a = el.parentElement; a && a !== doc.documentElement; a = a.parentElement) add(role + " group", a.transform?.baseVal); }}
  const cells = Object.entries(seen).map(([c, s]) => c + " lacks " + all.filter((k) => !s.has(SVGTransform[k])).length);
  return [all.filter((k) => !types.has(SVGTransform[k])), list, cells.sort()]; }})()"#,
        serde_json::to_string(&ex.svg).unwrap(),
        ids(true),
        ids(false),
        followed_paths
    ));
    assert_eq!(stands, json!([[], true, ["bound group lacks 0", "bound own lacks 0", "marker group lacks 0", "marker own lacks 0", "path group lacks 0", "path own lacks 0"]]), "every transform function as a marker's own, and every function at every place a transform stands, for a bound element, a marker and a path");
    // Every element with no area a level can be on, by the page's own branches: a line and a path,
    // each with no width and with no height.
    let flat = page.eval(r#"[...new Set([...document.querySelectorAll("svg.plant .sv-show-level[data-object]")].flatMap((el) => { const b = el.getBBox(), k = el instanceof SVGLineElement ? "line" : el.tagName; return [b.width === 0 ? k + " no width" : null, b.height === 0 ? k + " no height" : null].filter(Boolean); }))].sort()"#);
    assert_eq!(flat, json!(["line no height", "line no width", "path no height", "path no width"]), "every level with no area");
    // A marker drawn at its own size through its own transform (the rect's twice, the circle's half
    // again, beside an unscaled one of the same radius), not set aside by its place.
    let width = |id: &str| d(id, "rect")[2].as_f64().unwrap();
    assert!(near(width("m-rect") / width("m-polygon"), 2.0, 0.03) && near(width("m-circle") / width("m-polygon"), 1.5, 0.03), "m-rect {} m-circle {} m-polygon {}", width("m-rect"), width("m-circle"), width("m-polygon"));

    // Around it: the headline in its states, the objects not normal in order of weight.
    let around = around_when(&mut page, "a.attention.length === 5 && a.headline.length === 2");
    assert_eq!(around["headline"], json!([["speed", "1.5", "mono st-text-normal"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention_head"], json!("NEEDS ATTENTION · 5"));
    let quiet = json!([
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
        ["keyed", "◌", "mark st-stale", "Keyed no value"],
    ]);
    assert_eq!(around["attention"], quiet);

    // Every level within its element's outline, every element a level can be bound to among them:
    // each the reader lets a marker follow (its own list), a text and a group.
    let outlines = page.eval(LEVEL_OUTLINES);
    let tags: std::collections::BTreeSet<&str> = outlines.as_array().unwrap().iter().filter_map(|v| v.as_str()?.strip_prefix("tag ")).collect();
    let every_element: std::collections::BTreeSet<&str> = veredas_supervisao::svg::FOLLOWABLE.into_iter().chain(["text", "g"]).collect();
    assert_eq!(tags, every_element, "{outlines}");
    let issues: Vec<&Value> = outlines.as_array().unwrap().iter().filter(|v| !v.as_str().unwrap().starts_with("tag ")).collect();
    assert!(issues.is_empty(), "{issues:?}");

    // Every element the layout binds and draws takes the selection by a click and carries it
    // alone, marked on every shape it draws (outlet 1's group, its shape writing its own stroke,
    // among them).
    for (id, _) in &on_layout {
        if drawn[id]["hidden"] == json!(true) {
            continue;
        }
        click_at(&mut page, &drawn[id]["rect"]);
        settle(&mut page, &format!("{id} selected by a click"), &[id.as_str()]);
    }
    // Every way the selection changes: a click on the drawing's background clears it; a click on
    // each attention row selects its object, drawn on what the object draws (its element, every
    // clone of a keyed marker, nothing for a marker with no place or an object with no element).
    deselect(&mut page);
    settle(&mut page, "the selection cleared by a click on the background", &[]);
    pick_every_row(&mut page, &drawn, &settle);
    // Each way the side panel's sparkline names its series: a keyed marker picked whole reads every
    // key's, an object bound to one key that key's (their rows picked just now, before any key was
    // clicked).
    let query = |u: &str, k: &str| u.split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(String::from);
    let keyed_sparks = |page: &mut Browser| page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?") && query(u, "source").as_deref() == Some("sorter") && query(u, "series").as_deref() == Some("at")).map(|u| query(&u, "key")).collect::<Vec<_>>();
    let before_click = keyed_sparks(&mut page);
    assert!(before_click.contains(&None) && before_click.contains(&Some("A1".to_string())) && before_click.iter().all(|k| k.is_none() || k.as_deref() == Some("A1")), "the marker whole, the object bound to A1 by its key: {before_click:?}");

    // A click of the pointer on outlet 3 (full, so its fill covers it) selects it: the side panel
    // holds its value, state, sparkline (its own series over its last half hour) and offered
    // write, gated by portal's switch; the selection is drawn in Veredas' green (Q22).
    click_at(&mut page, &drawn["outlet-3"]["rect"]);
    let around = around_when(&mut page, "a.selected === 'Outlet 3' && a.spark > 0");
    assert_eq!((around["badge"].clone(), around["value"].clone()), (json!("■ full"), json!("100 %")));
    assert_eq!(around["buttons"], json!([["Release", true]]), "offered, and off while the switch is off");
    assert_eq!(around["gate"], json!("Writes go through portal. Its write switch is off."));
    let sparks: Vec<String> = page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?")).collect();
    assert!(
        sparks.iter().any(|u| query(u, "source").as_deref() == Some("plc") && query(u, "series").as_deref() == Some("fill3") && query(u, "key").is_none() && query(u, "to").zip(query(u, "from")).is_some_and(|(t, f)| t.parse::<i64>().unwrap() - f.parse::<i64>().unwrap() == 1_800_000)),
        "the sparkline reads outlet 3's own series, whole, over half an hour: {sparks:?}"
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
    let around = around_when(&mut page, "a.attention.length === 8 && a.headline[0][1] === '0.2'");
    // The Alarms view: every state it writes, none in the accent.
    page.eval("location.hash = 'alarms'; true");
    page.wait_for("document.querySelectorAll('.dt tbody tr').length >= 7 || null", Duration::from_secs(10));
    assert_eq!(accent_on_states(&mut page), json!([]), "the Alarms view: a state drawn in the accent");
    let alarm_places = places_told_apart(&mut page);
    assert_eq!(alarm_places[1], json!([]), "the Alarms view: a place painting two states alike: {}", alarm_places[0]);
    assert!(alarm_places[0].as_array().unwrap().iter().any(|p| p.as_str().unwrap().starts_with("strip-items")), "the alarm strip among the places: {}", alarm_places[0]);
    // The markers moved while the Alarms view is in front: each placed anew once the layout is.
    plc.set("p1", 40.0);
    assert!(wait(Duration::from_secs(20), || object(&get(addr, "/s/loop/objects.json").1, "parcel-1")["place"] == json!(0.4)));
    std::thread::sleep(Duration::from_millis(300));
    page.eval("location.hash = 'layout'; true");
    settle(&mut page, "the layout shown again, its markers moved meanwhile", &["outlet-3"]);
    assert_eq!(around["headline"], json!([["speed", "0.2", "mono st-text-warn"], ["read-rate", "—", "mono st-text-stale"]]));
    assert_eq!(around["attention"], json!([
        ["infeed", "!", "mark st-alarm", "Infeed fault"],
        ["speed", "■", "mark st-warn", "Loop speed slow"],
        ["outlet-3", "■", "mark st-warn", "Outlet 3 full"],
        ["outlet-2", "■", "mark st-near", "Outlet 2 near full"],
        ["outlet-1", "▨", "mark st-disabled", "Outlet 1 off"],
        ["read-rate", "◌", "mark st-stale", "Read rate no value"],
        ["tracked", "◌", "mark st-stale", "Tracked no value"],
        ["keyed", "◌", "mark st-stale", "Keyed no value"],
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
    let around = around_when(&mut page, "a.attention.length === 5 && a.headline[0][1] === '1.5'");
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
    assert_eq!((drawn["tracked"]["hidden"].clone(), drawn["tracked"]["shown"].clone(), tags.len()), (json!(true), json!(false), 2), "{tags:?}");
    assert!(tags.iter().all(|(_, t)| *t == json!("■ far")), "{tags:?}");
    let around = around_when(&mut page, "a.attention.length === 4 && a.attention[1][0] === 'tracked'");
    assert_eq!(around["attention"][1], json!(["tracked", "■", "mark st-warn", "Tracked far"]));
    // Drawn through its group's scale: a clone twice the radius it is written with (4, beside a
    // marker of 3).
    let clone_w = drawn["tracked"]["clones"][0]["rect"][2].as_f64().unwrap();
    assert!(near(clone_w / width("m-polygon"), 8.0 / 3.0, 0.04), "a clone {clone_w} wide, m-polygon {}", width("m-polygon"));
    // Every row again, the keyed marker's picked whole: every one of its clones carries it.
    pick_every_row(&mut page, &drawn, &settle);
    // Both keys said again, as they were, so neither is forgotten (6 s after its last) before A1 is
    // clicked, however slow the rows were to pick.
    let now = store_now();
    append(&log, &[format!("{} POS item=A1 at=10", stamp(now)), format!("{} POS item=B2 at=50", stamp(now))]);
    let positions = || host.session("loop").unwrap().live().store.query_rows("positions", &veredas_portal::store::RowsQuery { limit: 1, ..Default::default() }).map(|p| p.total).unwrap_or(0);
    assert!(wait(Duration::from_secs(20), || positions() >= 4), "both keys said again: {} rows", positions());
    let a1 = drawn["tracked"]["clones"].as_array().unwrap().iter().find(|c| c["key"] == json!("A1")).unwrap()["rect"].clone();
    let sparks_before = keyed_sparks(&mut page).len();
    click_at(&mut page, &a1);
    let around = around_when(&mut page, "a.selected === 'Tracked · A1'");
    assert_eq!(around["value"], json!("10"), "A1's own value, not the latest of any key");
    settle(&mut page, "A1 selected", &["tracked A1"]);
    // A key picked reads that key's sparkline.
    page.wait_for(r#"document.querySelector(".side .spark-wrap")?.children.length > 0 || null"#, Duration::from_secs(10));
    let after_click = keyed_sparks(&mut page);
    assert!(after_click.len() > sparks_before && after_click[sparks_before..].iter().all(|k| k.as_deref() == Some("A1")), "A1 picked reads A1's sparkline: {after_click:?}");
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
    // Placed by the element's box as it stands at the view's scale, whatever changed the scale (the
    // fit, a zoom in, a zoom out, Fit): Chromium lays a text out at the size it is drawn, so a
    // text's box moves with the scale, and what was placed by the box measured once would not.
    for (step, act) in [("the fit", ""), ("a zoom in", "Zoom in"), ("another zoom in", "Zoom in"), ("a zoom out", "Zoom out"), ("Fit", "Fit"), ("a smaller window", "resize"), ("the window back", "restore")] {
        match act {
            "" => {}
            "resize" => {
                page.call("Emulation.setDeviceMetricsOverride", json!({ "width": 900, "height": 640, "deviceScaleFactor": 1, "mobile": false }));
            }
            "restore" => {
                page.call("Emulation.clearDeviceMetricsOverride", json!({}));
            }
            _ => {
                page.eval(&format!(r#"(() => {{ document.querySelector('.zoom button[title="{act}"]').click(); return true; }})()"#));
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        page.eval(r#"new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => done(true))))"#);
        let off = page.eval(PLACED_NOW);
        assert!(off[0].as_u64().unwrap() >= 20 && off[1] == json!([]), "after {step}: placed otherwise than by the box as it stands: {off}");
        page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
        // (The plant away, a marker keeps the place it had or has none; those with one are read.)
        let moved = page.eval(MARKERS_NOW);
        assert!(moved[1] == json!([]), "after {step}: a marker placed otherwise than by its box as it stands: {moved}");
        assert_controls_clear(&mut page, &format!("after {step}"));
    }
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 300.0], 20, "Fit after zooms");
    drop(page);
    host.stop();

    // The bundle reads the session's binding, layout and feed, and styles every class.
    for needle in ["/binding", "/layout.svg", "/objects", "getPointAtLength", "NEEDS ATTENTION"] {
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

/// The drawing for the levels with no area and what the page adds around the drawing: a line with no
/// width and one with no height under every transform function the browser lists (each a stroke
/// whose width across the screen the function changes or keeps), at both places a transform stands
/// (its own and a group's around it); three lines in near, warn and alarm; a slanted straight path
/// and a polyline of collinear points; a level with an area inside a plain group; a group holding a
/// line with no height, and one holding a line with no width under a scale of its own; a bound group
/// holding a text that writes its fill alone and a span writing its own, and two holding a text that
/// writes none of the paint (one a child, one two groups deep, writing its stroke's width alone),
/// each beside the same group unbound; and an element at the drawing's bottom edge in a state.
const NO_AREA_SVG: &str = r##"  <line id="nv-vt-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="translate(20 240)"/>
  <line id="nv-vs-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="translate(50 240) scale(2 0.5)"/>
  <line id="nv-vr-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="translate(80 250) rotate(90)"/>
  <line id="nv-vk-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="translate(110 240) skewY(30)"/>
  <line id="nv-vm-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="matrix(0 2 -0.5 0 150 250)"/>
  <g transform="translate(180 240)"><line id="nv-vt-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(210 240) scale(2 0.5)"><line id="nv-vs-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(240 250) rotate(90)"><line id="nv-vr-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(270 240) skewY(30)"><line id="nv-vk-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="matrix(0 2 -0.5 0 310 250)"><line id="nv-vm-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <line id="nv-ht-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="translate(20 300)"/>
  <line id="nv-hs-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="translate(50 300) scale(0.5 2)"/>
  <line id="nv-hr-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="translate(90 290) rotate(90)"/>
  <line id="nv-hk-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="translate(110 300) skewX(30)"/>
  <line id="nv-hm-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="matrix(0 2 -0.5 0 150 290)"/>
  <g transform="translate(180 300)"><line id="nv-ht-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(210 300) scale(0.5 2)"><line id="nv-hs-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(250 290) rotate(90)"><line id="nv-hr-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="translate(270 300) skewX(30)"><line id="nv-hk-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <g transform="matrix(0 2 -0.5 0 310 290)"><line id="nv-hm-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <line id="nv-near" x1="20" y1="350" x2="20" y2="380" stroke="#3a3a40" stroke-width="6"/>
  <line id="nv-warn" x1="50" y1="350" x2="50" y2="380" stroke="#3a3a40" stroke-width="6"/>
  <line id="nv-alarm" x1="80" y1="350" x2="80" y2="380" stroke="#3a3a40" stroke-width="6"/>
  <path id="nv-diag" d="M 110 345 L 150 385" stroke="#3a3a40" stroke-width="6"/>
  <polyline id="nv-diag2" points="165,345 185,365 205,385" stroke="#3a3a40" stroke-width="6" fill="#3a3a40"/>
  <g id="nv-wrap"><rect id="nv-box" x="225" y="350" width="20" height="30"/></g>
  <g id="nv-grp"><rect x="260" y="350" width="40" height="24"/><text x="264" y="366" font-size="12" fill="#ffffff">A<tspan fill="#ffff00">B</tspan></text></g>
  <g id="nv-free"><rect x="320" y="350" width="40" height="24"/><text x="324" y="366" font-size="12" fill="#ffffff">A<tspan fill="#ffff00">B</tspan></text></g>
  <line id="nv-vf-o" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6" transform="matrix(-1 0 0 1 345 240)"/>
  <g transform="matrix(-1 0 0 1 375 240)"><line id="nv-vf-g" x1="0" y1="0" x2="0" y2="20" stroke="#3a3a40" stroke-width="6"/></g>
  <line id="nv-hf-o" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6" transform="matrix(-1 0 0 1 365 300)"/>
  <g transform="matrix(-1 0 0 1 395 300)"><line id="nv-hf-g" x1="0" y1="0" x2="20" y2="0" stroke="#3a3a40" stroke-width="6"/></g>
  <g id="nv-grp6" style="fill:#cccccc;stroke:#000000;stroke-width:2;stroke-dasharray:4,1;fill-opacity:0.9;stroke-opacity:0.8"><rect x="260" y="380" width="40" height="16"/><text x="264" y="392" style="font-size:8px">O1</text></g>
  <g id="nv-free6" style="fill:#cccccc;stroke:#000000;stroke-width:2;stroke-dasharray:4,1;fill-opacity:0.9;stroke-opacity:0.8"><rect x="320" y="380" width="40" height="16"/><text x="324" y="392" style="font-size:8px">O1</text></g>
  <g id="nv-gline"><path d="M 120,325 H 180" style="fill:none;stroke:#3a3a40;stroke-width:6"/></g>
  <g id="nv-gvline"><line x1="0" y1="0" x2="0" y2="20" transform="translate(200 318) scale(2 1)" style="stroke:#3a3a40;stroke-width:6"/></g>
  <g id="nv-grpd" style="fill:#cccccc;stroke:#000000;stroke-width:2;stroke-dasharray:4,1;fill-opacity:0.9;stroke-opacity:0.8"><rect x="260" y="318" width="40" height="16"/><g><g><text x="264" y="330" style="font-size:8px;stroke-width:0.5">O2</text></g></g></g>
  <g id="nv-freed" style="fill:#cccccc;stroke:#000000;stroke-width:2;stroke-dasharray:4,1;fill-opacity:0.9;stroke-opacity:0.8"><rect x="320" y="318" width="40" height="16"/><g><g><text x="324" y="330" style="font-size:8px;stroke-width:0.5">O2</text></g></g></g>
  <rect id="nv-edge" x="370" y="372" width="24" height="28"/>
  <g id="nv-gpipe"><path d="M 120,336 H 150" style="fill:none;stroke:#3a3a40;stroke-width:6"/><path d="M 0,0 H 15" transform="translate(150 336) scale(2 1)" style="fill:none;stroke:#3a3a40;stroke-width:6"/></g>
  <path id="nv-walls" d="m 20,268 h 50 m -50,12 h 50" style="fill:none;stroke:#3a3a40;stroke-width:4"/>
  <path id="nv-ell" d="M 100,268 H 140 M 140,268 V 284" style="fill:none;stroke:#3a3a40;stroke-width:4"/>
  <g id="nv-gell"><path d="M 180,268 H 220" style="fill:none;stroke:#3a3a40;stroke-width:4"/><path d="M 220,268 V 284" style="fill:none;stroke:#3a3a40;stroke-width:4"/></g>
  <g id="nv-ghid"><path d="M 250,276 H 300" style="fill:none;stroke:#3a3a40;stroke-width:6"/><path d="M 330,276 H 380" style="display:none;fill:none;stroke:#3a3a40;stroke-width:6"/></g>
  <g id="nv-gpoly"><polyline points="20,328 50,328 80,328" style="fill:none;stroke:#3a3a40;stroke-width:6"/></g>
  <g id="nv-gpgon"><polygon points="20,340 50,340 80,340" style="fill:none;stroke:#3a3a40;stroke-width:6"/></g>
</svg>"##;

/// The levels with no area whose stroke the page fills over, each with its level: the twenty-four
/// under a transform (each function and a mirroring matrix, upright and level, at an element's own
/// place and a group's) and the three in a state at 85 %, the slanted path and polyline at 100 %.
fn no_area_levels() -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for o in ["v", "h"] {
        // Each transform function, and a mirroring matrix (Inkscape's Flip writes one).
        for f in ["t", "s", "r", "k", "m", "f"] {
            for at in ["o", "g"] {
                out.push((format!("nv-{o}{f}-{at}"), 0.85));
            }
        }
    }
    for s in ["near", "warn", "alarm"] {
        out.push((format!("nv-{s}"), 0.85));
    }
    out.push(("nv-diag".into(), 1.0));
    out.push(("nv-diag2".into(), 1.0));
    // A group with no area: its shapes' strokes are what it draws (a pipe of two segments, one under
    // a scale of its own, among them).
    out.push(("nv-gline".into(), 0.85));
    out.push(("nv-gvline".into(), 0.85));
    out.push(("nv-gpipe".into(), 0.85));
    for (id, _) in NO_AREA_GROUPS {
        if !out.iter().any(|(o, _)| o == id) {
            out.push((id.to_string(), 0.85));
        }
    }
    out
}

/// The groups with no area the test draws, each with the kind of shape it holds: every kind the
/// reader lets an object draw on (`svg::DRAWN`, a group and a text apart) that can be drawn with no
/// area (`NOTHING_DRAWN` the others, which draw nothing at all with none).
const NO_AREA_GROUPS: [(&str, &str); 6] = [("nv-gline", "path"), ("nv-gvline", "line"), ("nv-gpipe", "path"), ("nv-gpoly", "polyline"), ("nv-gpgon", "polygon"), ("nv-ghid", "path")];

/// Elements with no area whose runs lie on more than one line, each with points of its own
/// coordinates on its strokes (the fill over them at a full level) and off them (between two walls,
/// inside an L's corner: no fill there): two walls Inkscape's Combine made one path, an L of two
/// subpaths, a group of an L of two paths.
const RUNS: [(&str, &[(f64, f64)], &[(f64, f64)]); 3] = [
    ("nv-walls", &[(45.0, 268.0), (45.0, 280.0)], &[(45.0, 274.0)]),
    ("nv-ell", &[(120.0, 268.0), (140.0, 276.0)], &[(120.0, 278.0)]),
    ("nv-gell", &[(200.0, 268.0), (220.0, 276.0)], &[(200.0, 278.0)]),
];

/// Each of `RUNS` filled over its strokes and nowhere between them, by what the browser hits.
const RUNS_FILLED: &str = r#"((cases) => cases.flatMap(([id, on, off]) => {
  const el = document.getElementById(id), fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
  if (!el || !fill) return [`${id}: no element or no fill`];
  const m = el.getScreenCTM();
  const prev = fill.style.pointerEvents;
  fill.style.pointerEvents = "visiblePainted";
  const hit = ([x, y]) => { const q = new DOMPoint(x, y).matrixTransform(m); return document.elementFromPoint(q.x, q.y) === fill; };
  const out = [...on.filter((p) => !hit(p)).map((p) => `${id}: no fill over its stroke at ${p}`), ...off.filter(hit).map((p) => `${id}: filled off its strokes at ${p}`)];
  fill.style.pointerEvents = prev;
  return out;
}))"#;

/// The kinds of shape that draw nothing with no area: a rect with no width or height, a circle or
/// an ellipse with no radius.
const NOTHING_DRAWN: [&str; 3] = ["rect", "circle", "ellipse"];

/// Each level with no area against its stroke as the browser paints it: on a line the screen keeps
/// level or upright, the stroke's painted width across it (scanned from its middle outward by what
/// the browser hits) is the fill's across it, and the fill covers it; on a slanted one, the fill is
/// painted all along the stroke (a full level); and on every one the fill's colour is not its
/// track's. With, for each place a transform stands (an element's own, a group's around it), the
/// transform functions the no-area levels stand under, from the browser's own list of them.
const NO_AREA_FILLS: &str = r##"(() => {
  const out = [];
  const pt = (el, at) => { const p = el.getPointAtLength(at); return new DOMPoint(p.x, p.y).matrixTransform(el.getScreenCTM()); };
  for (const [id, level] of LEVELS) {
    const el = document.getElementById(id);
    const fill = document.querySelector(`.sv-fill[data-for="${id}"]`);
    if (!el || !fill) { out.push(`${id}: no element or no fill`); continue; }
    // A group's stroke is its shapes', hit as the group's: read across at its first, and along the
    // whole group, the fill from the first shape's start to the last one's end and taking each one's
    // middle.
    const shape = el instanceof SVGGeometryElement ? el : el.querySelector("path, line, polyline, polygon, rect");
    // A group's shapes by their boxes' corners and middles (a polygon closes on its start, so its
    // ends say nothing of its reach).
    const box = (g) => { const b = g.getBBox(), m = g.getScreenCTM(); return [[b.x, b.y], [b.x + b.width, b.y + b.height], [b.x + b.width / 2, b.y + b.height / 2]].map(([x, y]) => new DOMPoint(x, y).matrixTransform(m)); };
    if (!(el instanceof SVGGeometryElement)) {
      const shapes = [...el.querySelectorAll("path, line, polyline, polygon, rect")].filter((g) => g.checkVisibility());
      const ends = shapes.flatMap((g) => box(g).slice(0, 2));
      const r = fill.getBoundingClientRect();
      const xs = ends.map((p) => p.x), ys = ends.map((p) => p.y);
      const flatGroup = Math.max(...ys) - Math.min(...ys) < 0.5;
      const [lo, hi, rlo, rhi] = flatGroup ? [Math.min(...xs), Math.max(...xs), r.left, r.right] : [Math.min(...ys), Math.max(...ys), r.top, r.bottom];
      // Across a flat group the fill spans every shape from end to end; up an upright one it rises
      // from the lowest shape's foot (to its level).
      if ((flatGroup && rlo > lo + 1) || rhi < hi - 1) out.push(`${id}: its fill [${rlo.toFixed(2)}, ${rhi.toFixed(2)}] short of its shapes [${lo.toFixed(2)}, ${hi.toFixed(2)}]`);
      if (flatGroup && (rlo < lo - 1 || rhi > hi + 1)) out.push(`${id}: its fill [${rlo.toFixed(2)}, ${rhi.toFixed(2)}] past its shapes [${lo.toFixed(2)}, ${hi.toFixed(2)}]`);
      const prev = fill.style.pointerEvents;
      fill.style.pointerEvents = "visiblePainted";
      for (const [i, g] of shapes.entries()) {
        const m = box(g)[2];
        const y = flatGroup ? r.bottom - 1 : m.y, x = flatGroup ? m.x : r.left + 1;
        if (document.elementFromPoint(x, y) !== fill) out.push(`${id}: its fill missing at its shape ${i}'s middle`);
      }
      fill.style.pointerEvents = prev;
    }
    const len = shape.getTotalLength();
    const [a, b, middle] = el instanceof SVGGeometryElement ? [pt(shape, 0), pt(shape, len), pt(shape, len / 2)] : box(shape);
    const r = fill.getBoundingClientRect();
    const ce = getComputedStyle(el), cf = getComputedStyle(fill);
    if (cf.fill === ce.stroke) out.push(`${id}: its fill painted as its track (${cf.fill})`);
    const upright = Math.abs(a.x - b.x) < 0.5, flat = Math.abs(a.y - b.y) < 0.5;
    if (upright || flat) {
      const mid = middle;
      const hits = (d) => { const x = upright ? mid.x + d : mid.x, y = flat ? mid.y + d : mid.y; const at = document.elementFromPoint(x, y); return at === el || el.contains(at); };
      let lo = 0, hi = 0;
      while (hits(lo - 0.25) && lo > -100) lo -= 0.25;
      while (hits(hi + 0.25) && hi < 100) hi += 0.25;
      const painted = hi - lo + 0.25;
      if (upright) {
        if (Math.abs(r.width - painted) > 1) out.push(`${id}: its fill ${r.width.toFixed(2)} wide over a stroke painted ${painted.toFixed(2)} wide`);
        if (r.left > mid.x + lo + 0.75 || r.right < mid.x + hi - 0.75) out.push(`${id}: its fill [${r.left.toFixed(2)}, ${r.right.toFixed(2)}] short of its stroke [${(mid.x + lo).toFixed(2)}, ${(mid.x + hi).toFixed(2)}]`);
      } else {
        if (Math.abs(r.height - painted * level) > 1) out.push(`${id}: its fill ${r.height.toFixed(2)} high over a stroke painted ${painted.toFixed(2)} across, at ${level}`);
        if (Math.abs(r.bottom - (mid.y + hi + 0.25)) > 1) out.push(`${id}: its fill's foot ${r.bottom.toFixed(2)} off its stroke's ${(mid.y + hi).toFixed(2)}`);
      }
    } else {
      const prev = fill.style.pointerEvents;
      fill.style.pointerEvents = "visiblePainted";
      let missed = 0;
      // Thirteen points inside its ends: an end of a slanted stroke is its box's corner, the fill's
      // edge, where a hit is no verdict.
      for (let k = 0; k <= 12; k++) { const p = pt(shape, (len * (k + 0.5)) / 13); const at = document.elementFromPoint(p.x, p.y); if (at !== fill) missed++; }
      fill.style.pointerEvents = prev;
      if (missed) out.push(`${id}: its fill missing at ${missed} of 13 points along its stroke`);
    }
  }
  const all = Object.keys(SVGTransform).filter((k) => k.startsWith("SVG_TRANSFORM_") && k !== "SVG_TRANSFORM_UNKNOWN");
  const seen = { own: new Set(), group: new Set() };
  // And each orientation: a transform keeping the drawing's handedness and one mirroring it (the
  // sign of its matrix's determinant, the list multiplied out).
  const det = (l) => { let m = new DOMMatrix(); for (let i = 0; i < l.numberOfItems; i++) { const t = l.getItem(i).matrix; m = m.multiply(new DOMMatrix([t.a, t.b, t.c, t.d, t.e, t.f])); } return m.a * m.d - m.b * m.c; };
  for (const [id] of LEVELS) {
    const el = document.getElementById(id);
    const add = (cell, l) => { if (!l?.numberOfItems) return; for (let i = 0; i < l.numberOfItems; i++) seen[cell].add(l.getItem(i).type); seen[cell].add(det(l) < 0 ? "mirrored" : "kept"); };
    add("own", el.transform?.baseVal);
    for (let p = el.parentElement; p && !p.matches("svg"); p = p.parentElement) add("group", p.transform?.baseVal);
  }
  return [out, Object.fromEntries(Object.entries(seen).map(([c, s]) => [c, [...all, "mirrored", "kept"].filter((k) => !s.has(SVGTransform[k] ?? k))]))];
})()"##;

/// What the page adds (the overlay's tags, labels and values, every level's fill, the hatch) read
/// by every property the browser computes (its own list), before and after the drawing's root and
/// every group a fill stands in write paint and text of their own: nothing changes.
const ROOT_PAINT: &str = r##"(() => {
  const plant = document.querySelector("svg.plant");
  const added = [...plant.querySelectorAll(".sv-overlay *, .sv-fill-holder *, #sv-hatch *")];
  const props = [...getComputedStyle(plant)].filter((p) => !p.startsWith("--"));
  const snap = () => added.map((e) => { const c = getComputedStyle(e); return props.map((p) => c.getPropertyValue(p)); });
  const before = snap();
  const WRITE = { fill: "#30d158", stroke: "#30d158", "stroke-width": "3", "stroke-dasharray": "1 1", "stroke-dashoffset": "1", "fill-opacity": "0", "stroke-opacity": "0.5", "stroke-linecap": "round", "stroke-linejoin": "round", "stroke-miterlimit": "1", "paint-order": "stroke", "fill-rule": "evenodd", "font-size": "40", "font-family": "serif", "font-weight": "100", "font-style": "italic", "letter-spacing": "3", "word-spacing": "3", "text-anchor": "end", "dominant-baseline": "hanging", "marker-start": "url(#nv-none)", "marker-mid": "url(#nv-none)", "marker-end": "url(#nv-none)", color: "#30d158", "shape-rendering": "crispEdges", "text-rendering": "optimizeSpeed" };
  const targets = [plant, ...new Set([...plant.querySelectorAll(".sv-fill-holder")].map((h) => h.parentElement))];
  const was = targets.map((t) => Object.keys(WRITE).map((k) => t.getAttribute(k)));
  for (const t of targets) for (const [k, v] of Object.entries(WRITE)) t.setAttribute(k, v);
  const after = snap();
  targets.forEach((t, i) => Object.keys(WRITE).forEach((k, j) => (was[i][j] === null ? t.removeAttribute(k) : t.setAttribute(k, was[i][j]))));
  const out = [];
  added.forEach((e, i) => props.forEach((p, j) => { if (before[i][j] !== after[i][j]) out.push(`${e.tagName} ${e.getAttribute("class") ?? ""} ${e.dataset.for ?? ""} ${p}: ${before[i][j]} -> ${after[i][j]}`); }));
  return [added.length, targets.length, out.slice(0, 30)];
})()"##;

/// A bound group's text, and its span, painted as the same group's unbound; and every tag and every
/// fill of the page stroked with nothing. And the paint the page outranks, read from the page's own
/// sheet (the rule that outranks it on a bound group's shapes and a bound text's spans), each kept on
/// a group's text that writes none of it, its group writing every one, the text a child of the group
/// and two groups deep: the text painted as the same group's unbound in each, each told from the
/// group's state (so the pin is what keeps it); and so in every state of the vocabulary (`STATES`,
/// the crate's own enum), unselected and selected, the group given each as the page gives it.
const PINNED: &str = r##"((STATES) => {
  const rule = [...document.styleSheets].flatMap((s) => { try { return [...s.cssRules]; } catch { return []; } }).find((r) => r.selectorText?.includes("text.sv-obj tspan"));
  const PAINT = rule ? [...rule.style] : [];
  const sig = (el) => { const c = getComputedStyle(el); return PAINT.map((p) => c.getPropertyValue(p)).join(" | "); };
  const b = document.querySelector("#nv-grp text"), f = document.querySelector("#nv-free text");
  const stroked = [...document.querySelectorAll("svg.plant .sv-tag, svg.plant .sv-fill")].filter((e) => getComputedStyle(e).stroke !== "none").map((e) => `${e.getAttribute("class")} ${e.dataset.for}: ${getComputedStyle(e).stroke}`);
  const pairs = [["nv-grp6", "nv-free6"], ["nv-grpd", "nv-freed"]].map(([g, u]) => [document.getElementById(g), document.getElementById(u)]);
  const each = [], states = [];
  for (const [g, u] of pairs) {
    const [bt, ft] = [g.querySelector("text"), u.querySelector("text")];
    for (const p of PAINT) {
      const [bv, fv, gv] = [bt, ft, g].map((e) => getComputedStyle(e).getPropertyValue(p));
      if (bv !== fv) each.push(`${g.id} ${p}: the bound group's text ${bv}, the unbound one's ${fv}`);
      if (gv === fv) each.push(`${g.id} ${p}: the group's state paints it as drawn (${gv}), so nothing is pinned`);
    }
    const was = g.getAttribute("class");
    for (const st of STATES) for (const selected of [false, true]) {
      for (const c of [...g.classList]) if (c.startsWith("sv-state-")) g.classList.remove(c);
      g.classList.add(`sv-state-${st}`);
      g.classList.toggle("sv-selected", selected);
      if (sig(bt) !== sig(ft)) states.push(`${g.id} in ${st}${selected ? ", selected" : ""}: its text ${sig(bt)}, the unbound one's ${sig(ft)}`);
    }
    g.setAttribute("class", was);
  }
  return [document.getElementById("nv-grp").classList.contains("sv-obj"), sig(b) === sig(f) ? null : [sig(b), sig(f)], sig(b.querySelector("tspan")) === sig(f.querySelector("tspan")) ? null : [sig(b.querySelector("tspan")), sig(f.querySelector("tspan"))], stroked, pairs.every(([g, u]) => g.classList.contains("sv-obj") && !u.classList.contains("sv-obj")), PAINT, each, states];
})"##;

#[test]
fn a_level_with_no_area_fills_over_its_stroke_and_the_drawing_reaches_nothing_the_page_adds() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    // A group with no area of every kind of shape the reader lets an object draw on that can have
    // none, each holding that kind (its id's element in the drawing).
    let kinds: std::collections::BTreeSet<&str> = NO_AREA_GROUPS.iter().map(|(_, k)| *k).chain(NOTHING_DRAWN).collect();
    assert_eq!(kinds, veredas_supervisao::svg::DRAWN.into_iter().filter(|k| *k != "g" && *k != "text").collect(), "a group with no area of every kind of shape that can have none");
    for (id, kind) in NO_AREA_GROUPS {
        let at = NO_AREA_SVG.find(&format!("<g id=\"{id}\">")).unwrap_or_else(|| panic!("{id}"));
        assert!(NO_AREA_SVG[at..at + NO_AREA_SVG[at..].find("</g>").unwrap()].contains(&format!("<{kind} ")), "{id} holds a {kind}");
    }
    assert!(ex.svg.contains("viewBox=\"0 0 400 220\"") && ex.svg.ends_with("</svg>\n"));
    ex.svg = ex.svg.replace("viewBox=\"0 0 400 220\"", "viewBox=\"0 0 400 400\"").replace("</svg>\n", NO_AREA_SVG).to_string() + "\n";
    let mut objects = String::new();
    for (id, level) in no_area_levels() {
        let series = if level == 1.0 { "fill3" } else { "fill2" };
        let rule = ["near", "warn", "alarm"].iter().find(|s| id == format!("nv-{s}")).map(|s| format!("states = [ {{ above = 50, state = \"{s}\" }} ]\n")).unwrap_or_default();
        objects.push_str(&format!("\n[[object]]\nid = \"{id}\"\nvalue = {{ source = \"plc\", series = \"{series}\" }}\nshow = \"level\"\n{rule}"));
    }
    for (id, _, _) in RUNS {
        objects.push_str(&format!("\n[[object]]\nid = \"{id}\"\nvalue = {{ source = \"plc\", series = \"fill3\" }}\nshow = \"level\"\n"));
    }
    objects.push_str("\n[[object]]\nid = \"nv-box\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"nv-grp\"\nvalue = { source = \"plc\", series = \"fill1\" }\n\n[[object]]\nid = \"nv-grp6\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"nv-grpd\"\nvalue = { source = \"plc\", series = \"fill1\" }\nshow = \"level\"\n\n[[object]]\nid = \"nv-edge\"\nvalue = { source = \"plc\", series = \"fill2\" }\nstates = [ { above = 50, state = \"warn\" } ]\n");
    ex.binding = ex.binding.replacen("\n[[dashboard]]", &format!("{objects}\n[[dashboard]]"), 1);
    let path = write_example(dir.path(), &ex);
    std::fs::write(dir.path().join("loop/sorts.log"), "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    let mut page = Browser::open(&format!("http://{addr}/s/loop/"));
    let levels = no_area_levels();
    let ids = serde_json::to_string(&levels.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>()).unwrap();
    // Every one painted at its level and in its state, then every slide run.
    page.wait_for(
        &format!(r#"(() => {{ const ids = {ids}; return ids.every((id) => {{ const f = document.querySelector(`.sv-fill[data-for="${{id}}"]`); return f && parseFloat(f.getAttribute("height")) > 0; }}) && document.getElementById("nv-alarm").classList.contains("sv-state-alarm") && document.getElementById("nv-near").classList.contains("sv-state-near") && document.getElementById("nv-warn").classList.contains("sv-state-warn") && document.getElementById("nv-box").classList.contains("sv-state-normal") || null; }})()"#),
        Duration::from_secs(20),
    );
    page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
    let got = page.eval(&format!("(() => {{ const LEVELS = {};\nreturn {NO_AREA_FILLS};\n}})()", serde_json::to_string(&levels).unwrap()));
    assert_eq!(got[0], json!([]), "a level with no area not filled over its stroke as painted");
    assert_eq!(got[1], json!({ "own": [], "group": [] }), "every transform function and both orientations a level with no area stands under, at both places");
    // Runs on more than one line, each filled over its strokes, never between them.
    let runs: Vec<Value> = RUNS.iter().map(|(id, on, off)| json!([id, on.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>(), off.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>()])).collect();
    page.wait_for(&format!(r#"{}.every((id) => {{ const f = document.querySelector(`.sv-fill[data-for="${{id}}"]`); return f && parseFloat(f.getAttribute("height")) > 0; }}) || null"#, serde_json::to_string(&RUNS.iter().map(|(id, _, _)| *id).collect::<Vec<_>>()).unwrap()), Duration::from_secs(20));
    assert_eq!(page.eval(&format!("{RUNS_FILLED}({})", serde_json::to_string(&runs).unwrap())), json!([]), "runs on more than one line filled over their strokes alone");
    let root = page.eval(ROOT_PAINT);
    assert!(root[0].as_u64().unwrap() > 60 && root[1].as_u64().unwrap() >= 3, "what the page adds, and where: {root}");
    assert_eq!(root[2], json!([]), "paint the drawing writes reaching what the page adds");
    let pinned = page.eval(&format!("{PINNED}({})", serde_json::to_string(&every_state()).unwrap()));
    assert_eq!(
        pinned,
        json!([true, null, null, [], true, ["fill", "stroke", "stroke-width", "stroke-dasharray", "fill-opacity", "stroke-opacity"], [], []]),
        "a bound group's text and its span as drawn, no tag or fill stroked; the paint the page outranks (its own rule's list) each kept on a group's text, a child or two groups deep, in every state, selected or not"
    );
    // What the page writes beside the drawing, at its top edge (the outlets' labels) and its bottom
    // (the mark and word under the element there in a state), inside the view at its fit; and again
    // after a zoom and Fit.
    page.wait_for(r#"(document.querySelector('.sv-tag[data-for="nv-edge"]')?.textContent ?? "") !== "" || null"#, Duration::from_secs(10));
    page.wait_for(r#"new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => done(true))))"#, Duration::from_secs(5));
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 400.0], 40, "at the first fit");
    page.eval(r#"(() => { document.querySelector('.zoom button[title="Zoom in"]').click(); document.querySelector('.zoom button[title="Fit"]').click(); return true; })()"#);
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 400.0], 40, "after a zoom and Fit");
    // The view's own handlers, read on the view itself: a zoom kept across a change of the feed, a
    // pan (a drag on the drawing) kept across one, and while fitted, the word under the element at
    // the drawing's bottom edge unsaid and said again leaving the frame as it grew (never shrinking).
    let vb = |page: &mut Browser| page.eval(r#"(() => { const v = document.querySelector("svg.plant").viewBox.baseVal; return [v.x, v.y, v.width, v.height]; })()"#);
    let frames = r#"new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => done(true))))"#;
    let click = |page: &mut Browser, title: &str| {
        page.eval(&format!(r#"(() => {{ document.querySelector('.zoom button[title="{title}"]').click(); return true; }})()"#));
        page.eval(frames);
    };
    let feed_to = |page: &mut Browser, fill: f64, state: &str| {
        bench.server("plc").unwrap().set("fill2", fill);
        page.wait_for(&format!(r#"document.getElementById("nv-edge")?.classList.contains("sv-state-{state}") ? true : null"#), Duration::from_secs(20));
        page.eval(frames);
        std::thread::sleep(Duration::from_millis(300));
    };
    let fit = vb(&mut page);
    click(&mut page, "Zoom in");
    let zoomed = vb(&mut page);
    assert!(zoomed[2].as_f64().unwrap() < fit[2].as_f64().unwrap() - 1.0, "a zoom in narrows the view: {zoomed} from {fit}");
    feed_to(&mut page, 10.0, "normal");
    assert_eq!(vb(&mut page), zoomed, "the zoom kept across a change of the feed");
    click(&mut page, "Fit");
    let fitted = vb(&mut page);
    page.eval(r#"(() => { const s = document.querySelector("svg.plant"); const r = s.getBoundingClientRect(); const at = (x, y) => ({ clientX: r.left + x, clientY: r.top + y, bubbles: true, button: 0, buttons: 1 }); s.dispatchEvent(new MouseEvent("mousedown", at(200, 200))); s.dispatchEvent(new MouseEvent("mousemove", at(260, 230))); window.dispatchEvent(new MouseEvent("mouseup", at(260, 230))); return true; })()"#);
    page.eval(frames);
    let panned = vb(&mut page);
    assert!(panned[0] != fitted[0] && panned[1] != fitted[1] && panned[2] == fitted[2], "a drag pans the view: {panned} from {fitted}");
    feed_to(&mut page, 85.0, "warn");
    assert_eq!(vb(&mut page), panned, "the pan kept across a change of the feed");
    click(&mut page, "Fit");
    let framed = vb(&mut page);
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 400.0], 40, "fitted with the edge's word said");
    feed_to(&mut page, 10.0, "normal");
    assert_eq!(vb(&mut page), framed, "the frame never shrinks while the view stays fitted: the edge's word unsaid");
    feed_to(&mut page, 85.0, "warn");
    assert_eq!(vb(&mut page), framed, "the frame never shrinks while the view stays fitted: the edge's word said again");
    drop(page);
    host.stop();
}

/// The page draws a drawing as Inkscape saves it (Q25 decided: layouts are drawn in Inkscape): the
/// reference's drawing written by hand to Inkscape 1.3's format (`INKSCAPE_LOOP`; no file saved by
/// Inkscape has gone through the reader or the page, the limit). Nothing outside the SVG namespace
/// reaches the page's drawing (Inkscape's elements dropped on read), Inkscape's attributes kept and
/// read by nothing; every bound element shown in its state, each level's fill inside its element to
/// its level, each label above its element and each tag under it, centred; the markers at their
/// places along the loop through the layer's own transform; the hidden layer left hidden.
#[test]
fn the_page_draws_a_drawing_saved_as_inkscape_saves_it() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    ex.svg = INKSCAPE_LOOP.to_string();
    let path = write_example(dir.path(), &ex);
    std::fs::write(dir.path().join("loop/sorts.log"), "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    let mut page = Browser::open(&format!("http://{addr}/s/loop/"));
    page.wait_for(
        r#"(() => { const s = (id) => document.getElementById(id); return s("outlet-3")?.classList.contains("sv-state-warn") && s("outlet-2").classList.contains("sv-state-near") && s("parcel-1").getAttribute("transform") && s("parcel-2").getAttribute("transform") ? true : null; })()"#,
        Duration::from_secs(20),
    );
    std::thread::sleep(Duration::from_millis(500));
    page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
    let foreign = page.eval(r#"[...document.querySelectorAll("svg.plant *")].filter((e) => e.namespaceURI !== "http://www.w3.org/2000/svg").map((e) => e.tagName)"#);
    assert_eq!(foreign, json!([]), "nothing outside the SVG namespace in the page's drawing");
    assert_eq!(page.eval(r#"document.getElementById("layer1").getAttributeNS("http://www.inkscape.org/namespaces/inkscape", "label")"#), json!("Plant"), "Inkscape's attributes kept");
    assert_eq!(page.eval(r#"getComputedStyle(document.getElementById("layer2")).display"#), json!("none"), "the hidden layer left hidden");
    let d = page.eval(DRAWN);
    let rect = |v: &Value| -> Vec<f64> { v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() };
    for (id, state) in [("infeed", "sv-state-normal"), ("outlet-1", "sv-state-normal"), ("outlet-2", "sv-state-near"), ("outlet-3", "sv-state-warn")] {
        let o = &d[id];
        assert_eq!((o["state"].clone(), o["shown"].clone()), (json!(state), json!(true)), "{id}: {o}");
        let r = rect(&o["rect"]);
        let mid = r[0] + r[2] / 2.0;
        let label = rect(&o["label_rect"]);
        assert!(label[1] + label[3] <= r[1] + 1.0 && near(label[0] + label[2] / 2.0, mid, 1.5), "{id}: its label above it, centred: {label:?} over {r:?}");
        if state != "sv-state-normal" {
            let tag = rect(&o["tag_rect"]);
            assert!(tag[1] >= r[1] + r[3] - 1.0 && near(tag[0] + tag[2] / 2.0, mid, 1.5) && o["tag_shown"] == json!(true), "{id}: its tag under it, centred: {tag:?} under {r:?}");
        }
    }
    for (id, level) in [("outlet-1", 0.35), ("outlet-2", 0.85), ("outlet-3", 1.0)] {
        let (r, f) = (rect(&d[id]["rect"]), rect(&d[id]["fill"]["rect"]));
        assert!(d[id]["fill"]["shown"] == json!(true) && f[0] >= r[0] - 0.5 && f[0] + f[2] <= r[0] + r[2] + 0.5 && near(f[1] + f[3], r[1] + r[3], 0.75) && near(f[3], r[3] * level, 1.0), "{id}: its fill inside it at {level}: {f:?} in {r:?}");
    }
    // Each drawn where Inkscape writes a circle, at its own centre (`cx`, `cy`): moved by the place
    // less that centre.
    for (id, place, (cx, cy)) in [("parcel-1", 0.25, (150.0, 60.0)), ("parcel-2", 0.75, (250.0, 160.0))] {
        assert_eq!(d[id]["shown"], json!(true), "{id}");
        let (x, y) = translate(d[id]["transform"].as_str().unwrap());
        let (lx, ly) = on_loop(place);
        assert!(near(x + cx, lx, 0.05) && near(y + cy, ly, 0.05), "{id} at {place} along the loop: its centre at ({}, {}), the loop's point ({lx}, {ly})", x + cx, y + cy);
    }
    // With the Markers layer turned off (the page's own button), each way the view's scale or its
    // place in the page changes (a zoom, the window's size, another view and back), then the layer
    // on again: every marker at its place along the loop, as the screen shows it.
    let markers = r#"(() => { [...document.querySelectorAll(".layers button")].find((b) => b.textContent === "Markers").click(); return true; })()"#;
    for how in ["a zoom", "the window", "another view"] {
        page.eval(markers);
        match how {
            "a zoom" => {
                page.eval(r#"(() => { document.querySelector('.zoom button[title="Zoom in"]').click(); return true; })()"#);
            }
            "the window" => {
                page.call("Emulation.setDeviceMetricsOverride", json!({ "width": 1100, "height": 700, "deviceScaleFactor": 1, "mobile": false }));
            }
            _ => {
                page.eval("location.hash = 'dashboards'; true");
                page.wait_for("document.querySelector('.dash-grid') ? true : null", Duration::from_secs(10));
                page.eval("location.hash = 'layout'; true");
                page.wait_for(r#"document.getElementById("parcel-1")?.isConnected ? true : null"#, Duration::from_secs(10));
            }
        }
        page.eval(r#"new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => done(true))))"#);
        std::thread::sleep(Duration::from_millis(300));
        page.eval(markers);
        page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
        let d = page.eval(DRAWN);
        for (id, place) in [("parcel-1", 0.25), ("parcel-2", 0.75)] {
            assert!(d[id]["shown"] == json!(true) && at_place(&d, &d[id]["rect"], "loop", place), "{how} with the markers off: {id} at {:?} on screen, the loop's point {:?}", centre(&d[id]["rect"]), on_screen(&d["__ctm"]["loop"], on_loop(place)));
        }
        match how {
            "a zoom" => {
                page.eval(r#"(() => { document.querySelector('.zoom button[title="Fit"]').click(); return true; })()"#);
            }
            "the window" => {
                page.call("Emulation.clearDeviceMetricsOverride", json!({}));
            }
            _ => {}
        }
    }
    // The markers write `display:inline` (Inkscape's, on an object shown again): with their layer
    // turned off the page hides them all the same.
    let off = page.eval(r#"(() => { const svg = document.querySelector("svg.plant"); svg.classList.add("no-markers"); const d = ["parcel-1", "parcel-2"].map((id) => getComputedStyle(document.getElementById(id)).display); svg.classList.remove("no-markers"); return d; })()"#);
    assert_eq!(off, json!(["none", "none"]), "the markers hidden with their layer turned off, whatever display they write");
    // The bound text's every span, the run inside its line's span among them, painted as the text.
    let spans = page.eval(
        r#"(() => { const PAINT = ["fill", "stroke", "stroke-width", "stroke-dasharray", "fill-opacity", "stroke-opacity"]; const t = document.getElementById("speed"); const sig = (e) => { const c = getComputedStyle(e); return PAINT.map((p) => c.getPropertyValue(p)).join(" | "); }; const all = [...t.querySelectorAll("tspan")]; return [t.classList.contains("sv-obj"), all.length, all.filter((s) => sig(s) !== sig(t)).map((s) => `${s.id}: ${sig(s)}, the text ${sig(t)}`)]; })()"#,
    );
    assert_eq!(spans, json!([true, 2, []]), "the bound text's spans at every depth painted as the text");
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 220.0], 5, "a drawing saved as Inkscape saves it");
    drop(page);
    host.stop();
}

/// Inkscape's drawing on a portrait page (Inkscape's default is A4 portrait): an outlet at the page's
/// top-left edge and one at its bottom-left edge; the infeed and an outlet each a group of a shape
/// and its name, under it and beside it (as a tank grouped with its name is drawn), the name setting
/// an edge of the group's box; one marker a group of a circle and its number beside it, the other a
/// text: each box moving with the view's scale.
fn portrait_groups() -> String {
    let mut svg = INKSCAPE_LOOP.to_string();
    let swaps: [(&str, &str); 6] = [
        ("viewBox=\"0 0 400 220\"", "viewBox=\"0 0 400 600\""),
        (
            "    <rect\n       style=\"fill:#cccccc;stroke:#000000;stroke-width:0.264583\"\n       id=\"infeed\"\n       width=\"40\"\n       height=\"28\"\n       x=\"20\"\n       y=\"96\"\n       ry=\"2\"\n       inkscape:label=\"Infeed\" />\n",
            "    <g\n       id=\"infeed\"\n       inkscape:label=\"Infeed\">\n      <rect\n         style=\"fill:#cccccc;stroke:#000000;stroke-width:0.264583\"\n         id=\"rect-infeed\"\n         width=\"40\"\n         height=\"28\"\n         x=\"20\"\n         y=\"96\"\n         ry=\"2\" />\n      <text\n         xml:space=\"preserve\"\n         style=\"font-size:6px;font-family:sans-serif;fill:#26262a\"\n         x=\"20\"\n         y=\"131\"\n         id=\"text-infeed\"><tspan\n           sodipodi:role=\"line\"\n           id=\"tspan-infeed\"\n           x=\"20\"\n           y=\"131\">Infeed</tspan></text>\n    </g>\n",
        ),
        (
            "      <rect\n         style=\"fill:#cccccc;stroke:#000000;stroke-width:0.264583\"\n         id=\"outlet-1\"\n         width=\"24\"\n         height=\"28\"\n         x=\"130\"\n         y=\"4\" />\n",
            "      <g\n         id=\"outlet-1\"\n         inkscape:label=\"Outlet 1\">\n        <rect\n           style=\"fill:#cccccc;stroke:#000000;stroke-width:0.264583\"\n           id=\"rect-outlet-1\"\n           width=\"24\"\n           height=\"28\"\n           x=\"-10\"\n           y=\"4\" />\n        <text\n           xml:space=\"preserve\"\n           style=\"font-size:6px;font-family:sans-serif;fill:#26262a\"\n           x=\"16\"\n           y=\"20\"\n           id=\"text-outlet-1\"><tspan\n             sodipodi:role=\"line\"\n             id=\"tspan-outlet-1\"\n             x=\"16\"\n             y=\"20\">Outlet 1</tspan></text>\n      </g>\n",
        ),
        ("         id=\"outlet-3\"\n         width=\"24\"\n         height=\"28\"\n         x=\"250\"\n         y=\"4\" />", "         id=\"outlet-3\"\n         width=\"24\"\n         height=\"28\"\n         x=\"-10\"\n         y=\"576\" />"),
        (
            "    <circle\n       style=\"display:inline;fill:#ffffff;stroke:#000000;stroke-width:0.264583\"\n       id=\"parcel-1\"\n       cx=\"150\"\n       cy=\"60\"\n       r=\"5\" />\n",
            "    <g\n       id=\"parcel-1\"\n       style=\"display:inline\">\n      <circle\n         style=\"fill:#ffffff;stroke:#000000;stroke-width:0.264583\"\n         id=\"circle-parcel-1\"\n         cx=\"150\"\n         cy=\"60\"\n         r=\"5\" />\n      <text\n         xml:space=\"preserve\"\n         style=\"font-size:5px;font-family:sans-serif;fill:#26262a\"\n         x=\"157\"\n         y=\"62\"\n         id=\"text-parcel-1\">1</text>\n    </g>\n",
        ),
        (
            "    <circle\n       style=\"display:inline;fill:#ffffff;stroke:#000000;stroke-width:0.264583\"\n       id=\"parcel-2\"\n       cx=\"250\"\n       cy=\"160\"\n       r=\"5\" />\n",
            "    <text\n       xml:space=\"preserve\"\n       style=\"display:inline;font-size:8px;font-family:sans-serif;fill:#ffffff\"\n       x=\"246\"\n       y=\"163\"\n       id=\"parcel-2\">P2</text>\n",
        ),
    ];
    for (from, to) in swaps {
        assert_eq!(svg.matches(from).count(), 1, "{from}");
        svg = svg.replacen(from, to, 1);
    }
    svg
}

/// Whether the view moved from `a` to `b` as a pan moves it: its size kept (within a hundredth of a
/// unit) and its corner carried by half a unit or more (the least pan, 4 px, carries it by about
/// three here). A click moves nothing; a refit while fitted (the frame grown by what the page writes
/// at an edge, which a selection changes and which turns on the platform's text metrics) changes
/// its size.
fn panned_between(a: &Value, b: &Value) -> bool {
    let d = |i: usize| (b[i].as_f64().unwrap() - a[i].as_f64().unwrap()).abs();
    d(2) < 0.01 && d(3) < 0.01 && (d(0) >= 0.5 || d(1) >= 0.5)
}

/// The page's own controls (the layers, the legend, the zoom) stand clear of the drawing, and what it
/// writes at the drawing's edges is cut and covered by nothing, at every window size; everything it
/// places by a box (a group holding a text, a marker a group with a text or a text) placed anew at
/// every scale, the wheel's among them; the wheel's zoom the person's until Fit, as the buttons' is;
/// and a click told from a pan. On Inkscape's drawing on a portrait page (`portrait_groups`).
#[test]
fn the_page_keeps_its_controls_off_the_drawing_and_tells_a_click_from_a_pan() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let mut ex = live_example(&bench, false);
    // And a keyed marker drawn as Inkscape draws a parcel (a group of a circle and its letter beside
    // it), one clone per key, its keys kept across every step (a key is forgotten 10 s after its last
    // sample by default).
    ex.svg = portrait_groups().replacen("</svg>", "  <g\n     id=\"tracked\"\n     style=\"display:inline\">\n    <circle\n       style=\"fill:#ffffff;stroke:#000000;stroke-width:0.264583\"\n       id=\"circle-tracked\"\n       cx=\"0\"\n       cy=\"0\"\n       r=\"4\" />\n    <text\n       xml:space=\"preserve\"\n       style=\"font-size:5px;font-family:sans-serif;fill:#26262a\"\n       x=\"6\"\n       y=\"2\"\n       id=\"text-tracked\">T</text>\n  </g>\n</svg>", 1);
    ex.project.push_str("\n[[rule]]\nname = \"positions\"\npattern = 'POS item=(?P<item>\\w+) at=(?P<at>[0-9.]+)'\nscope = \"sorter\"\ntypes = { at = \"float\" }\nkey = \"item\"\n");
    ex.binding = ex.binding.replacen("\n[[dashboard]]", "\n[[object]]\nid = \"tracked\"\nlabel = \"Tracked\"\nvalue = { source = \"sorter\", series = \"at\" }\nshow = \"move\"\npath = \"loop\"\nstale_after_ms = 3600000\n\n[[dashboard]]", 1);
    let path = write_example(dir.path(), &ex);
    std::fs::write(dir.path().join("loop/sorts.log"), "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    let now = store_now();
    append(&dir.path().join("loop/sorts.log"), &[format!("{} POS item=A1 at=10", stamp(now)), format!("{} POS item=B2 at=50", stamp(now))]);
    let mut page = Browser::open(&format!("http://{addr}/s/loop/"));
    page.wait_for(r#"document.querySelectorAll('svg.plant [data-object="tracked"][data-key]').length === 2 || null"#, Duration::from_secs(20));
    page.wait_for(
        r#"(() => { const s = (id) => document.getElementById(id); return s("outlet-3")?.classList.contains("sv-state-warn") && s("parcel-1").getAttribute("transform") && s("parcel-2").getAttribute("transform") ? true : null; })()"#,
        Duration::from_secs(20),
    );
    let still = |page: &mut Browser| {
        page.eval(r#"new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => done(true))))"#);
        std::thread::sleep(Duration::from_millis(300));
        page.wait_for(r#"document.getAnimations().every((a) => a.playState !== "running") || null"#, Duration::from_secs(5));
    };
    still(&mut page);
    let vb = |page: &mut Browser| page.eval(r#"(() => { const v = document.querySelector("svg.plant").viewBox.baseVal; return [v.x, v.y, v.width, v.height]; })()"#);
    let button = |page: &mut Browser, title: &str| {
        page.eval(&format!(r#"(() => {{ document.querySelector('.zoom button[title="{title}"]').click(); return true; }})()"#));
    };
    let window = |page: &mut Browser, size: Option<(u32, u32)>| match size {
        Some((w, h)) => page.call("Emulation.setDeviceMetricsOverride", json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false })),
        None => page.call("Emulation.clearDeviceMetricsOverride", json!({})),
    };
    let wheel = |page: &mut Browser, dy: f64| {
        let c = page.eval(r#"(() => { const r = document.querySelector("svg.plant").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()"#);
        let (x, y) = (c[0].as_f64().unwrap(), c[1].as_f64().unwrap());
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }));
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": dy }));
    };

    // At every window size: the controls clear of the drawing; nothing the page writes beside it, at
    // its top-left and bottom-left edges among them, cut by its frame or covered.
    // Narrow windows among them, where the legend wraps (its height read at each size).
    let mut legend = Vec::new();
    for size in [None, Some((1100, 700)), Some((900, 640)), Some((700, 600)), Some((600, 480)), None] {
        window(&mut page, size);
        still(&mut page);
        assert_controls_clear(&mut page, &format!("at {size:?}"));
        assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 600.0], 6, &format!("at {size:?}"));
        legend.push(page.eval(r#"document.querySelector(".legend").getBoundingClientRect().height"#).as_f64().unwrap());
    }
    assert!(legend.iter().cloned().fold(0.0, f64::max) > 1.4 * legend[0], "a window where the legend wraps driven: its heights {legend:?}");
    // Placed by the element's box as it stands at every scale, whatever changed it (the fit, the
    // buttons, the wheel, Fit, the window): a group holding a text, a marker a group with a text or
    // a text, each box moving with the scale.
    for (step, act) in [("the fit", ""), ("a zoom in", "Zoom in"), ("a wheel in", "wheel in"), ("a wheel out", "wheel out"), ("a zoom out", "Zoom out"), ("Fit", "Fit"), ("a smaller window", "resize"), ("the window back", "restore")] {
        match act {
            "" => {}
            "wheel in" => wheel(&mut page, -100.0),
            "wheel out" => wheel(&mut page, 100.0),
            "resize" => {
                window(&mut page, Some((900, 640)));
            }
            "restore" => {
                window(&mut page, None);
            }
            title => button(&mut page, title),
        }
        still(&mut page);
        let off = page.eval(PLACED_NOW);
        assert!(off[0].as_u64().unwrap() >= 12 && off[1] == json!([]), "after {step}: placed otherwise than by the box as it stands: {off}");
        let moved = page.eval(MARKERS_NOW);
        assert!(moved[0].as_u64().unwrap() >= 8 && moved[1] == json!([]) && moved[2] == json!(2), "after {step}: a marker or a key's clone placed otherwise than by its box as it stands: {moved}");
        assert_controls_clear(&mut page, &format!("after {step}"));
    }
    // The wheel's zoom, the person's until Fit, as the buttons' is: it narrows the view, which holds
    // across a change of the feed and of the window.
    button(&mut page, "Fit");
    still(&mut page);
    let fit = vb(&mut page);
    wheel(&mut page, -100.0);
    still(&mut page);
    let wheeled = vb(&mut page);
    assert!(wheeled[2].as_f64().unwrap() < fit[2].as_f64().unwrap() - 1.0, "the wheel narrows the view: {wheeled} from {fit}");
    bench.server("plc").unwrap().set("fill1", 60.0);
    page.wait_for(r#"(() => { const f = document.querySelector('.sv-fill[data-for="outlet-1"]'); const r = document.getElementById("outlet-1").getBBox(); return f && Number(f.getAttribute("height")) > 0.5 * r.height ? true : null; })()"#, Duration::from_secs(20));
    still(&mut page);
    assert_eq!(vb(&mut page), wheeled, "the wheel's zoom kept across a change of the feed");
    window(&mut page, Some((1100, 700)));
    still(&mut page);
    assert_eq!(vb(&mut page), wheeled, "the wheel's zoom kept across a change of the window");
    window(&mut page, None);
    // A click told from a pan, through the browser's own pointer: a press released a pixel from where
    // it went down is a click (the view kept, the object picked); a drag from an element, and from the
    // drawing's background, pans the view and picks nothing and clears nothing.
    button(&mut page, "Fit");
    still(&mut page);
    let fit = vb(&mut page);
    let centre_of = |page: &mut Browser, id: &str| {
        let r = page.eval(&format!(r#"(() => {{ const r = document.getElementById("{id}").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()"#));
        (r[0].as_f64().unwrap(), r[1].as_f64().unwrap())
    };
    let gesture = |page: &mut Browser, (x, y): (f64, f64), (dx, dy): (f64, f64), steps: u32| {
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }));
        page.call("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }));
        for i in 1..=steps {
            let f = i as f64 / steps as f64;
            page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x + dx * f, "y": y + dy * f, "button": "left", "buttons": 1 }));
        }
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": x + dx, "y": y + dy, "button": "left", "buttons": 0, "clickCount": 1 }));
    };
    let outlet3 = centre_of(&mut page, "outlet-3");
    gesture(&mut page, outlet3, (1.0, 0.0), 1);
    still(&mut page);
    assert_eq!((vb(&mut page), page.eval(SELECTED)), (fit.clone(), json!(["outlet-3"])), "a click with a pixel's jitter: the view kept, outlet 3 picked");
    let infeed = centre_of(&mut page, "infeed");
    gesture(&mut page, infeed, (40.0, 30.0), 8);
    still(&mut page);
    let panned = vb(&mut page);
    assert!(panned != fit && page.eval(SELECTED) == json!(["outlet-3"]), "a drag from the infeed pans the view and picks nothing: {panned} from {fit}, selected {}", page.eval(SELECTED));
    button(&mut page, "Fit");
    still(&mut page);
    let ground = page.eval(r#"(() => { const svg = document.querySelector("svg.plant"), r = svg.getBoundingClientRect(); for (const [x, y] of [[r.right - 60, r.top + 60], [r.right - 60, r.bottom - 60], [r.left + r.width / 2, r.bottom - 60]]) if (document.elementFromPoint(x, y) === svg) return [x, y]; return null; })()"#);
    assert!(!ground.is_null(), "no background to drag from");
    let ground = (ground[0].as_f64().unwrap(), ground[1].as_f64().unwrap());
    gesture(&mut page, ground, (-40.0, -30.0), 8);
    still(&mut page);
    let panned = vb(&mut page);
    assert!(panned != fit && page.eval(SELECTED) == json!(["outlet-3"]), "a drag from the background pans the view and clears nothing: {panned} from {fit}, selected {}", page.eval(SELECTED));
    // The slop, 4 px from where the press went down (`SLOP`, the clause's): a press released under
    // it across, down, aslant and back is a click (the view not panned, `panned_between`, outlet 3
    // picked, the background clicked first to clear it); one released past it each way a pan (the
    // view panned, the infeed not picked).
    for (dx, dy) in [(3.0, 0.0), (0.0, 3.0), (2.0, 2.0), (3.0, 2.0), (-3.0, 0.0), (0.0, -3.0)] {
        button(&mut page, "Fit");
        still(&mut page);
        gesture(&mut page, ground, (0.0, 0.0), 0);
        still(&mut page);
        let before = vb(&mut page);
        let outlet3 = centre_of(&mut page, "outlet-3");
        gesture(&mut page, outlet3, (dx, dy), 1);
        still(&mut page);
        let now = vb(&mut page);
        assert!(!panned_between(&before, &now) && page.eval(SELECTED) == json!(["outlet-3"]), "a press released ({dx}, {dy}) from where it went down, under the slop: a click: {now} from {before}, selected {}", page.eval(SELECTED));
    }
    for (dx, dy) in [(4.0, 0.0), (0.0, 4.0), (5.0, 0.0), (0.0, 5.0), (3.0, 3.0), (-5.0, 0.0), (0.0, -5.0)] {
        button(&mut page, "Fit");
        still(&mut page);
        let fitted = vb(&mut page);
        let infeed = centre_of(&mut page, "infeed");
        gesture(&mut page, infeed, (dx, dy), 1);
        still(&mut page);
        let now = vb(&mut page);
        assert!(panned_between(&fitted, &now) && page.eval(SELECTED) == json!(["outlet-3"]), "a press released ({dx}, {dy}) away, past the slop: a pan, the infeed not picked: {now} from {fitted}, selected {}", page.eval(SELECTED));
    }
    // A click leaves the view fitted: outlet 3's word unsaid at Fit (the frame then without it), a click
    // with a pixel's jitter, the word said again at the drawing's bottom edge: the frame grows over it,
    // nothing cut.
    bench.server("plc").unwrap().set("fill3", 50.0);
    page.wait_for(r#"document.getElementById("outlet-3")?.classList.contains("sv-state-normal") || null"#, Duration::from_secs(20));
    button(&mut page, "Fit");
    still(&mut page);
    let outlet3 = centre_of(&mut page, "outlet-3");
    gesture(&mut page, outlet3, (2.0, 1.0), 1);
    still(&mut page);
    bench.server("plc").unwrap().set("fill3", 100.0);
    page.wait_for(r#"document.getElementById("outlet-3")?.classList.contains("sv-state-warn") || null"#, Duration::from_secs(20));
    still(&mut page);
    assert_inside_frame(&mut page, [0.0, 0.0, 400.0, 600.0], 6, "a word said at the bottom edge after a click with a pixel's jitter, the view still fitted");
    // A click picks what its press went down on: a release a pixel past an element's edge (the
    // infeed's box, pressed a pixel inside its right edge), and a marker slid from under a still
    // pointer between the press and the release (parcel 1 moved along the loop meanwhile).
    button(&mut page, "Fit");
    still(&mut page);
    gesture(&mut page, ground, (0.0, 0.0), 0);
    still(&mut page);
    let edge = page.eval(r#"(() => { const r = document.getElementById("rect-infeed").getBoundingClientRect(); return [r.right - 1, r.top + r.height / 2]; })()"#);
    let edge = (edge[0].as_f64().unwrap(), edge[1].as_f64().unwrap());
    gesture(&mut page, edge, (2.0, 0.0), 1);
    still(&mut page);
    assert_eq!(page.eval(SELECTED), json!(["infeed"]), "a release a pixel past the infeed's edge picks it");
    let parcel = centre_of(&mut page, "circle-parcel-1");
    let before = page.eval(r#"document.getElementById("parcel-1").getAttribute("transform")"#);
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": parcel.0, "y": parcel.1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": parcel.0, "y": parcel.1, "button": "left", "buttons": 1, "clickCount": 1 }));
    bench.server("plc").unwrap().set("p1", 60.0);
    page.wait_for(&format!(r#"document.getElementById("parcel-1").getAttribute("transform") !== {before} || null"#), Duration::from_secs(20));
    still(&mut page);
    let gone = page.eval(&format!("(() => {{ const e = document.elementFromPoint({}, {}); return !document.getElementById('parcel-1').contains(e); }})()", parcel.0, parcel.1));
    assert_eq!(gone, json!(true), "parcel 1 slid from under the pointer");
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": parcel.0, "y": parcel.1, "button": "left", "buttons": 0, "clickCount": 1 }));
    still(&mut page);
    assert_eq!(page.eval(SELECTED), json!(["parcel-1"]), "a press on parcel 1 released where it was, the parcel slid away: parcel 1 picked");
    // Only the primary button pans, and a pan ends with its button: a right drag pans nothing, and a
    // left press whose release never comes (a native menu took it), moved on with no button held,
    // pans nothing past what the held button panned.
    button(&mut page, "Fit");
    still(&mut page);
    let fitted = vb(&mut page);
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0, "y": ground.1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": ground.0, "y": ground.1, "button": "right", "buttons": 2, "clickCount": 1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0 - 30.0, "y": ground.1 - 20.0, "button": "right", "buttons": 2 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": ground.0 - 30.0, "y": ground.1 - 20.0, "button": "right", "buttons": 0, "clickCount": 1 }));
    still(&mut page);
    assert_eq!(vb(&mut page), fitted, "a right drag pans nothing");
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0, "y": ground.1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": ground.0, "y": ground.1, "button": "left", "buttons": 1, "clickCount": 1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0 - 20.0, "y": ground.1 - 15.0, "button": "left", "buttons": 1 }));
    still(&mut page);
    let held = vb(&mut page);
    assert!(panned_between(&fitted, &held), "the held button pans: {held} from {fitted}");
    for i in 1..=5 {
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0 - 20.0 - 8.0 * i as f64, "y": ground.1 - 15.0 - 5.0 * i as f64, "buttons": 0 }));
    }
    still(&mut page);
    assert_eq!(vb(&mut page), held, "moves with no button held, the release never come, pan nothing");
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": ground.0 - 60.0, "y": ground.1 - 40.0, "button": "left", "buttons": 0, "clickCount": 1 }));
    still(&mut page);
    // A drag from the drawing released off it (over the side panel): the pan ends with the press,
    // and the pointer moved over the drawing after it, no button held, moves nothing.
    button(&mut page, "Fit");
    still(&mut page);
    let fitted = vb(&mut page);
    let side = page.eval(r#"(() => { const r = document.querySelector(".side").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()"#);
    let side = (side[0].as_f64().unwrap(), side[1].as_f64().unwrap());
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0, "y": ground.1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": ground.0, "y": ground.1, "button": "left", "buttons": 1, "clickCount": 1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0 - 20.0, "y": ground.1 - 15.0, "button": "left", "buttons": 1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": side.0, "y": side.1, "button": "left", "buttons": 1 }));
    page.call("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": side.0, "y": side.1, "button": "left", "buttons": 0, "clickCount": 1 }));
    still(&mut page);
    let released = vb(&mut page);
    assert!(panned_between(&fitted, &released), "the drag panned before it left the drawing: {released} from {fitted}");
    for i in 1..=6 {
        page.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": ground.0 - 10.0 * i as f64, "y": ground.1 - 6.0 * i as f64 }));
    }
    still(&mut page);
    assert_eq!(vb(&mut page), released, "a drag released off the drawing: the pointer moved over it after, no button held, pans nothing");
    // And a still click on the background clears the selection, as before.
    button(&mut page, "Fit");
    still(&mut page);
    gesture(&mut page, ground, (0.0, 0.0), 0);
    still(&mut page);
    assert_eq!(page.eval(SELECTED), json!([]), "a click on the background clears the selection");
    drop(page);
    host.stop();
}
