//! A controller's state: structured data, and its one line of text with one writer and one
//! reader (`docs/state-text.md`).
//!
//! The text has two layers. Layer 1 is one uppercase word ([`StateWord`]); layer 2 is
//! function-style terms after it: at most one reason, the start-up phase while starting, then
//! every condition that applies in a fixed order, then each maintenance request. A reason is an
//! entry of the table [`REASONS`] with a stable number whose hundreds digit is its state, and the
//! text shows both, the number as the first argument `id`:
//!
//! ```text
//! HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)
//! RUN maintenance_request(id=3, reason="device_missing", urgent=true)
//! ```
//!
//! [`ControllerState`]'s `Display` is the writer and its `FromStr` the reader. The writer is
//! total; the reader accepts exactly the texts the writer writes and refuses every other
//! spelling by name, so a text and a state are one thing: reading a text and writing it back
//! gives the same characters, and writing a state and reading it back gives the same state.
//! Values are TOML values (an integer in decimal, `true` or `false`, a string in double quotes
//! with TOML's escapes), each written one way only.
//!
//! Where the controller's plan left a reading open, the reading taken here is the architect's
//! ruling (`docs/questions.md` Q100, 2026-11-25: all eight readings accepted as taken).

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Layer 1: the state's word and its number. `UNKNOWN` (0) is the state of a controller whose
/// state is not known; its word is a reading (Q100).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateWord {
    Unknown,
    Stop,
    Startup,
    Run,
    Hold,
    Fault,
}

impl StateWord {
    /// Every state, in the order of its number.
    pub const ALL: [StateWord; 6] = [
        StateWord::Unknown,
        StateWord::Stop,
        StateWord::Startup,
        StateWord::Run,
        StateWord::Hold,
        StateWord::Fault,
    ];

    /// 0 unknown, 1 STOP, 2 STARTUP, 3 RUN, 4 HOLD, 5 FAULT: a reason's hundreds digit.
    pub fn number(self) -> u8 {
        match self {
            StateWord::Unknown => 0,
            StateWord::Stop => 1,
            StateWord::Startup => 2,
            StateWord::Run => 3,
            StateWord::Hold => 4,
            StateWord::Fault => 5,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            // Ruled (Q100): the plan numbers the unknown state 0 and gives it no word.
            StateWord::Unknown => "UNKNOWN",
            StateWord::Stop => "STOP",
            StateWord::Startup => "STARTUP",
            StateWord::Run => "RUN",
            StateWord::Hold => "HOLD",
            StateWord::Fault => "FAULT",
        }
    }

    pub fn from_number(number: u8) -> Option<StateWord> {
        StateWord::ALL.into_iter().find(|s| s.number() == number)
    }

    pub fn from_word(word: &str) -> Option<StateWord> {
        StateWord::ALL.into_iter().find(|s| s.word() == word)
    }
}

impl fmt::Display for StateWord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// Who commanded a stop or a halt. No peer is a source: nobody outside changes the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandSource {
    /// The engineering workbench, over its link to the controller.
    Oficina,
    /// At the target itself.
    Local,
    /// The running program, stopping itself.
    Program,
}

impl CommandSource {
    pub const ALL: [CommandSource; 3] = [
        CommandSource::Oficina,
        CommandSource::Local,
        CommandSource::Program,
    ];

    pub fn word(self) -> &'static str {
        match self {
            CommandSource::Oficina => "oficina",
            CommandSource::Local => "local",
            CommandSource::Program => "program",
        }
    }

    pub fn from_word(word: &str) -> Option<CommandSource> {
        CommandSource::ALL.into_iter().find(|s| s.word() == word)
    }
}

/// One row of the table of reasons: its stable number (the hundreds digit is its state), its
/// word, and the arguments that follow `id`, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReasonEntry {
    pub number: u16,
    pub word: &'static str,
    pub args: &'static [&'static str],
}

impl ReasonEntry {
    /// The state this reason belongs to: its hundreds digit.
    pub fn state(&self) -> Option<StateWord> {
        u8::try_from(self.number / 100)
            .ok()
            .and_then(StateWord::from_number)
    }
}

const fn reason(number: u16, word: &'static str, args: &'static [&'static str]) -> ReasonEntry {
    ReasonEntry { number, word, args }
}

/// The table of reason numbers and words. RUN has no reason.
pub const REASONS: [ReasonEntry; 18] = [
    reason(101, "command", &["source"]),
    reason(102, "power_on", &["was"]),
    reason(103, "acknowledged", &[]),
    reason(104, "no_program", &[]),
    reason(105, "realtime_refused", &[]),
    reason(201, "warm", &[]),
    reason(202, "cold", &[]),
    reason(401, "breakpoint", &["at"]),
    reason(402, "command", &["source"]),
    reason(403, "step", &[]),
    reason(404, "cycle", &[]),
    reason(501, "watchdog", &["task"]),
    reason(502, "program", &["at"]),
    reason(503, "user", &["code"]),
    reason(504, "handler", &["event"]),
    reason(505, "resource", &["kind"]),
    reason(506, "load", &["download"]),
    reason(507, "internal", &[]),
];

/// The row of the table with this number.
pub fn reason_entry(number: u16) -> Option<&'static ReasonEntry> {
    REASONS.iter().find(|r| r.number == number)
}

fn entry_word(number: u16) -> &'static str {
    reason_entry(number).map_or("", |r| r.word)
}

/// Why the controller is in STOP.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StopReason {
    /// 101: stopped on a command.
    Command { source: CommandSource },
    /// 102: back from a power cycle; `was` is the state it was in (STOP, HOLD and FAULT all
    /// return as STOP).
    PowerOn { was: StateWord },
    /// 103: a fault acknowledged.
    Acknowledged,
    /// 104: no program to run.
    NoProgram,
    /// 105: the target refused real-time scheduling.
    RealtimeRefused,
}

