//! An interpreted program on the controller (C-152): a program lowered by hand as rustc's MIR has it, made at
//! power-on and afresh at every start from what its host hands in, run beside its native twin; one that cannot be
//! made the load fault 506, its own error where Rust would panic the program's 502 `program(at)`, and what safe Rust
//! cannot reach or a failure of the adapter or the host the runtime's 507 `internal`.
use std::time::Duration;

use orcl_interp::{
    BinOp, Block, BlockId, Check, Const, Field, Function, FunctionId, Int, Local, LocalDecl, Operand, Place,
    Program as Code, Projection, Scalar, Span, Statement, Terminator, Type, TypeId, Value,
};
use orcl_runtime::{Clock, Image, Interpreted, Io, MakeError, Regions, TaskConfig, TaskError, TaskProgram, VirtualClock};
use orcl_std::{Program, Scan};
use veredas_controlador::{Command, Controller, Loaded, Refusal, Restart, Source, Switch, Target};
use veredas_core::state::{CommandSource, StartupReason, StateWord};

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

// The program, natively: it counts its scans from a first count, and writes the count times a hundred plus its
// input's value divided by its divisor (the arithmetic the wrapping one, as the lowered program's `Add` and `Mul`).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct In {
    value: u32,
    divisor: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Out {
    value: u32,
}

/// The count the interpreted program's first state holds, which a native default does not give.
const FIRST: u32 = 40;
/// The safe outputs, which no scan gives.
const SAFE: Out = Out { value: 7 };
/// An input whose image the machine refuses: its value written as what is no `u32`.
const REFUSED: u32 = u32::MAX;
/// An output the adapter cannot read back.
const UNREADABLE: u32 = 4_242;

/// Its first count is the interpreted program's first state, so the two count alike.
struct Count {
    scans: u32,
}

impl Default for Count {
    fn default() -> Self {
        Count { scans: FIRST }
    }
}

impl Program for Count {
    type Inputs = In;
    type Outputs = Out;

    fn scan(&mut self, _scan: &Scan, inputs: &In, outputs: &mut Out) {
        self.scans = self.scans.wrapping_add(1);
        outputs.value = self.scans.wrapping_mul(100).wrapping_add(inputs.value / inputs.divisor);
    }

    fn safe_outputs(&self) -> Out {
        SAFE
    }
}

fn u32_of(s: &Scalar) -> Option<u32> {
    match *s {
        Scalar::Int(Int::U32, v) => u32::try_from(v).ok(),
        _ => None,
    }
}

impl Image for In {
    fn write(&self, slots: &mut Vec<Scalar>) {
        let value = if self.value == REFUSED { Scalar::Bool(true) } else { Scalar::Int(Int::U32, self.value.into()) };
        slots.extend([value, Scalar::Int(Int::U32, self.divisor.into())]);
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        match slots {
            [value, divisor] => Some(In { value: u32_of(value)?, divisor: u32_of(divisor)? }),
            _ => None,
        }
    }
}

impl Image for Out {
    fn write(&self, slots: &mut Vec<Scalar>) {
        slots.push(Scalar::Int(Int::U32, self.value.into()));
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        match slots {
            [value] => u32_of(value).filter(|v| *v != UNREADABLE).map(|value| Out { value }),
            _ => None,
        }
    }
}

// The program, lowered by hand as rustc's MIR has it at opt-level 0, in `count.rs`:
//
// fn scan(_1: &mut Count, _2: &Scan, _3: &In, _4: &mut Out) {
//     bb0: (*_1).0 = Add(copy (*_1).0, const 1_u32);                              line 1
//          _5 = copy (*_3).1;   _6 = Eq(copy _5, const 0_u32);                      line 2
//          assert(!move _6, DivisionByZero) -> bb1                                  line 2
//     bb1: _7 = Div(copy (*_3).0, move _5);                                         line 2
//          _8 = Mul(copy (*_1).0, const 100_u32);   (*_4).0 = Add(move _8, move _7)  line 3
//          return                                                                   line 4
// }

