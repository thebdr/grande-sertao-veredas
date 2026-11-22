//! The tester machine's recipe data: `veredas-machine`, version 1
//! (`tools/entrega/maquina/maquina.toml`). The base image and the libraries it offers, the builder
//! machine and the tester machine; every choice the architect rules is a key here, not code.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use veredas_core::schema::{self, HeaderError};

pub const FORMAT: &str = "veredas-machine";
pub const VERSION: u32 = 1;
const NOUN: &str = "a machine recipe";

#[derive(Debug, Error)]
pub enum RecipeError {
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("{NOUN}: {0}")]
    Invalid(String),
    #[error("{}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    #[allow(dead_code)]
    format: toml::Value,
    #[allow(dead_code)]
    version: toml::Value,
    pub base: Base,
    pub builder: Builder,
    pub machine: Machine,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base {
    pub url: String,
    pub sha512: String,
    pub glibc: String,
    pub glibcxx: String,
    pub cxxabi: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Builder {
    pub cpus: u32,
    pub memory_mb: u32,
    pub disk_gb: u32,
    pub packages: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub cpus: u32,
    pub memory_mb: u32,
    pub disk_gb: u32,
    pub hardware_version: u32,
    pub user: String,
    pub password: String,
    pub packages: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
    pub supervisao_port: u16,
}

fn is_package(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+-.".contains(&b))
}

impl Recipe {
    pub fn load(text: &str) -> Result<Recipe, RecipeError> {
        schema::check(text, NOUN, FORMAT, VERSION)?;
        let r: Recipe = toml::from_str(text).map_err(|e| RecipeError::Invalid(e.message().to_string()))?;
        if !r.base.url.starts_with("https://") {
            return Err(RecipeError::Invalid(format!("`base.url` is `{}`, not an https address", r.base.url)));
        }
        if r.base.sha512.len() != 128 || !r.base.sha512.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(RecipeError::Invalid("`base.sha512` is not 128 lowercase hexadecimal characters".into()));
        }
        for (key, v) in [("base.glibc", &r.base.glibc), ("base.glibcxx", &r.base.glibcxx), ("base.cxxabi", &r.base.cxxabi)] {
            if !v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
                return Err(RecipeError::Invalid(format!("`{key}` is `{v}`, not a version of dotted digits")));
            }
        }
        for p in r.builder.packages.iter().chain(&r.machine.packages).chain(&r.machine.remove) {
            if !is_package(p) {
                return Err(RecipeError::Invalid(format!("`{p}` is not a package's name")));
            }
        }
        if r.machine.user.is_empty() || !r.machine.user.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
            return Err(RecipeError::Invalid(format!("`machine.user` is `{}`, not a lowercase login", r.machine.user)));
        }
        if r.machine.password.is_empty() || r.machine.password.contains(['\n', '"', '\'', '\\', ':']) {
            return Err(RecipeError::Invalid("`machine.password` is empty or holds a quote, a backslash, a colon or a line break".into()));
        }
        Ok(r)
    }

    pub fn load_file(path: &Path) -> Result<Recipe, RecipeError> {
        let text = std::fs::read_to_string(path).map_err(|source| RecipeError::Read { path: path.to_path_buf(), source })?;
        Recipe::load(&text)
    }

    /// The base image's file name, the last part of its address.
    pub fn base_name(&self) -> &str {
        self.base.url.rsplit('/').next().unwrap_or("base.qcow2")
    }
}
