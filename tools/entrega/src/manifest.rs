//! The suite release's manifest: `veredas-release`, version 1 (docs/entrega-plan.md, "What a
//! release is made of"). A release names the commit it was cut from and pins every component at
//! the version its own manifest states there, with the executables it ships; the manifest is read
//! through core's one header reader, a misspelled key refused, and checked against the
//! repository's history, so a manifest stays checkable at any later commit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use veredas_core::schema::{self, HeaderError};

use crate::git::{self, GitError};

pub const FORMAT: &str = "veredas-release";
pub const VERSION: u32 = 1;
const NOUN: &str = "a release manifest";

/// A release as its manifest states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The release's own SemVer (docs/questions.md Q431).
    pub release: semver::Version,
    /// The full id of the commit the release was built from.
    pub commit: String,
    /// The target triple the executables were built for.
    pub target: String,
    /// What `rustc --version` printed at the cut: stated, not enforced.
    pub toolchain: String,
    /// Every component at the commit, in the file's order.
    pub components: Vec<Component>,
    pub tested: Tested,
}

/// A component pinned by a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub version: semver::Version,
    /// The executables it ships, exactly the ones it builds at the commit; none for a library.
    pub bins: Vec<String>,
}

/// What was run to call the set tested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tested {
    /// The command whose green run at the commit tested the components against each other.
    pub suite: String,
    /// What the cut ran from the bundle itself.
    pub together: String,
}

