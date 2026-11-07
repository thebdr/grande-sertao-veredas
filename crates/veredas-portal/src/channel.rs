//! A portal telegram channel: one TCP link to a device, configured by rules and never by code.
//!
//! A device spoken over TCP is a channel file (`docs/channel-schema.md`: how its frames are framed
//! and the link held), a catalog (`docs/catalog-schema.md`: its messages by name, id and body
//! rules) and the gateway's own reply rules, plain text in a project folder (`docs/questions.md`
//! Q8). The channel asks the catalog whether its ids can ride the framing before it binds
//! (`Catalog::check_framing`), places and reads ids by the catalog (`frame`, `messages_of`),
//! reads each received message into named fields by the catalog's rules, and shapes a row of a
//! table named for the message, typed by the catalog's field types. It composes a reply by the
//! catalog (`encode`) when a reply rule says so and the master write switch is on; with the
//! switch off it reads and records and answers nothing, which is how a new channel goes in front
//! of a live device. The handle's `send` is the second write path of the same gate.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use veredas_core::opc::worker::WriteGate;
use veredas_core::telegram::capture::FieldType;
use veredas_core::telegram::channel::{Channel, ChannelConfig as WireConfig, ChannelEvent as WireEvent, ChannelStatus as WireStatus};
use veredas_core::telegram::{Catalog, ChannelSpec, Message, PeerId, Role};
use veredas_core::{SeriesType, SourceError, SourceHandle, SourceId, Value, now_unix_ms};

use crate::source::stop_flag;

/// The service loop's period: how long the worker waits on the wire before it looks at its
/// requests and its stop flag.
pub const TICK: Duration = Duration::from_millis(20);
/// The most columns a repeat may add to a message's table (`name1` to `name<max>`); a catalog
/// may declare a repeat of millions, a table cannot hold them.
pub const MAX_REPEAT_COLUMNS: usize = 64;
/// How long the handle waits for the worker to answer a send before it reports it gone.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// A constant written into a reply field, as a TOML value: a number, a flag or text, written as
/// text on the wire.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Literal {
    pub fn text(&self) -> String {
        match self {
            Literal::Int(i) => i.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::Bool(b) => (*b as u8).to_string(),
            Literal::Str(s) => s.clone(),
        }
    }
}

/// One field of a reply: a name the sent message writes, taken from a field of the message
/// answered (`from`) or a constant (`value`), exactly one of the two.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyField {
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub value: Option<Literal>,
}

/// A reply rule: when a message named `on` arrives, send the message named `send` back to the
/// peer it came from, its fields rendered from the incoming message's fields and constants.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyRule {
    pub on: String,
    pub send: String,
    #[serde(default)]
    pub fields: Vec<ReplyField>,
}

/// Everything one channel's worker needs: who it is, which side of the link, the channel file,
/// the catalog, and the replies it gives.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    pub source_id: String,
    pub display_name: String,
    pub role: Role,
    pub spec: ChannelSpec,
    pub catalog: Catalog,
    pub replies: Vec<ReplyRule>,
}

/// Why a channel did not start: every refusal names its place, and nothing has bound.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChannelError {
    /// The catalog's ids cannot ride the channel file's framing (the catalog's own reason).
    #[error("channel `{source_id}`: {reason}")]
    Catalog { source_id: String, reason: String },
    #[error("channel `{source_id}`, replies[{index}]: the catalog has no message `{message}`")]
    UnknownMessage { source_id: String, index: usize, message: String },
    #[error("channel `{source_id}`, replies[{index}]: `{send}` needs a value for `{name}`, which no field gives")]
    Unwritten { source_id: String, index: usize, send: String, name: String },
    #[error("channel `{source_id}`, replies[{index}]: `{send}` writes no field named `{name}`")]
    Unknown { source_id: String, index: usize, send: String, name: String },
    #[error("channel `{source_id}`, replies[{index}]: the field `{name}` is given twice")]
    Twice { source_id: String, index: usize, name: String },
    #[error("channel `{source_id}`, replies[{index}]: the field `{name}` needs exactly one of `from` and `value`")]
    OneSource { source_id: String, index: usize, name: String },
    #[error("channel `{source_id}`, replies[{index}]: `{on}` does not read a field named `{from}`, so `{name}` cannot be taken from it")]
    NotRead { source_id: String, index: usize, on: String, from: String, name: String },
    #[error("channel `{source_id}`: the message `{message}` repeats `{repeat}` up to {max} times, more than the {MAX_REPEAT_COLUMNS} columns a table holds")]
    TooManyColumns { source_id: String, message: String, repeat: String, max: usize },
    #[error("channel `{0}`: spawn the worker thread: {1}")]
    Thread(String, String),
}

