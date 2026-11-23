//! Search over logs at rest (contract: the grep clause, P-019): queries of patterns combined per
//! line, a tree, a file list or a log family as scope, files decoded as the live tail decodes
//! them, context lines and breaks, the size and binary guards, the caps, the cancel. Against the
//! bench's emitter writing a rotating family, and files the test writes for the edges.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use veredas_maquinar::emit::{self, EmitterConfig, Encoding, ValueField, family_shape};
use veredas_portal::family::{ActiveMode, FamilySpec};
use veredas_portal::grep::*;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn run(scope: GrepScope, q: &GrepQuery, o: GrepOptions) -> (Vec<GrepEvent>, GrepSummary) {
    let s = search(scope, q, o).unwrap();
    let mut events = Vec::new();
    for e in s.events.iter() {
        if let GrepEvent::Done(summary) = e {
            s.worker.join().unwrap();
            return (events, summary);
        }
        events.push(e);
    }
    panic!("no Done event");
}

/// The lines as (file name, line number, match or context, text), breaks as ("--", 0, false, "").
fn lines(events: &[GrepEvent]) -> Vec<(String, u64, bool, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            GrepEvent::Line(l) => Some((l.path.file_name().unwrap().to_string_lossy().into_owned(), l.line_number, l.is_match, l.text.clone())),
            GrepEvent::Break { .. } => Some(("--".into(), 0, false, String::new())),
            _ => None,
        })
        .collect()
}

fn matched(events: &[GrepEvent]) -> Vec<(String, u64)> {
    lines(events).into_iter().filter(|(_, _, m, _)| *m).map(|(f, n, _, _)| (f, n)).collect()
}

fn q(patterns: &[(&str, bool)], combine: Combine) -> GrepQuery {
    GrepQuery { patterns: patterns.iter().map(|(p, n)| GrepPattern { pattern: p.to_string(), negated: *n }).collect(), combine, ..GrepQuery::default() }
}

fn emitter(file: &str) -> EmitterConfig {
    EmitterConfig {
        name: "scanner".into(),
        template: "{ts} SCAN code={code} weight={weight}".into(),
        ts_format: "%Y-%m-%d %H:%M:%S%.3f".into(),
        values: vec![
            ValueField { name: "code".into(), low: 1000.0, high: 2999.0, decimals: 0 },
            ValueField { name: "weight".into(), low: 0.1, high: 30.0, decimals: 2 },
        ],
        interval_ms: 1,
        file: file.into(),
        max_lines: 25,
        encoding: Encoding::Utf16le,
        seed: Some(11),
    }
}

/// The bench writes a rotating family in UTF-16 with its mark; the grep reads it as the family
/// reads, oldest file first, every line decoded, and finds exactly the lines a reading of the
/// files finds, with their files and numbers.
#[test]
fn a_family_the_bench_wrote_is_searched_in_rotation_order_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let file = "%d%m%Y_Trace{n}.txt";
    let handle = emit::spawn(emitter(file), dir.path().to_path_buf(), 0).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    handle.join();
    let shape = family_shape(file).unwrap();
    let spec = FamilySpec::new(dir.path(), &shape.regex, Some(&shape.date_format), ActiveMode::NewestByOrder).unwrap();
    let members = spec.resolve().unwrap();
    assert!(members.len() >= 3, "the bench rotated at least twice: {}", members.len());
    // What a reading of the files finds: the lines whose code is in the 1000s.
    let code = regex::Regex::new(r"code=1\d{3} ").unwrap();
    let mut expected: Vec<(String, u64)> = Vec::new();
    for m in &members {
        let bytes = std::fs::read(&m.path).unwrap();
        let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for (i, l) in String::from_utf16(&units).unwrap().lines().enumerate() {
            if code.is_match(l) {
                expected.push((m.file_name.clone(), i as u64 + 1));
            }
        }
    }
    assert!(expected.len() >= 10, "{expected:?}");
    let (events, summary) = run(GrepScope::from_family(&spec).unwrap(), &GrepQuery::single(r"code=1\d{3} "), GrepOptions::default());
    assert_eq!(matched(&events), expected);
    let first = events.iter().find_map(|e| if let GrepEvent::Line(l) = e { Some(l.clone()) } else { None }).unwrap();
    assert!(first.text.contains(" SCAN code=1") && !first.text.contains('\u{feff}'), "decoded, the mark gone: {:?}", first.text);
    assert_eq!(first.spans.len(), 1);
    assert_eq!(&first.text[first.spans[0].start..first.spans[0].end], &first.text[first.text.find("code=").unwrap()..first.text.find("code=").unwrap() + 10]);
    assert_eq!((summary.stopped, summary.stats.files_searched, summary.stats.matches), (GrepStop::Completed, members.len() as u64, expected.len() as u64));
}

