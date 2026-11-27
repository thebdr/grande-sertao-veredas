//! The listener (`docs/aranha-plan.md`, "Scope", listening; "Rules": it listens before it asks,
//! every active step bounded and paced, frames kept verbatim): one worker per wire, hearing
//! everything it carries; every frame kept verbatim with its interface and time, up to a bound (the
//! oldest let go past it, counted); each decoded (C-140), its observations fused (C-141) unless
//! they are this machine's own (sent from one of its interfaces or about one: counted apart), its
//! class counted and its damage kept. Each worker is the only sender on its wire: a frame handed to
//! the listener is queued for that interface and sent paced (a rate per interface), the queue
//! bounded (a frame past it refused, never a flood); what it sends is kept too, marked sent.
//! Nothing is sent on an interface no worker hears, nor once its worker has ended: what was queued
//! then is said unsent.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::fusion::{Conflict, Fusion};
use crate::observe::{self, Class};
use crate::profinet::ether;
use crate::profinet::mac::Mac;
use crate::wire::{Frame, MAX_FRAME, Wire, WireError, stamp};

/// How a listener runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Frames kept, heard and sent; past it the oldest are let go.
    pub max_frames: usize,
    /// Frames sent a second on one interface, at most.
    pub rate: u32,
    /// Frames queued for one interface, at most; one past it is refused.
    pub queue: usize,
    /// The longest a worker waits for a frame before looking at its queue and at the stop.
    pub poll: Duration,
}

impl Default for Options {
    /// 200 000 frames kept, 20 frames a second (Veredas 1's pace), 100 queued, a 20 ms poll.
    fn default() -> Self {
        Options { max_frames: 200_000, rate: 20, queue: 100, poll: Duration::from_millis(20) }
    }
}

/// What became of a frame handed to the listener to send: kept as sent, with its index and stamp,
/// or refused by its wire, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent { index: u64, at_ns: u64 },
    Refused(String),
    /// Never sent: its worker ended (or the listener stopped) first, for this reason.
    Unsent(String),
}

/// A frame handed over to be sent and told of: its number (to withdraw it), the frames queued
/// ahead of it when it was queued, and where its outcome comes.
#[derive(Debug)]
pub struct Tracked {
    pub id: u64,
    pub ahead: usize,
    pub told: Receiver<Outcome>,
}

/// A kept frame: heard, or sent by the listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub frame: Frame,
    pub sent: bool,
}

/// One interface's account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub mac: Option<Mac>,
    pub heard: u64,
    pub sent: u64,
    /// Frames its wire refused to send (each reason among the damage).
    pub refused: u64,
    pub queued: usize,
    /// Frames still queued when its worker ended or the listener stopped, or being handed to its
    /// wire when its worker ended: never sent.
    pub unsent: u64,
    /// Frames taken back while they waited: never sent.
    pub withdrawn: u64,
    /// Why its worker stopped hearing, when it did: a capture read to its end, or the wire's error.
    pub ended: Option<String>,
}

/// What the listening has given so far.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Frames heard (sent ones apart).
    pub heard: u64,
    /// Frames let go past the bound, oldest first.
    pub let_go: u64,
    pub by_class: BTreeMap<Class, u64>,
    /// Observations fused, and observations about this machine's own interfaces left out.
    pub observations: u64,
    pub own: u64,
    /// The last 50 frames' damage, each `#<index> <interface>: <damage>`.
    pub damage: VecDeque<String>,
    pub damaged_frames: u64,
    pub interfaces: BTreeMap<String, Interface>,
    pub first_ns: Option<u64>,
    pub last_ns: Option<u64>,
}

/// The listening as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub fusion: Fusion,
    pub conflicts: Vec<Conflict>,
    pub stats: Stats,
    /// The kept frames' indices: the first still kept and the next to come.
    pub kept: (u64, u64),
}

