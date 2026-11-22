//! One TOML file behind its two views (the spike's point 2): the model's [`Document`] as a list
//! model whose rows are the form's fields, and whose properties carry the text view's text, where
//! a refused text fails, and whether the file changed since it was read or saved.

use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use oficina_spike_model::document::{Document, Field, Scalar};

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
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(QString, path)]
        #[qproperty(QString, text)]
        #[qproperty(QString, stamp)]
        #[qproperty(i32, revision)]
        #[qproperty(bool, modified)]
        #[qproperty(QString, error)]
        #[qproperty(i32, error_line, cxx_name = "errorLine")]
        #[qproperty(i32, error_column, cxx_name = "errorColumn")]
        #[qproperty(i32, error_offset, cxx_name = "errorOffset")]
        #[qproperty(bool, stale)]
        type TomlDocument = super::TomlDocumentRust;

        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TomlDocument, parent: &QModelIndex) -> i32;
        #[cxx_override]
        fn data(self: &TomlDocument, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TomlDocument) -> QHash_i32_QByteArray;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut TomlDocument>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut TomlDocument>);
        #[inherit]
        fn index(self: &TomlDocument, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[qsignal]
        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(self: Pin<&mut TomlDocument>, top_left: &QModelIndex, bottom_right: &QModelIndex, roles: &QList_i32);

        /// Read the file at `path`; false, with `error` set, if it does not open.
        #[qinvokable]
        fn open(self: Pin<&mut TomlDocument>, path: &QString) -> bool;
        /// The form's edit: the field at `row` takes the value its text says.
        #[qinvokable]
        #[cxx_name = "setField"]
        fn set_field(self: Pin<&mut TomlDocument>, row: i32, text: &QString) -> bool;
        /// The text view's edit: the whole text. A text that does not parse leaves the document
        /// and the form as they were, `stale` set and the error placed.
        #[qinvokable]
        #[cxx_name = "editText"]
        fn set_text_view(self: Pin<&mut TomlDocument>, text: &QString) -> bool;
        /// Write the file whole over itself.
        #[qinvokable]
        fn save(self: Pin<&mut TomlDocument>) -> bool;
    }
}

const PATH: i32 = 256;
const KIND: i32 = 257;
const VALUE: i32 = 258;
const RAW: i32 = 259;
const LINE: i32 = 260;
const COMMENT: i32 = 261;
const STAMP: i32 = 262;

#[derive(Default)]
pub struct TomlDocumentRust {
    doc: Option<Document>,
    file: PathBuf,
    fields: Vec<Field>,
    path: QString,
    text: QString,
    stamp: QString,
    revision: i32,
    modified: bool,
    error: QString,
    error_line: i32,
    error_column: i32,
    error_offset: i32,
    stale: bool,
}

fn qs(s: &str) -> QString {
    QString::from(s)
}

