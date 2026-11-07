//! Backfill (contract: the backfill clause): a start point in the past located inside a family
//! (name tokens a hint, line stamps the truth, name dates the fallback, a refusal with neither),
//! the retired files replayed from the located line onward at full speed, and the handoff to
//! live tailing at the exact byte, no gap and no duplicate at the seam, while the writer keeps
//! appending and rotating.

mod common;

use std::path::Path;
use std::time::Duration;

use chrono::NaiveDateTime;
use common::{FamilyWriter, NamingScheme, RotateStyle, collect_until};
use veredas_portal::encoding::DetectedEncoding;
use veredas_portal::tail::{line_table, plan_for};
use veredas_portal::{
    ActiveMode, FamilySpec, FamilyStart, FamilyTailConfig, FamilyTailSource, LineEvent, SourceError, SourceSet,
    TimestampSpec,
};

fn ts_spec() -> TimestampSpec {
    TimestampSpec::new(r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d{3})?)", ["%Y-%m-%d %H:%M:%S%.3f", "%Y-%m-%d %H:%M:%S"])
        .unwrap()
}

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

// ---- locating the start point ----------------------------------------------------------------

fn stamped_family(dir: &Path) -> FamilySpec {
    // Three files, two lines each, minute-spaced stamps.
    std::fs::write(dir.join("s_1.log"), "2026-07-17 10:00:00 a\n2026-07-17 10:01:00 b\n").unwrap();
    std::fs::write(dir.join("s_2.log"), "2026-07-17 10:02:00 c\n2026-07-17 10:03:00 d\n").unwrap();
    std::fs::write(dir.join("s_3.log"), "2026-07-17 10:04:00 e\n2026-07-17 10:05:00 f\n").unwrap();
    FamilySpec::new(dir, r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap()
}

#[test]
fn the_raw_line_tables_agree_across_encodings_and_offsets_are_byte_exact() {
    let text = "alpha\r\nbeta\ngamma"; // mixed endings, a partial last line
    let narrow = line_table(text.as_bytes(), DetectedEncoding::Utf8);
    assert_eq!(narrow, vec![(0, "alpha".to_string()), (7, "beta".to_string()), (12, "gamma".to_string())]);
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(text.as_bytes());
    assert_eq!(line_table(&bom, DetectedEncoding::Utf8)[1].0, 10, "the mark shifts every offset by three");

    let mut wide = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        wide.extend_from_slice(&u.to_le_bytes());
    }
    let table = line_table(&wide, DetectedEncoding::Utf16Le);
    assert_eq!(table.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["alpha", "beta", "gamma"]);
    // Offsets are line starts in raw bytes: the mark (2) plus "alpha\r\n" (14) is 16.
    assert_eq!(table[1].0, 16);
    // A torn trailing code unit is left off the table.
    let mut torn = wide.clone();
    torn.push(b'x');
    assert_eq!(line_table(&torn, DetectedEncoding::Utf16Le).len(), 3);
    assert_eq!(line_table(b"", DetectedEncoding::Cp1252), vec![]);
}

#[test]
fn a_start_mid_file_begins_at_that_line_and_skips_the_older_files() {
    let dir = tempfile::tempdir().unwrap();
    let spec = stamped_family(dir.path());
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:03:00")).unwrap().unwrap();
    // 10:03 is line 2 of s_2: the replay begins there, then s_3.
    assert_eq!(files, ["s_2.log", "s_3.log"]);
    assert_eq!(offset, 22); // after "2026-07-17 10:02:00 c\n"
    assert_eq!(skipped, vec!["s_1.log".to_string()]);
}

#[test]
fn a_start_between_files_or_before_everything_begins_at_a_file_s_top() {
    let dir = tempfile::tempdir().unwrap();
    let spec = stamped_family(dir.path());
    // Between s_1's last (10:01) and s_2's first (10:02).
    let (files, offset, _) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:01:30")).unwrap().unwrap();
    assert_eq!((files[0].as_str(), offset), ("s_2.log", 0));
    // Before the family existed: the whole family, offset 0.
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 09:00:00")).unwrap().unwrap();
    assert_eq!(files, ["s_1.log", "s_2.log", "s_3.log"]);
    assert_eq!(offset, 0);
    assert!(skipped.is_empty());
}

#[test]
fn a_start_after_everything_is_live_only_from_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let spec = stamped_family(dir.path());
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 11:00:00")).unwrap().unwrap();
    assert_eq!(files, ["s_3.log"]);
    assert_eq!(offset, 44, "the file's length: nothing to replay");
    assert_eq!(skipped, ["s_1.log", "s_2.log"]);
}

#[test]
fn an_empty_family_has_no_plan_and_unstamped_lines_before_the_start_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    assert!(plan_for(&spec, &ts_spec(), dt("2026-07-17 10:00:00")).unwrap().is_none());

    std::fs::write(dir.path().join("s_1.log"), "2026-07-17 10:00:00 a\ncontinuation\n2026-07-17 10:02:00 c\n").unwrap();
    let (_, offset, _) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:01:00")).unwrap().unwrap();
    assert_eq!(offset, 35, "the unstamped continuation belongs to the earlier stamp and is skipped");
}

