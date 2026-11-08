//! The file tail (contract: the file tail clause): one growing file followed through appends,
//! truncation and replacement, text decoded whatever its encoding, lines stamped by a timestamp
//! rule, merged with other sources into one tagged stream, and the plant's writer never blocked
//! by the reading. Every test is over a real file in a real folder.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use veredas_portal::tail::{PathKind, classify};
use veredas_portal::{
    Emitter, FileStart, FileTailSource, LineEvent, Source, SourceError, SourceHandle, SourceId, SourceSet, TailConfig,
    TimestampSpec, stop_flag,
};

const RECV_TIMEOUT: Duration = Duration::from_secs(10);

fn fast_cfg(start: FileStart) -> TailConfig {
    TailConfig {
        poll_interval: Duration::from_millis(25),
        drain_grace: Duration::from_millis(60),
        use_notify: None,
        start,
    }
}

fn tail(path: &Path, start: FileStart) -> (SourceSet, Receiver<LineEvent>) {
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FileTailSource::new("t", "test tail", path, fast_cfg(start)))).unwrap();
    (set, rx)
}

fn append(path: &Path, text: &str) {
    let mut f = OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
    f.flush().unwrap();
}

fn recv_line(rx: &Receiver<LineEvent>) -> String {
    rx.recv_timeout(RECV_TIMEOUT).expect("timed out waiting for a line").line
}

fn assert_quiet(rx: &Receiver<LineEvent>, ms: u64) {
    if let Ok(ev) = rx.recv_timeout(Duration::from_millis(ms)) {
        panic!("expected silence, got the line {:?}", ev.line);
    }
}

// ---- one growing file ------------------------------------------------------------------------

#[test]
fn appended_lines_arrive_in_order_with_dense_sequence_and_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log");
    std::fs::write(&path, "one\n").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);
    let before = veredas_portal::now_unix_ms();
    let first = rx.recv_timeout(RECV_TIMEOUT).unwrap();
    assert_eq!(first.line, "one");
    assert_eq!(first.seq, 0);
    assert_eq!(first.source.as_str(), "t");
    assert!(first.parsed_ts.is_none());
    assert!(first.arrival_unix_ms >= before - 1 && first.arrival_unix_ms <= veredas_portal::now_unix_ms() + 1);
    assert_eq!(first.index_ms(), first.arrival_unix_ms, "no stamp: the index is the arrival");

    append(&path, "two\nthree\n");
    let second = rx.recv_timeout(RECV_TIMEOUT).unwrap();
    let third = rx.recv_timeout(RECV_TIMEOUT).unwrap();
    assert_eq!((second.line.as_str(), second.seq), ("two", 1));
    assert_eq!((third.line.as_str(), third.seq), ("three", 2));
    assert!(third.arrival_unix_ms >= second.arrival_unix_ms);

    for (_, r) in set.stop_all() {
        r.unwrap();
    }
}

#[test]
fn crlf_is_stripped_and_a_bare_cr_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crlf.log");
    std::fs::write(&path, "alpha\r\nbeta\r\ngam\rma\n\n").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(recv_line(&rx), "alpha");
    assert_eq!(recv_line(&rx), "beta");
    assert_eq!(recv_line(&rx), "gam\rma", "a carriage return inside a line is content");
    assert_eq!(recv_line(&rx), "", "an empty line is a line");
    set.stop_all();
}

#[test]
fn partial_line_is_held_until_its_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.log");
    std::fs::write(&path, "").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);

    append(&path, "par");
    assert_quiet(&rx, 200);
    append(&path, "tial\nnext\n");
    assert_eq!(recv_line(&rx), "partial");
    assert_eq!(recv_line(&rx), "next");
    set.stop_all();
}

#[test]
fn start_at_end_skips_history_and_the_torn_fragment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("end.log");
    // The file ends mid-line: "half" has no newline yet.
    std::fs::write(&path, "old1\nold2\nhalf").unwrap();

    let (set, rx) = tail(&path, FileStart::End);
    assert_quiet(&rx, 200);

    // "rest" completes the torn line; the tail never saw its start, so "halfrest" is skipped
    // whole, and "fresh" is the first whole line born after the attach.
    append(&path, "rest\nfresh\n");
    assert_eq!(recv_line(&rx), "fresh");
    set.stop_all();
}

#[test]
fn start_at_end_of_a_file_ending_in_a_newline_emits_the_next_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("end2.log");
    std::fs::write(&path, "old\n").unwrap();
    let (set, rx) = tail(&path, FileStart::End);
    assert_quiet(&rx, 150);
    append(&path, "new\n");
    assert_eq!(recv_line(&rx), "new");
    set.stop_all();
}

#[test]
fn truncation_flushes_the_held_partial_and_rereads_from_the_top() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trunc.log");
    std::fs::write(&path, "AAA\nBB").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(recv_line(&rx), "AAA");

    // An in-place truncate and rewrite (the same file identity, a smaller size).
    {
        let mut f = File::create(&path).unwrap();
        f.write_all(b"CC\n").unwrap();
    }
    // The held "BB" will never complete: flushed, then the new content read from offset zero.
    assert_eq!(recv_line(&rx), "BB");
    assert_eq!(recv_line(&rx), "CC");
    set.stop_all();
}

#[test]
fn replacement_drains_the_old_file_then_follows_the_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repl.log");
    std::fs::write(&path, "old1\n").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(recv_line(&rx), "old1");

    // The writer appends a final line (torn, no newline), renames the file away and starts a
    // fresh one at the same path: the classic rotation.
    append(&path, "old2");
    std::fs::rename(&path, dir.path().join("repl.log.retired")).unwrap();
    std::fs::write(&path, "new1\n").unwrap();

    // The old handle drains to a stable end: the torn "old2" flushes as a final line, then the
    // new file is read from its beginning.
    assert_eq!(recv_line(&rx), "old2");
    assert_eq!(recv_line(&rx), "new1");
    set.stop_all();
}

