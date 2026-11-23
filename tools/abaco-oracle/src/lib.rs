//! Ábaco's oracle: the engine measured against the values Excel cached (`docs/abaco-plan.md`, "The
//! oracle and parity").
//!
//! For each workbook:
//!
//! 1. the sheet XML is read directly (the zip and patterns, no spreadsheet library) for every
//!    formula cell and the value Excel cached beside it: the expected side, independent of the
//!    engine's own reader;
//! 2. that reading is cross-checked against calamine, an independent reader, cell by cell, so a slip
//!    of the oracle's own shows as a number rather than as a false mismatch;
//! 3. the workbook is opened through Ábaco's link policy ([`veredas_abaco::links::open`]), evaluated,
//!    and every formula cell's value compared with the cached one, each difference classified
//!    ([`Class`]);
//! 4. every function the formulas name is listed with whether the engine knows it, probed by
//!    parsing `=NAME(1)` in an empty model;
//! 5. optionally, the model is saved by the fork's writer and the zip parts of input and output
//!    compared (the writer regenerates every part: why a foreign workbook is patched, never saved).
//!
//! Ported from Veredas 1's `abaco-oracle` (2026-11-03), whose numbers this oracle's are compared
//! with. Changed on the way: typed errors; the cross-check streams calamine's cells instead of
//! building a sheet's whole range (a sheet whose declared dimension is enormous made Veredas 1's
//! oracle abort on an allocation of 512 GB); the "open and calculate in Excel" answer is not
//! offered (`docs/questions.md` Q342); the corpus of workbooks Excel saved is selected here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use calamine::DataRef;
use ironcalc_base::cell::CellValue;
use ironcalc_base::types::CellType;
use regex::Regex;
use serde::{Deserialize, Serialize};
use veredas_abaco::links::{LinkChoice, LinkOutcome, LinkPolicy};

/// Why a workbook could not be read on the expected side.
#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a zip package: {source}")]
    Zip {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("{path} has no {part}")]
    MissingPart { path: PathBuf, part: &'static str },
    #[error("{path}: a cell reference `{reference}` that is not one")]
    BadReference { path: PathBuf, reference: String },
    #[error("{path}: {part} is no well-formed XML: {reason}")]
    Xml { path: PathBuf, part: String, reason: String },
}

/// How a workbook that links to others is opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinksMode {
    /// The values the file carries.
    Cached,
    /// The linked workbooks opened from their targets, live values read.
    Linked,
    /// The policy that asks; headless, the books are listed and the cache is chosen.
    Ask,
}

impl LinksMode {
    pub fn parse(s: &str) -> Option<LinksMode> {
        Some(match s {
            "cached" => LinksMode::Cached,
            "linked" => LinksMode::Linked,
            "ask" => LinksMode::Ask,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            LinksMode::Cached => "cached",
            LinksMode::Linked => "linked",
            LinksMode::Ask => "ask",
        }
    }

    fn policy(self) -> LinkPolicy {
        match self {
            LinksMode::Cached => LinkPolicy::AlwaysUseCached,
            LinksMode::Linked => LinkPolicy::AlwaysRecalculateLinked,
            LinksMode::Ask => LinkPolicy::AlwaysAsk,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The expected side: the sheet XML as Excel wrote it
// ---------------------------------------------------------------------------------------------

/// The value Excel cached for a formula cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Expected {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    /// A formula cell with no `<v>` at all: Excel writes the value every formula computed, so this
    /// is a cache it never wrote, and equals nothing the engine computes.
    Missing,
    /// A cache whose type and text Excel never pairs (a boolean that is neither 1 nor 0, an error
    /// none a file holds, a number that is none or not finite, a shared string or an inline one on
    /// a formula, a type Excel writes for no formula): the raw text, equal to nothing.
    Unwritten(String),
}

/// One formula cell of the file, with what Excel cached.
#[derive(Debug, Clone)]
pub struct FormulaCell {
    pub sheet: String,
    pub reference: String,
    pub row: u32,
    pub col: u32,
    /// The formula as written in the file (`_xlfn.`, `_xlpm.` kept); for a shared formula's copy,
    /// the master's text.
    pub formula: String,
    pub shared_copy: bool,
    pub array: bool,
    pub expected: Expected,
}

/// What the expected side read of one workbook.
pub struct RawWorkbook {
    pub sheets: Vec<String>,
    pub formula_cells: Vec<FormulaCell>,
    pub value_cells: usize,
    /// Part name to (uncompressed size, CRC-32), for the save round trip.
    pub parts: BTreeMap<String, (u64, u32)>,
    pub defined_names: usize,
    pub external_links: usize,
    pub external_cached_cells: usize,
}

fn re(pattern: &str) -> Regex {
    // The patterns are this file's constants: a bad one is a bug found by the first run.
    Regex::new(pattern).expect("a constant pattern")
}


/// SpreadsheetML's main namespace, transitional and strict: a sheet's, a workbook's, the
/// metadata's elements.
const MAIN: [&str; 2] = ["http://schemas.openxmlformats.org/spreadsheetml/2006/main", "http://purl.oclc.org/ooxml/spreadsheetml/main"];
/// The namespace of a part's `r:id`, transitional and strict.
const RELATIONSHIP_ID: [&str; 2] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships", "http://purl.oclc.org/ooxml/officeDocument/relationships"];
/// A package's relationships.
const PACKAGE_RELATIONSHIPS: [&str; 1] = ["http://schemas.openxmlformats.org/package/2006/relationships"];
/// Excel's rich data (the rich values, their structures, the metadata's `rvb`).
const RICH_DATA: [&str; 1] = ["http://schemas.microsoft.com/office/spreadsheetml/2017/richdata"];
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// The namespace declarations in scope while a part is read: (depth, prefix, URI), the default
/// namespace's prefix empty.
#[derive(Default)]
struct Scopes(Vec<(usize, String, String)>);

impl Scopes {
    /// The declarations an element at `depth` makes, in force for it and below.
    fn declare(&mut self, depth: usize, attrs: &[(String, String)]) {
        for (q, v) in attrs {
            if q == "xmlns" {
                self.0.push((depth, String::new(), v.clone()));
            } else if let Some(prefix) = q.strip_prefix("xmlns:") {
                self.0.push((depth, prefix.to_string(), v.clone()));
            }
        }
    }

    fn leave(&mut self, depth: usize) {
        self.0.retain(|(d, _, _)| *d < depth);
    }

    fn uri(&self, prefix: &str) -> String {
        if prefix == "xml" {
            return XML_NAMESPACE.to_string();
        }
        self.0.iter().rev().find(|(_, p, _)| p == prefix).map(|(_, _, u)| u.clone()).unwrap_or_default()
    }

    /// An element's (namespace, local name): unprefixed, the default namespace's.
    fn element(&self, qualified: &str) -> (String, String) {
        match qualified.split_once(':') {
            Some((p, l)) => (self.uri(p), l.to_string()),
            None => (self.uri(""), qualified.to_string()),
        }
    }

    /// An attribute's (namespace, local name): unprefixed, no namespace.
    fn attribute(&self, qualified: &str) -> (String, String) {
        match qualified.split_once(':') {
            Some((p, l)) => (self.uri(p), l.to_string()),
            None => (String::new(), qualified.to_string()),
        }
    }
}

/// An element as an XML parser reads it (quick-xml, the version the engine's reader uses): its
/// namespace and local name, its attributes (namespace, local name, value normalised as XML 1.0
/// says; namespace declarations apart), its children, and its text (references resolved, CDATA as
/// the text it is, comments and processing instructions left out, line ends normalised). An
/// element is known by its namespace and its local name, as the engine's reader knows it, never by
/// its prefix.
#[derive(Default, Debug, Clone)]
struct Node {
    ns: String,
    name: String,
    attrs: Vec<(String, String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    /// An attribute in no namespace, by its name.
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(ns, l, _)| ns.is_empty() && l == name).map(|(_, _, v)| v.as_str())
    }

    /// An attribute in one of `nss`, by its local name (`r:id` whatever its prefix).
    fn attr_in(&self, nss: &[&str], local: &str) -> Option<&str> {
        self.attrs.iter().find(|(ns, l, _)| l == local && nss.contains(&ns.as_str())).map(|(_, _, v)| v.as_str())
    }

    fn kids<'a>(&'a self, nss: &'a [&'a str], name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name && nss.contains(&c.ns.as_str()))
    }

    fn descendants<'a>(&'a self, nss: &[&str], name: &str) -> Vec<&'a Node> {
        let mut out = Vec::new();
        let mut stack: Vec<&Node> = vec![self];
        while let Some(n) = stack.pop() {
            if n.name == name && nss.contains(&n.ns.as_str()) {
                out.push(n);
            }
            stack.extend(n.children.iter().rev());
        }
        out
    }
}

