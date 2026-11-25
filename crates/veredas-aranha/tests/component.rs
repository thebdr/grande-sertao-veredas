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

/// The workspace's root, which every cargo here runs from, as the real build does: cargo reads its
/// configuration from the folder it runs in upward, so one in aranha's folder (which a build from
/// the root never reads) steers nothing checked here, and the check below refuses one.
fn repo_root() -> PathBuf {
    manifest_dir().join("../..").canonicalize().unwrap()
}

fn cargo() -> Command {
    let mut c = Command::new(env!("CARGO"));
    c.current_dir(repo_root());
    c
}

fn metadata(manifest: &Path, extra: &[&str]) -> Value {
    let out = cargo()
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

/// Every target's source file, as cargo lists it.
fn target_sources(dir: &Path) -> Vec<(String, PathBuf)> {
    let meta = metadata(&dir.join("Cargo.toml"), &["--no-deps", "--offline"]);
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()
        .clone();
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["kind"][0].as_str().unwrap().to_string(), PathBuf::from(t["src_path"].as_str().unwrap())))
        .collect()
}

/// Whether cargo finds a target of this kind at this path (from the package's folder) by itself:
/// `src/lib.rs`; `src/main.rs`, `src/bin/*.rs`, `src/bin/*/main.rs`; `tests/`, `examples/` or
/// `benches/` then `*.rs` or `*/main.rs`; `build.rs`. A copy of the folders then holds the target,
/// and the copy's cargo finds it.
fn found_by_cargo(kind: &str, rel: &Path) -> bool {
    let parts: Vec<&str> = rel.iter().map(|p| p.to_str().unwrap_or("")).collect();
    let rs = |s: &str| s.len() > 3 && s.ends_with(".rs");
    let under = |dir: &str| matches!(parts.as_slice(), [d, f] if *d == dir && rs(f)) || matches!(parts.as_slice(), [d, _, "main.rs"] if *d == dir);
    match kind {
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => parts == ["src", "lib.rs"],
        "bin" => parts == ["src", "main.rs"] || matches!(parts.as_slice(), ["src", "bin", f] if rs(f)) || matches!(parts.as_slice(), ["src", "bin", _, "main.rs"]),
        "test" => under("tests"),
        "example" => under("examples"),
        "bench" => under("benches"),
        "custom-build" => parts == ["build.rs"],
        _ => false,
    }
}

/// The three builds every check here holds: every feature, dev and release, x86-64 Linux and
/// Windows GNU.
const BUILDS: [&[&str]; 3] = [&["--all-features"], &["--all-features", "--release"], &["--all-features", "--target", "x86_64-pc-windows-gnu"]];

/// Every file rustc read to compile the targets of the package at `manifest`, by the dep-info it
/// wrote for each, in a check of every target with `args`, into `target_dir`.
fn files_rustc_read(manifest: &Path, target_dir: &Path, args: &[&str]) -> (BTreeSet<PathBuf>, BTreeSet<(String, String)>) {
    let out = cargo()
        .args(["check", "--offline", "--all-targets", "--message-format=json", "--manifest-path"])
        .arg(manifest)
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "check {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    let manifest = manifest.canonicalize().unwrap();
    // Dep-info names a file relative to the workspace's root.
    let meta = metadata(&manifest, &["--no-deps", "--offline"]);
    let root = PathBuf::from(meta["workspace_root"].as_str().unwrap());
    let mut files = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut units = 0;
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
        if msg["reason"] != "compiler-artifact" || Path::new(msg["manifest_path"].as_str().unwrap_or("")).canonicalize().ok() != Some(manifest.clone()) {
            continue;
        }
        // The artifact `deps/lib<name>-<hash>.rmeta` has its dep-info beside it, `<name>-<hash>.d`.
        let artifact = PathBuf::from(msg["filenames"][0].as_str().unwrap());
        let stem = artifact.file_stem().unwrap().to_str().unwrap();
        let dep = artifact.with_file_name(format!("{}.d", stem.strip_prefix("lib").unwrap_or(stem)));
        let text = std::fs::read_to_string(&dep).unwrap_or_else(|e| panic!("{}: {e}", dep.display()));
        let first = text.lines().next().unwrap();
        let (_, inputs) = first.split_once(": ").unwrap_or((first, ""));
        // Spaces in a path are written `\ `.
        let inputs = inputs.replace("\\ ", "\u{0}");
        files.extend(inputs.split_whitespace().map(|p| root.join(p.replace('\u{0}', " "))));
        names.insert((msg["target"]["kind"][0].as_str().unwrap().to_string(), msg["target"]["name"].as_str().unwrap().to_string()));
        units += 1;
    }
    assert!(units > 0, "no unit of the package was checked");
    // Every file the line names is there: a name that breaks the line (a line break in it) leaves a
    // piece that is no file, and the files after it unread.
    let missing: Vec<&PathBuf> = files.iter().filter(|f| !f.exists()).collect();
    assert!(missing.is_empty(), "dep-info names files that are not there (a name breaking its line?): {missing:?}");
    (files, names)
}

/// The package's targets by kind and name, as cargo's metadata names them.
fn target_names(dir: &Path) -> BTreeSet<(String, String)> {
    let meta = metadata(&dir.join("Cargo.toml"), &["--no-deps", "--offline"]);
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap()
        .clone();
    package["targets"].as_array().unwrap().iter().map(|t| (t["kind"][0].as_str().unwrap().to_string(), t["name"].as_str().unwrap().to_string())).collect()
}

/// Every cargo configuration in `dir`, in any folder under it, and in each folder from it up to
/// `root` (not the root's own, which a build from the root reads as this check does).
fn configs_between(dir: &Path, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let found = |d: &Path, out: &mut Vec<PathBuf>| {
        for name in ["config", "config.toml"] {
            let c = d.join(".cargo").join(name);
            if c.symlink_metadata().is_ok() {
                out.push(c);
            }
        }
    };
    let mut walk = vec![dir.to_path_buf()];
    while let Some(d) = walk.pop() {
        found(&d, &mut out);
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().is_dir() {
                walk.push(path);
            }
        }
    }
    let mut up = dir.parent();
    while let Some(d) = up.filter(|d| *d != root && d.starts_with(root)) {
        found(d, &mut out);
        up = d.parent();
    }
    out.sort();
    out
}

/// Every path under `dir` (folders and files, links not followed) for which `bad` holds.
fn paths_under(dir: &Path, bad: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut walk = vec![dir.to_path_buf()];
    while let Some(d) = walk.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if bad(&path) {
                out.push(path.clone());
            }
            if path.symlink_metadata().unwrap().is_dir() {
                walk.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A path's name holds a control character (a line break ends dep-info's line, and with it what
/// rustc's list says of the files after it).
fn control_in_name(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().chars().any(char::is_control))
}

/// A path is a link (a copy, or a move, carries the link and not its file).
fn is_link(p: &Path) -> bool {
    p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink())
}

