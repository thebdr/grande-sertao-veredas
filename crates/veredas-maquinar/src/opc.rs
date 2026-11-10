//! The configurable OPC UA server: a tag list served on a real endpoint, values moving by
//! themselves, writes landing where a consumer can see them, and reactions a plant would have.
//!
//! An [`OpcConfig`] (read from a bench file) names the tags: each a string identifier in the
//! bench's namespace, a type, an access, an initial value, and optionally a draw within a range
//! on an interval. A [`Reaction`] answers a client's write of a given value into a tag with a
//! timed sequence of values, so a consumer's pulse or handshake has a plant to land on. The
//! server runs on its own thread and runtime ([`start`]); dropping the handle takes every client
//! connection with it, which is what "the machine went away" looks like from a client. The
//! handle sets and reads values on the server side (write-only tags included: the ground truth a
//! write test asserts against), and starts a [`Relay`] for a link that can go silent.
//!
//! Threading follows core's `DetachedPlant`: a current-thread runtime, requests on std channels
//! drained from a bounded `select!` loop, the runtime's drop as the teardown.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use opcua_server::address_space::{AccessLevel, NodeType, ObjectBuilder, VariableBuilder};
use opcua_server::diagnostics::NamespaceMetadata;
use opcua_server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
use opcua_server::{ServerBuilder, ServerEndpoint, ServerHandle, ServerUserToken};
use opcua_types::{DataTypeId, DataValue, NodeId, ObjectId, Variant, VariableTypeId};
use serde::Deserialize;
use tokio::net::TcpListener;
use veredas_core::opc::demo::{DemoError, Relay};
use veredas_core::rng::Pcg32;

/// The bench's data namespace.
///
/// It must differ from [`APPLICATION_URI`]: the server's built-in diagnostics node manager claims
/// the application uri as a namespace of its own and is registered first, so a collision hands
/// it this namespace's index and every client read comes back `BadNodeIdUnknown` with the nodes
/// plainly present (a trap Veredas 1 met and fenced).
pub const NAMESPACE: &str = "urn:veredas:maquinar:tags";
const APPLICATION_URI: &str = "urn:veredas:maquinar";

/// The folder every tag hangs under, by its browse name.
pub const FOLDER: &str = "Maquinar";

/// The service loop's period: how often requests, draws and reactions are serviced.
pub const TICK: Duration = Duration::from_millis(20);

/// The server, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpcConfig {
    /// The address the server binds and advertises: a client matches the endpoint url the
    /// server advertises, so this is the reachable host, never `0.0.0.0`.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// 0 lets the machine choose a free port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// A user the server accepts on the username/password path; anonymous is always accepted.
    #[serde(default)]
    pub user: Option<Credentials>,
    /// The most nodes one Read may carry; a call with more is refused whole. Absent, the stack's
    /// own limit stands.
    #[serde(default)]
    pub max_nodes_per_read: Option<usize>,
    /// The most sessions held at once, as a controller with few slots does.
    #[serde(default)]
    pub max_sessions: Option<usize>,
    #[serde(default)]
    pub tags: Vec<TagConfig>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    4855
}

impl Default for OpcConfig {
    fn default() -> OpcConfig {
        OpcConfig {
            bind: default_bind(),
            port: default_port(),
            user: None,
            max_nodes_per_read: None,
            max_sessions: None,
            tags: Vec::new(),
            reactions: Vec::new(),
        }
    }
}

/// A username and password the server accepts. The password shows in neither `Debug` nor any
/// label.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub name: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("name", &self.name).field("password", &"<hidden>").finish()
    }
}

/// One tag.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagConfig {
    /// The string identifier inside the bench's namespace; the node id is `ns=<index>;s=<id>`.
    pub id: String,
    /// The browse and display name a client sees; the id when absent.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub ty: TagType,
    #[serde(default)]
    pub access: Access,
    /// The initial (and, without a draw, permanent) value, as text, parsed per the type.
    #[serde(default)]
    pub value: String,
    /// A fresh value within a range on an interval; text has no range and takes none.
    #[serde(default)]
    pub randomize: Option<Randomize>,
    /// A value stepping through a range on an interval and wrapping, for plant motion (a place
    /// along a loop, a level filling); an int or a float, never with a draw.
    #[serde(default)]
    pub ramp: Option<Ramp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagType {
    /// An Int32.
    Int,
    /// A Double.
    Float,
    /// A Boolean.
    Bool,
    /// A String.
    Text,
}

impl TagType {
    pub fn label(self) -> &'static str {
        match self {
            TagType::Int => "int",
            TagType::Float => "float",
            TagType::Bool => "bool",
            TagType::Text => "text",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    Read,
    /// A command a controller accepts but does not report back: a read is refused.
    Write,
    ReadWrite,
}

impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
            Access::ReadWrite => "read/write",
        }
    }
}

