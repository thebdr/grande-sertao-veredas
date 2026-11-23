//! An EtherNet/IP scanner: one Class 1 connection to an adapter, its outputs produced and its
//! inputs consumed at the requested packet interval, every step bounded in time.
//!
//! An adapter (a remote I/O station, a drive) holds assemblies: runs of bytes by instance. A
//! scanner opens a Class 1 connection naming three of them (the configuration, the output the
//! adapter consumes, the input it produces) over the encapsulation session on TCP, then the two
//! exchange their assemblies over UDP, each at its interval, until one closes the connection or
//! stops hearing from the other. The frames are the standard's (CIP Volume 1, the Connection
//! Manager; Volume 2, EtherNet/IP), encoded with the `ethernetip-core` and
//! `ethernetip-connections` crates' codecs. The scanner itself is written here, for a controller
//! that leaves it running for years:
//!
//! - a connection outside what the 16-bit Forward_Open can carry (a direction's size, data plus
//!   the 2-byte sequence count plus the 32-bit run/idle header where the direction has one, past
//!   511 bytes), an interval of zero or past 4294 s, a multiplier past 7, or an IPv6 address for
//!   the adapter or the I/O socket (the Sockaddr Info items carry IPv4 alone) is refused before
//!   anything is sent;
//! - connecting, registering the session, the Forward_Open, the Forward_Close: each bounded by
//!   [`Timeouts`]; an adapter's refusal comes back with the standard's general and extended
//!   status ([`EnipError::Open`]);
//! - the Sockaddr Info items carry the standard's numbers: the scanner names where the adapter
//!   sends its inputs in item 0x8001 (T→O) and reads where to send its outputs from item 0x8000
//!   (O→T), falling back to the adapter's address and port 2222;
//! - inputs are taken from the adapter alone (the address of its session, or the one its reply
//!   names for the outputs), for this connection's id and size; a repeated sequence count is no
//!   new data, and nothing else resets the connection's timeout;
//! - the connection's timeout is the adapter's actual interval times 4 shifted left by the
//!   multiplier: an adapter silent that long has lost the connection ([`LinkState::Lost`]), the
//!   scanner stops producing, and the inputs it holds are the last ones received; the first input
//!   is awaited the standard's initial watchdog, 10 s ([`FIRST_INPUT`]), or the timeout when
//!   longer, counted from the open's end, and the outputs go from the open on;
//! - [`Scanner::close`] (and a drop) stops producing, sends the Forward_Close and unregisters the
//!   session whatever the close's answer, bounded, so the adapter frees its output at once rather
//!   than at its own timeout.
//!
//! The scanner runs on a thread and a current-thread runtime of its own; its handle is blocking,
//! so a cyclic task sets outputs and reads inputs between scans.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ethernetip_connections::epio::{Frame, decode_frame, encode_frame};
use ethernetip_connections::forward_open::{
    ForwardCloseRequest, ForwardOpenRequest, ForwardOpenResponse, NetworkConnectionParameters,
};
use ethernetip_core::cip::reply_header::ReplyHeader;
use ethernetip_core::cpf::{Envelope, Item, decode_sockaddr_in_v4, encode_envelope};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;

/// The encapsulation port, TCP.
pub const ENCAPSULATION_PORT: u16 = 44818;
/// The Class 1 I/O port, UDP.
pub const IO_PORT: u16 = 2222;
/// How long a consumer waits for the first packet of a connection, by the standard's initial
/// watchdog, before the connection's own timeout applies.
pub const FIRST_INPUT: Duration = Duration::from_secs(10);
/// The largest size of one direction the 16-bit Forward_Open carries, in bytes on the wire.
pub const MAX_CONNECTION_SIZE: u16 = 511;

/// The encapsulation commands the scanner sends.
pub mod command {
    pub const REGISTER_SESSION: u16 = 0x0065;
    pub const UNREGISTER_SESSION: u16 = 0x0066;
    pub const SEND_RR_DATA: u16 = 0x006F;
}