#[derive(Debug, thiserror::Error)]
pub enum ListenError {
    #[error("two wires are named {0:?}: one worker per interface")]
    TwoWires(String),
    #[error("no wire is heard on {0:?}: nothing is sent where the listener does not listen")]
    NotListening(String),
    #[error("{iface:?} is heard no more ({reason}): nothing is sent on it")]
    Ended { iface: String, reason: String },
    #[error("{iface:?} has {queued} frames queued, its bound: the frame is refused")]
    QueueFull { iface: String, queued: usize },
    #[error("{0}")]
    Frame(WireError),
    #[error("a listener sends one to a thousand frames a second and keeps at least one frame")]
    Options,
    #[error("start a worker: {0}")]
    Thread(std::io::Error),
}

const DAMAGE_KEPT: usize = 50;

/// The fastest a listener sends on an interface: a thousand frames a second.
pub const MAX_RATE: u32 = 1000;

/// A frame waiting its turn, and who is to be told what became of it.
type Queued = (u64, Vec<u8>, Option<Sender<Outcome>>);

struct State {
    frames: VecDeque<Kept>,
    /// The index of `frames[0]`.
    base: u64,
    fusion: Fusion,
    stats: Stats,
    queues: BTreeMap<String, VecDeque<Queued>>,
    /// Every frame handed over is numbered, so one still waiting may be withdrawn.
    next_id: u64,
    /// The frame each worker is handing to its wire (and whom to tell), so that the worker's end
    /// counts it, and tells it, too.
    in_flight: BTreeMap<String, Option<Sender<Outcome>>>,
}

struct Shared {
    state: Mutex<State>,
    stop: AtomicBool,
    own: BTreeSet<Mac>,
    /// Each wire's interface index, where it has one.
    indexes: BTreeMap<String, u32>,
    options: Options,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    /// Keep a frame, letting the oldest go past the bound; its index.
    fn keep(&mut self, kept: Kept, max: usize) -> u64 {
        let index = self.base + self.frames.len() as u64;
        self.frames.push_back(kept);
        while self.frames.len() > max {
            self.frames.pop_front();
            self.base += 1;
            self.stats.let_go += 1;
        }
        index
    }
}