/// A draw within `[low, high]` (an int draws whole numbers in it; a bool flips and ignores it).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Randomize {
    #[serde(default)]
    pub low: f64,
    #[serde(default)]
    pub high: f64,
    #[serde(default)]
    pub decimals: u8,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
}

fn default_interval() -> u64 {
    1000
}

/// A value stepping through `[low, high]` on an interval: each step adds `step` to the tag's
/// current value, and a value past `high` wraps by the range's width, so a place along a loop
/// comes round and a level fills, then empties and fills again. It steps from what the tag holds,
/// so a client's write or a plant-side set inside the range moves the ramp on from there; a value
/// outside it starts again at `low`. With places, every step is counted in units of the last
/// place (the range and the step whole numbers of them, refused otherwise), so the ramp reaches
/// its high exactly and never steps back.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ramp {
    pub low: f64,
    pub high: f64,
    #[serde(default = "default_step")]
    pub step: f64,
    /// A float's places after each step; absent, the sum as it falls.
    #[serde(default)]
    pub decimals: Option<u8>,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
}

fn default_step() -> f64 {
    1.0
}

/// The value after one step of a ramp from `v`: `v + step`, wrapped by the range's width while
/// past `high`; with places, counted in units of the last place (`v` rounded to them first), so
/// no float sum's last bit carries a value past its high; a `v` outside the range starts at
/// `low`.
pub fn ramp_next(v: f64, r: &Ramp) -> f64 {
    if !(v >= r.low && v <= r.high) {
        return r.low;
    }
    match r.decimals {
        Some(d) => {
            let f = 10f64.powi(d as i32);
            let (lo, hi, step) = ((r.low * f).round(), (r.high * f).round(), (r.step * f).round());
            let mut n = (v * f).round() + step;
            while n > hi {
                n -= hi - lo;
            }
            n / f
        }
        None => {
            let width = r.high - r.low;
            let mut n = v + r.step;
            while n > r.high {
                n -= width;
            }
            n
        }
    }
}

/// The most units of its places a ramp counts: 2^51. A value below it times its power of ten is
/// within a fraction of a unit of its count, so rounding gives the count back, and the width and
/// every sum of a value and a step stay below 2^53, where a float holds every whole number.
pub const EXACT_UNITS: f64 = 2_251_799_813_685_248.0;

/// Whether `x` is a whole number of units of `d` places: its count of units, divided back,
/// is `x` itself (the float a decimal of `d` places parses to), at any magnitude.
fn at_places(x: f64, d: u8) -> bool {
    let f = 10f64.powi(d as i32);
    (x * f).round() / f == x
}

/// When a draw or a ramp's step is next due, from the one due `at` and served `now`: an
/// interval after it was due, never after the tick that served it; a stall skips what it missed
/// and keeps the phase.
pub fn next_due(at: tokio::time::Instant, every: Duration, now: tokio::time::Instant) -> tokio::time::Instant {
    let mut next = at + every;
    while next <= now {
        next += every;
    }
    next
}

/// What the plant does when a client writes into a tag: a timed sequence of values, on the same
/// tag or another. With `equals` the reaction fires when the tag's value becomes that value (and
/// again each time it does); without it, on any change of the value. A change is what a client
/// wrote or the plant side set: the server's own draws and the steps of any reaction are never
/// one, so a watched tag that is drawn, or set by another reaction's step, fires nothing.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reaction {
    /// The tag watched.
    pub on_write: String,
    /// The value that fires it, as text parsed per the watched tag's type; absent, any change.
    #[serde(default)]
    pub equals: Option<String>,
    /// The tag the steps set; the watched tag when absent.
    #[serde(default)]
    pub tag: Option<String>,
    /// The sequence, each step at `after_ms` from the moment the reaction fired.
    pub then: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub after_ms: u64,
    /// The value set, as text parsed per the target tag's type.
    pub set: String,
}

/// Why the server did not start, or a configuration was refused.
#[derive(Debug, thiserror::Error)]
pub enum OpcError {
    /// The configuration is wrong, with its place named.
    #[error("{0}")]
    Config(String),
    #[error("bind {addr}: {source}")]
    Bind { addr: String, source: std::io::Error },
    #[error("local_addr: {0}")]
    LocalAddr(std::io::Error),
    #[error("build the server: {0}")]
    Build(String),
    #[error("the server has no simple node manager")]
    NoNodeManager,
    #[error("namespace {NAMESPACE} was not registered")]
    NoNamespace,
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("spawn the server thread: {0}")]
    Thread(std::io::Error),
    #[error("the server thread died on startup")]
    DiedOnStartup,
    /// A relay is only possible for a server bound on `127.0.0.1`.
    #[error("a relay needs the server on 127.0.0.1, not {0}")]
    NoRelay(String),
    #[error(transparent)]
    Relay(DemoError),
}