#[test]
fn a_deleted_file_is_drained_and_whatever_reappears_is_read_from_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.log");
    std::fs::write(&path, "a\n").unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(recv_line(&rx), "a");
    append(&path, "b");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(recv_line(&rx), "b", "the torn final line of a deleted file is flushed");
    assert_quiet(&rx, 150);
    std::fs::write(&path, "c\n").unwrap();
    assert_eq!(recv_line(&rx), "c");
    set.stop_all();
}

#[test]
fn waits_for_a_file_to_appear_and_reads_it_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("late.log");

    let (set, rx) = tail(&path, FileStart::End);
    assert_quiet(&rx, 150);

    std::fs::write(&path, "hello\n").unwrap();
    // The configured start applies only to a file that existed at attach; a file born later is
    // new content, read from its start.
    assert_eq!(recv_line(&rx), "hello");
    set.stop_all();
}

#[test]
fn stop_flushes_the_final_partial_and_the_worker_ends_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stopflush.log");
    std::fs::write(&path, "done\nhalf").unwrap();

    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(recv_line(&rx), "done");
    assert_quiet(&rx, 100); // "half" is held as a partial

    for (id, r) in set.stop_all() {
        assert_eq!(id.as_str(), "t");
        r.unwrap();
    }
    let remaining: Vec<String> = rx.try_iter().map(|e| e.line).collect();
    assert_eq!(remaining, vec!["half".to_string()]);
}

#[test]
fn a_fast_writer_cannot_pin_the_reader_and_every_line_arrives_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fast.log");
    std::fs::write(&path, "").unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    let total = 20_000u64;
    let writer = {
        let p = path.clone();
        std::thread::spawn(move || {
            let mut f = OpenOptions::new().append(true).open(p).unwrap();
            for n in 0..total {
                writeln!(f, "L{n:05}").unwrap();
            }
            f.flush().unwrap();
        })
    };
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while got.len() < total as usize {
        let remaining = deadline.checked_duration_since(Instant::now()).expect("timed out");
        got.push(rx.recv_timeout(remaining).unwrap().line);
    }
    writer.join().unwrap();
    let expected: Vec<String> = (0..total).map(|n| format!("L{n:05}")).collect();
    assert_eq!(got, expected);
    set.stop_all();
}

// ---- encodings -------------------------------------------------------------------------------

fn lines(rx: &Receiver<LineEvent>, n: usize) -> Vec<String> {
    (0..n).map(|_| rx.recv_timeout(RECV_TIMEOUT).expect("timed out waiting for a line").line).collect()
}

fn utf16le(text: &str, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(&[0xFF, 0xFE]);
    }
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn utf16be(text: &str, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(&[0xFE, 0xFF]);
    }
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

const EXPECT: [&str; 2] = ["Temp 25°C éàü", "second line"];
const TEXT: &str = "Temp 25°C éàü\r\nsecond line\r\n";

#[test]
fn utf8_with_and_without_a_byte_order_mark() {
    let dir = tempfile::tempdir().unwrap();
    let with = dir.path().join("utf8bom.log");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(TEXT.as_bytes());
    std::fs::write(&with, bytes).unwrap();
    let (set, rx) = tail(&with, FileStart::Begin);
    assert_eq!(lines(&rx, 2), EXPECT);
    set.stop_all();

    let without = dir.path().join("utf8.log");
    std::fs::write(&without, TEXT).unwrap();
    let (set, rx) = tail(&without, FileStart::Begin);
    assert_eq!(lines(&rx, 2), EXPECT);
    set.stop_all();
}

#[test]
fn utf16_both_orders_with_a_mark_and_little_endian_without() {
    let dir = tempfile::tempdir().unwrap();
    let le = dir.path().join("utf16le.log");
    std::fs::write(&le, utf16le(TEXT, true)).unwrap();
    let (set, rx) = tail(&le, FileStart::Begin);
    assert_eq!(lines(&rx, 2), EXPECT);
    set.stop_all();

    let be = dir.path().join("utf16be.log");
    std::fs::write(&be, utf16be(TEXT, true)).unwrap();
    let (set, rx) = tail(&be, FileStart::Begin);
    assert_eq!(lines(&rx, 2), EXPECT);
    set.stop_all();

    let bare = dir.path().join("utf16le-nobom.log");
    std::fs::write(&bare, utf16le("hello sensor world\r\nline two\r\n", false)).unwrap();
    let (set, rx) = tail(&bare, FileStart::Begin);
    assert_eq!(lines(&rx, 2), ["hello sensor world", "line two"]);
    set.stop_all();
}

#[test]
fn cp1252_is_the_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cp1252.log");
    // "Temp 25°C – naïve" in CP1252 (0xB0 the degree sign, 0x96 an en dash, 0xEF ï).
    std::fs::write(&path, b"Temp 25\xB0C \x96 na\xEFve\r\nok\r\n").unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(lines(&rx, 2), ["Temp 25°C – naïve", "ok"]);
    set.stop_all();
}

#[test]
fn multibyte_sequences_split_across_appends_decode_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split.log");
    // A whole line first, so detection commits to UTF-8 before the split appends arrive.
    std::fs::write(&path, "seed\n").unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(lines(&rx, 1), ["seed"]);

    // "é" is C3 A9, split across two appends with a pause between.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"caf\xC3").unwrap();
    f.flush().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    f.write_all(b"\xA9 ready\n").unwrap();
    f.flush().unwrap();
    assert_eq!(lines(&rx, 1), ["café ready"]);
    set.stop_all();
}