impl StopReason {
    pub fn number(&self) -> u16 {
        match self {
            StopReason::Command { .. } => 101,
            StopReason::PowerOn { .. } => 102,
            StopReason::Acknowledged => 103,
            StopReason::NoProgram => 104,
            StopReason::RealtimeRefused => 105,
        }
    }

    pub fn word(&self) -> &'static str {
        entry_word(self.number())
    }
}

/// How the controller is starting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupReason {
    /// 201: retained variables kept; the normal start, also after a power cycle.
    Warm,
    /// 202: everything reinitialised, on command.
    Cold,
}

impl StartupReason {
    pub fn number(&self) -> u16 {
        match self {
            StartupReason::Warm => 201,
            StartupReason::Cold => 202,
        }
    }

    pub fn word(&self) -> &'static str {
        entry_word(self.number())
    }
}

/// One row of the table of start-up phases: a small enum of its own, numbered 1 to 4. A phase
/// shows no `id` in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseEntry {
    pub number: u8,
    pub word: &'static str,
    pub args: &'static [&'static str],
}

/// The table of start-up phases.
pub const PHASES: [PhaseEntry; 4] = [
    PhaseEntry {
        number: 1,
        word: "load",
        args: &[],
    },
    PhaseEntry {
        number: 2,
        word: "restore",
        args: &[],
    },
    PhaseEntry {
        number: 3,
        word: "handler",
        args: &[],
    },
    PhaseEntry {
        number: 4,
        word: "devices",
        args: &["up", "of"],
    },
];

/// Where a start-up stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupPhase {
    /// 1: the program is loaded.
    Load,
    /// 2: retained data is restored.
    Restore,
    /// 3: the start-up handlers run.
    Handler,
    /// 4: the devices come up, `up` of `of`.
    Devices { up: u32, of: u32 },
}

impl StartupPhase {
    pub fn number(&self) -> u8 {
        match self {
            StartupPhase::Load => 1,
            StartupPhase::Restore => 2,
            StartupPhase::Handler => 3,
            StartupPhase::Devices { .. } => 4,
        }
    }

    pub fn word(&self) -> &'static str {
        let n = self.number();
        PHASES.iter().find(|p| p.number == n).map_or("", |p| p.word)
    }
}

/// Why the controller is halted in HOLD.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HoldReason {
    /// 401: a breakpoint was hit, at a source location.
    Breakpoint { at: String },
    /// 402: paused on a command.
    Command { source: CommandSource },
    /// 403: a step ended.
    Step,
    /// 404: a single cycle ended.
    Cycle,
}

impl HoldReason {
    pub fn number(&self) -> u16 {
        match self {
            HoldReason::Breakpoint { .. } => 401,
            HoldReason::Command { .. } => 402,
            HoldReason::Step => 403,
            HoldReason::Cycle => 404,
        }
    }

    pub fn word(&self) -> &'static str {
        entry_word(self.number())
    }
}

/// Why the controller is in FAULT.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FaultReason {
    /// 501: a task's watchdog ran out.
    Watchdog { task: String },
    /// 502: a program error with no handler, at a source location.
    Program { at: String },
    /// 503: the program raised a fault with a code of its own.
    User { code: i64 },
    /// 504: a fault inside the handler of an event.
    Handler { event: String },
    /// 505: a resource ran out (call depth, memory).
    Resource { kind: String },
    /// 506: the application of a download could not be loaded.
    Load { download: u32 },
    /// 507: the runtime's own failure.
    Internal,
}

impl FaultReason {
    pub fn number(&self) -> u16 {
        match self {
            FaultReason::Watchdog { .. } => 501,
            FaultReason::Program { .. } => 502,
            FaultReason::User { .. } => 503,
            FaultReason::Handler { .. } => 504,
            FaultReason::Resource { .. } => 505,
            FaultReason::Load { .. } => 506,
            FaultReason::Internal => 507,
        }
    }

    pub fn word(&self) -> &'static str {
        entry_word(self.number())
    }
}

/// Layer 1 with what may stand beside it: a reason of its own state, and while starting the
/// phase. A reason of another state cannot be built.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum State {
    Unknown,
    Stop(Option<StopReason>),
    Startup {
        reason: Option<StartupReason>,
        phase: Option<StartupPhase>,
    },
    Run,
    Hold(Option<HoldReason>),
    Fault(Option<FaultReason>),
}

impl State {
    pub fn word(&self) -> StateWord {
        match self {
            State::Unknown => StateWord::Unknown,
            State::Stop(_) => StateWord::Stop,
            State::Startup { .. } => StateWord::Startup,
            State::Run => StateWord::Run,
            State::Hold(_) => StateWord::Hold,
            State::Fault(_) => StateWord::Fault,
        }
    }

    /// The reason's number, when there is a reason.
    pub fn reason_number(&self) -> Option<u16> {
        match self {
            State::Unknown | State::Run => None,
            State::Stop(r) => r.as_ref().map(StopReason::number),
            State::Startup { reason, .. } => reason.as_ref().map(StartupReason::number),
            State::Hold(r) => r.as_ref().map(HoldReason::number),
            State::Fault(r) => r.as_ref().map(FaultReason::number),
        }
    }
}

/// What is forced: inputs, outputs and variables, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Force {
    pub inputs: u32,
    pub outputs: u32,
    pub variables: u32,
}

impl Force {
    pub fn any(&self) -> bool {
        self.inputs > 0 || self.outputs > 0 || self.variables > 0
    }
}

/// A shadow test: the download numbers of the version driving the outputs and of the one
/// watching.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shadow {
    pub drives: u32,
    pub watches: u32,
}

