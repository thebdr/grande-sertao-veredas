//! The engine of a scan against a reference model of C-151, over thousands of random sequences on virtual time.
//!
//! The model is written from the clause, apart from the engine, in whole nanoseconds: a scan is due at `k` periods
//! from the clock's zero and starts there, or when the host steps it if later; it reads its inputs at its start, the
//! program scans while the task runs, and the scan ends when its outputs are handed to the plant; a scan longer than
//! its period is a time error, one longer than twice its period faults the task, dated at its start, its own outputs
//! never written but the program's safe outputs, which every later scan writes without scanning the program; the
//! next scan is the first whose deadline is not before the end, the others skipped and counted. The sequences are
//! drawn so that starts, ends and writes land on deadlines, a nanosecond either side of them, and at one and two
//! periods, with periods from a microsecond to two seconds; the program's safe outputs depend on its own state.
//! After every step the model and the engine must agree on everything the clause states.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use orcl_runtime::{Clock, Fault, Io, RunState, Task, TaskConfig, VirtualClock};
use orcl_std::{Program, Scan};

/// A small deterministic generator (xorshift64*), so every run draws the same sequences.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u128) -> u128 {
        if n == 0 { 0 } else { u128::from(self.next()) % n }
    }
}

/// The lengths the plant and the program take, one per read, scan and write, pushed in step by step.
#[derive(Default)]
struct Costs {
    read: Vec<u128>,
    scan: Vec<u128>,
    write: Vec<u128>,
}

fn ns(v: u128) -> Duration {
    Duration::from_nanos(u64::try_from(v).unwrap())
}

/// Takes the time it is told, computes its output from the outputs it is handed and its input (three times the
/// output it is handed, plus its input, plus one), so the output image the engine hands it shows in every write, and
/// declares safe outputs that depend on its state: 1000 plus the last input it scanned.
struct Program1 {
    clock: VirtualClock,
    costs: Rc<RefCell<Costs>>,
    scans: usize,
    last_input: u64,
    last_scan: Option<Scan>,
}

impl Program for Program1 {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, scan: &Scan, inputs: &u64, outputs: &mut u64) {
        let cost = self.costs.borrow().scan[self.scans];
        self.scans += 1;
        self.last_input = *inputs;
        self.last_scan = Some(*scan);
        self.clock.advance(ns(cost));
        *outputs = outputs.wrapping_mul(3).wrapping_add(inputs + 1);
    }

    fn safe_outputs(&self) -> u64 {
        1000 + self.last_input
    }
}

/// Feeds 100 plus the read's index, logs every read and write by time, and takes the time it is told.
struct Plant {
    clock: VirtualClock,
    costs: Rc<RefCell<Costs>>,
    reads: Vec<Duration>,
    writes: Vec<(Duration, u64)>,
}

impl Io<u64, u64> for Plant {
    fn read_inputs(&mut self, now: Duration) -> u64 {
        let index = self.reads.len();
        self.reads.push(now);
        self.clock.advance(ns(self.costs.borrow().read[index]));
        100 + index as u64
    }

    fn write_outputs(&mut self, now: Duration, outputs: &u64) {
        let index = self.writes.len();
        self.writes.push((now, *outputs));
        self.clock.advance(ns(self.costs.borrow().write[index]));
    }
}

/// The reference model: the clause, in nanoseconds.
#[derive(Default)]
struct Model {
    period: u128,
    now: u128,
    cycle: u128,
    faulted: Option<(u128, u128)>,
    last_input: u64,
    /// The output image as the last scan left it, which the next scan's program is handed.
    image: u64,
    reads: Vec<u128>,
    writes: Vec<(u128, u64)>,
    cycles: u128,
    overruns: u128,
    skipped: u128,
    lateness: (u128, u128),
    exec: (u128, u128),
}

/// What one scan was, as the model sees it.
#[derive(Debug, PartialEq)]
struct Seen {
    cycle: u128,
    start: u128,
    exec: u128,
    time_error: bool,
    program_scanned: bool,
}

impl Model {
    fn step(&mut self, host: u128, read: u128, scan: u128) -> Seen {
        self.now += host;
        let due = self.cycle * self.period;
        let start = self.now.max(due);
        self.now = start;
        let input = 100 + self.reads.len() as u64;
        self.reads.push(start);
        self.now += read;
        let running = self.faulted.is_none();
        if running {
            self.last_input = input;
            self.now += scan;
        }
        let end = self.now;
        let exec = end - start;
        let time_error = running && exec > self.period;
        if time_error {
            self.overruns += 1;
            if exec > 2 * self.period {
                self.faulted = Some((start, exec));
            }
        }
        let outputs = if self.faulted.is_none() {
            self.image.wrapping_mul(3).wrapping_add(input + 1)
        } else {
            1000 + self.last_input
        };
        self.image = outputs;
        self.writes.push((end, outputs));
        self.cycles += 1;
        self.lateness = (self.lateness.0.max(start - due), self.lateness.1 + (start - due));
        self.exec = (self.exec.0.max(exec), self.exec.1 + exec);
        let seen = Seen { cycle: self.cycle, start, exec, time_error, program_scanned: running };
        let next = self.cycle + 1;
        let first = end.div_ceil(self.period);
        if first > next {
            self.skipped += first - next;
            self.cycle = first;
        } else {
            self.cycle = next;
        }
        seen
    }

