//! The session store (contract: the store clause): a real embedded database, in memory and on
//! disk, holding every protocol's data alike: one wide table per rule, shaped by a capture rule,
//! an OPC rule and a telegram message alike, the narrow samples table, SQL views in one namespace,
//! windowed numeric queries, typed row pages, snapshots and CSV exports, and a CSV read live.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDateTime;
use veredas_core::telegram::catalog;
use veredas_portal::channel::MessageTable;
use veredas_portal::opc::{OpcRuleConfig, OpcSourceConfig, OpcTagConfig, compile, table_of};
use veredas_portal::store::*;
use veredas_portal::{CaptureRule, Sample, SeriesRef, SeriesType, SourceId, Value};

fn ram() -> DuckStore {
    DuckStore::open(&Residency::Ram, &[], &[]).unwrap()
}

fn sample(series: &str, ms: i64, seq: u64, value: Value, key: Option<&str>) -> Sample {
    Sample { source: SourceId::new("s"), series: Arc::from(series), key: key.map(Arc::from), ts: None, seq, arrival_unix_ms: ms, value }
}

fn r(series: &str) -> SeriesRef {
    SeriesRef::new("s", series)
}

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

// ---- the narrow surface ------------------------------------------------------------------------

#[test]
fn samples_append_list_and_query_time_ordered_whatever_the_append_order() {
    let store = ram();
    assert_eq!(store.residency(), &Residency::Ram);
    for (ms, seq, v) in [(300, 1, 3.0), (100, 2, 1.0), (200, 3, 2.0), (400, 4, 4.0)] {
        store.append_samples(&[sample("t", ms, seq, Value::Float(v), None)]).unwrap();
    }
    let infos = store.list();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].series, r("t"));
    assert_eq!(infos[0].ty, SeriesType::Float);
    assert_eq!(infos[0].count, 4);
    assert_eq!(infos[0].last_value, "4", "the value as it displays, not as the engine prints it");
    assert_eq!(infos[0].last_index_ms, 400);
    assert_eq!(infos[0].keys, 0);
    assert_eq!(store.total_samples(), 4);
    let pts = store.query_numeric(&r("t"), None, 0, 999);
    assert_eq!(pts.iter().map(|p| (p.index_ms, p.seq, p.value)).collect::<Vec<_>>(), vec![(100, 2, 1.0), (200, 3, 2.0), (300, 1, 3.0), (400, 4, 4.0)]);
    assert_eq!(store.query_numeric(&r("t"), None, 200, 300).iter().map(|p| p.value).collect::<Vec<_>>(), vec![2.0, 3.0], "inclusive both ends");
    assert!(store.query_numeric(&r("t"), None, 401, 999).is_empty());
    assert!(store.query_numeric(&r("ghost"), None, 0, 999).is_empty());
    assert!(store.query_numeric(&SeriesRef::new("other", "t"), None, 0, 999).is_empty(), "another source's series is another series");
    assert_eq!(store.append_samples(&[]).ok(), Some(()));
    assert_eq!(store.total_samples(), 4);
}

#[test]
fn keys_filter_a_query_and_are_counted_distinct_an_unkeyed_row_reading_as_the_empty_key() {
    let store = ram();
    store
        .append_samples(&[
            sample("v", 1, 1, Value::Float(10.0), Some("a")),
            sample("v", 2, 2, Value::Float(20.0), Some("b")),
            sample("v", 3, 3, Value::Float(30.0), Some("a")),
            sample("v", 4, 4, Value::Float(40.0), None),
        ])
        .unwrap();
    assert_eq!(store.query_numeric(&r("v"), Some("a"), 0, 999).iter().map(|p| p.value).collect::<Vec<_>>(), vec![10.0, 30.0]);
    assert_eq!(store.query_numeric(&r("v"), Some(""), 0, 999).iter().map(|p| p.value).collect::<Vec<_>>(), vec![40.0]);
    assert_eq!(store.query_numeric(&r("v"), None, 0, 999).len(), 4);
    let info = &store.list()[0];
    assert_eq!(info.keys, 2);
    assert!(!info.keys_capped);
}

#[test]
fn a_text_series_has_no_numeric_view_a_time_series_reads_as_epoch_milliseconds_and_an_int_as_itself() {
    let store = ram();
    store.append_samples(&[sample("state", 1, 1, Value::Text("RUN".into()), None)]).unwrap();
    assert!(store.query_numeric(&r("state"), None, 0, 999).is_empty());
    let infos = store.list();
    let state = infos.iter().find(|i| i.series.series.as_ref() == "state").unwrap();
    assert_eq!((state.ty, state.last_value.as_str()), (SeriesType::Text, "RUN"));
    let t = dt("2026-07-18 10:00:00");
    store.append_samples(&[sample("stamp", 2, 2, Value::Time(t), None), sample("n", 3, 3, Value::Int(-7), None)]).unwrap();
    let pts = store.query_numeric(&r("stamp"), None, 0, i64::MAX);
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0].value, t.and_utc().timestamp_millis() as f64);
    assert_eq!(store.query_numeric(&r("n"), None, 0, 9)[0].value, -7.0);
    let infos = store.list();
    assert_eq!(infos.iter().map(|i| (i.series.series.to_string(), i.ty)).collect::<Vec<_>>(), [("n".to_string(), SeriesType::Int), ("stamp".to_string(), SeriesType::Time), ("state".to_string(), SeriesType::Text)]);
    assert_eq!(infos[1].last_value, "2026-07-18 10:00:00");
    assert!(store.approx_bytes() > 0);
}