/// Every condition that applies, whatever the state. A count of zero, `None` and `false` are
/// the condition absent, and the text then has no term for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Conditions {
    /// `breakpoint(set)`: breakpoints set.
    pub breakpoints_set: u32,
    /// `force(in, out, var)`.
    pub force: Force,
    /// `shadow(drives, watches)`.
    pub shadow: Option<Shadow>,
    /// `dry()`: the logic runs on live inputs, every output stays safe.
    pub dry: bool,
    /// `sim()`: the plant is a simulator.
    pub sim: bool,
    /// `no_realtime()`: running without real-time scheduling.
    pub no_realtime: bool,
    /// `devices(missing)`: devices configured and not present.
    pub devices_missing: u32,
    /// `overruns(count)`: scans that overran their period.
    pub overruns: u32,
    /// `retain(reset)`: retained variables reset, a count (ruled, Q100).
    pub retain_reset: u32,
}

impl Conditions {
    /// What someone did to the controller (breakpoints, forcing, a shadow test) or the mode it
    /// runs in (dry, simulated, without real time): the conditions that make a running
    /// controller's health a function check. The plan never listed which conditions are debug
    /// conditions; these six are the ruled reading (Q100).
    pub fn debug(&self) -> bool {
        self.breakpoints_set > 0
            || self.force.any()
            || self.shadow.is_some()
            || self.dry
            || self.sim
            || self.no_realtime
    }
}

/// One row of the table of maintenance reasons: its number, its word, and its urgency, fixed or
/// (`None`) as the device or the program states it. `urgent = false` is PROFINET's maintenance
/// required, `urgent = true` its maintenance demanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaintenanceEntry {
    pub number: u8,
    pub word: &'static str,
    pub urgent: Option<bool>,
}

/// The table of maintenance reasons.
pub const MAINTENANCE_REASONS: [MaintenanceEntry; 6] = [
    MaintenanceEntry {
        number: 1,
        word: "fault_on_update",
        urgent: Some(true),
    },
    MaintenanceEntry {
        number: 2,
        word: "device",
        urgent: None,
    },
    MaintenanceEntry {
        number: 3,
        word: "device_missing",
        urgent: Some(true),
    },
    MaintenanceEntry {
        number: 4,
        word: "overruns",
        urgent: Some(false),
    },
    MaintenanceEntry {
        number: 5,
        word: "retain_reset",
        urgent: Some(false),
    },
    MaintenanceEntry {
        number: 6,
        word: "user",
        urgent: None,
    },
];

/// One maintenance request. A reason whose urgency is fixed carries none; a request of another
/// urgency cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MaintenanceRequest {
    /// 1: an edit fell back to the previous version (urgent).
    FaultOnUpdate,
    /// 2: as a device reports it.
    Device { urgent: bool },
    /// 3: a configured device is missing (urgent).
    DeviceMissing,
    /// 4: scans overran (not urgent).
    Overruns,
    /// 5: retained data was reset (not urgent).
    RetainReset,
    /// 6: as the program states it.
    User { urgent: bool },
}

impl MaintenanceRequest {
    pub fn number(&self) -> u8 {
        match self {
            MaintenanceRequest::FaultOnUpdate => 1,
            MaintenanceRequest::Device { .. } => 2,
            MaintenanceRequest::DeviceMissing => 3,
            MaintenanceRequest::Overruns => 4,
            MaintenanceRequest::RetainReset => 5,
            MaintenanceRequest::User { .. } => 6,
        }
    }

    pub fn word(&self) -> &'static str {
        let n = self.number();
        MAINTENANCE_REASONS
            .iter()
            .find(|m| m.number == n)
            .map_or("", |m| m.word)
    }

    pub fn urgent(&self) -> bool {
        match self {
            MaintenanceRequest::FaultOnUpdate | MaintenanceRequest::DeviceMissing => true,
            MaintenanceRequest::Overruns | MaintenanceRequest::RetainReset => false,
            MaintenanceRequest::Device { urgent } | MaintenanceRequest::User { urgent } => *urgent,
        }
    }
}

/// The standard health value (NE 107; OPC UA's DeviceHealth), derived from the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Health {
    Normal,
    Failure,
    FunctionCheck,
    MaintenanceRequired,
}

impl Health {
    /// The value of OPC UA's `DeviceHealthEnumeration`: NORMAL 0, FAILURE 1, CHECK_FUNCTION 2,
    /// MAINTENANCE_REQUIRED 4 (3 is off-spec, which the state never gives).
    pub fn device_health(self) -> u32 {
        match self {
            Health::Normal => 0,
            Health::Failure => 1,
            Health::FunctionCheck => 2,
            Health::MaintenanceRequired => 4,
        }
    }
}

impl fmt::Display for Health {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Health::Normal => "normal",
            Health::Failure => "failure",
            Health::FunctionCheck => "function check",
            Health::MaintenanceRequired => "maintenance required",
        })
    }
}

/// A controller's whole state: layer 1 with its reason and phase, the conditions, and the
/// maintenance requests, which the text holds in the table's order (by number, a request that
/// is not urgent before one that is) whatever order the list holds them in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ControllerState {
    pub state: State,
    pub conditions: Conditions,
    pub maintenance: Vec<MaintenanceRequest>,
}

impl ControllerState {
    /// A state with no condition and no maintenance request.
    pub fn new(state: State) -> ControllerState {
        ControllerState {
            state,
            conditions: Conditions::default(),
            maintenance: Vec::new(),
        }
    }

    /// The same state with its maintenance requests in the text's order: what reading its text
    /// gives back.
    pub fn normalized(mut self) -> ControllerState {
        self.maintenance.sort();
        self
    }

    /// FAULT gives failure; STOP, STARTUP, HOLD and any debug condition give function check; a
    /// maintenance request gives maintenance required; a clean RUN gives normal. An unknown
    /// state has no health, as ruled (Q100).
    pub fn health(&self) -> Option<Health> {
        match self.state.word() {
            StateWord::Unknown => None,
            StateWord::Fault => Some(Health::Failure),
            StateWord::Stop | StateWord::Startup | StateWord::Hold => Some(Health::FunctionCheck),
            StateWord::Run if self.conditions.debug() => Some(Health::FunctionCheck),
            StateWord::Run if !self.maintenance.is_empty() => Some(Health::MaintenanceRequired),
            StateWord::Run => Some(Health::Normal),
        }
    }

