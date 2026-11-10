// The layout view: the session's SVG placed in the page and bound by object ID. Bound once,
// then mutated, never rebuilt: a state is a class, a level a clipped fill, a marker a transform,
// a mark and a word a small text beside the element (below a marker, moving with it). What the
// page adds for an object carries its id in `data-for` (a marker's tag its key in `data-key`).
// Nothing here decides a state.

import type { ObjectDef, ObjectState } from "./api";
import { s } from "./dom";

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface Bound {
  def: ObjectDef;
  el: SVGGraphicsElement;
  /// The element's own box, in its own coordinates (a marker is placed by its centre).
  box: DOMRect;
  /// The same box in its parent's coordinates, where a level's fill is drawn.
  fillBox: Box;
  tag: SVGTextElement;
  label: SVGTextElement;
  fill?: SVGRectElement;
  filled?: boolean;
  valueText?: SVGTextElement;
  clones: Map<string, SVGGraphicsElement>;
  lastPlace: Map<string, number>;
  /// A marker's mark and word, one per key ("" for a marker of its own), in the overlay.
  tags: Map<string, SVGTextElement>;
}

export interface LayoutView {
  root: HTMLElement;
  update(o: ObjectState): void;
  select(id: string | null, key?: string | null): void;
  layers(l: { markers: boolean; states: boolean; labels: boolean }): void;
}

