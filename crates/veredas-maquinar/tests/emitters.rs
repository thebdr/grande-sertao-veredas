//! The log emitters (contract: the emitters clause), from Veredas 1's `logsim` tests: the pure
//! parts (rendering, naming, rotation, the family shape) pinned exactly, and the worker against a
//! real directory.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, TimeZone};
use veredas_maquinar::emit::*;

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

/// A throwaway directory, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(case: &str) -> Dir {
        let path = std::env::temp_dir().join(format!("veredas-maquinar-emit-{}-{case}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg(pattern: &str, interval_ms: u64, max_lines: u64) -> EmitterConfig {
    EmitterConfig {
        name: "test".into(),
        template: "{ts};T={temp}".into(),
        ts_format: "%Y-%m-%d %H:%M:%S%.3f".into(),
        values: vec![ValueField { name: "temp".into(), low: 20.0, high: 22.0, decimals: 1 }],
        interval_ms,
        file: pattern.into(),
        max_lines,
        encoding: Encoding::Utf8,
        seed: None,
    }
}

/// Wait until `f` holds, or fail: polling beats a fixed sleep on a busy machine.
fn until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

#[test]
fn a_bad_datetime_format_renders_a_message_instead_of_panicking() {
    // chrono's `format(..).to_string()` panics here; the formats come from a file an operator
    // wrote.
    let now = at(2026, 7, 28, 14, 30);
    assert_eq!(safe_format(&now, "%Y-%m-%d"), Some("2026-07-28".to_string()));
    assert_eq!(safe_format(&now, "%Q"), None);
    assert_eq!(safe_format(&now, "%"), None, "a lone percent");
    assert_eq!(render_line("{ts} X", "%Q", &now, &[]), format!("{BAD_FORMAT} X"));
    assert_eq!(BAD_FORMAT, "(bad datetime format)");
}

#[test]
fn templates_substitute_values_and_show_typos_verbatim() {
    let now = at(2026, 7, 28, 14, 30);
    let values = vec![("temp".to_string(), 21.456, 1u8), ("press".to_string(), 1.0, 2u8)];
    assert_eq!(render_line("{ts} T={temp} P={press}", "%H:%M", &now, &values), "14:30 T=21.5 P=1.00");
    // An unknown name and an unclosed brace stay exactly as typed, so the line shows the mistake.
    assert_eq!(render_line("{temp} {nope} {unclosed", "%H:%M", &now, &values), "21.5 {nope} {unclosed");
    assert_eq!(render_line("", "%H:%M", &now, &values), "");
    assert_eq!(render_line("plain text", "%H:%M", &now, &values), "plain text");
    assert_eq!(render_line("a } b", "%H:%M", &now, &values), "a } b", "a stray close brace");
    assert_eq!(render_line("{}", "%H:%M", &now, &values), "{}", "an empty placeholder");
    assert_eq!(render_line("{temp}{temp}", "%H:%M", &now, &values), "21.521.5", "twice");
    // Zero decimals rounds; a negative value keeps its sign.
    let values = vec![("v".to_string(), -2.5, 0u8)];
    assert_eq!(render_line("{v}", "%H:%M", &now, &values), "-2");
}

#[test]
fn values_are_drawn_within_their_range_to_their_decimals_and_a_seed_replays_them() {
    use veredas_core::rng::Pcg32;
    let fields = vec![
        ValueField { name: "a".into(), low: 18.0, high: 24.0, decimals: 1 },
        ValueField { name: "b".into(), low: 5.0, high: 0.0, decimals: 0 },
    ];
    let mut rng = Pcg32::new(1, 1);
    let mut again = Pcg32::new(1, 1);
    for _ in 0..200 {
        let drawn = draw_values(&fields, &mut rng);
        assert_eq!(drawn, draw_values(&fields, &mut again), "the same seed draws the same");
        assert_eq!(drawn[0].0, "a");
        assert!((18.0..=24.0).contains(&drawn[0].1), "{}", drawn[0].1);
        assert_eq!((drawn[0].1 * 10.0).round() / 10.0, drawn[0].1, "one decimal");
        assert_eq!(drawn[0].2, 1);
        assert!((0.0..=5.0).contains(&drawn[1].1), "an inverted range is swapped: {}", drawn[1].1);
        assert_eq!(drawn[1].1.fract(), 0.0, "zero decimals is a whole number");
    }
    assert!(draw_values(&[], &mut rng).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Naming and rotation
// ---------------------------------------------------------------------------------------------

#[test]
fn file_names_carry_the_date_and_the_index_a_family_orders_by() {
    let now = at(2026, 7, 28, 14, 30);
    // The plant shape.
    assert_eq!(file_name_for("%d%m%Y_Trace{n}.txt", &now, 1).unwrap(), "28072026_Trace1.txt");
    assert_eq!(file_name_for("%d%m%Y_Trace{n}.txt", &now, 12).unwrap(), "28072026_Trace12.txt");
    // No `{n}`: index 1 is the bare name, later ones gain a suffix before the pattern's last
    // dot (so a dotted date cannot split it).
    assert_eq!(file_name_for("Trace_%Y%m%d.log", &now, 1).unwrap(), "Trace_20260728.log");
    assert_eq!(file_name_for("Trace_%Y%m%d.log", &now, 3).unwrap(), "Trace_20260728_3.log");
    // A dotted date format has dots that are not an extension: the index must not land inside
    // the timestamp.
    assert_eq!(file_name_for("log_%d.%m.%Y", &now, 2).unwrap(), "log_28.07.2026_2");
    assert_eq!(file_name_for("%d.%m.%Y_Trace.txt", &now, 2).unwrap(), "28.07.2026_Trace_2.txt");
    // A long or non-alphanumeric tail is no extension.
    assert_eq!(file_name_for("a.backup2026", &now, 2).unwrap(), "a.backup2026_2");
    assert_eq!(file_name_for("a.b-c", &now, 2).unwrap(), "a.b-c_2");
    assert_eq!(file_name_for("a.", &now, 2).unwrap(), "a._2");
    // `{n}` wherever it sits, more than once, and index 0 as typed.
    assert_eq!(file_name_for("{n}_%Y.log", &now, 7).unwrap(), "7_2026.log");
    assert_eq!(file_name_for("{n}-{n}.log", &now, 3).unwrap(), "3-3.log");
    assert_eq!(file_name_for("t{n}.log", &now, 0).unwrap(), "t0.log");
    // Surrounding whitespace is not part of a name.
    assert_eq!(file_name_for("  t{n}.log  ", &now, 1).unwrap(), "t1.log");
    // The errors a pattern can produce, by name.
    assert_eq!(file_name_for("", &now, 1), Err(NameError::Empty));
    assert_eq!(file_name_for("   ", &now, 1), Err(NameError::Empty));
    assert_eq!(file_name_for("%Q.log", &now, 1), Err(NameError::BadFormat));
    assert_eq!(file_name_for("%H:%M.log", &now, 1), Err(NameError::InvalidChars("`:`".into())));
    assert_eq!(file_name_for("a/b{n}.log", &now, 1), Err(NameError::InvalidChars("`/`".into())));
    assert_eq!(file_name_for("a\tb", &now, 1), Err(NameError::InvalidChars("`\t`".into())));
    for c in ['<', '>', '"', '\\', '|', '?', '*'] {
        assert!(matches!(file_name_for(&format!("a{c}b"), &now, 1), Err(NameError::InvalidChars(_))), "{c}");
    }
    // Windows drops a trailing dot or space, so the file on disk would not be the name reported.
    assert_eq!(file_name_for("%Y{n}.", &now, 1), Err(NameError::BadEnding));
    assert_eq!(file_name_for("log_%Y%m%d{n}.", &now, 3), Err(NameError::BadEnding));
    assert_eq!(file_name_for("%Y%t", &now, 1), Err(NameError::InvalidChars("`\t`".into())), "a tab is a control character");
    // Names the system reserves whatever the extension.
    assert_eq!(file_name_for("con.txt", &now, 1), Err(NameError::Reserved("con.txt".into())));
    assert_eq!(file_name_for("NUL", &now, 1), Err(NameError::Reserved("NUL".into())));
    assert_eq!(file_name_for("Com1.log", &now, 1), Err(NameError::Reserved("Com1.log".into())));
    assert_eq!(file_name_for("lpt9.%Y", &now, 1), Err(NameError::Reserved("lpt9.2026".into())));
    assert_eq!(file_name_for("aux_%Y.log", &now, 1).unwrap(), "aux_2026.log", "a longer stem is not reserved");
    assert_eq!(file_name_for("con{n}.log", &now, 1).unwrap(), "con1.log");
    assert_eq!(NameError::Empty.to_string(), "the file pattern is empty");
    assert_eq!(NameError::BadFormat.to_string(), "the file pattern is not a valid datetime format");
    assert_eq!(NameError::InvalidChars("`:`".into()).to_string(), "a file name cannot contain `:`");
    assert_eq!(NameError::BadEnding.to_string(), "a file name cannot end with `.` or a space");
    assert_eq!(NameError::Reserved("NUL".into()).to_string(), "`NUL` is a name the system reserves");
}

#[test]
fn a_rotor_skips_to_the_next_index_without_writing() {
    let now = at(2026, 7, 28, 9, 0);
    let mut rotor = Rotor::new(1);
    let (name, _) = rotor.file_for("t{n}.log", &now, 0).unwrap();
    assert_eq!(name, "t1.log");
    rotor.note_line();
    assert_eq!(rotor.skip("t{n}.log", &now).unwrap(), "t2.log");
    assert_eq!((rotor.idx(), rotor.lines_in_file()), (2, 0));
    assert_eq!(rotor.skip("t{n}.log", &now).unwrap(), "t3.log");
    assert_eq!(rotor.skip("%Q", &now), Err(NameError::BadFormat));
    assert_eq!(MAX_SKIPS, 10_000);
}

#[test]
fn rotation_advances_on_count_and_resets_on_a_new_day() {
    let mut rotor = Rotor::new(1);
    let day1 = at(2026, 7, 28, 9, 0);
    let pattern = "%d%m%Y_Trace{n}.txt";

    let (name, rotated) = rotor.file_for(pattern, &day1, 2).unwrap();
    assert_eq!((name.as_str(), rotated), ("28072026_Trace1.txt", true), "the first file is a rotation");
    rotor.note_line();
    let (name, rotated) = rotor.file_for(pattern, &day1, 2).unwrap();
    assert_eq!((name.as_str(), rotated), ("28072026_Trace1.txt", false));
    rotor.note_line();
    assert_eq!(rotor.lines_in_file(), 2);
    // Two lines written, max is 2: the next one starts a sibling.
    let (name, rotated) = rotor.file_for(pattern, &day1, 2).unwrap();
    assert_eq!((name.as_str(), rotated), ("28072026_Trace2.txt", true));
    assert_eq!((rotor.idx(), rotor.lines_in_file()), (2, 0));
    rotor.note_line();

    // Later the same day, the same file.
    let later = at(2026, 7, 28, 23, 59);
    let (name, rotated) = rotor.file_for(pattern, &later, 2).unwrap();
    assert_eq!((name.as_str(), rotated), ("28072026_Trace2.txt", false));

    // A new day starts again at index 1, which is what makes (date, idx) ordering meaningful.
    let day2 = at(2026, 7, 29, 0, 0);
    let (name, rotated) = rotor.file_for(pattern, &day2, 2).unwrap();
    assert_eq!((name.as_str(), rotated), ("29072026_Trace1.txt", true));
    assert_eq!((rotor.idx(), rotor.lines_in_file()), (1, 0));

    // A bad pattern is refused by the rotor too, and leaves it where it was.
    assert_eq!(rotor.file_for("%Q{n}", &day2, 2), Err(NameError::BadFormat));
    assert_eq!(rotor.idx(), 1);
}

#[test]
fn a_rotor_started_mid_family_keeps_its_index() {
    // A restarted emitter probes past the files it already wrote and hands the rotor that index.
    // Treating the first line as a name change would reset it to 1 and append to the previous
    // run's file, silently corrupting a family's ordering.
    let now = at(2026, 7, 28, 9, 0);
    let mut rotor = Rotor::new(4);
    let (name, _) = rotor.file_for("%d%m%Y_Trace{n}.txt", &now, 10).unwrap();
    assert_eq!(name, "28072026_Trace4.txt");
    assert_eq!(rotor.idx(), 4);
    // A genuine date change still resets.
    let tomorrow = at(2026, 7, 29, 9, 0);
    let (name, rotated) = rotor.file_for("%d%m%Y_Trace{n}.txt", &tomorrow, 10).unwrap();
    assert_eq!((name.as_str(), rotated), ("29072026_Trace1.txt", true));
    // Index 0 is read as 1.
    assert_eq!(Rotor::new(0).idx(), 1);
}

#[test]
fn max_lines_zero_never_rotates_and_one_writes_a_file_per_line() {
    let now = at(2026, 7, 28, 9, 0);
    let mut unlimited = Rotor::new(1);
    for _ in 0..50 {
        let (name, _) = unlimited.file_for("t{n}.log", &now, 0).unwrap();
        assert_eq!(name, "t1.log");
        unlimited.note_line();
    }
    assert_eq!(unlimited.lines_in_file(), 50);
    let mut per_line = Rotor::new(1);
    let mut names = Vec::new();
    for _ in 0..3 {
        let (name, _) = per_line.file_for("t{n}.log", &now, 1).unwrap();
        names.push(name);
        per_line.note_line();
    }
    assert_eq!(names, ["t1.log", "t2.log", "t3.log"]);
}

#[test]
fn the_family_shape_describes_what_the_pattern_writes() {
    // Two strings a family reader needs; they must describe the real output.
    let shape = family_shape("%d%m%Y_Trace{n}.txt").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{2}\\d{2}\\d{4})_Trace(?P<idx>\\d+)\\.txt");
    assert_eq!(shape.date_format, "%d%m%Y");

    // Without `{n}` the index is optional where the emitter inserts it (before the extension):
    // index 1 is the bare name, its rotated siblings carry `_<n>` (Veredas 1's shape matched the
    // bare name alone, so a family rotated on count lost every sibling).
    let shape = family_shape("Trace_%Y%m%d.log").unwrap();
    assert_eq!(shape.regex, "Trace_(?P<date>\\d{4}\\d{2}\\d{2})(?:_(?P<idx>\\d+))?\\.log");
    assert_eq!(shape.date_format, "%Y%m%d");
    let shape = family_shape("log_%d.%m.%Y").unwrap();
    assert_eq!(shape.regex, "log_(?P<date>\\d{2}\\.\\d{2}\\.\\d{4})(?:_(?P<idx>\\d+))?");
    let shape = family_shape("  %Y.backup2026  ").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{4})\\.backup2026(?:_(?P<idx>\\d+))?", "no extension, trimmed");

    // Separators inside the date belong to its group; the escape of a dot is kept.
    let shape = family_shape("log_%d.%m.%Y_{n}").unwrap();
    assert_eq!(shape.regex, "log_(?P<date>\\d{2}\\.\\d{2}\\.\\d{4})_(?P<idx>\\d+)");
    assert_eq!(shape.date_format, "%d.%m.%Y");
    let shape = family_shape("%Y-%m-%d_%H%M.txt").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{4}-\\d{2}-\\d{2}_\\d{2}\\d{2})(?:_(?P<idx>\\d+))?\\.txt");
    assert_eq!(shape.date_format, "%Y-%m-%d_%H%M");
    // Two-digit years and seconds.
    let shape = family_shape("%y%m%d%H%M%S.log").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{2}\\d{2}\\d{2}\\d{2}\\d{2}\\d{2})(?:_(?P<idx>\\d+))?\\.log");
    assert_eq!(shape.date_format, "%y%m%d%H%M%S");
    // A regex metacharacter in the literal part is escaped.
    let shape = family_shape("a+b(%Y).log").unwrap();
    assert_eq!(shape.regex, "a\\+b\\((?P<date>\\d{4})\\)(?:_(?P<idx>\\d+))?\\.log");
    assert_eq!(shape.date_format, "%Y");
    // A specifier beyond a longer literal is outside the date: digits of the name, not of the
    // date (a format `%Y%m` would not parse `2026_T07`).
    let shape = family_shape("%Y_T%m.log").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{4})_T\\d{2}(?:_(?P<idx>\\d+))?\\.log");
    assert_eq!(shape.date_format, "%Y");
    // The index between two specifiers ends the date too.
    let shape = family_shape("%Y{n}%m.log").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{4})(?P<idx>\\d+)\\d{2}\\.log");
    assert_eq!(shape.date_format, "%Y");
    // A literal that is not ASCII stands for itself (an escaped `ç` is no regex); only the
    // metacharacters are escaped.
    let shape = family_shape("Medição_%Y%m%d_{n}.log").unwrap();
    assert_eq!(shape.regex, "Medição_(?P<date>\\d{4}\\d{2}\\d{2})_(?P<idx>\\d+)\\.log");
    let shape = family_shape("a&b~c#d$e^f[g]h{n}%Y").unwrap();
    assert_eq!(shape.regex, "a\\&b\\~c\\#d\\$e\\^f\\[g\\]h(?P<idx>\\d+)(?P<date>\\d{4})");
    // A second `{n}` renders the same number: matched as digits, since a group name is used once.
    let shape = family_shape("%Y%m%d_{n}_{n}.log").unwrap();
    assert_eq!(shape.regex, "(?P<date>\\d{4}\\d{2}\\d{2})_(?P<idx>\\d+)_\\d+\\.log");

    // What the shape pins: the regex matches the names the pattern renders, with the groups
    // holding the date (parseable with the format) and the index.
    let now = at(2026, 7, 28, 9, 0);
    for pattern in [
        "%d%m%Y_Trace{n}.txt",
        "Trace_%Y%m%d.log",
        "log_%d.%m.%Y_{n}",
        "log_%d.%m.%Y",
        "%Y-%m-%d_%H%M.txt",
        "%Y_T%m.log",
        "a+b(%Y).log",
        "%Y.backup2026",
        "Medição_%Y%m%d_{n}.log",
        "%Y%m%d_{n}_{n}.log",
        "a&b~c#d$e^f[g]h{n}%Y",
        "{n}%d%m%Y",
    ] {
        let shape = family_shape(pattern).unwrap();
        for idx in [1u32, 2, 10] {
            let name = file_name_for(pattern, &now, idx).unwrap();
            let caps = captures(&shape.regex, &name).unwrap_or_else(|| panic!("{} does not match {name}", shape.regex));
            if pattern.contains("{n}") || idx > 1 {
                assert_eq!(caps["idx"], idx.to_string(), "{pattern} at {idx}");
            } else {
                assert!(!caps.contains_key("idx"), "{pattern}: the bare name has no index: {caps:?}");
            }
            let rendered = safe_format(&now, &shape.date_format).unwrap();
            assert_eq!(caps["date"], rendered, "{pattern}");
            // And the date parses back with the format: the property a family reader rests on
            // (Veredas 1's shape put a dotted date's separators in the group but not in the
            // format, so `28.07.2026` never parsed with `%d%m%Y`).
            let mut parsed = chrono::format::Parsed::new();
            let items = chrono::format::StrftimeItems::new(&shape.date_format);
            let ok = chrono::format::parse(&mut parsed, &caps["date"], items);
            assert!(ok.is_ok(), "{pattern}: `{}` with `{}`: {ok:?}", caps["date"], shape.date_format);
            assert_eq!(parsed.year, Some(2026), "{pattern}: the year read back");
        }
        // And a name of another day, or an unrelated file, is not matched.
        assert!(captures(&shape.regex, "unrelated.txt").is_none(), "{pattern}");
    }

    // A pattern with no date has no family shape, and an exotic specifier, a placeholder other
    // than `{n}`, or a forbidden character is refused rather than guessed.
    assert_eq!(family_shape("plain{n}.log"), None);
    assert_eq!(family_shape("%j_%Y.log"), None);
    assert_eq!(family_shape("%Y_{x}.log"), None);
    assert_eq!(family_shape("%Y:{n}.log"), None);
    assert_eq!(family_shape("%Y%"), None, "a trailing percent");
    assert_eq!(family_shape(""), None);
}

