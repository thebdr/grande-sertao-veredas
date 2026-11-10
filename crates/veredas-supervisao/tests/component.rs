//! C-047: the component. A library and one executable named for it, no build script; its own
//! dependencies core, portal and the server's crates, no plant stack of its own; no device, vendor,
//! product or customer named in any file git holds or could hold under the crate, in its text or
//! its name, the built page included; the page compiled in from a `dist/` built from the sources in
//! the tree: every file under the crate is Rust, a hashed source of the page or the page itself, the
//! page is the three files the host serves, and both of the build's hashes are recomputed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use veredas_supervisao::web;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn metadata() -> Value {
    let out = Command::new(env!("CARGO")).args(["metadata", "--no-deps", "--format-version", "1", "--manifest-path"]).arg(crate_dir().join("Cargo.toml")).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn package(meta: &Value) -> &Value {
    meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == "veredas-supervisao").unwrap()
}

#[test]
fn supervisao_is_one_library_and_one_executable_named_for_it() {
    let meta = metadata();
    let p = package(&meta);
    assert_eq!(p["version"], "2.0.0-alpha.1");
    let mut targets: Vec<(String, Vec<String>)> = p["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_string(), t["kind"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect()))
        .collect();
    targets.sort();
    let non_test: Vec<&(String, Vec<String>)> = targets.iter().filter(|(_, k)| k != &vec!["test".to_string()]).collect();
    assert_eq!(non_test, [&("veredas-supervisao".to_string(), vec!["bin".to_string()]), &("veredas_supervisao".to_string(), vec!["lib".to_string()])]);
    let tests: Vec<&str> = targets.iter().filter(|(_, k)| k == &vec!["test".to_string()]).map(|(n, _)| n.as_str()).collect();
    assert_eq!(tests, ["component", "dashboards", "feed", "files", "host", "layout", "tables", "writes"], "one test file per clause");
    assert!(!crate_dir().join("build.rs").exists());
}

/// Crates a supervision app has no business holding itself: a plant's stack is portal's.
const BANNED_DIRECT: &[&str] = &["async-opcua-client", "async-opcua-server", "async-opcua-types", "duckdb", "libduckdb-sys", "socket2", "reqwest", "hyper", "hyper-util", "veredas-maquinar", "veredas-aranha"];

#[test]
fn its_own_dependencies_are_core_portal_and_the_servers_crates() {
    let meta = metadata();
    let p = package(&meta);
    let normal: BTreeSet<&str> = p["dependencies"].as_array().unwrap().iter().filter(|d| d["kind"].is_null()).map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(normal, ["axum", "chrono", "roxmltree", "serde", "serde_json", "thiserror", "tokio", "tokio-stream", "toml", "veredas-core", "veredas-portal"].into_iter().collect::<BTreeSet<_>>());
    assert!(normal.iter().all(|n| !BANNED_DIRECT.contains(n)));
    let build: Vec<&Value> = p["dependencies"].as_array().unwrap().iter().filter(|d| d["kind"] == "build").collect();
    assert!(build.is_empty());
    // The bench is a fixture: a dev-dependency, outside the map.
    let dev: BTreeSet<&str> = p["dependencies"].as_array().unwrap().iter().filter(|d| d["kind"] == "dev").map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(dev, ["tempfile", "veredas-maquinar"].into_iter().collect::<BTreeSet<_>>());
}

/// The crates in a package's normal and build graph on every platform, by name, as cargo would
/// build that package alone.
fn graph(package: &str) -> BTreeSet<String> {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "-e", "normal,build", "--target", "all", "--prefix", "none", "-f", "{p}", "-p", package, "--manifest-path"])
        .arg(crate_dir().join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
}

/// A plant's stack: what talks to a plant or keeps its data. Portal's to hold, never supervisão's.
const PLANT_STACK: &[&str] = &["async-opcua-client", "async-opcua-server", "async-opcua-types", "async-opcua-core", "async-opcua-crypto", "duckdb", "libduckdb-sys", "socket2", "reqwest", "veredas-maquinar", "veredas-aranha"];

#[test]
fn the_plant_s_stack_reaches_supervisao_through_portal_alone() {
    let mine = graph("veredas-supervisao");
    let portal = graph("veredas-portal");
    assert!(mine.contains("veredas-portal") && mine.contains("veredas-core") && mine.contains("axum"), "{mine:?}");
    // The engine and the OPC client are there, and portal brings them.
    for c in ["duckdb", "libduckdb-sys", "async-opcua-client"] {
        assert!(mine.contains(c) && portal.contains(c), "{c}");
    }
    let own: Vec<&String> = mine.difference(&portal).filter(|c| PLANT_STACK.contains(&c.as_str())).collect();
    assert_eq!(own, Vec::<&String>::new(), "a plant's crate supervisão brings itself");
    assert!(!mine.contains("veredas-maquinar") && !mine.contains("veredas-aranha"), "the bench is a dev-dependency alone");
}

