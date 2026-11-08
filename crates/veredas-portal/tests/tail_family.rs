//! Log families (contract: the families clause): a rotating family resolved from its names in
//! rotation order, and followed through its rotations with zero lines lost, zero duplicated and
//! the order kept: against the bench's emitters writing the files a plant writes, and against a
//! writer that plays the rotation races a plant logger shows (a straggler landing late in the
//! retired file, torn writes, generations born inside one rescan).

mod common;

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use common::{FamilyWriter, NamingScheme, RotateStyle, assert_exact, collect_until};
use veredas_maquinar::emit::{self, EmitterConfig, Encoding, ValueField, family_shape};
use veredas_portal::{
    ActiveMode, FamilyFile, FamilySpec, FamilyStart, FamilyTailConfig, FamilyTailSource, LineEvent, OrderKey,
    SourceError, SourceSet,
};

// ---- resolving a family ----------------------------------------------------------------------

fn touch(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), b"x\n").unwrap();
}

fn names(files: &[FamilyFile]) -> Vec<&str> {
    files.iter().map(|f| f.file_name.as_str()).collect()
}

#[test]
fn unpadded_indices_sort_numerically_and_the_newest_is_active() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["app_10.log", "app_2.log", "app_1.log", "app_9.log"] {
        touch(dir.path(), n);
    }
    let spec = FamilySpec::new(dir.path(), r"app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), vec!["app_1.log", "app_2.log", "app_9.log", "app_10.log"]);
    assert_eq!(files.iter().map(|f| f.is_active).collect::<Vec<_>>(), [false, false, false, true]);
    assert_eq!(files[3].key, OrderKey { date: None, idx: Some(10) });
    assert_eq!(files[0].path, dir.path().join("app_1.log"));
}

#[test]
fn dated_files_sort_chronologically_and_date_plus_index_by_date_then_index() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["sys-2026-07-15.log", "sys-2026-01-20.log", "sys-2026-07-01.log"] {
        touch(dir.path(), n);
    }
    let spec =
        FamilySpec::new(dir.path(), r"sys-(?P<date>\d{4}-\d{2}-\d{2})\.log", Some("%Y-%m-%d"), ActiveMode::NewestByOrder)
            .unwrap();
    assert_eq!(names(&spec.resolve().unwrap()), vec!["sys-2026-01-20.log", "sys-2026-07-01.log", "sys-2026-07-15.log"]);

    let dir = tempfile::tempdir().unwrap();
    for n in ["m_20260717_3.log", "m_20260716_12.log", "m_20260717_10.log", "m_20260716_2.log"] {
        touch(dir.path(), n);
    }
    let spec =
        FamilySpec::new(dir.path(), r"m_(?P<date>\d{8})_(?P<idx>\d+)\.log", Some("%Y%m%d"), ActiveMode::NewestByOrder)
            .unwrap();
    assert_eq!(
        names(&spec.resolve().unwrap()),
        vec!["m_20260716_2.log", "m_20260716_12.log", "m_20260717_3.log", "m_20260717_10.log"]
    );
}

#[test]
fn full_datetime_stamps_parse_and_a_date_alone_is_midnight() {
    let spec = FamilySpec::new("unused", r"t_(?P<date>\d{8}_\d{6})\.log", Some("%Y%m%d_%H%M%S"), ActiveMode::NewestByOrder)
        .unwrap();
    let key = spec.parse_key("t_20260717_143005.log").unwrap();
    assert_eq!(key.date.unwrap().to_string(), "2026-07-17 14:30:05");
    assert_eq!(key.idx, None);
    let spec = FamilySpec::new("unused", r"d_(?P<date>\d{8})\.log", Some("%Y%m%d"), ActiveMode::NewestByOrder).unwrap();
    assert_eq!(spec.parse_key("d_20260717.log").unwrap().date.unwrap().to_string(), "2026-07-17 00:00:00");
    assert_eq!(spec.parse_key("other.log"), None);
}

#[test]
fn fixed_mode_lists_the_active_file_last_and_marked() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["current.log", "retired_2026-07-15.log", "retired_2026-07-16.log"] {
        touch(dir.path(), n);
    }
    let spec = FamilySpec::new(
        dir.path(),
        r"retired_(?P<date>\d{4}-\d{2}-\d{2})\.log",
        Some("%Y-%m-%d"),
        ActiveMode::Fixed("current.log".into()),
    )
    .unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), vec!["retired_2026-07-15.log", "retired_2026-07-16.log", "current.log"]);
    assert!(files.last().unwrap().is_active);
    assert_eq!(files.iter().filter(|f| f.is_active).count(), 1);
    assert_eq!(files[2].key, OrderKey::NONE);
}

