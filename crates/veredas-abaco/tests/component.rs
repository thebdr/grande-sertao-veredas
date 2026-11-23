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
    let out = Command::new("git").args(args).current_dir(root()).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
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
    let later = git(&["log", "--format=%s", &format!("{base}..HEAD"), "--", "forks/ironcalc/base", "forks/ironcalc/xlsx"]);
    let named = regex_free_change_ids(&later);
    let record = std::fs::read_to_string(root().join("forks/ironcalc/VEREDAS-FORK.md")).unwrap();
    let table = record.split("## Local changes").nth(1).unwrap();
    let rows: Vec<String> = table
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| # ") && !l.starts_with("|---"))
        .map(|l| l.trim_start_matches("| ").split(' ').next().unwrap().to_string())
        .collect();
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
/// their `std`), which others' cryptography turns on. None changes what the engine computes or a
/// digest; the engine's
/// own two crates gain none. A feature joining fails until it is reviewed and added.
const REVIEWED_FEATURES: [(&str, &[(&str, &[&str])]); 2] = [
    ("x86_64-unknown-linux-gnu", &[
        ("chrono", &["serde"]),
        ("const-oid", &["db", "std"]),
        ("crypto-common", &["getrandom", "rand_core"]),
        ("digest", &["const-oid", "mac", "oid", "subtle"]),
        ("flate2", &["miniz_oxide", "rust_backend"]),
        ("getrandom", &["std"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("libc", &["extra_traits"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
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
        ("getrandom", &["std"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
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
    // stable cargo, stable settings
    for (var, _) in env.iter().filter(|(k, v)| {
        !v.is_empty() && (k == "RUSTC_BOOTSTRAP" || k == "__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS" || k.starts_with("CARGO_UNSTABLE_"))
    }) {
        panic!("{whose}: {var} is set: unstable settings");
    }
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
/// ignored at `15ecfec`, and the changes' 61 (seven of them changes 12, 14, 16, 17, 18 and 20's;
/// four, two, one and four changes 22, 23, 24 and 25's).
const FORK_PASSED: usize = 3253;
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
        let foreign: Vec<&String> = t.keys().filter(|k| !CONFIG_KEYS.contains(&k.as_str())).collect();
        assert!(foreign.is_empty(), "{}: {foreign:?} (a key that may patch or replace crates)", config.display());
        // nothing compiles a crate in rustc's place
        let build = t.get("build").and_then(|b| b.as_table());
        for key in ["rustc", "rustc-wrapper", "rustc-workspace-wrapper"] {
            assert!(build.is_none_or(|b| !b.contains_key(key)), "{}: build.{key} set", config.display());
        }
        // and no compiler flag sets a cfg (a feature cargo would not see), however it is carried
        let flags = |v: Option<&toml::Value>| match v {
            Some(toml::Value::String(s)) => flags_set_cfg(s.split_whitespace()),
            Some(toml::Value::Array(a)) => flags_set_cfg(a.iter().filter_map(|t| t.as_str())),
            _ => false,
        };
        assert!(!flags(build.and_then(|b| b.get("rustflags"))), "{}: build.rustflags sets a cfg", config.display());
        for (triple, table) in t.get("target").and_then(|t| t.as_table()).into_iter().flatten() {
            assert!(!flags(table.get("rustflags")), "{}: target.{triple}.rustflags sets a cfg", config.display());
        }
        for (name, table) in t.get("profile").and_then(|t| t.as_table()).into_iter().flatten() {
            assert!(!flags(table.get("rustflags")), "{}: profile.{name}.rustflags sets a cfg", config.display());
        }
        // and no alias gives cargo a configuration of its own
        let aliases = t.get("alias").and_then(|a| a.as_table());
        for (name, value) in aliases.into_iter().flatten() {
            assert!(!value.to_string().contains("--config"), "{}: alias {name} gives cargo a --config", config.display());
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
fn its_sources_are_its_own_and_core_s_header_reader_alone() {
    // what rustc read to compile each library of Ábaco's own and the engine's, from its dep-info:
    // each crate's own files, and of a member's source only core's header reader, compiled into
    // Ábaco by path while core carries its network layers (C-224, docs/questions.md Q345); no other
    // member's code, no other file of core's, nothing a build script wrote
    let json = cargo(&["build", "-p", "veredas-abaco", "--lib", "--message-format=json"]);
    let own: [(&str, &str, &[&str]); 3] = [
        ("veredas_abaco", "crates/veredas-abaco", &["crates/veredas-core/src/schema.rs"]),
        ("ironcalc_base", "forks/ironcalc/base", &[]),
        ("ironcalc", "forks/ironcalc/xlsx", &[]),
    ];
    for (target, folder, allowed) in own {
        let rmeta: Vec<PathBuf> = json
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == target)
            .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
            .filter_map(|f| f.as_str().map(PathBuf::from))
            .filter(|f| f.extension().is_some_and(|e| e == "rmeta"))
            .collect();
        assert_eq!(rmeta.len(), 1, "{target}: {rmeta:?}");
        let stem = rmeta[0].file_stem().unwrap().to_string_lossy().trim_start_matches("lib").to_string();
        let dep_info = rmeta[0].with_file_name(format!("{stem}.d"));
        let text = std::fs::read_to_string(&dep_info).unwrap_or_else(|e| panic!("{}: {e}", dep_info.display()));
        // each file rustc read stands on a line of its own, ending in `:`, a space in its path
        // written `\ `; every one resolves to a file, or the record is not read as it is
        let read: BTreeSet<PathBuf> = text
            .lines()
            .filter_map(|l| l.strip_suffix(':'))
            .filter(|f| !f.replace("\\ ", "").contains(' '))
            .map(|f| {
                let f = f.replace("\\ ", " ");
                let p = Path::new(&f);
                let p = if p.is_absolute() { p.to_path_buf() } else { root().join(p) };
                p.canonicalize().unwrap_or_else(|e| panic!("{target}: {} in its dep-info: {e}", p.display()))
            })
            .collect();
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
        if home.join("build.rs").is_file() {
            stack.push(home.join("build.rs"));
        }
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
            let named = paths_and_includes(&text);
            for name in named {
                let name = name.unwrap_or_else(|| panic!("{}: a path or an include not written as a plain string", file.display()));
                let target = file.parent().unwrap().join(&name);
                let resolved = target.canonicalize().unwrap_or_else(|e| panic!("{}: {name}: {e}", file.display()));
                if Some(resolved.clone()) == allowed.map(|a| root().join(a)) {
                    seen_schema += 1;
                    continue;
                }
                assert!(resolved.starts_with(&home), "{}: `{name}` reaches outside {folder}", file.display());
            }
        }
    }
    assert_eq!(seen_schema, 1, "core's header reader, compiled by path once");
    let links = git(&["ls-files", "-s", "--", "crates/veredas-abaco", "forks/ironcalc"]);
    let links: Vec<&str> = links.lines().filter(|l| l.starts_with("120000 ")).collect();
    assert!(links.is_empty(), "links among the sources: {links:?}");
}

/// Every file a source names by a `path` attribute (`#[path = "…"]`, `#![path = "…"]`, one inside
/// `cfg_attr`) or an include (`include!`, `include_str!`, `include_bytes!`); `None` for one whose
/// argument is no plain string (a `concat!`, an `env!`), which no reading can follow.
fn paths_and_includes(text: &str) -> Vec<Option<String>> {
    let literal = |rest: &str| -> Option<String> {
        let rest = rest.trim_start().strip_prefix('"')?;
        rest.find('"').map(|end| rest[..end].to_string())
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or("");
        if code.contains("#[") || code.contains("#![") {
            let mut from = 0;
            while let Some(at) = code[from..].find("path") {
                let at = from + at;
                let before = code[..at].chars().next_back();
                let rest = code[at + 4..].trim_start();
                if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && rest.starts_with('=') {
                    out.push(literal(&rest[1..]));
                }
                from = at + 4;
            }
        }
        for mac in ["include!", "include_str!", "include_bytes!"] {
            let mut from = 0;
            while let Some(at) = code[from..].find(mac) {
                let at = from + at;
                let before = code[..at].chars().next_back();
                if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    let rest = code[at + mac.len()..].trim_start();
                    out.push(rest.strip_prefix('(').and_then(literal));
                }
                from = at + mac.len();
            }
        }
    }
    out
}

#[test]
fn no_socket_in_its_sources_or_the_engine_s() {
    // no plant connection: its graph holds no network crate (`REVIEWED`), and no source of its own
    // or of the engine's opens a socket through the standard library or libc
    let words = ["std::net", "std::os::unix::net", "TcpStream", "TcpListener", "UdpSocket", "UnixStream", "UnixListener", "UnixDatagram", "libc::socket", "libc::connect"];
    for folder in ["crates/veredas-abaco/src", "forks/ironcalc/base/src", "forks/ironcalc/xlsx/src"] {
        let mut stack = vec![root().join(folder)];
        while let Some(d) = stack.pop() {
            if d.is_dir() {
                for e in std::fs::read_dir(&d).unwrap() {
                    stack.push(e.unwrap().path());
                }
            } else if d.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&d).unwrap();
                let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                for w in words {
                    for (at, _) in text.match_indices(w) {
                        let before = text[..at].chars().next_back();
                        let after = text[at + w.len()..].chars().next();
                        assert!(ident(before) || ident(after), "{}: `{w}`", d.display());
                    }
                }
            }
        }
    }
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
    for file in ["tests/column_outline_roundtrip.rs", "tests/missing_styles_part_import.rs", "tests/excel_function_spelling.rs"] {
        assert!(stderr.contains(file), "{file} did not run:\n{stderr}");
    }
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
