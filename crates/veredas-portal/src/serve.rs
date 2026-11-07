//! The server (feature `serve`): the live stream and windowed REST over a running session, and
//! the write surface, gated.
//!
//! Every query is a pure function of its parameters over the session's store, so the server
//! holds no state per client and N clients with N windows cost nothing. The listing and the
//! vitals come from the caches the ingest keeps ([`Live`]), never a store scan: at a plant's
//! scale a scan runs for minutes holding the store's connection, and every endpoint that
//! touches the store convoys behind it.
//!
//! The write surface is a whitelist, not a passthrough: a request names a source and a tag or a
//! message, and only what a rule marks writable resolves. The gates are the handles' own
//! ([`OpcHandle::write_tags`], [`ChannelHandle::send`]), below every caller; this layer checks
//! early for a better answer and maps a refusal onto a status code. There is no endpoint that
//! arms the switch: the shell or the project does that, in process.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value as Json_, json};
use tokio::net::TcpListener;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use veredas_core::opc::{StatusCode as OpcStatus, Variant, WriteError};
use veredas_core::{SeriesRef, SeriesType, now_unix_ms};

use crate::channel::SendError;
use crate::session::{HealthState, Live};
use crate::store::{RowsQuery, StoreError};

pub mod lttb;

/// The point budget bounds of a windowed query: fewer than ten points draw nothing, more than
/// five thousand draw nothing a screen can show.
pub const POINTS_MIN: usize = 10;
pub const POINTS_MAX: usize = 5_000;
pub const POINTS_DEFAULT: usize = 1_000;
/// Rows per page when the client names no limit.
pub const ROWS_DEFAULT: usize = 200;

type ApiError = (StatusCode, Json<Json_>);

