//! The machine runs a program as MIR means it: blocks and their terminators, calls by value and by reference,
//! places into structures, arrays and enums, a program's failures as errors that name where, one instruction a
//! step, and a run stopped at any step going on to the same end.
mod common;
use common::*;
use orcl_interp::*;

/// `fn sum(n: u32) -> u32 { let (mut s, mut i) = (0, 0); while i < n { i += 1; s += i; } s }`, with the checked
/// additions and assertions MIR gives it; and `fn twice(n: u32) -> u32`, which calls it.
fn sum_program() -> (Program, FunctionId, FunctionId) {
    let mut b = Build::default();
    let (u32t, boolean) = (b.int(Int::U32), b.bool());
    let pair = b.tuple(&[u32t, boolean]);
    let c = |v| konst(u32t, vec![int(Int::U32, v)]);
    let field = |l, i| copy_at(proj(l, vec![Projection::Field(i)]));
    let sum = b.function(Function {
        name: "sum".into(),
        params: 1,
        locals: locals(&[u32t, u32t, u32t, u32t, boolean, pair]),
        blocks: vec![
            // bb0
            block(vec![assign(place(2), Value::Use(c(0)), 1), assign(place(3), Value::Use(c(0)), 1)], Terminator::Goto(BlockId(1))),
            // bb1: while i < n
            block(
                vec![assign(place(4), Value::Binary(BinOp::Lt, copy(3), copy(1)), 2)],
                Terminator::SwitchInt { value: copy(4), targets: vec![(0, BlockId(4))], otherwise: BlockId(2), at: at(2) },
            ),
            // bb2: i += 1
            block(
                vec![assign(place(5), Value::Checked(BinOp::Add, copy(3), c(1)), 3)],
                Terminator::Assert {
                    cond: field(5, 1),
                    expected: false,
                    check: Check::Overflow(BinOp::Add),
                    next: BlockId(3),
                    at: at(3),
                },
            ),
            // bb3: s += i
            block(
                vec![assign(place(3), Value::Use(field(5, 0)), 3), assign(place(5), Value::Checked(BinOp::Add, copy(2), copy(3)), 4)],
                Terminator::Assert {
                    cond: field(5, 1),
                    expected: false,
                    check: Check::Overflow(BinOp::Add),
                    next: BlockId(5),
                    at: at(4),
                },
            ),
            // bb4: s
            block(vec![assign(place(0), Value::Use(copy(2)), 6)], ret()),
            // bb5
            block(vec![assign(place(2), Value::Use(field(5, 0)), 4)], Terminator::Goto(BlockId(1))),
        ],
    });
    let twice = b.function(Function {
        name: "twice".into(),
        params: 1,
        locals: locals(&[u32t, u32t, u32t]),
        blocks: vec![
            block(
                vec![],
                Terminator::Call { callee: Callee::Function(sum), args: vec![copy(1)], dest: place(2), next: BlockId(1), at: at(10) },
            ),
            block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(2), copy(2)), 11)], ret()),
        ],
    });
    b.program.files.push("sum.rs".into());
    (b.program, sum, twice)
}

fn u32s(v: i128) -> Vec<Scalar> {
    vec![int(Int::U32, v)]
}

#[test]
fn a_loop_runs_its_blocks_and_a_call_returns_its_result_to_the_caller() {
    let (program, sum, twice) = sum_program();
    let mut m = Machine::load(program, &[]).unwrap();
    for n in [0, 1, 2, 10, 1000] {
        let arg = [int(Int::U32, n)];
        assert_eq!(m.call(sum, &[Arg::Value(&arg)]).unwrap(), u32s(n * (n + 1) / 2).as_slice(), "sum({n})");
        // two statements and a goto, nine instructions a round, then the last test and the return
        assert_eq!(m.steps(), 9 * n as u64 + 7, "sum({n})'s instructions");
        assert_eq!(m.call(twice, &[Arg::Value(&arg)]).unwrap(), u32s(n * (n + 1)).as_slice(), "twice({n})");
        assert_eq!(m.steps(), 9 * n as u64 + 7 + 3, "the caller's call, its statement and its return");
    }
}

