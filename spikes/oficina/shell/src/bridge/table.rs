//! A million rows from a Rust model (the spike's point 3): the model's [`Synthetic`] table behind
//! a `QAbstractTableModel` written in Rust; the view asks for the cells it draws and nothing else.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{Orientation, QModelIndex, QString, QVariant};
use oficina_spike_model::table::Synthetic;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qt.h");
        #[namespace = "Qt"]
        type Orientation = cxx_qt_lib::Orientation;
        include!(<QtCore/QAbstractTableModel>);
        type QAbstractTableModel;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractTableModel]
        #[qproperty(i32, rows)]
        type TableSource = super::TableSourceRust;

        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TableSource, parent: &QModelIndex) -> i32;
        #[cxx_override]
        #[cxx_name = "columnCount"]
        fn column_count(self: &TableSource, parent: &QModelIndex) -> i32;
        #[cxx_override]
        fn data(self: &TableSource, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "headerData"]
        fn header_data(self: &TableSource, section: i32, orientation: Orientation, role: i32) -> QVariant;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut TableSource>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut TableSource>);

        /// Show a table of `rows` rows.
        #[qinvokable]
        #[cxx_name = "showRows"]
        fn show_rows(self: Pin<&mut TableSource>, rows: i32);
    }
}

/// The seed of every table the spike shows, so a test knows its cells.
pub const SEED: u64 = 2026;

pub struct TableSourceRust {
    table: Synthetic,
    rows: i32,
}

impl Default for TableSourceRust {
    fn default() -> Self {
        TableSourceRust { table: Synthetic::new(0, SEED), rows: 0 }
    }
}

impl qobject::TableSource {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { self.table.rows() as i32 }
    }

    fn column_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { self.table.columns() as i32 }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if role != 0 {
            return QVariant::default();
        }
        let (Ok(row), Ok(column)) = (usize::try_from(index.row()), usize::try_from(index.column())) else { return QVariant::default() };
        match self.table.cell(row, column) {
            Some(text) => QVariant::from(&QString::from(text.as_str())),
            None => QVariant::default(),
        }
    }

    fn header_data(&self, section: i32, orientation: Orientation, role: i32) -> QVariant {
        if role != 0 {
            return QVariant::default();
        }
        let header = match orientation {
            Orientation::Horizontal => usize::try_from(section).ok().and_then(|c| self.table.header(c)).map(str::to_string),
            _ => Some(section.to_string()),
        };
        header.map_or_else(QVariant::default, |h| QVariant::from(&QString::from(h.as_str())))
    }

    fn show_rows(mut self: Pin<&mut Self>, rows: i32) {
        // SAFETY: the whole table changes between the two calls.
        unsafe { self.as_mut().begin_reset_model() };
        self.as_mut().rust_mut().table = Synthetic::new(usize::try_from(rows).unwrap_or(0), SEED);
        unsafe { self.as_mut().end_reset_model() };
        self.as_mut().set_rows(rows);
    }
}
