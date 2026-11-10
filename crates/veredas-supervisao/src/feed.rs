//! The object feed: a session's objects live, evaluated in Rust from portal's frames and caches,
//! so the page decides nothing and every state is tested exactly.
//!
//! Numbers arrive in the session's data frames, one sample at a time with its key; text arrives
//! only through the series cache the ingest refreshes (portal broadcasts numeric samples alone),
//! which also seeds a series the feed has not seen in a frame. Staleness is judged on the arrival
//! clock: an object is stale while its source is not live, or when it names a freshness bound and
//! no sample of its series arrived within it. A quiet plant storing on change is not stale.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::{Value as Json, json};
use tokio::sync::broadcast;
use veredas_core::{SeriesType, now_unix_ms};
use veredas_portal::session::{HealthState, Live};

use crate::binding::{Binding, KEY_FORGET_MS, Object, SeriesBind, Show};
use crate::state::{Reading, StateClass, format_number};

/// How often the feed evaluates its objects and publishes what changed.
pub const TICK: Duration = Duration::from_millis(200);
/// How often the session's status is published even when nothing in it changed (the clock).
pub const STATUS_EVERY: Duration = Duration::from_secs(1);

/// One sample as the feed keeps it.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    reading: Reading,
    /// Its index on the store's axis.
    index_ms: i64,
    /// When the feed saw it, on the unix clock.
    arrived_ms: i64,
    /// The ingest's count of the series with this sample: under one stamp, the higher is the later.
    n: u64,
}

/// What the feed knows of one series: its latest sample whatever its key, per key, and whether
/// its samples carry keys (a frame said so, or the ingest's cache counted keys).
#[derive(Clone, Debug, Default)]
struct SeriesSeen {
    any: Option<Seen>,
    by_key: BTreeMap<String, Seen>,
    keyed: bool,
}

/// Whether a sample at `index_ms` with the ingest's count `n` comes after one held: a newer stamp,
/// or the same stamp with a higher count (a later line under it, never the same sample again).
fn after(held: &Seen, index_ms: i64, n: u64) -> bool {
    index_ms > held.index_ms || (index_ms == held.index_ms && n > held.n)
}

/// A marker of a keyed series: its key and its place along the path, 0 to 1.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Item {
    pub key: String,
    pub place: f64,
    /// The key's own value, as the object shows a number.
    pub text: String,
}

/// One object as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectView {
    pub id: String,
    pub value: Option<Reading>,
    /// The value as shown, without its unit.
    pub text: Option<String>,
    pub state: StateClass,
    pub word: String,
    pub stale: bool,
    /// When the value arrived, on the unix clock.
    pub since_ms: Option<i64>,
    /// A level's fill, 0 to 1.
    pub level: Option<f64>,
    /// A marker's place along its path, 0 to 1, for a series without keys.
    pub place: Option<f64>,
    /// The markers of a keyed series, by key.
    pub items: Vec<Item>,
}

impl ObjectView {
    /// Whether two views show the same: everything but the arrival, which a source polled on
    /// an interval refreshes with every sample. A change is published; an arrival alone rides the
    /// next change and the snapshot. Compared as the page receives them, so a value stuck at NaN
    /// (no number, never equal to itself) is the same as itself.
    pub fn same(&self, other: &ObjectView) -> bool {
        let shown = |v: &ObjectView| {
            let mut j = v.to_json();
            if let Some(m) = j.as_object_mut() {
                m.remove("since_ms");
            }
            j
        };
        shown(self) == shown(other)
    }

    pub fn to_json(&self) -> Json {
        json!({
            "id": self.id,
            "value": match &self.value { Some(Reading::Num(v)) => json!(v), Some(Reading::Text(t)) => json!(t), None => Json::Null },
            "text": self.text,
            "state": self.state.name(),
            "mark": self.state.mark(),
            "word": self.word,
            "stale": self.stale,
            "since_ms": self.since_ms,
            "level": self.level,
            "place": self.place,
            "items": self.items,
        })
    }
}

/// A source's health as the page reads it: a label and, where it has one, its detail.
pub fn health_label(state: &HealthState) -> (&'static str, Option<String>) {
    match state {
        HealthState::Waiting => ("waiting", None),
        HealthState::Live => ("live", None),
        HealthState::Idle { silence_ms } => ("idle", Some(format!("silent for {} s", silence_ms / 1000))),
        HealthState::Retrying(why) => ("retrying", Some(why.clone())),
        HealthState::Ended => ("ended", None),
        HealthState::Failed(why) => ("failed", Some(why.clone())),
    }
}