/// The assembly in the text files of `files` that lie in `dir`, by file name and line.
fn assembly_in(files: &BTreeSet<PathBuf>, dir: &Path) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.canonicalize().is_ok_and(|c| c.starts_with(dir)))
        // Every file read, text or not: none is passed over for being unreadable.
        .map(|f| (f, String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string()))
        .flat_map(|(f, t)| assembly(&t).into_iter().map(move |l| format!("{}: {l}", f.file_name().unwrap().to_string_lossy())))
        .collect()
}

/// The files of `files` that do not lie in `dir`, once every link is followed.
fn outside(files: &BTreeSet<PathBuf>, dir: &Path) -> Vec<PathBuf> {
    files.iter().filter(|f| !f.canonicalize().unwrap_or_else(|_| (*f).clone()).starts_with(dir)).cloned().collect()
}

#[test]
fn aranha_compiles_in_no_file_from_outside_its_folder() {
    let dir = manifest_dir().canonicalize().unwrap();
    // No cargo configuration in its folder or between it and the root; no name under it with a
    // control character; no link under it.
    assert_eq!(configs_between(&dir, &repo_root()), Vec::<PathBuf>::new());
    assert_eq!(paths_under(&dir, &control_in_name), Vec::<PathBuf>::new());
    assert_eq!(paths_under(&dir, &is_link), Vec::<PathBuf>::new());
    // Every target is one cargo finds by itself: the split of the test below, which copies the
    // folders, holds every one, and builds it.
    for (kind, src) in target_sources(&dir) {
        let rel = src.canonicalize().unwrap().strip_prefix(&dir).unwrap().to_path_buf();
        assert!(found_by_cargo(&kind, &rel), "a {kind} target cargo would not find by itself: {}", rel.display());
    }
    // And the rule refuses a target declared at a path cargo would not find.
    for (path, ok) in [("tests/x.rs", true), ("tests/x/main.rs", true), ("tests/x/y.rs", false), ("tests/x.txt", false), ("src/t.rs", false), ("tests/a/b/c.rs", false), ("tests/.rs", false)] {
        assert_eq!(found_by_cargo("test", Path::new(path)), ok, "{path}");
    }
    assert!(found_by_cargo("lib", Path::new("src/lib.rs")) && !found_by_cargo("lib", Path::new("src/other.rs")));
    assert!(found_by_cargo("bin", Path::new("src/bin/x/main.rs")) && !found_by_cargo("bin", Path::new("bin/x.rs")));
    assert!(found_by_cargo("bin", Path::new("src/bin/x.rs")) && found_by_cargo("bin", Path::new("src/main.rs")) && !found_by_cargo("bin", Path::new("src/cli.rs")));
    let hidden = planted("hidden", &[("Cargo.toml", manifest("[[test]]\nname = \"hidden\"\npath = \"tests/hidden.txt\"\n")), ("src/lib.rs", String::new()), ("tests/hidden.txt", String::new())]);
    let declared: Vec<(String, PathBuf)> = target_sources(&hidden).into_iter().map(|(k, p)| (k, p.strip_prefix(hidden.canonicalize().unwrap()).unwrap().to_path_buf())).collect();
    std::fs::remove_dir_all(&hidden).unwrap();
    assert!(declared.iter().any(|(k, p)| k == "test" && !found_by_cargo(k, p)), "{declared:?}");
    // Every file rustc reads for every target, in every build checked, by its own dep-info: no
    // spelling of an include, no module path, no link hides a file from it.
    // Every target cargo names is among those checked (a target set apart, `test = false` say,
    // is not left out), and no file rustc reads, whatever its name, holds assembly.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("depinfo");
    let (mut all_read, mut checked) = (BTreeSet::new(), BTreeSet::new());
    for args in BUILDS {
        let (read, names) = files_rustc_read(&dir.join("Cargo.toml"), &target, args);
        assert!(read.iter().any(|f| f.ends_with("src/profinet/dcp.rs")) && read.iter().any(|f| f.ends_with("tests/component.rs")), "{read:?}");
        assert_eq!(outside(&read, &dir), Vec::<PathBuf>::new(), "{args:?}");
        all_read.extend(read);
        checked.extend(names);
    }
    assert_eq!(target_names(&dir).difference(&checked).cloned().collect::<Vec<_>>(), Vec::<(String, String)>::new());
    assert_eq!(assembly_in(&all_read, &dir), Vec::<String>::new());
    // The checks can fail, in each build: a crate that pulls in a file by an include in braces
    // behind a comment, a module that is a link to a file outside, an include only a Windows build
    // and only a release build reads, a module file not named `.rs` holding assembly, and a test
    // target set apart from the checked builds.
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("depinfo-planted");
    let _ = std::fs::remove_dir_all(&base);
    let krate = base.join("krate");
    std::fs::create_dir_all(krate.join("src")).unwrap();
    std::fs::write(base.join("note.txt"), "x").unwrap();
    std::fs::write(base.join("outside_mod.rs"), "pub const M: u8 = 1;\n").unwrap();
    std::fs::write(base.join("win.txt"), "w").unwrap();
    std::fs::write(base.join("rel.txt"), "r").unwrap();
    std::fs::write(base.join("cfg.txt"), "c").unwrap();
    let inc = format!("{}{}", "inclu", "de_str!");
    std::fs::write(
        krate.join("Cargo.toml"),
        manifest("[[test]]\nname = \"hidden\"\ntest = false\n[[test]]\nname = \"veredas_aranha\"\npath = \"tests/veredas_aranha.rs\"\ntest = false\n"),
    )
    .unwrap();
    std::fs::create_dir_all(krate.join("tests")).unwrap();
    std::fs::write(krate.join("tests/hidden.rs"), "#[test]\nfn h() {}\n").unwrap();
    // A test target named as the library is told from it.
    std::fs::write(krate.join("tests/veredas_aranha.rs"), "#[test]\nfn h() {}\n").unwrap();
    // A configuration in the crate's folder setting a cfg that leaves an include out, read by a
    // cargo run from inside the folder and by no build from the root.
    std::fs::create_dir_all(krate.join(".cargo")).unwrap();
    std::fs::write(krate.join(".cargo/config.toml"), "[build]\nrustflags = [\"--cfg\", \"checked\"]\n").unwrap();
    std::fs::write(krate.join("src/carrier.in"), format!("core::arch::global_{}{}!(\"nop\");\n", "as", "m")).unwrap();
    std::fs::write(
        krate.join("src/lib.rs"),
        format!(
            "pub const N: &str = {inc} /* c */ {{\"../../note.txt\"}};\npub mod borrowed;\n#[path = \"carrier.in\"]\nmod carrier;\n#[cfg(windows)]\npub const W: &str = {inc}(\"../../win.txt\");\n#[cfg(not(debug_assertions))]\npub const R: &str = {inc}(\"../../rel.txt\");\n#[cfg(not(checked))]\npub const C: &str = {inc}(\"../../cfg.txt\");\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("outside_mod.rs"), krate.join("src/borrowed.rs")).unwrap();
    let here = krate.canonicalize().unwrap();
    let (mut out, mut all_read, mut checked) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for args in BUILDS {
        let (read, names) = files_rustc_read(&krate.join("Cargo.toml"), &base.join("target"), args);
        out.extend(outside(&read, &here).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()));
        all_read.extend(read);
        checked.extend(names);
    }
    assert_eq!(out.into_iter().collect::<Vec<_>>(), ["borrowed.rs", "cfg.txt", "note.txt", "rel.txt", "win.txt"]);
    let unchecked: Vec<(String, String)> = target_names(&krate).difference(&checked).cloned().collect();
    assert_eq!(unchecked, [("test".to_string(), "hidden".to_string()), ("test".to_string(), "veredas_aranha".to_string())]);
    assert_eq!(assembly_in(&all_read, &here), ["carrier.in: line 1"]);
    assert_eq!(configs_between(&here, &base.canonicalize().unwrap()), [here.join(".cargo/config.toml")]);
    assert_eq!(paths_under(&here, &is_link), [here.join("src/borrowed.rs")]);
    // A module whose name holds a line break: the dep-info line it breaks names a piece that is no
    // file, and the check fails; the name itself is refused.
    let broken = base.join("broken");
    std::fs::create_dir_all(broken.join("src")).unwrap();
    std::fs::write(broken.join("Cargo.toml"), manifest("")).unwrap();
    std::fs::write(broken.join("src/lib.rs"), "#[path = \"notes\n.in\"]\nmod notes;\n").unwrap();
    std::fs::write(broken.join("src/notes\n.in"), format!("pub const X: &str = {inc}(\"../../note.txt\");\n")).unwrap();
    let read = std::panic::catch_unwind(|| files_rustc_read(&broken.join("Cargo.toml"), &base.join("target"), BUILDS[0]));
    assert!(read.is_err(), "a line broken in dep-info went unseen");
    let b = broken.canonicalize().unwrap();
    assert_eq!(paths_under(&b, &control_in_name), [b.join("src/notes\n.in")]);
    std::fs::remove_dir_all(&base).unwrap();
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
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("component-planted-{case}"));
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
    let names: BTreeMap<&str, &str> =
        meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap())).collect();
    reached_ids(meta, root).iter().map(|id| names[id.as_str()].to_string()).collect()
}

