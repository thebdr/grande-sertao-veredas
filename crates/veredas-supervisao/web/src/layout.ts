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
  /// A marker's own transform as the drawing writes it, kept under the place the page gives it.
  own: string;
  ownM: DOMMatrix;
  /// The box a level fills, in the overlay's frame (the screen's, up to the view's zoom): the
  /// element's box there, widened to its stroke where it has no area (a line), its stroke's width
  /// read again whenever it is painted.
  fillBox: Box;
  /// The element's box in the overlay's frame, as it stands.
  ob: Box;
  /// For an element with no area: each shape that draws it (the element, or a group's shapes), its
  /// box in the overlay's frame, the matrix from it to the overlay and the way it runs, in its own
  /// coordinates.
  noArea?: { el: SVGGraphicsElement; ob: Box; m: DOMMatrix; dir: { x: number; y: number } | null }[];
  tag: SVGTextElement;
  label: SVGTextElement;
  fill?: SVGRectElement;
  filled?: boolean;
  valueText?: SVGTextElement;
  clones: Map<string, SVGGraphicsElement>;
  lastPlace: Map<string, number>;
  /// A marker's mark and word, one per key ("" for a marker of its own), in the overlay.
  tags: Map<string, SVGTextElement>;
  /// The object as last painted, painted again when the layout is shown.
  last?: ObjectState;
}

