//! A recorder's batch, through the recorder itself (contract: the recording clause, "every table's
//! rows in one transaction, so a batch lands whole or not at all; a write that fails stops the
//! recording, the batch counted unrecorded"). In the crate, since the recorder is the session's
//! own: a recorder whose second table its record lacks fails its write at once, no drive fault
//! needed.

use std::time::{Duration, Instant};

use veredas_core::{SeriesType, SourceId, Value};

use super::{Recorder, RecordingStatus};
use crate::store::{BaseTableSpec, DuckStore, Residency, RowsQuery, WideRow};

fn row() -> WideRow {
    WideRow { source: SourceId::new("s"), key: None, index_ms: 1, arrival_ms: 1, seq: 1, values: vec![Some(Value::Float(1.5))] }
}

fn numbered(seq: u64) -> WideRow {
    WideRow { source: SourceId::new("s"), key: None, index_ms: seq as i64, arrival_ms: seq as i64, seq, values: vec![Some(Value::Float(seq as f64 * 1.5))] }
}

/// The record's rows of `a`, as (seq, value), in seq order.
fn rows_of(r: &Recorder) -> Vec<(u64, f64)> {
    let page = r.store.query_rows("a", &RowsQuery { limit: 100, ..RowsQuery::default() }).unwrap();
    let at = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap();
    let mut out: Vec<(u64, f64)> = page.rows.iter().map(|row| (row[at("seq")].as_u64().unwrap(), row[at("v")].as_f64().unwrap())).collect();
    out.sort_by_key(|(seq, _)| *seq);
    out
}

fn held(r: &Recorder) -> u64 {
    r.store.query_rows("a", &RowsQuery { limit: 1, ..RowsQuery::default() }).unwrap().total
}

#[test]
fn a_failed_write_lands_nothing_stops_the_recording_and_counts_the_batch_unrecorded() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("probe.record.duckdb");
    let spec = BaseTableSpec { name: "a".into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false };
    let store = DuckStore::open(&Residency::Disk { path: file.clone(), memory_limit_mb: 64 }, &[spec], &[]).unwrap();
    let mut r = Recorder {
        store,
        pending: vec![Vec::new(), Vec::new()],
        write_every: Duration::from_secs(1),
        written_at: Instant::now(),
        status: RecordingStatus { file, limit_bytes: 1 << 30, ..RecordingStatus::default() },
    };
    // A batch whose first table is fine: written whole, the very rows taken.
    r.take(0, &[numbered(1), numbered(2)]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows, r.status.stopped_why.clone()), (2, 2, 0, None));
    assert_eq!(rows_of(&r), vec![(1, 1.5), (2, 3.0)]);
    // A batch whose second table the record lacks: nothing of it lands, the first table's rows
    // rolled back with the second's, the batch counted unrecorded, the recording stopped.
    r.take(0, &[row(), row()]);
    r.take(1, &[row()]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows), (2, 2, 3), "{:?}", r.status);
    assert!(r.status.stopped_why.as_deref().is_some_and(|w| w.starts_with("a write to the record failed")), "{:?}", r.status.stopped_why);
    assert_eq!(rows_of(&r), vec![(1, 1.5), (2, 3.0)], "the failed batch's rows nowhere");
    // Stopped: rows after are counted unrecorded and never written.
    r.take(0, &[row()]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows), (2, 2, 4));
    // The failed batch's transaction ended, not left open on the writer's connection (which the
    // reads above, through the store's query connection, cannot see): the stop folds the record's
    // log as it closes, and the size it measures then is the one the drive keeps once closed.
    r.store.checkpoint().expect("the stop's fold, no transaction left open");
    let measured = super::record_bytes(&r.status.file);
    let file = r.status.file.clone();
    drop(r);
    let wal = std::path::PathBuf::from(format!("{}.wal", file.display()));
    assert!(std::fs::metadata(&wal).map_or(true, |m| m.len() == 0), "the log folded");
    assert_eq!(super::record_bytes(&file), measured, "the size the stop publishes, the drive's");
    // Read by a connection of its own, the file holds the good batch alone.
    let conn = duckdb::Connection::open(&file).unwrap();
    let mut stmt = conn.prepare("SELECT CAST(seq AS BIGINT), v FROM a ORDER BY seq").unwrap();
    let on_drive: Vec<(i64, f64)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(on_drive, vec![(1, 1.5), (2, 3.0)]);
}
