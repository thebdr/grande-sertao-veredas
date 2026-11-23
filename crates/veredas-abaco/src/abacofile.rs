//! The `.abaco` file (the plan, "The `.abaco` file"; `docs/abaco-file.md`): one zip of plain-text
//! parts, readable with `unzip` and an editor, a workbook born in Ábaco and at the same time the
//! session that works on other files.
//!
//! ```text
//! manifest.toml                 who wrote it, the workbook's name, its sheets in order, its macros
//! workbook.toml                 styles, defined names, metadata, settings, theme, tables, links
//! sheets/01-<name>.toml         a sheet: its structure, then its cells, one line per cell
//! macros/<name>.py              the macros the workspace carries (never run at open)
//! workspace/references.toml     the foreign files it works on, each fingerprinted
//! workspace/decisions.toml      what its tools decided, oldest first
//! workspace/queue.toml          what its tools queued, in order
//! workspace/state/<tool>.toml   a tool's own state
//! workspace/log.txt             the log, one escaped entry per line
//! ```
//!
//! Every TOML part states its `format` and `version` first. A part this Ábaco does not read is
//! kept and written back as it came, so a newer Ábaco's additions survive an older one's save.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;

use ironcalc_base::Model;
use ironcalc_base::cf_types::ConditionalFormatting;
use ironcalc_base::expressions::token::get_error_by_english_name;
use ironcalc_base::external::ExternalBook;
use ironcalc_base::sheet_data::SheetData;
use ironcalc_base::types::{
    ArrayKind, Cell, Col, Color, Comment, DefinedName, Link, MergedCell, Metadata, Row, SheetState, Styles, Table, Theme,
    Workbook as EngineWorkbook, WorkbookSettings, Worksheet,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::a1::Addr;
use crate::book::{Content, LANGUAGE, ModelError, Origin, Value, Workbook};
use crate::workspace::{Reference, Workspace};

/// The version of every part this Ábaco writes and reads.
pub const VERSION: u32 = 1;
/// The manifest's format; every other part's is `veredas-abaco-<part>`.
pub const FORMAT: &str = "veredas-abaco";
const WORKBOOK_FORMAT: &str = "veredas-abaco-workbook";
const SHEET_FORMAT: &str = "veredas-abaco-sheet";
const REFERENCES_FORMAT: &str = "veredas-abaco-references";
const RECORDS_FORMAT: &str = "veredas-abaco-records";
const STATE_FORMAT: &str = "veredas-abaco-state";

const MANIFEST: &str = "manifest.toml";
const WORKBOOK: &str = "workbook.toml";
const REFERENCES: &str = "workspace/references.toml";
const DECISIONS: &str = "workspace/decisions.toml";
const QUEUE: &str = "workspace/queue.toml";
const STATE_DIR: &str = "workspace/state/";
const LOG: &str = "workspace/log.txt";

/// A `.abaco` file in memory: the workbook, the macros it carries, the session around it, and the
/// parts this Ábaco does not read, kept as they came.
pub struct AbacoFile {
    pub book: Workbook,
    /// Macro sources by name: letters, digits and `_`, not opening with a digit.
    pub macros: BTreeMap<String, String>,
    pub workspace: Workspace,
    kept: BTreeMap<String, Vec<u8>>,
}

impl AbacoFile {
    /// A workbook with no macros and an empty session.
    pub fn new(book: Workbook) -> AbacoFile {
        AbacoFile { book, macros: BTreeMap::new(), workspace: Workspace::default(), kept: BTreeMap::new() }
    }

    /// The parts read from the file that this Ábaco does not know, by name.
    pub fn kept_parts(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.kept
    }
}

// ---- the parts' shapes ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    /// The Ábaco that wrote the file (`veredas-abaco 2.0.0`).
    written_by: String,
    name: String,
    #[serde(default)]
    macros: Vec<String>,
    sheets: Vec<SheetEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetEntry {
    name: String,
    part: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkbookPart {
    format: String,
    version: u32,
    settings: WorkbookSettings,
    metadata: Metadata,
    #[serde(default)]
    defined_names: Vec<DefinedName>,
    styles: Styles,
    theme: Theme,
    #[serde(default)]
    tables: BTreeMap<String, Table>,
    #[serde(default)]
    external_books: Vec<ExternalBook>,
}

/// A sheet's structure; its cells follow in the part's `[cells]` table.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetPart {
    format: String,
    version: u32,
    name: String,
    sheet_id: u32,
    state: SheetState,
    color: Color,
    dimension: String,
    frozen_rows: i32,
    frozen_columns: i32,
    show_grid_lines: bool,
    #[serde(default)]
    cols: Vec<Col>,
    #[serde(default)]
    rows: Vec<Row>,
    #[serde(default)]
    merged_cells: Vec<MergedCell>,
    #[serde(default)]
    comments: Vec<Comment>,
    #[serde(default)]
    conditional_formatting: Vec<ConditionalFormatting>,
    /// Hyperlinks as `[row, column, link]`, in row-major order.
    #[serde(default)]
    links: Vec<(i32, i32, Link)>,
    #[serde(default, skip_serializing)]
    cells: BTreeMap<String, CellEntry>,
}

/// One cell of a sheet's `[cells]` table: exactly one of `n`, `s`, `b`, `e` (a value), `f`
/// (a formula, with `v` or `e` its last value, for the reader only), `array` (with its `size`,
/// columns then rows) or `dynamic`; or none of them, a cell that carries only its `style`.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CellEntry {
    n: Option<f64>,
    s: Option<String>,
    b: Option<bool>,
    e: Option<String>,
    f: Option<String>,
    v: Option<serde_json::Value>,
    array: Option<String>,
    size: Option<(i32, i32)>,
    dynamic: Option<String>,
    #[serde(default)]
    style: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferencesPart {
    format: String,
    version: u32,
    #[serde(default)]
    reference: Vec<Reference>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordsPart {
    format: String,
    version: u32,
    #[serde(default)]
    record: Vec<toml::Table>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatePart {
    format: String,
    version: u32,
    #[serde(default)]
    state: toml::Table,
}

// ---- names, numbers, text ------------------------------------------------------------------------

/// `01-R1.S1.C1`: the sheet's position and a file-safe spelling of its name.
fn sheet_stem(index: usize, name: &str) -> String {
    let safe: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ') { c } else { '_' }).collect();
    format!("{:02}-{}", index + 1, safe.trim())
}

/// A macro's or a tool's name: letters, digits and `_`, not opening with a digit.
pub fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// A number as the file writes it: a whole number under 10^15 as an integer (`42`), any other in
/// the shortest text that reads back as the same double (`0.1`, `1e300`), so it reads back exactly.
pub fn number_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 && !(n == 0.0 && n.is_sign_negative()) {
        format!("{}", n as i64)
    } else if n.is_nan() {
        "nan".into()
    } else if n.is_infinite() {
        if n > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        // Rust's shortest round-trip form, which TOML reads as a float (`1e300`, `-0.0`)
        format!("{n:?}")
    }
}

/// A TOML basic string holding `s` exactly.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// A log entry on one line: `\`, tab, line feed and carriage return escaped.
pub fn escape_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// The entry a log line holds; a backslash before anything but `\`, `t`, `n` or `r` refused.
pub fn unescape_line(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => return Err(format!("`\\{other}` is no escape")),
            None => return Err("a line ends in a lone `\\`".into()),
        }
    }
    Ok(out)
}