    /// The text's terms after the word, in the text's order.
    fn terms(&self) -> Vec<Term> {
        let mut terms = Vec::new();
        let id = |n: u16| ("id", Arg::Int(i64::from(n)));
        let source = |s: &CommandSource| ("source", Arg::Str(s.word().to_string()));
        match &self.state {
            State::Unknown | State::Run => {}
            State::Stop(r) => {
                if let Some(r) = r {
                    let mut args = vec![id(r.number())];
                    match r {
                        StopReason::Command { source: s } => args.push(source(s)),
                        StopReason::PowerOn { was } => {
                            args.push(("was", Arg::Str(was.word().to_string())))
                        }
                        StopReason::Acknowledged
                        | StopReason::NoProgram
                        | StopReason::RealtimeRefused => {}
                    }
                    terms.push(Term::new(r.word(), args));
                }
            }
            State::Startup { reason, phase } => {
                if let Some(r) = reason {
                    terms.push(Term::new(r.word(), vec![id(r.number())]));
                }
                if let Some(p) = phase {
                    let args = match p {
                        StartupPhase::Devices { up, of } => {
                            vec![
                                ("up", Arg::Int(i64::from(*up))),
                                ("of", Arg::Int(i64::from(*of))),
                            ]
                        }
                        StartupPhase::Load | StartupPhase::Restore | StartupPhase::Handler => {
                            vec![]
                        }
                    };
                    terms.push(Term::new(p.word(), args));
                }
            }
            State::Hold(r) => {
                if let Some(r) = r {
                    let mut args = vec![id(r.number())];
                    match r {
                        HoldReason::Breakpoint { at } => args.push(("at", Arg::Str(at.clone()))),
                        HoldReason::Command { source: s } => args.push(source(s)),
                        HoldReason::Step | HoldReason::Cycle => {}
                    }
                    terms.push(Term::new(r.word(), args));
                }
            }
            State::Fault(r) => {
                if let Some(r) = r {
                    let mut args = vec![id(r.number())];
                    match r {
                        FaultReason::Watchdog { task } => {
                            args.push(("task", Arg::Str(task.clone())))
                        }
                        FaultReason::Program { at } => args.push(("at", Arg::Str(at.clone()))),
                        FaultReason::User { code } => args.push(("code", Arg::Int(*code))),
                        FaultReason::Handler { event } => {
                            args.push(("event", Arg::Str(event.clone())))
                        }
                        FaultReason::Resource { kind } => {
                            args.push(("kind", Arg::Str(kind.clone())))
                        }
                        FaultReason::Load { download } => {
                            args.push(("download", Arg::Int(i64::from(*download))))
                        }
                        FaultReason::Internal => {}
                    }
                    terms.push(Term::new(r.word(), args));
                }
            }
        }
        let c = &self.conditions;
        let count = |n: u32| Arg::Int(i64::from(n));
        // Ruled (Q100, point 4): written beside a breakpoint reason too, the two told apart by
        // their arguments.
        if c.breakpoints_set > 0 {
            terms.push(Term::new(
                "breakpoint",
                vec![("set", count(c.breakpoints_set))],
            ));
        }
        if c.force.any() {
            terms.push(Term::new(
                "force",
                vec![
                    ("in", count(c.force.inputs)),
                    ("out", count(c.force.outputs)),
                    ("var", count(c.force.variables)),
                ],
            ));
        }
        if let Some(s) = c.shadow {
            terms.push(Term::new(
                "shadow",
                vec![("drives", count(s.drives)), ("watches", count(s.watches))],
            ));
        }
        for (on, word) in [
            (c.dry, "dry"),
            (c.sim, "sim"),
            (c.no_realtime, "no_realtime"),
        ] {
            if on {
                terms.push(Term::new(word, vec![]));
            }
        }
        if c.devices_missing > 0 {
            terms.push(Term::new(
                "devices",
                vec![("missing", count(c.devices_missing))],
            ));
        }
        if c.overruns > 0 {
            terms.push(Term::new("overruns", vec![("count", count(c.overruns))]));
        }
        if c.retain_reset > 0 {
            terms.push(Term::new("retain", vec![("reset", count(c.retain_reset))]));
        }
        let mut requests = self.maintenance.clone();
        // Ruled (Q100, point 6): by number, one not urgent before an urgent one.
        requests.sort();
        for m in requests {
            terms.push(Term::new(
                "maintenance_request",
                vec![
                    ("id", Arg::Int(i64::from(m.number()))),
                    ("reason", Arg::Str(m.word().to_string())),
                    ("urgent", Arg::Bool(m.urgent())),
                ],
            ));
        }
        terms
    }
}

/// The writer: the one spelling of this state.
impl fmt::Display for ControllerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.state.word().word())?;
        for term in self.terms() {
            f.write_str(" ")?;
            f.write_str(&term.name)?;
            f.write_str("(")?;
            for (i, (key, value)) in term.args.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(key)?;
                f.write_str("=")?;
                match value {
                    Arg::Int(n) => write!(f, "{n}")?,
                    Arg::Bool(b) => write!(f, "{b}")?,
                    Arg::Str(s) => write_string(f, s)?,
                }
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// The reader: exactly the texts the writer writes.
impl FromStr for ControllerState {
    type Err = StateTextError;

    fn from_str(text: &str) -> Result<ControllerState, StateTextError> {
        let (word, terms) = lex(text)?;
        assemble(word, terms)
    }
}

/// One argument's value as the text spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Arg {
    Int(i64),
    Bool(bool),
    Str(String),
}

/// One term as the text spells it, and where it starts (a byte offset into the text).
#[derive(Clone, Debug)]
struct Term {
    name: String,
    args: Vec<(String, Arg)>,
    at: usize,
}