const U32: TypeId = TypeId(0);
const U64: TypeId = TypeId(1);
const DURATION: TypeId = TypeId(2);
const BOOL: TypeId = TypeId(3);
const UNIT: TypeId = TypeId(4);
const COUNT: TypeId = TypeId(5);
const SCAN: TypeId = TypeId(6);
const IN: TypeId = TypeId(7);
const OUT: TypeId = TypeId(8);

/// A location in `count.rs`, the second of the program's files, so a fault names the file its location names.
fn at(line: u32) -> Span {
    Span { file: 1, line, column: 5 }
}

fn field(name: &str, ty: TypeId) -> Field {
    Field { name: name.into(), ty }
}

fn deref(local: u32, fields: &[u32]) -> Place {
    let mut projections = vec![Projection::Deref];
    projections.extend(fields.iter().map(|f| Projection::Field(*f)));
    Place { local: orcl_interp::Local(local), projections }
}

fn copy(place: Place) -> Operand {
    Operand::Copy(place)
}

fn moved(local: u32) -> Operand {
    Operand::Move(Place::local(local))
}

fn u32_const(v: u32) -> Operand {
    Operand::Const(Const { ty: U32, slots: vec![Scalar::Int(Int::U32, v.into())] })
}

fn assign(place: Place, value: Value, line: u32) -> Statement {
    Statement { place, value, at: at(line) }
}

fn code() -> Code {
    let types = vec![
        Type::Int(Int::U32),
        Type::Int(Int::U64),
        Type::Duration,
        Type::Bool,
        Type::Struct { name: "()".into(), fields: vec![] },
        Type::Struct { name: "Count".into(), fields: vec![field("scans", U32)] },
        Type::Struct { name: "Scan".into(), fields: vec![field("now", DURATION), field("period", DURATION), field("cycle", U64)] },
        Type::Struct { name: "In".into(), fields: vec![field("value", U32), field("divisor", U32)] },
        Type::Struct { name: "Out".into(), fields: vec![field("value", U32)] },
        Type::Ref { to: COUNT, mutable: true },
        Type::Ref { to: SCAN, mutable: false },
        Type::Ref { to: IN, mutable: false },
        Type::Ref { to: OUT, mutable: true },
    ];
    let local = |ty| LocalDecl { ty, name: None };
    let scan = Function {
        name: "<Count as Program>::scan".into(),
        params: 4,
        locals: [UNIT, TypeId(9), TypeId(10), TypeId(11), TypeId(12), U32, BOOL, U32, U32].map(local).to_vec(),
        blocks: vec![
            Block {
                statements: vec![
                    assign(deref(1, &[0]), Value::Binary(BinOp::Add, copy(deref(1, &[0])), u32_const(1)), 1),
                    assign(Place::local(5), Value::Use(copy(deref(3, &[1]))), 2),
                    assign(Place::local(6), Value::Binary(BinOp::Eq, copy(Place::local(5)), u32_const(0)), 2),
                ],
                terminator: Terminator::Assert {
                    cond: moved(6),
                    expected: false,
                    check: Check::DivisionByZero,
                    next: BlockId(1),
                    at: at(2),
                },
            },
            Block {
                statements: vec![
                    assign(Place::local(7), Value::Binary(BinOp::Div, copy(deref(3, &[0])), moved(5)), 2),
                    assign(Place::local(8), Value::Binary(BinOp::Mul, copy(deref(1, &[0])), u32_const(100)), 3),
                    assign(deref(4, &[0]), Value::Binary(BinOp::Add, moved(8), moved(7)), 3),
                ],
                terminator: Terminator::Return { at: at(4) },
            },
        ],
    };
    Code { types, functions: vec![scan], files: vec!["lib.rs".into(), "count.rs".into()] }
}

const REGIONS: Regions = Regions { state: COUNT, scan: SCAN, inputs: IN, outputs: OUT };

fn loaded(first: &[Scalar]) -> Loaded<In, Out> {
    Loaded::new(code(), FunctionId(0), REGIONS, first.to_vec(), SAFE)
}

fn first() -> Vec<Scalar> {
    vec![Scalar::Int(Int::U32, FIRST.into())]
}

