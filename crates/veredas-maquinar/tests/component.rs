//! The bench's own rules that no suite law checks (contract: the bench clause; the laws in
//! `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library first, with at most one executable on top (`docs/maquinar-plan.md`, "Rules"). Cargo
//! is asked what it would build (`cargo metadata --no-deps`, which parses and builds nothing): the
//! targets of `veredas-maquinar` are exactly one library of `lib`/`rlib` crate types, at most one
//! binary named `veredas-maquinar`, and tests. An example, a bench, a build script, a second
//! binary or a missing library each break the rule.
//!
//! Content-agnostic, like core (`docs/questions.md` Q8): no file of the crate names the vendor
//! protocol core's telegram layer was ported from, or cites a vendor document.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The one executable the bench may build.
const BINARY: &str = "veredas-maquinar";

/// What cargo would build for the package at `dir`: `(name, kinds, crate types)` per target.
fn targets(dir: &Path) -> Vec<(String, Vec<String>, Vec<String>)> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let manifest = dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap();
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
    };
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), strings(&t["kind"]), strings(&t["crate_types"])))
        .collect()
}

/// The rule: exactly one library target of `lib`/`rlib` crate types, at most one binary named
/// `binary`, everything else a test. Returns what breaks it, empty when it holds.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)], binary: &str) -> Vec<String> {
    // An empty list is no library: cargo accepts `crate-type = []` and then builds nothing.
    let library = |kinds: &[String]| !kinds.is_empty() && kinds.iter().all(|k| k == "lib" || k == "rlib");
    let mut out = Vec::new();
    let mut libs = 0;
    let mut bins = 0;
    for (name, kinds, crate_types) in targets {
        if kinds == &["test"] {
            continue;
        }
        if library(kinds) && library(crate_types) {
            libs += 1;
            continue;
        }
        if kinds == &["bin"] && crate_types == &["bin"] && name == binary {
            bins += 1;
            continue;
        }
        out.push(format!("{name}: {kinds:?} {crate_types:?}"));
    }
    if libs != 1 {
        out.push(format!("{libs} library targets"));
    }
    if bins > 1 {
        out.push(format!("{bins} binaries named {binary}"));
    }
    out
}

#[test]
fn the_bench_is_one_library_and_at_most_one_executable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let t = targets(&dir);
    assert_eq!(breaches(&t, BINARY), Vec::<String>::new(), "{t:?}");
    let lib: Vec<_> = t.iter().filter(|(_, k, _)| k == &["lib"]).collect();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].0, "veredas_maquinar");
}

