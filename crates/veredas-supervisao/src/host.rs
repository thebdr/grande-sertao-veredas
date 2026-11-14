//! The host: the sessions of a supervisão file started through portal's library and served side
//! by side on one address, each under `/s/<name>/` with its own portal router nested unchanged at
//! `/s/<name>/portal/` (Q19). Supervisão holds no write path: the page posts to portal's routes.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tokio_stream::{Stream, StreamExt};
use veredas_core::{SourceError, SourceId, now_unix_ms};
use veredas_portal::session::{Live, Session};

use crate::binding::Binding;
use crate::config::HostConfig;
use crate::feed::Feed;
use crate::{panel, web};

#[derive(Debug, Error)]
pub enum HostError {
    #[error("the host's runtime did not start: {0}")]
    Runtime(std::io::Error),
    #[error("cannot bind {addr}: {source}")]
    Bind { addr: SocketAddr, source: std::io::Error },
    #[error("session `{name}` did not start: {reason}")]
    Session { name: String, reason: String },
}

/// One session as the host serves it.
pub struct SessionApp {
    pub name: String,
    pub live: Arc<Live>,
    pub binding: Arc<Binding>,
    pub feed: Feed,
}

/// A running host.
pub struct Host {
    addr: SocketAddr,
    runtime: Option<tokio::runtime::Runtime>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
    sessions: Vec<(String, Session)>,
    /// Each session's feed snapshot, by name, as the routes read it.
    snapshots: Vec<(String, Arc<std::sync::RwLock<crate::feed::Snapshot>>)>,
}

impl Host {
    /// Bind the address, start every session in the file's order, their feeds, and serve. A
    /// session that does not start stops the ones started before it, and nothing is served.
    pub fn start(cfg: HostConfig) -> Result<Host, HostError> {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(HostError::Runtime)?;
        let listener = runtime.block_on(tokio::net::TcpListener::bind(cfg.bind)).map_err(|source| HostError::Bind { addr: cfg.bind, source })?;
        let addr = listener.local_addr().map_err(|source| HostError::Bind { addr: cfg.bind, source })?;
        let now = chrono::Local::now().naive_local();
        let mut sessions: Vec<(String, Session)> = Vec::new();
        for s in &cfg.sessions {
            match Session::start(s.project.session_config(now)) {
                Ok(session) => sessions.push((s.name.clone(), session)),
                Err(e) => {
                    for (_, started) in sessions {
                        started.stop();
                    }
                    return Err(HostError::Session { name: s.name.clone(), reason: e.to_string() });
                }
            }
        }
        let mut apps = Vec::new();
        let mut tasks = Vec::new();
        let mut snapshots = Vec::new();
        {
            let _enter = runtime.enter();
            for ((name, session), spec) in sessions.iter().zip(&cfg.sessions) {
                let live = session.live();
                let binding = Arc::new(spec.binding.clone());
                let (feed, task) = Feed::spawn(Arc::clone(&live), Arc::clone(&binding));
                tasks.push(task);
                snapshots.push((name.clone(), Arc::clone(&feed.snapshot)));
                apps.push(Arc::new(SessionApp { name: name.clone(), live, binding, feed }));
            }
        }
        let app = router(apps);
        let server = runtime.spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Host { addr, runtime: Some(runtime), tasks, server: Some(server), sessions, snapshots })
    }

    /// The address bound (a port of 0 resolved).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// A running session by name, for a shell (and a test) acting in process.
    pub fn session(&self, name: &str) -> Option<&Session> {
        self.sessions.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    /// What a session's feed holds now (its objects and status, as its routes read them), for a
    /// shell (and a test) acting in process.
    pub fn snapshot(&self, name: &str) -> Option<crate::feed::Snapshot> {
        self.snapshots.iter().find(|(n, _)| n == name).and_then(|(_, s)| s.read().ok().map(|s| s.clone()))
    }

    /// Stop serving, then stop every session; what each source ended with, by session.
    pub fn stop(mut self) -> Vec<(String, Vec<(SourceId, Result<(), SourceError>)>)> {
        self.halt();
        std::mem::take(&mut self.sessions).into_iter().map(|(name, s)| (name, s.stop())).collect()
    }

    fn halt(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
        if let Some(s) = self.server.take() {
            s.abort();
        }
        // Open streams never end on their own: the runtime is shut down under them.
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.halt();
        for (_, s) in std::mem::take(&mut self.sessions) {
            s.stop();
        }
    }
}

