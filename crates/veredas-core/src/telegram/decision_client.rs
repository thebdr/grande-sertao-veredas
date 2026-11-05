//! Asking a decision service where a parcel goes, and telling it what
//! happened (Veredas 1's trajetorias; maquinar's automated replies in
//! Veredas 2).
//!
//! A deliberately small HTTP/1.1 client rather than a framework one: the
//! traffic is one JSON object to a service we wrote, on a LAN, and
//! pulling hyper into this crate would put it into maquinar's binary too.
//! Every exchange is a fresh connection with `Connection: close`, so
//! there is no pool to go stale and no keep-alive state to reason about;
//! at sorter rates (a few parcels a second) that is free.
//!
//! **The rules that matter more than the transport:**
//!
//! - Every call is **bounded**. A sort controller waits tens of
//!   milliseconds for a destination; a decision service that hangs must
//!   produce a fallback, not a stalled channel.
//! - A failure is **never** an invented destination. The caller answers
//!   the wire with "ask me again" (FORCE_REASSIGNEMENT) and the parcel
//!   keeps circulating — the honest answer, and recoverable.
//! - Lifecycle events are **fire-and-forget through a bounded queue**: a
//!   slow or dead trajetorias must never delay a reply. When the queue
//!   overflows the oldest events are dropped and counted, because losing
//!   debugging history is survivable and missing a sort decision is not.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What veredas asks trajetorias about one parcel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecideRequest {
    pub object_id: u32,
    pub cell: u16,
    /// The captured fields (`infeed`, `id2`, `barcode1`…).
    pub fields: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub width_mm: Option<u32>,
    #[serde(default)]
    pub length_mm: Option<u32>,
    #[serde(default)]
    pub height_mm: Option<u32>,
    #[serde(default)]
    pub weight_g: Option<u32>,
    #[serde(default)]
    pub volume: Option<u32>,
}

/// What trajetorias answers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecideResponse {
    /// The chute. Absent = trajetorias has no answer either, and the
    /// caller should send FORCE_REASSIGNEMENT.
    #[serde(default)]
    pub destination: Option<u16>,
    /// Alternates, in the order the MSC should try them.
    #[serde(default)]
    pub alternatives: Vec<u16>,
    /// Which tier decided — recorded in the audit.
    #[serde(default)]
    pub tier: String,
    /// The store's id for this decision, so the reply that follows can
    /// be tied to it.
    #[serde(default)]
    pub decision_id: Option<i64>,
}

/// One thing that happened to a parcel, on its way to the store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    /// Milliseconds since the unix epoch, when veredas saw it.
    pub at_ms: i64,
    pub object_id: Option<u32>,
    /// `msc->eu` or `eu->msc`.
    pub direction: String,
    /// The channel it travelled on, as configured (`CHM`, `CHR`…).
    pub channel: String,
    /// The decoded message kind (`scanner request`, `outcome`…).
    pub kind: String,
    pub msg_id: u16,
    /// Decoded fields worth querying without re-parsing the frame.
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
    /// **The complete frame, hex.** Lossless: every stored event can be
    /// decoded again, which is the point of keeping it.
    pub raw_hex: String,
}

impl LifecycleEvent {
    /// Render a frame as the hex the store keeps.
    pub fn hex(raw: &[u8]) -> String {
        raw.iter().map(|b| format!("{b:02X}")).collect()
    }

    /// Read it back — the store's promise is that this round-trips.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        crate::telegram::codec::parse_hex(&self.raw_hex).ok()
    }
}

/// Where trajetorias is, and how patient to be with it.
#[derive(Clone, Debug)]
pub struct DecisionClient {
    /// e.g. `http://127.0.0.1:8930` — another machine is fine.
    base_url: String,
    /// How long one call may take before the fallback answers instead.
    timeout: Duration,
}