// ---- base tables: every protocol's rows alike ------------------------------------------------

/// The table a capture rule yields: its value groups in pattern order, the key apart.
fn capture_spec() -> BaseTableSpec {
    let rule = CaptureRule::new(
        "readings",
        r"S (?P<sensor>\w+) temp=(?P<temp>-?\d+\.\d+) tag=(?P<tag>\w+)",
        None,
        &HashMap::from([("temp".to_string(), SeriesType::Float)]),
        Some("sensor"),
    )
    .unwrap();
    BaseTableSpec { name: rule.name().to_string(), columns: rule.series().map(|(n, t)| (n.to_string(), t)).collect(), has_key: rule.key_group().is_some() }
}

/// The table an OPC rule yields: each tag's column then its bits, as core lays them out.
fn opc_spec() -> BaseTableSpec {
    let cfg = OpcSourceConfig {
        source_id: "plc".into(),
        display_name: String::new(),
        endpoint_url: "opc.tcp://127.0.0.1:1/".into(),
        user: None,
        password_env: None,
        poll_ms: 1000,
        rules: vec![OpcRuleConfig {
            name: "plc_main".into(),
            store_on_change: false,
            tags: vec![
                OpcTagConfig { node_id: "ns=2;s=speed".into(), series: "speed".into(), ty: "float".into(), writable: false, bits: vec![] },
                OpcTagConfig {
                    node_id: "ns=2;s=word".into(),
                    series: "word".into(),
                    ty: "int".into(),
                    writable: false,
                    bits: vec![veredas_portal::opc::OpcBitConfig { bit: 0, width: 1, series: "running".into() }],
                },
            ],
        }],
    };
    let spec = compile(&cfg, &|_| None).unwrap();
    let (name, columns) = table_of(&spec.rules[0]);
    BaseTableSpec { name, columns, has_key: false }
}

/// The table a telegram message yields: the catalog's fields typed by the catalog.
fn message_spec() -> BaseTableSpec {
    let text = "format = \"veredas-catalog\"\nversion = 1\nid_field = \"id\"\n\n[[message]]\nname = \"announce\"\nid = 21\nlen = 8\nfields = [ { name = \"item\", at = 0, ty = \"U32\" }, { name = \"code\", at = 4, len = 4, ty = \"Ascii\" } ]\n";
    let cat = catalog::load(text).unwrap();
    let table = MessageTable::of("sorter", cat.by_name("announce").unwrap()).unwrap();
    BaseTableSpec { name: table.name, columns: table.columns, has_key: false }
}

fn wide(source: &str, key: Option<&str>, ms: i64, seq: u64, values: Vec<Option<Value>>) -> WideRow {
    WideRow { source: SourceId::new(source), key: key.map(Arc::from), index_ms: ms, arrival_ms: ms + 7, seq, values }
}