#[test]
fn fixed_mode_without_the_active_file_lists_only_the_retirees() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "retired_1.log");
    let spec = FamilySpec::new(dir.path(), r"retired_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), vec!["retired_1.log"]);
    assert!(!files[0].is_active);
}

#[test]
fn non_members_are_excluded_and_the_match_is_the_whole_name() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["app_1.log", "notes.txt", "app_x.log", "xapp_2.logx"] {
        touch(dir.path(), n);
    }
    std::fs::create_dir(dir.path().join("app_3.log")).unwrap(); // a folder of a member's name
    let spec = FamilySpec::new(dir.path(), r"app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    assert_eq!(names(&spec.resolve().unwrap()), vec!["app_1.log"]);
}

#[test]
fn an_unparseable_date_token_degrades_to_none_and_sorts_first() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["d_99999999.log", "d_20260717.log"] {
        touch(dir.path(), n);
    }
    let spec = FamilySpec::new(dir.path(), r"d_(?P<date>\d{8})\.log", Some("%Y%m%d"), ActiveMode::NewestByOrder).unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), vec!["d_99999999.log", "d_20260717.log"]);
    assert_eq!(files[0].key, OrderKey::NONE);
    assert!(files[1].key.date.is_some());
}

#[test]
fn a_recursive_scan_matches_relative_paths_orders_across_subfolders_and_keeps_same_names_apart() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["01", "02", "02/deep"] {
        std::fs::create_dir_all(dir.path().join(sub)).unwrap();
    }
    touch(dir.path(), "f_0.log");
    touch(&dir.path().join("01"), "f_1.log");
    touch(&dir.path().join("02"), "f_2.log");
    touch(&dir.path().join("01"), "f_3.log");
    touch(&dir.path().join("02/deep"), "f_9.log");
    touch(&dir.path().join("01"), "notes.txt");
    let spec = FamilySpec::new(dir.path(), r"(?:[^/]+/)*f_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder)
        .unwrap()
        .with_recursive(true);
    assert!(spec.recursive());
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), vec!["f_0.log", "01/f_1.log", "02/f_2.log", "01/f_3.log", "02/deep/f_9.log"]);
    assert!(files.last().unwrap().is_active);
    assert_eq!(files[4].path, dir.path().join("02").join("deep").join("f_9.log"));
    // The same spec without the flag sees the flat folder alone.
    let flat = FamilySpec::new(dir.path(), r"(?:[^/]+/)*f_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    assert_eq!(names(&flat.resolve().unwrap()), vec!["f_0.log"]);
    // A pattern confined to a subtree.
    let one = FamilySpec::new(dir.path(), r"01/f_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
    assert_eq!(names(&one.resolve().unwrap()), vec!["01/f_1.log", "01/f_3.log"]);
    // The same file name in two folders stays two members.
    touch(&dir.path().join("01"), "x.log");
    touch(&dir.path().join("02"), "x.log");
    let same = FamilySpec::new(dir.path(), r"\d+/x\.log", None, ActiveMode::NewestByOrder).unwrap().with_recursive(true);
    let mut got = names(&same.resolve().unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
    got.sort();
    assert_eq!(got, vec!["01/x.log", "02/x.log"]);
}

#[test]
fn a_date_group_and_its_format_come_together_and_a_bad_pattern_is_refused() {
    let err = FamilySpec::new("x", r"(?P<date>\d+)\.log", None, ActiveMode::NewestByOrder).err().unwrap();
    assert_eq!(err.to_string(), r"family spec: pattern `(?P<date>\d+)\.log` captures (?P<date>…) but no date format was given");
    let err = FamilySpec::new("x", r"plain\.log", Some("%Y%m%d"), ActiveMode::NewestByOrder).err().unwrap();
    assert_eq!(err.to_string(), r"family spec: date format `%Y%m%d` given but pattern `plain\.log` has no (?P<date>…) group");
    assert!(matches!(FamilySpec::new("x", r"(broken", None, ActiveMode::NewestByOrder), Err(SourceError::Pattern(_))));
}

#[test]
fn keyless_members_order_by_mtime_then_name_and_a_missing_folder_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "b.log");
    touch(dir.path(), "a.log");
    let spec = FamilySpec::new(dir.path(), r"[ab]\.log", None, ActiveMode::NewestByOrder).unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(files.len(), 2);
    let mut sorted = names(&files);
    sorted.sort();
    assert_eq!(sorted, vec!["a.log", "b.log"]);

    let missing = FamilySpec::new(dir.path().join("nope"), r".*", None, ActiveMode::NewestByOrder).unwrap();
    match missing.resolve() {
        Err(SourceError::Io { path, .. }) => assert_eq!(path, dir.path().join("nope")),
        other => panic!("expected an I/O error naming the folder, got {other:?}"),
    }
}

