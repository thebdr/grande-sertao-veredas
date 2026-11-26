//! The controller: its states and the commands that move them, over one cyclic task.
//!
//! Decided in `docs/controlador-plan.md` and built here: nobody but a person changes the state, the program may
//! only stop itself; every engineering change needs the commissioning switch armed and stopping is free; leaving
//! FAULT takes an acknowledge to STOP, then a start; the whole controller faults and every output takes its safe
//! value while the plant's exchange goes on; after a power cycle RUN returns as RUN and STOP, HOLD and FAULT as STOP
//! with the reason saying what it was; a production target refused real-time scheduling stays in STOP. Where the
//! plan is silent the readings are `docs/questions.md` Q228's, accepted by the architect on 2026-11-25, each named
//! below by its point.
use std::convert::Infallible;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use orcl_interp::{ErrorKind, FunctionId, Scalar};
use orcl_runtime::{
    Clock, Fault, Image, Interpreted, Io, MakeError, Regions, RunState, ScanError, Task, TaskConfig, TaskError,
    TaskProgram,
};
use orcl_std::Program;
use veredas_core::state::{
    CommandSource, ControllerState, FaultReason, StartupPhase, StartupReason, State, StateWord, StopReason,
};

/// The commissioning switch: a file on the controller's own operating system, never set over the network
/// (2026-11-15). The controller only asks whether it is armed; on the target it is a file whose presence arms it,
/// so a lost switch reads as off (`docs/controlador/files.md`, Q224).
pub trait Switch {
    fn armed(&self) -> bool;
}

/// What the platform gave the controller when it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// A production target, which runs only with real-time scheduling; development and simulation run without.
    pub production: bool,
    /// The operating system granted real-time scheduling.
    pub realtime: bool,
    /// The controller runs in the simulator (the condition `sim`).
    pub simulated: bool,
}

/// How the controller came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// Never ran before: no state to come back to.
    First,
    /// The machine itself was powered off and on; `was` is the state it was in.
    PowerCycle { was: StateWord },
    /// The runtime restarted while the machine did not: its own failure.
    Crash,
}

/// A command that may change the controller's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start(StartupReason),
    Stop,
    Acknowledge,
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Command::Start(StartupReason::Warm) => "a warm start",
            Command::Start(StartupReason::Cold) => "a cold start",
            Command::Stop => "a stop",
            Command::Acknowledge => "an acknowledge",
        })
    }
}