#[test]
fn an_overflow_is_an_error_naming_the_function_the_block_and_the_source_and_the_machine_stays_there() {
    let (program, sum, twice) = sum_program();
    let mut m = Machine::load(program, &[]).unwrap();
    // 1 + 2 + ... + 92682 passes u32::MAX at its last addition
    let arg = [int(Int::U32, 92_682)];
    let err = m.call(twice, &[Arg::Value(&arg)]).unwrap_err();
    let want = ProgramError {
        function: sum,
        block: BlockId(3),
        statement: None,
        at: at(4),
        kind: ErrorKind::Check(Check::Overflow(BinOp::Add)),
    };
    assert_eq!(err, CallError::Program(want));
    assert!(m.running(), "a failed entry stands where it failed");
    assert_eq!(m.depth(), 2, "inside the callee");
    assert_eq!(m.position(), Some((sum, BlockId(3), 2)), "at the block's terminator");
    assert_eq!(m.step(), Err(want), "and every step gives the same error");
    assert_eq!(m.start(sum, &[Arg::Value(&arg)]), Err(HostError::Busy), "nothing starts over it");
    m.abort();
    assert!(!m.running());
    assert_eq!(m.step(), Ok(Step::Idle));
    let arg = [int(Int::U32, 92_681)];
    assert_eq!(m.call(sum, &[Arg::Value(&arg)]).unwrap(), u32s(92_681 * 92_682 / 2).as_slice(), "one less fits");
}

#[test]
fn a_run_stopped_at_any_instruction_goes_on_to_the_same_end_and_a_copy_of_it_too() {
    let (program, sum, twice) = sum_program();
    let arg = [int(Int::U32, 37)];
    let mut whole = Machine::load(program.clone(), &[]).unwrap();
    let want = whole.call(twice, &[Arg::Value(&arg)]).unwrap().to_vec();
    let steps = whole.steps();
    for budget in [1u64, 2, 3, 7, 50, 333] {
        let mut m = Machine::load(program.clone(), &[]).unwrap();
        m.start(twice, &[Arg::Value(&arg)]).unwrap();
        assert_eq!(m.position(), Some((twice, BlockId(0), 0)));
        let mut copy: Option<Machine> = None;
        loop {
            match m.run(budget).unwrap() {
                Step::Ran => {
                    if copy.is_none() && m.depth() == 2 {
                        copy = Some(m.clone());
                    }
                }
                Step::Returned => break,
                Step::Idle => panic!("idle while running"),
            }
        }
        assert_eq!((m.result(), m.steps()), (want.as_slice(), steps), "budget {budget}");
        assert_eq!(m.memory(), whole.memory(), "the same memory, budget {budget}");
        let mut copy = copy.expect("stopped inside the callee once");
        while copy.step().unwrap() == Step::Ran {}
        assert_eq!((copy.result(), copy.steps(), copy.memory()), (want.as_slice(), steps, whole.memory()));
    }
    let mut m = Machine::load(program, &[]).unwrap();
    m.start(sum, &[Arg::Value(&arg)]).unwrap();
    let mut seen = vec![];
    while let Some(p) = m.position() {
        seen.push(p);
        m.step().unwrap();
    }
    assert_eq!(seen.len() as u64, 9 * 37 + 7, "one position a step");
    assert_eq!(seen[..4], [(sum, BlockId(0), 0), (sum, BlockId(0), 1), (sum, BlockId(0), 2), (sum, BlockId(1), 0)]);
}

