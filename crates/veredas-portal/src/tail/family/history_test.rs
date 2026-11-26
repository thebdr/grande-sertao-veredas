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

/// The backfill's open of the fixed path followed by a listing that runs out of patience (the
/// folder denying its listing for two seconds, a share's hiccup) cannot tell whether a rotation
/// renamed the active just before the open: the live read goes on, but the history is never said
/// read, never at the 20 010 lines sent while the renamed active's 40 000 are unread (C-096's
/// ninth refute round).
#[cfg(unix)]
#[test]
fn a_listing_out_of_patience_after_the_fixed_open_never_says_the_history_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("fam");
    std::fs::create_dir_all(&folder).unwrap();
    write(&folder.join("done_1.log"), 0..20_000);
    let fixed = folder.join("current.log");
    write(&fixed, 20_000..60_000);
    let (from, to, again, shut) = (fixed.clone(), folder.join("done_2.log"), fixed.clone(), folder.clone());
    let mut done = false;
    BEFORE_OPEN.lock().unwrap().push((
        fixed.clone(),
        Box::new(move || {
            if !done {
                done = true;
                std::fs::rename(&from, &to).unwrap();
                write(&again, 60_000..60_010);
                std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o300)).unwrap();
                let shut = shut.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();
                });
            }
        }),
    ));
    let spec = FamilySpec::new(&folder, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
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
    while started.elapsed() < Duration::from_secs(6) {
        lines += rx.try_iter().count() as u64;
        let mark = history.load(Ordering::SeqCst);
        assert!(mark == u64::MAX || mark == crate::tail::HISTORY_LOST, "said read at {mark} of the 60 000 lines of history ({lines} sent)");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(history.load(Ordering::SeqCst), crate::tail::HISTORY_LOST, "the history cannot be shown whole");
    handle.request_stop();
    handle.join().unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| *at != fixed);
}