// ---- following a family through its rotations -------------------------------------------------

const DONE: &str = "DONE";
const ATTACH: &str = "ATTACH";

struct Scenario {
    name: &'static str,
    scheme: NamingScheme,
    active_mode: ActiveMode,
    pattern: &'static str,
    rotate_style: RotateStyle,
    split_writes: bool,
    total_lines: u64,
    cap_bytes: u64,
    /// The writer's pacing: a 1 ms pause every N lines (0 is flat out).
    breath_every: u64,
    cfg: FamilyTailConfig,
}

fn indexed_scheme() -> (NamingScheme, ActiveMode, &'static str) {
    (NamingScheme::Indexed { prefix: "syn_".into() }, ActiveMode::NewestByOrder, r"syn_(?P<idx>\d+)\.log")
}

fn fixed_scheme() -> (NamingScheme, ActiveMode, &'static str) {
    (
        NamingScheme::Fixed { active: "current.log".into(), retired_prefix: "old_".into() },
        ActiveMode::Fixed("current.log".into()),
        r"old_(?P<idx>\d+)\.log",
    )
}

fn first_file_name(scheme: &NamingScheme) -> String {
    match scheme {
        NamingScheme::Indexed { prefix } => format!("{prefix}1.log"),
        NamingScheme::Fixed { active, .. } => active.clone(),
    }
}

fn cfg(poll_ms: u64, rescan_ms: u64, grace_ms: u64) -> FamilyTailConfig {
    FamilyTailConfig {
        poll_interval: Duration::from_millis(poll_ms),
        rescan_interval: Duration::from_millis(rescan_ms),
        drain_grace: Duration::from_millis(grace_ms),
        use_notify: None,
        start: FamilyStart::ActiveBegin,
    }
}

fn spawn_tail(
    dir: &Path,
    pattern: &str,
    active_mode: ActiveMode,
    cfg: FamilyTailConfig,
    name: &'static str,
) -> (SourceSet, Receiver<LineEvent>) {
    let spec = FamilySpec::new(dir, pattern, None, active_mode).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("fam", name, spec, cfg))).unwrap();
    (set, rx)
}

fn run_writer(
    dir: &Path,
    scheme: NamingScheme,
    cap_bytes: u64,
    rotate_style: RotateStyle,
    split_writes: bool,
    total_lines: u64,
    breath_every: u64,
) -> std::thread::JoinHandle<u32> {
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let mut w = FamilyWriter::new(dir, scheme, cap_bytes);
        w.rotate_style = rotate_style;
        w.split_writes = split_writes;
        for n in 0..total_lines {
            w.write_line(&format!("L{n:06}"));
            if breath_every > 0 && n % breath_every == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        w.write_line(DONE);
        let generations = w.generations_finished();
        w.finish();
        generations
    })
}

fn expected_sequence(total: u64) -> Vec<String> {
    (0..total).map(|n| format!("L{n:06}")).chain(std::iter::once(DONE.to_string())).collect()
}

/// Drive one scenario end to end and hand back every received line.
fn run_scenario(s: Scenario) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    // The attach handshake: the first file exists with a sentinel line before the tail starts,
    // and the writer starts only after the tail emitted the sentinel, so the attach provably
    // precedes the first real line however fast the writer rotates.
    std::fs::write(dir.path().join(first_file_name(&s.scheme)), b"ATTACH\r\n").unwrap();
    let (set, rx) = spawn_tail(dir.path(), s.pattern, s.active_mode.clone(), s.cfg.clone(), s.name);
    let attach = rx.recv_timeout(Duration::from_secs(30)).expect("the tail never emitted the attach sentinel");
    assert_eq!(attach.line, ATTACH);

    let writer = run_writer(dir.path(), s.scheme.clone(), s.cap_bytes, s.rotate_style, s.split_writes, s.total_lines, s.breath_every);
    let collected: Vec<String> = collect_until(&rx, DONE, Duration::from_secs(120)).into_iter().map(|e| e.line).collect();
    let generations = writer.join().expect("the writer panicked");
    assert!(generations >= 3, "{}: the scenario is too tame, only {generations} generations finished", s.name);
    set.stop_all();
    collected
}