impl Term {
    fn new(name: &str, args: Vec<(&str, Arg)>) -> Term {
        Term {
            name: name.to_string(),
            args: args.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            at: 0,
        }
    }

    fn keys(&self) -> Vec<&str> {
        self.args.iter().map(|(k, _)| k.as_str()).collect()
    }
}

/// Why a text was refused. `at` is a byte offset into the text; a term is named as the text
/// spells its name.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StateTextError {
    #[error("an empty text: a state text starts with its word")]
    Empty,
    #[error("`{word}` at {at} is no state word (UNKNOWN, STOP, STARTUP, RUN, HOLD, FAULT)")]
    UnknownWord { word: String, at: usize },
    #[error("expected {expected} at {at}, found {found}")]
    Expected {
        expected: &'static str,
        found: String,
        at: usize,
    },
    #[error("an integer at {at} is not written as the writer writes it: {reason}")]
    Integer { reason: &'static str, at: usize },
    #[error("a string at {at} is not written as the writer writes it: {reason}")]
    String { reason: String, at: usize },
    #[error("`{term}` at {at} is no term of the state text")]
    UnknownTerm { term: String, at: usize },
    #[error("`{term}` at {at} is no term of {state}")]
    NotInState {
        term: String,
        state: StateWord,
        at: usize,
    },
    #[error("`{term}` at {at} takes ({expected}), not ({found})")]
    Arguments {
        term: String,
        expected: String,
        found: String,
        at: usize,
    },
    #[error("`{term}` at {at} has `{key}` of the wrong type: it is {expected}")]
    Type {
        term: String,
        key: String,
        expected: &'static str,
        at: usize,
    },
    #[error("`{term}` at {at} has `{key}={value}`, out of its range {range}")]
    Range {
        term: String,
        key: String,
        value: i64,
        range: &'static str,
        at: usize,
    },
    #[error("`{term}` at {at} has id={found}, where its number is {expected}")]
    Id {
        term: String,
        expected: u16,
        found: i64,
        at: usize,
    },
    #[error("`{term}` at {at} has `{key}=\"{value}\"`, which is not one of {known}")]
    Word {
        term: String,
        key: String,
        value: String,
        known: &'static str,
        at: usize,
    },
    #[error("`{term}` at {at} has {key}={value}, which is not its table's {expected}")]
    Table {
        term: String,
        key: &'static str,
        value: String,
        expected: String,
        at: usize,
    },
    #[error("`{term}` at {at} stands after `{after}`: terms stand in a fixed order")]
    Order {
        term: String,
        after: String,
        at: usize,
    },
    #[error(
        "`maintenance_request` at {at} stands after one later in the table: requests stand by number, one not urgent before one urgent"
    )]
    MaintenanceOrder { at: usize },
    #[error("`{term}` at {at} counts nothing: the writer leaves out a condition that is absent")]
    Absent { term: String, at: usize },
}

/// A character the writer spells as `\u` or `\U` rather than as itself: a control character,
/// a line or paragraph separator, or a format character (invisible ones and the marks that
/// reorder a line), so the text stays one line and shows on every screen as the characters it
/// holds.
fn escaped_by_number(c: char) -> bool {
    let n = u32::from(c);
    if c.is_control() {
        return !matches!(c, '\u{8}' | '\t' | '\n' | '\u{c}' | '\r');
    }
    matches!(
        n,
        0x2028 | 0x2029 // line and paragraph separators
        // Unicode's format characters (general category Cf).
        | 0x00AD
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x0890..=0x0891
        | 0x08E2
        | 0x180E
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0x110BD
        | 0x110CD
        | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A
        | 0xE0001
        | 0xE0020..=0xE007F
    )
}

fn named_escape(c: char) -> Option<char> {
    match c {
        '"' => Some('"'),
        '\\' => Some('\\'),
        '\u{8}' => Some('b'),
        '\t' => Some('t'),
        '\n' => Some('n'),
        '\u{c}' => Some('f'),
        '\r' => Some('r'),
        _ => None,
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        if let Some(e) = named_escape(c) {
            write!(f, "\\{e}")?;
        } else if escaped_by_number(c) {
            let n = u32::from(c);
            if n <= 0xFFFF {
                write!(f, "\\u{n:04X}")?;
            } else {
                write!(f, "\\U{n:08X}")?;
            }
        } else {
            write!(f, "{c}")?;
        }
    }
    f.write_str("\"")
}

