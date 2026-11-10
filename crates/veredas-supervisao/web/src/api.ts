// What the host serves the page, typed. The page decides nothing: states, staleness, places and
// every panel's numbers arrive computed (src/feed.rs, src/panel.rs in the crate).

export type StateName = "normal" | "near" | "warn" | "alarm" | "disabled" | "stale";

export interface Action {
  label: string;
  source: string;
  tag: string;
  value: string;
}

export interface ObjectDef {
  id: string;
  label: string;
  unit: string;
  show: "state" | "level" | "value" | "move";
  range: [number, number];
  path: string | null;
  source: string;
  series: string;
  key: string | null;
  normal: string;
  rules: { state: StateName; word: string; when: string }[];
  actions: Action[];
}

export interface PanelHead {
  kind: string;
  title: string;
  span: number;
}

export interface BindingView {
  name: string;
  title: string;
  layout: string | null;
  headline: string[];
  objects: ObjectDef[];
  dashboards: { name: string; columns: number; panels: PanelHead[] }[];
}

export interface Item {
  key: string;
  place: number;
  /// The key's own value, as the object shows a number.
  text: string;
}

export interface ObjectState {
  id: string;
  value: number | string | null;
  text: string | null;
  state: StateName;
  mark: string;
  word: string;
  stale: boolean;
  since_ms: number | null;
  level: number | null;
  place: number | null;
  items: Item[];
}

export interface SessionStatus {
  running: boolean;
  writes_armed: boolean;
  health: Record<string, { state: string; detail: string | null }>;
  now_ms: number;
}

export interface SessionEntry {
  name: string;
  title: string;
  path: string;
}

export async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(url, { cache: "no-store" });
  const body = await r.json().catch(() => ({ error: `${r.status} ${r.statusText}` }));
  if (!r.ok) throw new Error((body as { error?: string }).error ?? `${r.status}`);
  return body as T;
}

/// The session's base path: `/s/<name>`.
export function base(name: string): string {
  return `/s/${name}`;
}

export interface FeedHandlers {
  snapshot(objects: ObjectState[], status: SessionStatus): void;
  objects(changed: ObjectState[]): void;
  session(status: SessionStatus): void;
  connection(open: boolean): void;
}

/// The object feed: a snapshot, then what changed; reconnects on its own (EventSource does).
export function openFeed(name: string, h: FeedHandlers): EventSource {
  const es = new EventSource(`${base(name)}/objects`);
  es.addEventListener("snapshot", (e) => {
    const v = JSON.parse((e as MessageEvent).data);
    h.snapshot(v.objects, v.session);
  });
  es.addEventListener("objects", (e) => h.objects(JSON.parse((e as MessageEvent).data).objects));
  es.addEventListener("session", (e) => h.session(JSON.parse((e as MessageEvent).data)));
  es.onopen = () => h.connection(true);
  es.onerror = () => h.connection(false);
  return es;
}

/// A write, posted to the session's portal surface; portal's switch and whitelist answer.
export async function postWrite(name: string, a: Action): Promise<{ ok: boolean; text: string }> {
  const r = await fetch(`${base(name)}/portal/api/opc/write`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ source: a.source, tag: a.tag, value: a.value }),
  });
  const body = await r.json().catch(() => ({}));
  if (r.ok) return { ok: Boolean((body as { ok?: boolean }).ok), text: (body as { status?: string }).status ?? "written" };
  return { ok: false, text: `${r.status}: ${(body as { error?: string }).error ?? r.statusText}` };
}
