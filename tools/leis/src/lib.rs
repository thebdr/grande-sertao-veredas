//! The suite's laws, machine-enforced.
//!
//! `docs/veredas-plan.md` states the rules of the Veredas 2 workspace; this crate checks the ones a
//! machine can check from the shape of the tree, and `tests/` is the verdict (`cargo test -p leis`,
//! one file per law plus `real_tree.rs`). Each law is a pure function over a [`Suite`] built from
//! the manifests, so the tests attack the laws on synthetic suites and then run them on the real
//! tree ([`Suite::read`]). A law changes only with the contract clause that states it
//! (`.zen/contract.md`, C-001..C-006).
//!
//! THE LAWS (a component is a member under `crates/`; a member under `tools/` is repo tooling)
//!   L1 ROOTS       every `Cargo.toml` under `crates/` or `tools/`, at any depth, is a workspace
//!                  member listed by path or is excluded: a glob is not a member, an unlisted crate is
//!                  refused, a nested crate is refused, and every member lives directly under one of
//!                  those two roots; a `veredas-*` or `orcl-*` crate (by name or folder, `_` as `-`)
//!                  anywhere the repository holds or could hold it (git's listing, not a walk) is a
//!                  member under a root or nothing, never excluded.
//!   L2 NAMING      a component is `veredas-<nome>`, `<nome>` a lowercase ASCII letter then letters and
//!                  digits, in a folder of the same name; the language's crates are `orcl-<nome>`; a
//!                  `veredas-*` or `orcl-*` crate lives under `crates/`, never `tools/`.
//!   L3 VERSIONING  no `[workspace.package] version`; every component states its own SemVer, from 2.0.0.
//!   L4 CORE        `veredas-core` depends on no other member, in any dependency table, a dependency
//!                  inherited from `[workspace.dependencies]` or renamed with `package` included.
//!   L5 DIRECTION   every member→member edge (normal and build dependencies) is in [`MAP`]: anything
//!                  may use core, the apps use portal, the bench and the scanner never pull portal,
//!                  controlador embeds portal and aranha and runs ORCL. A dev-dependency is free (a
//!                  bench is a fixture); a tool may depend on anything.
//!   L6 PLAN        a component has its `docs/<nome>-plan.md`, not blank, and so does every commit
//!                  reachable from HEAD that holds a component folder under `crates/`, present at
//!                  HEAD or not ([`history`], over git's trees); the language's crates share
//!                  `docs/orcl/README.md`. The suite's own plan is nobody's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

/// The folders a member may live under.
pub const ROOTS: [&str; 2] = ["crates", "tools"];
/// The foundation: the one member every other may use, and the one that uses none.
pub const CORE: &str = "veredas-core";
/// Every component's prefix.
pub const PREFIX: &str = "veredas-";
/// The language's crates keep their names (the plan, "Naming").
pub const LANGUAGE: &str = "orcl-";
/// Where the language's crates document themselves.
pub const LANGUAGE_PLAN: &str = "docs/orcl/README.md";
/// The suite's own plan: the authority on intent, and no component's plan.
pub const SUITE_PLAN: &str = "docs/veredas-plan.md";
/// Every component starts at 2.0.0 (the plan, "Versioning").
pub const FIRST_MAJOR: u64 = 2;

/// L5's map: for each component (or family, `orcl-*`), the members it may depend on besides
/// core. A component absent from the map may depend on core alone; adding it here is a contract
/// change, never a fix.
pub const MAP: &[(&str, &[&str])] = &[
    ("veredas-core", &[]),
    ("veredas-portal", &[]),
    ("veredas-maquinar", &["veredas-aranha"]),
    ("veredas-aranha", &[]),
    ("veredas-controlador", &["veredas-portal", "veredas-aranha", "orcl-*"]),
    ("veredas-comissionador", &["veredas-portal"]),
    ("veredas-supervisao", &["veredas-portal"]),
    ("veredas-abaco", &["veredas-portal"]),
    ("veredas-wms", &["veredas-portal"]),
    ("orcl-*", &["orcl-*"]),
];

