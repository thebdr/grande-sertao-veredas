//! Core's layers by feature (contract: the features clause; `docs/core-plan.md`, "Features").
//!
//! Asked of cargo itself, as `tests/foundation.rs` asks it what core builds: the manifest's
//! features as cargo reads them, the dependency graph cargo resolves without the default
//! features, and the library cargo compiles in each form a component may ask for.

use std::collections::BTreeSet;
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

/// The library compiles in each form: without its default features (the header reader, the
/// PCG32, the state text) and with the fieldbus masters alone. Built in a folder of the test's
/// own, so the run's build folder is not shared.
#[test]
fn the_library_compiles_without_its_default_features_and_with_the_masters_alone() {
    let m = manifest();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("features-check");
    for extra in [
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
