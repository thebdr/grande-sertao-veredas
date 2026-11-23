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
    // A batch whose first table is fine: written whole.
    r.take(0, &[row(), row()]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows, r.status.stopped_why.clone()), (2, 2, 0, None));
    // A batch whose second table the record lacks: nothing of it lands, the first table's rows
    // rolled back with the second's, the batch counted unrecorded, the recording stopped.
    r.take(0, &[row(), row()]);
    r.take(1, &[row()]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows), (2, 2, 3), "{:?}", r.status);
    assert!(r.status.stopped_why.as_deref().is_some_and(|w| w.starts_with("a write to the record failed")), "{:?}", r.status.stopped_why);
    // Stopped: rows after are counted unrecorded and never written.
    r.take(0, &[row()]);
    r.write();
    assert_eq!((held(&r), r.status.recorded_rows, r.status.unrecorded_rows), (2, 2, 4));
}
