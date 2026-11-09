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
    // Without running it: a cast that fails on the row the table holds still describes, while
    // the same select run errors; and the binder names a bad column in its own words.
    conn.execute_batch("INSERT INTO t VALUES (1, 'x')").unwrap();
    let mut stmt = conn.prepare("DESCRIBE (SELECT CAST(b AS INTEGER) AS n FROM t)").unwrap();
    let cols: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(cols, vec![("n".to_string(), "INTEGER".to_string())]);
    let run = conn.query_row("SELECT CAST(b AS INTEGER) FROM t", [], |r| r.get::<_, i32>(0)).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(run.contains("Conversion Error"), "{run}");
    let err = conn.prepare("DESCRIBE (SELECT nope FROM t)").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Binder Error") && err.contains("Referenced column \"nope\" not found"), "{err}");
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
        assert_eq!(limit, "512.0 MiB", "the exact text the store reads back");
        let spill: String = conn.query_row("SELECT current_setting('temp_directory')", [], |r| r.get(0)).unwrap();
        assert_eq!(spill, tmp.display().to_string(), "the temp directory read back as set");
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
    let widened: (i64, String, f64, String) = conn.query_row("SELECT * FROM sensors", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert_eq!(widened, (1, "inlet".to_string(), 1.0, "hi".to_string()), "the rows widen with the describe");
}

/// The engine's version, pinned: a move of the dependency is a deliberate act.
#[test]
fn the_engine_is_the_pinned_version() {
    let conn = Connection::open_in_memory().unwrap();
    let version: String = conn.query_row("SELECT version()", [], |r| r.get(0)).unwrap();
    assert_eq!(version, "v1.5.4", "the bundled engine of duckdb 1.10504.0");
}

// ---- the first refute round: the read side the store relies on -------------------------------

fn sql_path(p: &std::path::Path) -> String {
    p.display().to_string().replace('\'', "''")
}

/// The value variants the engine hands back for the types the store's cells and listing match
/// on: a changed variant would fall into the store's debug-text arm in silence.
#[test]
fn the_value_variants_the_store_matches_on_are_these() {
    let conn = Connection::open_in_memory().unwrap();
    let row: Vec<Value> = conn
        .query_row(
            "SELECT DATE '2026-01-02', TIME '01:02:03.456789', CAST(1.5 AS DECIMAL(38,2)), INTERVAL '-1.5 seconds', CAST(5 AS HUGEINT), TIMESTAMP '2026-01-02 03:04:05.123456'",
            [],
            |r| Ok((0..6).map(|i| r.get::<_, Value>(i).unwrap()).collect()),
        )
        .unwrap();
    assert_eq!(row[0], Value::Date32(20455));
    assert_eq!(row[1], Value::Time64(TimeUnit::Microsecond, 3_723_456_789));
    assert!(matches!(&row[2], Value::Decimal(d) if d.to_string() == "1.50"), "{:?}", row[2]);
    assert_eq!(row[3], Value::Interval { months: 0, days: 0, nanos: -1_500_000_000 });
    assert_eq!(row[4], Value::HugeInt(5));
    assert_eq!(row[5], Value::Timestamp(TimeUnit::Microsecond, 1_767_323_045_123_456));
}