#[test]
fn a_reference_writes_its_callers_place_and_a_host_region_is_the_state_an_entry_keeps() {
    let mut b = Build::default();
    let (u32t, unit) = (b.int(Int::U32), b.unit());
    let state = b.ty(Type::Struct {
        name: "State".into(),
        fields: vec![Field { name: "a".into(), ty: u32t }, Field { name: "b".into(), ty: u32t }],
    });
    let r = b.reference(u32t, true);
    let rs = b.reference(state, true);
    let one = konst(u32t, vec![int(Int::U32, 1)]);
    // fn bump(x: &mut u32) { *x = *x + 1 }
    let bump = b.function(Function {
        name: "bump".into(),
        params: 1,
        locals: locals(&[unit, r]),
        blocks: vec![block(
            vec![assign(proj(1, vec![Projection::Deref]), Value::Binary(BinOp::Add, copy_at(proj(1, vec![Projection::Deref])), one.clone()), 1)],
            ret(),
        )],
    });
    // fn scan(s: &mut State, k: u32) { s.a = k; bump(&mut s.b); bump(&mut s.b) }
    let deref_b = proj(1, vec![Projection::Deref, Projection::Field(1)]);
    let scan = b.function(Function {
        name: "scan".into(),
        params: 2,
        locals: locals(&[unit, rs, u32t, r, unit]),
        blocks: vec![
            block(
                vec![
                    assign(proj(1, vec![Projection::Deref, Projection::Field(0)]), Value::Use(copy(2)), 2),
                    assign(place(3), Value::Ref(deref_b.clone(), true), 3),
                ],
                Terminator::Call { callee: Callee::Function(bump), args: vec![copy(3)], dest: place(4), next: BlockId(1), at: at(3) },
            ),
            block(
                vec![assign(place(3), Value::Ref(deref_b, true), 4)],
                Terminator::Call { callee: Callee::Function(bump), args: vec![copy(3)], dest: place(4), next: BlockId(2), at: at(4) },
            ),
            block(vec![assign(place(0), Value::Aggregate(unit, None, vec![]), 5)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[state]).unwrap();
    let region = RegionId(0);
    m.set_region(region, &[int(Int::U32, 0), int(Int::U32, 40)]).unwrap();
    for (k, b) in [(7, 42), (9, 44), (11, 46)] {
        let arg = [int(Int::U32, k)];
        m.call(scan, &[Arg::Ref(region), Arg::Value(&arg)]).unwrap();
        assert_eq!(m.region(region).unwrap(), &[int(Int::U32, k), int(Int::U32, b)], "the state after scan({k})");
    }
    assert_eq!(m.set_region(region, &[int(Int::U32, 0)]), Err(HostError::Slots), "too few slots refused");
    assert_eq!(m.set_region(region, &[int(Int::U32, 0), int(Int::U16, 1)]), Err(HostError::Slots), "a slot of another type refused");
    let arg = [int(Int::U32, 1)];
    assert_eq!(m.start(scan, &[Arg::Value(&arg), Arg::Value(&arg)]), Err(HostError::Arguments), "a value for a reference refused");
    assert_eq!(m.start(scan, &[Arg::Ref(RegionId(1)), Arg::Value(&arg)]), Err(HostError::NoSuchRegion));
    assert_eq!(m.start(scan, &[Arg::Ref(region)]), Err(HostError::Arguments), "too few arguments");
    assert_eq!(m.start(FunctionId(9), &[]), Err(HostError::NoSuchFunction));
}

/// `enum E { A, B(u8, bool), C(i64) }`, discriminants 0, 5 and -3, and `struct S { a: u8, e: E, c: [i32; 3] }`.
fn aggregates() -> (Build, TypeId, TypeId, TypeId) {
    let mut b = Build::default();
    let (u8t, boolean, i64t, i32t) = (b.int(Int::U8), b.bool(), b.int(Int::I64), b.int(Int::I32));
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant {
                name: "B".into(),
                discriminant: 5,
                fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: boolean }],
            },
            Variant { name: "C".into(), discriminant: -3, fields: vec![Field { name: "0".into(), ty: i64t }] },
        ],
    });
    let arr = b.ty(Type::Array { element: i32t, len: 3 });
    let s = b.ty(Type::Struct {
        name: "S".into(),
        fields: vec![
            Field { name: "a".into(), ty: u8t },
            Field { name: "e".into(), ty: e },
            Field { name: "c".into(), ty: arr },
        ],
    });
    (b, e, arr, s)
}

