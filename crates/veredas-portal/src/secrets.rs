//! The project's secrets file (`docs/project-schema.md`, "The secrets file"; `docs/questions.md`
//! Q15, decided 2026-11-09): an encrypted TOML file beside the project, committed with it, every
//! entry one value sealed with ChaCha20-Poly1305 under the project's key, which an environment
//! variable the project file names holds as 64 hexadecimal characters. The session reads the key
//! when it starts, opens the file in memory, and refuses a source whose entry is missing by the
//! entry's name, never a value; a wrong key refuses the file by the variable's name. The tool
//! that writes an entry (`veredas-portal --secret`) seals a value read from stdin and prints
//! nothing of it. No value of this module is ever shown by a `Debug` or an error.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use veredas_core::schema::{self, HeaderError};

pub const FORMAT: &str = "veredas-secrets";
pub const VERSION: u32 = 1;
const NOUN: &str = "a secrets file";
const NONCE_LEN: usize = 12;

/// Why a secrets file, its key or an entry was refused. No variant carries a value or a key.
#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("not a secrets file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `veredas-secrets`")]
    WrongFormat(String),
    #[error("version {0} of `veredas-secrets` is not known here (this reader knows 1)")]
    UnknownVersion(u32),
    #[error("{0}")]
    Invalid(String),
    #[error("the secrets file {path}: cannot read: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("the secrets file {path}: cannot write: {source}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("the variable `{var}` is not set: it holds the project's key")]
    KeyUnset { var: String },
    #[error("the variable `{var}` does not hold a key (64 hexadecimal characters)")]
    KeyMalformed { var: String },
    #[error("the secrets file {path} does not open with the key `{var}` holds (entry `{entry}`)")]
    WrongKey { path: PathBuf, var: String, entry: String },
    #[error("the entry `{entry}` is not in the secrets file {path}")]
    Missing { entry: String, path: PathBuf },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    format: String,
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

/// A 256-bit key. It shows in no `Debug`; its bytes are overwritten when it drops.
pub struct Key256([u8; 32]);

impl fmt::Debug for Key256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key256(..)")
    }
}

impl Drop for Key256 {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY-free: a plain volatile write so the overwrite is not optimised away.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl Key256 {
    /// A fresh key from the operating system's randomness.
    pub fn fresh() -> Result<Key256, SecretsError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| SecretsError::Invalid(format!("no randomness for a key: {e}")))?;
        Ok(Key256(bytes))
    }

    /// The key the variable `var` holds, read through `env` (the process's environment, or a
    /// test's closure): 64 hexadecimal characters, in either case.
    pub fn from_env(var: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<Key256, SecretsError> {
        let text = env(var).ok_or_else(|| SecretsError::KeyUnset { var: var.to_string() })?;
        let text = text.trim();
        if text.len() != 64 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SecretsError::KeyMalformed { var: var.to_string() });
        }
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| SecretsError::KeyMalformed { var: var.to_string() })?;
        }
        Ok(Key256(bytes))
    }

    /// The key as 64 hexadecimal characters, for the operator to set in the environment.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new((&self.0).into())
    }
}

/// The secrets file as it stands on disk: sealed entries, read and written whole.
pub struct SecretsFile {
    pub path: PathBuf,
    entries: BTreeMap<String, String>,
}

impl fmt::Debug for SecretsFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretsFile({} entries of {})", self.entries.len(), self.path.display())
    }
}

impl SecretsFile {
    /// Read the file at `path`, its stamp checked first (C-045).
    pub fn load(path: &Path) -> Result<SecretsFile, SecretsError> {
        let text = std::fs::read_to_string(path).map_err(|source| SecretsError::Read { path: path.to_path_buf(), source })?;
        Self::parse(path, &text)
    }

    /// The file at `path`, or an empty one when none is there yet (for the tool that writes).
    pub fn load_or_empty(path: &Path) -> Result<SecretsFile, SecretsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SecretsFile { path: path.to_path_buf(), entries: BTreeMap::new() }),
            Err(source) => Err(SecretsError::Read { path: path.to_path_buf(), source }),
        }
    }

    fn parse(path: &Path, text: &str) -> Result<SecretsFile, SecretsError> {
        schema::check(text, NOUN, FORMAT, VERSION).map_err(|e| match e {
            HeaderError::NotToml { reason, .. } => SecretsError::NotToml(reason),
            HeaderError::WrongFormat { found, .. } => SecretsError::WrongFormat(found),
            HeaderError::UnknownVersion { found, .. } => SecretsError::UnknownVersion(found),
            other => SecretsError::Invalid(other.to_string()),
        })?;
        let file: FileV1 = toml::from_str(text).map_err(|e| SecretsError::NotToml(e.to_string()))?;
        for name in file.entries.keys() {
            if name.trim().is_empty() {
                return Err(SecretsError::Invalid("an entry has an empty name".into()));
            }
        }
        Ok(SecretsFile { path: path.to_path_buf(), entries: file.entries })
    }

    /// The entries' names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Seal `value` under `key` as the entry `name`, a fresh nonce each time, the name as the
    /// associated data (an entry copied under another name does not open).
    pub fn set(&mut self, key: &Key256, name: &str, value: &str) -> Result<(), SecretsError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SecretsError::Invalid("an entry needs a name".into()));
        }
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| SecretsError::Invalid(format!("no randomness for a nonce: {e}")))?;
        let sealed = key
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: value.as_bytes(), aad: name.as_bytes() })
            .map_err(|_| SecretsError::Invalid("the value could not be sealed".into()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&sealed);
        self.entries.insert(name.to_string(), base64::engine::general_purpose::STANDARD.encode(blob));
        Ok(())
    }

    /// Write the file whole, stamped.
    pub fn save(&self) -> Result<(), SecretsError> {
        let file = FileV1 { format: FORMAT.into(), version: VERSION, entries: self.entries.clone() };
        let body = toml::to_string_pretty(&file).map_err(|e| SecretsError::Invalid(format!("writing the secrets file: {e}")))?;
        let text = format!("# The project's secrets, sealed under the key the project file's `key_env` names; written by `veredas-portal --secret`.\n{body}");
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| SecretsError::Write { path: self.path.clone(), source })?;
        }
        std::fs::write(&self.path, text).map_err(|source| SecretsError::Write { path: self.path.clone(), source })
    }

    /// Open every entry under `key` (the variable `var` named it, for the refusal's text): the
    /// secrets in memory, or the first entry that does not open, by name.
    pub fn open(&self, key: &Key256, var: &str) -> Result<Secrets, SecretsError> {
        let cipher = key.cipher();
        let mut values = BTreeMap::new();
        for (name, blob) in &self.entries {
            let wrong = || SecretsError::WrongKey { path: self.path.clone(), var: var.to_string(), entry: name.clone() };
            let bytes = base64::engine::general_purpose::STANDARD.decode(blob.trim()).map_err(|_| wrong())?;
            if bytes.len() < NONCE_LEN {
                return Err(wrong());
            }
            let (nonce, sealed) = bytes.split_at(NONCE_LEN);
            let plain = cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: name.as_bytes() }).map_err(|_| wrong())?;
            let value = String::from_utf8(plain).map_err(|_| wrong())?;
            values.insert(name.clone(), value);
        }
        Ok(Secrets { path: self.path.clone(), values })
    }
}

