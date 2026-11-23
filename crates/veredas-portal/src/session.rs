//! A session: a project's sources running as workers, their events pumped into the store, and
//! what a shell or the server reads while it runs.
//!
//! Three kinds of source feed one store: tailed logs (lines read into rows by capture rules),
//! OPC UA servers (polls mapped into rows by rules) and telegram channels (messages read into
//! rows by their catalog). Every row lands in a wide base table of the rule or message it came
//! from, every value is a sample on the two clocks, and the ingest keeps what the server and a
//! shell read every second: the series listing (never a store scan), the store's vitals and
//! every source's health on the arrival clock. One master write switch arms every write path.
//!
//! The pump runs on a thread of its own, so a frame loop that stalls never starves the plant's
//! liveness signal, and a reader takes the session's lock for a moment per read.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use thiserror::Error;

use veredas_core::opc::spec::PollSpec;
use veredas_core::{Sample, SeriesRef, SeriesType, SourceError, SourceHandle, SourceId, Value, now_unix_ms};

use crate::capture::{CaptureError, CaptureRule};
use crate::channel::{self, ChannelConfig, ChannelError, ChannelEvent, ChannelHandle, LinkState, MessageTable};
use crate::family::{ActiveMode, FamilySpec};
use crate::opc::{self, ConnState, DedupState, OpcError, OpcEvent, OpcHandle, OpcSourceConfig, WriteGate};
use crate::project::{Claims, Clash};
use crate::source::{LineEvent, SourceSet};
use crate::store::{BaseTableSpec, DuckStore, Residency, StoreError, StoreSnapshot, TableDef, WideRow};
use crate::tail::{FamilyStart, FamilyTailConfig, FamilyTailSource, FileStart, FileTailSource, TailConfig};
use crate::timestamp::TimestampSpec;

/// How often the pump wakes when nothing is pending.
pub const PUMP_IDLE: Duration = Duration::from_millis(50);
/// Events drained per pass, so a backfill's flood never holds the lock for long.
pub const MAX_DRAIN: usize = 20_000;
/// How often the vitals, the series listing and the health are refreshed.
pub const VITALS_EVERY: Duration = Duration::from_millis(500);
/// Distinct keys remembered per series before the count is declared capped.
pub const KEYS_CAP: usize = 64;
/// Numeric samples per data frame, so a backfill's flood becomes frames a client parses.
pub const FRAME_SAMPLES: usize = 2_000;
/// How often a store kept as a window lets go of what fell out of it.
pub const WINDOW_EVERY: Duration = Duration::from_secs(1);
/// The engine's memory ceiling for a record's database, which only takes appends.
pub const RECORD_MEMORY_MB: u32 = 64;

/// Recording to the drive (P-019, `docs/robustness.md` "Wear and filling"): every row the store
/// takes also written to a database file of the session's own, in batches, until the file reaches
/// its size limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingConfig {
    /// The record's database file; one already standing there refuses the session (C-041's guard).
    pub file: std::path::PathBuf,
    /// The size limit of the record's files (the database and its write-ahead log), in bytes.
    pub limit_bytes: u64,
    /// How often what arrived is written to the drive.
    pub write_every: Duration,
}

/// The controller a portal is embedded in, as a source (P-019): its state changes and its faults,
/// handed to the session in process by the embedding through a [`ControllerFeed`], stored like any
/// other data in two tables, [`CONTROLLER_STATES`] and [`CONTROLLER_FAULTS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerSourceConfig {
    pub source_id: String,
    pub display_name: String,
}

/// The table of a controller's state changes: one row per state the embedding hands in.
pub const CONTROLLER_STATES: &str = "controller_states";
/// The table of a controller's faults: one row per fault record.
pub const CONTROLLER_FAULTS: &str = "controller_faults";

/// A fault as the controller records it: its FAULT state (the reason, with its task, location,
/// code, event, kind or download; the debug conditions that were active), the download that was
/// running, and its handler's outcome when one ran (docs/controlador-plan.md, "Faults").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultRecord {
    pub fault_state: veredas_core::state::ControllerState,
    pub download: Option<u32>,
    pub outcome: Option<String>,
}

/// What the embedding hands its portal.
enum ControllerEvent {
    State { index_ms: i64, arrival_ms: i64, state: veredas_core::state::ControllerState },
    Fault { index_ms: i64, arrival_ms: i64, record: FaultRecord },
}

/// The embedding's side of a controller source: every call is a row, stamped on the controller's
/// own clock (its local time, the store's axis) and on arrival. A call after the session stopped
/// is refused, nothing stored.
#[derive(Clone)]
pub struct ControllerFeed {
    tx: std::sync::mpsc::Sender<ControllerEvent>,
    /// Closed by the session before its last drain: a push sends while holding it, so a push
    /// either lands before the drain that stores it or is refused, never accepted and lost.
    closed: Arc<Mutex<bool>>,
}

/// The session the feed belonged to has stopped.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("the session this controller feed belonged to has stopped")]
pub struct FeedClosed;

impl ControllerFeed {
    /// A state change: the state as the controller holds it.
    pub fn push_state(&self, state: &veredas_core::state::ControllerState) -> Result<(), FeedClosed> {
        let (index_ms, arrival_ms) = controller_clocks();
        self.send(ControllerEvent::State { index_ms, arrival_ms, state: state.clone() })
    }

    /// A fault record.
    pub fn push_fault(&self, record: &FaultRecord) -> Result<(), FeedClosed> {
        let (index_ms, arrival_ms) = controller_clocks();
        self.send(ControllerEvent::Fault { index_ms, arrival_ms, record: record.clone() })
    }

    fn send(&self, event: ControllerEvent) -> Result<(), FeedClosed> {
        let closed = self.closed.lock().unwrap_or_else(|p| p.into_inner());
        if *closed {
            return Err(FeedClosed);
        }
        self.tx.send(event).map_err(|_| FeedClosed)
    }
}

/// The controller's own clock read as the store's axis (local time as if UTC), and arrival.
fn controller_clocks() -> (i64, i64) {
    (chrono::Local::now().naive_local().and_utc().timestamp_millis(), now_unix_ms())
}

/// Where a recording stands, as the session publishes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingStatus {
    pub file: std::path::PathBuf,
    pub limit_bytes: u64,
    /// The record's files' size after the last write.
    pub written_bytes: u64,
    /// Rows written to the record.
    pub recorded_rows: u64,
    /// Rows the store took after the recording stopped, never written to the record.
    pub unrecorded_rows: u64,
    /// Why the recording stopped: its size limit, or a write that failed.
    pub stopped_why: Option<String>,
}
/// A source alive but silent this long reads as idle (a quiet log, a share away) rather than live.
pub use crate::project::DEFAULT_IDLE_MS;

// The declarations the session reads live in the project module, so the light build reads a
// project without a session.
pub use crate::project::{ActiveConfig, CaptureRuleConfig, LogKind, LogSourceConfig, TimestampConfig};

/// Everything a session needs to start.
#[derive(Clone)]
pub struct SessionConfig {
    pub name: String,
    /// A start point in the past for the log sources (a family is backfilled from it, a file
    /// read whole); absent, every log is followed live from its end.
    pub start: Option<NaiveDateTime>,
    pub residency: Residency,
    /// The store kept as a window (P-019, `docs/robustness.md` "Wear and filling"): the rows that
    /// arrived within this span, the older let go every second, so the store holds a bounded
    /// stretch however long the session runs. None keeps every row.
    pub store_window: Option<Duration>,
    /// Recording to the drive, the project's choice; none records nothing.
    pub recording: Option<RecordingConfig>,
    /// The controller the portal is embedded in, as a source fed in process; none for a portal
    /// that reads no controller's memory.
    pub controller: Option<ControllerSourceConfig>,
    /// The folder the store is written to at every fault record the controller source hands in
    /// (P-019: its window when it is kept as one, the whole store otherwise); none writes nothing.
    pub fault_copy_dir: Option<std::path::PathBuf>,
    /// The master write switch's initial position. Off unless the project says so; set inside
    /// portal alone, so a shell reads it (`writes_armed()`) and never decides it.
    pub(crate) writes_armed: bool,
    /// Silence after which a live source reads as idle.
    pub idle_after_ms: i64,
    pub logs: Vec<LogSourceConfig>,
    pub rules: Vec<CaptureRuleConfig>,
    pub opc: Vec<OpcSourceConfig>,
    pub channels: Vec<ChannelConfig>,
    pub tables: Vec<TableDef>,
    /// The project's secrets file and the variable holding its key, opened when the session
    /// starts (`docs/questions.md` Q15); absent, every source naming a secret is refused by it.
    pub secrets: Option<crate::secrets::SecretsSource>,
}

