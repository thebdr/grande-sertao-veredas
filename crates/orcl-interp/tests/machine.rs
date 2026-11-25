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
    for budget in 1u64..=333 {
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

/// A chain `f1` → `f2` → ... → `f{depth}`, the last giving 7 and each the one below's result plus one.
fn chain(depth: u32) -> (Program, FunctionId) {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let mut below = b.function(Function {
        name: format!("f{depth}"),
        params: 0,
        locals: locals(&[u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 7)])), 1)], ret())],
    });
    for k in (1..depth).rev() {
        below = b.function(Function {
            name: format!("f{k}"),
            params: 0,
            locals: locals(&[u8t, u8t]),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Function(below), args: vec![], dest: place(1), next: BlockId(1), at: at(k) }),
                block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(1), konst(u8t, vec![int(Int::U8, 1)])), k)], ret()),
            ],
        });
    }
    (b.program, below)
}

#[test]
fn a_copy_taken_at_any_instruction_or_idle_is_a_machine_of_its_own_that_goes_on_alike() {
    // a copy at every instruction of a run, at every depth, each finished and compared with the run left alone
    let (program, _, twice) = sum_program();
    let arg = [int(Int::U32, 12)];
    let mut alone = Machine::load(program.clone(), &[]).unwrap();
    let want = alone.call(twice, &[Arg::Value(&arg)]).unwrap().to_vec();
    let mut m = Machine::load(program, &[]).unwrap();
    m.start(twice, &[Arg::Value(&arg)]).unwrap();
    let mut copies = 0;
    loop {
        let mut copy = m.clone();
        while copy.step().unwrap() == Step::Ran {}
        assert_eq!((copy.result(), copy.steps(), copy.memory()), (want.as_slice(), alone.steps(), alone.memory()));
        copies += 1;
        if m.step().unwrap() == Step::Returned {
            break;
        }
    }
    assert_eq!(copies as u64, alone.steps(), "a copy at every instruction");
    // a copy of an idle machine, and of one just started, through a chain of calls five deep
    let (program, top) = chain(5);
    let idle = Machine::load(program, &[]).unwrap();
    let mut copy = idle.clone();
    assert_eq!(copy.call(top, &[]).unwrap(), &[int(Int::U8, 11)]);
    let mut started = idle.clone();
    started.start(top, &[]).unwrap();
    let mut copy = started.clone();
    assert_eq!(copy.run(u64::MAX).unwrap(), Step::Returned);
    assert_eq!((copy.result(), copy.depth()), (&[int(Int::U8, 11)][..], 0));
    assert_eq!(started.run(u64::MAX).unwrap(), Step::Returned, "and the original, untouched by its copy");
    assert_eq!(started.result(), &[int(Int::U8, 11)]);
}

#[test]
fn a_budget_of_zero_runs_nothing_and_says_what_stands() {
    let (program, sum, _) = sum_program();
    let mut m = Machine::load(program, &[]).unwrap();
    assert_eq!(m.run(0), Ok(Step::Idle), "nothing running");
    let arg = [int(Int::U32, 3)];
    m.start(sum, &[Arg::Value(&arg)]).unwrap();
    assert_eq!((m.run(0), m.steps(), m.position()), (Ok(Step::Ran), 0, Some((sum, BlockId(0), 0))), "running, untouched");
    assert_eq!(m.run(u64::MAX), Ok(Step::Returned));
    let big = [int(Int::U32, 92_682)];
    m.start(sum, &[Arg::Value(&big)]).unwrap();
    let err = m.run(u64::MAX).unwrap_err();
    assert_eq!(m.run(0), Err(err), "a failed entry's error");
}

