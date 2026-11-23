//! The Modbus server: a device's four tables served over Modbus/TCP on a real socket, so a Modbus
//! client meets its device on the bench before it meets the plant.
//!
//! A [`ModbusConfig`] (read from a bench file) sizes the four tables a Modbus device holds, coils
//! and discrete inputs (bits), input and holding registers (16-bit words), and names points in
//! them: a bool in a bit, or a number over one or two registers (`u16`, `i16`, `u32`, `i32`,
//! `f32`, the two-register types in either word order). [`start`] serves them on a thread and a
//! runtime of its own: every public function code a device's data model answers to (read coils,
//! discrete inputs, holding and input registers; write a coil, a register, several of each; mask a
//! register; read and write registers in one request), each request answered the moment it is
//! read, in the order it came, with the exceptions where the protocol puts them (an unknown
//! function 01, an address outside a table 02, a quantity, a byte count or a value the function
//! does not take 03), and a unit the server does not play answered 0B, as a gateway answers for a
//! device that is not there. The handle sets and reads the tables on the plant side, by point or
//! by address, inputs included, and holds the knobs a client's failure paths need: every answer
//! late, no answer at all, every connection dropped; dropping it frees the port.
//!
//! Threading follows the bench's OPC UA server: a thread, a current-thread runtime, the runtime's
//! drop as the teardown (every connection closed with it). The tables live behind one lock the
//! connections and the handle share, held for one request at a time.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The port a Modbus/TCP device listens on. On Linux a port below 1024 needs a privilege, so a
/// bench there names another, or 0.
pub const DEFAULT_PORT: u16 = 502;
/// How many exchanges the status keeps.
pub const RECENT: usize = 100;
/// The most addresses a table holds: the protocol's addresses are 16 bits.
pub const MAX_TABLE: u32 = 65_536;

/// The function codes the server answers.
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

/// The exception codes the server sends.
pub mod exception {
    pub const ILLEGAL_FUNCTION: u8 = 0x01;
    pub const ILLEGAL_DATA_ADDRESS: u8 = 0x02;
    pub const ILLEGAL_DATA_VALUE: u8 = 0x03;
    pub const GATEWAY_TARGET_FAILED: u8 = 0x0B;
}

/// One of a device's four tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Table {
    /// Bits a client reads and writes (outputs).
    Coil,
    /// Bits a client only reads (inputs).
    DiscreteInput,
    /// Words a client only reads (measurements).
    InputRegister,
    /// Words a client reads and writes (settings, commands).
    HoldingRegister,
}

impl Table {
    pub const ALL: [Table; 4] = [Table::Coil, Table::DiscreteInput, Table::InputRegister, Table::HoldingRegister];

    /// Whether the table holds bits (coils, discrete inputs) rather than registers.
    pub fn is_bits(self) -> bool {
        matches!(self, Table::Coil | Table::DiscreteInput)
    }

    pub fn label(self) -> &'static str {
        match self {
            Table::Coil => "coil",
            Table::DiscreteInput => "discrete-input",
            Table::InputRegister => "input-register",
            Table::HoldingRegister => "holding-register",
        }
    }

    fn index(self) -> usize {
        match self {
            Table::Coil => 0,
            Table::DiscreteInput => 1,
            Table::InputRegister => 2,
            Table::HoldingRegister => 3,
        }
    }

    /// The bench file's key for the table's size.
    pub fn size_key(self) -> &'static str {
        match self {
            Table::Coil => "coils",
            Table::DiscreteInput => "discrete_inputs",
            Table::InputRegister => "input_registers",
            Table::HoldingRegister => "holding_registers",
        }
    }
}

/// What a point holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointType {
    /// One bit of a coil or a discrete input.
    Bool,
    U16,
    I16,
    /// Two registers.
    U32,
    /// Two registers.
    I32,
    /// Two registers, an IEEE 754 single.
    F32,
}

impl PointType {
    pub fn label(self) -> &'static str {
        match self {
            PointType::Bool => "bool",
            PointType::U16 => "u16",
            PointType::I16 => "i16",
            PointType::U32 => "u32",
            PointType::I32 => "i32",
            PointType::F32 => "f32",
        }
    }

    /// How many addresses of its table the point covers.
    pub fn width(self) -> u32 {
        match self {
            PointType::Bool | PointType::U16 | PointType::I16 => 1,
            PointType::U32 | PointType::I32 | PointType::F32 => 2,
        }
    }
}

/// Which of a two-register point's registers carries the high word. A register's own two bytes
/// are always high byte first: that is the protocol's; the order of the words is the device's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WordOrder {
    /// The register at the point's address holds the high word.
    #[default]
    HighFirst,
    /// The register at the point's address holds the low word.
    LowFirst,
}

/// A named value in one of the tables.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointConfig {
    /// The name the handle reads and sets it by.
    pub name: String,
    pub table: Table,
    /// The protocol's address, from 0: what a request's starting address carries (not the
    /// one-based numbering some manuals print).
    pub address: u16,
    /// `bool` in a bit table, `u16` in a register table, when absent.
    #[serde(rename = "type", default)]
    pub ty: Option<PointType>,
    /// The word order of a two-register point; `high-first` when absent; refused on any other.
    #[serde(default)]
    pub words: Option<WordOrder>,
    /// The initial value as text, parsed per the type; the type's zero when empty.
    #[serde(default)]
    pub value: String,
}