// ---- writing -------------------------------------------------------------------------------------

fn toml_text<T: Serialize>(part: &str, value: &T) -> Result<String, ModelError> {
    toml::to_string(value).map_err(|e| ModelError::Engine(format!("{part}: {e}")))
}

/// A part holding the engine's own structures, as TOML: they serialise through JSON, whose null
/// (the engine's `None`, a colour that is none) TOML cannot hold, so a null is written `nan`,
/// which no JSON value holds and no structure of the engine keeps.
fn engine_text<T: Serialize>(part: &str, value: &T) -> Result<String, ModelError> {
    let bad = |e: String| ModelError::Engine(format!("{part}: {e}"));
    let json = serde_json::to_value(value).map_err(|e| bad(e.to_string()))?;
    let toml::Value::Table(mut table) = json_to_toml(json).map_err(bad)? else {
        return Err(bad("not a table".into()));
    };
    // the stamp first, as every part states it; the rest in the order TOML sorts it
    let (Some(toml::Value::String(format)), Some(toml::Value::Integer(version))) = (table.remove("format"), table.remove("version"))
    else {
        return Err(bad("no stamp".into()));
    };
    let body = toml::to_string(&table).map_err(|e| bad(e.to_string()))?;
    Ok(format!("format = {}\nversion = {version}\n{body}", quoted(&format)))
}

