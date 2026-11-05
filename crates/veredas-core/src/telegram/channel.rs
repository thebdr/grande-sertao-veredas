//! The telegram channel: one TCP link, held correctly, by rules that are configuration.
//!
//! Either side of a link uses it, listening or dialling, because the obligations are the same
//! in both directions and two copies would drift. It knows framing and timing from its
//! [`ChannelSpec`] (a channel file, [`crate::telegram::schema`]), never what a telegram means.
//!
//! The link rules, each a setting of the channel file:
//!
//! - Either role: **listen** for the peer, or **dial** it and re-dial with backoff until it
//!   answers.
//! - **Sequence** (when the layout has a sequence field and the file has `[sequence]` rules):
//!   the connect keep-alive carries `connect`; frames then carry `first..=last`, cycling,
//!   stamped at send time; the receiver skips a frame repeating the last number it saw, and a
//!   keep-alive's number counts as seen when `keepalive_counts` says so.
//! - **Keep-alive**: one on connect when `on_connect`; a probe after `idle` of silence (nothing
//!   read, nothing written), once per window, never while the peer talks.
//! - **A peer that stops reading** (its socket takes nothing for `write_timeout`) is dropped.
//!
//! Hardening, pinned by `tests/telegram_channel.rs`:
//!
//! - **Replies are unicast, keyed by a monotonic peer id**, never an index (a disconnect shifts
//!   indices and a queued reply would reach the wrong link) and never a broadcast.
//! - **Frames go to the socket as soon as it takes them**; a bounded per-peer outbox holds only
//!   what it refuses, and every queued command is handled on each wake-up, so a reading peer
//!   keeps up with a burst whatever other peers are connected.
//! - **Nothing in the service tick waits**: reads are non-blocking, and the write timeout is a
//!   clock checked each tick, so a deaf peer stalls no other peer and no shutdown.
//! - **`set_nodelay(true)`** on every socket: Nagle plus delayed ACK can add ~200 ms to a small
//!   reply.
//! - **Decode is interleaved with reads**, so the "not framed" guard only measures an
//!   undecodable prefix; resyncs are counted, a climbing count being the visible symptom of a
//!   wrong byte order.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::telegram::framing::{DecodeError, Frame};
use crate::telegram::schema::{ChannelSpec, SequenceRules};

/// How often the channel services its peers.
pub const TICK: Duration = Duration::from_millis(20);
/// How long a dial may take before it counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Reconnect backoff bounds for the dialling role.
const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(15);
/// Reads per peer per tick, so one flooding peer cannot starve the others.
const READS_PER_TICK: usize = 64;
/// Commands handled per wake-up before the tick gets its turn again.
const COMMANDS_PER_WAKE: usize = 4096;
/// Undecodable bytes tolerated (at least this, or one largest frame) before a peer is judged to
/// be speaking some other protocol.
const MIN_UNDECODABLE: usize = 64 * 1024;

/// Which side of the link this channel is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Wait for the peer to connect.
    Listen { bind: String, port: u16 },
    /// Dial the peer.
    Connect { host: String, port: u16 },
}

/// A channel: its name, its role, and the rules of its file.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    pub name: String,
    pub role: Role,
    pub spec: ChannelSpec,
}

impl ChannelConfig {
    pub fn new(name: impl Into<String>, role: Role, spec: ChannelSpec) -> ChannelConfig {
        ChannelConfig { name: name.into(), role, spec }
    }
}

/// A connected peer's identity. Monotonic and never reused, so a reply queued for a peer that
/// has since disconnected is dropped rather than delivered to whoever took its place.
pub type PeerId = u64;