#[test]
fn matching_lines_stream_with_their_context_and_a_break_between_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=10).map(|i| if [3, 4, 9].contains(&i) { format!("line {i} ERR\r\n") } else { format!("line {i} ok\n") }).collect();
    write(&dir.path().join("a.log"), &text);
    let (events, summary) = run(GrepScope::Files(vec![dir.path().join("a.log")]), &GrepQuery::single("ERR"), GrepOptions { context_before: 1, context_after: 1, ..GrepOptions::default() });
    let got: Vec<(u64, bool, String)> = lines(&events).into_iter().map(|(_, n, m, t)| (n, m, t)).collect();
    assert_eq!(
        got,
        vec![
            (2, false, "line 2 ok".to_string()),
            (3, true, "line 3 ERR".to_string()),
            (4, true, "line 4 ERR".to_string()),
            (5, false, "line 5 ok".to_string()),
            (0, false, String::new()),
            (8, false, "line 8 ok".to_string()),
            (9, true, "line 9 ERR".to_string()),
            (10, false, "line 10 ok".to_string()),
        ]
    );
    assert!(matches!(events.last(), Some(GrepEvent::FileDone { matches: 3, capped: false, .. })), "{:?}", events.last());
    assert_eq!((summary.stats.files_matched, summary.stats.matches, summary.stopped), (1, 3, GrepStop::Completed));
    // Two lines each side, as asked, not one.
    let ten: String = (1..=10).map(|i| if i == 5 { format!("line {i} ERR\n") } else { format!("line {i} ok\n") }).collect();
    write(&dir.path().join("c.log"), &ten);
    let (events, _) = run(GrepScope::Files(vec![dir.path().join("c.log")]), &GrepQuery::single("ERR"), GrepOptions { context_before: 2, context_after: 2, ..GrepOptions::default() });
    assert_eq!(lines(&events).into_iter().map(|(_, n, _, _)| n).collect::<Vec<_>>(), vec![3, 4, 5, 6, 7]);
    // No context: no break, the matches alone; a file with none says nothing.
    write(&dir.path().join("b.log"), "nothing here\n");
    let (events, _) = run(GrepScope::Files(vec![dir.path().join("a.log"), dir.path().join("b.log")]), &GrepQuery::single("ERR"), GrepOptions::default());
    assert_eq!(lines(&events).len(), 3);
    assert_eq!(events.iter().filter(|e| matches!(e, GrepEvent::FileDone { .. })).count(), 1);
}

#[test]
fn patterns_combine_per_line_and_mark_their_spans() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("p.log"), "temp ok\ntemp alarm\nalarm alone\nnothing\n");
    let scope = || GrepScope::Files(vec![dir.path().join("p.log")]);
    let numbers = |events: &[GrepEvent]| matched(events).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    assert_eq!(numbers(&run(scope(), &q(&[("temp", false), ("alarm", false)], Combine::And), GrepOptions::default()).0), vec![2]);
    assert_eq!(numbers(&run(scope(), &q(&[("temp", false), ("alarm", true)], Combine::And), GrepOptions::default()).0), vec![1]);
    assert_eq!(numbers(&run(scope(), &q(&[("temp", false), ("alarm", false)], Combine::Or), GrepOptions::default()).0), vec![1, 2, 3]);
    assert_eq!(numbers(&run(scope(), &q(&[("temp", false), ("alarm", false)], Combine::Xor), GrepOptions::default()).0), vec![1, 3]);
    // An odd number of tests holding, not exactly one: three patterns, three holding on line 1.
    write(&dir.path().join("abc.log"), "a b c\na b\na\nnone\n");
    let (events, _) = run(GrepScope::Files(vec![dir.path().join("abc.log")]), &q(&[("a", false), ("b", false), ("c", false)], Combine::Xor), GrepOptions::default());
    assert_eq!(numbers(&events), vec![1, 3]);
    assert_eq!(numbers(&run(scope(), &q(&[("temp", true), ("alarm", true)], Combine::And), GrepOptions::default()).0), vec![4]);
    // Spans: each pattern's matches tagged by its index; a negated pattern marks nothing; an
    // overlap with an earlier span dropped.
    let (events, _) = run(scope(), &q(&[("alarm", false), ("temp", false), ("emp a", false)], Combine::And), GrepOptions::default());
    let line = events.iter().find_map(|e| if let GrepEvent::Line(l) = e { Some(l.clone()) } else { None }).unwrap();
    assert_eq!((line.line_number, line.spans), (2, vec![Span { start: 0, end: 4, pattern: 1 }, Span { start: 5, end: 10, pattern: 0 }]));
    let (events, _) = run(scope(), &q(&[("temp", false), ("alarm", true)], Combine::And), GrepOptions::default());
    let line = events.iter().find_map(|e| if let GrepEvent::Line(l) = e { Some(l.clone()) } else { None }).unwrap();
    assert_eq!(line.spans, vec![Span { start: 0, end: 4, pattern: 0 }]);
}

