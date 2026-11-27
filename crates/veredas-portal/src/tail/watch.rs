//! The filesystem watcher that wakes a follower's poll early (C-035, C-036). Only what concerns
//! the follower wakes it: the names it follows in their folder (made, removed, renamed, a change of
//! attributes), the folder itself, and the writes to the file it reads, never a neighbour's writes
//! nor its own opens (C-035's second idle-cost refute round: a neighbour written flat out cost the
//! follower 80 % of a core, then the watcher's own thread a whole one). The loop keeps the watch at
//! every pass: armed again when its folder is made anew or a followed link leads elsewhere, since a
//! watch dies with the folder it was set on, and moved to the file the follower reads now.
//!
//! On Linux the watch is inotify's, asked for what it needs alone: the folder for its names, the
//! file for its writes, so a busy neighbour makes no event at all. Elsewhere it is notify's, whose
//! folder watch reports every write in the folder: there a neighbour's writes reach the watcher's
//! own thread, which sets them aside without waking the follower.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use crate::family::FamilySpec;
use crate::tail::cursor::TailedFile;

/// With a watcher armed, a family's folder is listed when the watcher reports a name made, removed
/// or renamed there, and at this interval otherwise (never more often than the configured rescan):
/// a fallback for an event the watcher lost, so a quiet family's idle cost does not grow with the
/// members its folder holds.
pub(crate) const WATCHED_RESCAN: Duration = Duration::from_secs(10);

/// What tells a folder from the one made anew at its path, without holding it open: its device,
/// inode and birth time on Unix, its birth time elsewhere.
type FolderId = (u64, u64, Option<SystemTime>);

fn folder_id(dir: &Path) -> Option<FolderId> {
    let meta = std::fs::metadata(dir).ok().filter(|m| m.is_dir())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino(), meta.created().ok()))
    }
    #[cfg(not(unix))]
    {
        Some((0, 0, meta.created().ok()))
    }
}

/// The folder a path's file lives in: `.` for a bare name.
pub(crate) fn folder_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// A path as a watcher names it: absolute, joined to the current folder when relative.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Where a symbolic link at `path` leads, followed link by link: the last link's text, its
/// folder resolved, so a link whose target is gone for the moment still names the folder and the
/// name it will be made at again (a target deleted and made again was read a poll late, C-035's
/// third idle-cost round). `None` for a file that is no link.
fn link_target(path: &Path) -> Option<PathBuf> {
    let mut at = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                let text = std::fs::read_link(&at).ok()?;
                at = if text.is_absolute() { text } else { folder_of(&at).join(text) };
            }
            _ => break,
        }
    }
    if at == path {
        return None;
    }
    let name = at.file_name()?.to_os_string();
    let folder = folder_of(&at);
    Some(std::fs::canonicalize(&folder).unwrap_or(folder).join(name))
}

/// What a watch wakes its loop for, by the name an event in a watched folder carries.
enum Interest {
    /// A followed file: its name, and its link's target's name.
    Names(Vec<OsString>),
    /// A family: the names its pattern matches and its fixed active's, relative to its folder; in
    /// a recursive one, a folder made or moved into the tree that holds a member (it is looked
    /// into, its members perhaps made before its watch).
    Family(FamilySpec),
}

impl Interest {
    /// Whether an event naming `name` in the watched folder at `rel` (relative to the family's
    /// folder, empty at its top) concerns what is followed. A name never UTF-8 is no member's. A
    /// folder's own event in a recursive tree is judged by what the folder holds, not here: a
    /// folder made and removed beside the members listed the whole tree each time (C-036's sixth
    /// idle-cost round).
    fn wants_name(&self, rel: &str, name: &OsStr, is_dir: bool) -> bool {
        match self {
            Interest::Names(names) => names.iter().any(|n| n == name),
            Interest::Family(spec) => {
                if spec.recursive() && is_dir {
                    return false;
                }
                let Some(name) = name.to_str() else { return false };
                if rel.is_empty() { spec.names(name) } else { spec.names(&format!("{rel}/{name}")) }
            }
        }
    }
}

struct Armed {
    /// `None` when no watcher could be set up on this folder: not tried again until the folder or
    /// the link's target changes, so a pass never spawns a watcher it cannot have.
    backend: Option<backend::Backend>,
    folder: Option<FolderId>,
    target: Option<PathBuf>,
    /// The identity of the link's target's folder at the arm: made anew, or made at last, the
    /// watch is armed again (C-035's fourth idle-cost round).
    target_folder: Option<FolderId>,
}

/// A follower's watcher, kept by its loop.
pub(crate) struct Watch {
    wanted: bool,
    wake: Sender<()>,
    /// Set by an event that may change the names a family's folder holds, before its wake.
    renamed: Arc<AtomicBool>,
    armed: Option<Armed>,
}

impl Watch {
    /// `wanted` false (a share, the watcher off): never armed, the poll alone. The watch holds a
    /// sender of the wake channel either way, so the channel never reads as disconnected.
    pub(crate) fn new(wanted: bool, wake: Sender<()>) -> Self {
        Watch { wanted, wake, renamed: Arc::new(AtomicBool::new(false)), armed: None }
    }

    /// Whether a watcher is armed and its folder still watched: a family then lists its folder on
    /// the watcher's word. A watch whose folder is gone is no watch (it read as armed, and the
    /// start's rounds waited their poll, C-036's fifth idle-cost round).
    pub(crate) fn alive(&self) -> bool {
        self.armed.as_ref().is_some_and(|a| a.backend.as_ref().is_some_and(|b| !b.gone()))
    }

    /// How many times the kernel's queue overflowed under this watch (Linux; a test's precondition).
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn overflows(&self) -> u64 {
        self.armed.as_ref().and_then(|a| a.backend.as_ref()).map_or(0, |b| b.overflows().0)
    }

    /// How many times the folders were walked after an overflow (Linux; a test's clock).
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn resyncs(&self) -> u64 {
        self.armed.as_ref().and_then(|a| a.backend.as_ref()).map_or(0, |b| b.overflows().1)
    }

    /// Whether the armed watch lost a folder it was set on: armed again at the next keep.
    fn dead(&self) -> bool {
        self.armed.as_ref().is_some_and(|a| a.backend.as_ref().is_some_and(|b| b.gone()))
    }

    /// Whether the watcher reported a name made, removed or renamed since the last call (or was
    /// armed anew, after a while nothing watched): the folder is to be listed.
    pub(crate) fn listing_due(&self) -> bool {
        self.renamed.swap(false, Ordering::AcqRel)
    }

