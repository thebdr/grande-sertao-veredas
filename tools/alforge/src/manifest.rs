//! The suite release's manifest: `veredas-release`, version 1 (docs/alforge-plan.md, "What a
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
use crate::inputs::{self, Inputs};

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

/// An executable's name as cargo makes one: letters (any script's, as `ação`), digits, `-` and
/// `_`; never a path, a space or a control character.
fn is_bin_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
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
    /// Its executables every release ships: each `bin` target cargo names for it, built with the
    /// features they need (docs/questions.md Q443, ruled 2026-11-26 "With the features its
    /// executables need").
    pub bins: BTreeSet<String>,
    /// The features its executables need, whatever its default features are: the union of its
    /// bins' `required-features` as cargo states them (`shell`, `dep/feature`), turned on in its
    /// one build.
    pub features: BTreeSet<String>,
    /// Its `bin` targets a release does not ship: each with required features in a package whose
    /// folder names a toolchain of its own (`rust-toolchain`), which alone builds them, as ORCL's
    /// front end's driver needs its pinned nightly (docs/questions.md Q449, taken meanwhile).
    pub unshipped: BTreeSet<String>,
    /// Its `bin` targets cargo never builds, by the required feature the package does not define
    /// (cargo warns and leaves such a bin out of every build): no release ships one.
    pub never: BTreeMap<String, String>,
    /// Its library target's name (`veredas_core`), if it has one: what the cut builds of a
    /// component that ships no executable, to hold that build too to its inputs.
    pub lib: Option<String>,
    /// What its version names: everything a build of it reads but the toolchain
    /// ([`crate::inputs`]), or why that cannot be read at the commit.
    pub inputs: Result<Inputs, String>,
    /// The packages its build compiles (none when its inputs do not read).
    pub packages: inputs::Packages,
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
/// name, as cargo itself reads them: the commit's tree taken out of git's objects as stored
/// into a scratch folder of this call's own and asked with `cargo metadata --no-deps --offline`,
/// so a member's spelling and an edition's discovery rules are cargo's, never a model of them;
/// each folder is read against the workspace root cargo names, in cargo's own spelling of paths.
/// What cannot be read is pushed to `refusals`.
/// The scratch folder's parent is the tool's cache (`tmp/` there, on disk), whatever `TMPDIR`
/// says: the verdict must not depend on where the system puts temporary files.
pub fn components_at(root: &Path, commit: &str, refusals: &mut Vec<String>) -> Result<BTreeMap<String, Held>, CheckError> {
    let parent = crate::cache::dir()?.join("tmp");
    std::fs::create_dir_all(&parent)?;
    components_at_in(root, commit, &parent, refusals)
}