/// An element's qualified name and its attributes (qualified name, value normalised as XML 1.0 says).
fn element_of(e: &quick_xml::events::BytesStart) -> Result<(String, Vec<(String, String)>), String> {
    let name = e.name().as_ref().to_string();
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| e.to_string())?;
        let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| e.to_string())?;
        attrs.push((a.key.as_ref().to_string(), value.to_string()));
    }
    Ok((name, attrs))
}

/// An element's namespace, local name and attributes resolved in `scopes`, its declarations made
/// at `depth`.
fn resolved(scopes: &mut Scopes, depth: usize, qualified: &str, attrs: &[(String, String)]) -> (String, String, Vec<(String, String, String)>) {
    scopes.declare(depth, attrs);
    let (ns, name) = scopes.element(qualified);
    let attrs = attrs
        .iter()
        .filter(|(q, _)| q != "xmlns" && !q.starts_with("xmlns:"))
        .map(|(q, v)| {
            let (ns, l) = scopes.attribute(q);
            (ns, l, v.clone())
        })
        .collect();
    (ns, name, attrs)
}

/// The text a reference stands for: a character reference or one of the five predefined entities.
fn reference_text(r: &quick_xml::events::BytesRef) -> Option<String> {
    match r.resolve_char_ref() {
        Ok(Some(c)) => Some(c.to_string()),
        Ok(None) => quick_xml::escape::resolve_predefined_entity(r).map(str::to_string),
        Err(_) => None,
    }
}

/// A small part (the workbook, its relationships, the metadata, the rich values) as a tree.
fn parse_tree(xml: &str) -> Result<Node, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut scopes = Scopes::default();
    let mut root = None;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Eof => break,
            Event::Start(e) => {
                let (q, attrs) = element_of(&e)?;
                let (ns, name, attrs) = resolved(&mut scopes, stack.len(), &q, &attrs);
                stack.push(Node { ns, name, attrs, ..Node::default() });
            }
            Event::Empty(e) => {
                let (q, attrs) = element_of(&e)?;
                let (ns, name, attrs) = resolved(&mut scopes, stack.len(), &q, &attrs);
                scopes.leave(stack.len());
                let node = Node { ns, name, attrs, ..Node::default() };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or("an end tag that closes nothing")?;
                scopes.leave(stack.len());
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::Text(t) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::CData(d) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&d.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(r) => {
                let text = reference_text(&r).ok_or("an entity the part does not declare")?;
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&text);
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("an element not closed".into());
    }
    root.ok_or_else(|| "no element".into())
}

/// One cell of a sheet as an XML parser reads it: its reference, its type and value metadata, its
/// formula (the `f` element's type, shared index and text), every value tag counted with the first
/// one's text, whether an inline string is there, and whether an element sits where the schema has
/// text (inside a formula or a value).
#[derive(Default)]
struct CellXml {
    reference: Option<String>,
    t: Option<String>,
    vm: Option<String>,
    formula: Option<(Option<String>, Option<String>, String)>,
    values: usize,
    value: Option<String>,
    inline: bool,
    broken: bool,
}

/// Every cell of a worksheet part, streamed (a sheet may be large), read by an XML parser: the
/// cells of `sheetData` alone, each element known by its namespace and local name; an element in
/// another namespace (one an Excel reading by Markup Compatibility ignores) is passed over with all
/// it holds.
fn sheet_cells(xml: &str) -> Result<Vec<CellXml>, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut scopes = Scopes::default();
    // the open elements in the main namespace, by local name; `foreign` counts the open elements
    // of another namespace, everything inside them passed over
    let mut path: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut foreign = 0usize;
    let mut cells = Vec::new();
    let mut cell: Option<CellXml> = None;
    // where text goes: the formula's, or the first value's
    #[derive(PartialEq)]
    enum Into {
        Nothing,
        Formula,
        Value,
    }
    let mut into = Into::Nothing;
    let start = |name: &str, attrs: &[(String, String, String)], path: &[String], cell: &mut Option<CellXml>, empty: bool, into: &mut Into| {
        let get = |n: &str| attrs.iter().find(|(ns, l, _)| ns.is_empty() && l == n).map(|(_, _, v)| v.clone());
        let parent = path.last().map(String::as_str);
        if name == "c" && parent == Some("row") && path.iter().any(|p| p == "sheetData") {
            *cell = Some(CellXml { reference: get("r"), t: get("t"), vm: get("vm"), ..CellXml::default() });
            return;
        }
        let Some(c) = cell.as_mut() else { return };
        match (name, parent) {
            ("f", Some("c")) => {
                c.formula = Some((get("t"), get("si"), String::new()));
                if !empty {
                    *into = Into::Formula;
                }
            }
            ("v", Some("c")) => {
                c.values += 1;
                if c.values == 1 {
                    c.value = Some(String::new());
                    if !empty {
                        *into = Into::Value;
                    }
                }
            }
            ("is", Some("c")) => c.inline = true,
            (_, Some("f") | Some("v")) => c.broken = true,
            _ => {}
        }
    };
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Eof => break,
            Event::Start(e) => {
                let (q, attrs) = element_of(&e)?;
                let (ns, name, attrs) = resolved(&mut scopes, depth, &q, &attrs);
                depth += 1;
                if foreign > 0 || !MAIN.contains(&ns.as_str()) {
                    foreign += 1;
                    continue;
                }
                start(&name, &attrs, &path, &mut cell, false, &mut into);
                path.push(name);
            }
            Event::Empty(e) => {
                let (q, attrs) = element_of(&e)?;
                let (ns, name, attrs) = resolved(&mut scopes, depth, &q, &attrs);
                scopes.leave(depth);
                if foreign > 0 || !MAIN.contains(&ns.as_str()) {
                    continue;
                }
                start(&name, &attrs, &path, &mut cell, true, &mut into);
                if name == "c" && let Some(c) = cell.take() {
                    cells.push(c);
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                scopes.leave(depth);
                if foreign > 0 {
                    foreign -= 1;
                    continue;
                }
                let name = path.pop().unwrap_or_default();
                if name == "f" || name == "v" {
                    into = Into::Nothing;
                }
                if name == "c" && let Some(c) = cell.take() {
                    cells.push(c);
                }
            }
            event @ (Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)) => {
                if foreign > 0 {
                    continue;
                }
                let Some(c) = cell.as_mut() else { continue };
                let text = match &event {
                    Event::Text(t) => t.xml_content(quick_xml::XmlVersion::Implicit1_0).to_string(),
                    Event::CData(d) => d.xml_content(quick_xml::XmlVersion::Implicit1_0).to_string(),
                    Event::GeneralRef(r) => reference_text(r).ok_or("an entity the part does not declare")?,
                    _ => String::new(),
                };
                match into {
                    Into::Formula => {
                        if let Some(f) = c.formula.as_mut() {
                            f.2.push_str(&text);
                        }
                    }
                    Into::Value => {
                        if let Some(v) = c.value.as_mut() {
                            v.push_str(&text);
                        }
                    }
                    Into::Nothing => {}
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("an element not closed".into());
    }
    Ok(cells)
}

/// XSD's whitespace (space, tab, line feed, carriage return) taken off both ends, and no other
/// character: a no-break space is part of the value.
fn xsd_trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}
static RE_STR: LazyLock<Regex> = LazyLock::new(|| re(r#""(?:[^"]|"")*""#));
static RE_FN: LazyLock<Regex> = LazyLock::new(|| re(r"([A-Za-z_][A-Za-z0-9_.]*)\s*\("));
static RE_EXT: LazyLock<Regex> = LazyLock::new(|| re(r"\[[0-9]+\]"));
static RE_APP: LazyLock<Regex> = LazyLock::new(|| re(r"<Application>([^<]*)</Application>"));
static RE_APP_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"<AppVersion>([0-9]+)\.[0-9]+</AppVersion>"));
static RE_FILE_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r#"<fileVersion\s[^>]*?appName="xl"[^>]*?lastEdited="([0-9]+)""#));

/// XML's references resolved: the five named entities and every character reference
/// (`&#13;`, `&#x1F600;`); an unknown or broken one is left as written.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|e| *e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "amp" => Some('&'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| name.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Element text as an XML parser and then OOXML read it: a raw carriage return, alone or before a
/// line feed, is a line feed (XML 1.0 normalises line ends; `&#13;` stays a carriage return), the
/// references resolved, then Excel's `_xHHHH_` escapes decoded (`_x005F_` the underscore of a
/// literal one; a surrogate pair written as two escapes is the one character).
pub fn xml_text(raw: &str) -> String {
    let normalized = if raw.contains('\r') { raw.replace("\r\n", "\n").replace('\r', "\n") } else { raw.to_string() };
    decode_escapes(&unescape(&normalized))
}

