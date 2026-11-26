//! C-295: the component list (docs/alforge-plan.md, "The component list"; docs/questions.md Q31
//! entry 38): a commit's CycloneDX 1.6 document, read from the commit alone, the same bytes from
//! `alforge components` and from the cut (whose own run, `tests/cut.rs`, holds its bundle's list to
//! it); every component with its version and licence, every package by path it uses, every crate
//! its version names by package URL, checksum and licence, and the dependencies between them.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// A throwaway repository: files written and committed on `main`, removed on drop.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Repo {
        let r = Repo { dir: tempfile::tempdir().unwrap() };
        r.git(&["init", "-q", "-b", "main"]);
        r
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_COMMITTER_DATE", "2026-11-26T12:00:00+01:00")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
        self.git(&["rev-parse", "HEAD"])
    }
}

/// A suite whose portal ships an executable and uses core and a fork by path and `itoa` from
/// crates.io (a version the machine's own cargo holds, its lock made offline), and whose core is a
/// library alone; `itoa` for a test of core's alone too.
fn suite() -> (Repo, String) {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\", \"crates/veredas-portal\"]\nexclude = [\"forks/garfo\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT OR Apache-2.0\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.3\"\nedition = \"2024\"\nlicense = \"MIT OR Apache-2.0\"\n\n\
         [dependencies]\nveredas-core = { path = \"../veredas-core\" }\ngarfo = { path = \"../../forks/garfo/lib\" }\nitoa = \"1\"\n",
    );
    r.write("crates/veredas-portal/src/main.rs", "fn main() {}\n");
    r.write("forks/garfo/Cargo.toml", "[workspace]\nmembers = [\"lib\"]\n");
    r.write("forks/garfo/lib/Cargo.toml", "[package]\nname = \"garfo\"\nversion = \"0.3.0\"\nedition = \"2024\"\nlicense-file = \"LICENCA\"\n");
    r.write("forks/garfo/lib/LICENCA", "the fork's own terms\n");
    r.write("forks/garfo/lib/src/lib.rs", "\n");
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let c = r.commit("a suite to list");
    (r, c)
}

fn entry<'a>(doc: &'a Value, bom_ref: &str) -> &'a Value {
    doc["components"].as_array().unwrap().iter().find(|c| c["bom-ref"] == bom_ref).unwrap_or_else(|| panic!("{bom_ref} not listed"))
}

