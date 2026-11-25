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

/// The options of a file read in `d` with the reader's own quote and escape.
fn delim(d: &str) -> CsvOptions {
    CsvOptions { delim: Some(d.to_string()), ..CsvOptions::default() }
}

/// The options a ragged file is forced onto: the sniffer's delimiter, quote and escape, written
/// out (empty for one it did not see).
fn dialect(d: &str, quote: &str, escape: &str) -> CsvOptions {
    CsvOptions { delim: Some(d.to_string()), quote_char: Some(quote.to_string()), escape_char: Some(escape.to_string()), decimal_comma: false }
}

/// A file the sniffer saw no quote in.
fn unquoted(d: &str) -> CsvOptions {
    dialect(d, "", "")
}

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
        secret: None,
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
    // A definition is one SELECT: a second statement in its text is refused, at validation and
    // when applied, and none of it runs (a COPY that would write a file writes none), however it
    // is spelled (closing the SELECT's parenthesis, after a `;`, inside dollar quotes, alone); a
    // `;` inside a string of one SELECT is no second statement.
    let out = tempfile::tempdir().unwrap();
    let forms = ["SELECT 1); COPY (SELECT 1) TO '{f}'; SELECT (1", "SELECT 1; COPY (SELECT 1) TO '{f}'", "SELECT $$x$$); COPY (SELECT 1) TO '{f}'; SELECT ($$y$$", "COPY (SELECT 1) TO '{f}'"];
    for (i, form) in forms.iter().enumerate() {
        let f = out.path().join(format!("{i}.csv")).display().to_string().replace('\\', "/");
        let sql = form.replace("{f}", &f);
        assert!(store.validate_definition(&sql).is_err(), "{sql}");
        let opened = DuckStore::open(&Residency::Ram, &[capture_spec()], &[TableDef { name: "evil".into(), sql: sql.clone() }]);
        assert!(matches!(opened, Err(StoreError::Definition { .. })), "{sql}");
        assert!(!std::path::Path::new(&f).exists(), "a statement of the definition ran: {sql}");
    }
    let cols = store.validate_definition("SELECT key, ';' AS sep FROM readings").unwrap();
    assert_eq!(cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["key", "sep"]);
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
    assert_eq!(
        csv_scan_sql("x.csv", &CsvOptions { delim: Some(";".into()), decimal_comma: true, ..CsvOptions::default() }),
        "read_csv('x.csv', delim=';', null_padding=true, parallel=false, decimal_separator=',')"
    );
    assert_eq!(
        csv_scan_sql("x.csv", &CsvOptions { delim: Some(",".into()), quote_char: Some("'".into()), escape_char: Some("\\".into()), decimal_comma: false }),
        r"read_csv('x.csv', delim=',', quote='''', escape='\', null_padding=true, parallel=false)"
    );
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

    // Any ragged file takes the padded reader, not the collapsed one alone (docs/questions.md
    // Q17): a short row and a long row the sniffer would keep, drop or rename on its own terms
    // are read with every row kept, the short one padded, the long one widening the schema.
    let short = write("short.csv", "a,b,c\r\n1,2,3\r\n4,5\r\n6,7,8\r\n");
    assert!(csv_is_ragged(&short, &delim(",")), "a short row is ragged");
    let opts = sniff_csv_options(&short).unwrap();
    assert_eq!(opts.delim.as_deref(), Some(","), "the padded reader, forced by the sniffer's delimiter");
    let page = preview_csv(&short, 10, &opts).unwrap();
    assert_eq!((page.total, page.columns.len()), (3, 3), "{:?}", page.columns);
    let long = write("long.csv", "a;b\r\n1;2\r\n3;4;5\r\n6;7\r\n");
    assert!(csv_is_ragged(&long, &delim(";")));
    let opts = sniff_csv_options(&long).unwrap();
    assert_eq!(opts.delim.as_deref(), Some(";"));
    let page = preview_csv(&long, 10, &opts).unwrap();
    assert_eq!((page.total, page.columns.len()), (3, 3), "the long row widens the schema, every row kept: {:?}", page.columns);
    // A regular file stays the sniffer's: a delimiter inside quotes is no field, a blank last
    // line is no row, and a ragged row past the 2 048-line window is not the window's to count.
    let regular = write("regular.csv", "a,b\r\n\"x,y\",2\r\n3,4\r\n\r\n");
    assert!(!csv_is_ragged(&regular, &delim(",")));
    assert_eq!(sniff_csv_options(&regular).unwrap().delim, None);
    let mut far = String::from("a,b\r\n");
    for i in 0..2100 {
        far.push_str(&format!("{i},x\r\n"));
    }
    far.push_str("1,2,3\r\n");
    let far = write("far.csv", &far);
    assert!(!csv_is_ragged(&far, &delim(",")), "a long row past the window is the engine's to refuse by its line");
    let mut near = String::from("a,b\r\n");
    for i in 0..100 {
        near.push_str(&format!("{i},x\r\n"));
    }
    near.push_str("1,2,3\r\n");
    assert!(csv_is_ragged(&write("near.csv", &near), &delim(",")));
    assert!(!csv_is_ragged("nope.csv", &delim(",")), "a missing file is not ragged, it is an error elsewhere");
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
/// Round 15's four. An open refused on disk (a view over a file not there, a reserved table name)
/// leaves nothing at its path, so the same open, its cause fixed, is not refused for a database
/// of its own failed start. The last value of a series is its last point by stamp then sequence,
/// two keys sharing a stamp (a keyed poll), the window's last alike, in a base table and in the
/// samples. A key whose rows hold only holes in a column is no key of that column. And a page's
/// window holds both its ends.
#[test]
fn a_refused_open_leaves_nothing_the_last_value_at_a_shared_stamp_keys_with_holes_and_a_page_s_edges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store").join("session.duckdb");
    let wal = std::path::PathBuf::from(format!("{}.wal", path.display()));
    let csv = dir.path().join("ref.csv");
    let view = || [TableDef { name: "refv".into(), sql: format!("SELECT * FROM {}", csv_scan_sql(&csv.display().to_string(), &CsvOptions::default())) }];
    let disk = Residency::Disk { path: path.clone(), memory_limit_mb: 0 };
    assert!(matches!(DuckStore::open(&disk, &[capture_spec()], &view()), Err(StoreError::Definition { .. })));
    assert!(!path.exists() && !wal.exists(), "a refused open leaves no file");
    let reserved = BaseTableSpec { name: "samples".into(), ..capture_spec() };
    assert!(matches!(DuckStore::open(&disk, &[reserved], &[]), Err(StoreError::TableName(_))));
    assert!(!path.exists() && !wal.exists(), "a refused name leaves no file");
    std::fs::write(&csv, "sensor,label\n1,inlet\n").unwrap();
    let store = DuckStore::open(&disk, &[capture_spec()], &view()).expect("the cause fixed, the same open");
    drop(store);

    // Two keys at one stamp: the last value the second appended, as the window's last point.
    let store = DuckStore::open(&Residency::Ram, &[capture_spec()], &[]).unwrap();
    store
        .append_wide(0, &[wide("a", Some("A"), 1000, 1, vec![Some(Value::Float(20.0)), Some(Value::Text("ok".into()))]), wide("a", Some("B"), 1000, 2, vec![Some(Value::Float(30.0)), None])])
        .unwrap();
    let infos = store.list();
    let temp = infos.iter().find(|i| i.series == SeriesRef::new("a", "temp")).unwrap();
    let last = *store.query_numeric(&SeriesRef::new("a", "temp"), None, 0, i64::MAX).last().unwrap();
    assert_eq!((temp.last_index_ms, temp.last_value.as_str(), last.seq, last.value), (1000, "30", 2, 30.0));
    // A key holding only holes in `tag` is no key of it.
    let tag = infos.iter().find(|i| i.series == SeriesRef::new("a", "tag")).unwrap();
    assert_eq!((temp.keys, tag.keys, tag.count), (2, 1, 1));
    // The samples alike: two at one stamp, the second's kind and text the last.
    let ram = ram();
    ram.append_samples(&[sample("x", 1000, 1, Value::Text("first".into()), Some("A")), sample("x", 1000, 2, Value::Int(7), Some("B"))]).unwrap();
    let x = ram.list().into_iter().find(|i| i.series == r("x")).unwrap();
    assert_eq!((x.ty, x.last_value.as_str(), x.keys), (SeriesType::Int, "7", 2));
    // A page's window holds both its ends.
    store.append_wide(0, &[wide("a", Some("A"), 2000, 3, vec![Some(Value::Float(21.0)), None])]).unwrap();
    let page = |from_ms, to_ms| store.query_rows("readings", &RowsQuery { from_ms, to_ms, limit: 10, offset: 0 }).unwrap().total;
    assert_eq!((page(Some(1000), Some(2000)), page(Some(1001), Some(2000)), page(Some(1000), Some(1999)), page(Some(2000), Some(2000))), (3, 1, 2, 1));
}

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