    /// How long a family waits for its next look while attaching: its poll with a watcher armed
    /// (the watcher wakes it), else no longer than its rescan.
    pub(crate) fn wait_between(&self, poll: Duration, rescan: Duration) -> Duration {
        if self.alive() { poll } else { poll.min(rescan) }
    }

    /// How long a family waits between two listings nothing asked for.
    pub(crate) fn rescan_every(&self, configured: Duration) -> Duration {
        if self.alive() { configured.max(WATCHED_RESCAN) } else { configured }
    }

    /// Keep a file tail's watch: on the followed path's folder and, for a link, on its target's,
    /// and on the file read (`open`) for its writes. Armed once the folder is there and again when
    /// it is made anew or the link leads elsewhere. Called at every pass, before the read, so a
    /// write after it wakes the next wait.
    pub(crate) fn keep_on_file(&mut self, path: &Path, open: Option<&TailedFile>) {
        if !self.wanted {
            return;
        }
        let folder = folder_of(path);
        let id = folder_id(&folder);
        let target = link_target(path);
        let target_folder = target.as_deref().map(folder_of).and_then(|f| folder_id(&f));
        let same = id.is_some() && !self.dead() && self.armed.as_ref().is_some_and(|a| a.folder == id && a.target == target && a.target_folder == target_folder);
        if !same {
            self.armed = None;
            if id.is_none() {
                return; // nothing to watch until the folder is there
            }
            let mut names: Vec<OsString> = path.file_name().map(|n| vec![n.to_os_string()]).unwrap_or_default();
            let mut folders = vec![absolute(&folder)];
            if let Some(t) = &target {
                if let Some(n) = t.file_name() {
                    names.push(n.to_os_string());
                }
                if let Some(parent) = t.parent()
                    && std::fs::canonicalize(&folder).ok().as_deref() != Some(parent)
                {
                    folders.push(parent.to_path_buf());
                }
            }
            let backend = backend::Backend::arm(&folders, false, Interest::Names(names), self.wake.clone(), Arc::clone(&self.renamed));
            self.armed = Some(Armed { backend, folder: id, target, target_folder });
            // Whatever happened while nothing watched is listed and read at this pass.
            self.renamed.store(true, Ordering::Release);
        }
        if let Some(b) = self.armed.as_mut().and_then(|a| a.backend.as_mut()) {
            b.keep_file(open);
        }
    }

