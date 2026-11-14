//! C-047: the component. A library and one executable named for it, no build script; its own
//! dependencies core, portal and the server's crates, no plant stack of its own; no device, vendor,
//! product or customer named in any file git holds or could hold under the crate, in its text or
//! its name, the built page included; the page compiled in from a `dist/` built from the sources in
//! the tree: every file under the crate is Rust, a hashed source of the page or the page itself, the
//! page is the three files the host serves, and both of the build's hashes are recomputed.

use std::collections::{BTreeMap, BTreeSet};
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
    // The bench is a fixture: a dev-dependency, outside the map; the Rust lexer reads the sources
    // for the writes test (C-054); Unicode's decomposition and HTML's named references (WHATWG's
    // table) read every file's letters here.
    let dev: BTreeSet<&str> = p["dependencies"].as_array().unwrap().iter().filter(|d| d["kind"] == "dev").map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(dev, ["entities", "icu_normalizer", "proc-macro2", "tempfile", "veredas-maquinar"].into_iter().collect::<BTreeSet<_>>());
}

/// The crates in a package's normal and build graph on every platform, by name, as cargo would
/// build that package alone.
fn graph(package: &str) -> BTreeSet<String> {
    graph_with(package, &[])
}

/// The same, with cargo tree's own further arguments (a crate's subtree pruned).
fn graph_with(package: &str, more: &[&str]) -> BTreeSet<String> {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "-e", "normal,build", "--target", "all", "--prefix", "none", "-f", "{p}", "-p", package])
        .args(more)
        .arg("--manifest-path")
        .arg(crate_dir().join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
}