/// The feed's knowledge of a session, folded from frames and caches; pure, so a test can drive
/// it with samples and a clock.
#[derive(Clone, Debug, Default)]
pub struct Fold {
    series: HashMap<(String, String), SeriesSeen>,
    watched: BTreeSet<(String, String)>,
    types: HashMap<(String, String), SeriesType>,
}

impl Fold {
    /// A fold watching every series the binding reads, its objects' and its rules', each with its
    /// type, so a text or a time is read as text whoever reads it.
    pub fn new(binding: &Binding) -> Fold {
        let mut f = Fold::default();
        for o in &binding.objects {
            f.watch(&o.value, o.ty);
            for r in &o.rules {
                if let (Some(s), Some(ty)) = (&r.series, r.ty) {
                    f.watch(s, ty);
                }
            }
        }
        f
    }

    fn watch(&mut self, s: &SeriesBind, ty: SeriesType) {
        let k = (s.source.clone(), s.series.clone());
        self.types.insert(k.clone(), ty);
        self.watched.insert(k);
    }

    /// One numeric sample from a frame, `n` the ingest's count of the series with it. A series
    /// typed text or time is not taken from frames (portal frames a time as its epoch
    /// milliseconds): it arrives through the cache alone, as the ingest renders it, so it reads one
    /// way whichever way it came. A sample is taken when it comes after the one held (`after`), for
    /// the series and for its key: under one stamp the ingest's count orders two lines, so the same
    /// sample read again (a cache polled before its frame) is no new arrival.
    #[allow(clippy::too_many_arguments)]
    pub fn sample(&mut self, source: &str, series: &str, key: Option<&str>, index_ms: i64, value: f64, n: u64, now_ms: i64) {
        let k = (source.to_string(), series.to_string());
        if !self.watched.contains(&k) || self.types.get(&k).is_some_and(|t| !matches!(t, SeriesType::Int | SeriesType::Float)) {
            return;
        }
        let seen = Seen { reading: Reading::Num(value), index_ms, arrived_ms: now_ms, n };
        let s = self.series.entry(k).or_default();
        if s.any.as_ref().is_none_or(|a| after(a, index_ms, n)) {
            s.any = Some(seen.clone());
        }
        if let Some(key) = key {
            s.keyed = true;
            if s.by_key.get(key).is_none_or(|prev| after(prev, index_ms, n)) {
                s.by_key.insert(key.to_string(), seen);
            }
        }
    }

    /// A data frame of the session's stream: `{"type":"data","samples":[{src,ser,k,ms,v,n}]}`;
    /// a sample missing any of them but its key is no sample.
    pub fn frame(&mut self, text: &str, now_ms: i64) {
        let Ok(v) = serde_json::from_str::<Json>(text) else { return };
        let Some(samples) = v.get("samples").and_then(Json::as_array) else { return };
        for s in samples {
            let (Some(src), Some(ser), Some(ms), Some(val), Some(n)) = (s["src"].as_str(), s["ser"].as_str(), s["ms"].as_i64(), s["v"].as_f64(), s["n"].as_u64()) else { continue };
            self.sample(src, ser, s["k"].as_str(), ms, val, n, now_ms);
        }
    }

    /// The ingest's cached last value of a series (whatever its key), taken when it comes after
    /// what the frames brought (`after`, its count its `n`), or when nothing came yet: text and
    /// time arrive only this way. `keys` is how many keys the ingest has seen on the series: a
    /// series with keys is keyed even before a frame carried one (a lag, or a backfill read before
    /// anyone listened). `count` is how many samples it has stored: none yet is no value (an empty
    /// text is a value). The cache trails the frames, so under one stamp a count no higher than the
    /// frame held is an earlier line or the same one, never taken; a higher one (a frame lost to a
    /// lag, a backfill, a text series) is. An entry polled again unchanged keeps its arrival.
    #[allow(clippy::too_many_arguments)]
    pub fn cached(&mut self, source: &str, series: &str, last_index_ms: i64, last_value: &str, keys: usize, count: u64, now_ms: i64) {
        let k = (source.to_string(), series.to_string());
        if !self.watched.contains(&k) {
            return;
        }
        let numeric = self.types.get(&k).is_none_or(|t| matches!(t, SeriesType::Int | SeriesType::Float));
        let s = self.series.entry(k).or_default();
        if keys > 0 {
            s.keyed = true;
        }
        if count == 0 {
            return;
        }
        let reading = match (numeric, last_value.trim().parse::<f64>()) {
            (true, Ok(v)) => Reading::Num(v),
            _ => Reading::Text(last_value.to_string()),
        };
        if s.any.as_ref().is_none_or(|a| after(a, last_index_ms, count)) {
            s.any = Some(Seen { reading, index_ms: last_index_ms, arrived_ms: now_ms, n: count });
        }
    }

