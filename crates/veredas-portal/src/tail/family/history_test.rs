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
