//! An interpreted program on the engine of a scan (C-155): a program lowered by hand as the front end lowers one,
//! run by a task beside its native twin on the same plant and the same clock, its faults, what the adapter refuses
//! when it is made, and a scan that allocates nothing.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Duration;

use orcl_interp::{
    BinOp, Block, BlockId, Check, Const, ErrorKind, Field, Function, FunctionId, HostError, Int, LocalDecl, Operand,
    Place, Program as Code, ProgramError, Projection, Scalar, Span, Statement, Terminator, Type, TypeId, Value,
};
use orcl_runtime::{
    Fault, Image, Interpreted, Io, MakeError, Regions, RunState, ScanError, Task, TaskConfig, TaskProgram, VirtualClock,
};
use orcl_std::{Program, Scan};

// The counting allocator: what a stretch of code asks of the heap, on this thread alone.

struct Counting;

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
    static ASKED: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every call is handed to the system's allocator unchanged; counting touches no memory it returns.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if WATCHED.with(|w| w.get()) {
            ASKED.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if WATCHED.with(|w| w.get()) {
            ASKED.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn asked_by<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = ASKED.with(|a| a.get());
    WATCHED.with(|w| w.set(true));
    let out = f();
    WATCHED.with(|w| w.set(false));
    (out, ASKED.with(|a| a.get()) - before)
}

// The program, natively: an accumulator, a quotient, and the scan's cycle and time copied out.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct In {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Out {
    sum: u32,
    quotient: u32,
    cycle: u64,
    now: Duration,
}

/// Its state is a running total; its outputs add the total to the sum the last scan left, so the output image the
/// task hands it shows in every write.
struct Acc {
    total: u32,
}

impl Program for Acc {
    type Inputs = In;
    type Outputs = Out;

    fn scan(&mut self, scan: &Scan, inputs: &In, outputs: &mut Out) {
        self.total = self.total.wrapping_add(inputs.x);
        outputs.sum = outputs.sum.wrapping_add(self.total);
        outputs.quotient = inputs.x / inputs.y;
        outputs.cycle = scan.cycle;
        outputs.now = scan.now;
    }

    fn safe_outputs(&self) -> Out {
        SAFE
    }
}

const SAFE: Out = Out { sum: 7, quotient: 7, cycle: 7, now: Duration::from_millis(7) };

fn u32_of(s: &Scalar) -> Option<u32> {
    match *s {
        Scalar::Int(Int::U32, v) => u32::try_from(v).ok(),
        _ => None,
    }
}

fn u64_of(s: &Scalar) -> Option<u64> {
    match *s {
        Scalar::Int(Int::U64, v) => u64::try_from(v).ok(),
        _ => None,
    }
}

impl Image for In {
    fn write(&self, slots: &mut Vec<Scalar>) {
        slots.extend([Scalar::Int(Int::U32, self.x.into()), Scalar::Int(Int::U32, self.y.into())]);
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        match slots {
            [x, y] => Some(In { x: u32_of(x)?, y: u32_of(y)? }),
            _ => None,
        }
    }
}

impl Image for Out {
    fn write(&self, slots: &mut Vec<Scalar>) {
        slots.extend([
            Scalar::Int(Int::U32, self.sum.into()),
            Scalar::Int(Int::U32, self.quotient.into()),
            Scalar::Int(Int::U64, self.cycle.into()),
            Scalar::Int(Int::U64, self.now.as_secs().into()),
            Scalar::Int(Int::U32, self.now.subsec_nanos().into()),
        ]);
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        match slots {
            [sum, quotient, cycle, secs, nanos] => {
                let now = Duration::new(u64_of(secs)?, u32_of(nanos).filter(|n| *n < 1_000_000_000)?);
                Some(Out { sum: u32_of(sum)?, quotient: u32_of(quotient)?, cycle: u64_of(cycle)?, now })
            }
            _ => None,
        }
    }
}

// The program, lowered by hand as rustc's MIR has it at opt-level 0 (the additions are the wrapping ones):
//
// fn scan(_1: &mut Acc, _2: &Scan, _3: &In, _4: &mut Out) {
//     bb0: (*_1).0 = Add(copy (*_1).0, copy (*_3).0);   (*_4).0 = Add(copy (*_4).0, copy (*_1).0);
//          _5 = copy (*_3).1;   _6 = Eq(copy _5, const 0_u32);   assert(!move _6, DivisionByZero) -> bb1
//     bb1: (*_4).1 = Div(copy (*_3).0, move _5);   (*_4).2 = copy (*_2).2;   (*_4).3 = copy (*_2).0;   return
// }

const U32: TypeId = TypeId(0);
const U64: TypeId = TypeId(1);
const DURATION: TypeId = TypeId(2);
const BOOL: TypeId = TypeId(3);
const UNIT: TypeId = TypeId(4);
const ACC: TypeId = TypeId(5);
const SCAN: TypeId = TypeId(6);
const IN: TypeId = TypeId(7);
const OUT: TypeId = TypeId(8);

