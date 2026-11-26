//! An interpreted program on the controller (C-152): a program lowered by hand as rustc's MIR has it, made at
//! power-on and afresh at every start from what its host hands in, run beside its native twin; one that cannot be
//! made leaving it in STOP 104 `no_program` (Q233), its own error where Rust would panic the program's 502
//! `program(at)`, and what safe Rust cannot reach or a failure of the adapter or the host the runtime's 507
//! `internal`.
use std::time::Duration;

use orcl_interp::{
    Limits,
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

/// The limits the program runs within, in slots, as the target's settings will give them (`docs/questions.md` Q230):
/// a stack of 1024 and a memory of 2^16.
const STACK: u32 = 1024;
const LIMITS: Limits = Limits { stack: STACK, memory: 1 << 16 };

/// The tests' allocator: the system's, refusing any one allocation past 1 GiB, an abort of the test binary, so a
/// regression that lays out what a program or a host names fails a test and never fills the machine.
struct Capped;

// SAFETY: every call within the ceiling is handed to the system's allocator unchanged; one past it is refused with a
// null pointer, which the collections answer with `handle_alloc_error`, an abort.
unsafe impl std::alloc::GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        if layout.size() > 1 << 30 {
            return std::ptr::null_mut();
        }
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        if new_size > 1 << 30 {
            return std::ptr::null_mut();
        }
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static CAPPED: Capped = Capped;

fn loaded(first: &[Scalar]) -> Loaded<In, Out> {
    Loaded::new(code(), FunctionId(0), REGIONS, first.to_vec(), SAFE, LIMITS)
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
fn a_program_that_cannot_be_made_leaves_the_controller_in_stop_no_program_exchanging_the_safe_outputs() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::State));
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)]);
    // started, its load fails again, the safe outputs going on
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(plant.writes.len(), 5);
    assert!(plant.writes.iter().all(|w| w.1 == SAFE.value), "{:?}", plant.writes);
    // whatever the state it would come back to; from its own failure, FAULT internal
    for restart in [Restart::PowerCycle { was: StateWord::Run }, Restart::PowerCycle { was: StateWord::Stop }] {
        let c = Controller::power_on_from(config(), SIM, restart, loaded(&two)).unwrap();
        assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()", "{restart:?}");
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
        let source = Loaded::new(program, FunctionId(0), REGIONS, first(), SAFE, LIMITS);
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
    Controller::power_on_from(config(), SIM, Restart::First, Loaded::new(program, FunctionId(0), REGIONS, first(), SAFE, LIMITS)).unwrap()
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
fn a_start_with_no_program_shows_its_load() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
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
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Scan));
    assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)]);
}