/// The named groups of `pattern` over the whole of `text`. The `regex` crate is not a dependency
/// of the bench; core's is, through the capture engine, which compiles a payload pattern and
/// extracts its named groups.
fn captures(pattern: &str, text: &str) -> Option<std::collections::HashMap<String, String>> {
    use veredas_core::telegram::capture::{Capture, CaptureSpec};
    let capture = Capture::new(&CaptureSpec { payload: Some(format!("^{pattern}$")), ..Default::default() });
    assert!(capture.warnings.is_empty(), "{:?}", capture.warnings);
    capture.extract(text.as_bytes(), veredas_core::telegram::Endian::Little)
}

// ---------------------------------------------------------------------------------------------
// The worker, against a real directory
// ---------------------------------------------------------------------------------------------

#[test]
fn an_emitter_writes_rotating_files_a_plant_would_recognise() {
    let dir = Dir::new("rotating");
    let handle = spawn(cfg("%d%m%Y_Trace{n}.txt", 10, 3), dir.path().to_path_buf(), 1).unwrap();

    // Three lines per file, so a handful of files must appear.
    until("three files", Duration::from_secs(10), || files_in(dir.path()).len() >= 3);
    handle.stop();
    until("the worker to stop", Duration::from_secs(3), || handle.is_finished());

    let names = files_in(dir.path());
    assert!(names.len() >= 3, "{names:?}");
    let today = safe_format(&Local::now(), "%d%m%Y").unwrap();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(*name, format!("{today}_Trace{}.txt", i + 1), "names carry the date and a running index");
    }

    let status = handle.status();
    assert_eq!(status.last_error, None);
    assert!(status.total_lines >= 6, "{status:?}");
    assert_eq!(status.current_file.as_deref(), Some(names.last().unwrap().as_str()));

    // Every file but the last (still filling) holds exactly max_lines.
    let mut total = 0u64;
    for (i, name) in names.iter().enumerate() {
        let bytes = std::fs::read(dir.path().join(name)).unwrap();
        let text = String::from_utf8(bytes.clone()).expect("utf-8");
        assert!(text.ends_with("\r\n"), "CRLF, like a plant logger: {name}");
        assert!(!text.contains("\n\n") && !text.contains("\r\r"), "one line ending per line");
        let lines: Vec<&str> = text.lines().collect();
        total += lines.len() as u64;
        if i + 1 < names.len() {
            assert_eq!(lines.len(), 3, "{name} should be full");
        } else {
            assert!((1..=3).contains(&lines.len()), "{name} is filling: {}", lines.len());
            assert_eq!(status.lines_in_file, lines.len() as u64);
        }
        for line in lines {
            // The timestamp reads back with the format that wrote it, the property a consumer
            // depends on to parse a line's time.
            let stamp = line.split(';').next().unwrap();
            chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f")
                .unwrap_or_else(|e| panic!("`{stamp}` did not parse back: {e}"));
            let value: f64 = line.split("T=").nth(1).unwrap().parse().unwrap();
            assert!((20.0..=22.0).contains(&value), "{value} out of range");
        }
    }
    assert_eq!(total, status.total_lines, "status matches what is on disk");
}