// ---- the second refute round -----------------------------------------------------------------

/// Two sources sharing a table share sequence numbers and times: every other column breaks the
/// tie, so a page walk sees each row once; and a view with a time column and no sequence pages
/// the same way.
#[test]
fn pages_see_each_row_once_across_sources_and_on_a_view_without_a_sequence() {
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[TableDef { name: "v".into(), sql: "SELECT source, index_ms, temp AS f FROM readings".into() }],
    )
    .unwrap();
    let mut rows = Vec::new();
    for source in ["a", "b"] {
        for i in 0..25u64 {
            rows.push(wide(source, None, 1000 + (i % 2) as i64, i, vec![Some(Value::Float((i * 10) as f64 + if source == "a" { 0.0 } else { 0.5 })), None]));
        }
    }
    store.append_wide(0, &rows).unwrap();
    for (table, a, b) in [("readings", "source", "seq"), ("v", "source", "f")] {
        for page_size in [1usize, 7] {
            let mut seen = std::collections::BTreeSet::new();
            let mut offset = 0;
            loop {
                let page = store.query_rows(table, &RowsQuery { limit: page_size, offset, ..Default::default() }).unwrap();
                if page.rows.is_empty() {
                    break;
                }
                let ca = page.columns.iter().position(|c| c.name == a).unwrap();
                let cb = page.columns.iter().position(|c| c.name == b).unwrap();
                for row in &page.rows {
                    assert!(seen.insert((row[ca].to_string(), row[cb].to_string())), "{table}: row {:?} seen twice at page size {page_size}", (&row[ca], &row[cb]));
                }
                offset += page_size;
            }
            assert_eq!(seen.len(), 50, "{table} at page size {page_size}");
        }
    }
}

/// A time's last value keeps its microseconds, as the suite displays a time and as the narrow
/// samples already listed it.
#[test]
fn a_time_s_last_value_keeps_its_microseconds() {
    let t = NaiveDateTime::parse_from_str("2026-07-18 10:00:00.123456", "%Y-%m-%d %H:%M:%S%.f").unwrap();
    let store = DuckStore::open(&Residency::Ram, &[BaseTableSpec { name: "times".into(), columns: vec![("at".into(), SeriesType::Time)], has_key: false }], &[]).unwrap();
    store.append_wide(0, &[wide("s", None, 1000, 1, vec![Some(Value::Time(t))])]).unwrap();
    store.append_samples(&[sample("tt", 1000, 1, Value::Time(t), None)]).unwrap();
    let infos = store.list();
    let last = |series: &str| infos.iter().find(|i| i.series == SeriesRef::new("s", series)).map(|i| i.last_value.clone()).unwrap_or_default();
    assert_eq!(last("at"), Value::Time(t).to_string());
    assert_eq!(last("at"), "2026-07-18 10:00:00.123456");
    assert_eq!(last("at"), last("tt"), "the two surfaces agree");
}

/// A negative interval reads as a negative duration, its sign on the seconds, never a floored
/// second with a positive fraction.
#[test]
fn a_negative_interval_keeps_its_sign() {
    let store = DuckStore::open(
        &Residency::Ram,
        &[capture_spec()],
        &[TableDef { name: "odd".into(), sql: "SELECT CAST(1 AS BIGINT) AS index_ms, INTERVAL '-1.5 seconds' AS neg, INTERVAL '1.5 seconds' AS pos".into() }],
    )
    .unwrap();
    let page = store.query_rows("odd", &RowsQuery { limit: 10, ..Default::default() }).unwrap();
    let cell = |name: &str| page.rows[0][page.columns.iter().position(|c| c.name == name).unwrap()].clone();
    assert_eq!(cell("neg"), serde_json::json!("P0M0DT-1.500000000S"));
    assert_eq!(cell("pos"), serde_json::json!("P0M0DT1.500000000S"));
}

// ---- the fourth refute round (2026-11-22) ----------------------------------------------------

/// A file `body` written as `name`, its path.
fn csv_file(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}

/// A header `a,b,c`, regular rows up to line `last - 1`, then `tail` from line `last` on.
fn long_file(last: usize, tail: &str) -> String {
    let mut s = String::from("a,b,c\n");
    for i in 2..last {
        s.push_str(&format!("{i},1,2\n"));
    }
    s.push_str(tail);
    s
}

/// What a preview of `path` reads with the options the sniffer settles on: the total, the column
/// names, the first rows.
fn read_as_sniffed(path: &str) -> (CsvOptions, RowsPage) {
    let opts = sniff_csv_options(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let page = preview_csv(path, 500, &opts).unwrap_or_else(|e| panic!("{path} with {opts:?}: {e}"));
    (opts, page)
}

fn names(page: &RowsPage) -> Vec<&str> {
    page.columns.iter().map(|c| c.name.as_str()).collect()
}

/// Q17's own case: a long row the sniffer does not collapse (it keeps the long row alone) takes
/// the padded reader, every row kept; the 2 048-line window holds at its edge (a long last row
/// on line 2 048 ragged and widening, on line 2 049 not the window's).
#[test]
fn a_long_row_the_sniffer_keeps_alone_takes_the_padded_reader_and_the_window_holds_at_its_edge() {
    let dir = tempfile::tempdir().unwrap();
    let last = csv_file(dir.path(), "last.csv", "a,b,c\n1,2,3\n4,5,6\n9,9,9,9\n");
    assert!(!csv_looks_unsplit(&last, &CsvOptions::default()), "the sniffer does not collapse it");
    assert_eq!(preview_csv(&last, 10, &CsvOptions::default()).unwrap().total, 1, "the sniffer alone keeps the long row only");
    assert!(csv_is_ragged(&last, &delim(",")));
    let (opts, page) = read_as_sniffed(&last);
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, page.columns.len()), (3, 4), "{:?}", page.columns);
    assert_eq!(page.rows[0], [serde_json::json!(1), serde_json::json!(2), serde_json::json!(3), serde_json::Value::Null]);
    assert_eq!(page.rows[2], [serde_json::json!(9), serde_json::json!(9), serde_json::json!(9), serde_json::json!(9)]);

    // The window's edge: the long row last, on line 2 047, 2 048 and 2 049.
    let at = |line: usize| csv_file(dir.path(), &format!("edge{line}.csv"), &long_file(line, "9,9,9,9\n"));
    assert!(csv_is_ragged(&at(2047), &delim(",")));
    assert!(csv_is_ragged(&at(2048), &delim(",")), "line 2 048 is inside the window");
    assert!(!csv_is_ragged(&at(2049), &delim(",")), "line 2 049 is past it");
    let (opts, page) = read_as_sniffed(&at(2048));
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, page.columns.len()), (2047, 4), "every row kept, the schema widened: {:?}", page.columns);
    // A carriage return and line feed is one line end: the same edge in a CRLF file.
    let crlf = |line: usize| csv_file(dir.path(), &format!("crlf{line}.csv"), &long_file(line, "9,9,9,9\n").replace('\n', "\r\n"));
    assert!(csv_is_ragged(&crlf(2048), &delim(",")));
    assert!(!csv_is_ragged(&crlf(2049), &delim(",")));
}