    /// Keep a family's watch on its folder (its tree when recursive), armed once the folder is
    /// there and again when it is made anew or its watch died, and on the file the follower reads
    /// now (`active`), for its writes. Called before the attach and at every pass of the live loop.
    pub(crate) fn keep_on_family(&mut self, spec: &FamilySpec, active: Option<&TailedFile>) {
        if !self.wanted {
            return;
        }
        let id = folder_id(spec.dir());
        if !(id.is_some() && !self.dead() && self.armed.as_ref().is_some_and(|a| a.folder == id)) {
            self.armed = None;
            if id.is_none() {
                return;
            }
            let backend = backend::Backend::arm(&[absolute(spec.dir())], spec.recursive(), Interest::Family(spec.clone()), self.wake.clone(), Arc::clone(&self.renamed));
            self.armed = Some(Armed { backend, folder: id, target: None, target_folder: None });
            self.renamed.store(true, Ordering::Release);
        }
        if let Some(b) = self.armed.as_mut().and_then(|a| a.backend.as_mut()) {
            b.keep_file(active);
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    //! inotify, asked for the folder's names and the file's writes alone.

    use std::collections::HashMap;
    use std::ffi::{CString, OsStr};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    use super::Interest;
    use crate::tail::cursor::TailedFile;

    /// How deep a recursive family's tree is watched: as deep as it is listed.
    const MAX_DEPTH: usize = 16;

    /// A folder's names: made, removed, renamed in or out, a change of attributes (a held file's
    /// mode that opens it), the folder itself removed or renamed. No write and no open: a
    /// neighbour's writes and the follower's own opens make no event.
    const FOLDER_MASK: u32 = libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_MOVED_FROM
        | libc::IN_MOVED_TO
        | libc::IN_ATTRIB
        | libc::IN_DELETE_SELF
        | libc::IN_MOVE_SELF
        | libc::IN_ONLYDIR;
    /// The followed file's writes: a write, a truncation, a close after writing.
    const FILE_MASK: u32 = libc::IN_MODIFY | libc::IN_CLOSE_WRITE;
    /// How long after the last overflow the folders are walked again.
    const QUIET: std::time::Duration = std::time::Duration::from_millis(50);
    /// How far apart two listings asked for after overflows are at least.
    const LIST_SPACING: std::time::Duration = std::time::Duration::from_secs(5);
    /// How long the reader waits after a batch before reading the next.
    const COALESCE: std::time::Duration = std::time::Duration::from_millis(10);
    /// The events that may change the names a folder holds.
    const RENAMES: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO;

    enum Role {
        /// A watched folder: its path, its path relative to the family's folder, its depth.
        Folder(PathBuf, String, usize),
        File,
    }

    struct Inner {
        fd: OwnedFd,
        interest: Interest,
        recursive: bool,
        roles: Mutex<HashMap<i32, Role>>,
        /// Set when a folder the watch was set on (depth 0) is removed or moved away: the watch
        /// is no watch from then on.
        top_gone: AtomicBool,
        /// The folders the watch was set on, walked again after the queue overflowed.
        tops: Vec<PathBuf>,
        /// How many times the kernel's queue overflowed, and the folders were walked after it.
        overflows: std::sync::atomic::AtomicU64,
        resyncs: std::sync::atomic::AtomicU64,
    }

    pub(crate) struct Backend {
        inner: Arc<Inner>,
        stop: OwnedFd,
        reader: Option<JoinHandle<()>>,
        /// The file watched for its writes: its watch and its device and inode.
        file: Option<(i32, (u64, u64))>,
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            // One byte on the stop pipe ends the reader's poll.
            let byte = 1u8;
            // SAFETY: a write of one byte from a live buffer to a pipe this backend owns.
            let _ = unsafe { libc::write(self.stop.as_raw_fd(), (&raw const byte).cast(), 1) };
            if let Some(r) = self.reader.take() {
                let _ = r.join();
            }
        }
    }

    fn cpath(path: &Path) -> Option<CString> {
        CString::new(path.as_os_str().as_bytes()).ok()
    }

    impl Inner {
        fn add(&self, path: &Path, mask: u32) -> Option<i32> {
            let c = cpath(path)?;
            // SAFETY: a NUL-terminated path and an inotify descriptor this watch owns.
            let wd = unsafe { libc::inotify_add_watch(self.fd.as_raw_fd(), c.as_ptr(), mask) };
            (wd >= 0).then_some(wd)
        }

        /// Watch a folder for its names and, recursive, every folder under it to the depth listed:
        /// whether the watch was set, and whether a member's name stands in what it watched (a
        /// folder born or moved into the tree asks for a listing only then).
        fn add_tree(&self, path: &Path, rel: &str, depth: usize) -> (bool, bool) {
            self.add_tree_seen(path, rel, depth, &mut Vec::new())
        }

        /// `add_tree`, noting each folder's watch in `seen`.
        fn add_tree_seen(&self, path: &Path, rel: &str, depth: usize, seen: &mut Vec<i32>) -> (bool, bool) {
            let Some(wd) = self.add(path, FOLDER_MASK) else { return (false, false) };
            seen.push(wd);
            if let Ok(mut roles) = self.roles.lock() {
                roles.insert(wd, Role::Folder(path.to_path_buf(), rel.to_string(), depth));
            }
            let mut members = false;
            if self.recursive
                && depth < MAX_DEPTH
                && let Ok(entries) = std::fs::read_dir(path)
            {
                for e in entries.flatten() {
                    let name = e.file_name();
                    let Some(n) = name.to_str() else { continue };
                    let sub = if rel.is_empty() { n.to_string() } else { format!("{rel}/{n}") };
                    match e.file_type() {
                        Ok(t) if t.is_dir() => members |= self.add_tree_seen(&e.path(), &sub, depth + 1, seen).1,
                        Ok(t) if t.is_file() => members |= matches!(&self.interest, Interest::Family(spec) if spec.names(&sub)),
                        _ => {}
                    }
                }
            }
            (true, members)
        }

        /// After an overflow, the folders as they stand now: walked again, a folder born while
        /// events were lost watched, a folder moved out or removed while they were lost watched no
        /// more (its events lost, a member's name made in it still woke the loop and listed the
        /// tree, C-036's eighth idle-cost round), a top folder not found again the watch's end.
        /// The file's watch is kept.
        fn resync(&self) {
            self.resyncs.fetch_add(1, Ordering::Relaxed);
            let mut seen = Vec::new();
            for top in &self.tops {
                self.add_tree_seen(top, "", 0, &mut seen);
            }
            let Ok(mut roles) = self.roles.lock() else { return };
            let lost: Vec<(i32, usize)> = roles
                .iter()
                .filter_map(|(wd, r)| match r {
                    Role::Folder(_, _, depth) if !seen.contains(wd) => Some((*wd, *depth)),
                    _ => None,
                })
                .collect();
            for (wd, depth) in lost {
                roles.remove(&wd);
                if depth == 0 {
                    self.top_gone.store(true, Ordering::Release);
                }
                // SAFETY: a watch descriptor of this inotify instance.
                let _ = unsafe { libc::inotify_rm_watch(self.fd.as_raw_fd(), wd) };
            }
        }

        /// Read one batch of events: whether to wake the loop, whether a name may have changed,
        /// whether the kernel's queue overflowed. A name the batch has just judged in the same
        /// folder is not judged again: a neighbour made and removed over and over is one pattern
        /// match a batch.
        fn handle(&self, bytes: &[u8]) -> (bool, bool, bool) {
            let (mut wake, mut renamed, mut overflowed) = (false, false, false);
            let mut new_dirs: Vec<(PathBuf, String, usize)> = Vec::new();
            // Folders moved away from where their watch named them: their watches dropped, so a
            // folder moved out of the tree is watched no more, and one moved within it is watched
            // again under its new name (C-036's fourth idle-cost round).
            let mut gone: Vec<PathBuf> = Vec::new();
            let mut last: Option<(i32, &[u8], bool, bool)> = None;
            {
                let Ok(mut roles) = self.roles.lock() else { return (true, true, false) };
                let mut o = 0usize;
                while let Some(head) = bytes.get(o..o + 16) {
                    let word = |i: usize| head.get(i..i + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).unwrap_or_default();
                    let wd = i32::from_ne_bytes(word(0));
                    let mask = u32::from_ne_bytes(word(4));
                    let len = u32::from_ne_bytes(word(12)) as usize;
                    let raw = bytes.get(o + 16..o + 16 + len).unwrap_or_default();
                    o += 16 + len;
                    let name = OsStr::from_bytes(raw.split(|b| *b == 0).next().unwrap_or_default());
                    if mask & libc::IN_Q_OVERFLOW != 0 {
                        // Events were lost, the read file's writes perhaps among them: the loop
                        // wakes to read. The listing waits for the flood to end (`read_loop`): a
                        // listing at every overflow listed a whole family a hundred times a second
                        // beside a neighbour making names flat out (C-036's seventh idle-cost
                        // round).
                        wake = true;
                        overflowed = true;
                        self.overflows.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if mask & libc::IN_IGNORED != 0 {
                        // A watch gone with what it watched: a folder removed, the file's last link.
                        // A folder below the top asks for nothing: its members report their own
                        // removal (a folder made and removed beside the members woke the loop and
                        // listed the tree each time, C-036's sixth idle-cost round).
                        match roles.remove(&wd) {
                            Some(Role::Folder(_, _, 0)) => {
                                (wake, renamed) = (true, true);
                                self.top_gone.store(true, Ordering::Release);
                            }
                            Some(Role::File) => wake = true,
                            // A folder below the top, or a watch this backend dropped itself.
                            _ => {}
                        }
                        continue;
                    }
                    match roles.get(&wd) {
                        None => {}
                        Some(Role::File) => wake = true,
                        Some(Role::Folder(path, rel, depth)) => {
                            if name.is_empty() {
                                // The folder itself, removed or renamed: the top one wakes the
                                // loop and kills the watch; one below asks for nothing (its
                                // members report their own removal, or left with it), its watch
                                // dropped when it moved.
                                if *depth == 0 {
                                    (wake, renamed) = (true, true);
                                    if mask & (libc::IN_MOVE_SELF | libc::IN_DELETE_SELF) != 0 {
                                        self.top_gone.store(true, Ordering::Release);
                                    }
                                } else if mask & libc::IN_MOVE_SELF != 0 {
                                    gone.push(path.clone());
                                }
                                continue;
                            }
                            let is_dir = mask & libc::IN_ISDIR != 0;
                            if is_dir && self.recursive && mask & libc::IN_MOVED_FROM != 0 {
                                gone.push(path.join(name));
                            }
                            if is_dir && self.recursive && mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 && *depth < MAX_DEPTH {
                                let sub = match name.to_str() {
                                    Some(n) if rel.is_empty() => Some(n.to_string()),
                                    Some(n) => Some(format!("{rel}/{n}")),
                                    None => None,
                                };
                                if let Some(sub) = sub {
                                    new_dirs.push((path.join(name), sub, depth + 1));
                                }
                            }
                            let wanted = match last {
                                Some((w, n, d, verdict)) if w == wd && n == name.as_bytes() && d == is_dir => verdict,
                                _ => {
                                    let verdict = self.interest.wants_name(rel, name, is_dir);
                                    last = Some((wd, name.as_bytes(), is_dir, verdict));
                                    verdict
                                }
                            };
                            if wanted {
                                wake = true;
                                renamed |= mask & RENAMES != 0;
                            }
                        }
                    }
                }
                for top in &gone {
                    let dropped: Vec<i32> = roles.iter().filter(|(_, r)| matches!(r, Role::Folder(p, ..) if p.starts_with(top))).map(|(wd, _)| *wd).collect();
                    for wd in dropped {
                        roles.remove(&wd);
                        // SAFETY: a watch descriptor of this inotify instance.
                        let _ = unsafe { libc::inotify_rm_watch(self.fd.as_raw_fd(), wd) };
                    }
                }
            }
            // A folder born or moved into a recursive tree is watched, and looked into: a member
            // there (perhaps made before its watch) wakes the loop and asks for a listing.
            for (path, rel, depth) in new_dirs {
                if self.add_tree(&path, &rel, depth).1 {
                    (wake, renamed) = (true, true);
                }
            }
            (wake, renamed, overflowed)
        }
    }

    fn read_loop(inner: Arc<Inner>, stop: OwnedFd, wake: Sender<()>, renamed: Arc<AtomicBool>) {
        // Ten milliseconds of a busy folder's names fit in one read.
        let mut buf = vec![0u8; 64 * 1024];
        // After an overflow, the folders are walked again and listed once no overflow has come
        // for a moment: a folder born or gone while events were lost is watched or dropped, and
        // the names lost are listed, once, not at every overflow of a flood. Timed from the last
        // overflow, not from silence: a neighbour's trickle of names put it off for good (C-036's
        // eighth idle-cost round).
        // The listing a walk asks for is spaced five seconds apart: a flood whose overflows pause
        // now and then walks at every pause (the reader's cost), but a listing of a family of
        // thousands is the follower's (45 ms in a test build). A walk that comes sooner leaves its
        // listing pending, asked for when it falls due.
        let mut last_overflow: Option<std::time::Instant> = None;
        let mut last_listing: Option<std::time::Instant> = None;
        let mut listing_pending = false;
        loop {
            let resync_in = last_overflow.map(|t| QUIET.saturating_sub(t.elapsed()));
            let listing_in = |last: Option<std::time::Instant>| last.map_or(std::time::Duration::ZERO, |l| LIST_SPACING.saturating_sub(l.elapsed()));
            if resync_in == Some(std::time::Duration::ZERO) {
                last_overflow = None;
                inner.resync();
                listing_pending = true;
            }
            if listing_pending && listing_in(last_listing).is_zero() {
                listing_pending = false;
                last_listing = Some(std::time::Instant::now());
                renamed.store(true, Ordering::Release);
                if wake.send(()).is_err() {
                    return;
                }
                continue;
            }
            let pending_in = listing_pending.then(|| listing_in(last_listing));
            let resync_in = match (last_overflow.and(resync_in), pending_in) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let mut fds = [
                libc::pollfd { fd: inner.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: stop.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            ];
            let timeout = match resync_in {
                Some(d) => i32::try_from(d.as_millis()).unwrap_or(i32::MAX - 1) + 1,
                None => -1,
            };
            // SAFETY: two pollfd entries in a live array.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            if n == 0 {
                continue; // the walk, at the loop's top
            }
            if fds[1].revents != 0 || fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return;
            }
            if fds[0].revents & libc::POLLIN == 0 {
                continue;
            }
            // SAFETY: a read into a live buffer of its own length.
            let got = unsafe { libc::read(inner.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if got <= 0 {
                let e = std::io::Error::last_os_error().kind();
                if got < 0 && matches!(e, std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) {
                    continue;
                }
                return;
            }
            let (w, r, o) = inner.handle(buf.get(..got as usize).unwrap_or_default());
            if o {
                last_overflow = Some(std::time::Instant::now());
            }
            if r {
                renamed.store(true, Ordering::Release);
            }
            if w && wake.send(()).is_err() {
                return; // the loop is gone
            }
            // What follows a batch waits a moment and is read as one: an event wakes the loop at
            // once, and a folder busy with names costs this thread a hundred reads a second at
            // most, not a wake an event.
            std::thread::sleep(COALESCE);
        }
    }

    impl Backend {
        pub(crate) fn arm(folders: &[PathBuf], recursive: bool, interest: Interest, wake: Sender<()>, renamed: Arc<AtomicBool>) -> Option<Backend> {
            // SAFETY: plain system calls returning new descriptors, owned at once.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            if fd < 0 {
                return None;
            }
            // SAFETY: a descriptor just returned by inotify_init1, owned by nothing else.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let mut pipe = [0i32; 2];
            // SAFETY: a two-entry array for the pipe's ends.
            if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
                return None;
            }
            // SAFETY: the two descriptors pipe2 just returned, owned by nothing else.
            let (stop_r, stop_w) = unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) };
            let inner = Arc::new(Inner {
                fd,
                interest,
                recursive,
                roles: Mutex::new(HashMap::new()),
                top_gone: AtomicBool::new(false),
                tops: folders.to_vec(),
                overflows: std::sync::atomic::AtomicU64::new(0),
                resyncs: std::sync::atomic::AtomicU64::new(0),
            });
            // The followed path's folder or nothing; a link's target's folder, if it can be.
            for (i, f) in folders.iter().enumerate() {
                if !inner.add_tree(f, "", 0).0 && i == 0 {
                    return None;
                }
            }
            let reader_inner = Arc::clone(&inner);
            let reader = std::thread::Builder::new()
                .name("tail-watch".into())
                .spawn(move || read_loop(reader_inner, stop_r, wake, renamed))
                .ok()?;
            Some(Backend { inner, stop: stop_w, reader: Some(reader), file: None })
        }

