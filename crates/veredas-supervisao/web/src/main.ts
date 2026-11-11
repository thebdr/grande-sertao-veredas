// Supervisão's page: the top bar (wordmark, session, views, headline, live clock), the layout
// with its side panel and alarm strip, the dashboards, the tables, the series, the alarms.

import "./style.css";
import { type BindingView, type ObjectState, type SessionEntry, type SessionStatus, base, getJson, openFeed } from "./api";
import { Dashboards } from "./dash";
import { ago, clock, h } from "./dom";
import { type LayoutView, mountLayout } from "./layout";
import { Side, badge } from "./side";
import { Series, Tables } from "./tables";

const app = document.getElementById("app")!;

async function boot() {
  const m = location.pathname.match(/^\/s\/([a-z0-9][a-z0-9-]*)\/?$/);
  const sessions = (await getJson<{ sessions: SessionEntry[] }>("/api/sessions")).sessions;
  if (!m) {
    if (sessions.length === 1) {
      location.replace(sessions[0].path);
      return;
    }
    app.replaceChildren(
      h("header", { class: "top" }, h("span", { class: "wordmark" }, "veredas ›"), h("span", { class: "plant" }, "Supervisão")),
      h("main", { class: "sessions" }, ...sessions.map((s) => h("a", { class: "session-card", href: s.path }, h("strong", {}, s.title), h("span", { class: "mono muted" }, s.path)))),
    );
    return;
  }
  await run(m[1], sessions);
}