/// A file the sniffer cannot read at all is read padded in the delimiter that splits it widest:
/// a long row inside the window with a row after it, a short row past the window. The sniffer's
/// refusal stands where no delimiter splits it (a missing file).
#[test]
fn a_file_the_sniffer_refuses_is_read_padded_when_a_delimiter_splits_it() {
    let dir = tempfile::tempdir().unwrap();
    let inside = csv_file(dir.path(), "inside.csv", &long_file(2048, "9,9,9,9\n7,7,7\n"));
    assert!(csv_is_ragged(&inside, &delim(",")));
    let (opts, page) = read_as_sniffed(&inside);
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, page.columns.len()), (2048, 4), "{:?}", page.columns);
    let short_far = csv_file(dir.path(), "short_far.csv", &long_file(2102, "4,5\n7,7,7\n"));
    assert!(!csv_is_ragged(&short_far, &delim(",")), "the short row is past the window");
    let (opts, page) = read_as_sniffed(&short_far);
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, names(&page)), (2102, vec!["a", "b", "c"]), "every row kept, the short one padded");
    let missing = dir.path().join("nope.csv").display().to_string();
    let e = sniff_csv_options(&missing).unwrap_err().to_string();
    assert!(e.contains("nope.csv"), "the sniffer's own refusal, naming the file: {e}");
}

/// A long row past the window is refused by its line, by the preview and by a page of the view
/// over the file alike: never a total of nothing beside the rows a LIMIT read before the line.
#[test]
fn a_long_row_past_the_window_is_refused_by_its_line_never_a_total_of_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // Ragged inside the window (a short row on line 3), and a long row on line 2 104.
    let mut body = String::from("a,b,c\n1,2,3\n4,5\n");
    for i in 0..2100 {
        body.push_str(&format!("{i},1,2\n"));
    }
    body.push_str("9,9,9,9\n");
    let file = csv_file(dir.path(), "far_long.csv", &body);
    let opts = sniff_csv_options(&file).unwrap();
    assert_eq!(opts, unquoted(","));
    let e = preview_csv(&file, 10, &opts).unwrap_err().to_string();
    assert!(e.contains("Line: 2104"), "the preview refused by the line: {e}");
    let store = DuckStore::open(&Residency::Ram, &[], &[TableDef { name: "linked".into(), sql: format!("SELECT * FROM {}", csv_scan_sql(&file, &opts)) }]).unwrap();
    let e = store.query_rows("linked", &RowsQuery { limit: 10, ..RowsQuery::default() }).unwrap_err().to_string();
    assert!(e.contains("Line: 2104"), "the page refused by the line: {e}");
    // Regular inside the window, the long row past it: the sniffer refuses the file, the padded
    // reader takes it, and the read is refused by the long row's line.
    let mut far = String::from("a,b\r\n");
    for i in 0..2100 {
        far.push_str(&format!("{i},x\r\n"));
    }
    far.push_str("1,2,3\r\n");
    let far = csv_file(dir.path(), "far.csv", &far);
    let opts = sniff_csv_options(&far).unwrap();
    assert_eq!(opts, unquoted(","));
    let e = preview_csv(&far, 10, &opts).unwrap_err().to_string();
    assert!(e.contains("Line: 2102"), "{e}");
}

/// The padded reader keeps a quoted line break, and reads with the quote and the escape the
/// sniffer saw; a regular file quoted otherwise than `"` stays the sniffer's. A line may end in a
/// bare carriage return.
#[test]
fn quotes_escapes_and_line_endings_are_read_as_the_file_writes_them() {
    let dir = tempfile::tempdir().unwrap();
    // A quoted line break in a ragged file: every row kept, the cell whole.
    let qnl = csv_file(dir.path(), "qnl.csv", "a,b,c\n1,\"x\ny\",3\n4,5\n6,7,8\n");
    let (opts, page) = read_as_sniffed(&qnl);
    assert_eq!(opts, dialect(",", "\"", ""));
    assert_eq!((page.total, names(&page)), (3, vec!["a", "b", "c"]));
    assert_eq!(page.rows[0][1], serde_json::json!("x\ny"));
    assert_eq!(page.rows[1][2], serde_json::Value::Null, "the short row padded");
    // A regular file quoted with `'`, a line break inside the quotes: not ragged in its own
    // quote, the sniffer's to read.
    let squote = csv_file(dir.path(), "squote.csv", "a,b\n'x\ny',2\n3,4\n");
    assert!(csv_is_ragged(&squote, &delim(",")), "counted in `\"` it would look ragged");
    assert!(!csv_is_ragged(&squote, &CsvOptions { quote_char: Some("'".into()), ..delim(",") }));
    let (opts, page) = read_as_sniffed(&squote);
    assert_eq!(opts, CsvOptions::default());
    assert_eq!((page.total, page.rows[0][0].clone()), (2, serde_json::json!("x\ny")));
    // A ragged file quoted with `'`: the padded reader with that quote.
    let sragged = csv_file(dir.path(), "sragged.csv", "a,b,c\n'x,y',2,3\n4,5\n6,7,8\n");
    let (opts, page) = read_as_sniffed(&sragged);
    assert_eq!(opts, dialect(",", "'", ""));
    assert_eq!((page.total, page.rows[0][0].clone(), page.rows[1][2].clone()), (3, serde_json::json!("x,y"), serde_json::Value::Null));
    // A backslash escape inside quotes: an escaped quote is no quote.
    let escaped = csv_file(dir.path(), "escaped.csv", "a,b,c\n\"x\\\",y\",2,3\n4,5\n6,7,8\n");
    assert!(csv_is_ragged(&escaped, &CsvOptions { escape_char: Some("\\".into()), ..delim(",") }));
    // Regular once the escape is honoured; an escaped quote taken for a closing one would leave
    // the next quote open to the end of the file, and the row short.
    let escaped_regular = csv_file(dir.path(), "escaped_regular.csv", "a,b\n\"x\\\",y\",2\n3,4\n");
    assert!(!csv_is_ragged(&escaped_regular, &CsvOptions { escape_char: Some("\\".into()), ..delim(",") }));
    assert!(csv_is_ragged(&escaped_regular, &delim(",")), "read without the escape it is ragged");
    let (opts, page) = read_as_sniffed(&escaped);
    assert_eq!(opts, dialect(",", "\"", "\\"));
    assert_eq!((page.total, page.rows[0][0].clone()), (3, serde_json::json!("x\",y")));
    // Lines ending in a bare carriage return: a long row seen, every row kept.
    let cr = csv_file(dir.path(), "cr.csv", "a,b,c\r1,2,3\r4,5,6\r9,9,9,9\r");
    assert!(csv_is_ragged(&cr, &delim(",")));
    assert!(!csv_is_ragged(&csv_file(dir.path(), "cr_regular.csv", "a,b\r1,2\r3,4\r"), &delim(",")));
    assert!(!csv_is_ragged(&csv_file(dir.path(), "crlf_regular.csv", "a,b\r\n1,2\r\n3,4\r\n\r\n"), &delim(",")), "a CRLF is one line end");
    let (opts, page) = read_as_sniffed(&cr);
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, page.columns.len()), (3, 4), "{:?}", page.columns);
}

// ---- the fifth refute round (2026-11-22) -----------------------------------------------------

