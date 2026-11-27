//! What a component's version names in a release (docs/questions.md Q441, ruled 2026-11-25 "All
//! build inputs"): everything a build of its executables reads but the toolchain, so a change to
//! any of it bumps the version. Its folder and the folder of every member it uses (its normal and
//! build dependencies by path, transitively; a test's never goes into an executable), each but its
//! top `tests/`, `benches/` and `examples/` (Q445); every file outside those folders their sources
//! name by a literal `#[path]`, `include_str!`, `include_bytes!` or `include!`; the lock file's
//! entries for its external dependencies, transitively; what cargo reads of the workspace's
//! manifest to build it (Q442); and the cargo configuration at the root. Each input is named and
//! identified as git stores it at the commit (a folder by its entries, a file by its blob, a part
//! of the manifest by its value, a locked package by its source and checksum), so two commits are
//! compared without building either.
//!
//! The cut builds each component alone, so cargo unifies features within its own inputs and no
//! other component's manifest reaches its executables, and holds what cargo's dep-info says each
//! compilation read to these inputs ([`read_outside`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component as Part, Path, PathBuf};

use crate::git;

/// The registry every crate of the suite comes from, left out of a locked package's name.
pub(crate) const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// A component's build inputs: each named (a folder ending in `/`, a file by its path, a locked
/// package as `Cargo.lock: <name> <version>`, a part of the workspace's manifest as
/// `Cargo.toml: <key>`) with its identity at the commit. A folder whose identity is [`LEFT_OUT`] is
/// one of an input folder's top `tests/`, `benches/` and `examples/`, which it does not name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs(pub BTreeMap<String, String>);

/// The identity of a package folder's top `tests/`, `benches/` or `examples/` folder: what only the
/// tests read never goes into an executable (docs/questions.md Q445, ruled 2026-11-26 "Tests left
/// out"), unless a shipped target starts there.
pub const LEFT_OUT: &str = "left out: its tests' own";

/// The top folders of a package that hold its tests, benches and examples, as cargo finds them.
pub const TEST_FOLDERS: [&str; 3] = ["tests", "benches", "examples"];

/// Whether an input's name is no file of the tree: a lock file's entry or a part of the manifest.
fn is_part(name: &str) -> bool {
    name.starts_with("Cargo.lock: ") || name.starts_with("Cargo.toml: ")
}

impl Inputs {
    /// The inputs whose identity differs between the two (one there and not in the other included),
    /// in name order. A folder left out ([`LEFT_OUT`]) is no input: a test folder added or removed
    /// charges nothing, and its package's folder says whether what ships moved.
    pub fn differ(&self, other: &Inputs) -> Vec<String> {
        let named = |i: &Inputs, n: &str| i.0.get(n).filter(|v| *v != LEFT_OUT).cloned();
        let names: BTreeSet<&String> = self.0.keys().chain(other.0.keys()).collect();
        names.into_iter().filter(|n| named(self, n) != named(other, n)).cloned().collect()
    }

    /// Whether a file of the tree (its path from the root) is one of these inputs or lies in one of
    /// their folders, and not in a folder they leave out ([`LEFT_OUT`]) unless named as a file.
    pub fn covers(&self, path: &str) -> bool {
        // A lock file's entry (`Cargo.lock: <name> <version>`) and a part of the manifest
        // (`Cargo.toml: <key>`) are no file of the tree.
        if !is_part(path) && self.0.get(path).is_some_and(|v| v != LEFT_OUT) {
            return true;
        }
        // The innermost folder holding it decides (a folder given by its own path, a build script's
        // `rerun-if-changed=.`, is that folder): an input folder covers it, a test folder of one
        // does not.
        self.0
            .iter()
            .filter(|(k, _)| k.ends_with('/') && (path.starts_with(k.as_str()) || k.strip_suffix('/') == Some(path)))
            .max_by_key(|(k, _)| k.len())
            .is_some_and(|(_, v)| v != LEFT_OUT)
    }
}

/// The packages a component's build compiles: what of the output folder and the vendored sources
/// the builds share its build may read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packages {
    /// Every package's name: a build script's output is `build/<name>/<16 hexadecimal>/` as cargo
    /// 1.100 writes it, `build/<name>-<16 hexadecimal>/` as cargo wrote it before.
    pub names: BTreeSet<String>,
    /// The folder `cargo vendor --versioned-dirs` gives each locked one: `<name>-<version>`.
    pub vendored: BTreeSet<String>,
}

/// A workspace member as cargo reads it (`cargo metadata --no-deps`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub version: String,
    /// Its folder, from the root (`crates/veredas-portal`).
    pub dir: String,
    /// The folder of the workspace cargo reads it in: empty for the repository's own, another for a
    /// path dependency kept in the tree outside it (`forks/ironcalc`).
    pub workspace: String,
    /// What goes into its executables: its normal and build dependencies, never its tests'.
    pub deps: Vec<Dep>,
    /// The source files its library, executables and build script start from (cargo's
    /// `src_path`), from the root: one may lie outside its folder (`[lib] path = "../x.rs"`).
    pub sources: Vec<String>,
    /// Whether it has a build script, which may read anything in its folder.
    pub build_script: bool,
    /// Its features as cargo's metadata names them (an optional dependency's own among them when
    /// no `dep:` hides it).
    pub features: BTreeSet<String>,
}

/// A normal or build dependency of a member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dep {
    /// The package's own name (a renamed dependency's `package`).
    pub name: String,
    /// The folder, from the root, of a dependency by path; `None` for one the lock file pins.
    pub dir: Option<String>,
    /// The version requirement it states (`^1`), which tells two locked versions of one name apart.
    pub req: String,
}

/// A package of the lock file.
#[derive(Clone, Debug)]
pub(crate) struct Locked {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
    pub(crate) checksum: Option<String>,
    pub(crate) dependencies: Vec<String>,
    /// What a `[replace]` puts in its place (`name version`, a package by path).
    pub(crate) replace: Option<String>,
}

/// A lock file's packages, as `Cargo.lock`'s text holds them.
pub(crate) fn parse_lock(text: &str) -> Result<Vec<Locked>, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| format!("its Cargo.lock does not read: {}", e.message()))?;
    // A lock of format 1 (which cargo still writes for a `rust-version` below 1.41) keeps each
    // package's checksum in `[metadata]`, as `"checksum <name> <version> (<source>)"`.
    let metadata = table.get("metadata").and_then(|m| m.as_table());
    let mut out = Vec::new();
    for p in table.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
        let s = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(name), Some(version)) = (s("name"), s("version")) else {
            return Err("its Cargo.lock holds a package without a name or a version".to_string());
        };
        let dependencies = p.get("dependencies").and_then(|d| d.as_array()).into_iter().flatten().filter_map(|d| d.as_str().map(str::to_string)).collect();
        let source = s("source");
        let checksum = s("checksum").or_else(|| {
            let src = source.as_deref()?;
            metadata.and_then(|m| m.get(&format!("checksum {name} {version} ({src})"))).and_then(|v| v.as_str()).map(str::to_string)
        });
        out.push(Locked { name, version, source, checksum, dependencies, replace: s("replace") });
    }
    Ok(out)
}