#[test]
fn a_second_start_while_an_entry_runs_is_refused_and_so_are_arguments_that_are_not_its_parameters() {
    let (program, sum, twice) = sum_program();
    let mut b = Build { program };
    let (u8t, u16t, unit) = (b.int(Int::U8), b.int(Int::U16), b.unit());
    let r = b.reference(u8t, true);
    let takes_ref = b.function(Function { name: "takes_ref".into(), params: 1, locals: locals(&[unit, r]), blocks: vec![block(vec![], ret())] });
    let mut m = Machine::load(b.program, &[u8t, u16t]).unwrap();
    let arg = [int(Int::U32, 5)];
    m.start(twice, &[Arg::Value(&arg)]).unwrap();
    m.step().unwrap();
    assert_eq!(m.start(sum, &[Arg::Value(&arg)]), Err(HostError::Busy), "inside a call");
    assert_eq!(m.run(u64::MAX).unwrap(), Step::Returned);
    assert_eq!(m.result(), u32s(30).as_slice(), "and the running entry went on undisturbed");
    assert_eq!(m.start(sum, &[Arg::Value(&arg), Arg::Value(&arg)]), Err(HostError::Arguments), "too many");
    assert_eq!(m.start(sum, &[]), Err(HostError::Arguments), "too few");
    let u16_arg = [int(Int::U16, 5)];
    assert_eq!(m.start(sum, &[Arg::Value(&u16_arg)]), Err(HostError::Arguments), "a value of another type");
    assert_eq!(m.start(takes_ref, &[Arg::Ref(RegionId(1))]), Err(HostError::Arguments), "a region of another type");
    assert_eq!(m.start(takes_ref, &[Arg::Ref(RegionId(0))]), Ok(()));
}

#[test]
fn slots_that_are_no_value_of_a_region_s_type_are_refused() {
    let mut b = Build::default();
    let (u8t, d) = (b.int(Int::U8), b.ty(Type::Duration));
    let opt = b.option(u8t);
    let mut m = Machine::load(b.program, &[opt, d]).unwrap();
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 0), int(Int::U8, 9)]), Err(HostError::Slots), "None with a payload");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 2), int(Int::U8, 9)]), Err(HostError::Slots), "no such variant");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 1), int(Int::U8, 9)]), Ok(()));
    assert_eq!(m.set_region(RegionId(1), &[int(Int::U64, 0), int(Int::U32, 1_000_000_000)]), Err(HostError::Slots), "a second's nanoseconds");
    assert_eq!(m.set_region(RegionId(1), &[int(Int::U64, 0), int(Int::U32, 999_999_999)]), Ok(()));
    assert_eq!(m.set_region(RegionId(2), &[]), Err(HostError::NoSuchRegion));
}