/// The server, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModbusConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// 0 lets the machine choose a free port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// The unit identifier the server plays; absent, it answers every unit.
    #[serde(default)]
    pub unit: Option<u8>,
    /// The most connections held at once; one more is accepted and closed at once.
    #[serde(default)]
    pub max_connections: Option<usize>,
    /// The tables' sizes (addresses 0 to the size less one); absent, a table reaches the end of
    /// its last point, none with no point.
    #[serde(default)]
    pub coils: Option<u32>,
    #[serde(default)]
    pub discrete_inputs: Option<u32>,
    #[serde(default)]
    pub input_registers: Option<u32>,
    #[serde(default)]
    pub holding_registers: Option<u32>,
    #[serde(default)]
    pub points: Vec<PointConfig>,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Default for ModbusConfig {
    fn default() -> ModbusConfig {
        ModbusConfig {
            bind: default_bind(),
            port: default_port(),
            unit: None,
            max_connections: None,
            coils: None,
            discrete_inputs: None,
            input_registers: None,
            holding_registers: None,
            points: Vec::new(),
        }
    }
}

impl ModbusConfig {
    fn size(&self, table: Table) -> Option<u32> {
        match table {
            Table::Coil => self.coils,
            Table::DiscreteInput => self.discrete_inputs,
            Table::InputRegister => self.input_registers,
            Table::HoldingRegister => self.holding_registers,
        }
    }
}

/// A point's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
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
            Value::U16(_) => PointType::U16,
            Value::I16(_) => PointType::I16,
            Value::U32(_) => PointType::U32,
            Value::I32(_) => PointType::I32,
            Value::F32(_) => PointType::F32,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{v}"),
            Value::U16(v) => write!(f, "{v}"),
            Value::I16(v) => write!(f, "{v}"),
            Value::U32(v) => write!(f, "{v}"),
            Value::I32(v) => write!(f, "{v}"),
            Value::F32(v) => write!(f, "{v}"),
        }
    }
}

/// Parse a value as text into a point's type: `bool` one of `true`/`false`/`1`/`0`/`on`/`off`/
/// `yes`/`no` (any case), an integer within its type's range, `f32` a finite number. Empty text is
/// the type's zero.
pub fn parse_value(ty: PointType, text: &str) -> Result<Value, String> {
    let t = text.trim();
    fn int<T: std::str::FromStr>(t: &str, label: &str) -> Result<T, String> {
        t.parse().map_err(|_| format!("`{t}` is not a {label}"))
    }
    if t.is_empty() {
        return Ok(match ty {
            PointType::Bool => Value::Bool(false),
            PointType::U16 => Value::U16(0),
            PointType::I16 => Value::I16(0),
            PointType::U32 => Value::U32(0),
            PointType::I32 => Value::I32(0),
            PointType::F32 => Value::F32(0.0),
        });
    }
    Ok(match ty {
        PointType::Bool => Value::Bool(match t.to_ascii_lowercase().as_str() {
            "false" | "0" | "off" | "no" => false,
            "true" | "1" | "on" | "yes" => true,
            _ => return Err(format!("`{t}` is not a bool")),
        }),
        PointType::U16 => Value::U16(int(t, "u16")?),
        PointType::I16 => Value::I16(int(t, "i16")?),
        PointType::U32 => Value::U32(int(t, "u32")?),
        PointType::I32 => Value::I32(int(t, "i32")?),
        PointType::F32 => {
            let v: f32 = t.parse().map_err(|_| format!("`{t}` is not an f32"))?;
            if !v.is_finite() {
                return Err(format!("`{t}` is not a finite f32"));
            }
            Value::F32(v)
        }
    })
}

/// Why the server did not start, a configuration was refused, or a plant-side call failed.
#[derive(Debug, thiserror::Error)]
pub enum ModbusError {
    /// The configuration is wrong, with its place named.
    #[error("{0}")]
    Config(String),
    #[error("bind {addr}: {source}")]
    Bind { addr: String, source: std::io::Error },
    #[error("local_addr: {0}")]
    LocalAddr(std::io::Error),
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("spawn the server thread: {0}")]
    Thread(std::io::Error),
    #[error("the server thread died on startup")]
    DiedOnStartup,
    #[error("no point `{0}`")]
    NoPoint(String),
    /// A value of another type than the point's.
    #[error("point `{point}` is {expected}, not {given}")]
    WrongType { point: String, expected: &'static str, given: &'static str },
    /// A value that does not parse as the point's type.
    #[error("point `{point}`: {reason}")]
    BadValue { point: String, reason: String },
    /// Addresses past a table's end, or a bit table asked for words and the reverse.
    #[error("{0}")]
    Range(String),
}

/// A point, compiled: where it is and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Point {
    table: Table,
    address: u16,
    ty: PointType,
    words: WordOrder,
}

/// The four tables.
#[derive(Clone, Debug, Default)]
struct Image {
    bits: [Vec<bool>; 2],
    words: [Vec<u16>; 2],
}