/// The CIP services the scanner asks of the Connection Manager.
pub mod service {
    pub const FORWARD_OPEN: u8 = 0x54;
    pub const FORWARD_CLOSE: u8 = 0x4E;
}

/// The Common Packet Format items, by the standard's numbers.
pub mod item {
    pub const NULL_ADDRESS: u16 = 0x0000;
    pub const UNCONNECTED_DATA: u16 = 0x00B2;
    /// Where O→T data goes: the adapter's I/O address, in a Forward_Open's reply.
    pub const SOCKADDR_O_TO_T: u16 = 0x8000;
    /// Where T→O data goes: the scanner's I/O address, in a Forward_Open's request.
    pub const SOCKADDR_T_TO_O: u16 = 0x8001;
}

/// What a direction's data carries before its bytes, besides the 16-bit sequence count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header {
    /// A 32-bit run/idle header, bit 0 set for run.
    RunIdle,
    /// Nothing: the data alone.
    Modeless,
}

impl Header {
    fn bytes(self) -> u16 {
        match self {
            Header::RunIdle => 4,
            Header::Modeless => 0,
        }
    }
}

/// How long connecting may take, and an answer on the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    pub response: Duration,
}

impl Default for Timeouts {
    /// 3 s to connect, 2 s for an answer.
    fn default() -> Timeouts {
        Timeouts {
            connect: Duration::from_secs(3),
            response: Duration::from_secs(2),
        }
    }
}

/// One Class 1 connection, as the scanner asks it of an adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The adapter's encapsulation address (TCP, port 44818 on a device).
    pub adapter: SocketAddr,
    /// Where the scanner receives the adapter's inputs (UDP; port 2222 on a controller, 0 for any).
    pub io_bind: SocketAddr,
    /// The assembly instances the connection path names: configuration, output (O→T, the
    /// adapter consumes it), input (T→O, the adapter produces it).
    pub config: u16,
    pub output: u16,
    pub input: u16,
    /// The data bytes of each direction, without the sequence count or a header.
    pub output_size: u16,
    pub input_size: u16,
    /// What each direction carries: outputs run/idle and inputs modeless, as is usual.
    pub output_header: Header,
    pub input_header: Header,
    /// The requested packet interval, both directions.
    pub rpi: Duration,
    /// The connection timeout's multiplier code: the timeout is the interval times 4 << code.
    pub timeout_multiplier: u8,
    /// How long the first input may take after the open: the standard's initial watchdog, 10 s,
    /// or the connection's timeout when that is longer.
    pub first_input: Duration,
    /// The originator's identity on the connection: its vendor and serial number.
    pub vendor: u16,
    pub originator_serial: u32,
    pub timeouts: Timeouts,
}

