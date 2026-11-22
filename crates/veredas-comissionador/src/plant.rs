//! The engine's one door to portal (`docs/comissionador-plan.md`, "How a run meets the plant").
//!
//! A run asks the plant four things: the catalog of tables, the rows of a table in a window of the
//! store's axis, a write of one OPC UA tag, a send of one catalog message; and it reads the write
//! switch's position, never setting it. [`LivePlant`] answers them over a portal session held in
//! this process (`docs/questions.md` Q313, taken: in process first); a reach over portal's HTTP
//! interface would be a second implementation of [`Plant`].

use std::collections::HashMap;
use std::sync::Arc;

use veredas_portal::SeriesType;
use veredas_portal::session::Live;
use veredas_portal::store::{RowsQuery, TableKind};

use crate::eval::{Row, Value};

/// The page a row read takes at a time: the store's own cap.
const PAGE: usize = veredas_portal::store::MAX_PAGE;

/// A column of a table, with the type its cells read as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: CellType,
}

/// What a cell reads as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellType {
    Int,
    Float,
    Bool,
    Text,
    Time,
}

/// A table a test may read: one with portal's time column.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub name: String,
    /// The value columns, the fixed ones (`source`, `key`, `index_ms`, `ts`, `arrival_ms`, `seq`)
    /// left out.
    pub columns: Vec<Column>,
}

/// Why the plant did not answer, or refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlantError {
    /// The store did not answer a read.
    #[error("the store: {0}")]
    Store(String),
    /// A write or a send refused or failed, in portal's words.
    #[error("{0}")]
    Refused(String),
    /// A write that may or may not have reached the plant.
    #[error("{0}")]
    Unconfirmed(String),
}

pub trait Plant {
    /// The portal session's name.
    fn name(&self) -> String;
    /// The store's axis now: the plant's local clock read as UTC.
    fn now_ms(&self) -> i64;
    /// Every table a test may read, by name.
    fn tables(&self) -> Result<Vec<Table>, PlantError>;
    /// The rows of `table` stamped in `(after, upto]`, oldest first.
    fn rows(&self, table: &Table, after: i64, upto: i64) -> Result<Vec<Row>, PlantError>;
    /// The latest value of `column` stamped at or before `at` (a hole read past), with its stamp.
    fn latest(&self, table: &Table, column: &str, at: i64) -> Result<Option<(i64, Value)>, PlantError>;
    /// Whether portal's write switch is armed. Read, never set.
    fn armed(&self) -> bool;
    /// Whether `table.column` is an OPC UA tag the session marks writable.
    fn writable(&self, table: &str, column: &str) -> bool;
    /// Whether `channel` is a channel of the session with `message` in its catalog.
    fn sendable(&self, channel: &str, message: &str) -> bool;
    /// Write one tag: the plant's answer, as text, on success.
    fn write(&self, table: &str, column: &str, value: &Value) -> Result<String, PlantError>;
    /// Send one catalog message.
    fn send(&self, channel: &str, message: &str, fields: Vec<(String, String)>) -> Result<(), PlantError>;
}

/// A portal session in this process.
pub struct LivePlant {
    live: Arc<Live>,
    runtime: tokio::runtime::Runtime,
}

impl LivePlant {
    pub fn new(live: Arc<Live>) -> Result<LivePlant, PlantError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PlantError::Store(format!("no runtime for the writes: {e}")))?;
        Ok(LivePlant { live, runtime })
    }

    /// The OPC UA source and the tag's node id text of a writable column.
    fn tag(&self, table: &str, column: &str) -> Option<(String, String, SeriesType)> {
        for s in &self.live.sources {
            let Some(spec) = self.live.opc_spec(&s.id) else { continue };
            for rule in &spec.rules {
                if rule.name != table {
                    continue;
                }
                if let Some(t) = rule.tags.iter().find(|t| t.series == column && t.writable) {
                    return Some((s.id.clone(), t.raw.clone(), t.ty));
                }
            }
        }
        None
    }
}

/// The fixed columns of every base table.
const FIXED: [&str; 6] = ["source", "key", "index_ms", "ts", "arrival_ms", "seq"];

fn cell_type(data_type: &str) -> Option<CellType> {
    let t = data_type.to_ascii_uppercase();
    Some(if t == "BOOLEAN" {
        CellType::Bool
    } else if t.starts_with("TIMESTAMP") || t == "DATE" {
        CellType::Time
    } else if ["TINYINT", "SMALLINT", "INTEGER", "BIGINT", "HUGEINT", "UTINYINT", "USMALLINT", "UINTEGER", "UBIGINT"].contains(&t.as_str()) {
        CellType::Int
    } else if t == "DOUBLE" || t == "FLOAT" || t.starts_with("DECIMAL") {
        CellType::Float
    } else if t == "VARCHAR" {
        CellType::Text
    } else {
        return None;
    })
}

fn cell(v: &serde_json::Value, ty: CellType) -> Option<Value> {
    match (ty, v) {
        (_, serde_json::Value::Null) => None,
        (CellType::Int, serde_json::Value::Number(n)) => n.as_i64().map(Value::Int),
        (CellType::Float, serde_json::Value::Number(n)) => n.as_f64().map(Value::Float),
        (CellType::Bool, serde_json::Value::Bool(b)) => Some(Value::Bool(*b)),
        (CellType::Time, serde_json::Value::Number(n)) => n.as_i64().map(Value::Time),
        (CellType::Text, serde_json::Value::String(s)) => Some(Value::Text(Arc::from(s.as_str()))),
        (_, other) => Some(Value::Text(Arc::from(other.to_string()))),
    }
}