#[test]
fn indexed_clean_rotation_loses_nothing() {
    let (scheme, active_mode, pattern) = indexed_scheme();
    let total = 5_000;
    let collected = run_scenario(Scenario {
        name: "indexed/clean",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::CleanClose,
        split_writes: false,
        total_lines: total,
        cap_bytes: 2_000,
        breath_every: 50,
        cfg: cfg(10, 25, 60),
    });
    assert_exact("indexed/clean", &collected, &expected_sequence(total));
}

#[test]
fn indexed_straggler_and_split_writes_lose_nothing() {
    let (scheme, active_mode, pattern) = indexed_scheme();
    let total = 3_000;
    let collected = run_scenario(Scenario {
        name: "indexed/straggler+split",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::LateFinalWrite,
        split_writes: true,
        total_lines: total,
        cap_bytes: 1_500,
        breath_every: 40,
        cfg: cfg(10, 25, 400), // the grace must comfortably exceed the straggle delay, under a loaded suite too
    });
    assert_exact("indexed/straggler+split", &collected, &expected_sequence(total));
}

#[test]
fn indexed_generation_bursts_faster_than_the_rescan_lose_nothing() {
    let (scheme, active_mode, pattern) = indexed_scheme();
    let total = 3_000;
    let collected = run_scenario(Scenario {
        name: "indexed/burst",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::CleanClose,
        split_writes: false,
        total_lines: total,
        // Tiny caps, a flat-out writer and a slow rescan: several generations are born inside
        // one rescan window, the missed-generation catch-up path.
        cap_bytes: 250,
        breath_every: 0,
        cfg: cfg(15, 80, 60),
    });
    assert_exact("indexed/burst", &collected, &expected_sequence(total));
}

#[test]
fn fixed_name_clean_rotation_loses_nothing() {
    let (scheme, active_mode, pattern) = fixed_scheme();
    let total = 2_000;
    let collected = run_scenario(Scenario {
        name: "fixed/clean",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::CleanClose,
        split_writes: false,
        total_lines: total,
        cap_bytes: 1_500,
        breath_every: 50,
        cfg: cfg(10, 25, 60),
    });
    assert_exact("fixed/clean", &collected, &expected_sequence(total));
}

#[test]
fn fixed_name_straggler_into_the_renamed_file_loses_nothing() {
    let (scheme, active_mode, pattern) = fixed_scheme();
    let total = 1_500;
    let collected = run_scenario(Scenario {
        name: "fixed/straggler",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::LateFinalWrite,
        split_writes: true,
        total_lines: total,
        cap_bytes: 1_200,
        breath_every: 40,
        cfg: cfg(10, 25, 400), // the grace must comfortably exceed the straggle delay, under a loaded suite too
    });
    assert_exact("fixed/straggler", &collected, &expected_sequence(total));
}

#[test]
fn fixed_name_generation_bursts_lose_nothing() {
    let (scheme, active_mode, pattern) = fixed_scheme();
    let total = 2_000;
    let collected = run_scenario(Scenario {
        name: "fixed/burst",
        scheme,
        active_mode,
        pattern,
        rotate_style: RotateStyle::CleanClose,
        split_writes: false,
        total_lines: total,
        cap_bytes: 250,
        breath_every: 0,
        cfg: cfg(15, 80, 60),
    });
    assert_exact("fixed/burst", &collected, &expected_sequence(total));
}

/// A family whose first file appears only after the tail attached is all new content, the
/// generations that retire before the rescan notices them included.
#[test]
fn a_family_born_after_attach_is_captured_whole_in_both_modes() {
    for (scheme, active_mode, pattern, name) in [
        {
            let (s, a, p) = fixed_scheme();
            (s, a, p, "fixed/born-after-attach")
        },
        {
            let (s, a, p) = indexed_scheme();
            (s, a, p, "indexed/born-after-attach")
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (set, rx) = spawn_tail(dir.path(), pattern, active_mode, cfg(10, 40, 60), "born");
        // Ample time for the first (empty) resolve, whatever the scheduler is doing.
        std::thread::sleep(Duration::from_millis(1500));
        let total = 300;
        let writer = run_writer(dir.path(), scheme, 400, RotateStyle::CleanClose, false, total, 0);
        let collected: Vec<String> = collect_until(&rx, DONE, Duration::from_secs(60)).into_iter().map(|e| e.line).collect();
        let generations = writer.join().expect("the writer panicked");
        assert!(generations >= 3, "only {generations} generations finished");
        set.stop_all();
        assert_exact(name, &collected, &expected_sequence(total));
    }
}

/// A family that predates the session: its retirees are history, only the active file is
/// followed, from its end or its beginning as configured.
#[test]
fn a_preexisting_family_is_followed_from_the_active_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = FamilyWriter::new(dir.path(), NamingScheme::Indexed { prefix: "pre_".into() }, 60);
    for n in 0..30 {
        w.write_line(&format!("H{n:03}"));
    }
    assert!(w.generations_finished() >= 3);
    // The active file holds the lines since the last rotation.
    let active = std::fs::read_to_string(w.active_path()).unwrap();
    let active_lines: Vec<String> = active.lines().map(String::from).collect();

    for (start, expect_history) in [(FamilyStart::ActiveBegin, true), (FamilyStart::ActiveEnd, false)] {
        let spec = FamilySpec::new(dir.path(), r"pre_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
        let (mut set, rx) = SourceSet::new();
        let mut c = cfg(10, 25, 60);
        c.start = start;
        set.spawn(Box::new(FamilyTailSource::new("fam", "pre", spec, c))).unwrap();
        if expect_history {
            for expected in &active_lines {
                assert_eq!(&rx.recv_timeout(Duration::from_secs(10)).unwrap().line, expected);
            }
        }
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "history leaked");
        w.write_line("NEW");
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().line, "NEW");
        set.stop_all();
    }
    w.finish();
}

