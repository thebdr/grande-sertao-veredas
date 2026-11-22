//! A Modbus/TCP client: a device's four tables read and written over one TCP connection, every
//! call bounded in time and every answer checked against its request.
//!
//! A Modbus device holds four tables: coils (bits a client reads and writes), discrete inputs
//! (bits it only reads), holding registers (16-bit words it reads and writes) and input registers
//! (words it only reads). A [`Client`] speaks ten of the standard's public functions on that data
//! model: the four reads, a coil or a register written alone or several at once, a register
//! masked, registers read and written in one request (Read FIFO Queue, 0x18, is not spoken). The frames are the standard's (an MBAP header with its
//! transaction number, protocol 0, length and unit identifier, then the function's PDU), framed
//! by the tokio-modbus crate; this module adds what a controller needs of a link it leaves running
//! for years:
//!
//! - a request outside the standard's bounds (a quantity of 0 or past the function's limit, a range
//!   past address 65535) is refused before anything is sent;
//! - connecting is bounded by [`Timeouts::connect`] and an answer by [`Timeouts::response`];
//! - every answer is checked against its request: the transaction number, the unit, the function,
//!   the quantity read (bits by their byte count, words one by one), and a write's echo (address,
//!   value or quantity, masks). A short or mismatched answer is never handed on;
//! - a call whose answer does not come in time, comes broken, or does not match ends the
//!   connection, so a late answer is never read as the next request's; the next call connects
//!   again. A device's exception answer keeps the connection: the device is there and answered;
//! - a write says what the device may have done. Refused before the wire, or no connection: it
//!   never left. An exception the standard gives for a request not carried out (an illegal
//!   function, address or value, a device busy, a gateway with no path): the device did not do it
//!   ([`ModbusError::Exception`]). No answer, a broken answer or a mismatched echo after the
//!   request left, or an exception that does not say the write was not done (a device failure
//!   while performing it, an acknowledge of a request still running, a gateway's target that did
//!   not answer, a code the standard does not name): [`ModbusError::Unconfirmed`], because the
//!   request may have landed and no client can recall it.
//!
//! The client is blocking, on a current-thread runtime of its own, so a cyclic task calls it
//! between scans; it must not be called from inside another async runtime's task. Dropping it
//! closes the connection.

use std::borrow::Cow;
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_modbus::client::{Client as _, Context};
use tokio_modbus::{Request, Response, Slave};

/// The port a Modbus/TCP device listens on.
pub const DEFAULT_PORT: u16 = 502;

/// The standard's limits on one request's quantity.
pub mod limit {
    /// Coils or discrete inputs read at once.
    pub const READ_BITS: u16 = 2000;
    /// Holding or input registers read at once (and read by a read-write request).
    pub const READ_REGISTERS: u16 = 125;
    /// Coils written at once.
    pub const WRITE_COILS: u16 = 1968;
    /// Registers written at once.
    pub const WRITE_REGISTERS: u16 = 123;
    /// Registers written by a read-write request.
    pub const READ_WRITE_WRITTEN: u16 = 121;
}

/// The function codes this client sends.
pub mod function {
    pub const READ_COILS: u8 = 0x01;
    pub const READ_DISCRETE_INPUTS: u8 = 0x02;
    pub const READ_HOLDING_REGISTERS: u8 = 0x03;
    pub const READ_INPUT_REGISTERS: u8 = 0x04;
    pub const WRITE_SINGLE_COIL: u8 = 0x05;
    pub const WRITE_SINGLE_REGISTER: u8 = 0x06;
    pub const WRITE_MULTIPLE_COILS: u8 = 0x0F;
    pub const WRITE_MULTIPLE_REGISTERS: u8 = 0x10;
    pub const MASK_WRITE_REGISTER: u8 = 0x16;
    pub const READ_WRITE_MULTIPLE_REGISTERS: u8 = 0x17;
}

