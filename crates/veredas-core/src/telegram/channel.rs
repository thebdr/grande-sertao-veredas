//! The telegram channel: one TCP link, held correctly.
//!
//! Either side of a link uses it, listening or dialling, because the
//! obligations are the same in both directions and two copies would drift.
//! It knows framing and timing, never what a telegram means.
//!
//! The link rules (sequence numbers and the keep-alive probe are settings
//! of the envelope framing, `codec`):
//!
//! - Either role: **listen** for the peer, or **dial** it and re-dial
//!   with backoff until it answers.
//! - On connect, both sides send a keep-alive with sequence number 0;
//!   after that the number cycles **1..255**, stamped at send time, and
//!   the receiver skips a frame repeating the last number it saw.
//! - A receiver that has heard nothing for the receive timeout (10 s by
//!   default) sends a keep-alive to probe the link; if the write fails
//!   the socket is closed — a listener waits again, a dialler re-dials.
//!
//! Hardening, pinned by `tests/telegram_channel.rs`:
//!
//! - **Replies are unicast, keyed by a monotonic peer id** — never a
//!   Vec index (a disconnect shifts indices and a queued reply would
//!   reach the wrong link) and never a broadcast (a second connection
//!   would receive another link's replies).
//! - **A non-reading peer cannot wedge the channel.** Frames go to the
//!   socket as soon as it takes them; a bounded per-peer outbox holds
//!   only what it refuses, and a peer whose writes cannot complete within
//!   the write timeout, or whose outbox overflows, is dropped and counted.
//! - **`set_nodelay(true)`** on every socket: Nagle plus delayed ACK can
//!   add ~200 ms to a small reply, and a device waiting on a reply may
//!   allow tens of milliseconds.
//! - **Sequence numbers are stamped at send time**, not when a reply is
//!   decided — a delayed reply would otherwise leave our own counter
//!   non-monotonic.
//! - **Decode is interleaved with reads**, so the "not telegram framed"
//!   guard only measures an undecodable prefix; a legitimate reconnect
//!   burst of valid frames is not mistaken for garbage.
//! - Resyncs are counted, because a climbing resync count is the visible
//!   symptom of a wrong endian setting.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;


use crate::telegram::codec::{self, DecodeError};
use crate::telegram::{Endian, Envelope, Telegram};

/// How often the channel services its queues.
pub const TICK: Duration = Duration::from_millis(20);
/// The default receive timeout: silence this long triggers a keep-alive
/// probe.
pub const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a dial may take before it counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Reconnect backoff bounds for the client role.
const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(15);
/// A peer whose socket has taken no byte of its queued frames for this long
/// is not reading; it is dropped rather than allowed to wedge the channel.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Bytes queued for one peer, beyond what its socket has taken, before it is
/// judged unable to keep up: a memory bound. Whether a peer reads is the
/// write timeout's judgement, not a frame count (C-012, refute round 2: a
/// count of 256 frames dropped peers that were reading).
pub const OUTBOX_LIMIT_BYTES: usize = 16 * 1024 * 1024;
/// Reads per peer per tick, so one flooding peer cannot starve the others.
const READS_PER_TICK: usize = 64;
/// Commands handled per wake-up before the tick gets its turn again.
const COMMANDS_PER_WAKE: usize = 4096;
/// Undecodable bytes tolerated before a peer is judged to be speaking
/// some other protocol. Only ever measures a prefix that yielded no
/// frame — a valid backlog is decoded as it arrives.
const MAX_UNDECODABLE: usize = 64 * 1024;
/// Simultaneous connections accepted in the listen role.
const MAX_PEERS: usize = 8;

/// Which side of the link this channel is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Wait for the peer to connect.
    Listen { bind: String, port: u16 },
    /// Dial the peer.
    Connect { host: String, port: u16 },
}