#[test]
fn a_caller_s_own_local_is_written_through_a_reference_and_a_callee_indexes_its_own_locals() {
    // fn bump(x: &mut u8) { *x = *x + 1 }  fn caller() -> u8 { let mut v = 5; bump(&mut v); bump(&mut v); v }
    let mut b = Build::default();
    let (u8t, unit, usize_t) = (b.int(Int::U8), b.unit(), b.int(Int::Usize));
    let r = b.reference(u8t, true);
    let one = konst(u8t, vec![int(Int::U8, 1)]);
    let bump = b.function(Function {
        name: "bump".into(),
        params: 1,
        locals: locals(&[unit, r]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref]), Value::Binary(BinOp::Add, copy_at(proj(1, vec![Projection::Deref])), one), 1)], ret())],
    });
    let call_bump = |next| Terminator::Call { callee: Callee::Function(bump), args: vec![copy(2)], dest: place(3), next: BlockId(next), at: at(2) };
    let caller = b.function(Function {
        name: "caller".into(),
        params: 0,
        locals: locals(&[u8t, u8t, r, unit]),
        blocks: vec![
            block(vec![assign(place(1), Value::Use(konst(u8t, vec![int(Int::U8, 5)])), 1), assign(place(2), Value::Ref(place(1), true), 2)], call_bump(1)),
            block(vec![], call_bump(2)),
            block(vec![assign(place(0), Value::Use(copy(1)), 3)], ret()),
        ],
    });
    // fn get(a: [u8; 4], i: usize) -> u8 { a[i] }  fn main(k: usize) -> u8 { get([10, 20, 30, 40], 3 - k) }
    let arr = b.ty(Type::Array { element: u8t, len: 4 });
    let get = b.function(Function {
        name: "get".into(),
        params: 2,
        locals: locals(&[u8t, arr, usize_t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Index(Local(2))]))), 1)], ret())],
    });
    let c = |v| konst(u8t, vec![int(Int::U8, v)]);
    let main = b.function(Function {
        name: "main".into(),
        params: 1,
        locals: locals(&[u8t, usize_t, arr, usize_t]),
        blocks: vec![
            block(
                vec![
                    assign(place(2), Value::Aggregate(arr, None, vec![c(10), c(20), c(30), c(40)]), 2),
                    assign(place(3), Value::Binary(BinOp::Sub, konst(usize_t, vec![int(Int::Usize, 3)]), copy(1)), 2),
                ],
                Terminator::Call { callee: Callee::Function(get), args: vec![copy(2), copy(3)], dest: place(0), next: BlockId(1), at: at(2) },
            ),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(m.call(caller, &[]).unwrap(), &[int(Int::U8, 7)]);
    for k in 0..4 {
        let arg = [int(Int::Usize, k)];
        assert_eq!(m.call(main, &[Arg::Value(&arg)]).unwrap(), &[int(Int::U8, 10 * (4 - k))], "a[3 - {k}] in the callee");
    }
}

#[test]
fn a_callee_calling_a_small_then_a_large_function_has_room_for_both() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let big = b.ty(Type::Array { element: u8t, len: 64 });
    let leaf = |b: &mut Build, name: &str, extra: &[TypeId], v: i128| {
        let mut types = vec![u8t];
        types.extend_from_slice(extra);
        b.function(Function {
            name: name.into(),
            params: 0,
            locals: locals(&types),
            blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, v)])), 1)], ret())],
        })
    };
    let small = leaf(&mut b, "small", &[], 1);
    let large = leaf(&mut b, "large", &[big], 2);
    let call = |f, dest, next| Terminator::Call { callee: Callee::Function(f), args: vec![], dest: place(dest), next: BlockId(next), at: at(1) };
    let mid = b.function(Function {
        name: "mid".into(),
        params: 0,
        locals: locals(&[u8t, u8t]),
        blocks: vec![block(vec![], call(small, 1, 1)), block(vec![], call(large, 0, 2)), block(vec![], ret())],
    });
    let main = b.function(Function {
        name: "main".into(),
        params: 0,
        locals: locals(&[u8t]),
        blocks: vec![block(vec![], call(mid, 0, 1)), block(vec![], ret())],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(m.call(main, &[]).unwrap(), &[int(Int::U8, 2)]);
    assert_eq!(m.memory().len(), 1 + 2 + 65, "main's frame, mid's and the larger callee's, laid out");
}

/// `fn g(flag: bool) -> u8 { let y; if flag { y = 7 } y + 0 }`: its `y` unwritten when the flag is off.
fn maybe_unwritten(b: &mut Build) -> FunctionId {
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    b.function(Function {
        name: "g".into(),
        params: 1,
        locals: locals(&[u8t, boolean, u8t]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: copy(1), targets: vec![(0, BlockId(2))], otherwise: BlockId(1), at: at(1) }),
            block(vec![assign(place(2), Value::Use(konst(u8t, vec![int(Int::U8, 7)])), 2)], Terminator::Goto(BlockId(2))),
            block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(2), konst(u8t, vec![int(Int::U8, 0)])), 3)], ret()),
        ],
    })
}