async function run(name: string, sessions: SessionEntry[]) {
  const binding = await getJson<BindingView>(`${base(name)}/binding`);
  document.title = `${binding.title} · Supervisão`;
  const objs = new Map<string, ObjectState>();
  let status: SessionStatus | null = null;
  let connected = false;
  const events: { at: number; id: string; text: string; state: string }[] = [];

  // ---- the top bar --------------------------------------------------------------------------
  const picker = h("select", { class: "plant", title: "Session" }) as HTMLSelectElement;
  for (const s of sessions) picker.append(h("option", { value: s.path, selected: s.name === name }, s.title));
  picker.onchange = () => { location.href = picker.value; };
  const tabsEl = h("nav", { class: "tabs" });
  const headline = h("div", { class: "headline" });
  const liveEl = h("span", { class: "live mono" });
  app.replaceChildren(h("header", { class: "top" }, h("span", { class: "wordmark" }, "veredas ›"), picker, tabsEl, headline, liveEl));

  // ---- the views ----------------------------------------------------------------------------
  const main = h("main", { class: "view" });
  const strip = h("footer", { class: "strip" });
  app.append(main, strip);

  const side = new Side(name, binding.objects, (id) => select(id, null));
  let layout: LayoutView | null = null;
  const layers = { markers: true, states: true, labels: true };
  const layoutView = h("section", { class: "layout" });
  const plantPane = h("div", { class: "plant-pane" });
  layoutView.append(plantPane, side.root);
  if (binding.layout) {
    layout = await mountLayout(base(name), binding.objects, (id, key) => select(id || null, key));
    const toggles = h("div", { class: "layers" }, ...(["markers", "states", "labels"] as const).map((k) => {
      const b = h("button", { class: "on" }, k[0].toUpperCase() + k.slice(1));
      b.onclick = () => { layers[k] = !layers[k]; b.classList.toggle("on", layers[k]); layout!.layers(layers); };
      return b;
    }));
    const legend = h("div", { class: "legend" }, ...[["normal", "●", "normal"], ["near", "■", "near"], ["warn", "■", "warn"], ["alarm", "!", "alarm"], ["disabled", "▨", "disabled"], ["stale", "◌", "stale"]].map(([c, m, w]) => h("span", { class: `st-text-${c}` }, `${m} ${w}`)), h("span", { class: "sel" }, "◆ selected"));
    plantPane.append(layout.root, toggles, legend);
  } else {
    plantPane.append(h("p", { class: "muted pad" }, "This binding has no layout."));
  }
  const dash = new Dashboards(name, binding.dashboards);
  const tables = new Tables(name, () => status);
  const series = new Series(name, () => status);
  const alarms = h("section", { class: "alarms" });

  function select(id: string | null, key: string | null) {
    side.select(id, key);
    layout?.select(id, key);
    side.render(objs, status);
  }

  function renderAlarms() {
    const rows = binding.objects.filter((d) => objs.get(d.id) && objs.get(d.id)!.state !== "normal");
    alarms.replaceChildren(h("table", { class: "dt" }, h("thead", {}, h("tr", {}, ...["State", "Object", "Value", "Since"].map((x) => h("th", {}, x)))),
      h("tbody", {}, ...rows.map((d) => { const o = objs.get(d.id)!; return h("tr", {}, h("td", {}, badge(o)), h("td", {}, d.label), h("td", { class: "mono" }, o.text ?? "—"), h("td", { class: "mono" }, ago(o.since_ms))); }))));
  }

  // The top bar is built once and updated in place: an element replaced every second would
  // swallow a click whose press and release fell on either side of the swap.
  const tabs = [["layout", "Layout"], ["dashboards", "Dashboards"], ["tables", "Tables"], ["series", "Series"], ["alarms", "Alarms"]].map(([k, label]) => {
    const count = h("span", { class: "count" });
    const a = h("a", { href: `#${k}`, "data-view": k }, label, count);
    return { k, a, count };
  });
  tabsEl.replaceChildren(...tabs.map((t) => t.a));
  const figs = binding.headline.map((id) => {
    const d = binding.objects.find((x) => x.id === id)!;
    const value = h("span", { class: "mono st-text-stale" }, "—");
    headline.append(h("span", { class: "fig", "data-object": id }, h("span", { class: "muted" }, d.label), value, d.unit ? h("span", { class: "unit" }, d.unit) : null));
    return { id, value };
  });

  function renderTop() {
    const n = binding.objects.filter((d) => { const o = objs.get(d.id); return o && o.state !== "normal"; }).length;
    const view = (location.hash.replace(/^#/, "").split("/")[0] || "layout");
    for (const t of tabs) {
      t.a.className = view === t.k ? "on" : "";
      t.count.textContent = t.k === "alarms" && n ? String(n) : "";
      t.count.hidden = !(t.k === "alarms" && n);
    }
    for (const f of figs) {
      const o = objs.get(f.id);
      f.value.className = `mono st-text-${o?.state ?? "stale"}`;
      f.value.textContent = o?.text ?? "—";
    }
    liveEl.className = `live mono ${status?.running && connected ? "on" : "off"}`;
    liveEl.textContent = status ? `${status.running && connected ? "● live" : "○ stopped"} ${clock(status.now_ms)}` : "○ connecting";
  }

  function renderStrip() {
    const last = events.slice(-6).reverse();
    strip.replaceChildren(
      h("span", { class: "strip-items" }, ...(last.length ? last.map((e) => h("span", { class: `st-text-${e.state}` }, `${clock(e.at)} ${e.text}`)) : [h("span", { class: "muted" }, "no alarm since the page opened")])),
      h("span", { class: "mono muted" }, binding.layout ? `layout ${binding.layout}` : ""),
    );
  }

  function note(o: ObjectState) {
    const prev = objs.get(o.id);
    objs.set(o.id, o);
    if (prev && prev.state !== o.state && o.state !== "normal" && status) {
      const d = binding.objects.find((x) => x.id === o.id);
      events.push({ at: status.now_ms, id: o.id, text: `${d?.label ?? o.id} ${o.word}`, state: o.state });
      if (events.length > 100) events.shift();
    }
    layout?.update(o);
  }

  openFeed(name, {
    snapshot(all, st) { status = st; for (const o of all) note(o); renderAll(); },
    objects(changed) { for (const o of changed) note(o); renderAll(); },
    session(st) { status = st; renderTop(); side.render(objs, status); },
    connection(open) { connected = open; renderTop(); },
  });

  function renderAll() {
    renderTop();
    renderStrip();
    side.render(objs, status);
    if (main.firstChild === alarms) renderAlarms();
  }

  function route() {
    const [view, arg] = location.hash.replace(/^#/, "").split("/");
    dash.hide();
    switch (view || "layout") {
      case "dashboards": main.replaceChildren(dash.root); dash.show(Number(arg ?? 0)); break;
      case "tables": main.replaceChildren(tables.root); void tables.show(); break;
      case "series": main.replaceChildren(series.root); void series.show(); break;
      case "alarms": main.replaceChildren(alarms); renderAlarms(); break;
      default: main.replaceChildren(layoutView); layout?.shown();
    }
    renderTop();
  }
  window.addEventListener("hashchange", route);
  route();
  window.setInterval(() => { renderTop(); }, 1000);
}

boot().catch((e) => app.replaceChildren(h("p", { class: "error pad" }, String(e))));