fn json_to_toml(j: serde_json::Value) -> Result<toml::Value, String> {
    use serde_json::Value as J;
    Ok(match j {
        J::Null => toml::Value::Float(f64::NAN),
        J::Bool(b) => toml::Value::Boolean(b),
        J::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => toml::Value::Integer(i),
            (None, Some(f)) if n.is_f64() => toml::Value::Float(f),
            _ => return Err(format!("{n} is past what TOML's integers hold")),
        },
        J::String(s) => toml::Value::String(s),
        J::Array(a) => toml::Value::Array(a.into_iter().map(json_to_toml).collect::<Result<_, _>>()?),
        J::Object(o) => toml::Value::Table(o.into_iter().map(|(k, v)| Ok((k, json_to_toml(v)?))).collect::<Result<_, String>>()?),
    })
}

fn toml_to_json(t: toml::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    Ok(match t {
        toml::Value::Float(f) if f.is_nan() => J::Null,
        toml::Value::Float(f) => J::Number(serde_json::Number::from_f64(f).ok_or_else(|| format!("{f} is no finite number"))?),
        toml::Value::Integer(i) => J::from(i),
        toml::Value::Boolean(b) => J::Bool(b),
        toml::Value::String(s) => J::String(s),
        toml::Value::Datetime(d) => J::String(d.to_string()),
        toml::Value::Array(a) => J::Array(a.into_iter().map(toml_to_json).collect::<Result<_, _>>()?),
        toml::Value::Table(o) => J::Object(o.into_iter().map(|(k, v)| Ok((k, toml_to_json(v)?))).collect::<Result<_, String>>()?),
    })
}

/// A formula as the file writes it: the engine's user spelling without its leading `=`.
fn bare(formula: &str) -> &str {
    formula.strip_prefix('=').unwrap_or(formula)
}

