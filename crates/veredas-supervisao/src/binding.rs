//! The binding file (`veredas-binding` v1, `docs/binding-schema.md`): a session's objects by ID,
//! each bound to a series its project carries, with its state rules, how the layout shows it and
//! the writes it offers; the layout; the headline; the dashboards and their panels.
//!
//! Everything a binding names is checked when it loads, against the project it binds (every
//! source, series, writable tag and table) and against the layout (every marker's path), so a
//! binding that loads lights up: a misspelled series is refused with the series the source does
//! carry, never left dark (Veredas 1's lesson: naming literals shared across components must be
//! pinned, or nothing lights up).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value as Json, json};
use veredas_core::SeriesType;
use veredas_portal::CaptureRule;
use veredas_portal::project::Project;

use crate::state::{Comparand, Compare, StateClass};
use crate::svg::{self, FOLLOWABLE, Layout};

pub const FORMAT: &str = "veredas-binding";
pub const VERSION: u32 = 1;
/// What the header reader calls this file.
pub const NOUN: &str = "a binding file";

/// The longest object ID.
pub const ID_MAX: usize = 64;
/// How long a key of a keyed marker is shown after its last sample, when the object names no
/// bound of its own.
pub const KEY_FORGET_MS: u64 = 10_000;
/// The most rows a table panel shows, and the most a cells panel reads.
pub const TABLE_LIMIT_MAX: usize = 1_000;
pub const CELLS_ROWS_MAX: usize = 50_000;
/// The bins of a cells panel's distribution.
pub const CELLS_BINS: usize = 20;

// ---- what a project carries ----------------------------------------------------------------------

/// A source of a project as a binding sees it: its kind, the series it carries with their types,
/// and, for an OPC source, the tags a rule marks writable (by node id text, as portal names them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceUniverse {
    pub kind: &'static str,
    pub series: BTreeMap<String, SeriesType>,
    pub writable: BTreeSet<String>,
}

/// Everything a project carries that a binding can name: its sources with their series and
/// writable tags, and its tables with their columns (a view's are the engine's to know when the
/// session runs, so a view's columns are `None`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Universe {
    pub sources: BTreeMap<String, SourceUniverse>,
    pub tables: BTreeMap<String, Option<BTreeMap<String, SeriesType>>>,
}

/// A group's type as the session reads a capture rule's: none is text, the words in any case.
fn capture_type(text: &str) -> SeriesType {
    if text.trim().is_empty() { SeriesType::Text } else { veredas_portal::opc::parse_type(text).unwrap_or(SeriesType::Text) }
}

/// A series several parts carry (two rules, two messages) is numeric only where every one of
/// them types it so: an int and a float read as a float, anything with text or a time as text.
fn merge(series: &mut BTreeMap<String, SeriesType>, name: &str, ty: SeriesType) {
    let merged = match (series.get(name).copied(), ty) {
        (None, t) => t,
        (Some(a), b) if a == b => a,
        (Some(SeriesType::Int | SeriesType::Float), SeriesType::Int | SeriesType::Float) => SeriesType::Float,
        _ => SeriesType::Text,
    };
    series.insert(name.to_string(), merged);
}

/// The columns every base table carries beside its value columns.
fn with_reserved(columns: &[(String, SeriesType)]) -> BTreeMap<String, SeriesType> {
    let mut out: BTreeMap<String, SeriesType> = [
        ("source", SeriesType::Text),
        ("key", SeriesType::Text),
        ("index_ms", SeriesType::Int),
        ("ts", SeriesType::Time),
        ("arrival_ms", SeriesType::Int),
        ("seq", SeriesType::Int),
    ]
    .into_iter()
    .map(|(n, t)| (n.to_string(), t))
    .collect();
    for (n, t) in columns {
        out.insert(n.clone(), *t);
    }
    out
}