/// The members of the workspace a crate depending on `root` compiles, by the workspace's own
/// list of members (whatever their names), `allowed` excepted: their names.
fn members_reached(meta: &Value, root: &str, allowed: &[&str]) -> Vec<String> {
    let names: BTreeMap<&str, &str> =
        meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap())).collect();
    let members: BTreeSet<&str> = meta["workspace_members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    let mut out: Vec<String> = reached_ids(meta, root)
        .iter()
        .filter(|id| members.contains(id.as_str()))
        .map(|id| names[id.as_str()].to_string())
        .filter(|n| !allowed.contains(&n.as_str()))
        .collect();
    out.sort();
    out
}

fn reached_ids(meta: &Value, root: &str) -> BTreeSet<String> {
    let nodes: BTreeMap<&str, &Value> =
        meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
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
    seen
}

/// The proc macros a build or a test of `root` compiles: they run at compile time and may read
/// files that rustc's dep-info does not name, so they are named here. The root's normal, build and
/// dev dependencies are followed, and theirs normal and build.
fn proc_macros_used(meta: &Value, root: &str) -> Vec<String> {
    let nodes: BTreeMap<&str, &Value> = meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let packages: BTreeMap<&str, &Value> = meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p)).collect();
    let mut seen = BTreeSet::new();
    let mut todo = vec![(root.to_string(), true)];
    while let Some((id, is_root)) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for dep in nodes[id.as_str()]["deps"].as_array().unwrap() {
            let used = dep["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null() || k["kind"] == "build" || (is_root && k["kind"] == "dev"));
            if used {
                todo.push((dep["pkg"].as_str().unwrap().to_string(), false));
            }
        }
    }
    let mut out: Vec<String> = seen
        .iter()
        .map(|id| packages[id.as_str()])
        .filter(|p| p["targets"].as_array().unwrap().iter().any(|t| t["kind"].as_array().unwrap().iter().any(|k| k == "proc-macro")))
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn the_proc_macros_its_builds_and_tests_use_are_named() {
    // A proc macro reads what it likes at compile time, outside the dep-info proof: the ones
    // aranha's builds and tests use are these, so a new one is seen and weighed. They are read
    // from the split, whose manifest is aranha's (the split test holds it so): there core is
    // resolved as aranha takes it, without its plant-facing layers, where the workspace's own
    // resolve gives core the features every member asks of it together.
    let split = Split::plant_at("proc-macros", &[]);
    let manifest = split.base.join("ws/rest/Cargo.toml");
    let meta = metadata(&manifest, &["--all-features", "--offline"]);
    assert_eq!(
        proc_macros_used(&meta, &root_id(&meta, &manifest)),
        ["displaydoc", "serde_derive", "thiserror-impl", "yoke-derive", "zerofrom-derive", "zerovec-derive"]
    );
    // The walk sees one a test alone uses, and not one only a dependency's tests would.
    let pkg = |name: &str, extra: &str| format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}");
    let dir = planted(
        "procmacros",
        &[
            ("Cargo.toml", "[workspace]\nmembers = [\"root\", \"a\", \"pm\", \"pm2\"]\nresolver = \"3\"\n".into()),
            ("root/Cargo.toml", pkg("root", "[dependencies]\na = { path = \"../a\" }\n[dev-dependencies]\npm = { path = \"../pm\" }\n")),
            ("root/src/lib.rs", String::new()),
            ("a/Cargo.toml", pkg("a", "[dev-dependencies]\npm2 = { path = \"../pm2\" }\n")),
            ("a/src/lib.rs", String::new()),
            ("pm/Cargo.toml", pkg("pm", "[lib]\nproc-macro = true\n")),
            ("pm/src/lib.rs", String::new()),
            ("pm2/Cargo.toml", pkg("pm2", "[lib]\nproc-macro = true\n")),
            ("pm2/src/lib.rs", String::new()),
        ],
    );
    let manifest = dir.join("root/Cargo.toml");
    let meta = metadata(&manifest, &["--all-features", "--offline"]);
    let used = proc_macros_used(&meta, &root_id(&meta, &manifest));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(used, ["pm"]);
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
    // No member of the workspace but core, whatever its name (a tool's, a crate cargo took in).
    let root = root_id(&meta, &manifest);
    assert_eq!(members_reached(&meta, &root, &["veredas-aranha", "veredas-core"]), Vec::<String>::new());
    // The ban list reads names by prefix, the scanner and core excepted.
    assert_eq!(
        banned_among(["duckdb", "libduckdb-sys", "axum-core", "hyper-util", "egui_extras", "veredas-portal", "veredas-maquinar", "orcl-std", "veredas-core", "veredas-aranha", "tokio", "serde"]),
        ["duckdb", "libduckdb-sys", "axum-core", "hyper-util", "egui_extras", "veredas-portal", "veredas-maquinar", "orcl-std"]
    );
}

