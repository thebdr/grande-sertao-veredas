//! Comissionador's own rules that no suite law checks (contract: C-200, the component clause;
//! the laws in `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library first, one executable on top (`docs/comissionador-plan.md`, "Rules"): cargo is asked
//! what it would build (`cargo metadata --no-deps`, which parses and builds nothing), and the
//! targets are exactly one library, one binary named `veredas-comissionador`, and tests; no build
//! script. Its members among its dependencies are core and portal at most (L5's row). And it never
//! arms portal's write switch (`docs/questions.md` Q314): its sources, read as the compiler's
//! tokens, name no call that sets the switch, and pull in no file from elsewhere that could.

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
/// (`#[...]`, `#![...]`, `cfg_attr` within them), and the literals' source text.
#[derive(Default, Debug)]
struct Reading {
    idents: Vec<String>,
    in_attributes: Vec<String>,
    literals: Vec<String>,
}

fn read(text: &str) -> Reading {
    fn walk(stream: proc_macro2::TokenStream, attr: bool, out: &mut Reading) {
        let trees: Vec<proc_macro2::TokenTree> = stream.into_iter().collect();
        for (i, tree) in trees.iter().enumerate() {
            match tree {
                proc_macro2::TokenTree::Ident(id) => {
                    let s = id.to_string();
                    let s = s.strip_prefix("r#").map(str::to_string).unwrap_or(s);
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
                    walk(g.stream(), attr || is_attr, out);
                }
                proc_macro2::TokenTree::Literal(l) => out.literals.push(l.to_string()),
                proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
    let stream: proc_macro2::TokenStream = text.parse().unwrap_or_else(|e| panic!("a source that does not lex: {e:?}"));
    let mut out = Reading::default();
    walk(stream, false, &mut out);
    out
}

/// The names a source of comissionador never holds, each with why.
const REFUSED: &[(&str, &str)] = &[
    ("set_writes_armed", "arms portal's switch"),
    ("WriteGate", "a gate of its own"),
    ("spawn", "a worker of its own: every one that writes takes a gate"),
    ("opc", "core's or portal's OPC UA layer, past the session"),
    ("telegram", "core's telegram layer, past the session"),
    ("fieldbus", "core's fieldbus masters, past the session"),
    ("net", "a socket of its own"),
    ("Command", "a process of its own"),
    ("include", "tokens from a file the reading does not see"),
    ("include_str", "text from a file the reading does not see"),
    ("include_bytes", "bytes from a file the reading does not see"),
];

/// What a source holds that the rule refuses, each as `name: why`.
fn breaches_of(text: &str) -> Vec<String> {
    let r = read(text);
    let mut out = Vec::new();
    for id in &r.idents {
        if let Some((_, why)) = REFUSED.iter().find(|(n, _)| n == id) {
            out.push(format!("{id}: {why}"));
        }
    }
    if r.in_attributes.iter().any(|i| i == "path") {
        out.push("path: a module read from a file of an attribute's choosing".to_string());
    }
    for l in &r.literals {
        if l.contains("writes_armed") {
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

#[test]
fn its_sources_reach_the_plant_through_portal_alone_and_never_arm_the_switch() {
    for f in files_under(&crate_dir().join("src")) {
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(breaches_of(&text), Vec::<String>::new(), "{}", f.display());
    }
}

/// Its normal dependencies, each reviewed: no OPC UA stack, no socket or HTTP crate of its own.
const DEPENDENCIES: &[&str] = &["chrono", "serde", "serde_json", "thiserror", "tokio", "toml", "veredas-core", "veredas-portal"];

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
    assert_eq!(one("fn f(s: &Session) { s.set_writes_armed(true); }"), ["set_writes_armed: arms portal's switch"]);
    assert_eq!(one("fn f(s: &Session) { s.r#set_writes_armed(true); }"), ["set_writes_armed: arms portal's switch"]);
    assert_eq!(one("fn f() { let g = WriteGate::new(true); }"), ["WriteGate: a gate of its own"]);
    assert_eq!(one("macro_rules! m { () => { x.set_writes_armed(true) } }"), ["set_writes_armed: arms portal's switch"]);
    // A gate made by `Default` is only of use to a worker, and no worker is spawned.
    assert_eq!(
        one("fn f() { let g = Default::default(); let w = veredas_core::opc::spawn(s, t, g); }"),
        ["opc: core's or portal's OPC UA layer, past the session", "spawn: a worker of its own: every one that writes takes a gate"]
    );
    assert_eq!(one("use veredas_core::telegram::Link;"), ["telegram: core's telegram layer, past the session"]);
    assert_eq!(one("use veredas_core::fieldbus;"), ["fieldbus: core's fieldbus masters, past the session"]);
    assert_eq!(one("use std::net::TcpStream;"), ["net: a socket of its own"]);
    assert_eq!(one("fn f() { std::process::Command::new(\"x\"); }"), ["Command: a process of its own"]);
    assert_eq!(one("include!(\"elsewhere.rs\");"), ["include: tokens from a file the reading does not see"]);
    assert_eq!(one("const T: &str = include_str!(\"x\");"), ["include_str: text from a file the reading does not see"]);
    assert_eq!(one("#[path = \"gate.txt\"] mod gate;"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(one("#[cfg_attr(unix, path = \"g.rs\")] mod gate;"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(one("mod m { #![path = \"g\"] }"), ["path: a module read from a file of an attribute's choosing"]);
    assert_eq!(
        one("const P: &str = \"[session]\\nwrites_armed = true\";"),
        ["\"[session]\\nwrites_armed = true\": a project's text that arms the switch"]
    );
    // A comment is not code; a path outside an attribute is a path; reading the switch is not
    // arming it; the protocol layers' names as parts of other names are other names.
    assert_eq!(one("// set_writes_armed spawn opc\nfn f() {}"), Vec::<String>::new());
    assert_eq!(one("fn f(path: &std::path::Path) {}"), Vec::<String>::new());
    assert_eq!(one("fn f(l: &Live) -> bool { l.writes_armed() && l.opc_handle(\"x\").is_some() }"), Vec::<String>::new());
}
