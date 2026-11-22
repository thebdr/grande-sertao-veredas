//! One TOML file as the document behind its two views (docs/oficina-plan.md, "Every TOML file
//! opens as a form and as text"; the spike's point 2).
//!
//! The document is the one source of both views. The text view replaces the whole text
//! ([`Document::set_text`]); the form sets one scalar at a key path ([`Document::set`]). Either
//! way the document holds a text that parses, and a text that does not is refused with where it
//! fails, the document left as it was. A form edit changes the characters of that one value and
//! nothing else: every comment, blank line, space, quoting style and line ending around it comes
//! back as it was, because the edit goes through `toml_edit`, which keeps them, with the old
//! value's decoration carried onto the new one.
//!
//! What `toml_edit` does not keep is handled here (measured in the spike, `.zen/evidence/P-015.md`):
//! it writes every line ending as LF and drops a byte-order mark. The document works on LF text
//! with no mark and puts both back when it writes the file's bytes ([`Document::to_bytes`]).
//!
//! The form here is built from the document itself (each scalar a field of its own type), not
//! from a schema: where a form's schema comes from is open (`docs/questions.md` Q31, the review's
//! entry 30). // DECISION PENDING (Q31 entry 30): a form generated from core's types would sit on
//! top of these fields; nothing below changes.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, Value};
use veredas_core::schema::{self, HeaderError, Stamp};

/// The parsed text, with the spans that say where each value sits.
type Parsed = toml_edit::Document<String>;

/// How the file's lines ended when it was read, and so how they are written back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEndings {
    /// Every line ends in LF (a file with one line and no ending reads as LF).
    Lf,
    /// Every line ends in CR LF.
    Crlf,
    /// Both, counted. Written back with the one more lines used, LF on a tie: a mixed file
    /// cannot come back byte for byte once a line of it is edited.
    Mixed { lf: usize, crlf: usize },
}

impl LineEndings {
    fn of(text: &str) -> LineEndings {
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count() - crlf;
        match (lf, crlf) {
            (_, 0) => LineEndings::Lf,
            (0, _) => LineEndings::Crlf,
            (lf, crlf) => LineEndings::Mixed { lf, crlf },
        }
    }

    /// The ending every line is written with.
    pub fn written(self) -> &'static str {
        match self {
            LineEndings::Lf => "\n",
            LineEndings::Crlf => "\r\n",
            LineEndings::Mixed { lf, crlf } if crlf > lf => "\r\n",
            LineEndings::Mixed { .. } => "\n",
        }
    }
}

/// The type of a value, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    String,
    Integer,
    Float,
    Boolean,
    Datetime,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::String => "a string",
            Kind::Integer => "an integer",
            Kind::Float => "a float",
            Kind::Boolean => "a boolean",
            Kind::Datetime => "a date-time",
        })
    }
}

/// One value, decoded: what the form shows in a field and sends back. A date-time travels as its
/// TOML text (`1979-05-27T07:32:00Z`), checked before it is written.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
}

impl Kind {
    /// The kind's name in a form's field.
    pub fn name(self) -> &'static str {
        match self {
            Kind::String => "string",
            Kind::Integer => "integer",
            Kind::Float => "float",
            Kind::Boolean => "boolean",
            Kind::Datetime => "datetime",
        }
    }
}

impl Scalar {
    /// What a form's field holds for this value, as a person types it: a string as its
    /// characters, an integer in decimal, a float as Rust writes it, `true` or `false`, a
    /// date-time as TOML writes it.
    pub fn to_text(&self) -> String {
        match self {
            Scalar::String(s) | Scalar::Datetime(s) => s.clone(),
            Scalar::Integer(n) => n.to_string(),
            Scalar::Float(x) => x.to_string(),
            Scalar::Boolean(b) => b.to_string(),
        }
    }

