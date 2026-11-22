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

// ---- the switch is never armed --------------------------------------------------------------

/// The identifiers of a Rust file as the compiler reads them: a raw identifier read as its name,
/// comments gone, macro bodies and groups walked.
fn identifiers(text: &str) -> Vec<String> {
    fn walk(stream: proc_macro2::TokenStream, out: &mut Vec<String>) {
        for tree in stream {
            match tree {
                proc_macro2::TokenTree::Ident(id) => {
                    let s = id.to_string();
                    out.push(s.strip_prefix("r#").map(str::to_string).unwrap_or(s));
                }
                proc_macro2::TokenTree::Group(g) => walk(g.stream(), out),
                _ => {}
            }
        }
    }
    let stream: proc_macro2::TokenStream = text.parse().unwrap_or_else(|e| panic!("a source that does not lex: {e:?}"));
    let mut out = Vec::new();
    walk(stream, &mut out);
    out
}

/// The names that would arm portal's switch or reach past what this reading sees: setting the
/// switch, a gate of its own, and a macro that pulls in another file's tokens.
const ARMING: &[&str] = &["set_writes_armed", "WriteGate", "include", "include_str", "include_bytes"];

fn arming(text: &str) -> Vec<String> {
    identifiers(text).into_iter().filter(|i| ARMING.contains(&i.as_str())).collect()
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(rust_files(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn its_sources_never_arm_portal_s_write_switch() {
    let files = rust_files(&crate_dir().join("src"));
    assert!(files.len() >= 6, "the crate's sources: {files:?}");
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        assert_eq!(arming(&text), Vec::<String>::new(), "{}", f.display());
    }
}

#[test]
fn the_reading_finds_an_arming_call_however_it_is_spelled() {
    assert_eq!(arming("fn f(s: &Session) { s.set_writes_armed(true); }"), ["set_writes_armed"]);
    assert_eq!(arming("fn f(s: &Session) { s.r#set_writes_armed(true); }"), ["set_writes_armed"]);
    assert_eq!(arming("fn f() { let g = WriteGate::new(true); }"), ["WriteGate"]);
    assert_eq!(arming("macro_rules! m { () => { x.set_writes_armed(true) } }"), ["set_writes_armed"]);
    assert_eq!(arming("include!(\"elsewhere.rs\");"), ["include"]);
    assert_eq!(arming("const T: &str = include_str!(\"x\");"), ["include_str"]);
    // A comment and a string are not code.
    assert_eq!(arming("// set_writes_armed\nconst S: &str = \"set_writes_armed\";"), Vec::<String>::new());
    // Reading the switch's position is not arming it.
    assert_eq!(arming("fn f(l: &Live) -> bool { l.writes_armed() }"), Vec::<String>::new());
}