/// A throwaway package outside the workspace, with the files given.
fn planted(case: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-maquinar-component-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

const LIB: (&str, &str) = ("src/lib.rs", "pub fn f() {}\n");

fn manifest(extra: &str) -> String {
    format!("[workspace]\n\n[package]\nname = \"x\"\nversion = \"2.0.0\"\nedition = \"2024\"\n{extra}")
}

#[test]
fn the_check_allows_one_named_binary_and_refuses_every_other_executable() {
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("plain", vec![("Cargo.toml", manifest(""))]),
        ("main", vec![("Cargo.toml", manifest("")), ("src/main.rs", "fn main() {}\n".into())]),
        ("named", vec![("Cargo.toml", manifest("[[bin]]\nname = \"x\"\npath = \"src/lib.rs\"\n"))]),
        ("other", vec![("Cargo.toml", manifest("[[bin]]\nname = \"y\"\npath = \"src/lib.rs\"\n"))]),
        (
            "two",
            vec![("Cargo.toml", manifest("")), ("src/main.rs", "fn main() {}\n".into()), ("src/bin/y.rs", "fn main() {}\n".into())],
        ),
        ("example", vec![("Cargo.toml", manifest("[[example]]\nname = \"e\"\npath = \"src/lib.rs\"\n"))]),
        ("examples", vec![("Cargo.toml", manifest("")), ("examples/e.rs", "fn main() {}\n".into())]),
        ("cratetype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"rlib\", \"bin\"]\n"))]),
        ("cdylib", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"cdylib\"]\n"))]),
        ("emptytype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = []\n"))]),
        ("bench", vec![("Cargo.toml", manifest("")), ("benches/b.rs", "fn main() {}\n".into())]),
        ("build", vec![("Cargo.toml", manifest("")), ("build.rs", "fn main() {}\n".into())]),
        // Only a binary and tests, no library.
        (
            "nolib",
            vec![("Cargo.toml", manifest("autolib = false\n")), ("src/main.rs", "fn main() {}\n".into()), ("tests/t.rs", "#[test]\nfn t() {}\n".into())],
        ),
    ];
    let mut verdicts = Vec::new();
    for (case, files) in &cases {
        let mut all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        if *case != "nolib" {
            all.push(LIB);
        }
        let dir = planted(case, &all);
        // The planted package is named `x`, so its one allowed binary is `x`.
        verdicts.push((*case, breaches(&targets(&dir), "x").is_empty()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert_eq!(
        verdicts,
        [
            ("plain", true),
            ("main", true),
            ("named", true),
            ("other", false),
            ("two", false),
            ("example", false),
            ("examples", false),
            ("cratetype", false),
            ("cdylib", false),
            ("emptytype", false),
            ("bench", false),
            ("build", false),
            ("nolib", false),
        ]
    );
}

/// Every file under `dir`, recursively, whatever its extension: rustc reads more than `.rs`
/// (`include!`, `#[path]`), the manifest is part of the crate, and a fixture is read by a test.
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

/// The workspace root cargo runs rustc from for the package at `manifest_dir`: the folder
/// dep-info paths are relative to.
fn workspace_root(manifest_dir: &Path) -> PathBuf {
    let out = Command::new(env!("CARGO"))
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .current_dir(manifest_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).parent().unwrap().to_path_buf()
}

/// The paths one dep-info line lists after its target: `target: dep dep ...`, a space in a path
/// escaped as `\ `.
fn dep_info_paths(line: &str) -> Vec<String> {
    let Some(at) = line.find(": ") else { return Vec::new() };
    let marker = '\u{1}';
    let deps = line[at + 2..].replace("\\ ", &marker.to_string());
    deps.split_whitespace().map(|p| p.replace(marker, " ")).collect()
}

/// The crate roots of every target of the package at `manifest_dir`, as cargo lists them
/// (`src_path`, wherever a `path` put them), canonical.
fn target_roots(manifest_dir: &Path) -> Vec<PathBuf> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path"])
        .arg(manifest_dir.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let manifest = manifest_dir.join("Cargo.toml").canonicalize().unwrap();
    let package = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == manifest)
        .unwrap();
    package["targets"].as_array().unwrap().iter().map(|t| PathBuf::from(t["src_path"].as_str().unwrap()).canonicalize().unwrap()).collect()
}

/// Every source file rustc read for the package at `manifest_dir`, as cargo's dep-info files
/// record it once `cargo build --all-targets --all-features` has run: each target's crate root
/// (wherever a `path` put it), its modules, and every file an `include_str!`, `include_bytes!`,
/// `include!` or `#[path]` pulled in, from anywhere, in whatever form rustc accepted (a raw
/// string, a `concat!`, an `env!`, a macro, an alias, a comment between the macro and its
/// argument). The compiler's own list, never a reading of the text; a build that fails fails
/// the check. Every dep-info whose crate root is one of the package's targets (a stale one from
/// an earlier build included: it can only add files) contributes the files of it that still
/// exist. What the build saw is what this platform's `cfg` compiled in: a file pulled in only
/// on another platform is seen by that platform's run of this check.
fn files_rustc_read(manifest_dir: &Path, target_dir: &Path, package: &str) -> Vec<PathBuf> {
    let out = Command::new(env!("CARGO"))
        .args(["build", "--all-targets", "--all-features", "--offline", "-p", package, "--target-dir"])
        .arg(target_dir)
        .current_dir(manifest_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "cargo build --all-targets: {}", String::from_utf8_lossy(&out.stderr));
    let root = workspace_root(manifest_dir);
    let roots = target_roots(manifest_dir);
    let deps = target_dir.join("debug").join("deps");
    let mut files: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut seen_roots: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&deps).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("d") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(first) = text.lines().find(|l| l.contains(": ")) else { continue };
        let listed = dep_info_paths(first);
        let resolve = |p: &str| {
            let p = Path::new(p);
            if p.is_absolute() { p.to_path_buf() } else { root.join(p) }
        };
        let Some(crate_root) = listed.first().map(|p| resolve(p)) else { continue };
        let Ok(crate_root) = crate_root.canonicalize() else { continue };
        if !roots.contains(&crate_root) {
            continue;
        }
        seen_roots.insert(crate_root);
        for p in listed {
            if let Ok(canonical) = resolve(&p).canonicalize() {
                files.insert(canonical);
            }
        }
    }
    for r in &roots {
        assert!(seen_roots.contains(r), "no dep-info for the target at {}: the build did not compile it", r.display());
    }
    files.into_iter().collect()
}

