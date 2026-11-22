//! The adapter as the bench file declares it, and its check.

use serde::Deserialize;

use super::EnipError;

/// The encapsulation port, TCP and UDP.
pub const DEFAULT_PORT: u16 = 44818;
/// The Class 1 I/O port, UDP.
pub const DEFAULT_IO_PORT: u16 = 2222;
/// The most data one assembly holds here: what a Large_Forward_Open's 16-bit size can carry less the
/// sequence count and a run/idle header.
pub const MAX_ASSEMBLY: u16 = 65_000;
/// The most data a Forward_Open (not large) carries: its 9-bit size, 511, less the sequence count
/// and a run/idle header.
pub const MAX_SMALL: u16 = 505;

/// The adapter, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnipConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// The encapsulation port, TCP for sessions and UDP for a ListIdentity broadcast; 0 lets the
    /// machine choose (the same port for both).
    #[serde(default = "default_port")]
    pub port: u16,
    /// The Class 1 I/O port; 0 lets the machine choose, the port then given to each originator in
    /// the Forward_Open's reply.
    #[serde(default = "default_io_port")]
    pub io_port: u16,
    #[serde(default)]
    pub identity: Identity,
    /// The least and the most RPI accepted, in microseconds.
    #[serde(default = "default_rpi_min")]
    pub rpi_min_us: u32,
    #[serde(default = "default_rpi_max")]
    pub rpi_max_us: u32,
    #[serde(default)]
    pub max_sessions: Option<usize>,
    /// The most Class 1 connections held at once.
    #[serde(default)]
    pub max_connections: Option<usize>,
    #[serde(default)]
    pub assemblies: Vec<AssemblyConfig>,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub points: Vec<PointConfig>,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_io_port() -> u16 {
    DEFAULT_IO_PORT
}
fn default_rpi_min() -> u32 {
    1_000
}
fn default_rpi_max() -> u32 {
    10_000_000
}

impl Default for EnipConfig {
    fn default() -> EnipConfig {
        EnipConfig {
            bind: default_bind(),
            port: default_port(),
            io_port: default_io_port(),
            identity: Identity::default(),
            rpi_min_us: default_rpi_min(),
            rpi_max_us: default_rpi_max(),
            max_sessions: None,
            max_connections: None,
            assemblies: Vec::new(),
            connections: Vec::new(),
            points: Vec::new(),
        }
    }
}

/// What the Identity object and ListIdentity report.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default)]
    pub vendor_id: u16,
    /// 12: a communications adapter.
    #[serde(default = "default_device_type")]
    pub device_type: u16,
    #[serde(default)]
    pub product_code: u16,
    /// `major.minor`.
    #[serde(default = "default_revision")]
    pub revision: String,
    #[serde(default)]
    pub serial: u32,
    #[serde(default = "default_product_name")]
    pub product_name: String,
}

fn default_device_type() -> u16 {
    12
}
fn default_revision() -> String {
    "1.1".into()
}
fn default_product_name() -> String {
    "veredas bench adapter".into()
}

impl Default for Identity {
    fn default() -> Identity {
        Identity {
            vendor_id: 0,
            device_type: default_device_type(),
            product_code: 0,
            revision: default_revision(),
            serial: 0,
            product_name: default_product_name(),
        }
    }
}

impl Identity {
    /// The revision as its two bytes; an error names what is wrong.
    pub fn revision_bytes(&self) -> Result<(u8, u8), String> {
        let (a, b) = self.revision.split_once('.').ok_or_else(|| format!("`{}` is not major.minor", self.revision))?;
        let major: u8 = a.parse().map_err(|_| format!("`{}` is not major.minor", self.revision))?;
        let minor: u8 = b.parse().map_err(|_| format!("`{}` is not major.minor", self.revision))?;
        if major == 0 || major > 127 || minor == 0 {
            return Err(format!("`{}`: a major revision is 1 to 127 and a minor 1 to 255", self.revision));
        }
        Ok((major, minor))
    }
}

/// An assembly: an instance of the assembly object, its data a run of bytes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyConfig {
    pub instance: u16,
    pub size: u16,
}

/// What a connection's data carries before its bytes, besides the sequence count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataHeader {
    /// A 32-bit run/idle header.
    RunIdle,
    /// Nothing: the data alone.
    Modeless,
}

/// A Class 1 connection the adapter accepts: the assemblies its path names.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    /// The configuration assembly the path names first.
    pub config: u16,
    /// The assembly the adapter consumes (O→T, the originator's outputs).
    pub output: u16,
    /// The assembly the adapter produces (T→O, the originator's inputs).
    pub input: u16,
    /// What the consumed data carries; `run-idle` when absent.
    #[serde(default)]
    pub output_header: Option<DataHeader>,
    /// What the produced data carries; `modeless` when absent.
    #[serde(default)]
    pub input_header: Option<DataHeader>,
}

/// What a point holds, little-endian as CIP carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointType {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
}

impl PointType {
    pub fn label(self) -> &'static str {
        match self {
            PointType::Bool => "bool",
            PointType::U8 => "u8",
            PointType::I8 => "i8",
            PointType::U16 => "u16",
            PointType::I16 => "i16",
            PointType::U32 => "u32",
            PointType::I32 => "i32",
            PointType::F32 => "f32",
        }
    }

    /// Its width in bytes (a bool takes one bit of one byte).
    pub fn width(self) -> u16 {
        match self {
            PointType::Bool | PointType::U8 | PointType::I8 => 1,
            PointType::U16 | PointType::I16 => 2,
            PointType::U32 | PointType::I32 | PointType::F32 => 4,
        }
    }
}

