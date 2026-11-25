//! A project folder as the workbench's tree (docs/oficina-plan.md, "one project tree covering every
//! component"; brought from the own-shell spike's point 1, `.zen/evidence/P-015.md`).
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

impl NodeId {
    /// The number a view keeps in its index for this node ([`Tree::get`] reads it back).
    pub fn raw(self) -> usize {
        self.0
    }
}

/// A folder's entries as the disk listed them, in the tree's order, before they enter the tree.
#[derive(Clone, Debug)]
pub struct Listing {
    folder: NodeId,
    entries: Vec<(OsString, Entry)>,
}

impl Listing {
    pub fn folder(&self) -> NodeId {
        self.folder
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One change of a folder's entries, at a row as the rows stand when it applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The entry at `row` left.
    Remove { row: usize },
    /// The listing's entry `at` comes in at `row`.
    Insert { row: usize, at: usize },
    /// The entry at `row` stayed and is now what the listing's entry `at` says.
    Update { row: usize, at: usize },
}

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
    /// How many times a folder was listed from the disk ([`Tree::reads`]).
    reads: u64,
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
        let mut tree = Tree { root: root.to_path_buf(), nodes: vec![node], reads: 0 };
        tree.expand(tree.root())?;
        Ok(tree)
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// How many times a folder has been listed from the disk since the tree opened, the root's
    /// first reading among them: a folder read when it opens adds one, and nothing else reads.
    pub fn reads(&self) -> u64 {
        self.reads
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
        if self.children(id).is_none() {
            let listing = self.read_folder(id)?;
            self.attach(listing);
        }
        Ok(self.children(id).unwrap_or(&[]))
    }

    /// Read an opened folder again: an entry still there under the same name, still a folder or
    /// still not, keeps its id (a file is named again by its stamp, a folder keeps what was read
    /// of it); the others leave, with everything under them, and new entries come. A folder not
    /// read yet is read, every entry new.
    pub fn refresh(&mut self, id: NodeId) -> Result<Refreshed, TreeError> {
        let listing = self.read_folder(id)?;
        if self.children(id).is_none() {
            let added = self.attach(listing).to_vec();
            return Ok(Refreshed { removed: Vec::new(), added });
        }
        let mut out = Refreshed::default();
        for step in self.steps(&listing) {
            let c = self.apply(&listing, &step);
            match step {
                Step::Remove { .. } => out.removed.push(c),
                Step::Insert { .. } => out.added.push(c),
                Step::Update { .. } => {}
            }
        }
        out.removed.sort();
        Ok(out)
    }

    /// A folder's entries as the disk lists them now, in the tree's order, not yet in the tree.
    /// A folder never read that cannot be listed is marked so; one already read keeps what was
    /// read of it. Either way the error is returned.
    pub fn read_folder(&mut self, id: NodeId) -> Result<Listing, TreeError> {
        self.live(id)?;
        if self.nodes[id.0].entry != Entry::Folder {
            return Err(TreeError::NotAFolder { path: self.path(id).display().to_string() });
        }
        self.reads += 1;
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
        match listed {
            Ok(mut entries) => {
                entries.sort_by_cached_key(|(os_name, entry)| order_key(os_name, entry));
                Ok(Listing { folder: id, entries })
            }
            Err((path, why)) => {
                if self.children(id).is_none() {
                    self.nodes[id.0].contents = Contents::Unreadable(why.clone());
                }
                Err(TreeError::Read { path, why })
            }
        }
    }

    /// Put a listing in the tree as its folder's entries, all at once: for a folder not read yet
    /// (a view announces the rows `0..listing.len()` before and after).
    pub fn attach(&mut self, listing: Listing) -> &[NodeId] {
        let id = listing.folder;
        let ids = listing.entries.into_iter().map(|(os_name, entry)| self.push(id, os_name, entry)).collect();
        self.nodes[id.0].contents = Contents::Read(ids);
        self.children(id).unwrap_or(&[])
    }

    /// The steps that turn a read folder's entries into a new listing of it, in the order they
    /// apply, each naming the row it touches as the rows stand when it applies: the entries that
    /// left, from the last row up; then the new ones, from the first row down; then the entries
    /// that stayed and changed what they are. An entry stays when its name is the same and it is
    /// still a folder or still not; a folder that stays keeps what was read of it. A view applies
    /// them one at a time, announcing each, and so keeps its open folders and its selection.
    pub fn steps(&self, listing: &Listing) -> Vec<Step> {
        let old = self.children(listing.folder).unwrap_or(&[]);
        let same = |c: &NodeId, (os_name, entry): &(OsString, Entry)| {
            let n = &self.nodes[c.0];
            n.os_name == *os_name && (n.entry == Entry::Folder) == (*entry == Entry::Folder)
        };
        // Each listed entry's old node, if it stayed; the order of what stayed is the order it had.
        let mut kept = vec![None; listing.entries.len()];
        let mut stays = vec![false; old.len()];
        for (i, e) in listing.entries.iter().enumerate() {
            if let Some(row) = old.iter().enumerate().position(|(r, c)| !stays[r] && same(c, e)) {
                stays[row] = true;
                kept[i] = Some(old[row]);
            }
        }
        let mut steps: Vec<Step> = (0..old.len()).rev().filter(|r| !stays[*r]).map(|row| Step::Remove { row }).collect();
        steps.extend((0..listing.entries.len()).filter(|i| kept[*i].is_none()).map(|i| Step::Insert { row: i, at: i }));
        steps.extend((0..listing.entries.len()).filter(|i| {
            kept[*i].is_some_and(|c| {
                let entry = &listing.entries[*i].1;
                *entry != Entry::Folder && self.nodes[c.0].entry != *entry
            })
        }).map(|i| Step::Update { row: i, at: i }));
        steps
    }

    /// Apply one step of [`Tree::steps`], returning the node it touched.
    pub fn apply(&mut self, listing: &Listing, step: &Step) -> NodeId {
        let id = listing.folder;
        let Contents::Read(mut ids) = std::mem::replace(&mut self.nodes[id.0].contents, Contents::NotRead) else {
            panic!("a step applies to a folder already read");
        };
        let touched = match *step {
            Step::Remove { row } => {
                let c = ids.remove(row);
                self.bury(c);
                c
            }
            Step::Insert { row, at } => {
                let (os_name, entry) = listing.entries[at].clone();
                let c = self.push(id, os_name, entry);
                ids.insert(row, c);
                c
            }
            Step::Update { row, at } => {
                let c = ids[row];
                self.nodes[c.0].entry = listing.entries[at].1.clone();
                c
            }
        };
        self.nodes[id.0].contents = Contents::Read(ids);
        touched
    }

    /// The node an index names, if it names one that is still in the tree.
    pub fn get(&self, raw: usize) -> Option<NodeId> {
        self.nodes.get(raw).filter(|n| !n.gone).map(|_| NodeId(raw))
    }

    fn live(&self, id: NodeId) -> Result<(), TreeError> {
        match self.nodes.get(id.0) {
            Some(n) if !n.gone => Ok(()),
            _ => Err(TreeError::Gone(id)),
        }
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
}

/// Folders first, then by name with case folded, then by name as written, so the order is one on
/// every system.
fn order_key(name: &OsString, entry: &Entry) -> (bool, String, String, OsString) {
    let shown = name.to_string_lossy();
    (*entry != Entry::Folder, shown.to_lowercase(), shown.into_owned(), name.clone())
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