impl Universe {
    /// What a loaded project carries, compiled by portal's own readers as its session compiles
    /// it: a log's series are the value groups of every capture rule that reads it (its key group
    /// is no series), typed as the session types them; an OPC source's are its rules' columns
    /// (a tag's series, a bit's), and its writable tags by node id text; a channel's are its
    /// catalog messages' columns. The tables are the capture rules, the OPC rules, the messages
    /// and the views. The project compiled every part when it loaded; a part that does not
    /// compile here carries nothing to bind. No secret is needed: names are compiled, not opened.
    pub fn of(project: &Project) -> Universe {
        let mut u = Universe::default();
        for log in &project.logs {
            u.sources.insert(log.id.clone(), SourceUniverse { kind: "log", ..Default::default() });
        }
        for rule in &project.rules {
            let types: HashMap<String, SeriesType> = rule.types.iter().map(|(k, v)| (k.clone(), capture_type(v))).collect();
            let Ok(c) = CaptureRule::new(&rule.name, rule.pattern.trim(), None, &types, rule.key.as_deref().map(str::trim)) else { continue };
            let columns: Vec<(String, SeriesType)> = c.series().map(|(n, t)| (n.to_string(), t)).collect();
            let scope = rule.scope.as_deref().map(str::trim).filter(|x| !x.is_empty());
            for log in &project.logs {
                if scope.is_some_and(|x| x != log.id) {
                    continue;
                }
                if let Some(src) = u.sources.get_mut(&log.id) {
                    for (n, t) in &columns {
                        merge(&mut src.series, n, *t);
                    }
                }
            }
            u.tables.insert(rule.name.trim().to_string(), Some(with_reserved(&columns)));
        }
        let anything = |_: &str| Some(String::new());
        for opc in &project.opc {
            let mut src = SourceUniverse { kind: "opc", ..Default::default() };
            if let Ok(spec) = veredas_portal::opc::compile(opc, &anything) {
                for rule in &spec.rules {
                    let (name, columns) = veredas_portal::opc::table_of(rule);
                    for (n, t) in &columns {
                        merge(&mut src.series, n, *t);
                    }
                    u.tables.insert(name, Some(with_reserved(&columns)));
                    for tag in rule.tags.iter().filter(|t| t.writable) {
                        src.writable.insert(tag.raw.trim().to_string());
                    }
                }
            }
            u.sources.insert(opc.source_id.clone(), src);
        }
        for ch in &project.channels {
            let mut src = SourceUniverse { kind: "channel", ..Default::default() };
            if let Ok(tables) = veredas_portal::channel::check(ch) {
                for t in tables {
                    for (n, ty) in &t.columns {
                        merge(&mut src.series, n, *ty);
                    }
                    u.tables.insert(t.name.clone(), Some(with_reserved(&t.columns)));
                }
            }
            u.sources.insert(ch.source_id.clone(), src);
        }
        for (name, _) in &project.tables {
            u.tables.insert(name.clone(), None);
        }
        u
    }
}

// ---- the compiled binding ----------------------------------------------------------------------

/// A series bound: a source, a series, and the one key read when given.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeriesBind {
    pub source: String,
    pub series: String,
    pub key: Option<String>,
}

/// How the layout shows an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    State,
    Level,
    Value,
    Move,
}

impl Show {
    pub fn name(self) -> &'static str {
        match self {
            Show::State => "state",
            Show::Level => "level",
            Show::Value => "value",
            Show::Move => "move",
        }
    }
}

/// A state rule: a comparison over the object's value, or over another series.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub compare: Compare,
    pub class: StateClass,
    pub word: String,
    /// Another series compared instead of the object's value.
    pub series: Option<SeriesBind>,
}

/// A write an object offers, posted to portal's surface by the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    pub source: String,
    pub tag: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: String,
    pub label: String,
    pub value: SeriesBind,
    pub ty: SeriesType,
    pub unit: String,
    pub decimals: Option<u8>,
    pub show: Show,
    pub range: (f64, f64),
    pub path: Option<String>,
    pub rules: Vec<Rule>,
    pub normal: String,
    pub stale_after_ms: Option<u64>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aggregate {
    Mean,
    Sum,
    Count,
    Min,
    Max,
    Last,
}

impl Aggregate {
    fn parse(s: &str) -> Option<Aggregate> {
        Some(match s {
            "mean" => Aggregate::Mean,
            "sum" => Aggregate::Sum,
            "count" => Aggregate::Count,
            "min" => Aggregate::Min,
            "max" => Aggregate::Max,
            "last" => Aggregate::Last,
            _ => return None,
        })
    }
}

/// A trend's limit: a breach is a point past it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Limit {
    Above(f64),
    Below(f64),
}