// ---- the bench's emitters: the files a plant writes --------------------------------------------

fn emitter(name: &str, file: &str, max_lines: u64, encoding: Encoding) -> EmitterConfig {
    EmitterConfig {
        name: name.into(),
        template: "{ts} SCAN code={code} weight={weight}".into(),
        ts_format: "%Y-%m-%d %H:%M:%S%.3f".into(),
        values: vec![
            ValueField { name: "code".into(), low: 1000.0, high: 9999.0, decimals: 0 },
            ValueField { name: "weight".into(), low: 0.1, high: 30.0, decimals: 2 },
        ],
        interval_ms: 1,
        file: file.into(),
        max_lines,
        encoding,
        seed: Some(7),
    }
}

/// Every line of every file of the family on disk, in family order, decoded per the encoding.
fn lines_on_disk(spec: &FamilySpec, encoding: Encoding) -> Vec<String> {
    let mut out = Vec::new();
    for f in spec.resolve().unwrap() {
        let bytes = std::fs::read(&f.path).unwrap();
        let text = match encoding {
            Encoding::Utf8 => String::from_utf8(bytes).unwrap(),
            Encoding::Utf16le => {
                assert_eq!(&bytes[..2], &[0xFF, 0xFE], "{}: a byte-order mark first", f.file_name);
                let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                String::from_utf16(&units).unwrap()
            }
        };
        out.extend(text.lines().map(String::from));
    }
    out
}

/// The bench's emitter rotates on a line count into dated, indexed names; the family spec is the
/// emitter's own `family_shape`, and the tail, attached before the first file, follows every
/// rotation: what it emitted is exactly what is on disk, in order, with nothing lost or doubled.
fn follow_the_emitter(encoding: Encoding, file: &str, max_lines: u64, run_for: Duration) {
    let dir = tempfile::tempdir().unwrap();
    let shape = family_shape(file).expect("the pattern has a family shape");
    let spec = FamilySpec::new(dir.path(), &shape.regex, Some(&shape.date_format), ActiveMode::NewestByOrder).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("plant", "plant", spec.clone(), cfg(10, 25, 60)))).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let handle = emit::spawn(emitter("plant", file, max_lines, encoding), dir.path().to_path_buf(), 0).unwrap();
    std::thread::sleep(run_for);
    handle.join();
    let status = emit::spawn(emitter("probe", "probe_%Y.log", 0, Encoding::Utf8), dir.path().join("probe"), 1).unwrap();
    status.join();
    let on_disk = lines_on_disk(&spec, encoding);
    let files = spec.resolve().unwrap();
    assert!(files.len() >= 3, "the emitter rotated at least twice: {}", files.len());
    assert!(on_disk.len() as u64 >= max_lines * 2, "{} lines written", on_disk.len());

    let mut got: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while got.len() < on_disk.len() {
        let remaining = deadline.checked_duration_since(Instant::now()).unwrap_or_else(|| {
            panic!("timed out at {}/{} lines (last {:?})", got.len(), on_disk.len(), got.last())
        });
        match rx.recv_timeout(remaining.min(Duration::from_secs(5))) {
            Ok(ev) => {
                assert_eq!(ev.seq, got.len() as u64, "the sequence is dense");
                got.push(ev.line);
            }
            Err(e) => panic!("stalled at {}/{}: {e:?}", got.len(), on_disk.len()),
        }
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "more lines than the files hold");
    assert_exact(&format!("emitter/{}", encoding.label()), &got, &on_disk);
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
}