#[test]
fn a_utf16_code_unit_split_across_appends_decodes_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split16.log");
    let full = utf16le("seed\r\nvalue 42\r\n", true);
    // Cut mid code unit inside "value 42": the seed line plus 7 bytes lands between the halves.
    let cut = 2 + "seed\r\n".encode_utf16().count() * 2 + 7;
    std::fs::write(&path, &full[..cut]).unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(lines(&rx, 1), ["seed"]);

    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(&full[cut..]).unwrap();
    f.flush().unwrap();
    assert_eq!(lines(&rx, 1), ["value 42"]);
    set.stop_all();
}

#[test]
fn a_torn_final_byte_is_a_replacement_character_never_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("torn.log");
    std::fs::write(&path, b"ok\ncaf\xC3").unwrap();
    let (set, rx) = tail(&path, FileStart::Begin);
    assert_eq!(lines(&rx, 1), ["ok"]);
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
    let rest: Vec<String> = rx.try_iter().map(|e| e.line).collect();
    assert_eq!(rest, ["caf\u{FFFD}"]);
}

// ---- timestamps ------------------------------------------------------------------------------

#[test]
fn tailed_lines_carry_parsed_stamps_and_failures_flow_by_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ts.log");
    std::fs::write(
        &path,
        "2026-07-17 10:00:01 boot\nno stamp on this continuation line\n2026-07-17 10:00:02.500 reading a=1\n",
    )
    .unwrap();
    let spec = TimestampSpec::new(
        r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?)",
        ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"],
    )
    .unwrap();
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FileTailSource::new("t", "ts test", &path, fast_cfg(FileStart::Begin)).with_timestamps(spec)))
        .unwrap();
    let mut events = Vec::new();
    while events.len() < 3 {
        events.push(rx.recv_timeout(RECV_TIMEOUT).unwrap());
    }
    set.stop_all();

    assert_eq!(events[0].parsed_ts.unwrap().to_string(), "2026-07-17 10:00:01");
    assert_eq!(events[0].index_ms(), events[0].parsed_ts.unwrap().and_utc().timestamp_millis());
    assert!(events[1].parsed_ts.is_none());
    assert_eq!(events[1].seq, 1);
    assert_eq!(events[1].index_ms(), events[1].arrival_unix_ms);
    assert_eq!(events[2].parsed_ts.unwrap().format("%Y-%m-%d %H:%M:%S%.3f").to_string(), "2026-07-17 10:00:02.500");
    assert!(events.iter().all(|e| e.arrival_unix_ms > 0));
}

// ---- the plant's writer is never blocked ----------------------------------------------------

fn seed(path: &Path, lines: usize) {
    let mut f = File::create(path).unwrap();
    for i in 0..lines {
        writeln!(f, "2026-07-19 10:00:{:02} seed {i}", i % 60).unwrap();
    }
}

/// The owner as a logger that reopens per write (open, append, close): the reopen asks for
/// write access while the read handle is held, and fails with a sharing violation if the read
/// handle lacks shared write access.
fn owner_append(path: &Path, msg: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().append(true).open(path)?;
    writeln!(f, "{msg}")
}

/// Truncate in place: fails if a memory map is held on the file.
fn owner_truncate(path: &Path, to: u64) -> std::io::Result<()> {
    OpenOptions::new().write(true).open(path)?.set_len(to)
}

fn assert_owner_writes_unblocked(path: &Path, rx: &Receiver<LineEvent>) {
    for i in 0..3 {
        owner_append(path, &format!("owner line {i} while we read")).expect("the owner's append must not be blocked");
    }
    // The tail reads the three lines while the handle is held; only then does the owner
    // truncate: a line overwritten before any poll could see it is lost to every tail.
    let got: Vec<String> = (0..3).map(|_| recv_line(rx)).collect();
    assert_eq!(got, ["owner line 0 while we read", "owner line 1 while we read", "owner line 2 while we read"]);
    owner_truncate(path, 8).expect("the owner's truncate must not be blocked");
}

#[test]
fn a_running_file_source_never_blocks_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plant.log");
    seed(&path, 64);

    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FileTailSource::new("t", "tail", &path, fast_cfg(FileStart::Begin)))).unwrap();
    // The worker holds the handle once the first line is out.
    assert!(recv_line(&rx).contains("seed 0"));
    while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}

    assert_owner_writes_unblocked(&path, &rx);
    // A rotation by rename while the source tails, and a delete: unblocked too.
    let rotated = dir.path().join("rotated.log");
    std::fs::rename(&path, &rotated).expect("the owner's rename must not be blocked while tailing");
    std::fs::write(&path, "after\n").unwrap();
    // What the owner wrote after that reached the tail too: the eight bytes the truncation
    // left (a torn line, flushed when the file was renamed away), then the new file.
    let mut got = Vec::new();
    while got.last().map(String::as_str) != Some("after") {
        got.push(recv_line(&rx));
    }
    assert_eq!(
        got,
        ["2026-07-", "after"]
    );
    std::fs::remove_file(&path).expect("the owner's delete must not be blocked while tailing");

    for (_, r) in set.stop_all() {
        r.unwrap();
    }
}

// ---- local against remote --------------------------------------------------------------------