#[test]
fn a_program_that_cannot_be_made_is_stop_no_program_after_a_power_cycle_from_every_state() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    for was in [StateWord::Run, StateWord::Startup, StateWord::Stop, StateWord::Hold, StateWord::Fault] {
        let production = Target { production: true, realtime: false, simulated: false };
        for (target, shown) in [(SIM, "STOP no_program(id=104) sim() no_realtime()"), (production, "STOP no_program(id=104)")] {
            let c = Controller::power_on_from(config(), target, Restart::PowerCycle { was }, loaded(&two)).unwrap();
            assert_eq!(text(&c), shown, "{was:?} {target:?}");
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
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(plant.writes, [0, 10, 20, 30, 40].map(|t| (ms(t), SAFE.value)));
}

#[test]
fn with_no_program_a_stop_and_an_acknowledge_are_refused_by_the_state_and_in_a_program_fault_a_start_and_a_stop() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    let before = c.state().clone();
    let in_stop = |command| Err(Refusal::State { command, state: StateWord::Stop });
    assert_eq!(c.stop(OFICINA), in_stop(Command::Stop));
    assert_eq!(c.acknowledge(OFICINA, &Key), in_stop(Command::Acknowledge));
    struct Off;
    impl Switch for Off {
        fn armed(&self) -> bool {
            false
        }
    }
    assert_eq!(c.start(StartupReason::Cold, OFICINA, &Off), Err(Refusal::Switch { command: Command::Start(StartupReason::Cold) }));
    let program = CommandSource::Program;
    let start = Command::Start(StartupReason::Warm);
    assert_eq!(c.start(StartupReason::Warm, program, &Key), Err(Refusal::Source { command: start, source: program }));
    assert_eq!(c.state(), &before);
    let refused = |command| Err(Refusal::State { command, state: StateWord::Fault });
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

// The tenth refute round's tests: a start after a load fault makes the program afresh, the reason a crash's power-on
// keeps, every check MIR asserts the program's 502, a zero period refused with any program, a power cycle's warm
// start making its program afresh, and the last load's reason kept.

/// A source whose makes go as scripted: the i-th make fails with `plan[i]` when it is `Some`, else makes the program.
struct Scripted {
    inner: Loaded<In, Out>,
    plan: Vec<Option<MakeError>>,
    makes: usize,
}

impl Source for Scripted {
    type Program = Interpreted<In, Out>;
    type Error = MakeError;
    fn make(&mut self) -> Result<Interpreted<In, Out>, MakeError> {
        let i = self.makes;
        self.makes += 1;
        match self.plan.get(i).cloned().flatten() {
            Some(error) => Err(error),
            None => self.inner.make(),
        }
    }
    fn safe_outputs(&mut self) -> Out {
        self.inner.safe_outputs()
    }
    fn reason(&self, program: &Interpreted<In, Out>, error: &orcl_runtime::ScanError) -> veredas_core::state::FaultReason {
        self.inner.reason(program, error)
    }
}

fn scripted(plan: Vec<Option<MakeError>>) -> Scripted {
    Scripted { inner: loaded(&first()), plan, makes: 0 }
}

// R10-M1: a start after no program makes the program afresh (a program mended on its drive runs).
#[test]
fn a_start_after_no_program_makes_the_program_afresh() {
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, scripted(vec![Some(MakeError::State)])).unwrap();
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "RUN sim() no_realtime()");
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, (0..3).map(|k| scanned(FIRST + 1 + k as u32, k)).collect::<Vec<_>>());
}

// R10-M2: a crash's power-on checks the program too, its reason kept for the host behind the 507.
#[test]
fn a_crash_s_power_on_checks_the_program_and_keeps_its_reason_behind_the_507() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::Crash, loaded(&two)).unwrap();
    assert_eq!(text(&c), "FAULT internal(id=507) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::State));
    c.acknowledge(OFICINA, &Key).unwrap();
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    step(&mut c, &mut plant, &mut clock, 1);
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
}

fn zero_at_1(index: usize) -> In {
    if index == 1 { In { value: 9, divisor: 0 } } else { steady(index) }
}

/// `assert(!(inputs.divisor == 0), check)` at line `line`, column 3, then `out.value = inputs.value`: an assertion
/// of the kind `check` that fails when the divisor read is zero.
fn asserting(check: Check, line: u32) -> Code {
    let zero = st(Place::local(5), Value::Binary(BinOp::Eq, copy(deref(3, &[1])), u32_const(0)), line);
    let at = Span { file: 1, line, column: 3 };
    function(
        &[BOOL],
        vec![
            Block {
                statements: vec![zero],
                terminator: Terminator::Assert { cond: moved(5), expected: false, check, next: BlockId(1), at },
            },
            Block { statements: vec![st(deref(4, &[0]), Value::Use(copy(deref(3, &[0]))), line)], terminator: Terminator::Return { at } },
        ],
    )
}

// R10-M3 to M5, and round 9's M2 and M3: every check MIR asserts, failed, is the program's own fault, 502 at its place
#[test]
fn every_check_mir_asserts_is_the_program_s_fault_at_its_place() {
    let ops = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem, BinOp::Shl, BinOp::Shr];
    let mut checks: Vec<Check> = ops.into_iter().map(Check::Overflow).collect();
    checks.extend([Check::NegOverflow, Check::DivisionByZero, Check::RemainderByZero, Check::Bounds]);
    for (k, check) in checks.into_iter().enumerate() {
        let line = 10 + u32::try_from(k).unwrap();
        let (c, plant, _) = started(on(asserting(check, line)), zero_at_1, 3);
        assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [0, SAFE.value, SAFE.value], "{check:?}");
        assert_eq!(text(&c), format!(r#"FAULT program(id=502, at="count.rs:{line}:3") sim() no_realtime()"#), "{check:?}");
    }
}

// R10-M6: a zero period is refused when the controller comes up, the program made or not.
#[test]
fn a_zero_period_is_refused_with_a_program_that_cannot_be_made_too() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let zero = TaskConfig { name: "main", period: Duration::ZERO };
    let refused = Controller::power_on_from(zero, SIM, Restart::First, loaded(&two)).err();
    assert_eq!(refused, Some(TaskError::ZeroPeriod { task: "main" }));
}