/// Which dependency table an edge was declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepTable {
    Normal,
    Dev,
    Build,
}

impl fmt::Display for DepTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DepTable::Normal => "dependencies",
            DepTable::Dev => "dev-dependencies",
            DepTable::Build => "build-dependencies",
        })
    }
}

/// One declared dependency: the package it names and the table it sits in. An entry with
/// `workspace = true` is named by the `[workspace.dependencies]` entry it inherits (its `package`
/// rename, else the key; cargo ignores a `package` written beside `workspace = true`); any other
/// by its own `package` rename, else the key. Target-specific tables count like the plain ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    pub name: String,
    pub table: DepTable,
}

/// One workspace member as its manifest declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The folder relative to the root, forward slashes: `crates/veredas-core`.
    pub dir: String,
    pub name: String,
    /// The version as written; `None` when absent or inherited.
    pub version: Option<String>,
    /// `version.workspace = true`.
    pub version_from_workspace: bool,
    /// Every dependency, in table order (normal, dev, build, then each target's three), each
    /// table sorted by key.
    pub deps: Vec<Dep>,
}

impl Member {
    /// A member under `crates/` is a component; under `tools/` it is repo tooling.
    pub fn is_component(&self) -> bool {
        self.dir.starts_with("crates/")
    }
}

/// The workspace as the laws see it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suite {
    /// `[workspace] members`, normalized.
    pub listed: Vec<String>,
    /// `[workspace] exclude`, normalized; an entry covers everything beneath it, as cargo reads it.
    pub excluded: Vec<String>,
    /// `[workspace.package]` carries a `version`.
    pub workspace_version: bool,
    /// Every folder whose `Cargo.toml` the repository holds or could hold (the root's own left
    /// out), wherever it sits: under a root or anywhere else.
    pub found: Vec<String>,
    /// The listed members whose manifest was found, in listed order.
    pub members: Vec<Member>,
    /// Every crate found under a root that is not a member (excluded, or stray), parsed like a
    /// member so a law can read its name, in found order.
    pub others: Vec<Member>,
    /// `docs/*-plan.md` (the suite's own plan left out) and `docs/orcl/README.md`, path to content.
    pub plans: BTreeMap<String, String>,
}

/// A law broken: `L2 crates/foo: ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub law: &'static str,
    pub member: String,
    pub what: String,
}

impl Violation {
    fn new(law: &'static str, member: impl Into<String>, what: impl Into<String>) -> Self {
        Violation { law, member: member.into(), what: what.into() }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.law, self.member, self.what)
    }
}

/// What stops the laws from reading a tree at all. A manifest cargo would refuse is an error, not
/// a violation.
#[derive(Debug)]
pub enum Error {
    Io { path: String, source: io::Error },
    Manifest { path: String, source: toml::de::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "cannot read {path}: {source}"),
            Error::Manifest { path, source } => write!(f, "{path} does not parse: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Manifest { source, .. } => Some(source),
        }
    }
}

#[derive(Deserialize, Default)]
struct WorkspaceManifest {
    #[serde(default)]
    workspace: WorkspaceTable,
}

#[derive(Deserialize, Default)]
struct WorkspaceTable {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    package: toml::Table,
    #[serde(default)]
    dependencies: toml::Table,
}

/// The three dependency tables, under the manifest root or under one `[target.<cfg>]`. The
/// underscore spellings are the ones cargo still accepts before edition 2024.
#[derive(Deserialize, Default)]
struct DepTables {
    #[serde(default)]
    dependencies: toml::Table,
    #[serde(default, rename = "dev-dependencies", alias = "dev_dependencies")]
    dev_dependencies: toml::Table,
    #[serde(default, rename = "build-dependencies", alias = "build_dependencies")]
    build_dependencies: toml::Table,
}

#[derive(Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    package: PackageTable,
    #[serde(flatten)]
    deps: DepTables,
    #[serde(default)]
    target: BTreeMap<String, DepTables>,
}

#[derive(Deserialize, Default)]
struct PackageTable {
    #[serde(default)]
    name: String,
    version: Option<toml::Value>,
}

