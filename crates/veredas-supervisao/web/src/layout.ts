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
  /// For an element with no area: each straight run that draws it (a line, a straight subpath of a
  /// path, a straight polyline or polygon, of the element or of a group's shapes), its box in its
  /// shape's own coordinates and in the overlay's frame, the matrix from its shape to the overlay and
  /// the way it runs, in its own coordinates.
  noArea?: { el: SVGGraphicsElement; box: Box; ob: Box; m: DOMMatrix; dir: { x: number; y: number } | null }[];
  /// The level's fill held to the bands of its runs' strokes (an element with no area).
  runsClip?: SVGClipPathElement;
  /// A level along lines (an element with an area drawn as a line, Q30): the copy of the element's
  /// shapes the level is drawn on, clipped to the level's rect (`fill`, then held in a clip), carrying
  /// the state.
  along?: SVGGElement;
  /// Its copy's lines' widths when its box was last read.
  alongWidths?: string;
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
function boxIn(target: SVGGraphicsElement, el: SVGGraphicsElement, box: Box): Box {
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

/// The shapes the page draws on (the reader's `DRAWN`, its group and text apart).
const GEOMETRY = "path, rect, circle, ellipse, line, polyline, polygon";

/// What a level's copy carries from its original as the browser computes it: the inherited
/// properties that shape what a stroke or a fill covers.
const CARRIED = ["stroke-linecap", "stroke-linejoin", "stroke-miterlimit", "stroke-dashoffset", "fill-rule"];

/// An open run's two ends, each with the way out of the run there, in the shape's own coordinates:
/// a line's, a polyline's, and each subpath of a path not closed back on its start (a path with an
/// arc read as one run, from its first point to its last unless it closes); none for a closed shape.
function openEnds(g: SVGGeometryElement): [Pt, Pt][] {
  const ends = (pts: Pt[]): [Pt, Pt][] => {
    if (pts.length < 2) return [];
    const [a, b, y, z] = [pts[0], pts[1], pts[pts.length - 2], pts[pts.length - 1]];
    return a.x === z.x && a.y === z.y ? [] : [[a, { x: a.x - b.x, y: a.y - b.y }], [z, { x: z.x - y.x, y: z.y - y.y }]];
  };
  if (g instanceof SVGLineElement) return ends([{ x: g.x1.baseVal.value, y: g.y1.baseVal.value }, { x: g.x2.baseVal.value, y: g.y2.baseVal.value }]);
  if (g instanceof SVGPolylineElement) {
    const pts: Pt[] = [];
    for (let i = 0; i < g.points.numberOfItems; i++) pts.push(g.points.getItem(i));
    return ends(pts);
  }
  if (g instanceof SVGPathElement) {
    const d = g.getAttribute("d") ?? "";
    const subs = subpaths(d);
    if (subs) return subs.flatMap(ends);
    if (/[zZ]/.test(d)) return [];
    const len = g.getTotalLength();
    const step = Math.min(1, len / 2);
    return ends([g.getPointAtLength(0), g.getPointAtLength(step), g.getPointAtLength(len - step), g.getPointAtLength(len)]);
  }
  return [];
}

/// What a shape paints, as points in the frame `m` carries it to: a point every quarter unit of that
/// frame or so along its outline (`getPointAtLength`), each spread by half the stroke's width `w`
/// across the way the outline runs there (none for a body, `w` 0), and an open run's ends grown by
/// its cap (a square's half width out of the run, a round's all round). A mitre join's tip past half
/// the width is not followed.
function painted(g: SVGGeometryElement, m: DOMMatrix, w: number, cap: string): Pt[] {
  const len = g.getTotalLength();
  const scale = Math.sqrt(Math.abs(m.a * m.d - m.b * m.c)) || 1;
  const n = Math.min(4096, Math.max(64, Math.ceil((len * scale) / 0.25)));
  const at = (i: number) => g.getPointAtLength(Math.min(len, Math.max(0, (len * i) / n)));
  const h = w / 2;
  const out: Pt[] = [];
  const put = (x: number, y: number) => {
    const q = new DOMPoint(x, y).matrixTransform(m);
    out.push({ x: q.x, y: q.y });
  };
  for (let i = 0; i <= n; i++) {
    const p = at(i), a = at(i - 1), b = at(i + 1);
    const dx = b.x - a.x, dy = b.y - a.y, l = Math.hypot(dx, dy);
    if (!h || !l) {
      put(p.x, p.y);
      continue;
    }
    put(p.x - (dy / l) * h, p.y + (dx / l) * h);
    put(p.x + (dy / l) * h, p.y - (dx / l) * h);
  }
  if (h && cap !== "butt") {
    for (const [e, out_] of openEnds(g)) {
      const l = Math.hypot(out_.x, out_.y);
      if (!l) continue;
      const [ux, uy] = [out_.x / l, out_.y / l];
      if (cap === "square") {
        put(e.x + ux * h - uy * h, e.y + uy * h + ux * h);
        put(e.x + ux * h + uy * h, e.y + uy * h - ux * h);
      } else for (let k = 0; k < 32; k++) put(e.x + h * Math.cos((k * Math.PI) / 16), e.y + h * Math.sin((k * Math.PI) / 16));
    }
  }
  return out;
}

type Pt = { x: number; y: number };
type Run = { box: Box; dir: Pt | null };

/// One straight run of points, its box and its way from its first point to its farthest (none for a
/// run of one point); null where the points do not lie on one straight line.
function straight(pts: Pt[]): Run | null {
  const a = pts[0];
  const far = pts.reduce((f, p) => (Math.hypot(p.x - a.x, p.y - a.y) > Math.hypot(f.x - a.x, f.y - a.y) ? p : f), a);
  const dx = far.x - a.x, dy = far.y - a.y, n = Math.hypot(dx, dy);
  const x = Math.min(...pts.map((p) => p.x)), y = Math.min(...pts.map((p) => p.y));
  const box = { x, y, width: Math.max(...pts.map((p) => p.x)) - x, height: Math.max(...pts.map((p) => p.y)) - y };
  if (!n) return { box, dir: null };
  const off = Math.max(...pts.map((p) => Math.abs((p.x - a.x) * dy - (p.y - a.y) * dx) / n));
  return off <= n * 1e-6 + 1e-6 ? { box, dir: { x: dx, y: dy } } : null;
}

/// A path's subpaths as the points that make each (every end and control point, absolute), from its
/// data; null where a subpath holds an arc (no run of its points says it is straight).
function subpaths(d: string): Pt[][] | null {
  const toks = d.match(/[MmLlHhVvCcSsQqTtAaZz]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g) ?? [];
  const out: Pt[][] = [];
  let cur: Pt[] = [];
  let x = 0, y = 0, sx = 0, sy = 0, cmd = "", i = 0;
  const num = () => Number(toks[i++]);
  const put = (px: number, py: number) => { x = px; y = py; cur.push({ x, y }); };
  while (i < toks.length) {
    if (/[A-Za-z]/.test(toks[i])) cmd = toks[i++];
    else if (!cmd) return null;
    const rel = cmd === cmd.toLowerCase();
    const ox = rel ? x : 0, oy = rel ? y : 0;
    switch (cmd.toUpperCase()) {
      case "M": {
        if (cur.length) out.push(cur);
        cur = [];
        put(ox + num(), oy + num());
        sx = x; sy = y;
        cmd = rel ? "l" : "L";
        break;
      }
      case "L": case "T": put(ox + num(), oy + num()); break;
      case "H": put(ox + num(), y); break;
      case "V": put(x, oy + num()); break;
      case "C": { const p = [num(), num(), num(), num(), num(), num()]; cur.push({ x: ox + p[0], y: oy + p[1] }, { x: ox + p[2], y: oy + p[3] }); put(ox + p[4], oy + p[5]); break; }
      case "S": case "Q": { const p = [num(), num(), num(), num()]; cur.push({ x: ox + p[0], y: oy + p[1] }); put(ox + p[2], oy + p[3]); break; }
      case "Z": put(sx, sy); cmd = ""; break;
      default: return null;
    }
  }
  if (cur.length) out.push(cur);
  return out;
}

/// The straight runs an element draws with no area, each its box and its way in its own coordinates:
/// a line; a shape with no width or no height; a path every subpath of which runs straight (two
/// walls Inkscape's Combine made one, an L of two); a polyline or polygon all of whose points lie on
/// one line. Null for an element with an area.
function runsOf(el: SVGGraphicsElement, box: DOMRect): Run[] | null {
  if (el instanceof SVGLineElement) return [straight([{ x: el.x1.baseVal.value, y: el.y1.baseVal.value }, { x: el.x2.baseVal.value, y: el.y2.baseVal.value }])!];
  if (box.width === 0) return [{ box, dir: { x: 0, y: 1 } }];
  if (box.height === 0) return [{ box, dir: { x: 1, y: 0 } }];
  if (el instanceof SVGPathElement) {
    const subs = subpaths(el.getAttribute("d") ?? "");
    if (!subs || !subs.length) return null;
    const runs = subs.map(straight);
    return runs.every((r) => r !== null) ? (runs as Run[]) : null;
  }
  if (el instanceof SVGPolylineElement || el instanceof SVGPolygonElement) {
    const pts: Pt[] = [];
    for (let i = 0; i < el.points.numberOfItems; i++) pts.push(el.points.getItem(i));
    const r = pts.length ? straight(pts) : null;
    return r ? [r] : null;
  }
  return null;
}

/// Whether an element draws no area, and the runs that draw it (each with the shape it is one of):
/// a shape's own runs (`runsOf`); a group every shape it shows of which draws none, and no text, its
/// shapes' runs (a group of no shape: none where its box has no width or no height). Null for an
/// element with an area.
function noAreaParts(el: SVGGraphicsElement, box: DOMRect): { el: SVGGraphicsElement; run: Run }[] | null {
  if (el instanceof SVGGeometryElement) return runsOf(el, box)?.map((run) => ({ el, run })) ?? null;
  if (el instanceof SVGTextElement || Array.from(el.querySelectorAll("text")).some((t) => t.checkVisibility())) return null;
  const shapes = Array.from(el.querySelectorAll<SVGGeometryElement>(GEOMETRY)).filter((g) => g.checkVisibility());
  if (!shapes.length) return box.width === 0 || box.height === 0 ? [] : null;
  const out: { el: SVGGraphicsElement; run: Run }[] = [];
  for (const g of shapes) {
    const runs = runsOf(g, g.getBBox());
    if (!runs) return null;
    out.push(...runs.map((run) => ({ el: g as SVGGraphicsElement, run })));
  }
  return out;
}

/// The shapes an element draws as lines (Q30, decided 2026-11-19: "Its line, as drawn"): the element
/// itself, or a group's shapes at any depth, each whose fill computes to `none` as the drawing
/// stands (written as an attribute, in `style` or on a group above), or which draws no area (a line,
/// a straight path or polyline: `runsOf`). A fill of zero opacity, or of the background's colour, is
/// a fill; a text is a body whatever its fill. Read before the page paints anything.
function linesOf(el: SVGGraphicsElement): SVGGeometryElement[] {
  const shapes = el instanceof SVGGeometryElement ? [el] : el instanceof SVGTextElement ? [] : Array.from(el.querySelectorAll<SVGGeometryElement>(GEOMETRY));
  return shapes.filter((g) => getComputedStyle(g).fill === "none" || runsOf(g, g.getBBox()) !== null);
}

/// The copy of an element's shapes a level along lines is drawn on: the element's geometry as it
/// stands, carried by `m` from its own coordinates (its own transform among them) to the frame it is
/// drawn in; its texts, titles and descriptions dropped; no id, class or `data-` attribute kept but a
/// line's mark (`sv-line`, its drawn width in its `style`), every other shape marked a body
/// (`sv-body`); each shape's caps, joins, mitre limit, dash offset and fill rule as the browser
/// computes them on the original (written on it or inherited from a group above, which the copy
/// does not stand under), so the level covers the track's line and body exactly.
function copyOf(el: SVGGraphicsElement, m: DOMMatrix): SVGGraphicsElement {
  const copy = el.cloneNode(true) as SVGGraphicsElement;
  copy.querySelectorAll("text, title, desc").forEach((n) => n.remove());
  for (const n of [copy, ...Array.from(copy.querySelectorAll("*"))]) {
    n.removeAttribute("id");
    for (const a of Array.from(n.attributes)) if (a.name.startsWith("data-")) n.removeAttribute(a.name);
    const line = n.classList.contains("sv-line");
    n.removeAttribute("class");
    if (line) n.classList.add("sv-line");
  }
  const from = el.matches(GEOMETRY) ? [el] : Array.from(el.querySelectorAll(GEOMETRY));
  const to = copy.matches(GEOMETRY) ? [copy] : Array.from(copy.querySelectorAll<SVGElement>(GEOMETRY));
  to.forEach((g, i) => {
    if (!g.classList.contains("sv-line")) g.classList.add("sv-body");
    const c = getComputedStyle(from[i]);
    for (const p of CARRIED) g.style.setProperty(p, c.getPropertyValue(p));
  });
  copy.setAttribute("transform", `matrix(${m.a} ${m.b} ${m.c} ${m.d} ${m.e} ${m.f})`);
  return copy;
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

/// The bands a level with no area fills: each run's box widened to its shape's stroke's width across
/// it.
function bands(parts: NonNullable<Bound["noArea"]>): Box[] {
  return parts.map((p) => widened(p.ob, across(p.m, parseFloat(getComputedStyle(p.el).strokeWidth) || 0, p.dir)));
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

  /// The box a level along lines rises over, in the overlay's frame: up and down, the reach of its
  /// copy's shapes as they are painted now (`painted`: a line's outline spread by its stroke's width
  /// in the state the copy carries, with its caps; a body's outline), whatever turns, skews or
  /// squashes them; across, past every line (the level is a height, and a join reaches out
  /// sideways). None for no shape shown.
  function alongBox(along: SVGGElement): Box | null {
    const oc = overlay.getCTM();
    if (!oc) return null;
    const o = oc.inverse();
    let [lo, hi] = [Infinity, -Infinity];
    for (const g of Array.from(along.querySelectorAll<SVGGeometryElement>(GEOMETRY)).filter((g) => g.checkVisibility())) {
      const gc = g.getCTM();
      if (!gc) continue;
      const c = getComputedStyle(g);
      const line = g.classList.contains("sv-line");
      for (const p of painted(g, o.multiply(gc), line ? parseFloat(c.strokeWidth) || 0 : 0, line ? c.strokeLinecap : "butt")) {
        lo = Math.min(lo, p.y);
        hi = Math.max(hi, p.y);
      }
    }
    return lo <= hi ? { x: -1e6, y: lo, width: 2e6, height: hi - lo } : null;
  }

  /// The widths its copy's lines are painted at now: its box read again when they change (the
  /// alarm's), never on every paint.
  const alongWidths = (along: SVGGElement) => Array.from(along.querySelectorAll(".sv-line")).map((g) => getComputedStyle(g).strokeWidth).join(" ");

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
      for (const p of b.noArea) p.ob = boxIn(overlay, p.el, p.box);
      const sel = el.classList.contains("sv-selected");
      el.classList.remove("sv-selected");
      // The fill rises over the box around its runs' bands, held to the bands themselves (an L, two
      // walls: over their strokes, never between them).
      const bs = bands(b.noArea);
      b.fillBox = union(bs) ?? b.ob;
      b.runsClip?.replaceChildren(...bs.map((r) => s("rect", { x: r.x, y: r.y, width: r.width, height: r.height })));
      if (sel) el.classList.add("sv-selected");
    } else if (b.along) {
      b.fillBox = alongBox(b.along) ?? { ...b.ob };
      b.alongWidths = alongWidths(b.along);
    }
    else b.fillBox = { ...b.ob };
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
    // A shape the drawing draws as a line takes its state on its line alone, at the width it was
    // drawn with (Q30): marked, that width written beside it (`--sv-w`), as the drawing stands.
    const lines = linesOf(el);
    for (const g of lines) {
      g.style.setProperty("--sv-w", getComputedStyle(g).strokeWidth);
      g.classList.add("sv-line");
    }
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
    const runs = def.show === "level" ? noAreaParts(el, box) : null;
    const noArea = runs !== null;
    // A level on a line that closes an area (Q30: a ring, a bent pipe, a hollow rect drawn with no
    // fill, or a group holding one) runs along its lines: drawn on a copy of the element's shapes.
    const along = def.show === "level" && !noArea && lines.some((g) => g.checkVisibility() && runsOf(g, g.getBBox()) === null);
    const oc0 = overlay.getCTM();
    let na: Bound["noArea"];
    if (runs && oc0) {
      na = runs.flatMap(({ el: g, run }) => {
        const gc = g.getCTM();
        if (!gc) return [];
        return [{ el: g, box: run.box, ob: boxIn(overlay, g, run.box), m: oc0.inverse().multiply(gc), dir: run.dir }];
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
      const pc = (el.parentNode as SVGGraphicsElement).getCTM();
      const m = pc && oc0 ? pc.inverse().multiply(oc0) : new DOMMatrix();
      const frame = s("g", { transform: `matrix(${m.a} ${m.b} ${m.c} ${m.d} ${m.e} ${m.f})` });
      if (along) {
        // Along lines (Q30): the level's rect is a clip, in the overlay's frame, and what it clips a
        // copy of the element's shapes, carried into that frame, over the element (its track): each
        // line stroked and each body filled in the state's colour (`.sv-along`).
        const ec = el.getCTM();
        b.fill = s("rect", { height: 0, class: "sv-along-rect" }) as SVGRectElement;
        b.along = s("g", { class: "sv-along", "data-for": def.id, "clip-path": `url(#sv-along-${def.id})` }, copyOf(el, oc0 && ec ? oc0.inverse().multiply(ec) : new DOMMatrix())) as SVGGElement;
        frame.append(s("clipPath", { id: `sv-along-${def.id}` }, b.fill), b.along);
      } else {
        b.fill = s("rect", { height: 0, class: "sv-fill", "data-for": def.id }) as SVGRectElement;
        frame.append(b.fill);
      }
      if (noArea) {
        // Its runs' bands, in the overlay's frame as the fill is (written by `measure`).
        b.runsClip = s("clipPath", { id: `sv-runs-${def.id}` }) as SVGClipPathElement;
        frame.append(b.runsClip);
        b.fill.setAttribute("clip-path", `url(#sv-runs-${def.id})`);
      }
      const holder = s("g", { class: "sv-fill-holder", "data-for": def.id }, s("g", { class: "sv-reset" }, frame));
      if ((el instanceof SVGGeometryElement || el instanceof SVGTextElement) && !noArea && !along) {
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
      // The state on what draws the level: its fill, or the copy a level along lines is drawn on.
      const carrier = b.along ?? b.fill;
      for (const c of Array.from(carrier.classList)) if (c.startsWith("sv-state-")) carrier.classList.remove(c);
      carrier.classList.add(cls);
      // Along lines, the box read again in the state just set (the alarm's width), while the drawing
      // is on screen (off it nothing is measured; the layout paints again when shown).
      if (b.along && svg.isConnected && alongWidths(b.along) !== b.alongWidths) {
        b.fillBox = alongBox(b.along) ?? b.fillBox;
        b.alongWidths = alongWidths(b.along);
        b.fill.setAttribute("x", String(b.fillBox.x));
        b.fill.setAttribute("width", String(b.fillBox.width));
      }
      // With no area, the stroke's width read again in the state just set (unselected: the
      // selection's own stroke is no part of the track).
      if (b.noArea) {
        const sel = b.el.classList.contains("sv-selected");
        b.el.classList.remove("sv-selected");
        const bs = bands(b.noArea);
        b.fillBox = union(bs) ?? b.ob;
        b.runsClip?.replaceChildren(...bs.map((r) => s("rect", { x: r.x, y: r.y, width: r.width, height: r.height })));
        if (sel) b.el.classList.add("sv-selected");
        b.fill.setAttribute("x", String(b.fillBox.x));
        b.fill.setAttribute("width", String(b.fillBox.width));
      }
      const f = o.level ?? 0;
      b.fill.setAttribute("y", String(b.fillBox.y + b.fillBox.height * (1 - f)));
      b.fill.setAttribute("height", String(b.fillBox.height * f));
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
  // A press becomes a pan only once the pointer has gone a few pixels from where it went down (a
  // hand's jitter in a click is no pan), and the click that ends a pan picks nothing and clears
  // nothing: stopped ahead of every handler, the elements' and the background's (the capture).
  // Only the primary button's press pans (another's release may never reach the page: a native menu
  // takes it), and a pan ends on a move with that button no longer held, its release lost or not.
  const SLOP = 4;
  let drag: { x: number; y: number; from: { x: number; y: number }; panning: boolean } | null = null;
  let panned = false;
  // What the last press went down on: a click that ends it picks that (`onSelect` below).
  let pressedOn: EventTarget | null = null;
  svg.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    pressedOn = e.target;
    drag = { x: e.clientX, y: e.clientY, from: { x: e.clientX, y: e.clientY }, panning: false };
    panned = false;
  });
  window.addEventListener("mouseup", () => { if (drag?.panning) panned = true; drag = null; });
  svg.addEventListener("click", (e) => { if (panned) { panned = false; e.stopImmediatePropagation(); } }, true);
  svg.addEventListener("mousemove", (e) => {
    if (!drag) return;
    if (!(e.buttons & 1)) {
      drag = null;
      return;
    }
    if (!drag.panning) {
      if (Math.hypot(e.clientX - drag.from.x, e.clientY - drag.from.y) < SLOP) return;
      drag.panning = true;
    }
    if (e.clientX === drag.x && e.clientY === drag.y) return;
    fitted = false;
    const r = svg.getBoundingClientRect();
    vb.x -= ((e.clientX - drag.x) / r.width) * vb.w;
    vb.y -= ((e.clientY - drag.y) / r.height) * vb.h;
    drag.x = e.clientX;
    drag.y = e.clientY;
    setVb();
  });
  svg.addEventListener("wheel", (e) => { e.preventDefault(); zoom(e.deltaY > 0 ? 1.12 : 1 / 1.12); }, { passive: false });
  // A click no element took (the browser sends it to what the press and the release share): it picks
  // what its press went down on (a marker slid from under a still pointer, a release a pixel past an
  // element's edge), and clears the selection only when the press went down on the background. A
  // click of no press (one dispatched, its `detail` 0) reads its own target.
  svg.addEventListener("click", (e) => {
    const from = e.detail > 0 ? pressedOn : e.target;
    const at = from instanceof Element ? (from.closest("[data-object]") as SVGElement | null) : null;
    if (at && svg.contains(at)) onSelect(at.dataset.object ?? "", at.dataset.key ?? null);
    else onSelect("", null);
  });

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