/// A column of a table panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub label: String,
    pub bar: bool,
    pub decimals: Option<u8>,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelKind {
    Value { object: usize, minutes: u32, target: Option<f64> },
    Trend { object: usize, minutes: u32, limit: Option<Limit> },
    Bars { object: usize, minutes: u32, bucket_minutes: u32, aggregate: Aggregate, target: Option<f64> },
    Timeline { objects: Vec<usize>, minutes: u32 },
    Grid { objects: Vec<usize> },
    Levels { objects: Vec<usize>, ticks: Vec<f64> },
    Table { table: String, columns: Vec<Column>, sort: Option<(String, bool)>, limit: usize },
    Cells { table: String, key: String, value: String, minutes: u32, bucket_minutes: u32, spread: f64 },
}

impl PanelKind {
    pub fn name(&self) -> &'static str {
        match self {
            PanelKind::Value { .. } => "value",
            PanelKind::Trend { .. } => "trend",
            PanelKind::Bars { .. } => "bars",
            PanelKind::Timeline { .. } => "timeline",
            PanelKind::Grid { .. } => "grid",
            PanelKind::Levels { .. } => "levels",
            PanelKind::Table { .. } => "table",
            PanelKind::Cells { .. } => "cells",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub title: String,
    pub span: u32,
    pub kind: PanelKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dashboard {
    pub name: String,
    pub columns: u32,
    pub panels: Vec<Panel>,
}

/// A binding as read and checked.
#[derive(Clone, Debug)]
pub struct Binding {
    pub title: String,
    pub layout: Option<Layout>,
    /// Object indices, in order.
    pub headline: Vec<usize>,
    pub objects: Vec<Object>,
    pub dashboards: Vec<Dashboard>,
}

impl Binding {
    pub fn object(&self, id: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    /// What the page needs to draw the session: the objects as it shows them, the headline, the
    /// dashboards' shapes (their numbers come from the dashboard route), whether there is a layout.
    pub fn page_view(&self) -> Json {
        json!({
            "title": self.title,
            "layout": self.layout.as_ref().and_then(|l| l.file.file_name()).map(|f| f.to_string_lossy().into_owned()),
            "headline": self.headline.iter().map(|i| self.objects[*i].id.clone()).collect::<Vec<_>>(),
            "objects": self.objects.iter().map(|o| json!({
                "id": o.id,
                "label": o.label,
                "unit": o.unit,
                "show": o.show.name(),
                "range": [o.range.0, o.range.1],
                "path": o.path,
                "source": o.value.source,
                "series": o.value.series,
                "key": o.value.key,
                "normal": o.normal,
                "rules": o.rules.iter().map(|r| json!({ "state": r.class.name(), "word": r.word, "when": r.compare.describe(&o.unit) })).collect::<Vec<_>>(),
                "actions": o.actions.iter().map(|a| json!({ "label": a.label, "source": a.source, "tag": a.tag, "value": a.value })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "dashboards": self.dashboards.iter().map(|d| json!({
                "name": d.name,
                "columns": d.columns,
                "panels": d.panels.iter().map(|p| json!({ "kind": p.kind.name(), "title": p.title, "span": p.span })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

// ---- the file as written -------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    #[allow(dead_code)]
    format: toml::Value,
    #[allow(dead_code)]
    version: toml::Value,
    title: Option<String>,
    layout: Option<String>,
    #[serde(default)]
    headline: Vec<String>,
    #[serde(default)]
    object: Vec<RawObject>,
    #[serde(default)]
    dashboard: Vec<RawDashboard>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSeries {
    source: String,
    series: String,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObject {
    id: String,
    label: Option<String>,
    value: RawSeries,
    #[serde(default)]
    unit: String,
    decimals: Option<u8>,
    show: Option<String>,
    range: Option<Vec<f64>>,
    path: Option<String>,
    #[serde(default)]
    states: Vec<RawRule>,
    normal: Option<String>,
    stale_after_ms: Option<u64>,
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    equals: Option<toml::Value>,
    above: Option<f64>,
    below: Option<f64>,
    at_least: Option<f64>,
    at_most: Option<f64>,
    state: String,
    word: Option<String>,
    source: Option<String>,
    series: Option<String>,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    label: String,
    source: String,
    tag: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDashboard {
    name: String,
    columns: Option<u32>,
    #[serde(default)]
    panel: Vec<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PValue {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    object: String,
    minutes: Option<u32>,
    target: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimit {
    above: Option<f64>,
    below: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PTrend {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    object: String,
    minutes: Option<u32>,
    limit: Option<RawLimit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PBars {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    object: String,
    minutes: Option<u32>,
    bucket_minutes: Option<u32>,
    aggregate: Option<String>,
    target: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PObjects {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    objects: Vec<String>,
    minutes: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PGrid {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    objects: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PLevels {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    objects: Vec<String>,
    #[serde(default)]
    ticks: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColumn {
    name: String,
    label: Option<String>,
    #[serde(default)]
    bar: bool,
    decimals: Option<u8>,
    #[serde(default)]
    states: Vec<RawRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSort {
    column: String,
    #[serde(default)]
    descending: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PTable {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    table: String,
    #[serde(default)]
    columns: Vec<RawColumn>,
    sort: Option<RawSort>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PCells {
    #[allow(dead_code)]
    kind: String,
    title: Option<String>,
    span: Option<u32>,
    table: String,
    key: String,
    value: String,
    minutes: Option<u32>,
    bucket_minutes: Option<u32>,
    spread: f64,
}

// ---- reading and checking ------------------------------------------------------------------------

/// The shape of an object ID: an ASCII letter, then ASCII letters, digits, `-`, `_` and `.`.
pub fn id_shape(id: &str) -> Result<(), String> {
    let mut chars = id.chars();
    match chars.next() {
        None => return Err("the id is empty".into()),
        Some(c) if !c.is_ascii_alphabetic() => return Err(format!("the id `{id}` does not start with an ASCII letter")),
        _ => {}
    }
    if let Some(c) = chars.find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
        return Err(format!("the id `{id}` holds `{c}`: an id is ASCII letters, digits, `-`, `_` and `.`"));
    }
    if id.len() > ID_MAX {
        return Err(format!("the id `{id}` is longer than {ID_MAX}"));
    }
    Ok(())
}

fn de_err(place: &str, e: impl std::fmt::Display) -> String {
    format!("{place}: {}", e.to_string().trim_end())
}

/// Check a series against the universe: the source is the project's, the series one it carries.
fn check_series(place: &str, source: &str, series: &str, key: Option<&str>, u: &Universe) -> Result<(SeriesBind, SeriesType), String> {
    let Some(src) = u.sources.get(source) else {
        let known: Vec<&str> = u.sources.keys().map(String::as_str).collect();
        return Err(format!("{place}: `{source}` is no source of the project (its sources: {})", list(&known)));
    };
    let Some(ty) = src.series.get(series) else {
        let carried: Vec<&str> = src.series.keys().map(String::as_str).collect();
        return Err(format!("{place}: the {} source `{source}` carries no series `{series}` (it carries: {})", src.kind, list(&carried)));
    };
    if key.is_some_and(|k| k.is_empty()) {
        return Err(format!("{place}: the key is empty"));
    }
    Ok((SeriesBind { source: source.to_string(), series: series.to_string(), key: key.map(String::from) }, *ty))
}

fn list(items: &[&str]) -> String {
    if items.is_empty() { "none".to_string() } else { items.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ") }
}

fn finite(place: &str, what: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() { Ok(v) } else { Err(format!("{place}: {what} is not a finite number")) }
}

fn compile_rule(place: &str, r: RawRule, default_source: &str, u: &Universe) -> Result<Rule, String> {
    let mut found: Vec<Compare> = Vec::new();
    if let Some(v) = r.equals {
        found.push(Compare::Equals(match v {
            toml::Value::Integer(i) => Comparand::Num(i as f64),
            toml::Value::Float(f) => Comparand::Num(finite(place, "`equals`", f)?),
            toml::Value::String(s) => Comparand::Text(s),
            other => return Err(format!("{place}: `equals` takes a number or a string, not {}", other.type_str())),
        }));
    }
    for (name, v, make) in [
        ("above", r.above, Compare::Above as fn(f64) -> Compare),
        ("below", r.below, Compare::Below),
        ("at_least", r.at_least, Compare::AtLeast),
        ("at_most", r.at_most, Compare::AtMost),
    ] {
        if let Some(v) = v {
            found.push(make(finite(place, &format!("`{name}`"), v)?));
        }
    }
    let compare = match found.len() {
        1 => found.remove(0),
        0 => return Err(format!("{place}: a rule needs one comparison (`equals`, `above`, `below`, `at_least` or `at_most`)")),
        n => return Err(format!("{place}: a rule takes one comparison, this one has {n}")),
    };
    let Some(class) = StateClass::of_rule(r.state.trim()) else {
        return Err(format!("{place}: the state `{}` is none of `near`, `warn`, `alarm`, `disabled`", r.state));
    };
    let word = r.word.unwrap_or_else(|| class.name().to_string());
    let series = match (r.source, r.series, r.key) {
        (None, None, None) => None,
        (source, Some(series), key) => {
            let source = source.unwrap_or_else(|| default_source.to_string());
            Some(check_series(place, &source, &series, key.as_deref(), u)?.0)
        }
        (_, None, _) => return Err(format!("{place}: a rule over another series names its `series`")),
    };
    Ok(Rule { compare, class, word, series })
}

/// The objects an `objects` list names: an entry ending in `*` every object whose ID starts with
/// what precedes it, in the binding's order; each entry naming at least one.
fn resolve_objects(place: &str, entries: &[String], objects: &[Object]) -> Result<Vec<usize>, String> {
    if entries.is_empty() {
        return Err(format!("{place}: `objects` names no object"));
    }
    let mut out = Vec::new();
    for e in entries {
        let found: Vec<usize> = match e.strip_suffix('*') {
            Some(prefix) => objects.iter().enumerate().filter(|(_, o)| o.id.starts_with(prefix)).map(|(i, _)| i).collect(),
            None => objects.iter().position(|o| o.id == *e).into_iter().collect(),
        };
        if found.is_empty() {
            return Err(format!("{place}: `{e}` names no object"));
        }
        out.extend(found);
    }
    Ok(out)
}

fn one_object(place: &str, id: &str, objects: &[Object]) -> Result<usize, String> {
    objects.iter().position(|o| o.id == id).ok_or_else(|| format!("{place}: `{id}` names no object"))
}

/// Read a binding's text, its layout resolved against `base`, everything it names checked
/// against the project's universe. `place` names the file in every refusal.
pub fn load(text: &str, base: &Path, universe: &Universe, project_name: &str) -> Result<Binding, String> {
    veredas_core::schema::check(text, NOUN, FORMAT, VERSION).map_err(|e| e.to_string())?;
    let raw: RawBinding = toml::from_str(text).map_err(|e| format!("not {NOUN}: {}", e.to_string().trim_end()))?;

    // The layout first: the objects' paths are checked against it.
    let layout = match raw.layout {
        None => None,
        Some(p) if p.trim().is_empty() => return Err("`layout` is empty".into()),
        Some(p) => Some(svg::read(&base.join(p.trim())).map_err(|e| format!("the layout: {e}"))?),
    };

    let mut objects: Vec<Object> = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (i, o) in raw.object.into_iter().enumerate() {
        id_shape(&o.id).map_err(|e| format!("object {i}: {e}"))?;
        let place = format!("object `{}`", o.id);
        if let Some(first) = seen.insert(o.id.to_ascii_lowercase(), o.id.clone()) {
            return Err(format!("{place}: the id is taken by `{first}` (ids differ by more than their case)"));
        }
        let (value, ty) = check_series(&format!("{place}, value"), &o.value.source, &o.value.series, o.value.key.as_deref(), universe)?;
        let show = match o.show.as_deref().map(str::trim) {
            None | Some("state") => Show::State,
            Some("level") => Show::Level,
            Some("value") => Show::Value,
            Some("move") => Show::Move,
            Some(other) => return Err(format!("{place}: `show` is `{other}`, none of `state`, `level`, `value`, `move`")),
        };
        let numeric = matches!(ty, SeriesType::Int | SeriesType::Float);
        if matches!(show, Show::Level | Show::Move) && !numeric {
            return Err(format!("{place}: a {} needs a numeric series, and `{}` is {}", if show == Show::Level { "level" } else { "marker" }, value.series, type_word(ty)));
        }
        let range = match o.range.as_deref() {
            None => (0.0, 100.0),
            Some([lo, hi]) => {
                let (lo, hi) = (finite(&place, "the range's low", *lo)?, finite(&place, "the range's high", *hi)?);
                if lo >= hi {
                    return Err(format!("{place}: the range is empty ({} is not below {})", crate::state::format_number(lo, None), crate::state::format_number(hi, None)));
                }
                (lo, hi)
            }
            Some(other) => return Err(format!("{place}: `range` takes a low and a high, not {} values", other.len())),
        };
        let path = match (show, o.path) {
            (Show::Move, None) => return Err(format!("{place}: a marker needs the `path` it follows")),
            (Show::Move, Some(p)) => {
                let Some(layout) = &layout else {
                    return Err(format!("{place}: a marker follows an element of the layout, and the binding has no `layout`"));
                };
                match layout.ids.get(&p) {
                    None => return Err(format!("{place}: the path `{p}` is no element of the layout")),
                    Some((_, n)) if *n > 1 => return Err(format!("{place}: the path `{p}` is carried by {n} elements of the layout")),
                    Some((name, _)) if !FOLLOWABLE.contains(&name.as_str()) => {
                        return Err(format!("{place}: the path `{p}` is a <{name}>, which no marker can follow (one of {})", FOLLOWABLE.join(", ")));
                    }
                    Some(_) => Some(p),
                }
            }
            (_, Some(_)) => return Err(format!("{place}: `path` is for a marker (`show = \"move\"`)")),
            (_, None) => None,
        };
        if let Some(d) = o.decimals
            && d > 6
        {
            return Err(format!("{place}: `decimals` is {d}, past 6"));
        }
        let mut rules = Vec::new();
        for (j, r) in o.states.into_iter().enumerate() {
            rules.push(compile_rule(&format!("{place}, states[{j}]"), r, &value.source, universe)?);
        }
        let mut actions = Vec::new();
        for (j, a) in o.actions.into_iter().enumerate() {
            let ap = format!("{place}, actions[{j}]");
            if a.label.trim().is_empty() {
                return Err(format!("{ap}: the label is empty"));
            }
            match universe.sources.get(&a.source) {
                Some(s) if s.kind == "opc" => {
                    if !s.writable.contains(a.tag.trim()) {
                        let w: Vec<&str> = s.writable.iter().map(String::as_str).collect();
                        return Err(format!("{ap}: `{}` is no tag of `{}` a rule marks writable (its writable tags: {})", a.tag, a.source, list(&w)));
                    }
                }
                _ => return Err(format!("{ap}: `{}` is no OPC source of the project", a.source)),
            }
            actions.push(Action { label: a.label, source: a.source, tag: a.tag.trim().to_string(), value: a.value });
        }
        if o.stale_after_ms == Some(0) {
            return Err(format!("{place}: `stale_after_ms` is 0"));
        }
        objects.push(Object {
            label: o.label.unwrap_or_else(|| o.id.clone()),
            id: o.id,
            value,
            ty,
            unit: o.unit,
            decimals: o.decimals,
            show,
            range,
            path,
            rules,
            normal: o.normal.unwrap_or_else(|| "ok".to_string()),
            stale_after_ms: o.stale_after_ms,
            actions,
        });
    }

    if let Some(layout) = &layout {
        for o in &objects {
            if let Some((_, n)) = layout.ids.get(&o.id)
                && *n > 1
            {
                return Err(format!("object `{}`: the layout carries the id on {n} elements", o.id));
            }
        }
    }

    let mut headline = Vec::new();
    for h in &raw.headline {
        headline.push(one_object("headline", h, &objects)?);
    }

    let mut dashboards = Vec::new();
    let mut names = BTreeSet::new();
    for (i, d) in raw.dashboard.into_iter().enumerate() {
        if d.name.trim().is_empty() {
            return Err(format!("dashboard {i}: the name is empty"));
        }
        let place = format!("dashboard `{}`", d.name);
        if !names.insert(d.name.clone()) {
            return Err(format!("{place}: the name is taken"));
        }
        let columns = d.columns.unwrap_or(4);
        if !(1..=12).contains(&columns) {
            return Err(format!("{place}: `columns` is {columns}, not 1 to 12"));
        }
        let mut panels = Vec::new();
        for (j, t) in d.panel.into_iter().enumerate() {
            let kind = t.get("kind").and_then(|k| k.as_str()).map(str::to_string);
            let Some(kind) = kind else {
                return Err(format!("{place}, panel[{j}]: a panel names its `kind`"));
            };
            let pp = format!("{place}, panel[{j}] ({kind})");
            panels.push(compile_panel(&pp, &kind, toml::Value::Table(t), &objects, universe, columns)?);
        }
        dashboards.push(Dashboard { name: d.name, columns, panels });
    }

    let title = raw.title.unwrap_or_else(|| project_name.to_string());
    Ok(Binding { title, layout, headline, objects, dashboards })
}

fn type_word(ty: SeriesType) -> &'static str {
    match ty {
        SeriesType::Int => "an int",
        SeriesType::Float => "a float",
        SeriesType::Text => "text",
        SeriesType::Time => "a time",
    }
}

fn span_of(place: &str, span: Option<u32>, columns: u32) -> Result<u32, String> {
    let s = span.unwrap_or(1);
    if s == 0 || s > columns {
        return Err(format!("{place}: `span` is {s}, not 1 to the dashboard's {columns} columns"));
    }
    Ok(s)
}

fn minutes_of(place: &str, m: Option<u32>, default: u32) -> Result<u32, String> {
    let m = m.unwrap_or(default);
    if m == 0 {
        return Err(format!("{place}: `minutes` is 0"));
    }
    Ok(m)
}

fn numeric_object(place: &str, i: usize, objects: &[Object]) -> Result<(), String> {
    let o = &objects[i];
    if matches!(o.ty, SeriesType::Int | SeriesType::Float) {
        Ok(())
    } else {
        Err(format!("{place}: `{}` reads `{}`, which is {}, and the panel charts numbers", o.id, o.value.series, type_word(o.ty)))
    }
}

fn compile_panel(place: &str, kind: &str, t: toml::Value, objects: &[Object], u: &Universe, columns: u32) -> Result<Panel, String> {
    let de = |e: toml::de::Error| de_err(place, e.message());
    let (title, span, kind) = match kind {
        "value" => {
            let p: PValue = t.try_into().map_err(de)?;
            let object = one_object(place, &p.object, objects)?;
            let target = p.target.map(|v| finite(place, "`target`", v)).transpose()?;
            (p.title, p.span, PanelKind::Value { object, minutes: minutes_of(place, p.minutes, 30)?, target })
        }
        "trend" => {
            let p: PTrend = t.try_into().map_err(de)?;
            let object = one_object(place, &p.object, objects)?;
            numeric_object(place, object, objects)?;
            let limit = match p.limit {
                None => None,
                Some(RawLimit { above: Some(a), below: None }) => Some(Limit::Above(finite(place, "the limit", a)?)),
                Some(RawLimit { above: None, below: Some(b) }) => Some(Limit::Below(finite(place, "the limit", b)?)),
                Some(_) => return Err(format!("{place}: a limit is `{{ above = x }}` or `{{ below = x }}`")),
            };
            (p.title, p.span, PanelKind::Trend { object, minutes: minutes_of(place, p.minutes, 60)?, limit })
        }
        "bars" => {
            let p: PBars = t.try_into().map_err(de)?;
            let object = one_object(place, &p.object, objects)?;
            numeric_object(place, object, objects)?;
            let bucket_minutes = p.bucket_minutes.unwrap_or(5);
            if bucket_minutes == 0 {
                return Err(format!("{place}: `bucket_minutes` is 0"));
            }
            let aggregate = match p.aggregate.as_deref() {
                None => Aggregate::Mean,
                Some(a) => Aggregate::parse(a).ok_or_else(|| format!("{place}: `aggregate` is `{a}`, none of `mean`, `sum`, `count`, `min`, `max`, `last`"))?,
            };
            let target = p.target.map(|v| finite(place, "`target`", v)).transpose()?;
            (p.title, p.span, PanelKind::Bars { object, minutes: minutes_of(place, p.minutes, 60)?, bucket_minutes, aggregate, target })
        }
        "timeline" => {
            let p: PObjects = t.try_into().map_err(de)?;
            let objs = resolve_objects(place, &p.objects, objects)?;
            for i in &objs {
                numeric_object(place, *i, objects)?;
            }
            (p.title, p.span, PanelKind::Timeline { objects: objs, minutes: minutes_of(place, p.minutes, 60)? })
        }
        "grid" => {
            let p: PGrid = t.try_into().map_err(de)?;
            (p.title, p.span, PanelKind::Grid { objects: resolve_objects(place, &p.objects, objects)? })
        }
        "levels" => {
            let p: PLevels = t.try_into().map_err(de)?;
            let objs = resolve_objects(place, &p.objects, objects)?;
            for i in &objs {
                numeric_object(place, *i, objects)?;
            }
            let ticks = p.ticks.iter().map(|v| finite(place, "a tick", *v)).collect::<Result<Vec<_>, _>>()?;
            (p.title, p.span, PanelKind::Levels { objects: objs, ticks })
        }
        "table" => {
            let p: PTable = t.try_into().map_err(de)?;
            if !u.tables.contains_key(&p.table) {
                return Err(format!("{place}: `{}` is no table of the project", p.table));
            }
            let limit = p.limit.unwrap_or(50);
            if !(1..=TABLE_LIMIT_MAX).contains(&limit) {
                return Err(format!("{place}: `limit` is {limit}, not 1 to {TABLE_LIMIT_MAX}"));
            }
            let mut cols = Vec::new();
            for (k, c) in p.columns.into_iter().enumerate() {
                let cp = format!("{place}, columns[{k}]");
                if c.name.trim().is_empty() {
                    return Err(format!("{cp}: the name is empty"));
                }
                if let Some(d) = c.decimals
                    && d > 6
                {
                    return Err(format!("{cp}: `decimals` is {d}, past 6"));
                }
                let mut rules = Vec::new();
                for (j, r) in c.states.into_iter().enumerate() {
                    if r.source.is_some() || r.series.is_some() || r.key.is_some() {
                        return Err(format!("{cp}, states[{j}]: a column's rule compares the column's own value"));
                    }
                    rules.push(compile_rule(&format!("{cp}, states[{j}]"), r, "", u)?);
                }
                cols.push(Column { label: c.label.unwrap_or_else(|| c.name.clone()), name: c.name, bar: c.bar, decimals: c.decimals, rules });
            }
            if let Some(s) = &p.sort
                && !cols.is_empty()
                && !cols.iter().any(|c| c.name == s.column)
            {
                return Err(format!("{place}: the sort column `{}` is none of the panel's columns", s.column));
            }
            if let Some(Some(known)) = u.tables.get(&p.table) {
                let names: Vec<&str> = known.keys().map(String::as_str).collect();
                for (k, c) in cols.iter().enumerate() {
                    if !known.contains_key(&c.name) {
                        return Err(format!("{place}, columns[{k}]: `{}` has no column `{}` (its columns: {})", p.table, c.name, list(&names)));
                    }
                }
                if let Some(s) = &p.sort
                    && cols.is_empty()
                    && !known.contains_key(&s.column)
                {
                    return Err(format!("{place}: the sort column `{}` is no column of `{}` (its columns: {})", s.column, p.table, list(&names)));
                }
            }
            let sort = p.sort.map(|s| (s.column, s.descending));
            (p.title.or(Some(p.table.clone())), p.span, PanelKind::Table { table: p.table, columns: cols, sort, limit })
        }
        "cells" => {
            let p: PCells = t.try_into().map_err(de)?;
            if !u.tables.contains_key(&p.table) {
                return Err(format!("{place}: `{}` is no table of the project", p.table));
            }
            if let Some(Some(known)) = u.tables.get(&p.table) {
                let names: Vec<&str> = known.keys().map(String::as_str).collect();
                for (what, col) in [("key", &p.key), ("value", &p.value)] {
                    if !known.contains_key(col) {
                        return Err(format!("{place}: the {what} `{col}` is no column of `{}` (its columns: {})", p.table, list(&names)));
                    }
                }
                if !matches!(known.get(&p.value), Some(SeriesType::Int | SeriesType::Float)) {
                    return Err(format!("{place}: the value `{}` of `{}` is not numeric", p.value, p.table));
                }
            }
            if !(p.spread.is_finite() && p.spread > 0.0) {
                return Err(format!("{place}: `spread` is not a number above 0"));
            }
            let bucket_minutes = p.bucket_minutes.unwrap_or(5);
            if bucket_minutes == 0 {
                return Err(format!("{place}: `bucket_minutes` is 0"));
            }
            let minutes = minutes_of(place, p.minutes, 60)?;
            (p.title.or(Some(p.table.clone())), p.span, PanelKind::Cells { table: p.table, key: p.key, value: p.value, minutes, bucket_minutes, spread: p.spread })
        }
        other => return Err(format!("{place}: the kind `{other}` is none of `value`, `trend`, `bars`, `timeline`, `grid`, `levels`, `table`, `cells`")),
    };
    let span = span_of(place, span, columns)?;
    let title = title.unwrap_or_else(|| match &kind {
        PanelKind::Value { object, .. } | PanelKind::Trend { object, .. } | PanelKind::Bars { object, .. } => objects[*object].label.clone(),
        PanelKind::Timeline { .. } => "State over time".to_string(),
        PanelKind::Grid { .. } => "States".to_string(),
        PanelKind::Levels { .. } => "Levels".to_string(),
        PanelKind::Table { table, .. } | PanelKind::Cells { table, .. } => table.clone(),
    });
    Ok(Panel { title, span, kind })
}