#[test]
fn the_bench_emitter_rotating_on_a_line_count_is_followed_whole_in_utf8() {
    follow_the_emitter(Encoding::Utf8, "%Y%m%d_scanner{n}.log", 40, Duration::from_millis(1500));
}

#[test]
fn the_bench_emitter_rotating_on_a_line_count_is_followed_whole_in_utf16() {
    follow_the_emitter(Encoding::Utf16le, "%d%m%Y_Trace{n}.txt", 25, Duration::from_millis(1200));
}

/// The emitter's family shape resolves exactly the names the emitter writes, in the order it
/// wrote them, with the date read back: the bench's shape and the gateway's reader agree.
#[test]
fn the_emitter_s_family_shape_resolves_its_files_in_rotation_order() {
    let dir = tempfile::tempdir().unwrap();
    let file = "%Y%m%d_scanner{n}.log";
    let handle = emit::spawn(emitter("s", file, 5, Encoding::Utf8), dir.path().to_path_buf(), 0).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    handle.join();
    let shape = family_shape(file).unwrap();
    assert_eq!(shape.regex, r"(?P<date>\d{4}\d{2}\d{2})_scanner(?P<idx>\d+)\.log");
    assert_eq!(shape.date_format, "%Y%m%d");
    let spec = FamilySpec::new(dir.path(), &shape.regex, Some(&shape.date_format), ActiveMode::NewestByOrder).unwrap();
    let files = spec.resolve().unwrap();
    assert!(files.len() >= 3, "{files:?}");
    let today = chrono::Local::now().date_naive();
    for (i, f) in files.iter().enumerate() {
        assert_eq!(f.key.idx, Some(i as u64 + 1), "{}", f.file_name);
        assert_eq!(f.key.date.unwrap().date(), today, "{}", f.file_name);
        assert_eq!(f.is_active, i + 1 == files.len());
    }
    // Every file but the active one holds exactly the rotation count.
    for f in &files[..files.len() - 1] {
        assert_eq!(std::fs::read_to_string(&f.path).unwrap().lines().count(), 5, "{}", f.file_name);
    }
}

// ---- the start's attach, the keys, the hours ---------------------------------------------------

fn from_the_end() -> FamilyTailConfig {
    FamilyTailConfig { start: FamilyStart::ActiveEnd, ..cfg(25, 50, 60) }
}

/// A family's start returns once its folder has been listed, so a line written to the active file
/// right after it is read, never taken as history.
#[test]
fn a_family_start_returns_attached_so_a_line_written_right_after_is_read() {
    for i in 0..20 {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app_1.log"), "old\n").unwrap();
        std::fs::write(dir.path().join("app_2.log"), "history\n").unwrap();
        let spec = FamilySpec::new(dir.path(), r"app_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
        let (mut set, rx) = SourceSet::new();
        set.spawn(Box::new(FamilyTailSource::new("f", "family", spec, from_the_end()))).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(dir.path().join("app_2.log")).unwrap();
        std::io::Write::write_all(&mut f, b"new\n").unwrap();
        let ev = rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|e| panic!("round {i}: {e:?}"));
        assert_eq!(ev.line, "new", "round {i}");
        set.stop_all();
    }
}

/// A keyless pattern lists its members by modification time, then name; following it is refused:
/// a family is followed by its keys, and without them no rotation order holds against a straggler.
#[test]
fn a_keyless_family_is_listed_by_time_then_name_and_refused_for_following() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "b.log");
    touch(dir.path(), "a.log");
    let older = std::time::SystemTime::now() - Duration::from_secs(60);
    std::fs::File::options().write(true).open(dir.path().join("b.log")).unwrap().set_modified(older).unwrap();
    let spec = FamilySpec::new(dir.path(), r"[ab]\.log", None, ActiveMode::NewestByOrder).unwrap();
    let files = spec.resolve().unwrap();
    assert_eq!(names(&files), ["b.log", "a.log"], "the older first, whatever the names");
    assert!(files[1].is_active);
    // The same time: the name decides.
    std::fs::File::options().write(true).open(dir.path().join("a.log")).unwrap().set_modified(older).unwrap();
    assert_eq!(names(&spec.resolve().unwrap()), ["a.log", "b.log"]);
    // Following needs a key: refused by text, no worker started.
    assert!(!spec.keyed());
    let (mut set, _rx) = SourceSet::new();
    let err = set.spawn(Box::new(FamilyTailSource::new("k", "keyless", spec, from_the_end()))).err().expect("refused").to_string();
    assert_eq!(
        err,
        "family spec: the pattern has neither a `date` nor an `idx` group: a family is followed by its keys, and one without them has no rotation order to follow (a listing orders it by modification time, then name)"
    );
    assert!(set.handles().is_empty());
}

