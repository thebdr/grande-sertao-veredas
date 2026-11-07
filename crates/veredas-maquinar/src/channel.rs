//! A bench channel: one TCP telegram link in either role, framed and held by core's channel
//! file, speaking the messages of a catalog, with cyclic and on-demand messages built from
//! fields, a monitor of what came and went, and, when a rules file rides it, the simulated
//! controller ([`crate::controller`]).
//!
//! The socket is core's [`Channel`]: the framing, the sequence, the keep-alive and the link
//! rules are the channel file's, and the bench adds nothing to the wire a consumer could tell
//! apart. Above it, a message is a name: the catalog says what id the frame carries and how its
//! body is written and read; the channel puts the id where the catalog's `id_field` names (a
//! segment header field, or the envelope's in a layout without segments).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use serde::Deserialize;
use veredas_core::rng::Pcg32;
use veredas_core::telegram::catalog::Catalog;
use veredas_core::telegram::channel::{Channel, ChannelConfig as WireConfig, ChannelEvent, ChannelStatus as WireStatus, PeerId, Role};
use veredas_core::telegram::framing::{Frame, preview};
use veredas_core::telegram::schema::ChannelSpec;

use crate::controller::{Context, Controller, ControllerStatus, FieldSpec, Fields, Outgoing, Place, Rules, RulesError};

/// The service loop's period.
pub const TICK: Duration = Duration::from_millis(20);
/// How many monitor lines are kept.
pub const RECENT: usize = 100;

/// A message the channel sends on an interval, or on demand with `every_ms` 0.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyclicMessage {
    pub message: String,
    #[serde(default)]
    pub every_ms: u64,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

/// How a bench channel is started: its name, which side of the link it is, the channel file
/// and the catalog it speaks, its own cyclic messages, and the controller riding it.
pub struct ChannelConfig {
    pub name: String,
    pub role: Role,
    pub spec: ChannelSpec,
    pub catalog: Catalog,
    pub messages: Vec<CyclicMessage>,
    pub controller: Option<Rules>,
    /// A fixed seed for the channel's own draws; the clock when absent.
    pub seed: Option<u64>,
}

/// Why a channel did not start.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The catalog's `id_field` is not a field of the framing where an id can ride.
    #[error("{0}")]
    IdField(String),
    #[error("messages[{index}] (`{message}`): {source}")]
    Message { index: usize, message: String, source: RulesError },
    #[error("messages[{index}]: the catalog has no message `{message}`")]
    UnknownMessage { index: usize, message: String },
    #[error("spawn the channel thread: {0}")]
    Thread(std::io::Error),
}

/// Which way a monitor line went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    /// Not a message: a connection, a refusal, the controller's own line.
    Note,
}

/// One line of the monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct RxLine {
    pub at: String,
    pub direction: Direction,
    /// The message's name, or what the line is about.
    pub message: String,
    pub fields: BTreeMap<String, String>,
    /// The body as an operator reads it, or the note.
    pub preview: String,
    pub peer: Option<PeerId>,
}

/// What the channel reports.
#[derive(Clone, Debug, Default)]
pub struct ChannelStatus {
    /// The link, as core's channel reports it.
    pub wire: WireStatus,
    /// Connected peers, by id.
    pub peers: Vec<PeerId>,
    /// Newest first, at most [`RECENT`].
    pub recent: VecDeque<RxLine>,
    /// Messages sent, by name.
    pub sent: BTreeMap<String, u64>,
    /// Messages received and decoded, by name.
    pub received: BTreeMap<String, u64>,
    /// Frames carrying an id the catalog does not hold.
    pub unknown_ids: u64,
    /// Bodies the catalog's rules refused.
    pub undecodable: u64,
    pub controller: Option<ControllerStatus>,
    pub last_error: Option<String>,
}

/// A knob turned on the controller from outside.
enum Knob {
    Interval(u64),
    Fault(String, u8),
    Destination(u64, u32),
    Bit(u64, String, bool),
    BitEverywhere(String, bool),
    Register(String, i64),
    AnnounceNow,
}

enum Request {
    SendNow { message: String, fields: HashMap<String, String>, to: Option<PeerId> },
    Knob(Knob, Sender<Result<(), String>>),
}

