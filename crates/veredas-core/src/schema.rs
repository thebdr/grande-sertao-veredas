//! One header reader for every plain-text file of the suite (the plan, "Versioned files"):
//! `format` and `version` are read before the body, so a file of another format, an unknown
//! version or a missing stamp is refused by name, and a reader that knows an older version can
//! hand it to a migration instead of guessing. Each file's reader states its own format and the
//! version it knows, names its file (`a catalog file`) for the texts, and maps a refusal onto
//! its own error; the texts are the ones every reference pins.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};
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
    /// The text is not TOML at all, with the parser's reason; or its stamp is of the wrong type
    /// (a `format` that is no string, a `version` that is no whole number), named by its key: a
    /// stamp of the wrong type is no stamp.
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

/// A header key as the file types it: a string, a whole number of any width the parser reads
/// (past 64 bits included, so the key's type is judged here and never by a value type's
/// limits), or something else, taken in whole and set aside.
enum Keyed {
    Str(String),
    Int(i128),
    Other,
}

impl<'de> Deserialize<'de> for Keyed {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Keyed;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a header key")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Keyed, E> {
                Ok(Keyed::Str(v.to_string()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Keyed, E> {
                Ok(Keyed::Str(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Keyed, E> {
                Ok(Keyed::Int(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Keyed, E> {
                Ok(Keyed::Int(v.into()))
            }
            fn visit_i128<E: de::Error>(self, v: i128) -> Result<Keyed, E> {
                Ok(Keyed::Int(v))
            }
            fn visit_u128<E: de::Error>(self, v: u128) -> Result<Keyed, E> {
                Ok(Keyed::Int(i128::try_from(v).unwrap_or(i128::MAX)))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Keyed, E> {
                Ok(Keyed::Other)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Keyed, E> {
                Ok(Keyed::Other)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Keyed, E> {
                Ok(Keyed::Other)
            }
            fn visit_none<E: de::Error>(self) -> Result<Keyed, E> {
                Ok(Keyed::Other)
            }
            fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<Keyed, E> {
                Ok(Keyed::Other)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Keyed, A::Error> {
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(Keyed::Other)
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Keyed, A::Error> {
                while map.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {}
                Ok(Keyed::Other)
            }
        }
        d.deserialize_any(V)
    }
}

/// The two keys as they are typed in the file; every other key is skipped unread, so a body
/// value the stamp never looks at (an offset past `i64`, say) is the reader's own to judge.
#[derive(Deserialize)]
struct Header {
    format: Option<Keyed>,
    version: Option<Keyed>,
}

fn not(noun: &str, reason: impl ToString) -> HeaderError {
    HeaderError::NotToml { noun: noun.to_string(), reason: reason.to_string() }
}

/// The text parsed for its syntax, the body included and skipped (a body that is not TOML is
/// refused here, with the parser's reason, before any key is read); nothing of it interpreted.
fn header(text: &str, noun: &str) -> Result<Header, HeaderError> {
    toml::from_str(text).map_err(|e| not(noun, e))
}

/// The `format` key, decided whole before the version is looked at: absent, a string, or of
/// the wrong type.
fn format_of<'a>(h: &'a Header, noun: &str) -> Result<&'a str, HeaderError> {
    match &h.format {
        None => Err(HeaderError::NoFormat { noun: noun.to_string() }),
        Some(Keyed::Str(s)) => Ok(s),
        Some(_) => Err(not(noun, "`format` is not a string")),
    }
}

/// The `version` key: absent, a whole number a `u32` holds, or of the wrong type.
fn version_of(h: &Header, noun: &str) -> Result<u32, HeaderError> {
    let v = match &h.version {
        None => return Err(HeaderError::NoVersion { noun: noun.to_string() }),
        Some(Keyed::Int(v)) => u32::try_from(*v).ok(),
        Some(_) => None,
    };
    v.ok_or_else(|| not(noun, "`version` is not a whole number from 0 to 4294967295"))
}

/// The stamp of a file's text, whatever its format: both keys present and of their type, the
/// format first, nothing else read. For a reader that knows more than one version and decides
/// what to do with each.
pub fn stamp(text: &str, noun: &str) -> Result<Stamp, HeaderError> {
    let h = header(text, noun)?;
    let format = format_of(&h, noun)?.to_string();
    let version = version_of(&h, noun)?;
    Ok(Stamp { format, version })
}

/// Check a file's header against the format and the one version a reader knows, format first:
/// no `format`, another format, no `version`, another version, each refused by name; the format
/// is decided whole before the version is looked at.
pub fn check(text: &str, noun: &str, format: &str, version: u32) -> Result<(), HeaderError> {
    let h = header(text, noun)?;
    let found = format_of(&h, noun)?;
    if found != format {
        return Err(HeaderError::WrongFormat { found: found.to_string(), expected: format.to_string() });
    }
    match version_of(&h, noun)? {
        v if v == version => Ok(()),
        v => Err(HeaderError::UnknownVersion { found: v, format: format.to_string(), known: version }),
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
        let refused = |text: &str| check(text, "a thing file", "veredas-thing", 1).unwrap_err().to_string();
        assert_eq!(refused("version = 1\n"), "no `format`: a thing file states `format` and `version`");
        assert_eq!(refused("format = \"veredas-other\"\n"), "`format` is `veredas-other`, not `veredas-thing`");
        assert_eq!(refused("format = \"veredas-thing\"\n"), "no `version`: a thing file states `format` and `version`");
        assert_eq!(refused("format = \"veredas-thing\"\nversion = 2\n"), "version 2 of `veredas-thing` is not known here (this reader knows 1)");
        // Both keys missing, or neither at the top: the format first.
        assert_eq!(refused("body = 1\n"), "no `format`: a thing file states `format` and `version`");
        assert_eq!(refused("[meta]\nformat = \"veredas-thing\"\nversion = 1\n"), "no `format`: a thing file states `format` and `version`");
        // A stamp of the wrong type is no stamp, named by its key with the reader's noun; the
        // format is decided whole before the version is looked at, so another format is refused
        // for its format whatever its version says, an integer past 64 bits included.
        assert_eq!(refused("format = \"veredas-other\"\nversion = \"1\"\n"), "`format` is `veredas-other`, not `veredas-thing`");
        assert_eq!(refused("format = \"veredas-other\"\nversion = 18446744073709551615\n"), "`format` is `veredas-other`, not `veredas-thing`");
        assert_eq!(refused("format = \"veredas-other\"\nversion = 18446744073709551616\n"), "`format` is `veredas-other`, not `veredas-thing`");
        assert_eq!(refused("format = 1\nversion = 1\n"), "not a thing file: `format` is not a string");
        assert_eq!(refused("format = 18446744073709551616\nversion = 1\n"), "not a thing file: `format` is not a string");
        let wide = ["\"1\"", "1.5", "-1", "4294967296", "18446744073709551615", "18446744073709551616", "-9223372036854775809", "true", "[1]", "{ v = 1 }", "1979-05-27", "nan"];
        for v in wide {
            assert_eq!(
                refused(&format!("format = \"veredas-thing\"\nversion = {v}\n")),
                "not a thing file: `version` is not a whole number from 0 to 4294967295",
                "version = {v}"
            );
        }
        assert_eq!(check("format = \"veredas-thing\"\nversion = 4294967295\n", "a thing file", "veredas-thing", 4294967295), Ok(()));
        // The parser reads an integer up to 128 bits and hands it over typed; past that it is
        // the parser's own refusal, with its reason, before any key, as any text it cannot read.
        let widest = "340282366920938463463374607431768211455";
        assert_eq!(refused(&format!("format = \"veredas-other\"\nversion = {widest}\n")), "`format` is `veredas-other`, not `veredas-thing`");
        assert_eq!(refused(&format!("format = \"veredas-thing\"\nversion = {widest}\n")), "not a thing file: `version` is not a whole number from 0 to 4294967295");
        let e = refused("format = \"veredas-other\"\nversion = 340282366920938463463374607431768211456\n");
        assert!(e.starts_with("not a thing file: ") && e.contains("overflow") && !e.contains("`format`"), "{e}");
        // A good header before a body that is not TOML: the parser's reason, before any key; a
        // body value the stamp never looks at is not judged here (an integer past `i64` is the
        // reader's own to refuse or to hold).
        let e = refused("format = \"veredas-thing\"\nversion = 1\nbody = [\n");
        assert!(e.starts_with("not a thing file: ") && !e.contains("`format`") && !e.contains("`version`"), "{e}");
        assert_eq!(check("format = \"veredas-thing\"\nversion = 1\nat = 18446744073709551615\n", "a thing file", "veredas-thing", 1), Ok(()));
        // The stamp alone reads any format, so a migrating reader can decide; both keys are
        // still required, the format first, a wrong type refused the same way, a version of 0
        // read as it stands for the reader to decide.
        assert_eq!(stamp("format = \"veredas-other\"\nversion = 7\n", "a file").unwrap().version, 7);
        assert_eq!(stamp("format = \"veredas-other\"\nversion = 0\n", "a file").unwrap().version, 0);
        assert_eq!(stamp("format = \"veredas-other\"\n", "a file").unwrap_err().to_string(), "no `version`: a file states `format` and `version`");
        assert_eq!(stamp("version = 1\n", "a file").unwrap_err().to_string(), "no `format`: a file states `format` and `version`");
        assert_eq!(stamp("", "a file").unwrap_err().to_string(), "no `format`: a file states `format` and `version`");
        assert_eq!(stamp("format = 1\nversion = 1\n", "a file").unwrap_err().to_string(), "not a file: `format` is not a string");
        assert_eq!(stamp("format = \"x\"\nversion = -1\n", "a file").unwrap_err().to_string(), "not a file: `version` is not a whole number from 0 to 4294967295");
        assert_eq!(stamp("format = \"x\"\nversion = 18446744073709551616\n", "a file").unwrap_err().to_string(), "not a file: `version` is not a whole number from 0 to 4294967295");
    }
}
