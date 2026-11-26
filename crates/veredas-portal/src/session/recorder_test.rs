//! A recorder's batch, through the recorder itself (contract: the recording clause, "every table's
//! rows in one transaction, so a batch lands whole or not at all; a write that fails stops the
//! recording, the batch counted unrecorded"). In the crate, since the recorder is the session's
//! own: a recorder whose second table its record lacks fails its write at once, no drive fault
//! needed.

use std::time::{Duration, Instant};

use veredas_core::{SeriesType, SourceId, Value};

use super::{Recorder, RecordingStatus};
use crate::store::{BaseTableSpec, DuckStore, RowsQuery, WideRow};

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
    let store = DuckStore::open_record(&file, 64, &[spec], &[]).unwrap();
    let mut r = Recorder {
        store,
        pending: vec![Vec::new(), Vec::new()],
        write_every: Duration::from_secs(1),
        written_at: Instant::now(),
        status: RecordingStatus { file, limit_bytes: 1 << 30, ..RecordingStatus::default() },
        last_batch_from: None,
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

/// A row of `b`, arrived at `seq`, its text long enough that batches fill blocks.
fn texted(seq: u64) -> WideRow {
    WideRow {
        source: SourceId::new("s"),
        key: None,
        index_ms: seq as i64,
        arrival_ms: seq as i64,
        seq,
        values: vec![Some(Value::Text(format!("line {seq} of a record at its limit, its oldest let go, {}", seq * 7919).into()))],
    }
}

/// At its limit a record lets its oldest rows go (docs/questions.md Q41, ruled 2026-11-25): written
/// in batches of 200 rows to two tables, a record of 1 MiB holds the newest rows each once, every
/// table's from the same arrival on, the last batch always whole (one whose rows take more than
/// half the limit held whole and alone, everything older let go); it never stops, the rows let go and those held adding to every row
/// written, and after every write its files on the drive under the limit and the data in use under
/// half of it.
#[test]
fn at_its_limit_a_record_lets_its_oldest_rows_go_and_stays_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("limit.record.duckdb");
    let specs = [
        BaseTableSpec { name: "a".into(), columns: vec![("v".into(), SeriesType::Float)], has_key: false },
        BaseTableSpec { name: "b".into(), columns: vec![("t".into(), SeriesType::Text)], has_key: false },
    ];
    let limit = 1024 * 1024;
    let mut r = Recorder {
        store: DuckStore::open_record(&file, 64, &specs, &[]).unwrap(),
        pending: vec![Vec::new(), Vec::new()],
        write_every: Duration::from_secs(1),
        written_at: Instant::now(),
        status: RecordingStatus { file: file.clone(), limit_bytes: limit, ..RecordingStatus::default() },
        last_batch_from: None,
    };
    let mut seq = 0u64;
    let mut sizes = Vec::new();
    for _ in 0..150 {
        let a: Vec<WideRow> = (seq..seq + 200).map(numbered).collect();
        let b: Vec<WideRow> = (seq..seq + 200).map(texted).collect();
        r.take(0, &a);
        r.take(1, &b);
        seq += 200;
        r.write();
        assert_eq!(r.status.stopped_why, None, "{:?}", r.status);
        assert!(r.store.bytes_in_use().unwrap() <= limit / 2, "the data in use under half the limit: {:?}", r.status);
        sizes.push(r.status.written_bytes);
        let (a, b) = (seqs_of(&r, "a").len() as u64, seqs_of(&r, "b").len() as u64);
        assert_eq!(a + b + r.status.let_go_rows, r.status.recorded_rows, "held and let go, every row written, at {seq}: {:?}", r.status);
    }
    assert!(sizes.iter().all(|s| *s <= limit), "every size after a write under the limit: {sizes:?}");
    assert_eq!(sizes.last().copied(), Some(super::record_bytes(&file)), "the size published, the drive's");
    assert!(r.status.let_go_rows > 0, "the oldest let go: {:?}", r.status);
    assert_eq!((r.status.recorded_rows, r.status.unrecorded_rows), (2 * seq, 0));
    // The newest rows, each once, both tables from the same arrival on, the last batch whole.
    let (a, b) = (seqs_of(&r, "a"), seqs_of(&r, "b"));
    let from = seq - a.len() as u64;
    assert_eq!(a, (from..seq).collect::<Vec<u64>>(), "table a: the newest rows, each once");
    assert_eq!(b, a, "table b from the same arrival on");
    assert!(a.len() >= 200 && a.len() < 30_000, "the last batch whole, the oldest gone: {}", a.len());
    // A last batch whose rows take more than half the limit (12 000 rows, the record holding some
    // 3 000 before it): every older row let go, the last batch held whole and alone, never stopped.
    let before = a.len() as u64;
    r.take(0, &(seq..seq + 12_000).map(numbered).collect::<Vec<_>>());
    r.take(1, &(seq..seq + 12_000).map(texted).collect::<Vec<_>>());
    seq += 12_000;
    r.write();
    assert_eq!(r.status.stopped_why, None, "{:?}", r.status);
    assert!(r.store.bytes_in_use().unwrap() > limit / 2, "the batch alone over half the limit: {before} before it, {:?}", r.status);
    let a = seqs_of(&r, "a");
    assert_eq!(a, (seq - 12_000..seq).collect::<Vec<u64>>(), "the last batch whole and alone");
    assert_eq!(seqs_of(&r, "b"), a, "table b alike");
}

/// Every row's seq of a record's table, page by page, in order.
fn seqs_of(r: &Recorder, table: &str) -> Vec<u64> {
    let mut out = Vec::new();
    loop {
        let page = r.store.query_rows(table, &RowsQuery { limit: crate::store::MAX_PAGE, offset: out.len(), ..RowsQuery::default() }).unwrap();
        let at = page.columns.iter().position(|c| c.name == "seq").unwrap();
        let n = page.rows.len();
        out.extend(page.rows.iter().map(|row| row[at].as_u64().unwrap()));
        if n < crate::store::MAX_PAGE {
            break;
        }
    }
    out.sort();
    out
}