#[test]
fn literal_case_and_whole_word() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("o.log"), "a.b\naxb\nRUN\nrunning\nrun now\n");
    let scope = || GrepScope::Files(vec![dir.path().join("o.log")]);
    let numbers = |query: GrepQuery| matched(&run(scope(), &query, GrepOptions::default()).0).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    assert_eq!(numbers(GrepQuery::single("a.b")), vec![1, 2], "a regex");
    assert_eq!(numbers(GrepQuery { literal: true, ..GrepQuery::single("a.b") }), vec![1], "literal");
    assert_eq!(numbers(GrepQuery::single("run")), vec![4, 5]);
    assert_eq!(numbers(GrepQuery { case_insensitive: true, ..GrepQuery::single("run") }), vec![3, 4, 5]);
    assert_eq!(numbers(GrepQuery { whole_word: true, ..GrepQuery::single("run") }), vec![5]);
    assert_eq!(numbers(GrepQuery { whole_word: true, case_insensitive: true, ..GrepQuery::single("run") }), vec![3, 5]);
    // Whole word around the operator's own pattern, never its text: an unbalanced group refused
    // in its own words, a comment of the extended mode closed before the wrapper's end.
    let scope_once = GrepScope::Files(vec![dir.path().join("o.log")]);
    let e = search(scope_once, &GrepQuery { whole_word: true, ..GrepQuery::single("a)|(?:b") }, GrepOptions::default()).err().unwrap();
    assert!(matches!(&e, GrepError::Pattern(m) if m.starts_with("pattern 1 `a)|(?:b`: ") && m.contains("unopened group")), "{e:?}");
    assert_eq!(numbers(GrepQuery { whole_word: true, ..GrepQuery::single("(?x)run # word") }), vec![5]);
    assert_eq!(numbers(GrepQuery::single("(?x)run # word")), vec![4, 5]);
    // A pattern whose ends are no word characters: whole where it stands apart, never where it is
    // glued to a word.
    write(&dir.path().join("o.log"), "offset -5 mm\nreg a-5b\nset a-5 b\ntemp 25°C\ntemp 25 °C\n");
    assert_eq!(numbers(GrepQuery { whole_word: true, literal: true, ..GrepQuery::single("-5") }), vec![1]);
    assert_eq!(numbers(GrepQuery { whole_word: true, literal: true, ..GrepQuery::single("°C") }), vec![5]);
    assert_eq!(numbers(GrepQuery { literal: true, ..GrepQuery::single("-5") }), vec![1, 2, 3], "not whole: every one");
}

/// A file is decoded as the live tail decodes it (C-035), in the tail's own slices: UTF-8 opening
/// it and one stray CP1252 byte past the tail's first 64 KiB, the grep's lines the tail's.
#[test]
fn a_file_is_decoded_as_the_tail_decodes_it_slice_by_slice() {
    // (and, below it, a second mark kept where the tail keeps it)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.log");
    let mut bytes = "Produção iniciada\n".as_bytes().to_vec();
    for i in 0..4_200 {
        bytes.extend_from_slice(format!("ascii line {i:05}\n").as_bytes());
    }
    bytes.extend_from_slice(b"temp 25\xB0C\n");
    std::fs::write(&path, &bytes).unwrap();
    let (mut set, rx) = veredas_portal::SourceSet::new();
    let cfg = veredas_portal::TailConfig { poll_interval: Duration::from_millis(25), drain_grace: Duration::from_millis(60), use_notify: None, start: veredas_portal::FileStart::Begin };
    set.spawn(Box::new(veredas_portal::FileTailSource::new("t", "tail", &path, cfg))).unwrap();
    let mut tail: Vec<String> = Vec::new();
    while tail.len() < 4_202 {
        tail.push(rx.recv_timeout(Duration::from_secs(10)).expect("the tail's lines").line);
    }
    drop(set);
    assert_eq!((tail[0].as_str(), tail[4_201].as_str()), ("Produção iniciada", "temp 25\u{FFFD}C"));
    let (events, _) = run(GrepScope::Files(vec![path.clone()]), &q(&[("Produção", false), ("^temp", false)], Combine::Or), GrepOptions::default());
    assert_eq!(lines(&events), vec![("mixed.log".to_string(), 1, true, tail[0].clone()), ("mixed.log".to_string(), 4_202, true, tail[4_201].clone())]);
}

