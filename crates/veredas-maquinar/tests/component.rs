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

/// The string literals in `text` from `from` to the close of the delimiter opened at `from`
/// (`(`, `[` or `{`), as rustc reads them: plain `"..."` with its escapes, raw `r"..."` and
/// `r#"..."#` with any number of hashes. Returns the literals and where the delimiter closed;
/// `None` when it never does.
fn literals_within(text: &[char], from: usize) -> Option<(Vec<String>, usize)> {
    let open = text[from];
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0;
    let mut out = Vec::new();
    let mut i = from;
    while i < text.len() {
        let c = text[i];
        if c == 'r' && (text.get(i + 1) == Some(&'"') || text.get(i + 1) == Some(&'#')) && (i == 0 || !text[i - 1].is_alphanumeric() && text[i - 1] != '_') {
            // A raw string: `r`, hashes, a quote, the text, a quote, the same hashes.
            let mut j = i + 1;
            let mut hashes = 0;
            while text.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if text.get(j) == Some(&'"') {
                let start = j + 1;
                let mut k = start;
                loop {
                    let ch = text.get(k)?;
                    if *ch == '"' && (1..=hashes).all(|h| text.get(k + h) == Some(&'#')) {
                        break;
                    }
                    k += 1;
                }
                out.push(text[start..k].iter().collect());
                i = k + 1 + hashes;
                continue;
            }
        }
        if c == '"' {
            let start = i + 1;
            let mut k = start;
            let mut s = String::new();
            loop {
                let ch = text.get(k)?;
                match ch {
                    '\\' => {
                        // The escapes a path may hold; others are kept as they are.
                        match text.get(k + 1) {
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => {
                                s.push('\\');
                                s.push(*other);
                            }
                            None => return None,
                        }
                        k += 2;
                    }
                    '"' => break,
                    ch => {
                        s.push(*ch);
                        k += 1;
                    }
                }
            }
            out.push(s);
            i = k + 1;
            continue;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some((out, i));
            }
        }
        i += 1;
    }
    None
}

/// What an include or a path attribute names: a file resolved, or an argument that is no
/// literal this reader can resolve (a macro variable, an expression), which fails closed.
#[derive(Debug, PartialEq, Eq)]
enum Included {
    File(PathBuf),
    Unresolvable { line: usize, what: String },
}

/// Where `index` (into `text` as chars) sits, as a 1-based line.
fn line_of(text: &[char], index: usize) -> usize {
    text[..index.min(text.len())].iter().filter(|c| **c == '\n').count() + 1
}

/// The files a crate file pulls in from elsewhere: every `include_str!`, `include_bytes!` and
/// `include!` argument and every `path = "..."` inside a `#[...]` attribute (`#[path]` itself,
/// or within `cfg_attr`), with any delimiter and whitespace rustc accepts, the argument's
/// string literals (plain or raw) joined in order, `env!("CARGO_MANIFEST_DIR")` standing for
/// the manifest dir; a relative path is resolved from the including file's folder, an absolute
/// one taken as it is. An argument with no literal in it (a macro variable, an expression)
/// cannot be read here and is reported as unresolvable, so the check fails closed. A file the
/// crate compiles in from outside its folder is part of the crate, so it is read too.
fn included_files(file: &Path, text: &str, manifest_dir: &Path) -> Vec<Included> {
    let here = file.parent().unwrap_or(manifest_dir);
    let chars: Vec<char> = text.chars().collect();
    let flat: String = chars.iter().collect();
    let mut out = Vec::new();
    let resolve = |literals: Vec<String>, at: usize, what: &str, out: &mut Vec<Included>| {
        // The manifest dir stands for `env!("CARGO_MANIFEST_DIR")`; any other literal is a
        // piece of the path.
        let manifest = literals.iter().any(|l| l == "CARGO_MANIFEST_DIR");
        let pieces: Vec<&String> = literals.iter().filter(|l| l.as_str() != "CARGO_MANIFEST_DIR").collect();
        if pieces.is_empty() {
            out.push(Included::Unresolvable { line: line_of(&chars, at), what: what.to_string() });
            return;
        }
        let joined: String = pieces.iter().map(|s| s.as_str()).collect();
        let p = Path::new(&joined);
        let resolved = if manifest {
            manifest_dir.join(joined.trim_start_matches(['/', '\\']))
        } else if p.is_absolute() {
            p.to_path_buf()
        } else {
            here.join(&joined)
        };
        if resolved.is_file() {
            out.push(Included::File(resolved));
        }
    };
    // The include macros.
    for marker in ["include_str!", "include_bytes!", "include!"] {
        let mut from = 0;
        while let Some(at) = flat[from..].find(marker) {
            let at = from + at;
            let index = flat[..at].chars().count();
            // `include!` must not match inside `include_str!`.
            let preceded = index > 0 && (chars[index - 1].is_alphanumeric() || chars[index - 1] == '_');
            let mut j = index + marker.chars().count();
            while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            if !preceded && matches!(chars.get(j), Some('(') | Some('[') | Some('{')) {
                match literals_within(&chars, j) {
                    Some((literals, _)) => resolve(literals, index, marker, &mut out),
                    None => out.push(Included::Unresolvable { line: line_of(&chars, index), what: marker.to_string() }),
                }
            }
            from = at + marker.len();
        }
    }
    // `path = "..."` inside any `#[...]` attribute.
    let mut from = 0;
    while let Some(at) = flat[from..].find("#[") {
        let at = from + at;
        let index = flat[..at].chars().count();
        let Some((_, end)) = literals_within(&chars, index + 1) else { break };
        let attr: String = chars[index + 2..end].iter().collect();
        let mut scan = 0;
        while let Some(p) = attr[scan..].find("path") {
            let p = scan + p;
            let before = attr[..p].chars().next_back();
            let rest = attr[p + 4..].trim_start();
            if before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
                && let Some(value) = rest.strip_prefix('=')
            {
                let value_chars: Vec<char> = format!("({})", value.trim_start()).chars().collect();
                match literals_within(&value_chars, 0) {
                    Some((literals, _)) if !literals.is_empty() => resolve(vec![literals[0].clone()], index, "#[path]", &mut out),
                    _ => out.push(Included::Unresolvable { line: line_of(&chars, index), what: "#[path]".into() }),
                }
            }
            scan = p + 4;
        }
        from = at + 2;
    }
    out
}