/// Where a device is: its address and the unit identifier its requests carry (a device behind a
/// gateway is told apart by it; a device of its own commonly answers any).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub addr: SocketAddr,
    pub unit: u8,
}

/// How long a connection may take to open, and an answer to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    pub response: Duration,
}

impl Default for Timeouts {
    /// 3 s to connect, 1 s for an answer.
    fn default() -> Timeouts {
        Timeouts {
            connect: Duration::from_secs(3),
            response: Duration::from_secs(1),
        }
    }
}

/// Why an answer was not taken, after the request left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// No answer within the response bound.
    Timeout(Duration),
    /// The connection broke, or an answer could not be decoded.
    Link(String),
    /// An answer came that does not answer the request.
    Mismatch(String),
    /// The device answered a write with an exception that does not say it was not done
    /// ([`refuses`] is false for the code); the connection stays.
    Exception(u8),
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cause::Timeout(d) => write!(f, "no answer within {} ms", d.as_millis()),
            Cause::Link(r) => write!(f, "the link failed: {r}"),
            Cause::Mismatch(r) => write!(f, "the answer does not match the request: {r}"),
            Cause::Exception(code) => {
                write!(
                    f,
                    "the device answered exception {code:#04x} ({})",
                    exception_name(*code)
                )
            }
        }
    }
}

/// Why a call produced nothing.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ModbusError {
    /// Refused before anything was sent: the request is outside the standard's bounds.
    #[error("not sent: {reason}")]
    Refused { reason: String },
    /// No connection within the bound: nothing was sent.
    #[error("no connection to {addr}: {reason}")]
    NotConnected { addr: SocketAddr, reason: String },
    /// The device answered with an exception: a read's data did not come, or a write was not
    /// carried out (a write's exception that does not say so is [`ModbusError::Unconfirmed`]).
    /// The connection stays.
    #[error("the device answered function {function:#04x} with exception {code:#04x} ({})", exception_name(*code))]
    Exception { function: u8, code: u8 },
    /// A read left and its answer was not taken; the connection was closed.
    #[error("function {function:#04x} got no answer: {cause}")]
    NoAnswer { function: u8, cause: Cause },
    /// A write left and no answer said it was not done; the device may have done it. The
    /// connection was closed, but for [`Cause::Exception`], a proper answer, when it stays.
    #[error("function {function:#04x} unconfirmed, it may have landed: {cause}")]
    Unconfirmed { function: u8, cause: Cause },
}

/// The standard's name of an exception code.
pub fn exception_name(code: u8) -> &'static str {
    match code {
        0x01 => "illegal function",
        0x02 => "illegal data address",
        0x03 => "illegal data value",
        0x04 => "server device failure",
        0x05 => "acknowledge",
        0x06 => "server device busy",
        0x08 => "memory parity error",
        0x0A => "gateway path unavailable",
        0x0B => "gateway target device failed to respond",
        _ => "not a standard exception",
    }
}

/// A Modbus/TCP client of one device.
pub struct Client {
    target: Target,
    timeouts: Timeouts,
    runtime: Runtime,
    link: Option<Context>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("target", &self.target)
            .field("timeouts", &self.timeouts)
            .field("connected", &self.link.is_some())
            .finish()
    }
}

/// What a request asks, kept to check its answer.
enum Asked {
    Bits(u16),
    Words(u16),
    Echo(Response),
}

