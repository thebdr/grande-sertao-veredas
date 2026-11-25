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

/// The packages `cargo tree` lists for `args` on every target cargo knows (Windows' dependencies
/// included, whichever host runs the test), by name, once each.
fn tree(args: &[&str]) -> BTreeSet<String> {
    let manifest = workspace_manifest();
    let mut all = vec!["tree", "--offline", "--locked", "--target", "all", "--prefix", "none", "--format", "{p}", "--manifest-path", manifest.to_str().expect("a UTF-8 path")];
    all.extend_from_slice(args);
    cargo(&all).lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

const QT: [&str; 4] = ["cxx-qt", "cxx-qt-lib", "cxx-qt-build", "cxx"];

#[test]
fn oficina_is_a_library_and_one_executable_its_window_behind_the_shell_feature() {
    let p = package();
    assert_eq!(p["name"], "veredas-oficina");
    assert_eq!(p["version"], "2.0.0-alpha.1");
    // No licence stated while Q35 is open.
    assert!(p["license"].is_null(), "a licence stated: {}", p["license"]);
    assert!(p["license_file"].is_null(), "a licence file named: {}", p["license_file"]);
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
    // The suite's default build and its test build (a test's dev-dependencies built with it).
    let everyone = tree(&["--workspace", "-e", "normal,build,dev"]);
    for qt in QT {
        assert!(!everyone.contains(qt), "{qt} in the suite's default build or its tests'");
    }
    let alone = tree(&["-p", "veredas-oficina", "-e", "normal,build,dev"]);
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

/// The model and its tests compiled with no feature, and with `QMAKE` naming a qmake that is not
/// there: had any crate of Qt's bridge come into the default build, its build script would fail.
#[test]
fn the_model_and_its_tests_build_with_no_qt() {
    let manifest = workspace_manifest();
    let out = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--locked", "-p", "veredas-oficina", "--lib", "--tests", "--manifest-path", manifest.to_str().expect("a UTF-8 path")])
        .env("QMAKE", crate_dir().join("no-such-qmake"))
        .output()
        .expect("cargo runs");
    assert!(out.status.success(), "the model's build with no Qt: {}", String::from_utf8_lossy(&out.stderr));
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

/// Where a name is assigned in Rust text: the name as a word, followed by `=`, a compound `|=`,
/// `&=` or `^=`, or a type's `:` (not `==` or `::`).
fn assignments(text: &str, name: &str) -> usize {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(name)
        .filter(|(i, _)| {
            let after = &text[i + name.len()..];
            if word(text[..*i].chars().next_back()) || word(after.chars().next()) {
                return false;
            }
            let rest = after.trim_start();
            (rest.starts_with('=') && !rest.starts_with("=="))
                || ["|=", "&=", "^="].iter().any(|op| rest.starts_with(op))
                || (rest.starts_with(':') && !rest.starts_with("::"))
        })
        .count()
}

/// The lines of C++ text that name Spix outside a branch compiled only with `OFICINA_DRIVER`. Each
/// `#if`, `#ifdef` and `#ifndef` opens a frame, `#else` and `#elif` move it to its next branch,
/// `#endif` closes it; a line is in the driver's branch when a frame on the stack is in
/// `#ifdef OFICINA_DRIVER`'s or `#if defined(OFICINA_DRIVER)`'s first branch, or in
/// `#ifndef OFICINA_DRIVER`'s `#else`. An `#elif` branch is never the driver's.
fn spix_outside_the_driver(cpp: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Cond {
        Driver,
        NotDriver,
        Other,
    }
    let mut frames: Vec<(Cond, bool)> = Vec::new();
    let mut found = Vec::new();
    for line in cpp.lines() {
        let t = line.trim();
        if let Some(d) = t.strip_prefix('#') {
            let d = d.split_whitespace().collect::<Vec<_>>().join(" ");
            let first = d.split([' ', '(']).next().unwrap_or("");
            match first {
                "ifdef" | "ifndef" | "if" => {
                    let squeezed: String = d.chars().filter(|c| !c.is_whitespace()).collect();
                    let cond = match squeezed.as_str() {
                        "ifdefOFICINA_DRIVER" | "ifdefined(OFICINA_DRIVER)" => Cond::Driver,
                        "ifndefOFICINA_DRIVER" | "if!defined(OFICINA_DRIVER)" => Cond::NotDriver,
                        _ => Cond::Other,
                    };
                    frames.push((cond, true));
                    continue;
                }
                "else" => {
                    if let Some(f) = frames.last_mut() {
                        f.1 = false;
                    }
                    continue;
                }
                "elif" => {
                    if let Some(f) = frames.last_mut() {
                        *f = (Cond::Other, false);
                    }
                    continue;
                }
                "endif" => {
                    frames.pop();
                    continue;
                }
                _ => {}
            }
        }
        let in_driver = frames.iter().any(|&(c, first)| (c == Cond::Driver && first) || (c == Cond::NotDriver && !first));
        if line.to_ascii_lowercase().contains("spix") && !in_driver {
            found.push(line.to_string());
        }
    }
    found
}

#[test]
fn the_reading_of_the_cpp_finds_spix_outside_the_driver_however_it_is_branched() {
    let found = |cpp: &str| spix_outside_the_driver(cpp).len();
    // Refute round 1's two: Spix under `#else`, and after a closed inner frame inside an outer one.
    assert_eq!(found("#ifdef OFICINA_DRIVER\n#include <Spix/QtQmlBot.h>\n#else\n#include <Spix/AnyRpcServer.h>\n#endif\n"), 1);
    assert_eq!(found("#if 1\n#ifdef OFICINA_DRIVER\nint a;\n#endif\n#include <Spix/QtQmlBot.h>\n#endif\n"), 1);
    assert_eq!(found("spix::QtQmlBot bot;\n"), 1);
    assert_eq!(found("#if defined(OFICINA_DRIVER)\nspix::x();\n#elif 1\nspix::y();\n#endif\n"), 1);
    assert_eq!(found("#ifdef OFICINA_DRIVER\n#else\n#endif\n// Spix\n"), 1);
    // The driver's own branches.
    assert_eq!(found("#ifdef OFICINA_DRIVER\n#if 1\nspix::x();\n#endif\n#endif\n"), 0);
    assert_eq!(found("#ifndef OFICINA_DRIVER\nint a;\n#else\nspix::x();\n#endif\n"), 0);
    assert_eq!(found("#  ifdef   OFICINA_DRIVER\nspix::x();\n#  endif\n"), 0);
}

#[test]
fn the_reading_of_the_build_script_finds_driver_rebound() {
    assert_eq!(assignments("let driver = a;", "driver"), 1);
    // Refute round 1's: `driver` bound again.
    assert_eq!(assignments("let driver = a;\nlet driver = driver || b;", "driver"), 2);
    assert_eq!(assignments("let mut driver = a;\ndriver |= b;", "driver"), 2);
    assert_eq!(assignments("let driver: bool = a;", "driver"), 1);
    assert_eq!(assignments("if driver == a {}\nlet not_driver = b;\nlet driverx = c;", "driver"), 0);
}

/// What ships carries no test server: the build script binds `driver` once, from cargo's word on
/// the `driver` feature, and defines `OFICINA_DRIVER` and links Spix and AnyRPC once each, inside a
/// block that `if driver` opens; the C++ side names Spix only in a branch compiled with
/// `OFICINA_DRIVER`. A reading of the two files, not of a built binary.
#[test]
fn the_tests_driver_is_compiled_in_only_on_asking() {
    let build = std::fs::read_to_string(crate_dir().join("build.rs")).expect("build.rs");
    assert_eq!(assignments(&build, "driver"), 1, "`driver` bound once");
    assert!(build.contains("let driver = std::env::var_os(\"CARGO_FEATURE_DRIVER\").is_some();"), "from cargo's word on the feature");
    // Inside the block `if driver {` opens, before a brace closes it.
    let under_driver = |at: usize| build[..at].rfind("if driver {").is_some_and(|open| !build[open..at].contains('}'));
    for name in ["OFICINA_DRIVER", "SpixQtQuick", "SpixCore", "anyrpc"] {
        assert_eq!(build.matches(name).count(), 1, "{name} once in build.rs");
        assert!(under_driver(build.find(name).expect("named")), "{name} only under `if driver`");
    }
    let cpp = std::fs::read_to_string(crate_dir().join("cpp/shell.cpp")).expect("shell.cpp");
    assert!(cpp.contains("#ifdef OFICINA_DRIVER"), "the driver's branch is there");
    assert_eq!(spix_outside_the_driver(&cpp), Vec::<String>::new());
}

#[test]
fn the_window_opens_as_oficina_with_the_project_pane_docked() {
    let shell = Shell::start(&[], &[]);
    assert_eq!(shell.property("oficina", "title"), "Oficina");
    assert_eq!(shell.property("oficina", "visible"), "true");
    assert!(shell.visible("oficina/projectPane"), "the project pane is drawn");
    // Docked: in the docking area, open, not floating, its pane drawn in the main window.
    assert_eq!(shell.property("oficina/area", "uniqueName"), "workbench");
    shell.invoke("oficina/area", "describeDocks", &[]);
    shell.wait_for("oficina/area", "docks", r#"[["project",true,false,true]]"#);
    // No folder named: the tree is empty and says nothing went wrong.
    assert_eq!(shell.property("oficina/projectError", "text"), "");
    assert_eq!(shell.rows(), Vec::<String>::new());
    assert_eq!(shell.errors(), Vec::<String>::new());
}