#[test]
fn a_seeded_emitter_replays_the_same_values() {
    let dir = Dir::new("seeded");
    let mut seeded = cfg("run{n}.log", 1, 0);
    seeded.seed = Some(42);
    seeded.template = "{temp}".into();
    seeded.values.push(ValueField { name: "p".into(), low: 0.0, high: 100.0, decimals: 3 });
    seeded.template = "{temp};{p}".into();
    let first = spawn(seeded.clone(), dir.path().join("a"), 7).unwrap();
    let second = spawn(seeded, dir.path().join("b"), 7).unwrap();
    until("twenty lines each", Duration::from_secs(10), || {
        first.status().total_lines >= 20 && second.status().total_lines >= 20
    });
    first.join();
    second.join();
    let a: Vec<String> = std::fs::read_to_string(dir.path().join("a/run1.log")).unwrap().lines().take(20).map(String::from).collect();
    let b: Vec<String> = std::fs::read_to_string(dir.path().join("b/run1.log")).unwrap().lines().take(20).map(String::from).collect();
    assert_eq!(a, b, "the same seed and stream write the same values");
    assert!(a.iter().any(|l| l != &a[0]), "and the values move: {a:?}");
}

#[test]
fn restarting_never_appends_to_a_file_it_already_wrote() {
    // A restarted emitter continuing yesterday's file would defeat the point of rotation, and
    // would silently corrupt a family's ordering.
    let dir = Dir::new("restart");
    let first = spawn(cfg("run{n}.log", 10, 2), dir.path().to_path_buf(), 1).unwrap();
    until("the first file", Duration::from_secs(10), || !files_in(dir.path()).is_empty());
    first.stop();
    until("stop", Duration::from_secs(3), || first.is_finished());
    let before = files_in(dir.path());
    let lengths: Vec<usize> = before.iter().map(|n| std::fs::read(dir.path().join(n)).unwrap().len()).collect();

    let second = spawn(cfg("run{n}.log", 10, 2), dir.path().to_path_buf(), 2).unwrap();
    until("a new file", Duration::from_secs(10), || files_in(dir.path()).len() > before.len());
    second.stop();
    until("stop", Duration::from_secs(3), || second.is_finished());

    for (name, len) in before.iter().zip(lengths) {
        assert_eq!(std::fs::read(dir.path().join(name)).unwrap().len(), len, "{name} must not grow after a restart");
    }
    // The second run started at the first free index.
    assert_eq!(second.status().current_file.as_deref().map(|n| n.starts_with("run")), Some(true));
    let next = format!("run{}.log", before.len() + 1);
    assert!(files_in(dir.path()).contains(&next), "{:?}", files_in(dir.path()));
}