/// The listing's aggregate over a union of tables skips null cells in every term and hands the
/// last value back typed: a double, a text and a timestamp in microseconds.
#[test]
fn the_listing_aggregate_over_a_union_skips_null_cells_and_types_the_last_value() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE a (source VARCHAR, key VARCHAR, index_ms BIGINT, v DOUBLE); \
         CREATE TABLE b (source VARCHAR, key VARCHAR, index_ms BIGINT, v DOUBLE); \
         INSERT INTO a VALUES ('s', 'k1', 1, 1.5), ('s', 'k2', 2, 2.5); INSERT INTO b VALUES ('s', NULL, 3, NULL); \
         CREATE TABLE c (source VARCHAR, key VARCHAR, index_ms BIGINT, v TIMESTAMP); \
         INSERT INTO c VALUES ('s', 'k', 1, TIMESTAMP '2026-01-02 00:00:00.000002'), ('s', NULL, 2, NULL); \
         CREATE TABLE d (source VARCHAR, key VARCHAR, index_ms BIGINT, v VARCHAR); \
         INSERT INTO d VALUES ('s', 'k', 1, 'a'), ('s', 'k', 2, 'b'), ('s', NULL, 3, NULL);",
    )
    .unwrap();
    let agg = "SELECT source, count(v), count(DISTINCT key) FILTER (WHERE v IS NOT NULL), max(CASE WHEN v IS NOT NULL THEN index_ms END), arg_max(v, index_ms)";
    let (src, cnt, keys, last_ms, last): (String, i64, i64, i64, Value) = conn
        .query_row(&format!("{agg} FROM (SELECT * FROM a UNION ALL SELECT * FROM b) GROUP BY source"), [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap();
    assert_eq!((src.as_str(), cnt, keys, last_ms, last), ("s", 2, 2, 2, Value::Double(2.5)));
    let last: Value = conn.query_row(&format!("{agg} FROM c GROUP BY source"), [], |r| r.get(4)).unwrap();
    assert_eq!(last, Value::Timestamp(TimeUnit::Microsecond, 1_767_312_000_000_002));
    let last: Value = conn.query_row(&format!("{agg} FROM d GROUP BY source"), [], |r| r.get(4)).unwrap();
    assert_eq!(last, Value::Text("b".into()));
}

/// A positional tie-break after the time column orders every column descending with nulls last,
/// so a page walk is a total order over the cells.
#[test]
fn a_positional_tie_break_sorts_nulls_last_under_desc() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (index_ms BIGINT, s VARCHAR); INSERT INTO t VALUES (1, 'a'), (1, NULL), (1, 'b'), (2, NULL);").unwrap();
    let mut stmt = conn.prepare("SELECT * FROM t ORDER BY index_ms DESC, 1 DESC, 2 DESC LIMIT 2 OFFSET 1").unwrap();
    let rows: Vec<(i64, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(rows, vec![(1, Some("b".to_string())), (1, Some("a".to_string()))]);
}

/// An empty table copies to a CSV of its header alone.
#[test]
fn an_empty_table_copies_to_a_header_alone() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("em.csv");
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("CREATE TABLE em (a INTEGER, b INTEGER); COPY em TO '{}' (HEADER, FORMAT CSV);", sql_path(&f))).unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "a,b\n");
}

/// `EXPORT DATABASE` writes the schema with its views, a load script and one CSV per table under
/// a sanitised file name that never leaves the folder.
#[test]
fn export_database_writes_the_schema_its_views_and_sanitised_file_names() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE em (a INTEGER); CREATE VIEW vv AS SELECT a FROM em; CREATE TABLE u1 (x INTEGER); CREATE TABLE u2 (x INTEGER); CREATE TABLE \"we ird/na\"\"me\" (x INTEGER);",
    )
    .unwrap();
    conn.execute_batch(&format!("EXPORT DATABASE '{}' (FORMAT CSV);", sql_path(&out))).unwrap();
    let schema = std::fs::read_to_string(out.join("schema.sql")).unwrap();
    assert!(schema.contains("CREATE VIEW vv AS SELECT a FROM em;"), "{schema}");
    assert!(out.join("load.sql").exists());
    let mut names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["em.csv", "load.sql", "schema.sql", "u_.csv", "u__1.csv", "we_ird_na_me.csv"]);
}

