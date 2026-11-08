//! An OPC UA source: a server polled on an interval, its tags mapped into tables by rules.
//!
//! A source is the connection (an endpoint, an identity, a poll interval); a rule is the mapping
//! (a table whose columns are tags and the bit fields carved out of them), so one endpoint feeds
//! several tables and a change of poll rate never touches a tag list. The configuration, as an
//! operator types it, compiles into core's [`PollSpec`] before anything connects, every mistake
//! refused where it was written and by name; core's `opc::worker` then polls, reconnects and
//! writes through its gated handle (C-022), and core's `opc::map` turns each poll into rows
//! (C-020). The password never sits in a project file: the file names the environment variable
//! that holds it (`docs/questions.md` Q15), and the lookup is handed in, so a test needs no
//! environment.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use veredas_core::SeriesType;
use veredas_core::opc::error::NodeIdError;
use veredas_core::opc::spec::{Auth, BitSpec, PollSpec, RuleSpec, TagSpec, parse_node_id};

pub use veredas_core::opc::map::{DedupState, Mapped, RuleRow, map_poll, map_poll_dedup, render_line};
pub use veredas_core::opc::worker::{ConnState, OpcEvent, OpcHandle, OpcWorker, TagRead, WriteError, WriteGate, spawn};

/// The shortest poll interval a source may ask for.
pub const MIN_POLL_MS: u64 = 50;
/// The longest a table or column name may be.
pub const MAX_NAME: usize = 64;

/// One OPC UA source, as the project file declares it: the connection and its rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpcSourceConfig {
    pub source_id: String,
    #[serde(default)]
    pub display_name: String,
    /// `opc.tcp://host:port/path`.
    pub endpoint_url: String,
    /// The user the server knows; absent, the session is anonymous.
    #[serde(default)]
    pub user: Option<String>,
    /// The environment variable holding the user's password; never the password.
    #[serde(default)]
    pub password_env: Option<String>,
    /// How often the whole tag set is read; one Read per tick covers every tag the rules map.
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    #[serde(default)]
    pub rules: Vec<OpcRuleConfig>,
}

fn default_poll_ms() -> u64 {
    1000
}

/// One rule: a table of this source's tags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpcRuleConfig {
    /// The table's name.
    pub name: String,
    /// Keep transitions, not every identical poll (a heartbeat row every 60 polls).
    #[serde(default)]
    pub store_on_change: bool,
    pub tags: Vec<OpcTagConfig>,
}

/// One tag of a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpcTagConfig {
    /// The standard text form, `ns=<n>;s=<name>` (or `i=`, `g=`, `b=`).
    pub node_id: String,
    /// The column the tag's own value lands in; empty means the tag is polled for its bits alone.
    #[serde(default)]
    pub series: String,
    /// `int`, `float`, `text` or `time`; float when absent.
    #[serde(default, rename = "type")]
    pub ty: String,
    /// May be written through the handle; a precondition, the master switch still has to be on.
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub bits: Vec<OpcBitConfig>,
}

/// A bit field carved out of an integer tag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpcBitConfig {
    /// The least-significant bit of the field, 0-based.
    pub bit: u8,
    /// How many bits wide; 1 when absent (0 reads as 1).
    #[serde(default = "default_width")]
    pub width: u8,
    /// The column, always an int.
    pub series: String,
}

fn default_width() -> u8 {
    1
}