/// Runs `source` until it has sent `lines` and a moment more, then stops it: what it sent, and
/// its history's mark.
fn run_cut(source: Box<dyn Source>, history: &Arc<AtomicU64>, lines: u64) -> (u64, u64) {
    let (tx, rx) = channel();
    let handle = source.start(tx).unwrap();
    let started = Instant::now();
    let mut sent = 0u64;
    while sent < lines && started.elapsed() < Duration::from_secs(20) {
        sent += rx.try_iter().count() as u64;
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    handle.request_stop();
    handle.join().unwrap();
    sent += rx.try_iter().count() as u64;
    (sent, history.load(Ordering::SeqCst))
}

/// A hook cutting `path` in place once (a copy-and-truncate rotation: the same file, emptied) and
/// writing ten new lines into it.
fn cut_once(path: &std::path::Path) {
    let (at, mut done) = (path.to_path_buf(), false);
    BEFORE_OPEN.lock().unwrap().push((
        path.to_path_buf(),
        Box::new(move || {
            if !done {
                done = true;
                write(&at, 90_000..90_010);
            }
        }),
    ));
}

/// A history cut in place before it is read is never said read (C-096's tenth refute round): a
/// file read from a start in the past, cut before its first read; a fixed-name family's active
/// and a newest-by-order family's, cut before the backfill's seam opens them; a member cut before
/// the replay opens it. Each reads the ten lines written after the cut, and the history is lost:
/// the read never reached where the start found the file's end.
#[test]
fn a_history_cut_in_place_before_it_is_read_is_lost() {
    use crate::tail::{FileStart, FileTailSource, TailConfig};
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let backfill = || FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() };
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();

    // A file read from its start, cut before its first read.
    let file = dir.path().join("one.log");
    write(&file, 0..50_000);
    cut_once(&file);
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let path = file.to_string_lossy().into_owned();
    let source = FileTailSource::new("one", "one", &path, TailConfig { start: FileStart::Begin, ..TailConfig::default() }).with_history_read(Arc::clone(&history));
    assert_eq!(run_cut(Box::new(source), &history, 10), (10, lost), "the file");

    // A fixed-name family's active, cut before the seam opens it.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write(&fixed.join("done_1.log"), 0..100);
    write(&fixed.join("current.log"), 100..50_000);
    cut_once(&fixed.join("current.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let spec = FamilySpec::new(&fixed, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let source = FamilyTailSource::new("fixed", "fixed", spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
    assert_eq!(run_cut(Box::new(source), &history, 110), (110, lost), "the fixed family's active");

    // A newest-by-order family's active, cut before the seam opens it; and a member before its
    // replay.
    for (name, cut) in [("newest", "plant_2.log"), ("member", "plant_1.log")] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(&fam).unwrap();
        let (first, second) = if cut == "plant_2.log" { (0..100, 100..50_000) } else { (0..50_000, 50_000..50_100) };
        write(&fam.join("plant_1.log"), first);
        write(&fam.join("plant_2.log"), second);
        cut_once(&fam.join(cut));
        let history = Arc::new(AtomicU64::new(u64::MAX));
        let spec = FamilySpec::new(&fam, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
        let source = FamilyTailSource::new(name, name, spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
        assert_eq!(run_cut(Box::new(source), &history, 110), (110, lost), "the {name}");
    }
    let cut: Vec<std::path::PathBuf> = [file, fixed.join("current.log"), dir.path().join("newest/plant_2.log"), dir.path().join("member/plant_1.log")].into();
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !cut.contains(at));
}

/// A hook cutting `path` in place once as a copy-and-truncate rotation does under a writer that
/// does not open with `O_APPEND`: the file emptied, the writer's next ten lines written at its own
/// offset, the old end, so the file is at once longer than before, a hole of NULs at its top.
fn cut_sparse_once(path: &std::path::Path) {
    let (at, mut done) = (path.to_path_buf(), false);
    BEFORE_OPEN.lock().unwrap().push((
        path.to_path_buf(),
        Box::new(move || {
            if !done {
                use std::io::{Seek, SeekFrom, Write};
                done = true;
                let mut f = std::fs::OpenOptions::new().write(true).open(&at).unwrap();
                let end = f.metadata().unwrap().len();
                f.set_len(0).unwrap();
                f.seek(SeekFrom::Start(end)).unwrap();
                let body: String = (90_000..90_010u32).map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
                f.write_all(body.as_bytes()).unwrap();
            }
        }),
    ));
}

/// A history cut in place and written past its old end before it is read, a writer without
/// `O_APPEND` (C-096's eleventh refute round): by its length a file that grew, by its first bytes
/// another file, so the history is lost, never said read: a file read from its start, a fixed-name
/// family's active at the seam, a member before its replay. The NULs of the hole read as one line.
#[cfg(unix)]
#[test]
fn a_history_cut_and_written_past_its_old_end_is_lost() {
    use crate::tail::{FileStart, FileTailSource, TailConfig};
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let backfill = || FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() };
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let mark = |(_, m): (u64, u64)| m;

    let file = dir.path().join("sparse.log");
    write(&file, 0..50_000);
    cut_sparse_once(&file);
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let path = file.to_string_lossy().into_owned();
    let source = FileTailSource::new("one", "one", &path, TailConfig { start: FileStart::Begin, ..TailConfig::default() }).with_history_read(Arc::clone(&history));
    assert_eq!(mark(run_cut(Box::new(source), &history, 10)), lost, "the file");

    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write(&fixed.join("done_1.log"), 0..100);
    write(&fixed.join("current.log"), 100..50_000);
    cut_sparse_once(&fixed.join("current.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let spec = FamilySpec::new(&fixed, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let source = FamilyTailSource::new("fixed", "fixed", spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
    assert_eq!(mark(run_cut(Box::new(source), &history, 110)), lost, "the fixed family's active");

    let fam = dir.path().join("member");
    std::fs::create_dir_all(&fam).unwrap();
    write(&fam.join("plant_1.log"), 0..50_000);
    write(&fam.join("plant_2.log"), 50_000..50_100);
    cut_sparse_once(&fam.join("plant_1.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let spec = FamilySpec::new(&fam, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let source = FamilyTailSource::new("member", "member", spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
    assert_eq!(mark(run_cut(Box::new(source), &history, 110)), lost, "the member");

    let cut: Vec<std::path::PathBuf> = [file, fixed.join("current.log"), fam.join("plant_1.log")].into();
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !cut.contains(at));
}

/// A plant log's constant first line, longer than the reach's 64 bytes of head.
const HEADER: &str = "Datum;Uhrzeit;Temperatur_Reaktor_1;Druck_Reaktor_1;Durchfluss_Kuehlwasser_1;Status\n";

fn write_with_header(path: &std::path::Path, range: std::ops::Range<u32>) {
    let body: String = range.map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
    std::fs::write(path, format!("{HEADER}{body}")).unwrap();
}

/// A hook running `act` on `path` once.
fn once(path: &std::path::Path, act: impl FnOnce(&std::path::Path) + Send + 'static) {
    let at = path.to_path_buf();
    let mut act = Some(act);
    BEFORE_OPEN.lock().unwrap().push((
        path.to_path_buf(),
        Box::new(move || {
            if let Some(a) = act.take() {
                a(&at);
            }
        }),
    ));
}

/// C-096's twelfth refute round. A log whose first line is a constant header longer than the
/// reach's head, cut in place and written again from its top past its old end (a writer opening
/// afresh), has the same head but other bytes at its old end: lost, a file before its first read
/// and a fixed-name family's active before the seam. A planned active deleted and made again under
/// its name before the seam is another file: nothing of the history is left in it, said read at the
/// 100 lines before it, a fixed-name family's and a newest-by-order family's alike.
#[test]
fn a_history_rewritten_under_its_header_is_lost_and_one_deleted_is_read() {
    use crate::tail::{FileStart, FileTailSource, TailConfig};
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let backfill = || FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() };
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // Rewritten under the same header, a file before its first read.
    let file = dir.path().join("csv.log");
    write_with_header(&file, 0..1_000);
    once(&file, |p| write_with_header(p, 90_000..92_000));
    hooked.push(file.clone());
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let path = file.to_string_lossy().into_owned();
    let source = FileTailSource::new("csv", "csv", &path, TailConfig { start: FileStart::Begin, ..TailConfig::default() }).with_history_read(Arc::clone(&history));
    assert_eq!(run_cut(Box::new(source), &history, 2_001), (2_001, lost), "the file");

    // The same, a fixed-name family's active before the seam.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write_with_header(&fixed.join("done_1.log"), 0..100);
    write_with_header(&fixed.join("current.log"), 100..1_000);
    once(&fixed.join("current.log"), |p| write_with_header(p, 90_000..92_000));
    hooked.push(fixed.join("current.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let spec = FamilySpec::new(&fixed, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let source = FamilyTailSource::new("fixed", "fixed", spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
    assert_eq!(run_cut(Box::new(source), &history, 2_102), (2_102, lost), "the fixed family's active");

    // Deleted and made again before the seam: read at the 100 lines before it.
    for (name, mode, member, active) in [
        ("gone-fixed", ActiveMode::Fixed("current.log".into()), "done_1.log", "current.log"),
        ("gone-newest", ActiveMode::NewestByOrder, "done_1.log", "done_2.log"),
    ] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(&fam).unwrap();
        write(&fam.join(member), 0..100);
        write(&fam.join(active), 100..1_000);
        once(&fam.join(active), |p| {
            std::fs::remove_file(p).unwrap();
            write(p, 90_000..90_010);
        });
        hooked.push(fam.join(active));
        let history = Arc::new(AtomicU64::new(u64::MAX));
        let spec = FamilySpec::new(&fam, r"done_(?P<idx>\d+)\.log", None, mode).unwrap();
        let source = FamilyTailSource::new(name, name, spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(&history));
        assert_eq!(run_cut(Box::new(source), &history, 110), (110, 100), "{name}");
    }
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));
}

/// Cuts `path` as a copy-and-truncate under a writer without `O_APPEND` does: emptied, ten lines
/// written at the old end, a hole of NULs before them.
fn cut_sparse(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    let end = f.metadata().unwrap().len();
    f.set_len(0).unwrap();
    f.seek(SeekFrom::Start(end)).unwrap();
    let body: String = (90_000..90_010u32).map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
    f.write_all(body.as_bytes()).unwrap();
}

/// C-096's thirteenth refute round. A cut landing inside a file's look (between its length and
/// its bytes) leaves the look a file that shrank under it or a hole at an end: lost, never said
/// read, a sparse regrowth and a shorter rewrite alike, the shorter one grown past the old end
/// before the first read too. A file holding a hole of NULs when the
/// start looks (copy-truncated before it, its history gone with the copy) is lost too, a file read
/// from its start and a fixed-name family's active at the locate alike.
#[cfg(unix)]
#[test]
fn a_cut_inside_the_look_and_a_hole_at_the_look_are_lost() {
    use super::DURING_LOOK;
    use crate::tail::{FileStart, FileTailSource, TailConfig};
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let file_source = |path: &std::path::Path, history: &Arc<AtomicU64>| {
        FileTailSource::new("one", "one", path, TailConfig { start: FileStart::Begin, ..TailConfig::default() }).with_history_read(Arc::clone(history))
    };
    let mut looked: Vec<std::path::PathBuf> = Vec::new();

    // Inside the look: a sparse regrowth, and a shorter rewrite.
    for (name, sparse) in [("sparse.log", true), ("shorter.log", false)] {
        let file = dir.path().join(name);
        write(&file, 0..50_000);
        let (at, mut done) = (file.clone(), false);
        DURING_LOOK.lock().unwrap().push((
            file.clone(),
            Box::new(move || {
                if !done {
                    done = true;
                    if sparse { cut_sparse(&at) } else { write(&at, 90_000..90_010) }
                }
            }),
        ));
        looked.push(file.clone());
        let history = Arc::new(AtomicU64::new(u64::MAX));
        assert_eq!(run_cut(Box::new(file_source(&file, &history)), &history, 10).1, lost, "{name}, cut inside the look");
    }

    // Cut shorter inside the look, then grown past the old end before the first read: the look
    // saw it shrink, whatever it grew to after.
    let regrown = dir.path().join("regrown.log");
    write(&regrown, 0..50_000);
    let (at, mut done) = (regrown.clone(), false);
    DURING_LOOK.lock().unwrap().push((
        regrown.clone(),
        Box::new(move || {
            if !done {
                done = true;
                write(&at, 90_000..90_010);
            }
        }),
    ));
    looked.push(regrown.clone());
    once(&regrown, |p| {
        use std::io::Write;
        let body: String = (100_000..160_000u32).map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + (i % 50_000) / 3600, (i / 60) % 60, i % 60)).collect();
        std::fs::OpenOptions::new().append(true).open(p).unwrap().write_all(body.as_bytes()).unwrap();
    });
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(Box::new(file_source(&regrown, &history)), &history, 60_010).1, lost, "cut shorter inside the look, then grown");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| *at != regrown);

    // A hole already there when the start looks: a file, and a fixed-name family's active.
    let holed = dir.path().join("holed.log");
    write(&holed, 0..50_000);
    cut_sparse(&holed);
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(Box::new(file_source(&holed, &history)), &history, 10).1, lost, "a file with a hole at its look");
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(&fixed).unwrap();
    write(&fixed.join("done_1.log"), 0..100);
    write(&fixed.join("current.log"), 100..50_000);
    cut_sparse(&fixed.join("current.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let spec = FamilySpec::new(&fixed, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let source = FamilyTailSource::new("fixed", "fixed", spec, FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() })
        .with_timestamps(stamps())
        .with_history_read(Arc::clone(&history));
    assert_eq!(run_cut(Box::new(source), &history, 110).1, lost, "a fixed family's active with a hole at the locate");
    DURING_LOOK.lock().unwrap().retain(|(at, _)| !looked.contains(at));
}

/// C-096's fourteenth refute round. A file's look comes before its start returns: a file read
/// from its start, cut and written again with ten new lines right after the start returned, is
/// lost, its reach the one the look took before. A held file's look brackets its length with the
/// identity at the path before the open: a rotation landing inside the held look (the held file
/// renamed away, another made at its path) leaves no identity kept, and when the new file opens the
/// held one's history is lost, never said read at the new file's ten lines.
#[cfg(unix)]
#[test]
fn a_cut_after_the_start_and_a_rotation_inside_the_held_look_are_lost() {
    use super::{AFTER_START, DURING_LOOK};
    use crate::tail::{FileStart, FileTailSource, TailConfig};
    use std::os::unix::fs::PermissionsExt;
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let file_source = |path: &std::path::Path, history: &Arc<AtomicU64>| {
        FileTailSource::new("one", "one", path, TailConfig { start: FileStart::Begin, ..TailConfig::default() }).with_history_read(Arc::clone(history))
    };

    // Cut right after the start returned.
    let file = dir.path().join("after.log");
    write(&file, 0..50_000);
    let (at, mut done) = (file.clone(), false);
    AFTER_START.lock().unwrap().push((
        file.clone(),
        Box::new(move || {
            if !done {
                done = true;
                write(&at, 90_000..90_010);
            }
        }),
    ));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(Box::new(file_source(&file, &history)), &history, 10), (10, lost), "cut right after the start");
    AFTER_START.lock().unwrap().retain(|(p, _)| *p != file);

    // A rotation inside the held look.
    let held = dir.path().join("held.log");
    write(&held, 0..5_000);
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (at, away, mut done) = (held.clone(), dir.path().join("held.log.1"), false);
    DURING_LOOK.lock().unwrap().push((
        held.clone(),
        Box::new(move || {
            if !done {
                done = true;
                std::fs::rename(&at, &away).unwrap();
                write(&at, 90_000..90_010);
            }
        }),
    ));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(Box::new(file_source(&held, &history)), &history, 10), (10, lost), "a rotation inside the held look");
    DURING_LOOK.lock().unwrap().retain(|(p, _)| *p != held);
    std::fs::set_permissions(dir.path().join("held.log.1"), std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// C-096's fifteenth refute round. The planned active renamed in two steps, through a name
/// outside the pattern (`current.log.rotating`, then `done_2.log` 600 ms later), is looked for by
/// its identity and waited for: its 1 000 lines read before the history is said, at 1 100. Renamed
/// outside the pattern for good, with another file made under its name (fixed and newest by
/// order) or none (newest by order, vanished), its history is unread: lost, never said read.
#[cfg(unix)]
#[test]
fn a_planned_active_renamed_outside_the_pattern_is_waited_for_or_lost() {
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let backfill = || FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() };
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();
    let family = |fam: &std::path::Path, mode: ActiveMode, name: &str, history: &Arc<AtomicU64>| -> Box<dyn Source> {
        let spec = FamilySpec::new(fam, r"done_(?P<idx>\d+)\.log", None, mode).unwrap();
        Box::new(FamilyTailSource::new(name, name, spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(history)))
    };

    // In two steps, the second into the pattern: waited for, read, said at 1 100.
    let two = dir.path().join("two-step");
    std::fs::create_dir_all(&two).unwrap();
    write(&two.join("done_1.log"), 0..100);
    write(&two.join("current.log"), 100..1_100);
    once(&two.join("current.log"), |p| {
        let (rotating, retired) = (p.with_extension("log.rotating"), p.with_file_name("done_2.log"));
        std::fs::rename(p, &rotating).unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            std::fs::rename(&rotating, &retired).unwrap();
        });
    });
    hooked.push(two.join("current.log"));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let source = family(&two, ActiveMode::Fixed("current.log".into()), "two", &history);
    assert_eq!(run_cut(source, &history, 1_100), (1_100, 1_100), "renamed in two steps");

    // Renamed outside the pattern for good, another file made under its name: lost.
    for (name, mode, active) in [("out-fixed", ActiveMode::Fixed("current.log".into()), "current.log"), ("out-newest", ActiveMode::NewestByOrder, "done_2.log")] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(&fam).unwrap();
        write(&fam.join("done_1.log"), 0..100);
        write(&fam.join(active), 100..1_000);
        once(&fam.join(active), |p| {
            std::fs::rename(p, p.with_extension("log.old")).unwrap();
            write(p, 90_000..90_010);
        });
        hooked.push(fam.join(active));
        let history = Arc::new(AtomicU64::new(u64::MAX));
        assert_eq!(run_cut(family(&fam, mode, name, &history), &history, 110), (110, lost), "{name}");
    }

    // Newest by order, renamed outside the pattern with nothing in its place: lost.
    let gone = dir.path().join("out-vanished");
    std::fs::create_dir_all(&gone).unwrap();
    write(&gone.join("done_1.log"), 0..100);
    write(&gone.join("done_2.log"), 100..1_000);
    once(&gone.join("done_2.log"), |p| std::fs::rename(p, p.with_extension("log.old")).unwrap());
    hooked.push(gone.join("done_2.log"));
    // The vanished active is awaited for two patiences before the search: wait for the mark.
    let history = Arc::new(AtomicU64::new(u64::MAX));
    let (tx, rx) = channel();
    let handle = family(&gone, ActiveMode::NewestByOrder, "vanished", &history).start(tx).unwrap();
    let started = Instant::now();
    while history.load(Ordering::SeqCst) == u64::MAX && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    handle.request_stop();
    handle.join().unwrap();
    assert_eq!((rx.try_iter().count(), history.load(Ordering::SeqCst)), (100, lost), "renamed out, nothing in its place");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));
}

