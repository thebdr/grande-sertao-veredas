//! Core's layers by feature (contract: the features clause; `docs/core-plan.md`, "Features").
//!
//! Asked of cargo itself, as `tests/foundation.rs` asks it what core builds: the manifest's
//! features as cargo reads them, the dependency graph cargo resolves without the default
//! features, and the library cargo compiles in each form a component may ask for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn cargo(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO")).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "cargo {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn strings(v: &serde_json::Value) -> BTreeSet<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// The crates of a plant's stack: none of them may reach a component that asks for core without
/// its default features.
const PLANT_STACK: &[&str] = &[
    "async-opcua-client",
    "async-opcua-types",
    "async-opcua-server",
    "tokio",
    "socket2",
    "regex",
    "chrono",
    "mio",
];

#[test]
fn the_default_holds_the_plant_facing_layers_and_each_master_is_its_own() {
    let m = manifest();
    let meta: serde_json::Value = serde_json::from_str(&cargo(&[
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--offline",
        "--manifest-path",
        m.to_str().unwrap(),
    ]))
    .unwrap();
    let core = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "veredas-core")
        .unwrap();
    let f = &core["features"];
    assert_eq!(strings(&f["default"]), set(&["net"]));
    assert_eq!(
        strings(&f["net"]),
        set(&[
            "dep:async-opcua-client",
            "dep:async-opcua-types",
            "dep:chrono",
            "dep:regex",
            "dep:socket2",
            "dep:tokio"
        ])
    );
    assert_eq!(
        strings(&f["demo-server"]),
        set(&["net", "dep:async-opcua-server"])
    );
    assert_eq!(
        strings(&f["modbus"]),
        set(&["dep:tokio-modbus", "dep:tokio"])
    );
    assert_eq!(
        strings(&f["enip"]),
        set(&[
            "dep:ethernetip-core",
            "dep:ethernetip-connections",
            "dep:tokio"
        ])
    );
    // Every normal or build dependency is optional but the three the header reader stands on.
    let required: BTreeSet<String> = core["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| {
            (d["kind"].is_null() || d["kind"] == "build")
                && d["optional"] == serde_json::json!(false)
        })
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(required, set(&["serde", "thiserror", "toml"]));
}

#[test]
fn without_its_default_features_core_stands_on_serde_toml_and_thiserror_alone() {
    let m = manifest();
    let tree = |extra: &[&str]| -> (BTreeSet<String>, BTreeSet<String>) {
        let mut args = vec![
            "tree",
            "-p",
            "veredas-core",
            "-e",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "-f",
            "{p}",
        ];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["--offline", "--manifest-path", m.to_str().unwrap()]);
        let whole: BTreeSet<String> = cargo(&args)
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(String::from))
            .collect();
        args.extend_from_slice(&["--depth", "1"]);
        let direct: BTreeSet<String> = cargo(&args)
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(String::from))
            .filter(|n| n != "veredas-core")
            .collect();
        (whole, direct)
    };
    let (whole, direct) = tree(&["--no-default-features"]);
    assert_eq!(direct, set(&["serde", "thiserror", "toml"]));
    let reached: Vec<&&str> = PLANT_STACK.iter().filter(|c| whole.contains(**c)).collect();
    let opcua: Vec<&String> = whole
        .iter()
        .filter(|c| c.starts_with("async-opcua"))
        .collect();
    assert!(
        opcua.is_empty(),
        "OPC UA crates without the default features: {opcua:?}"
    );
    assert!(
        reached.is_empty(),
        "a plant's stack without the default features: {reached:?}"
    );
    // The default form holds it all, so every current user sees no change.
    let (whole, _) = tree(&[]);
    for c in [
        "async-opcua-client",
        "async-opcua-types",
        "tokio",
        "socket2",
        "regex",
        "chrono",
    ] {
        assert!(whole.contains(c), "{c} missing from the default form");
    }
    assert!(
        !whole.contains("async-opcua-server"),
        "the server's crate in the default form"
    );
    // Each master asked alone brings its own framing and a runtime, and neither the other's
    // framing nor any OPC UA, regex or chrono.
    for (master, own, other) in [
        (
            "modbus",
            &["tokio-modbus"][..],
            &["ethernetip-core", "ethernetip-connections"][..],
        ),
        (
            "enip",
            &["ethernetip-core", "ethernetip-connections"][..],
            &["tokio-modbus"][..],
        ),
    ] {
        let (whole, _) = tree(&["--no-default-features", "--features", master]);
        assert!(whole.contains("tokio"), "{master}: no runtime");
        for c in own {
            assert!(whole.contains(*c), "{master}: {c} missing");
        }
        for c in other
            .iter()
            .chain(&["async-opcua-client", "async-opcua-types", "regex", "chrono"])
        {
            assert!(!whole.contains(*c), "{master} alone reaches {c}");
        }
    }
}