fn normalize(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    p.trim_end_matches('/').to_string()
}

fn package_of(entry: &toml::Value) -> Option<&str> {
    entry.as_table().and_then(|t| t.get("package")).and_then(|v| v.as_str())
}

fn inherits(entry: &toml::Value) -> bool {
    entry.as_table().and_then(|t| t.get("workspace")).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn deps_of(table: &toml::Table, kind: DepTable, workspace: &toml::Table) -> Vec<Dep> {
    let mut keys: Vec<&String> = table.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|key| {
            let entry = &table[key];
            // beside `workspace = true` cargo ignores the member's own `package`: the root entry
            // names the dependency (its `package`, else the key)
            let name = if inherits(entry) {
                workspace.get(key).and_then(package_of).unwrap_or(key)
            } else {
                package_of(entry).unwrap_or(key)
            };
            Dep { name: name.to_string(), table: kind }
        })
        .collect()
}

fn all_deps(tables: &DepTables, workspace: &toml::Table) -> Vec<Dep> {
    let mut deps = deps_of(&tables.dependencies, DepTable::Normal, workspace);
    deps.extend(deps_of(&tables.dev_dependencies, DepTable::Dev, workspace));
    deps.extend(deps_of(&tables.build_dependencies, DepTable::Build, workspace));
    deps
}

impl Member {
    fn parse(dir: &str, text: &str, workspace: &toml::Table) -> Result<Member, Error> {
        let m: Manifest = toml::from_str(text)
            .map_err(|source| Error::Manifest { path: format!("{dir}/Cargo.toml"), source })?;
        let (version, version_from_workspace) = match &m.package.version {
            Some(toml::Value::String(s)) => (Some(s.clone()), false),
            Some(v @ toml::Value::Table(_)) => (None, inherits(v)),
            _ => (None, false),
        };
        let mut deps = all_deps(&m.deps, workspace);
        for tables in m.target.values() {
            deps.extend(all_deps(tables, workspace));
        }
        Ok(Member { dir: dir.to_string(), name: m.package.name, version, version_from_workspace, deps })
    }
}

impl Suite {
    /// The pure constructor: the workspace manifest's text, every `(folder, manifest text)` found
    /// under the roots, and the plan files present. Synthetic suites come in here.
    pub fn parse(
        workspace: &str,
        crates: &[(&str, &str)],
        plans: BTreeMap<String, String>,
    ) -> Result<Suite, Error> {
        let ws: WorkspaceManifest = toml::from_str(workspace)
            .map_err(|source| Error::Manifest { path: "Cargo.toml".to_string(), source })?;
        let found: Vec<String> = crates.iter().map(|(dir, _)| normalize(dir)).collect();
        let listed: Vec<String> = ws.workspace.members.iter().map(|m| normalize(m)).collect();
        let mut members = Vec::new();
        for dir in &listed {
            if let Some((_, text)) = crates.iter().find(|(d, _)| normalize(d) == *dir) {
                members.push(Member::parse(dir, text, &ws.workspace.dependencies)?);
            }
        }
        let mut others = Vec::new();
        for (dir, text) in crates {
            let dir = normalize(dir);
            if !listed.contains(&dir) {
                others.push(Member::parse(&dir, text, &ws.workspace.dependencies)?);
            }
        }
        Ok(Suite {
            listed,
            excluded: ws.workspace.exclude.iter().map(|m| normalize(m)).collect(),
            workspace_version: ws.workspace.package.contains_key("version"),
            found,
            members,
            others,
            plans,
        })
    }

