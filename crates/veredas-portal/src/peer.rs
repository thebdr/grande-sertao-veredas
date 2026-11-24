//! Portal to portal, the reading side (P-019, C-095): another portal's own HTTP interface (C-044)
//! read over a socket, the way Oficina's portal reads the one embedded in a controller through
//! SSH's tunnel on the controller's loopback. The event stream, the snapshot, the listing and the
//! windowed reads, typed; of the gated surface, its one read (a tag on demand). No command: what
//! alters the controller waits for docs/questions.md Q37.
//!
//! Every call opens a connection of its own and speaks HTTP/1.1 itself over a tokio socket (a GET
//! closing its connection; an answer framed by its length, by chunks or by the connection's end),
//! bounded in time and in size: a peer that stops answering is a timeout, an answer past
//! [`ANSWER_LIMIT`] is refused, and an event stream silent past [`STREAM_SILENCE`] (its server
//! comments every five seconds) is taken for a dead link. An error answer is the peer's own text.
//! Not hyper's client: turning its client side on moves the build of every crate that shares
//! hyper, the database engine's build script among them (docs/portal-plan.md, the traps).

use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

mod json;

/// The time a call waits for its answer, whole, unless the client is given another.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest answer read: a page of rows or a window of points is far below it.
pub const ANSWER_LIMIT: usize = 32 * 1024 * 1024;
/// The longest an event stream may stay silent: its server comments every five seconds on an idle
/// stream, so three missed comments mean the link is gone.
pub const STREAM_SILENCE: Duration = Duration::from_secs(15);
/// The largest event read off the stream (a data frame carries a bounded number of samples).
pub const EVENT_LIMIT: usize = 16 * 1024 * 1024;
/// The longest an answer's head (its status line and headers) may be, and any one line in it or
/// a chunk's size line.
pub const HEAD_LIMIT: usize = 64 * 1024;
/// The most read off the socket at once.
const PIECE: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    #[error("cannot reach the portal at {addr}: {source}")]
    Connect { addr: String, source: std::io::Error },
    #[error("the portal at {addr} did not answer within {ms} ms")]
    Timeout { addr: String, ms: u128 },
    #[error("the link to the portal at {addr} failed: {text}")]
    Link { addr: String, text: String },
    /// The peer's own answer to a call it refused: its status and its error text.
    #[error("the portal answered {code}: {text}")]
    Status { code: u16, text: String },
    #[error("the portal's answer is not in the shape: {0}")]
    Shape(String),
    #[error("the portal's answer is over {limit} bytes")]
    TooLarge { limit: usize },
    /// The event stream ended: the peer stopped serving or closed the connection.
    #[error("the portal's event stream ended")]
    Closed,
    #[error("the portal's event stream was silent for {ms} ms")]
    Silent { ms: u128 },
}

/// A source of the peer's session, as its snapshot lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PeerSource {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub has_timestamp: bool,
}

/// The peer's session snapshot (C-044): what it runs, its clocks and its sources, typed; the
/// health, the vitals, the counters and the recording as the peer writes them, in `as_served`.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerSnapshot {
    pub session_name: String,
    pub is_running: bool,
    pub started_ms: i64,
    pub start_ms: Option<i64>,
    pub window_ms: Option<i64>,
    /// The store's axis now, as the peer read it.
    pub store_now_ms: i64,
    pub arrival_now_ms: i64,
    pub writes_armed: bool,
    pub sources: Vec<PeerSource>,
    /// The whole snapshot as the peer served it.
    pub as_served: Value,
}

/// A field the interface always writes, `null` for none: read as none when `null`, refused when
/// the key is missing (an answer out of the shape is refused, never guessed).
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Deserialize)]
struct SnapshotShape {
    session_name: String,
    running: bool,
    started_ms: i64,
    #[serde(deserialize_with = "present")]
    start_ms: Option<i64>,
    #[serde(deserialize_with = "present")]
    window_ms: Option<i64>,
    now_ms: i64,
    arrival_now_ms: i64,
    writes_armed: bool,
    sources: Vec<PeerSource>,
}