#[test]
fn file_name_dates_are_the_fallback_and_with_neither_the_backfill_refuses() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d_2026-07-15.log"), "x\ny\n").unwrap();
    std::fs::write(dir.path().join("d_2026-07-16.log"), "x\ny\n").unwrap();
    std::fs::write(dir.path().join("d_2026-07-17.log"), "x\ny\n").unwrap();
    let spec =
        FamilySpec::new(dir.path(), r"d_(?P<date>\d{4}-\d{2}-\d{2})\.log", Some("%Y-%m-%d"), ActiveMode::NewestByOrder)
            .unwrap();
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-16 12:00:00")).unwrap().unwrap();
    // Day granularity: begin at the 16th's file, offset 0.
    assert_eq!(files, ["d_2026-07-16.log", "d_2026-07-17.log"]);
    assert_eq!(offset, 0);
    assert_eq!(skipped, ["d_2026-07-15.log"]);

    let dir2 = tempfile::tempdir().unwrap();
    std::fs::write(dir2.path().join("n_1.log"), "x\n").unwrap();
    let spec2 = FamilySpec::new(dir2.path(), r"n_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    match plan_for(&spec2, &ts_spec(), dt("2026-07-16 12:00:00")) {
        Err(SourceError::Backfill(msg)) => assert_eq!(
            msg,
            "cannot locate the start point: no line timestamps parse and the file names carry no dates; check the timestamp spec"
        ),
        other => panic!("expected a backfill refusal, got {other:?}"),
    }
}

#[test]
fn the_prefilter_drops_files_whose_successor_proves_they_ended_before_the_start_day() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("p_2026-07-10.log"), "2026-07-10 08:00:00 old\n").unwrap();
    std::fs::write(dir.path().join("p_2026-07-11.log"), "2026-07-11 08:00:00 old\n").unwrap();
    std::fs::write(dir.path().join("p_2026-07-17.log"), "2026-07-17 08:00:00 new\n2026-07-17 12:00:00 newer\n").unwrap();
    let spec =
        FamilySpec::new(dir.path(), r"p_(?P<date>\d{4}-\d{2}-\d{2})\.log", Some("%Y-%m-%d"), ActiveMode::NewestByOrder)
            .unwrap();
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 09:00:00")).unwrap().unwrap();
    assert_eq!(files, ["p_2026-07-17.log"]);
    assert_eq!(offset, 24, "09:00 is after the 08:00 line: the replay starts at line 2");
    assert_eq!(skipped, ["p_2026-07-10.log", "p_2026-07-11.log"]);
}

#[test]
fn a_backfill_start_without_a_timestamp_rule_is_refused_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (mut set, _rx) = SourceSet::new();
    let err = set
        .spawn(Box::new(FamilyTailSource::new("bf", "bf", spec, cfg(FamilyStart::Backfill(dt("2026-07-17 10:00:00"))))))
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "source not configured: a Backfill start point requires a timestamp spec (with_timestamps)");
    assert!(set.handles().is_empty());
}

// ---- the seam ------------------------------------------------------------------------------

const DONE: &str = "DONE";

fn base_ts() -> NaiveDateTime {
    dt("2026-07-17 10:00:00")
}

/// A deterministic stamp for line n: the base plus n times 10 ms.
fn stamp(n: u64) -> NaiveDateTime {
    base_ts() + chrono::Duration::milliseconds(n as i64 * 10)
}

fn line(n: u64) -> String {
    format!("{} L{n:05}", stamp(n).format("%Y-%m-%d %H:%M:%S%.3f"))
}

fn cfg(start: FamilyStart) -> FamilyTailConfig {
    FamilyTailConfig {
        poll_interval: Duration::from_millis(10),
        rescan_interval: Duration::from_millis(25),
        drain_grace: Duration::from_millis(60),
        use_notify: None,
        start,
    }
}

/// The payload of a received line, without its stamp.
fn payload(l: &str) -> &str {
    l.rsplit(' ').next().unwrap_or(l)
}

fn assert_seamless(name: &str, events: &[LineEvent], first: u64, last: u64) {
    let payloads: Vec<String> = events.iter().map(|e| payload(&e.line).to_string()).collect();
    let expected: Vec<String> = (first..=last).map(|n| format!("L{n:05}")).chain(std::iter::once(DONE.to_string())).collect();
    common::assert_exact(name, &payloads, &expected);
    // The sequence is dense from 0: nothing was emitted before the backfill's output.
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.seq, i as u64, "seq gap at index {i}");
    }
    // Stamps parsed throughout the replayed range (the end marker has none).
    let with_ts = events.iter().filter(|e| e.parsed_ts.is_some()).count();
    assert_eq!(with_ts, events.len() - 1, "{name}: a parsed stamp is missing somewhere");
    assert_eq!(events[0].parsed_ts.unwrap(), stamp(first));
    assert_eq!(events[0].index_ms(), stamp(first).and_utc().timestamp_millis());
}

