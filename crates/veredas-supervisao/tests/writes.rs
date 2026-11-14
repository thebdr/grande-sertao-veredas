//! C-054: writes, gated. The binding offers a write; the page posts it to the session's portal
//! write route through supervisão's address, where portal's switch answers: refused 403 while
//! the switch is off with nothing reaching the plant, landing at the bench once the project arms
//! it (or a shell arms it in process). What the page sends is the binding's action, the one
//! pressed: its value and its source, asked and posted as the binding says, every offered button
//! off while the switch is off. Supervisão has no write path of its own: no route of its own takes
//! a POST, and its sources name of core and portal only what it reads with (an allow-list), no
//! socket but its own listener, no file written.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use common::browser::Browser;
use common::*;
use serde_json::json;
use veredas_maquinar::bench::Bench;
use veredas_maquinar::opc::render_value;
use veredas_supervisao::{Host, config};

fn release(bench: &Bench) -> String {
    render_value(&bench.server("plc").unwrap().get("release3"))
}

fn hold(bench: &Bench) -> String {
    render_value(&bench.server("aux").unwrap().get("hold3"))
}

/// A second controller, `aux`, with a tag of its own to write.
const AUX: &str = "\n[[opc]]\nname = \"aux\"\nport = 0\ntags = [\n  { id = \"hold3\", type = \"int\", value = \"0\", access = \"read-write\" },\n]\n";

/// The example with outlet 3 offering two writes no test meets by accident: a release of 7 to the
/// controller it reads, and a hold of 3 to `aux`, a source it does not read.
fn with_aux(mut ex: Example, bench: &Bench) -> Example {
    let aux = bench.server("aux").unwrap();
    ex.project.push_str(&format!(
        "\n[[opc]]\nsource_id = \"aux\"\nendpoint_url = \"{}\"\npoll_ms = 50\nrules = [\n  {{ name = \"aux_main\", tags = [\n    {{ node_id = \"ns={};s=hold3\", series = \"hold3\", type = \"int\", writable = true }},\n  ] }},\n]\n",
        aux.endpoint_url, aux.namespace
    ));
    let plc = bench.server("plc").unwrap().namespace;
    let one = format!("actions = [ {{ label = \"Release\", source = \"plc\", tag = \"ns={plc};s=release3\", value = \"1\" }} ]");
    assert!(ex.binding.contains(&one), "{}", ex.binding);
    let two = format!("actions = [ {{ label = \"Release\", source = \"plc\", tag = \"ns={plc};s=release3\", value = \"7\" }}, {{ label = \"Hold\", source = \"aux\", tag = \"ns={};s=hold3\", value = \"3\" }} ]", aux.namespace);
    ex.binding = ex.binding.replacen(&one, &two, 1);
    // Outlet 2, earlier in the file, offers a write of the same label as outlet 3's first, on
    // another controller with another value: a page that looks an action up by its label anywhere
    // in the binding posts this one for outlet 3's.
    assert!(ex.binding.contains("id = \"outlet-2\"\n"), "{}", ex.binding);
    ex.binding = ex.binding.replacen("id = \"outlet-2\"\n", &format!("id = \"outlet-2\"\nactions = [ {{ label = \"Release\", source = \"aux\", tag = \"ns={};s=hold3\", value = \"5\" }} ]\n", aux.namespace), 1);
    ex
}

/// What supervisão's sources may name of core and of portal: the reading and serving it does,
/// nothing that writes, dials or parses a project from text. A path is written whole, as from the
/// crate's root.
const ALLOWED: &[&str] = &[
    "veredas_core::SeriesType", "veredas_core::SeriesRef", "veredas_core::SourceError", "veredas_core::SourceId", "veredas_core::now_unix_ms",
    "veredas_core::schema::check", "veredas_core::schema::HeaderError",
    "veredas_portal::CaptureRule", "veredas_portal::project", "veredas_portal::project::Project", "veredas_portal::project::load_file",
    "veredas_portal::opc::parse_type", "veredas_portal::opc::compile", "veredas_portal::opc::table_of", "veredas_portal::channel::check",
    "veredas_portal::session::HealthState", "veredas_portal::session::Live", "veredas_portal::session::Session",
    "veredas_portal::serve::store_now_ms", "veredas_portal::serve::router", "veredas_portal::serve::lttb",
    "veredas_portal::store::MAX_PAGE", "veredas_portal::store::RowsQuery",
    // An enum's variants and a type's functions, each by name: data read, a value built, a
    // session started, a series thinned.
    "veredas_core::SeriesType::Int", "veredas_core::SeriesType::Float", "veredas_core::SeriesType::Text", "veredas_core::SeriesType::Time",
    "veredas_core::SeriesRef::new", "veredas_core::schema::HeaderError::NotToml", "veredas_portal::CaptureRule::new",
    "veredas_portal::session::HealthState::Waiting", "veredas_portal::session::HealthState::Live", "veredas_portal::session::HealthState::Idle",
    "veredas_portal::session::HealthState::Retrying", "veredas_portal::session::HealthState::Ended", "veredas_portal::session::HealthState::Failed",
    "veredas_portal::session::Session::start", "veredas_portal::serve::lttb::downsample",
    // Of std's and tokio's files, programs, sockets and process: reading files, stdin, its one
    // listener, an address, an error, its own exit. Nothing that writes, runs or dials.
    "std::fs::read_to_string", "std::fs::read_dir", "std::fs::read", "std::io", "std::io::Error", "std::io::ErrorKind::Interrupted", "std::io::Result", "std::io::stdin",
    "std::io::BufRead", "std::net::SocketAddr", "std::process::exit", "tokio::net::TcpListener::bind",
];

/// What every named path is judged against `ALLOWED` under: core and portal whole, and the modules
/// of std and tokio that reach files, programs, sockets and the process.
const JUDGED: &[&str] = &["veredas_core", "veredas_portal", "std::fs", "std::io", "std::process", "std::net", "std::os", "tokio::fs", "tokio::io", "tokio::process", "tokio::net"];

/// The crate roots whose paths are judged; every other root the sources can name is reviewed whole
/// in `REVIEWED_ROOTS`.
const JUDGED_ROOTS: &[&str] = &["std", "tokio", "veredas_core", "veredas_portal"];

/// The crate roots supervisão's sources may name without a path being judged, each reviewed: the
/// server (`axum`, `tokio_stream`: it serves, it dials nothing), data (`serde`, `serde_json`,
/// `toml`, `chrono`, `roxmltree`, `thiserror`: no file, no socket, no program), and Rust's `core`.
const REVIEWED_ROOTS: &[&str] = &["axum", "chrono", "core", "roxmltree", "serde", "serde_json", "thiserror", "tokio_stream", "toml"];

/// Rust's keywords, strict and reserved, every edition's (the Reference's list): none is a path's
/// segment but `crate`, `self`, `Self` and `super`, so `if ::std::fs::write(..)` names `std::fs::write`.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "gen", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield",
];

fn judged(p: &str) -> bool {
    JUDGED.iter().any(|j| p == *j || p.strip_prefix(j).is_some_and(|r| r.starts_with("::")))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Word,
    Raw,
    Punct,
    Lit,
}

