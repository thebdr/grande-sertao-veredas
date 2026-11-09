//! The session store: an embedded DuckDB database holding every protocol's data alike.
//!
//! One wide base table per rule, whatever the rule reads (a capture rule's line, an OPC rule's
//! poll, a telegram message): the fixed identity and axis columns, then one typed column per
//! value the rule yields, one row per line, poll or message. A narrow `samples` table catches
//! bare producers. User-defined tables are SQL views over them, in one namespace, so any table
//! may read any other (a nearest-in-time join across sources, a bucketed statistic, a filter).
//! Windowed numeric queries serve the charts, typed row pages the tables, a snapshot or a CSV
//! export the archive, and a CSV the operator keeps beside the plant is read live through a
//! view, never held open.
//!
//! Threading: one database, three connections behind their own mutexes: `ingest` (the session's
//! appends), `ui` (a shell's snapshot), `api` (the server's queries), so a slow query never
//! stalls the ingest and reads never block on the writer. The store is `Send + Sync`, shared as
//! an `Arc`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use duckdb::types::{TimeUnit, Value as DuckValue};
use duckdb::{Connection, ToSql};
use thiserror::Error;

use veredas_core::{Sample, SeriesRef, SeriesType, SourceId, Value};

/// The source id of computed series (a clock-skew diagnostic, a derived metric): they belong to
/// a calculation, not to any one plant source.
pub const DERIVED_SOURCE: &str = "derived";

/// The fixed columns every base table carries; a rule's own column may not take one of them.
pub const RESERVED_COLUMNS: [&str; 6] = ["source", "key", "index_ms", "ts", "arrival_ms", "seq"];

/// The narrow table's name; a reserved table name too.
pub const SAMPLES_TABLE: &str = "samples";

/// The most rows one page of a table may hold.
pub const MAX_PAGE: usize = 2_000;

/// Typed store errors: no panic on any SQL or I/O path.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("duckdb: {0}")]
    Duck(#[from] duckdb::Error),
    #[error("table `{0}`: a duplicate, reserved or empty name")]
    TableName(String),
    #[error("table `{table}`: the column `{column}` collides with a fixed column")]
    ReservedColumn { table: String, column: String },
    #[error("table `{name}`: {message}")]
    Definition { name: String, message: String },
    #[error("unknown table `{0}`")]
    UnknownTable(String),
    #[error("{0}")]
    Open(String),
}

/// Where the session database lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Residency {
    /// In memory: the fastest, gone when the session ends unless snapshotted.
    Ram,
    /// A file on disk; the engine pages out of core, bounded by `memory_limit_mb` (0 for the
    /// engine's default); its spill lands beside the file.
    Disk { path: PathBuf, memory_limit_mb: u32 },
}

/// A base table to create: a rule's name and its value columns in the rule's order (a capture
/// rule's key group is the `key` column, not a value column).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseTableSpec {
    pub name: String,
    pub columns: Vec<(String, SeriesType)>,
    pub has_key: bool,
}

/// One user-defined table: a name and the SELECT applied as a view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDef {
    pub name: String,
    pub sql: String,
}

/// One wide row for a base table: the values in the spec's column order, `None` where a value
/// was absent or refused its type.
#[derive(Clone, Debug, PartialEq)]
pub struct WideRow {
    pub source: SourceId,
    pub key: Option<std::sync::Arc<str>>,
    pub index_ms: i64,
    pub arrival_ms: i64,
    pub seq: u64,
    pub values: Vec<Option<Value>>,
}

/// A column of a listed table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    /// A wide base table of a rule.
    Base,
    /// The narrow `samples` table.
    Samples,
    /// A user-defined view.
    View,
}

impl TableKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TableKind::Base => "base",
            TableKind::Samples => "samples",
            TableKind::View => "view",
        }
    }
}

/// A listed table with its columns and, for a table that is not a view, its exact row count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<ColumnInfo>,
    pub rows: Option<u64>,
}

/// A windowed page over a table's rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowsQuery {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

/// A page of typed rows, with the schema that positions their cells.
#[derive(Clone, Debug, PartialEq)]
pub struct RowsPage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub total: u64,
}

/// One point of a numeric window, time-ordered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub index_ms: i64,
    pub seq: u64,
    pub value: f64,
}

/// A series as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesInfo {
    pub series: SeriesRef,
    pub ty: SeriesType,
    pub count: u64,
    /// Distinct identity keys seen.
    pub keys: usize,
    pub keys_capped: bool,
    pub last_index_ms: i64,
    pub last_value: String,
}

/// The store's vitals, for a shell's snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreSnapshot {
    pub series: Vec<SeriesInfo>,
    pub total_samples: u64,
    pub approx_bytes: usize,
    pub type_conflicts: u64,
}

/// The SQL type a series type's column takes.
pub fn sql_type(ty: SeriesType) -> &'static str {
    match ty {
        SeriesType::Text => "VARCHAR",
        SeriesType::Int => "BIGINT",
        SeriesType::Float => "DOUBLE",
        SeriesType::Time => "TIMESTAMP",
    }
}

/// Quote an identifier: double-quoted, an embedded quote doubled.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Escape a string for a single-quoted SQL literal.
pub fn sql_str(s: &str) -> String {
    s.replace('\'', "''")
}

/// A created base table: its name and its value columns.
struct BaseTable {
    name: String,
    columns: Vec<(String, SeriesType)>,
}

/// Where one series name can be read from.
#[derive(Clone)]
struct SeriesSource {
    table: String,
    column: String,
    ty: SeriesType,
}

/// The embedded session store.
pub struct DuckStore {
    ingest: Mutex<Connection>,
    ui: Mutex<Connection>,
    api: Mutex<Connection>,
    bases: Vec<BaseTable>,
    /// A series name to the base-table columns that produce it; the `samples` table is always
    /// consulted too.
    registry: BTreeMap<String, Vec<SeriesSource>>,
    total: AtomicU64,
    conflicts: u64,
    residency: Residency,
}