impl SessionConfig {
    /// An empty session named `name`: in memory, writes off, nothing to read yet.
    pub fn new(name: impl Into<String>) -> SessionConfig {
        SessionConfig {
            name: name.into(),
            start: None,
            residency: Residency::Ram,
            store_window: None,
            recording: None,
            controller: None,
            fault_copy_dir: None,
            writes_armed: false,
            idle_after_ms: DEFAULT_IDLE_MS,
            logs: Vec::new(),
            rules: Vec::new(),
            opc: Vec::new(),
            channels: Vec::new(),
            tables: Vec::new(),
            secrets: None,
        }
    }

    /// The write switch's position at start, as the project decided it.
    pub fn writes_armed(&self) -> bool {
        self.writes_armed
    }
}

/// Why a session did not start. Nothing has been spawned when one of these comes back, or
/// everything spawned before the failure has been stopped.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("the session has no source")]
    NoSources,
    /// The secrets file, its key or an entry refused by name, never a value (Q15).
    #[error(transparent)]
    Secrets(#[from] crate::secrets::SecretsError),
    #[error("a {0} has an empty id")]
    EmptyId(String),
    #[error("two sources share the id `{0}`")]
    DuplicateSource(String),
    #[error("log source `{id}`: {reason}")]
    Log { id: String, reason: String },
    #[error("log source `{id}`: a start point in the past needs a timestamp rule, to find the line to start at")]
    BackfillNeedsTimestamp { id: String },
    #[error("capture rule `{name}`: {reason}")]
    Rule { name: String, reason: String },
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Opc(#[from] OpcError),
    #[error(transparent)]
    Channel(#[from] ChannelError),
    #[error("two tables would be named `{name}`: {first} and {second}")]
    TableClash { name: String, first: String, second: String },
    #[error("the store: {0}")]
    Store(#[from] StoreError),
    #[error("a store kept as a window is in memory: this one is on disk at {0}")]
    WindowOnDisk(String),
    #[error("a record is for a store in memory: this one is on disk at {0}, where every row already goes")]
    RecordOnDisk(String),
    #[error("source `{id}`: {source}")]
    Spawn { id: String, source: SourceError },
}

impl From<Clash> for SessionError {
    fn from(c: Clash) -> Self {
        match c {
            Clash::EmptyId { kind } => SessionError::EmptyId(kind),
            Clash::DuplicateId(id) => SessionError::DuplicateSource(id),
            Clash::Table { name, first, second } => SessionError::TableClash { name, first, second },
        }
    }
}

/// One source as the session lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: String,
    pub display_name: String,
    /// `log`, `opc` or `channel`.
    pub kind: &'static str,
    /// Whether its lines carry a parsed stamp (a log with a timestamp rule).
    pub has_timestamp: bool,
}

/// Where a source stands, folding its worker's liveness with its arrival age, on the arrival
/// clock. A quiet log and a share away both read as idle: a tail cannot tell them apart; only a
/// worker that ended in error is failed. An OPC server or a telegram peer away is retrying,
/// which is alive and self-healing, never failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthState {
    /// Alive and nothing arrived yet.
    Waiting,
    /// Alive and something arrived within the idle window.
    Live,
    /// Alive and silent past the idle window, for this long.
    Idle { silence_ms: i64 },
    /// Alive and trying to reach its peer, with its reason.
    Retrying(String),
    /// The worker ended cleanly.
    Ended,
    /// The worker ended in error.
    Failed(String),
}

/// One source's health.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceHealth {
    pub state: HealthState,
    /// Lines, polls or messages received.
    pub count: u64,
    /// When something last arrived, on the unix clock; 0 before anything did.
    pub last_arrival_ms: i64,
    /// The last parsed stamp, for a log.
    pub last_ts: Option<NaiveDateTime>,
}

/// One series' latest state as the ingest keeps it: the listing a server serves instead of a
/// store scan, which at a plant's scale runs for minutes.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesLast {
    pub series: SeriesRef,
    pub ty: SeriesType,
    pub count: u64,
    pub keys: usize,
    pub keys_capped: bool,
    pub last_index_ms: i64,
    pub last_value: String,
}

/// The store's vitals, as of the last refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreVitals {
    pub samples: u64,
    pub bytes: u64,
    pub type_conflicts: u64,
}

/// The ingest's counters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counters {
    pub lines: u64,
    pub polls: u64,
    pub messages: u64,
    /// Rows written to base tables.
    pub rows: u64,
    /// Values that refused their type, and poll values that could not be mapped.
    pub failures: u64,
    pub store_errors: u64,
    /// Data frames broadcast to the live stream.
    pub frames: u64,
    /// Passes of the pump, a liveness count.
    pub passes: u64,
    /// Rows a store kept as a window let go.
    pub evicted: u64,
    /// Copies of the store written to the drive at a fault, and the last one's file.
    pub fault_copies: u64,
    pub last_fault_copy: Option<String>,
    pub last_error: Option<String>,
}

/// What a session shows while it runs: shared with a shell and the server, refreshed by the
/// pump, read under short locks.
pub struct Live {
    pub name: String,
    /// When the session started, on the unix clock.
    pub started_ms: i64,
    /// The chosen start point, on the store's axis, when the session backfills.
    pub start_ms: Option<i64>,
    /// The span of the store's window, when it is kept as one.
    pub window_ms: Option<i64>,
    pub sources: Vec<SourceInfo>,
    pub store: Arc<DuckStore>,
    /// The master write switch, live, shared with every write path. Crate-private: outside
    /// portal it is read through `writes_armed` and set through `Session::set_writes_armed`
    /// alone, so a component serving the session (supervisão) cannot arm it by reaching in.
    pub(crate) gate: WriteGate,
    /// Every OPC source's handle, with its compiled spec (the whitelist); reached through
    /// `opc_handle`.
    pub(crate) opc: Vec<OpcHandle>,
    /// Every telegram channel's handle; reached through `channel_handle`.
    pub(crate) channels: Vec<ChannelHandle>,
    pub health: RwLock<BTreeMap<String, SourceHealth>>,
    pub series: RwLock<BTreeMap<String, SeriesLast>>,
    pub vitals: RwLock<StoreVitals>,
    pub counters: RwLock<Counters>,
    /// Data frames for the live stream: JSON, every numeric sample since the last frame. A slow
    /// subscriber drops frames; nothing backpressures the plant.
    pub frames: tokio::sync::broadcast::Sender<Arc<str>>,
    /// Set once the session has stopped and published its end: the server's running flag.
    pub stopped: AtomicBool,
    /// The recording's state, when the session records to the drive.
    pub recording_status: RwLock<Option<RecordingStatus>>,
}

impl Live {
    /// Whether writes are armed right now.
    pub fn writes_armed(&self) -> bool {
        self.gate.is_armed()
    }

    /// The OPC handle of a source, by id.
    pub fn opc_handle(&self, source_id: &str) -> Option<&OpcHandle> {
        self.opc.iter().find(|h| h.source_id == source_id.trim())
    }

    /// The channel handle of a source, by id.
    pub fn channel_handle(&self, source_id: &str) -> Option<&ChannelHandle> {
        self.channels.iter().find(|h| h.source_id == source_id.trim())
    }