// R10-M7: a power cycle's warm start makes the program afresh at its load.
#[test]
fn a_power_cycle_s_warm_start_makes_the_program_afresh() {
    let source = scripted(vec![None, Some(MakeError::Scan)]);
    let mut c = Controller::power_on_from(config(), SIM, Restart::PowerCycle { was: StateWord::Run }, source).unwrap();
    assert_eq!(text(&c), "STARTUP warm(id=201) load() sim() no_realtime()");
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    step(&mut c, &mut plant, &mut clock, 1);
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Scan));
}

// R10-M9: the reason kept is the last load's.
#[test]
fn the_reason_kept_is_the_last_load_s() {
    let source = scripted(vec![Some(MakeError::State), Some(MakeError::Scan)]);
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, source).unwrap();
    assert_eq!(c.load_error(), Some(&MakeError::State));
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    step(&mut c, &mut plant, &mut clock, 1);
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Scan));
}

// The eleventh refute round's tests: a program error in a scan that overran within twice its period, and on the
// very first scan, the program's 502; a power cycle with no program keeping its reason and exchanging the safe
// outputs; and the load's reason outliving an acknowledge.

fn zero_at_0(index: usize) -> In {
    if index == 0 { In { value: 9, divisor: 0 } } else { steady(index) }
}

fn zero_at_2(index: usize) -> In {
    if index == 2 { In { value: 9, divisor: 0 } } else { steady(index) }
}

// a program error in a scan that overran its period, within twice it, is the program's 502,
// its overrun counted; never RUN with the task faulted, never the watchdog's.
#[test]
fn a_program_error_in_a_scan_that_overran_within_twice_its_period_is_the_program_s_fault_with_its_overrun() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, zero_at_2);
    plant.stall = |index| if index == 2 { ms(15) } else { Duration::ZERO };
    let mut c = interpreted();
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime() overruns(count=1)"#, "at the scan that failed");
    step(&mut c, &mut plant, &mut clock, 1);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime() overruns(count=1)"#);
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values, [scanned(FIRST + 1, 0), scanned(FIRST + 2, 1), SAFE.value, SAFE.value]);
}

// a program error on the program's very first scan (an input still zero at start-up) is the program's 502.
#[test]
fn a_program_error_on_the_very_first_scan_is_the_program_s_fault() {
    let (c, plant, _) = started(interpreted(), zero_at_0, 1);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime()"#);
    assert_eq!(plant.writes, [(ms(0), SAFE.value)]);
}

// a power cycle with no program keeps the adapter's reason and exchanges the safe outputs, from every state.
#[test]
fn a_power_cycle_with_no_program_keeps_its_reason_and_exchanges_the_safe_outputs_from_every_state() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    for was in [StateWord::Run, StateWord::Startup, StateWord::Stop, StateWord::Hold, StateWord::Fault] {
        let mut c = Controller::power_on_from(config(), SIM, Restart::PowerCycle { was }, loaded(&two)).unwrap();
        assert_eq!(c.load_error(), Some(&MakeError::State), "{was:?}");
        let mut clock = VirtualClock::new();
        let mut plant = Plant::new(&clock, steady);
        step(&mut c, &mut plant, &mut clock, 3);
        assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)], "{was:?}");
        assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()", "{was:?}");
    }
}

// the load's reason outlives the acknowledge of a crash's fault, and a refused acknowledge with no program.
#[test]
fn the_load_s_reason_outlives_an_acknowledge() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let mut c = Controller::power_on_from(config(), SIM, Restart::Crash, loaded(&two)).unwrap();
    c.acknowledge(OFICINA, &Key).unwrap();
    assert_eq!(c.load_error(), Some(&MakeError::State));
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, loaded(&two)).unwrap();
    assert!(c.acknowledge(OFICINA, &Key).is_err());
    assert_eq!(c.load_error(), Some(&MakeError::State));
}

// The twelfth refute round's tests: a program error whose location names no file still the program's 502, its file
// read as `?`; a power cycle with no program staying in STOP until a person's start, from every state; a first
// power-on and a crash's on a production target without real time checking the program; and a start's failed load
// after a run leaving no program, never the last run's.