fn recover<'a, T>(r: Result<MutexGuard<'a, T>, std::sync::PoisonError<MutexGuard<'a, T>>>) -> MutexGuard<'a, T> {
    r.unwrap_or_else(|e| e.into_inner())
}

impl DuckStore {
    /// Open a fresh session database, create the `samples` and base tables, then apply the view
    /// definitions (in any order: a fixpoint resolves the references). Every failure is typed.
    pub fn open(residency: &Residency, specs: &[BaseTableSpec], defs: &[TableDef]) -> Result<DuckStore, StoreError> {
        let conn = match residency {
            Residency::Ram => Connection::open_in_memory()?,
            Residency::Disk { path, memory_limit_mb } => {
                // A session store is fresh: a database already at the path is refused by name,
                // never a second session's rows beside the first's (docs/questions.md Q16).
                if path.exists() {
                    return Err(StoreError::Open(format!("open {}: a database already stands there; a session store is fresh, move it or snapshot elsewhere", path.display())));
                }
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent).map_err(|e| StoreError::Open(format!("create {}: {e}", parent.display())))?;
                }
                let c = Connection::open(path).map_err(|e| StoreError::Open(format!("open {}: {e}", path.display())))?;
                if *memory_limit_mb > 0 {
                    c.execute_batch(&format!("SET memory_limit='{}MiB';", memory_limit_mb))?;
                }
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    c.execute_batch(&format!("SET temp_directory='{}';", sql_str(&parent.display().to_string())))?;
                }
                c
            }
        };

        Self::create_samples(&conn)?;
        let bases = Self::create_bases(&conn, specs)?;
        Self::apply_views(&conn, defs)?;
        let (registry, conflicts) = Self::build_registry(specs);

        let ui = conn.try_clone()?;
        let api = conn.try_clone()?;
        Ok(DuckStore {
            ingest: Mutex::new(conn),
            ui: Mutex::new(ui),
            api: Mutex::new(api),
            bases,
            registry,
            total: AtomicU64::new(0),
            conflicts,
            residency: residency.clone(),
        })
    }

    /// Where this store lives.
    pub fn residency(&self) -> &Residency {
        &self.residency
    }

    fn create_samples(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE samples (
                 source   VARCHAR NOT NULL,
                 series   VARCHAR NOT NULL,
                 key      VARCHAR,
                 index_ms BIGINT  NOT NULL,
                 seq      UBIGINT NOT NULL,
                 v_num    DOUBLE,
                 v_disp   VARCHAR NOT NULL,
                 v_kind   VARCHAR NOT NULL
             );",
        )?;
        Ok(())
    }

    fn create_bases(conn: &Connection, specs: &[BaseTableSpec]) -> Result<Vec<BaseTable>, StoreError> {
        let mut seen = std::collections::HashSet::new();
        let mut bases = Vec::new();
        for spec in specs {
            let name = spec.name.trim();
            if name.is_empty()
                || name.contains('"')
                || name.eq_ignore_ascii_case(SAMPLES_TABLE)
                || !seen.insert(name.to_ascii_lowercase())
            {
                return Err(StoreError::TableName(spec.name.clone()));
            }
            let mut cols = String::from(
                "source VARCHAR NOT NULL, key VARCHAR, index_ms BIGINT NOT NULL, \
                 ts TIMESTAMP NOT NULL, arrival_ms BIGINT NOT NULL, seq UBIGINT NOT NULL",
            );
            let mut resolved = Vec::new();
            for (column, ty) in &spec.columns {
                if RESERVED_COLUMNS.iter().any(|r| r.eq_ignore_ascii_case(column)) {
                    return Err(StoreError::ReservedColumn { table: spec.name.clone(), column: column.clone() });
                }
                cols.push_str(&format!(", {} {}", quote_ident(column), sql_type(*ty)));
                resolved.push((column.clone(), *ty));
            }
            conn.execute_batch(&format!("CREATE TABLE {} ({cols});", quote_ident(name)))?;
            bases.push(BaseTable { name: name.to_string(), columns: resolved });
        }
        Ok(bases)
    }

    /// Apply the view definitions with a fixpoint, so a reference resolves whatever the order;
    /// a pass that creates nothing new means the remaining definitions have real binder errors,
    /// and the last of them is the error.
    fn apply_views(conn: &Connection, defs: &[TableDef]) -> Result<(), StoreError> {
        match Self::unresolved_views(conn, defs).pop() {
            None => Ok(()),
            Some((name, message)) => Err(StoreError::Definition { name, message }),
        }
    }

    /// The view definitions the engine refuses over the tables standing in `conn`, in their
    /// order, each with the engine's last reason, after a fixpoint (a view over another view
    /// resolves whatever the order); every view that stands is created along the way.
    fn unresolved_views(conn: &Connection, defs: &[TableDef]) -> Vec<(String, String)> {
        let mut pending: Vec<TableDef> = defs.to_vec();
        loop {
            let before = pending.len();
            let mut still = Vec::new();
            let mut reasons = Vec::new();
            for def in pending {
                let sql = format!("CREATE VIEW {} AS ({})", quote_ident(def.name.trim()), def.sql);
                match conn.execute_batch(&sql) {
                    Ok(()) => {}
                    Err(e) => {
                        reasons.push((def.name.clone(), e.to_string()));
                        still.push(def);
                    }
                }
            }
            if still.is_empty() || still.len() == before {
                return reasons;
            }
            pending = still;
        }
    }

    /// Judge view definitions over a fresh store that is never kept: the `samples` and base
    /// tables created, every view tried to a fixpoint, and the ones the engine refuses returned
    /// in their order with the engine's reason, all of them at once, so a caller deciding what
    /// to keep sees the whole picture and not the last failure alone. For the project's import
    /// (C-046), which lets the engine that will run a file judge the file before printing it.
    pub fn judge_views(specs: &[BaseTableSpec], defs: &[TableDef]) -> Result<Vec<(String, String)>, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::create_samples(&conn)?;
        Self::create_bases(&conn, specs)?;
        Ok(Self::unresolved_views(&conn, defs))
    }

    fn build_registry(specs: &[BaseTableSpec]) -> (BTreeMap<String, Vec<SeriesSource>>, u64) {
        let mut registry: BTreeMap<String, Vec<SeriesSource>> = BTreeMap::new();
        let mut types: BTreeMap<String, Vec<SeriesType>> = BTreeMap::new();
        for spec in specs {
            for (col, ty) in &spec.columns {
                registry.entry(col.clone()).or_default().push(SeriesSource {
                    table: spec.name.trim().to_string(),
                    column: col.clone(),
                    ty: *ty,
                });
                let seen = types.entry(col.clone()).or_default();
                if !seen.contains(ty) {
                    seen.push(*ty);
                }
            }
        }
        let conflicts = types.values().filter(|set| set.len() > 1).count() as u64;
        (registry, conflicts)
    }

    // ---- ingest ----------------------------------------------------------------------------

    /// Append wide rows to the base table at `table_index` (its position in the specs passed
    /// to [`open`](Self::open)).
    pub fn append_wide(&self, table_index: usize, rows: &[WideRow]) -> Result<(), StoreError> {
        if rows.is_empty() {
            return Ok(());
        }
        let Some(base) = self.bases.get(table_index) else {
            return Err(StoreError::UnknownTable(format!("#{table_index}")));
        };
        let conn = recover(self.ingest.lock());
        let mut app = conn.appender(&base.name)?;
        let mut stored = 0u64;
        for row in rows {
            let ts_us = row.index_ms.saturating_mul(1_000);
            let mut vals: Vec<DuckValue> = Vec::with_capacity(6 + row.values.len());
            vals.push(DuckValue::Text(row.source.as_str().to_string()));
            vals.push(match &row.key {
                Some(k) => DuckValue::Text(k.to_string()),
                None => DuckValue::Null,
            });
            vals.push(DuckValue::BigInt(row.index_ms));
            vals.push(DuckValue::Timestamp(TimeUnit::Microsecond, ts_us));
            vals.push(DuckValue::BigInt(row.arrival_ms));
            vals.push(DuckValue::UBigInt(row.seq));
            for cell in &row.values {
                vals.push(match cell {
                    Some(v) => {
                        stored += 1;
                        to_duck(v)
                    }
                    None => DuckValue::Null,
                });
            }
            let refs: Vec<&dyn ToSql> = vals.iter().map(|v| v as &dyn ToSql).collect();
            app.append_row(refs.as_slice())?;
        }
        app.flush()?;
        drop(app);
        self.total.fetch_add(stored, Ordering::Relaxed);
        Ok(())
    }

    /// Append bare samples to the narrow `samples` table.
    pub fn append_samples(&self, samples: &[Sample]) -> Result<(), StoreError> {
        if samples.is_empty() {
            return Ok(());
        }
        let conn = recover(self.ingest.lock());
        let mut app = conn.appender(SAMPLES_TABLE)?;
        for s in samples {
            let vals: [DuckValue; 8] = [
                DuckValue::Text(s.source.as_str().to_string()),
                DuckValue::Text(s.series.to_string()),
                match &s.key {
                    Some(k) => DuckValue::Text(k.to_string()),
                    None => DuckValue::Null,
                },
                DuckValue::BigInt(s.index_ms()),
                DuckValue::UBigInt(s.seq),
                match s.value.as_f64() {
                    Some(f) => DuckValue::Double(f),
                    None => DuckValue::Null,
                },
                DuckValue::Text(s.value.to_string()),
                DuckValue::Text(value_kind(&s.value).to_string()),
            ];
            let refs: Vec<&dyn ToSql> = vals.iter().map(|v| v as &dyn ToSql).collect();
            app.append_row(refs.as_slice())?;
        }
        app.flush()?;
        drop(app);
        self.total.fetch_add(samples.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    // ---- the narrow surface ----------------------------------------------------------------

    fn list_on(&self, conn: &Connection) -> Vec<SeriesInfo> {
        let mut acc: BTreeMap<SeriesRef, SeriesInfo> = BTreeMap::new();
        // One series per (source, column), whatever the number of tables that hold the column:
        // the later value wins, the counts add up (within one declared type the union below
        // already counts exactly; across a type conflict the merge is the best the listing can
        // say, and the conflict is counted apart).
        fn merge(acc: &mut BTreeMap<SeriesRef, SeriesInfo>, info: SeriesInfo) {
            match acc.entry(info.series.clone()) {
                std::collections::btree_map::Entry::Vacant(v) => {
                    v.insert(info);
                }
                std::collections::btree_map::Entry::Occupied(mut o) => {
                    let e = o.get_mut();
                    e.count += info.count;
                    e.keys = e.keys.max(info.keys);
                    if info.last_index_ms >= e.last_index_ms {
                        e.last_index_ms = info.last_index_ms;
                        e.last_value = info.last_value;
                        e.ty = info.ty;
                    }
                }
            }
        }
        // The base tables, grouped by column name and declared type: a column two tables of one
        // source share is one series, read through one union.
        let mut groups: BTreeMap<(String, u8), (SeriesType, Vec<String>)> = BTreeMap::new();
        for base in &self.bases {
            for (col, ty) in &base.columns {
                groups.entry((col.clone(), *ty as u8)).or_insert_with(|| (*ty, Vec::new())).1.push(base.name.clone());
            }
        }
        for ((col, _), (ty, tables)) in groups {
            let qc = quote_ident(&col);
            let union = tables
                .iter()
                .map(|t| format!("SELECT source, key, index_ms, {qc} AS v FROM {}", quote_ident(t)))
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            // The last value typed, never cast by the engine: the suite's own display of it.
            let sql = format!(
                "SELECT source, count(v) AS cnt, \
                 count(DISTINCT key) FILTER (WHERE v IS NOT NULL) AS keys, \
                 max(CASE WHEN v IS NOT NULL THEN index_ms END) AS last_ms, \
                 arg_max(v, index_ms) AS last \
                 FROM ({union}) GROUP BY source HAVING count(v) > 0"
            );
            let Ok(mut stmt) = conn.prepare(&sql) else { continue };
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0) as usize,
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    r.get::<_, DuckValue>(4).unwrap_or(DuckValue::Null),
                ))
            });
            if let Ok(rows) = rows {
                for (source, count, keys, last_ms, last) in rows.flatten() {
                    merge(
                        &mut acc,
                        SeriesInfo {
                            series: SeriesRef::new(&source, &col),
                            ty,
                            count,
                            keys,
                            keys_capped: false,
                            last_index_ms: last_ms,
                            last_value: duck_to_value(&last, ty).map(|v| v.to_string()).unwrap_or_default(),
                        },
                    );
                }
            }
        }
        let sql = "SELECT source, series, \
                   arg_max(v_kind, index_ms) AS kind, count(*) AS cnt, \
                   count(DISTINCT key) AS keys, max(index_ms) AS last_ms, \
                   arg_max(v_disp, index_ms) AS last_disp \
                   FROM samples GROUP BY source, series";
        if let Ok(mut stmt) = conn.prepare(sql) {
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as u64,
                    r.get::<_, Option<i64>>(4)?.unwrap_or(0) as usize,
                    r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                ))
            });
            if let Ok(rows) = rows {
                for (source, series, kind, count, keys, last_ms, last_disp) in rows.flatten() {
                    merge(
                        &mut acc,
                        SeriesInfo {
                            series: SeriesRef::new(&source, &series),
                            ty: kind_series_type(&kind),
                            count,
                            keys,
                            keys_capped: false,
                            last_index_ms: last_ms,
                            last_value: last_disp,
                        },
                    );
                }
            }
        }
        acc.into_values().collect()
    }

    fn query_numeric_on(&self, conn: &Connection, series: &SeriesRef, key: Option<&str>, from_ms: i64, to_ms: i64) -> Vec<Point> {
        let mut out = Vec::new();
        let key_clause = if key.is_some() { " AND coalesce(key,'') = ?" } else { "" };
        if let Some(sources) = self.registry.get(series.series.as_ref()) {
            for s in sources {
                if s.ty == SeriesType::Text {
                    continue;
                }
                let expr = if s.ty == SeriesType::Time {
                    format!("epoch_ms({})", quote_ident(&s.column))
                } else {
                    format!("CAST({} AS DOUBLE)", quote_ident(&s.column))
                };
                let sql = format!(
                    "SELECT index_ms, CAST(seq AS BIGINT), {expr} FROM {tbl} \
                     WHERE source = ? AND {col} IS NOT NULL \
                     AND index_ms BETWEEN ? AND ?{key_clause} \
                     ORDER BY index_ms, seq",
                    tbl = quote_ident(&s.table),
                    col = quote_ident(&s.column),
                );
                let mut params: Vec<DuckValue> =
                    vec![DuckValue::Text(series.source.as_str().to_string()), DuckValue::BigInt(from_ms), DuckValue::BigInt(to_ms)];
                if let Some(k) = key {
                    params.push(DuckValue::Text(k.to_string()));
                }
                push_points(conn, &sql, params, &mut out);
            }
        }
        let sql = format!(
            "SELECT index_ms, CAST(seq AS BIGINT), v_num FROM samples \
             WHERE source = ? AND series = ? AND v_num IS NOT NULL \
             AND index_ms BETWEEN ? AND ?{key_clause} ORDER BY index_ms, seq"
        );
        let mut params: Vec<DuckValue> = vec![
            DuckValue::Text(series.source.as_str().to_string()),
            DuckValue::Text(series.series.to_string()),
            DuckValue::BigInt(from_ms),
            DuckValue::BigInt(to_ms),
        ];
        if let Some(k) = key {
            params.push(DuckValue::Text(k.to_string()));
        }
        push_points(conn, &sql, params, &mut out);
        out.sort_by_key(|p| (p.index_ms, p.seq));
        out
    }

    fn approx_bytes_on(&self, conn: &Connection) -> usize {
        scalar_i64(conn, "SELECT coalesce(sum(memory_usage_bytes), 0) FROM duckdb_memory()")
            .map(|b| b.max(0) as usize)
            .unwrap_or(0)
    }

    /// Every series, in (source, series) order.
    pub fn list(&self) -> Vec<SeriesInfo> {
        let conn = recover(self.api.lock());
        self.list_on(&conn)
    }

    /// The numeric view of `[from_ms, to_ms]` (inclusive), optionally filtered to one key,
    /// time-ordered. A text series yields nothing; a time series its epoch milliseconds.
    pub fn query_numeric(&self, series: &SeriesRef, key: Option<&str>, from_ms: i64, to_ms: i64) -> Vec<Point> {
        let conn = recover(self.api.lock());
        self.query_numeric_on(&conn, series, key, from_ms, to_ms)
    }

    /// Value cells stored so far (a hole counts for nothing).
    pub fn total_samples(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// The engine's own memory figure: a rough footprint.
    pub fn approx_bytes(&self) -> usize {
        let conn = recover(self.api.lock());
        self.approx_bytes_on(&conn)
    }

    /// Series names declared with different types by different rules: never silent.
    pub fn type_conflicts(&self) -> u64 {
        self.conflicts
    }

    // ---- the table surface -----------------------------------------------------------------

    /// Every table and view with its columns; a base table and `samples` carry an exact row
    /// count, a view `None`.
    pub fn list_tables(&self) -> Result<Vec<TableInfo>, StoreError> {
        let conn = recover(self.api.lock());
        self.list_tables_on(&conn)
    }

    fn list_tables_on(&self, conn: &Connection) -> Result<Vec<TableInfo>, StoreError> {
        let mut kinds: BTreeMap<String, TableKind> = BTreeMap::new();
        {
            let mut stmt = conn.prepare("SELECT table_name FROM duckdb_tables() WHERE NOT internal AND database_name = current_database()")?;
            for name in stmt.query_map([], |r| r.get::<_, String>(0))?.flatten() {
                let kind = if name == SAMPLES_TABLE { TableKind::Samples } else { TableKind::Base };
                kinds.insert(name, kind);
            }
        }
        {
            let mut stmt = conn.prepare("SELECT view_name FROM duckdb_views() WHERE NOT internal AND database_name = current_database()")?;
            for name in stmt.query_map([], |r| r.get::<_, String>(0))?.flatten() {
                kinds.entry(name).or_insert(TableKind::View);
            }
        }
        let mut out = Vec::new();
        for (name, kind) in kinds {
            // The columns as they read now: a view over a file the operator widened shows the
            // new column, which the catalog's own record of the view does not.
            let cols = match describe_types(conn, &quote_ident(&name)) {
                Ok(cols) => cols.into_iter().map(|(name, data_type)| ColumnInfo { name, data_type }).collect(),
                // A view that cannot be described now (a linked file gone between a removal and
                // its replacement) is listed with no columns; it hides no other table.
                Err(_) if kind == TableKind::View => Vec::new(),
                Err(e) => return Err(e),
            };
            let rows = if kind == TableKind::View {
                None
            } else {
                scalar_i64(conn, &format!("SELECT count(*) FROM {}", quote_ident(&name))).map(|n| n.max(0) as u64)
            };
            out.push(TableInfo { name, kind, columns: cols, rows });
        }
        Ok(out)
    }

    /// A windowed, ordered page of one table's rows as typed JSON cells, newest first by the
    /// time column (`index_ms`, else `ts`); a table with neither is unordered.
    pub fn query_rows(&self, table: &str, q: &RowsQuery) -> Result<RowsPage, StoreError> {
        let conn = recover(self.api.lock());
        let tables = self.list_tables_on(&conn)?;
        let info = tables.iter().find(|t| t.name == table).ok_or_else(|| StoreError::UnknownTable(table.to_string()))?;
        let columns = info.columns.clone();
        let has = |c: &str| columns.iter().any(|col| col.name == c);
        let time_col = if has("index_ms") {
            Some("index_ms".to_string())
        } else if has("ts") {
            Some("epoch_ms(\"ts\")".to_string())
        } else {
            None
        };

        let mut where_sql = String::new();
        let mut params: Vec<DuckValue> = Vec::new();
        if let Some(tc) = &time_col {
            if let Some(f) = q.from_ms {
                where_sql.push_str(if where_sql.is_empty() { " WHERE " } else { " AND " });
                where_sql.push_str(&format!("{tc} >= ?"));
                params.push(DuckValue::BigInt(f));
            }
            if let Some(t) = q.to_ms {
                where_sql.push_str(if where_sql.is_empty() { " WHERE " } else { " AND " });
                where_sql.push_str(&format!("{tc} <= ?"));
                params.push(DuckValue::BigInt(t));
            }
        }
        let order = match &time_col {
            // Every other column breaks a tie, positionally, so a page walk sees a row once (two
            // rows identical in every cell aside): the sequence alone is per source, and a view
            // may carry none.
            Some(tc) => {
                let rest: Vec<String> = (1..=columns.len()).map(|i| format!("{i} DESC")).collect();
                if rest.is_empty() { format!(" ORDER BY {tc} DESC") } else { format!(" ORDER BY {tc} DESC, {}", rest.join(", ")) }
            }
            None => String::new(),
        };
        let limit = q.limit.clamp(1, MAX_PAGE);
        let qt = quote_ident(table);

        let total = {
            let sql = format!("SELECT count(*) FROM {qt}{where_sql}");
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_row(duckdb::params_from_iter(params.iter()), |r| r.get::<_, i64>(0)).map(|n| n.max(0) as u64).unwrap_or(0)
        };

        let sql = format!("SELECT * FROM {qt}{where_sql}{order} LIMIT {limit} OFFSET {}", q.offset);
        let mut stmt = conn.prepare(&sql)?;
        let ncols = columns.len();
        let mut rows_out: Vec<Vec<serde_json::Value>> = Vec::new();
        let mut rows = stmt.query(duckdb::params_from_iter(params.iter()))?;
        while let Some(row) = rows.next()? {
            let mut cells = Vec::with_capacity(ncols);
            for i in 0..ncols {
                let v: DuckValue = row.get(i).unwrap_or(DuckValue::Null);
                cells.push(duck_to_json(&v));
            }
            rows_out.push(cells);
        }
        Ok(RowsPage { columns, rows: rows_out, total })
    }

    /// One numeric column of a table or view as a time-ordered series (a chart of a table
    /// column). `y` and `key` are checked against the catalog, so no text reaches the SQL
    /// unquoted. Empty when the table has no time column.
    pub fn query_table_series(&self, table: &str, y: &str, key: Option<&str>, from_ms: i64, to_ms: i64) -> Result<Vec<Point>, StoreError> {
        let conn = recover(self.api.lock());
        let info = self
            .list_tables_on(&conn)?
            .into_iter()
            .find(|t| t.name == table)
            .ok_or_else(|| StoreError::UnknownTable(table.to_string()))?;
        let has = |c: &str| info.columns.iter().any(|col| col.name == c);
        if !has(y) {
            return Err(StoreError::UnknownTable(format!("{table}.{y}")));
        }
        let time = if has("index_ms") {
            "index_ms".to_string()
        } else if has("ts") {
            "epoch_ms(\"ts\")".to_string()
        } else {
            return Ok(Vec::new());
        };
        // A key asked of a table with no key column is a name the catalog lacks, refused as one;
        // never a filter silently dropped.
        if key.is_some() && !has("key") {
            return Err(StoreError::UnknownTable(format!("{table}.key")));
        }
        let keyed = key.is_some();
        let key_clause = if keyed { " AND coalesce(key,'') = ?" } else { "" };
        let sql = format!(
            "SELECT {time} AS t, CAST({col} AS DOUBLE) AS v FROM {tbl} \
             WHERE {col} IS NOT NULL AND {time} BETWEEN ? AND ?{key_clause} ORDER BY t",
            col = quote_ident(y),
            tbl = quote_ident(table),
        );
        let mut params: Vec<DuckValue> = vec![DuckValue::BigInt(from_ms), DuckValue::BigInt(to_ms)];
        if keyed {
            params.push(DuckValue::Text(key.unwrap_or("").to_string()));
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(duckdb::params_from_iter(params.iter()), |r| {
            Ok(Point { index_ms: r.get::<_, i64>(0)?, seq: 0, value: r.get::<_, f64>(1)? })
        })?;
        Ok(rows.flatten().collect())
    }

    /// The column names and types a SELECT would yield, without running it: a definition's
    /// validation. `Err` carries the engine's binder message.
    pub fn validate_definition(&self, sql: &str) -> Result<Vec<ColumnInfo>, String> {
        let conn = recover(self.api.lock());
        let mut stmt = conn.prepare(&format!("DESCRIBE ({sql})")).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok(ColumnInfo { name: r.get::<_, String>(0)?, data_type: r.get::<_, String>(1)? }))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Export one table or view to a CSV file with a header row (a view materialised by the
    /// copy). The name is checked against the catalog.
    pub fn export_table_csv(&self, table: &str, path: &Path) -> Result<(), StoreError> {
        let conn = recover(self.api.lock());
        let known = self.list_tables_on(&conn)?;
        if !known.iter().any(|t| t.name == table) {
            return Err(StoreError::UnknownTable(table.to_string()));
        }
        conn.execute_batch(&format!("COPY {} TO '{}' (HEADER, FORMAT CSV);", quote_ident(table), sql_str(&path.display().to_string())))?;
        Ok(())
    }

    /// Export the whole database to a folder as CSV: a file per base table plus `schema.sql`
    /// and `load.sql` (the views live in the schema).
    pub fn export_all_csv(&self, dir: &Path) -> Result<(), StoreError> {
        std::fs::create_dir_all(dir).map_err(|e| StoreError::Open(format!("create {}: {e}", dir.display())))?;
        let conn = recover(self.api.lock());
        conn.execute_batch(&format!("EXPORT DATABASE '{}' (FORMAT CSV);", sql_str(&dir.display().to_string())))?;
        Ok(())
    }

    /// Snapshot the whole session database to a standalone `.duckdb` file, base tables as data
    /// and views as views, consistent even mid-session. The target must not exist.
    pub fn save_to(&self, path: &Path) -> Result<(), StoreError> {
        // Never over an existing file: refused by name before anything is attached (the engine
        // would attach a foreign database and copy the session into it).
        if path.exists() {
            return Err(StoreError::Open(format!("snapshot {}: a file already stands there; a snapshot never writes over one", path.display())));
        }
        let conn = recover(self.api.lock());
        let dbname: String = conn.query_row("SELECT current_database()", [], |r| r.get(0))?;
        conn.execute_batch(&format!("ATTACH '{}' AS _vsnap;", sql_str(&path.display().to_string())))?;
        // The copy may fail; the detach runs regardless, so a refused snapshot leaves nothing
        // attached and the next one writes. A half-written file is removed.
        let copied = conn.execute_batch(&format!("COPY FROM DATABASE {} TO _vsnap;", quote_ident(&dbname)));
        let detached = conn.execute_batch("DETACH _vsnap;");
        if copied.is_err() {
            let _ = std::fs::remove_file(path);
        }
        copied?;
        detached?;
        Ok(())
    }

    /// A shell's snapshot: the series list and the totals, off the `ui` connection.
    pub fn snapshot(&self) -> StoreSnapshot {
        let conn = recover(self.ui.lock());
        StoreSnapshot {
            series: self.list_on(&conn),
            total_samples: self.total.load(Ordering::Relaxed),
            approx_bytes: self.approx_bytes_on(&conn),
            type_conflicts: self.conflicts,
        }
    }
}

fn to_duck(v: &Value) -> DuckValue {
    match v {
        Value::Int(i) => DuckValue::BigInt(*i),
        Value::Float(f) => DuckValue::Double(*f),
        Value::Text(s) => DuckValue::Text(s.to_string()),
        Value::Time(t) => DuckValue::Timestamp(TimeUnit::Microsecond, t.and_utc().timestamp_micros()),
    }
}

fn value_kind(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Text(_) => "text",
        Value::Time(_) => "time",
    }
}

fn kind_series_type(kind: &str) -> SeriesType {
    match kind {
        "int" => SeriesType::Int,
        "float" => SeriesType::Float,
        "time" => SeriesType::Time,
        _ => SeriesType::Text,
    }
}

fn scalar_i64(conn: &Connection, sql: &str) -> Option<i64> {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).ok()
}