/// The emitter's hourly shape: an hour without its minutes reads back as the start of the hour,
/// so the next hour outranks every index of the one before, and its first file is followed; a
/// pattern whose date is not one run has no shape.
#[test]
fn an_hourly_shape_orders_the_hours_and_the_next_hour_is_followed() {
    use chrono::TimeZone;
    let dir = tempfile::tempdir().unwrap();
    let pattern = "trace_%Y%m%d_%H_{n}.log";
    let shape = family_shape(pattern).unwrap();
    assert_eq!((shape.regex.as_str(), shape.date_format.as_str()), (r"trace_(?P<date>\d{4}\d{2}\d{2}_\d{2})_(?P<idx>\d+)\.log", "%Y%m%d_%H"));
    let spec = FamilySpec::new(dir.path(), &shape.regex, Some(&shape.date_format), ActiveMode::NewestByOrder).unwrap();
    let ten = chrono::Local.with_ymd_and_hms(2026, 11, 8, 10, 15, 0).unwrap();
    let eleven = ten + chrono::Duration::minutes(50);
    let at_ten: Vec<String> = (1..=3).map(|i| emit::file_name_for(pattern, &ten, i).unwrap()).collect();
    let at_eleven = emit::file_name_for(pattern, &eleven, 1).unwrap();
    let key = |n: &str| spec.parse_key(n).unwrap();
    assert_eq!(key(&at_eleven).date, Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(11, 0, 0).unwrap()));
    assert!(key(&at_eleven) > key(&at_ten[2]), "{:?} over {:?}", key(&at_eleven), key(&at_ten[2]));
    for n in &at_ten {
        std::fs::write(dir.path().join(n), format!("{n}\n")).unwrap();
    }
    assert_eq!(names(&spec.resolve().unwrap()), at_ten.iter().map(String::as_str).collect::<Vec<_>>());
    // Followed from the end of the active ten o'clock file, the next hour's first file is read.
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("h", "hourly", spec, from_the_end()))).unwrap();
    std::fs::write(dir.path().join(&at_eleven), "H11_1\n").unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().line, "H11_1");
    set.stop_all();
    assert!(family_shape("%Y_x{n}_%m%d.log").is_none(), "two runs of date specifiers have no shape");
}

/// A name carries what it carries: a year alone is its first day, a month its first, so names of
/// every granularity order on one axis with the full stamps.
#[test]
fn a_name_carries_what_it_carries_and_orders_with_the_full_stamps() {
    let dir = tempfile::tempdir().unwrap();
    let key = |pattern: &str, fmt: &str, name: &str| {
        FamilySpec::new(dir.path(), pattern, Some(fmt), ActiveMode::NewestByOrder).unwrap().parse_key(name).unwrap().date.unwrap()
    };
    let at = |y: i32, m: u32, d: u32, h: u32| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap();
    assert_eq!(key(r"y(?P<date>\d{4})\.log", "%Y", "y2026.log"), at(2026, 1, 1, 0));
    assert_eq!(key(r"m(?P<date>\d{6})\.log", "%Y%m", "m202611.log"), at(2026, 11, 1, 0));
    assert_eq!(key(r"h(?P<date>\d{10})\.log", "%Y%m%d%H", "h2026110813.log"), at(2026, 11, 8, 13));
    assert_eq!(key(r"s(?P<date>\d{12})\.log", "%Y%m%d%H%M", "s202611081345.log"), at(2026, 11, 8, 13) + chrono::Duration::minutes(45));
}

/// A fixed-name family whose active file is not there at the start: the folder is listed, so
/// the start returns at once, and the active, when it comes, is new content read from its start.
#[test]
fn a_fixed_active_absent_at_the_start_is_new_content_when_it_comes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("old_1.log"), "R1\n").unwrap();
    let spec = FamilySpec::new(dir.path(), r"old_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
    let (mut set, rx) = SourceSet::new();
    let started = Instant::now();
    set.spawn(Box::new(FamilyTailSource::new("f", "fixed", spec, from_the_end()))).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2), "the start waited {:?}", started.elapsed());
    std::fs::write(dir.path().join("current.log"), "X\n").unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().line, "X");
    set.stop_all();
}