impl LivePlant {
    /// A page of `table` newest first, as rows.
    fn page(&self, table: &Table, q: &RowsQuery) -> Result<(Vec<Row>, u64), PlantError> {
        let page = self.live.store.query_rows(&table.name, q).map_err(|e| PlantError::Store(e.to_string()))?;
        let at = |name: &str| page.columns.iter().position(|c| c.name == name);
        let (i_index, i_arr, i_src, i_key, i_seq) = (at("index_ms"), at("arrival_ms"), at("source"), at("key"), at("seq"));
        let mut out = Vec::with_capacity(page.rows.len());
        for r in &page.rows {
            let get = |i: Option<usize>| i.and_then(|i| r.get(i));
            let num = |i: Option<usize>| get(i).and_then(|v| v.as_i64()).unwrap_or(0);
            let text = |i: Option<usize>| get(i).and_then(|v| v.as_str()).map(str::to_string);
            let mut columns = Vec::with_capacity(table.columns.len());
            for c in &table.columns {
                let v = at(&c.name).and_then(|i| r.get(i)).and_then(|v| cell(v, c.ty));
                columns.push((c.name.clone(), v));
            }
            out.push(Row {
                table: table.name.clone(),
                time_ms: num(i_index),
                arrival_ms: num(i_arr),
                source: text(i_src).unwrap_or_default(),
                key: text(i_key),
                seq: num(i_seq),
                columns,
            });
        }
        Ok((out, page.total))
    }
}

impl Plant for LivePlant {
    fn name(&self) -> String {
        self.live.name.clone()
    }

    fn now_ms(&self) -> i64 {
        veredas_portal::serve::store_now_ms()
    }

    fn tables(&self) -> Result<Vec<Table>, PlantError> {
        let infos = self.live.store.list_tables().map_err(|e| PlantError::Store(e.to_string()))?;
        Ok(infos
            .into_iter()
            .filter(|t| t.kind != TableKind::Samples && t.columns.iter().any(|c| c.name == "index_ms"))
            .map(|t| Table {
                name: t.name,
                columns: t
                    .columns
                    .iter()
                    .filter(|c| !FIXED.contains(&c.name.as_str()))
                    .filter_map(|c| cell_type(&c.data_type).map(|ty| Column { name: c.name.clone(), ty }))
                    .collect(),
            })
            .collect())
    }

    fn rows(&self, table: &Table, after: i64, upto: i64) -> Result<Vec<Row>, PlantError> {
        if upto <= after {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut offset = 0;
        loop {
            let q = RowsQuery { from_ms: Some(after + 1), to_ms: Some(upto), limit: PAGE, offset };
            let (rows, total) = self.page(table, &q)?;
            let n = rows.len();
            out.extend(rows);
            offset += n;
            if n == 0 || offset as u64 >= total {
                break;
            }
        }
        out.reverse();
        Ok(out)
    }

    fn latest(&self, table: &Table, column: &str, at: i64) -> Result<Option<(i64, Value)>, PlantError> {
        let mut offset = 0;
        loop {
            let q = RowsQuery { from_ms: None, to_ms: Some(at), limit: 64, offset };
            let (rows, total) = self.page(table, &q)?;
            for r in &rows {
                if let Some(Some(v)) = r.field(column) {
                    return Ok(Some((r.time_ms, v)));
                }
            }
            offset += rows.len();
            if rows.is_empty() || offset as u64 >= total {
                return Ok(None);
            }
        }
    }

    fn armed(&self) -> bool {
        self.live.writes_armed()
    }

    fn writable(&self, table: &str, column: &str) -> bool {
        self.tag(table, column).is_some()
    }

    fn sendable(&self, channel: &str, message: &str) -> bool {
        self.live.channel_handle(channel).is_some_and(|h| h.tables().iter().any(|t| t.name == message))
    }

    fn write(&self, table: &str, column: &str, value: &Value) -> Result<String, PlantError> {
        let Some((source, raw, ty)) = self.tag(table, column) else {
            return Err(PlantError::Refused(format!("`{table}.{column}` is not a writable tag in this session")));
        };
        let text = match (value, ty) {
            (Value::Bool(b), _) => (if *b { "1" } else { "0" }).to_string(),
            (Value::Float(x), SeriesType::Int) if x.fract() == 0.0 => format!("{}", *x as i64),
            (v, _) => v.to_string(),
        };
        let variant = veredas_portal::serve::coerce(&text, ty).map_err(PlantError::Refused)?;
        let handle = self.live.opc_handle(&source).ok_or_else(|| PlantError::Refused(format!("`{source}` is no OPC source of this session")))?;
        let answer = self.runtime.block_on(handle.write_tags(vec![(raw, variant)]));
        match answer {
            Ok(statuses) if statuses.iter().all(|s| s.is_good()) => Ok(statuses.first().map(|s| s.to_string()).unwrap_or_default()),
            Ok(statuses) => Err(PlantError::Refused(format!(
                "the plant answered {}",
                statuses.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ")
            ))),
            // Portal's own reading of a refusal: 504 is a write that may have reached the plant.
            Err(e) if veredas_portal::serve::refused_code(&e) == 504 => Err(PlantError::Unconfirmed(e.to_string())),
            Err(e) => Err(PlantError::Refused(e.to_string())),
        }
    }

    fn send(&self, channel: &str, message: &str, fields: Vec<(String, String)>) -> Result<(), PlantError> {
        let handle = self.live.channel_handle(channel).ok_or_else(|| PlantError::Refused(format!("`{channel}` is no channel of this session")))?;
        let fields: HashMap<String, String> = fields.into_iter().collect();
        handle.send(None, message, fields).map_err(|e| PlantError::Refused(e.to_string()))
    }
}
