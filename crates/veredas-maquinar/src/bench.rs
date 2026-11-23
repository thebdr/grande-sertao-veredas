//! The bench file: one versioned TOML file for the whole plant a consumer meets on the bench
//! (`docs/bench-schema.md`), and [`Bench::start`], which runs every part of it behind one handle.
//!
//! A bench file lists its parts by name: log emitters ([`crate::emit`]), OPC UA servers
//! ([`crate::opc`]), Modbus servers ([`crate::modbus`]), EtherNet/IP adapters
//! ([`crate::ethernetip`]), PROFINET devices ([`crate::profinet`]), emulated EtherCAT segments
//! ([`crate::ethercat`]), scenes ([`crate::fisica`]) and their models ([`crate::reflexo`]), and
//! telegram channels
//! ([`crate::channel`]), each channel pointing at the
//! device's own files kept beside the bench file (core's channel file, the catalog, and, when a
//! controller rides the channel, its rules file). Every referenced file is read and checked when
//! the bench file loads, so a bench that loads is a bench every part of which can start; what
//! `check` finds wrong is named with its place, and nothing binds, opens or spawns before it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use veredas_core::schema::HeaderError;
use veredas_core::telegram::Role;
use veredas_core::telegram::catalog::{self, Catalog};
use veredas_core::telegram::schema::{self as channel_schema, ChannelSpec};

use crate::channel::{self, BenchChannel, ChannelConfig, ChannelError, ChannelStatus, CyclicMessage};
use crate::controller::{self, Fields, Place, Rules};
use crate::emit::{self, EmitterConfig, EmitterHandle, EmitterStatus, SpawnError};
use crate::ethercat::{self, EthercatConfig, EthercatError, EthercatHandle, EthercatStatus};
use crate::ethernetip::{self, EnipConfig, EnipError, EnipHandle, EnipStatus};
use crate::fisica::{self, FisicaError, FisicaHandle, FisicaStatus, Scene};
use crate::reflexo::{self, PointKind, Points, ReflexoConfig, ReflexoError, ReflexoHandle, ReflexoStatus};
use crate::modbus::{self, ModbusConfig, ModbusError, ModbusHandle, ModbusStatus};
use crate::profinet::{self, ProfinetConfig, ProfinetError, ProfinetHandle, ProfinetStatus};
use crate::opc::{self, OpcConfig, OpcError, OpcHandle, OpcStatus};

/// The format name every bench file states.
pub const FORMAT: &str = "veredas-bench";
/// The one version this reader knows.
pub const VERSION: u32 = 1;

/// Why a bench file was refused, or a part did not start.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("not a bench file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `{FORMAT}`")]
    WrongFormat(String),
    #[error("version {0} of `{FORMAT}` is not known here (this reader knows {VERSION})")]
    UnknownVersion(u32),
    /// The file is wrong, with its place named.
    #[error("{0}")]
    Invalid(String),
    /// A file the bench file points at could not be read.
    #[error("{place}: read {path}: {source}")]
    Read { place: String, path: PathBuf, source: std::io::Error },
    #[error("emitter `{name}`: {source}")]
    Emitter { name: String, source: SpawnError },
    #[error("opc `{name}`: {source}")]
    Opc { name: String, source: OpcError },
    #[error("modbus `{name}`: {source}")]
    Modbus { name: String, source: ModbusError },
    #[error("ethernetip `{name}`: {source}")]
    Enip { name: String, source: EnipError },
    #[error("profinet `{name}`: {source}")]
    Profinet { name: String, source: ProfinetError },
    #[error("ethercat `{name}`: {source}")]
    Ethercat { name: String, source: EthercatError },
    #[error("fisica `{name}`: {source}")]
    Fisica { name: String, source: FisicaError },
    #[error("reflexo `{name}`: {source}")]
    Reflexo { name: String, source: ReflexoError },
    #[error("channel `{name}`: {source}")]
    Channel { name: String, source: ChannelError },
}

fn invalid(msg: impl Into<String>) -> BenchError {
    BenchError::Invalid(msg.into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default = "default_out_dir")]
    out_dir: String,
    #[serde(default)]
    emitter: Vec<EmitterConfig>,
    #[serde(default)]
    opc: Vec<NamedOpc>,
    #[serde(default)]
    modbus: Vec<NamedModbus>,
    #[serde(default)]
    ethernetip: Vec<NamedEnip>,
    #[serde(default)]
    profinet: Vec<NamedProfinet>,
    #[serde(default)]
    ethercat: Vec<NamedEthercat>,
    #[serde(default)]
    fisica: Vec<NamedFisica>,
    #[serde(default)]
    reflexo: Vec<NamedReflexo>,
    #[serde(default)]
    channel: Vec<ChannelFile>,
}

fn default_out_dir() -> String {
    ".".into()
}