#[test]
fn every_encoding_the_tail_reads_is_searched_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let text = "first\ntemp 25\u{b0}C\nlast\n";
    let utf16 = |be: bool, mark: bool| -> Vec<u8> {
        let mut out = Vec::new();
        if mark {
            out.extend(if be { [0xFE, 0xFF] } else { [0xFF, 0xFE] });
        }
        for u in text.encode_utf16() {
            out.extend(if be { u.to_be_bytes() } else { u.to_le_bytes() });
        }
        out
    };
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("utf8.log", text.as_bytes().to_vec()),
        ("utf8bom.log", [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat()),
        ("le.log", utf16(false, true)),
        ("be.log", utf16(true, true)),
        ("le_nomark.log", utf16(false, false)),
        ("cp1252.log", b"first\ntemp 25\xb0C\nlast\n".to_vec()),
    ];
    for (name, bytes) in &files {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    let (events, summary) = run(GrepScope::Files(files.iter().map(|(n, _)| dir.path().join(n)).collect()), &GrepQuery::single("25\u{b0}C"), GrepOptions::default());
    let got: Vec<(String, u64, String)> = lines(&events).into_iter().map(|(f, n, _, t)| (f, n, t)).collect();
    assert_eq!(got, files.iter().map(|(n, _)| (n.to_string(), 2, "temp 25\u{b0}C".to_string())).collect::<Vec<_>>());
    assert_eq!((summary.stats.files_searched, summary.stats.skipped_binary), (6, 0), "a UTF-16 file is no binary file");
}

#[test]
fn a_tree_by_masks_recursive_or_not_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    for p in ["x.log", "y.LOG", "z.txt", "plant1.txt", "deep/w.log", "deep/deeper/v.log"] {
        write(&a.join(p), "hit\n");
    }
    write(&b.join("u.log"), "hit\n");
    let names = |events: &[GrepEvent]| matched(events).into_iter().map(|(f, _)| f).collect::<Vec<_>>();
    let tree = |masks: &[&str], recursive: bool| GrepScope::Tree { roots: vec![a.clone(), b.clone()], masks: masks.iter().map(|m| m.to_string()).collect(), recursive };
    assert_eq!(names(&run(tree(&["*.log"], true), &GrepQuery::single("hit"), GrepOptions::default()).0), vec!["v.log", "w.log", "x.log", "y.LOG", "u.log"]);
    assert_eq!(names(&run(tree(&["*.log"], false), &GrepQuery::single("hit"), GrepOptions::default()).0), vec!["x.log", "y.LOG", "u.log"]);
    assert_eq!(names(&run(tree(&["plant?.txt"], true), &GrepQuery::single("hit"), GrepOptions::default()).0), vec!["plant1.txt"]);
    assert_eq!(names(&run(tree(&[], false), &GrepQuery::single("hit"), GrepOptions::default()).0), vec!["plant1.txt", "x.log", "y.LOG", "z.txt", "u.log"]);
    // A root that is not there: an error in its place, the others searched.
    let (events, summary) = run(GrepScope::Tree { roots: vec![dir.path().join("gone"), b.clone()], masks: vec![], recursive: true }, &GrepQuery::single("hit"), GrepOptions::default());
    assert!(matches!(&events[0], GrepEvent::WalkError { error } if error.contains("gone")), "{:?}", events[0]);
    assert_eq!((names(&events), summary.stats.errors), (vec!["u.log".to_string()], 1));
}

