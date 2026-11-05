//! The suite's laws, machine-enforced.
//!
//! `docs/veredas-plan.md` states the rules of the Veredas 2 workspace; this crate checks the ones a
//! machine can check from the shape of the tree, and `tests/leis.rs` is the verdict
//! (`cargo test -p leis`). Each law is a pure function over a [`Suite`] built from the manifests, so
//! the tests attack the laws on synthetic suites and then run them on the real tree
//! ([`Suite::read`]). A law changes only with the contract clause that states it
//! (`.zen/contract.md`, C-001..C-006).
//!
//! THE LAWS (a component is a member under `crates/`; a member under `tools/` is repo tooling)
//!   L1 ROOTS       every crate under `crates/` or `tools/` is a workspace member, listed by path: a
//!                  glob is not a member, an unlisted crate is refused unless excluded, and every member
//!                  lives under one of those two roots.
//!   L2 NAMING      a component is `veredas-<nome>`, `<nome>` lowercase ASCII, in a folder of the same
//!                  name; the language's crates (`orcl-*`) keep their names.
//!   L3 VERSIONING  no `[workspace.package] version`; every component states its own SemVer, from 2.0.0.
//!   L4 CORE        `veredas-core` depends on no other member, in any dependency table.
//!   L5 DIRECTION   every member→member edge (normal and build dependencies) is in [`MAP`]: anything
//!                  may use core, the apps use portal, the bench and the scanner never pull portal,
//!                  controlador embeds portal and aranha and runs ORCL. A dev-dependency is free (a
//!                  bench is a fixture); a tool may depend on anything.
//!   L6 PLAN        a component has its `docs/<nome>-plan.md`, non-empty, before its code; the
//!                  language's crates share `docs/orcl/README.md`.

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

/// One declared dependency: the package it names (the `package` rename when there is one, else
/// the key) and the table it sits in. Target-specific tables count like the plain ones.
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
    /// `[workspace] exclude`, normalized.
    pub excluded: Vec<String>,
    /// `[workspace.package]` carries a `version`.
    pub workspace_version: bool,
    /// Every folder under a root that holds a `Cargo.toml`.
    pub found: Vec<String>,
    /// The listed members whose manifest was found, in listed order.
    pub members: Vec<Member>,
    /// `docs/*-plan.md` and `docs/orcl/README.md`, path to content.
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
}

#[derive(Deserialize, Default)]
struct DepTables {
    #[serde(default)]
    dependencies: toml::Table,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: toml::Table,
    #[serde(default, rename = "build-dependencies")]
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

fn deps_of(table: &toml::Table, kind: DepTable) -> Vec<Dep> {
    let mut keys: Vec<&String> = table.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|key| {
            let renamed = table[key].as_table().and_then(|t| t.get("package")).and_then(|v| v.as_str());
            Dep { name: renamed.unwrap_or(key).to_string(), table: kind }
        })
        .collect()
}

fn all_deps(tables: &DepTables) -> Vec<Dep> {
    let mut deps = deps_of(&tables.dependencies, DepTable::Normal);
    deps.extend(deps_of(&tables.dev_dependencies, DepTable::Dev));
    deps.extend(deps_of(&tables.build_dependencies, DepTable::Build));
    deps
}

impl Member {
    fn parse(dir: &str, text: &str) -> Result<Member, Error> {
        let m: Manifest = toml::from_str(text)
            .map_err(|source| Error::Manifest { path: format!("{dir}/Cargo.toml"), source })?;
        let (version, version_from_workspace) = match &m.package.version {
            Some(toml::Value::String(s)) => (Some(s.clone()), false),
            Some(toml::Value::Table(t)) => (None, t.get("workspace").and_then(|v| v.as_bool()).unwrap_or(false)),
            _ => (None, false),
        };
        let mut deps = all_deps(&m.deps);
        for tables in m.target.values() {
            deps.extend(all_deps(tables));
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
                members.push(Member::parse(dir, text)?);
            }
        }
        Ok(Suite {
            listed,
            excluded: ws.workspace.exclude.iter().map(|m| normalize(m)).collect(),
            workspace_version: ws.workspace.package.contains_key("version"),
            found,
            members,
            plans,
        })
    }

    /// The real tree: `Cargo.toml`, every `crates/*/Cargo.toml` and `tools/*/Cargo.toml`, every
    /// `docs/*-plan.md` and `docs/orcl/README.md`.
    pub fn read(root: &Path) -> Result<Suite, Error> {
        let read = |rel: &str| -> Result<String, Error> {
            fs::read_to_string(root.join(rel)).map_err(|source| Error::Io { path: rel.to_string(), source })
        };
        let workspace = read("Cargo.toml")?;
        let mut crates: Vec<(String, String)> = Vec::new();
        for r in ROOTS {
            for name in folder_entries(root, r)? {
                let rel = format!("{r}/{name}");
                if root.join(&rel).join("Cargo.toml").is_file() {
                    let text = read(&format!("{rel}/Cargo.toml"))?;
                    crates.push((rel, text));
                }
            }
        }
        let mut plans = BTreeMap::new();
        for name in folder_entries(root, "docs")? {
            if name.ends_with("-plan.md") {
                let rel = format!("docs/{name}");
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

fn under_a_root(dir: &str) -> bool {
    let mut parts = dir.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(root), Some(name), None) if ROOTS.contains(&root) && !name.is_empty()
    )
}

fn is_nome(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
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

/// L1: every crate under a root is a listed member, every listed member is under a root.
pub fn roots(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in &s.listed {
        if !under_a_root(m) {
            out.push(Violation::new("L1", m, "a member lives under crates/ or tools/"));
        } else if !s.found.contains(m) {
            out.push(Violation::new("L1", m, "listed, but no Cargo.toml there (a glob is not a member; list each one)"));
        }
    }
    for c in &s.found {
        if !s.listed.contains(c) && !s.excluded.contains(c) {
            out.push(Violation::new("L1", c, "has a Cargo.toml but is not a workspace member (list it, or exclude it)"));
        }
    }
    out
}

/// L2: a component is `veredas-<nome>` in a folder of its own name; `orcl-*` keep their names.
pub fn naming(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in s.components() {
        let named = m.name.starts_with(LANGUAGE) || m.name.strip_prefix(PREFIX).is_some_and(is_nome);
        if !named {
            out.push(Violation::new(
                "L2",
                &m.dir,
                format!("a component is named veredas-<nome>, <nome> lowercase ASCII (got `{}`)", m.name),
            ));
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

/// L6: a component's plan exists, non-empty, before its code.
pub fn plans(s: &Suite) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in s.components() {
        let plan = if m.name.starts_with(LANGUAGE) {
            LANGUAGE_PLAN.to_string()
        } else {
            match m.name.strip_prefix(PREFIX) {
                Some(nome) => format!("docs/{nome}-plan.md"),
                None => continue, // L2 names it
            }
        };
        match s.plans.get(&plan) {
            None => out.push(Violation::new("L6", &m.dir, format!("no {plan} (a role nobody covers, written before the code)"))),
            Some(text) if text.trim().is_empty() => out.push(Violation::new("L6", &m.dir, format!("{plan} is empty"))),
            Some(_) => {}
        }
    }
    out
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