/// Why a manifest was refused when read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("{NOUN}: {0}")]
    Invalid(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[allow(dead_code)]
    format: toml::Value,
    #[allow(dead_code)]
    version: toml::Value,
    release: String,
    commit: String,
    target: String,
    toolchain: String,
    #[serde(default)]
    component: Vec<RawComponent>,
    tested: RawTested,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComponent {
    name: String,
    version: String,
    #[serde(default)]
    bins: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTested {
    suite: String,
    together: String,
}

fn invalid(what: impl Into<String>) -> ManifestError {
    ManifestError::Invalid(what.into())
}

/// Whether `name` is a suite crate's name: `veredas-<nome>` or `orcl-<nome>`, the nome a lowercase
/// ASCII letter followed by lowercase letters and digits (law L2's rule).
pub fn is_suite_crate(name: &str) -> bool {
    let nome = name.strip_prefix("veredas-").or_else(|| name.strip_prefix("orcl-"));
    match nome {
        Some(n) => {
            let mut c = n.chars();
            c.next().is_some_and(|f| f.is_ascii_lowercase()) && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit())
        }
        None => false,
    }
}

fn is_commit_id(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_bin_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn stated(key: &str, value: &str) -> Result<String, ManifestError> {
    if value.trim().is_empty() {
        Err(invalid(format!("`{key}` is empty")))
    } else if value.trim() != value {
        Err(invalid(format!("`{key}` begins or ends with whitespace")))
    } else {
        Ok(value.to_string())
    }
}

/// Reads a manifest's text: the header first (core's reader), then every key, a misspelled one
/// refused, each value checked for its form. Nothing of the repository is read here: that is
/// [`check`]'s.
pub fn load(text: &str) -> Result<Manifest, ManifestError> {
    schema::check(text, NOUN, FORMAT, VERSION)?;
    let raw: Raw = toml::from_str(text).map_err(|e| invalid(e.message().to_string()))?;
    let release = semver::Version::parse(&raw.release)
        .map_err(|_| invalid(format!("`release` is `{}`, not SemVer", raw.release)))?;
    if !is_commit_id(&raw.commit) {
        return Err(invalid(format!(
            "`commit` is `{}`, not a full commit id (40 lowercase hexadecimal characters)",
            raw.commit
        )));
    }
    let target = stated("target", &raw.target)?;
    if target.chars().any(char::is_whitespace) {
        return Err(invalid(format!("`target` is `{target}`, not a target triple")));
    }
    let toolchain = stated("toolchain", &raw.toolchain)?;
    if raw.component.is_empty() {
        return Err(invalid("no `[[component]]`: a release pins at least one component"));
    }
    let mut seen = BTreeSet::new();
    let mut components = Vec::new();
    for c in raw.component {
        if !is_suite_crate(&c.name) {
            return Err(invalid(format!("component `{}` is not a suite crate (veredas-<nome> or orcl-<nome>)", c.name)));
        }
        if !seen.insert(c.name.clone()) {
            return Err(invalid(format!("component `{}` is listed twice", c.name)));
        }
        let version = semver::Version::parse(&c.version)
            .map_err(|_| invalid(format!("component `{}`: `version` is `{}`, not SemVer", c.name, c.version)))?;
        let mut bins_seen = BTreeSet::new();
        for b in &c.bins {
            if !is_bin_name(b) {
                return Err(invalid(format!("component `{}`: bin `{b}` is not an executable's name", c.name)));
            }
            if !bins_seen.insert(b.clone()) {
                return Err(invalid(format!("component `{}`: bin `{b}` is listed twice", c.name)));
            }
        }
        components.push(Component { name: c.name, version, bins: c.bins });
    }
    let tested = Tested {
        suite: stated("tested.suite", &raw.tested.suite)?,
        together: stated("tested.together", &raw.tested.together)?,
    };
    Ok(Manifest { release, commit: raw.commit, target, toolchain, components, tested })
}

/// A component as the repository holds it at a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    /// Its folder, as the workspace lists it (`crates/veredas-portal`).
    pub dir: String,
    pub name: String,
    /// The version its manifest states, as written.
    pub version: String,
    /// The executables it builds, by cargo's rules: its `[[bin]]` tables, `src/main.rs` (named
    /// after the package) and `src/bin/` unless `autobins = false`.
    pub bins: BTreeSet<String>,
}

/// Why a manifest could not be checked at all.
#[derive(Debug, Error)]
pub enum CheckError {
    #[error(transparent)]
    Git(#[from] GitError),
}

#[derive(Deserialize)]
struct Workspace {
    workspace: Option<WorkspaceTable>,
}

#[derive(Deserialize)]
struct WorkspaceTable {
    #[serde(default)]
    members: Vec<String>,
}

#[derive(Deserialize)]
struct CrateManifest {
    package: Option<Package>,
    #[serde(default)]
    bin: Vec<BinTable>,
}

#[derive(Deserialize)]
struct Package {
    name: Option<String>,
    version: Option<toml::Value>,
    autobins: Option<bool>,
}

#[derive(Deserialize)]
struct BinTable {
    name: Option<String>,
}

/// The components (the members under `crates/`) the repository holds at `commit`, by name, with
/// what each states; refusals for what cannot be read are pushed to `refusals`.
pub fn components_at(root: &Path, commit: &str, refusals: &mut Vec<String>) -> Result<BTreeMap<String, Held>, CheckError> {
    let mut held = BTreeMap::new();
    let Some(ws) = git::show(root, commit, "Cargo.toml")? else {
        refusals.push(format!("commit {commit} holds no Cargo.toml"));
        return Ok(held);
    };
    let members = match toml::from_str::<Workspace>(&ws) {
        Ok(Workspace { workspace: Some(w) }) => w.members,
        _ => {
            refusals.push(format!("commit {commit}: its Cargo.toml lists no workspace"));
            return Ok(held);
        }
    };
    for dir in members.into_iter().filter(|m| m.starts_with("crates/")) {
        let Some(text) = git::show(root, commit, &format!("{dir}/Cargo.toml"))? else {
            refusals.push(format!("commit {commit}: member {dir} has no Cargo.toml"));
            continue;
        };
        let m: CrateManifest = match toml::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                refusals.push(format!("commit {commit}: {dir}/Cargo.toml is not read: {}", e.message()));
                continue;
            }
        };
        let Some(package) = m.package else {
            refusals.push(format!("commit {commit}: {dir}/Cargo.toml has no [package]"));
            continue;
        };
        let Some(name) = package.name else {
            refusals.push(format!("commit {commit}: {dir}/Cargo.toml names no package"));
            continue;
        };
        let version = match package.version {
            Some(toml::Value::String(v)) => v,
            _ => {
                refusals.push(format!("commit {commit}: {name} states no version of its own"));
                continue;
            }
        };
        let mut bins: BTreeSet<String> = m.bin.iter().map(|b| b.name.clone().unwrap_or_else(|| name.clone())).collect();
        if package.autobins != Some(false) {
            if git::list(root, commit, &format!("{dir}/src"))?.is_some_and(|l| l.iter().any(|f| f == "main.rs")) {
                bins.insert(name.clone());
            }
            if let Some(entries) = git::list(root, commit, &format!("{dir}/src/bin"))? {
                for e in entries {
                    match e.strip_suffix(".rs") {
                        Some(stem) => {
                            bins.insert(stem.to_string());
                        }
                        None => {
                            let inner = git::list(root, commit, &format!("{dir}/src/bin/{e}"))?;
                            if inner.is_some_and(|l| l.iter().any(|f| f == "main.rs")) {
                                bins.insert(e);
                            }
                        }
                    }
                }
            }
        }
        held.insert(name.clone(), Held { dir, name, version, bins });
    }
    Ok(held)
}

