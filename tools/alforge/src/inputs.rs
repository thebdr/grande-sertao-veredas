//! What a component's version names in a release (docs/questions.md Q441, ruled 2026-11-25 "All
//! build inputs"): everything a build of its executables reads but the toolchain, so a change to
//! any of it bumps the version. Its folder and the folder of every member it uses (its normal and
//! build dependencies by path, transitively; a test's never goes into an executable); every file
//! outside those folders their sources name by a literal `#[path]`, `include_str!`, `include_bytes!`
//! or `include!`; the lock file's entries for its external dependencies, transitively; the
//! workspace's manifest; and the cargo configuration at the root. Each input is named and
//! identified as git stores it at the commit (a folder by its tree, a file by its blob, a locked
//! package by its source and checksum), so two commits are compared without building either.
//!
//! The cut builds each component alone, so cargo unifies features within its own inputs and no
//! other component's manifest reaches its executables, and holds what cargo's dep-info says each
//! compilation read to these inputs ([`read_outside`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component as Part, Path, PathBuf};

use crate::git;

/// The registry every crate of the suite comes from, left out of a locked package's name.
const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// A component's build inputs: each named (a folder ending in `/`, a file by its path, a locked
/// package as `Cargo.lock: <name> <version>`) with its identity at the commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs(pub BTreeMap<String, String>);

impl Inputs {
    /// The inputs whose identity differs between the two (one there and not in the other included),
    /// in name order.
    pub fn differ(&self, other: &Inputs) -> Vec<String> {
        let names: BTreeSet<&String> = self.0.keys().chain(other.0.keys()).collect();
        names.into_iter().filter(|n| self.0.get(*n) != other.0.get(*n)).cloned().collect()
    }

    /// Whether a file of the tree (its path from the root) is one of these inputs or lies in one of
    /// their folders.
    pub fn covers(&self, path: &str) -> bool {
        self.0.contains_key(path) || self.0.keys().any(|k| k.ends_with('/') && path.starts_with(k.as_str()))
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
struct Locked {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    dependencies: Vec<String>,
    /// What a `[replace]` puts in its place (`name version`, a package by path).
    replace: Option<String>,
}

impl Locked {
    fn named(&self) -> String {
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
        let table: toml::Table = toml::from_str(&text).map_err(|e| format!("its Cargo.lock does not read: {}", e.message()))?;
        let mut out = Vec::new();
        for p in table.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
            let s = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let (Some(name), Some(version)) = (s("name"), s("version")) else {
                return Err("its Cargo.lock holds a package without a name or a version".to_string());
            };
            let dependencies = p.get("dependencies").and_then(|d| d.as_array()).into_iter().flatten().filter_map(|d| d.as_str().map(str::to_string)).collect();
            out.push(Locked { name, version, source: s("source"), checksum: s("checksum"), dependencies, replace: s("replace") });
        }
        Ok(Some(out))
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
        // The workspace's manifest whole, any byte of it charging every component (docs/questions.md
        // Q442: what cargo reads of it for this component alone is recommended).
        // DECISION PENDING (Q442)
        for f in ["Cargo.toml", ".cargo/config.toml", ".cargo/config"] {
            named.insert(f.to_string(), self.object(f)?);
        }
        // The members it uses, by path, transitively.
        let by_dir: BTreeMap<&str, &Member> = self.members.iter().map(|m| (m.dir.as_str(), m)).collect();
        let start = self.members.iter().find(|m| m.name == name).ok_or_else(|| format!("{name} is no member"))?;
        let mut used: Vec<&Member> = vec![start];
        let mut seen: BTreeSet<&str> = BTreeSet::from([start.dir.as_str()]);
        let mut i = 0;
        while i < used.len() {
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
        }
        for m in &used {
            named.insert(format!("{}/", m.dir), self.object(&m.dir)?);
            // A package of another workspace inherits keys from that workspace's manifest.
            if !m.workspace.is_empty() {
                let f = format!("{}/Cargo.toml", m.workspace);
                let id = self.object(&f)?;
                named.insert(f, id);
            }
        }
        // Files outside those folders their sources name.
        let folders: Vec<String> = used.iter().map(|m| format!("{}/", m.dir)).collect();
        for path in self.named_outside(&used, &folders)? {
            let id = self.object(&path)?;
            named.insert(path, id);
        }
        // The lock file's entries for their external dependencies, transitively.
        let mut queue: Vec<usize> = Vec::new();
        for m in &used {
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
            let own = lock
                .iter()
                .find(|l| l.name == m.name && l.version == m.version && l.source.is_none())
                .ok_or_else(|| format!("its Cargo.lock does not pin {} {}: the lock file is not this workspace's", m.name, m.version))?;
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
                    queue.push(at);
                }
            }
        }
        let mut taken: BTreeSet<usize> = BTreeSet::new();
        while let Some(at) = queue.pop() {
            if !taken.insert(at) {
                continue;
            }
            let lock = lock.as_ref().expect("a package was resolved in it");
            let l = &lock[at];
            if l.source.is_none() {
                // A package by path the workspace knows (a member, or a fork one uses by path, as
                // a `[patch]` may name it too): its folder, and its workspace's manifest.
                let Some(known) = self.members.iter().find(|m| m.name == l.name && m.version == l.version) else {
                    return Err(format!("its Cargo.lock's {} {} has no source and is no member a release builds from its folder", l.name, l.version));
                };
                named.insert(format!("{}/", known.dir), self.object(&known.dir)?);
                if !known.workspace.is_empty() {
                    let f = format!("{}/Cargo.toml", known.workspace);
                    let id = self.object(&f)?;
                    named.insert(f, id);
                }
                for d in &l.dependencies {
                    queue.push(resolve(lock, d)?);
                }
                continue;
            }
            named.insert(l.named(), l.checksum.clone().unwrap_or_default());
            for d in &l.dependencies {
                queue.push(resolve(lock, d)?);
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
                queue.push(at.ok_or_else(|| format!("its Cargo.lock replaces {} {} by `{r}`, which it does not pin", l.name, l.version))?);
            }
        }
        let mut packages = Packages { names: used.iter().map(|m| m.name.clone()).collect(), vendored: BTreeSet::new() };
        if let Some(lock) = &lock {
            for &at in &taken {
                let l = &lock[at];
                packages.names.insert(l.name.clone());
                if l.source.is_some() {
                    packages.vendored.insert(format!("{}-{}", l.name, l.version));
                }
            }
        }
        Ok((Inputs(named), packages))
    }

    /// The files outside `folders` that the Rust sources of `used` (but their tests', benches' and
    /// examples') name by a literal `#[path]`, `include_str!`, `include_bytes!` or `include!`,
    /// resolved from the naming file's folder; a named Rust file is read in turn.
    fn named_outside(&self, used: &[&Member], folders: &[String]) -> Result<BTreeSet<String>, String> {
        let mut files: Vec<String> = Vec::new();
        let mut out = BTreeSet::new();
        for m in used {
            walk(self.tree, &m.dir, &mut files).map_err(|e| format!("{}'s sources do not read: {e}", m.dir))?;
        }
        // Every Rust file a target starts from or a source names is read, wherever it lies (a
        // shipped target's source may sit in a folder's `examples/`, which the sweep leaves out).
        let mut scanned: BTreeSet<String> = files.iter().cloned().collect();
        for m in used {
            // A target whose source lies outside the folders: that file, and what it names.
            for s in &m.sources {
                let path = normal(Path::new(s)).ok_or_else(|| format!("{} builds from `{s}`, outside the commit's tree", m.name))?;
                if !folders.iter().any(|f| path.starts_with(f.as_str())) {
                    out.insert(path.clone());
                }
                if scanned.insert(path.clone()) {
                    files.push(path);
                }
            }
        }
        let mut i = 0;
        while i < files.len() {
            let file = files[i].clone();
            i += 1;
            let Ok(text) = std::fs::read_to_string(self.tree.join(&file)) else { continue };
            let from = Path::new(&file).parent().unwrap_or(Path::new(""));
            for (lit, rust) in named(&text) {
                let path = normal(&from.join(&lit)).ok_or_else(|| format!("{file} names `{lit}`, outside the commit's tree"))?;
                if !folders.iter().any(|f| path.starts_with(f.as_str())) {
                    out.insert(path.clone());
                }
                // Read in turn when rustc reads it as Rust, whatever its name ends with
                // (`include!("x.in")`); a file of data (`include_str!`) names nothing.
                if rust && self.tree.join(&path).is_file() && scanned.insert(path.clone()) {
                    files.push(path);
                }
            }
        }
        Ok(out)
    }
}

