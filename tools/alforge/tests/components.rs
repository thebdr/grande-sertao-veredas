//! C-295: the component list (docs/alforge-plan.md, "The component list"; docs/questions.md Q31
//! entry 38): a commit's CycloneDX 1.6 document, read from the commit alone, the same bytes from
//! `alforge components` and from the cut (whose own run, `tests/cut.rs`, holds its bundle's list to
//! it); every component with its version and licence, every package by path it uses, every crate
//! its version names by package URL, checksum and licence, and the dependencies between them.

use std::collections::BTreeSet;
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

type Package = (String, String);

/// What cargo's own build of `packages` in the workspace at `dir` compiles for the release's target
/// with every feature on, as its unit graph has it (`cargo build --unit-graph`, which compiles
/// nothing; an unstable flag, taken under `RUSTC_BOOTSTRAP` for this check alone), with `envs` set:
/// each package by its name and version, and each edge from a package to another one of its units
/// uses (a library's, a build script's, an executable's).
fn cargo_builds(dir: &Path, packages: &[&str], envs: &[(&str, &Path)]) -> (BTreeSet<Package>, BTreeSet<(Package, Package)>) {
    let target = tempfile::tempdir().unwrap();
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.args(["build", "--unit-graph", "-Z", "unstable-options", "--offline", "--all-features", "--target", alforge::components::TARGET, "--target-dir"]).arg(target.path());
    if dir.join("Cargo.lock").is_file() {
        c.arg("--locked");
    }
    for p in packages {
        c.args(["-p", p]);
    }
    for (k, v) in envs {
        c.env(k, v);
    }
    let out = c.current_dir(dir).env("RUSTC_BOOTSTRAP", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let graph: Value = serde_json::from_slice(&out.stdout).unwrap();
    let units = graph["units"].as_array().unwrap();
    // A package id spec, `<source URL>#<name>@<version>`, or `#<version>` where the URL's last
    // part is the name.
    let spec = |u: &Value| -> Package {
        let (url, frag) = u["pkg_id"].as_str().unwrap().rsplit_once('#').unwrap();
        match frag.split_once('@') {
            Some((n, v)) => (n.to_string(), v.to_string()),
            None => (url.split('?').next().unwrap().trim_end_matches('/').rsplit('/').next().unwrap().to_string(), frag.to_string()),
        }
    };
    let built: BTreeSet<Package> = units.iter().map(spec).collect();
    let mut edges = BTreeSet::new();
    for u in units {
        for d in u["dependencies"].as_array().unwrap() {
            let (from, to) = (spec(u), spec(&units[d["index"].as_u64().unwrap() as usize]));
            if from != to {
                edges.insert((from, to));
            }
        }
    }
    (built, edges)
}

/// The cargo packages a list names (Qt and KDDockWidgets, the bundle's own, aside) and the edges
/// between them, by name and version, to hold against [`cargo_builds`].
fn listed(doc: &Value) -> (BTreeSet<Package>, BTreeSet<(Package, Package)>) {
    let by_ref: std::collections::BTreeMap<&str, Package> = doc["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["purl"].as_str().is_some_and(|p| p.starts_with("pkg:generic/")))
        .map(|c| (c["bom-ref"].as_str().unwrap(), (c["name"].as_str().unwrap().to_string(), c["version"].as_str().unwrap().to_string())))
        .collect();
    let mut edges = BTreeSet::new();
    for d in doc["dependencies"].as_array().unwrap() {
        let Some(from) = by_ref.get(d["ref"].as_str().unwrap()) else { continue };
        for to in d["dependsOn"].as_array().unwrap() {
            edges.insert((from.clone(), by_ref[to.as_str().unwrap()].clone()));
        }
    }
    (by_ref.into_values().collect(), edges)
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
    // A name-based UUID of version 5, as CycloneDX 1.6's pattern takes one (versions 1 to 5, RFC
    // 4122's variant): `^urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`.
    let serial = doc["serialNumber"].as_str().unwrap();
    assert_eq!(serial, alforge::components::serial("2.0.0", &c));
    let uuid = serial.strip_prefix("urn:uuid:").unwrap();
    let groups: Vec<&str> = uuid.split('-').collect();
    assert_eq!(groups.iter().map(|g| g.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12], "{serial}");
    assert!(uuid.chars().all(|x| x == '-' || x.is_ascii_hexdigit() && !x.is_ascii_uppercase()), "{serial}");
    assert!(groups[2].starts_with('5') && groups[3].starts_with(['8', '9', 'a', 'b']), "{serial}");
    // RFC 4122's own version 5 of the URL namespace and the name (Python's `uuid.uuid5` agrees).
    assert_eq!(alforge::components::serial("2.0.0", "0123456789abcdef0123456789abcdef01234567"), "urn:uuid:f517fd32-73ff-5a08-a3d3-aca088a26325");

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
    // A flag where the commit goes is a usage error, never a commit's name.
    let out = run(&["components", "--release"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(&["components", "--release", "2.0.0"]);
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
    let purls: Vec<&Value> = listed.iter().filter(|c| c["purl"].as_str().is_some_and(|p| p.starts_with("pkg:cargo/"))).copied().collect();
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
    // Qt and KDDockWidgets at the recipe's versions, which the release carries for Oficina's window
    // (Q446), each with its licence expression; the release depends on them.
    let recipe = alforge::machine::Recipe::load_file(&root.join("tools/alforge/maquina/maquina.toml")).unwrap();
    let q = recipe.builder.qt.unwrap();
    let qt = listed.iter().find(|c| c["name"] == "qt").unwrap();
    assert_eq!((qt["version"].as_str(), qt["purl"].as_str()), (Some(q.version.as_str()), Some(format!("pkg:generic/qt@{}", q.version).as_str())));
    // Its CPE in CycloneDX's own field, as advisories name the product.
    assert_eq!(qt["cpe"], format!("cpe:2.3:a:qt:qt:{}:*:*:*:*:*:*:*", q.version));
    // What only another platform, a test's dependency or a weak feature nothing turns on would
    // reach is not listed (cargo's feature resolver: `arrow-array`'s wasm32-only `ahash` feature
    // reaches `const-random`, `rust_decimal`'s `borsh?/std` reaches no `borsh`).
    for gone in ["const-random", "tiny-keccak", "borsh", "quinn"] {
        assert!(!listed.iter().any(|c| c["name"] == gone), "{gone} is compiled by no build of the target");
    }
    assert_eq!(qt["licenses"], serde_json::json!([{ "expression": "LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only" }]));
    let kdd = listed.iter().find(|c| c["name"] == "kddockwidgets").unwrap();
    assert_eq!((kdd["version"].as_str(), kdd["licenses"].clone()), (Some(q.kddockwidgets.as_str()), serde_json::json!([{ "expression": "GPL-2.0-only OR GPL-3.0-only" }])));
    let release = doc["dependencies"].as_array().unwrap().iter().find(|d| d["ref"] == format!("veredas@{head}")).unwrap();
    assert_eq!(release["dependsOn"], serde_json::json!([format!("pkg:generic/kddockwidgets@{}", q.kddockwidgets), format!("pkg:generic/qt@{}", q.version)]));
    // Ábaco uses the IronCalc fork kept in the tree, by path.
    assert!(listed.iter().any(|c| c["properties"].as_array().is_some_and(|p| p.iter().any(|x| x["value"] == "forks/ironcalc/base"))));
    // Every package cargo's own build of the components compiles for the target, and every edge
    // between them, and nothing else: the list is cargo's unit graph, package by package.
    let tmp = alforge::cache::dir().unwrap().join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let tree = tempfile::Builder::new().prefix("alforge-units-").tempdir_in(&tmp).unwrap();
    alforge::git::export_tree(&root, &head, &[], tree.path()).unwrap();
    let names: Vec<&str> = held.values().map(|h| h.name.as_str()).collect();
    let (built, edges) = cargo_builds(tree.path(), &names, &[]);
    let (packages, dependencies) = self::listed(&doc);
    assert!(built.len() > 300, "{}", built.len());
    assert_eq!(packages.symmetric_difference(&built).collect::<Vec<_>>(), Vec::<&Package>::new(), "the list against cargo's unit graph");
    assert_eq!(dependencies.symmetric_difference(&edges).collect::<Vec<_>>(), Vec::<&(Package, Package)>::new(), "the list's edges against cargo's unit graph");
}

/// The list follows what cargo's own builds compile (its feature resolver, normal and build edges,
/// for the release's target): a dependency of another platform that is also a test's, and a
/// feature a test's dependency alone turns on, list nothing (round 1's forms); and a commit holding
/// no lock file for the crates it uses is refused, as C-290 and the cut refuse it.
#[test]
fn what_no_build_of_the_target_compiles_is_not_listed_and_an_unpinned_crate_refused() {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\n");
    r.write(
        "crates/veredas-core/Cargo.toml",
        "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n\
         [dependencies]\nserde = { version = \"1\", default-features = false }\n\n[target.'cfg(windows)'.dependencies]\nitoa = \"1\"\n\n\
         [dev-dependencies]\nitoa = \"1\"\nserde = { version = \"1\", features = [\"derive\"] }\n",
    );
    r.write("crates/veredas-core/src/lib.rs", "\n");
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let c = r.commit("a test's and another platform's dependencies");
    let doc: Value = serde_json::from_str(&alforge::components::list(r.path(), &c, None).unwrap()).unwrap();
    let names: Vec<&str> = doc["components"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"serde") && names.contains(&"veredas-core"), "{names:?}");
    for gone in ["itoa", "serde_derive", "syn", "quote", "proc-macro2"] {
        assert!(!names.contains(&gone), "{gone}: {names:?}");
    }
    let core = doc["dependencies"].as_array().unwrap().iter().find(|d| d["ref"] == "veredas-core@2.1.0").unwrap();
    assert!(core["dependsOn"].as_array().unwrap().iter().all(|d| d.as_str().unwrap().contains("/serde")), "{core}");
    // No lock file to pin its crates: refused, as the cut refuses it, whatever this machine's cache.
    r.git(&["rm", "-q", "Cargo.lock"]);
    let c = r.commit("no lock file");
    let e = alforge::components::list(r.path(), &c, None).unwrap_err();
    assert!(e.to_string().starts_with("veredas-core's build inputs do not read: veredas-core depends on `") && e.to_string().ends_with("` and the commit holds no Cargo.lock to pin it"), "{e}");
}

/// Round 2's real forms: what `cargo tree` prints and cargo never builds (a build-dependency of a
/// package with no build script, indoc's `rustversion`; a member's build-dependency with no
/// `build.rs`; a dependency on a package with no library) is not listed; an in-tree proc-macro
/// beside its crates.io namesake is told apart (`name vX (proc-macro) (path)`); a signed commit
/// shown under `log.showSignature` gives its time alone; a lock of format 1 gives its checksums.
#[test]
fn what_cargo_prints_and_never_builds_a_proc_macro_twin_a_signed_commit_and_a_lock_of_format_one() {
    let cargo = || Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\nexclude = [\"forks\"]\n");
    r.write(
        "crates/veredas-core/Cargo.toml",
        "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n\
         [dependencies]\nindoc = \"=2.0.8\"\nitoa = \"=1.0.19\"\nitoa_twin = { package = \"itoa\", path = \"../../forks/itoa (twin)\" }\nferramenta = { path = \"../../forks/ferramenta\" }\n\n\
         [build-dependencies]\nryu = \"1\"\n",
    );
    r.write("crates/veredas-core/src/lib.rs", "\n");
    // The twin's folder named with parentheses of its own (round 4's form).
    r.write("forks/itoa (twin)/Cargo.toml", "[workspace]\n\n[package]\nname = \"itoa\"\nversion = \"1.0.19\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[lib]\nproc-macro = true\n");
    r.write("forks/itoa (twin)/src/lib.rs", "\n");
    r.write("forks/ferramenta/Cargo.toml", "[workspace]\n\n[package]\nname = \"ferramenta\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[[bin]]\nname = \"ferramenta\"\npath = \"src/main.rs\"\n");
    r.write("forks/ferramenta/src/main.rs", "fn main() {}\n");
    let out = cargo().args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // Signed, and read where `log.showSignature` is on.
    let key = r.path().join("../signing-key");
    let _ = std::fs::remove_file(&key);
    let made = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    r.git(&["config", "gpg.format", "ssh"]);
    r.git(&["config", "user.signingkey", &format!("{}.pub", key.display())]);
    r.git(&["config", "log.showSignature", "true"]);
    r.git(&["add", "-A"]);
    r.git(&["-c", "commit.gpgsign=true", "commit", "-q", "-m", "a signed commit"]);
    let c = r.git(&["rev-parse", "HEAD"]);
    let doc: Value = serde_json::from_str(&alforge::components::list(r.path(), &c, None).unwrap()).unwrap();
    assert_eq!(doc["metadata"]["timestamp"], "2026-11-26T11:00:00Z");
    let entries = doc["components"].as_array().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    for gone in ["rustversion", "ryu", "ferramenta"] {
        assert!(!names.contains(&gone), "{gone}: {names:?}");
    }
    // The twin by path and the crate of the registry, each its own entry.
    assert!(entries.iter().any(|e| e["bom-ref"] == "itoa@1.0.19") && entries.iter().any(|e| e["bom-ref"] == "pkg:cargo/itoa@1.0.19"), "{names:?}");
    let _ = std::fs::remove_file(&key);
    let _ = std::fs::remove_file(format!("{}.pub", key.display()));

    // A lock of format 1, as cargo writes one for a `rust-version` below 1.41: its checksums kept in
    // `[metadata]`, read as the lock's.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"2\"\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2018\"\nrust-version = \"1.40\"\nlicense = \"MIT\"\n\n[dependencies]\nitoa = \"=1.0.19\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    let out = cargo().args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let lock = std::fs::read_to_string(r.path().join("Cargo.lock")).unwrap();
    assert!(lock.contains("[metadata]") && lock.contains("\"checksum itoa 1.0.19 (registry+"), "a lock of format 1: {lock}");
    let c = r.commit("a lock of format 1");
    let doc: Value = serde_json::from_str(&alforge::components::list(r.path(), &c, None).unwrap()).unwrap();
    let itoa = doc["components"].as_array().unwrap().iter().find(|e| e["bom-ref"] == "pkg:cargo/itoa@1.0.19").unwrap();
    let sum = lock.lines().find(|l| l.starts_with("\"checksum itoa 1.0.19")).unwrap().rsplit(" = ").next().unwrap().trim_matches('"');
    assert_eq!(itoa["hashes"], serde_json::json!([{ "alg": "SHA-256", "content": sum }]));
}

/// What lies below a dependency no build compiles is not listed for it, and what a build does
/// compile is listed even where `cargo tree` would print it first below such a dependency (round
/// 3's forms): a member's build-dependency with no build script asking serde's `derive`, serde used
/// without it; a dependency on a fork with no library that uses serde too. Two git sources of one
/// repository (its default branch, and `branch = "main"`), at one commit, are told apart and make
/// one entry, the same code. Each list is held to cargo's unit graph.
#[test]
fn what_lies_below_what_no_build_compiles_and_two_sources_of_one_repository() {
    let cargo = || Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    let core = |deps: &str| format!("[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n{deps}");
    let serde = |r: &Repo| -> (Value, String) {
        let out = cargo().args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let c = r.commit("serde");
        (serde_json::from_str(&alforge::components::list(r.path(), &c, None).unwrap()).unwrap(), c)
    };
    let names = |doc: &Value| -> Vec<String> { doc["components"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect() };

    // A build-dependency of a package with no build script: none of what lies below it.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", &core("[dependencies]\nserde = { version = \"=1.0.228\", default-features = false }\n\n[build-dependencies]\nserde = { version = \"=1.0.228\", features = [\"derive\"] }\n"));
    r.write("crates/veredas-core/src/lib.rs", "\n");
    let (doc, _) = serde(&r);
    assert_eq!(names(&doc), ["serde", "serde_core", "veredas-core"]);
    assert_eq!(entry(&doc, "pkg:cargo/serde@1.0.228")["name"], "serde");
    let serde_deps = doc["dependencies"].as_array().unwrap().iter().find(|d| d["ref"] == "pkg:cargo/serde@1.0.228").unwrap();
    assert_eq!(serde_deps["dependsOn"], serde_json::json!(["pkg:cargo/serde_core@1.0.228"]));
    assert_eq!(listed(&doc), cargo_builds(r.path(), &["veredas-core"], &[]));

    // A dependency on a fork with no library, which shares serde with the component: serde's own
    // dependencies listed all the same.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\nexclude = [\"forks\"]\n");
    r.write("crates/veredas-core/Cargo.toml", &core("[dependencies]\nferramenta = { path = \"../../forks/ferramenta\" }\nserde = \"=1.0.228\"\n"));
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("forks/ferramenta/Cargo.toml", "[workspace]\n\n[package]\nname = \"ferramenta\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\nserde = \"=1.0.228\"\n");
    r.write("forks/ferramenta/src/main.rs", "fn main() {}\n");
    let (doc, _) = serde(&r);
    assert_eq!(names(&doc), ["serde", "serde_core", "veredas-core"]);
    assert_eq!(listed(&doc), cargo_builds(r.path(), &["veredas-core"], &[]));

    // Two sources of one git repository at one commit, fetched from a folder of the machine into a
    // cargo home of the test's own.
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("cargo-home");
    let dep = Repo::new();
    dep.write("Cargo.toml", "[package]\nname = \"gitdep\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n");
    dep.write("src/lib.rs", "\n");
    let at = dep.commit("a crate of git");
    // A branch whose name cargo prints URL-encoded (`branch=fix%2Fx`), at the same commit.
    dep.git(&["branch", "fix/x"]);
    let url = format!("file://{}", dep.path().display());
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\nexclude = [\"forks\"]\n");
    r.write("crates/veredas-core/Cargo.toml", &core(&format!("[dependencies]\ngitdep = {{ git = \"{url}\" }}\ngarfo = {{ path = \"../../forks/garfo\" }}\nforquilha = {{ path = \"../../forks/forquilha\" }}\n")));
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("forks/garfo/Cargo.toml", &format!("[workspace]\n\n[package]\nname = \"garfo\"\nversion = \"0.3.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\ngitdep = {{ git = \"{url}\", branch = \"main\" }}\n"));
    r.write("forks/garfo/src/lib.rs", "\n");
    r.write("forks/forquilha/Cargo.toml", &format!("[workspace]\n\n[package]\nname = \"forquilha\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\ngitdep = {{ git = \"{url}\", branch = \"fix/x\" }}\n"));
    r.write("forks/forquilha/src/lib.rs", "\n");
    let out = cargo().args(["generate-lockfile"]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let lock = std::fs::read_to_string(r.path().join("Cargo.lock")).unwrap();
    assert!(lock.contains(&format!("git+{url}#{at}")) && lock.contains(&format!("git+{url}?branch=main#{at}")) && lock.contains(&format!("git+{url}?branch=fix%2Fx#{at}")), "{lock}");
    let c = r.commit("two sources of one repository");
    let run = Command::new(env!("CARGO_BIN_EXE_alforge")).args(["components", &c]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let doc: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(names(&doc), ["forquilha", "garfo", "gitdep", "veredas-core"]);
    let purl = format!("pkg:cargo/gitdep@0.1.0?vcs_url={}", url_encoded(&format!("git+{url}@{at}")));
    assert_eq!(entry(&doc, &purl)["purl"], purl);
    assert_eq!(listed(&doc), cargo_builds(r.path(), &["veredas-core"], &[("CARGO_HOME", &home)]));
}

/// Every byte percent-encoded but the unreserved ones, as a package URL's qualifier.
fn url_encoded(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A cargo configuration of the caller's that would steer the graph (`build.rustflags` holding a
/// `--cfg`, as rustix documents `rustix_use_libc`) steers no list (round 4's form): the builder's
/// build runs with none. The configuration does steer cargo's own tree, so the check can fail.
#[test]
fn the_callers_cargo_configuration_steers_no_list() {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\nrustix = { version = \"=0.38.44\", features = [\"fs\"] }\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    r.commit("rustix");
    // A cargo home of the test's own: the machine's registry, and the setting.
    let real = std::env::var_os("CARGO_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap()).join(".cargo"));
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("cargo-home");
    std::fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(real.join("registry"), home.join("registry")).unwrap();
    std::fs::write(home.join("config.toml"), "[build]\nrustflags = [\"--cfg\", \"rustix_use_libc\"]\n").unwrap();
    let list = |home: Option<&Path>| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_alforge"));
        c.args(["components", "HEAD", "--release", "2.0.0"]).current_dir(r.path());
        if let Some(h) = home {
            c.env("CARGO_HOME", h);
        }
        let out = c.output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    let plain = list(None);
    assert_eq!(String::from_utf8(list(Some(&home))).unwrap(), String::from_utf8(plain.clone()).unwrap());
    let doc: Value = serde_json::from_slice(&plain).unwrap();
    let names: Vec<&str> = doc["components"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"linux-raw-sys") && !names.contains(&"errno"), "{names:?}");
    // The setting steers cargo's own tree: rustix over libc, through errno.
    let tree = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["tree", "--offline", "--locked", "--target", alforge::components::TARGET, "-e", "normal", "--prefix", "none", "--format", "{p}"])
        .env("CARGO_HOME", &home)
        .current_dir(r.path())
        .output()
        .unwrap();
    assert!(tree.status.success(), "{}", String::from_utf8_lossy(&tree.stderr));
    assert!(String::from_utf8_lossy(&tree.stdout).lines().any(|l| l.starts_with("errno ")), "{}", String::from_utf8_lossy(&tree.stdout));
}

/// The machine's cargo home, whose registry a test's own cargo home links to.
fn machine_cargo_home() -> std::path::PathBuf {
    std::env::var_os("CARGO_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap()).join(".cargo"))
}

/// A cargo home of a test's own at `home`, crates.io's index, downloads and sources linked to the
/// machine's (nothing else of the machine's written through it), `config` its configuration.
fn own_cargo_home(home: &Path, config: &str) {
    let real = machine_cargo_home().join("registry");
    for part in ["index", "cache", "src"] {
        let from = std::fs::read_dir(real.join(part)).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("index.crates.io-")).unwrap();
        std::fs::create_dir_all(home.join("registry").join(part)).unwrap();
        std::os::unix::fs::symlink(&from, home.join("registry").join(part).join(from.file_name().unwrap())).unwrap();
    }
    std::fs::write(home.join("config.toml"), config).unwrap();
}

/// Round 5's forms: a `paths` override of the caller's cargo configuration, which reads a locked
/// crate from outside the commit's tree, refused; a component named like a package by path of its
/// graph at another version, in a folder whose name ends in ` (*`, listed; a `:/<text>` name of
/// the commit taken as git takes it.
#[test]
fn a_paths_override_a_component_s_namesake_in_a_starred_folder_and_a_commit_named_by_its_message() {
    let cargo = || Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\nexclude = [\"forks\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\nitoa = \"=1.0.19\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\nveredas-core = { path = \"../veredas-core\" }\ncore_v1 = { package = \"veredas-core\", path = \"../../forks/core (*\" }\n",
    );
    r.write("crates/veredas-portal/src/main.rs", "fn main() {}\n");
    r.write("forks/core (*/Cargo.toml", "[workspace]\n\n[package]\nname = \"veredas-core\"\nversion = \"1.4.0\"\nedition = \"2021\"\nlicense = \"MIT\"\n");
    r.write("forks/core (*/src/lib.rs", "\n");
    let out = cargo().args(["generate-lockfile", "--offline"]).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let c = r.commit("an old core beside the new");
    let run = |name: &str, home: Option<&Path>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_alforge"));
        cmd.args(["components", name]).current_dir(r.path());
        if let Some(h) = home {
            cmd.env("CARGO_HOME", h);
        }
        cmd.output().unwrap()
    };
    let out = run(&c, None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = |r: &str| entry(&doc, r)["properties"].as_array().unwrap().iter().find(|p| p["name"] == "veredas:path").map(|p| p["value"].clone());
    assert_eq!(path("veredas-core@2.1.0"), Some(serde_json::json!("crates/veredas-core")));
    assert_eq!(path("veredas-core@1.4.0"), Some(serde_json::json!("forks/core (*")));
    let portal = doc["dependencies"].as_array().unwrap().iter().find(|d| d["ref"] == "veredas-portal@2.0.0").unwrap();
    assert_eq!(portal["dependsOn"], serde_json::json!(["veredas-core@1.4.0", "veredas-core@2.1.0"]));
    // The commit by a `:/<text>` name: the same document.
    let by_message = run(":/an old core", None);
    assert!(by_message.status.success(), "{}", String::from_utf8_lossy(&by_message.stderr));
    assert_eq!(by_message.stdout, out.stdout);

    // A `paths` override of the caller's configuration, an unchanged copy of the locked crate
    // outside the tree: cargo reads it there, which the builder never does, so the list is refused.
    let d = tempfile::tempdir().unwrap();
    let copy = d.path().join("override/itoa");
    let src = std::fs::read_dir(machine_cargo_home().join("registry/src")).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("index.crates.io-")).unwrap().join("itoa-1.0.19");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    assert!(Command::new("cp").arg("-r").arg(&src).arg(&copy).status().unwrap().success());
    let home = d.path().join("cargo-home");
    own_cargo_home(&home, &format!("paths = [\"{}\"]\n", copy.display()));
    let over = run(&c, Some(&home));
    assert_eq!(over.status.code(), Some(2), "{}", String::from_utf8_lossy(&over.stdout));
    let why = String::from_utf8_lossy(&over.stderr);
    assert!(why.contains(&format!("cargo reads itoa 1.0.19 from {}, outside the commit's tree", copy.display())), "{why}");
    // The same home without the override: the same document as the machine's.
    std::fs::write(home.join("config.toml"), "").unwrap();
    let plain = run(&c, Some(&home));
    assert!(plain.status.success(), "{}", String::from_utf8_lossy(&plain.stderr));
    assert_eq!(plain.stdout, out.stdout);
}

/// A crate of a registry the configuration names (`registry = "outra"`, a `file://` index of the
/// test's own holding crates.io's own itoa 1.0.19) beside its crates.io twin: cargo prints the
/// registry by its name, and each twin is its own entry (round 5's form).
#[test]
fn a_crate_of_a_named_registry_beside_its_crates_io_twin() {
    use sha2::Digest;
    let cargo = || Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    let d = tempfile::tempdir().unwrap();
    let crate_file = machine_cargo_home().join("registry/cache").read_dir().unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("index.crates.io-")).unwrap().join("itoa-1.0.19.crate");
    let bytes = std::fs::read(&crate_file).unwrap();
    let sum: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(d.path().join("dl")).unwrap();
    std::fs::write(d.path().join("dl/itoa-1.0.19.crate"), &bytes).unwrap();
    let index = Repo::new();
    index.write("config.json", &format!("{{\"dl\":\"file://{}/dl/{{crate}}-{{version}}.crate\",\"api\":null}}", d.path().display()));
    index.write("it/oa/itoa", &format!("{{\"name\":\"itoa\",\"vers\":\"1.0.19\",\"deps\":[{{\"name\":\"no-panic\",\"req\":\"^0.1\",\"features\":[],\"optional\":true,\"default_features\":true,\"target\":null,\"kind\":\"normal\"}}],\"cksum\":\"{sum}\",\"features\":{{}},\"yanked\":false}}\n"));
    index.commit("an index of one crate");
    let index_url = format!("file://{}", index.path().display());
    let home = d.path().join("cargo-home");
    own_cargo_home(&home, &format!("[registries.outra]\nindex = \"{index_url}\"\n"));
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/veredas-core\"]\n");
    let core = |deps: &str| format!("[package]\nname = \"veredas-core\"\nversion = \"2.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n\n[dependencies]\n{deps}");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    // The other registry's crate fetched first (its index and download by `file://` alone), then
    // crates.io's twin locked offline.
    r.write("crates/veredas-core/Cargo.toml", &core("itoa_outra = { package = \"itoa\", version = \"=1.0.19\", registry = \"outra\" }\n"));
    let out = cargo().args(["fetch"]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    r.write("crates/veredas-core/Cargo.toml", &core("itoa = \"=1.0.19\"\nitoa_outra = { package = \"itoa\", version = \"=1.0.19\", registry = \"outra\" }\n"));
    let out = cargo().args(["generate-lockfile", "--offline"]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let c = r.commit("twins of two registries");
    let tree = cargo().args(["tree", "--offline", "--locked", "--prefix", "none", "--format", "{p}"]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(String::from_utf8_lossy(&tree.stdout).lines().any(|l| l == "itoa v1.0.19 (registry `outra`)"), "{}", String::from_utf8_lossy(&tree.stdout));
    let run = Command::new(env!("CARGO_BIN_EXE_alforge")).args(["components", &c]).env("CARGO_HOME", &home).current_dir(r.path()).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let doc: Value = serde_json::from_slice(&run.stdout).unwrap();
    let outra = format!("pkg:cargo/itoa@1.0.19?repository_url={}", url_encoded(&index_url));
    for r in ["pkg:cargo/itoa@1.0.19", outra.as_str()] {
        assert_eq!(entry(&doc, r)["hashes"][0]["content"], sum, "{r}");
    }
    let core_deps = doc["dependencies"].as_array().unwrap().iter().find(|d| d["ref"] == "veredas-core@2.1.0").unwrap();
    assert_eq!(core_deps["dependsOn"].as_array().unwrap().len(), 2);
}
