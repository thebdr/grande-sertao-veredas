//! Worker plumbing: the stop/join handle every source worker thread returns, and its error.
//!
//! One health model for every source, a tailed log or an OPC poll: the worker owns a thread,
//! watches a shared stop flag, and ends with `Result<(), SourceError>`; the handle stops it,
//! polls whether it has ended without blocking, and keeps the outcome once seen.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use thiserror::Error;

use crate::types::SourceId;

/// Why a source worker failed. An I/O failure always carries the path it happened on:
/// "access denied" without a path is useless when four shares are read at once.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid pattern: {0}")]
    Pattern(#[from] regex::Error),

    #[error("family spec: {0}")]
    Spec(String),

    #[error("backfill: {0}")]
    Backfill(String),

    #[error("source not configured: {0}")]
    NotConfigured(String),

    #[error("source worker thread panicked")]
    Panicked,
}

impl SourceError {
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        SourceError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Stop/join handle for one running source worker.
pub struct SourceHandle {
    id: SourceId,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<(), SourceError>>>,
    /// The worker's result once seen, so health polling can read a dead worker's error
    /// without consuming the handle.
    outcome: Option<Result<(), SourceError>>,
}

impl SourceHandle {
    pub fn new(
        id: SourceId,
        stop: Arc<AtomicBool>,
        join: JoinHandle<Result<(), SourceError>>,
    ) -> Self {
        SourceHandle {
            id,
            stop,
            join: Some(join),
            outcome: None,
        }
    }

    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// Ask the worker to wind down. Non-blocking; the worker notices on its next tick.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.outcome.is_some() || self.join.as_ref().is_none_or(|j| j.is_finished())
    }

    /// If the worker has exited, join it (it is done, so this does not block) and keep its
    /// result. Returns the kept outcome, or `None` while the worker runs. Never blocks a live
    /// worker, so it is safe to call every frame for health surfacing.
    pub fn poll_outcome(&mut self) -> Option<&Result<(), SourceError>> {
        if self.outcome.is_none()
            && let Some(j) = self.join.take_if(|j| j.is_finished())
        {
            self.outcome = Some(j.join().map_err(|_| SourceError::Panicked).and_then(|r| r));
        }
        self.outcome.as_ref()
    }

    /// Stop and wait for the worker, surfacing its exit result; the outcome
    /// [`poll_outcome`](Self::poll_outcome) already kept, if it kept one.
    pub fn join(mut self) -> Result<(), SourceError> {
        self.request_stop();
        if let Some(outcome) = self.outcome.take() {
            return outcome;
        }
        match self.join.take() {
            None => Ok(()),
            Some(join) => join.join().map_err(|_| SourceError::Panicked)?,
        }
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        // Signal only: a drop never blocks on a worker thread.
        self.request_stop();
    }
}

/// A fresh shared stop flag for a worker loop, lowered.
pub fn stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