/// The decrypted secrets, in memory only: a name answers its value, nothing lists the values.
pub struct Secrets {
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secrets({} entries of {})", self.values.len(), self.path.display())
    }
}

impl Secrets {
    /// No file: every lookup answers nothing.
    pub fn none() -> Secrets {
        Secrets { path: PathBuf::new(), values: BTreeMap::new() }
    }

    /// The value of an entry, when the file holds it.
    pub fn get(&self, entry: &str) -> Option<&str> {
        self.values.get(entry.trim()).map(String::as_str)
    }

    /// The entries' names, for a refusal that names what is there.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }
}

/// Where a project keeps its secrets: the file and the variable holding the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretsSource {
    pub file: PathBuf,
    pub key_env: String,
}

impl SecretsSource {
    /// Read the key from `env`, load the file and open it: the session's first step when the
    /// project names secrets. No value reaches any error.
    pub fn open(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<Secrets, SecretsError> {
        let key = Key256::from_env(&self.key_env, env)?;
        SecretsFile::load(&self.file)?.open(&key, &self.key_env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_sealed_opens_under_its_key_and_name_alone_and_nothing_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let key = Key256::fresh().unwrap();
        let hex = key.to_hex();
        assert_eq!(hex.len(), 64);
        let env = |var: &str| (var == "K").then(|| hex.clone());
        let read = Key256::from_env("K", &env).unwrap();
        let mut file = SecretsFile::load_or_empty(&path).unwrap();
        file.set(&key, "plc", "s3cret").unwrap();
        file.set(&key, "other", "x").unwrap();
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# The project's secrets") && text.contains("format = \"veredas-secrets\"") && text.contains("version = 1"), "{text}");
        assert!(!text.contains("s3cret"), "the value is not in the file");
        let opened = SecretsFile::load(&path).unwrap().open(&read, "K").unwrap();
        assert_eq!((opened.get("plc"), opened.get("other"), opened.get("nope")), (Some("s3cret"), Some("x"), None));
        assert_eq!(format!("{opened:?}"), format!("Secrets(2 entries of {})", path.display()));
        assert_eq!(format!("{key:?}"), "Key256(..)");
        // Another key, a copied entry under another name, a malformed or unset variable.
        let wrong = Key256::fresh().unwrap();
        let e = SecretsFile::load(&path).unwrap().open(&wrong, "K").unwrap_err().to_string();
        assert_eq!(e, format!("the secrets file {} does not open with the key `K` holds (entry `other`)", path.display()));
        assert!(!e.contains("s3cret"));
        let copied = text.replace("plc = ", "plc2 = ");
        std::fs::write(&path, copied).unwrap();
        let e = SecretsFile::load(&path).unwrap().open(&read, "K").unwrap_err().to_string();
        assert!(e.ends_with("(entry `plc2`)"), "{e}");
        assert_eq!(Key256::from_env("K", &|_| None).unwrap_err().to_string(), "the variable `K` is not set: it holds the project's key");
        assert_eq!(Key256::from_env("K", &|_| Some("abc".into())).unwrap_err().to_string(), "the variable `K` does not hold a key (64 hexadecimal characters)");
        // The stamp first, as every file of the suite.
        std::fs::write(&path, "format = \"veredas-other\"\nversion = 1\n").unwrap();
        assert_eq!(SecretsFile::load(&path).unwrap_err().to_string(), "`format` is `veredas-other`, not `veredas-secrets`");
        std::fs::write(&path, "format = \"veredas-secrets\"\nversion = 2\n").unwrap();
        assert_eq!(SecretsFile::load(&path).unwrap_err().to_string(), "version 2 of `veredas-secrets` is not known here (this reader knows 1)");
        std::fs::write(&path, "format = \"veredas-secrets\"\nversion = 1\nentrees = {}\n").unwrap();
        assert!(SecretsFile::load(&path).unwrap_err().to_string().contains("entrees"));
        let e = SecretsFile::load(&dir.path().join("none.toml")).unwrap_err().to_string();
        assert!(e.starts_with("the secrets file ") && e.contains("cannot read"), "{e}");
    }
}