/// The plant: the inputs it is told for each read, a read taking the time it is told, every write logged.
struct Plant {
    clock: VirtualClock,
    inputs: fn(usize) -> In,
    stall: fn(usize) -> Duration,
    reads: usize,
    writes: Vec<(Duration, u32)>,
}

impl Plant {
    fn new(clock: &VirtualClock, inputs: fn(usize) -> In) -> Self {
        Plant { clock: clock.clone(), inputs, stall: |_| Duration::ZERO, reads: 0, writes: Vec::new() }
    }
}

impl Io<In, Out> for Plant {
    fn read_inputs(&mut self, _now: Duration) -> In {
        let index = self.reads;
        self.reads += 1;
        self.clock.advance((self.stall)(index));
        (self.inputs)(index)
    }

    fn write_outputs(&mut self, now: Duration, outputs: &Out) {
        self.writes.push((now, outputs.value));
    }
}

struct Key;

impl Switch for Key {
    fn armed(&self) -> bool {
        true
    }
}

const SIM: Target = Target { production: false, realtime: false, simulated: true };
const OFICINA: CommandSource = CommandSource::Oficina;

fn config() -> TaskConfig {
    TaskConfig { name: "main", period: ms(10) }
}

fn interpreted() -> Controller<Interpreted<In, Out>, Loaded<In, Out>> {
    Controller::power_on_from(config(), SIM, Restart::First, loaded(&first())).unwrap()
}

fn text<P: TaskProgram, S: Source<Program = P>>(c: &Controller<P, S>) -> String {
    c.state().to_string()
}

fn steady(index: usize) -> In {
    In { value: u32::try_from(index).unwrap() * 3, divisor: 1 + u32::try_from(index % 3).unwrap() }
}

/// The writes of a controller started at once and stepped `steps` times on a plant fed `inputs`.
fn started<P, S>(mut c: Controller<P, S>, inputs: fn(usize) -> In, steps: usize) -> (Controller<P, S>, Plant, VirtualClock)
where
    P: TaskProgram<Inputs = In, Outputs = Out>,
    S: Source<Program = P>,
{
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, inputs);
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, steps);
    (c, plant, clock)
}

fn step<P, S>(c: &mut Controller<P, S>, plant: &mut Plant, clock: &mut VirtualClock, steps: usize)
where
    P: TaskProgram<Inputs = In, Outputs = Out>,
    S: Source<Program = P>,
{
    for _ in 0..steps {
        c.step(plant, clock);
    }
}

/// What a scan of the count `n` writes for the inputs of read `index`.
fn scanned(n: u32, index: usize) -> u32 {
    let i = steady(index);
    n.wrapping_mul(100).wrapping_add(i.value / i.divisor)
}

#[test]
fn an_interpreted_program_runs_on_the_controller_as_its_native_twin_does() {
    let native = Controller::<Count>::power_on(config(), SIM, Restart::First).unwrap();
    let (native, native_plant, _) = started(native, steady, 300);
    let (machine, plant, clock) = started(interpreted(), steady, 300);
    assert_eq!(text(&machine), "RUN sim() no_realtime()");
    assert_eq!(text(&machine), text(&native));
    assert_eq!(plant.writes, native_plant.writes);
    let want: Vec<(Duration, u32)> = (0..300).map(|k| (ms(10) * k as u32, scanned(FIRST + 1 + k as u32, k))).collect();
    assert_eq!(plant.writes, want);
    assert_eq!(clock.now(), ms(2_990));
}

