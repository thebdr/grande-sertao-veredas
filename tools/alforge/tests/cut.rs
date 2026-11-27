//! C-291: a trial cut of HEAD, for real (feature `corte`; docs/questions.md Q440): the executables
//! built in the builder machine of the tester machine's distribution, the bundle packed, extracted
//! and run together on its plant; then the test's own reading of what came out: the manifest
//! against the repository, the bundle's files and sums, a tampered copy refused, every
//! executable's needs within the tester machine's libraries, the packing repeated to the same
//! bytes, and what the run saw of the plant.

use std::path::{Path, PathBuf};

use alforge::bundle;
use alforge::cut::{self, Options};
use alforge::machine::Recipe;
use alforge::manifest;
use alforge::together;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn a_trial_cut_builds_in_the_builder_packs_and_runs_the_bundle_together() {
    let root = repo_root();
    let out = tempfile::tempdir().unwrap();
    let release = "0.0.0-trial.291";
    let work = root.join("target/alforge").join(release);
    let options = Options { release: semver::Version::parse(release).unwrap(), out: out.path().to_path_buf(), work: work.clone(), run_suite: false };
    // HEAD as the cut finds it: what it cuts, whatever is committed while it runs.
    let head = alforge::git::text(&root, &["rev-parse", "HEAD"]).unwrap().unwrap().trim().to_string();
    let c = match cut::cut(&root, &options) {
        Ok(c) => c,
        Err(e) => panic!("the trial cut was refused: {e}"),
    };

    // The manifest: HEAD, every component at its version and executables, checked like any other.
    assert_eq!(c.manifest.commit, head);
    assert_eq!(c.manifest_path, out.path().join("0.0.0-trial.291.toml"));
    assert_eq!(manifest::check(&root, &c.manifest).unwrap(), Vec::<String>::new());
    let held = manifest::components_at(&root, &head, &mut Vec::new()).unwrap();
    // Each component ships every bin, built with the features its executables need (Q443):
    // Oficina's window among them, with `shell` on.
    for x in &c.manifest.components {
        let h = &held[&x.name];
        assert_eq!(x.bins, h.bins.iter().cloned().collect::<Vec<_>>(), "{}", x.name);
        for b in &x.bins {
            assert!(work.join("out/bin").join(b).is_file(), "{b} built");
        }
    }
    assert!(c.manifest.components.iter().any(|x| x.name == "veredas-oficina" && x.bins == ["veredas-oficina"]), "oficina's window ships");
    let pinned: Vec<(String, String)> = c.manifest.components.iter().map(|x| (x.name.clone(), x.version.to_string())).collect();
    let at_head: Vec<(String, String)> = held.values().map(|h| (h.name.clone(), h.version.clone())).collect();
    assert_eq!(pinned, at_head);
    assert_eq!(c.manifest.tested.suite, "not run: a trial cut");

    // Each executable's dep-info, as the builder handed it back beside it (each component built
    // alone): its compilation read its own sources, and nothing outside what its component's
    // version names (C-290's inputs), or the cut would have stopped.
    // A component that ships none, by its library's own build.
    for h in held.values() {
        let inputs = h.inputs.as_ref().unwrap();
        let shipped = &c.manifest.components.iter().find(|x| x.name == h.name).unwrap().bins;
        let files: Vec<String> = if shipped.is_empty() { h.lib.iter().map(|l| format!("lib{l}.d")).collect() } else { shipped.iter().map(|b| format!("{b}.d")).collect() };
        assert!(!files.is_empty(), "{} is built for its dep-info", h.name);
        for f in files {
            let info = std::fs::read_to_string(work.join("out/deps").join(&f)).unwrap();
            assert!(info.contains(&format!("/build/src/{}/src/", h.dir)), "{f}: {info}");
            assert_eq!(alforge::inputs::read_outside(&info, "/build/src", "/build/vendor", inputs, &h.packages), Vec::<String>::new(), "{f}");
        }
    }
    assert!(c.manifest.toolchain.starts_with("rustc "), "{}", c.manifest.toolchain);

    // Each component's build traced (docs/questions.md Q448): its log beside its executables, rustc
    // among what it ran, and nothing its version does not name read by any process but cargo.
    for h in held.values() {
        let log = std::fs::read_to_string(work.join("out/trace").join(format!("{}.trace", h.name))).unwrap();
        let reads = alforge::trace::reads(&log, "/build/src");
        assert_eq!(alforge::trace::outside(&reads, "/build/src", "/build/vendor", h.inputs.as_ref().unwrap(), &h.packages), Vec::<String>::new(), "{}", h.name);
    }
    let all: Vec<alforge::trace::Read> = held.keys().flat_map(|n| alforge::trace::reads(&std::fs::read_to_string(work.join("out/trace").join(format!("{n}.trace"))).unwrap(), "/build/src")).collect();
    assert!(all.iter().any(|r| r.program.ends_with("/rustc") && r.path.starts_with("/build/src/crates/")), "rustc's reads of the tree are traced");
    assert!(all.iter().any(|r| r.program.ends_with("build_script_build")), "a build script's reads are traced");

    // The bundle: its name, its files, the manifest inside byte for byte, its sums.
    let name = "veredas-0.0.0-trial.291-x86_64-unknown-linux-gnu";
    assert_eq!(c.bundle, work.join(format!("{name}.tar.gz")));
    let files = bundle::files(&c.extracted).unwrap();
    let bins: Vec<String> = c.manifest.components.iter().flat_map(|x| x.bins.iter().map(|b| format!("bin/{b}"))).collect();
    let in_bin: Vec<&String> = files.iter().filter(|f| f.starts_with("bin/")).collect();
    let mut want_bins = bins.clone();
    want_bins.sort();
    assert_eq!(in_bin, want_bins.iter().collect::<Vec<_>>());
    for f in [
        "README.md",
        "docs/veredas-plan.md",
        "docs/binding-schema.md",
        "docs/bench-schema.md",
        "docs/project-schema.md",
        "docs/portal-plan.md",
        "docs/supervisao-plan.md",
        "docs/maquinar-plan.md",
        "docs/design/README.md",
        "planta/bench.toml",
        "planta/supervisao.toml",
        "planta/loop/project.toml",
        "planta/loop/binding.toml",
        "planta/loop/layout.svg",
        "release.toml",
        "SHA256SUMS",
    ] {
        assert!(files.iter().any(|x| x == f), "the bundle holds no {f}: {files:?}");
    }
    assert!(!files.iter().any(|f| f.starts_with("tools/") || f.starts_with("crates/") || f.ends_with(".rs")), "{files:?}");
    // Oficina's window with the Qt it loads, laid out as Qt lays itself out (Q446): its RUNPATH the
    // bundle's own, its libraries, its platform plugins and QML modules there, no WebEngine.
    for f in ["lib/qt/lib/libQt6Core.so.6", "lib/qt/lib/libQt6Quick.so.6", "lib/qt/lib/libkddockwidgets-qt6.so.4", "lib/qt/plugins/platforms/libqxcb.so", "lib/qt/plugins/platforms/libqoffscreen.so", "lib/qt/qml/QtQuick/Controls/qmldir"] {
        assert!(files.iter().any(|x| x == f), "the bundle holds no {f}");
    }
    assert!(!files.iter().any(|f| f.contains("WebEngine")), "only what the window imports");
    let dynamic = std::process::Command::new("readelf").arg("-d").arg(c.extracted.join("bin/veredas-oficina")).output().unwrap();
    assert!(String::from_utf8_lossy(&dynamic.stdout).contains("Library runpath: [$ORIGIN/../lib/qt/lib]"), "{}", String::from_utf8_lossy(&dynamic.stdout));
    let kdd = std::process::Command::new("readelf").arg("-d").arg(c.extracted.join("lib/qt/lib/libkddockwidgets-qt6.so.4")).output().unwrap();
    assert!(String::from_utf8_lossy(&kdd.stdout).contains("Library runpath: [$ORIGIN]"), "{}", String::from_utf8_lossy(&kdd.stdout));
    // Its licences (C-294): a row in THIRD-PARTY.md for every entry of the component list, the
    // components' own files, Qt's and KDDockWidgets' texts.
    let third = std::fs::read_to_string(c.extracted.join("licenses/THIRD-PARTY.md")).unwrap();
    let list: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(c.extracted.join("sbom.cdx.json")).unwrap()).unwrap();
    assert_eq!(third.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| Package")).count(), list["components"].as_array().unwrap().len());
    for f in ["licenses/components/veredas-core/LICENSE-MIT", "licenses/components/veredas-oficina/COPYING", "licenses/qt/LGPL-3", "licenses/kddockwidgets/LICENSE.txt"] {
        assert!(files.iter().any(|x| x == f), "the bundle holds no {f}");
    }
    assert!(files.iter().any(|f| f.starts_with("licenses/crates/serde-") && f.contains("LICENSE")), "a crate's own licence files");
    // At any depth and whole folders of them (C-294's first refute round): ring's once_cell texts,
    // crc-catalog's `LICENSES/`, IronCalc's notices, KDDockWidgets' every text, Qt's own SBOM.
    for (start, end) in [
        ("licenses/crates/ring-", "/src/polyfill/once_cell/LICENSE-MIT"),
        ("licenses/crates/crc-catalog-", "/LICENSES/MIT.txt"),
        ("licenses/components/ironcalc_base/", "THIRD-PARTY-NOTICES.md"),
        ("licenses/kddockwidgets/", "LICENSES/MIT.txt"),
        ("licenses/qt/sbom/", "qtbase-6.12.0.spdx.json"),
        ("licenses/qt/sbom/", "qtdeclarative-6.12.0.spdx.json"),
    ] {
        assert!(files.iter().any(|f| f.starts_with(start) && f.ends_with(end)), "the bundle holds no {start}…{end}");
    }
    assert!(!third.contains("| none shipped |") || third.lines().filter(|l| l.ends_with("| none shipped |")).all(|l| !l.contains("tokio-modbus")), "tokio-modbus ships its texts");
    let manifest_text = std::fs::read_to_string(&c.manifest_path).unwrap();
    assert_eq!(std::fs::read_to_string(c.extracted.join("release.toml")).unwrap(), manifest_text);
    // The component list in the bundle and beside it: `alforge components`'s for the commit and
    // the release, byte for byte (C-295).
    let sbom = std::fs::read_to_string(c.extracted.join("sbom.cdx.json")).unwrap();
    assert_eq!(sbom, alforge::components::list(&root, &head, Some(release)).unwrap());
    assert_eq!(std::fs::read_to_string(work.join(format!("{name}.cdx.json"))).unwrap(), sbom);
    assert_eq!(bundle::verify_sums(&c.extracted).unwrap(), Vec::<String>::new());
    let plant_at_head = alforge::git::show(&root, &head, "tools/alforge/planta/loop/binding.toml").unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(c.extracted.join("planta/loop/binding.toml")).unwrap(), plant_at_head);
    // Copies of a bundle go under target/ (on disk): /tmp is a tmpfs of the machine's memory.
    let tampered = work.join("tampered");
    let _ = std::fs::remove_dir_all(&tampered);
    std::fs::create_dir_all(&tampered).unwrap();
    std::process::Command::new("cp").arg("-r").arg(&c.extracted).arg(&tampered).status().unwrap();
    let copy = tampered.join(name);
    std::fs::write(copy.join("planta/bench.toml"), "changed\n").unwrap();
    assert_eq!(bundle::verify_sums(&copy).unwrap(), ["planta/bench.toml does not match its sum"]);

    // Every executable built in the builder asks no more of the libraries than the guest offers;
    // the ones that link the database engine's C++ ask for the C++ library at all.
    let recipe = Recipe::load_file(&root.join("tools/alforge/maquina/maquina.toml")).unwrap();
    for b in &bins {
        let out = std::process::Command::new("objdump").arg("-p").arg(c.extracted.join(b)).output().unwrap();
        let needs = bundle::needs(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(bundle::check_needs(b, &needs, &recipe), Vec::<String>::new());
        assert!(!bundle::newer_than(&needs["GLIBC"], "2.41"), "{b}: {needs:?}");
        if b.ends_with("portal") || b.ends_with("supervisao") {
            assert!(needs.contains_key("GLIBCXX"), "{b}: {needs:?}");
        }
    }

    // Packed again from the same commit and executables, the archive is the same bytes.
    let again = work.join("again");
    let _ = std::fs::remove_dir_all(&again);
    std::fs::create_dir_all(&again).unwrap();
    let names: Vec<String> = held.keys().cloned().collect();
    let plain_bins: Vec<String> = bins.iter().map(|b| b.trim_start_matches("bin/").to_string()).collect();
    let repacked = bundle::assemble(&recipe, &root, &head, name, &c.extracted.join("bin"), &plain_bins, &names, &manifest_text, &sbom, &again).unwrap();
    assert_eq!(alforge::cache::sha256(&repacked).unwrap(), alforge::cache::sha256(&c.bundle).unwrap());

    // What the run saw: the page, every object live, both sources live, the parcels moving along
    // the loop, the offered write refused by portal's switch, both executables ending with 0.
    let r = &c.report;
    assert_eq!(together::faults(r), Vec::<String>::new());
    assert_eq!(r.page.status, 200);
    assert_eq!(r.write.status, 403);
    assert_eq!(r.write.body, r#"{"error":"writes are turned off for this project"}"#);
    for id in ["parcel-1", "parcel-2"] {
        let a = together::object(&r.first, id).unwrap()["place"].as_f64().unwrap();
        let b = together::object(&r.second, id).unwrap()["place"].as_f64().unwrap();
        assert!(a != b && (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b), "{id}: {a} then {b}");
    }
    let rate = together::object(&r.second, "read-rate").unwrap()["value"].as_f64().unwrap();
    assert!((97.5..=100.0).contains(&rate), "the sorter's rate {rate}");
    assert_eq!(r.second["session"]["writes_armed"], false);
    assert_eq!((r.bench_exit, r.supervisao_exit), (Some(0), Some(0)));
    assert!(r.bench_out.contains("2 part(s) running") && r.bench_out.contains("stopped"), "{}", r.bench_out);
    assert!(r.supervisao_out.contains("session `loop` at http://127.0.0.1:"), "{}", r.supervisao_out);
    let _ = std::fs::remove_dir_all(&work);
}