/// The library compiles in each form: the default alone, without its default features (the
/// header reader, the PCG32, the state text), and with each fieldbus master alone and both. Built in a folder of the test's
/// own, so the run's build folder is not shared.
#[test]
fn the_library_compiles_without_its_default_features_and_with_the_masters_alone() {
    let m = manifest();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check");
    for extra in [
        // The default form alone: every other build of core in the suite also asks for the demo
        // plant or a master (core's own tests, maquinar, mestres), which would hide a layer of
        // `net` standing on a crate `net` does not hold.
        &[][..],
        &["--no-default-features"][..],
        &["--no-default-features", "--features", "modbus"][..],
        &["--no-default-features", "--features", "enip"][..],
        &["--no-default-features", "--features", "modbus,enip"][..],
    ] {
        let mut args = vec![
            "check",
            "-p",
            "veredas-core",
            "--lib",
            "--offline",
            "--manifest-path",
            m.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let out = Command::new(env!("CARGO"))
            .args(&args)
            .env("CARGO_TARGET_DIR", &dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cargo {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A component asking for core without its default features gets the header reader, the PCG32
/// and the state text, and they work: a consumer crate of its own, written here and run, reads a
/// stamp and refuses three, writes and reads a state back, and draws the same numbers twice. Core's
/// own tests always see `net` (the self dev-dependency asks for the demo plant), so only a crate
/// apart can show the slim form's contents.
#[test]
fn a_consumer_without_the_default_features_reads_stamps_states_and_draws() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("slim-consumer");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            "[package]\nname = \"slim-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
             [workspace]\n\n[dependencies]\nveredas-core = {{ path = {:?}, default-features = false }}\n",
            core.to_str().unwrap()
        ),
    )
    .unwrap();
    // The workspace's lock, so the same versions resolve offline.
    std::fs::copy(core.join("../../Cargo.lock"), root.join("Cargo.lock")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        r#"use veredas_core::rng::Pcg32;
use veredas_core::schema::{self, HeaderError};
use veredas_core::state::ControllerState;

fn main() {
    let good = "format = \"veredas-x\"\nversion = 1\n[body]\nk = 2\n";
    assert!(schema::check(good, "an x file", "veredas-x", 1).is_ok());
    assert!(matches!(
        schema::check("format = \"veredas-x\"\nversion = 2\n", "an x file", "veredas-x", 1),
        Err(HeaderError::UnknownVersion { found: 2, .. })
    ));
    assert!(matches!(schema::check("version = 1\n", "an x file", "veredas-x", 1), Err(HeaderError::NoFormat { .. })));
    assert!(matches!(schema::check("= =", "an x file", "veredas-x", 1), Err(HeaderError::NotToml { .. })));
    let stamp = schema::stamp(good, "an x file").unwrap();
    assert_eq!((stamp.format.as_str(), stamp.version), ("veredas-x", 1));
    for text in ["RUN", "STOP command(id=101, source=\"oficina\")"] {
        let state: ControllerState = text.parse().unwrap();
        assert_eq!(state.to_string(), text);
    }
    let draws = |seed| {
        let mut r = Pcg32::new(seed, 1);
        (0..4).map(|_| r.next_u32()).collect::<Vec<u32>>()
    };
    assert_eq!(draws(7), draws(7));
    assert_ne!(draws(7), draws(8));
    println!("slim core works");
}
"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO"))
        .args(["run", "--offline", "--quiet", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "slim core works"
    );
}

/// A Rust source with its comments and the contents of its string and character literals blanked
/// out, so only code is left to read.
fn code_only(src: &str) -> String {
    lexed(src, false)
}

/// A Rust source with its comments taken out and its literals kept as written.
fn without_comments(src: &str) -> String {
    lexed(src, true)
}

fn lexed(src: &str, keep_literals: bool) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let ident_before = |i: usize| i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_');
    while i < c.len() {
        if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if c[i] == 'r'
            && !ident_before(i)
            && matches!(c.get(i + 1), Some('"') | Some('#'))
            && c[i + 1..].iter().find(|x| **x != '#') == Some(&'"')
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while c.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            j += 1;
            while j < c.len()
                && !(c[j] == '"'
                    && c[j + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|x| **x == '#')
                        .count()
                        == hashes)
            {
                j += 1;
            }
            let end = (j + 1 + hashes).min(c.len());
            if keep_literals {
                out.extend(&c[i..end]);
            } else {
                out.push_str("\"\"");
            }
            i = end;
        } else if c[i] == '"' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '"' {
                if c[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(c.len());
            if keep_literals {
                out.extend(&c[i..end]);
            } else {
                out.push_str("\"\"");
            }
            i = end;
        } else if c[i] == '\'' && c.get(i + 1) == Some(&'\\') {
            let mut j = i + 2;
            while j < c.len() && c[j] != '\'' {
                j += 1;
            }
            let end = (j + 1).min(c.len());
            if keep_literals {
                out.extend(&c[i..end]);
            } else {
                out.push_str("' '");
            }
            i = end;
        } else if c[i] == '\'' && c.get(i + 2) == Some(&'\'') {
            if keep_literals {
                out.extend(&c[i..i + 3]);
            } else {
                out.push_str("' '");
            }
            i += 3;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// Every path into core the other members' sources name, read from their files: a `use` tree
/// expanded (`veredas_core::schema::{self, HeaderError}` is `schema` and `schema::HeaderError`), a
/// path in an expression or a type up to its first capitalized segment (the type, never its
/// associated items), comments skipped, globs left out. Those under a layer the default form does
/// not hold (`opc::demo`, `opc::server`, a master's module) are left to their own forms.
fn members_default_paths() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![root.join("crates"), root.join("tools")];
    let mut text = String::new();
    while let Some(path) = files.pop() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name == "veredas-core" || name == "target" || name.starts_with('.') {
                continue;
            }
            files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        } else if name.ends_with(".rs") {
            text.push_str(&code_only(&std::fs::read_to_string(&path).unwrap()));
            text.push('\n');
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let ident = |i: &mut usize| -> String {
        let start = *i;
        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
            *i += 1;
        }
        chars[start..*i].iter().collect()
    };
    let skip = |i: &mut usize| {
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
    };
    fn tree(
        chars: &[char],
        i: &mut usize,
        prefix: Vec<String>,
        out: &mut Vec<Vec<String>>,
        ident: &dyn Fn(&mut usize) -> String,
        skip: &dyn Fn(&mut usize),
    ) {
        skip(i);
        if chars.get(*i) == Some(&'{') {
            *i += 1;
            loop {
                skip(i);
                match chars.get(*i) {
                    Some('}') => {
                        *i += 1;
                        return;
                    }
                    Some(',') => *i += 1,
                    None => return,
                    _ => {
                        let before = *i;
                        tree(chars, i, prefix.clone(), out, ident, skip);
                        if *i == before {
                            *i += 1;
                        }
                    }
                }
            }
        }
        if chars.get(*i) == Some(&'*') {
            *i += 1;
            return;
        }
        let seg = ident(i);
        if seg.is_empty() {
            return;
        }
        let path = if seg == "self" {
            prefix
        } else {
            [prefix, vec![seg]].concat()
        };
        skip(i);
        if chars.get(*i) == Some(&':') && chars.get(*i + 1) == Some(&':') {
            *i += 2;
            tree(chars, i, path, out, ident, skip);
            return;
        }
        let at = *i;
        if ident(i) == "as" {
            skip(i);
            let _ = ident(i);
            if chars.get(*i) == Some(&'_') {
                *i += 1;
            }
        } else {
            *i = at;
        }
        out.push(path);
    }
    let needle: Vec<char> = "veredas_core::".chars().collect();
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()] != needle[..]
            || i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
        {
            i += 1;
            continue;
        }
        let before: String = chars[..i]
            .iter()
            .rev()
            .take(12)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let in_use = before.trim_end().ends_with("use");
        i += needle.len();
        if in_use {
            tree(&chars, &mut i, Vec::new(), &mut paths, &ident, &skip);
        } else {
            let mut path = Vec::new();
            loop {
                let seg = ident(&mut i);
                if seg.is_empty() {
                    break;
                }
                let capital = seg.starts_with(|c: char| c.is_ascii_uppercase());
                path.push(seg);
                if capital || !(chars.get(i) == Some(&':') && chars.get(i + 1) == Some(&':')) {
                    break;
                }
                i += 2;
            }
            paths.push(path);
        }
    }
    paths
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.join("::"))
        .filter(|p| {
            ![
                "opc::demo",
                "opc::server",
                "fieldbus::modbus",
                "fieldbus::enip",
            ]
            .iter()
            .any(|l| p == l || p.starts_with(&format!("{l}::")))
        })
        .collect()
}

