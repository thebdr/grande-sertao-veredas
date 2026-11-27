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

/// Where a symbolic link at `path` leads, resolved whole; `None` for a file that is no link, or a
/// link that leads nowhere for the moment.
fn link_target(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_symlink() {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

/// What a watch wakes its loop for, by the name an event in a watched folder carries.
enum Interest {
    /// A followed file: its name, and its link's target's name.
    Names(Vec<OsString>),
    /// A family: the names its pattern matches and its fixed active's, relative to its folder; in
    /// a recursive one, any folder made, removed or renamed in the tree too.
    Family(FamilySpec),
}

impl Interest {
    /// Whether an event naming `name` in the watched folder at `rel` (relative to the family's
    /// folder, empty at its top) concerns what is followed. A name never UTF-8 is no member's.
    fn wants_name(&self, rel: &str, name: &OsStr, is_dir: bool) -> bool {
        match self {
            Interest::Names(names) => names.iter().any(|n| n == name),
            Interest::Family(spec) => {
                if spec.recursive() && is_dir {
                    return true;
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

    /// Whether a watcher is armed: a family then lists its folder on the watcher's word.
    pub(crate) fn alive(&self) -> bool {
        self.armed.as_ref().is_some_and(|a| a.backend.is_some())
    }

    /// Whether the watcher reported a name made, removed or renamed since the last call (or was
    /// armed anew, after a while nothing watched): the folder is to be listed.
    pub(crate) fn listing_due(&self) -> bool {
        self.renamed.swap(false, Ordering::AcqRel)
    }

    /// How long a family waits between two listings nothing asked for.
    pub(crate) fn rescan_every(&self, configured: Duration) -> Duration {
        if self.alive() { configured.max(WATCHED_RESCAN) } else { configured }
    }

    /// Keep a file tail's watch: on the followed path's folder and, for a link, on its target's,
    /// and on the file at the path for its writes. Armed once the folder is there and again when
    /// it is made anew or the link leads elsewhere. Called at every pass, before the read, so a
    /// write after it wakes the next wait.
    pub(crate) fn keep_on_file(&mut self, path: &Path) {
        if !self.wanted {
            return;
        }
        let folder = folder_of(path);
        let id = folder_id(&folder);
        let target = link_target(path);
        let same = id.is_some() && self.armed.as_ref().is_some_and(|a| a.folder == id && a.target == target);
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
            self.armed = Some(Armed { backend, folder: id, target });
            // Whatever happened while nothing watched is listed and read at this pass.
            self.renamed.store(true, Ordering::Release);
        }
        if let Some(b) = self.armed.as_mut().and_then(|a| a.backend.as_mut()) {
            b.keep_file(Some(path));
        }
    }

    /// Keep a family's watch on its folder (its tree when recursive), armed once the folder is
    /// there and again when it is made anew, and on the file the follower reads now (`active`), for
    /// its writes. Called before the attach and at every pass of the live loop.
    pub(crate) fn keep_on_family(&mut self, spec: &FamilySpec, active: Option<&Path>) {
        if !self.wanted {
            return;
        }
        let id = folder_id(spec.dir());
        if !(id.is_some() && self.armed.as_ref().is_some_and(|a| a.folder == id)) {
            self.armed = None;
            if id.is_none() {
                return;
            }
            let backend = backend::Backend::arm(&[absolute(spec.dir())], spec.recursive(), Interest::Family(spec.clone()), self.wake.clone(), Arc::clone(&self.renamed));
            self.armed = Some(Armed { backend, folder: id, target: None });
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
    use crate::tail::cursor::{Identity, identity_at};

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
    }

    pub(crate) struct Backend {
        inner: Arc<Inner>,
        stop: OwnedFd,
        reader: Option<JoinHandle<()>>,
        /// The file watched for its writes and its identity then.
        file: Option<(i32, Identity)>,
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

        /// Watch a folder for its names and, recursive, every folder under it to the depth listed.
        fn add_tree(&self, path: &Path, rel: &str, depth: usize) -> bool {
            let Some(wd) = self.add(path, FOLDER_MASK) else { return false };
            if let Ok(mut roles) = self.roles.lock() {
                roles.insert(wd, Role::Folder(path.to_path_buf(), rel.to_string(), depth));
            }
            if self.recursive && depth < MAX_DEPTH
                && let Ok(entries) = std::fs::read_dir(path)
            {
                for e in entries.flatten() {
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        let name = e.file_name();
                        let Some(n) = name.to_str() else { continue };
                        let sub = if rel.is_empty() { n.to_string() } else { format!("{rel}/{n}") };
                        self.add_tree(&e.path(), &sub, depth + 1);
                    }
                }
            }
            true
        }

        /// Read one batch of events: whether to wake the loop, whether a name may have changed. A
        /// name the batch has just judged in the same folder is not judged again: a neighbour
        /// made and removed over and over is one pattern match a batch.
        fn handle(&self, bytes: &[u8]) -> (bool, bool) {
            let (mut wake, mut renamed) = (false, false);
            let mut new_dirs: Vec<(PathBuf, String, usize)> = Vec::new();
            let mut last: Option<(i32, &[u8], bool, bool)> = None;
            {
                let Ok(mut roles) = self.roles.lock() else { return (true, true) };
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
                        (wake, renamed) = (true, true);
                        continue;
                    }
                    if mask & libc::IN_IGNORED != 0 {
                        // A watch gone with what it watched: a folder removed, the file's last link.
                        if let Some(Role::Folder(..)) = roles.remove(&wd) {
                            renamed = true;
                        }
                        wake = true;
                        continue;
                    }
                    match roles.get(&wd) {
                        None => {}
                        Some(Role::File) => wake = true,
                        Some(Role::Folder(path, rel, depth)) => {
                            if name.is_empty() {
                                // The folder itself, removed or renamed.
                                (wake, renamed) = (true, true);
                                continue;
                            }
                            let is_dir = mask & libc::IN_ISDIR != 0;
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
            }
            // A folder born in a recursive tree is watched, and its members found by the listing
            // its birth asks for (they may have been made before its watch).
            for (path, rel, depth) in new_dirs {
                self.add_tree(&path, &rel, depth);
            }
            (wake, renamed)
        }
    }

    fn read_loop(inner: Arc<Inner>, stop: OwnedFd, wake: Sender<()>, renamed: Arc<AtomicBool>) {
        // Ten milliseconds of a busy folder's names fit in one read.
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut fds = [
                libc::pollfd { fd: inner.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: stop.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            ];
            // SAFETY: two pollfd entries in a live array.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
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
            let (w, r) = inner.handle(buf.get(..got as usize).unwrap_or_default());
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
            let inner = Arc::new(Inner { fd, interest, recursive, roles: Mutex::new(HashMap::new()) });
            for f in folders {
                if !inner.add_tree(f, "", 0) {
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

        /// Watch the file at `path` for its writes, moved to the file there now when it is another
        /// one; tried again at every pass while it cannot be watched (not there yet, unreadable).
        pub(crate) fn keep_file(&mut self, path: Option<&Path>) {
            let id = path.and_then(identity_at);
            if id.is_some() && self.file.as_ref().map(|(_, f)| *f) == id {
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
            let (Some(path), Some(id)) = (path, id) else { return };
            if let Some(wd) = self.inner.add(path, FILE_MASK) {
                if let Ok(mut roles) = self.inner.roles.lock() {
                    // The same inode as a watched folder never: a file is no folder.
                    roles.entry(wd).or_insert(Role::File);
                }
                self.file = Some((wd, id));
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

    pub(crate) struct Backend {
        _watcher: RecommendedWatcher,
    }

    /// An event the follower makes itself: a file opened, read, or closed after reading.
    fn woken_by_self(kind: &EventKind) -> bool {
        matches!(kind, EventKind::Access(a) if !matches!(a, AccessKind::Close(AccessMode::Write)))
    }

    /// An event that may change the names a folder holds, or one the watcher cannot tell.
    fn may_rename(kind: &EventKind) -> bool {
        matches!(kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) | EventKind::Any | EventKind::Other)
    }

    fn may_be_folder(kind: &EventKind) -> bool {
        use notify::event::{CreateKind, RemoveKind};
        matches!(
            kind,
            EventKind::Create(CreateKind::Folder | CreateKind::Any | CreateKind::Other)
                | EventKind::Remove(RemoveKind::Folder | RemoveKind::Any | RemoveKind::Other)
                | EventKind::Modify(ModifyKind::Name(_))
                | EventKind::Any
                | EventKind::Other
        )
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
                    Some(parts) => spec.names(&parts.join("/")) || may_be_folder(kind),
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
            for f in folders {
                watcher.watch(f, mode).ok()?;
            }
            Some(Backend { _watcher: watcher })
        }

        /// The folder watch reports the file's writes already.
        pub(crate) fn keep_file(&mut self, _path: Option<&Path>) {}
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    //! The watch's filter, pinned directly: what wakes the loop and what asks for a listing.

    use std::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::family::ActiveMode;

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
        watch.keep_on_file(&path);
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
        // The watch moves to the file now at the name: its writes wake.
        watch.keep_on_file(&path);
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
        watch.keep_on_family(&spec, Some(&active));
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