#[test]
fn the_plant_s_stack_reaches_supervisao_through_portal_alone() {
    let mine = graph("veredas-supervisao");
    let portal = graph("veredas-portal");
    assert!(mine.contains("veredas-portal") && mine.contains("veredas-core") && mine.contains("axum"), "{mine:?}");
    // The engine and the OPC client are there, and portal brings them.
    for c in ["duckdb", "libduckdb-sys", "async-opcua-client"] {
        assert!(mine.contains(c) && portal.contains(c), "{c}");
    }
    // Nothing beyond portal's graph but the layout's XML reader and supervisão itself: the whole
    // difference cargo reports, so no crate of any stack (a socket, a database, an HTTP client),
    // however it is brought in (a dependency renamed, vendored or patched), joins unseen.
    let own: Vec<&str> = mine.difference(&portal).map(String::as_str).collect();
    assert_eq!(own, ["roxmltree", "veredas-supervisao"], "a crate supervisão brings beyond portal's graph");
    assert!(!mine.contains("veredas-maquinar") && !mine.contains("veredas-aranha"), "the bench is a dev-dependency alone");
    // By its identity: every package of supervisão's normal and build graph, as cargo resolves
    // it, is a member of the workspace or a crate of crates.io; none taken from a path outside the
    // workspace (vendored, patched in), from git or from another registry.
    let resolved = {
        let out = Command::new(env!("CARGO")).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(crate_dir().join("Cargo.toml")).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let members: BTreeSet<&str> = resolved["workspace_members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    let source: std::collections::BTreeMap<&str, Option<&str>> = resolved["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p["source"].as_str())).collect();
    let nodes: std::collections::BTreeMap<&str, &Value> = resolved["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let start = resolved["packages"].as_array().unwrap().iter().find(|p| p["name"] == "veredas-supervisao").and_then(|p| p["id"].as_str()).unwrap();
    assert!(members.contains(start));
    let (mut seen, mut todo) = (BTreeSet::new(), vec![start]);
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        for d in nodes[id]["deps"].as_array().unwrap() {
            if d["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null() || k["kind"] == "build") {
                todo.push(d["pkg"].as_str().unwrap());
            }
        }
    }
    assert!(seen.len() >= mine.len(), "the walk reached {} packages, cargo tree {}", seen.len(), mine.len());
    let foreign: Vec<&&str> = seen.iter().filter(|id| !members.contains(**id) && source[**id] != Some("registry+https://github.com/rust-lang/crates.io-index")).collect();
    assert!(foreign.is_empty(), "a package neither of the workspace nor of crates.io: {foreign:?}");
    // And by its route: each of supervisão's own dependencies but portal, its graph as cargo
    // builds it alone (by its exact version), brings exactly the crates of a plant's stack the
    // table here reviews (core its opc layer's client and socket crate, the server and its runtime
    // their HTTP stack and sockets), none other; and with portal's subtree cut away (`--prune`) supervisão's graph holds
    // no crate of that stack those do not bring. A crate portal's graph also holds joins by no
    // other way unseen.
    let reviewed: std::collections::BTreeMap<&str, Vec<&str>> = [
        ("veredas-core", vec!["async-opcua-client", "async-opcua-types", "socket2"]),
        ("axum", vec!["hyper", "hyper-util", "socket2"]),
        ("tokio", vec!["socket2"]),
        ("tokio-stream", vec!["socket2"]),
    ]
    .into_iter()
    .collect();
    let version: std::collections::BTreeMap<&str, (&str, &str)> = resolved["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), (p["name"].as_str().unwrap(), p["version"].as_str().unwrap()))).collect();
    let mut brought: BTreeSet<&str> = BTreeSet::new();
    for d in nodes[start]["deps"].as_array().unwrap() {
        if !d["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null() || k["kind"] == "build") {
            continue;
        }
        let (name, v) = version[d["pkg"].as_str().unwrap()];
        if name == "veredas-portal" {
            continue;
        }
        let g = graph(&format!("{name}@{v}"));
        let stack: BTreeSet<&str> = BANNED_DIRECT.iter().copied().filter(|c| g.contains(*c)).collect();
        assert_eq!(stack, reviewed.get(name).cloned().unwrap_or_default().into_iter().collect::<BTreeSet<_>>(), "the plant's stack {name} brings");
        brought.extend(stack);
    }
    let pruned = graph_with("veredas-supervisao", &["--prune", "veredas-portal"]);
    assert!(!pruned.contains("veredas-portal") && pruned.contains("axum"), "{pruned:?}");
    let other: Vec<&&str> = BANNED_DIRECT.iter().filter(|c| pruned.contains(**c) && !brought.contains(**c)).collect();
    assert!(other.is_empty(), "a plant's stack reaching supervisão other than through portal or a reviewed dependency: {other:?}");
}

/// Every file git holds or could hold under the crate (tracked, and untracked files git does not
/// ignore): what would be committed is what is judged, `web/dist/` included, `web/node_modules/`
/// never (the repository ignores it).
fn crate_files() -> Vec<PathBuf> {
    let root = crate_dir();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(&root).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    let staged = git(&["ls-files", "-z", "--stage", "--", "."]);
    // Ignored by the repository's own `.gitignore` alone: a rule of this machine's (`.git/info/exclude`,
    // `core.excludesFile`) hides nothing from this reading.
    let others = git(&["ls-files", "-z", "--others", "--exclude-per-directory=.gitignore", "--", "."]);
    let (tracked, untracked) = regular_entries(&staged, &others).unwrap_or_else(|e| panic!("{e}"));
    let mut files = Vec::new();
    for (rel, held) in tracked.iter().map(|r| (r, true)).chain(untracked.iter().map(|r| (r, false))) {
        let p = root.join(rel);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_file() => files.push(p),
            Ok(_) => panic!("`{rel}` is no regular file on disk (a link or a folder): every file git holds or would hold is one"),
            // A tracked file not on disk would be read by no one here: its deletion is committed, or
            // it is restored.
            Err(_) if held => panic!("`{rel}` is held by git and not on disk: commit its deletion or restore it"),
            Err(e) => panic!("`{rel}`: {e}"),
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Git's own list of entries under the crate, refused unless each is a regular file: a tracked
/// entry by its mode (`100644` or `100755`; a link `120000` or a submodule `160000` is none), an
/// untracked one unless it names a file (git lists a nested repository as its folder, `dir/`).
/// Returns the tracked paths and the untracked ones, as git wrote them.
fn regular_entries(staged: &[u8], others: &[u8]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut tracked = Vec::new();
    for e in staged.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let e = String::from_utf8(e.to_vec()).map_err(|_| format!("an entry git holds whose name is not UTF-8 ({}): every name under the crate is text", String::from_utf8_lossy(e)))?;
        let (meta, path) = e.split_once('\t').ok_or_else(|| format!("an entry git wrote without its path: {e}"))?;
        let mode = meta.split(' ').next().unwrap_or("");
        if mode != "100644" && mode != "100755" {
            return Err(format!("`{path}` is held by git as mode {mode}, no regular file: every file git holds is one"));
        }
        tracked.push(path.to_string());
    }
    let mut untracked = Vec::new();
    for e in others.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let path = String::from_utf8(e.to_vec()).map_err(|_| format!("an entry git would hold whose name is not UTF-8 ({}): every name under the crate is text", String::from_utf8_lossy(e)))?;
        if path.ends_with('/') {
            return Err(format!("`{path}` is a repository of its own inside the crate, no file"));
        }
        untracked.push(path);
    }
    Ok((tracked, untracked))
}

#[test]
fn an_address_is_read_by_its_scheme_or_its_slashes() {
    let read = |t: &str| addresses(t).into_iter().collect::<Vec<_>>();
    assert_eq!(read(r#"fetch("https:cdn.example/x.json")"#), ["https:cdn.example/x.json"]);
    assert_eq!(read("new WebSocket('WSS://feed.example/s')"), ["//feed.example/s", "WSS://feed.example/s"]);
    assert_eq!(read("url(//cdn.example/f.woff)"), ["//cdn.example/f.woff"]);
    assert_eq!(read("rows:1,news:2 // a comment"), Vec::<String>::new());
    // Broken by a tab or a line break, as a parser removes them before it reads: still one address.
    assert_eq!(read("fetch(`//
	cdn.example/b.json`)"), ["//cdn.example/b.json"]);
    assert_eq!(read("fetch(\"ht	tps:cdn.example/two.json\")"), ["https:cdn.example/two.json"]);
    // A backslash for a slash, as the parser reads it: before a host, after a scheme, doubled.
    assert_eq!(read(r"void fetch(String.raw`/\cdn.example/x.json`)"), ["//cdn.example/x.json"]);
    assert_eq!(read(r#"fetch("\\cdn.example/y.json")"#), ["//cdn.example/y.json"]);
    assert_eq!(read(r#"fetch("https:\\cdn.example/z.json")"#), ["//cdn.example/z.json", "https://cdn.example/z.json"]);
    // A pattern's escaped slashes name no host.
    assert_eq!(read(r#"t.replace(/\.?0+$/,"");e.match(/^\/s\/([a-z0-9][a-z0-9-]*)\/?$/)"#), Vec::<String>::new());
}

#[test]
fn an_entry_git_holds_that_is_no_regular_file_is_refused() {
    let entries = |lines: &[&str]| lines.iter().flat_map(|l| l.bytes().chain([0])).collect::<Vec<u8>>();
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let file = format!("100644 {sha} 0\tsrc/lib.rs");
    let exe = format!("100755 {sha} 0\tweb/run.sh");
    assert_eq!(regular_entries(&entries(&[&file, &exe]), &entries(&["notes.txt"])), Ok((vec!["src/lib.rs".to_string(), "web/run.sh".to_string()], vec!["notes.txt".to_string()])));
    let link = format!("120000 {sha} 0\tweb/linked");
    let module = format!("160000 {sha} 0\tweb/vendor-docs");
    assert_eq!(regular_entries(&entries(&[&file, &link]), &[]), Err("`web/linked` is held by git as mode 120000, no regular file: every file git holds is one".to_string()));
    assert_eq!(regular_entries(&entries(&[&module]), &[]), Err("`web/vendor-docs` is held by git as mode 160000, no regular file: every file git holds is one".to_string()));
    assert_eq!(regular_entries(&[], &entries(&["web/nested/"])), Err("`web/nested/` is a repository of its own inside the crate, no file".to_string()));
    // A name that is not UTF-8 (Latin-1 bytes, as a Linux checkout keeps them) is refused, never
    // read through a replacement character.
    let latin1: Vec<u8> = format!("100644 {sha} 0\ttests/data/Capit").into_bytes().into_iter().chain([0xe3, b'o', 0]).collect();
    assert!(regular_entries(&latin1, &[]).is_err_and(|e| e.contains("not UTF-8")));
    assert!(regular_entries(&[], &[b'C', 0xe3, b'o', 0]).is_err_and(|e| e.contains("not UTF-8")));
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
    let section = char::from_u32(0xA7).unwrap();
    let mut hits = Vec::new();
    let mut hit = |h: String| {
        if !hits.contains(&h) {
            hits.push(h);
        }
    };
    // The text read twice: each line with the HTML entities that spell a letter decoded; then each
    // line with its entities (every one as its characters, `&lt;`, `&gt;` and `&amp;` among them,
    // and a name WHATWG does not list a space), its code escapes and its percent-encoded bytes
    // (`%20`, `%C3%A3`, as UTF-8) decoded, over and over until the line stops changing (a listed
    // form under one more level of escaping: draw.io's `&amp;nbsp;`, a JSON encoder's
    // `&nbsp;`), and only then the whole text's markup tags read as spaces, a tag across lines
    // keeping its lines (an editor's attribute per line, a converter's runs, a label's lines, a
    // break written as `&lt;br&gt;` or `<br>`). A name or a section sign in either
    // reading is a hit; the first keeps the letters the second reads otherwise (a pattern's `\b`
    // before hex letters, an `&` that only looks like an entity, a generic's angle brackets).
    let first: Vec<String> = text.lines().map(|l| decode_entities(l, false)).collect();
    let decoded = text.lines().map(second_reading).collect::<Vec<_>>().join("\n");
    let second: Vec<String> = untagged(&decoded).split('\n').map(String::from).collect();
    // And a third: the quoted values of the tags the second reads as spaces, each read as text of
    // its own (a label an editor keeps in an attribute: draw.io's `value`, a `title`).
    let third = tag_values(&decoded);
    // And a fourth: the decoded lines themselves, no span taken for a tag, so one the second reading
    // takes wrongly (a comparison in code, a comment, a CDATA section) still has its text read.
    let fourth: Vec<String> = decoded.split('\n').map(String::from).collect();
    for lines in [first, second, third, fourth] {
        let mut flat = Vec::new();
        let mut line_of = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            for c in letters(line) {
                flat.push(c);
                line_of.push(n + 1);
            }
        }
        for term in &banned {
            let t: Vec<char> = term.chars().collect();
            for start in 0..flat.len().saturating_sub(t.len() - 1) {
                if flat[start..start + t.len()] == t[..] {
                    hit(format!("line {}: {term}", line_of[start]));
                }
            }
        }
        for (n, line) in lines.iter().enumerate() {
            if line.contains(section) {
                hit(format!("line {}: section sign", n + 1));
            }
        }
    }
    hits
}

/// A line's letters and digits as the name check reads them: Unicode's own decomposition first
/// (NFKD: an accented letter of any block its base letter and its marks, a compatibility form, a
/// ligature or a full-width letter its plain letters), lowercased, then the letters it leaves by
/// the table here; marks and anything else that is no letter or digit dropped.
fn letters(line: &str) -> Vec<char> {
    let decomposed = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(line);
    // A mark is what Unicode gives a combining class other than 0 (its own list, in every block).
    let class = icu_normalizer::properties::CanonicalCombiningClassMapBorrowed::new();
    decomposed.chars().flat_map(char::to_lowercase).flat_map(fold).filter(|c| c.is_alphanumeric() && class.get32_u8(*c as u32) == 0).collect()
}

/// A line's second reading: its entities, code escapes and percent-encoded bytes decoded, again
/// and again until it stops changing (at most eight levels).
fn second_reading(line: &str) -> String {
    let mut l = line.to_string();
    for _ in 0..8 {
        let next = decode_percent(&decode_escapes(&decode_entities(&l, true)));
        if next == l {
            break;
        }
        l = next;
    }
    l
}

/// A line with its code escapes read as the crate's languages write a character: Rust's and a
/// script's braced `\u{..}` (Rust's `_` between its digits read through), a script's four-digit
/// `\u....` and two-digit `\x..`, TOML's eight-digit `\U........`, a stylesheet's one to six hex
/// digits and the one space that may end them; a line break or a tab written as an escape (`\n`,
/// `\r`, `\t`) as a space. Anything else is kept as written.
fn decode_escapes(line: &str) -> String {
    let hex_run = |s: &str, max: usize| s.chars().take(max).take_while(char::is_ascii_hexdigit).count();
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let (code, used) = if let Some(braced) = tail.strip_prefix("u{") {
            let inner = braced.find('}').map(|end| &braced[..end]);
            match inner {
                Some(i) if i.starts_with(|c: char| c.is_ascii_hexdigit()) && i.chars().all(|c| c.is_ascii_hexdigit() || c == '_') && (1..=6).contains(&i.chars().filter(|c| *c != '_').count()) => {
                    (u32::from_str_radix(&i.replace('_', ""), 16).ok(), i.len() + 3)
                }
                _ => (None, 0),
            }
        } else if let Some(u) = tail.strip_prefix('U').filter(|u| hex_run(u, 8) == 8) {
            (u32::from_str_radix(&u[..8], 16).ok(), 9)
        } else if let Some(u) = tail.strip_prefix('u').filter(|u| hex_run(u, 4) == 4) {
            (u32::from_str_radix(&u[..4], 16).ok(), 5)
        } else if let Some(x) = tail.strip_prefix('x').filter(|x| hex_run(x, 2) == 2) {
            (u32::from_str_radix(&x[..2], 16).ok(), 3)
        } else if tail.starts_with(['n', 'r', 't']) {
            (Some(u32::from(b' ')), 1)
        } else {
            match hex_run(tail, 6) {
                0 => (None, 0),
                n => (u32::from_str_radix(&tail[..n], 16).ok(), n + usize::from(tail[n..].starts_with(' '))),
            }
        };
        match code.and_then(char::from_u32) {
            Some(c) => {
                out.push(c);
                rest = &tail[used..];
            }
            None => {
                out.push('\\');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A text with every markup tag (`<` then a letter, `/` or `!`, up to the next `>` with no `<`
/// between, within 2 000 bytes) read as a space, a tag across lines keeping its line breaks:
/// `<span class="c2">`, `<tspan x="0" dy="1.2em">`, `<br>`, `</b>`, Inkscape's `<tspan` with an
/// attribute per line.
fn untagged(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        match tag_end(tail) {
            Some(end) => {
                out.push(' ');
                out.push_str(&"\n".repeat(tail[..end].matches('\n').count()));
                rest = &tail[end + 1..];
            }
            None => {
                out.push('<');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The quoted attribute values (after `=`, in double or single quotes) of every markup tag
/// `untagged` reads as a space, each with its own markup read as spaces, gathered by the line its
/// tag starts on.
fn tag_values(text: &str) -> Vec<String> {
    let mut lines = vec![String::new(); text.split('\n').count()];
    let mut rest = text;
    let mut offset = 0;
    while let Some(at) = rest.find('<') {
        let tail = &rest[at + 1..];
        match tag_end(tail) {
            Some(end) => {
                let line = text[..offset + at].matches('\n').count();
                let mut body = &tail[..end];
                while let Some(eq) = body.find('=') {
                    let after = body[eq + 1..].trim_start();
                    match after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                        Some(q) => match after[1..].find(q) {
                            Some(close) => {
                                lines[line].push(' ');
                                lines[line].push_str(&untagged(&after[1..1 + close]).replace('\n', " "));
                                body = &after[1 + close + 1..];
                            }
                            None => break,
                        },
                        // A value in no quotes: up to the white space or the end of the tag.
                        None => {
                            let end = after.find(|c: char| c.is_ascii_whitespace()).unwrap_or(after.len());
                            lines[line].push(' ');
                            lines[line].push_str(&after[..end]);
                            body = &after[end..];
                        }
                    }
                }
                offset += at + 1 + end + 1;
                rest = &tail[end + 1..];
            }
            None => {
                offset += at + 1;
                rest = tail;
            }
        }
    }
    lines
}

/// A line with its percent-encoded bytes (`%20`, `%C3%A3`) read, the bytes as UTF-8.
fn decode_percent(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).filter(|h| h.iter().all(u8::is_ascii_hexdigit)).and_then(|h| std::str::from_utf8(h).ok());
        match (b[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A lowercase letter Unicode's decomposition leaves as it is, read as the plain letters it stands
/// for: every such letter of Latin-1 and Latin Extended-A (a stroke, a letter of its own, a
/// ligature: the test below reads the two blocks whole), the apostrophe NFKD leaves of `ŉ`
/// (dropped), and the three Latin Extended-B letters HTML names (`&fnof;`, `&imped;`, `&jmath;`);
/// any other character as is.
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

/// HTML's named character references as WHATWG lists them (the `entities` crate): each name with
/// its `;` and its characters, and the legacy names a browser reads without their `;`, longest
/// first.
type HtmlNames = (std::collections::HashMap<&'static str, &'static str>, Vec<(&'static str, &'static str)>);
fn html_names() -> &'static HtmlNames {
    static TABLE: std::sync::OnceLock<HtmlNames> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut terminated = std::collections::HashMap::new();
        let mut legacy = Vec::new();
        for e in entities::ENTITIES.iter() {
            let name = e.entity.strip_prefix('&').unwrap_or(e.entity);
            match name.strip_suffix(';') {
                Some(n) => {
                    terminated.insert(n, e.characters);
                }
                None => legacy.push((name, e.characters)),
            }
        }
        legacy.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
        (terminated, legacy)
    })
}

/// A text with its HTML entities read: a number (`&#227;`, `&#xe3;`) as its character; a name
/// WHATWG lists as its characters (`&atilde;`, `&lstrok;`, `&aelig;`, `&eth;`, `&fnof;`, the section sign's),
/// a legacy name (which a browser reads without its `;`) and a number without the `;` too. Where
/// `others_as_space` is false, only the names whose characters hold a letter, a digit or the
/// section sign are read (any other kept as written); where it is true, every one is, and a name
/// WHATWG does not list is a space.
fn decode_entities(text: &str, others_as_space: bool) -> String {
    let (names, legacy) = html_names();
    let section = char::from_u32(0xA7).unwrap();
    let wanted = |chars: &str| others_as_space || chars.chars().any(|c| c.is_alphanumeric() || c == section);
    let number = |digits: &str| -> Option<String> {
        let code = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse::<u32>().ok(),
        };
        code.map(windows_1252).and_then(char::from_u32).map(String::from)
    };
    // Without its `;`: a number's digits, or the longest legacy name the text starts with.
    let unterminated = |tail: &str| -> Option<(String, usize)> {
        if let Some(n) = tail.strip_prefix('#') {
            let hex = n.starts_with(['x', 'X']);
            let skip = usize::from(hex);
            let len = n[skip..].chars().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
            return (len > 0).then(|| number(&n[..skip + len]).map(|r| (r, 1 + skip + len))).flatten();
        }
        legacy.iter().find(|(n, _)| tail.starts_with(n)).filter(|(_, c)| wanted(c)).map(|(n, c)| (c.to_string(), n.len()))
    };
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let terminated = tail.find(';').filter(|e| *e <= 40).and_then(|e| {
            let name = &tail[..e];
            // In the second reading every listed name as its characters (so a tag written
            // `&lt;br&gt;` is read as a tag), a name WHATWG does not list as a space.
            let unlisted = || {
                (others_as_space && !names.contains_key(name) && name.starts_with(|c: char| c.is_ascii_alphabetic()) && name.chars().all(|c| c.is_ascii_alphanumeric())).then(|| " ".to_string())
            };
            let read = match name.strip_prefix('#') {
                Some(digits) => number(digits),
                None => names.get(name).filter(|c| wanted(c)).map(|c| c.to_string()).or_else(unlisted),
            };
            read.map(|r| (r, e + 1))
        });
        match terminated.or_else(|| unterminated(tail)) {
            Some((r, used)) => {
                out.push_str(&r);
                rest = &tail[used..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn no_file_of_supervisao_names_a_vendor_a_product_or_a_customer() {
    let files = crate_files();
    let rel: Vec<String> = files.iter().map(|f| f.strip_prefix(crate_dir()).unwrap().to_string_lossy().replace('\\', "/")).collect();
    for must in ["Cargo.toml", "src/lib.rs", "tests/component.rs", "web/src/main.ts", "web/dist/assets/app.js", "web/dist/assets/app.css", "web/dist/index.html", "web/dist/source.hash", "web/package-lock.json"] {
        assert!(rel.iter().any(|r| r == must), "{must} is judged: {rel:?}");
    }
    assert!(rel.iter().all(|r| !r.contains("node_modules")), "{rel:?}");
    // Every file is read whole as UTF-8: a file that is not (a Windows-1252 note) is refused, its
    // bytes never guessed at.
    let not_text: Vec<&String> = files.iter().zip(&rel).filter(|(f, _)| std::str::from_utf8(&std::fs::read(f).unwrap()).is_err()).map(|(_, r)| r).collect();
    assert_eq!(not_text, Vec::<&String>::new(), "a file that is not UTF-8");
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
        // A ligature (from a PDF), full-width capitals, an HTML entity, a macron.
        format!("// {}", rev("tnalpg\u{fb01}noc")),
        format!("// {}", rev("TLEBINEG").chars().map(|c| char::from_u32(c as u32 + 0xfee0).unwrap()).collect::<String>()),
        format!("<!-- {} -->", rev("oivan od o;edlita&tipac")),
        format!("// {}", rev("oivan od o\u{101}tipac")),
        format!("// {}", rev("oivan od o;722#&tipac")),
        // Code escapes: the braced form of Rust and a script, a script's four digits and two, a
        // stylesheet's hex with the space that ends it; and a pattern's `\b` before hex letters,
        // which the escape reading swallows and the plain reading keeps.
        format!("// {}", rev("oivan od o}3e{u\\tipac")),
        format!("const s = \"{}\";", rev("oivan od o3e00u\\tipac")),
        format!("const s = \"{}\";", rev("oivan od o3ex\\tipac")),
        format!("p::after {{ content: \"{}\" }}", rev("oivan od o 3e\\tipac")),
        format!("const r = /\\b{}/;", rev("seulavbd")),
        // The section sign by an entity's name and number, and by an escape.
        format!("// spec {}3.2", rev(";tces&")),
        format!("<!-- spec {}3.2 -->", rev(";761#&")),
        format!("// spec {}3.2", rev("}7a{u\\")),
        // Rust's `_` inside a braced escape, TOML's eight-digit escape, a ligature's and a
        // stroke's entity names, and a legacy name and a number without their `;`.
        format!("const S: &str = \"{}\";", rev("oivan od o}3_e{u\\tipac")),
        format!("note = \"{}\"", rev("oivan od o3e000000U\\tipac")),
        format!("/// the {} file", rev("tnalpg;gilif&noc")),
        format!("/// a {} part", rev("t;kortsl&ebineg")),
        format!("<p>{}</p>", rev("oivan od oedlita&tipac")),
        format!("<p>{}</p>", rev("oivan od o722#&tipac")),
        // A separator written otherwise: a percent-encoded space and accent, a non-breaking
        // space's entity, a line break and a tab written as escapes.
        format!("// see {}", rev("erawtfoS02%arutappaM")),
        format!("// see {}", rev("oivan od o3A%3C%tipac")),
        format!("<td>{}</td>", rev("oivan;psbn&od;psbn&o;edlita&tipac")),
        format!("const L: &str = \"{}\";", rev("elareneGn\\moB")),
        format!("const L: &str = \"{}\";", rev("erawtfoSt\\arutappaM")),
        // A markup tag between a name's words: a converter's runs, a label's lines, a break, a
        // word in bold.
        format!("<span class=\"c1\">{} </span><span class=\"c2\">{}</span>", rev("arutappaM"), rev("erawtfoS")),
        format!("<text><tspan x=\"0\" dy=\"0\">{}</tspan><tspan x=\"0\" dy=\"1.2em\">{}</tspan></text>", rev("moB"), rev("elareneG")),
        format!("<th>{}<br>{}</th>", rev("tupnI"), rev("sreirraC")),
        format!("<b>{}</b> {}", rev("tramS"), rev("rotarugifnoC")),
        // A tag as the tools write it: Inkscape's attribute per line, draw.io's break as entities,
        // Go's JSON encoder's break as escapes.
        format!("<text\n   x=\"20\"><tspan\n     sodipodi:role=\"line\"\n     y=\"40\">{}</tspan><tspan\n     sodipodi:role=\"line\"\n     y=\"52\">{}</tspan></text>", rev("moB"), rev("elareneG")),
        format!("value=\"{}&lt;br&gt;{}\"", rev("arutappaM"), rev("erawtfoS")),
        format!("{{\"label\":\"{}\\u003cbr\\u003e{}\"}}", rev("tupnI"), rev("sreirraC")),
        // An accented letter beyond Latin-1 and Latin Extended-A, read by Unicode's own table:
        // Latin Extended-B's t with a comma below (a Romanian keyboard's), as a letter and as a
        // number; Latin Extended Additional's a with a dot below (a Vietnamese one).
        format!("// a {} part", rev("\u{21b}lebineg")),
        format!("<p>a {} part</p>", rev(";b12x#&lebineg")),
        format!("// {}", rev("erawtfoS arut\u{1ea1}ppaM")),
        // Letters of Latin-1 and Latin Extended-A that Unicode leaves undecomposed, written as
        // themselves (an eth, a thorn, an n with an apostrophe, a ligature, a sharp s).
        format!("// see the {} folder", rev("rodnevsco\u{f0}")),
        format!("// the {} rule", rev("txenhtiwelgna").replacen("th", "\u{fe}", 1)),
        format!("// the {} file", rev("tnalpgifnoc").replacen('n', "\u{149}", 1)),
        format!("// a {} line", rev("srei\u{e6}\u{e6}ractupni").replace("\u{e6}\u{e6}", "r")),
        format!("// {}", rev("sretemarapenil").replacen("ss", "\u{df}", 1).replacen("eni", "eni", 1)),
        // HTML's names WHATWG lists for Latin letters beyond the marks: an angstrom sign, an f with
        // a hook, a z with a stroke.
        format!("<p>the {} file</p>", rev("tnalpgi;fonf&noc")),
        format!("<p>the {} file</p>", rev("inoi;depmi&urtsi")),
        format!("<p>the {} file</p>", rev("tnalpgifnoc").replacen('a', "&angst;", 1)),
        // A listed form under one more level of escaping: draw.io's label HTML escaped once more
        // inside its attribute, and a JSON encoder's escaped ampersand.
        format!("<mxCell id=\"7\" value=\"{}&amp;nbsp;{}\" vertex=\"1\" />", rev("arutappaM"), rev("erawtfoS")),
        format!("<mxCell id=\"7\" value=\"{}\" vertex=\"1\" />", rev("oivan od o;edlita&tipac").replace('&', "&amp;")),
        format!("{{\"label\":\"{}\"}}", rev("oivan od o;edlita&tipac").replace('&', "\\u0026")),
        format!("{{\"label\":\"{}\\u0026nbsp;{}\"}}", rev("arutappaM"), rev("erawtfoS")),
        // A label kept in an attribute, the tag's other values beside it.
        format!("<rect id=\"r1\" title='{}&amp;nbsp;{}' width=\"40\" />", rev("moB"), rev("elareneG")),
        // A decoded spelling inside what the second reading takes for a tag: a comparison and a
        // generic arrow in Rust, a script's comparison, an XML comment, a CDATA section.
        format!("pub fn label(n: usize, cap: usize) -> &'static str {{ if n<cap {{ \"{}\" }} else {{ \"\" }} }} pub fn mark(n: usize) -> usize {{ n }}", rev("oivan od o}3e{u\\tipac")),
        format!("xs.filter((x) => x<limit && say(\"{}\")).map((x) => x);", rev("erawtfoS02%arutappaM")),
        format!("const NOTE: &str = \"<!-- drawn after the {} sheet -->\";", rev("erawtfoS;psbn&arutappaM")),
        format!("<text><![CDATA[{}]]></text>", rev("elareneGn\\moB")),
        // A numbered reference from 0x80 to 0x9F, as a browser reads it (Windows-1252's š).
        format!("// <p>the {} table</p>", rev("seulavbd").replacen('s', "&#154;", 1)),
        // A tag between the words, however long, a `>` inside a quoted value of it, a value in no
        // quotes.
        format!("<p>{}<img src=\"data:image/png;base64,{}\">{}</p>", rev("arutappaM"), "A".repeat(2100), rev("erawtfoS")),
        format!("<p>{}<span title=\"a>b\">{}</span></p>", rev("arutappaM"), rev("erawtfoS")),
        format!("<rect title={}&nbsp;{} width=4>", rev("moB"), rev("elareneG")),
        // A letter NFKD leaves with a modifier half ring; a mark of the Latin diacritic blocks
        // outside U+0300 to U+036F.
        format!("// the {} sheet", rev("wardatad").replacen('a', "\u{1e9a}", 1)),
        format!("// the {} rule", rev("oivan od oatipac").replacen('a', "a\u{1de7}", 1)),
    ];
    for s in &samples {
        assert_eq!(mentions(s).len(), 1, "{s}: {:?}", mentions(s));
    }
    assert_eq!(mentions(&format!("x\n// {}\n// {}", rev("erawtfoS arutappaM"), rev("elareneG moB"))), vec![format!("line 2: {}", rev("erawtfosarutappam")), format!("line 3: {}", rev("elarenegmob"))]);
    assert_eq!(mentions("// a loop with three outlets, two parcels and an infeed\nlet sorts = 1;"), Vec::<String>::new());
}

/// What the crate's Rust compiles in from a file, read as the compiler's tokens (the Rust lexer):
/// each `include_str!`, `include_bytes!` and `include!` with its argument when that is one literal,
/// and each `path = ...` an attribute holds (a module read from another file), however deep.
fn compiled_from(ts: proc_macro2::TokenStream, in_attribute: bool, out: &mut Vec<String>) {
    use proc_macro2::{Delimiter, TokenTree};
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            TokenTree::Group(g) => {
                let attribute = in_attribute || (g.delimiter() == Delimiter::Bracket && matches!(i.checked_sub(1).and_then(|j| toks.get(j)), Some(TokenTree::Punct(p)) if p.as_char() == '#' || p.as_char() == '!'));
                compiled_from(g.stream(), attribute, out);
            }
            TokenTree::Ident(id) => {
                let name = id.to_string();
                let name = name.trim_start_matches("r#");
                let bang = matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if bang && ["include_str", "include_bytes", "include"].contains(&name) {
                    let arg = match toks.get(i + 2) {
                        Some(TokenTree::Group(g)) => match g.stream().into_iter().collect::<Vec<_>>().as_slice() {
                            [TokenTree::Literal(l)] => l.to_string(),
                            other => format!("not one literal: {}", other.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ")),
                        },
                        _ => "no argument".to_string(),
                    };
                    out.push(format!("{name}! {arg}"));
                }
                if in_attribute && name == "path" && matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '=') {
                    out.push(format!("path = {}", toks.get(i + 2).map(ToString::to_string).unwrap_or_default()));
                }
            }
            _ => {}
        }
    }
}

/// The name check reads the files under the crate: so the crate compiles in no file from outside
/// it. No module is read from another file by `path`, and every file an `include` macro names is
/// one literal, resolved under the crate (the page's five, today).
#[test]
fn the_crate_compiles_in_no_file_from_outside_itself() {
    let root = crate_dir().canonicalize().unwrap();
    let mut included = Vec::new();
    for f in crate_files().iter().filter(|f| f.extension().is_some_and(|e| e == "rs")) {
        let text = std::fs::read_to_string(f).unwrap();
        let mut found = Vec::new();
        compiled_from(text.parse().unwrap_or_else(|e| panic!("{}: {e:?}", f.display())), false, &mut found);
        for c in found {
            let lit = c.strip_prefix("include_str! ").or_else(|| c.strip_prefix("include_bytes! ")).or_else(|| c.strip_prefix("include! "));
            let path = lit.and_then(|l| l.strip_prefix('"')).and_then(|l| l.strip_suffix('"')).unwrap_or_else(|| panic!("{}: {c}: a file compiled in by other than one plain literal", f.display()));
            let target = f.parent().unwrap().join(path).canonicalize().unwrap_or_else(|e| panic!("{}: {c}: {e}", f.display()));
            assert!(target.starts_with(&root), "{}: {c} reads {} from outside the crate", f.display(), target.display());
            included.push(target.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
    included.sort();
    assert_eq!(included, ["web/dist/assets/app.css", "web/dist/assets/app.js", "web/dist/index.html", "web/dist/output.hash", "web/dist/source.hash"]);
    // The reading sees each form: a path attribute (in `cfg_attr` too), a raw name, an argument
    // assembled, an include of a file outside.
    let read = |s: &str| {
        let mut out = Vec::new();
        compiled_from(s.parse().unwrap(), false, &mut out);
        out
    };
    assert_eq!(read("#[path = \"../../x.rs\"] mod x;"), ["path = \"../../x.rs\""]);
    assert_eq!(read("#[cfg_attr(unix, path = \"y.rs\")] mod y;"), ["path = \"y.rs\""]);
    assert_eq!(read("const A: &str = r#include_str ! (\"../a.txt\");"), ["include_str! \"../a.txt\""]);
    assert_eq!(read("const B: &[u8] = include_bytes!(concat!(env!(\"HOME\"), \"/b\"));"), ["include_bytes! not one literal: concat ! (env ! (\"HOME\") , \"/b\")"]);
    assert_eq!(read("fn f() { let path = 1; include!(\"c.rs\"); }"), ["include! \"c.rs\""]);
}

/// Where a markup tag opened at `tail` (just past its `<`) ends: the byte of its `>`, a value in
/// quotes after `=` read through (a `>` inside it is text, as HTML's tokenizer reads one), with no
/// length cap; none when it does not open as a tag (a letter, `/` or `!` first) or a `<` comes first.
fn tag_end(tail: &str) -> Option<usize> {
    if !tail.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!') {
        return None;
    }
    let mut quote: Option<char> = None;
    let mut last = ' ';
    for (i, c) in tail.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if (c == '"' || c == '\'') && last == '=' => quote = Some(c),
            None if c == '>' => return Some(i),
            None if c == '<' => return None,
            None => {}
        }
        if quote.is_none() && !c.is_whitespace() {
            last = c;
        }
    }
    None
}

/// WHATWG's remapping of a numbered reference from 0x80 to 0x9F: Windows-1252's characters there,
/// as a browser reads `&#154;` (š); any other number as itself.
fn windows_1252(code: u32) -> u32 {
    const HIGH: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F,
        0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..=0x9F).contains(&code) { HIGH[(code - 0x80) as usize] } else { code }
}

/// The files the compiler read for the crate's library and executable, by its own dep-info written
/// beside this test's binary (the newest of each): every dependency it lists, as written.
fn compiled_files() -> BTreeMap<&'static str, Vec<PathBuf>> {
    let deps = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let mut newest: BTreeMap<&'static str, (std::time::SystemTime, PathBuf)> = BTreeMap::new();
    for e in std::fs::read_dir(&deps).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("veredas_supervisao-") && name.ends_with(".d")) {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        let kind = if text.contains(".rlib:") || text.contains(".rmeta:") { "lib" } else { "bin" };
        let at = e.metadata().and_then(|m| m.modified()).unwrap();
        if newest.get(kind).is_none_or(|(t, _)| at > *t) {
            newest.insert(kind, (at, e.path()));
        }
    }
    newest
        .into_iter()
        .map(|(kind, (_, path))| {
            let text = std::fs::read_to_string(&path).unwrap();
            // Each dependency stands alone on a line of its own, ending in `:`.
            let files = text.lines().filter(|l| !l.starts_with('#') && l.ends_with(':') && !l.contains(": ")).map(|l| PathBuf::from(l.trim_end_matches(':').replace("\\ ", " "))).collect();
            (kind, files)
        })
        .collect()
}

/// What the crate builds and what the compiler reads for it: each target's source where the crate's
/// layout puts it (the library `src/lib.rs`, the executable `src/main.rs`, a test `tests/<name>.rs`,
/// cargo metadata's own `src_path`); and every file the compiler read for the library and the
/// executable (its dep-info, so a module, an include however spelled or a file an attribute reads),
/// under the crate, one git holds or could hold, Rust under `src/` or one of the five files of the
/// page compiled in; so the files read here are the files compiled. No cargo configuration in the
/// repository replaces a source (one in a folder above it or the user's own is a limit).
#[test]
fn every_file_the_crate_compiles_is_one_read_here() {
    let root = crate_dir().canonicalize().unwrap();
    let meta = metadata();
    let p = package(&meta);
    for t in p["targets"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let kind = t["kind"][0].as_str().unwrap();
        let src = PathBuf::from(t["src_path"].as_str().unwrap()).canonicalize().unwrap();
        let rel = src.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| panic!("{kind} {name} from {} outside the crate", src.display()));
        let want = match kind {
            "lib" => "src/lib.rs".to_string(),
            "bin" => "src/main.rs".to_string(),
            "test" => format!("tests/{name}.rs"),
            other => panic!("a target of a kind this test does not know: {other} {name}"),
        };
        assert_eq!(rel, want, "{kind} {name}");
    }
    let held: BTreeSet<PathBuf> = crate_files().into_iter().map(|f| f.canonicalize().unwrap()).collect();
    let page: Vec<String> = web::OUTPUT_FILES.iter().map(|f| format!("web/dist/{f}")).chain(["web/dist/source.hash".to_string(), "web/dist/output.hash".to_string()]).collect();
    let compiled = compiled_files();
    assert_eq!(compiled.keys().copied().collect::<Vec<_>>(), ["bin", "lib"], "the library's and the executable's dep-info beside this test");
    for (kind, files) in &compiled {
        assert!(files.iter().any(|f| f.ends_with("lib.rs") || f.ends_with("main.rs")), "{kind}: {files:?}");
        for f in files {
            // Written relative to the workspace's root, where cargo runs the compiler.
            let at = if f.is_absolute() { f.clone() } else { root.parent().unwrap().parent().unwrap().join(f) };
            let real = at.canonicalize().unwrap_or_else(|e| panic!("{kind} read {}: {e}", f.display()));
            let rel = real.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| panic!("{kind} compiles in {} from outside the crate", real.display()));
            assert!(held.contains(&real), "{kind} compiles in {rel}, which git neither holds nor could hold");
            assert!((rel.starts_with("src/") && rel.ends_with(".rs")) || page.contains(&rel), "{kind} compiles in {rel}, neither Rust under src/ nor the page");
        }
    }
    let repo = root.parent().unwrap().parent().unwrap();
    for dir in [repo, root.as_path()] {
        for f in [".cargo/config.toml", ".cargo/config"] {
            assert!(!dir.join(f).exists(), "{}: a cargo configuration could replace a source", dir.join(f).display());
        }
    }
}

/// The letters the reader claims, read whole from the lists that own them: every letter of Latin-1
/// and Latin Extended-A (Unicode's two blocks) and every name WHATWG lists whose character is a
/// Latin letter (its blocks, or a letter Unicode decomposes to plain ones), with its `;` and, for a
/// legacy name, without, in both readings: each read as plain letters, none left as a letter of
/// its own a name could hide behind.
#[test]
fn every_latin_letter_and_every_html_name_of_one_reads_as_plain_letters() {
    let plain = |r: &[char]| !r.is_empty() && r.iter().all(|c| c.is_ascii_lowercase());
    let latin = |c: char| matches!(c as u32, 0xc0..=0x24f | 0x1e00..=0x1eff);
    let mut letters_read = 0;
    // (µ, U+00B5, is Greek by Unicode's own decomposition, its mu: it stands for an m as often as it
    // looks like a u, and is read as itself, a limit.)
    for c in ('\u{80}'..='\u{17f}').filter(|c| c.is_alphabetic() && *c != '\u{b5}') {
        let read = letters(&c.to_string());
        assert!(plain(&read), "{c} (U+{:04X}) reads as {read:?}", c as u32);
        letters_read += 1;
    }
    assert_eq!(letters_read, 64 + 128, "Latin-1's letters (ª and º among them, µ apart) and Latin Extended-A's, every one");
    // Every letter of every block that Unicode's NFKD reads with a plain letter in it reads as
    // plain letters; every character Unicode gives a combining class is dropped between two letters.
    let nfkd = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd();
    let class = icu_normalizer::properties::CanonicalCombiningClassMapBorrowed::new();
    let (mut lettered, mut marks) = (0, 0);
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        if c.is_alphabetic() && nfkd.normalize(&c.to_string()).chars().any(|x| x.is_ascii_alphabetic()) {
            let read = letters(&c.to_string());
            assert!(plain(&read), "{c} (U+{:04X}) reads as {read:?}", c as u32);
            lettered += 1;
        }
        if class.get32_u8(c as u32) != 0 {
            assert_eq!(letters(&format!("a{c}b")), ['a', 'b'], "U+{:04X}, a mark", c as u32);
            marks += 1;
        }
    }
    assert!(lettered > 1000 && marks > 800, "{lettered} letters, {marks} marks");
    let (names, legacy) = html_names();
    assert_eq!(names.len() + legacy.len(), entities::ENTITIES.len());
    let mut names_read = 0;
    for (name, chars, written) in names.iter().map(|(n, c)| (*n, *c, format!("&{n};"))).chain(legacy.iter().map(|(n, c)| (*n, *c, format!("&{n} ")))) {
        let mut cs = chars.chars();
        let (Some(c), None) = (cs.next(), cs.next()) else { continue };
        let nfkd: Vec<char> = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(chars).chars().collect();
        if !c.is_alphabetic() || !(latin(c) || nfkd.iter().any(char::is_ascii_alphabetic)) {
            continue;
        }
        for second in [false, true] {
            let read = letters(&if second { second_reading(&written) } else { decode_entities(&written, false) });
            assert!(plain(&read), "{written} ({name}, U+{:04X}) reads as {read:?}", c as u32);
        }
        names_read += 1;
    }
    assert!(names_read >= 450, "the names of letters WHATWG lists: {names_read}");
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
    // The page's own sources are TypeScript and stylesheets alone: a config beside them (a
    // `tsconfig.json` or a `package.json` under `src/`, which the build reads for the files near
    // it and would follow outside) is refused, never hashed in.
    let other: Vec<&String> = sources.iter().filter(|s| s.starts_with("src/") && !(s.ends_with(".ts") || s.ends_with(".css"))).collect();
    assert_eq!(other, Vec::<&String>::new(), "a source under web/src that is neither TypeScript nor a stylesheet");
    // The page compiled in is the page the build wrote: a hand edit is refused.
    let written = web::BUILT_AS.trim();
    assert_eq!(written.len(), 16, "{written}");
    assert_eq!(web::output_hash(), written, "web/dist was edited after its build: run `npm run build` in web/");
    // What the crate compiles in is the committed build, file for file (CR LF read as LF): the
    // files the host serves (the crate's own list) and the two hashes.
    let dist = webdir.join("dist");
    for f in web::OUTPUT_FILES.iter().copied().chain(["source.hash", "output.hash"]) {
        let compiled = match f {
            "index.html" => web::INDEX_HTML,
            "assets/app.js" => web::APP_JS,
            "assets/app.css" => web::APP_CSS,
            "source.hash" => web::BUILT_FROM,
            "output.hash" => web::BUILT_AS,
            other => panic!("a file of the page this test does not know: {other}"),
        };
        assert_eq!(compiled.replace("\r\n", "\n"), std::fs::read_to_string(dist.join(f)).unwrap().replace("\r\n", "\n"), "web/dist/{f} is not what the crate compiles in");
    }
    let edited = format!("{};", web::APP_JS);
    assert_ne!(web::hash_texts([("assets/app.css", web::APP_CSS), ("assets/app.js", edited.as_str()), ("index.html", web::INDEX_HTML)]), written, "one byte of the page is a change");

    // Every file git holds or could hold under the crate is Rust, a hashed source of the page or
    // the page itself; so a new input the build could read either joins the hashed set or is
    // refused here.
    let rel: Vec<String> = crate_files().iter().map(|f| f.strip_prefix(crate_dir()).unwrap().to_string_lossy().replace('\\', "/")).collect();
    // And every hashed source is one git holds or could hold: a file git ignores (under a
    // `target/` folder in `src/`) would be hashed here and missing from every other checkout.
    let unlisted: Vec<&String> = sources.iter().filter(|s| !rel.iter().any(|r| r.strip_prefix("web/") == Some(s.as_str()))).collect();
    assert_eq!(unlisted, Vec::<&String>::new(), "a hashed source git ignores");
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
    // The build reads nothing beside them and emits nothing beside the page: its config is the
    // one this test holds, word for word (the guards' refusals are C-047's evidence, CI never
    // running Node), so a guard turned into a warning or a stamp moved is a change here.
    let config = std::fs::read_to_string(webdir.join("vite.config.ts")).unwrap().replace("\r\n", "\n");
    assert_eq!(config, VITE_CONFIG, "web/vite.config.ts is not the config this test holds");
    // Nor does a source reach outside: the TypeScript config names its compiler options and `src`
    // alone (no `extends`, no `references`, no `files`); a stylesheet or the page imports nothing
    // and points at nothing but a fragment of the page or a data URI (`url()` and `image-set()`
    // alike), and names no address; and no source reads the build's environment
    // (`import.meta.env`, `process.env`), which vite fills from the process, never from a file.
    let tsconfig: serde_json::Value = serde_json::from_slice(&std::fs::read(webdir.join("tsconfig.json")).unwrap()).unwrap();
    let keys: BTreeSet<&str> = tsconfig.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!((keys, &tsconfig["include"]), (["compilerOptions", "include"].into_iter().collect(), &serde_json::json!(["src"])), "{tsconfig}");
    for s in sources.iter().filter(|s| s.ends_with(".css") || s.ends_with(".html")) {
        let text = std::fs::read_to_string(webdir.join(s)).unwrap();
        assert!(!text.contains("@import"), "{s} imports a stylesheet");
        // No CSS module: its `composes ... from` and `@value ... from` read files the build's guard
        // never sees (the build reads none: `modules: false`).
        assert!(!s.contains(".module.") && !text.contains("composes") && !text.contains("@value"), "{s} is a CSS module");
        assert_eq!(urls(&text), Vec::<String>::new(), "{s} points outside the page");
        assert_eq!(addresses(&text), BTreeSet::<String>::new(), "{s} names an address");
    }
    // The words themselves refused, whatever stands between them (a comment, an escape in an
    // identifier, a line break): no source under `src/` says `meta`, `env` or `process` at all.
    for s in sources.iter().filter(|s| s.starts_with("src/")) {
        let text = decode_escapes(&std::fs::read_to_string(webdir.join(s)).unwrap());
        let words: BTreeSet<&str> = text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).collect();
        for w in ["meta", "env", "process"] {
            assert!(!words.contains(w), "{s} says `{w}`: a source may not read the build's environment");
        }
    }
    // The build runs what `package.json` names, and npm runs a script's `pre` and `post` too: its
    // keys and its scripts are the ones this test holds, its tools' versions the lock's.
    let package: serde_json::Value = serde_json::from_slice(&std::fs::read(webdir.join("package.json")).unwrap()).unwrap();
    let keys: BTreeSet<&str> = package.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["description", "devDependencies", "name", "private", "scripts", "type", "version"].into_iter().collect(), "{package}");
    assert_eq!(package["scripts"], serde_json::json!({ "build": "tsc --noEmit && vite build", "check": "tsc --noEmit" }), "web/package.json's scripts are not the ones this test holds");
    let tools: BTreeSet<&str> = package["devDependencies"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(tools, ["typescript", "vite"].into_iter().collect());
    // The source page word for word as well: vite moves an inline script into the bundle and fills
    // a `%NAME%` placeholder from the environment before it writes the built page, which would show
    // neither.
    let source_page = std::fs::read_to_string(webdir.join("index.html")).unwrap().replace("\r\n", "\n");
    assert_eq!(source_page, SOURCE_PAGE, "web/index.html is not the page this test holds");
    // The bundle's names are the ones the host serves, and it loads no other file: the page as
    // built word for word (its two references however quoted, no `srcset`, nothing else), the
    // stylesheet's every `url()` and `image-set()` a fragment or a data URI, no `@import` and no
    // address, the script no dynamic import and no address but the SVG namespace.
    assert_eq!(web::INDEX_HTML.replace("\r\n", "\n"), INDEX_PAGE, "web/dist/index.html is not the page this test holds");
    assert!(!web::APP_JS.is_empty() && !web::APP_CSS.is_empty());
    assert!(!web::APP_CSS.contains("@import"), "the stylesheet imports nothing");
    assert_eq!(urls(web::APP_CSS), Vec::<String>::new(), "the stylesheet points outside the page");
    assert_eq!(addresses(web::APP_CSS), BTreeSet::<String>::new(), "the stylesheet names an address");
    assert!(!web::APP_JS.contains("import(") && !web::APP_JS.contains("import.meta.url"), "the bundle loads no other file");
    let named = addresses(web::APP_JS);
    assert!(named.iter().all(|a| a == "http://www.w3.org/2000/svg" || a == "//www.w3.org/2000/svg"), "the script names an address: {named:?}");
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
    // Bytes, not text: two sources differing in a byte that is not UTF-8 differ.
    std::fs::write(copy.path().join("src/extra.ts"), [b'x', 0xff, b'x']).unwrap();
    let a = web::source_hash(copy.path()).unwrap();
    std::fs::write(copy.path().join("src/extra.ts"), [b'x', 0x80, b'x']).unwrap();
    assert_ne!(web::source_hash(copy.path()).unwrap(), a, "a byte that is not text counts");
}