/// The window counts rows, as the engine's does: a blank line moves it by none, so a long row on
/// the 2 048th row is inside it on physical line 2 049 and the read is refused by line 2 050 one
/// row on.
#[test]
fn a_blank_line_moves_the_window_by_none() {
    let dir = tempfile::tempdir().unwrap();
    let blank = |line: usize| {
        let mut s = String::from("a,b,c\n\n");
        for i in 3..line {
            s.push_str(&format!("{i},1,2\n"));
        }
        s.push_str("9,9,9,9\n");
        csv_file(dir.path(), &format!("blank{line}.csv"), &s)
    };
    let inside = blank(2049);
    assert!(csv_is_ragged(&inside, &delim(",")), "the 2 048th row, on line 2 049");
    let (opts, page) = read_as_sniffed(&inside);
    assert_eq!(opts, unquoted(","));
    assert_eq!((page.total, names(&page)), (2047, vec!["a", "b", "c", "column3"]));
    let past = blank(2050);
    assert!(!csv_is_ragged(&past, &delim(",")), "the 2 049th row is past it");
    let opts = sniff_csv_options(&past).unwrap();
    let e = preview_csv(&past, 10, &opts).unwrap_err().to_string();
    assert!(e.contains("Line: 2050"), "{e}");
}

/// The forced reader is given the sniffer's quote, never left to find one: a quoted line break
/// near the window's end and a long row on its last row read whole, where a reader left to
/// itself refuses the file (C-042).
#[test]
fn the_forced_reader_reads_with_the_sniffer_s_quote() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::from("a,b,c\n1,\"x\ny\",3\n");
    for i in 3..2048 {
        body.push_str(&format!("{i},1,2\n"));
    }
    body.push_str("9,9,9,9\n");
    let file = csv_file(dir.path(), "late.csv", &body);
    assert!(csv_is_ragged(&file, &dialect(",", "\"", "\"")), "the long row is the 2 048th row");
    let (opts, page) = read_as_sniffed(&file);
    assert_eq!(opts, dialect(",", "\"", "\""));
    assert_eq!((page.total, names(&page)), (2047, vec!["a", "b", "c", "column3"]));
    assert_eq!(page.rows.iter().filter(|r| r[1] == serde_json::json!("x\ny")).count(), 1, "the quoted line break whole");
    assert!(preview_csv(&file, 10, &CsvOptions { quote_char: None, escape_char: None, ..opts.clone() }).is_err(), "a reader left to find its own quote refuses it");
}

/// A quote opens a field only at its start, and only the quote the sniffer saw: a stray `"`
/// inside a field of a file with no quoting is text, so the long row after it is seen; inside a
/// quoted field a doubled quote is text, and an escape outside quotes is text.
#[test]
fn a_quote_opens_a_field_only_at_its_start_and_only_the_sniffer_s() {
    let dir = tempfile::tempdir().unwrap();
    let inches = csv_file(dir.path(), "inches.csv", "item,size,qty\nruler,12\" long,3\npipe,M8,2\nbolt,M8,3/4\" wide,5\n");
    assert!(csv_is_ragged(&inches, &unquoted(",")));
    assert!(csv_is_ragged(&inches, &delim(",")), "in `\"` too: neither stray quote starts a field");
    let (opts, page) = read_as_sniffed(&inches);
    assert_eq!(opts, unquoted(","));
    // Every line kept; the engine reads an all-text file's header as a row on the padded path.
    assert_eq!((page.total, page.columns.len()), (4, 4), "{:?}", page.columns);
    assert_eq!(page.rows[1][1], serde_json::json!("12\" long"));
    assert_eq!(page.rows[3][2], serde_json::json!("3/4\" wide"));
    // A doubled quote inside quotes is one quote: the file is regular.
    let doubled = csv_file(dir.path(), "doubled.csv", "a,b\n\"x\"\",y\",2\n3,4\n");
    assert!(!csv_is_ragged(&doubled, &delim(",")));
    // An escape outside quotes is text.
    let bare = csv_file(dir.path(), "bare.csv", "a,b\nx\\,y\n3,4\n");
    assert!(!csv_is_ragged(&bare, &dialect(",", "\"", "\\")));
    // A ragged file in tabs, its long row last: the tab the sniffer names.
    let tabs = csv_file(dir.path(), "tabs_long.csv", "a\tb\tc\n1\t2\t3\n4\t5\t6\n9\t9\t9\t9\n");
    let (opts, page) = read_as_sniffed(&tabs);
    assert_eq!(opts, unquoted("\t"));
    assert_eq!((page.total, names(&page)), (3, vec!["a", "b", "c", "column3"]));
    // A file the sniffer refuses and no delimiter splits keeps the sniffer's refusal.
    let binary = dir.path().join("binary.csv");
    std::fs::write(&binary, (0u8..=255).cycle().take(4096).collect::<Vec<u8>>()).unwrap();
    let e = sniff_csv_options(&binary.display().to_string()).unwrap_err().to_string();
    assert!(e.contains("binary.csv"), "{e}");
}

// ---- the sixth refute round (2026-11-22) -----------------------------------------------------

/// A quote either sniff saw: a ragged file the sniffer reads as it is with no quote, where asked
/// to pad it sees `"`, is read with `"`, every quoted cell whole (round 5 wrote none and split
/// them); a quote opens a field after leading spaces, as the engine reads it.
#[test]
fn a_quote_either_sniff_saw_and_a_quote_after_leading_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let addresses = csv_file(dir.path(), "addresses.csv", "name,addr,qty\nAna,\"R. Augusta 1\nLisboa\",3\nRui,Porto\nBo,\"Faro, PT\",5\n");
    let (opts, page) = read_as_sniffed(&addresses);
    assert_eq!(opts.quote_char.as_deref(), Some("\""), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["name", "addr", "qty"]));
    assert_eq!(page.rows[0][1], serde_json::json!("R. Augusta 1\nLisboa"));
    assert_eq!(page.rows[1][2], serde_json::Value::Null, "the short row padded");
    assert_eq!(page.rows[2][1], serde_json::json!("Faro, PT"));
    let mixed = csv_file(dir.path(), "mixed.csv", "a,b,c\n1,\"x\ny\",3\n4,5\n6,\"p,q\",8\n");
    let (_, page) = read_as_sniffed(&mixed);
    assert_eq!((page.total, page.rows[2][1].clone()), (3, serde_json::json!("p,q")));
    // Leading spaces before a quote: the quote is the field's, the long row last is seen.
    let spaced = csv_file(dir.path(), "spaced.csv", "id, \"name, full\",qty\n1, \"Ana, S\",3\n2, \"Rui, P\",4\n3,Bo,5,9\n");
    assert!(csv_is_ragged(&spaced, &delim(",")), "three fields, three, three, four");
    let (opts, page) = read_as_sniffed(&spaced);
    assert_eq!(opts.delim.as_deref(), Some(","));
    assert_eq!((page.total, page.columns.len()), (3, 4), "{:?}", page.columns);
    assert_eq!(page.rows[0][1], serde_json::json!("Ana, S"));
    // A space inside a field, right before a quote, is no field's start.
    assert!(!csv_is_ragged(&csv_file(dir.path(), "inner.csv", "a,b\nx \"z,1\nw,2\n"), &delim(",")));
}

/// The decimal comma on the padded path: judged by position (the engine may take the header for
/// a row in one reading and not in the other), and, for a file the sniffer collapses, judged for
/// every candidate delimiter, so a `;` file written with decimal commas is not split on `,`.
#[test]
fn the_decimal_comma_on_the_padded_path() {
    let dir = tempfile::tempdir().unwrap();
    let long = csv_file(dir.path(), "euro_long.csv", "a;b;c\n1,5;2,5;3,5\n4,5;5,5;6,5\n9;9;9;9\n");
    let (opts, page) = read_as_sniffed(&long);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["a", "b", "c", "column3"]));
    assert!(page.columns[..3].iter().all(|c| c.data_type == "DOUBLE"), "{:?}", page.columns);
    assert_eq!(page.rows[0][0], serde_json::json!(1.5));
    let short = csv_file(dir.path(), "euro_short.csv", "a;b;c\n1,5;2,5;3,5\n4,5;5,5\n6,5;7,5;8,5\n");
    let (opts, page) = read_as_sniffed(&short);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["a", "b", "c"]));
    assert_eq!((page.rows[1][1].clone(), page.rows[1][2].clone()), (serde_json::json!(5.5), serde_json::Value::Null), "the short row padded");
    // A ragged `,` file of plain numbers stays on `,`.
    let plain = csv_file(dir.path(), "plain_short.csv", "a,b,c\n1.5,2.5,3.5\n4.5,5.5\n6.5,7.5,8.5\n");
    let (opts, page) = read_as_sniffed(&plain);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma, page.total), (Some(","), false, 3));
}