/// The `[cells]` table of one sheet, one line per cell, row-major.
fn cells_text(book: &Workbook, sheet: u32) -> Result<String, ModelError> {
    let mut out = String::from("\n[cells]\n");
    for (addr, style) in book.cells(sheet)? {
        let styled = |out: &mut String| {
            if style != 0 {
                let _ = write!(out, ", style = {style}");
            }
        };
        let line = match book.content(sheet, addr)? {
            Content::Spill if style == 0 => continue,
            Content::Empty | Content::Spill | Content::Value(Value::Empty) => format!("{addr} = {{ style = {style} }}"),
            Content::Value(v) => {
                let mut l = match v {
                    Value::Number(n) => format!("{addr} = {{ n = {}", number_text(n)),
                    Value::Text(s) => format!("{addr} = {{ s = {}", quoted(&s)),
                    Value::Bool(b) => format!("{addr} = {{ b = {b}"),
                    Value::Error(e) => format!("{addr} = {{ e = {}", quoted(&e)),
                    Value::Empty => unreachable!("matched above"),
                };
                styled(&mut l);
                l + " }"
            }
            Content::Formula(f) => {
                let mut l = format!("{addr} = {{ f = {}", quoted(bare(&f)));
                match book.value_now(sheet, addr)? {
                    Value::Empty => {}
                    Value::Number(n) => {
                        let _ = write!(l, ", v = {}", number_text(n));
                    }
                    Value::Text(s) => {
                        let _ = write!(l, ", v = {}", quoted(&s));
                    }
                    Value::Bool(b) => {
                        let _ = write!(l, ", v = {b}");
                    }
                    Value::Error(e) => {
                        let _ = write!(l, ", e = {}", quoted(&e));
                    }
                }
                styled(&mut l);
                l + " }"
            }
            Content::ArrayFormula { formula, columns, rows } => {
                let mut l = format!("{addr} = {{ array = {}, size = [{columns}, {rows}]", quoted(bare(&formula)));
                styled(&mut l);
                l + " }"
            }
            Content::DynamicFormula(f) => {
                let mut l = format!("{addr} = {{ dynamic = {}", quoted(bare(&f)));
                styled(&mut l);
                l + " }"
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

fn sheet_text(book: &Workbook, sheet: u32, ws: &Worksheet) -> Result<String, ModelError> {
    let mut links: Vec<(i32, i32, Link)> = ws.links.iter().map(|(&(r, c), l)| (r, c, l.clone())).collect();
    links.sort_by_key(|&(r, c, _)| (r, c));
    let part = SheetPart {
        format: SHEET_FORMAT.into(),
        version: VERSION,
        name: ws.name.clone(),
        sheet_id: ws.sheet_id,
        state: ws.state.clone(),
        color: ws.color.clone(),
        dimension: ws.dimension.clone(),
        frozen_rows: ws.frozen_rows,
        frozen_columns: ws.frozen_columns,
        show_grid_lines: ws.show_grid_lines,
        cols: ws.cols.clone(),
        rows: ws.rows.clone(),
        merged_cells: ws.merged_cells.clone(),
        comments: ws.comments.clone(),
        conditional_formatting: ws.conditional_formatting.clone(),
        links,
        cells: BTreeMap::new(),
    };
    Ok(engine_text(&ws.name, &part)? + &cells_text(book, sheet)?)
}

/// Every part of the file, by name, in the order they are written.
pub fn parts(file: &AbacoFile) -> Result<Vec<(String, Vec<u8>)>, ModelError> {
    let model = file.book.model();
    let ewb = &model.workbook;
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut entries = Vec::new();
    let mut sheets = Vec::new();
    for (i, ws) in ewb.worksheets.iter().enumerate() {
        let part = format!("sheets/{}.toml", sheet_stem(i, &ws.name));
        sheets.push((part.clone(), sheet_text(&file.book, i as u32, ws)?.into_bytes()));
        entries.push(SheetEntry { name: ws.name.clone(), part });
    }
    for name in file.macros.keys() {
        if !is_identifier(name) {
            return Err(ModelError::Engine(format!("macro {name:?}: letters, digits and `_`, not opening with a digit")));
        }
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        written_by: format!("veredas-abaco {}", env!("CARGO_PKG_VERSION")),
        name: ewb.name.clone(),
        macros: file.macros.keys().cloned().collect(),
        sheets: entries,
    };
    let book = WorkbookPart {
        format: WORKBOOK_FORMAT.into(),
        version: VERSION,
        settings: ewb.settings.clone(),
        metadata: ewb.metadata.clone(),
        defined_names: ewb.defined_names.clone(),
        styles: ewb.styles.clone(),
        theme: ewb.theme.clone(),
        tables: ewb.tables.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        external_books: ewb.external_books.clone(),
    };
    out.push((MANIFEST.into(), toml_text(MANIFEST, &manifest)?.into_bytes()));
    out.push((WORKBOOK.into(), engine_text(WORKBOOK, &book)?.into_bytes()));
    out.append(&mut sheets);
    for (name, source) in &file.macros {
        out.push((format!("macros/{name}.py"), source.clone().into_bytes()));
    }
    let ws = &file.workspace;
    if !ws.references.is_empty() {
        let part = ReferencesPart { format: REFERENCES_FORMAT.into(), version: VERSION, reference: ws.references.clone() };
        out.push((REFERENCES.into(), toml_text(REFERENCES, &part)?.into_bytes()));
    }
    for (name, records) in [(DECISIONS, &ws.decisions), (QUEUE, &ws.queue)] {
        if !records.is_empty() {
            let part = RecordsPart { format: RECORDS_FORMAT.into(), version: VERSION, record: records.clone() };
            out.push((name.into(), toml_text(name, &part)?.into_bytes()));
        }
    }
    for (tool, state) in &ws.state {
        if !is_identifier(tool) {
            return Err(ModelError::Engine(format!("tool {tool:?}: letters, digits and `_`, not opening with a digit")));
        }
        let name = format!("{STATE_DIR}{tool}.toml");
        let part = StatePart { format: STATE_FORMAT.into(), version: VERSION, state: state.clone() };
        out.push((name.clone(), toml_text(&name, &part)?.into_bytes()));
    }
    if !ws.log.is_empty() {
        let text: String = ws.log.iter().map(|l| escape_line(l) + "\n").collect();
        out.push((LOG.into(), text.into_bytes()));
    }
    for (name, bytes) in &file.kept {
        if !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), bytes.clone()));
        }
    }
    Ok(out)
}