/// A token of a source as the Rust lexer reads it: comments are none (a doc comment is an
/// attribute's string), a raw identifier is its name (`r#fs` is `fs`, its kind saying it was raw),
/// `::` is one token and a group's delimiters are tokens of their own.
#[derive(Clone, Debug, PartialEq)]
struct Tok {
    kind: Kind,
    text: String,
}

impl Tok {
    /// A path's segment: an identifier, raw or not, that is no keyword (`crate`, `self`, `Self`
    /// and `super` are segments).
    fn segment(&self) -> bool {
        self.kind == Kind::Raw || (self.kind == Kind::Word && !KEYWORDS.contains(&self.text.as_str()))
    }
    fn is(&self, punct: &str) -> bool {
        self.kind == Kind::Punct && self.text == punct
    }
    fn keyword(&self, word: &str) -> bool {
        self.kind == Kind::Word && self.text == word
    }
}

fn lex(text: &str) -> Vec<Tok> {
    fn walk(stream: proc_macro2::TokenStream, out: &mut Vec<Tok>) {
        let trees: Vec<proc_macro2::TokenTree> = stream.into_iter().collect();
        let mut i = 0;
        while i < trees.len() {
            match &trees[i] {
                proc_macro2::TokenTree::Ident(id) => {
                    let s = id.to_string();
                    out.push(match s.strip_prefix("r#") {
                        Some(name) => Tok { kind: Kind::Raw, text: name.to_string() },
                        None => Tok { kind: Kind::Word, text: s },
                    });
                }
                proc_macro2::TokenTree::Punct(p) => {
                    let pair = p.as_char() == ':' && p.spacing() == proc_macro2::Spacing::Joint && matches!(trees.get(i + 1), Some(proc_macro2::TokenTree::Punct(q)) if q.as_char() == ':');
                    out.push(Tok { kind: Kind::Punct, text: if pair { "::".to_string() } else { p.as_char().to_string() } });
                    if pair {
                        i += 1;
                    }
                }
                proc_macro2::TokenTree::Literal(l) => out.push(Tok { kind: Kind::Lit, text: l.to_string() }),
                proc_macro2::TokenTree::Group(g) => {
                    let (open, close) = match g.delimiter() {
                        proc_macro2::Delimiter::Parenthesis => ("(", ")"),
                        proc_macro2::Delimiter::Brace => ("{", "}"),
                        proc_macro2::Delimiter::Bracket => ("[", "]"),
                        proc_macro2::Delimiter::None => ("", ""),
                    };
                    if !open.is_empty() {
                        out.push(Tok { kind: Kind::Punct, text: open.to_string() });
                    }
                    walk(g.stream(), out);
                    if !close.is_empty() {
                        out.push(Tok { kind: Kind::Punct, text: close.to_string() });
                    }
                }
            }
            i += 1;
        }
    }
    let stream: proc_macro2::TokenStream = text.parse().unwrap_or_else(|e| panic!("the Rust lexer refused a source: {e:?}: {}", text.chars().take(80).collect::<String>()));
    let mut out = Vec::new();
    walk(stream, &mut out);
    out
}

/// One name a `use` declaration brings in: the path written whole (`std::io` for
/// `use std::{io::{self}}`), the name it takes here, and whether it was renamed or a glob.
#[derive(Clone, Debug, PartialEq)]
struct Import {
    path: String,
    name: String,
    renamed: bool,
    glob: bool,
}

/// One `use` tree read from `toks[*i]` under `prefix`: a leading `::`, segments joined by `::`, then
/// a group `{..}`, a glob, `self`, or a name with or without `as`; a tree opening with `{` is a group.
fn use_tree(toks: &[Tok], i: &mut usize, prefix: &[String], out: &mut Vec<Import>) -> Result<(), String> {
    let mut path = prefix.to_vec();
    if toks.get(*i).is_some_and(|t| t.is("::")) {
        *i += 1;
    }
    loop {
        let t = toks.get(*i).ok_or("a `use` cut short")?;
        if t.is("{") {
            *i += 1;
            loop {
                if toks.get(*i).is_some_and(|t| t.is("}")) {
                    *i += 1;
                    return Ok(());
                }
                use_tree(toks, i, &path, out)?;
                match toks.get(*i) {
                    Some(t) if t.is(",") => *i += 1,
                    Some(t) if t.is("}") => {}
                    other => return Err(format!("a `use` group read to {other:?}")),
                }
            }
        }
        if t.is("*") {
            *i += 1;
            out.push(Import { path: [path.clone(), vec!["*".to_string()]].concat().join("::"), name: "*".to_string(), renamed: false, glob: true });
            return Ok(());
        }
        if !t.segment() {
            return Err(format!("a `use` tree read to `{}`", t.text));
        }
        *i += 1;
        // `self` inside a group names the group's own path; first, it is the current module.
        if !(t.keyword("self") && !path.is_empty()) {
            path.push(t.text.clone());
        }
        if toks.get(*i).is_some_and(|t| t.is("::")) {
            *i += 1;
            continue;
        }
        let (name, renamed) = if toks.get(*i).is_some_and(|t| t.keyword("as")) {
            let alias = toks.get(*i + 1).filter(|t| t.segment() || t.keyword("_") || t.text == "_").ok_or("a `use` renamed to nothing")?.text.clone();
            *i += 2;
            (alias, true)
        } else {
            (path.last().cloned().unwrap_or_default(), false)
        };
        out.push(Import { path: path.join("::"), name, renamed, glob: false });
        return Ok(());
    }
}

/// Every `use` declaration of a source read by its tree, wherever it stands and however it opens
/// (`use ::std::..`, `use {std::..}`), with the span of its tokens so the inline reading passes over
/// them. A `use` this reading cannot follow is an error, not passed over.
fn imports(toks: &[Tok]) -> Result<(Vec<Import>, Vec<(usize, usize)>), String> {
    let (mut out, mut spans) = (Vec::new(), Vec::new());
    for (at, t) in toks.iter().enumerate() {
        if !t.keyword("use") {
            continue;
        }
        let mut i = at + 1;
        use_tree(toks, &mut i, &[], &mut out)?;
        if !toks.get(i).is_some_and(|t| t.is(";")) {
            return Err(format!("a `use` not ended by `;`: {:?}", toks.get(i)));
        }
        spans.push((at, i));
    }
    Ok((out, spans))
}