/// The crate has no doctest: a doctest is compiled by rustdoc, outside any dep-info, so a file
/// it pulled in would escape the check. Asked of `cargo test --doc`, which reports what it ran.
fn no_doctests(manifest_dir: &Path, target_dir: &Path, package: &str) {
    let out = Command::new(env!("CARGO"))
        .args(["test", "--doc", "--all-features", "--offline", "-p", package, "--target-dir"])
        .arg(target_dir)
        .current_dir(manifest_dir)
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "cargo test --doc: {text}");
    assert!(text.contains("running 0 tests"), "the crate has doctests, which this check cannot read: {text}");
}

/// Where this test's own build put its artifacts: the target dir the nested build reuses, so
/// nothing is rebuilt twice.
fn own_target_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    // <target>/debug/deps/<test>.exe
    exe.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf()
}

/// Every file of the crate: those under its folder (fixtures, the manifest and documents a
/// test reads included), and every file rustc read for it from anywhere, each once.
fn all_crate_files(manifest_dir: &Path, target_dir: &Path, package: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    crate_files(manifest_dir, &mut files);
    let mut seen: std::collections::BTreeSet<PathBuf> = files.iter().map(|f| f.canonicalize().unwrap()).collect();
    for read in files_rustc_read(manifest_dir, target_dir, package) {
        if seen.insert(read.clone()) {
            files.push(read);
        }
    }
    files
}

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one.
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites a
/// vendor document, as `line N: <term>`: the same reader as core's (`tests/foundation.rs`). The
/// whole text is lowercased and stripped to letters and digits as one string first, so no
/// separator, case, spacing or line break hides a name; each hit names the line where it
/// starts. A section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> =
        ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"]
            .iter()
            .map(|b| rev(b))
            .collect();
    let mut flat = String::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in line.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase) {
            flat.push(c);
            line_of.push(n + 1);
        }
    }
    let chars: Vec<char> = flat.chars().collect();
    let mut hits = Vec::new();
    for term in &banned {
        let t: Vec<char> = term.chars().collect();
        for start in 0..chars.len().saturating_sub(t.len() - 1) {
            if chars[start..start + t.len()] == t[..] {
                hits.push(format!("line {}: {term}", line_of[start]));
            }
        }
    }
    let section = char::from_u32(0xA7).unwrap();
    for (n, line) in text.lines().enumerate() {
        if line.contains(section) {
            hits.push(format!("line {}: section sign", n + 1));
        }
    }
    hits
}

#[test]
fn the_bench_names_no_vendor_and_cites_no_vendor_document() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    no_doctests(&root, &own_target_dir(), BINARY);
    let files = all_crate_files(&root, &own_target_dir(), BINARY);
    assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("lib.rs")) && files.iter().any(|f| f.ends_with("component.rs")), "{files:?}");
    assert!(files.len() >= 4, "{files:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.display()))
        })
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The reader sees every spelling, in any case, across line breaks, and the vendor's terms
    // and citations; each sample holds exactly one, found on the line it starts.
    let samples = [
        (format!("// the {} catalog", rev("tleBineG")), 1),
        (format!("const {}: u16 = 1;", rev("TROP_TLEB_INEG")), 1),
        (format!("// see {}/spec.txt", rev("RODNEV/SCOD")), 1),
        (format!("// the {} speaks", rev("lortnoC troS retsaM")), 1),
        (format!("// {} telegrams", rev("MAOOTXTR")), 1),
        (format!("// spec {}3.2", char::from_u32(0xA7).unwrap()), 1),
        (format!("// a {}", rev("ecafretni-tsoh")), 1),
        (format!("fn x() {{}}\n/// the {}\n/// {} over TCP", rev("troS retsaM"), rev("lortnoC")), 2),
        (format!("/// the {}\n/// {} catalog", rev("-ineG"), rev("tleB")), 1),
        (format!("id = 51004 # {}", rev("maootxtr")), 1),
    ];
    for (s, line) in &samples {
        let hits = vendor_mentions(s);
        assert_eq!(hits.len(), 1, "{s}: {hits:?}");
        assert!(hits[0].starts_with(&format!("line {line}: ")), "{s}: {hits:?}");
    }
    assert_eq!(vendor_mentions("fn decode(frame: &[u8]) {}\n// an announcement\nid = 21"), Vec::<String>::new());
}