fn err(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

fn store_err(e: StoreError) -> ApiError {
    match e {
        StoreError::UnknownTable(t) => err(StatusCode::NOT_FOUND, format!("unknown table `{t}`")),
        other => err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn query_failed() -> ApiError {
    err(StatusCode::INTERNAL_SERVER_ERROR, "the store query did not complete")
}

/// Everything the server exposes, over one session. A constructor, so a test and a shell serve
/// the same router.
pub fn router(live: Arc<Live>) -> Router {
    Router::new()
        .route("/events", get(sse_events))
        .route("/api/session", get(session))
        .route("/api/series", get(series))
        .route("/api/series/data", get(series_data))
        .route("/api/tables", get(tables))
        .route("/api/table/data", get(table_data))
        .route("/api/opc/read", get(opc_read))
        .route("/api/opc/write", post(opc_write))
        .route("/api/channels/send", post(channel_send))
        .with_state(live)
}

/// Serve on a bound listener until the runtime stops.
pub async fn serve(listener: TcpListener, live: Arc<Live>) -> std::io::Result<()> {
    axum::serve(listener, router(live)).await
}

/// The store's time axis "now": the plant's local clock read as if UTC, the axis a stamped line
/// lands on. A client computes "the last N minutes" against this, never its own clock.
pub fn store_now_ms() -> i64 {
    chrono::Local::now().naive_local().and_utc().timestamp_millis()
}

fn health_label(state: &HealthState) -> (&'static str, Option<String>) {
    match state {
        HealthState::Waiting => ("waiting", None),
        HealthState::Live => ("live", None),
        HealthState::Idle { silence_ms } => ("idle", Some(format!("silent for {} s", silence_ms / 1000))),
        HealthState::Retrying(why) => ("retrying", Some(why.clone())),
        HealthState::Ended => ("ended", None),
        HealthState::Failed(why) => ("failed", Some(why.clone())),
    }
}

/// The `/api/session` body, and the stream's `hello`: what runs, its sources and their health,
/// the store's vitals and the ingest's counters, the two clocks' now, the switch's position.
/// Nothing here queries the store: the session endpoint answers while the store grinds a heavy
/// window.
pub fn snapshot(live: &Live) -> Json_ {
    let mut body = json!({
        "running": true,
        "session_name": live.name,
        "started_ms": live.started_ms,
        "start_ms": live.start_ms,
        "now_ms": store_now_ms(),
        "arrival_now_ms": now_unix_ms(),
        "writes_armed": live.writes_armed(),
        "sources": live.sources.iter().map(|s| json!({
            "id": s.id,
            "display_name": s.display_name,
            "kind": s.kind,
            "has_timestamp": s.has_timestamp,
        })).collect::<Vec<_>>(),
    });
    if let Ok(h) = live.health.read() {
        let mut health = serde_json::Map::new();
        for (id, s) in h.iter() {
            let (label, detail) = health_label(&s.state);
            health.insert(
                id.clone(),
                json!({
                    "state": label,
                    "detail": detail,
                    "count": s.count,
                    "last_arrival_ms": s.last_arrival_ms,
                    "last_ts": s.last_ts.map(|t| t.and_utc().timestamp_millis()),
                }),
            );
        }
        body["health"] = Json_::Object(health);
    }
    if let Ok(v) = live.vitals.read() {
        body["samples"] = json!(v.samples);
        body["store_bytes"] = json!(v.bytes);
        body["type_conflicts"] = json!(v.type_conflicts);
    }
    if let Ok(c) = live.counters.read() {
        body["counters"] = json!({
            "lines": c.lines,
            "polls": c.polls,
            "messages": c.messages,
            "rows": c.rows,
            "failures": c.failures,
            "store_errors": c.store_errors,
            "last_error": c.last_error,
        });
    }
    body
}

/// The live stream: a `hello` with the session snapshot on connect, then every data frame the
/// ingest broadcasts; a client that lags gets a `lag` event where its frames were dropped, its
/// cue to re-window over REST. Keep-alive comments hold an idle connection open.
async fn sse_events(State(live): State<Arc<Live>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = live.subscribe();
    let hello = snapshot(&live).to_string();
    let first = tokio_stream::once(Ok(Event::default().event("hello").data(hello)));
    let data = BroadcastStream::new(rx).map(|msg| {
        Ok(match msg {
            Ok(frame) => Event::default().event("data").data(frame.as_ref()),
            Err(_lagged) => Event::default().event("lag").data("frames dropped"),
        })
    });
    Sse::new(first.chain(data)).keep_alive(KeepAlive::default())
}

async fn session(State(live): State<Arc<Live>>) -> Json<Json_> {
    Json(snapshot(&live))
}

fn type_name(ty: SeriesType) -> &'static str {
    match ty {
        SeriesType::Int => "int",
        SeriesType::Float => "float",
        SeriesType::Text => "text",
        SeriesType::Time => "time",
    }
}

/// Every series the session carries, from the ingest's cache, sorted by source then series.
async fn series(State(live): State<Arc<Live>>) -> Result<Json<Json_>, ApiError> {
    let cache = live.series.read().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "the series cache is poisoned"))?;
    let mut out: Vec<Json_> = cache
        .values()
        .map(|s| {
            json!({
                "source": s.series.source.as_str(),
                "series": s.series.series.as_ref(),
                "type": type_name(s.ty),
                "count": s.count,
                "keys": s.keys,
                "keys_capped": s.keys_capped,
                "last_ms": s.last_index_ms,
                "last_value": s.last_value,
            })
        })
        .collect();
    out.sort_by(|a, b| (a["source"].as_str(), a["series"].as_str()).cmp(&(b["source"].as_str(), b["series"].as_str())));
    Ok(Json(json!({ "series": out })))
}

#[derive(Debug, Deserialize)]
pub struct SeriesDataParams {
    pub source: String,
    pub series: String,
    pub key: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub points: Option<usize>,
}

fn budget(points: Option<usize>) -> usize {
    points.unwrap_or(POINTS_DEFAULT).clamp(POINTS_MIN, POINTS_MAX)
}

fn window_body(raw: Vec<(i64, f64)>, points: Option<usize>) -> Json_ {
    let total = raw.len();
    let shown = lttb::downsample(&raw, budget(points));
    json!({
        "points": shown.iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>(),
        "total": total,
        "downsampled": shown.len() < total,
    })
}

