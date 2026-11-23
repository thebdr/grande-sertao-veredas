//! A target's clock: monotonic, never early, keeping the wall's pace. Only what holds whatever the machine's load is
//! asserted: a wait returns at its deadline or after it, never before; a task's scans start at their deadlines or
//! after them; and the clock, read after the wall, is never behind the wall by more than the wall's head start.
use orcl_runtime::{Clock, Io, MonoClock, Task, TaskConfig};
use orcl_std::{Program, Scan};
use std::time::Duration;

#[test]
fn a_wait_never_returns_before_its_deadline_and_the_clock_never_goes_back() {
    // a wall clock started before the target's clock, so the target's own reading is judged too
    let wall = std::time::Instant::now();
    let mut clock = MonoClock::new();
    let mut last = clock.now();
    for step in [50_u64, 200, 400, 1_000, 3_000, 6_000, 10_000, 20_000] {
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
    let wall = std::time::Instant::now();
    let mut clock = MonoClock::new();
    let head_start = wall.elapsed();
    for step in [0_u64, 50, 1_000, 5_000, 20_000] {
        let deadline = clock.now() + Duration::from_micros(step);
        clock.sleep_until(deadline);
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
    let wall = std::time::Instant::now();
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