impl Client {
    /// A client of `target`, not yet connected: the first call connects.
    pub fn new(target: Target, timeouts: Timeouts) -> std::io::Result<Client> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        Ok(Client {
            target,
            timeouts,
            runtime,
            link: None,
        })
    }

    pub fn target(&self) -> Target {
        self.target
    }

    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// Whether a connection is open (as far as this client knows: a peer that went away shows
    /// at the next call).
    pub fn is_connected(&self) -> bool {
        self.link.is_some()
    }

    /// Opens the connection if none is open, within the connect bound.
    pub fn connect(&mut self) -> Result<(), ModbusError> {
        if self.link.is_some() {
            return Ok(());
        }
        let Target { addr, unit } = self.target;
        let bound = self.timeouts.connect;
        let opened = self.runtime.block_on(async move {
            match tokio::time::timeout(bound, TcpStream::connect(addr)).await {
                Err(_) => Err(format!("no connection within {} ms", bound.as_millis())),
                Ok(Err(e)) => Err(e.to_string()),
                Ok(Ok(stream)) => {
                    // A request is one small segment: send it at once.
                    stream.set_nodelay(true).map_err(|e| e.to_string())?;
                    Ok(tokio_modbus::client::tcp::attach_slave(stream, Slave(unit)))
                }
            }
        });
        match opened {
            Ok(link) => {
                self.link = Some(link);
                Ok(())
            }
            Err(reason) => Err(ModbusError::NotConnected { addr, reason }),
        }
    }

    /// Closes the connection, if one is open. The next call connects again.
    pub fn disconnect(&mut self) {
        self.link = None;
    }

    /// Reads `count` coils from `start` (function 0x01).
    pub fn read_coils(&mut self, start: u16, count: u16) -> Result<Vec<bool>, ModbusError> {
        check_range(start, count, limit::READ_BITS, "coils read")?;
        self.bits(Request::ReadCoils(start, count), count)
    }

    /// Reads `count` discrete inputs from `start` (function 0x02).
    pub fn read_discrete_inputs(
        &mut self,
        start: u16,
        count: u16,
    ) -> Result<Vec<bool>, ModbusError> {
        check_range(start, count, limit::READ_BITS, "discrete inputs read")?;
        self.bits(Request::ReadDiscreteInputs(start, count), count)
    }

    /// Reads `count` holding registers from `start` (function 0x03).
    pub fn read_holding_registers(
        &mut self,
        start: u16,
        count: u16,
    ) -> Result<Vec<u16>, ModbusError> {
        check_range(
            start,
            count,
            limit::READ_REGISTERS,
            "holding registers read",
        )?;
        self.words(Request::ReadHoldingRegisters(start, count), count)
    }

    /// Reads `count` input registers from `start` (function 0x04).
    pub fn read_input_registers(
        &mut self,
        start: u16,
        count: u16,
    ) -> Result<Vec<u16>, ModbusError> {
        check_range(start, count, limit::READ_REGISTERS, "input registers read")?;
        self.words(Request::ReadInputRegisters(start, count), count)
    }

    /// Writes one coil (function 0x05).
    pub fn write_coil(&mut self, address: u16, value: bool) -> Result<(), ModbusError> {
        self.echo(
            Request::WriteSingleCoil(address, value),
            Response::WriteSingleCoil(address, value),
        )
    }

    /// Writes one holding register (function 0x06).
    pub fn write_register(&mut self, address: u16, value: u16) -> Result<(), ModbusError> {
        self.echo(
            Request::WriteSingleRegister(address, value),
            Response::WriteSingleRegister(address, value),
        )
    }

    /// Writes consecutive coils from `start` (function 0x0F).
    pub fn write_coils(&mut self, start: u16, values: &[bool]) -> Result<(), ModbusError> {
        let count = quantity(values.len(), limit::WRITE_COILS, "coils written")?;
        check_range(start, count, limit::WRITE_COILS, "coils written")?;
        self.echo(
            Request::WriteMultipleCoils(start, Cow::Borrowed(values)),
            Response::WriteMultipleCoils(start, count),
        )
    }

    /// Writes consecutive holding registers from `start` (function 0x10).
    pub fn write_registers(&mut self, start: u16, values: &[u16]) -> Result<(), ModbusError> {
        let count = quantity(values.len(), limit::WRITE_REGISTERS, "registers written")?;
        check_range(start, count, limit::WRITE_REGISTERS, "registers written")?;
        self.echo(
            Request::WriteMultipleRegisters(start, Cow::Borrowed(values)),
            Response::WriteMultipleRegisters(start, count),
        )
    }

    /// Sets a holding register to `(current AND and) OR (or AND NOT and)` (function 0x16).
    pub fn mask_write_register(
        &mut self,
        address: u16,
        and: u16,
        or: u16,
    ) -> Result<(), ModbusError> {
        self.echo(
            Request::MaskWriteRegister(address, and, or),
            Response::MaskWriteRegister(address, and, or),
        )
    }

    /// Writes `values` from `write_start`, then reads `read_count` holding registers from
    /// `read_start`, in one request (function 0x17).
    pub fn read_write_registers(
        &mut self,
        read_start: u16,
        read_count: u16,
        write_start: u16,
        values: &[u16],
    ) -> Result<Vec<u16>, ModbusError> {
        check_range(
            read_start,
            read_count,
            limit::READ_REGISTERS,
            "registers read",
        )?;
        let written = quantity(values.len(), limit::READ_WRITE_WRITTEN, "registers written")?;
        check_range(
            write_start,
            written,
            limit::READ_WRITE_WRITTEN,
            "registers written",
        )?;
        let request = Request::ReadWriteMultipleRegisters(
            read_start,
            read_count,
            write_start,
            Cow::Borrowed(values),
        );
        match self.call(request, Asked::Words(read_count))? {
            Response::ReadWriteMultipleRegisters(words) => Ok(words),
            other => Err(self.unexpected(function::READ_WRITE_MULTIPLE_REGISTERS, &other)),
        }
    }

    fn bits(&mut self, request: Request<'_>, count: u16) -> Result<Vec<bool>, ModbusError> {
        let function = request.function_code().value();
        match self.call(request, Asked::Bits(count))? {
            Response::ReadCoils(mut bits) | Response::ReadDiscreteInputs(mut bits) => {
                bits.truncate(usize::from(count));
                Ok(bits)
            }
            other => Err(self.unexpected(function, &other)),
        }
    }

    fn words(&mut self, request: Request<'_>, count: u16) -> Result<Vec<u16>, ModbusError> {
        let function = request.function_code().value();
        match self.call(request, Asked::Words(count))? {
            Response::ReadHoldingRegisters(words) | Response::ReadInputRegisters(words) => {
                Ok(words)
            }
            other => Err(self.unexpected(function, &other)),
        }
    }

    fn echo(&mut self, request: Request<'_>, expected: Response) -> Result<(), ModbusError> {
        self.call(request, Asked::Echo(expected)).map(|_| ())
    }

    /// A response `call` let through that is not the request's kind: `call` checks the function,
    /// so this does not happen; it is refused like any mismatch rather than unwrapped.
    fn unexpected(&mut self, function: u8, response: &Response) -> ModbusError {
        self.link = None;
        failed(function, Cause::Mismatch(format!("{response:?}")))
    }

    /// Sends one request and takes its answer, or ends the connection.
    fn call(&mut self, request: Request<'_>, asked: Asked) -> Result<Response, ModbusError> {
        let function = request.function_code().value();
        self.connect()?;
        let bound = self.timeouts.response;
        let Some(link) = self.link.as_mut() else {
            return Err(ModbusError::NotConnected {
                addr: self.target.addr,
                reason: "the connection closed before the request".to_string(),
            });
        };
        // The bound's timer is made inside the runtime, which drives it.
        let answered = self
            .runtime
            .block_on(async { tokio::time::timeout(bound, link.call(request)).await });
        let cause = match answered {
            Err(_) => Cause::Timeout(bound),
            Ok(Err(e)) => match e {
                tokio_modbus::Error::Transport(io) => Cause::Link(io.to_string()),
                tokio_modbus::Error::Protocol(p) => Cause::Mismatch(p.to_string()),
            },
            Ok(Ok(Err(exception))) => {
                let code = u8::from(exception);
                return Err(if writes(function) && !refuses(code) {
                    ModbusError::Unconfirmed {
                        function,
                        cause: Cause::Exception(code),
                    }
                } else {
                    ModbusError::Exception { function, code }
                });
            }
            Ok(Ok(Ok(response))) => match check(&asked, &response) {
                Ok(()) => return Ok(response),
                Err(reason) => Cause::Mismatch(reason),
            },
        };
        self.link = None;
        Err(failed(function, cause))
    }
}