#[test]
fn rows_of_a_capture_rule_an_opc_rule_and_a_telegram_message_are_stored_alike_and_feed_the_narrow_surface() {
    let specs = [capture_spec(), opc_spec(), message_spec()];
    assert_eq!(specs[0].columns, vec![("temp".to_string(), SeriesType::Float), ("tag".to_string(), SeriesType::Text)]);
    assert_eq!(specs[1].columns, vec![("speed".to_string(), SeriesType::Float), ("word".to_string(), SeriesType::Int), ("running".to_string(), SeriesType::Int)]);
    assert_eq!(specs[2].columns, vec![("item".to_string(), SeriesType::Int), ("code".to_string(), SeriesType::Text)]);
    let store = DuckStore::open(&Residency::Ram, &specs, &[]).unwrap();
    // A line's row, keyed by its sensor; a poll's row; a message's row: one shape.
    store
        .append_wide(0, &[wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(25.0)), Some(Value::Text("ok".into()))]), wide("a", Some("s1"), 2000, 2, vec![Some(Value::Float(26.0)), None])])
        .unwrap();
    store.append_wide(1, &[wide("plc", None, 1500, 1, vec![Some(Value::Float(1.5)), Some(Value::Int(5)), Some(Value::Int(1))])]).unwrap();
    store.append_wide(2, &[wide("sorter", None, 1200, 1, vec![Some(Value::Int(7)), Some(Value::Text("PKG7".into()))])]).unwrap();
    assert_eq!(store.total_samples(), 3 + 3 + 2, "value cells, a hole counting for nothing");

    let tables = store.list_tables().unwrap();
    assert_eq!(tables.iter().map(|t| (t.name.as_str(), t.kind, t.rows)).collect::<Vec<_>>(), [("announce", TableKind::Base, Some(1)), ("plc_main", TableKind::Base, Some(1)), ("readings", TableKind::Base, Some(2)), ("samples", TableKind::Samples, Some(0))]);
    let readings = tables.iter().find(|t| t.name == "readings").unwrap();
    assert_eq!(
        readings.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect::<Vec<_>>(),
        [("source", "VARCHAR"), ("key", "VARCHAR"), ("index_ms", "BIGINT"), ("ts", "TIMESTAMP"), ("arrival_ms", "BIGINT"), ("seq", "UBIGINT"), ("temp", "DOUBLE"), ("tag", "VARCHAR")]
    );
    let plc = tables.iter().find(|t| t.name == "plc_main").unwrap();
    assert_eq!(plc.columns[6..].iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect::<Vec<_>>(), [("speed", "DOUBLE"), ("word", "BIGINT"), ("running", "BIGINT")]);
    let announce = tables.iter().find(|t| t.name == "announce").unwrap();
    assert_eq!(announce.columns[6..].iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect::<Vec<_>>(), [("item", "BIGINT"), ("code", "VARCHAR")]);

    // Every column reads as a series of its source on the narrow surface.
    let pts = store.query_numeric(&SeriesRef::new("a", "temp"), Some("s1"), 0, i64::MAX);
    assert_eq!(pts.iter().map(|p| (p.index_ms, p.seq, p.value)).collect::<Vec<_>>(), [(1000, 1, 25.0), (2000, 2, 26.0)]);
    assert_eq!(store.query_numeric(&SeriesRef::new("plc", "running"), None, 0, i64::MAX)[0].value, 1.0);
    assert_eq!(store.query_numeric(&SeriesRef::new("sorter", "item"), None, 0, i64::MAX)[0].value, 7.0);
    assert!(store.query_numeric(&SeriesRef::new("sorter", "code"), None, 0, i64::MAX).is_empty(), "text has no numeric view");
    let infos = store.list();
    let names: Vec<String> = infos.iter().map(|i| i.series.to_string()).collect();
    assert_eq!(names, ["a/tag", "a/temp", "plc/running", "plc/speed", "plc/word", "sorter/code", "sorter/item"]);
    let temp = infos.iter().find(|i| i.series == SeriesRef::new("a", "temp")).unwrap();
    assert_eq!((temp.ty, temp.count, temp.keys, temp.last_index_ms, temp.last_value.as_str()), (SeriesType::Float, 2, 1, 2000, "26"));
    let tag = infos.iter().find(|i| i.series == SeriesRef::new("a", "tag")).unwrap();
    assert_eq!((tag.count, tag.last_index_ms, tag.last_value.as_str()), (1, 1000, "ok"), "a hole is not a value");

    // A row page, newest first, every cell typed, the time axis in both forms.
    let page = store.query_rows("readings", &RowsQuery { limit: 10, ..Default::default() }).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows.len(), 2);
    let col = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap();
    assert_eq!(page.rows[0][col("index_ms")], serde_json::json!(2000));
    assert_eq!(page.rows[0][col("ts")], serde_json::json!(2000), "a timestamp as milliseconds");
    assert_eq!(page.rows[0][col("arrival_ms")], serde_json::json!(2007));
    assert_eq!(page.rows[0][col("seq")], serde_json::json!(2));
    assert_eq!(page.rows[0][col("temp")], serde_json::json!(26.0));
    assert_eq!(page.rows[0][col("tag")], serde_json::Value::Null);
    assert_eq!(page.rows[1][col("tag")], serde_json::json!("ok"));
    assert_eq!(page.rows[1][col("key")], serde_json::json!("s1"));
    let page = store.query_rows("readings", &RowsQuery { from_ms: Some(1500), to_ms: None, limit: 10, offset: 0 }).unwrap();
    assert_eq!((page.total, page.rows.len()), (1, 1));
    let page = store.query_rows("readings", &RowsQuery { from_ms: None, to_ms: Some(1500), limit: 10, offset: 0 }).unwrap();
    assert_eq!(page.rows[0][col("temp")], serde_json::json!(25.0));
    let page = store.query_rows("readings", &RowsQuery { limit: 1, offset: 1, ..Default::default() }).unwrap();
    assert_eq!((page.total, page.rows.len()), (2, 1), "the total counts the window, the page what fits");
    assert_eq!(page.rows[0][col("temp")], serde_json::json!(25.0));
    let page = store.query_rows("readings", &RowsQuery { limit: 0, ..Default::default() }).unwrap();
    assert_eq!(page.rows.len(), 1, "a limit of 0 is 1");
    let page = store.query_rows("readings", &RowsQuery { limit: 5_000, ..Default::default() }).unwrap();
    assert_eq!(page.rows.len(), 2, "a limit over the cap is the cap");
    let page = store.query_rows("announce", &RowsQuery { limit: 10, ..Default::default() }).unwrap();
    assert_eq!(page.rows[0][6], serde_json::json!(7));
    assert_eq!(page.rows[0][7], serde_json::json!("PKG7"));
    // The chart of a table column, keyed or not.
    let pts = store.query_table_series("readings", "temp", Some("s1"), 0, i64::MAX).unwrap();
    assert_eq!(pts.iter().map(|p| (p.index_ms, p.value)).collect::<Vec<_>>(), [(1000, 25.0), (2000, 26.0)]);
    assert!(store.query_table_series("readings", "temp", Some("s9"), 0, i64::MAX).unwrap().is_empty());
    assert_eq!(store.query_table_series("plc_main", "word", None, 0, i64::MAX).unwrap()[0].value, 5.0);
    assert!(matches!(store.query_table_series("readings", "nope", None, 0, 1).unwrap_err(), StoreError::UnknownTable(t) if t == "readings.nope"));
    assert!(matches!(store.query_table_series("ghost", "temp", None, 0, 1).unwrap_err(), StoreError::UnknownTable(t) if t == "ghost"));
    assert!(matches!(store.query_rows("ghost", &RowsQuery::default()).unwrap_err(), StoreError::UnknownTable(_)));
    assert!(matches!(store.append_wide(3, &[wide("x", None, 1, 1, vec![])]).unwrap_err(), StoreError::UnknownTable(t) if t == "#3"));
    assert!(store.append_wide(0, &[]).is_ok());
}