impl PeerSnapshot {
    fn from_value(raw: Value) -> Result<Self, PeerError> {
        let s: SnapshotShape = serde_json::from_value(raw.clone()).map_err(|e| PeerError::Shape(format!("the snapshot: {e}")))?;
        Ok(PeerSnapshot {
            session_name: s.session_name,
            is_running: s.running,
            started_ms: s.started_ms,
            start_ms: s.start_ms,
            window_ms: s.window_ms,
            store_now_ms: s.now_ms,
            arrival_now_ms: s.arrival_now_ms,
            writes_armed: s.writes_armed,
            sources: s.sources,
            as_served: raw,
        })
    }
}

/// One series of the peer's listing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerSeries {
    pub source: String,
    pub series: String,
    /// `int`, `float`, `text` or `time`.
    #[serde(rename = "type")]
    pub ty: String,
    pub count: u64,
    pub keys: u64,
    pub keys_capped: bool,
    #[serde(deserialize_with = "present")]
    pub last_ms: Option<i64>,
    /// The last value as the suite displays it.
    pub last_value: String,
}

/// A numeric window, downsampled by the peer to the budget asked: each point its time and its
/// value, none where the peer served no finite number (NaN or an infinity, which JSON carries as
/// `null`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerWindow {
    pub points: Vec<(i64, Option<f64>)>,
    pub total: u64,
    pub downsampled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PeerColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

/// A table or a view of the peer's store, with its exact row count where one is (a table).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PeerTable {
    pub name: String,
    pub kind: String,
    pub columns: Vec<PeerColumn>,
    #[serde(deserialize_with = "present")]
    pub rows: Option<u64>,
}

/// A page of a table's typed rows, newest first.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerPage {
    #[serde(rename = "schema")]
    pub columns: Vec<PeerColumn>,
    pub rows: Vec<Vec<Value>>,
    pub total: u64,
    pub truncated: bool,
}

/// A tag read on demand by the peer from its plant.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PeerTagRead {
    pub source: String,
    pub tag: String,
    /// The plant's status for the read is good.
    #[serde(rename = "ok")]
    pub good: bool,
    #[serde(deserialize_with = "present")]
    pub value: Option<String>,
    pub status: String,
}

/// One numeric sample of a data event.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PeerSample {
    #[serde(rename = "src")]
    pub source: String,
    #[serde(rename = "ser")]
    pub series: String,
    #[serde(rename = "k", deserialize_with = "present")]
    pub key: Option<String>,
    /// The store's axis: the plant's stamp, else the arrival.
    #[serde(rename = "ms")]
    pub index_ms: i64,
    /// None where the peer served no finite number (NaN or an infinity, which JSON carries as
    /// `null`): the stream goes on.
    #[serde(rename = "v", deserialize_with = "present")]
    pub value: Option<f64>,
    /// The series' count once this sample was taken.
    #[serde(rename = "n")]
    pub count: u64,
}

/// What the peer's event stream carries.
#[derive(Debug, Clone, PartialEq)]
pub enum PeerEvent {
    /// The stream's first event: the snapshot when it opened.
    Hello(PeerSnapshot),
    /// Numeric samples as the peer's store took them.
    Data(Vec<PeerSample>),
    /// Frames the peer dropped for this reader: the cue to read the gap again by window.
    Lag(u64),
}

/// A window of one series: `from_ms` and `to_ms` inclusive, both open when none; the peer
/// downsamples to `points`, clamped by it to 10..=5000, 1000 when none.
#[derive(Debug, Clone, Default)]
pub struct SeriesWindow<'a> {
    pub source: &'a str,
    pub series: &'a str,
    pub key: Option<&'a str>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub points: Option<usize>,
}

/// A page of a table: its rows in a window of its time column, newest first, `limit` of them
/// (the peer's default and cap when none) after `offset`.
#[derive(Debug, Clone, Default)]
pub struct TablePage<'a> {
    pub table: &'a str,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// One numeric column of a table or a view charted against its time column, like a series window.
#[derive(Debug, Clone, Default)]
pub struct TableChart<'a> {
    pub table: &'a str,
    /// The column charted, by name.
    pub y: &'a str,
    pub key: Option<&'a str>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub points: Option<usize>,
}

/// A reader of another portal at `addr` (`host:port`, the tunnel's end on this machine).
#[derive(Debug, Clone)]
pub struct PeerClient {
    addr: String,
    timeout: Duration,
}

