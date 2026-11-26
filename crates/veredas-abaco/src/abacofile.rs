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

use std::collections::{BTreeMap, HashMap, HashSet};
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
    v: Option<toml::Value>,
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

/// A TOML basic string holding `s` exactly, on one line: `"` and `\` escaped, and every control
/// character (U+0000 to U+001F, U+007F to U+009F, a tab too) and every other line break Unicode
/// names (U+2028, U+2029) written as its escape, so a text with a line break keeps its cell on one
/// line for any reader, Python's `splitlines` among them.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c) || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A log entry on one line: `\`, tab, line feed and carriage return escaped (`\\`, `\t`, `\n`,
/// `\r`), and every other control character and Unicode line break (U+000B, U+000C, U+001C to
/// U+001E, U+0085, U+2028, U+2029 among them) as `\uXXXX`, so no reader splits an entry.
pub fn escape_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c) || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
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
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                // four hex digits, no sign (`from_str_radix` would take `+041`)
                let c = (hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                    .then(|| u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32))
                    .flatten()
                    .ok_or_else(|| format!("`\\u{hex}` is no character"))?;
                out.push(c);
            }
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
/// which no JSON value holds. JSON writes a number that is no finite number (a NaN or an infinity,
/// which a malformed workbook can give the engine) as null too, where the engine's structure takes
/// no none: such a number refuses the write by its path ([`non_finite_at`], found in the value
/// itself), so every file written reads back.
fn engine_text<T: Serialize + DeserializeOwned>(part: &str, value: &T) -> Result<String, ModelError> {
    let bad = |e: String| ModelError::Engine(format!("{part}: {e}"));
    if let Some(path) = non_finite_at(value) {
        return Err(bad(format!("`{path}` is a number that is no finite number (a NaN or an infinity), which the file cannot hold")));
    }
    let json = serde_json::to_value(value).map_err(|e| bad(e.to_string()))?;
    // and the JSON read back as the engine's type, whatever else could keep it from reading back
    serde_json::from_value::<T>(json.clone()).map_err(|e| bad(e.to_string()))?;
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

/// The path of the first number in `value` that is no finite number, as its JSON would spell the
/// path (`cols.0.width`, `color.1`, `conditional_formatting.0.cf_rule.min.Number`): `value`
/// serialised into [`FiniteProbe`], which keeps no output, only where it is.
fn non_finite_at<T: Serialize>(value: &T) -> Option<String> {
    let mut probe = FiniteProbe::default();
    let _ = value.serialize(&mut probe);
    probe.found
}

/// A serde serializer that follows a value's structure to its first non-finite float: a struct's
/// field, a map's key, a list's index and an externally tagged variant's name each a step of the
/// path, as JSON nests them (an internally tagged or untagged enum adds no step of its own).
#[derive(Default)]
struct FiniteProbe {
    path: Vec<String>,
    found: Option<String>,
}

#[derive(Debug)]
struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stopped")
    }
}

impl std::error::Error for Stopped {}

impl serde::ser::Error for Stopped {
    fn custom<M: std::fmt::Display>(_: M) -> Self {
        Stopped
    }
}

impl FiniteProbe {
    fn float(&mut self, finite: bool) -> Result<(), Stopped> {
        if !finite && self.found.is_none() {
            self.found = Some(self.path.join("."));
        }
        if self.found.is_some() { Err(Stopped) } else { Ok(()) }
    }

    fn at<T: Serialize + ?Sized>(&mut self, step: String, value: &T) -> Result<(), Stopped> {
        self.path.push(step);
        let outcome = value.serialize(&mut *self);
        self.path.pop();
        outcome
    }
}

/// A list, a tuple, a map or a struct being followed, with the next index and whether it opened a
/// variant's step.
struct ProbeCompound<'p> {
    probe: &'p mut FiniteProbe,
    index: usize,
    variant: bool,
    key: Option<String>,
}

impl ProbeCompound<'_> {
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        let step = self.index.to_string();
        self.index += 1;
        self.probe.at(step, value)
    }

    fn close(self) -> Result<(), Stopped> {
        if self.variant {
            self.probe.path.pop();
        }
        Ok(())
    }
}

impl<'p> serde::Serializer for &'p mut FiniteProbe {
    type Ok = ();
    type Error = Stopped;
    type SerializeSeq = ProbeCompound<'p>;
    type SerializeTuple = ProbeCompound<'p>;
    type SerializeTupleStruct = ProbeCompound<'p>;
    type SerializeTupleVariant = ProbeCompound<'p>;
    type SerializeMap = ProbeCompound<'p>;
    type SerializeStruct = ProbeCompound<'p>;
    type SerializeStructVariant = ProbeCompound<'p>;