impl DecisionClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> DecisionClient {
        DecisionClient {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            timeout,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Ask where a parcel goes.
    ///
    /// Any failure — unreachable, slow, malformed — is an `Err` the
    /// caller turns into "ask me again". It never guesses a chute.
    pub async fn decide(&self, request: &DecideRequest) -> Result<DecideResponse, DecisionError> {
        let body = serde_json::to_vec(request).map_err(DecisionError::Encode)?;
        let response = self.post("/decide", &body).await?;
        serde_json::from_slice(&response).map_err(DecisionError::Decode)
    }

    /// Send a batch of lifecycle events.
    pub async fn post_events(&self, events: &[LifecycleEvent]) -> Result<(), DecisionError> {
        if events.is_empty() {
            return Ok(());
        }
        let body = serde_json::to_vec(events).map_err(DecisionError::Encode)?;
        self.post("/events", &body).await.map(|_| ())
    }

    /// Tell the decision service a chute's state changed (FULL, JAM, recovered).
    pub async fn post_bin_state(
        &self,
        dest: u16,
        state: u32,
        blocked: bool,
    ) -> Result<(), DecisionError> {
        let body = serde_json::json!({
            "destination": dest,
            "state": state,
            "blocked": blocked,
        });
        let body = serde_json::to_vec(&body).map_err(DecisionError::Encode)?;
        self.post("/binstate", &body).await.map(|_| ())
    }

    async fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, DecisionError> {
        let (host, port) = split_host_port(&self.base_url)?;
        let exchange = async {
            let mut socket = tokio::net::TcpStream::connect((host.as_str(), port))
                .await
                .map_err(|source| DecisionError::Connect {
                    addr: format!("{host}:{port}"),
                    source,
                })?;
            // Small request, small reply: Nagle would only add latency.
            let _ = socket.set_nodelay(true);

            let head = format!(
                "POST {path} HTTP/1.1\r\n\
                 Host: {host}:{port}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            socket
                .write_all(head.as_bytes())
                .await
                .map_err(DecisionError::Write)?;
            socket.write_all(body).await.map_err(DecisionError::Write)?;

            // `Connection: close` means the reply ends at EOF — no
            // Content-Length or chunked decoding to get wrong.
            let mut response = Vec::new();
            socket
                .read_to_end(&mut response)
                .await
                .map_err(DecisionError::Read)?;
            Ok::<Vec<u8>, DecisionError>(response)
        };

        let response = tokio::time::timeout(self.timeout, exchange)
            .await
            .map_err(|_| DecisionError::Timeout(self.timeout))??;
        parse_response(&response)
    }
}

/// Why a call to the decision service produced no answer. The caller's
/// fallback is the same for every one ("ask me again"); the variant says
/// what to fix.
#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("`{0}` is not an http:// address")]
    NotHttp(String),
    #[error("`{0}` is not a port number")]
    BadPort(String),
    #[error("encoding: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("connecting to {addr}: {source}")]
    Connect {
        addr: String,
        #[source]
        source: std::io::Error,
    },
    #[error("writing: {0}")]
    Write(#[source] std::io::Error),
    #[error("reading: {0}")]
    Read(#[source] std::io::Error),
    #[error("no answer within {0:?}")]
    Timeout(Duration),
    #[error("the answer had no headers")]
    NoHeaders,
    #[error("unreadable status line: `{0}`")]
    StatusLine(String),
    /// The service answered, but not with success: its refusal is not an answer.
    #[error("the decision service answered {0}")]
    Refused(u16),
    #[error("decoding the answer: {0}")]
    Decode(#[source] serde_json::Error),
}

/// Split `http://host:port` into its parts.
fn split_host_port(url: &str) -> Result<(String, u16), DecisionError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| DecisionError::NotHttp(url.to_string()))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| DecisionError::BadPort(port.to_string()))?;
            Ok((host.to_string(), port))
        }
        None => Ok((authority.to_string(), 80)),
    }
}

/// Pull the body out of an HTTP response, refusing non-2xx.
fn parse_response(response: &[u8]) -> Result<Vec<u8>, DecisionError> {
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(DecisionError::NoHeaders)?;
    let head = String::from_utf8_lossy(&response[..split]);
    let status_line = head.lines().next().unwrap_or_default();
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| DecisionError::StatusLine(status_line.to_string()))?;
    if !(200..300).contains(&code) {
        return Err(DecisionError::Refused(code));
    }
    Ok(response[split + 4..].to_vec())
}

// ---------------------------------------------------------------------------
// The lifecycle queue
// ---------------------------------------------------------------------------

/// A bounded outbound queue for lifecycle events.
///
/// The channel worker pushes; a background task drains. Overflow drops
/// the OLDEST events and counts them: a stalled store must never slow a
/// sort decision, and recent history is more useful than ancient.
#[derive(Clone)]
pub struct EventQueue {
    inner: Arc<Mutex<QueueInner>>,
    capacity: usize,
}

struct QueueInner {
    events: VecDeque<LifecycleEvent>,
    dropped: u64,
    sent: u64,
    last_error: Option<String>,
}

impl EventQueue {
    pub fn new(capacity: usize) -> EventQueue {
        EventQueue {
            inner: Arc::new(Mutex::new(QueueInner {
                events: VecDeque::new(),
                dropped: 0,
                sent: 0,
                last_error: None,
            })),
            capacity: capacity.max(1),
        }
    }

    /// Queue an event. Never blocks, never fails.
    pub fn push(&self, event: LifecycleEvent) {
        if let Ok(mut inner) = self.inner.lock() {
            while inner.events.len() >= self.capacity {
                inner.events.pop_front();
                inner.dropped += 1;
            }
            inner.events.push_back(event);
        }
    }

    /// Take up to `max` events to send.
    pub fn take(&self, max: usize) -> Vec<LifecycleEvent> {
        let Ok(mut inner) = self.inner.lock() else {
            return Vec::new();
        };
        let n = max.min(inner.events.len());
        inner.events.drain(..n).collect()
    }

    /// Put a failed batch back at the FRONT, so ordering survives a
    /// transient failure (subject to the same overflow rule).
    pub fn requeue(&self, events: Vec<LifecycleEvent>, why: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_error = Some(why);
            for event in events.into_iter().rev() {
                if inner.events.len() >= self.capacity {
                    inner.dropped += 1;
                    continue;
                }
                inner.events.push_front(event);
            }
        }
    }

    pub fn note_sent(&self, n: usize) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.sent += n as u64;
            inner.last_error = None;
        }
    }

    pub fn stats(&self) -> QueueStats {
        self.inner
            .lock()
            .map(|inner| QueueStats {
                queued: inner.events.len(),
                dropped: inner.dropped,
                sent: inner.sent,
                last_error: inner.last_error.clone(),
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub queued: usize,
    pub dropped: u64,
    pub sent: u64,
    pub last_error: Option<String>,
}
