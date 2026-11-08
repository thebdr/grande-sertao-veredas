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
use veredas_portal::tail::{line_table, plan_for, plan_for_with};
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

/// The writer rotates while the backfill locates its start: the active file, listed as
/// `current.log`, is retired into the pattern before it is peeked. The locate lists again and
/// plans the family as it now is, instead of ending the worker on a file that is no longer there
/// (found by the fixed-name seam test, which met the race one run in six).
#[test]
fn a_rotation_during_the_locate_is_listed_again_never_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let writer =
        write_history(dir.path(), NamingScheme::Fixed { active: "current.log".into(), retired_prefix: "done_".into() }, 700, 400);
    assert!(writer.generations_finished() >= 3);
    let spec = FamilySpec::new(dir.path(), r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    // The plan at rest: the file holding line 150 at its offset, through the active.
    let (rest_files, rest_offset, _) = plan_for(&spec, &ts_spec(), stamp(150)).unwrap().unwrap();
    assert_eq!(rest_files.last().map(String::as_str), Some("current.log"));
    // The next retired name the writer would use.
    let next = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("done_")?.strip_suffix(".log")?.parse::<u64>().ok())
        .max()
        .unwrap()
        + 1;
    let retired = format!("done_{next}.log");
    let mut rotated = false;
    let (files, offset, _) = plan_for_with(&spec, &ts_spec(), stamp(150), &mut |path| {
        if !rotated && path.file_name().is_some_and(|n| n == "current.log") {
            // Retired between the listing and its peek, the fresh active not born yet: the peek
            // finds nothing at the path it was given.
            std::fs::rename(dir.path().join("current.log"), dir.path().join(&retired)).unwrap();
            rotated = true;
        }
    })
    .unwrap()
    .unwrap();
    assert!(rotated, "the hook never saw the active file");
    // The same start in the same file, and the retired generation planned under its new name as
    // the last file: the fresh active is not there to plan.
    assert_eq!(offset, rest_offset);
    let mut expected = rest_files.clone();
    expected.pop();
    expected.push(retired.clone());
    assert_eq!(files, expected);
    writer.finish();
}

// ---- the first refute round ------------------------------------------------------------------

fn utf16le(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// A stamp equal to the start and shared across a seam begins in the earlier file: the located
/// start is the first line at or after the start, wherever it stands, so the earlier file's
/// lines on that stamp are replayed, not skipped with the file.
#[test]
fn a_start_on_a_stamp_shared_across_the_seam_begins_in_the_earlier_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s_1.log"), "2026-07-17 10:00:00 a\n2026-07-17 10:00:01 b\n").unwrap();
    std::fs::write(dir.path().join("s_2.log"), "2026-07-17 10:00:01 c\n2026-07-17 10:00:02 d\n").unwrap();
    let spec = FamilySpec::new(dir.path(), r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:00:01")).unwrap().unwrap();
    assert_eq!(files, ["s_1.log", "s_2.log"]);
    assert_eq!(offset, 22, "line b, the first at 10:00:01");
    assert!(skipped.is_empty(), "{skipped:?}");
}

/// A plan taken while the active is still empty (the state right after a rotation) leaves the
/// encoding to the live tail: the UTF-16 the writer then puts there is decoded as such, never
/// forced to UTF-8 by a peek that had no bytes to detect from.
#[test]
fn a_file_planned_while_empty_is_decoded_by_what_it_later_holds() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("w_1.log"), utf16le("2026-07-17 10:00:00 a\n")).unwrap();
    std::fs::write(dir.path().join("w_2.log"), b"").unwrap();
    let spec = FamilySpec::new(dir.path(), r"w_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let start = dt("2026-07-17 10:00:30");
    assert_eq!(plan_for(&spec, &ts_spec(), start).unwrap().unwrap(), (vec!["w_2.log".to_string()], 0, vec!["w_1.log".to_string()]));
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("w", "w", spec, cfg(FamilyStart::Backfill(start))).with_timestamps(ts_spec()))).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(dir.path().join("w_2.log"), utf16le("2026-07-17 10:01:00 b\nDONE\n")).unwrap();
    let events = collect_until(&rx, DONE, Duration::from_secs(20));
    set.stop_all();
    assert_eq!(events.iter().map(|e| e.line.as_str()).collect::<Vec<_>>(), ["2026-07-17 10:01:00 b", DONE]);
    assert_eq!(events[0].parsed_ts, Some(dt("2026-07-17 10:01:00")));
}

