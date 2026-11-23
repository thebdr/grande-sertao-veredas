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
    assert_eq!(tests, ["component", "links", "patch", "patch_corpus", "patch_no_styles", "patch_shapes", "patch_untouched"].iter().map(|s| s.to_string()).collect());
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
    // writes packages with, serialisation and the error derive, the engine's two by path under the
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
        ["ironcalc", "ironcalc_base", "serde", "serde_json", "thiserror", "zip"].iter().map(|s| s.to_string()).collect()
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
/// database, socket, HTTP, file-watching or async runtime crate). A crate joining the graph fails
/// this test until it is reviewed and added.
const REVIEWED: &[&str] = &[
    "aho-corasick", "android_system_properties", "approx", "bitcode", "bitcode_derive", "bitflags", "bumpalo",
    "bytemuck", "cfg-if", "chacha20", "chrono", "chrono-tz", "constant_time_eq", "core-foundation-sys",
    "cpufeatures", "crc32fast", "csv", "csv-core", "either", "equivalent", "flate2", "futures-core",
    "futures-task", "futures-util", "getrandom", "hashbrown", "iana-time-zone", "iana-time-zone-haiku",
    "indexmap", "ironcalc", "ironcalc_base", "itertools", "itoa", "js-sys", "libc", "libm", "log", "memchr",
    "nix", "num-traits", "phf", "phf_shared", "pin-project-lite", "proc-macro2", "quick-xml", "quote", "r-efi",
    "rand", "rand_core", "regex", "regex-automata", "regex-lite", "regex-syntax", "ryu", "serde", "serde_core",
    "serde_derive", "serde_json", "simd-adler32", "siphasher", "slab", "statrs", "syn", "thiserror",
    "thiserror-impl", "typed-path", "unicode-ident", "veredas-abaco", "wasm-bindgen", "wasm-bindgen-macro",
    "wasm-bindgen-macro-support", "wasm-bindgen-shared", "windows-core", "windows-implement",
    "windows-interface", "windows-link", "windows-result", "windows-strings", "windows-sys", "zeroize",
    "zeroize_derive", "zip", "zlib-rs", "zmij", "zopfli",
];

/// The crates Ábaco's graph gains in the suite's workspace build from features other members turn
/// on (cargo unifies a crate's features across the build), each reviewed: `miniz_oxide` and
/// `adler2`, flate2's second backend; `foldhash`, hashbrown's default hasher, which maquinar's
/// physics (`parry3d`) turns on, the hasher-free table indexmap uses unchanged. A crate joining
/// this way fails until it is reviewed and added.
const WORKSPACE_EXTRA: &[&str] = &["adler2", "foldhash", "miniz_oxide"];

/// The targets the suite is built and tested on: this machine's, the gate, and the Windows VM's.
const SUITE_TARGETS: [&str; 2] = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"];

/// The features Ábaco's crates carry in the suite's test build beyond Ábaco built alone, per
/// target, each reviewed (`.zen/evidence/C-220.md`): serde implementations (chrono, indexmap,
/// serde's and serde_core's `alloc`, which a `std` build has anyway), flate2's second backend and
/// what it turns on in miniz_oxide, hashbrown's default hasher (indexmap uses its hasher-free
/// table), num-traits' 128-bit integers, libc's trait implementations, serde_json's `RawValue`
/// (its parsing of numbers unchanged), syn's parsing and traversal features for the macros that use
/// syn 3 (`full`, `extra-traits`, `fold`, `visit`, `visit-mut`: the macros' output unchanged), and on Windows the
/// API bindings of windows-sys others call. None changes what the engine computes; the engine's
/// own two crates gain none. A feature joining fails until it is reviewed and added.
const REVIEWED_FEATURES: [(&str, &[(&str, &[&str])]); 2] = [
    ("x86_64-unknown-linux-gnu", &[
        ("chrono", &["serde"]),
        ("flate2", &["miniz_oxide", "rust_backend"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("libc", &["extra_traits"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
        ("serde", &["alloc"]),
        ("serde_core", &["alloc"]),
        ("serde_json", &["raw_value"]),
        ("syn", &["extra-traits", "fold", "full", "visit", "visit-mut"]),
    ]),
    ("x86_64-pc-windows-msvc", &[
        ("chrono", &["serde"]),
        ("flate2", &["miniz_oxide", "rust_backend"]),
        ("hashbrown", &["default-hasher"]),
        ("indexmap", &["serde"]),
        ("miniz_oxide", &["default", "simd", "simd-adler32", "with-alloc"]),
        ("num-traits", &["i128"]),
        ("serde", &["alloc"]),
        ("serde_core", &["alloc"]),
        ("serde_json", &["raw_value"]),
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

/// The fork's suite, as upstream's tests and the changes' count them on 2026-11-22: upstream's
/// 3 192 and their 8 ignored at `15ecfec`, and the changes' 50 (changes 12, 14, 16, 17, 18 and 20
/// added seven).
const FORK_PASSED: usize = 3242;
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
fn the_fork_s_own_suite_passes_on_the_versions_abaco_builds() {
    let paths = [
        format!("veredas-abaco v2.0.0-alpha.1 ({})", root().join("crates/veredas-abaco").display()),
        format!("ironcalc_base v0.9.1 ({})", root().join("forks/ironcalc/base").display()),
        format!("ironcalc v0.9.1 ({})", root().join("forks/ironcalc/xlsx").display()),
    ];
    // the fork's lock pins every crate of Ábaco's graph on every platform at the version and the
    // source the suite's lock gives it, so the fork's suite tests the versions Ábaco links
    let fork_lock = locked(&root().join("forks/ironcalc/Cargo.lock"));
    let root_lock = locked(&root().join("Cargo.lock"));
    for p in normal_graph(true) {
        if paths.contains(&p) {
            continue;
        }
        assert!(!p.contains('('), "{p}: not from crates.io");
        let mut words = p.split_whitespace();
        let (name, version) = (words.next().unwrap(), words.next().unwrap().trim_start_matches('v'));
        assert!(root_lock.get(name).is_some_and(|v| v.contains(version)), "{name} {version} not in the suite's lock from crates.io");
        assert!(
            fork_lock.get(name).is_some_and(|v| v.contains(version)),
            "{name} {version} in Ábaco's graph, {:?} in the fork's lock",
            fork_lock.get(name)
        );
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
    for file in ["tests/column_outline_roundtrip.rs", "tests/missing_styles_part_import.rs"] {
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
