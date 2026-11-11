// The dashboards: every panel drawn from the numbers the host computed for it
// (`/s/<name>/dashboard/<index>`), refreshed every five seconds.

import { base, getJson } from "./api";
import { bars, type Bucket, distribution, type Pt, type Segment, sparkline, stateBar, trend } from "./charts";
import { clock, h, num } from "./dom";

interface Tile {
  id: string;
  label: string;
  unit: string;
  text: string | null;
  state: string;
  mark: string;
  word: string;
  stale: boolean;
  since_ms?: number | null;
  level?: number | null;
  fraction?: number | null;
  ticks?: number[];
  sub?: string;
  target?: number | null;
  spark?: Pt[];
}

interface PanelOut {
  kind: string;
  title: string;
  span: number;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data?: any;
  error?: string;
}

interface DashOut {
  name: string;
  columns: number;
  to: number;
  panels: PanelOut[];
}

function head(title: string, right?: Node | string): HTMLElement {
  return h("div", { class: "panel-head" }, h("span", { class: "panel-title" }, title), right ? h("span", { class: "panel-bind" }, right) : null);
}

function valueTile(p: PanelOut): HTMLElement {
  const t = p.data as Tile;
  const word = t.state === "normal" ? null : h("span", { class: `badge st-${t.state}` }, `${t.mark} ${t.word.toUpperCase()}`);
  return h("div", { class: `panel tile st-box-${t.state}` },
    head(p.title, word ?? h("span", { class: "mono muted" }, t.id)),
    h("div", { class: `big mono st-text-${t.state}` }, t.text ?? "—", h("span", { class: "unit" }, t.unit)),
    h("div", { class: `sub st-text-${t.state}` }, t.sub ?? ""),
    sparkline(t.spark ?? [], 260, 30, `spark st-line-${t.state}`),
  );
}

function trendPanel(p: PanelOut): HTMLElement {
  const d = p.data;
  const limit = d.limit ? (d.limit.above ?? d.limit.below) : null;
  const right = d.limit ? `${d.limit.above !== undefined ? "max" : "min"} ${num(limit)}${d.unit ? " " + d.unit : ""} · ${d.breaches.length} breach${d.breaches.length === 1 ? "" : "es"}` : `${d.total} points`;
  return h("div", { class: "panel" }, head(p.title, right), trend(d.points, d.breaches, d.from, d.to, limit, d.unit));
}

function barsPanel(p: PanelOut): HTMLElement {
  const d = p.data;
  return h("div", { class: "panel" }, head(p.title, d.target !== null ? `target ${num(d.target)}${d.unit ? " " + d.unit : ""}` : undefined), bars(d.buckets as Bucket[], d.target));
}

function timelinePanel(p: PanelOut): HTMLElement {
  const d = p.data;
  const rows = (d.rows as { id: string; label: string; segments: Segment[]; now: Tile }[]).map((r) =>
    h("div", { class: "tl-row" },
      h("span", { class: "tl-label" }, r.label),
      h("span", { class: `tl-now st-text-${r.now.state}` }, `${r.now.mark} ${r.now.word}`),
      stateBar(r.segments, d.from, d.to)),
  );
  return h("div", { class: "panel" }, head(p.title), ...rows, h("div", { class: "tl-axis mono muted" }, h("span", {}, clock(d.from, false)), h("span", {}, "now")));
}

function gridPanel(p: PanelOut): HTMLElement {
  const cells = (p.data.cells as Tile[]).map((c) =>
    h("div", { class: `cell st-box-${c.state}`, title: `${c.label}: ${c.word}` },
      h("span", { class: "cell-id" }, c.label),
      h("span", { class: `cell-v mono st-text-${c.state}` }, c.state === "normal" || c.state === "near" ? (c.text ?? "—") : c.word)),
  );
  return h("div", { class: "panel" }, head(p.title, `${cells.length} · ${p.data.not_normal} not normal`), h("div", { class: "grid-cells" }, ...cells));
}

function levelsPanel(p: PanelOut): HTMLElement {
  const rows = (p.data.rows as Tile[]).map((r) => {
    const track = h("div", { class: "lv-track" }, h("div", { class: `lv-fill st-fill-${r.state}`, style: `width:${((r.fraction ?? 0) * 100).toFixed(1)}%` }));
    for (const t of r.ticks ?? []) track.append(h("div", { class: "lv-tick", style: `left:${(t * 100).toFixed(1)}%` }));
    return h("div", { class: "lv-row" }, h("span", { class: "lv-label" }, r.label), track, h("span", { class: `lv-v mono st-text-${r.state}` }, r.state === "normal" || r.state === "near" ? `${r.text ?? "—"}${r.unit ? " " + r.unit : ""}` : r.word));
  });
  return h("div", { class: "panel" }, head(p.title), ...rows);
}

