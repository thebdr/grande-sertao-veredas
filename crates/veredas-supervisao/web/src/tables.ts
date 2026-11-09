// The Tables and Series views: a session's store as portal's own REST serves it, nested at
// `/s/<name>/portal/`. Nothing here reaches past that router.

import { base, getJson, type SessionStatus } from "./api";
import { type Pt, trend } from "./charts";
import { h, num } from "./dom";

interface TableInfo {
  name: string;
  kind: string;
  columns: { name: string; type: string }[];
  rows: number | null;
}

interface Page {
  schema: { name: string; type: string }[];
  rows: unknown[][];
  total: number;
  truncated: boolean;
}

interface SeriesInfo {
  source: string;
  series: string;
  type: string;
  count: number;
  keys: number;
  last_ms: number;
  last_value: string;
}

const PAGE = 50;

function cell(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "number") return num(v, 3);
  return typeof v === "string" ? v : JSON.stringify(v);
}

export class Tables {
  readonly root = h("section", { class: "split" });
  private list = h("nav", { class: "list" });
  private main = h("div", { class: "list-main" });
  private table: string | null = null;
  private offset = 0;

  constructor(private name: string) {
    this.root.append(this.list, this.main);
  }

  async show() {
    try {
      const t = await getJson<{ tables: TableInfo[] }>(`${base(this.name)}/portal/api/tables`);
      this.list.replaceChildren(h("div", { class: "caps small" }, `Tables · ${t.tables.length}`), ...t.tables.map((x) => {
        const b = h("button", { class: x.name === this.table ? "on" : "", "data-table": x.name }, h("span", {}, x.name), h("span", { class: "mono muted" }, `${x.kind} · ${x.rows ?? "?"}`));
        b.onclick = () => { this.table = x.name; this.offset = 0; void this.show(); };
        return b;
      }));
      if (this.table) await this.page();
      else this.main.replaceChildren(h("p", { class: "muted" }, "Pick a table."));
    } catch (e) {
      this.main.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }

  private async page() {
    const q = new URLSearchParams({ name: this.table!, limit: String(PAGE), offset: String(this.offset) });
    const p = await getJson<Page>(`${base(this.name)}/portal/api/table/data?${q}`);
    const prev = h("button", { disabled: this.offset === 0 }, "‹ Newer") as HTMLButtonElement;
    const next = h("button", { disabled: this.offset + PAGE >= p.total }, "Older ›") as HTMLButtonElement;
    prev.onclick = () => { this.offset = Math.max(0, this.offset - PAGE); void this.page(); };
    next.onclick = () => { this.offset += PAGE; void this.page(); };
    const from = p.total === 0 ? 0 : this.offset + 1;
    this.main.replaceChildren(
      h("div", { class: "pager" }, h("strong", {}, this.table!), h("span", { class: "mono muted", "data-rows": `${from}-${this.offset + p.rows.length}/${p.total}` }, `rows ${from}–${this.offset + p.rows.length} of ${p.total}, newest first`), prev, next),
      h("div", { class: "scroll" }, h("table", { class: "dt" },
        h("thead", {}, h("tr", {}, ...p.schema.map((c) => h("th", {}, c.name, h("span", { class: "muted" }, ` ${c.type.toLowerCase()}`))))),
        h("tbody", {}, ...p.rows.map((r) => h("tr", {}, ...r.map((v) => h("td", { class: "mono" }, cell(v)))))))),
    );
  }
}

export class Series {
  readonly root = h("section", { class: "split" });
  private list = h("nav", { class: "list" });
  private main = h("div", { class: "list-main" });
  private pick: SeriesInfo | null = null;

  constructor(private name: string, private status: () => SessionStatus | null) {
    this.root.append(this.list, this.main);
  }

  async show() {
    try {
      const s = await getJson<{ series: SeriesInfo[] }>(`${base(this.name)}/portal/api/series`);
      this.list.replaceChildren(h("div", { class: "caps small" }, `Series · ${s.series.length}`), ...s.series.map((x) => {
        const on = this.pick && this.pick.source === x.source && this.pick.series === x.series;
        const b = h("button", { class: on ? "on" : "", "data-series": `${x.source}/${x.series}` }, h("span", {}, `${x.source} · ${x.series}`), h("span", { class: "mono muted" }, `${x.type} · ${x.count}`));
        b.onclick = () => { this.pick = x; void this.show(); };
        return b;
      }));
      if (this.pick) await this.chart(this.pick);
      else this.main.replaceChildren(h("p", { class: "muted" }, "Pick a series."));
    } catch (e) {
      this.main.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }

  private async chart(x: SeriesInfo) {
    const now = this.status()?.now_ms ?? x.last_ms;
    const from = now - 60 * 60_000;
    const q = new URLSearchParams({ source: x.source, series: x.series, from: String(from), to: String(now), points: "600" });
    const w = await getJson<{ points: Pt[]; total: number; downsampled: boolean }>(`${base(this.name)}/portal/api/series/data?${q}`);
    this.main.replaceChildren(
      h("div", { class: "pager" }, h("strong", {}, `${x.source} · ${x.series}`), h("span", { class: "mono muted", "data-points": `${w.points.length}/${w.total}` }, `last 60 min · ${w.points.length} of ${w.total} points${w.downsampled ? ", downsampled" : ""} · last ${x.last_value}`)),
      h("div", { class: "panel" }, trend(w.points, [], from, now, null, "")),
    );
  }
}