impl Spec {
    /// A connection to `adapter` naming these assemblies, with the usual settings: inputs on port
    /// 2222 of every interface, outputs run/idle, inputs modeless, 10 ms, the multiplier 0 (×4).
    pub fn new(
        adapter: SocketAddr,
        config: u16,
        output: u16,
        input: u16,
        output_size: u16,
        input_size: u16,
    ) -> Spec {
        Spec {
            adapter,
            io_bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), IO_PORT),
            config,
            output,
            input,
            output_size,
            input_size,
            output_header: Header::RunIdle,
            input_header: Header::Modeless,
            rpi: Duration::from_millis(10),
            timeout_multiplier: 0,
            first_input: FIRST_INPUT,
            vendor: 0,
            originator_serial: 0x5645_5245, // "VERE"
            timeouts: Timeouts::default(),
        }
    }

    /// A direction's size on the wire: its data, the sequence count, its header.
    fn wire_size(data: u16, header: Header) -> Option<u16> {
        data.checked_add(2)?
            .checked_add(header.bytes())
            .filter(|n| *n <= MAX_CONNECTION_SIZE)
    }

    fn rpi_us(&self) -> Option<u32> {
        u32::try_from(self.rpi.as_micros())
            .ok()
            .filter(|us| *us > 0)
    }

    /// The connection path: the assembly class, the configuration instance, the two connection
    /// points; an instance past 255 in its 16-bit form.
    pub fn connection_path(&self) -> Vec<u8> {
        let mut path = vec![0x20, 0x04];
        let mut segment = |eight: u8, sixteen: u8, value: u16| match u8::try_from(value) {
            Ok(v) => path.extend_from_slice(&[eight, v]),
            Err(_) => {
                path.extend_from_slice(&[sixteen, 0x00]);
                path.extend_from_slice(&value.to_le_bytes());
            }
        };
        segment(0x24, 0x25, self.config);
        segment(0x2C, 0x2D, self.output);
        segment(0x2C, 0x2D, self.input);
        path
    }

    fn check(&self) -> Result<(u16, u16, u32), EnipError> {
        let refuse = |reason: String| Err(EnipError::Refused { reason });
        let Some(out) = Spec::wire_size(self.output_size, self.output_header) else {
            return refuse(format!(
                "{} output bytes with their count and header pass the {MAX_CONNECTION_SIZE} bytes a Forward_Open carries",
                self.output_size
            ));
        };
        let Some(inp) = Spec::wire_size(self.input_size, self.input_header) else {
            return refuse(format!(
                "{} input bytes with their count and header pass the {MAX_CONNECTION_SIZE} bytes a Forward_Open carries",
                self.input_size
            ));
        };
        let Some(rpi) = self.rpi_us() else {
            return refuse(format!(
                "an interval of {} µs: the standard takes 1 to 4294967295",
                self.rpi.as_micros()
            ));
        };
        if self.timeout_multiplier > 7 {
            return refuse(format!(
                "a timeout multiplier of {}: the standard takes 0 to 7",
                self.timeout_multiplier
            ));
        }
        // EtherNet/IP's Sockaddr Info items carry IPv4 addresses alone, and a socket bound to an
        // IPv6 address would see the adapter's IPv4 datagrams under another address.
        if !self.adapter.is_ipv4() {
            return refuse(format!(
                "the adapter {}: EtherNet/IP's I/O addresses are IPv4",
                self.adapter
            ));
        }
        if !self.io_bind.is_ipv4() {
            return refuse(format!(
                "the I/O socket {}: EtherNet/IP's I/O addresses are IPv4",
                self.io_bind
            ));
        }
        Ok((out, inp, rpi))
    }
}

/// Why opening or closing produced nothing.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EnipError {
    /// Refused before anything was sent.
    #[error("not sent: {reason}")]
    Refused { reason: String },
    /// No connection to the adapter within the bound, or no I/O socket: nothing was sent.
    #[error("no connection to {addr}: {reason}")]
    NotConnected { addr: SocketAddr, reason: String },
    /// The adapter refused the encapsulation session, with its status.
    #[error("the adapter refused the session: encapsulation status {status:#06x}")]
    Session { status: u32 },
    /// The adapter refused the Forward_Open, with the standard's general and extended status.
    #[error(
        "the adapter refused the connection: general status {general:#04x}{}",
        extended_text(extended)
    )]
    Open { general: u8, extended: Vec<u16> },
    /// The adapter refused the Forward_Close: the connection may stay open at its end until its
    /// own timeout.
    #[error(
        "the adapter refused the close: general status {general:#04x}{}",
        extended_text(extended)
    )]
    Close { general: u8, extended: Vec<u16> },
    /// A request on the session got no answer within the bound, or one that does not answer it;
    /// the session was closed.
    #[error("{step}: {cause}")]
    NoAnswer { step: &'static str, cause: String },
}

fn extended_text(extended: &[u16]) -> String {
    match extended {
        [] => String::new(),
        [one] => format!(", extended {one:#06x}"),
        many => format!(", extended {many:04x?}"),
    }
}

