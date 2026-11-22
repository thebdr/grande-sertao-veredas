//! The controller's states and the commands that move them, over one cyclic task on virtual time: every state
//! read as core's structured state and as its text, every refusal by its text, every exchange with the plant by
//! its time and value.
use std::time::Duration;

use orcl_runtime::{Io, TaskConfig, TaskError, VirtualClock};
use orcl_std::{Program, Scan};
use veredas_controlador::{Command, Controller, Refusal, Restart, Switch, Target};
use veredas_core::state::{
    CommandSource, FaultReason, Health, StartupPhase, StartupReason, State, StateWord, StopReason,
};

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// Counts its scans into its output, plus its input; its safe outputs are 7, which no default gives.
#[derive(Default)]
struct Count {
    scans: u32,
    last_now: Duration,
}

impl Program for Count {
    type Inputs = u32;
    type Outputs = u32;

    fn scan(&mut self, scan: &Scan, inputs: &u32, outputs: &mut u32) {
        self.scans += 1;
        self.last_now = scan.now;
        *outputs = self.scans * 100 + inputs;
    }

    fn safe_outputs(&self) -> u32 {
        7
    }
}

/// A plant that logs every exchange, feeds a constant input, and stands for slow I/O: the clock moves by
/// `stall(read)` between a read and the write that follows it.
struct Plant {
    clock: VirtualClock,
    input: u32,
    reads: Vec<Duration>,
    writes: Vec<(Duration, u32)>,
    stall: fn(usize) -> Duration,
}

impl Plant {
    fn new(clock: &VirtualClock) -> Self {
        Plant { clock: clock.clone(), input: 1, reads: Vec::new(), writes: Vec::new(), stall: |_| Duration::ZERO }
    }
}

impl Io<u32, u32> for Plant {
    fn read_inputs(&mut self, now: Duration) -> u32 {
        self.reads.push(now);
        self.clock.advance((self.stall)(self.reads.len() - 1));
        self.input
    }

    fn write_outputs(&mut self, now: Duration, outputs: &u32) {
        self.writes.push((now, *outputs));
    }
}

struct Key(bool);

impl Switch for Key {
    fn armed(&self) -> bool {
        self.0
    }
}

const ARMED: Key = Key(true);
const OFF: Key = Key(false);
const OFICINA: CommandSource = CommandSource::Oficina;
const LOCAL: CommandSource = CommandSource::Local;
const PROGRAM: CommandSource = CommandSource::Program;
const PRODUCTION: Target = Target { production: true, realtime: true, simulated: false };

fn config() -> TaskConfig {
    TaskConfig { name: "main", period: ms(10) }
}

fn controller(target: Target, restart: Restart) -> Controller<Count> {
    Controller::power_on(config(), target, restart).unwrap()
}

fn text(c: &Controller<Count>) -> String {
    c.state().to_string()
}

#[test]
fn a_period_of_zero_is_refused_when_the_controller_comes_up() {
    let zero = TaskConfig { name: "main", period: Duration::ZERO };
    let err = Controller::<Count>::power_on(zero, PRODUCTION, Restart::First).err();
    assert_eq!(err, Some(TaskError::ZeroPeriod { task: "main" }));
}

#[test]
fn a_new_controller_is_stopped_and_exchanges_the_safe_outputs_every_period() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let mut c = controller(PRODUCTION, Restart::First);
    assert_eq!(c.state().state, State::Stop(None));
    assert_eq!(text(&c), "STOP");
    assert_eq!(c.state().health(), Some(Health::FunctionCheck));
    for _ in 0..3 {
        c.step(&mut plant, &mut clock);
    }
    assert_eq!(plant.reads, vec![ms(0), ms(10), ms(20)]);
    assert_eq!(plant.writes, vec![(ms(0), 7), (ms(10), 7), (ms(20), 7)], "the program's safe outputs, in STOP");
    assert!(c.program().is_none());
}

#[test]
fn a_start_goes_through_startup_to_run_and_the_program_scans_from_its_own_clock() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let mut c = controller(PRODUCTION, Restart::First);
    c.step(&mut plant, &mut clock);
    clock.advance(ms(13)); // the command arrives at 13 ms
    assert_eq!(c.start(StartupReason::Cold, OFICINA, &ARMED), Ok(()));
    assert_eq!(
        c.state().state,
        State::Startup { reason: Some(StartupReason::Cold), phase: Some(StartupPhase::Load) }
    );
    assert_eq!(text(&c), "STARTUP cold(id=202) load()");
    assert_eq!(c.state().health(), Some(Health::FunctionCheck));
    c.step(&mut plant, &mut clock); // start-up ends and the first scan runs at once, at 13 ms
    assert_eq!(c.state().state, State::Run);
    assert_eq!(text(&c), "RUN");
    assert_eq!(c.state().health(), Some(Health::Normal));
    c.step(&mut plant, &mut clock);
    c.step(&mut plant, &mut clock);
    assert_eq!(plant.reads, vec![ms(0), ms(13), ms(23), ms(33)], "the program's periods from its start");
    assert_eq!(plant.writes, vec![(ms(0), 7), (ms(13), 101), (ms(23), 201), (ms(33), 301)]);
    let p = c.program().unwrap();
    assert_eq!((p.scans, p.last_now), (3, ms(20)), "the program's clock starts at its start");
}

