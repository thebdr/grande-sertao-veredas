//! A conformance program: Veredas 1's conveyor as `crates/orcl-runtime/tests/conveyor.rs` carries it (a conveyor with
//! a guarded door and a start, stop and fault machine), its program alone, compiled natively into the lowering test
//! and by the driver as a crate of its own.
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