impl PeerClient {
    pub fn new(addr: impl Into<String>) -> Self {
        PeerClient { addr: addr.into(), timeout: DEFAULT_TIMEOUT }
    }

    /// The time every call waits for its whole answer, and an event stream for its opening.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn peer_addr(&self) -> &str {
        &self.addr
    }

    pub async fn snapshot(&self) -> Result<PeerSnapshot, PeerError> {
        PeerSnapshot::from_value(self.get_json("/api/session").await?)
    }

    /// The peer's series, sorted by source then series as it lists them.
    pub async fn series(&self) -> Result<Vec<PeerSeries>, PeerError> {
        field(self.get_json("/api/series").await?, "series")
    }

    pub async fn series_window(&self, w: &SeriesWindow<'_>) -> Result<PeerWindow, PeerError> {
        let mut q = Query::new("/api/series/data");
        q.text("source", w.source).text("series", w.series).opt_text("key", w.key);
        q.opt_num("from", w.from_ms).opt_num("to", w.to_ms).opt_num("points", w.points);
        shaped(self.get_json(&q.0).await?, "the window")
    }

    /// The peer's catalog: its base tables, its samples table and its views.
    pub async fn tables(&self) -> Result<Vec<PeerTable>, PeerError> {
        field(self.get_json("/api/tables").await?, "tables")
    }

    pub async fn table_page(&self, p: &TablePage<'_>) -> Result<PeerPage, PeerError> {
        let mut q = Query::new("/api/table/data");
        q.text("name", p.table).opt_num("from", p.from_ms).opt_num("to", p.to_ms);
        q.opt_num("limit", p.limit).opt_num("offset", p.offset);
        shaped(self.get_json(&q.0).await?, "the page")
    }

    pub async fn table_chart(&self, c: &TableChart<'_>) -> Result<PeerWindow, PeerError> {
        let mut q = Query::new("/api/table/data");
        q.text("name", c.table).text("y", c.y).opt_text("key", c.key);
        q.opt_num("from", c.from_ms).opt_num("to", c.to_ms).opt_num("points", c.points);
        shaped(self.get_json(&q.0).await?, "the chart")
    }

    /// One polled tag read on demand by the peer: a read, which its switch does not gate.
    pub async fn tag_read(&self, source: &str, tag: &str) -> Result<PeerTagRead, PeerError> {
        let mut q = Query::new("/api/opc/read");
        q.text("source", source).text("tag", tag);
        shaped(self.get_json(&q.0).await?, "the tag read")
    }

    /// The peer's event stream, opened: its hello comes first.
    pub async fn event_stream(&self) -> Result<PeerEvents, PeerError> {
        let answer = self.bounded(async { refused(self.open("/events", "text/event-stream").await?).await }).await?;
        if !answer.content_type.as_deref().is_some_and(|t| t.starts_with("text/event-stream")) {
            return Err(PeerError::Shape("the event stream is not `text/event-stream`".into()));
        }
        Ok(PeerEvents { body: answer.body, parser: SseParser::default(), ready: VecDeque::new(), silence: STREAM_SILENCE, done: false, heard: tokio::time::Instant::now() })
    }

    async fn get_json(&self, path: &str) -> Result<Value, PeerError> {
        self.bounded(async {
            let mut answer = refused(self.open(path, "application/json").await?).await?;
            let bytes = answer.body.whole().await?;
            read_json(&bytes).map_err(|e| PeerError::Shape(format!("{path}: {e}")))
        })
        .await
    }

    async fn bounded<T>(&self, call: impl std::future::Future<Output = Result<T, PeerError>>) -> Result<T, PeerError> {
        tokio::time::timeout(self.timeout, call).await.map_err(|_| PeerError::Timeout { addr: self.addr.clone(), ms: self.timeout.as_millis() })?
    }

    /// A GET of `path` on a connection of its own, its answer's head read and its body left on
    /// the socket.
    async fn open(&self, path: &str, accept: &str) -> Result<Answer, PeerError> {
        let mut stream = TcpStream::connect(&self.addr).await.map_err(|source| PeerError::Connect { addr: self.addr.clone(), source })?;
        let _ = stream.set_nodelay(true);
        let request = format!("GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: {accept}\r\nConnection: close\r\n\r\n", self.addr);
        stream.write_all(request.as_bytes()).await.map_err(|e| PeerError::Link { addr: self.addr.clone(), text: e.to_string() })?;
        let mut body = Body { addr: self.addr.clone(), reader: BufReader::new(stream), framing: Framing::ToEnd, left: 0, finished: false, line_buf: Vec::new(), crlf_due: false, trailers: false };
        let status_line = body.line().await?;
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next().unwrap_or_default();
        let status = parts.next().and_then(|c| c.parse::<u16>().ok()).filter(|c| (100..600).contains(c));
        let Some(status) = status.filter(|_| version.starts_with("HTTP/1.")) else {
            return Err(PeerError::Shape(format!("the status line `{status_line}`")));
        };
        let (mut content_type, mut length, mut chunked, mut head) = (None, None, false, status_line.len());
        loop {
            let line = body.line().await?;
            head += line.len() + 2;
            if head > HEAD_LIMIT {
                return Err(PeerError::TooLarge { limit: HEAD_LIMIT });
            }
            if line.is_empty() {
                break;
            }
            let Some((name, value)) = line.split_once(':') else {
                return Err(PeerError::Shape(format!("a header line `{line}`")));
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-type" => content_type = Some(value.to_string()),
                "content-length" => length = Some(value.parse::<u64>().map_err(|_| PeerError::Shape(format!("a content length `{value}`")))?),
                "transfer-encoding" => chunked = value.to_ascii_lowercase().split(',').any(|t| t.trim() == "chunked"),
                _ => {}
            }
        }
        // Chunks over a length, as HTTP/1.1 reads a message carrying both; else the length; else
        // the connection's end.
        match (chunked, length) {
            (true, _) => body.framing = Framing::Chunked,
            (false, Some(n)) => {
                body.framing = Framing::Length;
                body.left = n;
            }
            (false, None) => body.framing = Framing::ToEnd,
        }
        Ok(Answer { status, content_type, body })
    }
}