/// The zip of `parts`, each entry deflated and dated at the format's epoch (1980-01-01), so the
/// same parts make the same bytes.
pub fn zip_of(parts: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, bytes) in parts {
        zip.start_file(name.as_str(), options).map_err(|e| format!("{name}: {e}"))?;
        zip.write_all(bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Writes the file at `path`, through a file beside it renamed into place, the file it replaces
/// kept as `<name>.bak`. The workbook is evaluated first, so each formula's last value is current.
pub fn write(file: &mut AbacoFile, path: &Path) -> Result<(), ModelError> {
    file.book.ensure_calculated();
    let bytes = zip_of(&parts(file)?).map_err(|e| ModelError::file(path, e))?;
    crate::files::atomic_write(path, |mut f| {
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    })
    .map_err(|e| ModelError::file(path, e))
}

// ---- reading -------------------------------------------------------------------------------------

struct Parts<'p> {
    path: &'p Path,
    all: BTreeMap<String, Vec<u8>>,
}

impl Parts<'_> {
    fn bad(&self, message: impl std::fmt::Display) -> ModelError {
        ModelError::file(self.path, message)
    }

    fn text(&mut self, name: &str) -> Result<String, ModelError> {
        let bytes = self.all.remove(name).ok_or_else(|| self.bad(format!("no part `{name}`")))?;
        String::from_utf8(bytes).map_err(|_| self.bad(format!("{name}: not UTF-8")))
    }

    /// A TOML part: its stamp checked before its body is read.
    fn toml<T: DeserializeOwned>(&mut self, name: &str, format: &str) -> Result<T, ModelError> {
        let text = self.text(name)?;
        stamp(&text, name, format).map_err(|e| self.bad(format!("{name}: {e}")))?;
        toml::from_str(&text).map_err(|e| self.bad(format!("{name}: {e}")))
    }

    /// A part holding the engine's structures ([`engine_text`]): its stamp, then its body read as
    /// TOML, `nan` as the engine's none.
    fn engine<T: DeserializeOwned>(&mut self, name: &str, format: &str) -> Result<T, ModelError> {
        let text = self.text(name)?;
        stamp(&text, name, format).map_err(|e| self.bad(format!("{name}: {e}")))?;
        let table: toml::Table = toml::from_str(&text).map_err(|e| self.bad(format!("{name}: {e}")))?;
        let json = toml_to_json(toml::Value::Table(table)).map_err(|e| self.bad(format!("{name}: {e}")))?;
        // the engine's structures take a key they do not know without a word: a key nothing
        // reads, at any depth, is refused by its path, as every file of the suite refuses one
        let mut unread = Vec::new();
        let value = serde_ignored::deserialize(json, |path| unread.push(path.to_string()))
            .map_err(|e| self.bad(format!("{name}: {e}")))?;
        match unread.first() {
            Some(path) => Err(self.bad(format!("{name}: `{path}` is a key nothing reads"))),
            None => Ok(value),
        }
    }
}

/// The suite's one header reader (C-045): core's own source, compiled here by path, so the stamp
/// is read by the very code every other file of the suite is read by, its order and its texts.
/// Ábaco cannot take `veredas-core` as a dependency while core carries its network layers
/// unconditionally (C-220 holds no plant connection); the day core puts them behind a feature this
/// becomes `use veredas_core::schema` (docs/questions.md Q345).
// DECISION PENDING (Q345)
#[allow(dead_code)]
#[path = "../../veredas-core/src/schema.rs"]
mod schema;

/// The stamp of a part: `format` as `format` says and the `version` this Ábaco reads, through
/// core's reader, the part named by its path.
fn stamp(text: &str, part: &str, format: &str) -> Result<(), String> {
    schema::check(text, part, format, VERSION).map_err(|e| e.to_string())
}

/// One cell of a sheet as the file states it, checked.
#[derive(Debug)]
enum Entry {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(ironcalc_base::expressions::token::Error),
    Formula(String),
    Array(String, i32, i32),
    Dynamic(String),
    Style,
}

fn entry(e: CellEntry) -> Result<Entry, String> {
    let kinds = [e.n.is_some(), e.s.is_some(), e.b.is_some(), e.f.is_some(), e.array.is_some(), e.dynamic.is_some()]
        .iter()
        .filter(|k| **k)
        .count()
        + usize::from(e.e.is_some() && e.f.is_none());
    if kinds > 1 {
        return Err("more than one of `n`, `s`, `b`, `e`, `f`, `array`, `dynamic`".into());
    }
    if e.v.is_some() && e.f.is_none() {
        return Err("`v` without `f`".into());
    }
    if e.size.is_some() != e.array.is_some() {
        return Err("`size` goes with `array`, and only with it".into());
    }
    let error = |code: &str| get_error_by_english_name(code).ok_or_else(|| format!("{code:?} is no error"));
    Ok(if let Some(n) = e.n {
        if !n.is_finite() {
            return Err(format!("{n} is no finite number"));
        }
        Entry::Number(n)
    } else if let Some(s) = e.s {
        Entry::Text(s)
    } else if let Some(b) = e.b {
        Entry::Bool(b)
    } else if let Some(f) = e.f {
        if let Some(code) = &e.e {
            error(code)?;
        }
        Entry::Formula(f)
    } else if let Some(code) = e.e {
        Entry::Error(error(&code)?)
    } else if let Some(f) = e.array {
        let (w, h) = e.size.unwrap_or((1, 1));
        if w < 1 || h < 1 {
            return Err(format!("an array of {w} by {h}"));
        }
        Entry::Array(f, w, h)
    } else if let Some(f) = e.dynamic {
        Entry::Dynamic(f)
    } else {
        Entry::Style
    })
}