// a program error whose span names no file of the program (which neither the file's reader nor the
// load refuses) is still a fault of the controller's, with the line and column, never a panic of its thread.
#[test]
fn a_program_error_whose_location_names_no_file_is_the_program_s_fault_its_file_unknown() {
    for file in [2, 9] {
    let mut program = code();
    for block in &mut program.functions[0].blocks {
        for s in &mut block.statements {
            s.at.file = file;
        }
        if let Terminator::Assert { at, .. } = &mut block.terminator {
            at.file = file;
        }
    }
    let (c, plant, _) = started(on(program), zero_at_1, 2);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="?:2:5") sim() no_realtime()"#);
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [scanned(FIRST + 1, 0), SAFE.value]);
    }
}

// with no program after a power cycle, whatever state it came back from and whether its source can make the
// program later or never, the controller stays in STOP 104 until a person's start, which makes it afresh.
#[test]
fn a_power_cycle_with_no_program_stays_in_stop_until_a_person_s_start_from_every_state() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    for was in [StateWord::Run, StateWord::Startup, StateWord::Stop, StateWord::Hold, StateWord::Fault] {
        let never = || Scripted { inner: loaded(&two), plan: vec![], makes: 0 };
        let mended = || scripted(vec![Some(MakeError::State)]);
        for (source, kind, then) in [(never(), "never made", "STOP no_program(id=104) sim() no_realtime()"), (mended(), "made once mended", "RUN sim() no_realtime()")] {
            let mut c = Controller::power_on_from(config(), SIM, Restart::PowerCycle { was }, source).unwrap();
            let mut clock = VirtualClock::new();
            let mut plant = Plant::new(&clock, steady);
            step(&mut c, &mut plant, &mut clock, 3);
            assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()", "{was:?}, {kind}");
            assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)], "{was:?}, {kind}");
            c.start(StartupReason::Warm, OFICINA, &Key).unwrap();
            step(&mut c, &mut plant, &mut clock, 1);
            assert_eq!(text(&c), then, "{was:?}, {kind}");
        }
    }
}

// with no program at power-on, its source able to make the program on a later try, the controller stays in
// STOP 104 until a person starts it: the load is never tried again by itself.
#[test]
fn a_power_on_with_no_program_is_never_retried_by_itself() {
    for restart in [Restart::First, Restart::PowerCycle { was: StateWord::Run }, Restart::PowerCycle { was: StateWord::Startup }] {
        let mut c = Controller::power_on_from(config(), SIM, restart, scripted(vec![Some(MakeError::State)])).unwrap();
        let mut clock = VirtualClock::new();
        let mut plant = Plant::new(&clock, steady);
        step(&mut c, &mut plant, &mut clock, 3);
        assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()", "{restart:?}");
        assert_eq!(plant.writes, [(ms(0), SAFE.value), (ms(10), SAFE.value), (ms(20), SAFE.value)], "{restart:?}");
    }
}

// a first power-on and a crash's on a production target without real time check the program too.
#[test]
fn a_first_power_on_and_a_crash_s_on_a_production_target_without_real_time_check_the_program() {
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    let production = Target { production: true, realtime: false, simulated: false };
    let mut c = Controller::power_on_from(config(), production, Restart::First, loaded(&two)).unwrap();
    assert_eq!(text(&c), "STOP no_program(id=104)");
    assert_eq!(c.load_error(), Some(&MakeError::State));
    // a start there is the real-time refusal, before any load
    assert_eq!(c.start(StartupReason::Cold, OFICINA, &Key), Err(Refusal::Realtime));
    assert_eq!(text(&c), "STOP realtime_refused(id=105)");
    let c = Controller::power_on_from(config(), production, Restart::Crash, loaded(&two)).unwrap();
    assert_eq!(text(&c), "FAULT internal(id=507)");
    assert_eq!(c.load_error(), Some(&MakeError::State));
}

// a start whose load fails after a run leaves no program, never the last run's (no substitute).
#[test]
fn a_start_s_failed_load_after_a_run_leaves_no_program_never_the_last_run_s() {
    let mut c = Controller::power_on_from(config(), SIM, Restart::First, scripted(vec![None, None, Some(MakeError::Scan)])).unwrap();
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "RUN sim() no_realtime()");
    c.stop(OFICINA).unwrap();
    c.start(StartupReason::Warm, OFICINA, &Key).unwrap();
    plant.writes.clear();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Scan));
    assert!(c.program().is_none());
    assert_eq!(plant.writes, [(ms(30), SAFE.value), (ms(40), SAFE.value), (ms(50), SAFE.value)]);
}

