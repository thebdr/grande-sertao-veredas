//! The line-producing worker model: a [`Source`] starts a worker that pushes tagged
//! [`LineEvent`]s into a channel, a [`SourceSet`] merges every worker's stream into one.
//!
//! Channel-based on purpose: the stream is consumed headless (tests), from a frame loop
//! (`Receiver::try_iter` per frame), or bridged into an async runtime by the server. The handle
//! is core's [`SourceHandle`], one health model for every worker of the suite.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};

use veredas_core::{SourceError, SourceHandle, SourceId, now_unix_ms};

use crate::timestamp::{TimestampExtractor, TimestampSpec};

/// One log line, tagged with its origin.
///
/// `seq` is a per-source monotonic counter assigned at emission: the ordering fallback whenever
/// a line's stamp is missing or does not parse. `parsed_ts` is the stamp read from the line by
/// the source's timestamp rule (the source's wall clock, no zone); `arrival_unix_ms` is kept
/// beside it forever, because cross-source arithmetic never trusts a parsed stamp alone.
#[derive(Clone, Debug, PartialEq)]
pub struct LineEvent {
    pub source: SourceId,
    pub seq: u64,
    pub arrival_unix_ms: i64,
    pub parsed_ts: Option<chrono::NaiveDateTime>,
    pub line: String,
}

impl LineEvent {
    /// The time index: the parsed stamp when present (read as if UTC), arrival otherwise, on
    /// the axis every sample of the suite lands on.
    pub fn index_ms(&self) -> i64 {
        match self.parsed_ts {
            Some(t) => t.and_utc().timestamp_millis(),
            None => self.arrival_unix_ms,
        }
    }
}

/// A stream of tagged log lines.
///
/// An implementation spawns its own worker thread in `start` and pushes [`LineEvent`]s into
/// the given sender until the source ends, fails, the consumer disconnects, or the handle's
/// stop flag is raised.
pub trait Source: Send {
    fn id(&self) -> &SourceId;
    fn display_name(&self) -> &str;

    /// Consume the source and start streaming.
    fn start(self: Box<Self>, tx: Sender<LineEvent>) -> Result<SourceHandle, SourceError>;
}

/// The consumer went away; a worker treats it as a clean end, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected;

/// Stamps the source id, the per-source dense `seq`, the arrival clock and, when a timestamp
/// rule is configured, the parsed stamp onto every emitted line. One per source worker.
pub struct Emitter {
    source: SourceId,
    tx: Sender<LineEvent>,
    seq: u64,
    ts: Option<TimestampExtractor>,
}

impl Emitter {
    pub fn new(source: SourceId, tx: Sender<LineEvent>) -> Self {
        Emitter::with_timestamps(source, tx, None)
    }

    pub fn with_timestamps(source: SourceId, tx: Sender<LineEvent>, spec: Option<TimestampSpec>) -> Self {
        Emitter { source, tx, seq: 0, ts: spec.map(TimestampExtractor::new) }
    }

    pub fn emit(&mut self, line: String) -> Result<(), Disconnected> {
        let parsed_ts = self.ts.as_mut().and_then(|x| x.extract(&line));
        let event = LineEvent {
            source: self.source.clone(),
            seq: self.seq,
            arrival_unix_ms: now_unix_ms(),
            parsed_ts,
            line,
        };
        self.seq += 1;
        self.tx.send(event).map_err(|_| Disconnected)
    }

    pub fn next_seq(&self) -> u64 {
        self.seq
    }
}

/// A worker's liveness at a poll. `finished` once the thread has exited; `error` its failure's
/// text when it exited unsuccessfully (a running or cleanly ended worker has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerReport {
    pub id: SourceId,
    pub finished: bool,
    pub error: Option<String>,
}

/// Owns the merged multi-source stream: every spawned source shares one sender, the one receiver
/// yields the interleaved tagged stream. Order is preserved within a source (one worker each);
/// across sources it is arrival order.
///
/// The receiver disconnects only after `stop_all` (the set holds a sender clone), so a consumer
/// outlives a momentarily empty source list.
pub struct SourceSet {
    tx: Sender<LineEvent>,
    handles: Vec<SourceHandle>,
}

impl SourceSet {
    pub fn new() -> (SourceSet, Receiver<LineEvent>) {
        let (tx, rx) = channel();
        (SourceSet { tx, handles: Vec::new() }, rx)
    }

    /// A sender into the merged stream, for a producer that is not a [`Source`].
    pub fn sender(&self) -> Sender<LineEvent> {
        self.tx.clone()
    }

    /// Start a source and merge its stream into this set's receiver.
    pub fn spawn(&mut self, source: Box<dyn Source>) -> Result<&SourceHandle, SourceError> {
        let handle = source.start(self.tx.clone())?;
        self.handles.push(handle);
        self.handles.last().ok_or_else(|| SourceError::NotConfigured("the handle was not kept".into()))
    }

    pub fn handles(&self) -> &[SourceHandle] {
        &self.handles
    }

    /// Stop and join one source, leaving the rest running. The worker's exit result, or `None`
    /// when no source with that id is in the set. The merged receiver stays connected.
    pub fn remove(&mut self, id: &SourceId) -> Option<Result<(), SourceError>> {
        let pos = self.handles.iter().position(|h| h.id() == id)?;
        Some(self.handles.remove(pos).join())
    }

    /// Every worker's liveness, for health. Never blocks on a live worker.
    pub fn worker_reports(&mut self) -> Vec<WorkerReport> {
        self.handles
            .iter_mut()
            .map(|h| {
                let id = h.id().clone();
                match h.poll_outcome() {
                    None => WorkerReport { id, finished: false, error: None },
                    Some(Ok(())) => WorkerReport { id, finished: true, error: None },
                    Some(Err(e)) => WorkerReport { id, finished: true, error: Some(e.to_string()) },
                }
            })
            .collect()
    }

    /// Stop every source, join the workers, and drop the set's sender so the receiver drains
    /// and then disconnects.
    pub fn stop_all(self) -> Vec<(SourceId, Result<(), SourceError>)> {
        for h in &self.handles {
            h.request_stop();
        }
        self.handles.into_iter().map(|h| (h.id().clone(), h.join())).collect()
    }
}

/// A fresh stop flag, lowered: core's, re-exported for the workers here.
pub(crate) fn stop_flag() -> Arc<AtomicBool> {
    veredas_core::stop_flag()
}