/// A lock file's dependency string (`name`, `name version` or `name version (source)`) resolved to
/// the one package it names.
fn resolve(lock: &[Locked], d: &str) -> Result<usize, String> {
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
    match found[..] {
        [one] => Ok(one),
        [] => Err(format!("its Cargo.lock names `{d}` and pins no such package")),
        _ => Err(format!("its Cargo.lock names `{d}`, which more than one of its packages answers")),
    }
}

/// Every `.rs` file under `dir` of the tree (paths from the root, in name order), its tests',
/// benches' and examples' included (a shipped target may start there; docs/questions.md Q445 asks
/// whether what tests alone read should charge).
fn walk(tree: &Path, dir: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    fn inner(tree: &Path, rel: &str, top: bool, out: &mut Vec<String>) -> std::io::Result<()> {
        let _ = top;
        let mut entries: Vec<_> = std::fs::read_dir(tree.join(rel))?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = format!("{rel}/{name}");
            let kind = e.file_type()?;
            if kind.is_dir() {
                inner(tree, &path, false, out)?;
            } else if kind.is_file() && name.ends_with(".rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    inner(tree, dir, true, out)
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
    match lf.parse::<proc_macro2::TokenStream>() {
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
            let mut out: Vec<(String, bool)> = literals_in(&lf).into_iter().map(|p| (p, true)).collect();
            for l in literals_in(&uncommented(&lf)) {
                if !out.iter().any(|(p, _)| *p == l) {
                    out.push((l, true));
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

fn literals_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
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
            let before = inner[..inner.len() - scan.len() + p].trim_end();
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
            .and_then(|s| s.trim_start().strip_prefix("path"))
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
    out
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
                    let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                '\n' => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
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
pub fn read_outside(dep_info: &str, src: &str, vendor: &str, inputs: &Inputs, packages: &Packages) -> Vec<String> {
    // The builds share one output folder and one copy of the vendored sources: a build may read
    // there only what its own packages made or brought (a build script's `OUT_DIR` and a locked
    // crate's vendored folder), never another's. The package named whole (`foo` never owns
    // `foo-bar`), and a vendored crate at the version the build compiles.
    let hash = |h: &str| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let built_by = |first: &str, second: Option<&str>| {
        (packages.names.contains(first) && second.is_some_and(hash))
            || packages.names.iter().any(|p| first.strip_prefix(p.as_str()).and_then(|r| r.strip_prefix('-')).is_some_and(hash))
    };
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
        let landed = PathBuf::from(format!("/{}", rooted(&path)));
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
                c if c.is_whitespace() => {
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