/// How this channel identifies itself in the envelope.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    pub name: String,
    pub role: Role,
    pub endian: Endian,
    pub source_node: u8,
    pub destination_node: u8,
    /// Idle time before a keep-alive probe. 0 uses the spec's 10 s.
    pub keepalive_idle_ms: u64,
    /// Peers allowed at once (listen role).
    pub max_peers: usize,
}

impl ChannelConfig {
    pub fn new(name: impl Into<String>, role: Role) -> ChannelConfig {
        ChannelConfig {
            name: name.into(),
            role,
            endian: Endian::default(),
            source_node: 1,
            destination_node: 2,
            keepalive_idle_ms: RECEIVE_TIMEOUT.as_millis() as u64,
            max_peers: MAX_PEERS,
        }
    }

    fn keepalive_idle(&self) -> Duration {
        if self.keepalive_idle_ms == 0 {
            RECEIVE_TIMEOUT
        } else {
            Duration::from_millis(self.keepalive_idle_ms)
        }
    }
}

/// A connected peer's identity. Monotonic and never reused, so a reply
/// queued for a peer that has since disconnected is dropped rather than
/// delivered to whoever took its place.
pub type PeerId = u64;

/// What the channel tells its owner.
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A peer connected. `addr` is what it dialled from.
    Connected {
        peer: PeerId,
        addr: String,
    },
    Disconnected {
        peer: PeerId,
        why: String,
    },
    /// A decoded frame. Segments are delivered whole so the owner can
    /// answer per segment or per telegram as its protocol requires.
    Received {
        peer: PeerId,
        telegram: Telegram,
        /// The frame exactly as it arrived — what the lifecycle store
        /// keeps so every event can be re-decoded later.
        raw: Vec<u8>,
    },
    /// A keep-alive arrived: the link is alive though quiet. Health must
    /// fold this in, or a quiet-but-healthy peer reads as idle.
    KeepAlive {
        peer: PeerId,
    },
    /// The channel's own trouble (a bind failure, a dial that keeps
    /// failing, a stream that will not decode).
    Trouble(String),
}

/// Something to send.
pub enum ChannelCommand {
    /// Send to one peer. `raw` already contains the encoded frame apart
    /// from its PRG, which is stamped at send time.
    Send {
        peer: PeerId,
        telegram: Telegram,
    },
    /// Send to every connected peer (status broadcasts, not replies).
    SendAll {
        telegram: Telegram,
    },
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
    /// Frames skipped because their PRG repeated the previous one
    /// A climbing count means the peer is retransmitting.
    pub duplicates_skipped: u64,
    /// Bytes discarded to regain framing. A climbing count is the
    /// visible symptom of a wrong endian setting.
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
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
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
        Channel {
            status,
            commands: cmd_tx,
            events: event_rx,
            thread,
        }
    }

    /// Drain whatever the channel has said. Never blocks.
    pub fn poll(&self) -> Vec<ChannelEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    /// Wait for the next event, up to `timeout` — for tests and for a
    /// caller that has nothing else to do.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<ChannelEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    pub fn send(&self, peer: PeerId, telegram: Telegram) {
        let _ = self.commands.send(ChannelCommand::Send { peer, telegram });
    }

    pub fn send_all(&self, telegram: Telegram) {
        let _ = self.commands.send(ChannelCommand::SendAll { telegram });
    }

    pub fn status(&self) -> ChannelStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
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

/// The next peer id, process-wide: ids must never repeat, so a stale
/// reply cannot reach a new connection.
static NEXT_PEER: AtomicU64 = AtomicU64::new(1);

/// One connected socket: its buffers, its counters, its sequence state.
struct Peer {
    id: PeerId,
    addr: String,
    socket: tokio::net::TcpStream,
    /// Bytes not yet forming a complete frame.
    inbox: Vec<u8>,
    /// Frames waiting to go out, bounded.
    outbox: VecDeque<Vec<u8>>,
    /// How much of the front frame the socket has already taken.
    front_sent: usize,
    /// Bytes in the outbox not yet taken by the socket.
    queued_bytes: usize,
    /// Since when the socket has taken nothing while bytes wait: the write
    /// timeout runs from here, checked on each tick, never awaited.
    stalled_since: Option<tokio::time::Instant>,
    /// Our own sequence number, stamped at SEND time.
    prg: u8,
    /// The last PRG we accepted from this peer, for duplicate skipping.
    last_seen_prg: Option<u8>,
    /// When we last wrote and last read, for the keep-alive probe.
    last_tx: tokio::time::Instant,
    last_rx: tokio::time::Instant,
    /// Undecodable prefix length, reset whenever a frame decodes.
    undecodable: usize,
}

