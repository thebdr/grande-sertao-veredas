//! The page, compiled in from the committed build (`web/dist/`), and the hashes that tie the build
//! to its sources and to itself.
//!
//! The build writes two hashes beside the page, each FNV-1a 64 over a set of files: each file's
//! path relative to its folder with `/` separators, a NUL, its bytes with every CR LF pair read as
//! LF, a NUL, in the order of the paths (bytes, so a file that is not text counts byte for byte).
//! `dist/source.hash` is over the page's sources under `web/` (below); `dist/output.hash` is over
//! the page as written under `web/dist/`. The component test recomputes
//! the first from the tree and the second from the page compiled in, so a source edited without a
//! rebuild, or a page edited by hand, is refused on every machine, Node or not.

use std::io;
use std::path::Path;

pub const INDEX_HTML: &str = include_str!("../web/dist/index.html");
pub const APP_JS: &str = include_str!("../web/dist/assets/app.js");
pub const APP_CSS: &str = include_str!("../web/dist/assets/app.css");
/// The hash the build recorded of the sources it was built from.
pub const BUILT_FROM: &str = include_str!("../web/dist/source.hash");
/// The hash the build recorded of the page it wrote.
pub const BUILT_AS: &str = include_str!("../web/dist/output.hash");

/// The files the page is built from, besides everything under `src/`.
pub const SOURCE_FILES: [&str; 5] = ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"];

/// The page as the build writes it under `dist/`, every file the host serves, in path order.
pub const OUTPUT_FILES: [&str; 3] = ["assets/app.css", "assets/app.js", "index.html"];

/// FNV-1a, 64 bits.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>, mut h: u64) -> u64 {
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// The hash of named byte strings, as the build records it: 16 lowercase hex digits, every CR LF
/// pair read as LF. They are taken in the order of their names.
pub fn hash_bytes<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut files: Vec<(&str, &[u8])> = files.into_iter().collect();
    files.sort();
    let mut h = FNV_OFFSET;
    for (name, bytes) in files {
        h = fnv1a(name.bytes(), h);
        h = fnv1a([0u8], h);
        let lf = bytes.iter().enumerate().filter(|(i, b)| !(**b == b'\r' && bytes.get(i + 1) == Some(&b'\n'))).map(|(_, b)| *b);
        h = fnv1a(lf, h);
        h = fnv1a([0u8], h);
    }
    format!("{h:016x}")
}

/// The same over named texts.
pub fn hash_texts<'a>(texts: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    hash_bytes(texts.into_iter().map(|(n, t)| (n, t.as_bytes())))
}

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

/// The hash of the page's sources under `web`, as the build records it in `dist/source.hash`.
pub fn source_hash(web: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    for rel in sources(web)? {
        let bytes = std::fs::read(web.join(&rel))?;
        files.push((rel, bytes));
    }
    Ok(hash_bytes(files.iter().map(|(r, b)| (r.as_str(), b.as_slice()))))
}

/// The hash of the page compiled in, as the build records it in `dist/output.hash`.
pub fn output_hash() -> String {
    hash_texts(OUTPUT_FILES.into_iter().zip([APP_CSS, APP_JS, INDEX_HTML]))
}