/// Why a send through the handle did not go out. Nothing reached the wire in any of these.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum SendError {
    /// The master switch is off.
    #[error("writes are turned off for this project")]
    Disarmed,
    #[error("the catalog has no message `{0}`")]
    UnknownMessage(String),
    /// The catalog refused to write the body (a value missing or not fitting).
    #[error("`{message}`: {reason}")]
    Compose { message: String, reason: String },
    /// A body that renders empty in a layout without segments is a keep-alive on the wire.
    #[error("`{0}`: an empty body is a keep-alive in a layout without segments")]
    EmptyBody(String),
    /// The framing refused the frame (a body over the frame limit).
    #[error("`{message}`: {reason}")]
    Framing { message: String, reason: String },
    /// The peer named is not connected.
    #[error("peer {0} is not connected")]
    NoPeer(PeerId),
    /// Nobody is connected to send to.
    #[error("no peer is connected")]
    NoPeers,
    /// The worker has ended, or did not answer in time.
    #[error("the channel worker is gone")]
    WorkerGone,
}

/// How the link stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Listening with nobody connected, or dialling and not yet through.
    Waiting,
    /// At least one peer is connected.
    Linked { peers: usize },
    /// The wire reported trouble (a bind failure, a dial that keeps failing); the channel stays
    /// and keeps trying.
    Trouble(String),
    Stopped,
}

/// What became of a reply a rule would have sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// Composed and handed to the wire.
    Sent,
    /// The master switch is off: nothing composed, nothing sent.
    Withheld,
    /// The rule could not be rendered or the catalog refused the body: nothing sent.
    Refused(String),
}

/// What a channel worker sends upstream.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelEvent {
    /// The link changed state: the source's health, not an error path.
    Status { source_id: String, state: LinkState },
    /// A message the catalog knows, read into its fields.
    Message {
        source_id: String,
        peer: PeerId,
        /// Where the message sits on the plant's time axis: the plant-local wall clock as naive
        /// milliseconds, the axis a stamp parsed from a plant's own text lands on.
        index_ms: i64,
        /// When it arrived on the unix clock.
        arrival_ms: i64,
        name: String,
        id: u64,
        fields: HashMap<String, String>,
        /// The frame exactly as it arrived, so the event can be decoded again later.
        raw: Vec<u8>,
    },
    /// A body the catalog could not read: an id it does not hold (`id` the frame's, or none
    /// when the frame carried no id), or a body its rules refused.
    Unreadable { source_id: String, peer: PeerId, arrival_ms: i64, id: Option<u64>, body: Vec<u8>, raw: Vec<u8> },
    /// A keep-alive: the link is alive though quiet.
    KeepAlive { source_id: String, peer: PeerId, arrival_ms: i64 },
    /// A reply rule met its message.
    Reply { source_id: String, peer: PeerId, on: String, send: String, outcome: ReplyOutcome },
}

/// Live counters and state, for a status pane.
#[derive(Clone, Debug, Default)]
pub struct ChannelStatus {
    /// The link, as core's channel reports it.
    pub wire: WireStatus,
    pub peers: Vec<PeerId>,
    /// Messages received and read, by name.
    pub received: BTreeMap<String, u64>,
    /// Messages sent (replies and sends), by name.
    pub sent: BTreeMap<String, u64>,
    /// Frames carrying an id the catalog does not hold, or none.
    pub unknown_ids: u64,
    /// Bodies the catalog's rules refused.
    pub undecodable: u64,
    pub replies_sent: u64,
    /// Replies a rule would have sent while the switch was off.
    pub replies_withheld: u64,
    /// Replies a rule could not render or the catalog refused.
    pub replies_refused: u64,
    pub last_error: Option<String>,
}

