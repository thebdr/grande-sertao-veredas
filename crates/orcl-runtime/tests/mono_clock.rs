//! A target's clock: monotonic, never early, keeping the wall's pace. Only what holds whatever the machine's load is
//! asserted: a wait returns at its deadline or after it, never before; a task's scans start at their deadlines or
//! after them; the clock, read after the wall, is never behind the wall by more than the wall's head start, between
//! waits as well as at their ends; and a scan that takes its program five milliseconds is measured as at least that.
//! That the clock only counts forward is read in the source, since no test can step the system's clock without
//! privileges: it is `std::time::Instant`, never the settable `SystemTime` that NTP or a rewind moves, and its wait
//! judges the deadline on that reading. Since another crate or unsafe code would reach the system's clock, or the
//! operating system, without naming either, cargo's own report pins the engine to orcl-std and orcl-interp all the
//! way down, with every feature on, and each root to `forbid(unsafe_code)`; none of the three runs a build script,
//! which could link a native library defining the system's clock without any of that; std is used only at the paths
//! the engine needs, and the interpreter's sources name it nowhere.
use orcl_runtime::{Clock, Io, MonoClock, RunState, Task, TaskConfig};
use orcl_std::{Program, Scan};
use std::time::{Duration, Instant};

/// Spins for `d` on the wall clock, whatever the target's clock says.
fn spin_on_the_wall(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

#[test]
fn a_wait_never_returns_before_its_deadline_and_the_clock_never_goes_back() {
    // a wall clock started before the target's clock, so the target's own reading is judged too
    let wall = Instant::now();
    let mut clock = MonoClock::new();
    let mut last = clock.now();
    // from 50 µs to 100 ms: the waits a fast task and a slow one make every cycle, past the spin's margin
    for step in [50_u64, 200, 400, 1_000, 3_000, 6_000, 10_000, 20_000, 25_000, 50_000, 100_000] {
        let deadline = clock.now() + Duration::from_micros(step);
        clock.sleep_until(deadline);
        let now = clock.now();
        assert!(now >= deadline, "woke at {now:?}, before its deadline {deadline:?}");
        assert!(wall.elapsed() >= deadline, "the wall says {:?}, before the deadline {deadline:?}", wall.elapsed());
        assert!(now >= last);
        last = now;
    }
    let past = clock.now();
    clock.sleep_until(Duration::ZERO);
    assert!(clock.now() >= past, "a deadline in the past returns at once");
}

#[test]
fn the_clock_keeps_the_wall_s_pace_never_falling_behind_it() {
    // the wall starts first; its head start is measured once the clock has started, so it is at least the true one.
    // Read the wall, then the clock: whatever happens between the two readings only puts the clock further ahead.
    // Each reading comes after 100 µs of spinning on the wall, so a clock that moves only at a wait's end is caught.
    let wall = Instant::now();
    let mut clock = MonoClock::new();
    let head_start = wall.elapsed();
    for step in [0_u64, 50, 1_000, 5_000, 20_000] {
        let deadline = clock.now() + Duration::from_micros(step);
        clock.sleep_until(deadline);
        spin_on_the_wall(Duration::from_micros(100));
        let on_the_wall = wall.elapsed();
        let now = clock.now();
        assert!(now + head_start >= on_the_wall, "the clock reads {now:?} where the wall says {on_the_wall:?}");
    }
}

struct Echo;

impl Program for Echo {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, _scan: &Scan, inputs: &u64, outputs: &mut u64) {
        *outputs = *inputs;
    }
}

struct Feed(u64);

impl Io<u64, u64> for Feed {
    fn read_inputs(&mut self, _now: Duration) -> u64 {
        self.0 += 1;
        self.0
    }

    fn write_outputs(&mut self, _now: Duration, _outputs: &u64) {}
}

#[test]
fn on_a_target_clock_every_scan_starts_at_its_deadline_or_after_it() {
    let period = Duration::from_millis(2);
    let mut task = Task::new(TaskConfig { name: "main", period }, Echo).unwrap();
    let wall = Instant::now();
    let mut clock = MonoClock::new();
    let mut seen = 0_u64;
    task.run_until(&mut Feed(0), &mut clock, Duration::from_millis(60), |r| {
        let due = period * u32::try_from(r.scan.cycle).unwrap();
        assert!(r.scan.now >= due, "scan {} started at {:?}, before {due:?}", r.scan.cycle, r.scan.now);
        assert!(wall.elapsed() >= due, "the wall says {:?}, before {due:?}", wall.elapsed());
        seen += 1;
    });
    let s = *task.stats();
    assert_eq!(seen, s.cycles);
    assert_eq!(s.cycles + s.skipped, task.next_cycle(), "every cycle before the next one ran or was counted as skipped");
    assert!(task.next_due() >= Duration::from_millis(60));
    assert!(s.cycles >= 1);
}

