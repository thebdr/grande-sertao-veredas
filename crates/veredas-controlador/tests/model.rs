//! The controller against a reference model of C-152, over thousands of random sequences on virtual time.
//!
//! The model is written from the clause, apart from the controller: how it comes up (from nothing, a power cycle from
//! each state, a crash) on every kind of target; every command from every source, with the switch armed or not, in
//! every state, with the refusal each one owes; the state and its conditions, the count of overruns, the program's
//! scans and clock; and every exchange with the plant by its time and value, the program's on its own periods and the
//! safe outputs' on the target's, once in each, while host steps come late, the clock jumps and reads stall. A
//! sequence is cut at its first disagreement and reported with the operations that led to it. First written by the
//! critic of the fourth refute round (`.zen/evidence/C-152.md`) and taken in here as C-152's own test.
use std::time::Duration;

use orcl_runtime::{Clock, Io, TaskConfig, VirtualClock};
use orcl_std::{Program, Scan};
use veredas_controlador::{Controller, Refusal, Restart, Switch, Target};
use veredas_core::state::{
    CommandSource, ControllerState, FaultReason, StartupPhase, StartupReason, State, StateWord, StopReason,
};

/// Counts its scans into its output; its safe outputs are 7.
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

/// Logs every exchange; its `k`-th read stalls the clock by `stalls[k]`; feeds 1.
struct Plant {
    clock: VirtualClock,
    reads: Vec<Duration>,
    writes: Vec<(Duration, u32)>,
    stalls: Vec<Duration>,
}

impl Io<u32, u32> for Plant {
    fn read_inputs(&mut self, now: Duration) -> u32 {
        let index = self.reads.len();
        self.reads.push(now);
        self.clock.advance(self.stalls.get(index).copied().unwrap_or(Duration::ZERO));
        1
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

/// A deterministic generator, so every run draws the same sequences.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn source(&mut self) -> CommandSource {
        match self.below(3) {
            0 => CommandSource::Program,
            1 => CommandSource::Local,
            _ => CommandSource::Oficina,
        }
    }
}

const PERIOD: Duration = Duration::from_millis(10);

fn periods_up_to(t: Duration) -> u128 {
    t.as_nanos().div_ceil(PERIOD.as_nanos())
}

fn period_of(t: Duration) -> u128 {
    t.as_nanos() / PERIOD.as_nanos()
}

fn deadline(k: u128) -> Duration {
    Duration::from_nanos(u64::try_from(k * PERIOD.as_nanos()).unwrap())
}

/// The model's task: where its program's clock starts, its next cycle, its program's scans and clock, faulted or not.
struct Task {
    origin: Duration,
    cycle: u128,
    scans: u32,
    last_now: Duration,
    faulted: bool,
}

/// How often the draws reached the cases the clause names, over all sequences.
#[derive(Default)]
struct Reached {
    starts: u32,
    stops_while_running: u32,
    acknowledges: u32,
    watchdog_faults: u32,
    late_idle_steps: u32,
    refusals: u32,
    starts_after_a_fault: u32,
}

