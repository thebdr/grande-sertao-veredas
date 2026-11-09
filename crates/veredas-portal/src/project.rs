//! The project file (contract: the project clause): one TOML file, `veredas-project`, stating
//! what the gateway reads, stores and serves: the log sources with their timestamp rules, the
//! capture rules, the OPC sources with their rules, the telegram channels (their channel file
//! and catalog beside it) with their reply rules, the project's SQL views, the session's start,
//! store and switch, and the server's address. Its header is read by core's reader
//! ([`veredas_core::schema`]) before the body, so another format or an unknown version is
//! refused by name and never guessed at, and a misspelled key anywhere is refused. Everything
//! the file says is checked here, before anything runs: the executable's `--check` is this
//! reader. The only older form is Veredas 1's JSON project folder, read into this file by
//! [`from_v1`] once, as text the operator keeps and edits.
//!
//! The declarations the session reads (a log source, a timestamp rule, a capture rule) live
//! here too, so the light build, which has no session, still reads the project.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use veredas_core::schema::{self, HeaderError};
use veredas_core::telegram::{Role, catalog, schema as channel_schema};
use veredas_core::{SeriesType, SourceId};

use crate::capture::CaptureRule;
use crate::channel::{self, ChannelConfig, ReplyRule};
use crate::family::{ActiveMode, FamilySpec};
use crate::opc::{self, OpcSourceConfig};
use crate::timestamp::TimestampSpec;

/// The file's format, as its header states it.
pub const FORMAT: &str = "veredas-project";
/// The one version this reader knows.
pub const VERSION: u32 = 1;
/// What the reader calls its file in a refusal.
pub const NOUN: &str = "a project file";
/// Silence after which a live source reads as idle, when the file says nothing.
pub const DEFAULT_IDLE_MS: i64 = 20_000;
/// Where a listening channel binds when the file says nothing.
pub const DEFAULT_BIND: &str = "0.0.0.0";

// ---- the declarations the session reads ------------------------------------------------------

/// The stamp a log line carries: a pattern locating it (its `ts` group) and the chrono formats
/// to try, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimestampConfig {
    pub pattern: String,
    pub formats: Vec<String>,
}

/// Which file of a family is the one being written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveConfig {
    /// The member with the greatest (date, index) key.
    Newest,
    /// The member of this constant name.
    Fixed(String),
}

/// What a log source reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogKind {
    /// One growing file.
    File { path: String },
    /// A rotating family in a folder, its members named by a pattern.
    Family { dir: String, pattern: String, date_format: Option<String>, active: ActiveConfig, recursive: bool },
}

/// A log source, as the project declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSourceConfig {
    pub id: String,
    pub display_name: String,
    pub kind: LogKind,
    pub timestamp: Option<TimestampConfig>,
}

/// A capture rule, as the project declares it: a table named for the rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRuleConfig {
    pub name: String,
    pub pattern: String,
    /// The one log source the rule reads; absent, every log source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// A group's type (`int`, `float`, `text`, `time`); text when absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, String>,
    /// The group that is the identity key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

// ---- the project --------------------------------------------------------------------------------

/// Where a session starts in the logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// Live: every log followed from its end.
    Live,
    /// From the day's midnight, on the plant's clock.
    Midnight,
    /// From this many hours before now.
    LastHours(u32),
    /// From a point in time.
    At(NaiveDateTime),
}

impl Start {
    /// The start point on the plant's clock, or none for a live start.
    pub fn resolve(&self, now: NaiveDateTime) -> Option<NaiveDateTime> {
        match self {
            Start::Live => None,
            Start::Midnight => Some(now.date().and_hms_opt(0, 0, 0).unwrap_or(now)),
            Start::LastHours(h) => Some(now - chrono::Duration::hours((*h).max(1) as i64)),
            Start::At(t) => Some(*t),
        }
    }
}

/// Where the session's store lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Store {
    Ram,
    /// A database file (resolved against the project's folder) and the engine's memory ceiling
    /// in megabytes, 0 for the engine's default.
    Disk { path: PathBuf, memory_limit_mb: u32 },
}

/// A project as read: every reference loaded, every rule compiled once to prove it compiles.
#[derive(Clone)]
pub struct Project {
    pub name: String,
    pub start: Start,
    pub store: Store,
    pub writes_armed: bool,
    pub idle_after_ms: i64,
    /// The server's address, when the project serves.
    pub server: Option<SocketAddr>,
    pub logs: Vec<LogSourceConfig>,
    pub rules: Vec<CaptureRuleConfig>,
    pub opc: Vec<OpcSourceConfig>,
    pub channels: Vec<ChannelConfig>,
    /// The project's views: a name and its SELECT.
    pub tables: Vec<(String, String)>,
}

/// Why a project file was refused. The header's refusals read like every other file's.
#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("not a project file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `{FORMAT}`")]
    WrongFormat(String),
    #[error("version {0} of `{FORMAT}` is not known here (this reader knows {VERSION})")]
    UnknownVersion(u32),
    #[error("{0}")]
    Invalid(String),
    #[error("{place}: cannot read {}: {source}", path.display())]
    Read { place: String, path: PathBuf, source: std::io::Error },
    #[error("{place} ({}): {reason}", path.display())]
    Referenced { place: String, path: PathBuf, reason: String },
}

fn invalid(msg: impl Into<String>) -> ProjectError {
    ProjectError::Invalid(msg.into())
}

/// What a project claims once: every source id across its logs, OPC sources and channels, and
/// every table name across its capture rules, OPC rules, catalog messages and views, compared
/// without regard to case (the store compares names so, C-041). The session (C-043) claims
/// through this same type for a configuration built by hand, so the two never drift.
#[derive(Default)]
pub struct Claims {
    ids: BTreeSet<String>,
    /// (the name lowered, its owner).
    tables: Vec<(String, String)>,
}

/// A claim refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clash {
    EmptyId { kind: String },
    DuplicateId(String),
    Table { name: String, first: String, second: String },
}

impl std::fmt::Display for Clash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Clash::EmptyId { kind } => write!(f, "a {kind} has an empty id"),
            Clash::DuplicateId(id) => write!(f, "two sources share the id `{id}`"),
            Clash::Table { name, first, second } => write!(f, "two tables would be named `{name}`: {first} and {second}"),
        }
    }
}

impl Claims {
    /// Claim a source id (trimmed, returned): empty or taken, a clash. `kind` names the source in
    /// the text: `log`, `opc source`, `channel`.
    pub fn source(&mut self, kind: &str, id: &str) -> Result<String, Clash> {
        let id = id.trim().to_string();
        if id.is_empty() {
            return Err(Clash::EmptyId { kind: kind.to_string() });
        }
        if !self.ids.insert(id.clone()) {
            return Err(Clash::DuplicateId(id));
        }
        Ok(id)
    }

