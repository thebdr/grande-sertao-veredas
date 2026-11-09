// Small DOM helpers: elements built by name with attributes and children, no framework.

export const SVG_NS = "http://www.w3.org/2000/svg";

type Attrs = Record<string, string | number | boolean | null | undefined>;

function apply(e: Element, attrs: Attrs) {
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    e.setAttribute(k, v === true ? "" : String(v));
  }
}

export function h(tag: string, attrs: Attrs = {}, ...children: (Node | string | null | undefined)[]): HTMLElement {
  const e = document.createElement(tag);
  apply(e, attrs);
  for (const c of children) if (c !== null && c !== undefined) e.append(c);
  return e;
}

export function s(tag: string, attrs: Attrs = {}, ...children: (Node | string | null | undefined)[]): SVGElement {
  const e = document.createElementNS(SVG_NS, tag);
  apply(e, attrs);
  for (const c of children) if (c !== null && c !== undefined) e.append(c);
  return e;
}

/// A time on the store's axis (the plant's clock read as UTC), as the plant reads it.
export function clock(ms: number, seconds = true): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return seconds ? `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}` : `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}

/// A number for an axis or a cell: up to `places` places, trailing zeros dropped.
export function num(v: number | null | undefined, places = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  const t = v.toFixed(places).replace(/\.?0+$/, "");
  return t === "-0" ? "0" : t;
}

/// Seconds since an arrival on the unix clock, as words.
export function ago(sinceMs: number | null): string {
  if (sinceMs === null) return "no value";
  const s = Math.max(0, Math.round((Date.now() - sinceMs) / 1000));
  if (s < 90) return `${s} s ago`;
  const m = Math.round(s / 60);
  return m < 90 ? `${m} min ago` : `${Math.round(m / 60)} h ago`;
}
