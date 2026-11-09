//! The page, compiled in from the committed build (`web/dist/`), and the hash that ties the build
//! to its sources.
//!
//! The build writes `dist/source.hash`: an FNV-1a 64 hash over the page's sources (below), each
//! file's path relative to `web/` with `/` separators, a NUL, its text with CRLF read as LF, a NUL,
//! in the order of the paths. The component test recomputes it from the tree, so a source edited
//! without a rebuild is refused on every machine, Node or not.

use std::io;
use std::path::Path;

pub const INDEX_HTML: &str = include_str!("../web/dist/index.html");
pub const APP_JS: &str = include_str!("../web/dist/assets/app.js");
pub const APP_CSS: &str = include_str!("../web/dist/assets/app.css");
/// The hash the build recorded of the sources it was built from.
pub const BUILT_FROM: &str = include_str!("../web/dist/source.hash");

/// The files the page is built from, besides everything under `src/`.
pub const SOURCE_FILES: [&str; 5] = ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"];

/// FNV-1a, 64 bits.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>, mut h: u64) -> u64 {
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let r = format!("{rel}/{name}");
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), &r, out)?;
        } else {
            out.push(r);
        }
    }
    Ok(())
}

/// The sources of the page under `web`, as relative paths, sorted.
pub fn sources(web: &Path) -> io::Result<Vec<String>> {
    let mut out: Vec<String> = SOURCE_FILES.iter().map(|s| s.to_string()).collect();
    walk(&web.join("src"), "src", &mut out)?;
    out.sort();
    Ok(out)
}

/// The hash of the page's sources under `web`, as the build records it: 16 lowercase hex digits.
pub fn source_hash(web: &Path) -> io::Result<String> {
    let mut h = FNV_OFFSET;
    for rel in sources(web)? {
        let bytes = std::fs::read(web.join(&rel))?;
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        h = fnv1a(rel.bytes(), h);
        h = fnv1a([0u8], h);
        h = fnv1a(text.bytes(), h);
        h = fnv1a([0u8], h);
    }
    Ok(format!("{h:016x}"))
}