/// An OPC UA server with the name the bench calls it by, and the server's own keys beside
/// (spelled out, since serde's `flatten` would let a misspelled key through).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedOpc {
    name: String,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<opc::Credentials>,
    #[serde(default)]
    max_nodes_per_read: Option<usize>,
    #[serde(default)]
    max_sessions: Option<usize>,
    #[serde(default)]
    tags: Vec<opc::TagConfig>,
    #[serde(default)]
    reactions: Vec<opc::Reaction>,
}

impl NamedOpc {
    fn server(self) -> (String, OpcConfig) {
        let defaults = OpcConfig::default();
        (
            self.name,
            OpcConfig {
                bind: self.bind.unwrap_or(defaults.bind),
                port: self.port.unwrap_or(defaults.port),
                user: self.user,
                max_nodes_per_read: self.max_nodes_per_read,
                max_sessions: self.max_sessions,
                tags: self.tags,
                reactions: self.reactions,
            },
        )
    }
}

/// A Modbus server with the name the bench calls it by, and the server's own keys beside
/// (spelled out, as the OPC UA server's are).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedModbus {
    name: String,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    unit: Option<u8>,
    #[serde(default)]
    max_connections: Option<usize>,
    #[serde(default)]
    coils: Option<u32>,
    #[serde(default)]
    discrete_inputs: Option<u32>,
    #[serde(default)]
    input_registers: Option<u32>,
    #[serde(default)]
    holding_registers: Option<u32>,
    #[serde(default)]
    points: Vec<modbus::PointConfig>,
}

impl NamedModbus {
    fn server(self) -> (String, ModbusConfig) {
        let defaults = ModbusConfig::default();
        (
            self.name,
            ModbusConfig {
                bind: self.bind.unwrap_or(defaults.bind),
                port: self.port.unwrap_or(defaults.port),
                unit: self.unit,
                max_connections: self.max_connections,
                coils: self.coils,
                discrete_inputs: self.discrete_inputs,
                input_registers: self.input_registers,
                holding_registers: self.holding_registers,
                points: self.points,
            },
        )
    }
}

/// An EtherNet/IP adapter with the name the bench calls it by, the adapter's own keys beside.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedEnip {
    name: String,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    io_port: Option<u16>,
    #[serde(default)]
    identity: Option<ethernetip::Identity>,
    #[serde(default)]
    rpi_min_us: Option<u32>,
    #[serde(default)]
    rpi_max_us: Option<u32>,
    #[serde(default)]
    max_sessions: Option<usize>,
    #[serde(default)]
    max_connections: Option<usize>,
    #[serde(default)]
    assemblies: Vec<ethernetip::AssemblyConfig>,
    #[serde(default)]
    connections: Vec<ethernetip::ConnectionConfig>,
    #[serde(default)]
    points: Vec<ethernetip::PointConfig>,
}

impl NamedEnip {
    fn adapter(self) -> (String, EnipConfig) {
        let d = EnipConfig::default();
        (
            self.name,
            EnipConfig {
                bind: self.bind.unwrap_or(d.bind),
                port: self.port.unwrap_or(d.port),
                io_port: self.io_port.unwrap_or(d.io_port),
                identity: self.identity.unwrap_or(d.identity),
                rpi_min_us: self.rpi_min_us.unwrap_or(d.rpi_min_us),
                rpi_max_us: self.rpi_max_us.unwrap_or(d.rpi_max_us),
                max_sessions: self.max_sessions,
                max_connections: self.max_connections,
                assemblies: self.assemblies,
                connections: self.connections,
                points: self.points,
            },
        )
    }
}

/// A PROFINET device with the name the bench calls it by, the device's own keys beside.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedProfinet {
    name: String,
    interface: String,
    #[serde(default)]
    station: Option<String>,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    vendor_id: Option<u16>,
    #[serde(default)]
    device_id: Option<u16>,
    #[serde(default)]
    ip: Option<profinet::IpConfig>,
    #[serde(default)]
    hello: Option<bool>,
    #[serde(default)]
    lldp_interval_ms: Option<u64>,
    #[serde(default)]
    port: Option<String>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    serial_number: Option<String>,
}

impl NamedProfinet {
    fn device(self) -> (String, ProfinetConfig) {
        let d: ProfinetConfig = ProfinetConfig::new(&self.interface);
        (
            self.name,
            ProfinetConfig {
                interface: self.interface,
                station: self.station.unwrap_or(d.station),
                alias: self.alias,
                vendor: self.vendor.unwrap_or(d.vendor),
                vendor_id: self.vendor_id.unwrap_or(d.vendor_id),
                device_id: self.device_id.unwrap_or(d.device_id),
                ip: self.ip,
                hello: self.hello.unwrap_or(d.hello),
                lldp_interval_ms: self.lldp_interval_ms.unwrap_or(d.lldp_interval_ms),
                port: self.port.unwrap_or(d.port),
                seed: self.seed,
                order_id: self.order_id.unwrap_or(d.order_id),
                serial_number: self.serial_number.unwrap_or(d.serial_number),
            },
        )
    }
}