/// An answer's head, and its body still on the socket.
struct Answer {
    status: u16,
    content_type: Option<String>,
    body: Body,
}

/// How an answer's body is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// `Content-Length`: `left` bytes still to read.
    Length,
    /// `Transfer-Encoding: chunked`: `left` bytes still to read of the current chunk.
    Chunked,
    /// Neither: the body runs to the connection's end.
    ToEnd,
}

/// An answer's body on its socket, read so that a call dropped midway (a caller's timeout, a
/// `select!`) loses nothing: the bytes of a line read so far, a chunk's line end still owed and the
/// trailers being read are kept here, and a chunk's data is handed over before its line end is
/// waited for, so the next call goes on where the dropped one stopped.
struct Body {
    addr: String,
    reader: BufReader<TcpStream>,
    framing: Framing,
    left: u64,
    finished: bool,
    line_buf: Vec<u8>,
    crlf_due: bool,
    trailers: bool,
}

impl Body {
    fn link(&self, text: impl Into<String>) -> PeerError {
        PeerError::Link { addr: self.addr.clone(), text: text.into() }
    }

    /// One line, its CRLF (or LF) dropped, at most [`HEAD_LIMIT`] long.
    async fn line(&mut self) -> Result<String, PeerError> {
        // The line's bytes gather in `line_buf`, which outlives a dropped call.
        let room = (HEAD_LIMIT as u64 + 2).saturating_sub(self.line_buf.len() as u64);
        let read = (&mut self.reader).take(room).read_until(b'\n', &mut self.line_buf).await;
        read.map_err(|e| self.link(e.to_string()))?;
        if !self.line_buf.ends_with(b"\n") {
            let partial = std::mem::take(&mut self.line_buf).len();
            return Err(if partial > HEAD_LIMIT {
                PeerError::TooLarge { limit: HEAD_LIMIT }
            } else if partial == 0 {
                self.link("the connection closed before the answer's end")
            } else {
                self.link("the connection closed inside a line")
            });
        }
        let mut raw = std::mem::take(&mut self.line_buf);
        raw.pop();
        if raw.ends_with(b"\r") {
            raw.pop();
        }
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    /// The body's next piece of data, none at its end.
    async fn piece(&mut self) -> Result<Option<Vec<u8>>, PeerError> {
        if self.finished {
            return Ok(None);
        }
        // The line end a chunk's data was handed over without.
        if self.crlf_due {
            if !self.line().await?.is_empty() {
                return Err(PeerError::Shape("a chunk not ended by its line end".into()));
            }
            self.crlf_due = false;
        }
        if self.framing == Framing::Chunked && self.left == 0 && !self.trailers {
            let line = self.line().await?;
            let size = line.split(';').next().unwrap_or_default().trim();
            let size = u64::from_str_radix(size, 16).map_err(|_| PeerError::Shape(format!("a chunk's size `{line}`")))?;
            if size == 0 {
                self.trailers = true;
            } else {
                self.left = size;
            }
        }
        if self.trailers {
            // The trailers, then the blank line that ends the message.
            while !self.line().await?.is_empty() {}
            self.trailers = false;
            self.finished = true;
            return Ok(None);
        }
        if self.framing == Framing::Length && self.left == 0 {
            self.finished = true;
            return Ok(None);
        }
        let want = match self.framing {
            Framing::ToEnd => PIECE,
            _ => self.left.min(PIECE as u64) as usize,
        };
        let mut buf = vec![0u8; want];
        let n = self.reader.read(&mut buf).await.map_err(|e| self.link(e.to_string()))?;
        if n == 0 {
            if self.framing == Framing::ToEnd {
                self.finished = true;
                return Ok(None);
            }
            return Err(self.link(format!("the connection closed {} bytes short of the answer's end", self.left)));
        }
        buf.truncate(n);
        if self.framing != Framing::ToEnd {
            self.left -= n as u64;
            // The chunk's line end is read by the next call, so its data is never held across a
            // wait.
            self.crlf_due = self.framing == Framing::Chunked && self.left == 0;
        }
        Ok(Some(buf))
    }

    /// The whole body, refused past [`ANSWER_LIMIT`] (a length past it before a byte is read).
    async fn whole(&mut self) -> Result<Vec<u8>, PeerError> {
        if self.framing == Framing::Length && self.left > ANSWER_LIMIT as u64 {
            return Err(PeerError::TooLarge { limit: ANSWER_LIMIT });
        }
        let mut out = Vec::new();
        while let Some(piece) = self.piece().await? {
            if out.len() + piece.len() > ANSWER_LIMIT {
                return Err(PeerError::TooLarge { limit: ANSWER_LIMIT });
            }
            out.extend_from_slice(&piece);
        }
        Ok(out)
    }
}

/// An answer that is no success, as the peer's own error: its status and the text of its
/// `{"error": ...}` body, else the body as it came.
async fn refused(mut answer: Answer) -> Result<Answer, PeerError> {
    if (200..300).contains(&answer.status) {
        return Ok(answer);
    }
    let bytes = answer.body.whole().await.unwrap_or_default();
    let text = read_json(&bytes)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    Err(PeerError::Status { code: answer.status, text })
}

/// An answer's body as JSON, every number read as its text says ([`json`]).
fn read_json(bytes: &[u8]) -> Result<Value, String> {
    json::parse(std::str::from_utf8(bytes).map_err(|_| "the answer is no UTF-8".to_string())?)
}

fn shaped<T: serde::de::DeserializeOwned>(v: Value, what: &str) -> Result<T, PeerError> {
    serde_json::from_value(v).map_err(|e| PeerError::Shape(format!("{what}: {e}")))
}

fn field<T: serde::de::DeserializeOwned>(mut v: Value, name: &str) -> Result<T, PeerError> {
    match v.get_mut(name) {
        Some(inner) => shaped(inner.take(), name),
        None => Err(PeerError::Shape(format!("no `{name}` in the answer"))),
    }
}

/// A path with its query, every value percent-encoded.
struct Query(String);

impl Query {
    fn new(path: &str) -> Self {
        Query(path.to_string())
    }

