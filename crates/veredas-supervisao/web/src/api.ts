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

/// What portal's answer to a write means: written (the plant took it); refused (by portal's switch
/// or whitelist, a value or a body it cannot read, or the plant's own status); not sent (503: the
/// plant never saw it); not confirmed (504: it may have reached the plant); no answer (portal's
/// answer never came whole, or came unreadable: the post may have left, and the write landed).
export type WriteSaid = "written" | "refused" | "not sent" | "not confirmed" | "no answer";

/// Why a write with no answer may still have landed: a request fails the same way whether it never
/// left or its answer was lost on the way back (a connection reset, a host restarted), and portal
/// does not take back a write it has handed to the plant.
const MAY_HAVE_LANDED = "the write may have reached the plant";

/// A write, posted to the session's portal surface; portal's switch and whitelist answer.
export async function postWrite(name: string, a: Action): Promise<{ said: WriteSaid; text: string }> {
  let r: Response;
  try {
    r = await fetch(`${base(name)}/portal/api/opc/write`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ source: a.source, tag: a.tag, value: a.value }),
    });
  } catch (e) {
    return { said: "no answer", text: `${MAY_HAVE_LANDED} (${String(e)})` };
  }
  // Portal's answers, by its own route: a 200 saying `ok` (the plant's own status beside it), or a
  // refusal `{ error }` under one of its codes. Anything else (a body that cannot be read, another
  // shape, another code: an answer of something between the page and portal) says nothing of the
  // write, which may have reached the plant.
  let body: unknown;
  try {
    body = await r.json();
  } catch (e) {
    return { said: "no answer", text: `${MAY_HAVE_LANDED} (${r.status}, its answer unreadable: ${String(e)})` };
  }
  const o = (typeof body === "object" && body !== null ? body : {}) as { ok?: unknown; status?: unknown; error?: unknown };
  if (r.status === 200 && typeof o.ok === "boolean") return { said: o.ok ? "written" : "refused", text: typeof o.status === "string" ? o.status : "no status" };
  if (PORTAL_REFUSALS.includes(r.status) && typeof o.error === "string") {
    const said: WriteSaid = r.status === 504 ? "not confirmed" : r.status === 503 ? "not sent" : "refused";
    return { said, text: `${r.status}: ${o.error}` };
  }
  return { said: "no answer", text: `${MAY_HAVE_LANDED} (${r.status}, an answer not portal's)` };
}

/// The codes portal refuses a write with, each with its `{ error }`: a value or a body it cannot read
/// (400), the switch or a tag it does not write (403), a body past its limit (413), the plant never
/// reached (503), no answer from the plant (504).
const PORTAL_REFUSALS = [400, 403, 413, 503, 504];