/// The CSV reader's options type a decimal comma where the sniffer would read text, and pad a
/// ragged file's short rows with nulls instead of dropping them.
#[test]
fn the_csv_reader_options_type_a_decimal_comma_and_pad_a_ragged_file() {
    let dir = tempfile::tempdir().unwrap();
    let eu = dir.path().join("eu.csv");
    std::fs::write(&eu, "a;b\n1,5;2\n").unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let ty = |sql: &str| -> String { conn.query_row(&format!("SELECT column_type FROM (DESCRIBE {sql}) LIMIT 1"), [], |r| r.get(0)).unwrap() };
    assert_eq!(ty(&format!("SELECT * FROM read_csv_auto('{}')", sql_path(&eu))), "VARCHAR");
    assert_eq!(ty(&format!("SELECT * FROM read_csv('{}', delim=';', null_padding=true, decimal_separator=',')", sql_path(&eu))), "DOUBLE");
    let a: f64 = conn.query_row(&format!("SELECT a FROM read_csv('{}', delim=';', null_padding=true, decimal_separator=',')", sql_path(&eu)), [], |r| r.get(0)).unwrap();
    assert_eq!(a, 1.5);
    let ragged = dir.path().join("ragged.csv");
    std::fs::write(&ragged, "a;b;c\n1;2;3\n4;5\n").unwrap();
    let mut stmt = conn.prepare(&format!("SELECT a, b, c FROM read_csv('{}', delim=';', null_padding=true) ORDER BY a", sql_path(&ragged))).unwrap();
    let rows: Vec<(i64, i64, Option<i64>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(rows, vec![(1, 2, Some(3)), (4, 5, None)]);
}

/// The sniffer's outcomes on the edge files the store meets: a byte-order mark read through, a
/// header alone typed text with no rows, an empty file one text column, a missing file an I/O
/// error naming the pattern.
#[test]
fn the_sniffer_s_outcomes_on_a_marked_a_header_only_an_empty_and_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let describe = |p: &std::path::Path| -> Vec<(String, String)> {
        let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM read_csv_auto('{}')", sql_path(p))).unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    };
    let bom = dir.path().join("bom.csv");
    std::fs::write(&bom, "\u{feff}a,b\n1,2\n").unwrap();
    assert_eq!(describe(&bom), vec![("a".to_string(), "BIGINT".to_string()), ("b".to_string(), "BIGINT".to_string())]);
    let header = dir.path().join("header.csv");
    std::fs::write(&header, "a,b\n").unwrap();
    assert_eq!(describe(&header), vec![("a".to_string(), "VARCHAR".to_string()), ("b".to_string(), "VARCHAR".to_string())]);
    let n: i64 = conn.query_row(&format!("SELECT count(*) FROM read_csv_auto('{}')", sql_path(&header)), [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    let empty = dir.path().join("empty.csv");
    std::fs::write(&empty, "").unwrap();
    assert_eq!(describe(&empty), vec![("column0".to_string(), "VARCHAR".to_string())]);
    let gone = dir.path().join("gone.csv");
    std::fs::write(&gone, "a,b\n1,2\n").unwrap();
    conn.execute_batch(&format!("CREATE VIEW gone AS SELECT * FROM read_csv_auto('{}')", sql_path(&gone))).unwrap();
    std::fs::remove_file(&gone).unwrap();
    // The store's own form: the view's select rebound, so the missing file is met now.
    let err = match conn.prepare("DESCRIBE SELECT * FROM gone") {
        Err(e) => e.to_string(),
        Ok(mut stmt) => stmt.query([]).and_then(|mut rows| rows.next().map(|_| ())).err().map(|e| e.to_string()).unwrap_or_default(),
    };
    assert!(err.contains("IO Error") && err.contains("No files found that match the pattern"), "{err}");
}

/// A copy into a foreign database is refused by the engine on the first table it already holds,
/// and the attachment detaches afterwards.
#[test]
fn a_copy_into_a_foreign_database_is_refused_and_detaches_after() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.duckdb");
    {
        let ext = Connection::open(&other).unwrap();
        ext.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (42);").unwrap();
    }
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);").unwrap();
    let dbname: String = conn.query_row("SELECT current_database()", [], |r| r.get(0)).unwrap();
    conn.execute_batch(&format!("ATTACH '{}' AS _vsnap;", sql_path(&other))).unwrap();
    let err = conn.execute_batch(&format!("COPY FROM DATABASE \"{dbname}\" TO _vsnap;")).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Catalog Error") && err.contains("Table with name \"t\" already exists"), "{err}");
    conn.execute_batch("DETACH _vsnap;").unwrap();
    let ext = Connection::open(&other).unwrap();
    let x: i64 = ext.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(x, 42, "the foreign file untouched");
}

// ---- the second refute round -----------------------------------------------------------------

/// The filtered distinct count the listing uses counts the keys of the rows that hold a value
/// alone: a row with a key and a null value is left out, which the plain distinct count would
/// not do, so the pin shows the filter mattering.
#[test]
fn the_filtered_distinct_count_leaves_out_a_keyed_row_with_a_null_value() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE a (source VARCHAR, key VARCHAR, index_ms BIGINT, v DOUBLE); \
         INSERT INTO a VALUES ('s', 'k1', 1, 1.5), ('s', 'k2', 2, 2.5), ('s', 'k3', 4, NULL);",
    )
    .unwrap();
    let (cnt, keys, last_ms, last): (i64, i64, i64, Value) = conn
        .query_row(
            "SELECT count(v), count(DISTINCT key) FILTER (WHERE v IS NOT NULL), max(CASE WHEN v IS NOT NULL THEN index_ms END), arg_max(v, index_ms) FROM a GROUP BY source",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((cnt, keys, last_ms, last), (2, 2, 2, Value::Double(2.5)));
    let unfiltered: i64 = conn.query_row("SELECT count(DISTINCT key) FROM a", [], |r| r.get(0)).unwrap();
    assert_eq!(unfiltered, 3, "without the filter the keyed null row would count");
}