fn not_found(what: String) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": what }))).into_response()
}

fn text(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

async fn index() -> Response {
    text("text/html; charset=utf-8", web::INDEX_HTML)
}

/// Everything the host serves: the page and its bundle, the sessions' list, and per session its
/// binding, layout, object feed, dashboards and its portal router.
pub fn router(apps: Vec<Arc<SessionApp>>) -> Router {
    let list: Arc<Vec<serde_json::Value>> = Arc::new(
        apps.iter().map(|a| json!({ "name": a.name, "title": a.binding.title, "path": format!("/s/{}/", a.name) })).collect(),
    );
    let mut r = Router::new()
        .route("/", get(index))
        .route("/assets/app.js", get(|| async { text("text/javascript; charset=utf-8", web::APP_JS) }))
        .route("/assets/app.css", get(|| async { text("text/css; charset=utf-8", web::APP_CSS) }))
        .route("/api/sessions", get(move || {
            let list = Arc::clone(&list);
            async move { Json(json!({ "sessions": *list })) }
        }));
    for app in apps {
        let base = format!("/s/{}", app.name);
        let session = Router::new()
            .route("/binding", get(binding))
            .route("/layout.svg", get(layout))
            .route("/objects", get(objects))
            .route("/objects.json", get(objects_json))
            .route("/dashboard/{index}", get(dashboard))
            .with_state(Arc::clone(&app))
            .nest("/portal", veredas_portal::serve::router(Arc::clone(&app.live)));
        r = r.route(&base, get(index)).route(&format!("{base}/"), get(index)).nest(&base, session);
    }
    r.fallback(|uri: axum::http::Uri| async move { not_found(format!("nothing is served at {}", uri.path())) })
}

async fn binding(State(app): State<Arc<SessionApp>>) -> Json<serde_json::Value> {
    let mut v = app.binding.page_view();
    v["name"] = json!(app.name);
    Json(v)
}

async fn layout(State(app): State<Arc<SessionApp>>) -> Response {
    match &app.binding.layout {
        Some(l) => ([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-cache")], l.text.clone()).into_response(),
        None => not_found(format!("session `{}` has no layout", app.name)),
    }
}

async fn objects_json(State(app): State<Arc<SessionApp>>) -> Json<serde_json::Value> {
    Json(app.feed.snapshot.read().map(|s| s.to_json()).unwrap_or_default())
}

/// The object feed as a stream: a `snapshot` on connect, then `objects` (what changed) and
/// `session` (the status) events; a client that lags gets a fresh `snapshot`.
async fn objects(State(app): State<Arc<SessionApp>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = app.feed.events.subscribe();
    let snapshot = Arc::clone(&app.feed.snapshot);
    let events = crate::feed::stream(rx, move || snapshot.read().map(|s| s.to_json().to_string()).unwrap_or_default());
    Sse::new(events.map(|(name, data)| Ok(Event::default().event(name).data(data)))).keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
}

#[derive(Deserialize)]
struct DashboardParams {
    to: Option<i64>,
}

async fn dashboard(State(app): State<Arc<SessionApp>>, UrlPath(index): UrlPath<String>, Query(p): Query<DashboardParams>) -> Response {
    let Ok(i) = index.parse::<usize>() else {
        return not_found(format!("`{index}` is no dashboard of session `{}`", app.name));
    };
    if i >= app.binding.dashboards.len() {
        return not_found(format!("session `{}` has no dashboard {i}", app.name));
    }
    let to = p.to.unwrap_or_else(veredas_portal::serve::store_now_ms);
    let snap = app.feed.snapshot.read().map(|s| s.clone()).unwrap_or_default();
    let app2 = Arc::clone(&app);
    let out = tokio::task::spawn_blocking(move || panel::dashboard(&app2.live, &app2.binding, &snap, i, to, now_unix_ms())).await;
    match out {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => not_found(format!("session `{}` has no dashboard {i}", app.name)),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "the dashboard was not computed" }))).into_response(),
    }
}
