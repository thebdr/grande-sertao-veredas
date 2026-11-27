//! The EtherNet/IP adapter: a device's assemblies served over EtherNet/IP, explicit messages on a
//! TCP session and Class 1 cyclic I/O over UDP, so a scanner meets its adapter on the bench before
//! it meets the plant.
//!
//! An [`EnipConfig`] (read from a bench file) gives the adapter's identity, its assemblies (runs
//! of bytes, by instance), the Class 1 connections it accepts (a configuration, a consumed and a
//! produced assembly, and what each direction's data carries), and named points in the
//! assemblies' bytes. [`start`] serves them on a thread and a runtime of its own: the
//! encapsulation's commands (ListIdentity on TCP and UDP, ListServices, ListInterfaces,
//! RegisterSession, UnRegisterSession, SendRRData, NOP), the Identity object, the Assembly object's
//! data, the Connection Manager's Forward_Open, Large_Forward_Open and Forward_Close (directly or
//! inside an Unconnected_Send), each refusal with the status the protocol gives it; and for every
//! connection open, the produced assembly sent at its RPI to the originator and the consumed one
//! taken from its frames, the connection closed when the originator falls silent past its
//! timeout. The handle reads and sets the assemblies on the plant side, by point or as bytes, and
//! holds the knobs a scanner's failure paths need.

mod config;
pub mod wire;

pub use config::*;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::Instant;

use wire::*;

/// How many exchanges the status keeps.
pub const RECENT: usize = 100;

/// Why the adapter did not start, a configuration was refused, or a plant-side call failed.
#[derive(Debug, thiserror::Error)]
pub enum EnipError {
    #[error("{0}")]
    Config(String),
    #[error("bind {addr}: {source}")]
    Bind { addr: String, source: std::io::Error },
    #[error("local_addr: {0}")]
    LocalAddr(std::io::Error),
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("spawn the adapter thread: {0}")]
    Thread(std::io::Error),
    #[error("the adapter thread died on startup")]
    DiedOnStartup,
    #[error("no point `{0}`")]
    NoPoint(String),
    #[error("no assembly {0}")]
    NoAssembly(u16),
    #[error("point `{point}` is {expected}, not {given}")]
    WrongType { point: String, expected: &'static str, given: &'static str },
    #[error("point `{point}`: {reason}")]
    BadValue { point: String, reason: String },
    #[error("assembly {instance} holds {size} byte(s), not {given}")]
    WrongSize { instance: u16, size: u16, given: usize },
}

/// A point's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
}

impl Value {
    pub fn ty(self) -> PointType {
        match self {
            Value::Bool(_) => PointType::Bool,
            Value::U8(_) => PointType::U8,
            Value::I8(_) => PointType::I8,
            Value::U16(_) => PointType::U16,
            Value::I16(_) => PointType::I16,
            Value::U32(_) => PointType::U32,
            Value::I32(_) => PointType::I32,
            Value::F32(_) => PointType::F32,
        }
    }
}

/// Parse a value as text into a point's type, as the Modbus server's points are parsed: empty is
/// the type's zero.
pub fn parse_value(ty: PointType, text: &str) -> Result<Value, String> {
    let t = text.trim();
    fn int<T: std::str::FromStr>(t: &str, label: &str) -> Result<T, String> {
        if t.is_empty() {
            "0".parse().map_err(|_| String::new())
        } else {
            t.parse().map_err(|_| format!("`{t}` is not a {label}"))
        }
    }
    Ok(match ty {
        PointType::Bool => Value::Bool(match t.to_ascii_lowercase().as_str() {
            "" | "false" | "0" | "off" | "no" => false,
            "true" | "1" | "on" | "yes" => true,
            _ => return Err(format!("`{t}` is not a bool")),
        }),
        PointType::U8 => Value::U8(int(t, "u8")?),
        PointType::I8 => Value::I8(int(t, "i8")?),
        PointType::U16 => Value::U16(int(t, "u16")?),
        PointType::I16 => Value::I16(int(t, "i16")?),
        PointType::U32 => Value::U32(int(t, "u32")?),
        PointType::I32 => Value::I32(int(t, "i32")?),
        PointType::F32 => {
            let v: f32 = if t.is_empty() { 0.0 } else { t.parse().map_err(|_| format!("`{t}` is not an f32"))? };
            if !v.is_finite() {
                return Err(format!("`{t}` is not a finite f32"));
            }
            Value::F32(v)
        }
    })
}

#[derive(Clone, Copy, Debug)]
struct Point {
    assembly: u16,
    byte: u16,
    bit: Option<u8>,
    ty: PointType,
}

fn put(bytes: &mut [u8], p: Point, v: Value) {
    let a = p.byte as usize;
    match v {
        Value::Bool(b) => {
            let mask = 1u8 << p.bit.unwrap_or(0);
            if b {
                bytes[a] |= mask;
            } else {
                bytes[a] &= !mask;
            }
        }
        Value::U8(x) => bytes[a] = x,
        Value::I8(x) => bytes[a] = x as u8,
        Value::U16(x) => bytes[a..a + 2].copy_from_slice(&x.to_le_bytes()),
        Value::I16(x) => bytes[a..a + 2].copy_from_slice(&x.to_le_bytes()),
        Value::U32(x) => bytes[a..a + 4].copy_from_slice(&x.to_le_bytes()),
        Value::I32(x) => bytes[a..a + 4].copy_from_slice(&x.to_le_bytes()),
        Value::F32(x) => bytes[a..a + 4].copy_from_slice(&x.to_le_bytes()),
    }
}

fn take(bytes: &[u8], p: Point) -> Value {
    let a = p.byte as usize;
    let b2 = || [bytes[a], bytes[a + 1]];
    let b4 = || [bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]];
    match p.ty {
        PointType::Bool => Value::Bool(bytes[a] & (1 << p.bit.unwrap_or(0)) != 0),
        PointType::U8 => Value::U8(bytes[a]),
        PointType::I8 => Value::I8(bytes[a] as i8),
        PointType::U16 => Value::U16(u16::from_le_bytes(b2())),
        PointType::I16 => Value::I16(i16::from_le_bytes(b2())),
        PointType::U32 => Value::U32(u32::from_le_bytes(b4())),
        PointType::I32 => Value::I32(i32::from_le_bytes(b4())),
        PointType::F32 => Value::F32(f32::from_le_bytes(b4())),
    }
}

/// One Class 1 connection, as the status shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub o_to_t_id: u32,
    pub t_to_o_id: u32,
    /// The originator's triple: connection serial, vendor, originator serial.
    pub serial: u16,
    pub vendor: u16,
    pub originator_serial: u32,
    pub config: u16,
    pub output: u16,
    pub input: u16,
    pub o_to_t_rpi_us: u32,
    pub t_to_o_rpi_us: u32,
    /// Where the produced frames go.
    pub sends_to: SocketAddr,
    /// Frames consumed (new data or not) and frames produced.
    pub consumed: u64,
    pub produced: u64,
    /// The last run/idle header consumed: `Some(true)` run, `Some(false)` idle, `None` before the
    /// first frame or on a modeless connection.
    pub run: Option<bool>,
}

