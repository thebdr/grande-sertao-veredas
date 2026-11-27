//! Comissionador's own rules that no suite law checks (contract: C-200, the component clause;
//! the laws in `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library first, one executable on top (`docs/comissionador-plan.md`, "Rules"): cargo is asked
//! what it would build (`cargo metadata --no-deps`, which parses and builds nothing), and the
//! targets are exactly one library, one binary named `veredas-comissionador`, and tests; no build
//! script. Its members among its dependencies are core and portal at most (L5's row). And it arms
//! portal's write switch in one place, for a run whose gated writes the person confirmed
//! (`docs/questions.md` Q314 (b)): its sources, read as the compiler's tokens, name the session's
//! arming once, in the executable, and the plant's in its definitions and the run's arm and
//! disarm alone; they reach the session's store by its two readers alone, and pull in no file
//! from elsewhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BINARY: &str = "veredas-comissionador";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn metadata(dir: &Path) -> Value {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn package(meta: &Value, dir: &Path) -> Value {
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()
        .clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

/// What breaks the rule: exactly one library, one binary named `binary`, everything else a test.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)], binary: &str) -> Vec<String> {
    let library = |kinds: &[String]| !kinds.is_empty() && kinds.iter().all(|k| k == "lib" || k == "rlib");
    let mut out = Vec::new();
    let (mut libs, mut bins) = (0, 0);
    for (name, kinds, crate_types) in targets {
        if kinds == &["test"] {
            continue;
        }
        if library(kinds) && library(crate_types) {
            libs += 1;
            continue;
        }
        if kinds == &["bin"] && crate_types == &["bin"] && name == binary {
            bins += 1;
            continue;
        }
        out.push(format!("{name}: {kinds:?} {crate_types:?}"));
    }
    if libs != 1 {
        out.push(format!("{libs} library targets"));
    }
    if bins != 1 {
        out.push(format!("{bins} binaries named {binary}"));
    }
    out
}

fn targets(pkg: &Value) -> Vec<(String, Vec<String>, Vec<String>)> {
    pkg["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), strings(&t["kind"]), strings(&t["crate_types"])))
        .collect()
}

#[test]
fn comissionador_is_one_library_and_one_executable_named_for_it() {
    let dir = crate_dir();
    let pkg = package(&metadata(&dir), &dir);
    let t = targets(&pkg);
    assert_eq!(breaches(&t, BINARY), Vec::<String>::new(), "{t:?}");
    let lib: Vec<_> = t.iter().filter(|(_, k, _)| k == &["lib"]).collect();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].0, "veredas_comissionador");
    assert!(!t.iter().any(|(_, k, _)| k.iter().any(|k| k == "custom-build")), "no build script: {t:?}");
    assert_eq!(pkg["version"], "2.0.0-alpha.1");
}

#[test]
fn the_rule_of_targets_refuses_what_it_should() {
    let lib = ("veredas_comissionador".to_string(), vec!["lib".to_string()], vec!["lib".to_string()]);
    let bin = (BINARY.to_string(), vec!["bin".to_string()], vec!["bin".to_string()]);
    let test = ("grammar".to_string(), vec!["test".to_string()], vec!["bin".to_string()]);
    assert_eq!(breaches(&[lib.clone(), bin.clone(), test.clone()], BINARY), Vec::<String>::new());
    assert_eq!(breaches(&[lib.clone(), test.clone()], BINARY), ["0 binaries named veredas-comissionador"]);
    let other = ("helper".to_string(), vec!["bin".to_string()], vec!["bin".to_string()]);
    assert_eq!(breaches(&[lib.clone(), bin.clone(), other], BINARY), ["helper: [\"bin\"] [\"bin\"]"]);
    let cdylib = ("veredas_comissionador".to_string(), vec!["cdylib".to_string()], vec!["cdylib".to_string()]);
    assert_eq!(
        breaches(&[cdylib, bin.clone()], BINARY),
        ["veredas_comissionador: [\"cdylib\"] [\"cdylib\"]", "0 library targets"]
    );
    let build = ("build-script-build".to_string(), vec!["custom-build".to_string()], vec!["bin".to_string()]);
    assert_eq!(breaches(&[lib, bin, build], BINARY), ["build-script-build: [\"custom-build\"] [\"bin\"]"]);
}