// ---- the seventh refute round (2026-11-22) ---------------------------------------------------

/// Forty-eight rows `i,"p,q",3`, then `last` (and `after`).
fn quoted_rows(last: &str, after: &str) -> String {
    let mut s = String::from("a,b,c\n");
    for i in 0..48 {
        s.push_str(&format!("{i},\"p,q\",3\n"));
    }
    s.push_str(last);
    s.push_str(after);
    s
}

/// One leading space before a quote opens it, as the engine reads it; two spaces or a tab open
/// none, so the field holds the quote as text and the row is long: the long row is seen and every
/// row kept, the quoted cells whole.
#[test]
fn one_space_before_a_quote_opens_it_two_or_a_tab_do_not() {
    let dir = tempfile::tempdir().unwrap();
    for (name, last) in [("two", "1,  \"x,y\",3\n"), ("three", "1,   \"x,y\",3\n"), ("tab", "1,\t\"x,y\",3\n")] {
        let file = csv_file(dir.path(), &format!("{name}.csv"), &quoted_rows(last, ""));
        assert!(csv_is_ragged(&file, &delim(",")), "{name}: the last row is long");
        let (opts, page) = read_as_sniffed(&file);
        assert_eq!((opts.delim.as_deref(), page.total, page.columns.len()), (Some(","), 49, 4), "{name}: {opts:?} {:?}", page.columns);
        assert_eq!(page.rows[0][1], serde_json::json!("p,q"), "{name}");
    }
    let one = csv_file(dir.path(), "one.csv", &quoted_rows("1, \"x,y\",3\n", ""));
    assert!(!csv_is_ragged(&one, &delim(",")), "one space: the quote opens, three fields");
    // Two rows after the long one: every quoted cell still whole.
    let after = csv_file(dir.path(), "after.csv", &quoted_rows("1,  \"x,y\",3\n", "7,\"r,s\",9\n8,\"t,u\",9\n"));
    let (_, page) = read_as_sniffed(&after);
    assert_eq!(page.total, 51);
    assert!(page.rows.iter().filter(|r| r[1] == serde_json::json!("p,q")).count() == 48, "{:?}", &page.rows[..2]);
}

/// The sniffer's padded dialect gives way only to the decimal comma: a `|` file whose last column
/// is a list `10,20,30` types more numbers split on `,`, by splitting a text column, and stays on
/// `|`; and the decimal comma is never adopted where a column that read as numbers would stop.
#[test]
fn the_sniffed_dialect_gives_way_to_the_decimal_comma_alone() {
    let dir = tempfile::tempdir().unwrap();
    let lists = csv_file(dir.path(), "lists.csv", "id|note|vals\n1|a|10,20,30\n2|b|40,50,60\n3|c\n");
    let (opts, page) = read_as_sniffed(&lists);
    assert_eq!(opts.delim.as_deref(), Some("|"), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["id", "note", "vals"]));
    assert_eq!(page.rows[0][2], serde_json::json!("10,20,30"));
    // A regular file where the comma would turn a dotted column to text: no decimal comma.
    let mixed = csv_file(dir.path(), "mixed_decimals.csv", "a;b\n4.0;1,5\n5.0;2,5\n");
    let opts = sniff_csv_options(&mixed).unwrap();
    assert!(!opts.decimal_comma, "{opts:?}");
    let page = preview_csv(&mixed, 10, &opts).unwrap();
    assert_eq!(page.columns[0].data_type, "DOUBLE");
}

// ---- the eighth refute round (2026-11-22) ----------------------------------------------------

/// A dialect fits when the header splits into as many fields as most rows: a `;` or `|` file whose
/// rows hold commas (decimal commas, lists, surnames) is read in its own delimiter, never on `,`,
/// whatever the sniffer named.
#[test]
fn a_dialect_fits_the_header_as_it_fits_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    // Two columns of decimal commas: read on `;` with the comma.
    let two = csv_file(dir.path(), "two.csv", "t;v\n1,5;2,5\n3,5\n4,5;5,5\n");
    assert!(!csv_header_fits(&two, &delim(",")) && csv_header_fits(&two, &delim(";")));
    let (opts, page) = read_as_sniffed(&two);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["t", "v"]));
    assert_eq!((page.rows[0][0].clone(), page.rows[1][1].clone()), (serde_json::json!(1.5), serde_json::Value::Null));
    // A `|` file of lists the sniffer refuses (a long row inside the window, a row after it):
    // read on `|`, the long row widening the schema, every list whole.
    let mut body = String::from("id|note|vals\n");
    for i in 2..2048 {
        body.push_str(&format!("{i}|a|10,20,30\n"));
    }
    body.push_str("9|9|9|9\n7|x|1,2\n");
    let lists = csv_file(dir.path(), "lists_long.csv", &body);
    let (opts, page) = read_as_sniffed(&lists);
    assert_eq!(opts.delim.as_deref(), Some("|"), "{opts:?}");
    assert_eq!((page.total, names(&page)), (2048, vec!["id", "note", "vals", "column3"]));
    assert_eq!(page.rows[0][2], serde_json::json!("10,20,30"));
    // Surnames with a comma, a short row: read on `;`, every name whole.
    let names_file = csv_file(dir.path(), "surnames.csv", "name;age;city\nSilva, Ana;34;Lisboa\nCosta, Rui;28\nSousa, Bo;41;Faro\n");
    let (opts, page) = read_as_sniffed(&names_file);
    assert_eq!(opts.delim.as_deref(), Some(";"), "{opts:?}");
    assert_eq!((page.total, page.rows[0][0].clone(), page.rows[1][2].clone()), (3, serde_json::json!("Silva, Ana"), serde_json::Value::Null));
    // A dotted column beside decimal commas: on `;`, the comma refused (the dotted column would
    // turn to text).
    let mixed = csv_file(dir.path(), "dotted_and_commas.csv", "a;b;c\n7;1,5;4.0\n8;2,5\n9;3,5;6.0\n");
    let (opts, page) = read_as_sniffed(&mixed);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma, page.total), (Some(";"), false, 3), "{opts:?}");
    assert_eq!((page.rows[0][1].clone(), page.rows[0][2].clone()), (serde_json::json!("1,5"), serde_json::json!(4.0)));
    // A `|` file with a `;` decimal-comma cell: it fits on `|`, and stays there.
    let piped = csv_file(dir.path(), "piped.csv", "id|a|b|note\n1|2|3|x;1,5\n4|5|6\n7|8|9|y;2,5\n");
    let (opts, page) = read_as_sniffed(&piped);
    assert_eq!(opts.delim.as_deref(), Some("|"), "{opts:?}");
    assert_eq!((page.total, page.columns.len(), page.rows[0][3].clone()), (3, 4, serde_json::json!("x;1,5")));
    // A `;` file with a comma in a column name: the sniffer reads it on `,` and drops its header
    // line (regular on `;`, ragged on `,`); read on `;` with the comma, the name whole.
    let unit = csv_file(dir.path(), "unit.csv", "time;pressure, bar\n1,5;2,5\n3,5;4,5\n5,5;6,5\n");
    assert!(!csv_header_fits(&unit, &delim(",")) && csv_header_fits(&unit, &delim(";")));
    assert!(csv_is_ragged(&unit, &delim(",")) && !csv_is_ragged(&unit, &delim(";")));
    let (opts, page) = read_as_sniffed(&unit);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, names(&page)), (3, vec!["time", "pressure, bar"]));
    assert_eq!((page.rows[0][0].clone(), page.rows[0][1].clone()), (serde_json::json!(1.5), serde_json::json!(2.5)));
    // A title line over a `;` table: no dialect splits a header of one field, so none replaces the
    // sniffer's `;`, and the padded reader keeps the title's row (the sniffer alone drops it).
    let titled = csv_file(dir.path(), "titled.csv", "Report\na;b;c\n1;2;3\n4;5;6\n7;8;9\n");
    assert!(!csv_header_fits(&titled, &delim(";")) && !csv_header_fits(&titled, &delim("\t")));
    let (opts, page) = read_as_sniffed(&titled);
    assert_eq!(opts.delim.as_deref(), Some(";"), "{opts:?}");
    assert_eq!((page.total, names(&page)), (4, vec!["Report", "column1", "column2"]));
    // Collapsed by a short row, the padded sniff splitting on `,` the text and the lists of a `;`
    // file: read on `;`, every cell whole.
    let notes = csv_file(dir.path(), "notes.csv", "id;note\n1;a, b, c\n2\n3;d, e, f\n4;g, h, i\n");
    let lists = csv_file(dir.path(), "lists_short.csv", "k;v\nx;10,20,30\ny;40,50,60\nz\nw;70,80,90\n");
    for (file, cell) in [(notes, "a, b, c"), (lists, "10,20,30")] {
        assert!(csv_looks_unsplit(&file, &CsvOptions::default()), "{file}");
        let (opts, page) = read_as_sniffed(&file);
        assert_eq!(opts.delim.as_deref(), Some(";"), "{file}: {opts:?}");
        assert_eq!((page.total, page.columns.len(), page.rows[0][1].clone()), (4, 2, serde_json::json!(cell)), "{file}");
    }
}