#[test]
fn views_join_across_sources_apply_in_any_order_and_a_broken_one_names_itself() {
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[
            TableDef { name: "hot".into(), sql: "SELECT * FROM dev WHERE dev > 1.5".into() },
            TableDef {
                name: "dev".into(),
                sql: "SELECT b.key, b.index_ms, a.temp - b.temp AS dev \
                      FROM (SELECT * FROM readings WHERE source = 'b') AS b \
                      ASOF JOIN (SELECT * FROM readings WHERE source = 'a') AS a \
                      ON b.key = a.key AND b.index_ms >= a.index_ms \
                      WHERE b.index_ms - a.index_ms <= 2000"
                    .into(),
            },
        ],
    )
    .unwrap();
    // a leads, b trails by 200 ms; the gap grows 1 then 2.
    store
        .append_wide(
            0,
            &[
                wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(25.0)), None]),
                wide("b", Some("s1"), 1200, 2, vec![Some(Value::Float(24.0)), None]),
                wide("a", Some("s1"), 6000, 3, vec![Some(Value::Float(26.0)), None]),
                wide("b", Some("s1"), 6200, 4, vec![Some(Value::Float(24.0)), None]),
            ],
        )
        .unwrap();
    let page = store.query_rows("dev", &RowsQuery { limit: 100, ..Default::default() }).unwrap();
    assert_eq!(page.total, 2);
    let dev_col = page.columns.iter().position(|c| c.name == "dev").unwrap();
    assert_eq!(page.rows.iter().map(|row| row[dev_col].as_f64().unwrap()).collect::<Vec<_>>(), vec![2.0, 1.0], "newest first");
    let hot = store.query_rows("hot", &RowsQuery { limit: 100, ..Default::default() }).unwrap();
    assert_eq!(hot.total, 1, "the view over the view, applied though listed first");
    let tables = store.list_tables().unwrap();
    let dev = tables.iter().find(|t| t.name == "dev").unwrap();
    assert_eq!(dev.kind, TableKind::View);
    assert_eq!(dev.rows, None, "a view reports no exact count");
    assert!(dev.columns.iter().any(|c| c.name == "dev"));

    match DuckStore::open(&Residency::Ram, &[capture_spec()], &[TableDef { name: "bad".into(), sql: "SELECT nonesuch FROM readings".into() }]).err().unwrap() {
        StoreError::Definition { name, message } => {
            assert_eq!(name, "bad");
            assert!(message.to_lowercase().contains("nonesuch"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // Two views that need each other never resolve: the last one named.
    let err = DuckStore::open(
        &Residency::Ram,
        &[],
        &[TableDef { name: "x".into(), sql: "SELECT * FROM y".into() }, TableDef { name: "y".into(), sql: "SELECT * FROM x".into() }],
    )
    .err()
    .unwrap();
    assert!(matches!(err, StoreError::Definition { .. }), "{err}");
    let cols = store.validate_definition("SELECT key, temp - 1.0 AS adj FROM readings").unwrap();
    assert_eq!(cols.iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect::<Vec<_>>(), [("key", "VARCHAR"), ("adj", "DOUBLE")]);
    assert!(store.validate_definition("SELECT nope FROM readings").unwrap_err().contains("nope"));
}

#[test]
fn reserved_duplicate_and_empty_names_are_refused_and_type_conflicts_counted() {
    let bad_col = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "r".into(), columns: vec![("index_ms".into(), SeriesType::Int)], has_key: false }], &[]);
    assert!(matches!(bad_col.err().unwrap(), StoreError::ReservedColumn { table, column } if table == "r" && column == "index_ms"));
    let bad_col = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "r".into(), columns: vec![("SEQ".into(), SeriesType::Int)], has_key: false }], &[]);
    assert!(matches!(bad_col.err().unwrap(), StoreError::ReservedColumn { .. }), "whatever the case");
    let dup = DuckStore::open(&Residency::Ram, &[capture_spec(), capture_spec()], &[]);
    assert!(matches!(dup.err().unwrap(), StoreError::TableName(n) if n == "readings"));
    let case = DuckStore::open(&Residency::Ram, &[capture_spec(), BaseTableSpec { name: "Readings".into(), columns: vec![], has_key: false }], &[]);
    assert!(matches!(case.err().unwrap(), StoreError::TableName(n) if n == "Readings"), "names are one namespace whatever the case");
    let clash = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "samples".into(), columns: vec![("v".into(), SeriesType::Int)], has_key: false }], &[]);
    assert!(matches!(clash.err().unwrap(), StoreError::TableName(n) if n == "samples"));
    let empty = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: " ".into(), columns: vec![], has_key: false }], &[]);
    assert!(matches!(empty.err().unwrap(), StoreError::TableName(_)));
    let quote = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "a\"b".into(), columns: vec![], has_key: false }], &[]);
    assert!(matches!(quote.err().unwrap(), StoreError::TableName(_)));
    let store = DuckStore::open(
        &Residency::Ram,
        &[
            BaseTableSpec { name: "ra".into(), columns: vec![("temp".into(), SeriesType::Float)], has_key: false },
            BaseTableSpec { name: "rb".into(), columns: vec![("temp".into(), SeriesType::Text), ("other".into(), SeriesType::Int)], has_key: false },
            BaseTableSpec { name: "rc".into(), columns: vec![("other".into(), SeriesType::Int)], has_key: false },
        ],
        &[],
    )
    .unwrap();
    assert_eq!(store.type_conflicts(), 1, "`temp` declared float and text; `other` agrees");
    assert_eq!(ram().type_conflicts(), 0);
    assert_eq!(store.snapshot().type_conflicts, 1);
}

