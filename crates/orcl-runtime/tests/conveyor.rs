//! The conveyor of Veredas 1's `demos/conveyor`, carried as a fixture (`docs/orcl/README.md`, "What it absorbs
//! from Veredas 1"): an ORCL program, a conveyor with a guarded door and a start, stop and fault machine, with its
//! tests on simulated time. Its host binary is not carried.
#![allow(clippy::arithmetic_side_effects)] // the tests' own time arithmetic, as Veredas 1's tests allowed it

/// The program, held to ORCL's rules as Veredas 1's `no_std` logic crate was: no `unsafe`, no panic path, no
/// interior mutability (the lints below; the list of types O13 forbids is kept by convention until a `clippy.toml`
/// may sit in the repository, `docs/questions.md` Q225, DECISION PENDING). Its function blocks, [`Door`] and
/// [`Conveyor`], are not `Copy`.
///
/// - [`Door`]: a two-channel guard door. A channel discrepancy that lasts 500 ms raises the alarm, so a short
///   flicker is filtered out. The pattern comes from a safety-door template.
/// - [`Conveyor`]: the machine - a `match` over [`State`]. The compiler forces every state to be handled.
mod conveyor {
    #![forbid(unsafe_code)]
    #![deny(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::disallowed_types
    )]

    use orcl_std::{Ctu, Program, RTrig, Scan, Ton, ms, s};

    /// The process image the program reads: one consistent snapshot per scan.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Inputs {
        pub start_pb: bool,
        pub stop_pb: bool,
        pub ack_pb: bool,
        pub door_ch1: bool,
        pub door_ch2: bool,
        pub part_at_end: bool,
    }

    /// What the program drives. The default (everything off) is the safe state.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Outputs {
        pub motor_on: bool,
        pub door_alarm: bool,
        pub lamp_running: bool,
        pub lamp_fault: bool,
        pub parts: i32,
    }

    /// A two-channel guard door. Both channels on = closed. A discrepancy, or the door open, for 500 ms raises
    /// the alarm.
    #[derive(Debug, Default)]
    pub struct Door {
        discrepancy: Ton,
        alarm: bool,
    }

    impl Door {
        pub fn scan(&mut self, scan: &Scan, ch1: bool, ch2: bool) {
            let not_closed = !ch1 || !ch2;
            self.alarm = self.discrepancy.scan(scan, not_closed, ms(500));
        }

        pub fn alarm(&self) -> bool {
            self.alarm
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum State {
        #[default]
        Idle,
        Running,
        /// Stop requested: the belt runs on for 2 s to clear, then stops.
        Stopping,
        /// Door alarm: the motor stops at once; an acknowledge with the door closed returns to Idle.
        Fault,
    }

    #[derive(Debug, Default)]
    pub struct Conveyor {
        state: State,
        door: Door,
        start: RTrig,
        part: RTrig,
        clearing: Ton,
        parts: Ctu,
    }

    impl Conveyor {
        pub fn state(&self) -> State {
            self.state
        }
    }

    impl Program for Conveyor {
        type Inputs = Inputs;
        type Outputs = Outputs;

        fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
            self.door.scan(scan, i.door_ch1, i.door_ch2);
            let alarm = self.door.alarm();
            let start = self.start.scan(i.start_pb);
            let cleared = self
                .clearing
                .scan(scan, self.state == State::Stopping, s(2));

            self.state = match self.state {
                State::Idle => {
                    if start && !alarm {
                        State::Running
                    } else {
                        State::Idle
                    }
                }
                State::Running => {
                    if alarm {
                        State::Fault
                    } else if i.stop_pb {
                        State::Stopping
                    } else {
                        State::Running
                    }
                }
                State::Stopping => {
                    if alarm {
                        State::Fault
                    } else if cleared {
                        State::Idle
                    } else {
                        State::Stopping
                    }
                }
                State::Fault => {
                    if i.ack_pb && !alarm {
                        State::Idle
                    } else {
                        State::Fault
                    }
                }
            };

            let moving = matches!(self.state, State::Running | State::Stopping);
            let part = self.part.scan(i.part_at_end);
            self.parts.scan(part && moving, false, i32::MAX);

            o.motor_on = moving;
            o.door_alarm = alarm;
            o.lamp_running = self.state == State::Running;
            o.lamp_fault = self.state == State::Fault;
            o.parts = self.parts.cv();
        }
    }
}