/// Every path a source writes outside its `use` declarations, by its segments: a segment, then `::`
/// and a segment, as long as they go (a leading `::` dropped, a keyword before it none of the path;
/// a turbofish, a group or a glob ends it).
fn runs(toks: &[Tok], skip: &[(usize, usize)]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if let Some(&(_, end)) = skip.iter().find(|(a, _)| *a == i) {
            i = end + 1;
            continue;
        }
        if toks[i].segment() {
            let mut path = vec![toks[i].text.clone()];
            let mut j = i + 1;
            while j + 1 < toks.len() && toks[j].is("::") && toks[j + 1].segment() {
                path.push(toks[j + 1].text.clone());
                j += 2;
            }
            out.push(path);
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// Every judged crate root a source names that the reading cannot follow: as a path's first segment
/// not followed at once by `::` and a segment or a group (renamed with `as`, passed to a macro
/// whole, spliced by one).
fn unfollowed_roots(toks: &[Tok]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let first = !(i >= 2 && toks[i - 1].is("::") && toks[i - 2].segment());
        if !(t.segment() && first && JUDGED_ROOTS.contains(&t.text.as_str())) {
            continue;
        }
        let follows = toks.get(i + 1).is_some_and(|n| n.is("::")) && toks.get(i + 2).is_some_and(|n| n.segment() || n.is("{"));
        if !follows {
            out.push(toks[i..(i + 3).min(toks.len())].iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" "));
        }
    }
    out
}

/// A path read through the names a source imports: its first segment, a name the file imports,
/// read as the imported path; a path through `crate`, `self` or `super` read as well from each of
/// its segments that names, anywhere in the crate, an import of core's, portal's, std's or tokio's
/// (a re-export followed by its name, not by the compiler's resolution). The path itself is kept.
fn expand(path: &str, local: &BTreeMap<String, String>, wide: &BTreeMap<String, BTreeSet<String>>, depth: usize) -> BTreeSet<String> {
    let mut out = BTreeSet::from([path.to_string()]);
    if depth > 8 {
        return out;
    }
    let segs: Vec<&str> = path.split("::").collect();
    if let Some(full) = local.get(segs[0]).filter(|f| f.split("::").next() != Some(segs[0])) {
        let whole = [vec![full.as_str()], segs[1..].to_vec()].concat().join("::");
        out.extend(expand(&whole, local, wide, depth + 1));
    }
    if ["crate", "self", "super"].contains(&segs[0]) {
        for k in 1..segs.len() {
            for t in wide.get(segs[k]).into_iter().flatten() {
                let whole = [vec![t.as_str()], segs[k + 1..].to_vec()].concat().join("::");
                out.extend(expand(&whole, local, wide, depth + 1));
            }
        }
    }
    out
}

/// The reading of a crate's sources, each `(file, text)`: every path of core's, portal's, std's or
/// tokio's they name, imported or written inline, read through the names they import; and what the
/// reading refuses to follow (a glob, a judged path renamed, a root not followed, a `use` it cannot read).
struct Reading {
    named: Vec<(String, String)>,
    refused: Vec<String>,
}

fn read_sources(files: &[(String, String)]) -> Reading {
    let mut refused = Vec::new();
    let mut parsed = Vec::new();
    for (file, text) in files {
        let toks = lex(text);
        let (found, spans) = imports(&toks).unwrap_or_else(|e| {
            refused.push(format!("{file}: {e}"));
            (Vec::new(), Vec::new())
        });
        refused.extend(unfollowed_roots(&toks).into_iter().map(|r| format!("{file}: a root not followed: `{r}`")));
        let local: BTreeMap<String, String> = found.iter().filter(|i| !i.glob).map(|i| (i.name.clone(), i.path.clone())).collect();
        let inline = runs(&toks, &spans);
        parsed.push((file.clone(), found, local, inline));
    }
    // Every name imported anywhere from core, portal, std or tokio, followed until it holds still.
    let mut wide: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    loop {
        let before = wide.clone();
        for (_, found, local, _) in &parsed {
            for i in found.iter().filter(|i| !i.glob) {
                for e in expand(&i.path, local, &wide, 0).into_iter().filter(|e| judged(e)) {
                    wide.entry(i.name.clone()).or_default().insert(e);
                }
            }
        }
        if wide == before {
            break;
        }
    }
    let mut named = Vec::new();
    for (file, found, local, inline) in &parsed {
        for i in found {
            if i.glob {
                refused.push(format!("{file}: `{}` taken whole", i.path));
                continue;
            }
            let all = expand(&i.path, local, &wide, 0);
            if i.renamed && all.iter().any(|e| judged(e)) {
                refused.push(format!("{file}: `{}` renamed `{}`", i.path, i.name));
            }
            named.extend(all.into_iter().map(|e| (file.clone(), e)));
        }
        for r in inline.iter().filter(|r| r.len() >= 2) {
            named.extend(expand(&r.join("::"), local, &wide, 0).into_iter().map(|e| (file.clone(), e)));
        }
    }
    Reading { named, refused }
}

/// Whether a source's tokens hold `word`'s tokens in a row, `word` written with a space between
/// each (a name matched whole, never inside a longer one).
fn holds(toks: &[Tok], word: &str) -> bool {
    let w: Vec<&str> = word.split_whitespace().collect();
    toks.windows(w.len()).any(|win| win.iter().zip(&w).all(|(t, s)| t.kind != Kind::Lit && t.text == *s))
}

/// Every Rust file under `dir`, however deep.
/// The names of portal's members any of supervisão's tokens holds, reviewed: each a read (a query,
/// a field, a listing, a window's bounds), the session's own start, stop, live state and
/// subscription, a path already judged by the reading above (`load_file`, `compile`, `router`,
/// `downsample`, ...), or a name portal shares with std, tokio, a crate or supervisão's own code
/// (`send`, `spawn`, `new`, `format`, `list`, `of`, `width`, ...), which this reading cannot tell apart.
const PORTAL_MEMBERS: &[&str] = &[
    "as_str", "bytes", "channels", "check", "columns", "compile", "count", "data_type", "dir", "downsample", "error", "extract", "feed", "file", "file_name", "files", "format", "fresh", "from",
    "from_ms", "get", "health", "id", "idx", "index_ms", "key", "keyed", "keys", "kind", "last_index_ms", "last_value", "len", "limit", "line", "list", "live", "load", "load_file",
    "logs", "message", "name", "names", "new", "of", "offset", "opc", "parse_type", "path", "pattern", "points", "query_numeric", "query_rows", "remove", "router", "rows", "rules",
    "samples", "scope", "send", "series", "serve", "server", "session_config", "snapshot", "source", "source_id", "sources", "spawn", "spec", "start", "state", "status", "stop",
    "stopped", "store", "store_now_ms", "subscribe", "table", "table_of", "tables", "tag", "tags", "text", "to", "to_ms", "toml", "total", "ty", "types", "value", "values",
    "width", "writable", "writes_armed", "y",
];

/// Every public function and public field portal declares, by name, read from its own sources as
/// the Rust lexer reads them, however they are spelled across lines: a `pub` (not `pub(..)`), then
/// any of `const`, `async`, `unsafe` and `extern "abi"`, then `fn` NAME; a `pub` NAME `:` (a field);
/// and every `fn` NAME in a `pub trait`'s body (a method its values answer to).
fn portal_members() -> BTreeSet<String> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../veredas-portal/src");
    let mut out = BTreeSet::new();
    for path in rust_files(&src) {
        out.extend(public_members(&lex(&std::fs::read_to_string(&path).unwrap())));
    }
    out
}