/// Why a command changed nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The program may stop itself and nothing else.
    Source { command: Command, source: CommandSource },
    /// The commissioning switch is not armed.
    Switch { command: Command },
    /// The command does nothing in this state.
    State { command: Command, state: StateWord },
    /// A production target without real-time scheduling does not run; it stays in STOP.
    Realtime,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Source { command, source } => {
                write!(f, "{command} from `{}` refused: the program may only stop", source.word())
            }
            Refusal::Switch { command } => write!(f, "{command} refused: the commissioning switch is not armed"),
            Refusal::State { command, state } => write!(f, "{command} refused in {state}"),
            Refusal::Realtime => f.write_str("a start refused: this production target has no real-time scheduling"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Where a controller's program comes from: made at power-on, which checks it, and afresh at every start, so a start
/// runs the program from its initial values; its safe outputs apply while no program runs, made or not.
pub trait Source {
    type Program: TaskProgram;
    /// Why the program cannot be made.
    type Error;

    fn make(&mut self) -> Result<Self::Program, Self::Error>;

    /// The outputs written while no program runs, the program made or not.
    fn safe_outputs(&mut self) -> <Self::Program as TaskProgram>::Outputs;

    /// The fault a scan's error is to the controller: the program's own, with where it stood, or the runtime's.
    fn reason(&self, program: &Self::Program, error: &<Self::Program as TaskProgram>::Error) -> FaultReason;
}

/// A native program, made from its initial values; its scan never fails.
pub struct Native<P>(PhantomData<fn() -> P>);

impl<P> Default for Native<P> {
    fn default() -> Self {
        Native(PhantomData)
    }
}

impl<P: Program + Default> Source for Native<P> {
    type Program = P;
    type Error = Infallible;

    fn make(&mut self) -> Result<P, Infallible> {
        Ok(P::default())
    }

    /// Those of the program as it is before any scan.
    fn safe_outputs(&mut self) -> P::Outputs {
        Program::safe_outputs(&P::default())
    }

    fn reason(&self, _program: &P, error: &Infallible) -> FaultReason {
        match *error {}
    }
}

/// An interpreted program as its host hands it: the program `orcl-interp` loads, its scan entry, its four regions'
/// types, its first state, its safe outputs (`docs/questions.md` Q231, ruled: the front end computes the two and the
/// program file will carry them, once its format holds them; until then the host hands them in) and the stack it
/// runs on, in slots (Q230: the target's settings give it, `veredas-alvo` not built yet). Each make loads it afresh.
pub struct Loaded<I: Image, O: Image> {
    pub program: orcl_interp::Program,
    pub entry: FunctionId,
    pub regions: Regions,
    pub state: Vec<Scalar>,
    pub safe: O,
    pub stack: u32,
    inputs: PhantomData<fn(&I)>,
}

impl<I: Image, O: Image> Loaded<I, O> {
    pub fn new(program: orcl_interp::Program, entry: FunctionId, regions: Regions, state: Vec<Scalar>, safe: O, stack: u32) -> Self {
        Loaded { program, entry, regions, state, safe, stack, inputs: PhantomData }
    }
}

impl<I: Image, O: Image> Source for Loaded<I, O> {
    type Program = Interpreted<I, O>;
    type Error = MakeError;

    fn make(&mut self) -> Result<Interpreted<I, O>, MakeError> {
        Interpreted::new(self.program.clone(), self.entry, self.regions, &self.state, self.safe, self.stack)
    }

    fn safe_outputs(&mut self) -> O {
        self.safe
    }

    /// The program's own error, where Rust would panic (a failed check, an index past the end), and a call past
    /// the stack (Q230), is 502 `program(at)`, `at` its source file, line and column. The runtime's own failure is 507 `internal`: code
    /// marked unreachable reached, a value read before anything wrote it, an inconsistency the load should have
    /// refused, none of which safe Rust can reach, so a fault of the front end or the machine; and an image the
    /// machine refuses or outputs read back as none, the adapter's or the host's.
    fn reason(&self, program: &Interpreted<I, O>, error: &ScanError) -> FaultReason {
        match error {
            ScanError::Program(error) if matches!(error.kind, ErrorKind::Check(_) | ErrorKind::Bounds { .. } | ErrorKind::Stack) => {
                let files = &program.machine().program().files;
                let file = files.get(error.at.file as usize).map_or("?", String::as_str);
                FaultReason::Program { at: format!("{file}:{}:{}", error.at.line, error.at.column) }
            }
            ScanError::Program(_) | ScanError::Host(_) | ScanError::Outputs => FaultReason::Internal,
        }
    }
}

/// The program clock a task runs on: the target's clock from the moment the program started.
struct ProgramClock<'a, C: Clock> {
    target: &'a mut C,
    origin: Duration,
}

impl<C: Clock> Clock for ProgramClock<'_, C> {
    fn now(&self) -> Duration {
        self.target.now().saturating_sub(self.origin)
    }

    fn sleep_until(&mut self, deadline: Duration) {
        self.target.sleep_until(self.origin.saturating_add(deadline));
    }
}

/// The plant as a task sees it: every time it is handed is on the program's clock, and the plant reads it back on
/// the target's.
struct TargetIo<'a, D> {
    plant: &'a mut D,
    origin: Duration,
}

impl<I, O, D: Io<I, O>> Io<I, O> for TargetIo<'_, D> {
    fn read_inputs(&mut self, now: Duration) -> I {
        self.plant.read_inputs(self.origin.saturating_add(now))
    }

    fn write_outputs(&mut self, now: Duration, outputs: &O) {
        self.plant.write_outputs(self.origin.saturating_add(now), outputs);
    }
}

/// A soft PLC running one program `P`, made by its source `S`, as one cyclic task.
pub struct Controller<P: TaskProgram, S: Source<Program = P> = Native<P>> {
    config: TaskConfig,
    source: S,
    /// Why the program could not be made, the last time it was not.
    load_error: Option<S::Error>,
    target: Target,
    state: ControllerState,
    /// The running program's task, from its start until a stop or an acknowledge.
    task: Option<Task<P>>,
    /// Where the running program's clock starts on the target's clock.
    origin: Duration,
    /// The safe outputs that apply while no task runs.
    idle: P::Outputs,
    /// The next exchange while no task runs, counted in periods of the target's clock.
    idle_cycle: u64,
    /// When the last exchange with the plant read its inputs, on the target's clock.
    last_exchange: Option<Duration>,
}

impl<P: Program + Default> Controller<P> {
    /// The controller of a native program as it comes up: from nothing, from a power cycle or from its own failure.
    pub fn power_on(config: TaskConfig, target: Target, restart: Restart) -> Result<Self, TaskError> {
        Self::power_on_from(config, target, restart, Native::default())
    }
}