#[test]
fn a_restart_never_appends_past_a_gap_in_the_family() {
    // A collector archived run1; run2 and run3 are still there. Probing to the first free index
    // alone would take run1 and then, at the first rotation, append to run2.
    let dir = Dir::new("gap");
    std::fs::write(dir.path().join("run2.log"), b"OLD2\r\n").unwrap();
    std::fs::write(dir.path().join("run3.log"), b"OLD3\r\n").unwrap();
    let handle = spawn(cfg("run{n}.log", 5, 1), dir.path().to_path_buf(), 1).unwrap();
    until("three new files", Duration::from_secs(10), || files_in(dir.path()).len() >= 5);
    handle.join();
    assert_eq!(std::fs::read(dir.path().join("run2.log")).unwrap(), b"OLD2\r\n", "run2 untouched");
    assert_eq!(std::fs::read(dir.path().join("run3.log")).unwrap(), b"OLD3\r\n", "run3 untouched");
    let names = files_in(dir.path());
    assert_eq!(&names[..5], ["run1.log", "run2.log", "run3.log", "run4.log", "run5.log"]);
    for name in ["run1.log", "run4.log", "run5.log"] {
        assert_eq!(std::fs::read_to_string(dir.path().join(name)).unwrap().lines().count(), 1, "{name}: one line per file");
    }
}