/// The keys must separate generations: a pre-existing family on which no key parses is refused
/// by text; members whose keys tie are followed by name, the lower name history (the listing
/// breaks the tie by time), so a writer that reuses a key is not followed.
#[test]
fn keys_that_tie_are_followed_by_name_and_keys_that_never_parse_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app_b.log"), "B1\n").unwrap();
    let spec = FamilySpec::new(dir.path(), r"app_(?P<date>[a-z])\.log", None, ActiveMode::NewestByOrder);
    assert!(spec.is_err(), "a date group needs its format");
    let spec = FamilySpec::new(dir.path(), r"app_(?P<date>[a-z])\.log", Some("%Y"), ActiveMode::NewestByOrder).unwrap();
    assert!(spec.keyed());
    let (mut set, _rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("n", "none", spec, from_the_end()))).unwrap();
    let ended = set.stop_all();
    assert_eq!(
        ended[0].1.as_ref().unwrap_err().to_string(),
        "family spec: the pattern's key groups parse on no member of the family: a family is followed by its keys, and these name none (check the date format against the names)"
    );
    // A reused index: the file of the lower name ranks below the active and is history.
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"[a-z]_(?P<idx>\d+)\.log", None, ActiveMode::NewestByOrder).unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FamilyTailSource::new("t", "tie", spec, from_the_end()))).unwrap();
    std::fs::write(dir.path().join("b_1.log"), "B1\n").unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().line, "B1");
    std::fs::write(dir.path().join("a_1.log"), "A1\n").unwrap();
    std::fs::write(dir.path().join("a_2.log"), "A2\n").unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().line, "A2", "the tie at 1 is broken by name, and a_1 is history");
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    set.stop_all();
}

/// A twelve-hour hour without its minutes is the start of the hour too.
#[test]
fn a_twelve_hour_hour_without_minutes_is_the_start_of_the_hour() {
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"t_(?P<date>\d{8}_\d{2}[AP]M)\.log", Some("%Y%m%d_%I%p"), ActiveMode::NewestByOrder).unwrap();
    let key = |name: &str| spec.parse_key(name).unwrap().date.unwrap();
    let at = |h: u32| chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(h, 0, 0).unwrap();
    assert_eq!((key("t_20261108_10AM.log"), key("t_20261108_11AM.log"), key("t_20261108_01PM.log")), (at(10), at(11), at(13)));
}

/// A fixed-name active born after the start may retire before the worker opens it: the family
/// is followed from the start as one born after it, with the pre-start retirees as history, so
/// every generation born since is read, in order.
#[test]
fn a_fixed_active_born_after_the_start_keeps_the_generations_that_retire_before_it_is_opened() {
    for round in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old_1.log"), "R1\n").unwrap();
        let spec = FamilySpec::new(dir.path(), r"old_(?P<idx>\d+)\.log", None, ActiveMode::Fixed("current.log".into())).unwrap();
        // The watcher off, as over a share: the poll alone sees the births.
        let cfg = FamilyTailConfig { start: FamilyStart::ActiveEnd, use_notify: Some(false), ..cfg(100, 50, 60) };
        let (mut set, rx) = SourceSet::new();
        set.spawn(Box::new(FamilyTailSource::new("f", "fixed", spec, cfg))).unwrap();
        for (i, line) in ["G1", "G2", "G3"].iter().enumerate() {
            std::fs::write(dir.path().join("current.log"), format!("{line}\n")).unwrap();
            if i < 2 {
                std::fs::rename(dir.path().join("current.log"), dir.path().join(format!("old_{}.log", i + 2))).unwrap();
            }
        }
        let got: Vec<String> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|e| panic!("round {round}: {e:?}")).line).collect();
        assert_eq!(got, ["G1", "G2", "G3"], "round {round}");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "round {round}: nothing twice");
        set.stop_all();
    }
}

/// A padding modifier on the hour (`%-H`) is still an hour: the minutes are filled.
#[test]
fn a_padded_hour_specifier_still_gets_its_minutes_filled() {
    let dir = tempfile::tempdir().unwrap();
    let spec = FamilySpec::new(dir.path(), r"t_(?P<date>\d{8}_\d{1,2})\.log", Some("%Y%m%d_%-H"), ActiveMode::NewestByOrder).unwrap();
    let key = |name: &str| spec.parse_key(name).unwrap().date.unwrap();
    let at = |h: u32| chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(h, 0, 0).unwrap();
    assert_eq!((key("t_20261108_9.log"), key("t_20261108_10.log")), (at(9), at(10)));
    assert!(key("t_20261108_9.log") < key("t_20261108_10.log"));
}