/// Where the connection stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Both directions exchanging.
    Running,
    /// The adapter fell silent past the connection's timeout (or sent no first input within its
    /// window, the duration then that window's): no outputs are produced any more, and a new
    /// connection must be opened.
    Lost { timeout: Duration },
    /// Closed by the scanner.
    Closed,
}

/// What the scanner holds of the adapter's inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// The last data received, zeros before the first frame.
    pub data: Vec<u8>,
    /// Frames that carried new data (a new sequence count).
    pub updates: u64,
    /// The last run/idle header received, on a run/idle input; `None` before it or on a
    /// modeless one.
    pub run: Option<bool>,
}

/// What the scanner counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Output frames sent.
    pub produced: u64,
    /// Input frames of this connection received, new data or not.
    pub consumed: u64,
    /// Datagrams that were not this connection's input: another host, id or size, or unreadable.
    pub stray: u64,
}

/// The connection's identity, as the Forward_Open's reply gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    pub o_to_t_id: u32,
    pub t_to_o_id: u32,
    pub connection_serial: u16,
    /// The intervals the adapter granted.
    pub o_to_t_rpi: Duration,
    pub t_to_o_rpi: Duration,
    /// Where outputs are sent.
    pub sends_to: SocketAddr,
    /// Where inputs are received.
    pub receives_on: SocketAddr,
    /// The connection's timeout: no input for this long and it is lost.
    pub timeout: Duration,
    /// How long the first input may take after the open: the longer of the spec's `first_input`
    /// and the timeout.
    pub first_input: Duration,
}

struct Io {
    outputs: Vec<u8>,
    run: bool,
    inputs: Inputs,
    counters: Counters,
    state: LinkState,
}