fn push_points(conn: &Connection, sql: &str, params: Vec<DuckValue>, out: &mut Vec<Point>) {
    let Ok(mut stmt) = conn.prepare(sql) else { return };
    let rows = stmt.query_map(duckdb::params_from_iter(params.iter()), |r| {
        Ok(Point { index_ms: r.get::<_, i64>(0)?, seq: r.get::<_, i64>(1)? as u64, value: r.get::<_, f64>(2)? })
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
}

/// How a linked CSV is read. Nothing set lets the engine sniff it, right for most files; the
/// two overrides exist for the two ways sniffing goes wrong in the field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CsvOptions {
    /// Force the field delimiter (`;`, a tab, `|`, `,`); none sniffs it.
    pub delim: Option<String>,
    /// Numbers are written with a decimal comma (`4,0`).
    pub decimal_comma: bool,
}

/// The delimiters tried when the sniffer collapses a file to one column.
const CSV_DELIMS: [&str; 4] = [";", "\t", "|", ","];

/// The scan expression of a linked CSV: the one place its read options live, shared by the
/// preview, the sniffer and a view's SQL, so what is previewed is what the session creates.
pub fn csv_scan_sql(path: &str, opts: &CsvOptions) -> String {
    let lit = path.trim().replace('\'', "''");
    let mut args: Vec<String> = Vec::new();
    if let Some(d) = &opts.delim {
        args.push(format!("delim='{}'", d.replace('\'', "''")));
        // A forced delimiter needs null padding: the files that need forcing are the ragged
        // ones, and without padding the engine errors or silently drops the offending row.
        args.push("null_padding=true".to_string());
    }
    if opts.decimal_comma {
        args.push("decimal_separator=','".to_string());
    }
    if args.is_empty() { format!("read_csv_auto('{lit}')") } else { format!("read_csv('{lit}', {})", args.join(", ")) }
}

/// Whether the sniffer gave up: one column whose name still holds a delimiter, the whole line
/// read as one field.
fn looks_unsplit(columns: &[(String, String)]) -> bool {
    columns.len() == 1 && CSV_DELIMS.iter().any(|d| columns[0].0.contains(*d))
}

/// The engine's type names that count as a number for the decimal-comma sniff.
fn is_numeric_type(t: &str) -> bool {
    let t = t.trim().to_ascii_uppercase();
    t.starts_with("DECIMAL")
        || matches!(
            t.as_str(),
            "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT" | "UINTEGER" | "UBIGINT" | "FLOAT" | "DOUBLE" | "REAL"
        )
}

/// `(name, type)` for every column a scan expression yields.
fn describe_types(conn: &Connection, scan: &str) -> Result<Vec<(String, String)>, StoreError> {
    let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM {scan}"))?;
    let mut out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        out.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
    }
    Ok(out)
}