impl qobject::TomlDocument {
    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.fields.len() as i32
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(f) = usize::try_from(index.row()).ok().and_then(|r| self.fields.get(r)) else { return QVariant::default() };
        match role {
            0 | PATH => QVariant::from(&qs(&f.path.to_string())),
            KIND => QVariant::from(&qs(f.value.kind().name())),
            VALUE => QVariant::from(&qs(&f.value.to_text())),
            RAW => QVariant::from(&qs(&f.raw)),
            LINE => QVariant::from(&(f.line as i32)),
            COMMENT => QVariant::from(&qs(f.comment.as_deref().unwrap_or(""))),
            STAMP => QVariant::from(&f.stamp),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [(PATH, "path"), (KIND, "kind"), (VALUE, "value"), (RAW, "raw"), (LINE, "line"), (COMMENT, "comment"), (STAMP, "stamp")] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    fn fail(mut self: Pin<&mut Self>, why: &str) -> bool {
        self.as_mut().set_error(qs(why));
        false
    }

    /// The properties and the rows, from the document as it now stands. The rows are reset for a
    /// new file or when their number changed, and announced as changed otherwise, so a form keeps
    /// its place.
    fn show(mut self: Pin<&mut Self>, changed: bool, reset: bool) {
        let Some(doc) = self.doc.as_ref() else { return };
        let fields = doc.fields();
        let text = qs(doc.text());
        let stamp = qs(&doc.stamp().map(|s| format!("{} {}", s.format, s.version)).unwrap_or_default());
        if !reset && fields.len() == self.fields.len() {
            self.as_mut().rust_mut().fields = fields;
            if !self.fields.is_empty() {
                let top = self.index(0, 0, &QModelIndex::default());
                let bottom = self.index(self.fields.len() as i32 - 1, 0, &QModelIndex::default());
                self.as_mut().data_changed(&top, &bottom, &QList::default());
            }
        } else {
            // SAFETY: the rows change whole between the two calls.
            unsafe { self.as_mut().begin_reset_model() };
            self.as_mut().rust_mut().fields = fields;
            unsafe { self.as_mut().end_reset_model() };
        }
        self.as_mut().set_stamp(stamp);
        self.as_mut().set_text(text);
        let revision = self.revision + i32::from(changed);
        self.as_mut().set_revision(revision);
        if changed {
            self.as_mut().set_modified(true);
        }
        self.as_mut().set_error(QString::default());
        self.as_mut().set_error_line(0);
        self.as_mut().set_error_column(0);
        self.as_mut().set_stale(false);
    }

    fn open(mut self: Pin<&mut Self>, path: &QString) -> bool {
        let file = PathBuf::from(path.to_string());
        let opened = std::fs::read(&file).map_err(|e| e.to_string()).and_then(|b| Document::from_bytes(&b).map_err(|e| e.to_string()));
        match opened {
            Ok(doc) => {
                {
                    let mut r = self.as_mut().rust_mut();
                    r.doc = Some(doc);
                    r.file = file;
                }
                self.as_mut().set_path(path.clone());
                self.as_mut().set_revision(0);
                self.as_mut().set_modified(false);
                self.as_mut().show(false, true);
                true
            }
            Err(e) => self.fail(&e),
        }
    }

    fn set_field(mut self: Pin<&mut Self>, row: i32, text: &QString) -> bool {
        let Some(f) = usize::try_from(row).ok().and_then(|r| self.fields.get(r)).cloned() else { return self.fail("no such field") };
        let value = match Scalar::parse(f.value.kind(), &text.to_string()) {
            Ok(v) => v,
            Err(why) => return self.fail(&format!("`{}`: {why}", f.path)),
        };
        let before = self.doc.as_ref().map(Document::revision);
        let set = match self.as_mut().rust_mut().doc.as_mut() {
            Some(doc) => doc.set(&f.path, value),
            None => return self.fail("no file is open"),
        };
        match set {
            Ok(()) => {
                let changed = self.doc.as_ref().map(Document::revision) != before;
                self.as_mut().show(changed, false);
                true
            }
            Err(e) => self.fail(&e.to_string()),
        }
    }

    fn set_text_view(mut self: Pin<&mut Self>, text: &QString) -> bool {
        let before = self.doc.as_ref().map(Document::revision);
        let set = match self.as_mut().rust_mut().doc.as_mut() {
            Some(doc) => doc.set_text(&text.to_string()),
            None => return self.fail("no file is open"),
        };
        match set {
            Ok(()) => {
                let changed = self.doc.as_ref().map(Document::revision) != before;
                self.as_mut().show(changed, false);
                true
            }
            Err(e) => {
                // Where the view's cursor counts it: UTF-16 units into the text it holds.
                let typed = text.to_string().replace("\r\n", "\n");
                let offset = typed.get(..e.offset).map_or(0, |b| b.encode_utf16().count());
                self.as_mut().set_error_offset(offset as i32);
                self.as_mut().set_error_line(e.line as i32);
                self.as_mut().set_error_column(e.column as i32);
                self.as_mut().set_stale(true);
                self.fail(&e.to_string())
            }
        }
    }

    fn save(mut self: Pin<&mut Self>) -> bool {
        let written = match self.doc.as_ref() {
            Some(doc) => doc.write_to(&self.file),
            None => return self.fail("no file is open"),
        };
        match written {
            Ok(()) => {
                self.as_mut().set_modified(false);
                true
            }
            Err(e) => self.fail(&e.to_string()),
        }
    }
}