    /// Claim a table name for `owner` (`capture rule `x``, `OPC rule `x` of source `y``,
    /// `message `x` of channel `y``, `table `x``): taken, whatever the case, a clash naming both.
    pub fn table(&mut self, name: &str, owner: String) -> Result<(), Clash> {
        let name = name.trim();
        let lowered = name.to_ascii_lowercase();
        if let Some((_, first)) = self.tables.iter().find(|(n, _)| *n == lowered) {
            return Err(Clash::Table { name: name.to_string(), first: first.clone(), second: owner });
        }
        self.tables.push((lowered, owner));
        Ok(())
    }
}

// ---- the file --------------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    format: String,
    version: u32,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<SessionFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<ServerFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    log: Vec<LogFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rule: Vec<CaptureRuleConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    opc: Vec<OpcSourceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    channel: Vec<ChannelFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    table: Vec<TableFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<StartFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    store: Option<StoreFile>,
    #[serde(default)]
    writes_armed: bool,
    #[serde(default = "default_idle")]
    idle_after_ms: i64,
}

/// The table absent: every default, the idle window included (a derived default would make it 0).
impl Default for SessionFile {
    fn default() -> Self {
        SessionFile { start: None, store: None, writes_armed: false, idle_after_ms: DEFAULT_IDLE_MS }
    }
}

fn default_idle() -> i64 {
    DEFAULT_IDLE_MS
}

#[derive(Serialize)]
#[serde(untagged)]
enum StartFile {
    /// `midnight`, or a date-time.
    Text(String),
    LastHours { last_hours: u32 },
}

#[derive(Serialize)]
#[serde(untagged)]
enum StoreFile {
    /// `ram`.
    Named(String),
    Disk {
        disk: String,
        #[serde(default)]
        memory_limit_mb: u32,
    },
}

// A key that takes a word or an inline table is read key by key, never through an untagged enum:
// that would swallow a misspelled key's name into "did not match any variant", where the file's
// rule is that every misspelled key, anywhere, is refused with its name, in the same words as
// the rest (serde's own, with the parser's place).

/// Every key of an inline table is one of the allowed, or refused by name.
fn only_keys<E: serde::de::Error>(t: &toml::Table, allowed: &'static [&'static str]) -> Result<(), E> {
    match t.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(E::unknown_field(k, allowed)),
        None => Ok(()),
    }
}

/// A required string of an inline table.
fn string_at<E: serde::de::Error>(t: &toml::Table, key: &'static str) -> Result<String, E> {
    match t.get(key) {
        None => Err(E::missing_field(key)),
        Some(toml::Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(E::invalid_type(unexpected(other), &"a string")),
    }
}

/// A whole number of an inline table, a `u32`, with its default when it may be left out.
fn u32_at<E: serde::de::Error>(t: &toml::Table, key: &'static str, default: Option<u32>) -> Result<u32, E> {
    match t.get(key) {
        None => default.ok_or_else(|| E::missing_field(key)),
        Some(toml::Value::Integer(i)) => u32::try_from(*i).map_err(|_| E::invalid_value(serde::de::Unexpected::Signed(*i), &"a whole number from 0 to 4294967295")),
        Some(other) => Err(E::invalid_type(unexpected(other), &"a whole number")),
    }
}

fn unexpected(v: &toml::Value) -> serde::de::Unexpected<'_> {
    match v {
        toml::Value::String(s) => serde::de::Unexpected::Str(s),
        toml::Value::Integer(i) => serde::de::Unexpected::Signed(*i),
        toml::Value::Float(f) => serde::de::Unexpected::Float(*f),
        toml::Value::Boolean(b) => serde::de::Unexpected::Bool(*b),
        toml::Value::Datetime(_) => serde::de::Unexpected::Other("a date-time"),
        toml::Value::Array(_) => serde::de::Unexpected::Seq,
        toml::Value::Table(_) => serde::de::Unexpected::Map,
    }
}

impl<'de> Deserialize<'de> for StartFile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match toml::Value::deserialize(d)? {
            toml::Value::String(s) => Ok(StartFile::Text(s)),
            toml::Value::Table(t) => {
                only_keys(&t, &["last_hours"])?;
                Ok(StartFile::LastHours { last_hours: u32_at(&t, "last_hours", None)? })
            }
            other => Err(serde::de::Error::invalid_type(unexpected(&other), &"`midnight`, a date-time, a date, or { last_hours = N }")),
        }
    }
}

