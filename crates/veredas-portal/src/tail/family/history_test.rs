//! A family's history read when a rotation or a stop lands between a listing and an open
//! (contract: C-096, its third and fourth refute rounds). In the crate: a hook before the open
//! holds that window, which no test outside the crate can reach.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;

use super::{BEFORE_OPEN, FamilyStart, FamilyTailConfig, FamilyTailSource};
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
    BEFORE_OPEN.lock().unwrap().push((
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
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| *at != fixed);
}

/// A stop that lands while the backfill waits on its planned active (there, nobody may read it)
/// leaves the history unread: never said read, the 20 000 lines of the member before it sent and
/// the active's 10 not (C-096's fourth refute round). The hook holds the replay's open of the
/// active until the stop is asked.
#[cfg(unix)]
#[test]
fn a_stop_during_the_planned_active_s_read_never_says_the_history_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("plant_1.log"), 0..20_000);
    let planned = dir.path().join("plant_2.log");
    write(&planned, 20_000..20_010);
    let (asked_tx, asked_rx) = channel::<()>();
    let (stopped_tx, stopped_rx) = channel::<()>();
    let mut held = false;
    BEFORE_OPEN.lock().unwrap().push((
        planned.clone(),
        Box::new(move || {
            if !held {
                held = true;
                let _ = asked_tx.send(());
                let _ = stopped_rx.recv_timeout(Duration::from_secs(30));
            }
        }),
    ));
    let spec = FamilySpec::new(dir.path(), r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let source = FamilyTailSource::new("plant", "plant", spec, FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() })
        .with_timestamps(stamps)
        .with_history_read(Arc::clone(&history));
    let (tx, rx) = channel();
    let handle = Box::new(source).start(tx).unwrap();
    // Located (the start has returned): the active shut before the replay reaches it.
    std::fs::set_permissions(&planned, std::fs::Permissions::from_mode(0o000)).unwrap();
    asked_rx.recv_timeout(Duration::from_secs(30)).expect("the replay never reached its active");
    handle.request_stop();
    stopped_tx.send(()).unwrap();
    handle.join().unwrap();
    assert_eq!(rx.try_iter().count(), 20_000, "the member before the active read whole");
    assert_eq!(history.load(Ordering::SeqCst), u64::MAX, "never said read");
    std::fs::set_permissions(&planned, std::fs::Permissions::from_mode(0o644)).unwrap();
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| *at != planned);
}

/// A fixed name's held active renamed into a retiree, another file made at the fixed path, between
/// the round's listing and the held open (the hook holds that window): the open lands on the new
/// file, and the held one's lines since the look are read first, awaited while it is held, then
/// the new file's, in order, nothing of the history; the new file rotating while the sweep awaits
/// is read once, by its cursor, and the next active after it (C-098).
#[cfg(unix)]
#[test]
fn a_held_fixed_active_renamed_before_its_open_is_read_first() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicBool;
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("done_1.log"), 0..10);
    let fixed = dir.path().join("current.log");
    write(&fixed, 10..20);
    std::fs::set_permissions(&fixed, std::fs::Permissions::from_mode(0o200)).unwrap();
    let armed = Arc::new(AtomicBool::new(false));
    let (from, to, again, arm) = (fixed.clone(), dir.path().join("done_2.log"), fixed.clone(), Arc::clone(&armed));
    BEFORE_OPEN.lock().unwrap().push((
        fixed.clone(),
        Box::new(move || {
            if arm.swap(false, Ordering::SeqCst) {
                std::fs::rename(&from, &to).unwrap();
                write(&again, 50..60);
            }
        }),
    ));
    let spec = FamilySpec::new(dir.path(), r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let source = FamilyTailSource::new("fixed", "fixed", spec, FamilyTailConfig { start: FamilyStart::ActiveEnd, ..FamilyTailConfig::default() });
    let (tx, rx) = channel();
    let handle = Box::new(source).start(tx).unwrap();
    // The held active takes 30 lines after the look; the next round's open is held open.
    {
        use std::io::Write;
        let body: String = (20..50u32).map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
        std::fs::OpenOptions::new().append(true).open(&fixed).unwrap().write_all(body.as_bytes()).unwrap();
    }
    armed.store(true, Ordering::SeqCst);
    let started = Instant::now();
    while armed.load(Ordering::SeqCst) {
        assert!(started.elapsed() < Duration::from_secs(10), "the held open never came");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(rx.try_iter().count(), 0, "nothing read while the held file is held");
    // While the sweep awaits it, the file the open landed on rotates in turn: the sweep ends at it
    // (the cursor reads it, once), and the next active follows.
    std::fs::rename(&fixed, dir.path().join("done_3.log")).unwrap();
    write(&fixed, 60..65);
    std::thread::sleep(Duration::from_millis(500));
    std::fs::set_permissions(dir.path().join("done_2.log"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut temps: Vec<u32> = Vec::new();
    let started = Instant::now();
    while temps.len() < 45 && started.elapsed() < Duration::from_secs(10) {
        for e in rx.try_iter() {
            let t = e.line.split("temp=").nth(1).and_then(|r| r.split(' ').next()).and_then(|t| t.parse().ok()).unwrap();
            temps.push(t);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    for e in rx.try_iter() {
        temps.push(e.line.split("temp=").nth(1).and_then(|r| r.split(' ').next()).and_then(|t| t.parse().ok()).unwrap());
    }
    assert_eq!(temps, (20..65).collect::<Vec<u32>>(), "the held file's 30 lines, the new file's 10, the next's 5, each once");
    handle.request_stop();
    handle.join().unwrap();
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| *at != fixed);
}