impl Locked {
    pub(crate) fn named(&self) -> String {
        match self.source.as_deref() {
            None | Some(CRATES_IO) => format!("Cargo.lock: {} {}", self.name, self.version),
            Some(s) => format!("Cargo.lock: {} {} ({s})", self.name, self.version),
        }
    }
}

/// A workspace at one commit, its tree written out at `tree`, from which each member's inputs are
/// read.
pub struct Workspace<'a> {
    pub root: &'a Path,
    pub commit: &'a str,
    pub tree: &'a Path,
    /// The workspace's members, then every package by path outside them that one of them uses.
    pub members: Vec<Member>,
    /// The folders of packages by path cargo could not read, with its reason.
    pub unread: BTreeMap<String, String>,
}

impl Workspace<'_> {
    fn object(&self, path: &str) -> Result<String, String> {
        git::object(self.root, self.commit, path)
            .map(|o| o.unwrap_or_else(|| "absent".to_string()))
            .map_err(|e| format!("git cannot name {path} at {}: {e}", self.commit))
    }

    /// The lock file's packages; `None` when the commit holds no `Cargo.lock`.
    fn lock(&self) -> Result<Option<Vec<Locked>>, String> {
        let text = match std::fs::read_to_string(self.tree.join("Cargo.lock")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("its Cargo.lock does not read: {e}")),
        };
        parse_lock(&text).map(Some)
    }

    /// The build inputs of the member `name`, or why they cannot be read.
    pub fn inputs(&self, name: &str) -> Result<Inputs, String> {
        self.inputs_and_packages(name).map(|(i, _)| i)
    }

    /// [`Workspace::inputs`], and the names of every package its build compiles (the component,
    /// the members and packages by path it uses, the locked ones), whose own build output and
    /// vendored sources its build may read.
    pub fn inputs_and_packages(&self, name: &str) -> Result<(Inputs, Packages), String> {
        let lock = self.lock()?;
        let mut named = BTreeMap::new();
        // A cargo configuration at the root (the cut refuses one before cargo reads it).
        for f in [".cargo/config.toml", ".cargo/config"] {
            named.insert(f.to_string(), self.object(f)?);
        }
        // The packages by path it uses, transitively: those it names by path, and those the lock
        // file reaches through an external crate (a `[patch]` to a member or to a fork kept in the
        // tree), each read alike: its folder, its workspace's manifest, its own dependencies by
        // its metadata (normal and build, never a member's dev-dependencies, which its lock entry
        // holds too), and the files its sources name. The lock file's entries for the external
        // ones, transitively.
        let by_dir: BTreeMap<&str, &Member> = self.members.iter().map(|m| (m.dir.as_str(), m)).collect();
        let start = self.members.iter().find(|m| m.name == name).ok_or_else(|| format!("{name} is no member"))?;
        let mut used: Vec<&Member> = vec![start];
        let mut seen: BTreeSet<&str> = BTreeSet::from([start.dir.as_str()]);
        // Each lock entry to read, with the crate of the registry or git it was reached through
        // (none for one a package by path names itself).
        let mut queue: Vec<(usize, Option<String>)> = Vec::new();
        // Each lock entry with a source read once as reached by a package by path and once as
        // reached behind a crate of the registry or git: a `[replace]` takes how it was reached.
        let mut taken: BTreeSet<(usize, bool)> = BTreeSet::new();
        // The packages by path reached only behind a crate of the registry or git, so far: their
        // folder, their name and version, and that crate.
        let mut behind: BTreeMap<&str, (String, String)> = BTreeMap::new();
        let mut i = 0;
        loop {
            if i < used.len() {
                let m = used[i];
                i += 1;
                for d in &m.deps {
                    let Some(dir) = d.dir.as_deref() else { continue };
                    let Some(other) = by_dir.get(dir) else {
                        return Err(self.unread.get(dir).cloned().unwrap_or_else(|| format!("{} depends on `{}` by a path ({dir}) where cargo names no package", m.name, d.name)));
                    };
                    if seen.insert(dir) {
                        used.push(other);
                    }
                }
                // By name and requirement: a test's other version of the same crate charges none.
                let wanted_by: Vec<(&str, Option<semver::VersionReq>)> =
                    m.deps.iter().filter(|d| d.dir.is_none()).map(|d| (d.name.as_str(), semver::VersionReq::parse(&d.req).ok())).collect();
                let wanted: BTreeSet<&str> = wanted_by.iter().map(|(n, _)| *n).collect();
                if wanted.is_empty() {
                    continue;
                }
                let Some(lock) = &lock else {
                    return Err(format!("{} depends on `{}` and the commit holds no Cargo.lock to pin it", m.name, wanted.iter().next().copied().unwrap_or_default()));
                };
                let Some(own) = lock.iter().find(|l| l.name == m.name && l.version == m.version && l.source.is_none()) else {
                    // A package of another workspace (a fork kept in the tree) that the lock leaves
                    // out is not compiled: an optional dependency nothing turns on. It uses nothing
                    // of the lock (its folder stays charged, which fails safe). A member of the
                    // workspace is always locked, so one missing names another workspace's lock.
                    if !m.workspace.is_empty() {
                        continue;
                    }
                    return Err(format!("its Cargo.lock does not pin {} {}: the lock file is not this workspace's", m.name, m.version));
                };
                for d in &own.dependencies {
                    let at = resolve(lock, d)?;
                    let l = &lock[at];
                    // `*` is cargo's "any version" for a git or path dependency stated without one,
                    // which a pre-release meets too (SemVer's `*` would refuse it).
                    let meets = |req: &Option<semver::VersionReq>| match (req, semver::Version::parse(&l.version)) {
                        (Some(r), Ok(v)) => r.comparators.is_empty() || r.matches(&v),
                        _ => true,
                    };
                    if wanted_by.iter().any(|(n, req)| *n == l.name && meets(req)) {
                        queue.push((at, None));
                    }
                }
                continue;
            }
            let Some((at, through)) = queue.pop() else { break };
            let lock = lock.as_ref().expect("a package was resolved in it");
            let l = &lock[at];
            if l.source.is_some() && !taken.insert((at, through.is_some())) {
                continue;
            }
            if l.source.is_none() {
                // A package by path the workspace knows (a member, or a fork one uses by path, as
                // a `[patch]` may name it too): used as one reached by path.
                let known = match self.members.iter().filter(|m| m.name == l.name && m.version == l.version).collect::<Vec<_>>()[..] {
                    [one] => one,
                    [] => return Err(format!("its Cargo.lock's {} {} has no source and is no member a release builds from its folder", l.name, l.version)),
                    ref many => return Err(format!("its Cargo.lock's {} {} has no source and names more than one package by path ({}): which one cargo builds cannot be told", l.name, l.version, many.iter().map(|m| m.dir.as_str()).collect::<Vec<_>>().join(", "))),
                };
                // Behind a crate of the registry or git (a `[patch]` of a crate that one uses), cargo's
                // dep-info does not follow it: what its build reads beyond what its sources name (a
                // build script's reads, an include by `concat!`) would be judged by no one. Refused
                // once the walk is done, each one named.
                if let Some(via) = through {
                    behind.entry(known.dir.as_str()).or_insert((format!("{} {}", l.name, l.version), via));
                    continue;
                }
                if seen.insert(known.dir.as_str()) {
                    used.push(known);
                }
                continue;
            }
            named.insert(l.named(), l.checksum.clone().unwrap_or_default());
            for d in &l.dependencies {
                queue.push((resolve(lock, d)?, Some(format!("{} {}", l.name, l.version))));
            }
            // A `[replace]` builds another package in its place: the one without a source.
            // (By path: the one of that name and version without a source; else as cargo names it.)
            if let Some(r) = &l.replace {
                let at = if r.contains(" (") {
                    resolve(lock, r).ok()
                } else {
                    let (n, v) = r.split_once(' ').unwrap_or((r.as_str(), ""));
                    lock.iter().position(|x| x.name == n && x.version == v && x.source.is_none())
                };
                // In that package's place, reached as it was.
                queue.push((at.ok_or_else(|| format!("its Cargo.lock replaces {} {} by `{r}`, which it does not pin", l.name, l.version))?, through.clone()));
            }
        }
        // Even one a package by path also uses: behind a proc-macro or a build-dependency of the
        // registry, cargo resolves it apart and compiles it a second time, in no dep-info judged.
        let unfollowed: Vec<String> = behind.values().map(|(what, via)| format!("{what} through {via}")).collect();
        if !unfollowed.is_empty() {
            return Err(format!(
                "its Cargo.lock reaches a package by path through a crate of the registry or git ({}): cargo's dep-info does not follow it there, so its build's reads would go unjudged",
                unfollowed.join(", ")
            ));
        }
        // Each folder but its top tests, benches and examples, unless a shipped target starts there
        // (docs/questions.md Q445, ruled 2026-11-26 "Tests left out").
        let dirs: BTreeSet<&str> = used.iter().map(|m| m.dir.as_str()).collect();
        // Every file rustc reads as Rust from these packages: a test folder a module of one at a
        // package's top can reach is not left out.
        let mut left: Vec<String> = Vec::new();
        for m in &used {
            // A package nested in another a build uses: the outer one's modules (`imp.rs` beside
            // `imp/`, an inline `mod imp {}`) and build script can read its test folders.
            let nested = dirs.iter().any(|d| *d != m.dir && m.dir.starts_with(&prefix(d)));
            if !nested {
                left.extend(left_out(self.tree, m, &dirs));
            }
        }
        // Then what rustc may read as Rust from everything not left out, until it reaches no
        // folder left out: a test folder a module, a `#[path]` or an `include!` reaches is the
        // package's like the rest, and so is every test folder of a package one of whose files at
        // its top rustc may read.
        loop {
            let reached = self.read_as_rust(&used, &left)?.rust;
            let kept: Vec<String> = left
                .iter()
                .filter(|f| {
                    let top = f.rsplitn(3, '/').nth(2).unwrap_or_default().to_string();
                    reached.iter().any(|r| format!("{r}/").starts_with(f.as_str()) || Path::new(r).parent() == Some(Path::new(&top)))
                })
                .cloned()
                .collect();
            if kept.is_empty() {
                break;
            }
            left.retain(|f| !kept.contains(f));
        }
        // A package's folder without its test folders and without the packages a build uses
        // inside it, each of which has a key of its own.
        let mut skip: Vec<String> = left.clone();
        skip.extend(dirs.iter().map(|d| prefix(d)));
        for m in &used {
            let own = prefix(&m.dir);
            let skip: Vec<String> = skip.iter().filter(|s| **s != own).cloned().collect();
            named.insert(format!("{}/", m.dir), self.folder_id(&m.dir, &skip)?);
        }
        for f in &left {
            named.entry(f.clone()).or_insert_with(|| LEFT_OUT.to_string());
        }
        for m in &used {
            // A package of another workspace inherits keys from that workspace's manifest, read
            // whole.
            if !m.workspace.is_empty() {
                let f = format!("{}/Cargo.toml", m.workspace);
                let id = self.object(&f)?;
                named.insert(f, id);
            }
        }
        // What cargo reads of the workspace's own manifest to build it (docs/questions.md Q442,
        // ruled 2026-11-26 "Only what cargo reads for it").
        named.extend(self.manifest_parts(&used)?);
        // Every file and folder rustc may read from what is not left out that lies outside those
        // folders, or in a folder left out: a target's source, a module, a folder a `#[path]`
        // names (by its tree), a file of data.
        let folders: Vec<String> = used.iter().map(|m| format!("{}/", m.dir)).collect();
        let inside = |path: &str| folders.iter().any(|f| path.starts_with(f.as_str())) && !left.iter().any(|l| format!("{path}/").starts_with(l.as_str()));
        let reading = self.read_as_rust(&used, &left)?;
        let outside: BTreeSet<String> = reading.rust.into_iter().chain(reading.data).filter(|p| !inside(p)).collect();
        for path in outside {
            let id = self.object(&path)?;
            named.insert(path, id);
        }
        let mut packages = Packages { names: used.iter().map(|m| m.name.clone()).collect(), vendored: BTreeSet::new() };
        if let Some(lock) = &lock {
            for &(at, _) in &taken {
                let l = &lock[at];
                packages.names.insert(l.name.clone());
                if l.source.is_some() {
                    packages.vendored.insert(format!("{}-{}", l.name, l.version));
                }
            }
        }
        Ok((Inputs(named), packages))
    }

    /// The identity of a package's folder at the commit: its entries as git stores them, but the
    /// folders `left` leaves out (its top tests, benches and examples), so a change there charges
    /// nothing.
    fn folder_id(&self, dir: &str, skip: &[String]) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let spec = format!("{}:{dir}", self.commit);
        let listing = git::run(self.root, &["ls-tree", "-z", &spec])
            .map_err(|e| format!("git cannot name {dir} at {}: {e}", self.commit))?
            .ok_or_else(|| format!("git cannot list {dir} at {}", self.commit))?;
        let mut h = Sha256::new();
        for entry in listing.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let text = String::from_utf8_lossy(entry);
            let (info, name) = text.split_once('\t').unwrap_or_default();
            if info.split_whitespace().nth(1) == Some("tree") {
                let sub = format!("{}{name}/", prefix(dir));
                // A folder left out, or a package a build uses with a key of its own: not this
                // one's. A folder holding one deeper: identified without it, in turn.
                if skip.contains(&sub) {
                    continue;
                }
                if skip.iter().any(|s| s.starts_with(&sub)) {
                    let inner = self.folder_id(&sub[..sub.len() - 1], skip)?;
                    h.update(format!("{} {inner}\t{name}", info.split_whitespace().next().unwrap_or_default()).as_bytes());
                    h.update([0]);
                    continue;
                }
            }
            h.update(entry);
            h.update([0]);
        }
        Ok(format!("{:x}", h.finalize()))
    }

    /// What cargo reads of the workspace's manifest (`Cargo.toml` at the root) to build the packages
    /// `used`, each part named `Cargo.toml: <key>` and identified by its value (its text's layout
    /// and comments apart): the resolver, every profile, `[patch]` and `[replace]` whole, a
    /// `cargo-features` line, and each `[workspace.package]` key, `[workspace.dependencies]` entry
    /// and `[workspace.lints]` table one of them inherits from it, in its package, its normal and
    /// build dependencies (by target too) and its lints. Its member list, its other tables and what
    /// other members alone inherit are not read for them.
    fn manifest_parts(&self, used: &[&Member]) -> Result<BTreeMap<String, String>, String> {
        let read = |file: &str| -> Result<toml::Table, String> {
            let text = std::fs::read_to_string(self.tree.join(file)).map_err(|e| format!("its {file} does not read: {e}"))?;
            toml::from_str(&text).map_err(|e| format!("its {file} does not read: {}", e.message()))
        };
        let root = read("Cargo.toml")?;
        let table = |t: &toml::Table, k: &str| t.get(k).and_then(|v| v.as_table()).cloned();
        // `[project]` is cargo's older name for `[package]`, which it still reads.
        let package_of = |t: &toml::Table| table(t, "package").or_else(|| table(t, "project"));
        let ws = table(&root, "workspace").unwrap_or_default();
        let package = table(&ws, "package").unwrap_or_default();
        let resolver = effective_resolver(&root, &ws, &package);
        if resolver == "1" {
            return Err(
                "its workspace resolves features with resolver 1 (no `resolver` stated in a virtual workspace, or a root package of edition 2015 or 2018): a test's dependencies and the root package's then turn features on in what ships, which no version names; a release's workspace resolves with 2 or later".to_string(),
            );
        }
        let mut out = BTreeMap::new();
        let mut part = |key: String, value: Option<&toml::Value>| {
            out.insert(format!("Cargo.toml: {key}"), value.map(value_id).unwrap_or_else(|| "absent".to_string()));
        };
        // The resolver cargo uses, wherever it comes from (the workspace's, the root package's
        // own, or its edition's).
        part("resolver".into(), Some(&toml::Value::String(resolver)));
        // Each named where the manifest states it: one stated at one commit alone differs.
        for (key, value) in [("cargo-features", root.get("cargo-features")), ("patch", root.get("patch")), ("replace", root.get("replace"))] {
            if value.is_some() {
                part(key.into(), value);
            }
        }
        for (name, profile) in table(&root, "profile").unwrap_or_default() {
            part(format!("profile.{name}"), Some(&profile));
        }
        let inherits = |v: &toml::Value| v.as_table().and_then(|t| t.get("workspace")).and_then(|w| w.as_bool()) == Some(true);
        let dependencies = table(&ws, "dependencies").unwrap_or_default();
        for m in used.iter().filter(|m| m.workspace.is_empty()) {
            let own = read(&format!("{}Cargo.toml", prefix(&m.dir)))?;
            for (k, v) in package_of(&own).unwrap_or_default() {
                if inherits(&v) {
                    part(format!("workspace.package.{k}"), package.get(&k));
                }
            }
            // Its normal and build dependencies, by target too; a test's never goes into an
            // executable.
            let mut tables: Vec<toml::Table> = ["dependencies", "build-dependencies", "build_dependencies"].iter().filter_map(|k| table(&own, k)).collect();
            for (_, t) in table(&own, "target").unwrap_or_default() {
                if let Some(t) = t.as_table() {
                    tables.extend(["dependencies", "build-dependencies", "build_dependencies"].iter().filter_map(|k| table(t, k)));
                }
            }
            for t in tables {
                for (name, v) in t {
                    if inherits(&v) {
                        part(format!("workspace.dependencies.{name}"), dependencies.get(&name));
                    }
                }
            }
            if own.get("lints").is_some_and(inherits) {
                part("workspace.lints".into(), ws.get("lints"));
            }
        }
        Ok(out)
    }

    /// What rustc may read from the packages `used`, `left` left out: as Rust (`rust`), every `.rs`
    /// file in their folders but in `left`, every file a target starts from, and, in turn, every
    /// file or folder one of them names as Rust (`#[path]`, `include!`; a folder read whole) and
    /// every file a `mod x;` of one may be (a module's file beside it or in its own folder, `x.rs` or
    /// `x/mod.rs`, inside its inline modules' folders: both ways rustc reads a file, as a `mod.rs`
    /// or not); and as data (`data`), every file one of them names by `include_str!`,
    /// `include_bytes!` or a `debugger_visualizer`. A name climbing out of the tree is refused.
    fn read_as_rust(&self, used: &[&Member], left: &[String]) -> Result<Reading, String> {
        let mut files: Vec<String> = Vec::new();
        for m in used {
            walk(self.tree, &m.dir, left, &mut files).map_err(|e| format!("{}'s sources do not read: {e}", m.dir))?;
            for s in &m.sources {
                files.push(normal(Path::new(s)).ok_or_else(|| format!("{} builds from `{s}`, outside the commit's tree", m.name))?);
            }
        }
        let mut reading = Reading::default();
        let mut i = 0;
        while i < files.len() {
            let file = files[i].clone();
            i += 1;
            if !reading.rust.insert(file.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(self.tree.join(&file)) else { continue };
            let from = Path::new(&file).parent().unwrap_or(Path::new(""));
            for (lit, rust) in named(&text) {
                let path = normal(&from.join(&lit)).ok_or_else(|| format!("{file} names `{lit}`, outside the commit's tree"))?;
                if !rust {
                    reading.data.insert(path);
                } else if self.tree.join(&path).is_dir() {
                    // A folder a `#[path]` gives an inline module: the modules rustc loads there
                    // are read in turn, every `.rs` file of it.
                    let mut inner = Vec::new();
                    walk(self.tree, &path, left, &mut inner).map_err(|e| format!("{path}'s sources do not read: {e}"))?;
                    files.extend(inner);
                    reading.rust.insert(path);
                } else {
                    files.push(path);
                }
            }
            let stem = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            // A file that does not lex: every `.rs` file of its folder, below it too, read in
            // turn, as a `mod x;` of it can name no file elsewhere (beside it, in its own folder,
            // inside its inline modules' folders); its `#[path]` names are read from its text.
            let Some(modules) = declared(&text) else {
                let mut inner = Vec::new();
                let folder = from.display().to_string();
                walk(self.tree, &folder, left, &mut inner).map_err(|e| format!("{folder}'s sources do not read: {e}"))?;
                files.extend(inner);
                continue;
            };
            for module in modules {
                for base in [from.to_path_buf(), from.join(&stem)] {
                    for candidate in [base.join(format!("{module}.rs")), base.join(&module).join("mod.rs")] {
                        if let Some(path) = normal(&candidate).filter(|p| self.tree.join(p).is_file()) {
                            files.push(path);
                        }
                    }
                }
            }
        }
        Ok(reading)
    }

}

