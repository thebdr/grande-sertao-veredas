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

/// A shape's outline as the runs of points that draw it, in its own coordinates, each with whether
/// it closes and the way out at each of its ends: a path's subpaths (its curves and arcs followed,
/// closed by a `Z` alone, the ways out its own tangents), a polyline's points, a polygon's (closed),
/// a line's two ends, a rect's four corners; a circle, an ellipse or a rounded rect followed by the
/// browser (`getPointAtLength`, `n` points, one closed run). Each point says whether it is a corner
/// (where two segments meet, joined as the stroke's join says) or a point inside a curve (the browser
/// strokes a curve smoothly through it).
type Outline = { pts: Pt[]; corner: boolean[]; closed: boolean; outs: [Pt, Pt] | null };
function outline(g: SVGGeometryElement, n: number): Outline[] {
  const plain = (pts: Pt[], closed: boolean, corner = true): Outline => ({ pts, corner: pts.map(() => corner), closed, outs: null });
  if (g instanceof SVGLineElement) return [plain([{ x: g.x1.baseVal.value, y: g.y1.baseVal.value }, { x: g.x2.baseVal.value, y: g.y2.baseVal.value }], false)];
  if (g instanceof SVGPolylineElement || g instanceof SVGPolygonElement) {
    const pts: Pt[] = [];
    for (let i = 0; i < g.points.numberOfItems; i++) pts.push(g.points.getItem(i));
    return [plain(pts, g instanceof SVGPolygonElement)];
  }
  // A rect's corners by its used radii (one written `auto`, or not at all, takes the other's): sharp
  // where either is 0, as the browser draws it.
  const radius = (own: string, other: string) => parseFloat(own === "auto" ? (other === "auto" ? "0" : other) : own) || 0;
  const rc = g instanceof SVGRectElement ? getComputedStyle(g) : null;
  if (g instanceof SVGRectElement && rc && (!radius(rc.getPropertyValue("rx"), rc.getPropertyValue("ry")) || !radius(rc.getPropertyValue("ry"), rc.getPropertyValue("rx")))) {
    const [x, y, w, h] = [g.x.baseVal.value, g.y.baseVal.value, g.width.baseVal.value, g.height.baseVal.value];
    return [plain([{ x, y }, { x: x + w, y }, { x: x + w, y: y + h }, { x, y: y + h }], true)];
  }
  if (g instanceof SVGPathElement) {
    const subs = subpaths(g.getAttribute("d") ?? "");
    if (!subs) return [];
    return subs.map((s) => {
      // Its ways out from the points that make it: a curve's own tangent at each end (its handle,
      // or the next one where the handle lies on its node).
      const off = (pts: Pt[], e: Pt) => pts.find((p) => p.x !== e.x || p.y !== e.y);
      const [a, z] = [s.pts[0], s.pts[s.pts.length - 1]];
      const b = off(s.pts, a), y = off([...s.pts].reverse(), z);
      return { pts: s.drawn, corner: s.corner, closed: s.closed, outs: b && y ? [{ x: a.x - b.x, y: a.y - b.y }, { x: z.x - y.x, y: z.y - y.y }] : null };
    });
  }
  const len = g.getTotalLength();
  const pts: Pt[] = [];
  for (let i = 0; i < n; i++) pts.push(g.getPointAtLength((len * i) / n));
  return [plain(pts, true, false)];
}

