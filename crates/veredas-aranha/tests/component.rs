//! The scanner as a component (contract: the aranha component clause; the laws in `tools/leis`
//! judge its name, version, dependency direction and plan).
//!
//! - A library first, one executable on top: cargo, asked what it would build, names exactly one
//!   library (`lib`/`rlib`), at most one binary named `veredas-aranha`, and tests.
//! - Light, because the bench depends on it for the codecs: what a crate that depends on aranha
//!   compiles (its normal and build dependencies, every feature on, transitively) holds no
//!   database, no web stack, no UI crate and no member of the suite but core. Dev-dependencies are
//!   free: a bench is a fixture.
//! - The codecs move into core as a move (P-018): `src/profinet/` compiles on its own as a crate
//!   whose only dependencies are the two core already has, `serde` and `thiserror`.
//! - It names no vendor: no file of the crate holds the terms core's reader bans, or a section
//!   sign citing a document (`docs/questions.md` Q8, applied to the code that moves into core and
//!   to the rest alike).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn metadata(manifest: &Path, extra: &[&str]) -> Value {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .args(extra)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// What cargo would build for the package at `dir`: `(name, kinds, crate types)` per target.
fn targets(dir: &Path) -> Vec<(String, Vec<String>, Vec<String>)> {
    let meta = metadata(&dir.join("Cargo.toml"), &["--no-deps", "--offline"]);
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()
        .clone();
    let strings = |v: &Value| -> Vec<String> { v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect() };
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), strings(&t["kind"]), strings(&t["crate_types"])))
        .collect()
}

/// The rule: one library of `lib`/`rlib` crate types, at most one binary named
/// `veredas-aranha`, everything else a test. What breaks it, empty when it holds.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)]) -> Vec<String> {
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
        if kinds == &["bin"] && crate_types == &["bin"] && name == "veredas-aranha" {
            bins += 1;
            continue;
        }
        out.push(format!("{name}: {kinds:?} {crate_types:?}"));
    }
    if libs != 1 {
        out.push(format!("{libs} library targets"));
    }
    if bins > 1 {
        out.push(format!("{bins} binaries"));
    }
    out
}

/// Every target's source file, as cargo lists it.
fn target_sources(dir: &Path) -> Vec<PathBuf> {
    let meta = metadata(&dir.join("Cargo.toml"), &["--no-deps", "--offline"]);
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()
        .clone();
    package["targets"].as_array().unwrap().iter().map(|t| PathBuf::from(t["src_path"].as_str().unwrap())).collect()
}

/// Every file rustc read to compile the targets of the package at `manifest`, by the dep-info it
/// wrote for each, in a check of every target with `args`, into `target_dir`.
fn files_rustc_read(manifest: &Path, target_dir: &Path, args: &[&str]) -> BTreeSet<PathBuf> {
    let out = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--all-targets", "--message-format=json", "--manifest-path"])
        .arg(manifest)
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "check {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    let manifest = manifest.canonicalize().unwrap();
    // Dep-info names a file relative to the workspace's root.
    let meta = metadata(&manifest, &["--no-deps", "--offline"]);
    let root = PathBuf::from(meta["workspace_root"].as_str().unwrap());
    let mut files = BTreeSet::new();
    let mut units = 0;
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
        if msg["reason"] != "compiler-artifact" || Path::new(msg["manifest_path"].as_str().unwrap_or("")).canonicalize().ok() != Some(manifest.clone()) {
            continue;
        }
        // The artifact `deps/lib<name>-<hash>.rmeta` has its dep-info beside it, `<name>-<hash>.d`.
        let artifact = PathBuf::from(msg["filenames"][0].as_str().unwrap());
        let stem = artifact.file_stem().unwrap().to_str().unwrap();
        let dep = artifact.with_file_name(format!("{}.d", stem.strip_prefix("lib").unwrap_or(stem)));
        let text = std::fs::read_to_string(&dep).unwrap_or_else(|e| panic!("{}: {e}", dep.display()));
        let first = text.lines().next().unwrap();
        let (_, inputs) = first.split_once(": ").unwrap_or((first, ""));
        // Spaces in a path are written `\ `.
        let inputs = inputs.replace("\\ ", "\u{0}");
        files.extend(inputs.split_whitespace().map(|p| root.join(p.replace('\u{0}', " "))));
        units += 1;
    }
    assert!(units > 0, "no unit of the package was checked");
    files
}