/// What the channel tells its owner.
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A peer connected. `addr` is what it dialled from.
    Connected { peer: PeerId, addr: String },
    Disconnected { peer: PeerId, why: String },
    /// A decoded frame, segments whole.
    Received {
        peer: PeerId,
        frame: Frame,
        /// The frame exactly as it arrived, so every event can be decoded again later.
        raw: Vec<u8>,
    },
    /// A keep-alive arrived: the link is alive though quiet.
    KeepAlive { peer: PeerId },
    /// The channel's own trouble (a bind failure, a dial that keeps failing).
    Trouble(String),
}

/// Something to send.
pub enum ChannelCommand {
    /// Send to one peer; the sequence number is stamped at send time.
    Send { peer: PeerId, frame: Frame },
    /// Send to every connected peer (status broadcasts, not replies).
    SendAll { frame: Frame },
    Stop,
}

/// Live counters and state, for a status pane.
#[derive(Clone, Debug, Default)]
pub struct ChannelStatus {
    pub state: String,
    pub peers: usize,
    pub sent: u64,
    pub received: u64,
    pub keepalives_sent: u64,
    pub keepalives_received: u64,
    /// Frames skipped because their sequence number repeated the last one seen. A climbing
    /// count means the peer is retransmitting.
    pub duplicates_skipped: u64,
    /// Times bytes were discarded to regain framing.
    pub resyncs: u64,
    pub dropped_peers: u64,
    pub last_error: Option<String>,
}