/// How a CSV reads: a forced delimiter when the sniffer collapsed it to one column (the one
/// that splits it widest), and a decimal comma only when reading it so strictly improves the
/// typing (a column becomes numeric and none that was numeric regresses), decided by reading
/// the file both ways, never by guessing a locale. A false positive is impossible by
/// construction: forcing the comma on an ordinary `4.0` file would turn a working number
/// column into text.
pub fn sniff_csv_options(path: &str) -> Result<CsvOptions, StoreError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(StoreError::Open("the CSV path is empty".into()));
    }
    let conn = Connection::open_in_memory()?;
    let mut opts = CsvOptions::default();

    let auto = describe_types(&conn, &csv_scan_sql(path, &opts))?;
    if looks_unsplit(&auto) {
        let mut best: Option<(usize, String)> = None;
        for d in CSV_DELIMS {
            let probe = CsvOptions { delim: Some(d.to_string()), ..Default::default() };
            if let Ok(cols) = describe_types(&conn, &csv_scan_sql(path, &probe))
                && cols.len() > 1
                && best.as_ref().is_none_or(|(n, _)| cols.len() > *n)
            {
                best = Some((cols.len(), d.to_string()));
            }
        }
        if let Some((_, d)) = best {
            opts.delim = Some(d);
        }
    }

    // A ragged file of any shape takes the padded reader (docs/questions.md Q17, decided
    // 2026-11-09): where the sniffer kept, dropped or renamed rows on its own terms, the padded
    // reader keeps every row, pads a short one and widens the schema for a long one within its
    // window, refusing the read by its line past it. Raggedness is read off the file itself,
    // the first 2 048 lines counted in the sniffer's delimiter, quotes honoured.
    if opts.delim.is_none()
        && let Some(delim) = sniffed_delimiter(&conn, path)
        && csv_is_ragged(path, &delim)
    {
        opts.delim = Some(delim);
    }

    let plain = describe_types(&conn, &csv_scan_sql(path, &opts))?;
    let comma_opts = CsvOptions { decimal_comma: true, ..opts.clone() };
    let Ok(comma) = describe_types(&conn, &csv_scan_sql(path, &comma_opts)) else {
        return Ok(opts);
    };
    if plain.len() != comma.len() {
        return Ok(opts);
    }
    let mut gained = 0usize;
    for ((n_plain, t_plain), (n_comma, t_comma)) in plain.iter().zip(comma.iter()) {
        if n_plain != n_comma {
            return Ok(opts);
        }
        match (is_numeric_type(t_plain), is_numeric_type(t_comma)) {
            (true, false) => return Ok(opts),
            (false, true) => gained += 1,
            _ => {}
        }
    }
    opts.decimal_comma = gained > 0;
    Ok(opts)
}