/// Every `url(...)` target, and every quoted string inside an `image-set(...)` (prefixed or
/// not), in a text, that is neither a fragment of the page nor a data URI.
fn urls(text: &str) -> Vec<String> {
    let targets = text.match_indices("url(").map(|(i, _)| text[i + 4..].split(')').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string());
    let sets = text.match_indices("image-set(").flat_map(|(i, _)| {
        let inside = text[i + 10..].split(')').next().unwrap_or("");
        ['"', '\''].into_iter().flat_map(move |q| inside.split(q).skip(1).step_by(2).map(String::from))
    });
    targets.chain(sets).filter(|u| !u.starts_with('#') && !u.starts_with("data:")).collect()
}

/// Every address a text names (`http://`, `https://`, or `//` but a comment's), up to the
/// quote, space or parenthesis that ends it.
/// Every address a text names: a scheme a browser fetches from another host, in any case and with
/// `//` or without (`https:cdn.example/x` resolves against the page's own scheme to another host),
/// and a host written without a scheme (`//cdn.example/x`); each up to its quote, space or `)`.
/// The text is read as the URL parser reads one, its tabs, line feeds and carriage returns removed
/// first, so an address broken by one is still one. An address assembled at run time from pieces,
/// or spelled through an escape, is not read (the clause states it).
fn addresses(text: &str) -> BTreeSet<String> {
    fn upto(t: &str) -> String {
        t[..t.find(|c: char| c == '"' || c == '\'' || c == '`' || c.is_whitespace() || c == ')').unwrap_or(t.len())].to_string()
    }
    let text: String = text.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    // A backslash read as a slash, as the URL parser reads one for the schemes it fetches from
    // (`/\host`, `\\host`, `https:\\host`); kept apart from the text as written, so a pattern's
    // escaped slashes (`/^\/s\//`, `/\.?0+$/`) are told from a host by the dotted name that follows.
    let written = text;
    let text: String = written.replace('\\', "/");
    let text = text.as_str();
    let lower = text.to_ascii_lowercase();
    let mut out = BTreeSet::new();
    for scheme in ["http:", "https:", "ws:", "wss:", "ftp:"] {
        for (i, _) in lower.match_indices(scheme) {
            if i == 0 || !lower.as_bytes()[i - 1].is_ascii_alphanumeric() {
                out.insert(upto(&text[i..]));
            }
        }
    }
    // A host follows the slashes (a comment's `// ` names none); where a backslash stands for one,
    // a dotted host (two labels or more).
    let dotted = |a: &str| {
        let host = a[2..].split(['/', ':', '?', '#']).next().unwrap_or("");
        let labels: Vec<&str> = host.split('.').collect();
        labels.len() > 1 && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    };
    for (i, _) in text.match_indices("//") {
        let a = upto(&text[i..]);
        if a.len() > 2 && (&written[i..i + 2] == "//" || dotted(&a)) {
            out.insert(a);
        }
    }
    out
}