/// Parse a value as text into a tag's wire type: `int` an Int32, `float` a Double, `bool` one of
/// `true`/`false`/`1`/`0`/`on`/`off`/`yes`/`no` (any case), `text` as it is. Empty text is the
/// type's zero (`false`, `""`).
pub fn parse_value(ty: TagType, text: &str) -> Result<Variant, String> {
    let t = text.trim();
    Ok(match ty {
        TagType::Int => {
            if t.is_empty() {
                Variant::Int32(0)
            } else {
                Variant::Int32(t.parse().map_err(|_| format!("`{t}` is not an int"))?)
            }
        }
        TagType::Float => {
            if t.is_empty() {
                Variant::Double(0.0)
            } else {
                let v: f64 = t.parse().map_err(|_| format!("`{t}` is not a float"))?;
                if !v.is_finite() {
                    return Err(format!("`{t}` is not a finite float"));
                }
                Variant::Double(v)
            }
        }
        TagType::Bool => Variant::Boolean(match t.to_ascii_lowercase().as_str() {
            "" | "false" | "0" | "off" | "no" => false,
            "true" | "1" | "on" | "yes" => true,
            _ => return Err(format!("`{t}` is not a bool")),
        }),
        TagType::Text => Variant::from(text.to_string()),
    })
}

/// A value as a monitor shows it.
pub fn render_value(value: &Option<Variant>) -> String {
    match value {
        None => "-".into(),
        Some(Variant::Double(v)) => format!("{v}"),
        Some(Variant::Float(v)) => format!("{v}"),
        Some(Variant::Int32(v)) => format!("{v}"),
        Some(Variant::Int64(v)) => format!("{v}"),
        Some(Variant::UInt32(v)) => format!("{v}"),
        Some(Variant::Boolean(v)) => format!("{v}"),
        Some(Variant::String(s)) => s.as_ref().to_string(),
        Some(Variant::Empty) => "(empty)".into(),
        Some(other) => format!("{other:?}"),
    }
}

/// A compiled reaction: its tag indices and parsed values.
#[derive(Clone, Debug)]
struct CompiledReaction {
    watched: String,
    equals: Option<Variant>,
    target: String,
    steps: Vec<(Duration, Variant)>,
}

/// Validate a configuration before anything is started, naming what is wrong: a duplicate or
/// empty id, an empty bind, a value or a draw that does not fit its type, a reaction on an
/// unknown tag or with no steps. Checked up front rather than while the address space is built,
/// so a bench says what is wrong instead of a server half-starting. Returns how many reactions
/// compiled.
pub fn validate(cfg: &OpcConfig) -> Result<usize, OpcError> {
    compile(cfg).map(|r| r.len())
}