#[test]
fn the_snapshot_carries_the_vitals() {
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[]).unwrap();
    store.append_wide(0, &[wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(1.0)), Some(Value::Text("x".into()))])]).unwrap();
    let snap = store.snapshot();
    assert_eq!(snap.total_samples, 2);
    assert!(snap.approx_bytes > 0);
    assert_eq!(snap.type_conflicts, 0);
    assert_eq!(snap.series.iter().map(|i| i.series.to_string()).collect::<Vec<_>>(), ["a/tag", "a/temp"]);
}

// ---- persistence ---------------------------------------------------------------------------------

#[test]
fn a_snapshot_is_a_standalone_database_any_tool_reads_views_included() {
    let dir = tempfile::tempdir().unwrap();
    let snap = dir.path().join("snap.duckdb");
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[TableDef { name: "hot".into(), sql: "SELECT * FROM readings WHERE temp > 20.0".into() }]).unwrap();
    store
        .append_wide(0, &[wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(25.0)), None]), wide("a", Some("s1"), 2000, 2, vec![Some(Value::Float(10.0)), None])])
        .unwrap();
    store.save_to(&snap).unwrap();
    // A second snapshot cannot overwrite the first.
    assert!(store.save_to(&snap).is_err());
    drop(store);
    let ext = duckdb::Connection::open(&snap).unwrap();
    let rows: i64 = ext.query_row("SELECT count(*) FROM readings", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 2);
    let hot: i64 = ext.query_row("SELECT count(*) FROM hot", [], |r| r.get(0)).unwrap();
    assert_eq!(hot, 1, "the view survived as a view");
    let key: String = ext.query_row("SELECT key FROM readings LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(key, "s1");
}

#[test]
fn csv_exports_per_table_and_of_the_whole_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[TableDef { name: "hot".into(), sql: "SELECT source, key, temp FROM readings WHERE temp > 20.0".into() }]).unwrap();
    store
        .append_wide(0, &[wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(25.0)), Some(Value::Text("ok".into()))]), wide("a", Some("s1"), 2000, 2, vec![Some(Value::Float(10.0)), None])])
        .unwrap();
    let csv = dir.path().join("hot.csv");
    store.export_table_csv("hot", &csv).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "source,key,temp");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1], "a,s1,25.0");
    assert!(matches!(store.export_table_csv("ghost", &dir.path().join("x.csv")).unwrap_err(), StoreError::UnknownTable(_)));
    let out = dir.path().join("dump");
    store.export_all_csv(&out).unwrap();
    assert!(out.join("schema.sql").exists());
    assert!(out.join("load.sql").exists());
    let csvs: Vec<String> = std::fs::read_dir(&out).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".csv")).collect();
    assert!(csvs.iter().any(|n| n.contains("readings")), "{csvs:?}");
    let schema = std::fs::read_to_string(out.join("schema.sql")).unwrap();
    assert!(schema.contains("CREATE VIEW hot"), "{schema}");
}

#[test]
fn a_disk_store_persists_and_reopens_with_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("deeper").join("session.duckdb");
    {
        let residency = Residency::Disk { path: db.clone(), memory_limit_mb: 256 };
        let store = DuckStore::open(&residency, &[capture_spec()], &[TableDef { name: "limits".into(), sql: "SELECT current_setting('memory_limit') AS memory_limit, current_setting('temp_directory') AS temp_directory".into() }]).unwrap();
        assert_eq!(store.residency(), &residency);
        for i in 0..50i64 {
            store.append_wide(0, &[wide("a", Some("s1"), 1000 + i, i as u64, vec![Some(Value::Float(i as f64)), None])]).unwrap();
        }
        assert_eq!(store.query_numeric(&SeriesRef::new("a", "temp"), None, 0, i64::MAX).len(), 50);
        let limits = store.query_rows("limits", &RowsQuery { limit: 1, ..Default::default() }).unwrap();
        assert_eq!(limits.rows[0][0], serde_json::json!("256.0 MiB"), "the memory limit as set, read through the store");
        assert_eq!(limits.rows[0][1], serde_json::json!(db.parent().unwrap().display().to_string()), "the spill folder beside the file");
    }
    assert!(db.exists(), "the folder was created and the file written");
    let back = duckdb::Connection::open(&db).unwrap();
    let n: i64 = back.query_row("SELECT count(*) FROM readings", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 50);
    let limit: String = back.query_row("SELECT current_setting('memory_limit')", [], |r| r.get(0)).unwrap();
    assert!(!limit.is_empty());
}