/// Whether a function writes to the device.
fn writes(function: u8) -> bool {
    matches!(
        function,
        function::WRITE_SINGLE_COIL
            | function::WRITE_SINGLE_REGISTER
            | function::WRITE_MULTIPLE_COILS
            | function::WRITE_MULTIPLE_REGISTERS
            | function::MASK_WRITE_REGISTER
            | function::READ_WRITE_MULTIPLE_REGISTERS
    )
}

/// Whether an exception code says the request was not carried out: an illegal function (0x01),
/// data address (0x02) or data value (0x03), a device busy with a long command (0x06, retry
/// later), a gateway with no path to the target (0x0A). The rest do not: a device failure while
/// performing the request (0x04), an acknowledge of a request it goes on processing (0x05), a
/// memory parity error (0x08), a gateway whose target did not answer after it was sent (0x0B), and
/// any code the standard does not name.
pub fn refuses(code: u8) -> bool {
    matches!(code, 0x01 | 0x02 | 0x03 | 0x06 | 0x0A)
}

/// A write that left unanswered may have landed; a read only got nothing.
fn failed(function: u8, cause: Cause) -> ModbusError {
    if writes(function) {
        ModbusError::Unconfirmed { function, cause }
    } else {
        ModbusError::NoAnswer { function, cause }
    }
}