impl Image {
    fn sized(sizes: [u32; 4]) -> Image {
        Image {
            bits: [vec![false; sizes[0] as usize], vec![false; sizes[1] as usize]],
            words: [vec![0; sizes[2] as usize], vec![0; sizes[3] as usize]],
        }
    }

    fn bits(&self, t: Table) -> &Vec<bool> {
        &self.bits[t.index()]
    }

    fn bits_mut(&mut self, t: Table) -> &mut Vec<bool> {
        &mut self.bits[t.index()]
    }

    fn words(&self, t: Table) -> &Vec<u16> {
        &self.words[t.index() - 2]
    }

    fn words_mut(&mut self, t: Table) -> &mut Vec<u16> {
        &mut self.words[t.index() - 2]
    }

    fn put(&mut self, p: Point, v: Value) {
        let a = p.address as usize;
        let pair = |x: u32| -> [u16; 2] {
            let (hi, lo) = ((x >> 16) as u16, x as u16);
            match p.words {
                WordOrder::HighFirst => [hi, lo],
                WordOrder::LowFirst => [lo, hi],
            }
        };
        match v {
            Value::Bool(b) => self.bits_mut(p.table)[a] = b,
            Value::U16(x) => self.words_mut(p.table)[a] = x,
            Value::I16(x) => self.words_mut(p.table)[a] = x as u16,
            Value::U32(x) => self.words_mut(p.table)[a..a + 2].copy_from_slice(&pair(x)),
            Value::I32(x) => self.words_mut(p.table)[a..a + 2].copy_from_slice(&pair(x as u32)),
            Value::F32(x) => self.words_mut(p.table)[a..a + 2].copy_from_slice(&pair(x.to_bits())),
        }
    }

    fn take(&self, p: Point) -> Value {
        let a = p.address as usize;
        let joined = || -> u32 {
            let w = &self.words(p.table)[a..a + 2];
            let (hi, lo) = match p.words {
                WordOrder::HighFirst => (w[0], w[1]),
                WordOrder::LowFirst => (w[1], w[0]),
            };
            ((hi as u32) << 16) | lo as u32
        };
        match p.ty {
            PointType::Bool => Value::Bool(self.bits(p.table)[a]),
            PointType::U16 => Value::U16(self.words(p.table)[a]),
            PointType::I16 => Value::I16(self.words(p.table)[a] as i16),
            PointType::U32 => Value::U32(joined()),
            PointType::I32 => Value::I32(joined() as i32),
            PointType::F32 => Value::F32(f32::from_bits(joined())),
        }
    }
}

/// A configuration compiled: the tables' sizes, the points by name, the initial image.
struct Compiled {
    sizes: [u32; 4],
    points: Vec<(String, Point)>,
    image: Image,
}

/// Validate a configuration before anything is started, naming what is wrong: an empty bind, a
/// table larger than the protocol's addresses, a point without a name or with another's, a type
/// its table cannot hold, a word order on a point of one register or a bit, a point past its
/// table's end or over another point, an initial value that does not parse.
pub fn validate(cfg: &ModbusConfig) -> Result<(), ModbusError> {
    compile(cfg).map(|_| ())
}

fn compile(cfg: &ModbusConfig) -> Result<Compiled, ModbusError> {
    let bad = |m: String| ModbusError::Config(m);
    if cfg.bind.trim().is_empty() {
        return Err(bad("modbus.bind is empty".into()));
    }
    if cfg.max_connections == Some(0) {
        return Err(bad("modbus.max_connections is 0: no client could connect".into()));
    }
    for t in Table::ALL {
        if let Some(n) = cfg.size(t)
            && n > MAX_TABLE
        {
            return Err(bad(format!("modbus.{} is {n}, over the {MAX_TABLE} addresses a table can hold", t.size_key())));
        }
    }
    let mut points: Vec<(String, Point)> = Vec::new();
    let mut initial: Vec<Value> = Vec::new();
    let mut ends = [0u32; 4];
    for (i, pc) in cfg.points.iter().enumerate() {
        let place = format!("modbus.points[{i}]");
        if pc.name.trim().is_empty() {
            return Err(bad(format!("{place}: the point has no name")));
        }
        if pc.name != pc.name.trim() {
            return Err(bad(format!("{place}: the name `{}` has surrounding whitespace", pc.name)));
        }
        if points.iter().any(|(n, _)| *n == pc.name) {
            return Err(bad(format!("{place}: two points are named `{}`", pc.name)));
        }
        let place = format!("{place} (`{}`)", pc.name);
        let ty = pc.ty.unwrap_or(if pc.table.is_bits() { PointType::Bool } else { PointType::U16 });
        if pc.table.is_bits() != (ty == PointType::Bool) {
            return Err(bad(if pc.table.is_bits() {
                format!("{place}: a {} holds a bool, not {}", pc.table.label(), ty.label())
            } else {
                format!("{place}: a {} holds a number (u16, i16, u32, i32 or f32), not a bool", pc.table.label())
            }));
        }
        if pc.words.is_some() && ty.width() != 2 {
            return Err(bad(format!("{place}: `words` orders the two registers of a u32, i32 or f32, not a {}", ty.label())));
        }
        let end = pc.address as u32 + ty.width();
        if end > MAX_TABLE {
            return Err(bad(format!("{place}: a {} at {} runs past the last address, 65535", ty.label(), pc.address)));
        }
        let value = parse_value(ty, &pc.value).map_err(|e| bad(format!("{place}.value: {e}")))?;
        let p = Point { table: pc.table, address: pc.address, ty, words: pc.words.unwrap_or_default() };
        for (other, q) in &points {
            let q_end = q.address as u32 + q.ty.width();
            if q.table == p.table && (p.address as u32) < q_end && (q.address as u32) < end {
                return Err(bad(format!("{place}: {} {}..{} overlaps point `{other}`", p.table.label(), p.address, end - 1)));
            }
        }
        let e = &mut ends[pc.table.index()];
        *e = (*e).max(end);
        points.push((pc.name.clone(), p));
        initial.push(value);
    }
    let mut sizes = [0u32; 4];
    for t in Table::ALL {
        sizes[t.index()] = match cfg.size(t) {
            Some(n) => {
                if let Some((name, p)) = points.iter().find(|(_, p)| p.table == t && p.address as u32 + p.ty.width() > n) {
                    return Err(bad(format!(
                        "modbus.points: point `{name}` ({} at {}) is past the end of the {} table ({} = {n})",
                        p.ty.label(),
                        p.address,
                        t.label(),
                        t.size_key()
                    )));
                }
                n
            }
            None => ends[t.index()],
        };
    }
    let mut image = Image::sized(sizes);
    for ((_, p), v) in points.iter().zip(initial) {
        image.put(*p, v);
    }
    Ok(Compiled { sizes, points, image })
}