/// Runs one sequence; returns the first disagreement, with the operations that led to it.
fn run_one(seed: u64, reached: &mut Reached) -> Result<(), String> {
    let mut draw = Draw(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x0bad_c0de);
    let clock = VirtualClock::new();
    let mut host_clock = clock.clone();
    let stalls: Vec<Duration> = (0..120)
        .map(|_| match draw.below(10) {
            0 => Duration::from_millis(draw.below(32)),
            1 => Duration::from_micros(draw.below(32_000)),
            2 => [PERIOD, PERIOD * 2, PERIOD + Duration::from_nanos(1), PERIOD * 2 + Duration::from_nanos(1)]
                [usize::try_from(draw.below(4)).unwrap()],
            _ => Duration::ZERO,
        })
        .collect();
    let mut plant = Plant { clock: clock.clone(), reads: Vec::new(), writes: Vec::new(), stalls: stalls.clone() };
    let restart = match draw.below(8) {
        0 => Restart::Crash,
        1 => Restart::PowerCycle { was: StateWord::Run },
        2 => Restart::PowerCycle { was: StateWord::Startup },
        3 => Restart::PowerCycle { was: StateWord::Fault },
        4 => Restart::PowerCycle { was: StateWord::Hold },
        5 => Restart::PowerCycle { was: StateWord::Stop },
        _ => Restart::First,
    };
    let target = Target { production: draw.below(3) != 0, realtime: draw.below(3) != 0, simulated: draw.below(2) == 0 };
    let may_run = !(target.production && !target.realtime);
    let mut c = Controller::<Count>::power_on(TaskConfig { name: "t", period: PERIOD }, target, restart).unwrap();

    let mut m = ControllerState::new(State::Stop(None));
    m.conditions.sim = target.simulated;
    m.conditions.no_realtime = !target.realtime && !target.production;
    m.state = match restart {
        Restart::First => State::Stop(None),
        Restart::PowerCycle { was: StateWord::Run | StateWord::Startup } if may_run => {
            State::Startup { reason: Some(StartupReason::Warm), phase: Some(StartupPhase::Load) }
        }
        Restart::PowerCycle { was: StateWord::Run | StateWord::Startup } => State::Stop(Some(StopReason::RealtimeRefused)),
        Restart::PowerCycle { was } => State::Stop(Some(StopReason::PowerOn { was })),
        Restart::Crash => State::Fault(Some(FaultReason::Internal)),
    };
    let mut task: Option<Task> = None;
    let mut last_read: Option<Duration> = None;
    let mut faulted_before = false;
    let mut trace = vec![format!("{restart:?} on {target:?}")];

    for op in 0..60 {
        let word = m.state.word();
        let what = match draw.below(10) {
            0..=4 => {
                let arrival = host_clock.now();
                let (reads, writes) = (plant.reads.len(), plant.writes.len());
                c.step(&mut plant, &mut host_clock);
                if word == StateWord::Startup {
                    task = Some(Task { origin: arrival, cycle: 0, scans: 0, last_now: Duration::ZERO, faulted: false });
                    m.state = State::Run;
                }
                let stall = stalls.get(reads).copied().unwrap_or(Duration::ZERO);
                let (read_at, written) = match task.as_mut() {
                    Some(t) => {
                        let read_at = (t.origin + deadline(t.cycle)).max(arrival);
                        let end = read_at + stall;
                        let mut out = 7;
                        if !t.faulted {
                            t.scans += 1;
                            t.last_now = read_at - t.origin;
                            out = t.scans * 100 + 1;
                            if stall > PERIOD {
                                m.conditions.overruns += 1;
                                if stall > PERIOD * 2 {
                                    t.faulted = true;
                                    out = 7;
                                    if m.state.word() == StateWord::Run {
                                        m.state = State::Fault(Some(FaultReason::Watchdog { task: "t".into() }));
                                        reached.watchdog_faults += 1;
                                        faulted_before = true;
                                    }
                                }
                            }
                        }
                        t.cycle = (t.cycle + 1).max(periods_up_to(end - t.origin));
                        (read_at, (end, out))
                    }
                    None => {
                        let k = periods_up_to(arrival).max(last_read.map_or(0, |l| period_of(l) + 1));
                        if deadline(k) > arrival && !arrival.as_nanos().is_multiple_of(PERIOD.as_nanos()) {
                            reached.late_idle_steps += 1;
                        }
                        let read_at = deadline(k);
                        (read_at, (read_at + stall, 7))
                    }
                };
                last_read = Some(read_at);
                if plant.reads.len() != reads + 1 || plant.reads[reads] != read_at {
                    return Err(format!("seed {seed} op {op}: read {:?}, the model's {read_at:?} | {}", &plant.reads[reads..], trace.join("; ")));
                }
                if plant.writes.len() != writes + 1 || plant.writes[writes] != written {
                    return Err(format!("seed {seed} op {op}: write {:?}, the model's {written:?} | {}", &plant.writes[writes..], trace.join("; ")));
                }
                format!("step at {arrival:?}")
            }
            5 => {
                let jump = Duration::from_micros(draw.below(35_000));
                clock.advance(jump);
                format!("clock +{jump:?}")
            }
            6 => {
                let kind = if draw.below(2) == 0 { StartupReason::Warm } else { StartupReason::Cold };
                let source = draw.source();
                let armed = draw.below(4) != 0;
                let got = c.start(kind, source, &Key(armed));
                let want = if source == CommandSource::Program {
                    Err("source")
                } else if !armed {
                    Err("switch")
                } else if word != StateWord::Stop {
                    Err("state")
                } else if !may_run {
                    Err("real time")
                } else {
                    Ok(())
                };
                let got_kind = got.as_ref().map(|_| ()).map_err(|r| match r {
                    Refusal::Source { .. } => "source",
                    Refusal::Switch { .. } => "switch",
                    Refusal::State { .. } => "state",
                    Refusal::Realtime => "real time",
                });
                if got_kind != want {
                    return Err(format!("seed {seed} op {op}: start {got:?}, the model's {want:?} in {m}"));
                }
                match want {
                    Ok(()) => {
                        m.state = State::Startup { reason: Some(kind), phase: Some(StartupPhase::Load) };
                        m.conditions.overruns = 0;
                        reached.starts += 1;
                        if faulted_before {
                            reached.starts_after_a_fault += 1;
                        }
                    }
                    Err("real time") => m.state = State::Stop(Some(StopReason::RealtimeRefused)),
                    Err(_) => reached.refusals += 1,
                }
                format!("start {kind:?} by {source:?}, armed {armed}: {got:?}")
            }
            7 | 8 => {
                let source = draw.source();
                let got = c.stop(source);
                let allowed = matches!(word, StateWord::Run | StateWord::Startup);
                if got.is_ok() != allowed {
                    return Err(format!("seed {seed} op {op}: stop {got:?} in {m}"));
                }
                if allowed {
                    m.state = State::Stop(Some(StopReason::Command { source }));
                    task = None;
                    if word == StateWord::Run {
                        reached.stops_while_running += 1;
                    }
                } else {
                    reached.refusals += 1;
                }
                format!("stop by {source:?}: {got:?}")
            }
            _ => {
                let source = draw.source();
                let armed = draw.below(3) != 0;
                let got = c.acknowledge(source, &Key(armed));
                let allowed = source != CommandSource::Program && armed && word == StateWord::Fault;
                if got.is_ok() != allowed {
                    return Err(format!("seed {seed} op {op}: acknowledge {got:?} in {m}"));
                }
                if allowed {
                    m.state = State::Stop(Some(StopReason::Acknowledged));
                    task = None;
                    reached.acknowledges += 1;
                } else {
                    reached.refusals += 1;
                }
                format!("acknowledge by {source:?}, armed {armed}: {got:?}")
            }
        };
        trace.push(what);
        if c.state() != &m {
            return Err(format!("seed {seed} op {op}: state {}, the model's {m} | {}", c.state(), trace.join("; ")));
        }
        if c.state().health() != m.health() {
            return Err(format!("seed {seed} op {op}: health {:?}, the model's {:?}", c.state().health(), m.health()));
        }
        let program = c.program().map(|p| (p.scans, p.last_now));
        let model_program = task.as_ref().map(|t| (t.scans, t.last_now));
        if program != model_program {
            return Err(format!("seed {seed} op {op}: program {program:?}, the model's {model_program:?} | {}", trace.join("; ")));
        }
    }
    Ok(())
}

#[test]
fn the_controller_agrees_with_the_clause_s_model_over_five_thousand_random_sequences() {
    let mut reached = Reached::default();
    let disagreements: Vec<String> = (0..5_000).filter_map(|seed| run_one(seed, &mut reached).err()).collect();
    assert!(disagreements.is_empty(), "{} sequences disagree; the first: {}", disagreements.len(), disagreements[0]);
    for (case, count) in [
        ("a start", reached.starts),
        ("a stop while running", reached.stops_while_running),
        ("an acknowledge", reached.acknowledges),
        ("a watchdog fault", reached.watchdog_faults),
        ("an idle step coming between periods", reached.late_idle_steps),
        ("a refusal", reached.refusals),
        ("a start after a fault", reached.starts_after_a_fault),
    ] {
        assert!(count >= 100, "{case}: reached {count} times only");
    }
}