/// An emulated EtherCAT segment with the name the bench calls it by, its own keys beside.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedEthercat {
    name: String,
    interface: String,
    #[serde(default)]
    devices: Vec<ethercat::DeviceConfig>,
}

/// A scene with the name the bench calls it by: its file beside the bench file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedFisica {
    name: String,
    scene: String,
}

/// A part's models with the name the bench calls them by, their keys beside.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedReflexo {
    name: String,
    #[serde(default)]
    scene: Option<String>,
    #[serde(default)]
    drive: Vec<reflexo::Drive>,
    #[serde(default)]
    conveyor: Vec<reflexo::Conveyor>,
    #[serde(default)]
    sensor: Vec<reflexo::SensorModel>,
    #[serde(default)]
    actuator: Vec<reflexo::Actuator>,
}

/// A channel as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelFile {
    pub name: String,
    pub role: RoleFile,
    /// The address to bind (listen) or the host to dial (connect).
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    /// Core's channel file, relative to the bench file.
    pub channel: String,
    /// The catalog file, relative to the bench file.
    pub catalog: String,
    /// The controller rules file, relative to the bench file; none for a channel that only
    /// sends its own messages.
    #[serde(default)]
    pub controller: Option<String>,
    #[serde(default)]
    pub messages: Vec<CyclicMessage>,
}

fn default_host() -> String {
    "127.0.0.1".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleFile {
    Listen,
    Connect,
}

/// A bench file read and checked, every referenced file loaded: what [`Bench::start`] runs.
pub struct BenchSpec {
    pub seed: Option<u64>,
    /// Where the emitters write, resolved against the bench file's folder.
    pub out_dir: PathBuf,
    pub emitters: Vec<EmitterConfig>,
    pub servers: Vec<(String, OpcConfig)>,
    pub modbus: Vec<(String, ModbusConfig)>,
    pub ethernetip: Vec<(String, EnipConfig)>,
    pub profinet: Vec<(String, ProfinetConfig)>,
    pub ethercat: Vec<(String, EthercatConfig)>,
    pub fisica: Vec<FisicaPart>,
    pub reflexo: Vec<(String, ReflexoConfig)>,
    pub channels: Vec<ChannelPart>,
}

/// A scene of the bench, its file read.
pub struct FisicaPart {
    pub name: String,
    pub path: PathBuf,
    pub scene: Scene,
}

/// A channel of the bench, its files loaded.
pub struct ChannelPart {
    pub name: String,
    pub role: Role,
    pub spec: ChannelSpec,
    pub catalog: Catalog,
    pub controller: Option<Rules>,
    pub messages: Vec<CyclicMessage>,
}

impl BenchSpec {
    /// One line per part: what `--check` prints.
    pub fn plan(&self) -> Vec<String> {
        let mut out = Vec::new();
        for e in &self.emitters {
            out.push(format!("emitter `{}`: `{}` every {} ms into {}", e.name, e.file, e.interval_ms, self.out_dir.display()));
        }
        for (name, s) in &self.servers {
            out.push(format!("opc `{name}`: opc.tcp://{}:{}/ with {} tag(s), {} reaction(s)", s.bind, s.port, s.tags.len(), s.reactions.len()));
        }
        for (name, m) in &self.modbus {
            let unit = m.unit.map(|u| format!("unit {u}")).unwrap_or_else(|| "every unit".into());
            out.push(format!("modbus `{name}`: {}:{} answering {unit} with {} point(s)", m.bind, m.port, m.points.len()));
        }
        for (name, e) in &self.ethernetip {
            out.push(format!(
                "ethernetip `{name}`: {}:{} (class 1 on {}) with {} assembly(ies), {} connection(s), {} point(s)",
                e.bind,
                e.port,
                e.io_port,
                e.assemblies.len(),
                e.connections.len(),
                e.points.len()
            ));
        }
        for (name, p) in &self.profinet {
            let station = if p.station.is_empty() { "no station name".to_string() } else { format!("station `{}`", p.station) };
            out.push(format!("profinet `{name}`: on {} as {station}", p.interface));
        }
        for (name, e) in &self.ethercat {
            out.push(format!("ethercat `{name}`: on {} with {} device(s)", e.interface, e.devices.len()));
        }
        for f in &self.fisica {
            let s = &f.scene;
            out.push(format!(
                "fisica `{}`: {} with {} body(ies), {} belt(s), {} mover(s), {} sensor(s) at {} ms",
                f.name,
                f.path.display(),
                s.bodies.len(),
                s.belts.len(),
                s.movers.len(),
                s.sensors.len(),
                s.step_ms
            ));
        }
        for (name, r) in &self.reflexo {
            let n = r.drive.len() + r.conveyor.len() + r.sensor.len() + r.actuator.len();
            let on = match &r.scene {
                Some(scene) => format!("on the scene `{scene}`"),
                None => format!("every {} ms", reflexo::STEP_WITHOUT_SCENE.as_millis()),
            };
            out.push(format!("reflexo `{name}`: {n} model(s) {on}"));
        }
        for c in &self.channels {
            let role = match &c.role {
                Role::Listen { bind, port } => format!("listening on {bind}:{port}"),
                Role::Connect { host, port } => format!("dialling {host}:{port}"),
            };
            let controller = match &c.controller {
                Some(r) => format!("a controller sending {:?} and receiving {:?}", r.sends(), r.receives()),
                None => "no controller".to_string(),
            };
            out.push(format!("channel `{}`: {role}, {} message(s) of its own, {controller}", c.name, c.messages.len()));
        }
        out
    }
}

fn read_beside(base: &Path, place: &str, rel: &str) -> Result<(PathBuf, String), BenchError> {
    let path = base.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|source| BenchError::Read { place: place.to_string(), path: path.clone(), source })?;
    Ok((path, text))
}

