//! The channel configuration schema: a telegram channel's framing and rules, as plain text.
//!
//! A versioned TOML file, like every plain-text file of the suite (the plan, "Versioned
//! files"): it states `format = "veredas-channel"` and `version = 1`, and a reader refuses a
//! format or version it does not know rather than guess (migrating an older file is portal's,
//! P-010). The worked example and every key are in `docs/channel-schema.md`.
//!
//! ```toml
//! format = "veredas-channel"
//! version = 1
//!
//! [framing]
//! byte_order = "little"      # default for every field; a field may say otherwise
//! start = "FD"               # optional delimiter, hex
//! end = "FE"                 # optional delimiter, hex
//! max_frame = 65536          # optional, bytes, delimiters included
//! envelope = [
//!   { name = "sequence", size = 1, role = "sequence" },
//!   { name = "length", size = 2, role = "length" },
//!   { name = "count", size = 2, role = "count" },
//! ]
//! segment = [ { name = "id", size = 2 }, { name = "length", size = 2, role = "length" } ]
//! tail = [ { name = "check", size = 1 } ]
//!
//! [sequence]                 # optional; needs an envelope field with role "sequence"
//! connect = 0
//! first = 1
//! last = 255
//! skip_duplicates = true
//! keepalive_counts = true
//!
//! [keepalive]
//! on_connect = true
//! idle_ms = 10000            # 0: never probe
//!
//! [link]
//! write_timeout_ms = 2000
//! outbox_limit_bytes = 16777216
//! max_peers = 8
//! ```

use std::collections::HashSet;
use std::time::Duration;

use serde::Deserialize;
use crate::schema::HeaderError;

use crate::telegram::Endian;
use crate::telegram::framing::{Field, FieldRole, Framing, parse_hex};

/// The format name every channel file states.
pub const FORMAT: &str = "veredas-channel";
/// The one version this reader knows.
pub const VERSION: u32 = 1;

/// The sequence rules: what the channel stamps and what the receiver skips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceRules {
    /// Stamped on the connect keep-alive.
    pub connect: u64,
    /// The first number after connect, and where the cycle wraps back to.
    pub first: u64,
    /// The last number before the cycle wraps.
    pub last: u64,
    /// The receiver skips a frame repeating the last number it saw.
    pub skip_duplicates: bool,
    /// A keep-alive's number (other than `connect`) counts as the last seen. Whether a device's
    /// keep-alives consume numbers is the device's: `docs/questions.md` Q9.
    pub keepalive_counts: bool,
}

/// When the channel sends keep-alives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepaliveRules {
    pub on_connect: bool,
    /// Silence this long (nothing read and nothing written) sends a probe; zero never does.
    pub idle: Duration,
}

/// How the link treats a peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRules {
    /// A peer whose socket has taken nothing for this long is not reading, and is dropped.
    pub write_timeout: Duration,
    /// Bytes queued for one peer beyond what its socket took: a memory bound.
    pub outbox_limit_bytes: usize,
    /// Simultaneous connections accepted when listening.
    pub max_peers: usize,
}

/// A channel configuration, compiled and validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSpec {
    pub framing: Framing,
    pub sequence: Option<SequenceRules>,
    pub keepalive: KeepaliveRules,
    pub link: LinkRules,
}