/// C-096's sixteenth refute round. A member holding history renamed outside the pattern before
/// the replay opens it is looked for by its identity: there under another name, its history
/// unread, lost (a fixed-name family's member, a newest-by-order family's, and a planned active's
/// remains after it rotated into the pattern at the fixed seam). And the search reads each entry's
/// identity from the listing itself: a rename landing between the listing and the entries' look
/// (the second step of a two-step rename) still finds the file, waited for and read, said at 1 100.
#[cfg(unix)]
#[test]
fn a_member_renamed_outside_the_pattern_is_lost_and_the_search_holds_under_a_rename() {
    use super::DURING_WALK;
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let backfill = || FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() };
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();
    let family = |fam: &std::path::Path, mode: ActiveMode, name: &str, history: &Arc<AtomicU64>| -> Box<dyn Source> {
        let spec = FamilySpec::new(fam, r"done_(?P<idx>\d+)\.log", None, mode).unwrap();
        Box::new(FamilyTailSource::new(name, name, spec, backfill()).with_timestamps(stamps()).with_history_read(Arc::clone(history)))
    };
    let out = |p: &std::path::Path| std::fs::rename(p, p.with_extension("log.old")).unwrap();

    // A member renamed out before its replay: fixed name, and newest by order.
    for (name, mode, active) in [("member-fixed", ActiveMode::Fixed("current.log".into()), "current.log"), ("member-newest", ActiveMode::NewestByOrder, "done_3.log")] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(&fam).unwrap();
        write(&fam.join("done_1.log"), 0..100);
        write(&fam.join("done_2.log"), 100..1_100);
        write(&fam.join(active), 1_100..1_200);
        once(&fam.join("done_2.log"), out);
        hooked.push(fam.join("done_2.log"));
        let history = Arc::new(AtomicU64::new(u64::MAX));
        assert_eq!(run_cut(family(&fam, mode, name, &history), &history, 200), (200, lost), "{name}");
    }

    // The planned active rotated into the pattern at the fixed seam, then renamed out before its
    // remains are read.
    let remains = dir.path().join("remains");
    std::fs::create_dir_all(&remains).unwrap();
    write(&remains.join("done_1.log"), 0..100);
    write(&remains.join("current.log"), 100..1_100);
    once(&remains.join("current.log"), |p| {
        std::fs::rename(p, p.with_file_name("done_2.log")).unwrap();
        write(p, 90_000..90_010);
    });
    once(&remains.join("done_2.log"), out);
    hooked.extend([remains.join("current.log"), remains.join("done_2.log")]);
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(family(&remains, ActiveMode::Fixed("current.log".into()), "remains", &history), &history, 110), (110, lost), "the remains renamed out");

    // The second step of a two-step rename landing inside the search's walk.
    let walked = dir.path().join("walked");
    std::fs::create_dir_all(&walked).unwrap();
    write(&walked.join("done_1.log"), 0..100);
    write(&walked.join("current.log"), 100..1_100);
    once(&walked.join("current.log"), |p| std::fs::rename(p, p.with_extension("log.rotating")).unwrap());
    hooked.push(walked.join("current.log"));
    let (rotating, retired, mut done) = (walked.join("current.log.rotating"), walked.join("done_2.log"), false);
    DURING_WALK.lock().unwrap().push((
        walked.clone(),
        Box::new(move || {
            if !done && rotating.exists() {
                done = true;
                std::fs::rename(&rotating, &retired).unwrap();
            }
        }),
    ));
    let history = Arc::new(AtomicU64::new(u64::MAX));
    assert_eq!(run_cut(family(&walked, ActiveMode::Fixed("current.log".into()), "walked", &history), &history, 1_100), (1_100, 1_100), "renamed inside the walk");
    DURING_WALK.lock().unwrap().retain(|(at, _)| *at != walked);
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));
}