/// A listener on its wires, its workers running until it is stopped.
pub struct Listener {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Listener {
    /// Start one worker per wire. Two wires of one name are refused.
    pub fn start(wires: Vec<Box<dyn Wire>>, options: Options) -> Result<Listener, ListenError> {
        if !(1..=MAX_RATE).contains(&options.rate) || options.max_frames == 0 {
            return Err(ListenError::Options);
        }
        let mut names = BTreeSet::new();
        for w in &wires {
            if !names.insert(w.name().to_string()) {
                return Err(ListenError::TwoWires(w.name().to_string()));
            }
        }
        let own: BTreeSet<Mac> = wires.iter().filter_map(|w| w.mac()).collect();
        let indexes: BTreeMap<String, u32> = wires.iter().filter_map(|w| Some((w.name().to_string(), w.index()?))).collect();
        let mut stats = Stats::default();
        let mut queues = BTreeMap::new();
        for w in &wires {
            stats.interfaces.insert(w.name().to_string(), Interface { mac: w.mac(), ..Interface::default() });
            queues.insert(w.name().to_string(), VecDeque::new());
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State { frames: VecDeque::new(), base: 0, fusion: Fusion::new(), stats, queues, next_id: 0, in_flight: BTreeMap::new() }),
            stop: AtomicBool::new(false),
            own,
            indexes,
            options,
        });
        let mut workers = Vec::new();
        for wire in wires {
            let s = Arc::clone(&shared);
            // (A thread's name holds no NUL.)
            let name = format!("aranha-listen-{}", wire.name().replace('\0', ""));
            match std::thread::Builder::new().name(name).spawn(move || work(wire, s)) {
                Ok(h) => workers.push(h),
                Err(e) => {
                    shared.stop.store(true, Ordering::SeqCst);
                    for h in workers {
                        let _ = h.join();
                    }
                    return Err(ListenError::Thread(e));
                }
            }
        }
        Ok(Listener { shared, workers })
    }

    /// Queue a whole frame to send on `iface`, paced by its worker. Refused on an interface no
    /// worker hears, past the queue's bound, or of a length no wire carries.
    pub fn send(&self, iface: &str, frame: Vec<u8>) -> Result<(), ListenError> {
        self.queue(iface, frame, None).map(|_| ())
    }

    /// Queue a frame as [`Listener::send`] does, and be told what became of it once its worker
    /// has sent it or its wire refused it, or that it never will (its worker ended first, even
    /// while handing it to the wire, or the listener stopped): told once, then the channel closes.
    /// The ticket says how many frames were queued ahead of it, counted as it was queued.
    pub fn send_tracked(&self, iface: &str, frame: Vec<u8>) -> Result<Tracked, ListenError> {
        let (tx, told) = channel();
        let (id, ahead) = self.queue(iface, frame, Some(tx))?;
        Ok(Tracked { id, ahead, told })
    }

    /// Take back a frame still waiting in its interface's queue: whether it was there (it is then
    /// never sent, counted withdrawn, and nothing is told of it). A frame its worker is already
    /// handing to its wire, or has sent, is not withdrawn.
    pub fn withdraw(&self, iface: &str, id: u64) -> bool {
        let mut st = lock(&self.shared.state);
        let Some(q) = st.queues.get_mut(iface) else { return false };
        let Some(at) = q.iter().position(|(i, _, _)| *i == id) else { return false };
        q.remove(at);
        let left = q.len();
        if let Some(i) = st.stats.interfaces.get_mut(iface) {
            i.queued = left;
            i.withdrawn += 1;
        }
        true
    }

    fn queue(&self, iface: &str, frame: Vec<u8>, tell: Option<Sender<Outcome>>) -> Result<(u64, usize), ListenError> {
        if frame.len() < 14 {
            return Err(ListenError::Frame(WireError::TooShort(frame.len())));
        }
        if frame.len() > MAX_FRAME {
            return Err(ListenError::Frame(WireError::TooLong(frame.len())));
        }
        let mut st = lock(&self.shared.state);
        if let Some(reason) = st.stats.interfaces.get(iface).and_then(|i| i.ended.clone()) {
            return Err(ListenError::Ended { iface: iface.to_string(), reason });
        }
        let bound = self.shared.options.queue;
        let q = st.queues.get_mut(iface).ok_or_else(|| ListenError::NotListening(iface.to_string()))?;
        if q.len() >= bound {
            return Err(ListenError::QueueFull { iface: iface.to_string(), queued: q.len() });
        }
        let ahead = q.len();
        let id = st.next_id;
        st.next_id += 1;
        let Some(q) = st.queues.get_mut(iface) else { return Err(ListenError::NotListening(iface.to_string())) };
        q.push_back((id, frame, tell));
        let queued = q.len();
        if let Some(i) = st.stats.interfaces.get_mut(iface) {
            i.queued = queued;
        }
        Ok((id, ahead))
    }

    /// The listening as it stands.
    pub fn snapshot(&self) -> Snapshot {
        let st = lock(&self.shared.state);
        Snapshot { fusion: st.fusion.clone(), conflicts: st.fusion.conflicts(), stats: st.stats.clone(), kept: (st.base, st.base + st.frames.len() as u64) }
    }

    /// The kept frame of this index, unless it was let go (or is yet to come).
    pub fn frame(&self, index: u64) -> Option<Kept> {
        let st = lock(&self.shared.state);
        index.checked_sub(st.base).and_then(|i| st.frames.get(usize::try_from(i).ok()?)).cloned()
    }

    /// Every kept frame, in the order kept.
    pub fn frames(&self) -> Vec<Kept> {
        lock(&self.shared.state).frames.iter().cloned().collect()
    }

    /// The kept frames from this index on, each with its index.
    pub fn frames_from(&self, index: u64) -> Vec<(u64, Kept)> {
        self.read_from(index).1
    }

    /// How many frames from this index on were let go, and the kept ones after them, each with its
    /// index: read at one moment, so none is let go between the count and the read.
    pub fn read_from(&self, index: u64) -> (u64, Vec<(u64, Kept)>) {
        let st = lock(&self.shared.state);
        let lost = st.base.saturating_sub(index);
        let skip = usize::try_from(index.saturating_sub(st.base)).unwrap_or(usize::MAX);
        (lost, st.frames.iter().enumerate().skip(skip).map(|(i, k)| (st.base + i as u64, k.clone())).collect())
    }

    /// The indices of the first frame still kept and of the next to come.
    pub fn kept(&self) -> (u64, u64) {
        let st = lock(&self.shared.state);
        (st.base, st.base + st.frames.len() as u64)
    }

    /// One interface's account, when a worker hears it.
    pub fn interface(&self, iface: &str) -> Option<Interface> {
        lock(&self.shared.state).stats.interfaces.get(iface).cloned()
    }

    /// This machine's own MACs: its wires', whose frames and whose subjects it leaves out of fusion.
    pub fn own(&self) -> &BTreeSet<Mac> {
        &self.shared.own
    }

    /// The system's index of the interface a wire of this name was opened on (C-139), taken at
    /// the start: `None` for a name it does not listen on and for a wire without one (a capture,
    /// the memory hub).
    pub fn index(&self, iface: &str) -> Option<u32> {
        self.shared.indexes.get(iface).copied()
    }

    /// The options it runs with.
    pub fn options(&self) -> Options {
        self.shared.options
    }

    /// Wait until `done` holds of the listening, at most `timeout`; whether it did.
    pub fn until(&self, timeout: Duration, done: impl Fn(&Snapshot) -> bool) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            if done(&self.snapshot()) {
                return true;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Stop every worker and give the listening as it ends.
    pub fn stop(mut self) -> Snapshot {
        self.halt();
        self.snapshot()
    }

    fn halt(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for h in self.workers.drain(..) {
            let _ = h.join();
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.halt();
    }
}

/// One wire's worker: its wire heard and sent on until the listener stops or the wire ends (a
/// panic of the wire's or of the worker's own ends it too, said so); then what is still queued for
/// it said unsent, and nothing queued for it again.
fn work(wire: Box<dyn Wire>, s: Arc<Shared>) {
    let iface = wire.name().to_string();
    let (shared, name) = (Arc::clone(&s), iface.clone());
    let ended = match std::panic::catch_unwind(AssertUnwindSafe(move || run(wire, &shared, &name))) {
        Ok(ended) => ended,
        Err(panic) => {
            let what = panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned());
            Some(format!("its worker panicked: {}", what.unwrap_or_else(|| "no message".to_string())))
        }
    };
    let mut st = lock(&s.state);
    let why = ended.clone().unwrap_or_else(|| "the listener stopped".to_string());
    let left: Vec<Queued> = st.queues.get_mut(&iface).map(|q| q.drain(..).collect()).unwrap_or_default();
    // A frame its worker was handing to the wire when the worker ended (a panic there) is never
    // sent either.
    let in_flight = st.in_flight.remove(&iface);
    if let Some(i) = st.stats.interfaces.get_mut(&iface) {
        i.ended = ended;
        i.unsent += left.len() as u64 + u64::from(in_flight.is_some());
        i.queued = 0;
    }
    // Told before the lock goes, as a send's outcome is.
    for tx in left.into_iter().filter_map(|(_, _, tell)| tell).chain(in_flight.flatten()) {
        let _ = tx.send(Outcome::Unsent(why.clone()));
    }
    drop(st);
}

