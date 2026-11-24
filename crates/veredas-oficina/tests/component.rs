//! Oficina, the component (C-250): what cargo builds of it, with and without its window; that no
//! member's default build needs Qt; that its members are core alone, without core's plant-facing
//! layers; that the tests' driver is compiled in only on asking; and the window itself, opened and
//! read through the driver.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::Shell;
use serde_json::Value;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_manifest() -> PathBuf {
    crate_dir().join("../../Cargo.toml")
}

fn cargo(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO")).args(args).output().expect("cargo runs");
    assert!(out.status.success(), "cargo {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("UTF-8")
}

fn package() -> Value {
    let text = cargo(&["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path", workspace_manifest().to_str().expect("a UTF-8 path")]);
    let meta: Value = serde_json::from_str(&text).expect("cargo's JSON");
    let manifest = crate_dir().join("Cargo.toml").canonicalize().expect("the manifest");
    meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().expect("a path")).canonicalize().expect("a manifest") == manifest)
        .expect("veredas-oficina among the workspace's packages")
        .clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().expect("an array").iter().map(|s| s.as_str().expect("a string").to_string()).collect()
}

/// The packages `cargo tree` lists for `args`, by name, once each.
fn tree(args: &[&str]) -> BTreeSet<String> {
    let manifest = workspace_manifest();
    let mut all = vec!["tree", "--offline", "--locked", "--prefix", "none", "--format", "{p}", "--manifest-path", manifest.to_str().expect("a UTF-8 path")];
    all.extend_from_slice(args);
    cargo(&all).lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

const QT: [&str; 4] = ["cxx-qt", "cxx-qt-lib", "cxx-qt-build", "cxx"];

#[test]
fn oficina_is_a_library_and_one_executable_its_window_behind_the_shell_feature() {
    let p = package();
    assert_eq!(p["name"], "veredas-oficina");
    assert_eq!(p["version"], "2.0.0-alpha.1");
    let mut targets: Vec<(String, String, Vec<String>)> = p["targets"]
        .as_array()
        .expect("targets")
        .iter()
        .map(|t| {
            let kinds = strings(&t["kind"]).join(",");
            let features = t.get("required-features").map(strings).unwrap_or_default();
            (kinds, t["name"].as_str().expect("a name").to_string(), features)
        })
        .collect();
    targets.sort();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            ("bin".to_string(), "veredas-oficina".to_string(), s(&["shell"])),
            ("custom-build".to_string(), "build-script-build".to_string(), vec![]),
            ("lib".to_string(), "veredas_oficina".to_string(), vec![]),
            ("test".to_string(), "component".to_string(), s(&["driver"])),
            ("test".to_string(), "tree".to_string(), s(&["driver"])),
            ("test".to_string(), "tree_model".to_string(), vec![]),
        ]
    );
    let features = p["features"].as_object().expect("features");
    let feature = |name: &str| strings(&features[name]);
    assert_eq!(features.len(), 3, "{features:?}");
    assert_eq!(feature("default"), Vec::<String>::new(), "nothing on by default");
    assert_eq!(feature("shell"), s(&["dep:cxx", "dep:cxx-qt", "dep:cxx-qt-lib", "dep:cxx-qt-build"]));
    assert_eq!(feature("driver"), s(&["shell"]));
}

#[test]
fn no_member_s_default_build_needs_qt_and_the_shell_brings_it() {
    let everyone = tree(&["--workspace", "-e", "normal,build"]);
    for qt in QT {
        assert!(!everyone.contains(qt), "{qt} in the suite's default build");
    }
    let alone = tree(&["-p", "veredas-oficina", "-e", "normal,build"]);
    assert!(alone.contains("veredas-core"), "{alone:?}");
    for qt in QT {
        assert!(!alone.contains(qt), "{qt} in Oficina's default build");
    }
    let window = tree(&["-p", "veredas-oficina", "--features", "shell", "-e", "normal,build"]);
    for qt in QT {
        assert!(window.contains(qt), "{qt} missing from the shell's build: {window:?}");
    }
}