/// The files of `files` that do not lie in `dir`, once every link is followed.
fn outside(files: &BTreeSet<PathBuf>, dir: &Path) -> Vec<PathBuf> {
    files.iter().filter(|f| !f.canonicalize().unwrap_or_else(|_| (*f).clone()).starts_with(dir)).cloned().collect()
}

#[test]
fn aranha_compiles_in_no_file_from_outside_its_folder() {
    let dir = manifest_dir().canonicalize().unwrap();
    // Every target is one cargo finds by itself (src/lib.rs, a file under tests/): the split of
    // the test below, which copies those, holds every one.
    for src in target_sources(&dir) {
        let rel = src.canonicalize().unwrap().strip_prefix(&dir).unwrap().to_path_buf();
        assert!(rel == Path::new("src/lib.rs") || rel.parent() == Some(Path::new("tests")), "a target cargo would not find by itself: {}", rel.display());
    }
    // Every file rustc reads for every target, in every build checked, by its own dep-info: no
    // spelling of an include, no module path, no link hides a file from it.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("depinfo");
    for args in [&["--all-features"][..], &["--all-features", "--release"], &["--all-features", "--target", "x86_64-pc-windows-gnu"]] {
        let read = files_rustc_read(&dir.join("Cargo.toml"), &target, args);
        assert!(read.iter().any(|f| f.ends_with("src/profinet/dcp.rs")) && read.iter().any(|f| f.ends_with("tests/component.rs")), "{read:?}");
        assert_eq!(outside(&read, &dir), Vec::<PathBuf>::new(), "{args:?}");
    }
    // The check can fail: a crate that pulls in a file by an include in braces behind a comment,
    // and a module that is a link to a file outside.
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("depinfo-planted");
    let _ = std::fs::remove_dir_all(&base);
    let krate = base.join("krate");
    std::fs::create_dir_all(krate.join("src")).unwrap();
    std::fs::write(base.join("note.txt"), "x").unwrap();
    std::fs::write(base.join("outside_mod.rs"), "pub const M: u8 = 1;\n").unwrap();
    let inc = format!("{}{}", "inclu", "de_str!");
    std::fs::write(krate.join("Cargo.toml"), manifest("")).unwrap();
    std::fs::write(krate.join("src/lib.rs"), format!("pub const N: &str = {inc} /* c */ {{\"../../note.txt\"}};\npub mod borrowed;\n")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("outside_mod.rs"), krate.join("src/borrowed.rs")).unwrap();
    let read = files_rustc_read(&krate.join("Cargo.toml"), &base.join("target"), &[]);
    let here = krate.canonicalize().unwrap();
    let mut out: Vec<String> = outside(&read, &here).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
    out.sort();
    assert_eq!(out, ["borrowed.rs", "note.txt"]);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn aranha_is_one_library_and_at_most_its_one_executable() {
    let t = targets(&manifest_dir());
    assert_eq!(breaches(&t), Vec::<String>::new(), "{t:?}");
    let built: Vec<&String> = t.iter().filter(|(_, k, _)| k != &["test"]).map(|(n, _, _)| n).collect();
    assert_eq!(built, ["veredas_aranha"]);
}

/// A throwaway package outside the workspace, with the files given.
fn planted(case: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-aranha-component-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn manifest(extra: &str) -> String {
    format!("[workspace]\n\n[package]\nname = \"veredas-aranha\"\nversion = \"2.0.0\"\nedition = \"2024\"\n{extra}")
}

#[test]
fn the_check_refuses_every_target_but_the_library_and_its_one_executable() {
    let main = || ("src/main.rs", "fn main() {}\n".to_string());
    type Case<'a> = (&'a str, Vec<(&'a str, String)>, bool);
    let cases: Vec<Case> = vec![
        ("plain", vec![("Cargo.toml", manifest(""))], true),
        ("main", vec![("Cargo.toml", manifest("")), main()], true),
        ("binnamed", vec![("Cargo.toml", manifest("[[bin]]\nname = \"veredas-aranha\"\npath = \"src/lib.rs\"\n"))], true),
        ("binother", vec![("Cargo.toml", manifest("[[bin]]\nname = \"scan\"\npath = \"src/lib.rs\"\n"))], false),
        ("twobins", vec![("Cargo.toml", manifest("")), main(), ("src/bin/more.rs", "fn main() {}\n".into())], false),
        ("example", vec![("Cargo.toml", manifest("")), ("examples/e.rs", "fn main() {}\n".into())], false),
        ("bench", vec![("Cargo.toml", manifest("")), ("benches/b.rs", "fn main() {}\n".into())], false),
        ("build", vec![("Cargo.toml", manifest("")), ("build.rs", "fn main() {}\n".into())], false),
        ("cdylib", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"cdylib\"]\n"))], false),
        ("libbin", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"rlib\", \"bin\"]\n"))], false),
        ("emptytype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = []\n"))], false),
        ("nolib", vec![("Cargo.toml", manifest("autolib = false\n")), main()], false),
    ];
    let mut verdicts = Vec::new();
    let mut want = Vec::new();
    for (case, files, ok) in &cases {
        let mut all = files.clone();
        all.push(("src/lib.rs", "pub fn f() {}\n".into()));
        let dir = planted(case, &all);
        verdicts.push((*case, breaches(&targets(&dir)).is_empty()));
        want.push((*case, *ok));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert_eq!(verdicts, want);
}