/// Read and check a bench file whose referenced files live under `base` (the bench file's
/// folder). Nothing starts: every part is validated as its own module validates it, and every
/// referenced file is loaded, so what comes back can start.
pub fn load(text: &str, base: &Path) -> Result<BenchSpec, BenchError> {
    veredas_core::schema::check(text, "a bench file", FORMAT, VERSION).map_err(|e| match e {
        HeaderError::NotToml { reason, .. } => BenchError::NotToml(reason),
        HeaderError::WrongFormat { found, .. } => BenchError::WrongFormat(found),
        HeaderError::UnknownVersion { found, .. } => BenchError::UnknownVersion(found),
        other => invalid(other.to_string()),
    })?;
    let file: FileV1 = toml::from_str(text).map_err(|e| BenchError::NotToml(e.to_string()))?;
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut name = |kind: &str, place: &str, n: &str| -> Result<(), BenchError> {
        if n.trim().is_empty() {
            return Err(invalid(format!("{place}: the {kind} has no name")));
        }
        if n != n.trim() {
            return Err(invalid(format!("{place}: the name `{n}` has surrounding whitespace")));
        }
        if let Some(other) = names.insert(n.to_string(), format!("{kind} `{n}`")) {
            return Err(invalid(format!("{place}: `{n}` is already the name of the {other}")));
        }
        Ok(())
    };
    let out_dir = base.join(&file.out_dir);
    let mut emitters = Vec::new();
    for (i, e) in file.emitter.into_iter().enumerate() {
        let place = format!("emitter[{i}]");
        name("emitter", &place, &e.name)?;
        let place = format!("{place} (`{}`)", e.name);
        emit::file_name_for(&e.file, &chrono::Local::now(), 1).map_err(|err| invalid(format!("{place}.file: {err}")))?;
        if emit::safe_format(&chrono::Local::now(), &e.ts_format).is_none() {
            return Err(invalid(format!("{place}.ts_format: `{}` is not a datetime format", e.ts_format)));
        }
        for (j, v) in e.values.iter().enumerate() {
            if v.name.trim().is_empty() {
                return Err(invalid(format!("{place}.values[{j}]: the value has no name")));
            }
            if !v.low.is_finite() || !v.high.is_finite() || v.low > v.high {
                return Err(invalid(format!("{place}.values[{j}] (`{}`): the range {}..={} is empty or not finite", v.name, v.low, v.high)));
            }
            if v.decimals > 15 {
                return Err(invalid(format!("{place}.values[{j}] (`{}`): decimals is {}, over 15", v.name, v.decimals)));
            }
        }
        emitters.push(e);
    }
    let mut servers = Vec::new();
    for (i, s) in file.opc.into_iter().enumerate() {
        let place = format!("opc[{i}]");
        let (server_name, server) = s.server();
        name("opc", &place, &server_name)?;
        opc::validate(&server).map_err(|e| invalid(format!("{place} (`{server_name}`): {e}")))?;
        servers.push((server_name, server));
    }
    let mut modbus_servers = Vec::new();
    for (i, m) in file.modbus.into_iter().enumerate() {
        let place = format!("modbus[{i}]");
        let (server_name, server) = m.server();
        name("modbus", &place, &server_name)?;
        modbus::validate(&server).map_err(|e| invalid(format!("{place} (`{server_name}`): {e}")))?;
        modbus_servers.push((server_name, server));
    }
    let mut adapters = Vec::new();
    for (i, e) in file.ethernetip.into_iter().enumerate() {
        let place = format!("ethernetip[{i}]");
        let (adapter_name, adapter) = e.adapter();
        name("ethernetip", &place, &adapter_name)?;
        ethernetip::validate(&adapter).map_err(|e| invalid(format!("{place} (`{adapter_name}`): {e}")))?;
        adapters.push((adapter_name, adapter));
    }
    let mut devices = Vec::new();
    for (i, p) in file.profinet.into_iter().enumerate() {
        let place = format!("profinet[{i}]");
        let (device_name, device) = p.device();
        name("profinet", &place, &device_name)?;
        profinet::validate(&device).map_err(|e| invalid(format!("{place} (`{device_name}`): {e}")))?;
        devices.push((device_name, device));
    }
    let mut segments = Vec::new();
    for (i, e) in file.ethercat.into_iter().enumerate() {
        let place = format!("ethercat[{i}]");
        name("ethercat", &place, &e.name)?;
        let segment = EthercatConfig { interface: e.interface, devices: e.devices };
        ethercat::validate(&segment).map_err(|err| invalid(format!("{place} (`{}`): {err}", e.name)))?;
        segments.push((e.name, segment));
    }
    let mut scenes = Vec::new();
    for (i, f) in file.fisica.into_iter().enumerate() {
        let place = format!("fisica[{i}]");
        name("fisica", &place, &f.name)?;
        let place = format!("{place} (`{}`)", f.name);
        let (path, text) = read_beside(base, &format!("{place}.scene"), &f.scene)?;
        let scene = fisica::load(&text).map_err(|e| invalid(format!("{place}.scene ({}): {e}", path.display())))?;
        scenes.push(FisicaPart { name: f.name, path, scene });
    }
    let mut models = Vec::new();
    // Every point a model writes, across the file's parts, with the place that writes it.
    let mut written: Vec<(String, String)> = Vec::new();
    for (i, r) in file.reflexo.into_iter().enumerate() {
        let place = format!("reflexo[{i}]");
        name("reflexo", &place, &r.name)?;
        let place = format!("{place} (`{}`)", r.name);
        let cfg = ReflexoConfig { scene: r.scene, drive: r.drive, conveyor: r.conveyor, sensor: r.sensor, actuator: r.actuator };
        let scene = match &cfg.scene {
            None => None,
            Some(s) => Some(&scenes.iter().find(|f| &f.name == s).ok_or_else(|| invalid(format!("{place}.scene: no [[fisica]] part is named `{s}`")))?.scene),
        };
        // What a `<part>.<point>` carries, from the Modbus servers' and the adapters' own points.
        let point = |part: &str, point: &str| -> Result<PointKind, String> {
            let kind = |bool_: bool| if bool_ { PointKind::Bool } else { PointKind::Number };
            if let Some((_, m)) = modbus_servers.iter().find(|(n, _): &&(String, ModbusConfig)| n == part) {
                let p = m.points.iter().find(|p| p.name == point).ok_or_else(|| format!("`{part}` has no point `{point}`"))?;
                return Ok(kind(p.ty.unwrap_or(if p.table.is_bits() { modbus::PointType::Bool } else { modbus::PointType::U16 }) == modbus::PointType::Bool));
            }
            if let Some((_, e)) = adapters.iter().find(|(n, _): &&(String, EnipConfig)| n == part) {
                let p = e.points.iter().find(|p| p.name == point).ok_or_else(|| format!("`{part}` has no point `{point}`"))?;
                return Ok(kind(p.ty.unwrap_or(if p.bit.is_some() { ethernetip::PointType::Bool } else { ethernetip::PointType::U8 }) == ethernetip::PointType::Bool));
            }
            Err(format!("no Modbus server or EtherNet/IP adapter is named `{part}`"))
        };
        let mine = reflexo::validate_beside(&cfg, scene, &point, &written).map_err(|e| invalid(format!("{place}.{e}")))?;
        written.extend(mine.into_iter().map(|(reference, by)| (reference, format!("{place}.{by}"))));
        models.push((r.name, cfg));
    }
    let mut channels = Vec::new();
    for (i, c) in file.channel.into_iter().enumerate() {
        let place = format!("channel[{i}]");
        name("channel", &place, &c.name)?;
        let place = format!("{place} (`{}`)", c.name);
        if c.host.trim().is_empty() {
            return Err(invalid(format!("{place}.host is empty")));
        }
        let (path, text) = read_beside(base, &format!("{place}.channel"), &c.channel)?;
        let spec = channel_schema::load(&text).map_err(|e| invalid(format!("{place}.channel ({}): {e}", path.display())))?;
        let (path, text) = read_beside(base, &format!("{place}.catalog"), &c.catalog)?;
        let catalog = catalog::load(&text).map_err(|e| invalid(format!("{place}.catalog ({}): {e}", path.display())))?;
        channel::check_id_field(&spec, &catalog).map_err(|e| invalid(format!("{place}: {e}")))?;
        let controller = match &c.controller {
            None => None,
            Some(rel) => {
                let (path, text) = read_beside(base, &format!("{place}.controller"), rel)?;
                Some(controller::load(&text, &catalog).map_err(|e| invalid(format!("{place}.controller ({}): {e}", path.display())))?)
            }
        };
        let empty = BTreeMap::new();
        for (j, m) in c.messages.iter().enumerate() {
            let where_ = format!("{place}.messages[{j}]");
            let message = catalog.by_name(&m.message).ok_or_else(|| invalid(format!("{where_}: the catalog has no message `{}`", m.message)))?;
            Fields::compile(&where_, &m.fields, Place::Plain, &empty, Some(&message.names())).map_err(|e| invalid(format!("{where_} (`{}`): {e}", m.message)))?;
        }
        let role = match c.role {
            RoleFile::Listen => Role::Listen { bind: c.host.clone(), port: c.port },
            RoleFile::Connect => Role::Connect { host: c.host.clone(), port: c.port },
        };
        channels.push(ChannelPart { name: c.name, role, spec, catalog, controller, messages: c.messages });
    }
    Ok(BenchSpec { seed: file.seed, out_dir, emitters, servers, modbus: modbus_servers, ethernetip: adapters, profinet: devices, ethercat: segments, fisica: scenes, reflexo: models, channels })
}