/// The delimiter the engine's sniffer settles on for a file, or nothing when it cannot sniff it.
fn sniffed_delimiter(conn: &Connection, path: &str) -> Option<String> {
    let lit = path.trim().replace('\'', "''");
    let mut stmt = conn.prepare(&format!("SELECT Delimiter FROM sniff_csv('{lit}')")).ok()?;
    let d: String = stmt.query_row([], |r| r.get(0)).ok()?;
    (!d.is_empty()).then_some(d)
}

/// The lines the raggedness check reads, the engine's own window for widening the schema.
pub const CSV_RAGGED_WINDOW: usize = 2048;

/// Whether the file's rows, counted in `delim` over the first 2 048 lines with quotes honoured,
/// do not all hold the same number of fields (a blank last line is no row). A file that cannot
/// be read here is not ragged: its reading is refused elsewhere, by name.
pub fn csv_is_ragged(path: &str, delim: &str) -> bool {
    let Ok(text) = std::fs::read(path.trim()) else { return false };
    let text = String::from_utf8_lossy(&text);
    let mut counts: Vec<usize> = Vec::new();
    let mut fields = 1usize;
    let mut quoted = false;
    let mut at_line_start = true;
    let mut lines = 0usize;
    let d = delim.chars().next().unwrap_or(',');
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
            at_line_start = false;
        } else if !quoted && c == '\n' {
            if !at_line_start {
                counts.push(fields);
            }
            fields = 1;
            at_line_start = true;
            lines += 1;
            if lines >= CSV_RAGGED_WINDOW {
                break;
            }
        } else if !quoted && c == d {
            fields += 1;
            at_line_start = false;
        } else if c != '\r' {
            at_line_start = false;
        }
    }
    if !at_line_start && lines < CSV_RAGGED_WINDOW {
        counts.push(fields);
    }
    counts.windows(2).any(|w| w[0] != w[1])
}