/// The table a message's rows land in: one column per name the message reads, typed by the
/// catalog (an integer field an `Int`, text, hex, a payload group and a repeat's numbered
/// names `Text`), in the catalog's order: the binary fields, the payload's groups, the repeats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageTable {
    pub name: String,
    pub columns: Vec<(String, SeriesType)>,
}

impl MessageTable {
    /// The table of a message, or a refusal for a repeat too wide for a table.
    pub fn of(source_id: &str, message: &Message) -> Result<MessageTable, ChannelError> {
        let mut columns: Vec<(String, SeriesType)> = Vec::new();
        for f in &message.capture_spec.fields {
            let ty = match f.ty {
                FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::I16 | FieldType::I32 => SeriesType::Int,
                FieldType::Ascii | FieldType::Hex => SeriesType::Text,
            };
            columns.push((f.name.clone(), ty));
        }
        if let Some(payload) = &message.capture_spec.payload
            && let Ok(re) = regex::Regex::new(payload)
        {
            // The catalog compiled this pattern when it loaded; its named groups are what the
            // body reads into.
            for g in re.capture_names().flatten() {
                if !columns.iter().any(|(n, _)| n == g) {
                    columns.push((g.to_string(), SeriesType::Text));
                }
            }
        }
        for r in &message.capture_spec.repeat {
            if r.max > MAX_REPEAT_COLUMNS {
                return Err(ChannelError::TooManyColumns {
                    source_id: source_id.to_string(),
                    message: message.name.clone(),
                    repeat: r.name.clone(),
                    max: r.max,
                });
            }
            for i in 1..=r.max.max(1) {
                columns.push((format!("{}{i}", r.name), SeriesType::Text));
            }
        }
        Ok(MessageTable { name: message.name.clone(), columns })
    }

    /// One row from a message's fields, positional against the columns: a name the fields lack
    /// (an optional group that did not match, a repeat past what the body held) is a hole, and so
    /// is an integer column whose text is not a number.
    pub fn row(&self, fields: &HashMap<String, String>) -> Vec<Option<Value>> {
        self.columns
            .iter()
            .map(|(name, ty)| {
                let text = fields.get(name)?;
                match ty {
                    SeriesType::Int => text.trim().parse::<i64>().ok().map(Value::Int),
                    SeriesType::Float => text.trim().parse::<f64>().ok().map(Value::Float),
                    SeriesType::Text => Some(Value::Text(Arc::from(text.as_str()))),
                    SeriesType::Time => None,
                }
            })
            .collect()
    }
}

/// A reply rule compiled against the catalog.
#[derive(Clone, Debug)]
struct Reply {
    on: String,
    send: String,
    fields: Vec<(String, Rendering)>,
}

#[derive(Clone, Debug)]
enum Rendering {
    From(String),
    Value(String),
}

fn compile_replies(cfg: &ChannelConfig) -> Result<Vec<Reply>, ChannelError> {
    let sid = &cfg.source_id;
    let mut out = Vec::new();
    for (index, rule) in cfg.replies.iter().enumerate() {
        let unknown = |message: &str| ChannelError::UnknownMessage { source_id: sid.clone(), index, message: message.to_string() };
        let on = cfg.catalog.by_name(&rule.on).ok_or_else(|| unknown(&rule.on))?;
        let send = cfg.catalog.by_name(&rule.send).ok_or_else(|| unknown(&rule.send))?;
        let needs = send.names();
        let mut fields: Vec<(String, Rendering)> = Vec::new();
        for f in &rule.fields {
            if fields.iter().any(|(n, _)| *n == f.name) {
                return Err(ChannelError::Twice { source_id: sid.clone(), index, name: f.name.clone() });
            }
            if !needs.contains(&f.name) {
                return Err(ChannelError::Unknown { source_id: sid.clone(), index, send: rule.send.clone(), name: f.name.clone() });
            }
            let rendering = match (&f.from, &f.value) {
                (Some(from), None) => {
                    if !on.reads(from) {
                        return Err(ChannelError::NotRead {
                            source_id: sid.clone(),
                            index,
                            on: rule.on.clone(),
                            from: from.clone(),
                            name: f.name.clone(),
                        });
                    }
                    Rendering::From(from.clone())
                }
                (None, Some(value)) => Rendering::Value(value.text()),
                _ => return Err(ChannelError::OneSource { source_id: sid.clone(), index, name: f.name.clone() }),
            };
            fields.push((f.name.clone(), rendering));
        }
        if let Some(name) = needs.iter().find(|n| !fields.iter().any(|(given, _)| given == *n)) {
            return Err(ChannelError::Unwritten { source_id: sid.clone(), index, send: rule.send.clone(), name: name.clone() });
        }
        out.push(Reply { on: rule.on.clone(), send: rule.send.clone(), fields });
    }
    Ok(out)
}