// ---- a CSV read live --------------------------------------------------------------------------

#[test]
fn a_linked_csv_is_read_live_through_a_view_never_held_open_and_previewed_without_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("sensors.csv");
    std::fs::write(&csv, "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,6.5\n").unwrap();
    let path = csv.display().to_string();
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[TableDef { name: "sensors".into(), sql: format!("SELECT * FROM {}", csv_scan_sql(&path, &CsvOptions::default())) }],
    )
    .unwrap();
    let count = |store: &DuckStore| store.query_rows("sensors", &RowsQuery { limit: 100, ..Default::default() }).unwrap().total;
    assert_eq!(count(&store), 2);
    // The operator edits the file: a value, a row, then a column.
    std::fs::write(&csv, "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,9.25\n3,reject,2.0\n").unwrap();
    assert_eq!(count(&store), 3, "the view re-reads the file");
    let staged = dir.path().join("sensors.next");
    std::fs::write(&staged, "sensor,label,limit_mm,note\n1,inlet,1.0,hi\n").unwrap();
    std::fs::rename(&staged, &csv).expect("replacing the CSV must not be blocked by the view");
    let page = store.query_rows("sensors", &RowsQuery { limit: 100, ..Default::default() }).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.columns.len(), 4, "a column added to the file appears through the view");
    // Reference data joined to plant rows.
    store.append_wide(0, &[wide("a", Some("1"), 1000, 1, vec![Some(Value::Float(0.5)), None])]).unwrap();
    let cols = store.validate_definition("SELECT s.label FROM readings r JOIN sensors s ON CAST(r.key AS BIGINT) = s.sensor").unwrap();
    assert_eq!(cols[0].name, "label");

    // A preview with no session, following edits, bounded by its limit, erring on bad input.
    std::fs::write(&csv, "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,6.5\n").unwrap();
    let page = preview_csv(&path, 10, &CsvOptions::default()).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["sensor", "label", "limit_mm"]);
    assert!(page.columns[2].data_type.to_uppercase().contains("DOUBLE"), "{}", page.columns[2].data_type);
    assert_eq!(page.rows[0][1], serde_json::json!("inlet"));
    std::fs::write(&csv, "a\n1\n2\n3\n").unwrap();
    let page = preview_csv(&path, 2, &CsvOptions::default()).unwrap();
    assert_eq!((page.total, page.rows.len()), (3, 2));
    assert!(preview_csv("   ", 5, &CsvOptions::default()).is_err());
    assert!(preview_csv(&dir.path().join("nope.csv").display().to_string(), 5, &CsvOptions::default()).is_err());
}

#[test]
fn the_csv_sniffer_adopts_a_decimal_comma_only_when_it_strictly_improves_the_typing() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p.display().to_string()
    };
    let dc = |p: &str| sniff_csv_options(p).unwrap().decimal_comma;
    let euro = write("euro.csv", "sensor;label;limit_mm\n1;inlet;4,0\n2;outlet;6,5\n");
    assert!(dc(&euro));
    let opts = sniff_csv_options(&euro).unwrap();
    let page = preview_csv(&euro, 5, &opts).unwrap();
    assert!(page.columns[2].data_type.to_uppercase().contains("DOUBLE"), "{}", page.columns[2].data_type);
    assert_eq!(page.rows[0][2], serde_json::json!(4.0));
    let plain = write("plain.csv", "sensor,label,limit_mm\n1,inlet,4.0\n2,outlet,6.5\n");
    assert!(!dc(&plain), "forcing the comma would break a working number column");
    let semi_dot = write("semidot.csv", "sensor;label;limit_mm\n1;inlet;4.0\n2;outlet;6.5\n");
    assert!(!dc(&semi_dot), "nothing gained");
    let text = write("text.csv", "a;b\nx;y\np;q\n");
    assert!(!dc(&text));
    assert!(sniff_csv_options("   ").is_err());
    assert_eq!(csv_scan_sql(" /p/a's.csv ", &CsvOptions::default()), "read_csv_auto('/p/a''s.csv')");
    assert_eq!(csv_scan_sql("x.csv", &CsvOptions { delim: Some(";".into()), decimal_comma: true }), "read_csv('x.csv', delim=';', null_padding=true, decimal_separator=',')");
}

#[test]
fn the_csv_sniffer_recovers_a_file_the_engine_collapsed_to_one_column_and_leaves_a_single_column_alone() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p.display().to_string()
    };
    // Row 2 has a stray unquoted `;`.
    let ragged = write("ragged.csv", "id;note\r\n1;hello; world\r\n2;ok\r\n");
    assert!(csv_looks_unsplit(&ragged, &CsvOptions::default()), "the engine collapses this file on its own");
    let opts = sniff_csv_options(&ragged).unwrap();
    assert_eq!(opts.delim.as_deref(), Some(";"));
    assert!(!csv_looks_unsplit(&ragged, &opts));
    let page = preview_csv(&ragged, 10, &opts).unwrap();
    assert!(page.columns.len() >= 2, "{:?}", page.columns);
    assert_eq!(page.total, 2, "no row dropped: the ragged one is padded");
    let single = write("single.csv", "sensor\r\n1\r\n2\r\n");
    let opts = sniff_csv_options(&single).unwrap();
    assert_eq!(opts.delim, None);
    assert_eq!(preview_csv(&single, 10, &opts).unwrap().columns.len(), 1);
    let tabs = write("tabs.csv", "a\tb\r\n1\tx\r\n");
    assert!(!csv_looks_unsplit(&tabs, &CsvOptions::default()));
    assert!(!csv_looks_unsplit("nope.csv", &CsvOptions::default()), "a missing file is not unsplit, it is an error elsewhere");
}