/// [`components_at`], its scratch folder made under `parent` (a folder reached through a link
/// included).
pub fn components_at_in(root: &Path, commit: &str, parent: &Path, refusals: &mut Vec<String>) -> Result<BTreeMap<String, Held>, CheckError> {
    use std::process::{Command, Stdio};
    let mut held = BTreeMap::new();
    // Cargo reads a workspace's member globs against the root's absolute path unescaped: a glob
    // character in that path (`*`, `?` or `[`; braces are no pattern to it) changes which members
    // it finds. The path is judged absolute, as cargo
    // will see it.
    // Canonical, links and `..` resolved as the kernel resolves them: cargo removes a `..` without
    // following the link before it, and would look elsewhere than where the tree was written.
    let parent = parent.canonicalize()?;
    if parent.to_str().is_none() {
        refusals.push(format!("the scratch folder's parent {} is not UTF-8: cargo takes no such path", parent.display()));
        return Ok(held);
    }
    if parent.to_string_lossy().contains(['*', '?', '[']) {
        refusals.push(format!("the scratch folder's parent {} holds a glob character cargo would read as a pattern", parent.display()));
        return Ok(held);
    }
    let scratch_dir = tempfile::Builder::new().prefix("alforge-tree-").tempdir_in(&parent)?;
    let scratch = scratch_dir.path().to_path_buf();
    let result = (|| -> Result<Option<String>, CheckError> {
        // The tree as git stores it, no attribute or filter applied (git::export_tree).
        let tree = scratch.join("tree");
        std::fs::create_dir_all(&tree)?;
        match git::export_tree(root, commit, &[], &tree) {
            Ok(_) => {}
            Err(git::ExportError::Refused(why)) => return Ok(Some(format!("commit {commit}: {why}"))),
            Err(git::ExportError::Git(e)) => return Ok(Some(format!("commit {commit}: git cannot give its tree: {}", e.source))),
        }
        if !tree.join("Cargo.toml").is_file() {
            return Ok(Some(format!("commit {commit} holds no Cargo.toml")));
        }
        // A cargo configuration in the tree is refused whole, before cargo reads it: what one may
        // set reaches past what a version names (`paths`, sources, an `include` of another file, a
        // flag naming a linker or a wrapper), and none is tracked.
        for f in [".cargo/config.toml", ".cargo/config"] {
            if tree.join(f).exists() {
                return Ok(Some(format!("commit {commit}: its {f} is a cargo configuration in the tree, which a release's build does not follow: what one sets reaches past what a version names")));
            }
        }
        // A root that is no workspace sends cargo looking for one above the tree, in folders the
        // history does not hold.
        let Ok(root_manifest) = String::from_utf8(std::fs::read(tree.join("Cargo.toml"))?) else {
            return Ok(Some(format!("commit {commit}: its Cargo.toml is not UTF-8")));
        };
        // A manifest that does not parse goes to cargo, which stops on it with its own reason.
        if toml::from_str::<toml::Table>(&root_manifest).is_ok_and(|t| !t.contains_key("workspace")) {
            return Ok(Some(format!("commit {commit}: its Cargo.toml is no workspace (`[workspace]`): cargo would look for one above the tree")));
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let out = Command::new(cargo)
            .args(["metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path"])
            .arg(tree.join("Cargo.toml"))
            .current_dir(&tree)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            // Cargo's whole reason, its causes included, the scratch folder's path dropped.
            let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
            let reason: Vec<&str> = why.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            return Ok(Some(format!("commit {commit}: cargo cannot read its workspace: {}", reason.join(" / "))));
        }
        let meta: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(m) => m,
            Err(e) => return Ok(Some(format!("commit {commit}: cargo's metadata does not read: {e}"))),
        };
        let base = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
        // Everything cargo read must lie in the commit's tree: its workspace root, every package's
        // manifest, every path dependency (a path out of the tree reads a file the history does not
        // hold).
        let inside = |p: &Path| p.canonicalize().ok().zip(tree.canonicalize().ok()).is_some_and(|(p, t)| p.starts_with(t));
        if base.canonicalize().ok() != tree.canonicalize().ok() {
            return Ok(Some(format!("commit {commit}: cargo names a workspace root outside the commit's tree")));
        }
        for pkg in meta["packages"].as_array().into_iter().flatten() {
            let name = pkg["name"].as_str().unwrap_or_default();
            if !inside(Path::new(pkg["manifest_path"].as_str().unwrap_or_default())) {
                return Ok(Some(format!("commit {commit}: the package `{name}` lies outside the commit's tree")));
            }
            for dep in pkg["dependencies"].as_array().into_iter().flatten() {
                if let Some(path) = dep["path"].as_str()
                    && !inside(Path::new(path))
                {
                    let dep_name = dep["name"].as_str().unwrap_or_default();
                    return Ok(Some(format!("commit {commit}: the package `{name}` depends on `{dep_name}` by a path outside the commit's tree")));
                }
            }
        }
        // A folder from the root, in cargo's spelling of the path it gives.
        let from_root = |p: &Path| p.strip_prefix(&base).ok().map(|d| d.to_string_lossy().replace('\\', "/"));
        let packages_of = |meta: &serde_json::Value, workspace: &str| -> Vec<inputs::Member> {
            let mut out = Vec::new();
            for pkg in meta["packages"].as_array().into_iter().flatten() {
                let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default());
                let Some(dir) = manifest.parent().and_then(from_root) else { continue };
                let deps = pkg["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| d["kind"].as_str() != Some("dev"))
                    .map(|d| inputs::Dep {
                        name: d["name"].as_str().unwrap_or_default().to_string(),
                        dir: d["path"].as_str().map(|p| from_root(Path::new(p)).unwrap_or_else(|| p.to_string())),
                        req: d["req"].as_str().unwrap_or("*").to_string(),
                    })
                    .collect();
                // Each target that goes into what ships starts from a file cargo names; its tests',
                // benches' and examples' do not.
                let sources = pkg["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| ["lib", "rlib", "dylib", "cdylib", "staticlib", "bin", "custom-build", "proc-macro"].contains(&k.as_str().unwrap_or_default()))))
                    .filter_map(|t| t["src_path"].as_str().map(|p| from_root(Path::new(p)).unwrap_or_else(|| p.to_string())))
                    .collect();
                let build_script = pkg["targets"].as_array().into_iter().flatten().any(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "custom-build")));
                let features = pkg["features"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                out.push(inputs::Member {
                    name: pkg["name"].as_str().unwrap_or_default().to_string(),
                    version: pkg["version"].as_str().unwrap_or_default().to_string(),
                    dir,
                    workspace: workspace.to_string(),
                    deps,
                    sources,
                    build_script,
                    features,
                });
            }
            out
        };
        let mut members = packages_of(&meta, "");
        // A path dependency outside the workspace's members (a fork kept in the tree, excluded from
        // the workspace) is read by cargo too, in the workspace it belongs to, so its own
        // dependencies and its inherited keys are cargo's; one cargo cannot read is its users'
        // refusal. That package alone, never its workspace's other members: cargo neither locks
        // nor builds one nothing reaches, and one of the same name and version would be taken for
        // the package the lock pins (a `[patch]` to another folder, which is refused).
        let mut unread = BTreeMap::new();
        loop {
            let known: BTreeSet<String> = members.iter().map(|m| m.dir.clone()).chain(unread.keys().cloned()).collect();
            let unknown: BTreeSet<String> = members.iter().flat_map(|m| m.deps.iter().filter_map(|d| d.dir.clone())).filter(|d| !known.contains(d)).collect();
            if unknown.is_empty() {
                break;
            }
            for dir in unknown {
                let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
                let out = Command::new(cargo)
                    .args(["metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path"])
                    .arg(tree.join(&dir).join("Cargo.toml"))
                    .current_dir(&tree)
                    .stdin(Stdio::null())
                    .output()?;
                let meta: Option<serde_json::Value> = out.status.success().then(|| serde_json::from_slice(&out.stdout).ok()).flatten();
                let Some(meta) = meta else {
                    let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
                    let reason: Vec<&str> = why.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                    let why = format!("cargo cannot read the package at {dir}: {}", reason.join(" / "));
                    unread.insert(dir, why);
                    continue;
                };
                // What cargo read for it lies in the commit's tree too, as the root's must.
                for pkg in meta["packages"].as_array().into_iter().flatten() {
                    let name = pkg["name"].as_str().unwrap_or_default();
                    if !inside(Path::new(pkg["manifest_path"].as_str().unwrap_or_default())) {
                        return Ok(Some(format!("commit {commit}: the package `{name}` lies outside the commit's tree")));
                    }
                    for dep in pkg["dependencies"].as_array().into_iter().flatten() {
                        if let Some(path) = dep["path"].as_str()
                            && !inside(Path::new(path))
                        {
                            let dep_name = dep["name"].as_str().unwrap_or_default();
                            return Ok(Some(format!("commit {commit}: the package `{name}` depends on `{dep_name}` by a path outside the commit's tree")));
                        }
                    }
                }
                let ws = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
                let found = packages_of(&meta, &from_root(&ws).unwrap_or_default());
                if found.iter().all(|m| m.dir != dir) {
                    unread.insert(dir.clone(), format!("cargo names no package at {dir}"));
                }
                for m in found.into_iter().filter(|m| m.dir == dir) {
                    if !members.iter().any(|x| x.dir == m.dir) {
                        members.push(m);
                    }
                }
            }
        }
        let workspace = inputs::Workspace { root, commit, tree: &tree, members, unread };
        // The release's target as rustc configures it (`unix`, `target_os="linux"`), which a
        // dependency's platform (`[target.'cfg(windows)'.dependencies]`) is read against.
        let printed = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--print", "cfg", "--target", crate::components::TARGET])
            .current_dir(&tree)
            .stdin(Stdio::null())
            .output()?;
        if !printed.status.success() {
            return Ok(Some(format!("commit {commit}: rustc cannot say the configuration of {}: {}", crate::components::TARGET, String::from_utf8_lossy(&printed.stderr).trim())));
        }
        let target_cfg: Vec<(String, Option<String>)> = String::from_utf8_lossy(&printed.stdout)
            .lines()
            .map(|l| match l.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.trim_matches('"').to_string())),
                None => (l.to_string(), None),
            })
            .collect();
        for pkg in meta["packages"].as_array().into_iter().flatten() {
            let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default());
            let Some(dir) = manifest.parent().and_then(from_root).filter(|d| d.starts_with("crates/")) else { continue };
            let name = pkg["name"].as_str().unwrap_or_default().to_string();
            let version = pkg["version"].as_str().unwrap_or_default().to_string();
            // Every bin ships, built with what its executables require (Q443), but one whose
            // features only the toolchain its package's folder names builds (Q449).
            // DECISION PENDING (Q449)
            let own_toolchain = ["rust-toolchain", "rust-toolchain.toml"].iter().any(|f| tree.join(&dir).join(f).is_file());
            // A required feature cargo can meet for a build of the bins: one cargo's metadata names
            // for the package (an optional dependency's own when no `dep:` hides it), or a
            // feature of one of its normal or build dependencies (`dep/feature`), as that
            // dependency's metadata names it for one by path, as cargo's own resolve answers for
            // one of the registry or git. A dev-dependency is in no build of the bins, nor one of
            // another platform than the release's target alone.
            let defined: BTreeSet<String> = pkg["features"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            let dependencies: Vec<&serde_json::Value> = pkg["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| d["kind"].as_str() != Some("dev"))
                .filter(|d| d["target"].as_str().is_none_or(|t| inputs::platform_holds(t, crate::components::TARGET, &target_cfg)))
                .collect();
            let named = |d: &serde_json::Value| d["rename"].as_str().or_else(|| d["name"].as_str()).unwrap_or_default().to_string();
            let defines = |f: &str| -> Result<bool, String> {
                let Some((d, feature)) = f.split_once('/') else { return Ok(defined.contains(f)) };
                let Some(dep) = dependencies.iter().find(|x| named(x) == d.trim_end_matches('?')) else { return Ok(false) };
                if let Some(path) = dep["path"].as_str() {
                    let dir = from_root(Path::new(path)).unwrap_or_default();
                    return Ok(workspace.members.iter().find(|m| m.dir == dir).is_some_and(|m| m.features.contains(feature)));
                }
                // Cargo's own word: its resolve with that feature on, offline, held to the lock.
                let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
                let mut c = Command::new(cargo);
                c.args(["tree", "--offline", "--depth", "0", "-e", "normal", "--prefix", "none", "--manifest-path"]).arg(tree.join("Cargo.toml"));
                if tree.join("Cargo.lock").is_file() {
                    c.arg("--locked");
                }
                let out = c.args(["-p", &format!("{name}@{version}"), "--features", f]).current_dir(&tree).stdin(Stdio::null()).output().map_err(|e| format!("commit {commit}: cargo tree does not run: {e}"))?;
                let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
                match out.status.success() {
                    true => Ok(true),
                    false if why.contains("does not have that feature") || why.contains("does not contain this feature") => Ok(false),
                    false => Err(format!("commit {commit}: whether cargo builds {name}'s bins requiring `{f}` cannot be told offline: {}", why.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" / "))),
                }
            };
            let (mut bins, mut features, mut unshipped, mut never) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), BTreeMap::new());
            for t in pkg["targets"].as_array().into_iter().flatten().filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "bin"))) {
                let Some(b) = t["name"].as_str() else { continue };
                let required: Vec<String> = t["required-features"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
                let mut unmet = None;
                for f in &required {
                    match defines(f) {
                        Ok(true) => {}
                        Ok(false) => {
                            unmet = Some(f.clone());
                            break;
                        }
                        Err(why) => return Ok(Some(why)),
                    }
                }
                if let Some(f) = unmet {
                    never.insert(b.to_string(), f);
                    continue;
                }
                if own_toolchain && !required.is_empty() {
                    unshipped.insert(b.to_string());
                    continue;
                }
                bins.insert(b.to_string());
                features.extend(required);
            }
            let lib = pkg["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"].contains(&k.as_str().unwrap_or_default()))))
                .and_then(|t| t["name"].as_str().map(String::from));
            let (inputs, packages) = match workspace.inputs_and_packages(&name) {
                Ok((i, p)) => (Ok(i), p),
                Err(e) => (Err(e), inputs::Packages::default()),
            };
            held.insert(name.clone(), Held { dir, name, version, bins, features, unshipped, never, lib, inputs, packages });
        }
        Ok(None)
    })();
    drop(scratch_dir);
    // Whatever went wrong reading this one commit is its refusal: it never aborts a check and
    // hides the refusals of other commits.
    match result {
        Ok(Some(r)) => refusals.push(r),
        Ok(None) => {}
        Err(e) => {
            held.clear();
            refusals.push(format!("commit {commit}: its tree could not be read: {e}"));
        }
    }
    Ok(held)
}