/// C-096's seventeenth refute round. A recursive family whose subfolder cannot be listed when the
/// start locates its history (`b/`, no listing allowed, holding 1 000 lines of it) leaves the plan
/// without that subfolder's members: the history cannot be shown whole, lost, never said read at
/// the 200 lines the replay read, a fixed-name family and a newest-by-order one alike; the live
/// read goes on.
#[cfg(unix)]
#[test]
fn a_recursive_family_s_unlistable_subfolder_loses_the_history() {
    use std::os::unix::fs::PermissionsExt;
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let lost = crate::tail::HISTORY_LOST;
    let dir = tempfile::tempdir().unwrap();
    for (name, mode, active) in [("fixed", ActiveMode::Fixed("current.log".into()), "current.log"), ("newest", ActiveMode::NewestByOrder, "done_3.log")] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(fam.join("a")).unwrap();
        std::fs::create_dir_all(fam.join("b")).unwrap();
        write(&fam.join("a/done_1.log"), 0..100);
        write(&fam.join("b/done_2.log"), 100..1_100);
        write(&fam.join(active), 1_100..1_200);
        std::fs::set_permissions(fam.join("b"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let spec = FamilySpec::new(&fam, r"(?:[ab]/)?done_(?P<idx>\d+)\.log", None, mode).unwrap().with_recursive(true);
        let history = Arc::new(AtomicU64::new(u64::MAX));
        let source = FamilyTailSource::new(name, name, spec, FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() })
            .with_timestamps(stamps())
            .with_history_read(Arc::clone(&history));
        assert_eq!(run_cut(Box::new(source), &history, 200), (200, lost), "{name}");
        std::fs::set_permissions(fam.join("b"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// C-096's eighteenth refute round. An empty member keeps its identity from the locate's peek: one
/// deleted before the backfill reaches it is known gone, nothing of the history left in it, said
/// read at the lines before it, a replayed member, a newest-by-order planned active and a fixed
/// name's planned active alike, never lost for a search that cannot tell.
#[test]
fn an_empty_member_deleted_before_the_backfill_reaches_it_is_read() {
    let start = NaiveDateTime::parse_from_str("2026-07-19 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let stamps = || TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();
    for (name, mode, members, deleted, lines) in [
        ("mid", ActiveMode::NewestByOrder, vec![("done_1.log", 0..100), ("done_2.log", 0..0), ("done_3.log", 100..200)], "done_2.log", 200u64),
        ("newest", ActiveMode::NewestByOrder, vec![("done_1.log", 0..100), ("done_2.log", 0..0)], "done_2.log", 100),
        ("fixed", ActiveMode::Fixed("current.log".into()), vec![("done_1.log", 0..100), ("current.log", 0..0)], "current.log", 100),
    ] {
        let fam = dir.path().join(name);
        std::fs::create_dir_all(&fam).unwrap();
        for (file, range) in members {
            write(&fam.join(file), range);
        }
        once(&fam.join(deleted), |p| std::fs::remove_file(p).unwrap());
        hooked.push(fam.join(deleted));
        let spec = FamilySpec::new(&fam, r"done_(?P<idx>\d+)\.log", None, mode).unwrap();
        let history = Arc::new(AtomicU64::new(u64::MAX));
        let source = FamilyTailSource::new(name, name, spec, FamilyTailConfig { start: FamilyStart::Backfill(start), ..FamilyTailConfig::default() })
            .with_timestamps(stamps())
            .with_history_read(Arc::clone(&history));
        let (tx, rx) = channel();
        let handle = Box::new(source).start(tx).unwrap();
        let started = Instant::now();
        while history.load(Ordering::SeqCst) == u64::MAX && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.request_stop();
        handle.join().unwrap();
        assert_eq!((rx.try_iter().count() as u64, history.load(Ordering::SeqCst)), (lines, lines), "{name}");
    }
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));
}

/// Starts `spec` at its end, runs `act` once the start has returned, and collects the `temp=` of
/// every line sent until `n` have come (or ten seconds), then stops: the lines in order, and how
/// long the start took.
#[cfg(unix)]
fn at_end(spec: FamilySpec, n: usize, act: impl FnOnce()) -> (Vec<u32>, Duration) {
    let cfg = FamilyTailConfig { start: FamilyStart::ActiveEnd, use_notify: Some(false), ..FamilyTailConfig::default() };
    let (tx, rx) = channel();
    let begun = Instant::now();
    let handle = Box::new(FamilyTailSource::new("held", "held", spec, cfg)).start(tx).unwrap();
    let took = begun.elapsed();
    act();
    let mut temps: Vec<u32> = Vec::new();
    let started = Instant::now();
    while temps.len() < n && started.elapsed() < Duration::from_secs(10) {
        temps.extend(rx.try_iter().map(|e| e.line.split("temp=").nth(1).and_then(|r| r.split(' ').next()).and_then(|t| t.parse::<u32>().ok()).unwrap()));
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    handle.request_stop();
    handle.join().unwrap();
    temps.extend(rx.try_iter().map(|e| e.line.split("temp=").nth(1).and_then(|r| r.split(' ').next()).and_then(|t| t.parse::<u32>().ok()).unwrap()));
    (temps, took)
}

#[cfg(unix)]
fn append(path: &std::path::Path, range: std::ops::Range<u32>) {
    use std::io::Write;
    let body: String = range.map(|i| format!("2026-07-19 {:02}:{:02}:{:02} temp={i} state=run\n", 1 + i / 3600, (i / 60) % 60, i % 60)).collect();
    std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(body.as_bytes()).unwrap();
}

#[cfg(unix)]
fn mode(path: &std::path::Path, m: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(m)).unwrap();
}

/// A file made at `path` already held (written aside, write alone allowed, renamed in).
#[cfg(unix)]
fn make_held(path: &std::path::Path, range: std::ops::Range<u32>) {
    let aside = path.with_extension("stage");
    write(&aside, range);
    mode(&aside, 0o200);
    std::fs::rename(&aside, path).unwrap();
}

/// C-098's first refute round. A family started at its end whose active is held at the first
/// look reads what is written after the look in order, none of it lost and nothing of the
/// history: the held active retired with no active after it, then the next active held in turn
/// and retiring while held (its lines awaited, never dropped after a patience); a rotation inside
/// the held look (the look taken again, the renamed file history, the new one whole); a first open
/// that finds nothing then a held active (the look taken when it is there, the start returning at
/// once); two generations after a retired held active, in order; and a held active deleted with a
/// generation after it, that generation and the next active read whole.
#[cfg(unix)]
#[test]
fn a_held_active_at_its_end_is_followed_in_order_through_every_rotation() {
    use super::DURING_LOOK;
    let dir = tempfile::tempdir().unwrap();
    let fixed = |d: &std::path::Path| FamilySpec::new(d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let newest = |d: &std::path::Path| FamilySpec::new(d, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // Retired with no active after it, then the next active held in turn, retiring while held.
    let d = dir.path().join("chain");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    mode(&d.join("current.log"), 0o200);
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 45, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("done_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        mode(&dd.join("done_2.log"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        make_held(&p, 50..60);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(&p, dd.join("done_3.log")).unwrap();
        write(&p, 60..65);
        std::thread::sleep(Duration::from_millis(2000));
        mode(&dd.join("done_3.log"), 0o644);
    });
    assert_eq!(lines, (20..65).collect::<Vec<u32>>(), "the chain");

    // A rotation inside the held look.
    let d = dir.path().join("inside");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    mode(&d.join("current.log"), 0o200);
    let (p, retired, mut done) = (d.join("current.log"), d.join("done_2.log"), false);
    DURING_LOOK.lock().unwrap().push((
        p.clone(),
        Box::new(move || {
            if !done {
                done = true;
                std::fs::rename(&p, &retired).unwrap();
                make_held(&p, 0..0);
            }
        }),
    ));
    hooked.push(d.join("current.log"));
    let p = d.join("current.log");
    let (lines, _) = at_end(fixed(&d), 50, move || {
        append(&p, 50..100);
        mode(&p, 0o644);
    });
    assert_eq!(lines, (50..100).collect::<Vec<u32>>(), "a rotation inside the look");
    DURING_LOOK.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // A first open that finds nothing, then a held active.
    let d = dir.path().join("first");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    once(&d.join("current.log"), |p| {
        std::fs::rename(p, p.with_file_name("done_2.log")).unwrap();
        let p = p.to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            make_held(&p, 0..0);
        });
    });
    hooked.push(d.join("current.log"));
    let p = d.join("current.log");
    let (lines, took) = at_end(fixed(&d), 50, move || {
        while !p.exists() {
            std::thread::sleep(Duration::from_millis(5));
        }
        append(&p, 50..100);
        mode(&p, 0o644);
    });
    assert!(took < Duration::from_secs(3), "the start took {took:?}");
    assert_eq!(lines, (50..100).collect::<Vec<u32>>(), "not found, then held");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // Two generations after a retired held active, in order.
    let d = dir.path().join("order");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("plant_1.log"), 0..10);
    mode(&d.join("plant_1.log"), 0o200);
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 80, move || {
        append(&dd.join("plant_1.log"), 10..40);
        write(&dd.join("plant_2.log"), 40..60);
        write(&dd.join("plant_3.log"), 60..80);
        write(&dd.join("plant_4.log"), 80..90);
        std::thread::sleep(Duration::from_millis(1000));
        mode(&dd.join("plant_1.log"), 0o644);
    });
    assert_eq!(lines, (10..90).collect::<Vec<u32>>(), "two generations in order");

    // A held active deleted with a generation after it.
    let d = dir.path().join("deleted");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("plant_1.log"), 0..10);
    mode(&d.join("plant_1.log"), 0o200);
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 30, move || {
        append(&dd.join("plant_1.log"), 10..40);
        write(&dd.join("plant_2.log"), 40..60);
        write(&dd.join("plant_3.log"), 60..70);
        std::fs::remove_file(dd.join("plant_1.log")).unwrap();
    });
    assert_eq!(lines, (40..70).collect::<Vec<u32>>(), "deleted, a generation after it");
}