#[test]
fn unc_paths_are_remote_and_the_rest_classify_without_panicking() {
    assert_eq!(classify(Path::new(r"\\server\share\logs")), PathKind::Remote);
    assert_eq!(classify(Path::new(r"\\?\UNC\server\share\logs\a.log")), PathKind::Remote);
    assert!(!PathKind::Remote.notify_useful());
    assert!(PathKind::Local.notify_useful());
    let _ = classify(Path::new("some/relative/file.log"));
    #[cfg(windows)]
    {
        assert_eq!(classify(Path::new(r"C:\Windows")), PathKind::Local);
        assert_eq!(classify(Path::new(r"\\?\C:\Windows")), PathKind::Local);
    }
    #[cfg(not(windows))]
    assert_eq!(classify(Path::new("/var/log")), PathKind::Local);
}

// ---- the merged stream -----------------------------------------------------------------------

/// Emits a fixed list of lines, then ends.
struct VecSource {
    id: SourceId,
    lines: Vec<String>,
}

impl VecSource {
    fn new(id: &str, lines: &[&str]) -> Box<Self> {
        Box::new(VecSource { id: SourceId::new(id), lines: lines.iter().map(|s| s.to_string()).collect() })
    }
}

impl Source for VecSource {
    fn id(&self) -> &SourceId {
        &self.id
    }
    fn display_name(&self) -> &str {
        self.id.as_str()
    }
    fn start(self: Box<Self>, tx: Sender<LineEvent>) -> Result<SourceHandle, SourceError> {
        let stop = stop_flag();
        let id = self.id.clone();
        let thread_stop = stop.clone();
        let join = std::thread::spawn(move || {
            let mut emitter = Emitter::new(self.id.clone(), tx);
            for line in self.lines {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                if emitter.emit(line).is_err() {
                    break;
                }
            }
            Ok(())
        });
        Ok(SourceHandle::new(id, stop, join))
    }
}

/// Emits forever until stopped, or fails after its lines when told to.
struct EndlessSource {
    id: SourceId,
    fail_after: Option<usize>,
}

impl Source for EndlessSource {
    fn id(&self) -> &SourceId {
        &self.id
    }
    fn display_name(&self) -> &str {
        "endless"
    }
    fn start(self: Box<Self>, tx: Sender<LineEvent>) -> Result<SourceHandle, SourceError> {
        let stop = stop_flag();
        let id = self.id.clone();
        let thread_stop = stop.clone();
        let fail_after = self.fail_after;
        let join = std::thread::spawn(move || {
            let mut emitter = Emitter::new(id, tx);
            let mut n = 0;
            while !thread_stop.load(Ordering::Relaxed) {
                if Some(n) == fail_after {
                    return Err(SourceError::io(
                        Path::new(r"\\server\share\log"),
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "share unreachable"),
                    ));
                }
                if emitter.emit("tick".into()).is_err() {
                    break;
                }
                n += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        });
        Ok(SourceHandle::new(self.id.clone(), stop, join))
    }
}

#[test]
fn the_merged_stream_tags_every_line_and_sequences_per_source() {
    let (mut set, rx) = SourceSet::new();
    set.spawn(VecSource::new("a", &["a0", "a1", "a2"])).unwrap();
    set.spawn(VecSource::new("b", &["b0", "b1"])).unwrap();
    let mut events = Vec::new();
    while events.len() < 5 {
        events.push(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    for id in ["a", "b"] {
        let per: Vec<&LineEvent> = events.iter().filter(|e| e.source.as_str() == id).collect();
        for (i, ev) in per.iter().enumerate() {
            assert_eq!(ev.seq, i as u64, "seq gap for {id}");
            assert_eq!(ev.line, format!("{id}{i}"), "order broken for {id}");
        }
    }
    for (id, result) in set.stop_all() {
        result.unwrap_or_else(|e| panic!("source {id} failed: {e}"));
    }
}

#[test]
fn the_receiver_disconnects_only_after_stop_all() {
    let (mut set, rx) = SourceSet::new();
    set.spawn(VecSource::new("a", &["x"])).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().line, "x");
    // The source ended, but the set still holds a sender: no disconnect yet.
    assert!(matches!(rx.recv_timeout(Duration::from_millis(50)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
    set.stop_all();
    assert!(matches!(rx.recv_timeout(Duration::from_secs(1)), Err(std::sync::mpsc::RecvTimeoutError::Disconnected)));
}

#[test]
fn stop_reaches_a_busy_worker_and_a_dropped_consumer_ends_it() {
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("endless"), fail_after: None })).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().line, "tick");
    for (_, result) in set.stop_all() {
        result.unwrap();
    }

    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("endless"), fail_after: None })).unwrap();
    drop(rx);
    std::thread::sleep(Duration::from_millis(100));
    assert!(set.handles()[0].is_finished(), "the worker kept running into a dead channel");
    for (_, result) in set.stop_all() {
        result.unwrap();
    }
}

/// Poll the reports until the named source reports the wanted state.
fn wait_report(set: &mut SourceSet, id: &str, want_finished: bool) -> veredas_portal::WorkerReport {
    for _ in 0..1000 {
        if let Some(r) = set.worker_reports().into_iter().find(|r| r.id.as_str() == id)
            && r.finished == want_finished
        {
            return r;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for `{id}` finished={want_finished}");
}

#[test]
fn remove_stops_one_source_and_the_rest_keep_producing() {
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("a"), fail_after: None })).unwrap();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("b"), fail_after: None })).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().line, "tick");
    assert!(set.remove(&SourceId::new("nope")).is_none());
    assert!(matches!(set.remove(&SourceId::new("a")), Some(Ok(()))));
    assert_eq!(set.handles().len(), 1);
    assert_eq!(set.handles()[0].id().as_str(), "b");
    while rx.try_recv().is_ok() {}
    let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((ev.source.as_str(), ev.line.as_str()), ("b", "tick"));
    for (_, result) in set.stop_all() {
        result.unwrap();
    }
}

