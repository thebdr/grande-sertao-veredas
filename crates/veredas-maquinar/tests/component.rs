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
    let mut files = Vec::new();
    crate_files(&root, &mut files);
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