#[test]
fn a_local_is_unwritten_again_at_every_start_and_every_call() {
    let mut b = Build::default();
    let g = maybe_unwritten(&mut b);
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, boolean]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Function(g), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(4) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    for entry in [g, f] {
        assert_eq!(m.call(entry, &[Arg::Value(&[Scalar::Bool(true)])]).unwrap(), &[int(Int::U8, 7)]);
        let e = m.call(entry, &[Arg::Value(&[Scalar::Bool(false)])]).map(|r| r.to_vec());
        assert!(matches!(e, Err(CallError::Program(ProgramError { kind: ErrorKind::Uninit, function, .. })) if function == g), "{e:?}");
        m.abort();
    }
}

#[test]
fn rustc_s_bounds_check_asserts_true_and_a_shift_check_too() {
    // fn get(a: [u8; 4], i: usize) -> u8 { a[i] } as rustc lowers it at opt-level 0:
    //   _3 = const 4_usize; _4 = Lt(_2, _3); assert(move _4, "index out of bounds") -> bb1; bb1: _0 = _1[_2]
    // fn shl(x: u8, n: u32) -> u8 { x << n }: _3 = Lt(_2, const 8_u32); assert(move _3, "shift left with overflow")
    let mut b = Build::default();
    let (u8t, usize_t, boolean, u32t) = (b.int(Int::U8), b.int(Int::Usize), b.bool(), b.int(Int::U32));
    let arr = b.ty(Type::Array { element: u8t, len: 4 });
    let get = b.function(Function {
        name: "get".into(),
        params: 2,
        locals: locals(&[u8t, arr, usize_t, usize_t, boolean]),
        blocks: vec![
            block(
                vec![
                    assign(place(3), Value::Use(konst(usize_t, vec![int(Int::Usize, 4)])), 1),
                    assign(place(4), Value::Binary(BinOp::Lt, copy(2), copy(3)), 1),
                ],
                Terminator::Assert { cond: copy(4), expected: true, check: Check::Bounds, next: BlockId(1), at: at(1) },
            ),
            block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Index(Local(2))]))), 1)], ret()),
        ],
    });
    let shl = b.function(Function {
        name: "shl".into(),
        params: 2,
        locals: locals(&[u8t, u8t, u32t, boolean]),
        blocks: vec![
            block(
                vec![assign(place(3), Value::Binary(BinOp::Lt, copy(2), konst(u32t, vec![int(Int::U32, 8)])), 2)],
                Terminator::Assert { cond: copy(3), expected: true, check: Check::Overflow(BinOp::Shl), next: BlockId(1), at: at(2) },
            ),
            block(vec![assign(place(0), Value::Binary(BinOp::Shl, copy(1), copy(2)), 2)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let a = [int(Int::U8, 10), int(Int::U8, 20), int(Int::U8, 30), int(Int::U8, 40)];
    for i in 0..4 {
        let arg = [int(Int::Usize, i)];
        assert_eq!(m.call(get, &[Arg::Value(&a), Arg::Value(&arg)]).unwrap(), &[int(Int::U8, 10 * (i + 1))], "a[{i}]");
    }
    let past = [int(Int::Usize, 4)];
    assert_eq!(
        m.call(get, &[Arg::Value(&a), Arg::Value(&past)]),
        Err(CallError::Program(ProgramError { function: get, block: BlockId(0), statement: None, at: at(1), kind: ErrorKind::Check(Check::Bounds) }))
    );
    m.abort();
    for n in 0..8 {
        let args = ([int(Int::U8, 3)], [int(Int::U32, n)]);
        assert_eq!(m.call(shl, &[Arg::Value(&args.0), Arg::Value(&args.1)]).unwrap(), &[int(Int::U8, (3 << n) & 0xFF)], "3 << {n}");
    }
    let args = ([int(Int::U8, 3)], [int(Int::U32, 8)]);
    assert_eq!(
        m.call(shl, &[Arg::Value(&args.0), Arg::Value(&args.1)]).map(|r| r.to_vec()).unwrap_err(),
        CallError::Program(ProgramError { function: shl, block: BlockId(0), statement: None, at: at(2), kind: ErrorKind::Check(Check::Overflow(BinOp::Shl)) })
    );
}

#[test]
fn a_callee_s_result_of_several_slots_reaches_its_caller_whole() {
    // fn pair(x: u8) -> (u8, bool, Duration) { (x, true, d) }  fn main(x: u8) -> (u8, bool, Duration) { pair(x) },
    // and a callee's Option: fn none() -> Option<u8> { None }, its payload unwritten in the caller's place too
    let mut b = Build::default();
    let (u8t, boolean, d) = (b.int(Int::U8), b.bool(), b.ty(Type::Duration));
    let t = b.tuple(&[u8t, boolean, d]);
    let opt = b.option(u8t);
    let pair = b.function(Function {
        name: "pair".into(),
        params: 1,
        locals: locals(&[t, u8t]),
        blocks: vec![block(
            vec![assign(
                place(0),
                Value::Aggregate(t, None, vec![copy(1), konst(boolean, vec![Scalar::Bool(true)]), konst(d, vec![int(Int::U64, 7), int(Int::U32, 9)])]),
                1,
            )],
            ret(),
        )],
    });
    let none = b.function(Function {
        name: "none".into(),
        params: 0,
        locals: locals(&[opt]),
        blocks: vec![block(vec![assign(place(0), Value::Aggregate(opt, Some(0), vec![]), 1)], ret())],
    });
    let main = b.function(Function {
        name: "main".into(),
        params: 1,
        locals: locals(&[t, u8t, opt]),
        blocks: vec![
            block(vec![assign(place(2), Value::Aggregate(opt, Some(1), vec![copy(1)]), 2)], Terminator::Call { callee: Callee::Function(pair), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(2) }),
            block(vec![], Terminator::Call { callee: Callee::Function(none), args: vec![], dest: place(2), next: BlockId(2), at: at(3) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let arg = [int(Int::U8, 9)];
    assert_eq!(m.call(main, &[Arg::Value(&arg)]).unwrap(), &[int(Int::U8, 9), Scalar::Bool(true), int(Int::U64, 7), int(Int::U32, 9)]);
    // main's frame follows its result's four slots and its u8: the Option written by `none` over `Some(9)`
    assert_eq!(m.memory()[5..7], [int(Int::Isize, 0), Scalar::Uninit], "the callee's None whole in the caller's place");
}

#[test]
fn an_enum_s_variant_is_written_with_a_discriminant_of_its_own_type() {
    // #[repr(u8)] enum E { A = 0, B = 1 }; fn set(e: &mut E) { *e = E::B }
    let mut b = Build::default();
    let unit = b.unit();
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::U8,
        variants: vec![Variant { name: "A".into(), discriminant: 0, fields: vec![] }, Variant { name: "B".into(), discriminant: 1, fields: vec![] }],
    });
    let re = b.reference(e, true);
    let set = b.function(Function {
        name: "set".into(),
        params: 1,
        locals: locals(&[unit, re]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref]), Value::Aggregate(e, Some(1), vec![]), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[e]).unwrap();
    m.set_region(RegionId(0), &[int(Int::U8, 0)]).unwrap();
    m.call(set, &[Arg::Ref(RegionId(0))]).unwrap();
    assert_eq!(m.region(RegionId(0)).unwrap(), &[int(Int::U8, 1)]);
}

/// `fn f(x: u8) -> u8 { x + 255 }` with MIR's checked addition and assertion: an overflow for any `x` but 0.
fn overflow() -> (Program, FunctionId) {
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let pair = b.tuple(&[u8t, boolean]);
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, u8t, pair]),
        blocks: vec![
            block(
                vec![assign(place(2), Value::Checked(BinOp::Add, copy(1), konst(u8t, vec![int(Int::U8, 255)])), 1)],
                Terminator::Assert { cond: copy_at(proj(2, vec![Projection::Field(1)])), expected: false, check: Check::Overflow(BinOp::Add), next: BlockId(1), at: at(1) },
            ),
            block(vec![assign(place(0), Value::Use(copy_at(proj(2, vec![Projection::Field(0)]))), 2)], ret()),
        ],
    });
    (b.program, f)
}

#[test]
fn a_copy_of_a_failed_machine_stands_where_it_failed_and_a_failed_entry_shows_no_result() {
    let (program, f) = overflow();
    let mut m = Machine::load(program, &[]).unwrap();
    let zero = [int(Int::U8, 0)];
    assert_eq!(m.call(f, &[Arg::Value(&zero)]).unwrap(), &[int(Int::U8, 255)]);
    let one = [int(Int::U8, 1)];
    let Err(CallError::Program(e)) = m.call(f, &[Arg::Value(&one)]) else { panic!("an overflow") };
    assert_eq!(m.result(), &[] as &[Scalar], "the failed entry gave no result, nor shows the one before");
    let mut copy = m.clone();
    assert_eq!(copy.run(0), Err(e), "a copy of a failed machine, a budget of zero");
    assert_eq!(copy.step(), Err(e));
    assert_eq!((copy.steps(), copy.position(), copy.memory(), copy.running()), (m.steps(), m.position(), m.memory(), true));
    copy.abort();
    assert_eq!(m.step(), Err(e), "and the original, its copy aborted, still failed");
}

#[test]
fn a_value_never_written_is_an_error_wherever_an_instruction_reads_it() {
    // each function reads its local 1, never written, in one way; the error names that instruction
    let mut b = Build::default();
    let (u8t, boolean, usize_t, f32t, d, unit) = (b.int(Int::U8), b.bool(), b.int(Int::Usize), b.ty(Type::F32), b.ty(Type::Duration), b.unit());
    let arr = b.ty(Type::Array { element: u8t, len: 2 });
    let opt = b.option(u8t);
    let (ru8, sopt, isize_t) = (b.reference(u8t, true), b.reference(opt, false), b.int(Int::Isize));
    let one_statement = |b: &mut Build, name: &str, types: &[TypeId], value: Value| {
        b.function(Function { name: name.into(), params: 0, locals: locals(types), blocks: vec![block(vec![assign(place(0), value, 5)], ret())] })
    };
    let one_terminator = |b: &mut Build, name: &str, types: &[TypeId], t: Terminator| {
        b.function(Function { name: name.into(), params: 0, locals: locals(types), blocks: vec![block(vec![], t), block(vec![], ret())] })
    };
    let mut cases = vec![
        (one_statement(&mut b, "binary", &[u8t, u8t], Value::Binary(BinOp::Add, copy(1), konst(u8t, vec![int(Int::U8, 1)]))), Some(0)),
        (one_statement(&mut b, "unary", &[u8t, u8t], Value::Unary(UnOp::Not, copy(1))), Some(0)),
        (one_statement(&mut b, "cast", &[f32t, u8t], Value::Cast(copy(1), f32t)), Some(0)),
        (one_statement(&mut b, "discriminant", &[isize_t, opt], Value::Discriminant(place(1))), Some(0)),
        (one_statement(&mut b, "deref", &[u8t, ru8], Value::Use(copy_at(proj(1, vec![Projection::Deref])))), Some(0)),
        (one_statement(&mut b, "index", &[u8t, usize_t, arr], Value::Use(copy_at(proj(2, vec![Projection::Index(Local(1))])))), Some(0)),
        (one_terminator(&mut b, "switch", &[unit, boolean], Terminator::SwitchInt { value: copy(1), targets: vec![], otherwise: BlockId(1), at: at(5) }), None),
        (one_terminator(&mut b, "assert", &[unit, boolean], Terminator::Assert { cond: copy(1), expected: true, check: Check::Bounds, next: BlockId(1), at: at(5) }), None),
    ];
    let core = |b: &mut Build, name: &str, op: CoreOp, types: &[TypeId], args: Vec<Operand>| {
        let t = Terminator::Call { callee: Callee::Core(op), args, dest: place(0), next: BlockId(1), at: at(5) };
        b.function(Function { name: name.into(), params: 0, locals: locals(types), blocks: vec![block(vec![], t), block(vec![], ret())] })
    };
    cases.push((core(&mut b, "saturating", CoreOp::SaturatingAdd(Int::U8), &[u8t, u8t], vec![copy(1), copy(1)]), None));
    cases.push((core(&mut b, "duration", CoreOp::DurationMin, &[d, d], vec![copy(1), copy(1)]), None));
    cases.push((core(&mut b, "is_none", CoreOp::OptionIsNone, &[boolean, sopt], vec![copy(1)]), None));
    let mut m = Machine::load(b.program, &[]).unwrap();
    for (f, statement) in cases {
        let name = m.program().functions[f.0 as usize].name.clone();
        let got = m.call(f, &[]).map(|r| r.to_vec());
        assert_eq!(got, Err(CallError::Program(ProgramError { function: f, block: BlockId(0), statement, at: at(5), kind: ErrorKind::Uninit })), "{name}");
        m.abort();
    }
}

#[test]
fn a_duration_read_through_another_variant_is_an_internal_error_and_never_a_panic() {
    // enum E { A(Duration), B(u64, u32) }; let e = E::B(u64::MAX, 2_000_000_000); (e as A).0.min(...)
    let mut b = Build::default();
    let (u64t, u32t, d) = (b.int(Int::U64), b.int(Int::U32), b.ty(Type::Duration));
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![Field { name: "0".into(), ty: d }] },
            Variant { name: "B".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: u64t }, Field { name: "1".into(), ty: u32t }] },
        ],
    });
    let f = b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[d, e, d]),
        blocks: vec![
            block(
                vec![
                    assign(place(1), Value::Aggregate(e, Some(1), vec![konst(u64t, vec![int(Int::U64, u64::MAX as i128)]), konst(u32t, vec![int(Int::U32, 2_000_000_000)])]), 1),
                    assign(place(2), Value::Use(copy_at(proj(1, vec![Projection::Downcast(0), Projection::Field(0)]))), 2),
                ],
                Terminator::Call { callee: Callee::Core(CoreOp::DurationMin), args: vec![copy(2), copy(2)], dest: place(0), next: BlockId(1), at: at(3) },
            ),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(
        m.call(f, &[]),
        Err(CallError::Program(ProgramError {
            function: f,
            block: BlockId(0),
            statement: None,
            at: at(3),
            kind: ErrorKind::Internal("a duration's slots are not a duration"),
        }))
    );
}