#[test]
fn a_commit_lists_its_components_its_packages_by_path_and_its_crates_with_their_dependencies() {
    let (r, c) = suite();
    let lock = std::fs::read_to_string(r.path().join("Cargo.lock")).unwrap();
    let itoa_at = lock.find("name = \"itoa\"").unwrap();
    let itoa_version = lock[itoa_at..].lines().nth(1).unwrap().trim_start_matches("version = \"").trim_end_matches('"').to_string();
    let itoa_sum = lock[itoa_at..].lines().find(|l| l.starts_with("checksum")).unwrap().trim_start_matches("checksum = \"").trim_end_matches('"').to_string();

    let text = alforge::components::list(r.path(), &c, Some("2.0.0")).unwrap();
    assert!(text.ends_with("}\n"));
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((doc["bomFormat"].as_str(), doc["specVersion"].as_str(), doc["version"].as_i64()), (Some("CycloneDX"), Some("1.6"), Some(1)));
    // The release, dated at its commit (UTC), named by it.
    let m = &doc["metadata"];
    assert_eq!(m["timestamp"], "2026-11-26T11:00:00Z");
    assert_eq!((m["component"]["name"].as_str(), m["component"]["version"].as_str(), m["component"]["bom-ref"].as_str()), (Some("veredas"), Some("2.0.0"), Some("veredas@2.0.0")));
    assert_eq!(m["component"]["properties"][0], serde_json::json!({ "name": "veredas:commit", "value": c }));
    assert_eq!(m["tools"]["components"][0]["name"], "alforge");
    let serial = doc["serialNumber"].as_str().unwrap();
    let uuid = serial.strip_prefix("urn:uuid:").unwrap();
    let groups: Vec<usize> = uuid.split('-').map(str::len).collect();
    assert_eq!(groups, [8, 4, 4, 4, 12], "{serial}");
    assert!(uuid.chars().all(|x| x == '-' || x.is_ascii_hexdigit() && !x.is_ascii_uppercase()), "{serial}");

    // Each component, at its version, by its licence; portal ships its executable.
    let portal = entry(&doc, "veredas-portal@2.0.3");
    assert_eq!(portal["type"], "application");
    assert_eq!(portal["licenses"], serde_json::json!([{ "expression": "MIT OR Apache-2.0" }]));
    assert_eq!(portal["properties"], serde_json::json!([{ "name": "veredas:commit", "value": c }, { "name": "veredas:path", "value": "crates/veredas-portal" }, { "name": "veredas:bins", "value": "veredas-portal" }]));
    let core = entry(&doc, "veredas-core@2.1.0");
    assert_eq!(core["type"], "library");
    // The fork, a package by path a component uses, its licence in a file of its own.
    let garfo = entry(&doc, "garfo@0.3.0");
    assert_eq!(garfo["licenses"], serde_json::json!([{ "license": { "name": "the package's own file forks/garfo/lib/LICENCA" } }]));
    assert_eq!(garfo["properties"][1], serde_json::json!({ "name": "veredas:path", "value": "forks/garfo/lib" }));
    // The crate, by its package URL and the lock's checksum, its licence as its manifest states.
    let purl = format!("pkg:cargo/itoa@{itoa_version}");
    let itoa = entry(&doc, &purl);
    assert_eq!((itoa["purl"].as_str(), itoa["name"].as_str()), (Some(purl.as_str()), Some("itoa")));
    assert_eq!(itoa["hashes"], serde_json::json!([{ "alg": "SHA-256", "content": itoa_sum }]));
    assert_eq!(itoa["licenses"], serde_json::json!([{ "expression": "MIT OR Apache-2.0" }]));
    assert_eq!(doc["components"].as_array().unwrap().len(), 4, "{text}");
    // The dependencies as the lock states them.
    let deps: Vec<(String, Vec<String>)> = doc["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["ref"].as_str().unwrap().to_string(), d["dependsOn"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()))
        .collect();
    assert_eq!(
        deps,
        [
            ("garfo@0.3.0".to_string(), vec![]),
            (purl.clone(), vec![]),
            ("veredas-core@2.1.0".to_string(), vec![]),
            ("veredas-portal@2.0.3".to_string(), vec!["garfo@0.3.0".to_string(), purl.clone(), "veredas-core@2.1.0".to_string()]),
        ]
    );

    // One commit and release, one document; another release, another serial number; the commit
    // alone names itself.
    assert_eq!(alforge::components::list(r.path(), &c, Some("2.0.0")).unwrap(), text);
    let other: Value = serde_json::from_str(&alforge::components::list(r.path(), &c, Some("2.0.1")).unwrap()).unwrap();
    assert_ne!(other["serialNumber"], doc["serialNumber"]);
    assert_eq!(other["components"], doc["components"]);
    let alone: Value = serde_json::from_str(&alforge::components::list(r.path(), &c, None).unwrap()).unwrap();
    assert_eq!(alone["metadata"]["component"]["version"], c.as_str());
    assert_eq!((alone["components"].clone(), alone["dependencies"].clone()), (doc["components"].clone(), doc["dependencies"].clone()));

    // The executable prints the same, from any folder of the repository, by any name of the commit.
    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_alforge")).args(args).current_dir(r.path().join("crates")).output().unwrap();
    let out = run(&["components", "HEAD", "--release", "2.0.0"]);
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned()), (Some(0), text.clone()));
    let out = run(&["components", "nada"]);
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).trim().to_string()), (Some(2), format!("nada is no commit of {}", r.path().canonicalize().unwrap().display())));
    let out = run(&["components", "HEAD", "--release", "dois"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(&["components"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_crate_of_another_source_is_named_by_its_own_package_url() {
    let reg = "registry+https://github.com/rust-lang/crates.io-index";
    assert_eq!(alforge::components::purl("serde", "1.0.200", reg), "pkg:cargo/serde@1.0.200");
    assert_eq!(
        alforge::components::purl("itoa", "1.0.11", "git+https://example.invalid/itoa?rev=0123#0123456789abcdef0123456789abcdef01234567"),
        "pkg:cargo/itoa@1.0.11?vcs_url=git%2Bhttps%3A%2F%2Fexample.invalid%2Fitoa%400123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(alforge::components::purl("x", "0.1.0", "sparse+https://registry.example.invalid/index/"), "pkg:cargo/x@0.1.0?repository_url=https%3A%2F%2Fregistry.example.invalid%2Findex%2F");
}

#[test]
fn this_repositorys_head_lists_every_component_and_every_crate_its_versions_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let head = alforge::git::text(&root, &["rev-parse", "HEAD"]).unwrap().unwrap().trim().to_string();
    let doc: Value = serde_json::from_str(&alforge::components::list(&root, &head, None).unwrap()).unwrap();
    let listed: Vec<&Value> = doc["components"].as_array().unwrap().iter().collect();
    let held = alforge::manifest::components_at(&root, &head, &mut Vec::new()).unwrap();
    for h in held.values() {
        assert!(listed.iter().any(|c| c["bom-ref"] == format!("{}@{}", h.name, h.version)), "{} listed", h.name);
    }
    // Every crate listed is one a component's version names (C-290), with its licence and the
    // lock's checksum; the database engine's among them, a component's own dependency.
    let mut named = std::collections::BTreeSet::new();
    for h in held.values() {
        named.extend(h.inputs.as_ref().unwrap().0.keys().filter_map(|k| k.strip_prefix("Cargo.lock: ")).map(|k| k.split(" (").next().unwrap().to_string()));
    }
    let purls: Vec<&Value> = listed.iter().filter(|c| c["purl"].is_string()).copied().collect();
    assert!(purls.len() > 100, "{}", purls.len());
    for c in &purls {
        let key = format!("{} {}", c["name"].as_str().unwrap(), c["version"].as_str().unwrap());
        assert!(named.contains(&key), "{key} is named by a component's version");
        assert!(c["licenses"].is_array(), "{} states a licence", c["bom-ref"]);
        assert_eq!(c["hashes"][0]["alg"], "SHA-256", "{}", c["bom-ref"]);
    }
    assert!(purls.iter().any(|c| c["name"] == "libduckdb-sys"));
    // No crate of another platform alone (Windows' own, which the lock holds and the version names).
    assert!(named.iter().any(|n| n.starts_with("windows-sys ")));
    assert!(!purls.iter().any(|c| c["name"].as_str().is_some_and(|n| n.starts_with("windows"))));
    // Ábaco uses the IronCalc fork kept in the tree, by path.
    assert!(listed.iter().any(|c| c["properties"].as_array().is_some_and(|p| p.iter().any(|x| x["value"] == "forks/ironcalc/base"))));
}