impl<P: TaskProgram, S: Source<Program = P>> Controller<P, S> {
    /// The controller as it comes up, its program made by `source`: refused when the engine refuses the task's
    /// configuration, checked once, here. The program is made here too, as every load checks it: one that cannot be
    /// made leaves the controller in STOP 104 `no_program`, exchanging the safe outputs, unless it comes up from its
    /// own failure, FAULT 507 `internal` (`docs/questions.md` Q233: until downloads exist; a stored download failing
    /// its check will be the FAULT 506 `load(download)`, Q36).
    pub fn power_on_from(config: TaskConfig, target: Target, restart: Restart, mut source: S) -> Result<Self, TaskError> {
        config.check()?;
        let idle = source.safe_outputs();
        let load_error = source.make().err();
        let mut state = ControllerState::new(State::Stop(None));
        state.conditions.sim = target.simulated;
        state.conditions.no_realtime = !target.realtime && !target.production;
        let loads = load_error.is_none();
        let mut controller = Controller {
            config,
            source,
            load_error,
            target,
            state,
            task: None,
            origin: Duration::ZERO,
            idle,
            idle_cycle: 0,
            last_exchange: None,
        };
        match restart {
            // Q228, point 8 (the briefing's K7 reading): the runtime restarting while the machine did not is its
            // own failure, and it comes up in FAULT, which stops a crash loop.
            Restart::Crash => controller.state.state = State::Fault(Some(FaultReason::Internal)),
            // Q233: a program handed in that cannot be made leaves no program, whatever the state it would come back
            // to, never a substitute.
            _ if !loads => controller.state.state = State::Stop(Some(StopReason::NoProgram)),
            // Q228, point 2: a controller that never ran comes up in STOP with no reason.
            Restart::First => {}
            // RUN returns as RUN through a warm start; so does a start-up a power cut interrupted, its aim having
            // been RUN (Q228, point 3). STOP, HOLD and FAULT return as STOP, the reason naming them.
            Restart::PowerCycle { was: StateWord::Run | StateWord::Startup } => {
                controller.begin(StartupReason::Warm);
            }
            Restart::PowerCycle { was } => {
                controller.state.state = State::Stop(Some(StopReason::PowerOn { was }));
            }
        }
        Ok(controller)
    }

    /// The controller's state as structured data; its text is `state().to_string()`, core's writer.
    pub fn state(&self) -> &ControllerState {
        &self.state
    }

    /// Why the program could not be made, the last time it was not: at power-on or at a start's load.
    pub fn load_error(&self) -> Option<&S::Error> {
        self.load_error.as_ref()
    }

    /// The running program, while a task runs.
    pub fn program(&self) -> Option<&P> {
        self.task.as_ref().map(Task::program)
    }

    /// A start, warm or cold, from STOP: a person's command with the switch armed.
    pub fn start(&mut self, kind: StartupReason, source: CommandSource, switch: &impl Switch) -> Result<(), Refusal> {
        let command = Command::Start(kind);
        by_a_person(command, source)?;
        if !switch.armed() {
            return Err(Refusal::Switch { command });
        }
        let word = self.state.state.word();
        if word != StateWord::Stop {
            return Err(Refusal::State { command, state: word });
        }
        if self.begin(kind) {
            Ok(())
        } else {
            Err(Refusal::Realtime)
        }
    }

    /// A stop, from RUN or STARTUP (Q228, point 9): free, from a person or from the program itself.
    pub fn stop(&mut self, source: CommandSource) -> Result<(), Refusal> {
        let word = self.state.state.word();
        if !matches!(word, StateWord::Run | StateWord::Startup) {
            // Q228, point 7: a command that would change nothing is refused, naming the state.
            return Err(Refusal::State { command: Command::Stop, state: word });
        }
        self.halt(State::Stop(Some(StopReason::Command { source })));
        Ok(())
    }

    /// An acknowledge of a fault, to STOP: a person's command with the switch armed.
    pub fn acknowledge(&mut self, source: CommandSource, switch: &impl Switch) -> Result<(), Refusal> {
        let command = Command::Acknowledge;
        by_a_person(command, source)?;
        if !switch.armed() {
            return Err(Refusal::Switch { command });
        }
        let word = self.state.state.word();
        if word != StateWord::Fault {
            return Err(Refusal::State { command, state: word });
        }
        self.halt(State::Stop(Some(StopReason::Acknowledged)));
        Ok(())
    }