/// Why a channel file was refused. Never a guess: an unknown version, a misspelled key or an
/// impossible layout is named.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("not a channel file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `{FORMAT}`")]
    WrongFormat(String),
    #[error("version {0} of `{FORMAT}` is not known here (this reader knows {VERSION})")]
    UnknownVersion(u32),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    framing: FramingFile,
    sequence: Option<SequenceFile>,
    #[serde(default)]
    keepalive: KeepaliveFile,
    #[serde(default)]
    link: LinkFile,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum OrderFile {
    Little,
    Big,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FramingFile {
    byte_order: Option<OrderFile>,
    start: Option<String>,
    end: Option<String>,
    max_frame: Option<usize>,
    envelope: Vec<FieldFile>,
    segment: Option<Vec<FieldFile>>,
    #[serde(default)]
    tail: Vec<FieldFile>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
enum RoleFile {
    #[default]
    Opaque,
    Sequence,
    Length,
    Count,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldFile {
    name: String,
    size: usize,
    #[serde(default)]
    role: RoleFile,
    byte_order: Option<OrderFile>,
    /// `None` when the file does not state one: a computed field stating any value is refused.
    value: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct SequenceFile {
    connect: u64,
    first: u64,
    last: u64,
    skip_duplicates: bool,
    keepalive_counts: bool,
}

impl Default for SequenceFile {
    fn default() -> Self {
        SequenceFile { connect: 0, first: 1, last: 255, skip_duplicates: true, keepalive_counts: true }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct KeepaliveFile {
    on_connect: bool,
    idle_ms: u64,
}

impl Default for KeepaliveFile {
    fn default() -> Self {
        KeepaliveFile { on_connect: true, idle_ms: 10_000 }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct LinkFile {
    write_timeout_ms: u64,
    outbox_limit_bytes: usize,
    max_peers: usize,
}

impl Default for LinkFile {
    fn default() -> Self {
        LinkFile { write_timeout_ms: 2_000, outbox_limit_bytes: 16 * 1024 * 1024, max_peers: 8 }
    }
}

/// The default `max_frame`: 64 KiB.
pub const DEFAULT_MAX_FRAME: usize = 64 * 1024;
/// The largest `max_frame` a file may state: 16 MiB. A peer may declare a frame this large and
/// the channel will hold its bytes while they arrive, so the ceiling bounds that memory.
pub const MAX_MAX_FRAME: usize = 16 * 1024 * 1024;

fn invalid(msg: impl Into<String>) -> SchemaError {
    SchemaError::Invalid(msg.into())
}

fn order(o: Option<OrderFile>, default: Endian) -> Endian {
    match o {
        Some(OrderFile::Little) => Endian::Little,
        Some(OrderFile::Big) => Endian::Big,
        None => default,
    }
}

fn fields(
    part: &str,
    files: Vec<FieldFile>,
    default: Endian,
    allowed: &[RoleFile],
) -> Result<Vec<Field>, SchemaError> {
    let mut names = HashSet::new();
    let mut roles = HashSet::new();
    let mut out = Vec::new();
    for f in files {
        if f.name.trim().is_empty() {
            return Err(invalid(format!("{part}: a field has no name")));
        }
        if !names.insert(f.name.clone()) {
            return Err(invalid(format!("{part}: two fields are named `{}`", f.name)));
        }
        if !(1..=8).contains(&f.size) {
            return Err(invalid(format!("{part}.{}: size {} is not 1 to 8 bytes", f.name, f.size)));
        }
        if !allowed.contains(&f.role) {
            return Err(invalid(format!("{part}.{}: this part takes no such role", f.name)));
        }
        if f.role != RoleFile::Opaque && !roles.insert(f.role) {
            return Err(invalid(format!("{part}: two fields share the role of `{}`", f.name)));
        }
        if f.role != RoleFile::Opaque && f.value.is_some() {
            return Err(invalid(format!("{part}.{}: a computed field takes no `value`", f.name)));
        }
        let field = Field {
            name: f.name,
            size: f.size,
            role: match f.role {
                RoleFile::Opaque => FieldRole::Opaque,
                RoleFile::Sequence => FieldRole::Sequence,
                RoleFile::Length => FieldRole::Length,
                RoleFile::Count => FieldRole::Count,
            },
            order: order(f.byte_order, default),
            value: f.value.unwrap_or(0),
        };
        if field.size < 8 && field.value >= 1u64 << (8 * field.size) {
            return Err(invalid(format!("{part}.{}: value {} does not fit {} byte(s)", field.name, field.value, field.size)));
        }
        out.push(field);
    }
    Ok(out)
}

fn delimiter(name: &str, text: Option<String>) -> Result<Vec<u8>, SchemaError> {
    match text {
        None => Ok(Vec::new()),
        Some(t) => {
            let bytes = parse_hex(&t).map_err(|e| invalid(format!("framing.{name}: {e}")))?;
            if bytes.is_empty() {
                return Err(invalid(format!("framing.{name}: an empty delimiter; leave it out instead")));
            }
            Ok(bytes)
        }
    }
}

/// Read and validate a channel file.
pub fn load(text: &str) -> Result<ChannelSpec, SchemaError> {
    crate::schema::check(text, "a channel file", FORMAT, VERSION).map_err(|e| match e {
        HeaderError::NotToml { reason, .. } => SchemaError::NotToml(reason),
        HeaderError::WrongFormat { found, .. } => SchemaError::WrongFormat(found),
        HeaderError::UnknownVersion { found, .. } => SchemaError::UnknownVersion(found),
        other => invalid(other.to_string()),
    })?;
    let file: FileV1 = toml::from_str(text).map_err(|e| SchemaError::NotToml(e.to_string()))?;

    let f = file.framing;
    let default = order(f.byte_order, Endian::Little);
    use RoleFile::*;
    let envelope = fields("envelope", f.envelope, default, &[Opaque, Sequence, Length, Count])?;
    let segment = match f.segment {
        Some(s) if s.is_empty() => {
            return Err(invalid("segment: an empty segment header; leave `segment` out instead"));
        }
        Some(s) => Some(fields("segment", s, default, &[Opaque, Length])?),
        None => None,
    };
    let tail = fields("tail", f.tail, default, &[Opaque])?;
    // Envelope and tail fields share one map in a frame: a name names one field.
    if let Some(clash) = tail.iter().find(|t| envelope.iter().any(|e| e.name == t.name)) {
        return Err(invalid(format!("tail.{0}: the envelope already has a field named `{0}`", clash.name)));
    }
    let has = |r: FieldRole| envelope.iter().any(|x| x.role == r);
    match &segment {
        None => {
            if !has(FieldRole::Length) {
                return Err(invalid("envelope: without segments the envelope needs a `length` field"));
            }
            if has(FieldRole::Count) {
                return Err(invalid("envelope: a `count` field needs segments"));
            }
        }
        Some(header) => {
            if !header.iter().any(|x| x.role == FieldRole::Length) {
                return Err(invalid("segment: a segment header needs a `length` field"));
            }
            if !has(FieldRole::Length) && !has(FieldRole::Count) {
                return Err(invalid("envelope: segments need a `length` or a `count` field to end"));
            }
        }
    }
    let max_frame = f.max_frame.unwrap_or(DEFAULT_MAX_FRAME);
    if max_frame > MAX_MAX_FRAME {
        return Err(invalid(format!("framing.max_frame: {max_frame} is over the {MAX_MAX_FRAME}-byte ceiling")));
    }
    let start = delimiter("start", f.start)?;
    let end = delimiter("end", f.end)?;
    let fixed: usize = start.len()
        + end.len()
        + envelope.iter().chain(tail.iter()).map(|x| x.size).sum::<usize>();
    if max_frame < fixed {
        return Err(invalid(format!("framing.max_frame: {max_frame} is less than the {fixed} fixed bytes")));
    }

    let sequence = match file.sequence {
        // A sequence field states that frames are numbered: the rules that number them must be
        // stated too (an empty `[sequence]` takes every default). Never a silent zero.
        None => match envelope.iter().find(|x| x.role == FieldRole::Sequence) {
            Some(field) => {
                return Err(invalid(format!(
                    "envelope.{}: a sequence field needs `[sequence]` rules (an empty `[sequence]` takes the defaults)",
                    field.name
                )));
            }
            None => None,
        },
        Some(s) => {
            let field = envelope.iter().find(|x| x.role == FieldRole::Sequence).ok_or_else(|| {
                invalid("sequence: rules for a sequence the envelope has no `sequence` field for")
            })?;
            let max = if field.size >= 8 { u64::MAX } else { (1u64 << (8 * field.size)) - 1 };
            if s.first > s.last || s.last > max || s.connect > max {
                return Err(invalid(format!(
                    "sequence: first {} .. last {} (connect {}) does not fit a {}-byte field in order",
                    s.first, s.last, s.connect, field.size
                )));
            }
            if s.first == s.last && s.skip_duplicates {
                return Err(invalid("sequence: a cycle of one number cannot skip duplicates (every frame would repeat)"));
            }
            // The rule (docs/questions.md Q11, decided 2026-11-09): the receiver tells the connect keep-alive
            // by its number, so a cycle that can stamp that number would hide a keep-alive from
            // the duplicate check and let a data frame repeat the last one.
            if (s.first..=s.last).contains(&s.connect) {
                return Err(invalid(format!(
                    "sequence: connect {} lies inside the cycle {} .. {} (the connect keep-alive is told by its number)",
                    s.connect, s.first, s.last
                )));
            }
            Some(SequenceRules {
                connect: s.connect,
                first: s.first,
                last: s.last,
                skip_duplicates: s.skip_duplicates,
                keepalive_counts: s.keepalive_counts,
            })
        }
    };
    // A zero timeout would judge a peer deaf the moment its socket took bytes; an outbox that
    // cannot hold one largest frame would drop a peer on its first send (C-012, refute round 4).
    if file.link.write_timeout_ms == 0 {
        return Err(invalid("link.write_timeout_ms: at least 1"));
    }
    if file.link.outbox_limit_bytes < max_frame {
        return Err(invalid(format!(
            "link.outbox_limit_bytes: {} cannot hold one largest frame ({max_frame})",
            file.link.outbox_limit_bytes
        )));
    }
    if file.link.max_peers == 0 {
        return Err(invalid("link.max_peers: at least one"));
    }
    Ok(ChannelSpec {
        framing: Framing { start, end, envelope, segment, tail, max_frame },
        sequence,
        keepalive: KeepaliveRules {
            on_connect: file.keepalive.on_connect,
            idle: Duration::from_millis(file.keepalive.idle_ms),
        },
        link: LinkRules {
            write_timeout: Duration::from_millis(file.link.write_timeout_ms),
            outbox_limit_bytes: file.link.outbox_limit_bytes,
            max_peers: file.link.max_peers,
        },
    })
}