/// A lock file's dependency string (`name`, `name version` or `name version (source)`) resolved to
/// the one package it names.
pub(crate) fn resolve(lock: &[Locked], d: &str) -> Result<usize, String> {
    let mut parts = d.splitn(3, ' ');
    let name = parts.next().unwrap_or_default();
    let version = parts.next();
    let source = parts.next().map(|s| s.trim_start_matches('(').trim_end_matches(')'));
    let found: Vec<usize> = lock
        .iter()
        .enumerate()
        // Cargo writes a git source in a dependency string without its `#<commit>`.
        .filter(|(_, l)| {
            l.name == name
                && version.is_none_or(|v| l.version == v)
                && source.is_none_or(|s| l.source.as_deref().is_some_and(|ls| ls == s || ls.split_once('#').is_some_and(|(base, _)| base == s)))
        })
        .map(|(i, _)| i)
        .collect();
    // One name and version locked twice: cargo writes the source of the one that has one, so a
    // line without a source names the package by path.
    let pathless: Vec<usize> = found.iter().copied().filter(|&i| lock[i].source.is_none()).collect();
    match (&found[..], &pathless[..]) {
        ([one], _) => Ok(*one),
        ([], _) => Err(format!("its Cargo.lock names `{d}` and pins no such package")),
        (_, [one]) if source.is_none() => Ok(*one),
        _ => Err(format!("its Cargo.lock names `{d}`, which more than one of its packages answers")),
    }
}