#[test]
fn one_clock_decides_the_file_and_stamps_the_line_across_midnight() {
    // The writer apart from its thread, driven at chosen instants: a line at 23:59 lands in that
    // day's file with that day's stamp, the next at 00:00 in the new day's, and a file of the new
    // day that already exists is skipped, not appended to.
    let dir = Dir::new("midnight");
    std::fs::write(dir.path().join("29072026_Trace1.txt"), b"yesterday's run\r\n").unwrap();
    let mut c = cfg("%d%m%Y_Trace{n}.txt", 0, 0);
    c.ts_format = "%Y-%m-%d %H:%M:%S%.3f".into();
    c.template = "{ts};T={temp}".into();
    let mut writer = Writer::new(c, dir.path().to_path_buf(), 1);
    writer.write_at(&at(2026, 7, 28, 23, 59));
    assert_eq!(writer.status().current_file.as_deref(), Some("28072026_Trace1.txt"));
    assert_eq!(writer.status().last_error, None);
    writer.write_at(&at(2026, 7, 29, 0, 0));
    assert_eq!(writer.status().current_file.as_deref(), Some("29072026_Trace2.txt"), "the new day, past the file that exists");
    assert_eq!((writer.status().total_lines, writer.status().lines_in_file), (2, 1));
    let late = std::fs::read_to_string(dir.path().join("28072026_Trace1.txt")).unwrap();
    assert!(late.starts_with("2026-07-28 23:59:00.000;T="), "{late}");
    let early = std::fs::read_to_string(dir.path().join("29072026_Trace2.txt")).unwrap();
    assert!(early.starts_with("2026-07-29 00:00:00.000;T="), "{early}");
    assert_eq!(std::fs::read(dir.path().join("29072026_Trace1.txt")).unwrap(), b"yesterday's run\r\n");
    // A bad pattern leaves the writer where it was, reporting it.
    let mut bad = Writer::new(cfg("%Q{n}.log", 0, 0), dir.path().to_path_buf(), 2);
    bad.write_at(&at(2026, 7, 29, 0, 0));
    assert_eq!(bad.status().last_error, Some(EmitError::Name(NameError::BadFormat)));
    assert_eq!(bad.status().total_lines, 0);
}