#[test]
fn every_file_rustc_pulls_in_from_outside_the_crate_folder_is_read_too() {
    // A planted crate, built for real, whose targets pull in files outside its folder by every
    // form rustc accepts, each holding a vendor term: the compiler's dep-info lists every one.
    // The markers are built here, never written in this file: the planted sources are not
    // the bench's, and a planted `include` read as text would be.
    let (inc_str, inc_bytes, inc_mac) = (["inc", "lude_str!"].concat(), ["inc", "lude_bytes!"].concat(), ["inc", "lude!"].concat());
    let dir = planted("rustc-includes", &[]);
    let outside = dir.join("outside");
    let abs = |name: &str| outside.join(name).to_string_lossy().replace('\\', "/");
    let lib = format!(
        "pub const A: &str = INCSTR!(\"../../outside/a.txt\");\n\
         pub const B: &[u8] = INCBYTES!( \"../../outside/b.bin\" );\n\
         #[path = \"../../outside/c.rs\"]\npub mod c;\n\
         pub const D: &str = INCSTR!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../outside/d.txt\"));\n\
         INCMAC!(\"../../outside/e.rs\");\n\
         pub const G: &str = INCSTR!(r\"{g}\");\n\
         pub const H: &str = INCSTR!(r#\"{h}\"#);\n\
         pub const I: &str = INCSTR![\"../../outside/i.txt\"];\n\
         pub const J: &str = INCSTR!{{\"../../outside/j.txt\"}};\n\
         #[ path = \"../../outside/k.rs\" ]\npub mod k;\n\
         #[cfg_attr(all(), path = \"../../outside/l.rs\")]\npub mod l;\n\
         pub const M: &str = INCSTR!(concat!(\"{m_dir}\", \"/m.txt\"));\n\
         pub const N: &str = INCSTR!(concat!( env!(\"CARGO_MANIFEST_DIR\"), \"/../outside\", \"/n.txt\" ));\n\
         pub const O: &str = INCSTR!(concat!(\"../../outside/o\", \".txt\"));\n\
         macro_rules! inc {{ ($p:literal) => {{ INCSTR!($p) }}; }}\n\
         pub const Q: &str = inc!(\"../../outside/q.txt\");\n\
         // the attribute opener #[ starts every attribute\n\
         #[path = \"../../outside/r.rs\"]\npub mod r;\n\
         #[path = \"../../outside/p/root.rs\"]\npub mod root;\n\
         pub const S: &str = INCSTR!(concat!(\"{m_dir}/\", env!(\"CARGO_PKG_NAME\"), \".txt\"));\n\
         pub const T: &str = INCSTR! /* a comment */ (\"../../outside/t.txt\");\n\
         use std::INCSTRALIAS as grab;\n\
         pub const U: &str = grab!(\"../../outside/u.txt\");\n\
         #[cfg(any())]\npub const V: &str = INCSTR!(\"../../outside/v.txt\");\n\
         #[cfg(feature = \"extra\")]\npub const W: &str = INCSTR!(\"../../outside/w.txt\");\n\
         #[cfg(windows)]\npub const Y: &str = INCSTR!(\"../../outside/y-windows.txt\");\n\
         #[cfg(unix)]\npub const Y: &str = INCSTR!(\"../../outside/y-unix.txt\");\n",
        g = abs("g.txt"),
        h = abs("h.txt"),
        m_dir = outside.to_string_lossy().replace('\\', "/"),
    )
    .replace("INCSTRALIAS", inc_str.trim_end_matches('!'))
    .replace("INCSTR!", &inc_str)
    .replace("INCBYTES!", &inc_bytes)
    .replace("INCMAC!", &inc_mac);
    let terms = [rev("tleBineG"), rev("MAOOTXTR"), rev("lortnoC troS retsaM"), rev("rodnev/scod"), rev("ecafretni tsoh"), rev("XTROTMAO")];
    let term = |i: usize| format!("x {} y", terms[i % terms.len()]);
    let mut files: Vec<(String, String)> = vec![
        (
            "crate/Cargo.toml".into(),
            "[workspace]\n\n[package]\nname = \"x\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[features]\nextra = []\n\n[[test]]\nname = \"elsewhere\"\npath = \"../outside/elsewhere.rs\"\n".into(),
        ),
        ("crate/src/lib.rs".into(), lib),
        ("outside/elsewhere.rs".into(), format!("// {}\n#[test]\nfn t() {{}}\n", term(3))),
        ("crate/src/inner.rs".into(), "// an announcement\n".into()),
        ("outside/e.rs".into(), "// an outcome\n// and a nested one\npub const F: &str = INCSTR!(\"f.txt\");\n".replace("INCSTR!", &inc_str)),
        ("outside/f.txt".into(), format!("{}3.1", char::from_u32(0xA7).unwrap())),
        ("outside/p/root.rs".into(), "pub mod inline { #[path = \"x.rs\"] pub mod m; }\n".into()),
        ("outside/p/inline/x.rs".into(), format!("// {}\npub fn f() {{}}\n", term(0))),
        ("outside/x.txt".into(), term(1)),
    ];
    let named = ["a.txt", "b.bin", "c.rs", "d.txt", "g.txt", "h.txt", "i.txt", "j.txt", "k.rs", "l.rs", "m.txt", "n.txt", "o.txt", "q.txt", "r.rs", "t.txt", "u.txt"];
    for (i, name) in named.iter().enumerate() {
        let text = if name.ends_with(".rs") { format!("// {}\npub fn f() {{}}\n", term(i)) } else { term(i) };
        files.push((format!("outside/{name}"), text));
    }
    // The file `cfg(any())` leaves out holds a term too: rustc never reads it, so it is not
    // the crate's. A file behind a feature that is off by default is read (every feature is
    // on); of the two platform files, this platform's is read and the other's is not: a file
    // pulled in only on another platform is that platform's run's to see.
    files.push(("outside/v.txt".into(), term(2)));
    files.push(("outside/w.txt".into(), term(4)));
    files.push(("outside/y-windows.txt".into(), term(5)));
    files.push(("outside/y-unix.txt".into(), term(0)));
    for (path, text) in &files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let manifest_dir = dir.join("crate");
    let target_dir = dir.join("target");
    let read = files_rustc_read(&manifest_dir, &target_dir, "x");
    let names: Vec<String> = read.iter().map(|f| f.file_name().unwrap().to_string_lossy().to_string()).collect();
    let (here, elsewhere) = if cfg!(windows) { ("y-windows.txt", "y-unix.txt") } else { ("y-unix.txt", "y-windows.txt") };
    let mut expected: Vec<String> = vec!["lib.rs".into(), "e.rs".into(), "f.txt".into(), "root.rs".into(), "x.rs".into(), "x.txt".into(), "elsewhere.rs".into(), "w.txt".into(), here.into()];
    expected.extend(named.iter().map(|n| n.to_string()));
    for e in &expected {
        assert!(names.contains(e), "{e} missing from what rustc read: {names:?}");
    }
    assert!(!names.contains(&"inner.rs".to_string()), "a module never declared is not read: {names:?}");
    assert!(!names.contains(&"v.txt".to_string()), "a cfg'd-out include is not read: {names:?}");
    assert!(!names.contains(&elsewhere.to_string()), "another platform's file is not read here: {names:?}");
    // And through the whole reader: every term found, each file reported once.
    let all = all_crate_files(&manifest_dir, &target_dir, "x");
    let found: Vec<String> = all
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.file_name().unwrap().to_string_lossy()))
        })
        .collect();
    let mut reported: Vec<String> = named.iter().map(|n| n.to_string()).collect();
    reported.extend(["f.txt".to_string(), "x.rs".to_string(), "x.txt".to_string(), "elsewhere.rs".to_string(), "w.txt".to_string(), here.to_string()]);
    assert_eq!(found.len(), reported.len(), "{found:?}");
    for file in reported {
        assert!(found.iter().any(|h| h.starts_with(&format!("{file}: "))), "{file} not reported: {found:?}");
    }
    assert!(all.iter().any(|f| f.ends_with("inner.rs")), "a file in the folder is read whether or not rustc did");
    // The dep-info reader on its own line forms.
    assert_eq!(dep_info_paths("C:\\t\\deps\\x.d: crates\\x\\src\\lib.rs C:\\a\\b.txt"), ["crates\\x\\src\\lib.rs", "C:\\a\\b.txt"]);
    assert_eq!(dep_info_paths("/t/x.d: /a/with\\ space.txt /b.rs"), ["/a/with space.txt", "/b.rs"]);
    assert_eq!(dep_info_paths("# env-dep:CARGO=x"), Vec::<String>::new());
    assert_eq!(dep_info_paths("crates\\x\\src\\lib.rs:"), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Crates the bench must never carry, by name prefix: a database (test-only and light), a
/// desktop UI (Q12), and any member of the suite but core (the plan's rule: core alone until
/// aranha exists; never portal, whose store and desktop the bench never pulls).
const BANNED: &[&str] = &["duckdb", "libduckdb", "eframe", "egui", "winit", "veredas-"];

/// The banned crates among `names`: the bench itself and `veredas-core` excepted.
fn banned_among<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|n| *n != "veredas-core" && *n != BINARY && BANNED.iter().any(|b| n.starts_with(b)))
        .map(String::from)
        .collect()
}