/// Hear and send until stopped (`None`) or the wire ends (why).
fn run(mut wire: Box<dyn Wire>, s: &Shared, iface: &str) -> Option<String> {
    let iface = iface.to_string();
    let gap = Duration::from_secs(1) / s.options.rate;
    // A frame queued while the worker waits on its wire goes when the wait ends: no wait is longer
    // than half a gap, so the first goes within it and none is late by more.
    let poll = s.options.poll.min(gap / 2);
    let mut next_send = Instant::now();
    while !s.stop.load(Ordering::SeqCst) {
        // A frame queued, its turn come: sent, and kept as sent.
        if Instant::now() >= next_send {
            let out = {
                let mut st = lock(&s.state);
                let f = st.queues.get_mut(&iface).and_then(VecDeque::pop_front);
                let left = st.queues.get(&iface).map_or(0, VecDeque::len);
                if let Some(i) = st.stats.interfaces.get_mut(&iface) {
                    i.queued = left;
                }
                // A tracked frame being handed to the wire is told even if the worker ends there.
                f.map(|(_, bytes, tell)| {
                    st.in_flight.insert(iface.clone(), tell);
                    bytes
                })
            };
            if let Some(bytes) = out {
                next_send = Instant::now() + gap;
                let result = wire.send(&bytes);
                let mut st = lock(&s.state);
                let outcome = match result {
                    Ok(()) => {
                        let at_ns = stamp(SystemTime::now());
                        let index = st.keep(Kept { frame: Frame::new(at_ns, iface.clone(), bytes), sent: true }, s.options.max_frames);
                        if let Some(i) = st.stats.interfaces.get_mut(&iface) {
                            i.sent += 1;
                        }
                        Outcome::Sent { index, at_ns }
                    }
                    Err(e) => {
                        if let Some(i) = st.stats.interfaces.get_mut(&iface) {
                            i.refused += 1;
                        }
                        push_damage(&mut st.stats, format!("{iface}: send refused: {e}"));
                        Outcome::Refused(e.to_string())
                    }
                };
                // Told before the lock goes, so whoever finds the frame kept as sent (or refused,
                // counted) finds what became of it too; while the wire takes it, it is gone from
                // the queue and not yet told. An asker that stopped waiting is no fault of the
                // frame's.
                if let Some(tx) = st.in_flight.remove(&iface).flatten() {
                    let _ = tx.send(outcome);
                }
                drop(st);
            }
        }
        // Hear, waiting no longer than the poll, nor past the next send's turn when one is queued.
        let queued = lock(&s.state).queues.get(&iface).is_some_and(|q| !q.is_empty());
        let wait = if queued { poll.min(next_send.saturating_duration_since(Instant::now())) } else { poll };
        match wire.recv(wait) {
            Ok(Some(frame)) => hear(s, &iface, frame),
            Ok(None) => {}
            Err(WireError::Ended(_)) => return Some("its capture is read to the end".to_string()),
            Err(e) => return Some(e.to_string()),
        }
    }
    None
}