impl Peer {
    fn new(id: PeerId, addr: String, socket: tokio::net::TcpStream) -> Peer {
        // Nagle off: a small destination reply must not wait for an ACK.
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
            // Sequence 0 on connect (the connect keep-alive);
            // the first real frame is 1.
            prg: 0,
            last_seen_prg: None,
            last_tx: now,
            last_rx: now,
            undecodable: 0,
        }
    }

    /// The next PRG to put on the wire: 1..255, never 0 again (0 is
    /// reserved for the connect keep-alive).
    fn next_prg(&mut self) -> u8 {
        self.prg = if self.prg == u8::MAX { 1 } else { self.prg + 1 };
        self.prg
    }

    /// Queue a frame and hand the socket whatever it takes now, without
    /// waiting. The outbox only fills while the socket refuses bytes, so a
    /// burst to a peer that reads goes straight out (Veredas 1 queued until
    /// the next tick, and a burst or a steady stream to a reading peer
    /// overflowed the outbox and dropped it as "not reading"; C-012, refute
    /// round 1). `Err` means the peer must be dropped.
    fn queue(&mut self, bytes: Vec<u8>, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        self.queued_bytes += bytes.len();
        self.outbox.push_back(bytes);
        self.push_now(status)?;
        if self.queued_bytes > OUTBOX_LIMIT_BYTES {
            return Err(format!(
                "{} is not reading: {} bytes queued",
                self.addr, self.queued_bytes
            ));
        }
        Ok(())
    }

    /// Write queued bytes as far as the socket accepts them right now, and
    /// note since when it has taken nothing while bytes wait.
    fn push_now(&mut self, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        let mut progressed = false;
        while let Some(frame) = self.outbox.front() {
            match self.socket.try_write(&frame[self.front_sent..]) {
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

    /// Push what the socket takes now, and judge the peer: one whose socket
    /// has taken nothing for `WRITE_TIMEOUT` is not reading. Never waits, so
    /// a deaf peer stalls nobody else (C-012, refute round 2: awaiting the
    /// socket here held every other peer for the whole timeout). `Err` means
    /// the peer must be dropped.
    fn service_writes(&mut self, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        self.push_now(status)?;
        match self.stalled_since {
            Some(since) if since.elapsed() >= WRITE_TIMEOUT => {
                Err("write timed out — the peer is not reading".into())
            }
            _ => Ok(()),
        }
    }

    /// Read what is already there and decode every complete frame. Never
    /// waits: a timed read per peer (Veredas 1 gave each 1 ms, which a coarse
    /// timer stretches to 15 ms) made the tick outlast its period with two
    /// peers, and commands backed up behind it (C-012, refute round 2).
    fn pump(
        &mut self,
        endian: Endian,
        status: &Arc<Mutex<ChannelStatus>>,
        events: &std::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), String> {
        let mut chunk = [0u8; 8192];
        for _ in 0..READS_PER_TICK {
            match self.socket.try_read(&mut chunk) {
                Ok(0) => return Err("the peer closed the connection".into()),
                Ok(n) => {
                    self.inbox.extend_from_slice(&chunk[..n]);
                    self.last_rx = tokio::time::Instant::now();
                    // Decode as we read (not after): a legitimate burst
                    // of valid frames must not look like garbage.
                    self.drain_frames(endian, status, events);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
            if self.undecodable > MAX_UNDECODABLE {
                return Err(format!(
                    "{} bytes without a telegram — is this the right protocol (or byte order)?",
                    self.undecodable
                ));
            }
        }
        self.drain_frames(endian, status, events);
        Ok(())
    }

    fn drain_frames(
        &mut self,
        endian: Endian,
        status: &Arc<Mutex<ChannelStatus>>,
        events: &std::sync::mpsc::Sender<ChannelEvent>,
    ) {
        loop {
            match codec::decode(&self.inbox, endian) {
                Ok((telegram, used)) => {
                    let raw = self.inbox[..used].to_vec();
                    self.inbox.drain(..used);
                    self.undecodable = 0;

                    // The receiver skips a repeated sequence
                    // number. A keep-alive is never skipped, but its PRG
                    // (bar the connect keep-alive's 0) is the last one
                    // seen: idle probes take PRGs from the sender's
                    // counter, and ignoring them let the frame after 254
                    // probes carry the last data frame's PRG and be skipped
                    // (C-012, refute round 1).
                    let prg = telegram.envelope.prg;
                    if !telegram.is_keepalive() && self.last_seen_prg == Some(prg) {
                        bump(status, |s| s.duplicates_skipped += 1);
                        continue;
                    }
                    if !telegram.is_keepalive() || prg != 0 {
                        self.last_seen_prg = Some(prg);
                    }

                    if telegram.is_keepalive() {
                        bump(status, |s| s.keepalives_received += 1);
                        let _ = events.send(ChannelEvent::KeepAlive { peer: self.id });
                    } else {
                        bump(status, |s| s.received += 1);
                        let _ = events.send(ChannelEvent::Received {
                            peer: self.id,
                            telegram,
                            raw,
                        });
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

async fn run(
    cfg: ChannelConfig,
    status: Arc<Mutex<ChannelStatus>>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<ChannelCommand>,
    events: std::sync::mpsc::Sender<ChannelEvent>,
) {
    let mut peers: Vec<Peer> = Vec::new();
    let mut listener: Option<tokio::net::TcpListener> = None;
    let mut retry_at: Option<tokio::time::Instant> = Some(tokio::time::Instant::now());
    let mut backoff = RETRY_MIN;

    if let Role::Listen { bind, port } = &cfg.role {
        match tokio::net::TcpListener::bind((bind.trim(), *port)).await {
            Ok(l) => {
                let where_ = l
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| format!("{bind}:{port}"));
                set_state(&status, format!("listening on {where_}"));
                listener = Some(l);
            }
            Err(e) => {
                set_state(&status, "failed");
                set_error(&status, format!("binding {bind}:{port}: {e}"));
                let _ = events.send(ChannelEvent::Trouble(format!(
                    "cannot listen on {bind}:{port}: {e}"
                )));
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

    let keepalive_idle = cfg.keepalive_idle();
    let envelope = |prg: u8| Envelope {
        class: 0,
        source: cfg.source_node,
        destination: cfg.destination_node,
        prg,
    };

    // One interval for the life of the channel. Veredas 1 armed a fresh
    // `sleep(TICK)` on every pass of the loop, so every command restarted
    // it: under a steady stream of sends the tick never fired, and nothing
    // was read, flushed or probed (C-012, refute round 1).
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let accepted = async {
            match &listener {
                Some(l) if peers.len() < cfg.max_peers.max(1) => {
                    l.accept().await.map(|(s, a)| (s, a.to_string())).ok()
                }
                // Nothing to accept (dialling role, or at the peer cap).
                _ => std::future::pending().await,
            }
        };

        tokio::select! {
            cmd = commands.recv() => {
                // Handle this command and every one already queued behind
                // it, so a burst never waits a tick per frame and a Stop is
                // never stuck behind a backlog (C-012, refute round 2).
                let mut next = cmd;
                let mut stop = false;
                for _ in 0..COMMANDS_PER_WAKE {
                    match next {
                        None | Some(ChannelCommand::Stop) => {
                            stop = true;
                            break;
                        }
                        Some(cmd) => handle_command(cmd, &mut peers, &cfg, &status, &events),
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
                // Sequence 0 and a keep-alive on connect.
                if let Ok(bytes) = codec::encode(&Telegram::keepalive(envelope(0)), cfg.endian) {
                    let _ = peer.queue(bytes, &status);
                }
                peers.push(peer);
                set_state(&status, format!("{} peer(s)", peers.len()));
                bump(&status, |s| s.peers = peers.len());
                let _ = events.send(ChannelEvent::Connected { peer: id, addr });
            }
            _ = tick.tick() => {
                // Dial out when it is time to (client role).
                if let Role::Connect { host, port } = &cfg.role
                    && peers.is_empty()
                        && retry_at.map(|t| tokio::time::Instant::now() >= t).unwrap_or(false)
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
                                if let Ok(bytes) =
                                    codec::encode(&Telegram::keepalive(envelope(0)), cfg.endian)
                                {
                                    let _ = peer.queue(bytes, &status);
                                }
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

                // Read, decode, then write — and drop whoever failed.
                let mut doomed: Vec<(PeerId, String)> = Vec::new();
                for p in peers.iter_mut() {
                    if let Err(e) = p.pump(cfg.endian, &status, &events) {
                        doomed.push((p.id, e));
                        continue;
                    }
                    // Silence for the receive timeout means probe
                    // the link with a keep-alive.
                    if p.last_rx.elapsed() >= keepalive_idle
                        && p.last_tx.elapsed() >= keepalive_idle
                    {
                        let prg = p.next_prg();
                        if let Ok(bytes) =
                            codec::encode(&Telegram::keepalive(envelope(prg)), cfg.endian)
                            && p.queue(bytes, &status).is_ok() {
                                bump(&status, |s| s.keepalives_sent += 1);
                            }
                        // Reset the read clock so the probe is sent once
                        // per idle window, not every tick.
                        p.last_rx = tokio::time::Instant::now();
                    }
                    if let Err(e) = p.service_writes(&status) {
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

/// The envelope this channel stamps on a frame.
fn envelope_for(cfg: &ChannelConfig, prg: u8) -> Envelope {
    Envelope {
        class: 0,
        source: cfg.source_node,
        destination: cfg.destination_node,
        prg,
    }
}

/// One send command: stamp, encode, queue, and drop a peer whose outbox
/// overflows.
fn handle_command(
    cmd: ChannelCommand,
    peers: &mut Vec<Peer>,
    cfg: &ChannelConfig,
    status: &Arc<Mutex<ChannelStatus>>,
    events: &std::sync::mpsc::Sender<ChannelEvent>,
) {
    match cmd {
        ChannelCommand::Stop => {}
        ChannelCommand::Send { peer, telegram } => {
            // Unicast by ID: a peer that has gone is a drop, never a
            // delivery to whoever replaced it.
            match peers.iter_mut().find(|p| p.id == peer) {
                Some(p) => {
                    let prg = p.next_prg();
                    let mut frame = telegram;
                    frame.envelope = envelope_for(cfg, prg);
                    match codec::encode(&frame, cfg.endian) {
                        Ok(bytes) => {
                            if let Err(e) = p.queue(bytes, status) {
                                set_error(status, e.clone());
                                let id = p.id;
                                drop_peer(peers, id, &e, status, events);
                            }
                        }
                        Err(e) => set_error(status, e.to_string()),
                    }
                }
                None => set_error(status, format!("peer {peer} is gone; the reply was dropped")),
            }
        }
        ChannelCommand::SendAll { telegram } => {
            let mut doomed = Vec::new();
            for p in peers.iter_mut() {
                let prg = p.next_prg();
                let mut frame = telegram.clone();
                frame.envelope = envelope_for(cfg, prg);
                if let Ok(bytes) = codec::encode(&frame, cfg.endian)
                    && let Err(e) = p.queue(bytes, status)
                {
                    doomed.push((p.id, e));
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
        let _ = events.send(ChannelEvent::Disconnected {
            peer: id,
            why: why.to_string(),
        });
    }
}
