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
