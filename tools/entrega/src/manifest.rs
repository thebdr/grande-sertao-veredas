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
    /// Its folder, relative to the repository's root (`crates/veredas-portal`).
    pub dir: String,
    pub name: String,
    /// Its version, as cargo reads its manifest.
    pub version: String,
    /// Its executables: every `bin` target cargo itself names for it. Whether cargo builds each
    /// with the default features is the cut's to see (C-291), never modelled here.
    pub bins: BTreeSet<String>,
}

/// Why a manifest could not be checked at all.
#[derive(Debug, Error)]
pub enum CheckError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The components (the workspace's members under `crates/`) the repository holds at `commit`, by
/// name, as cargo itself reads them: the commit's tree taken out of git's objects (`git archive`)
/// into a scratch folder of this call's own and asked with `cargo metadata --no-deps --offline`,
/// so a member's spelling and an edition's discovery rules are cargo's, never a model of them;
/// each folder is read against the workspace root cargo names, in cargo's own spelling of paths.
/// What cannot be read is pushed to `refusals`.
pub fn components_at(root: &Path, commit: &str, refusals: &mut Vec<String>) -> Result<BTreeMap<String, Held>, CheckError> {
    components_at_in(root, commit, &std::env::temp_dir(), refusals)
}

/// [`components_at`], its scratch folder made under `parent` (a folder reached through a link
/// included).
pub fn components_at_in(root: &Path, commit: &str, parent: &Path, refusals: &mut Vec<String>) -> Result<BTreeMap<String, Held>, CheckError> {
    use std::process::{Command, Stdio};
    let mut held = BTreeMap::new();
    let scratch_dir = tempfile::Builder::new().prefix("entrega-tree-").tempdir_in(parent)?;
    let scratch = scratch_dir.path().to_path_buf();
    let result = (|| -> Result<Option<String>, CheckError> {
        let tar = scratch.join("tree.tar");
        if git::run(root, &["archive", "--format=tar", "-o", &tar.display().to_string(), commit])?.is_none() {
            return Ok(Some(format!("commit {commit}: git cannot archive its tree")));
        }
        let tree = scratch.join("tree");
        std::fs::create_dir_all(&tree)?;
        let untar = Command::new("tar").arg("-xf").arg(&tar).arg("-C").arg(&tree).status()?;
        if !untar.success() {
            return Ok(Some(format!("commit {commit}: its tree does not unpack")));
        }
        if !tree.join("Cargo.toml").is_file() {
            return Ok(Some(format!("commit {commit} holds no Cargo.toml")));
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let out = Command::new(cargo)
            .args(["metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path"])
            .arg(tree.join("Cargo.toml"))
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            let first = why.lines().find(|l| l.starts_with("error")).unwrap_or("cargo failed").trim().to_string();
            return Ok(Some(format!("commit {commit}: cargo cannot read its workspace: {first}")));
        }
        let meta: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(m) => m,
            Err(e) => return Ok(Some(format!("commit {commit}: cargo's metadata does not read: {e}"))),
        };
        let base = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
        for pkg in meta["packages"].as_array().into_iter().flatten() {
            let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default());
            let dir = manifest.parent().and_then(|d| d.strip_prefix(&base).ok()).map(|d| d.to_string_lossy().replace('\\', "/"));
            let Some(dir) = dir.filter(|d| d.starts_with("crates/")) else { continue };
            let name = pkg["name"].as_str().unwrap_or_default().to_string();
            let version = pkg["version"].as_str().unwrap_or_default().to_string();
            let bins = pkg["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "bin")))
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect();
            held.insert(name.clone(), Held { dir, name, version, bins });
        }
        Ok(None)
    })();
    drop(scratch_dir);
    if let Some(r) = result? {
        refusals.push(r);
    }
    Ok(held)
}

/// Checks a manifest against the repository at `root`: its commit there and an ancestor of HEAD,
/// every component the commit holds listed at the version it states and with exactly the
/// executables it builds, nothing else listed. Every refusal is returned, in a fixed order.
pub fn check(root: &Path, m: &Manifest) -> Result<Vec<String>, CheckError> {
    let mut refusals = Vec::new();
    let c = &m.commit;
    match git::text(root, &["cat-file", "-t", c])?.map(|t| t.trim().to_string()) {
        None => {
            refusals.push(format!("commit {c} is not in the repository"));
            return Ok(refusals);
        }
        Some(kind) if kind != "commit" => {
            refusals.push(format!("{c} names a {kind}, not a commit: a release names the commit itself"));
            return Ok(refusals);
        }
        Some(_) => {}
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

/// Reads every manifest of a releases folder (`<release>.toml`): the manifests that read, and a
/// refusal for every file that does not or is named after another release than its own, so a
/// caller judges the rest and says every refusal. A folder that is not there holds none.
pub fn load_dir(dir: &Path) -> (Vec<(PathBuf, Manifest)>, Vec<String>) {
    let mut out = Vec::new();
    let mut refusals = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (out, refusals),
        Err(e) => return (out, vec![format!("{}: {e}", dir.display())]),
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
    (out, refusals)
}