/// Every file git holds or could hold under the crate (tracked, and untracked files git does not
/// ignore): what would be committed is what is judged, `web/dist/` included, `web/node_modules/`
/// never (the repository ignores it).
fn crate_files() -> Vec<PathBuf> {
    let root = crate_dir();
    let out = Command::new("git").arg("-C").arg(&root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "."]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut files: Vec<PathBuf> = out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| root.join(String::from_utf8_lossy(s).as_ref())).filter(|p| p.is_file()).collect();
    files.sort();
    files.dedup();
    files
}

fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// Where a text names a device, vendor, product or customer the suite keeps out of its code, or
/// cites a specification: the whole text lowercased, its accented Latin letters read as their base
/// letters (precomposed or decomposed alike) and stripped to letters and digits, so no separator,
/// case, accent or line break hides a name; each hit names the line where it starts. The
/// names are written backwards here, so no line of this file spells one: portal's (the protocol
/// core's telegram layer came from, its terms, its document folder) and those of Veredas 1's
/// supervision app (the exports it parsed, their tables, the plants it was fitted to).
fn mentions(text: &str) -> Vec<String> {
    let banned: Vec<String> = [
        "tlebineg", "rodnevscod", "lortnoctrosretsam", "ecafretnitsoh", "maootxtr", "xtrotmao",
        "wardatad", "seulavbd", "stsilmrala", "tnalpgifnoc", "sysroyevnoc", "etalpmetretros", "setalpmetenil",
        "erawtfosarutappam", "elarenegmob", "sreirractupni", "gifnocniart", "ecneirepxed3", "inoizurtsi",
        "03103morfvsc", "rotarugifnoctrams", "hcramretni", "goldaj", "noitisopfin", "thgirenilllec", "elgnagnidaol",
        "txenhtiwelgna", "ditlebtxen", "ditlebdecafretni", "rotacolmeti", "lrtcedalb", "liocrotaludom",
        "sretemarapenil", "stnenopmocenil", "noitavitcasllec", "ecafretnimetsys", "lrtcgnigrem", "smarapbd", "gaidbd",
        "puorgsys", "03103koobkrow", "sgatcpopmud", "latemodrodatnacne", "oivanodoatipac",
    ]
    .iter()
    .map(|b| rev(b))
    .collect();
    let mut flat = Vec::new();
    let mut line_of = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for c in line.chars().flat_map(char::to_lowercase).map(base_letter).filter(|c| c.is_alphanumeric() && !('\u{300}'..='\u{36f}').contains(c)) {
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
    let section = char::from_u32(0xA7).unwrap();
    for (n, line) in text.lines().enumerate() {
        if line.contains(section) {
            hits.push(format!("line {}: section sign", n + 1));
        }
    }
    hits
}

/// A lowercase Latin-1 letter with an accent read as its base letter; any other character as is.
fn base_letter(c: char) -> char {
    match c {
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' | 'ø' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        c => c,
    }
}

#[test]
fn no_file_of_supervisao_names_a_vendor_a_product_or_a_customer() {
    let files = crate_files();
    let rel: Vec<String> = files.iter().map(|f| f.strip_prefix(crate_dir()).unwrap().to_string_lossy().replace('\\', "/")).collect();
    for must in ["Cargo.toml", "src/lib.rs", "tests/component.rs", "web/src/main.ts", "web/dist/assets/app.js", "web/dist/assets/app.css", "web/dist/index.html", "web/dist/source.hash", "web/package-lock.json"] {
        assert!(rel.iter().any(|r| r == must), "{must} is judged: {rel:?}");
    }
    assert!(rel.iter().all(|r| !r.contains("node_modules")), "{rel:?}");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| mentions(&String::from_utf8_lossy(&std::fs::read(f).unwrap())).into_iter().map(move |l| format!("{}: {l}", f.display())))
        .collect();
    assert_eq!(found, Vec::<String>::new());
    // Nor does a file's name, read from the crate's folder down.
    let named: Vec<String> = rel.iter().flat_map(|r| mentions(r).into_iter().map(move |l| format!("{r}: {l}"))).collect();
    assert_eq!(named, Vec::<String>::new());
    assert_eq!(mentions(&format!("tests/data/{}-export.txt", rev("tlebineg"))).len(), 1, "a name in a path is a name");
    // The reader sees every spelling, in any case, across line breaks; each sample holds one.
    let samples = [
        format!("// the {} catalog", rev("tleBineG")),
        format!("const X: &str = \"{}\";", rev("seulaVBD")),
        format!("// read the {}", rev("syS-royevnoC")),
        format!("// {}\n// {} sheet", rev("arutappaM"), rev("erawtfoS")),
        format!("// a plant of {}", rev("éhcramretnI")),
        format!("<!-- {} -->", rev("YTITNAUQ_ROTACOLMETI")),
        format!("// spec {}3.2", char::from_u32(0xA7).unwrap()),
        // The accented term, precomposed, capitalised, unaccented and decomposed.
        format!("// {}", rev("oivan od oãtipac")),
        format!("// {}", rev("OIVAN OD OÃTIPAC")),
        format!("// {}", rev("oivaN od oatipaC")),
        format!("// {}", rev("oivan od o\u{303}atipac")),
    ];
    for s in &samples {
        assert_eq!(mentions(s).len(), 1, "{s}: {:?}", mentions(s));
    }
    assert_eq!(mentions(&format!("x\n// {}\n// {}", rev("erawtfoS arutappaM"), rev("elareneG moB"))), vec![format!("line 2: {}", rev("erawtfosarutappam")), format!("line 3: {}", rev("elarenegmob"))]);
    assert_eq!(mentions("// a loop with three outlets, two parcels and an infeed\nlet sorts = 1;"), Vec::<String>::new());
}

