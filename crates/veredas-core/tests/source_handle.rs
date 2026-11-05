//! Worker plumbing (contract: the worker-handle clause): the stop/join handle every source
//! worker returns, over real threads. Veredas 1 proved it through its `SourceSet`, which stays
//! with the data plane; here the handle is driven directly.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use veredas_core::{SourceError, SourceHandle, SourceId, stop_flag};

/// A worker that ticks until its stop flag rises, then ends cleanly.
fn until_stopped(id: &str) -> SourceHandle {
    let stop = stop_flag();
    let flag = Arc::clone(&stop);
    let join = thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    });
    SourceHandle::new(SourceId::new(id), stop, join)
}

/// A worker that ends on its own with `result`, without looking at its flag.
fn ending_with(result: Result<(), SourceError>) -> SourceHandle {
    let join = thread::spawn(move || result);
    SourceHandle::new(SourceId::new("w"), stop_flag(), join)
}

fn wait_finished(h: &mut SourceHandle) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.poll_outcome().is_none() {
        assert!(Instant::now() < deadline, "the worker never finished");
        thread::sleep(Duration::from_millis(2));
    }
}

fn share_unreachable() -> SourceError {
    SourceError::io(
        Path::new(r"\\server\share\log"),
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "share unreachable"),
    )
}

#[test]
fn a_fresh_stop_flag_is_lowered_and_not_shared() {
    let a = stop_flag();
    let b = stop_flag();
    assert!(!a.load(Ordering::Relaxed));
    a.store(true, Ordering::Relaxed);
    assert!(!b.load(Ordering::Relaxed), "each worker gets its own flag");
}

#[test]
fn polling_a_live_worker_never_blocks_and_reports_nothing() {
    let mut h = until_stopped("live");
    assert_eq!(h.id(), &SourceId::new("live"));
    let started = Instant::now();
    for _ in 0..100 {
        assert!(h.poll_outcome().is_none());
    }
    assert!(!h.is_finished());
    assert!(started.elapsed() < Duration::from_secs(1), "polling blocked on a live worker");
    assert!(h.join().is_ok(), "join raises the flag, so the worker ends cleanly");
}

#[test]
fn request_stop_reaches_the_worker_and_its_end_is_seen() {
    let mut h = until_stopped("w");
    h.request_stop();
    wait_finished(&mut h);
    assert!(h.is_finished());
    assert!(matches!(h.poll_outcome(), Some(Ok(()))));
    assert!(h.join().is_ok());
}

#[test]
fn a_failed_worker_keeps_its_error_for_every_poll_and_for_join() {
    let mut h = ending_with(Err(share_unreachable()));
    wait_finished(&mut h);
    for _ in 0..3 {
        let msg = h.poll_outcome().unwrap().as_ref().unwrap_err().to_string();
        assert_eq!(msg, r"I/O error on \\server\share\log: share unreachable");
    }
    // Join returns the outcome the poll kept: the thread is joined once, never twice.
    match h.join() {
        Err(SourceError::Io { path, source }) => {
            assert_eq!(path, Path::new(r"\\server\share\log"));
            assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        }
        other => panic!("expected the kept I/O error, got {other:?}"),
    }
}

#[test]
fn join_without_a_poll_surfaces_the_worker_result() {
    let h = ending_with(Err(SourceError::NotConfigured("no endpoint".into())));
    match h.join() {
        Err(SourceError::NotConfigured(why)) => assert_eq!(why, "no endpoint"),
        other => panic!("expected NotConfigured, got {other:?}"),
    }
    assert!(ending_with(Ok(())).join().is_ok());
}

