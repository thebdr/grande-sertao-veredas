//! The bench file: one versioned TOML file for the whole plant a consumer meets on the bench
//! (`docs/bench-schema.md`), and [`Bench::start`], which runs every part of it behind one handle.
//!
//! A bench file lists its parts by name: log emitters ([`crate::emit`]), OPC UA servers
//! ([`crate::opc`]) and telegram channels ([`crate::channel`]), each channel pointing at the
//! device's own files kept beside the bench file (core's channel file, the catalog, and, when a
//! controller rides the channel, its rules file). Every referenced file is read and checked when
//! the bench file loads, so a bench that loads is a bench every part of which can start; what
//! `check` finds wrong is named with its place, and nothing binds, opens or spawns before it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use veredas_core::telegram::Role;
use veredas_core::telegram::catalog::{self, Catalog};
use veredas_core::telegram::schema::{self as channel_schema, ChannelSpec};

use crate::channel::{self, BenchChannel, ChannelConfig, ChannelError, ChannelStatus, CyclicMessage};
use crate::controller::{self, Fields, Place, Rules};
use crate::emit::{self, EmitterConfig, EmitterHandle, EmitterStatus, SpawnError};
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
    #[error("channel `{name}`: {source}")]
    Channel { name: String, source: ChannelError },
}

fn invalid(msg: impl Into<String>) -> BenchError {
    BenchError::Invalid(msg.into())
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
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
    pub channels: Vec<ChannelPart>,
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
    let header: Header = toml::from_str(text).map_err(|e| BenchError::NotToml(e.to_string()))?;
    match header.format.as_deref() {
        Some(FORMAT) => {}
        Some(other) => return Err(BenchError::WrongFormat(other.to_string())),
        None => return Err(invalid("no `format`: a bench file states `format` and `version`")),
    }
    match header.version {
        Some(VERSION) => {}
        Some(v) => return Err(BenchError::UnknownVersion(v)),
        None => return Err(invalid("no `version`: a bench file states `format` and `version`")),
    }
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
    Ok(BenchSpec { seed: file.seed, out_dir, emitters, servers, channels })
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
    /// Boxed: a channel's status carries its monitor's hundred lines.
    Channel(Box<ChannelStatus>),
}

/// A running bench: every part behind one handle. Dropping it stops every part.
pub struct Bench {
    pub emitters: Vec<(String, EmitterHandle)>,
    pub servers: Vec<(String, OpcHandle)>,
    pub channels: Vec<(String, BenchChannel)>,
}

impl Bench {
    /// Start every part of a loaded bench, in the file's order; the first part that cannot
    /// start stops the ones before it and is named. Emitters draw from the bench's seed plus
    /// their index when they have no seed of their own; channels from the bench's seed.
    pub fn start(spec: BenchSpec) -> Result<Bench, BenchError> {
        if !spec.emitters.is_empty() {
            std::fs::create_dir_all(&spec.out_dir).map_err(|source| BenchError::Read { place: "out_dir".into(), path: spec.out_dir.clone(), source })?;
        }
        let mut bench = Bench { emitters: Vec::new(), servers: Vec::new(), channels: Vec::new() };
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
        for (_, c) in self.channels {
            c.stop();
        }
    }
}