/// Every crate in a package's resolved dependency graph (normal, build and dev, every feature
/// on, transitively), by name: what cargo would build for it, asked with `cargo metadata` and
/// walked from its node by package id (a crate present in two versions is two nodes, each
/// walked). Returns the names, and how many nodes the walk visited.
fn dependency_graph(manifest: &Path) -> (std::collections::BTreeSet<String>, usize) {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--offline", "--all-features", "--manifest-path"])
        .arg(manifest)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let packages = meta["packages"].as_array().unwrap();
    let name_of = |id: &str| -> String {
        packages.iter().find(|p| p["id"].as_str() == Some(id)).map(|p| p["name"].as_str().unwrap().to_string()).unwrap()
    };
    let wanted = manifest.canonicalize().unwrap();
    let root = packages
        .iter()
        .find(|p| Path::new(p["manifest_path"].as_str().unwrap()).canonicalize().unwrap() == wanted)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let nodes = meta["resolve"]["nodes"].as_array().unwrap();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut queue = vec![root];
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        names.insert(name_of(&id));
        let node = nodes.iter().find(|n| n["id"].as_str() == Some(&id)).unwrap();
        for dep in node["deps"].as_array().unwrap() {
            queue.push(dep["pkg"].as_str().unwrap().to_string());
        }
    }
    (names, seen.len())
}

