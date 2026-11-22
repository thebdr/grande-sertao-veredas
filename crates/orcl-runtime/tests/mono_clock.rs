//! A target's clock: monotonic, never early. Only what holds whatever the machine's load is asserted: a wait
//! returns at its deadline or after it, never before, and a task's scans start at their deadlines or after them.
use orcl_runtime::{Clock, Io, MonoClock, Task, TaskConfig};
use orcl_std::{Program, Scan};
use std::time::Duration;

#[test]
fn a_wait_never_returns_before_its_deadline_and_the_clock_never_goes_back() {
    let mut clock = MonoClock::new();
    let mut last = clock.now();
    for step in [50_u64, 200, 400, 1_000, 3_000, 6_000, 10_000, 20_000] {
        let deadline = clock.now() + Duration::from_micros(step);
        clock.sleep_until(deadline);
        let now = clock.now();
        assert!(now >= deadline, "woke at {now:?}, before its deadline {deadline:?}");
        assert!(now >= last);
        last = now;
    }
    let past = clock.now();
    clock.sleep_until(Duration::ZERO);
    assert!(clock.now() >= past, "a deadline in the past returns at once");
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
    let mut clock = MonoClock::new();
    let mut seen = 0_u64;
    task.run_until(&mut Feed(0), &mut clock, Duration::from_millis(60), |r| {
        let due = period * u32::try_from(r.scan.cycle).unwrap();
        assert!(r.scan.now >= due, "scan {} started at {:?}, before {due:?}", r.scan.cycle, r.scan.now);
        seen += 1;
    });
    let s = *task.stats();
    assert_eq!(seen, s.cycles);
    assert_eq!(s.cycles + s.skipped, task.next_cycle(), "every cycle before the next one ran or was counted as skipped");
    assert!(task.next_due() >= Duration::from_millis(60));
    assert!(s.cycles >= 1);
}
