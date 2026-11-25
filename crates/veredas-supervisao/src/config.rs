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
    shared_files(&sessions)?;
    Ok(HostConfig { bind, sessions })
}

/// Two sessions of one host never open one database file: the host starts the second, portal
/// finds the first's database standing there and refuses it, so the check says so first. The files
/// are the ones portal opens for each project (`session_config`): a store pinned to a file, a store
/// of each run's own in a folder and a record, the last two named from the project's name as portal
/// writes it in a file name (`session_store_file`: two names may come out as one) and the start,
/// the host's one instant for every session (read here at one instant alike).
fn shared_files(sessions: &[SessionSpec]) -> Result<(), ConfigError> {
    use veredas_portal::project::Store;
    use veredas_portal::store::Residency;
    let at = chrono::NaiveDateTime::default();
    let mut opened: Vec<(PathBuf, &str, String)> = Vec::new();
    for s in sessions {
        let cfg = s.project.session_config(at);
        let name = s.project.name.trim();
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        if let Residency::Disk { path, .. } = &cfg.residency {
            let what = match &s.project.store {
                Store::PerSession { dir, .. } => format!("its store (a file of each run's own in {}, named from the project's name `{name}` and the start)", dir.display()),
                _ => format!("its store ({})", path.display()),
            };
            files.push((path.clone(), what));
        }
        if let (Some(r), Some(rec)) = (&cfg.recording, &s.project.recording) {
            files.push((r.file.clone(), format!("its record (a file of each run's own in {}, named from the project's name `{name}` and the start)", rec.dir.display())));
        }
        for (file, what) in files {
            // A path the engine reads as a database of another type than its own (an extension's,
            // `sqlite:`, a lone `file:/`) is opened through that extension or nowhere: refused by
            // name, this host placing the engine's own databases alone.
            let engine = as_the_engine_reads(&file).map_err(|kind| {
                ConfigError::Invalid(format!(
                    "session `{}`: {what} is at {}, which the database engine reads as a database of type `{kind}` (the part before its first colon), opened through an extension if at all: give the project a folder whose path the engine reads as its own",
                    s.name,
                    file.display(),
                ))
            })?;
            // A path through more than forty links opens nowhere (the drive refuses it): no place,
            // so nothing to share.
            let Some(at) = same_place(&engine) else { continue };
            if let Some((_, other, theirs)) = opened.iter().find(|(k, _, _)| *k == at) {
                return Err(ConfigError::Invalid(format!(
                    "session `{}`: {what} is the database file session `{other}` opens for {theirs}: two sessions of one host open no file alike (give one project another folder, or a name portal writes as another file name)",
                    s.name,
                    theirs = theirs.split(" (").next().unwrap_or(theirs),
                )));
            }
            opened.push((at, &s.name, what));
        }
    }
    Ok(())
}

/// A file's place, read as one place however its path is written (its caller hands it the path as
/// the engine reads it, `as_the_engine_reads`): made absolute, then read part by
/// part as the drive reads it, a part that is a link replaced by where it leads (its target taken
/// from its folder, read part by part in turn), standing or not yet; a `..` the parent of what is
/// read so far, so a `..` after a link, written or in a link's target, is the parent of where the
/// link leads. Forty links at most, the drive's own bound: a path that meets one more opens nowhere,
/// and has no place (`None`). What of it stands is then read by the drive itself (on Windows, its
/// names as the drive keeps them), and on Windows the whole compared without case (Unicode's
/// lowercase).
fn same_place(p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let owned = |q: &Path| q.components().map(|c| PathBuf::from(c.as_os_str())).collect::<std::collections::VecDeque<PathBuf>>();
    let mut pending = owned(&abs);
    let mut out = PathBuf::new();
    let mut hops = 0;
    while let Some(part) = pending.pop_front() {
        match part.components().next() {
            Some(Component::CurDir) | None => {}
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::Normal(_)) => {
                let next = out.join(&part);
                match std::fs::read_link(&next) {
                    Ok(_) if hops == 40 => return None,
                    Ok(to) => {
                        hops += 1;
                        let mut led = owned(&out.join(to));
                        led.extend(pending.drain(..));
                        pending = led;
                        out = PathBuf::new();
                    }
                    Err(_) => out = next,
                }
            }
            Some(_) => out.push(&part),
        }
    }
    let parts: Vec<Component> = out.components().collect();
    let mut stands = parts.len();
    while stands > 0 {
        if let Ok(read) = std::fs::canonicalize(parts[..stands].iter().collect::<PathBuf>()) {
            out = parts[stands..].iter().fold(read, |o, c| o.join(c));
            break;
        }
        stands -= 1;
    }
    if cfg!(windows) {
        out = PathBuf::from(out.to_string_lossy().to_lowercase());
    }
    Some(out)
}