/// Read and check the bench file at `path`; its referenced files are beside it.
pub fn load_file(path: &Path) -> Result<BenchSpec, BenchError> {
    let text = std::fs::read_to_string(path).map_err(|source| BenchError::Read { place: "the bench file".into(), path: path.to_path_buf(), source })?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    load(&text, &base)
}

/// What one part reports.
#[derive(Clone, Debug)]
pub enum PartStatus {
    Emitter(EmitterStatus),
    Opc { endpoint_url: String, status: OpcStatus },
    /// Boxed: a Modbus server's status carries its last hundred exchanges.
    Modbus { addr: std::net::SocketAddr, status: Box<ModbusStatus> },
    /// Boxed: an adapter's status carries its open connections.
    Enip { addr: std::net::SocketAddr, io_addr: std::net::SocketAddr, status: Box<EnipStatus> },
    Profinet { interface: String, station: String, status: ProfinetStatus },
    Ethercat { interface: String, status: EthercatStatus },
    Fisica(FisicaStatus),
    Reflexo { models: usize, status: ReflexoStatus },
    /// Boxed: a channel's status carries its monitor's hundred lines.
    Channel(Box<ChannelStatus>),
}

/// A running bench: every part behind one handle. Dropping it stops every part.
pub struct Bench {
    pub emitters: Vec<(String, EmitterHandle)>,
    pub servers: Vec<(String, OpcHandle)>,
    pub modbus: Vec<(String, ModbusHandle)>,
    pub ethernetip: Vec<(String, EnipHandle)>,
    pub profinet: Vec<(String, ProfinetHandle)>,
    pub ethercat: Vec<(String, EthercatHandle)>,
    pub fisica: Vec<(String, FisicaHandle)>,
    pub reflexo: Vec<(String, ReflexoHandle)>,
    pub channels: Vec<(String, BenchChannel)>,
}