    /// The real tree, as the repository holds it or could: `git ls-files` with the tracked files
    /// and the untracked ones git does not ignore, so a crate git would never take is nobody's
    /// business and one it would take is seen wherever it sits, a nested ignore rule or a forced
    /// add included. `Cargo.toml` (the root's own), every other `Cargo.toml` listed, every
    /// `docs/*-plan.md` listed but the suite's own, and `docs/orcl/README.md`. A tree that is no
    /// repository is an error: the laws govern a repository.
    pub fn read(root: &Path) -> Result<Suite, Error> {
        let read = |rel: &str| -> Result<String, Error> {
            fs::read_to_string(root.join(rel)).map_err(|source| Error::Io { path: rel.to_string(), source })
        };
        let not_a_repo = || Error::Io { path: root.display().to_string(), source: io::Error::other("not a git repository: the laws read what the repository holds") };
        let listing = git_bytes(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"], None)?.ok_or_else(not_a_repo)?;
        let paths: Vec<String> = listing
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .filter(|p| root.join(p).is_file()) // tracked but deleted from the tree: gone
            .collect();
        // a submodule is a folder the repository holds whose files it never lists: a crate by
        // its folder, with its manifest when it is checked out
        let staged = git_bytes(root, &["ls-files", "-z", "-s"], None)?.ok_or_else(not_a_repo)?;
        let gitlinks: Vec<String> = staged
            .split(|b| *b == 0)
            .filter_map(|rec| {
                let rec = String::from_utf8_lossy(rec);
                let (info, path) = rec.split_once('\t')?;
                info.starts_with("160000 ").then(|| path.to_string())
            })
            .collect();
        let workspace = read("Cargo.toml")?;
        let mut crates: Vec<(String, String)> = Vec::new();
        let mut plans = BTreeMap::new();
        for p in &paths {
            let (dir, file) = p.rsplit_once('/').unwrap_or(("", p.as_str()));
            // the manifest's name as a filesystem that ignores case reads it: cargo builds
            // `cargo.toml` on Windows
            if file.eq_ignore_ascii_case("Cargo.toml") {
                if !dir.is_empty() {
                    crates.push((dir.to_string(), read(p)?));
                }
            } else if p == LANGUAGE_PLAN
                || (p.starts_with("docs/") && p.ends_with("-plan.md") && !p["docs/".len()..].contains('/') && p != SUITE_PLAN)
            {
                plans.insert(p.clone(), read(p)?);
            }
        }
        for g in gitlinks {
            let manifest = format!("{g}/Cargo.toml");
            let text = if root.join(&manifest).is_file() { read(&manifest)? } else { String::new() };
            crates.push((g, text));
        }
        crates.sort();
        crates.dedup_by(|a, b| a.0 == b.0);
        let refs: Vec<(&str, &str)> = crates.iter().map(|(d, t)| (d.as_str(), t.as_str())).collect();
        Suite::parse(&workspace, &refs, plans)
    }

    /// The members under `crates/`.
    pub fn components(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| m.is_component())
    }

    fn names(&self) -> BTreeSet<&str> {
        self.members.iter().map(|m| m.name.as_str()).collect()
    }

    fn is_excluded(&self, dir: &str) -> bool {
        self.excluded.iter().any(|e| dir == e || dir.starts_with(&format!("{e}/")))
    }
}

fn under_a_root(dir: &str) -> bool {
    let mut parts = dir.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(root), Some(name), None) if ROOTS.contains(&root) && !name.is_empty()
    )
}

/// Somewhere beneath `crates/` or `tools/`, at any depth, or the root folder itself.
fn in_a_root(dir: &str) -> bool {
    ROOTS.iter().any(|r| dir == *r || dir.starts_with(&format!("{r}/")))
}

fn is_nome(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Nothing but whitespace, byte-order marks and zero-width characters: what an editor leaves in
/// a file it calls empty.
fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || matches!(c, '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}'))
}

/// `orcl-*` matches the family; anything else matches itself.
fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

/// What a component may depend on: core, then its row of [`MAP`].
fn allowed_for(name: &str) -> Vec<&'static str> {
    let mut allowed = vec![CORE];
    for (who, list) in MAP {
        if matches(who, name) {
            allowed.extend(list.iter().copied());
        }
    }
    allowed
}

