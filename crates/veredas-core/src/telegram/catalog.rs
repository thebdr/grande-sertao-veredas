//! The catalog file: the messages a channel speaks, by name, by id, and by the rules their
//! bodies are written and read with.
//!
//! A versioned TOML file, like every plain-text file of the suite (the plan, "Versioned files"):
//! it states `format = "veredas-catalog"` and `version = 1`, and a reader refuses a format or
//! version it does not know rather than guess. Every key is in `docs/catalog-schema.md`. A
//! message holds its fields once: [`crate::telegram::compose`] writes them and
//! [`crate::telegram::capture`] reads them, so a body a channel sends is a body the same
//! catalog reads back. Core knows no device: a device's catalog is a file like the documented
//! one, kept with its channel file outside the suite's code.
//!
//! ```toml
//! format = "veredas-catalog"
//! version = 1
//! id_field = "id"            # the frame field carrying a message's id
//! byte_order = "little"      # of the binary fields in bodies
//!
//! [[message]]
//! name = "answer"
//! id = 22
//! len = 8
//! fields = [
//!   { name = "item", at = 0, ty = "U32" },
//!   { name = "dest", at = 4, ty = "U16" },
//!   { name = "flag", at = 6, ty = "U8" },
//! ]
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::telegram::Endian;
use crate::telegram::capture::{BinaryField, Capture, CaptureSpec, RepeatSpec};
use crate::telegram::compose::{Compose, ComposeError, ComposeSpec, width};
use crate::telegram::framing::{FieldRole, Frame, Framing, Segment};

/// The format name every catalog file states.
pub const FORMAT: &str = "veredas-catalog";
/// The one version this reader knows.
pub const VERSION: u32 = 1;

/// Why a catalog file was refused. Never a guess: an unknown version, a misspelled key, a name
/// or id used twice, a field past its body, a bad pattern or template is named.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("not a catalog file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `{FORMAT}`")]
    WrongFormat(String),
    #[error("version {0} of `{FORMAT}` is not known here (this reader knows {VERSION})")]
    UnknownVersion(u32),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum OrderFile {
    Little,
    Big,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    id_field: String,
    byte_order: Option<OrderFile>,
    #[serde(default)]
    message: Vec<MessageFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageFile {
    name: String,
    id: u64,
    len: Option<usize>,
    #[serde(default)]
    pad: u8,
    #[serde(default)]
    fields: Vec<BinaryField>,
    payload: Option<String>,
    template: Option<String>,
    #[serde(default)]
    repeat: Vec<RepeatSpec>,
}

/// One message of a catalog: its name, its id, and its body rules compiled both ways.
#[derive(Clone)]
pub struct Message {
    pub name: String,
    pub id: u64,
    /// The reading rules, as configured.
    pub capture_spec: CaptureSpec,
    /// The writing rules, as configured.
    pub compose_spec: ComposeSpec,
    capture: std::sync::Arc<Capture>,
    compose: Compose,
    /// The payload pattern's named groups.
    groups: Vec<String>,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("name", &self.name)
            .field("id", &self.id)
            .field("capture_spec", &self.capture_spec)
            .field("compose_spec", &self.compose_spec)
            .finish()
    }
}

impl Message {
    /// Read a body into its fields; `None` when the body does not satisfy the rules, or is not
    /// of the message's fixed length when it declares one (a body of another length is not this
    /// message, whatever its fields would read as).
    pub fn read(&self, body: &[u8], endian: Endian) -> Option<HashMap<String, String>> {
        if let Some(len) = self.compose_spec.len
            && body.len() != len
        {
            return None;
        }
        self.capture.extract(body, endian)
    }

    /// Write a body from named values. A message that only reads (a payload pattern or a repeat,
    /// no template and no fields) cannot be written: its values would be dropped into an empty
    /// body.
    pub fn write(&self, values: &HashMap<String, String>, endian: Endian) -> Result<Vec<u8>, ComposeError> {
        if (self.capture_spec.payload.is_some() || !self.capture_spec.repeat.is_empty()) && !self.compose.writes() {
            return Err(ComposeError::NoRules);
        }
        self.compose.render(values, endian)
    }