/// Each form a member asks for holds the layers its form promises, named by a consumer crate of
/// the test's own and checked by cargo: the default (every plant-facing layer), the default with
/// the demo plant (maquinar's form), each master alone. Every build of core inside the suite merges
/// these forms (core's own tests and the workspace ask for all of them at once), so only a crate
/// apart sees one form as a member that asks for it alone does.
#[test]
fn each_form_holds_the_layers_it_promises() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("form-consumers");
    let forms: [(&str, &str, &str); 4] = [
        (
            "default-form",
            "",
            "veredas_core::Quality, veredas_core::Sample, veredas_core::SeriesRef, veredas_core::SeriesType, \
             veredas_core::SourceId, veredas_core::Value, veredas_core::SourceError, veredas_core::SourceHandle, \
             veredas_core::types::Value, veredas_core::types::Sample, veredas_core::source::SourceHandle, \
             veredas_core::telegram::Channel, veredas_core::telegram::Catalog, veredas_core::opc::Browser, \
             veredas_core::opc::OpcWorker, veredas_core::schema::Stamp, veredas_core::state::ControllerState, \
             veredas_core::rng::Pcg32",
        ),
        (
            "demo-form",
            ", features = [\"demo-server\"]",
            "veredas_core::opc::demo::DemoPlant, veredas_core::opc::demo::Relay, veredas_core::opc::Browser",
        ),
        (
            "modbus-form",
            ", default-features = false, features = [\"modbus\"]",
            "veredas_core::fieldbus::modbus::Client, veredas_core::schema::Stamp",
        ),
        (
            "enip-form",
            ", default-features = false, features = [\"enip\"]",
            "veredas_core::fieldbus::enip::Scanner, veredas_core::schema::Stamp",
        ),
    ];
    for (name, spec, items) in forms {
        let root = base.join(name);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
                 [workspace]\n\n[dependencies]\nveredas-core = {{ path = {:?}{spec} }}\n",
                core.to_str().unwrap()
            ),
        )
        .unwrap();
        std::fs::copy(core.join("../../Cargo.lock"), root.join("Cargo.lock")).unwrap();
        let names: Vec<String> = items
            .split(", ")
            .map(|t| format!("    let _ = std::any::type_name::<{}>();", t.trim()))
            .collect();
        // The default form also takes every path into its layers the other members' sources name,
        // read from their files at each run (`members_default_paths`).
        let uses: Vec<String> = if name == "default-form" {
            let paths = members_default_paths();
            assert!(paths.len() > 60, "{paths:?}");
            paths
                .iter()
                .map(|p| format!("use veredas_core::{p} as _;"))
                .collect()
        } else {
            Vec::new()
        };
        std::fs::write(
            root.join("src/main.rs"),
            format!(
                "#![allow(unused_imports)]\n{}\nfn main() {{\n{}\n}}\n",
                uses.join("\n"),
                names.join("\n")
            ),
        )
        .unwrap();
        // One build folder for every consumer, the one the compile check above uses, so the
        // dependencies each form shares are compiled once.
        let out = Command::new(env!("CARGO"))
            .args(["check", "--offline", "--quiet", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .env(
                "CARGO_TARGET_DIR",
                Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check"),
            )
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{name} ({spec}): {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Every feature gate in core's source, as it is ordinarily written, is one of the expected set:
/// `net` on the four plant-facing modules and the crate root's two re-exports, `demo-server` on the
/// demo plant, each master on its own module; and nothing hides an item from rustdoc. A gate
/// written another way (a comment inside it, a macro building it, a file pulled in by `include!`
/// or `#[path]`) escapes a reading of the text; the compiler's own view, the next test, is what
/// closes the class for every item rustdoc sees.
#[test]
fn every_feature_gate_is_where_it_belongs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.clone()];
    let mut gates: BTreeSet<String> = BTreeSet::new();
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // The code alone, comments taken out, every attribute joined onto one line, so a gate
        // split by a comment or a line break reads as one.
        let code = without_comments(&std::fs::read_to_string(&path).unwrap());
        let mut text = String::with_capacity(code.len());
        let mut depth = 0usize;
        let chars: Vec<char> = code.chars().collect();
        for (k, ch) in chars.iter().enumerate() {
            let opens = (k > 0 && chars[k - 1] == '#')
                || (k > 1 && chars[k - 1] == '!' && chars[k - 2] == '#');
            match ch {
                '[' if depth > 0 || opens => depth += 1,
                ']' if depth > 0 => depth -= 1,
                '\n' if depth > 0 => {
                    text.push(' ');
                    continue;
                }
                _ => {}
            }
            text.push(*ch);
        }
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            // A mention of a feature in a cfg, an attribute or the macro; and what would make
            // rustdoc see an item otherwise than the build (`doc(hidden)`, a cfg on `doc`), of
            // which core has none.
            // A raw identifier (`r#doc`) is the identifier itself.
            let squeezed: String = line
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .replace("r#", "");
            let hides = squeezed.contains("doc(hidden")
                || ["cfg(doc", "(doc)", "(doc,", ",doc)", ",doc,"]
                    .iter()
                    .any(|p| squeezed.contains(p));
            if squeezed.contains("feature=") || squeezed.contains("cfg!(") || hides {
                let gated = lines[n + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.starts_with("#[") && !l.starts_with("//"))
                    .unwrap_or("");
                let item: String = gated
                    .split(['{', ';'])
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                gates.insert(format!("{rel}: {squeezed} -> {item}"));
            }
        }
    }
    let want: BTreeSet<String> = [
        "fieldbus/mod.rs: #[cfg(feature=\"enip\")] -> pub mod enip",
        "fieldbus/mod.rs: #[cfg(feature=\"modbus\")] -> pub mod modbus",
        "lib.rs: #[cfg(feature=\"net\")] -> pub mod opc",
        "lib.rs: #[cfg(feature=\"net\")] -> pub mod source",
        "lib.rs: #[cfg(feature=\"net\")] -> pub mod telegram",
        "lib.rs: #[cfg(feature=\"net\")] -> pub mod types",
        "lib.rs: #[cfg(feature=\"net\")] -> pub use source::",
        "lib.rs: #[cfg(feature=\"net\")] -> pub use types::",
        "opc/mod.rs: #[cfg(feature=\"demo-server\")] -> pub mod demo",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(gates, want);
}

