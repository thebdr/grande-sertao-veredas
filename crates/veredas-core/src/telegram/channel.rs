//! The telegram channel: one TCP link, spoken correctly.
//!
//! Shared by both sides — maquinar's simulated MSC and veredas's
//! External Unit — because the protocol's obligations are the same in
//! either direction and two copies would drift.
//!
//! Spec behaviour (`docs/vendor/genibelt-protocol.txt` §3):
//!
//! - The MSC is the **client**; the External Unit **listens**. Both
//!   roles exist here because the simulator plays either side.
//! - On connect, both sides set PRG to 0 and send a keep-alive; after
//!   that PRG cycles **1..255** and the RECEIVER skips a frame whose PRG
//!   repeats the last one (§3.1, duplicate detection).
//! - A receiver that has heard nothing for **10 seconds** sends a
//!   keep-alive to probe the link; if the write fails the socket is
//!   closed — a server re-listens, a client re-dials (§3.2).
//!
//! Hardening the adversarial review demanded, all pinned by tests below:
//!
//! - **Replies are unicast, keyed by a monotonic peer id** — never a
//!   Vec index (a disconnect shifts indices and a queued reply would
//!   reach the wrong link) and never a broadcast (a second connection
//!   would receive another link's sort decisions).
//! - **A non-reading peer cannot wedge the channel.** Writes go to a
//!   bounded per-peer outbox drained with a timeout; a peer that stalls
//!   or overflows is dropped and counted. The previous shape awaited
//!   `write_all` inside the select arm, so one hung controller froze
//!   every other peer, the keep-alives, and shutdown itself.
//! - **`set_nodelay(true)`** on every socket: Nagle plus delayed ACK can
//!   add ~200 ms to a small reply, and a sort controller's destination
//!   deadline is measured in tens of milliseconds.
//! - **PRG is stamped at send time**, not when a reply is decided — a
//!   delayed reply would otherwise leave our own counter non-monotonic.
//! - **Decode is interleaved with reads**, so the "not telegram framed"
//!   guard only measures an undecodable prefix; a legitimate reconnect
//!   burst of valid frames is not mistaken for garbage.
//! - Resyncs are counted, because a climbing resync count is the visible
//!   symptom of a wrong endian setting.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;

use crate::telegram::codec::{self, DecodeError};
use crate::telegram::{Endian, Envelope, Telegram};

/// How often the channel services its queues.
pub const TICK: Duration = Duration::from_millis(20);
/// §3.2: "each receiving side will enable a socket receive timeout (10
/// Seconds)" — silence this long triggers a keep-alive probe.
pub const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a dial may take before it counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Reconnect backoff bounds for the client role.
const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(15);
/// A write that cannot complete in this long means the peer is not
/// reading; it is dropped rather than allowed to wedge the channel.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Frames queued for one peer before it is judged unable to keep up.
pub const OUTBOX_LIMIT: usize = 256;
/// Undecodable bytes tolerated before a peer is judged to be speaking
/// some other protocol. Only ever measures a prefix that yielded no
/// frame — a valid backlog is decoded as it arrives.
const MAX_UNDECODABLE: usize = 64 * 1024;
/// Simultaneous connections accepted in the listen role.
const MAX_PEERS: usize = 8;