    /// A field's text read back as a value of `kind`, or why it is not one. A date-time is only
    /// carried here; [`Document::set`] checks it.
    pub fn parse(kind: Kind, text: &str) -> Result<Scalar, String> {
        let t = text.trim();
        match kind {
            Kind::String => Ok(Scalar::String(text.to_string())),
            Kind::Integer => t.parse().map(Scalar::Integer).map_err(|_| format!("`{text}` is not a whole number from -9223372036854775808 to 9223372036854775807")),
            Kind::Float => match t {
                "inf" | "+inf" => Ok(Scalar::Float(f64::INFINITY)),
                "-inf" => Ok(Scalar::Float(f64::NEG_INFINITY)),
                "nan" | "+nan" | "-nan" => Ok(Scalar::Float(f64::NAN)),
                _ if t.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')) && !t.is_empty() => {
                    t.parse().map(Scalar::Float).map_err(|_| format!("`{text}` is not a number"))
                }
                _ => Err(format!("`{text}` is not a number")),
            },
            Kind::Boolean => match t {
                "true" => Ok(Scalar::Boolean(true)),
                "false" => Ok(Scalar::Boolean(false)),
                _ => Err(format!("`{text}` is neither `true` nor `false`")),
            },
            Kind::Datetime => Ok(Scalar::Datetime(t.to_string())),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Scalar::String(_) => Kind::String,
            Scalar::Integer(_) => Kind::Integer,
            Scalar::Float(_) => Kind::Float,
            Scalar::Boolean(_) => Kind::Boolean,
            Scalar::Datetime(_) => Kind::Datetime,
        }
    }

    fn of(v: &Value) -> Option<Scalar> {
        Some(match v {
            Value::String(f) => Scalar::String(f.value().clone()),
            Value::Integer(f) => Scalar::Integer(*f.value()),
            Value::Float(f) => Scalar::Float(*f.value()),
            Value::Boolean(f) => Scalar::Boolean(*f.value()),
            Value::Datetime(f) => Scalar::Datetime(f.value().to_string()),
            Value::Array(_) | Value::InlineTable(_) => return None,
        })
    }
}

/// One step of a key path: a key of a table, or the place of an element in an array or of a
/// table in an array of tables.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Segment {
    Key(String),
    Index(usize),
}

/// Where a value sits, from the top of the file: `log[0].timestamp.pattern`. Written as TOML
/// writes keys, a key that is not bare quoted (`"a.b"`), and read back the same way, so a path
/// survives the trip to the interface and back as text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct KeyPath(pub Vec<Segment>);

impl KeyPath {
    /// A path of keys alone.
    pub fn keys(keys: &[&str]) -> KeyPath {
        KeyPath(keys.iter().map(|k| Segment::Key((*k).to_string())).collect())
    }

    /// The top-level `format` or `version`: the file's stamp, which names its schema.
    fn is_stamp(&self) -> bool {
        matches!(self.0.as_slice(), [Segment::Key(k)] if k == "format" || k == "version")
    }
}

fn is_bare(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, s) in self.0.iter().enumerate() {
            match s {
                Segment::Index(n) => write!(f, "[{n}]")?,
                Segment::Key(k) => {
                    if i > 0 {
                        f.write_str(".")?;
                    }
                    if is_bare(k) {
                        f.write_str(k)?;
                    } else {
                        f.write_str("\"")?;
                        for c in k.chars() {
                            match c {
                                '"' => f.write_str("\\\"")?,
                                '\\' => f.write_str("\\\\")?,
                                c if c.is_control() => write!(f, "\\u{:04X}", c as u32)?,
                                c => write!(f, "{c}")?,
                            }
                        }
                        f.write_str("\"")?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// A path that does not read.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("not a key path at character {at}: {why}")]
pub struct PathError {
    pub at: usize,
    pub why: &'static str,
}

impl FromStr for KeyPath {
    type Err = PathError;

    fn from_str(s: &str) -> Result<KeyPath, PathError> {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        let mut out = Vec::new();
        let err = |at, why| Err(PathError { at, why });
        while i < chars.len() {
            match chars[i] {
                '[' => {
                    let start = i + 1;
                    let mut end = start;
                    while end < chars.len() && chars[end].is_ascii_digit() {
                        end += 1;
                    }
                    if end == start || end >= chars.len() || chars[end] != ']' {
                        return err(i, "an index is digits between `[` and `]`");
                    }
                    let digits: String = chars[start..end].iter().collect();
                    let Ok(n) = digits.parse() else { return err(start, "an index too large") };
                    out.push(Segment::Index(n));
                    i = end + 1;
                }
                '.' if !out.is_empty() => {
                    i += 1;
                    if i >= chars.len() {
                        return err(i, "a key after `.`");
                    }
                    let (key, next) = read_key(&chars, i)?;
                    out.push(Segment::Key(key));
                    i = next;
                }
                _ if out.is_empty() => {
                    let (key, next) = read_key(&chars, i)?;
                    out.push(Segment::Key(key));
                    i = next;
                }
                _ => return err(i, "`.` or `[` between the steps"),
            }
        }
        if out.is_empty() {
            return err(0, "an empty path");
        }
        Ok(KeyPath(out))
    }
}

/// A key at `i`: bare, or quoted with the escapes [`KeyPath`]'s text writes.
fn read_key(chars: &[char], mut i: usize) -> Result<(String, usize), PathError> {
    let err = |at, why| Err(PathError { at, why });
    if chars[i] != '"' {
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
            i += 1;
        }
        if i == start {
            return err(start, "a bare key is ASCII letters, digits, `-` and `_`; quote any other");
        }
        return Ok((chars[start..i].iter().collect(), i));
    }
    let mut key = String::new();
    i += 1;
    loop {
        match chars.get(i) {
            None => return err(i, "a quoted key closes with `\"`"),
            Some('"') => return Ok((key, i + 1)),
            Some('\\') => match chars.get(i + 1) {
                Some('"') => {
                    key.push('"');
                    i += 2;
                }
                Some('\\') => {
                    key.push('\\');
                    i += 2;
                }
                Some('u') if i + 6 <= chars.len() => {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) else {
                        return err(i, "`\\u` takes four hex digits of a character");
                    };
                    key.push(c);
                    i += 6;
                }
                _ => return err(i, "an escape is `\\\"`, `\\\\` or `\\uXXXX`"),
            },
            Some(c) => {
                key.push(*c);
                i += 1;
            }
        }
    }
}

/// One scalar of the file, as the form shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub path: KeyPath,
    pub value: Scalar,
    /// The value exactly as the text writes it: `0x1F`, `'C:\dir'`.
    pub raw: String,
    /// The line it starts on, from 1: where the text view goes when the field is picked.
    pub line: usize,
    /// The comment written after it on its line, without its `#` and outer spaces.
    pub comment: Option<String>,
    /// `format` or `version` at the top: shown, never edited through the form.
    pub stamp: bool,
}

/// Where a text fails to parse. `line` and `column` count from 1, the column in UTF-16 code
/// units as both views count it (a QML text field and Monaco alike): `ã` is one, `😀` two.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("line {line}, column {column}: {message}")]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    /// The byte in the document's text (LF, no mark) where it fails.
    pub offset: usize,
}