/// A `;` file whose header's units hold as many commas as its rows' decimals, most rows short or
/// every one ending on `;`: read on `;` with the comma, never on `,`, where every number is cut in
/// two. A header fits no fewer fields than most rows split into and as many as one row at least;
/// among the delimiters that fit, the widest header is taken, and never one typing fewer numbers
/// than the dialect it replaces, unless that one leaves the header a single field.
#[test]
fn a_header_fits_no_fewer_fields_than_most_rows_and_a_fit_never_types_fewer_numbers() {
    let dir = tempfile::tempdir().unwrap();
    // A logger whose note is written only at an alarm.
    let alarms = csv_file(dir.path(), "alarms.csv", "Zeit;Druck, bar;Bemerkung\n10:00;2,5\n10:01;2,6\n10:02;2,7;Alarm, kurz\n10:03;2,8\n10:04;2,9\n");
    assert!(csv_header_fits(&alarms, &delim(";")), "a column most rows leave short");
    let (opts, page) = read_as_sniffed(&alarms);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, names(&page)), (5, vec!["Zeit", "Druck, bar", "Bemerkung"]));
    assert_eq!((page.rows[0][1].clone(), page.rows[2][2].clone()), (serde_json::json!(2.5), serde_json::json!("Alarm, kurz")));
    // An export ending every data row on `;`, with a short row.
    let trailing = csv_file(dir.path(), "trailing.csv", "time;pressure, bar;temp, C\n10:00;2,5;3,5;\n10:01;2,6;3,6;\n10:02;2,7\n10:03;2,8;3,8;\n");
    assert!(csv_header_fits(&trailing, &delim(",")) && !csv_header_fits(&trailing, &delim(";")), "on `,` the units split as the decimals do");
    let (opts, page) = read_as_sniffed(&trailing);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, page.rows[0][1].clone(), page.rows[0][2].clone()), (4, serde_json::json!(2.5), serde_json::json!(3.5)));
    assert_eq!(csv_fitting_dialect(&trailing, &delim(";")).delim.as_deref(), Some(";"), "`,` fits, and types no number");
    // The header against most rows, and against every row.
    let minority = csv_file(dir.path(), "minority.csv", "time;pressure, bar\n1,5;2,5\n3,5;4,5\n7;6,5\n");
    assert!(!csv_header_fits(&minority, &delim(",")), "two fields on `,` where most rows split into three");
    assert!(csv_header_fits(&minority, &delim(";")));
    let tied = csv_file(dir.path(), "tied.csv", "time;pressure, bar\n1,5;2,5\n7;6,5\n");
    assert!(csv_header_fits(&tied, &delim(",")), "rows of three and of two as many: a fit by the tie");
    assert_eq!(csv_fitting_dialect(&tied, &delim(",")).delim.as_deref(), Some(";"), "weighed: `;` types two numbers, `,` one");
    let unfilled = csv_file(dir.path(), "unfilled.csv", "a;b;c;d\n1;2\n3;4\n5;6;7\n");
    assert!(!csv_header_fits(&unfilled, &delim(";")), "a header naming more columns than any row holds");
    // Two delimiters fit a header the given one leaves whole: the wider.
    let two_fit = csv_file(dir.path(), "two_fit.csv", "a;b|c|d\n1;2|3|4\n5;6|7|8\n");
    assert_eq!(csv_fitting_dialect(&two_fit, &delim(",")).delim.as_deref(), Some("|"), "three fields on `|`, two on `;`");
}

/// Round 10: rows whose counts tie fit a header of either count, and the dialects that fit are
/// weighed by the columns they type as numbers, then the dialect itself, then the widest header,
/// then the order; where none fits, a delimiter splitting a one-field header and typing more
/// numbers is taken. A text-only file both delimiters fit stays the sniffer's (a limit).
#[test]
fn dialects_that_fit_are_weighed_by_their_numbers() {
    let dir = tempfile::tempdir().unwrap();
    // An alarm column filled every other row: the rows tie between two and three fields.
    let alarm = csv_file(dir.path(), "alarm.csv", "Zeit;Wert\n10:00;2,5\n10:01;2,6;Alarm\n");
    let (opts, page) = read_as_sniffed(&alarm);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{opts:?}");
    assert_eq!((page.total, page.rows[0][1].clone()), (2, serde_json::json!(2.5)));
    let mut body = String::from("Zeit;Wert\n");
    for i in 0..40 {
        body.push_str(&format!("10:{i:02};2,{}{}\n", i % 10, if i % 2 == 1 { ";Alarm" } else { "" }));
    }
    let alarms = csv_file(dir.path(), "alarms40.csv", &body);
    let (opts, page) = read_as_sniffed(&alarms);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma, page.total), (Some(";"), true, 40), "{opts:?}");
    let pairs = csv_file(dir.path(), "pairs.csv", "id|vals\n1|10,20\n2|30,40|x\n3|50,60\n4|70,80|y\n");
    assert_eq!(read_as_sniffed(&pairs).0.delim.as_deref(), Some("|"));
    // Two thirds of the rows carry a column the header does not name: none fits, and `;` types the
    // value where the sniffer's `,` leaves the header one field.
    let mut body = String::from("Zeit;Wert\n");
    for i in 0..40 {
        body.push_str(&format!("10:{i:02};2,{}{}\n", i % 10, if i % 3 != 0 { ";Alarm" } else { "" }));
    }
    let unnamed = csv_file(dir.path(), "unnamed.csv", &body);
    assert!(!csv_header_fits(&unnamed, &delim(";")));
    assert_eq!(csv_fitting_dialect(&unnamed, &delim(",")).delim.as_deref(), Some(";"));
    // The weighing's order, each step on its own: the numbers (`;` types the id), the dialect
    // itself on equal numbers, the order on equal widths.
    let notes = csv_file(dir.path(), "notes_units.csv", "id;note, a, b\n1;a, b, c\n2\n3;d, e, f\n4;g, h, i\n");
    assert!(csv_header_fits(&notes, &delim(",")) && csv_header_fits(&notes, &delim(";")));
    assert_eq!(csv_fitting_dialect(&notes, &delim(",")).delim.as_deref(), Some(";"), "the id a number on `;` alone");
    let text = csv_file(dir.path(), "text_tie.csv", "a;b, c, d\nx;y, z, w\nx;y, z, w;v\n");
    assert!(csv_header_fits(&text, &delim(",")) && csv_header_fits(&text, &delim(";")));
    assert_eq!(csv_fitting_dialect(&text, &delim(";")).delim.as_deref(), Some(";"), "no number either way: the dialect itself, though `,` splits the header wider");
    let town = csv_file(dir.path(), "towns.csv", "name;city\nAna;Lisboa\nRui;Porto;x\n");
    assert_eq!(csv_fitting_dialect(&town, &delim(",")).delim.as_deref(), Some(";"), "a fit by the tie, the header left whole on `,`");
    let order = csv_file(dir.path(), "order.csv", "x;y|z\n1;2|3\n4;5|6\n");
    assert_eq!(csv_fitting_dialect(&order, &delim(",")).delim.as_deref(), Some(";"), "equal numbers, equal widths: the first in the order");
    // The limit: a text-only export with as many commas in its header as in its rows fits `,` too;
    // with no number to weigh, the dialect given stands.
    let export = csv_file(dir.path(), "export.csv", "name;city, country;note\nAna;Lisboa, PT;x\nRui;Porto, PT;y\n");
    assert_eq!(csv_fitting_dialect(&export, &delim(",")).delim.as_deref(), Some(","));
}