/// Every file of the crate: those under its folder, and those its files pull in from elsewhere,
/// each read once; and every include whose path could not be read, which fails the check.
fn all_crate_files(manifest_dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    crate_files(manifest_dir, &mut files);
    let mut seen: std::collections::BTreeSet<PathBuf> = files.iter().map(|f| f.canonicalize().unwrap()).collect();
    let mut queue = files.clone();
    let mut unresolvable = Vec::new();
    while let Some(file) = queue.pop() {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).to_string();
        for inc in included_files(&file, &text, manifest_dir) {
            match inc {
                Included::File(inc) => {
                    let key = inc.canonicalize().unwrap();
                    if seen.insert(key) {
                        files.push(inc.clone());
                        queue.push(inc);
                    }
                }
                Included::Unresolvable { line, what } => {
                    unresolvable.push(format!("{}: line {line}: {what} with a path this reader cannot resolve", file.display()))
                }
            }
        }
    }
    (files, unresolvable)
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
    let (files, unresolvable) = all_crate_files(&root);
    assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "{files:?}");
    assert!(files.len() >= 4, "{files:?}");
    assert_eq!(unresolvable, Vec::<String>::new(), "every include names a literal path");
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
fn a_file_the_crate_pulls_in_from_outside_its_folder_is_read_too() {
    // A planted crate whose files include, by every form rustc accepts, files outside its
    // folder that each hold a vendor term: every one is found; a file that is not there is
    // left alone; an include whose path is no literal is reported, so the check fails closed.
    let dir = planted("includes", &[("Cargo.toml", "[package]\nname = \"x\"\n")]);
    let outside = dir.join("outside");
    let abs = |name: &str| outside.join(name).to_string_lossy().replace('\\', "/");
    // The markers are built here, never written in this file: the reader scans this test too,
    // and a planted `include` inside a string of it would read as one of the bench's own.
    let (inc_str, inc_bytes, inc_mac) = (["inc", "lude_str!"].concat(), ["inc", "lude_bytes!"].concat(), ["inc", "lude!"].concat());
    let lib = format!(
        "const A: &str = INCSTR!(\"../../outside/a.txt\");\n\
         const B: &[u8] = INCBYTES!( \"../../outside/b.bin\" );\n\
         #[path = \"../../outside/c.rs\"]\nmod c;\n\
         const D: &str = INCSTR!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../outside/d.txt\"));\n\
         const E: &str = INCSTR!(\"../../outside/missing.txt\");\n\
         INCMAC!(\"../../outside/e.rs\");\n\
         const G: &str = INCSTR!(r\"{g}\");\n\
         const H: &str = INCSTR!(r#\"{h}\"#);\n\
         const I: &str = INCSTR![\"../../outside/i.txt\"];\n\
         const J: &str = INCSTR!{{\"../../outside/j.txt\"}};\n\
         #[ path = \"../../outside/k.rs\" ]\nmod k;\n\
         #[cfg_attr(all(), path = \"../../outside/l.rs\")]\nmod l;\n\
         const M: &str = INCSTR!(concat!(\"{m_dir}\", \"/m.txt\"));\n\
         const N: &str = INCSTR!(concat!( env!(\"CARGO_MANIFEST_DIR\"), \"/../outside\", \"/n.txt\" ));\n\
         const O: &str = INCSTR!(concat!(\"../../outside/o\", \".txt\"));\n\
         const P: &str = INCSTR!(\"..\\\\..\\\\outside\\\\p.txt\");\n\
         macro_rules! inc {{ ($p:literal) => {{ INCSTR!($p) }}; }}\n\
         const Q: &str = inc!(\"../../outside/q.txt\");\n",
        g = abs("g.txt"),
        h = abs("h.txt"),
        m_dir = outside.to_string_lossy().replace('\\', "/"),
    )
    .replace("INCSTR!", &inc_str)
    .replace("INCBYTES!", &inc_bytes)
    .replace("INCMAC!", &inc_mac);
    let mut files: Vec<(String, String)> = vec![("crate/src/lib.rs".into(), lib), ("crate/src/inner.rs".into(), "// an announcement\n".into())];
    let term = |i: usize| -> String {
        let terms = [rev("tleBineG"), rev("MAOOTXTR"), rev("lortnoC troS retsaM"), rev("rodnev/scod"), rev("ecafretni tsoh"), rev("XTROTMAO")];
        format!("x {} y", terms[i % terms.len()])
    };
    let named = ["a.txt", "b.bin", "c.rs", "d.txt", "g.txt", "h.txt", "i.txt", "j.txt", "k.rs", "l.rs", "m.txt", "n.txt", "o.txt", "p.txt", "q.txt"];
    for (i, name) in named.iter().enumerate() {
        let text = if name.ends_with(".rs") { format!("// {}\npub fn f() {{}}\n", term(i)) } else { term(i) };
        files.push((format!("outside/{name}"), text));
    }
    files.push(("outside/e.rs".into(), "// an outcome\n// and a nested one\nconst F: &str = include_str!(\"f.txt\");\n".into()));
    files.push(("outside/f.txt".into(), format!("{}3.1", char::from_u32(0xA7).unwrap())));
    for (path, text) in &files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let manifest_dir = dir.join("crate");
    let (found_files, unresolvable) = all_crate_files(&manifest_dir);
    let names: Vec<String> = found_files.iter().map(|f| f.file_name().unwrap().to_string_lossy().to_string()).collect();
    let mut expected: Vec<&str> = vec!["lib.rs", "inner.rs", "e.rs", "f.txt"];
    expected.extend(named.iter().filter(|n| **n != "q.txt"));
    for e in &expected {
        assert!(names.contains(&e.to_string()), "{e} missing from {names:?}");
    }
    assert_eq!(found_files.len(), expected.len(), "{names:?}");
    // The include through a macro variable cannot be read here: reported, never passed over.
    assert_eq!(unresolvable.len(), 1, "{unresolvable:?}");
    assert!(unresolvable[0].ends_with(&format!("line 20: {inc_str} with a path this reader cannot resolve")), "{unresolvable:?}");
    let found: Vec<String> = found_files
        .iter()
        .flat_map(|f| {
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
            vendor_mentions(&text).into_iter().map(move |l| format!("{}: {l}", f.file_name().unwrap().to_string_lossy()))
        })
        .collect();
    let mut reported: Vec<&str> = named.iter().copied().filter(|n| *n != "q.txt").collect();
    reported.push("f.txt");
    assert_eq!(found.len(), reported.len(), "{found:?}");
    for file in reported {
        assert!(found.iter().any(|h| h.starts_with(&format!("{file}: "))), "{file} not reported: {found:?}");
    }
    // `include!` does not match inside `include_str!`; an attribute without `path` is nothing.
    let plain: Vec<Included> = included_files(&manifest_dir.join("src/x.rs"), "#[derive(Debug)]\nstruct S;\n// an include_str! in a comment with no argument\n", &manifest_dir);
    assert_eq!(plain, Vec::<Included>::new());
    let unclosed = included_files(&manifest_dir.join("src/x.rs"), &format!("const X: &str = {inc_str}(\"never closed;\n"), &manifest_dir);
    assert_eq!(unclosed, vec![Included::Unresolvable { line: 1, what: inc_str.clone() }]);
    // Inside a longer word, or in a comment with no argument, a marker is nothing.
    let none = included_files(&manifest_dir.join("src/x.rs"), &format!("fn my_{inc_str}() {{}}\n// an {inc_str} without an argument\nfn f() {{ let x = \"{inc_str}\"; }}\n"), &manifest_dir);
    assert_eq!(none, Vec::<Included>::new());
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
