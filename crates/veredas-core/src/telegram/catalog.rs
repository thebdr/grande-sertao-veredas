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

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::telegram::Endian;
use crate::telegram::capture::{BinaryField, Capture, CaptureSpec, FieldType, RepeatSpec};
use crate::telegram::compose::{Compose, ComposeError, ComposeSpec};

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

    /// Write a body from named values.
    pub fn write(&self, values: &HashMap<String, String>, endian: Endian) -> Result<Vec<u8>, ComposeError> {
        self.compose.render(values, endian)
    }

    /// The names a body needs to be written.
    pub fn names(&self) -> Vec<String> {
        self.compose.names()
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
}

fn invalid(msg: impl Into<String>) -> CatalogError {
    CatalogError::Invalid(msg.into())
}

/// A binary field's width in bytes, as capture reads it and compose writes it.
fn width(f: &BinaryField) -> usize {
    match f.ty {
        FieldType::U8 => 1,
        FieldType::U16 | FieldType::I16 => 2,
        FieldType::U32 | FieldType::I32 => 4,
        FieldType::Ascii | FieldType::Hex => f.len,
    }
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
    for (i, m) in file.message.into_iter().enumerate() {
        let place = format!("message[{i}]");
        if m.name.trim().is_empty() {
            return Err(invalid(format!("{place}: the message has no name")));
        }
        let place = format!("{place} (`{}`)", m.name);
        if !names.insert(m.name.clone()) {
            return Err(invalid(format!("{place}: two messages are named `{}`", m.name)));
        }
        if !ids.insert(m.id) {
            return Err(invalid(format!("{place}: two messages have the id {}", m.id)));
        }
        let mut field_names = HashSet::new();
        for f in &m.fields {
            if f.name.trim().is_empty() {
                return Err(invalid(format!("{place}: a field has no name")));
            }
            if !field_names.insert(f.name.clone()) {
                return Err(invalid(format!("{place}: two fields are named `{}`", f.name)));
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
        });
    }
    Ok(Catalog { id_field: file.id_field, byte_order, messages })
}