    /// The compiled spec of an OPC source, by id.
    pub fn opc_spec(&self, source_id: &str) -> Option<&PollSpec> {
        self.opc_handle(source_id).map(|h| h.spec())
    }

    /// Subscribe to the data frames.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Arc<str>> {
        self.frames.subscribe()
    }
}

/// A channel's message tables and where each sits in the store.
struct ChannelRuntime {
    handle: SourceHandle,
    writer: ChannelHandle,
    /// The message name to (the base table index, the table).
    tables: HashMap<String, (usize, MessageTable)>,
    state: LinkState,
}

struct OpcRuntime {
    handle: SourceHandle,
    spec: PollSpec,
    /// The base table index of the spec's first rule; its rule N is at `base + N`.
    base: usize,
    seq: u64,
    state: ConnState,
    dedup: DedupState,
}

struct Stats {
    count: u64,
    last_arrival_ms: i64,
    last_ts: Option<NaiveDateTime>,
}

struct Inner {
    set: Option<SourceSet>,
    rx: Receiver<LineEvent>,
    opc_rx: Receiver<OpcEvent>,
    channel_rx: Receiver<ChannelEvent>,
    /// The controller source's events, its id and its two tables' indexes.
    controller: Option<ControllerRuntime>,
    opc: Vec<OpcRuntime>,
    channels: Vec<ChannelRuntime>,
    captures: Vec<(CaptureRule, Vec<Arc<str>>)>,
    time_formats: HashMap<SourceId, Vec<String>>,
    wide_buf: Vec<Vec<WideRow>>,
    frame_buf: Vec<serde_json::Value>,
    stats: BTreeMap<String, Stats>,
    series: BTreeMap<String, SeriesLastLocal>,
    counters: Counters,
    vitals_at: Option<Instant>,
    /// The window's span and its last pass, when the store is kept as one.
    window_ms: Option<i64>,
    evicted_at: Option<Instant>,
    recorder: Option<Recorder>,
    idle_after_ms: i64,
    stopped: bool,
    /// Each worker's outcome at the stop (none for a clean end, the text of a failure), read by
    /// the last refresh once the runtimes are gone, so a failed source stays failed.
    ended: HashMap<String, Option<String>>,
}

struct ControllerRuntime {
    source: SourceId,
    rx: Receiver<ControllerEvent>,
    /// The feed's gate, closed before the last drain.
    closed: Arc<Mutex<bool>>,
    states: usize,
    faults: usize,
    seq: u64,
    /// Fault records drained this pass, each owed a copy of the window once its row is stored.
    copies_owed: Vec<i64>,
    /// Where those copies go, and the session's name their files carry.
    copy_dir: Option<std::path::PathBuf>,
    session_name: String,
}

/// A recording in progress: its own store on the drive, the rows waiting for the next write.
struct Recorder {
    store: DuckStore,
    pending: Vec<Vec<WideRow>>,
    write_every: Duration,
    written_at: Instant,
    status: RecordingStatus,
}

impl Recorder {
    /// Write what is pending, then measure the record's files; at the size limit the recording
    /// stops. Nothing already written is deleted, and what comes after is counted, not written.
    // DECISION PENDING (docs/questions.md Q41): what gives way at a record's limit is not ruled;
    // the recording stopping and nothing deleted is the reversible answer, as Q16 decided for a
    // project's disk store ("nothing is refused, deleted or merged").
    fn write(&mut self) {
        self.written_at = Instant::now();
        if self.status.stopped_why.is_some() {
            return;
        }
        // Every table's rows in one transaction: the batch reaches the drive in one commit, whole
        // or not at all; a failure stops the recording, the batch counted unrecorded.
        let batch: Vec<(usize, Vec<WideRow>)> = self.pending.iter_mut().enumerate().filter(|(_, rows)| !rows.is_empty()).map(|(i, rows)| (i, std::mem::take(rows))).collect();
        let rows: u64 = batch.iter().map(|(_, r)| r.len() as u64).sum();
        let tables: Vec<(usize, &[WideRow])> = batch.iter().map(|(i, r)| (*i, r.as_slice())).collect();
        match self.store.append_wide_together(&tables) {
            Ok(()) => self.status.recorded_rows += rows,
            Err(e) => {
                self.status.unrecorded_rows += rows;
                self.status.stopped_why = Some(format!("a write to the record failed: {e}"));
            }
        }
        self.status.written_bytes = record_bytes(&self.status.file);
        if self.status.stopped_why.is_none() && self.status.written_bytes >= self.status.limit_bytes {
            self.status.stopped_why = Some(format!("the record reached its size limit ({} of {} bytes)", self.status.written_bytes, self.status.limit_bytes));
        }
        if self.status.stopped_why.is_some() {
            let left: u64 = self.pending.iter().map(|r| r.len() as u64).sum();
            self.status.unrecorded_rows += left;
            self.pending.iter_mut().for_each(Vec::clear);
        }
    }

    /// Rows the store took: pending for the next write, or counted unrecorded once stopped.
    fn take(&mut self, table: usize, rows: &[WideRow]) {
        if self.status.stopped_why.is_some() {
            self.status.unrecorded_rows += rows.len() as u64;
        } else if let Some(p) = self.pending.get_mut(table) {
            p.extend_from_slice(rows);
        }
    }
}

/// The size of a record's files: the database and its write-ahead log beside it.
fn record_bytes(file: &std::path::Path) -> u64 {
    let wal = std::path::PathBuf::from(format!("{}.wal", file.display()));
    [file.to_path_buf(), wal].iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
}

struct SeriesLastLocal {
    series: SeriesRef,
    ty: SeriesType,
    count: u64,
    keys: BTreeSet<Box<str>>,
    keys_capped: bool,
    last_index_ms: i64,
    last_value: String,
}