use conveyor::{Conveyor, Inputs, Outputs, State};
use orcl_runtime::{Io, RunState, Task, TaskConfig, VirtualClock};
use std::time::Duration;
use orcl_std::{Program, Scan, ms};

/// Drives the program on SIMULATED time: 10 ms scans from `from` to `to` (ms), the inputs given by `inputs`
/// as a function of time. Returns the outputs of the last scan. No waiting: hours of plant time run in
/// milliseconds.
fn run(c: &mut Conveyor, from: u64, to: u64, inputs: impl Fn(u64) -> Inputs) -> Outputs {
    let mut o = Outputs::default();
    let mut t = from;
    while t < to {
        c.scan(
            &Scan {
                now: ms(t),
                period: ms(10),
                cycle: t / 10,
            },
            &inputs(t),
            &mut o,
        );
        t += 10;
    }
    o
}

fn closed() -> Inputs {
    Inputs {
        door_ch1: true,
        door_ch2: true,
        ..Inputs::default()
    }
}

fn started(c: &mut Conveyor) {
    run(c, 0, 100, |t| Inputs {
        start_pb: t < 50,
        ..closed()
    });
    assert_eq!(c.state(), State::Running);
}

#[test]
fn start_runs_and_counts_parts() {
    let mut c = Conveyor::default();
    started(&mut c);
    // a part at the end of the belt every 300 ms, 50 ms long
    let o = run(&mut c, 100, 1300, |t| Inputs {
        part_at_end: t % 300 < 50,
        ..closed()
    });
    assert!(o.motor_on && o.lamp_running);
    assert_eq!(o.parts, 4);
}

#[test]
fn a_door_flicker_shorter_than_500ms_is_filtered() {
    let mut c = Conveyor::default();
    started(&mut c);
    let o = run(&mut c, 100, 1000, |t| Inputs {
        door_ch1: !(200..690).contains(&t),
        ..closed()
    });
    assert_eq!(c.state(), State::Running, "490 ms of discrepancy: no alarm");
    assert!(!o.door_alarm);
}

#[test]
fn a_door_open_for_500ms_stops_the_motor_at_once() {
    let mut c = Conveyor::default();
    started(&mut c);
    let open = || Inputs {
        door_ch2: false,
        ..closed()
    };
    let o = run(&mut c, 100, 600, |_| closed());
    assert!(o.motor_on, "door closed so far");
    let o = run(&mut c, 600, 1100, |_| open());
    assert_eq!(
        c.state(),
        State::Running,
        "the last scan saw 490 ms open: not yet"
    );
    assert!(o.motor_on);
    let o = run(&mut c, 1100, 1110, |_| open());
    assert_eq!(c.state(), State::Fault, "500 ms open");
    assert!(
        o.door_alarm && o.lamp_fault && !o.motor_on,
        "the motor stops in the same scan"
    );
}

#[test]
fn a_fault_needs_an_acknowledge_with_the_door_closed() {
    let mut c = Conveyor::default();
    started(&mut c);
    run(&mut c, 100, 700, |_| Inputs {
        door_ch1: false,
        ..closed()
    });
    assert_eq!(c.state(), State::Fault);
    run(&mut c, 700, 800, |_| Inputs {
        door_ch1: false,
        ack_pb: true,
        ..closed()
    });
    assert_eq!(
        c.state(),
        State::Fault,
        "acknowledge with the door still open: refused"
    );
    run(&mut c, 800, 900, |_| Inputs {
        ack_pb: true,
        ..closed()
    });
    assert_eq!(c.state(), State::Idle);
    run(&mut c, 900, 1000, |t| Inputs {
        start_pb: t < 950,
        ..closed()
    });
    assert_eq!(c.state(), State::Running, "and it can start again");
}