#[test]
fn only_a_person_with_the_switch_armed_starts_it_and_only_from_stop() {
    let mut c = controller(PRODUCTION, Restart::First);
    let warm = Command::Start(StartupReason::Warm);
    let refused = c.start(StartupReason::Warm, PROGRAM, &ARMED).unwrap_err();
    assert_eq!(refused, Refusal::Source { command: warm, source: PROGRAM });
    assert_eq!(refused.to_string(), "a warm start from `program` refused: the program may only stop");
    let refused = c.start(StartupReason::Warm, OFICINA, &OFF).unwrap_err();
    assert_eq!(refused, Refusal::Switch { command: warm });
    assert_eq!(refused.to_string(), "a warm start refused: the commissioning switch is not armed");
    assert_eq!(text(&c), "STOP", "nothing changed");
    assert_eq!(c.start(StartupReason::Warm, LOCAL, &ARMED), Ok(()));
    let refused = c.start(StartupReason::Cold, OFICINA, &ARMED).unwrap_err();
    assert_eq!(refused, Refusal::State { command: Command::Start(StartupReason::Cold), state: StateWord::Startup });
    assert_eq!(refused.to_string(), "a cold start refused in STARTUP");
}

#[test]
fn a_stop_is_free_the_program_may_stop_itself_and_the_safe_outputs_follow_at_once() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let mut c = controller(PRODUCTION, Restart::First);
    c.start(StartupReason::Warm, OFICINA, &ARMED).unwrap();
    c.step(&mut plant, &mut clock);
    c.step(&mut plant, &mut clock);
    assert_eq!(c.stop(OFICINA), Ok(()), "no switch asked");
    assert_eq!(c.state().state, State::Stop(Some(StopReason::Command { source: OFICINA })));
    assert_eq!(text(&c), r#"STOP command(id=101, source="oficina")"#);
    assert!(c.program().is_none());
    c.step(&mut plant, &mut clock);
    c.step(&mut plant, &mut clock);
    assert_eq!(plant.writes, vec![(ms(0), 101), (ms(10), 201), (ms(10), 7), (ms(20), 7)]);
    let refused = c.stop(LOCAL).unwrap_err();
    assert_eq!(refused, Refusal::State { command: Command::Stop, state: StateWord::Stop });
    assert_eq!(refused.to_string(), "a stop refused in STOP");

    c.start(StartupReason::Warm, LOCAL, &ARMED).unwrap();
    assert_eq!(c.stop(PROGRAM), Ok(()), "a stop from STARTUP, by the program itself");
    assert_eq!(text(&c), r#"STOP command(id=101, source="program")"#);
}

#[test]
fn a_scan_past_twice_its_period_faults_the_controller_and_only_an_acknowledge_then_a_start_leave_fault() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    plant.stall = |read| if read == 2 { ms(25) } else { Duration::ZERO };
    let mut c = controller(PRODUCTION, Restart::First);
    c.start(StartupReason::Warm, OFICINA, &ARMED).unwrap();
    c.step(&mut plant, &mut clock); // scan 0 at 0 ms
    c.step(&mut plant, &mut clock); // scan 1 at 10 ms
    c.step(&mut plant, &mut clock); // scan 2 at 20 ms, its read stalls 25 ms: past twice the period
    let fault = State::Fault(Some(FaultReason::Watchdog { task: "main".to_string() }));
    assert_eq!(c.state().state, fault);
    assert_eq!(text(&c), r#"FAULT watchdog(id=501, task="main") overruns(count=1)"#);
    assert_eq!(c.state().health(), Some(Health::Failure));
    c.step(&mut plant, &mut clock); // faulted: the inputs read, the safe outputs written, the exchange goes on
    assert_eq!(plant.writes, vec![(ms(0), 101), (ms(10), 201), (ms(45), 7), (ms(50), 7)]);

    assert_eq!(c.stop(OFICINA), Err(Refusal::State { command: Command::Stop, state: StateWord::Fault }));
    assert_eq!(
        c.start(StartupReason::Warm, OFICINA, &ARMED),
        Err(Refusal::State { command: Command::Start(StartupReason::Warm), state: StateWord::Fault }),
        "leaving FAULT takes an acknowledge first"
    );
    assert_eq!(
        c.acknowledge(PROGRAM, &ARMED),
        Err(Refusal::Source { command: Command::Acknowledge, source: PROGRAM }),
        "a fault always needs a person"
    );
    assert_eq!(c.acknowledge(OFICINA, &OFF), Err(Refusal::Switch { command: Command::Acknowledge }));
    assert_eq!(c.state().state, fault, "nothing changed");
    assert_eq!(c.acknowledge(LOCAL, &ARMED), Ok(()));
    assert_eq!(c.state().state, State::Stop(Some(StopReason::Acknowledged)));
    assert_eq!(text(&c), "STOP acknowledged(id=103) overruns(count=1)");
    assert_eq!(c.acknowledge(LOCAL, &ARMED), Err(Refusal::State { command: Command::Acknowledge, state: StateWord::Stop }));

    c.start(StartupReason::Cold, OFICINA, &ARMED).unwrap();
    c.step(&mut plant, &mut clock);
    assert_eq!(text(&c), "RUN", "a new start counts its overruns from zero");
    assert_eq!(c.program().unwrap().scans, 1, "the program starts again from its initial values");
}

#[test]
fn after_a_power_cycle_run_returns_as_run_and_stop_hold_and_fault_as_stop() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let mut c = controller(PRODUCTION, Restart::PowerCycle { was: StateWord::Run });
    assert_eq!(text(&c), "STARTUP warm(id=201) load()", "a warm start, no person needed");
    c.step(&mut plant, &mut clock);
    assert_eq!(text(&c), "RUN");
    assert_eq!(plant.writes, vec![(ms(0), 101)]);

    let c = controller(PRODUCTION, Restart::PowerCycle { was: StateWord::Startup });
    assert_eq!(text(&c), "STARTUP warm(id=201) load()", "a start-up a power cut interrupted starts again");

    for (was, word) in [(StateWord::Stop, "STOP"), (StateWord::Hold, "HOLD"), (StateWord::Fault, "FAULT")] {
        let c = controller(PRODUCTION, Restart::PowerCycle { was });
        assert_eq!(c.state().state, State::Stop(Some(StopReason::PowerOn { was })));
        assert_eq!(text(&c), format!(r#"STOP power_on(id=102, was="{word}")"#));
    }
}

#[test]
fn a_runtime_that_restarted_alone_comes_up_in_fault_and_exchanges_the_safe_outputs() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let mut c = controller(PRODUCTION, Restart::Crash);
    assert_eq!(c.state().state, State::Fault(Some(FaultReason::Internal)));
    assert_eq!(text(&c), "FAULT internal(id=507)");
    c.step(&mut plant, &mut clock);
    c.step(&mut plant, &mut clock);
    assert_eq!(plant.writes, vec![(ms(0), 7), (ms(10), 7)]);
    assert_eq!(c.acknowledge(OFICINA, &ARMED), Ok(()));
    assert_eq!(text(&c), "STOP acknowledged(id=103)");
}

#[test]
fn a_production_target_without_real_time_stays_in_stop() {
    let without = Target { production: true, realtime: false, simulated: false };
    let mut c = controller(without, Restart::First);
    assert_eq!(text(&c), "STOP", "no condition: a production target never runs without real time");
    let refused = c.start(StartupReason::Warm, OFICINA, &ARMED).unwrap_err();
    assert_eq!(refused, Refusal::Realtime);
    assert_eq!(refused.to_string(), "a start refused: this production target has no real-time scheduling");
    assert_eq!(text(&c), "STOP realtime_refused(id=105)");
    let c = controller(without, Restart::PowerCycle { was: StateWord::Run });
    assert_eq!(text(&c), "STOP realtime_refused(id=105)", "after a power cycle too");
}

#[test]
fn development_and_simulation_run_without_real_time_and_say_so() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    let simulator = Target { production: false, realtime: false, simulated: true };
    let mut c = controller(simulator, Restart::First);
    assert_eq!(text(&c), "STOP sim() no_realtime()");
    c.start(StartupReason::Warm, OFICINA, &ARMED).unwrap();
    c.step(&mut plant, &mut clock);
    assert_eq!(text(&c), "RUN sim() no_realtime()");
    assert_eq!(c.state().health(), Some(Health::FunctionCheck), "a debug condition is a function check");
    let desk = Target { production: false, realtime: true, simulated: false };
    assert_eq!(text(&controller(desk, Restart::First)), "STOP");
}

#[test]
fn time_errors_show_as_overruns_while_the_controller_runs() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock);
    plant.stall = |read| if read == 1 { ms(15) } else { Duration::ZERO };
    let mut c = controller(PRODUCTION, Restart::First);
    c.start(StartupReason::Warm, OFICINA, &ARMED).unwrap();
    c.step(&mut plant, &mut clock);
    c.step(&mut plant, &mut clock); // 15 ms: a time error, under twice the period
    assert_eq!(text(&c), "RUN overruns(count=1)");
    assert_eq!(c.state().health(), Some(Health::Normal), "an overrun is no debug condition");
    c.step(&mut plant, &mut clock);
    assert_eq!(text(&c), "RUN overruns(count=1)");
}