/// The folder's path with a final `/`, or nothing for the root.
fn prefix(dir: &str) -> String {
    if dir.is_empty() { String::new() } else { format!("{dir}/") }
}

/// What rustc may read from the packages a build uses ([`Workspace::read_as_rust`]).
#[derive(Default)]
struct Reading {
    /// The files and folders read as Rust.
    rust: BTreeSet<String>,
    /// The files read as data (`include_str!`, `include_bytes!`, a debugger's script).
    data: BTreeSet<String>,
}

/// The top folders of a package (`crates/x/tests/`) that may hold its tests, benches and examples
/// alone, before what rustc may read decides ([`Workspace::read_as_rust`]): each that the commit
/// holds, that no target that ships (a library, an executable, a build script) starts in and that
/// holds no package a build uses (`dirs`), in a package without a build script.
fn left_out(tree: &Path, m: &Member, dirs: &BTreeSet<&str>) -> Vec<String> {
    // A build script may read anything in its package's folder (IronCalc's xlsx lists its
    // `tests/` to write code), so none of it is what only the tests read.
    if m.build_script {
        return Vec::new();
    }
    let top = prefix(&m.dir);
    TEST_FOLDERS
        .iter()
        .map(|t| format!("{top}{t}/"))
        .filter(|f| tree.join(f).is_dir())
        .filter(|f| !m.sources.iter().any(|s| normal(Path::new(s)).is_some_and(|p| p.starts_with(f.as_str()))))
        .filter(|f| !dirs.iter().any(|d| format!("{d}/").starts_with(f.as_str())))
        .collect()
}