struct Shared {
    io: Mutex<Io>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Io> {
        // A panic while the lock was held leaves bytes mid-copy at worst.
        self.io.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// An open Class 1 connection.
pub struct Scanner {
    opened: Opened,
    shared: Arc<Shared>,
    close_tx: Option<mpsc::UnboundedSender<std::sync::mpsc::Sender<Result<(), EnipError>>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scanner")
            .field("opened", &self.opened)
            .finish()
    }
}

/// A connection serial number for each Forward_Open of the process: unique among the
/// connections one originator holds, never 0.
static NEXT_SERIAL: AtomicU16 = AtomicU16::new(0);

fn next_serial() -> u16 {
    loop {
        let seed = (std::process::id() as u16).wrapping_mul(0x9E37);
        let s = NEXT_SERIAL
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(seed);
        if s != 0 {
            return s;
        }
    }
}

impl Scanner {
    /// Opens the connection: the session, then the Forward_Open, each bounded; then the exchange
    /// runs on a thread of its own until [`Scanner::close`], a drop, or the adapter's silence.
    pub fn open(spec: &Spec) -> Result<Scanner, EnipError> {
        let (out_size, in_size, rpi_us) = spec.check()?;
        let spec = spec.clone();
        let shared = Arc::new(Shared {
            io: Mutex::new(Io {
                outputs: vec![0; usize::from(spec.output_size)],
                run: false,
                inputs: Inputs {
                    data: vec![0; usize::from(spec.input_size)],
                    updates: 0,
                    run: None,
                },
                counters: Counters::default(),
                state: LinkState::Running,
            }),
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Opened, EnipError>>();
        let (close_tx, close_rx) = mpsc::unbounded_channel();
        let thread_shared = Arc::clone(&shared);
        let addr = spec.adapter;
        let thread = std::thread::Builder::new()
            .name("enip-scanner".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ready_tx.send(Err(EnipError::NotConnected {
                            addr,
                            reason: e.to_string(),
                        }));
                        return;
                    }
                };
                runtime.block_on(run(
                    spec,
                    out_size,
                    in_size,
                    rpi_us,
                    thread_shared,
                    ready_tx,
                    close_rx,
                ));
            })
            .map_err(|e| EnipError::NotConnected {
                addr,
                reason: e.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok(opened)) => Ok(Scanner {
                opened,
                shared,
                close_tx: Some(close_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(EnipError::NotConnected {
                    addr,
                    reason: "the scanner's thread ended".into(),
                })
            }
        }
    }

    /// The connection as the adapter granted it.
    pub fn opened(&self) -> Opened {
        self.opened
    }

    /// The outputs the next frames carry; exactly the connection's output size.
    pub fn set_outputs(&self, data: &[u8]) -> Result<(), EnipError> {
        let mut io = self.shared.lock();
        if data.len() != io.outputs.len() {
            return Err(EnipError::Refused {
                reason: format!(
                    "{} output bytes for a connection of {}",
                    data.len(),
                    io.outputs.len()
                ),
            });
        }
        io.outputs.copy_from_slice(data);
        Ok(())
    }

    /// The run/idle header the outputs carry, on a run/idle output: idle (the start) or run.
    pub fn set_run(&self, run: bool) {
        self.shared.lock().run = run;
    }

    pub fn inputs(&self) -> Inputs {
        self.shared.lock().inputs.clone()
    }

    pub fn counters(&self) -> Counters {
        self.shared.lock().counters.clone()
    }

    pub fn state(&self) -> LinkState {
        self.shared.lock().state.clone()
    }

    /// Stops producing, sends the Forward_Close and unregisters the session, bounded. The
    /// connection ends whatever the answer; the result says whether the adapter confirmed it.
    pub fn close(mut self) -> Result<(), EnipError> {
        self.shut()
    }

    fn shut(&mut self) -> Result<(), EnipError> {
        let Some(close_tx) = self.close_tx.take() else {
            return Ok(());
        };
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let result = if close_tx.send(done_tx).is_ok() {
            done_rx.recv().unwrap_or(Ok(()))
        } else {
            Ok(())
        };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut io = self.shared.lock();
        if io.state == LinkState::Running {
            io.state = LinkState::Closed;
        }
        result
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        let _ = self.shut();
    }
}

/// The encapsulation session on TCP: one request at a time, each answer bounded and checked.
struct Session {
    stream: TcpStream,
    handle: u32,
    context: u64,
    bound: Duration,
}

impl Session {
    async fn exchange(
        &mut self,
        step: &'static str,
        command: u16,
        body: &[u8],
    ) -> Result<Vec<u8>, EnipError> {
        self.context = self.context.wrapping_add(1);
        let context = self.context.to_le_bytes();
        let mut frame = Vec::with_capacity(24 + body.len());
        frame.extend_from_slice(&command.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u16).to_le_bytes());
        frame.extend_from_slice(&self.handle.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.extend_from_slice(&context);
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.extend_from_slice(body);
        let fail = |cause: String| EnipError::NoAnswer { step, cause };
        let exchanged = tokio::time::timeout(self.bound, async {
            self.stream.write_all(&frame).await?;
            let mut header = [0u8; 24];
            self.stream.read_exact(&mut header).await?;
            let length = u16::from_le_bytes([header[2], header[3]]);
            let mut data = vec![0u8; usize::from(length)];
            self.stream.read_exact(&mut data).await?;
            Ok::<_, std::io::Error>((header, data))
        })
        .await;
        let (header, data) = match exchanged {
            Err(_) => {
                return Err(fail(format!(
                    "no answer within {} ms",
                    self.bound.as_millis()
                )));
            }
            Ok(Err(e)) => return Err(fail(format!("the link failed: {e}"))),
            Ok(Ok(x)) => x,
        };
        let got_command = u16::from_le_bytes([header[0], header[1]]);
        let status = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        if got_command != command {
            return Err(fail(format!(
                "an answer to command {got_command:#06x}, not {command:#06x}"
            )));
        }
        if header[12..20] != context {
            return Err(fail(
                "an answer for another request (its sender context)".into(),
            ));
        }
        if status != 0 {
            if command == command::REGISTER_SESSION {
                return Err(EnipError::Session { status });
            }
            return Err(fail(format!("encapsulation status {status:#010x}")));
        }
        if command == command::REGISTER_SESSION {
            self.handle = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
            if self.handle == 0 {
                return Err(fail("a session handle of 0".into()));
            }
        } else if u32::from_le_bytes([header[4], header[5], header[6], header[7]]) != self.handle {
            return Err(fail("an answer on another session".into()));
        }
        Ok(data)
    }