#[test]
fn the_bench_carries_no_database_no_desktop_and_no_member_but_core() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let (graph, nodes) = dependency_graph(&manifest);
    // The walk reaches the direct dependencies and the transitive ones behind them, and every
    // node, a crate present twice included (the graph holds more nodes than names).
    for expected in ["veredas-core", "async-opcua-server", "async-opcua-client", "tokio", "async-opcua-core", "parking_lot", "regex-syntax"] {
        assert!(graph.contains(expected), "{expected} not in the graph: {graph:?}");
    }
    assert!(nodes >= graph.len(), "{nodes} nodes, {} names", graph.len());
    assert_eq!(banned_among(graph.iter().map(String::as_str)), Vec::<String>::new());
    // The ban list itself, on planted names.
    assert_eq!(
        banned_among(["duckdb", "libduckdb-sys", "eframe", "egui_extras", "winit", "veredas-portal", "veredas-core", "veredas-maquinar", "tokio", "veredas_portal"]),
        ["duckdb", "libduckdb-sys", "eframe", "egui_extras", "winit", "veredas-portal"]
    );
}

/// A planted package `x` beside path crates, with `deps` as its dependency tables; what the walk
/// finds banned in its graph.
fn banned_in_planted(case: &str, deps: &str, crates: &[(&str, &str, &str)]) -> Vec<String> {
    let mut files: Vec<(&str, String)> = vec![("x/Cargo.toml", manifest(deps)), ("x/src/lib.rs", LIB.1.into())];
    let mut owned = Vec::new();
    for (folder, name, version) in crates {
        owned.push((format!("{folder}/Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n")));
        owned.push((format!("{folder}/src/lib.rs"), String::new()));
    }
    // Every planted crate is its own workspace, as `manifest` makes `x`.
    let owned: Vec<(String, String)> = owned
        .into_iter()
        .map(|(p, t)| if p.ends_with("Cargo.toml") { (p, format!("[workspace]\n\n{t}")) } else { (p, t) })
        .collect();
    files.extend(owned.iter().map(|(p, t)| (p.as_str(), t.clone())));
    let all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let dir = planted(case, &all);
    let (graph, _) = dependency_graph(&dir.join("x/Cargo.toml"));
    let banned = banned_among(graph.iter().map(String::as_str));
    std::fs::remove_dir_all(&dir).unwrap();
    banned
}