/// Why a file's bytes do not open.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// TOML is UTF-8; `offset` is the first byte of the file, its mark counted, that begins no
    /// character.
    #[error("not text in UTF-8: the byte at offset {offset} begins no character")]
    NotUtf8 { offset: usize },
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Why a form edit is refused; the document is left as it was.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EditError {
    #[error("no value at `{0}`")]
    NoValue(KeyPath),
    #[error("`{0}` is no single value: a table or an array is edited as text")]
    NotScalar(KeyPath),
    #[error("`{0}` is the file's stamp: its format and version are not the form's to change")]
    Stamp(KeyPath),
    #[error("`{path}` holds {found}, and the form sets {sent}: a value keeps its type")]
    Kind { path: KeyPath, found: Kind, sent: Kind },
    #[error("`{path}`: {why}")]
    BadValue { path: KeyPath, why: String },
}

/// One TOML file, held as text that parses.
#[derive(Debug, Clone)]
pub struct Document {
    parsed: Parsed,
    endings: LineEndings,
    bom: bool,
    revision: u64,
}

const BOM: &str = "\u{feff}";

impl Document {
    /// A file's bytes, as read from the disk: UTF-8 with or without a mark, its lines ended in
    /// LF, CR LF or both.
    pub fn from_bytes(bytes: &[u8]) -> Result<Document, OpenError> {
        let text = std::str::from_utf8(bytes).map_err(|e| OpenError::NotUtf8 { offset: e.valid_up_to() })?;
        let (bom, text) = match text.strip_prefix(BOM) {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let endings = LineEndings::of(text);
        let parsed = parse(text.replace("\r\n", "\n"))?;
        Ok(Document { parsed, endings, bom, revision: 0 })
    }

    /// The file's bytes: the text with its mark and line endings put back as they were read.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(self.text().len() + 8);
        if self.bom {
            out.push_str(BOM);
        }
        match self.endings.written() {
            "\n" => out.push_str(self.text()),
            ending => out.push_str(&self.text().replace('\n', ending)),
        }
        out.into_bytes()
    }

