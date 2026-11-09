//! The supervisão file (`veredas-supervisao` v1, `docs/binding-schema.md`): the address the host
//! serves and the sessions it runs, each a name, a portal project file and a binding file. Loading
//! it loads every project as portal does and every binding against its project and its layout, so
//! a file that loads is a host every session of which can start.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use veredas_core::schema::HeaderError;
use veredas_portal::project::{self, Project};

use crate::binding::{self, Binding, Universe};

pub const FORMAT: &str = "veredas-supervisao";
pub const VERSION: u32 = 1;
pub const NOUN: &str = "a supervisão file";
/// The address served when the file names none: the loopback (Q20, local, no accounts).
pub const DEFAULT_BIND: &str = "127.0.0.1:8902";
/// The longest session name.
pub const NAME_MAX: usize = 32;

/// Why a supervisão file was refused; every refusal names its place.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0}")]
    Header(String),
    #[error("not {NOUN}: {0}")]
    NotToml(String),
    #[error("{0}")]
    Invalid(String),
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
    #[error("session `{session}`: the project {}: {reason}", path.display())]
    Project { session: String, path: PathBuf, reason: String },
    #[error("session `{session}`: the binding {}: {reason}", path.display())]
    Binding { session: String, path: PathBuf, reason: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[allow(dead_code)]
    format: toml::Value,
    #[allow(dead_code)]
    version: toml::Value,
    bind: Option<String>,
    #[serde(default)]
    session: Vec<RawSession>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSession {
    name: String,
    project: String,
    binding: String,
}

/// One session as loaded: its name, its project and its binding.
#[derive(Clone)]
pub struct SessionSpec {
    pub name: String,
    pub project_path: PathBuf,
    pub project: Project,
    pub binding_path: PathBuf,
    pub binding: Binding,
}

/// A supervisão file as loaded and checked whole.
#[derive(Clone)]
pub struct HostConfig {
    pub bind: SocketAddr,
    pub sessions: Vec<SessionSpec>,
}

/// The shape of a session name: a lowercase ASCII letter or digit, then lowercase letters, digits
/// and `-`, at most [`NAME_MAX`]: a path segment that needs no escaping.
pub fn name_shape(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err("the name is empty".into()),
        Some(c) if !(c.is_ascii_lowercase() || c.is_ascii_digit()) => {
            return Err(format!("the name `{name}` does not start with a lowercase ASCII letter or a digit"));
        }
        _ => {}
    }
    if let Some(c) = chars.find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')) {
        return Err(format!("the name `{name}` holds `{c}`: a name is lowercase ASCII letters, digits and `-`"));
    }
    if name.len() > NAME_MAX {
        return Err(format!("the name `{name}` is longer than {NAME_MAX}"));
    }
    Ok(())
}

/// Read a supervisão file's text, every path resolved against `base`.
pub fn load(text: &str, base: &Path) -> Result<HostConfig, ConfigError> {
    veredas_core::schema::check(text, NOUN, FORMAT, VERSION).map_err(|e| match e {
        HeaderError::NotToml { reason, .. } => ConfigError::NotToml(reason),
        other => ConfigError::Header(other.to_string()),
    })?;
    let raw: RawFile = toml::from_str(text).map_err(|e| ConfigError::NotToml(e.to_string().trim_end().to_string()))?;
    let bind_text = raw.bind.unwrap_or_else(|| DEFAULT_BIND.to_string());
    let bind: SocketAddr = bind_text
        .trim()
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("`bind` is `{bind_text}`, not an address and a port (`127.0.0.1:8902`)")))?;
    if raw.session.is_empty() {
        return Err(ConfigError::Invalid("no session: a supervisão file names at least one `[[session]]`".into()));
    }
    let mut names = BTreeSet::new();
    let mut sessions = Vec::new();
    for (i, s) in raw.session.into_iter().enumerate() {
        name_shape(&s.name).map_err(|e| ConfigError::Invalid(format!("session {i}: {e}")))?;
        if !names.insert(s.name.clone()) {
            return Err(ConfigError::Invalid(format!("session `{}`: the name is taken", s.name)));
        }
        if s.project.trim().is_empty() || s.binding.trim().is_empty() {
            return Err(ConfigError::Invalid(format!("session `{}`: `project` and `binding` name files", s.name)));
        }
        let project_path = base.join(s.project.trim());
        let project = project::load_file(&project_path).map_err(|e| ConfigError::Project { session: s.name.clone(), path: project_path.clone(), reason: e.to_string() })?;
        let binding_path = base.join(s.binding.trim());
        let text = std::fs::read_to_string(&binding_path).map_err(|e| ConfigError::Binding { session: s.name.clone(), path: binding_path.clone(), reason: format!("cannot read it: {e}") })?;
        let binding_base = binding_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let universe = Universe::of(&project);
        let binding = binding::load(&text, &binding_base, &universe, &project.name).map_err(|reason| ConfigError::Binding { session: s.name.clone(), path: binding_path.clone(), reason })?;
        sessions.push(SessionSpec { name: s.name, project_path, project, binding_path, binding });
    }
    Ok(HostConfig { bind, sessions })
}

/// Read a supervisão file from disk; its folder is where its references live.
pub fn load_file(path: &Path) -> Result<HostConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read { path: path.to_path_buf(), source })?;
    let base = path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    load(&text, &base)
}

/// The plan `--check` prints: the address, then per session its path, its project, its binding's
/// objects, layout, headline and dashboards, and what of the project it does not serve.
pub fn plan(cfg: &HostConfig) -> String {
    let mut out = format!("supervisão on http://{}/\n", cfg.bind);
    for s in &cfg.sessions {
        let b = &s.binding;
        out.push_str(&format!("session `{}` at /s/{}/: \"{}\", the project \"{}\"\n", s.name, s.name, b.title, s.project.name));
        out.push_str(&format!(
            "  {} object(s), {} on the layout{}; {} write(s) offered; headline: {}\n",
            b.objects.len(),
            b.objects.iter().filter(|o| b.layout.as_ref().is_some_and(|l| l.ids.contains_key(&o.id))).count(),
            match &b.layout {
                Some(l) => format!(" ({})", l.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                None => " (no layout)".to_string(),
            },
            b.objects.iter().map(|o| o.actions.len()).sum::<usize>(),
            if b.headline.is_empty() { "none".to_string() } else { b.headline.iter().map(|i| b.objects[*i].id.clone()).collect::<Vec<_>>().join(", ") },
        ));
        for d in &b.dashboards {
            out.push_str(&format!("  dashboard `{}`: {} panel(s): {}\n", d.name, d.panels.len(), d.panels.iter().map(|p| p.kind.name()).collect::<Vec<_>>().join(", ")));
        }
        out.push_str(&format!("  writes {}; portal's surface at /s/{}/portal/\n", if s.project.writes_armed { "armed by the project" } else { "off" }, s.name));
        if let Some(addr) = s.project.server {
            out.push_str(&format!("  the project's own server address {addr} is not bound\n"));
        }
    }
    out
}