fn at(line: u32) -> Span {
    Span { file: 0, line, column: 1 }
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

fn assign(place: Place, value: Value, line: u32) -> Statement {
    Statement { place, value, at: at(line) }
}

/// The program's types, then four references (9 `&mut Acc`, 10 `&Scan`, 11 `&In`, 12 `&mut Out`), then `extra`.
fn types(extra: &[Type]) -> Vec<Type> {
    let mut types = vec![
        Type::Int(Int::U32),
        Type::Int(Int::U64),
        Type::Duration,
        Type::Bool,
        Type::Struct { name: "()".into(), fields: vec![] },
        Type::Struct { name: "Acc".into(), fields: vec![field("total", U32)] },
        Type::Struct { name: "Scan".into(), fields: vec![field("now", DURATION), field("period", DURATION), field("cycle", U64)] },
        Type::Struct { name: "In".into(), fields: vec![field("x", U32), field("y", U32)] },
        Type::Struct {
            name: "Out".into(),
            fields: vec![field("sum", U32), field("quotient", U32), field("cycle", U64), field("now", DURATION)],
        },
        Type::Ref { to: ACC, mutable: true },
        Type::Ref { to: SCAN, mutable: false },
        Type::Ref { to: IN, mutable: false },
        Type::Ref { to: OUT, mutable: true },
    ];
    types.extend_from_slice(extra);
    types
}

fn scan_function(params: [TypeId; 4]) -> Function {
    let local = |ty| LocalDecl { ty, name: None };
    let zero = Operand::Const(Const { ty: U32, slots: vec![Scalar::Int(Int::U32, 0)] });
    Function {
        name: "<Acc as Program>::scan".into(),
        params: 4,
        locals: vec![local(UNIT), local(params[0]), local(params[1]), local(params[2]), local(params[3]), local(U32), local(BOOL)],
        blocks: vec![
            Block {
                statements: vec![
                    assign(deref(1, &[0]), Value::Binary(BinOp::Add, copy(deref(1, &[0])), copy(deref(3, &[0]))), 1),
                    assign(deref(4, &[0]), Value::Binary(BinOp::Add, copy(deref(4, &[0])), copy(deref(1, &[0]))), 2),
                    assign(Place::local(5), Value::Use(copy(deref(3, &[1]))), 3),
                    assign(Place::local(6), Value::Binary(BinOp::Eq, copy(Place::local(5)), zero), 3),
                ],
                terminator: Terminator::Assert {
                    cond: Operand::Move(Place::local(6)),
                    expected: false,
                    check: Check::DivisionByZero,
                    next: BlockId(1),
                    at: at(3),
                },
            },
            Block {
                statements: vec![
                    assign(deref(4, &[1]), Value::Binary(BinOp::Div, copy(deref(3, &[0])), Operand::Move(Place::local(5))), 3),
                    assign(deref(4, &[2]), Value::Use(copy(deref(2, &[2]))), 4),
                    assign(deref(4, &[3]), Value::Use(copy(deref(2, &[0]))), 5),
                ],
                terminator: Terminator::Return { at: at(6) },
            },
        ],
    }
}

/// A function that takes `params` and returns.
fn leaf(params: &[TypeId]) -> Function {
    let mut locals = vec![LocalDecl { ty: UNIT, name: None }];
    locals.extend(params.iter().map(|ty| LocalDecl { ty: *ty, name: None }));
    let params = u32::try_from(params.len()).unwrap();
    Function { name: "leaf".into(), params, locals, blocks: vec![Block { statements: vec![], terminator: Terminator::Return { at: at(1) } }] }
}

fn code() -> Code {
    Code { types: types(&[]), functions: vec![scan_function([TypeId(9), TypeId(10), TypeId(11), TypeId(12)])], files: vec!["acc.rs".into()] }
}

const REGIONS: Regions = Regions { state: ACC, scan: SCAN, inputs: IN, outputs: OUT };

fn interpreted(total: u32) -> Interpreted<In, Out> {
    Interpreted::new(code(), FunctionId(0), REGIONS, &[Scalar::Int(Int::U32, total.into())], SAFE).unwrap()
}

// The plant: inputs drawn from the read's index, a divisor of zero on the reads it is told, and the time each read
// takes, so scans run late, overrun and skip; every write logged by its time.

struct Plant {
    clock: VirtualClock,
    reads: u64,
    zero_on: Vec<u64>,
    writes: Vec<(Duration, Out)>,
}

impl Plant {
    fn new(clock: &VirtualClock, zero_on: &[u64]) -> Self {
        Plant { clock: clock.clone(), reads: 0, zero_on: zero_on.to_vec(), writes: Vec::new() }
    }
}

impl Io<In, Out> for Plant {
    fn read_inputs(&mut self, _now: Duration) -> In {
        let k = self.reads;
        self.reads += 1;
        // 0 to 20 ms: most scans within the 10 ms period, many a time error, none past twice it
        self.clock.advance(Duration::from_millis(k.wrapping_mul(7919) % 21));
        let x = u32::try_from(k.wrapping_mul(2_654_435_761) % 4_000_000_000).unwrap();
        In { x, y: if self.zero_on.contains(&k) { 0 } else { 1 + u32::try_from(k % 97).unwrap() } }
    }

    fn write_outputs(&mut self, now: Duration, outputs: &Out) {
        self.writes.push((now, *outputs));
    }
}

fn config() -> TaskConfig {
    TaskConfig { name: "acc", period: Duration::from_millis(10) }
}

/// What a report says of a scan, but the program and its state.
type Seen = (u64, Duration, Duration, bool, Out, bool);

fn seen<P: TaskProgram<Inputs = In, Outputs = Out>>(task: &mut Task<P>, plant: &mut Plant, clock: &mut VirtualClock) -> Seen {
    let r = task.step(plant, clock);
    (r.scan.cycle, r.scan.now, r.exec, r.time_error, *r.outputs, r.state == RunState::Running)
}

#[test]
fn an_interpreted_program_and_its_native_twin_write_the_same_outputs_at_the_same_times() {
    // 2000 scans of each, from a state of 100: on every scan the same cycle, time, length, time error, outputs and
    // state, and the same writes reach the plant; scans run late, overrun and skip on the way, none past twice the
    // period
    let (mut native_clock, mut interp_clock) = (VirtualClock::new(), VirtualClock::new());
    let (mut native_plant, mut interp_plant) = (Plant::new(&native_clock, &[]), Plant::new(&interp_clock, &[]));
    let mut native = Task::new(config(), Acc { total: 100 }).unwrap();
    let mut interp = Task::new(config(), interpreted(100)).unwrap();
    let mut time_errors = 0;
    for k in 0..2_000 {
        let a = seen(&mut native, &mut native_plant, &mut native_clock);
        let b = seen(&mut interp, &mut interp_plant, &mut interp_clock);
        assert_eq!(a, b, "scan {k}");
        assert!(a.5, "scan {k} still running");
        time_errors += usize::from(a.3);
    }
    assert_eq!(native_plant.writes, interp_plant.writes, "every write, by its time and value");
    assert_eq!(native.stats(), interp.stats());
    let first = native_plant.writes[0].1;
    assert_eq!(first, Out { sum: 100, quotient: 0, cycle: 0, now: Duration::ZERO }, "the first scan, from the state handed in");
    let skipped = native.stats().skipped;
    assert!(time_errors > 100 && skipped > 100, "the run met time errors ({time_errors}) and skips ({skipped})");
}

#[test]
fn the_scan_s_time_its_cycle_and_the_output_image_reach_the_program_scan_by_scan() {
    // reads that take no time: scan k at 10k ms, its outputs the sum so far, its cycle and its time
    let mut clock = VirtualClock::new();
    let mut task = Task::new(config(), interpreted(5)).unwrap();
    struct Steady;
    impl Io<In, Out> for Steady {
        fn read_inputs(&mut self, _now: Duration) -> In {
            In { x: 3, y: 2 }
        }
        fn write_outputs(&mut self, _now: Duration, _outputs: &Out) {}
    }
    let mut sum = 0u32;
    for k in 0..5u32 {
        let r = task.step(&mut Steady, &mut clock);
        sum += 5 + 3 * (k + 1);
        let want = Out { sum, quotient: 1, cycle: u64::from(k), now: Duration::from_millis(10 * u64::from(k)) };
        assert_eq!((*r.outputs, r.state), (want, RunState::Running), "scan {k}");
    }
    assert_eq!(task.program().machine().steps(), 9, "a scan's nine instructions: seven statements, an assertion, a return");
}

#[test]
fn a_program_error_faults_the_task_at_its_scan_s_start_and_leaves_the_machine_at_the_failed_assertion() {
    // the read of scan 6 hands a divisor of zero: the scan fails at its assertion, after it wrote a new sum into the
    // outputs region; the task faults at its start and writes the safe outputs, never that sum, and every later scan
    // writes them without scanning the program
    let mut clock = VirtualClock::new();
    let mut plant = Plant::new(&clock, &[6]);
    let mut task = Task::new(config(), interpreted(0)).unwrap();
    let mut states = Vec::new();
    for _ in 0..10 {
        let r = task.step(&mut plant, &mut clock);
        states.push((r.scan.cycle, r.scan.now, r.state));
    }
    let error = ProgramError {
        function: FunctionId(0),
        block: BlockId(0),
        statement: None,
        at: at(3),
        kind: ErrorKind::Check(Check::DivisionByZero),
    };
    let (cycle, start, _) = states[6];
    let fault = RunState::Faulted { at: start, fault: Fault::Program { error: ScanError::Program(error) } };
    assert!(states[..6].iter().all(|s| s.2 == RunState::Running), "running up to it");
    assert_eq!(states[6], (cycle, start, fault), "the failing scan, dated at its start");
    assert!(states[7..].iter().all(|s| s.2 == fault), "the first fault kept");
    let written: Vec<Out> = plant.writes.iter().map(|w| w.1).collect();
    assert!(written[..6].iter().all(|o| *o != SAFE), "the program's own outputs up to it");
    assert!(written[6..].iter().all(|o| *o == SAFE), "the safe outputs from the failing scan on");
    assert_eq!(task.program().machine().position(), Some((FunctionId(0), BlockId(0), 4)), "the machine at the assertion");
    assert_eq!(task.program().machine().steps(), 4, "the four statements before it; the failed assertion ran none");
}

#[test]
fn a_native_program_and_an_interpreted_one_are_task_programs_alike() {
    // the native program's error is `Infallible`; a task of each reports its state in its own error type
    fn state<P: TaskProgram>(task: &Task<P>) -> RunState<P::Error> {
        task.state()
    }
    let native: RunState = state(&Task::new(config(), Acc { total: 0 }).unwrap());
    let interp: RunState<ScanError> = state(&Task::new(config(), interpreted(0)).unwrap());
    assert_eq!((native, interp), (RunState::Running, RunState::Running));
}

/// Inputs whose image is wrong for an `x` of 13: a `u16` where the program's type has a `u32`.
#[derive(Clone, Copy, Default)]
struct Wrong(In);

impl Image for Wrong {
    fn write(&self, slots: &mut Vec<Scalar>) {
        if self.0.x == 13 {
            slots.extend([Scalar::Int(Int::U16, 13), Scalar::Int(Int::U32, self.0.y.into())]);
        } else {
            self.0.write(slots);
        }
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        In::read(slots).map(Wrong)
    }
}

/// Outputs that read back as none when their sum is 13.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Picky(Out);

impl Image for Picky {
    fn write(&self, slots: &mut Vec<Scalar>) {
        self.0.write(slots);
    }

    fn read(slots: &[Scalar]) -> Option<Self> {
        Out::read(slots).filter(|o| o.sum != 13).map(Picky)
    }
}

struct Feed<I>(Vec<I>);

impl<I: Copy, O> Io<I, O> for Feed<I> {
    fn read_inputs(&mut self, _now: Duration) -> I {
        self.0.remove(0)
    }
    fn write_outputs(&mut self, _now: Duration, _outputs: &O) {}
}

#[test]
fn an_image_the_machine_refuses_or_outputs_that_read_back_as_none_fault_the_task_by_name() {
    let state = [Scalar::Int(Int::U32, 0)];
    let program = Interpreted::<Wrong, Out>::new(code(), FunctionId(0), REGIONS, &state, SAFE).unwrap();
    let mut task = Task::new(config(), program).unwrap();
    let mut feed = Feed(vec![Wrong(In { x: 1, y: 1 }), Wrong(In { x: 13, y: 1 })]);
    let mut clock = VirtualClock::new();
    task.step(&mut feed, &mut clock);
    let r = task.step(&mut feed, &mut clock);
    let want = RunState::Faulted { at: Duration::from_millis(10), fault: Fault::Program { error: ScanError::Host(HostError::Slots) } };
    assert_eq!((r.state, *r.outputs), (want, SAFE), "inputs the machine refuses");

    let program = Interpreted::<In, Picky>::new(code(), FunctionId(0), REGIONS, &state, Picky(SAFE)).unwrap();
    let mut task = Task::new(config(), program).unwrap();
    // from a total of 0, an `x` of 13: the scan's sum is 13
    let mut feed = Feed(vec![In { x: 13, y: 1 }]);
    let mut clock = VirtualClock::new();
    let r = task.step(&mut feed, &mut clock);
    let want = RunState::Faulted { at: Duration::ZERO, fault: Fault::Program { error: ScanError::Outputs } };
    assert_eq!((r.state, *r.outputs), (want, Picky(SAFE)), "outputs that read back as none");
}

#[test]
fn the_adapter_refuses_by_name_what_does_not_fit_when_it_is_made() {
    let state = [Scalar::Int(Int::U32, 0)];
    let make = |code: Code, entry: u32, regions: Regions, state: &[Scalar]| {
        Interpreted::<In, Out>::new(code, FunctionId(entry), regions, state, SAFE).map(|_| ())
    };
    assert_eq!(make(code(), 0, REGIONS, &state), Ok(()), "the program as it is");
    // a program the machine refuses: a reference to a type past the last
    let mut bad = code();
    bad.types.push(Type::Ref { to: TypeId(99), mutable: false });
    assert!(matches!(make(bad, 0, REGIONS, &state), Err(MakeError::Load(_))), "a program the machine refuses");
    // a first state of another type, of too many slots, of none
    assert_eq!(make(code(), 0, REGIONS, &[Scalar::Int(Int::U64, 0)]), Err(MakeError::State), "a state of another type");
    assert_eq!(make(code(), 0, REGIONS, &[state[0], state[0]]), Err(MakeError::State), "a state of two slots");
    assert_eq!(make(code(), 0, REGIONS, &[]), Err(MakeError::State), "no state");
    // a scan region whose type does not hold a scan: its fields in another order, or a u64 alone
    let swapped = Type::Struct { name: "Scan".into(), fields: vec![field("cycle", U64), field("now", DURATION), field("period", DURATION)] };
    let mut other = code();
    other.types = types(&[swapped]);
    assert_eq!(make(other, 0, Regions { scan: TypeId(13), ..REGIONS }, &state), Err(MakeError::Scan), "a scan laid out otherwise");
    assert_eq!(make(code(), 0, Regions { scan: U64, ..REGIONS }, &state), Err(MakeError::Scan), "a u64 for a scan");
    // inputs and outputs whose types are not the images'
    assert_eq!(make(code(), 0, Regions { inputs: OUT, ..REGIONS }, &state), Err(MakeError::Inputs), "inputs of another type");
    assert_eq!(make(code(), 0, Regions { outputs: IN, ..REGIONS }, &state), Err(MakeError::Outputs), "outputs of another type");
    // safe outputs that read back as none, and outputs whose default does
    let picky = Interpreted::<In, Picky>::new(code(), FunctionId(0), REGIONS, &state, Picky(Out { sum: 13, ..SAFE }));
    assert_eq!(picky.map(|_| ()), Err(MakeError::Outputs), "safe outputs that read back as none");
    let picky_default = Interpreted::<In, Picky>::new(code(), FunctionId(0), REGIONS, &state, Picky(SAFE));
    assert!(picky_default.is_ok(), "outputs whose default reads back");
    // an entry that is no function, and ones whose parameters are not the four regions' references
    assert_eq!(make(code(), 1, REGIONS, &state), Err(MakeError::Entry(HostError::NoSuchFunction)), "no such function");
    for (params, case) in [
        (vec![TypeId(9), TypeId(11), TypeId(10), TypeId(12)], "the scan and the inputs swapped"),
        (vec![TypeId(9), TypeId(10), TypeId(11)], "three parameters"),
        (vec![TypeId(9), TypeId(10), TypeId(11), TypeId(12), U32], "five parameters"),
        (vec![TypeId(9), TypeId(10), TypeId(11), TypeId(13)], "the outputs by a reference to the inputs"),
    ] {
        let mut other = code();
        other.types = types(&[Type::Ref { to: IN, mutable: true }]);
        other.functions.push(leaf(&params));
        assert_eq!(make(other, 1, REGIONS, &state), Err(MakeError::Entry(HostError::Arguments)), "{case}");
    }
    let mut other = code();
    other.functions.push(leaf(&[TypeId(9), TypeId(10), TypeId(11), TypeId(12)]));
    assert_eq!(make(other, 1, REGIONS, &state), Ok(()), "an entry that takes the four regions and does nothing");
}

#[test]
fn a_scan_of_an_interpreted_program_allocates_nothing() {
    // after the task is made, a thousand scans and the fault ask nothing of the heap
    struct Quiet(u64);
    impl Io<In, Out> for Quiet {
        fn read_inputs(&mut self, _now: Duration) -> In {
            self.0 += 1;
            In { x: u32::try_from(self.0).unwrap(), y: if self.0 == 1_000 { 0 } else { 3 } }
        }
        fn write_outputs(&mut self, _now: Duration, _outputs: &Out) {}
    }
    let mut task = Task::new(config(), interpreted(1)).unwrap();
    let mut clock = VirtualClock::new();
    let mut plant = Quiet(0);
    let (state, asked) = asked_by(|| {
        for _ in 0..1_100 {
            task.step(&mut plant, &mut clock);
        }
        task.state()
    });
    assert!(matches!(state, RunState::Faulted { fault: Fault::Program { .. }, .. }), "the scan with a zero divisor faulted: {state:?}");
    assert_eq!(asked, 0, "the heap was asked {asked} times");
}

// The first refute round's tests (`.zen/evidence/C-155.md`): what the lowered program above does not read, the
// make's read-backs, a scan of many instructions, the image handed to a scan, the outputs' image refused, times and
// cycles past 32 bits, and a faulted machine left as it stood.

fn scan_region_slots(now: Duration, period: Duration, cycle: u64) -> Vec<Scalar> {
    vec![
        Scalar::Int(Int::U64, now.as_secs().into()),
        Scalar::Int(Int::U32, now.subsec_nanos().into()),
        Scalar::Int(Int::U64, period.as_secs().into()),
        Scalar::Int(Int::U32, period.subsec_nanos().into()),
        Scalar::Int(Int::U64, cycle.into()),
    ]
}

#[test]
fn the_scan_region_holds_the_scan_s_time_its_period_and_its_cycle_every_scan() {
    // M01, M02 (and M20): the period's slots are what the task's period is, every scan
    let mut clock = VirtualClock::new();
    let mut task = Task::new(config(), interpreted(5)).unwrap();
    let mut feed = Feed(vec![In { x: 3, y: 2 }; 5]);
    for k in 0..5u64 {
        let r = task.step(&mut feed, &mut clock);
        let (now, period) = (r.scan.now, r.scan.period);
        assert_eq!(r.state, RunState::Running);
        assert_eq!(task.program().machine().region(orcl_runtime::interpreted::SCAN).unwrap(), &scan_region_slots(now, period, k)[..], "scan {k}");
    }
}

/// Outputs whose image of the default is no value of the outputs' type (a `u16` for the sum).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct BadDefault(Out);

impl Image for BadDefault {
    fn write(&self, slots: &mut Vec<Scalar>) {
        if self.0 == Out::default() {
            slots.extend([Scalar::Int(Int::U16, 0)]);
            let mut rest = Vec::new();
            self.0.write(&mut rest);
            slots.extend_from_slice(&rest[1..]);
        } else {
            self.0.write(slots);
        }
    }
    fn read(slots: &[Scalar]) -> Option<Self> {
        Out::read(slots).map(BadDefault)
    }
}

/// Outputs whose default reads back as none.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct NoneDefault(Out);

impl Image for NoneDefault {
    fn write(&self, slots: &mut Vec<Scalar>) {
        self.0.write(slots);
    }
    fn read(slots: &[Scalar]) -> Option<Self> {
        Out::read(slots).filter(|o| *o != Out::default()).map(NoneDefault)
    }
}

#[test]
fn outputs_whose_default_is_no_value_of_their_type_or_reads_back_as_none_are_refused_when_made() {
    // M04, M04b (the default's image refused) and M05 (the default read back)
    let state = [Scalar::Int(Int::U32, 0)];
    let bad = Interpreted::<In, BadDefault>::new(code(), FunctionId(0), REGIONS, &state, BadDefault(SAFE));
    assert_eq!(bad.map(|_| ()), Err(MakeError::Outputs), "a default that is no value of the outputs' type");
    let none = Interpreted::<In, NoneDefault>::new(code(), FunctionId(0), REGIONS, &state, NoneDefault(SAFE));
    assert_eq!(none.map(|_| ()), Err(MakeError::Outputs), "a default that reads back as none");
}

/// `fn scan(..)`: a loop of `n` turns, each adding one to the outputs' sum; then return.
fn looping(n: u32) -> Code {
    let local = |ty| LocalDecl { ty, name: None };
    let c = |v: i128| Operand::Const(Const { ty: U32, slots: vec![Scalar::Int(Int::U32, v)] });
    let f = Function {
        name: "loop".into(),
        params: 4,
        locals: vec![local(UNIT), local(TypeId(9)), local(TypeId(10)), local(TypeId(11)), local(TypeId(12)), local(U32), local(BOOL)],
        blocks: vec![
            Block { statements: vec![assign(Place::local(5), Value::Use(c(0)), 1)], terminator: Terminator::Goto(BlockId(1)) },
            Block {
                statements: vec![assign(Place::local(6), Value::Binary(BinOp::Lt, copy(Place::local(5)), c(n.into())), 2)],
                terminator: Terminator::SwitchInt { value: Operand::Move(Place::local(6)), targets: vec![(0, BlockId(3))], otherwise: BlockId(2), at: at(2) },
            },
            Block {
                statements: vec![
                    assign(Place::local(5), Value::Binary(BinOp::Add, copy(Place::local(5)), c(1)), 3),
                    assign(deref(4, &[0]), Value::Binary(BinOp::Add, copy(deref(4, &[0])), c(1)), 3),
                ],
                terminator: Terminator::Goto(BlockId(1)),
            },
            Block { statements: vec![], terminator: Terminator::Return { at: at(4) } },
        ],
    };
    Code { types: types(&[]), functions: vec![f], files: vec!["loop.rs".into()] }
}

#[test]
fn a_scan_of_half_a_million_instructions_runs_to_its_return_with_no_budget() {
    // M07: no instruction budget; a scan of some 500 000 instructions ends at its return
    let p = Interpreted::<In, Out>::new(looping(100_000), FunctionId(0), REGIONS, &[Scalar::Int(Int::U32, 0)], SAFE).unwrap();
    let mut task = Task::new(config(), p).unwrap();
    let mut clock = VirtualClock::new();
    let r = task.step(&mut Feed(vec![In::default()]), &mut clock);
    assert_eq!((r.state, r.outputs.sum), (RunState::Running, 100_000));
    assert!(!task.program().machine().running());
}

#[test]
fn the_output_image_handed_to_a_scan_is_the_one_the_program_reads() {
    // M03: a scan called on outputs other than those the region holds
    let mut p = interpreted(0);
    let mut out = Out { sum: 1_000, ..Out::default() };
    let scan = Scan { now: Duration::ZERO, period: Duration::from_millis(10), cycle: 0 };
    TaskProgram::scan(&mut p, &scan, &In { x: 1, y: 1 }, &mut out).unwrap();
    assert_eq!(out.sum, 1_001);
}

/// Outputs whose image is wrong for a sum of 13: a `u16` where the program's type has a `u32`.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct WrongOut(Out);

impl Image for WrongOut {
    fn write(&self, slots: &mut Vec<Scalar>) {
        self.0.write(slots);
        if self.0.sum == 13 {
            slots[0] = Scalar::Int(Int::U16, 13);
        }
    }
    fn read(slots: &[Scalar]) -> Option<Self> {
        Out::read(slots).map(WrongOut)
    }
}

#[test]
fn an_output_image_the_machine_refuses_faults_the_task_as_the_host_s_error() {
    // M08 (refusal ignored), M10 (named Outputs)
    let state = [Scalar::Int(Int::U32, 0)];
    let p = Interpreted::<In, WrongOut>::new(code(), FunctionId(0), REGIONS, &state, WrongOut(SAFE)).unwrap();
    let mut task = Task::new(config(), p).unwrap();
    let mut feed = Feed(vec![In { x: 13, y: 1 }, In { x: 1, y: 1 }]);
    let mut clock = VirtualClock::new();
    let r = task.step(&mut feed, &mut clock);
    assert_eq!((r.state, r.outputs.0.sum), (RunState::Running, 13));
    let r = task.step(&mut feed, &mut clock);
    let want = RunState::Faulted { at: Duration::from_millis(10), fault: Fault::Program { error: ScanError::Host(HostError::Slots) } };
    assert_eq!((r.state, *r.outputs), (want, WrongOut(SAFE)));
}

#[test]
fn a_time_and_a_cycle_past_32_bits_reach_the_program_exactly() {
    // M09 (seconds truncated), M17 (cycle truncated)
    let mut clock = VirtualClock::new();
    let far = Duration::new(1 << 33, 123_456_789);
    clock.advance(far);
    let mut task = Task::new(TaskConfig { name: "acc", period: Duration::from_millis(1) }, interpreted(0)).unwrap();
    let mut feed = Feed(vec![In { x: 1, y: 1 }; 2]);
    let r = task.step(&mut feed, &mut clock);
    assert_eq!((r.outputs.now, r.outputs.cycle), (far, 0));
    let r = task.step(&mut feed, &mut clock);
    let cycle = r.scan.cycle;
    assert!(cycle > u64::from(u32::MAX));
    assert_eq!((r.outputs.now, r.outputs.cycle), (r.scan.now, cycle));
}

#[test]
fn nothing_of_the_program_runs_after_a_fault_that_leaves_the_machine_idle() {
    // M22: after an Outputs fault the machine is idle, so a run would show in the state region
    let state = [Scalar::Int(Int::U32, 0)];
    let p = Interpreted::<In, Picky>::new(code(), FunctionId(0), REGIONS, &state, Picky(SAFE)).unwrap();
    let mut task = Task::new(config(), p).unwrap();
    let mut feed = Feed(vec![In { x: 13, y: 1 }, In { x: 1, y: 1 }, In { x: 1, y: 1 }, In { x: 1, y: 1 }]);
    let mut clock = VirtualClock::new();
    for _ in 0..4 {
        let r = task.step(&mut feed, &mut clock);
        assert_eq!(*r.outputs, Picky(SAFE));
    }
    let m = task.program().machine();
    assert_eq!((m.region(orcl_runtime::interpreted::STATE).unwrap(), m.steps()), (&[Scalar::Int(Int::U32, 13)][..], 9));
}

// The second refute round's tests: the entry handed in is the one each scan calls, a period of seconds, inputs wider
// than the other regions, an image of more slots than its region, and safe outputs refused beside a good default.

#[test]
fn the_entry_handed_in_is_the_one_each_scan_calls() {
    // a leaf taking the four regions is function 0; the scan entry handed in is function 1
    let mut other = code();
    other.functions.insert(0, leaf(&[TypeId(9), TypeId(10), TypeId(11), TypeId(12)]));
    let mut p = Interpreted::<In, Out>::new(other, FunctionId(1), REGIONS, &[Scalar::Int(Int::U32, 5)], SAFE).unwrap();
    let mut out = Out::default();
    let scan = Scan { now: Duration::ZERO, period: Duration::from_millis(10), cycle: 0 };
    TaskProgram::scan(&mut p, &scan, &In { x: 3, y: 1 }, &mut out).unwrap();
    assert_eq!((out.sum, p.machine().steps()), (8, 9), "the entry handed in ran");
}

#[test]
fn a_period_of_seconds_reaches_the_scan_region() {
    let mut clock = VirtualClock::new();
    let period = Duration::new(1, 500_000_000);
    let mut task = Task::new(TaskConfig { name: "acc", period }, interpreted(0)).unwrap();
    let mut feed = Feed(vec![In { x: 1, y: 1 }; 2]);
    for k in 0..2u64 {
        let r = task.step(&mut feed, &mut clock);
        let (now, period) = (r.scan.now, r.scan.period);
        let region = task.program().machine().region(orcl_runtime::interpreted::SCAN).unwrap();
        assert_eq!(region, &scan_region_slots(now, period, k)[..], "scan {k}");
    }
}

/// Inputs of eight `u32`s: a region larger than the scan's and the outputs'.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Wide([u32; 8]);

impl Image for Wide {
    fn write(&self, slots: &mut Vec<Scalar>) {
        slots.extend(self.0.map(|v| Scalar::Int(Int::U32, v.into())));
    }
    fn read(slots: &[Scalar]) -> Option<Self> {
        let mut out = [0u32; 8];
        if slots.len() != 8 {
            return None;
        }
        for (o, s) in out.iter_mut().zip(slots) {
            *o = u32_of(s)?;
        }
        Some(Wide(out))
    }
}

#[test]
fn inputs_wider_than_the_other_regions_allocate_nothing_scan_after_scan() {
    // types 13 = [u32; 8], 14 = &[u32; 8]; the entry a leaf taking the four
    let mut other = code();
    other.types = types(&[Type::Array { element: U32, len: 8 }, Type::Ref { to: TypeId(13), mutable: false }]);
    other.functions = vec![leaf(&[TypeId(9), TypeId(10), TypeId(14), TypeId(12)])];
    let regions = Regions { inputs: TypeId(13), ..REGIONS };
    let p = Interpreted::<Wide, Out>::new(other, FunctionId(0), regions, &[Scalar::Int(Int::U32, 0)], SAFE).unwrap();
    let mut task = Task::new(config(), p).unwrap();
    let mut clock = VirtualClock::new();
    struct Many;
    impl Io<Wide, Out> for Many {
        fn read_inputs(&mut self, _now: Duration) -> Wide {
            Wide([1; 8])
        }
        fn write_outputs(&mut self, _now: Duration, _outputs: &Out) {}
    }
    let (state, asked) = asked_by(|| {
        for _ in 0..100 {
            task.step(&mut Many, &mut clock);
        }
        task.state()
    });
    assert_eq!((state, asked), (RunState::Running, 0));
}

/// Inputs whose image writes a third slot when `x` is 13: more slots than the region holds.
#[derive(Clone, Copy, Default)]
struct Long(In);

impl Image for Long {
    fn write(&self, slots: &mut Vec<Scalar>) {
        self.0.write(slots);
        if self.0.x == 13 {
            slots.push(Scalar::Int(Int::U32, 0));
        }
    }
    fn read(slots: &[Scalar]) -> Option<Self> {
        In::read(slots).map(Long)
    }
}

#[test]
fn an_image_of_more_slots_than_its_region_faults_the_task_as_the_host_s_error() {
    let state = [Scalar::Int(Int::U32, 0)];
    let p = Interpreted::<Long, Out>::new(code(), FunctionId(0), REGIONS, &state, SAFE).unwrap();
    let mut task = Task::new(config(), p).unwrap();
    let mut feed = Feed(vec![Long(In { x: 1, y: 1 }), Long(In { x: 13, y: 1 })]);
    let mut clock = VirtualClock::new();
    task.step(&mut feed, &mut clock);
    let r = task.step(&mut feed, &mut clock);
    let want = RunState::Faulted { at: Duration::from_millis(10), fault: Fault::Program { error: ScanError::Host(HostError::Slots) } };
    assert_eq!((r.state, *r.outputs), (want, SAFE));
}

#[test]
fn safe_outputs_that_are_no_value_of_the_outputs_type_are_refused_beside_a_default_that_is() {
    let state = [Scalar::Int(Int::U32, 0)];
    let refused = Interpreted::<In, WrongOut>::new(code(), FunctionId(0), REGIONS, &state, WrongOut(Out { sum: 13, ..SAFE }));
    assert_eq!(refused.map(|_| ()), Err(MakeError::Outputs));
}