/// The weighing's limits, at their reach (rounds 11 to 14): it counts numbers and nothing else.
/// A list of numbers on the other delimiter, in any column, integers or decimals, types its items
/// there, so a file holding one is read on that delimiter when it gains numbers there, and as
/// given on a tie: a `;` file of lists under `x,y,z` on `,` (decimals alike), its mirror on `;`, a
/// `;` file with a list of four in its middle on `,`, a `,` file with a list of four between a
/// stamp and a temperature on `;`, the stamp glued to the list's first item as a decimal; a list
/// of three there a tie, as given. And a `;` export of decimal commas with a comma in a column's
/// name and no column but its decimal typed (`Zeit;Temperatur, C`) ties with `,`, its fractions
/// integers there: as given, and a cut last line makes it ragged on the sniffer's `,`, its
/// decimals cut in two. Each reads right in the dialect a view names (`CsvOptions`), which the
/// weighing only proposes. A rule telling a list from its cut (rounds 11 and 12) broke the far
/// commoner `;` exports of decimal commas each time it was refined; the numbers alone weigh.
#[test]
fn the_weighing_counts_numbers_alone_its_limits_and_the_dialect_a_view_names() {
    let dir = tempfile::tempdir().unwrap();
    let file = |name: &str, body: &str| csv_file(dir.path(), name, body);
    let lists = file("lists.csv", "id;name;x,y,z\n1;alpha;10,20,30\n2;beta;40,50,60\n3;gamma\n");
    let decimals = file("decimal_lists.csv", "id;name;x,y,z\n1;alpha;1.5,2.5,3.5\n2;beta;4.5,5.5,6.5\n3;gamma\n");
    let mirror = file("mirror.csv", "id,name,x;y;z\n1,alpha,10;20;30\n2,beta,40;50;60\n3,gamma\n");
    let middle = file("middle4.csv", "id;x,y,z,w;name\n1;10,20,30,40;alpha\n2;50,60,70,80;beta\n3;gamma\n");
    let stamp = file("stamp4.csv", "ts,x;y;z;w,temp\n1,10;20;30;40,21.5\n2,50;60;70;80,22.5\n3,90;11;12;13,23.0\n4,14;15\n");
    for given in [",", ";"] {
        for (path, read) in [(&lists, ","), (&decimals, ","), (&mirror, ";"), (&middle, ","), (&stamp, ";")] {
            assert_eq!(csv_fitting_dialect(path, &delim(given)).delim.as_deref(), Some(read), "{path}, given {given}");
        }
    }
    let (opts, page) = read_as_sniffed(&stamp);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma, names(&page)), (Some(";"), true, vec!["ts,x", "y", "z", "w,temp"]), "{opts:?}");
    assert_eq!(page.rows[0][0], serde_json::json!(1.1), "the stamp glued to the list's first item");
    let three = file("stamp3.csv", "ts,x;y;z,temp\n1,10;20;30,21.5\n2,40;50;60,22.5\n3,70;80;90,23.0\n");
    for given in [",", ";"] {
        assert_eq!(csv_fitting_dialect(&three, &delim(given)).delim.as_deref(), Some(given), "a tie, as given {given}");
    }
    // The export of one decimal, its last line cut: on the sniffer's `,`, every decimal in two.
    let export = file("export.csv", "Zeit;Temperatur, C\n10:00:00;21,5\n10:00:01;22,0\n10:00:02;21,7\n10:00:03\n");
    for given in [",", ";"] {
        assert_eq!(csv_fitting_dialect(&export, &delim(given)).delim.as_deref(), Some(given), "a tie, as given {given}");
    }
    let (opts, page) = read_as_sniffed(&export);
    assert_eq!((opts.delim.as_deref(), page.rows[0][1].clone()), (Some(","), serde_json::json!(5)), "{opts:?}");
    // Named, each reads right.
    let named = |path: &str, d: &str, comma: bool| preview_csv(path, 10, &CsvOptions { decimal_comma: comma, ..delim(d) }).unwrap();
    let page = named(&export, ";", true);
    assert_eq!((names(&page), page.rows[0][1].clone(), page.total), (vec!["Zeit", "Temperatur, C"], serde_json::json!(21.5), 4));
    let page = named(&stamp, ",", false);
    assert_eq!((names(&page), page.rows[0].clone()), (vec!["ts", "x;y;z;w", "temp"], vec![serde_json::json!(1), serde_json::json!("10;20;30;40"), serde_json::json!(21.5)]));
    let page = named(&lists, ";", false);
    assert_eq!(page.rows[0], vec![serde_json::json!(1), serde_json::json!("alpha"), serde_json::json!("10,20,30")]);
}

/// Every way a linked CSV reads, for a person to pick where the weighing cannot tell
/// (docs/questions.md Q131): the limit's files give both readings, the weighing's proposal first,
/// each with its first rows (the `;` reading of the one-decimal export with its decimal comma, its
/// temperature 21.5); a file only one dialect fits gives that one.
#[test]
fn every_way_a_linked_csv_reads_with_its_first_rows() {
    let dir = tempfile::tempdir().unwrap();
    let json = |v: serde_json::Value| v;
    let lists = csv_file(dir.path(), "lists.csv", "id;name;x,y,z\n1;alpha;10,20,30\n2;beta;40,50,60\n3;gamma\n");
    let readings = csv_readings(&lists, 2).unwrap();
    assert_eq!(readings.iter().map(|r| r.dialect.delim.as_deref()).collect::<Vec<_>>(), [Some(","), Some(";")]);
    assert_eq!(readings[0].first_rows.rows.len(), 2, "the rows asked for");
    assert_eq!((names(&readings[1].first_rows), readings[1].first_rows.rows[0].clone()), (vec!["id", "name", "x,y,z"], vec![json(1.into()), json("alpha".into()), json("10,20,30".into())]));
    let export = csv_file(dir.path(), "export.csv", "Zeit;Temperatur, C\n10:00:00;21,5\n10:00:01;22,0\n10:00:02;21,7\n10:00:03\n");
    let readings = csv_readings(&export, 10).unwrap();
    assert_eq!(readings.iter().map(|r| (r.dialect.delim.as_deref(), r.dialect.decimal_comma)).collect::<Vec<_>>(), [(Some(","), false), (Some(";"), true)]);
    assert_eq!((names(&readings[1].first_rows), readings[1].first_rows.rows[0][1].clone(), readings[1].first_rows.total), (vec!["Zeit", "Temperatur, C"], serde_json::json!(21.5), 4));
    let plain = csv_file(dir.path(), "plain.csv", "a;b\n1;2\n3;4\n");
    let readings = csv_readings(&plain, 10).unwrap();
    assert_eq!((readings.len(), readings[0].first_rows.rows.len()), (1, 2), "{:?}", readings.iter().map(|r| &r.dialect).collect::<Vec<_>>());
}