/// Whether an answer answers what was asked: bits by their byte count, words one by one, a
/// write's echo whole.
fn check(asked: &Asked, response: &Response) -> Result<(), String> {
    match (asked, response) {
        (Asked::Bits(count), Response::ReadCoils(bits) | Response::ReadDiscreteInputs(bits)) => {
            let bytes = usize::from(*count).div_ceil(8);
            if bits.len() == bytes * 8 {
                Ok(())
            } else {
                Err(format!(
                    "{} bytes of bits for {count} bits, not {bytes}",
                    bits.len() / 8
                ))
            }
        }
        (
            Asked::Words(count),
            Response::ReadHoldingRegisters(words)
            | Response::ReadInputRegisters(words)
            | Response::ReadWriteMultipleRegisters(words),
        ) => {
            if words.len() == usize::from(*count) {
                Ok(())
            } else {
                Err(format!("{} registers for {count}", words.len()))
            }
        }
        (Asked::Echo(expected), got) => {
            if got == expected {
                Ok(())
            } else {
                Err(format!("echo {got:?}, not {expected:?}"))
            }
        }
        (_, got) => Err(format!("{got:?}")),
    }
}

/// A slice's length as a request's quantity, within the function's limit.
fn quantity(len: usize, max: u16, what: &str) -> Result<u16, ModbusError> {
    match u16::try_from(len) {
        Ok(n) if (1..=max).contains(&n) => Ok(n),
        _ => Err(ModbusError::Refused {
            reason: format!("{len} {what}: the standard takes 1 to {max}"),
        }),
    }
}

/// A quantity within the function's limit, and the range within the 65 536 addresses.
fn check_range(start: u16, count: u16, max: u16, what: &str) -> Result<(), ModbusError> {
    if count == 0 || count > max {
        return Err(ModbusError::Refused {
            reason: format!("{count} {what}: the standard takes 1 to {max}"),
        });
    }
    if u32::from(start) + u32::from(count) > 65_536 {
        return Err(ModbusError::Refused {
            reason: format!("{count} {what} from {start} run past address 65535"),
        });
    }
    Ok(())
}
