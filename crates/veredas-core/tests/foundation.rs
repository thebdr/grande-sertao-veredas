//! The foundation's own rule that no suite law checks (contract: the foundation clause; the
//! laws in `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library container, never an executable (`docs/core-plan.md`, "Rules"). Cargo is asked
//! what it would build (`cargo metadata --no-deps`, which parses and builds nothing) rather than
//! having its target rules re-implemented here: the targets of `veredas-core` must be exactly
//! one library, whose crate types are `lib`/`rlib`, and tests. A `[[bin]]`, an `[[example]]`,
//! `src/main.rs`, `src/bin/`, `examples/`, a `crate-type`/`crate_type` naming `bin`, a bench,
//! a build script, or a missing library (`autolib = false`) each show as a target that breaks
//! the rule.
//!
//! Content-agnostic (`docs/questions.md` Q8): no source or test file names the vendor protocol
//! the telegram layer was ported from, or cites a vendor document.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The rule: exactly one library target of `lib`/`rlib` crate types, everything else a test.
/// Returns what breaks it, empty when it holds.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)]) -> Vec<String> {
    // An empty list is no library: cargo accepts `crate-type = []` and then builds nothing
    // for the target (C-007, refute round 2).
    let library =
        |kinds: &[String]| !kinds.is_empty() && kinds.iter().all(|k| k == "lib" || k == "rlib");
    let mut out = Vec::new();
    let mut libs = 0;
    for (name, kinds, crate_types) in targets {
        if kinds == &["test"] {
            continue;
        }
        if library(kinds) && library(crate_types) {
            libs += 1;
            continue;
        }
        out.push(format!("{name}: {kinds:?} {crate_types:?}"));
    }
    if libs != 1 {
        out.push(format!("{libs} library targets"));
    }
    out
}

#[test]
fn core_is_one_library_and_builds_no_executable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let t = targets(&dir);
    assert_eq!(breaches(&t), Vec::<String>::new(), "{t:?}");
    let lib: Vec<_> = t.iter().filter(|(_, k, _)| k != &["test"]).collect();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].0, "veredas_core");
}

/// A throwaway package outside the workspace, with the files given.
fn planted(case: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-core-foundation-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

const LIB: (&str, &str) = ("src/lib.rs", "pub fn f() {}\n");

fn manifest(edition: &str, extra: &str) -> String {
    format!("[workspace]\n\n[package]\nname = \"x\"\nversion = \"2.0.0\"\nedition = \"{edition}\"\n{extra}")
}

#[test]
fn the_check_refuses_every_way_cargo_makes_an_executable() {
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("plain", vec![("Cargo.toml", manifest("2024", ""))]),
        ("bin", vec![("Cargo.toml", manifest("2024", "[[bin]]\nname = \"b\"\npath = \"src/lib.rs\"\n"))]),
        ("example", vec![("Cargo.toml", manifest("2024", "[[example]]\nname = \"e\"\npath = \"src/lib.rs\"\n"))]),
        ("main", vec![("Cargo.toml", manifest("2024", "")), ("src/main.rs", "fn main() {}\n".into())]),
        ("srcbin", vec![("Cargo.toml", manifest("2024", "")), ("src/bin/b.rs", "fn main() {}\n".into())]),
        ("examples", vec![("Cargo.toml", manifest("2024", "")), ("examples/e.rs", "fn main() {}\n".into())]),
        ("cratetype", vec![("Cargo.toml", manifest("2024", "[lib]\ncrate-type = [\"rlib\", \"bin\"]\n"))]),
        ("underscore", vec![("Cargo.toml", manifest("2021", "[lib]\ncrate_type = [\"rlib\", \"bin\"]\n"))]),
        ("cdylib", vec![("Cargo.toml", manifest("2024", "[lib]\ncrate-type = [\"cdylib\"]\n"))]),
        ("emptytype", vec![("Cargo.toml", manifest("2024", "[lib]\ncrate-type = []\n"))]),
        ("bench", vec![("Cargo.toml", manifest("2024", "")), ("benches/b.rs", "fn main() {}\n".into())]),
        ("build", vec![("Cargo.toml", manifest("2024", "")), ("build.rs", "fn main() {}\n".into())]),
        // Only tests left, no library (with no target at all cargo refuses the package).
        (
            "nolib",
            vec![
                ("Cargo.toml", manifest("2024", "autolib = false\n")),
                ("tests/t.rs", "#[test]\nfn t() {}\n".into()),
            ],
        ),
    ];
    let mut verdicts = Vec::new();
    for (case, files) in &cases {
        let mut all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        all.push(LIB);
        let dir = planted(case, &all);
        verdicts.push((*case, breaches(&targets(&dir)).is_empty()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert_eq!(
        verdicts,
        [
            ("plain", true),
            ("bin", false),
            ("example", false),
            ("main", false),
            ("srcbin", false),
            ("examples", false),
            ("cratetype", false),
            ("underscore", false),
            ("cdylib", false),
            ("emptytype", false),
            ("bench", false),
            ("build", false),
            ("nolib", false),
        ]
    );
}

/// Every file under `dir`, recursively, whatever its extension: rustc reads more than `.rs`
/// (`include!`, `#[path]`), and the manifest is part of the crate too.
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

/// The lines that name the vendor protocol or its terms, or cite a vendor document. Each line
/// is lowercased and stripped to letters and digits first, so no separator, case or spacing
/// hides a name; a section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> =
        ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"]
            .iter()
            .map(|b| rev(b))
            .collect();
    let section = char::from_u32(0xA7).unwrap();
    text.lines()
        .filter(|line| {
            let flat: String = line
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect();
            line.contains(section) || banned.iter().any(|b| flat.contains(b.as_str()))
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn core_names_no_vendor_and_cites_no_vendor_document() {
    // docs/questions.md Q8: what a device's telegrams mean is configuration, not core code.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    crate_files(&root, &mut files);
    assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "{files:?}");
    assert!(files.len() >= 10, "{files:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display()))
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The check sees every spelling, in any case, and the vendor's terms and citations.
    let samples = [
        format!("// the {} catalog", rev("tleBineG")),
        format!("const {}: u16 = 1;", rev("TROP_TLEB_INEG")),
        format!("// see {}/spec.txt", rev("RODNEV/SCOD")),
        format!("// the {} speaks", rev("lortnoC troS retsaM")),
        format!("// {} telegrams", rev("MAOOTXTR")),
        format!("// spec {}3.2", char::from_u32(0xA7).unwrap()),
        format!("// a {}", rev("ecafretni-tsoh")),
    ];
    for s in &samples {
        assert_eq!(vendor_mentions(s).len(), 1, "{s}");
    }
    assert_eq!(vendor_mentions("fn decode(frame: &[u8]) {}\n// a capture spec"), Vec::<String>::new());
}