/// A running session: its workers, its pump thread, and what it shows.
pub struct Session {
    live: Arc<Live>,
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    pump: Option<std::thread::JoinHandle<()>>,
    /// The controller source's feed, when the session has one; handed out to the embedding.
    controller_feed: Option<ControllerFeed>,
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn value_type(v: &Value) -> SeriesType {
    match v {
        Value::Int(_) => SeriesType::Int,
        Value::Float(_) => SeriesType::Float,
        Value::Text(_) => SeriesType::Text,
        Value::Time(_) => SeriesType::Time,
    }
}

/// The window's span in milliseconds, at least one.
fn window_ms(cfg: &SessionConfig) -> Option<i64> {
    cfg.store_window.map(|w| i64::try_from(w.as_millis()).unwrap_or(i64::MAX).max(1))
}

fn parse_type(text: &str) -> Option<SeriesType> {
    opc::parse_type(text).filter(|_| !text.trim().is_empty()).or(if text.trim().is_empty() { Some(SeriesType::Text) } else { None })
}

/// A log source built and checked: the source, its timestamp formats, whether it stamps, its display name.
type BuiltLog = (Box<dyn crate::source::Source>, Vec<String>, bool, String);

/// What the store is opened over, compiled once from a configuration: every capture rule a
/// table, every OPC source's rules a table each (the source compiled with its password from
/// `secret`, the lookup of an entry of the secrets file by name), every channel checked whole and
/// each catalog message a table, the names claimed as they go, the views named once too. The
/// session compiles with the secrets file decrypted in memory (Q15); the project's import (C-046)
/// compiles the same way with a lookup of its own, so the engine can judge the views over the very
/// tables the file creates before the file is printed.
pub struct Tables {
    pub specs: Vec<BaseTableSpec>,
    captures: Vec<(CaptureRule, Vec<Arc<str>>)>,
    opc_specs: Vec<(PollSpec, usize)>,
    channel_tables: Vec<HashMap<String, (usize, MessageTable)>>,
    /// The controller source's two tables, when there is one: states, then faults.
    controller_tables: Option<(usize, usize)>,
}

pub fn compile_tables(cfg: &SessionConfig, secret: &dyn Fn(&str) -> Option<String>, claims: &mut Claims) -> Result<Tables, SessionError> {
    // The capture rules, each a table.
    let mut captures: Vec<(CaptureRule, Vec<Arc<str>>)> = Vec::new();
    let mut specs: Vec<BaseTableSpec> = Vec::new();
    for rule in &cfg.rules {
        let name = rule.name.trim();
        opc::check_name(name).map_err(|reason| SessionError::Rule { name: name.to_string(), reason })?;
        let mut types = HashMap::new();
        for (group, ty) in &rule.types {
            let t = parse_type(ty).ok_or_else(|| SessionError::Rule {
                name: name.to_string(),
                reason: format!("group `{group}`: unknown type `{}` (int, float, text or time)", ty.trim()),
            })?;
            types.insert(group.clone(), t);
        }
        let scope = rule.scope.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(s) = scope
            && !cfg.logs.iter().any(|l| l.id.trim() == s)
        {
            return Err(SessionError::Rule { name: name.to_string(), reason: format!("the scope `{s}` is no log source of this session") });
        }
        let compiled = CaptureRule::new(name, rule.pattern.trim(), scope.map(SourceId::new), &types, rule.key.as_deref().map(str::trim))?;
        for (series, _) in compiled.series() {
            opc::check_name(series).map_err(|reason| SessionError::Rule { name: name.to_string(), reason: format!("group `{series}`: {reason}") })?;
        }
        claims.table(name, format!("capture rule `{name}`"))?;
        specs.push(BaseTableSpec {
            name: name.to_string(),
            columns: compiled.series().map(|(n, t)| (n.to_string(), t)).collect(),
            has_key: compiled.key_group().is_some(),
        });
        let columns = compiled.series().map(|(n, _)| Arc::<str>::from(n)).collect();
        captures.push((compiled, columns));
    }

    // The OPC sources, each rule a table.
    let mut opc_specs: Vec<(PollSpec, usize)> = Vec::new();
    for source in &cfg.opc {
        let spec = opc::compile(source, secret)?;
        let base = specs.len();
        for rule in &spec.rules {
            let (name, columns) = opc::table_of(rule);
            claims.table(&name, format!("OPC rule `{name}` of source `{}`", spec.source_id))?;
            specs.push(BaseTableSpec { name, columns, has_key: false });
        }
        opc_specs.push((spec, base));
    }

    // The channels, each checked whole (the framing against the channel file, every reply rule
    // against the catalog, every message's table within the cap) before anything spawns, each
    // catalog message a table.
    let mut channel_tables: Vec<HashMap<String, (usize, MessageTable)>> = Vec::new();
    for ch in &cfg.channels {
        let mut tables = HashMap::new();
        for table in channel::check(ch)? {
            claims.table(&table.name, format!("message `{}` of channel `{}`", table.name, ch.source_id))?;
            specs.push(BaseTableSpec { name: table.name.clone(), columns: table.columns.clone(), has_key: false });
            tables.insert(table.name.clone(), (specs.len() - 1, table));
        }
        channel_tables.push(tables);
    }
    // The controller's two tables, typed by what the state and a fault record carry.
    let mut controller_tables = None;
    if let Some(c) = &cfg.controller {
        let id = c.source_id.trim();
        let text = |n: &str| (n.to_string(), SeriesType::Text);
        let int = |n: &str| (n.to_string(), SeriesType::Int);
        claims.table(CONTROLLER_STATES, format!("the state changes of controller `{id}`"))?;
        specs.push(BaseTableSpec { name: CONTROLLER_STATES.into(), columns: vec![text("state_text"), text("state"), int("reason"), text("reason_word"), text("health")], has_key: false });
        let states = specs.len() - 1;
        claims.table(CONTROLLER_FAULTS, format!("the faults of controller `{id}`"))?;
        specs.push(BaseTableSpec { name: CONTROLLER_FAULTS.into(), columns: vec![text("fault_text"), int("cause"), text("cause_word"), int("download"), text("outcome")], has_key: false });
        controller_tables = Some((states, specs.len() - 1));
    }
    // The views, named once too: against the base tables and against each other.
    for def in &cfg.tables {
        claims.table(def.name.trim(), format!("table `{}`", def.name.trim()))?;
    }
    Ok(Tables { specs, captures, opc_specs, channel_tables, controller_tables })
}

impl Session {
    /// Check the whole configuration, open the store, spawn every source and the pump. A
    /// failure after a source was spawned stops what was spawned and reports it.
    pub fn start(cfg: SessionConfig) -> Result<Session, SessionError> {
        // Every source has an id once and every table a name once: the claims a project file
        // makes (C-046), made again here through the same type for a configuration built by hand.
        if cfg.logs.is_empty() && cfg.opc.is_empty() && cfg.channels.is_empty() && cfg.controller.is_none() {
            return Err(SessionError::NoSources);
        }
        // The window keeps nothing on the drive (P-019): a store on disk is no window.
        if cfg.store_window.is_some()
            && let Residency::Disk { path, .. } = &cfg.residency
        {
            return Err(SessionError::WindowOnDisk(path.display().to_string()));
        }
        // A record is a store in memory's way to the drive (C-091): a store on disk already
        // holds every row.
        if cfg.recording.is_some()
            && let Residency::Disk { path, .. } = &cfg.residency
        {
            return Err(SessionError::RecordOnDisk(path.display().to_string()));
        }
        let mut claims = Claims::default();
        for l in &cfg.logs {
            claims.source("log", &l.id)?;
        }
        for o in &cfg.opc {
            claims.source("opc source", &o.source_id)?;
        }
        for c in &cfg.channels {
            claims.source("channel", &c.source_id)?;
        }
        if let Some(c) = &cfg.controller {
            claims.source("controller", &c.source_id)?;
        }

        // The secrets file opens first, decrypted in memory under the key the environment holds
        // (Q15): a session with no `[secrets]` table has none, and a source naming an entry
        // then refuses by the entry's name. A missing key or a wrong key refuses here, by the
        // variable's name and the file's path, never by a value.
        let secrets = match &cfg.secrets {
            Some(source) => source.open(&|var| std::env::var(var).ok())?,
            None => crate::secrets::Secrets::none(),
        };

        // Every table the store is opened over, the names claimed as they are compiled; the
        // project's import compiles the same way (C-046), so the two never drift.
        let Tables { specs, captures, opc_specs, channel_tables, controller_tables } =
            compile_tables(&cfg, &|entry| secrets.get(entry).map(str::to_string), &mut claims)?;

        // The log sources, built before anything spawns so a bad one refuses the whole session.
        let mut built: Vec<BuiltLog> = Vec::new();
        for log in &cfg.logs {
            let id = log.id.trim().to_string();
            let display = match log.display_name.trim() {
                "" => id.clone(),
                d => d.to_string(),
            };
            let ts = match &log.timestamp {
                None => None,
                Some(t) => Some(
                    TimestampSpec::new(t.pattern.trim(), t.formats.iter().map(|f| f.trim().to_string()))
                        .map_err(|e| SessionError::Log { id: id.clone(), reason: e.to_string() })?,
                ),
            };
            let formats: Vec<String> = log.timestamp.as_ref().map(|t| t.formats.iter().map(|f| f.trim().to_string()).collect()).unwrap_or_default();
            let source: Box<dyn crate::source::Source> = match &log.kind {
                LogKind::File { path } => {
                    if path.trim().is_empty() {
                        return Err(SessionError::Log { id, reason: "the file path is empty".into() });
                    }
                    let start = if cfg.start.is_some() { FileStart::Begin } else { FileStart::End };
                    let mut src = FileTailSource::new(&id, &display, path.trim(), TailConfig { start, ..TailConfig::default() });
                    if let Some(spec) = ts.clone() {
                        src = src.with_timestamps(spec);
                    }
                    Box::new(src)
                }
                LogKind::Family { dir, pattern, date_format, active, recursive } => {
                    if dir.trim().is_empty() {
                        return Err(SessionError::Log { id, reason: "the folder is empty".into() });
                    }
                    let mode = match active {
                        ActiveConfig::Newest => ActiveMode::NewestByOrder,
                        ActiveConfig::Fixed(name) => ActiveMode::Fixed(name.trim().to_string()),
                    };
                    let spec = FamilySpec::new(dir.trim(), pattern.trim(), date_format.as_deref().map(str::trim).filter(|d| !d.is_empty()), mode)
                        .map_err(|e| SessionError::Log { id: id.clone(), reason: e.to_string() })?
                        .with_recursive(*recursive);
                    let start = match cfg.start {
                        None => FamilyStart::ActiveEnd,
                        Some(t) => {
                            if ts.is_none() {
                                return Err(SessionError::BackfillNeedsTimestamp { id });
                            }
                            FamilyStart::Backfill(t)
                        }
                    };
                    let mut src = FamilyTailSource::new(&id, &display, spec, FamilyTailConfig { start, ..FamilyTailConfig::default() });
                    if let Some(spec) = ts.clone() {
                        src = src.with_timestamps(spec);
                    }
                    Box::new(src)
                }
            };
            built.push((source, formats, ts.is_some(), display));
        }

        // The record's file asked first (refused like the store when a database stands there),
        // so a refused session leaves no store behind; then the store, the record, the workers.
        if let Some(r) = &cfg.recording {
            crate::store::fresh_at(&r.file)?;
        }
        let store = Arc::new(DuckStore::open(&cfg.residency, &specs, &cfg.tables)?);
        let window = window_ms(&cfg);
        let recorder = match &cfg.recording {
            None => None,
            Some(r) => Some(Recorder {
                store: DuckStore::open(&Residency::Disk { path: r.file.clone(), memory_limit_mb: RECORD_MEMORY_MB }, &specs, &cfg.tables)?,
                pending: specs.iter().map(|_| Vec::new()).collect(),
                write_every: r.write_every.max(Duration::from_millis(1)),
                written_at: Instant::now(),
                status: RecordingStatus { file: r.file.clone(), limit_bytes: r.limit_bytes, written_bytes: record_bytes(&r.file), ..RecordingStatus::default() },
            }),
        };
        let recording_status = RwLock::new(recorder.as_ref().map(|r| r.status.clone()));
        let gate = WriteGate::new(cfg.writes_armed);
        let (frames, _) = tokio::sync::broadcast::channel(256);
        let mut sources: Vec<SourceInfo> = Vec::new();
        let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
        let mut time_formats: HashMap<SourceId, Vec<String>> = HashMap::new();

        let (mut set, rx) = SourceSet::new();
        for (source, formats, has_ts, display) in built {
            let id = source.id().as_str().to_string();
            if let Err(e) = set.spawn(source) {
                set.stop_all();
                return Err(SessionError::Spawn { id, source: e });
            }
            time_formats.insert(SourceId::new(&id), formats);
            stats.insert(id.clone(), Stats { count: 0, last_arrival_ms: 0, last_ts: None });
            sources.push(SourceInfo { id, display_name: display, kind: "log", has_timestamp: has_ts });
        }

        let (opc_tx, opc_rx) = std::sync::mpsc::channel();
        let mut opc_runtimes: Vec<OpcRuntime> = Vec::new();
        let mut opc_handles: Vec<OpcHandle> = Vec::new();
        for (spec, base) in opc_specs {
            let worker = match opc::spawn(spec.clone(), opc_tx.clone(), gate.clone()) {
                Ok(w) => w,
                Err(e) => {
                    set.stop_all();
                    for rt in &opc_runtimes {
                        rt.handle.request_stop();
                    }
                    return Err(SessionError::Spawn { id: spec.source_id.clone(), source: e });
                }
            };
            stats.insert(spec.source_id.clone(), Stats { count: 0, last_arrival_ms: 0, last_ts: None });
            sources.push(SourceInfo { id: spec.source_id.clone(), display_name: spec.display_name.clone(), kind: "opc", has_timestamp: false });
            opc_handles.push(worker.writer.clone());
            opc_runtimes.push(OpcRuntime { handle: worker.handle, spec, base, seq: 0, state: ConnState::Connecting, dedup: DedupState::default() });
        }

        let (channel_tx, channel_rx) = std::sync::mpsc::channel();
        let mut channel_runtimes: Vec<ChannelRuntime> = Vec::new();
        let mut channel_handles: Vec<ChannelHandle> = Vec::new();
        for (ch, tables) in cfg.channels.into_iter().zip(channel_tables) {
            let id = ch.source_id.clone();
            let display = ch.display_name.clone();
            let worker = match channel::spawn(ch, channel_tx.clone(), gate.clone()) {
                Ok(w) => w,
                Err(e) => {
                    set.stop_all();
                    for rt in &opc_runtimes {
                        rt.handle.request_stop();
                    }
                    for rt in &channel_runtimes {
                        rt.handle.request_stop();
                    }
                    return Err(SessionError::Channel(e));
                }
            };
            stats.insert(id.clone(), Stats { count: 0, last_arrival_ms: 0, last_ts: None });
            sources.push(SourceInfo { id, display_name: display, kind: "channel", has_timestamp: false });
            channel_handles.push(worker.writer.clone());
            channel_runtimes.push(ChannelRuntime { handle: worker.handle, writer: worker.writer, tables, state: LinkState::Waiting });
        }

        // The controller source: its feed kept by the session and handed out, its events drained
        // by the pump like every other source's.
        let (controller, controller_feed) = match (&cfg.controller, controller_tables) {
            (Some(c), Some((states, faults))) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let closed = Arc::new(Mutex::new(false));
                let id = c.source_id.trim().to_string();
                let display = if c.display_name.trim().is_empty() { id.clone() } else { c.display_name.trim().to_string() };
                stats.insert(id.clone(), Stats { count: 0, last_arrival_ms: 0, last_ts: None });
                sources.push(SourceInfo { id: id.clone(), display_name: display, kind: "controller", has_timestamp: false });
                let runtime = ControllerRuntime {
                    source: SourceId::new(&id),
                    rx,
                    closed: Arc::clone(&closed),
                    states,
                    faults,
                    seq: 0,
                    copies_owed: Vec::new(),
                    copy_dir: cfg.fault_copy_dir.clone(),
                    session_name: cfg.name.clone(),
                };
                (Some(runtime), Some(ControllerFeed { tx, closed }))
            }
            _ => (None, None),
        };
        let wide_buf = specs.iter().map(|_| Vec::new()).collect();
        let live = Arc::new(Live {
            name: cfg.name.clone(),
            started_ms: now_unix_ms(),
            start_ms: cfg.start.map(|t| t.and_utc().timestamp_millis()),
            window_ms: window,
            sources,
            store: Arc::clone(&store),
            gate,
            opc: opc_handles,
            channels: channel_handles,
            health: RwLock::new(BTreeMap::new()),
            series: RwLock::new(BTreeMap::new()),
            vitals: RwLock::new(StoreVitals::default()),
            counters: RwLock::new(Counters::default()),
            frames,
            stopped: AtomicBool::new(false),
            recording_status,
        });
        let inner = Arc::new(Mutex::new(Inner {
            set: Some(set),
            rx,
            opc_rx,
            channel_rx,
            controller,
            opc: opc_runtimes,
            channels: channel_runtimes,
            captures,
            time_formats,
            wide_buf,
            frame_buf: Vec::new(),
            stats,
            series: BTreeMap::new(),
            counters: Counters::default(),
            vitals_at: None,
            window_ms: window,
            evicted_at: None,
            recorder,
            idle_after_ms: cfg.idle_after_ms.max(1),
            stopped: false,
            ended: HashMap::new(),
        }));
        // Hold the senders the workers write to, so a session with no source of a kind never
        // sees its receiver disconnect.
        let _ = (&opc_tx, &channel_tx);
        let stop = Arc::new(AtomicBool::new(false));
        let pump = {
            let inner = Arc::clone(&inner);
            let live = Arc::clone(&live);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name(format!("session-{}", cfg.name))
                .spawn(move || {
                    let _keep = (opc_tx, channel_tx);
                    while !stop.load(Ordering::Relaxed) {
                        let drained = lock(&inner).pump(&live);
                        if drained < MAX_DRAIN {
                            std::thread::sleep(PUMP_IDLE);
                        }
                    }
                    // The last passes: the workers are stopped and their senders dropped, so
                    // the channels drain dry, cap or no cap, and every row they sent is stored;
                    // then the end is published, the refresh timer notwithstanding, so a reader
                    // still holding the session sees every source ended and the final counters.
                    lock(&inner).close_feed();
                    while lock(&inner).pump(&live) > 0 {}
                    lock(&inner).record(&live, true);
                    lock(&inner).refresh_now(&live);
                    live.stopped.store(true, Ordering::Relaxed);
                })
                .map_err(|e| SessionError::Spawn { id: cfg.name.clone(), source: SourceError::NotConfigured(format!("spawn the pump: {e}")) })?
        };
        Ok(Session { live, inner, stop, pump: Some(pump), controller_feed })
    }