/// Crates no build of aranha may pull in, by name prefix: the database, the web stack, the UI
/// crates, and every member of the suite (core excepted below).
const BANNED: &[&str] = &[
    "duckdb", "libduckdb", "axum", "hyper", "tower-http", "warp", "actix", "rocket", "eframe", "egui", "winit", "iced", "slint", "gtk",
    "cxx-qt", "qmetaobject", "veredas-", "orcl-",
];

fn banned_among<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|n| *n != "veredas-core" && *n != "veredas-aranha" && BANNED.iter().any(|b| n.starts_with(b)))
        .map(str::to_string)
        .collect()
}

/// Every package a crate depending on `root` compiles: its normal and build dependencies, on every
/// platform, every feature on, walked by package id (two versions of one crate are two nodes).
fn reached(meta: &Value, root: &str) -> BTreeSet<String> {
    let names: BTreeMap<&str, &str> =
        meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap())).collect();
    reached_ids(meta, root).iter().map(|id| names[id.as_str()].to_string()).collect()
}

/// The members of the workspace a crate depending on `root` compiles, by the workspace's own
/// list of members (whatever their names), `allowed` excepted: their names.
fn members_reached(meta: &Value, root: &str, allowed: &[&str]) -> Vec<String> {
    let names: BTreeMap<&str, &str> =
        meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap())).collect();
    let members: BTreeSet<&str> = meta["workspace_members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    let mut out: Vec<String> = reached_ids(meta, root)
        .iter()
        .filter(|id| members.contains(id.as_str()))
        .map(|id| names[id.as_str()].to_string())
        .filter(|n| !allowed.contains(&n.as_str()))
        .collect();
    out.sort();
    out
}

fn reached_ids(meta: &Value, root: &str) -> BTreeSet<String> {
    let nodes: BTreeMap<&str, &Value> =
        meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let mut seen = BTreeSet::new();
    let mut todo = vec![root.to_string()];
    while let Some(id) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for dep in nodes[id.as_str()]["deps"].as_array().unwrap() {
            let shipped = dep["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null() || k["kind"] == "build");
            if shipped {
                todo.push(dep["pkg"].as_str().unwrap().to_string());
            }
        }
    }
    seen
}