fn copy_tree(from: &Path, to: &Path, rel: &[String]) {
    for r in rel {
        let dst = to.join(r);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(from.join(r), dst).unwrap();
    }
}

/// The page's source, word for word (`web/index.html`): its one module and nothing else.
const SOURCE_PAGE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Supervisão</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
"#;

/// The page as built, word for word (`web/dist/index.html`): its two references and nothing else.
const INDEX_PAGE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Supervisão</title>
    <script type="module" crossorigin src="/assets/app.js"></script>
    <link rel="stylesheet" crossorigin href="/assets/app.css">
  </head>
  <body>
    <div id="app"></div>
  </body>
</html>
"#;

/// The page's build configuration, word for word (`web/vite.config.ts`).
const VITE_CONFIG: &str = r####"// Supervisão's page: built into dist/ with fixed names (the crate compiles them in with
// include_str!), with two hashes beside them, both recomputed by the crate's component test
// (src/web.rs states the algorithm: FNV-1a 64 over each file's relative path, a NUL, its bytes
// with every CR LF pair read as LF, a NUL, in path order):
//   dist/source.hash  the sources the page was built from (SOURCE_FILES and everything under src/)
//   dist/output.hash  the page as written (OUTPUT_FILES)
// The build reads nothing else: no .env, no public/ folder, no PostCSS file, no CSS module (its
// `composes ... from` reads a file the guard never sees), no module and no
// file a stylesheet pulls in outside src/ and the hashed files (refused; node_modules/ too, whose
// packages the hash never sees), no asset inlined (an asset is a file of its own, so the
// three-file rule refuses it), and it emits the three files alone (refused otherwise). The hashes
// are written once the bundle is on disk: a failed build never writes them.
import { defineConfig, type Plugin } from "vite";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const SOURCE_FILES = ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"];
const OUTPUT_FILES = ["assets/app.css", "assets/app.js", "index.html"];