/// Spins 5 ms on the wall every scan: past twice a 2 ms period under any load.
struct Busy;

impl Program for Busy {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, _scan: &Scan, _inputs: &u64, _outputs: &mut u64) {
        spin_on_the_wall(Duration::from_millis(5));
    }
}

#[test]
fn on_a_target_clock_a_scan_past_twice_its_period_is_measured_whole_and_faults() {
    let period = Duration::from_millis(2);
    let mut task = Task::new(TaskConfig { name: "busy", period }, Busy).unwrap();
    let mut clock = MonoClock::new();
    let report = task.step(&mut Feed(0), &mut clock);
    assert!(report.exec >= Duration::from_millis(5), "a 5 ms scan measured {:?}", report.exec);
    assert!(report.time_error, "a time error");
    assert!(matches!(report.state, RunState::Faulted { .. }), "and a fault: {:?}", report.state);
}

/// Every file of the crate's `src/`.
const SOURCES: [(&str, &str); 4] = [
    ("clock.rs", include_str!("../src/clock.rs")),
    ("interpreted.rs", include_str!("../src/interpreted.rs")),
    ("lib.rs", include_str!("../src/lib.rs")),
    ("task.rs", include_str!("../src/task.rs")),
];

/// Every file of `src/` of the interpreter the engine uses, `orcl-interp`.
const INTERPRETER: [(&str, &str); 7] = [
    ("core_ops.rs", include_str!("../../orcl-interp/src/core_ops.rs")),
    ("file.rs", include_str!("../../orcl-interp/src/file.rs")),
    ("layout.rs", include_str!("../../orcl-interp/src/layout.rs")),
    ("lib.rs", include_str!("../../orcl-interp/src/lib.rs")),
    ("machine.rs", include_str!("../../orcl-interp/src/machine.rs")),
    ("ops.rs", include_str!("../../orcl-interp/src/ops.rs")),
    ("program.rs", include_str!("../../orcl-interp/src/program.rs")),
];

/// The paths at which the engine uses std, as they read with whitespace removed: std brought into the `no_std`
/// root by name, shared time for the virtual clock, the monotonic `Instant`, the sleep of the target's wait, and the
/// vector that holds the slots an interpreted program's regions are handed, reserved when it is made. Everything else
/// the engine uses comes from `core`, which reaches no operating system.
const STD_PATHS: [&str; 5] = [
    "externcratestd;",
    "usestd::rc::Rc;",
    "usestd::time::Instant;",
    "std::thread::sleep(remaining.saturating_sub(self.spin));",
    "usestd::vec::Vec;",
];