/// Whether reading `path` with `opts` still yields one unsplit column.
pub fn csv_looks_unsplit(path: &str, opts: &CsvOptions) -> bool {
    let Ok(conn) = Connection::open_in_memory() else { return false };
    describe_types(&conn, &csv_scan_sql(path, opts)).map(|c| looks_unsplit(&c)).unwrap_or(false)
}

/// Preview a CSV with no session: a scratch connection reads the file through the scan, and
/// answers the sniffed columns and the first `limit` rows (at most 500). The scan re-reads the
/// file, so a later preview shows the operator's edits.
pub fn preview_csv(path: &str, limit: usize, opts: &CsvOptions) -> Result<RowsPage, StoreError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(StoreError::Open("the CSV path is empty".into()));
    }
    let scan = csv_scan_sql(path, opts);
    let conn = Connection::open_in_memory()?;
    let mut columns: Vec<ColumnInfo> = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM {scan}"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            columns.push(ColumnInfo { name: row.get::<_, String>(0)?, data_type: row.get::<_, String>(1)? });
        }
    }
    let total = {
        let mut stmt = conn.prepare(&format!("SELECT count(*) FROM {scan}"))?;
        stmt.query_row([], |r| r.get::<_, i64>(0)).map(|n| n.max(0) as u64).unwrap_or(0)
    };
    let limit = limit.clamp(1, 500);
    let mut stmt = conn.prepare(&format!("SELECT * FROM {scan} LIMIT {limit}"))?;
    let ncols = columns.len();
    let mut rows_out: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let mut cells = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let v: DuckValue = row.get(i).unwrap_or(DuckValue::Null);
            cells.push(duck_to_json(&v));
        }
        rows_out.push(cells);
    }
    Ok(RowsPage { columns, rows: rows_out, total })
}