// ---- the first refute round ------------------------------------------------------------------

/// The last value of a base-table column displays as the suite displays it, never as the engine
/// prints it: 26.0 reads "26", 1e20 reads its digits, as the narrow samples already did.
#[test]
fn a_last_value_displays_as_the_suite_does_not_as_the_engine_prints() {
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[]).unwrap();
    store.append_wide(0, &[wide("s", None, 1000, 1, vec![Some(Value::Float(26.0)), None])]).unwrap();
    store.append_wide(0, &[wide("t", None, 1000, 1, vec![Some(Value::Float(1e20)), None])]).unwrap();
    let infos = store.list();
    let last = |src: &str| infos.iter().find(|i| i.series == SeriesRef::new(src, "temp")).map(|i| i.last_value.clone()).unwrap_or_default();
    assert_eq!(last("s"), Value::Float(26.0).to_string());
    assert_eq!(last("s"), "26");
    assert_eq!(last("t"), "100000000000000000000");
}

/// A snapshot never writes over an existing file, a foreign database least of all, refused by
/// name before anything is attached; and a refused snapshot leaves nothing attached, so the next
/// one writes.
#[test]
fn a_snapshot_refuses_an_existing_file_and_a_refusal_leaves_the_next_one_free() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.duckdb");
    {
        let ext = duckdb::Connection::open(&other).unwrap();
        ext.execute_batch("CREATE TABLE precious(x INTEGER); INSERT INTO precious VALUES (42);").unwrap();
    }
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[]).unwrap();
    store.append_wide(0, &[wide("a", Some("s1"), 1000, 1, vec![Some(Value::Float(25.0)), None])]).unwrap();
    let err = store.save_to(&other).unwrap_err().to_string();
    assert!(err.contains("a snapshot never writes over one") && err.contains("other.duckdb"), "{err}");
    {
        let ext = duckdb::Connection::open(&other).unwrap();
        let tables: i64 = ext.query_row("SELECT count(*) FROM duckdb_tables() WHERE NOT internal", [], |r| r.get(0)).unwrap();
        let x: i64 = ext.query_row("SELECT x FROM precious", [], |r| r.get(0)).unwrap();
        assert_eq!((tables, x), (1, 42), "the foreign file holds its one table and nothing of the session");
    }
    let fresh = dir.path().join("fresh.duckdb");
    store.save_to(&fresh).unwrap();
    let ext = duckdb::Connection::open(&fresh).unwrap();
    let rows: i64 = ext.query_row("SELECT count(*) FROM readings", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

/// A linked file gone between a removal and its replacement breaks its own view alone: every
/// base table still lists, pages and charts, the view is listed with no columns, and the
/// replacement is seen on the next query.
#[test]
fn a_missing_linked_csv_hides_no_other_table() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("ref.csv");
    std::fs::write(&csv, "sensor,label\n1,inlet\n").unwrap();
    let path = csv.display().to_string();
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[TableDef { name: "refv".into(), sql: format!("SELECT * FROM {}", csv_scan_sql(&path, &CsvOptions::default())) }],
    )
    .unwrap();
    store.append_wide(0, &[wide("a", Some("1"), 1000, 1, vec![Some(Value::Float(0.5)), None])]).unwrap();
    std::fs::remove_file(&csv).unwrap();
    assert_eq!(store.query_rows("readings", &RowsQuery { limit: 10, ..Default::default() }).unwrap().total, 1);
    assert_eq!(store.query_table_series("readings", "temp", None, 0, i64::MAX).unwrap().len(), 1);
    let tables = store.list_tables().unwrap();
    assert!(tables.iter().any(|t| t.name == "readings" && !t.columns.is_empty()), "{tables:?}");
    let refv = tables.iter().find(|t| t.name == "refv").expect("the view is still listed");
    assert!(refv.columns.is_empty(), "with no columns while its file is gone: {refv:?}");
    assert!(store.query_rows("refv", &RowsQuery { limit: 10, ..Default::default() }).is_err(), "the view itself cannot read");
    std::fs::write(&csv, "sensor,label\n1,inlet\n2,outlet\n").unwrap();
    assert_eq!(store.query_rows("refv", &RowsQuery { limit: 10, ..Default::default() }).unwrap().total, 2, "the replacement seen on the next query");
}