/// Checks a manifest against the repository at `root`: its commit there and an ancestor of HEAD,
/// every component the commit holds listed at the version it states and with exactly the
/// executables it builds, nothing else listed. Every refusal is returned, in a fixed order.
pub fn check(root: &Path, m: &Manifest) -> Result<Vec<String>, CheckError> {
    let mut refusals = Vec::new();
    // A shallow clone's history is cut: ancestry would be judged on what is missing (law L6
    // refuses one likewise).
    if git::text(root, &["rev-parse", "--is-shallow-repository"])?.is_some_and(|t| t.trim() == "true") {
        refusals.push("the repository is a shallow clone: its history is cut, and a release is checked against the whole of it".to_string());
        return Ok(refusals);
    }
    // A release names its commit by SHA-1's 40 characters; in a repository of other ids such a
    // string would be read as a ref or a prefix.
    let format = git::text(root, &["rev-parse", "--show-object-format"])?.map(|t| t.trim().to_string());
    if format.as_deref() != Some("sha1") {
        refusals.push(format!("the repository names its objects by {}, not sha1: a release's commit is a SHA-1 id", format.unwrap_or_else(|| "an unknown format".into())));
        return Ok(refusals);
    }
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
    if !git::is_commit_id(root, c)? {
        refusals.push(format!("{c} is not a commit's own id: a release names the commit itself"));
        return Ok(refusals);
    }
    if git::run(root, &["merge-base", "--is-ancestor", c, "HEAD"])?.is_none() {
        refusals.push(format!("commit {c} is not an ancestor of HEAD: a release is cut from the history it is committed in"));
    }
    let before = refusals.len();
    let held = components_at(root, c, &mut refusals)?;
    if refusals.len() > before {
        // The tree did not read: its refusal says why, and no component is judged against it.
        return Ok(refusals);
    }
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
            if let Some(f) = h.never.get(b) {
                refusals.push(format!("{}'s `{b}` requires `{f}`, which the package does not define: cargo builds it never at {c}", x.name));
            } else if h.unshipped.contains(b) {
                refusals.push(format!("{}'s `{b}` needs a feature only the toolchain its folder names builds at {c}: a release does not ship it (docs/questions.md Q449)", x.name));
            } else {
                refusals.push(format!("{} builds no `{b}` at {c}", x.name));
            }
        }
    }
    Ok(refusals)
}