#[test]
fn a_panicking_worker_is_reported_as_panicked_not_as_a_crash() {
    let mut h = SourceHandle::new(
        SourceId::new("p"),
        stop_flag(),
        thread::spawn(|| -> Result<(), SourceError> { panic!("worker bug") }),
    );
    wait_finished(&mut h);
    assert_eq!(
        h.poll_outcome().unwrap().as_ref().unwrap_err().to_string(),
        "source worker thread panicked"
    );
    assert!(matches!(h.join(), Err(SourceError::Panicked)));

    // The same through join alone.
    let h = SourceHandle::new(
        SourceId::new("p"),
        stop_flag(),
        thread::spawn(|| -> Result<(), SourceError> { panic!("worker bug") }),
    );
    assert!(matches!(h.join(), Err(SourceError::Panicked)));
}

/// A panic payload whose own drop panics too.
struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("the payload's drop panics too");
    }
}

#[test]
fn a_panic_is_panicked_even_when_its_payload_panics_on_drop() {
    let bomb = || -> Result<(), SourceError> { std::panic::panic_any(Bomb) };
    let mut h = SourceHandle::new(SourceId::new("b"), stop_flag(), thread::spawn(bomb));
    wait_finished(&mut h);
    assert!(matches!(h.poll_outcome(), Some(Err(SourceError::Panicked))));
    assert!(matches!(h.join(), Err(SourceError::Panicked)), "the kept outcome, not a clean end");

    let h = SourceHandle::new(SourceId::new("b"), stop_flag(), thread::spawn(bomb));
    assert!(matches!(h.join(), Err(SourceError::Panicked)));
}

#[test]
fn a_worker_that_ended_reads_finished_before_any_poll() {
    let h = ending_with(Ok(()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.is_finished() {
        assert!(Instant::now() < deadline, "an ended, unpolled worker never read finished");
        thread::sleep(Duration::from_millis(2));
    }
    // A live worker does not.
    let live = until_stopped("live");
    assert!(!live.is_finished());
    assert!(live.join().is_ok());
    assert!(h.join().is_ok());
}

#[test]
fn dropping_the_handle_raises_the_flag_without_waiting() {
    let stop = stop_flag();
    let flag = Arc::clone(&stop);
    let seen = Arc::new(AtomicBool::new(false));
    let seen_by_worker = Arc::clone(&seen);
    let (done_tx, done_rx) = mpsc::channel();
    let join = thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(1));
        }
        // Linger after the stop, so a drop that waited for the thread would be measurable.
        thread::sleep(Duration::from_millis(300));
        seen_by_worker.store(true, Ordering::Relaxed);
        let _ = done_tx.send(());
        Ok(())
    });
    let h = SourceHandle::new(SourceId::new("d"), Arc::clone(&stop), join);
    let started = Instant::now();
    drop(h);
    assert!(started.elapsed() < Duration::from_millis(200), "drop blocked on the worker");
    assert!(stop.load(Ordering::Relaxed), "drop raises the stop flag");
    done_rx.recv_timeout(Duration::from_secs(5)).expect("the worker saw the flag and ended");
    assert!(seen.load(Ordering::Relaxed));
}

#[test]
fn source_errors_read_as_the_operator_sees_them() {
    let shown: Vec<String> = [
        SourceError::Spec("no files match".into()),
        SourceError::Backfill("offset past the end".into()),
        SourceError::NotConfigured("plc".into()),
        SourceError::Panicked,
    ]
    .iter()
    .map(|e| e.to_string())
    .collect();
    assert_eq!(
        shown,
        [
            "family spec: no files match",
            "backfill: offset past the end",
            "source not configured: plc",
            "source worker thread panicked",
        ]
    );
    let unclosed = String::from("(");
    let bad = regex::Regex::new(&unclosed).unwrap_err();
    let e: SourceError = bad.clone().into();
    assert_eq!(e.to_string(), format!("invalid pattern: {bad}"));
    // The I/O error is the error's source, for a caller walking the chain.
    let io = share_unreachable();
    let source = std::error::Error::source(&io).expect("an io error has a source");
    assert_eq!(source.to_string(), "share unreachable");
}