#[test]
fn worker_reports_surface_the_terminal_state_and_remove_returns_the_kept_error() {
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("live"), fail_after: None })).unwrap();
    set.spawn(VecSource::new("done", &["d0", "d1"])).unwrap();
    set.spawn(Box::new(EndlessSource { id: SourceId::new("fail"), fail_after: Some(3) })).unwrap();
    std::thread::spawn(move || while rx.recv().is_ok() {});

    let live = wait_report(&mut set, "live", false);
    assert_eq!(live, veredas_portal::WorkerReport { id: SourceId::new("live"), finished: false, error: None });
    let done = wait_report(&mut set, "done", true);
    assert_eq!(done, veredas_portal::WorkerReport { id: SourceId::new("done"), finished: true, error: None });
    let fail = wait_report(&mut set, "fail", true);
    assert!(fail.finished);
    let msg = fail.error.expect("a failed worker reports its error");
    assert_eq!(msg, r"I/O error on \\server\share\log: share unreachable");
    match set.remove(&SourceId::new("fail")) {
        Some(Err(e)) => assert_eq!(e.to_string(), msg),
        other => panic!("expected the kept error from remove, got {other:?}"),
    }
    set.stop_all();
}

#[test]
fn three_files_tailed_at_once_merge_tagged_with_dense_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["alpha", "beta", "gamma"].iter().map(|n| dir.path().join(format!("{n}.log"))).collect();
    for p in &paths {
        std::fs::write(p, "").unwrap();
    }
    let (mut set, rx) = SourceSet::new();
    for (name, path) in ["alpha", "beta", "gamma"].iter().zip(&paths) {
        set.spawn(Box::new(FileTailSource::new(*name, *name, path, fast_cfg(FileStart::Begin)))).unwrap();
    }
    const PER_SOURCE: u64 = 300;
    let writers: Vec<_> = ["alpha", "beta", "gamma"]
        .iter()
        .zip(&paths)
        .map(|(name, path)| {
            let (name, path) = (name.to_string(), path.clone());
            std::thread::spawn(move || {
                let mut f = OpenOptions::new().append(true).open(path).unwrap();
                for n in 0..PER_SOURCE {
                    writeln!(f, "{name}-{n:04}").unwrap();
                    f.flush().unwrap();
                    if n % 20 == 0 {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            })
        })
        .collect();
    let total = PER_SOURCE as usize * 3;
    let mut next_seq: std::collections::HashMap<SourceId, u64> = Default::default();
    let mut counts: std::collections::HashMap<String, u64> = Default::default();
    let deadline = Instant::now() + Duration::from_secs(60);
    for received in 0..total {
        let remaining = deadline.checked_duration_since(Instant::now()).unwrap_or_else(|| panic!("timed out after {received}/{total}"));
        let ev = rx.recv_timeout(remaining).unwrap_or_else(|e| panic!("stalled at {received}/{total}: {e:?}"));
        let prefix = ev.line.split('-').next().unwrap_or("");
        assert_eq!(prefix, ev.source.as_str(), "line {:?} tagged as {}", ev.line, ev.source);
        let expected = next_seq.entry(ev.source.clone()).or_insert(0);
        assert_eq!(ev.seq, *expected, "seq gap on {}", ev.source);
        assert_eq!(ev.line, format!("{prefix}-{:04}", *expected));
        *expected += 1;
        *counts.entry(prefix.to_string()).or_insert(0) += 1;
    }
    for w in writers {
        w.join().unwrap();
    }
    assert_quiet(&rx, 100);
    for id in ["alpha", "beta", "gamma"] {
        assert_eq!(counts[id], PER_SOURCE, "count for {id}");
    }
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
}

// ---- the critic's first round ------------------------------------------------------------------

/// UTF-16 text, little- or big-endian, with a mark.
fn utf16(text: &str, le: bool) -> Vec<u8> {
    if le { utf16le(text, true) } else { utf16be(text, true) }
}

#[test]
fn a_utf16_file_tailed_from_its_end_emits_the_first_new_line_and_skips_a_torn_one() {
    for le in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        // Ending cleanly on a line feed: the next line is the first new one.
        let clean = dir.path().join("clean.log");
        std::fs::write(&clean, utf16("old\r\n", le)).unwrap();
        let (set, rx) = tail(&clean, FileStart::End);
        assert_quiet(&rx, 150);
        append_bytes(&clean, &utf16("new\r\nnext\r\n", le)[2..]);
        assert_eq!(lines(&rx, 2), ["new", "next"], "little-endian {le}");
        set.stop_all();
        // Ending mid-line: the torn line's completion is skipped, the next whole line is first.
        let torn = dir.path().join("torn.log");
        std::fs::write(&torn, utf16("old\r\nhalf", le)).unwrap();
        let (set, rx) = tail(&torn, FileStart::End);
        assert_quiet(&rx, 150);
        append_bytes(&torn, &utf16("rest\r\nfresh\r\n", le)[2..]);
        assert_eq!(lines(&rx, 1), ["fresh"], "little-endian {le}");
        set.stop_all();
    }
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    let mut f = OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
}