#[test]
fn the_size_and_binary_guards_the_caps_and_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("big.log"), &"hit\n".repeat(1000));
    std::fs::write(dir.path().join("bin.dat"), b"hit\0\x01\x02hit\n").unwrap();
    write(&dir.path().join("five.log"), &"hit\n".repeat(5));
    let files = |names: &[&str]| GrepScope::Files(names.iter().map(|n| dir.path().join(n)).collect());
    // A file past the size limit: skipped and named; a binary file: skipped, counted; asked for,
    // searched.
    let (events, s) = run(files(&["big.log", "bin.dat", "five.log"]), &GrepQuery::single("hit"), GrepOptions { max_file_size: Some(1000), ..GrepOptions::default() });
    assert!(matches!(&events[0], GrepEvent::SkippedLarge { size: 4000, path } if path.ends_with("big.log")), "{:?}", events[0]);
    assert_eq!((s.stats.skipped_large, s.stats.skipped_binary, s.stats.files_searched, s.stats.matches), (1, 1, 1, 5));
    let (events, s) = run(files(&["bin.dat"]), &GrepQuery::single("hit"), GrepOptions { search_binary: true, ..GrepOptions::default() });
    assert_eq!((matched(&events).len(), s.stats.skipped_binary), (1, 0));
    // The edges: a file of exactly the limit is searched; a NUL past the head is no binary file
    // (a log padded with NULs after a power cut).
    let (events, s) = run(files(&["five.log"]), &GrepQuery::single("hit"), GrepOptions { max_file_size: Some(20), ..GrepOptions::default() });
    assert_eq!((matched(&events).len(), s.stats.skipped_large), (5, 0), "five lines of four bytes, the limit twenty");
    let mut padded = "hit\n".repeat(2_100).into_bytes();
    padded.extend_from_slice(&[0u8; 64]);
    padded.extend_from_slice(b"\nhit\n");
    std::fs::write(dir.path().join("padded.log"), &padded).unwrap();
    let (events, s) = run(files(&["padded.log"]), &GrepQuery::single("hit"), GrepOptions::default());
    assert_eq!((matched(&events).len(), s.stats.skipped_binary), (2_101, 0), "its first NUL at byte {}", 2_100 * 4);
    // A cap per file: its done event says so; a total cap: the search stops there.
    let (events, s) = run(files(&["five.log", "big.log"]), &GrepQuery::single("hit"), GrepOptions { max_matches_per_file: Some(2), ..GrepOptions::default() });
    assert_eq!(events.iter().filter(|e| matches!(e, GrepEvent::FileDone { matches: 2, capped: true, .. })).count(), 2);
    assert_eq!((s.stats.matches, s.stopped), (4, GrepStop::Completed));
    let (events, s) = run(files(&["five.log", "big.log"]), &GrepQuery::single("hit"), GrepOptions { max_total_matches: Some(7), ..GrepOptions::default() });
    assert_eq!((matched(&events).len(), s.stats.matches, s.stopped), (7, 7, GrepStop::MatchCap));
    // Each file with matches ends with its count, the one the total cap stopped among them.
    let counts: Vec<(String, u64)> = events.iter().filter_map(|e| if let GrepEvent::FileDone { path, matches, .. } = e { Some((path.file_name().unwrap().to_string_lossy().into_owned(), *matches)) } else { None }).collect();
    assert_eq!((counts, s.stats.files_matched), (vec![("five.log".to_string(), 5), ("big.log".to_string(), 2)], 2));
    // A file that is not there: an error in its place, the next searched.
    let (events, s) = run(files(&["nope.log", "five.log"]), &GrepQuery::single("hit"), GrepOptions::default());
    assert!(matches!(&events[0], GrepEvent::FileError { path, .. } if path.ends_with("nope.log")), "{:?}", events[0]);
    assert_eq!((s.stats.errors, s.stats.matches), (1, 5));
}

#[test]
fn a_cancel_stops_a_running_search_and_a_bad_query_starts_none() {
    let dir = tempfile::tempdir().unwrap();
    // One large file: a cancel is heard inside a file, not only between files.
    let big = dir.path().join("big.log");
    write(&big, &"line with a hit\n".repeat(2_000_000));
    let names: Vec<PathBuf> = vec![big, dir.path().join("big.log")];
    let s = search(GrepScope::Files(names), &GrepQuery::single("hit"), GrepOptions::default()).unwrap();
    let handle: Arc<GrepHandle> = Arc::clone(&s.handle);
    let mut seen = 0u64;
    let mut done = None;
    let mut file_done = None;
    for e in s.events.iter() {
        match e {
            GrepEvent::Line(_) => {
                seen += 1;
                if seen == 10 {
                    handle.cancel();
                }
            }
            GrepEvent::FileDone { matches, capped, .. } => file_done = Some((matches, capped)),
            GrepEvent::Done(summary) => {
                done = Some(summary);
                break;
            }
            _ => {}
        }
    }
    let done = done.unwrap();
    assert_eq!(done.stopped, GrepStop::Cancelled);
    assert!(done.stats.matches < 2_000_000, "stopped inside the first file: {:?}", done.stats);
    // The file cancelled mid-scan still ends with its count, the lines it gave, and the totals
    // count it matched.
    assert_eq!(file_done, Some((seen, false)), "{:?}", done.stats);
    assert_eq!((done.stats.matches, done.stats.files_matched), (seen, 1));
    s.worker.join().unwrap();
    // Refused before anything is read.
    let scope = || GrepScope::Files(vec![]);
    assert_eq!(search(scope(), &GrepQuery::default(), GrepOptions::default()).err(), Some(GrepError::Pattern("the query has no pattern".into())));
    assert_eq!(search(scope(), &GrepQuery::single(""), GrepOptions::default()).err(), Some(GrepError::Pattern("pattern 1 is empty".into())));
    let e = search(scope(), &q(&[("ok", false), ("(", false)], Combine::And), GrepOptions::default()).err().unwrap();
    assert!(matches!(&e, GrepError::Pattern(m) if m.starts_with("pattern 2 `(`: ") && m.contains("unclosed group")), "{e:?}");
    assert_eq!(
        search(GrepScope::Tree { roots: vec![], masks: vec![], recursive: true }, &GrepQuery::single("x"), GrepOptions::default()).err(),
        Some(GrepError::Scope("a tree with no root".into()))
    );
    // Lookaround is not the suite's syntax: refused by the engine's own words.
    let e = search(scope(), &GrepQuery::single("(?<=a)b"), GrepOptions::default()).err().unwrap();
    assert!(matches!(&e, GrepError::Pattern(m) if m.contains("look-around, including look-ahead and look-behind, is not supported")), "{e:?}");
}