#[test]
fn a_program_error_faults_the_controller_naming_where_it_stood_and_the_exchange_goes_on() {
    fn zero_at_5(index: usize) -> In {
        if index == 5 { In { value: 9, divisor: 0 } } else { steady(index) }
    }
    let (mut c, mut plant, mut clock) = started(interpreted(), zero_at_5, 8);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime()"#);
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    let mut want: Vec<u32> = (0..5).map(|k| scanned(FIRST + 1 + k as u32, k)).collect();
    want.extend([SAFE.value; 3]);
    assert_eq!(values, want, "the safe outputs from the failing scan on, never what it computed");
    let times: Vec<Duration> = plant.writes.iter().map(|w| w.0).collect();
    assert_eq!(times, (0..8).map(|k| ms(10) * k).collect::<Vec<_>>(), "the exchange on every period");
    // acknowledged and started again, the program runs from the first state handed in, not where it stood
    c.acknowledge(OFICINA, &Key).unwrap();
    assert_eq!(text(&c), r#"STOP acknowledged(id=103) sim() no_realtime()"#);
    c.start(StartupReason::Warm, OFICINA, &Key).unwrap();
    plant.writes.clear();
    plant.inputs = steady;
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "RUN sim() no_realtime()");
    let index = plant.reads - 3;
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, (0..3).map(|k| scanned(FIRST + 1 + k as u32, index + k)).collect::<Vec<_>>());
}

#[test]
fn an_image_the_machine_refuses_and_outputs_read_back_as_none_are_the_runtime_s_fault() {
    fn refused_at_2(index: usize) -> In {
        if index == 2 { In { value: REFUSED, divisor: 1 } } else { steady(index) }
    }
    let (c, plant, _) = started(interpreted(), refused_at_2, 4);
    assert_eq!(text(&c), "FAULT internal(id=507) sim() no_realtime()");
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, [scanned(FIRST + 1, 0), scanned(FIRST + 2, 1), SAFE.value, SAFE.value]);
    // the count of 42 with a quotient of 42 writes 4242, which the outputs' image does not read back
    fn unreadable_at_1(index: usize) -> In {
        if index == 1 { In { value: 42, divisor: 1 } } else { steady(index) }
    }
    let (c, plant, _) = started(interpreted(), unreadable_at_1, 3);
    assert_eq!(text(&c), "FAULT internal(id=507) sim() no_realtime()");
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, [scanned(FIRST + 1, 0), SAFE.value, SAFE.value]);
}

#[test]
fn a_scan_past_twice_its_period_is_the_watchdog_s_fault_for_an_interpreted_program_too() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    plant.stall = |index| if index == 3 { ms(25) } else { Duration::ZERO };
    let mut c = interpreted();
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 6);
    assert_eq!(text(&c), r#"FAULT watchdog(id=501, task="main") sim() no_realtime() overruns(count=1)"#);
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values[..3], (0..3).map(|k| scanned(FIRST + 1 + k as u32, k)).collect::<Vec<_>>()[..]);
    assert_eq!(values[3..], [SAFE.value; 3]);
}

#[test]
fn while_no_program_runs_the_safe_outputs_handed_in_are_written() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    let mut c = interpreted();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "STOP sim() no_realtime()");
    assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)]);
}

#[test]
fn a_program_that_cannot_be_made_leaves_the_controller_in_fault_load_exchanging_the_safe_outputs() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    assert_eq!(text(&c), "FAULT load(id=506, download=0) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::State));
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)]);
    // acknowledged and started, its load fails again, the safe outputs going on
    c.acknowledge(OFICINA, &Key).unwrap();
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(text(&c), "FAULT load(id=506, download=0) sim() no_realtime()");
    assert_eq!(plant.writes.len(), 5);
    assert!(plant.writes.iter().all(|w| w.1 == SAFE.value), "{:?}", plant.writes);
    // whatever the state it would come back to; from its own failure, FAULT internal
    for restart in [Restart::PowerCycle { was: StateWord::Run }, Restart::PowerCycle { was: StateWord::Stop }] {
        let c = Controller::power_on_from(config(), SIM, restart, loaded(&two)).unwrap();
        assert_eq!(text(&c), "FAULT load(id=506, download=0) sim() no_realtime()", "{restart:?}");
    }
    let c = Controller::power_on_from(config(), SIM, Restart::Crash, loaded(&two)).unwrap();
    assert_eq!(text(&c), "FAULT internal(id=507) sim() no_realtime()");
    // an entry that is no function, by the adapter's reason
    let mut wrong = loaded(&first());
    wrong.entry = FunctionId(3);
    let c = Controller::power_on_from(config(), SIM, Restart::First, wrong).unwrap();
    assert!(matches!(c.load_error(), Some(MakeError::Entry(_))), "{:?}", c.load_error());
    // a program that is made has no load error; a zero period is refused when the controller comes up
    assert_eq!(interpreted().load_error(), None);
    let zero = TaskConfig { name: "main", period: Duration::ZERO };
    let refused = Controller::power_on_from(zero, SIM, Restart::First, loaded(&first())).err();
    assert_eq!(refused, Some(TaskError::ZeroPeriod { task: "main" }));
}