    /// The names a body needs to be written.
    pub fn names(&self) -> Vec<String> {
        self.compose.names()
    }

    /// Whether a body read by these rules can yield `name`: a binary field's, a payload
    /// group's, or a repeat's numbered name (`name1` to `name<max>`, by the same parsing rule
    /// the catalog refuses collisions with). A file read against a catalog asks this rather than
    /// re-deriving the catalog's names.
    pub fn reads(&self, name: &str) -> bool {
        self.capture_spec.fields.iter().any(|f| f.name == name)
            || self.groups.iter().any(|g| g == name)
            || self.capture_spec.repeat.iter().any(|r| numbered_by(name, &r.name, r.max))
    }
}

/// A loaded catalog.
#[derive(Clone, Debug)]
pub struct Catalog {
    /// The frame field (a segment header's, or the envelope's in a layout without segments)
    /// that carries a message's id.
    pub id_field: String,
    /// The byte order of the binary fields in bodies.
    pub byte_order: Endian,
    pub messages: Vec<Message>,
}

impl Catalog {
    pub fn by_name(&self, name: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.name == name)
    }

    pub fn by_id(&self, id: u64) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// The message with this id and its fields read from `body`, in the catalog's byte order;
    /// `None` for an id the catalog does not hold or a body its rules do not accept.
    pub fn decode(&self, id: u64, body: &[u8]) -> Option<(&Message, HashMap<String, String>)> {
        let message = self.by_id(id)?;
        let fields = message.read(body, self.byte_order)?;
        Some((message, fields))
    }

    /// The id and the body of the message named, written from `values` in the catalog's byte
    /// order; `None` for a name the catalog does not hold.
    pub fn encode(&self, name: &str, values: &HashMap<String, String>) -> Option<Result<(u64, Vec<u8>), ComposeError>> {
        let message = self.by_name(name)?;
        Some(message.write(values, self.byte_order).map(|body| (message.id, body)))
    }

    /// Where a catalog meets a channel file: the catalog knows no framing, so whether its ids
    /// can ride the framing is checked here, before a channel that speaks it binds. The
    /// `id_field` must be a field of the framing's segment header (of the envelope in a layout
    /// without segments) that the framing leaves to the sender (an opaque field: never the
    /// sequence, a length or a count, which a channel fills in itself) and wide enough for
    /// every id; and in a layout without segments no message may write an empty body, which
    /// the wire carries as a keep-alive, not a message.
    pub fn check_framing(&self, framing: &Framing) -> Result<(), CatalogError> {
        let id_field = &self.id_field;
        let (fields, where_) = match &framing.segment {
            Some(header) => (header, "segment header"),
            None => (&framing.envelope, "envelope"),
        };
        let Some(field) = fields.iter().find(|f| f.name == *id_field) else {
            return Err(invalid(format!("the catalog's id_field `{id_field}` is no field of the framing's {where_}")));
        };
        let role = match field.role {
            FieldRole::Opaque => None,
            FieldRole::Sequence => Some("sequence"),
            FieldRole::Length => Some("length"),
            FieldRole::Count => Some("count"),
        };
        if let Some(role) = role {
            return Err(invalid(format!(
                "the catalog's id_field `{id_field}` is the framing's {where_} field with the role `{role}`, which the channel fills in itself: an id cannot ride it"
            )));
        }
        let widest = if field.size >= 8 { u64::MAX } else { (1u64 << (8 * field.size)) - 1 };
        if let Some(m) = self.messages.iter().find(|m| m.id > widest) {
            return Err(invalid(format!(
                "the catalog's message `{}` has the id {}, which does not fit the framing's {where_} field `{id_field}` of {} byte(s)",
                m.name, m.id, field.size
            )));
        }
        if framing.segment.is_none()
            && let Some(m) = self.messages.iter().find(|m| m.compose.writes_empty() && m.capture_spec.payload.is_none() && m.capture_spec.repeat.is_empty())
        {
            return Err(invalid(format!(
                "the catalog's message `{}` writes an empty body, which a layout without segments carries as a keep-alive, not a message",
                m.name
            )));
        }
        Ok(())
    }

    /// A frame carrying `id` and `body` where this framing takes them: the id in the segment
    /// header's field, or in the envelope's in a layout without segments.
    pub fn frame(&self, framing: &Framing, id: u64, body: Vec<u8>) -> Frame {
        let id_field = self.id_field.clone();
        match &framing.segment {
            Some(_) => Frame { fields: BTreeMap::new(), segments: vec![Segment { fields: BTreeMap::from([(id_field, id)]), body }] },
            None => Frame { fields: BTreeMap::from([(id_field, id)]), segments: vec![Segment { fields: BTreeMap::new(), body }] },
        }
    }

    /// The messages a received frame carries: `(id, body)` per segment, the id from each
    /// segment header, or from the envelope for the one body of a layout without segments;
    /// `None` for an id the frame does not carry.
    pub fn messages_of(&self, framing: &Framing, frame: &Frame) -> Vec<(Option<u64>, Vec<u8>)> {
        let id_field = &self.id_field;
        match &framing.segment {
            Some(_) => frame.segments.iter().map(|s| (s.fields.get(id_field).copied(), s.body.clone())).collect(),
            None => frame.segments.iter().map(|s| (frame.fields.get(id_field).copied(), s.body.clone())).collect(),
        }
    }
}