    fn seen(&self, b: &SeriesBind) -> Option<&Seen> {
        let s = self.series.get(&(b.source.clone(), b.series.clone()))?;
        match &b.key {
            Some(k) => s.by_key.get(k),
            None => s.any.as_ref(),
        }
    }

    /// One object as it stands at `now_ms`, `live` saying whether its source is live.
    pub fn view(&self, o: &Object, live: bool, now_ms: i64) -> ObjectView {
        let seen = self.seen(&o.value);
        let value = seen.map(|s| s.reading.clone());
        let text = value.as_ref().map(|r| match r {
            Reading::Num(v) => format_number(*v, o.decimals),
            Reading::Text(t) => t.clone(),
        });
        let frac = |v: f64| ((v - o.range.0) / (o.range.1 - o.range.0)).clamp(0.0, 1.0);
        let num = value.as_ref().and_then(Reading::as_f64);
        let level = if o.show == Show::Level { num.map(frac) } else { None };
        let mut place = None;
        let mut items = Vec::new();
        if o.show == Show::Move {
            // A keyed series shows a marker per key it has a place for, never one place for the
            // latest sample whatever its key.
            let keyed = o.value.key.is_none() && self.series.get(&(o.value.source.clone(), o.value.series.clone())).is_some_and(|s| s.keyed);
            if keyed {
                let forget = o.stale_after_ms.unwrap_or(KEY_FORGET_MS) as i64;
                if let Some(s) = self.series.get(&(o.value.source.clone(), o.value.series.clone())) {
                    for (k, seen) in &s.by_key {
                        if let (Some(v), true) = (seen.reading.as_f64(), now_ms - seen.arrived_ms <= forget) {
                            items.push(Item { key: k.clone(), place: frac(v), text: format_number(v, o.decimals) });
                        }
                    }
                }
            } else {
                place = num.map(frac);
            }
        }
        let fresh = match (seen, o.stale_after_ms) {
            (Some(s), Some(bound)) => now_ms - s.arrived_ms <= bound as i64,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let (state, word, stale) = if seen.is_none() {
            (StateClass::Stale, "no value".to_string(), true)
        } else if !live || !fresh {
            (StateClass::Stale, "stale".to_string(), true)
        } else {
            let hit = o.rules.iter().find(|r| {
                let reading = match &r.series {
                    Some(b) => self.seen(b).map(|s| &s.reading),
                    None => seen.map(|s| &s.reading),
                };
                reading.is_some_and(|x| r.compare.holds(x))
            });
            match hit {
                Some(r) => (r.class, r.word.clone(), false),
                None => (StateClass::Normal, o.normal.clone(), false),
            }
        };
        ObjectView { id: o.id.clone(), value, text, state, word, stale, since_ms: seen.map(|s| s.arrived_ms), level, place, items }
    }
}

/// The session's status as the feed publishes it: running, the switch, every source's health,
/// the store's now (the plant's clock, for the page's clock and windows).
pub fn status(live: &Live) -> Json {
    let mut health = serde_json::Map::new();
    if let Ok(h) = live.health.read() {
        for (id, s) in h.iter() {
            let (label, detail) = health_label(&s.state);
            health.insert(id.clone(), json!({ "state": label, "detail": detail }));
        }
    }
    json!({
        "running": !live.stopped.load(std::sync::atomic::Ordering::Relaxed),
        "writes_armed": live.writes_armed(),
        "health": health,
        "now_ms": veredas_portal::serve::store_now_ms(),
    })
}

/// Whether a source is away, by its health and how much it has delivered: idle past the
/// project's window, retrying, ended or failed; or waiting again once it has delivered (portal
/// reads an OPC source reconnecting and a channel whose peer left as waiting). A source waiting
/// before its first delivery is not away: it has given an object nothing to show, and the
/// health is the pump's last refresh, so a first value can arrive before the health reads live.
pub fn away(state: &HealthState, count: u64) -> bool {
    match state {
        HealthState::Live => false,
        HealthState::Waiting => count > 0,
        HealthState::Idle { .. } | HealthState::Retrying(_) | HealthState::Ended | HealthState::Failed(_) => true,
    }
}

/// Which sources are away right now.
fn away_sources(live: &Live) -> BTreeSet<String> {
    live.health
        .read()
        .map(|h| h.iter().filter(|(_, s)| away(&s.state, s.count)).map(|(id, _)| id.clone()).collect())
        .unwrap_or_default()
}

/// One evaluation: the cache folded in, every object read, the session's status.
fn evaluate(fold: &mut Fold, live: &Live, binding: &Binding) -> (Vec<ObjectView>, Json) {
    let now = now_unix_ms();
    if let Ok(cache) = live.series.read() {
        for s in cache.values() {
            fold.cached(s.series.source.as_str(), s.series.series.as_ref(), s.last_index_ms, &s.last_value, s.keys, s.count, now);
        }
    }
    let away = away_sources(live);
    let objects = binding.objects.iter().map(|o| fold.view(o, !away.contains(&o.value.source), now)).collect();
    (objects, status(live))
}

/// The stream a client of the feed reads, as `(event, data)`: a `snapshot` first, then each
/// event as the feed sends it; a client that lags past the channel's buffer gets a fresh
/// `snapshot` in place of what it lost, never a silent gap.
pub fn stream(rx: broadcast::Receiver<(&'static str, Arc<str>)>, snapshot: impl Fn() -> String + Send + Sync + 'static) -> impl tokio_stream::Stream<Item = (String, String)> + Send {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
    let snapshot = Arc::new(snapshot);
    let first = tokio_stream::once(("snapshot".to_string(), snapshot()));
    let rest = BroadcastStream::new(rx).map(move |m| match m {
        Ok((name, data)) => (name.to_string(), data.to_string()),
        Err(BroadcastStreamRecvError::Lagged(_)) => ("snapshot".to_string(), snapshot()),
    });
    first.chain(rest)
}

/// What a feed publishes: the objects as they stand and the session's status.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub objects: Vec<ObjectView>,
    pub status: Json,
}

impl Snapshot {
    pub fn to_json(&self) -> Json {
        json!({ "objects": self.objects.iter().map(ObjectView::to_json).collect::<Vec<_>>(), "session": self.status })
    }
}

/// A running feed: its snapshot, and a channel of its events (`objects` with what changed,
/// `session` with the status), each a ready SSE event name and its JSON.
pub struct Feed {
    pub snapshot: Arc<RwLock<Snapshot>>,
    pub events: broadcast::Sender<(&'static str, Arc<str>)>,
}

impl Feed {
    /// Start the feed of a session on the current tokio runtime; it runs until aborted.
    pub fn spawn(live: Arc<Live>, binding: Arc<Binding>) -> (Feed, tokio::task::JoinHandle<()>) {
        // Subscribed before the first evaluation, so no frame falls between the seed and the stream;
        // evaluated once before anything is served, so neither route ever serves an empty snapshot.
        let mut rx = live.subscribe();
        let mut fold = Fold::new(&binding);
        let (objects, first_status) = evaluate(&mut fold, &live, &binding);
        let snapshot = Arc::new(RwLock::new(Snapshot { objects, status: first_status.clone() }));
        let (events, _) = broadcast::channel(256);
        let feed = Feed { snapshot: Arc::clone(&snapshot), events: events.clone() };
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_status_at = tokio::time::Instant::now() - STATUS_EVERY;
            let mut last_status = first_status;
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Ok(frame) => fold.frame(&frame, now_unix_ms()),
                        // Frames dropped: the cache poll below heals the last values.
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {
                            // The session's stream ended: keep publishing the status (not
                            // running) on the tick alone.
                            rx = live.subscribe();
                            tick.tick().await;
                        }
                    },
                    _ = tick.tick() => {
                        let (objects, status_now) = evaluate(&mut fold, &live, &binding);
                        let changed: Vec<Json> = {
                            let prev = snapshot.read().map(|s| s.objects.clone()).unwrap_or_default();
                            objects.iter().enumerate().filter(|(i, o)| prev.get(*i).is_none_or(|p| !p.same(o))).map(|(_, o)| o.to_json()).collect()
                        };
                        let mut status_changed = false;
                        {
                            let strip = |s: &Json| { let mut s = s.clone(); if let Some(m) = s.as_object_mut() { m.remove("now_ms"); } s };
                            if strip(&status_now) != strip(&last_status) {
                                status_changed = true;
                            }
                        }
                        if let Ok(mut s) = snapshot.write() {
                            s.objects = objects;
                            s.status = status_now.clone();
                        }
                        if !changed.is_empty() {
                            let _ = events.send(("objects", Arc::from(json!({ "objects": changed }).to_string())));
                        }
                        if status_changed || last_status_at.elapsed() >= STATUS_EVERY {
                            last_status_at = tokio::time::Instant::now();
                            last_status = status_now.clone();
                            let _ = events.send(("session", Arc::from(status_now.to_string())));
                        }
                    }
                }
            }
        });
        (feed, task)
    }
}