/// A named value in an assembly's bytes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointConfig {
    pub name: String,
    pub assembly: u16,
    /// The offset of its first byte.
    pub byte: u16,
    /// A bool's bit in its byte, 0 to 7; only a bool takes one, and a bool needs one.
    #[serde(default)]
    pub bit: Option<u8>,
    #[serde(rename = "type", default)]
    pub ty: Option<PointType>,
    #[serde(default)]
    pub value: String,
}

/// A compiled configuration's check, before anything binds.
pub(super) fn check(cfg: &EnipConfig) -> Result<(), EnipError> {
    let bad = |m: String| EnipError::Config(m);
    if cfg.bind.trim().is_empty() {
        return Err(bad("ethernetip.bind is empty".into()));
    }
    cfg.identity.revision_bytes().map_err(|e| bad(format!("ethernetip.identity.revision: {e}")))?;
    if cfg.identity.product_name.len() > 32 || !cfg.identity.product_name.is_ascii() {
        return Err(bad("ethernetip.identity.product_name: at most 32 ASCII characters".into()));
    }
    if cfg.rpi_min_us == 0 || cfg.rpi_min_us > cfg.rpi_max_us {
        return Err(bad(format!("ethernetip: the RPI range {}..={} us is empty", cfg.rpi_min_us, cfg.rpi_max_us)));
    }
    if cfg.max_sessions == Some(0) || cfg.max_connections == Some(0) {
        return Err(bad("ethernetip: a limit of 0 lets nobody in".into()));
    }
    for (i, a) in cfg.assemblies.iter().enumerate() {
        if a.instance == 0 {
            return Err(bad(format!("ethernetip.assemblies[{i}]: instance 0 is the class, not an assembly")));
        }
        if cfg.assemblies[..i].iter().any(|b| b.instance == a.instance) {
            return Err(bad(format!("ethernetip.assemblies[{i}]: instance {} is declared twice", a.instance)));
        }
        if a.size > MAX_ASSEMBLY {
            return Err(bad(format!("ethernetip.assemblies[{i}]: {} bytes, over {MAX_ASSEMBLY}", a.size)));
        }
    }
    let assembly = |n: u16| cfg.assemblies.iter().find(|a| a.instance == n);
    for (i, c) in cfg.connections.iter().enumerate() {
        for (what, n) in [("config", c.config), ("output", c.output), ("input", c.input)] {
            if assembly(n).is_none() {
                return Err(bad(format!("ethernetip.connections[{i}].{what}: no assembly {n}")));
            }
        }
        if c.output == c.input {
            return Err(bad(format!("ethernetip.connections[{i}]: assembly {} cannot be both consumed and produced", c.output)));
        }
        if cfg.connections[..i].iter().any(|d| d.config == c.config && d.output == c.output && d.input == c.input) {
            return Err(bad(format!("ethernetip.connections[{i}]: the same three assemblies twice")));
        }
    }
    let mut taken: Vec<(u16, u16, u8, &str)> = Vec::new();
    for (i, p) in cfg.points.iter().enumerate() {
        let place = format!("ethernetip.points[{i}]");
        if p.name.trim().is_empty() || p.name != p.name.trim() {
            return Err(bad(format!("{place}: a point's name is not empty and has no surrounding whitespace")));
        }
        if cfg.points[..i].iter().any(|q| q.name == p.name) {
            return Err(bad(format!("{place}: two points are named `{}`", p.name)));
        }
        let place = format!("{place} (`{}`)", p.name);
        let ty = p.ty.unwrap_or(if p.bit.is_some() { PointType::Bool } else { PointType::U8 });
        let a = assembly(p.assembly).ok_or_else(|| bad(format!("{place}: no assembly {}", p.assembly)))?;
        match (ty, p.bit) {
            (PointType::Bool, None) => return Err(bad(format!("{place}: a bool names its bit"))),
            (PointType::Bool, Some(b)) if b > 7 => return Err(bad(format!("{place}: bit {b} is not 0 to 7"))),
            (t, Some(_)) if t != PointType::Bool => return Err(bad(format!("{place}: only a bool takes a bit"))),
            _ => {}
        }
        if p.byte as u32 + ty.width() as u32 > a.size as u32 {
            return Err(bad(format!("{place}: a {} at byte {} runs past assembly {}'s {} byte(s)", ty.label(), p.byte, a.instance, a.size)));
        }
        super::parse_value(ty, &p.value).map_err(|e| bad(format!("{place}.value: {e}")))?;
        // Two points of one assembly overlap when their bytes do, two bools only on one bit.
        for (asm, byte, width_or_bit, other) in &taken {
            if *asm != p.assembly {
                continue;
            }
            let other_point = cfg.points.iter().find(|q| q.name == *other);
            let other_bit = other_point.and_then(|q| q.bit);
            let (lo, hi) = (p.byte, p.byte + ty.width());
            let (olo, ohi) = (*byte, *byte + *width_or_bit as u16);
            let bytes_meet = lo < ohi && olo < hi;
            let both_bits = p.bit.is_some() && other_bit.is_some();
            if bytes_meet && (!both_bits || p.bit == other_bit) {
                return Err(bad(format!("{place}: it overlaps point `{other}`")));
            }
        }
        taken.push((p.assembly, p.byte, ty.width() as u8, p.name.as_str()));
    }
    Ok(())
}