/// Rows that share a time read once each across pages: the sequence breaks the tie, so a walk
/// of one row at a time, or seven, sees every row exactly once.
#[test]
fn newest_first_pages_neither_repeat_nor_skip_rows_that_share_a_time() {
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[]).unwrap();
    let rows: Vec<WideRow> = (0..50u64).map(|i| wide("a", None, 1000 + (i % 2) as i64, i, vec![Some(Value::Float(i as f64)), None])).collect();
    store.append_wide(0, &rows).unwrap();
    for page_size in [1usize, 7] {
        let mut seen = std::collections::BTreeSet::new();
        let mut offset = 0;
        loop {
            let page = store.query_rows("readings", &RowsQuery { limit: page_size, offset, ..Default::default() }).unwrap();
            if page.rows.is_empty() {
                break;
            }
            let seq_col = page.columns.iter().position(|c| c.name == "seq").unwrap();
            for row in &page.rows {
                assert!(seen.insert(row[seq_col].as_u64().unwrap()), "row {:?} seen twice at page size {page_size}", row[seq_col]);
            }
            offset += page_size;
        }
        assert_eq!(seen.len(), 50, "page size {page_size}");
    }
}

/// Every cell of a page is typed JSON: a date and a time of day as milliseconds like a
/// timestamp, a decimal a number, an interval an ISO 8601 duration; never the engine's debug
/// text for a type the suite's tables never make.
#[test]
fn a_page_types_dates_decimals_and_intervals_never_as_debug_text() {
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[TableDef {
            name: "odd".into(),
            sql: "SELECT CAST(1 AS BIGINT) AS index_ms, 1.5 AS dec, DATE '2026-01-02' AS d, TIME '01:02:03' AS t, INTERVAL 1 HOUR AS iv".into(),
        }],
    )
    .unwrap();
    let page = store.query_rows("odd", &RowsQuery { limit: 10, ..Default::default() }).unwrap();
    let cell = |name: &str| page.rows[0][page.columns.iter().position(|c| c.name == name).unwrap()].clone();
    assert_eq!(cell("dec"), serde_json::json!(1.5));
    assert_eq!(cell("d"), serde_json::json!(1_767_312_000_000i64), "2026-01-02 at midnight, in milliseconds");
    assert_eq!(cell("t"), serde_json::json!(3_723_000i64), "01:02:03 past midnight, in milliseconds");
    assert_eq!(cell("iv"), serde_json::json!("P0M0DT3600.000000000S"));
    for c in &page.rows[0] {
        assert!(!c.to_string().contains("Date32") && !c.to_string().contains("Decimal(") && !c.to_string().contains("Interval {"), "{c}");
    }
}

/// A column two tables of one source share is one series on the narrow surface: counted
/// together, the last value the later one, as the window already merged their points.
#[test]
fn a_column_two_tables_of_one_source_share_is_one_series_in_the_listing() {
    let spec = |name: &str| BaseTableSpec { name: name.into(), columns: vec![("status".into(), SeriesType::Int)], has_key: false };
    let store = DuckStore::open(&Residency::Ram, &[spec("m1"), spec("m2")], &[]).unwrap();
    store
        .append_wide(0, &[wide("tg", None, 10, 1, vec![Some(Value::Int(1))]), wide("tg", None, 20, 2, vec![Some(Value::Int(2))]), wide("tg", None, 30, 3, vec![Some(Value::Int(3))])])
        .unwrap();
    store.append_wide(1, &[wide("tg", None, 5, 1, vec![Some(Value::Int(9))])]).unwrap();
    assert_eq!(store.query_numeric(&SeriesRef::new("tg", "status"), None, 0, i64::MAX).len(), 4);
    let infos = store.list();
    let status = infos.iter().filter(|i| i.series == SeriesRef::new("tg", "status")).collect::<Vec<_>>();
    assert_eq!(status.len(), 1, "{infos:?}");
    assert_eq!((status[0].count, status[0].last_index_ms, status[0].last_value.as_str()), (4, 30, "3"));
}

/// A session store is fresh: a database already at its path is refused by name, never a second
/// session's rows beside the first's.
#[test]
fn a_disk_store_refuses_a_database_already_at_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session.duckdb");
    let residency = Residency::Disk { path: db.clone(), memory_limit_mb: 64 };
    drop(DuckStore::open(&residency, &[capture_spec()], &[]).unwrap());
    let err = DuckStore::open(&residency, &[capture_spec()], &[]).err().expect("a database already at the path is refused");
    assert!(matches!(err, StoreError::Open(_)), "{err}");
    let text = err.to_string();
    assert!(text.contains(&db.display().to_string()) && text.contains("already stands there"), "{text}");
}

/// A key asked of a table or view with no key column is a name the catalog lacks: refused as
/// one, never a filter silently dropped.
#[test]
fn a_chart_s_key_on_a_keyless_table_is_refused_not_dropped() {
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[TableDef { name: "v".into(), sql: "SELECT index_ms, temp AS f FROM readings".into() }]).unwrap();
    store.append_wide(0, &[wide("a", Some("k"), 1000, 1, vec![Some(Value::Float(1.0)), None])]).unwrap();
    assert_eq!(store.query_table_series("v", "f", None, 0, i64::MAX).unwrap().len(), 1);
    assert_eq!(store.query_table_series("v", "f", Some("zzz"), 0, i64::MAX).unwrap_err().to_string(), "unknown table `v.key`");
    assert!(store.query_table_series("readings", "temp", Some("zzz"), 0, i64::MAX).unwrap().is_empty(), "a keyed table: empty for an unknown key");
}