#[test]
fn a_folder_that_comes_back_with_an_earlier_run_s_file_is_not_appended_to() {
    // The open failed (the share was away); the rotor had already moved on. When the folder
    // returns holding a file of that name from an earlier run, the writer must not take it.
    let dir = Dir::new("comeback");
    let blocker = dir.path().join("blocked");
    std::fs::write(&blocker, b"i am a file").unwrap();
    let mut writer = Writer::new(cfg("x{n}.log", 0, 0), blocker.clone(), 1);
    writer.write_at(&at(2026, 7, 28, 9, 0));
    assert!(matches!(writer.status().last_error, Some(EmitError::Io { action: "creating", .. })), "{:?}", writer.status());
    std::fs::remove_file(&blocker).unwrap();
    std::fs::create_dir_all(&blocker).unwrap();
    std::fs::write(blocker.join("x1.log"), b"EARLIER RUN\r\n").unwrap();
    writer.write_at(&at(2026, 7, 28, 9, 0));
    assert_eq!(writer.status().last_error, None);
    assert_eq!(writer.status().current_file.as_deref(), Some("x2.log"));
    assert_eq!(std::fs::read(blocker.join("x1.log")).unwrap(), b"EARLIER RUN\r\n", "untouched");
    assert_eq!(std::fs::read_to_string(blocker.join("x2.log")).unwrap().lines().count(), 1);
    // The file this run opened is its own: a write that fails closes it, and the next line
    // reopens and appends to it rather than skipping to a third name.
    writer.write_at(&at(2026, 7, 28, 9, 1));
    assert_eq!(writer.status().current_file.as_deref(), Some("x2.log"));
    assert_eq!(std::fs::read_to_string(blocker.join("x2.log")).unwrap().lines().count(), 2);
}