/// A ragged file under the reader's options: a short row is padded, a long row inside the sniff
/// sample widens the schema by a numbered column and every row is kept, and a long row past the
/// sample refuses the whole read with the line named.
#[test]
fn a_long_row_widens_the_schema_in_the_sample_and_refuses_the_read_past_it() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let early = dir.path().join("early.csv");
    std::fs::write(&early, "a;b;c\n1;2;3\n4;5\n6;7;8;9\n").unwrap();
    let scan = |p: &std::path::Path| format!("read_csv('{}', delim=';', null_padding=true)", sql_path(p));
    let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM {}", scan(&early))).unwrap();
    let cols: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        cols,
        vec![("a".to_string(), "BIGINT".to_string()), ("b".to_string(), "BIGINT".to_string()), ("c".to_string(), "BIGINT".to_string()), ("column3".to_string(), "BIGINT".to_string())]
    );
    let mut stmt = conn.prepare(&format!("SELECT a, b, c, column3 FROM {} ORDER BY a", scan(&early))).unwrap();
    let rows: Vec<(i64, i64, Option<i64>, Option<i64>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(rows, vec![(1, 2, Some(3), None), (4, 5, None, None), (6, 7, Some(8), Some(9))]);
    let late = dir.path().join("late.csv");
    let mut text = String::from("a;b;c\n");
    for i in 0..30_000 {
        text.push_str(&format!("{i};1;2\n"));
    }
    text.push_str("9;9;9;9\n");
    std::fs::write(&late, text).unwrap();
    let err = conn.query_row(&format!("SELECT count(*) FROM {}", scan(&late)), [], |r| r.get::<_, i64>(0)).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("CSV Error on Line: 30002"), "{err}");
}

// ---- the third refute round ------------------------------------------------------------------

/// The window in which a long row widens the schema is the file's first 2 048 lines, the header
/// included, whatever the sample size: a long row at line 2 048 widens, one at 2 049 refuses the
/// read by that line, and sampling the whole file moves nothing.
#[test]
fn a_long_row_s_window_is_the_first_2048_lines_whatever_the_sample_size() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let file = |name: &str, rows: usize| -> std::path::PathBuf {
        let p = dir.path().join(name);
        let mut text = String::from("a;b;c\n");
        for i in 0..rows {
            text.push_str(&format!("{i};1;2\n"));
        }
        text.push_str("9;9;9;9\n");
        std::fs::write(&p, text).unwrap();
        p
    };
    let scan = |p: &std::path::Path, extra: &str| format!("read_csv('{}', delim=';', null_padding=true{extra})", sql_path(p));
    let width = |sql: &str| -> i64 { conn.query_row(&format!("SELECT count(*) FROM (DESCRIBE SELECT * FROM {sql})"), [], |r| r.get(0)).unwrap() };
    let count = |sql: &str| -> Result<i64, String> { conn.query_row(&format!("SELECT count(*) FROM {sql}"), [], |r| r.get(0)).map_err(|e| e.to_string()) };
    let inside = file("inside.csv", 2046);
    assert_eq!(width(&scan(&inside, "")), 4, "a long row at line 2048 widens the schema");
    assert_eq!(count(&scan(&inside, "")), Ok(2047));
    let past = file("past.csv", 2047);
    assert_eq!(width(&scan(&past, "")), 3, "a long row at line 2049 is past the window");
    let err = count(&scan(&past, "")).unwrap_err();
    assert!(err.contains("CSV Error on Line: 2049"), "{err}");
    let late = file("late.csv", 30_000);
    let err = count(&scan(&late, ", sample_size=-1")).unwrap_err();
    assert!(err.contains("CSV Error on Line: 30002"), "the whole file sampled moves nothing: {err}");
}