/// The plant-local wall clock as naive milliseconds: the axis a stamp parsed from a plant's own
/// text lands on, so a telegram and a logged line from the same second line up.
fn index_now_ms() -> i64 {
    chrono::Local::now().naive_local().and_utc().timestamp_millis()
}

/// A request answered by the worker once its status mirrors the outcome.
type Answer = (Sender<Result<(), SendError>>, Result<(), SendError>);

enum Request {
    Send { to: Option<PeerId>, message: String, fields: HashMap<String, String>, reply: Sender<Result<(), SendError>> },
}

/// The handle to a running channel: its status, and the gated send.
#[derive(Clone)]
pub struct ChannelHandle {
    pub source_id: String,
    requests: Sender<Request>,
    status: Arc<Mutex<ChannelStatus>>,
    gate: WriteGate,
    tables: Arc<Vec<MessageTable>>,
}

impl ChannelHandle {
    pub fn status(&self) -> ChannelStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Whether writes are armed right now.
    pub fn is_armed(&self) -> bool {
        self.gate.is_armed()
    }

    /// The tables this channel's messages land in, one per message of the catalog.
    pub fn tables(&self) -> &[MessageTable] {
        &self.tables
    }

    /// Send a message of the catalog, by name and fields, to one peer or to every connected
    /// peer. The master switch is checked first; nothing is composed while it is off.
    pub fn send(&self, to: Option<PeerId>, message: &str, fields: HashMap<String, String>) -> Result<(), SendError> {
        if !self.gate.is_armed() {
            return Err(SendError::Disarmed);
        }
        let (reply, rx) = std::sync::mpsc::channel();
        self.requests
            .send(Request::Send { to, message: message.to_string(), fields, reply })
            .map_err(|_| SendError::WorkerGone)?;
        rx.recv_timeout(REQUEST_TIMEOUT).map_err(|_| SendError::WorkerGone)?
    }
}

/// A spawned channel: the stop/join handle, and the handle for status and sends.
pub struct ChannelWorker {
    pub handle: SourceHandle,
    pub writer: ChannelHandle,
}

/// Start one channel's worker on its own thread. Everything is checked first, and nothing binds
/// or dials until it all holds: the catalog's ids ride the framing, every reply rule fits the
/// catalog, every message's table fits. `gate` is the master write switch, shared with every
/// other write path of the session.
pub fn spawn(cfg: ChannelConfig, tx: Sender<ChannelEvent>, gate: WriteGate) -> Result<ChannelWorker, ChannelError> {
    cfg.catalog
        .check_framing(&cfg.spec.framing)
        .map_err(|e| ChannelError::Catalog { source_id: cfg.source_id.clone(), reason: e.to_string() })?;
    let replies = compile_replies(&cfg)?;
    let tables: Vec<MessageTable> =
        cfg.catalog.messages.iter().map(|m| MessageTable::of(&cfg.source_id, m)).collect::<Result<_, _>>()?;
    let status = Arc::new(Mutex::new(ChannelStatus {
        wire: WireStatus { state: "starting".into(), ..Default::default() },
        ..Default::default()
    }));
    let (req_tx, req_rx) = std::sync::mpsc::channel::<Request>();
    let stop = stop_flag();
    let writer = ChannelHandle {
        source_id: cfg.source_id.clone(),
        requests: req_tx,
        status: status.clone(),
        gate: gate.clone(),
        tables: Arc::new(tables),
    };
    let source_id = SourceId::new(&cfg.source_id);
    let thread_stop = stop.clone();
    let worker_status = status;
    let join = std::thread::Builder::new()
        .name(format!("channel-{}", cfg.source_id))
        .spawn(move || run(cfg, replies, tx, req_rx, worker_status, gate, thread_stop))
        .map_err(|e| ChannelError::Thread(writer.source_id.clone(), e.to_string()))?;
    Ok(ChannelWorker { handle: SourceHandle::new(source_id, stop, join), writer })
}