/// A numeric window of one series, downsampled server-side to the client's point budget so
/// every client sees the same picture for the same window.
async fn series_data(State(live): State<Arc<Live>>, Query(p): Query<SeriesDataParams>) -> Result<Json<Json_>, ApiError> {
    let store = Arc::clone(&live.store);
    let (from, to) = (p.from.unwrap_or(i64::MIN), p.to.unwrap_or(i64::MAX));
    let (source, series, key) = (p.source, p.series, p.key);
    let raw = tokio::task::spawn_blocking(move || {
        store
            .query_numeric(&SeriesRef::new(&source, &series), key.as_deref(), from, to)
            .into_iter()
            .map(|pt| (pt.index_ms, pt.value))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| query_failed())?;
    Ok(Json(window_body(raw, p.points)))
}

/// The store's catalog: every base table, the samples table and every view, with columns and
/// a row count where one is exact.
async fn tables(State(live): State<Arc<Live>>) -> Result<Json<Json_>, ApiError> {
    let store = Arc::clone(&live.store);
    let infos = tokio::task::spawn_blocking(move || store.list_tables()).await.map_err(|_| query_failed())?.map_err(store_err)?;
    let out: Vec<Json_> = infos
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "kind": t.kind.as_str(),
                "columns": t.columns.iter().map(|c| json!({ "name": c.name, "type": c.data_type })).collect::<Vec<_>>(),
                "rows": t.rows,
            })
        })
        .collect();
    Ok(Json(json!({ "tables": out })))
}

#[derive(Debug, Deserialize)]
pub struct TableDataParams {
    pub name: String,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    /// A numeric column charted against the time column, shaped like a series window.
    pub y: Option<String>,
    pub key: Option<String>,
    pub points: Option<usize>,
}

/// Typed rows of a table, windowed and paged, or with `y` one column charted.
async fn table_data(State(live): State<Arc<Live>>, Query(p): Query<TableDataParams>) -> Result<Json<Json_>, ApiError> {
    let store = Arc::clone(&live.store);
    if let Some(y) = p.y {
        let (from, to) = (p.from.unwrap_or(i64::MIN), p.to.unwrap_or(i64::MAX));
        let (name, key) = (p.name, p.key);
        let raw = tokio::task::spawn_blocking(move || store.query_table_series(&name, &y, key.as_deref(), from, to))
            .await
            .map_err(|_| query_failed())?
            .map_err(store_err)?;
        return Ok(Json(window_body(raw.into_iter().map(|pt| (pt.index_ms, pt.value)).collect(), p.points)));
    }
    let q = RowsQuery { from_ms: p.from, to_ms: p.to, limit: p.limit.unwrap_or(ROWS_DEFAULT), offset: p.offset.unwrap_or(0) };
    let offset = q.offset;
    let name = p.name;
    let page = tokio::task::spawn_blocking(move || store.query_rows(&name, &q)).await.map_err(|_| query_failed())?.map_err(store_err)?;
    let truncated = page.total as usize > page.rows.len() + offset;
    Ok(Json(json!({
        "schema": page.columns.iter().map(|c| json!({ "name": c.name, "type": c.data_type })).collect::<Vec<_>>(),
        "rows": page.rows,
        "total": page.total,
        "truncated": truncated,
    })))
}

// ---- the write surface -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TagRef {
    pub source: String,
    pub tag: String,
}

/// The text of a read value, as a plant operator reads it.
fn variant_text(v: &Variant) -> String {
    match v {
        Variant::Boolean(b) => b.to_string(),
        Variant::SByte(x) => x.to_string(),
        Variant::Byte(x) => x.to_string(),
        Variant::Int16(x) => x.to_string(),
        Variant::UInt16(x) => x.to_string(),
        Variant::Int32(x) => x.to_string(),
        Variant::UInt32(x) => x.to_string(),
        Variant::Int64(x) => x.to_string(),
        Variant::UInt64(x) => x.to_string(),
        Variant::Float(x) => x.to_string(),
        Variant::Double(x) => x.to_string(),
        Variant::String(s) => s.value().clone().unwrap_or_default(),
        other => format!("{other:?}"),
    }
}

/// One polled tag read on demand. Reading needs no switch: it shows nothing a chart does not.
async fn opc_read(State(live): State<Arc<Live>>, Query(q): Query<TagRef>) -> Result<Json<Json_>, ApiError> {
    let handle = live.opc_handle(&q.source).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("`{}` is no OPC source of this session", q.source)))?;
    let node = handle
        .spec()
        .tag(&q.tag)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("`{}` / `{}` is not a tag in this session", q.source, q.tag)))?
        .node_id
        .clone();
    let reads = handle.read(vec![node]).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let Some(read) = reads.first() else {
        return Err(err(StatusCode::BAD_GATEWAY, "the server returned no value"));
    };
    Ok(Json(json!({
        "source": q.source,
        "tag": q.tag,
        "ok": read.is_good(),
        "value": read.value.as_ref().map(variant_text),
        "status": read.status.to_string(),
    })))
}