/// A view binds when it is created: a reference to a table not there yet fails at creation with
/// the catalog's text and leaves no view behind, a bad column fails with the binder's text, and
/// the retry once the reference exists succeeds. The store's fixpoint over view definitions
/// rests on this.
#[test]
fn a_view_binds_when_it_is_created_so_a_fixpoint_can_order_them() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (a BIGINT)").unwrap();
    let err = conn.execute_batch("CREATE VIEW v2 AS (SELECT * FROM v1)").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Catalog Error") && err.contains("Table with name v1 does not exist"), "{err}");
    let left: i64 = conn.query_row("SELECT count(*) FROM duckdb_views() WHERE view_name = 'v2'", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0, "a failed create leaves no view behind");
    let err = conn.execute_batch("CREATE VIEW b AS (SELECT nope FROM t)").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Binder Error") && err.contains("Referenced column \"nope\" not found"), "{err}");
    conn.execute_batch("CREATE VIEW v1 AS (SELECT a FROM t); CREATE VIEW v2 AS (SELECT * FROM v1);").unwrap();
    let views: i64 = conn.query_row("SELECT count(*) FROM duckdb_views() WHERE NOT internal", [], |r| r.get(0)).unwrap();
    assert_eq!(views, 2);
}

/// The appender takes the store's typed cells through a slice of `ToSql`: a text, a null, a
/// microsecond timestamp, a double and nulls in typed columns, read back exactly as given.
#[test]
fn the_appender_takes_the_store_s_typed_cells_through_a_slice_of_to_sql() {
    use duckdb::ToSql;
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE w (source VARCHAR, key VARCHAR, ts TIMESTAMP, v DOUBLE, i BIGINT, s VARCHAR, t TIMESTAMP)").unwrap();
    let vals = vec![
        Value::Text("src".into()),
        Value::Null,
        Value::Timestamp(TimeUnit::Microsecond, 1),
        Value::Double(2.5),
        Value::Null,
        Value::Null,
        Value::Null,
    ];
    {
        let mut app = conn.appender("w").unwrap();
        let refs: Vec<&dyn ToSql> = vals.iter().map(|v| v as &dyn ToSql).collect();
        app.append_row(refs.as_slice()).unwrap();
        app.flush().unwrap();
    }
    let back: Vec<Value> = conn.query_row("SELECT * FROM w", [], |r| Ok((0..7).map(|i| r.get::<_, Value>(i).unwrap()).collect())).unwrap();
    assert_eq!(back, vals);
}

// ---- the fourth refute round -----------------------------------------------------------------

/// The sniffer's outcomes on a ragged file, the first step the store takes on every linked CSV:
/// a short row collapses the file to one column (the one case the store recovers), a long row
/// within the first 2 048 lines keeps that row alone with no error, one up to line 20 480
/// refuses the sniff, and one past it refuses the read with the line named.
#[test]
fn the_sniffer_s_outcomes_on_a_ragged_file() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    let file = |name: &str, rows: usize| -> std::path::PathBuf {
        let p = dir.path().join(name);
        let mut text = String::from("a;b;c\n");
        for i in 0..rows {
            text.push_str(&format!("{i};1;2\n"));
        }
        text.push_str("9;9;9;9\n");
        std::fs::write(&p, text).unwrap();
        p
    };
    let scan = |p: &std::path::Path| format!("read_csv_auto('{}')", sql_path(p));
    let width = |sql: &str| -> Result<i64, String> {
        conn.query_row(&format!("SELECT count(*) FROM (DESCRIBE SELECT * FROM {sql})"), [], |r| r.get(0)).map_err(|e| e.to_string())
    };
    let count = |sql: &str| -> Result<i64, String> { conn.query_row(&format!("SELECT count(*) FROM {sql}"), [], |r| r.get(0)).map_err(|e| e.to_string()) };
    let short = dir.path().join("short.csv");
    std::fs::write(&short, "a;b;c\n1;2;3\n4;5\n6;7;8\n").unwrap();
    assert_eq!((width(&scan(&short)), count(&scan(&short))), (Ok(1), Ok(3)), "a short row collapses the file to one column");
    let early = file("early.csv", 2046);
    assert_eq!((width(&scan(&early)), count(&scan(&early))), (Ok(4), Ok(1)), "a long row within the first 2048 lines keeps that row alone");
    let mid = file("mid.csv", 2047);
    let err = width(&scan(&mid)).unwrap_err();
    assert!(err.contains("Error when sniffing file"), "{err}");
    let late = file("late.csv", 20_479);
    assert_eq!(width(&scan(&late)), Ok(3));
    let err = count(&scan(&late)).unwrap_err();
    assert!(err.contains("CSV Error on Line: 20481"), "{err}");
}