fn duck_to_json(v: &DuckValue) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        DuckValue::Null => J::Null,
        DuckValue::Boolean(b) => J::Bool(*b),
        DuckValue::TinyInt(i) => J::from(*i),
        DuckValue::SmallInt(i) => J::from(*i),
        DuckValue::Int(i) => J::from(*i),
        DuckValue::BigInt(i) => J::from(*i),
        DuckValue::HugeInt(i) => J::from(*i as i64),
        DuckValue::UTinyInt(i) => J::from(*i),
        DuckValue::USmallInt(i) => J::from(*i),
        DuckValue::UInt(i) => J::from(*i),
        DuckValue::UBigInt(i) => J::from(*i),
        DuckValue::Float(f) => serde_json::Number::from_f64(*f as f64).map(J::Number).unwrap_or(J::Null),
        DuckValue::Double(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        DuckValue::Timestamp(unit, v) => J::from(unit_to_ms(*unit, *v)),
        DuckValue::Text(s) => J::String(s.clone()),
        DuckValue::Date32(d) => J::from((*d as i64).saturating_mul(86_400_000)),
        DuckValue::Time64(unit, v) => J::from(unit_to_ms(*unit, *v)),
        DuckValue::Decimal(d) => {
            d.to_string().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(J::Number).unwrap_or_else(|| J::String(d.to_string()))
        }
        DuckValue::Interval { months, days, nanos } => {
            J::String(format!("P{months}M{days}DT{}{}.{:09}S", if *nanos < 0 { "-" } else { "" }, nanos.abs() / 1_000_000_000, nanos.abs() % 1_000_000_000))
        }
        DuckValue::Blob(b) => J::String(b.iter().map(|x| format!("{x:02X}")).collect()),
        DuckValue::Enum(s) => J::String(s.clone()),
        // A nested value (a list, a struct, a map, a union) as the engine's debug text: no JSON
        // scalar fits it, and the suite's tables never make one.
        other => J::String(format!("{other:?}")),
    }
}