#[test]
fn what_safe_rust_cannot_reach_is_the_runtime_s_fault_not_the_program_s() {
    // a value read before anything wrote it: the quotient's statement taken out
    let mut uninit = code();
    uninit.functions[0].blocks[1].statements.remove(0);
    // code marked unreachable reached: the first block ends there
    let mut unreachable = code();
    unreachable.functions[0].blocks[0].terminator = Terminator::Unreachable { at: at(2) };
    for program in [uninit, unreachable] {
        let source = Loaded::new(program, FunctionId(0), REGIONS, first(), SAFE);
        let c = Controller::power_on_from(config(), SIM, Restart::First, source).unwrap();
        let (c, plant, _) = started(c, steady, 3);
        assert_eq!(text(&c), "FAULT internal(id=507) sim() no_realtime()");
        assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [SAFE.value; 3]);
    }
}

#[test]
fn a_stop_and_a_start_run_the_program_again_from_its_first_state() {
    let (mut c, mut plant, mut clock) = started(interpreted(), steady, 4);
    c.stop(OFICINA).unwrap();
    c.start(StartupReason::Warm, OFICINA, &Key).unwrap();
    plant.writes.clear();
    step(&mut c, &mut plant, &mut clock, 2);
    let index = plant.reads - 2;
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, [scanned(FIRST + 1, index), scanned(FIRST + 2, index + 1)]);
}

// The ninth refute round's tests (`.zen/evidence/C-152.md`): every program error a program reaches is 502 at its
// place, a faulted program's exchange on its own periods, what a load fault leaves, and the refusals in FAULT.

const USIZE: TypeId = TypeId(13);
const ARR2: TypeId = TypeId(14);
const PAIR: TypeId = TypeId(15);

fn usize_const(v: u64) -> Operand {
    Operand::Const(Const { ty: USIZE, slots: vec![Scalar::Int(Int::Usize, v.into())] })
}

/// A location in `count.rs`, line `line`, column 9, so a fault's column is the span's own.
fn at9(line: u32) -> Span {
    Span { file: 1, line, column: 9 }
}

fn st(place: Place, value: Value, line: u32) -> Statement {
    Statement { place, value, at: at9(line) }
}

/// The test's types, and a `usize`, a `[u32; 2]` and the `(u32, bool)` a checked operation gives.
fn types() -> Vec<Type> {
    let mut types = code().types;
    types.push(Type::Int(Int::Usize));
    types.push(Type::Array { element: U32, len: 2 });
    types.push(Type::Struct { name: "(u32, bool)".into(), fields: vec![field("0", U32), field("1", BOOL)] });
    types
}

fn function(locals: &[TypeId], blocks: Vec<Block>) -> Code {
    let local = |ty: &TypeId| LocalDecl { ty: *ty, name: None };
    let mut all = vec![UNIT, TypeId(9), TypeId(10), TypeId(11), TypeId(12)];
    all.extend_from_slice(locals);
    let scan = Function { name: "<Count as Program>::scan".into(), params: 4, locals: all.iter().map(local).collect(), blocks };
    Code { types: types(), functions: vec![scan], files: vec!["lib.rs".into(), "count.rs".into()] }
}

