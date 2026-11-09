// The side panel: the selected object (its state, value, since when, a 30-minute sparkline, the
// writes it offers, shown gated and disabled while the session's switch is off), and below it
// everything not normal. Built when the selection changes and updated in place on every feed
// event: an element replaced between a press and its release would swallow the click.

import { type Action, type ObjectDef, type ObjectState, type SessionStatus, base, getJson, postWrite } from "./api";
import { sparkline, type Pt } from "./charts";
import { ago, h } from "./dom";

const WEIGHT: Record<string, number> = { alarm: 0, warn: 1, near: 2, disabled: 3, stale: 4, normal: 5 };

export function badge(o: ObjectState | undefined): HTMLElement {
  if (!o) return h("span", { class: "badge st-stale" }, "◌ no value");
  return h("span", { class: `badge st-${o.state}` }, `${o.mark} ${o.word}`);
}

function setBadge(el: HTMLElement, o: ObjectState | undefined) {
  el.className = `badge st-${o ? o.state : "stale"}`;
  el.textContent = o ? `${o.mark} ${o.word}` : "◌ no value";
}

interface Selected {
  def: ObjectDef;
  key: string | null;
  badge: HTMLElement;
  value: HTMLElement;
  since: HTMLElement;
  spark: HTMLElement;
  buttons: HTMLButtonElement[];
  gate: HTMLElement;
  note: HTMLElement;
  sparkAt: number;
}

export class Side {
  readonly root = h("aside", { class: "side" });
  private top = h("div", { class: "side-top" });
  private attentionHead = h("span", { class: "caps" });
  private list = h("ul", { class: "attention" });
  private listKey = "";
  private sel: Selected | null = null;
  private inState = new Map<string, { state: string; at: number }>();

  constructor(
    private name: string,
    private defs: ObjectDef[],
    private onPick: (id: string) => void,
  ) {
    this.root.append(this.top, h("div", { class: "side-head" }, this.attentionHead), this.list);
    this.select(null, null);
  }

  selected(): string | null {
    return this.sel?.def.id ?? null;
  }

  /// Build the selected object's part, once per selection.
  select(id: string | null, key: string | null) {
    const def = id ? this.defs.find((d) => d.id === id) : undefined;
    if (!def) {
      this.sel = null;
      this.top.replaceChildren(h("div", { class: "side-head" }, h("span", { class: "caps" }, "Selected")), h("p", { class: "muted" }, "Pick an object on the layout."));
      return;
    }
    const s: Selected = {
      def,
      key,
      badge: h("span", { class: "badge" }),
      value: h("dd", { class: "mono" }),
      since: h("dd", { class: "mono" }),
      spark: h("div", { class: "spark-wrap" }),
      buttons: def.actions.map((a) => this.actionButton(a)),
      gate: h("p", { class: "gate-note" }),
      note: h("p", { class: "write-note mono" }),
      sparkAt: 0,
    };
    this.sel = s;
    const parts: Node[] = [
      h("div", { class: "side-head" }, h("span", { class: "caps" }, "Selected")),
      h("div", { class: "sel-title" }, h("strong", {}, key ? `${def.label} · ${key}` : def.label), s.badge),
      h("dl", { class: "facts" },
        h("dt", {}, "Value"), s.value,
        h("dt", {}, "Since"), s.since,
        h("dt", {}, "Series"), h("dd", { class: "mono" }, `${def.source} · ${def.series}${def.key ? " · " + def.key : ""}`),
        ...(def.rules.length ? [h("dt", {}, "Rules"), h("dd", {}, ...def.rules.map((r) => h("div", { class: "rule" }, h("span", { class: `badge st-${r.state}` }, r.word), ` when ${r.when}`)))] : []),
      ),
      h("div", { class: "caps small" }, "Last 30 min"),
      s.spark,
    ];
    if (def.actions.length) parts.push(h("div", { class: "actions" }, ...s.buttons), s.gate, s.note);
    this.top.replaceChildren(...parts);
  }