/// The twenty forms core can be asked for: the plant-facing layer off, on, or on with the demo
/// plant, each with `net` with the feature `default` or without it (a member that keeps the
/// default features gets `default` too, and a gate may name it); each master off or on. The last
/// is every feature, the form core's own tests run in.
fn forms() -> Vec<Vec<&'static str>> {
    let mut forms = Vec::new();
    for net in [
        &[][..],
        &["net"][..],
        &["net", "default"][..],
        &["net", "demo-server"][..],
        &["net", "demo-server", "default"][..],
    ] {
        for modbus in [&[][..], &["modbus"][..]] {
            for enip in [&[][..], &["enip"][..]] {
                forms.push([net, modbus, enip].concat());
            }
        }
    }
    forms
}

/// A consumer crate of the test's own asking for core in `form` alone, with `main` as its source,
/// in the build folder every consumer here shares.
fn consumer(name: &str, form: &[&str], main: &str) -> PathBuf {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("form-consumers")
        .join(name);
    std::fs::create_dir_all(root.join("src")).unwrap();
    let features: Vec<String> = form.iter().map(|f| format!("{f:?}")).collect();
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
             [workspace]\n\n[dependencies]\nveredas-core = {{ path = {:?}, default-features = false, features = [{}] }}\n",
            core.to_str().unwrap(),
            features.join(", ")
        ),
    )
    .unwrap();
    std::fs::copy(core.join("../../Cargo.lock"), root.join("Cargo.lock")).unwrap();
    std::fs::write(root.join("src/main.rs"), main).unwrap();
    root
}

/// Every string literal of a Rust source, plain or raw, that starts with a capital letter, as
/// written (escapes and hashes kept), so it can be pasted into another source.
fn capitalized_literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' if chars.get(i + 1) == Some(&'\\') => {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '\'' if chars.get(i + 2) == Some(&'\'') => i += 3,
            'r' if matches!(chars.get(i + 1), Some('#') | Some('"'))
                && !chars
                    .get(i.wrapping_sub(1))
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_') =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                let close: Vec<char> = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                while j < chars.len() && chars[j..].iter().take(close.len()).ne(close.iter()) {
                    j += 1;
                }
                // A raw string is kept as written too: the state lines with quoted arguments
                // are raw in the state-text tests.
                let end = (j + close.len()).min(chars.len());
                let body_start = i + 1 + hashes + 1;
                if chars
                    .get(body_start)
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    found.push(chars[i..end].iter().collect());
                }
                i = end;
            }
            '"' => {
                let byte = i > 0 && chars[i - 1] == 'b';
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let lit: String = chars[i..=j.min(chars.len() - 1)].iter().collect();
                if !byte && chars.get(i + 1).is_some_and(|c| c.is_ascii_uppercase()) {
                    found.push(lit);
                }
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    found
}