fn write_history(dir: &Path, scheme: NamingScheme, cap: u64, upto: u64) -> FamilyWriter {
    let mut w = FamilyWriter::new(dir, scheme, cap);
    w.rotate_style = RotateStyle::CleanClose;
    for n in 0..upto {
        w.write_line(&line(n));
    }
    w
}

fn run_seam_scenario(name: &'static str, scheme: NamingScheme, active_mode: ActiveMode, pattern: &'static str) {
    let dir = tempfile::tempdir().unwrap();
    // History: lines 0..400 across several retired files, the writer parked.
    let mut writer = write_history(dir.path(), scheme, 700, 400);
    assert!(writer.generations_finished() >= 3, "the history is too small: {} generations", writer.generations_finished());

    // The start point: exactly line 150's stamp (an inclusive boundary).
    let start = stamp(150);
    let spec = FamilySpec::new(dir.path(), pattern, None, active_mode).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("bf", name, spec, cfg(FamilyStart::Backfill(start))).with_timestamps(ts_spec())))
        .unwrap();

    // The writer resumes at once: the backfill races live appends and rotations.
    let writer_thread = std::thread::spawn(move || {
        for n in 400..700 {
            writer.write_line(&line(n));
            if n % 60 == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        writer.write_line(DONE);
        writer.finish();
    });
    let events = collect_until(&rx, DONE, Duration::from_secs(120));
    writer_thread.join().expect("the writer panicked");
    set.stop_all();
    assert_seamless(name, &events, 150, 699);
}

#[test]
fn the_seam_is_exact_in_an_indexed_family() {
    run_seam_scenario(
        "seam/indexed",
        NamingScheme::Indexed { prefix: "bf_".into() },
        ActiveMode::NewestByOrder,
        r"bf_(?P<idx>\d+)\.log",
    );
}

#[test]
fn the_seam_is_exact_in_a_fixed_name_family() {
    run_seam_scenario(
        "seam/fixed",
        NamingScheme::Fixed { active: "current.log".into(), retired_prefix: "done_".into() },
        ActiveMode::Fixed("current.log".into()),
        r"done_(?P<idx>\d+)\.log",
    );
}

/// A purely historical session: the family is at rest, the backfill replays the requested range
/// and live mode follows a file that never grows again.
#[test]
fn a_resting_family_replays_exactly_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = write_history(dir.path(), NamingScheme::Indexed { prefix: "hist_".into() }, 700, 300);
    writer.write_line(DONE);
    writer.finish();
    let spec = FamilySpec::new(dir.path(), r"hist_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(
        FamilyTailSource::new("hist", "resting", spec, cfg(FamilyStart::Backfill(stamp(120)))).with_timestamps(ts_spec()),
    ))
    .unwrap();
    let events = collect_until(&rx, DONE, Duration::from_secs(60));
    set.stop_all();
    assert_seamless("seam/resting", &events, 120, 299);
}

/// A start point after everything on disk: nothing replays, the tail is live-only from the end.
#[test]
fn a_start_after_everything_replays_nothing_and_follows_live() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = write_history(dir.path(), NamingScheme::Indexed { prefix: "fut_".into() }, 700, 100);
    let spec = FamilySpec::new(dir.path(), r"fut_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(
        FamilyTailSource::new("fut", "future", spec, cfg(FamilyStart::Backfill(stamp(5000)))).with_timestamps(ts_spec()),
    ))
    .unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "history leaked into a future-start session");
    // New lines flow (they carry stamps before the start; a live tail follows reality, the
    // start point only bounds the replay).
    writer.write_line(&line(100));
    writer.write_line(DONE);
    writer.finish();
    let events = collect_until(&rx, DONE, Duration::from_secs(30));
    set.stop_all();
    assert_eq!(events.iter().map(|e| payload(&e.line)).collect::<Vec<_>>(), ["L00100", DONE]);
}

/// A backfill into an empty folder falls through to a live attach: whatever appears is newer
/// than the past start and is read whole.
#[test]
fn a_backfill_into_an_empty_family_reads_whatever_is_born_whole() {
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"born_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(
        FamilyTailSource::new("born", "born", spec, cfg(FamilyStart::Backfill(stamp(0)))).with_timestamps(ts_spec()),
    ))
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let mut writer = FamilyWriter::new(dir.path(), NamingScheme::Indexed { prefix: "born_".into() }, 700);
    for n in 0..50 {
        writer.write_line(&line(n));
    }
    writer.write_line(DONE);
    writer.finish();
    let events = collect_until(&rx, DONE, Duration::from_secs(30));
    set.stop_all();
    assert_seamless("seam/born", &events, 0, 49);
}
