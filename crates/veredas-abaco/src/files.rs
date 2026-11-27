//! Writing a file so that a reader never sees half of it, and knowing a source has not moved.
//!
//! [`atomic_write`] writes beside the destination and renames over it, the version it replaces kept
//! as `<name>.bak`; [`contents`] reads what a source holds now, so a writer that read it before can
//! tell it was not changed underneath (the patcher compares the two before its rename).

use std::path::Path;

#[cfg(test)]
thread_local! {
    /// A test's failure of the second name, a copy cut short.
    static SECOND_NAME_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `path`'s file given a second name, `keep`: a hard link, a copy where the file system has none.
fn second_name(path: &Path, keep: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if SECOND_NAME_FAILS.with(std::cell::Cell::get) {
        std::fs::write(keep, b"part of it")?;
        return Err(std::io::Error::other("a copy cut short"));
    }
    std::fs::hard_link(path, keep).or_else(|_| std::fs::copy(path, keep).map(|_| ()))
}

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
        // the replaced file kept as `.bak` by a second name (a hard link, a copy where the file
        // system has none) before the new one takes the destination's name in one rename, which
        // replaces it: the destination is never missing, a write killed partway leaving it whole,
        // the old or the new (refute round 18: renamed to `.bak` first, a kill between the two
        // renames left no destination)
        let existed = path.symlink_metadata().is_ok();
        if existed {
            // the second name made beside under a name of its own, then given `.bak`'s in one
            // rename: a link or a copy that fails partway leaves an earlier `.bak` as it was and
            // nothing of its own (refute round 19: a copy cut short left a truncated `.bak`)
            let keep = dir.join(format!(".veredas-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let kept = second_name(path, &keep).and_then(|()| std::fs::rename(&keep, &bak));
            if let Err(e) = kept {
                let _ = std::fs::remove_file(&keep);
                return Err(format!("{}: {e}", bak.display()));
            }
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            // the destination as it was, and nothing beside it
            if existed {
                let _ = std::fs::remove_file(&bak);
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

#[cfg(test)]
mod second_name_tests {
    #[test]
    fn a_second_name_that_fails_leaves_the_destination_and_an_earlier_bak_as_they_were() {
        // refute round 19: the copy fallback cut short left a truncated `.bak`, an earlier one gone
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("d.abaco");
        std::fs::write(&dest, b"the destination").unwrap();
        std::fs::write(dir.path().join("d.abaco.bak"), b"an earlier one").unwrap();
        super::SECOND_NAME_FAILS.with(|f| f.set(true));
        let got = super::atomic_write(&dest, |mut f| std::io::Write::write_all(&mut f, b"new").map_err(|e| e.to_string()));
        super::SECOND_NAME_FAILS.with(|f| f.set(false));
        assert!(got.is_err_and(|e| e.contains("a copy cut short")));
        assert_eq!(std::fs::read(&dest).unwrap(), b"the destination");
        assert_eq!(std::fs::read(dir.path().join("d.abaco.bak")).unwrap(), b"an earlier one");
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["d.abaco", "d.abaco.bak"]);
        // and once it succeeds, the replaced file is the `.bak`
        super::atomic_write(&dest, |mut f| std::io::Write::write_all(&mut f, b"new").map_err(|e| e.to_string())).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.path().join("d.abaco.bak")).unwrap(), b"the destination");
    }
}