#[test]
fn the_page_compiled_in_is_built_from_the_sources_in_the_tree() {
    let webdir = crate_dir().join("web");
    let built = web::BUILT_FROM.trim();
    assert_eq!(built.len(), 16, "{built}");
    assert_eq!(web::source_hash(&webdir).unwrap(), built, "web/dist was built from other sources: run `npm run build` in web/");
    let sources = web::sources(&webdir).unwrap();
    for s in ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts", "src/main.ts", "src/style.css"] {
        assert!(sources.iter().any(|x| x == s), "{s} in {sources:?}");
    }
    assert!(sources.iter().all(|s| !s.starts_with("dist/") && !s.contains("node_modules")));
    // The page compiled in is the page the build wrote: a hand edit is refused.
    let written = web::BUILT_AS.trim();
    assert_eq!(written.len(), 16, "{written}");
    assert_eq!(web::output_hash(), written, "web/dist was edited after its build: run `npm run build` in web/");
    let edited = format!("{};", web::APP_JS);
    assert_ne!(web::hash_texts([("assets/app.css", web::APP_CSS), ("assets/app.js", edited.as_str()), ("index.html", web::INDEX_HTML)]), written, "one byte of the page is a change");

    // Every file git holds or could hold under the crate is Rust, a hashed source of the page or
    // the page itself; so a new input the build could read either joins the hashed set or is
    // refused here.
    let rel: Vec<String> = crate_files().iter().map(|f| f.strip_prefix(crate_dir()).unwrap().to_string_lossy().replace('\\', "/")).collect();
    let dist: BTreeSet<&str> = rel.iter().filter_map(|r| r.strip_prefix("web/dist/")).collect();
    let page: BTreeSet<&str> = web::OUTPUT_FILES.into_iter().chain(["source.hash", "output.hash"]).collect();
    assert_eq!(dist, page, "the page is the three files the host serves and the build's two hashes");
    let stray: Vec<&String> = rel
        .iter()
        .filter(|r| {
            let rust = r.as_str() == "Cargo.toml" || ((r.starts_with("src/") || r.starts_with("tests/")) && r.ends_with(".rs"));
            let source = r.strip_prefix("web/").is_some_and(|w| sources.iter().any(|s| s == w));
            !(rust || source || r.starts_with("web/dist/"))
        })
        .collect();
    assert_eq!(stray, Vec::<&String>::new(), "a file outside the crate's code, the page's hashed sources and the page");
    // The build reads nothing beside them and emits nothing beside the page: the guards stand in
    // its config (their refusals are C-047's evidence), and the hashes are written once the bundle
    // is on disk.
    let config = std::fs::read_to_string(webdir.join("vite.config.ts")).unwrap();
    for guard in ["envDir: false", "publicDir: false", "css: { postcss: {} }", "enforce: \"post\"", "this.getModuleIds()", "writeBundle()"] {
        assert!(config.contains(guard), "vite.config.ts lacks `{guard}`");
    }
    assert!(!config.contains("closeBundle"), "the hashes are written after a written bundle alone");
    // The bundle's names are the ones the host serves, and it loads no other file.
    assert!(web::INDEX_HTML.contains("src=\"/assets/app.js\"") && web::INDEX_HTML.contains("href=\"/assets/app.css\""), "{}", web::INDEX_HTML);
    assert!(!web::APP_JS.is_empty() && !web::APP_CSS.is_empty());
    assert!(!web::APP_JS.contains("import(") && !web::APP_JS.contains("import.meta.url"), "the bundle loads no other file");
    // The hash sees one byte of any source change, and not a line ending.
    let copy = tempfile::tempdir().unwrap();
    copy_tree(&webdir, copy.path(), &sources);
    assert_eq!(web::source_hash(copy.path()).unwrap(), built);
    let main = copy.path().join("src/main.ts");
    let text = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, text.replace("\r\n", "\n").replace('\n', "\r\n")).unwrap();
    assert_eq!(web::source_hash(copy.path()).unwrap(), built, "CRLF reads as LF");
    std::fs::write(&main, format!("{text} ")).unwrap();
    assert_ne!(web::source_hash(copy.path()).unwrap(), built);
    std::fs::write(&main, &text).unwrap();
    std::fs::write(copy.path().join("src/extra.ts"), "").unwrap();
    assert_ne!(web::source_hash(copy.path()).unwrap(), built, "a new source is a change");
}

fn copy_tree(from: &Path, to: &Path, rel: &[String]) {
    for r in rel {
        let dst = to.join(r);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(from.join(r), dst).unwrap();
    }
}
