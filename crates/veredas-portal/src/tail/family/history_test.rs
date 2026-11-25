//! A fixed-name family's history read when a rotation renames its fixed active between the
//! sweep's listing and its open (contract: C-096, its third refute round). In the crate: a hook
//! before the open holds that window, which no test outside the crate can reach.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;

use super::{BEFORE_FIXED_OPEN, FamilyStart, FamilyTailConfig, FamilyTailSource};
use crate::family::{ActiveMode, FamilySpec};
use crate::source::Source;
use crate::timestamp::TimestampSpec;

fn write(path: &std::path::Path, range: std::ops::Range<u32>) {
    let body: String = range.map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
    std::fs::write(path, body).unwrap();
}

/// The fixed active, 40 000 lines of history, renamed into a retiree after the sweep listed the
/// family and before it opened the fixed path (the writer not yet making the new active): the
/// open finds nothing there, and the history is said read only once the renamed active is read,
/// with every one of the 60 000 lines sent, never at the 20 000 of the retiree swept before it.
#[test]
fn a_fixed_active_renamed_before_the_sweep_s_open_is_read_before_the_history_is_said_read() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("done_1.log"), 0..20_000);
    let fixed = dir.path().join("current.log");
    write(&fixed, 20_000..60_000);
    let (from, to) = (fixed.clone(), dir.path().join("done_2.log"));
    let mut renamed = false;
    BEFORE_FIXED_OPEN.lock().unwrap().push((
        fixed.clone(),
        Box::new(move || {
            if !renamed {
                renamed = true;
                std::fs::rename(&from, &to).unwrap();
            }
        }),
    ));
    let spec = FamilySpec::new(dir.path(), r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let source = FamilyTailSource::new("fixed", "fixed", spec, FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() })
        .with_timestamps(stamps)
        .with_history_read(Arc::clone(&history));
    let (tx, rx) = channel();
    let handle = Box::new(source).start(tx).unwrap();
    let started = Instant::now();
    let mut lines = 0u64;
    while history.load(Ordering::SeqCst) == u64::MAX {
        lines += rx.try_iter().count() as u64;
        assert!(started.elapsed() < Duration::from_secs(30), "never said read: {lines} lines sent");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(history.load(Ordering::SeqCst), 60_000, "said read when that many lines were sent");
    handle.request_stop();
    handle.join().unwrap();
    lines += rx.try_iter().count() as u64;
    assert_eq!(lines, 60_000, "every line read once");
    BEFORE_FIXED_OPEN.lock().unwrap().retain(|(at, _)| *at != fixed);
}