/// Checks a manifest against the repository at `root`: its commit there and an ancestor of HEAD,
/// every component the commit holds listed at the version it states and with exactly the
/// executables it builds, nothing else listed. Every refusal is returned, in a fixed order.
pub fn check(root: &Path, m: &Manifest) -> Result<Vec<String>, CheckError> {
    let mut refusals = Vec::new();
    let c = &m.commit;
    if git::text(root, &["rev-parse", "--verify", "--quiet", &format!("{c}^{{commit}}")])?.is_none() {
        refusals.push(format!("commit {c} is not in the repository"));
        return Ok(refusals);
    }
    if git::run(root, &["merge-base", "--is-ancestor", c, "HEAD"])?.is_none() {
        refusals.push(format!("commit {c} is not an ancestor of HEAD: a release is cut from the history it is committed in"));
    }
    let held = components_at(root, c, &mut refusals)?;
    for h in held.values() {
        if !m.components.iter().any(|x| x.name == h.name) {
            refusals.push(format!("{} {} is a component at {c} and the manifest leaves it out", h.name, h.version));
        }
    }
    for x in &m.components {
        let Some(h) = held.get(&x.name) else {
            refusals.push(format!("{} is no component at {c}", x.name));
            continue;
        };
        if h.version != x.version.to_string() {
            refusals.push(format!("{} is {} at {c}, not {}", x.name, h.version, x.version));
        }
        let listed: BTreeSet<String> = x.bins.iter().cloned().collect();
        for b in h.bins.difference(&listed) {
            refusals.push(format!("{} builds `{b}` at {c} and the manifest does not ship it", x.name));
        }
        for b in listed.difference(&h.bins) {
            refusals.push(format!("{} builds no `{b}` at {c}", x.name));
        }
    }
    Ok(refusals)
}

/// Checks manifests against each other: a component's version names one tree, so two releases
/// pinning one version must hold the same code for it (its folder's tree at each commit).
pub fn check_together(root: &Path, all: &[(PathBuf, Manifest)]) -> Result<Vec<String>, CheckError> {
    let mut refusals = Vec::new();
    // (name, version) -> the first release seen pinning it, with its folder's tree there.
    let mut first: BTreeMap<(String, String), (String, Option<String>)> = BTreeMap::new();
    for (_, m) in all {
        let mut ignore = Vec::new();
        let held = components_at(root, &m.commit, &mut ignore)?;
        for x in &m.components {
            let tree = match held.get(&x.name) {
                Some(h) => git::object(root, &m.commit, &h.dir)?,
                None => None,
            };
            let key = (x.name.clone(), x.version.to_string());
            match first.get(&key) {
                None => {
                    first.insert(key, (m.release.to_string(), tree));
                }
                Some((other, t)) if *t != tree => refusals.push(format!(
                    "{} {} is pinned by releases {other} and {} at different code: a version names one tree",
                    x.name, x.version, m.release
                )),
                Some(_) => {}
            }
        }
    }
    Ok(refusals)
}

/// Reads every manifest of a releases folder (`<release>.toml`), refusing a file named after
/// another release than its own. A folder that is not there holds none.
pub fn load_dir(dir: &Path) -> Result<Vec<(PathBuf, Manifest)>, Vec<String>> {
    let mut out = Vec::new();
    let mut refusals = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(vec![format!("{}: {e}", dir.display())]),
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            refusals.push(format!("{}: a releases folder holds manifests (`<release>.toml`) alone", p.display()));
            continue;
        }
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| load(&t).map_err(|e| e.to_string())) {
            Ok(m) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                if stem != m.release.to_string() {
                    refusals.push(format!("{}: the file is named `{stem}.toml`, not after its release {}", p.display(), m.release));
                }
                out.push((p, m));
            }
            Err(e) => refusals.push(format!("{}: {e}", p.display())),
        }
    }
    if refusals.is_empty() { Ok(out) } else { Err(refusals) }
}