/// Why a source's configuration did not compile. Every refusal names the source, and the rule,
/// tag or bit at fault.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum OpcError {
    #[error("source `{source_id}`: the endpoint url is empty")]
    EmptyEndpoint { source_id: String },
    #[error("source `{source_id}`: the poll interval of {ms} ms is under {MIN_POLL_MS} ms")]
    PollTooShort { source_id: String, ms: u64 },
    #[error("source `{source_id}`: the user name is empty")]
    EmptyUser { source_id: String },
    #[error("source `{source_id}`: `password_env` is given but no `user` is")]
    PasswordWithoutUser { source_id: String },
    #[error("source `{source_id}`: the user `{user}` has no `password_env`")]
    NoPasswordEnv { source_id: String, user: String },
    #[error("source `{source_id}`: the environment variable `{var}` for the password of `{user}` is not set")]
    PasswordUnset { source_id: String, user: String, var: String },
    #[error("source `{source_id}`: no rules, so nothing to poll")]
    NoRules { source_id: String },
    #[error("source `{source_id}`, rule `{name}`: {reason}")]
    RuleName { source_id: String, name: String, reason: String },
    #[error("source `{source_id}`: two rules are named `{name}`")]
    RuleTwice { source_id: String, name: String },
    #[error("source `{source_id}`, rule `{rule}`: no tags, so nothing to poll")]
    NoTags { source_id: String, rule: String },
    #[error("source `{source_id}`, rule `{rule}`: {source}")]
    NodeId { source_id: String, rule: String, source: NodeIdError },
    #[error("source `{source_id}`, rule `{rule}`, tag `{tag}`: unknown type `{ty}` (int, float, text or time)")]
    Type { source_id: String, rule: String, tag: String, ty: String },
    #[error("source `{source_id}`, rule `{rule}`, tag `{tag}`: give it a series name, or at least one bit")]
    NoSeriesNoBits { source_id: String, rule: String, tag: String },
    #[error("source `{source_id}`, rule `{rule}`, column `{column}`: {reason}")]
    ColumnName { source_id: String, rule: String, column: String, reason: String },
    #[error("source `{source_id}`, rule `{rule}`: two fields both write the column `{column}`")]
    ColumnTwice { source_id: String, rule: String, column: String },
    #[error("source `{source_id}`, rule `{rule}`, bit `{series}`: bit {bit} is past the widest word (0 to 63)")]
    BitTooHigh { source_id: String, rule: String, series: String, bit: u8 },
    #[error("source `{source_id}`, rule `{rule}`, tag `{tag}`: bits are carved out of an int or float word, not a {ty} tag")]
    BitsOnText { source_id: String, rule: String, tag: String, ty: SeriesType },
}

/// A table or column name: lowercase ASCII letters, digits and `_`, starting with a letter or
/// `_`, at most [`MAX_NAME`] long, so it needs no quoting anywhere it is used.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name.len() > MAX_NAME {
        return Err(format!("the name is longer than {MAX_NAME}"));
    }
    let first = name.chars().next().unwrap_or('_');
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err("the name must start with a lowercase letter or `_`".into());
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err("the name may use lowercase letters, digits and `_` alone".into());
    }
    Ok(())
}

/// The series type a configuration spells: `int`, `float` (the default), `text` or `time`.
pub fn parse_type(text: &str) -> Option<SeriesType> {
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "float" => Some(SeriesType::Float),
        "int" => Some(SeriesType::Int),
        "text" => Some(SeriesType::Text),
        "time" => Some(SeriesType::Time),
        _ => None,
    }
}