/// Round 13's files, read on their own delimiter as round 10 read them: a `;` export of decimal
/// commas whose note column's name holds the comma (`Notiz, frei`), with a short row or a note
/// holding a comma too, on `;` whatever the dialect given (their ids give `;` the numbers); and a
/// `,` file whose list of three on `;` sits between a stamp and a temperature, regular, read by
/// the sniffer on `,` (to the weighing a tie, as given).
#[test]
fn decimal_exports_with_a_comma_in_a_note_and_a_list_between_numbers_read_on_their_own_delimiter() {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [
        ("notiz.csv", "Nr;Zaehler;Wert;Notiz, frei\n1;5;21,5;ab\n2;6;22,0;cd\n3;7;21,7;ef\n"),
        ("notiz_short.csv", "Nr;Zaehler;Wert;Notiz, frei\n1;5;21,5;ab\n2;6;22,0;cd\n3;7;21,7;ef\n4;8\n"),
        ("comment.csv", "Zeit;Zaehler;Temperatur;Kommentar, Bediener\n1;5;21,5;ab\n2;6;22,0;cd, ef\n3;7;21,7;gh\n"),
    ] {
        let file = csv_file(dir.path(), name, body);
        for given in [",", ";"] {
            assert_eq!(csv_fitting_dialect(&file, &delim(given)).delim.as_deref(), Some(";"), "{name}, given {given}");
        }
    }
    let middle = csv_file(dir.path(), "middle.csv", "ts,x;y;z,temp\n1,10;20;30,21.5\n2,40;50;60,22.5\n3,70;80;90,23.0\n");
    // Regular, it is the sniffer's to read, and the sniffer reads it on `,`.
    let (opts, page) = read_as_sniffed(&middle);
    assert_eq!(names(&page), ["ts", "x;y;z", "temp"], "{opts:?}");
    assert_eq!(page.rows[0], vec![serde_json::json!(1), serde_json::json!("10;20;30"), serde_json::json!(21.5)]);
}

/// Round 12's files: two ids glued on `,` to the integer part of a decimal (`1;5;21` of `21,5`)
/// type nothing, and `;` keeps its numbers, so `,` never cuts every decimal in two, whatever the
/// dialect given.
#[test]
fn ids_glued_to_half_a_decimal_leave_the_file_on_its_own_delimiter() {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [("counter.csv", "Nr;Zaehler;Wert, C\n1;5;21,5\n2;6;22\n3;7;21,7\n"), ("short.csv", "id;n;T, C\n1;2;21,5\n2;3;22\n3;4;23,5\n4;5\n")] {
        let file = csv_file(dir.path(), name, body);
        assert!(csv_header_fits(&file, &delim(",")) && csv_header_fits(&file, &delim(";")), "{name}");
        for given in [",", ";"] {
            assert_eq!(csv_fitting_dialect(&file, &delim(given)).delim.as_deref(), Some(";"), "{name}, given {given}");
        }
    }
}

/// A file a logger is still writing to is weighed and sniffed without a panic, whatever the
/// logger adds between two reads of it (round 11's weighing panicked there, one read's places cut
/// out of another's text).
#[test]
fn a_file_growing_while_it_is_weighed_never_panics() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::from("id;name;x,y,z\n");
    for i in 0..50 {
        body.push_str(&format!("{i};n{i};{},{},{}\n", 10 * i, 10 * i + 1, 10 * i + 2));
    }
    let path = csv_file(dir.path(), "growing.csv", &body);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let logger = {
        let (path, stop) = (path.clone(), std::sync::Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            let mut i = 50;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                // A line written in two pieces, as a logger flushing mid-line does.
                write!(f, "{i};n{i}").unwrap();
                std::thread::sleep(std::time::Duration::from_micros(200));
                writeln!(f, ";{},{},{}", 10 * i, 10 * i + 1, 10 * i + 2).unwrap();
                std::thread::sleep(std::time::Duration::from_micros(200));
                i += 1;
            }
        })
    };
    for round in 0..30 {
        let given = if round % 2 == 0 { "," } else { ";" };
        let weighed = std::panic::catch_unwind(|| csv_fitting_dialect(&path, &delim(given)));
        let sniffed = std::panic::catch_unwind(|| sniff_csv_options(&path).is_ok());
        if weighed.is_err() || sniffed.is_err() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            panic!("round {round}: a panic while the file grew");
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    logger.join().unwrap();
}

/// A headerless `;` file of decimal commas, collapsed by a short row (a logger's last line cut
/// off as it writes): on `,` it types as many numbers as on `;` with the comma (the integers and
/// the fractions, text between them), so the fewer columns left as text decide it; a full tie
/// stays the sniffer's.
#[test]
fn a_headerless_file_of_decimal_commas_cut_short_reads_on_its_own_delimiter() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("headless.csv", "1,5;2,5\n3,5;4,5\n5,5\n6,5;7,5\n", 2),
        ("cut.csv", "1,5;2,5\n3,5;4,5\n5,5;6,5\n7,\n", 2),
        ("with_text.csv", "1,5;2,5;x,7\n3,5;4,5;y,8\n5,5\n6,5;7,5;z,9\n", 3),
    ];
    for (name, body, width) in cases {
        let file = csv_file(dir.path(), name, body);
        let (opts, page) = read_as_sniffed(&file);
        assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(";"), true), "{name}: {opts:?}");
        assert_eq!((page.total, page.columns.len()), (4, width), "{name}");
        assert_eq!((page.rows[0][0].clone(), page.rows[0][1].clone()), (serde_json::json!(1.5), serde_json::json!(2.5)), "{name}");
    }
    let with_text = csv_file(dir.path(), "with_text.csv", cases[2].1);
    assert_eq!(read_as_sniffed(&with_text).1.rows[0][2], serde_json::json!("x,7"));
    // One number and one text column either way: the sniffer's `,` stands.
    let tie = csv_file(dir.path(), "tie.csv", "1,5;x\n2,5;y\n3\n4,5;z\n");
    let (opts, page) = read_as_sniffed(&tie);
    assert_eq!((opts.delim.as_deref(), opts.decimal_comma), (Some(","), false), "{opts:?}");
    assert_eq!((page.total, page.rows[0][1].clone()), (4, serde_json::json!("5;x")));
}

/// The one leading space a quote may follow is counted afresh at every field: two space-led
/// quoted fields in a row, then a long row last, read every row.
#[test]
fn the_leading_space_is_counted_afresh_at_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::from("a, \"b,x\", \"c,y\"\n");
    for i in 0..48 {
        body.push_str(&format!("{i}, \"p,q\", \"r,s\"\n"));
    }
    body.push_str("1,2,3,4\n");
    let file = csv_file(dir.path(), "two_spaced.csv", &body);
    assert!(csv_is_ragged(&file, &delim(",")), "three fields a row, four on the last");
    let (_, page) = read_as_sniffed(&file);
    assert_eq!((page.total, page.columns.len()), (49, 4), "{:?}", page.columns);
    assert_eq!((page.rows[0][1].clone(), page.rows[0][2].clone()), (serde_json::json!("p,q"), serde_json::json!("r,s")));
}