#[test]
fn structures_arrays_and_enums_are_built_read_and_switched_on_as_mir_has_them() {
    let (mut b, e, arr, s) = aggregates();
    let (u8t, boolean, i64t, i32t, usize_t, isize_t) =
        (b.int(Int::U8), b.bool(), b.int(Int::I64), b.int(Int::I32), b.int(Int::Usize), b.int(Int::Isize));
    let i32c = |v| konst(i32t, vec![int(Int::I32, v)]);
    // fn build(a: u8, flag: bool, k: usize) -> i64:
    //   s = S { a, e: E::B(a, flag), c: [10, 20, 30] }; s.c[k] = s.c[2] + 1;
    //   match s.e { E::A => 0, E::B(x, _) => x as i64 + s.c[k] as i64, E::C(v) => v }
    let field = |path: Vec<Projection>| copy_at(proj(4, path));
    let build = b.function(Function {
        name: "build".into(),
        params: 3,
        locals: locals(&[i64t, u8t, boolean, usize_t, s, e, arr, isize_t, i64t, i64t, i32t]),
        blocks: vec![
            block(
                vec![
                    assign(place(5), Value::Aggregate(e, Some(1), vec![copy(1), copy(2)]), 1),
                    assign(place(6), Value::Aggregate(arr, None, vec![i32c(10), i32c(20), i32c(30)]), 1),
                    assign(place(4), Value::Aggregate(s, None, vec![copy(1), copy(5), copy(6)]), 1),
                    assign(place(10), Value::Use(field(vec![Projection::Field(2), Projection::ConstantIndex(2)])), 2),
                    assign(
                        proj(4, vec![Projection::Field(2), Projection::Index(Local(3))]),
                        Value::Binary(BinOp::Add, copy(10), i32c(1)),
                        2,
                    ),
                    assign(place(7), Value::Discriminant(proj(4, vec![Projection::Field(1)])), 3),
                ],
                Terminator::SwitchInt {
                    value: copy(7),
                    targets: vec![(0, BlockId(1)), (5, BlockId(2)), (-3, BlockId(3))],
                    otherwise: BlockId(4),
                    at: at(3),
                },
            ),
            block(vec![assign(place(0), Value::Use(konst(i64t, vec![int(Int::I64, 0)])), 4)], ret()),
            block(
                vec![
                    assign(place(8), Value::Cast(field(vec![Projection::Field(1), Projection::Downcast(1), Projection::Field(0)]), i64t), 5),
                    assign(place(9), Value::Cast(field(vec![Projection::Field(2), Projection::Index(Local(3))]), i64t), 5),
                    assign(place(0), Value::Binary(BinOp::Add, copy(8), copy(9)), 5),
                ],
                ret(),
            ),
            block(
                vec![assign(place(0), Value::Use(field(vec![Projection::Field(1), Projection::Downcast(2), Projection::Field(0)])), 6)],
                ret(),
            ),
            block(vec![], Terminator::Unreachable { at: at(7) }),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    for (a, k) in [(3, 0), (200, 1), (255, 2)] {
        let args = ([int(Int::U8, a)], [Scalar::Bool(true)], [int(Int::Usize, k)]);
        let got = m.call(build, &[Arg::Value(&args.0), Arg::Value(&args.1), Arg::Value(&args.2)]).unwrap().to_vec();
        // c[k] = c[2] + 1 = 31, so x + 31
        assert_eq!(got, vec![int(Int::I64, a + 31)], "build({a}, true, {k})");
    }
    let args = ([int(Int::U8, 1)], [Scalar::Bool(false)], [int(Int::Usize, 3)]);
    let err = m.call(build, &[Arg::Value(&args.0), Arg::Value(&args.1), Arg::Value(&args.2)]).unwrap_err();
    assert_eq!(
        err,
        CallError::Program(ProgramError {
            function: build,
            block: BlockId(0),
            statement: Some(4),
            at: at(2),
            kind: ErrorKind::Bounds { index: 3, len: 3 },
        }),
        "an index past the end, and the statement it was in"
    );
}

#[test]
fn an_enum_s_variant_writes_its_own_fields_and_leaves_the_rest_unwritten() {
    let (mut b, e, _, _) = aggregates();
    let i64t = b.int(Int::I64);
    let pair = b.tuple(&[e, e]);
    // fn f(v: i64) -> (E, E) { (E::C(v), E::A) }
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[pair, i64t, e, e]),
        blocks: vec![block(
            vec![
                assign(place(2), Value::Aggregate(e, Some(2), vec![copy(1)]), 1),
                assign(place(3), Value::Aggregate(e, Some(0), vec![]), 1),
                assign(place(0), Value::Aggregate(pair, None, vec![copy(2), copy(3)]), 1),
            ],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let arg = [int(Int::I64, -9)];
    let got = m.call(f, &[Arg::Value(&arg)]).unwrap().to_vec();
    // E is three slots: its discriminant, then B's u8 and bool or C's i64
    assert_eq!(
        got,
        vec![int(Int::Isize, -3), int(Int::I64, -9), Scalar::Uninit, int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit]
    );
}

#[test]
fn unreachable_code_and_a_value_never_written_are_errors_and_an_abort_starts_afresh() {
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    // fn f(x: u8) -> u8 { let y; _0 = x; if x == 0 { unreachable } _0 = x + y }
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, u8t, u8t, boolean]),
        blocks: vec![
            block(
                vec![assign(place(0), Value::Use(copy(1)), 1), assign(place(3), Value::Binary(BinOp::Eq, copy(1), konst(u8t, vec![int(Int::U8, 0)])), 1)],
                Terminator::SwitchInt { value: copy(3), targets: vec![(1, BlockId(1))], otherwise: BlockId(2), at: at(1) },
            ),
            block(vec![], Terminator::Unreachable { at: at(2) }),
            block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(1), copy(2)), 3)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let zero = [int(Int::U8, 0)];
    assert_eq!(
        m.call(f, &[Arg::Value(&zero)]),
        Err(CallError::Program(ProgramError { function: f, block: BlockId(1), statement: None, at: at(2), kind: ErrorKind::Unreachable }))
    );
    m.abort();
    let five = [int(Int::U8, 5)];
    assert_eq!(
        m.call(f, &[Arg::Value(&five)]),
        Err(CallError::Program(ProgramError { function: f, block: BlockId(2), statement: Some(0), at: at(3), kind: ErrorKind::Uninit }))
    );
    // the failing statement wrote nothing: the return value is still what the first statement wrote
    let frame = m.memory().to_vec();
    assert_eq!(frame[0], int(Int::U8, 5));
    m.abort();
    assert_eq!(m.memory(), frame.as_slice(), "an abort leaves memory as it was");
}

#[test]
fn the_same_program_and_arguments_give_the_same_memory_result_and_count_every_time() {
    let (program, _, twice) = sum_program();
    let runs: Vec<(Vec<Scalar>, Vec<Scalar>, u64)> = (0..3)
        .map(|_| {
            let mut m = Machine::load(program.clone(), &[]).unwrap();
            let arg = [int(Int::U32, 500)];
            let r = m.call(twice, &[Arg::Value(&arg)]).unwrap().to_vec();
            (r, m.memory().to_vec(), m.steps())
        })
        .collect();
    assert!(runs.windows(2).all(|w| w[0] == w[1]));
}
