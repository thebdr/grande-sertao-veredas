//! The bench's own rules that no suite law checks (contract: the bench clause; the laws in
//! `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library first, with at most one executable on top (`docs/maquinar-plan.md`, "Rules"). Cargo
//! is asked what it would build (`cargo metadata --no-deps`, which parses and builds nothing): the
//! targets of `veredas-maquinar` are exactly one library of `lib`/`rlib` crate types, at most one
//! binary named `veredas-maquinar`, and tests. An example, a bench, a build script, a second
//! binary or a missing library each break the rule.
//!
//! Content-agnostic, like core (`docs/questions.md` Q8): no file of the crate names the vendor
//! protocol core's telegram layer was ported from, or cites a vendor document.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The one executable the bench may build.
const BINARY: &str = "veredas-maquinar";

/// What cargo would build for the package at `dir`: `(name, kinds, crate types)` per target.
fn targets(dir: &Path) -> Vec<(String, Vec<String>, Vec<String>)> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap();
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
    };
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), strings(&t["kind"]), strings(&t["crate_types"])))
        .collect()
}

/// The rule: exactly one library target of `lib`/`rlib` crate types, at most one binary named
/// `binary`, everything else a test. Returns what breaks it, empty when it holds.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)], binary: &str) -> Vec<String> {
    // An empty list is no library: cargo accepts `crate-type = []` and then builds nothing.
    let library = |kinds: &[String]| !kinds.is_empty() && kinds.iter().all(|k| k == "lib" || k == "rlib");
    let mut out = Vec::new();
    let mut libs = 0;
    let mut bins = 0;
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
    if bins > 1 {
        out.push(format!("{bins} binaries named {binary}"));
    }
    out
}

#[test]
fn the_bench_is_one_library_and_at_most_one_executable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let t = targets(&dir);
    assert_eq!(breaches(&t, BINARY), Vec::<String>::new(), "{t:?}");
    let lib: Vec<_> = t.iter().filter(|(_, k, _)| k == &["lib"]).collect();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].0, "veredas_maquinar");
}

/// A throwaway package outside the workspace, with the files given.
fn planted(case: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-maquinar-component-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

const LIB: (&str, &str) = ("src/lib.rs", "pub fn f() {}\n");

fn manifest(extra: &str) -> String {
    format!("[workspace]\n\n[package]\nname = \"x\"\nversion = \"2.0.0\"\nedition = \"2024\"\n{extra}")
}

#[test]
fn the_check_allows_one_named_binary_and_refuses_every_other_executable() {
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("plain", vec![("Cargo.toml", manifest(""))]),
        ("main", vec![("Cargo.toml", manifest("")), ("src/main.rs", "fn main() {}\n".into())]),
        ("named", vec![("Cargo.toml", manifest("[[bin]]\nname = \"x\"\npath = \"src/lib.rs\"\n"))]),
        ("other", vec![("Cargo.toml", manifest("[[bin]]\nname = \"y\"\npath = \"src/lib.rs\"\n"))]),
        (
            "two",
            vec![("Cargo.toml", manifest("")), ("src/main.rs", "fn main() {}\n".into()), ("src/bin/y.rs", "fn main() {}\n".into())],
        ),
        ("example", vec![("Cargo.toml", manifest("[[example]]\nname = \"e\"\npath = \"src/lib.rs\"\n"))]),
        ("examples", vec![("Cargo.toml", manifest("")), ("examples/e.rs", "fn main() {}\n".into())]),
        ("cratetype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"rlib\", \"bin\"]\n"))]),
        ("cdylib", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"cdylib\"]\n"))]),
        ("emptytype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = []\n"))]),
        ("bench", vec![("Cargo.toml", manifest("")), ("benches/b.rs", "fn main() {}\n".into())]),
        ("build", vec![("Cargo.toml", manifest("")), ("build.rs", "fn main() {}\n".into())]),
        // Only a binary and tests, no library.
        (
            "nolib",
            vec![("Cargo.toml", manifest("autolib = false\n")), ("src/main.rs", "fn main() {}\n".into()), ("tests/t.rs", "#[test]\nfn t() {}\n".into())],
        ),
    ];
    let mut verdicts = Vec::new();
    for (case, files) in &cases {
        let mut all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        if *case != "nolib" {
            all.push(LIB);
        }
        let dir = planted(case, &all);
        // The planted package is named `x`, so its one allowed binary is `x`.
        verdicts.push((*case, breaches(&targets(&dir), "x").is_empty()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert_eq!(
        verdicts,
        [
            ("plain", true),
            ("main", true),
            ("named", true),
            ("other", false),
            ("two", false),
            ("example", false),
            ("examples", false),
            ("cratetype", false),
            ("cdylib", false),
            ("emptytype", false),
            ("bench", false),
            ("build", false),
            ("nolib", false),
        ]
    );
}

/// Every file under `dir`, recursively, whatever its extension: rustc reads more than `.rs`
/// (`include!`, `#[path]`), the manifest is part of the crate, and a fixture is read by a test.
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

/// The files a crate file pulls in from elsewhere: every `include_str!`, `include_bytes!` and
/// `include!` argument and every `#[path = "..."]`, as a string literal or as
/// `concat!(env!("CARGO_MANIFEST_DIR"), "...")`, resolved from the including file's folder (an
/// absolute path as it is; the manifest dir for the `concat!` form). A file the crate compiles
/// in from outside its folder is part of the crate, so it is read too.
fn included_files(file: &Path, text: &str, manifest_dir: &Path) -> Vec<PathBuf> {
    let here = file.parent().unwrap_or(manifest_dir);
    let mut out = Vec::new();
    let mut rest = text;
    let markers = ["include_str!", "include_bytes!", "include!", "#[path"];
    while let Some((at, marker)) = markers.iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|(i, _)| *i) {
        let after = &rest[at + marker.len()..];
        let arg = after.trim_start_matches([' ', '\t', '(', '=']);
        let (literal, base): (Option<&str>, &Path) = if let Some(s) = arg.strip_prefix('"') {
            (s.split('"').next(), here)
        } else if let Some(s) = arg.strip_prefix("concat!(env!(\"CARGO_MANIFEST_DIR\")") {
            let s = s.trim_start_matches([' ', ',']);
            (s.strip_prefix('"').and_then(|s| s.split('"').next()), manifest_dir)
        } else {
            (None, here)
        };
        if let Some(l) = literal {
            let p = Path::new(l);
            let resolved = if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(l.trim_start_matches(['/', '\\']))
            };
            if resolved.is_file() {
                out.push(resolved);
            }
        }
        rest = &rest[at + marker.len()..];
    }
    out
}

