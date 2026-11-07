//! Local against remote paths. The directory-change notifications a watcher relies on are
//! unreliable over a network share, so a remote path is polled alone; a local path gets the
//! watcher on top of polling.
//!
//! Remote means a UNC path (`\\server\share`, `\\?\UNC\...`) or a drive letter mapped to a
//! network drive.

use std::path::{Component, Path, Prefix};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Local,
    /// A UNC path or a mapped network drive: polled alone, and gentler poll intervals suit it.
    Remote,
}

impl PathKind {
    /// Whether the watcher is worth arming for this kind of path.
    pub fn notify_useful(self) -> bool {
        self == PathKind::Local
    }
}

/// Classify a path. A relative path resolves against the current directory; on any failure to
/// resolve, the answer is `Local` (the poll loop works everywhere; a watcher armed on an odd
/// path only wastes a watcher).
pub fn classify(path: &Path) -> PathKind {
    let resolved;
    let path = if path.is_absolute() {
        path
    } else {
        match std::path::absolute(path) {
            Ok(p) => {
                resolved = p;
                &resolved
            }
            Err(_) => return PathKind::Local,
        }
    };
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => PathKind::Remote,
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => drive_kind(letter),
            Prefix::DeviceNS(_) | Prefix::Verbatim(_) => PathKind::Local,
        },
        // No prefix component: an absolute path on a system without drive letters.
        _ => PathKind::Local,
    }
}

#[cfg(windows)]
fn drive_kind(letter: u8) -> PathKind {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    // A stable Win32 value (winbase.h), not exported by this feature set.
    const DRIVE_REMOTE: u32 = 4;

    // `X:\` as a NUL-terminated UTF-16 string.
    let root: [u16; 4] = [letter as u16, b':' as u16, b'\\' as u16, 0];
    // SAFETY: a pointer to a valid NUL-terminated buffer that outlives the call; the function
    // only reads it.
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    if kind == DRIVE_REMOTE { PathKind::Remote } else { PathKind::Local }
}

#[cfg(not(windows))]
fn drive_kind(_letter: u8) -> PathKind {
    PathKind::Local
}