impl Bench {
    /// Start every part of a loaded bench: emitters first, then servers (OPC UA, then Modbus,
    /// then EtherNet/IP), then PROFINET devices, then EtherCAT segments, then scenes, then their
    /// models, then channels, each kind in the file's order (TOML groups a kind's tables under one key, so the file keeps
    /// no order across kinds); the first part that cannot start stops the ones before it and
    /// is named. Emitters draw from the bench's seed plus their index when they have no seed
    /// of their own; channels from the bench's seed.
    pub fn start(spec: BenchSpec) -> Result<Bench, BenchError> {
        if !spec.emitters.is_empty() {
            std::fs::create_dir_all(&spec.out_dir).map_err(|source| BenchError::Read { place: "out_dir".into(), path: spec.out_dir.clone(), source })?;
        }
        let mut bench = Bench {
            emitters: Vec::new(),
            servers: Vec::new(),
            modbus: Vec::new(),
            ethernetip: Vec::new(),
            profinet: Vec::new(),
            ethercat: Vec::new(),
            fisica: Vec::new(),
            reflexo: Vec::new(),
            channels: Vec::new(),
        };
        for (i, mut e) in spec.emitters.into_iter().enumerate() {
            if e.seed.is_none() {
                e.seed = spec.seed.map(|s| s.wrapping_add(i as u64));
            }
            let name = e.name.clone();
            let handle = emit::spawn(e, spec.out_dir.clone(), i as u64).map_err(|source| BenchError::Emitter { name: name.clone(), source })?;
            bench.emitters.push((name, handle));
        }
        for (name, cfg) in spec.servers {
            let handle = opc::start(&cfg).map_err(|source| BenchError::Opc { name: name.clone(), source })?;
            bench.servers.push((name, handle));
        }
        for (name, cfg) in spec.modbus {
            let handle = modbus::start(&cfg).map_err(|source| BenchError::Modbus { name: name.clone(), source })?;
            bench.modbus.push((name, handle));
        }
        for (name, cfg) in spec.ethernetip {
            let handle = ethernetip::start(&cfg).map_err(|source| BenchError::Enip { name: name.clone(), source })?;
            bench.ethernetip.push((name, handle));
        }
        for (name, cfg) in spec.profinet {
            let handle = profinet::start(&cfg).map_err(|source| BenchError::Profinet { name: name.clone(), source })?;
            bench.profinet.push((name, handle));
        }
        for (name, cfg) in spec.ethercat {
            let handle = ethercat::start(&cfg).map_err(|source| BenchError::Ethercat { name: name.clone(), source })?;
            bench.ethercat.push((name, handle));
        }
        // Every scene built before any runs, and run once its models' hooks are in: no scene steps
        // without its models, however long another scene takes to build.
        for f in spec.fisica {
            let handle = fisica::paused(&f.scene).map_err(|source| BenchError::Fisica { name: f.name.clone(), source })?;
            bench.fisica.push((f.name, handle));
        }
        for (name, cfg) in spec.reflexo {
            let scene = cfg.scene.as_ref().and_then(|s| bench.fisica.iter().find(|(n, _)| n == s)).map(|(_, h)| h);
            let step = match scene {
                Some(h) => h.with(|w| w.step_length()),
                None => reflexo::STEP_WITHOUT_SCENE,
            };
            let part = |p: &str| -> Option<std::sync::Arc<dyn Points>> {
                if let Some((_, m)) = bench.modbus.iter().find(|(n, _)| n == p) {
                    return Some(std::sync::Arc::new(m.points()));
                }
                bench.ethernetip.iter().find(|(n, _)| n == p).map(|(_, e)| std::sync::Arc::new(e.points()) as std::sync::Arc<dyn Points>)
            };
            let models = reflexo::Models::new(&cfg, step, &part).map_err(|source| BenchError::Reflexo { name: name.clone(), source })?;
            let handle = reflexo::start(models, scene).map_err(|source| BenchError::Reflexo { name: name.clone(), source })?;
            bench.reflexo.push((name, handle));
        }
        for (name, f) in &mut bench.fisica {
            f.run().map_err(|source| BenchError::Fisica { name: name.clone(), source })?;
        }
        for c in spec.channels {
            let name = c.name.clone();
            let handle = channel::start(ChannelConfig { name: c.name, role: c.role, spec: c.spec, catalog: c.catalog, messages: c.messages, controller: c.controller, seed: spec.seed })
                .map_err(|source| BenchError::Channel { name: name.clone(), source })?;
            bench.channels.push((name, handle));
        }
        Ok(bench)
    }

