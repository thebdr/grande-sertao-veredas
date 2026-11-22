//! The project tree as a Qt item model (the spike's point 1): the model's [`Tree`] behind a
//! `QAbstractItemModel` written in Rust, read lazily through `canFetchMore` and `fetchMore`, and
//! re-read step by step so a view keeps its open folders and its selection.

use std::cell::RefCell;
use std::path::Path;
use std::pin::Pin;

use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use oficina_spike_model::tree::{Contents, Entry, FileKind, NodeId, Step, Tree};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!(<QtCore/QAbstractItemModel>);
        type QAbstractItemModel;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractItemModel]
        #[qproperty(QString, root_path, cxx_name = "rootPath")]
        #[qproperty(QString, error)]
        type ProjectTree = super::ProjectTreeRust;

        #[cxx_override]
        fn index(self: &ProjectTree, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[cxx_override]
        fn parent(self: &ProjectTree, child: &QModelIndex) -> QModelIndex;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ProjectTree, parent: &QModelIndex) -> i32;
        #[cxx_override]
        #[cxx_name = "columnCount"]
        fn column_count(self: &ProjectTree, parent: &QModelIndex) -> i32;
        #[cxx_override]
        fn data(self: &ProjectTree, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "hasChildren"]
        fn has_children(self: &ProjectTree, parent: &QModelIndex) -> bool;
        #[cxx_override]
        #[cxx_name = "canFetchMore"]
        fn can_fetch_more(self: &ProjectTree, parent: &QModelIndex) -> bool;
        #[cxx_override]
        #[cxx_name = "fetchMore"]
        fn fetch_more(self: Pin<&mut ProjectTree>, parent: &QModelIndex);
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ProjectTree) -> QHash_i32_QByteArray;

        #[inherit]
        #[cxx_name = "createIndex"]
        fn create_index(self: &ProjectTree, row: i32, column: i32, id: usize) -> QModelIndex;
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(self: Pin<&mut ProjectTree>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut ProjectTree>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(self: Pin<&mut ProjectTree>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut ProjectTree>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut ProjectTree>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut ProjectTree>);
        #[qsignal]
        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(self: Pin<&mut ProjectTree>, top_left: &QModelIndex, bottom_right: &QModelIndex, roles: &QList_i32);

        /// Open the folder at `path` as the tree's root; false, with `error` set, if it does not.
        #[qinvokable]
        fn open(self: Pin<&mut ProjectTree>, path: &QString) -> bool;
        /// Read again every folder the tree has read, the root first; false, with `error` set,
        /// if one could not be read.
        #[qinvokable]
        #[cxx_name = "refreshOpen"]
        fn refresh_open(self: Pin<&mut ProjectTree>) -> bool;
    }
}

/// The roles a delegate reads, after Qt's display role.
const KIND: i32 = 256;
const DETAIL: i32 = 257;
const HIDDEN: i32 = 258;
const STATE: i32 = 259;
const NODE: i32 = 260;
const PATH: i32 = 261;

#[derive(Default)]
pub struct ProjectTreeRust {
    tree: RefCell<Option<Tree>>,
    root_path: QString,
    error: QString,
}

/// What a node is, as the delegate words it.
fn describe(entry: &Entry) -> (&'static str, &'static str, String) {
    match entry {
        Entry::Folder => ("folder", "folder", String::new()),
        Entry::Link { target } => ("link", "link", format!("link to {target}")),
        Entry::Special => ("special", "other", String::new()),
        Entry::File(kind) => match kind {
            FileKind::Stamped { format, version, component } => {
                let whose = component.map_or_else(String::new, |c| format!(" · {}", c.display));
                ("file", "stamped", format!("{format} {version}{whose}"))
            }
            FileKind::Refused { why } => ("file", "refused", why.clone()),
            FileKind::TooLarge { bytes } => ("file", "refused", format!("too large to read: {bytes} bytes")),
            FileKind::Unreadable { why } => ("file", "refused", why.clone()),
            FileKind::Manifest => ("file", "other", "cargo manifest".to_string()),
            FileKind::Other => ("file", "other", String::new()),
        },
    }
}

impl qobject::ProjectTree {
    /// The node an index names: the root for an index that names none.
    fn node_of(&self, tree: &Tree, index: &QModelIndex) -> Option<NodeId> {
        if index.is_valid() { tree.get(index.internal_id()) } else { Some(tree.root()) }
    }

    fn index_of(&self, tree: &Tree, id: NodeId) -> QModelIndex {
        match tree.row(id) {
            Some(row) if id != tree.root() => self.create_index(row as i32, 0, id.raw()),
            _ => QModelIndex::default(),
        }
    }

    fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return QModelIndex::default() };
        let child = self
            .node_of(tree, parent)
            .and_then(|p| tree.children(p))
            .and_then(|c| usize::try_from(row).ok().and_then(|r| c.get(r)).copied());
        match child {
            Some(c) if column == 0 => self.create_index(row, 0, c.raw()),
            _ => QModelIndex::default(),
        }
    }

    fn parent(&self, child: &QModelIndex) -> QModelIndex {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return QModelIndex::default() };
        match child.is_valid().then(|| tree.get(child.internal_id())).flatten().and_then(|c| tree.node(c).parent) {
            Some(p) => self.index_of(tree, p),
            None => QModelIndex::default(),
        }
    }

    fn row_count(&self, parent: &QModelIndex) -> i32 {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return 0 };
        self.node_of(tree, parent).and_then(|p| tree.children(p)).map_or(0, |c| c.len() as i32)
    }

    fn column_count(&self, _parent: &QModelIndex) -> i32 {
        1
    }

    fn has_children(&self, parent: &QModelIndex) -> bool {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return false };
        let Some(id) = self.node_of(tree, parent) else { return false };
        let n = tree.node(id);
        n.entry == Entry::Folder
            && match &n.contents {
                Contents::NotRead => true,
                Contents::Read(c) => !c.is_empty(),
                Contents::Unreadable(_) => false,
            }
    }

    fn can_fetch_more(&self, parent: &QModelIndex) -> bool {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return false };
        self.node_of(tree, parent).is_some_and(|id| tree.node(id).entry == Entry::Folder && tree.node(id).contents == Contents::NotRead)
    }

    fn fetch_more(mut self: Pin<&mut Self>, parent: &QModelIndex) {
        let listing = {
            let mut tree = self.tree.borrow_mut();
            let Some(t) = tree.as_mut() else { return };
            let Some(id) = (if parent.is_valid() { t.get(parent.internal_id()) } else { Some(t.root()) }) else { return };
            t.read_folder(id)
        };
        match listing {
            Ok(listing) if listing.is_empty() => {
                if let Some(t) = self.tree.borrow_mut().as_mut() {
                    t.attach(listing);
                }
            }
            Ok(listing) => {
                let last = listing.len() as i32 - 1;
                // SAFETY: the rows announced are the rows attached, between the two calls.
                unsafe { self.as_mut().begin_insert_rows(parent, 0, last) };
                if let Some(t) = self.tree.borrow_mut().as_mut() {
                    t.attach(listing);
                }
                unsafe { self.as_mut().end_insert_rows() };
            }
            Err(e) => self.as_mut().set_error(QString::from(e.to_string().as_str())),
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let tree = self.tree.borrow();
        let Some(tree) = tree.as_ref() else { return QVariant::default() };
        let Some(id) = index.is_valid().then(|| tree.get(index.internal_id())).flatten() else { return QVariant::default() };
        let n = tree.node(id);
        let (kind, state, detail) = describe(&n.entry);
        match role {
            0 => QVariant::from(&QString::from(n.name.as_str())),
            KIND => QVariant::from(&QString::from(kind)),
            DETAIL => QVariant::from(&QString::from(detail.as_str())),
            HIDDEN => QVariant::from(&n.hidden),
            STATE => QVariant::from(&QString::from(state)),
            NODE => QVariant::from(&(id.raw() as i64)),
            PATH => QVariant::from(&QString::from(tree.path(id).to_string_lossy().as_ref())),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [(0, "display"), (KIND, "kind"), (DETAIL, "detail"), (HIDDEN, "hidden"), (STATE, "state"), (NODE, "nodeId"), (PATH, "path")] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    fn open(mut self: Pin<&mut Self>, path: &QString) -> bool {
        let opened = Tree::open(Path::new(&path.to_string()));
        // SAFETY: the model's whole content changes between the two calls.
        unsafe { self.as_mut().begin_reset_model() };
        let ok = match opened {
            Ok(t) => {
                *self.tree.borrow_mut() = Some(t);
                true
            }
            Err(e) => {
                *self.tree.borrow_mut() = None;
                self.as_mut().set_error(QString::from(e.to_string().as_str()));
                false
            }
        };
        unsafe { self.as_mut().end_reset_model() };
        if ok {
            self.as_mut().set_error(QString::default());
            self.as_mut().set_root_path(path.clone());
        }
        ok
    }

    fn refresh_open(mut self: Pin<&mut Self>) -> bool {
        let mut pending = match self.tree.borrow().as_ref() {
            Some(t) => vec![t.root()],
            None => return false,
        };
        let mut ok = true;
        while let Some(id) = pending.pop() {
            let index = match self.tree.borrow().as_ref() {
                Some(t) if t.get(id.raw()).is_some() => self.index_of(t, id),
                _ => continue,
            };
            ok &= self.as_mut().refresh_folder(id, &index);
            if let Some(t) = self.tree.borrow().as_ref() {
                let read = t.children(id).unwrap_or(&[]).iter().filter(|c| t.children(**c).is_some());
                pending.extend(read.rev());
            }
        }
        ok
    }

    /// Read one folder again, each step announced around its change.
    fn refresh_folder(mut self: Pin<&mut Self>, id: NodeId, index: &QModelIndex) -> bool {
        let read = {
            let mut tree = self.tree.borrow_mut();
            let Some(t) = tree.as_mut() else { return false };
            t.read_folder(id).map(|l| {
                let steps = t.steps(&l);
                (l, steps)
            })
        };
        let (listing, steps) = match read {
            Ok(r) => r,
            Err(e) => {
                self.as_mut().set_error(QString::from(e.to_string().as_str()));
                return false;
            }
        };
        for step in steps {
            match step {
                Step::Remove { row } => {
                    // SAFETY: each step announces the one row it changes, around the change.
                    unsafe { self.as_mut().begin_remove_rows(index, row as i32, row as i32) };
                    self.apply(&listing, &step);
                    unsafe { self.as_mut().end_remove_rows() };
                }
                Step::Insert { row, .. } => {
                    unsafe { self.as_mut().begin_insert_rows(index, row as i32, row as i32) };
                    self.apply(&listing, &step);
                    unsafe { self.as_mut().end_insert_rows() };
                }
                Step::Update { row, .. } => {
                    let id = self.apply(&listing, &step);
                    let at = self.create_index(row as i32, 0, id.raw());
                    self.as_mut().data_changed(&at, &at, &QList::default());
                }
            }
        }
        true
    }

    fn apply(&self, listing: &oficina_spike_model::tree::Listing, step: &Step) -> NodeId {
        let mut tree = self.tree.borrow_mut();
        tree.as_mut().expect("a tree is open").apply(listing, step)
    }
}