/// What the adapter reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnipStatus {
    /// TCP connections open now, and sessions registered on them.
    pub tcp_connections: usize,
    pub sessions: usize,
    /// Encapsulation commands answered, by command.
    pub commands: BTreeMap<u16, u64>,
    /// CIP requests answered, by service and general status.
    pub requests: BTreeMap<(u8, u8), u64>,
    /// Forward_Opens refused, by extended status.
    pub refused_opens: BTreeMap<u16, u64>,
    /// Connections open now.
    pub connections: Vec<ConnectionStatus>,
    /// Connections closed by a Forward_Close, by their timeout, or by the knob.
    pub closed: u64,
    pub timed_out: u64,
    /// Class 1 frames that named no open connection, or came from another host than its
    /// originator, or could not be read.
    pub stray_frames: u64,
    /// Class 1 frames older than the last one taken on their connection.
    pub stale_frames: u64,
}

struct Conn {
    status: ConnectionStatus,
    output_header: DataHeader,
    input_header: DataHeader,
    /// When the originator's silence closes the connection.
    deadline: Instant,
    timeout: Duration,
    /// The last sequence count consumed, so a repeat carries no new data.
    last_count: Option<u16>,
    /// The last 32-bit sequence number taken, so a late frame is dropped.
    last_sequence: Option<u32>,
    next_produce: Instant,
    produce_sequence: u32,
    produce_count: u16,
    originator_ip: Ipv4Addr,
}

struct State {
    assemblies: BTreeMap<u16, Vec<u8>>,
    conns: Vec<Conn>,
    status: EnipStatus,
    next_session: u32,
    next_conn_id: u32,
    /// TCP connections' generation, as the Modbus server keeps it (`drop_sessions`).
    generation: u64,
    open: BTreeMap<u64, usize>,
}

struct Shared {
    state: Mutex<State>,
    cfg: EnipConfig,
    identity: (u8, u8),
    io_addr: Mutex<Option<SocketAddrV4>>,
    /// Wakes the Class 1 loop when a connection opens, so its first frame is not held until the
    /// loop's next due time.
    opened: tokio::sync::Notify,
    producing: AtomicBool,
    silent: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// The server proper follows: the TCP listener and its sessions, the UDP encapsulation socket, the
// Class 1 socket with its producer tick. (Drafted below once the wire layer is settled.)

/// A running adapter. Dropping it stops the adapter, closes every session and connection and
/// frees its ports.
pub struct EnipHandle {
    /// The encapsulation address (TCP, and UDP for ListIdentity), with the port it got.
    pub addr: SocketAddr,
    /// The Class 1 address, with the port it got.
    pub io_addr: SocketAddr,
    shared: Arc<Shared>,
    points: Vec<(String, Point)>,
    /// A restart asked of the accept loop, which answers with the generation it ended.
    restarts: tokio::sync::mpsc::UnboundedSender<std::sync::mpsc::Sender<u64>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A running adapter's points, apart from its handle.
#[derive(Clone)]
pub struct EnipPoints {
    shared: Arc<Shared>,
    points: Vec<(String, Point)>,
}

impl EnipPoints {
    fn point(&self, name: &str) -> Result<Point, EnipError> {
        self.points.iter().find(|(n, _)| n == name).map(|(_, p)| *p).ok_or_else(|| EnipError::NoPoint(name.to_string()))
    }

    /// A point's value.
    pub fn get(&self, point: &str) -> Result<Value, EnipError> {
        let p = self.point(point)?;
        let state = self.shared.lock();
        Ok(take(&state.assemblies[&p.assembly], p))
    }

    /// Sets a point, of its own type.
    pub fn set_value(&self, point: &str, value: Value) -> Result<(), EnipError> {
        let p = self.point(point)?;
        if value.ty() != p.ty {
            return Err(EnipError::WrongType { point: point.to_string(), expected: p.ty.label(), given: value.ty().label() });
        }
        let mut state = self.shared.lock();
        if let Some(bytes) = state.assemblies.get_mut(&p.assembly) {
            put(bytes, p, value);
        }
        Ok(())
    }

    /// A point's type.
    pub fn ty(&self, point: &str) -> Result<PointType, EnipError> {
        Ok(self.point(point)?.ty)
    }
}

impl std::fmt::Debug for EnipHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnipHandle").field("addr", &self.addr).field("io_addr", &self.io_addr).finish()
    }
}

impl EnipHandle {
    fn point(&self, name: &str) -> Result<Point, EnipError> {
        self.points.iter().find(|(n, _)| n == name).map(|(_, p)| *p).ok_or_else(|| EnipError::NoPoint(name.to_string()))
    }

    pub fn point_names(&self) -> Vec<String> {
        self.points.iter().map(|(n, _)| n.clone()).collect()
    }

    /// The points, readable and settable from another thread while the adapter runs (reflexo's
    /// models).
    pub fn points(&self) -> EnipPoints {
        EnipPoints { shared: Arc::clone(&self.shared), points: self.points.clone() }
    }

    pub fn get(&self, point: &str) -> Result<Value, EnipError> {
        let p = self.point(point)?;
        let state = self.shared.lock();
        Ok(take(&state.assemblies[&p.assembly], p))
    }

    pub fn set_value(&self, point: &str, value: Value) -> Result<(), EnipError> {
        let p = self.point(point)?;
        if value.ty() != p.ty {
            return Err(EnipError::WrongType { point: point.to_string(), expected: p.ty.label(), given: value.ty().label() });
        }
        let mut state = self.shared.lock();
        if let Some(bytes) = state.assemblies.get_mut(&p.assembly) {
            put(bytes, p, value);
        }
        Ok(())
    }

    pub fn set(&self, point: &str, text: &str) -> Result<(), EnipError> {
        let p = self.point(point)?;
        let value = parse_value(p.ty, text).map_err(|reason| EnipError::BadValue { point: point.to_string(), reason })?;
        self.set_value(point, value)
    }

    pub fn snapshot(&self) -> Vec<(String, Value)> {
        let state = self.shared.lock();
        self.points.iter().map(|(n, p)| (n.clone(), take(&state.assemblies[&p.assembly], *p))).collect()
    }

    /// An assembly's bytes, read on the plant side.
    pub fn assembly(&self, instance: u16) -> Result<Vec<u8>, EnipError> {
        self.shared.lock().assemblies.get(&instance).cloned().ok_or(EnipError::NoAssembly(instance))
    }