function walk(dir: string, rel: string, out: string[]) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const r = `${rel}/${name}`;
    if (statSync(p).isDirectory()) walk(p, r, out);
    else out.push(r);
  }
}

function fnv(bytes: Uint8Array, h: bigint): bigint {
  for (const b of bytes) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h;
}

// The hash of the files under root named by rel, sorted as Rust sorts strings: by bytes.
function hashFiles(root: string, rel: string[]): string {
  const files = [...rel].sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const enc = new TextEncoder();
  let h = 0xcbf29ce484222325n;
  for (const r of files) {
    const raw = readFileSync(join(root, r));
    const lf = raw.filter((b, i) => !(b === 13 && raw[i + 1] === 10));
    h = fnv(enc.encode(r), h);
    h = fnv(new Uint8Array([0]), h);
    h = fnv(lf, h);
    h = fnv(new Uint8Array([0]), h);
  }
  return h.toString(16).padStart(16, "0");
}

export function sourceHash(root: string): string {
  const files = [...SOURCE_FILES];
  walk(join(root, "src"), "src", files);
  return hashFiles(root, files);
}

// A path as written, its separators forward and its drive letter upper case; its case kept, so a
// file the walk did not list (another case, a stream after a colon) is no hashed file.
const norm = (p: string) => resolve(p).replace(/\\/g, "/").replace(/^([a-z]):/, (_m, d: string) => `${d.toUpperCase()}:`);

