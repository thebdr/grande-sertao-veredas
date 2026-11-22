//! A project folder as the workbench's tree (docs/oficina-plan.md, "one project tree covering every
//! component"; the spike's point 1).
//!
//! The tree reads lazily, one folder at a time, when the view opens it ([`Tree::expand`]), so a
//! folder of a hundred thousand log files costs nothing until someone opens it. A TOML file is
//! named by its stamp, read by core's one header reader (`veredas_core::schema`), and so by the
//! component whose reader states that format; a file core's reader refuses carries the reader's
//! own words. Reading is all the tree does: it runs nothing, follows no link (a link is shown with
//! where it points), and honours no configuration it finds in the folder.
//!
//! Every node keeps its id for as long as its entry stays in its folder, across
//! [`Tree::refresh`], so a view's selection and open folders survive a re-read. A name is shown
//! as text (a name that is not UTF-8 shows U+FFFD where its bytes fail) while the node keeps the
//! exact name to reach the file.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;
use veredas_core::schema;

use crate::formats::{Component, component_of};

/// A TOML file larger than this is not read for its stamp: no file of the suite comes near it,
/// and the tree reads a folder's files when the folder opens.
pub const STAMP_READ_LIMIT: u64 = 1 << 20;

/// A node, for as long as its entry stays in its folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

/// What a node is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Folder,
    File(FileKind),
    /// A symbolic link, shown with the text it points to and never followed.
    Link { target: String },
    /// Neither a folder, a file nor a link (a socket, a device): shown, never read.
    Special,
}

/// What a file is, as far as its name and stamp say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A TOML file whose stamp core's reader reads; `component` is whose reader states the format.
    Stamped { format: String, version: u32, component: Option<Component> },
    /// A TOML file core's reader refuses, with the reader's words (`no `format`: a TOML file
    /// states `format` and `version``).
    Refused { why: String },
    /// A TOML file larger than [`STAMP_READ_LIMIT`], not read.
    TooLarge { bytes: u64 },
    /// A TOML file that could not be read.
    Unreadable { why: String },
    /// `Cargo.toml`: cargo's manifest, which carries no stamp.
    Manifest,
    /// Any other file.
    Other,
}

/// What the tree knows of a folder's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Contents {
    /// Not opened yet.
    NotRead,
    /// Its entries, folders first, then by name.
    Read(Vec<NodeId>),
    /// The folder could not be listed, with the system's words.
    Unreadable(String),
}

#[derive(Clone, Debug)]
pub struct Node {
    /// The name as shown.
    pub name: String,
    /// The name begins with a dot.
    pub hidden: bool,
    pub parent: Option<NodeId>,
    pub entry: Entry,
    /// For a folder, what is known of its entries; `NotRead` for anything else.
    pub contents: Contents,
    os_name: OsString,
    gone: bool,
}

/// What a refresh changed in a folder: the ids that left it and the ids that came.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    pub removed: Vec<NodeId>,
    pub added: Vec<NodeId>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TreeError {
    #[error("`{path}` is not a folder")]
    NotAFolder { path: String },
    #[error("`{path}` cannot be read: {why}")]
    Read { path: String, why: String },
    #[error("no node {0:?}: it left its folder")]
    Gone(NodeId),
}

/// A project folder, read as far as it was opened.
#[derive(Clone, Debug)]
pub struct Tree {
    root: PathBuf,
    nodes: Vec<Node>,
}