/// C-098's second refute round. The look's names are listed again after it, and a rotation
/// between the round's listing and the look (the active renamed into a retiree and a held file
/// made at its path; a new member made) makes it no look, taken again against the new listing: the
/// renamed file is then history, never read, the held one read from where it stood (50 to 100);
/// a newest-by-order family then has a readable newest active and is followed from its end (45 to
/// 50). A held active whose identity the look could not read (its folder listable, its files not
/// to be looked up) is found by its name once it retires under it, and read whole (0 to 50). And
/// on a filesystem that hands a deleted file's inode number to the next file (ext4), a new file
/// made at the held active's path after it is deleted is another file, its birth time apart, read
/// whole (50 to 80).
#[cfg(unix)]
#[test]
fn a_held_look_is_retaken_when_the_family_moves_and_a_reused_inode_is_another_file() {
    let dir = tempfile::tempdir().unwrap();
    let fixed = |d: &std::path::Path| FamilySpec::new(d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let newest = |d: &std::path::Path| FamilySpec::new(d, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // Fixed name: the active renamed into a retiree and a held file made at its path, between
    // the listing and the look.
    let d = dir.path().join("moved-fixed");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    once(&d.join("current.log"), |p| {
        std::fs::rename(p, p.with_file_name("done_2.log")).unwrap();
        make_held(p, 0..0);
    });
    hooked.push(d.join("current.log"));
    let p = d.join("current.log");
    let (lines, _) = at_end(fixed(&d), 50, move || {
        append(&p, 50..100);
        mode(&p, 0o644);
    });
    assert_eq!(lines, (50..100).collect::<Vec<u32>>(), "fixed name, moved before the look");

    // Newest by order: a new member made between the listing and the look.
    let d = dir.path().join("moved-newest");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("plant_1.log"), 0..10);
    mode(&d.join("plant_1.log"), 0o200);
    once(&d.join("plant_1.log"), |p| write(&p.with_file_name("plant_2.log"), 40..45));
    hooked.push(d.join("plant_1.log"));
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 5, move || {
        append(&dd.join("plant_2.log"), 45..50);
    });
    assert_eq!(lines, (45..50).collect::<Vec<u32>>(), "newest by order, moved before the look");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // No identity at the look: found by its name once it retires under it, read whole.
    let d = dir.path().join("unknown");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("plant_1.log"), 0..10);
    mode(&d.join("plant_1.log"), 0o200);
    mode(&d, 0o600);
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 50, move || {
        mode(&dd, 0o700);
        append(&dd.join("plant_1.log"), 10..40);
        write(&dd.join("plant_2.log"), 40..50);
        mode(&dd.join("plant_1.log"), 0o644);
    });
    assert_eq!(lines, (0..50).collect::<Vec<u32>>(), "no identity at the look");

    // A reused inode number, on a filesystem that reuses them: the build's folder, not /tmp.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/history-reuse");
    std::fs::create_dir_all(&base).unwrap();
    let reuse = tempfile::tempdir_in(&base).unwrap();
    let d = reuse.path().to_path_buf();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    mode(&d.join("current.log"), 0o200);
    let p = d.join("current.log");
    let (lines, _) = at_end(fixed(&d), 30, move || {
        std::fs::remove_file(&p).unwrap();
        let aside = p.with_extension("stage");
        write(&aside, 50..80);
        std::fs::rename(&aside, &p).unwrap();
    });
    assert_eq!(lines, (50..80).collect::<Vec<u32>>(), "a new file at the held path, its inode number reused");
}

