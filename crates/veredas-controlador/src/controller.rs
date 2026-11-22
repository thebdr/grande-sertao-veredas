//! The controller: its states and the commands that move them, over one cyclic task.
//!
//! Decided in `docs/controlador-plan.md` and built here: nobody but a person changes the state, the program may
//! only stop itself; every engineering change needs the commissioning switch armed and stopping is free; leaving
//! FAULT takes an acknowledge to STOP, then a start; the whole controller faults and every output takes its safe
//! value while the plant's exchange goes on; after a power cycle RUN returns as RUN and STOP, HOLD and FAULT as STOP
//! with the reason saying what it was; a production target refused real-time scheduling stays in STOP. Where the
//! plan is silent the readings are `docs/questions.md` Q228's, each marked DECISION PENDING below.
use std::fmt;
use std::time::Duration;

use orcl_runtime::{Clock, Fault, Io, RunState, Task, TaskConfig, TaskError};
use orcl_std::Program;
use veredas_core::state::{
    CommandSource, ControllerState, FaultReason, StartupPhase, StartupReason, State, StateWord, StopReason,
};

/// The commissioning switch: a file on the controller's own operating system, never set over the network
/// (2026-11-15). The controller only asks whether it is armed; its form on the target is Q224's.
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

/// A soft PLC running one program `P` as one cyclic task.
pub struct Controller<P: Program + Default> {
    config: TaskConfig,
    target: Target,
    state: ControllerState,
    /// The running program's task, from its start until a stop or an acknowledge.
    task: Option<Task<P>>,
    /// Where the running program's clock starts on the target's clock.
    origin: Duration,
    /// A program as it is before any scan, whose safe outputs apply while no task runs.
    idle: P,
    /// The next exchange while no task runs, counted in periods of the target's clock.
    idle_cycle: u64,
    /// When the last exchange with the plant read its inputs, on the target's clock.
    last_exchange: Option<Duration>,
}

impl<P: Program + Default> Controller<P> {
    /// The controller as it comes up: from nothing, from a power cycle or from its own failure.
    pub fn power_on(config: TaskConfig, target: Target, restart: Restart) -> Result<Self, TaskError> {
        // the task's configuration is checked once, here, by the engine that will run it
        let _checked = Task::new(config.clone(), P::default())?;
        let mut state = ControllerState::new(State::Stop(None));
        state.conditions.sim = target.simulated;
        state.conditions.no_realtime = !target.realtime && !target.production;
        let mut controller = Controller {
            config,
            target,
            state,
            task: None,
            origin: Duration::ZERO,
            idle: P::default(),
            idle_cycle: 0,
            last_exchange: None,
        };
        match restart {
            // DECISION PENDING (Q228): a controller that never ran comes up in STOP with no reason.
            Restart::First => {}
            // RUN returns as RUN through a warm start; DECISION PENDING (Q228): so does a start-up a power cut
            // interrupted, its aim having been RUN. STOP, HOLD and FAULT return as STOP, the reason naming them.
            Restart::PowerCycle { was: StateWord::Run | StateWord::Startup } => {
                controller.begin(StartupReason::Warm);
            }
            Restart::PowerCycle { was } => {
                controller.state.state = State::Stop(Some(StopReason::PowerOn { was }));
            }
            // DECISION PENDING (Q228, the briefing's K7 reading): the runtime restarting while the machine did not
            // is its own failure, and it comes up in FAULT, which stops a crash loop.
            Restart::Crash => controller.state.state = State::Fault(Some(FaultReason::Internal)),
        }
        Ok(controller)
    }

    /// The controller's state as structured data; its text is `state().to_string()`, core's writer.
    pub fn state(&self) -> &ControllerState {
        &self.state
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

    /// A stop, from RUN or STARTUP: free, from a person or from the program itself.
    pub fn stop(&mut self, source: CommandSource) -> Result<(), Refusal> {
        let word = self.state.state.word();
        if !matches!(word, StateWord::Run | StateWord::Startup) {
            // DECISION PENDING (Q228): a command that would change nothing is refused, naming the state.
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
    /// program's first scan at once; running, or faulted by its program, it runs the task's next scan; stopped,
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
                // DECISION PENDING (Q228): the condition `overruns` counts the time errors since the last start.
                self.state.conditions.overruns = u32::try_from(task.stats().overruns).unwrap_or(u32::MAX);
                if let (RunState::Faulted { fault: Fault::Watchdog { .. }, .. }, StateWord::Run) =
                    (task.state(), self.state.state.word())
                {
                    self.state.state = State::Fault(Some(FaultReason::Watchdog { task: name.to_string() }));
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
                // DECISION PENDING (Q228): in STOP every output takes its safe value, as in FAULT.
                io.write_outputs(clock.now(), &self.idle.safe_outputs());
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

    /// Load, restore, handler, devices, then RUN, the program's clock starting `now`. DECISION PENDING (Q228):
    /// with no retained variable that ORCL can mark yet, a warm start and a cold one both start the program from
    /// its initial values; with no start-up handler and no device, the phases pass at once.
    fn finish_startup(&mut self, now: Duration) {
        self.task = Task::new(self.config.clone(), P::default()).ok();
        self.origin = now;
        self.state.state = if self.task.is_some() { State::Run } else { State::Fault(Some(FaultReason::Internal)) };
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
