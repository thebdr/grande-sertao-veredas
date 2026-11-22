//! The engine of a scan on virtual time: one cyclic task over a process image, absolute deadlines, skipped cycles,
//! the time-error rule and safe outputs, every figure exact because the clock moves only when the test moves it.
use orcl_runtime::{Clock, Fault, Io, RunState, Task, TaskConfig, TaskError, VirtualClock};
use orcl_std::{Program, Scan};
use std::time::Duration;

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn us(v: u64) -> Duration {
    Duration::from_micros(v)
}

/// Adds one to its input; safe outputs are 0. What it saw last is kept, so a test can read the scan it was given.
#[derive(Default)]
struct Count {
    last_scan: Scan,
}

impl Program for Count {
    type Inputs = u32;
    type Outputs = u32;

    fn scan(&mut self, scan: &Scan, inputs: &u32, outputs: &mut u32) {
        self.last_scan = *scan;
        *outputs = inputs.saturating_add(1);
    }

    fn safe_outputs(&self) -> u32 {
        0
    }
}

/// A plant that feeds each scan the outputs of the one before, and stands for the time a scan takes: the clock
/// moves by `cost(cycle)` between the inputs' read and the outputs' write. Every read and write is logged with
/// its time.
struct Loop {
    clock: VirtualClock,
    value: u32,
    reads: Vec<Duration>,
    writes: Vec<(Duration, u32)>,
    cost: fn(usize) -> Duration,
}

impl Loop {
    fn new(clock: &VirtualClock, cost: fn(usize) -> Duration) -> Self {
        Self {
            clock: clock.clone(),
            value: 0,
            reads: Vec::new(),
            writes: Vec::new(),
            cost,
        }
    }
}

impl Io<u32, u32> for Loop {
    fn read_inputs(&mut self, now: Duration) -> u32 {
        self.reads.push(now);
        self.clock.advance((self.cost)(self.reads.len() - 1));
        self.value
    }

    fn write_outputs(&mut self, now: Duration, outputs: &u32) {
        self.writes.push((now, *outputs));
        self.value = *outputs;
    }
}

fn task(period: Duration) -> Task<Count> {
    Task::new(TaskConfig { name: "main", period }, Count::default()).unwrap()
}

#[test]
fn a_period_of_zero_is_refused() {
    let err = Task::new(TaskConfig { name: "main", period: Duration::ZERO }, Count::default()).err();
    assert_eq!(err, Some(TaskError::ZeroPeriod { task: "main" }));
    assert_eq!(err.unwrap().to_string(), "task `main`: a period of zero");
}

#[test]
fn one_scan_per_period_through_the_process_image_on_absolute_deadlines() {
    let mut clock = VirtualClock::new();
    let mut plant = Loop::new(&clock, |_| us(3_000));
    let mut t = task(ms(10));
    let mut seen = Vec::new();
    t.run_until(&mut plant, &mut clock, ms(1_000), |r| {
        seen.push((r.scan.cycle, r.scan.now, r.scan.period, *r.inputs, *r.outputs, r.exec, r.time_error));
    });
    assert_eq!(seen.len(), 100, "deadlines 0, 10, ..., 990 ms: 100 scans before 1 s");
    for (k, &(cycle, now, period, input, output, exec, time_error)) in seen.iter().enumerate() {
        let k64 = k as u64;
        assert_eq!(cycle, k64);
        assert_eq!(now, ms(10 * k64), "scan {k} starts on its absolute deadline: 3 ms of work never drifts it");
        assert_eq!(period, ms(10));
        assert_eq!(input, k as u32, "scan {k} reads what scan {} wrote", k as i64 - 1);
        assert_eq!(output, k as u32 + 1);
        assert_eq!(exec, us(3_000));
        assert!(!time_error);
    }
    assert_eq!(plant.reads, (0..100).map(|k| ms(10 * k)).collect::<Vec<_>>(), "inputs read once per scan, at its start");
    assert_eq!(
        plant.writes,
        (0..100).map(|k| (ms(10 * k) + us(3_000), k as u32 + 1)).collect::<Vec<_>>(),
        "outputs written once per scan, at its end"
    );
    let s = *t.stats();
    assert_eq!((s.cycles, s.overruns, s.skipped), (100, 0, 0));
    assert_eq!((s.max_lateness, s.mean_lateness()), (Duration::ZERO, Duration::ZERO));
    assert_eq!((s.max_exec, s.mean_exec(), s.total_exec), (us(3_000), us(3_000), ms(300)));
    assert_eq!(t.state(), RunState::Running);
    assert_eq!(t.next_cycle(), 100);
    assert_eq!(t.next_due(), ms(1_000));
    assert_eq!(t.program().last_scan.now, ms(990), "the program is handed the scan's time, read once");
}

