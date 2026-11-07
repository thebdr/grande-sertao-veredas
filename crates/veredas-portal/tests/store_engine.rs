//! What the embedded engine does, pinned against the engine itself (contract: the engine clause).
//! Each test pins one assumption the store is built on; when the pinned version moves, the
//! probe that fails names the assumption that broke, rather than a store test failing for a
//! reason nobody can see. Run first after a version bump, as core's OPC capabilities are.

use std::sync::{Arc, Mutex};

use duckdb::types::{TimeUnit, Value};
use duckdb::{Connection, params};

/// Rows appended through an appender are visible to a cloned connection on another thread once
/// the appender is flushed: the ingest and query split the store is built on.
#[test]
fn an_appender_s_rows_reach_a_cloned_connection_on_another_thread_once_flushed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (v BIGINT)").unwrap();
    {
        let mut app = conn.appender("t").unwrap();
        for i in 0..1_000i64 {
            app.append_row(params![i]).unwrap();
        }
        app.flush().unwrap();
    }
    let clone = conn.try_clone().unwrap();
    let count = std::thread::spawn(move || clone.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0)).unwrap()).join().unwrap();
    assert_eq!(count, 1_000);
}

/// An unsigned 64-bit sequence survives the appender, and a timestamp travels as microseconds;
/// `epoch_ms` is the axis conversion the narrow surface uses.
#[test]
fn ubigint_and_timestamp_round_trip_and_epoch_ms_converts() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (seq UBIGINT, ts TIMESTAMP)").unwrap();
    let big: u64 = (1 << 62) + 7;
    let micros: i64 = 1_752_845_000_123_000;
    {
        let mut app = conn.appender("t").unwrap();
        app.append_row(params![Value::UBigInt(big), Value::Timestamp(TimeUnit::Microsecond, micros)]).unwrap();
        app.flush().unwrap();
    }
    let (seq, ts): (Value, Value) = conn.query_row("SELECT seq, ts FROM t", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(seq, Value::UBigInt(big));
    assert_eq!(ts, Value::Timestamp(TimeUnit::Microsecond, micros));
    let ms: i64 = conn.query_row("SELECT epoch_ms(ts) FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(ms, micros / 1_000);
}

/// A nearest-preceding join: one match per probe row, the tolerance a plain conjunct.
#[test]
fn an_asof_join_takes_the_nearest_preceding_row_within_a_tolerance() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE a (key VARCHAR, ms BIGINT);
         CREATE TABLE b (key VARCHAR, ms BIGINT, v BIGINT);
         INSERT INTO b VALUES ('s', 1000, 1), ('s', 10000, 2);
         INSERT INTO a VALUES ('s', 1500), ('s', 2500), ('s', 5000), ('s', 10100);",
    )
    .unwrap();
    let mut stmt = conn.prepare("SELECT a.ms, b.ms, b.v FROM a ASOF JOIN b ON a.key = b.key AND a.ms >= b.ms WHERE a.ms - b.ms <= 2000 ORDER BY a.ms").unwrap();
    let pairs: Vec<(i64, i64, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(pairs, vec![(1500, 1000, 1), (2500, 1000, 1), (10100, 10000, 2)], "5000 matched 1000 but the 4000 ms gap is filtered");
}

/// `time_bucket` is a core function of the bundled build and snaps to its absolute grid.
#[test]
fn time_bucket_snaps_to_an_absolute_grid() {
    let conn = Connection::open_in_memory().unwrap();
    for (ms, want) in [(946_857_612_345i64, 946_857_610_000i64), (946_857_619_999, 946_857_610_000), (946_857_620_000, 946_857_620_000)] {
        let got: i64 = conn
            .query_row("SELECT epoch_ms(time_bucket(INTERVAL 10 SECONDS, make_timestamp(?)))", params![ms * 1_000], |r| r.get(0))
            .unwrap();
        assert_eq!(got, want, "the bucket of {ms}");
    }
}

/// `DESCRIBE (<select>)` yields the columns without running the query; a bad reference fails
/// with a binder message naming it.
#[test]
fn describe_validates_a_select_without_running_it() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (a BIGINT, b VARCHAR)").unwrap();
    let mut stmt = conn.prepare("DESCRIBE (SELECT a, b FROM t)").unwrap();
    let cols: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(cols, vec![("a".to_string(), "BIGINT".to_string()), ("b".to_string(), "VARCHAR".to_string())]);
    let err = conn.prepare("DESCRIBE (SELECT nope FROM t)").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("nope"), "{err}");
}

/// The catalog functions list the user's tables and views with their columns, and `NOT
/// internal` leaves the system's out.
#[test]
fn the_catalog_functions_list_tables_views_and_columns() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE base (x BIGINT, name VARCHAR); CREATE VIEW derived_v AS SELECT x * 2 AS twice FROM base;").unwrap();
    let tables: Vec<String> = conn.prepare("SELECT table_name FROM duckdb_tables() WHERE NOT internal").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(tables, vec!["base".to_string()]);
    let views: Vec<String> = conn.prepare("SELECT view_name FROM duckdb_views() WHERE NOT internal").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(views, vec!["derived_v".to_string()]);
    let cols: Vec<(String, String, String)> = conn
        .prepare("SELECT table_name, column_name, data_type FROM duckdb_columns() WHERE NOT internal ORDER BY table_name, column_index")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(cols, vec![("base".into(), "x".into(), "BIGINT".into()), ("base".into(), "name".into(), "VARCHAR".into()), ("derived_v".into(), "twice".into(), "BIGINT".into())]);
}