#[test]
fn an_ascii_opening_does_not_commit_the_encoding_before_the_first_byte_beyond_ascii() {
    let dir = tempfile::tempdir().unwrap();
    // An empty file at attach, a header, then CP1252.
    let cp = dir.path().join("cp.log");
    std::fs::write(&cp, "").unwrap();
    let (set, rx) = tail(&cp, FileStart::Begin);
    append(&cp, "header\n");
    assert_eq!(lines(&rx, 1), ["header"]);
    append_bytes(&cp, b"Temp 25\xB0C\n");
    assert_eq!(lines(&rx, 1), ["Temp 25°C"]);
    append_bytes(&cp, b"na\xEFve\n");
    assert_eq!(lines(&rx, 1), ["naïve"], "committed to CP1252 from the first byte beyond ASCII");
    set.stop_all();
    // The same opening, then UTF-8: committed to UTF-8, a later sequence decoded whole.
    let u8 = dir.path().join("u8.log");
    std::fs::write(&u8, "header\nsecond\n").unwrap();
    let (set, rx) = tail(&u8, FileStart::Begin);
    assert_eq!(lines(&rx, 2), ["header", "second"]);
    append(&u8, "café\n");
    assert_eq!(lines(&rx, 1), ["café"]);
    append(&u8, "naïve ünïcode\n");
    assert_eq!(lines(&rx, 1), ["naïve ünïcode"]);
    set.stop_all();
    // Attached at the end of an ASCII file, the appended CP1252 line decodes.
    let late = dir.path().join("late.log");
    std::fs::write(&late, "2026-07-19 10:00:00 seed\n").unwrap();
    let (set, rx) = tail(&late, FileStart::End);
    assert_quiet(&rx, 100);
    append_bytes(&late, b"owner 25\xB0C\n");
    assert_eq!(lines(&rx, 1), ["owner 25°C"]);
    set.stop_all();
}

/// The process's CPU time so far, in milliseconds (user and kernel), as the OS counts it.
fn process_cpu_ms() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the current process's pseudo-handle and four valid out-pointers.
        let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) };
        assert!(ok != 0);
        let hundreds = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        (hundreds(kernel) + hundreds(user)) / 10_000
    }
    #[cfg(not(windows))]
    {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let after = &stat[stat.rfind(')').unwrap() + 2..];
        let fields: Vec<&str> = after.split_whitespace().collect();
        // After the state: utime is the 12th field and stime the 13th, in clock ticks (100 Hz).
        let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
        ticks * 10
    }
}

#[test]
fn a_tail_without_a_watcher_polls_at_its_interval_and_reads_the_file_when_it_comes() {
    // The CPU a loop burns is measured on a process that runs nothing else: this test spawns
    // the test binary again on this one test alone, and that child measures itself and prints
    // its figure. Three loops that never sleep would burn about a core each for 3 s; three
    // loops polling every 25 ms cost a fraction of one second.
    if std::env::var_os("PORTAL_TAIL_CPU_CHILD").is_some() {
        tails_without_a_watcher();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["a_tail_without_a_watcher_polls_at_its_interval_and_reads_the_file_when_it_comes", "--exact", "--test-threads", "1", "--nocapture"])
        .env("PORTAL_TAIL_CPU_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the child failed:\n{stdout}\n{stderr}");
    // The figure shares its line with the harness's own text.
    let at = stdout.find("cpu_ms=").unwrap_or_else(|| panic!("no figure in:\n{stdout}")) + "cpu_ms=".len();
    let spent: u64 = stdout[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap();
    assert!(spent < 1_000, "{spent} ms of CPU over 3 s in a process running the three tails alone: a poll loop is spinning");
}

/// Three tails no watcher serves: a parent folder not there yet (the watcher cannot be set up),
/// the watcher switched off, and the watcher asked for on a folder not there. Prints the CPU
/// the process spent over 3 s, then checks every tail reads its file when it comes.
fn tails_without_a_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("later").join("a.log");
    let plain = dir.path().join("plain.log");
    let forced = dir.path().join("later").join("b.log");
    let mut cfg = fast_cfg(FileStart::End);
    let (mut set, rx) = SourceSet::new();
    set.spawn(Box::new(FileTailSource::new("missing", "m", &missing, cfg.clone()))).unwrap();
    cfg.use_notify = Some(false);
    set.spawn(Box::new(FileTailSource::new("plain", "p", &plain, cfg.clone()))).unwrap();
    cfg.use_notify = Some(true);
    set.spawn(Box::new(FileTailSource::new("forced", "f", &forced, cfg))).unwrap();
    let before = process_cpu_ms();
    std::thread::sleep(Duration::from_secs(3));
    let spent = process_cpu_ms() - before;
    println!("cpu_ms={spent}");
    assert_quiet(&rx, 50);
    std::fs::create_dir_all(dir.path().join("later")).unwrap();
    std::fs::write(&missing, "a1\n").unwrap();
    std::fs::write(&plain, "p1\n").unwrap();
    std::fs::write(&forced, "b1\n").unwrap();
    let mut got: Vec<(String, String)> = (0..3).map(|_| rx.recv_timeout(RECV_TIMEOUT).unwrap()).map(|e| (e.source.to_string(), e.line)).collect();
    got.sort();
    assert_eq!(got, [("forced".to_string(), "b1".to_string()), ("missing".to_string(), "a1".to_string()), ("plain".to_string(), "p1".to_string())]);
    append(&plain, "p2\n");
    assert_eq!(recv_line(&rx), "p2");
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
}

// ---- the critic's second round -----------------------------------------------------------------

#[test]
fn a_file_holding_its_mark_alone_tailed_from_its_end_emits_the_first_line_written() {
    // A logger writes the mark when it creates the file; a tail attached right after sees a
    // file ending on its mark, not on a line feed, and there is no torn line to skip.
    let dir = tempfile::tempdir().unwrap();
    for (name, mark, body) in [
        ("le.log", vec![0xFFu8, 0xFE], utf16le("new\r\nnext\r\n", false)),
        ("be.log", vec![0xFE, 0xFF], utf16be("new\r\nnext\r\n", false)),
        ("u8.log", vec![0xEF, 0xBB, 0xBF], b"new\nnext\n".to_vec()),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, &mark).unwrap();
        let (set, rx) = tail(&path, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&path, &body);
        assert_eq!(lines(&rx, 2), ["new", "next"], "{name}");
        set.stop_all();
    }
}