impl Tree {
    /// Open the folder at `root` and read its own entries.
    pub fn open(root: &Path) -> Result<Tree, TreeError> {
        let shown = root.display().to_string();
        let meta = fs::metadata(root).map_err(|e| TreeError::Read { path: shown.clone(), why: e.to_string() })?;
        if !meta.is_dir() {
            return Err(TreeError::NotAFolder { path: shown });
        }
        let os_name = root.file_name().map_or_else(|| root.as_os_str().to_owned(), ToOwned::to_owned);
        let name = os_name.to_string_lossy().into_owned();
        let node = Node { hidden: false, name, parent: None, entry: Entry::Folder, contents: Contents::NotRead, os_name, gone: false };
        let mut tree = Tree { root: root.to_path_buf(), nodes: vec![node] };
        tree.expand(tree.root())?;
        Ok(tree)
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// A folder's entries, once it has been read.
    pub fn children(&self, id: NodeId) -> Option<&[NodeId]> {
        match &self.nodes[id.0].contents {
            Contents::Read(ids) => Some(ids),
            _ => None,
        }
    }

    /// Where a node sits among its folder's entries.
    pub fn row(&self, id: NodeId) -> Option<usize> {
        let parent = self.nodes[id.0].parent?;
        self.children(parent)?.iter().position(|c| *c == id)
    }

    /// The node's path on the disk, from its exact names.
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        let mut at = id;
        while let Some(parent) = self.nodes[at.0].parent {
            names.push(&self.nodes[at.0].os_name);
            at = parent;
        }
        let mut path = self.root.clone();
        for n in names.into_iter().rev() {
            path.push(n);
        }
        path
    }

    /// Read a folder's entries, once: a folder already read is left as it is.
    pub fn expand(&mut self, id: NodeId) -> Result<&[NodeId], TreeError> {
        self.live(id)?;
        if self.nodes[id.0].entry != Entry::Folder {
            return Err(TreeError::NotAFolder { path: self.path(id).display().to_string() });
        }
        if !matches!(self.nodes[id.0].contents, Contents::Read(_)) {
            let listed = self.list(id)?;
            let ids = listed.into_iter().map(|(os_name, entry)| self.push(id, os_name, entry)).collect();
            self.nodes[id.0].contents = Contents::Read(ids);
            self.sort(id);
        }
        Ok(self.children(id).unwrap_or(&[]))
    }

    /// Read an opened folder again: an entry still there under the same name, still a folder or
    /// still not, keeps its id (a file is named again by its stamp, a folder keeps what was read
    /// of it); the others leave, with everything under them, and new entries come.
    pub fn refresh(&mut self, id: NodeId) -> Result<Refreshed, TreeError> {
        self.live(id)?;
        let Some(old) = self.children(id).map(<[NodeId]>::to_vec) else {
            self.expand(id)?;
            return Ok(Refreshed { removed: Vec::new(), added: self.children(id).unwrap_or(&[]).to_vec() });
        };
        let listed = self.list(id)?;
        let mut out = Refreshed::default();
        let mut kept = Vec::new();
        let mut left: Vec<NodeId> = old;
        for (os_name, entry) in listed {
            let same = left.iter().position(|c| {
                let n = &self.nodes[c.0];
                n.os_name == os_name && (n.entry == Entry::Folder) == (entry == Entry::Folder)
            });
            match same {
                Some(i) => {
                    let c = left.swap_remove(i);
                    if entry != Entry::Folder {
                        self.nodes[c.0].entry = entry;
                    }
                    kept.push(c);
                }
                None => {
                    let c = self.push(id, os_name, entry);
                    out.added.push(c);
                    kept.push(c);
                }
            }
        }
        for c in &left {
            self.bury(*c);
        }
        left.sort();
        out.removed = left;
        self.nodes[id.0].contents = Contents::Read(kept);
        self.sort(id);
        out.added.sort_by_key(|c| self.row(*c));
        Ok(out)
    }

    fn live(&self, id: NodeId) -> Result<(), TreeError> {
        match self.nodes.get(id.0) {
            Some(n) if !n.gone => Ok(()),
            _ => Err(TreeError::Gone(id)),
        }
    }

