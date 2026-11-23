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

/// What the members import from core today, every path of it; the default form must hold each.
const MEMBERS_IMPORT: &[&str] = &[
    "Quality",
    "Sample",
    "SeriesRef",
    "SeriesType",
    "SourceError",
    "SourceHandle",
    "SourceId",
    "Value",
    "now_unix_ms",
    "stop_flag",
    "fieldbus",
    "opc::NodeId",
    "opc::OpcError",
    "opc::StatusCode",
    "opc::Variant",
    "opc::WriteError",
    "opc::run_on_own_runtime",
    "opc::spawn",
    "opc::error::NodeIdError",
    "opc::spec::Auth",
    "opc::spec::BitSpec",
    "opc::spec::PollSpec",
    "opc::spec::RuleSpec",
    "opc::spec::TagSpec",
    "opc::spec::parse_node_id",
    "opc::worker::CONNECT_TIMEOUT",
    "opc::worker::MAX_NODES_PER_READ",
    "opc::worker::WriteGate",
    "rng::Pcg32",
    "schema::HeaderError",
    "schema::check",
    "schema::stamp",
    "state::ControllerState",
    "state::reason_entry",
    "telegram::Catalog",
    "telegram::ChannelSpec",
    "telegram::Endian",
    "telegram::Message",
    "telegram::PeerId",
    "telegram::Role",
    "telegram::capture::FieldType",
    "telegram::catalog::Catalog",
    "telegram::catalog::load",
    "telegram::schema::ChannelSpec",
    "telegram::schema::load",
];

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
        // The default form also takes every path the members import from core today (portal,
        // supervisão, comissionador, controlador, maquinar, mestres, as `grep -rho
        // 'veredas_core::...' crates tools` lists them), functions and constants included.
        let uses: Vec<String> = if name == "default-form" {
            MEMBERS_IMPORT
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
        let text = std::fs::read_to_string(&path).unwrap();
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            // A mention of a feature in a cfg, an attribute or the macro, as ordinarily written;
            // and what would hide an item from rustdoc, the compiler's view below
            // (`doc(hidden)`, a cfg on `doc`), of which core has none.
            let squeezed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
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

/// The twelve forms core can be asked for: the plant-facing layer off, on, or on with the demo
/// plant; each master off or on. The last is every feature, the form core's own tests run in.
fn forms() -> Vec<Vec<&'static str>> {
    let mut forms = Vec::new();
    for net in [&[][..], &["net"][..], &["net", "demo-server"][..]] {
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

/// Every plain string literal of a Rust source that starts with a capital letter, as written
/// (escapes kept), so it can be pasted into another source.
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
                i = j + close.len();
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
/// twelve forms alone, run, prints what the header reader answers (every stamp and check of a
/// set of headers, refusals with their texts), the state text's tables and lookups, every state
/// line of core's own state-text tests read and written back (or refused, with the text), and
/// draws of the PCG32 from fixed seeds; all twelve print the same, so the form core's own tests
/// run in (every feature) speaks for each. A gate that changes what an item does without changing
/// what rustdoc renders of it (a constant's value, a derive's attribute, a function's body) shows
/// here as a form that prints otherwise, for the slim layer; for the plant-facing layer and the
/// masters, core's own tests hold their behaviour in the merged form alone.
#[test]
fn the_slim_layer_behaves_the_same_in_every_form() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lines =
        capitalized_literals(&std::fs::read_to_string(core.join("tests/state_text.rs")).unwrap());
    assert!(lines.len() > 100, "{} state lines", lines.len());
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
    ];
    let main = format!(
        r##"use std::fmt::Write;
use veredas_core::rng::Pcg32;
use veredas_core::schema;
use veredas_core::state::{{self, CommandSource, ControllerState, StateWord}};

const HEADERS: &[&str] = &{headers:?};
const LINES: &[&str] = &[
{lines}
];

fn main() {{
    let mut out = String::new();
    for h in HEADERS {{
        let _ = writeln!(out, "stamp {{h:?}} = {{:?}}", schema::stamp(h, "an x file").map_err(|e| (e.to_string(), e)));
        for (f, v) in [("veredas-x", 1), ("veredas-x", 2), ("veredas-y", 1)] {{
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
    for l in LINES {{
        match l.parse::<ControllerState>() {{
            Ok(s) => {{
                let _ = writeln!(out, "line {{l:?}} = {{s:?}} / {{s}} / {{:?}} / {{}}", s.health(), s.clone().normalized());
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
        printed.push(String::from_utf8(out.stdout).unwrap());
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

/// Text with its tags stripped and its whitespace collapsed.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
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

/// Rustdoc's view of one form of core, with private items or without them: every page and every
/// key a page shows ([`page_view`]), as `page` and `page#key`, each with what it renders.
/// `all.html` is kept as a page only: its sections mirror the pages.
fn rustdoc_view(features: &[&str], private: bool) -> BTreeMap<String, String> {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check");
    let docs = target.join("doc").join("veredas_core");
    let _ = std::fs::remove_dir_all(&docs);
    let list = features.join(",");
    let m = manifest();
    let mut args = vec![
        "doc",
        "-p",
        "veredas-core",
        "--no-deps",
        "--lib",
        "--offline",
        "--quiet",
        "--no-default-features",
        "--manifest-path",
        m.to_str().unwrap(),
    ];
    if private {
        args.push("--document-private-items");
    }
    if !features.is_empty() {
        args.extend(["--features", list.as_str()]);
    }
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

/// The compiler's own view of every form core can be asked for, built for each of the twelve
/// forms (the plant-facing layer off, on, or on with the demo plant; each master off or on):
/// rustdoc's pages of core with private items and without them, and a consumer crate of the
/// test's own naming every public path the form's view shows, checked by cargo against that form
/// alone, so rustdoc's view is the build's (rustdoc sets `cfg(doc)`, a build does not). Adding a
/// feature to any form keeps every key of the smaller form, rendered the same (an item's
/// declaration, a header, a field's type, a row's visibility), and adds only where that feature's
/// layer lies: `net` under its four modules and the crate root's re-exports of them and rows to
/// them, the demo plant under `opc::demo` and its row, each master under its module and its line in
/// `fieldbus`. However an item's gate is written (a comment inside it, a line break, a macro, an
/// `include!`d file, a `#[path]` module, a cfg a build script sets), an item behind a feature
/// other than its own's shows as a key on the wrong side of some step, or as a path a form's
/// consumer cannot name.
#[test]
fn the_compiler_sees_each_layer_where_its_feature_puts_it() {
    const NET_ROOT: &[&str] = &[
        "index.html#reexports",
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
                ["types/", "source/", "telegram/", "opc/"]
                    .iter()
                    .any(|d| path.starts_with(d))
                    && !path.starts_with("opc/demo/")
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
                assert!(!gained.is_empty(), "{base:?} + {step} adds nothing");
                let astray: Vec<&&String> = gained.iter().filter(|e| !belongs(step, e)).collect();
                assert!(
                    astray.is_empty(),
                    "{base:?} + {step} (private items {private}) adds outside its layer: {astray:?}"
                );
            }
        }
        assert_eq!(
            steps, 20,
            "four steps of net, four of the demo plant, six of each master"
        );
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
                !views[0].contains_key(page) && views[4].contains_key(page),
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
