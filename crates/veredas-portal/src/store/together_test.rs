//! A record's batch reaches its database whole or not at all (contract: the recording clause, "in
//! one batch"): several base tables' rows appended in one transaction, a failure on any table
//! rolling every table's rows back. In the crate, since the method is the crate's own.

use veredas_core::{SourceId, Value};

use super::{BaseTableSpec, DuckStore, Residency, RowsQuery, StoreError, WideRow};
use veredas_core::SeriesType;

fn spec(name: &str) -> BaseTableSpec {
    BaseTableSpec { name: name.into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false }
}

fn rows(n: u64) -> Vec<WideRow> {
    (0..n).map(|i| WideRow { source: SourceId::new("s"), key: None, index_ms: i as i64, arrival_ms: i as i64, seq: i, values: vec![Some(Value::Float(i as f64))] }).collect()
}

fn count(store: &DuckStore, table: &str) -> u64 {
    store.query_rows(table, &RowsQuery { limit: 1, ..RowsQuery::default() }).unwrap().total
}

#[test]
fn a_batch_of_several_tables_lands_whole_or_not_at_all() {
    let store = DuckStore::open(&Residency::Ram, &[spec("a"), spec("b")], &[]).unwrap();
    let (a, b) = (rows(3), rows(2));
    // The second table unknown: the first table's rows, already appended, rolled back with it.
    match store.append_wide_together(&[(0, &a), (7, &b)]) {
        Err(StoreError::UnknownTable(t)) => assert_eq!(t, "#7"),
        other => panic!("{other:?}"),
    }
    assert_eq!((count(&store, "a"), count(&store, "b")), (0, 0), "nothing of the batch stays");
    // Whole: both tables, and the store usable after the rollback; an empty table's part skipped.
    store.append_wide_together(&[(0, &a), (1, &b), (1, &[])]).unwrap();
    assert_eq!((count(&store, "a"), count(&store, "b")), (3, 2));
    store.append_wide_together(&[]).unwrap();
    assert_eq!((count(&store, "a"), count(&store, "b")), (3, 2));
}