function stamp(): Plugin {
  return {
    name: "source-hash",
    // After vite's own plugins, so the bundle it judges holds index.html.
    enforce: "post",
    generateBundle(_options, bundle) {
      // Every module read, and every file one pulls in (a stylesheet's @import, what vite
      // watches), is exactly one of the files the source hash reads.
      const files = [...SOURCE_FILES];
      walk(join(__dirname, "src"), "src", files);
      const hashed = new Set(files.map((f) => norm(join(__dirname, f))));
      for (const id of [...this.getModuleIds(), ...this.getWatchFiles()]) {
        if (id.startsWith("\0")) continue;
        const p = norm(id.split("?")[0]);
        if (!hashed.has(p)) {
          this.error(`the page reads ${id}, which is none of the files the source hash reads (the files under src/ and the hashed ones, as the walk lists them)`);
        }
      }
      // The page is the three files the host serves, and nothing else.
      const names = Object.keys(bundle).sort();
      if (JSON.stringify(names) !== JSON.stringify(OUTPUT_FILES)) {
        this.error(`the page must build into ${OUTPUT_FILES.join(", ")} alone, not ${names.join(", ")}`);
      }
    },
    writeBundle() {
      const dist = join(__dirname, "dist");
      writeFileSync(join(dist, "source.hash"), sourceHash(__dirname));
      writeFileSync(join(dist, "output.hash"), hashFiles(dist, OUTPUT_FILES));
    },
  };
}

export default defineConfig({
  base: "/",
  plugins: [stamp()],
  envDir: false,
  publicDir: false,
  css: { postcss: {}, modules: false },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    modulePreload: false,
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/app-[name].js",
        assetFileNames: (info) => (info.names?.some((n) => n.endsWith(".css")) ? "assets/app.css" : "assets/[name][extname]"),
      },
    },
  },
});
"####;