#[test]
fn the_graph_walk_sees_every_version_every_feature_and_every_table() {
    // Two versions of one crate, only the first leading to a banned crate, in either order of
    // declaration: the walk goes by package id, so both are expanded.
    let two = |first: &str, second: &str| {
        format!("[dependencies]\n{first} = {{ path = \"../foo1\", package = \"foo\" }}\n{second} = {{ path = \"../foo2\", package = \"foo\" }}\n")
    };
    let crates = [("foo1", "foo", "1.0.0"), ("foo2", "foo", "2.0.0"), ("sys", "libduckdb-sys", "0.1.0")];
    let with_sys = |case: &str, deps: String| {
        // foo 1.0.0 depends on the banned crate.
        let mut files: Vec<(&str, String)> = vec![("x/Cargo.toml", manifest(&deps)), ("x/src/lib.rs", LIB.1.into())];
        for (folder, name, version) in crates {
            let extra = if folder == "foo1" { "\n[dependencies]\nlibduckdb-sys = { path = \"../sys\" }\n" } else { "" };
            files.push((Box::leak(format!("{folder}/Cargo.toml").into_boxed_str()), format!("[workspace]\n\n[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n{extra}")));
            files.push((Box::leak(format!("{folder}/src/lib.rs").into_boxed_str()), String::new()));
        }
        let all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        let dir = planted(case, &all);
        let (graph, nodes) = dependency_graph(&dir.join("x/Cargo.toml"));
        let banned = banned_among(graph.iter().map(String::as_str));
        std::fs::remove_dir_all(&dir).unwrap();
        (banned, nodes)
    };
    assert_eq!(with_sys("twoversions-a", two("foo_a", "foo_b")), (vec!["libduckdb-sys".to_string()], 4));
    assert_eq!(with_sys("twoversions-b", two("foo_b", "foo_a")), (vec!["libduckdb-sys".to_string()], 4));
    // An optional dependency behind a feature that is off is still carried.
    assert_eq!(
        banned_in_planted(
            "optional",
            "[features]\nstore = [\"dep:duckdb\"]\n\n[dependencies]\nduckdb = { path = \"../duckdb\", optional = true }\n",
            &[("duckdb", "duckdb", "1.0.0")]
        ),
        ["duckdb"]
    );
    // A rename, a target table and a dev table are seen; a clean package is clean.
    assert_eq!(
        banned_in_planted("rename", "[dependencies]\nstore = { path = \"../db\", package = \"duckdb\" }\n", &[("db", "duckdb", "1.0.0")]),
        ["duckdb"]
    );
    assert_eq!(
        banned_in_planted("target", "[target.'cfg(any())'.dependencies]\negui = { path = \"../egui\" }\n", &[("egui", "egui", "0.1.0")]),
        ["egui"]
    );
    assert_eq!(
        banned_in_planted("dev", "[dev-dependencies]\neframe = { path = \"../eframe\" }\n", &[("eframe", "eframe", "0.1.0")]),
        ["eframe"]
    );
    assert_eq!(banned_in_planted("clean", "[dependencies]\nfine = { path = \"../fine\" }\n", &[("fine", "fine", "0.1.0")]), Vec::<String>::new());
}
