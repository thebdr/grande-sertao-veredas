// The charts, drawn as SVG from numbers the host computed: a sparkline, a trend with its limit
// and breaches, bars against a target, state bars over time, a distribution on a fixed scale.

import { clock, num, s } from "./dom";

export type Pt = [number, number];

function scale(d0: number, d1: number, r0: number, r1: number) {
  const span = d1 - d0 || 1;
  return (v: number) => r0 + ((v - d0) / span) * (r1 - r0);
}

function extent(vals: number[], pad = 0.08): [number, number] {
  if (vals.length === 0) return [0, 1];
  let lo = Math.min(...vals);
  let hi = Math.max(...vals);
  if (lo === hi) {
    lo -= 1;
    hi += 1;
  }
  const p = (hi - lo) * pad;
  return [lo - p, hi + p];
}

/// A small line, no axes.
export function sparkline(pts: Pt[], w = 220, hgt = 32, cls = "spark"): SVGElement {
  const svg = s("svg", { viewBox: `0 0 ${w} ${hgt}`, width: "100%", height: hgt, preserveAspectRatio: "none", class: cls });
  if (pts.length < 2) return svg;
  const x = scale(pts[0][0], pts[pts.length - 1][0], 1, w - 1);
  const [lo, hi] = extent(pts.map((p) => p[1]));
  const y = scale(lo, hi, hgt - 2, 2);
  svg.append(s("path", { d: pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)} ${y(p[1]).toFixed(1)}`).join(" "), class: "line" }));
  return svg;
}

/// A trend over [from, to] with a dashed limit and its breaches as dots; hover shows a value.
export function trend(pts: Pt[], breaches: Pt[], from: number, to: number, limit: number | null, unit: string): SVGElement {
  const W = 600, H = 200, L = 44, R = 10, T = 10, B = 22;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, width: "100%", class: "chart trend" });
  const vals = pts.map((p) => p[1]).concat(limit === null ? [] : [limit]);
  const [lo, hi] = extent(vals);
  const x = scale(from, to, L, W - R);
  const y = scale(lo, hi, H - B, T);
  for (let i = 0; i <= 3; i++) {
    const v = lo + ((hi - lo) * i) / 3;
    svg.append(s("line", { x1: L, x2: W - R, y1: y(v), y2: y(v), class: "grid" }));
    svg.append(s("text", { x: L - 6, y: y(v) + 4, class: "axis", "text-anchor": "end" }, num(v, 1)));
  }
  for (let i = 0; i <= 4; i++) {
    const t = from + ((to - from) * i) / 4;
    svg.append(s("text", { x: x(t), y: H - 6, class: "axis", "text-anchor": i === 0 ? "start" : i === 4 ? "end" : "middle" }, i === 4 ? "now" : clock(t, false)));
  }
  if (limit !== null) svg.append(s("line", { x1: L, x2: W - R, y1: y(limit), y2: y(limit), class: "limit" }));
  if (pts.length > 1) svg.append(s("path", { d: pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)} ${y(p[1]).toFixed(1)}`).join(" "), class: "line" }));
  for (const b of breaches) svg.append(s("circle", { cx: x(b[0]), cy: y(b[1]), r: 3, class: "breach" }));
  const tip = s("g", { class: "tip", visibility: "hidden" });
  const tipDot = s("circle", { r: 3.5, class: "tipdot" });
  const tipText = s("text", { class: "tiptext" });
  tip.append(tipDot, tipText);
  svg.append(tip);
  svg.addEventListener("mousemove", (ev) => {
    if (pts.length === 0) return;
    const r = svg.getBoundingClientRect();
    const px = ((ev.clientX - r.left) / r.width) * W;
    let best = pts[0];
    for (const p of pts) if (Math.abs(x(p[0]) - px) < Math.abs(x(best[0]) - px)) best = p;
    tip.setAttribute("visibility", "visible");
    tipDot.setAttribute("cx", String(x(best[0])));
    tipDot.setAttribute("cy", String(y(best[1])));
    tipText.setAttribute("x", String(Math.min(x(best[0]) + 6, W - 120)));
    tipText.setAttribute("y", String(Math.max(y(best[1]) - 8, 12)));
    tipText.textContent = `${clock(best[0])} · ${num(best[1], 2)}${unit ? " " + unit : ""}`;
  });
  svg.addEventListener("mouseleave", () => tip.setAttribute("visibility", "hidden"));
  return svg;
}

