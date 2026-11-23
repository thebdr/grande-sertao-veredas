//! A page and its total are one moment of the store (contract: the store clause, a table's page
//! "with its total"): a row the ingest commits between the count and the rows is in neither. In
//! the crate, since the moment between the two is the store's own.

use veredas_core::{SeriesType, SourceId, Value};

use super::{BaseTableSpec, DuckStore, Residency, RowsQuery, WideRow};

fn row(i: i64) -> WideRow {
    WideRow { source: SourceId::new("s"), key: None, index_ms: i, arrival_ms: i, seq: i as u64, values: vec![Some(Value::Float(i as f64))] }
}

#[test]
fn a_row_committed_between_the_count_and_the_page_is_in_neither() {
    let store = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "t".into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false }], &[]).unwrap();
    store.append_wide(0, &(0..100).map(row).collect::<Vec<_>>()).unwrap();
    // The last page: rows 91 to 100 of 100, a row committed after the count.
    let mut next = 100;
    let mut commit = || {
        store.append_wide(0, &[row(next)]).unwrap();
        next += 1;
    };
    let q = RowsQuery { limit: 20, offset: 90, ..RowsQuery::default() };
    let page = store.query_rows_between("t", &q, &mut commit).unwrap();
    assert_eq!((page.total, page.rows.len()), (100, 10), "the page and its total of one moment");
    // An offset past the end: an empty page and the total it was read with.
    let q = RowsQuery { limit: 20, offset: 101, ..RowsQuery::default() };
    let page = store.query_rows_between("t", &q, &mut commit).unwrap();
    assert_eq!((page.total, page.rows.len()), (101, 0));
    // The rows committed meanwhile are there for the next page.
    let page = store.query_rows("t", &RowsQuery { limit: 500, ..RowsQuery::default() }).unwrap();
    assert_eq!((page.total, page.rows.len()), (102, 102));
}

/// A view over a file a logger still writes to: the file is read again at every statement, which
/// the transaction does not freeze, so the view's window is read once for its page, and a line the
/// logger adds between the count and the rows is in neither.
#[test]
fn a_line_a_logger_adds_between_the_count_and_the_page_is_in_neither() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.csv");
    let mut body = String::from("t,v\n");
    for i in 0..100 {
        body.push_str(&format!("{i},{i}\n"));
    }
    std::fs::write(&path, body).unwrap();
    let scan = super::csv_scan_sql(&path.display().to_string(), &super::CsvOptions::default());
    let defs = [super::TableDef { name: "logv".into(), sql: format!("SELECT * FROM {scan}") }];
    let store = DuckStore::open(&Residency::Ram, &[], &defs).unwrap();
    let mut next = 100;
    let mut write = || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{next},{next}").unwrap();
        next += 1;
    };
    let q = RowsQuery { limit: 20, offset: 90, ..RowsQuery::default() };
    let page = store.query_rows_between("logv", &q, &mut write).unwrap();
    assert_eq!((page.total, page.rows.len()), (100, 10), "the page and its total of one read of the file");
    let q = RowsQuery { limit: 20, offset: 101, ..RowsQuery::default() };
    let page = store.query_rows_between("logv", &q, &mut write).unwrap();
    assert_eq!((page.total, page.rows.len()), (101, 0));
    let page = store.query_rows("logv", &RowsQuery { limit: 500, ..RowsQuery::default() }).unwrap();
    assert_eq!((page.total, page.rows.len()), (102, 102));
}

/// A panic inside a page's transaction (here between its count and its rows) rolls it back: the
/// next page on the same connection reads, never "the transaction is aborted".
#[test]
fn a_panic_inside_a_page_leaves_the_connection_usable() {
    let store = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "t".into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false }], &[]).unwrap();
    store.append_wide(0, &(0..3).map(row).collect::<Vec<_>>()).unwrap();
    let q = RowsQuery { limit: 10, ..RowsQuery::default() };
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.query_rows_between("t", &q, &mut || panic!("between the count and the rows"))));
    assert!(panicked.is_err());
    let page = store.query_rows("t", &q).unwrap();
    assert_eq!((page.total, page.rows.len()), (3, 3));
}

/// A time of day in nanoseconds, a type the engine's wrapper cannot read, panics in it (as it did
/// before the page was a transaction): the panic leaves the page's transaction ended, and the pages
/// after it read on the same connection, the same panic again.
#[test]
fn a_value_the_wrapper_cannot_read_ends_its_transaction_and_the_next_page_reads() {
    let defs = [super::TableDef { name: "times".into(), sql: "SELECT 1::BIGINT AS index_ms, CAST('12:34:56.123456789' AS TIME_NS) AS at".into() }];
    let store = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "t".into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false }], &defs).unwrap();
    store.append_wide(0, &[row(1)]).unwrap();
    let q = RowsQuery { limit: 10, ..RowsQuery::default() };
    let why = |r: std::thread::Result<Result<super::RowsPage, super::StoreError>>| {
        let cause = r.expect_err("the wrapper panics on the nanosecond time");
        cause.downcast_ref::<String>().cloned().or_else(|| cause.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
    };
    let first = why(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.query_rows("times", &q))));
    assert!(first.contains("Time64(ns)"), "{first}");
    let page = store.query_rows("t", &q).unwrap();
    assert_eq!((page.total, page.rows.len()), (1, 1));
    let again = why(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.query_rows("times", &q))));
    assert_eq!(again, first, "the same panic again, nothing left open");
}