#[test]
fn a_callee_large_in_slots_and_another_deep_in_frames_both_have_room() {
    // main calls big (one frame of 65 slots) and a -> b -> c (three frames of few slots): the stack needs four
    // frames, though the callee deepest in slots is one frame deep
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let arr = b.ty(Type::Array { element: u8t, len: 64 });
    let c8 = |v| konst(u8t, vec![int(Int::U8, v)]);
    let call = |f, dest, next| Terminator::Call { callee: Callee::Function(f), args: vec![], dest: place(dest), next: BlockId(next), at: at(1) };
    let big = b.function(Function { name: "big".into(), params: 0, locals: locals(&[u8t, arr]), blocks: vec![block(vec![assign(place(0), Value::Use(c8(1)), 1)], ret())] });
    let c = b.function(Function { name: "c".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(c8(3)), 2)], ret())] });
    let pass = |b: &mut Build, name: &str, g| {
        b.function(Function {
            name: name.into(),
            params: 0,
            locals: locals(&[u8t, u8t]),
            blocks: vec![block(vec![], call(g, 1, 1)), block(vec![assign(place(0), Value::Use(copy(1)), 3)], ret())],
        })
    };
    let bb = pass(&mut b, "b", c);
    let a = pass(&mut b, "a", bb);
    let main = b.function(Function {
        name: "main".into(),
        params: 0,
        locals: locals(&[u8t, u8t, u8t]),
        blocks: vec![block(vec![], call(big, 1, 1)), block(vec![], call(a, 2, 2)), block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(1), copy(2)), 4)], ret())],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(m.call(main, &[]).unwrap(), &[int(Int::U8, 4)], "big() + a(), a four frames deep");
    let mut copy = m.clone();
    assert_eq!(copy.call(main, &[]).unwrap(), &[int(Int::U8, 4)], "and in a copy");
}