#[test]
fn a_scan_of_exactly_one_period_is_no_time_error_and_skips_nothing() {
    let mut clock = VirtualClock::new();
    let mut plant = Loop::new(&clock, |_| ms(10));
    let mut t = task(ms(10));
    let mut starts = Vec::new();
    t.run_until(&mut plant, &mut clock, ms(50), |r| starts.push((r.scan.now, r.time_error)));
    assert_eq!(starts, vec![(ms(0), false), (ms(10), false), (ms(20), false), (ms(30), false), (ms(40), false)]);
    assert_eq!((t.stats().overruns, t.stats().skipped), (0, 0));
}

#[test]
fn a_late_scan_is_a_time_error_and_skips_the_deadlines_it_missed_never_catching_up() {
    let mut clock = VirtualClock::new();
    // the third scan run takes 15 ms (past its period, under twice it); the fifth takes exactly 20 ms, twice the
    // period: the limit itself, so a time error and no fault; every other scan takes 1 ms
    let mut plant = Loop::new(&clock, |k| match k {
        2 => ms(15),
        4 => ms(20),
        _ => ms(1),
    });
    let mut t = task(ms(10));
    let mut seen = Vec::new();
    t.run_until(&mut plant, &mut clock, ms(100), |r| {
        seen.push((r.scan.cycle, r.scan.now, r.exec, r.time_error, r.stats.skipped))
    });
    assert_eq!(
        seen[..6],
        [
            (0, ms(0), ms(1), false, 0),
            (1, ms(10), ms(1), false, 0),
            (2, ms(20), ms(15), true, 1), // ends at 35 ms: the deadline at 30 ms has passed
            (4, ms(40), ms(1), false, 1), // cycle 3 never runs
            (5, ms(50), ms(20), true, 2), // ends at 70 ms exactly: 60 ms passed, 70 ms runs now
            (7, ms(70), ms(1), false, 2),
        ]
    );
    assert_eq!(t.stats().overruns, 2);
    assert_eq!(t.state(), RunState::Running, "twice the period is the limit, not past it");
    assert_eq!(plant.writes[2], (ms(35), 3), "a time error still writes the scan's own outputs");
}

#[test]
fn a_scan_past_twice_its_period_faults_and_never_writes_its_own_outputs() {
    let mut clock = VirtualClock::new();
    let mut plant = Loop::new(&clock, |k| if k == 3 { ms(20) + Duration::from_nanos(1) } else { ms(1) });
    let mut t = task(ms(10));
    let mut seen = Vec::new();
    t.run_until(&mut plant, &mut clock, ms(100), |r| seen.push((r.scan.cycle, r.scan.now, *r.outputs, r.state)));
    let fault = RunState::Faulted {
        at: ms(30),
        fault: Fault::Watchdog { exec: ms(20) + Duration::from_nanos(1) },
    };
    assert_eq!(seen[2], (2, ms(20), 3, RunState::Running));
    assert_eq!(seen[3], (3, ms(30), 0, fault), "the faulting scan writes the safe outputs, never its own 4");
    assert_eq!(plant.writes[3], (ms(50) + Duration::from_nanos(1), 0));
    // the deadlines at 40 and 50 ms passed: the next scan is at 60 ms, and every scan after the fault applies the
    // safe outputs without scanning the program, so the plant's exchange goes on
    let after: Vec<_> = seen[4..].iter().map(|&(c, now, out, st)| (c, now, out, st)).collect();
    assert_eq!(after, vec![(6, ms(60), 0, fault), (7, ms(70), 0, fault), (8, ms(80), 0, fault), (9, ms(90), 0, fault)]);
    assert_eq!(t.program().last_scan.cycle, 3, "the program is not scanned once faulted");
    assert_eq!(plant.reads.len(), 8, "the inputs are still read every scan");
    let s = *t.stats();
    assert_eq!((s.cycles, s.overruns, s.skipped), (8, 1, 2));
}

#[test]
fn the_same_inputs_and_times_give_the_same_scans() {
    fn run() -> Vec<(u64, Duration, u32, Duration)> {
        let mut clock = VirtualClock::new();
        let mut plant = Loop::new(&clock, |k| ms((k as u64 * 7) % 23));
        let mut t = task(ms(10));
        let mut seen = Vec::new();
        t.run_until(&mut plant, &mut clock, ms(5_000), |r| seen.push((r.scan.cycle, r.scan.now, *r.outputs, r.exec)));
        seen
    }
    let first = run();
    assert!(first.len() > 100);
    assert_eq!(first, run());
}

#[test]
fn virtual_time_never_waits_and_never_goes_back() {
    let mut clock = VirtualClock::new();
    clock.sleep_until(ms(5));
    assert_eq!(clock.now(), ms(5));
    clock.sleep_until(ms(2));
    assert_eq!(clock.now(), ms(5), "a deadline in the past returns at once");
    let shared = clock.clone();
    shared.advance(ms(1));
    assert_eq!(clock.now(), ms(6), "clones share one time");
}
