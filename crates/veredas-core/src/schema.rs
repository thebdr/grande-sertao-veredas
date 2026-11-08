//! One header reader for every plain-text file of the suite (the plan, "Versioned files"):
//! `format` and `version` are read before the body, so a file of another format, an unknown
//! version or a missing stamp is refused by name, and a reader that knows an older version can
//! hand it to a migration instead of guessing. Each file's reader states its own format and the
//! version it knows, names its file (`a catalog file`) for the texts, and maps a refusal onto
//! its own error; the texts are the ones every reference pins.

use serde::Deserialize;
use thiserror::Error;

/// The stamp every file opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub format: String,
    pub version: u32,
}

/// Why a header was refused. `noun` is what the reader calls its file, as in `a bench file`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// The text is not TOML at all, with the parser's reason.
    #[error("not {noun}: {reason}")]
    NotToml { noun: String, reason: String },
    #[error("no `format`: {noun} states `format` and `version`")]
    NoFormat { noun: String },
    #[error("`format` is `{found}`, not `{expected}`")]
    WrongFormat { found: String, expected: String },
    #[error("no `version`: {noun} states `format` and `version`")]
    NoVersion { noun: String },
    #[error("version {found} of `{format}` is not known here (this reader knows {known})")]
    UnknownVersion { found: u32, format: String, known: u32 },
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

fn header(text: &str, noun: &str) -> Result<Header, HeaderError> {
    toml::from_str(text).map_err(|e| HeaderError::NotToml { noun: noun.to_string(), reason: e.to_string() })
}

/// The stamp of a file's text, whatever its format: both keys present, nothing else read. For
/// a reader that knows more than one version and decides what to do with each.
pub fn stamp(text: &str, noun: &str) -> Result<Stamp, HeaderError> {
    let h = header(text, noun)?;
    let format = h.format.ok_or_else(|| HeaderError::NoFormat { noun: noun.to_string() })?;
    let version = h.version.ok_or_else(|| HeaderError::NoVersion { noun: noun.to_string() })?;
    Ok(Stamp { format, version })
}

/// Check a file's header against the format and the one version a reader knows, format first:
/// no `format`, another format, no `version`, another version, each refused by name.
pub fn check(text: &str, noun: &str, format: &str, version: u32) -> Result<(), HeaderError> {
    let h = header(text, noun)?;
    match h.format.as_deref() {
        None => return Err(HeaderError::NoFormat { noun: noun.to_string() }),
        Some(f) if f == format => {}
        Some(other) => return Err(HeaderError::WrongFormat { found: other.to_string(), expected: format.to_string() }),
    }
    match h.version {
        None => Err(HeaderError::NoVersion { noun: noun.to_string() }),
        Some(v) if v == version => Ok(()),
        Some(v) => Err(HeaderError::UnknownVersion { found: v, format: format.to_string(), known: version }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_checked_format_first_then_version_with_the_pinned_texts() {
        let ok = "format = \"veredas-thing\"\nversion = 1\nbody = 1\n";
        assert_eq!(check(ok, "a thing file", "veredas-thing", 1), Ok(()));
        assert_eq!(stamp(ok, "a thing file"), Ok(Stamp { format: "veredas-thing".into(), version: 1 }));
        let e = check("not = [toml", "a thing file", "veredas-thing", 1).unwrap_err();
        assert!(matches!(&e, HeaderError::NotToml { noun, .. } if noun == "a thing file"), "{e}");
        assert!(e.to_string().starts_with("not a thing file: "), "{e}");
        assert_eq!(
            check("version = 1\n", "a thing file", "veredas-thing", 1).unwrap_err().to_string(),
            "no `format`: a thing file states `format` and `version`"
        );
        assert_eq!(
            check("format = \"veredas-other\"\n", "a thing file", "veredas-thing", 1).unwrap_err().to_string(),
            "`format` is `veredas-other`, not `veredas-thing`"
        );
        assert_eq!(
            check("format = \"veredas-thing\"\n", "a thing file", "veredas-thing", 1).unwrap_err().to_string(),
            "no `version`: a thing file states `format` and `version`"
        );
        assert_eq!(
            check("format = \"veredas-thing\"\nversion = 2\n", "a thing file", "veredas-thing", 1).unwrap_err().to_string(),
            "version 2 of `veredas-thing` is not known here (this reader knows 1)"
        );
        // The stamp alone reads any format, so a migrating reader can decide; both keys are
        // still required.
        assert_eq!(stamp("format = \"veredas-other\"\nversion = 7\n", "a file").unwrap().version, 7);
        assert_eq!(stamp("format = \"veredas-other\"\n", "a file").unwrap_err().to_string(), "no `version`: a file states `format` and `version`");
    }
}