/// A crate the suite's names mark as its own: `veredas-*` or `orcl-*`, by its name or its folder,
/// an underscore reading as the hyphen (cargo and crates.io treat the two spellings as one name).
fn is_suite_crate(m: &Member) -> bool {
    let folder = m.dir.rsplit('/').next().unwrap_or("");
    [m.name.as_str(), folder].iter().any(|n| {
        let n = n.replace('_', "-");
        n.starts_with(PREFIX) || n.starts_with(LANGUAGE)
    })
}

/// L1: every crate under a root is a listed member or excluded, every listed member is directly
/// under a root, and so is every crate found there; a suite crate (`veredas-*`, `orcl-*`) is a
/// member under a root or nothing, wherever in the repository it sits, excluded or not.
pub fn roots(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in &s.listed {
        if !under_a_root(m) {
            out.push(Violation::new("L1", m, "a member lives directly under crates/ or tools/"));
        } else if !s.found.contains(m) {
            out.push(Violation::new("L1", m, "listed, but no Cargo.toml there (a glob is not a member; list each one)"));
        }
    }
    for c in &s.found {
        let suite = s.others.iter().any(|o| o.dir == *c && is_suite_crate(o));
        if s.is_excluded(c) {
            if suite {
                out.push(Violation::new("L1", c, "a veredas-* or orcl-* crate is the suite's; it is a member, never excluded"));
            }
        } else if !under_a_root(c) {
            // nested under a root, or anywhere else in the repository
            if suite {
                out.push(Violation::new("L1", c, "a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member"));
            } else if in_a_root(c) {
                out.push(Violation::new("L1", c, "a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)"));
            }
        } else if !s.listed.contains(c) {
            out.push(Violation::new("L1", c, "has a Cargo.toml but is not a workspace member (list it, or exclude it)"));
        }
    }
    out
}

/// L2: a component is `veredas-<nome>` in a folder of its own name; the language's crates are
/// `orcl-<nome>`; a `veredas-*` crate is a component and lives under `crates/`.
pub fn naming(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in &s.members {
        if !m.is_component() {
            // the suite's prefixes by name or by folder, an underscore reading as the hyphen, as
            // L1 reads them: a suite crate does not become a tool by its spelling or its folder
            let folder = m.dir.rsplit('/').next().unwrap_or("");
            let folded = [m.name.as_str(), folder].map(|n| n.replace('_', "-"));
            if folded.iter().any(|n| n.starts_with(PREFIX)) {
                out.push(Violation::new("L2", &m.dir, "a veredas-* crate is a component; it lives under crates/"));
            } else if folded.iter().any(|n| n.starts_with(LANGUAGE)) {
                out.push(Violation::new("L2", &m.dir, "an orcl-* crate is the language's; it lives under crates/"));
            }
            continue;
        }
        match m.name.strip_prefix(LANGUAGE) {
            Some(rest) if !is_nome(rest) => out.push(Violation::new(
                "L2",
                &m.dir,
                format!("the language's crates are orcl-<nome>, <nome> lowercase ASCII (got `{}`)", m.name),
            )),
            Some(_) => {}
            None if !m.name.strip_prefix(PREFIX).is_some_and(is_nome) => out.push(Violation::new(
                "L2",
                &m.dir,
                format!("a component is named veredas-<nome>, <nome> lowercase ASCII (got `{}`)", m.name),
            )),
            None => {}
        }
        let folder = m.dir.rsplit('/').next().unwrap_or("");
        if folder != m.name {
            out.push(Violation::new("L2", &m.dir, format!("the folder is named after the crate ({})", m.name)));
        }
    }
    out
}

/// L3: no shared workspace version; every component states its own SemVer, from 2.0.0.
pub fn versioning(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    if s.workspace_version {
        out.push(Violation::new("L3", "Cargo.toml", "[workspace.package] carries a version; each component owns its own"));
    }
    for m in s.components() {
        if m.version_from_workspace {
            out.push(Violation::new("L3", &m.dir, "version.workspace = true; a component states its own version"));
            continue;
        }
        match &m.version {
            None => out.push(Violation::new("L3", &m.dir, "no version")),
            Some(v) => match semver::Version::parse(v) {
                Err(_) => out.push(Violation::new("L3", &m.dir, format!("version `{v}` is not SemVer"))),
                Ok(parsed) if parsed.major < FIRST_MAJOR => out.push(Violation::new(
                    "L3",
                    &m.dir,
                    format!("version {v} is below 2.0.0 (every component starts at 2.0.0)"),
                )),
                Ok(_) => {}
            },
        }
    }
    out
}