#[test]
fn the_walk_follows_what_ships_and_leaves_dev_dependencies_to_the_tests() {
    // A planted graph: the root uses `a` normally and `b` to build; `c` only in its tests; `a`
    // uses `d`, which only `c` would otherwise reach, and builds with `i`; `b` uses `h`.
    let dir = planted(
        "walk",
        &[
            ("Cargo.toml", "[workspace]\nmembers = [\"root\", \"a\", \"b\", \"c\", \"d\", \"e\", \"f\", \"g\", \"h\", \"i\"]\nresolver = \"3\"\n".into()),
            ("h/Cargo.toml", "[package]\nname = \"h\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("h/src/lib.rs", String::new()),
            ("i/Cargo.toml", "[package]\nname = \"i\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("i/src/lib.rs", String::new()),
            (
                "root/Cargo.toml",
                "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[features]\nextra = [\"dep:e\"]\n[dependencies]\na = { path = \"../a\" }\ne = { path = \"../e\", optional = true }\n[build-dependencies]\nb = { path = \"../b\" }\n[dev-dependencies]\nc = { path = \"../c\" }\n[target.'cfg(windows)'.dependencies]\nf = { path = \"../f\" }\n[target.'cfg(target_arch = \"aarch64\")'.dependencies]\ng = { path = \"../g\" }\n".into(),
            ),
            ("e/Cargo.toml", "[package]\nname = \"e\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("e/src/lib.rs", String::new()),
            ("f/Cargo.toml", "[package]\nname = \"f\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("f/src/lib.rs", String::new()),
            ("g/Cargo.toml", "[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("g/src/lib.rs", String::new()),
            ("root/src/lib.rs", String::new()),
            ("a/Cargo.toml", "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nd = { path = \"../d\" }\n[build-dependencies]\ni = { path = \"../i\" }\n".into()),
            ("a/src/lib.rs", String::new()),
            ("b/Cargo.toml", "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nh = { path = \"../h\" }\n".into()),
            ("b/src/lib.rs", String::new()),
            ("c/Cargo.toml", "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("c/src/lib.rs", String::new()),
            ("d/Cargo.toml", "[package]\nname = \"d\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".into()),
            ("d/src/lib.rs", String::new()),
        ],
    );
    let manifest = dir.join("root/Cargo.toml");
    let meta = metadata(&manifest, &["--all-features", "--offline"]);
    let root = root_id(&meta, &manifest);
    let graph = reached(&meta, &root);
    let members = members_reached(&meta, &root, &["root"]);
    std::fs::remove_dir_all(&dir).unwrap();
    // An optional dependency behind a feature, and dependencies on Windows and on another
    // architecture, all reached: every feature on, every platform.
    // A build-dependency's dependency (h, b's) and a dependency's build-dependency (i, a's) too.
    assert_eq!(graph.into_iter().collect::<Vec<_>>(), ["a", "b", "d", "e", "f", "g", "h", "i", "root"]);
    // Every one of them a member of that workspace, named however: each reached member is named.
    assert_eq!(members, ["a", "b", "d", "e", "f", "g", "h", "i"]);
}

/// A manifest's dependencies (name, kind, platform; the codecs' crate left out) and its features.
fn manifest_deps(manifest: &Path) -> (BTreeSet<(String, String, String)>, String) {
    let meta = metadata(manifest, &["--no-deps", "--offline"]);
    let id = root_id(&meta, manifest);
    let package = meta["packages"].as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).unwrap().clone();
    let deps = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let field = |k: &str| d[k].as_str().unwrap_or("").to_string();
            (field("name"), field("kind"), field("target"))
        })
        .filter(|(n, _, _)| n != "codecs-alone")
        .collect();
    (deps, package["features"].to_string())
}

/// What keeps a folder of codecs from moving as it is: an entry that is not a plain file (a folder,
/// a link), and a file with a `cfg` of anything but `cfg(test)`, each by its name.
fn codec_folder_breaches(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !path.symlink_metadata().unwrap().file_type().is_file() {
            out.push(format!("{name}: not a flat file"));
            continue;
        }
        let flat: String = std::fs::read_to_string(&path).unwrap().chars().filter(|c| !c.is_whitespace()).collect();
        if flat.matches("cfg").count() != flat.matches("cfg(test)").count() {
            out.push(format!("{name}: cfg"));
        }
    }
    out.sort();
    out
}

/// Copy the tree at `from` into `to`, leaving out `skip` (a folder under `from`).
fn copy_tree(from: &Path, to: &Path, skip: Option<&Path>) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        if Some(path.as_path()) == skip {
            continue;
        }
        let target = to.join(path.file_name().unwrap());
        assert!(!is_link(&path), "a link would be carried, not its file: {}", path.display());
        if path.is_dir() {
            copy_tree(&path, &target, skip);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The codecs as a crate of their own beside the rest of aranha built on them as another crate,
/// planted under the folder cargo gives integration tests; `poison` edits one file first.
struct Split {
    base: PathBuf,
}

impl Split {
    fn plant(poisons: &[(&str, &str)]) -> Split {
        Split::plant_at("codecs-alone", poisons)
    }

    /// The split under a folder of this name, so two tests do not plant one folder at once.
    fn plant_at(name: &str, poisons: &[(&str, &str)]) -> Split {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let ws = base.join("ws");
        let _ = std::fs::remove_dir_all(&ws);
        let src = manifest_dir().join("src");
        let write = |p: PathBuf, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"codecs\", \"rest\"]\nresolver = \"3\"\n");
        // The codecs: src/profinet as it is, depending on serde and thiserror alone.
        write(
            ws.join("codecs/Cargo.toml"),
            "[package]\nname = \"codecs-alone\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nthiserror = \"2\"\n",
        );
        write(ws.join("codecs/src/lib.rs"), "#![deny(warnings)]\npub mod profinet;\n");
        copy_tree(&src.join("profinet"), &ws.join("codecs/src/profinet"), None);
        // The rest: aranha's own manifest's dependencies, its sources but the codecs, its tests,
        // the codecs taken from the other crate.
        write(
            ws.join("rest/Cargo.toml"),
            &format!("[package]\nname = \"veredas-aranha\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\ncodecs-alone = {{ path = \"../codecs\" }}\nserde = {{ version = \"1\", features = [\"derive\"] }}\nthiserror = \"2\"\nveredas-core = {{ path = {:?}, default-features = false }}\ntoml = \"1\"\nserde_ignored = \"0.1\"\n\n[target.'cfg(target_os = \"linux\")'.dependencies]\nlibc = \"0.2\"\n\n[dev-dependencies]\nicu_normalizer = {{ version = \"2\", default-features = false, features = [\"compiled_data\"] }}\nicu_properties = {{ version = \"2\", default-features = false, features = [\"compiled_data\"] }}\nserde_json = \"1\"\n",
                repo_root().join("crates/veredas-core").display().to_string()
            ),
        );
        copy_tree(&src, &ws.join("rest/src"), Some(&src.join("profinet")));
        copy_tree(&manifest_dir().join("tests"), &ws.join("rest/tests"), None);
        let lib = std::fs::read_to_string(ws.join("rest/src/lib.rs")).unwrap();
        assert_eq!(lib.matches("pub mod profinet;").count(), 1, "{lib}");
        write(ws.join("rest/src/lib.rs"), &lib.replace("pub mod profinet;", "pub use codecs_alone::profinet;"));
        for (file, text) in poisons {
            let f = ws.join(file);
            let old = std::fs::read_to_string(&f).unwrap();
            write(f, &format!("{old}\n{text}\n"));
        }
        Split { base }
    }

    /// `cargo <args>` on the planted workspace, offline, into a target folder kept between runs (its
    /// crates are named apart from the real one's tests, so nothing built there is taken for them).
    fn cargo(&self, args: &[&str]) -> std::process::Output {
        cargo()
            .args(args)
            .args(["--offline", "--quiet", "--manifest-path"])
            .arg(self.base.join("ws/Cargo.toml"))
            .env("CARGO_TARGET_DIR", self.base.join("target"))
            .output()
            .unwrap()
    }
}

#[test]
fn the_codecs_build_alone_and_the_rest_of_aranha_builds_on_them_as_another_crate() {
    // src/profinet/ holds flat files (no folder, no link), and no `cfg` but `cfg(test)`; code
    // bound to a platform without a `cfg` (an architecture's module, a pointer width, an
    // alignment) is what the builds of other architectures below find.
    assert_eq!(codec_folder_breaches(&manifest_dir().join("src/profinet")), Vec::<String>::new());
    // The check can fail: a cfg of a platform, a cfg_attr, a cfg! macro, a folder, a link.
    let poisoned = planted(
        "codec-folder",
        &[
            ("a.rs", "#[cfg(windows)]\npub fn a() {}\n".to_string()),
            ("b.rs", "#[cfg_attr(test, derive(Debug))]\npub struct B;\n".to_string()),
            ("c.rs", "pub fn c() -> bool {\n    cfg!(windows)\n}\n".to_string()),
            ("d.rs", "#[cfg(test)]\nmod t {}\n".to_string()),
            ("sub/e.rs", String::new()),
        ],
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(poisoned.join("d.rs"), poisoned.join("f.rs")).unwrap();
    let found = codec_folder_breaches(&poisoned);
    std::fs::remove_dir_all(&poisoned).unwrap();
    assert_eq!(found, ["a.rs: cfg", "b.rs: cfg", "c.rs: cfg", "f.rs: not a flat file", "sub: not a flat file"]);
    // Every target of both crates (libraries, unit tests, aranha's integration tests) checks, and
    // the codecs' doctests run.
    let split = Split::plant(&[]);
    // The rest's manifest names what aranha's does (each dependency with its kind and platform,
    // and the features), the codecs' crate besides.
    assert_eq!(manifest_deps(&split.base.join("ws/rest/Cargo.toml")), manifest_deps(&manifest_dir().join("Cargo.toml")));
    // The comparison sees a kind, a platform and a feature apart.
    let base = "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n";
    let reading = |extra: &str| {
        let dir = planted("deps", &[("Cargo.toml", format!("{base}{extra}")), ("src/lib.rs", String::new())]);
        let d = manifest_deps(&dir.join("Cargo.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
        d
    };
    let plain = reading("[dependencies]\nlibc = \"0.2\"\nserde = \"1\"\n");
    for other in [
        "[target.'cfg(target_os = \"linux\")'.dependencies]\nlibc = \"0.2\"\n[dependencies]\nserde = \"1\"\n",
        "[dependencies]\nlibc = \"0.2\"\n[build-dependencies]\nserde = \"1\"\n",
        "[dependencies]\nlibc = \"0.2\"\nserde = \"1\"\n[features]\nextra = []\n",
    ] {
        assert_ne!(reading(other), plain, "{other}");
    }
    // Every configuration a build of aranha may take: every feature on, in both profiles, and for
    // a second platform (Windows, where the plan's Npcap phase adds code to the rest).
    let checks: [&[&str]; 3] = [
        &["check", "--workspace", "--all-targets", "--all-features"],
        &["check", "--workspace", "--all-targets", "--all-features", "--release"],
        &["check", "--workspace", "--all-targets", "--all-features", "--target", "x86_64-pc-windows-gnu"],
    ];
    for args in checks {
        let out = split.cargo(args);
        assert!(out.status.success(), "the split does not build with {args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    let out = split.cargo(&["test", "--doc", "-p", "codecs-alone"]);
    assert!(out.status.success(), "the codecs' doctests:\n{}", String::from_utf8_lossy(&out.stderr));
    // The codecs alone, their unit tests with them, for a 32-bit target of another architecture
    // (wasm32) and for a 32-bit big-endian one without 64-bit atomics (powerpc): no code of one
    // platform hides in them without a `cfg`.
    let wasm: &[&str] = &["check", "-p", "codecs-alone", "--lib", "--tests", "--target", "wasm32-unknown-unknown"];
    let ppc: &[&str] = &["check", "-p", "codecs-alone", "--lib", "--tests", "--target", "powerpc-unknown-linux-gnu"];
    // And a 32-bit x86 one, where a 64-bit integer aligns to four bytes.
    let i686: &[&str] = &["check", "-p", "codecs-alone", "--lib", "--tests", "--target", "i686-unknown-linux-gnu"];
    for target in [wasm, ppc, i686] {
        let out = split.cargo(target);
        assert!(out.status.success(), "the codecs with {target:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    for (poison, target, want) in [
        ("pub fn lane() -> usize {\n    core::mem::size_of::<core::arch::x86_64::__m128i>()\n}", wasm, "x86_64"),
        ("const _: () = assert!(usize::BITS == 64);", wasm, "E0080"),
        ("#[cfg(test)]\nmod lane {\n    #[test]\n    fn width() {\n        assert_eq!(core::mem::size_of::<core::arch::x86_64::__m128i>(), 16);\n    }\n}", wasm, "x86_64"),
        ("const _: () = assert!(u16::from_ne_bytes([1, 0]) == 1);", ppc, "E0080"),
        ("pub static COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);", ppc, "AtomicU64"),
        ("const _: () = assert!(core::mem::align_of::<u64>() == 8);", i686, "E0080"),
    ] {
        let bound = Split::plant(&[("codecs/src/profinet/mod.rs", poison)]);
        let native = bound.cargo(checks[0]);
        assert!(native.status.success(), "{poison}: {}", String::from_utf8_lossy(&native.stderr));
        let out = bound.cargo(target);
        assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains(want), "{poison} went unseen with {target:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    // The check can fail, from either side: a codec test that reaches for the rest of the crate,
    // a codec that reaches for it, and the rest giving a codec type a method of its own (which it
    // could not do to core's type).
    let reach = "impl crate::profinet::mac::Mac {\n    pub fn heard(&self) -> bool {\n        true\n    }\n}";
    let gated = |gate: &str| format!("#[cfg({gate})]\n{reach}");
    let (windows, release, feature) = (gated("windows"), gated("not(debug_assertions)"), gated("feature = \"npcap\""));
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![("codecs/src/profinet/mod.rs", "#[cfg(test)]\nmod reach {\n    #[test]\n    fn f() {\n        let _ = crate::wire::raw::MAX_FRAME;\n    }\n}")], "wire"),
        (vec![("codecs/src/profinet/mod.rs", "pub use crate::wire::raw::RawLink;")], "wire"),
        (vec![("rest/src/lib.rs", reach)], "E0116"),
        // The same reach behind each configuration only some build takes.
        (vec![("rest/src/lib.rs", &windows)], "E0116"),
        (vec![("rest/src/lib.rs", &release)], "E0116"),
        (vec![("rest/Cargo.toml", "[features]\nnpcap = []"), ("rest/src/lib.rs", &feature)], "E0116"),
    ];
    for (poisons, want) in cases {
        let split = Split::plant(&poisons);
        let errs: Vec<String> =
            checks.iter().map(|a| split.cargo(a)).filter(|o| !o.status.success()).map(|o| String::from_utf8_lossy(&o.stderr).to_string()).collect();
        assert!(errs.iter().any(|e| e.contains(want)), "{poisons:?} went unseen: {errs:?}");
    }
    Split::plant(&[]);
}

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one. The same terms core's reader bans (`tests/foundation.rs`).
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// The terms, reversed: core's list, word for word (the test below holds it to core's file).
const TERMS: [&str; 6] = ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"];

/// A line's letters and digits as the reader sees them: Unicode's decomposition first (NFKD: an
/// accented letter its base letter and its marks, a full-width or compatibility form its plain
/// letters), lowercased, then every mark (by general category, whatever its combining class),
/// every default-ignorable code point (the invisible fillers among them) and everything that is no
/// letter or digit dropped.
fn letters(line: &str) -> Vec<char> {
    use icu_properties::props::{DefaultIgnorableCodePoint, GeneralCategory};
    use icu_properties::{CodePointMapData, CodePointSetData};
    let decomposed = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(line);
    let category = CodePointMapData::<GeneralCategory>::new();
    let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    decomposed
        .chars()
        .flat_map(char::to_lowercase)
        .flat_map(fold)
        .filter(|c| {
            let mark = matches!(category.get(*c), GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark | GeneralCategory::EnclosingMark);
            c.is_alphanumeric() && !mark && !ignorable.contains(*c)
        })
        .collect()
}

/// The Latin letters NFKD leaves whole, read as their base letters: supervisão's table
/// (`crates/veredas-supervisao/tests/component.rs`).
fn fold(c: char) -> Vec<char> {
    let plain = match c {
        'æ' => "ae",
        'ð' | 'đ' => "d",
        'ø' => "o",
        'þ' => "th",
        'ß' => "ss",
        'ħ' => "h",
        'ı' => "i",
        'ĸ' => "k",
        'ł' => "l",
        'ŋ' => "n",
        'œ' => "oe",
        'ŧ' => "t",
        // The spacing modifier letters NFKD leaves (the apostrophe of `ŉ`, the half ring of `ẚ`).
        '\u{2b0}'..='\u{2ff}' => "",
        'ƒ' => "f",
        'ƶ' => "z",
        'ȷ' => "j",
        c => return vec![c],
    };
    plain.chars().collect()
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites a
/// vendor document, as `line N: <term>`: core's reader, reading through Unicode's decomposition.
/// The whole text is read as one string of letters and digits, so no separator, case, accent,
/// letter form or line break hides a name; a section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> = TERMS.iter().map(|b| rev(b)).collect();
    let mut flat = Vec::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in letters(line) {
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
    for (n, line) in text.lines().enumerate() {
        if has_section_sign(line) {
            hits.push(format!("line {}: section sign", n + 1));
        }
    }
    hits
}

/// Whether a line holds the section sign as itself, or as an entity, an escape or a percent
/// encoding writes it: its named entity, whatever follows it (with its semicolon or without; HTML
/// reads the legacy name even before a letter or digit, the name then 4.2 as the sign and 4.2), a
/// numeric entity in decimal or hex with any leading zeros, a Rust escape in braces (any zeros,
/// underscores), a JSON or TOML escape of four or eight hex digits, a byte escape, and its UTF-8
/// bytes percent-encoded. The sign's spellings are decoded, not listed (and are written here in
/// pieces, so this file holds none whole).
fn has_section_sign(line: &str) -> bool {
    if line.contains(char::from_u32(0xA7).unwrap()) {
        return true;
    }
    let named = format!("&{}", "sect");
    let percent = format!("%c2%{}", "a7");
    let l = line.to_lowercase();
    let b = l.as_bytes();
    let hex_value = |digits: &str| u32::from_str_radix(digits, 16).ok();
    let mut i = 0;
    while i < b.len() {
        let rest = &l[i..];
        let found = if rest.starts_with(named.as_str()) {
            true
        } else if let Some(r) = rest.strip_prefix("&#x") {
            let d: String = r.chars().take_while(char::is_ascii_hexdigit).collect();
            !d.is_empty() && hex_value(&d) == Some(0xA7)
        } else if let Some(r) = rest.strip_prefix("&#") {
            let d: String = r.chars().take_while(char::is_ascii_digit).collect();
            !d.is_empty() && d.parse::<u32>().ok() == Some(167)
        } else if let Some(r) = rest.strip_prefix("\\u{") {
            let inner: String = r.chars().take_while(|c| *c != '}').filter(|c| *c != '_').collect();
            hex_value(&inner) == Some(0xA7)
        } else if let Some(r) = rest.strip_prefix("\\u") {
            let d: String = r.chars().take_while(char::is_ascii_hexdigit).take(8).collect();
            (d.len() >= 4 && hex_value(&d[..4]) == Some(0xA7)) || (d.len() == 8 && hex_value(&d) == Some(0xA7))
        } else if let Some(r) = rest.strip_prefix("\\x") {
            let d: String = r.chars().take(2).collect();
            d.chars().count() == 2 && hex_value(&d) == Some(0xA7)
        } else {
            rest.starts_with(percent.as_str()) || rest.starts_with(&format!("%{}", "a7"))
        };
        if found {
            return true;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    false
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

/// A folder's files read as the vendor reader reads them: those that are not text, or whose name is
/// not (a Windows code page, Latin-1 bytes would hide a spelling, so they are refused), and every
/// mention in a file's contents and in its name, its path from the folder.
fn read_folder(dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    crate_files(dir, &mut files);
    let text_of = |f: &PathBuf| String::from_utf8(std::fs::read(f).unwrap()).ok();
    let name_of = |f: &PathBuf| f.strip_prefix(dir).unwrap().to_str().map(str::to_string);
    let not_text: Vec<String> = files.iter().filter(|f| text_of(f).is_none() || name_of(f).is_none()).map(|f| f.display().to_string()).collect();
    let found: Vec<String> = files
        .iter()
        .filter(|f| text_of(f).is_some() && name_of(f).is_some())
        .flat_map(|f| {
            let (text, name) = (text_of(f).unwrap(), name_of(f).unwrap());
            let named = vendor_mentions(&name).into_iter().map(move |l| format!("the name {}: {l}", f.display()));
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display())).chain(named)
        })
        .collect();
    (not_text, found)
}

/// Where a Rust text names assembly (the three assembly macros, as words): an assembler
/// reads files (`.incbin`) rustc's dep-info does not name, so the crate holds none.
fn assembly(text: &str) -> Vec<String> {
    let words = [format!("{}{}", "as", "m"), format!("global_{}{}", "as", "m"), format!("naked_{}{}", "as", "m")];
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| words.iter().any(|x| x == w)))
        .map(|(n, _)| format!("line {}", n + 1))
        .collect()
}

/// The doctests rustdoc finds in the package at `manifest` (whose files neither the dep-info check
/// nor the split builds), by rustdoc's own list: however a doc comment, a doc attribute or a macro
/// writes one, and in whatever file.
fn doctests_listed(manifest: &Path, target_dir: &Path, args: &[&str]) -> Vec<String> {
    let out = cargo()
        .args(["test", "--doc", "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .args(args)
        .args(["--", "--list"])
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "rustdoc's list {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| l.ends_with(": test")).map(str::to_string).collect()
}

#[test]
fn aranha_holds_no_assembly_and_no_doctest() {
    let mut files = Vec::new();
    crate_files(&manifest_dir(), &mut files);
    // Every file under it, whatever its name (a module may be any file), read as text.
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            assembly(&text).into_iter().map(move |l| format!("{}: {l}", f.display())).collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // rustdoc finds no doctest in it, in every build checked.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("doclist");
    for args in BUILDS {
        assert_eq!(doctests_listed(&manifest_dir().join("Cargo.toml"), &target, args), Vec::<String>::new(), "{args:?}");
    }
    // And finds each of five a text check would miss: an indented block, a doc attribute after
    // another on its line, a doc block comment mid-line, a fence opened in the comment after one
    // left open, and a doctest in a module file not named `.rs`.
    let (fence, txt) = (format!("{}{}", "``", "`"), "text");
    let lib = format!(
        "/// v1\n///\n///     let a = 1;\npub fn v1() {{}}\n\
         #[inline] #[doc = \"{fence}\\nlet b = 2;\\n{fence}\"]\npub fn v3() {{}}\n\
         pub const BEFORE: u8 = 0; /** {fence}\nlet c = 3;\n{fence} */ pub const V6: u8 = 0;\n\
         /// {fence}{txt}\n/// open\npub fn v8a() {{}}\n/// {fence}\n/// let d = 4;\npub fn v8b() {{}}\n\
         #[path = \"carrier.in\"]\npub mod carrier;\n\
         #[cfg(windows)]\n/// {fence}\n/// let w = 6;\n/// {fence}\npub fn vw() {{}}\n\
         #[cfg(feature = \"extra\")]\n/// {fence}\n/// let x = 7;\n/// {fence}\npub fn vx() {{}}\n"
    );
    let dir = planted(
        "doctests",
        &[("Cargo.toml", manifest("[features]\nextra = []\n")), ("src/lib.rs", lib), ("src/carrier.in", format!("/// {fence}\n/// let e = 5;\n/// {fence}\npub fn v9() {{}}\n"))],
    );
    // Each build's own list: one behind Windows, one behind a feature, listed only where built.
    let mut listed = BTreeSet::new();
    for args in BUILDS {
        listed.extend(doctests_listed(&dir.join("Cargo.toml"), &Path::new(env!("CARGO_TARGET_TMPDIR")).join("doclist-planted"), args));
    }
    let host = doctests_listed(&dir.join("Cargo.toml"), &Path::new(env!("CARGO_TARGET_TMPDIR")).join("doclist-planted"), &[]);
    std::fs::remove_dir_all(&dir).unwrap();
    for item in ["v1", "v3", "V6", "v8b", "v9", "vw", "vx"] {
        assert!(listed.iter().any(|l| l.contains(&format!(" {item} ")) || l.contains(&format!("::{item} "))), "{item} not listed: {listed:?}");
    }
    assert_eq!(listed.len(), 7, "{listed:?}");
    assert_eq!(host.len(), 5, "the host's default build alone misses two: {host:?}");
    // The checks can fail: assembly however reached, and each way a doctest is written.
    let a = format!("{}{}", "as", "m");
    for text in [
        format!("core::arch::global_{a}!(\".incbin \\\"x\\\"\");"),
        format!("use std::arch::{a} as z;"),
        format!("unsafe {{ {a}!(\"nop\") }}"),
        format!("m!({a});"),
        format!("#[unsafe(naked)] fn f() {{ core::arch::naked_{a}!(\"ret\") }}"),
    ] {
        assert_eq!(assembly(&text), ["line 1"], "{text}");
    }
    for innocent in ["let plasma = 1;", "// asmara", "fn asm_like() {}"] {
        assert_eq!(assembly(innocent), Vec::<String>::new(), "{innocent}");
    }
}

#[test]
fn aranha_names_no_vendor_and_cites_no_vendor_document() {
    let mut files = Vec::new();
    crate_files(&manifest_dir(), &mut files);
    for f in ["Cargo.toml", "src/lib.rs", "src/profinet/dcp.rs", "tests/dcp.rs", "tests/common/mod.rs", "tests/component.rs"] {
        assert!(files.contains(&manifest_dir().join(f)), "{f} not read: {files:?}");
    }
    // Every file is text and has a name that is text; no contents and no name holds a mention.
    let (not_text, found) = read_folder(&manifest_dir());
    assert_eq!(not_text, Vec::<String>::new());
    assert_eq!(found, Vec::<String>::new());
    // The folder's reading can fail: a name holding a term, a file of Latin-1 bytes, a name that
    // is not text.
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vendor-planted");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a")).unwrap();
    std::fs::write(base.join("a").join(format!("{}.txt", rev("tlebineg"))), "nothing here").unwrap();
    std::fs::write(base.join("latin1.txt"), b"estac\xe3o").unwrap();
    #[cfg(unix)]
    std::fs::write(base.join(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b"c\xff.txt")), "x").unwrap();
    let (not_text, found) = read_folder(&base);
    std::fs::remove_dir_all(&base).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("the name ") && found[0].ends_with(&format!(": line 1: {}", rev("tlebineg"))), "{found:?}");
    let mut not_text: Vec<String> = not_text.iter().map(|p| Path::new(p).file_name().unwrap().to_string_lossy().to_string()).collect();
    not_text.sort();
    assert_eq!(not_text, ["c\u{fffd}.txt", "latin1.txt"]);
    // The letter table is supervisão's, held to its file; with it every letter of Latin-1 but µ
    // (NFKD makes it Greek's mu), every letter of Latin Extended-A, and the table's three of
    // Extended-B, read as plain letters.
    let body = |file: &str| -> String {
        let text = std::fs::read_to_string(file).unwrap();
        let start = text.find("fn fold(c: char) -> Vec<char> {").unwrap();
        let end = start + text[start..].find("c => return vec![c],").unwrap();
        text[start..end].lines().map(str::trim).collect::<Vec<_>>().join("\n")
    };
    let own = manifest_dir().join("tests/component.rs");
    let theirs = manifest_dir().join("../veredas-supervisao/tests/component.rs");
    assert_eq!(body(own.to_str().unwrap()), body(theirs.to_str().unwrap()));
    let not_plain: Vec<char> = ('\u{80}'..='\u{17f}')
        .chain(['ƒ', 'ƶ', 'ȷ'])
        .filter(|c| c.is_alphabetic() && *c != '\u{b5}')
        .filter(|c| !letters(&c.to_string()).iter().all(|l| l.is_ascii_lowercase()))
        .collect();
    assert_eq!(not_plain, Vec::<char>::new());
    // The terms are core's, held to core's own file.
    let core = std::fs::read_to_string(manifest_dir().join("../veredas-core/tests/foundation.rs")).unwrap();
    let list = format!("[{}]", TERMS.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", "));
    assert!(core.contains(&list), "core's list is not {list}");
    // The reader sees every spelling, across line breaks, and the citation sign.
    let fullwidth: String = rev("tlebineg").chars().map(|c| char::from_u32(c as u32 - 0x61 + 0xFF41).unwrap()).collect();
    let samples = [
        (format!("// the {} catalog", rev("tleBineG")), 1),
        (format!("// see {}/x.txt", rev("RODNEV/SCOD")), 1),
        (format!("fn x() {{}}\n/// the {}\n/// {} link", rev("troS retsaM"), rev("lortnoC")), 2),
        (format!("// a {}", rev("ecafretni-tsoh")), 1),
        (format!("// {} telegrams", rev("MAOOTXTR")), 1),
        (format!("// {} frames", rev("XTROTMAO")), 1),
        (format!("// clause {}4.1", char::from_u32(0xA7).unwrap()), 1),
        // Accented, and in full-width letters.
        (format!("// a device of the {} family", rev("tlebinég")), 1),
        (format!("// the {fullwidth} catalog"), 1),
        // A file's name.
        (format!("tests/fixtures/{}-io-device.bin", rev("tlebineg")), 1),
        // Latin letters NFKD leaves whole.
        (format!("// {}", rev("tlebineg").replacen('l', "\u{142}", 1)), 1),
        (format!("// {}", rev("rodnevscod").replace('o', "\u{f8}")), 1),
        (format!("// {}", rev("tlebineg").replacen('i', "\u{131}", 1)), 1),
        (format!("// {}", rev("ecafretnitsoh").replacen('t', "\u{167}", 1)), 1),
        // The citation sign as an entity and as escapes.
        (format!("/// clause &{}4.3", "sect;"), 1),
        (format!("/// clause &{} 4.3", "sect"), 1),
        (format!("let s = \"\\{}\";", "u{a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{0", "a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{0000", "a7}"), 1),
        (format!("let s = \"\\{}{}\";", "u{a_", "7}"), 1),
        (format!("/// clause &#{} 2", "167;"), 1),
        (format!("/// clause &#{} 2", "0167"), 1),
        (format!("/// clause &#{} 2", "x00a7;"), 1),
        (format!("description = \"\\{}{} 4.2\"", "u00", "A7"), 1),
        (format!("description = \"\\{}{} 4.2\"", "U000000", "A7"), 1),
        (format!("b\"\\{}{}\"", "x", "a7"), 1),
        (format!("https://x/{}%{}", "%C2", "A7"), 1),
        (format!("https://x/?q=%{}4.2", "A7"), 1),
        // The named entity before a digit, and at a line's end.
        (format!("/// clause &{}4.2 of the spec", "sect"), 1),
        (format!("/// see the clause &{}", "sect"), 1),
        // Invisible fillers and a mark of combining class 0 inside a term.
        (format!("// {}", rev("tlebineg").replacen('e', "e\u{3164}", 1)), 1),
        (format!("// {}", rev("tlebineg").replacen('e', "e\u{115f}", 1)), 1),
        (format!("// {}", rev("tlebineg").replacen('e', "e\u{ffa0}", 1)), 1),
        (format!("// {}", rev("tlebineg").replacen('n', "n\u{e34}", 1)), 1),
        // A spacing mark (Devanagari's vowel sign aa, general category Mc) inside a term.
        (format!("// {}", rev("tlebineg").replacen('e', "e\u{93e}", 1)), 1),
    ];
    for (s, line) in &samples {
        let hits = vendor_mentions(s);
        assert_eq!(hits.len(), 1, "{s}: {hits:?}");
        assert!(hits[0].starts_with(&format!("line {line}: ")), "{s}: {hits:?}");
    }
    assert_eq!(vendor_mentions("// the host's own interface"), Vec::<String>::new());
    // Near the sign's spellings, but not it (a byte escape before a letter past ASCII among them).
    for innocent in [format!("&{}", "sec;"), format!("&#{}", "168;"), format!("\\{}", "u00a8"), format!("{}{}", "%C2", "%A8"), "%A8".to_string(), format!("/// the escape \\{}\u{20ac} is not a byte", "xa")] {
        assert_eq!(vendor_mentions(&innocent), Vec::<String>::new(), "{innocent}");
    }
}
