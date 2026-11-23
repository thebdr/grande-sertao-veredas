//! A target's clock: monotonic, never early, keeping the wall's pace. Only what holds whatever the machine's load is
//! asserted: a wait returns at its deadline or after it, never before; a task's scans start at their deadlines or
//! after them; the clock, read after the wall, is never behind the wall by more than the wall's head start, between
//! waits as well as at their ends; and a scan that takes its program five milliseconds is measured as at least that.
//! That the clock only counts forward is read in the source, since no test can step the system's clock without
//! privileges: it is `std::time::Instant`, never the settable `SystemTime` that NTP or a rewind moves.
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
const SOURCES: [(&str, &str); 3] = [
    ("clock.rs", include_str!("../src/clock.rs")),
    ("lib.rs", include_str!("../src/lib.rs")),
    ("task.rs", include_str!("../src/task.rs")),
];

/// A source with its comment lines taken out and every whitespace character removed.
fn code_squeezed(text: &str) -> String {
    let code: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
    code.concat().chars().filter(|c| !c.is_whitespace()).collect()
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
    }
    let clock = code_squeezed(SOURCES[0].1);
    for needed in [
        "usestd::time::{Duration,Instant};",
        "pubstructMonoClock{start:Instant,",
        "start:Instant::now(),",
        "implClockforMonoClock{fnnow(&self)->Duration{self.start.elapsed()}",
    ] {
        assert!(clock.contains(needed), "MonoClock's reading is std's Instant: {needed}");
    }
}