export interface LayoutView {
  root: HTMLElement;
  update(o: ObjectState): void;
  select(id: string | null, key?: string | null): void;
  layers(l: { markers: boolean; states: boolean; labels: boolean }): void;
  /// The layout put on screen again: every object painted anew, its markers placed now.
  shown(): void;
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

/// The paint the page outranks on a bound element (and pins on a group's text, as drawn).
const PAINT = ["fill", "stroke", "stroke-width", "stroke-dasharray", "fill-opacity", "stroke-opacity"];

/// Whether an element draws no area, and the way it runs in its own coordinates: a line; any element
/// with no width or no height (a shape, or a group of such shapes); or a shape every point of which
/// lies on one straight line (a straight path, polyline or polygon, slanted or not: a polyline's and
/// a polygon's own points, and 65 points along any shape), its way from its start to its farthest
/// point. Null for an element with an area.
function noAreaOf(el: SVGGraphicsElement, box: DOMRect): { dir: { x: number; y: number } | null } | null {
  if (el instanceof SVGLineElement) return { dir: { x: el.x2.baseVal.value - el.x1.baseVal.value, y: el.y2.baseVal.value - el.y1.baseVal.value } };
  if (box.width === 0) return { dir: { x: 0, y: 1 } };
  if (box.height === 0) return { dir: { x: 1, y: 0 } };
  if (!(el instanceof SVGGeometryElement)) return null;
  const pts: { x: number; y: number }[] = [];
  if (el instanceof SVGPolylineElement || el instanceof SVGPolygonElement) for (let i = 0; i < el.points.numberOfItems; i++) pts.push(el.points.getItem(i));
  const len = el.getTotalLength();
  for (let k = 0; k <= 64; k++) pts.push(el.getPointAtLength((len * k) / 64));
  const a = pts[0];
  const far = pts.reduce((f, p) => (Math.hypot(p.x - a.x, p.y - a.y) > Math.hypot(f.x - a.x, f.y - a.y) ? p : f), a);
  const dx = far.x - a.x, dy = far.y - a.y, n = Math.hypot(dx, dy);
  if (!n) return { dir: null };
  const off = Math.max(...pts.map((p) => Math.abs((p.x - a.x) * dy - (p.y - a.y) * dx) / n));
  return off <= n * 1e-6 + 1e-6 ? { dir: { x: dx, y: dy } } : null;
}

/// A stroke's width across the way it runs, carried through a matrix: its band of width `sw` is
/// stretched by the matrix's area and along its length, so across it by the one over the other
/// (`sw` times the scale where the matrix scales alike every way).
function across(m: DOMMatrix, sw: number, dir: { x: number; y: number } | null): number {
  const det = Math.abs(m.a * m.d - m.b * m.c);
  const n = dir ? Math.hypot(dir.x, dir.y) : 0;
  if (!dir || !n) return sw * Math.sqrt(det);
  const ux = dir.x / n, uy = dir.y / n;
  const along = Math.hypot(m.a * ux + m.c * uy, m.b * ux + m.d * uy);
  return along ? (sw * det) / along : sw * Math.sqrt(det);
}

/// The box around boxes; none for none.
function union(boxes: Box[]): Box | null {
  if (!boxes.length) return null;
  const x = Math.min(...boxes.map((b) => b.x)), y = Math.min(...boxes.map((b) => b.y));
  return { x, y, width: Math.max(...boxes.map((b) => b.x + b.width)) - x, height: Math.max(...boxes.map((b) => b.y + b.height)) - y };
}

/// The box a level with no area fills: each shape's box widened to its stroke's width across it, the
/// box around them; the element's own box where it has no shape to measure.
function overStroke(parts: NonNullable<Bound["noArea"]>, ob: Box): Box {
  return union(parts.map((p) => widened(p.ob, across(p.m, parseFloat(getComputedStyle(p.el).strokeWidth) || 0, p.dir)))) ?? ob;
}

/// A box widened, about its centre, to `w` where it is narrower or lower.
function widened(b: Box, w: number): Box {
  const out = { ...b };
  if (out.width < w) { out.x -= (w - out.width) / 2; out.width = w; }
  if (out.height < w) { out.y -= (w - out.height) / 2; out.height = w; }
  return out;
}

/// What a layout may not carry into the page: scripts, foreign content, event handlers; and every
/// element outside the SVG namespace dropped on read (Inkscape's own, the reader's only exception).
function clean(doc: Document) {
  doc.querySelectorAll("script, foreignObject").forEach((n) => n.remove());
  doc.querySelectorAll("*").forEach((el) => { if (el.namespaceURI !== "http://www.w3.org/2000/svg") el.remove(); });
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
  // A change of the view's scale (a fit, a zoom) has everything measured anew (`rescale`, below).
  let scaledAt = { w: vb.w, h: vb.h };
  let onScale = () => {};
  const setVb = () => {
    svg.setAttribute("viewBox", `${vb.x} ${vb.y} ${vb.w} ${vb.h}`);
    if (vb.w !== scaledAt.w || vb.h !== scaledAt.h) {
      scaledAt = { w: vb.w, h: vb.h };
      onScale();
    }
  };
  setVb();
  const fontSize = Math.max(vb0.h, vb0.w * 0.5) / 34;

  const defsEl = s("defs");
  // What the page adds stands in a group of its own that inherits nothing from the drawing
  // (`sv-reset`, the overlay alike: every property at its initial value), so paint the drawing
  // writes on its root or on a group around an element reaches none of it.
  defsEl.innerHTML = `<pattern id="sv-hatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><g class="sv-reset"><rect width="4" height="4" fill="#26262A"/><line x1="0" y1="0" x2="0" y2="4" stroke="#6F6F77" stroke-width="1.4"/></g></pattern>`;
  svg.prepend(defsEl);
  const overlay = s("g", { class: "sv-overlay" }) as SVGGElement;
  svg.append(overlay);

  /// Where the page draws what it adds for an object, from the element's box as it stands now: the
  /// box (a text's moves with the view's scale: Chromium lays a text out at the size it is drawn,
  /// so a fit or a zoom moves its box a little), carried to the overlay; its label over it, its
  /// mark and word under it and its value over it; a level's fill box (each shape of one with no
  /// area measured too, its stroke read unselected). Done when the object is bound and whenever the
  /// view's scale changes, the object then painted again.
  function measure(b: Bound) {
    const el = b.el;
    const hidden = el.classList.contains("sv-hidden");
    if (hidden) el.classList.remove("sv-hidden");
    b.box = el.getBBox();
    if (hidden) el.classList.add("sv-hidden");
    if (b.def.show === "move") return;
    b.ob = boxIn(overlay, el, b.box);
    const { x, y, width, height } = b.ob;
    const mid = String(x + width / 2);
    b.tag.setAttribute("x", mid);
    b.tag.setAttribute("y", String(y + height + fontSize * 1.05));
    b.label.setAttribute("x", mid);
    b.label.setAttribute("y", String(y - fontSize * 0.35));
    if (b.valueText) {
      b.valueText.setAttribute("x", mid);
      b.valueText.setAttribute("y", String(y + height / 2 + fontSize * 0.35));
    }
    if (b.noArea) {
      for (const p of b.noArea) p.ob = boxIn(overlay, p.el, p.el.getBBox());
      const sel = el.classList.contains("sv-selected");
      el.classList.remove("sv-selected");
      b.fillBox = overStroke(b.noArea, b.ob);
      if (sel) el.classList.add("sv-selected");
    } else b.fillBox = { ...b.ob };
    if (b.fill) {
      const f = b.filled ? b.last?.level ?? 0 : 0;
      b.fill.setAttribute("x", String(b.fillBox.x));
      b.fill.setAttribute("width", String(b.fillBox.width));
      b.fill.setAttribute("y", String(b.fillBox.y + b.fillBox.height * (1 - f)));
      b.fill.setAttribute("height", String(b.fillBox.height * f));
    }
  }

  const bound = new Map<string, Bound>();
  for (const def of defs) {
    const el = svg.querySelector(`[id="${CSS.escape(def.id)}"]`) as SVGGraphicsElement | null;
    if (!el) continue;
    // A group's text keeps its paint as drawn: what it writes and what it inherited from the group
    // before the group took a state, pinned on it.
    const pinned = el instanceof SVGTextElement ? [] : Array.from(el.querySelectorAll("text")).map((t) => {
      const c = getComputedStyle(t);
      return { t, vals: PAINT.map((p) => [p, c.getPropertyValue(p)] as const) };
    });
    el.classList.add("sv-obj", `sv-show-${def.show}`, "sv-state-stale");
    for (const { t, vals } of pinned) for (const [p, v] of vals) t.style.setProperty(p, v, "important");
    el.dataset.object = def.id;
    const box = el.getBBox();
    // Placed by `measure`, from the element's box as it stands.
    const tag = s("text", { class: "sv-tag", "font-size": fontSize * 0.8, "text-anchor": "middle", "data-for": def.id }) as SVGTextElement;
    const label = s("text", { class: "sv-label", "font-size": fontSize * 0.85, "text-anchor": "middle", "data-for": def.id }, def.label) as SVGTextElement;
    const own = el.getAttribute("transform") ?? "";
    // Its own transform as one matrix (read, never consolidated in place).
    let ownM = new DOMMatrix();
    const list = el.transform?.baseVal;
    if (list) for (let i = 0; i < list.numberOfItems; i++) { const t = list.getItem(i).matrix; ownM = ownM.multiply(new DOMMatrix([t.a, t.b, t.c, t.d, t.e, t.f])); }
    // An element with no area (a line, a straight path or polyline, a group of them) is filled over
    // its stroke: each shape's box widened to its stroke's width across it, carried to the overlay
    // (each of a group's shapes through its own matrix), the box around them; its stroke then drawn
    // as the level's track (`sv-noarea`, a group's shapes taking it from the group), the fill over
    // it in the state's colour.
    const flat = def.show === "level" ? noAreaOf(el, box) : null;
    const noArea = flat !== null;
    const oc0 = overlay.getCTM();
    let na: Bound["noArea"];
    if (flat && oc0) {
      const shapes: SVGGraphicsElement[] = el instanceof SVGGeometryElement ? [el] : Array.from(el.querySelectorAll<SVGGeometryElement>("path, rect, circle, ellipse, line, polyline, polygon")).filter((g) => g.checkVisibility());
      na = shapes.flatMap((g) => {
        const gc = g.getCTM();
        if (!gc) return [];
        const gb = g.getBBox();
        return [{ el: g, ob: boxIn(overlay, g, gb), m: oc0.inverse().multiply(gc), dir: g === el ? flat.dir : noAreaOf(g, gb)?.dir ?? null }];
      });
      el.classList.add("sv-noarea");
    }
    const none = { x: 0, y: 0, width: 0, height: 0 };
    const b: Bound = { def, el, box, own, ownM, fillBox: none, ob: none, noArea: na, tag, label, clones: new Map(), lastPlace: new Map(), tags: new Map() };
    if (def.show === "level") {
      // The fill rises up the screen over the element's box, whatever turns, skews or scales the
      // element and its groups: a rect in the overlay's frame, carried into the element's parent
      // by a frame of the matrix between them. Its holder, in the parent's own space, is clipped
      // by the element itself when it is a shape or a text with an area (the clip's `use` draws
      // it with its own transform, in that same space); a clip's `use` may draw nothing else, so
      // a group, and an element with no area, fills its box.
      b.fill = s("rect", { height: 0, class: "sv-fill", "data-for": def.id }) as SVGRectElement;
      const pc = (el.parentNode as SVGGraphicsElement).getCTM();
      const m = pc && oc0 ? pc.inverse().multiply(oc0) : new DOMMatrix();
      const frame = s("g", { transform: `matrix(${m.a} ${m.b} ${m.c} ${m.d} ${m.e} ${m.f})` }, b.fill);
      const holder = s("g", { class: "sv-fill-holder", "data-for": def.id }, s("g", { class: "sv-reset" }, frame));
      if ((el instanceof SVGGeometryElement || el instanceof SVGTextElement) && !noArea) {
        const clipId = `sv-clip-${def.id}`;
        // A clip reads its shape's inside by `clip-rule`, never by the `fill-rule` it is painted by
        // (the hole Inkscape's Combine leaves, `fill-rule:evenodd`): the element's own rule is
        // written as its clip rule, on it, where it draws nothing else.
        el.style.setProperty("clip-rule", getComputedStyle(el).fillRule, "important");
        defsEl.append(s("clipPath", { id: clipId }, s("use", { href: `#${def.id}` })));
        holder.setAttribute("clip-path", `url(#${clipId})`);
      }
      el.after(holder);
    }
    if (def.show === "value") {
      b.valueText = s("text", { class: "sv-value", "font-size": fontSize, "text-anchor": "middle", "data-for": def.id }) as SVGTextElement;
      overlay.append(b.valueText);
    }
    measure(b);
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

  // The frame the view fits: the drawing's own, grown to take in everything the page writes beside
  // its elements (a label over one at the drawing's top edge, a mark and word under one at its
  // bottom, a value), so the drawing's frame cuts nothing the page adds. From the last fit on it
  // grows and never shrinks while the view stays fitted (a word coming and going at an edge moves
  // nothing back and forth); a zoom or a pan leaves the view to the person until Fit.
  const frame = { ...vb0 };
  let fitted = true;
  const refit = () => {
    if (!fitted || !svg.isConnected) return;
    const said = Array.from(overlay.children).filter((t): t is SVGGraphicsElement => t instanceof SVGGraphicsElement && !!t.textContent && getComputedStyle(t).display !== "none").map((t) => t.getBBox()).filter((b) => b.width || b.height);
    const all = union([{ x: frame.x, y: frame.y, width: frame.w, height: frame.h }, ...said]);
    // A text's box moves a little with the scale it is measured at: the frame, when it grows, grows
    // a hundredth of itself past what it must take in, so the fit settles, never chasing its words.
    if (all && (all.x < frame.x || all.y < frame.y || all.x + all.width > frame.x + frame.w || all.y + all.height > frame.y + frame.h)) {
      const slack = Math.max(all.width, all.height) / 100;
      Object.assign(frame, { x: all.x - slack, y: all.y - slack, w: all.width + 2 * slack, h: all.height + 2 * slack });
    }
    if (vb.x !== frame.x || vb.y !== frame.y || vb.w !== frame.w || vb.h !== frame.h) {
      Object.assign(vb, frame);
      setVb();
    }
  };
  // The view's scale changed: everything the page placed by a box measured anew and painted.
  const rescale = () => {
    if (!svg.isConnected) return;
    // A marker its layer hides is measured as it is drawn: the browser gives an element it does not
    // display an empty box, so the layer's hiding is lifted while the boxes are read (as `measure`
    // lifts the page's own hiding of one).
    const off = svg.classList.contains("no-markers");
    if (off) svg.classList.remove("no-markers");
    for (const b of bound.values()) measure(b);
    if (off) svg.classList.add("no-markers");
    for (const b of bound.values()) if (b.last) paint(b, b.last);
  };
  onScale = rescale;
  new ResizeObserver(rescale).observe(svg);
  let refitting = false;
  const refitSoon = () => {
    if (refitting) return;
    refitting = true;
    requestAnimationFrame(() => { refitting = false; refit(); });
  };
  refit();

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
    // Off screen (another view in front), the drawing's transforms are not followed: a place
    // measured now would be wrong, so none is; the layout places every marker when it is shown.
    if (!svg.isConnected) return;
    const p = b.def.path ? pathOf(b.def.path) : null;
    if (!p || typeof p.getTotalLength !== "function") return;
    const pt = p.getPointAtLength(place * p.getTotalLength());
    const parent = el.parentNode as SVGGraphicsElement;
    const pc = parent.getCTM();
    const ec = p.getCTM();
    // No matrix yet (not laid out), nothing can be measured: the layout places it when shown.
    if (!pc || !ec) return;
    const m = pc.inverse().multiply(ec);
    const q = new DOMPoint(pt.x, pt.y).matrixTransform(m);
    // Its centre where its own transform (a scale, a turn) puts it, set on the path's point, its
    // own transform kept under the place.
    const c = new DOMPoint(b.box.x + b.box.width / 2, b.box.y + b.box.height / 2).matrixTransform(b.ownM);
    const dx = q.x - c.x, dy = q.y - c.y;
    const prev = b.lastPlace.get(key);
    // A wrap around the path jumps, never slides back across the drawing.
    // The first place and a wrap around the path are set at once; a step slides.
    el.classList.toggle("sv-jump", prev === undefined || Math.abs(prev - place) > 0.5);
    b.lastPlace.set(key, place);
    el.setAttribute("transform", `translate(${dx.toFixed(2)} ${dy.toFixed(2)})${b.own ? ` ${b.own}` : ""}`);
    // Its mark and word below it, in the overlay's coordinates: its box carried there through the
    // place just set (not through a slide still running), its own transform and its groups'.
    const oc = overlay.getCTM();
    if (oc) {
      // As DOM matrices: an SVG matrix multiplies none but its own kind.
      const dom = (k: DOMMatrixReadOnly | SVGMatrix) => new DOMMatrix([k.a, k.b, k.c, k.d, k.e, k.f]);
      const to = dom(oc).inverse().multiply(dom(pc)).multiply(new DOMMatrix().translate(dx, dy)).multiply(b.ownM);
      const { x, y, width, height } = b.box;
      const pts = [[x, y], [x + width, y], [x, y + height], [x + width, y + height]].map(([px, py]) => new DOMPoint(px, py).matrixTransform(to));
      const t = markerTag(b, key);
      t.setAttribute("x", ((Math.min(...pts.map((p) => p.x)) + Math.max(...pts.map((p) => p.x))) / 2).toFixed(2));
      t.setAttribute("y", (Math.max(...pts.map((p) => p.y)) + fontSize * 0.9).toFixed(2));
    }
  }

  let selected: { id: string | null; key: string | null } = { id: null, key: null };

  /// The selection drawn on what the object draws: its element while that is drawn (never a marker
  /// with no place, nor a keyed marker's own element while its clones stand), every clone when the
  /// object is picked whole (from its row), a key's own clone when a key is.
  function mark(b: Bound) {
    const mine = selected.id === b.def.id;
    b.el.classList.toggle("sv-selected", mine && b.clones.size === 0 && !b.el.classList.contains("sv-hidden"));
    for (const [k, c] of b.clones) c.classList.toggle("sv-selected", mine && (selected.key === null || selected.key === k));
  }

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
      // With no area, the stroke's width read again in the state just set (unselected: the
      // selection's own stroke is no part of the track).
      if (b.noArea) {
        const sel = b.el.classList.contains("sv-selected");
        b.el.classList.remove("sv-selected");
        b.fillBox = overStroke(b.noArea, b.ob);
        if (sel) b.el.classList.add("sv-selected");
        b.fill.setAttribute("x", String(b.fillBox.x));
        b.fill.setAttribute("width", String(b.fillBox.width));
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
    mark(b);
    refitSoon();
  }

  // Zoom and pan by the viewBox.
  const zoom = (f: number) => {
    fitted = false;
    const cx = vb.x + vb.w / 2, cy = vb.y + vb.h / 2;
    vb.w *= f; vb.h *= f; vb.x = cx - vb.w / 2; vb.y = cy - vb.h / 2;
    setVb();
  };
  let drag: { x: number; y: number } | null = null;
  svg.addEventListener("mousedown", (e) => { drag = { x: e.clientX, y: e.clientY }; });
  window.addEventListener("mouseup", () => { drag = null; });
  svg.addEventListener("mousemove", (e) => {
    if (!drag || (e.clientX === drag.x && e.clientY === drag.y)) return;
    fitted = false;
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
    b.onclick = () => {
      if (f === 0) {
        fitted = true;
        Object.assign(frame, vb0);
        refit();
      } else zoom(f);
    };
    controls.append(b);
  }
  root.append(controls);
  root.remove();

  return {
    root,
    update(o) {
      const b = bound.get(o.id);
      if (b) {
        b.last = o;
        paint(b, o);
      }
    },
    shown() {
      for (const b of bound.values()) if (b.last) paint(b, b.last);
    },
    select(id, key = null) {
      selected = { id, key };
      for (const b of bound.values()) mark(b);
    },
    layers(l) {
      svg.classList.toggle("no-markers", !l.markers);
      svg.classList.toggle("no-states", !l.states);
      svg.classList.toggle("no-labels", !l.labels);
      refitSoon();
    },
  };
}