#[test]
fn an_attach_inside_a_utf16_code_unit_moves_back_to_the_unit_boundary() {
    for le in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let whole = utf16("old\r\nhalf", le);
        // Cut one byte into the last unit of a torn line: that line's completion is skipped and
        // the next whole line is the first.
        let torn = dir.path().join("torn.log");
        let cut = whole.len() - 1;
        std::fs::write(&torn, &whole[..cut]).unwrap();
        let (set, rx) = tail(&torn, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&torn, &whole[cut..]);
        append_bytes(&torn, &utf16("rest\r\nfresh\r\nmore\r\n", le)[2..]);
        assert_eq!(lines(&rx, 2), ["fresh", "more"], "little-endian {le}");
        set.stop_all();
        // Cut one byte into the first unit of a line: the attach moves back to that line's
        // start and the line is whole, not torn.
        let clean = dir.path().join("clean.log");
        let cut = utf16("old\r\n", le).len() + 1;
        std::fs::write(&clean, &whole[..cut]).unwrap();
        let (set, rx) = tail(&clean, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&clean, &whole[cut..]);
        append_bytes(&clean, &utf16("rest\r\nfresh\r\n", le)[2..]);
        assert_eq!(lines(&rx, 2), ["halfrest", "fresh"], "little-endian {le}");
        set.stop_all();
    }
}

#[test]
fn a_torn_utf8_lead_at_the_end_of_an_ascii_opened_file_is_a_replacement_character() {
    // The file opened in plain ASCII, so the encoding is still open when the lead byte of a
    // two-byte sequence lands alone and the stop ends the stream before its continuation.
    let dir = tempfile::tempdir().unwrap();
    let torn = dir.path().join("torn.log");
    std::fs::write(&torn, "ok\n").unwrap();
    let (set, rx) = tail(&torn, FileStart::Begin);
    assert_eq!(lines(&rx, 1), ["ok"]);
    append_bytes(&torn, b"caf\xC3");
    std::thread::sleep(Duration::from_millis(150));
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
    let rest: Vec<String> = rx.try_iter().map(|e| e.line).collect();
    assert_eq!(rest, ["caf\u{FFFD}"]);
    // A byte no UTF-8 sequence starts is a CP1252 character, at the end of the stream or not.
    let cp = dir.path().join("cp.log");
    std::fs::write(&cp, "ok\n").unwrap();
    let (set, rx) = tail(&cp, FileStart::Begin);
    assert_eq!(lines(&rx, 1), ["ok"]);
    append_bytes(&cp, b"25\xB0");
    std::thread::sleep(Duration::from_millis(150));
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
    let rest: Vec<String> = rx.try_iter().map(|e| e.line).collect();
    assert_eq!(rest, ["25°"]);
}

#[test]
fn a_fragment_skipped_at_an_end_attach_never_decides_the_encoding() {
    // An ASCII head longer than the attach reads (over 4 KiB) leaves UTF-8 and CP1252 open, and
    // the file ends inside a two-byte UTF-8 character: the continuation byte is the first thing
    // the tail reads, and alone it is a byte no sequence starts. It belongs to the fragment the
    // attach skips, so it must not commit the file to CP1252.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("head.log");
    let mut head: Vec<u8> =
        (0..200).flat_map(|i| format!("2026-07-19 10:{:02}:{:02} seed {i} ascii only\n", i / 60, i % 60).into_bytes()).collect();
    assert!(head.len() > 4096);
    head.extend_from_slice(b"caf\xC3");
    std::fs::write(&path, &head).unwrap();
    let (set, rx) = tail(&path, FileStart::End);
    assert_quiet(&rx, 100);
    append_bytes(&path, b"\xA9\nna\xC3\xAFve\n");
    append_bytes(&path, "café 25°\n".as_bytes());
    assert_eq!(lines(&rx, 2), ["naïve", "café 25°"]);
    set.stop_all();
}

#[test]
fn a_head_under_four_bytes_decides_with_what_follows_it() {
    // A tail attached while a new UTF-16 file holds under four bytes: the writer caught inside
    // its mark, or one character in. Too little to decide from, so the bytes vote with what
    // follows them, as a start from the beginning would. A torn mark is no text yet, and the
    // first line written is whole; one character is a torn line, whose completion is skipped.
    let dir = tempfile::tempdir().unwrap();
    for (name, first, rest) in [
        ("le.log", vec![0xFFu8], [vec![0xFE], utf16le("new\r\nnext\r\n", false)].concat()),
        ("be.log", vec![0xFE], [vec![0xFF], utf16be("new\r\nnext\r\n", false)].concat()),
        ("u8.log", vec![0xEF, 0xBB], [vec![0xBF], b"new\nnext\n".to_vec()].concat()),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, &first).unwrap();
        let (set, rx) = tail(&path, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&path, &rest);
        assert_eq!(lines(&rx, 2), ["new", "next"], "{name}");
        set.stop_all();
    }
    for le in [true, false] {
        let path = dir.path().join(format!("char-{le}.log"));
        std::fs::write(&path, if le { utf16le("a", false) } else { utf16be("a", false) }).unwrap();
        let (set, rx) = tail(&path, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&path, &if le { utf16le("bc\r\nnext\r\nmore\r\n", false) } else { utf16be("bc\r\nnext\r\nmore\r\n", false) });
        assert_eq!(lines(&rx, 2), ["next", "more"], "little-endian {le}");
        set.stop_all();
    }
}

