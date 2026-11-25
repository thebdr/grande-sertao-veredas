//! One cyclic task: the scan over a process image, absolute deadlines, the time-error rule, safe outputs and the
//! cycle statistics.
use crate::clock::Clock;
use orcl_std::{Program, Scan};
use core::convert::Infallible;
use core::fmt;
use core::time::Duration;

/// What a task runs: a program whose scan may end in an error of its own, which faults the task. A native
/// [`Program`] is one whose scan never returns an error (a native panic is ORCL's O11 limit, outside the engine);
/// an interpreted one ([`crate::Interpreted`]) fails where Rust would panic (`docs/orcl/interpreter.md`, "Running").
/// No handler of a program error is built yet: an error faults the task (`docs/questions.md` Q223, point 7).
pub trait TaskProgram {
    type Inputs: Copy + Default;
    type Outputs: Copy + Default;
    /// Why a scan of the program failed.
    type Error: Copy + fmt::Debug + PartialEq + Eq;

    /// Scans the inputs into the outputs. An error ends the scan: its outputs, whatever it left in them, are never
    /// written, and the task faults.
    fn scan(&mut self, scan: &Scan, inputs: &Self::Inputs, outputs: &mut Self::Outputs) -> Result<(), Self::Error>;

    /// The outputs the task applies while faulted, asked on the scan that faults and on every faulted scan. It
    /// takes `&mut self` so that a program that computes them may run (`docs/questions.md` Q231).
    fn safe_outputs(&mut self) -> Self::Outputs;
}

impl<P: Program> TaskProgram for P {
    type Inputs = P::Inputs;
    type Outputs = P::Outputs;
    type Error = Infallible;

    fn scan(&mut self, scan: &Scan, inputs: &P::Inputs, outputs: &mut P::Outputs) -> Result<(), Infallible> {
        Program::scan(self, scan, inputs, outputs);
        Ok(())
    }

    fn safe_outputs(&mut self) -> P::Outputs {
        Program::safe_outputs(self)
    }
}

/// The task's connection to the plant: a fieldbus, real I/O, or a simulation. `read_inputs` is called once at
/// the start of every scan, with the scan's time; `write_outputs` once at its end, with the time the scan ended
/// and the program's outputs or, once faulted, its safe outputs.
pub trait Io<I, O> {
    fn read_inputs(&mut self, now: Duration) -> I;
    fn write_outputs(&mut self, now: Duration, outputs: &O);
}

/// A cyclic task's configuration: its name and its period, the time its scans are due apart and the limit the
/// time-error rule measures a scan against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskConfig {
    pub name: &'static str,
    pub period: Duration,
}

/// Why a task cannot be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    /// A period of zero: every scan would be due at once.
    ZeroPeriod { task: &'static str },
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::ZeroPeriod { task } => write!(f, "task `{task}`: a period of zero"),
        }
    }
}

impl core::error::Error for TaskError {}

/// Why a task faulted. `E` is the program's error, [`Infallible`] for a native program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault<E = Infallible> {
    /// One scan ran longer than twice the task's period (the controller's reason 501 `watchdog(task)`).
    Watchdog { exec: Duration },
    /// The program's scan ended in an error, within twice its period. A scan that ran past twice its period is the
    /// watchdog's fault even when it then ended in an error: the host's watchdog faults it at twice the period, while
    /// it still runs (`docs/controlador/execution-model.md`, point 6).
    Program { error: E },
}

/// Whether the task runs its program or, faulted, applies the safe outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState<E = Infallible> {
    Running,
    /// Faulted by the scan that started `at`. From that scan on the program is not scanned and every scan
    /// applies the safe outputs, so the plant's exchange goes on.
    Faulted { at: Duration, fault: Fault<E> },
}

/// The statistics of every scan so far. Lateness is a scan's start against its absolute deadline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CycleStats {
    /// Scans run, faulted ones included.
    pub cycles: u64,
    /// Scans that ran longer than the period.
    pub overruns: u64,
    /// Deadlines that passed while a scan was still running, and were never run.
    pub skipped: u64,
    pub max_lateness: Duration,
    pub total_lateness: Duration,
    pub max_exec: Duration,
    pub total_exec: Duration,
}

impl CycleStats {
    pub fn mean_lateness(&self) -> Duration {
        mean(self.total_lateness, self.cycles)
    }

    pub fn mean_exec(&self) -> Duration {
        mean(self.total_exec, self.cycles)
    }

    fn record(&mut self, lateness: Duration, exec: Duration) {
        self.cycles = self.cycles.saturating_add(1);
        self.max_lateness = self.max_lateness.max(lateness);
        self.total_lateness = self.total_lateness.saturating_add(lateness);
        self.max_exec = self.max_exec.max(exec);
        self.total_exec = self.total_exec.saturating_add(exec);
    }
}