/// Coerce the operator's text to the tag's type. Refused rather than guessed: writing 0 because
/// `on` did not parse would be worse than an error.
pub fn coerce(value: &str, ty: SeriesType) -> Result<Variant, String> {
    let v = value.trim();
    match ty {
        SeriesType::Int => match v.to_ascii_lowercase().as_str() {
            "true" | "on" => Ok(Variant::Int32(1)),
            "false" | "off" => Ok(Variant::Int32(0)),
            _ => v.parse::<i32>().map(Variant::Int32).map_err(|_| format!("`{v}` is not a whole number")),
        },
        SeriesType::Float => v.parse::<f64>().map(Variant::Double).map_err(|_| format!("`{v}` is not a number")),
        SeriesType::Text => Ok(Variant::from(v)),
        SeriesType::Time => Err("writing a time-typed tag is not supported".to_string()),
    }
}

/// A refusal on a status code. The switch off and a tag not writable are 403: the capability
/// exists, it is switched off or not granted. A write the plant never saw is 503; one that may
/// have reached it is 504, and the text says so.
fn refused(e: WriteError) -> ApiError {
    let code = match e {
        WriteError::Disarmed | WriteError::NotWritable { .. } => StatusCode::FORBIDDEN,
        WriteError::NotSent(_) => StatusCode::SERVICE_UNAVAILABLE,
        WriteError::Unconfirmed(_) => StatusCode::GATEWAY_TIMEOUT,
    };
    err(code, e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct TagWrite {
    pub source: String,
    pub tag: String,
    pub value: String,
}

/// Write one writable tag. The switch is checked first, so a disarmed session answers before
/// anything is coerced; then the tag against the whitelist; the handle checks both again.
async fn opc_write(State(live): State<Arc<Live>>, Json(body): Json<TagWrite>) -> Result<Json<Json_>, ApiError> {
    if !live.writes_armed() {
        return Err(refused(WriteError::Disarmed));
    }
    let handle = live.opc_handle(&body.source).ok_or_else(|| {
        err(StatusCode::FORBIDDEN, format!("`{}` / `{}` is not a writable tag in this session", body.source, body.tag))
    })?;
    let ty = handle
        .writable_type(&body.tag)
        .ok_or_else(|| refused(WriteError::NotWritable { source_id: body.source.clone(), tag: body.tag.clone() }))?;
    let variant = coerce(&body.value, ty).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let statuses = handle.write_tags(vec![(body.tag.clone(), variant)]).await.map_err(refused)?;
    let ok = statuses.iter().all(OpcStatus::is_good);
    Ok(Json(json!({ "ok": ok, "status": statuses.first().map(|s| s.to_string()) })))
}

#[derive(Debug, Deserialize)]
pub struct MessageSend {
    pub source: String,
    /// The peer to send to; absent, the channel's one peer (refused when there are several).
    pub to: Option<u64>,
    pub message: String,
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
}

fn send_refused(e: SendError) -> ApiError {
    let code = match e {
        SendError::Disarmed => StatusCode::FORBIDDEN,
        SendError::UnknownMessage(_) | SendError::Compose { .. } | SendError::EmptyBody(_) | SendError::Framing { .. } => StatusCode::BAD_REQUEST,
        SendError::NoPeer(_) | SendError::NoPeers => StatusCode::CONFLICT,
        SendError::WorkerGone => StatusCode::SERVICE_UNAVAILABLE,
    };
    err(code, e.to_string())
}

/// Compose and send one catalog message on a channel. The same switch gates it: disarmed, the
/// request is refused before the catalog is consulted.
async fn channel_send(State(live): State<Arc<Live>>, Json(body): Json<MessageSend>) -> Result<Json<Json_>, ApiError> {
    if !live.writes_armed() {
        return Err(send_refused(SendError::Disarmed));
    }
    let handle = live
        .channel_handle(&body.source)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("`{}` is no channel of this session", body.source)))?
        .clone();
    let (to, message, fields) = (body.to, body.message, body.fields);
    tokio::task::spawn_blocking(move || handle.send(to, &message, fields))
        .await
        .map_err(|_| query_failed())?
        .map_err(send_refused)?;
    Ok(Json(json!({ "ok": true })))
}

impl IntoResponse for crate::session::SessionError {
    fn into_response(self) -> axum::response::Response {
        err(StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}