/// Checks manifests against each other: a component's version names its build's inputs
/// ([`crate::inputs`], docs/questions.md Q441), so two releases pinning one version must hold the
/// same inputs for it at their commits; each difference is named.
pub fn check_together(root: &Path, all: &[(PathBuf, Manifest)]) -> Result<Vec<String>, CheckError> {
    let mut refusals = Vec::new();
    // (name, version) -> the first release seen pinning it, with its inputs there.
    let mut first: BTreeMap<(String, String), (String, Result<Inputs, String>)> = BTreeMap::new();
    for (_, m) in all {
        // A manifest whose commit is no commit here is `check`'s to refuse; it pins no tree.
        if !git::is_commit_id(root, &m.commit)? {
            continue;
        }
        let mut unread = Vec::new();
        let held = components_at(root, &m.commit, &mut unread)?;
        if !unread.is_empty() {
            // A tree that did not read pins nothing: `check` says why, and no other release is
            // charged with a difference against it.
            continue;
        }
        for x in &m.components {
            // Only a true pin names a tree: a component absent at the commit, or there at another
            // version, is `check`'s refusal and charges no other release.
            let Some(h) = held.get(&x.name).filter(|h| h.version == x.version.to_string()) else { continue };
            let key = (x.name.clone(), x.version.to_string());
            match first.get(&key) {
                // A first pin whose inputs do not read gives way to the next that does, so it
                // hides no conflict between later ones.
                Some((other, Err(why))) => {
                    let said = format!("{} {} is pinned by releases {other} and {}, and its build inputs at {other}'s commit do not read: {why}", x.name, x.version, m.release);
                    let own = h.inputs.as_ref().err().map(|own| format!("{} {} is pinned by releases {other} and {}, and its build inputs at {}'s commit do not read: {own}", x.name, x.version, m.release, m.release));
                    refusals.push(said);
                    // This one's own reason said too: no unread pin hides another's.
                    match own {
                        Some(own) => refusals.push(own),
                        None => {
                            first.insert(key, (m.release.to_string(), h.inputs.clone()));
                        }
                    }
                }
                None => {
                    first.insert(key, (m.release.to_string(), h.inputs.clone()));
                }
                Some((other, Ok(a))) => match &h.inputs {
                    Ok(b) => {
                        let differ = a.differ(b);
                        if !differ.is_empty() {
                            refusals.push(format!(
                                "{} {} is pinned by releases {other} and {} at different build inputs ({}): a version names everything its build reads",
                                x.name,
                                x.version,
                                m.release,
                                differ.join(", ")
                            ));
                        }
                    }
                    Err(why) => refusals.push(format!("{} {} is pinned by releases {other} and {}, and its build inputs at {}'s commit do not read: {why}", x.name, x.version, m.release, m.release)),
                },
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
