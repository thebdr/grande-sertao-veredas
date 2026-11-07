//! The gateway's own rules that no suite law checks (contract: the component clause; the laws
//! in `tools/leis` judge its name, version, dependencies and plan).
//!
//! A library first, one executable on top (`docs/portal-plan.md`, "Rules"): cargo is asked what
//! it would build (`cargo metadata --no-deps`, which parses and builds nothing), and the targets
//! must be exactly one library of `lib`/`rlib` crate types, at most one binary named
//! `veredas-portal`, and tests. Content-agnostic (`docs/questions.md` Q8): no file of the crate
//! names the vendor protocol core's telegram layer came from, or cites a vendor document. The
//! light build (no default features) carries no database, no server and no desktop crate, and
//! no member of the suite but core in its normal or build dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY: &str = "veredas-portal";

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
    let strings =
        |v: &serde_json::Value| -> Vec<String> { v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect() };
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), strings(&t["kind"]), strings(&t["crate_types"])))
        .collect()
}

/// The rule: exactly one library of `lib`/`rlib` crate types, at most one binary named
/// `binary`, everything else a test. Returns what breaks it, empty when it holds.
fn breaches(targets: &[(String, Vec<String>, Vec<String>)], binary: &str) -> Vec<String> {
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
        out.push(format!("{bins} binaries"));
    }
    out
}

#[test]
fn the_gateway_is_one_library_and_at_most_one_executable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let t = targets(&dir);
    assert_eq!(breaches(&t, BINARY), Vec::<String>::new(), "{t:?}");
    let lib: Vec<_> = t.iter().filter(|(_, k, _)| k == &["lib"]).collect();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].0, "veredas_portal");
}

/// A throwaway package outside the workspace, with the files given.
fn planted(case: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-portal-component-{}-{case}", std::process::id()));
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
    let main = ("src/main.rs", "fn main() {}\n");
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("plain", vec![("Cargo.toml", manifest(""))]),
        ("main", vec![("Cargo.toml", manifest("")), (main.0, main.1.into())]),
        ("named", vec![("Cargo.toml", manifest("[[bin]]\nname = \"x\"\npath = \"src/main.rs\"\n")), (main.0, main.1.into())]),
        ("other", vec![("Cargo.toml", manifest("[[bin]]\nname = \"y\"\npath = \"src/main.rs\"\n")), (main.0, main.1.into())]),
        (
            "two",
            vec![
                ("Cargo.toml", manifest("")),
                (main.0, main.1.into()),
                ("src/bin/z.rs", "fn main() {}\n".into()),
            ],
        ),
        ("example", vec![("Cargo.toml", manifest("[[example]]\nname = \"e\"\npath = \"src/lib.rs\"\n"))]),
        ("examples", vec![("Cargo.toml", manifest("")), ("examples/e.rs", "fn main() {}\n".into())]),
        ("cratetype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"rlib\", \"bin\"]\n"))]),
        ("cdylib", vec![("Cargo.toml", manifest("[lib]\ncrate-type = [\"cdylib\"]\n"))]),
        ("emptytype", vec![("Cargo.toml", manifest("[lib]\ncrate-type = []\n"))]),
        ("bench", vec![("Cargo.toml", manifest("")), ("benches/b.rs", "fn main() {}\n".into())]),
        ("build", vec![("Cargo.toml", manifest("")), ("build.rs", "fn main() {}\n".into())]),
        ("nolib", vec![("Cargo.toml", manifest("autolib = false\n")), (main.0, main.1.into())]),
    ];
    let mut verdicts = Vec::new();
    for (case, files) in &cases {
        let mut all: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        if *case != "nolib" {
            all.push(LIB);
        }
        let dir = planted(case, &all);
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

/// Every file under `dir`, recursively, whatever its extension: rustc reads more than `.rs`,
/// and the manifest is part of the crate too.
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

/// Text reversed: the banned terms are written backwards here, so that no line of this file,
/// however normalised, spells one.
fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Where a file names the vendor protocol core's telegram layer came from, its terms, or cites
/// a vendor document, as `line N: <term>`: core's reader (`tests/foundation.rs`) over the text.
/// A file that is not UTF-8 (a fixture in a plant's encoding: this crate reads such files for
/// a living) is read lossily for the names (a UTF-16 NUL is no letter, so the letters join up;
/// CP1252 names are ASCII), and its section signs are counted in the bytes themselves, where
/// the lossy reading loses them: the byte 0xA7, which is the sign in CP1252 and the one non-zero
/// byte of its UTF-16 forms (A7 00, 00 A7), unless it follows 0xC2 (the UTF-8 pair, which the
/// text reading already counts).
fn vendor_mentions_in(bytes: &[u8]) -> Vec<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return vendor_mentions(text);
    }
    let mut hits = vendor_mentions(&String::from_utf8_lossy(bytes));
    for (i, b) in bytes.iter().enumerate() {
        if *b == 0xA7 && !(i > 0 && bytes[i - 1] == 0xC2) {
            let line = bytes[..i].iter().filter(|b| **b == b'\n').count() + 1;
            hits.push(format!("line {line}: section sign (byte {i})"));
        }
    }
    hits
}