    /// What the session shows: shared, refreshed by the pump.
    pub fn live(&self) -> Arc<Live> {
        Arc::clone(&self.live)
    }

    /// The feed of the session's controller source, for the embedding to hand its states and
    /// faults to; none when the session has no controller source. A push after the stop is
    /// refused.
    pub fn controller_feed(&self) -> Option<ControllerFeed> {
        self.controller_feed.clone()
    }

    /// Arm or disarm every write path at once, live: the next write obeys it.
    pub fn set_writes_armed(&self, armed: bool) {
        self.live.gate.set(armed);
    }

    /// Stop every worker and the pump, join them all, and report each worker's end.
    pub fn stop(mut self) -> Vec<(SourceId, Result<(), SourceError>)> {
        // The workers first, so every event they sent stands in the channels when the pump
        // drains them dry; then the pump, whose last passes take everything and publish the end.
        let ended = lock(&self.inner).stop_workers();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(p) = self.pump.take() {
            let _ = p.join();
        }
        ended
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        lock(&self.inner).stop_workers();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(p) = self.pump.take() {
            let _ = p.join();
        }
    }
}

impl Inner {
    fn stop_workers(&mut self) -> Vec<(SourceId, Result<(), SourceError>)> {
        if self.stopped {
            return Vec::new();
        }
        self.stopped = true;
        // Signal every worker before joining any, so they wind down together.
        for rt in &self.opc {
            rt.handle.request_stop();
        }
        for rt in &self.channels {
            rt.handle.request_stop();
        }
        let mut out: Vec<(SourceId, Result<(), SourceError>)> = match self.set.take() {
            Some(set) => set.stop_all(),
            None => Vec::new(),
        };
        for rt in std::mem::take(&mut self.opc) {
            let id = SourceId::new(&rt.spec.source_id);
            out.push((id, rt.handle.join()));
        }
        for rt in std::mem::take(&mut self.channels) {
            let id = SourceId::new(&rt.writer.source_id);
            out.push((id, rt.handle.join()));
        }
        if let Some(c) = &self.controller {
            out.push((c.source.clone(), Ok(())));
        }
        for (id, r) in &out {
            self.ended.insert(id.as_str().to_string(), r.as_ref().err().map(|e| e.to_string()));
        }
        out
    }