/// `let table = [10, 20]; out.value = table[inputs.value as usize];` with MIR's bounds assertion or without it.
fn indexing(asserted: bool) -> Code {
    let index = st(Place::local(5), Value::Cast(copy(deref(3, &[0])), USIZE), 2);
    let table = st(Place::local(6), Value::Aggregate(ARR2, None, vec![u32_const(10), u32_const(20)]), 2);
    let element = Place { local: Local(6), projections: vec![Projection::Index(Local(5))] };
    let read = st(deref(4, &[0]), Value::Use(copy(element)), 3);
    if asserted {
        let lt = st(Place::local(7), Value::Binary(BinOp::Lt, copy(Place::local(5)), usize_const(2)), 3);
        function(
            &[USIZE, ARR2, BOOL],
            vec![
                Block {
                    statements: vec![index, table, lt],
                    terminator: Terminator::Assert { cond: moved(7), expected: true, check: Check::Bounds, next: BlockId(1), at: at9(3) },
                },
                Block { statements: vec![read], terminator: Terminator::Return { at: at9(4) } },
            ],
        )
    } else {
        function(&[USIZE, ARR2], vec![Block { statements: vec![index, table, read], terminator: Terminator::Return { at: at9(4) } }])
    }
}

/// `out.value = inputs.value * 1000;` as MIR has it with overflow checks: the checked product, then the assertion.
fn overflowing() -> Code {
    let product = st(Place::local(5), Value::Checked(BinOp::Mul, copy(deref(3, &[0])), u32_const(1000)), 3);
    let flag = Operand::Copy(Place { local: Local(5), projections: vec![Projection::Field(1)] });
    let result = Operand::Copy(Place { local: Local(5), projections: vec![Projection::Field(0)] });
    function(
        &[PAIR],
        vec![
            Block {
                statements: vec![product],
                terminator: Terminator::Assert { cond: flag, expected: false, check: Check::Overflow(BinOp::Mul), next: BlockId(1), at: at9(3) },
            },
            Block { statements: vec![st(deref(4, &[0]), Value::Use(result), 3)], terminator: Terminator::Return { at: at9(4) } },
        ],
    )
}

fn on(program: Code) -> Controller<Interpreted<In, Out>, Loaded<In, Out>> {
    Controller::power_on_from(config(), SIM, Restart::First, Loaded::new(program, FunctionId(0), REGIONS, first(), SAFE)).unwrap()
}

#[test]
fn an_index_past_the_end_with_no_assertion_before_it_is_the_program_s_fault_at_its_statement() {
    fn past_at_2(index: usize) -> In {
        In { value: if index == 2 { 5 } else { u32::try_from(index % 2).unwrap() }, divisor: 1 }
    }
    let (c, plant, _) = started(on(indexing(false)), past_at_2, 4);
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [10, 20, SAFE.value, SAFE.value]);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:3:9") sim() no_realtime()"#);
}

#[test]
fn an_index_past_the_end_caught_by_mir_s_bounds_assertion_is_the_program_s_fault() {
    fn past_at_2(index: usize) -> In {
        In { value: if index == 2 { 2 } else { u32::try_from(index % 2).unwrap() }, divisor: 1 }
    }
    let (c, plant, _) = started(on(indexing(true)), past_at_2, 4);
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [10, 20, SAFE.value, SAFE.value]);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:3:9") sim() no_realtime()"#);
}

#[test]
fn an_overflow_caught_by_mir_s_assertion_is_the_program_s_fault() {
    fn big_at_1(index: usize) -> In {
        In { value: if index == 1 { 5_000_000 } else { 4 }, divisor: 1 }
    }
    let (c, plant, _) = started(on(overflowing()), big_at_1, 3);
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [4000, SAFE.value, SAFE.value]);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:3:9") sim() no_realtime()"#);
}

#[test]
fn a_program_s_fault_keeps_its_exchange_on_the_program_s_own_periods() {
    fn zero_at_6(index: usize) -> In {
        if index == 6 { In { value: 9, divisor: 0 } } else { steady(index) }
    }
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, zero_at_6);
    let mut c = interpreted();
    step(&mut c, &mut plant, &mut clock, 1); // the idle exchange at 0
    clock.advance(ms(13));
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 8);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime()"#);
    let times: Vec<Duration> = plant.writes.iter().map(|w| w.0).collect();
    assert_eq!(times, [0, 13, 23, 33, 43, 53, 63, 73, 83].map(ms), "the faulted program's exchange on its own periods");
    assert!(c.program().is_some(), "the faulted program is kept, where it stood");
}