#[test]
fn its_members_are_core_alone() {
    let p = package();
    let members: BTreeSet<String> = p["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter(|d| d["path"].is_string() && d["kind"].as_str() != Some("dev"))
        .map(|d| d["name"].as_str().expect("a name").to_string())
        .collect();
    assert_eq!(members, BTreeSet::from(["veredas-core".to_string()]));
    let names: BTreeSet<String> = p["dependencies"].as_array().expect("dependencies").iter().map(|d| d["name"].as_str().expect("a name").to_string()).collect();
    assert!(!names.iter().any(|n| n.starts_with("veredas-") && n != "veredas-core"), "{names:?}");
}

/// Core without its default features (C-073): its header reader, and no crate of a plant's stack in
/// Oficina's own graph, the window's included.
#[test]
fn its_own_graph_holds_no_plant_stack() {
    const PLANT: [&str; 7] = ["tokio", "async-opcua-client", "async-opcua-types", "socket2", "regex", "chrono", "duckdb"];
    for features in [&[][..], &["--features", "driver"][..]] {
        let mut args = vec!["-p", "veredas-oficina", "-e", "normal,build"];
        args.extend_from_slice(features);
        let graph = tree(&args);
        assert!(graph.contains("veredas-core"), "{graph:?}");
        for crate_ in PLANT {
            assert!(!graph.contains(crate_), "{crate_} in Oficina's graph with {features:?}: {graph:?}");
        }
    }
}

/// What ships carries no test server: the build script defines `OFICINA_DRIVER` and links Spix and
/// AnyRPC only when cargo turns `driver` on, and the C++ side names Spix only under that
/// definition. A reading of the two files, not of a built binary.
#[test]
fn the_tests_driver_is_compiled_in_only_on_asking() {
    let build = std::fs::read_to_string(crate_dir().join("build.rs")).expect("build.rs");
    assert!(build.contains("let driver = std::env::var_os(\"CARGO_FEATURE_DRIVER\").is_some();"));
    assert_eq!(build.matches("OFICINA_DRIVER").count(), 1, "defined in one place");
    // Inside the block `if driver {` opens, before a brace closes it.
    let under_driver = |at: usize| build[..at].rfind("if driver {").is_some_and(|open| !build[open..at].contains('}'));
    let define = build.find("cc.define(\"OFICINA_DRIVER\"").expect("the definition");
    assert!(under_driver(define), "defined only under `if driver`");
    let link = build.find("[\"SpixQtQuick\", \"SpixCore\", \"anyrpc\"]").expect("Spix linked");
    assert!(under_driver(link), "linked only under `if driver`");
    assert_eq!(build.matches("SpixQtQuick").count(), 1, "linked in one place");

    let cpp = std::fs::read_to_string(crate_dir().join("cpp/shell.cpp")).expect("shell.cpp");
    let mut depth = 0;
    let mut guarded = false;
    for line in cpp.lines() {
        let t = line.trim_start();
        if t.starts_with("#if") {
            depth += 1;
            guarded = guarded || t == "#ifdef OFICINA_DRIVER";
        } else if t.starts_with("#endif") {
            depth -= 1;
            if depth == 0 {
                guarded = false;
            }
        } else if t.contains("spix") || t.contains("Spix") || t.contains("OFICINA_SPIX_PORT") {
            assert!(guarded, "Spix named outside `#ifdef OFICINA_DRIVER`: {line}");
        }
    }
}

#[test]
fn the_window_opens_as_oficina_with_the_project_pane_docked() {
    let shell = Shell::start(&[], &[]);
    assert_eq!(shell.property("oficina", "title"), "Oficina");
    assert_eq!(shell.property("oficina", "visible"), "true");
    assert!(shell.visible("oficina/projectPane"), "the project pane is drawn");
    // No folder named: the tree is empty and says nothing went wrong.
    assert_eq!(shell.property("oficina/projectError", "text"), "");
    assert_eq!(shell.rows(), Vec::<String>::new());
    assert_eq!(shell.errors(), Vec::<String>::new());
}
