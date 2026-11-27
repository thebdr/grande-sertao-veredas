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
    /// Its CPE, CycloneDX's own field for the name advisories give a product (Qt's).
    cpe: Option<String>,
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
    /// Whether it has a build script, without which cargo compiles no build-dependency of it.
    build_script: bool,
    /// Whether it has a library (a proc-macro's among them), without which cargo ignores a
    /// dependency on it.
    lib: bool,
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
fn resolve_packages(root: &Path, commit: &str, target: &str, components: &[(String, Vec<String>)]) -> Result<BTreeMap<String, Package>, ListError> {
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
    let mut argv = vec!["cargo", "metadata", "--format-version", "1", "--all-features", "--offline", "--manifest-path", &manifest];
    if tree.join("Cargo.lock").is_file() {
        argv.push("--locked");
    }
    let out = list_cargo(&tree, &parent, &argv).stdin(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
        let reason: Vec<&str> = why.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        return Err(refused(format!("commit {commit}: cargo cannot resolve its packages offline (`cargo fetch --locked` brings them): {}", reason.join(" / "))));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| refused(format!("commit {commit}: cargo's metadata does not read: {e}")))?;
    let base = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
    let mut found = BTreeMap::new();
    for p in meta["packages"].as_array().into_iter().flatten() {
        let s = |k: &str| p[k].as_str().map(str::to_string);
        let (Some(id), Some(name), Some(version)) = (s("id"), s("name"), s("version")) else { continue };
        let source = s("source");
        let manifest_path = PathBuf::from(s("manifest_path").unwrap_or_default());
        // A package cargo reads by path from outside the commit's tree: a `paths` override of the
        // caller's cargo configuration in place of a locked crate, which the builder's build, with
        // no configuration, never reads.
        if source.is_none() && !manifest_path.starts_with(&base) {
            return Err(refused(format!(
                "commit {commit}: cargo reads {name} {version} from {}, outside the commit's tree: a cargo configuration of the caller's (a `paths` override) puts it there, which the builder's build has none of",
                manifest_path.parent().unwrap_or(&manifest_path).display()
            )));
        }
        let dir = manifest_path.parent().and_then(|d| d.strip_prefix(&base).ok()).map(|d| d.display().to_string());
        // An empty or blank `license` or `license-file` states nothing (`license = ""`): none.
        let stated = |k: &str| s(k).filter(|v| !v.trim().is_empty());
        let licence = match (stated("license"), stated("license_file")) {
            (Some(l), _) => Licence::Expression(l),
            // A licence file, named from the root for a package by path (the root's own as it is).
            (None, Some(f)) => Licence::File(match (&source, &dir) {
                (None, Some(d)) if Path::new(&f).is_relative() && !d.is_empty() => format!("{d}/{f}"),
                _ => f,
            }),
            (None, None) => Licence::None,
        };
        let dir = if source.is_none() { dir } else { None };
        let kinds = |t: &Value| t["kind"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(str::to_string)).collect::<Vec<_>>();
        let targets: Vec<Vec<String>> = p["targets"].as_array().into_iter().flatten().map(kinds).collect();
        let build_script = targets.iter().any(|k| k.iter().any(|k| k == "custom-build"));
        let lib = targets.iter().any(|k| k.iter().any(|k| ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"].contains(&k.as_str())));
        found.insert(id.clone(), Package { name, version, source, dir, licence, deps: BTreeSet::new(), build_script, lib });
    }
    // What the components' builds compile, as cargo's own feature resolver has it (`cargo tree`
    // with normal and build edges alone, for the target, every feature of theirs on): a
    // dependency of another platform alone, the features a test's dependency alone asks for and a
    // weak feature (`dep?/f`) nothing else turns on reach nothing, as no build compiles them.
    // Each node by its name, version and source, as cargo prints it; every one printed whole where
    // it stands (`--no-dedupe`: this repository's 13 components print 156,000 lines in about a
    // second), so what lies below a dependency no build compiles is left with it and nothing a
    // build compiles is printed there alone; drawn in ASCII, which marks a package's
    // build-dependencies (`[build-dependencies]`, at the depth of its children).
    // Each component alone, as the builder builds it (docs/questions.md Q444: cargo unifies features
    // within its own inputs, never across components, so a weak feature one turns on and an
    // optional dependency another does make nothing there), every feature of its own on and the
    // dependencies' features its executables need (`serde_json/preserve_order`, which
    // `--all-features` never turns on; crate::builder::feature_args).
    let mut runs: Vec<Vec<String>> = Vec::new();
    let base_argv: Vec<String> = ["cargo", "tree", "--offline", "--all-features", "--target", target, "-e", "normal,build", "--no-dedupe", "--charset", "ascii", "--format", "{p}", "--manifest-path", &manifest].iter().map(|s| s.to_string()).collect();
    let locked = argv.contains(&"--locked");
    for (spec, needs) in components {
        let mut one = base_argv.clone();
        one.extend(["-p".to_string(), spec.clone()]);
        for f in needs {
            one.extend(["--features".to_string(), f.clone()]);
        }
        runs.push(one);
    }
    let mut printed: Vec<String> = Vec::new();
    for mut run in runs {
        if locked {
            run.push("--locked".into());
        }
        let words: Vec<&str> = run.iter().map(String::as_str).collect();
        let out = list_cargo(&tree, &parent, &words).stdin(std::process::Stdio::null()).output()?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).replace(&format!("{}/", tree.display()), "");
            let reason: Vec<&str> = why.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            return Err(refused(format!("commit {commit}: cargo cannot tree its packages offline: {}", reason.join(" / "))));
        }
        printed.push(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let by_name: BTreeMap<(String, String), Vec<String>> = found.iter().fold(BTreeMap::new(), |mut m, (id, p)| {
        m.entry((p.name.clone(), p.version.clone())).or_insert_with(Vec::new).push(id.clone());
        m
    });
    let identify = |line: &str| -> Result<Option<String>, ListError> {
        // `name vX`, then cargo's marks and the source as it prints them (`(proc-macro)` first,
        // `(<source>)` its parentheses alone taken off, a folder's own kept): a name and a version
        // hold no space. No ` (*)` follows: `--no-dedupe` prints none, and normal and build edges
        // make no cycle.
        let unread = || refused(format!("commit {commit}: cargo tree printed `{line}`"));
        let (name, rest) = line.split_once(" v").ok_or_else(unread)?;
        let (version, rest) = rest.split_once(' ').unwrap_or((rest, ""));
        let rest = rest.strip_prefix("(proc-macro)").map_or(rest, str::trim_start);
        let paren = match rest {
            "" => None,
            _ => Some(rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')).ok_or_else(unread)?),
        };
        let ids = by_name.get(&(name.to_string(), version.to_string())).cloned().unwrap_or_default();
        // The source cargo prints (crates.io's none; a path; a git repository's URL, its reference
        // and its commit's first 8 digits, `url?branch=main#8a2f5f93`; `registry `<url or name>``)
        // against the one its metadata holds, whole.
        let fits = |id: &&String| {
            let p = &found[*id];
            match (paren, &p.source) {
                (None, Some(src)) => src == CRATES_IO,
                (Some(path), None) => p.dir.as_ref().is_some_and(|d| Path::new(path) == base.join(d)),
                (Some(printed), Some(src)) => match (printed.strip_prefix("registry `"), src.strip_prefix("git+")) {
                    (Some(r), _) => {
                        // A registry the configuration names is printed by its name
                        // (`registry `outra``): any of another registry than crates.io fits it.
                        let r = r.trim_end_matches('`');
                        src != CRATES_IO && (src == r || src.strip_prefix("registry+") == Some(r) || (!r.contains("://") && (src.starts_with("registry+") || src.starts_with("sparse+"))))
                    }
                    (None, Some(git)) => {
                        // Its reference as cargo prints it, URL-encoded (`branch=fix%2Fx`), where
                        // its metadata holds it as it is (`branch=fix/x`).
                        let (at, commit) = git.rsplit_once('#').unwrap_or((git, ""));
                        let printed_as = match at.split_once('?').and_then(|(u, q)| q.split_once('=').map(|(k, v)| (u, k, v))) {
                            Some((u, k, v)) => format!("{u}?{k}={}", form_encoded(v)),
                            None => at.to_string(),
                        };
                        let (pat, short) = printed.rsplit_once('#').unwrap_or((printed, ""));
                        printed_as == pat && commit.starts_with(short)
                    }
                    (None, None) => false,
                },
                (None, None) => false,
            }
        };
        // A package cargo's metadata does not hold (a dependency on a package with no library,
        // which cargo ignores): no package of the build, nor anything below it.
        if ids.is_empty() {
            return Ok(None);
        }
        let picked: Vec<&String> = if ids.len() == 1 { ids.iter().collect() } else { ids.iter().filter(fits).collect() };
        match picked.as_slice() {
            [one] => Ok(Some((*one).clone())),
            _ => Err(refused(format!("commit {commit}: cargo tree printed `{line}`, which names {} of its packages", picked.len()))),
        }
    };
    // Each node printed, with the kind of the dependencies printed below it so far (a package's
    // normal ones first, then its build-dependencies after their mark); `None` for a node no build
    // compiles, and everything printed below it: a build-dependency of a package with no build
    // script (pest_meta's `sha2`, indoc's `rustversion`), a dependency on a package with no library
    // (which cargo ignores, and warns of), a package cargo's metadata does not hold.
    let mut stack: Vec<(Option<String>, bool)> = Vec::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    for raw in printed.iter().flat_map(|t| t.lines()) {
        let Some((depth, text)) = drawn(raw) else { continue };
        if let Some(mark) = text.strip_prefix('[') {
            // A mark of the dependencies printed after it, at the depth of their parent.
            let build = match mark {
                "build-dependencies]" => true,
                _ => return Err(refused(format!("commit {commit}: cargo tree printed `{text}`"))),
            };
            stack.truncate(depth + 1);
            if let Some(frame) = stack.get_mut(depth) {
                frame.1 = build;
            }
            continue;
        }
        stack.truncate(depth);
        let id = match stack.last() {
            None if depth == 0 => identify(text)?,
            Some((Some(parent), build)) if stack.len() == depth => match identify(text)? {
                Some(child) => {
                    let (f, t) = (&found[parent], &found[&child]);
                    (t.lib && (!*build || f.build_script)).then(|| {
                        edges.push((parent.clone(), child.clone()));
                        child
                    })
                }
                None => None,
            },
            _ => None,
        };
        stack.push((id, false));
    }
    for (from, to) in edges {
        if let Some(p) = found.get_mut(&from) {
            p.deps.insert(to);
        }
    }
    Ok(found)
}

/// A git reference's value as cargo prints it in a source (the `application/x-www-form-urlencoded`
/// bytes: letters, digits and `*-._` as they are, a space as `+`, any other byte `%XX`).
fn form_encoded(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'*' | b'-' | b'.' | b'_' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Cargo as the list runs it: in the suite's own environment (crate::cut::suite_command), so no
/// variable of the caller's steers it, and with what a configuration of the caller's would steer
/// the graph by set aside (`build.rustflags` and `target.*.rustflags`, whose `--cfg` turns a
/// platform's dependencies on; `build.rustc` and its wrappers, which answer what the target is), as
/// the builder's build runs with none (crate::builder writes it the vendored sources alone). A
/// `links` override reaches no `cargo tree`, which prints the build script's dependencies the
/// builder compiles.
fn list_cargo(cwd: &Path, tmp: &Path, argv: &[&str]) -> std::process::Command {
    let mut c = crate::cut::suite_command(cwd, tmp, argv, std::env::vars_os());
    c.env("CARGO_ENCODED_RUSTFLAGS", "").env("RUSTC", "rustc").env("RUSTC_WRAPPER", "").env("RUSTC_WORKSPACE_WRAPPER", "").env("CARGO_TERM_COLOR", "never");
    c
}

/// A line `cargo tree --charset ascii` printed, read as its depth and what follows the drawing: a
/// package at depth 0 bare, below it after a group of four characters per depth, the last `|-- `
/// or `` `-- ``; a mark of a package's dependencies (`[build-dependencies]`) after the groups of
/// its own depth's children, `|   ` or four spaces each. `None` for a blank line between two roots.
fn drawn(raw: &str) -> Option<(usize, &str)> {
    if raw.trim().is_empty() {
        return None;
    }
    let mut rest = raw;
    let mut depth = 0;
    loop {
        if let Some(r) = rest.strip_prefix("|-- ").or_else(|| rest.strip_prefix("`-- ")) {
            return Some((depth + 1, r));
        }
        match rest.strip_prefix("|   ").or_else(|| rest.strip_prefix("    ")) {
            Some(r) => {
                rest = r;
                depth += 1;
            }
            None => return Some((depth, rest)),
        }
    }
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

/// The document's serial number: a name-based UUID of RFC 4122's version 5 (SHA-1 of the URL
/// namespace and a name made of the release and the commit), so one commit and release give one
/// document, and CycloneDX 1.6's own pattern (versions 1 to 5) takes it.
pub fn serial(release: &str, commit: &str) -> String {
    use sha1::{Digest, Sha1};
    // RFC 4122's namespace for URLs, 6ba7b811-9dad-11d1-80b4-00c04fd430c8.
    const URL: [u8; 16] = [0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8];
    let mut h = Sha1::new();
    h.update(URL);
    h.update(format!("urn:veredas:components:{release}:{commit}").as_bytes());
    let d = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let x: String = b.iter().map(|v| format!("{v:02x}")).collect();
    format!("urn:uuid:{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32])
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
    // What C-290 and the cut refuse, the list refuses: a component whose inputs do not read (a
    // crate no lock file pins among them).
    let unread: Vec<String> = held.values().filter_map(|h| h.inputs.as_ref().err().map(|why| format!("{}'s build inputs do not read: {why}", h.name))).collect();
    if !unread.is_empty() {
        return Err(refused(unread.join("\n")));
    }
    // Each by its name and version (`-p veredas-core@2.1.0`): a package by path of the same name at
    // another version may share the graph. Only one the builder builds something of (its library,
    // or executables it ships): one of examples, tests or benches alone compiles nothing, nor what
    // it depends on; it is listed itself all the same. With the dependencies' features its
    // executables need.
    let specs: Vec<(String, Vec<String>)> = held
        .values()
        .filter(|h| h.lib.is_some() || !h.bins.is_empty())
        .map(|h| (format!("{}@{}", h.name, h.version), h.features.iter().filter(|f| f.contains('/')).cloned().collect()))
        .collect();
    let packages = resolve_packages(root, commit, TARGET, &specs)?;
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
        entries.insert(bom_ref.clone(), Entry { bom_ref, kind, name: p.name.clone(), version: p.version.clone(), licence: p.licence.clone(), purl: purl_of, cpe: None, sha256, properties });
    }

    // Qt and KDDockWidgets, which the bundle carries for Oficina's window (docs/questions.md Q446),
    // at the versions the commit's recipe names: the release's own, not a crate's.
    let mut carried: Vec<String> = Vec::new();
    if let Some(folder) = crate::folder_at(root, commit).map_err(|e| refused(e.to_string()))?
        && let Some(text) = git::show(root, commit, &format!("{folder}/maquina/maquina.toml")).map_err(|e| refused(e.to_string()))?
        && let Some(q) = crate::machine::Recipe::load(&text).map_err(|e| refused(format!("commit {commit}: its recipe: {e}")))?.builder.qt
    {
        for (name, version, licence, cpe) in [
            ("qt", q.version.as_str(), "LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only", Some(format!("cpe:2.3:a:qt:qt:{}:*:*:*:*:*:*:*", q.version))),
            ("kddockwidgets", q.kddockwidgets.as_str(), "GPL-2.0-only OR GPL-3.0-only", None),
        ] {
            let bom_ref = format!("pkg:generic/{name}@{version}");
            let properties = vec![("veredas:carried".to_string(), "lib/qt".to_string())];
            entries.insert(bom_ref.clone(), Entry { bom_ref: bom_ref.clone(), kind: "library", name: name.to_string(), version: version.to_string(), licence: Licence::Expression(licence.to_string()), purl: Some(bom_ref.clone()), cpe, sha256: None, properties });
            depends.entry(bom_ref.clone()).or_default();
            carried.push(bom_ref);
        }
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
            if let Some(cpe) = &e.cpe {
                c["cpe"] = json!(cpe);
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
    let mut dependencies: Vec<Value> = depends.iter().map(|(r, on)| json!({ "ref": r, "dependsOn": on.iter().collect::<Vec<_>>() })).collect();
    carried.sort();
    if !carried.is_empty() {
        dependencies.push(json!({ "ref": format!("veredas@{version}"), "dependsOn": carried }));
    }
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