    /// Set an assembly's bytes from the plant side, all of them.
    pub fn set_assembly(&self, instance: u16, bytes: &[u8]) -> Result<(), EnipError> {
        let mut state = self.shared.lock();
        let a = state.assemblies.get_mut(&instance).ok_or(EnipError::NoAssembly(instance))?;
        if a.len() != bytes.len() {
            return Err(EnipError::WrongSize { instance, size: a.len() as u16, given: bytes.len() });
        }
        a.copy_from_slice(bytes);
        Ok(())
    }

    pub fn status(&self) -> EnipStatus {
        let state = self.shared.lock();
        let mut s = state.status.clone();
        s.connections = state.conns.iter().map(|c| c.status.clone()).collect();
        s
    }

    /// Produce nothing (false) on every open connection, as a device whose producer stalled: each
    /// originator sees its inputs fall silent; true produces again on each one's old phase.
    pub fn set_producing(&self, producing: bool) {
        self.shared.producing.store(producing, Ordering::SeqCst);
    }

    /// Answer no encapsulation request (true), a session's connection up; Class 1 runs on.
    pub fn set_silent(&self, silent: bool) {
        self.shared.silent.store(silent, Ordering::SeqCst);
    }

    /// Close every Class 1 connection and every TCP connection, as a device's restart does: those
    /// its sessions serve and those still in the listen queue (a client whose connect returned
    /// before the call); the adapter listens on. Returns once every TCP connection open at the call
    /// is closed (or 5 s).
    pub fn restart(&self) {
        let started = std::time::Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        if self.restarts.send(tx).is_err() {
            return;
        }
        let Ok(ended) = rx.recv_timeout(Duration::from_secs(5)) else { return };
        while started.elapsed() < Duration::from_secs(5) {
            if self.shared.lock().open.range(..ended).all(|(_, n)| *n == 0) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for EnipHandle {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

/// Validate a configuration before anything starts, naming what is wrong.
pub fn validate(cfg: &EnipConfig) -> Result<(), EnipError> {
    config::check(cfg)
}

/// Start the adapter. Blocks until it listens (or failed).
pub fn start(cfg: &EnipConfig) -> Result<EnipHandle, EnipError> {
    config::check(cfg)?;
    let identity = cfg.identity.revision_bytes().map_err(EnipError::Config)?;
    let mut assemblies: BTreeMap<u16, Vec<u8>> = cfg.assemblies.iter().map(|a| (a.instance, vec![0u8; a.size as usize])).collect();
    let mut points = Vec::new();
    for p in &cfg.points {
        let ty = p.ty.unwrap_or(if p.bit.is_some() { PointType::Bool } else { PointType::U8 });
        let point = Point { assembly: p.assembly, byte: p.byte, bit: p.bit, ty };
        let value = parse_value(ty, &p.value).map_err(EnipError::Config)?;
        if let Some(bytes) = assemblies.get_mut(&p.assembly) {
            put(bytes, point, value);
        }
        points.push((p.name.clone(), point));
    }
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            assemblies,
            conns: Vec::new(),
            status: EnipStatus::default(),
            next_session: 1,
            next_conn_id: 0x4000_0001,
            generation: 0,
            open: BTreeMap::new(),
        }),
        cfg: cfg.clone(),
        identity,
        io_addr: Mutex::new(None),
        opened: tokio::sync::Notify::new(),
        producing: AtomicBool::new(true),
        silent: AtomicBool::new(false),
    });
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(SocketAddr, SocketAddr), EnipError>>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (drops_tx, _) = tokio::sync::watch::channel(0u64);
    let (restarts, restarts_rx) = tokio::sync::mpsc::unbounded_channel::<std::sync::mpsc::Sender<u64>>();
    let server = Arc::clone(&shared);
    let thread = std::thread::Builder::new()
        .name("maquinar-enip".into())
        .spawn(move || {
            let _timer = crate::timer::FineTimer::hold();
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(EnipError::Runtime(e)));
                    return;
                }
            };
            rt.block_on(async move {
                let bind = server.cfg.bind.trim().to_string();
                let bound = async {
                    // Port 0: the machine's choice for TCP may be taken for UDP by another socket;
                    // choose again till one number is free for both.
                    let mut tries = 0;
                    let (tcp, addr, udp) = loop {
                        let tcp = TcpListener::bind((bind.as_str(), server.cfg.port))
                            .await
                            .map_err(|source| EnipError::Bind { addr: format!("{bind}:{} (tcp)", server.cfg.port), source })?;
                        let addr = tcp.local_addr().map_err(EnipError::LocalAddr)?;
                        match UdpSocket::bind(addr).await {
                            Ok(udp) => break (tcp, addr, udp),
                            Err(e) if server.cfg.port == 0 && e.kind() == std::io::ErrorKind::AddrInUse && tries < 64 => tries += 1,
                            Err(source) => return Err(EnipError::Bind { addr: format!("{addr} (udp)"), source }),
                        }
                    };
                    let io = UdpSocket::bind((bind.as_str(), server.cfg.io_port))
                        .await
                        .map_err(|source| EnipError::Bind { addr: format!("{bind}:{} (udp, class 1)", server.cfg.io_port), source })?;
                    let io_addr = io.local_addr().map_err(EnipError::LocalAddr)?;
                    Ok::<_, EnipError>((tcp, udp, io, addr, io_addr))
                }
                .await;
                let (tcp, udp, io, addr, io_addr) = match bound {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if let SocketAddr::V4(v4) = io_addr {
                    *server.io_addr.lock().unwrap_or_else(|e| e.into_inner()) = Some(v4);
                }
                if ready_tx.send(Ok((addr, io_addr))).is_err() {
                    return;
                }
                let io = Arc::new(io);
                tokio::spawn(list_identity_udp(udp, Arc::clone(&server)));
                tokio::spawn(class1(io, Arc::clone(&server)));
                accept_loop(tcp, server, drops_tx, restarts_rx, shutdown_rx).await;
            });
        })
        .map_err(EnipError::Thread)?;
    let (addr, io_addr) = ready_rx.recv().map_err(|_| EnipError::DiedOnStartup)??;
    Ok(EnipHandle { addr, io_addr, shared, points, restarts, shutdown: Some(shutdown_tx), thread: Some(thread) })
}

/// The ListIdentity item: the encapsulation version, the adapter's socket address, its identity,
/// its state.
fn identity_item(shared: &Shared, addr: SocketAddr) -> Item {
    let id = &shared.cfg.identity;
    let mut d = Vec::new();
    d.extend_from_slice(&1u16.to_le_bytes());
    let v4 = match addr {
        SocketAddr::V4(v) => v,
        SocketAddr::V6(_) => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, addr.port()),
    };
    d.extend_from_slice(&sockaddr(v4));
    d.extend_from_slice(&id.vendor_id.to_le_bytes());
    d.extend_from_slice(&id.device_type.to_le_bytes());
    d.extend_from_slice(&id.product_code.to_le_bytes());
    d.push(shared.identity.0);
    d.push(shared.identity.1);
    d.extend_from_slice(&identity_status(shared).to_le_bytes());
    d.extend_from_slice(&id.serial.to_le_bytes());
    d.push(id.product_name.len() as u8);
    d.extend_from_slice(id.product_name.as_bytes());
    // State: 3, operational.
    d.push(3);
    Item { kind: item::LIST_IDENTITY, data: d }
}