/// Compile a source's configuration into the spec its worker runs. `secret` answers an
/// environment variable's name with the password it holds (the session reads the environment;
/// a test hands in a closure). Nothing connects here.
pub fn compile(cfg: &OpcSourceConfig, secret: &dyn Fn(&str) -> Option<String>) -> Result<PollSpec, OpcError> {
    let sid = cfg.source_id.trim().to_string();
    let endpoint_url = cfg.endpoint_url.trim().to_string();
    if endpoint_url.is_empty() {
        return Err(OpcError::EmptyEndpoint { source_id: sid });
    }
    if cfg.poll_ms < MIN_POLL_MS {
        return Err(OpcError::PollTooShort { source_id: sid, ms: cfg.poll_ms });
    }
    let auth = match (&cfg.user, &cfg.password_env) {
        (None, None) => Auth::Anonymous,
        (None, Some(_)) => return Err(OpcError::PasswordWithoutUser { source_id: sid }),
        (Some(user), env) => {
            let user = user.trim().to_string();
            if user.is_empty() {
                return Err(OpcError::EmptyUser { source_id: sid });
            }
            let Some(var) = env.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
                return Err(OpcError::NoPasswordEnv { source_id: sid, user });
            };
            let Some(password) = secret(var) else {
                return Err(OpcError::PasswordUnset { source_id: sid, user, var: var.to_string() });
            };
            Auth::UserPass { username: user, password }
        }
    };
    if cfg.rules.is_empty() {
        return Err(OpcError::NoRules { source_id: sid });
    }
    let mut rules: Vec<RuleSpec> = Vec::new();
    for rule in &cfg.rules {
        let name = rule.name.trim().to_string();
        check_name(&name).map_err(|reason| OpcError::RuleName { source_id: sid.clone(), name: name.clone(), reason })?;
        if rules.iter().any(|r| r.name == name) {
            return Err(OpcError::RuleTwice { source_id: sid, name });
        }
        if rule.tags.is_empty() {
            return Err(OpcError::NoTags { source_id: sid, rule: name });
        }
        let mut columns: Vec<String> = Vec::new();
        let mut tags = Vec::new();
        for tag in &rule.tags {
            let raw = tag.node_id.trim().to_string();
            let node_id = parse_node_id(&raw).map_err(|source| OpcError::NodeId { source_id: sid.clone(), rule: name.clone(), source })?;
            let ty = parse_type(&tag.ty).ok_or_else(|| OpcError::Type {
                source_id: sid.clone(),
                rule: name.clone(),
                tag: raw.clone(),
                ty: tag.ty.trim().to_string(),
            })?;
            let series = tag.series.trim().to_string();
            if series.is_empty() && tag.bits.is_empty() {
                return Err(OpcError::NoSeriesNoBits { source_id: sid, rule: name, tag: raw });
            }
            if !tag.bits.is_empty() && !matches!(ty, SeriesType::Int | SeriesType::Float) {
                return Err(OpcError::BitsOnText { source_id: sid, rule: name, tag: raw, ty });
            }
            if !series.is_empty() {
                check_name(&series).map_err(|reason| OpcError::ColumnName {
                    source_id: sid.clone(),
                    rule: name.clone(),
                    column: series.clone(),
                    reason,
                })?;
                if columns.contains(&series) {
                    return Err(OpcError::ColumnTwice { source_id: sid, rule: name, column: series });
                }
                columns.push(series.clone());
            }
            let mut bits = Vec::new();
            for b in &tag.bits {
                let bseries = b.series.trim().to_string();
                check_name(&bseries).map_err(|reason| OpcError::ColumnName {
                    source_id: sid.clone(),
                    rule: name.clone(),
                    column: bseries.clone(),
                    reason,
                })?;
                if columns.contains(&bseries) {
                    return Err(OpcError::ColumnTwice { source_id: sid, rule: name, column: bseries });
                }
                if b.bit >= 64 {
                    return Err(OpcError::BitTooHigh { source_id: sid, rule: name, series: bseries, bit: b.bit });
                }
                columns.push(bseries.clone());
                bits.push(BitSpec { bit: b.bit, width: b.width.max(1), series: bseries });
            }
            tags.push(TagSpec { node_id, raw, series, ty, writable: tag.writable, bits });
        }
        rules.push(RuleSpec { name, tags, store_on_change: rule.store_on_change });
    }
    let display_name = match cfg.display_name.trim() {
        "" => sid.clone(),
        d => d.to_string(),
    };
    Ok(PollSpec {
        source_id: sid,
        display_name,
        endpoint_url,
        auth,
        poll: std::time::Duration::from_millis(cfg.poll_ms),
        rules,
    })
}

/// The table a rule's rows land in: its name and its columns in the rule's order (each tag's
/// own column, then its bit fields as ints), as core's `RuleSpec::columns` lays them out.
pub fn table_of(rule: &RuleSpec) -> (String, Vec<(String, SeriesType)>) {
    (rule.name.clone(), rule.columns())
}
