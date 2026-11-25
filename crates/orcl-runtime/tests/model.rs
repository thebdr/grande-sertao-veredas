//! The engine of a scan against a reference model of C-151, over thousands of random sequences on virtual time.
//!
//! The model is written from the clause, apart from the engine, in whole nanoseconds: a scan is due at `k` periods
//! from the clock's zero and starts there, or when the host steps it if later; it reads its inputs at its start, the
//! program scans while the task runs, and the scan ends when its outputs are handed to the plant; a scan longer than
//! its period is a time error, one longer than twice its period faults the task, dated at its start, its own outputs
//! never written but the program's safe outputs, which every later scan writes without scanning the program; the
//! next scan is the first whose deadline is not before the end, the others skipped and counted. The sequences are
//! drawn so that starts, ends and writes land on deadlines, a nanosecond either side of them, and at one and two
//! periods, with periods from a microsecond to two seconds; the program's safe outputs depend on its own state. One
//! step in twenty the host is away for up to a million periods, and a wait that has to wait wakes past its deadline,
//! by a nanosecond, part of a period or more, as a target's does under load. The program's scan may end in an error
//! of its own, after writing junk into its outputs: the task faults so too, dated at the scan's start, its outputs
//! never written, and a scan past twice the period is the watchdog's fault even when it fails. Three mixes: anything (the
//! watchdog's fault early in most sequences, one scan in forty failing), every scan held to twice the period and none
//! failing (the task runs the whole way), and held so until a step drawn late in the run, after a drawn streak of time
//! errors, where the task faults by the watchdog, by the program's error, or by both on one scan. After every step the
//! model and the engine must agree on everything the clause states. Every sequence runs twice: the program as a
//! `TaskProgram` of its own, and as a native `orcl_std::Program`, which the engine runs through its blanket
//! implementation, the path of every program controlador runs today; a native program has no error, so its runs draw
//! the same timings with no scan failing.
use std::cell::RefCell;
use std::convert::Infallible;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use orcl_runtime::{Clock, Fault, Io, RunState, Task, TaskConfig, TaskProgram, VirtualClock};
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

/// The lengths the plant and the program take, one per read, scan and write, and whether each scan fails, pushed in
/// step by step.
#[derive(Default)]
struct Costs {
    read: Vec<u128>,
    scan: Vec<u128>,
    fail: Vec<bool>,
    write: Vec<u128>,
}

fn ns(v: u128) -> Duration {
    Duration::from_nanos(u64::try_from(v).unwrap())
}

/// Takes the time it is told, computes its output from the outputs it is handed and its input (three times the
/// output it is handed, plus its input, plus one), so the output image the engine hands it shows in every write, and
/// declares safe outputs that depend on its state: 1000 plus the last input it scanned. A scan it is told to fail
/// takes its time, writes junk into its outputs and fails with its own index.
struct Program1 {
    clock: VirtualClock,
    costs: Rc<RefCell<Costs>>,
    scans: usize,
    last_input: u64,
    last_scan: Option<Scan>,
}

/// The junk a failing scan leaves in its outputs, which no write may show.
const JUNK: u64 = 0xdead_beef_dead_beef;

impl TaskProgram for Program1 {
    type Inputs = u64;
    type Outputs = u64;
    type Error = usize;

    fn scan(&mut self, scan: &Scan, inputs: &u64, outputs: &mut u64) -> Result<(), usize> {
        let (cost, fail) = (self.costs.borrow().scan[self.scans], self.costs.borrow().fail[self.scans]);
        let index = self.scans;
        self.scans += 1;
        self.last_input = *inputs;
        self.last_scan = Some(*scan);
        self.clock.advance(ns(cost));
        if fail {
            *outputs = JUNK;
            return Err(index);
        }
        *outputs = outputs.wrapping_mul(3).wrapping_add(inputs + 1);
        Ok(())
    }

    fn safe_outputs(&mut self) -> u64 {
        self.safe()
    }
}

impl Program1 {
    fn safe(&self) -> u64 {
        1000 + self.last_input
    }
}

/// `Program1` as a native program, which the engine runs through `impl<P: Program> TaskProgram for P`: it scans as
/// `Program1` does, never told to fail, and declares the same safe outputs.
struct Native1(Program1);