// The fourteenth refute round's tests, adopted at the clause's next change (Q233): a location's file shown as the
// program names it, its folders included; a start's failed load on a production target with real time left as a
// power-on's is; and after an acknowledged program fault the safe outputs on the target's periods.

/// The program of `code()` with its files named by paths, as a crate with modules in folders lists them.
fn with_files(files: &[&str], file: u32) -> Code {
    let mut program = code();
    program.files = files.iter().map(|f| f.to_string()).collect();
    for block in &mut program.functions[0].blocks {
        for s in &mut block.statements {
            s.at.file = file;
        }
        if let Terminator::Assert { at, .. } = &mut block.terminator {
            at.file = file;
        }
    }
    program
}

// R14-M3: the location's file is shown as the program names it, its folders included, so two `mod.rs` read apart.
#[test]
fn a_location_s_file_is_shown_as_the_program_names_it() {
    let files = ["src/conveyor/mod.rs", "src/valve/mod.rs"];
    for (file, want) in [(0, "src/conveyor/mod.rs"), (1, "src/valve/mod.rs")] {
        let (c, _, _) = started(on(with_files(&files, file)), zero_at_1, 2);
        assert_eq!(text(&c), format!(r#"FAULT program(id=502, at="{want}:2:5") sim() no_realtime()"#));
    }
}

// no program at power-on on a production target with real time too, then a start's own failed load and its
// refusals, the safe outputs on the target's periods throughout, and a start that makes it running it.
#[test]
fn a_start_s_failed_load_is_left_as_a_power_on_s_is() {
    let production = Target { production: true, realtime: true, simulated: false };
    let two = [Scalar::Int(Int::U32, 1), Scalar::Int(Int::U32, 2)];
    for restart in [Restart::First, Restart::PowerCycle { was: StateWord::Run }, Restart::PowerCycle { was: StateWord::Hold }] {
        let c = Controller::power_on_from(config(), production, restart, loaded(&two)).unwrap();
        assert_eq!(text(&c), "STOP no_program(id=104)", "{restart:?}");
    }
    let source = scripted(vec![None, Some(MakeError::Scan)]);
    let mut c = Controller::power_on_from(config(), production, Restart::First, source).unwrap();
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    clock.advance(ms(13));
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(text(&c), "STOP no_program(id=104)");
    let before = c.state().clone();
    let refused = |command| Err(Refusal::State { command, state: StateWord::Stop });
    assert_eq!(c.stop(OFICINA), refused(Command::Stop));
    assert_eq!(c.acknowledge(CommandSource::Local, &Key), refused(Command::Acknowledge));
    assert_eq!(c.state(), &before);
    step(&mut c, &mut plant, &mut clock, 1);
    c.start(StartupReason::Cold, CommandSource::Local, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 2);
    assert_eq!(text(&c), "RUN");
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    let times: Vec<Duration> = plant.writes.iter().map(|w| w.0).collect();
    // the failed load at 13 exchanges at 20, then 30 and 40, the run from 40 on its own clock
    assert_eq!(times, [20, 30, 40, 40, 50].map(ms));
    assert_eq!(values, [SAFE.value, SAFE.value, SAFE.value, scanned(FIRST + 1, 3), scanned(FIRST + 2, 4)]);
}

// after an acknowledged program fault the safe outputs handed in are exchanged on the target's periods, once in each,
// and a power cycle from RUN on a production target with real time runs the program from its first state.
#[test]
fn after_an_acknowledged_program_fault_the_safe_outputs_are_on_the_target_s_periods() {
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, zero_at_2);
    let mut c = interpreted();
    clock.advance(ms(7));
    c.start(StartupReason::Cold, OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 4); // scans at 7, 17, the fault at 27, the faulted task at 37
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:2:5") sim() no_realtime()"#);
    c.acknowledge(OFICINA, &Key).unwrap();
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "STOP acknowledged(id=103) sim() no_realtime()");
    let times: Vec<Duration> = plant.writes.iter().map(|w| w.0).collect();
    assert_eq!(times, [7, 17, 27, 37, 40, 50, 60].map(ms));
    let values: Vec<u32> = plant.writes.iter().map(|w| w.1).collect();
    assert_eq!(values[2..], [SAFE.value; 5]);
    let production = Target { production: true, realtime: true, simulated: false };
    let mut c = Controller::power_on_from(config(), production, Restart::PowerCycle { was: StateWord::Run }, loaded(&first())).unwrap();
    assert_eq!(text(&c), "STARTUP warm(id=201) load()");
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, steady);
    step(&mut c, &mut plant, &mut clock, 3);
    assert_eq!(text(&c), "RUN");
    assert_eq!(plant.writes, (0..3).map(|k| (ms(10) * k as u32, scanned(FIRST + 1 + k as u32, k as usize))).collect::<Vec<_>>());
}