fn compile(cfg: &OpcConfig) -> Result<Vec<CompiledReaction>, OpcError> {
    let bad = |m: String| OpcError::Config(m);
    if cfg.bind.trim().is_empty() {
        return Err(bad("opc.bind is empty".into()));
    }
    if let Some(user) = &cfg.user
        && user.name.trim().is_empty()
    {
        return Err(bad("opc.user.name is empty".into()));
    }
    if cfg.max_nodes_per_read == Some(0) {
        return Err(bad("opc.max_nodes_per_read is 0: no Read could carry a node".into()));
    }
    if cfg.max_sessions == Some(0) {
        return Err(bad("opc.max_sessions is 0: no client could connect".into()));
    }
    let mut seen: Vec<&str> = Vec::new();
    for (i, tag) in cfg.tags.iter().enumerate() {
        let id = tag.id.trim();
        let place = format!("opc.tags[{i}] (`{id}`)");
        if id.is_empty() {
            return Err(bad(format!("opc.tags[{i}]: the tag has no id")));
        }
        if seen.contains(&id) {
            return Err(bad(format!("{place}: two tags share this id")));
        }
        seen.push(id);
        let initial = parse_value(tag.ty, &tag.value).map_err(|e| bad(format!("{place}.value: {e}")))?;
        if let Some(r) = &tag.ramp {
            if tag.randomize.is_some() {
                return Err(bad(format!("{place}: a tag takes a draw or a ramp, not both")));
            }
            if !matches!(tag.ty, TagType::Int | TagType::Float) {
                return Err(bad(format!("{place}.ramp: a {} has no range to step through", tag.ty.label())));
            }
            if !(r.low.is_finite() && r.high.is_finite() && r.step.is_finite()) {
                return Err(bad(format!("{place}.ramp: the range and the step are not all finite")));
            }
            if r.low >= r.high {
                return Err(bad(format!("{place}.ramp: the range is empty ({} is not below {})", r.low, r.high)));
            }
            if !(r.high - r.low).is_finite() {
                return Err(bad(format!("{place}.ramp: the range's width ({:?}..={:?}) is past what a float holds", r.low, r.high)));
            }
            if !(r.step > 0.0 && r.step <= r.high - r.low) {
                return Err(bad(format!("{place}.ramp: the step {} is not above 0 and at most the range's width {}", r.step, r.high - r.low)));
            }
            if tag.ty == TagType::Int {
                let whole = |x: f64| x.fract() == 0.0 && x >= i32::MIN as f64 && x <= i32::MAX as f64;
                if !(whole(r.low) && whole(r.high) && whole(r.step)) {
                    return Err(bad(format!("{place}.ramp: an int steps by whole numbers inside an int ({}..={} by {})", r.low, r.high, r.step)));
                }
                if r.decimals.is_some() {
                    return Err(bad(format!("{place}.ramp.decimals: an int has no places")));
                }
            }
            if let Some(d) = r.decimals
                && d > 15
            {
                return Err(bad(format!("{place}.ramp.decimals {d} is past 15")));
            }
            if let Some(d) = r.decimals {
                for (key, x) in [("low", r.low), ("high", r.high), ("step", r.step)] {
                    if (x * 10f64.powi(d as i32)).abs() > EXACT_UNITS {
                        return Err(bad(format!("{place}.ramp.{key} {x} at {d} places is past what a float counts exactly")));
                    }
                    if !at_places(x, d) {
                        return Err(bad(format!("{place}.ramp.{key} {x} is not a whole number of the ramp's places ({d})")));
                    }
                }
            }
            let v = match &initial {
                Variant::Int32(x) => *x as f64,
                Variant::Double(x) => *x,
                _ => r.low,
            };
            if v < r.low || v > r.high {
                return Err(bad(format!("{place}.ramp: the initial value {v} lies outside {}..={}", r.low, r.high)));
            }
            if r.interval_ms == 0 {
                return Err(bad(format!("{place}.ramp.interval_ms is 0")));
            }
            if r.interval_ms < TICK.as_millis() as u64 {
                return Err(bad(format!("{place}.ramp.interval_ms {} is shorter than the server's tick of {} ms", r.interval_ms, TICK.as_millis())));
            }
        }
        if let Some(r) = &tag.randomize {
            if tag.ty == TagType::Text {
                return Err(bad(format!("{place}.randomize: text has no range to draw from")));
            }
            if !r.low.is_finite() || !r.high.is_finite() {
                return Err(bad(format!("{place}.randomize: the range is not finite")));
            }
            if r.low > r.high {
                return Err(bad(format!("{place}.randomize: the range is empty ({} > {})", r.low, r.high)));
            }
            if tag.ty == TagType::Int {
                let (lo, hi) = int_bounds(r);
                if lo > hi {
                    return Err(bad(format!("{place}.randomize: no whole number lies in {}..={}", r.low, r.high)));
                }
                if lo < i32::MIN as i64 || hi > i32::MAX as i64 {
                    return Err(bad(format!("{place}.randomize: {}..={} is outside an int", r.low, r.high)));
                }
            }
            if r.interval_ms == 0 {
                return Err(bad(format!("{place}.randomize.interval_ms is 0")));
            }
            // At most one draw comes per tick: an interval the tick could never serve is
            // refused rather than silently drawn once a tick.
            if r.interval_ms < TICK.as_millis() as u64 {
                return Err(bad(format!("{place}.randomize.interval_ms {} is shorter than the server's tick of {} ms", r.interval_ms, TICK.as_millis())));
            }
        }
    }
    let find = |id: &str| cfg.tags.iter().find(|t| t.id.trim() == id.trim());
    let mut out = Vec::new();
    for (i, r) in cfg.reactions.iter().enumerate() {
        let place = format!("opc.reactions[{i}]");
        let watched = find(&r.on_write).ok_or_else(|| bad(format!("{place}.on_write: no tag `{}`", r.on_write)))?;
        let target = match &r.tag {
            Some(t) => find(t).ok_or_else(|| bad(format!("{place}.tag: no tag `{t}`")))?,
            None => watched,
        };
        let equals = match &r.equals {
            Some(text) => Some(parse_value(watched.ty, text).map_err(|e| bad(format!("{place}.equals: {e}")))?),
            None => None,
        };
        if r.then.is_empty() {
            return Err(bad(format!("{place}.then is empty: a reaction sets something")));
        }
        let mut steps = Vec::new();
        for (j, step) in r.then.iter().enumerate() {
            let value =
                parse_value(target.ty, &step.set).map_err(|e| bad(format!("{place}.then[{j}].set: {e}")))?;
            steps.push((Duration::from_millis(step.after_ms), value));
        }
        out.push(CompiledReaction {
            watched: watched.id.trim().to_string(),
            equals,
            target: target.id.trim().to_string(),
            steps,
        });
    }
    Ok(out)
}