fn decode_escapes(s: &str) -> String {
    if !s.contains("_x") {
        return s.to_string();
    }
    // `_x`, four ASCII hex digits, `_`: anything else (a sign, a space) is the text it is
    let unit = |b: &[u8], i: usize| -> Option<u32> {
        (b.len() >= i + 7 && b[i] == b'_' && b[i + 1] == b'x' && b[i + 6] == b'_' && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit))
            .then(|| std::str::from_utf8(&b[i + 2..i + 6]).ok().and_then(|h| u32::from_str_radix(h, 16).ok()))
            .flatten()
    };
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(u) = unit(b, i) {
            if (0xD800..0xDC00).contains(&u)
                && let Some(low) = unit(b, i + 7).filter(|l| (0xDC00..0xE000).contains(l))
                && let Some(c) = char::from_u32(0x10000 + ((u - 0xD800) << 10) + (low - 0xDC00))
            {
                out.push(c);
                i += 14;
                continue;
            }
            if let Some(c) = char::from_u32(u) {
                out.push(c);
                i += 7;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(c);
        i += c.len_utf8().max(1);
    }
    out
}

/// The errors a workbook file holds (ECMA-376 `ST_CellType` `e`): Excel writes its newer ones
/// (`#SPILL!`, `#CALC!`, …) as `#VALUE!` with value metadata.
pub const FILE_ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];


/// The errors Excel writes as `#VALUE!` with value metadata, by the `errorType` of the rich
/// value the metadata points at (the corpus's two; another stays the `#VALUE!` the file says).
const RICH_ERRORS: [(&str, &str); 2] = [("8", "#SPILL!"), ("13", "#CALC!")];

/// What each value-metadata index (a cell's `vm`, from 1) says the cell's error is, where Excel
/// wrote a `#SPILL!` or a `#CALC!` as `#VALUE!`: the metadata part's value metadata, its
/// `XLRICHVALUE` entry, the rich value it points at, and that value's `errorType`; each part read
/// by an XML parser.
fn rich_errors(archive: &mut zip::ZipArchive<fs::File>, rels: &[(String, String)]) -> HashMap<usize, String> {
    let part = |kind: &str| -> Option<String> {
        rels.iter().find(|(t, _)| t.ends_with(kind)).map(|(_, target)| match target.strip_prefix('/') {
            Some(absolute) => absolute.to_string(),
            None => format!("xl/{target}"),
        })
    };
    let mut out = HashMap::new();
    let tree = |archive: &mut zip::ZipArchive<fs::File>, kind: &str| part(kind).and_then(|p| read_part(archive, &p)).and_then(|x| parse_tree(&x).ok());
    let (Some(meta), Some(values), Some(structures)) =
        (tree(archive, "/sheetMetadata"), tree(archive, "/rdRichValue"), tree(archive, "/rdRichValueStructure"))
    else {
        return out;
    };
    let Some(rich_type) = meta
        .kids(&MAIN, "metadataTypes")
        .flat_map(|t| t.kids(&MAIN, "metadataType"))
        .position(|t| t.attr("name") == Some("XLRICHVALUE"))
        .map(|i| i + 1)
    else {
        return out;
    };
    let index = |s: Option<&str>| s.and_then(|v| xsd_trim(v).parse::<usize>().ok());
    let future: Vec<Option<usize>> = meta
        .kids(&MAIN, "futureMetadata")
        .filter(|f| f.attr("name") == Some("XLRICHVALUE"))
        .flat_map(|f| f.kids(&MAIN, "bk"))
        .map(|bk| bk.descendants(&RICH_DATA, "rvb").first().and_then(|r| index(r.attr("i"))))
        .collect();
    let structs: Vec<(Option<&str>, Vec<&str>)> =
        structures.kids(&RICH_DATA, "s").map(|s| (s.attr("t"), s.kids(&RICH_DATA, "k").filter_map(|k| k.attr("n")).collect())).collect();
    let rvs: Vec<(Option<usize>, Vec<&str>)> =
        values.kids(&RICH_DATA, "rv").map(|rv| (index(rv.attr("s")), rv.kids(&RICH_DATA, "v").map(|v| xsd_trim(&v.text)).collect())).collect();
    for (i, bk) in meta.kids(&MAIN, "valueMetadata").flat_map(|m| m.kids(&MAIN, "bk")).enumerate() {
        let Some(rc) = bk.kids(&MAIN, "rc").next() else { continue };
        if index(rc.attr("t")) != Some(rich_type) {
            continue;
        }
        let error = index(rc.attr("v"))
            .and_then(|v| future.get(v).copied().flatten())
            .and_then(|rv| rvs.get(rv))
            .and_then(|(s, vals)| {
                let (kind, keys) = structs.get((*s)?)?;
                let at = keys.iter().position(|k| *k == "errorType").filter(|_| *kind == Some("_error"))?;
                let code = vals.get(at)?;
                RICH_ERRORS.iter().find(|(c, _)| c == code).map(|(_, e)| e.to_string())
            });
        if let Some(e) = error {
            out.insert(i + 1, e);
        }
    }
    out
}