    /// Note one sample: the series cache, and a frame when numeric. The frame carries the series'
    /// count with this sample (`n`), the count the cache shows once it holds it: a listener orders
    /// two samples under one stamp by it, whether a frame came before the cache's refresh or was
    /// lost to a lag.
    fn note_sample(&mut self, sample: &Sample) {
        let key = format!("{}|{}", sample.source.as_str(), sample.series.as_ref());
        let entry = self.series.entry(key).or_insert_with(|| SeriesLastLocal {
            series: sample.series_ref(),
            ty: value_type(&sample.value),
            count: 0,
            keys: BTreeSet::new(),
            keys_capped: false,
            last_index_ms: 0,
            last_value: String::new(),
        });
        entry.count += 1;
        entry.ty = value_type(&sample.value);
        entry.last_index_ms = sample.index_ms();
        entry.last_value = sample.value.to_string();
        if let Some(k) = sample.key.as_deref()
            && !entry.keys_capped
            && !entry.keys.contains(k)
        {
            if entry.keys.len() >= KEYS_CAP {
                entry.keys_capped = true;
            } else {
                entry.keys.insert(k.into());
            }
        }
        let n = entry.count;
        if let Some(v) = sample.value.as_f64() {
            self.frame_buf.push(serde_json::json!({
                "src": sample.source.as_str(),
                "ser": sample.series.as_ref(),
                "k": sample.key.as_deref(),
                "ms": sample.index_ms(),
                "v": v,
                "n": n,
            }));
        }
    }

    fn pump_lines(&mut self, live: &Live) -> usize {
        let batch: Vec<LineEvent> = self.rx.try_iter().take(MAX_DRAIN).collect();
        let n = batch.len();
        for ev in batch {
            if let Some(st) = self.stats.get_mut(ev.source.as_str()) {
                st.count += 1;
                st.last_arrival_ms = ev.arrival_unix_ms;
                if ev.parsed_ts.is_some() {
                    st.last_ts = ev.parsed_ts;
                }
            }
            self.counters.lines += 1;
            static NO_FORMATS: Vec<String> = Vec::new();
            let formats = self.time_formats.get(&ev.source).cloned().unwrap_or_else(|| NO_FORMATS.clone());
            for i in 0..self.captures.len() {
                let outcome = self.captures[i].0.apply(&ev, &formats);
                if !outcome.matched {
                    continue;
                }
                self.counters.failures += outcome.coercion_failures as u64;
                let columns = self.captures[i].1.clone();
                let mut values: Vec<Option<Value>> = vec![None; columns.len()];
                for sample in &outcome.samples {
                    self.note_sample(sample);
                    if let Some(ci) = columns.iter().position(|c| c.as_ref() == sample.series.as_ref()) {
                        values[ci] = Some(sample.value.clone());
                    }
                }
                self.wide_buf[i].push(WideRow {
                    source: ev.source.clone(),
                    key: outcome.key.clone(),
                    index_ms: ev.index_ms(),
                    arrival_ms: ev.arrival_unix_ms,
                    seq: ev.seq,
                    values,
                });
            }
        }
        let _ = live;
        n
    }

    fn pump_opc(&mut self) -> usize {
        let batch: Vec<OpcEvent> = self.opc_rx.try_iter().take(MAX_DRAIN).collect();
        let n = batch.len();
        for event in batch {
            match event {
                OpcEvent::Status { source_id, state } => {
                    if let Some(rt) = self.opc.iter_mut().find(|r| r.spec.source_id == source_id) {
                        rt.state = state;
                    }
                }
                OpcEvent::Poll { source_id, index_ms, arrival_ms, values } => {
                    let Some(i) = self.opc.iter().position(|r| r.spec.source_id == source_id) else { continue };
                    self.counters.polls += 1;
                    let (spec, base, seq) = {
                        let rt = &mut self.opc[i];
                        rt.seq += 1;
                        (rt.spec.clone(), rt.base, rt.seq)
                    };
                    let mapped = opc::map_poll_dedup(&spec, &values, index_ms, seq, Some(&mut self.opc[i].dedup));
                    self.counters.failures += mapped.failures.len() as u64;
                    if let Some((column, why)) = mapped.failures.first() {
                        self.counters.last_error = Some(format!("opc `{source_id}` column `{column}`: {why}"));
                    }
                    for sample in &mapped.samples {
                        self.note_sample(sample);
                    }
                    let sid = SourceId::new(&source_id);
                    for row in &mapped.rows {
                        self.wide_buf[base + row.rule_index].push(WideRow {
                            source: sid.clone(),
                            key: None,
                            index_ms,
                            arrival_ms,
                            seq,
                            values: row.values.clone(),
                        });
                    }
                    if let Some(st) = self.stats.get_mut(&source_id) {
                        st.count += 1;
                        st.last_arrival_ms = arrival_ms;
                    }
                }
            }
        }
        n
    }