/// The words of a source's code, its comment lines left out.
fn words(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
    without_block_comments(&lines.join("\n"))
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A source with its comment lines and block comments taken out and every whitespace character removed.
fn code_squeezed(text: &str) -> String {
    let code: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
    without_block_comments(&code.join("\n")).chars().filter(|c| !c.is_whitespace()).collect()
}

/// A text with its block comments taken out, nested ones included.
fn without_block_comments(text: &str) -> String {
    let (mut out, mut depth, mut rest) = (String::new(), 0_usize, text);
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("/*") {
            depth += 1;
            rest = &rest[2..];
        } else if depth > 0 && rest.starts_with("*/") {
            depth -= 1;
            rest = &rest[2..];
        } else {
            if depth == 0 {
                out.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

#[test]
fn the_target_s_clock_reads_the_monotonic_instant_never_the_settable_system_time() {
    let listed = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap().count();
    assert_eq!(listed, SOURCES.len(), "every file of src/ is read");
    for (file, text) in SOURCES {
        let code = code_squeezed(text);
        for refused in ["SystemTime", "UNIX_EPOCH"] {
            assert!(!code.contains(refused), "{file} names {refused}, the clock the system's time setting moves");
        }
        // and no file outside src/ is taken in, where none of this would be read
        for refused in ["#[path", "include!(", "include_str!(", "include_bytes!("] {
            assert!(!code.contains(refused), "{file} takes a file in with {refused}");
        }
        // std only at the paths the engine needs: no thread started, no file, socket, environment or process
        let lines: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
        let words = lines.join("\n");
        let std_words = words.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| *w == "std").count();
        let allowed = STD_PATHS.iter().map(|path| code.matches(path).count()).sum::<usize>();
        assert_eq!(std_words, allowed, "{file} reaches std at a path the engine does not need");
    }
    let clock = code_squeezed(SOURCES[0].1);
    for needed in [
        "usestd::time::Instant;",
        "pubstructMonoClock{start:Instant,",
        "start:Instant::now(),",
        "implClockforMonoClock{fnnow(&self)->Duration{self.start.elapsed()}",
        "letremaining=deadline.saturating_sub(self.now());",
        "whileself.now()<deadline{core::hint::spin_loop();}",
    ] {
        assert!(clock.contains(needed), "MonoClock's reading is std's Instant: {needed}");
    }
}

#[test]
fn the_engine_depends_on_orcl_std_and_orcl_interp_alone_and_forbids_unsafe_code() {
    // cargo's own report of the crate's normal and build dependencies, on every target, all the way down
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = std::process::Command::new(env!("CARGO"))
        .args(["tree", "-p", "orcl-runtime", "-e", "normal,build", "--prefix", "none", "--all-features"])
        .args(["--target", "all", "--offline", "--manifest-path", manifest])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let listed = String::from_utf8(out.stdout).unwrap();
    let crates: Vec<&str> = listed.lines().filter_map(|line| line.split_whitespace().next()).collect();
    assert_eq!(crates, ["orcl-runtime", "orcl-interp", "orcl-std"], "the engine and the two crates it uses, which use none");
    let root = code_squeezed(SOURCES[2].1);
    assert!(root.contains("#![forbid(unsafe_code)]"), "lib.rs forbids unsafe code");
    let interpreter_root = code_squeezed(INTERPRETER[3].1);
    assert!(interpreter_root.contains("#![forbid(unsafe_code)]"), "the interpreter's lib.rs forbids unsafe code");
    // no_std, std brought in by name and no macro of it imported: every use of std is written out and counted
    assert!(root.contains("#![no_std]") && root.contains("externcratestd;"), "lib.rs: no_std, std by name");
    for (file, text) in SOURCES {
        assert!(!code_squeezed(text).contains("macro_use"), "{file} imports macros wholesale");
    }
}

#[test]
fn no_crate_of_the_engine_runs_a_build_script() {
    // a build script links what it likes, a static archive defining libc's clock among it, with no crate in cargo's
    // tree, no unsafe code and no line of src/: cargo runs one found at a crate's root or named by its manifest
    for krate in ["orcl-runtime", "orcl-interp", "orcl-std"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(krate);
        assert!(!dir.join("build.rs").exists(), "{krate} has a build script");
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        for line in manifest.lines().filter(|line| !line.trim_start().starts_with('#')) {
            let Some((key, _)) = line.split_once('=') else { continue };
            let last = key.rsplit('.').next().unwrap_or(key).trim().trim_matches(['"', '\'']);
            assert!(last != "build" && last != "links", "{krate}'s manifest names `{}`", key.trim());
        }
    }
}

#[test]
fn the_interpreter_the_engine_uses_names_no_std_prints_nothing_and_takes_no_file_in() {
    // orcl-interp has std's prelude alone: no path into std, so no clock, thread, file, socket, environment or
    // process; no print to the terminal; no file taken in from elsewhere, where none of this would be read
    let listed = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../orcl-interp/src")).unwrap().count();
    assert_eq!(listed, INTERPRETER.len(), "every file of the interpreter's src/ is read");
    for (file, text) in INTERPRETER {
        let words = words(text);
        for refused in ["std", "SystemTime", "UNIX_EPOCH", "Instant", "print", "println", "eprint", "eprintln", "dbg"] {
            assert!(!words.iter().any(|w| w == refused), "the interpreter's {file} names {refused}");
        }
        let code = code_squeezed(text);
        for refused in ["#[path", "include!(", "include_str!(", "include_bytes!(", "externcrate", "macro_use"] {
            assert!(!code.contains(refused), "the interpreter's {file} holds {refused}");
        }
    }
}