    fn serialize_f64(self, v: f64) -> Result<(), Stopped> {
        self.float(v.is_finite())
    }
    fn serialize_f32(self, v: f32) -> Result<(), Stopped> {
        self.float(v.is_finite())
    }
    fn serialize_bool(self, _: bool) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_i8(self, _: i8) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_i16(self, _: i16) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_i32(self, _: i32) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_i64(self, _: i64) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_u8(self, _: u8) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_u16(self, _: u16) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_u32(self, _: u32) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_u64(self, _: u64) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_char(self, _: char) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_str(self, _: &str) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Stopped> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Stopped> {
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<(), Stopped> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, variant: &'static str, value: &T) -> Result<(), Stopped> {
        self.at(variant.to_string(), value)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<ProbeCompound<'p>, Stopped> {
        Ok(ProbeCompound { probe: self, index: 0, variant: false, key: None })
    }
    fn serialize_tuple(self, _: usize) -> Result<ProbeCompound<'p>, Stopped> {
        Ok(ProbeCompound { probe: self, index: 0, variant: false, key: None })
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<ProbeCompound<'p>, Stopped> {
        Ok(ProbeCompound { probe: self, index: 0, variant: false, key: None })
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, variant: &'static str, _: usize) -> Result<ProbeCompound<'p>, Stopped> {
        self.path.push(variant.to_string());
        Ok(ProbeCompound { probe: self, index: 0, variant: true, key: None })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<ProbeCompound<'p>, Stopped> {
        Ok(ProbeCompound { probe: self, index: 0, variant: false, key: None })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<ProbeCompound<'p>, Stopped> {
        Ok(ProbeCompound { probe: self, index: 0, variant: false, key: None })
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, variant: &'static str, _: usize) -> Result<ProbeCompound<'p>, Stopped> {
        self.path.push(variant.to_string());
        Ok(ProbeCompound { probe: self, index: 0, variant: true, key: None })
    }
}

impl serde::ser::SerializeSeq for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        self.element(value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeTuple for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        self.element(value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeTupleStruct for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        self.element(value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeTupleVariant for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        self.element(value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeMap for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Stopped> {
        // the key as JSON writes it: a text as it is, anything else as its JSON
        self.key = serde_json::to_value(key).ok().map(|k| k.as_str().map(str::to_string).unwrap_or_else(|| k.to_string()));
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stopped> {
        let step = self.key.take().unwrap_or_else(|| self.index.to_string());
        self.index += 1;
        self.probe.at(step, value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeStruct for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Stopped> {
        self.probe.at(key.to_string(), value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
}

impl serde::ser::SerializeStructVariant for ProbeCompound<'_> {
    type Ok = ();
    type Error = Stopped;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Stopped> {
        self.probe.at(key.to_string(), value)
    }
    fn end(self) -> Result<(), Stopped> {
        self.close()
    }
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

/// The engine's structures from TOML, `nan` as their none; an infinity, and a date-time (which no
/// structure of the engine holds), refused by their path.
fn toml_to_json(t: toml::Value, path: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    let at = |k: &str| if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
    Ok(match t {
        toml::Value::Float(f) if f.is_nan() => J::Null,
        toml::Value::Float(f) => J::Number(serde_json::Number::from_f64(f).ok_or_else(|| format!("`{path}` is {f}, no finite number"))?),
        toml::Value::Integer(i) => J::from(i),
        toml::Value::Boolean(b) => J::Bool(b),
        toml::Value::String(s) => J::String(s),
        toml::Value::Datetime(d) => return Err(format!("`{path}` is the date-time {d}, which nothing of the engine holds")),
        toml::Value::Array(a) => {
            J::Array(a.into_iter().enumerate().map(|(i, v)| toml_to_json(v, &at(&i.to_string()))).collect::<Result<_, _>>()?)
        }
        toml::Value::Table(o) => {
            J::Object(o.into_iter().map(|(k, v)| Ok((k.clone(), toml_to_json(v, &at(&k))?))).collect::<Result<_, String>>()?)
        }
    })
}

/// A formula as the file writes it: the engine's user spelling without its leading `=`.
fn bare(formula: &str) -> &str {
    formula.strip_prefix('=').unwrap_or(formula)
}

/// A legacy array's area, its first and last rows and columns, reckoned wide so no size overflows;
/// refused when it holds no cell or runs past the sheet's edge.
fn array_area(anchor: Addr, columns: i64, rows: i64) -> Result<[i64; 4], String> {
    if columns < 1 || rows < 1 {
        return Err(format!("an array formula of {columns} by {rows}, which holds no cell"));
    }
    let (r, c) = (i64::from(anchor.row), i64::from(anchor.column));
    let (r2, c2) = (r + rows - 1, c + columns - 1);
    if r2 > i64::from(crate::a1::LAST_ROW) || c2 > i64::from(crate::a1::LAST_COLUMN) {
        return Err(format!("an array formula of {columns} by {rows}, which runs past the sheet's edge"));
    }
    Ok([r, c, r2, c2])
}

fn holds(area: &[i64; 4], row: i64, column: i64) -> bool {
    (area[0]..=area[2]).contains(&row) && (area[1]..=area[3]).contains(&column)
}

/// The first legacy array, row-major, whose area overlaps an earlier one's, with that one's anchor:
/// the engine enters none such (the writer and the reader refuse it alike).
fn overlapping_arrays(arrays: &[(Addr, [i64; 4])]) -> Option<(Addr, Addr)> {
    let mut sorted = arrays.to_vec();
    sorted.sort_by_key(|(a, _)| *a);
    let mut open: Vec<(Addr, [i64; 4])> = Vec::new();
    for (a, area) in sorted {
        open.retain(|(_, o)| o[2] >= area[0]);
        if let Some((b, _)) = open.iter().find(|(_, o)| o[0] <= area[2] && area[0] <= o[2] && o[1] <= area[3] && area[1] <= o[3]) {
            return Some((a, *b));
        }
        open.push((a, area));
    }
    None
}

/// The anchor of the legacy array, another than `addr`'s own, whose area holds `addr`.
fn array_holding(arrays: &[(Addr, [i64; 4])], addr: Addr) -> Option<Addr> {
    arrays
        .iter()
        .filter(|(_, o)| o[0] != o[2] || o[1] != o[3])
        .find(|(a, o)| *a != addr && holds(o, i64::from(addr.row), i64::from(addr.column)))
        .map(|(a, _)| *a)
}

/// The `[cells]` table of one sheet, one line per cell, row-major. A cell the reader would refuse is
/// refused here by its address, so every file written reads back: a style the workbook does not
/// hold, a legacy array formula over merged cells, of no size, past the sheet's edge or over
/// another's area, a cell with content inside another's area (the engine enters none such on reading).
fn cells_text(book: &Workbook, sheet: u32, part: &str) -> Result<String, ModelError> {
    let styles = book.model().workbook.styles.cell_xfs.len();
    let merged = &book.model().workbook.worksheet(sheet).map_err(|e| ModelError::Engine(e.to_string()))?.merged_cells;
    let mut out = String::from("\n[cells]\n");
    let cells = book.cells(sheet)?;
    let mut arrays = Vec::new();
    for (addr, _) in &cells {
        if let Content::ArrayFormula { columns, rows, .. } = book.content(sheet, *addr)? {
            let area = array_area(*addr, i64::from(columns), i64::from(rows)).map_err(|e| ModelError::Engine(format!("{part} {addr}: {e}")))?;
            arrays.push((*addr, area));
        }
    }
    if let Some((a, b)) = overlapping_arrays(&arrays) {
        return Err(ModelError::Engine(format!("{part} {a}: an array formula over the one at {b}, which the engine enters none of")));
    }
    for (addr, style) in cells {
        let bad = |what: String| ModelError::Engine(format!("{part} {addr}: {what}"));
        if style < 0 {
            return Err(bad(format!("style {style} is no style")));
        }
        if style as usize >= styles {
            return Err(bad(format!("style {style} is none of the workbook's {styles}")));
        }
        let content = book.content(sheet, addr)?;
        if let Content::Formula(f) | Content::ArrayFormula { formula: f, .. } | Content::DynamicFormula(f) = &content
            && bare(f).trim().is_empty()
        {
            return Err(bad("an empty formula, which the engine enters none of".into()));
        }
        // a formula the engine could not read (`SUM(A1:A3` from a malformed `.xlsx`): entered again,
        // the engine repairs it (`SUM(A1:A3)`), so the file would read back another formula
        if matches!(content, Content::Formula(_) | Content::ArrayFormula { .. } | Content::DynamicFormula(_)) {
            let model = book.model();
            let index = model.workbook.worksheet(sheet).ok().and_then(|ws| ws.cell(addr.row, addr.column)).and_then(|c| c.get_formula());
            let parsed = index.and_then(|f| model.parsed_formulas.get(sheet as usize)?.get(usize::try_from(f).ok()?));
            if parsed.is_some_and(|(node, _)| matches!(**node, ironcalc_base::expressions::parser::Node::ParseErrorKind { .. })) {
                return Err(bad("a formula the engine cannot read, which it would read back repaired or as another".into()));
            }
        }
        if !matches!(content, Content::Spill | Content::Empty | Content::Value(Value::Empty))
            && let Some(anchor) = array_holding(&arrays, addr)
        {
            return Err(bad(format!("a cell inside the array formula at {anchor}, which the engine enters none of")));
        }
        if let Content::ArrayFormula { .. } = content
            && let Some((_, [r, c, r2, c2])) = arrays.iter().find(|(a, _)| *a == addr)
        {
            let meets = |m: &ironcalc_base::types::MergedCell| {
                let (mr, mc) = (i64::from(m.row), i64::from(m.column));
                mr <= *r2 && *r < mr + i64::from(m.height) && mc <= *c2 && *c < mc + i64::from(m.width)
            };
            if merged.iter().any(meets) {
                return Err(bad("an array formula over merged cells, which the engine enters none of".into()));
            }
        }
        let styled = |out: &mut String| {
            if style != 0 {
                let _ = write!(out, ", style = {style}");
            }
        };
        // a formula spelled as the reader reads it: in the workbook settings' locale and Ábaco's
        // language, whatever the model's own (fork change 31b)
        let settings = &book.model().workbook.settings;
        let spelled = |held: String| -> Result<String, ModelError> {
            Ok(book
                .model()
                .get_cell_formula_in(sheet, addr.row, addr.column, &settings.locale, LANGUAGE)
                .map_err(|e| bad(e))?
                .unwrap_or(held))
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
                let f = spelled(f)?;
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
                let formula = spelled(formula)?;
                let mut l = format!("{addr} = {{ array = {}, size = [{columns}, {rows}]", quoted(bare(&formula)));
                styled(&mut l);
                l + " }"
            }
            Content::DynamicFormula(f) => {
                let f = spelled(f)?;
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

fn sheet_text(book: &Workbook, sheet: u32, ws: &Worksheet, part_name: &str) -> Result<String, ModelError> {
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
    };
    Ok(engine_text(part_name, &part)? + &cells_text(book, sheet, part_name)?)
}

/// Every part of the file, by name, in the order they are written.
pub fn parts(file: &AbacoFile) -> Result<Vec<(String, Vec<u8>)>, ModelError> {
    let model = file.book.model();
    let ewb = &model.workbook;
    let names: Vec<String> =
        ewb.worksheets.iter().enumerate().map(|(i, ws)| format!("sheets/{}.toml", sheet_stem(i, &ws.name))).collect();
    check_book(ewb, &names).map_err(ModelError::Engine)?;
    // the settings the engine takes, as the reader's own model asks
    Model::new_empty("check", &ewb.settings.locale, &ewb.settings.tz, LANGUAGE)
        .map_err(|e| ModelError::Engine(format!("{WORKBOOK}: `settings`: {e}")))?;
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut entries = Vec::new();
    let mut sheets = Vec::new();
    for (i, ws) in ewb.worksheets.iter().enumerate() {
        let part = names[i].clone();
        sheets.push((part.clone(), sheet_text(&file.book, i as u32, ws, &part)?.into_bytes()));
        entries.push(SheetEntry { name: ws.name.clone(), part });
    }
    // every formula's spelling, read again as the reader reads it (a parser made from the workbook,
    // the cell its context), is the formula the engine holds: one that reads back as another (an
    // engine spelling that loses a bracket, a locale's array rows its lexer does not read) is
    // refused by its address (fork change 31)
    let otherwise = model.formulas_reading_back_otherwise_in(&ewb.settings.locale, LANGUAGE).map_err(|e| ModelError::Engine(format!("{WORKBOOK}: `settings`: {e}")))?;
    if let Some((sheet, row, column)) = otherwise.into_iter().min() {
        let part = names.get(sheet as usize).cloned().unwrap_or_default();
        return Err(ModelError::Engine(format!(
            "{part} {}: a formula whose spelling the reader would read as another formula",
            crate::a1::Addr::new(row, column)
        )));
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
    for (i, r) in ws.references.iter().enumerate() {
        check_reference(r).map_err(|what| ModelError::Engine(format!("{REFERENCES} reference {}: {what}", i + 1)))?;
    }
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
    // a tool's records and state are its own tables: each part TOML's reader reads back (a table
    // nested past its depth would not)
    for (name, bytes) in &out {
        if name.starts_with("workspace/") && name.ends_with(".toml") {
            let text = std::str::from_utf8(bytes).map_err(|e| ModelError::Engine(format!("{name}: {e}")))?;
            toml::from_str::<toml::Table>(text).map_err(|e| ModelError::Engine(format!("{name}: TOML's reader cannot read it back: {}", e.message())))?;
        }
    }
    if !ws.log.is_empty() {
        let text: String = ws.log.iter().map(|l| escape_line(l) + "\n").collect();
        out.push((LOG.into(), text.into_bytes()));
    }
    for (name, bytes) in &file.kept {
        // a part it kept, not reading it, is written back as it came; one a written part would
        // overwrite (a sheet or a macro added since under its name) refuses the write
        if out.iter().any(|(n, _)| n == name) {
            return Err(ModelError::Engine(format!("{name}: a part this file kept without reading it has the name of one it now writes")));
        }
        out.push((name.clone(), bytes.clone()));
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

    /// A TOML part's table, its stamp checked first.
    fn stamped(&mut self, name: &str, format: &str) -> Result<toml::Table, ModelError> {
        let text = self.text(name)?;
        stamp(&text, name, format).map_err(|e| self.bad(format!("{name}: {e}")))?;
        toml::from_str(&text).map_err(|e| self.bad(format!("{name}: {e}")))
    }

    /// The engine's structures from a part's table ([`engine_text`]), `nan` as the engine's none.
    fn engine_table<T: DeserializeOwned + Serialize>(&self, name: &str, table: toml::Table) -> Result<T, ModelError> {
        let json = toml_to_json(toml::Value::Table(table), "").map_err(|e| self.bad(format!("{name}: {e}")))?;
        // the engine's structures take a key they do not know without a word: a key nothing
        // reads, at any depth, is refused by its path, as every file of the suite refuses one
        // (`serde_ignored` marks an optional field's step with `?`, which the file has not: dropped)
        let mut unread = Vec::new();
        let read: Result<T, _> = serde_ignored::deserialize(json.clone(), |path| {
            let path = path.to_string();
            unread.push(path.split('.').filter(|step| *step != "?").collect::<Vec<_>>().join("."));
        });
        // a key nothing reads is named first, a misspelt one replacing the field it meant too (the
        // field then missing: refute round 11)
        if let Some(path) = unread.first() {
            return Err(self.bad(format!("{name}: `{path}` is a key nothing reads")));
        }
        let value = match read {
            Ok(value) => value,
            Err(e) => {
                // a field missing inside an internally tagged object (a link, a format's rule),
                // which serde reads from a buffer `serde_ignored` never sees: a key replacing it is
                // named by its path (refute round 12)
                if let Some(path) = key_replacing_a_field::<T>(&json, &e.to_string()) {
                    return Err(self.bad(format!("{name}: `{path}` is a key nothing reads")));
                }
                return Err(self.bad(format!("{name}: {e}")));
            }
        };
        // inside an internally tagged enum (an object with its `type`: a link, a format's rule)
        // serde reads from a buffer `serde_ignored` never sees; those types skip no field when
        // written, so a key the value written back lacks is one nothing read
        let back = serde_json::to_value(&value).map_err(|e| self.bad(format!("{name}: {e}")))?;
        if let Some(path) = unread_in_tagged(&json, &back, String::new()) {
            return Err(self.bad(format!("{name}: `{path}` is a key nothing reads")));
        }
        Ok(value)
    }
}

/// The path of the first cached cell of a linked workbook (`external_books.N.sheets.M.cells.K`, a
/// `[row, column, value]`) whose row and column an earlier one of its sheet states.
fn twice_stated_external_cell(book: &toml::Table) -> Option<String> {
    let list = |v: Option<&toml::Value>| v.and_then(toml::Value::as_array).cloned().unwrap_or_default();
    for (n, b) in list(book.get("external_books")).iter().enumerate() {
        for (m, sheet) in list(b.get("sheets")).iter().enumerate() {
            let mut seen = std::collections::HashSet::new();
            for (k, cell) in list(sheet.get("cells")).iter().enumerate() {
                let at = cell.as_array().map(|c| (c.first().cloned(), c.get(1).cloned()));
                if let Some((Some(row), Some(column))) = at
                    && !seen.insert((row.to_string(), column.to_string()))
                {
                    return Some(format!("external_books.{n}.sheets.{m}.cells.{k}"));
                }
            }
        }
    }
    None
}

/// The key that replaced a field the deserialiser found missing, found by reading again rather
/// than guessed (refute rounds 12 to 14): objects lacking the field are given it, as each kind of
/// JSON value and, for a tag, each variant the deserialiser names. An object holds it once a kind
/// is taken there, the error then moving on or, a kind refused there first, staying (the same field
/// missing in another object); an object where every kind leaves the error as it was does not read
/// the field and is left as it came. Each new missing field is filled in its turn, 64 rounds and
/// 1 024 readings at most, until the whole reads; the keys the value written back then lacks, as
/// `serde_ignored` and [`unread_in_tagged`] read them, the fields put in left out, are the ones
/// nothing reads. Of the kinds that make the whole read, the one leaving fewest such keys is taken,
/// and its first key is named. A field deleted,
/// no key standing for it, names none, and the deserialiser's own words stand. The objects are tried tagged ones and what they hold first,
/// then those holding a variant-like text, each by its key nearest to the field's name, 64 a round
/// at most.
fn key_replacing_a_field<T: DeserializeOwned + Serialize>(json: &serde_json::Value, error: &str) -> Option<String> {
    use serde_json::Value as J;
    let missing = |e: &str| e.split_once("missing field `").and_then(|(_, r)| r.split_once('`')).map(|(f, _)| f.to_string());
    // the deserialiser refusing the value put in (or, in a structure that takes no key it does not
    // know, the key itself)
    let rejected = |e: &str| ["invalid type", "unknown variant", "invalid value", "invalid length", "unknown field"].iter().any(|r| e.starts_with(r));
    let dotted = |path: &[String], field: &str| path.iter().map(String::as_str).chain([field]).collect::<Vec<_>>().join(".");
    let mut filled = json.clone();
    let mut text = error.to_string();
    // the fields this search put in, which are no key of the file's
    let mut inserted: Vec<(Vec<String>, String)> = Vec::new();
    // readings tried, all rounds together: a refused file of a large sheet stays a quick refusal
    let mut budget = 1024usize;
    for _ in 0..64 {
        let field = missing(&text)?;
        let mut moved = None;
        for path in candidates_lacking(&filled, &field).into_iter().take(64) {
            let mut kinds = vec![J::Bool(false), J::from(0), J::String(String::new()), J::Null, J::Array(Vec::new()), J::Object(Default::default())];
            // a kind refused shows the field is read there; a kind then taken with the error
            // unchanged fills it, the same field missing in another object too (two links)
            let mut refused = false;
            let mut taken_unchanged = None;
            let mut best = None;
            let mut k = 0;
            while k < kinds.len() {
                budget = budget.checked_sub(1)?;
                let mut trial = filled.clone();
                if !insert_at(&mut trial, &path, &field, kinds[k].clone()) {
                    break;
                }
                match serde_json::from_value::<T>(trial.clone()) {
                    Ok(value) => {
                        let back = serde_json::to_value(&value).ok()?;
                        let mut ours = inserted.clone();
                        ours.push((path.clone(), field.clone()));
                        let dotted_ours: Vec<String> = ours.iter().map(|(p, f)| dotted(p, f)).collect();
                        // the file as it came, our fields out, against the value written back
                        let mut file = trial.clone();
                        for (p, f) in &ours {
                            remove_at(&mut file, p, f);
                        }
                        let mut unread = Vec::new();
                        let _: Result<T, _> = serde_ignored::deserialize(trial, |p| {
                            unread.push(p.to_string().split('.').filter(|step| *step != "?").collect::<Vec<_>>().join("."));
                        });
                        unread.retain(|p| !dotted_ours.contains(p));
                        unread_below(&file, &back, String::new(), false, &mut unread);
                        // the reading that leaves fewest keys unread: none, a field deleted (no
                        // key names it); one, the key that replaced it
                        if unread.len() <= 1 {
                            return unread.into_iter().next();
                        }
                        if best.as_ref().is_none_or(|b: &Vec<String>| unread.len() < b.len()) {
                            best = Some(unread);
                        }
                        k += 1;
                        continue;
                    }
                    Err(e) => {
                        let error = e.to_string();
                        if !rejected(&error) && error != text {
                            moved = Some((trial, error, path.clone()));
                            break;
                        }
                        if !rejected(&error) {
                            taken_unchanged.get_or_insert(trial);
                        } else {
                            refused = true;
                        }
                        // a tag: each variant the deserialiser names (`expected `A` or `B``), one
                        // the object holds as a text first (a tag renamed keeps its variant)
                        if error.starts_with("unknown variant")
                            && let Some((_, expected)) = error.split_once("expected")
                        {
                            let held = |v: &J| object_at(&filled, &path).is_some_and(|o| o.values().any(|x| x == v));
                            let named: Vec<J> = expected.split('`').skip(1).step_by(2).map(|v| J::String(v.to_string())).filter(|v| !kinds.contains(v)).collect();
                            let (first, rest): (Vec<J>, Vec<J>) = named.into_iter().partition(held);
                            kinds.splice(k + 1..k + 1, first);
                            kinds.extend(rest);
                        }
                    }
                }
                k += 1;
            }
            if let Some(unread) = best {
                return unread.into_iter().next();
            }
            if moved.is_some() {
                break;
            }
            // filled here, the round going on to the next object lacking it
            if refused && let Some(trial) = taken_unchanged {
                filled = trial;
                inserted.push((path, field.clone()));
            }
        }
        let (next, error, path) = moved?;
        filled = next;
        text = error;
        inserted.push((path, field));
    }
    None
}

/// The object at `path` in `json`, if one stands there.
fn object_at<'a>(json: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value as J;
    let mut at = json;
    for step in path {
        at = match at {
            J::Object(o) => o.get(step)?,
            J::Array(a) => a.get(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    at.as_object()
}

/// `field` taken out of the object at `path` in `json`, if it stands there.
fn remove_at(json: &mut serde_json::Value, path: &[String], field: &str) {
    use serde_json::Value as J;
    let mut at = json;
    for step in path {
        at = match at {
            J::Object(o) => match o.get_mut(step) {
                Some(v) => v,
                None => return,
            },
            J::Array(a) => match step.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                Some(v) => v,
                None => return,
            },
            _ => return,
        };
    }
    if let J::Object(o) = at {
        o.remove(field);
    }
}

/// The paths of the objects of `json` lacking `field`, the likeliest first: tagged ones (a `type`
/// text) and the objects below them, then those holding a text that reads as a variant (`CellIs`,
/// `External`), then the rest; among each, the one holding the key nearest to `field` first.
fn candidates_lacking(json: &serde_json::Value, field: &str) -> Vec<Vec<String>> {
    use serde_json::Value as J;
    fn walk(v: &J, field: &str, path: &mut Vec<String>, below_tagged: bool, out: &mut Vec<(usize, usize, Vec<String>)>) {
        match v {
            J::Object(o) => {
                let tagged = o.get("type").is_some_and(J::is_string);
                if !o.contains_key(field) {
                    let variant_like = o.values().any(|x| x.as_str().is_some_and(|t| t.starts_with(|c: char| c.is_ascii_uppercase())));
                    let rank = if tagged || below_tagged { 0 } else if variant_like { 1 } else { 2 };
                    let nearest = o.keys().map(|k| edit_distance(k, field)).min().unwrap_or(usize::MAX);
                    out.push((rank, nearest, path.clone()));
                }
                for (k, child) in o {
                    path.push(k.clone());
                    walk(child, field, path, below_tagged || tagged, out);
                    path.pop();
                }
            }
            J::Array(a) => {
                for (i, child) in a.iter().enumerate() {
                    path.push(i.to_string());
                    walk(child, field, path, below_tagged, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(json, field, &mut Vec::new(), false, &mut out);
    out.sort_by_key(|(rank, nearest, _)| (*rank, *nearest));
    out.into_iter().map(|(_, _, path)| path).collect()
}

/// `value` set as `field` of the object at `path` in `json`; false when no object stands there.
fn insert_at(json: &mut serde_json::Value, path: &[String], field: &str, value: serde_json::Value) -> bool {
    use serde_json::Value as J;
    let mut at = json;
    for step in path {
        at = match at {
            J::Object(o) => match o.get_mut(step) {
                Some(v) => v,
                None => return false,
            },
            J::Array(a) => match step.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                Some(v) => v,
                None => return false,
            },
            _ => return false,
        };
    }
    match at {
        J::Object(o) => {
            o.insert(field.to_string(), value);
            true
        }
        _ => false,
    }
}

/// The optimal string alignment distance between two names (letters added, dropped, changed or two
/// swapped).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The first key, in an object holding a `type` (an internally tagged enum, in the file or in the
/// value written back) or in any object below one (all of it read from serde's buffer: a threshold
/// of a format's rule), that `back`, the value read written out again, lacks, by its path.
fn unread_in_tagged(read: &serde_json::Value, back: &serde_json::Value, path: String) -> Option<String> {
    let mut unread = Vec::new();
    unread_below(read, back, path, false, &mut unread);
    unread.into_iter().next()
}

/// Every key [`unread_in_tagged`] names, in the order of the file, pushed to `out`.
fn unread_below(read: &serde_json::Value, back: &serde_json::Value, path: String, inside: bool, out: &mut Vec<String>) {
    use serde_json::Value as J;
    let at = |k: &str| if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
    match (read, back) {
        (J::Object(r), J::Object(b)) => {
            // the tag read on either side: the search for a replacing key takes its own `type` out
            let inside = inside || r.get("type").is_some_and(J::is_string) || b.get("type").is_some_and(J::is_string);
            // this object's keys first, then those below them
            if inside {
                out.extend(r.keys().filter(|k| !b.contains_key(*k)).map(|k| at(k)));
            }
            for (k, v) in r {
                if let Some(bv) = b.get(k) {
                    unread_below(v, bv, at(k), inside, out);
                }
            }
        }
        (J::Array(r), J::Array(b)) => {
            for (i, (v, bv)) in r.iter().zip(b).enumerate() {
                unread_below(v, bv, at(&i.to_string()), inside, out);
            }
        }
        _ => {}
    }
}

/// What a reference must be for the file to hold it: its `sha256` 64 lowercase hex digits, its
/// `modified` RFC 3339 in UTC to the second. The writer and the reader both hold one to it.
fn check_reference(r: &Reference) -> Result<(), &'static str> {
    if r.sha256.as_deref().is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) {
        return Err("`sha256` is 64 lowercase hex digits");
    }
    if r.modified.as_deref().is_some_and(|m| !crate::workspace::is_rfc3339_utc(m)) {
        return Err("`modified` is RFC 3339 in UTC to the second (`2026-11-23T10:04:05Z`)");
    }
    Ok(())
}

/// What the engine's workbook must be for the file to hold it and the engine to work on it, by part
/// and place: sheet names as the engine takes them (1 to 31 characters, none of `\ / * ? : [ ]`),
/// told apart in any case, each sheet's id its own; a row's and a column's style one of the
/// workbook's; a defined name that does not read as a cell, stated once in its scope (a name local
/// to a sheet the workbook does not hold, one of a chart sheet of the workbook it came from, kept).
/// The writer and the reader both hold a workbook to it.
fn check_book(ewb: &EngineWorkbook, parts: &[String]) -> Result<(), String> {
    let styles = ewb.styles.cell_xfs.len();
    let style_of = |s: i32| -> Option<String> {
        if s < 0 {
            Some(format!("{s} is no style"))
        } else if s as usize >= styles {
            Some(format!("{s} is none of the workbook's {styles} styles"))
        } else {
            None
        }
    };
    if ewb.worksheets.is_empty() {
        return Err(format!("{MANIFEST}: a workbook holds a sheet at least"));
    }
    for (i, ws) in ewb.worksheets.iter().enumerate() {
        let part = parts.get(i).map(String::as_str).unwrap_or(MANIFEST);
        let name = &ws.name;
        let invalid = ['\\', '/', '*', '?', ':', '[', ']'];
        if name.is_empty() || name.chars().count() > 31 || name.contains(&invalid[..]) {
            return Err(format!("{MANIFEST}: sheet {} `{name}` is no sheet name (1 to 31 characters, none of \\ / * ? : [ ])", i + 1));
        }
        if let Some(j) = ewb.worksheets[..i].iter().position(|w| w.name.to_uppercase() == name.to_uppercase()) {
            return Err(format!("{MANIFEST}: sheet {} is named `{name}` as sheet {} is", i + 1, j + 1));
        }
        if let Some(j) = ewb.worksheets[..i].iter().position(|w| w.sheet_id == ws.sheet_id) {
            return Err(format!("{part}: `sheet_id` {} is sheet {}'s too", ws.sheet_id, j + 1));
        }
        for (k, c) in ws.cols.iter().enumerate() {
            if let Some(why) = c.style.and_then(style_of) {
                return Err(format!("{part}: `cols.{k}.style` {why}"));
            }
        }
        for (k, r) in ws.rows.iter().enumerate() {
            if let Some(why) = style_of(r.s) {
                return Err(format!("{part}: `rows.{k}.s` {why}"));
            }
        }
    }
    // a style's font, fill and border are each one the workbook holds (the engine indexes them
    // unchecked when a formula reads the cell, or enters one in it), and a named style's too, which
    // applying it copies into a cell's style
    let records = ewb.styles.cell_xfs.iter().map(|x| ("cell_xfs", x.num_fmt_id, x.font_id, x.fill_id, x.border_id));
    let named = ewb.styles.cell_style_xfs.iter().map(|x| ("cell_style_xfs", x.num_fmt_id, x.font_id, x.fill_id, x.border_id));
    let mut at = std::collections::HashMap::new();
    for (table, num_fmt_id, font_id, fill_id, border_id) in records.chain(named) {
        let i = *at.entry(table).and_modify(|i| *i += 1).or_insert(0usize);
        for (what, id, len) in [
            ("font_id", font_id, ewb.styles.fonts.len()),
            ("fill_id", fill_id, ewb.styles.fills.len()),
            ("border_id", border_id, ewb.styles.borders.len()),
        ] {
            if usize::try_from(id).map_or(true, |id| id >= len) {
                return Err(format!("{WORKBOOK}: `styles.{table}.{i}.{what}` {id} is none of the workbook's {len}"));
            }
        }
        // and its number format an id the engine reads: below 50 it indexes Excel's built-in
        // formats unchecked, a negative one past them (refute round 14)
        if num_fmt_id < 0 {
            return Err(format!("{WORKBOOK}: `styles.{table}.{i}.num_fmt_id` {num_fmt_id} is no number format's"));
        }
    }
    for (i, d) in ewb.defined_names.iter().enumerate() {
        if d.name.is_empty() || crate::book::name_reads_as_a_cell(&d.name) {
            return Err(format!("{WORKBOOK}: `defined_names.{i}` `{}` is no name (it reads as a cell)", d.name));
        }
        if !crate::book::name_is_spelled_as_a_name(&d.name) {
            return Err(format!(
                "{WORKBOOK}: `defined_names.{i}` `{}` is no name (a letter, `_`, `\\` or a character beyond ASCII first, then those, digits, `.` and `?`)",
                d.name
            ));
        }
        if let Some(j) =
            ewb.defined_names[..i].iter().position(|e| {
                e.sheet_id == d.sheet_id
                    && ironcalc_base::expressions::utils::name_key(&e.name) == ironcalc_base::expressions::utils::name_key(&d.name)
            })
        {
            return Err(format!("{WORKBOOK}: `defined_names.{i}` `{}` is stated in its scope already, as `defined_names.{j}`", d.name));
        }
    }
    // a table's cells a range the engine reads (its parser reads a structured reference by them,
    // and panics on one it cannot), its corners in order, its header and totals rows Excel's (none
    // or one each) and fewer together than its rows (the parser's sums of them overflowed, and past
    // the rows they read another range: refute round 11), and its sheet one of the workbook's, by
    // name as the parser looks it up, in any case (refute rounds 10 and 11)
    for (key, table) in &ewb.tables {
        let Some((top, left, bottom, right)) = table_range(&table.reference) else {
            return Err(format!("{WORKBOOK}: `tables.{key}.reference` `{}` is no range of cells, which the engine reads a table by", table.reference));
        };
        if top > bottom || left > right {
            return Err(format!("{WORKBOOK}: `tables.{key}.reference` `{}` has its corners out of order", table.reference));
        }
        for (field, count) in [("header_row_count", table.header_row_count), ("totals_row_count", table.totals_row_count)] {
            if count > 1 {
                return Err(format!("{WORKBOOK}: `tables.{key}.{field}` {count} is none of Excel's (0 or 1)"));
            }
        }
        let rows = i64::from(bottom) - i64::from(top) + 1;
        if i64::from(table.header_row_count) + i64::from(table.totals_row_count) >= rows {
            return Err(format!(
                "{WORKBOOK}: `tables.{key}` holds no data row: {} header and {} totals rows of the {rows} of `{}`",
                table.header_row_count, table.totals_row_count, table.reference
            ));
        }
        // its sheet in the sheet's own spelling: the parser finds one in any case, but the engine's
        // rename, its deletion and SUBTOTAL's filter compare it exactly (refute round 12)
        if !ewb.worksheets.iter().any(|w| w.name == table.sheet_name) {
            return Err(format!("{WORKBOOK}: `tables.{key}.sheet_name` `{}` is none of the workbook's sheets, as the sheet spells its name", table.sheet_name));
        }
        // one column for each of its range's columns: the parser places a column at the range's
        // start and its place, past the range (or the sheet's edge) otherwise (refute round 12)
        let width = i64::from(right) - i64::from(left) + 1;
        if table.columns.len() as i64 != width {
            return Err(format!(
                "{WORKBOOK}: `tables.{key}.columns` holds {} columns, where its range `{}` is {width} wide",
                table.columns.len(),
                table.reference
            ));
        }
    }
    // a linked workbook states each of its sheets once, as the engine tells sheet names apart
    // (`Data` and `DATA` are one): its map keeps one sheet's cells, the other's dropped without a
    // word (refute round 13)
    for (n, book) in ewb.external_books.iter().enumerate() {
        for (m, sheet) in book.sheets.iter().enumerate() {
            if let Some(j) = book.sheets[..m].iter().position(|s| ironcalc_base::external::same_sheet_name(&s.name, &sheet.name)) {
                return Err(format!("{WORKBOOK}: `external_books.{n}.sheets.{m}` names the sheet `external_books.{n}.sheets.{j}` names"));
            }
        }
    }
    Ok(())
}

/// A table's range as the engine reads one (`A1:E6`, `$A$1:$E$6`), its top row, left column,
/// bottom row and right column: its lexer's first token a range, as `Model::set_table_reference`
/// and the parser take it; `None` for anything else.
fn table_range(reference: &str) -> Option<(i32, i32, i32, i32)> {
    use ironcalc_base::expressions::lexer::{Lexer, LexerMode};
    use ironcalc_base::expressions::token::TokenType;
    let locale = ironcalc_base::locale::get_locale("en").ok()?;
    let language = ironcalc_base::language::get_language("en").ok()?;
    match Lexer::new(reference, LexerMode::A1, locale, language).next_token() {
        TokenType::Range { left, right, .. } => Some((left.row, left.column, right.row, right.column)),
        _ => None,
    }
}

/// The suite's one header reader (C-045): core's own source, compiled here by path, so the stamp
/// is read by the very code every other file of the suite is read by, its order and its texts.
/// Core's layers stand behind its `net` feature now, but a workspace build unifies a package's
/// features across members, so `veredas-core` without `net` would still bring the plant stack into
/// Ábaco's test builds, which C-220 refuses (measured 2026-11-23): compiling it by path is the
/// ruled way (docs/questions.md Q345, 2026-11-25).
#[allow(dead_code)]
#[path = "../../veredas-core/src/schema.rs"]
mod schema;

/// The stamp of a part: `format` as `format` says and the `version` this Ábaco reads, through
/// core's reader, the part named by its path.
fn stamp(text: &str, part: &str, format: &str) -> Result<(), String> {
    schema::check(text, part, format, VERSION).map_err(|e| e.to_string())
}

/// The engine's error a cell's `e` names: in the English name its reader takes (`#N/IMPL!`) or in
/// the spelling it shows a value in (`#N/IMPL`), which is how the writer writes every error.
fn error_named(code: &str) -> Option<ironcalc_base::expressions::token::Error> {
    use ironcalc_base::expressions::token::Error as E;
    get_error_by_english_name(code).or_else(|| {
        [E::REF, E::NAME, E::VALUE, E::DIV, E::NA, E::NUM, E::ERROR, E::NIMPL, E::SPILL, E::CALC, E::CIRC, E::NULL]
            .into_iter()
            .find(|e| e.to_string() == code)
    })
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
    // a formula's last value: one number, text or boolean, or its error as `e`, never both
    match &e.v {
        Some(_) if e.e.is_some() => return Err("both `v` and `e`: a formula's last value is one".into()),
        Some(toml::Value::Float(f)) if !f.is_finite() => return Err(format!("`v` is {f}, no finite number")),
        Some(toml::Value::Float(_) | toml::Value::Integer(_) | toml::Value::String(_) | toml::Value::Boolean(_)) | None => {}
        Some(other) => return Err(format!("`v` is a {}, where a formula's last value is a number, a text or a boolean", other.type_str())),
    }
    if e.size.is_some() != e.array.is_some() {
        return Err("`size` goes with `array`, and only with it".into());
    }
    if e.style < 0 {
        return Err(format!("style {} is no style", e.style));
    }
    let error = |code: &str| error_named(code).ok_or_else(|| format!("{code:?} is no error"));
    // a formula of white space alone, which the engine enters none of (the writer refuses it too)
    if [&e.f, &e.array, &e.dynamic].iter().any(|f| f.as_ref().is_some_and(|f| f.trim().is_empty())) {
        return Err("an empty formula, which the engine enters none of".into());
    }
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
    let book_table = parts.stamped(WORKBOOK, WORKBOOK_FORMAT)?;
    // a cached cell of a linked workbook stated twice: the engine's map would keep one without a word
    if let Some(path) = twice_stated_external_cell(&book_table) {
        return Err(parts.bad(format!("{WORKBOOK}: `{path}` states a cell its sheet states already")));
    }
    let book: WorkbookPart = parts.engine_table(WORKBOOK, book_table)?;

    // The engine's workbook, values and styles placed as they are (text never parsed again, styles
    // keeping their indices); formulas entered once the model exists.
    let mut shared: Vec<String> = Vec::new();
    let mut shared_index: HashMap<String, i32> = HashMap::new();
    let mut worksheets = Vec::new();
    let mut formulas: Vec<(u32, String, Addr, Entry, i32)> = Vec::new();
    // the cells that carry only a style (a styled cell an array spills into among them): entering an
    // array may give its spilled cells a derived style, so theirs is set again after
    let mut style_only: Vec<(u32, i32, i32, i32)> = Vec::new();
    // every cell the file states, so a spilled cell it does not state is known after the arrays spill
    let mut stated: HashSet<(u32, i32, i32)> = HashSet::new();
    if manifest.sheets.is_empty() {
        return Err(parts.bad(format!("{MANIFEST}: a workbook holds a sheet at least")));
    }
    // the manifest lists each part once (refute round 14: a part listed again was taken from the
    // file by the first listing and missed by the second)
    for (i, sheet) in manifest.sheets.iter().enumerate() {
        if let Some(j) = manifest.sheets[..i].iter().position(|s| s.part == sheet.part) {
            return Err(parts.bad(format!("{MANIFEST}: `sheets.{i}` lists the part `{}`, which `sheets.{j}` lists", sheet.part)));
        }
    }
    for (i, name) in manifest.macros.iter().enumerate() {
        if let Some(j) = manifest.macros[..i].iter().position(|m| m == name) {
            return Err(parts.bad(format!("{MANIFEST}: `macros.{i}` lists the macro `{name}`, which `macros.{j}` lists")));
        }
    }
    let styles = book.styles.cell_xfs.len();
    for (i, sheet) in manifest.sheets.iter().enumerate() {
        let mut table = parts.stamped(&sheet.part, SHEET_FORMAT)?;
        // the cells read apart, each with TOML's own types (a `nan` is a NaN), refused by address
        let cells = match table.remove("cells") {
            None => toml::Table::new(),
            Some(toml::Value::Table(t)) => t,
            Some(_) => return Err(parts.bad(format!("{}: `cells` is no table", sheet.part))),
        };
        let part: SheetPart = parts.engine_table(&sheet.part, table)?;
        // a cell's link stated twice: the sheet's map of links would keep one without a word
        let mut linked: HashMap<(i32, i32), usize> = HashMap::new();
        for (i, (r, c, _)) in part.links.iter().enumerate() {
            if let Some(j) = linked.insert((*r, *c), i) {
                return Err(parts.bad(format!("{}: `links.{i}` states the link of a cell `links.{j}` states already", sheet.part)));
            }
        }
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
        // the legacy arrays' areas, so a cell stated inside one is refused whatever order the keys
        // come in (the array's own anchor and the style-only cells it spills into aside)
        // (an area of no cell or past the sheet's edge refused by its anchor before any sum is taken,
        // and one over another's area)
        let mut arrays: Vec<(Addr, [i64; 4])> = Vec::new();
        for (k, v) in &cells {
            let size = v.as_table().filter(|t| t.contains_key("array")).and_then(|t| t.get("size")).and_then(toml::Value::as_array);
            let Some(size) = size else { continue };
            let (Some(w), Some(h), Ok(a)) = (size.first().and_then(toml::Value::as_integer), size.get(1).and_then(toml::Value::as_integer), Addr::parse(k)) else {
                continue;
            };
            arrays.push((a, array_area(a, w, h).map_err(|e| parts.bad(format!("{} {k}: {e}", sheet.part)))?));
        }
        if let Some((a, b)) = overlapping_arrays(&arrays) {
            return Err(parts.bad(format!("{} {a}: an array formula over the one at {b}, which the engine enters none of", sheet.part)));
        }
        for (key, cell) in cells {
            let bad = |e: String| parts.bad(format!("{} {key}: {e}", sheet.part));
            let addr = Addr::parse(&key).map_err(|e| bad(e.to_string()))?;
            // the address as Ábaco writes it (`B1`, never `b1` or `$B$1`), so no cell is stated
            // twice under two spellings
            if addr.to_string() != key {
                return Err(bad(format!("an address is written `{addr}`")));
            }
            // a date-time is no value a cell holds (serde would read it as its text)
            if let Some((k, d)) = cell.as_table().and_then(|t| t.iter().find_map(|(k, v)| v.as_datetime().map(|d| (k, d)))) {
                return Err(bad(format!("`{k}` is the date-time {d}, which no cell holds")));
            }
            // an integer is read as a double: one no double holds exactly is refused, not rounded
            for key in ["n", "v"] {
                if let Some(toml::Value::Integer(i)) = cell.get(key)
                    && (*i as f64) as i128 != i128::from(*i)
                {
                    return Err(bad(format!("`{key} = {i}` is an integer no number of the file holds exactly")));
                }
            }
            let cell: CellEntry = cell.try_into().map_err(|e: toml::de::Error| bad(e.message().to_string()))?;
            let s = cell.style;
            if usize::try_from(s).is_ok_and(|s| s >= styles) {
                return Err(bad(format!("style {s} is none of the workbook's {styles}")));
            }
            let (row, column) = (addr.row, addr.column);
            let entry = entry(cell).map_err(bad)?;
            if !matches!(entry, Entry::Style)
                && let Some(anchor) = array_holding(&arrays, addr)
            {
                return Err(bad(format!("a cell stated inside the array formula at {anchor}")));
            }
            if matches!(entry, Entry::Style) {
                style_only.push((i as u32, row, column, s));
            }
            stated.insert((i as u32, row, column));
            match entry {
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
    let sheet_parts: Vec<String> = manifest.sheets.iter().map(|s| s.part.clone()).collect();
    check_book(&engine, &sheet_parts).map_err(|e| parts.bad(e))?;
    // the engine refuses a workbook only for its settings (its locale, its time zone)
    let mut model = Model::from_workbook(engine, LANGUAGE).map_err(|e| parts.bad(format!("{WORKBOOK}: `settings`: {e}")))?;
    // Entering a formula derives styles the file does not hold (a quote prefix's plain copy, a
    // number format from the units of what an array reads: `$#,##0.00`, a date's): every cell gets
    // the style the file states below, and the styles are put back whole, as the file holds them.
    let file_styles = model.workbook.styles.clone();
    // A formula the engine cannot read as the file states it (`1+`, `=A1`, `SUM(A1:A3`) is refused
    // by its address, as the writer refuses one: entered, the engine would keep the error or repair
    // the text (`SUM(A1:A3)`, a `)` added), and the file would hold another formula than it states.
    // Read by a parser of the file's own locale and English, the reader's, with no repair.
    let locale = ironcalc_base::locale::get_locale(&model.workbook.settings.locale)
        .map_err(|e| parts.bad(format!("{WORKBOOK}: `settings`: {e}")))?;
    let language = ironcalc_base::language::get_language(LANGUAGE).map_err(|e| parts.bad(format!("{WORKBOOK}: {e}")))?;
    let mut parser = ironcalc_base::expressions::parser::Parser::new(
        model.workbook.get_worksheet_names(),
        model.workbook.get_defined_names_with_scope(),
        model.workbook.tables.clone(),
        locale,
        language,
    );
    for (sheet, key, addr, formula, style) in formulas {
        let (row, column) = (addr.row, addr.column);
        let bad = |e: String| ModelError::file(path, format!("{} {key}: {e}", manifest.sheets[sheet as usize].part));
        if let Entry::Array(f, ..) | Entry::Formula(f) | Entry::Dynamic(f) = &formula {
            let sheet_name = model.workbook.worksheet(sheet).map_err(bad)?.get_name();
            let context = ironcalc_base::expressions::types::CellReferenceRC { sheet: sheet_name, row, column };
            if matches!(parser.parse(f, &context), ironcalc_base::expressions::parser::Node::ParseErrorKind { .. }) {
                return Err(bad("a formula the engine cannot read".into()));
            }
        }
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
        for (i, r) in workspace.references.iter().enumerate() {
            check_reference(r).map_err(|what| parts.bad(format!("{REFERENCES} reference {}: {what}", i + 1)))?;
        }
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
    // The styles the file states, set again once every array has spilled (a legacy one on entering,
    // a dynamic one on evaluating; either gives a cell it creates its row's or column's style, a
    // legacy one a quote prefix's plain copy): a spilled cell the file states keeps its style, one it
    // does not state holds style 0. The styles entering derived are orphans then: the styles are
    // the file's again, every list of them (formats, fonts, fills, borders) as it was read.
    let mut spilled_unstated = Vec::new();
    for sheet in 0..manifest.sheets.len() as u32 {
        for (addr, style) in book.cells(sheet)? {
            if style != 0 && !stated.contains(&(sheet, addr.row, addr.column)) && book.content(sheet, addr)? == Content::Spill {
                spilled_unstated.push((sheet, addr.row, addr.column, 0));
            }
        }
    }
    let model = book.model_mut();
    for (sheet, row, column, style) in style_only.into_iter().chain(spilled_unstated) {
        let ws = model.workbook.worksheet_mut(sheet).map_err(|e| ModelError::file(path, e))?;
        ws.set_cell_style(row, column, style).map_err(|e| ModelError::file(path, e))?;
    }
    model.workbook.styles = file_styles;
    Ok(AbacoFile { book, macros, workspace, kept: parts.all })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_round_trip_and_refuse_a_bad_escape() {
        for s in ["", "plain", "a\tb", "line1\nline2\r\n", "back\\slash", "\\t literal", "a\u{2028}b\u{85}c\u{b}d\u{c}e\u{1c}f\u{2029}", "\\u0041 literal"] {
            assert_eq!(unescape_line(&escape_line(s)).unwrap(), s);
            assert!(!escape_line(s).contains(['\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{85}', '\u{2028}', '\u{2029}']), "{s:?}");
        }
        assert_eq!(escape_line("a\u{2028}b"), "a\\u2028b");
        assert_eq!(unescape_line("bad \\u12").unwrap_err(), "`\\u12` is no character");
        // four hex digits and nothing else: no sign, no space
        for bad in ["\\u+041", "\\u-041", "\\u 041", "\\u004G"] {
            assert!(unescape_line(bad).is_err(), "{bad}");
        }
        assert_eq!(unescape_line("\\u004f\\u004F").unwrap(), "OO");
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
    fn a_string_is_written_on_one_line_and_reads_back_exactly() {
        let every: String = (0u8..=0x7f).map(char::from).chain("ação \u{2028} 😀".chars()).collect();
        for s in [every.as_str(), "", "\"", "\\", "a\r\nb", "'''", "\"\"\""] {
            let q = quoted(s);
            assert_eq!(q.lines().count(), 1, "{q}");
            let back: toml::Table = toml::from_str(&format!("x = {q}")).unwrap();
            assert_eq!(back["x"].as_str(), Some(s), "{q}");
        }
        assert_eq!(quoted("a\nb\t\"c\"\\\u{1}\u{7f}"), r#""a\nb\t\"c\"\\\u0001\u007F""#);
        // the C1 controls and Unicode's other line breaks escaped: no reader splits the line
        for c in ('\u{80}'..='\u{9f}').chain(['\u{2028}', '\u{2029}']) {
            let q = quoted(&format!("a{c}b"));
            assert_eq!(q, format!("\"a\\u{:04X}b\"", c as u32));
            let back: toml::Table = toml::from_str(&format!("x = {q}")).unwrap();
            assert_eq!(back["x"].as_str(), Some(format!("a{c}b").as_str()));
        }
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