    /// A request to the Connection Manager in SendRRData; its reply's header and body.
    async fn connection_manager(
        &mut self,
        step: &'static str,
        service: u8,
        body: &[u8],
        extra: Vec<Item>,
    ) -> Result<(ReplyHeader, Vec<u8>, Envelope), EnipError> {
        let mut request = vec![service, 0x02, 0x20, 0x06, 0x24, 0x01];
        request.extend_from_slice(body);
        let mut items = vec![
            Item::new(item::NULL_ADDRESS, Vec::new()),
            Item::new(item::UNCONNECTED_DATA, request),
        ];
        items.extend(extra);
        let answer = self
            .exchange(step, command::SEND_RR_DATA, &encode_envelope(0, 0, &items))
            .await?;
        let fail = |cause: String| EnipError::NoAnswer { step, cause };
        let envelope = Envelope::parse(&answer).map_err(|e| fail(e.to_string()))?;
        let data = envelope
            .find(item::UNCONNECTED_DATA)
            .ok_or_else(|| fail("an answer without its unconnected data".into()))?
            .data
            .clone();
        if data.first().copied() != Some(service | 0x80) {
            return Err(fail(format!(
                "a reply to another service ({:02x?})",
                data.first()
            )));
        }
        let header = ReplyHeader::parse(&data).map_err(|e| fail(e.to_string()))?;
        let rest = data.get(header.body_offset..).unwrap_or_default().to_vec();
        Ok((header, rest, envelope))
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    spec: Spec,
    out_size: u16,
    in_size: u16,
    rpi_us: u32,
    shared: Arc<Shared>,
    ready: std::sync::mpsc::Sender<Result<Opened, EnipError>>,
    mut close_rx: mpsc::UnboundedReceiver<std::sync::mpsc::Sender<Result<(), EnipError>>>,
) {
    let serial = next_serial();
    let (mut session, udp, opened) = match open(&spec, out_size, in_size, rpi_us, serial).await {
        Ok(x) => x,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if ready.send(Ok(opened)).is_err() {
        return;
    }
    let mut tick = tokio::time::interval(opened.o_to_t_rpi);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The watch runs from the open's end: the first input within its window, each next one
    // within the timeout of the last.
    let mut last_heard = tokio::time::Instant::now();
    let mut window = opened.first_input;
    let mut last_count: Option<u16> = None;
    let mut sequence: u32 = 0;
    let mut count: u16 = 0;
    let mut buf = vec![0u8; 2048];
    let reply = loop {
        // Saturated: a window too long for the clock (a caller waiting "forever") is a deadline
        // a year away rather than an overflow.
        let deadline = last_heard
            .checked_add(window)
            .unwrap_or_else(|| last_heard + Duration::from_secs(365 * 24 * 3600));
        // In this order: a close first, then the timeout (once it has passed nothing more goes
        // out), then the next output when it is due (so no flood of datagrams, of this
        // connection or not, can hold the outputs back), then inputs.
        tokio::select! {
            biased;
            done = close_rx.recv() => break done,
            _ = tokio::time::sleep_until(deadline) => {
                shared.lock().state = LinkState::Lost { timeout: window };
                // Lost: produce no more, close the session, wait for the handle's close or drop.
                drop(session);
                let done = close_rx.recv().await;
                if let Some(done) = done {
                    let _ = done.send(Ok(()));
                }
                return;
            }
            _ = tick.tick() => {
                let (data, run) = {
                    let io = shared.lock();
                    (io.outputs.clone(), io.run)
                };
                sequence = sequence.wrapping_add(1);
                count = count.wrapping_add(1);
                let frame = encode_frame(&Frame {
                    connection_id: opened.o_to_t_id,
                    sequence,
                    cip_sequence: count,
                    run_idle: (spec.output_header == Header::RunIdle).then_some(run),
                    data,
                });
                if udp.send_to(&frame, opened.sends_to).await.is_ok() {
                    shared.lock().counters.produced += 1;
                }
            }
            received = udp.recv_from(&mut buf) => {
                let Ok((n, from)) = received else { continue };
                // From the adapter: the address of its session, or the one its reply named for the
                // outputs (a device with an I/O interface of its own).
                let from_adapter = from.ip() == spec.adapter.ip() || from.ip() == opened.sends_to.ip();
                let frame = from_adapter
                    .then(|| decode_frame(&buf[..n], spec.input_header == Header::RunIdle).ok())
                    .flatten()
                    .filter(|f| f.connection_id == opened.t_to_o_id && f.data.len() == usize::from(spec.input_size));
                let mut io = shared.lock();
                match frame {
                    None => io.counters.stray += 1,
                    Some(f) => {
                        last_heard = tokio::time::Instant::now();
                        window = opened.timeout;
                        io.counters.consumed += 1;
                        if last_count != Some(f.cip_sequence) {
                            last_count = Some(f.cip_sequence);
                            io.inputs.data = f.data;
                            io.inputs.updates += 1;
                            io.inputs.run = f.run_idle;
                        }
                    }
                }
            }
        }
    };
    let result = close(&mut session, &spec, serial).await;
    if let Some(done) = reply {
        let _ = done.send(result);
    }
}

async fn open(
    spec: &Spec,
    out_size: u16,
    in_size: u16,
    rpi_us: u32,
    serial: u16,
) -> Result<(Session, UdpSocket, Opened), EnipError> {
    let addr = spec.adapter;
    let not_connected = |reason: String| EnipError::NotConnected { addr, reason };
    let stream = match tokio::time::timeout(spec.timeouts.connect, TcpStream::connect(addr)).await {
        Err(_) => {
            return Err(not_connected(format!(
                "no connection within {} ms",
                spec.timeouts.connect.as_millis()
            )));
        }
        Ok(Err(e)) => return Err(not_connected(e.to_string())),
        Ok(Ok(s)) => s,
    };
    stream
        .set_nodelay(true)
        .map_err(|e| not_connected(e.to_string()))?;
    let udp = UdpSocket::bind(spec.io_bind)
        .await
        .map_err(|e| not_connected(format!("the I/O socket {}: {e}", spec.io_bind)))?;
    let receives_on = udp.local_addr().map_err(|e| not_connected(e.to_string()))?;
    let mut session = Session {
        stream,
        handle: 0,
        context: 0,
        bound: spec.timeouts.response,
    };
    session
        .exchange("RegisterSession", command::REGISTER_SESSION, &[1, 0, 0, 0])
        .await?;

    let params = |size: u16| NetworkConnectionParameters {
        redundant_owner: false,
        connection_type: 2, // point to point
        priority: 2,        // scheduled
        variable_size: false,
        size,
    };
    let t_to_o_id = 0x8000_0000 | u32::from(serial);
    let request = ForwardOpenRequest {
        priority_tick: 0x0A,
        timeout_ticks: 0x0E,
        o_to_t_connection_id: 0,
        t_to_o_connection_id: t_to_o_id,
        connection_serial: serial,
        originator_vendor: spec.vendor,
        originator_serial: spec.originator_serial,
        connection_timeout_mult: spec.timeout_multiplier,
        o_to_t_rpi_us: rpi_us,
        o_to_t_params: params(out_size),
        t_to_o_rpi_us: rpi_us,
        t_to_o_params: params(in_size),
        transport_type: 0x01, // Class 1, cyclic, client
        connection_path: spec.connection_path(),
    };
    // Where the adapter sends its inputs: this socket's address, the address 0 when it is bound to
    // every interface (the adapter then sends to the session's peer).
    let mut sockaddr = vec![0u8; 16];
    sockaddr[0..2].copy_from_slice(&2u16.to_be_bytes());
    sockaddr[2..4].copy_from_slice(&receives_on.port().to_be_bytes());
    if let IpAddr::V4(ip) = receives_on.ip() {
        sockaddr[4..8].copy_from_slice(&ip.octets());
    }
    let (header, body, envelope) = session
        .connection_manager(
            "Forward_Open",
            service::FORWARD_OPEN,
            &request.encode(),
            vec![Item::new(item::SOCKADDR_T_TO_O, sockaddr)],
        )
        .await?;
    if header.general_status != 0 {
        return Err(EnipError::Open {
            general: header.general_status,
            extended: header.extended_status,
        });
    }
    let fail = |cause: String| EnipError::NoAnswer {
        step: "Forward_Open",
        cause,
    };
    let reply = ForwardOpenResponse::decode(&body).map_err(|e| fail(e.to_string()))?;
    if (
        reply.connection_serial,
        reply.originator_vendor,
        reply.originator_serial,
    ) != (serial, spec.vendor, spec.originator_serial)
    {
        return Err(fail("a reply for another originator's connection".into()));
    }
    if reply.o_to_t_actual_rpi_us == 0 || reply.t_to_o_actual_rpi_us == 0 {
        return Err(fail("a granted interval of 0".into()));
    }
    let adapter_ip = addr.ip();
    let sends_to = match envelope.find(item::SOCKADDR_O_TO_T) {
        None => SocketAddr::new(adapter_ip, IO_PORT),
        Some(i) => {
            let v4 = decode_sockaddr_in_v4(&i.data).map_err(|e| fail(e.to_string()))?;
            let ip = if v4.ip().is_unspecified() {
                adapter_ip
            } else {
                IpAddr::V4(*v4.ip())
            };
            SocketAddr::new(ip, if v4.port() == 0 { IO_PORT } else { v4.port() })
        }
    };
    let t_to_o_rpi = Duration::from_micros(u64::from(reply.t_to_o_actual_rpi_us));
    let opened = Opened {
        o_to_t_id: reply.o_to_t_connection_id,
        t_to_o_id: reply.t_to_o_connection_id,
        connection_serial: serial,
        o_to_t_rpi: Duration::from_micros(u64::from(reply.o_to_t_actual_rpi_us)),
        t_to_o_rpi,
        sends_to,
        receives_on,
        timeout: t_to_o_rpi * (4u32 << spec.timeout_multiplier),
        first_input: spec
            .first_input
            .max(t_to_o_rpi * (4u32 << spec.timeout_multiplier)),
    };
    Ok((session, udp, opened))
}

async fn close(session: &mut Session, spec: &Spec, serial: u16) -> Result<(), EnipError> {
    let request = ForwardCloseRequest {
        priority_tick: 0x0A,
        timeout_ticks: 0x0E,
        connection_serial: serial,
        originator_vendor: spec.vendor,
        originator_serial: spec.originator_serial,
        connection_path: spec.connection_path(),
    };
    let closed = session
        .connection_manager(
            "Forward_Close",
            service::FORWARD_CLOSE,
            &request.encode(),
            Vec::new(),
        )
        .await;
    // UnRegisterSession has no answer: sent whatever the close's answer, then the session is
    // closed.
    let mut frame = Vec::with_capacity(24);
    frame.extend_from_slice(&command::UNREGISTER_SESSION.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&session.handle.to_le_bytes());
    frame.extend_from_slice(&[0u8; 16]);
    let _ = tokio::time::timeout(session.bound, session.stream.write_all(&frame)).await;
    let _ = tokio::time::timeout(session.bound, session.stream.shutdown()).await;
    let (header, _, _) = closed?;
    if header.general_status != 0 {
        return Err(EnipError::Close {
            general: header.general_status,
            extended: header.extended_status,
        });
    }
    Ok(())
}
