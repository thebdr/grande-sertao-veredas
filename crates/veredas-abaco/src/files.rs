//! Writing a file so that a reader never sees half of it, and knowing a source has not moved.
//!
//! [`atomic_write`] writes beside the destination and renames over it, the version it replaces kept
//! as `<name>.bak`; [`contents`] reads what a source holds now, so a writer that read it before can
//! tell it was not changed underneath (the patcher compares the two before its rename).

use std::path::Path;

/// Writes `path` through `write`, into a temporary file of its own beside it
/// (`.veredas-<process>-<n>.tmp`, made new, so no file or link already there is followed or
/// overwritten), then renames it into place; an existing `path` becomes `<name>.bak` (a previous
/// `.bak` replaced). The names are the file name as the system gives it, whatever its encoding. Any
/// failure (the write, the `.bak` that cannot be replaced, a rename) leaves `path` as it was and
/// removes the temporary file. Two writes of one destination at once are not serialised (a limit).
pub fn atomic_write(path: &Path, write: impl FnOnce(std::fs::File) -> Result<(), String>) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let file_name = path.file_name().ok_or_else(|| format!("{}: not a file path", path.display()))?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let beside = |suffix: &str| {
        let mut name = file_name.to_os_string();
        name.push(suffix);
        dir.join(name)
    };
    let bak = beside(".bak");
    let (tmp, file) = loop {
        // a name of its own, not grown from the destination's, so a long name (up to the file
        // system's limit less the `.bak` four) is never too long for it
        let tmp = dir.join(format!(".veredas-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", tmp.display())),
        }
    };
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