/// Where a text names the vendor protocol core's telegram layer came from, its terms, or cites
/// a vendor document, as `line N: <term>`: the same reader as core's (`tests/foundation.rs`) and
/// the bench's. The whole text is lowercased and stripped to letters and digits as one string
/// first, so no separator, case, spacing or line break hides a name; each hit names the line
/// where it starts. A section sign is a specification citation.
fn vendor_mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> = ["tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao"]
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
fn the_gateway_names_no_vendor_and_cites_no_vendor_document() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    crate_files(&root, &mut files);
    assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("lib.rs")) && files.iter().any(|f| f.ends_with("component.rs")), "{files:?}");
    assert!(files.len() >= 10, "{files:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| vendor_mentions_in(&std::fs::read(f).unwrap()).into_iter().map(move |l| format!("{}: {l}", f.display())))
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // The citation is seen in a plant's encodings too, where a UTF-8 reading loses it: CP1252's
    // one byte, UTF-16 in either order, each exactly once; the UTF-8 pair once, not twice.
    let section = char::from_u32(0xA7).unwrap();
    let cp1252: Vec<u8> = b"// spec \xA73.2\n".to_vec();
    assert_eq!(vendor_mentions_in(&cp1252).len(), 1, "{:?}", vendor_mentions_in(&cp1252));
    let utf16le: Vec<u8> = format!("// spec {section}3.2\n").encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(vendor_mentions_in(&utf16le).len(), 1, "{:?}", vendor_mentions_in(&utf16le));
    let utf16be: Vec<u8> = format!("// spec {section}3.2\n").encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    assert_eq!(vendor_mentions_in(&utf16be).len(), 1, "{:?}", vendor_mentions_in(&utf16be));
    assert_eq!(vendor_mentions_in(format!("// spec {section}3.2\n").as_bytes()).len(), 1, "the UTF-8 pair once");
    assert_eq!(vendor_mentions_in(&[0xA7]).len(), 1, "a lone first byte");
    assert_eq!(vendor_mentions_in(b"x\n\xA7\n\xA7").len(), 2, "two signs, on lines 2 and 3");
    assert_eq!(vendor_mentions_in(b"x\n\xA7\n\xA7")[1], "line 3: section sign (byte 4)");
    assert_eq!(vendor_mentions_in("// a degree 25\u{B0}C and \u{E9} and \u{167}\n".as_bytes()), Vec::<String>::new(), "other non-ASCII, a 0xA7 continuation byte included, is not a citation");
    assert_eq!(vendor_mentions_in(b"// cp1252 25\xB0C na\xEFve\n"), Vec::<String>::new());
    // The names are still read in a plant's encodings.
    let name16: Vec<u8> = format!("// the {} catalog\n", rev("tleBineG")).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(vendor_mentions_in(&name16).len(), 1, "{:?}", vendor_mentions_in(&name16));
    let mut name1252 = format!("// the {} catalog ", rev("tleBineG")).into_bytes();
    name1252.extend_from_slice(b"25\xB0C\n");
    assert_eq!(vendor_mentions_in(&name1252).len(), 1);
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

/// Crates the light build must not carry: a database, a server, a desktop UI, and any member of
/// the suite but core.
const BANNED: &[&str] = &["duckdb", "libduckdb", "axum", "hyper", "eframe", "egui", "winit", "veredas-"];

/// The banned crates among `names`: the gateway itself and `veredas-core` excepted.
fn banned_among<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|n| *n != "veredas-core" && *n != BINARY && BANNED.iter().any(|b| n.starts_with(b)))
        .map(String::from)
        .collect()
}