/// The modules a Rust source declares by `mod x;`, each as its path below the file's own module
/// folder (`x`, or `a/x` inside an inline `mod a { … }`), read from the tokens as rustc lexes them;
/// `None` for a text proc-macro2 does not lex (rustc's editions 2015 and 2018 take `'a#b`, which it
/// does not).
pub fn declared(text: &str) -> Option<Vec<String>> {
    fn inner(ts: proc_macro2::TokenStream, prefix: &str, out: &mut Vec<String>) {
        use proc_macro2::{Delimiter, TokenTree};
        let tts: Vec<TokenTree> = ts.into_iter().collect();
        let mut i = 0;
        while i < tts.len() {
            match &tts[i] {
                TokenTree::Ident(id) if id == "mod" => {
                    if let Some(TokenTree::Ident(name)) = tts.get(i + 1) {
                        let name = name.to_string().trim_start_matches("r#").to_string();
                        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                        match tts.get(i + 2) {
                            Some(TokenTree::Punct(p)) if p.as_char() == ';' => out.push(path),
                            // An inline module: its body's modules lie in its own folder.
                            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                                inner(g.stream(), &path, out);
                                i += 3;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => inner(g.stream(), prefix, out),
                _ => {}
            }
            i += 1;
        }
    }
    let mut lf = text.replace("\r\n", "\n");
    if let Some(rest) = lf.strip_prefix('\u{feff}') {
        lf = rest.to_string();
    }
    if is_shebang(&lf) {
        lf = lf.find('\n').map(|i| lf[i..].to_string()).unwrap_or_default();
    }
    let mut out = Vec::new();
    inner(lexed(&lf)?, "", &mut out);
    Some(out)
}

/// A text's tokens as rustc lexes it in every edition, or `None`: proc-macro2 reads a raw C string
/// (`cr"…"`, `cr#"…"#`) in every edition, where rustc's 2015 and 2018 read `cr`, then a `#` and a
/// plain string that may end elsewhere, so a text holding one is read as one that does not lex.
fn lexed(text: &str) -> Option<proc_macro2::TokenStream> {
    fn raw_c(ts: proc_macro2::TokenStream) -> bool {
        ts.into_iter().any(|t| match t {
            proc_macro2::TokenTree::Literal(l) => {
                let s = l.to_string();
                s.starts_with("cr\"") || s.starts_with("cr#")
            }
            proc_macro2::TokenTree::Group(g) => raw_c(g.stream()),
            _ => false,
        })
    }
    let ts = text.parse::<proc_macro2::TokenStream>().ok()?;
    (!raw_c(ts.clone())).then_some(ts)
}

/// Whether a dependency's platform (`target` in cargo's metadata: a `cfg(…)` expression, or a
/// target's own name) holds for the target `target`, whose configuration `cfg` is as `rustc --print
/// cfg --target <target>` says it (`unix`, `target_os="linux"`), as cargo reads it: `all`, `any`,
/// `not`, a name, a name and its value, `true` and `false`.
pub fn platform_holds(spec: &str, target: &str, cfg: &[(String, Option<String>)]) -> bool {
    fn expr(s: &mut &str, cfg: &[(String, Option<String>)]) -> Option<bool> {
        *s = s.trim_start();
        let end = s.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(s.len());
        let name = s[..end].to_string();
        *s = s[end..].trim_start();
        if name.is_empty() {
            return None;
        }
        if let Some(rest) = s.strip_prefix('(') {
            *s = rest;
            let mut values = Vec::new();
            loop {
                *s = s.trim_start();
                if let Some(rest) = s.strip_prefix(')') {
                    *s = rest;
                    break;
                }
                values.push(expr(s, cfg)?);
                *s = s.trim_start();
                if let Some(rest) = s.strip_prefix(',') {
                    *s = rest;
                }
            }
            return match name.as_str() {
                "all" => Some(values.iter().all(|v| *v)),
                "any" => Some(values.iter().any(|v| *v)),
                "not" if values.len() == 1 => Some(!values[0]),
                _ => None,
            };
        }
        if let Some(rest) = s.strip_prefix('=') {
            let rest = rest.trim_start().strip_prefix('"')?;
            let close = rest.find('"')?;
            let value = rest[..close].to_string();
            *s = &rest[close + 1..];
            return Some(cfg.iter().any(|(n, v)| *n == name && v.as_deref() == Some(value.as_str())));
        }
        Some(match name.as_str() {
            "true" => true,
            "false" => false,
            _ => cfg.iter().any(|(n, v)| *n == name && v.is_none()),
        })
    }
    match spec.trim().strip_prefix("cfg(").and_then(|r| r.strip_suffix(')')) {
        Some(inner) => {
            let mut s = inner;
            let held = expr(&mut s, cfg);
            // An expression it cannot read holds, as a dependency it may build: the bin is
            // required, never dropped by a misreading.
            held.filter(|_| s.trim().is_empty()).unwrap_or(true)
        }
        None => spec.trim() == target,
    }
}

/// The feature resolver cargo builds the workspace with: the workspace's `resolver`, else the root
/// package's own, else its edition's (2015 and 2018: 1; 2021: 2; later: 3), else 1 for a virtual
/// workspace, as cargo 1.100 reads it.
fn effective_resolver(root: &toml::Table, ws: &toml::Table, ws_package: &toml::Table) -> String {
    let text = |v: &toml::Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    if let Some(r) = ws.get("resolver") {
        return text(r);
    }
    let Some(package) = root.get("package").or_else(|| root.get("project")).and_then(|p| p.as_table()) else {
        return "1".to_string();
    };
    if let Some(r) = package.get("resolver") {
        return text(r);
    }
    let edition = match package.get("edition") {
        Some(v) if v.as_table().and_then(|t| t.get("workspace")).and_then(|w| w.as_bool()) == Some(true) => ws_package.get("edition").map(text),
        Some(v) => Some(text(v)),
        None => None,
    };
    match edition.as_deref() {
        None | Some("2015") | Some("2018") => "1",
        Some("2021") => "2",
        Some(_) => "3",
    }
    .to_string()
}

/// A TOML value's identity: its canonical JSON, tables in key order, its text's layout and comments
/// apart.
fn value_id(v: &toml::Value) -> String {
    serde_json::to_value(v).map(|j| j.to_string()).unwrap_or_else(|e| format!("unreadable: {e}"))
}

/// Every `.rs` file under `dir` of the tree (paths from the root, in name order) but under the
/// folders `left` leaves out (its tests', benches' and examples' own).
fn walk(tree: &Path, dir: &str, left: &[String], out: &mut Vec<String>) -> std::io::Result<()> {
    fn inner(tree: &Path, rel: &str, left: &[String], out: &mut Vec<String>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(tree.join(rel))?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = format!("{}{name}", prefix(rel));
            let kind = e.file_type()?;
            if kind.is_dir() {
                if !left.contains(&format!("{path}/")) {
                    inner(tree, &path, left, out)?;
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    inner(tree, dir, left, out)
}

/// The string literals a Rust source names files by: `#[path = "…"]`, `path = "…"` inside a
/// `#[cfg_attr(…)]`, and the argument of `include_str!`, `include_bytes!` and `include!` when it is
/// one literal (a raw one included). Read as text, comments too: a path a comment names is one more
/// input, never one less.
pub fn literals(text: &str) -> Vec<String> {
    named(text).into_iter().map(|(p, _)| p).collect()
}

/// The files a Rust source names, in its order, each with whether rustc reads it as Rust (a
/// `#[path]` module or an `include!`; `include_str!` and `include_bytes!` read data). Read from the
/// tokens as rustc lexes them (proc-macro2's lexer, the line endings made LF first, as rustc does):
/// a char literal `'"'`, a raw string, nested comments and tokens apart by comments are rustc's.
/// A text that does not lex as Rust is read for the same forms as text instead.
pub fn named(text: &str) -> Vec<(String, bool)> {
    let mut lf = text.replace("\r\n", "\n");
    // rustc drops a byte-order mark, then a first line that is a shebang, as here.
    if let Some(rest) = lf.strip_prefix('\u{feff}') {
        lf = rest.to_string();
    }
    if is_shebang(&lf) {
        lf = lf.find('\n').map(|i| lf[i..].to_string()).unwrap_or_default();
    }
    match lexed(&lf).ok_or(()) {
        Ok(ts) => {
            let mut all = Vec::new();
            tokens(ts, &mut all);
            // An attribute's group is read as an attribute and as a group: each name once.
            let mut out: Vec<(String, bool)> = Vec::new();
            for n in all {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
            out
        }
        Err(_) => {
            let mut out: Vec<(String, bool)> = literals_in(&lf);
            for l in literals_in(&uncommented(&lf)) {
                if !out.iter().any(|(p, _)| *p == l.0) {
                    out.push(l);
                }
            }
            out
        }
    }
}

/// Whether a source's first line is a shebang as rustc reads it (`rustc_lexer::strip_shebang`): it
/// opens with `#!` and the next token, past whitespace and comments that are not doc comments
/// (across lines), is not `[`, which would open an inner attribute.
pub fn is_shebang(text: &str) -> bool {
    let Some(tail) = text.strip_prefix("#!") else { return false };
    let mut rest = tail;
    loop {
        // Rust's whitespace (`Pattern_White_Space`).
        rest = rest.trim_start_matches(['\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}']);
        if let Some(after) = rest.strip_prefix("//") {
            // `///` (not `////`) and `//!` are doc comments, tokens of their own.
            if (after.starts_with('/') && !after.starts_with("//")) || after.starts_with('!') {
                return true;
            }
            rest = after.find('\n').map(|i| &after[i..]).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("/*") {
            // `/**` (not `/**/` or `/***`) and `/*!` are doc comments.
            if (after.starts_with('*') && !after.starts_with("*/") && !after.starts_with("**")) || after.starts_with('!') {
                return true;
            }
            // Nested, to its end (an unterminated one runs to the end of the text).
            let mut depth = 1;
            let mut i = 0;
            let b = after.as_bytes();
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            rest = &after[i..];
        } else {
            return !rest.starts_with('[');
        }
    }
}

/// The string a literal token holds, if it is a string literal (`"…"` or a raw one).
fn string_of(lit: &proc_macro2::Literal) -> Option<String> {
    literal(&lit.to_string())
}

fn tokens(ts: proc_macro2::TokenStream, out: &mut Vec<(String, bool)>) {
    use proc_macro2::{Delimiter, TokenTree};
    let tts: Vec<TokenTree> = ts.into_iter().collect();
    for (i, tt) in tts.iter().enumerate() {
        match tt {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let at = if matches!(tts.get(i + 1), Some(TokenTree::Punct(q)) if q.as_char() == '!') { i + 2 } else { i + 1 };
                if let Some(TokenTree::Group(g)) = tts.get(at)
                    && g.delimiter() == Delimiter::Bracket
                {
                    attribute(g.stream(), out);
                }
            }
            // A raw identifier (`r#include_str`) is the same macro.
            TokenTree::Ident(id) if ["include_str", "include_bytes", "include"].contains(&id.to_string().trim_start_matches("r#")) => {
                if let (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(g))) = (tts.get(i + 1), tts.get(i + 2))
                    && bang.as_char() == '!'
                    && let Some(TokenTree::Literal(lit)) = g.stream().into_iter().next()
                    && let Some(s) = string_of(&lit)
                {
                    out.push((s, id.to_string().trim_start_matches("r#") == "include"));
                }
            }
            TokenTree::Group(g) => tokens(g.stream(), out),
            _ => {}
        }
    }
}

/// An attribute's tokens: `path = "…"`, a `cfg_attr(<predicate>, …)` whose attributes are read in
/// turn, and whatever else it holds read for the macros that name files (`doc = include_str!(…)`).
fn attribute(ts: proc_macro2::TokenStream, out: &mut Vec<(String, bool)>) {
    use proc_macro2::{Delimiter, TokenTree};
    let tts: Vec<TokenTree> = ts.clone().into_iter().collect();
    match tts.as_slice() {
        [TokenTree::Ident(id), TokenTree::Punct(eq), TokenTree::Literal(lit), ..] if id.to_string().trim_start_matches("r#") == "path" && eq.as_char() == '=' => {
            if let Some(s) = string_of(lit) {
                out.push((s, true));
            }
        }
        // `debugger_visualizer(gdb_script_file = "…")` or `natvis_file`: a file rustc reads and
        // embeds, from the attribute's own file's folder, as data.
        [TokenTree::Ident(id), TokenTree::Group(g), ..] if id.to_string().trim_start_matches("r#") == "debugger_visualizer" && g.delimiter() == Delimiter::Parenthesis => {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            for w in inner.windows(3) {
                if let [TokenTree::Ident(k), TokenTree::Punct(eq), TokenTree::Literal(lit)] = w
                    && ["gdb_script_file", "natvis_file"].contains(&k.to_string().trim_start_matches("r#"))
                    && eq.as_char() == '='
                    && let Some(s) = string_of(lit)
                {
                    out.push((s, false));
                }
            }
        }
        [TokenTree::Ident(id), TokenTree::Group(g), ..] if id.to_string().trim_start_matches("r#") == "cfg_attr" && g.delimiter() == Delimiter::Parenthesis => {
            // The predicate, then attributes apart by commas at the top.
            let mut parts: Vec<Vec<TokenTree>> = vec![vec![]];
            for t in g.stream() {
                match &t {
                    TokenTree::Punct(c) if c.as_char() == ',' => parts.push(vec![]),
                    _ => parts.last_mut().expect("one part at least").push(t),
                }
            }
            for part in parts.into_iter().skip(1) {
                attribute(part.into_iter().collect(), out);
            }
        }
        _ => tokens(ts, out),
    }
}

/// `text` with every comment (`//…`, `/*…*/`, nested) replaced by spaces, strings kept whole.
fn uncommented(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '"' => {
                out.push('"');
                i += 1;
                while i < b.len() && b[i] != '"' {
                    if b[i] == '\\' && i + 1 < b.len() {
                        out.push(b[i]);
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push('"');
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                let mut depth = 0;
                while i < b.len() {
                    if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        out.push(if b[i] == '\n' { '\n' } else { ' ' });
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn literals_in(text: &str) -> Vec<(String, bool)> {
    let mut out: Vec<String> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    // `#[cfg_attr(<predicate>, path = "…")]`: each `path =` the attribute's parentheses hold.
    let mut rest = text;
    while let Some(at) = rest.find("cfg_attr") {
        rest = &rest[at + "cfg_attr".len()..];
        let Some(open) = rest.trim_start().strip_prefix('(') else { continue };
        let mut depth = 1;
        let mut end = open.len();
        let mut in_string = false;
        let mut chars = open.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' if in_string => {
                    chars.next();
                }
                '"' => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let inner = &open[..end];
        let mut scan = inner;
        while let Some(p) = scan.find("path") {
            let before = &inner[..inner.len() - scan.len() + p];
            // A raw identifier (`r#path`) is the same key.
            let before = before.strip_suffix("r#").unwrap_or(before).trim_end();
            scan = &scan[p + "path".len()..];
            if !(before.ends_with(',') || before.ends_with('(') || before.is_empty()) {
                continue;
            }
            if let Some(lit) = scan.trim_start().strip_prefix('=').and_then(|s| literal(s.trim_start())) {
                out.push(lit);
            }
        }
    }
    // `#[path = "…"]`, spaces between its tokens allowed.
    let mut rest = text;
    while let Some(at) = rest.find('#') {
        rest = &rest[at + 1..];
        if let Some(lit) = rest
            .trim_start()
            .strip_prefix('[')
            .map(|s| s.trim_start())
            .and_then(|s| s.strip_prefix("r#").unwrap_or(s).strip_prefix("path"))
            .and_then(|s| s.trim_start().strip_prefix('='))
            .and_then(|s| literal(s.trim_start()))
        {
            out.push(lit);
        }
    }
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    for (token, sep) in [("include_str", '('), ("include_bytes", '('), ("include", '(')] {
        let mut rest = text;
        while let Some(at) = rest.find(token) {
            let before = text[..text.len() - rest.len() + at].chars().next_back();
            rest = &rest[at + token.len()..];
            if before.is_some_and(ident) || rest.starts_with(ident) {
                continue;
            }
            let Some(after) = rest.trim_start().strip_prefix('!') else { continue };
            let after = after.trim_start();
            let Some(after) = after.strip_prefix(sep).or_else(|| if sep == '(' { after.strip_prefix(['[', '{']) } else { None }) else { continue };
            if let Some(lit) = literal(after.trim_start()) {
                out.push(lit);
            }
        }
    }
    // `debugger_visualizer(gdb_script_file = "…")` or `natvis_file`: data rustc embeds.
    for key in ["gdb_script_file", "natvis_file"] {
        let mut rest = text;
        while let Some(at) = rest.find(key) {
            rest = &rest[at + key.len()..];
            if let Some(lit) = rest.trim_start().strip_prefix('=').and_then(|s| literal(s.trim_start())) {
                data.push(lit);
            }
        }
    }
    out.into_iter().map(|p| (p, true)).chain(data.into_iter().map(|p| (p, false))).collect()
}

/// The string literal `s` starts with, unescaped as Rust reads it (`\\`, `\"`, `\n`, `\t`, `\r`,
/// `\0`, `\'`, `\xNN`, `\u{…}`, a `\` before a line break dropping it and the next line's
/// leading spaces), or a raw one (`r"…"`, `r#"…"#`).
fn literal(s: &str) -> Option<String> {
    if let Some(raw) = s.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = raw[hashes..].strip_prefix('"')?;
        let end = body.find(&format!("\"{}", "#".repeat(hashes)))?;
        return Some(body[..end].to_string());
    }
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'x' => {
                    let hex: String = [chars.next()?, chars.next()?].iter().collect();
                    out.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
                }
                'u' => {
                    if chars.next()? != '{' {
                        return None;
                    }
                    // Underscores between the digits, as rustc reads them (`\u{2E_}`).
                    let hex: String = chars.by_ref().take_while(|c| *c != '}').filter(|c| *c != '_').collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                // A line continued: rustc skips the ASCII whitespace that follows, no other.
                '\n' => {
                    while chars.peek().is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// An absolute path with its `.` and `..` parts resolved as the kernel resolves them (`/..` is `/`),
/// without its leading `/`.
fn rooted(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// A path from the tree's root with its `.` and `..` parts resolved; `None` when it is absolute or
/// climbs out of the tree.
fn normal(p: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for part in p.components() {
        match part {
            Part::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Part::CurDir => {}
            Part::ParentDir => {
                parts.pop()?;
            }
            Part::RootDir | Part::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

/// What a dep-info file (cargo's `target/release/<bin>.d`: every file of the workspace a bin's
/// compilation read, its build scripts' declared ones too) names that `inputs` does not: each file
/// under `src` (the tree's root in the builder) outside them, and each file elsewhere but the
/// build's own output (`src/target/`) and the vendored sources the lock file pins (`vendor`).
/// Whether a folder of the build's output, `target/release/build/<first>/<second>`, is one a package
/// of `packages` made: `<name>/<16 hexadecimal>` as cargo 1.100 writes it, `<name>-<16
/// hexadecimal>` as cargo wrote it before. The package named whole (`foo` never owns `foo-bar`).
pub(crate) fn built_by(packages: &Packages, first: &str, second: Option<&str>) -> bool {
    let hash = |h: &str| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit());
    (packages.names.contains(first) && second.is_some_and(hash)) || packages.names.iter().any(|p| first.strip_prefix(p.as_str()).and_then(|r| r.strip_prefix('-')).is_some_and(hash))
}

/// An absolute path with its `.` and `..` parts resolved as the kernel resolves them, from `/`.
pub(crate) fn landed(path: &str) -> PathBuf {
    PathBuf::from(format!("/{}", rooted(path)))
}

pub fn read_outside(dep_info: &str, src: &str, vendor: &str, inputs: &Inputs, packages: &Packages) -> Vec<String> {
    // The builds share one output folder and one copy of the vendored sources: a build may read
    // there only what its own packages made or brought (a build script's `OUT_DIR` and a locked
    // crate's vendored folder), never another's; a vendored crate at the version the build
    // compiles.
    let built_by = |first: &str, second: Option<&str>| built_by(packages, first, second);
    let vendored = |dir: &str| packages.vendored.contains(dir);
    let mut out = BTreeSet::new();
    for path in dep_info_paths(dep_info) {
        // A path as rustc or cargo writes it may climb back (`crates/x/src/../../../docs/y.rs`, a
        // build script's `rerun-if-changed={OUT_DIR}/../../…`): resolved first, then judged where
        // it lands, the build's own output and the vendored sources included.
        // Cargo escapes a space as `\ ` and nothing else, so a name ending in `\` before the space
        // between two paths reads as one path holding ` /`: no file's name holds `/`, so such a
        // word is ambiguous and refused as it stands; so is a path that is not absolute.
        if path.contains(" /") || !path.starts_with('/') {
            out.insert(path);
            continue;
        }
        let landed = landed(&path);
        let built = Path::new(src).join("target/release/build");
        if let Ok(rest) = landed.strip_prefix(&built) {
            let mut parts = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
            let (first, second) = (parts.next(), parts.next());
            if first.is_some_and(|f| built_by(&f, second.as_deref())) {
                continue;
            }
        }
        if let Ok(rest) = landed.strip_prefix(vendor)
            && rest.components().next().is_some_and(|c| vendored(&c.as_os_str().to_string_lossy()))
        {
            continue;
        }
        if landed.starts_with(vendor) {
            out.insert(path);
            continue;
        }
        match landed.strip_prefix(src) {
            Ok(rel) => {
                let rel = rel.to_string_lossy().into_owned();
                if !inputs.covers(&rel) {
                    out.insert(rel);
                }
            }
            Err(_) => {
                out.insert(path);
            }
        }
    }
    out.into_iter().collect()
}

/// The prerequisites of every rule of a dep-info file as cargo writes it (`target: dep dep`, a
/// space in a name escaped as `\ `, a line continued by a final `\`), its targets left out.
fn dep_info_paths(text: &str) -> Vec<String> {
    let joined = text.replace("\\\n", " ");
    let mut out = Vec::new();
    for line in joined.lines() {
        // The rule's colon is the first one followed by a space or the line's end.
        let Some(colon) = line.char_indices().find(|&(i, c)| c == ':' && line[i + 1..].chars().next().is_none_or(char::is_whitespace)).map(|(i, _)| i) else {
            continue;
        };
        // Cargo's writer escapes a space as `\ ` and writes everything else, `$` included, as it is.
        let mut word = String::new();
        let mut chars = line[colon + 1..].chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&' ') => word.push(chars.next().unwrap_or(' ')),
                // Cargo writes the paths apart by a space, and nothing else.
                ' ' => {
                    if !word.is_empty() {
                        out.push(std::mem::take(&mut word));
                    }
                }
                c => word.push(c),
            }
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}
