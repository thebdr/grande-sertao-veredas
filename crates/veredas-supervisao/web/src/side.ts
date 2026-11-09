// The side panel: the selected object (its state, value, arrival, a 30-minute sparkline, the
// writes it offers, shown gated and disabled while the session's switch is off), and below it
// everything not normal.

import { type Action, type ObjectDef, type ObjectState, type SessionStatus, base, getJson, postWrite } from "./api";
import { sparkline, type Pt } from "./charts";
import { ago, h } from "./dom";

const WEIGHT: Record<string, number> = { alarm: 0, warn: 1, near: 2, disabled: 3, stale: 4, normal: 5 };

export function badge(o: ObjectState | undefined): HTMLElement {
  if (!o) return h("span", { class: "badge st-stale" }, "◌ no value");
  return h("span", { class: `badge st-${o.state}` }, `${o.mark} ${o.word}`);
}

export class Side {
  readonly root = h("aside", { class: "side" });
  private sel: { id: string; key: string | null } | null = null;
  private sparkAt = 0;
  private sparkFor = "";
  private sparkEl: HTMLElement = h("div", { class: "spark-wrap" });
  private note = "";

  constructor(
    private name: string,
    private defs: ObjectDef[],
    private onPick: (id: string) => void,
  ) {}

  select(id: string | null, key: string | null) {
    this.sel = id ? { id, key } : null;
    this.sparkFor = "";
    this.note = "";
  }

  selected(): string | null {
    return this.sel?.id ?? null;
  }

  private async loadSpark(def: ObjectDef, status: SessionStatus) {
    const key = this.sel?.key ?? def.key;
    const q = new URLSearchParams({ source: def.source, series: def.series, from: String(status.now_ms - 30 * 60_000), to: String(status.now_ms), points: "120" });
    if (key) q.set("key", key);
    try {
      const w = await getJson<{ points: Pt[] }>(`${base(this.name)}/portal/api/series/data?${q}`);
      this.sparkEl.replaceChildren(w.points.length > 1 ? sparkline(w.points, 260, 44) : h("div", { class: "muted" }, "nothing in the last 30 min"));
    } catch (e) {
      this.sparkEl.replaceChildren(h("div", { class: "muted" }, String(e)));
    }
  }

  private actionButton(a: Action, armed: boolean): HTMLElement {
    const b = h("button", { class: "gated", disabled: !armed, title: armed ? "Writes through portal" : "Portal's write switch is off" }, a.label) as HTMLButtonElement;
    b.onclick = async () => {
      if (!confirm(`Write ${a.value} to ${a.tag} of ${a.source}?`)) return;
      b.disabled = true;
      const r = await postWrite(this.name, a);
      this.note = r.ok ? `${a.label}: written (${r.text})` : `${a.label}: refused, ${r.text}`;
      b.disabled = !armed;
    };
    return b;
  }

  render(objs: Map<string, ObjectState>, status: SessionStatus | null) {
    const parts: Node[] = [];
    const def = this.sel ? this.defs.find((d) => d.id === this.sel!.id) : undefined;
    if (def && status) {
      const o = objs.get(def.id);
      const key = this.sel?.key;
      parts.push(
        h("div", { class: "side-head" }, h("span", { class: "caps" }, "Selected")),
        h("div", { class: "sel-title" }, h("strong", {}, key ? `${def.label} · ${key}` : def.label), badge(o)),
        h("dl", { class: "facts" },
          h("dt", {}, "Value"), h("dd", { class: o?.stale ? "mono stale" : "mono" }, o?.text !== null && o?.text !== undefined ? `${o.text}${def.unit ? " " + def.unit : ""}` : "—"),
          h("dt", {}, "Arrived"), h("dd", { class: "mono" }, ago(o?.since_ms ?? null)),
          h("dt", {}, "Series"), h("dd", { class: "mono" }, `${def.source} · ${def.series}${def.key ? " · " + def.key : ""}`),
          ...(def.rules.length ? [h("dt", {}, "Rules"), h("dd", {}, ...def.rules.map((r) => h("div", { class: "rule" }, h("span", { class: `badge st-${r.state}` }, r.word), ` when ${r.when}`)))] : []),
        ),
        h("div", { class: "caps small" }, "Last 30 min"),
        this.sparkEl,
      );
      const sparkKey = `${def.id}|${key ?? ""}`;
      if (this.sparkFor !== sparkKey || Date.now() - this.sparkAt > 10_000) {
        this.sparkFor = sparkKey;
        this.sparkAt = Date.now();
        void this.loadSpark(def, status);
      }
      if (def.actions.length) {
        parts.push(h("div", { class: "actions" }, ...def.actions.map((a) => this.actionButton(a, status.writes_armed))));
        parts.push(h("p", { class: "gate-note" }, status.writes_armed ? "Writes go through portal. Its write switch is on." : "Writes go through portal. Its write switch is off."));
        if (this.note) parts.push(h("p", { class: "write-note mono" }, this.note));
      }
    } else {
      parts.push(h("div", { class: "side-head" }, h("span", { class: "caps" }, "Selected")), h("p", { class: "muted" }, "Pick an object on the layout."));
    }
    const attention = this.defs
      .map((d) => ({ d, o: objs.get(d.id) }))
      .filter((x) => x.o && x.o.state !== "normal")
      .sort((a, b) => WEIGHT[a.o!.state] - WEIGHT[b.o!.state]);
    const list = h("ul", { class: "attention" });
    for (const { d, o } of attention) {
      const li = h("li", { class: this.sel?.id === d.id ? "on" : "" }, h("span", { class: `mark st-${o!.state}` }, o!.mark), h("span", { class: "grow" }, `${d.label} ${o!.word}`), h("span", { class: "mono muted" }, ago(o!.since_ms)));
      li.onclick = () => this.onPick(d.id);
      list.append(li);
    }
    parts.push(h("div", { class: "side-head" }, h("span", { class: "caps" }, `Needs attention · ${attention.length}`)), list);
    this.root.replaceChildren(...parts);
  }
}