/// A running channel.
pub struct Channel {
    pub status: Arc<Mutex<ChannelStatus>>,
    commands: tokio::sync::mpsc::UnboundedSender<ChannelCommand>,
    events: std::sync::mpsc::Receiver<ChannelEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Channel {
    /// Start the channel on its own thread and runtime.
    pub fn start(cfg: ChannelConfig) -> Channel {
        let status = Arc::new(Mutex::new(ChannelStatus {
            state: "starting".into(),
            ..Default::default()
        }));
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let worker_status = status.clone();
        let name = cfg.name.clone();
        let thread = std::thread::Builder::new()
            .name(format!("telegram-{name}"))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        if let Ok(mut s) = worker_status.lock() {
                            s.state = "failed".into();
                            s.last_error = Some(format!("runtime: {e}"));
                        }
                        return;
                    }
                };
                rt.block_on(run(cfg, worker_status, cmd_rx, event_tx));
            })
            .ok();
        Channel { status, commands: cmd_tx, events: event_rx, thread }
    }

    /// Drain whatever the channel has said. Never blocks.
    pub fn poll(&self) -> Vec<ChannelEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    /// Wait for the next event, up to `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<ChannelEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    pub fn send(&self, peer: PeerId, frame: Frame) {
        let _ = self.commands.send(ChannelCommand::Send { peer, frame });
    }

    pub fn send_all(&self, frame: Frame) {
        let _ = self.commands.send(ChannelCommand::SendAll { frame });
    }

    pub fn status(&self) -> ChannelStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().map(|t| t.is_finished()).unwrap_or(true)
    }

    fn shutdown_and_join(&mut self) {
        let _ = self.commands.send(ChannelCommand::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

fn set_state(status: &Arc<Mutex<ChannelStatus>>, state: impl Into<String>) {
    if let Ok(mut s) = status.lock() {
        s.state = state.into();
    }
}

fn set_error(status: &Arc<Mutex<ChannelStatus>>, msg: impl Into<String>) {
    if let Ok(mut s) = status.lock() {
        s.last_error = Some(msg.into());
    }
}

fn bump(status: &Arc<Mutex<ChannelStatus>>, f: impl FnOnce(&mut ChannelStatus)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
}

/// The next peer id, process-wide: ids never repeat, so a stale reply cannot reach a new
/// connection.
static NEXT_PEER: AtomicU64 = AtomicU64::new(1);

/// One connected socket: its buffers, its counters, its sequence state.
struct Peer {
    id: PeerId,
    addr: String,
    socket: tokio::net::TcpStream,
    /// Bytes not yet forming a complete frame.
    inbox: Vec<u8>,
    /// Frames the socket has not taken yet.
    outbox: VecDeque<Vec<u8>>,
    /// How much of the front frame the socket has already taken.
    front_sent: usize,
    /// Bytes in the outbox not yet taken by the socket.
    queued_bytes: usize,
    /// Since when the socket has taken nothing while bytes wait: the write timeout runs from
    /// here, checked on each tick, never awaited.
    stalled_since: Option<tokio::time::Instant>,
    /// The last sequence number stamped after connect (`None`: the next is `first`).
    sequence: Option<u64>,
    /// The last sequence number seen from this peer, for duplicate skipping.
    last_seen: Option<u64>,
    /// When we last wrote and last read, for the keep-alive probe.
    last_tx: tokio::time::Instant,
    last_rx: tokio::time::Instant,
    /// Undecodable prefix length, reset whenever a frame decodes.
    undecodable: usize,
}

impl Peer {
    fn new(id: PeerId, addr: String, socket: tokio::net::TcpStream) -> Peer {
        // Nagle off: a small reply must not wait for an ACK.
        let _ = socket.set_nodelay(true);
        let now = tokio::time::Instant::now();
        Peer {
            id,
            addr,
            socket,
            inbox: Vec::new(),
            outbox: VecDeque::new(),
            front_sent: 0,
            queued_bytes: 0,
            stalled_since: None,
            sequence: None,
            last_seen: None,
            last_tx: now,
            last_rx: now,
            undecodable: 0,
        }
    }

    /// The next sequence number to put on the wire: `first..=last`, cycling.
    fn next_sequence(&mut self, rules: &SequenceRules) -> u64 {
        let next = match self.sequence {
            Some(s) if s >= rules.first && s < rules.last => s + 1,
            _ => rules.first,
        };
        self.sequence = Some(next);
        next
    }

    /// Queue a frame and hand the socket whatever it takes now, without waiting. `Err` means
    /// the peer must be dropped (its outbox outgrew the memory bound).
    fn queue(&mut self, bytes: Vec<u8>, spec: &ChannelSpec, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        self.queued_bytes += bytes.len();
        self.outbox.push_back(bytes);
        self.push_now(status)?;
        if self.queued_bytes > spec.link.outbox_limit_bytes {
            return Err(format!("{} is not reading: {} bytes queued", self.addr, self.queued_bytes));
        }
        Ok(())
    }

    /// Write queued bytes as far as the socket accepts them right now, and note since when it
    /// has taken nothing while bytes wait.
    fn push_now(&mut self, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        let mut progressed = false;
        while let Some(frame) = self.outbox.front() {
            // Written through the socket itself, not tokio's readiness cache: tokio answers
            // WouldBlock without asking the OS until it has seen a writable edge, and Linux
            // signals one only when a third of the send buffer is free, so a slow but reading
            // peer would look deaf (C-012, refute round 3).
            match socket2::SockRef::from(&self.socket).send(&frame[self.front_sent..]) {
                Ok(n) => {
                    progressed |= n > 0;
                    self.front_sent += n;
                    self.queued_bytes -= n;
                    if self.front_sent == frame.len() {
                        self.outbox.pop_front();
                        self.front_sent = 0;
                        self.last_tx = tokio::time::Instant::now();
                        bump(status, |s| s.sent += 1);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if self.outbox.is_empty() {
            self.stalled_since = None;
        } else if progressed || self.stalled_since.is_none() {
            self.stalled_since = Some(tokio::time::Instant::now());
        }
        Ok(())
    }

    /// Push what the socket takes now, and judge the peer: one whose socket has taken nothing
    /// for the write timeout is not reading. Never waits.
    fn service_writes(&mut self, spec: &ChannelSpec, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        self.push_now(status)?;
        match self.stalled_since {
            Some(since) if since.elapsed() >= spec.link.write_timeout => {
                Err("write timed out — the peer is not reading".into())
            }
            _ => Ok(()),
        }
    }

    /// Read what is already there and decode every complete frame. Never waits.
    fn pump(
        &mut self,
        spec: &ChannelSpec,
        status: &Arc<Mutex<ChannelStatus>>,
        events: &std::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), String> {
        let mut chunk = [0u8; 8192];
        let limit = MIN_UNDECODABLE.max(spec.framing.max_frame);
        for _ in 0..READS_PER_TICK {
            match self.socket.try_read(&mut chunk) {
                Ok(0) => return Err("the peer closed the connection".into()),
                Ok(n) => {
                    self.inbox.extend_from_slice(&chunk[..n]);
                    self.last_rx = tokio::time::Instant::now();
                    // Decode as we read: a legitimate burst must not look like garbage.
                    self.drain_frames(spec, status, events);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
            if self.undecodable > limit {
                return Err(format!(
                    "{} bytes without a frame — is this the right framing (or byte order)?",
                    self.undecodable
                ));
            }
        }
        Ok(())
    }

    fn drain_frames(
        &mut self,
        spec: &ChannelSpec,
        status: &Arc<Mutex<ChannelStatus>>,
        events: &std::sync::mpsc::Sender<ChannelEvent>,
    ) {
        loop {
            match spec.framing.decode(&self.inbox) {
                Ok((frame, used)) => {
                    let raw: Vec<u8> = self.inbox.drain(..used).collect();
                    self.undecodable = 0;
                    let keepalive = frame.is_keepalive();
                    if let (Some(rules), Some(seq)) = (&spec.sequence, spec.framing.sequence_of(&frame)) {
                        // The receiver skips a repeated number. A keep-alive is never skipped;
                        // its number counts as seen when the rules say so (bar the connect
                        // keep-alive's), because a sender whose probes take numbers can come
                        // back round to the last data frame's.
                        if rules.skip_duplicates && !keepalive && self.last_seen == Some(seq) {
                            bump(status, |s| s.duplicates_skipped += 1);
                            continue;
                        }
                        if !keepalive || (rules.keepalive_counts && seq != rules.connect) {
                            self.last_seen = Some(seq);
                        }
                    }
                    if keepalive {
                        bump(status, |s| s.keepalives_received += 1);
                        let _ = events.send(ChannelEvent::KeepAlive { peer: self.id });
                    } else {
                        bump(status, |s| s.received += 1);
                        let _ = events.send(ChannelEvent::Received { peer: self.id, frame, raw });
                    }
                }
                Err(DecodeError::NeedMore) => break,
                Err(DecodeError::BadFrame { resync }) => {
                    let skip = resync.max(1).min(self.inbox.len());
                    self.inbox.drain(..skip);
                    self.undecodable += skip;
                    bump(status, |s| s.resyncs += 1);
                    if self.inbox.is_empty() {
                        break;
                    }
                }
            }
        }
    }
}

/// Encode a frame for a peer, stamping its next sequence number when the rules ask for one.
fn encode_for(peer: &mut Peer, spec: &ChannelSpec, frame: &Frame) -> Result<Vec<u8>, String> {
    let sequence = spec.sequence.as_ref().map(|rules| peer.next_sequence(rules));
    spec.framing.encode(frame, sequence).map_err(|e| e.to_string())
}

/// A new peer: send the connect keep-alive when the rules ask for one.
fn greet(peer: &mut Peer, spec: &ChannelSpec, status: &Arc<Mutex<ChannelStatus>>) {
    if spec.keepalive.on_connect {
        let connect = spec.sequence.as_ref().map(|r| r.connect);
        if let Ok(bytes) = spec.framing.encode(&Frame::default(), connect) {
            let _ = peer.queue(bytes, spec, status);
        }
    }
}

async fn run(
    cfg: ChannelConfig,
    status: Arc<Mutex<ChannelStatus>>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<ChannelCommand>,
    events: std::sync::mpsc::Sender<ChannelEvent>,
) {
    let spec = &cfg.spec;
    let mut peers: Vec<Peer> = Vec::new();
    let mut listener: Option<tokio::net::TcpListener> = None;
    let mut retry_at: Option<tokio::time::Instant> = Some(tokio::time::Instant::now());
    let mut backoff = RETRY_MIN;

    if let Role::Listen { bind, port } = &cfg.role {
        match tokio::net::TcpListener::bind((bind.trim(), *port)).await {
            Ok(l) => {
                let where_ = l.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| format!("{bind}:{port}"));
                set_state(&status, format!("listening on {where_}"));
                listener = Some(l);
            }
            Err(e) => {
                set_state(&status, "failed");
                set_error(&status, format!("binding {bind}:{port}: {e}"));
                let _ = events.send(ChannelEvent::Trouble(format!("cannot listen on {bind}:{port}: {e}")));
                // Nothing to serve; wait to be stopped rather than spin.
                while let Some(cmd) = commands.recv().await {
                    if matches!(cmd, ChannelCommand::Stop) {
                        break;
                    }
                }
                return;
            }
        }
    }

    // One interval for the life of the channel: a sleep re-armed on every pass would be
    // restarted by every command, and under a steady stream the tick would never fire.
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let accepted = async {
            match &listener {
                Some(l) if peers.len() < spec.link.max_peers => {
                    l.accept().await.map(|(s, a)| (s, a.to_string())).ok()
                }
                // Nothing to accept (dialling role, or at the peer cap).
                _ => std::future::pending().await,
            }
        };

        tokio::select! {
            cmd = commands.recv() => {
                // This command and every one already queued behind it, so a burst never waits
                // a tick per frame and a Stop is never stuck behind a backlog.
                let mut next = cmd;
                let mut stop = false;
                for _ in 0..COMMANDS_PER_WAKE {
                    match next {
                        None | Some(ChannelCommand::Stop) => {
                            stop = true;
                            break;
                        }
                        Some(cmd) => handle_command(cmd, &mut peers, spec, &status, &events),
                    }
                    match commands.try_recv() {
                        Ok(cmd) => next = Some(cmd),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => next = None,
                    }
                }
                if stop {
                    break;
                }
            }
            Some((socket, addr)) = accepted => {
                let id = NEXT_PEER.fetch_add(1, Ordering::Relaxed);
                let mut peer = Peer::new(id, addr.clone(), socket);
                greet(&mut peer, spec, &status);
                peers.push(peer);
                set_state(&status, format!("{} peer(s)", peers.len()));
                bump(&status, |s| s.peers = peers.len());
                let _ = events.send(ChannelEvent::Connected { peer: id, addr });
            }
            _ = tick.tick() => {
                // Dial out when it is time to (dialling role).
                if let Role::Connect { host, port } = &cfg.role
                    && peers.is_empty()
                    && retry_at.is_some_and(|t| tokio::time::Instant::now() >= t)
                {
                    set_state(&status, format!("connecting to {host}:{port}"));
                    match tokio::time::timeout(
                        CONNECT_TIMEOUT,
                        tokio::net::TcpStream::connect((host.trim(), *port)),
                    )
                    .await
                    {
                        Ok(Ok(socket)) => {
                            let id = NEXT_PEER.fetch_add(1, Ordering::Relaxed);
                            let addr = format!("{host}:{port}");
                            let mut peer = Peer::new(id, addr.clone(), socket);
                            greet(&mut peer, spec, &status);
                            peers.push(peer);
                            set_state(&status, format!("connected to {addr}"));
                            bump(&status, |s| { s.peers = 1; s.last_error = None; });
                            retry_at = None;
                            backoff = RETRY_MIN;
                            let _ = events.send(ChannelEvent::Connected { peer: id, addr });
                        }
                        Ok(Err(e)) => {
                            set_error(&status, format!("connecting: {e}"));
                            set_state(&status, "retrying");
                            retry_at = Some(tokio::time::Instant::now() + backoff);
                            backoff = (backoff * 2).min(RETRY_MAX);
                        }
                        Err(_) => {
                            set_error(&status, "connect timed out");
                            set_state(&status, "retrying");
                            retry_at = Some(tokio::time::Instant::now() + backoff);
                            backoff = (backoff * 2).min(RETRY_MAX);
                        }
                    }
                }

                // Read, decode, probe, write — and drop whoever failed. Nothing here waits.
                let mut doomed: Vec<(PeerId, String)> = Vec::new();
                for p in peers.iter_mut() {
                    if let Err(e) = p.pump(spec, &status, &events) {
                        doomed.push((p.id, e));
                        continue;
                    }
                    let idle = spec.keepalive.idle;
                    if !idle.is_zero() && p.last_rx.elapsed() >= idle && p.last_tx.elapsed() >= idle {
                        if let Ok(bytes) = encode_for(p, spec, &Frame::default())
                            && p.queue(bytes, spec, &status).is_ok()
                        {
                            bump(&status, |s| s.keepalives_sent += 1);
                        }
                        // Reset the read clock so the probe goes once per idle window.
                        p.last_rx = tokio::time::Instant::now();
                    }
                    if let Err(e) = p.service_writes(spec, &status) {
                        doomed.push((p.id, e));
                    }
                }
                for (id, why) in doomed {
                    drop_peer(&mut peers, id, &why, &status, &events);
                    if matches!(cfg.role, Role::Connect { .. }) {
                        retry_at = Some(tokio::time::Instant::now() + RETRY_MIN);
                    }
                }
            }
        }
    }
    set_state(&status, "stopped");
}

/// One send command: stamp, encode, queue, and drop a peer whose outbox overflows.
fn handle_command(
    cmd: ChannelCommand,
    peers: &mut Vec<Peer>,
    spec: &ChannelSpec,
    status: &Arc<Mutex<ChannelStatus>>,
    events: &std::sync::mpsc::Sender<ChannelEvent>,
) {
    match cmd {
        ChannelCommand::Stop => {}
        ChannelCommand::Send { peer, frame } => {
            // Unicast by id: a peer that has gone is a drop, never a delivery to whoever
            // replaced it.
            match peers.iter_mut().find(|p| p.id == peer) {
                Some(p) => match encode_for(p, spec, &frame) {
                    Ok(bytes) => {
                        if let Err(e) = p.queue(bytes, spec, status) {
                            set_error(status, e.clone());
                            let id = p.id;
                            drop_peer(peers, id, &e, status, events);
                        }
                    }
                    Err(e) => set_error(status, e),
                },
                None => set_error(status, format!("peer {peer} is gone; the reply was dropped")),
            }
        }
        ChannelCommand::SendAll { frame } => {
            let mut doomed = Vec::new();
            for p in peers.iter_mut() {
                match encode_for(p, spec, &frame) {
                    Ok(bytes) => {
                        if let Err(e) = p.queue(bytes, spec, status) {
                            doomed.push((p.id, e));
                        }
                    }
                    Err(e) => set_error(status, e),
                }
            }
            for (id, why) in doomed {
                drop_peer(peers, id, &why, status, events);
            }
        }
    }
}

fn drop_peer(
    peers: &mut Vec<Peer>,
    id: PeerId,
    why: &str,
    status: &Arc<Mutex<ChannelStatus>>,
    events: &std::sync::mpsc::Sender<ChannelEvent>,
) {
    if let Some(i) = peers.iter().position(|p| p.id == id) {
        let peer = peers.remove(i);
        bump(status, |s| {
            s.peers = peers.len();
            s.dropped_peers += 1;
            s.last_error = Some(format!("{}: {why}", peer.addr));
        });
        set_state(status, format!("{} peer(s)", peers.len()));
        let _ = events.send(ChannelEvent::Disconnected { peer: id, why: why.to_string() });
    }
}