  private actionButton(a: Action): HTMLButtonElement {
    const b = h("button", { class: "gated", disabled: true, "data-action": a.label }, a.label) as HTMLButtonElement;
    b.onclick = async () => {
      if (!confirm(`Write ${a.value} to ${a.tag} of ${a.source}?`)) return;
      b.dataset.busy = "1";
      b.disabled = true;
      const r = await postWrite(this.name, a);
      if (this.sel) this.sel.note.textContent = r.ok ? `${a.label}: written (${r.text})` : `${a.label}: refused, ${r.text}`;
      delete b.dataset.busy;
    };
    return b;
  }

  private async loadSpark(s: Selected, status: SessionStatus) {
    const key = s.key ?? s.def.key;
    const q = new URLSearchParams({ source: s.def.source, series: s.def.series, from: String(status.now_ms - 30 * 60_000), to: String(status.now_ms), points: "120" });
    if (key) q.set("key", key);
    try {
      const w = await getJson<{ points: Pt[] }>(`${base(this.name)}/portal/api/series/data?${q}`);
      s.spark.replaceChildren(w.points.length > 1 ? sparkline(w.points, 260, 44) : h("div", { class: "muted" }, "nothing in the last 30 min"));
    } catch (e) {
      s.spark.replaceChildren(h("div", { class: "muted" }, String(e)));
    }
  }

  /// Update in place: the selected object's state, value, since, the switch; the attention list
  /// only when what it lists changed.
  render(objs: Map<string, ObjectState>, status: SessionStatus | null) {
    const s = this.sel;
    if (s && status) {
      const o = objs.get(s.def.id);
      setBadge(s.badge, o);
      s.value.className = o?.stale ? "mono stale" : "mono";
      s.value.textContent = o?.text !== null && o?.text !== undefined ? `${o.text}${s.def.unit ? " " + s.def.unit : ""}` : "—";
      s.since.textContent = ago(o?.since_ms ?? null);
      if (Date.now() - s.sparkAt > 10_000) {
        s.sparkAt = Date.now();
        void this.loadSpark(s, status);
      }
      for (const b of s.buttons) if (!b.dataset.busy) b.disabled = !status.writes_armed;
      s.gate.textContent = status.writes_armed ? "Writes go through portal. Its write switch is on." : "Writes go through portal. Its write switch is off.";
    }
    const attention = this.defs
      .map((d) => ({ d, o: objs.get(d.id) }))
      .filter((x) => x.o && x.o.state !== "normal")
      .sort((a, b) => WEIGHT[a.o!.state] - WEIGHT[b.o!.state]);
    this.attentionHead.textContent = `Needs attention · ${attention.length}`;
    // Since when each object reads as it does: when the page first saw it so (the feed's own
    // arrival, on the first snapshot).
    for (const { d, o } of attention) {
      const seen = this.inState.get(d.id);
      if (!seen || seen.state !== `${o!.state}:${o!.word}`) this.inState.set(d.id, { state: `${o!.state}:${o!.word}`, at: seen ? Date.now() : (o!.since_ms ?? Date.now()) });
    }
    for (const id of Array.from(this.inState.keys())) if (!attention.some((x) => x.d.id === id)) this.inState.delete(id);
    const key = attention.map(({ d, o }) => `${d.id}:${o!.state}:${o!.word}`).join("|") + `#${s?.def.id ?? ""}`;
    if (key !== this.listKey) {
      this.listKey = key;
      this.list.replaceChildren(...attention.map(({ d, o }) => {
        const li = h("li", { class: s?.def.id === d.id ? "on" : "", "data-id": d.id }, h("span", { class: `mark st-${o!.state}` }, o!.mark), h("span", { class: "grow" }, `${d.label} ${o!.word}`), h("span", { class: "mono muted since", "data-since": String(this.inState.get(d.id)?.at ?? "") }));
        li.onclick = () => this.onPick(d.id);
        return li;
      }));
    }
    for (const el of Array.from(this.list.querySelectorAll<HTMLElement>(".since"))) el.textContent = ago(el.dataset.since ? Number(el.dataset.since) : null);
  }
}
