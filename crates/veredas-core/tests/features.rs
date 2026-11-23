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
        std::fs::write(
            root.join("src/main.rs"),
            format!("fn main() {{\n{}\n}}\n", names.join("\n")),
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
/// demo plant, each master on its own module. A gate written another way (a comment inside it, a
/// macro building it, a file pulled in by `include!` or `#[path]`) escapes a reading of the text;
/// the compiler's own view, the next test, is what closes the class.
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
            // A mention of a feature in a cfg, an attribute or the macro, as ordinarily written.
            let squeezed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if squeezed.contains("feature=") || squeezed.contains("cfg!(") {
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

/// Rustdoc's view of one form of core, private items included: every page, and every anchor on
/// a page (a method, a field, a variant, an impl, a re-export), as `page` and `page#anchor`.
/// `all.html` is kept as a page only: its sections mirror the pages. A page's blanket impls are
/// left out (below).
fn rustdoc_view(features: &[&str]) -> BTreeSet<String> {
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
        "--document-private-items",
        "--offline",
        "--quiet",
        "--no-default-features",
        "--manifest-path",
        m.to_str().unwrap(),
    ];
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
    let mut found = BTreeSet::new();
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
                // The blanket impls, the page's last section, are every crate's impls for every
                // type, not core's: which of them show depends on the crates in the graph.
                let html = match html.find("id=\"blanket-implementations\"") {
                    Some(i) => &html[..i],
                    None => &html[..],
                };
                for part in html.split(" id=\"").skip(1) {
                    let id = part.split('"').next().unwrap();
                    // The tooltips of notable traits: rustdoc's data, not an item.
                    if id != "notable-traits-data" {
                        found.insert(format!("{rel}#{id}"));
                    }
                }
            }
            found.insert(rel);
        }
    }
    found
}

/// The compiler's own view of every form core can be asked for: rustdoc's pages of core and the
/// anchors on them, private items included, built once for each of the twelve forms (the slim
/// form, `net`, `net` with the demo plant, each with or without each master). Adding a feature to
/// any form only adds, and only where that feature's layer lies: `net` under its four modules and
/// the crate root's re-exports of them, the demo plant under `opc::demo`, each master under its own
/// module and its line in `fieldbus`. However a gate is written (a comment inside it, a line break,
/// a macro, an `include!`d file, a `#[path]` module, a cfg a build script sets), an item behind a
/// feature other than its own's shows here as an anchor on the wrong side of some step.
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
    let belongs = |feature: &str, entry: &str| -> bool {
        match feature {
            "net" => {
                (["types/", "source/", "telegram/", "opc/"]
                    .iter()
                    .any(|d| entry.starts_with(d))
                    && !entry.starts_with("opc/demo/"))
                    || NET_ROOT.contains(&entry)
            }
            "demo-server" => entry.starts_with("opc/demo/"),
            "modbus" => entry.starts_with("fieldbus/modbus/") || FIELDBUS_LINE.contains(&entry),
            "enip" => entry.starts_with("fieldbus/enip/") || FIELDBUS_LINE.contains(&entry),
            other => panic!("{other}"),
        }
    };
    // The twelve forms: the plant-facing layer off, on, or on with the demo plant; each master
    // off or on.
    let mut forms: Vec<Vec<&str>> = Vec::new();
    for net in [&[][..], &["net"][..], &["net", "demo-server"][..]] {
        for modbus in [&[][..], &["modbus"][..]] {
            for enip in [&[][..], &["enip"][..]] {
                forms.push([net, modbus, enip].concat());
            }
        }
    }
    let views: Vec<BTreeSet<String>> = forms.iter().map(|f| rustdoc_view(f)).collect();
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
            let lost: Vec<&String> = views[a].difference(&views[b]).collect();
            assert!(lost.is_empty(), "{base:?} + {step} loses {lost:?}");
            let gained: Vec<&String> = views[b].difference(&views[a]).collect();
            assert!(!gained.is_empty(), "{base:?} + {step} adds nothing");
            let astray: Vec<&&String> = gained.iter().filter(|e| !belongs(step, e)).collect();
            assert!(
                astray.is_empty(),
                "{base:?} + {step} adds outside its layer: {astray:?}"
            );
        }
    }
    assert_eq!(
        steps, 20,
        "four steps of net, four of the demo plant, six of each master"
    );
    // The slim form is the header reader, the PCG32, the state text and the fieldbus module; the
    // plant-facing layer brings the OPC UA client and the telegram.
    for page in [
        "schema/index.html",
        "rng/index.html",
        "state/index.html",
        "fieldbus/index.html",
    ] {
        assert!(views[0].contains(page), "{page} missing from the slim form");
    }
    for page in [
        "opc/index.html",
        "telegram/index.html",
        "opc/struct.StatusCode.html",
    ] {
        assert!(
            !views[0].contains(page) && views[4].contains(page),
            "{page}: the plant-facing layer's"
        );
    }
}