/// A running bench channel. Dropping it stops the link.
pub struct BenchChannel {
    pub name: String,
    status: Arc<Mutex<ChannelStatus>>,
    requests: Sender<Request>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BenchChannel {
    pub fn status(&self) -> ChannelStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Send a message now, built from these values, to one peer or to everyone.
    pub fn send_now(&self, message: &str, fields: HashMap<String, String>, to: Option<PeerId>) {
        let _ = self.requests.send(Request::SendNow { message: message.to_string(), fields, to });
    }

    fn knob(&self, knob: Knob) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.requests.send(Request::Knob(knob, tx)).map_err(|_| "the channel has stopped".to_string())?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "the channel did not answer".to_string())?
    }

    /// The controller's announcement interval, live.
    pub fn set_interval(&self, ms: u64) -> Result<(), String> {
        self.knob(Knob::Interval(ms))
    }

    /// A named fault's share of items, live.
    pub fn set_fault(&self, name: &str, percent: u8) -> Result<(), String> {
        self.knob(Knob::Fault(name.to_string(), percent))
    }

    /// A destination's whole state word.
    pub fn set_destination(&self, destination: u64, word: u32) -> Result<(), String> {
        self.knob(Knob::Destination(destination, word))
    }

    /// One named bit of a destination.
    pub fn set_bit(&self, destination: u64, bit: &str, on: bool) -> Result<(), String> {
        self.knob(Knob::Bit(destination, bit.to_string(), on))
    }

    /// One named bit on every destination: every chute full, every chute back.
    pub fn set_bit_everywhere(&self, bit: &str, on: bool) -> Result<(), String> {
        self.knob(Knob::BitEverywhere(bit.to_string(), on))
    }

    /// A register's value, as a command would set it.
    pub fn set_register(&self, name: &str, value: i64) -> Result<(), String> {
        self.knob(Knob::Register(name.to_string(), value))
    }

    /// Announce an item now, whatever the interval.
    pub fn announce_now(&self) -> Result<(), String> {
        self.knob(Knob::AnnounceNow)
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().map(|t| t.is_finished()).unwrap_or(true)
    }

    fn shutdown_and_join(&mut self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for BenchChannel {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

/// Where the catalog's id rides in this framing: a segment header field, or an envelope field
/// in a layout without segments.
pub fn check_id_field(spec: &ChannelSpec, catalog: &Catalog) -> Result<(), ChannelError> {
    // The catalog's own check: it knows what portal's channel will need to know too.
    catalog.check_framing(&spec.framing).map_err(|e| ChannelError::IdField(e.to_string()))
}

/// Start a bench channel. Returns once its thread runs; a bind failure shows in the status.
pub fn start(cfg: ChannelConfig) -> Result<BenchChannel, ChannelError> {
    check_id_field(&cfg.spec, &cfg.catalog)?;
    let empty = BTreeMap::new();
    let mut cyclic = Vec::new();
    for (i, m) in cfg.messages.iter().enumerate() {
        let message = cfg.catalog.by_name(&m.message).ok_or_else(|| ChannelError::UnknownMessage { index: i, message: m.message.clone() })?;
        let fields = Fields::compile(&format!("messages[{i}]"), &m.fields, Place::Plain, &empty, Some(&message.names()))
            .map_err(|source| ChannelError::Message { index: i, message: m.message.clone(), source })?;
        cyclic.push((m.message.clone(), m.every_ms, fields));
    }
    let status = Arc::new(Mutex::new(ChannelStatus { wire: WireStatus { state: "starting".into(), ..Default::default() }, ..Default::default() }));
    let (req_tx, req_rx) = std::sync::mpsc::channel::<Request>();
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_status = status.clone();
    let worker_shutdown = shutdown.clone();
    let name = cfg.name.clone();
    let thread = std::thread::Builder::new()
        .name(format!("maquinar-channel-{name}"))
        .spawn(move || run(cfg, cyclic, worker_status, req_rx, worker_shutdown))
        .map_err(ChannelError::Thread)?;
    Ok(BenchChannel { name, status, requests: req_tx, shutdown, thread: Some(thread) })
}

fn stamp() -> String {
    Local::now().format("%H:%M:%S%.3f").to_string()
}

fn note(status: &Arc<Mutex<ChannelStatus>>, line: RxLine) {
    if let Ok(mut s) = status.lock() {
        s.recent.push_front(line);
        while s.recent.len() > RECENT {
            s.recent.pop_back();
        }
    }
}

fn note_text(status: &Arc<Mutex<ChannelStatus>>, text: String) {
    note(status, RxLine { at: stamp(), direction: Direction::Note, message: String::new(), fields: BTreeMap::new(), preview: text, peer: None });
}

struct Link {
    channel: Channel,
    catalog: Catalog,
    spec: ChannelSpec,
    status: Arc<Mutex<ChannelStatus>>,
}

impl Link {
    /// A frame carrying `id` and `body` where this framing takes them (the catalog's placing).
    fn frame(&self, id: u64, body: Vec<u8>) -> Frame {
        self.catalog.frame(&self.spec.framing, id, body)
    }

    /// Build and send a message, or say why not.
    fn send(&self, message: &str, fields: &HashMap<String, String>, to: Option<PeerId>) {
        match self.catalog.encode(message, fields) {
            None => {
                if let Ok(mut s) = self.status.lock() {
                    s.last_error = Some(format!("{message}: the catalog has no such message"));
                }
                note_text(&self.status, format!("{message} not sent: the catalog has no such message"));
            }
            Some(Err(e)) => {
                if let Ok(mut s) = self.status.lock() {
                    s.last_error = Some(format!("{message}: {e}"));
                }
                note_text(&self.status, format!("{message} not sent: {e}"));
            }
            Some(Ok((id, body))) => {
                if body.is_empty() && self.spec.framing.segment.is_none() {
                    // The wire carries an empty body as a keep-alive in a layout without
                    // segments: the peer would never see a message.
                    if let Ok(mut s) = self.status.lock() {
                        s.last_error = Some(format!("{message}: an empty body is a keep-alive in a layout without segments"));
                    }
                    note_text(&self.status, format!("{message} not sent: an empty body is a keep-alive in a layout without segments"));
                    return;
                }
                let line = RxLine {
                    at: stamp(),
                    direction: Direction::Out,
                    message: message.to_string(),
                    fields: fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    preview: preview(&body),
                    peer: to,
                };
                let frame = self.frame(id, body);
                // What the framing would refuse (a body over the frame limit) is refused here,
                // before the message is counted as sent; the sequence is the channel's.
                if let Err(e) = self.spec.framing.encode(&frame, None) {
                    if let Ok(mut s) = self.status.lock() {
                        s.last_error = Some(format!("{message}: {e}"));
                    }
                    note_text(&self.status, format!("{message} not sent: {e}"));
                    return;
                }
                match to {
                    Some(peer) => self.channel.send(peer, frame),
                    None => self.channel.send_all(frame),
                }
                if let Ok(mut s) = self.status.lock() {
                    *s.sent.entry(message.to_string()).or_default() += 1;
                }
                note(&self.status, line);
            }
        }
    }

    /// The messages a received frame carries: `(id, body)` per segment (or the one body of a
    /// segmentless layout), as the catalog reads them.
    fn messages_of(&self, frame: &Frame) -> Vec<(Option<u64>, Vec<u8>)> {
        self.catalog.messages_of(&self.spec.framing, frame)
    }
}

fn run(cfg: ChannelConfig, mut cyclic: Vec<(String, u64, Fields)>, status: Arc<Mutex<ChannelStatus>>, requests: Receiver<Request>, shutdown: Arc<std::sync::atomic::AtomicBool>) {
    let channel = Channel::start(WireConfig::new(cfg.name.clone(), cfg.role.clone(), cfg.spec.clone()));
    let link = Link { channel, catalog: cfg.catalog, spec: cfg.spec, status: status.clone() };
    let mut controller = cfg.controller.map(|rules| Controller::new(rules, cfg.seed));
    let mut rng = match cfg.seed {
        Some(s) => Pcg32::new(s, 3),
        None => Pcg32::from_entropy(3),
    };
    let mut peers: Vec<PeerId> = Vec::new();
    let mut last = Instant::now();
    let mut due: Vec<Instant> = cyclic.iter().map(|(_, every, _)| last + Duration::from_millis((*every).max(1))).collect();
    let empty_fields = HashMap::new();
    let empty_registers = BTreeMap::new();

    while !shutdown.load(std::sync::atomic::Ordering::SeqCst) {
        // What the wire said (waiting at most a tick for it).
        // DECISION PENDING (docs/questions.md Q13): the wait lands at the host's timer quantum,
        // about 31 ms on Windows unless the process raises the resolution; every due time below
        // is scheduled from the last due time, never from the tick, so the rates hold.
        let mut events = Vec::new();
        if let Some(e) = link.channel.recv_timeout(TICK) {
            events.push(e);
        }
        events.extend(link.channel.poll());
        let mut outgoing: Vec<Outgoing> = Vec::new();
        for event in events {
            match event {
                ChannelEvent::Connected { peer, addr } => {
                    peers.push(peer);
                    note_text(&status, format!("peer {peer} connected from {addr}"));
                }
                ChannelEvent::Disconnected { peer, why } => {
                    peers.retain(|p| *p != peer);
                    note_text(&status, format!("peer {peer} disconnected: {why}"));
                }
                ChannelEvent::KeepAlive { peer, .. } => {
                    note(&status, RxLine { at: stamp(), direction: Direction::In, message: String::new(), fields: BTreeMap::new(), preview: "(keep-alive)".into(), peer: Some(peer) });
                }
                ChannelEvent::Received { peer, frame, .. } => {
                    for (id, body) in link.messages_of(&frame) {
                        let Some(id) = id else {
                            if let Ok(mut s) = status.lock() {
                                s.unknown_ids += 1;
                            }
                            note(&status, RxLine { at: stamp(), direction: Direction::In, message: "(no id)".into(), fields: BTreeMap::new(), preview: preview(&body), peer: Some(peer) });
                            continue;
                        };
                        match link.catalog.decode(id, &body) {
                            Some((message, fields)) => {
                                if let Ok(mut s) = status.lock() {
                                    *s.received.entry(message.name.clone()).or_default() += 1;
                                }
                                note(&status, RxLine {
                                    at: stamp(),
                                    direction: Direction::In,
                                    message: message.name.clone(),
                                    fields: fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                                    preview: preview(&body),
                                    peer: Some(peer),
                                });
                                if let Some(c) = controller.as_mut() {
                                    outgoing.extend(c.on_message(&message.name, &fields, Some(peer)));
                                }
                            }
                            None => {
                                let known = link.catalog.by_id(id).is_some();
                                if let Ok(mut s) = status.lock() {
                                    if known {
                                        s.undecodable += 1;
                                    } else {
                                        s.unknown_ids += 1;
                                    }
                                }
                                let what = match link.catalog.by_id(id) {
                                    Some(m) => format!("{} (body refused)", m.name),
                                    None => format!("unknown id {id}"),
                                };
                                note(&status, RxLine { at: stamp(), direction: Direction::In, message: what, fields: BTreeMap::new(), preview: preview(&body), peer: Some(peer) });
                            }
                        }
                    }
                }
                ChannelEvent::Trouble(e) => {
                    if let Ok(mut s) = status.lock() {
                        s.last_error = Some(e.clone());
                    }
                    note_text(&status, e);
                }
            }
        }

        // Requests from outside. A knob is answered only once the status pane shows its
        // effect, at the end of this pass, so a caller that reads the status right after the
        // call sees what it set.
        type Reply = (Sender<Result<(), String>>, Result<(), String>);
        let mut replies: Vec<Reply> = Vec::new();
        while let Ok(request) = requests.try_recv() {
            match request {
                Request::SendNow { message, fields, to } => link.send(&message, &fields, to),
                Request::Knob(knob, reply) => {
                    let result = match controller.as_mut() {
                        None => Err("this channel has no controller".to_string()),
                        Some(c) => match knob {
                            Knob::Interval(ms) => c.set_interval(ms).map_err(|e| e.to_string()),
                            Knob::Fault(name, pct) => c.set_fault(&name, pct).map_err(|e| e.to_string()),
                            Knob::Destination(d, w) => c.set_destination(d, w).map_err(|e| e.to_string()),
                            Knob::Bit(d, bit, on) => c.set_bit(d, &bit, on).map_err(|e| e.to_string()),
                            Knob::BitEverywhere(bit, on) => c.set_bit_everywhere(&bit, on).map_err(|e| e.to_string()),
                            Knob::Register(name, v) => c.set_register(&name, v).map_err(|e| e.to_string()),
                            Knob::AnnounceNow => {
                                if let Some(o) = c.announce_now() {
                                    outgoing.push(o);
                                }
                                Ok(())
                            }
                        },
                    };
                    replies.push((reply, result));
                }
            }
        }

        // Time passes: the controller's own traffic, and the channel's cyclic messages.
        // Whole milliseconds, the remainder carried to the next pass, so the controller's
        // clock does not fall behind the wall clock by a fraction every pass.
        let now = Instant::now();
        let elapsed = now.duration_since(last).as_millis() as u64;
        last += Duration::from_millis(elapsed);
        if let Some(c) = controller.as_mut() {
            outgoing.extend(c.tick(elapsed, peers.len()));
        }
        for (i, (message, every, fields)) in cyclic.iter_mut().enumerate() {
            if *every == 0 || now < due[i] {
                continue;
            }
            // Due an interval after the last was due; a stall skips what it missed.
            let step = Duration::from_millis(*every);
            let mut next = due[i] + step;
            while next <= now {
                next += step;
            }
            due[i] = next;
            let mut ctx = Context { fields: &empty_fields, registers: &empty_registers, blocked: &[], rng: &mut rng };
            match fields.render(&mut ctx) {
                Ok(values) => link.send(message, &values, None),
                Err(e) => note_text(&status, format!("{message} not sent: {e}")),
            }
        }
        for o in outgoing {
            if o.message.is_empty() {
                note_text(&status, o.note);
            } else {
                note_text(&status, o.note.clone());
                link.send(&o.message, &o.fields, o.to);
            }
        }

        // The status pane, then the knobs' answers.
        if let Ok(mut s) = status.lock() {
            s.wire = link.channel.status();
            s.peers = peers.clone();
            s.controller = controller.as_ref().map(Controller::status);
        }
        for (reply, result) in replies {
            let _ = reply.send(result);
        }
    }
    link.channel.stop();
    if let Ok(mut s) = status.lock() {
        s.wire.state = "stopped".into();
    }
}