impl Program for Native1 {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, scan: &Scan, inputs: &u64, outputs: &mut u64) {
        let scanned = TaskProgram::scan(&mut self.0, scan, inputs, outputs);
        assert_eq!(scanned, Ok(()), "a native program's scan is never told to fail");
    }

    fn safe_outputs(&self) -> u64 {
        self.0.safe()
    }
}

/// The two ways a sequence runs `Program1`: as it is, its scans failing as drawn, or as a native program, none failing.
trait Twin: TaskProgram<Inputs = u64, Outputs = u64> {
    /// Whether its scans fail as drawn.
    const FAILS: bool;

    fn wrap(program: Program1) -> Self;

    fn inner(&self) -> &Program1;

    /// The error of the scan of that index.
    fn error(index: usize) -> Self::Error;
}

impl Twin for Program1 {
    const FAILS: bool = true;

    fn wrap(program: Program1) -> Self {
        program
    }

    fn inner(&self) -> &Program1 {
        self
    }

    fn error(index: usize) -> usize {
        index
    }
}

impl Twin for Native1 {
    const FAILS: bool = false;

    fn wrap(program: Program1) -> Self {
        Native1(program)
    }

    fn inner(&self) -> &Program1 {
        &self.0
    }

    fn error(_index: usize) -> Infallible {
        unreachable!("a native program's scan never fails")
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

/// Why the model's task faulted: the watchdog, with the scan's length, or the program, with its scan's index.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Why {
    Watchdog(u128),
    Program(usize),
}

/// The reference model: the clause, in nanoseconds.
#[derive(Default)]
struct Model {
    period: u128,
    now: u128,
    cycle: u128,
    faulted: Option<(u128, Why)>,
    /// The program's scans so far.
    scans: usize,
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
    fn step(&mut self, host: u128, wake: u128, read: u128, scan: u128, fail: bool) -> Seen {
        self.now += host;
        let due = self.cycle * self.period;
        // a wait that has to wait wakes `wake` past the deadline; a deadline passed already runs at once
        let start = if self.now < due { due + wake } else { self.now };
        self.now = start;
        let input = 100 + self.reads.len() as u64;
        self.reads.push(start);
        self.now += read;
        let running = self.faulted.is_none();
        let failed = running && fail;
        if running {
            self.last_input = input;
            self.now += scan;
            self.scans += 1;
        }
        let end = self.now;
        let exec = end - start;
        let time_error = running && exec > self.period;
        if time_error {
            self.overruns += 1;
        }
        if time_error && exec > 2 * self.period {
            self.faulted = Some((start, Why::Watchdog(exec)));
        } else if failed {
            self.faulted = Some((start, Why::Program(self.scans - 1)));
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

/// How a sequence's scans are drawn.
#[derive(Clone, Copy)]
enum Mix {
    /// Anything: most sequences fault early, one scan in forty failing.
    Free,
    /// Every scan held to twice the period, none failing, so the task runs the whole sequence with its time errors,
    /// skips, late starts and output image.
    NeverFaults,
    /// Held to twice the period up to a step drawn between the 60th and the 1960th, the scans before it a drawn streak
    /// of up to 40 time errors, then a scan that faults the task far into its run: past twice the period, failing
    /// within it, or both.
    FaultsLate,
}

/// How a `FaultsLate` sequence's task faults.
#[derive(Clone, Copy, PartialEq)]
enum Late {
    Watchdog,
    Program,
    Both,
}

/// What a sequence draws once: its period, and for `FaultsLate` the step of its fault, the streak before it and how
/// it faults. Whether a scan fails is drawn from a stream of its own, returned with the sequence's.
fn draw_sequence(seed: u64, mix: Mix) -> (Draw, Draw, u128, Option<(usize, usize, Late)>) {
    let mut draw = Draw(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut fails = Draw(seed.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ 0x94d0_49bb_1331_11eb | 1);
    let periods: [u128; 7] = [1_000, 333_333, 500_000, 2_500_000, 10_000_000, 1_000_000_000, 1_500_000_000];
    let period = if draw.below(3) == 0 {
        1_000 + draw.below(2_000_000_000)
    } else {
        periods[usize::try_from(draw.below(7)).unwrap()]
    };
    let late_fault = match mix {
        Mix::FaultsLate => {
            let how = [Late::Watchdog, Late::Program, Late::Both][usize::try_from(fails.below(3)).unwrap()];
            Some((60 + usize::try_from(draw.below(1_900)).unwrap(), usize::try_from(draw.below(41)).unwrap(), how))
        }
        Mix::Free | Mix::NeverFaults => None,
    };
    (draw, fails, period, late_fault)
}

/// One step's draws: the host's delay before it, how late a wait wakes, the read's and the program's lengths, the
/// write's, and whether the scan fails (1) or not (0).
fn draw_step(draw: &mut Draw, fails: &mut Draw, model: &Model, mix: Mix, late_fault: Option<(usize, usize, Late)>, step: usize) -> [u128; 6] {
    let period = model.period;
    // one step in twenty the host is away for up to a million periods, coming back on a deadline, a nanosecond past
    // one, or anywhere in a period
    let host = if draw.below(20) == 0 {
        let away = model.now.div_ceil(period).max(1) * period + draw.below(1_000_000) * period - model.now;
        match draw.below(3) {
            0 => away,
            1 => away + 1,
            _ => away + draw.below(period),
        }
    } else if draw.below(3) == 0 {
        length(draw, period, model.now, true)
    } else {
        0
    };
    let wake = match draw.below(6) {
        0 => 1,
        1 => draw.below(period / 3 + 1),
        2 => draw.below(3 * period + 1),
        _ => 0,
    };
    let (arrival, due) = (model.now + host, model.cycle * period);
    let start = if arrival < due { due + wake } else { arrival };
    let total = length(draw, period, start, true);
    let total = match (mix, late_fault) {
        (Mix::Free, _) => total,
        (Mix::NeverFaults, _) | (Mix::FaultsLate, None) => total.min(2 * period),
        (Mix::FaultsLate, Some((at, _, Late::Program))) if step == at => total.min(2 * period),
        (Mix::FaultsLate, Some((at, _, _))) if step == at => 2 * period + 1 + draw.below(period),
        (Mix::FaultsLate, Some((at, streak, _))) if step < at && step + streak >= at => period + 1 + draw.below(period),
        (Mix::FaultsLate, Some((at, _, _))) if step < at => total.min(2 * period),
        (Mix::FaultsLate, Some(_)) => total,
    };
    let read = draw.below(total + 1);
    let write = if draw.below(4) == 0 { length(draw, period, start + total, true) } else { 0 };
    let fail = match (mix, late_fault) {
        (Mix::Free, _) => fails.below(40) == 0,
        (Mix::NeverFaults, _) | (Mix::FaultsLate, None) => false,
        (Mix::FaultsLate, Some((at, _, how))) => step == at && how != Late::Watchdog,
    };
    [host, wake, read, total - read, write, u128::from(fail)]
}

/// Virtual time whose wait, when it has to wait, wakes `wake` past its deadline, as a target's does under load.
struct WakesLate {
    clock: VirtualClock,
    wake: Duration,
}

impl Clock for WakesLate {
    fn now(&self) -> Duration {
        self.clock.now()
    }

    fn sleep_until(&mut self, deadline: Duration) {
        let now = self.clock.now();
        if deadline > now {
            self.clock.advance(deadline + self.wake - now);
        }
    }
}

/// One sequence of `steps` scans, drawn as `mix` says, the program run as `P`: a native program's scans drawn failing
/// do not fail, the other draws, kept apart from those, the same.
fn run_one<P: Twin>(seed: u64, steps: usize, mix: Mix) {
    let (mut draw, mut fails, period, late_fault) = draw_sequence(seed, mix);
    let clock = VirtualClock::new();
    let costs = Rc::new(RefCell::new(Costs::default()));
    let program = P::wrap(Program1 { clock: clock.clone(), costs: costs.clone(), scans: 0, last_input: 0, last_scan: None });
    let mut task = Task::new(TaskConfig { name: "model", period: ns(period) }, program).unwrap();
    let mut plant = Plant { clock: clock.clone(), costs: costs.clone(), reads: Vec::new(), writes: Vec::new() };
    let mut model = Model { period, ..Model::default() };
    let mut target = WakesLate { clock: clock.clone(), wake: Duration::ZERO };

    for step in 0..steps {
        let [host, wake, read, scan, write, fail] = draw_step(&mut draw, &mut fails, &model, mix, late_fault, step);
        let fail = fail == 1 && P::FAILS;
        {
            let mut c = costs.borrow_mut();
            c.read.push(read);
            if model.faulted.is_none() {
                c.scan.push(scan);
                c.fail.push(fail);
            }
            c.write.push(write);
        }
        let want = model.step(host, wake, read, if model.faulted.is_none() { scan } else { 0 }, fail);
        clock.advance(ns(host));
        target.wake = ns(wake);
        let report = task.step(&mut plant, &mut target);
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
            Some((at, Why::Watchdog(exec))) => RunState::Faulted { at: ns(at), fault: Fault::Watchdog { exec: ns(exec) } },
            Some((at, Why::Program(index))) => RunState::Faulted { at: ns(at), fault: Fault::Program { error: P::error(index) } },
        };
        assert_eq!(report.state, state, "{at}: the state");
        if want.program_scanned {
            assert_eq!(report.program.inner().last_scan, Some(report.scan), "{at}: the scan the program is handed");
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
    assert!(writes.iter().all(|&(_, v)| v != JUNK), "seed {seed}: no failed scan's outputs written");
}

/// Every run of the model: its mix, its seeds and its steps.
const RUNS: [(Mix, Range<u64>, usize); 4] = [
    (Mix::Free, 0..4_000, 50),
    (Mix::Free, 10_000..10_040, 2_000),
    (Mix::NeverFaults, 20_000..20_040, 2_000),
    (Mix::FaultsLate, 30_000..30_100, 2_000),
];

#[test]
fn the_engine_agrees_with_the_clause_s_model_over_four_thousand_random_sequences() {
    for seed in 0..4_000 {
        run_one::<Program1>(seed, 50, Mix::Free);
    }
}

#[test]
fn the_engine_agrees_with_the_model_over_forty_runs_of_two_thousand_scans() {
    // long enough that a statistic kept over a window, or reset every so many scans, shows
    for seed in 10_000..10_040 {
        run_one::<Program1>(seed, 2_000, Mix::Free);
    }
}

#[test]
fn a_task_that_never_faults_agrees_with_the_model_over_forty_runs_of_two_thousand_scans() {
    // the faulting runs fault early; these run the whole way, with hundreds of time errors, late starts and skips each
    for seed in 20_000..20_040 {
        run_one::<Program1>(seed, 2_000, Mix::NeverFaults);
    }
}

#[test]
fn a_task_that_faults_late_in_its_run_agrees_with_the_model_over_a_hundred_runs_of_two_thousand_scans() {
    // the watchdog met far into a run: after hundreds of scans, time errors, skips and long absences, and after a
    // streak of time errors in a row
    for seed in 30_000..30_100 {
        run_one::<Program1>(seed, 2_000, Mix::FaultsLate);
    }
}

#[test]
fn a_native_program_agrees_with_the_model_over_every_run_none_failing() {
    // the path of every program controlador runs today: what it is handed, its time to the nanosecond, the output
    // image as the last scan left it, on every step of every run
    for (mix, seeds, steps) in RUNS {
        for seed in seeds {
            run_one::<Native1>(seed, steps, mix);
        }
    }
}

/// Takes no time and outputs its input.
struct Idle;

impl Program for Idle {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, _scan: &Scan, inputs: &u64, outputs: &mut u64) {
        *outputs = *inputs;
    }
}

struct Quiet;

impl Io<u64, u64> for Quiet {
    fn read_inputs(&mut self, _now: Duration) -> u64 {
        1
    }

    fn write_outputs(&mut self, _now: Duration, _outputs: &u64) {}
}

#[test]
fn a_long_absence_counts_every_deadline_it_skipped() {
    // a host away for 300 periods, then for 2^33: every deadline passed is counted, past 8 and 32 bits
    let period = Duration::from_millis(1);
    let mut clock = VirtualClock::new();
    let mut task = Task::new(TaskConfig { name: "absent", period }, Idle).unwrap();
    task.step(&mut Quiet, &mut clock);
    clock.advance(period * 300 + Duration::from_nanos(1));
    // the deadline found passed runs late, as it is, its lateness counted; its length alone judged
    let report = task.step(&mut Quiet, &mut clock);
    let late = (report.scan.cycle, report.scan.now, report.time_error, report.state);
    assert_eq!(late, (1, period * 300 + Duration::from_nanos(1), false, RunState::Running), "the late scan");
    let s = *task.stats();
    assert_eq!(s.max_lateness, period * 299 + Duration::from_nanos(1), "its lateness");
    assert_eq!((s.skipped, s.cycles + s.skipped, task.next_cycle()), (299, 301, 301));
    let k: u64 = 1 << 33;
    clock.advance(Duration::from_nanos(1_000_000 * k));
    let report = task.step(&mut Quiet, &mut clock);
    let late = (report.scan.cycle, report.scan.now, report.time_error, report.state);
    let back = period * 300 + Duration::from_nanos(1 + 1_000_000 * k);
    assert_eq!(late, (301, back, false, RunState::Running), "the scan after 2^33 periods away");
    let s = *task.stats();
    assert_eq!(s.max_lateness, back - period * 301, "its lateness");
    assert_eq!(s.cycles + s.skipped, task.next_cycle(), "every cycle run or counted");
    assert_eq!(s.skipped, 299 + k - 1);
}

/// Runs 8 ms every scan, and keeps the time it was handed.
struct Eight {
    clock: VirtualClock,
    handed: Vec<Duration>,
}

impl Program for Eight {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, scan: &Scan, _inputs: &u64, outputs: &mut u64) {
        self.handed.push(scan.now);
        self.clock.advance(Duration::from_millis(8));
        *outputs += 1;
    }
}

/// The second scan of an 8 ms program on a 10 ms period, on a clock whose wait wakes `wake` past its deadline.
fn second_scan_waking(wake: Duration) -> (Duration, Duration, bool, RunState, Duration, Vec<Duration>) {
    let clock = VirtualClock::new();
    let mut target = WakesLate { clock: clock.clone(), wake };
    let program = Eight { clock: clock.clone(), handed: Vec::new() };
    let period = Duration::from_millis(10);
    let mut task = Task::new(TaskConfig { name: "late", period }, program).unwrap();
    task.step(&mut Quiet, &mut target);
    let r = task.step(&mut Quiet, &mut target);
    let seen = (r.scan.now, r.exec, r.time_error, r.state, r.stats.max_lateness);
    (seen.0, seen.1, seen.2, seen.3, seen.4, task.program().handed.clone())
}

#[test]
fn a_wait_that_wakes_late_starts_the_scan_then_its_length_alone_judged() {
    // the scan starts when the wait wakes, its lateness counted against its deadline, its length from its start, and
    // the rule measures the length alone: 3 ms late is no time error, and 13 ms late no fault
    let ms = Duration::from_millis;
    let (now, exec, time_error, state, lateness, handed) = second_scan_waking(ms(3));
    assert_eq!((now, exec, time_error, state, lateness), (ms(13), ms(8), false, RunState::Running, ms(3)));
    assert_eq!(handed, [ms(0), ms(13)], "the program is handed the scan's start");
    let (now, exec, time_error, state, lateness, handed) = second_scan_waking(ms(13));
    assert_eq!((now, exec, time_error, state, lateness), (ms(23), ms(8), false, RunState::Running, ms(13)));
    assert_eq!(handed, [ms(0), ms(23)], "the program is handed the scan's start");
}

/// Runs one and a half periods every scan: a time error each time, never a fault.
struct Slow {
    clock: VirtualClock,
}

impl Program for Slow {
    type Inputs = u64;
    type Outputs = u64;

    fn scan(&mut self, scan: &Scan, inputs: &u64, outputs: &mut u64) {
        self.clock.advance(scan.period * 3 / 2);
        *outputs = *inputs;
    }
}

#[test]
fn seventy_thousand_late_scans_are_each_counted_past_sixteen_bits() {
    // scans, time errors and skips counted past 65 535, the mean length exact, the task still running
    let period = Duration::from_millis(1);
    let mut clock = VirtualClock::new();
    let mut task = Task::new(TaskConfig { name: "long", period }, Slow { clock: clock.clone() }).unwrap();
    let n: u64 = 70_000;
    for _ in 0..n {
        task.step(&mut Quiet, &mut clock);
    }
    let s = *task.stats();
    assert_eq!((s.cycles, s.overruns, s.skipped), (n, n, n));
    assert_eq!((s.mean_exec(), s.max_exec, task.state()), (period * 3 / 2, period * 3 / 2, RunState::Running));
}

#[test]
fn the_draws_reach_every_case_the_clause_names() {
    // the sequences are only as good as what they reach: count, over the same seeds and draws, the cases that matter
    let (mut late_below_a_ms, mut faulted_late, mut ends_on_a_deadline, mut faulting_ends_on_a_deadline) = (0, 0, 0, 0);
    let (mut skips_after_a_fault, mut long_periods, mut slow_writes_across_a_deadline) = (0, 0, 0);
    let (mut faults_past_scan_1000, mut faults_after_20_time_errors, mut faults_after_10_in_a_row) = (0, 0, 0);
    let (mut faults_after_100_skips, mut late_by_1000_periods_running, mut late_by_1000_periods_faulted) = (0, 0, 0);
    let mut woke_late = 0;
    let (mut program_faults, mut program_faults_on_a_time_error, mut failing_past_twice) = (0, 0, 0);
    let (mut program_faults_past_scan_1000, mut program_faults_in_free_sequences) = (0, 0);
    for (mix, seeds, steps) in RUNS {
        for seed in seeds {
            let (mut draw, mut fails, period, late_fault) = draw_sequence(seed, mix);
            if period >= 1_000_000_000 && steps == 50 {
                long_periods += 1;
            }
            let mut model = Model { period, ..Model::default() };
            let mut in_a_row = 0;
            for step in 0..steps {
                let [host, wake, read, scan, write, fail] = draw_step(&mut draw, &mut fails, &model, mix, late_fault, step);
                let was_faulted = model.faulted.is_some();
                let skipped = model.skipped;
                if wake > 0 && model.now + host < model.cycle * period {
                    woke_late += 1;
                }
                let seen = model.step(host, wake, read, if was_faulted { 0 } else { scan }, fail == 1);
                in_a_row = if seen.time_error { in_a_row + 1 } else { 0 };
                let late = seen.start - seen.cycle * period;
                if late > 0 && late < 1_000_000 && !late.is_multiple_of(1_000) {
                    late_below_a_ms += 1;
                }
                if was_faulted && late > 0 {
                    faulted_late += 1;
                }
                if late >= 1_000 * period {
                    if was_faulted {
                        late_by_1000_periods_faulted += 1;
                    } else {
                        late_by_1000_periods_running += 1;
                    }
                }
                if (seen.start + seen.exec).is_multiple_of(period) && seen.exec > 0 {
                    ends_on_a_deadline += 1;
                    if !was_faulted && model.faulted.is_some() {
                        faulting_ends_on_a_deadline += 1;
                    }
                }
                if !was_faulted && fail == 1 && seen.exec > 2 * period {
                    failing_past_twice += 1;
                }
                if let (false, Some((_, Why::Program(_)))) = (was_faulted, model.faulted) {
                    program_faults += 1;
                    program_faults_on_a_time_error += usize::from(seen.time_error);
                    program_faults_past_scan_1000 += usize::from(step >= 1_000);
                    program_faults_in_free_sequences += usize::from(matches!(mix, Mix::Free));
                }
                if !was_faulted && model.faulted.is_some() {
                    faults_past_scan_1000 += usize::from(step >= 1_000);
                    faults_after_20_time_errors += usize::from(model.overruns > 20);
                    faults_after_10_in_a_row += usize::from(in_a_row > 10);
                    faults_after_100_skips += usize::from(model.skipped >= 100);
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
    }
    for (case, count) in [
        ("a start late by less than a millisecond, off the microsecond", late_below_a_ms),
        ("a faulted scan started late", faulted_late),
        ("a scan ending exactly on a deadline", ends_on_a_deadline),
        ("a faulting scan ending exactly on a deadline", faulting_ends_on_a_deadline),
        ("a faulted scan skipping deadlines", skips_after_a_fault),
        ("a period of a second or more", long_periods),
        ("a slow write across a deadline", slow_writes_across_a_deadline),
        ("a fault past the 1000th scan", faults_past_scan_1000),
        ("a fault after more than 20 time errors", faults_after_20_time_errors),
        ("a fault after more than 10 time errors in a row", faults_after_10_in_a_row),
        ("a fault after 100 skipped deadlines", faults_after_100_skips),
        ("a running scan late by 1000 periods or more", late_by_1000_periods_running),
        ("a faulted scan late by 1000 periods or more", late_by_1000_periods_faulted),
        ("a scan started by a wait that woke past its deadline", woke_late),
        ("a program fault", program_faults),
        ("a program fault on a scan that is a time error", program_faults_on_a_time_error),
        ("a failing scan past twice the period, the watchdog's fault", failing_past_twice),
        ("a program fault past the 1000th scan", program_faults_past_scan_1000),
        ("a program fault in a free sequence", program_faults_in_free_sequences),
    ] {
        assert!(count >= 20, "{case}: reached {count} times only");
    }
}