fn mean(total: Duration, count: u64) -> Duration {
    let nanos = total.as_nanos().checked_div(u128::from(count)).unwrap_or(0);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// The deadline of cycle `k`: `k * period` from the start, saturating.
fn deadline(period: Duration, cycle: u64) -> Duration {
    let nanos = period.as_nanos().saturating_mul(u128::from(cycle));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// The first cycle whose deadline is not before `now`: `ceil(now / period)`.
fn first_due_at_or_after(period: Duration, now: Duration) -> u64 {
    let p = period.as_nanos();
    let n = now.as_nanos();
    let k = n.div_ceil(p.max(1));
    u64::try_from(k).unwrap_or(u64::MAX)
}

/// What one scan did, for the host to watch.
pub struct Report<'a, P: TaskProgram> {
    /// The scan the program was handed: its time, the period and its cycle number.
    pub scan: Scan,
    pub program: &'a P,
    pub inputs: &'a P::Inputs,
    /// The outputs written at the end of the scan: the program's, or the safe ones once faulted.
    pub outputs: &'a P::Outputs,
    /// How long the scan ran, from its start to the outputs' write.
    pub exec: Duration,
    /// The scan ran longer than the period: a time error, the one the time-error handler answers.
    pub time_error: bool,
    pub stats: &'a CycleStats,
    pub state: RunState<P::Error>,
}

/// A cyclic task running one program.
pub struct Task<P: TaskProgram> {
    config: TaskConfig,
    program: P,
    inputs: P::Inputs,
    outputs: P::Outputs,
    state: RunState<P::Error>,
    stats: CycleStats,
    /// The cycle that runs next.
    cycle: u64,
}

impl<P: TaskProgram> Task<P> {
    /// A task that has run nothing yet; its first scan is due at zero on the clock it is stepped on.
    pub fn new(config: TaskConfig, program: P) -> Result<Self, TaskError> {
        if config.period.is_zero() {
            return Err(TaskError::ZeroPeriod { task: config.name });
        }
        Ok(Self {
            config,
            program,
            inputs: P::Inputs::default(),
            outputs: P::Outputs::default(),
            state: RunState::Running,
            stats: CycleStats::default(),
            cycle: 0,
        })
    }

    pub fn config(&self) -> &TaskConfig {
        &self.config
    }

    pub fn program(&self) -> &P {
        &self.program
    }

    pub fn state(&self) -> RunState<P::Error> {
        self.state
    }

    pub fn stats(&self) -> &CycleStats {
        &self.stats
    }

    /// The cycle number of the next scan.
    pub fn next_cycle(&self) -> u64 {
        self.cycle
    }

    /// The absolute deadline of the next scan.
    pub fn next_due(&self) -> Duration {
        deadline(self.config.period, self.cycle)
    }

    /// Waits on `clock` for the next scan's deadline, then runs it: the inputs read once, the program scanned on
    /// that snapshot (or, faulted, not scanned), the outputs written once at the end. The scan ends when its
    /// outputs are handed to the plant: its length, the time-error rule and the deadlines it skipped are all judged
    /// at that moment, and the plant's own time to take the outputs is not the scan's. A scan longer than the
    /// period is a time error; a scan longer than twice the period faults the task, and so does a scan whose
    /// program ends in an error, the watchdog's fault when both meet: that scan's own outputs are never written, the
    /// safe outputs are, from that scan on. Deadlines that passed while the scan ran are skipped and counted, never
    /// caught up.
    ///
    /// The rule is measured when the scan ends: a scan that never ends is the host's watchdog to catch, below
    /// this engine (`docs/controlador/execution-model.md`, point 6).
    pub fn step<D, C>(&mut self, io: &mut D, clock: &mut C) -> Report<'_, P>
    where
        D: Io<P::Inputs, P::Outputs>,
        C: Clock,
    {
        let period = self.config.period;
        let due = deadline(period, self.cycle);
        clock.sleep_until(due);
        let start = clock.now();
        let scan = Scan {
            now: start,
            period,
            cycle: self.cycle,
        };
        self.inputs = io.read_inputs(start);
        let running = self.state == RunState::Running;
        let mut fault = None;
        if running {
            if let Err(error) = self.program.scan(&scan, &self.inputs, &mut self.outputs) {
                fault = Some(Fault::Program { error });
            }
        } else {
            self.outputs = self.program.safe_outputs();
        }
        let end = clock.now();
        let exec = end.saturating_sub(start);
        let time_error = running && exec > period;
        if time_error {
            self.stats.overruns = self.stats.overruns.saturating_add(1);
            // past twice its period the watchdog faulted the scan while it still ran, before its end and any error
            if exec > period.saturating_mul(2) {
                fault = Some(Fault::Watchdog { exec });
            }
        }
        if let Some(fault) = fault {
            self.state = RunState::Faulted { at: start, fault };
            self.outputs = self.program.safe_outputs();
        }
        io.write_outputs(end, &self.outputs);
        self.stats.record(start.saturating_sub(due), exec);

        // the scan ended when its outputs were handed over: the plant's own time to take them is not the scan's,
        // and a deadline that passes meanwhile runs late rather than being skipped
        let next = self.cycle.saturating_add(1);
        let first_due = first_due_at_or_after(period, end);
        if first_due > next {
            self.stats.skipped = self.stats.skipped.saturating_add(first_due.saturating_sub(next));
            self.cycle = first_due;
        } else {
            self.cycle = next;
        }

        Report {
            scan,
            program: &self.program,
            inputs: &self.inputs,
            outputs: &self.outputs,
            exec,
            time_error,
            stats: &self.stats,
            state: self.state,
        }
    }

    /// Steps the task for every scan due before `until`, calling `monitor` after each.
    pub fn run_until<D, C, M>(&mut self, io: &mut D, clock: &mut C, until: Duration, mut monitor: M)
    where
        D: Io<P::Inputs, P::Outputs>,
        C: Clock,
        M: FnMut(&Report<'_, P>),
    {
        while self.next_due() < until {
            let report = self.step(io, clock);
            monitor(&report);
        }
    }
}