/// Every crate in a package's resolved normal and build dependency graph with the features
/// given (dev-dependencies left out: the bench is a fixture, and a fixture may carry anything),
/// by name: what cargo would build for it, asked with `cargo metadata` and walked from its node
/// by package id, following only the edges whose kinds are normal or build.
fn dependency_graph(manifest: &Path, features: &[&str]) -> (std::collections::BTreeSet<String>, usize) {
    // Not offline: the graph spans every platform, and a registry cache holds only what this
    // platform fetched to build, so a fresh machine may fetch the rest of the index.
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["metadata", "--format-version", "1"]);
    for f in features {
        cmd.arg(f);
    }
    let out = cmd.arg("--manifest-path").arg(manifest).output().unwrap();
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
            // A dependency may be declared in several tables; follow it when any is not dev.
            let not_dev = dep["dep_kinds"]
                .as_array()
                .map(|kinds| kinds.iter().any(|k| k["kind"].as_str() != Some("dev")))
                .unwrap_or(true);
            if not_dev {
                queue.push(dep["pkg"].as_str().unwrap().to_string());
            }
        }
    }
    (names, seen.len())
}

#[test]
fn the_light_build_carries_no_database_no_server_no_desktop_and_no_member_but_core() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let (graph, nodes) = dependency_graph(&manifest, &["--no-default-features"]);
    for expected in ["veredas-core", "notify", "encoding_rs", "regex", "tokio", "async-opcua-client"] {
        assert!(graph.contains(expected), "{expected} not in the graph: {graph:?}");
    }
    assert!(nodes >= graph.len(), "{nodes} nodes, {} names", graph.len());
    assert!(!graph.contains("veredas-maquinar"), "a dev-dependency was walked: {graph:?}");
    assert_eq!(banned_among(graph.iter().map(String::as_str)), Vec::<String>::new());
    // The ban list itself, on planted names.
    assert_eq!(
        banned_among([
            "duckdb",
            "libduckdb-sys",
            "axum",
            "hyper-util",
            "eframe",
            "egui_extras",
            "winit",
            "veredas-maquinar",
            "veredas-core",
            "veredas-portal",
            "tokio",
            "veredas_abaco"
        ]),
        ["duckdb", "libduckdb-sys", "axum", "hyper-util", "eframe", "egui_extras", "winit", "veredas-maquinar"]
    );
}

/// A planted package `x` beside path crates, with `deps` as its dependency tables; what the walk
/// finds banned in its graph.
fn banned_in_planted(case: &str, deps: &str, crates: &[(&str, &str)]) -> Vec<String> {
    let mut files: Vec<(String, String)> = vec![("x/Cargo.toml".into(), manifest(deps)), ("x/src/lib.rs".into(), LIB.1.into())];
    for (folder, name) in crates {
        files.push((
            format!("{folder}/Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"2.0.0\"\nedition = \"2024\"\n"),
        ));
        files.push((format!("{folder}/src/lib.rs"), String::new()));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    let dir = planted(case, &refs);
    let (graph, _) = dependency_graph(&dir.join("x/Cargo.toml"), &[]);
    std::fs::remove_dir_all(&dir).unwrap();
    banned_among(graph.iter().map(String::as_str))
}

#[test]
fn the_graph_walk_follows_normal_and_build_edges_and_skips_dev_ones() {
    let crates = [("duck", "duckdb"), ("ui", "eframe"), ("member", "veredas-maquinar")];
    assert_eq!(
        banned_in_planted("graph-normal", "[dependencies]\nduckdb = { path = \"../duck\" }\n", &crates),
        ["duckdb"]
    );
    assert_eq!(
        banned_in_planted("graph-build", "[build-dependencies]\neframe = { path = \"../ui\" }\n", &crates),
        ["eframe"]
    );
    assert_eq!(
        banned_in_planted("graph-dev", "[dev-dependencies]\nveredas-maquinar = { path = \"../member\" }\n", &crates),
        Vec::<String>::new()
    );
    assert_eq!(
        banned_in_planted(
            "graph-both",
            "[dependencies]\nduckdb = { path = \"../duck\" }\n[dev-dependencies]\nduckdb = { path = \"../duck\" }\n",
            &crates
        ),
        ["duckdb"]
    );
    assert_eq!(banned_in_planted("graph-clean", "", &crates), Vec::<String>::new());
}