/// What a shape paints, as points in the frame `m` carries it to: its outline (`outline`), each edge
/// spread by half the stroke's width `w` across its own way (none for a body, `w` 0), each corner
/// joined as the browser joins it (`join`: a mitre's tip where its length over the width stays
/// within `limit`, else a bevel; a bevel; a round), a point inside a curve joined round (so a cusp,
/// where the curve turns back on itself, is a disc of half the width all round, as the browser's
/// stroker paints it whatever the join), an open run's ends grown by its cap (a square's half width out of the run, a
/// round's all round), and a run of no length a dot where its cap is round or square (a square one
/// along the axes). A stroke that does not scale (`fixed`: `vector-effect: non-scaling-stroke`,
/// Inkscape's hairline) is spread in the frame `m` carries it to, `fixed` its half width there.
function painted(g: SVGGeometryElement, m: DOMMatrix, w: number, cap: string, join: string, limit: number, fixed: number | null = null): Pt[] {
  const scale = Math.sqrt(Math.abs(m.a * m.d - m.b * m.c)) || 1;
  const n = Math.min(4096, Math.max(64, Math.ceil((g.getTotalLength() * scale) / 0.25)));
  const h = fixed ?? w / 2;
  const out: Pt[] = [];
  const to = (x: number, y: number) => new DOMPoint(x, y).matrixTransform(m);
  const put = fixed === null ? (x: number, y: number) => { const q = to(x, y); out.push({ x: q.x, y: q.y }); } : (x: number, y: number) => out.push({ x, y });
  const unit = (d: Pt) => { const l = Math.hypot(d.x, d.y); return l ? { x: d.x / l, y: d.y / l } : null; };
  for (const shape of outline(g, n)) {
    // A stroke that does not scale: its outline carried to the frame first, its ways out with it.
    const run = fixed === null ? shape : {
      ...shape,
      pts: shape.pts.map((p) => { const q = to(p.x, p.y); return { x: q.x, y: q.y }; }),
      outs: shape.outs && (shape.outs.map((o) => ({ x: m.a * o.x + m.c * o.y, y: m.b * o.x + m.d * o.y })) as [Pt, Pt]),
    };
    // Its points, each once (a point written twice, a curve's handle on its node), a closed run's
    // start not again at its end; a point a corner where any of its copies is. One point within
    // rounding of the shape's size (a billionth of it): relative coordinates summed in doubles miss
    // the point they come back to by a hair, a leaf closed on its tip by its last curve and a `Z`.
    const xs = run.pts.map((p) => p.x), ys = run.pts.map((p) => p.y);
    const eps = Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys), 1) * 1e-9;
    const same = (a: Pt, b: Pt) => Math.abs(a.x - b.x) <= eps && Math.abs(a.y - b.y) <= eps;
    const pts: Pt[] = [], corner: boolean[] = [];
    run.pts.forEach((p, k) => {
      const last = pts[pts.length - 1];
      if (last && same(last, p)) corner[corner.length - 1] ||= run.corner[k];
      else { pts.push(p); corner.push(run.corner[k]); }
    });
    if (run.closed && pts.length > 1 && same(pts[0], pts[pts.length - 1])) { pts.pop(); corner[0] ||= corner.pop()!; }
    for (const p of pts) put(p.x, p.y);
    if (!h) continue;
    if (pts.length === 1) {
      // A run of no length: a dot, round or square (along the axes), where the cap is one.
      const e = pts[0];
      if (cap === "round") for (let k = 0; k < 32; k++) put(e.x + h * Math.cos((k * Math.PI) / 16), e.y + h * Math.sin((k * Math.PI) / 16));
      else if (cap === "square") for (const [sx, sy] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) put(e.x + sx * h, e.y + sy * h);
      continue;
    }
    const edges = run.closed ? pts.length : pts.length - 1;
    const dirs: Pt[] = [];
    for (let k = 0; k < edges; k++) {
      const [a, b] = [pts[k], pts[(k + 1) % pts.length]];
      const u = unit({ x: b.x - a.x, y: b.y - a.y })!;
      dirs.push(u);
      for (const p of [a, b]) {
        put(p.x - u.y * h, p.y + u.x * h);
        put(p.x + u.y * h, p.y - u.x * h);
      }
    }
    // Each corner where one edge meets the next, joined on its outer side; a point inside a curve
    // joined round (a cusp, the curve turning back, so a disc all round: the join's half on its outer
    // side, the edges' spread the other).
    for (let k = run.closed ? 0 : 1; k < pts.length - (run.closed ? 0 : 1); k++) {
      const d1 = dirs[(k - 1 + edges) % edges], d2 = dirs[k % edges], v = pts[k];
      const cross = d1.x * d2.y - d1.y * d2.x, dot = d1.x * d2.x + d1.y * d2.y;
      if (Math.abs(cross) < 1e-9 && dot > 0) continue;
      const joined = corner[k] ? join : "round";
      const s = cross > 0 ? -1 : 1;
      const [n1, n2] = [{ x: -d1.y * s, y: d1.x * s }, { x: -d2.y * s, y: d2.x * s }];
      if (joined === "round") {
        const [a1, a2] = [Math.atan2(n1.y, n1.x), Math.atan2(n2.y, n2.x)];
        let da = a2 - a1;
        while (da > Math.PI) da -= 2 * Math.PI;
        while (da < -Math.PI) da += 2 * Math.PI;
        // A line turning exactly back on itself: its half disc on the side of its tip, the way it
        // was going.
        if (Math.abs(cross) < 1e-9) da = Math.sign(n1.x * d1.y - n1.y * d1.x) * Math.PI;
        const steps = Math.max(1, Math.ceil(Math.abs(da) / (Math.PI / 16)));
        for (let q = 1; q < steps; q++) put(v.x + h * Math.cos(a1 + (da * q) / steps), v.y + h * Math.sin(a1 + (da * q) / steps));
      } else if (joined !== "bevel") {
        // A mitre: its length over the width is one over the sine of half the angle between the
        // edges; past the limit the browser bevels.
        const half = Math.acos(Math.max(-1, Math.min(1, -dot))) / 2;
        const ratio = half > 1e-9 ? 1 / Math.sin(half) : Infinity;
        const b = unit({ x: n1.x + n2.x, y: n1.y + n2.y });
        if (b && ratio <= limit) put(v.x + b.x * h * ratio, v.y + b.y * h * ratio);
      }
    }
    // An open run's ends, by their own ways out (a path's tangents; else the first point off it).
    if (!run.closed && cap !== "butt") {
      const outs = run.outs ?? [{ x: pts[0].x - pts[1].x, y: pts[0].y - pts[1].y }, { x: pts[pts.length - 1].x - pts[pts.length - 2].x, y: pts[pts.length - 1].y - pts[pts.length - 2].y }];
      for (const [e, o] of [[pts[0], outs[0]], [pts[pts.length - 1], outs[1]]] as [Pt, Pt][]) {
        const u = unit(o);
        if (!u) continue;
        if (cap === "square") {
          put(e.x + u.x * h - u.y * h, e.y + u.y * h + u.x * h);
          put(e.x + u.x * h + u.y * h, e.y + u.y * h - u.x * h);
        } else for (let k = 0; k < 32; k++) put(e.x + h * Math.cos((k * Math.PI) / 16), e.y + h * Math.sin((k * Math.PI) / 16));
      }
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

/// A point `t` of the way along a Bézier curve of the points `p` (de Casteljau).
function bezier(p: Pt[], t: number): Pt {
  let q: Pt[] = p;
  while (q.length > 1) {
    const r: Pt[] = q;
    q = r.slice(1).map((b, k) => ({ x: r[k].x + (b.x - r[k].x) * t, y: r[k].y + (b.y - r[k].y) * t }));
  }
  return q[0];
}

/// The points an elliptical arc draws from (x1, y1) to (x2, y2), `k` of them past its start (SVG's
/// own conversion of an arc's end points to its centre); a radius of 0 a straight line to its end.
function arcPoints(x1: number, y1: number, rx: number, ry: number, phi: number, large: boolean, sweep: boolean, x2: number, y2: number, k: number): Pt[] {
  if (x1 === x2 && y1 === y2) return [];
  rx = Math.abs(rx); ry = Math.abs(ry);
  if (!rx || !ry) return [{ x: x2, y: y2 }];
  const [c, s] = [Math.cos(phi), Math.sin(phi)];
  const [dx, dy] = [(x1 - x2) / 2, (y1 - y2) / 2];
  const [xp, yp] = [c * dx + s * dy, -s * dx + c * dy];
  const lam = (xp * xp) / (rx * rx) + (yp * yp) / (ry * ry);
  if (lam > 1) { rx *= Math.sqrt(lam); ry *= Math.sqrt(lam); }
  const num = rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp, den = rx * rx * yp * yp + ry * ry * xp * xp;
  const co = (large === sweep ? -1 : 1) * Math.sqrt(Math.max(0, num / den));
  const [cxp, cyp] = [(co * rx * yp) / ry, (-co * ry * xp) / rx];
  const [cx, cy] = [c * cxp - s * cyp + (x1 + x2) / 2, s * cxp + c * cyp + (y1 + y2) / 2];
  const ang = (ux: number, uy: number, vx: number, vy: number) => Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
  const t1 = ang(1, 0, (xp - cxp) / rx, (yp - cyp) / ry);
  let dt = ang((xp - cxp) / rx, (yp - cyp) / ry, (-xp - cxp) / rx, (-yp - cyp) / ry);
  if (!sweep && dt > 0) dt -= 2 * Math.PI;
  else if (sweep && dt < 0) dt += 2 * Math.PI;
  const out: Pt[] = [];
  for (let q = 1; q <= k; q++) {
    const a = t1 + (dt * q) / k;
    const [px, py] = [rx * Math.cos(a), ry * Math.sin(a)];
    out.push(q === k ? { x: x2, y: y2 } : { x: c * px - s * py + cx, y: s * px + c * py + cy });
  }
  return out;
}

/// A subpath: the points that make it (every end and control point, absolute; an arc's the points
/// it draws), the points it draws (each curve and arc followed at 32 points), and whether a `Z`
/// closes it.
type Sub = { pts: Pt[]; drawn: Pt[]; corner: boolean[]; closed: boolean };

/// A path's subpaths, from its data; null where its data cannot be read. A subpath is closed by a `Z` alone, whatever its ends; after one, the next
/// command starts a new subpath at the closed one's start. A smooth curve's first control (`S`, `T`)
/// is the one before it mirrored, as the browser draws it. A move with nothing drawn after it is no
/// subpath (the browser strokes none); each point drawn says whether it ends a segment (a corner) or
/// lies inside a curve or an arc.
function subpaths(d: string): Sub[] | null {
  const toks = d.match(/[MmLlHhVvCcSsQqTtAaZz]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g) ?? [];
  const out: Sub[] = [];
  let cur: Pt[] = [], drawn: Pt[] = [], corner: boolean[] = [];
  let x = 0, y = 0, sx = 0, sy = 0, cmd = "", i = 0, after = false;
  let c2: Pt | null = null, qc: Pt | null = null;
  const num = () => Number(toks[i++]);
  const begin = () => {
    if (after && !cur.length) { cur.push({ x: sx, y: sy }); drawn.push({ x: sx, y: sy }); corner.push(true); }
    after = false;
  };
  const put = (px: number, py: number) => { begin(); x = px; y = py; cur.push({ x, y }); drawn.push({ x, y }); corner.push(true); };
  const bend = (pts: Pt[]) => {
    begin();
    const all = [{ x, y }, ...pts];
    for (let k = 1; k <= 32; k++) { drawn.push(bezier(all, k / 32)); corner.push(k === 32); }
    cur.push(...pts);
    ({ x, y } = pts[pts.length - 1]);
  };
  const end = (closed: boolean) => { if (drawn.length > 1) out.push({ pts: cur, drawn, corner, closed }); cur = []; drawn = []; corner = []; };
  while (i < toks.length) {
    // A command is a letter alone: a number's exponent holds one too (`2.5e-4`).
    if (/^[A-Za-z]$/.test(toks[i])) cmd = toks[i++];
    else if (!cmd) return null;
    const rel = cmd === cmd.toLowerCase();
    const ox = rel ? x : 0, oy = rel ? y : 0;
    switch (cmd.toUpperCase()) {
      case "M": {
        end(false);
        after = false;
        put(ox + num(), oy + num());
        sx = x; sy = y;
        cmd = rel ? "l" : "L";
        c2 = qc = null;
        break;
      }
      case "L": put(ox + num(), oy + num()); c2 = qc = null; break;
      case "H": put(ox + num(), y); c2 = qc = null; break;
      case "V": put(x, oy + num()); c2 = qc = null; break;
      case "C": {
        const p = [num(), num(), num(), num(), num(), num()];
        const b: Pt = { x: ox + p[2], y: oy + p[3] };
        bend([{ x: ox + p[0], y: oy + p[1] }, b, { x: ox + p[4], y: oy + p[5] }]);
        c2 = b; qc = null;
        break;
      }
      case "S": {
        const p = [num(), num(), num(), num()];
        const a: Pt = c2 ? { x: 2 * x - c2.x, y: 2 * y - c2.y } : { x, y };
        const b: Pt = { x: ox + p[0], y: oy + p[1] };
        bend([a, b, { x: ox + p[2], y: oy + p[3] }]);
        c2 = b; qc = null;
        break;
      }
      case "Q": {
        const p = [num(), num(), num(), num()];
        const q: Pt = { x: ox + p[0], y: oy + p[1] };
        bend([q, { x: ox + p[2], y: oy + p[3] }]);
        qc = q; c2 = null;
        break;
      }
      case "T": {
        const p = [num(), num()];
        const q: Pt = qc ? { x: 2 * x - qc.x, y: 2 * y - qc.y } : { x, y };
        bend([q, { x: ox + p[0], y: oy + p[1] }]);
        qc = q; c2 = null;
        break;
      }
      case "A": {
        const p = [num(), num(), num(), num(), num(), num(), num()];
        begin();
        const arc = arcPoints(x, y, p[0], p[1], (p[2] * Math.PI) / 180, p[3] !== 0, p[4] !== 0, ox + p[5], oy + p[6], 32);
        cur.push(...arc);
        drawn.push(...arc);
        corner.push(...arc.map((_, k) => k === arc.length - 1));
        ({ x, y } = { x: ox + p[5], y: oy + p[6] });
        c2 = qc = null;
        break;
      }
      case "Z": put(sx, sy); end(true); after = true; x = sx; y = sy; cmd = ""; c2 = qc = null; break;
      default: return null;
    }
  }
  end(false);
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
    // Straight by the points that make it, its box where its curves go (a handle reaching past its
    // node along the line draws nothing there).
    const runs = subs.map((s) => { const r = straight(s.pts); return r && { ...r, box: straight(s.drawn)?.box ?? r.box }; });
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
      const w = line ? parseFloat(c.strokeWidth) || 0 : 0;
      // A stroke that does not scale is as wide on the screen as written: its half width in the
      // overlay's frame.
      const px = overlay.getScreenCTM();
      const fixed = line && c.vectorEffect === "non-scaling-stroke" && px ? w / 2 / (Math.sqrt(Math.abs(px.a * px.d - px.b * px.c)) || 1) : null;
      for (const p of painted(g, o.multiply(gc), w, line ? c.strokeLinecap : "butt", c.strokeLinejoin, parseFloat(c.strokeMiterlimit) || 4, fixed)) {
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

