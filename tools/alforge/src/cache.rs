//! The cache outside the repository (`~/.cache/veredas/alforge/`, or `ALFORGE_CACHE`): the base
//! image, downloaded once and checked against its pinned SHA-512 on every use, and the builder
//! machine's disk, built once per toolchain. Large files are never committed (the plan's rule).

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256, Sha512};

use crate::machine::Recipe;
use crate::vm::{self, VmError};

/// The cache folder, made if absent.
pub fn dir() -> Result<PathBuf, std::io::Error> {
    let d = match std::env::var_os("ALFORGE_CACHE") {
        Some(d) => PathBuf::from(d),
        None => {
            let home = std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is not set"))?;
            PathBuf::from(home).join(".cache/veredas/alforge")
        }
    };
    std::fs::create_dir_all(&d)?;
    std::path::absolute(&d)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A file's SHA-512, read in pieces.
pub fn sha512(p: &Path) -> Result<String, std::io::Error> {
    let mut h = Sha512::new();
    let mut f = std::fs::File::open(p)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

/// A file's SHA-256, read in pieces.
pub fn sha256(p: &Path) -> Result<String, std::io::Error> {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(p)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

/// A text's SHA-256.
pub fn sha256_text(t: &str) -> String {
    hex(&Sha256::digest(t.as_bytes()))
}

/// The base image, downloaded once from its pinned address and checked against its pinned SHA-512
/// every time it is handed out; a file that does not match is refused, never used.
pub fn base_image(r: &Recipe) -> Result<PathBuf, VmError> {
    base_image_in(&dir()?, r)
}

/// [`base_image`] with its cache at `cache`: the base downloaded there once, and refused, both
/// digests named, unless it matches the recipe's pinned SHA-512.
pub fn base_image_in(cache: &Path, r: &Recipe) -> Result<PathBuf, VmError> {
    let path = cache.join(r.base_name());
    if !path.exists() {
        let part = path.with_extension("part");
        eprintln!("alforge: downloading {} once into {}", r.base.url, path.display());
        vm::run("curl", &["-sSfL", "--retry", "3", "-o", &part.display().to_string(), &r.base.url], None)?;
        std::fs::rename(&part, &path)?;
    }
    let got = sha512(&path)?;
    if got != r.base.sha512 {
        return Err(VmError::Failed {
            program: "the base image's check".into(),
            status: "refused".into(),
            stderr: format!("{} has SHA-512 {got}, the recipe pins {}", path.display(), r.base.sha512),
        });
    }
    Ok(path)
}