/// An element's box in another element's coordinates: its corners mapped through every transform
/// between them, as the box around them. Drawn there, a tag, a label or a fill follows a moved,
/// scaled or grouped element.
function boxIn(target: SVGGraphicsElement, el: SVGGraphicsElement, box: DOMRect): Box {
  const t = target.getCTM();
  const e = el.getCTM();
  if (!t || !e) return box;
  const m = t.inverse().multiply(e);
  const pts = [[box.x, box.y], [box.x + box.width, box.y], [box.x, box.y + box.height], [box.x + box.width, box.y + box.height]].map(([x, y]) => new DOMPoint(x, y).matrixTransform(m));
  const xs = pts.map((p) => p.x);
  const ys = pts.map((p) => p.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

/// What a layout may not carry into the page: scripts, foreign content, event handlers.
function clean(doc: Document) {
  doc.querySelectorAll("script, foreignObject").forEach((n) => n.remove());
  doc.querySelectorAll("*").forEach((el) => {
    for (const a of Array.from(el.attributes)) if (/^on/i.test(a.name)) el.removeAttribute(a.name);
  });
}

export async function mountLayout(base: string, defs: ObjectDef[], onSelect: (id: string, key: string | null) => void): Promise<LayoutView> {
  const text = await (await fetch(`${base}/layout.svg`, { cache: "no-store" })).text();
  const doc = new DOMParser().parseFromString(text, "image/svg+xml");
  clean(doc);
  const svg = document.importNode(doc.documentElement, true) as unknown as SVGSVGElement;
  svg.classList.add("plant");
  svg.removeAttribute("width");
  svg.removeAttribute("height");
  const root = document.createElement("div");
  root.className = "plant-wrap";
  root.append(svg);
  // Measuring needs the element in the document.
  document.body.append(root);

  const vb0 = svg.viewBox.baseVal && svg.viewBox.baseVal.width ? { x: svg.viewBox.baseVal.x, y: svg.viewBox.baseVal.y, w: svg.viewBox.baseVal.width, h: svg.viewBox.baseVal.height } : (() => { const b = svg.getBBox(); return { x: b.x, y: b.y, w: b.width, h: b.height }; })();
  const vb = { ...vb0 };
  const setVb = () => svg.setAttribute("viewBox", `${vb.x} ${vb.y} ${vb.w} ${vb.h}`);
  setVb();
  const fontSize = Math.max(vb0.h, vb0.w * 0.5) / 34;

  const defsEl = s("defs");
  defsEl.innerHTML = `<pattern id="sv-hatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="4" height="4" fill="#26262A"/><line x1="0" y1="0" x2="0" y2="4" stroke="#6F6F77" stroke-width="1.4"/></pattern>`;
  svg.prepend(defsEl);
  const overlay = s("g", { class: "sv-overlay" }) as SVGGElement;
  svg.append(overlay);

  const bound = new Map<string, Bound>();
  for (const def of defs) {
    const el = svg.querySelector(`[id="${CSS.escape(def.id)}"]`) as SVGGraphicsElement | null;
    if (!el) continue;
    el.classList.add("sv-obj", `sv-show-${def.show}`, "sv-state-stale");
    el.dataset.object = def.id;
    const box = el.getBBox();
    // Where the element stands in the overlay and in its parent, its transforms followed.
    const ob = boxIn(overlay, el, box);
    const fillBox = boxIn(el.parentNode as SVGGraphicsElement, el, box);
    const below = ob.y + ob.height + fontSize * 1.05;
    const tag = s("text", { x: ob.x + ob.width / 2, y: below, class: "sv-tag", "font-size": fontSize * 0.8, "text-anchor": "middle", "data-for": def.id }) as SVGTextElement;
    const label = s("text", { x: ob.x + ob.width / 2, y: ob.y - fontSize * 0.35, class: "sv-label", "font-size": fontSize * 0.85, "text-anchor": "middle", "data-for": def.id }, def.label) as SVGTextElement;
    const b: Bound = { def, el, box, fillBox, tag, label, clones: new Map(), lastPlace: new Map(), tags: new Map() };
    if (def.show === "level") {
      // The fill in the element's parent, over the element's box as transformed there, clipped
      // by the element itself when it is a shape or a text (the `use` draws it with its own
      // transform, in the same space); a clip's `use` may draw nothing else, so a group fills
      // its box.
      b.fill = s("rect", { x: fillBox.x, y: fillBox.y + fillBox.height, width: fillBox.width, height: 0, class: "sv-fill", "data-for": def.id }) as SVGRectElement;
      if (el instanceof SVGGeometryElement || el instanceof SVGTextElement) {
        const clipId = `sv-clip-${def.id}`;
        defsEl.append(s("clipPath", { id: clipId }, s("use", { href: `#${def.id}` })));
        b.fill.setAttribute("clip-path", `url(#${clipId})`);
      }
      el.after(b.fill);
    }
    if (def.show === "value") {
      b.valueText = s("text", { x: ob.x + ob.width / 2, y: ob.y + ob.height / 2 + fontSize * 0.35, class: "sv-value", "font-size": fontSize, "text-anchor": "middle", "data-for": def.id }) as SVGTextElement;
      overlay.append(b.valueText);
    }
    if (def.show === "move") {
      el.classList.add("sv-hidden");
      tag.remove();
    } else {
      overlay.append(tag);
      overlay.append(label);
    }
    const pick = (ev: Event) => {
      ev.stopPropagation();
      onSelect(def.id, (ev.currentTarget as SVGElement).dataset.key ?? null);
    };
    el.addEventListener("click", pick);
    bound.set(def.id, b);
  }

  const pathOf = (id: string) => svg.querySelector(`[id="${CSS.escape(id)}"]`) as SVGGeometryElement | null;

  /// A marker's tag for one key, made on first use.
  function markerTag(b: Bound, key: string): SVGTextElement {
    let t = b.tags.get(key);
    if (!t) {
      t = s("text", { class: "sv-tag", "font-size": fontSize * 0.8, "text-anchor": "middle", "data-for": b.def.id, "data-key": key }) as SVGTextElement;
      overlay.append(t);
      b.tags.set(key, t);
    }
    return t;
  }

  function dropTag(b: Bound, key: string) {
    b.tags.get(key)?.remove();
    b.tags.delete(key);
  }

  function placeAt(b: Bound, el: SVGGraphicsElement, key: string, place: number) {
    const p = b.def.path ? pathOf(b.def.path) : null;
    if (!p || typeof p.getTotalLength !== "function") return;
    const pt = p.getPointAtLength(place * p.getTotalLength());
    const parent = el.parentNode as SVGGraphicsElement;
    const pc = parent.getCTM();
    const ec = p.getCTM();
    // Hidden (another view in front), nothing can be measured: the next update places it.
    if (!pc || !ec) return;
    const m = pc.inverse().multiply(ec);
    const q = new DOMPoint(pt.x, pt.y).matrixTransform(m);
    const cx = b.box.x + b.box.width / 2;
    const cy = b.box.y + b.box.height / 2;
    const prev = b.lastPlace.get(key);
    // A wrap around the path jumps, never slides back across the drawing.
    // The first place and a wrap around the path are set at once; a step slides.
    el.classList.toggle("sv-jump", prev === undefined || Math.abs(prev - place) > 0.5);
    b.lastPlace.set(key, place);
    el.setAttribute("transform", `translate(${(q.x - cx).toFixed(2)} ${(q.y - cy).toFixed(2)})`);
    // Its mark and word below it, in the overlay's coordinates.
    const oc = overlay.getCTM();
    if (oc) {
      const r = new DOMPoint(pt.x, pt.y).matrixTransform(oc.inverse().multiply(ec));
      const t = markerTag(b, key);
      t.setAttribute("x", r.x.toFixed(2));
      t.setAttribute("y", (r.y + b.box.height / 2 + fontSize * 0.9).toFixed(2));
    }
  }

  let selected: { id: string | null; key: string | null } = { id: null, key: null };

  function paint(b: Bound, o: ObjectState) {
    const cls = `sv-state-${o.state}`;
    const targets = [b.el, ...b.clones.values()];
    for (const t of targets) {
      for (const c of Array.from(t.classList)) if (c.startsWith("sv-state-")) t.classList.remove(c);
      t.classList.add(cls);
    }
    if (b.fill) {
      // The first fill is set at once, the next ones slide.
      if (!b.filled) {
        b.filled = true;
        b.fill.classList.add("sv-instant");
        const fill = b.fill;
        requestAnimationFrame(() => requestAnimationFrame(() => fill.classList.remove("sv-instant")));
      }
      const f = o.level ?? 0;
      b.fill.setAttribute("y", String(b.fillBox.y + b.fillBox.height * (1 - f)));
      b.fill.setAttribute("height", String(b.fillBox.height * f));
      for (const c of Array.from(b.fill.classList)) if (c.startsWith("sv-state-")) b.fill.classList.remove(c);
      b.fill.classList.add(cls);
    }
    if (b.valueText) b.valueText.textContent = o.text === null ? "" : `${o.text}${b.def.unit ? " " + b.def.unit : ""}`;
    const said = o.state === "normal" ? "" : `${o.mark} ${o.word}`;
    b.tag.textContent = said;
    b.tag.setAttribute("class", `sv-tag ${cls}`);
    if (b.def.show === "move") {
      if (o.items.length > 0) {
        b.el.classList.add("sv-hidden");
        const keys = new Set(o.items.map((i) => i.key));
        for (const [k, c] of b.clones) if (!keys.has(k)) { c.remove(); b.clones.delete(k); b.lastPlace.delete(k); dropTag(b, k); }
        dropTag(b, "");
        for (const it of o.items) {
          let c = b.clones.get(it.key);
          if (!c) {
            c = b.el.cloneNode(true) as SVGGraphicsElement;
            c.removeAttribute("id");
            c.dataset.key = it.key;
            c.classList.remove("sv-hidden");
            c.addEventListener("click", (ev) => { ev.stopPropagation(); onSelect(b.def.id, it.key); });
            b.el.after(c);
            b.clones.set(it.key, c);
          }
          c.classList.toggle("sv-selected", selected.id === b.def.id && selected.key === it.key);
          for (const x of Array.from(c.classList)) if (x.startsWith("sv-state-")) c.classList.remove(x);
          c.classList.add(cls);
          placeAt(b, c, it.key, it.place);
        }
      } else {
        for (const k of b.clones.keys()) dropTag(b, k);
        for (const c of b.clones.values()) c.remove();
        b.clones.clear();
        b.el.classList.toggle("sv-hidden", o.place === null);
        if (o.place !== null) placeAt(b, b.el, "", o.place);
        else dropTag(b, "");
      }
      for (const t of b.tags.values()) {
        t.textContent = said;
        t.setAttribute("class", `sv-tag ${cls}`);
      }
    }
  }

  // Zoom and pan by the viewBox.
  const zoom = (f: number) => {
    const cx = vb.x + vb.w / 2, cy = vb.y + vb.h / 2;
    vb.w *= f; vb.h *= f; vb.x = cx - vb.w / 2; vb.y = cy - vb.h / 2;
    setVb();
  };
  let drag: { x: number; y: number } | null = null;
  svg.addEventListener("mousedown", (e) => { drag = { x: e.clientX, y: e.clientY }; });
  window.addEventListener("mouseup", () => { drag = null; });
  svg.addEventListener("mousemove", (e) => {
    if (!drag) return;
    const r = svg.getBoundingClientRect();
    vb.x -= ((e.clientX - drag.x) / r.width) * vb.w;
    vb.y -= ((e.clientY - drag.y) / r.height) * vb.h;
    drag = { x: e.clientX, y: e.clientY };
    setVb();
  });
  svg.addEventListener("wheel", (e) => { e.preventDefault(); zoom(e.deltaY > 0 ? 1.12 : 1 / 1.12); }, { passive: false });
  svg.addEventListener("click", () => onSelect("", null));

  const controls = document.createElement("div");
  controls.className = "zoom";
  for (const [label, f] of [["+", 1 / 1.25], ["−", 1.25], ["⤢", 0]] as [string, number][]) {
    const b = document.createElement("button");
    b.textContent = label;
    b.title = f === 0 ? "Fit" : label === "+" ? "Zoom in" : "Zoom out";
    b.onclick = () => { if (f === 0) { Object.assign(vb, vb0); setVb(); } else zoom(f); };
    controls.append(b);
  }
  root.append(controls);
  root.remove();

  return {
    root,
    update(o) {
      const b = bound.get(o.id);
      if (b) paint(b, o);
    },
    select(id, key = null) {
      selected = { id, key };
      for (const b of bound.values()) {
        b.el.classList.toggle("sv-selected", b.def.id === id && (key === null || b.clones.size === 0));
        for (const [k, c] of b.clones) c.classList.toggle("sv-selected", b.def.id === id && k === key);
      }
    },
    layers(l) {
      svg.classList.toggle("no-markers", !l.markers);
      svg.classList.toggle("no-states", !l.states);
      svg.classList.toggle("no-labels", !l.labels);
    },
  };
}

