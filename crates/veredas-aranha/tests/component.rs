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
    let nodes: BTreeMap<&str, &Value> =
        meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let names: BTreeMap<&str, &str> =
        meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap())).collect();
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
    seen.iter().map(|id| names[id.as_str()].to_string()).collect()
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
            ("Cargo.toml", "[workspace]\nmembers = [\"root\", \"a\", \"b\", \"c\", \"d\"]\nresolver = \"3\"\n".into()),
            ("root/Cargo.toml", "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\na = { path = \"../a\" }\n[build-dependencies]\nb = { path = \"../b\" }\n[dev-dependencies]\nc = { path = \"../c\" }\n".into()),
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
    let graph = reached(&meta, &root_id(&meta, &manifest));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(graph.into_iter().collect::<Vec<_>>(), ["a", "b", "d", "root"]);
}

#[test]
fn the_codecs_compile_alone_on_what_core_already_has() {
    // src/profinet/ copied as it is into a crate of its own whose only dependencies are serde and
    // thiserror: anything it took from the rest of aranha, or from another crate, fails to build.
    let src = manifest_dir().join("src/profinet");
    let mut files = vec![
        (
            "Cargo.toml".to_string(),
            "[workspace]\n\n[package]\nname = \"codecs-alone\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nthiserror = \"2\"\n".to_string(),
        ),
        ("src/lib.rs".to_string(), "#![deny(warnings)]\npub mod profinet;\n".to_string()),
    ];
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        assert!(path.is_file(), "the codecs are flat files: {}", path.display());
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        files.push((format!("src/profinet/{name}"), std::fs::read_to_string(&path).unwrap()));
    }
    let names: BTreeSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    assert!(["src/profinet/mod.rs", "src/profinet/dcp.rs", "src/profinet/lldp.rs", "src/profinet/ether.rs", "src/profinet/mac.rs"].iter().all(|f| names.contains(f)), "{names:?}");
    // Planted under the folder cargo gives integration tests, with a target folder of its own
    // kept between runs (a crate of another name, so nothing built there is taken for aranha).
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("codecs-alone");
    let dir = base.join("crate");
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in &files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let check = || {
        Command::new(env!("CARGO"))
            .args(["check", "--offline", "--quiet", "--manifest-path"])
            .arg(dir.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", base.join("target"))
            .output()
            .unwrap()
    };
    // The check can fail: a codec that reached for the rest of the crate does not build alone.
    let modrs = dir.join("src/profinet/mod.rs");
    let clean = std::fs::read_to_string(&modrs).unwrap();
    std::fs::write(&modrs, format!("{clean}\npub use crate::wire::Frame;\n")).unwrap();
    let poisoned = check();
    assert!(!poisoned.status.success(), "a borrowed path went unseen");
    assert!(String::from_utf8_lossy(&poisoned.stderr).contains("wire"), "{}", String::from_utf8_lossy(&poisoned.stderr));
    std::fs::write(&modrs, clean).unwrap();
    let out = check();
    assert!(out.status.success(), "the codecs need more than serde and thiserror:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one. The same terms core's reader bans (`tests/foundation.rs`).
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites a
/// vendor document, as `line N: <term>`: core's reader. The whole text is lowercased and stripped
/// to letters and digits as one string, so no separator, case or line break hides a name; a
/// section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> =
        ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"].iter().map(|b| rev(b)).collect();
    let mut flat = Vec::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in line.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase) {
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
    let section = char::from_u32(0xA7).unwrap();
    for (n, line) in text.lines().enumerate() {
        if line.contains(section) {
            hits.push(format!("line {}: section sign", n + 1));
        }
    }
    hits
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
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display()))
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The reader sees every spelling, across line breaks, and the citation sign.
    let samples = [
        (format!("// the {} catalog", rev("tleBineG")), 1),
        (format!("// see {}/x.txt", rev("RODNEV/SCOD")), 1),
        (format!("fn x() {{}}\n/// the {}\n/// {} link", rev("troS retsaM"), rev("lortnoC")), 2),
        (format!("// a {}", rev("ecafretni-tsoh")), 1),
        (format!("// clause {}4.1", char::from_u32(0xA7).unwrap()), 1),
    ];
    for (s, line) in &samples {
        let hits = vendor_mentions(s);
        assert_eq!(hits.len(), 1, "{s}: {hits:?}");
        assert!(hits[0].starts_with(&format!("line {line}: ")), "{s}: {hits:?}");
    }
    assert_eq!(vendor_mentions("// the host's own interface"), Vec::<String>::new());
}