/// Masks match a file's name by its characters: `?` one letter, an accented one too.
#[test]
fn a_mask_s_question_mark_is_one_letter_whatever_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["relé.log", "rele.log", "pressão1.log", "rel.log"] {
        write(&dir.path().join(name), "hit\n");
    }
    let found = |mask: &str| -> Vec<String> {
        let (events, _) = run(GrepScope::Tree { roots: vec![dir.path().to_path_buf()], masks: vec![mask.into()], recursive: false }, &GrepQuery::single("hit"), GrepOptions::default());
        matched(&events).into_iter().map(|(f, _)| f).collect()
    };
    assert_eq!(found("rel?.log"), vec!["rele.log", "relé.log"]);
    assert_eq!(found("press?o?.log"), vec!["pressão1.log"]);
}

/// The walk follows no link, to a folder or to a file, as Veredas 1's did: a `loop -> .` searches
/// the tree once; and every file is opened for reading alone, a read-only one searched.
#[cfg(unix)]
#[test]
fn the_walk_follows_no_link_and_reads_a_read_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    std::fs::create_dir(&a).unwrap();
    write(&a.join("x.log"), "hit\n");
    std::os::unix::fs::symlink(".", a.join("loop")).unwrap();
    std::os::unix::fs::symlink("x.log", a.join("y.log")).unwrap();
    let locked = a.join("z.log");
    write(&locked, "hit locked\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (events, summary) = run(GrepScope::Tree { roots: vec![dir.path().to_path_buf()], masks: vec![], recursive: true }, &GrepQuery::single("hit"), GrepOptions::default());
    assert_eq!(matched(&events), vec![("x.log".to_string(), 1), ("z.log".to_string(), 1)]);
    let s = summary.stats;
    assert_eq!((s.files_searched, s.files_matched, s.matches, s.errors), (2, 2, 2, 0), "{s:?}");
}


/// A cancel is heard while a file is read and decoded, not only once its lines are scanned: a
/// file whose reading takes seconds, a query that matches nothing, a cancel 50 ms in.
#[test]
fn a_cancel_is_heard_while_a_file_is_read() {
    // In the build's own scratch folder: the file is large, and /tmp is kept small on this machine.
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let big = dir.path().join("huge.log");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&big).unwrap());
        let line = format!("{}\n", "a reading with nothing in it ".repeat(4));
        for _ in 0..(192usize << 20) / line.len() {
            std::io::Write::write_all(&mut f, line.as_bytes()).unwrap();
        }
    }
    let s = search(GrepScope::Files(vec![big]), &GrepQuery::single("zzz"), GrepOptions::default()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let cancelled = std::time::Instant::now();
    s.handle.cancel();
    let done = s.events.iter().find_map(|e| if let GrepEvent::Done(summary) = e { Some(summary) } else { None }).expect("a done event");
    let took = cancelled.elapsed();
    s.worker.join().unwrap();
    assert_eq!(done.stopped, GrepStop::Cancelled, "{done:?}");
    assert!(took < Duration::from_millis(500), "the cancel heard {took:?} after it was asked, the file still being read");
}


/// The lines a live tail started at the file's beginning gives, the first `count` of them.
fn tail_lines(path: &Path, count: usize) -> Vec<String> {
    let (mut set, rx) = veredas_portal::SourceSet::new();
    let cfg = veredas_portal::TailConfig { poll_interval: Duration::from_millis(25), drain_grace: Duration::from_millis(60), use_notify: None, start: veredas_portal::FileStart::Begin };
    set.spawn(Box::new(veredas_portal::FileTailSource::new("t", "tail", path, cfg))).unwrap();
    let lines = (0..count).map(|_| rx.recv_timeout(Duration::from_secs(10)).expect("the tail's lines").line).collect();
    drop(set);
    lines
}