    /// The folder's entries as the disk lists them, each named and classified. A folder that
    /// cannot be listed is marked so, and the error returned.
    fn list(&mut self, id: NodeId) -> Result<Vec<(OsString, Entry)>, TreeError> {
        let dir = self.path(id);
        let fail = |e: std::io::Error| (dir.display().to_string(), e.to_string());
        let listed: Result<Vec<(OsString, Entry)>, (String, String)> = (|| {
            let mut out = Vec::new();
            for e in fs::read_dir(&dir).map_err(fail)? {
                let e = e.map_err(fail)?;
                let kind = e.file_type().map_err(fail)?;
                let os_name = e.file_name();
                let entry = if kind.is_symlink() {
                    Entry::Link { target: fs::read_link(e.path()).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default() }
                } else if kind.is_dir() {
                    Entry::Folder
                } else if kind.is_file() {
                    Entry::File(classify(&e.path(), &os_name))
                } else {
                    Entry::Special
                };
                out.push((os_name, entry));
            }
            Ok(out)
        })();
        listed.map_err(|(path, why)| {
            self.nodes[id.0].contents = Contents::Unreadable(why.clone());
            TreeError::Read { path, why }
        })
    }

    fn push(&mut self, parent: NodeId, os_name: OsString, entry: Entry) -> NodeId {
        let name = os_name.to_string_lossy().into_owned();
        let node = Node { hidden: name.starts_with('.'), name, parent: Some(parent), entry, contents: Contents::NotRead, os_name, gone: false };
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    fn bury(&mut self, id: NodeId) {
        self.nodes[id.0].gone = true;
        if let Contents::Read(ids) = std::mem::replace(&mut self.nodes[id.0].contents, Contents::NotRead) {
            for c in ids {
                self.bury(c);
            }
        }
    }

    /// Folders first, then by name with case folded, then by name as written, so the order is
    /// one on every system.
    fn sort(&mut self, id: NodeId) {
        let Contents::Read(mut ids) = std::mem::replace(&mut self.nodes[id.0].contents, Contents::NotRead) else { return };
        ids.sort_by(|a, b| order(&self.nodes[a.0], &self.nodes[b.0]));
        self.nodes[id.0].contents = Contents::Read(ids);
    }
}

fn order(a: &Node, b: &Node) -> Ordering {
    let folder = |n: &Node| n.entry != Entry::Folder;
    folder(a)
        .cmp(&folder(b))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.os_name.cmp(&b.os_name))
}

/// A file named by what it says it is.
fn classify(path: &Path, name: &OsString) -> FileKind {
    let name = name.to_string_lossy();
    if name.eq_ignore_ascii_case("Cargo.toml") {
        return FileKind::Manifest;
    }
    if !name.to_ascii_lowercase().ends_with(".toml") {
        return FileKind::Other;
    }
    let read = || -> std::io::Result<Result<Vec<u8>, u64>> {
        let f = fs::File::open(path)?;
        let bytes = f.metadata()?.len();
        if bytes > STAMP_READ_LIMIT {
            return Ok(Err(bytes));
        }
        let mut buf = Vec::with_capacity(bytes as usize);
        f.take(STAMP_READ_LIMIT + 1).read_to_end(&mut buf)?;
        Ok(Ok(buf))
    };
    let buf = match read() {
        Ok(Ok(buf)) if buf.len() as u64 > STAMP_READ_LIMIT => return FileKind::TooLarge { bytes: buf.len() as u64 },
        Ok(Ok(buf)) => buf,
        Ok(Err(bytes)) => return FileKind::TooLarge { bytes },
        Err(e) => return FileKind::Unreadable { why: e.to_string() },
    };
    let Ok(text) = std::str::from_utf8(&buf) else {
        return FileKind::Refused { why: "not text in UTF-8".to_string() };
    };
    match schema::stamp(text, "a TOML file") {
        Ok(s) => FileKind::Stamped { component: component_of(&s.format), format: s.format, version: s.version },
        Err(e) => FileKind::Refused { why: e.to_string() },
    }
}