/// L4: core depends on no other member, in any table (itself, for its own tests, is allowed).
pub fn core(s: &Suite) -> Vec<Violation> {
    let names = s.names();
    let mut out = Vec::new();
    if let Some(c) = s.members.iter().find(|m| m.name == CORE) {
        for d in &c.deps {
            if d.name != CORE && names.contains(d.name.as_str()) {
                out.push(Violation::new(
                    "L4",
                    &c.dir,
                    format!("depends on {} in [{}]; core depends on no other member", d.name, d.table),
                ));
            }
        }
    }
    out
}

/// L5: every component's normal and build edges to other members are in the map.
pub fn direction(s: &Suite) -> Vec<Violation> {
    let names = s.names();
    let mut out = Vec::new();
    for m in s.components() {
        let allowed = allowed_for(&m.name);
        for d in m.deps.iter().filter(|d| d.table != DepTable::Dev) {
            if d.name == m.name || !names.contains(d.name.as_str()) {
                continue;
            }
            if !allowed.iter().any(|p| matches(p, &d.name)) {
                out.push(Violation::new(
                    "L5",
                    &m.dir,
                    format!("depends on {} in [{}]; the map allows {} → {}", d.name, d.table, m.name, allowed.join(", ")),
                ));
            }
        }
    }
    out
}

/// The plan a component must have: `docs/<nome>-plan.md`, the language's shared one for
/// `orcl-*`, none for a name L2 refuses anyway.
fn plan_of(name: &str) -> Option<String> {
    if name.starts_with(LANGUAGE) {
        Some(LANGUAGE_PLAN.to_string())
    } else {
        name.strip_prefix(PREFIX).map(|nome| format!("docs/{nome}-plan.md"))
    }
}

/// L6 on the tree: a component's plan exists and is not blank.
pub fn plans(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in s.components() {
        let Some(plan) = plan_of(&m.name) else { continue }; // L2 names it
        match s.plans.get(&plan) {
            None => out.push(Violation::new("L6", &m.dir, format!("no {plan} (a role nobody covers, written before the code)"))),
            Some(text) if is_blank(text) => out.push(Violation::new("L6", &m.dir, format!("{plan} is empty"))),
            Some(_) => {}
        }
    }
    out
}

/// Runs git at `root`; `Some(stdout)` on success, `None` when git said no, an error when it could
/// not run at all. The input is fed from its own thread while the output is read, so a long
/// `--batch` conversation never deadlocks on a full pipe (refute round 4 of C-006: it hung past a
/// thousand commits).
fn git_bytes(root: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Option<Vec<u8>>, Error> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let failed = |source: io::Error| Error::Io { path: "git".to_string(), source };
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    let feeder = match (stdin, child.stdin.take()) {
        (Some(data), Some(mut pipe)) => {
            let data = data.to_vec();
            Some(std::thread::spawn(move || pipe.write_all(&data)))
        }
        _ => None,
    };
    let out = child.wait_with_output().map_err(failed)?;
    if let Some(feeder) = feeder {
        let _ = feeder.join(); // a pipe git closed early is git's answer, read below
    }
    Ok(if out.status.success() { Some(out.stdout) } else { None })
}