    /// Every part's status, by name, in the file's order.
    pub fn status(&self) -> Vec<(String, PartStatus)> {
        let mut out = Vec::new();
        for (name, e) in &self.emitters {
            out.push((name.clone(), PartStatus::Emitter(e.status())));
        }
        for (name, s) in &self.servers {
            out.push((name.clone(), PartStatus::Opc { endpoint_url: s.endpoint_url.clone(), status: s.status() }));
        }
        for (name, m) in &self.modbus {
            out.push((name.clone(), PartStatus::Modbus { addr: m.addr, status: Box::new(m.status()) }));
        }
        for (name, e) in &self.ethernetip {
            out.push((name.clone(), PartStatus::Enip { addr: e.addr, io_addr: e.io_addr, status: Box::new(e.status()) }));
        }
        for (name, p) in &self.profinet {
            out.push((name.clone(), PartStatus::Profinet { interface: p.interface().to_string(), station: p.station(), status: p.status() }));
        }
        for (name, e) in &self.ethercat {
            out.push((name.clone(), PartStatus::Ethercat { interface: e.interface().to_string(), status: e.status() }));
        }
        for (name, f) in &self.fisica {
            out.push((name.clone(), PartStatus::Fisica(f.status())));
        }
        for (name, r) in &self.reflexo {
            out.push((name.clone(), PartStatus::Reflexo { models: r.models(), status: r.status() }));
        }
        for (name, c) in &self.channels {
            out.push((name.clone(), PartStatus::Channel(Box::new(c.status()))));
        }
        out
    }