/// `A` is 1, `Z` 26, `AA` 27.
pub fn column_index(letters: &str) -> u32 {
    letters.bytes().fold(0u32, |acc, b| acc.saturating_mul(26).saturating_add(u32::from(b - b'A' + 1)))
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<fs::File>, OracleError> {
    let file = fs::File::open(path).map_err(|source| OracleError::Io { path: path.to_path_buf(), source })?;
    zip::ZipArchive::new(file).map_err(|source| OracleError::Zip { path: path.to_path_buf(), source })
}

fn read_part(archive: &mut zip::ZipArchive<fs::File>, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// The names Microsoft Excel writes as its application, on Windows, on the Mac, online and on
/// Android.
pub const EXCEL_APPLICATIONS: [&str; 4] = ["Microsoft Excel", "Microsoft Macintosh Excel", "Microsoft Excel Online", "Excel Android"];

/// The application `docProps/app.xml` names, when the workbook was saved by Microsoft Excel itself:
/// the corpus's rule. The package says so three ways, all required: the application is one of
/// [`EXCEL_APPLICATIONS`] exactly (openpyxl calls itself "Microsoft Excel Compatible / Openpyxl");
/// its `AppVersion` is Excel 2010's (14) or later (XlsxWriter writes "Microsoft Excel" with 12, an
/// older Excel's number it never was); and `xl/workbook.xml` carries Excel's `<fileVersion
/// appName="xl">` edited by Excel 2010 or later (`lastEdited` from 5). `None` for any other writer.
/// The rule reads what the package says of itself: a writer that forged all three would pass, a
/// limit.
pub fn excel_application(path: &Path) -> Option<String> {
    let mut archive = open_zip(path).ok()?;
    let app = read_part(&mut archive, "docProps/app.xml")?;
    let name = RE_APP.captures(&app)?[1].to_string();
    let major: u32 = RE_APP_VERSION.captures(&app)?[1].parse().ok()?;
    let workbook = read_part(&mut archive, "xl/workbook.xml")?;
    let edited: u32 = RE_FILE_VERSION.captures(&workbook)?[1].parse().ok()?;
    (EXCEL_APPLICATIONS.contains(&name.as_str()) && major >= 14 && edited >= 5).then_some(name)
}

/// Every workbook under `dir`, at any depth, that Microsoft Excel saved, sorted by path.
pub fn corpus(dir: &Path) -> Result<Vec<PathBuf>, OracleError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|source| OracleError::Io { path: d.clone(), source })?;
        for entry in entries {
            let entry = entry.map_err(|source| OracleError::Io { path: d.clone(), source })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xlsx")) && excel_application(&path).is_some() {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The value Excel cached for a formula cell, from its type and its value as the parser read them.
fn expected_of(c: &CellXml, rich: &HashMap<usize, String>) -> Expected {
    let never = |what: String| Expected::Unwritten(what);
    match (c.t.as_deref(), c.value.clone(), c.inline) {
        // a cell holds one value (CT_Cell): two value tags are no cache Excel writes
        (_, Some(v), _) if c.values > 1 => never(v),
        // an inline string (`<is>`, whatever the cell's type) is a constant's: Excel writes a
        // formula's text as `str`, never inline
        (_, _, true) => never("an inline string".into()),
        // Excel writes the value every formula computed: no `<v>` is a cache it never wrote
        (_, None, false) => Expected::Missing,
        // a formula's text Excel writes `str`
        (Some("str"), Some(v), false) => Expected::Text(decode_escapes(&v)),
        // a boolean Excel writes as 1 or 0
        (Some("b"), Some(v), false) => match xsd_trim(&v) {
            "1" => Expected::Bool(true),
            "0" => Expected::Bool(false),
            _ => never(v),
        },
        // an error a file holds; a `#VALUE!` with value metadata is the error the rich value names
        // (Excel's `#SPILL!` and `#CALC!`)
        (Some("e"), Some(v), false) => match FILE_ERRORS.iter().find(|e| **e == xsd_trim(&v)) {
            Some(e) => {
                let vm = c.vm.as_deref().and_then(|m| xsd_trim(m).parse::<usize>().ok());
                Expected::Error(vm.and_then(|i| rich.get(&i).cloned()).filter(|_| *e == "#VALUE!").unwrap_or_else(|| e.to_string()))
            }
            None => never(v),
        },
        // a number Excel can cache is finite: `inf`, `NaN`, `1E+999`, a word or nothing is none
        (None | Some("n"), Some(v), false) => match xsd_trim(&v).parse::<f64>() {
            Ok(n) if n.is_finite() && !xsd_trim(&v).is_empty() => Expected::Number(n),
            _ => never(v),
        },
        // a shared string (`s`), a date (`d`), an empty type, any other: none Excel writes for a
        // formula's value
        (Some(_), Some(v), false) => never(v),
    }
}

/// Reads the expected side of a workbook from its parts.
pub fn scan_raw(path: &Path) -> Result<RawWorkbook, OracleError> {
    let mut archive = open_zip(path)?;
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let e = archive.by_index(i).map_err(|source| OracleError::Zip { path: path.to_path_buf(), source })?;
        let name = e.name().map_err(|source| OracleError::Zip { path: path.to_path_buf(), source })?.to_string();
        parts.insert(name, (e.size(), e.crc32()));
    }

    // Excel writes a formula's text as `t="str"`, never as a shared string: the shared strings are
    // no formula's cache, and are not read

    // Sheet order and names, to part names, through workbook.xml and its relationships, each read
    // by an XML parser.
    let bad_xml = |part: &str, reason: String| OracleError::Xml { path: path.to_path_buf(), part: part.to_string(), reason };
    let workbook_xml = read_part(&mut archive, "xl/workbook.xml")
        .ok_or(OracleError::MissingPart { path: path.to_path_buf(), part: "xl/workbook.xml" })?;
    let rels_xml = read_part(&mut archive, "xl/_rels/workbook.xml.rels")
        .ok_or(OracleError::MissingPart { path: path.to_path_buf(), part: "xl/_rels/workbook.xml.rels" })?;
    let workbook = parse_tree(&workbook_xml).map_err(|e| bad_xml("xl/workbook.xml", e))?;
    let rels_tree = parse_tree(&rels_xml).map_err(|e| bad_xml("xl/_rels/workbook.xml.rels", e))?;
    let relationships: Vec<&Node> = rels_tree.kids(&PACKAGE_RELATIONSHIPS, "Relationship").collect();
    let rels: HashMap<String, String> =
        relationships.iter().filter_map(|r| Some((r.attr("Id")?.to_string(), r.attr("Target")?.to_string()))).collect();
    // the relationships by their type, for the parts found that way (the metadata, rich data)
    let typed: Vec<(String, String)> =
        relationships.iter().filter_map(|r| Some((r.attr("Type")?.to_string(), r.attr("Target")?.to_string()))).collect();
    let rich = rich_errors(&mut archive, &typed);
    let sheets: Vec<(String, String)> = workbook
        .kids(&MAIN, "sheets")
        .flat_map(|s| s.kids(&MAIN, "sheet"))
        .filter_map(|s| {
            let target = rels.get(s.attr_in(&RELATIONSHIP_ID, "id")?).cloned().unwrap_or_default();
            let part = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_string(),
                None => format!("xl/{target}"),
            };
            Some((s.attr("name")?.to_string(), part))
        })
        .collect();
    let defined_names = workbook.descendants(&MAIN, "definedName").len();
    let external_links = workbook.descendants(&MAIN, "externalReference").len();

    let mut external_cached_cells = 0;
    for name in parts.keys().cloned().collect::<Vec<_>>() {
        if name.starts_with("xl/externalLinks/externalLink")
            && name.ends_with(".xml")
            && let Some(xml) = read_part(&mut archive, &name)
        {
            external_cached_cells += parse_tree(&xml).map(|t| t.descendants(&MAIN, "cell").len()).unwrap_or(0);
        }
    }

    let mut formula_cells = Vec::new();
    let mut value_cells = 0usize;
    for (sheet_name, part) in &sheets {
        let Some(xml) = read_part(&mut archive, part) else { continue };
        let mut shared_masters: HashMap<u32, String> = HashMap::new();
        for c in sheet_cells(&xml).map_err(|e| bad_xml(part, e))? {
            let reference = c.reference.clone().unwrap_or_default();
            let split = reference.find(|ch: char| ch.is_ascii_digit()).unwrap_or(reference.len());
            let (col_letters, digits) = reference.split_at(split);
            let row: u32 = digits.parse().map_err(|_| OracleError::BadReference { path: path.to_path_buf(), reference: reference.clone() })?;
            if col_letters.is_empty() || !col_letters.bytes().all(|b| b.is_ascii_uppercase()) {
                return Err(OracleError::BadReference { path: path.to_path_buf(), reference });
            }
            let Some((ftype, si, ftext)) = c.formula.clone() else {
                if c.values > 0 || c.inline {
                    value_cells += 1;
                }
                continue;
            };
            let text = decode_escapes(&ftext);
            let si = si.as_deref().and_then(|v| xsd_trim(v).parse::<u32>().ok());
            let array = ftype.as_deref() == Some("array");
            let (formula, shared_copy) = match (ftype.as_deref(), si) {
                (Some("shared"), Some(si)) if !text.is_empty() => {
                    shared_masters.insert(si, text.clone());
                    (text, false)
                }
                (Some("shared"), Some(si)) => (shared_masters.get(&si).cloned().unwrap_or_default(), true),
                _ => (text, false),
            };
            let never = |what: String| Expected::Unwritten(what);
            let expected = if c.broken {
                never("an element where the schema has text".into())
            } else {
                expected_of(&c, &rich)
            };
            formula_cells.push(FormulaCell {
                sheet: sheet_name.clone(),
                reference: format!("{col_letters}{row}"),
                row,
                col: column_index(col_letters),
                formula,
                shared_copy,
                array,
                expected,
            });
        }
    }

    Ok(RawWorkbook {
        sheets: sheets.into_iter().map(|(n, _)| n).collect(),
        formula_cells,
        value_cells,
        parts,
        defined_names,
        external_links,
        external_cached_cells,
    })
}

// ---------------------------------------------------------------------------------------------
// The cross-check of the expected side against calamine
// ---------------------------------------------------------------------------------------------

/// How far the oracle's own reading and calamine's agree on the formula cells.
pub struct CalamineCheck {
    pub compared: usize,
    pub disagreements: usize,
    pub examples: Vec<String>,
    pub error: Option<String>,
}

/// A calamine value, owned.
#[derive(Debug, Clone)]
enum Seen {
    Number(f64),
    Text(String),
    Bool(bool),
    Error,
    /// A date or duration calamine gives as ISO text: not compared.
    Iso,
    Empty,
}

fn seen(value: &DataRef<'_>) -> Seen {
    match value {
        DataRef::Int(i) => Seen::Number(*i as f64),
        DataRef::Float(f) => Seen::Number(*f),
        DataRef::String(s) => Seen::Text(s.clone()),
        DataRef::SharedString(s) => Seen::Text((*s).to_string()),
        DataRef::Bool(b) => Seen::Bool(*b),
        DataRef::DateTime(d) => Seen::Number(d.as_f64()),
        DataRef::DateTimeIso(_) | DataRef::DurationIso(_) => Seen::Iso,
        DataRef::Error(_) => Seen::Error,
        DataRef::Empty => Seen::Empty,
    }
}