fn push_damage(stats: &mut Stats, line: String) {
    stats.damage.push_back(line);
    while stats.damage.len() > DAMAGE_KEPT {
        stats.damage.pop_front();
    }
}

/// A frame heard by the worker of `iface`: kept under that interface's name, decoded, its
/// observations fused but for those of this machine (sent from one of its MACs, or about one).
fn hear(s: &Shared, iface: &str, mut frame: Frame) {
    if frame.iface != iface {
        frame.iface = iface.to_string();
    }
    let mut st = lock(&s.state);
    let index = st.base + st.frames.len() as u64;
    let decoded = observe::decode(&frame, index);
    let from_here = ether::parse(&frame.bytes).is_ok_and(|e| s.own.contains(&e.src));
    let at = frame.at_ns;
    st.keep(Kept { frame, sent: false }, s.options.max_frames);
    st.stats.heard += 1;
    *st.stats.by_class.entry(decoded.class).or_default() += 1;
    st.stats.first_ns = Some(st.stats.first_ns.map_or(at, |f| f.min(at)));
    st.stats.last_ns = Some(st.stats.last_ns.map_or(at, |l| l.max(at)));
    if let Some(i) = st.stats.interfaces.get_mut(iface) {
        i.heard += 1;
    }
    if !decoded.damage.is_empty() {
        st.stats.damaged_frames += 1;
        for d in &decoded.damage {
            push_damage(&mut st.stats, format!("#{index} {iface}: {d}"));
        }
    }
    for o in &decoded.observations {
        if from_here || s.own.contains(&o.observed.subject()) {
            st.stats.own += 1;
            continue;
        }
        st.fusion.apply(o);
        st.stats.observations += 1;
    }
}