    fn text(&mut self, name: &str, value: &str) -> &mut Self {
        self.0.push(if self.0.contains('?') { '&' } else { '?' });
        self.0.push_str(name);
        self.0.push('=');
        for b in value.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                self.0.push(b as char);
            } else {
                self.0.push_str(&format!("%{b:02X}"));
            }
        }
        self
    }

    fn opt_text(&mut self, name: &str, value: Option<&str>) -> &mut Self {
        if let Some(v) = value {
            self.text(name, v);
        }
        self
    }

    fn opt_num(&mut self, name: &str, value: Option<impl std::fmt::Display>) -> &mut Self {
        if let Some(v) = value {
            self.text(name, &v.to_string());
        }
        self
    }
}

/// An open event stream of a peer.
pub struct PeerEvents {
    body: Body,
    parser: SseParser,
    ready: VecDeque<(String, String)>,
    silence: Duration,
    done: bool,
    /// When the stream last gave a byte: the silence runs from here, across every call, so a
    /// caller dropping its calls more often than the silence still learns the link is gone.
    heard: tokio::time::Instant,
}

impl PeerEvents {
    /// The next event, cancel-safe: a call dropped while it waits (a caller's timeout) loses no
    /// byte of the stream, and the next call returns what was in flight. An error ends the stream (`Closed` when the peer ended it, `Silent` when
    /// nothing came for [`STREAM_SILENCE`], `Link` when the connection broke), and every later
    /// call answers `Closed`: a reader opens a new stream, whose hello says where the peer is, and
    /// reads the gap again by window. An event of a kind this reader does not know is skipped.
    pub async fn next_event(&mut self) -> Result<PeerEvent, PeerError> {
        loop {
            if self.done {
                return Err(PeerError::Closed);
            }
            if let Some((kind, data)) = self.ready.pop_front() {
                match event_of(&kind, &data) {
                    Ok(Some(e)) => return Ok(e),
                    Ok(None) => continue,
                    Err(e) => return self.end(e),
                }
            }
            let data = match tokio::time::timeout_at(self.heard + self.silence, self.body.piece()).await {
                Err(_) => return self.end(PeerError::Silent { ms: self.silence.as_millis() }),
                Ok(Ok(None)) => return self.end(PeerError::Closed),
                Ok(Err(e)) => return self.end(e),
                Ok(Ok(Some(data))) => {
                    self.heard = tokio::time::Instant::now();
                    data
                }
            };
            if let Err(e) = self.parser.push(&data, &mut self.ready) {
                return self.end(e);
            }
        }
    }