#[test]
fn a_short_head_that_proves_no_mark_is_text_from_before_the_attach_and_never_emitted() {
    // One CP1252 thorn (the byte a UTF-16 mark begins with), and one UTF-8 character opening
    // like the UTF-8 mark: text written before the attach, skipped with the torn line it opens.
    let dir = tempfile::tempdir().unwrap();
    for (name, first, rest) in [("thorn.log", vec![0xFEu8], b"rest\nnext\n".to_vec()), ("wide.log", vec![0xEF], b"\xBC\x81rest\nnext\n".to_vec())] {
        let path = dir.path().join(name);
        std::fs::write(&path, &first).unwrap();
        let (set, rx) = tail(&path, FileStart::End);
        assert_quiet(&rx, 100);
        append_bytes(&path, &rest);
        assert_eq!(lines(&rx, 1), ["next"], "{name}");
        assert_quiet(&rx, 100);
        set.stop_all();
    }
    // A UTF-16 file without a mark opening on a bare line feed: the line-feed shortcut waits for
    // the pattern when a NUL rides with it, so the file is UTF-16 from the end and from the
    // beginning alike.
    let end = dir.path().join("feed-end.log");
    std::fs::write(&end, [0x0A, 0x00]).unwrap();
    let (set, rx) = tail(&end, FileStart::End);
    assert_quiet(&rx, 100);
    append_bytes(&end, &utf16le("next\r\nmore\r\n", false));
    assert_eq!(lines(&rx, 2), ["next", "more"]);
    set.stop_all();
    let begin = dir.path().join("feed-begin.log");
    std::fs::write(&begin, [0x0A, 0x00]).unwrap();
    let (set, rx) = tail(&begin, FileStart::Begin);
    append_bytes(&begin, &utf16le("next\r\nmore\r\n", false));
    assert_eq!(lines(&rx, 3), ["", "next", "more"]);
    set.stop_all();
    // Stopped before the short head decided: nothing of it is a line.
    let torn = dir.path().join("torn.log");
    std::fs::write(&torn, [0xFE]).unwrap();
    let (set, rx) = tail(&torn, FileStart::End);
    append_bytes(&torn, b"re");
    std::thread::sleep(Duration::from_millis(150));
    for (_, r) in set.stop_all() {
        r.unwrap();
    }
    assert_eq!(rx.try_iter().map(|e| e.line).collect::<Vec<_>>(), Vec::<String>::new());
}

#[test]
fn a_short_head_decided_on_its_own_line_feed_starts_clean_or_mid_line_as_it_ends() {
    // Two CP1252 bytes with a line feed and no NUL decide on their own; the head's text is
    // discarded, and the attach is clean when the head ended on that feed.
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("clean.log");
    std::fs::write(&clean, b"\xE9\n").unwrap();
    let (set, rx) = tail(&clean, FileStart::End);
    assert_quiet(&rx, 100);
    append_bytes(&clean, b"Temp 25\xB0C\n");
    assert_eq!(lines(&rx, 1), ["Temp 25°C"]);
    set.stop_all();
    // A byte after the feed is a torn line: its completion is skipped.
    let torn = dir.path().join("torn.log");
    std::fs::write(&torn, b"\xE9\nA").unwrap();
    let (set, rx) = tail(&torn, FileStart::End);
    assert_quiet(&rx, 100);
    append_bytes(&torn, b"rest\nnext\n");
    assert_eq!(lines(&rx, 1), ["next"]);
    assert_quiet(&rx, 100);
    set.stop_all();
}

// ---- the start's attach ------------------------------------------------------------------------

/// A start returns once the worker has looked at the path, so a line written right after it is
/// read, never taken as history. Found by the server's stream test, which wrote right after the
/// session started and saw its lines skipped one run in three under load.
#[test]
fn a_start_returns_attached_so_a_line_written_right_after_is_read() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..30 {
        let path = dir.path().join(format!("live-{i}.log"));
        std::fs::write(&path, "history\n").unwrap();
        let (set, rx) = tail(&path, FileStart::End);
        append(&path, "new\n");
        assert_eq!(recv_line(&rx), "new", "round {i}");
        set.stop_all();
    }
    // Absent at the start: awaited, and read whole when it appears right after.
    let late = dir.path().join("late").join("a.log");
    let (set, rx) = tail(&late, FileStart::End);
    std::fs::create_dir_all(late.parent().unwrap()).unwrap();
    std::fs::write(&late, "born\n").unwrap();
    assert_eq!(recv_line(&rx), "born");
    set.stop_all();
}

/// A file present but held against reading at the start (a writer with no sharing, a share's
/// passing error) is awaited at the configured start, never read whole as if it had just
/// appeared: its history stays history.
#[cfg(windows)]
#[test]
fn a_file_held_at_the_start_is_awaited_at_its_configured_start_and_its_history_stays_history() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("held.log");
    std::fs::write(&path, "history 1\nhistory 2\n").unwrap();
    // No sharing at all: every open fails until the handle is dropped.
    let lock = OpenOptions::new().read(true).write(true).share_mode(0).open(&path).unwrap();
    let started = Instant::now();
    let (set, rx) = tail(&path, FileStart::End);
    assert!(started.elapsed() < Duration::from_secs(5), "the start waited {:?}", started.elapsed());
    std::thread::sleep(Duration::from_millis(200));
    drop(lock);
    std::thread::sleep(Duration::from_millis(200));
    append(&path, "new\n");
    assert_eq!(recv_line(&rx), "new");
    assert_quiet(&rx, 200);
    set.stop_all();
}