#[test]
fn a_variant_s_field_after_a_field_of_two_slots_is_read_where_it_was_written() {
    // enum E { A, B(Duration, u8) }: B's u8 follows a field of two slots
    let mut b = Build::default();
    let (u8t, d) = (b.int(Int::U8), b.ty(Type::Duration));
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant { name: "B".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: d }, Field { name: "1".into(), ty: u8t }] },
        ],
    });
    // fn f(x: u8) -> u8 { let e = E::B(Duration::new(7, 9), x); (e as B).1 }
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, u8t, e]),
        blocks: vec![block(
            vec![
                assign(place(2), Value::Aggregate(e, Some(1), vec![konst(d, vec![int(Int::U64, 7), int(Int::U32, 9)]), copy(1)]), 1),
                assign(place(0), Value::Use(copy_at(proj(2, vec![Projection::Downcast(1), Projection::Field(1)]))), 2),
            ],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[e]).unwrap();
    let arg = [int(Int::U8, 42)];
    assert_eq!(m.call(f, &[Arg::Value(&arg)]).unwrap(), &[int(Int::U8, 42)], "(e as B).1");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 1), int(Int::U64, 7), int(Int::U32, 9), int(Int::U8, 42)]), Ok(()), "E::B as a region's slots");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 1), int(Int::U8, 42), int(Int::U64, 7), int(Int::U32, 9)]), Err(HostError::Slots), "the fields the other way round");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::U8, 1), int(Int::U64, 7), int(Int::U32, 9), int(Int::U8, 42)]), Err(HostError::Slots), "a discriminant of another integer type");
}