#[test]
fn writers_started_together_never_share_a_file() {
    // Two runs live at once in one folder with one pattern (two benches on one share, a new
    // bench started before the old one stopped): the name is taken by creating the file new,
    // so the check and the claim are one step, and a line is one write.
    let dir = Dir::new("together");
    for round in 0..20 {
        let folder = dir.path().join(format!("r{round}"));
        std::fs::create_dir_all(&folder).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|w| {
                let folder = folder.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut c = cfg("c{n}.log", 0, 0);
                    c.template = format!("W{w} {{temp}}");
                    let mut writer = Writer::new(c, folder, w as u64 + 1);
                    barrier.wait();
                    writer.write_at(&at(2026, 7, 28, 9, 0));
                    writer.status().clone()
                })
            })
            .collect();
        let statuses: Vec<EmitterStatus> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let mut names: Vec<String> = statuses.iter().map(|s| s.current_file.clone().unwrap_or_else(|| panic!("{s:?}"))).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 6, "round {round}: six writers, six files: {statuses:?}");
        for s in &statuses {
            assert_eq!(s.last_error, None);
            assert_eq!((s.total_lines, s.lines_in_file), (1, 1));
        }
        for name in &names {
            let text = std::fs::read_to_string(folder.join(name)).unwrap();
            assert!(text.starts_with('W') && text.ends_with("\r\n") && text.lines().count() == 1, "{name}: {text:?}");
        }
    }
}

#[test]
fn when_every_name_exists_nothing_is_written_and_the_status_says_so() {
    let dir = Dir::new("nofree");
    for i in 1..=4 {
        std::fs::write(dir.path().join(format!("r{i}.log")), b"OLD\r\n").unwrap();
    }
    let mut writer = Writer::new(cfg("r{n}.log", 0, 0), dir.path().to_path_buf(), 1);
    writer.set_max_skips(3);
    writer.write_at(&at(2026, 7, 28, 9, 0));
    assert_eq!(writer.status().last_error, Some(EmitError::NoFreeName { tried: 4 }));
    assert_eq!(writer.status().total_lines, 0);
    assert_eq!(writer.status().current_file, None);
    for i in 1..=4 {
        assert_eq!(std::fs::read(dir.path().join(format!("r{i}.log"))).unwrap(), b"OLD\r\n", "r{i} untouched");
    }
    assert_eq!(files_in(dir.path()).len(), 4, "nothing else written");
    assert_eq!(EmitError::NoFreeName { tried: 4 }.to_string(), "no free file name after 4 names that exist");
    // With one more name allowed, the fifth is free and taken.
    writer.set_max_skips(4);
    writer.write_at(&at(2026, 7, 28, 9, 0));
    assert_eq!(writer.status().last_error, None);
    assert_eq!(writer.status().current_file.as_deref(), Some("r5.log"));
    // On a rotation that finds no free name, the status shows no file open, not the old one.
    let mut rotating = Writer::new(cfg("s{n}.log", 0, 1), dir.path().to_path_buf(), 2);
    rotating.set_max_skips(1);
    rotating.write_at(&at(2026, 7, 28, 9, 0));
    assert_eq!(rotating.status().current_file.as_deref(), Some("s1.log"));
    std::fs::write(dir.path().join("s2.log"), b"OLD\r\n").unwrap();
    std::fs::write(dir.path().join("s3.log"), b"OLD\r\n").unwrap();
    rotating.write_at(&at(2026, 7, 28, 9, 1));
    assert_eq!(rotating.status().last_error, Some(EmitError::NoFreeName { tried: 2 }));
    assert_eq!((rotating.status().current_file.as_deref(), rotating.status().lines_in_file, rotating.status().total_lines), (None, 0, 1));
    assert_eq!(std::fs::read_to_string(dir.path().join("s1.log")).unwrap().lines().count(), 1, "the closed file is left alone");
}

#[test]
fn a_restart_without_rotation_still_starts_a_new_file() {
    // `max_lines` 0 never rotates on count, and still a restart must not append.
    let dir = Dir::new("restart-unlimited");
    let first = spawn(cfg("one{n}.log", 10, 0), dir.path().to_path_buf(), 1).unwrap();
    until("a line", Duration::from_secs(10), || first.status().total_lines > 0);
    first.join();
    let second = spawn(cfg("one{n}.log", 10, 0), dir.path().to_path_buf(), 2).unwrap();
    until("a line", Duration::from_secs(10), || second.status().total_lines > 0);
    second.join();
    assert_eq!(files_in(dir.path()), ["one1.log", "one2.log"]);
}