// The stack the host hands in (`docs/questions.md` Q230): a scan that calls a function calling itself runs until the
// stack is full, and that call past it is the program's own fault, 502 at its place; a stack that cannot hold the
// scan entry's own frame makes no program, STOP 104.

/// The program with its scan calling `spin` after it wrote its outputs, `fn spin() { spin() }` at `count.rs:7:9`.
fn spinning() -> Code {
    let mut program = code();
    let then = Terminator::Call { callee: orcl_interp::Callee::Function(FunctionId(1)), args: vec![], dest: Place::local(0), next: BlockId(2), at: at(4) };
    program.functions[0].blocks[1].terminator = then;
    program.functions[0].blocks.push(Block { statements: vec![], terminator: Terminator::Return { at: at(5) } });
    let spin = Terminator::Call {
        callee: orcl_interp::Callee::Function(FunctionId(1)),
        args: vec![],
        dest: Place::local(0),
        next: BlockId(0),
        at: Span { file: 1, line: 7, column: 9 },
    };
    program.functions.push(Function {
        name: "spin".into(),
        params: 0,
        locals: vec![LocalDecl { ty: UNIT, name: None }],
        blocks: vec![Block { statements: vec![], terminator: spin }],
    });
    program
}

#[test]
fn a_call_past_the_stack_is_the_program_s_fault_at_its_place() {
    let source = Loaded::new(spinning(), FunctionId(0), REGIONS, first(), SAFE, LIMITS);
    let c = Controller::power_on_from(config(), SIM, Restart::First, source).unwrap();
    let (c, plant, _) = started(c, steady, 3);
    assert_eq!(text(&c), r#"FAULT program(id=502, at="count.rs:7:9") sim() no_realtime()"#);
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [SAFE.value; 3], "the faulting scan's outputs never sent");
    let program = c.program().unwrap();
    assert_eq!(program.machine().depth(), STACK as usize, "as many frames as the stack has slots, spin's holding none");
}

#[test]
fn a_stack_that_cannot_hold_the_scan_entry_s_frame_makes_no_program() {
    // the scan's frame: four references and four scalars, eight slots
    let c = Controller::power_on_from(config(), SIM, Restart::First, Loaded::<In, Out>::new(code(), FunctionId(0), REGIONS, first(), SAFE, Limits { stack: 8, ..LIMITS })).unwrap();
    assert_eq!((text(&c), c.load_error()), ("STOP sim() no_realtime()".to_string(), None));
    let (c, plant, _) = started(c, steady, 2);
    assert_eq!(text(&c), "RUN sim() no_realtime()");
    assert_eq!(plant.writes.iter().map(|w| w.1).collect::<Vec<_>>(), [scanned(FIRST + 1, 0), scanned(FIRST + 2, 1)]);
    let c = Controller::power_on_from(config(), SIM, Restart::First, Loaded::<In, Out>::new(code(), FunctionId(0), REGIONS, first(), SAFE, Limits { stack: 7, ..LIMITS })).unwrap();
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    assert_eq!(c.load_error(), Some(&MakeError::Entry(orcl_interp::HostError::Stack)));
}

// A memory ceiling the program does not fit (Q230) makes no program: STOP 104, the machine's reason kept.
#[test]
fn a_memory_ceiling_the_program_does_not_fit_makes_no_program() {
    let small = Limits { memory: STACK, ..LIMITS };
    let c = Controller::power_on_from(config(), SIM, Restart::First, Loaded::<In, Out>::new(code(), FunctionId(0), REGIONS, first(), SAFE, small)).unwrap();
    assert_eq!(text(&c), "STOP no_program(id=104) sim() no_realtime()");
    let reason = orcl_interp::LoadError::Region { index: 4, reason: "memory past the host's limit" };
    assert_eq!(c.load_error(), Some(&MakeError::Load(reason)));
}