    fn pump_channels(&mut self) -> usize {
        let batch: Vec<ChannelEvent> = self.channel_rx.try_iter().take(MAX_DRAIN).collect();
        let n = batch.len();
        for event in batch {
            match event {
                ChannelEvent::Status { source_id, state } => {
                    if let Some(rt) = self.channels.iter_mut().find(|r| r.writer.source_id == source_id) {
                        rt.state = state;
                    }
                }
                ChannelEvent::Message { source_id, index_ms, arrival_ms, name, fields, .. } => {
                    let Some(i) = self.channels.iter().position(|r| r.writer.source_id == source_id) else { continue };
                    self.counters.messages += 1;
                    let Some((table_index, table)) = self.channels[i].tables.get(&name).cloned() else { continue };
                    let values = table.row(&fields);
                    let sid = SourceId::new(&source_id);
                    let seq = self.counters.messages;
                    for ((column, _), value) in table.columns.iter().zip(values.iter()) {
                        if let Some(v) = value {
                            let sample = Sample {
                                source: sid.clone(),
                                series: Arc::from(column.as_str()),
                                key: None,
                                ts: None,
                                seq,
                                arrival_unix_ms: arrival_ms,
                                value: v.clone(),
                            };
                            // The message's index is the plant clock; a sample indexes on its
                            // stamp, so the plant clock rides as the stamp.
                            let sample = Sample { ts: chrono::DateTime::from_timestamp_millis(index_ms).map(|d| d.naive_utc()), ..sample };
                            self.note_sample(&sample);
                        }
                    }
                    self.wide_buf[table_index].push(WideRow { source: sid, key: None, index_ms, arrival_ms, seq, values });
                    if let Some(st) = self.stats.get_mut(&source_id) {
                        st.count += 1;
                        st.last_arrival_ms = arrival_ms;
                    }
                }
                ChannelEvent::Unreadable { source_id, arrival_ms, id, .. } => {
                    self.counters.failures += 1;
                    self.counters.last_error = Some(match id {
                        Some(id) => format!("channel `{source_id}`: a body of id {id} the catalog could not read"),
                        None => format!("channel `{source_id}`: a frame carrying no id"),
                    });
                    if let Some(st) = self.stats.get_mut(&source_id) {
                        st.last_arrival_ms = arrival_ms;
                    }
                }
                ChannelEvent::KeepAlive { source_id, arrival_ms, .. } => {
                    if let Some(st) = self.stats.get_mut(&source_id) {
                        st.last_arrival_ms = arrival_ms;
                    }
                }
                ChannelEvent::Reply { source_id, on, send, outcome, .. } => {
                    if let channel::ReplyOutcome::Refused(why) = outcome {
                        self.counters.failures += 1;
                        self.counters.last_error = Some(format!("channel `{source_id}`: the reply `{send}` to `{on}` was refused: {why}"));
                    }
                }
            }
        }
        n
    }

    /// The controller feed closed: every push from now on is refused, every one before it is in
    /// the channel the last drain empties.
    fn close_feed(&mut self) {
        if let Some(c) = &self.controller {
            *c.closed.lock().unwrap_or_else(|p| p.into_inner()) = true;
        }
    }

    /// The controller's state changes and fault records into their tables' rows, every value a
    /// sample of the source like any other.
    fn pump_controller(&mut self) -> usize {
        let Some(c) = self.controller.as_mut() else { return 0 };
        let batch: Vec<ControllerEvent> = c.rx.try_iter().take(MAX_DRAIN).collect();
        let n = batch.len();
        let (source, states, faults) = (c.source.clone(), c.states, c.faults);
        for event in batch {
            let seq = {
                let c = self.controller.as_mut().expect("the controller source");
                c.seq += 1;
                c.seq
            };
            let (table, index_ms, arrival_ms, columns, values): (usize, i64, i64, [&str; 5], Vec<Option<Value>>) = match event {
                ControllerEvent::State { index_ms, arrival_ms, state } => {
                    let number = state.state.reason_number();
                    (
                        states,
                        index_ms,
                        arrival_ms,
                        ["state_text", "state", "reason", "reason_word", "health"],
                        vec![
                            Some(Value::Text(Arc::from(state.to_string().as_str()))),
                            Some(Value::Text(Arc::from(state.state.word().word()))),
                            number.map(|n| Value::Int(i64::from(n))),
                            number.and_then(veredas_core::state::reason_entry).map(|r| Value::Text(Arc::from(r.word))),
                            state.health().map(|h| Value::Text(Arc::from(h.to_string().as_str()))),
                        ],
                    )
                }
                ControllerEvent::Fault { index_ms, arrival_ms, record } => {
                    if let Some(c) = self.controller.as_mut() {
                        c.copies_owed.push(index_ms);
                    }
                    let number = record.fault_state.state.reason_number();
                    (
                        faults,
                        index_ms,
                        arrival_ms,
                        ["fault_text", "cause", "cause_word", "download", "outcome"],
                        vec![
                            Some(Value::Text(Arc::from(record.fault_state.to_string().as_str()))),
                            number.map(|n| Value::Int(i64::from(n))),
                            number.and_then(veredas_core::state::reason_entry).map(|r| Value::Text(Arc::from(r.word))),
                            record.download.map(|d| Value::Int(i64::from(d))),
                            record.outcome.as_deref().map(|o| Value::Text(Arc::from(o))),
                        ],
                    )
                }
            };
            let ts = chrono::DateTime::from_timestamp_millis(index_ms).map(|d| d.naive_utc());
            for (column, value) in columns.iter().zip(values.iter()) {
                if let Some(v) = value {
                    let sample = Sample { source: source.clone(), series: Arc::from(*column), key: None, ts, seq, arrival_unix_ms: arrival_ms, value: v.clone() };
                    self.note_sample(&sample);
                }
            }
            self.wide_buf[table].push(WideRow { source: source.clone(), key: None, index_ms, arrival_ms, seq, values });
            if let Some(st) = self.stats.get_mut(source.as_str()) {
                st.count += 1;
                st.last_arrival_ms = arrival_ms;
                st.last_ts = ts;
            }
        }
        n
    }

    fn flush(&mut self, live: &Live) {
        for i in 0..self.wide_buf.len() {
            if self.wide_buf[i].is_empty() {
                continue;
            }
            let rows = std::mem::take(&mut self.wide_buf[i]);
            match live.store.append_wide(i, &rows) {
                Ok(()) => {
                    self.counters.rows += rows.len() as u64;
                    if let Some(r) = self.recorder.as_mut() {
                        r.take(i, &rows);
                    }
                }
                Err(e) => {
                    self.counters.store_errors += 1;
                    self.counters.last_error = Some(format!("store: {e}"));
                }
            }
        }
        if !self.frame_buf.is_empty() {
            if live.frames.receiver_count() > 0 {
                for chunk in self.frame_buf.chunks(FRAME_SAMPLES) {
                    let frame = serde_json::json!({ "type": "data", "samples": chunk }).to_string();
                    if live.frames.send(Arc::from(frame)).is_ok() {
                        self.counters.frames += 1;
                    }
                }
            }
            self.frame_buf.clear();
        }
    }

    /// A refresh now, the timer notwithstanding: the last one after a stop.
    fn refresh_now(&mut self, live: &Live) {
        self.vitals_at = None;
        self.refresh(live);
    }