/// One request and what the server did with it, as the status keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub peer: SocketAddr,
    pub transaction: u16,
    pub unit: u8,
    /// The request's PDU: the function code and its data.
    pub request: Vec<u8>,
    /// The answer's PDU, once it is sent; `None` while it waits out a delay, and for good when the
    /// server withheld it (silent) or its connection was dropped first.
    pub response: Option<Vec<u8>>,
}

/// What the server reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModbusStatus {
    /// Connections open now.
    pub connections: usize,
    /// Connections accepted since the start, the refused ones included.
    pub accepted: u64,
    /// Connections closed at once because `max_connections` were open.
    pub refused: u64,
    /// Requests answered normally, by function code: answers handed to the socket, counted as each
    /// write begins and taken back when the write fails or a drop cuts it. Answers already in the
    /// socket's buffer when a drop's reset discards them stay counted: the server cannot see what a
    /// client received.
    pub answered: BTreeMap<u8, u64>,
    /// Requests answered with an exception, by function code and exception code, counted as
    /// `answered` is.
    pub exceptions: BTreeMap<(u8, u8), u64>,
    /// Requests whose answer the server withheld: read while it was silent, or not yet handed to
    /// the socket when silence began.
    pub unanswered: u64,
    /// Frames dropped without an answer: their protocol identifier was not 0, Modbus's.
    pub dropped: u64,
    /// Connections closed because a frame's length could not be a Modbus frame's.
    pub malformed: u64,
    /// The last exchanges, oldest first, at most [`RECENT`].
    pub recent: VecDeque<Exchange>,
}

struct State {
    image: Image,
    status: ModbusStatus,
    /// Each exchange's own number, beside `status.recent` (popped with it), so an answer sent late
    /// fills its own exchange, never a twin's.
    recent_ids: VecDeque<u64>,
    next_exchange: u64,
    /// The generation of `drop_connections` calls: a connection belongs to the one it was
    /// accepted in, and a call closes every connection of the generations before its own.
    generation: u64,
    /// Connections open, by the generation they were accepted in.
    open: BTreeMap<u64, usize>,
}

struct Shared {
    state: Mutex<State>,
    sizes: [u32; 4],
    unit: Option<u8>,
    max_connections: Option<usize>,
    silent: AtomicBool,
    /// How many times silence was switched on: an answer waiting out a delay is withheld when
    /// silence began meanwhile, even if it ended before the delay did.
    silenced: AtomicU64,
    delay_us: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves a table mid-write at worst: the bench serves on.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A running server. Dropping it stops the server, closes every connection and frees the port.
pub struct ModbusHandle {
    /// The address the server listens on, with the port it got.
    pub addr: SocketAddr,
    pub port: u16,
    shared: Arc<Shared>,
    points: Vec<(String, Point)>,
    restarts: tokio::sync::mpsc::UnboundedSender<std::sync::mpsc::Sender<u64>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A running server's points, apart from its handle.
#[derive(Clone)]
pub struct ModbusPoints {
    shared: Arc<Shared>,
    points: Vec<(String, Point)>,
}

impl ModbusPoints {
    fn point(&self, name: &str) -> Result<Point, ModbusError> {
        self.points.iter().find(|(n, _)| n == name).map(|(_, p)| *p).ok_or_else(|| ModbusError::NoPoint(name.to_string()))
    }

    /// A point's value.
    pub fn get(&self, point: &str) -> Result<Value, ModbusError> {
        let p = self.point(point)?;
        Ok(self.shared.lock().image.take(p))
    }

    /// Sets a point, of its own type.
    pub fn set_value(&self, point: &str, value: Value) -> Result<(), ModbusError> {
        let p = self.point(point)?;
        if value.ty() != p.ty {
            return Err(ModbusError::WrongType { point: point.to_string(), expected: p.ty.label(), given: value.ty().label() });
        }
        self.shared.lock().image.put(p, value);
        Ok(())
    }