fn git(root: &Path, args: &[&str], stdin: Option<&str>) -> Result<Option<String>, Error> {
    Ok(git_bytes(root, args, stdin.map(str::as_bytes))?.map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// One entry of a git tree object: its mode, its name, its object id.
struct TreeEntry {
    mode: String,
    name: String,
    oid: String,
}

/// The entries of a raw tree object: `<mode> <name>\0<20-byte id>`, repeated.
fn tree_entries(raw: &[u8]) -> Vec<TreeEntry> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < raw.len() {
        let Some(sp) = raw[at..].iter().position(|b| *b == b' ') else { break };
        let Some(nul) = raw[at + sp..].iter().position(|b| *b == 0) else { break };
        let mode = String::from_utf8_lossy(&raw[at..at + sp]).into_owned();
        let name = String::from_utf8_lossy(&raw[at + sp + 1..at + sp + nul]).into_owned();
        let id_at = at + sp + nul + 1;
        if id_at + 20 > raw.len() {
            break;
        }
        let oid = raw[id_at..id_at + 20].iter().map(|b| format!("{b:02x}")).collect();
        out.push(TreeEntry { mode, name, oid });
        at = id_at + 20;
    }
    out
}

/// The objects a `git cat-file --batch` answers, in the order asked: `None` for a missing one,
/// else `(oid, type, bytes)`.
fn batch_objects(raw: &[u8]) -> Vec<Option<(String, String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < raw.len() {
        let Some(nl) = raw[at..].iter().position(|b| *b == b'\n') else { break };
        let header = String::from_utf8_lossy(&raw[at..at + nl]).into_owned();
        at += nl + 1;
        if header.ends_with(" missing") {
            out.push(None);
            continue;
        }
        let mut parts = header.split(' ');
        let (Some(oid), Some(kind), Some(size)) = (parts.next(), parts.next(), parts.next().and_then(|s| s.parse::<usize>().ok())) else { break };
        if at + size > raw.len() {
            break;
        }
        out.push(Some((oid.to_string(), kind.to_string(), raw[at..at + size].to_vec())));
        at += size + 1; // the body, then its newline
    }
    out
}

/// L6 over history: every commit reachable from HEAD that holds a component's folder under
/// `crates/` (`veredas-*` or `orcl-*`, a folder, a submodule or a link by that name, present at
/// HEAD or not: renamed and deleted components included) also holds that component's plan as a
/// regular file of text, not blank, so no component's code ever landed without it, through
/// merges, re-additions and deletions alike. A fixed number of git conversations whatever the
/// history's length: the `crates`, `docs` and `docs/orcl` trees of every commit, the distinct
/// trees' entries, the distinct plan blobs. The earliest offending commit is named, once per
/// component folder. A tree that is no repository, or a shallow clone with its history cut, is
/// an error.
pub fn history(root: &Path) -> Result<Vec<Violation>, Error> {
    let refuse = |what: &str| Error::Io { path: root.display().to_string(), source: io::Error::other(what.to_string()) };
    git(root, &["rev-parse", "--is-inside-work-tree"], None)?.ok_or_else(|| refuse("not a git repository"))?;
    if git(root, &["rev-parse", "--is-shallow-repository"], None)?.is_some_and(|o| o.trim() == "true") {
        return Err(refuse("a shallow clone: the history to judge is cut"));
    }
    if git(root, &["rev-parse", "--verify", "-q", "HEAD"], None)?.is_none() {
        return Ok(Vec::new()); // no commit yet: nothing has landed
    }
    let commits: Vec<String> = git(root, &["rev-list", "--reverse", "HEAD"], None)?
        .ok_or_else(|| refuse("git rev-list failed"))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    const FOLDERS: [&str; 3] = ["crates", "docs", "docs/orcl"];
    // 1. which tree each commit holds at each of the three folders
    let mut asks = String::new();
    for c in &commits {
        for f in FOLDERS {
            asks.push_str(&format!("{c}:{f}\n"));
        }
    }
    let answers = git(root, &["cat-file", "--batch-check"], Some(&asks))?.ok_or_else(|| refuse("git cat-file failed"))?;
    let trees_at: Vec<Option<String>> = answers
        .lines()
        .map(|l| {
            let mut parts = l.split(' ');
            match (parts.next(), parts.next()) {
                (Some(oid), Some("tree")) => Some(oid.to_string()),
                _ => None, // missing, or a file where a folder was expected
            }
        })
        .collect();
    if trees_at.len() != commits.len() * FOLDERS.len() {
        return Err(refuse("git cat-file answered out of step"));
    }
    // 2. the entries of every distinct tree, read once
    let distinct: BTreeSet<&String> = trees_at.iter().flatten().collect();
    let mut entries: BTreeMap<String, Vec<TreeEntry>> = BTreeMap::new();
    if !distinct.is_empty() {
        let asks: String = distinct.iter().map(|t| format!("{t}\n")).collect();
        let raw = git_bytes(root, &["cat-file", "--batch"], Some(asks.as_bytes()))?.ok_or_else(|| refuse("git cat-file failed"))?;
        for object in batch_objects(&raw).into_iter().flatten() {
            entries.insert(object.0, tree_entries(&object.2));
        }
    }
    // 3. every commit, every component folder it holds (a folder, a submodule or a link by that
    //    name), and the plan blob it finds for it, if a regular file; the suite's own plan is
    //    nobody's
    struct Hold {
        short: String,
        dir: String,
        plan: String,
        blob: Option<String>,
    }
    let mut holds: Vec<Hold> = Vec::new();
    for (i, c) in commits.iter().enumerate() {
        let Some(crates) = &trees_at[i * 3] else { continue };
        let docs = trees_at[i * 3 + 1].as_ref().and_then(|t| entries.get(t));
        let orcl = trees_at[i * 3 + 2].as_ref().and_then(|t| entries.get(t));
        for e in entries.get(crates).map(Vec::as_slice).unwrap_or(&[]) {
            if !matches!(e.mode.as_str(), "40000" | "160000" | "120000") {
                continue;
            }
            let folded = e.name.replace('_', "-");
            let (plan_dir, plan_file, plan) = if folded.starts_with(LANGUAGE) {
                (orcl, "README.md".to_string(), LANGUAGE_PLAN.to_string())
            } else if let Some(nome) = folded.strip_prefix(PREFIX) {
                (docs, format!("{nome}-plan.md"), format!("docs/{nome}-plan.md"))
            } else {
                continue;
            };
            let blob = if plan == SUITE_PLAN {
                None
            } else {
                plan_dir
                    .and_then(|d| d.iter().find(|x| x.name == plan_file))
                    .filter(|x| x.mode == "100644" || x.mode == "100755")
                    .map(|x| x.oid.clone())
            };
            holds.push(Hold { short: c[..c.len().min(7)].to_string(), dir: format!("crates/{}", e.name), plan, blob });
        }
    }
    // 4. the distinct plan blobs, read once: blank, or not text at all, is blank
    let distinct: BTreeSet<&String> = holds.iter().filter_map(|h| h.blob.as_ref()).collect();
    let mut blank_of: BTreeMap<String, bool> = BTreeMap::new();
    if !distinct.is_empty() {
        let asks: String = distinct.iter().map(|b| format!("{b}\n")).collect();
        let raw = git_bytes(root, &["cat-file", "--batch"], Some(asks.as_bytes()))?.ok_or_else(|| refuse("git cat-file failed"))?;
        for object in batch_objects(&raw).into_iter().flatten() {
            blank_of.insert(object.0, std::str::from_utf8(&object.2).map(is_blank).unwrap_or(true));
        }
    }
    // 5. the earliest offending commit, once per folder
    let mut named: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for h in holds {
        if named.contains(&h.dir) {
            continue;
        }
        match &h.blob {
            None => {
                out.push(Violation::new("L6", &h.dir, format!("commit {} holds the code and no {}", h.short, h.plan)));
                named.insert(h.dir);
            }
            Some(oid) if blank_of.get(oid).copied().unwrap_or(true) => {
                out.push(Violation::new("L6", &h.dir, format!("commit {} holds the code and {} blank", h.short, h.plan)));
                named.insert(h.dir);
            }
            Some(_) => {}
        }
    }
    Ok(out)
}

/// Every law, in order.
pub fn all(s: &Suite) -> Vec<Violation> {
    let mut out = roots(s);
    out.extend(naming(s));
    out.extend(versioning(s));
    out.extend(core(s));
    out.extend(direction(s));
    out.extend(plans(s));
    out
}
