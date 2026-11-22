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
    /// A formula cell with no `<v>` at all (never calculated, or a dynamic-array cell whose value
    /// lives elsewhere).
    Empty,
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

static RE_SI: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<si>(.*?)</si>"));
static RE_T: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<t(?:\s[^>]*)?>(.*?)</t>"));
static RE_REL: LazyLock<Regex> = LazyLock::new(|| re(r#"<Relationship\s[^>]*?Id="([^"]+)"[^>]*?Target="([^"]+)""#));
static RE_REL2: LazyLock<Regex> = LazyLock::new(|| re(r#"<Relationship\s[^>]*?Target="([^"]+)"[^>]*?Id="([^"]+)""#));
static RE_SHEET: LazyLock<Regex> = LazyLock::new(|| re(r#"<sheet\s[^>]*?name="([^"]+)"[^>]*?r:id="([^"]+)""#));
static RE_CELL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)<c r="([A-Z]+)([0-9]+)"((?:\s+[a-zA-Z0-9:]+="[^"]*")*)\s*(?:/>|>(.*?)</c>)"#));
static RE_ATTR_T: LazyLock<Regex> = LazyLock::new(|| re(r#"\st="([^"]+)""#));
static RE_F: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<f(\s[^>]*)?(?:/>|>(.*?)</f>)"#));
static RE_F_SI: LazyLock<Regex> = LazyLock::new(|| re(r#"\ssi="([0-9]+)""#));
static RE_V: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<v(?:\s[^>]*)?>(.*?)</v>"#));
static RE_IS: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<is>(.*?)</is>"#));
static RE_STR: LazyLock<Regex> = LazyLock::new(|| re(r#""(?:[^"]|"")*""#));
static RE_FN: LazyLock<Regex> = LazyLock::new(|| re(r"([A-Za-z_][A-Za-z0-9_.]*)\s*\("));
static RE_EXT: LazyLock<Regex> = LazyLock::new(|| re(r"\[[0-9]+\]"));
static RE_APP: LazyLock<Regex> = LazyLock::new(|| re(r"<Application>([^<]*)</Application>"));
static RE_APP_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"<AppVersion>([0-9]+)\.[0-9]+</AppVersion>"));
static RE_FILE_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r#"<fileVersion\s[^>]*?appName="xl"[^>]*?lastEdited="([0-9]+)""#));

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&#13;", "\r")
        .replace("&#9;", "\t")
        .replace("&amp;", "&")
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

/// Reads the expected side of a workbook from its parts.
pub fn scan_raw(path: &Path) -> Result<RawWorkbook, OracleError> {
    let mut archive = open_zip(path)?;
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let e = archive.by_index(i).map_err(|source| OracleError::Zip { path: path.to_path_buf(), source })?;
        let name = e.name().map_err(|source| OracleError::Zip { path: path.to_path_buf(), source })?.to_string();
        parts.insert(name, (e.size(), e.crc32()));
    }

    // Shared strings: every <t> inside an <si>, concatenated (rich text runs).
    let shared: Vec<String> = match read_part(&mut archive, "xl/sharedStrings.xml") {
        Some(xml) => RE_SI
            .captures_iter(&xml)
            .map(|c| RE_T.captures_iter(&c[1]).map(|t| unescape(&t[1])).collect::<String>())
            .collect(),
        None => Vec::new(),
    };

    // Sheet order and names, to part names, through workbook.xml and its relationships.
    let workbook_xml = read_part(&mut archive, "xl/workbook.xml")
        .ok_or(OracleError::MissingPart { path: path.to_path_buf(), part: "xl/workbook.xml" })?;
    let rels_xml = read_part(&mut archive, "xl/_rels/workbook.xml.rels")
        .ok_or(OracleError::MissingPart { path: path.to_path_buf(), part: "xl/_rels/workbook.xml.rels" })?;
    let mut rels: HashMap<String, String> = HashMap::new();
    for c in RE_REL.captures_iter(&rels_xml) {
        rels.insert(c[1].to_string(), c[2].to_string());
    }
    for c in RE_REL2.captures_iter(&rels_xml) {
        rels.entry(c[2].to_string()).or_insert_with(|| c[1].to_string());
    }
    let sheets: Vec<(String, String)> = RE_SHEET
        .captures_iter(&workbook_xml)
        .map(|c| {
            let target = rels.get(&c[2]).cloned().unwrap_or_default();
            let part = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_string(),
                None => format!("xl/{target}"),
            };
            (unescape(&c[1]), part)
        })
        .collect();
    let defined_names = workbook_xml.matches("<definedName ").count();
    let external_links = workbook_xml.matches("<externalReference ").count();

    let mut external_cached_cells = 0;
    for name in parts.keys().cloned().collect::<Vec<_>>() {
        if name.starts_with("xl/externalLinks/externalLink")
            && name.ends_with(".xml")
            && let Some(xml) = read_part(&mut archive, &name)
        {
            external_cached_cells += xml.matches("<cell ").count();
        }
    }

    let mut formula_cells = Vec::new();
    let mut value_cells = 0usize;
    for (sheet_name, part) in &sheets {
        let Some(xml) = read_part(&mut archive, part) else { continue };
        let mut shared_masters: HashMap<u32, String> = HashMap::new();
        for c in RE_CELL.captures_iter(&xml) {
            let col_letters = &c[1];
            let row: u32 = c[2]
                .parse()
                .map_err(|_| OracleError::BadReference { path: path.to_path_buf(), reference: c[0].chars().take(40).collect() })?;
            let attrs = c.get(3).map(|m| m.as_str()).unwrap_or("");
            let inner = c.get(4).map(|m| m.as_str()).unwrap_or("");
            let t = RE_ATTR_T.captures(attrs).map(|m| m[1].to_string());

            let Some(fc) = RE_F.captures(inner) else {
                if inner.contains("<v>") || inner.contains("<is>") {
                    value_cells += 1;
                }
                continue;
            };
            let fattrs = fc.get(1).map(|m| m.as_str()).unwrap_or("");
            let text = fc.get(2).map(|m| unescape(m.as_str())).unwrap_or_default();
            let ftype = RE_ATTR_T.captures(fattrs).map(|m| m[1].to_string());
            let si = RE_F_SI.captures(fattrs).and_then(|m| m[1].parse::<u32>().ok());
            let array = ftype.as_deref() == Some("array");
            let (formula, shared_copy) = match (ftype.as_deref(), si) {
                (Some("shared"), Some(si)) if !text.is_empty() => {
                    shared_masters.insert(si, text.clone());
                    (text, false)
                }
                (Some("shared"), Some(si)) => (shared_masters.get(&si).cloned().unwrap_or_default(), true),
                _ => (text, false),
            };

            let v = RE_V.captures(inner).map(|m| unescape(&m[1])).or_else(|| inner.contains("<v/>").then(String::new));
            let expected = match (t.as_deref(), v) {
                (_, None) => match RE_IS.captures(inner) {
                    Some(is) => Expected::Text(RE_T.captures_iter(&is[1]).map(|t| unescape(&t[1])).collect()),
                    None => Expected::Empty,
                },
                (Some("s"), Some(v)) => {
                    Expected::Text(v.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default())
                }
                (Some("str"), Some(v)) | (Some("inlineStr"), Some(v)) => Expected::Text(v),
                (Some("b"), Some(v)) => Expected::Bool(v.trim() == "1"),
                (Some("e"), Some(v)) => Expected::Error(v),
                // a number Excel can cache is finite: `inf`, `NaN` or `1E+999` in a `<v>` is
                // read as the text it is, never as a number every engine value would equal
                (_, Some(v)) => match v.trim().parse::<f64>() {
                    Ok(n) if n.is_finite() => Expected::Number(n),
                    _ => Expected::Text(v),
                },
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
            (None | Some(Seen::Empty), Expected::Empty) => true,
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
}

fn fmt_expected(e: &Expected) -> String {
    match e {
        Expected::Number(n) => format!("{n}"),
        Expected::Text(t) => format!("{t:?}"),
        Expected::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        Expected::Error(e) => e.clone(),
        Expected::Empty => "(empty)".into(),
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
        (E::Empty, A::Text(b)) => b.is_empty(),
        (E::Bool(a), A::Bool(b)) => a == b,
        (E::Error(a), A::Error(b)) => a == b,
        (E::Empty, A::Empty) => true,
        _ => false,
    };
    if matched {
        return Class::Match;
    }
    if matches!(actual, A::NoSheet) {
        return Class::MissingSheet;
    }
    if cell.array {
        return Class::Spill;
    }
    if has_external {
        return Class::ExternalLink;
    }
    match (&cell.expected, actual) {
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
        (E::Empty, _) | (_, A::Empty) => Class::EmptyVsValue,
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
    let ty = model.get_cell_type(si, row, col).unwrap_or(CellType::Number);
    match model.get_cell_value_by_index(si, row, col) {
        Ok(CellValue::None) => Actual::Empty,
        Ok(CellValue::Number(n)) => Actual::Number(n),
        Ok(CellValue::Boolean(b)) => Actual::Bool(b),
        Ok(CellValue::String(s)) => {
            if ty == CellType::ErrorValue || (s.starts_with('#') && s.ends_with(['?', '!'])) {
                Actual::Error(s)
            } else {
                Actual::Text(s)
            }
        }
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
    rep.calamine = Some(calamine_check(path, &raw));

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