fn public_members(toks: &[Tok]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for i in 0..toks.len() {
        if !toks[i].keyword("pub") || toks.get(i + 1).is_some_and(|t| t.is("(")) {
            continue;
        }
        let mut j = i + 1;
        while toks.get(j).is_some_and(|t| t.keyword("const") || t.keyword("async") || t.keyword("unsafe") || t.keyword("extern") || t.kind == Kind::Lit) {
            j += 1;
        }
        let qualified = j > i + 1;
        match toks.get(j) {
            Some(t) if t.keyword("fn") => out.extend(toks.get(j + 1).filter(|n| n.segment()).map(|n| n.text.clone())),
            Some(t) if !qualified && t.segment() && toks.get(j + 1).is_some_and(|n| n.is(":")) => {
                out.insert(t.text.clone());
            }
            Some(t) if t.keyword("trait") => {
                let Some(open) = (j..toks.len()).find(|k| toks[*k].is("{")) else { continue };
                let mut depth = 0;
                for k in open..toks.len() {
                    if toks[k].is("{") {
                        depth += 1;
                    } else if toks[k].is("}") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if depth == 1 && toks[k].keyword("fn") {
                        out.extend(toks.get(k + 1).filter(|n| n.segment()).map(|n| n.text.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Every name a source's tokens hold as a path's segment (a method called or a field read after a
/// `.`, a function through a qualified path, a field in a struct's pattern, a variable): every way
/// a name of portal's could be reached, and supervisão's own names alike, which this reading cannot
/// tell from them.
fn names_held(toks: &[Tok]) -> BTreeSet<String> {
    toks.iter().filter(|t| t.segment()).map(|t| t.text.clone()).collect()
}

fn rust_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn a_write_is_refused_on_the_wire_while_the_switch_is_off_and_lands_once_it_is_armed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant_with(&dir.path().join("bench"), AUX);
    let ns = bench.server("plc").unwrap().namespace;
    let aux_ns = bench.server("aux").unwrap().namespace;
    let path = write_example(dir.path(), &with_aux(live_example(&bench, false), &bench));
    // The project supervisão loads is portal's reading of the file, every OPC tag and whether a
    // rule marks it writable as the file says: supervisão widens no whitelist on its way in.
    let tags_of = |opc: &[veredas_portal::opc::OpcSourceConfig]| opc.iter().flat_map(|s| s.rules.iter().flat_map(move |r| r.tags.iter().map(move |t| (s.source_id.clone(), t.node_id.clone(), t.writable)))).collect::<Vec<_>>();
    let file_project = veredas_portal::project::load_file(&dir.path().join("loop/project.toml")).unwrap();
    let loaded = config::load_file(&path).unwrap();
    assert_eq!(tags_of(&loaded.sessions[0].project.opc), tags_of(&file_project.opc), "the project as supervisão loads it");
    let unwritable: Vec<(String, String)> = tags_of(&file_project.opc).into_iter().filter(|(_, _, w)| !w).map(|(s, n, _)| (s, n)).collect();
    assert!(unwritable.len() >= 4 && unwritable.iter().all(|(s, _)| s == "plc"), "{unwritable:?}");
    let host = Host::start(loaded).unwrap();
    let addr = host.addr();

    // The writes the binding offers, as the page reads them, and the switch the feed shows.
    let (_, b) = get(addr, "/s/loop/binding");
    let actions = b["objects"].as_array().unwrap().iter().find(|o| o["id"] == "outlet-3").unwrap()["actions"].clone();
    assert_eq!(actions, json!([
        { "label": "Release", "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "7" },
        { "label": "Hold", "source": "aux", "tag": format!("ns={aux_ns};s=hold3"), "value": "3" },
    ]));
    let action = actions[0].clone();
    assert!(wait(Duration::from_secs(20), || {
        let s = get(addr, "/s/loop/objects.json").1;
        s["session"]["writes_armed"] == json!(false) && s["session"]["health"]["plc"]["state"] == json!("live") && s["session"]["health"]["aux"]["state"] == json!("live")
    }));
    let body = json!({ "source": action["source"], "tag": action["tag"], "value": action["value"] });

    // Off: refused by the switch, on the wire, through supervisão's address; nothing at the plant.
    assert_eq!(release(&bench), "0");
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body), (403, json!({ "error": "writes are turned off for this project" })));
    assert_eq!(post(addr, "/s/loop/portal/api/channels/send", &json!({ "source": "plc", "message": "x" })), (403, json!({ "error": "writes are turned off for this project" })));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0", "nothing reached the plant");

    // Armed in process by the shell that runs the session: the same post lands, seen by the
    // plant; the feed shows the switch.
    host.session("loop").unwrap().set_writes_armed(true);
    assert!(wait(Duration::from_secs(10), || get(addr, "/s/loop/objects.json").1["session"]["writes_armed"] == json!(true)));
    let (status, v) = post(addr, "/s/loop/portal/api/opc/write", &body);
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(10), || release(&bench) == "7"), "{}", release(&bench));
    // Armed, the whitelist still holds: every tag of every rule the file leaves unwritable (portal's
    // own reading of it, on both sources), and a node no rule names, refused by name.
    let nowhere = ("plc".to_string(), format!("ns={ns};s=nowhere"));
    for (source, tag) in unwritable.iter().chain([&nowhere]) {
        assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &json!({ "source": source, "tag": tag, "value": "9" })), (403, json!({ "error": format!("`{source}` / `{tag}` is not a writable tag in this session") })), "{source} {tag}");
    }
    // Disarmed again: refused again, the plant keeps what it had.
    host.session("loop").unwrap().set_writes_armed(false);
    bench.server("plc").unwrap().set("release3", 0);
    assert!(wait(Duration::from_secs(10), || release(&bench) == "0"));
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body).0, 403);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0");

    // Supervisão's code reaches no write handle and no switch: the library and the executable
    // name none of portal's write paths, and the page posts to portal's route alone.
    // Nor does it decide the switch's start position (portal keeps it crate-private), start a
    // worker of its own, or hide code from this reading (no file pulled in by path, no build
    // script).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("build.rs").exists() && !std::fs::read_to_string(root.join("Cargo.toml")).unwrap().contains("build ="), "no build script writes code this reading cannot see");
    // No foreign function either (the C runtime's `system` and `fopen` name no path this reading
    // judges): both crate roots forbid unsafe code, so the compiler refuses one.
    for crate_root in ["src/lib.rs", "src/main.rs"] {
        let text = std::fs::read_to_string(root.join(crate_root)).unwrap();
        assert!(text.lines().any(|l| l.trim() == "#![forbid(unsafe_code)]"), "{crate_root} forbids unsafe code");
    }
    let files = rust_files(&root.join("src"));
    assert!(files.len() >= 8, "{files:?}");
    // Every source read as the Rust lexer reads it: a raw identifier is its name, a comment is
    // nothing, so no spelling of a word hides it.
    let sources: Vec<(String, String)> = files.iter().map(|p| (p.file_name().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(p).unwrap())).collect();
    let mut listeners = 0;
    for (file, text) in &sources {
        let toks = lex(text);
        for word in [
            "write_tags", "opc_handle", "channel_handle", "set_writes_armed", "WriteGate", ". gate", "live . opc", "live . channels",
            "writes_armed =", "writes_armed | =", "writes_armed :", "channel :: spawn", "worker :: spawn", "include !", "# [ path",
            // No socket but its own listener, no file written, no program run.
            "TcpStream", "TcpSocket", "UdpSocket", "UnixStream", "UnixListener", "UnixDatagram", "fs :: write", "File :: create", "OpenOptions", "fs :: copy", "fs :: rename", "Command :: new",
            // No crate brought in by `extern`, no macro of its own (it could splice a path), no
            // foreign function and no unsafe code.
            "extern", "macro_rules !", "unsafe",
            // None of the methods of portal's store that write a file (reached as methods of the
            // session's `store`, they name no path this reading judges).
            "save_to", "export_table_csv", "export_all_csv", "append_wide", "append_samples",
        ] {
            assert!(!holds(&toks, word), "{file} names `{word}`");
        }
        for route in ["/api/opc/write", "/api/channels/send"] {
            assert!(!toks.iter().any(|t| t.kind == Kind::Lit && t.text.contains(route)), "{file} names `{route}`");
        }
        listeners += toks.iter().filter(|t| t.segment() && t.text == "TcpListener").count();
    }
    assert_eq!(listeners, 1, "one listener, the host's own");
    let reading = read_sources(&sources);
    assert!(reading.refused.is_empty(), "the reading cannot follow: {:?}", reading.refused);
    let named = reading.named;
    // Every crate root the sources can name: the keys of the manifest's dependency tables (a
    // target's included; a dependency renamed is named by its key) and Rust's `std` and `core`
    // (`alloc` and every other would take an `extern crate`, refused above). Each is judged path by
    // path or reviewed whole; a root neither is refused.
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    let mut roots: BTreeSet<String> = ["std", "core"].map(String::from).into();
    let targets = manifest.get("target").and_then(|t| t.as_table()).into_iter().flat_map(|t| t.values().map(|v| v.get("dependencies")));
    for table in std::iter::once(manifest.get("dependencies")).chain(targets).flatten() {
        roots.extend(table.as_table().unwrap().keys().map(|k| k.replace('-', "_")));
    }
    let known: BTreeSet<String> = JUDGED_ROOTS.iter().chain(REVIEWED_ROOTS).map(|s| s.to_string()).collect();
    assert_eq!(roots, known, "every crate root the sources can name, judged or reviewed");
    // What supervisão reaches of portal through the values it holds (a session, its live state, its
    // store, a project) names no path; it is read by name instead, against portal's own list of
    // public functions and fields (read by the lexer from portal's sources): every name of that
    // list any of supervisão's tokens holds (after a `.`, through a qualified path, in a struct's
    // pattern, or as a name of supervisão's own that this reading cannot tell from it) is one
    // reviewed here, and any other (a writer such as `validate_definition`, `save_to`,
    // `set_writes_armed` or `list_tables` among them) is refused. A name portal shares with a std
    // or tokio member supervisão calls, or with a name of supervisão's own, is judged by its name
    // alone (the clause states the limit).
    let members = portal_members();
    assert!(["validate_definition", "save_to", "set_writes_armed", "query_rows", "store", "list_tables"].iter().all(|m| members.contains(*m)), "portal's list read: {} names", members.len());
    let mut reached = BTreeSet::new();
    for (_, text) in &sources {
        reached.extend(names_held(&lex(text)).into_iter().filter(|n| members.contains(n)));
    }
    let unreviewed: Vec<&String> = reached.iter().filter(|n| !PORTAL_MEMBERS.contains(&n.as_str())).collect();
    assert!(unreviewed.is_empty(), "portal members reached beyond the reviewed ones: {unreviewed:?}");
    let stale: Vec<&&str> = PORTAL_MEMBERS.iter().filter(|n| !reached.contains(**n)).collect();
    assert!(stale.is_empty(), "reviewed names no longer reached: {stale:?}");
    // The reading itself: every way a name is reached, a raw one by its name; portal's list read
    // whatever the spelling (a `pub` alone on its line, a one-line struct, a trait's methods).
    let held = names_held(&lex("live.store.query_rows(t, &q); let n = 1.5; <Live>::opc_spec(&l, \"plc\"); let Live { frames, vitals, .. } = &*l; v.r#list_tables(); w.0"));
    assert!(["store", "query_rows", "opc_spec", "frames", "vitals", "list_tables"].iter().all(|n| held.contains(*n)), "{held:?}");
    let spelled = public_members(&lex("pub\nfn list_tables(&self) {}\npub struct S { pub a: u8, #[serde(default)] pub b: u8 }\npub(crate) fn hidden() {}\npub const K: u8 = 1;\npub trait T { fn start(&self); }\npub async unsafe extern \"C\" fn odd() {}"));
    assert_eq!(spelled, ["a", "b", "list_tables", "odd", "start"].into_iter().map(String::from).collect());
    let outside: Vec<&(String, String)> = named.iter().filter(|(_, p)| judged(p) && !ALLOWED.contains(&p.as_str())).collect();
    assert!(outside.is_empty(), "named outside what supervisão reads with: {outside:?}");
    // The reading itself: what the sources name today, every allowed path seen at least once; a
    // `use` tree, an inline path and a module's item each read whole.
    for p in ALLOWED {
        assert!(named.iter().any(|(_, n)| n == p), "{p} never seen: {named:?}");
    }
    let one = |text: &str| read_sources(&[("probe".to_string(), text.to_string())]);
    let judged_of = |r: Reading| {
        let mut got: Vec<String> = r.named.into_iter().map(|(_, p)| p).filter(|p| judged(p) && !ALLOWED.contains(&p.as_str())).collect();
        got.sort();
        got.dedup();
        got
    };
    let probe = "use veredas_portal::{project::{self, Project}, session::Live};\nlet x = veredas_core::schema::check(t);\nlet p = project::load(t, b);\nuse veredas_core::telegram;";
    let mut got: Vec<String> = one(probe).named.into_iter().map(|(_, p)| p).filter(|p| judged(p)).collect();
    got.sort();
    got.dedup();
    assert_eq!(got, ["veredas_core::schema::check", "veredas_core::telegram", "veredas_portal::project", "veredas_portal::project::Project", "veredas_portal::project::load", "veredas_portal::session::Live"]);
    // A file written or a program run however it is spelled is a path outside: an `OpenOptions`
    // reached through `File::options`, a write through the trait's own path or a qualified one, a
    // module imported whole, a path broken across lines with spaces or a comment inside it, raw
    // identifiers, a path after a keyword, and a module another file imports followed through it.
    let escapes = "let mut f = std::fs::File::options().write(true).open(p)?;\nstd::io::Write::write_all(&mut f, b)?;\nuse std::fs;\nfs::write(p, b);\nlet c = std :: process\n  :: Command::new(\"cmd\");\nstd::r#fs::r#write(p, b);\nlet g = std::/**/fs::rename(a, b);\nif ::std::fs::copy(a, b).is_ok() {}\n<f as std::io::Write>::flush(&mut f);\nuse crate::shared::io;\nio::Write::write_all(&mut f, b);";
    let r = read_sources(&[("probe".to_string(), escapes.to_string()), ("shared".to_string(), "pub(crate) use std::io;".to_string())]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(judged_of(r), ["std::fs", "std::fs::File::options", "std::fs::copy", "std::fs::rename", "std::fs::write", "std::io::Write", "std::io::Write::write_all", "std::process::Command::new"]);
    // And what the reading cannot follow is refused, not passed over: a root renamed or passed to a
    // macro whole or spliced by one, a judged path renamed in a tree however it opens, a glob; a
    // doc comment naming the runtime is no code.
    let hidden = "use std as sd;\nsd::fs::File::options();\nuse {std::{fs as disk}};\nuse ::std::process::{Command as Run};\nuse super::*;\nmacro_rules! sx { ($($r:tt)*) => { std::$($r)* } }\nm!(tokio);\n/// on the current tokio runtime\nfn f() {}";
    assert_eq!(one(hidden).refused, [
        "probe: a root not followed: `std as sd`", "probe: a root not followed: `std :: $`", "probe: a root not followed: `tokio ) ;`",
        "probe: `std::fs` renamed `disk`", "probe: `std::process::Command` renamed `Run`", "probe: `super::*` taken whole",
    ]);
    assert_eq!(one("fn f() -> impl Sized + use<'a> {}").refused, ["probe: a `use` tree read to `<`"]);
    // The banned words are read as tokens too: a raw identifier is its name, a longer name is not.
    let raw = lex("std::r#fs::r#write(p, b); r#Command::r#new(x); let included = 1;");
    assert!(holds(&raw, "fs :: write") && holds(&raw, "Command :: new") && !holds(&raw, "include !"));
    let bundle = veredas_supervisao::web::APP_JS;
    assert_eq!(bundle.matches("/api/opc/write").count(), 1, "the page posts to one route");
    assert!(bundle.contains("/portal/api/opc/write") && !bundle.contains("/api/channels/send"), "the route is portal's own");
    assert_eq!(bundle.matches("method:\"POST\"").count() + bundle.matches("method: \"POST\"").count(), 1, "one POST in the page");

    // Supervisão's own routes take no write: every one its host declares, read from host.rs's
    // tokens (a literal path, at the root before its loop over the sessions or under a session in
    // it, `{index}` read as 0; the session's page by its name, with and without its slash, the two
    // routes it writes from the session's name).
    let host_toks = lex(&std::fs::read_to_string(root.join("src/host.rs")).unwrap());
    let split = (0..host_toks.len()).find(|i| host_toks[*i].keyword("for") && host_toks.get(i + 1).is_some_and(|t| t.text == "app") && host_toks.get(i + 3).is_some_and(|t| t.text == "apps")).expect("the host's loop over its sessions");
    let mut own_routes: Vec<String> = Vec::new();
    let mut written = 0;
    for i in 0..host_toks.len().saturating_sub(3) {
        if !(host_toks[i].is(".") && host_toks[i + 1].text == "route" && host_toks[i + 2].is("(")) {
            continue;
        }
        let arg = &host_toks[i + 3];
        if arg.kind == Kind::Lit {
            let literal = arg.text.trim_matches('"').replace("{index}", "0");
            own_routes.push(if i < split { literal } else { format!("/s/loop{literal}") });
        } else {
            written += 1;
        }
    }
    assert_eq!(written, 2, "the session's page, by its name with and without its slash");
    own_routes.extend(["/s/loop".to_string(), "/s/loop/".to_string()]);
    own_routes.sort();
    let swept = ["/", "/assets/app.js", "/assets/app.css", "/api/sessions", "/s/loop", "/s/loop/", "/s/loop/binding", "/s/loop/objects", "/s/loop/objects.json", "/s/loop/dashboard/0", "/s/loop/layout.svg"];
    let mut sorted = swept.to_vec();
    sorted.sort();
    assert_eq!(own_routes, sorted, "every route of supervisão's own swept");
    // Nor is a route declared any other way: every method axum's `Router` has (its own source, the
    // version cargo resolves) called anywhere in supervisão's sources, counted, against the calls
    // reviewed here (the routes swept above, the two nests: portal's router under `/portal` and a
    // session's own under its name, the one 404, the state); a `route_service`, a `merge`, a
    // `nest_service` or a fallback service added is refused until it is reviewed and swept.
    let meta: serde_json::Value = serde_json::from_slice(&std::process::Command::new(env!("CARGO")).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(root.join("Cargo.toml")).output().unwrap().stdout).unwrap();
    let axum_dir = meta["packages"].as_array().unwrap().iter().filter(|p| p["name"] == "axum").map(|p| std::path::PathBuf::from(p["manifest_path"].as_str().unwrap()).parent().unwrap().to_path_buf()).collect::<Vec<_>>();
    assert_eq!(axum_dir.len(), 1, "one axum");
    let router_src = std::fs::read_to_string(axum_dir[0].join("src/routing/mod.rs")).unwrap();
    let router_methods: BTreeSet<String> = public_members(&lex(&router_src));
    assert!(["route", "route_service", "nest", "nest_service", "merge", "fallback", "fallback_service", "method_not_allowed_fallback"].iter().all(|m| router_methods.contains(*m)), "axum's Router read: {router_methods:?}");
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    let mut nests: Vec<String> = Vec::new();
    for (_, text) in &sources {
        let toks = lex(text);
        for i in 0..toks.len().saturating_sub(2) {
            if toks[i].is(".") && router_methods.contains(&toks[i + 1].text) && (toks[i + 2].is("(") || toks[i + 2].is("::")) {
                *calls.entry(toks[i + 1].text.clone()).or_default() += 1;
                if toks[i + 1].text == "nest" {
                    nests.push(toks[i + 3..(i + 8).min(toks.len())].iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" "));
                }
            }
        }
    }
    let reviewed: BTreeMap<String, usize> = [("fallback", 1), ("nest", 2), ("route", own_routes.len() - 2 + written), ("with_state", 1)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    assert_eq!(calls, reviewed, "every call of axum's Router methods in supervisão's sources, reviewed");
    assert_eq!(nests, ["\"/portal\" , veredas_portal :: serve", "& base , session )"], "portal's router under `/portal`, a session's own under its name");
    for p in swept {
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let (status, _, _) = http(addr, method, p, Some(&body.to_string()));
            assert_eq!(status, 405, "{method} {p}");
        }
        // Nor does a read move the switch (the object stream never ends; it is read elsewhere).
        if p != "/s/loop/objects" {
            let _ = http(addr, "GET", p, None);
        }
    }
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(get(addr, "/s/loop/objects.json").1["session"]["writes_armed"], json!(false), "no route of supervisão's armed the switch");
    assert_eq!(post(addr, "/s/loop/portal/api/opc/write", &body).0, 403);

    // The page itself, run in a headless browser: the offered write is a button, disabled while
    // the switch is off; pressed anyway (the page's own handler, the button re-enabled by hand),
    // its post is refused on the wire and the plant keeps its value; enabled once the switch is
    // armed, pressed and confirmed, it lands at the plant; asked and not confirmed, nothing is
    // sent; the other action pressed, its own value lands at its own controller and the first
    // tag keeps what it had; disarmed, disabled again. Every question asked and every body posted
    // is the binding's action, the one pressed.
    let mut page = Browser::watched(&format!("http://{addr}/s/loop/"));
    page.wait_for(r#"document.getElementById("outlet-3") !== null && document.getElementById("outlet-3").classList.contains("sv-obj")"#, Duration::from_secs(20));
    page.eval(r#"(() => { window.asked = []; window.answer = true; window.confirm = (m) => { window.asked.push(m); return window.answer; }; return true; })()"#);
    page.eval(r#"document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true }))"#);
    const BUTTON: &str = r#"(() => { const b = document.querySelector('.side .actions button[data-action="Release"]'); return b ? { disabled: b.disabled, gate: document.querySelector(".side .gate-note").textContent, note: document.querySelector(".side .write-note").textContent } : null; })()"#;
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.gate !== '' ? b : null; }})()"), Duration::from_secs(20));
    assert_eq!(b, json!({ "disabled": true, "gate": "Writes go through portal. Its write switch is off.", "note": "" }));
    const BUTTONS: &str = r#"[...document.querySelectorAll(".side .actions button")].map((b) => [b.dataset.action, b.disabled])"#;
    assert_eq!(page.eval(BUTTONS), json!([["Release", true], ["Hold", true]]), "both offered, both off while the switch is off");
    page.eval(r#"(() => { window.answer = true; const b = document.querySelector('.side .actions button[data-action="Release"]'); b.click(); return b.disabled; })()"#);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(page.eval(BUTTON)["note"], json!(""), "a disabled button sends nothing");
    page.eval(r#"(() => { const b = document.querySelector('.side .actions button[data-action="Release"]'); b.disabled = false; b.click(); return true; })()"#);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.note !== '' ? b : null; }})()"), Duration::from_secs(10));
    assert_eq!(b["note"], json!("Release: refused, 403: writes are turned off for this project"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(release(&bench), "0", "nothing reached the plant");
    host.session("loop").unwrap().set_writes_armed(true);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && !b.disabled ? b : null; }})()"), Duration::from_secs(10));
    assert_eq!(b["gate"], json!("Writes go through portal. Its write switch is on."));
    assert_eq!(page.eval(BUTTONS), json!([["Release", false], ["Hold", false]]), "both on while it is on");
    page.eval(r#"(() => { window.answer = false; document.querySelector('.side .actions button[data-action="Release"]').click(); return true; })()"#);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((page.eval(BUTTON)["note"].clone(), release(&bench)), (json!("Release: refused, 403: writes are turned off for this project"), "0".to_string()), "not confirmed, nothing sent");
    page.eval(r#"(() => { window.answer = true; document.querySelector('.side .actions button[data-action="Release"]').click(); return true; })()"#);
    let b = page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.note.startsWith('Release: written') ? b : null; }})()"), Duration::from_secs(10));
    assert!(wait(Duration::from_secs(10), || release(&bench) == "7"), "the page's write landed: {b}");
    assert_eq!(hold(&bench), "0", "and nothing else");
    bench.server("plc").unwrap().set("release3", 0);
    assert!(wait(Duration::from_secs(10), || release(&bench) == "0"));
    page.eval(r#"(() => { window.answer = true; document.querySelector('.side .actions button[data-action="Hold"]').click(); return true; })()"#);
    page.wait_for(r#"document.querySelector(".side .write-note").textContent.startsWith("Hold: written")"#, Duration::from_secs(10));
    assert!(wait(Duration::from_secs(10), || hold(&bench) == "3"), "the hold landed at its own controller: {}", hold(&bench));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(release(&bench), "0", "the hold wrote nothing to the release");
    let (release_asked, hold_asked) = (format!("Write 7 to ns={ns};s=release3 of plc?"), format!("Write 3 to ns={aux_ns};s=hold3 of aux?"));
    assert_eq!(page.eval("window.asked"), json!([release_asked, release_asked, release_asked, hold_asked]), "the refused press, the one not confirmed, the one that landed, the hold");
    let sent: Vec<(String, serde_json::Value)> = page.posts().into_iter().map(|(u, b)| (u, serde_json::from_str(&b).unwrap())).collect();
    let to = format!("http://{addr}/s/loop/portal/api/opc/write");
    let (release_body, hold_body) = (json!({ "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "7" }), json!({ "source": "aux", "tag": format!("ns={aux_ns};s=hold3"), "value": "3" }));
    assert_eq!(sent, vec![(to.clone(), release_body.clone()), (to.clone(), release_body), (to, hold_body)], "the refused post, the release, the hold: each the binding's action");
    host.session("loop").unwrap().set_writes_armed(false);
    page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.disabled && b.gate.endsWith('is off.') ? b : null; }})()"), Duration::from_secs(10));
    assert_eq!(page.eval(BUTTONS), json!([["Release", true], ["Hold", true]]), "both off again");
    // Every request the page made for a write went to portal's route under the session.
    let posts = page.eval(r#"performance.getEntriesByType("resource").filter((e) => e.name.includes("/write")).map((e) => new URL(e.name).pathname)"#);
    assert_eq!(posts, json!(["/s/loop/portal/api/opc/write", "/s/loop/portal/api/opc/write", "/s/loop/portal/api/opc/write"]), "the refused post, the release and the hold");
    // Every object the layout draws, selected in turn, armed: its buttons its own actions in their
    // order (none for an object with none); each pressed and confirmed, the question its own, the
    // body posted its own, its value landing at its own tag (outlet 2's "Release" at the aux
    // controller's hold, outlet 3's at the release).
    host.session("loop").unwrap().set_writes_armed(true);
    page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && !b.disabled ? true : null; }})()"), Duration::from_secs(10));
    let (_, b) = get(addr, "/s/loop/binding");
    let drawn: Vec<serde_json::Value> = b["objects"].as_array().unwrap().iter().filter(|o| page.eval(&format!("document.getElementById({})?.classList.contains('sv-obj') ?? false", serde_json::to_string(o["id"].as_str().unwrap()).unwrap())) == json!(true)).cloned().collect();
    let (mut offering, mut silent) = (0, 0);
    for o in &drawn {
        let id = serde_json::to_string(o["id"].as_str().unwrap()).unwrap();
        let own = o["actions"].as_array().unwrap();
        page.eval(&format!("document.getElementById({id}).dispatchEvent(new MouseEvent('click', {{ bubbles: true }}))"));
        page.wait_for(&format!("document.querySelector('.side .sel-title')?.dataset.id === {id} || null"), Duration::from_secs(10));
        let labels: Vec<serde_json::Value> = own.iter().map(|a| a["label"].clone()).collect();
        assert_eq!(page.eval(r#"[...document.querySelectorAll(".side .actions button")].map((b) => b.dataset.action)"#), json!(labels), "{id}: its own actions");
        if own.is_empty() {
            silent += 1;
        } else {
            offering += 1;
        }
        for (i, a) in own.iter().enumerate() {
            bench.server("plc").unwrap().set("release3", 0);
            bench.server("aux").unwrap().set("hold3", 0);
            assert!(wait(Duration::from_secs(10), || release(&bench) == "0" && hold(&bench) == "0"));
            let before = page.posts().len();
            page.eval("window.asked = []; window.answer = true; true");
            page.wait_for(&format!("(() => {{ const b = document.querySelectorAll('.side .actions button')[{i}]; return b && !b.disabled ? true : null; }})()"), Duration::from_secs(10));
            page.eval(&format!("document.querySelector('.side .write-note').textContent = ''; document.querySelectorAll('.side .actions button')[{i}].click(); true"));
            let label = a["label"].as_str().unwrap();
            page.wait_for(&format!("document.querySelector('.side .write-note').textContent.startsWith({}) || null", serde_json::to_string(&format!("{label}: written")).unwrap()), Duration::from_secs(10));
            let (source, tag, value) = (a["source"].as_str().unwrap(), a["tag"].as_str().unwrap(), a["value"].as_str().unwrap());
            assert_eq!(page.eval("window.asked"), json!([format!("Write {value} to {tag} of {source}?")]), "{id} {label}: the question its own");
            let sent = page.posts();
            assert_eq!(sent.len(), before + 1, "{id} {label}: one post");
            let (url, body) = sent.last().unwrap();
            assert_eq!((url.clone(), serde_json::from_str::<serde_json::Value>(body).unwrap()), (format!("http://{addr}/s/loop/portal/api/opc/write"), json!({ "source": source, "tag": tag, "value": value })), "{id} {label}: the body its own");
            let landed = || if source == "plc" { release(&bench) == value && hold(&bench) == "0" } else { hold(&bench) == value && release(&bench) == "0" };
            assert!(wait(Duration::from_secs(10), landed), "{id} {label}: its value at its own tag alone: release {} hold {}", release(&bench), hold(&bench));
        }
    }
    assert!(offering >= 2 && silent >= 1, "objects offering writes ({offering}) and offering none ({silent})");
    // A post that never left (the network refused it) is said, and the button is the switch's again.
    page.eval(r#"document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true })); true"#);
    page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && !b.disabled ? true : null; }})()"), Duration::from_secs(10));
    page.eval(r#"(() => { const f = window.fetch; window.fetch = (...a) => { if (String(a[0]).includes("/api/opc/write") && !window.__failedOnce) { window.__failedOnce = true; return Promise.reject(new TypeError("Failed to fetch")); } return f(...a); }; document.querySelector(".side .write-note").textContent = ""; document.querySelector('.side .actions button[data-action="Release"]').click(); return true; })()"#);
    let note = page.wait_for(r#"document.querySelector(".side .write-note").textContent || null"#, Duration::from_secs(10));
    assert_eq!(note, json!("Release: not sent, TypeError: Failed to fetch"));
    let freed = page.wait_for(&format!("(() => {{ const b = document.querySelector('.side .actions button[data-action=\"Release\"]'); return b && !b.disabled && b.dataset.busy === undefined ? true : null; }})()"), Duration::from_secs(10));
    assert_eq!(freed, json!(true), "the button the switch's again");
    host.session("loop").unwrap().set_writes_armed(false);
    drop(page);
    host.stop();

    // Armed by the project: the post lands from the start.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let path = write_example(dir.path(), &live_example(&bench, true));
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    // Written once the source is connected: before that, the plant never saw it (503).
    assert!(wait(Duration::from_secs(20), || { let s = get(addr, "/s/loop/objects.json").1; s["session"]["writes_armed"] == json!(true) && s["session"]["health"]["plc"]["state"] == json!("live") }));
    let ns = bench.server("plc").unwrap().namespace;
    let (status, v) = post(addr, "/s/loop/portal/api/opc/write", &json!({ "source": "plc", "tag": format!("ns={ns};s=release3"), "value": "1" }));
    assert_eq!((status, v["ok"].clone()), (200, json!(true)), "{v}");
    assert!(wait(Duration::from_secs(10), || release(&bench) == "1"));
    host.stop();

    // Two sessions on one host, hall armed and loop not (hall drawn with the loop's own drawing, so
    // its page offers outlet 3's write too): each page posts its own session's route alone, hall's
    // write landing at hall's plant alone, loop's refused with its plant untouched.
    let dir = tempfile::tempdir().unwrap();
    let (loop_bench, hall_bench) = (plant(&dir.path().join("bench-a")), plant(&dir.path().join("bench-b")));
    let path = two_plants(dir.path(), &loop_bench, &hall_bench);
    let hall_binding = dir.path().join("hall/binding.toml");
    let text = std::fs::read_to_string(&hall_binding).unwrap();
    assert!(text.contains("title = \"The hall\"\n"));
    std::fs::write(&hall_binding, text.replacen("title = \"The hall\"\n", "title = \"The hall\"\nlayout = \"layout.svg\"\n", 1)).unwrap();
    std::fs::copy(dir.path().join("loop/layout.svg"), dir.path().join("hall/layout.svg")).unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    host.session("hall").unwrap().set_writes_armed(true);
    assert!(wait(Duration::from_secs(20), || ["loop", "hall"].iter().all(|s| get(addr, &format!("/s/{s}/objects.json")).1["session"]["health"]["plc"]["state"] == json!("live")) && get(addr, "/s/hall/objects.json").1["session"]["writes_armed"] == json!(true)));
    for (name, plant_of, armed) in [("loop", &loop_bench, false), ("hall", &hall_bench, true)] {
        let mut page = Browser::watched(&format!("http://{addr}/s/{name}/"));
        page.wait_for(r#"document.getElementById("outlet-3") !== null && document.getElementById("outlet-3").classList.contains("sv-obj")"#, Duration::from_secs(20));
        page.eval(r#"(() => { window.confirm = () => true; document.getElementById("outlet-3").dispatchEvent(new MouseEvent("click", { bubbles: true })); return true; })()"#);
        page.wait_for(&format!("(() => {{ const b = {BUTTON}; return b && b.gate.endsWith({}) ? true : null; }})()", if armed { "'is on.'" } else { "'is off.'" }), Duration::from_secs(20));
        page.eval(r#"(() => { const b = document.querySelector('.side .actions button[data-action="Release"]'); b.disabled = false; b.click(); return true; })()"#);
        let note = page.wait_for(r#"document.querySelector(".side .write-note").textContent || null"#, Duration::from_secs(10));
        if armed {
            assert!(note.as_str().unwrap().starts_with("Release: written"), "{name}: {note}");
            assert!(wait(Duration::from_secs(10), || release(plant_of) == "1"), "{name}'s plant");
        } else {
            assert_eq!(note, json!("Release: refused, 403: writes are turned off for this project"), "{name}");
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(release(plant_of), "0", "{name}'s plant untouched");
        }
        let urls: Vec<String> = page.posts().into_iter().map(|(u, _)| u).collect();
        assert_eq!(urls, [format!("http://{addr}/s/{name}/portal/api/opc/write")], "{name}'s page posts its own session's route");
        drop(page);
    }
    assert_eq!(release(&loop_bench), "0", "hall's write reached hall's plant alone");
    host.stop();
}
