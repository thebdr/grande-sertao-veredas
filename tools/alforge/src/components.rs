//! The component list (docs/alforge-plan.md, "The component list"; docs/questions.md Q31 entry 38,
//! ruled "(b) Security file, component list"): what a release is made of, as a CycloneDX 1.6 JSON
//! document, the form vulnerability scanners read by package URL. It names the release, every
//! component at the commit with its version and licence, and every package the components' normal
//! and build dependencies reach as cargo resolves them for the release's target with every feature
//! of every member on (a superset of what one build compiles): a package by path outside the
//! components (a fork kept in the tree) by its folder, a crate by its package URL, the lock file's
//! checksum and its licence; with the dependencies between them.
//!
//! It is read from a commit alone (its tree as git stores it, cargo's own reading of its packages,
//! offline), so `alforge components <commit>` and the cut give one document for one commit and
//! release, byte for byte: the cut puts it in the bundle (`sbom.cdx.json`) and beside it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use thiserror::Error;

use crate::git;
use crate::inputs::{self, CRATES_IO};
use crate::manifest::{self, CheckError};

/// Why a commit's component list could not be made.
#[derive(Debug, Error)]
pub enum ListError {
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Check(#[from] CheckError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn refused(what: impl Into<String>) -> ListError {
    ListError::Refused(what.into())
}

/// One entry of the list: a component, a package by path or a locked crate.
struct Entry {
    bom_ref: String,
    kind: &'static str,
    name: String,
    version: String,
    licence: Licence,
    purl: Option<String>,
    sha256: Option<String>,
    properties: Vec<(String, String)>,
}

/// A package's licence as its manifest states it.
#[derive(Clone)]
enum Licence {
    Expression(String),
    File(String),
    None,
}

/// A package as cargo resolves it.
struct Package {
    name: String,
    version: String,
    /// Its source as the lock writes it; `None` for one by path.
    source: Option<String>,
    /// Its folder from the root, for one by path.
    dir: Option<String>,
    licence: Licence,
    /// Its normal and build dependencies on the target, by cargo's id.
    deps: BTreeSet<String>,
}

/// `text` percent-encoded for a package URL's qualifier: every byte but the unreserved ones.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// A locked crate's package URL: `pkg:cargo/<name>@<version>` for crates.io's; one of another
/// source names it (`?repository_url=` for another registry, `?vcs_url=<url>@<commit>` for git).
pub fn purl(name: &str, version: &str, source: &str) -> String {
    if source == CRATES_IO {
        return format!("pkg:cargo/{name}@{version}");
    }
    if let Some(git) = source.strip_prefix("git+") {
        let (url, commit) = git.split_once('#').unwrap_or((git, ""));
        let url = url.split_once('?').map_or(url, |(u, _)| u);
        return format!("pkg:cargo/{name}@{version}?vcs_url={}", encoded(&format!("git+{url}@{commit}")));
    }
    let registry = source.split_once('+').map_or(source, |(_, r)| r);
    format!("pkg:cargo/{name}@{version}?repository_url={}", encoded(registry))
}

/// The commit's packages as cargo resolves them for `target` with every feature of every member on
/// (`cargo metadata --all-features --filter-platform <target> --offline`, `--locked` to the lock
/// file the commit holds, in its tree,
/// written out as git stores it), by cargo's id: what any build of a member may compile there.
fn resolve_packages(root: &Path, commit: &str, target: &str) -> Result<BTreeMap<String, Package>, ListError> {
    let parent = crate::cache::dir()?.join("tmp");
    std::fs::create_dir_all(&parent)?;
    let scratch = tempfile::Builder::new().prefix("alforge-list-").tempdir_in(&parent)?;
    let tree = scratch.path().join("tree");
    std::fs::create_dir_all(&tree)?;
    git::export_tree(root, commit, &[], &tree).map_err(|e| refused(format!("commit {commit}: {e}")))?;
    // Cargo in the suite's own environment (crate::cut::suite_command): no variable of the
    // caller's that would steer cargo or rustc reads the commit another way.
    // Held to its lock file; a commit without one depends on no crate (C-290 refuses one that
    // would), so cargo's resolve of its packages by path is the whole of it.
    let manifest = tree.join("Cargo.toml").display().to_string();
    let mut argv = vec!["cargo", "metadata", "--format-version", "1", "--all-features", "--filter-platform", target, "--offline", "--manifest-path", &manifest];
    if tree.join("Cargo.lock").is_file() {
        argv.push("--locked");
    }
    let out = crate::cut::suite_command(&tree, &parent, &argv, std::env::vars_os())
    .stdin(std::process::Stdio::null())
    .output()?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
        let reason: Vec<&str> = why.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        return Err(refused(format!("commit {commit}: cargo cannot resolve its packages offline (`cargo fetch --locked` brings them): {}", reason.join(" / "))));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| refused(format!("commit {commit}: cargo's metadata does not read: {e}")))?;
    let base = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for n in meta["resolve"]["nodes"].as_array().into_iter().flatten() {
        let id = n["id"].as_str().unwrap_or_default().to_string();
        // A normal (`kind` null) or build dependency goes into what ships; a test's never does.
        let on = n["deps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["dep_kinds"].as_array().into_iter().flatten().any(|k| k["kind"].as_str() != Some("dev")))
            .filter_map(|d| d["pkg"].as_str().map(str::to_string))
            .collect();
        deps.insert(id, on);
    }
    let mut found = BTreeMap::new();
    for p in meta["packages"].as_array().into_iter().flatten() {
        let s = |k: &str| p[k].as_str().map(str::to_string);
        let (Some(id), Some(name), Some(version)) = (s("id"), s("name"), s("version")) else { continue };
        let source = s("source");
        let dir = PathBuf::from(s("manifest_path").unwrap_or_default());
        let dir = dir.parent().and_then(|d| d.strip_prefix(&base).ok()).map(|d| d.display().to_string());
        let licence = match (s("license"), s("license_file")) {
            (Some(l), _) => Licence::Expression(l),
            // A licence file, named from the root for a package by path.
            (None, Some(f)) => Licence::File(match (&source, &dir) {
                (None, Some(d)) if Path::new(&f).is_relative() => format!("{d}/{f}"),
                _ => f,
            }),
            (None, None) => Licence::None,
        };
        let dir = if source.is_none() { dir } else { None };
        found.insert(id.clone(), Package { name, version, source, dir, licence, deps: deps.remove(&id).unwrap_or_default() });
    }
    Ok(found)
}

/// The commit's time, in UTC as RFC 3339 writes it.
fn commit_time(root: &Path, commit: &str) -> Result<String, ListError> {
    let out = git::command(root)
        .env("TZ", "UTC")
        .args(["show", "-s", "--format=%cd", "--date=format-local:%Y-%m-%dT%H:%M:%SZ", commit])
        .output()?;
    if !out.status.success() {
        return Err(refused(format!("git cannot show {commit}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The document's serial number: a UUID of the release and the commit, so one commit and release
/// give one document.
fn serial(release: &str, commit: &str) -> String {
    let h = crate::cache::sha256_text(&format!("veredas component list\n{release}\n{commit}\n"));
    let mut b: Vec<char> = h[..32].chars().collect();
    // Version 8 (RFC 9562, a name of its own), variant 10.
    b[12] = '8';
    b[16] = ['8', '9', 'a', 'b'][usize::from_str_radix(&b[16].to_string(), 16).unwrap_or(0) % 4];
    let s: String = b.into_iter().collect();
    format!("urn:uuid:{}-{}-{}-{}-{}", &s[..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

/// The target a release is built for, whose resolve the list follows.
pub const TARGET: &str = "x86_64-unknown-linux-gnu";

/// The component list of `commit` in the repository at `root`, for `release` (`None`: the commit
/// alone, its id as the version), as the JSON text the cut writes: every component (C-290's) and
/// every package its normal and build dependencies reach as cargo resolves them for the release's
/// target with every feature on.
pub fn list(root: &Path, commit: &str, release: Option<&str>) -> Result<String, ListError> {
    let mut refusals = Vec::new();
    let held = manifest::components_at(root, commit, &mut refusals)?;
    if !refusals.is_empty() {
        return Err(refused(refusals.join("\n")));
    }
    let packages = resolve_packages(root, commit, TARGET)?;
    let lock_text = git::show(root, commit, "Cargo.lock").map_err(|e| refused(e.to_string()))?.unwrap_or_default();
    let lock = inputs::parse_lock(&lock_text).map_err(|e| refused(format!("commit {commit}: {e}")))?;
    // Each component's package, by its folder.
    let mut start: Vec<&str> = Vec::new();
    for h in held.values() {
        let id = packages.iter().find(|(_, p)| p.source.is_none() && p.dir.as_deref() == Some(h.dir.as_str())).map(|(id, _)| id.as_str());
        start.push(id.ok_or_else(|| refused(format!("commit {commit}: cargo resolves no package for {} at {}", h.name, h.dir)))?);
    }
    // Everything their normal and build dependencies reach.
    let mut reached: BTreeSet<&str> = start.iter().copied().collect();
    let mut queue = start.clone();
    while let Some(id) = queue.pop() {
        for d in packages.get(id).map(|p| &p.deps).into_iter().flatten() {
            if reached.insert(d.as_str()) {
                queue.push(d.as_str());
            }
        }
    }
    let component_dirs: BTreeMap<&str, &manifest::Held> = held.values().map(|h| (h.dir.as_str(), h)).collect();
    // Each reached package's reference: a component's and a package by path's by name and
    // version, a crate's by its package URL.
    let reference = |p: &Package| match &p.source {
        Some(src) => purl(&p.name, &p.version, src),
        None => format!("{}@{}", p.name, p.version),
    };
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut depends: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for id in &reached {
        let Some(p) = packages.get(*id) else { continue };
        let bom_ref = reference(p);
        let mut properties = Vec::new();
        let (kind, purl_of, sha256) = match &p.source {
            Some(src) => {
                let sum = lock.iter().find(|l| l.name == p.name && l.version == p.version && l.source.as_deref() == Some(src.as_str())).and_then(|l| l.checksum.clone());
                ("library", Some(bom_ref.clone()), sum)
            }
            None => {
                let dir = p.dir.clone().unwrap_or_default();
                properties.push(("veredas:commit".to_string(), commit.to_string()));
                properties.push(("veredas:path".to_string(), dir.clone()));
                match component_dirs.get(dir.as_str()) {
                    Some(h) if !h.bins.is_empty() => {
                        properties.push(("veredas:bins".to_string(), h.bins.iter().cloned().collect::<Vec<_>>().join(" ")));
                        ("application", None, None)
                    }
                    _ => ("library", None, None),
                }
            }
        };
        let on: BTreeSet<String> = p.deps.iter().filter(|d| reached.contains(d.as_str())).filter_map(|d| packages.get(d).map(reference)).collect();
        depends.entry(bom_ref.clone()).or_default().extend(on);
        entries.insert(bom_ref.clone(), Entry { bom_ref, kind, name: p.name.clone(), version: p.version.clone(), licence: p.licence.clone(), purl: purl_of, sha256, properties });
    }

    let version = release.map(str::to_string).unwrap_or_else(|| commit.to_string());
    let components: Vec<Value> = entries
        .values()
        .map(|e| {
            let mut c = json!({ "type": e.kind, "bom-ref": e.bom_ref, "name": e.name, "version": e.version });
            match &e.licence {
                Licence::Expression(x) => c["licenses"] = json!([{ "expression": x }]),
                Licence::File(f) => c["licenses"] = json!([{ "license": { "name": format!("the package's own file {f}") } }]),
                Licence::None => {}
            }
            if let Some(p) = &e.purl {
                c["purl"] = json!(p);
            }
            if let Some(h) = &e.sha256 {
                c["hashes"] = json!([{ "alg": "SHA-256", "content": h }]);
            }
            if !e.properties.is_empty() {
                c["properties"] = Value::Array(e.properties.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect());
            }
            c
        })
        .collect();
    let dependencies: Vec<Value> = depends.iter().map(|(r, on)| json!({ "ref": r, "dependsOn": on.iter().collect::<Vec<_>>() })).collect();
    let doc = json!({
        "$schema": "http://cyclonedx.org/schema/bom-1.6.schema.json",
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "serialNumber": serial(release.unwrap_or_default(), commit),
        "version": 1,
        "metadata": {
            "timestamp": commit_time(root, commit)?,
            "tools": { "components": [{ "type": "application", "name": "alforge", "version": env!("CARGO_PKG_VERSION") }] },
            "component": {
                "type": "application",
                "bom-ref": format!("veredas@{version}"),
                "name": "veredas",
                "version": version,
                "properties": [{ "name": "veredas:commit", "value": commit }],
            },
        },
        "components": components,
        "dependencies": dependencies,
    });
    let mut text = serde_json::to_string_pretty(&doc).map_err(|e| refused(e.to_string()))?;
    text.push('\n');
    Ok(text)
}