    /// Write the file's bytes to `path` whole or not at all: into a file beside it, then renamed
    /// over it, so a cut in the middle leaves the old file as it was.
    pub fn write_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut beside = path.as_os_str().to_owned();
        beside.push(".oficina-new");
        let beside = std::path::PathBuf::from(beside);
        std::fs::write(&beside, self.to_bytes())?;
        std::fs::rename(&beside, path)
    }

    /// The text both views show: LF endings, no mark.
    pub fn text(&self) -> &str {
        self.parsed.raw()
    }

    pub fn endings(&self) -> LineEndings {
        self.endings
    }

    pub fn has_bom(&self) -> bool {
        self.bom
    }

    /// Counts the changes made since the file was read, one per accepted edit of either view, so
    /// a view knows the other changed the document.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The file's stamp, read by core's one header reader.
    pub fn stamp(&self) -> Result<Stamp, HeaderError> {
        schema::stamp(self.text(), "a TOML file")
    }

    /// Every scalar of the file, in the order the text writes them: the form's fields.
    pub fn fields(&self) -> Vec<Field> {
        let raw = self.text();
        let mut out = Vec::new();
        walk_table(raw, self.parsed.as_table(), &mut Vec::new(), &mut out);
        out.sort_by_key(|(start, _)| *start);
        out.into_iter().map(|(_, f)| f).collect()
    }

    /// The field at `path`, if it is a scalar.
    pub fn field(&self, path: &KeyPath) -> Option<Field> {
        self.fields().into_iter().find(|f| &f.path == path)
    }

    /// The text view's edit: the whole text replaced. CR LF read as LF. A text that does not
    /// parse is refused with where it fails, and the document keeps its last text.
    pub fn set_text(&mut self, text: &str) -> Result<(), ParseError> {
        let text = text.replace("\r\n", "\n");
        if text == self.text() {
            return Ok(());
        }
        self.parsed = parse(text)?;
        self.revision += 1;
        Ok(())
    }

    /// The form's edit: the scalar at `path` becomes `value`, of the same type, written in the
    /// old value's quoting (a literal string stays literal where the new text allows it) and
    /// radix (a hexadecimal, octal or binary integer stays so where it is not negative), with the
    /// old value's surroundings, its trailing comment among them. The stamp is refused.
    pub fn set(&mut self, path: &KeyPath, value: Scalar) -> Result<(), EditError> {
        if path.is_stamp() {
            return Err(EditError::Stamp(path.clone()));
        }
        let mut doc: DocumentMut = self.parsed.clone().into_mut();
        let slot = match value_mut(&mut doc, path) {
            Reached::Value(v) => v,
            Reached::Nothing => return Err(EditError::NoValue(path.clone())),
            Reached::NotAValue => return Err(EditError::NotScalar(path.clone())),
        };
        let found = Scalar::of(slot).ok_or_else(|| EditError::NotScalar(path.clone()))?.kind();
        if found != value.kind() {
            return Err(EditError::Kind { path: path.clone(), found, sent: value.kind() });
        }
        let mut new = written(slot, &value).map_err(|why| EditError::BadValue { path: path.clone(), why })?;
        *new.decor_mut() = slot.decor().clone();
        *slot = new;
        let text = doc.to_string();
        if text == self.text() {
            return Ok(());
        }
        // The text toml_edit writes always parses; a refusal here is this module's bug, said so.
        self.parsed = parse(text).map_err(|e| EditError::BadValue { path: path.clone(), why: format!("the edited text does not parse: {e}") })?;
        self.revision += 1;
        Ok(())
    }
}

fn parse(text: String) -> Result<Parsed, ParseError> {
    Parsed::parse(text.clone()).map_err(|e| {
        let offset = e.span().map_or(0, |s| s.start);
        let (line, column) = position(&text, offset);
        ParseError { message: e.message().to_string(), line, column, offset }
    })
}

/// The line and the UTF-16 column, both from 1, of a byte in `text`.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let mut at = offset.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].encode_utf16().count() + 1)
}

/// The text of a piece of decoration, whether the parser left it as a span of `raw` or as text.
fn decor_text<'a>(raw: &'a str, r: Option<&'a toml_edit::RawString>) -> &'a str {
    r.and_then(|r| r.as_str().or_else(|| r.span().and_then(|s| raw.get(s)))).unwrap_or("")
}

type Found = (usize, Field);

fn walk_table(raw: &str, t: &Table, path: &mut Vec<Segment>, out: &mut Vec<Found>) {
    for (k, item) in t.iter() {
        path.push(Segment::Key(k.to_string()));
        walk_item(raw, item, path, out);
        path.pop();
    }
}

fn walk_item(raw: &str, item: &Item, path: &mut Vec<Segment>, out: &mut Vec<Found>) {
    match item {
        Item::Value(v) => walk_value(raw, v, path, out),
        Item::Table(t) => walk_table(raw, t, path, out),
        Item::ArrayOfTables(a) => {
            for (i, t) in a.iter().enumerate() {
                path.push(Segment::Index(i));
                walk_table(raw, t, path, out);
                path.pop();
            }
        }
        Item::None => {}
    }
}