#[test]
fn an_acknowledge_of_a_load_fault_leaves_stop_acknowledged_and_a_start_shows_its_load() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    c.acknowledge(OFICINA, &Key).unwrap();
    assert_eq!(text(&c), "STOP acknowledged(id=103) sim() no_realtime()");
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    assert_eq!(text(&c), "STARTUP cold(id=202) load() sim() no_realtime()");
}

/// A source whose program is made at power-on and not at the starts after it, as a stored program that changed
/// on its drive would be.
struct Fails {
    inner: Loaded<In, Out>,
    makes: usize,
}

impl Source for Fails {
    type Program = Interpreted<In, Out>;
    type Error = MakeError;
    fn make(&mut self) -> Result<Interpreted<In, Out>, MakeError> {
        self.makes += 1;
        if self.makes == 1 { self.inner.make() } else { Err(MakeError::Scan) }
    }
    fn safe_outputs(&mut self) -> Out {
        self.inner.safe_outputs()
    }
    fn reason(&self, program: &Interpreted<In, Out>, error: &orcl_runtime::ScanError) -> veredas_core::state::FaultReason {
        self.inner.reason(program, error)
    }
}

#[test]
fn a_start_s_failed_load_keeps_its_own_reason() {
    let source = Fails { inner: loaded(&first()), makes: 0 };
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, source).unwrap();
    assert_eq!(text(&c), "STOP sim() no_realtime()");
    assert_eq!(c.load_error(), None);
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "FAULT load(id=506, download=0) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Scan));
    assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)]);
}

#[test]
fn a_program_that_cannot_be_made_is_the_load_fault_after_a_power_cycle_from_every_state() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    for was in [StateWord::Run, StateWord::Startup, StateWord::Stop, StateWord::Hold, StateWord::Fault] {
        for target in [SIM, Target { production: true, realtime: false, simulated: false }] {
            let c = Controller::power_on_from(config(), target, Restart::PowerCycle { was }, loaded(&two)).unwrap();
            assert_eq!(c.state().state.word(), StateWord::Fault, "{was:?} {target:?}");
            assert!(text(&c).starts_with("FAULT load(id=506, download=0)"), "{was:?}: {}", text(&c));
        }
    }
}

#[test]
fn a_start_s_failed_load_exchanges_on_the_next_period_once() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    c.acknowledge(OFICINA, &Key).unwrap();
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(plant.writes, [0, 10, 20, 30, 40].map(|t| (ms(t), SAFE.value)));
}

#[test]
fn in_a_load_fault_and_a_program_fault_a_start_and_a_stop_are_refused_by_the_state() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    let before = c.state().clone();
    let refused = |command| Err(Refusal::State { command, state: StateWord::Fault });
    assert_eq!(c.start(StartupReason::Cold, OFICINA, &Key), refused(Command::Start(StartupReason::Cold)));
    assert_eq!(c.stop(OFICINA), refused(Command::Stop));
    assert_eq!(c.state(), &before);
    fn zero_at_1(index: usize) -> In {
        if index == 1 { In { value: 9, divisor: 0 } } else { steady(index) }
    }
    let (mut c, _, _) = started(interpreted(), zero_at_1, 3);
    let before = c.state().clone();
    assert_eq!(c.start(StartupReason::Warm, OFICINA, &Key), refused(Command::Start(StartupReason::Warm)));
    assert_eq!(c.stop(OFICINA), refused(Command::Stop));
    assert_eq!(c.state(), &before);
}

#[test]
fn a_fault_in_the_first_file_names_the_first_file() {
    let mut program = code();
    for block in &mut program.functions[0].blocks {
        for s in &mut block.statements {
            s.at.file = 0;
        }
        if let Terminator::Assert { at, .. } = &mut block.terminator {
            at.file = 0;
        }
    }
    fn zero_at_1(index: usize) -> In {
        if index == 1 { In { value: 9, divisor: 0 } } else { steady(index) }
    }
    let (c, _, _) = started(on(program), zero_at_1, 2);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="lib.rs:2:5") sim() no_realtime()"#);
}