    /// A point's type.
    pub fn ty(&self, point: &str) -> Result<PointType, ModbusError> {
        Ok(self.point(point)?.ty)
    }
}

impl std::fmt::Debug for ModbusHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusHandle").field("addr", &self.addr).finish()
    }
}

impl ModbusHandle {
    fn point(&self, name: &str) -> Result<Point, ModbusError> {
        self.points.iter().find(|(n, _)| n == name).map(|(_, p)| *p).ok_or_else(|| ModbusError::NoPoint(name.to_string()))
    }

    /// The names of the points, in the configuration's order.
    pub fn point_names(&self) -> Vec<String> {
        self.points.iter().map(|(n, _)| n.clone()).collect()
    }

    /// The points, readable and settable from another thread while the server runs (reflexo's
    /// models): what [`ModbusHandle::get`] and [`ModbusHandle::set_value`] do, without the handle.
    pub fn points(&self) -> ModbusPoints {
        ModbusPoints { shared: Arc::clone(&self.shared), points: self.points.clone() }
    }

    /// A point's value, read on the plant side.
    pub fn get(&self, point: &str) -> Result<Value, ModbusError> {
        let p = self.point(point)?;
        Ok(self.shared.lock().image.take(p))
    }

    /// Set a point from the plant side, inputs included; what a client reads next.
    pub fn set_value(&self, point: &str, value: Value) -> Result<(), ModbusError> {
        let p = self.point(point)?;
        if value.ty() != p.ty {
            return Err(ModbusError::WrongType { point: point.to_string(), expected: p.ty.label(), given: value.ty().label() });
        }
        self.shared.lock().image.put(p, value);
        Ok(())
    }

    /// Set a point from text parsed per its type, as the bench file writes a value.
    pub fn set(&self, point: &str, text: &str) -> Result<(), ModbusError> {
        let p = self.point(point)?;
        let value = parse_value(p.ty, text).map_err(|reason| ModbusError::BadValue { point: point.to_string(), reason })?;
        self.shared.lock().image.put(p, value);
        Ok(())
    }

    /// Every point's value, in the configuration's order, read at one instant.
    pub fn snapshot(&self) -> Vec<(String, Value)> {
        let state = self.shared.lock();
        self.points.iter().map(|(n, p)| (n.clone(), state.image.take(*p))).collect()
    }

    fn span(&self, table: Table, address: u16, count: usize, bits: bool) -> Result<(usize, usize), ModbusError> {
        if table.is_bits() != bits {
            return Err(ModbusError::Range(format!("the {} table holds {}", table.label(), if table.is_bits() { "bits" } else { "registers" })));
        }
        let size = self.shared.sizes[table.index()] as usize;
        let start = address as usize;
        match start.checked_add(count) {
            Some(end) if end <= size => Ok((start, end)),
            _ => Err(ModbusError::Range(format!(
                "{} {start}..{} is past the table's end ({size} address(es))",
                table.label(),
                start as u128 + count as u128
            ))),
        }
    }

    /// Bits of a coil or discrete-input table, read on the plant side.
    pub fn bits(&self, table: Table, address: u16, count: usize) -> Result<Vec<bool>, ModbusError> {
        let (a, b) = self.span(table, address, count, true)?;
        Ok(self.shared.lock().image.bits(table)[a..b].to_vec())
    }

    /// Set bits of a coil or discrete-input table from the plant side.
    pub fn set_bits(&self, table: Table, address: u16, values: &[bool]) -> Result<(), ModbusError> {
        let (a, b) = self.span(table, address, values.len(), true)?;
        self.shared.lock().image.bits_mut(table)[a..b].copy_from_slice(values);
        Ok(())
    }

    /// Registers of an input or holding table, read on the plant side.
    pub fn registers(&self, table: Table, address: u16, count: usize) -> Result<Vec<u16>, ModbusError> {
        let (a, b) = self.span(table, address, count, false)?;
        Ok(self.shared.lock().image.words(table)[a..b].to_vec())
    }

    /// Set registers of an input or holding table from the plant side.
    pub fn set_registers(&self, table: Table, address: u16, values: &[u16]) -> Result<(), ModbusError> {
        let (a, b) = self.span(table, address, values.len(), false)?;
        self.shared.lock().image.words_mut(table)[a..b].copy_from_slice(values);
        Ok(())
    }

    /// Every answer from now on held for `d` before it is sent, as a slow device answers; zero
    /// answers at once again.
    pub fn set_delay(&self, d: Duration) {
        self.shared.delay_us.store(d.as_micros().min(u64::MAX as u128) as u64, Ordering::SeqCst);
    }