#[test]
fn a_missing_output_folder_is_reported_and_then_recovered_from() {
    // A bench may point at a folder that does not exist yet (or a share that is down). The
    // worker says so and keeps trying; it does not die.
    let dir = Dir::new("missing");
    let target = dir.path().join("not-there-yet");
    // A path whose parent is a file cannot be created: a durable failure to observe, unlike a
    // missing folder, which the worker creates.
    let blocker = dir.path().join("blocked");
    std::fs::write(&blocker, b"i am a file").unwrap();
    let handle = spawn(cfg("x{n}.log", 10, 0), blocker.join("sub"), 1).unwrap();
    until("an error to surface", Duration::from_secs(10), || handle.status().last_error.is_some());
    match handle.status().last_error.unwrap() {
        EmitError::Io { action, path, message } => {
            assert_eq!(action, "creating");
            assert_eq!(path, blocker.join("sub"));
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert!(handle.status().last_error.unwrap().to_string().contains("blocked"), "the message names the path");
    assert!(!handle.is_finished(), "still trying");
    // Retried: once the blocker goes, the same emitter writes, and the error clears.
    std::fs::remove_file(&blocker).unwrap();
    until("the retry to write", Duration::from_secs(10), || handle.status().total_lines > 0);
    assert_eq!(handle.status().last_error, None);
    assert_eq!(handle.status().current_file.as_deref(), Some("x1.log"));
    assert!(blocker.join("sub").join("x1.log").is_file());
    handle.join();

    // The ordinary case: a folder that simply is not there is created.
    let ok = spawn(cfg("y{n}.log", 10, 0), target.clone(), 2).unwrap();
    until("writing to succeed", Duration::from_secs(10), || ok.status().total_lines > 0);
    assert_eq!(ok.status().last_error, None);
    assert!(target.join("y1.log").is_file());
    ok.join();
}

#[test]
fn a_utf16_emitter_writes_a_bom_so_the_encoding_is_detectable() {
    // Plant families come in both encodings; a reader sniffs the byte-order mark, so the bench
    // has to write one, once, on a fresh file.
    let dir = Dir::new("utf16");
    let mut c = cfg("wide{n}.log", 10, 0);
    c.encoding = Encoding::Utf16le;
    let handle = spawn(c, dir.path().to_path_buf(), 1).unwrap();
    until("two lines", Duration::from_secs(10), || handle.status().total_lines >= 2);
    handle.join();

    let bytes = std::fs::read(dir.path().join("wide1.log")).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xFE], "UTF-16LE BOM");
    assert_eq!(bytes.len() % 2, 0);
    let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
    assert!(!units.contains(&0xFEFF), "one BOM, at the start only");
    let text = String::from_utf16(&units).expect("valid utf-16");
    assert!(text.contains(";T="), "{text}");
    assert!(text.ends_with("\r\n"));
    assert!(text.lines().count() >= 2);
    assert_eq!(Encoding::Utf16le.label(), "UTF-16LE");
    assert_eq!(Encoding::Utf8.label(), "UTF-8");
}

#[test]
fn a_bad_file_pattern_reports_instead_of_writing_rubbish() {
    let dir = Dir::new("badpattern");
    // `%H:%M` renders a colon, which no file name may hold.
    let handle = spawn(cfg("%H:%M{n}.log", 10, 0), dir.path().to_path_buf(), 1).unwrap();
    until("the error", Duration::from_secs(10), || handle.status().last_error.is_some());
    assert_eq!(handle.status().last_error, Some(EmitError::Name(NameError::InvalidChars("`:`".into()))));
    assert!(files_in(dir.path()).is_empty(), "nothing was written");
    assert_eq!(handle.status().total_lines, 0);
    handle.join();

    let handle = spawn(cfg("", 10, 0), dir.path().to_path_buf(), 1).unwrap();
    until("the error", Duration::from_secs(10), || handle.status().last_error.is_some());
    assert_eq!(handle.status().last_error, Some(EmitError::Name(NameError::Empty)));
    assert!(files_in(dir.path()).is_empty());
    handle.join();
}

#[test]
fn a_stop_is_prompt_whatever_the_interval() {
    let dir = Dir::new("stop");
    let handle = spawn(cfg("slow{n}.log", 60_000, 0), dir.path().to_path_buf(), 1).unwrap();
    until("a line", Duration::from_secs(10), || handle.status().total_lines == 1);
    let started = Instant::now();
    handle.join();
    // One slice plus a margin for a busy machine, never the interval.
    assert!(started.elapsed() < SLICE + Duration::from_millis(300), "stopped in {:?}", started.elapsed());
    assert_eq!(SLICE, Duration::from_millis(100));
}

#[test]
fn an_emitter_config_reads_from_toml_with_its_defaults_and_refuses_a_misspelled_key() {
    let text = r#"
name = "trace"
template = "{ts} TEMP={temp} OK"
ts_format = "%Y-%m-%d %H:%M:%S%.3f"
interval_ms = 1000
file = "%d%m%Y_Trace{n}.txt"
values = [ { name = "temp", low = 18.0, high = 24.0, decimals = 1 }, { name = "n", low = 0, high = 9 } ]
"#;
    let cfg: EmitterConfig = toml::from_str(text).unwrap();
    assert_eq!(cfg.max_lines, 0);
    assert_eq!(cfg.encoding, Encoding::Utf8);
    assert_eq!(cfg.seed, None);
    assert_eq!(cfg.values[1].decimals, 0);
    assert_eq!(cfg.values[0], ValueField { name: "temp".into(), low: 18.0, high: 24.0, decimals: 1 });

    let full: EmitterConfig = toml::from_str(&format!("{text}max_lines = 5000\nencoding = \"utf-16le\"\nseed = 7\n")).unwrap();
    assert_eq!((full.max_lines, full.encoding, full.seed), (5000, Encoding::Utf16le, Some(7)));

    let err = toml::from_str::<EmitterConfig>(&format!("{text}max_line = 5\n")).unwrap_err().to_string();
    assert!(err.contains("max_line"), "{err}");
    let err = toml::from_str::<EmitterConfig>(&format!("{text}encoding = \"utf8\"\n")).unwrap_err().to_string();
    assert!(err.contains("utf8"), "{err}");
    let err = toml::from_str::<EmitterConfig>(&text.replace("interval_ms = 1000\n", "")).unwrap_err().to_string();
    assert!(err.contains("interval_ms"), "{err}");
    let err = toml::from_str::<EmitterConfig>(&text.replace("decimals = 1", "decimal = 1")).unwrap_err().to_string();
    assert!(err.contains("decimal"), "{err}");
}
