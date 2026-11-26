//! Ábaco's own rules that no suite law checks (contract: Ábaco's component clause; the laws in
//! `tools/leis` judge its name, version, dependencies and plan).
//!
//! - A library, nothing else yet: cargo, asked what it would build (`cargo metadata --no-deps`,
//!   which builds nothing), lists one library of crate type `lib` and its tests.
//! - Its engine is its own fork, at `forks/ironcalc`: the two engine crates resolve to the fork's
//!   folder by path, never to a registry, and neither is a workspace member as cargo reads the
//!   workspace (a crate beneath a listed member's folder would be one whatever `exclude` says).
//! - The fork is upstream's `base` and `xlsx` at a known commit, then each local change a commit
//!   of its own, listed in `forks/ironcalc/VEREDAS-FORK.md`, read from git's history.
//! - It holds no plant connection (`docs/abaco-plan.md`, "Scope"): its normal dependency graph, on
//!   every platform, is a reviewed list of crates, none of them a member of the suite.
//! - The fork's own suite (upstream's tests and each local change's) passes in the fork's own
//!   workspace, upstream's eight ignored tests ignored and no other, on a lock that pins every
//!   crate of Ábaco's graph at the version and the source the suite builds, every one from
//!   crates.io, and with the engine's float math on the platform's functions in both builds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Cargo on the suite's manifest, `--locked`: a lock that does not match is an error, never
/// rewritten.
fn cargo(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO"))
        .args(args)
        .arg("--locked")
        .arg("--manifest-path")
        .arg(root().join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "cargo {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn git(args: &[&str]) -> String {
    git_in(&root(), args)
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The subjects of the commits after `base` that changed the engine, every one: a merge's other
/// side too, which git's default reading drops when the merge keeps one parent's tree (the round at
/// d7cb0b35), and the merge itself.
fn engine_commit_subjects(dir: &Path, base: &str) -> String {
    git_in(dir, &["log", "--full-history", "--format=%s", &format!("{base}..HEAD"), "--", "forks/ironcalc/base", "forks/ironcalc/xlsx"])
}

fn metadata() -> serde_json::Value {
    serde_json::from_str(&cargo(&["metadata", "--format-version", "1", "--no-deps", "--offline"])).unwrap()
}

fn package<'a>(meta: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap()
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

#[test]
fn a_library_and_tests_nothing_else() {
    let meta = metadata();
    let abaco = package(&meta, "veredas-abaco");
    assert_eq!(abaco["version"], "2.0.0-alpha.1");
    let mut libs = 0;
    for t in abaco["targets"].as_array().unwrap() {
        let kinds = strings(&t["kind"]);
        let types = strings(&t["crate_types"]);
        if kinds == ["test"] {
            continue;
        }
        assert_eq!((kinds.as_slice(), types.as_slice()), (["lib".to_string()].as_slice(), ["lib".to_string()].as_slice()), "{t}");
        libs += 1;
    }
    assert_eq!(libs, 1);
    let tests: BTreeSet<String> = abaco["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| strings(&t["kind"]) == ["test"])
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        tests,
        ["abacofile", "component", "links", "patch", "patch_corpus", "patch_no_styles", "patch_shapes", "patch_untouched"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
}

/// Ábaco's normal dependency graph as cargo prints it (`name vX.Y.Z`, a path package followed by its
/// folder in parentheses), each package once: on this host, or with `all` on every platform.
fn normal_graph(all_targets: bool) -> BTreeSet<String> {
    let mut args = vec!["tree", "-e", "normal", "--prefix", "none", "--format", "{p}", "-p", "veredas-abaco"];
    if all_targets {
        args.extend(["--target", "all"]);
    }
    cargo(&args)
        .lines()
        .map(|l| l.trim_end_matches(" (*)").trim_end_matches(" (proc-macro)").to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn names(graph: &BTreeSet<String>) -> BTreeSet<String> {
    graph.iter().filter_map(|p| p.split_whitespace().next()).map(|s| s.to_string()).collect()
}

#[test]
fn the_engine_is_the_fork_at_its_folder_and_no_member() {
    let graph = normal_graph(false);
    let fork = root().join("forks/ironcalc");
    for (name, folder) in [("ironcalc_base", "base"), ("ironcalc", "xlsx")] {
        let expected = format!("{name} v0.9.1 ({})", fork.join(folder).display());
        assert!(graph.contains(&expected), "{expected} not in {graph:?}");
    }
    // no other copy of the engine anywhere in the graph, on any platform, from a registry or
    // elsewhere
    let everywhere = normal_graph(true);
    let copies: Vec<&String> =
        everywhere.iter().filter(|p| p.starts_with("ironcalc_base ") || p.starts_with("ironcalc ")).collect();
    assert_eq!(copies.len(), 2, "{copies:?}");
    let meta = metadata();
    let members: Vec<String> = strings(&meta["workspace_members"]);
    assert!(members.iter().any(|m| m.contains("veredas-abaco")), "{members:?}");
    assert!(members.iter().all(|m| !m.contains("ironcalc")), "{members:?}");
    // the root manifest excludes the fork's folder by name
    let manifest: toml::Table = toml::from_str(&std::fs::read_to_string(root().join("Cargo.toml")).unwrap()).unwrap();
    let exclude = manifest["workspace"]["exclude"].as_array().unwrap();
    assert!(exclude.iter().any(|e| e.as_str() == Some("forks/ironcalc")), "{exclude:?}");
    // its direct normal dependencies: the engine, its file layer, the zip the patcher reads and
    // writes packages with, serialisation and the error derive, the `.abaco` file's TOML, its
    // references' SHA-256 and its check of keys nothing reads, the engine's two by path under the
    // fork's folder
    let abaco = package(&meta, "veredas-abaco");
    let normal: BTreeSet<String> = abaco["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["kind"].is_null())
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        normal,
        ["ironcalc", "ironcalc_base", "serde", "serde_ignored", "serde_json", "sha2", "thiserror", "toml", "zip"].iter().map(|s| s.to_string()).collect()
    );
    for d in abaco["dependencies"].as_array().unwrap() {
        if d["name"] == "ironcalc" || d["name"] == "ironcalc_base" {
            assert!(d["source"].is_null() && d["path"].as_str().unwrap().starts_with(fork.to_str().unwrap()), "{d}");
        }
    }
}

/// The trees of upstream's `base` and `xlsx` at `15ecfecb` (2026-11-02), as git hashes them.
const UPSTREAM_BASE_TREE: &str = "1aaecb54134a8942f02f467484b099f49417548d";
const UPSTREAM_XLSX_TREE: &str = "1556e78cd99f4e764f61d4a105d26b28f7b62f76";

#[test]
fn the_fork_is_upstream_at_its_commit_then_each_listed_change_a_commit() {
    // the commit that brought the fork in, found by what it added (a rebase changes hashes)
    let added = git(&["log", "--format=%H", "--diff-filter=A", "--", "forks/ironcalc/base/Cargo.toml"]);
    let added: Vec<&str> = added.lines().collect();
    assert_eq!(added.len(), 1, "the fork's base was added {} times: {added:?}", added.len());
    let base = added[0];
    assert_eq!(git(&["rev-parse", &format!("{base}:forks/ironcalc/base")]).trim(), UPSTREAM_BASE_TREE);
    assert_eq!(git(&["rev-parse", &format!("{base}:forks/ironcalc/xlsx")]).trim(), UPSTREAM_XLSX_TREE);
    // every later commit touching the engine is a change the record lists, one row each: its
    // subject names the change (`(fork change 7)`, a refinement `(fork change 1, refined)` row 1b)
    let later = engine_commit_subjects(&root(), base);
    // each commit names one change, never two
    for subject in later.lines() {
        assert!(subject.to_lowercase().matches("(fork change ").count() <= 1, "a commit names more than one change: {subject}");
    }
    let named = regex_free_change_ids(&later);
    let record = std::fs::read_to_string(root().join("forks/ironcalc/VEREDAS-FORK.md")).unwrap();
    let rows = change_rows(&record).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(named.len(), later.lines().count(), "a commit changed the engine without naming its change:\n{later}");
    let mut sorted_named = named.clone();
    sorted_named.sort();
    let mut sorted_rows = rows.clone();
    sorted_rows.sort();
    assert_eq!(sorted_named, sorted_rows, "the commits' changes and the record's rows");
    // and the working tree changes none of it unrecorded
    // and the working tree changes none of the fork (its manifests and lock included) unrecorded;
    // the scratch workbooks upstream's own tests write beside their crate while they run (the fork's
    // suite runs beside this test) are not a change
    let dirty = git(&["status", "--porcelain", "--untracked-files=all", "--", "forks/ironcalc"]);
    let dirty: Vec<&str> = dirty
        .lines()
        .filter(|l| {
            let scratch = ["?? forks/ironcalc/xlsx/", "?? forks/ironcalc/base/"]
                .iter()
                .any(|crate_dir| l.strip_prefix(crate_dir).is_some_and(|f| f.starts_with("temp_file_") && !f.contains('/')))
                || l.strip_prefix("?? ").is_some_and(|f| UPSTREAM_SCRATCH.contains(&f));
            !scratch
        })
        .collect();
    assert!(dirty.is_empty(), "the fork changed outside a commit:\n{}", dirty.join("\n"));
}

#[test]
fn an_engine_commit_a_merge_drops_from_git_s_default_reading_is_read_too() {
    // the round at d7cb0b35: a side branch's commit to the engine, merged `-s ours`, which git's
    // default reading of the engine's history drops; the check's reading lists it and the merge
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let run = |args: &[&str]| git_in(repo, &[&["-c", "user.name=critic", "-c", "user.email=critic@example.org", "-c", "commit.gpgsign=false"], args].concat());
    run(&["init", "-q", "-b", "main"]);
    let write = |path: &str, text: &str| {
        let at = repo.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    };
    write("forks/ironcalc/base/README.md", "upstream\n");
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "ironcalc: upstream at its commit"]);
    let base = run(&["rev-parse", "HEAD"]).trim().to_string();
    run(&["checkout", "-q", "-b", "side"]);
    write("forks/ironcalc/base/README.md", "changed, no change named\n");
    run(&["commit", "-q", "-am", "an unnamed engine change"]);
    run(&["checkout", "-q", "main"]);
    write("notes.md", "unrelated\n");
    run(&["add", "notes.md"]);
    run(&["commit", "-q", "-m", "an unrelated commit"]);
    run(&["merge", "-q", "-s", "ours", "--no-edit", "-m", "a merge keeping main's tree", "side"]);
    let default = run(&["log", "--format=%s", &format!("{base}..HEAD"), "--", "forks/ironcalc/base", "forks/ironcalc/xlsx"]);
    assert_eq!(default, "", "git's default reading drops the side's commit");
    let read = engine_commit_subjects(repo, &base);
    assert!(read.lines().any(|s| s == "an unnamed engine change") && read.lines().any(|s| s == "a merge keeping main's tree"), "{read}");
    assert!(regex_free_change_ids(&read).len() < read.lines().count(), "the check fails on it");
}

/// Ábaco's normal graph on every platform, by crate name, as reviewed on 2026-11-22: the engine,
/// its file layer and what they need (numbers, dates, zip and XML, statistics, regular
/// expressions, serialisation), the platform crates they pull in, and no plant stack (no OPC UA,
/// database, socket, HTTP, file-watching or async runtime crate). Added 2026-11-23 with the `.abaco`
/// file (C-224): TOML (`toml` and its `toml_datetime`, `toml_parser`, `toml_writer`,
/// `serde_spanned`, `winnow`), `serde_ignored` (the keys nothing reads) and SHA-256 (`sha2` and RustCrypto's `digest`, `block-buffer`,
/// `crypto-common`, `generic-array`, `typenum`), each pure Rust, none of the plant stack. A crate
/// joining the graph fails this test until it is reviewed and added.
const REVIEWED: &[&str] = &[
    "aho-corasick", "android_system_properties", "approx", "bitcode", "bitcode_derive", "bitflags",
    "block-buffer", "bumpalo", "bytemuck", "cfg-if", "chacha20", "chrono", "chrono-tz", "constant_time_eq",
    "core-foundation-sys", "cpufeatures", "crc32fast", "crypto-common", "csv", "csv-core", "digest", "either",
    "equivalent", "flate2", "futures-core", "futures-task", "futures-util", "generic-array", "getrandom",
    "hashbrown", "iana-time-zone", "iana-time-zone-haiku", "indexmap", "ironcalc", "ironcalc_base",
    "itertools", "itoa", "js-sys", "libc", "libm", "log", "memchr", "nix", "num-traits", "phf", "phf_shared",
    "pin-project-lite", "proc-macro2", "quick-xml", "quote", "r-efi", "rand", "rand_core", "regex",
    "regex-automata", "regex-lite", "regex-syntax", "ryu", "serde", "serde_core", "serde_derive",
    "serde_ignored", "serde_json", "serde_spanned", "sha2", "simd-adler32", "siphasher", "slab", "statrs",
    "syn", "thiserror", "thiserror-impl", "toml", "toml_datetime", "toml_parser", "toml_writer", "typed-path",
    "typenum", "unicode-ident", "veredas-abaco", "wasm-bindgen", "wasm-bindgen-macro",
    "wasm-bindgen-macro-support", "wasm-bindgen-shared", "windows-core", "windows-implement",
    "windows-interface", "windows-link", "windows-result", "windows-strings", "windows-sys", "winnow",
    "zeroize", "zeroize_derive", "zip", "zlib-rs", "zmij", "zopfli",
];

/// The crates of Ábaco's graph that are its own and not the engine's (the engine's graph, read from
/// cargo, holds none of them at the version listed, a bare name at any version): the `.abaco`
/// file's TOML, its check of keys nothing reads and its references' SHA-256 (C-224). The fork's
/// suite has nothing of them to test; every other crate of Ábaco's graph is the engine's, in the
/// fork's lock at the suite's version.
/// (`name@version` names one version of a crate the engine uses at another: sha2's `cpufeatures`.)
const ABACO_ONLY: &[&str] = &[
    "block-buffer", "cpufeatures@0.2.", "crypto-common", "digest", "generic-array", "serde_ignored", "serde_spanned",
    "sha2", "toml", "toml_datetime", "toml_parser", "toml_writer", "typenum", "winnow",
];

/// The crates Ábaco's graph gains in the suite's workspace build from features other members turn
/// on (cargo unifies a crate's features across the build), each reviewed: `miniz_oxide` and
/// `adler2`, flate2's second backend; `foldhash`, hashbrown's default hasher, which maquinar's
/// physics (`parry3d`) turns on, the hasher-free table indexmap uses unchanged. A crate joining
/// this way fails until it is reviewed and added. `const-oid` and `subtle` (2026-11-23): digest's
/// `oid` and `mac` features, which other members' cryptography turns on; SHA-256's output unchanged.
const WORKSPACE_EXTRA: &[&str] = &["adler2", "const-oid", "foldhash", "miniz_oxide", "subtle"];

/// The targets the suite is built and tested on: this machine's, the gate, and the Windows VM's.
const SUITE_TARGETS: [&str; 2] = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"];

/// The features Ábaco's crates carry in the suite's test build beyond Ábaco built alone, per
/// target, each reviewed (`.zen/evidence/C-220.md`): serde implementations (chrono, indexmap,
/// serde's and serde_core's `alloc`, which a `std` build has anyway), flate2's second backend and
/// what it turns on in miniz_oxide, hashbrown's default hasher (indexmap uses its hasher-free
/// table), num-traits' 128-bit integers, libc's trait implementations, serde_json's `RawValue`
/// (its parsing of numbers unchanged), syn's parsing and traversal features for the macros that use
/// syn 3 (`full`, `extra-traits`, `fold`, `visit`, `visit-mut`: the macros' output unchanged), and on Windows the
/// API bindings of windows-sys others call; and, with the `.abaco` file's SHA-256 (2026-11-23),
/// digest's `oid` and `mac` (object identifiers, `const-oid` with its database, and `subtle`'s
/// constant-time comparison) and crypto-common's random keys (`getrandom`, `rand_core` 0.6 and
/// their `std`), which others' cryptography turns on; and (2026-11-26, Oficina's window built by
/// default, Q370) proc-macro2's `span-locations`, which cxx-qt-build's C++ code generator turns on,
/// spans' lines and columns tracked at compile time (the derives' output unchanged), and
/// foldhash's `std`, its seed from the system, a hasher Ábaco's crates never call (indexmap's table
/// is hasher-free). None changes what the engine computes or a digest; the engine's own two crates
/// gain none. A feature joining fails until it is reviewed and added.
const REVIEWED_FEATURES: [(&str, &[(&str, &[&str])]); 2] = [
    ("x86_64-unknown-linux-gnu", &[
        ("chrono", &["serde"]),
        ("const-oid", &["db", "std"]),
        ("crypto-common", &["getrandom", "rand_core"]),
        ("digest", &["const-oid", "mac", "oid", "subtle"]),
        ("flate2", &["miniz_oxide", "rust_backend"]),
        ("foldhash", &["std"]),
        ("getrandom", &["std"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("libc", &["extra_traits"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
        ("proc-macro2", &["span-locations"]),
        ("rand_core", &["alloc", "getrandom", "std"]),
        ("serde", &["alloc"]),
        ("serde_core", &["alloc"]),
        ("serde_json", &["raw_value"]),
        ("sha2", &["oid"]),
        ("syn", &["extra-traits", "fold", "full", "visit", "visit-mut"]),
    ]),
    ("x86_64-pc-windows-msvc", &[
        ("chrono", &["serde"]),
        ("const-oid", &["db", "std"]),
        ("crypto-common", &["getrandom", "rand_core"]),
        ("digest", &["const-oid", "mac", "oid", "subtle"]),
        ("flate2", &["miniz_oxide", "rust_backend"]),
        ("foldhash", &["std"]),
        ("getrandom", &["std"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
        ("proc-macro2", &["span-locations"]),
        ("rand_core", &["alloc", "getrandom", "std"]),
        ("serde", &["alloc"]),
        ("serde_core", &["alloc"]),
        ("serde_json", &["raw_value"]),
        ("sha2", &["oid"]),
        ("syn", &["extra-traits", "fold", "full", "visit", "visit-mut"]),
        ("windows-sys", &[
            "Wdk", "Wdk_Foundation", "Wdk_Storage", "Wdk_Storage_FileSystem", "Wdk_System", "Wdk_System_IO",
            "Win32_Networking", "Win32_Networking_WinSock", "Win32_System", "Win32_System_Console",
            "Win32_System_Diagnostics", "Win32_System_Diagnostics_Debug", "Win32_System_IO",
            "Win32_System_LibraryLoader", "Win32_System_Pipes", "Win32_System_SystemInformation",
            "Win32_System_SystemServices", "Win32_System_Threading", "Win32_System_WindowsProgramming",
        ]),
    ]),
];

/// Each node of a `cargo tree --format '{p}|{f}'` listing (tree characters allowed): its crate,
/// its version and its features. A crate may stand at two versions, and one version may be built
/// twice (for the target, and for the host where a macro or a build script uses it) with other
/// features: each node is kept apart.
fn nodes<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<(String, String, BTreeSet<String>)> {
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim_start_matches(['│', '├', '└', '─', ' ']);
        let Some((package, features)) = line.rsplit_once('|') else { continue };
        let mut words = package.split_whitespace();
        let (name, version) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
        let features = features.trim_end_matches(" (*)").split(',').filter(|f| !f.is_empty()).map(str::to_string).collect();
        out.push((name, version, features));
    }
    out
}

#[test]
fn in_the_suites_workspace_build_its_graph_is_the_reviewed_one() {
    // the suite's build is a test build of every member: features unified with dev units on, on
    // each target it runs on (not all at once: no build combines every platform). Every member's
    // tree, each crate's dependencies repeated wherever it stands; Ábaco's is the block from its
    // root to its own dev-dependencies (its tests' crates, not its graph)
    for target in SUITE_TARGETS {
        let tree = cargo(&[
            "tree", "--workspace", "-e", "normal,dev", "--no-dedupe", "--prefix", "indent", "--format", "{p}|{f}", "--target", target,
        ]);
        let block: Vec<&str> = tree
            .lines()
            .skip_while(|l| !l.starts_with("veredas-abaco "))
            .take_while(|l| !l.is_empty() && !l.starts_with("[dev-dependencies]"))
            .collect();
        let in_suite = nodes(block.iter().copied());
        let crates: BTreeSet<&String> = in_suite.iter().map(|(n, _, _)| n).collect();
        assert!(["veredas-abaco", "ironcalc_base", "zip"].iter().all(|c| crates.iter().any(|n| n == c)), "{target}: {crates:?}");
        let unreviewed: Vec<&&String> =
            crates.iter().filter(|c| !REVIEWED.contains(&c.as_str()) && !WORKSPACE_EXTRA.contains(&c.as_str())).collect();
        assert!(unreviewed.is_empty(), "{target}: in the workspace's test build, not reviewed: {unreviewed:?}");
        // and the features: each node, beyond the same crate at the same version in Ábaco built
        // alone (whichever of its nodes there it is closest to), only reviewed ones
        let alone = nodes(
            cargo(&["tree", "-p", "veredas-abaco", "-e", "normal", "--no-dedupe", "--prefix", "none", "--format", "{p}|{f}", "--target", target])
                .lines(),
        );
        let reviewed = REVIEWED_FEATURES.iter().find(|(t, _)| *t == target).map(|(_, f)| *f).unwrap();
        for (name, version, features) in &in_suite {
            let allowed = reviewed.iter().find(|(n, _)| n == name).map(|(_, f)| *f).unwrap_or(&[]);
            let unreviewed = |base: &BTreeSet<String>| -> Vec<String> {
                features.iter().filter(|f| !base.contains(*f) && !allowed.contains(&f.as_str())).cloned().collect()
            };
            let none = BTreeSet::new();
            let least = alone
                .iter()
                .filter(|(n, v, _)| n == name && v == version)
                .map(|(_, _, f)| unreviewed(f))
                .min_by_key(|u| u.len())
                .unwrap_or_else(|| unreviewed(&none));
            assert!(least.is_empty(), "{target}: in the workspace's test build {name} {version} gains unreviewed features {least:?}");
        }
    }
}

/// A cargo process among those that started this test: the folder it was started in (cargo reads
/// that folder's configuration), its program, its arguments and its environment, which is what
/// cargo itself reads (a runner may give the test another).
struct Invoking {
    folder: PathBuf,
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// The folder of the toolchain it compiles with: a cargo's own folder; for cargo-nextest, the
    /// folder of the cargo that started it, which cargo hands every subcommand in `CARGO`.
    toolchain: PathBuf,
}

/// Every cargo among the processes that started this test, read on Linux from `/proc`, up the
/// whole chain of parent processes, any other (a runner, a script) passed over. Off Linux, none.
fn invoking_cargo() -> Vec<Invoking> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    assert!(Path::new("/proc/self/stat").exists(), "/proc cannot be read: the cargo that runs this test is out of sight");
    let mut out = Vec::new();
    let mut pid = std::process::id();
    while let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // the parent is the second field after the command, which is in parentheses
        let Some(ppid) = stat.rsplit_once(") ").and_then(|(_, rest)| rest.split_whitespace().nth(1)?.parse::<u32>().ok()) else {
            break;
        };
        if ppid == 0 {
            break;
        }
        let comm = std::fs::read_to_string(format!("/proc/{ppid}/comm")).unwrap_or_default();
        if matches!(comm.trim(), "cargo" | "cargo-nextest") {
            let split = |file: &str| -> Vec<String> {
                std::fs::read(format!("/proc/{ppid}/{file}"))
                    .unwrap_or_default()
                    .split(|b| *b == 0)
                    .filter(|a| !a.is_empty())
                    .map(|a| String::from_utf8_lossy(a).to_string())
                    .collect()
            };
            let program = std::fs::read_link(format!("/proc/{ppid}/exe")).unwrap_or_default();
            let env: Vec<(String, String)> =
                split("environ").into_iter().filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
            let toolchain = if comm.trim() == "cargo-nextest" {
                env.iter().find(|(k, _)| k == "CARGO").and_then(|(_, v)| Path::new(v).parent().map(Path::to_path_buf)).unwrap_or_default()
            } else {
                program.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            out.push(Invoking {
                folder: std::fs::read_link(format!("/proc/{ppid}/cwd")).unwrap_or_default(),
                program,
                args: split("cmdline"),
                env,
                toolchain,
            });
        }
        pid = ppid;
    }
    assert!(!out.is_empty(), "no cargo among the processes that started this test");
    out
}

/// Whether compiler flags (each a token) set a `cfg` the features would not show: a `--cfg`, or an
/// `@file` argument, which rustc expands into flags of its own.
fn flags_set_cfg<'a>(mut tokens: impl Iterator<Item = &'a str>) -> bool {
    tokens.any(|t| t == "--cfg" || t.starts_with("--cfg=") || t.starts_with('@'))
}

/// What an environment cargo runs in may not hold: an alias handing cargo a `--config`, a rustc
/// wrapper, a rustc that is not the toolchain's (`RUSTC`, `CARGO_BUILD_RUSTC`, else the first on
/// `PATH`: the toolchain's own, or rustup's proxy choosing the toolchain cargo is from), compiler
/// flags setting a `cfg` (`--cfg feature="mock_time"` would turn on the engine's fixed clock with no
/// feature cargo sees), unstable settings (which let profiles carry flags). `toolchain` is the
/// folder of that cargo's program.
fn check_environment(whose: &str, env: &[(String, String)], toolchain: &Path) {
    let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
    for (var, value) in env.iter().filter(|(k, _)| k.starts_with("CARGO_ALIAS_")) {
        assert!(!value.contains("--config"), "{whose}: {var} gives cargo a --config");
    }
    for var in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"] {
        assert!(get(var).is_none(), "{whose}: {var} is set: something compiles crates in rustc's place");
    }
    // stable cargo, stable settings: no unstable setting is set, the first one named
    let unstable = env.iter().find(|(k, v)| {
        !v.is_empty() && (k == "RUSTC_BOOTSTRAP" || k == "__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS" || k.starts_with("CARGO_UNSTABLE_"))
    });
    assert!(unstable.is_none(), "{whose}: {} is set: unstable settings", unstable.map_or("", |(k, _)| k.as_str()));
    let toolchain = toolchain.canonicalize().unwrap_or_else(|_| toolchain.to_path_buf());
    let beside = toolchain.join("rustc");
    // rustup's proxy runs the toolchain RUSTUP_TOOLCHAIN names, in RUSTUP_HOME: a name, the one
    // cargo itself is from
    let proxy_runs_cargos_own = || {
        let Some(name) = get("RUSTUP_TOOLCHAIN") else { return true };
        if name.contains('/') || name.contains('\\') {
            return false;
        }
        let home = get("RUSTUP_HOME").map(PathBuf::from).or_else(|| get("HOME").map(|h| Path::new(h).join(".rustup")));
        home.and_then(|h| h.join("toolchains").join(name).join("bin").join("rustc").canonicalize().ok())
            == beside.canonicalize().ok()
    };
    let own = |named: &Path| {
        let real = named.canonicalize().unwrap_or_else(|_| named.to_path_buf());
        if real.file_stem().is_some_and(|f| f == "rustup") {
            return proxy_runs_cargos_own();
        }
        real.parent() == Some(toolchain.as_path()) && real.file_stem().is_some_and(|f| f == "rustc")
    };
    match get("RUSTC").or(get("CARGO_BUILD_RUSTC")) {
        Some(named) => assert!(own(Path::new(named)), "{whose}: RUSTC is {named}: a rustc not the toolchain's ({})", toolchain.display()),
        None => {
            if let Some(path) = get("PATH") {
                let first = std::env::split_paths(path).map(|d| d.join("rustc")).find(|r| r.is_file());
                if let Some(first) = first {
                    assert!(own(&first), "{whose}: the first rustc on PATH, {}, is not the toolchain's ({})", first.display(), toolchain.display());
                }
            }
        }
    }
    if let Some(why) = loader_refusal(env, &toolchain, whose != "this test") {
        panic!("{whose}: {why}");
    }
    for (var, value) in env.iter().filter(|(k, _)| {
        k == "RUSTFLAGS"
            || k == "CARGO_ENCODED_RUSTFLAGS"
            || k == "CARGO_BUILD_RUSTFLAGS"
            || (k.starts_with("CARGO_TARGET_") && k.ends_with("_RUSTFLAGS"))
            || (k.starts_with("CARGO_PROFILE_") && k.ends_with("_RUSTFLAGS"))
    }) {
        let tokens: Vec<&str> = if var == "CARGO_ENCODED_RUSTFLAGS" { value.split('\x1f').collect() } else { value.split_whitespace().collect() };
        assert!(!flags_set_cfg(tokens.into_iter()), "{whose}: {var} sets a cfg: {value}");
    }
}

/// What of an environment would have the dynamic loader put other code into every rustc cargo starts
/// (the round at 444cead9: `LD_PRELOAD` handed rustc a copy of an engine source holding a socket,
/// the files on disk clean): `LD_PRELOAD`, `LD_AUDIT` or any `DYLD_` variable set, or (`library`,
/// for a cargo's environment) an `LD_LIBRARY_PATH` naming any folder but `toolchain`'s own `lib`,
/// where rustc finds its driver, the folder rustup's proxy names (a test binary's own is cargo's
/// business: its deps folders).
fn loader_refusal(env: &[(String, String)], toolchain: &Path, library: bool) -> Option<String> {
    let set = |k: &str| env.iter().any(|(n, v)| n == k && !v.is_empty());
    if let Some(var) = ["LD_PRELOAD", "LD_AUDIT"].into_iter().find(|k| set(k)) {
        return Some(format!("{var} is set: the loader puts other code into rustc"));
    }
    if let Some((var, _)) = env.iter().find(|(n, v)| n.starts_with("DYLD_") && !v.is_empty()) {
        return Some(format!("{var} is set: the loader puts other code into rustc"));
    }
    if library && let Some((_, value)) = env.iter().find(|(n, v)| n == "LD_LIBRARY_PATH" && !v.is_empty()) {
        let own = toolchain.parent().map(|t| t.join("lib"));
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let foreign: Vec<PathBuf> = std::env::split_paths(value).filter(|d| own.as_deref().is_none_or(|o| canon(d) != canon(o))).collect();
        if !foreign.is_empty() {
            return Some(format!("LD_LIBRARY_PATH names {foreign:?}, beside the toolchain's own lib: rustc may load another driver"));
        }
    }
    None
}

/// The keys of a cargo configuration's `[env]`, which cargo hands every rustc it starts, as reviewed
/// on this machine (the round at 444cead9): the browser supervisão's tests drive and the Qt build
/// tool; a key joining, `LD_PRELOAD` or `LD_LIBRARY_PATH` above all, fails until it is reviewed.
const CONFIG_ENV: &[&str] = &["QMAKE", "VEREDAS_BROWSER"];

/// What a cargo configuration may not hold, the first named: a top-level key that may patch or
/// replace crates, a rustc or a wrapper in rustc's place, compiler flags setting a `cfg`, an alias
/// giving cargo a configuration of its own, an `[env]` key not reviewed.
fn config_refusal(t: &toml::Table) -> Option<String> {
    let foreign: Vec<&String> = t.keys().filter(|k| !CONFIG_KEYS.contains(&k.as_str())).collect();
    if !foreign.is_empty() {
        return Some(format!("{foreign:?} (a key that may patch or replace crates)"));
    }
    // nothing compiles a crate in rustc's place
    let build = t.get("build").and_then(|b| b.as_table());
    if let Some(key) = ["rustc", "rustc-wrapper", "rustc-workspace-wrapper"].into_iter().find(|k| build.is_some_and(|b| b.contains_key(*k))) {
        return Some(format!("build.{key} set"));
    }
    // and no compiler flag sets a cfg (a feature cargo would not see), however it is carried
    let flags = |v: Option<&toml::Value>| match v {
        Some(toml::Value::String(s)) => flags_set_cfg(s.split_whitespace()),
        Some(toml::Value::Array(a)) => flags_set_cfg(a.iter().filter_map(|t| t.as_str())),
        _ => false,
    };
    if flags(build.and_then(|b| b.get("rustflags"))) {
        return Some("build.rustflags sets a cfg".into());
    }
    for (triple, table) in t.get("target").and_then(|t| t.as_table()).into_iter().flatten() {
        if flags(table.get("rustflags")) {
            return Some(format!("target.{triple}.rustflags sets a cfg"));
        }
    }
    for (name, table) in t.get("profile").and_then(|t| t.as_table()).into_iter().flatten() {
        if flags(table.get("rustflags")) {
            return Some(format!("profile.{name}.rustflags sets a cfg"));
        }
    }
    // and no alias gives cargo a configuration of its own
    for (name, value) in t.get("alias").and_then(|a| a.as_table()).into_iter().flatten() {
        if value.to_string().contains("--config") {
            return Some(format!("alias {name} gives cargo a --config"));
        }
    }
    // and what it hands rustc's environment is reviewed
    if let Some(env) = t.get("env") {
        let keys: Vec<String> = env.as_table().map(|e| e.keys().cloned().collect()).unwrap_or_else(|| vec![env.to_string()]);
        if let Some(key) = keys.iter().find(|k| !CONFIG_ENV.contains(&k.as_str())) {
            return Some(format!("env.{key} is no key reviewed: cargo hands it every rustc"));
        }
    }
    None
}

/// The top-level keys a cargo configuration of this machine may hold: none of them names where a
/// crate comes from (`patch`, `source`, `paths`, and `include`, which would bring any of them in
/// from another file, are not among them).
const CONFIG_KEYS: &[&str] = &[
    "alias", "build", "cache", "cargo-new", "credential-alias", "doc", "env", "future-incompat-report", "http", "install",
    "net", "profile", "registries", "registry", "resolver", "target", "term",
];

/// Names that would be a plant connection, for a clearer message when one joins.
const PLANT_STACK: &[&str] =
    &["async-opcua", "opcua", "duckdb", "libduckdb", "rusqlite", "socket2", "tokio", "mio", "axum", "hyper", "http", "reqwest", "notify", "inotify", "async-std", "smol"];

#[test]
fn no_plant_connection_and_no_member_in_its_normal_graph_on_any_platform() {
    let crates = names(&normal_graph(true));
    assert!(crates.contains("ironcalc_base") && crates.contains("zip"), "{crates:?}");
    // no member of the suite but itself: the members as cargo lists them, tools and the language
    // included
    let meta = metadata();
    let members: BTreeSet<String> = meta["packages"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    assert!(members.len() > 3 && members.contains("leis") && members.contains("veredas-core"), "{members:?}");
    let in_graph: Vec<&String> = crates.iter().filter(|c| members.contains(*c) && *c != "veredas-abaco").collect();
    assert!(in_graph.is_empty(), "members of the suite in Ábaco's graph: {in_graph:?}");
    let plant: Vec<&String> = crates.iter().filter(|c| PLANT_STACK.iter().any(|b| c.starts_with(b))).collect();
    assert!(plant.is_empty(), "a plant connection in Ábaco's graph: {plant:?}");
    let reviewed: BTreeSet<String> = REVIEWED.iter().map(|s| s.to_string()).collect();
    let new: Vec<&String> = crates.difference(&reviewed).collect();
    assert!(new.is_empty(), "crates not reviewed joined Ábaco's graph: {new:?}");
}

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The other scratch workbooks upstream's own tests write while the fork's suite runs, and remove
/// after (besides the `temp_file_*` ones beside each crate).
const UPSTREAM_SCRATCH: [&str; 4] = [
    "forks/ironcalc/xlsx/existing_file.xlsx",
    "forks/ironcalc/xlsx/temp_cf_round_trip.xlsx",
    "forks/ironcalc/xlsx/tests/conditional_formatting/cf_tests_round_trip.xlsx",
    "forks/ironcalc/xlsx/tests/templates/crossword_round_trip.xlsx",
];

/// The packages of a lock file that come from crates.io, `name` to its versions (one from a git
/// repository, a path or another registry is not counted, whatever its version).
fn locked(path: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let lock: toml::Table = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in lock["package"].as_array().unwrap() {
        if p.get("source").and_then(|s| s.as_str()) == Some(CRATES_IO) {
            out.entry(p["name"].as_str().unwrap().to_string()).or_default().insert(p["version"].as_str().unwrap().to_string());
        }
    }
    out
}

/// The fork's suite, as upstream's tests and the changes' count them: upstream's 3 192 and their 8
/// ignored at `15ecfec`, and the changes' 88 (the record's rows say what each change adds; change
/// 41's one).
const FORK_PASSED: usize = 3280;
const FORK_IGNORED: usize = 8;

/// Upstream's ignored tests at `15ecfec`, by name.
const UPSTREAM_IGNORED: [&str; 8] = [
    "formatter::test::test_de_examples::format_millions_scale_decimal_de",
    "formatter::test::test_en_examples::format_fraction_eighths",
    "formatter::test::test_en_examples::format_fraction_simple",
    "formatter::test::test_en_examples::format_fraction_two_digit_den",
    "formatter::test::test_en_examples::format_group_with_leading_hashes",
    "formatter::test::test_en_examples::format_scientific_failing",
    "test::test_fn_or_xor::spill_behaviour",
    "test::test_general::test_xlfn",
];

#[test]
fn no_crate_is_patched_or_replaced_by_any_route() {
    // not in the manifests: the suite's, nor the fork's own, whose suite would then build other
    // crates than Ábaco links
    for manifest in ["Cargo.toml", "forks/ironcalc/Cargo.toml"] {
        let m: toml::Table = toml::from_str(&std::fs::read_to_string(root().join(manifest)).unwrap()).unwrap();
        assert!(m.get("patch").is_none() && m.get("replace").is_none(), "{manifest} patches crates");
    }
    // nor in a cargo configuration cargo would read for either workspace or for this test: from
    // each folder up to the root of the drive, and cargo's home
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")));
    let mut configs: BTreeSet<PathBuf> = BTreeSet::new();
    let invoking = invoking_cargo();
    // rustup's proxy starts the toolchain's own cargo, which stands below it in the chain with the
    // environment it was given: that cargo's environment is the one judged
    let is_proxy = |c: &Invoking| c.program.canonicalize().ok().and_then(|p| p.file_stem().map(|f| f == "rustup")).unwrap_or(false);
    assert!(!cfg!(target_os = "linux") || invoking.iter().any(|c| !is_proxy(c)), "no toolchain's cargo among the processes that started this test");
    for cargo in &invoking {
        let given: Vec<&String> = cargo.args.iter().filter(|a| *a == "--config" || a.starts_with("--config=")).collect();
        assert!(given.is_empty(), "the cargo that runs this test ({}) was given {given:?}", cargo.program.display());
        if !is_proxy(cargo) {
            check_environment(&format!("the cargo in {}", cargo.folder.display()), &cargo.env, &cargo.toolchain);
        }
    }
    let own_env: Vec<(String, String)> = std::env::vars().collect();
    check_environment("this test", &own_env, Path::new(env!("CARGO")).parent().unwrap());
    // and the cargo homes each of them reads
    for cargo in &invoking {
        let get = |k: &str| cargo.env.iter().find(|(n, _)| n == k).map(|(_, v)| PathBuf::from(v));
        if let Some(home) = get("CARGO_HOME").or_else(|| get("HOME").map(|h| h.join(".cargo"))) {
            configs.insert(home.join("config"));
            configs.insert(home.join("config.toml"));
        }
    }
    let here = std::env::current_dir().unwrap();
    for start in [root(), root().join("forks/ironcalc"), PathBuf::from(env!("CARGO_MANIFEST_DIR")), here]
        .into_iter()
        .chain(invoking.iter().map(|c| c.folder.clone()))
    {
        for dir in start.ancestors() {
            for file in [".cargo/config", ".cargo/config.toml"] {
                configs.insert(dir.join(file));
            }
        }
    }
    if let Some(home) = home {
        configs.insert(home.join("config"));
        configs.insert(home.join("config.toml"));
    }
    // a toolchain file choosing a toolchain by its path (rustup would run whatever sits there)
    let toolchain_files: BTreeSet<PathBuf> = configs
        .iter()
        .filter_map(|c| c.parent()?.parent().map(Path::to_path_buf))
        .flat_map(|dir| [dir.join("rust-toolchain"), dir.join("rust-toolchain.toml")])
        .collect();
    for file in toolchain_files.iter().filter(|f| f.is_file()) {
        let text = std::fs::read_to_string(file).unwrap();
        let by_path = toml::from_str::<toml::Table>(&text)
            .ok()
            .is_some_and(|t| t.get("toolchain").and_then(|c| c.get("path")).is_some());
        assert!(!by_path, "{}: chooses a toolchain by its path", file.display());
    }
    for config in configs.iter().filter(|c| c.is_file()) {
        let t: toml::Table = toml::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        if let Some(why) = config_refusal(&t) {
            panic!("{}: {why}", config.display());
        }
    }

    // and in what the locks say, whatever route did it: every package from crates.io, or a path
    // package of that workspace (the suite's members and the fork's two crates; the fork's two
    // alone), no patch left unused
    let members: BTreeSet<String> = metadata()["packages"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    for (lock, own) in [
        ("Cargo.lock", members.iter().map(String::as_str).chain(["ironcalc", "ironcalc_base"]).collect::<BTreeSet<&str>>()),
        ("forks/ironcalc/Cargo.lock", ["ironcalc", "ironcalc_base"].into_iter().collect()),
    ] {
        let l: toml::Table = toml::from_str(&std::fs::read_to_string(root().join(lock)).unwrap()).unwrap();
        assert!(l.get("patch").is_none(), "{lock} records a patch");
        for p in l["package"].as_array().unwrap() {
            let name = p["name"].as_str().unwrap();
            match p.get("source").and_then(|s| s.as_str()) {
                Some(CRATES_IO) => {}
                None => assert!(own.contains(name), "{lock}: {name} is a path package of no member"),
                Some(other) => panic!("{lock}: {name} from {other}"),
            }
        }
    }
}

#[test]
fn a_configuration_s_env_and_the_loader_s_variables_are_held_to_what_was_reviewed() {
    // the round at 444cead9: cargo hands a configuration's `[env]` to every rustc it starts, and
    // `LD_PRELOAD` there had rustc compile a copy of an engine source holding a socket
    let config = |text: &str| config_refusal(&toml::from_str(text).unwrap());
    assert_eq!(config("[build]\njobs = 16\n[env]\nVEREDAS_BROWSER = \"/x\"\nQMAKE = \"/y\"\n"), None);
    for key in ["LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "DYLD_INSERT_LIBRARIES", "RUSTC_WRAPPER", "CARGO_BUILD_RUSTC"] {
        assert!(config(&format!("[env]\n{key} = \"/x\"\n")).is_some_and(|w| w.contains(key)), "{key}");
        assert!(config(&format!("[env]\n{key} = {{ value = \"/x\", force = true }}\n")).is_some(), "{key} as a table");
    }
    assert!(config("[build]\nrustc-wrapper = \"/x\"\n").is_some());
    let toolchain = Path::new("/home/a/.rustup/toolchains/stable/bin");
    let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    assert_eq!(loader_refusal(&env(&[("LD_LIBRARY_PATH", "/home/a/.rustup/toolchains/stable/lib")]), toolchain, true), None);
    assert!(loader_refusal(&env(&[("LD_LIBRARY_PATH", "/home/a/.rustup/toolchains/stable/lib:/tmp/x")]), toolchain, true).is_some());
    assert_eq!(loader_refusal(&env(&[("LD_LIBRARY_PATH", "/tmp/deps")]), toolchain, false), None, "a test binary's own");
    for var in ["LD_PRELOAD", "LD_AUDIT", "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH"] {
        assert!(loader_refusal(&env(&[(var, "/tmp/x.so")]), toolchain, false).is_some_and(|w| w.contains(var)), "{var}");
    }
    assert_eq!(loader_refusal(&env(&[("LD_PRELOAD", "")]), toolchain, true), None, "set empty, as unset");
}

/// The files a dep-info record lists, each on a line of its own ending in `:`, a space in its path
/// written `\ `; every one resolves to a file, or the record is not read as it is.
fn dep_info_files(dep_info: &Path, whose: &str) -> BTreeSet<PathBuf> {
    let text = std::fs::read_to_string(dep_info).unwrap_or_else(|e| panic!("{}: {e}", dep_info.display()));
    text.lines()
        .filter_map(|l| l.strip_suffix(':'))
        .filter(|f| !f.replace("\\ ", "").contains(' '))
        .map(|f| {
            let f = f.replace("\\ ", " ");
            let p = Path::new(&f);
            let p = if p.is_absolute() { p.to_path_buf() } else { root().join(p) };
            p.canonicalize().unwrap_or_else(|e| panic!("{whose}: {} in its dep-info: {e}", p.display()))
        })
        .collect()
}

/// The three packages whose code ships in Ábaco's library, each with its folder.
const SHIPPED: [(&str, &str); 3] = [("veredas-abaco", "crates/veredas-abaco"), ("ironcalc_base", "forks/ironcalc/base"), ("ironcalc", "forks/ironcalc/xlsx")];

/// A package's name from cargo's id (`path+file:///…/base#ironcalc_base@0.9.1`, or `…/veredas-abaco#2.0.0`
/// when the folder is named as the package).
fn package_of(id: &str) -> String {
    let (url, fragment) = id.rsplit_once('#').unwrap_or((id, ""));
    match fragment.split_once('@') {
        Some((name, _)) => name.to_string(),
        None => url.rsplit('/').next().unwrap_or("").to_string(),
    }
}

/// What one build of Ábaco's library shows of the three packages: every file rustc read to compile
/// each library and each build script (from their dep-info), and what each build script told cargo
/// (the `cfg`s it set, the native libraries it linked, and every instruction it printed, read from
/// the output cargo keeps of its run).
struct Built {
    libraries: Vec<(&'static str, BTreeSet<PathBuf>)>,
    scripts: Vec<(String, BTreeSet<PathBuf>)>,
    executed: Vec<(String, Vec<String>, Vec<String>, Vec<String>)>,
    /// Each build script's search paths and variables, as cargo's message carries them.
    paths_and_env: Vec<(String, Vec<String>, Vec<String>)>,
}

/// A build script's instructions, from the output cargo keeps beside its `out` folder (`run/stdout`
/// since cargo 1.100's layout, `output` before it): its lines that start `cargo:`, which cargo reads
/// as instructions, white space before them passed over as cargo passes it over. No such file fails,
/// so the reading never passes on output it did not see.
fn instructions(out_dir: &Path, package: &str) -> Vec<String> {
    let folder = out_dir.parent().unwrap();
    let file = [folder.join("run").join("stdout"), folder.join("output")]
        .into_iter()
        .find(|f| f.is_file())
        .unwrap_or_else(|| panic!("{package}: no output of its build script beside {}", out_dir.display()));
    std::fs::read_to_string(&file).unwrap().lines().map(str::trim).filter(|l| l.starts_with("cargo:")).map(str::to_string).collect()
}

/// The variables a build script of the three packages sets for `env!`, each reviewed: base's
/// release (`INFO("release")`). Another (`LD_PRELOAD`, which the rustc compiling the crate would
/// load) is refused.
const BUILD_ENV: [&str; 1] = ["GIT_VERSION"];

/// An instruction a build script may give, or why not: it may name what reruns it, declare a `cfg`
/// name, warn, fail, and set a variable of [`BUILD_ENV`] for `env!`. Every other one, a link
/// argument, a search path, a flag, a `cfg` set, a library linked, metadata, another variable, is
/// refused, as is a key cargo does not know (read as metadata under `cargo:`).
fn refused_instruction(line: &str) -> Option<String> {
    let rest = line.strip_prefix("cargo::").or_else(|| line.strip_prefix("cargo:"))?;
    let (key, value) = rest.split_once('=').unwrap_or((rest, ""));
    let allowed = match key {
        "rerun-if-changed" | "rerun-if-env-changed" | "rustc-check-cfg" | "warning" | "error" => true,
        "rustc-env" => BUILD_ENV.contains(&value.split_once('=').map_or(value, |(n, _)| n)),
        _ => false,
    };
    (!allowed).then(|| line.to_string())
}

fn built() -> Built {
    // One build at a time across the test processes, held while its outputs are read: base's build
    // script reruns on every build (its `rerun-if-changed=.git/HEAD` names no file from base's
    // folder), and another test's build rewrote the output this one read (found empty, the suite
    // red at f407829b).
    // The lock stands in the folder the build below builds into, as cargo resolves it
    // (`CARGO_TARGET_DIR`, a configuration's `build.target-dir`, else `target`), made if it is not
    // there yet: a build into a folder outside the checkout found no `target` to hold it (the
    // orchestrator's whole-suite run, 2026-11-26)
    let folder = PathBuf::from(metadata()["target_directory"].as_str().expect("cargo names its target directory"));
    std::fs::create_dir_all(&folder).unwrap();
    let lock = std::fs::File::create(folder.join("abaco-built.lock")).unwrap();
    lock.lock().unwrap();
    let json = cargo(&["build", "-p", "veredas-abaco", "--lib", "--message-format=json"]);
    let messages: Vec<serde_json::Value> = json.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).collect();
    let shipped = |id: &serde_json::Value| SHIPPED.iter().any(|(name, _)| id.as_str().is_some_and(|i| package_of(i) == *name));
    let mut libraries = Vec::new();
    for target in ["veredas_abaco", "ironcalc_base", "ironcalc"] {
        let rmeta: Vec<PathBuf> = messages
            .iter()
            .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == target)
            .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
            .filter_map(|f| f.as_str().map(PathBuf::from))
            .filter(|f| f.extension().is_some_and(|e| e == "rmeta"))
            .collect();
        assert_eq!(rmeta.len(), 1, "{target}: {rmeta:?}");
        let stem = rmeta[0].file_stem().unwrap().to_string_lossy().trim_start_matches("lib").to_string();
        libraries.push((target, dep_info_files(&rmeta[0].with_file_name(format!("{stem}.d")), target)));
    }
    // each build script compiled for the three packages, its dep-info beside its executable
    let mut scripts = Vec::new();
    for m in messages.iter().filter(|m| {
        m["reason"] == "compiler-artifact" && shipped(&m["package_id"]) && m["target"]["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "custom-build"))
    }) {
        let package = package_of(m["package_id"].as_str().unwrap());
        let executable = PathBuf::from(m["filenames"][0].as_str().unwrap());
        let folder = executable.parent().unwrap();
        let dep_info = std::fs::read_dir(folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "d") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("build_script_")))
            .unwrap_or_else(|| panic!("{package}: no dep-info beside its build script {}", executable.display()));
        scripts.push((package.clone(), dep_info_files(&dep_info, &package)));
    }
    let executed = messages
        .iter()
        .filter(|m| m["reason"] == "build-script-executed" && shipped(&m["package_id"]))
        .map(|m| {
            let list = |k: &str| m[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let package = package_of(m["package_id"].as_str().unwrap());
            let printed = instructions(Path::new(m["out_dir"].as_str().unwrap()), &package);
            (package, list("cfgs"), list("linked_libs"), printed)
        })
        .collect();
    let paths_and_env = messages
        .iter()
        .filter(|m| m["reason"] == "build-script-executed" && shipped(&m["package_id"]))
        .map(|m| {
            let paths = m["linked_paths"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let env = m["env"].as_array().map(|a| a.iter().filter_map(|v| v[0].as_str().map(str::to_string)).collect()).unwrap_or_default();
            (package_of(m["package_id"].as_str().unwrap()), paths, env)
        })
        .collect();
    drop(lock);
    Built { libraries, scripts, executed, paths_and_env }
}

/// Every file rustc read to compile the libraries of Ábaco and the engine, by its target's name.
fn compiled_files() -> Vec<(&'static str, BTreeSet<PathBuf>)> {
    built().libraries
}

#[test]
fn a_build_script_sets_no_cfg_links_no_library_and_compiles_its_crate_s_files_alone() {
    // a build script's `cargo::rustc-cfg` is a flag no feature shows (`feature="mock_time"` would
    // compile the fixed clock into the engine), and its `rustc-link-lib` a native library (Winsock's)
    // the graph does not show: the three packages' scripts set and link none; and each script compiles
    // files of its own crate alone, every library and binary of the three having its root under `src/`
    let built = built();
    assert!(built.scripts.iter().any(|(p, _)| p == "ironcalc_base"), "{:?}", built.scripts.iter().map(|(p, _)| p).collect::<Vec<_>>());
    assert!(built.executed.iter().any(|(p, ..)| p == "ironcalc"), "{:?}", built.executed.iter().map(|(p, ..)| p).collect::<Vec<_>>());
    for (package, cfgs, libs, printed) in &built.executed {
        assert!(cfgs.is_empty(), "{package}'s build script sets {cfgs:?}");
        assert!(libs.is_empty(), "{package}'s build script links {libs:?}");
        // and every instruction it printed is one that changes neither the compiled code nor the link
        // (`rustc-link-arg=-lws2_32` passes a library past `linked_libs`)
        assert!(!printed.is_empty(), "{package}'s build script printed no instruction");
        for line in printed {
            assert_eq!(refused_instruction(line), None, "{package}'s build script prints `{line}`");
        }
    }
    // and what cargo's own message says each set: no search path, no variable not reviewed
    for (package, paths, env) in &built.paths_and_env {
        assert!(paths.is_empty(), "{package}'s build script adds the search paths {paths:?}");
        assert!(env.iter().all(|e| BUILD_ENV.contains(&e.as_str())), "{package}'s build script sets {env:?}");
    }
    for (package, files) in &built.scripts {
        let home = root().join(SHIPPED.iter().find(|(n, _)| n == package).unwrap().1);
        for f in files {
            assert!(f.starts_with(&home), "{package}'s build script compiles {}", f.display());
        }
    }
    let full: serde_json::Value = serde_json::from_str(&cargo(&["metadata", "--format-version", "1", "--offline"])).unwrap();
    for (name, folder) in SHIPPED {
        let p = package(&full, name);
        for t in p["targets"].as_array().unwrap() {
            let kinds = strings(&t["kind"]);
            if kinds.iter().any(|k| matches!(k.as_str(), "lib" | "rlib" | "bin" | "cdylib" | "dylib" | "staticlib" | "proc-macro")) {
                let src = PathBuf::from(t["src_path"].as_str().unwrap());
                assert!(src.starts_with(root().join(folder).join("src")), "{name}: {kinds:?} rooted at {}", src.display());
            }
        }
    }
}

#[test]
fn a_build_script_s_instructions_are_read_by_their_key() {
    for line in [
        "cargo:rustc-link-arg=-lws2_32",
        "cargo::rustc-link-arg-bins=-lws2_32",
        "cargo:rustc-cdylib-link-arg=-lws2_32",
        "cargo:rustc-link-search=native=/opt/sockets",
        "cargo:rustc-flags=-l ws2_32",
        "cargo::rustc-cfg=feature=\"mock_time\"",
        "cargo:rustc-link-lib=ws2_32",
        "cargo::metadata=socket=1",
        "cargo:socket=1",
        "cargo:rustc-env=RUSTC_BOOTSTRAP=1",
        "cargo::rustc-env=CARGO_ENCODED_RUSTFLAGS=-lws2_32",
        "cargo:rustc-env=LD_PRELOAD=libplant.so",
        "cargo:rustc-env=CARGO_PKG_VERSION=9.9.9",
    ] {
        assert_eq!(refused_instruction(line), Some(line.to_string()), "{line}");
    }
    for line in [
        "cargo:rerun-if-changed=.git/HEAD",
        "cargo::rerun-if-env-changed=PATH",
        "cargo::rustc-check-cfg=cfg(fuzzing)",
        "cargo:warning=a word",
        "cargo:rustc-env=GIT_VERSION=d528905a",
        "not an instruction",
    ] {
        assert_eq!(refused_instruction(line), None, "{line}");
    }
    // cargo reads an instruction with white space before it: so does the reading
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    std::fs::write(dir.path().join("run/stdout"), " cargo:rustc-link-search=native=/opt\n\tcargo::rustc-link-arg=-lws2_32\nplain\n").unwrap();
    let read = instructions(&out, "probe");
    assert_eq!(read, ["cargo:rustc-link-search=native=/opt", "cargo::rustc-link-arg=-lws2_32"]);
    assert!(read.iter().all(|l| refused_instruction(l).is_some()));
}

#[test]
fn a_package_s_id_names_it() {
    assert_eq!(package_of("path+file:///w/forks/ironcalc/base#ironcalc_base@0.9.1"), "ironcalc_base");
    assert_eq!(package_of("path+file:///w/crates/veredas-abaco#2.0.0-alpha.1"), "veredas-abaco");
}

#[test]
fn its_sources_are_its_own_and_core_s_header_reader_alone() {
    // what rustc read to compile each library of Ábaco's own and the engine's, from its dep-info:
    // each crate's own files, and of a member's source only core's header reader, compiled into
    // Ábaco by path while core carries its network layers (C-224, docs/questions.md Q345); no other
    // member's code, no other file of core's, nothing a build script wrote
    let own: [(&str, &str, &[&str]); 3] = [
        ("veredas_abaco", "crates/veredas-abaco", &["crates/veredas-core/src/schema.rs"]),
        ("ironcalc_base", "forks/ironcalc/base", &[]),
        ("ironcalc", "forks/ironcalc/xlsx", &[]),
    ];
    let compiled = compiled_files();
    for (target, folder, allowed) in own {
        let read = &compiled.iter().find(|(t, _)| *t == target).unwrap().1;
        let home = root().join(folder);
        assert!(read.iter().any(|f| f.ends_with(format!("{folder}/src/lib.rs"))), "{target}: {read:?}");
        let outside: Vec<PathBuf> = read.iter().filter(|f| !f.starts_with(&home)).cloned().collect();
        let allowed: Vec<PathBuf> = allowed.iter().map(|a| root().join(a)).collect();
        assert_eq!(outside, allowed, "{target}: files outside its folder that its library compiles");
    }
}

#[test]
fn no_source_reaches_outside_its_crate_by_a_path_or_an_include_on_any_platform() {
    // the dep-info is one build's; a `#[path]` or an `include!` under another platform's `cfg`
    // is read from the sources: every one, compiled or not, names a file of its own crate, but
    // core's header reader in Ábaco (Q345), and no source is a link
    let mut seen_schema = 0;
    for (folder, allowed) in [
        ("crates/veredas-abaco", Some("crates/veredas-core/src/schema.rs")),
        ("forks/ironcalc/base", None),
        ("forks/ironcalc/xlsx", None),
    ] {
        let home = root().join(folder);
        let mut stack = vec![home.join("src")];
        stack.extend(build_script(&home));
        let mut files = Vec::new();
        while let Some(d) = stack.pop() {
            if d.is_dir() {
                for e in std::fs::read_dir(&d).unwrap() {
                    stack.push(e.unwrap().path());
                }
            } else if d.extension().is_some_and(|e| e == "rs") {
                files.push(d);
            }
        }
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (kind, name) in paths_and_includes(&text) {
                let name = name.unwrap_or_else(|| panic!("{}: a path or an include not written as a plain string", file.display()));
                let target = file.parent().unwrap().join(&name);
                let resolved = target.canonicalize().unwrap_or_else(|e| panic!("{}: {name}: {e}", file.display()));
                if Some(resolved.clone()) == allowed.map(|a| root().join(a)) {
                    seen_schema += 1;
                    continue;
                }
                assert!(resolved.starts_with(&home), "{}: `{name}` reaches outside {folder}", file.display());
                // a module's code is a `.rs` file under the crate's `src/`, which the socket scan
                // reads whatever the platform; data (a document's text, a table) may stand anywhere
                // in the crate
                if kind == Named::Module {
                    assert!(
                        resolved.starts_with(home.join("src")) && resolved.extension().is_some_and(|e| e == "rs"),
                        "{}: `{name}` is a module's code outside {folder}/src or no `.rs` file",
                        file.display()
                    );
                }
            }
        }
    }
    assert_eq!(seen_schema, 1, "core's header reader, compiled by path once");
    // the round at c0de423e: core's header reader names no file at all, under any `cfg` (no
    // `#[path]`, no include, no out-of-line module), so nothing beyond it is compiled into Ábaco
    // on any platform
    let schema = std::fs::read_to_string(root().join("crates/veredas-core/src/schema.rs")).unwrap();
    assert_eq!(paths_and_includes(&schema), Vec::new(), "core's header reader names a file by a path or an include");
    assert_eq!(out_of_line_modules(&without_comments(&schema)), Vec::<String>::new(), "core's header reader loads a module from a file");
    let links = git(&["ls-files", "-s", "--", "crates/veredas-abaco", "forks/ironcalc"]);
    let links: Vec<&str> = links.lines().filter(|l| l.starts_with("120000 ")).collect();
    assert!(links.is_empty(), "links among the sources: {links:?}");
}

/// The modules `code` (comments blanked) loads from a file of their own: `mod x;`, any visibility
/// or attribute before it, a raw identifier read as its name.
fn out_of_line_modules(code: &str) -> Vec<String> {
    let words = identifiers(code);
    let mut out = Vec::new();
    for pair in words.windows(2) {
        let ((first, first_end), (name, name_end)) = (&pair[0], &pair[1]);
        // what stands between `mod` and the name: white space, a raw identifier's `r#`
        let between = code[*first_end..name_end - name.len()].trim();
        if first == "mod" && (between.is_empty() || between == "r#") && code[*name_end..].trim_start().starts_with(';') {
            out.push(name.clone());
        }
    }
    out
}

/// What a source names a file for: a module's code (`#[path]`, `include!`) or data
/// (`include_str!`, `include_bytes!`, a document's text among them).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Named {
    Module,
    Data,
}

/// The end (one past) of the string literal opening at `i` (`"…"` with its escapes, `r"…"`,
/// `r#"…"#`, a `b` or a `c` before either: byte and C strings), and its text as written; `None` when
/// none opens there.
fn string_at(code: &str, i: usize) -> Option<(usize, String)> {
    let b = code.as_bytes();
    let mut j = i;
    if matches!(b.get(j), Some(b'b' | b'c')) {
        j += 1;
    }
    if b.get(j) == Some(&b'r') && matches!(b.get(j + 1), Some(b'"' | b'#')) {
        let mut k = j + 1;
        let mut hashes = 0;
        while b.get(k) == Some(&b'#') {
            hashes += 1;
            k += 1;
        }
        if b.get(k) != Some(&b'"') {
            return None;
        }
        let close = format!("\"{}", "#".repeat(hashes));
        let end = code[k + 1..].find(&close)? + k + 1;
        return Some((end + close.len(), code[k + 1..end].to_string()));
    }
    if b.get(j) != Some(&b'"') {
        return None;
    }
    let mut k = j + 1;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'"' => return Some((k + 1, code[j + 1..k].to_string())),
            _ => k += 1,
        }
    }
    None
}

/// The value of the string literal opening at `i`, as rustc and a derive (syn's `LitStr::value`)
/// read it: a raw string's text as written, any other's escapes decoded (`\x73`, `\u{73}`, `\n`, a
/// `\` before a line break and the white space after it gone; the round at 8736f277).
fn string_value_at(code: &str, i: usize) -> Option<String> {
    let (_, text) = string_at(code, i)?;
    let b = code.as_bytes();
    let j = if matches!(b.get(i), Some(b'b' | b'c')) { i + 1 } else { i };
    if b.get(j) == Some(&b'r') {
        return Some(text);
    }
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some(q @ ('\\' | '\'' | '"')) => out.push(q),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some('u') => {
                let mut hex = String::new();
                if chars.next_if_eq(&'{').is_some() {
                    for d in chars.by_ref() {
                        if d == '}' {
                            break;
                        }
                        if d != '_' {
                            hex.push(d);
                        }
                    }
                }
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            // the white space rustc and syn skip after a line continuation: spaces, tabs and line
            // breaks alone (`\u{b}` stays; the round at 27f6099e)
            Some('\n' | '\r') => {
                while chars.next_if(|c| matches!(c, ' ' | '\t' | '\n' | '\r')).is_some() {}
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// The string literal opening at `i` as rustc reads one: at every `"` (after a number too: `1"/*"`
/// is the number 1 and the string `/*`), and at a `b`, `c` or `r` prefix that starts a token: not
/// the `r` ending `attr`, nor the one of a raw identifier (`r#r"\"…"` is `r#r` and a plain string)
/// or of a lifetime (`'r"\"…"`), nor the one right after a string's closing quote, which is that
/// string's suffix (`"x"r"\"…"` is `"x"`, the suffix `r` and a plain string).
fn string_token_at(code: &str, i: usize) -> Option<(usize, String)> {
    let b = code.as_bytes();
    if b.get(i) != Some(&b'"') && i > 0 && (b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'_' | b'#' | b'\'' | b'"')) {
        return None;
    }
    string_at(code, i)
}

/// The end (one past) of the character literal opening at `i` (`'"'`, `'\''`), so no quote in one
/// is taken for a string's; a lifetime (`'a`) is none.
fn char_at(code: &str, i: usize) -> Option<usize> {
    let b = code.as_bytes();
    if b.get(i) != Some(&b'\'') {
        return None;
    }
    if b.get(i + 1) == Some(&b'\\') {
        return code.get(i + 3..)?.find('\'').map(|e| e + i + 4);
    }
    let c = code[i + 1..].chars().next()?;
    (b.get(i + 1 + c.len_utf8()) == Some(&b'\'')).then_some(i + 2 + c.len_utf8())
}

/// The source with every comment (line, block, nested block) blanked to spaces, its line breaks
/// kept, and its strings and character literals as written: what the compiler reads as tokens.
fn without_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if let Some((end, _)) = string_token_at(text, i) {
            out.push_str(&text[i..end]);
            i = end;
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(if b[i] == b'\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if let Some(end) = char_at(text, i) {
            // a character literal ('"' among them), never a string's start
            out.push_str(&text[i..end]);
            i = end;
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// The identifiers of `code` (comments blanked) outside its strings, each with where it ends.
fn identifiers(code: &str) -> Vec<(String, usize)> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some((end, _)) = string_token_at(code, i) {
            i = end;
        } else if let Some(end) = char_at(code, i) {
            i = end;
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            // a raw identifier (`r#socket`) is the identifier it names
            let start = if b[i] == b'r' && b.get(i + 1) == Some(&b'#') && b.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
                i += 2;
                i
            } else {
                i
            };
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((code[start..i].to_string(), i));
        } else {
            i += 1;
        }
    }
    out
}

/// Every file a source names by a `path` attribute (`#[path = "…"]`, `#![path = "…"]`, one inside
/// `cfg_attr`, written over any lines) or an include (`include!`, `include_str!`,
/// `include_bytes!`), read from its tokens, comments blanked; `None` for one whose argument is no
/// plain string (a `concat!`, an `env!`), which no reading can follow.
fn paths_and_includes(text: &str) -> Vec<(Named, Option<String>)> {
    let code = without_comments(text);
    let b = code.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let literal = |i: usize| string_at(&code, skip_ws(i)).map(|(_, s)| s);
    let mut out = Vec::new();
    for (start, end) in attribute_spans(&code) {
        // an attribute a macro builds from its input (`#[$m]`, `#[cfg_attr(windows, $m)]`): no
        // reading can follow it
        if code[start..end].contains('$') {
            out.push((Named::Module, None));
        }
        for (word, at) in identifiers(&code[start..end]) {
            let after = skip_ws(start + at);
            if word == "path" && b.get(after) == Some(&b'=') && b.get(after + 1) != Some(&b'=') {
                out.push((Named::Module, literal(after + 1)));
            }
        }
    }
    paths_and_includes_rest(&code, out)
}

/// Every attribute of `code` (comments blanked), `#[` or `#![` to its matching `]`, by the span of
/// what it holds; strings and character literals skipped.
fn attribute_spans(code: &str) -> Vec<(usize, usize)> {
    let b = code.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut attributes: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some((end, _)) = string_token_at(code, i) {
            i = end;
            continue;
        }
        if let Some(end) = char_at(code, i) {
            i = end;
            continue;
        }
        if b[i] == b'#' {
            let mut j = skip_ws(i + 1);
            if b.get(j) == Some(&b'!') {
                j = skip_ws(j + 1);
            }
            if b.get(j) == Some(&b'[') {
                let (start, mut k, mut depth) = (j + 1, j, 0);
                while k < b.len() {
                    if let Some((end, _)) = string_token_at(code, k) {
                        k = end;
                        continue;
                    }
                    if let Some(end) = char_at(code, k) {
                        k = end;
                        continue;
                    }
                    match b[k] {
                        b'[' => depth += 1,
                        b']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                attributes.push((start, k));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    attributes
}

/// The spans of `code`'s `use` items, `use` to its `;`.
fn use_spans(code: &str) -> Vec<(usize, usize)> {
    identifiers(code)
        .into_iter()
        .filter(|(w, _)| w == "use")
        .map(|(_, at)| (at, code[at..].find(';').map_or(code.len(), |e| at + e)))
        .collect()
}

/// The includes and the macro calls a macro's input names, after the path attributes.
fn paths_and_includes_rest(code: &str, mut out: Vec<(Named, Option<String>)>) -> Vec<(Named, Option<String>)> {
    let b = code.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let literal = |i: usize| string_at(code, skip_ws(i)).map(|(_, s)| s);
    let uses = use_spans(code);
    // a macro calling a macro its input names (`$m!(…)`): it can call `include!` or build `#[path]`
    // out of any reading's sight, so the reading fails closed
    for (i, _) in code.match_indices('$') {
        let after: String = code[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !after.is_empty() && code[i + 1 + after.len()..].trim_start().starts_with('!') {
            out.push((Named::Module, None));
        }
    }
    for (word, at) in identifiers(code) {
        let kind = match word.as_str() {
            "include" => Named::Module,
            "include_str" | "include_bytes" => Named::Data,
            _ => continue,
        };
        let bang = skip_ws(at);
        if b.get(bang) != Some(&b'!') {
            // the builtin reached by its path, not called (`use std::include as grab`, `use
            // std::{include_str as data}`): its call is out of reach under another name, so the
            // reading fails closed
            let before = code[..at - word.len()].trim_end();
            if before.ends_with("::") || uses.iter().any(|(a, e)| *a < at && at <= *e) {
                out.push((kind, None));
            }
            continue;
        }
        let open = skip_ws(bang + 1);
        out.push((kind, matches!(b.get(open), Some(b'(' | b'[' | b'{')).then(|| literal(open + 1)).flatten()));
    }
    out
}

#[test]
fn paths_and_includes_are_read_from_the_tokens_wherever_they_stand() {
    // the round at c0de423e: a module of core's header reader under another platform's `cfg`
    let probe = "#[cfg(windows)]\n#[path = \"wire_probe.rs\"]\nmod wire;";
    assert_eq!(paths_and_includes(probe).len(), 1);
    assert_eq!(out_of_line_modules(&without_comments(probe)), ["wire"]);
    assert_eq!(out_of_line_modules(&without_comments("pub(crate) mod x ;\nmod r#y;\nmod inline { fn f() {} }\nlet module = 1; // mod z;")), ["x", "y"]);
    let text = r##"
        #[cfg_attr(
            windows,
            path = "../far/a.rs"
        )]
        mod a;
        #![path="b.rs"]
        // #[path = "commented.rs"]
        /* #[path = "blocked.rs"] */
        #[doc = "path = \"not.rs\""] fn f() { let _ = '"'; }
        const T: &str = include_str! ( r#"data.txt"# );
        include!(concat!(env!("OUT_DIR"), "/x.rs"));
        #[path = "plant.txt"] pub mod plant;
        const _: &CStr = c"/*";
        #[cfg_attr(windows, diagnostic::nothing(']'), path = "../c.rs")] mod c;
        // */
        use std::include as grab;
        macro_rules! module_at { ($m:meta) => { #[$m] mod plant; } }
        macro_rules! call { ($m:ident, $f:literal) => { $m!($f); } }
        fn filter(include: bool) -> bool { include }
    "##;
    assert_eq!(
        paths_and_includes(text),
        [
            (Named::Module, Some("../far/a.rs".to_string())),
            (Named::Module, Some("b.rs".to_string())),
            (Named::Module, Some("plant.txt".to_string())),
            (Named::Module, Some("../c.rs".to_string())),
            (Named::Module, None),
            (Named::Module, None),
            (Named::Data, Some("data.txt".to_string())),
            (Named::Module, None),
            (Named::Module, None),
        ]
    );
    // a string after a number (`1"/*"`, the number 1 and a string) opens no comment that would hide
    // a path; an attribute a macro builds from its input inside `cfg_attr` fails closed
    let text = r##"
        const _: &str = stringify!(1"/*");
        #[cfg(windows)] #[path = "../../veredas-aranha/src/fusion.rs"] mod plant;
        // */
        macro_rules! module_at { ($m:meta) => { #[cfg_attr(windows, $m)] #[cfg(windows)] mod plant; } }
    "##;
    assert_eq!(paths_and_includes(text), [(Named::Module, Some("../../veredas-aranha/src/fusion.rs".to_string())), (Named::Module, None)]);
    // a raw identifier before a string hides nothing; an include named in a `use` tree, grouped or
    // not, fails closed
    let text = r##"
        const _: &str = stringify!(r#r"\" /* ");
        #[path = "../../veredas-aranha/src/fusion.rs"] mod plant;
        // */
        use std::{include as grab};
        use std::include_str as data;
    "##;
    assert_eq!(
        paths_and_includes(text),
        [(Named::Module, Some("../../veredas-aranha/src/fusion.rs".to_string())), (Named::Module, None), (Named::Data, None)]
    );
}

/// The build script cargo runs for the crate at `home`: `build.rs` unless the manifest names another
/// (or none, `build = false`).
fn build_script(home: &Path) -> Option<PathBuf> {
    let manifest: toml::Table = toml::from_str(&std::fs::read_to_string(home.join("Cargo.toml")).unwrap()).unwrap();
    match manifest.get("package").and_then(|p| p.get("build")) {
        Some(toml::Value::String(script)) => Some(home.join(script)),
        Some(toml::Value::Boolean(false)) => None,
        _ => home.join("build.rs").is_file().then(|| home.join("build.rs")),
    }
}

/// The functions an `extern` block would declare to open a socket or reach a host, POSIX's and
/// Winsock's.
const SOCKET_FUNCTIONS: &[&str] = &[
    "socket", "connect", "bind", "listen", "accept", "accept4", "getaddrinfo", "gethostbyname", "send", "sendto", "sendmsg", "recv", "recvfrom",
    "recvmsg", "closesocket", "WSAStartup", "WSASocketA", "WSASocketW", "WSAConnect", "WSAIoctl",
];

/// The functions the `extern` blocks of `code` (comments blanked) declare.
fn extern_functions(code: &str) -> Vec<String> {
    let words = identifiers(code);
    let mut out = Vec::new();
    for (i, (word, at)) in words.iter().enumerate() {
        if word != "extern" {
            continue;
        }
        // `extern "C" {`, `extern {`: the block's braces, an ABI string between; an ABI or a body a
        // macro supplies (`extern $abi {`, `extern "C" $body`) is out of any reading's sight
        let rest = &code[*at..];
        let rest = rest.trim_start();
        if rest.starts_with('$') {
            out.push("$".to_string());
            continue;
        }
        let rest = match string_at(rest, 0) {
            Some((end, _)) => rest[end..].trim_start(),
            None => rest,
        };
        if rest.starts_with('$') {
            out.push("$".to_string());
            continue;
        }
        if !rest.starts_with('{') {
            continue;
        }
        let open = code.len() - rest.len();
        // the block's closing brace, strings and character literals skipped (`#[doc = "}"]`)
        let mut depth = 0;
        let mut close = code.len();
        let mut k = open;
        while k < code.len() {
            if let Some((end, _)) = string_at(code, k) {
                k = end;
                continue;
            }
            if let Some(end) = char_at(code, k) {
                k = end;
                continue;
            }
            match code.as_bytes()[k] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = k;
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        // a block a macro fills from its input (`extern "C" { $($i)* }`) or by a call in it
        // (`foreign!(socket);`): what it declares is out of any reading's sight
        if code[open..close].contains('$') {
            out.push("$".to_string());
        }
        let inside: Vec<&(String, usize)> = words[i + 1..].iter().filter(|(_, at)| *at > open && *at <= close).filter(|(w, _)| w != "mut").collect();
        if inside.iter().any(|(_, at)| code[*at..close].trim_start().starts_with('!') && !code[*at..close].trim_start().starts_with("!=")) {
            out.push("!".to_string());
        }
        // every item it declares, a function, a static (`static socket: u8`, called through its
        // address) or a type, `mut` passed over
        for pair in inside.windows(2) {
            if matches!(pair[0].0.as_str(), "fn" | "static" | "type") {
                out.push(pair[1].0.clone());
            }
        }
    }
    out
}

#[test]
fn no_socket_in_its_sources_or_the_engine_s() {
    // no plant connection: its graph holds no network crate (`REVIEWED`), and nothing of its own or
    // the engine's that is or could be compiled opens a socket: every source under each crate's
    // `src/` whatever the platform (a module's code is there, the test above), every file the
    // libraries' dep-info lists whatever its kind, and each build script, each read by
    // `socket_route`.
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for folder in ["crates/veredas-abaco", "forks/ironcalc/base", "forks/ironcalc/xlsx"] {
        let home = root().join(folder);
        let mut stack = vec![home.join("src")];
        while let Some(d) = stack.pop() {
            if d.is_dir() {
                for e in std::fs::read_dir(&d).unwrap() {
                    stack.push(e.unwrap().path());
                }
            } else if d.extension().is_some_and(|e| e == "rs") {
                files.insert(d);
            }
        }
        files.extend(build_script(&home));
    }
    let built = built();
    let library_files: BTreeSet<PathBuf> = built.libraries.into_iter().flat_map(|(_, f)| f).collect();
    let mut script_files: BTreeSet<PathBuf> = built.scripts.into_iter().flat_map(|(_, f)| f).collect();
    for folder in ["crates/veredas-abaco", "forks/ironcalc/base", "forks/ironcalc/xlsx"] {
        script_files.extend(build_script(&root().join(folder)).and_then(|f| f.canonicalize().ok()));
    }
    files.extend(library_files.iter().cloned());
    files.extend(script_files.iter().cloned());
    assert!(files.iter().any(|f| f.ends_with("forks/ironcalc/base/build.rs")));
    let relative = |f: &Path| f.canonicalize().unwrap_or_else(|_| f.to_path_buf()).strip_prefix(root().canonicalize().unwrap()).map(|p| p.display().to_string()).unwrap_or_else(|_| f.display().to_string());
    let mut macros: Vec<(String, String)> = Vec::new();
    let mut scripts: Vec<(String, String)> = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file).unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        // a build script's source (and none a library compiles) may run `git`
        let canonical = file.canonicalize().unwrap_or_else(|_| file.clone());
        let script = script_files.contains(&canonical) && !library_files.contains(&canonical);
        if let Some(route) = socket_route_in(&text, script) {
            panic!("{}: {route}", file.display());
        }
        for m in macro_definitions(&without_comments(&text)) {
            macros.push((relative(file), sha256_hex(m.as_bytes())));
        }
        if script {
            scripts.push((relative(file), sha256_hex(&bytes)));
        }
    }
    // every macro and every build script the reviewed one
    macros.sort();
    macros.dedup();
    scripts.sort();
    scripts.dedup();
    let pinned = |list: &[(&str, &str)]| -> Vec<(String, String)> { list.iter().map(|(f, h)| (f.to_string(), h.to_string())).collect() };
    assert_eq!(macros, pinned(REVIEWED_MACROS), "a `macro_rules!` not reviewed (its file and SHA-256)");
    assert_eq!(scripts, pinned(REVIEWED_BUILD_SCRIPTS), "a build script not reviewed (its file and SHA-256)");
    // and every file compiled is the repository's: tracked and unchanged (an ignored
    // `src/target/mod.rs` a module names is none)
    for file in &files {
        let rel = relative(file);
        let tracked = Command::new("git").args(["ls-files", "--error-unmatch", "--", &rel]).current_dir(root()).output().unwrap();
        assert!(tracked.status.success(), "{rel} is compiled and not in the repository");
    }
    let mut paths: Vec<String> = ["crates/veredas-abaco/src", "forks/ironcalc/base/src", "forks/ironcalc/xlsx/src"].iter().map(|s| s.to_string()).collect();
    paths.extend(scripts.iter().map(|(f, _)| f.clone()));
    // the round at 36bd57e3: every file compiled, wherever it stands, unchanged too (core's header
    // reader, which Ábaco's library compiles by path, Q345, is outside the three folders)
    paths.extend(files.iter().map(|f| relative(f)));
    assert!(paths.iter().any(|p| p == "crates/veredas-core/src/schema.rs"), "core's header reader is among the files compiled");
    paths.sort();
    paths.dedup();
    let mut args: Vec<&str> = vec!["status", "--porcelain", "--ignored", "--untracked-files=all", "--"];
    args.extend(paths.iter().map(String::as_str));
    let changed = git(&args);
    assert!(changed.trim().is_empty(), "sources changed, added or ignored beside the repository's:\n{changed}");
}

/// The words of the standard library's sockets and Winsock, matched in a source's text whole.
const SOCKET_WORDS: [&str; 11] =
    ["std::net", "std::os::unix::net", "TcpStream", "TcpListener", "UdpSocket", "UnixStream", "UnixListener", "UnixDatagram", "Win32::Networking", "WinSock", "ws2_32"];

/// The identifiers that reach a socket, a host or a symbol by its name: the standard library's
/// `net` module and its name lookup (`ToSocketAddrs`, `lookup_host`), `libc`, and the loaders that
/// find a function by a name built at run time.
const SOCKET_IDENTIFIERS: &[&str] =
    &["net", "ToSocketAddrs", "lookup_host", "libc", "dlsym", "dlvsym", "dlopen", "dlmopen", "GetProcAddress", "LoadLibraryA", "LoadLibraryW", "LoadLibraryExA", "LoadLibraryExW", "link_ordinal"];

/// Every `macro_rules!` definition of `code` (comments blanked): its text from `macro_rules` to the
/// end of the group that holds its rules. A definition is found by its token, the identifier
/// `macro_rules` wherever it stands outside a string, whatever spaces or blanked comments come
/// before its `!` (`macro_rules ! m`, `macro_rules /**/ ! m`, which rustc reads alike); one not
/// followed by `!` is taken whole too, and fails as no reviewed text.
fn macro_definitions(code: &str) -> Vec<String> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let starts = identifiers(code).into_iter().filter(|(w, _)| w == "macro_rules").map(|(w, end)| end - w.len());
    for start in starts {
        let Some(open) = code[start..].find(['{', '(', '[']).map(|o| start + o) else { continue };
        let mut depth = 0;
        let mut k = open;
        while k < b.len() {
            if let Some((end, _)) = string_token_at(code, k) {
                k = end;
                continue;
            }
            if let Some(end) = char_at(code, k) {
                k = end;
                continue;
            }
            match b[k] {
                b'{' | b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        out.push(code[start..(k + 1).min(code.len())].to_string());
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The `macro_rules!` of the three packages' sources, each reviewed, by its file and the SHA-256 of
/// its text: a macro's transcriber can put a call, an attribute or an `extern` block together from
/// its inputs out of any reading's sight, so a macro not here, or changed, fails until it is
/// reviewed again.
const REVIEWED_MACROS: &[(&str, &str)] = &[
    // `time_part_fn`, `date_part_fn`, the date parts' `elem`
    ("forks/ironcalc/base/src/functions/date_and_time.rs", "65439b6defe783f49745a964093e999614cc1da1bc2ca1f3a6c9e8bbee62dabe"),
    ("forks/ironcalc/base/src/functions/date_and_time.rs", "783a1593b7bcb6357acf87eeb671cf5fc9340439d807595909c1a59b2c27771f"),
    ("forks/ironcalc/base/src/functions/date_and_time.rs", "988b6a49db116e4b6e513d26b247e0f1f7ce0fbc1fd7b793abac3a9a550bfe90"),
    // `single_number_fn`
    ("forks/ironcalc/base/src/functions/macros.rs", "234c2ec3b8f017246b18ede98174d187d241c6fbebe1a56144ab2048e0fd19ca"),
    // `impl_function_lookup`
    ("forks/ironcalc/base/src/functions/mod.rs", "bee14d1023e56c9b03408594a0424ea5ad520d69adcc5324094749bb3daa3b7f"),
];

/// The build scripts of the three packages, each reviewed, by the SHA-256 of its bytes: what a
/// build script runs (base's runs `git describe`) is read in it, so a changed one fails until it is
/// reviewed again.
const REVIEWED_BUILD_SCRIPTS: &[(&str, &str)] = &[
    // base's release for `INFO("release")`: `git describe` twice, `rerun-if-changed`, `rustc-env`
    ("forks/ironcalc/base/build.rs", "88c2bf9753fcf76cb217662b3a11714e5bc3d00e95d012be27b3cb12d519ed2a"),
    // xlsx's: the calc tests' functions generated from the test folders, `rerun-if-changed`
    ("forks/ironcalc/xlsx/build.rs", "39e76676fbda0d81d9a944edbd12c206bb23b765402f1e803b001839cfe41604"),
];

/// The foreign items the three packages declare, each reviewed: the engine's time zones in a
/// browser, read through the JavaScript of [`INLINE_JS_SHA256`]. An `extern` block declaring any
/// other (a socket function by any name, `rexec` as much as `socket`) is refused.
const FOREIGN_ITEMS: [&str; 3] = ["ic_tz_validate", "ic_tz_all", "ic_tz_parts"];

/// The SHA-256 of the one `inline_js` text the three packages hold, as written (the engine's time
/// zones in a browser; it opens nothing): a changed one is reviewed again.
const INLINE_JS_SHA256: &str = "638d33c970c627b0c2b44443f89593e34b06be18eb0ee493652f92c66d8d3fa0";

/// What of `js_sys` the three packages use: a JavaScript `Date`, `Math` and `Array`, each reached as
/// `js_sys::Date` and the like (no alias); none of its `eval`, `Function` or `Reflect`, which reach
/// any global (`WebSocket`, `fetch`).
const JS_SYS_ITEMS: [&str; 3] = ["Date", "Math", "Array"];

/// What reaches a browser's globals past [`JS_SYS_ITEMS`] and [`FOREIGN_ITEMS`]: a foreign item bound
/// to another JavaScript name or object (`js_name = fetch`), an object's constructor (an `Array`'s
/// is `Function`), and the casts that make a value of it callable; and a child process, which can
/// hold a connection of its own (`bash -c 'exec 3<>/dev/tcp/…'`).
const BRIDGE_IDENTIFIERS: &[&str] = &[
    "js_name", "js_namespace", "js_class", "static_method_of", "constructor", "JsCast", "unchecked_into", "dyn_into", "unchecked_ref", "dyn_ref",
    "unchecked_from_js", "unchecked_from_js_ref", "CommandExt",
];

/// How a source reaches a socket, if it does, by what it says: a socket word, an identifier of
/// [`SOCKET_IDENTIFIERS`], a function or a static named as a socket function anywhere, an `extern`
/// block declaring an item not in [`FOREIGN_ITEMS`] (or one a macro fills), any `link_name`, a native
/// library a `#[link(…)]` attribute links wherever it stands (inside `cfg_attr` too, `raw-dylib`
/// among them), and the browser's: `web_sys`, a `js_sys` item not in [`JS_SYS_ITEMS`], a JavaScript
/// module bound, an `inline_js` text not the reviewed one.
fn socket_route(text: &str) -> Option<String> {
    socket_route_in(text, false)
}

/// [`socket_route`] for a source that is a build script's (`build_script`), whose one child process
/// is base's `git` (its release for `INFO("release")`): `Command::new("git")` alone.
fn socket_route_in(text: &str, build_script: bool) -> Option<String> {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    for w in SOCKET_WORDS {
        for (at, _) in text.match_indices(w) {
            if !(ident(text[..at].chars().next_back()) || ident(text[at + w.len()..].chars().next())) {
                return Some(format!("`{w}`"));
            }
        }
    }
    if text.to_ascii_lowercase().contains("ws2_32") || text.contains("raw-dylib") {
        return Some("Winsock's library or a raw `dylib` link".into());
    }
    let code = without_comments(text);
    let words = identifiers(&code);
    if let Some((w, _)) = words.iter().find(|(w, _)| SOCKET_IDENTIFIERS.contains(&w.as_str())) {
        return Some(format!("`{w}`"));
    }
    for f in extern_functions(&code) {
        if !FOREIGN_ITEMS.contains(&f.as_str()) {
            return Some(format!("an extern block declares `{f}`, no reviewed foreign item"));
        }
    }
    for pair in words.windows(2) {
        if matches!(pair[0].0.as_str(), "fn" | "static") && SOCKET_FUNCTIONS.contains(&pair[1].0.as_str()) {
            return Some(format!("`{} {}`", pair[0].0, pair[1].0));
        }
    }
    if let Some((w, _)) = words.iter().find(|(w, _)| BRIDGE_IDENTIFIERS.contains(&w.as_str())) {
        return Some(format!("`{w}`"));
    }
    // the round at 36bd57e3: a system crate named (`windows_link::link!`, a crate's path, `extern
    // crate`), and a `link!` call, which writes an `extern` block and its library out of sight
    for (w, at) in &words {
        if SYSTEM.iter().any(|c| c.replace('-', "_") == *w && c.contains(['-', '_'])) {
            return Some(format!("`{w}`, a crate that binds the system's own interfaces"));
        }
        if w == "link" && code[*at..].trim_start().starts_with('!') {
            return Some("a `link!` call, which writes an `extern` block and the library it links".into());
        }
    }
    let uses = use_spans(&code);
    for (w, at) in &words {
        // a child process: none in a library; in a build script `Command::new("git")` alone, and its
        // `use` under its own name
        if w == "Command" {
            let in_use = uses.iter().any(|(a, b)| a < at && at <= b);
            let rest = code[*at..].trim_start();
            let git = rest.strip_prefix("::").map(str::trim_start).and_then(|r| r.strip_prefix("new")).map(str::trim_start).and_then(|r| r.strip_prefix('(')).map(str::trim_start).is_some_and(|r| r.starts_with("\"git\""));
            let renamed = rest.starts_with("as") && rest[2..].starts_with(|c: char| c.is_whitespace());
            if !build_script || !(git || in_use && !renamed) {
                return Some("a child process (`Command`), which can hold a connection of its own".into());
            }
        }
        // a foreign item linked under another name: no reviewed item is
        if w == "link_name" {
            return Some("a `link_name`, which no reviewed foreign item has".into());
        }
        if w == "web_sys" {
            return Some("`web_sys`, the browser's every interface".into());
        }
        // `js_sys` reached as `js_sys::Date` and the like alone: no alias (`use js_sys as j`), no
        // `extern crate`, no other item
        if w == "js_sys" {
            let item = code[*at..].trim_start().strip_prefix("::").map(|r| r.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>());
            if !item.as_deref().is_some_and(|i| JS_SYS_ITEMS.contains(&i)) {
                return Some(format!("`js_sys` reached otherwise than by `js_sys::{}`", JS_SYS_ITEMS.join("`, `js_sys::")));
            }
        }
    }
    // inside every attribute: a native library linked (`#[link(…)]`, `#[cfg_attr(unix, link(…))]`),
    // which the graph shows not; a JavaScript module bound or an inline text not the reviewed one
    for (start, end) in attribute_spans(&code) {
        let inner = &code[start..end];
        for (w, at) in identifiers(inner) {
            let rest = inner[at..].trim_start();
            if w == "link" && rest.starts_with('(') {
                return Some("a native library linked by `link(…)` in an attribute".into());
            }
            if matches!(w.as_str(), "module" | "raw_module") && rest.starts_with('=') {
                return Some(format!("a JavaScript module bound by `{w}`"));
            }
            // the round at c0de423e: a `key = "…"` string a derive reads as code (serde's
            // `getter`, `with`, `serialize_with`, `default` and their kin call the path they name):
            // every such string but a `doc`'s and the reviewed `inline_js` text is read as code
            if !matches!(w.as_str(), "doc" | "inline_js")
                && let Some(value) = rest.strip_prefix('=').map(str::trim_start)
                && let Some(text) = string_value_at(value, 0)
                && let Some(route) = socket_route_in(&text, build_script)
            {
                return Some(format!("an attribute's `{w}` string, which a derive reads as code: {route}"));
            }
            // the one `wasm_bindgen` attribute is the reviewed inline text's
            if w == "wasm_bindgen" {
                let names: Vec<String> = identifiers(inner).into_iter().map(|(w, _)| w).collect();
                if names != ["wasm_bindgen", "inline_js"] {
                    return Some(format!("a `wasm_bindgen` attribute not the reviewed one ({names:?})"));
                }
            }
            if w == "inline_js" {
                let text = rest.strip_prefix('=').map(str::trim_start).and_then(|r| string_at(r, 0)).map(|(_, t)| t);
                let hash = text.map(|t| {
                    use sha2::Digest;
                    sha2::Sha256::digest(t.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()
                });
                if hash.as_deref() != Some(INLINE_JS_SHA256) {
                    return Some(format!("an `inline_js` text not the reviewed one ({hash:?})"));
                }
            }
        }
    }
    None
}

#[test]
fn the_shipped_packages_rename_no_dependency_and_take_no_system_or_socket_crate() {
    // `sys = { package = "libc" }` would call `sys::socket` past every spelling: the three packages'
    // own dependencies (normal and build) carry their own names, and none is a crate that reaches
    // the system's sockets
    let full: serde_json::Value = serde_json::from_str(&cargo(&["metadata", "--format-version", "1", "--offline"])).unwrap();
    for (name, _) in SHIPPED {
        let mut direct = Vec::new();
        for d in package(&full, name)["dependencies"].as_array().unwrap() {
            if d["kind"] == "dev" {
                continue;
            }
            let dep = d["name"].as_str().unwrap();
            assert!(d["rename"].is_null(), "{name} takes `{dep}` as `{}`", d["rename"]);
            // a `-sys` crate binds the system's own library; `js-sys` is the browser's, reached by
            // `js_sys::Date`, `::Math` and `::Array` alone (`socket_route`)
            assert!(!SYSTEM.contains(&dep) && !(dep.ends_with("-sys") && dep != "js-sys"), "{name} takes `{dep}`");
            direct.push(format!("{dep} {} {}", d["kind"].as_str().unwrap_or("normal"), d["target"].as_str().unwrap_or("")).trim_end().to_string());
        }
        // the round at 36bd57e3: a reviewed crate of the graph taken as a direct dependency calls
        // its macros (`windows_link::link!` writes an `extern` block and its `link`): every direct
        // dependency, for every target, is the reviewed one, and any other fails until it is
        // reviewed
        direct.sort();
        let reviewed = DIRECT_DEPENDENCIES.iter().find(|(p, _)| *p == name).map(|(_, d)| d.to_vec()).unwrap_or_default();
        assert_eq!(direct, reviewed, "{name}'s direct dependencies (name, kind, target) are the reviewed ones");
    }
}

/// The crates that bind the system's own interfaces, sockets among them, which none of the three
/// packages takes (and whose names none of their sources holds, `socket_route`).
const SYSTEM: &[&str] = &[
    "libc", "nix", "rustix", "windows-sys", "windows", "winapi", "socket2", "mio", "tokio", "async-std", "smol",
    "windows-link", "windows-targets", "windows-core", "core-foundation-sys", "core-foundation", "r-efi",
    "android_system_properties",
];

/// Each shipped package's direct dependencies, normal and build, as cargo states them: the name, the
/// kind and the target a dependency is taken for, sorted.
const DIRECT_DEPENDENCIES: &[(&str, &[&str])] = &[
    ("veredas-abaco", &["ironcalc normal", "ironcalc_base normal", "serde normal", "serde_ignored normal", "serde_json normal", "sha2 normal", "thiserror normal", "toml normal", "zip normal"]),
    ("ironcalc", &["bitcode normal", "chrono normal", "ironcalc_base normal", "itertools normal", "quick-xml normal", "serde normal", "serde_json normal", "thiserror normal", "zip normal"]),
    (
        "ironcalc_base",
        &[
            "bitcode normal",
            "chrono normal",
            "chrono-tz normal cfg(not(all(target_arch = \"wasm32\", target_os = \"unknown\")))",
            "csv normal",
            "js-sys normal cfg(all(target_arch = \"wasm32\", target_os = \"unknown\"))",
            "rand normal cfg(not(all(target_arch = \"wasm32\", target_os = \"unknown\")))",
            "regex normal cfg(not(all(target_arch = \"wasm32\", target_os = \"unknown\")))",
            "regex-lite normal cfg(all(target_arch = \"wasm32\", target_os = \"unknown\"))",
            "ryu normal",
            "serde normal",
            "statrs normal",
            "wasm-bindgen normal cfg(all(target_arch = \"wasm32\", target_os = \"unknown\"))",
        ],
    ),
];

#[test]
fn every_route_the_critics_found_is_a_socket_route() {
    for (what, text) in [
        ("a raw identifier", "unsafe extern \"C\" { fn r#socket(d: i32, t: i32, p: i32) -> i32; }"),
        ("an escaped link_name", "extern \"C\" { #[link_name = \"\\x73ocket\"] fn open_channel() -> i32; }"),
        ("a link_name built", "extern \"C\" { #[link_name = concat!(\"sock\", \"et\")] fn open_channel() -> i32; }"),
        ("a static", "extern \"C\" { static mut connect: u8; }"),
        ("a loader", "let f = unsafe { dlsym(std::ptr::null_mut(), c\"socket\".as_ptr()) };"),
        ("a native library", "#[link(name = \"gio-2.0\")] unsafe extern \"C\" { fn g_socket_client_new(); }"),
        ("a grouped use", "use std::{net::ToSocketAddrs};"),
        ("std :: net", "let _ = std :: net :: Ipv4Addr::LOCALHOST;"),
        ("raw-dylib", "#[link(name = \"wsock32\", kind = \"raw-dylib\")] extern \"system\" {}"),
        // the round at 6e528bd8: a string after a number, which no comment opens in
        ("a string after a number", "const _: &str = stringify!(1\"/*\");\nunsafe extern \"C\" { fn socket(d: i32, t: i32, p: i32) -> i32; }\n// */"),
        ("a link inside cfg_attr", "#[cfg_attr(unix, link(name = \"resolv\"))] unsafe extern \"C\" { fn ic_tz_all(); }"),
        ("a block a macro call fills", "macro_rules! foreign { ($n:ident) => { fn $n(d: i32) -> i32; } }\nunsafe extern \"C\" { foreign!(socket); }"),
        ("a socket function by another name", "unsafe extern \"C\" { fn rexec(a: *const u8) -> i32; }"),
        ("a foreign type", "#[wasm_bindgen(js_name = WebSocket)] extern \"C\" { type Ws; }"),
        ("any link_name", "extern \"C\" { #[link_name = \"ic_tz_all\"] fn ic_tz_all(); }"),
        ("other JavaScript inline", "#[wasm_bindgen(inline_js = \"export function ic_tz_all() { new WebSocket('ws://plant'); }\")] extern \"C\" { fn ic_tz_all(); }"),
        ("a JavaScript module", "#[wasm_bindgen(module = \"/net.js\")] extern \"C\" {}"),
        ("web_sys", "use web_sys::WebSocket;"),
        ("js_sys's eval", "let _ = js_sys::eval(\"new WebSocket('ws://plant')\");"),
        ("js_sys's Reflect", "let _ = js_sys::Reflect::get(&js_sys::global(), &\"fetch\".into());"),
        // the round at c434b0fb: a raw identifier's and a lifetime's `r` open no raw string
        ("a raw identifier before a string", "const _: &str = stringify!(r#r\"\\\" /* \");\nunsafe extern \"C\" { fn socket(d: i32) -> i32; }\n// */"),
        ("a lifetime before a string", "const _: &str = stringify!('r\"\\\" /* \");\nunsafe extern \"C\" { fn socket(d: i32) -> i32; }\n// */"),
        ("an extern ABI from a macro", "macro_rules! ffi { ($abi:tt) => { unsafe extern $abi { fn rexec() -> i32; } } } ffi!(\"C\");"),
        ("an extern body from a macro", "macro_rules! ffi { ($b:tt) => { unsafe extern \"C\" $b } } ffi!({ pub fn rexec() -> i32; });"),
        ("a reviewed name bound to a global", "#[wasm_bindgen] extern \"C\" { #[wasm_bindgen(js_name = fetch)] pub fn ic_tz_all(url: &str) -> JsValue; }"),
        ("js_sys under an alias", "use js_sys as j; fn f() { let _ = j::eval(\"x\"); }"),
        ("an Array's constructor", "let f = js_sys::Array::new().constructor().constructor();"),
        ("a child process", "fn f() { let _ = std::process::Command::new(\"bash\").args([\"-c\", \"exec 3<>/dev/tcp/plant/4840\"]).status(); }"),
        // the round at 3df0bd89: an `r` after a string's closing quote is its suffix
        ("a suffix before a string", "const _: &str = stringify!(\"x\"r\"\\\" /* \");\nunsafe extern \"C\" { fn socket(d: i32) -> i32; }\n// */"),
        // the round at c0de423e: a path a derive calls, written in an attribute's string
        ("serde's getter", "#[derive(serde::Serialize)] #[serde(remote = \"String\")] struct Run { #[serde(getter = \"js_sys::eval\", serialize_with = \"nothing\")] done: Result<JsValue, JsValue> }"),
        ("serde's with", "#[derive(serde::Serialize)] struct H(#[serde(with = \"libc :: connect\")] String);"),
        ("serde's default", "#[derive(serde::Deserialize)] struct D { #[serde(default = \"plant::Command::new\")] x: u8 }"),
        // the round at 8736f277: the string's value, its escapes decoded as the derive reads it
        ("an escaped remote", "#[derive(serde::Serialize)] #[serde(remote = \"\\x73td::\\x6e\\x65\\x74::Shutdown\")] enum P { Read }"),
        ("a getter in hex", "#[serde(getter = \"\\x6a\\x73_sys::eval\")] struct R;"),
        ("a getter in a Unicode escape", "#[serde(getter = \"js_\\u{73}ys::eval\")] struct R;"),
        ("a getter across a line break", "#[serde(getter = \"js_\\\n    sys::eval\")] struct R;"),
        ("a with in hex", "#[serde(with = \"\\x6cibc\")] struct R;"),
        // the round at 27f6099e: a vertical tab after a continuation stays, so `as` and `libc` part
        ("a vertical tab after a continuation", "#[serde(with = \"<u8 as\\\n\u{b}libc::X>::f\")] struct R;"),
        // the round at 36bd57e3: a dependency's macro writing the block, a system crate's path
        ("windows_link's link!", "windows_link::link!(\"c\" \"C\" fn rexec(ahost: *mut *mut c_char, inport: i32) -> i32);"),
        ("a spaced link! call", "link ! (\"c\" \"C\" fn rcmd() -> i32);"),
        ("windows_link renamed", "use windows_link::link as l;"),
        ("core_foundation_sys", "let _ = unsafe { core_foundation_sys::stream::CFStreamCreatePairWithSocketToHost(a, h, p, r, w) };"),
    ] {
        assert!(socket_route(text).is_some(), "{what}: {text}");
    }
    // and what the engine holds is none: a raw identifier of a keyword, `include` as a word, `net`
    // in a comment (the socket words are read in comments too, the identifiers in code alone)
    assert_eq!(socket_route("fn f(r#if: bool, include: bool) -> bool { r#if && include } // the net value"), None);
    assert_eq!(socket_route("use js_sys::Date; fn r() -> f64 { js_sys::Math::random() } extern \"C\" { fn ic_tz_all() -> js_sys::Array; }"), None);
    // the attribute strings the three packages hold read as code that reaches nothing
    assert_eq!(socket_route("#[serde(skip_serializing_if = \"Option::is_none\", default = \"default_as_false\", rename_all = \"snake_case\", tag = \"type\", with = \"cell_map\")] #[cfg(all(target_arch = \"wasm32\", target_os = \"unknown\", feature = \"mock_time\"))] struct S;"), None);
    // a build script runs `git` and no other program; a library none; `std::process::id()` is no child
    let git = "use std::process::Command;\nfn main() { let _ = Command::new(\"git\").args([\"describe\"]).output(); }";
    assert_eq!(socket_route_in(git, true), None);
    assert!(socket_route_in(git, false).is_some());
    for script in ["use std::process::Command as C; fn main() { C::new(\"bash\"); }", "fn main() { std::process::Command::new(\"bash\"); }"] {
        assert!(socket_route_in(script, true).is_some(), "{script}");
    }
    assert_eq!(socket_route("let n = std::process::id();"), None);
    // a macro is found whole, so a transcriber that puts a call, an attribute or an `extern` block
    // together from its inputs is a definition the reviewed list must hold
    let glue = "macro_rules! glue { ($m:ident, $b:tt, $c:tt) => { $m $b $c; } }\nmacro_rules! item { ($h:tt, $a:tt, $i:item) => { $h $a $i }; }";
    let found = macro_definitions(glue);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].starts_with("macro_rules! glue") && found[0].ends_with("$m $b $c; } }"), "{found:?}");
    assert!(found.iter().all(|m| !REVIEWED_MACROS.iter().any(|(_, h)| *h == sha256_hex(m.as_bytes()))));
    // the round at 36bd57e3: a definition spaced or with a comment before its `!` is one, and a
    // `macro_rules!` in a string is none
    for spaced in ["macro_rules ! glue { ($a:tt, $b:tt, $c:tt) => { $a $b $c } }", "macro_rules /**/ ! glue { ($a:tt) => { $a } }", "macro_rules\n!\tglue { () => {} }"] {
        let found = macro_definitions(&without_comments(spaced));
        assert_eq!(found.len(), 1, "{spaced}: {found:?}");
        assert!(found[0].starts_with("macro_rules") && found[0].ends_with('}'), "{found:?}");
    }
    assert_eq!(macro_definitions("const _: &str = \"macro_rules! m { () => {} }\";"), Vec::<String>::new());
    assert!(socket_route("// see std::net").is_some());
}

#[test]
fn a_socket_by_any_of_those_routes_is_seen() {
    let code = without_comments(
        r#"
        unsafe extern "C" { fn socket(domain: i32, kind: i32, protocol: i32) -> i32; fn getpid() -> i32; }
        extern { pub fn WSAStartup(v: u16, d: *mut u8) -> i32; }
        #[wasm_bindgen] extern "C" { pub fn ic_tz_validate(name: &str) -> bool; }
        // extern "C" { fn connect(); }
        use libc::{connect};
        extern "C" { #[doc = "}"] fn accept(fd: i32) -> i32; }
        macro_rules! ffi { ($($i:tt)*) => { unsafe extern "C" { $($i)* } } }
        "#,
    );
    assert_eq!(extern_functions(&code), ["socket", "getpid", "WSAStartup", "ic_tz_validate", "accept", "$"]);
    assert!(identifiers(&code).iter().any(|(w, _)| w == "libc"));
}

/// The edges (parent to child, each `name version`) of the subtrees under the engine's two crates in
/// a `cargo tree --prefix depth --no-dedupe` listing, and every crate in them but the two.
fn engine_graph(tree: &str) -> (BTreeSet<(String, String)>, BTreeSet<(String, String)>) {
    let node = |l: &str| -> (usize, String, String) {
        let depth = l.chars().take_while(char::is_ascii_digit).collect::<String>();
        let mut w = l[depth.len()..].split_whitespace();
        (depth.parse().unwrap(), w.next().unwrap_or("").to_string(), w.next().unwrap_or("").trim_start_matches('v').to_string())
    };
    let mut edges = BTreeSet::new();
    let mut crates = BTreeSet::new();
    // the open ancestors of the line read, by depth, and the depth of the engine crate whose
    // subtree is being read
    let mut ancestors: Vec<(String, String)> = Vec::new();
    let mut root: Option<usize> = None;
    for line in tree.lines().filter(|l| !l.trim().is_empty()) {
        let (depth, name, version) = node(line);
        ancestors.truncate(depth);
        if root.is_some_and(|r| depth <= r) {
            root = None;
        }
        if root.is_none() && (name == "ironcalc" || name == "ironcalc_base") {
            root = Some(depth);
        } else if root.is_some() {
            if let Some(parent) = ancestors.last() {
                edges.insert((format!("{} {}", parent.0, parent.1), format!("{name} {version}")));
            }
            if name != "ironcalc" && name != "ironcalc_base" {
                crates.insert((name.clone(), version.clone()));
            }
        }
        if root == Some(depth) && depth > 0 && let Some(parent) = ancestors.last().filter(|p| p.0 == "ironcalc" || p.0 == "ironcalc_base") {
            edges.insert((format!("{} {}", parent.0, parent.1), format!("{name} {version}")));
        }
        ancestors.push((name, version));
    }
    (edges, crates)
}

#[test]
fn the_fork_s_own_suite_passes_on_the_versions_abaco_builds() {
    let paths = [
        format!("veredas-abaco v2.0.0-alpha.1 ({})", root().join("crates/veredas-abaco").display()),
        format!("ironcalc_base v0.9.1 ({})", root().join("forks/ironcalc/base").display()),
        format!("ironcalc v0.9.1 ({})", root().join("forks/ironcalc/xlsx").display()),
    ];
    // the fork's lock pins every crate of the engine's graph (its two crates' own, in the suite's
    // build, on every platform) at the version and the source the suite's lock gives it, so the
    // fork's suite tests the versions Ábaco links; a crate of Ábaco's graph outside the engine's is
    // Ábaco's own, and listed so (`ABACO_ONLY`)
    let fork_lock = locked(&root().join("forks/ironcalc/Cargo.lock"));
    let root_lock = locked(&root().join("Cargo.lock"));
    // the engine's graph, build dependencies included, as Ábaco builds it (the subtrees under its
    // two crates in Ábaco's own tree) and as the fork's workspace builds it for the fork's suite: the
    // same edges, parent's version to child's, so the suite tests the very versions Ábaco links
    let abaco_tree = cargo(&["tree", "-p", "veredas-abaco", "-e", "normal,build", "--target", "all", "--no-dedupe", "--prefix", "depth", "--format", "{p}"]);
    let fork_tree = {
        let out = Command::new(env!("CARGO"))
            .args(["tree", "-p", "ironcalc_base", "-p", "ironcalc", "-e", "normal,build", "--target", "all", "--no-dedupe", "--prefix", "depth", "--format", "{p}", "--locked", "--manifest-path"])
            .arg(root().join("forks/ironcalc/Cargo.toml"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let (abaco_edges, engine) = engine_graph(&abaco_tree);
    let (fork_edges, _) = engine_graph(&fork_tree);
    // the engine's build dependencies are among them (num-traits' `autocfg`)
    assert!(abaco_edges.len() > 50 && abaco_edges.iter().any(|(_, c)| c.starts_with("autocfg ")), "{abaco_edges:?}");
    let only_abaco: Vec<_> = abaco_edges.difference(&fork_edges).collect();
    let only_fork: Vec<_> = fork_edges.difference(&abaco_edges).collect();
    assert!(only_abaco.is_empty() && only_fork.is_empty(), "the engine's edges in Ábaco's build and not the fork's: {only_abaco:?}; in the fork's and not Ábaco's: {only_fork:?}");
    assert!(engine.iter().any(|(n, _)| n == "quick-xml") && engine.iter().any(|(n, _)| n == "zip"), "{engine:?}");
    let listed = |name: &str, version: &str| {
        ABACO_ONLY.iter().any(|o| match o.split_once('@') {
            Some((n, prefix)) => n == name && version.starts_with(prefix),
            None => *o == name,
        })
    };
    for p in normal_graph(true) {
        if paths.contains(&p) {
            continue;
        }
        assert!(!p.contains('('), "{p}: not from crates.io");
        let mut words = p.split_whitespace();
        let (name, version) = (words.next().unwrap(), words.next().unwrap().trim_start_matches('v'));
        assert!(root_lock.get(name).is_some_and(|v| v.contains(version)), "{name} {version} not in the suite's lock from crates.io");
        if engine.contains(&(name.to_string(), version.to_string())) {
            assert!(!listed(name, version), "{name} {version}: listed as Ábaco's own, yet in the engine's graph");
            assert!(
                fork_lock.get(name).is_some_and(|v| v.contains(version)),
                "{name} {version} in the engine's graph, {:?} in the fork's lock",
                fork_lock.get(name)
            );
        } else {
            // Ábaco's own, which the engine never uses: the fork's suite has nothing of them to test
            assert!(listed(name, version), "{name} {version}: in Ábaco's graph and not the engine's, yet not listed as Ábaco's own");
        }
    }
    // each entry names a crate Ábaco has and the engine has not, at any version a bare name says
    let abaco = normal_graph(true);
    for o in ABACO_ONLY {
        let (n, prefix) = o.split_once('@').unwrap_or((o, ""));
        assert!(
            abaco.iter().any(|p| p.split_whitespace().next() == Some(n) && p.split_whitespace().nth(1).is_some_and(|v| v.trim_start_matches('v').starts_with(prefix))),
            "{o}: listed, not in Ábaco's graph"
        );
        assert!(!engine.iter().any(|(en, ev)| en == n && ev.starts_with(prefix)), "{o}: listed as Ábaco's own, yet in the engine's graph");
    }
    // one float path: `num-traits` (the statistical functions' float math) with `std` in Ábaco's
    // own build and in the fork's test build (fork change 13), so neither falls back to `libm`
    for (what, args) in [
        ("Ábaco's build", vec!["tree", "-e", "normal", "--prefix", "none", "-f", "{p} {f}", "-p", "veredas-abaco"]),
        (
            "the fork's test build",
            vec!["tree", "-e", "normal,dev", "--prefix", "none", "-f", "{p} {f}", "-p", "ironcalc_base", "--manifest-path"],
        ),
    ] {
        let text = if what == "Ábaco's build" {
            cargo(&args)
        } else {
            let out = Command::new(env!("CARGO"))
                .args(&args)
                .arg(root().join("forks/ironcalc/Cargo.toml"))
                .arg("--locked")
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap()
        };
        let line = text.lines().find(|l| l.starts_with("num-traits ")).unwrap_or_else(|| panic!("{what}: no num-traits"));
        let features = line.split(' ').nth(2).unwrap_or("");
        assert!(features.split(',').any(|f| f == "std"), "{what}: {line}");
    }
    // upstream's tests and the changes', in the fork's own workspace, its lock as committed, into a
    // target folder of its own beside the suite's
    let target = root().join("target/ironcalc");
    let out = Command::new(env!("CARGO"))
        .args(["test", "--locked", "--manifest-path"])
        .arg(root().join("forks/ironcalc/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the fork's suite failed:\n{stdout}\n{stderr}");
    let (mut passed, mut failed, mut ignored, mut filtered) = (0usize, 0usize, 0usize, 0usize);
    for line in stdout.lines().filter(|l| l.starts_with("test result:")) {
        let words: Vec<&str> = line.split_whitespace().collect();
        passed += words[3].parse::<usize>().unwrap();
        failed += words[5].parse::<usize>().unwrap();
        ignored += words[7].parse::<usize>().unwrap();
        filtered += words[11].parse::<usize>().unwrap();
    }
    eprintln!("the fork's suite: {passed} passed, {failed} failed, {ignored} ignored, {filtered} filtered out");
    assert_eq!((passed, failed, ignored, filtered), (FORK_PASSED, 0, FORK_IGNORED, 0));
    // and the ignored ones are upstream's, by name
    let ignored_names: BTreeSet<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("test ") && l.contains(" ... ignored"))
        .map(|l| l.trim_start_matches("test ").split(" ... ").next().unwrap())
        .collect();
    assert_eq!(ignored_names, UPSTREAM_IGNORED.iter().copied().collect());
    // the changes' own test files ran
    for file in ["tests/column_outline_roundtrip.rs", "tests/missing_styles_part_import.rs", "tests/excel_function_spelling.rs", "tests/package_relationships.rs"] {
        assert!(stderr.contains(file), "{file} did not run:\n{stderr}");
    }
    // and the engine's build scripts, run for the fork's own test build (its features its own:
    // `mock_time` among them), gave cargo no instruction Ábaco's build refuses: what cargo kept of
    // every run under this target folder, read as `built` reads Ábaco's
    let mut outputs = Vec::new();
    let mut stack = vec![target.join("debug").join("build")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                // the engine's two packages' folders (`ironcalc_base`, `ironcalc-<hash>`), and below them
                if d != target.join("debug").join("build") || name.starts_with("ironcalc") {
                    stack.push(path);
                }
            } else if name == "output" || (name == "stdout" && path.parent().is_some_and(|p| p.ends_with("run"))) {
                outputs.push(path);
            }
        }
    }
    assert!(!outputs.is_empty(), "no kept output of the engine's build scripts under {}", target.display());
    for output in &outputs {
        for line in std::fs::read_to_string(output).unwrap().lines().map(str::trim).filter(|l| l.starts_with("cargo:")) {
            assert_eq!(refused_instruction(line), None, "{}: the fork's test build's script prints `{line}`", output.display());
        }
    }
}

/// Where the record's one `Local changes` heading line ends (its line's end, the newline excluded),
/// every heading found as GFM reads one, by the blocks [`change_rows`] reads: an ATX heading (up to
/// three columns of spaces and tabs, one to six `#`, a space or a tab, its closing `#`s passed
/// over) or a setext one (the paragraph its `===` or `---` underlines), in a quote or a list item
/// too, none in a fence, an HTML block (a comment among them) or indented code; its text read as
/// GitHub shows it, entities decoded and inline markup dropped ([`heading_words_with`]). The record
/// holds exactly one heading that reads `Local changes`, an ATX heading of level 2 outside every
/// container, spelled plainly: `##  Local changes` with two spaces is that heading; a second, or one
/// that reads so through markup (`## Local&#32;changes`, `## *Local changes*`), is refused, so no
/// decoy heading is read as the section or hides one (the rounds at 36bd57e3 and c0de423e).
fn local_changes_heading(record: &str) -> Result<usize, String> {
    let lines: Vec<(usize, &str)> = record
        .split_inclusive('\n')
        .scan(0, |at, l| {
            let start = *at;
            *at += l.len();
            Some((start, l.trim_end_matches(['\n', '\r'])))
        })
        .collect();
    // the lines a list item holds, at its content column or beyond (the round at 8736f277)
    let texts: Vec<&str> = lines.iter().map(|(_, l)| *l).collect();
    let items = list_items(&texts);
    // each heading that reads `Local changes`: whether it is the section's own, and its line's end
    let mut found: Vec<(bool, usize)> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let underline = |l: &str| {
        let u = l.trim_matches([' ', '\t']);
        column_of(l) <= 3 && !u.is_empty() && (u.chars().all(|c| c == '=') || u.chars().all(|c| c == '-'))
    };
    let mut k = 0;
    while k < lines.len() {
        let (start, whole) = lines[k];
        let end = start + whole.len();
        // a line a list item holds is read from the item's content column, inside a container
        let (raw, in_item) = match items[k] {
            Some(column) => (after_columns(whole, column), true),
            None => (whole, false),
        };
        // a setext heading's underline, the paragraph above it its text
        if !paragraph.is_empty() && (underline(raw) || underline(without_containers(raw).0)) {
            if reads_as_local_changes(&paragraph.join(" ")) {
                found.push((false, end));
            }
            paragraph.clear();
            k += 1;
            continue;
        }
        match block_start(raw) {
            Some(Block::Blank) => {
                paragraph.clear();
                k += 1;
                continue;
            }
            // a fence or an HTML block holds no heading, to its end
            Some(Block::Fence(ch, n)) => {
                k += 1;
                while k < lines.len() && !closes_fence(lines[k].1, ch, n) {
                    k += 1;
                }
                k += 1;
                paragraph.clear();
                continue;
            }
            Some(Block::Html(kind)) if kind != 7 || paragraph.is_empty() => {
                while k < lines.len() && !html_block_ends(kind, lines[k].1) {
                    k += 1;
                }
                k += 1;
                paragraph.clear();
                continue;
            }
            // indented code, but for a paragraph's continuation
            Some(Block::IndentedCode) if paragraph.is_empty() => {
                k += 1;
                continue;
            }
            _ => {}
        }
        let (inner, contained) = without_containers(raw);
        let contained = contained || in_item;
        let t = inner.trim_start_matches([' ', '\t']);
        let hashes = t.chars().take_while(|c| *c == '#').count();
        let rest = &t[hashes..];
        if column_of(inner) <= 3 && (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
            // a closing sequence of `#`s, after a space or a tab, is none of the text
            let text = rest.trim_matches([' ', '\t']);
            let closed = text.trim_end_matches('#');
            let text = if closed.is_empty() || closed.ends_with([' ', '\t']) { closed.trim_matches([' ', '\t']) } else { text };
            if reads_as_local_changes(text) {
                found.push((!contained && hashes == 2 && text == "Local changes", end));
            }
            paragraph.clear();
        } else if matches!(block_start(inner), Some(Block::ThematicBreak) | Some(Block::Blank)) {
            paragraph.clear();
        } else {
            paragraph.push(t.to_string());
        }
        k += 1;
    }
    match found.as_slice() {
        [(true, end)] => Ok(*end),
        [] => Err("the record has no `Local changes` heading".into()),
        [_] => Err("the record's `Local changes` heading is not a plain ATX heading of level 2 outside every container".into()),
        _ => Err(format!("the record has {} `Local changes` headings, one the section", found.len())),
    }
}

/// A line without the markers of the quotes and list items that hold it (`> `, `- `, `1. `), and
/// whether it had any.
fn without_containers(line: &str) -> (&str, bool) {
    let mut l = line;
    let mut contained = false;
    loop {
        if column_of(l) > 3 {
            return (l, contained);
        }
        let t = l.trim_start_matches([' ', '\t']);
        if let Some(rest) = t.strip_prefix('>') {
            l = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
            contained = true;
            continue;
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        let marker = if t.starts_with(['-', '+', '*']) {
            1
        } else if (1..=9).contains(&digits) && t[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            0
        };
        if marker > 0 && (t.len() == marker || t[marker..].starts_with([' ', '\t'])) {
            l = t[marker..].trim_start_matches([' ', '\t']);
            contained = true;
            continue;
        }
        return (l, contained);
    }
}

/// Whether a heading's text reads `Local changes` as GitHub shows it ([`heading_words_with`]): an entity
/// the reading does not decode read both as nothing and as a space, so one that shows as white
/// space or as nothing (`&Tab;`, `&lrm;`) is caught whatever its name (the round at 8736f277).
fn reads_as_local_changes(text: &str) -> bool {
    [None, Some(""), Some(" ")].iter().any(|unknown| heading_words_with(text, *unknown) == "local changes")
}

/// A heading's text as GitHub shows it, for comparing: HTML comments and tags dropped, a link's
/// target (balanced parentheses) or its reference label dropped and its brackets, backslash escapes
/// and entities read (numeric ones, and the named ones that show as white space or nothing or as
/// `&`, `<`, `>` and quotes), emphasis and code markers dropped, every format character (Unicode's
/// `Cf`: `&lrm;`, U+200B to U+200F, U+2060 to U+2064, U+180E, U+061C and their kin) dropped, white
/// space of every kind one space, in lower case; an entity it does not decode kept as written
/// (`unknown` `None`) or read as `unknown`.
fn heading_words_with(text: &str, unknown: Option<&str>) -> String {
    let mut s = text.to_string();
    // comments, then tags
    while let Some(a) = s.find("<!--") {
        let b = s[a..].find("-->").map(|b| a + b + 3).unwrap_or(s.len());
        s.replace_range(a..b, "");
    }
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // a link's target after its text (its parentheses balanced), or its reference label
    let chars: Vec<char> = out.chars().collect();
    let mut s = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ']' && matches!(chars.get(i + 1), Some('(' | '[')) {
            let (open, close) = if chars[i + 1] == '(' { ('(', ')') } else { ('[', ']') };
            let mut depth = 0;
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == open {
                    depth += 1;
                } else if chars[j] == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        s.push(chars[i]);
        i += 1;
    }
    let mut words = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            words.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '&' {
            let end = chars[i..].iter().take(34).position(|c| *c == ';');
            if let Some(end) = end {
                let name: String = chars[i + 1..i + end].iter().collect();
                let decoded: Option<String> = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from)
                } else if let Some(dec) = name.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32).map(String::from)
                } else {
                    match name.as_str() {
                        "nbsp" | "NonBreakingSpace" | "ensp" | "emsp" | "emsp13" | "emsp14" | "thinsp" | "ThinSpace" | "numsp" | "puncsp" | "hairsp"
                        | "VeryThinSpace" | "MediumSpace" | "ThickSpace" | "NoBreak" | "Tab" | "NewLine" | "Space" => Some(" ".into()),
                        "amp" | "AMP" => Some("&".into()),
                        "lt" | "LT" => Some("<".into()),
                        "gt" | "GT" => Some(">".into()),
                        "quot" | "QUOT" => Some("\"".into()),
                        "apos" => Some("'".into()),
                        "shy" | "zwnj" | "zwj" | "lrm" | "rlm" | "ZeroWidthSpace" | "NegativeVeryThinSpace" | "NegativeThinSpace" | "NegativeMediumSpace"
                        | "NegativeThickSpace" | "InvisibleTimes" | "it" | "InvisibleComma" | "ic" | "ApplyFunction" | "af" => Some(String::new()),
                        _ if name.chars().all(|c| c.is_ascii_alphanumeric()) && !name.is_empty() => unknown.map(String::from),
                        _ => None,
                    }
                };
                if let Some(d) = decoded {
                    words.push_str(&d);
                    i += end + 1;
                    continue;
                }
            }
        }
        if !matches!(c, '*' | '_' | '`' | '~' | '[' | ']') {
            words.push(c);
        }
        i += 1;
    }
    let words: String = words.chars().filter(|c| !is_format_char(*c)).collect();
    words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Whether `c` is a format character (Unicode's general category `Cf`), which GitHub shows as
/// nothing.
fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// A line's text after the markers of every quote and list item that holds it, at any depth (a
/// nested item's marker past the fourth column too), and the spaces before it.
fn after_every_container(line: &str) -> &str {
    let mut t = line.trim_start_matches(' ');
    loop {
        if let Some(rest) = t.strip_prefix('>') {
            t = rest.trim_start_matches(' ');
            continue;
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        let marker = if t.starts_with(['-', '+', '*']) {
            1
        } else if (1..=9).contains(&digits) && t[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            0
        };
        if marker > 0 && (t.len() == marker || t[marker..].starts_with(' ')) {
            t = t[marker..].trim_start_matches(' ');
            continue;
        }
        return t;
    }
}

/// The header and the delimiter of the record's one table, as written.
const RECORD_HEADER: &str = "| # | Change | Where | Upstream |";
const RECORD_DELIMITER: &str = "|---|---|---|---|";

/// The record's rows, each by its first cell, read from the one form the record is held to, which
/// GitHub's Markdown reads one way only (the round at 27f6099e: the rounds before read the section
/// as GFM 0.29 reads any Markdown, and each found a corner the reading took otherwise):
///
/// - the line `## Local changes`, the one heading that reads so ([`local_changes_heading`]);
/// - after it, lines of prose at the margin, opening no block and holding no `|`, and blank lines;
/// - after a blank line, the one table: its header and its delimiter as written above, and every
///   row a line of four cells opening and closing with a pipe;
/// - the table running to the end of the record, but for blank lines.
///
/// The record holds no carriage return and no `<` (so no HTML, which GitHub reads however it pairs
/// a backtick), and no other line, nor two lines joined, reads `local changes` in any spelling (its
/// ASCII letters, [`heading_words_with`]'s readings). Anything else fails the reading, naming its
/// line.
fn change_rows(record: &str) -> Result<Vec<String>, String> {
    if record.contains('\r') {
        return Err("the record holds a carriage return, which ends a line to GitHub".into());
    }
    let lines: Vec<&str> = record.split('\n').collect();
    // the whole record, before the section too, in what GitHub reads one way only (the round at
    // 7a0bedec): no white space but the space (a no-break space or a tab GitHub's lexer and the
    // reading's take apart: a fence closed to one and not the other, a setext paragraph split); no
    // run of three backticks or tildes (a fence, in an item at any depth; the round at d7cb0b35);
    // no `<` at all (the round at b6dc3602): raw HTML in a cell ends or adds a row and before the
    // heading holds it, and which `<` GitHub reads as HTML turns on how it pairs a backtick, which
    // an extended autolink, a link's target and an escaped `)` each take before a code span does,
    // so the record names an XML element without its brackets (`externalReferences`); and no line
    // opening with `[` after its containers' markers at any depth (a link reference or a footnote's
    // definition, its label over two lines too)
    for (k, l) in lines.iter().enumerate() {
        if let Some(c) = l.chars().find(|c| c.is_whitespace() && *c != ' ') {
            return Err(format!("line {} of the record holds white space other than a space ({c:?})", k + 1));
        }
        if l.contains("```") || l.contains("~~~") {
            return Err(format!("line {} of the record holds three backticks or tildes, which may open a fence", k + 1));
        }
        if l.contains('<') {
            return Err(format!("line {} of the record holds a `<`, which GitHub may read as HTML", k + 1));
        }
        if after_every_container(l).starts_with('[') {
            return Err(format!("line {} of the record opens with `[`, which may open a link reference or a footnote's definition", k + 1));
        }
    }
    let heading = lines.iter().position(|l| *l == "## Local changes").ok_or("the record has no line `## Local changes`")?;
    let at = local_changes_heading(record)?;
    let heading_end: usize = lines[..=heading].iter().map(|l| l.len() + 1).sum::<usize>() - 1;
    if at != heading_end {
        return Err("the record's `Local changes` heading is not its line `## Local changes`".into());
    }
    // nothing before the heading holds it: it follows a blank line, which ends every paragraph (and
    // the record holds no `<`, so no HTML block, which may run past a blank line, `<textarea>`, or
    // start on a lazy line; the round at 0394f0ba)
    if heading == 0 || !lines[heading - 1].is_empty() {
        return Err(format!("line {} of the record, before `## Local changes`, is not blank", heading));
    }
    // no other line, nor two lines joined, reads `local changes`, its ASCII letters read
    let letters = |l: &str| -> Vec<String> {
        [None, Some(""), Some(" ")].iter().map(|u| heading_words_with(l, *u).chars().filter(char::is_ascii_alphabetic).collect()).collect()
    };
    for k in (0..lines.len()).filter(|k| *k != heading) {
        let here = letters(lines[k]);
        if here.iter().any(|w| w.contains("localchanges")) {
            return Err(format!("line {} of the record reads `local changes`", k + 1));
        }
        if k + 1 < lines.len() && k + 1 != heading {
            let next = letters(lines[k + 1]);
            if here.iter().any(|a| next.iter().any(|b| format!("{a}{b}").contains("localchanges"))) {
                return Err(format!("lines {} and {} of the record read `local changes` joined", k + 1, k + 2));
            }
        }
    }
    let leaves = |k: usize, why: &str| format!("line {} of the record leaves its one form: {why}", k + 1);
    // the section's prose and blank lines, to the table
    let mut k = heading + 1;
    while k < lines.len() && lines[k] != RECORD_HEADER {
        let l = lines[k];
        // a line of prose holds a letter or a digit: one of `-`, `:`, `=` alone is a setext
        // underline or a pipeless table's delimiter to GFM (`:-`, `===\t`; the round at 0394f0ba)
        let underline = l.trim_matches(['=', '-', ' ', '\t']).is_empty();
        let words = l.chars().any(char::is_alphanumeric);
        if !l.is_empty() && (column_of(l) > 0 || block_start(l).is_some() || l.contains('|') || underline || !words) {
            return Err(leaves(k, "prose stands at the margin, opens no block, holds a letter or a digit and no `|`"));
        }
        k += 1;
    }
    if k == lines.len() {
        return Err(format!("the record has no table `{RECORD_HEADER}`"));
    }
    if !lines[k - 1].is_empty() {
        return Err(leaves(k, "the table follows a blank line"));
    }
    if lines.get(k + 1) != Some(&RECORD_DELIMITER) {
        return Err(leaves(k + 1, &format!("the table's delimiter is `{RECORD_DELIMITER}`")));
    }
    let mut rows = Vec::new();
    k += 2;
    while k < lines.len() && !lines[k].is_empty() {
        let l = lines[k];
        let cells = gfm_cells(l);
        if !(l.starts_with("| ") && l.ends_with(" |") && cells.len() == 4) {
            return Err(leaves(k, "a row is a line of four cells opening and closing with a pipe"));
        }
        rows.push(cells[0].clone());
        k += 1;
    }
    if let Some(j) = (k..lines.len()).find(|j| !lines[*j].is_empty()) {
        return Err(leaves(j, "the table runs to the end of the record"));
    }
    Ok(rows)
}

/// For each line, the content column of the innermost list item holding it as GFM nests them,
/// `None` outside every item (the round at 8736f277): a marker (`-`, `+`, `*`, `1.`, `1)`, then a
/// space or a tab) opens an item whose content starts after the marker and the white space after it
/// (one column past the marker when that is more than four or the line ends there); a later line at
/// that column or beyond is the item's, a blank line too, and a line left of it closes the item. A
/// paragraph's lazy continuation left of the column is the paragraph's, which opens no table and
/// is read as `change_rows` reads one: `None` here.
fn list_items(lines: &[&str]) -> Vec<Option<usize>> {
    let mut open: Vec<usize> = Vec::new();
    let mut paragraph = false;
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        if l.trim_matches([' ', '\t']).is_empty() {
            paragraph = false;
            out.push(open.last().copied());
            continue;
        }
        let col = column_of(l);
        let inside = open.last().is_some_and(|c| col >= *c);
        let lazy = !inside && paragraph && matches!(block_start(l), None | Some(Block::IndentedCode));
        if lazy {
            out.push(None);
            continue;
        }
        while open.last().is_some_and(|c| col < *c) {
            open.pop();
        }
        let base = open.last().copied().unwrap_or(0);
        let inner = after_columns(l, base);
        match block_start(inner) {
            Some(Block::ListItem) => {
                let lead = inner.len() - inner.trim_start_matches([' ', '\t']).len();
                let t = &inner[lead..];
                let marker = if t.starts_with(['-', '+', '*']) { 1 } else { t.chars().take_while(char::is_ascii_digit).count() + 1 };
                let rest = &t[marker..];
                let marker_end = base + columns_to(inner, lead + marker);
                let text_at = base + columns_to(inner, lead + marker + (rest.len() - rest.trim_start_matches([' ', '\t']).len()));
                let blank = rest.trim_matches([' ', '\t']).is_empty();
                open.push(if blank || text_at - marker_end > 4 { marker_end + 1 } else { text_at });
                paragraph = !blank;
            }
            None => paragraph = true,
            Some(Block::IndentedCode) if paragraph => {}
            _ => paragraph = false,
        }
        out.push(open.last().copied());
    }
    out
}

/// A line with its first `n` columns of white space passed over (its text when it has fewer).
fn after_columns(line: &str, n: usize) -> &str {
    let mut col = 0;
    for (i, ch) in line.char_indices() {
        if col >= n {
            return &line[i..];
        }
        match ch {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => return &line[i..],
        }
    }
    ""
}

/// The columns `line[..end]` spans, a tab advancing to the next multiple of four.
fn columns_to(line: &str, end: usize) -> usize {
    let mut col = 0;
    for ch in line[..end].chars() {
        col += if ch == '\t' { 4 - col % 4 } else { 1 };
    }
    col
}

/// The column a line's text starts at, a tab advancing to the next multiple of four.
fn column_of(line: &str) -> usize {
    let mut col = 0;
    for ch in line.chars() {
        match ch {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => break,
        }
    }
    col
}

/// A table row's cells as GFM splits them: one leading and one trailing `|` passed over, a split at
/// each other `|` no backslash escapes, each cell trimmed.
fn gfm_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = match t.strip_suffix('|') {
        Some(rest) if !rest.ends_with('\\') => rest,
        _ => t,
    };
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for ch in t.chars() {
        if ch == '|' && !escaped {
            cells.push(String::new());
        } else {
            cells.last_mut().unwrap().push(ch);
        }
        // a pipe after a backslash is the cell's whatever stands before the backslash, as
        // cmark-gfm's cell scanner reads it (`\\|` splits nothing; the round at 0394f0ba)
        escaped = ch == '\\';
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// The blocks a line can open that end a table (GFM 0.29 and GitHub's footnotes).
#[derive(Debug, PartialEq, Clone, Copy)]
enum Block {
    Blank,
    IndentedCode,
    Quote,
    Heading,
    Fence(char, usize),
    Html(u8),
    ThematicBreak,
    ListItem,
    Footnote,
}

/// The block a line opens, if any: what ends a table's rows.
fn block_start(line: &str) -> Option<Block> {
    if line.trim().is_empty() {
        return Some(Block::Blank);
    }
    if column_of(line) >= 4 {
        return Some(Block::IndentedCode);
    }
    let t = line.trim_start_matches([' ', '\t']);
    let space_or_end = |rest: &str| rest.is_empty() || rest.starts_with([' ', '\t']);
    if t.starts_with('>') {
        return Some(Block::Quote);
    }
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && space_or_end(&t[hashes..]) {
        return Some(Block::Heading);
    }
    for ch in ['`', '~'] {
        let n = t.chars().take_while(|c| *c == ch).count();
        if n >= 3 && !(ch == '`' && t[n..].contains('`')) {
            return Some(Block::Fence(ch, n));
        }
    }
    if let Some(kind) = html_block_start(t) {
        return Some(Block::Html(kind));
    }
    let marks: String = t.chars().filter(|c| !matches!(c, ' ' | '\t')).collect();
    if marks.len() >= 3 && ["-", "_", "*"].iter().any(|m| marks.chars().all(|c| c.to_string() == *m)) {
        return Some(Block::ThematicBreak);
    }
    if t.starts_with(['-', '+', '*']) && space_or_end(&t[1..]) {
        return Some(Block::ListItem);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) && t[digits..].starts_with(['.', ')']) && space_or_end(&t[digits + 1..]) {
        return Some(Block::ListItem);
    }
    if let Some(rest) = t.strip_prefix("[^") {
        if let Some(close) = rest.find("]:") {
            if close > 0 && !rest[..close].contains([' ', '\t', ']']) {
                return Some(Block::Footnote);
            }
        }
    }
    None
}

/// GFM's seven HTML block starts, by their number, on a line with its indentation removed.
fn html_block_start(t: &str) -> Option<u8> {
    let lower = t.to_ascii_lowercase();
    let after_name = |rest: &str| rest.is_empty() || rest.starts_with([' ', '\t', '>']) || rest.starts_with("/>");
    for name in ["script", "pre", "style", "textarea"] {
        if let Some(rest) = lower.strip_prefix('<').and_then(|r| r.strip_prefix(name)) {
            if rest.is_empty() || rest.starts_with([' ', '\t', '>']) {
                return Some(1);
            }
        }
    }
    if lower.starts_with("<!--") {
        return Some(2);
    }
    if lower.starts_with("<?") {
        return Some(3);
    }
    if lower.starts_with("<![cdata[") {
        return Some(5);
    }
    // (GFM 0.29: `<!` and an uppercase letter, `<!DOCTYPE`; `<!x` is text)
    if t.strip_prefix("<!").is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase())) {
        return Some(4);
    }
    const BLOCK_TAGS: [&str; 62] = [
        "address", "article", "aside", "base", "basefont", "blockquote", "body", "caption", "center", "col", "colgroup", "dd", "details", "dialog", "dir", "div", "dl", "dt",
        "fieldset", "figcaption", "figure", "footer", "form", "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hr", "html", "iframe", "legend",
        "li", "link", "main", "menu", "menuitem", "nav", "noframes", "ol", "optgroup", "option", "p", "param", "section", "source", "summary", "table", "tbody", "td",
        "tfoot", "th", "thead", "title", "tr", "track", "ul",
    ];
    let tag = lower.strip_prefix("</").or_else(|| lower.strip_prefix('<'));
    if let Some(tag) = tag {
        let name_len = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).count();
        if BLOCK_TAGS.contains(&&tag[..name_len]) && after_name(&tag[name_len..]) {
            return Some(6);
        }
    }
    complete_tag_alone(t).then_some(7)
}

/// A line that is one complete open or closing tag and whitespace (GFM's HTML block 7).
fn complete_tag_alone(t: &str) -> bool {
    let b = t.trim_end().as_bytes();
    let name = |b: &[u8], i: usize| -> usize {
        let mut j = i;
        if j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
        }
        j
    };
    let skip_ws = |b: &[u8], mut i: usize| {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n') {
            i += 1;
        }
        i
    };
    if b.first() != Some(&b'<') {
        return false;
    }
    if b.get(1) == Some(&b'/') {
        let end = name(b, 2);
        if end == 2 {
            return false;
        }
        let i = skip_ws(b, end);
        return i + 1 == b.len() && b[i] == b'>';
    }
    let mut i = name(b, 1);
    if i == 1 {
        return false;
    }
    loop {
        let ws = skip_ws(b, i);
        if ws < b.len() && (b[ws] == b'>' || (b[ws] == b'/' && b.get(ws + 1) == Some(&b'>'))) {
            return if b[ws] == b'>' { ws + 1 == b.len() } else { ws + 2 == b.len() };
        }
        // an attribute needs whitespace before it
        if ws == i || ws >= b.len() || !(b[ws].is_ascii_alphabetic() || b[ws] == b'_' || b[ws] == b':') {
            return false;
        }
        i = ws + 1;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b':' | b'-')) {
            i += 1;
        }
        let eq = skip_ws(b, i);
        if eq < b.len() && b[eq] == b'=' {
            let v = skip_ws(b, eq + 1);
            match b.get(v) {
                Some(q @ (b'"' | b'\'')) => {
                    let Some(close) = b[v + 1..].iter().position(|c| c == q) else { return false };
                    i = v + 1 + close + 1;
                }
                Some(_) => {
                    let mut j = v;
                    while j < b.len() && !matches!(b[j], b' ' | b'\t' | b'\n' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`') {
                        j += 1;
                    }
                    if j == v {
                        return false;
                    }
                    i = j;
                }
                None => return false,
            }
        }
    }
}

/// Where an HTML block of GFM's kind `kind` ends: on the line holding its end mark (kinds 1 to 5),
/// or before a blank line (6 and 7).
fn html_block_ends(kind: u8, line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    match kind {
        1 => ["</script>", "</pre>", "</style>", "</textarea>"].iter().any(|m| lower.contains(m)),
        2 => line.contains("-->"),
        3 => line.contains("?>"),
        4 => line.contains('>'),
        5 => line.contains("]]>"),
        _ => line.trim().is_empty(),
    }
}

/// A line that closes a fence of `n` marks `ch`: as many or more, then whitespace alone.
fn closes_fence(line: &str, ch: char, n: usize) -> bool {
    let t = line.trim_start_matches(' ');
    column_of(line) <= 3 && t.chars().take_while(|c| *c == ch).count() >= n && t.trim_start_matches(ch).trim().is_empty()
}

#[test]
fn the_record_is_read_in_its_one_form() {
    // the one form: the heading line, prose, a blank line, the table to the end
    let canon = |rows: &str| format!("# Fork\n\nAbout it.\n\n## Local changes\nProse of the section,\non two lines.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n{rows}");
    assert_eq!(change_rows(&canon("| 1 | a | base | none |\n| 1b | b \\| c | `xlsx` | none |\n\n")), Ok(vec!["1".to_string(), "1b".to_string()]));
    let refused = |record: &str| change_rows(record).is_err();
    // the heading read as GFM reads one, one and only one (the rounds at 36bd57e3, c0de423e and
    // 8736f277); the reading takes the plain line alone
    let decoy = "# Fork\n##  Local changes\n| # | Change |\n|---|---|\n| 35 | no commit names it |\n## Local changes\n| # | Change |\n|---|---|\n| 1 | a |\n";
    assert!(local_changes_heading(decoy).unwrap_err().contains("2 `Local changes` headings"));
    for one in ["##  Local changes\n", "## Local changes  \n", "##\tLocal changes\n", "   ## Local changes ##\n"] {
        assert!(local_changes_heading(one).is_ok(), "{one:?}");
        assert!(refused(&format!("{one}\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 7 | a | b | c |\n")), "{one:?}");
    }
    assert!(local_changes_heading("## Local changes\nLocal changes\n---\n").is_err());
    let plain = "\n\n## Local changes\n| # | Change |\n|---|---|\n| 1 | a |\n";
    for decoy in ["## Local&Tab;changes", "## Local&NewLine;changes", "## Local&NonBreakingSpace;changes", "## Local&emsp13;changes", "## Local changes&lrm;", "## Local changes\u{200E}", "## \u{200F}Local changes", "## Local changes\u{2061}", "## Local\u{180E} changes", "## [Local changes][x]\n\n[x]: #a", "## [Local changes](a(b))", "## Local&UnknownSpace;changes", "10. A note:\n\n    ## Local changes", "- a\n  - b\n\n    ## Local changes", "- a\n\n  ## Local changes", "## Local&#32;changes", "## Local&#x20;changes", "## Local&nbsp;changes", "## Local changes<!-- -->", "## *Local changes*", "## `Local` changes", "## [Local changes](#x)", "Local\nchanges\n---", "> ## Local changes", "- ## Local changes", "1. ## Local changes", "## Local\u{a0}changes", "# Local changes"] {
        let record = format!("{decoy}\n| # | Change |\n|---|---|\n| 36 | no commit names it |{plain}");
        assert!(local_changes_heading(&record).is_err(), "{decoy:?}");
        // and before the one form's heading, the reading fails on it too
        assert!(refused(&format!("{decoy}\n\n{}", canon("| 1 | a | b | c |\n"))), "{decoy:?}");
    }
    for hidden in ["<!--\n## Local changes\n-->", "<div>\n## Local changes\n</div>", "\u{a0}## Local changes", "\u{3000}## Local changes", "```\n## Local changes\n```", "    ## Local changes"] {
        assert!(local_changes_heading(&format!("{hidden}{plain}")).is_ok(), "{hidden:?}");
    }
    let record = "## Local&#32;changes\n| # | Change |\n|---|---|\n| 36 | a |\n\n\u{a0}## Local changes\n| # | Change |\n|---|---|\n| 1 | a |\n";
    assert!(local_changes_heading(record).is_err());
    assert!(local_changes_heading("Local changes\n---\n| 1 | a |\n").is_err());
    assert!(local_changes_heading("### Local changes\n").is_err());
    assert!(local_changes_heading("## Local changesx\n").is_err());
    // the round at 27f6099e: headings GitHub reads as `Local changes` the decoder took otherwise,
    // and look-alike letters, fail the reading
    for decoy in ["## [Local changes](a\\)b)", "## [Local changes](x \"a)b\")", "## [Local changes](x 'a)b')", "## Local changes\u{034F}", "## Local\u{3164}changes", "## Local changes\u{FE0F}", "Intro\r## Local changes"] {
        assert!(refused(&format!("{decoy}\n\n{}", canon("| 1 | a | b | c |\n"))), "{decoy:?}");
    }
    // every construct beyond the one form fails the reading, each where a round found GitHub's
    // reading and a reading's apart: among the rows, between the prose and the table, after the
    // table
    for between_rows in [
        "",
        "- 5 | a list item |",
        "1. 5 | an ordered item |",
        "***",
        "[^5]: a footnote",
        "<div>",
        "<span class=\"x\">",
        "``` | a fence |",
        "#5 | not a heading |",
        "<b>5</b> | inline html |",
        "-5 | no space after the dash |",
        "    | 30 | code, not a row |",
        "   | 29 | indented | x | none |",
        "|28| **no space** | x | none |",
        "32 | no leading pipe | base | none",
        "> | 31 | quoted | x | none |",
        "| 39 | three | cells |",
        "| 39 | five | cells | x | y |",
        "| 39 | a | b | c |\r| 40 | after a lone CR | x | none |",
    ] {
        assert!(refused(&canon(&format!("| 1 | a | b | c |\n{between_rows}\n| 2 | b | c | d |\n"))), "{between_rows:?}");
    }
    for before_table in [
        "- More:",
        "> quoted:",
        "10. Later:",
        "<!--",
        "<span class=\"x\">",
        "    indented code",
        "```",
        "Notes\n---",
        "Notes\n===",
        "Some paragraph text\n2. | # | Change |\n|---|---|---|\n| 39 | x | y |",
        "text\n+ \n|---|\n| 39 |",
        "- a\n  2. | # | Change |\n  |---|---|---|\n  | 39 | x | y |",
        "- ```",
        "> ### Notes",
        "- ### Notes",
        "text\n    x\n<span>",
        "- a\n    b\n| # | Change |\n|---|---|\n| 39 | x |",
        "> a\n    b\n| # | Change |\n|---|---|\n| 39 | x |",
        "> ## Notes\n\n| # | x |\n|---|---|\n| 39 | not a change |",
        "| # | Change | Where | Upstream |\n|---|---|---|---|\n| 39 | a second table | x | none |",
        "-\t| # | Change | Where | Upstream |\n\t|---|---|---|---|\n\t| 38 | after a tab marker | base | none |",
    ] {
        let record = format!("# Fork\n\n## Local changes\nProse.\n{before_table}\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_table:?}");
    }
    for after_table in ["\n## After\n| 31 | another table |", "\n<!--\n| 30 | hidden |\n-->", "\nMore prose.", "\n- a\n\n  | # | Change |\n  |---|---|\n  | 33 | listed |"] {
        assert!(refused(&canon(&format!("| 1 | a | b | c |\n{after_table}\n"))), "{after_table:?}");
    }
    // the round at 0394f0ba: an HTML block before the heading that holds it (`<textarea>`, kind 1,
    // runs past a blank line; kind 7 after a table row or on a lazy line), a pipeless table's
    // delimiter or an underline with a tab in the prose, a row GitHub splits otherwise
    for before_heading in ["<textarea>\n", "para\n<textarea>", "| | |\n|---|---|\n| Rows | below |\n<x-y>", "> a\n<x-y>", "[^1]: a\n<x-y>", "- a\n<x-y>"] {
        let record = format!("# Fork\n\n{before_heading}\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_heading:?}");
        let record = format!("# Fork\n\n{before_heading}\n\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_heading:?} and a blank line");
    }
    for prose in [":-\n40 an unlisted change, base, none", "-:", ":-:", "===\t", "---\t"] {
        let record = format!("# Fork\n\n## Local changes\nProse.\n{prose}\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{prose:?}");
    }
    assert!(refused(&canon("| 1 | a | b | c |\n| 4 | a \\\\| b | c |\n")));
    assert_eq!(gfm_cells("| 4 | a \\\\| b | c |"), ["4", "a \\\\| b", "c"]);
    // the round at 7a0bedec: a fence GitHub closes otherwise, before the heading or in an item;
    // white space other than the space; raw HTML in a cell; a link reference definition in the
    // prose; HTML in a footnote or a nested item
    for before_heading in [
        "```\nA note.\n```\u{a0}",
        "```\nA note.\n```\u{3000}",
        "```\nA note.\n```\u{b}",
        "```\nA note.\n```\u{c}",
        "- A note:\n\n  ```\n  x\n```",
        "Local\n\u{a0}\nchanges\n---\n| # | Change |\n|---|---|\n| 41 | an uncommitted change |",
        "[^1]: <textarea>",
        "- a\n    - <details>",
    ] {
        let record = format!("# Fork\n\n{before_heading}\n\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_heading:?}");
    }
    for row in [
        "| 1 | a </td><td>x</td><td>y</td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none</td></tr><tr><td>1</td><td>(cont.) | b | c |",
        "| 1 | a </table> | b | c |",
        "| 1 | a </td><td> x | b | c |",
        "| 1 | a \\`<b>\\` | b | c |",
    ] {
        assert!(refused(&canon(&format!("{row}\n"))), "{row:?}");
    }
    assert!(refused(&format!("# Fork\n\n## Local changes\n[x]: #a \"Each was\nfirst made\"\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n")));
    // and (the round at b6dc3602) a `<` in a code span too: which `<` GitHub reads as HTML turns on
    // how it pairs backticks
    assert!(refused(&canon("| 1 | `<v>9</v>` | `a <b>` | none |\n")));
    // the round at d7cb0b35: a backtick GitHub pairs otherwise (a row's span across its cells, a
    // backtick in a link's target, a span across a line break) letting raw HTML through, a
    // definition's label over two lines, a fence in a nested item
    for row in [
        "| 41 | a | `x.rs`, ` | none</td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none ` |",
        "| 41 | a | b | [none](#n`) </td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none `x` |",
    ] {
        assert!(refused(&canon(&format!("| 1 | a | b | c |\n{row}\n"))), "{row:?}");
    }
    for before_heading in [
        "A note on the `x\ny` <details> `z` copy.",
        "A note on the [copy](#c`) <details> `x`.",
        "A note on the [copy](#c \"the\n`\") <details> ` copy.",
        "[Each change\nlisted]: #a \"The rows below are\nnot every change\"",
        "[^a\nb]: x",
        "- A note:\n    - ```\n      Shown as code, not prose.",
        "- a\n    - [x]: #y",
    ] {
        let record = format!("# Fork\n\n{before_heading}\n\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_heading:?}");
    }
    assert!(refused(&format!("# Fork\n\n## Local changes\n[Each change\nlisted]: #a\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n")));
    // the round at b6dc3602: an extended autolink, and a link's target past an escaped `)`, each
    // taking a backtick GitHub's code spans then pair otherwise, raw HTML in a row or before the
    // heading: a `<` anywhere is refused
    for row in [
        "| 41 | a | b | none www.a.com/`x </td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none</td></tr><tr><td>39</td><td>(cont.) `y` z |",
        "| 41 | a | b | none ftp://a.example/`x </td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none</td></tr><tr><td>39</td><td>(cont.) `y` z |",
        "| 41 | a | b | [none](#n\\)`) </td></tr><tr><td>41</td><td>an uncommitted change</td><td>base</td><td>none</td></tr><tr><td>39</td><td>(cont.) `x` and |",
    ] {
        assert!(refused(&canon(&format!("| 1 | a | b | c |\n{row}\n"))), "{row:?}");
    }
    for before_heading in [
        "See http://a.example/`x <details> `y` and the copy.",
        "(www.a.example/`x <details> `y`",
        "A note](www.a.example/x)`y <details> `z`",
        "A note on the [copy](#c\\)`) <details> `x` and its copy.",
        "![the logo](#c\\)`) <details> `x`",
    ] {
        let record = format!("# Fork\n\n{before_heading}\n\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n");
        assert!(refused(&record), "{before_heading:?}");
    }
    // and with no `<`, a backtick GitHub pairs otherwise, a link's title, an escaped backtick are
    // only inline text to it: read, as the real record's first lines hold a link
    for before_heading in ["A copy of [IronCalc](https://example.org/a_b).", "A [copy](#c 'the') of it.", "Escaped \\` backtick and www.a.com/`x `y`."] {
        let record = format!("# Fork\n\n{before_heading}\n\n## Local changes\nProse.\n\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | `a \\| b` | c ` | none |\n");
        assert_eq!(change_rows(&record), Ok(vec!["1".to_string()]), "{before_heading:?}");
    }
    assert_eq!(after_every_container("- a"), "a");
    assert_eq!(after_every_container("    - > 1. [x]: y"), "[x]: y");
    // the table after the heading with no blank line, a delimiter of another spelling, no table
    assert!(refused(&format!("## Local changes\n{RECORD_HEADER}\n{RECORD_DELIMITER}\n| 1 | a | b | c |\n")));
    assert!(refused(&format!("## Local changes\n\n{RECORD_HEADER}\n|:--|---|---|---|\n| 1 | a | b | c |\n")));
    assert!(refused("## Local changes\nProse alone.\n"));
    // cells split at unescaped pipes alone, one leading pipe passed over
    assert_eq!(gfm_cells("||28| a \\| b |"), ["", "28", "a \\| b"]);
}

/// The change each commit subject names, `(fork change 7)` as `7`, `(fork change 1, refined)` as
/// `1b`; a subject naming none is left out (the caller counts them).
fn regex_free_change_ids(subjects: &str) -> Vec<String> {
    let mut out = Vec::new();
    for s in subjects.lines() {
        let Some(at) = s.rfind("(fork change ") else { continue };
        let rest = &s[at + "(fork change ".len()..];
        let Some(end) = rest.find(')') else { continue };
        let inner = &rest[..end];
        let (number, refined) = match inner.split_once(", refined") {
            Some((n, _)) => (n, true),
            None => (inner, false),
        };
        if number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty() {
            out.push(if refined { format!("{number}b") } else { number.to_string() });
        }
    }
    out
}