/// `duckdb_memory()` answers a figure, and a plan is inspectable.
#[test]
fn memory_is_reported_and_a_plan_is_inspectable() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t AS SELECT range AS v FROM range(10000); CREATE VIEW w AS SELECT v, v * 2 AS y FROM t;").unwrap();
    let bytes: i64 = conn.query_row("SELECT coalesce(sum(memory_usage_bytes), 0) FROM duckdb_memory()", [], |r| r.get(0)).unwrap();
    assert!(bytes > 0, "{bytes}");
    let plan: Vec<(String, String)> = conn.prepare("EXPLAIN SELECT * FROM w WHERE v > 5").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert!(!plan.is_empty());
}

/// `COPY FROM DATABASE` snapshots an in-memory database, tables as data and views as views, into
/// a file a fresh connection reads.
#[test]
fn copy_from_database_snapshots_tables_and_views_into_a_readable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snap.duckdb");
    let path_sql = path.display().to_string().replace('\'', "''");
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (v BIGINT); INSERT INTO t VALUES (1), (2), (3); CREATE VIEW doubled AS SELECT v * 2 AS d FROM t;").unwrap();
    conn.execute_batch(&format!("ATTACH '{path_sql}' AS snap; COPY FROM DATABASE memory TO snap; DETACH snap;")).unwrap();
    drop(conn);
    let back = Connection::open(&path).unwrap();
    let n: i64 = back.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 3);
    let dsum: i64 = back.query_row("SELECT sum(d) FROM doubled", [], |r| r.get(0)).unwrap();
    assert_eq!(dsum, 12);
}

/// A file-backed database honours a memory limit and a temp directory, and reopens with its
/// rows once the connection drops.
#[test]
fn a_file_backed_database_takes_its_limits_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session.duckdb");
    let tmp = dir.path().join("spill");
    std::fs::create_dir_all(&tmp).unwrap();
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(&format!("SET memory_limit = '512MiB'; SET temp_directory = '{}';", tmp.display().to_string().replace('\'', "''"))).unwrap();
        let limit: String = conn.query_row("SELECT current_setting('memory_limit')", [], |r| r.get(0)).unwrap();
        assert!(limit.contains("512"), "{limit}");
        conn.execute_batch("CREATE TABLE t (v BIGINT)").unwrap();
        let mut app = conn.appender("t").unwrap();
        for i in 0..100i64 {
            app.append_row(params![i]).unwrap();
        }
        app.flush().unwrap();
    }
    let back = Connection::open(&db).unwrap();
    let n: i64 = back.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 100);
}

/// A connection behind a mutex is shared across threads: the store's layout.
#[test]
fn a_mutexed_connection_is_shared_across_threads() {
    fn assert_sync<T: Sync + Send>(_: &T) {}
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    assert_sync(&conn);
    let c2 = Arc::clone(&conn);
    let one = std::thread::spawn(move || {
        let guard = c2.lock().unwrap_or_else(|e| e.into_inner());
        guard.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(one, 1);
}

/// A view over `read_csv_auto` re-reads the file on every query, never holds it open, and
/// re-expands `*` per query, so an edited, replaced or widened file shows without recreating it.
#[test]
fn a_csv_view_reflects_edits_a_replacement_and_a_new_column_and_never_locks_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("sensors.csv");
    std::fs::write(&csv, "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,6.5\n").unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let path = csv.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!("CREATE VIEW sensors AS SELECT * FROM read_csv_auto('{path}')")).unwrap();
    let count = |c: &Connection| -> i64 { c.query_row("SELECT count(*) FROM sensors", [], |r| r.get(0)).unwrap() };
    let limit_of = |c: &Connection, id: i64| -> f64 { c.query_row("SELECT limit_mm FROM sensors WHERE sensor = ?", [id], |r| r.get(0)).unwrap() };
    assert_eq!(count(&conn), 2);
    assert!((limit_of(&conn, 2) - 6.5).abs() < 1e-9);
    std::fs::write(&csv, "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,9.25\n3,reject,2.0\n").unwrap();
    assert_eq!(count(&conn), 3);
    assert!((limit_of(&conn, 2) - 9.25).abs() < 1e-9);
    let staged = dir.path().join("sensors.next");
    std::fs::write(&staged, "sensor,label,limit_mm\n1,inlet,1.0\n").unwrap();
    std::fs::rename(&staged, &csv).expect("the replacement must not be blocked by the view");
    assert_eq!(count(&conn), 1);
    std::fs::write(&csv, "sensor,label,limit_mm,notes\n1,inlet,1.0,hi\n").unwrap();
    let cols: i64 = conn.query_row("SELECT count(*) FROM (DESCRIBE SELECT * FROM sensors)", [], |r| r.get(0)).unwrap();
    assert_eq!(cols, 4);
}

/// The engine's version, pinned: a move of the dependency is a deliberate act.
#[test]
fn the_engine_is_the_pinned_version() {
    let conn = Connection::open_in_memory().unwrap();
    let version: String = conn.query_row("SELECT version()", [], |r| r.get(0)).unwrap();
    assert_eq!(version, "v1.5.4", "the bundled engine of duckdb 1.10504.0");
}