/// C-098's third refute round. A held active renamed in two steps, through a name outside the
/// pattern while a new active is made at its path, is found in the folder by its identity and read
/// from where it stood before the new active's first line (20 to 60), still held there or once in
/// the pattern, then never read again: read where it was and landing in the pattern before the
/// fixed path's open (the sweep), or while a generation after it is awaited (20 to 70), or found
/// and moved again before its read there, another file left at that name (20 to 60). An active
/// held in turn and renamed the same way is read whole (20 to 50). A held active copied and cut in
/// place (copy-and-truncate) has its copy read from where the held file stood, then itself from
/// its top (20 to 55): cut, it is shorter at its open than the most of it seen at its path (20 to
/// 70, written again past the look's length right before its open), or was seen shorter there (20 to 100, written again
/// past the most seen); a file dropped in beside a held active that is not cut is not replayed (20
/// to 50, never 900). And when the search cannot tell where the held file went (moved out of a
/// folder one of whose subfolders cannot be listed), its lines are lost and the new active is read,
/// never waited on (50 to 60).
#[cfg(unix)]
#[test]
fn a_held_active_renamed_outside_the_pattern_or_cut_in_place_is_read_once_in_order() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let fixed = |d: &std::path::Path| FamilySpec::new(d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let held_family = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("done_1.log"), 0..10);
        write(&d.join("current.log"), 10..20);
        mode(&d.join("current.log"), 0o200);
        d
    };
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // Two steps, still held through both.
    let d = held_family("two-steps");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 40, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.log.rotating")).unwrap();
        write(&p, 50..60);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(dd.join("current.log.rotating"), dd.join("done_2.log")).unwrap();
        mode(&dd.join("done_2.log"), 0o644);
    });
    assert_eq!(lines, (20..60).collect::<Vec<u32>>(), "two steps, held through both");

    // Read where it was, landing in the pattern between that read and the fixed path's open.
    let d = held_family("read-aside");
    let (aside, landed) = (d.join("current.log.rotating"), d.join("done_2.log"));
    BEFORE_OPEN.lock().unwrap().push((
        d.join("current.log"),
        Box::new(move || {
            if std::fs::File::open(&aside).is_ok() {
                std::fs::rename(&aside, &landed).unwrap();
            }
        }),
    ));
    hooked.push(d.join("current.log"));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 40, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.log.rotating")).unwrap();
        mode(&dd.join("current.log.rotating"), 0o644);
        write(&p, 50..60);
    });
    assert_eq!(lines, (20..60).collect::<Vec<u32>>(), "read aside, landed before the open");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // Found where it was and moved again before its read there (a hook), another file left at
    // that name: read once it is found again, the other file never.
    let d = held_family("moved-again");
    once(&d.join("current.log.rotating"), |p| {
        std::fs::rename(p, p.with_file_name("done_2.log")).unwrap();
        write(p, 900..905);
    });
    hooked.push(d.join("current.log.rotating"));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 40, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.log.rotating")).unwrap();
        mode(&dd.join("current.log.rotating"), 0o644);
        write(&p, 50..60);
    });
    assert_eq!(lines, (20..60).collect::<Vec<u32>>(), "moved again before its read");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // Read where it was, landing in the pattern while a generation after it is awaited.
    let d = held_family("read-aside-awaited");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 50, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.log.rotating")).unwrap();
        mode(&dd.join("current.log.rotating"), 0o644);
        make_held(&dd.join("done_3.log"), 50..60);
        write(&p, 60..70);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(dd.join("current.log.rotating"), dd.join("done_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        mode(&dd.join("done_3.log"), 0o644);
    });
    assert_eq!(lines, (20..70).collect::<Vec<u32>>(), "read aside, landed while a generation is awaited");

    // An active held in turn, renamed in two steps.
    let d = held_family("in-turn");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 30, move || {
        append(&p, 20..30);
        std::fs::rename(&p, dd.join("done_2.log")).unwrap();
        mode(&dd.join("done_2.log"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        make_held(&p, 30..40);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(&p, dd.join("current.log.rotating")).unwrap();
        write(&p, 40..50);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(dd.join("current.log.rotating"), dd.join("done_3.log")).unwrap();
        mode(&dd.join("done_3.log"), 0o644);
    });
    assert_eq!(lines, (20..50).collect::<Vec<u32>>(), "held in turn, two steps");

    // Copy-and-truncate.
    let d = held_family("copy-truncate");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 35, move || {
        append(&p, 20..50);
        write(&dd.join("done_2.log"), 10..50);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(0).unwrap();
        append(&p, 50..55);
        std::thread::sleep(Duration::from_millis(1000));
        mode(&p, 0o644);
    });
    assert_eq!(lines, (20..55).collect::<Vec<u32>>(), "copy-and-truncate");

    // Cut and written again past the look's length, short of the most of it seen, right before its
    // open (a hook): no round saw it short, its open does.
    let d = held_family("copy-truncate-regrown");
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hp, trigger) = (d.join("current.log"), Arc::clone(&armed));
    BEFORE_OPEN.lock().unwrap().push((
        d.join("current.log"),
        Box::new(move || {
            if trigger.swap(false, Ordering::SeqCst) {
                std::fs::OpenOptions::new().write(true).open(&hp).unwrap().set_len(0).unwrap();
                append(&hp, 50..70);
                mode(&hp, 0o644);
            }
        }),
    ));
    hooked.push(d.join("current.log"));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 50, move || {
        append(&p, 20..50);
        write(&dd.join("done_2.log"), 10..50);
        std::thread::sleep(Duration::from_millis(700));
        armed.store(true, Ordering::SeqCst);
    });
    assert_eq!(lines, (20..70).collect::<Vec<u32>>(), "copy-and-truncate, grown back short of the most seen");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // Cut, seen short, then written again past the most of it seen.
    let d = held_family("copy-truncate-seen");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 80, move || {
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(500));
        write(&dd.join("done_2.log"), 10..50);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(0).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        append(&p, 50..100);
        mode(&p, 0o644);
    });
    assert_eq!(lines, (20..100).collect::<Vec<u32>>(), "copy-and-truncate, seen cut, grown back past");

    // A file dropped in beside a held active that is not cut.
    let d = held_family("dropped-in");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 30, move || {
        write(&dd.join("done_2.log"), 900..905);
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(1000));
        mode(&p, 0o644);
    });
    assert_eq!(lines, (20..50).collect::<Vec<u32>>(), "dropped in, not cut");

    // The search cannot tell: the held file moved out of a recursive family's folder, one of whose
    // subfolders cannot be listed.
    let d = held_family("cannot-tell");
    std::fs::create_dir_all(d.join("locked")).unwrap();
    std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let spec = FamilySpec::new(&d, r"(?:locked/)?done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap().with_recursive(true);
    let (p, outside) = (d.join("current.log"), dir.path().join("cannot-tell-moved-out.log"));
    let (lines, _) = at_end(spec, 10, move || {
        append(&p, 20..50);
        std::fs::rename(&p, &outside).unwrap();
        write(&p, 50..60);
    });
    std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(lines, (50..60).collect::<Vec<u32>>(), "the search cannot tell");
}