/// Every file of the crate: those under its folder, and those its files pull in from elsewhere,
/// each read once.
fn all_crate_files(manifest_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    crate_files(manifest_dir, &mut files);
    let mut seen: std::collections::BTreeSet<PathBuf> = files.iter().map(|f| f.canonicalize().unwrap()).collect();
    let mut queue = files.clone();
    while let Some(file) = queue.pop() {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).to_string();
        for inc in included_files(&file, &text, manifest_dir) {
            let key = inc.canonicalize().unwrap();
            if seen.insert(key) {
                files.push(inc.clone());
                queue.push(inc);
            }
        }
    }
    files
}

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one.
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites a
/// vendor document, as `line N: <term>`: the same reader as core's (`tests/foundation.rs`). The
/// whole text is lowercased and stripped to letters and digits as one string first, so no
/// separator, case, spacing or line break hides a name; each hit names the line where it
/// starts. A section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> =
        ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"]
            .iter()
            .map(|b| rev(b))
            .collect();
    let mut flat = String::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in line.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase) {
            flat.push(c);
            line_of.push(n + 1);
        }
    }
    let chars: Vec<char> = flat.chars().collect();
    let mut hits = Vec::new();
    for term in &banned {
        let t: Vec<char> = term.chars().collect();
        for start in 0..chars.len().saturating_sub(t.len() - 1) {
            if chars[start..start + t.len()] == t[..] {
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

#[test]
fn the_bench_names_no_vendor_and_cites_no_vendor_document() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = all_crate_files(&root);
    assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "{files:?}");
    assert!(files.len() >= 4, "{files:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display()))
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The reader sees every spelling, in any case, across line breaks, and the vendor's terms
    // and citations; each sample holds exactly one, found on the line it starts.
    let samples = [
        (format!("// the {} catalog", rev("tleBineG")), 1),
        (format!("const {}: u16 = 1;", rev("TROP_TLEB_INEG")), 1),
        (format!("// see {}/spec.txt", rev("RODNEV/SCOD")), 1),
        (format!("// the {} speaks", rev("lortnoC troS retsaM")), 1),
        (format!("// {} telegrams", rev("MAOOTXTR")), 1),
        (format!("// spec {}3.2", char::from_u32(0xA7).unwrap()), 1),
        (format!("// a {}", rev("ecafretni-tsoh")), 1),
        (format!("fn x() {{}}\n/// the {}\n/// {} over TCP", rev("troS retsaM"), rev("lortnoC")), 2),
        (format!("/// the {}\n/// {} catalog", rev("-ineG"), rev("tleB")), 1),
        (format!("id = 51004 # {}", rev("maootxtr")), 1),
    ];
    for (s, line) in &samples {
        let hits = vendor_mentions(s);
        assert_eq!(hits.len(), 1, "{s}: {hits:?}");
        assert!(hits[0].starts_with(&format!("line {line}: ")), "{s}: {hits:?}");
    }
    assert_eq!(vendor_mentions("fn decode(frame: &[u8]) {}\n// an announcement\nid = 21"), Vec::<String>::new());
}

#[test]
fn a_file_the_crate_pulls_in_from_outside_its_folder_is_read_too() {
    // A planted crate whose files include, by each form rustc knows, files outside its folder
    // that hold a vendor term: every one is found; a file that is not there is left alone.
    let dir = planted(
        "includes",
        &[
            ("Cargo.toml", "[package]\nname = \"x\"\n"),
            (
                "crate/src/lib.rs",
                "const A: &str = include_str!(\"../../outside/a.txt\");\n\
                 const B: &[u8] = include_bytes!( \"../../outside/b.bin\" );\n\
                 #[path = \"../../outside/c.rs\"]\nmod c;\n\
                 const D: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../outside/d.txt\"));\n\
                 const E: &str = include_str!(\"../../outside/missing.txt\");\n\
                 include!(\"../../outside/e.rs\");\n",
            ),
            ("crate/src/inner.rs", "// an announcement\n"),
            ("outside/a.txt", &format!("the {} catalog", rev("tleBineG"))),
            ("outside/b.bin", &format!("x {} y", rev("MAOOTXTR"))),
            ("outside/c.rs", &format!("// {}\npub fn f() {{}}\n", rev("lortnoC troS retsaM"))),
            ("outside/d.txt", &format!("see {}/spec.txt", rev("rodnev/scod"))),
            ("outside/e.rs", "// an outcome\n// and a nested one\nconst F: &str = include_str!(\"f.txt\");\n"),
            ("outside/f.txt", &format!("{}3.1", char::from_u32(0xA7).unwrap())),
        ],
    );
    let manifest_dir = dir.join("crate");
    let files = all_crate_files(&manifest_dir);
    let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().to_string()).collect();
    for expected in ["lib.rs", "inner.rs", "a.txt", "b.bin", "c.rs", "d.txt", "e.rs", "f.txt"] {
        assert!(names.contains(&expected.to_string()), "{expected} missing from {names:?}");
    }
    assert_eq!(files.len(), 8, "{names:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.file_name().unwrap().to_string_lossy()))
        })
        .collect();
    assert_eq!(found.len(), 5, "{found:?}");
    for file in ["a.txt", "b.bin", "c.rs", "d.txt", "f.txt"] {
        assert!(found.iter().any(|h| h.starts_with(&format!("{file}: "))), "{file} not reported: {found:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Crates the bench must never carry, by name prefix: a database (test-only and light), a
/// desktop UI (Q12), and any member of the suite but core (the plan's rule: core alone until
/// aranha exists; never portal, whose store and desktop the bench never pulls).
const BANNED: &[&str] = &["duckdb", "libduckdb", "eframe", "egui", "winit", "veredas-"];

/// The banned crates among `names`: the bench itself and `veredas-core` excepted.
fn banned_among<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|n| *n != "veredas-core" && *n != BINARY && BANNED.iter().any(|b| n.starts_with(b)))
        .map(String::from)
        .collect()
}

/// Every crate in the bench's resolved dependency graph (normal, build and dev, transitively), by
/// name: what cargo would build for it, asked with `cargo metadata` and walked from its node.
fn dependency_graph(manifest: &Path) -> std::collections::BTreeSet<String> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--manifest-path"])
        .arg(manifest)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let packages = meta["packages"].as_array().unwrap();
    let name_of = |id: &str| -> String {
        packages.iter().find(|p| p["id"].as_str() == Some(id)).map(|p| p["name"].as_str().unwrap().to_string()).unwrap()
    };
    let wanted = manifest.canonicalize().unwrap();
    let root = packages
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == wanted)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let nodes = meta["resolve"]["nodes"].as_array().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = vec![root];
    while let Some(id) = queue.pop() {
        if !seen.insert(name_of(&id)) {
            continue;
        }
        let node = nodes.iter().find(|n| n["id"].as_str() == Some(&id)).unwrap();
        for dep in node["deps"].as_array().unwrap() {
            queue.push(dep["pkg"].as_str().unwrap().to_string());
        }
    }
    seen
}

#[test]
fn the_bench_carries_no_database_no_desktop_and_no_member_but_core() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let graph = dependency_graph(&manifest);
    // The walk reaches the direct dependencies and the transitive ones behind them.
    for expected in ["veredas-core", "async-opcua-server", "async-opcua-client", "tokio", "async-opcua-core", "parking_lot", "regex-syntax"] {
        assert!(graph.contains(expected), "{expected} not in the graph: {graph:?}");
    }
    assert_eq!(banned_among(graph.iter().map(String::as_str)), Vec::<String>::new());
    // The ban list itself, on planted names.
    assert_eq!(
        banned_among(["duckdb", "libduckdb-sys", "eframe", "egui_extras", "winit", "veredas-portal", "veredas-core", "veredas-maquinar", "tokio", "veredas_portal"]),
        ["duckdb", "libduckdb-sys", "eframe", "egui_extras", "winit", "veredas-portal"]
    );
}