        /// Whether a folder the watch was set on is gone.
        pub(crate) fn gone(&self) -> bool {
            self.inner.top_gone.load(Ordering::Acquire)
        }

        /// How many times the kernel's queue overflowed, and the folders were walked after it (a
        /// test's precondition and its clock).
        #[cfg(test)]
        pub(crate) fn overflows(&self) -> (u64, u64) {
            (self.inner.overflows.load(Ordering::Relaxed), self.inner.resyncs.load(Ordering::Relaxed))
        }

        /// Watch the file the follower reads for its writes, through its own descriptor
        /// (`/proc/self/fd/N`, which inotify follows to the file however it is named now, deleted
        /// included), moved to the next file read; a watch the kernel dropped is set again. A path
        /// watched instead lost the file once it was deleted or moved away while its writer still
        /// wrote, every line a poll late (C-036's fifth idle-cost round).
        pub(crate) fn keep_file(&mut self, open: Option<&TailedFile>) {
            let key = open.map(|tf| tf.watch_key());
            if let (Some((_, dev, ino)), Some((wd, k))) = (key, self.file)
                && k == (dev, ino)
                && self.inner.roles.lock().is_ok_and(|r| matches!(r.get(&wd), Some(Role::File)))
            {
                return;
            }
            if let Some((wd, _)) = self.file.take() {
                if let Ok(mut roles) = self.inner.roles.lock()
                    && matches!(roles.get(&wd), Some(Role::File))
                {
                    roles.remove(&wd);
                }
                // SAFETY: a watch descriptor of this inotify instance; one already gone is refused.
                let _ = unsafe { libc::inotify_rm_watch(self.inner.fd.as_raw_fd(), wd) };
            }
            let Some((fd, dev, ino)) = key else { return };
            if let Some(wd) = self.inner.add(Path::new(&format!("/proc/self/fd/{fd}")), FILE_MASK) {
                if let Ok(mut roles) = self.inner.roles.lock() {
                    // The same inode as a watched folder never: a file is no folder.
                    roles.entry(wd).or_insert(Role::File);
                }
                self.file = Some((wd, (dev, ino)));
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod backend {
    //! notify's watcher: its folder watch reports every event in the folder, filtered here by the
    //! names they carry.

    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;

    use notify::event::{AccessKind, AccessMode, ModifyKind};
    use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    use super::Interest;
    use crate::tail::cursor::TailedFile;

    pub(crate) struct Backend {
        _watcher: RecommendedWatcher,
        /// The folder watched and its identity at the arm: another there, or none, is a dead watch.
        root: PathBuf,
        id: Option<super::FolderId>,
    }

    /// Whether a member's name stands in a folder of a recursive tree, to the depth listed.
    fn holds_member(spec: &crate::family::FamilySpec, root: &Path, folder: &Path, depth: usize) -> bool {
        let Ok(entries) = std::fs::read_dir(folder) else { return false };
        for e in entries.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() && depth < 16 => {
                    if holds_member(spec, root, &path, depth + 1) {
                        return true;
                    }
                }
                Ok(t) if t.is_file() => {
                    let Some(rel) = path.strip_prefix(root).ok() else { continue };
                    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
                    if parts.is_some_and(|p| spec.names(&p.join("/"))) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// An event the follower makes itself: a file opened, read, or closed after reading.
    fn woken_by_self(kind: &EventKind) -> bool {
        matches!(kind, EventKind::Access(a) if !matches!(a, AccessKind::Close(AccessMode::Write)))
    }

    /// An event that may change the names a folder holds, or one the watcher cannot tell.
    fn may_rename(kind: &EventKind) -> bool {
        matches!(kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) | EventKind::Any | EventKind::Other)
    }

    /// Whether an event in a recursive tree is a folder's, which may bring or take members: told
    /// by its kind, or, where the kind does not say (Windows reports every create and a rename's
    /// new name untyped), by a folder standing at the path now. An untyped remove or a rename's old
    /// name is never taken for one: a file made and removed beside the members woke the follower
    /// and listed the tree each time (C-036's third idle-cost round), and a folder removed has its
    /// members report their own removal.
    fn folder_event(kind: &EventKind, path: &Path) -> bool {
        use notify::event::{CreateKind, RemoveKind, RenameMode};
        match kind {
            EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => true,
            EventKind::Create(CreateKind::Any | CreateKind::Other)
            | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both | RenameMode::Any | RenameMode::Other))
            | EventKind::Any
            | EventKind::Other => path.is_dir(),
            _ => false,
        }
    }

    /// Whether an event at `path` concerns what is followed; a path this cannot place wakes.
    fn wants(interest: &Interest, folders: &[PathBuf], kind: &EventKind, path: &Path) -> bool {
        if folders.iter().any(|f| f == path) {
            return true;
        }
        match interest {
            Interest::Names(_) => path.file_name().is_none_or(|n| interest.wants_name("", n, false)),
            Interest::Family(spec) if spec.recursive() => {
                let Some(rel) = folders.first().and_then(|root| path.strip_prefix(root).ok()) else { return true };
                let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
                match parts {
                    // A folder's event asks for a listing only when the folder holds a member.
                    Some(parts) => spec.names(&parts.join("/")) || (folder_event(kind, path) && folders.first().is_some_and(|root| holds_member(spec, root, path, 0))),
                    None => false,
                }
            }
            Interest::Family(_) => path.file_name().is_none_or(|n| interest.wants_name("", n, false)),
        }
    }

    impl Backend {
        pub(crate) fn arm(folders: &[PathBuf], recursive: bool, interest: Interest, wake: Sender<()>, renamed: Arc<AtomicBool>) -> Option<Backend> {
            let watched = folders.to_vec();
            let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let (wanted, lists) = match &res {
                    Ok(e) => (!woken_by_self(&e.kind) && (e.paths.is_empty() || e.paths.iter().any(|p| wants(&interest, &watched, &e.kind, p))), may_rename(&e.kind)),
                    Err(_) => (true, true),
                };
                if !wanted {
                    return;
                }
                if lists {
                    renamed.store(true, Ordering::Release);
                }
                let _ = wake.send(());
            })
            .ok()?;
            let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            // The followed path's folder or nothing; a link's target's folder, if it can be.
            for (i, f) in folders.iter().enumerate() {
                if watcher.watch(f, mode).is_err() && i == 0 {
                    return None;
                }
            }
            let root = folders.first().cloned().unwrap_or_default();
            let id = super::folder_id(&root);
            Some(Backend { _watcher: watcher, root, id })
        }

        /// The folder watch reports the file's writes already.
        pub(crate) fn keep_file(&mut self, _open: Option<&TailedFile>) {}

        /// Whether the folder watched is gone or another now: told by its identity, a stat, since
        /// what notify reports of a watched folder's removal differs by system.
        pub(crate) fn gone(&self) -> bool {
            super::folder_id(&self.root) != self.id
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    //! The watch's filter, pinned directly: what wakes the loop and what asks for a listing.

    use std::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::family::ActiveMode;
    use crate::tail::FileStart;

    fn open(path: &Path) -> TailedFile {
        TailedFile::open(path, FileStart::End).unwrap()
    }

    /// Whatever the steps before made is taken, so the next step's wakes are its own.
    fn settle(rx: &Receiver<()>) {
        std::thread::sleep(Duration::from_millis(100));
        while rx.try_recv().is_ok() {}
    }

    fn quiet(rx: &Receiver<()>) -> bool {
        rx.recv_timeout(Duration::from_millis(300)).is_err()
    }

    fn woken(rx: &Receiver<()>) -> bool {
        rx.recv_timeout(Duration::from_secs(2)).is_ok()
    }

    /// A file tail's watch wakes for its file's writes, a create, a remove, a rename and a change
    /// of mode at its name, never for a neighbour's writes, names or modes, nor for the follower's
    /// own opens and reads.
    #[test]
    fn a_file_watch_wakes_for_its_name_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("followed.log");
        let other = dir.path().join("other.log");
        std::fs::write(&path, "").unwrap();
        std::fs::write(&other, "").unwrap();
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        let read = open(&path);
        watch.keep_on_file(&path, Some(&read));
        assert!(watch.alive());
        assert!(quiet(&rx), "nothing yet");
        settle(&rx);
        let append = |p: &Path| {
            let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
            std::io::Write::write_all(&mut f, b"x\n").unwrap();
        };
        // A neighbour: its writes, a file made, renamed and removed beside, its mode changed.
        append(&other);
        std::fs::write(dir.path().join("made.tmp"), "").unwrap();
        std::fs::rename(dir.path().join("made.tmp"), dir.path().join("moved.tmp")).unwrap();
        std::fs::remove_file(dir.path().join("moved.tmp")).unwrap();
        std::fs::set_permissions(&other, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        // The follower's own: an open, a read, a listing of the folder.
        let _ = std::fs::read(&path).unwrap();
        let _ = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(quiet(&rx), "a neighbour's writes and names and the follower's own reads wake nothing");
        append(&path);
        assert!(woken(&rx), "a write to the file");
        settle(&rx);
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        assert!(woken(&rx), "a change of the file's mode");
        settle(&rx);
        let aside = dir.path().join("followed.log.new");
        std::fs::write(&aside, "").unwrap();
        assert!(quiet(&rx), "a file made beside under another name");
        std::fs::rename(&aside, &path).unwrap();
        assert!(woken(&rx), "a file renamed over the name");
        // The watch moves to the file read now, renamed into place: its writes wake.
        let read = open(&path);
        watch.keep_on_file(&path, Some(&read));
        settle(&rx);
        append(&path);
        assert!(woken(&rx), "a write to the file renamed into place");
        settle(&rx);
        std::fs::remove_file(&path).unwrap();
        assert!(woken(&rx), "the file removed");
        settle(&rx);
        std::fs::write(&path, "").unwrap();
        assert!(woken(&rx), "the file made again");
    }

    /// A family's watch wakes for its members' names and its fixed active's, and asks for a
    /// listing then, never for a name that is no member's; a write to the file the follower reads
    /// wakes without asking for one.
    #[test]
    fn a_family_watch_wakes_for_its_members_and_lists_on_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let spec = FamilySpec::new(dir.path(), r"app_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("app.log".into())).unwrap();
        let active = dir.path().join("app.log");
        std::fs::write(&active, "").unwrap();
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        let read = open(&active);
        watch.keep_on_family(&spec, Some(&read));
        assert!(watch.listing_due(), "armed: a listing for what happened before");
        assert!(quiet(&rx));
        settle(&rx);
        std::fs::write(dir.path().join("noise.txt"), "x").unwrap();
        std::fs::rename(dir.path().join("noise.txt"), dir.path().join("noise.old")).unwrap();
        std::fs::remove_file(dir.path().join("noise.old")).unwrap();
        assert!(quiet(&rx), "a name no member has");
        assert!(!watch.listing_due(), "and no listing asked for");
        let mut f = std::fs::OpenOptions::new().append(true).open(&active).unwrap();
        std::io::Write::write_all(&mut f, b"x\n").unwrap();
        assert!(woken(&rx), "a write to the active");
        assert!(!watch.listing_due(), "a write is read, never listed for");
        settle(&rx);
        std::fs::set_permissions(&active, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        assert!(woken(&rx), "a change of the active's mode");
        assert!(!watch.listing_due(), "nor a change of mode");
        settle(&rx);
        std::fs::rename(&active, dir.path().join("app_1.log")).unwrap();
        assert!(woken(&rx), "the active renamed into a member");
        assert!(watch.listing_due(), "a rename asks for a listing");
        settle(&rx);
        std::fs::write(&active, "").unwrap();
        assert!(woken(&rx), "the fixed active made");
        assert!(watch.listing_due());
    }

    /// A folder moved out of a recursive tree is watched no more, nor are the folders under it: its
    /// move wakes nothing, a member's name made in it or in its subfolder wakes nothing and asks for
    /// no listing; moved back into the tree holding members, it wakes, asks for a listing and is
    /// watched again.
    #[test]
    fn a_folder_moved_out_of_the_tree_is_watched_no_more() {
        let root = tempfile::tempdir().unwrap();
        let top = root.path().join("top");
        std::fs::create_dir_all(top.join("sub/deeper")).unwrap();
        let spec = FamilySpec::new(&top, r"sub/(?:deeper/)?app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        watch.keep_on_family(&spec, None);
        settle(&rx);
        let _ = watch.listing_due();
        std::fs::write(top.join("sub/app_1.log"), "").unwrap();
        assert!(woken(&rx), "a member made in the subfolder");
        assert!(watch.listing_due());
        let archived = root.path().join("archived");
        settle(&rx);
        let _ = watch.listing_due();
        std::fs::rename(top.join("sub"), &archived).unwrap();
        assert!(quiet(&rx), "the subfolder moved out: its members gone with it, nothing to read");
        assert!(!watch.listing_due());
        std::fs::write(archived.join("app_2.log"), "").unwrap();
        std::fs::write(archived.join("deeper/app_2.log"), "").unwrap();
        assert!(quiet(&rx), "a member's name made in the folder moved out or in its subfolder");
        assert!(!watch.listing_due(), "and no listing asked for");
        std::fs::rename(&archived, top.join("sub")).unwrap();
        assert!(woken(&rx), "the folder moved back in");
        assert!(watch.listing_due(), "holding members, it asks for a listing");
        settle(&rx);
        let _ = watch.listing_due();
        std::fs::write(top.join("sub/app_3.log"), "").unwrap();
        assert!(woken(&rx), "a member made in it, watched again");
        assert!(watch.listing_due());
    }

    /// A file's watch follows the file read, not its path: deleted or moved out of its folder while
    /// a writer still writes it, or deleted before the watch was first set on it, its writes wake;
    /// a watch whose folder is removed is no watch.
    #[test]
    fn a_file_watch_follows_the_file_read_and_dies_with_its_folder() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        std::fs::write(&path, "").unwrap();
        let spec = FamilySpec::new(&dir, r"app\.log", None, ActiveMode::Fixed("app.log".into())).unwrap();
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        let read = open(&path);
        watch.keep_on_family(&spec, Some(&read));
        let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        settle(&rx);
        std::fs::remove_file(&path).unwrap();
        assert!(woken(&rx), "the file removed");
        watch.keep_on_family(&spec, Some(&read));
        settle(&rx);
        std::io::Write::write_all(&mut writer, b"x\n").unwrap();
        assert!(woken(&rx), "a write to the file read, deleted");
        // The deleted file closed: an open one in the folder would pin it, its removal reported
        // only once the last handle on it closed.
        drop((read, writer));
        std::fs::write(&path, "").unwrap();
        let read = open(&path);
        let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        watch.keep_on_family(&spec, Some(&read));
        settle(&rx);
        std::fs::rename(&path, root.path().join("away.log")).unwrap();
        assert!(woken(&rx), "the file moved out");
        watch.keep_on_family(&spec, Some(&read));
        settle(&rx);
        std::io::Write::write_all(&mut writer, b"x\n").unwrap();
        assert!(woken(&rx), "a write to the file read, moved out of its folder");
        assert!(watch.alive());
        // A file read, deleted before the watch was first set on it: watched through its
        // descriptor, its path gone.
        let late = dir.join("late.log");
        std::fs::write(&late, "").unwrap();
        let late_read = open(&late);
        let mut late_writer = std::fs::OpenOptions::new().append(true).open(&late).unwrap();
        std::fs::remove_file(&late).unwrap();
        watch.keep_on_family(&spec, Some(&late_read));
        settle(&rx);
        std::io::Write::write_all(&mut late_writer, b"x\n").unwrap();
        assert!(woken(&rx), "a write to a file deleted before its first watch");
        drop((read, writer, late_read, late_writer));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(woken(&rx), "the folder removed");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!watch.alive(), "a watch whose folder is gone is no watch");
    }

    /// A folder moved into a recursive tree whose only member is in a folder under it wakes the
    /// loop and asks for a listing.
    #[test]
    fn a_folder_moved_in_with_a_member_only_deeper_asks_for_a_listing() {
        let root = tempfile::tempdir().unwrap();
        let top = root.path().join("top");
        std::fs::create_dir_all(&top).unwrap();
        std::fs::create_dir_all(root.path().join("incoming/deeper")).unwrap();
        std::fs::write(root.path().join("incoming/deeper/app_9.log"), "").unwrap();
        let spec = FamilySpec::new(&top, r"[a-z]+/(?:[a-z]+/)?app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        watch.keep_on_family(&spec, None);
        settle(&rx);
        let _ = watch.listing_due();
        std::fs::rename(root.path().join("incoming"), top.join("incoming")).unwrap();
        assert!(woken(&rx), "the folder moved in");
        assert!(watch.listing_due(), "its member, a folder down, asks for a listing");
    }

    /// Flood the folder with names from `writers` threads until `stop`.
    fn flood(dir: &Path, writers: usize, stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
        (0..writers)
            .map(|t| {
                let (dir, stop) = (dir.to_path_buf(), Arc::clone(stop));
                std::thread::spawn(move || {
                    let mut n = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let p = dir.join(format!("spool_{t}_{n}.tmp"));
                        if std::fs::write(&p, "").and_then(|()| std::fs::remove_file(&p)).is_err() {
                            break;
                        }
                        n += 1;
                    }
                })
            })
            .collect()
    }

    /// Wait for the kernel's queue to have overflowed under the watch (a test's precondition).
    fn overflowed(watch: &Watch) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while watch.overflows() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(watch.overflows() > 0, "the flood overflowed the kernel's queue");
    }

    /// After a flood: every overflow's wake, and any listing a walk during it asked for, taken as
    /// they come; then, once a walk has run after the last overflow (a walk needs 50 ms without one,
    /// so none runs between an overflow and its being seen here), asserted within half a second of
    /// it, whether it asked for a listing and woke the loop, a wake only that walk sends by then (nothing the tests flood with is a
    /// name the watch wants).
    fn the_walk_lists_and_wakes(watch: &Watch, rx: &Receiver<()>) -> (bool, bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut last = watch.overflows();
        let mut walks = watch.resyncs();
        let mut last_seen = std::time::Instant::now();
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            let now = watch.overflows();
            if now != last {
                last = now;
                last_seen = std::time::Instant::now();
                while rx.try_recv().is_ok() {}
                let _ = watch.listing_due();
                walks = watch.resyncs();
                continue;
            }
            if watch.resyncs() > walks {
                // The walk comes 50 ms after the last overflow: within half a second of its being
                // seen here, whatever the rest of the run's load (a walk two seconds late passed
                // every test, C-036's ninth idle-cost round).
                let after = last_seen.elapsed();
                assert!(after < Duration::from_millis(500), "the walk {after:?} after the last overflow was seen");
                // The walk's listing, perhaps put off (five seconds after the last at least),
                // then its wake.
                let listed_by = std::time::Instant::now() + Duration::from_secs(6);
                while std::time::Instant::now() < listed_by {
                    if watch.listing_due() {
                        return (true, rx.recv_timeout(Duration::from_millis(200)).is_ok());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                return (false, false);
            }
        }
        (false, false)
    }

    /// After a flood, the watch settled: no overflow and no walk for longer than a listing may be put
    /// off (`LIST_SPACING`), so nothing of the flood wakes the loop any more, whatever woke it
    /// meanwhile taken.
    fn settled_after_the_flood(watch: &Watch, rx: &Receiver<()>) {
        let mut seen = (watch.overflows(), watch.resyncs());
        let mut since = std::time::Instant::now();
        while since.elapsed() < Duration::from_millis(5500) {
            std::thread::sleep(Duration::from_millis(100));
            let now = (watch.overflows(), watch.resyncs());
            if now != seen {
                seen = now;
                since = std::time::Instant::now();
            }
        }
        assert!(watch.resyncs() > 0, "the folders were walked after the flood");
        while rx.try_recv().is_ok() {}
        let _ = watch.listing_due();
    }

    /// A flood of names that overflows the kernel's queue wakes the loop (a write to the file read
    /// may be among what is lost) and asks for a listing five seconds apart at most while it
    /// lasts; once no overflow has come for a moment, whatever a neighbour still trickles (a name every 20 ms), the folders are
    /// walked again, the loop woken and one listing asked for, so a folder born in a recursive
    /// tree while events were lost is watched: a member made in each of twenty folders born
    /// during the flood wakes the loop and asks for a listing. Each overflow listed the family, a
    /// folder whose birth was lost was never watched (C-036's seventh idle-cost round), and the
    /// walk waited for a silence a trickle never gave (its eighth).
    #[test]
    fn an_overflow_lists_once_the_flood_is_over_and_watches_what_was_born_in_it() {
        let top = tempfile::tempdir().unwrap();
        let spec = FamilySpec::new(top.path(), r"d\d+/app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        watch.keep_on_family(&spec, None);
        settle(&rx);
        let _ = watch.listing_due();
        let stop_trickle = Arc::new(AtomicBool::new(false));
        let trickle = {
            let (dir, stop) = (top.path().to_path_buf(), Arc::clone(&stop_trickle));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let p = dir.join("status.lock");
                    if std::fs::write(&p, "").and_then(|()| std::fs::remove_file(&p)).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flooding = flood(top.path(), 8, &stop);
        overflowed(&watch);
        let mut listed = 0;
        for i in 1..=20 {
            std::fs::create_dir(top.path().join(format!("d{i}"))).unwrap();
            std::thread::sleep(Duration::from_millis(30));
            listed += usize::from(watch.listing_due());
        }
        assert!(watch.overflows() > 1, "the queue overflowed while the folders were born");
        assert!(listed <= 1, "a listing five seconds apart at most while the flood lasts ({listed} in 600 ms)");
        stop.store(true, Ordering::Relaxed);
        for f in flooding {
            f.join().unwrap();
        }
        let (listed, woke) = the_walk_lists_and_wakes(&watch, &rx);
        assert!(listed, "one listing asked for once the flood is over, the trickle going on");
        assert!(woke, "and the loop woken for it");
        for i in 1..=20 {
            settle(&rx);
            let _ = watch.listing_due();
            std::fs::write(top.path().join(format!("d{i}/app_1.log")), "").unwrap();
            assert!(woken(&rx), "a member made in d{i}, born during the flood");
            assert!(watch.listing_due(), "and a listing asked for (d{i})");
        }
        stop_trickle.store(true, Ordering::Relaxed);
        trickle.join().unwrap();
    }

    /// Folders moved out of a recursive tree while a flood overflows the kernel's queue (their
    /// moves lost) are watched no more once the folders are walked again after it: a member's name
    /// made in each of twenty wakes nothing and asks for no listing. The walk only added watches,
    /// and nine of twenty still woke the loop and listed the tree (C-036's eighth idle-cost round).
    #[test]
    fn folders_moved_out_during_an_overflow_are_watched_no_more_after_it() {
        let root = tempfile::tempdir().unwrap();
        let top = root.path().join("top");
        let out = root.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        for i in 1..=20 {
            std::fs::create_dir_all(top.join(format!("d{i}"))).unwrap();
        }
        let spec = FamilySpec::new(&top, r"d\d+/app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
        let (tx, rx) = channel();
        let mut watch = Watch::new(true, tx);
        watch.keep_on_family(&spec, None);
        settle(&rx);
        let _ = watch.listing_due();
        let stop = Arc::new(AtomicBool::new(false));
        let flooding = flood(&top, 8, &stop);
        overflowed(&watch);
        for i in 1..=20 {
            std::fs::rename(top.join(format!("d{i}")), out.join(format!("d{i}"))).unwrap();
            std::thread::sleep(Duration::from_millis(30));
        }
        stop.store(true, Ordering::Relaxed);
        for f in flooding {
            f.join().unwrap();
        }
        settled_after_the_flood(&watch, &rx);
        for i in 1..=20 {
            settle(&rx);
            let _ = watch.listing_due();
            std::fs::write(out.join(format!("d{i}/app_1.log")), "").unwrap();
            assert!(quiet(&rx), "a member's name made in d{i}, moved out during the flood, wakes nothing");
            assert!(!watch.listing_due(), "and asks for no listing (d{i})");
        }
    }

    /// With a watcher armed a family lists its folder every ten seconds at least, never sooner
    /// than its rescan; with none, every rescan.
    #[test]
    fn a_family_lists_on_the_watcher_s_word_and_every_ten_seconds_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let spec = FamilySpec::new(dir.path(), r"app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
        let (tx, _rx) = channel();
        let mut watch = Watch::new(true, tx.clone());
        watch.keep_on_family(&spec, None);
        assert!(watch.alive());
        assert_eq!(watch.rescan_every(Duration::from_millis(250)), Duration::from_secs(10));
        assert_eq!(watch.rescan_every(Duration::from_secs(30)), Duration::from_secs(30));
        let off = Watch::new(false, tx);
        assert!(!off.alive());
        assert_eq!(off.rescan_every(Duration::from_millis(250)), Duration::from_millis(250));
    }
}