/// A second byte-order mark is text, as the tail reads it: only the first, the mark, is dropped.
#[test]
fn a_second_mark_is_kept_as_the_tail_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marked.log");
    let mut bytes = vec![0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("Header ação\nsecond\n".as_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let tail = tail_lines(&path, 2);
    assert_eq!(tail[0], "\u{feff}Header ação");
    let (events, _) = run(GrepScope::Files(vec![path.clone()]), &GrepQuery::single("Header"), GrepOptions::default());
    assert_eq!(lines(&events), vec![("marked.log".to_string(), 1, true, tail[0].clone())]);
    let (events, _) = run(GrepScope::Files(vec![path]), &GrepQuery::single("^Header"), GrepOptions::default());
    assert!(matched(&events).is_empty(), "the line starts with the second mark, as the tail's does");
}

/// A cancel is heard while the lines are scanned, at every line: a file of few long lines and a
/// slow pattern, the reading long done, the scan seconds from its end.
#[test]
fn a_cancel_is_heard_at_every_line_of_a_slow_scan() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let path = dir.path().join("long_lines.log");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let line = format!("{}\n", "ação válvula ".repeat(64 * 1024 / 15));
        for _ in 0..300 {
            std::io::Write::write_all(&mut f, line.as_bytes()).unwrap();
        }
    }
    let q = GrepQuery { whole_word: true, ..GrepQuery::single(r"\d{3}") };
    let s = search(GrepScope::Files(vec![path]), &q, GrepOptions::default()).unwrap();
    std::thread::sleep(Duration::from_millis(2_500));
    let cancelled = std::time::Instant::now();
    s.handle.cancel();
    let done = s.events.iter().find_map(|e| if let GrepEvent::Done(summary) = e { Some(summary) } else { None }).expect("a done event");
    let took = cancelled.elapsed();
    s.worker.join().unwrap();
    assert_eq!(done.stopped, GrepStop::Cancelled, "{done:?}");
    assert!(took < Duration::from_millis(500), "the cancel heard {took:?} after it was asked");
}

/// A line past `LINE_LIMIT` is searched in its first 4 MiB and no further, wherever the 64 KiB
/// slices fall: a match ending at the limit found, the line's text the limit's bytes exactly, a
/// match just past it not, whether the line's end comes in the slice that took it past the limit
/// or many slices later, and the next line numbered as the next. A line far longer than a slice
/// and shorter than the limit is searched to its end.
#[test]
fn a_line_past_the_limit_is_searched_in_its_first_4_mib_wherever_the_slices_fall() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let spans_of = |events: &[GrepEvent]| -> Vec<(u64, usize, Vec<usize>)> {
        events.iter().filter_map(|e| if let GrepEvent::Line(l) = e { Some((l.line_number, l.text.len(), l.spans.iter().map(|s| s.start).collect())) } else { None }).collect()
    };
    let long = dir.path().join("long.log");
    write(&long, &format!("{} MID\n", "x".repeat(200_000)));
    let (events, _) = run(GrepScope::Files(vec![long]), &GrepQuery::single("MID"), GrepOptions::default());
    assert_eq!(spans_of(&events), vec![(1, 200_004, vec![200_001])]);
    // The limit is 4 MiB, the figure the clause names, whatever the constant says: a line one
    // byte short of it is searched to its last bytes.
    const FOUR_MIB: usize = 4 << 20;
    assert_eq!(LINE_LIMIT, FOUR_MIB);
    let under = dir.path().join("under.log");
    write(&under, &format!("{} END
next
", "x".repeat(FOUR_MIB - 1 - 4)));
    let (events, _) = run(GrepScope::Files(vec![under]), &GrepQuery::single("END"), GrepOptions::default());
    assert_eq!(spans_of(&events), vec![(1, FOUR_MIB - 1, vec![FOUR_MIB - 4])]);
    let inside = format!("{} EARLY", "x".repeat(FOUR_MIB - 6));
    let far = format!(" LATE {}", "y".repeat(100_000));
    for (name, rest) in [("near.log", " LATE"), ("far.log", far.as_str())] {
        let path = dir.path().join(name);
        write(&path, &format!("{inside}{rest}\nnext LATE\n"));
        let (events, summary) = run(GrepScope::Files(vec![path]), &q(&[("EARLY", false), ("LATE", false)], Combine::Or), GrepOptions::default());
        assert_eq!(spans_of(&events), vec![(1, FOUR_MIB, vec![FOUR_MIB - 5]), (2, 9, vec![5])], "{name}");
        assert_eq!((summary.stopped, summary.stats.matches), (GrepStop::Completed, 2), "{name}");
    }
}

/// A line cut at 4 MiB inside a character (`ç`'s two bytes straddling the limit) is cut before
/// that character, at the last whole one: its first 4 194 303 bytes searched, the next line
/// read, the search ended as any other.
#[test]
fn a_line_cut_inside_a_character_is_cut_before_it() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let path = dir.path().join("straddle.log");
    write(&path, &format!("{}ç TAIL\nnext\n", "x".repeat((4 << 20) - 1)));
    let (events, summary) = run(GrepScope::Files(vec![path]), &q(&[("^x", false), ("next", false)], Combine::Or), GrepOptions::default());
    let got: Vec<(u64, usize)> = lines(&events).into_iter().map(|(_, n, _, t)| (n, t.len())).collect();
    assert_eq!(got, vec![(1, (4 << 20) - 1), (2, 4)]);
    assert_eq!(summary.stopped, GrepStop::Completed);
}