function tablePanel(p: PanelOut): HTMLElement {
  const d = p.data;
  const thead = h("tr", {}, ...(d.columns as { label: string }[]).map((c) => h("th", {}, c.label)));
  const body = (d.rows as { text: string; bar?: number; state?: string; mark?: string; word?: string }[][]).map((r) =>
    h("tr", {}, ...r.map((c) => {
      if (c.bar !== undefined) return h("td", { class: "mono" }, h("div", { class: "inbar" }, h("div", { class: "inbar-fill", style: `width:${(c.bar * 100).toFixed(1)}%` }), h("span", {}, c.text)));
      if (c.state !== undefined) return h("td", { class: `mono st-text-${c.state}` }, c.word ? `${c.mark} ${c.word}` : c.text);
      return h("td", { class: "mono" }, c.text);
    })),
  );
  return h("div", { class: "panel" }, head(p.title, `${d.total} rows`), h("table", { class: "dt" }, h("thead", {}, thead), h("tbody", {}, ...body)));
}

function cellsPanel(p: PanelOut): HTMLElement {
  const d = p.data;
  const rows = (d.rows as { key: string; n: number; median: number; sigma: number | null; spark: [number, number | null][]; bins: number[] }[]).map((r) =>
    h("tr", {},
      h("td", { class: "mono" }, r.key),
      h("td", { class: "mono" }, num(r.median, 1)),
      h("td", {}, sparkline(r.spark.filter((x) => x[1] !== null) as Pt[], 120, 24)),
      h("td", {}, distribution(r.bins)),
      h("td", { class: "mono" }, num(r.sigma, 1)),
      h("td", { class: "mono" }, String(r.n))),
  );
  const note = `last ${Math.round((d.to - d.from) / 60000)} min · distribution ±${num(d.spread)} around each median, same scale on every row${d.capped ? ` · newest ${d.read} of ${d.total} rows` : ""}`;
  return h("div", { class: "panel" }, head(p.title, note),
    h("table", { class: "dt" }, h("thead", {}, h("tr", {}, ...["Key", "Median", "Median, window", "Distribution", "σ", "n"].map((x) => h("th", {}, x)))), h("tbody", {}, ...rows)));
}

const DRAW: Record<string, (p: PanelOut) => HTMLElement> = {
  value: valueTile,
  trend: trendPanel,
  bars: barsPanel,
  timeline: timelinePanel,
  grid: gridPanel,
  levels: levelsPanel,
  table: tablePanel,
  cells: cellsPanel,
};

export class Dashboards {
  readonly root = h("section", { class: "dashboards" });
  private current = 0;
  private timer: number | undefined;
  private body = h("div", { class: "dash-grid" });
  private tabs = h("div", { class: "subtabs" });

  constructor(private name: string, private dashboards: { name: string }[]) {
    this.root.append(this.tabs, this.body);
  }

  show(index: number) {
    this.current = Math.min(Math.max(index, 0), Math.max(this.dashboards.length - 1, 0));
    this.tabs.replaceChildren(...this.dashboards.map((d, i) => {
      const b = h("button", { class: i === this.current ? "on" : "" }, d.name);
      b.onclick = () => { location.hash = `#dashboards/${i}`; };
      return b;
    }));
    if (this.dashboards.length === 0) {
      this.body.replaceChildren(h("p", { class: "muted" }, "This binding has no dashboard."));
      return;
    }
    void this.refresh();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => void this.refresh(), 5000);
  }

  hide() {
    window.clearInterval(this.timer);
  }

  async refresh() {
    try {
      const d = await getJson<DashOut>(`${base(this.name)}/dashboard/${this.current}`);
      this.body.style.gridTemplateColumns = `repeat(${d.columns}, minmax(0, 1fr))`;
      this.body.replaceChildren(...d.panels.map((p) => {
        const el = p.error !== undefined ? h("div", { class: "panel" }, head(p.title, p.kind), h("p", { class: "error" }, p.error)) : (DRAW[p.kind] ?? (() => h("div", { class: "panel" }, head(p.title, p.kind))))(p);
        el.style.gridColumn = `span ${p.span}`;
        el.dataset.kind = p.kind;
        return el;
      }));
    } catch (e) {
      this.body.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }
}