/// A typed cell as the suite's own value, by the series' declared type: what a last value
/// displays as is the suite's display, never the engine's print.
fn duck_to_value(v: &DuckValue, ty: SeriesType) -> Option<Value> {
    match (ty, v) {
        (_, DuckValue::Null) => None,
        (SeriesType::Int, DuckValue::BigInt(i)) => Some(Value::Int(*i)),
        (SeriesType::Int, DuckValue::Int(i)) => Some(Value::Int(*i as i64)),
        (SeriesType::Int, DuckValue::HugeInt(i)) => Some(Value::Int(*i as i64)),
        (SeriesType::Int, DuckValue::Double(f)) => Some(Value::Int(*f as i64)),
        (SeriesType::Float, DuckValue::Double(f)) => Some(Value::Float(*f)),
        (SeriesType::Float, DuckValue::Float(f)) => Some(Value::Float(*f as f64)),
        (SeriesType::Float, DuckValue::BigInt(i)) => Some(Value::Float(*i as f64)),
        (SeriesType::Text, DuckValue::Text(s)) => Some(Value::Text(std::sync::Arc::from(s.as_str()))),
        (SeriesType::Time, DuckValue::Timestamp(unit, t)) => {
            chrono::DateTime::from_timestamp_micros(unit_to_us(*unit, *t)).map(|d| Value::Time(d.naive_utc()))
        }
        (_, other) => Some(Value::Text(std::sync::Arc::from(duck_to_json(other).to_string().trim_matches('"')))),
    }
}

fn unit_to_ms(unit: TimeUnit, v: i64) -> i64 {
    match unit {
        TimeUnit::Second => v.saturating_mul(1_000),
        TimeUnit::Millisecond => v,
        TimeUnit::Microsecond => v / 1_000,
        TimeUnit::Nanosecond => v / 1_000_000,
    }
}

fn unit_to_us(unit: TimeUnit, v: i64) -> i64 {
    match unit {
        TimeUnit::Second => v.saturating_mul(1_000_000),
        TimeUnit::Millisecond => v.saturating_mul(1_000),
        TimeUnit::Microsecond => v,
        TimeUnit::Nanosecond => v / 1_000,
    }
}