/// The Identity object's status word: bit 0 owned while a connection is open.
fn identity_status(shared: &Shared) -> u16 {
    if shared.lock().conns.is_empty() { 0x0000 } else { 0x0001 }
}

/// The local address a datagram to `to` leaves from: what an adapter bound to the any-address
/// states in its identity item.
fn leaving_address(to: SocketAddr) -> Option<std::net::IpAddr> {
    let probe = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    probe.connect(to).ok()?;
    probe.local_addr().ok().map(|a| a.ip())
}

async fn list_identity_udp(udp: UdpSocket, shared: Arc<Shared>) {
    let mut buf = vec![0u8; 1500];
    let own = udp.local_addr().ok();
    loop {
        let Ok((n, from)) = udp.recv_from(&mut buf).await else { continue };
        if n < HEADER {
            continue;
        }
        let Ok(h) = <[u8; HEADER]>::try_from(&buf[..HEADER]) else { continue };
        let h = Header::parse(&h);
        // A request alone: one carries no data, an answer its identity item. Answering an answer
        // set two adapters answering each other for ever, and one its own (round 8). A header
        // whose status or options are not 0 is ignored (Vol 2 2-3.3, 2-3.5): an error answer has
        // no data either, and answering it looped with a peer that answers so (round 9).
        if h.command != command::LIST_IDENTITY || h.length != 0 || h.status != 0 || h.options != 0 || shared.silent.load(Ordering::SeqCst) {
            continue;
        }
        let Some(own) = own else { continue };
        let own = if own.ip().is_unspecified() { leaving_address(from).map(|ip| SocketAddr::new(ip, own.port())).unwrap_or(own) } else { own };
        let body = write_items(&[identity_item(&shared, own)]);
        *shared.lock().status.commands.entry(h.command).or_default() += 1;
        let _ = udp.send_to(&h.reply(0, encap_status::SUCCESS, &body), from).await;
    }
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    drops: tokio::sync::watch::Sender<u64>,
    mut restarts: tokio::sync::mpsc::UnboundedReceiver<std::sync::mpsc::Sender<u64>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => return,
            Some(reply) = restarts.recv() => {
                // The kernel completes a connection before this loop takes it: a client whose
                // connect returned before the restart may still sit in the listen queue. Take every
                // one there now, straight from the socket, and close it with the rest.
                while let Ok((socket, _)) = socket2::SockRef::from(&listener).accept() {
                    drop(socket);
                }
                let ended = {
                    let mut state = shared.lock();
                    let n = state.conns.len() as u64;
                    state.conns.clear();
                    state.status.closed += n;
                    state.generation += 1;
                    state.generation
                };
                drops.send_modify(|g| *g = ended);
                let _ = reply.send(ended);
            }
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                let generation = {
                    let mut state = shared.lock();
                    state.status.tcp_connections += 1;
                    let g = state.generation;
                    *state.open.entry(g).or_default() += 1;
                    g
                };
                let gone = drops.subscribe();
                tokio::spawn(session(stream, peer, Arc::clone(&shared), gone, generation));
            }
        }
    }
}

/// Counts a TCP connection and its session out when its task ends.
struct Open {
    shared: Arc<Shared>,
    generation: u64,
    registered: bool,
}

impl Drop for Open {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.status.tcp_connections = state.status.tcp_connections.saturating_sub(1);
        if self.registered {
            state.status.sessions = state.status.sessions.saturating_sub(1);
        }
        if let Some(n) = state.open.get_mut(&self.generation) {
            *n = n.saturating_sub(1);
        }
    }
}

