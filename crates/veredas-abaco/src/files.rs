//! Writing a file so that a reader never sees half of it, and knowing a source has not moved.
//!
//! [`atomic_write`] writes beside the destination and renames over it, the version it replaces kept
//! as `<name>.bak`; [`contents`] reads what a source holds now, so a writer that read it before can
//! tell it was not changed underneath (the patcher compares the two before its rename).

use std::path::Path;

use std::sync::atomic::{AtomicU64, Ordering};

/// The number of the next name of the process's own beside a destination
/// (`.veredas-<process>-<n>.tmp`).
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name of the process's own in `dir`, not grown from the destination's, so a long name (up to
/// the file system's limit less the `.bak` four) is never too long for it.
fn own_name(dir: &Path) -> std::path::PathBuf {
    dir.join(format!(".veredas-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(test)]
thread_local! {
    /// A test's failure of the second name, a copy cut short.
    static SECOND_NAME_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A test's failure of the last rename, the new file onto the destination.
    static LAST_RENAME_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A test's file system with no hard links.
    static NO_HARD_LINKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A hard link, as the file system (or a test's) makes one.
fn hard_link(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if NO_HARD_LINKS.with(std::cell::Cell::get) || SECOND_NAME_FAILS.with(std::cell::Cell::get) {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no hard links here"));
    }
    std::fs::hard_link(from, to)
}

/// `path`'s file given a second name of the process's own in `dir`, made new: a hard link, a copy
/// into a file made new where the file system has none. A name something already stands at, a link
/// among them, is passed over, never followed (refute round 20: the copy opened a link planted
/// there and wrote the replaced file through it). A copy that fails removes its own file.
fn second_name(path: &Path, dir: &Path) -> std::io::Result<std::path::PathBuf> {
    loop {
        let keep = own_name(dir);
        match hard_link(path, &keep) {
            Ok(()) => return Ok(keep),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {}
        }
        let mut to = match std::fs::OpenOptions::new().write(true).create_new(true).open(&keep) {
            Ok(to) => to,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let copied = (|| {
            #[cfg(test)]
            if SECOND_NAME_FAILS.with(std::cell::Cell::get) {
                std::io::Write::write_all(&mut to, b"part of it")?;
                return Err(std::io::Error::other("a copy cut short"));
            }
            let mut from = std::fs::File::open(path)?;
            std::io::copy(&mut from, &mut to)?;
            to.sync_all()?;
            std::fs::set_permissions(&keep, std::fs::metadata(path)?.permissions())
        })();
        return match copied {
            Ok(()) => Ok(keep),
            Err(e) => {
                drop(to);
                let _ = std::fs::remove_file(&keep);
                Err(e)
            }
        };
    }
}

/// An earlier `.bak` kept aside under a name of the process's own until the write is done: a hard
/// link, made new, where the file system has one (the `.bak` staying where it is), else the `.bak`
/// renamed there, a name something stands at passed over.
fn earlier_aside(bak: &Path, dir: &Path) -> std::io::Result<std::path::PathBuf> {
    loop {
        let aside = own_name(dir);
        match hard_link(bak, &aside) {
            Ok(()) => return Ok(aside),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {}
        }
        if aside.symlink_metadata().is_ok() {
            continue;
        }
        std::fs::rename(bak, &aside)?;
        return Ok(aside);
    }
}

/// Writes `path` through `write`, into a temporary file of its own beside it
/// (`.veredas-<process>-<n>.tmp`, made new, so no file or link already there is followed or
/// overwritten), then renames it into place; an existing `path` becomes `<name>.bak` (a previous
/// `.bak` replaced). The names are the file name as the system gives it, whatever its encoding. Any
/// failure (the write, the `.bak` that cannot be replaced, a rename) leaves `path` and an earlier
/// `.bak` as they were and removes the files of its own. Two writes of one destination at once are
/// not serialised (a limit).
pub fn atomic_write(path: &Path, write: impl FnOnce(std::fs::File) -> Result<(), String>) -> Result<(), String> {
    let file_name = path.file_name().ok_or_else(|| format!("{}: not a file path", path.display()))?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let bak = {
        let mut name = file_name.to_os_string();
        name.push(".bak");
        dir.join(name)
    };
    let (tmp, file) = loop {
        let tmp = own_name(dir);
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
        if path.symlink_metadata().is_err() {
            return last_rename(&tmp, path);
        }
        // the second name made beside under a name of its own, then given `.bak`'s in one rename:
        // a link or a copy that fails partway leaves an earlier `.bak` as it was and nothing of its
        // own (refute round 19: a copy cut short left a truncated `.bak`)
        let keep = second_name(path, dir).map_err(|e| format!("{}: {e}", bak.display()))?;
        // an earlier `.bak` kept aside until the new file has the destination's name, put back if
        // it does not (refute round 20: a last rename that failed removed the `.bak` and an earlier
        // one with it); a folder there is no `.bak` to replace, and the rename onto it fails
        let earlier = if bak.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
            match earlier_aside(&bak, dir) {
                Ok(aside) => Some(aside),
                Err(e) => {
                    let _ = std::fs::remove_file(&keep);
                    return Err(format!("{}: {e}", bak.display()));
                }
            }
        } else {
            None
        };
        let put_back = |earlier: &Option<std::path::PathBuf>| match earlier {
            Some(aside) => {
                let _ = std::fs::rename(aside, &bak);
            }
            None => {
                let _ = std::fs::remove_file(&bak);
            }
        };
        if let Err(e) = std::fs::rename(&keep, &bak) {
            let _ = std::fs::remove_file(&keep);
            if let Some(aside) = &earlier
                && bak.symlink_metadata().is_err()
            {
                let _ = std::fs::rename(aside, &bak);
            } else if let Some(aside) = &earlier {
                let _ = std::fs::remove_file(aside);
            }
            return Err(format!("{}: {e}", bak.display()));
        }
        match last_rename(&tmp, path) {
            Ok(()) => {
                if let Some(aside) = &earlier {
                    let _ = std::fs::remove_file(aside);
                }
                Ok(())
            }
            Err(e) => {
                // the destination as it was, an earlier `.bak` with it, and nothing beside them
                put_back(&earlier);
                Err(e)
            }
        }
    });
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed
}

/// The new file renamed onto the destination.
fn last_rename(tmp: &Path, path: &Path) -> Result<(), String> {
    #[cfg(test)]
    if LAST_RENAME_FAILS.with(std::cell::Cell::get) {
        return Err(format!("{}: a rename refused", path.display()));
    }
    std::fs::rename(tmp, path).map_err(|e| format!("{}: {e}", path.display()))
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

    fn left(dir: &std::path::Path, planted: &[String]) -> Vec<String> {
        let mut left: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !planted.contains(n))
            .collect();
        left.sort();
        left
    }

    fn write(dest: &std::path::Path, bytes: &'static [u8]) -> Result<(), String> {
        super::atomic_write(dest, |mut f| std::io::Write::write_all(&mut f, bytes).map_err(|e| e.to_string()))
    }

    #[cfg(unix)]
    #[test]
    fn a_link_standing_at_a_name_of_its_own_is_passed_over_never_followed() {
        // refute round 20: the second name's copy opened a link planted at its name, writing the
        // replaced file into another folder, the write reported done and `.bak` that link
        for no_hard_links in [false, true] {
            let (dir, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let victim = other.path().join("victim.txt");
            std::fs::write(&victim, b"untouched").unwrap();
            let dest = dir.path().join("d.abaco");
            write(&dest, b"first").unwrap();
            write(&dest, b"second").unwrap();
            // links at every name the process takes next, the temporary file's, the second name's
            // and the earlier `.bak`'s
            let from = super::NEXT.load(std::sync::atomic::Ordering::Relaxed);
            let planted: Vec<String> = (from..from + 400).map(|n| format!(".veredas-{}-{n}.tmp", std::process::id())).collect();
            for name in &planted {
                std::os::unix::fs::symlink(&victim, dir.path().join(name)).unwrap();
            }
            super::NO_HARD_LINKS.with(|f| f.set(no_hard_links));
            let got = write(&dest, b"third");
            super::NO_HARD_LINKS.with(|f| f.set(false));
            got.unwrap();
            assert_eq!(std::fs::read(&victim).unwrap(), b"untouched", "{no_hard_links}");
            assert_eq!(std::fs::read(&dest).unwrap(), b"third");
            let bak = dir.path().join("d.abaco.bak");
            assert!(bak.symlink_metadata().unwrap().is_file(), "{no_hard_links}");
            assert_eq!(std::fs::read(&bak).unwrap(), b"second");
            assert_eq!(left(dir.path(), &planted), ["d.abaco", "d.abaco.bak"], "{no_hard_links}");
            for name in &planted {
                assert!(dir.path().join(name).symlink_metadata().unwrap().file_type().is_symlink(), "{name}");
            }
        }
    }

    #[test]
    fn a_last_rename_that_fails_leaves_the_destination_and_an_earlier_bak_as_they_were() {
        // refute round 20: the `.bak` had replaced an earlier one before the last rename, whose
        // failure then removed it
        for no_hard_links in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join("d.abaco");
            std::fs::write(&dest, b"the destination").unwrap();
            std::fs::write(dir.path().join("d.abaco.bak"), b"an earlier one").unwrap();
            super::NO_HARD_LINKS.with(|f| f.set(no_hard_links));
            super::LAST_RENAME_FAILS.with(|f| f.set(true));
            let got = write(&dest, b"new");
            super::LAST_RENAME_FAILS.with(|f| f.set(false));
            super::NO_HARD_LINKS.with(|f| f.set(false));
            assert!(got.is_err_and(|e| e.contains("a rename refused")), "{no_hard_links}");
            assert_eq!(std::fs::read(&dest).unwrap(), b"the destination");
            assert_eq!(std::fs::read(dir.path().join("d.abaco.bak")).unwrap(), b"an earlier one", "{no_hard_links}");
            assert_eq!(left(dir.path(), &[]), ["d.abaco", "d.abaco.bak"], "{no_hard_links}");
            // with no earlier `.bak`, none left
            std::fs::remove_file(dir.path().join("d.abaco.bak")).unwrap();
            super::LAST_RENAME_FAILS.with(|f| f.set(true));
            let got = write(&dest, b"new");
            super::LAST_RENAME_FAILS.with(|f| f.set(false));
            assert!(got.is_err());
            assert_eq!(left(dir.path(), &[]), ["d.abaco"]);
            // and once it succeeds, the replaced file is the `.bak`
            write(&dest, b"new").unwrap();
            assert_eq!(std::fs::read(dir.path().join("d.abaco.bak")).unwrap(), b"the destination");
            assert_eq!(left(dir.path(), &[]), ["d.abaco", "d.abaco.bak"]);
        }
    }
}
