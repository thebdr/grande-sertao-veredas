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
//!                  anywhere in the repository is a member under a root or nothing, never excluded.
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
//!                  reachable from HEAD that holds the component ([`history`], over git); the
//!                  language's crates share `docs/orcl/README.md`. The suite's own plan is nobody's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

/// The folders a member may live under.
pub const ROOTS: [&str; 2] = ["crates", "tools"];
/// Folders the reader never descends into: git's own store, build output, a frontend's modules.
/// A hidden folder is walked like any other: `.github` and `.zen` are committed here, and a
/// suite crate parked under a dot is still parked.
pub const SKIPPED: [&str; 3] = [".git", "target", "node_modules"];
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
    /// Every folder in the repository that holds a `Cargo.toml` (the root's own left out), at
    /// any depth outside the skipped folders: under a root or anywhere else.
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

    /// The real tree: `Cargo.toml`, every other `Cargo.toml` in the repository at any depth
    /// (`.git/`, `target/` and `node_modules/` skipped), every `docs/*-plan.md` but the suite's
    /// own, and `docs/orcl/README.md`.
    pub fn read(root: &Path) -> Result<Suite, Error> {
        let read = |rel: &str| -> Result<String, Error> {
            fs::read_to_string(root.join(rel)).map_err(|source| Error::Io { path: rel.to_string(), source })
        };
        let workspace = read("Cargo.toml")?;
        let mut dirs = Vec::new();
        for name in folder_entries(root, ".")? {
            if !SKIPPED.contains(&name.as_str()) && root.join(&name).is_dir() {
                collect_crates(root, &name, &mut dirs)?;
            }
        }
        let mut crates: Vec<(String, String)> = Vec::new();
        for rel in dirs {
            let text = read(&format!("{rel}/Cargo.toml"))?;
            crates.push((rel, text));
        }
        let mut plans = BTreeMap::new();
        for name in folder_entries(root, "docs")? {
            let rel = format!("docs/{name}");
            if name.ends_with("-plan.md") && rel != SUITE_PLAN && root.join(&rel).is_file() {
                plans.insert(rel.clone(), read(&rel)?);
            }
        }
        if root.join(LANGUAGE_PLAN).is_file() {
            plans.insert(LANGUAGE_PLAN.to_string(), read(LANGUAGE_PLAN)?);
        }
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

/// The entries of `root/<rel>`, sorted by name; an absent folder is empty.
fn folder_entries(root: &Path, rel: &str) -> Result<Vec<String>, Error> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let io = |source: io::Error| Error::Io { path: rel.to_string(), source };
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(io)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(io)?;
    names.sort();
    Ok(names)
}

/// Every folder under `root/<rel>` (itself included) that holds a `Cargo.toml`, depth first in
/// name order, never descending into [`SKIPPED`].
fn collect_crates(root: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), Error> {
    if !root.join(rel).is_dir() {
        return Ok(());
    }
    if root.join(rel).join("Cargo.toml").is_file() {
        out.push(rel.to_string());
    }
    for name in folder_entries(root, rel)? {
        if SKIPPED.contains(&name.as_str()) {
            continue;
        }
        let child = format!("{rel}/{name}");
        if root.join(&child).is_dir() {
            collect_crates(root, &child, out)?;
        }
    }
    Ok(())
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
/// not run at all.
fn git(root: &Path, args: &[&str], stdin: Option<&str>) -> Result<Option<String>, Error> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::Io { path: "git".to_string(), source })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).map_err(|source| Error::Io { path: "git".to_string(), source })?;
    }
    let out = child.wait_with_output().map_err(|source| Error::Io { path: "git".to_string(), source })?;
    if out.status.success() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    } else {
        Ok(None)
    }
}

/// L6 over history: every commit reachable from HEAD that holds a component's manifest also
/// holds its plan, not blank, so no component's code ever landed without it, through merges,
/// re-additions and deletions alike. One `git cat-file --batch-check` over every (commit, path)
/// pair, then each distinct plan blob read once. The earliest offending commit is named. A
/// component not yet committed is the tree law's business; a tree that is no repository, or a
/// shallow clone with its history cut, is an error.
pub fn history(root: &Path) -> Result<Vec<Violation>, Error> {
    let s = Suite::read(root)?;
    let refuse = |what: &str| Error::Io { path: root.display().to_string(), source: io::Error::other(what.to_string()) };
    git(root, &["rev-parse", "--is-inside-work-tree"], None)?.ok_or_else(|| refuse("not a git repository"))?;
    if git(root, &["rev-parse", "--is-shallow-repository"], None)?.is_some_and(|o| o.trim() == "true") {
        return Err(refuse("a shallow clone: the history to judge is cut"));
    }
    if git(root, &["rev-parse", "--verify", "-q", "HEAD"], None)?.is_none() {
        return Ok(Vec::new()); // no commit yet: nothing has landed
    }
    let wanted: Vec<(&Member, String)> = s.components().filter_map(|m| plan_of(&m.name).map(|p| (m, p))).collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let commits: Vec<String> = git(root, &["rev-list", "--reverse", "HEAD"], None)?
        .ok_or_else(|| refuse("git rev-list failed"))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let mut asks = String::new();
    for c in &commits {
        for (m, plan) in &wanted {
            asks.push_str(&format!("{c}:{}/Cargo.toml\n{c}:{plan}\n", m.dir));
        }
    }
    let answers = git(root, &["cat-file", "--batch-check"], Some(&asks))?.ok_or_else(|| refuse("git cat-file failed"))?;
    // one answer per ask, in order: `<oid> <type> <size>` or `<ask> missing`
    let found: Vec<Option<String>> = answers
        .lines()
        .map(|l| if l.ends_with(" missing") { None } else { l.split(' ').next().map(String::from) })
        .collect();
    if found.len() != commits.len() * wanted.len() * 2 {
        return Err(refuse("git cat-file answered out of step"));
    }
    let mut blank_of: BTreeMap<String, bool> = BTreeMap::new();
    let mut out = Vec::new();
    for (k, (m, plan)) in wanted.iter().enumerate() {
        for (i, c) in commits.iter().enumerate() {
            let at = (i * wanted.len() + k) * 2;
            if found[at].is_none() {
                continue; // this commit does not hold the component
            }
            let short = &c[..c.len().min(7)];
            match &found[at + 1] {
                None => {
                    out.push(Violation::new("L6", &m.dir, format!("commit {short} holds the code and no {plan}")));
                    break;
                }
                Some(oid) => {
                    if !blank_of.contains_key(oid) {
                        let text = git(root, &["cat-file", "-p", oid], None)?.ok_or_else(|| refuse("git cat-file failed"))?;
                        blank_of.insert(oid.clone(), is_blank(&text));
                    }
                    if blank_of[oid] {
                        out.push(Violation::new("L6", &m.dir, format!("commit {short} holds the code and {plan} blank")));
                        break;
                    }
                }
            }
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