/// Cross-checks the expected side cell by cell against calamine's reading of the same file. The
/// cells are streamed (calamine's cell reader), never gathered into a sheet's whole range.
pub fn calamine_check(path: &Path, raw: &RawWorkbook) -> CalamineCheck {
    let mut out = CalamineCheck { compared: 0, disagreements: 0, examples: Vec::new(), error: None };
    let mut wb: calamine::Xlsx<_> = match calamine::open_workbook(path) {
        Ok(w) => w,
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
    };
    let mut by_sheet: BTreeMap<&str, HashMap<(u32, u32), Option<Seen>>> = BTreeMap::new();
    for cell in &raw.formula_cells {
        by_sheet.entry(cell.sheet.as_str()).or_default().insert((cell.row - 1, cell.col - 1), None);
    }
    for (sheet, wanted) in by_sheet.iter_mut() {
        let mut reader = match wb.worksheet_cells_reader(sheet) {
            Ok(r) => r,
            Err(e) => {
                out.error = Some(format!("{sheet}: {e}"));
                continue;
            }
        };
        loop {
            match reader.next_cell() {
                Ok(Some(c)) => {
                    if let Some(slot) = wanted.get_mut(&c.get_position()) {
                        *slot = Some(seen(c.get_value()));
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    out.error = Some(format!("{sheet}: {e}"));
                    break;
                }
            }
        }
    }
    for cell in &raw.formula_cells {
        let got = by_sheet.get(cell.sheet.as_str()).and_then(|m| m.get(&(cell.row - 1, cell.col - 1))).cloned().flatten();
        out.compared += 1;
        let agree = match (&got, &cell.expected) {
            (None | Some(Seen::Empty), Expected::Missing) => true,
            (None | Some(Seen::Empty), Expected::Text(t)) => t.is_empty(),
            (Some(Seen::Number(a)), Expected::Number(b)) => (a - b).abs() <= 1e-9 * a.abs().max(1.0),
            (Some(Seen::Text(a)), Expected::Text(b)) => a == b,
            (Some(Seen::Bool(a)), Expected::Bool(b)) => a == b,
            (Some(Seen::Error), Expected::Error(_)) => true,
            (Some(Seen::Iso), _) => true,
            _ => false,
        };
        if !agree {
            out.disagreements += 1;
            if out.examples.len() < 5 {
                out.examples.push(format!("{}!{}: raw {:?} vs calamine {:?}", cell.sheet, cell.reference, cell.expected, got));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The function census
// ---------------------------------------------------------------------------------------------

fn strip_prefix(name: &str) -> String {
    let mut n = name.to_ascii_uppercase();
    for p in ["_XLFN.", "_XLWS."] {
        while let Some(rest) = n.strip_prefix(p) {
            n = rest.to_string();
        }
    }
    n
}

/// The functions a formula calls, upper-case, their `_xlfn.`/`_xlws.` prefixes dropped, names
/// inside string literals ignored.
pub fn function_names(formula: &str) -> Vec<String> {
    let cleaned = RE_STR.replace_all(formula, "\"\"");
    RE_FN.captures_iter(&cleaned).map(|c| strip_prefix(&c[1])).filter(|n| !n.is_empty()).collect()
}

/// Which functions the engine knows, probed by parsing `=NAME(1)` in an empty model: `#NAME?` is
/// a name the parser does not know. Answers are kept.
#[derive(Default)]
pub struct Probe {
    known: HashMap<String, bool>,
}

impl Probe {
    pub fn knows(&mut self, name: &str) -> bool {
        if let Some(&b) = self.known.get(name) {
            return b;
        }
        let known = match ironcalc_base::Model::new_empty("probe", "en", "UTC", "en") {
            Ok(mut m) => {
                let _ = m.set_user_input(0, 1, 1, format!("={name}(1)"));
                m.evaluate();
                !matches!(m.get_cell_value_by_index(0, 1, 1), Ok(CellValue::String(ref s)) if s == "#NAME?")
            }
            Err(_) => false,
        };
        self.known.insert(name.to_string(), known);
        known
    }

    pub fn known(&self, name: &str) -> bool {
        self.known.get(name).copied().unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------------------------

/// What the engine holds at a formula cell.
#[derive(Debug, Clone)]
pub enum Actual {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Empty,
    /// The engine has no such sheet.
    NoSheet,
    /// The engine holds the cell as a constant (the file's cache), having computed nothing there:
    /// equal to nothing.
    NotComputed,
}

fn fmt_expected(e: &Expected) -> String {
    match e {
        Expected::Number(n) => format!("{n}"),
        Expected::Text(t) => format!("{t:?}"),
        Expected::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        Expected::Error(e) => e.clone(),
        Expected::Missing => "(no cached value)".into(),
        Expected::Unwritten(raw) => format!("(never written so: {raw:?})"),
    }
}

fn fmt_actual(a: &Actual) -> String {
    match a {
        Actual::Number(n) => format!("{n}"),
        Actual::Text(t) => format!("{t:?}"),
        Actual::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        Actual::Error(e) => e.clone(),
        Actual::Empty => "(empty)".into(),
        Actual::NoSheet => "(sheet missing in the engine)".into(),
        Actual::NotComputed => "(not computed by the engine)".into(),
    }
}

/// The class of one comparison, in the report's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Match,
    /// A mismatch on a formula that reaches into another workbook (`[1]Sheet!A1`).
    ExternalLink,
    /// `#NAME?` where Excel had a value, and the formula calls a function the engine does not know.
    UnsupportedFunction,
    /// `#NAME?` with every function known: a defined name, a sheet, or a parse the engine lacks.
    NameError,
    /// Another error where Excel had a value.
    ErrorForValue,
    /// Excel cached an error, the engine produced a value.
    ValueForError,
    /// Both errors, of different kinds.
    ErrorType,
    /// Both numbers, within 1e-6 relative.
    NumericEpsilon,
    /// Both numbers, really different.
    Numeric,
    /// Both text, different.
    Text,
    /// A boolean against something else, or two different booleans.
    Bool,
    /// A value on one side, nothing on the other.
    EmptyVsValue,
    /// A number on one side, text on the other.
    TypeNumberText,
    /// An array or spill formula.
    Spill,
    /// The file has a sheet the engine did not load.
    MissingSheet,
}

impl Class {
    pub const ALL: [Class; 15] = [
        Class::Match,
        Class::ExternalLink,
        Class::UnsupportedFunction,
        Class::NameError,
        Class::ErrorForValue,
        Class::ValueForError,
        Class::ErrorType,
        Class::NumericEpsilon,
        Class::Numeric,
        Class::Text,
        Class::Bool,
        Class::EmptyVsValue,
        Class::TypeNumberText,
        Class::Spill,
        Class::MissingSheet,
    ];

    /// The class's name in a summary or a baseline.
    pub fn id(self) -> &'static str {
        match self {
            Class::Match => "match",
            Class::ExternalLink => "external_link",
            Class::UnsupportedFunction => "unsupported_function",
            Class::NameError => "name_error",
            Class::ErrorForValue => "error_for_value",
            Class::ValueForError => "value_for_error",
            Class::ErrorType => "error_type",
            Class::NumericEpsilon => "numeric_epsilon",
            Class::Numeric => "numeric",
            Class::Text => "text",
            Class::Bool => "boolean",
            Class::EmptyVsValue => "empty_vs_value",
            Class::TypeNumberText => "number_vs_text",
            Class::Spill => "spill",
            Class::MissingSheet => "missing_sheet",
        }
    }

    /// The class's name in the report, as Veredas 1's oracle wrote it.
    pub fn label(self) -> &'static str {
        match self {
            Class::Match => "match",
            Class::ExternalLink => "external link `[n]`",
            Class::UnsupportedFunction => "unsupported function (#NAME?)",
            Class::NameError => "#NAME? (name/parse, all functions known)",
            Class::ErrorForValue => "error where Excel had a value",
            Class::ValueForError => "value where Excel had an error",
            Class::ErrorType => "error type differs",
            Class::NumericEpsilon => "numeric (within 1e-6 rel.)",
            Class::Numeric => "numeric",
            Class::Text => "text",
            Class::Bool => "boolean",
            Class::EmptyVsValue => "empty vs value",
            Class::TypeNumberText => "number vs text",
            Class::Spill => "array / spill",
            Class::MissingSheet => "sheet missing in IronCalc",
        }
    }
}

/// Classifies one formula cell's comparison.
pub fn classify(cell: &FormulaCell, actual: &Actual, unsupported_in_formula: bool, has_external: bool) -> Class {
    use Actual as A;
    use Expected as E;
    let matched = match (&cell.expected, actual) {
        (E::Number(a), A::Number(b)) => a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * a.abs().max(1.0),
        (E::Text(a), A::Text(b)) => a == b,
        (E::Text(a), A::Empty) => a.is_empty(),
        (E::Bool(a), A::Bool(b)) => a == b,
        (E::Error(a), A::Error(b)) => a == b,
        _ => false,
    };
    if matched {
        return Class::Match;
    }
    // a cache Excel never wrote is classed as the text it is (it matched nothing above)
    let expected = match &cell.expected {
        E::Unwritten(raw) => E::Text(raw.clone()),
        e => e.clone(),
    };
    if matches!(actual, A::NoSheet) {
        return Class::MissingSheet;
    }
    if cell.array {
        return Class::Spill;
    }
    if has_external {
        return Class::ExternalLink;
    }
    match (&expected, actual) {
        (E::Error(_), A::Error(_)) => Class::ErrorType,
        (E::Error(_), _) => Class::ValueForError,
        (_, A::Error(e)) if e == "#NAME?" => {
            if unsupported_in_formula {
                Class::UnsupportedFunction
            } else {
                Class::NameError
            }
        }
        (_, A::Error(e)) if e == "#SPILL!" => Class::Spill,
        (_, A::Error(_)) => Class::ErrorForValue,
        (E::Number(a), A::Number(b)) => {
            if (a - b).abs() <= 1e-6 * a.abs().max(1.0) {
                Class::NumericEpsilon
            } else {
                Class::Numeric
            }
        }
        (E::Text(_), A::Text(_)) => Class::Text,
        (E::Bool(_), _) | (_, A::Bool(_)) => Class::Bool,
        (E::Missing, _) | (_, A::Empty) | (_, A::NotComputed) => Class::EmptyVsValue,
        (E::Number(_), A::Text(_)) | (E::Text(_), A::Number(_)) => Class::TypeNumberText,
        _ => Class::Text,
    }
}

/// One mismatch, as the report shows it.
pub struct Example {
    pub sheet: String,
    pub reference: String,
    pub formula: String,
    pub expected: String,
    pub actual: String,
}

impl Example {
    fn new(cell: &FormulaCell, actual: &Actual) -> Self {
        let mut f = cell.formula.clone();
        if f.chars().count() > 110 {
            f = f.chars().take(107).collect::<String>() + "…";
        }
        Example {
            sheet: cell.sheet.clone(),
            reference: cell.reference.clone(),
            formula: f,
            expected: fmt_expected(&cell.expected),
            actual: fmt_actual(actual),
        }
    }
}

fn read_actual(model: &ironcalc_base::Model, si: u32, cell: &FormulaCell) -> Actual {
    let (row, col) = (cell.row as i32, cell.col as i32);
    // a cell the file says is a formula that the engine holds as neither a formula nor an array's
    // spilled cell (an empty `<f ca="1"/>` outside every array, which the importer reads as no
    // formula), or does not hold at all: its value is the file's cache, or nothing, not anything
    // the engine computed
    let computed = model.workbook.worksheet(si).ok().and_then(|ws| ws.cell(row, col)).is_some_and(|c| {
        c.get_formula().is_some() || matches!(c.as_ref(), ironcalc_base::types::Cell::SpillCell { .. })
    });
    if !computed {
        return Actual::NotComputed;
    }
    let ty = model.get_cell_type(si, row, col).unwrap_or(CellType::Number);
    match model.get_cell_value_by_index(si, row, col) {
        Ok(CellValue::None) => Actual::Empty,
        Ok(CellValue::Number(n)) => Actual::Number(n),
        Ok(CellValue::Boolean(b)) => Actual::Bool(b),
        // the cell's type says error or text: a text that reads like an error (`#div/0!`) is text
        Ok(CellValue::String(s)) if ty == CellType::ErrorValue => Actual::Error(s),
        Ok(CellValue::String(s)) => Actual::Text(s),
        Err(e) => Actual::Error(format!("(api error: {e})")),
    }
}

/// How many cells use a function, and how many of those matched.
#[derive(Default, Clone, Copy)]
pub struct FunctionStat {
    pub cells: usize,
    pub matched: usize,
}

/// Everything measured of one workbook.
pub struct FileReport {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub raw: Option<RawWorkbook>,
    pub raw_error: Option<String>,
    pub calamine: Option<CalamineCheck>,
    pub load_time: Option<Duration>,
    pub load_error: Option<String>,
    pub eval_time: Option<Duration>,
    pub classes: BTreeMap<Class, usize>,
    pub examples: BTreeMap<Class, Vec<Example>>,
    pub external_formulas: usize,
    pub functions: BTreeMap<String, FunctionStat>,
    pub engine_sheets: Vec<String>,
    pub save_time: Option<Duration>,
    pub save_error: Option<String>,
    pub roundtrip: Vec<(String, String)>,
    pub reload_error: Option<String>,
    pub reload_time: Option<Duration>,
    pub reload_classes: BTreeMap<Class, usize>,
    pub link_choice: Option<LinkChoice>,
    pub link_outcome: Option<LinkOutcome>,
    pub link_strategy: String,
    /// Under a linked resolution, one line per external cell whose live value differs from the
    /// cache.
    pub link_diff: Vec<String>,
}

impl FileReport {
    fn new(path: &Path) -> FileReport {
        FileReport {
            path: path.to_path_buf(),
            name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            raw: None,
            raw_error: None,
            calamine: None,
            load_time: None,
            load_error: None,
            eval_time: None,
            classes: BTreeMap::new(),
            examples: BTreeMap::new(),
            external_formulas: 0,
            functions: BTreeMap::new(),
            engine_sheets: Vec::new(),
            save_time: None,
            save_error: None,
            roundtrip: Vec::new(),
            reload_error: None,
            reload_time: None,
            reload_classes: BTreeMap::new(),
            link_choice: None,
            link_outcome: None,
            link_strategy: String::new(),
            link_diff: Vec::new(),
        }
    }

    /// The comparison's numbers, `None` when the file could not be measured (its expected side
    /// unread, or the engine unable to open it).
    pub fn measure(&self) -> Option<Measure> {
        if self.raw_error.is_some() || self.load_error.is_some() {
            return None;
        }
        let raw = self.raw.as_ref()?;
        Some(Measure {
            formula_cells: raw.formula_cells.len(),
            matched: self.classes.get(&Class::Match).copied().unwrap_or(0),
            classes: self.classes.iter().map(|(c, n)| (c.id().to_string(), *n)).collect(),
        })
    }
}

/// What one run measures of each workbook.
pub struct Options {
    pub links: LinksMode,
    /// How many mismatches of each class the report shows.
    pub max_examples: usize,
    /// Where the fork's writer saves its copy for the round trip; `None` skips the round trip.
    pub roundtrip_dir: Option<PathBuf>,
    /// Whether progress is written to stderr.
    pub verbose: bool,
    /// The clock the engine's date functions read while a workbook is measured, in milliseconds
    /// since 1970 UTC (`TODAY()`, `NOW()`); `None` is the system's. A measure compared with one
    /// taken on another day pins the day it was taken ([`noon_utc`]).
    pub clock: Option<i64>,
    /// The seed of the engine's random functions while a workbook is measured (`RAND()`,
    /// `RANDARRAY()`), so a random draw that equals Excel's cache by chance is the same chance on
    /// every run; `None` is the system's randomness.
    pub random_seed: Option<u64>,
}

/// Noon UTC of a `YYYY-MM-DD` date, in milliseconds since 1970: the clock a measure taken that day
/// is pinned to (noon, so the day is the same in every time zone from UTC-11 to UTC+11).
pub fn noon_utc(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if parts.next().is_some() || !(1..=days_in_month).contains(&day) {
        return None;
    }
    // days from 1970-01-01 to the civil date (Howard Hinnant's algorithm)
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000 + 12 * 3_600_000)
}

/// The engine's random functions seeded for as long as it lives (the sequence started at the
/// seed), returned to chance when it drops.
struct SeedPin;

impl SeedPin {
    fn new(seed: Option<u64>) -> SeedPin {
        ironcalc_base::seed_random(seed);
        SeedPin
    }
}

impl Drop for SeedPin {
    fn drop(&mut self) {
        ironcalc_base::seed_random(None);
    }
}

/// The engine's clock pinned for as long as it lives; when it drops, the clock is what it was.
struct ClockPin {
    before: Option<i64>,
}

impl ClockPin {
    fn new(clock: Option<i64>) -> ClockPin {
        let before = ironcalc_base::pinned_clock();
        ironcalc_base::pin_clock(clock);
        ClockPin { before }
    }
}

impl Drop for ClockPin {
    fn drop(&mut self) {
        ironcalc_base::pin_clock(self.before);
    }
}

/// Measures one workbook.
pub fn run_file(path: &Path, options: &Options, probe: &mut Probe) -> FileReport {
    let _clock = ClockPin::new(options.clock);
    let _seed = SeedPin::new(options.random_seed);
    let mut rep = FileReport::new(path);
    let name = rep.name.clone();
    let say = |what: String| {
        if options.verbose {
            eprintln!("[{name}] {what}");
        }
    };

    let raw = match scan_raw(path) {
        Ok(r) => r,
        Err(e) => {
            rep.raw_error = Some(e.to_string());
            return rep;
        }
    };
    say(format!("{} formula cells, {} value cells; calamine cross-check", raw.formula_cells.len(), raw.value_cells));
    // calamine panics on some files Excel never wrote (a shared-string index past the table): the
    // cross-check reports it rather than abort the measure
    rep.calamine = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| calamine_check(path, &raw))).unwrap_or_else(|_| CalamineCheck {
            compared: 0,
            disagreements: 0,
            examples: Vec::new(),
            error: Some("calamine panicked reading the file".into()),
        }),
    );

    let links = options.links;
    let t0 = Instant::now();
    let opened = veredas_abaco::links::open(path, links.policy(), |books| {
        for (i, book) in books.iter().enumerate() {
            say(format!(
                "link [{}] to `{}` ({} sheets, {} cached cells)",
                i + 1,
                book.target,
                book.sheets.len(),
                book.sheets.iter().map(|s| s.cells.len()).sum::<usize>()
            ));
        }
        // headless, the answer that is never wrong to make silently
        LinkChoice::UseCached
    });
    let mut model = match opened {
        Ok(mut opened) => {
            rep.link_diff = opened.link_diffs().iter().map(|d| d.to_string()).collect();
            rep.link_choice = opened.choice;
            rep.link_outcome = Some(opened.links);
            rep.link_strategy = opened.model.external_strategy().to_string();
            opened.model
        }
        Err(e) => {
            rep.load_time = Some(t0.elapsed());
            rep.load_error = Some(e.to_string());
            rep.raw = Some(raw);
            return rep;
        }
    };
    // `open` evaluates once, the links resolved; evaluate again to time the recalculation alone.
    rep.load_time = Some(t0.elapsed());
    let t1 = Instant::now();
    model.evaluate();
    rep.eval_time = Some(t1.elapsed());
    say(format!("opened and evaluated in {:?}; compare", rep.load_time.unwrap_or_default()));

    let sheet_index: HashMap<String, u32> =
        model.get_worksheets_properties().iter().enumerate().map(|(i, p)| (p.name.clone(), i as u32)).collect();
    rep.engine_sheets = model.get_worksheets_properties().iter().map(|p| p.name.clone()).collect();

    for cell in &raw.formula_cells {
        let names = function_names(&cell.formula);
        let unsupported = names.iter().any(|n| !probe.knows(n));
        let has_external = RE_EXT.is_match(&cell.formula);
        if has_external {
            rep.external_formulas += 1;
        }
        let actual = match sheet_index.get(&cell.sheet) {
            Some(&si) => read_actual(&model, si, cell),
            None => Actual::NoSheet,
        };
        let class = classify(cell, &actual, unsupported, has_external);
        *rep.classes.entry(class).or_insert(0) += 1;
        for n in names.iter().collect::<BTreeSet<_>>() {
            let st = rep.functions.entry(n.clone()).or_default();
            st.cells += 1;
            if class == Class::Match {
                st.matched += 1;
            }
        }
        if class != Class::Match {
            let ex = rep.examples.entry(class).or_default();
            if ex.len() < options.max_examples {
                ex.push(Example::new(cell, &actual));
            }
        }
    }

    if let Some(dir) = &options.roundtrip_dir {
        roundtrip(&mut rep, &raw, &model, dir, probe);
    }
    rep.raw = Some(raw);
    rep
}