    fn write(&mut self, write: u128) {
        self.now += write;
    }
}

/// A length drawn so that something lands on a deadline, a nanosecond either side of one, at one or two periods, or
/// anywhere: from `from`, to the `k`-th deadline after it, or of a span of periods.
fn length(draw: &mut Draw, period: u128, from: u128, rarely_long: bool) -> u128 {
    let next_deadline = from.div_ceil(period).max(1) * period;
    let to_deadline = |k: u128| next_deadline + k * period - from;
    match draw.below(12) {
        0..=2 => 0,
        3 => draw.below(period / 3 + 1),
        4 => draw.below(period + 1),
        5 => period,
        6 => period + 1,
        7 => 2 * period,
        8 => 2 * period + 1,
        9 => to_deadline(draw.below(3)),
        10 => to_deadline(draw.below(3)).saturating_sub(1),
        _ if rarely_long => draw.below(3 * period + 1),
        _ => to_deadline(0) + 1,
    }
}

fn run_one(seed: u64, steps: usize) {
    let mut draw = Draw(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let periods: [u128; 7] = [1_000, 333_333, 500_000, 2_500_000, 10_000_000, 1_000_000_000, 1_500_000_000];
    let period = if draw.below(3) == 0 {
        1_000 + draw.below(2_000_000_000)
    } else {
        periods[usize::try_from(draw.below(7)).unwrap()]
    };
    let mut clock = VirtualClock::new();
    let costs = Rc::new(RefCell::new(Costs::default()));
    let program = Program1 { clock: clock.clone(), costs: costs.clone(), scans: 0, last_input: 0, last_scan: None };
    let mut task = Task::new(TaskConfig { name: "model", period: ns(period) }, program).unwrap();
    let mut plant = Plant { clock: clock.clone(), costs: costs.clone(), reads: Vec::new(), writes: Vec::new() };
    let mut model = Model { period, ..Model::default() };

    for step in 0..steps {
        // the host's delay before this step, drawn from the model's own time
        let host = if draw.below(3) == 0 { length(&mut draw, period, model.now, true) } else { 0 };
        let start = (model.now + host).max(model.cycle * period);
        let total = length(&mut draw, period, start, true);
        let read = draw.below(total + 1);
        let scan = total - read;
        let write = if draw.below(4) == 0 { length(&mut draw, period, start + total, true) } else { 0 };
        {
            let mut c = costs.borrow_mut();
            c.read.push(read);
            if model.faulted.is_none() {
                c.scan.push(scan);
            }
            c.write.push(write);
        }
        let want = model.step(host, read, if model.faulted.is_none() { scan } else { 0 });
        clock.advance(ns(host));
        let report = task.step(&mut plant, &mut clock);
        let at = format!("seed {seed}, period {period} ns, step {step}");

        assert_eq!(u128::from(report.scan.cycle), want.cycle, "{at}: the scan's cycle");
        assert_eq!(report.scan.now.as_nanos(), want.start, "{at}: the scan's time");
        assert_eq!(report.scan.period.as_nanos(), period, "{at}: the scan's period");
        assert_eq!(report.exec.as_nanos(), want.exec, "{at}: the scan's length");
        assert_eq!(report.time_error, want.time_error, "{at}: the time error");
        assert_eq!(*report.inputs, 100 + u64::try_from(step).unwrap(), "{at}: the inputs read");
        assert_eq!(*report.outputs, model.writes.last().unwrap().1, "{at}: the outputs written");
        let state = match model.faulted {
            None => RunState::Running,
            Some((at, exec)) => RunState::Faulted { at: ns(at), fault: Fault::Watchdog { exec: ns(exec) } },
        };
        assert_eq!(report.state, state, "{at}: the state");
        if want.program_scanned {
            assert_eq!(report.program.last_scan, Some(report.scan), "{at}: the scan the program is handed");
        }
        let s = *report.stats;
        let mean = |total: u128| total / model.cycles;
        assert_eq!(u128::from(s.cycles), model.cycles, "{at}: cycles");
        assert_eq!(u128::from(s.overruns), model.overruns, "{at}: overruns");
        assert_eq!(u128::from(s.skipped), model.skipped, "{at}: skipped");
        assert_eq!(
            (s.max_lateness.as_nanos(), s.total_lateness.as_nanos(), s.mean_lateness().as_nanos()),
            (model.lateness.0, model.lateness.1, mean(model.lateness.1)),
            "{at}: lateness"
        );
        assert_eq!(
            (s.max_exec.as_nanos(), s.total_exec.as_nanos(), s.mean_exec().as_nanos()),
            (model.exec.0, model.exec.1, mean(model.exec.1)),
            "{at}: length"
        );
        assert_eq!(u128::from(task.next_cycle()), model.cycle, "{at}: the next cycle");
        assert_eq!(task.next_due().as_nanos(), model.cycle * period, "{at}: the next deadline");
        model.write(write);
        assert_eq!(clock.now().as_nanos(), model.now, "{at}: the clock after the write");
    }
    let reads: Vec<u128> = plant.reads.iter().map(Duration::as_nanos).collect();
    let writes: Vec<(u128, u64)> = plant.writes.iter().map(|&(t, v)| (t.as_nanos(), v)).collect();
    assert_eq!(reads, model.reads, "seed {seed}: every read, by its time");
    assert_eq!(writes, model.writes, "seed {seed}: every write, by its time and value");
}

#[test]
fn the_engine_agrees_with_the_clause_s_model_over_four_thousand_random_sequences() {
    for seed in 0..4_000 {
        run_one(seed, 50);
    }
}

#[test]
fn the_engine_agrees_with_the_model_over_forty_runs_of_two_thousand_scans() {
    // long enough that a statistic kept over a window, or reset every so many scans, shows
    for seed in 10_000..10_040 {
        run_one(seed, 2_000);
    }
}

#[test]
fn the_draws_reach_every_case_the_clause_names() {
    // the sequences are only as good as what they reach: count, over the same seeds, the cases that matter
    let (mut late_below_a_ms, mut faulted_late, mut ends_on_a_deadline, mut faulting_ends_on_a_deadline) = (0, 0, 0, 0);
    let (mut skips_after_a_fault, mut long_periods, mut slow_writes_across_a_deadline) = (0, 0, 0);
    for seed in 0..4_000_u64 {
        let mut draw = Draw(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let periods: [u128; 7] = [1_000, 333_333, 500_000, 2_500_000, 10_000_000, 1_000_000_000, 1_500_000_000];
        let period = if draw.below(3) == 0 {
            1_000 + draw.below(2_000_000_000)
        } else {
            periods[usize::try_from(draw.below(7)).unwrap()]
        };
        if period >= 1_000_000_000 {
            long_periods += 1;
        }
        let mut model = Model { period, ..Model::default() };
        for _ in 0..50 {
            let host = if draw.below(3) == 0 { length(&mut draw, period, model.now, true) } else { 0 };
            let start = (model.now + host).max(model.cycle * period);
            let total = length(&mut draw, period, start, true);
            let read = draw.below(total + 1);
            let scan = total - read;
            let write = if draw.below(4) == 0 { length(&mut draw, period, start + total, true) } else { 0 };
            let was_faulted = model.faulted.is_some();
            let skipped = model.skipped;
            let seen = model.step(host, read, if was_faulted { 0 } else { scan });
            let late = seen.start - seen.cycle * period;
            if late > 0 && late < 1_000_000 && !late.is_multiple_of(1_000) {
                late_below_a_ms += 1;
            }
            if was_faulted && late > 0 {
                faulted_late += 1;
            }
            if (seen.start + seen.exec).is_multiple_of(period) && seen.exec > 0 {
                ends_on_a_deadline += 1;
                if !was_faulted && model.faulted.is_some() {
                    faulting_ends_on_a_deadline += 1;
                }
            }
            if was_faulted && model.skipped > skipped {
                skips_after_a_fault += 1;
            }
            let end = seen.start + seen.exec;
            if write > 0 && (end + write) / period > end / period {
                slow_writes_across_a_deadline += 1;
            }
            model.write(write);
        }
    }
    for (case, count) in [
        ("a start late by less than a millisecond, off the microsecond", late_below_a_ms),
        ("a faulted scan started late", faulted_late),
        ("a scan ending exactly on a deadline", ends_on_a_deadline),
        ("a faulting scan ending exactly on a deadline", faulting_ends_on_a_deadline),
        ("a faulted scan skipping deadlines", skips_after_a_fault),
        ("a period of a second or more", long_periods),
        ("a slow write across a deadline", slow_writes_across_a_deadline),
    ] {
        assert!(count >= 20, "{case}: reached {count} times only");
    }
}
