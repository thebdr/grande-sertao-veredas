//! Writing a file so that a reader never sees half of it, and knowing a source has not moved.
//!
//! [`atomic_write`] writes beside the destination and renames over it, the version it replaces kept
//! as `<name>.bak`; [`contents`] reads what a source holds now, so a writer that read it before can
//! tell it was not changed underneath (the patcher compares the two before its rename).

use std::path::Path;

/// Writes `path` through `write`, into `<name>.tmp` beside it, then renames it into place; an
/// existing `path` becomes `<name>.bak` (a previous `.bak` replaced). Any failure (the write, the
/// `.bak` that cannot be replaced, a rename) leaves `path` as it was and removes the temporary
/// file.
pub fn atomic_write(path: &Path, write: impl FnOnce(std::fs::File) -> Result<(), String>) -> Result<(), String> {
    let file_name =
        path.file_name().ok_or_else(|| format!("{}: not a file path", path.display()))?.to_string_lossy().to_string();
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let tmp = dir.join(format!("{file_name}.tmp"));
    let bak = dir.join(format!("{file_name}.bak"));
    let file = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let placed = write(file).and_then(|()| {
        let existed = path.exists();
        if existed {
            if bak.exists() {
                std::fs::remove_file(&bak).map_err(|e| format!("{}: {e}", bak.display()))?;
            }
            std::fs::rename(path, &bak).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            // the destination back where it was
            if existed {
                let _ = std::fs::rename(&bak, path);
            }
            format!("{}: {e}", path.display())
        })
    });
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed
}

/// What the file at `path` holds now, byte for byte: read before and again before a write, the two
/// equal means it did not move in between.
pub fn contents(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}
