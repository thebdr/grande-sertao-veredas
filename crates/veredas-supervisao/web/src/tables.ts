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
  /// The window paged through, frozen at the session's now when the view or a table is shown: a
  /// row stamped later joins no page until the view is shown again. Portal holds the window by
  /// time, so a row stamped inside it but stored late (a late line, the tail's lag) still joins it.
  /// Shown before the page holds any status (no now yet), the window is held at the first page's
  /// newest row, the latest time it has.
  private to: number | null = null;
  /// The latest request: an answer to an earlier one is not drawn.
  private asked = 0;

  constructor(private name: string, private status: () => SessionStatus | null) {
    this.root.append(this.list, this.main);
  }

  async show() {
    const token = ++this.asked;
    this.to = this.status()?.now_ms ?? null;
    // The pager of the page shown is for the table it was drawn for: off from a pick on.
    for (const b of Array.from(this.main.querySelectorAll(".pager button"))) (b as HTMLButtonElement).disabled = true;
    try {
      const t = await getJson<{ tables: TableInfo[] }>(`${base(this.name)}/portal/api/tables`);
      if (token !== this.asked) return;
      this.list.replaceChildren(h("div", { class: "caps small" }, `Tables · ${t.tables.length}`), ...t.tables.map((x) => {
        const b = h("button", { class: x.name === this.table ? "on" : "", "data-table": x.name }, h("span", {}, x.name), h("span", { class: "mono muted" }, `${x.kind} · ${x.rows ?? "?"}`));
        b.onclick = () => { this.table = x.name; this.offset = 0; void this.show(); };
        return b;
      }));
      if (this.table) await this.page();
      else this.main.replaceChildren(h("p", { class: "muted" }, "Pick a table."));
    } catch (e) {
      // A listing refused for a pick no longer the latest says nothing over the latest's page.
      if (token === this.asked) this.main.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }

  private async page() {
    // What this request asks, fixed now: a click while it is out changes nothing it reads.
    const table = this.table!, offset = this.offset, to = this.to, token = ++this.asked;
    for (const b of Array.from(this.main.querySelectorAll(".pager button"))) (b as HTMLButtonElement).disabled = true;
    try {
      const q = new URLSearchParams({ name: table, limit: String(PAGE), offset: String(offset) });
      if (to !== null) q.set("to", String(to));
      const p = await getJson<Page>(`${base(this.name)}/portal/api/table/data?${q}`);
      if (token !== this.asked) return;
      // Portal orders a table newest first by its time (`index_ms`, else `ts`) and holds its window
      // by it; one with neither comes in the store's own order, unheld, and is said to be unordered.
      const ordered = p.schema.some((c) => c.name === "index_ms" || c.name === "ts");
      // No now when it was asked: the window held from here at the newest row's time (portal's own
      // order: `index_ms`, else `ts`), so a row stored since joins no page.
      const at = p.schema.findIndex((c) => c.name === "index_ms");
      const time = at >= 0 ? at : p.schema.findIndex((c) => c.name === "ts");
      if (to === null && this.to === null && this.table === table && time >= 0 && typeof p.rows[0]?.[time] === "number") this.to = p.rows[0][time] as number;
      const prev = h("button", { disabled: offset === 0 }, "‹ Newer") as HTMLButtonElement;
      const next = h("button", { disabled: offset + PAGE >= p.total }, "Older ›") as HTMLButtonElement;
      prev.onclick = () => { if (this.table === table) { this.offset = Math.max(0, offset - PAGE); void this.page(); } };
      next.onclick = () => { if (this.table === table && offset + PAGE < p.total) { this.offset = offset + PAGE; void this.page(); } };
      const from = p.total === 0 ? 0 : offset + 1;
      this.main.replaceChildren(
        h("div", { class: "pager" }, h("strong", {}, table), h("span", { class: "mono muted", "data-rows": `${from}-${offset + p.rows.length}/${p.total}` }, `rows ${from}–${offset + p.rows.length} of ${p.total}${ordered ? ", newest first" : ", unordered"}`), prev, next),
        h("div", { class: "scroll" }, h("table", { class: "dt" },
          h("thead", {}, h("tr", {}, ...p.schema.map((c) => h("th", {}, c.name, h("span", { class: "muted" }, ` ${c.type.toLowerCase()}`))))),
          h("tbody", {}, ...p.rows.map((r) => h("tr", {}, ...r.map((v) => h("td", { class: "mono" }, cell(v)))))))),
      );
    } catch (e) {
      if (token === this.asked) this.main.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }
}

export class Series {
  readonly root = h("section", { class: "split" });
  private list = h("nav", { class: "list" });
  private main = h("div", { class: "list-main" });
  private pick: SeriesInfo | null = null;
  /// The latest request: an answer to an earlier one is not drawn.
  private asked = 0;

  constructor(private name: string, private status: () => SessionStatus | null) {
    this.root.append(this.list, this.main);
  }

  async show() {
    const token = ++this.asked;
    try {
      const s = await getJson<{ series: SeriesInfo[] }>(`${base(this.name)}/portal/api/series`);
      if (token !== this.asked) return;
      // The pick read again from the listing just fetched, so its heading's last value is this one's.
      const was = this.pick;
      if (was) this.pick = s.series.find((y) => y.source === was.source && y.series === was.series) ?? was;
      this.list.replaceChildren(h("div", { class: "caps small" }, `Series · ${s.series.length}`), ...s.series.map((x) => {
        const on = this.pick && this.pick.source === x.source && this.pick.series === x.series;
        const b = h("button", { class: on ? "on" : "", "data-series": `${x.source}/${x.series}` }, h("span", {}, `${x.source} · ${x.series}`), h("span", { class: "mono muted" }, `${x.type} · ${x.count}`));
        b.onclick = () => { this.pick = x; void this.show(); };
        return b;
      }));
      if (this.pick) await this.chart(this.pick, token);
      else this.main.replaceChildren(h("p", { class: "muted" }, "Pick a series."));
    } catch (e) {
      if (token === this.asked) this.main.replaceChildren(h("p", { class: "error" }, String(e)));
    }
  }

  private async chart(x: SeriesInfo, token: number) {
    const now = this.status()?.now_ms ?? x.last_ms;
    const from = now - 60 * 60_000;
    const q = new URLSearchParams({ source: x.source, series: x.series, from: String(from), to: String(now), points: "600" });
    const w = await getJson<{ points: Pt[]; total: number; downsampled: boolean }>(`${base(this.name)}/portal/api/series/data?${q}`);
    if (token !== this.asked) return;
    this.main.replaceChildren(
      h("div", { class: "pager" }, h("strong", {}, `${x.source} · ${x.series}`), h("span", { class: "mono muted", "data-points": `${w.points.length}/${w.total}` }, `last 60 min · ${w.points.length} of ${w.total} points${w.downsampled ? ", downsampled" : ""} · last ${x.last_value}`)),
      h("div", { class: "panel" }, trend(w.points, [], from, now, null, "")),
    );
  }
}