/// The text cut into its word and its terms, every spelling checked against the writer's.
struct Lexer<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn found(&self) -> String {
        match self.peek() {
            None => "the end of the text".to_string(),
            Some(c) => format!("{c:?}"),
        }
    }

    fn expect(
        &mut self,
        literal: &'static str,
        expected: &'static str,
    ) -> Result<(), StateTextError> {
        if self.text[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(())
        } else {
            Err(StateTextError::Expected {
                expected,
                found: self.found(),
                at: self.pos,
            })
        }
    }

    /// A run of the characters `ok` admits, possibly empty.
    fn run(&mut self, ok: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if !ok(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        &self.text[start..self.pos]
    }

    fn value(&mut self) -> Result<Arg, StateTextError> {
        let at = self.pos;
        match self.peek() {
            Some('"') => self.string().map(Arg::Str),
            Some('t' | 'f') => {
                let word = self.run(|c| c.is_ascii_lowercase());
                match word {
                    "true" => Ok(Arg::Bool(true)),
                    "false" => Ok(Arg::Bool(false)),
                    _ => Err(StateTextError::Expected {
                        expected: "a value (an integer, true, false or a quoted string)",
                        found: format!("{word:?}"),
                        at,
                    }),
                }
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.integer(),
            _ => Err(StateTextError::Expected {
                expected: "a value (an integer, true, false or a quoted string)",
                found: self.found(),
                at,
            }),
        }
    }

    /// `0`, or an optional `-` and digits with no leading zero, within TOML's 64 bits.
    fn integer(&mut self) -> Result<Arg, StateTextError> {
        let at = self.pos;
        let negative = self.peek() == Some('-');
        if negative {
            self.pos += 1;
        }
        let digits = self.run(|c| c.is_ascii_digit());
        let err = |reason| StateTextError::Integer { reason, at };
        if digits.is_empty() {
            return Err(err("a sign with no digits"));
        }
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(err("a leading zero"));
        }
        if negative && digits == "0" {
            return Err(err("zero is written 0"));
        }
        if matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            return Err(err(
                "only decimal digits, no prefix, separator, exponent or fraction",
            ));
        }
        let spelled = &self.text[at..self.pos];
        spelled
            .parse::<i64>()
            .map(Arg::Int)
            .map_err(|_| err("beyond 64 bits"))
    }

    fn string(&mut self) -> Result<String, StateTextError> {
        let at = self.pos;
        self.pos += 1; // the opening quote
        let mut out = String::new();
        loop {
            let here = self.pos;
            let err = |reason: String| StateTextError::String { reason, at: here };
            let Some(c) = self.peek() else {
                return Err(StateTextError::String {
                    reason: "no closing quote".to_string(),
                    at,
                });
            };
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(e) = self.peek() else {
                        return Err(err("an escape cut short".to_string()));
                    };
                    self.pos += e.len_utf8();
                    let decoded = match e {
                        '"' => '"',
                        '\\' => '\\',
                        'b' => '\u{8}',
                        't' => '\t',
                        'n' => '\n',
                        'f' => '\u{c}',
                        'r' => '\r',
                        'u' | 'U' => {
                            let width = if e == 'u' { 4 } else { 8 };
                            let hex = self.text.get(self.pos..self.pos + width).unwrap_or("");
                            if hex.len() != width
                                || !hex
                                    .chars()
                                    .all(|h| h.is_ascii_digit() || ('A'..='F').contains(&h))
                            {
                                return Err(err(format!(
                                    "`\\{e}` takes {width} uppercase hexadecimal digits"
                                )));
                            }
                            self.pos += width;
                            let decoded = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| err(format!("`\\{e}{hex}` is no character")))?;
                            let canonical_width = if u32::from(decoded) <= 0xFFFF {
                                'u'
                            } else {
                                'U'
                            };
                            if !escaped_by_number(decoded) || named_escape(decoded).is_some() {
                                return Err(err(format!(
                                    "`\\{e}{hex}` spells a character the writer writes as itself or by its short escape"
                                )));
                            }
                            if e != canonical_width {
                                return Err(err(format!(
                                    "`\\{e}{hex}` is written `\\{canonical_width}` with {} digits",
                                    if canonical_width == 'u' { 4 } else { 8 }
                                )));
                            }
                            decoded
                        }
                        other => {
                            return Err(err(format!("`\\{other}` is no escape of the text")));
                        }
                    };
                    out.push(decoded);
                }
                c if escaped_by_number(c) || named_escape(c).is_some() => {
                    return Err(err(format!("U+{:04X} stands unescaped", u32::from(c))));
                }
                c => out.push(c),
            }
        }
    }

    fn term(&mut self) -> Result<Term, StateTextError> {
        let at = self.pos;
        let name = self.run(|c| c.is_ascii_lowercase() || c == '_');
        if name.is_empty() || name.starts_with('_') {
            return Err(StateTextError::Expected {
                expected: "a term's name (lowercase letters and underscores)",
                found: self.found(),
                at: self.pos,
            });
        }
        self.expect("(", "`(` after a term's name")?;
        let mut args = Vec::new();
        if self.peek() == Some(')') {
            self.pos += 1;
            return Ok(Term {
                name: name.to_string(),
                args,
                at,
            });
        }
        loop {
            let key = self.run(|c| c.is_ascii_lowercase());
            if key.is_empty() {
                return Err(StateTextError::Expected {
                    expected: "an argument's name (lowercase letters)",
                    found: self.found(),
                    at: self.pos,
                });
            }
            self.expect("=", "`=` after an argument's name, with no space")?;
            let value = self.value()?;
            args.push((key.to_string(), value));
            match self.peek() {
                Some(')') => {
                    self.pos += 1;
                    return Ok(Term {
                        name: name.to_string(),
                        args,
                        at,
                    });
                }
                _ => self.expect(", ", "`, ` between arguments, or `)`")?,
            }
        }
    }
}

fn lex(text: &str) -> Result<(StateWord, Vec<Term>), StateTextError> {
    if text.is_empty() {
        return Err(StateTextError::Empty);
    }
    let mut lx = Lexer { text, pos: 0 };
    let word_text = lx.run(|c| c.is_ascii_uppercase());
    if word_text.is_empty() {
        return Err(StateTextError::Expected {
            expected: "a state word",
            found: lx.found(),
            at: 0,
        });
    }
    let word = StateWord::from_word(word_text).ok_or_else(|| StateTextError::UnknownWord {
        word: word_text.to_string(),
        at: 0,
    })?;
    let mut terms = Vec::new();
    while lx.pos < text.len() {
        lx.expect(" ", "one space before a term, or the end of the text")?;
        terms.push(lx.term()?);
    }
    Ok((word, terms))
}

/// The places a term can take, in the text's order. Every place holds one term at most, but
/// the maintenance requests, which repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Reason,
    Phase,
    Breakpoints,
    Force,
    Shadow,
    Dry,
    Sim,
    NoRealtime,
    DevicesMissing,
    Overruns,
    Retain,
    Maintenance,
}