async fn ended(gone: &mut tokio::sync::watch::Receiver<u64>, generation: u64) {
    if gone.wait_for(|g| *g > generation).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// The most an encapsulation body may carry: what its 16-bit length allows less the header.
const MAX_BODY: usize = 65_511;

async fn session(stream: TcpStream, peer: SocketAddr, shared: Arc<Shared>, mut gone: tokio::sync::watch::Receiver<u64>, generation: u64) {
    let mut open = Open { shared: Arc::clone(&shared), generation, registered: false };
    let _ = stream.set_nodelay(true);
    let local = stream.local_addr().ok();
    let (mut rd, mut wr) = stream.into_split();
    let mut handle: u32 = 0;
    loop {
        let mut h = [0u8; HEADER];
        tokio::select! {
            r = rd.read_exact(&mut h) => if r.is_err() { return },
            _ = ended(&mut gone, generation) => return,
        }
        let header = Header::parse(&h);
        if header.length as usize > MAX_BODY {
            return;
        }
        let mut body = vec![0u8; header.length as usize];
        tokio::select! {
            r = rd.read_exact(&mut body) => if r.is_err() { return },
            _ = ended(&mut gone, generation) => return,
        }
        // A request whose status or options are not 0 is ignored, no reply (Vol 2 2-3.3, 2-3.5).
        if shared.silent.load(Ordering::SeqCst) || header.status != 0 || header.options != 0 {
            continue;
        }
        *shared.lock().status.commands.entry(header.command).or_default() += 1;
        let reply: Option<Vec<u8>> = match header.command {
            command::NOP => None,
            command::LIST_IDENTITY => {
                let own = local.unwrap_or(peer);
                Some(header.reply(0, encap_status::SUCCESS, &write_items(&[identity_item(&shared, own)])))
            }
            command::LIST_SERVICES => {
                let mut d = Vec::new();
                d.extend_from_slice(&1u16.to_le_bytes());
                // CIP over TCP (bit 5) and Class 0/1 over UDP (bit 8).
                d.extend_from_slice(&0x0120u16.to_le_bytes());
                let mut name = *b"Communications\0\0";
                name[14] = 0;
                d.extend_from_slice(&name);
                Some(header.reply(0, encap_status::SUCCESS, &write_items(&[Item { kind: item::LIST_SERVICES, data: d }])))
            }
            command::LIST_INTERFACES => Some(header.reply(0, encap_status::SUCCESS, &write_items(&[]))),
            command::REGISTER_SESSION => {
                if body.len() != 4 {
                    Some(header.reply(0, encap_status::INVALID_LENGTH, &body))
                } else if le16(&body, 0) != 1 {
                    // The version this adapter speaks, in the refusal.
                    Some(header.reply(0, encap_status::UNSUPPORTED_PROTOCOL, &[1, 0, 0, 0]))
                } else if handle != 0 {
                    Some(header.reply(handle, encap_status::INVALID_COMMAND, &body))
                } else {
                    let mut state = shared.lock();
                    if shared.cfg.max_sessions.is_some_and(|m| state.status.sessions >= m) {
                        Some(header.reply(0, encap_status::INSUFFICIENT_MEMORY, &body))
                    } else {
                        handle = state.next_session;
                        state.next_session = state.next_session.wrapping_add(1).max(1);
                        state.status.sessions += 1;
                        open.registered = true;
                        Some(header.reply(handle, encap_status::SUCCESS, &body))
                    }
                }
            }
            command::UNREGISTER_SESSION => return,
            command::SEND_RR_DATA => {
                if handle == 0 || header.session != handle {
                    Some(header.reply(header.session, encap_status::INVALID_SESSION, &[]))
                } else {
                    match rr_data(&shared, peer, generation, &body) {
                        Some(out) => Some(header.reply(handle, encap_status::SUCCESS, &out)),
                        None => Some(header.reply(handle, encap_status::INCORRECT_DATA, &[])),
                    }
                }
            }
            // Connected explicit messaging is not served: the command is not taken.
            _ => Some(header.reply(header.session, encap_status::INVALID_COMMAND, &[])),
        };
        if let Some(reply) = reply {
            // A client that does not read leaves the write waiting: a restart does not wait for it.
            let written = tokio::select! {
                r = wr.write_all(&reply) => r.is_ok(),
                _ = ended(&mut gone, generation) => false,
            };
            if !written {
                return;
            }
        }
    }
}

/// A SendRRData body answered: the interface handle and timeout, then the request's items read
/// (a null address and unconnected data), the request answered, the reply's items written.
fn rr_data(shared: &Shared, peer: SocketAddr, generation: u64, body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 6 {
        return None;
    }
    let items = parse_items(&body[6..])?;
    let data = items.iter().find(|i| i.kind == item::UNCONNECTED_DATA)?;
    if !items.iter().any(|i| i.kind == item::NULL_ADDRESS && i.data.is_empty()) {
        return None;
    }
    let t_to_o = items.iter().find(|i| i.kind == item::SOCKADDR_T_TO_O).and_then(|i| read_sockaddr(&i.data));
    let (reply, extra) = cip(shared, peer, generation, &data.data, t_to_o);
    let mut out = vec![0u8; 6];
    let mut items = vec![Item { kind: item::NULL_ADDRESS, data: Vec::new() }, Item { kind: item::UNCONNECTED_DATA, data: reply }];
    items.extend(extra);
    out.extend(write_items(&items));
    Some(out)
}

fn count(shared: &Shared, service: u8, general: u8) {
    *shared.lock().status.requests.entry((service, general)).or_default() += 1;
}

/// A CIP request answered: the reply frame, and any items the reply carries besides.
fn cip(shared: &Shared, peer: SocketAddr, generation: u64, frame: &[u8], t_to_o: Option<SocketAddrV4>) -> (Vec<u8>, Vec<Item>) {
    let Some(req) = parse_request(frame) else {
        let service = frame.first().copied().unwrap_or(0);
        count(shared, service, general::NOT_ENOUGH_DATA);
        return (reply(service, general::NOT_ENOUGH_DATA, &[], &[]), Vec::new());
    };
    // A request's own path names an object by logical segments alone: a port, a key or data there
    // is a segment it cannot read, refused at its word (as OpENer does).
    let path = match parse_path(&req.path) {
        Ok(p) if p.first_other.is_none() => p,
        Ok(Path { first_other: Some(at), .. }) | Err(BadSegment(at)) => {
            count(shared, req.service, general::PATH_SEGMENT_ERROR);
            return (reply(req.service, general::PATH_SEGMENT_ERROR, &[(at / 2) as u16], &[]), Vec::new());
        }
        Ok(_) => unreachable!("a path with no other segment is taken above"),
    };
    let (general_status, additional, data, extra): Answer = match (path.class, path.instance) {
        (Some(class::CONNECTION_MANAGER), Some(1)) => connection_manager(shared, peer, generation, &req, t_to_o),
        (Some(class::IDENTITY), Some(i)) => identity(shared, &req, i, path.attribute),
        (Some(class::ASSEMBLY), Some(i)) => assembly(shared, &req, i, path.attribute),
        _ => (general::PATH_DESTINATION_UNKNOWN, Vec::new(), Vec::new(), Vec::new()),
    };
    if req.service == service::UNCONNECTED_SEND && general_status == general::SUCCESS {
        // An Unconnected_Send's answer is the embedded request's own reply, already framed and
        // counted under its own service.
        return (data, extra);
    }
    count(shared, req.service, general_status);
    (reply(req.service, general_status, &additional, &data), extra)
}

/// What a handler answers: the general status, the additional status words, the data, the
/// reply's extra items.
type Answer = (u8, Vec<u16>, Vec<u8>, Vec<Item>);

fn status_only(general: u8) -> Answer {
    (general, Vec::new(), Vec::new(), Vec::new())
}

/// The Identity object: instance 1 its attributes 1 to 7, the class (instance 0) its revision
/// and its highest instance.
fn identity(shared: &Shared, req: &Request, instance: u16, attribute: Option<u16>) -> Answer {
    let id = &shared.cfg.identity;
    let attr = |n: u16| -> Option<Vec<u8>> {
        Some(match n {
            1 => id.vendor_id.to_le_bytes().to_vec(),
            2 => id.device_type.to_le_bytes().to_vec(),
            3 => id.product_code.to_le_bytes().to_vec(),
            4 => vec![shared.identity.0, shared.identity.1],
            5 => identity_status(shared).to_le_bytes().to_vec(),
            6 => id.serial.to_le_bytes().to_vec(),
            7 => {
                let mut v = vec![id.product_name.len() as u8];
                v.extend_from_slice(id.product_name.as_bytes());
                v
            }
            _ => return None,
        })
    };
    match (instance, req.service, attribute) {
        (0, service::GET_ATTRIBUTE_SINGLE, Some(1 | 2)) => (general::SUCCESS, Vec::new(), 1u16.to_le_bytes().to_vec(), Vec::new()),
        (0, service::GET_ATTRIBUTE_SINGLE, _) => status_only(general::ATTRIBUTE_NOT_SUPPORTED),
        (1, service::GET_ATTRIBUTES_ALL, None) => {
            let mut all = Vec::new();
            for n in 1..=7 {
                all.extend(attr(n).unwrap_or_default());
            }
            (general::SUCCESS, Vec::new(), all, Vec::new())
        }
        (1, service::GET_ATTRIBUTE_SINGLE, Some(n)) => match attr(n) {
            Some(v) => (general::SUCCESS, Vec::new(), v, Vec::new()),
            None => status_only(general::ATTRIBUTE_NOT_SUPPORTED),
        },
        (1, service::SET_ATTRIBUTE_SINGLE, Some(n)) if attr(n).is_some() => status_only(general::ATTRIBUTE_NOT_SETTABLE),
        (1, service::SET_ATTRIBUTE_SINGLE, Some(_)) => status_only(general::ATTRIBUTE_NOT_SUPPORTED),
        (0 | 1, _, _) => status_only(general::SERVICE_NOT_SUPPORTED),
        _ => status_only(general::PATH_DESTINATION_UNKNOWN),
    }
}

/// The Assembly object: an instance's data (attribute 3) read and, unless a connection produces it
/// or owns it, written whole; its size (attribute 4) read.
fn assembly(shared: &Shared, req: &Request, instance: u16, attribute: Option<u16>) -> Answer {
    let mut state = shared.lock();
    let Some(size) = state.assemblies.get(&instance).map(Vec::len) else {
        return status_only(general::PATH_DESTINATION_UNKNOWN);
    };
    match (req.service, attribute) {
        (service::GET_ATTRIBUTE_SINGLE, Some(3)) => (general::SUCCESS, Vec::new(), state.assemblies[&instance].clone(), Vec::new()),
        (service::GET_ATTRIBUTE_SINGLE, Some(4)) => (general::SUCCESS, Vec::new(), (size as u16).to_le_bytes().to_vec(), Vec::new()),
        (service::GET_ATTRIBUTE_SINGLE, _) => status_only(general::ATTRIBUTE_NOT_SUPPORTED),
        (service::SET_ATTRIBUTE_SINGLE, Some(3)) => {
            if shared.cfg.connections.iter().any(|c| c.input == instance) {
                return status_only(general::ATTRIBUTE_NOT_SETTABLE);
            }
            if state.conns.iter().any(|c| c.status.output == instance) {
                return status_only(general::OBJECT_STATE_CONFLICT);
            }
            match req.data.len().cmp(&size) {
                std::cmp::Ordering::Less => status_only(general::NOT_ENOUGH_DATA),
                std::cmp::Ordering::Greater => status_only(general::TOO_MUCH_DATA),
                std::cmp::Ordering::Equal => {
                    if let Some(a) = state.assemblies.get_mut(&instance) {
                        a.copy_from_slice(&req.data);
                    }
                    status_only(general::SUCCESS)
                }
            }
        }
        (service::SET_ATTRIBUTE_SINGLE, Some(4)) => status_only(general::ATTRIBUTE_NOT_SETTABLE),
        (service::SET_ATTRIBUTE_SINGLE, _) => status_only(general::ATTRIBUTE_NOT_SUPPORTED),
        _ => status_only(general::SERVICE_NOT_SUPPORTED),
    }
}

/// The Connection Manager: Forward_Open, Large_Forward_Open, Forward_Close, and an
/// Unconnected_Send whose route ends here.
fn connection_manager(shared: &Shared, peer: SocketAddr, generation: u64, req: &Request, t_to_o: Option<SocketAddrV4>) -> Answer {
    match req.service {
        service::FORWARD_OPEN => forward_open(shared, peer, generation, &req.data, false, t_to_o),
        service::LARGE_FORWARD_OPEN => forward_open(shared, peer, generation, &req.data, true, t_to_o),
        service::FORWARD_CLOSE => forward_close(shared, &req.data),
        service::UNCONNECTED_SEND => unconnected_send(shared, peer, generation, &req.data, t_to_o),
        _ => status_only(general::SERVICE_NOT_SUPPORTED),
    }
}

/// An Unconnected_Send: its embedded request answered here when the route is empty or names this
/// adapter's own backplane slot 0 (port 1, link 0); any other route cannot be followed.
fn unconnected_send(shared: &Shared, peer: SocketAddr, generation: u64, data: &[u8], t_to_o: Option<SocketAddrV4>) -> Answer {
    if data.len() < 4 {
        return status_only(general::NOT_ENOUGH_DATA);
    }
    let size = le16(data, 2) as usize;
    let pad = size % 2;
    if data.len() < 4 + size + pad + 2 {
        return status_only(general::NOT_ENOUGH_DATA);
    }
    let message = &data[4..4 + size];
    let at = 4 + size + pad;
    let route_words = data[at] as usize;
    let route = &data[at + 2..];
    if route.len() != 2 * route_words {
        return status_only(general::NOT_ENOUGH_DATA);
    }
    let here = route.is_empty() || route == [0x01, 0x00];
    if !here {
        // 0x0312: the link address is not valid here: an adapter routes nowhere. A routing error
        // carries the route's remaining size in words, as the router that met it saw it (all of
        // it: the first hop), then a reserved byte.
        return (general::CONNECTION_FAILURE, vec![0x0312], vec![route_words as u8, 0], Vec::new());
    }
    // An Unconnected_Send wrapped in another goes nowhere either: the adapter routes one level
    // and no more, so no request can nest the stack away.
    if message.first() == Some(&service::UNCONNECTED_SEND) {
        return status_only(general::SERVICE_NOT_SUPPORTED);
    }
    let (reply, extra) = cip(shared, peer, generation, message, t_to_o);
    (general::SUCCESS, Vec::new(), reply, extra)
}

/// A Forward_Open's refusal: the general status 01, its extended status (and the size or the RPI
/// it wanted, for a size or an RPI refused), the originator's triple, the remaining path size.
fn refuse_open(shared: &Shared, fo_triple: (u16, u16, u32), extended: u16, more: Option<u16>) -> Answer {
    *shared.lock().status.refused_opens.entry(extended).or_default() += 1;
    let mut data = Vec::new();
    data.extend_from_slice(&fo_triple.0.to_le_bytes());
    data.extend_from_slice(&fo_triple.1.to_le_bytes());
    data.extend_from_slice(&fo_triple.2.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    let mut additional = vec![extended];
    additional.extend(more);
    (general::CONNECTION_FAILURE, additional, data, Vec::new())
}

/// The size a direction's connection parameters must state: the data, the 16-bit sequence count,
/// and a run/idle header when the direction carries one.
fn wire_size(data: usize, header: DataHeader) -> usize {
    data + 2 + if header == DataHeader::RunIdle { 4 } else { 0 }
}

/// A connection's timeout: its consumed RPI times 4 shifted by the multiplier.
fn timeout_of(rpi_us: u32, multiplier: u8) -> Duration {
    Duration::from_micros(rpi_us as u64 * (4u64 << multiplier.min(7)))
}

/// The first frame may come this much later than the timeout after the open: an originator starts
/// producing once its open is answered, and a slow one's first frame is not a silence.
const FIRST_FRAME_GRACE: Duration = Duration::from_secs(10);

fn forward_open(shared: &Shared, peer: SocketAddr, generation: u64, body: &[u8], large: bool, t_to_o: Option<SocketAddrV4>) -> Answer {
    let Some(fo) = parse_forward_open(body, large) else {
        return status_only(general::NOT_ENOUGH_DATA);
    };
    let triple = (fo.serial, fo.vendor, fo.originator_serial);
    if shared.lock().conns.iter().any(|c| (c.status.serial, c.status.vendor, c.status.originator_serial) == triple) {
        return refuse_open(shared, triple, extended::CONNECTION_IN_USE, None);
    }
    // Class 1 (the low nibble), cyclic (trigger 0); the direction bit is the client's (0).
    if fo.transport & 0x0F != 1 || (fo.transport >> 4) & 0x07 != 0 || fo.transport & 0x80 != 0 {
        return refuse_open(shared, triple, extended::TRANSPORT_NOT_SUPPORTED, None);
    }
    if fo.o_to_t.kind != 2 || fo.t_to_o.kind != 2 {
        return refuse_open(shared, triple, extended::INVALID_CONNECTION_TYPE, None);
    }
    let path = match parse_path(&fo.path) {
        Ok(p) => p,
        Err(_) => return refuse_open(shared, triple, extended::INVALID_SEGMENT_IN_PATH, None),
    };
    if let Some((vendor, device_type, product, major, minor)) = path.key {
        let id = &shared.cfg.identity;
        let compatible = major & 0x80 != 0;
        let major = major & 0x7F;
        if (vendor != 0 && vendor != id.vendor_id) || (product != 0 && product != id.product_code) {
            return refuse_open(shared, triple, extended::VENDOR_OR_PRODUCT_MISMATCH, None);
        }
        if device_type != 0 && device_type != id.device_type {
            return refuse_open(shared, triple, extended::DEVICE_TYPE_MISMATCH, None);
        }
        let (own_major, own_minor) = shared.identity;
        let revision_ok = major == 0
            || (major == own_major && (minor == 0 || if compatible { minor <= own_minor } else { minor == own_minor }));
        if !revision_ok {
            return refuse_open(shared, triple, extended::REVISION_MISMATCH, None);
        }
    }
    if path.class != Some(class::ASSEMBLY) || path.points.len() != 2 || !path.ports.is_empty() {
        return refuse_open(shared, triple, extended::INVALID_SEGMENT_IN_PATH, None);
    }
    let config_instance = path.instance.unwrap_or(0);
    let (output, input) = (path.points[0], path.points[1]);
    let cfg = &shared.cfg;
    if !cfg.connections.iter().any(|c| c.config == config_instance) {
        return refuse_open(shared, triple, extended::INVALID_CONFIGURATION_PATH, None);
    }
    if !cfg.connections.iter().any(|c| c.config == config_instance && c.output == output) {
        return refuse_open(shared, triple, extended::INVALID_CONSUMING_PATH, None);
    }
    let Some(conn) = cfg.connections.iter().find(|c| c.config == config_instance && c.output == output && c.input == input) else {
        return refuse_open(shared, triple, extended::INVALID_PRODUCING_PATH, None);
    };
    let size_of = |n: u16| cfg.assemblies.iter().find(|a| a.instance == n).map(|a| a.size as usize).unwrap_or(0);
    if let Some(data) = &path.data {
        // The data segment carries whole words: an odd-sized configuration ends in a pad byte.
        let want = size_of(conn.config);
        if !(data.len() == want || data.len() == want + 1 && want % 2 == 1) {
            return refuse_open(shared, triple, extended::INVALID_CONFIGURATION_SIZE, Some(want as u16));
        }
    }
    let output_header = conn.output_header.unwrap_or(DataHeader::RunIdle);
    let input_header = conn.input_header.unwrap_or(DataHeader::Modeless);
    let o_size = wire_size(size_of(output), output_header);
    let t_size = wire_size(size_of(input), input_header);
    if fo.o_to_t.size as usize != o_size || (!large && o_size > 511) {
        return refuse_open(shared, triple, extended::INVALID_O_TO_T_SIZE, Some(o_size as u16));
    }
    if fo.t_to_o.size as usize != t_size || (!large && t_size > 511) {
        return refuse_open(shared, triple, extended::INVALID_T_TO_O_SIZE, Some(t_size as u16));
    }
    let rpi_ok = |r: u32| r >= cfg.rpi_min_us && r <= cfg.rpi_max_us;
    if !rpi_ok(fo.o_to_t_rpi) || !rpi_ok(fo.t_to_o_rpi) {
        return refuse_open(shared, triple, extended::RPI_NOT_SUPPORTED, None);
    }
    let originator_ip = match peer {
        SocketAddr::V4(v) => *v.ip(),
        SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
    };
    // Where the produced frames go: the originator's own host (the session's), at the port its T→O
    // socket-address item gives, else at the standard I/O port. The item's address is not obeyed:
    // a session cannot send another host the adapter's frames (OpENer takes the port alone too).
    // A port of 0 names none: the standard port then too, as a scanner reads a 0 in the reply's.
    let sends_to = SocketAddr::V4(SocketAddrV4::new(originator_ip, t_to_o.map(|a| a.port()).filter(|p| *p != 0).unwrap_or(DEFAULT_IO_PORT)));
    let mut state = shared.lock();
    // A session a restart has ended (its generation past) opens nothing: the restart cleared the
    // connections under this lock, and one opened after it would outlive the device's restart.
    if state.generation != generation {
        drop(state);
        return refuse_open(shared, triple, extended::NO_MORE_CONNECTIONS, None);
    }
    if state.conns.iter().any(|c| c.status.output == output) {
        drop(state);
        return refuse_open(shared, triple, extended::OWNERSHIP_CONFLICT, None);
    }
    if cfg.max_connections.is_some_and(|m| state.conns.len() >= m) {
        drop(state);
        return refuse_open(shared, triple, extended::NO_MORE_CONNECTIONS, None);
    }
    if let Some(data) = &path.data {
        let want = size_of(conn.config);
        if let Some(a) = state.assemblies.get_mut(&conn.config) {
            a.copy_from_slice(&data[..want]);
        }
    }
    let o_to_t_id = state.next_conn_id;
    state.next_conn_id = state.next_conn_id.wrapping_add(1).max(1);
    let now = Instant::now();
    let timeout = timeout_of(fo.o_to_t_rpi, fo.timeout_multiplier);
    let status = ConnectionStatus {
        o_to_t_id,
        t_to_o_id: fo.t_to_o_id,
        serial: fo.serial,
        vendor: fo.vendor,
        originator_serial: fo.originator_serial,
        config: conn.config,
        output,
        input,
        o_to_t_rpi_us: fo.o_to_t_rpi,
        t_to_o_rpi_us: fo.t_to_o_rpi,
        sends_to,
        consumed: 0,
        produced: 0,
        run: None,
    };
    state.conns.push(Conn {
        status,
        output_header,
        input_header,
        deadline: now + timeout.max(FIRST_FRAME_GRACE),
        timeout,
        last_count: None,
        last_sequence: None,
        next_produce: now,
        produce_sequence: 0,
        produce_count: 0,
        originator_ip,
    });
    drop(state);
    // The Class 1 loop may be asleep until another connection's due time: wake it for this one.
    shared.opened.notify_one();
    let mut data = Vec::new();
    data.extend_from_slice(&o_to_t_id.to_le_bytes());
    data.extend_from_slice(&fo.t_to_o_id.to_le_bytes());
    data.extend_from_slice(&fo.serial.to_le_bytes());
    data.extend_from_slice(&fo.vendor.to_le_bytes());
    data.extend_from_slice(&fo.originator_serial.to_le_bytes());
    data.extend_from_slice(&fo.o_to_t_rpi.to_le_bytes());
    data.extend_from_slice(&fo.t_to_o_rpi.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    // The I/O port, when it is not the standard one, is the originator's only way to find it.
    let io = *shared.io_addr.lock().unwrap_or_else(|e| e.into_inner());
    let extra = match io {
        Some(a) if a.port() != DEFAULT_IO_PORT => vec![Item { kind: item::SOCKADDR_O_TO_T, data: sockaddr(a) }],
        _ => Vec::new(),
    };
    (general::SUCCESS, Vec::new(), data, extra)
}

fn forward_close(shared: &Shared, body: &[u8]) -> Answer {
    let Some(fc) = parse_forward_close(body) else {
        return status_only(general::NOT_ENOUGH_DATA);
    };
    let triple = (fc.serial, fc.vendor, fc.originator_serial);
    let mut state = shared.lock();
    let before = state.conns.len();
    state.conns.retain(|c| (c.status.serial, c.status.vendor, c.status.originator_serial) != triple);
    let closed = before - state.conns.len();
    state.status.closed += closed as u64;
    drop(state);
    let mut data = Vec::new();
    data.extend_from_slice(&fc.serial.to_le_bytes());
    data.extend_from_slice(&fc.vendor.to_le_bytes());
    data.extend_from_slice(&fc.originator_serial.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    if closed == 0 {
        return (general::CONNECTION_FAILURE, vec![extended::CONNECTION_NOT_FOUND], data, Vec::new());
    }
    (general::SUCCESS, Vec::new(), data, Vec::new())
}

/// The Class 1 socket: frames consumed as they come, each open connection's frames produced at its
/// RPI (due from the last due time, a stall skipping what it missed), a connection whose
/// originator falls silent past its timeout closed.
async fn class1(io: Arc<UdpSocket>, shared: Arc<Shared>) {
    let mut buf = vec![0u8; 65_536];
    loop {
        let wake = {
            let state = shared.lock();
            state.conns.iter().flat_map(|c| [c.next_produce, c.deadline]).min()
        };
        let wake = wake.unwrap_or_else(|| Instant::now() + Duration::from_millis(50));
        tokio::select! {
            r = io.recv_from(&mut buf) => {
                if let Ok((n, from)) = r {
                    consume(&shared, &buf[..n], from);
                }
            }
            _ = tokio::time::sleep_until(wake) => {}
            _ = shared.opened.notified() => {}
        }
        let now = Instant::now();
        let mut out: Vec<(u32, SocketAddr, Vec<u8>)> = Vec::new();
        {
            let mut state = shared.lock();
            let before = state.conns.len();
            state.conns.retain(|c| c.deadline > now);
            let timed_out = (before - state.conns.len()) as u64;
            state.status.timed_out += timed_out;
            state.status.closed += timed_out;
            let producing = shared.producing.load(Ordering::SeqCst);
            let State { conns, assemblies, .. } = &mut *state;
            for c in conns.iter_mut() {
                if c.next_produce > now {
                    continue;
                }
                let every = Duration::from_micros(c.status.t_to_o_rpi_us as u64);
                let mut next = c.next_produce + every;
                while next <= now {
                    next += every;
                }
                c.next_produce = next;
                if !producing {
                    continue;
                }
                c.produce_sequence = c.produce_sequence.wrapping_add(1);
                c.produce_count = c.produce_count.wrapping_add(1);
                let run = (c.input_header == DataHeader::RunIdle).then_some(true);
                let data = assemblies.get(&c.status.input).cloned().unwrap_or_default();
                // Keyed by the adapter's own O->T id, one to an open connection: the T->O id is the
                // originator's, and two originators may choose the same.
                out.push((c.status.o_to_t_id, c.status.sends_to, class1_frame(c.status.t_to_o_id, c.produce_sequence, c.produce_count, run, &data)));
                // Counted as it goes out, so whoever reads the frame finds it counted; a send that
                // fails takes it back.
                c.status.produced += 1;
            }
        }
        for (id, to, frame) in out {
            if io.send_to(&frame, to).await.is_err()
                && let Some(c) = shared.lock().conns.iter_mut().find(|c| c.status.o_to_t_id == id)
            {
                c.status.produced = c.status.produced.saturating_sub(1);
            }
        }
    }
}

fn consume(shared: &Shared, frame: &[u8], from: SocketAddr) {
    let mut state = shared.lock();
    let Some((id, sequence, payload)) = read_class1(frame) else {
        state.status.stray_frames += 1;
        return;
    };
    let State { conns, assemblies, status, .. } = &mut *state;
    let Some(c) = conns.iter_mut().find(|c| c.status.o_to_t_id == id) else {
        status.stray_frames += 1;
        return;
    };
    let from_ip = match from {
        SocketAddr::V4(v) => *v.ip(),
        SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
    };
    let size = assemblies.get(&c.status.output).map(Vec::len).unwrap_or(0);
    let header = if c.output_header == DataHeader::RunIdle { 4 } else { 0 };
    if from_ip != c.originator_ip || payload.len() != 2 + header + size {
        status.stray_frames += 1;
        return;
    }
    // A frame no newer than the last taken (the 32-bit sequence, read across its wrap) is a late
    // one: it neither feeds the watchdog nor touches the outputs.
    if let Some(last) = c.last_sequence
        && (sequence.wrapping_sub(last) as i32) <= 0
    {
        status.stale_frames += 1;
        return;
    }
    c.last_sequence = Some(sequence);
    c.deadline = Instant::now() + c.timeout;
    c.status.consumed += 1;
    let count = le16(&payload, 0);
    if c.last_count == Some(count) {
        return;
    }
    c.last_count = Some(count);
    let run = if header == 4 { le32(&payload, 2) & 1 == 1 } else { true };
    if header == 4 {
        c.status.run = Some(run);
    }
    // Idle data is not the outputs': the assembly keeps what the last run frame set.
    if run && let Some(a) = assemblies.get_mut(&c.status.output) {
        a.copy_from_slice(&payload[2 + header..]);
    }
}
