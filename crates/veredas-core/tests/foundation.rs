//! The foundation's own rules that no suite law checks (contract: the foundation clause; the
//! laws in `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library container, never an executable (`docs/core-plan.md`, "Rules"): cargo makes a
//! binary of `[[bin]]`, `src/main.rs`, `src/bin/` and of every example, so none may exist.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest() -> toml::Table {
    let text = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    text.parse::<toml::Table>().unwrap()
}

/// What in the manifest would make cargo build an executable.
fn executables_declared(m: &toml::Table) -> Vec<String> {
    let mut found = Vec::new();
    for key in ["bin", "example"] {
        if m.contains_key(key) {
            found.push(format!("[[{key}]]"));
        }
    }
    found
}

/// What on disk cargo would discover as an executable target.
fn executables_on_disk(dir: &Path) -> Vec<String> {
    ["src/main.rs", "src/bin", "examples"]
        .into_iter()
        .filter(|p| dir.join(p).exists())
        .map(String::from)
        .collect()
}

#[test]
fn core_is_a_library_and_builds_no_executable() {
    let m = manifest();
    assert_eq!(executables_declared(&m), Vec::<String>::new());
    assert_eq!(executables_on_disk(&root()), Vec::<String>::new());
    assert!(root().join("src/lib.rs").is_file(), "the library root");
    let package = m["package"].as_table().unwrap();
    assert_eq!(package["name"].as_str(), Some("veredas-core"));
    // A `[lib]` table may tune the library, never turn it into something else.
    if let Some(lib) = m.get("lib").and_then(|l| l.as_table()) {
        assert!(lib.get("crate-type").is_none_or(|t| t
            .as_array()
            .unwrap()
            .iter()
            .all(|k| k.as_str() == Some("lib") || k.as_str() == Some("rlib"))));
    }
}

#[test]
fn the_check_sees_every_way_to_an_executable() {
    let m: toml::Table = "[package]\nname = \"x\"\n[[bin]]\nname = \"a\"\n[[example]]\nname = \"b\"\n"
        .parse()
        .unwrap();
    assert_eq!(executables_declared(&m), ["[[bin]]", "[[example]]"]);

    let dir = std::env::temp_dir().join(format!("veredas-core-foundation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/bin")).unwrap();
    std::fs::create_dir_all(dir.join("examples")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    assert_eq!(executables_on_disk(&dir), ["src/main.rs", "src/bin", "examples"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