    fn refresh(&mut self, live: &Live) {
        let due = self.vitals_at.is_none_or(|at| at.elapsed() >= VITALS_EVERY);
        if !due {
            return;
        }
        self.vitals_at = Some(Instant::now());
        let snap: StoreSnapshot = StoreSnapshot {
            series: Vec::new(),
            total_samples: live.store.total_samples(),
            approx_bytes: live.store.approx_bytes(),
            type_conflicts: live.store.type_conflicts(),
        };
        if let Ok(mut v) = live.vitals.write() {
            *v = StoreVitals { samples: snap.total_samples, bytes: snap.approx_bytes as u64, type_conflicts: snap.type_conflicts };
        }
        if let Ok(mut s) = live.series.write() {
            s.clear();
            for (k, e) in &self.series {
                s.insert(
                    k.clone(),
                    SeriesLast {
                        series: e.series.clone(),
                        ty: e.ty,
                        count: e.count,
                        keys: e.keys.len(),
                        keys_capped: e.keys_capped,
                        last_index_ms: e.last_index_ms,
                        last_value: e.last_value.clone(),
                    },
                );
            }
        }
        if let Ok(mut c) = live.counters.write() {
            *c = self.counters.clone();
        }
        // Health: the workers' liveness and the arrival age, on the arrival clock.
        let now_ms = now_unix_ms();
        let idle_ms = self.idle_after_ms;
        let mut reports: HashMap<String, (bool, Option<String>)> = HashMap::new();
        if let Some(set) = self.set.as_mut() {
            for r in set.worker_reports() {
                reports.insert(r.id.as_str().to_string(), (r.finished, r.error));
            }
        }
        // The outcomes the stop kept: a failed worker stays failed, a clean one is ended.
        for (id, err) in &self.ended {
            reports.insert(id.clone(), (true, err.clone()));
        }
        let mut states: HashMap<String, HealthState> = HashMap::new();
        for rt in &mut self.opc {
            let (finished, error) = match rt.handle.poll_outcome() {
                None => (false, None),
                Some(Ok(())) => (true, None),
                Some(Err(e)) => (true, Some(e.to_string())),
            };
            reports.insert(rt.spec.source_id.clone(), (finished, error));
            if let ConnState::Retrying { error, .. } = &rt.state {
                states.insert(rt.spec.source_id.clone(), HealthState::Retrying(error.to_string()));
            } else if rt.state == ConnState::Connecting {
                states.insert(rt.spec.source_id.clone(), HealthState::Waiting);
            }
        }
        for rt in &mut self.channels {
            let (finished, error) = match rt.handle.poll_outcome() {
                None => (false, None),
                Some(Ok(())) => (true, None),
                Some(Err(e)) => (true, Some(e.to_string())),
            };
            reports.insert(rt.writer.source_id.clone(), (finished, error));
            match &rt.state {
                LinkState::Trouble(why) => {
                    states.insert(rt.writer.source_id.clone(), HealthState::Retrying(why.clone()));
                }
                LinkState::Waiting => {
                    states.insert(rt.writer.source_id.clone(), HealthState::Waiting);
                }
                _ => {}
            }
        }
        // The controller source: alive while the session runs; a state stands until it changes,
        // so once one came it reads live however long since, never idle.
        if let Some(c) = &self.controller {
            reports.entry(c.source.as_str().to_string()).or_insert((false, None));
            if let Some(st) = self.stats.get(c.source.as_str())
                && st.count > 0
            {
                states.insert(c.source.as_str().to_string(), HealthState::Live);
            }
        }
        let mut health = BTreeMap::new();
        for (id, st) in &self.stats {
            let state = match reports.get(id) {
                Some((true, Some(e))) => HealthState::Failed(e.clone()),
                Some((true, None)) | None => HealthState::Ended,
                Some((false, _)) => match states.get(id) {
                    Some(s) => s.clone(),
                    None if st.count == 0 && st.last_arrival_ms == 0 => HealthState::Waiting,
                    None => {
                        let silence = now_ms - st.last_arrival_ms;
                        if silence > idle_ms { HealthState::Idle { silence_ms: silence } } else { HealthState::Live }
                    }
                },
            };
            health.insert(id.clone(), SourceHealth { state, count: st.count, last_arrival_ms: st.last_arrival_ms, last_ts: st.last_ts });
        }
        if let Ok(mut h) = live.health.write() {
            *h = health;
        }
    }

    /// One pass: drain every source's events into rows and frames, flush, refresh what is
    /// shown. Returns how many events were drained; the cap means more are pending.
    fn pump(&mut self, live: &Live) -> usize {
        self.counters.passes += 1;
        let lines = self.pump_lines(live);
        let polls = self.pump_opc();
        let messages = self.pump_channels() + self.pump_controller();
        let drained = lines + polls + messages;
        if drained > 0 {
            self.flush(live);
        }
        self.copy_at_faults(live);
        self.evict(live);
        self.record(live, false);
        self.refresh(live);
        drained
    }

    /// The recording's pass: what arrived written to the drive every `write_every`, and at the
    /// stop whatever is left; its state published.
    fn record(&mut self, live: &Live, last: bool) {
        let Some(r) = self.recorder.as_mut() else { return };
        if !last && r.written_at.elapsed() < r.write_every {
            return;
        }
        r.write();
        if last {
            // The record closed: its log folded into its file now, and the size published the one
            // the drive holds (the engine allocates the file in blocks, so a closed record may
            // stand above the size the last write measured).
            if let Err(e) = r.store.checkpoint() {
                r.status.stopped_why.get_or_insert_with(|| format!("the record's last checkpoint failed: {e}"));
            }
            r.status.written_bytes = record_bytes(&r.status.file);
        }
        if let Ok(mut s) = live.recording_status.write() {
            *s = Some(r.status.clone());
        }
    }

    /// At every fault record stored this pass, the store written to the drive (P-019: "at a
    /// fault the window is written to the drive with the fault record"): through the ingest
    /// connection, the fault's row already in it, a file of its own per fault, never over one.
    fn copy_at_faults(&mut self, live: &Live) {
        // DECISION PENDING (docs/questions.md Q41): how many copies are kept and how often one may
        // be written is not ruled; every fault gets its copy and none is deleted.
        let Some(c) = self.controller.as_mut() else { return };
        let owed = std::mem::take(&mut c.copies_owed);
        let Some(dir) = c.copy_dir.clone() else { return };
        let name = c.session_name.clone();
        for index_ms in owed {
            let at = chrono::DateTime::from_timestamp_millis(index_ms).map(|d| d.naive_utc()).unwrap_or_default();
            let base = crate::project::session_store_file(&name, at);
            let stem = base.trim_end_matches(".duckdb");
            let file = (0u32..)
                .map(|n| dir.join(if n == 0 { format!("{stem}.fault.duckdb") } else { format!("{stem}.fault-{n}.duckdb") }))
                .find(|p| !p.exists())
                .unwrap_or_else(|| dir.join(format!("{stem}.fault.duckdb")));
            let written = std::fs::create_dir_all(&dir)
                .map_err(|e| StoreError::Open(format!("create {}: {e}", dir.display())))
                .and_then(|()| live.store.copy_to_drive(&file));
            match written {
                Ok(()) => {
                    self.counters.fault_copies += 1;
                    self.counters.last_fault_copy = Some(file.display().to_string());
                }
                Err(e) => {
                    self.counters.store_errors += 1;
                    self.counters.last_error = Some(format!("store: the copy at a fault: {e}"));
                }
            }
        }
    }

    /// The window's pass, once a second: what arrived before the span let go, through the
    /// store's ingest connection, the pump's own.
    fn evict(&mut self, live: &Live) {
        let Some(span) = self.window_ms else { return };
        if self.evicted_at.is_some_and(|at| at.elapsed() < WINDOW_EVERY) {
            return;
        }
        self.evicted_at = Some(Instant::now());
        match live.store.evict_before(now_unix_ms().saturating_sub(span)) {
            Ok(n) => self.counters.evicted += n,
            Err(e) => {
                self.counters.store_errors += 1;
                self.counters.last_error = Some(format!("store: the window: {e}"));
            }
        }
    }
}