#[test]
fn its_members_are_core_and_portal_at_most() {
    let dir = crate_dir();
    let pkg = package(&metadata(&dir), &dir);
    let mut members: Vec<String> = pkg["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["kind"].is_null() || d["kind"] == "build")
        .map(|d| d["name"].as_str().unwrap().to_string())
        .filter(|n| n.starts_with("veredas-") || n.starts_with("orcl-"))
        .collect();
    members.sort();
    assert!(members.iter().all(|m| m == "veredas-core" || m == "veredas-portal"), "{members:?}");
    assert!(members.contains(&"veredas-core".to_string()), "{members:?}");
}

// ---- through portal, never armed ---------------------------------------------------------------

/// The reading of one Rust file: the identifiers as the compiler reads them (a raw identifier read
/// as its name, comments gone, macro bodies and groups walked), the identifiers inside attributes
/// (`#[...]`, `#![...]`, `cfg_attr` within them), the literals' source text (a doc comment's
/// text aside: `///` is a `doc` attribute to the lexer, and a comment is not code), and every
/// `writes_armed` that is not the switch's position read, `.writes_armed()`.
#[derive(Default, Debug)]
struct Reading {
    idents: Vec<String>,
    in_attributes: Vec<String>,
    literals: Vec<String>,
    switch_set: usize,
}

fn read(text: &str) -> Reading {
    fn walk(stream: proc_macro2::TokenStream, attr: bool, doc: bool, out: &mut Reading) {
        let trees: Vec<proc_macro2::TokenTree> = stream.into_iter().collect();
        for (i, tree) in trees.iter().enumerate() {
            match tree {
                proc_macro2::TokenTree::Ident(id) => {
                    let s = id.to_string();
                    let s = s.strip_prefix("r#").map(str::to_string).unwrap_or(s);
                    if s == "writes_armed" {
                        let after_dot = i > 0 && matches!(&trees[i - 1], proc_macro2::TokenTree::Punct(p) if p.as_char() == '.');
                        let called_bare = matches!(trees.get(i + 1), Some(proc_macro2::TokenTree::Group(g))
                            if g.delimiter() == proc_macro2::Delimiter::Parenthesis && g.stream().is_empty());
                        if !(after_dot && called_bare) {
                            out.switch_set += 1;
                        }
                    }
                    if attr {
                        out.in_attributes.push(s.clone());
                    }
                    out.idents.push(s);
                }
                proc_macro2::TokenTree::Group(g) => {
                    // `#` (and `!`) right before a bracket group: an attribute.
                    let is_attr = g.delimiter() == proc_macro2::Delimiter::Bracket
                        && i > 0
                        && match &trees[i - 1] {
                            proc_macro2::TokenTree::Punct(p) if p.as_char() == '#' => true,
                            proc_macro2::TokenTree::Punct(p) if p.as_char() == '!' => {
                                i > 1 && matches!(&trees[i - 2], proc_macro2::TokenTree::Punct(q) if q.as_char() == '#')
                            }
                            _ => false,
                        };
                    // A doc attribute's text is a comment's.
                    let is_doc = is_attr && matches!(g.stream().into_iter().next(), Some(proc_macro2::TokenTree::Ident(d)) if d == "doc");
                    walk(g.stream(), attr || is_attr, doc || is_doc, out);
                }
                proc_macro2::TokenTree::Literal(_) if doc => {}
                proc_macro2::TokenTree::Literal(l) => out.literals.push(l.to_string()),
                proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
    let stream: proc_macro2::TokenStream = text.parse().unwrap_or_else(|e| panic!("a source that does not lex: {e:?}"));
    let mut out = Reading::default();
    walk(stream, false, false, &mut out);
    out
}

/// The names a source of comissionador never holds, each with why.
const REFUSED: &[(&str, &str)] = &[
    ("set_writes_armed", "arms portal's switch: once, in `main.rs`, the session's arming handed to the run alone"),
    ("WriteGate", "a gate of its own"),
    ("spawn", "a worker of its own: every one that writes takes a gate"),
    ("opc", "core's or portal's OPC UA layer, past the session"),
    ("telegram", "core's telegram layer, past the session"),
    ("fieldbus", "core's fieldbus masters, past the session"),
    ("net", "a socket of its own"),
    ("tail", "portal's log layer, past the session"),
    ("FileTailSource", "a log read past the session"),
    ("FamilyTailSource", "a family of logs read past the session"),
    ("SourceSet", "portal's sources started past the session"),
    ("Source", "a source started past the session"),
    ("Command", "a process of its own"),
    ("unsafe", "code past the compiler's guarantees: a socket by the system's own calls, among others"),
    ("from_v1", "Veredas 1's project read in, its own spelling of the switch with it"),
    ("include", "tokens from a file the reading does not see"),
    ("include_str", "text from a file the reading does not see"),
    ("include_bytes", "bytes from a file the reading does not see"),
];

/// What the engine takes from portal, each a path an item of which a source may name (a method of
/// an allowed type among them): the session and what it shows, the project's reader, the store's
/// queries and the coercion and clock of portal's serving. Anything else of portal (its search of
/// logs at rest, its CSV previews, its tails, its families) reaches the plant past the session.
const PORTAL: &[&str] = &[
    "veredas_portal::session::Session",
    "veredas_portal::session::Live",
    "veredas_portal::session::HealthState",
    "veredas_portal::project::load_file",
    "veredas_portal::project::LogKind",
    "veredas_portal::SeriesType",
    "veredas_portal::store::RowsQuery",
    "veredas_portal::store::TableKind",
    "veredas_portal::store::MAX_PAGE",
    "veredas_portal::serve::store_now_ms",
    "veredas_portal::serve::coerce",
    "veredas_portal::serve::refused_code",
];

/// Every path into portal a source names, in full, a use tree's braces opened (`*` for a glob,
/// the crate's name alone when nothing follows it).
fn portal_paths(stream: proc_macro2::TokenStream, out: &mut Vec<String>) {
    use proc_macro2::TokenTree as T;
    let trees: Vec<T> = stream.into_iter().collect();
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            T::Ident(id) if id.to_string().trim_start_matches("r#") == "veredas_portal" => {
                i = path_from(&trees, i + 1, "veredas_portal".to_string(), out);
            }
            T::Group(g) => {
                portal_paths(g.stream(), out);
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// The path `prefix` continued by the `::` segments from `j`: kept whole, a use tree's braces
/// opened. Returns where it ends.
fn path_from(trees: &[proc_macro2::TokenTree], mut j: usize, mut path: String, out: &mut Vec<String>) -> usize {
    use proc_macro2::TokenTree as T;
    let colon = |k: usize| matches!(trees.get(k), Some(T::Punct(p)) if p.as_char() == ':');
    while colon(j) && colon(j + 1) {
        match trees.get(j + 2) {
            Some(T::Ident(seg)) => {
                path = format!("{path}::{}", seg.to_string().trim_start_matches("r#"));
                j += 3;
            }
            Some(T::Group(g)) if g.delimiter() == proc_macro2::Delimiter::Brace => {
                use_tree(g.stream(), &path, out);
                return j + 3;
            }
            Some(T::Punct(p)) if p.as_char() == '*' => {
                out.push(format!("{path}::*"));
                return j + 3;
            }
            _ => break,
        }
    }
    out.push(path);
    j
}

/// The items of a use tree's braces under `prefix`: `self` is the module itself.
fn use_tree(stream: proc_macro2::TokenStream, prefix: &str, out: &mut Vec<String>) {
    use proc_macro2::TokenTree as T;
    let trees: Vec<T> = stream.into_iter().collect();
    for item in trees.split(|t| matches!(t, T::Punct(p) if p.as_char() == ',')) {
        match item.first() {
            Some(T::Ident(id)) if id == "self" => out.push(prefix.to_string()),
            Some(T::Ident(id)) => {
                path_from(item, 1, format!("{prefix}::{}", id.to_string().trim_start_matches("r#")), out);
            }
            Some(T::Group(g)) if g.delimiter() == proc_macro2::Delimiter::Brace => use_tree(g.stream(), prefix, out),
            Some(T::Punct(p)) if p.as_char() == '*' => out.push(format!("{prefix}::*")),
            _ => {}
        }
    }
}

/// Every path into portal a source names that does not start with an item of [`PORTAL`].
fn portal_breaches(text: &str) -> Vec<String> {
    let stream: proc_macro2::TokenStream = text.parse().unwrap_or_else(|e| panic!("a source that does not lex: {e:?}"));
    let mut paths = Vec::new();
    portal_paths(stream, &mut paths);
    paths
        .into_iter()
        .filter(|p| !PORTAL.iter().any(|a| p == a || p.starts_with(&format!("{a}::"))))
        .map(|p| format!("{p}: portal past what the engine takes from it"))
        .collect()
}

/// What a source holds that the rule refuses, each as `name: why`.
fn breaches_of(text: &str) -> Vec<String> {
    let r = read(text);
    let mut out = Vec::new();
    for id in &r.idents {
        if let Some((_, why)) = REFUSED.iter().find(|(n, _)| n == id) {
            out.push(format!("{id}: {why}"));
        }
    }
    for _ in 0..r.switch_set {
        out.push("writes_armed: the switch's position set, not read".to_string());
    }
    if r.in_attributes.iter().any(|i| i == "path") {
        out.push("path: a module read from a file of an attribute's choosing".to_string());
    }
    for l in &r.literals {
        if ["writes_armed", "allow_writes", "allow_dashboard_writes"].iter().any(|k| l.contains(k)) {
            out.push(format!("{l}: a project's text that arms the switch"));
        }
    }
    out
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn its_library_and_executable_are_built_from_its_src_folder_alone() {
    let dir = crate_dir();
    let pkg = package(&metadata(&dir), &dir);
    let src = dir.join("src").canonicalize().unwrap();
    for t in pkg["targets"].as_array().unwrap() {
        if strings(&t["kind"]) == ["test"] {
            continue;
        }
        let path = Path::new(t["src_path"].as_str().unwrap()).canonicalize().unwrap();
        assert!(path.starts_with(&src), "{} is built from {}", t["name"], path.display());
    }
    let files = files_under(&src);
    assert!(files.len() >= 6, "{files:?}");
    for f in &files {
        assert!(f.extension().is_some_and(|x| x == "rs"), "every file under src is Rust: {}", f.display());
    }
}

/// The one file the switch is armed in: the executable's, whose session hands the run its arming,
/// used by a run for a test whose gated writes the person confirmed (Q314 (b)).
const ARMING_FILE: &str = "main.rs";

/// Where the plant's arming (`Plant::set_armed`) is named, and how often: its definitions (the
/// trait's and `LivePlant`'s) and the run's arm and disarm, nothing else.
const SET_ARMED: [(&str, usize); 2] = [("plant.rs", 2), ("run.rs", 2)];

/// Where the arming the executable hands the plant is named (the ident `arming`), and how often:
/// in `plant.rs` its field, its making empty, its handing in (twice) and its two reads, `can_arm`'s
/// and `set_armed`'s; in `main.rs` its making and its handing. Nothing else, so no method of the
/// plant and no line of the executable calls it but through `set_armed` (refute round 7).
const ARMING_HANDLE: [(&str, usize); 2] = [("plant.rs", 7), ("main.rs", 2)];

/// The session store's methods the engine reads with: the rows of a table, and the catalog.
const STORE_READERS: [&str; 2] = ["query_rows", "list_tables"];

/// Every reach of the session's store but by its readers: `store` as anything but portal's module
/// in a path or `.store.` followed by one of [`STORE_READERS`] (the store bound to a name, handed
/// on, destructured, or one of its other methods, which take SQL or a path, called).
fn store_breaches(text: &str) -> Vec<String> {
    use proc_macro2::TokenTree as T;
    fn walk(stream: proc_macro2::TokenStream, out: &mut Vec<String>) {
        let trees: Vec<T> = stream.into_iter().collect();
        let punct = |k: usize, c: char| matches!(trees.get(k), Some(T::Punct(p)) if p.as_char() == c);
        for (i, t) in trees.iter().enumerate() {
            match t {
                T::Ident(id) if id.to_string().trim_start_matches("r#") == "store" => {
                    let in_path = i >= 2 && punct(i - 1, ':') && punct(i - 2, ':');
                    let read = i >= 1 && punct(i - 1, '.') && punct(i + 1, '.') && matches!(trees.get(i + 2), Some(T::Ident(m)) if STORE_READERS.contains(&m.to_string().as_str()));
                    if !in_path && !read {
                        let next = match trees.get(i + 2) {
                            Some(T::Ident(m)) if punct(i + 1, '.') => format!(".{m}"),
                            _ => String::new(),
                        };
                        out.push(format!("store{next}: the session's store reached past its readers ({})", STORE_READERS.join(", ")));
                    }
                }
                T::Group(g) => walk(g.stream(), out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(text.parse().unwrap_or_else(|e| panic!("a source that does not lex: {e:?}")), &mut out);
    out
}

#[test]
fn its_sources_reach_the_plant_through_portal_alone_and_arm_the_switch_in_one_place() {
    let mut armings = Vec::new();
    for f in files_under(&crate_dir().join("src")) {
        let text = std::fs::read_to_string(&f).unwrap();
        let mut breaches = breaches_of(&text);
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let arming = |b: &String| b.starts_with("set_writes_armed:");
        armings.extend(breaches.iter().filter(|b| arming(b)).map(|_| name.clone()));
        if name == ARMING_FILE {
            breaches.retain(|b| !arming(b));
        }
        assert_eq!(breaches, Vec::<String>::new(), "{}", f.display());
        assert_eq!(portal_breaches(&text), Vec::<String>::new(), "{}", f.display());
        assert_eq!(store_breaches(&text), Vec::<String>::new(), "{}", f.display());
        let set_armed = read(&text).idents.iter().filter(|i| *i == "set_armed").count();
        let allowed = SET_ARMED.iter().find(|(n, _)| *n == name).map_or(0, |(_, k)| *k);
        assert_eq!(set_armed, allowed, "{}: the plant's arming named {set_armed} times, where {allowed}", f.display());
        let handle = read(&text).idents.iter().filter(|i| *i == "arming").count();
        let allowed = ARMING_HANDLE.iter().find(|(n, _)| *n == name).map_or(0, |(_, k)| *k);
        assert_eq!(handle, allowed, "{}: the arming handed to the plant named {handle} times, where {allowed}", f.display());
        assert!(!fs_outside(&f, &text), "{} reaches files of its own: `fs` is the engine's own files', in {FS_FILES:?} alone", f.display());
    }
    assert_eq!(armings, [ARMING_FILE], "the switch is armed once, in {ARMING_FILE}");
}

/// The files that read and write the engine's own files (a test and its function files, the runs
/// folder): `fs`, and a path's own reach to the disk, are named there alone, so no other file
/// reads, lists or looks at a plant's file past the session.
const FS_FILES: [&str; 3] = ["check.rs", "main.rs", "record.rs"];

/// The names that reach the disk: `fs` (`std::fs`, `tokio::fs`) and a path's methods that list a
/// folder or look at a file without it.
const DISK: &[&str] = &["fs", "read_dir", "metadata", "symlink_metadata", "exists", "try_exists", "is_file", "is_dir", "is_symlink", "canonicalize", "read_link"];

/// Whether a source file names one of [`DISK`] and is not one of `FS_FILES`.
fn fs_outside(file: &Path, text: &str) -> bool {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    !FS_FILES.contains(&name) && read(text).idents.iter().any(|i| DISK.contains(&i.as_str()))
}

#[test]
fn files_are_reached_where_the_engine_keeps_its_own() {
    let at = |name: &str, text: &str| fs_outside(Path::new("src").join(name).as_path(), text);
    assert!(at("plant.rs", "fn f(p: &Path) -> String { std::fs::read_to_string(p).unwrap_or_default() }"));
    assert!(at("run.rs", "use std::fs;"));
    assert!(at("eval.rs", "async fn f(p: &Path) { let _ = tokio::fs::read(p).await; }"));
    assert!(at("plant.rs", "use std::{fs as files};"));
    assert!(!at("plant.rs", "// std::fs, in a comment\nfn f() {}"));
    assert!(!at("record.rs", "fn f(p: &Path) { std::fs::write(p, \"\").unwrap(); }"));
    // A folder listed, a file looked at, without `fs`: the same.
    assert!(at("plant.rs", "fn f(p: &Path) { let _ = p.read_dir(); }"));
    assert!(at("run.rs", "fn f(p: &Path) -> bool { p.exists() || p.metadata().is_ok() }"));
    assert!(at("eval.rs", "fn f(p: &Path) -> bool { p.is_file() }"));
    assert!(!at("main.rs", "fn f(p: &Path) -> bool { p.exists() }"));
    // Where `fs` is named today: those three files.
    let mut named: Vec<String> = files_under(&crate_dir().join("src"))
        .into_iter()
        .filter(|f| read(&std::fs::read_to_string(f).unwrap()).idents.iter().any(|i| DISK.contains(&i.as_str())))
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    named.sort();
    assert_eq!(named, FS_FILES);
}

/// Its normal dependencies, each reviewed: no OPC UA stack, no socket or HTTP crate of its own
/// (`sha2` a hash over bytes held in memory, the fingerprint of a run's confirmed writes).
const DEPENDENCIES: &[&str] = &["chrono", "serde", "serde_json", "sha2", "thiserror", "tokio", "toml", "veredas-core", "veredas-portal"];

#[test]
fn its_normal_dependencies_are_the_reviewed_ones() {
    let dir = crate_dir();
    let pkg = package(&metadata(&dir), &dir);
    for d in pkg["dependencies"].as_array().unwrap() {
        if d["kind"].is_null() || d["kind"] == "build" {
            let name = d["name"].as_str().unwrap();
            assert!(DEPENDENCIES.contains(&name), "`{name}` is not among the reviewed dependencies");
        }
    }
}

#[test]
fn the_reading_finds_every_refused_form() {
    let one = |t: &str| breaches_of(t);
    assert_eq!(one("fn f(s: &Session) { s.set_writes_armed(true); }"), ["set_writes_armed: arms portal's switch: once, in `main.rs`, the session's arming handed to the run alone"]);
    assert_eq!(one("fn f(s: &Session) { s.r#set_writes_armed(true); }"), ["set_writes_armed: arms portal's switch: once, in `main.rs`, the session's arming handed to the run alone"]);
    assert_eq!(one("fn f() { let g = WriteGate::new(true); }"), ["WriteGate: a gate of its own"]);
    assert_eq!(one("macro_rules! m { () => { x.set_writes_armed(true) } }"), ["set_writes_armed: arms portal's switch: once, in `main.rs`, the session's arming handed to the run alone"]);
    // A gate made by `Default` is only of use to a worker, and no worker is spawned.
    assert_eq!(
        one("fn f() { let g = Default::default(); let w = veredas_core::opc::spawn(s, t, g); }"),
        ["opc: core's or portal's OPC UA layer, past the session", "spawn: a worker of its own: every one that writes takes a gate"]
    );
    assert_eq!(one("use veredas_core::telegram::Link;"), ["telegram: core's telegram layer, past the session"]);
    assert_eq!(one("use veredas_core::fieldbus;"), ["fieldbus: core's fieldbus masters, past the session"]);
    assert_eq!(one("use std::net::TcpStream;"), ["net: a socket of its own"]);
    assert_eq!(one("use veredas_portal::tail::single;"), ["tail: portal's log layer, past the session"]);
    assert_eq!(
        one("fn f(p: &Path) { let s = veredas_portal::FileTailSource::new(\"plant\", \"plant\", p, Default::default()); }"),
        ["FileTailSource: a log read past the session"]
    );
    assert_eq!(one("use veredas_portal::FamilyTailSource as F;"), ["FamilyTailSource: a family of logs read past the session"]);
    assert_eq!(one("fn f() { let s = veredas_portal::SourceSet::default(); }"), ["SourceSet: portal's sources started past the session"]);
    assert_eq!(one("fn f(s: Box<dyn veredas_portal::Source>) {}"), ["Source: a source started past the session"]);
    assert_eq!(one("fn f() { std::process::Command::new(\"x\"); }"), ["Command: a process of its own"]);
    assert_eq!(one("include!(\"elsewhere.rs\");"), ["include: tokens from a file the reading does not see"]);
    assert_eq!(one("const T: &str = include_str!(\"x\");"), ["include_str: text from a file the reading does not see"]);
    assert_eq!(one("const B: &[u8] = include_bytes!(\"x\");"), ["include_bytes: bytes from a file the reading does not see"]);
    // Every refused name is asserted above, by its own text: none can leave the list unseen.
    let asserted = [
        "set_writes_armed",
        "WriteGate",
        "spawn",
        "opc",
        "telegram",
        "fieldbus",
        "net",
        "tail",
        "FileTailSource",
        "FamilyTailSource",
        "SourceSet",
        "Source",
        "Command",
        "unsafe",
        "from_v1",
        "include",
        "include_str",
        "include_bytes",
    ];
    assert_eq!(REFUSED.iter().map(|(n, _)| *n).collect::<Vec<_>>(), asserted);
    assert_eq!(one("#[path = \"gate.txt\"] mod gate;"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(one("#[cfg_attr(unix, path = \"g.rs\")] mod gate;"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(one("mod m { #![path = \"g\"] }"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(
        one("const P: &str = \"[session]\\nwrites_armed = true\";"),
        ["\"[session]\\nwrites_armed = true\": a project's text that arms the switch"]
    );
    // The switch's position set by a field, a struct literal, an assignment: anything but a read.
    assert_eq!(one("struct S { writes_armed: bool }"), ["writes_armed: the switch's position set, not read"]);
    assert_eq!(one("fn f() { let s = S { store: 1, writes_armed: true }; }"), ["writes_armed: the switch's position set, not read"]);
    assert_eq!(one("fn f(s: &mut S) { s.writes_armed = true; }"), ["writes_armed: the switch's position set, not read"]);
    assert_eq!(one("fn f(l: &Live) -> bool { l.writes_armed(true) }"), ["writes_armed: the switch's position set, not read"]);
    // Veredas 1's project read in, and its spellings of the switch in a literal.
    assert_eq!(one("fn f(d: &Path) { veredas_portal::project::from_v1(d); }"), ["from_v1: Veredas 1's project read in, its own spelling of the switch with it"]);
    assert_eq!(one("const C: &str = r#\"{ \"allow_writes\": true }\"#;"), ["r#\"{ \"allow_writes\": true }\"#: a project's text that arms the switch"]);
    assert_eq!(one("const C: &str = \"allow_dashboard_writes\";"), ["\"allow_dashboard_writes\": a project's text that arms the switch"]);
    // Code past the compiler's guarantees.
    assert_eq!(one("unsafe extern \"C\" { fn socket(a: i32) -> i32; }"), ["unsafe: code past the compiler's guarantees: a socket by the system's own calls, among others"]);
    assert_eq!(one("fn f() { unsafe { g() } }"), ["unsafe: code past the compiler's guarantees: a socket by the system's own calls, among others"]);
    // A comment is not code, a doc comment included; a path outside an attribute is a path;
    // reading the switch is not arming it; the protocol layers' names as parts of other names are
    // other names.
    assert_eq!(one("// set_writes_armed spawn opc\nfn f() {}"), Vec::<String>::new());
    assert_eq!(one("/// reads `Live::writes_armed`, never `writes_armed = true`\n//! allow_writes\nfn f() {}"), Vec::<String>::new());
    assert_eq!(one("fn f(path: &std::path::Path) {}"), Vec::<String>::new());
    assert_eq!(one("fn f(l: &Live) -> bool { l.writes_armed() && l.opc_handle(\"x\").is_some() }"), Vec::<String>::new());
}

#[test]
fn portal_is_reached_through_what_the_engine_takes_from_it_alone() {
    let one = |t: &str| portal_breaches(t);
    let past = |p: &str| vec![format!("{p}: portal past what the engine takes from it")];
    // Portal's search of logs at rest, its CSV previews, its families: refused, however written.
    assert_eq!(one("fn f(p: PathBuf) { veredas_portal::grep::search(veredas_portal::grep::GrepScope::Files(vec![p]), q, d); }"), [past("veredas_portal::grep::search"), past("veredas_portal::grep::GrepScope::Files")].concat());
    assert_eq!(one("fn f() { let r = veredas_portal::store::preview_csv(\"x\", 10, &Default::default()); }"), past("veredas_portal::store::preview_csv"));
    assert_eq!(one("use veredas_portal::store::{RowsQuery, sniff_csv_options};"), past("veredas_portal::store::sniff_csv_options"));
    assert_eq!(one("use veredas_portal::{grep::{self, GrepQuery}, SeriesType};"), [past("veredas_portal::grep"), past("veredas_portal::grep::GrepQuery")].concat());
    assert_eq!(one("use veredas_portal::tail::FamilySpec as F;"), past("veredas_portal::tail::FamilySpec"));
    // Portal whole, a glob, a module imported or renamed: every later path past the list.
    assert_eq!(one("use veredas_portal as p;"), past("veredas_portal"));
    assert_eq!(one("extern crate veredas_portal;"), past("veredas_portal"));
    assert_eq!(one("use veredas_portal::*;"), past("veredas_portal::*"));
    assert_eq!(one("use veredas_portal::store::*;"), past("veredas_portal::store::*"));
    assert_eq!(one("use veredas_portal::store;"), past("veredas_portal::store"));
    assert_eq!(one("use veredas_portal::store::{self};"), past("veredas_portal::store"));
    assert_eq!(one("use r#veredas_portal::r#grep::search;"), past("veredas_portal::grep::search"));
    assert_eq!(one("macro_rules! m { () => { veredas_portal::grep::search } }"), past("veredas_portal::grep::search"));
    // What the engine takes: an item, a method of an allowed type, a renamed item.
    assert_eq!(one("use veredas_portal::session::{HealthState, Session};\nuse veredas_portal::store::{RowsQuery, TableKind};"), Vec::<String>::new());
    assert_eq!(one("fn f() { let s = veredas_portal::session::Session::start(c); let t = veredas_portal::serve::store_now_ms(); }"), Vec::<String>::new());
    assert_eq!(one("use veredas_portal::session::Live as L;\n// veredas_portal::grep, in a comment\nconst S: &str = \"veredas_portal::grep\";"), Vec::<String>::new());
    // Every item of the list is one the sources take today: none stands there unused.
    let mut used = Vec::new();
    for f in files_under(&crate_dir().join("src")) {
        let mut paths = Vec::new();
        portal_paths(std::fs::read_to_string(&f).unwrap().parse().unwrap(), &mut paths);
        used.extend(PORTAL.iter().filter(|a| paths.iter().any(|p| p == *a || p.starts_with(&format!("{a}::")))).map(|a| a.to_string()));
    }
    used.sort();
    used.dedup();
    let mut listed: Vec<String> = PORTAL.iter().map(|a| a.to_string()).collect();
    listed.sort();
    assert_eq!(used, listed);
}

#[test]
fn the_session_s_store_is_reached_by_its_readers_alone() {
    let one = |t: &str| store_breaches(t);
    let past = |m: &str| vec![format!("store{m}: the session's store reached past its readers (query_rows, list_tables)")];
    // Its readers, and portal's module in a path: passed.
    assert_eq!(one("fn f(l: &Live) { l.store.query_rows(t, q); l.store.list_tables(); }"), Vec::<String>::new());
    assert_eq!(one("use veredas_portal::store::{RowsQuery, TableKind};\nconst P: usize = veredas_portal::store::MAX_PAGE;"), Vec::<String>::new());
    // Its other methods, which take SQL or a path, the store bound, handed on, destructured.
    assert_eq!(one("fn f(l: &Live) { l.store.validate_definition(\"SELECT * FROM read_csv('x')\"); }"), past(".validate_definition"));
    assert_eq!(one("fn f(l: &Live, p: &Path) { l.store.save_to(p); }"), past(".save_to"));
    assert_eq!(one("fn f(l: &Live) { let s = &l.store; s.export_all_csv(p); }"), past(""));
    assert_eq!(one("fn f(l: &Live) { g(&l.store); }"), past(""));
    assert_eq!(one("fn f(l: &Live) { let Live { store, .. } = l; }"), past(""));
    assert_eq!(one("fn f(l: &Live) { l.r#store.checkpoint(); }"), past(".checkpoint"));
    // A comment and a literal are not code.
    assert_eq!(one("// l.store.save_to(p)\nconst S: &str = \"the store\";"), Vec::<String>::new());
}

#[test]
fn the_plant_s_arming_is_named_where_the_run_arms_and_disarms_alone() {
    // A call of the plant's arming in any other file, or a third in `run.rs`, is counted.
    let count = |t: &str| read(t).idents.iter().filter(|i| *i == "set_armed").count();
    assert_eq!(count("fn keep(plant: &dyn Plant) { plant.set_armed(true); }"), 1);
    assert_eq!(count("fn f(p: &dyn Plant) { p.r#set_armed(true) }"), 1);
    assert_eq!(count("// p.set_armed(true)\nfn f() {}"), 0);
    let at = |name: &str| SET_ARMED.iter().find(|(n, _)| *n == name).map_or(0, |(_, k)| *k);
    assert_eq!((at("record.rs"), at("eval.rs"), at("main.rs"), at("plant.rs"), at("run.rs")), (0, 0, 0, 2, 2));
    // The arming handed to the plant called past `set_armed` (refute round 7): a method of the
    // plant reaching its field, the executable calling its closure, each one more of its name.
    let handle = |t: &str| read(t).idents.iter().filter(|i| *i == "arming").count();
    assert_eq!(handle("fn now_ms(&self) -> i64 { if let Some(arm) = &self.arming { arm(true); } 0 }"), 1);
    assert_eq!(handle("fn f(arming: Box<dyn Fn(bool)>) { arming(true); }"), 2);
    assert_eq!(handle("fn f(p: &LivePlant) { (p.r#arming.as_ref().unwrap())(true) }"), 1);
    assert_eq!(handle("// self.arming\nfn with_arming() {}"), 0);
    let at = |name: &str| ARMING_HANDLE.iter().find(|(n, _)| *n == name).map_or(0, |(_, k)| *k);
    assert_eq!((at("record.rs"), at("run.rs"), at("main.rs"), at("plant.rs")), (0, 0, 2, 7));
}