    /// Answer nothing from now on (true), as a device gone dumb with its connection up: a request
    /// read meanwhile, and an answer still waiting out a delay, are never sent; false answers the
    /// next request again.
    pub fn set_silent(&self, silent: bool) {
        // Under the lock an answer's count takes: once this returns, no answer's write begins
        // until silence ends. Silence first, then its epoch.
        let _state = self.shared.lock();
        self.shared.silent.store(silent, Ordering::SeqCst);
        if silent {
            self.shared.silenced.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Close every open connection, as a device's restart does: those the server serves, those
    /// still in the listen queue (a client whose connect returned before the call), and those
    /// blocked on an answer the client does not read; the server keeps listening and a client may
    /// connect again at once. Returns when every connection open at the call is closed (or after
    /// 5 s); one accepted since is not touched.
    pub fn drop_connections(&self) {
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

    pub fn status(&self) -> ModbusStatus {
        self.shared.lock().status.clone()
    }

    fn shutdown_and_join(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Stop the server, close every connection and wait for the port to be released.
    pub fn stop(mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for ModbusHandle {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

/// Start the server. Blocks until it is listening (or failed), so a caller holding a handle knows
/// the port is claimed.
pub fn start(cfg: &ModbusConfig) -> Result<ModbusHandle, ModbusError> {
    let compiled = compile(cfg)?;
    let bind = cfg.bind.trim().to_string();
    let port = cfg.port;
    let shared = Arc::new(Shared {
        state: Mutex::new(State { image: compiled.image, status: ModbusStatus::default(), recent_ids: VecDeque::new(), next_exchange: 0, generation: 0, open: BTreeMap::new() }),
        sizes: compiled.sizes,
        unit: cfg.unit,
        max_connections: cfg.max_connections,
        silent: AtomicBool::new(false),
        silenced: AtomicU64::new(0),
        delay_us: AtomicU64::new(0),
    });
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<SocketAddr, ModbusError>>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (drops_tx, _) = tokio::sync::watch::channel(0u64);
    let (restarts_tx, restarts_rx) = tokio::sync::mpsc::unbounded_channel();
    let server = Arc::clone(&shared);
    let thread = std::thread::Builder::new()
        .name("maquinar-modbus".into())
        .spawn(move || {
            // An answer's delay is a timed wait: the host's timer held fine, as the other parts do.
            let _timer = crate::timer::FineTimer::hold();
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(ModbusError::Runtime(e)));
                    return;
                }
            };
            rt.block_on(async move {
                let addr = format!("{bind}:{port}");
                let listener = match TcpListener::bind((bind.as_str(), port)).await {
                    Ok(l) => l,
                    Err(source) => {
                        let _ = ready_tx.send(Err(ModbusError::Bind { addr, source }));
                        return;
                    }
                };
                match listener.local_addr() {
                    Ok(a) => {
                        if ready_tx.send(Ok(a)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(ModbusError::LocalAddr(e)));
                        return;
                    }
                }
                accept_loop(listener, server, drops_tx, restarts_rx, shutdown_rx).await;
            });
            // Dropping the runtime here closes every connection still open.
        })
        .map_err(ModbusError::Thread)?;
    let addr = ready_rx.recv().map_err(|_| ModbusError::DiedOnStartup)??;
    Ok(ModbusHandle {
        addr,
        port: addr.port(),
        shared,
        points: compiled.points,
        restarts: restarts_tx,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
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
                // connect returned before the call may still sit in the listen queue. Take every
                // one there now, straight from the socket, and close it with the rest.
                let mut queued = 0u64;
                while let Ok((socket, _)) = socket2::SockRef::from(&listener).accept() {
                    drop(socket);
                    queued += 1;
                }
                let ended = {
                    let mut state = shared.lock();
                    state.status.accepted += queued;
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
                    state.status.accepted += 1;
                    if shared.max_connections.is_some_and(|m| state.status.connections >= m) {
                        state.status.refused += 1;
                        // Dropped here: closed at once, as a device with no socket free closes it.
                        continue;
                    }
                    state.status.connections += 1;
                    let g = state.generation;
                    *state.open.entry(g).or_default() += 1;
                    g
                };
                // The connection waits for the watched generation to pass its own, which it sees
                // whether a call came before its subscription or after.
                let gone = drops.subscribe();
                let open = Open { shared: Arc::clone(&shared), generation };
                tokio::spawn(connection(stream, peer, Arc::clone(&shared), gone, open));
            }
        }
    }
}

/// Counts a connection out when its task ends, however it ends.
struct Open {
    shared: Arc<Shared>,
    generation: u64,
}

impl Drop for Open {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.status.connections = state.status.connections.saturating_sub(1);
        if let Some(n) = state.open.get_mut(&self.generation) {
            *n = n.saturating_sub(1);
        }
    }
}

/// The MBAP header's length counts the unit identifier and the PDU: a PDU holds a function code
/// at least and 253 bytes at most.
const MIN_LENGTH: u16 = 2;
const MAX_LENGTH: u16 = 254;

async fn connection(stream: TcpStream, peer: SocketAddr, shared: Arc<Shared>, mut gone: tokio::sync::watch::Receiver<u64>, open: Open) {
    let generation = open.generation;
    let _open = open;
    // Each answer goes out whole the moment it is ready.
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    loop {
        let mut header = [0u8; 7];
        tokio::select! {
            r = rd.read_exact(&mut header) => if r.is_err() { return },
            _ = ended(&mut gone, generation) => return,
        }
        let transaction = u16::from_be_bytes([header[0], header[1]]);
        let protocol = u16::from_be_bytes([header[2], header[3]]);
        let length = u16::from_be_bytes([header[4], header[5]]);
        let unit = header[6];
        if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
            // The stream cannot be read on from a length no frame has: close it.
            shared.lock().status.malformed += 1;
            return;
        }
        let mut pdu = vec![0u8; length as usize - 1];
        tokio::select! {
            r = rd.read_exact(&mut pdu) => if r.is_err() { return },
            _ = ended(&mut gone, generation) => return,
        }
        if protocol != 0 {
            shared.lock().status.dropped += 1;
            continue;
        }
        // The silence count is read before the silence: a switch between the two is seen by one.
        let epoch = shared.silenced.load(Ordering::SeqCst);
        let silent = shared.silent.load(Ordering::SeqCst);
        let response = {
            let mut state = shared.lock();
            let response = if silent {
                state.status.unanswered += 1;
                None
            } else {
                // The request takes effect now, as a device does what it is asked before it
                // answers; the answer is counted when it is sent.
                let function = pdu[0];
                let answer = if shared.unit.is_some_and(|u| u != unit) {
                    Err(exception::GATEWAY_TARGET_FAILED)
                } else {
                    answer(&mut state.image, &pdu)
                };
                Some(match answer {
                    Ok(r) => r,
                    Err(code) => vec![function | 0x80, code],
                })
            };
            if state.status.recent.len() == RECENT {
                state.status.recent.pop_front();
                state.recent_ids.pop_front();
            }
            state.status.recent.push_back(Exchange { peer, transaction, unit, request: pdu.clone(), response: None });
            let id = state.next_exchange;
            state.next_exchange += 1;
            state.recent_ids.push_back(id);
            response.map(|r| (r, id))
        };
        let Some((response, exchange)) = response else { continue };
        let delay = Duration::from_micros(shared.delay_us.load(Ordering::SeqCst));
        if !delay.is_zero() {
            // The delay, raced against a drop and against the client's own end: a client that
            // closes (a FIN, as its timeout does) is gone, its answer counted as none and its place
            // freed at once. Requests it pipelined behind are read after the delay, as before.
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            let mut waiting_data = false;
            loop {
                let mut probe = [0u8; 1];
                tokio::select! {
                    _ = &mut sleep => break,
                    _ = ended(&mut gone, generation) => return,
                    r = rd.peek(&mut probe), if !waiting_data => match r {
                        Ok(0) | Err(_) => return,
                        Ok(_) => waiting_data = true,
                    },
                }
            }
        }
        // Counted, and shown in its exchange, as its write begins: a client reading its answer finds
        // it counted. A write that fails, or that a drop cuts before it is done, takes it back.
        let count = |state: &mut State, by: i64| {
            fn add<K: Ord>(map: &mut BTreeMap<K, u64>, key: K, by: i64) {
                let n = map.entry(key).or_default();
                *n = n.saturating_add_signed(by);
                map.retain(|_, n| *n > 0);
            }
            if response[0] & 0x80 == 0 {
                add(&mut state.status.answered, response[0], by);
            } else {
                add(&mut state.status.exceptions, (pdu[0], response[1]), by);
            }
            if let Some(i) = state.recent_ids.iter().position(|id| *id == exchange) {
                state.status.recent[i].response = if by > 0 { Some(response.clone()) } else { None };
            }
        };
        {
            // Silence is decided under the lock `set_silent` takes: gone dumb since the request was
            // read, even for a moment, the answer is never handed to the socket.
            let mut state = shared.lock();
            if shared.silent.load(Ordering::SeqCst) || shared.silenced.load(Ordering::SeqCst) != epoch {
                state.status.unanswered += 1;
                continue;
            }
            count(&mut state, 1);
        }
        let mut frame = Vec::with_capacity(7 + response.len());
        frame.extend_from_slice(&transaction.to_be_bytes());
        frame.extend_from_slice(&0u16.to_be_bytes());
        frame.extend_from_slice(&(response.len() as u16 + 1).to_be_bytes());
        frame.push(unit);
        frame.extend_from_slice(&response);
        // A client that does not read leaves the write waiting: the drop does not wait for it.
        let written = tokio::select! {
            r = wr.write_all(&frame) => r.is_ok(),
            _ = ended(&mut gone, generation) => false,
        };
        if !written {
            count(&mut shared.lock(), -1);
            return;
        }
    }
}

/// Resolves when a `drop_connections` call has ended `generation`, or the handle is gone.
async fn ended(gone: &mut tokio::sync::watch::Receiver<u64>, generation: u64) {
    if gone.wait_for(|g| *g > generation).await.is_err() {
        // The sender lives as long as the handle: gone means the server is stopping, and the
        // runtime's drop ends the connection. Never resolve meanwhile.
        std::future::pending::<()>().await;
    }
}

fn be(pdu: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([pdu[at], pdu[at + 1]])
}

/// The range `[address, address + quantity)` inside a table of `size`, or exception 02.
fn inside(size: usize, address: u16, quantity: u16) -> Result<std::ops::Range<usize>, u8> {
    let start = address as usize;
    let end = start + quantity as usize;
    if end > size {
        return Err(exception::ILLEGAL_DATA_ADDRESS);
    }
    Ok(start..end)
}

fn pack(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Answer one request PDU against the tables: the response PDU, or the exception code. The checks
/// run in the protocol's order: the function, then the request's length, quantity, byte count and
/// value (03), then the addresses (02).
fn answer(image: &mut Image, pdu: &[u8]) -> Result<Vec<u8>, u8> {
    use exception::*;
    use function::*;
    let f = pdu[0];
    let len = |n: usize| if pdu.len() == n { Ok(()) } else { Err(ILLEGAL_DATA_VALUE) };
    match f {
        READ_COILS | READ_DISCRETE_INPUTS => {
            len(5)?;
            let (address, quantity) = (be(pdu, 1), be(pdu, 3));
            if !(1..=2000).contains(&quantity) {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let table = if f == READ_COILS { Table::Coil } else { Table::DiscreteInput };
            let range = inside(image.bits(table).len(), address, quantity)?;
            let packed = pack(&image.bits(table)[range]);
            let mut out = vec![f, packed.len() as u8];
            out.extend(packed);
            Ok(out)
        }
        READ_HOLDING_REGISTERS | READ_INPUT_REGISTERS => {
            len(5)?;
            let (address, quantity) = (be(pdu, 1), be(pdu, 3));
            if !(1..=125).contains(&quantity) {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let table = if f == READ_HOLDING_REGISTERS { Table::HoldingRegister } else { Table::InputRegister };
            let range = inside(image.words(table).len(), address, quantity)?;
            let mut out = vec![f, (quantity * 2) as u8];
            for w in &image.words(table)[range] {
                out.extend_from_slice(&w.to_be_bytes());
            }
            Ok(out)
        }
        WRITE_SINGLE_COIL => {
            len(5)?;
            let (address, value) = (be(pdu, 1), be(pdu, 3));
            let on = match value {
                0xFF00 => true,
                0x0000 => false,
                _ => return Err(ILLEGAL_DATA_VALUE),
            };
            let range = inside(image.bits(Table::Coil).len(), address, 1)?;
            image.bits_mut(Table::Coil)[range.start] = on;
            Ok(pdu.to_vec())
        }
        WRITE_SINGLE_REGISTER => {
            len(5)?;
            let (address, value) = (be(pdu, 1), be(pdu, 3));
            let range = inside(image.words(Table::HoldingRegister).len(), address, 1)?;
            image.words_mut(Table::HoldingRegister)[range.start] = value;
            Ok(pdu.to_vec())
        }
        WRITE_MULTIPLE_COILS => {
            if pdu.len() < 6 {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let (address, quantity, count) = (be(pdu, 1), be(pdu, 3), pdu[5] as usize);
            if !(1..=0x07B0).contains(&quantity) || count != (quantity as usize).div_ceil(8) || pdu.len() != 6 + count {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let range = inside(image.bits(Table::Coil).len(), address, quantity)?;
            let data = &pdu[6..];
            for (i, slot) in image.bits_mut(Table::Coil)[range].iter_mut().enumerate() {
                *slot = data[i / 8] & (1 << (i % 8)) != 0;
            }
            Ok(pdu[..5].to_vec())
        }
        WRITE_MULTIPLE_REGISTERS => {
            if pdu.len() < 6 {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let (address, quantity, count) = (be(pdu, 1), be(pdu, 3), pdu[5] as usize);
            if !(1..=0x007B).contains(&quantity) || count != quantity as usize * 2 || pdu.len() != 6 + count {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let range = inside(image.words(Table::HoldingRegister).len(), address, quantity)?;
            for (i, slot) in image.words_mut(Table::HoldingRegister)[range].iter_mut().enumerate() {
                *slot = be(pdu, 6 + 2 * i);
            }
            Ok(pdu[..5].to_vec())
        }
        MASK_WRITE_REGISTER => {
            len(7)?;
            let (address, and, or) = (be(pdu, 1), be(pdu, 3), be(pdu, 5));
            let range = inside(image.words(Table::HoldingRegister).len(), address, 1)?;
            let slot = &mut image.words_mut(Table::HoldingRegister)[range.start];
            *slot = (*slot & and) | (or & !and);
            Ok(pdu.to_vec())
        }
        READ_WRITE_MULTIPLE_REGISTERS => {
            if pdu.len() < 10 {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let (read_at, read_n, write_at, write_n, count) = (be(pdu, 1), be(pdu, 3), be(pdu, 5), be(pdu, 7), pdu[9] as usize);
            if !(1..=0x007D).contains(&read_n) || !(1..=0x0079).contains(&write_n) || count != write_n as usize * 2 || pdu.len() != 10 + count {
                return Err(ILLEGAL_DATA_VALUE);
            }
            let size = image.words(Table::HoldingRegister).len();
            let reads = inside(size, read_at, read_n)?;
            let writes = inside(size, write_at, write_n)?;
            // The write is done before the read, as the protocol has it.
            for (i, slot) in image.words_mut(Table::HoldingRegister)[writes].iter_mut().enumerate() {
                *slot = be(pdu, 10 + 2 * i);
            }
            let mut out = vec![f, (read_n * 2) as u8];
            for w in &image.words(Table::HoldingRegister)[reads] {
                out.extend_from_slice(&w.to_be_bytes());
            }
            Ok(out)
        }
        _ => Err(ILLEGAL_FUNCTION),
    }
}