fn root_id(meta: &Value, manifest: &Path) -> String {
    let manifest = manifest.canonicalize().unwrap();
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn what_a_consumer_of_aranha_compiles_is_light() {
    let manifest = manifest_dir().join("Cargo.toml");
    let meta = metadata(&manifest, &["--all-features"]);
    let graph = reached(&meta, &root_id(&meta, &manifest));
    assert!(graph.contains("serde") && graph.contains("thiserror"), "{graph:?}");
    assert_eq!(banned_among(graph.iter().map(String::as_str)), Vec::<String>::new(), "{graph:?}");
    // No member of the workspace but core, whatever its name (a tool's, a crate cargo took in).
    let root = root_id(&meta, &manifest);
    assert_eq!(members_reached(&meta, &root, &["veredas-aranha", "veredas-core"]), Vec::<String>::new());
    // The ban list reads names by prefix, the scanner and core excepted.
    assert_eq!(
        banned_among(["duckdb", "libduckdb-sys", "axum-core", "hyper-util", "egui_extras", "veredas-portal", "veredas-maquinar", "orcl-std", "veredas-core", "veredas-aranha", "tokio", "serde"]),
        ["duckdb", "libduckdb-sys", "axum-core", "hyper-util", "egui_extras", "veredas-portal", "veredas-maquinar", "orcl-std"]
    );
}

#[test]
fn the_walk_follows_what_ships_and_leaves_dev_dependencies_to_the_tests() {
    // A planted graph: the root uses `a` normally and `b` to build; `c` only in its tests; `a`
    // uses `d`, which only `c` would otherwise reach.
    let dir = planted(
        "walk",
        &[
            ("Cargo.toml", "[workspace]\nmembers = [\"root\", \"a\", \"b\", \"c\", \"d\", \"e\", \"f\", \"g\"]\nresolver = \"3\"\n".into()),
            (
                "root/Cargo.toml",
                "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[features]\nextra = [\"dep:e\"]\n[dependencies]\na = { path = \"../a\" }\ne = { path = \"../e\", optional = true }\n[build-dependencies]\nb = { path = \"../b\" }\n[dev-dependencies]\nc = { path = \"../c\" }\n[target.'cfg(windows)'.dependencies]\nf = { path = \"../f\" }\n[target.'cfg(target_arch = \"aarch64\")'.dependencies]\ng = { path = \"../g\" }\n".into(),
            ),
            ("e/Cargo.toml", "[package]\nname = \"e\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("e/src/lib.rs", String::new()),
            ("f/Cargo.toml", "[package]\nname = \"f\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("f/src/lib.rs", String::new()),
            ("g/Cargo.toml", "[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("g/src/lib.rs", String::new()),
            ("root/src/lib.rs", String::new()),
            ("a/Cargo.toml", "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nd = { path = \"../d\" }\n".into()),
            ("a/src/lib.rs", String::new()),
            ("b/Cargo.toml", "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("b/src/lib.rs", String::new()),
            ("c/Cargo.toml", "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("c/src/lib.rs", String::new()),
            ("d/Cargo.toml", "[package]\nname = \"d\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("d/src/lib.rs", String::new()),
        ],
    );
    let manifest = dir.join("root/Cargo.toml");
    let meta = metadata(&manifest, &["--all-features", "--offline"]);
    let root = root_id(&meta, &manifest);
    let graph = reached(&meta, &root);
    let members = members_reached(&meta, &root, &["root"]);
    std::fs::remove_dir_all(&dir).unwrap();
    // An optional dependency behind a feature, and dependencies on Windows and on another
    // architecture, all reached: every feature on, every platform.
    assert_eq!(graph.into_iter().collect::<Vec<_>>(), ["a", "b", "d", "e", "f", "g", "root"]);
    // Every one of them a member of that workspace, named however: each reached member is named.
    assert_eq!(members, ["a", "b", "d", "e", "f", "g"]);
}

/// Copy the tree at `from` into `to`, leaving out `skip` (a folder under `from`).
fn copy_tree(from: &Path, to: &Path, skip: Option<&Path>) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        if Some(path.as_path()) == skip {
            continue;
        }
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target, skip);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The codecs as a crate of their own beside the rest of aranha built on them as another crate,
/// planted under the folder cargo gives integration tests; `poison` edits one file first.
struct Split {
    base: PathBuf,
}

impl Split {
    fn plant(poisons: &[(&str, &str)]) -> Split {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("codecs-alone");
        let ws = base.join("ws");
        let _ = std::fs::remove_dir_all(&ws);
        let src = manifest_dir().join("src");
        let write = |p: PathBuf, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"codecs\", \"rest\"]\nresolver = \"3\"\n");
        // The codecs: src/profinet as it is, depending on serde and thiserror alone.
        write(
            ws.join("codecs/Cargo.toml"),
            "[package]\nname = \"codecs-alone\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nthiserror = \"2\"\n",
        );
        write(ws.join("codecs/src/lib.rs"), "#![deny(warnings)]\npub mod profinet;\n");
        copy_tree(&src.join("profinet"), &ws.join("codecs/src/profinet"), None);
        // The rest: aranha's own manifest's dependencies, its sources but the codecs, its tests,
        // the codecs taken from the other crate.
        write(
            ws.join("rest/Cargo.toml"),
            "[package]\nname = \"veredas-aranha\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\ncodecs-alone = { path = \"../codecs\" }\nserde = { version = \"1\", features = [\"derive\"] }\nthiserror = \"2\"\n\n[target.'cfg(target_os = \"linux\")'.dependencies]\nlibc = \"0.2\"\n\n[dev-dependencies]\nicu_normalizer = { version = \"2\", default-features = false, features = [\"compiled_data\"] }\nserde_json = \"1\"\n",
        );
        copy_tree(&src, &ws.join("rest/src"), Some(&src.join("profinet")));
        copy_tree(&manifest_dir().join("tests"), &ws.join("rest/tests"), None);
        let lib = std::fs::read_to_string(ws.join("rest/src/lib.rs")).unwrap();
        assert_eq!(lib.matches("pub mod profinet;").count(), 1, "{lib}");
        write(ws.join("rest/src/lib.rs"), &lib.replace("pub mod profinet;", "pub use codecs_alone::profinet;"));
        for (file, text) in poisons {
            let f = ws.join(file);
            let old = std::fs::read_to_string(&f).unwrap();
            write(f, &format!("{old}\n{text}\n"));
        }
        Split { base }
    }

    /// `cargo <args>` on the planted workspace, offline, into a target folder kept between runs (its
    /// crates are named apart from the real one's tests, so nothing built there is taken for them).
    fn cargo(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO"))
            .args(args)
            .args(["--offline", "--quiet", "--manifest-path"])
            .arg(self.base.join("ws/Cargo.toml"))
            .env("CARGO_TARGET_DIR", self.base.join("target"))
            .output()
            .unwrap()
    }
}

#[test]
fn the_codecs_build_alone_and_the_rest_of_aranha_builds_on_them_as_another_crate() {
    // src/profinet/ holds flat files, and no `cfg` but `cfg(test)`: what a build on this platform
    // checks is what every platform compiles.
    for entry in std::fs::read_dir(manifest_dir().join("src/profinet")).unwrap() {
        let path = entry.unwrap().path();
        assert!(path.is_file(), "the codecs are flat files: {}", path.display());
        let text = std::fs::read_to_string(&path).unwrap();
        let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(flat.matches("cfg").count(), flat.matches("cfg(test)").count(), "{}", path.display());
    }
    // Every target of both crates (libraries, unit tests, aranha's integration tests) checks, and
    // the codecs' doctests run.
    let split = Split::plant(&[]);
    // The rest's manifest names what aranha's does (each dependency with its kind and platform,
    // and the features), the codecs' crate besides.
    let deps = |manifest: &Path| -> (BTreeSet<(String, String, String)>, String) {
        let meta = metadata(manifest, &["--no-deps", "--offline"]);
        let id = root_id(&meta, manifest);
        let package = meta["packages"].as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).unwrap().clone();
        let deps = package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                let field = |k: &str| d[k].as_str().unwrap_or("").to_string();
                (field("name"), field("kind"), field("target"))
            })
            .filter(|(n, _, _)| n != "codecs-alone")
            .collect();
        (deps, package["features"].to_string())
    };
    assert_eq!(deps(&split.base.join("ws/rest/Cargo.toml")), deps(&manifest_dir().join("Cargo.toml")));
    // Every configuration a build of aranha may take: every feature on, in both profiles, and for
    // a second platform (Windows, where the plan's Npcap phase adds code to the rest).
    let checks: [&[&str]; 3] = [
        &["check", "--workspace", "--all-targets", "--all-features"],
        &["check", "--workspace", "--all-targets", "--all-features", "--release"],
        &["check", "--workspace", "--all-targets", "--all-features", "--target", "x86_64-pc-windows-gnu"],
    ];
    for args in checks {
        let out = split.cargo(args);
        assert!(out.status.success(), "the split does not build with {args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    let out = split.cargo(&["test", "--doc", "-p", "codecs-alone"]);
    assert!(out.status.success(), "the codecs' doctests:\n{}", String::from_utf8_lossy(&out.stderr));

    // The check can fail, from either side: a codec test that reaches for the rest of the crate,
    // a codec that reaches for it, and the rest giving a codec type a method of its own (which it
    // could not do to core's type).
    let reach = "impl crate::profinet::mac::Mac {\n    pub fn heard(&self) -> bool {\n        true\n    }\n}";
    let gated = |gate: &str| format!("#[cfg({gate})]\n{reach}");
    let (windows, release, feature) = (gated("windows"), gated("not(debug_assertions)"), gated("feature = \"npcap\""));
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![("codecs/src/profinet/mod.rs", "#[cfg(test)]\nmod reach {\n    #[test]\n    fn f() {\n        let _ = crate::wire::raw::MAX_FRAME;\n    }\n}")], "wire"),
        (vec![("codecs/src/profinet/mod.rs", "pub use crate::wire::raw::RawLink;")], "wire"),
        (vec![("rest/src/lib.rs", reach)], "E0116"),
        // The same reach behind each configuration only some build takes.
        (vec![("rest/src/lib.rs", &windows)], "E0116"),
        (vec![("rest/src/lib.rs", &release)], "E0116"),
        (vec![("rest/Cargo.toml", "[features]\nnpcap = []"), ("rest/src/lib.rs", &feature)], "E0116"),
    ];
    for (poisons, want) in cases {
        let split = Split::plant(&poisons);
        let errs: Vec<String> =
            checks.iter().map(|a| split.cargo(a)).filter(|o| !o.status.success()).map(|o| String::from_utf8_lossy(&o.stderr).to_string()).collect();
        assert!(errs.iter().any(|e| e.contains(want)), "{poisons:?} went unseen: {errs:?}");
    }
    Split::plant(&[]);
}

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one. The same terms core's reader bans (`tests/foundation.rs`).
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// The terms, reversed: core's list, word for word (the test below holds it to core's file).
const TERMS: [&str; 6] = ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"];

/// A line's letters and digits as the reader sees them: Unicode's decomposition first (NFKD: an
/// accented letter its base letter and its marks, a full-width or compatibility form its plain
/// letters), lowercased, marks and everything that is no letter or digit dropped.
fn letters(line: &str) -> Vec<char> {
    let decomposed = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(line);
    let class = icu_normalizer::properties::CanonicalCombiningClassMapBorrowed::new();
    decomposed.chars().flat_map(char::to_lowercase).flat_map(fold).filter(|c| c.is_alphanumeric() && class.get32_u8(*c as u32) == 0).collect()
}

/// The Latin letters NFKD leaves whole, read as their base letters: supervisão's table
/// (`crates/veredas-supervisao/tests/component.rs`).
fn fold(c: char) -> Vec<char> {
    let plain = match c {
        'æ' => "ae",
        'ð' | 'đ' => "d",
        'ø' => "o",
        'þ' => "th",
        'ß' => "ss",
        'ħ' => "h",
        'ı' => "i",
        'ĸ' => "k",
        'ł' => "l",
        'ŋ' => "n",
        'œ' => "oe",
        'ŧ' => "t",
        // The spacing modifier letters NFKD leaves (the apostrophe of `ŉ`, the half ring of `ẚ`).
        '\u{2b0}'..='\u{2ff}' => "",
        'ƒ' => "f",
        'ƶ' => "z",
        'ȷ' => "j",
        c => return vec![c],
    };
    plain.chars().collect()
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites a
/// vendor document, as `line N: <term>`: core's reader, reading through Unicode's decomposition.
/// The whole text is read as one string of letters and digits, so no separator, case, accent,
/// letter form or line break hides a name; a section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> = TERMS.iter().map(|b| rev(b)).collect();
    let mut flat = Vec::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in letters(line) {
            flat.push(c);
            line_of.push(n + 1);
        }
    }
    let mut hits = Vec::new();
    for term in &banned {
        let t: Vec<char> = term.chars().collect();
        for start in 0..flat.len().saturating_sub(t.len() - 1) {
            if flat[start..start + t.len()] == t[..] {
                hits.push(format!("line {}: {term}", line_of[start]));
            }
        }
    }
    for (n, line) in text.lines().enumerate() {
        if has_section_sign(line) {
            hits.push(format!("line {}: section sign", n + 1));
        }
    }
    hits
}

/// Whether a line holds the section sign as itself, or as an entity, an escape or a percent
/// encoding writes it: its named entity (with its semicolon or without, as browsers read it), a
/// numeric entity in decimal or hex with any leading zeros, a Rust escape in braces (any zeros,
/// underscores), a JSON or TOML escape of four or eight hex digits, a byte escape, and its UTF-8
/// bytes percent-encoded. The sign's spellings are decoded, not listed (and are written here in
/// pieces, so this file holds none whole).
fn has_section_sign(line: &str) -> bool {
    if line.contains(char::from_u32(0xA7).unwrap()) {
        return true;
    }
    let named = format!("&{}", "sect");
    let percent = format!("%c2{}", "%a7");
    let l = line.to_lowercase();
    let b = l.as_bytes();
    let hex_value = |digits: &str| u32::from_str_radix(digits, 16).ok();
    let mut i = 0;
    while i < b.len() {
        let rest = &l[i..];
        let found = if let Some(r) = rest.strip_prefix(named.as_str()) {
            r.is_empty() || !r.as_bytes()[0].is_ascii_alphanumeric()
        } else if let Some(r) = rest.strip_prefix("&#x") {
            let d: String = r.chars().take_while(char::is_ascii_hexdigit).collect();
            !d.is_empty() && hex_value(&d) == Some(0xA7)
        } else if let Some(r) = rest.strip_prefix("&#") {
            let d: String = r.chars().take_while(char::is_ascii_digit).collect();
            !d.is_empty() && d.parse::<u32>().ok() == Some(167)
        } else if let Some(r) = rest.strip_prefix("\\u{") {
            let inner: String = r.chars().take_while(|c| *c != '}').filter(|c| *c != '_').collect();
            hex_value(&inner) == Some(0xA7)
        } else if let Some(r) = rest.strip_prefix("\\u") {
            let d: String = r.chars().take_while(char::is_ascii_hexdigit).take(8).collect();
            (d.len() >= 4 && hex_value(&d[..4]) == Some(0xA7)) || (d.len() == 8 && hex_value(&d) == Some(0xA7))
        } else if let Some(r) = rest.strip_prefix("\\x") {
            r.len() >= 2 && hex_value(&r[..2]) == Some(0xA7)
        } else {
            rest.starts_with(percent.as_str())
        };
        if found {
            return true;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn crate_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            crate_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn aranha_names_no_vendor_and_cites_no_vendor_document() {
    let mut files = Vec::new();
    crate_files(&manifest_dir(), &mut files);
    for f in ["Cargo.toml", "src/lib.rs", "src/profinet/dcp.rs", "tests/dcp.rs", "tests/common/mod.rs", "tests/component.rs"] {
        assert!(files.contains(&manifest_dir().join(f)), "{f} not read: {files:?}");
    }
    // Every file is text and has a name that is text: a file or a name that is not UTF-8 (a
    // Windows code page, Latin-1 bytes) would hide a spelling from the reader, so it is refused.
    let not_text: Vec<String> = files
        .iter()
        .filter(|f| std::str::from_utf8(&std::fs::read(f).unwrap()).is_err() || f.strip_prefix(manifest_dir()).unwrap().to_str().is_none())
        .map(|f| f.display().to_string())
        .collect();
    assert_eq!(not_text, Vec::<String>::new());
    // Every file's contents, and every file's name, its path from the crate's folder.
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8(std::fs::read(f).unwrap()).unwrap();
            let name = f.strip_prefix(manifest_dir()).unwrap().to_str().unwrap().to_string();
            let named = vendor_mentions(&name).into_iter().map(move |l| format!("the name {}: {l}", f.display()));
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display())).chain(named)
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The letter table is supervisão's, held to its file; with it every letter of Latin-1 and
    // Latin Extended-A, and the table's three of Extended-B, reads as plain letters.
    let body = |file: &str| -> String {
        let text = std::fs::read_to_string(file).unwrap();
        let start = text.find("fn fold(c: char) -> Vec<char> {").unwrap();
        let end = start + text[start..].find("c => return vec![c],").unwrap();
        text[start..end].lines().map(str::trim).collect::<Vec<_>>().join("\n")
    };
    let own = manifest_dir().join("tests/component.rs");
    let theirs = manifest_dir().join("../veredas-supervisao/tests/component.rs");
    assert_eq!(body(own.to_str().unwrap()), body(theirs.to_str().unwrap()));
    let not_plain: Vec<char> = ('\u{c0}'..='\u{17f}')
        .chain(['ƒ', 'ƶ', 'ȷ'])
        .filter(|c| c.is_alphabetic())
        .filter(|c| !letters(&c.to_string()).iter().all(|l| l.is_ascii_lowercase()))
        .collect();
    assert_eq!(not_plain, Vec::<char>::new());
    // The terms are core's, held to core's own file.
    let core = std::fs::read_to_string(manifest_dir().join("../veredas-core/tests/foundation.rs")).unwrap();
    let list = format!("[{}]", TERMS.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", "));
    assert!(core.contains(&list), "core's list is not {list}");
    // The reader sees every spelling, across line breaks, and the citation sign.
    let fullwidth: String = rev("tlebineg").chars().map(|c| char::from_u32(c as u32 - 0x61 + 0xFF41).unwrap()).collect();
    let samples = [
        (format!("// the {} catalog", rev("tleBineG")), 1),
        (format!("// see {}/x.txt", rev("RODNEV/SCOD")), 1),
        (format!("fn x() {{}}\n/// the {}\n/// {} link", rev("troS retsaM"), rev("lortnoC")), 2),
        (format!("// a {}", rev("ecafretni-tsoh")), 1),
        (format!("// {} telegrams", rev("MAOOTXTR")), 1),
        (format!("// {} frames", rev("XTROTMAO")), 1),
        (format!("// clause {}4.1", char::from_u32(0xA7).unwrap()), 1),
        // Accented, and in full-width letters.
        (format!("// a device of the {} family", rev("tlebinég")), 1),
        (format!("// the {fullwidth} catalog"), 1),
        // A file's name.
        (format!("tests/fixtures/{}-io-device.bin", rev("tlebineg")), 1),
        // Latin letters NFKD leaves whole.
        (format!("// {}", rev("tlebineg").replacen('l', "\u{142}", 1)), 1),
        (format!("// {}", rev("rodnevscod").replace('o', "\u{f8}")), 1),
        (format!("// {}", rev("tlebineg").replacen('i', "\u{131}", 1)), 1),
        (format!("// {}", rev("ecafretnitsoh").replacen('t', "\u{167}", 1)), 1),
        // The citation sign as an entity and as escapes.
        (format!("/// clause &{}4.3", "sect;"), 1),
        (format!("/// clause &{} 4.3", "sect"), 1),
        (format!("let s = \"\\{}\";", "u{a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{0", "a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{0000", "a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{a_", "7}"), 1),
        (format!("/// clause &#{} 2", "167;"), 1),
        (format!("/// clause &#{} 2", "0167"), 1),
        (format!("/// clause &#{} 2", "x00a7;"), 1),
        (format!("description = \"\\{}{} 4.2\"", "u00", "A7"), 1),
        (format!("description = \"\\{}{} 4.2\"", "U000000", "A7"), 1),
        (format!("b\"\\{}{}\"", "x", "a7"), 1),
        (format!("https://x/{}{}", "%C2", "%A7"), 1),
    ];
    for (s, line) in &samples {
        let hits = vendor_mentions(s);
        assert_eq!(hits.len(), 1, "{s}: {hits:?}");
        assert!(hits[0].starts_with(&format!("line {line}: ")), "{s}: {hits:?}");
    }
    assert_eq!(vendor_mentions("// the host's own interface"), Vec::<String>::new());
    // Near the sign's spellings, but not it.
    for innocent in [format!("&{}ion", "sect"), format!("&#{}", "168;"), format!("\\{}", "u00a8"), format!("{}{}", "%C2", "%A8")] {
        assert_eq!(vendor_mentions(&innocent), Vec::<String>::new(), "{innocent}");
    }
}