fn invalid(msg: impl Into<String>) -> CatalogError {
    CatalogError::Invalid(msg.into())
}

/// Whether `name` is one of the numbered names the repeat `r` reads, `r1` to `r<max>`: `r`
/// followed by a decimal with no sign and no leading zero, from 1 to `max`. Decided by parsing,
/// never by listing the names: `max` may be as large as the file likes.
fn numbered_by(name: &str, r: &str, max: usize) -> bool {
    let Some(rest) = name.strip_prefix(r) else { return false };
    if rest.is_empty() || rest.starts_with('0') || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    rest.parse::<usize>().is_ok_and(|n| n >= 1 && n <= max)
}

/// A name two repeats would both read, when there is one. The longer name is the shorter one
/// followed by text `t`; its numbered names are the shorter's only when `t` is digits with no
/// leading zero, and the least of them, `<long>1`, is `<short><t>1`: within the shorter's `max`
/// or not, every other is larger.
fn repeats_meet(a: &str, a_max: usize, b: &str, b_max: usize) -> Option<String> {
    let meet = |short: &str, short_max: usize, long: &str| {
        let first = format!("{long}1");
        numbered_by(&first, short, short_max).then_some(first)
    };
    // Each repeat reads at least `<name>1`: `max` is never 0 here.
    debug_assert!(a_max >= 1 && b_max >= 1);
    meet(a, a_max, b).or_else(|| meet(b, b_max, a))
}