/// Saves the model with the fork's writer, compares the zip parts, and compares the saved file's
/// evaluation against the ORIGINAL cached values.
fn roundtrip(rep: &mut FileReport, raw: &RawWorkbook, model: &ironcalc_base::Model, dir: &Path, probe: &mut Probe) {
    if let Err(e) = fs::create_dir_all(dir) {
        rep.save_error = Some(format!("cannot create {}: {e}", dir.display()));
        return;
    }
    let stem = rep.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let out_path = dir.join(format!("{stem}.ironcalc.xlsx"));
    let _ = fs::remove_file(&out_path);
    let t2 = Instant::now();
    if let Err(e) = ironcalc::export::save_to_xlsx(model, &out_path.to_string_lossy()) {
        rep.save_time = Some(t2.elapsed());
        rep.save_error = Some(e.to_string());
        return;
    }
    rep.save_time = Some(t2.elapsed());
    let out_raw = match scan_raw(&out_path) {
        Ok(r) => r,
        Err(e) => {
            rep.reload_error = Some(e.to_string());
            return;
        }
    };
    let mut all: BTreeSet<&String> = raw.parts.keys().collect();
    all.extend(out_raw.parts.keys());
    for part in all {
        let verdict = match (raw.parts.get(part), out_raw.parts.get(part)) {
            (Some(a), Some(b)) if a == b => "identical (size+crc)".to_string(),
            (Some(a), Some(b)) => format!("changed ({} → {} bytes)", a.0, b.0),
            (Some(a), None) => format!("DROPPED ({} bytes)", a.0),
            (None, Some(b)) => format!("added ({} bytes)", b.0),
            (None, None) => continue,
        };
        rep.roundtrip.push((part.clone(), verdict));
    }
    let t3 = Instant::now();
    match ironcalc::import::load_from_xlsx(&out_path.to_string_lossy(), "en", "UTC", "en") {
        Ok(mut m2) => {
            m2.evaluate();
            rep.reload_time = Some(t3.elapsed());
            let idx2: HashMap<String, u32> =
                m2.get_worksheets_properties().iter().enumerate().map(|(i, p)| (p.name.clone(), i as u32)).collect();
            for cell in &raw.formula_cells {
                let actual = match idx2.get(&cell.sheet) {
                    None => Actual::NoSheet,
                    Some(&si) => read_actual(&m2, si, cell),
                };
                let unsupported = function_names(&cell.formula).iter().any(|n| !probe.knows(n));
                let class = classify(cell, &actual, unsupported, RE_EXT.is_match(&cell.formula));
                *rep.reload_classes.entry(class).or_insert(0) += 1;
            }
        }
        Err(e) => rep.reload_error = Some(e.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// The summary and the baseline
// ---------------------------------------------------------------------------------------------

/// The comparison's numbers for one workbook under one resolution of its links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    pub formula_cells: usize,
    pub matched: usize,
    /// Cells per [`Class::id`], the matched ones included.
    pub classes: BTreeMap<String, usize>,
}

/// One workbook of a baseline: its numbers under each resolution, or why it was not measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineFile {
    /// The path below the corpus folder, forward slashes.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<Measure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked: Option<Measure>,
    /// Why the measuring oracle could not measure it, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmeasured: Option<String>,
}

/// The format of a baseline file (law L7).
pub const BASELINE_FORMAT: &str = "veredas-abaco-oracle-baseline";
pub const BASELINE_VERSION: u32 = 1;

/// A baseline: what an oracle measured of a corpus, file by file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub format: String,
    pub version: u32,
    /// Which oracle and which engine measured, at which commit.
    pub measured_by: String,
    pub measured_on: String,
    /// The corpus folder, from the repository's root.
    pub corpus: String,
    pub file: Vec<BaselineFile>,
}

