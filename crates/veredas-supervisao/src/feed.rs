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
}

/// What the feed knows of one series: its latest sample whatever its key, and per key.
#[derive(Clone, Debug, Default)]
struct SeriesSeen {
    any: Option<Seen>,
    by_key: BTreeMap<String, Seen>,
}

/// A marker of a keyed series: its key and its place along the path, 0 to 1.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Item {
    pub key: String,
    pub place: f64,
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
    /// next change and the snapshot.
    pub fn same(&self, other: &ObjectView) -> bool {
        ObjectView { since_ms: None, ..self.clone() } == ObjectView { since_ms: None, ..other.clone() }
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

/// The health a feed reads of a source: whether it is live, and its label.
fn health_label(state: &HealthState) -> (&'static str, Option<String>) {
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
    /// A fold watching every series the binding reads, its objects' and its rules'.
    pub fn new(binding: &Binding) -> Fold {
        let mut f = Fold::default();
        for o in &binding.objects {
            f.watch(&o.value, o.ty);
            for r in &o.rules {
                if let Some(s) = &r.series {
                    f.watched.insert((s.source.clone(), s.series.clone()));
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

    /// One numeric sample from a frame.
    pub fn sample(&mut self, source: &str, series: &str, key: Option<&str>, index_ms: i64, value: f64, now_ms: i64) {
        let k = (source.to_string(), series.to_string());
        if !self.watched.contains(&k) {
            return;
        }
        let seen = Seen { reading: Reading::Num(value), index_ms, arrived_ms: now_ms };
        let s = self.series.entry(k).or_default();
        if s.any.as_ref().is_none_or(|a| index_ms >= a.index_ms) {
            s.any = Some(seen.clone());
        }
        if let Some(key) = key {
            match s.by_key.get(key) {
                Some(prev) if prev.index_ms > index_ms => {}
                _ => {
                    s.by_key.insert(key.to_string(), seen);
                }
            }
        }
    }

    /// A data frame of the session's stream: `{"type":"data","samples":[{src,ser,k,ms,v}]}`.
    pub fn frame(&mut self, text: &str, now_ms: i64) {
        let Ok(v) = serde_json::from_str::<Json>(text) else { return };
        let Some(samples) = v.get("samples").and_then(Json::as_array) else { return };
        for s in samples {
            let (Some(src), Some(ser), Some(ms), Some(val)) = (s["src"].as_str(), s["ser"].as_str(), s["ms"].as_i64(), s["v"].as_f64()) else { continue };
            self.sample(src, ser, s["k"].as_str(), ms, val, now_ms);
        }
    }

    /// The ingest's cached last value of a series (whatever its key), taken when it is newer than
    /// what the frames brought, or when nothing came yet: text arrives only this way.
    pub fn cached(&mut self, source: &str, series: &str, last_index_ms: i64, last_value: &str, now_ms: i64) {
        let k = (source.to_string(), series.to_string());
        if !self.watched.contains(&k) || last_value.is_empty() {
            return;
        }
        let numeric = self.types.get(&k).is_none_or(|t| matches!(t, SeriesType::Int | SeriesType::Float));
        let reading = match (numeric, last_value.trim().parse::<f64>()) {
            (true, Ok(v)) => Reading::Num(v),
            _ => Reading::Text(last_value.to_string()),
        };
        let s = self.series.entry(k).or_default();
        if s.any.as_ref().is_none_or(|a| last_index_ms > a.index_ms) {
            s.any = Some(Seen { reading, index_ms: last_index_ms, arrived_ms: now_ms });
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
            let keyed = o.value.key.is_none()
                && self.series.get(&(o.value.source.clone(), o.value.series.clone())).is_some_and(|s| !s.by_key.is_empty());
            if keyed {
                let forget = o.stale_after_ms.unwrap_or(KEY_FORGET_MS) as i64;
                if let Some(s) = self.series.get(&(o.value.source.clone(), o.value.series.clone())) {
                    for (k, seen) in &s.by_key {
                        if let (Some(v), true) = (seen.reading.as_f64(), now_ms - seen.arrived_ms <= forget) {
                            items.push(Item { key: k.clone(), place: frac(v) });
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

/// Which sources are away right now: idle past the session's window, retrying, ended or failed.
/// A source still waiting has delivered nothing an object could show, and the health is the
/// pump's last refresh, so a first value can arrive before the health reads live: waiting is
/// not away.
fn away_sources(live: &Live) -> BTreeSet<String> {
    live.health
        .read()
        .map(|h| h.iter().filter(|(_, s)| !matches!(s.state, HealthState::Live | HealthState::Waiting)).map(|(id, _)| id.clone()).collect())
        .unwrap_or_default()
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
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let (events, _) = broadcast::channel(256);
        let feed = Feed { snapshot: Arc::clone(&snapshot), events: events.clone() };
        // Subscribed before the first evaluation, so no frame falls between the seed and the stream.
        let mut rx = live.subscribe();
        let mut fold = Fold::new(&binding);
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_status_at = tokio::time::Instant::now() - STATUS_EVERY;
            let mut last_status = Json::Null;
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
                        let now = now_unix_ms();
                        if let Ok(cache) = live.series.read() {
                            for s in cache.values() {
                                fold.cached(s.series.source.as_str(), s.series.series.as_ref(), s.last_index_ms, &s.last_value, now);
                            }
                        }
                        let away = away_sources(&live);
                        let objects: Vec<ObjectView> = binding.objects.iter().map(|o| fold.view(o, !away.contains(&o.value.source), now)).collect();
                        let status_now = status(&live);
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