/// The conditions and the maintenance request: name, arguments, place.
const CONDITION_TERMS: [(&str, &[&str], Place); 10] = [
    ("breakpoint", &["set"], Place::Breakpoints),
    ("force", &["in", "out", "var"], Place::Force),
    ("shadow", &["drives", "watches"], Place::Shadow),
    ("dry", &[], Place::Dry),
    ("sim", &[], Place::Sim),
    ("no_realtime", &[], Place::NoRealtime),
    ("devices", &["missing"], Place::DevicesMissing),
    ("overruns", &["count"], Place::Overruns),
    ("retain", &["reset"], Place::Retain),
    (
        "maintenance_request",
        &["id", "reason", "urgent"],
        Place::Maintenance,
    ),
];

/// What a term is, judged by its name, its arguments' names and the state it stands in.
enum Kind {
    Reason(&'static ReasonEntry),
    Phase(&'static PhaseEntry),
    Condition(Place),
}

fn join(keys: &[&str]) -> String {
    keys.join(", ")
}

fn classify(word: StateWord, term: &Term) -> Result<(Kind, Place), StateTextError> {
    let keys = term.keys();
    // The argument lists this name takes where it stands, for the refusal's text.
    let mut expected: Vec<String> = Vec::new();
    // The term is a reason of another state, spelled as one (its `id` and arguments).
    let mut elsewhere = false;
    for r in REASONS.iter().filter(|r| r.word == term.name) {
        let mut wanted = vec!["id"];
        wanted.extend_from_slice(r.args);
        if r.state() == Some(word) {
            if keys == wanted {
                return Ok((Kind::Reason(r), Place::Reason));
            }
            expected.push(join(&wanted));
        } else if keys == wanted {
            elsewhere = true;
        }
    }
    for p in PHASES.iter().filter(|p| p.word == term.name) {
        if word == StateWord::Startup {
            if keys == p.args {
                return Ok((Kind::Phase(p), Place::Phase));
            }
            expected.push(join(p.args));
        }
    }
    for (_, args, place) in CONDITION_TERMS.iter().filter(|(n, _, _)| *n == term.name) {
        if keys == *args {
            return Ok((Kind::Condition(*place), *place));
        }
        expected.push(join(args));
    }
    let named_elsewhere =
        REASONS.iter().any(|r| r.word == term.name) || PHASES.iter().any(|p| p.word == term.name);
    // A reason of another state is named as such; a name this state also uses is refused for
    // its arguments; a name only another state uses is named as such.
    if elsewhere || (expected.is_empty() && named_elsewhere) {
        return Err(StateTextError::NotInState {
            term: term.name.clone(),
            state: word,
            at: term.at,
        });
    }
    if !expected.is_empty() {
        return Err(StateTextError::Arguments {
            term: term.name.clone(),
            expected: expected.join(") or ("),
            found: join(&keys),
            at: term.at,
        });
    }
    // Ruled (Q100, point 7): no version, and a term not in the table is refused.
    Err(StateTextError::UnknownTerm {
        term: term.name.clone(),
        at: term.at,
    })
}

/// A term's arguments, read by name in the order `classify` checked.
struct Args<'t> {
    term: &'t Term,
}

impl<'t> Args<'t> {
    fn get(&self, key: &str) -> &'t Arg {
        // `classify` matched the keys exactly, so the key is there.
        &self.term.args[self.term.keys().iter().position(|k| *k == key).unwrap_or(0)].1
    }

    fn int(&self, key: &str) -> Result<i64, StateTextError> {
        match self.get(key) {
            Arg::Int(n) => Ok(*n),
            _ => Err(self.type_error(key, "a whole number")),
        }
    }

    fn count(&self, key: &str) -> Result<u32, StateTextError> {
        let n = self.int(key)?;
        u32::try_from(n).map_err(|_| StateTextError::Range {
            term: self.term.name.clone(),
            key: key.to_string(),
            value: n,
            range: "0 to 4294967295",
            at: self.term.at,
        })
    }

    fn boolean(&self, key: &str) -> Result<bool, StateTextError> {
        match self.get(key) {
            Arg::Bool(b) => Ok(*b),
            _ => Err(self.type_error(key, "true or false")),
        }
    }

    fn string(&self, key: &str) -> Result<String, StateTextError> {
        match self.get(key) {
            Arg::Str(s) => Ok(s.clone()),
            _ => Err(self.type_error(key, "a quoted string")),
        }
    }

    fn type_error(&self, key: &str, expected: &'static str) -> StateTextError {
        StateTextError::Type {
            term: self.term.name.clone(),
            key: key.to_string(),
            expected,
            at: self.term.at,
        }
    }

    fn id(&self, number: u16) -> Result<(), StateTextError> {
        let found = self.int("id")?;
        if found == i64::from(number) {
            Ok(())
        } else {
            Err(StateTextError::Id {
                term: self.term.name.clone(),
                expected: number,
                found,
                at: self.term.at,
            })
        }
    }

    fn source(&self) -> Result<CommandSource, StateTextError> {
        let s = self.string("source")?;
        CommandSource::from_word(&s).ok_or_else(|| StateTextError::Word {
            term: self.term.name.clone(),
            key: "source".to_string(),
            value: s,
            known: "oficina, local, program",
            at: self.term.at,
        })
    }

    fn state_word(&self, key: &str) -> Result<StateWord, StateTextError> {
        let s = self.string(key)?;
        StateWord::from_word(&s).ok_or_else(|| StateTextError::Word {
            term: self.term.name.clone(),
            key: key.to_string(),
            value: s,
            known: "UNKNOWN, STOP, STARTUP, RUN, HOLD, FAULT",
            at: self.term.at,
        })
    }
}