/// One row of a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct TagValue {
    pub id: String,
    /// `None` when the node holds no value.
    pub value: Option<Variant>,
}

/// What the server reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpcStatus {
    /// Draws applied to randomized tags.
    pub draws: u64,
    /// Steps applied to ramped tags.
    pub ramp_steps: u64,
    /// Reactions fired.
    pub reactions_fired: u64,
    /// Writes of the wrong wire type refused (`BadTypeMismatch`, from a client) or dropped (a
    /// plant-side `set`): the stack's own validation would have taken them, so the bench checks
    /// and counts them here, where a consumer's test can see its mistake.
    pub type_mismatches: u64,
    /// The last mismatch: the tag and the variant it was given.
    pub last_mismatch: Option<(String, String)>,
}

/// Whether a variant is of a tag's wire type.
fn is_of_type(ty: TagType, v: &Variant) -> bool {
    matches!(
        (ty, v),
        (TagType::Int, Variant::Int32(_)) | (TagType::Float, Variant::Double(_)) | (TagType::Bool, Variant::Boolean(_)) | (TagType::Text, Variant::String(_))
    )
}

enum Request {
    Set(String, Variant),
    Get(String, Sender<Option<Variant>>),
    Snapshot(Sender<Vec<TagValue>>),
    Status(Sender<OpcStatus>),
}