/// Which side of the link this channel is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Wait for the peer to connect (the External Unit's role).
    Listen { bind: String, port: u16 },
    /// Dial the peer (the MSC's role).
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
    /// fold this in, or a quiet-but-healthy MSC reads as Idle.
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
    /// (§3.1). A climbing count means the peer is retransmitting.
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
    /// Our own progressive number, stamped at SEND time (§3.1).
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
            // §3: "Set the Progressive Number field to 0" on connect;
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
        self.outbox.push_back(bytes);
        self.push_now(status)?;
        if self.outbox.len() > OUTBOX_LIMIT {
            return Err(format!(
                "{} is not reading: {OUTBOX_LIMIT} frames queued",
                self.addr
            ));
        }
        Ok(())
    }

    /// Write queued bytes as far as the socket accepts them right now.
    fn push_now(&mut self, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        while let Some(frame) = self.outbox.front() {
            match self.socket.try_write(&frame[self.front_sent..]) {
                Ok(n) => {
                    self.front_sent += n;
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
        Ok(())
    }

    /// Write everything queued, waiting for the socket at most
    /// `WRITE_TIMEOUT` in all. `Err` means the peer must be dropped.
    async fn flush(&mut self, status: &Arc<Mutex<ChannelStatus>>) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + WRITE_TIMEOUT;
        loop {
            self.push_now(status)?;
            if self.outbox.is_empty() {
                return Ok(());
            }
            match tokio::time::timeout_at(deadline, self.socket.writable()).await {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => return Err(e.to_string()),
                // The peer stopped reading. Dropping it is the only way
                // to keep the channel (and shutdown) responsive.
                Err(_) => return Err("write timed out — the peer is not reading".into()),
            }
        }
    }

    /// Read what is available and decode every complete frame.
    async fn pump(
        &mut self,
        endian: Endian,
        status: &Arc<Mutex<ChannelStatus>>,
        events: &std::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), String> {
        let mut chunk = [0u8; 8192];
        loop {
            // Read only what is already there: this must never block the
            // channel's other work.
            match tokio::time::timeout(Duration::from_millis(1), self.socket.read(&mut chunk)).await
            {
                Err(_) => break,
                Ok(Ok(0)) => return Err("the peer closed the connection".into()),
                Ok(Ok(n)) => {
                    self.inbox.extend_from_slice(&chunk[..n]);
                    self.last_rx = tokio::time::Instant::now();
                    // Decode as we read (not after): a legitimate burst
                    // of valid frames must not look like garbage.
                    self.drain_frames(endian, status, events);
                }
                Ok(Err(e)) => return Err(e.to_string()),
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

                    // §3.1: the receiver skips a repeated progressive
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
                match cmd {
                    None | Some(ChannelCommand::Stop) => break,
                    Some(ChannelCommand::Send { peer, telegram }) => {
                        // Unicast by ID: a peer that has gone is a drop,
                        // never a delivery to whoever replaced it.
                        match peers.iter_mut().find(|p| p.id == peer) {
                            Some(p) => {
                                let prg = p.next_prg();
                                let mut frame = telegram.clone();
                                frame.envelope = envelope(prg);
                                match codec::encode(&frame, cfg.endian) {
                                    Ok(bytes) => {
                                        if let Err(e) = p.queue(bytes, &status) {
                                            set_error(&status, e.clone());
                                            let id = p.id;
                                            drop_peer(&mut peers, id, &e, &status, &events);
                                        }
                                    }
                                    Err(e) => set_error(&status, e.to_string()),
                                }
                            }
                            None => set_error(
                                &status,
                                format!("peer {peer} is gone; the reply was dropped"),
                            ),
                        }
                    }
                    Some(ChannelCommand::SendAll { telegram }) => {
                        let mut doomed = Vec::new();
                        for p in peers.iter_mut() {
                            let prg = p.next_prg();
                            let mut frame = telegram.clone();
                            frame.envelope = envelope(prg);
                            if let Ok(bytes) = codec::encode(&frame, cfg.endian)
                                && p.queue(bytes, &status).is_err() {
                                    doomed.push(p.id);
                                }
                        }
                        for id in doomed {
                            drop_peer(&mut peers, id, "not reading", &status, &events);
                        }
                    }
                }
            }
            Some((socket, addr)) = accepted => {
                let id = NEXT_PEER.fetch_add(1, Ordering::Relaxed);
                let mut peer = Peer::new(id, addr.clone(), socket);
                // §3 STEP 1: PRG 0 and a keep-alive on connect.
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
                    if let Err(e) = p.pump(cfg.endian, &status, &events).await {
                        doomed.push((p.id, e));
                        continue;
                    }
                    // §3.2: silence for the receive timeout means probe
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
                    if let Err(e) = p.flush(&status).await {
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