    /// The silence after which the stream is taken for dead, [`STREAM_SILENCE`] unless set.
    pub fn with_silence(mut self, silence: Duration) -> Self {
        self.silence = silence;
        self
    }

    fn end(&mut self, e: PeerError) -> Result<PeerEvent, PeerError> {
        self.done = true;
        Err(e)
    }
}

fn event_of(kind: &str, data: &str) -> Result<Option<PeerEvent>, PeerError> {
    match kind {
        "hello" => {
            let v = json::parse(data).map_err(|e| PeerError::Shape(format!("the hello: {e}")))?;
            PeerSnapshot::from_value(v).map(|s| Some(PeerEvent::Hello(s)))
        }
        "data" => {
            #[derive(Deserialize)]
            struct Frame {
                samples: Vec<PeerSample>,
            }
            let f: Frame = json::parse(data).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())).map_err(|e| PeerError::Shape(format!("a data event: {e}")))?;
            Ok(Some(PeerEvent::Data(f.samples)))
        }
        // `frames dropped: N`
        "lag" => data
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .map(|n| Some(PeerEvent::Lag(n)))
            .ok_or_else(|| PeerError::Shape(format!("a lag event: `{data}`"))),
        _ => Ok(None),
    }
}

/// The event stream's text (the WHATWG rules): lines end at LF, CRLF or a lone CR, even split
/// across reads; `event:` names the event, `data:` lines join with a line feed, one space after
/// the colon dropped; a line starting with `:` is a comment; a blank line dispatches an event
/// that has data, as `message` when unnamed.
#[derive(Default)]
struct SseParser {
    line: Vec<u8>,
    after_cr: bool,
    event: String,
    data: String,
    has_data: bool,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8], out: &mut VecDeque<(String, String)>) -> Result<(), PeerError> {
        for &b in chunk {
            if std::mem::take(&mut self.after_cr) && b == b'\n' {
                continue;
            }
            match b {
                b'\r' => {
                    self.after_cr = true;
                    self.line_end(out);
                }
                b'\n' => self.line_end(out),
                _ => {
                    self.line.push(b);
                    if self.line.len() + self.data.len() > EVENT_LIMIT {
                        return Err(PeerError::TooLarge { limit: EVENT_LIMIT });
                    }
                }
            }
        }
        Ok(())
    }

    fn line_end(&mut self, out: &mut VecDeque<(String, String)>) {
        let raw = std::mem::take(&mut self.line);
        if raw.is_empty() {
            if self.has_data {
                let kind = std::mem::take(&mut self.event);
                out.push_back((if kind.is_empty() { "message".into() } else { kind }, std::mem::take(&mut self.data)));
            }
            self.event.clear();
            self.data.clear();
            self.has_data = false;
            return;
        }
        let line = String::from_utf8_lossy(&raw);
        if line.starts_with(':') {
            return;
        }
        let (name, value) = match line.split_once(':') {
            Some((n, v)) => (n, v.strip_prefix(' ').unwrap_or(v)),
            None => (&*line, ""),
        };
        match name {
            "event" => self.event = value.to_string(),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]]) -> Vec<(String, String)> {
        let mut p = SseParser::default();
        let mut out = VecDeque::new();
        for c in chunks {
            p.push(c, &mut out).unwrap();
        }
        out.into_iter().collect()
    }

    #[test]
    fn the_stream_s_text_is_read_by_its_rules_whatever_the_reads() {
        let ev = |k: &str, d: &str| (k.to_string(), d.to_string());
        // Every line ending, a comment, an unnamed event, data on two lines, a field without a
        // colon, the one space after the colon.
        let text: &[u8] = b": keep\r\nevent: hello\r\ndata: {\"a\":1}\r\n\r\nevent:lag\rdata:frames dropped: 3\r\rdata: x\ndata:  y\ndata\n\nid: 7\nretry: 5\n\n";
        let want = vec![ev("hello", "{\"a\":1}"), ev("lag", "frames dropped: 3"), ev("message", "x\n y\n")];
        assert_eq!(parse(&[text]), want);
        // Split at every byte, a CRLF and a UTF-8 character cut in two among them.
        let bytes: Vec<&[u8]> = text.chunks(1).collect();
        assert_eq!(parse(&bytes), want);
        let accented = "event: data\r\ndata: Supervisão\r\n\r\n".as_bytes();
        let cut = accented.iter().position(|&b| b == 0xC3).unwrap() + 1;
        assert_eq!(parse(&[&accented[..cut], &accented[cut..]]), vec![ev("data", "Supervisão")]);
        // An event never dispatched without its blank line, nor one with no data.
        assert_eq!(parse(&[b"event: hello\n\ndata: half".as_slice()]), vec![]);
    }

    #[test]
    fn an_event_past_its_limit_is_refused_before_it_ends() {
        let mut p = SseParser::default();
        let mut out = VecDeque::new();
        p.push(b"event: data\ndata: ", &mut out).unwrap();
        let piece = vec![b'x'; 1 << 20];
        let mut refused = None;
        for _ in 0..=(EVENT_LIMIT >> 20) {
            if let Err(e) = p.push(&piece, &mut out) {
                refused = Some(e);
                break;
            }
        }
        assert!(matches!(refused, Some(PeerError::TooLarge { limit: EVENT_LIMIT })), "{refused:?}");
        assert!(out.is_empty());
    }

    #[test]
    fn a_query_s_values_are_percent_encoded() {
        let mut q = Query::new("/api/series/data");
        q.text("source", "plc main").text("series", "a&b=c/ç").opt_text("key", None).opt_num("from", Some(-5)).opt_num("to", None::<i64>);
        assert_eq!(q.0, "/api/series/data?source=plc%20main&series=a%26b%3Dc%2F%C3%A7&from=-5");
    }

    #[test]
    fn a_lag_event_carries_its_count_and_an_unknown_event_is_skipped() {
        assert_eq!(event_of("lag", "frames dropped: 12").unwrap(), Some(PeerEvent::Lag(12)));
        assert!(matches!(event_of("lag", "frames dropped"), Err(PeerError::Shape(_))));
        assert_eq!(event_of("ping", "{}").unwrap(), None);
        assert!(matches!(event_of("data", "{\"samples\": 3}"), Err(PeerError::Shape(_))));
    }
}