/// C-098's fourth refute round. A newest-by-order family whose held newest is deleted (40 to 50)
/// or moved out of the pattern (20 to 60, and back into it once read, 20 to 30 then 40 to 50)
/// lists an older member, there at the look, as the newest: it is followed from its end, its lines
/// history. A copy-and-truncate made between a round's listing and the held open has its copy read
/// from where the held file stood (20 to 55). And a held file seen cut, then retired (20 to 55,
/// then the next active's 60 to 65), between rounds or between the listing and its open, has its
/// copy read from where it stood before its own lines, wherever it is when it opens; seen cut,
/// then deleted, its copy is read all the same (20 to 50, then 60 to 65).
#[cfg(unix)]
#[test]
fn a_held_newest_gone_or_a_cut_held_file_moved_reads_no_history() {
    let dir = tempfile::tempdir().unwrap();
    let fixed = |d: &std::path::Path| FamilySpec::new(d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let newest = |d: &std::path::Path| FamilySpec::new(d, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let held_newest = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("plant_1.log"), 0..10);
        write(&d.join("plant_2.log"), 10..20);
        mode(&d.join("plant_2.log"), 0o200);
        d
    };
    let held_fixed = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("done_1.log"), 0..10);
        write(&d.join("current.log"), 10..20);
        mode(&d.join("current.log"), 0o200);
        d
    };
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // The held newest deleted, an older member the newest until the next is born.
    let d = held_newest("newest-deleted");
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 10, move || {
        append(&dd.join("plant_2.log"), 20..30);
        std::fs::remove_file(dd.join("plant_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        write(&dd.join("plant_3.log"), 40..50);
    });
    assert_eq!(lines, (40..50).collect::<Vec<u32>>(), "the held newest deleted");

    // The held newest moved out of the pattern.
    let d = held_newest("newest-aside");
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 20, move || {
        append(&dd.join("plant_2.log"), 20..30);
        std::fs::rename(dd.join("plant_2.log"), dd.join("plant_2.log.old")).unwrap();
        mode(&dd.join("plant_2.log.old"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        write(&dd.join("plant_3.log"), 40..50);
    });
    assert_eq!(lines, (20..30).chain(40..50).collect::<Vec<u32>>(), "the held newest moved aside");

    // Moved out of the pattern, read there, and back into it, the newest again.
    let d = held_newest("newest-back");
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 20, move || {
        append(&dd.join("plant_2.log"), 20..30);
        std::fs::rename(dd.join("plant_2.log"), dd.join("plant_2.log.old")).unwrap();
        mode(&dd.join("plant_2.log.old"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(dd.join("plant_2.log.old"), dd.join("plant_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        write(&dd.join("plant_3.log"), 40..50);
    });
    assert_eq!(lines, (20..30).chain(40..50).collect::<Vec<u32>>(), "moved aside, read, and back");

    // A copy-and-truncate between the round's listing and the held open (a hook).
    let d = held_fixed("copy-in-the-window");
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hp, trigger) = (d.join("current.log"), Arc::clone(&armed));
    BEFORE_OPEN.lock().unwrap().push((
        d.join("current.log"),
        Box::new(move || {
            if trigger.swap(false, Ordering::SeqCst) {
                write(&hp.with_file_name("done_2.log"), 10..50);
                std::fs::OpenOptions::new().write(true).open(&hp).unwrap().set_len(0).unwrap();
                append(&hp, 50..55);
                mode(&hp, 0o644);
            }
        }),
    ));
    hooked.push(d.join("current.log"));
    let p = d.join("current.log");
    let (lines, _) = at_end(fixed(&d), 35, move || {
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(700));
        armed.store(true, Ordering::SeqCst);
    });
    assert_eq!(lines, (20..55).collect::<Vec<u32>>(), "a copy made between the listing and the open");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // Seen cut, then retired between rounds.
    let d = held_fixed("cut-then-retired");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 40, move || {
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(500));
        write(&dd.join("done_2.log"), 10..50);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(0).unwrap();
        append(&p, 50..55);
        std::thread::sleep(Duration::from_millis(700));
        std::fs::rename(&p, dd.join("done_3.log")).unwrap();
        mode(&dd.join("done_3.log"), 0o644);
        write(&p, 60..65);
    });
    assert_eq!(lines, (20..55).chain(60..65).collect::<Vec<u32>>(), "seen cut, then retired");

    // Seen cut, then deleted: its copy read from where it stood, its own lines gone with it.
    let d = held_fixed("cut-then-deleted");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 35, move || {
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(500));
        write(&dd.join("done_2.log"), 10..50);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(0).unwrap();
        append(&p, 50..55);
        std::thread::sleep(Duration::from_millis(700));
        std::fs::remove_file(&p).unwrap();
        write(&p, 60..65);
    });
    assert_eq!(lines, (20..50).chain(60..65).collect::<Vec<u32>>(), "seen cut, then deleted");

    // Seen cut, then retired between the round's listing and the held open (a hook).
    let d = held_fixed("cut-then-retired-in-the-window");
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hp, trigger) = (d.join("current.log"), Arc::clone(&armed));
    BEFORE_OPEN.lock().unwrap().push((
        d.join("current.log"),
        Box::new(move || {
            if trigger.swap(false, Ordering::SeqCst) {
                std::fs::rename(&hp, hp.with_file_name("done_3.log")).unwrap();
                mode(&hp.with_file_name("done_3.log"), 0o644);
                write(&hp, 60..65);
            }
        }),
    ));
    hooked.push(d.join("current.log"));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 40, move || {
        append(&p, 20..50);
        std::thread::sleep(Duration::from_millis(500));
        write(&dd.join("done_2.log"), 10..50);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_len(0).unwrap();
        append(&p, 50..55);
        std::thread::sleep(Duration::from_millis(700));
        armed.store(true, Ordering::SeqCst);
    });
    assert_eq!(lines, (20..55).chain(60..65).collect::<Vec<u32>>(), "seen cut, then retired before its open");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));
}