impl<'de> Deserialize<'de> for StoreFile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match toml::Value::deserialize(d)? {
            toml::Value::String(s) => Ok(StoreFile::Named(s)),
            toml::Value::Table(t) => {
                only_keys(&t, &["disk", "memory_limit_mb"])?;
                Ok(StoreFile::Disk { disk: string_at(&t, "disk")?, memory_limit_mb: u32_at(&t, "memory_limit_mb", Some(0))? })
            }
            other => Err(serde::de::Error::invalid_type(unexpected(&other), &"`ram`, or { disk = \"<file>\", memory_limit_mb = N }")),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerFile {
    bind: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogFile {
    id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<ActiveFile>,
    #[serde(default)]
    recursive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timestamp: Option<TimestampConfig>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ActiveFile {
    /// `newest`.
    Named(String),
    Fixed { fixed: String },
}

impl<'de> Deserialize<'de> for ActiveFile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match toml::Value::deserialize(d)? {
            toml::Value::String(s) => Ok(ActiveFile::Named(s)),
            toml::Value::Table(t) => {
                only_keys(&t, &["fixed"])?;
                Ok(ActiveFile::Fixed { fixed: string_at(&t, "fixed")? })
            }
            other => Err(serde::de::Error::invalid_type(unexpected(&other), &"`newest`, or { fixed = \"<name>\" }")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RoleFile {
    Listen,
    Connect,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelFile {
    source_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    display_name: String,
    role: RoleFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    port: u16,
    channel: String,
    catalog: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    replies: Vec<ReplyRule>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct TableFile {
    name: String,
    sql: String,
}

/// A path of the file, against the file's folder.
fn resolve(base: &Path, p: &str) -> PathBuf {
    let p = Path::new(p.trim());
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// Read a start point as the file writes it.
fn parse_start(text: &str) -> Result<Start, ProjectError> {
    let s = text.trim();
    if s.eq_ignore_ascii_case("midnight") {
        return Ok(Start::Midnight);
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(Start::At(t));
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(t) = d.and_hms_opt(0, 0, 0)
    {
        return Ok(Start::At(t));
    }
    Err(invalid(format!("session.start: `{s}` is neither `midnight`, a date-time `YYYY-MM-DD HH:MM[:SS]`, a date `YYYY-MM-DD`, nor {{ last_hours = N }}")))
}

/// Veredas 1's absolute start as its operator typed it (a free text there): a date-time with a
/// space or a `T` between, seconds and a fraction optional, or a date. Anything else is no
/// start, and the import says so rather than guess.
#[cfg(feature = "store")]
fn v1_absolute(text: &str) -> Option<NaiveDateTime> {
    let s = text.trim();
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(t);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Whether a text is a date-time carrying an offset (`Z`, `+02:00`, `+0200`), to the second or
/// to the minute, with a `T` or a space: a date-time, but not on the plant's clock.
#[cfg(feature = "store")]
fn names_an_offset(text: &str) -> bool {
    let s = text.trim();
    let s = match s.strip_suffix(['Z', 'z']) {
        Some(head) => format!("{head}+00:00"),
        None => s.to_string(),
    };
    // `%#z` reads an offset with or without its minutes, with or without the colon.
    let formats = ["%Y-%m-%dT%H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%dT%H:%M:%S%#z", "%Y-%m-%d %H:%M:%S%#z", "%Y-%m-%dT%H:%M%#z", "%Y-%m-%d %H:%M%#z"];
    formats.iter().any(|f| chrono::DateTime::parse_from_str(&s, f).is_ok())
}


/// Read and check a project file whose referenced files live under `base` (the file's folder).
/// Nothing runs: every rule is compiled once, every referenced file loaded, every path
/// resolved, so what comes back can start.
pub fn load(text: &str, base: &Path) -> Result<Project, ProjectError> {
    schema::check(text, NOUN, FORMAT, VERSION).map_err(|e| match e {
        HeaderError::NotToml { reason, .. } => ProjectError::NotToml(reason),
        HeaderError::WrongFormat { found, .. } => ProjectError::WrongFormat(found),
        HeaderError::UnknownVersion { found, .. } => ProjectError::UnknownVersion(found),
        other => invalid(other.to_string()),
    })?;
    let file: FileV1 = toml::from_str(text).map_err(|e| ProjectError::NotToml(e.to_string()))?;
    let name = file.name.trim().to_string();
    if name.is_empty() {
        return Err(invalid("`name` is empty: the project names its session"));
    }

    // The session.
    let session = file.session.unwrap_or_default();
    let start = match session.start {
        None => Start::Live,
        Some(StartFile::Text(t)) => parse_start(&t)?,
        Some(StartFile::LastHours { last_hours }) => {
            if last_hours == 0 {
                return Err(invalid("session.start: `last_hours` is 0; name at least one hour, or leave `start` out for a live start"));
            }
            Start::LastHours(last_hours)
        }
    };
    let store = match session.store {
        None => Store::Ram,
        Some(StoreFile::Named(n)) if n.trim().eq_ignore_ascii_case("ram") => Store::Ram,
        Some(StoreFile::Named(n)) => return Err(invalid(format!("session.store: `{}` is neither `ram` nor {{ disk = \"<file>\" }}", n.trim()))),
        Some(StoreFile::Disk { disk, memory_limit_mb }) => {
            if disk.trim().is_empty() {
                return Err(invalid("session.store: `disk` is empty: name the database file"));
            }
            Store::Disk { path: resolve(base, &disk), memory_limit_mb }
        }
    };
    if session.idle_after_ms < 1 {
        return Err(invalid(format!("session.idle_after_ms: {} is under 1 ms", session.idle_after_ms)));
    }
    let server = match file.server {
        None => None,
        Some(s) => Some(
            s.bind
                .trim()
                .parse::<SocketAddr>()
                .map_err(|_| invalid(format!("server.bind: `{}` is not an address `host:port`", s.bind.trim())))?,
        ),
    };

    // Every source id once and every table name once, through the claims the session makes too.
    let mut claims = Claims::default();
    let claimed = |c: Clash| invalid(c.to_string());

    // The logs: a file, or a family (a folder and a pattern), never both nor neither.
    let mut logs: Vec<LogSourceConfig> = Vec::new();
    for l in &file.log {
        let id = claims.source("log", &l.id).map_err(claimed)?;
        let place = format!("log `{id}`");
        let display_name = if l.display_name.trim().is_empty() { id.clone() } else { l.display_name.trim().to_string() };
        let kind = match (&l.file, &l.dir, &l.pattern) {
            (Some(f), None, None) => {
                if l.date_format.is_some() || l.active.is_some() || l.recursive {
                    return Err(invalid(format!("{place}: `date_format`, `active` and `recursive` belong to a family (`dir` and `pattern`), not to a `file`")));
                }
                if f.trim().is_empty() {
                    return Err(invalid(format!("{place}: `file` is empty")));
                }
                LogKind::File { path: resolve(base, f).to_string_lossy().into_owned() }
            }
            (None, Some(d), Some(p)) => {
                let date_format = l.date_format.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                let active = match &l.active {
                    None => ActiveConfig::Newest,
                    Some(ActiveFile::Named(n)) if n.trim().eq_ignore_ascii_case("newest") => ActiveConfig::Newest,
                    Some(ActiveFile::Named(n)) => return Err(invalid(format!("{place}: `active` is `{}`, neither `newest` nor {{ fixed = \"<name>\" }}", n.trim()))),
                    Some(ActiveFile::Fixed { fixed }) => {
                        if fixed.trim().is_empty() {
                            return Err(invalid(format!("{place}: `active.fixed` is empty")));
                        }
                        ActiveConfig::Fixed(fixed.trim().to_string())
                    }
                };
                let dir = resolve(base, d).to_string_lossy().into_owned();
                // Compiled once here, to refuse a bad pattern or a date group without its format
                // by name; the tail compiles it again when it starts.
                let mode = match &active {
                    ActiveConfig::Newest => ActiveMode::NewestByOrder,
                    ActiveConfig::Fixed(n) => ActiveMode::Fixed(n.clone()),
                };
                FamilySpec::new(&dir, p.trim(), date_format.as_deref(), mode).map_err(|e| invalid(format!("{place}: {e}")))?;
                LogKind::Family { dir, pattern: p.trim().to_string(), date_format, active, recursive: l.recursive }
            }
            (None, None, None) => return Err(invalid(format!("{place}: a log is a `file`, or a family (`dir` and `pattern`); this one is neither"))),
            (Some(_), _, _) => return Err(invalid(format!("{place}: a log is a `file` or a family (`dir` and `pattern`), not both"))),
            (None, Some(_), None) => return Err(invalid(format!("{place}: a family needs its `pattern` beside `dir`"))),
            (None, None, Some(_)) => return Err(invalid(format!("{place}: a family needs its `dir` beside `pattern`"))),
        };
        if let Some(t) = &l.timestamp {
            TimestampSpec::new(t.pattern.trim(), t.formats.iter().map(|f| f.trim().to_string())).map_err(|e| invalid(format!("{place}: timestamp: {e}")))?;
        }
        logs.push(LogSourceConfig { id, display_name, kind, timestamp: l.timestamp.clone() });
    }
    // A start in the past is located in a family by its stamps: a family without a timestamp rule
    // cannot be started there (the session would refuse it; so does the file).
    if start != Start::Live {
        for l in &logs {
            if matches!(l.kind, LogKind::Family { .. }) && l.timestamp.is_none() {
                return Err(invalid(format!("log `{}`: a start point in the past needs a timestamp rule, to find the line to start at", l.id)));
            }
        }
    }

    // The capture rules, each compiled once.
    let mut rules: Vec<CaptureRuleConfig> = Vec::new();
    for r in &file.rule {
        let name = r.name.trim();
        opc::check_name(name).map_err(|reason| invalid(format!("rule `{name}`: {reason}")))?;
        let mut types: HashMap<String, SeriesType> = HashMap::new();
        for (group, ty) in &r.types {
            let t = match ty.trim() {
                "" => SeriesType::Text,
                t => opc::parse_type(t).ok_or_else(|| invalid(format!("rule `{name}`: group `{group}`: unknown type `{t}` (int, float, text or time)")))?,
            };
            types.insert(group.clone(), t);
        }
        let scope = r.scope.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(s) = scope
            && !logs.iter().any(|l| l.id == s)
        {
            return Err(invalid(format!("rule `{name}`: the scope `{s}` is no log of this project")));
        }
        let compiled = CaptureRule::new(name, r.pattern.trim(), scope.map(SourceId::new), &types, r.key.as_deref().map(str::trim)).map_err(|e| invalid(format!("rule `{name}`: {e}")))?;
        for (series, _) in compiled.series() {
            opc::check_name(series).map_err(|reason| invalid(format!("rule `{name}`: group `{series}`: {reason}")))?;
        }
        claims.table(name, format!("capture rule `{name}`")).map_err(claimed)?;
        rules.push(CaptureRuleConfig {
            name: name.to_string(),
            pattern: r.pattern.trim().to_string(),
            scope: scope.map(String::from),
            types: r.types.clone(),
            key: r.key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(String::from),
        });
    }

    // The OPC sources, compiled once without their secrets: the password is read from the
    // environment when the session starts, and the check verifies everything but its presence.
    let mut opcs: Vec<OpcSourceConfig> = Vec::new();
    for o in &file.opc {
        let id = claims.source("opc source", &o.source_id).map_err(claimed)?;
        let spec = opc::compile(o, &|_| Some(String::from("*"))).map_err(|e| invalid(format!("opc `{id}`: {e}")))?;
        for rule in &spec.rules {
            claims.table(&rule.name, format!("OPC rule `{}` of source `{id}`", rule.name)).map_err(claimed)?;
        }
        let mut cfg = o.clone();
        cfg.source_id = id.clone();
        if cfg.display_name.trim().is_empty() {
            cfg.display_name = id;
        }
        opcs.push(cfg);
    }

    // The channels: the channel file and the catalog read from beside the project, the reply
    // rules checked against the catalog.
    let mut channels: Vec<ChannelConfig> = Vec::new();
    for c in &file.channel {
        let id = claims.source("channel", &c.source_id).map_err(claimed)?;
        let place = format!("channel `{id}`");
        let role = match c.role {
            RoleFile::Listen => {
                if c.host.is_some() {
                    return Err(invalid(format!("{place}: `host` belongs to a connecting channel; a listening one has `bind`")));
                }
                let bind = c.bind.as_deref().map(str::trim).filter(|b| !b.is_empty()).unwrap_or(DEFAULT_BIND).to_string();
                Role::Listen { bind, port: c.port }
            }
            RoleFile::Connect => {
                if c.bind.is_some() {
                    return Err(invalid(format!("{place}: `bind` belongs to a listening channel; a connecting one has `host`")));
                }
                let host = c.host.as_deref().map(str::trim).filter(|h| !h.is_empty()).ok_or_else(|| invalid(format!("{place}: a connecting channel names its `host`")))?;
                Role::Connect { host: host.to_string(), port: c.port }
            }
        };
        let read = |what: &str, p: &str| -> Result<(PathBuf, String), ProjectError> {
            let path = resolve(base, p);
            let text = std::fs::read_to_string(&path).map_err(|source| ProjectError::Read { place: format!("{place}: its {what}"), path: path.clone(), source })?;
            Ok((path, text))
        };
        let (channel_path, channel_text) = read("channel file", &c.channel)?;
        let spec = channel_schema::load(&channel_text).map_err(|e| ProjectError::Referenced { place: format!("{place}: its channel file"), path: channel_path, reason: e.to_string() })?;
        let (catalog_path, catalog_text) = read("catalog", &c.catalog)?;
        let cat = catalog::load(&catalog_text).map_err(|e| ProjectError::Referenced { place: format!("{place}: its catalog"), path: catalog_path, reason: e.to_string() })?;
        for reply in &c.replies {
            for (what, message) in [("on", &reply.on), ("send", &reply.send)] {
                if cat.by_name(message.trim()).is_none() {
                    return Err(invalid(format!("{place}: a reply rule's `{what}` names no message `{}` of its catalog", message.trim())));
                }
            }
        }
        let display_name = if c.display_name.trim().is_empty() { id.clone() } else { c.display_name.trim().to_string() };
        let cfg = ChannelConfig { source_id: id, display_name, role, spec, catalog: cat, replies: c.replies.clone() };
        // The channel checked whole, as the session checks it before it spawns: the framing
        // against the channel file, every reply rule against the catalog, every message a table.
        for table in channel::check(&cfg).map_err(|e| invalid(format!("{place}: {e}")))? {
            claims.table(&table.name, format!("message `{}` of channel `{}`", table.name, cfg.source_id)).map_err(claimed)?;
        }
        channels.push(cfg);
    }

    // The views.
    let mut views: Vec<(String, String)> = Vec::new();
    for t in &file.table {
        let name = t.name.trim();
        opc::check_name(name).map_err(|reason| invalid(format!("table `{name}`: {reason}")))?;
        if t.sql.trim().is_empty() {
            return Err(invalid(format!("table `{name}`: `sql` is empty")));
        }
        claims.table(name, format!("table `{name}`")).map_err(claimed)?;
        views.push((name.to_string(), t.sql.trim().to_string()));
    }
    if logs.is_empty() && opcs.is_empty() && channels.is_empty() {
        return Err(invalid("the project has no source: a `[[log]]`, an `[[opc]]` or a `[[channel]]`"));
    }

    Ok(Project { name, start, store, writes_armed: session.writes_armed, idle_after_ms: session.idle_after_ms, server, logs, rules, opc: opcs, channels, tables: views })
}

/// Read a project file from disk; its folder is where its references live.
pub fn load_file(path: &Path) -> Result<Project, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|source| ProjectError::Read { place: "the project file".into(), path: path.to_path_buf(), source })?;
    let base = path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    load(&text, &base)
}

#[cfg(feature = "store")]
impl Project {
    /// The session this project starts, `now` being the plant's clock for a relative start.
    pub fn session_config(&self, now: NaiveDateTime) -> crate::session::SessionConfig {
        use crate::store::Residency;
        let mut cfg = crate::session::SessionConfig::new(self.name.clone());
        cfg.start = self.start.resolve(now);
        cfg.residency = match &self.store {
            Store::Ram => Residency::Ram,
            Store::Disk { path, memory_limit_mb } => Residency::Disk { path: path.clone(), memory_limit_mb: *memory_limit_mb },
        };
        cfg.writes_armed = self.writes_armed;
        cfg.idle_after_ms = self.idle_after_ms;
        cfg.logs = self.logs.clone();
        cfg.rules = self.rules.clone();
        cfg.opc = self.opc.clone();
        cfg.channels = self.channels.clone();
        cfg.tables = self.tables.iter().map(|(name, sql)| crate::store::TableDef { name: name.clone(), sql: sql.clone() }).collect();
        cfg
    }
}

// ---- Veredas 1's project folder ---------------------------------------------------------------

/// A Veredas 1 project folder read into a project file: the text to keep, and what was not
/// carried, by name.
#[cfg(feature = "store")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migrated {
    pub toml: String,
    pub notes: Vec<String>,
}

/// The shapes of Veredas 1's JSON files, as far as the project absorbs them. Unknown keys are
/// ignored: Veredas 1 wrote more than the gateway keeps (its screens, its grep, its sequencer).
#[cfg(feature = "store")]
mod v1 {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    fn one() -> u32 {
        1
    }
    fn yes() -> bool {
        true
    }
    fn poll() -> u32 {
        1_000
    }
    fn float() -> String {
        "float".into()
    }
    fn four() -> u32 {
        4
    }

    #[derive(Deserialize, Default)]
    pub struct Manifest {
        #[serde(default = "one")]
        pub format: u32,
        #[serde(default)]
        pub name: String,
    }

    #[derive(Deserialize)]
    pub struct Source {
        pub id: String,
        #[serde(default)]
        pub display_name: String,
        #[serde(default = "yes")]
        pub enabled: bool,
        pub kind: Kind,
        #[serde(default)]
        pub ts: Ts,
    }

    #[derive(Deserialize)]
    pub enum Kind {
        SingleFile {
            path: String,
        },
        Family {
            dir: String,
            pattern: String,
            #[serde(default)]
            date_format: String,
            active: Active,
            #[serde(default)]
            recursive: bool,
        },
        OpcUa {
            endpoint_url: String,
            #[serde(default)]
            security: serde_json::Value,
            #[serde(default)]
            auth: Auth,
            #[serde(default = "poll")]
            poll_ms: u32,
        },
        Telegram {
            #[serde(default)]
            channel: String,
            #[serde(default)]
            address: String,
        },
    }

    #[derive(Deserialize)]
    pub enum Active {
        NewestByOrder,
        Fixed { name: String },
    }

    #[derive(Deserialize, Default)]
    pub enum Auth {
        #[default]
        Anonymous,
        UserPass {
            username: String,
        },
        Certificate,
    }

    #[derive(Deserialize, Default)]
    pub struct Ts {
        #[serde(default)]
        pub pattern: String,
        /// One chrono format per line.
        #[serde(default)]
        pub formats: String,
    }

    #[derive(Deserialize, Default)]
    pub struct Rules {
        #[serde(default)]
        pub extracts: Vec<Extract>,
        #[serde(default)]
        pub opc: Vec<OpcRule>,
    }

    #[derive(Deserialize)]
    pub struct Extract {
        pub name: String,
        pub pattern: String,
        #[serde(default)]
        pub scope: String,
        #[serde(default)]
        pub types: BTreeMap<String, String>,
        #[serde(default)]
        pub key_group: String,
        #[serde(default = "yes")]
        pub enabled: bool,
    }

    #[derive(Deserialize)]
    pub struct OpcRule {
        pub name: String,
        pub source_id: String,
        #[serde(default = "yes")]
        pub enabled: bool,
        #[serde(default)]
        pub store_on_change: bool,
        #[serde(default)]
        pub tags: Vec<Tag>,
    }

    #[derive(Deserialize)]
    pub struct Tag {
        pub node_id: String,
        #[serde(default)]
        pub series: String,
        #[serde(default = "float")]
        pub ty: String,
        #[serde(default)]
        pub writable: bool,
        #[serde(default)]
        pub bits: Vec<Bit>,
    }

    #[derive(Deserialize)]
    pub struct Bit {
        #[serde(default)]
        pub bit: u8,
        #[serde(default)]
        pub width: u8,
        #[serde(default)]
        pub series: String,
    }

    #[derive(Deserialize)]
    pub struct Table {
        pub name: String,
        #[serde(default)]
        pub sql: String,
        #[serde(default = "yes")]
        pub enabled: bool,
        #[serde(default)]
        pub csv_path: String,
        #[serde(default)]
        pub csv_decimal_comma: bool,
        #[serde(default)]
        pub csv_delim: String,
    }

    #[derive(Deserialize, Default)]
    pub struct Session {
        #[serde(default)]
        pub start: Option<Start>,
        #[serde(default)]
        pub store: Option<Store>,
    }

    #[derive(Deserialize)]
    pub struct Start {
        pub kind: String,
        #[serde(default = "four")]
        pub last_hours: u32,
        #[serde(default)]
        pub absolute: String,
    }

    #[derive(Deserialize)]
    pub struct Store {
        #[serde(default)]
        pub mode: String,
        #[serde(default)]
        pub disk_dir: String,
        #[serde(default)]
        pub memory_limit_mb: u32,
        #[serde(default)]
        pub auto_export_mins: u32,
        #[serde(default)]
        pub export_dir: String,
    }

    #[derive(Deserialize, Default)]
    pub struct Commands {
        #[serde(default, alias = "allow_dashboard_writes")]
        pub allow_writes: bool,
    }
}

/// Read a Veredas 1 project folder (`veredas.json` and the files beside it) into a project file.
/// What the gateway keeps is carried; what it does not (the screens, the grep, the command
/// sequencer, a disabled part, a telegram source that had no catalog, a security mode never
/// implemented) is named in the notes, never guessed at. The passwords Veredas 1 kept in the
/// machine's credential store become `password_env` names the operator sets.
#[cfg(feature = "store")]
pub fn from_v1(dir: &Path) -> Result<Migrated, ProjectError> {
    use crate::opc::{OpcBitConfig, OpcRuleConfig, OpcTagConfig};
    use crate::store::{CsvOptions, csv_scan_sql};

    fn read<T: serde::de::DeserializeOwned + Default>(dir: &Path, name: &str, required: bool) -> Result<T, ProjectError> {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Ok(T::default()),
            Err(source) => Err(ProjectError::Read { place: format!("the Veredas 1 project's {name}"), path, source }),
            Ok(text) => serde_json::from_str(&text).map_err(|e| ProjectError::Referenced { place: format!("the Veredas 1 project's {name}"), path, reason: e.to_string() }),
        }
    }

    let manifest: v1::Manifest = read(dir, "veredas.json", true)?;
    if manifest.format != 1 {
        return Err(invalid(format!("veredas.json: format {} is not the Veredas 1 project format (1)", manifest.format)));
    }
    let sources: Vec<v1::Source> = read(dir, "sources.json", false)?;
    let rules: v1::Rules = read(dir, "rules.json", false)?;
    let tables: Vec<v1::Table> = read(dir, "tables.json", false)?;
    let session: v1::Session = read(dir, "session.json", false)?;
    let commands: v1::Commands = read(dir, "commands.json", false)?;
    let mut notes: Vec<String> = Vec::new();
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = if manifest.name.trim().is_empty() { folder } else { manifest.name.trim().to_string() };

    let timestamp = |ts: &v1::Ts| -> Option<TimestampConfig> {
        let formats: Vec<String> = ts.formats.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
        (!ts.pattern.trim().is_empty() && !formats.is_empty()).then(|| TimestampConfig { pattern: ts.pattern.trim().to_string(), formats })
    };
    let mut log: Vec<LogFile> = Vec::new();
    let mut opc: Vec<OpcSourceConfig> = Vec::new();
    for s in &sources {
        if !s.enabled {
            notes.push(format!("source `{}` was disabled: not carried (a project has no disabled parts; add it back when it is wanted)", s.id));
            continue;
        }
        let display_name = s.display_name.trim().to_string();
        match &s.kind {
            v1::Kind::SingleFile { path } => log.push(LogFile {
                id: s.id.clone(),
                display_name,
                file: Some(path.clone()),
                dir: None,
                pattern: None,
                date_format: None,
                active: None,
                recursive: false,
                timestamp: timestamp(&s.ts),
            }),
            v1::Kind::Family { dir, pattern, date_format, active, recursive } => log.push(LogFile {
                id: s.id.clone(),
                display_name,
                file: None,
                dir: Some(dir.clone()),
                pattern: Some(pattern.clone()),
                date_format: (!date_format.trim().is_empty()).then(|| date_format.trim().to_string()),
                active: match active {
                    v1::Active::NewestByOrder => None,
                    v1::Active::Fixed { name } => Some(ActiveFile::Fixed { fixed: name.clone() }),
                },
                recursive: *recursive,
                timestamp: timestamp(&s.ts),
            }),
            v1::Kind::OpcUa { endpoint_url, security, auth, poll_ms } => {
                // Whatever shape the mode took in the JSON (a word, or an object naming a policy),
                // anything but `None` is named: the gateway speaks to the server unsecured.
                let sec = match security {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(word) => (word != "None").then(|| word.clone()),
                    other => Some(other.to_string()),
                };
                if let Some(sec) = sec {
                    notes.push(format!("opc `{}`: security `{sec}` was never implemented; the source is carried without it", s.id));
                }
                let (user, password_env) = match auth {
                    v1::Auth::Anonymous => (None, None),
                    v1::Auth::UserPass { username } => {
                        let var: String = format!("{}_PASSWORD", s.id.to_ascii_uppercase()).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
                        notes.push(format!("opc `{}`: the password lived in the machine's credential store; set the variable `{var}` in the environment", s.id));
                        (Some(username.clone()), Some(var))
                    }
                    v1::Auth::Certificate => {
                        notes.push(format!("opc `{}`: certificate authentication was never implemented; the source is carried anonymous", s.id));
                        (None, None)
                    }
                };
                let mut source = OpcSourceConfig { source_id: s.id.clone(), display_name, endpoint_url: endpoint_url.clone(), user, password_env, poll_ms: (*poll_ms).max(1) as u64, rules: Vec::new() };
                for r in rules.opc.iter().filter(|r| r.source_id == s.id) {
                    if !r.enabled {
                        notes.push(format!("opc rule `{}` was disabled: not carried", r.name));
                        continue;
                    }
                    let mut tags = Vec::with_capacity(r.tags.len());
                    for t in &r.tags {
                        let mut ty = if t.ty.trim().is_empty() { "float".to_string() } else { t.ty.clone() };
                        // Veredas 1 accepted bits on a float it could never carve; bits are carved
                        // out of an int word (C-040), so such a tag is carried as an int, named.
                        if !t.bits.is_empty() && ty == "float" {
                            notes.push(format!(
                                "opc rule `{}`, tag `{}`: bits are carved out of an int word; its type `float` became `int`",
                                r.name, t.node_id
                            ));
                            ty = "int".into();
                        }
                        tags.push(OpcTagConfig {
                            node_id: t.node_id.clone(),
                            series: t.series.clone(),
                            ty,
                            writable: t.writable,
                            bits: t.bits.iter().map(|b| OpcBitConfig { bit: b.bit, width: b.width.max(1), series: b.series.clone() }).collect(),
                        });
                    }
                    source.rules.push(OpcRuleConfig { name: r.name.clone(), store_on_change: r.store_on_change, tags });
                }
                // A source without a rule is no source the file can hold (an `[[opc]]` needs its
                // rules), so it is not carried, and the note says what to write.
                if source.rules.is_empty() {
                    notes.push(format!("opc `{}`: no rule mapped its tags: not carried (write its [[opc]] with its rules when it is wanted)", s.id));
                    continue;
                }
                opc.push(source);
            }
            v1::Kind::Telegram { channel, address } => notes.push(format!(
                "source `{}` (a telegram channel `{channel}` at `{address}`) is not carried: a channel needs its channel file and catalog (docs/channel-schema.md, docs/catalog-schema.md) written beside the project, then a [[channel]]",
                s.id
            )),
        }
    }
    for r in rules.opc.iter().filter(|r| !sources.iter().any(|s| s.id == r.source_id)) {
        notes.push(format!("opc rule `{}` names a source `{}` the project does not have: not carried", r.name, r.source_id));
    }
    // What is not carried takes its dependents with it, named: a rule scoped to a log that was
    // left out would make the file refuse itself.
    let carried_logs: HashSet<&str> = log.iter().map(|l| l.id.as_str()).collect();
    let mut rule: Vec<CaptureRuleConfig> = Vec::new();
    for e in &rules.extracts {
        if !e.enabled {
            notes.push(format!("rule `{}` was disabled: not carried", e.name));
            continue;
        }
        let scope = e.scope.trim();
        if !scope.is_empty() && !carried_logs.contains(scope) {
            notes.push(format!("rule `{}`: its scope `{scope}` is a log not carried: not carried (write it back with a scope the project has)", e.name));
            continue;
        }
        rule.push(CaptureRuleConfig {
            name: e.name.clone(),
            pattern: e.pattern.clone(),
            scope: (!e.scope.trim().is_empty()).then(|| e.scope.trim().to_string()),
            types: e.types.clone(),
            key: (!e.key_group.trim().is_empty()).then(|| e.key_group.trim().to_string()),
        });
    }
    let mut pending: Vec<TableFile> = Vec::new();
    for t in &tables {
        if !t.enabled {
            notes.push(format!("table `{}` was disabled: not carried", t.name));
            continue;
        }
        if !t.csv_path.trim().is_empty() {
            let d = t.csv_delim.trim();
            let scan = csv_scan_sql(&t.csv_path, &CsvOptions { delim: (!d.is_empty()).then(|| d.to_string()), decimal_comma: t.csv_decimal_comma });
            pending.push(TableFile { name: t.name.clone(), sql: format!("SELECT * FROM {scan}") });
        } else if !t.sql.trim().is_empty() {
            pending.push(TableFile { name: t.name.clone(), sql: t.sql.trim().to_string() });
        } else {
            notes.push(format!("table `{}` had neither SQL nor a CSV: not carried", t.name));
        }
    }
    // The views are the engine's to judge, below, once the file stands.
    let table = pending;
    let start = match &session.start {
        None => None,
        Some(s) => match s.kind.as_str() {
            "Now" => None,
            "SinceMidnight" => Some(StartFile::Text("midnight".into())),
            "LastHours" => {
                if s.last_hours == 0 {
                    notes.push("session.start: `last_hours` was 0 and is carried as 1 (a start names at least one hour)".into());
                }
                Some(StartFile::LastHours { last_hours: s.last_hours.max(1) })
            }
            "Absolute" => match v1_absolute(&s.absolute) {
                // Written in the file's own spelling, whichever Veredas 1's operator typed.
                Some(t) => Some(StartFile::Text(t.format("%Y-%m-%d %H:%M:%S").to_string())),
                None => {
                    let text = s.absolute.trim();
                    // A date-time with an offset is one, but not on the plant's clock, which is
                    // what this file's times are; it is named as such, not read into a guess.
                    let with_offset = names_an_offset(text);
                    notes.push(if with_offset {
                        format!("session.start: the absolute start `{text}` carries an offset, and this file's times are the plant's clock; set `start` by hand; the session starts live")
                    } else {
                        format!("session.start: the absolute start `{text}` is not a date-time (`YYYY-MM-DD HH:MM[:SS]`, a `T` or a space between) nor a date; the session starts live")
                    });
                    None
                }
            },
            other => {
                notes.push(format!("session.start: kind `{other}` is not known; the session starts live"));
                None
            }
        },
    };
    // The store: Veredas 1 had `Ram` and `Disk`; a mode it never had is named and the store is
    // in memory; a memory limit on an in-memory store, and the auto-export, have no place in
    // this file and are named.
    let store = match &session.store {
        None => None,
        Some(st) => {
            if st.auto_export_mins > 0 || !st.export_dir.trim().is_empty() {
                notes.push(format!(
                    "session.store: the auto-export (every {} minutes to `{}`) is not carried: a snapshot is taken on request (C-041), not on a timer",
                    st.auto_export_mins,
                    st.export_dir.trim()
                ));
            }
            match st.mode.as_str() {
                "Disk" => {
                    let dir = if st.disk_dir.trim().is_empty() { "cache".to_string() } else { st.disk_dir.trim().to_string() };
                    Some(StoreFile::Disk { disk: format!("{dir}/session.duckdb"), memory_limit_mb: st.memory_limit_mb })
                }
                mode => {
                    if !matches!(mode, "Ram" | "") {
                        notes.push(format!("session.store: mode `{mode}` is not known (Veredas 1 had `Ram` and `Disk`); the store is in memory"));
                    }
                    if st.memory_limit_mb > 0 {
                        notes.push(format!("session.store: the memory limit of {} MB is not carried: an in-memory store has none here (a limit belongs to a disk store)", st.memory_limit_mb));
                    }
                    None
                }
            }
        }
    };
    notes.push("not carried, by design: the dashboard layout, the grep, the command sequencer, the pages, the highlights and the rules' skew-monitor threshold (the apps own the screens; a command is composed from the write endpoint; the session's snapshot shows both clocks)".into());
    let file = FileV1 {
        format: FORMAT.into(),
        version: VERSION,
        name,
        session: Some(SessionFile { start, store, writes_armed: commands.allow_writes, idle_after_ms: DEFAULT_IDLE_MS }),
        server: None,
        log,
        rule,
        opc,
        channel: Vec::new(),
        table,
    };
    let mut file = file;
    let render = |file: &FileV1| -> Result<String, ProjectError> {
        let body = toml::to_string_pretty(file).map_err(|e| invalid(format!("writing the project file: {e}")))?;
        Ok(format!("# Read into Veredas 2 from a Veredas 1 project folder: check every path and every secret, then keep this file.\n{body}"))
    };
    let with_notes = |what: &str, e: &dyn std::fmt::Display, notes: &[String]| {
        let notes = notes.iter().map(|n| format!("\nnote: {n}")).collect::<String>();
        invalid(format!("the file read from the folder does not {what}: {e}{notes}"))
    };
    // The file printed loads and its store opens, or nothing is printed: the file is loaded as
    // the gateway loads it, its tables compiled as the session compiles them (with a blank in
    // place of every secret), and an in-memory store opened over them, so the engine that will
    // run the file judges its views over the very tables it creates. A view the engine refuses
    // (a table nothing creates, a column no table has, whatever the shape of its SQL) is left
    // out with the engine's reason, and a view over one of those falls next, until every view
    // stands. A view over a file the engine cannot read where the import runs (a CSV on the
    // plant's machine) is no fault of the file: it is carried as written and named, set aside
    // for the judging, and so is a view that reads it. A folder that leaves no source to carry,
    // or a part the import wrote wrong, is refused with the reason and the notes, never handed
    // over as a file the gateway would then refuse.
    let all: Vec<TableFile> = std::mem::take(&mut file.table);
    let mut dropped: HashSet<String> = HashSet::new();
    let mut unjudged: HashSet<String> = HashSet::new();
    let mut noted: HashSet<String> = HashSet::new();
    let catalog_missing = |reason: &str| reason.strip_prefix("Catalog Error: Table with name ").and_then(|r| r.strip_suffix(" does not exist!")).map(|m| m.trim().to_string());
    loop {
        file.table = all.iter().filter(|t| !dropped.contains(t.name.trim()) && !unjudged.contains(t.name.trim())).cloned().collect();
        let toml = render(&file)?;
        let project = load(&toml, dir).map_err(|e| with_notes("load", &e, &notes))?;
        let cfg = project.session_config(chrono::Local::now().naive_local());
        let tables = crate::session::compile_tables(&cfg, &|_| Some(String::new()), &mut Claims::default()).map_err(|e| with_notes("start", &e, &notes))?;
        // Two judgings of every view at once, each refusal the first line of the engine's
        // reason. With every set-aside view standing in as an empty table of its name, the
        // engine binds whatever else a reader names, so a table nothing creates is its own
        // Catalog Error wherever it stands in the SQL; without them, a Catalog Error naming a
        // set-aside view tells which views read one, to be named as not judged here.
        let mut with_stubs = cfg.tables.clone();
        with_stubs.extend(unjudged.iter().map(|n| crate::store::TableDef { name: n.clone(), sql: "SELECT NULL AS placeholder WHERE false".into() }));
        let judge = |defs: &[crate::store::TableDef]| -> Result<HashMap<String, String>, ProjectError> {
            Ok(crate::store::DuckStore::judge_views(&tables.specs, defs)
                .map_err(|e| with_notes("open its store", &e, &notes))?
                .into_iter()
                .map(|(n, m)| (n.trim().to_string(), m.lines().next().unwrap_or("").trim().to_string()))
                .collect())
        };
        let refused = judge(&with_stubs)?;
        let alone = judge(&cfg.tables)?;
        // Without the stand-ins, a Catalog Error naming a set-aside view, or a view that in turn
        // reads one, marks a view as over a file not here, through any number of views between.
        let mut over_file: HashSet<String> = unjudged.clone();
        let mut reads: HashMap<String, String> = HashMap::new();
        loop {
            let before = reads.len();
            for (name, reason) in &alone {
                if !reads.contains_key(name)
                    && let Some(m) = catalog_missing(reason).filter(|m| over_file.iter().any(|u| u.eq_ignore_ascii_case(m)))
                {
                    over_file.insert(name.clone());
                    reads.insert(name.clone(), m);
                }
            }
            if reads.len() == before {
                break;
            }
        }
        let over_file_note = |name: &str, m: &str| format!("table `{name}`: it reads `{m}`, a view over a file not here: carried as written; check it with the file");
        if refused.is_empty() {
            for def in &cfg.tables {
                let name = def.name.trim().to_string();
                if let Some(m) = reads.get(&name)
                    && noted.insert(name.clone())
                {
                    notes.push(over_file_note(&name, m));
                }
            }
            break;
        }
        // The files not here first; then every reader refused for naming a view set aside this
        // round, or one deferred this round, is deferred to the next, where the stand-ins exist
        // (to a fixpoint among them, whatever their order in the file); then the rest, in the
        // file's order. A deferral traces back to a view set aside this very round: one whose
        // error names a set-aside view that already stands in (through a schema the stand-in is
        // not in, say) would fail on the plant the same way, and falls with the reason. So every
        // round decides at least one view, and the judging ends.
        let mut pending_now: HashSet<String> = HashSet::new();
        for def in &cfg.tables {
            let name = def.name.trim().to_string();
            if let Some(reason) = refused.get(&name)
                && reason.starts_with("IO Error")
            {
                notes.push(format!("table `{name}`: the engine cannot read its file here ({reason}): carried as written for the plant's machine; check the path"));
                unjudged.insert(name.clone());
                pending_now.insert(name);
            }
        }
        let mut undecided: Vec<(String, &String)> = cfg
            .tables
            .iter()
            .map(|d| d.name.trim().to_string())
            .filter_map(|name| refused.get(&name).map(|reason| (name, reason)))
            .filter(|(name, reason)| !reason.starts_with("IO Error") && !unjudged.contains(name))
            .collect();
        // A Binder Error that names a table: a table the file creates is judged in full here, so
        // the view falls; a view over a file not here (a stand-in's columns) only the file can
        // tell, so the view is set aside.
        let binder_table = |reason: &str| reason.strip_prefix("Binder Error: Table \"").and_then(|r| r.split_once('"')).map(|(t, _)| t.to_string());
        let refused_note = |name: &str, reason: &str| format!("table `{name}`: the engine refuses it over the tables this file creates ({reason}): not carried (write it back once what it reads is)");
        loop {
            // Defer every reader of a view set aside or deferred this round, to a fixpoint.
            loop {
                let before = undecided.len();
                undecided.retain(|(name, reason)| match catalog_missing(reason) {
                    Some(m) if pending_now.iter().any(|u| u.eq_ignore_ascii_case(&m)) => {
                        pending_now.insert(name.clone());
                        false
                    }
                    _ => true,
                });
                if undecided.len() == before {
                    break;
                }
            }
            if undecided.is_empty() {
                break;
            }
            // Decide the first view not waiting on another undecided one (a reader cycle leaves
            // none: every view of it falls with its reason); a view set aside by its own reading
            // joins the round's pending views, so what reads it is deferred before anything falls.
            let waits = |reason: &str, rest: &[(String, &String)]| catalog_missing(reason).is_some_and(|m| rest.iter().any(|(n, _)| n.eq_ignore_ascii_case(&m)));
            let Some(at) = undecided.iter().position(|(_, reason)| !waits(reason, &undecided)) else {
                for (name, reason) in undecided.drain(..) {
                    notes.push(refused_note(&name, reason));
                    dropped.insert(name);
                }
                break;
            };
            let (name, reason) = undecided.remove(at);
            let created_fault = binder_table(reason).is_some_and(|t| !over_file.iter().any(|u| u.eq_ignore_ascii_case(&t)));
            match (catalog_missing(reason), created_fault, reads.get(&name)) {
                (None, false, Some(m)) => {
                    notes.push(over_file_note(&name, m));
                    unjudged.insert(name.clone());
                    pending_now.insert(name);
                }
                _ => {
                    notes.push(refused_note(&name, reason));
                    dropped.insert(name);
                }
            }
        }
    }
    file.table = all.into_iter().filter(|t| !dropped.contains(t.name.trim())).collect();
    let toml = render(&file)?;
    Ok(Migrated { toml, notes })
}