/// Reads a `.abaco` file into a workbook and its session, evaluated. Nothing it carries runs.
pub fn read(path: &Path) -> Result<AbacoFile, ModelError> {
    let file = std::fs::File::open(path).map_err(|e| ModelError::file(path, e))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| ModelError::file(path, format!("not a .abaco file (a zip): {e}")))?;
    let mut all = BTreeMap::new();
    for i in 0..zip.len() {
        let name = zip
            .name_for_index(i)
            .ok_or_else(|| ModelError::file(path, format!("zip entry {i} has no name")))?
            .map_err(|e| ModelError::file(path, format!("zip entry {i}: {e}")))?
            .to_string();
        let mut entry = zip.by_index(i).map_err(|e| ModelError::file(path, format!("{name}: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| ModelError::file(path, format!("{name}: {e}")))?;
        // the zip reader keeps one entry of a name (a zip holding two is malformed)
        all.insert(name, bytes);
    }
    let mut parts = Parts { path, all };
    let manifest: Manifest = parts.toml(MANIFEST, FORMAT)?;
    let book: WorkbookPart = parts.engine(WORKBOOK, WORKBOOK_FORMAT)?;

    // The engine's workbook, values and styles placed as they are (text never parsed again, styles
    // keeping their indices); formulas entered once the model exists.
    let mut shared: Vec<String> = Vec::new();
    let mut shared_index: HashMap<String, i32> = HashMap::new();
    let mut worksheets = Vec::new();
    let mut formulas: Vec<(u32, String, Addr, Entry, i32)> = Vec::new();
    for (i, sheet) in manifest.sheets.iter().enumerate() {
        let part: SheetPart = parts.engine(&sheet.part, SHEET_FORMAT)?;
        if part.name != sheet.name {
            return Err(parts.bad(format!("{}: the sheet `{}`, where the manifest says `{}`", sheet.part, part.name, sheet.name)));
        }
        let mut ws = Worksheet {
            dimension: part.dimension,
            cols: part.cols,
            rows: part.rows,
            name: part.name,
            sheet_data: SheetData::default(),
            shared_formulas: Vec::new(),
            sheet_id: part.sheet_id,
            state: part.state,
            color: part.color,
            merged_cells: part.merged_cells,
            comments: part.comments,
            frozen_rows: part.frozen_rows,
            frozen_columns: part.frozen_columns,
            show_grid_lines: part.show_grid_lines,
            conditional_formatting: part.conditional_formatting,
            links: part.links.into_iter().map(|(r, c, l)| ((r, c), l)).collect(),
        };
        for (key, cell) in part.cells {
            let bad = |e: String| parts.bad(format!("{} {key}: {e}", sheet.part));
            let addr = Addr::parse(&key).map_err(|e| bad(e.to_string()))?;
            let s = cell.style;
            let (row, column) = (addr.row, addr.column);
            match entry(cell).map_err(bad)? {
                Entry::Number(v) => ws.set_cell_with_number(row, column, v, s).map_err(bad)?,
                Entry::Text(t) => {
                    let si = *shared_index.entry(t.clone()).or_insert_with(|| {
                        shared.push(t);
                        (shared.len() - 1) as i32
                    });
                    ws.set_cell_with_string(row, column, si, s).map_err(bad)?;
                }
                Entry::Bool(v) => ws.set_cell_with_boolean(row, column, v, s).map_err(bad)?,
                Entry::Error(e) => ws.set_cell_with_error(row, column, e, s).map_err(bad)?,
                Entry::Style => ws.set_cell_style(row, column, s).map_err(bad)?,
                formula => {
                    ws.set_cell_style(row, column, s).map_err(bad)?;
                    formulas.push((i as u32, key, addr, formula, s));
                }
            }
        }
        worksheets.push(ws);
    }
    let engine = EngineWorkbook {
        shared_strings: shared,
        defined_names: book.defined_names,
        worksheets,
        styles: book.styles,
        name: manifest.name.clone(),
        settings: book.settings,
        metadata: book.metadata,
        tables: book.tables.into_iter().collect(),
        theme: book.theme,
        external_books: book.external_books,
    };
    let mut model = Model::from_workbook(engine, LANGUAGE).map_err(|e| parts.bad(e))?;
    // Entering a formula in a quote-prefixed cell derives a new style; every formula cell gets its
    // own style back below, so the derived ones are orphans, cut back to what the file holds.
    let styles_in_file = model.workbook.styles.cell_xfs.len();
    for (sheet, key, addr, formula, style) in formulas {
        let (row, column) = (addr.row, addr.column);
        let bad = |e: String| ModelError::file(path, format!("{} {key}: {e}", manifest.sheets[sheet as usize].part));
        let entered = match &formula {
            Entry::Array(f, w, h) => model.set_user_array_formula(sheet, row, column, *w, *h, &format!("={f}")),
            Entry::Formula(f) | Entry::Dynamic(f) => model.update_cell_with_formula(sheet, row, column, format!("={f}")),
            _ => Ok(()),
        };
        entered.map_err(bad)?;
        // The engine picks a formula's kind from its static analysis; the file states it, and the
        // file wins. The setter may have touched the style (a quote prefix): it is set back too.
        let ws = model.workbook.worksheet_mut(sheet).map_err(bad)?;
        let restored = match (&formula, ws.cell(row, column).map(|c| c.into_owned())) {
            (Entry::Formula(_), Some(Cell::ArrayFormula { f, kind: ArrayKind::Dynamic, .. })) => {
                ws.set_cell_with_formula(row, column, f, style)
            }
            (Entry::Dynamic(_), Some(Cell::CellFormula { f, .. })) => {
                ws.set_cell_with_dynamic_formula(row, column, f, style, 1, 1)
            }
            _ => ws.set_cell_style(row, column, style),
        };
        restored.map_err(bad)?;
    }
    model.workbook.styles.cell_xfs.truncate(styles_in_file);

    let mut macros = BTreeMap::new();
    for name in &manifest.macros {
        if !is_identifier(name) {
            return Err(parts.bad(format!("{MANIFEST}: macro {name:?}: letters, digits and `_`, not opening with a digit")));
        }
        macros.insert(name.clone(), parts.text(&format!("macros/{name}.py"))?);
    }
    let mut workspace = Workspace::default();
    if parts.all.contains_key(REFERENCES) {
        workspace.references = parts.toml::<ReferencesPart>(REFERENCES, REFERENCES_FORMAT)?.reference;
    }
    if parts.all.contains_key(DECISIONS) {
        workspace.decisions = parts.toml::<RecordsPart>(DECISIONS, RECORDS_FORMAT)?.record;
    }
    if parts.all.contains_key(QUEUE) {
        workspace.queue = parts.toml::<RecordsPart>(QUEUE, RECORDS_FORMAT)?.record;
    }
    let states: Vec<String> = parts
        .all
        .keys()
        .filter(|k| k.strip_prefix(STATE_DIR).and_then(|t| t.strip_suffix(".toml")).is_some_and(is_identifier))
        .cloned()
        .collect();
    for name in states {
        let tool = name[STATE_DIR.len()..name.len() - ".toml".len()].to_string();
        workspace.state.insert(tool, parts.toml::<StatePart>(&name, STATE_FORMAT)?.state);
    }
    if parts.all.contains_key(LOG) {
        let text = parts.text(LOG)?;
        for (n, line) in text.lines().enumerate() {
            workspace.log.push(unescape_line(line).map_err(|e| parts.bad(format!("{LOG} line {}: {e}", n + 1)))?);
        }
    }
    let mut book = Workbook::from_model(model, Origin::Abaco { path: path.to_path_buf() });
    book.recalc();
    Ok(AbacoFile { book, macros, workspace, kept: parts.all })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_round_trip_and_refuse_a_bad_escape() {
        for s in ["", "plain", "a\tb", "line1\nline2\r\n", "back\\slash", "\\t literal"] {
            assert_eq!(unescape_line(&escape_line(s)).unwrap(), s);
        }
        assert_eq!(escape_line("a\tb\\"), "a\\tb\\\\");
        assert_eq!(unescape_line("bad \\x").unwrap_err(), "`\\x` is no escape");
        assert_eq!(unescape_line("trailing \\").unwrap_err(), "a line ends in a lone `\\`");
    }

    #[test]
    fn numbers_read_back_exactly() {
        for n in [0.0, -0.0, 42.0, -7.0, 0.1, 1.0 / 3.0, 1e300, 1e-300, 123456789012345.0, 1e15, 2.5e-7, f64::MAX, f64::MIN_POSITIVE, 5e-324] {
            let t = number_text(n);
            let back: toml::Value = toml::from_str::<toml::Table>(&format!("x = {t}")).unwrap()["x"].clone();
            let back = back.as_float().or(back.as_integer().map(|i| i as f64)).unwrap();
            assert_eq!(back.to_bits(), n.to_bits(), "{t}");
        }
        assert_eq!(number_text(42.0), "42");
        assert_eq!(number_text(0.1), "0.1");
        assert_eq!(number_text(-0.0), "-0.0");
    }

    #[test]
    fn sheet_stems_are_file_safe_and_names_identifiers() {
        assert_eq!(sheet_stem(0, "R1.S1.C1"), "01-R1.S1.C1");
        assert_eq!(sheet_stem(11, "a/b:c*d"), "12-a_b_c_d");
        assert!(is_identifier("acerto_join"));
        assert!(!is_identifier("1x") && !is_identifier("../evil") && !is_identifier("") && !is_identifier("a-b"));
    }

    #[test]
    fn a_stamp_is_read_by_core_s_reader_format_first() {
        let ok = "format = \"veredas-abaco\"\nversion = 1\n";
        assert_eq!(stamp(ok, "manifest.toml", FORMAT), Ok(()));
        assert_eq!(
            stamp("format = \"veredas-abaco\"\nversion = 2\n", "manifest.toml", FORMAT).unwrap_err(),
            "version 2 of `veredas-abaco` is not known here (this reader knows 1)"
        );
        assert_eq!(
            stamp("format = \"veredas-abaco-sheet\"\nversion = 1\n", "manifest.toml", FORMAT).unwrap_err(),
            "`format` is `veredas-abaco-sheet`, not `veredas-abaco`"
        );
        // the format decided whole before the version: one past any integer type is no matter
        for version in ["18446744073709551615", "18446744073709551616", "-1"] {
            assert_eq!(
                stamp(&format!("format = \"veredas-other\"\nversion = {version}\n"), "manifest.toml", FORMAT).unwrap_err(),
                "`format` is `veredas-other`, not `veredas-abaco`",
                "{version}"
            );
        }
        assert_eq!(
            stamp("format = \"veredas-abaco\"\nversion = -1\n", "manifest.toml", FORMAT).unwrap_err(),
            "not manifest.toml: `version` is not a whole number from 0 to 4294967295"
        );
        assert_eq!(stamp("format = 1\nversion = 1\n", "manifest.toml", FORMAT).unwrap_err(), "not manifest.toml: `format` is not a string");
        assert_eq!(stamp("version = 1\n", "manifest.toml", FORMAT).unwrap_err(), "no `format`: manifest.toml states `format` and `version`");
        assert_eq!(stamp("format = \"veredas-abaco\"\n", "manifest.toml", FORMAT).unwrap_err(), "no `version`: manifest.toml states `format` and `version`");
    }

    #[test]
    fn a_cell_states_one_kind() {
        let parse = |t: &str| -> Result<Entry, String> {
            let c: CellEntry = toml::from_str(t).map_err(|e| e.to_string())?;
            entry(c)
        };
        assert!(matches!(parse("n = 42"), Ok(Entry::Number(n)) if n == 42.0));
        assert!(matches!(parse("n = 0.5\nstyle = 3"), Ok(Entry::Number(_))));
        assert!(matches!(parse("f = \"A1*2\"\nv = 84"), Ok(Entry::Formula(_))));
        assert!(matches!(parse("f = \"1/0\"\ne = \"#DIV/0!\""), Ok(Entry::Formula(_))));
        assert!(matches!(parse("e = \"#N/A\""), Ok(Entry::Error(_))));
        assert!(matches!(parse("array = \"A1:A3*2\"\nsize = [1, 3]"), Ok(Entry::Array(_, 1, 3))));
        assert!(matches!(parse("style = 5"), Ok(Entry::Style)));
        assert_eq!(parse("n = 1\ns = \"x\"").err().unwrap(), "more than one of `n`, `s`, `b`, `e`, `f`, `array`, `dynamic`");
        assert_eq!(parse("n = 1\nv = 2").err().unwrap(), "`v` without `f`");
        assert_eq!(parse("array = \"A1\"").err().unwrap(), "`size` goes with `array`, and only with it");
        assert_eq!(parse("e = \"#NOPE!\"").err().unwrap(), "\"#NOPE!\" is no error");
        assert_eq!(parse("array = \"A1\"\nsize = [0, 3]").err().unwrap(), "an array of 0 by 3");
        assert!(parse("x = 1").unwrap_err().contains("unknown field"));
    }
}