/// The slim layer behaves the same in every form: a probe crate asking for core in each of the
/// twenty forms alone, run, calls every public function and reads every public table of `schema`,
/// `rng` and `state` (bar the PCG32's seeding from entropy) on the inputs listed here and prints
/// what each answers. That covers every stamp and check of a set of headers with the refusals'
/// texts; every state line of core's own state-text tests read, written back and asked its state,
/// reasons, health and conditions; every health and maintenance request; and PCG32 draws from
/// fixed seeds with the edges of each range. All twenty must print the same, so the form core's
/// own tests run in (every feature) speaks for each. A gate that changes what an item does
/// without changing what rustdoc renders of it (a constant's value, a derive's attribute, a
/// function's body) shows here as a form that prints otherwise, for these calls and inputs. For
/// the plant-facing layer and the masters, core's own tests hold their behaviour in the merged
/// form alone.
#[test]
fn the_slim_layer_behaves_the_same_in_every_form() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lines =
        capitalized_literals(&std::fs::read_to_string(core.join("tests/state_text.rs")).unwrap());
    assert!(lines.len() > 120, "{} state lines", lines.len());
    let headers = [
        "format = \"veredas-x\"\nversion = 1\n[body]\nk = 2\n",
        "format = \"veredas-x\"\nversion = 2\n",
        "version = 1\n",
        "format = \"veredas-x\"\n",
        "format = 3\nversion = 1\n",
        "format = \"veredas-x\"\nversion = \"1\"\n",
        "format = \"veredas-x\"\nversion = -1\n",
        "format = \"veredas-x\"\nversion = 0\n",
        "format = \"veredas-x\"\nversion = 4294967296\n",
        "format = \"veredas-y\"\nversion = 1\n",
        "[body]\nformat = \"veredas-x\"\nversion = 1\n",
        "format = \"veredas-x\"\nversion = 1\n[[t]]\nk = 18446744073709551615\n",
        "= =",
        "",
        "format = \"veredas-x\"\r\nversion = 1\r\n",
        "\u{feff}format = \"veredas-x\"\nversion = 1\n",
        "format = \"veredas-x\"\nversion = 1.0\n",
        "format = \"\"\nversion = 1\n",
        "format = \"veredas-x\"\nversion = 4294967295\n",
        "format = \"veredas-x\"\nformat = \"veredas-x\"\nversion = 1\n",
        "format = [\"veredas-x\"]\nversion = 1\n",
        "  format   =   \"veredas-x\"  \n\n  version=1",
    ];
    let main = format!(
        r##"use std::fmt::Write;
use veredas_core::rng::Pcg32;
use veredas_core::schema;
use veredas_core::state::{{
    self, CommandSource, ControllerState, Health, MaintenanceRequest, State, StateWord,
}};

const HEADERS: &[&str] = &{headers:?};
const LINES: &[&str] = &[
{lines}
];

fn main() {{
    let mut out = String::new();
    for h in HEADERS {{
        let _ = writeln!(out, "stamp {{h:?}} = {{:?}}", schema::stamp(h, "an x file").map_err(|e| (e.to_string(), e)));
        for (f, v) in [("veredas-x", 1), ("veredas-x", 2), ("veredas-y", 1), ("veredas-x", 0), ("veredas-x", u32::MAX), ("", 1)] {{
            let r = schema::check(h, "an x file", f, v).map_err(|e| (e.to_string(), e));
            let _ = writeln!(out, "check {{h:?}} {{f}} {{v}} = {{r:?}}");
        }}
    }}
    let _ = writeln!(out, "words {{:?}}", StateWord::ALL.map(|w| (w, w.number(), w.word())));
    let _ = writeln!(out, "sources {{:?}}", CommandSource::ALL.map(|c| (c, c.word())));
    let _ = writeln!(out, "reasons {{:?}}", state::REASONS);
    let _ = writeln!(out, "phases {{:?}}", state::PHASES);
    let _ = writeln!(out, "maintenance {{:?}}", state::MAINTENANCE_REASONS);
    for n in 0..=255u8 {{
        let _ = writeln!(out, "number {{n}} {{:?}}", StateWord::from_number(n));
    }}
    for n in 0..1000u16 {{
        if let Some(r) = state::reason_entry(n) {{
            let _ = writeln!(out, "reason {{n}} {{r:?}}");
        }}
    }}
    for w in ["RUN", "STOP", "HOLD", "FAULT", "STARTUP", "MAINTENANCE", "run", "oficina", "plc", "X", ""] {{
        let _ = writeln!(out, "word {{w:?}} {{:?}} {{:?}}", StateWord::from_word(w), CommandSource::from_word(w));
    }}
    for w in StateWord::ALL {{
        let _ = writeln!(out, "word {{w}} {{:?}}", StateWord::from_word(&w.to_string()));
    }}
    for c in CommandSource::ALL {{
        let _ = writeln!(out, "source {{:?}}", CommandSource::from_word(c.word()));
    }}
    for r in state::REASONS {{
        let _ = writeln!(out, "reason state {{:?}} {{:?}}", r, r.state());
    }}
    for h in [Health::Normal, Health::Failure, Health::FunctionCheck, Health::MaintenanceRequired] {{
        let _ = writeln!(out, "health {{h:?}} {{}} {{h}}", h.device_health());
    }}
    for m in [
        MaintenanceRequest::FaultOnUpdate,
        MaintenanceRequest::Device {{ urgent: false }},
        MaintenanceRequest::Device {{ urgent: true }},
        MaintenanceRequest::DeviceMissing,
        MaintenanceRequest::Overruns,
        MaintenanceRequest::RetainReset,
        MaintenanceRequest::User {{ urgent: false }},
        MaintenanceRequest::User {{ urgent: true }},
    ] {{
        let _ = writeln!(out, "maintenance {{m:?}} {{}} {{}} {{}}", m.number(), m.word(), m.urgent());
    }}
    for st in [
        State::Unknown,
        State::Run,
        State::Stop(None),
        State::Startup {{ reason: None, phase: None }},
        State::Hold(None),
        State::Fault(None),
    ] {{
        let c = ControllerState::new(st.clone());
        let _ = writeln!(out, "new {{st:?}} {{c}} {{:?}} {{:?}} {{:?}}", c.health(), st.word(), st.reason_number());
    }}
    for l in LINES {{
        match l.parse::<ControllerState>() {{
            Ok(s) => {{
                let _ = writeln!(out, "line {{l:?}} = {{s:?}} / {{s}} / {{:?}} / {{}}", s.health(), s.clone().normalized());
                let again = s.to_string().parse::<ControllerState>();
                let _ = writeln!(out, "  again {{:?}} {{}}", again, again.as_ref().map(|a| a == &s).unwrap_or(false));
                let st = &s.state;
                let reason = match st {{
                    State::Stop(r) => format!("{{:?}}", r.as_ref().map(|r| (r.number(), r.word()))),
                    State::Startup {{ reason, phase }} => format!(
                        "{{:?}} {{:?}}",
                        reason.as_ref().map(|r| (r.number(), r.word())),
                        phase.as_ref().map(|p| (p.number(), p.word()))
                    ),
                    State::Hold(r) => format!("{{:?}}", r.as_ref().map(|r| (r.number(), r.word()))),
                    State::Fault(r) => format!("{{:?}}", r.as_ref().map(|r| (r.number(), r.word()))),
                    State::Unknown | State::Run => "-".into(),
                }};
                let health = s.health().map(|h| (h.device_health(), h.to_string()));
                let maintenance: Vec<_> = s.maintenance.iter().map(|m| (m.number(), m.word(), m.urgent())).collect();
                let _ = writeln!(
                    out,
                    "  {{}} {{:?}} {{reason}} {{}} {{}} {{health:?}} {{maintenance:?}} {{}}",
                    st.word(),
                    st.reason_number(),
                    s.conditions.force.any(),
                    s.conditions.debug(),
                    ControllerState::new(st.clone())
                );
            }}
            Err(e) => {{
                let _ = writeln!(out, "line {{l:?}} ! {{e:?}} / {{e}}");
            }}
        }}
    }}
    for (seed, stream) in [(0u64, 0u64), (1, 1), (7, 54), (42, 3), (u64::MAX, u64::MAX)] {{
        let mut r = Pcg32::new(seed, stream);
        let u: Vec<u32> = (0..8).map(|_| r.next_u32()).collect();
        let f: Vec<u64> = (0..4).map(|_| r.next_f64().to_bits()).collect();
        let g: Vec<u64> = (0..4).map(|_| r.uniform(-5.0, 5.0, 2).to_bits()).collect();
        let i: Vec<i64> = (0..6).map(|_| r.int_in(-3, 3)).collect();
        let b: Vec<bool> = (0..8).map(|_| r.flip()).collect();
        let _ = writeln!(out, "pcg {{seed}} {{stream}} {{u:?}} {{f:?}} {{g:?}} {{i:?}} {{b:?}}");
        // The edges: bounds reversed, equal, not finite or far apart, places none or many.
        for (lo, hi, d) in [
            (f64::NAN, 1.0, 2), (1.0, f64::NAN, 2), (5.0, -5.0, 2), (0.0, 0.0, 2),
            (f64::NEG_INFINITY, f64::INFINITY, 2), (-1e300, 1e300, 0), (0.0, 1.0, 0),
            (0.0, 1.0, 20), (0.0, 1.0, 255), (-0.0, 0.0, 3),
        ] {{
            let _ = writeln!(out, "uniform {{lo}} {{hi}} {{d}} {{}}", r.uniform(lo, hi, d).to_bits());
        }}
        for (lo, hi) in [(3, -3), (i64::MIN, i64::MAX), (0, 0), (-1, 0), (i64::MAX - 1, i64::MAX), (i64::MIN, i64::MIN + 1)] {{
            let _ = writeln!(out, "int_in {{lo}} {{hi}} {{}}", r.int_in(lo, hi));
        }}
    }}
    print!("{{out}}");
}}
"##,
        lines = lines.join(",\n")
    );
    let forms = forms();
    let mut printed = Vec::new();
    for (n, form) in forms.iter().enumerate() {
        let root = consumer(&format!("probe-{n}"), form, &main);
        let out = Command::new(env!("CARGO"))
            .args(["run", "--offline", "--quiet", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .env(
                "CARGO_TARGET_DIR",
                Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check"),
            )
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "the probe of {form:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // What it writes to stderr is part of what it does.
        printed.push(format!(
            "{}\n-- stderr --\n{}",
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap()
        ));
    }
    let merged = printed.last().unwrap();
    assert!(
        merged.lines().count() > 400,
        "{} lines",
        merged.lines().count()
    );
    assert!(merged.contains("line \"RUN\" = ") && merged.contains("UnknownVersion"));
    for (form, out) in forms.iter().zip(&printed) {
        if out != merged {
            let first = out
                .lines()
                .zip(merged.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("{a}\n  where every feature prints\n{b}"))
                .unwrap_or_else(|| "a different number of lines".into());
            panic!("the slim layer in {form:?} behaves otherwise:\n{first}");
        }
    }
}

/// Text with its tags stripped and its whitespace collapsed, each link's title kept with its
/// text (`‹struct core::time::Duration› Duration`): rustdoc names a type's full path there, so two
/// types of one name are told apart.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
                if tag.starts_with("a ")
                    && let Some(t) = tag
                        .split("title=\"")
                        .nth(1)
                        .and_then(|t| t.split('"').next())
                {
                    out.push_str(&format!("‹{t}› "));
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What one rustdoc page shows of core, by key: the item's declaration (`item-decl`); every
/// item's anchor with what it renders (an impl's or a method's header, a field with its type, a
/// variant, a re-export's line), and every anchor of the page's frame as a key alone;
/// and every row of a module's table by the page it points to, with what it shows (the name and
/// the mark of restricted visibility), and every other line of a table (a glob's re-export) by
/// what it shows. Left out, as not core's: the blanket impls, the page's last section, which show
/// whichever crates are in the graph (a master's framing brings tracing's `Instrument` onto every
/// type), and the tooltips of notable traits. Rustdoc makes every id on a page unique and puts the
/// blanket impls' heading only on a type's page, once; a page where either fails holds an id
/// written in a doc comment, which could hide what follows it, and is refused.
fn page_view(page: &str, html: &str) -> Vec<(String, String)> {
    const BLANKET: &str = "<h2 id=\"blanket-implementations\" class=\"section-header\">";
    let (dir, file) = page.rsplit_once('/').unwrap_or(("", page));
    let typed = ["struct.", "enum.", "union.", "type.", "primitive."]
        .iter()
        .any(|k| file.starts_with(k));
    let html = match html.matches("id=\"blanket-implementations\"").count() {
        0 => html,
        1 if typed && html.contains(BLANKET) => &html[..html.find(BLANKET).unwrap()],
        n => panic!(
            "{page}: the blanket impls' id {n} times, or not as rustdoc writes it: an id in a doc comment"
        ),
    };
    let mut ids = BTreeSet::new();
    let mut keys = Vec::new();
    if let Some(i) = html.find("<pre class=\"rust item-decl\">") {
        let decl = &html[i..];
        let end = decl.find("</pre>").unwrap_or(decl.len());
        keys.push(("item-decl".to_string(), plain(&decl[..end])));
    }
    let mut at = 0;
    while let Some(found) = html[at..].find(" id=\"") {
        let start = at + found;
        at = start + 5;
        let id = html[at..].split('"').next().unwrap().to_string();
        assert!(
            ids.insert(id.clone()),
            "{page}: the id `{id}` twice: an id in a doc comment"
        );
        if id == "notable-traits-data" {
            continue;
        }
        let open = html[..start].rfind('<').unwrap();
        let tag: String = html[open + 1..start]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let body_start = start + html[start..].find('>').map(|i| i + 1).unwrap_or(0);
        // An item's anchor (`method.x`, `structfield.x`, `variant.X`, `reexport.X`, `impl-..`)
        // keeps what it renders; the page's frame (`main-content`, the contents, a section's
        // heading) is a key alone, its prose and its lists not being declarations.
        let item = id.contains('.') || id.starts_with("impl-");
        let text = match tag.as_str() {
            "section" | "span" | "dt" | "h3" | "h4" if item => {
                let close = format!("</{tag}>");
                let body = &html[body_start..];
                plain(&body[..body.find(&close).unwrap_or(body.len())])
            }
            _ => String::new(),
        };
        keys.push((id, text));
    }
    // A module's table: each row by the page it points to, resolved from the module's folder;
    // a line that points to no page of its own (a glob's re-export) by what it shows.
    let mut at = 0;
    while let Some(found) = html[at..].find("<dt") {
        let start = at + found;
        let row = &html[start..];
        let end = row.find("</dt>").unwrap_or(row.len());
        at = start + 3;
        if !row.starts_with("<dt>") {
            // `<dt id=..>`: a re-export, keyed by its id above.
            continue;
        }
        if !row.starts_with("<dt><a ") {
            let text = plain(&row[..end]);
            keys.push((format!("dt:{text}"), text));
            continue;
        }
        let Some(h) = row[..end].find("href=\"") else {
            continue;
        };
        let href = row[h + 6..].split('"').next().unwrap();
        if href.contains("://") {
            continue;
        }
        let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        for seg in href.split('/') {
            match seg {
                ".." => {
                    parts.pop();
                }
                "." | "" => {}
                other => parts.push(other),
            }
        }
        keys.push((format!("row:{}", parts.join("/")), plain(&row[..end])));
    }
    keys
}

/// The library name of every crate the workspace's lock holds, core's own left out.
fn lock_crates() -> BTreeSet<String> {
    let m = manifest();
    let meta: serde_json::Value = serde_json::from_str(&cargo(&[
        "metadata",
        "--format-version",
        "1",
        "--offline",
        "--manifest-path",
        m.to_str().unwrap(),
    ]))
    .unwrap();
    let mut names = BTreeSet::new();
    for package in meta["packages"].as_array().unwrap() {
        for target in package["targets"].as_array().unwrap() {
            let kinds = strings(&target["kind"]);
            if kinds
                .iter()
                .any(|k| ["lib", "rlib", "proc-macro"].contains(&k.as_str()))
            {
                names.insert(target["name"].as_str().unwrap().replace('-', "_"));
            }
        }
    }
    names.remove("veredas_core");
    assert!(
        names.contains("regex") && names.contains("tokio"),
        "{names:?}"
    );
    names
}

/// Rustdoc's view of one form of core, with private items or without them: every page and every
/// key a page shows ([`page_view`]), as `page` and `page#key`, each with what it renders.
/// `all.html` is kept as a page only: its sections mirror the pages.
fn rustdoc_view(features: &[&str], private: bool) -> BTreeMap<String, String> {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check");
    // Rustdoc links a dependency's type, with its full path as the link's title, only when a
    // folder of that crate's name stands in the doc folder; without one it writes the bare name,
    // and two dependencies' `Error`s read the same. A folder for every crate of the lock.
    for name in lock_crates() {
        std::fs::create_dir_all(target.join("doc").join(name)).unwrap();
    }
    let list = features.join(",");
    let (docs, args): (PathBuf, Vec<String>) = if private {
        // Core's own pages, private items included: rustdoc reads core's source, under `cfg(doc)`.
        let m = manifest();
        let mut args: Vec<String> = [
            "doc",
            "-p",
            "veredas-core",
            "--no-deps",
            "--lib",
            "--offline",
            "--quiet",
            "--no-default-features",
            "--document-private-items",
            "--manifest-path",
            m.to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec();
        if !features.is_empty() {
            args.extend(["--features".to_string(), list.clone()]);
        }
        (target.join("doc").join("veredas_core"), args)
    } else {
        // The public pages as the build holds them: a facade crate asking for core in this form
        // re-exports all of it inlined, so rustdoc renders core's items from core's compiled
        // metadata, where `cfg(doc)` was never set, not from its source.
        std::fs::create_dir_all(target.join("doc").join("veredas_core")).unwrap();
        let root = consumer("facade", features, "");
        let lib = std::fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .replace(
                "[dependencies]",
                "[lib]\npath = \"src/lib.rs\"\n\n[dependencies]",
            );
        std::fs::write(root.join("Cargo.toml"), lib).unwrap();
        let _ = std::fs::remove_file(root.join("src/main.rs"));
        std::fs::write(
            root.join("src/lib.rs"),
            "#[doc(inline)]\npub use veredas_core::*;\n",
        )
        .unwrap();
        let args: Vec<String> = [
            "doc",
            "--no-deps",
            "--lib",
            "--offline",
            "--quiet",
            "--manifest-path",
            root.join("Cargo.toml").to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec();
        (target.join("doc").join("facade"), args)
    };
    let _ = std::fs::remove_dir_all(&docs);
    let out = Command::new(env!("CARGO"))
        .args(&args)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cargo {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut found = BTreeMap::new();
    let mut dirs = vec![docs.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("html") {
                continue;
            }
            let rel = p
                .strip_prefix(&docs)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel != "all.html" {
                let html = std::fs::read_to_string(&p).unwrap();
                for (key, text) in page_view(&rel, &html) {
                    found.insert(format!("{rel}#{key}"), text);
                }
            }
            found.insert(rel, String::new());
        }
    }
    found
}

/// The paths a consumer names for every public item a view shows: each item's page
/// (`opc/struct.Browser.html` is `opc::Browser`), each module's, and each re-export's line.
fn public_paths(view: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for key in view.keys() {
        let (page, anchor) = key.split_once('#').unwrap_or((key.as_str(), ""));
        let (dir, file) = page.rsplit_once('/').unwrap_or(("", page));
        let module: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        let name = if file == "index.html" {
            match anchor.strip_prefix("reexport.") {
                Some(n) => Some(n),
                None if anchor.is_empty() => None,
                None => continue,
            }
        } else if anchor.is_empty() && file != "all.html" {
            file.strip_suffix(".html")
                .and_then(|f| f.split_once('.'))
                .map(|(_, n)| n)
        } else {
            continue;
        };
        let mut path = module.clone();
        if let Some(n) = name {
            path.push(n);
        }
        if !path.is_empty() {
            paths.insert(path.join("::"));
        }
    }
    paths
}

/// The compiler's own view of every form core can be asked for, built for each of the twenty
/// forms ([`forms`]): rustdoc's pages of core with private items, from core's source, and the
/// pages of its public items as the build holds them, from a facade crate inlining core in that
/// form (rendered from core's compiled metadata, where `cfg(doc)` was never set); and a consumer
/// crate of the test's own naming every public path the facade shows, checked by cargo against
/// that form alone. Adding `default` changes nothing. Adding any other feature to any form keeps
/// every key of the smaller form, rendered the same (an item's declaration, a header, a field's
/// type, a row's visibility), and adds only where that feature's
/// layer lies: `net` under its four modules and the crate root's re-exports of them and rows to
/// them, the demo plant under `opc::demo` and its row, each master under its module and its line in
/// `fieldbus`. However an item's gate is written (a comment inside it, a line break, a macro, an
/// `include!`d file, a `#[path]` module, a cfg a build script sets), an item behind a feature
/// other than its own's shows as a key on the wrong side of some step, or as a path a form's
/// consumer cannot name.
#[test]
fn the_compiler_sees_each_layer_where_its_feature_puts_it() {
    const NET_ROOT_NAMES: &[&str] = &[
        "Quality",
        "Sample",
        "SeriesRef",
        "SeriesType",
        "SourceId",
        "Value",
        "now_unix_ms",
        "SourceError",
        "SourceHandle",
        "stop_flag",
    ];
    const NET_ROOT: &[&str] = &[
        "index.html#reexports",
        // The root's table in the build's view, its re-exports inlined: their sections.
        "index.html#enums",
        "index.html#functions",
        "index.html#structs",
        "index.html#reexport.Quality",
        "index.html#reexport.Sample",
        "index.html#reexport.SeriesRef",
        "index.html#reexport.SeriesType",
        "index.html#reexport.SourceId",
        "index.html#reexport.Value",
        "index.html#reexport.now_unix_ms",
        "index.html#reexport.SourceError",
        "index.html#reexport.SourceHandle",
        "index.html#reexport.stop_flag",
    ];
    // A master's line in `fieldbus`: the modules section and its entry in the page's contents,
    // there once one master is.
    const FIELDBUS_LINE: &[&str] = &[
        "fieldbus/index.html#modules",
        "fieldbus/index.html#rustdoc-toc",
    ];
    let layer = |feature: &str, path: &str| -> bool {
        match feature {
            "net" => {
                (["types/", "source/", "telegram/", "opc/"]
                    .iter()
                    .any(|d| path.starts_with(d))
                    && !path.starts_with("opc/demo/"))
                    // The root's re-exports, inlined as pages of the root in the build's view.
                    || (!path.contains('/')
                        && NET_ROOT_NAMES.iter().any(|n| {
                            path.split('#').next().unwrap().split('.').nth(1) == Some(*n)
                        }))
            }
            "demo-server" => path.starts_with("opc/demo/"),
            "modbus" => path.starts_with("fieldbus/modbus/"),
            "enip" => path.starts_with("fieldbus/enip/"),
            other => panic!("{other}"),
        }
    };
    let belongs = |feature: &str, entry: &str| -> bool {
        if let Some((_, row)) = entry.split_once("#row:") {
            return layer(feature, row);
        }
        layer(feature, entry)
            || (feature == "net" && NET_ROOT.contains(&entry))
            || (matches!(feature, "modbus" | "enip") && FIELDBUS_LINE.contains(&entry))
    };
    let forms = forms();
    for private in [true, false] {
        let views: Vec<BTreeMap<String, String>> =
            forms.iter().map(|f| rustdoc_view(f, private)).collect();
        // Every step from one form to the same form with one feature more.
        let mut steps = 0;
        for (a, base) in forms.iter().enumerate() {
            for (b, more) in forms.iter().enumerate() {
                let added: Vec<&&str> = more.iter().filter(|f| !base.contains(f)).collect();
                let step = match added.as_slice() {
                    [f] if base.iter().all(|x| more.contains(x)) => **f,
                    _ => continue,
                };
                steps += 1;
                let changed: Vec<String> = views[a]
                    .iter()
                    .filter(|(k, v)| views[b].get(*k) != Some(v))
                    .map(|(k, v)| format!("{k} = {v:?} -> {:?}", views[b].get(k)))
                    .collect();
                assert!(
                    changed.is_empty(),
                    "{base:?} + {step} (private items {private}) loses or changes {changed:?}"
                );
                let gained: Vec<&String> = views[b]
                    .keys()
                    .filter(|k| !views[a].contains_key(*k))
                    .collect();
                // `default` is `net` by another name: it adds nothing at all.
                if step == "default" {
                    assert!(
                        gained.is_empty(),
                        "{base:?} + default (private items {private}) adds {gained:?}"
                    );
                    continue;
                }
                assert!(!gained.is_empty(), "{base:?} + {step} adds nothing");
                let astray: Vec<&&String> = gained.iter().filter(|e| !belongs(step, e)).collect();
                assert!(
                    astray.is_empty(),
                    "{base:?} + {step} (private items {private}) adds outside its layer: {astray:?}"
                );
            }
        }
        assert_eq!(
            steps, 40,
            "four steps of net, eight of default, eight of the demo plant, ten of each master"
        );
        // The members' default form: `net` with `default`, no demo plant, no master.
        let members = forms
            .iter()
            .position(|f| f[..] == ["net", "default"])
            .unwrap();
        // A form holds nothing of a feature it lacks: no page, row or re-export in that
        // feature's layer (the line in `fieldbus` both masters share aside).
        let strict = |feature: &str, entry: &str| -> bool {
            match entry.split_once("#row:") {
                Some((_, row)) => layer(feature, row),
                None => layer(feature, entry) || (feature == "net" && NET_ROOT.contains(&entry)),
            }
        };
        for (form, view) in forms.iter().zip(&views) {
            for feature in ["net", "demo-server", "modbus", "enip"] {
                if form.contains(&feature) {
                    continue;
                }
                let held: Vec<&String> = view.keys().filter(|k| strict(feature, k)).collect();
                assert!(
                    held.is_empty(),
                    "{form:?} (private items {private}) holds {feature}'s layer: {held:?}"
                );
            }
        }
        // A dependency's type is named by its full path: the variant `SourceError::Pattern`
        // carries regex's error, told from any other crate's `Error`.
        let pattern = &views[members]["source/enum.SourceError.html#variant.Pattern"];
        assert!(pattern.contains("‹enum regex::error::Error›"), "{pattern}");
        for page in [
            "schema/index.html",
            "rng/index.html",
            "state/index.html",
            "fieldbus/index.html",
        ] {
            assert!(
                views[0].contains_key(page),
                "{page} missing from the slim form"
            );
        }
        for page in [
            "opc/index.html",
            "telegram/index.html",
            "opc/struct.StatusCode.html",
        ] {
            assert!(
                !views[0].contains_key(page) && views[members].contains_key(page),
                "{page}: the plant-facing layer's"
            );
        }
        if private {
            continue;
        }
        // Each form's consumer names every public path its view shows, checked against that form.
        let core = Path::new(env!("CARGO_MANIFEST_DIR"));
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("form-consumers");
        for (n, (form, view)) in forms.iter().zip(&views).enumerate() {
            let paths = public_paths(view);
            assert!(paths.contains("schema::stamp") && paths.contains("state::ControllerState"));
            let root = base.join(format!("view-{n}"));
            std::fs::create_dir_all(root.join("src")).unwrap();
            let features: Vec<String> = form.iter().map(|f| format!("{f:?}")).collect();
            std::fs::write(
                root.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"view-{n}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
                     [workspace]\n\n[dependencies]\nveredas-core = {{ path = {:?}, default-features = false, features = [{}] }}\n",
                    core.to_str().unwrap(),
                    features.join(", ")
                ),
            )
            .unwrap();
            std::fs::copy(core.join("../../Cargo.lock"), root.join("Cargo.lock")).unwrap();
            let uses: Vec<String> = paths
                .iter()
                .map(|p| format!("use veredas_core::{p} as _;"))
                .collect();
            std::fs::write(
                root.join("src/main.rs"),
                format!(
                    "#![allow(unused_imports)]\n{}\nfn main() {{}}\n",
                    uses.join("\n")
                ),
            )
            .unwrap();
            let out = Command::new(env!("CARGO"))
                .args(["check", "--offline", "--quiet", "--manifest-path"])
                .arg(root.join("Cargo.toml"))
                .env(
                    "CARGO_TARGET_DIR",
                    Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check"),
                )
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "the consumer of {form:?} cannot name what its view shows: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