/// A running server. Dropping it stops the server, closes every connection and frees the port.
pub struct OpcHandle {
    /// What a client connects to: `opc.tcp://<bind>:<port>/`.
    pub endpoint_url: String,
    /// The namespace index the server assigned: asked for, never assumed.
    pub namespace: u16,
    pub port: u16,
    bind: String,
    requests: Sender<Request>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl OpcHandle {
    /// A tag's node id in the `ns=N;s=...` form a configuration holds.
    pub fn node_id_str(&self, id: &str) -> String {
        format!("ns={};s={id}", self.namespace)
    }

    /// Set a value from the plant side; applied within a tick. A tag the server does not hold
    /// is ignored.
    pub fn set(&self, id: &str, value: impl Into<Variant>) {
        let _ = self.requests.send(Request::Set(id.to_string(), value.into()));
    }

    /// Read a tag on the server side, write-only tags included; `None` when the tag holds no
    /// value, is not there, or the server did not answer within 5 s.
    pub fn get(&self, id: &str) -> Option<Variant> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.requests.send(Request::Get(id.to_string(), tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(5)).ok()?
    }

    /// Every configured tag's value, in one round trip; empty when the server did not answer
    /// within 5 s.
    pub fn snapshot(&self) -> Vec<TagValue> {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.requests.send(Request::Snapshot(tx)).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
    }

    pub fn status(&self) -> OpcStatus {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.requests.send(Request::Status(tx)).is_err() {
            return OpcStatus::default();
        }
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
    }

    /// A relay in front of this server: a link that can be frozen (core's [`Relay`], listening
    /// on `127.0.0.2` at this server's port). Only for a server bound on `127.0.0.1`.
    pub fn relay(&self) -> Result<Relay, OpcError> {
        if self.bind != "127.0.0.1" {
            return Err(OpcError::NoRelay(self.bind.clone()));
        }
        Relay::start(self.port).map_err(OpcError::Relay)
    }

    fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Stop the server and wait for the port to be released.
    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for OpcHandle {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl std::fmt::Debug for OpcHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpcHandle")
            .field("endpoint_url", &self.endpoint_url)
            .field("namespace", &self.namespace)
            .field("port", &self.port)
            .finish()
    }
}

type Ready = Result<(String, u16, u16), OpcError>;

/// Start the server. Blocks until it is listening (or failed), so a caller holding a handle
/// knows the port is really claimed.
pub fn start(cfg: &OpcConfig) -> Result<OpcHandle, OpcError> {
    let reactions = compile(cfg)?;
    let cfg = cfg.clone();
    let bind = cfg.bind.trim().to_string();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (req_tx, req_rx) = std::sync::mpsc::channel::<Request>();

    let thread = std::thread::Builder::new()
        .name("maquinar-opc".into())
        .spawn(move || {
            let _timer = crate::timer::FineTimer::hold();
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(OpcError::Runtime(e)));
                    return;
                }
            };
            rt.block_on(async move {
                let status = Arc::new(std::sync::Mutex::new(OpcStatus::default()));
                let (handle, nodes, endpoint_url, bound, ns) = match serve(&cfg, Arc::clone(&status)).await {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if ready_tx.send(Ok((endpoint_url, bound, ns))).is_err() {
                    return;
                }
                service_loop(handle, nodes, cfg, reactions, ns, req_rx, shutdown_rx, status).await;
            });
            // Dropping the runtime here takes every client connection with it.
        })
        .map_err(OpcError::Thread)?;

    let (endpoint_url, port, namespace) = ready_rx.recv().map_err(|_| OpcError::DiedOnStartup)??;
    Ok(OpcHandle {
        endpoint_url,
        namespace,
        port,
        bind,
        requests: req_tx,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

type Started = (ServerHandle, Arc<SimpleNodeManager>, String, u16, u16);

async fn serve(cfg: &OpcConfig, status: Arc<std::sync::Mutex<OpcStatus>>) -> Result<Started, OpcError> {
    let host = cfg.bind.trim();
    // Bind first, then tell the server the port it really got: with port 0 the advertised
    // endpoint urls would otherwise disagree with reality and a client's match would find nothing.
    let addr = format!("{host}:{}", cfg.port);
    let listener = TcpListener::bind((host, cfg.port)).await.map_err(|source| OpcError::Bind { addr, source })?;
    let bound = listener.local_addr().map_err(OpcError::LocalAddr)?.port();

    let mut users = vec![opcua_server::ANONYMOUS_USER_TOKEN_ID.to_string()];
    let mut builder = ServerBuilder::new()
        .application_name("veredas maquinar")
        .application_uri(APPLICATION_URI)
        .product_uri(APPLICATION_URI)
        .host(host)
        .port(bound)
        .discovery_urls(vec!["/".to_owned()]);
    if let Some(user) = &cfg.user {
        builder = builder.add_user_token(&user.name, ServerUserToken::user_pass(&user.name, &user.password));
        users.push(user.name.clone());
    }
    builder = builder
        .add_endpoint("none", ServerEndpoint::new_none("/", &users))
        .default_endpoint("none")
        .with_node_manager(simple_node_manager(
            NamespaceMetadata { namespace_uri: NAMESPACE.to_owned(), ..Default::default() },
            "maquinar",
        ));
    if let Some(n) = cfg.max_sessions {
        builder = builder.max_sessions(n);
    }
    if let Some(n) = cfg.max_nodes_per_read {
        builder.limits_mut().operational.max_nodes_per_read = n;
    }
    let (server, handle) = builder.build().map_err(|e| OpcError::Build(e.to_string()))?;

    let nodes = handle.node_managers().get_of_type::<SimpleNodeManager>().ok_or(OpcError::NoNodeManager)?;
    let ns = handle.get_namespace_index(NAMESPACE).ok_or(OpcError::NoNamespace)?;

    populate(&nodes, ns, &cfg.tags)?;
    guard_writes(&nodes, &handle, ns, &cfg.tags, status);
    tokio::spawn(server.run_with(listener));

    Ok((handle, nodes, format!("opc.tcp://{host}:{bound}/"), bound, ns))
}

/// A write callback on every writable tag: the stack's own validation takes any scalar into
/// any typed tag, so the bench checks the wire type itself and answers `BadTypeMismatch`, as a
/// real controller does. A value of the right type is stored: the callback runs under the
/// address-space lock and cannot store it itself, so it hands the value to a task on the
/// server's runtime that lands it the moment the lock is free, before the client's next request
/// can arrive. A write with an index range is refused.
fn guard_writes(nodes: &Arc<SimpleNodeManager>, handle: &ServerHandle, ns: u16, tags: &[TagConfig], status: Arc<std::sync::Mutex<OpcStatus>>) {
    for tag in tags.iter().filter(|t| t.access != Access::Read) {
        let id = tag.id.trim().to_string();
        let node = NodeId::new(ns, id.as_str());
        let ty = tag.ty;
        let nodes = Arc::clone(nodes);
        let subscriptions = handle.subscriptions().clone();
        let status = Arc::clone(&status);
        nodes.clone().inner().add_write_callback(node.clone(), move |value, range| {
            if *range != opcua_types::NumericRange::None {
                return opcua_types::StatusCode::BadIndexRangeInvalid;
            }
            let Some(variant) = value.value else { return opcua_types::StatusCode::BadTypeMismatch };
            if !is_of_type(ty, &variant) {
                if let Ok(mut s) = status.lock() {
                    s.type_mismatches += 1;
                    s.last_mismatch = Some((id.clone(), format!("{variant:?}")));
                }
                return opcua_types::StatusCode::BadTypeMismatch;
            }
            let (nodes, subscriptions, node) = (Arc::clone(&nodes), subscriptions.clone(), node.clone());
            tokio::spawn(async move {
                let _ = nodes.set_value(&subscriptions, &node, None, DataValue::new_now(variant));
            });
            opcua_types::StatusCode::Good
        });
    }
}

/// Build the address space from the configured tags.
fn populate(nodes: &SimpleNodeManager, ns: u16, tags: &[TagConfig]) -> Result<(), OpcError> {
    let mut space = nodes.address_space().write();
    let folder = NodeId::new(ns, "maquinar");
    ObjectBuilder::new(&folder, FOLDER, FOLDER).organized_by(ObjectId::ObjectsFolder).insert(&mut *space);

    for tag in tags {
        let id = tag.id.trim();
        let name = tag.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(id);
        let value = parse_value(tag.ty, &tag.value).map_err(OpcError::Config)?;
        let builder = VariableBuilder::new(&NodeId::new(ns, id), name, name)
            // Not decoration: a variable without a type definition browses with a null one, and
            // a browsing client reads it to tell a plain variable from a structured one.
            .has_type_definition(VariableTypeId::BaseDataVariableType)
            .value(value)
            .organized_by(folder.clone());
        let builder = match tag.ty {
            TagType::Int => builder.data_type(DataTypeId::Int32),
            TagType::Float => builder.data_type(DataTypeId::Double),
            TagType::Bool => builder.data_type(DataTypeId::Boolean),
            TagType::Text => builder.data_type(DataTypeId::String),
        };
        let builder = match tag.access {
            Access::Read => builder,
            Access::ReadWrite => builder.writable(),
            // Set explicitly rather than through `writable()`, which ORs write onto the
            // default read.
            Access::Write => {
                builder.access_level(AccessLevel::CURRENT_WRITE).user_access_level(AccessLevel::CURRENT_WRITE)
            }
        };
        builder.insert(&mut *space);
    }
    Ok(())
}

/// Read a node's value straight from the address space.
fn read_node(nodes: &SimpleNodeManager, node: &NodeId) -> Option<Variant> {
    let space = nodes.address_space().read();
    match space.find(node)? {
        NodeType::Variable(v) => {
            v.value(
                opcua_types::TimestampsToReturn::Neither,
                &opcua_types::NumericRange::None,
                &opcua_types::DataEncoding::Binary,
                0.0,
            )
            .value
        }
        _ => None,
    }
}

/// The whole numbers an int draw may take: from the ceiling of `low` to the floor of `high`.
fn int_bounds(r: &Randomize) -> (i64, i64) {
    (r.low.ceil() as i64, r.high.floor() as i64)
}

fn draw(tag: &TagConfig, r: &Randomize, rng: &mut Pcg32) -> Variant {
    match tag.ty {
        TagType::Float => Variant::Double(rng.uniform(r.low, r.high, r.decimals)),
        TagType::Int => {
            // The bounds were checked to hold a whole number inside an int at validation.
            let (lo, hi) = int_bounds(r);
            Variant::Int32(rng.int_in(lo, hi).clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        }
        TagType::Bool => Variant::Boolean(rng.flip()),
        // Refused by `validate`; left static if reached.
        TagType::Text => Variant::from(tag.value.clone()),
    }
}

/// Service requests, move the randomized tags and fire reactions until shutdown. One loop does
/// everything, on a tick that keeps its period whatever the body took, so every await is bounded
/// and there is no timer bookkeeping to leak.
#[allow(clippy::too_many_arguments)]
async fn service_loop(
    handle: ServerHandle,
    nodes: Arc<SimpleNodeManager>,
    cfg: OpcConfig,
    reactions: Vec<CompiledReaction>,
    ns: u16,
    requests: Receiver<Request>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
    status: Arc<std::sync::Mutex<OpcStatus>>,
) {
    let mut rng = Pcg32::from_entropy(0xA1);
    let start = tokio::time::Instant::now();
    let node = |id: &str| NodeId::new(ns, id);
    // Next-due per randomized tag, so a slow tick cannot make a fast tag drift.
    let mut due: Vec<Option<tokio::time::Instant>> = cfg
        .tags
        .iter()
        .map(|t| t.randomize.as_ref().map(|r| start + Duration::from_millis(r.interval_ms)))
        .collect();
    // Next-due per ramped tag, scheduled the same way.
    let mut ramp_due: Vec<Option<tokio::time::Instant>> = cfg
        .tags
        .iter()
        .map(|t| t.ramp.as_ref().map(|r| start + Duration::from_millis(r.interval_ms)))
        .collect();
    // What each watched tag held at the last tick, so a reaction is edge-triggered.
    let mut last_seen: Vec<Option<Variant>> = reactions.iter().map(|r| read_node(&nodes, &node(&r.watched))).collect();
    // Steps scheduled: when, which node, what value.
    let mut pending: Vec<(tokio::time::Instant, NodeId, Variant)> = Vec::new();
    let mut shutdown = shutdown;
    let set = |id: &NodeId, value: Variant| {
        let _ = nodes.set_value(handle.subscriptions(), id, None, DataValue::new_now(value));
    };
    // The tick lands at the host's timer quantum, held at a millisecond while the bench runs
    // (timer::FineTimer; docs/questions.md Q13, decided 2026-11-09), so 20 ms on both platforms;
    // the rates hold anyway,
    // since every due time is scheduled from the last due time, never from the tick.
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = interval.tick() => {
                while let Ok(req) = requests.try_recv() {
                    match req {
                        Request::Set(id, value) => {
                            // A plant-side set of the wrong type is a mistake of the bench's
                            // caller: counted and dropped, like a client's is refused.
                            match cfg.tags.iter().find(|t| t.id.trim() == id.trim()) {
                                Some(t) if !is_of_type(t.ty, &value) => {
                                    if let Ok(mut s) = status.lock() {
                                        s.type_mismatches += 1;
                                        s.last_mismatch = Some((t.id.trim().to_string(), format!("{value:?}")));
                                    }
                                }
                                _ => set(&node(&id), value),
                            }
                        }
                        Request::Get(id, reply) => {
                            let _ = reply.send(read_node(&nodes, &node(&id)));
                        }
                        Request::Snapshot(reply) => {
                            let rows = cfg
                                .tags
                                .iter()
                                .map(|t| TagValue { id: t.id.trim().to_string(), value: read_node(&nodes, &node(t.id.trim())) })
                                .collect();
                            let _ = reply.send(rows);
                        }
                        Request::Status(reply) => {
                            let _ = reply.send(status.lock().map(|s| s.clone()).unwrap_or_default());
                        }
                    }
                }
                let now = tokio::time::Instant::now();
                for (i, r) in reactions.iter().enumerate() {
                    let current = read_node(&nodes, &node(&r.watched));
                    let fired = match &r.equals {
                        Some(v) => current.as_ref() == Some(v) && last_seen[i].as_ref() != Some(v),
                        None => current != last_seen[i],
                    };
                    if fired {
                        if let Ok(mut s) = status.lock() {
                            s.reactions_fired += 1;
                        }
                        for (after, value) in &r.steps {
                            pending.push((now + *after, node(&r.target), value.clone()));
                        }
                    }
                    last_seen[i] = current;
                }
                // Steps are applied in the order scheduled; a step due now lands this tick, so
                // the watched value it sets is what the next tick compares against.
                let mut i = 0;
                while i < pending.len() {
                    if pending[i].0 <= now {
                        let (_, id, value) = pending.remove(i);
                        set(&id, value);
                    } else {
                        i += 1;
                    }
                }
                for (i, tag) in cfg.tags.iter().enumerate() {
                    let (Some(r), Some(at)) = (&tag.randomize, due[i]) else { continue };
                    if now < at {
                        continue;
                    }
                    set(&node(tag.id.trim()), draw(tag, r, &mut rng));
                    if let Ok(mut s) = status.lock() {
                        s.draws += 1;
                    }
                    // The next draw is due an interval after this one was due, never after the
                    // tick that served it: a due time set from the tick's clock falls just past
                    // a tick boundary and waits one more, stretching every interval by a tick.
                    // A stall skips the draws it missed and keeps the phase.
                    due[i] = Some(next_due(at, Duration::from_millis(r.interval_ms), now));
                }
                for (i, tag) in cfg.tags.iter().enumerate() {
                    let (Some(r), Some(at)) = (&tag.ramp, ramp_due[i]) else { continue };
                    if now < at {
                        continue;
                    }
                    let id = node(tag.id.trim());
                    let current = match read_node(&nodes, &id) {
                        Some(Variant::Int32(x)) => x as f64,
                        Some(Variant::Double(x)) => x,
                        _ => f64::NAN,
                    };
                    let next = ramp_next(current, r);
                    set(&id, if tag.ty == TagType::Int { Variant::Int32(next as i32) } else { Variant::Double(next) });
                    if let Ok(mut s) = status.lock() {
                        s.ramp_steps += 1;
                    }
                    // As a draw: due an interval after this one was due, a stall skipping the
                    // steps it missed and keeping the phase.
                    ramp_due[i] = Some(next_due(at, Duration::from_millis(r.interval_ms), now));
                }
                for (i, r) in reactions.iter().enumerate() {
                    // A step or a draw this tick changed the watched value: seen, not a write.
                    last_seen[i] = read_node(&nodes, &node(&r.watched));
                }
            }
        }
    }
}