fn assemble(word: StateWord, terms: Vec<Term>) -> Result<ControllerState, StateTextError> {
    let mut reason: Option<(&'static ReasonEntry, &Term)> = None;
    let mut phase: Option<StartupPhase> = None;
    let mut c = Conditions::default();
    let mut maintenance = Vec::new();
    let mut last: Option<(Place, &Term)> = None;
    for term in &terms {
        let (kind, place) = classify(word, term)?;
        if let Some((before, prev)) = last {
            let repeats = place == Place::Maintenance && before == Place::Maintenance;
            if place < before || (place == before && !repeats) {
                return Err(StateTextError::Order {
                    term: term.name.clone(),
                    after: prev.name.clone(),
                    at: term.at,
                });
            }
        }
        last = Some((place, term));
        let a = Args { term };
        let absent = || StateTextError::Absent {
            term: term.name.clone(),
            at: term.at,
        };
        match kind {
            Kind::Reason(r) => {
                a.id(r.number)?;
                reason = Some((r, term));
            }
            Kind::Phase(p) => {
                phase = Some(match p.number {
                    1 => StartupPhase::Load,
                    2 => StartupPhase::Restore,
                    3 => StartupPhase::Handler,
                    _ => StartupPhase::Devices {
                        up: a.count("up")?,
                        of: a.count("of")?,
                    },
                });
            }
            Kind::Condition(Place::Breakpoints) => {
                c.breakpoints_set = a.count("set")?;
                if c.breakpoints_set == 0 {
                    return Err(absent());
                }
            }
            Kind::Condition(Place::Force) => {
                c.force = Force {
                    inputs: a.count("in")?,
                    outputs: a.count("out")?,
                    variables: a.count("var")?,
                };
                if !c.force.any() {
                    return Err(absent());
                }
            }
            Kind::Condition(Place::Shadow) => {
                c.shadow = Some(Shadow {
                    drives: a.count("drives")?,
                    watches: a.count("watches")?,
                });
            }
            Kind::Condition(Place::Dry) => c.dry = true,
            Kind::Condition(Place::Sim) => c.sim = true,
            Kind::Condition(Place::NoRealtime) => c.no_realtime = true,
            Kind::Condition(Place::DevicesMissing) => {
                c.devices_missing = a.count("missing")?;
                if c.devices_missing == 0 {
                    return Err(absent());
                }
            }
            Kind::Condition(Place::Overruns) => {
                c.overruns = a.count("count")?;
                if c.overruns == 0 {
                    return Err(absent());
                }
            }
            Kind::Condition(Place::Retain) => {
                c.retain_reset = a.count("reset")?;
                if c.retain_reset == 0 {
                    return Err(absent());
                }
            }
            Kind::Condition(Place::Maintenance) => {
                let m = maintenance_request(&a)?;
                if maintenance.last().is_some_and(|prev| m < *prev) {
                    return Err(StateTextError::MaintenanceOrder { at: term.at });
                }
                maintenance.push(m);
            }
            Kind::Condition(Place::Reason | Place::Phase) => {}
        }
    }
    let state = match word {
        StateWord::Unknown => State::Unknown,
        StateWord::Run => State::Run,
        StateWord::Stop => State::Stop(match reason {
            None => None,
            Some((r, t)) => {
                let a = Args { term: t };
                Some(match r.number {
                    101 => StopReason::Command {
                        source: a.source()?,
                    },
                    102 => StopReason::PowerOn {
                        was: a.state_word("was")?,
                    },
                    103 => StopReason::Acknowledged,
                    104 => StopReason::NoProgram,
                    _ => StopReason::RealtimeRefused,
                })
            }
        }),
        StateWord::Startup => State::Startup {
            reason: reason.map(|(r, _)| {
                if r.number == 201 {
                    StartupReason::Warm
                } else {
                    StartupReason::Cold
                }
            }),
            phase,
        },
        StateWord::Hold => State::Hold(match reason {
            None => None,
            Some((r, t)) => {
                let a = Args { term: t };
                Some(match r.number {
                    401 => HoldReason::Breakpoint {
                        at: a.string("at")?,
                    },
                    402 => HoldReason::Command {
                        source: a.source()?,
                    },
                    403 => HoldReason::Step,
                    _ => HoldReason::Cycle,
                })
            }
        }),
        StateWord::Fault => State::Fault(match reason {
            None => None,
            Some((r, t)) => {
                let a = Args { term: t };
                Some(match r.number {
                    501 => FaultReason::Watchdog {
                        task: a.string("task")?,
                    },
                    502 => FaultReason::Program {
                        at: a.string("at")?,
                    },
                    503 => FaultReason::User {
                        code: a.int("code")?,
                    },
                    504 => FaultReason::Handler {
                        event: a.string("event")?,
                    },
                    505 => FaultReason::Resource {
                        kind: a.string("kind")?,
                    },
                    506 => FaultReason::Load {
                        download: a.count("download")?,
                    },
                    _ => FaultReason::Internal,
                })
            }
        }),
    };
    Ok(ControllerState {
        state,
        conditions: c,
        maintenance,
    })
}

fn maintenance_request(a: &Args) -> Result<MaintenanceRequest, StateTextError> {
    let id = a.int("id")?;
    let word = a.string("reason")?;
    let urgent = a.boolean("urgent")?;
    let table = |key: &'static str, value: String, expected: String| StateTextError::Table {
        term: a.term.name.clone(),
        key,
        value,
        expected,
        at: a.term.at,
    };
    let entry = MAINTENANCE_REASONS
        .iter()
        .find(|m| i64::from(m.number) == id)
        .ok_or_else(|| table("id", id.to_string(), "1 to 6".to_string()))?;
    if entry.word != word {
        return Err(table(
            "reason",
            format!("{word:?}"),
            format!("{:?} for id={id}", entry.word),
        ));
    }
    if let Some(fixed) = entry.urgent
        && fixed != urgent
    {
        return Err(table(
            "urgent",
            urgent.to_string(),
            format!("{fixed} for {:?}", entry.word),
        ));
    }
    Ok(match entry.number {
        1 => MaintenanceRequest::FaultOnUpdate,
        2 => MaintenanceRequest::Device { urgent },
        3 => MaintenanceRequest::DeviceMissing,
        4 => MaintenanceRequest::Overruns,
        5 => MaintenanceRequest::RetainReset,
        _ => MaintenanceRequest::User { urgent },
    })
}