/// The writer rotates while the backfill locates its start and the fresh active is already
/// there when the retired one is peeked: the peek reads a file the listing did not name. The
/// locate lists again, and the plan carries the retired generation before the active at the
/// same offset, so nothing before the start leaks in as a fresh retiree read whole.
#[test]
fn a_rotation_with_a_fresh_active_during_the_locate_is_listed_again() {
    let dir = tempfile::tempdir().unwrap();
    let writer =
        write_history(dir.path(), NamingScheme::Fixed { active: "current.log".into(), retired_prefix: "done_".into() }, 700, 400);
    assert!(writer.generations_finished() >= 3);
    let spec = FamilySpec::new(dir.path(), r"done_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let (rest_files, rest_offset, _) = plan_for(&spec, &ts_spec(), stamp(150)).unwrap().unwrap();
    assert_eq!(rest_files.last().map(String::as_str), Some("current.log"));
    let next = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("done_")?.strip_suffix(".log")?.parse::<u64>().ok())
        .max()
        .unwrap()
        + 1;
    let retired = format!("done_{next}.log");
    let mut rotated = false;
    let (files, offset, _) = plan_for_with(&spec, &ts_spec(), stamp(150), &mut |path| {
        if !rotated && path.file_name().is_some_and(|n| n == "current.log") {
            std::fs::rename(dir.path().join("current.log"), dir.path().join(&retired)).unwrap();
            std::fs::write(dir.path().join("current.log"), format!("{}\n", line(400))).unwrap();
            rotated = true;
        }
    })
    .unwrap()
    .unwrap();
    assert!(rotated, "the hook never saw the active file");
    assert_eq!(offset, rest_offset);
    let mut expected = rest_files.clone();
    expected.pop();
    expected.push(retired.clone());
    expected.push("current.log".into());
    assert_eq!(files, expected);
    writer.finish();
}

/// A family whose only member holds no lines yet is no family to locate in, in either mode: the
/// backfill falls through to the live attach, the lines then written are read whole and the
/// worker ends cleanly, instead of refusing for lines that do not exist.
#[test]
fn a_backfill_into_a_family_with_no_lines_falls_through_to_live() {
    let start = dt("2026-07-17 10:00:00");
    for (pattern, active, mode) in [
        (r"s_(?P<idx>\d+)\.log", "s_1.log", ActiveMode::NewestByOrder),
        (r"done_(?P<idx>\d+)\.log", "current.log", ActiveMode::Fixed("current.log".into())),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(active), b"").unwrap();
        let spec = FamilySpec::new(dir.path(), pattern, None, mode).unwrap();
        assert!(plan_for(&spec, &ts_spec(), start).unwrap().is_none(), "{active}: nothing to locate in");
        let (mut set, rx) = SourceSet::new();
        set.spawn(Box::new(FamilyTailSource::new("e", active, spec, cfg(FamilyStart::Backfill(start))).with_timestamps(ts_spec()))).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.path().join(active), "2026-07-17 10:00:05 born\nDONE\n").unwrap();
        let events = collect_until(&rx, DONE, Duration::from_secs(20));
        assert_eq!(events.iter().map(|e| e.line.as_str()).collect::<Vec<_>>(), ["2026-07-17 10:00:05 born", DONE], "{active}");
        for (_, r) in set.stop_all() {
            r.unwrap();
        }
    }
}

/// A start between two files begins at the next file's first stamped line: its unstamped opening
/// lines belong to the earlier file's last stamp, before the start, and are skipped.
#[test]
fn a_start_between_files_skips_the_next_file_s_unstamped_opening_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s_1.log"), "2026-07-17 10:00:00 a\n").unwrap();
    let s2 = "  at frame 7\n  at frame 8\n2026-07-17 10:00:02 c\n";
    std::fs::write(dir.path().join("s_2.log"), s2).unwrap();
    let spec = FamilySpec::new(dir.path(), r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:00:01")).unwrap().unwrap();
    assert_eq!(files, ["s_2.log"]);
    assert_eq!(offset, s2.find("2026-07-17").unwrap() as u64);
    assert_eq!(skipped, ["s_1.log"]);
}

/// A head carrying no stamp within the peek's cap, under a tail that does, is scanned forward to
/// its first stamp: a long unstamped header never hides a file from the locate, so the start
/// inside it is found rather than the whole family replayed.
#[test]
fn a_long_unstamped_header_does_not_hide_a_file_from_the_locate() {
    let dir = tempfile::tempdir().unwrap();
    let mut s1 = String::new();
    for i in 0..450 {
        s1.push_str(&format!("header line {i}\n"));
    }
    for s in 0..10 {
        s1.push_str(&format!("2026-07-17 10:00:0{s} x\n"));
    }
    std::fs::write(dir.path().join("s_1.log"), &s1).unwrap();
    std::fs::write(dir.path().join("s_2.log"), "2026-07-17 10:01:00 y\n").unwrap();
    let spec = FamilySpec::new(dir.path(), r"s_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (files, offset, skipped) = plan_for(&spec, &ts_spec(), dt("2026-07-17 10:00:05")).unwrap().unwrap();
    assert_eq!(files, ["s_1.log", "s_2.log"]);
    assert_eq!(offset, s1.find("2026-07-17 10:00:05").unwrap() as u64);
    assert!(skipped.is_empty(), "{skipped:?}");
}