    /// One line per part, as the executable prints them.
    pub fn lines(&self) -> Vec<String> {
        self.status()
            .into_iter()
            .map(|(name, status)| match status {
                PartStatus::Emitter(s) => format!(
                    "emitter {name}: {} line(s), {} in {}{}",
                    s.total_lines,
                    s.lines_in_file,
                    s.current_file.as_deref().unwrap_or("no file yet"),
                    s.last_error.as_ref().map(|e| format!(", error: {e}")).unwrap_or_default()
                ),
                PartStatus::Opc { endpoint_url, status } => format!(
                    "opc {name}: {endpoint_url} {} draw(s), {} reaction(s) fired, {} type mismatch(es)",
                    status.draws, status.reactions_fired, status.type_mismatches
                ),
                PartStatus::Modbus { addr, status } => format!(
                    "modbus {name}: {addr} {} connection(s), {} request(s) answered, {} exception(s), {} unanswered",
                    status.connections,
                    status.answered.values().sum::<u64>(),
                    status.exceptions.values().sum::<u64>(),
                    status.unanswered
                ),
                PartStatus::Enip { addr, io_addr, status } => format!(
                    "ethernetip {name}: {addr} (class 1 on {io_addr}) {} session(s), {} connection(s) open, {} request(s), {} refused open(s), {} timed out",
                    status.sessions,
                    status.connections.len(),
                    status.requests.values().sum::<u64>(),
                    status.refused_opens.values().sum::<u64>(),
                    status.timed_out
                ),
                PartStatus::Profinet { interface, station, status } => format!(
                    "profinet {name}: on {interface} as `{station}` ({}), {} identify answered, {} set(s), {} LLDP frame(s)",
                    status.mac, status.identify_answered, status.sets, status.lldp_sent
                ),
                PartStatus::Ethercat { interface, status } => format!(
                    "ethercat {name}: on {interface} {} frame(s), {} datagram(s), {} dropped",
                    status.frames, status.datagrams, status.dropped
                ),
                PartStatus::Fisica(s) => format!("fisica {name}: {} step(s), {} late, {} body(ies), {} sensor(s)", s.steps, s.late, s.bodies, s.sensors),
                PartStatus::Reflexo { models, status } => format!(
                    "reflexo {name}: {models} model(s), {} step(s), {} error(s){}",
                    status.steps,
                    status.errors,
                    status.last_error.as_ref().map(|e| format!(", last: {e}")).unwrap_or_default()
                ),
                PartStatus::Channel(s) => {
                    let sent: u64 = s.sent.values().sum();
                    let received: u64 = s.received.values().sum();
                    let controller = match &s.controller {
                        Some(c) => format!(", controller: {} announced, {} answered, {} awaiting", c.stats.announced, c.stats.answered, c.awaiting.len()),
                        None => String::new(),
                    };
                    format!(
                        "channel {name}: {} ({} peer(s)), {sent} sent, {received} received, {} unknown id(s), {} undecodable{controller}{}",
                        s.wire.state,
                        s.peers.len(),
                        s.unknown_ids,
                        s.undecodable,
                        s.last_error.as_ref().map(|e| format!(", error: {e}")).unwrap_or_default()
                    )
                }
            })
            .collect()
    }

    /// A channel by name, for the knobs.
    pub fn channel(&self, name: &str) -> Option<&BenchChannel> {
        self.channels.iter().find(|(n, _)| n == name).map(|(_, c)| c)
    }

    /// A Modbus server by name.
    pub fn modbus(&self, name: &str) -> Option<&ModbusHandle> {
        self.modbus.iter().find(|(n, _)| n == name).map(|(_, m)| m)
    }

    /// An EtherNet/IP adapter by name.
    pub fn ethernetip(&self, name: &str) -> Option<&EnipHandle> {
        self.ethernetip.iter().find(|(n, _)| n == name).map(|(_, e)| e)
    }

    /// An EtherCAT segment by name.
    pub fn ethercat(&self, name: &str) -> Option<&EthercatHandle> {
        self.ethercat.iter().find(|(n, _)| n == name).map(|(_, e)| e)
    }

    /// A scene by name.
    pub fn fisica(&self, name: &str) -> Option<&FisicaHandle> {
        self.fisica.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }

    /// A part's models by name.
    pub fn reflexo(&self, name: &str) -> Option<&ReflexoHandle> {
        self.reflexo.iter().find(|(n, _)| n == name).map(|(_, r)| r)
    }

    /// A PROFINET device by name.
    pub fn profinet(&self, name: &str) -> Option<&ProfinetHandle> {
        self.profinet.iter().find(|(n, _)| n == name).map(|(_, p)| p)
    }

    /// An OPC server by name.
    pub fn server(&self, name: &str) -> Option<&OpcHandle> {
        self.servers.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    /// Stop every part and wait for each.
    pub fn stop(self) {
        for (_, e) in self.emitters {
            e.join();
        }
        for (_, s) in self.servers {
            s.stop();
        }
        for (_, m) in self.modbus {
            m.stop();
        }
        for (_, e) in self.ethernetip {
            e.stop();
        }
        for (_, p) in self.profinet {
            p.stop();
        }
        for (_, e) in self.ethercat {
            e.stop();
        }
        // The models first: none steps once its scene's thread is told to stop.
        for (_, mut r) in self.reflexo {
            r.stop();
        }
        for (_, mut f) in self.fisica {
            f.stop();
        }
        for (_, c) in self.channels {
            c.stop();
        }
    }
}