/// Read and validate a catalog file.
pub fn load(text: &str) -> Result<Catalog, CatalogError> {
    let header: Header = toml::from_str(text).map_err(|e| CatalogError::NotToml(e.to_string()))?;
    match header.format.as_deref() {
        Some(FORMAT) => {}
        Some(other) => return Err(CatalogError::WrongFormat(other.to_string())),
        None => return Err(invalid("no `format`: a catalog file states `format` and `version`")),
    }
    match header.version {
        Some(VERSION) => {}
        Some(v) => return Err(CatalogError::UnknownVersion(v)),
        None => return Err(invalid("no `version`: a catalog file states `format` and `version`")),
    }
    let file: FileV1 = toml::from_str(text).map_err(|e| CatalogError::NotToml(e.to_string()))?;
    if file.id_field.trim().is_empty() {
        return Err(invalid("id_field is empty: name the frame field that carries a message's id"));
    }
    let byte_order = match file.byte_order {
        Some(OrderFile::Big) => Endian::Big,
        Some(OrderFile::Little) | None => Endian::Little,
    };
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    let mut messages = Vec::new();
    // A name is exact: no surrounding whitespace, compared as written.
    let exact = |place: &str, what: &str, name: &str| -> Result<(), CatalogError> {
        if name.trim().is_empty() {
            return Err(invalid(format!("{place}: {what} has no name")));
        }
        if name != name.trim() {
            return Err(invalid(format!("{place}: the name `{name}` has surrounding whitespace")));
        }
        Ok(())
    };
    for (i, m) in file.message.into_iter().enumerate() {
        let place = format!("message[{i}]");
        exact(&place, "the message", &m.name)?;
        let place = format!("{place} (`{}`)", m.name);
        if !names.insert(m.name.clone()) {
            return Err(invalid(format!("{place}: two messages are named `{}`", m.name)));
        }
        if !ids.insert(m.id) {
            return Err(invalid(format!("{place}: two messages have the id {}", m.id)));
        }
        // The names a payload pattern reads, and the numbered names each repeat reads: no two
        // readers may write one name, or one would silently overwrite the other.
        let groups: Vec<String> = match &m.payload {
            Some(p) => regex::Regex::new(p).map(|re| re.capture_names().flatten().map(String::from).collect()).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut repeat_names: Vec<(String, usize)> = Vec::new();
        for (j, r) in m.repeat.iter().enumerate() {
            let where_ = format!("{place}.repeat[{j}]");
            exact(&where_, "the repeat", &r.name)?;
            if r.max == 0 {
                return Err(invalid(format!("{where_} (`{}`): max is 0", r.name)));
            }
            if repeat_names.iter().any(|(n, _)| *n == r.name) {
                return Err(invalid(format!("{place}: two repeats are named `{}`", r.name)));
            }
            if let Some(name) = groups.iter().find(|g| numbered_by(g, &r.name, r.max)) {
                return Err(invalid(format!("{where_} (`{}`): the payload pattern reads a group named `{name}`", r.name)));
            }
            if let Some((other, name)) = repeat_names.iter().find_map(|(o, max)| repeats_meet(o, *max, &r.name, r.max).map(|n| (o, n))) {
                return Err(invalid(format!("{where_} (`{}`): the repeat `{other}` reads `{name}` too", r.name)));
            }
            repeat_names.push((r.name.clone(), r.max));
        }
        let mut field_names = HashSet::new();
        for f in &m.fields {
            exact(&place, "a field", &f.name)?;
            if !field_names.insert(f.name.clone()) {
                return Err(invalid(format!("{place}: two fields are named `{}`", f.name)));
            }
            if groups.contains(&f.name) {
                return Err(invalid(format!("{place}.{}: the payload pattern reads a group of that name", f.name)));
            }
            if let Some((r, _)) = repeat_names.iter().find(|(r, max)| numbered_by(&f.name, r, *max)) {
                return Err(invalid(format!("{place}.{}: the repeat `{r}` reads a field of that name", f.name)));
            }
            let end = f.at.checked_add(width(f)).ok_or_else(|| invalid(format!("{place}.{}: the offset is beyond any body", f.name)))?;
            if let Some(len) = m.len
                && end > len
            {
                return Err(invalid(format!("{place}.{}: ends at byte {end}, past the body's {len}", f.name)));
            }
            if end > crate::telegram::compose::MAX_BODY {
                return Err(invalid(format!("{place}.{}: ends at byte {end}, past the largest body", f.name)));
            }
        }
        let capture_spec = CaptureSpec { payload: m.payload, fields: m.fields.clone(), repeat: m.repeat };
        let capture = Capture::new(&capture_spec);
        if let Some(w) = capture.warnings.first() {
            return Err(invalid(format!("{place}: {w}")));
        }
        let compose_spec = ComposeSpec { template: m.template, fields: m.fields, len: m.len, pad: m.pad };
        let compose = Compose::new(&compose_spec).map_err(|e| invalid(format!("{place}: {e}")))?;
        messages.push(Message {
            name: m.name,
            id: m.id,
            capture_spec,
            compose_spec,
            capture: std::sync::Arc::new(capture),
            compose,
            groups,
        });
    }
    Ok(Catalog { id_field: file.id_field, byte_order, messages })
}