export interface Bucket {
  start: number;
  end: number;
  value: number | null;
  n: number;
  running: boolean;
}

/// Bars per bucket against a dashed target; the running bucket outlined.
export function bars(buckets: Bucket[], target: number | null): SVGElement {
  const W = 600, H = 180, L = 44, R = 10, T = 10, B = 22;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, width: "100%", class: "chart bars" });
  const vals = buckets.map((b) => b.value ?? 0).concat(target === null ? [] : [target], [0]);
  const hi = Math.max(...vals) * 1.1 || 1;
  const y = scale(0, hi, H - B, T);
  const bw = (W - L - R) / Math.max(buckets.length, 1);
  svg.append(s("text", { x: L - 6, y: y(hi) + 10, class: "axis", "text-anchor": "end" }, num(hi, 1)));
  svg.append(s("text", { x: L - 6, y: H - B, class: "axis", "text-anchor": "end" }, "0"));
  buckets.forEach((b, i) => {
    const x0 = L + i * bw + 2;
    if (b.value !== null) {
      svg.append(s("rect", { x: x0, y: y(b.value), width: bw - 4, height: Math.max(0, y(0) - y(b.value)), class: b.running ? "bar running" : "bar" }, s("title", {}, `${clock(b.start, false)} · ${num(b.value, 2)} (n ${b.n})`)));
    }
    if (i % Math.ceil(buckets.length / 6) === 0) svg.append(s("text", { x: x0, y: H - 6, class: "axis" }, clock(b.start, false)));
  });
  if (target !== null) {
    svg.append(s("line", { x1: L, x2: W - R, y1: y(target), y2: y(target), class: "limit" }));
    svg.append(s("text", { x: W - R, y: y(target) - 4, class: "axis", "text-anchor": "end" }, `target ${num(target, 2)}`));
  }
  return svg;
}

export interface Segment {
  from: number;
  to: number;
  state: string;
  mark: string;
  word: string;
}

/// One bar of states over [from, to].
export function stateBar(segs: Segment[], from: number, to: number): SVGElement {
  const W = 400, H = 14;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, width: "100%", height: H, preserveAspectRatio: "none", class: "statebar" });
  const x = scale(from, to, 0, W);
  svg.append(s("rect", { x: 0, y: 0, width: W, height: H, class: "st-none" }));
  for (const g of segs) {
    const x0 = Math.max(0, x(g.from));
    const x1 = Math.min(W, x(g.to));
    if (x1 <= x0) continue;
    svg.append(s("rect", { x: x0, y: 0, width: x1 - x0, height: H, class: `st-${g.state}` }, s("title", {}, `${g.word} ${clock(g.from)}–${clock(g.to)}`)));
  }
  return svg;
}

/// A distribution of `bins` counts on a fixed scale, the median at the middle.
export function distribution(bins: number[]): SVGElement {
  const W = 160, H = 26;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, width: W, height: H, class: "dist" });
  const max = Math.max(...bins, 1);
  const bw = W / bins.length;
  const y = scale(0, max, H - 1, 2);
  const d = bins.map((c, i) => `${i ? "L" : "M"}${(i * bw + bw / 2).toFixed(1)} ${y(c).toFixed(1)}`).join(" ");
  svg.append(s("path", { d: `${d} L${W - bw / 2} ${H - 1} L${bw / 2} ${H - 1} Z`, class: "area" }));
  svg.append(s("path", { d, class: "line" }));
  svg.append(s("line", { x1: W / 2, x2: W / 2, y1: 0, y2: H, class: "median" }));
  return svg;
}