struct Link {
    channel: Channel,
    cfg: ChannelConfig,
    status: Arc<Mutex<ChannelStatus>>,
}

impl Link {
    fn note_error(&self, text: String) {
        if let Ok(mut s) = self.status.lock() {
            s.last_error = Some(text);
        }
    }

    /// Compose and send a message of the catalog, or say why not. Nothing reaches the wire on an
    /// error.
    fn send(&self, to: Option<PeerId>, peers: &[PeerId], message: &str, fields: &HashMap<String, String>) -> Result<(), SendError> {
        match to {
            Some(p) if !peers.contains(&p) => return Err(SendError::NoPeer(p)),
            None if peers.is_empty() => return Err(SendError::NoPeers),
            _ => {}
        }
        let (id, body) = match self.cfg.catalog.encode(message, fields) {
            None => return Err(SendError::UnknownMessage(message.to_string())),
            Some(Err(e)) => return Err(SendError::Compose { message: message.to_string(), reason: e.to_string() }),
            Some(Ok(v)) => v,
        };
        if body.is_empty() && self.cfg.spec.framing.segment.is_none() {
            return Err(SendError::EmptyBody(message.to_string()));
        }
        let frame = self.cfg.catalog.frame(&self.cfg.spec.framing, id, body);
        // What the framing would refuse (a body over the frame limit) is refused here, before
        // the message is counted as sent; the sequence is the channel's.
        if let Err(e) = self.cfg.spec.framing.encode(&frame, None) {
            return Err(SendError::Framing { message: message.to_string(), reason: e.to_string() });
        }
        match to {
            Some(peer) => self.channel.send(peer, frame),
            None => self.channel.send_all(frame),
        }
        if let Ok(mut s) = self.status.lock() {
            *s.sent.entry(message.to_string()).or_default() += 1;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    cfg: ChannelConfig,
    replies: Vec<Reply>,
    tx: Sender<ChannelEvent>,
    requests: Receiver<Request>,
    status: Arc<Mutex<ChannelStatus>>,
    gate: WriteGate,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), SourceError> {
    let source_id = cfg.source_id.clone();
    let channel = Channel::start(WireConfig::new(cfg.display_name.clone(), cfg.role.clone(), cfg.spec.clone()));
    let link = Link { channel, cfg, status: status.clone() };
    let mut peers: Vec<PeerId> = Vec::new();
    let mut state = LinkState::Waiting;
    let mut last_wire_error: Option<String> = None;
    let emit = |event: ChannelEvent| tx.send(event).is_ok();
    if !emit(ChannelEvent::Status { source_id: source_id.clone(), state: state.clone() }) {
        return Ok(());
    }
    let mut consumer_gone = false;

    while !stop.load(std::sync::atomic::Ordering::Relaxed) && !consumer_gone {
        let mut events = Vec::new();
        if let Some(e) = link.channel.recv_timeout(TICK) {
            events.push(e);
        }
        events.extend(link.channel.poll());
        for event in events {
            let arrival_ms = now_unix_ms();
            match event {
                WireEvent::Connected { peer, .. } => {
                    peers.push(peer);
                }
                WireEvent::Disconnected { peer, .. } => {
                    peers.retain(|p| *p != peer);
                }
                WireEvent::KeepAlive { peer, .. } => {
                    if !emit(ChannelEvent::KeepAlive { source_id: source_id.clone(), peer, arrival_ms }) {
                        consumer_gone = true;
                    }
                }
                WireEvent::Received { peer, frame, raw } => {
                    let index_ms = index_now_ms();
                    for (id, body) in link.cfg.catalog.messages_of(&link.cfg.spec.framing, &frame) {
                        let decoded = id.and_then(|id| link.cfg.catalog.decode(id, &body).map(|(m, f)| (id, m.name.clone(), f)));
                        match decoded {
                            Some((id, name, fields)) => {
                                if let Ok(mut s) = status.lock() {
                                    *s.received.entry(name.clone()).or_default() += 1;
                                }
                                for rule in replies.iter().filter(|r| r.on == name) {
                                    let outcome = if !gate.is_armed() {
                                        if let Ok(mut s) = status.lock() {
                                            s.replies_withheld += 1;
                                        }
                                        ReplyOutcome::Withheld
                                    } else {
                                        let mut values: HashMap<String, String> = HashMap::new();
                                        let mut missing: Option<String> = None;
                                        for (name, rendering) in &rule.fields {
                                            match rendering {
                                                Rendering::Value(v) => {
                                                    values.insert(name.clone(), v.clone());
                                                }
                                                Rendering::From(from) => match fields.get(from) {
                                                    Some(v) => {
                                                        values.insert(name.clone(), v.trim().to_string());
                                                    }
                                                    None => missing = Some(from.clone()),
                                                },
                                            }
                                        }
                                        let result = match missing {
                                            Some(from) => Err(format!("the message carried no `{from}`")),
                                            None => link.send(Some(peer), &peers, &rule.send, &values).map_err(|e| e.to_string()),
                                        };
                                        match result {
                                            Ok(()) => {
                                                if let Ok(mut s) = status.lock() {
                                                    s.replies_sent += 1;
                                                }
                                                ReplyOutcome::Sent
                                            }
                                            Err(why) => {
                                                if let Ok(mut s) = status.lock() {
                                                    s.replies_refused += 1;
                                                    s.last_error = Some(format!("reply `{}` to `{}`: {why}", rule.send, rule.on));
                                                }
                                                ReplyOutcome::Refused(why)
                                            }
                                        }
                                    };
                                    if !emit(ChannelEvent::Reply {
                                        source_id: source_id.clone(),
                                        peer,
                                        on: rule.on.clone(),
                                        send: rule.send.clone(),
                                        outcome,
                                    }) {
                                        consumer_gone = true;
                                    }
                                }
                                if !emit(ChannelEvent::Message {
                                    source_id: source_id.clone(),
                                    peer,
                                    index_ms,
                                    arrival_ms,
                                    name,
                                    id,
                                    fields,
                                    raw: raw.clone(),
                                }) {
                                    consumer_gone = true;
                                }
                            }
                            None => {
                                let known = id.is_some_and(|id| link.cfg.catalog.by_id(id).is_some());
                                if let Ok(mut s) = status.lock() {
                                    if known {
                                        s.undecodable += 1;
                                    } else {
                                        s.unknown_ids += 1;
                                    }
                                }
                                if !emit(ChannelEvent::Unreadable {
                                    source_id: source_id.clone(),
                                    peer,
                                    arrival_ms,
                                    id,
                                    body,
                                    raw: raw.clone(),
                                }) {
                                    consumer_gone = true;
                                }
                            }
                        }
                    }
                }
                WireEvent::Trouble(e) => {
                    link.note_error(e.clone());
                    last_wire_error = Some(e);
                }
            }
        }

        // Requests from outside, answered once the status mirrors them.
        let mut answers: Vec<Answer> = Vec::new();
        while let Ok(request) = requests.try_recv() {
            match request {
                Request::Send { to, message, fields, reply } => {
                    let result = if gate.is_armed() { link.send(to, &peers, &message, &fields) } else { Err(SendError::Disarmed) };
                    if let Err(e) = &result {
                        link.note_error(e.to_string());
                    }
                    answers.push((reply, result));
                }
            }
        }

        // The link's state, from the peers and the wire's own word.
        let wire = link.channel.status();
        let next_state = if !peers.is_empty() {
            LinkState::Linked { peers: peers.len() }
        } else if wire.state == "failed" || wire.state == "retrying" {
            // The wire could not bind, or a dial failed and the next is due later: the channel
            // stays and keeps trying, and says why.
            LinkState::Trouble(wire.last_error.clone().or_else(|| last_wire_error.clone()).unwrap_or_else(|| wire.state.clone()))
        } else {
            LinkState::Waiting
        };
        if next_state != state {
            state = next_state;
            if !emit(ChannelEvent::Status { source_id: source_id.clone(), state: state.clone() }) {
                consumer_gone = true;
            }
        }
        if let Ok(mut s) = status.lock() {
            s.wire = wire;
            s.peers = peers.clone();
        }
        for (reply, result) in answers {
            let _ = reply.send(result);
        }
    }
    link.channel.stop();
    if let Ok(mut s) = status.lock() {
        s.wire.state = "stopped".into();
        s.peers.clear();
    }
    let _ = emit(ChannelEvent::Status { source_id, state: LinkState::Stopped });
    Ok(())
}