/// C-098's fifth refute round. An active held in turn has its look bracketed as the first one is:
/// a rotation between the round's listing and that open (a hook at the held file's read, the
/// active renamed into a retiree and another file held at its path) makes it no look, the
/// generation it made read first (20 to 70). In a newest-by-order family a new file made at the
/// held newest's name, the held file moved aside and read (20 to 30, then 40 to 55) or deleted
/// (40 to 55), is born after the look and read whole. With nothing at the fixed path, a held file
/// outside the pattern is still looked for, and no generation after it is read before its lines (20
/// to 65). And a held file the search finds back at the fixed path (a hook in the walk) is followed
/// there as the active, from where it stood, never read twice (20 to 55).
#[cfg(unix)]
#[test]
fn a_held_in_turn_look_a_new_file_at_the_held_name_and_an_empty_fixed_path_keep_the_order() {
    use super::DURING_WALK;
    let dir = tempfile::tempdir().unwrap();
    let fixed = |d: &std::path::Path| FamilySpec::new(d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let newest = |d: &std::path::Path| FamilySpec::new(d, r"plant_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let held_fixed = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("done_1.log"), 0..10);
        write(&d.join("current.log"), 10..20);
        mode(&d.join("current.log"), 0o200);
        d
    };
    let held_newest = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("plant_1.log"), 0..10);
        write(&d.join("plant_2.log"), 10..20);
        mode(&d.join("plant_2.log"), 0o200);
        d
    };
    let mut hooked: Vec<std::path::PathBuf> = Vec::new();

    // A rotation inside an active held in turn's look.
    let d = held_fixed("in-turn-window");
    let (retired, current, mut done) = (d.join("done_2.log"), d.join("current.log"), false);
    BEFORE_OPEN.lock().unwrap().push((
        d.join("done_2.log"),
        Box::new(move || {
            if !done && std::fs::File::open(&retired).is_ok() {
                done = true;
                std::fs::rename(&current, current.with_file_name("done_3.log")).unwrap();
                make_held(&current, 60..70);
            }
        }),
    ));
    hooked.push(d.join("done_2.log"));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 50, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("done_2.log")).unwrap();
        make_held(&p, 50..60);
        std::thread::sleep(Duration::from_millis(500));
        mode(&dd.join("done_2.log"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        mode(&dd.join("done_3.log"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        mode(&p, 0o644);
    });
    assert_eq!(lines, (20..70).collect::<Vec<u32>>(), "a rotation inside the held-in-turn look");
    BEFORE_OPEN.lock().unwrap().retain(|(at, _)| !hooked.contains(at));

    // A new file at the held newest's name, the held file moved aside.
    let d = held_newest("new-at-name-aside");
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 25, move || {
        append(&dd.join("plant_2.log"), 20..30);
        std::fs::rename(dd.join("plant_2.log"), dd.join("plant_2.log.old")).unwrap();
        mode(&dd.join("plant_2.log.old"), 0o644);
        write(&dd.join("aside.tmp"), 40..50);
        std::fs::rename(dd.join("aside.tmp"), dd.join("plant_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        append(&dd.join("plant_2.log"), 50..55);
    });
    assert_eq!(lines, (20..30).chain(40..55).collect::<Vec<u32>>(), "a new file at the held name, the held file aside");

    // A new file at the held newest's name, the held file deleted.
    let d = held_newest("new-at-name-deleted");
    let dd = d.clone();
    let (lines, _) = at_end(newest(&d), 15, move || {
        append(&dd.join("plant_2.log"), 20..30);
        std::fs::remove_file(dd.join("plant_2.log")).unwrap();
        write(&dd.join("aside.tmp"), 40..50);
        std::fs::rename(dd.join("aside.tmp"), dd.join("plant_2.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        append(&dd.join("plant_2.log"), 50..55);
    });
    assert_eq!(lines, (40..55).collect::<Vec<u32>>(), "a new file at the held name, the held file deleted");

    // Nothing at the fixed path while the held file is outside the pattern.
    let d = held_fixed("empty-fixed-path");
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 45, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.rotating")).unwrap();
        write(&p, 50..60);
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(&p, dd.join("done_3.log")).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        std::fs::rename(dd.join("current.rotating"), dd.join("done_2.log")).unwrap();
        mode(&dd.join("done_2.log"), 0o644);
        write(&p, 60..65);
    });
    assert_eq!(lines, (20..65).collect::<Vec<u32>>(), "nothing at the fixed path, the held file outside the pattern");

    // Found by the search back at the fixed path (a hook in the walk).
    let d = held_fixed("found-at-the-fixed-path");
    let (rotating, current, mut done) = (d.join("current.rotating"), d.join("current.log"), false);
    DURING_WALK.lock().unwrap().push((
        d.clone(),
        Box::new(move || {
            if !done && std::fs::File::open(&rotating).is_ok() {
                done = true;
                std::fs::rename(&rotating, &current).unwrap();
            }
        }),
    ));
    let (p, dd) = (d.join("current.log"), d.clone());
    let (lines, _) = at_end(fixed(&d), 35, move || {
        append(&p, 20..50);
        std::fs::rename(&p, dd.join("current.rotating")).unwrap();
        mode(&dd.join("current.rotating"), 0o644);
        std::thread::sleep(Duration::from_millis(1000));
        append(&p, 50..55);
    });
    assert_eq!(lines, (20..55).collect::<Vec<u32>>(), "found back at the fixed path");
    DURING_WALK.lock().unwrap().retain(|(at, _)| *at != d);
}

/// C-098's sixth refute round. The family listed again once the held file opened where it stood is
/// checked after that listing: a burst of two rotations between the open and the listing (a hook)
/// leaves the held file gone from the active's path, so the generations listed are generations, not
/// members born beside it, and the live loop reads them in order (20 to 65, each line once).
#[cfg(unix)]
#[test]
fn a_burst_between_the_held_open_and_the_listing_again_loses_no_generation() {
    use super::AFTER_HELD_OPEN;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("burst");
    std::fs::create_dir_all(&d).unwrap();
    write(&d.join("done_1.log"), 0..10);
    write(&d.join("current.log"), 10..20);
    mode(&d.join("current.log"), 0o200);
    let spec = FamilySpec::new(&d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let (current, mut done) = (d.join("current.log"), false);
    AFTER_HELD_OPEN.lock().unwrap().push((
        d.join("current.log"),
        Box::new(move || {
            if !done {
                done = true;
                std::fs::rename(&current, current.with_file_name("done_2.log")).unwrap();
                write(&current, 50..60);
                std::fs::rename(&current, current.with_file_name("done_3.log")).unwrap();
                write(&current, 60..65);
            }
        }),
    ));
    let p = d.join("current.log");
    let (lines, _) = at_end(spec, 45, move || {
        append(&p, 20..50);
        mode(&p, 0o644);
    });
    assert_eq!(lines, (20..65).collect::<Vec<u32>>(), "a burst between the held open and the listing again");
    AFTER_HELD_OPEN.lock().unwrap().retain(|(at, _)| *at != d.join("current.log"));
}

/// C-098's seventh refute round. A copy-and-truncate between the held open and the listing again
/// (a hook): the cut is judged again from the open file's length once the family is listed, the
/// copy read from where the held file stood and the held file from its top (20 to 55, each line
/// once), reopened there even when it was written again past the look's length (20 to 70).
#[cfg(unix)]
#[test]
fn a_copy_and_truncate_between_the_held_open_and_the_listing_again_is_read_from_where_it_stood() {
    use super::AFTER_HELD_OPEN;
    let dir = tempfile::tempdir().unwrap();
    // Cut and written again short of the look's length (20 to 55), and past it, short of the most
    // of it seen (20 to 70): the held file reopened from its top, never read on from the look.
    for (name, again_to) in [("cut-after-open", 55u32), ("cut-after-open-past-the-look", 70)] {
        let d = dir.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        write(&d.join("done_1.log"), 0..10);
        write(&d.join("current.log"), 10..20);
        mode(&d.join("current.log"), 0o200);
        let spec = FamilySpec::new(&d, r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
        let (current, mut done) = (d.join("current.log"), false);
        AFTER_HELD_OPEN.lock().unwrap().push((
            d.join("current.log"),
            Box::new(move || {
                if !done {
                    done = true;
                    write(&current.with_file_name("done_2.log"), 10..50);
                    std::fs::OpenOptions::new().write(true).open(&current).unwrap().set_len(0).unwrap();
                    append(&current, 50..again_to);
                }
            }),
        ));
        let p = d.join("current.log");
        let (lines, _) = at_end(spec, (again_to - 20) as usize, move || {
            append(&p, 20..50);
            std::thread::sleep(Duration::from_millis(500));
            mode(&p, 0o644);
        });
        assert_eq!(lines, (20..again_to).collect::<Vec<u32>>(), "{name}");
        AFTER_HELD_OPEN.lock().unwrap().retain(|(at, _)| *at != d.join("current.log"));
    }
}