/// Why a baseline file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Header {
        path: PathBuf,
        #[source]
        source: veredas_core::schema::HeaderError,
    },
    #[error("{path}: {source}")]
    Body {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Baseline {
    /// Reads a baseline, its stamp checked first by core's header reader.
    pub fn read(path: &Path) -> Result<Baseline, BaselineError> {
        let text = fs::read_to_string(path).map_err(|source| BaselineError::Io { path: path.to_path_buf(), source })?;
        veredas_core::schema::check(&text, "an oracle baseline", BASELINE_FORMAT, BASELINE_VERSION)
            .map_err(|source| BaselineError::Header { path: path.to_path_buf(), source })?;
        toml::from_str(&text).map_err(|source| BaselineError::Body { path: path.to_path_buf(), source })
    }
}

// ---------------------------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------------------------

fn ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{:.0} ms", d.as_secs_f64() * 1000.0),
        None => "—".into(),
    }
}

/// The Markdown report of a run.
pub fn write_report(reports: &[FileReport], probe: &Probe, links: LinksMode, max_examples: usize) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# abaco-oracle report (links: {})\n", links.label());
    let _ = writeln!(md, "Ábaco's engine (its fork of IronCalc) against the values Excel cached in each workbook. ");
    let _ = writeln!(md, "Expected side read from the sheet XML directly; cross-checked against calamine. ");
    let _ = writeln!(md, "Function support is probed per name (`=NAME(1)` typed into an empty model; `#NAME?` is unknown).\n");

    let _ = writeln!(md, "## Summary\n");
    let _ = writeln!(md, "| File | Size | Sheets | Value cells | Formula cells | Matched | Mismatched | Load | Eval | Save |");
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for r in reports {
        let (sheets, values, formulas) =
            r.raw.as_ref().map(|w| (w.sheets.len(), w.value_cells, w.formula_cells.len())).unwrap_or((0, 0, 0));
        let matched = r.classes.get(&Class::Match).copied().unwrap_or(0);
        let total: usize = r.classes.values().sum();
        let (m, mm) = if r.load_error.is_some() || r.raw_error.is_some() {
            ("not measured".to_string(), "—".to_string())
        } else {
            (matched.to_string(), (total - matched).to_string())
        };
        let save = if r.save_error.is_some() { "failed".to_string() } else { ms(r.save_time) };
        let _ = writeln!(
            md,
            "| {} | {:.1} MB | {sheets} | {values} | {formulas} | {m} | {mm} | {} | {} | {save} |",
            r.name,
            r.size as f64 / 1e6,
            ms(r.load_time),
            ms(r.eval_time),
        );
    }
    let _ = writeln!(md);

    for r in reports {
        let _ = writeln!(md, "## {}\n", r.name);
        if let Some(e) = &r.raw_error {
            let _ = writeln!(md, "**Raw scan failed:** `{e}`\n");
            continue;
        }
        let Some(raw) = r.raw.as_ref() else { continue };
        let _ = writeln!(
            md,
            "- Sheets: {} (`{}`)\n- Value cells: {} · formula cells: {} (of which shared-formula copies: {}, array: {})\n- Defined names: {} · external links: {} (cached external cells carried in the file: {})\n- Formulas reaching an external workbook (`[n]`): {}",
            raw.sheets.len(),
            raw.sheets.join("`, `"),
            raw.value_cells,
            raw.formula_cells.len(),
            raw.formula_cells.iter().filter(|c| c.shared_copy).count(),
            raw.formula_cells.iter().filter(|c| c.array).count(),
            raw.defined_names,
            raw.external_links,
            raw.external_cached_cells,
            r.external_formulas
        );
        if let Some(c) = &r.calamine {
            let _ = writeln!(
                md,
                "- Expected-side self-check (raw XML vs calamine): {} compared, {} disagreements{}",
                c.compared,
                c.disagreements,
                c.error.as_ref().map(|e| format!(" — calamine error: `{e}`")).unwrap_or_default()
            );
            for ex in &c.examples {
                let _ = writeln!(md, "    - {ex}");
            }
        }
        let _ = writeln!(
            md,
            "- Timings: open+evaluate {} · evaluate again {} · save {} · reload+evaluate {}",
            ms(r.load_time),
            ms(r.eval_time),
            ms(r.save_time),
            ms(r.reload_time)
        );
        if let Some(outcome) = &r.link_outcome {
            let choice = r.link_choice.map(|c| c.label()).unwrap_or("—");
            match outcome {
                LinkOutcome::NoLinks => {
                    let _ = writeln!(md, "- External links: none (the policy was not consulted)");
                }
                LinkOutcome::Cached { books } => {
                    let _ = writeln!(md, "- External links: {books} book(s), choice **{choice}**, strategy `{}`: the values the file carries", r.link_strategy);
                }
                LinkOutcome::Linked(statuses) => {
                    let _ = writeln!(md, "- External links: choice **{choice}**, strategy `{}`:", r.link_strategy);
                    for st in statuses {
                        let _ = writeln!(md, "    - [{}] `{}` → `{}`: {:?}", st.index, st.target, st.path.display(), st.state);
                    }
                    let _ = writeln!(md, "- Link diff (cells whose live value differs from the cache): **{}**", r.link_diff.len());
                    let shown = max_examples.max(50);
                    for line in r.link_diff.iter().take(shown) {
                        let _ = writeln!(md, "    - `{line}`");
                    }
                    if r.link_diff.len() > shown {
                        let _ = writeln!(md, "    - … {} more", r.link_diff.len() - shown);
                    }
                }
            }
        }
        if let Some(e) = &r.load_error {
            let _ = writeln!(md, "\n**The engine could not load this file:** `{e}`\n");
            continue;
        }
        if r.engine_sheets.len() != raw.sheets.len() {
            let _ = writeln!(md, "- The engine's sheets ({}): `{}`", r.engine_sheets.len(), r.engine_sheets.join("`, `"));
        }
        let _ = writeln!(md);

        let total: usize = r.classes.values().sum();
        let _ = writeln!(md, "### Comparison by class\n");
        let _ = writeln!(md, "| Class | Cells | Share |");
        let _ = writeln!(md, "|---|---:|---:|");
        for (class, n) in &r.classes {
            let _ = writeln!(md, "| {} | {} | {:.1}% |", class.label(), n, *n as f64 * 100.0 / total.max(1) as f64);
        }
        let _ = writeln!(md);
        for (class, exs) in &r.examples {
            let _ = writeln!(md, "**{}** — examples:\n", class.label());
            let _ = writeln!(md, "| Cell | Formula | Excel cached | Engine |");
            let _ = writeln!(md, "|---|---|---|---|");
            for ex in exs {
                let _ = writeln!(
                    md,
                    "| {}!{} | `{}` | {} | {} |",
                    ex.sheet,
                    ex.reference,
                    ex.formula.replace('|', "\\|"),
                    ex.expected.replace('|', "\\|"),
                    ex.actual.replace('|', "\\|")
                );
            }
            let _ = writeln!(md);
        }

        let _ = writeln!(md, "### Functions used\n");
        let _ = writeln!(md, "| Function | Known | Cells using it | Of which matched |");
        let _ = writeln!(md, "|---|---|---:|---:|");
        for (f, st) in &r.functions {
            let _ = writeln!(md, "| {f} | {} | {} | {} |", if probe.known(f) { "yes" } else { "**no**" }, st.cells, st.matched);
        }
        let _ = writeln!(md);

        if r.save_time.is_some() || r.save_error.is_some() {
            let _ = writeln!(md, "### Save round trip (the fork's writer)\n");
            if let Some(e) = &r.save_error {
                let _ = writeln!(md, "**save_to_xlsx failed:** `{e}`\n");
            } else {
                let count = |p: &str| r.roundtrip.iter().filter(|(_, v)| v.starts_with(p)).count();
                let _ = writeln!(
                    md,
                    "Parts: {} identical, {} changed, {} dropped, {} added.\n",
                    count("identical"),
                    count("changed"),
                    count("DROPPED"),
                    count("added")
                );
                let _ = writeln!(md, "| Part | Verdict |");
                let _ = writeln!(md, "|---|---|");
                for (p, v) in &r.roundtrip {
                    let _ = writeln!(md, "| `{p}` | {v} |");
                }
                let _ = writeln!(md);
                if let Some(e) = &r.reload_error {
                    let _ = writeln!(md, "**Reloading the saved file failed:** `{e}`\n");
                } else if !r.reload_classes.is_empty() {
                    let _ = writeln!(md, "Reloaded the saved file and compared against the ORIGINAL cached values:\n");
                    let _ = writeln!(md, "| Class | Cells |");
                    let _ = writeln!(md, "|---|---:|");
                    for (class, n) in &r.reload_classes {
                        let _ = writeln!(md, "| {} | {} |", class.label(), n);
                    }
                    let _ = writeln!(md);
                }
            }
        }
    }

    let _ = writeln!(md, "## All functions across the files\n");
    let mut union: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in reports {
        for (f, st) in &r.functions {
            let e = union.entry(f.clone()).or_default();
            e.0 += st.cells;
            e.1 += st.matched;
        }
    }
    let _ = writeln!(md, "| Function | Known | Cells | Matched |");
    let _ = writeln!(md, "|---|---|---:|---:|");
    for (f, (c, m)) in &union {
        let _ = writeln!(md, "| {f} | {} | {c} | {m} |", if probe.known(f) { "yes" } else { "**no**" });
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_census_drops_prefixes_and_ignores_strings() {
        let names = function_names(
            r#"_xlfn.LET(_xlpm.t,[1]DB!$A$1,SUBSTITUTE(_xlpm.t,"FOO(",CHAR(1)))&_xlfn._xlws.FILTER(A1:A2,B1:B2)"#,
        );
        assert_eq!(names, vec!["LET", "SUBSTITUTE", "CHAR", "FILTER"]);
    }

    #[test]
    fn column_letters() {
        assert_eq!(column_index("A"), 1);
        assert_eq!(column_index("Z"), 26);
        assert_eq!(column_index("AA"), 27);
        assert_eq!(column_index("AW"), 49);
        assert_eq!(column_index("XFD"), 16384);
    }

    #[test]
    fn the_probe_knows_let_and_not_a_made_up_name() {
        let mut probe = Probe::default();
        assert!(probe.knows("LET"));
        assert!(probe.knows("SUBSTITUTE"));
        assert!(!probe.knows("NOSUCHFUNCTION"));
        assert!(probe.known("LET"));
        assert!(!probe.known("NEVERASKED"));
    }
}