fn walk_value(raw: &str, v: &Value, path: &mut Vec<Segment>, out: &mut Vec<Found>) {
    match v {
        Value::Array(a) => {
            for (i, e) in a.iter().enumerate() {
                path.push(Segment::Index(i));
                walk_value(raw, e, path, out);
                path.pop();
            }
        }
        Value::InlineTable(t) => {
            for (k, e) in t.iter() {
                path.push(Segment::Key(k.to_string()));
                walk_value(raw, e, path, out);
                path.pop();
            }
        }
        scalar => {
            let Some(value) = Scalar::of(scalar) else { return };
            let span = scalar.span().unwrap_or(0..0);
            let suffix = decor_text(raw, scalar.decor().suffix());
            let comment = suffix.split_once('#').map(|(_, c)| c.trim().to_string());
            let path = KeyPath(path.clone());
            let stamp = path.is_stamp();
            let field = Field { raw: raw.get(span.clone()).unwrap_or("").to_string(), line: position(raw, span.start).0, path, value, comment, stamp };
            out.push((span.start, field));
        }
    }
}

/// Where a walk to a value stands.
enum Place<'a> {
    Item(&'a mut Item),
    Table(&'a mut Table),
    Value(&'a mut Value),
}

fn step<'a>(place: Place<'a>, s: &Segment) -> Option<Place<'a>> {
    match (place, s) {
        (Place::Item(Item::Value(v)), s) => step(Place::Value(v), s),
        (Place::Item(Item::Table(t)) | Place::Table(t), Segment::Key(k)) => t.get_mut(k).map(Place::Item),
        (Place::Item(Item::ArrayOfTables(a)), Segment::Index(i)) => a.get_mut(*i).map(Place::Table),
        (Place::Value(Value::InlineTable(t)), Segment::Key(k)) => t.get_mut(k).map(Place::Value),
        (Place::Value(Value::Array(a)), Segment::Index(i)) => a.get_mut(*i).map(Place::Value),
        _ => None,
    }
}

/// What a walk down a path found at its end.
enum Reached<'a> {
    Value(&'a mut Value),
    /// A table or an array of tables.
    NotAValue,
    Nothing,
}

fn value_mut<'a>(doc: &'a mut DocumentMut, path: &KeyPath) -> Reached<'a> {
    let mut place = Place::Item(doc.as_item_mut());
    for s in &path.0 {
        match step(place, s) {
            Some(next) => place = next,
            None => return Reached::Nothing,
        }
    }
    match place {
        Place::Item(Item::Value(v)) | Place::Value(v) => Reached::Value(v),
        Place::Item(Item::None) => Reached::Nothing,
        _ => Reached::NotAValue,
    }
}

/// The old value's text as written, once the parser's spans are gone.
fn repr_of(v: &Value) -> &str {
    let raw = match v {
        Value::String(f) => f.as_repr().map(|r| r.as_raw()),
        Value::Integer(f) => f.as_repr().map(|r| r.as_raw()),
        _ => None,
    };
    raw.and_then(|r| r.as_str()).unwrap_or("")
}

/// The new value, written as TOML in the old one's style where it allows.
fn written(old: &Value, value: &Scalar) -> Result<Value, String> {
    let text = match value {
        Scalar::String(s) => {
            let literal = repr_of(old).starts_with('\'') && !repr_of(old).starts_with("'''");
            if literal && !s.chars().any(|c| c == '\'' || (c.is_control() && c != '\t')) {
                format!("'{s}'")
            } else {
                return Ok(Value::from(s.as_str()));
            }
        }
        Scalar::Integer(n) => {
            let old = repr_of(old);
            let upper = old.chars().skip(2).any(|c| c.is_ascii_uppercase());
            match (old.get(..2), *n >= 0) {
                (Some("0x"), true) if upper => format!("0x{n:X}"),
                (Some("0x"), true) => format!("0x{n:x}"),
                (Some("0o"), true) => format!("0o{n:o}"),
                (Some("0b"), true) => format!("0b{n:b}"),
                _ => return Ok(Value::from(*n)),
            }
        }
        Scalar::Float(x) => return Ok(Value::from(*x)),
        Scalar::Boolean(b) => return Ok(Value::from(*b)),
        Scalar::Datetime(t) => {
            let d: toml_edit::Datetime = t.parse().map_err(|_| format!("`{t}` is not a TOML date-time"))?;
            return Ok(Value::from(d));
        }
    };
    text.parse::<Value>().map_err(|e| e.to_string())
}