#[test]
fn a_duration_whose_seconds_are_not_a_u64_and_a_structure_of_other_slots_are_refused_by_the_host_s_calls() {
    let mut b = Build::default();
    let (unit, d, u8t, u16t) = (b.unit(), b.ty(Type::Duration), b.int(Int::U8), b.int(Int::U16));
    let pair = b.tuple(&[u8t, u16t]);
    let takes_d = b.function(Function { name: "d".into(), params: 1, locals: locals(&[unit, d]), blocks: vec![block(vec![], ret())] });
    let takes_pair = b.function(Function { name: "p".into(), params: 1, locals: locals(&[unit, pair]), blocks: vec![block(vec![], ret())] });
    let mut m = Machine::load(b.program, &[d]).unwrap();
    let u32_seconds = [int(Int::U32, 5), int(Int::U32, 0)];
    assert_eq!(m.set_region(RegionId(0), &u32_seconds), Err(HostError::Slots), "seconds of a u32");
    assert_eq!(m.start(takes_d, &[Arg::Value(&u32_seconds)]), Err(HostError::Arguments));
    assert_eq!(m.start(takes_pair, &[Arg::Value(&[int(Int::U16, 1), int(Int::U8, 2)])]), Err(HostError::Arguments), "the fields the other way round");
    assert_eq!(m.start(takes_pair, &[Arg::Value(&[int(Int::U8, 1), int(Int::U16, 2), int(Int::U8, 3)])]), Err(HostError::Arguments), "a slot too many");
    assert_eq!(m.start(takes_pair, &[Arg::Value(&[int(Int::U8, 1), int(Int::U16, 2)])]), Ok(()));
}