    /// One cycle of the controller on the target's clock. Starting, it finishes its start-up and runs the
    /// program's first scan at once; running, or faulted while its program ran (the watchdog's fault or the
    /// program's own), it runs the task's next scan; stopped,
    /// or faulted before any program ran, it waits for the first period of the target's clock that is not before
    /// this step and comes after the last exchange, reads the inputs and writes the safe outputs, so the plant's
    /// exchange goes on, on those periods alone and at most once in each.
    pub fn step<D, C>(&mut self, io: &mut D, clock: &mut C)
    where
        D: Io<P::Inputs, P::Outputs>,
        C: Clock,
    {
        if let State::Startup { .. } = self.state.state {
            self.finish_startup(clock.now());
        }
        let name = self.config.name;
        match self.task.as_mut() {
            Some(task) => {
                let mut program_clock = ProgramClock { target: &mut *clock, origin: self.origin };
                let mut plant = TargetIo { plant: io, origin: self.origin };
                let report = task.step(&mut plant, &mut program_clock);
                self.last_exchange = Some(self.origin.saturating_add(report.scan.now));
                // Q228, point 6: the condition `overruns` counts the time errors since the last start.
                self.state.conditions.overruns = u32::try_from(task.stats().overruns).unwrap_or(u32::MAX);
                if let (RunState::Faulted { fault, .. }, StateWord::Run) = (task.state(), self.state.state.word()) {
                    let reason = match fault {
                        Fault::Watchdog { .. } => FaultReason::Watchdog { task: name.to_string() },
                        Fault::Program { error } => self.source.reason(task.program(), &error),
                    };
                    self.state.state = State::Fault(Some(reason));
                }
            }
            None => {
                // on the target's periods alone, at most once in each: the first not before this step and after
                // the last exchange, the task's or this one's
                let period = self.config.period;
                let after_last = self.last_exchange.map_or(0, |at| periods_in(period, at).saturating_add(1));
                let not_before_now = first_period_at_or_after(period, clock.now());
                self.idle_cycle = self.idle_cycle.max(after_last).max(not_before_now);
                clock.sleep_until(deadline(period, self.idle_cycle));
                let now = clock.now();
                let _ = io.read_inputs(now);
                self.last_exchange = Some(now);
                // Q228, point 1: in STOP every output takes its safe value, as in FAULT.
                io.write_outputs(clock.now(), &self.idle);
                self.idle_cycle = self.idle_cycle.saturating_add(1);
            }
        }
    }

    /// Enters STARTUP, or STOP 105 on a production target without real-time scheduling. Whether it started.
    fn begin(&mut self, kind: StartupReason) -> bool {
        if self.target.production && !self.target.realtime {
            self.state.state = State::Stop(Some(StopReason::RealtimeRefused));
            return false;
        }
        // a start counts its time errors afresh from the moment it begins, its start-up included
        self.state.conditions.overruns = 0;
        self.state.state = State::Startup { reason: Some(kind), phase: Some(StartupPhase::Load) };
        true
    }

    /// Load, restore, handler, devices, then RUN, the program's clock starting `now`: the load makes the program
    /// afresh from its source. With no retained variable that ORCL can mark yet, a warm start and a cold one both
    /// start the program from its initial values; with no start-up handler and no device, the phases pass at once
    /// (Q228, points 4 and 5). A program that cannot be made fails the load, leaving no program: STOP 104
    /// `no_program` (Q233), from where a person's start tries again.
    fn finish_startup(&mut self, now: Duration) {
        self.origin = now;
        self.task = None;
        self.state.state = match self.source.make() {
            Ok(program) => match Task::new(self.config.clone(), program) {
                Ok(task) => {
                    self.task = Some(task);
                    State::Run
                }
                // the configuration was checked at power-on, so the engine refusing it is the runtime's failure
                Err(_) => State::Fault(Some(FaultReason::Internal)),
            },
            Err(error) => {
                self.load_error = Some(error);
                State::Stop(Some(StopReason::NoProgram))
            }
        };
    }

    /// Leaves the program: the task ends and the idle exchange takes over at the next step.
    fn halt(&mut self, to: State) {
        self.task = None;
        self.state.state = to;
    }
}

/// The program may stop itself and nothing else (2026-11-15).
fn by_a_person(command: Command, source: CommandSource) -> Result<(), Refusal> {
    if source == CommandSource::Program {
        return Err(Refusal::Source { command, source });
    }
    Ok(())
}

/// The deadline of cycle `k`: `k * period` from the start, saturating.
fn deadline(period: Duration, cycle: u64) -> Duration {
    let nanos = period.as_nanos().saturating_mul(u128::from(cycle));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// The first period whose start is not before `time`: `ceil(time / period)`.
fn first_period_at_or_after(period: Duration, time: Duration) -> u64 {
    let k = time.as_nanos().div_ceil(period.as_nanos().max(1));
    u64::try_from(k).unwrap_or(u64::MAX)
}

/// How many whole periods lie in `time`: the period `time` falls in.
fn periods_in(period: Duration, time: Duration) -> u64 {
    let k = time.as_nanos().checked_div(period.as_nanos()).unwrap_or(0);
    u64::try_from(k).unwrap_or(u64::MAX)
}