/// A cancel is heard at every slice: a file of lines a little longer than a slice, each
/// matching under slow patterns (a slice completes one line, scanned in some 100 ms), cancelled as
/// its line `k` arrives, gives at most the next line, which the slice after line `k` completes;
/// at three lines in a row, so that a check every second or third slice gives more at one of them.
#[test]
fn a_cancel_is_heard_at_every_slice() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let path = dir.path().join("every_slice.log");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let line = format!("{}\n", "ação válvula ".repeat(64 * 1024 / 15));
        for _ in 0..300 {
            std::io::Write::write_all(&mut f, line.as_bytes()).unwrap();
        }
    }
    let query = GrepQuery { whole_word: true, ..q(&[(r"\d{3}", false), (r"\d{4}", false), ("ação", false)], Combine::Or) };
    for k in [10u64, 11, 12] {
        let s = search(GrepScope::Files(vec![path.clone()]), &query, GrepOptions::default()).unwrap();
        let mut matched = 0u64;
        for e in s.events.iter() {
            match e {
                GrepEvent::Line(l) if l.is_match => {
                    matched += 1;
                    if l.line_number == k {
                        s.handle.cancel();
                    }
                }
                GrepEvent::Done(done) => {
                    assert_eq!(done.stopped, GrepStop::Cancelled, "{done:?}");
                    break;
                }
                _ => {}
            }
        }
        s.worker.join().unwrap();
        assert!(matched >= k && matched <= k + 1, "cancelled at line {k}, {matched} lines given");
    }
}

/// A cap of 0 gives no line: a file whose cap is 0 is capped before its first line, and a total
/// cap of 0 ends the search before its first match (grep's `-m 0`).
#[test]
fn a_cap_of_zero_gives_no_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("three.log");
    write(&file, "hit 1\nhit 2\nhit 3\n");
    let (events, s) = run(GrepScope::Files(vec![file.clone(), file.clone()]), &GrepQuery::single("hit"), GrepOptions { max_matches_per_file: Some(0), ..GrepOptions::default() });
    assert!(matched(&events).is_empty(), "{events:?}");
    assert_eq!((s.stats.matches, s.stats.files_matched, s.stopped), (0, 0, GrepStop::Completed));
    let (events, s) = run(GrepScope::Files(vec![file]), &GrepQuery::single("hit"), GrepOptions { max_total_matches: Some(0), ..GrepOptions::default() });
    assert!(matched(&events).is_empty(), "{events:?}");
    assert_eq!((s.stats.matches, s.stopped), (0, GrepStop::MatchCap));
}

/// With no size limit, what a search holds is a slice and a line, whatever the file: a file of
/// 256 MiB, nearly all of it one endless line of NULs past its text, searched whole with the next
/// file after it, the process's peak memory far below the file's size.
#[cfg(target_os = "linux")]
#[test]
fn a_search_holds_a_slice_and_a_line_whatever_the_file_s_size() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let huge = dir.path().join("huge.log");
    std::fs::write(&huge, "hit line\n".repeat(1_000)).unwrap();
    std::fs::OpenOptions::new().write(true).open(&huge).unwrap().set_len(256 << 20).unwrap();
    let after = dir.path().join("after.log");
    write(&after, "hit after\n");
    let (events, summary) = run(GrepScope::Files(vec![huge, after]), &GrepQuery::single("hit"), GrepOptions::default());
    assert_eq!((summary.stopped, summary.stats.matches, summary.stats.files_searched, summary.stats.errors), (GrepStop::Completed, 1_001, 2, 0));
    assert_eq!(matched(&events).last(), Some(&("after.log".to_string(), 1)));
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let peak_kib: u64 = status.lines().find_map(|l| l.strip_prefix("VmHWM:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()).unwrap();
    assert!(peak_kib < 160 * 1024, "the process peaked at {} MiB for a 256 MiB file", peak_kib / 1024);
}