/// A database file's path as the engine reads it before the drive does (DuckDB 1.5.4, the engine
/// portal bundles, for the database it opens and for one it attaches), read as its bytes: first its
/// type (`ExtractExtensionPrefixFromPath`: what stands before the path's first colon, two or more
/// ASCII letters, digits or `_`, no `://` after the colon), its own type (`duckdb`, in any case)
/// dropped and any other returned as the type the engine opens the path through (an extension's,
/// `sqlite:`, a lone `file:/`); then the rest as `FileSystem::ExpandPath` reads it: a leading `~` the
/// home folder (`HOME`, on Windows `USERPROFILE`; the engine's `home_directory` setting, which
/// portal leaves unset, apart) and the rest after it as written; a leading `file:` URL its path
/// (`file:///`, `file://localhost/`; on Windows without the slash before the drive's letter); any
/// other path as it is.
fn as_the_engine_reads(p: &Path) -> Result<PathBuf, String> {
    #[cfg(unix)]
    let (bytes, from) = {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        (p.as_os_str().as_bytes().to_vec(), |b: Vec<u8>| PathBuf::from(std::ffi::OsString::from_vec(b)))
    };
    #[cfg(not(unix))]
    let (bytes, from) = (p.to_string_lossy().into_owned().into_bytes(), |b: Vec<u8>| PathBuf::from(String::from_utf8_lossy(&b).into_owned()));
    let mut rest: &[u8] = &bytes;
    if let Some(colon) = bytes.iter().position(|&c| c == b':') {
        if colon >= 2 && bytes.get(colon..colon + 3) != Some(b"://".as_slice()) && bytes[..colon].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            let kind = String::from_utf8_lossy(&bytes[..colon]).to_ascii_lowercase();
            if kind != "duckdb" {
                return Err(kind);
            }
            rest = &bytes[colon + 1..];
        }
    }
    if let Some(after) = rest.strip_prefix(b"~") {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
        #[cfg(unix)]
        let mut out = { use std::os::unix::ffi::OsStrExt; home.as_bytes().to_vec() };
        #[cfg(not(unix))]
        let mut out = home.to_string_lossy().into_owned().into_bytes();
        out.extend_from_slice(after);
        return Ok(from(out));
    }
    let win = usize::from(cfg!(windows));
    let offset = if !rest.starts_with(b"file:/") {
        0
    } else if rest.get(6) != Some(&b'/') {
        5 + win
    } else if rest.get(7) == Some(&b'/') {
        7 + win
    } else if rest.get(7..17) == Some(b"localhost/".as_slice()) {
        16 + win
    } else {
        0
    };
    Ok(from(rest[offset..].to_vec()))
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
        out.push_str(&format!("  writes {}; portal's surface at /s/{}/portal/\n", if s.project.writes_armed() { "armed by the project" } else { "off" }, s.name));
        if let Some(addr) = s.project.server {
            out.push_str(&format!("  the project's own server address {addr} is not bound\n"));
        }
    }
    out
}
