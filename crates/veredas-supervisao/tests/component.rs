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
    let section = char::from_u32(0xA7).unwrap();
    let mut hits = Vec::new();
    let mut hit = |h: String| {
        if !hits.contains(&h) {
            hits.push(h);
        }
    };
    // Each line read twice: its HTML entities decoded, then its code escapes too. A name or a
    // section sign in either reading is a hit; the first keeps the letters an escape-like run (a
    // pattern's `\b` before hex letters) would swallow in the second.
    let readings: [fn(&str) -> String; 2] = [decode_entities, |l: &str| decode_escapes(&decode_entities(l))];
    for read in readings {
        let lines: Vec<String> = text.lines().map(read).collect();
        let mut flat = Vec::new();
        let mut line_of = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            for c in line.chars().flat_map(char::to_lowercase).flat_map(fold).filter(|c| c.is_alphanumeric() && !('\u{300}'..='\u{36f}').contains(c)) {
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

/// A line with its code escapes read as the crate's languages write a character: Rust's and a
/// script's braced `\u{..}`, a script's four-digit `\u....` and two-digit `\x..`, a stylesheet's
/// one to six hex digits and the one space that may end them. Anything else is kept as written.
fn decode_escapes(line: &str) -> String {
    let hex_run = |s: &str, max: usize| s.chars().take(max).take_while(char::is_ascii_hexdigit).count();
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let (code, used) = if let Some(braced) = tail.strip_prefix("u{") {
            match braced.find('}') {
                Some(end) if (1..=6).contains(&end) && hex_run(braced, 6) == end => (u32::from_str_radix(&braced[..end], 16).ok(), end + 3),
                _ => (None, 0),
            }
        } else if let Some(u) = tail.strip_prefix('u').filter(|u| hex_run(u, 4) == 4) {
            (u32::from_str_radix(&u[..4], 16).ok(), 5)
        } else if let Some(x) = tail.strip_prefix('x').filter(|x| hex_run(x, 2) == 2) {
            (u32::from_str_radix(&x[..2], 16).ok(), 3)
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

/// A lowercase letter read as the plain letters it stands for: an accented Latin-1 or Latin
/// Extended-A letter as its base letter, a ligature (U+FB00 to U+FB06) as its letters, a full-width
/// form (U+FF01 to U+FF5E) as its ASCII; any other character as is.
fn fold(c: char) -> Vec<char> {
    let one = |x: char| vec![x];
    match c {
        'à'..='å' => one('a'),
        'ç' => one('c'),
        'è'..='ë' => one('e'),
        'ì'..='ï' => one('i'),
        'ñ' => one('n'),
        'ò'..='ö' | 'ø' => one('o'),
        'ù'..='ü' => one('u'),
        'ý' | 'ÿ' => one('y'),
        '\u{100}'..='\u{17f}' => {
            let base = match c as u32 {
                0x100..=0x105 => "a",
                0x106..=0x10d => "c",
                0x10e..=0x111 => "d",
                0x112..=0x11b => "e",
                0x11c..=0x123 => "g",
                0x124..=0x127 => "h",
                0x128..=0x131 => "i",
                0x132..=0x133 => "ij",
                0x134..=0x135 => "j",
                0x136..=0x138 => "k",
                0x139..=0x142 => "l",
                0x143..=0x14b => "n",
                0x14c..=0x151 => "o",
                0x152..=0x153 => "oe",
                0x154..=0x159 => "r",
                0x15a..=0x161 => "s",
                0x162..=0x167 => "t",
                0x168..=0x173 => "u",
                0x174..=0x175 => "w",
                0x176..=0x178 => "y",
                0x179..=0x17e => "z",
                _ => "s",
            };
            base.chars().collect()
        }
        '\u{fb00}' => "ff".chars().collect(),
        '\u{fb01}' => "fi".chars().collect(),
        '\u{fb02}' => "fl".chars().collect(),
        '\u{fb03}' => "ffi".chars().collect(),
        '\u{fb04}' => "ffl".chars().collect(),
        '\u{fb05}' | '\u{fb06}' => "st".chars().collect(),
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).map(|x| x.to_ascii_lowercase()).into_iter().collect(),
        c => one(c),
    }
}

/// A text with its HTML entities read: a number (`&#227;`, `&#xe3;`) as its character, an
/// accented letter's name (`&atilde;`) as its base letter, the section sign's name as the sign.
fn decode_entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let entity = tail.find(';').filter(|e| *e <= 12).map(|e| &tail[1..e]);
        let read = entity.and_then(|e| {
            if let Some(hex) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from)
            } else if let Some(dec) = e.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32).map(String::from)
            } else if e == "sect" {
                char::from_u32(0xA7).map(String::from)
            } else {
                let mut cs = e.chars();
                let letter = cs.next().filter(char::is_ascii_alphabetic)?;
                ["acute", "grave", "circ", "tilde", "uml", "ring", "cedil", "slash", "macr", "caron", "breve", "ogon", "dot"].contains(&cs.as_str()).then(|| letter.to_string())
            }
        });
        match (entity, read) {
            (Some(e), Some(r)) => {
                out.push_str(&r);
                rest = &tail[e.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
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
    // The page's own sources are TypeScript and stylesheets alone: a config beside them (a
    // `tsconfig.json` or a `package.json` under `src/`, which the build reads for the files near
    // it and would follow outside) is refused, never hashed in.
    let other: Vec<&String> = sources.iter().filter(|s| s.starts_with("src/") && !(s.ends_with(".ts") || s.ends_with(".css"))).collect();
    assert_eq!(other, Vec::<&String>::new(), "a source under web/src that is neither TypeScript nor a stylesheet");
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
    // alone (no `extends`, no `references`, no `files`), and a stylesheet imports nothing and
    // points at nothing but a fragment of the page or a data URI.
    let tsconfig: serde_json::Value = serde_json::from_slice(&std::fs::read(webdir.join("tsconfig.json")).unwrap()).unwrap();
    let keys: BTreeSet<&str> = tsconfig.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!((keys, &tsconfig["include"]), (["compilerOptions", "include"].into_iter().collect(), &serde_json::json!(["src"])), "{tsconfig}");
    for s in sources.iter().filter(|s| s.ends_with(".css") || s.ends_with(".html")) {
        let text = std::fs::read_to_string(webdir.join(s)).unwrap();
        assert!(!text.contains("@import"), "{s} imports a stylesheet");
        assert_eq!(urls(&text), Vec::<String>::new(), "{s} points outside the page");
    }
    // The bundle's names are the ones the host serves, and it loads no other file: the page's
    // every `src` and `href` one of the two, the stylesheet's every `url()` a fragment or a data
    // URI and no `@import`, the script no dynamic import and no address but the SVG namespace.
    let refs: Vec<String> = ["src=\"", "href=\""].iter().flat_map(|a| web::INDEX_HTML.match_indices(a).map(move |(i, _)| web::INDEX_HTML[i + a.len()..].split('"').next().unwrap().to_string())).collect();
    assert_eq!(refs.iter().map(String::as_str).collect::<BTreeSet<_>>(), ["/assets/app.css", "/assets/app.js"].into_iter().collect::<BTreeSet<_>>(), "{}", web::INDEX_HTML);
    assert!(!web::APP_JS.is_empty() && !web::APP_CSS.is_empty());
    assert!(!web::APP_CSS.contains("@import"), "the stylesheet imports nothing");
    assert_eq!(urls(web::APP_CSS), Vec::<String>::new(), "the stylesheet points outside the page");
    assert!(!web::APP_JS.contains("import(") && !web::APP_JS.contains("import.meta.url"), "the bundle loads no other file");
    let addresses: BTreeSet<&str> = ["http://", "https://", "//"]
        .iter()
        .flat_map(|scheme| web::APP_JS.match_indices(scheme).map(|(i, _)| &web::APP_JS[i..]).map(|t| &t[..t.find(|c: char| c == '"' || c == '\'' || c == '`' || c.is_whitespace() || c == ')').unwrap_or(t.len())]))
        .filter(|a| !a.starts_with("//") || !a[2..].starts_with(' '))
        .collect();
    assert!(addresses.iter().all(|a| *a == "http://www.w3.org/2000/svg" || *a == "//www.w3.org/2000/svg"), "the script names an address: {addresses:?}");
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

/// Every `url(...)` in a text whose target is neither a fragment of the page nor a data URI.
fn urls(text: &str) -> Vec<String> {
    text.match_indices("url(")
        .map(|(i, _)| text[i + 4..].split(')').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string())
        .filter(|u| !u.starts_with('#') && !u.starts_with("data:"))
        .collect()
}

fn copy_tree(from: &Path, to: &Path, rel: &[String]) {
    for r in rel {
        let dst = to.join(r);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(from.join(r), dst).unwrap();
    }
}

/// The page's build configuration, word for word (`web/vite.config.ts`).
const VITE_CONFIG: &str = r####"// Supervisão's page: built into dist/ with fixed names (the crate compiles them in with
// include_str!), with two hashes beside them, both recomputed by the crate's component test
// (src/web.rs states the algorithm: FNV-1a 64 over each file's relative path, a NUL, its bytes
// with every CR LF pair read as LF, a NUL, in path order):
//   dist/source.hash  the sources the page was built from (SOURCE_FILES and everything under src/)
//   dist/output.hash  the page as written (OUTPUT_FILES)
// The build reads nothing else: no .env, no public/ folder, no PostCSS file, no module and no
// file a stylesheet pulls in outside src/, the hashed files and node_modules/ (refused), no asset
// inlined (an asset is a file of its own, so the three-file rule refuses it), and it emits the
// three files alone (refused otherwise). The hashes are written once the bundle is on disk: a
// failed build never writes them.
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

const norm = (p: string) => resolve(p).replace(/\\/g, "/").toLowerCase();

function stamp(): Plugin {
  const src = norm(join(__dirname, "src")) + "/";
  const modules = norm(join(__dirname, "node_modules")) + "/";
  const hashed = SOURCE_FILES.map((f) => norm(join(__dirname, f)));
  return {
    name: "source-hash",
    // After vite's own plugins, so the bundle it judges holds index.html.
    enforce: "post",
    generateBundle(_options, bundle) {
      // Every module read, and every file one pulls in (a stylesheet's @import, what vite
      // watches), is a hashed source or a locked dependency.
      for (const id of [...this.getModuleIds(), ...this.getWatchFiles()]) {
        if (id.startsWith("\0")) continue;
        const p = norm(id.split("?")[0]);
        if (!(p.startsWith(src) || p.startsWith(modules) || hashed.includes(p))) {
          this.error(`the page reads ${id}, outside src/, the hashed files and node_modules/: move it under src/`);
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
  css: { postcss: {} },
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