#[test]
fn stop_runs_the_belt_on_for_two_seconds_to_clear() {
    let mut c = Conveyor::default();
    started(&mut c);
    let o = run(&mut c, 100, 2000, |t| Inputs {
        stop_pb: t < 150,
        ..closed()
    });
    assert_eq!(c.state(), State::Stopping);
    assert!(
        o.motor_on && !o.lamp_running,
        "clearing: motor on, running lamp off"
    );
    let o = run(&mut c, 2000, 2200, |_| closed());
    assert_eq!(c.state(), State::Idle);
    assert!(!o.motor_on);
}

/// The plant of Veredas 1's demo host, as it was: an operator's script (who presses what, when) and a belt that
/// brings a part to the end sensor every 1.2 s while the motor runs, the motor being the last output written.
struct Plant {
    motor_on: bool,
}

fn between(t: u128, from: u128, to: u128) -> bool {
    (from..to).contains(&t)
}

impl Io<Inputs, Outputs> for Plant {
    fn read_inputs(&mut self, now: Duration) -> Inputs {
        let t = now.as_millis();
        Inputs {
            start_pb: between(t, 1_000, 1_200) || between(t, 14_000, 14_200),
            stop_pb: between(t, 19_000, 19_200),
            ack_pb: between(t, 13_000, 13_200),
            // a 300 ms flicker (filtered), then the door open for 2 s (alarm after 500 ms)
            door_ch1: !(between(t, 7_000, 7_300) || between(t, 10_000, 12_000)),
            door_ch2: true,
            part_at_end: self.motor_on && t % 1_200 < 150,
        }
    }

    fn write_outputs(&mut self, _now: Duration, outputs: &Outputs) {
        self.motor_on = outputs.motor_on;
    }
}

/// The demo's 22 s, run by the engine on virtual time with a 10 ms task: every change of state, of the door
/// alarm and of the part count lands on the scan the program's own timers and edges say, and the run takes
/// milliseconds.
#[test]
fn the_engine_runs_the_conveyor_against_its_plant_on_virtual_time() {
    let mut clock = VirtualClock::new();
    let mut task = Task::new(TaskConfig { name: "main", period: ms(10) }, Conveyor::default()).unwrap();
    let mut plant = Plant { motor_on: false };
    let (mut state, mut alarm, mut parts) = (State::Idle, false, 0);
    let (mut states, mut alarms, mut counted) = (Vec::new(), Vec::new(), Vec::new());
    task.run_until(&mut plant, &mut clock, ms(22_000), |r| {
        let t = r.scan.now.as_millis() as u64;
        if r.program.state() != state {
            states.push((t, state, r.program.state()));
            state = r.program.state();
        }
        if r.outputs.door_alarm != alarm {
            alarm = r.outputs.door_alarm;
            alarms.push((t, alarm));
        }
        if r.outputs.parts != parts {
            parts = r.outputs.parts;
            counted.push(t);
        }
    });
    assert_eq!(
        states,
        vec![
            (1_000, State::Idle, State::Running),
            (10_500, State::Running, State::Fault),
            (13_000, State::Fault, State::Idle),
            (14_000, State::Idle, State::Running),
            (19_000, State::Running, State::Stopping),
            (21_010, State::Stopping, State::Idle),
        ],
        "start, the door's alarm 500 ms after it opened, the acknowledge, a new start, the stop and 2 s of clearing \
         from the scan after it"
    );
    assert_eq!(alarms, vec![(10_500, true), (12_000, false)], "the 300 ms flicker at 7 s raised nothing");
    assert_eq!(
        counted,
        vec![1_200, 2_400, 3_600, 4_800, 6_000, 7_200, 8_400, 9_600, 14_400, 15_600, 16_800, 18_000, 19_200, 20_400],
        "a part every 1.2 s while the belt moves, clearing included"
    );
    let s = *task.stats();
    assert_eq!((s.cycles, s.overruns, s.skipped), (2_200, 0, 0));
    assert_eq!(task.state(), RunState::Running);
}
