//! The machine runs a program as MIR means it: blocks and their terminators, calls by value and by reference,
//! places into structures, arrays and enums, a program's failures as errors that name where, one instruction a
//! step, and a run stopped at any step going on to the same end.
mod common;
use common::*;
use core::time::Duration;
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

/// `enum E { A = 0, B(u8, bool) = 5, C(i64) = -3 }`: its variants listed out of their discriminants' order.
fn out_of_order(b: &mut Build) -> TypeId {
    let (u8t, boolean, i64t) = (b.int(Int::U8), b.bool(), b.int(Int::I64));
    b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant { name: "B".into(), discriminant: 5, fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: boolean }] },
            Variant { name: "C".into(), discriminant: -3, fields: vec![Field { name: "0".into(), ty: i64t }] },
        ],
    })
}

fn out_of_order_values() -> Vec<(&'static str, Vec<Scalar>, bool)> {
    let u = Scalar::Uninit;
    vec![
        ("A", vec![int(Int::Isize, 0), u, u], true),
        ("B(7, true)", vec![int(Int::Isize, 5), int(Int::U8, 7), Scalar::Bool(true)], true),
        ("C(-9)", vec![int(Int::Isize, -3), int(Int::I64, -9), u], true),
        ("C with its payload unwritten", vec![int(Int::Isize, -3), u, u], false),
        ("A with B's payload", vec![int(Int::Isize, 0), int(Int::U8, 7), Scalar::Bool(true)], false),
        ("no variant, below the least", vec![int(Int::Isize, -4), u, u], false),
        ("no variant, between two", vec![int(Int::Isize, 2), u, u], false),
        ("no variant, past the greatest", vec![int(Int::Isize, 6), u, u], false),
    ]
}

#[test]
fn an_enum_listed_out_of_order_takes_each_of_its_values_and_no_other_as_a_region_a_constant_and_an_argument() {
    let mut b = Build::default();
    let e = out_of_order(&mut b);
    let mut m = Machine::load(b.program, &[e]).unwrap();
    for (name, slots, good) in out_of_order_values() {
        assert_eq!(m.set_region(RegionId(0), &slots), if good { Ok(()) } else { Err(HostError::Slots) }, "region {name}");
    }
    for (name, slots, good) in out_of_order_values() {
        let mut b = Build::default();
        let e = out_of_order(&mut b);
        b.function(Function { name: "f".into(), params: 0, locals: locals(&[e]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(e, slots.clone())), 1)], ret())] });
        let got = Machine::load(b.program, &[]).map(|_| ());
        if good {
            assert_eq!(got, Ok(()), "constant {name}");
        } else {
            assert!(matches!(got, Err(LoadError::Instruction { reason: "a constant whose slots are not its type's", .. })), "constant {name}: {got:?}");
        }
        let mut b = Build::default();
        let e = out_of_order(&mut b);
        let f = b.function(Function { name: "g".into(), params: 1, locals: locals(&[e, e]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
        let mut m = Machine::load(b.program, &[]).unwrap();
        let got = m.call(f, &[Arg::Value(&slots)]).map(|r| r.to_vec());
        assert_eq!(got, if good { Ok(slots.clone()) } else { Err(CallError::Host(HostError::Arguments)) }, "argument {name}");
    }
    // an Option's discriminant below None is no variant either
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let opt = b.option(u8t);
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, -1), Scalar::Uninit]), Err(HostError::Slots), "below None");
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, 0), Scalar::Uninit]), Ok(()), "None");
}

#[test]
fn a_type_of_no_slot_given_a_slot_is_refused_as_a_constant_a_region_and_an_argument_never_by_a_panic() {
    let one = [Scalar::Bool(true)];
    let mut b = Build::default();
    let (unit, boolean) = (b.unit(), b.bool());
    b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[boolean, unit, boolean]),
        blocks: vec![block(
            vec![
                assign(place(2), Value::Use(konst(boolean, vec![Scalar::Bool(false)])), 1),
                assign(place(1), Value::Use(konst(unit, one.to_vec())), 2),
                assign(place(0), Value::Use(copy(2)), 3),
            ],
            ret(),
        )],
    });
    let got = Machine::load(b.program, &[]).map(|_| ());
    assert!(matches!(got, Err(LoadError::Instruction { reason: "a constant whose slots are not its type's", .. })), "a constant: {got:?}");
    let mut b = Build::default();
    let (unit, boolean) = (b.unit(), b.bool());
    let mut m = Machine::load(b.program, &[unit, boolean]).unwrap();
    m.set_region(RegionId(1), &[Scalar::Bool(false)]).unwrap();
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.set_region(RegionId(0), &one)));
    assert_eq!(got.ok(), Some(Err(HostError::Slots)), "a region: refused, never a panic");
    assert_eq!(m.region(RegionId(1)), Some(&[Scalar::Bool(false)][..]), "the region beside it untouched");
    assert_eq!(m.set_region(RegionId(0), &[]), Ok(()), "and given none, taken");
    let mut b = Build::default();
    let (unit, boolean) = (b.unit(), b.bool());
    let f = b.function(Function { name: "g".into(), params: 2, locals: locals(&[boolean, unit, boolean]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(2)), 1)], ret())] });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let got = m.call(f, &[Arg::Value(&one), Arg::Value(&[Scalar::Bool(false)])]).map(|r| r.to_vec());
    assert_eq!(got, Err(CallError::Host(HostError::Arguments)), "an argument");
    assert_eq!(m.call(f, &[Arg::Value(&[]), Arg::Value(&[Scalar::Bool(false)])]).unwrap(), &[Scalar::Bool(false)]);
}

#[test]
fn an_array_of_elements_of_two_slots_is_indexed_at_its_element_s_slots_and_takes_its_values_and_no_other() {
    // fn f(i: usize) -> ([(u8, u16); 3], (u8, u16), (u8, u16)) {
    //     let mut a = [(1, 2), (3, 4), (5, 6)]; let x = a[i]; let y = a[1]; a[i].1 = 9; (a, x, y) }
    let mut b = Build::default();
    let (u8t, u16t, usize_t) = (b.int(Int::U8), b.int(Int::U16), b.int(Int::Usize));
    let pair = b.tuple(&[u8t, u16t]);
    let arr = b.ty(Type::Array { element: pair, len: 3 });
    let out = b.tuple(&[arr, pair, pair]);
    let p = |x: i128, y: i128| konst(pair, vec![int(Int::U8, x), int(Int::U16, y)]);
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[out, usize_t, arr, pair, pair]),
        blocks: vec![block(
            vec![
                assign(place(2), Value::Aggregate(arr, None, vec![p(1, 2), p(3, 4), p(5, 6)]), 1),
                assign(place(3), Value::Use(copy_at(proj(2, vec![Projection::Index(Local(1))]))), 2),
                assign(place(4), Value::Use(copy_at(proj(2, vec![Projection::ConstantIndex(1)]))), 3),
                assign(proj(2, vec![Projection::Index(Local(1)), Projection::Field(1)]), Value::Use(konst(u16t, vec![int(Int::U16, 9)])), 4),
                assign(place(0), Value::Aggregate(out, None, vec![copy(2), copy(3), copy(4)]), 5),
            ],
            ret(),
        )],
    });
    let g = b.function(Function { name: "g".into(), params: 1, locals: locals(&[arr, arr]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let program = b.program;
    let mut m = Machine::load(program.clone(), &[arr]).unwrap();
    let (a, c) = (|v| int(Int::U8, v), |v| int(Int::U16, v));
    for (i, x, written) in [(0, (1, 2), [(1, 9), (3, 4), (5, 6)]), (2, (5, 6), [(1, 2), (3, 4), (5, 9)])] {
        let arg = [int(Int::Usize, i)];
        let mut want = vec![];
        for (u, v) in written {
            want.extend([a(u), c(v)]);
        }
        want.extend([a(x.0), c(x.1), a(3), c(4)]);
        assert_eq!(m.call(f, &[Arg::Value(&arg)]).unwrap(), want.as_slice(), "a[{i}] read, a[1] read, a[{i}].1 written");
    }
    // the array's values and no other, as a region, a constant and an argument
    let good = vec![a(1), c(2), a(3), c(4), a(5), c(6)];
    let wrong = vec![a(1), c(2), a(3), c(4), a(5), a(6)];
    assert_eq!(m.set_region(RegionId(0), &good), Ok(()));
    assert_eq!(m.set_region(RegionId(0), &wrong), Err(HostError::Slots), "the last element's u16 a u8");
    assert_eq!(m.call(g, &[Arg::Value(&good)]).unwrap(), good.as_slice());
    assert_eq!(m.start(g, &[Arg::Value(&wrong)]), Err(HostError::Arguments));
    for (slots, fits_) in [(good, true), (wrong, false)] {
        let mut p = program.clone();
        p.functions.push(Function { name: "k".into(), params: 0, locals: locals(&[arr]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(arr, slots)), 1)], ret())] });
        let got = Machine::load(p, &[]).map(|_| ());
        assert_eq!(got.is_ok(), fits_, "a constant: {got:?}");
    }
}

#[test]
fn fields_of_no_slot_before_those_with_one_are_passed_by_at_their_own_offsets() {
    // S { a: (), b: u8, c: bool }, T { a: (), b: u8, c: u8 }, enum E { A((), u8, bool) = 0 }
    let mut b = Build::default();
    let (unit, u8t, boolean) = (b.unit(), b.int(Int::U8), b.bool());
    let f = |n: &str, t| Field { name: n.into(), ty: t };
    let s = b.ty(Type::Struct { name: "S".into(), fields: vec![f("a", unit), f("b", u8t), f("c", boolean)] });
    let t = b.ty(Type::Struct { name: "T".into(), fields: vec![f("a", unit), f("b", u8t), f("c", u8t)] });
    let e = b.ty(Type::Enum { name: "E".into(), discriminant: Int::U8, variants: vec![Variant { name: "A".into(), discriminant: 0, fields: vec![f("0", unit), f("1", u8t), f("2", boolean)] }] });
    let read_c = b.function(Function { name: "c".into(), params: 1, locals: locals(&[boolean, s]), blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Field(2)]))), 1)], ret())] });
    let program = b.program;
    let mut m = Machine::load(program.clone(), &[s, t, e]).unwrap();
    for (name, region, slots, good) in [
        ("S(7, true)", 0, vec![int(Int::U8, 7), Scalar::Bool(true)], true),
        ("S(7, 8)", 0, vec![int(Int::U8, 7), int(Int::U8, 8)], false),
        ("T(7, true)", 1, vec![int(Int::U8, 7), Scalar::Bool(true)], false),
        ("T(7, 8)", 1, vec![int(Int::U8, 7), int(Int::U8, 8)], true),
        ("E::A(7, true)", 2, vec![int(Int::U8, 0), int(Int::U8, 7), Scalar::Bool(true)], true),
        ("E::A(7, 8)", 2, vec![int(Int::U8, 0), int(Int::U8, 7), int(Int::U8, 8)], false),
    ] {
        assert_eq!(m.set_region(RegionId(region), &slots), if good { Ok(()) } else { Err(HostError::Slots) }, "{name}");
        let ty = [s, t, e][region as usize];
        let mut p = program.clone();
        p.functions.push(Function { name: "k".into(), params: 0, locals: locals(&[ty]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(ty, slots.clone())), 1)], ret())] });
        assert_eq!(Machine::load(p, &[]).is_ok(), good, "a constant {name}");
    }
    let arg = [int(Int::U8, 7), Scalar::Bool(true)];
    assert_eq!(m.call(read_c, &[Arg::Value(&arg)]).unwrap(), &[Scalar::Bool(true)], "S's field c read past its field of no slot");
}

#[test]
fn a_duration_outside_its_integers_ranges_is_refused_as_a_region_an_argument_and_a_constant_and_a_refused_region_keeps_its_value() {
    let bad = [
        ("nanoseconds of -1", [int(Int::U64, 0), Scalar::Int(Int::U32, -1)]),
        ("seconds of -1", [Scalar::Int(Int::U64, -1), int(Int::U32, 0)]),
        ("seconds of 2^64", [Scalar::Int(Int::U64, 1 << 64), int(Int::U32, 0)]),
        ("nanoseconds of 2^32", [int(Int::U64, 0), Scalar::Int(Int::U32, 1 << 32)]),
    ];
    let mut b = Build::default();
    let (u8t, d) = (b.int(Int::U8), b.ty(Type::Duration));
    let opt = b.option(u8t);
    let f = b.function(Function {
        name: "min".into(),
        params: 2,
        locals: locals(&[d, d, d]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::DurationMin), args: vec![copy(1), copy(2)], dest: place(0), next: BlockId(1), at: at(1) }),
            block(vec![], ret()),
        ],
    });
    let program = b.program;
    let mut m = Machine::load(program.clone(), &[d, opt]).unwrap();
    let kept = [int(Int::U64, 3), int(Int::U32, 5)];
    m.set_region(RegionId(0), &kept).unwrap();
    let one = [int(Int::U64, 1), int(Int::U32, 0)];
    for (name, slots) in bad {
        assert_eq!(m.set_region(RegionId(0), &slots), Err(HostError::Slots), "region: {name}");
        assert_eq!(m.region(RegionId(0)), Some(&kept[..]), "the region keeps its value: {name}");
        assert_eq!(m.start(f, &[Arg::Value(&slots), Arg::Value(&one)]), Err(HostError::Arguments), "argument: {name}");
        let mut p = program.clone();
        p.functions.push(Function { name: "k".into(), params: 0, locals: locals(&[d]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(d, slots.to_vec())), 1)], ret())] });
        assert!(matches!(Machine::load(p, &[]), Err(LoadError::Instruction { reason: "a constant whose slots are not its type's", .. })), "constant: {name}");
    }
    let some = [int(Int::Isize, 1), int(Int::U8, 9)];
    m.set_region(RegionId(1), &some).unwrap();
    assert_eq!(m.set_region(RegionId(1), &[int(Int::Isize, 0), int(Int::U8, 5)]), Err(HostError::Slots));
    assert_eq!(m.region(RegionId(1)), Some(&some[..]), "a None with a payload refused, the Some kept");
}

#[test]
fn a_return_value_is_unwritten_at_every_start_and_every_call() {
    // fn g() -> u8 { 7 }  fn f() -> u8 { _0 = _0 + 1 }  fn main() -> u8 { g(); f() }
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let g = b.function(Function { name: "g".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 7)])), 1)], ret())] });
    let f = b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[u8t, u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(0), konst(u8t, vec![int(Int::U8, 1)])), 2)], ret())],
    });
    let main = b.function(Function {
        name: "main".into(),
        params: 0,
        locals: locals(&[u8t, u8t]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Function(g), args: vec![], dest: place(1), next: BlockId(1), at: at(3) }),
            block(vec![], Terminator::Call { callee: Callee::Function(f), args: vec![], dest: place(0), next: BlockId(2), at: at(4) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(m.call(g, &[]).unwrap(), &[int(Int::U8, 7)]);
    for entry in [f, main] {
        let got = m.call(entry, &[]).map(|r| r.to_vec());
        assert!(matches!(got, Err(CallError::Program(ProgramError { kind: ErrorKind::Uninit, function, .. })) if function == f), "{got:?}");
        m.abort();
    }
}

// Round 7's shapes (the critic's probe, kept): what real MIR has, each with the exact slots a program gets.

fn dur(d: Duration) -> Vec<Scalar> {
    vec![int(Int::U64, d.as_secs() as i128), int(Int::U32, d.subsec_nanos() as i128)]
}

/// `enum E { A, B(u8, S) }`, `struct S { d: Duration, c: [(u8, u16); 3] }`: an array inside a structure inside a
/// variant that is not the last; `enum F { P(u8, u16), Q(Duration) }`: a variant of two fields before the last.
#[test]
fn an_index_into_an_array_inside_a_structure_inside_a_variant_reads_and_writes_its_own_slots() {
    let mut b = Build::default();
    let (u8t, u16t, usize_t, d) = (b.int(Int::U8), b.int(Int::U16), b.int(Int::Usize), b.ty(Type::Duration));
    let pair = b.tuple(&[u8t, u16t]);
    let arr = b.ty(Type::Array { element: pair, len: 3 });
    let s = b.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "d".into(), ty: d }, Field { name: "c".into(), ty: arr }] });
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant { name: "B".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: s }] },
            Variant { name: "C".into(), discriminant: 2, fields: vec![Field { name: "0".into(), ty: u16t }] },
        ],
    });
    let rpair = b.reference(pair, true);
    let spair = b.reference(pair, false);
    let unit = b.unit();
    let out = b.tuple(&[e, u16t, u16t]);
    // fn set(p: &mut (u8, u16)) { (*p).1 = (*p).1 + 100 }
    let set = b.function(Function {
        name: "set".into(),
        params: 1,
        locals: locals(&[unit, rpair]),
        blocks: vec![block(
            vec![assign(
                proj(1, vec![Projection::Deref, Projection::Field(1)]),
                Value::Binary(BinOp::Add, copy_at(proj(1, vec![Projection::Deref, Projection::Field(1)])), konst(u16t, vec![int(Int::U16, 100)])),
                1,
            )],
            ret(),
        )],
    });
    // fn f(e: E, i: usize) -> (E, u16, u16) {
    //   let x = ((e as B).1.c[i]).1; ((e as B).1.c[i]).1 = 7; set(&mut (e as B).1.c[2]); let r = &(e as B).1.c[i];
    //   (e, x, (*r).1) }
    let path = |last: Vec<Projection>| {
        let mut p = vec![Projection::Downcast(1), Projection::Field(1), Projection::Field(1)];
        p.extend(last);
        proj(1, p)
    };
    let f = b.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[out, e, usize_t, u16t, rpair, unit, spair, u16t]),
        blocks: vec![
            block(
                vec![
                    assign(place(3), Value::Use(copy_at(path(vec![Projection::Index(Local(2)), Projection::Field(1)]))), 1),
                    assign(path(vec![Projection::Index(Local(2)), Projection::Field(1)]), Value::Use(konst(u16t, vec![int(Int::U16, 7)])), 2),
                    assign(place(4), Value::Ref(path(vec![Projection::ConstantIndex(2)]), true), 3),
                ],
                Terminator::Call { callee: Callee::Function(set), args: vec![copy(4)], dest: place(5), next: BlockId(1), at: at(3) },
            ),
            block(
                vec![
                    assign(place(6), Value::Ref(path(vec![Projection::Index(Local(2))]), false), 4),
                    assign(place(7), Value::Use(copy_at(proj(6, vec![Projection::Deref, Projection::Field(1)]))), 4),
                    assign(place(0), Value::Aggregate(out, None, vec![copy(1), copy(3), copy(7)]), 5),
                ],
                ret(),
            ),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let (a, c) = (|v| int(Int::U8, v), |v| int(Int::U16, v));
    // E::B(9, S { d: 3.5 s, c: [(1, 2), (3, 4), (5, 6)] }): discriminant, u8, d's two slots, six slots of c
    let mut e_b = vec![int(Int::Isize, 1), a(9)];
    e_b.extend(dur(Duration::from_millis(3500)));
    e_b.extend([a(1), c(2), a(3), c(4), a(5), c(6)]);
    for i in 0..3i128 {
        let got = m.call(f, &[Arg::Value(&e_b), Arg::Value(&[int(Int::Usize, i)])]).unwrap().to_vec();
        let mut want_e = e_b.clone();
        let x = [2, 4, 6][i as usize];
        want_e[5 + 2 * i as usize] = c(7);
        let last = want_e[9];
        let Scalar::Int(_, l) = last else { panic!() };
        want_e[9] = c(l + 100);
        let mut want = want_e.clone();
        want.extend([c(x), want_e[5 + 2 * i as usize]]);
        assert_eq!(got, want, "i = {i}");
    }
}

/// `enum F { P(u8, u16), Q(Duration) }`: P is not the last variant and its second field sits at slot 2.
#[test]
fn a_variant_before_the_last_reads_its_second_field_where_it_was_written() {
    let mut b = Build::default();
    let (u8t, u16t, d) = (b.int(Int::U8), b.int(Int::U16), b.ty(Type::Duration));
    let fty = b.ty(Type::Enum {
        name: "F".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "P".into(), discriminant: 0, fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: u16t }] },
            Variant { name: "Q".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: d }] },
        ],
    });
    // fn g(x: F) -> u16 { (x as P).1 }
    let g = b.function(Function {
        name: "g".into(),
        params: 1,
        locals: locals(&[u16t, fty]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Downcast(0), Projection::Field(1)]))), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let arg = [int(Int::Isize, 0), int(Int::U8, 5), int(Int::U16, 600)];
    assert_eq!(m.call(g, &[Arg::Value(&arg)]).unwrap(), &[int(Int::U16, 600)]);
}

/// `p = (p.1, p.0)`, `a = [a[2], a[1], a[0]]` and `*r = ((*r).1, (*r).0)`: a value built from the place it is written to.
#[test]
fn an_assignment_whose_value_reads_its_own_place_writes_the_value_read_before() {
    let mut b = Build::default();
    let u16t = b.int(Int::U16);
    let pair = b.tuple(&[u16t, u16t]);
    let arr = b.ty(Type::Array { element: pair, len: 3 });
    let rpair = b.reference(pair, true);
    let unit = b.unit();
    let out = b.tuple(&[pair, arr]);
    let swap = b.function(Function {
        name: "swap".into(),
        params: 1,
        locals: locals(&[unit, rpair]),
        blocks: vec![block(
            vec![assign(
                proj(1, vec![Projection::Deref]),
                Value::Aggregate(pair, None, vec![copy_at(proj(1, vec![Projection::Deref, Projection::Field(1)])), copy_at(proj(1, vec![Projection::Deref, Projection::Field(0)]))]),
                1,
            )],
            ret(),
        )],
    });
    let f = b.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[out, pair, arr, rpair, unit]),
        blocks: vec![
            block(
                vec![
                    assign(place(1), Value::Aggregate(pair, None, vec![copy_at(proj(1, vec![Projection::Field(1)])), copy_at(proj(1, vec![Projection::Field(0)]))]), 1),
                    assign(
                        place(2),
                        Value::Aggregate(arr, None, vec![
                            copy_at(proj(2, vec![Projection::ConstantIndex(2)])),
                            copy_at(proj(2, vec![Projection::ConstantIndex(1)])),
                            copy_at(proj(2, vec![Projection::ConstantIndex(0)])),
                        ]),
                        2,
                    ),
                    assign(place(3), Value::Ref(proj(2, vec![Projection::ConstantIndex(0)]), true), 3),
                ],
                Terminator::Call { callee: Callee::Function(swap), args: vec![copy(3)], dest: place(4), next: BlockId(1), at: at(3) },
            ),
            block(vec![assign(place(0), Value::Aggregate(out, None, vec![copy(1), copy(2)]), 4)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let c = |v| int(Int::U16, v);
    let got = m.call(f, &[Arg::Value(&[c(1), c(2)]), Arg::Value(&[c(10), c(11), c(20), c(21), c(30), c(31)])]).unwrap().to_vec();
    assert_eq!(got, vec![c(2), c(1), c(31), c(30), c(20), c(21), c(10), c(11)]);
}

/// `fn put(a: &mut [(u8, u16); 3], i: usize, v: u16) { (*a)[i].1 = v }` called with a reference to the caller's array,
/// and `[[u8; 3]; 2]` indexed twice, and `[Option<Duration>; 3]`'s element read as `Some`.
#[test]
fn a_callee_writes_the_caller_s_array_through_a_reference_and_nested_indices_reach_their_slots() {
    let mut b = Build::default();
    let (u8t, u16t, usize_t, d) = (b.int(Int::U8), b.int(Int::U16), b.int(Int::Usize), b.ty(Type::Duration));
    let pair = b.tuple(&[u8t, u16t]);
    let arr = b.ty(Type::Array { element: pair, len: 3 });
    let rarr = b.reference(arr, true);
    let unit = b.unit();
    let put = b.function(Function {
        name: "put".into(),
        params: 3,
        locals: locals(&[unit, rarr, usize_t, u16t]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref, Projection::Index(Local(2)), Projection::Field(1)]), Value::Use(copy(3)), 1)], ret())],
    });
    let caller = b.function(Function {
        name: "caller".into(),
        params: 2,
        locals: locals(&[arr, usize_t, u16t, arr, rarr, unit]),
        blocks: vec![
            block(
                vec![
                    assign(place(3), Value::Use(konst(arr, vec![int(Int::U8, 1), int(Int::U16, 2), int(Int::U8, 3), int(Int::U16, 4), int(Int::U8, 5), int(Int::U16, 6)])), 1),
                    assign(place(4), Value::Ref(place(3), true), 2),
                ],
                Terminator::Call { callee: Callee::Function(put), args: vec![copy(4), copy(1), copy(2)], dest: place(5), next: BlockId(1), at: at(2) },
            ),
            block(vec![assign(place(0), Value::Use(copy(3)), 3)], ret()),
        ],
    });
    let row = b.ty(Type::Array { element: u8t, len: 3 });
    let grid = b.ty(Type::Array { element: row, len: 2 });
    let get2 = b.function(Function {
        name: "get2".into(),
        params: 3,
        locals: locals(&[u8t, grid, usize_t, usize_t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Index(Local(2)), Projection::Index(Local(3))]))), 1)], ret())],
    });
    let opt = b.option(d);
    let opts = b.ty(Type::Array { element: opt, len: 3 });
    let some_at = b.function(Function {
        name: "some_at".into(),
        params: 2,
        locals: locals(&[d, opts, usize_t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Index(Local(2)), Projection::Downcast(1), Projection::Field(0)]))), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    for i in 0..3i128 {
        let got = m.call(caller, &[Arg::Value(&[int(Int::Usize, i)]), Arg::Value(&[int(Int::U16, 99)])]).unwrap().to_vec();
        let mut want = vec![int(Int::U8, 1), int(Int::U16, 2), int(Int::U8, 3), int(Int::U16, 4), int(Int::U8, 5), int(Int::U16, 6)];
        want[1 + 2 * i as usize] = int(Int::U16, 99);
        assert_eq!(got, want, "put at {i}");
    }
    let g: Vec<Scalar> = (0..6).map(|v| int(Int::U8, 10 + v)).collect();
    for i in 0..2i128 {
        for j in 0..3i128 {
            let got = m.call(get2, &[Arg::Value(&g), Arg::Value(&[int(Int::Usize, i)]), Arg::Value(&[int(Int::Usize, j)])]).unwrap().to_vec();
            assert_eq!(got, vec![int(Int::U8, 10 + 3 * i + j)], "g[{i}][{j}]");
        }
    }
    let mut o = vec![int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit];
    o.extend([int(Int::Isize, 1)]);
    o.extend(dur(Duration::new(4, 5)));
    o.extend([int(Int::Isize, 1)]);
    o.extend(dur(Duration::new(6, 7)));
    for (i, want) in [(1, Duration::new(4, 5)), (2, Duration::new(6, 7))] {
        let got = m.call(some_at, &[Arg::Value(&o), Arg::Value(&[int(Int::Usize, i)])]).unwrap().to_vec();
        assert_eq!(got, dur(want), "(o[{i}] as Some).0");
    }
}

// Round 8's (the critic's kill-tests, kept): a failed statement's position, and a budget's count.

#[test]
fn a_failed_statement_leaves_the_machine_at_that_statement() {
    // fn f() -> u8 { let y; _0 = 1; _0 = y + 1 }: the second statement fails, and the machine stands at it
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let one = || konst(u8t, vec![int(Int::U8, 1)]);
    let f = b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[u8t, u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Use(one()), 1), assign(place(0), Value::Binary(BinOp::Add, copy(1), one()), 2)], ret())],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    let err = m.call(f, &[]).unwrap_err();
    assert_eq!(err, CallError::Program(ProgramError { function: f, block: BlockId(0), statement: Some(1), at: at(2), kind: ErrorKind::Uninit }));
    assert_eq!(m.position(), Some((f, BlockId(0), 1)), "the machine stays at the failed statement");
    assert_eq!(m.clone().position(), Some((f, BlockId(0), 1)), "and so does a copy of it");
}

#[test]
fn a_budget_runs_that_many_instructions_and_no_more() {
    // fn f() -> u8 { _0 = 1; _0 = _0 + 1; ... } with nine statements, then its return
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let one = || konst(u8t, vec![int(Int::U8, 1)]);
    let mut statements = vec![assign(place(0), Value::Use(one()), 1)];
    for k in 2..=9 {
        statements.push(assign(place(0), Value::Binary(BinOp::Add, copy(0), one()), k));
    }
    let f = b.function(Function { name: "f".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(statements, ret())] });
    let mut m = Machine::load(b.program, &[]).unwrap();
    m.start(f, &[]).unwrap();
    let mut ran = 0u64;
    for budget in [1u64, 2, 3] {
        assert_eq!(m.run(budget), Ok(Step::Ran));
        ran += budget;
        assert_eq!((m.steps(), m.position()), (ran, Some((f, BlockId(0), ran as u32))), "a budget of {budget}");
    }
    // three statements and the return are left: a budget of four ends on the return, one more than that is unused
    assert_eq!(m.run(5), Ok(Step::Returned));
    assert_eq!((m.steps(), m.result()), (10, &[int(Int::U8, 9)][..]));
}

// Round 9's (the critic's probe, kept): a host region never set, read by well-formed MIR; slots of a reference or
// of the other float kind offered as values.

fn uninit_at(f: FunctionId, block: u32, statement: Option<u32>, line: u32) -> Result<Vec<Scalar>, CallError> {
    Err(CallError::Program(ProgramError { function: f, block: BlockId(block), statement, at: at(line), kind: ErrorKind::Uninit }))
}

#[test]
fn is_none_on_a_region_never_set_is_uninit_at_its_call() {
    // fn f(o: &Option<u8>) -> bool { o.is_none() }: _0 = Option::<u8>::is_none(copy _1) -> bb1
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let opt = b.option(u8t);
    let sopt = b.reference(opt, false);
    let mut fs = vec![];
    for op in [CoreOp::OptionIsNone, CoreOp::OptionIsSome] {
        fs.push(b.function(Function {
            name: format!("{op:?}"),
            params: 1,
            locals: locals(&[boolean, sopt]),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Core(op), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(4) }),
                block(vec![], ret()),
            ],
        }));
    }
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    for f in fs {
        assert_eq!(m.call(f, &[Arg::Ref(RegionId(0))]).map(|r| r.to_vec()), uninit_at(f, 0, None, 4), "{f:?} on a region never set");
        m.abort();
    }
}

#[test]
fn get_or_insert_on_a_region_never_set_is_uninit_and_writes_nothing() {
    // fn g(o: &mut Option<Duration>, now: Duration) -> Duration { *o.get_or_insert(now) }
    let mut b = Build::default();
    let d = b.ty(Type::Duration);
    let opt = b.option(d);
    let (ropt, rd) = (b.reference(opt, true), b.reference(d, true));
    let g = b.function(Function {
        name: "g".into(),
        params: 2,
        locals: locals(&[d, ropt, d, rd]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionGetOrInsert), args: vec![copy(1), copy(2)], dest: place(3), next: BlockId(1), at: at(4) }),
            block(vec![assign(place(0), Value::Use(copy_at(proj(3, vec![Projection::Deref]))), 5)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    let now = [int(Int::U64, 3), int(Int::U32, 0)];
    assert_eq!(m.call(g, &[Arg::Ref(RegionId(0)), Arg::Value(&now)]).map(|r| r.to_vec()), uninit_at(g, 0, None, 4));
    assert_eq!(m.region(RegionId(0)).unwrap(), &[Scalar::Uninit; 3][..], "the region keeps what it held");
}

#[test]
fn a_checked_addition_on_a_region_never_set_is_uninit_at_its_statement() {
    // fn h(s: &mut u8) { *s += 1 }: _2 = AddWithOverflow(copy (*_1), const 1_u8); assert(!move (_2.1: bool)) -> bb1
    let mut b = Build::default();
    let (u8t, boolean, unit) = (b.int(Int::U8), b.bool(), b.unit());
    let r = b.reference(u8t, true);
    let pair = b.tuple(&[u8t, boolean]);
    let h = b.function(Function {
        name: "h".into(),
        params: 1,
        locals: locals(&[unit, r, pair]),
        blocks: vec![
            block(
                vec![assign(place(2), Value::Checked(BinOp::Add, copy_at(proj(1, vec![Projection::Deref])), konst(u8t, vec![int(Int::U8, 1)])), 3)],
                Terminator::Assert { cond: copy_at(proj(2, vec![Projection::Field(1)])), expected: false, check: Check::Overflow(BinOp::Add), next: BlockId(1), at: at(4) },
            ),
            block(vec![assign(proj(1, vec![Projection::Deref]), Value::Use(copy_at(proj(2, vec![Projection::Field(0)]))), 5)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[u8t]).unwrap();
    assert_eq!(m.call(h, &[Arg::Ref(RegionId(0))]).map(|r| r.to_vec()), uninit_at(h, 0, Some(0), 3));
}

#[test]
fn a_reference_offered_as_a_value_is_refused_as_a_constant_a_region_and_an_argument() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let r = b.reference(u8t, false);
    // fn f(x: &u8) -> u8 { *x }, and fn k() -> u8 { let c: &u8 = <a reference constant>; *c }
    let f = b.function(Function { name: "f".into(), params: 1, locals: locals(&[u8t, r]), blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Deref]))), 1)], ret())] });
    let good = b.program.clone();
    b.function(Function {
        name: "k".into(),
        params: 0,
        locals: locals(&[u8t, r]),
        blocks: vec![block(vec![assign(place(1), Value::Use(konst(r, vec![Scalar::Ref(0)])), 1), assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Deref]))), 2)], ret())],
    });
    assert_eq!(
        Machine::load(b.program, &[u8t]).map(|_| ()),
        Err(LoadError::Instruction { function: "k".into(), block: 0, statement: Some(0), reason: "a constant whose slots are not its type's" })
    );
    let mut m = Machine::load(good, &[u8t, r]).unwrap();
    m.set_region(RegionId(0), &[int(Int::U8, 42)]).unwrap();
    assert_eq!(m.set_region(RegionId(1), &[Scalar::Ref(0)]), Err(HostError::Slots), "a region");
    assert_eq!(m.start(f, &[Arg::Value(&[Scalar::Ref(0)])]), Err(HostError::Arguments), "an argument");
}

#[test]
fn a_float_slot_of_the_other_kind_is_refused_as_a_constant_a_region_and_an_argument() {
    let mut b = Build::default();
    let (f32t, f64t) = (b.ty(Type::F32), b.ty(Type::F64));
    // fn f(x: f32) -> f32 { x }, fn g(x: f64) -> f64 { x }
    let f = b.function(Function { name: "f".into(), params: 1, locals: locals(&[f32t, f32t]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let g = b.function(Function { name: "g".into(), params: 1, locals: locals(&[f64t, f64t]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let good = b.program.clone();
    let mut m = Machine::load(good.clone(), &[f32t, f64t]).unwrap();
    assert_eq!(m.set_region(RegionId(0), &[Scalar::F64(1.5)]), Err(HostError::Slots), "an f64 in an f32 region");
    assert_eq!(m.set_region(RegionId(1), &[Scalar::F32(1.5)]), Err(HostError::Slots), "an f32 in an f64 region");
    assert_eq!(m.start(f, &[Arg::Value(&[Scalar::F64(1.5)])]), Err(HostError::Arguments), "an f64 for an f32");
    assert_eq!(m.start(g, &[Arg::Value(&[Scalar::F32(1.5)])]), Err(HostError::Arguments), "an f32 for an f64");
    for (ty, slot) in [(f32t, Scalar::F64(1.5)), (f64t, Scalar::F32(1.5))] {
        let mut b = Build { program: good.clone() };
        b.function(Function { name: "k".into(), params: 0, locals: locals(&[ty]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(ty, vec![slot])), 1)], ret())] });
        assert_eq!(
            Machine::load(b.program, &[]).map(|_| ()),
            Err(LoadError::Instruction { function: "k".into(), block: 0, statement: Some(0), reason: "a constant whose slots are not its type's" }),
            "a constant {slot:?}"
        );
    }
}

// Round 10's (the critic's kill-tests, kept): an Option of another shape, a failed core operation's position, a
// region for a by-value parameter, a scalar type listed twice, a switch on what is no integer, a checked result
// into a structure of three fields.

type Shape = Box<dyn Fn(&mut Build, TypeId) -> TypeId>;

fn opt_like(b: &mut Build, name: &str, variants: Vec<(i128, Vec<TypeId>)>) -> TypeId {
    let variants = variants
        .into_iter()
        .enumerate()
        .map(|(i, (d, fields))| Variant {
            name: format!("V{i}"),
            discriminant: d,
            fields: fields.into_iter().enumerate().map(|(k, ty)| Field { name: k.to_string(), ty }).collect(),
        })
        .collect();
    b.ty(Type::Enum { name: name.into(), discriminant: Int::Isize, variants })
}

fn core_call(b: &mut Build, op: CoreOp, dest: TypeId, args: &[TypeId]) {
    let mut types = vec![dest];
    types.extend_from_slice(args);
    b.function(Function {
        name: "f".into(),
        params: args.len() as u32,
        locals: locals(&types),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(op), args: (1..=args.len() as u32).map(copy).collect(), dest: place(0), next: BlockId(1), at: at(1) }),
            block(vec![], ret()),
        ],
    });
}

#[test]
fn an_option_of_another_shape_is_no_option_to_a_core_operation() {
    let shapes: Vec<(&str, Shape, bool)> = vec![
        ("Option { None = 0, Some(u8) = 1 }", Box::new(|b, u| opt_like(b, "Option", vec![(0, vec![]), (1, vec![u])])), true),
        ("Option { None = 1, Some(u8) = 0 }", Box::new(|b, u| opt_like(b, "Option", vec![(1, vec![]), (0, vec![u])])), false),
        ("Option { None = 0, Some(u8) = 2 }", Box::new(|b, u| opt_like(b, "Option", vec![(0, vec![]), (2, vec![u])])), false),
        ("Option { None(u8) = 0, Some(u8) = 1 }", Box::new(|b, u| opt_like(b, "Option", vec![(0, vec![u]), (1, vec![u])])), false),
        ("Option { None = 0, Some(u8, u8) = 1 }", Box::new(|b, u| opt_like(b, "Option", vec![(0, vec![]), (1, vec![u, u])])), false),
        ("Option { None = 0, Some(u8) = 1, Other = 2 }", Box::new(|b, u| opt_like(b, "Option", vec![(0, vec![]), (1, vec![u]), (2, vec![])])), false),
    ];
    for (shape, make, good) in &shapes {
        for op in [CoreOp::OptionIsNone, CoreOp::OptionIsSome, CoreOp::OptionGetOrInsert, CoreOp::CheckedShl(Int::U8)] {
            let mut b = Build::default();
            let (u8t, u32t, boolean) = (b.int(Int::U8), b.int(Int::U32), b.bool());
            let opt = make(&mut b, u8t);
            match op {
                CoreOp::OptionIsNone | CoreOp::OptionIsSome => {
                    let sopt = b.reference(opt, false);
                    core_call(&mut b, op, boolean, &[sopt]);
                }
                CoreOp::OptionGetOrInsert => {
                    let (ropt, ru8) = (b.reference(opt, true), b.reference(u8t, true));
                    core_call(&mut b, op, ru8, &[ropt, u8t]);
                }
                _ => core_call(&mut b, op, opt, &[u8t, u32t]),
            }
            let got = Machine::load(b.program, &[]).map(|_| ());
            if *good {
                assert_eq!(got, Ok(()), "{op:?} on {shape}");
            } else {
                assert_eq!(
                    got,
                    Err(LoadError::Instruction { function: "f".into(), block: 0, statement: None, reason: "a core operation called with types it does not take" }),
                    "{op:?} on {shape}"
                );
            }
        }
    }
}

#[test]
fn a_failed_core_operation_leaves_the_machine_at_its_call() {
    // fn f(o: &Option<u8>) -> bool { o.is_none() } on a region never set
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let opt = b.option(u8t);
    let sopt = b.reference(opt, false);
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[boolean, sopt]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionIsNone), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(4) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    let err = m.call(f, &[Arg::Ref(RegionId(0))]).unwrap_err();
    assert!(matches!(err, CallError::Program(ProgramError { kind: ErrorKind::Uninit, .. })), "{err:?}");
    assert_eq!(m.position(), Some((f, BlockId(0), 0)), "the machine stays at the failed call");
    assert_eq!(m.clone().position(), Some((f, BlockId(0), 0)), "and so does a copy of it");
}

#[test]
fn a_region_by_reference_for_a_parameter_taken_by_value_is_refused() {
    // fn f(x: u8) -> u8 { x }: a reference to a u8 region is no u8
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let f = b.function(Function { name: "f".into(), params: 1, locals: locals(&[u8t, u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let mut m = Machine::load(b.program, &[u8t]).unwrap();
    m.set_region(RegionId(0), &[int(Int::U8, 5)]).unwrap();
    assert_eq!(m.start(f, &[Arg::Ref(RegionId(0))]), Err(HostError::Arguments));
}

#[test]
fn a_scalar_type_listed_twice_is_one_type() {
    // types: u8, u8 again, &u8 (to the first); fn f(x: &u8) -> u8 { *x } returning the second u8; a region of it
    let mut p = Program::default();
    p.types.push(Type::Int(Int::U8));
    p.types.push(Type::Int(Int::U8));
    p.types.push(Type::Ref { to: TypeId(0), mutable: false });
    p.functions.push(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[TypeId(1), TypeId(2)]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Deref]))), 1)], ret())],
    });
    let mut m = Machine::load(p, &[TypeId(1)]).expect("one type listed twice loads");
    m.set_region(RegionId(0), &[int(Int::U8, 5)]).unwrap();
    assert_eq!(m.call(FunctionId(0), &[Arg::Ref(RegionId(0))]).map(|r| r.to_vec()), Ok(vec![int(Int::U8, 5)]));
}

#[test]
fn a_switch_with_values_on_what_is_no_integer_is_refused() {
    for kind in ["struct", "duration", "f32"] {
        let mut b = Build::default();
        let (u8t, unit) = (b.int(Int::U8), b.unit());
        let ty = match kind {
            "struct" => b.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }),
            "duration" => b.ty(Type::Duration),
            _ => b.ty(Type::F32),
        };
        b.function(Function {
            name: "f".into(),
            params: 1,
            locals: locals(&[unit, ty]),
            blocks: vec![
                block(vec![], Terminator::SwitchInt { value: copy(1), targets: vec![(1, BlockId(1))], otherwise: BlockId(1), at: at(1) }),
                block(vec![assign(place(0), Value::Aggregate(unit, None, vec![]), 2)], ret()),
            ],
        });
        assert_eq!(
            Machine::load(b.program, &[]).map(|_| ()),
            Err(LoadError::Instruction { function: "f".into(), block: 0, statement: None, reason: "a switch's value outside its operand's type" }),
            "a switch on a {kind}"
        );
    }
}

#[test]
fn a_checked_result_into_a_structure_of_three_fields_is_refused() {
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let three = b.tuple(&[u8t, boolean, u8t]);
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[three, u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Checked(BinOp::Add, copy(1), copy(1)), 1)], ret())],
    });
    assert_eq!(
        Machine::load(b.program, &[]).map(|_| ()),
        Err(LoadError::Instruction { function: "f".into(), block: 0, statement: Some(0), reason: "a checked result into what is no (value, bool)" })
    );
}

// Round 11's (the critic's kill-tests, kept): every scalar kind listed twice at every typed place, an array's first
// and middle element, every argument of a start, a later call argument and aggregate part.

fn func(name: &str, params: u32, types: &[TypeId], blocks: Vec<Block>) -> Function {
    Function { name: name.into(), params, locals: locals(types), blocks }
}

fn push(p: &mut Program, ty: Type) -> TypeId {
    p.types.push(ty);
    TypeId(p.types.len() as u32 - 1)
}

#[test]
fn every_scalar_kind_listed_twice_is_one_type_at_every_typed_place() {
    let mut p = Program::default();
    let (u8a, u8b) = (push(&mut p, Type::Int(Int::U8)), push(&mut p, Type::Int(Int::U8)));
    let (ba, bb) = (push(&mut p, Type::Bool), push(&mut p, Type::Bool));
    let (fa, fb) = (push(&mut p, Type::F32), push(&mut p, Type::F32));
    let (ga, gb) = (push(&mut p, Type::F64), push(&mut p, Type::F64));
    let (da, db) = (push(&mut p, Type::Duration), push(&mut p, Type::Duration));
    let opt = push(
        &mut p,
        Type::Enum {
            name: "Option".into(),
            discriminant: Int::Isize,
            variants: vec![
                Variant { name: "None".into(), discriminant: 0, fields: vec![] },
                Variant { name: "Some".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: da }] },
            ],
        },
    );
    let ropt = push(&mut p, Type::Ref { to: opt, mutable: true });
    let rdb = push(&mut p, Type::Ref { to: db, mutable: true });
    let pair = push(&mut p, Type::Struct { name: "(..)".into(), fields: vec![Field { name: "0".into(), ty: u8a }, Field { name: "1".into(), ty: ba }] });
    let ru8a = push(&mut p, Type::Ref { to: u8a, mutable: false });
    let s = push(&mut p, Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8a }] });
    let one = |v: Value| vec![block(vec![assign(place(0), v, 1)], ret())];
    let two = |a, b, op| Value::Binary(op, copy(a), copy(b));
    p.functions.push(func("g", 1, &[u8a, u8a], one(Value::Use(copy(1))))); // 0
    p.functions.push(func(
        "call_g", // 1: an argument of the second u8 to a parameter of the first, its result into the second
        1,
        &[u8b, u8b, u8b],
        vec![
            block(vec![], Terminator::Call { callee: Callee::Function(FunctionId(0)), args: vec![copy(1)], dest: place(2), next: BlockId(1), at: at(1) }),
            block(vec![assign(place(0), Value::Use(copy(2)), 2)], ret()),
        ],
    ));
    p.functions.push(func("add", 2, &[u8b, u8a, u8b], one(two(1, 2, BinOp::Add)))); // 2
    p.functions.push(func("and", 2, &[u8a, u8a, u8b], one(two(1, 2, BinOp::BitAnd)))); // 3
    p.functions.push(func("lt", 2, &[bb, u8a, u8b], one(two(1, 2, BinOp::Lt)))); // 4
    p.functions.push(func("checked", 2, &[pair, u8b, u8a], one(Value::Checked(BinOp::Add, copy(1), copy(2))))); // 5
    p.functions.push(func("not", 1, &[u8b, u8a], one(Value::Unary(UnOp::Not, copy(1))))); // 6
    p.functions.push(func("cast", 1, &[u8a, u8a], one(Value::Cast(copy(1), u8b)))); // 7
    p.functions.push(func("agg", 1, &[s, u8b], one(Value::Aggregate(s, None, vec![copy(1)])))); // 8
    p.functions.push(func(
        "refer", // 9
        1,
        &[u8a, u8b, ru8a],
        vec![block(
            vec![assign(place(2), Value::Ref(place(1), false), 1), assign(place(0), Value::Use(copy_at(proj(2, vec![Projection::Deref]))), 2)],
            ret(),
        )],
    ));
    p.functions.push(func(
        "fill", // 10: o.get_or_insert(v) with v of the second Duration, the Option's payload of the first
        2,
        &[db, ropt, db, rdb],
        vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionGetOrInsert), args: vec![copy(1), copy(2)], dest: place(3), next: BlockId(1), at: at(1) }),
            block(vec![assign(place(0), Value::Use(copy_at(proj(3, vec![Projection::Deref]))), 2)], ret()),
        ],
    ));
    p.functions.push(func("xor", 2, &[ba, ba, bb], one(two(1, 2, BinOp::BitXor)))); // 11
    p.functions.push(func("fadd", 2, &[fa, fa, fb], one(two(1, 2, BinOp::Add)))); // 12
    p.functions.push(func("gadd", 2, &[ga, ga, gb], one(two(1, 2, BinOp::Add)))); // 13
    let mut m = Machine::load(p, &[opt]).expect("a program listing every scalar kind twice loads");
    let u = |v| vec![int(Int::U8, v)];
    let cases: Vec<(u32, Vec<Vec<Scalar>>, Vec<Scalar>)> = vec![
        (1, vec![u(7)], u(7)),
        (2, vec![u(200), u(100)], u(44)),
        (3, vec![u(12), u(10)], u(8)),
        (4, vec![u(3), u(5)], vec![Scalar::Bool(true)]),
        (5, vec![u(250), u(10)], vec![int(Int::U8, 4), Scalar::Bool(true)]),
        (6, vec![u(5)], u(250)),
        (7, vec![u(7)], u(7)),
        (8, vec![u(9)], u(9)),
        (9, vec![u(11)], u(11)),
        (11, vec![vec![Scalar::Bool(true)], vec![Scalar::Bool(false)]], vec![Scalar::Bool(true)]),
        (12, vec![vec![Scalar::F32(1.5)], vec![Scalar::F32(2.25)]], vec![Scalar::F32(3.75)]),
        (13, vec![vec![Scalar::F64(1.5)], vec![Scalar::F64(2.25)]], vec![Scalar::F64(3.75)]),
    ];
    for (f, args, want) in cases {
        let args: Vec<Arg> = args.iter().map(|a| Arg::Value(a)).collect();
        assert_eq!(m.call(FunctionId(f), &args).map(|r| r.to_vec()), Ok(want), "function {f}");
    }
    m.set_region(RegionId(0), &[int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit]).unwrap();
    let v = dur(Duration::new(3, 5));
    assert_eq!(m.call(FunctionId(10), &[Arg::Ref(RegionId(0)), Arg::Value(&v)]).map(|r| r.to_vec()), Ok(v.clone()));
    let mut filled = vec![int(Int::Isize, 1)];
    filled.extend(v);
    assert_eq!(m.region(RegionId(0)).unwrap(), filled.as_slice());
}

#[test]
fn an_array_whose_first_or_middle_element_is_not_of_its_type_is_refused_as_a_region_an_argument_and_a_constant() {
    let mut b = Build::default();
    let (u8t, u16t) = (b.int(Int::U8), b.int(Int::U16));
    let pair = b.tuple(&[u8t, u16t]);
    let arr = b.ty(Type::Array { element: pair, len: 3 });
    let g = b.function(Function { name: "g".into(), params: 1, locals: locals(&[arr, arr]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let program = b.program;
    let (a, c) = (|v| int(Int::U8, v), |v| int(Int::U16, v));
    let good = vec![a(1), c(2), a(3), c(4), a(5), c(6)];
    let mut m = Machine::load(program.clone(), &[arr]).unwrap();
    assert_eq!(m.set_region(RegionId(0), &good), Ok(()));
    for (name, wrong) in [("the first", vec![a(1), a(2), a(3), c(4), a(5), c(6)]), ("the middle", vec![a(1), c(2), a(3), a(4), a(5), c(6)])] {
        assert_eq!(m.set_region(RegionId(0), &wrong), Err(HostError::Slots), "{name} element's u16 a u8, as a region");
        assert_eq!(m.region(RegionId(0)).unwrap(), good.as_slice(), "{name}: the region keeps its value");
        assert_eq!(m.start(g, &[Arg::Value(&wrong)]), Err(HostError::Arguments), "{name}, as an argument");
        let mut p = program.clone();
        p.functions.push(Function { name: "k".into(), params: 0, locals: locals(&[arr]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(arr, wrong.clone())), 1)], ret())] });
        assert_eq!(
            Machine::load(p, &[]).map(|_| ()),
            Err(LoadError::Instruction { function: "k".into(), block: 0, statement: Some(0), reason: "a constant whose slots are not its type's" }),
            "{name}, as a constant"
        );
    }
}

#[test]
fn every_argument_of_a_start_is_checked_not_the_first_alone() {
    // fn f(a: u8, r: &u16, b: u8, s: &u16) -> u8 { a }
    let mut b = Build::default();
    let (u8t, u16t) = (b.int(Int::U8), b.int(Int::U16));
    let r16 = b.reference(u16t, false);
    let f = b.function(Function { name: "f".into(), params: 4, locals: locals(&[u8t, u8t, r16, u8t, r16]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let mut m = Machine::load(b.program, &[u16t, u8t]).unwrap();
    let (x, y, wide, two) = ([int(Int::U8, 1)], [int(Int::U8, 2)], [int(Int::U16, 2)], [int(Int::U8, 2), int(Int::U8, 3)]);
    assert_eq!(m.start(f, &[Arg::Value(&x), Arg::Ref(RegionId(0)), Arg::Value(&y), Arg::Ref(RegionId(0))]), Ok(()));
    m.abort();
    let cases: [(&str, [Arg; 4]); 3] = [
        ("a later value of another type", [Arg::Value(&x), Arg::Ref(RegionId(0)), Arg::Value(&wide), Arg::Ref(RegionId(0))]),
        ("a later value of two slots", [Arg::Value(&x), Arg::Ref(RegionId(0)), Arg::Value(&two), Arg::Ref(RegionId(0))]),
        ("a later region of another type", [Arg::Value(&x), Arg::Ref(RegionId(0)), Arg::Value(&y), Arg::Ref(RegionId(1))]),
    ];
    for (name, args) in cases {
        assert_eq!(m.start(f, &args), Err(HostError::Arguments), "{name}");
        assert!(!m.running(), "{name}: nothing started");
    }
}

#[test]
fn a_later_call_argument_and_a_later_aggregate_part_of_another_type_are_refused() {
    let mut b = Build::default();
    let (u8t, u16t) = (b.int(Int::U8), b.int(Int::U16));
    let s = b.tuple(&[u8t, u8t]);
    b.function(Function { name: "h".into(), params: 2, locals: locals(&[u8t, u8t, u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let mut call = Build { program: b.program.clone() };
    call.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[u8t, u8t, u16t]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Function(FunctionId(0)), args: vec![copy(1), copy(2)], dest: place(0), next: BlockId(1), at: at(1) }),
            block(vec![], ret()),
        ],
    });
    assert_eq!(
        Machine::load(call.program, &[]).map(|_| ()),
        Err(LoadError::Instruction { function: "f".into(), block: 0, statement: None, reason: "an argument of another type than its parameter" })
    );
    let mut agg = Build { program: b.program };
    agg.function(Function { name: "f".into(), params: 2, locals: locals(&[s, u8t, u16t]), blocks: vec![block(vec![assign(place(0), Value::Aggregate(s, None, vec![copy(1), copy(2)]), 1)], ret())] });
    assert_eq!(
        Machine::load(agg.program, &[]).map(|_| ()),
        Err(LoadError::Instruction { function: "f".into(), block: 0, statement: Some(0), reason: "a part of another type than its place" })
    );
}

// Round 12's (the critic's kill-tests, kept): moved operands, every operand of an operation checked, an unsigned
// enum's negative discriminant, every check an assertion carries, a call's own location, a variant written over a
// larger one, and a copy's regions and last result.

fn mv(local: u32) -> Operand {
    Operand::Move(place(local))
}

fn mv_at(p: Place) -> Operand {
    Operand::Move(p)
}


/// fn f(n: u32, d: Duration) -> (u32, Duration) { if n > 0 { g((n + 1, d.min(FIVE))) } else { (0, d) } }, every
/// temporary read by `move` as rustc gives it: switchInt(move _3), assert(!move (_4.1)), _5 = move (_4.0), a core
/// operation's Duration by move, an aggregate's parts by move, a call's argument by move, and g's `_0 = move _1`.
#[test]
fn moved_operands_run_as_copies_do() {
    let mut b = Build::default();
    let (u32t, boolean, d) = (b.int(Int::U32), b.bool(), b.ty(Type::Duration));
    let pair = b.tuple(&[u32t, boolean]);
    let out = b.tuple(&[u32t, d]);
    let g = b.function(Function {
        name: "g".into(),
        params: 1,
        locals: locals(&[out, out]),
        blocks: vec![block(vec![assign(place(0), Value::Use(mv(1)), 1)], ret())],
    });
    let five = konst(d, dur(Duration::from_secs(5)));
    let f = b.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[out, u32t, d, boolean, pair, u32t, d, out, u32t]),
        blocks: vec![
            block(
                vec![
                    assign(place(8), Value::Use(copy(1)), 1),
                    assign(place(3), Value::Binary(BinOp::Gt, mv(8), konst(u32t, vec![int(Int::U32, 0)])), 1),
                ],
                Terminator::SwitchInt { value: mv(3), targets: vec![(0, BlockId(5))], otherwise: BlockId(1), at: at(1) },
            ),
            block(
                vec![assign(place(4), Value::Checked(BinOp::Add, mv(1), konst(u32t, vec![int(Int::U32, 1)])), 2)],
                Terminator::Assert { cond: mv_at(proj(4, vec![Projection::Field(1)])), expected: false, check: Check::Overflow(BinOp::Add), next: BlockId(2), at: at(2) },
            ),
            block(
                vec![assign(place(5), Value::Use(mv_at(proj(4, vec![Projection::Field(0)]))), 3)],
                Terminator::Call { callee: Callee::Core(CoreOp::DurationMin), args: vec![mv(2), five], dest: place(6), next: BlockId(3), at: at(3) },
            ),
            block(
                vec![assign(place(7), Value::Aggregate(out, None, vec![mv(5), mv(6)]), 4)],
                Terminator::Call { callee: Callee::Function(g), args: vec![mv(7)], dest: place(0), next: BlockId(4), at: at(4) },
            ),
            block(vec![], ret()),
            block(vec![assign(place(0), Value::Aggregate(out, None, vec![konst(u32t, vec![int(Int::U32, 0)]), mv(2)]), 6)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).expect("a program of moves loads");
    let run = |m: &mut Machine, n: i128, x: Duration| {
        let (a, c) = ([int(Int::U32, n)], dur(x));
        let r = m.call(f, &[Arg::Value(&a), Arg::Value(&c)]).map(|r| r.to_vec());
        m.abort();
        r
    };
    let mut want = vec![int(Int::U32, 0)];
    want.extend(dur(Duration::from_secs(3)));
    assert_eq!(run(&mut m, 0, Duration::from_secs(3)), Ok(want), "n = 0: (0, d)");
    let mut want = vec![int(Int::U32, 8)];
    want.extend(dur(Duration::from_secs(5)));
    assert_eq!(run(&mut m, 7, Duration::from_secs(9)), Ok(want), "n = 7: (8, d.min(5 s))");
    let mut want = vec![int(Int::U32, 8)];
    want.extend(dur(Duration::from_millis(1500)));
    assert_eq!(run(&mut m, 7, Duration::from_millis(1500)), Ok(want), "n = 7: (8, 1.5 s)");
    assert_eq!(
        run(&mut m, u32::MAX as i128, Duration::ZERO),
        Err(CallError::Program(ProgramError { function: f, block: BlockId(1), statement: None, at: at(2), kind: ErrorKind::Check(Check::Overflow(BinOp::Add)) })),
        "the overflow named at its assertion"
    );
}

/// fn h(a: u8, r: &u8) -> u8 { a + *r }, the region never set: its right operand is the one never written, as in
/// `self.total + inputs.step`.
#[test]
fn an_unwritten_right_operand_is_uninit() {
    let mut b = Build::default();
    let (u8t, f32t, boolean) = (b.int(Int::U8), b.ty(Type::F32), b.bool());
    let (ru8, rf32) = (b.reference(u8t, false), b.reference(f32t, false));
    let h = b.function(Function {
        name: "h".into(),
        params: 2,
        locals: locals(&[u8t, u8t, ru8]),
        blocks: vec![block(vec![assign(place(0), Value::Binary(BinOp::Add, copy(1), copy_at(proj(2, vec![Projection::Deref]))), 7)], ret())],
    });
    let k = b.function(Function {
        name: "k".into(),
        params: 2,
        locals: locals(&[boolean, f32t, rf32]),
        blocks: vec![block(vec![assign(place(0), Value::Binary(BinOp::Lt, copy(1), copy_at(proj(2, vec![Projection::Deref]))), 8)], ret())],
    });
    let mut m = Machine::load(b.program, &[u8t, f32t]).unwrap();
    let one = [int(Int::U8, 1)];
    let e = m.call(h, &[Arg::Value(&one), Arg::Ref(RegionId(0))]).map(|r| r.to_vec());
    assert_eq!(e, Err(CallError::Program(ProgramError { function: h, block: BlockId(0), statement: Some(0), at: at(7), kind: ErrorKind::Uninit })));
    m.abort();
    let x = [Scalar::F32(1.0)];
    let e = m.call(k, &[Arg::Value(&x), Arg::Ref(RegionId(1))]).map(|r| r.to_vec());
    assert_eq!(e, Err(CallError::Program(ProgramError { function: k, block: BlockId(0), statement: Some(0), at: at(8), kind: ErrorKind::Uninit })));
}

fn one_statement(types: &[Type], locals_: &[u32], value: Value) -> LoadError {
    let mut p = Program { types: types.to_vec(), ..Program::default() };
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&locals_.iter().map(|t| TypeId(*t)).collect::<Vec<_>>()),
        blocks: vec![block(vec![assign(place(0), value, 1)], ret())],
    });
    Machine::load(p, &[]).map(|_| ()).expect_err("refused")
}

fn at_statement(reason: &'static str) -> LoadError {
    LoadError::Instruction { function: "f".into(), block: 0, statement: Some(0), reason }
}

#[test]
fn every_operand_of_a_binary_or_checked_operation_is_checked_at_load() {
    let u8 = Type::Int(Int::U8);
    let u16 = Type::Int(Int::U16);
    let pair = |a: u32, b: u32| Type::Struct { name: "(..)".into(), fields: vec![Field { name: "0".into(), ty: TypeId(a) }, Field { name: "1".into(), ty: TypeId(b) }] };
    // types: 0 u8, 1 u16, 2 bool, 3 f32, 4 (u8, bool), 5 (f32, bool), 6 (u8, u8), 7 Duration
    let types = [u8.clone(), u16, Type::Bool, Type::F32, pair(0, 2), pair(3, 2), pair(0, 0), Type::Duration];
    assert_eq!(one_statement(&types, &[0, 0, 2], Value::Binary(BinOp::Shl, copy(1), copy(2))), at_statement("a shift of what is no integer"), "a shift by a bool");
    assert_eq!(one_statement(&types, &[4, 0, 1], Value::Checked(BinOp::Add, copy(1), copy(2))), at_statement("a checked operation that MIR has no checked form of"), "u8 + u16");
    assert_eq!(one_statement(&types, &[5, 3, 3], Value::Checked(BinOp::Add, copy(1), copy(2))), at_statement("a checked operation that MIR has no checked form of"), "a float's");
    assert_eq!(one_statement(&types, &[6, 0, 0], Value::Checked(BinOp::Add, copy(1), copy(2))), at_statement("a checked result into what is no (value, bool)"), "into (u8, u8)");
    assert_eq!(one_statement(&types, &[2, 7, 7], Value::Binary(BinOp::Lt, copy(1), copy(2))), at_statement("a comparison of values that do not compare"), "Durations");
    assert_eq!(one_statement(&types, &[3, 2], Value::Cast(copy(1), TypeId(3))), at_statement("a cast `as` does not make"), "bool as f32");
    let s = Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: TypeId(0) }] };
    let types = [u8, s];
    assert_eq!(one_statement(&types, &[1, 0], Value::Aggregate(TypeId(1), Some(0), vec![copy(1)])), at_statement("an aggregate of what is built otherwise"), "a structure built as a variant");
}

#[test]
fn a_negative_discriminant_of_an_unsigned_enum_and_a_switch_value_below_the_least_are_refused() {
    let mut b = Build::default();
    b.ty(Type::Enum { name: "E".into(), discriminant: Int::U8, variants: vec![Variant { name: "A".into(), discriminant: -1, fields: vec![] }] });
    assert_eq!(Machine::load(b.program, &[]).map(|_| ()), Err(LoadError::Type { ty: 0, reason: "a discriminant outside its type" }));
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, u8t]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: copy(1), targets: vec![(-1, BlockId(1))], otherwise: BlockId(1), at: at(1) }),
            block(vec![], ret()),
        ],
    });
    assert_eq!(
        Machine::load(b.program, &[]).map(|_| ()),
        Err(LoadError::Instruction { function: "f".into(), block: 0, statement: None, reason: "a switch's value outside its operand's type" })
    );
}

#[test]
fn get_or_insert_through_a_shared_reference_and_a_default_of_a_structure_are_refused() {
    let mut b = Build::default();
    let (u8t, d) = (b.int(Int::U8), b.ty(Type::Duration));
    let optd = b.option(d);
    let (ropt, sopt, rd, sd) = (b.reference(optd, true), b.reference(optd, false), b.reference(d, true), b.reference(d, false));
    let s = b.tuple(&[u8t]);
    let call = |b: &Build, op: CoreOp, dest: TypeId, args: &[TypeId]| {
        let mut p = Build { program: b.program.clone() };
        let mut types = vec![dest];
        types.extend_from_slice(args);
        p.function(Function {
            name: "f".into(),
            params: args.len() as u32,
            locals: locals(&types),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Core(op), args: (1..=args.len() as u32).map(copy).collect(), dest: place(0), next: BlockId(1), at: at(1) }),
                block(vec![], ret()),
            ],
        });
        Machine::load(p.program, &[]).map(|_| ())
    };
    let refused = Err(LoadError::Instruction { function: "f".into(), block: 0, statement: None, reason: "a core operation called with types it does not take" });
    assert_eq!(call(&b, CoreOp::OptionGetOrInsert, rd, &[ropt, d]), Ok(()), "the right shape");
    assert_eq!(call(&b, CoreOp::OptionGetOrInsert, rd, &[sopt, d]), refused, "through a shared reference");
    assert_eq!(call(&b, CoreOp::OptionGetOrInsert, sd, &[ropt, d]), refused, "into a shared reference");
    assert_eq!(call(&b, CoreOp::Default, s, &[]), refused, "a structure's default");
}

/// fn f(a: u8, b: u8) -> u8 { a / b }, MIR's own check first: _3 = Eq(copy _2, const 0); assert(!move _3,
/// "attempt to divide by zero"): the error names that check.
#[test]
fn an_assertion_s_error_names_the_check_it_carries() {
    for (check, line) in [(Check::DivisionByZero, 3), (Check::RemainderByZero, 4), (Check::NegOverflow, 5)] {
        let mut b = Build::default();
        let (u8t, boolean) = (b.int(Int::U8), b.bool());
        let f = b.function(Function {
            name: "f".into(),
            params: 1,
            locals: locals(&[u8t, u8t, boolean]),
            blocks: vec![
                block(
                    vec![assign(place(2), Value::Binary(BinOp::Eq, copy(1), konst(u8t, vec![int(Int::U8, 0)])), 1)],
                    Terminator::Assert { cond: mv(2), expected: false, check, next: BlockId(1), at: at(line) },
                ),
                block(vec![assign(place(0), Value::Use(copy(1)), 2)], ret()),
            ],
        });
        let mut m = Machine::load(b.program, &[]).unwrap();
        let zero = [int(Int::U8, 0)];
        assert_eq!(
            m.call(f, &[Arg::Value(&zero)]).map(|r| r.to_vec()),
            Err(CallError::Program(ProgramError { function: f, block: BlockId(0), statement: None, at: at(line), kind: ErrorKind::Check(check) }))
        );
    }
}

/// A call's argument past an array's end (the machine's own bounds check), named at the call; and a callee's result
/// into a host's region through a reference, `(*_1) = g() -> bb1`.
#[test]
fn a_call_fails_at_its_own_location_and_writes_its_result_through_a_reference() {
    let mut b = Build::default();
    let (u8t, usize_t, unit) = (b.int(Int::U8), b.int(Int::Usize), b.unit());
    let arr = b.ty(Type::Array { element: u8t, len: 2 });
    let ru8 = b.reference(u8t, true);
    let g = b.function(Function { name: "g".into(), params: 1, locals: locals(&[u8t, u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())] });
    let f = b.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[u8t, arr, usize_t]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Function(g), args: vec![copy_at(proj(1, vec![Projection::Index(Local(2))]))], dest: place(0), next: BlockId(1), at: at(9) }),
            block(vec![], ret()),
        ],
    });
    let w = b.function(Function {
        name: "w".into(),
        params: 1,
        locals: locals(&[unit, ru8]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Function(g), args: vec![konst(u8t, vec![int(Int::U8, 42)])], dest: proj(1, vec![Projection::Deref]), next: BlockId(1), at: at(10) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[u8t]).unwrap();
    let (a, i) = ([int(Int::U8, 1), int(Int::U8, 2)], [int(Int::Usize, 2)]);
    assert_eq!(
        m.call(f, &[Arg::Value(&a), Arg::Value(&i)]).map(|r| r.to_vec()),
        Err(CallError::Program(ProgramError { function: f, block: BlockId(0), statement: None, at: at(9), kind: ErrorKind::Bounds { index: 2, len: 2 } }))
    );
    m.abort();
    m.call(w, &[Arg::Ref(RegionId(0))]).unwrap();
    assert_eq!(m.region(RegionId(0)).unwrap(), &[int(Int::U8, 42)]);
}

/// fn reset(o: &mut Option<Duration>, d: Duration) { *o = Some(d); *o = None }, a timer's reset: the None written over
/// the Some leaves no payload, so the region the program wrote is a value of its type the host can set back.
#[test]
fn a_variant_written_over_a_larger_one_leaves_the_rest_unwritten() {
    let mut b = Build::default();
    let (d, unit) = (b.ty(Type::Duration), b.unit());
    let opt = b.option(d);
    let ropt = b.reference(opt, true);
    let reset = b.function(Function {
        name: "reset".into(),
        params: 2,
        locals: locals(&[unit, ropt, d]),
        blocks: vec![block(
            vec![
                assign(proj(1, vec![Projection::Deref]), Value::Aggregate(opt, Some(1), vec![copy(2)]), 1),
                assign(proj(1, vec![Projection::Deref]), Value::Aggregate(opt, Some(0), vec![]), 2),
            ],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    m.call(reset, &[Arg::Ref(RegionId(0)), Arg::Value(&dur(Duration::from_millis(250)))]).unwrap();
    let read = m.region(RegionId(0)).unwrap().to_vec();
    assert_eq!(read, vec![int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit], "None, and no payload left");
    assert_eq!(m.set_region(RegionId(0), &read), Ok(()), "what the program wrote is a value of the region's type");
}

/// fn bump(s: &mut u32) -> u32 { *s = *s + 1 (wrapping); *s }: a copy taken mid-run of a machine whose state region
/// holds 41 stands where the original stood, its region included, and goes on alike.
#[test]
fn a_copy_stands_where_the_original_stood_in_its_regions_too() {
    let mut b = Build::default();
    let u32t = b.int(Int::U32);
    let r = b.reference(u32t, true);
    let bump = b.function(Function {
        name: "bump".into(),
        params: 1,
        locals: locals(&[u32t, r]),
        blocks: vec![block(
            vec![
                assign(proj(1, vec![Projection::Deref]), Value::Binary(BinOp::Add, copy_at(proj(1, vec![Projection::Deref])), konst(u32t, vec![int(Int::U32, 1)])), 1),
                assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Deref]))), 2),
            ],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[u32t]).unwrap();
    m.set_region(RegionId(0), &[int(Int::U32, 41)]).unwrap();
    m.start(bump, &[Arg::Ref(RegionId(0))]).unwrap();
    let mut idle = m.clone();
    assert_eq!(idle.region(RegionId(0)).unwrap(), &[int(Int::U32, 41)], "the copy's region");
    assert_eq!(m.run(1), Ok(Step::Ran));
    let mut copy = m.clone();
    assert_eq!(copy.memory(), m.memory(), "the copy's whole memory, regions first");
    assert_eq!(m.run(10), Ok(Step::Returned));
    assert_eq!(copy.run(10), Ok(Step::Returned));
    assert_eq!((copy.result(), copy.memory(), copy.steps()), (m.result(), m.memory(), m.steps()));
    assert_eq!(m.result(), &[int(Int::U32, 42)]);
    assert_eq!(idle.run(10), Ok(Step::Returned));
    assert_eq!(idle.result(), &[int(Int::U32, 42)], "a copy taken at the start goes on alike");
}

/// A copy of an idle machine whose last entry returned shows that entry's result, as the original does.
#[test]
fn a_copy_of_an_idle_machine_shows_the_last_result() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let f = b.function(Function { name: "f".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 7)])), 1)], ret())] });
    let mut m = Machine::load(b.program, &[]).unwrap();
    m.call(f, &[]).unwrap();
    let copy = m.clone();
    assert_eq!((copy.result(), copy.running(), copy.steps()), (m.result(), m.running(), m.steps()));
    assert_eq!(copy.result(), &[int(Int::U8, 7)]);
}

// Round 13's (the critic's kill-tests, kept): arithmetic and bits of another right operand, core operations taking
// moved operands, a downcast past the last variant, a switch on a constant, the last variant's trailing slots.



#[test]
fn arithmetic_and_bits_whose_right_operand_is_of_another_type_are_refused_at_load() {
    // types: 0 u8, 1 u16, 2 bool, 3 f32, 4 f64
    let types = [Type::Int(Int::U8), Type::Int(Int::U16), Type::Bool, Type::F32, Type::F64];
    for (op, l, r, name) in [
        (BinOp::Add, 0, 1, "u8 + u16"),
        (BinOp::Sub, 0, 2, "u8 - bool"),
        (BinOp::Mul, 3, 4, "f32 * f64"),
        (BinOp::Div, 0, 3, "u8 / f32"),
    ] {
        assert_eq!(one_statement(&types, &[l, l, r], Value::Binary(op, copy(1), copy(2))), at_statement("arithmetic on what is no number"), "{name}");
    }
    for (op, l, r, name) in [(BinOp::BitAnd, 0, 1, "u8 & u16"), (BinOp::BitOr, 2, 0, "bool | u8"), (BinOp::BitXor, 1, 0, "u16 ^ u8")] {
        assert_eq!(one_statement(&types, &[l, l, r], Value::Binary(op, copy(1), copy(2))), at_statement("bits of what has none"), "{name}");
    }
}


fn dur_r13(d: core::time::Duration) -> Vec<Scalar> {
    vec![int(Int::U64, d.as_secs() as i128), int(Int::U32, d.subsec_nanos() as i128)]
}

/// The core operations' operands as rustc moves them: an autoref temporary into `&self` (`is_none(move _3)`,
/// `get_or_insert(move _5, move _6)`, `ge(move _9, move _10)`), and a temporary integer (`checked_shl(move _12, ..)`,
/// `saturating_add(move _14, ..)`).
///
/// fn f(o: &mut Option<Duration>, v: Duration, x: u8) -> (bool, Duration, bool, Option<u8>, u8) {
///     let none = o.is_none(); let got = *o.get_or_insert(v); let late = got >= v;
///     (none, got, late, (x + 0).checked_shl(1), (x + 0).saturating_add(200)) }
#[test]
fn core_operations_take_their_operands_by_move_as_rustc_passes_them() {
    use core::time::Duration;
    let mut b = Build::default();
    let (d, boolean, u8t, u32t) = (b.ty(Type::Duration), b.bool(), b.int(Int::U8), b.int(Int::U32));
    let opt = b.option(d);
    let optu8 = b.option(u8t);
    let (ropt, sopt, rd, sd) = (b.reference(opt, true), b.reference(opt, false), b.reference(d, true), b.reference(d, false));
    let out = b.tuple(&[boolean, d, boolean, optu8, u8t]);
    let zero = konst(u8t, vec![int(Int::U8, 0)]);
    let core = |op, args, dest, next| Terminator::Call { callee: Callee::Core(op), args, dest: place(dest), next: BlockId(next), at: at(next) };
    let f = b.function(Function {
        name: "f".into(),
        params: 3,
        //            0    1     2  3   4     5     6  7   8  9   10  11     12   13     14   15
        locals: locals(&[out, ropt, d, u8t, sopt, boolean, ropt, d, rd, d, sd, sd, boolean, u8t, optu8, u8t]),
        blocks: vec![
            block(vec![assign(place(4), Value::Ref(proj(1, vec![Projection::Deref]), false), 1)], core(CoreOp::OptionIsNone, vec![mv(4)], 5, 1)),
            block(
                vec![assign(place(6), Value::Ref(proj(1, vec![Projection::Deref]), true), 2), assign(place(7), Value::Use(copy(2)), 2)],
                core(CoreOp::OptionGetOrInsert, vec![mv(6), mv(7)], 8, 2),
            ),
            block(
                vec![
                    assign(place(9), Value::Use(copy_at(proj(8, vec![Projection::Deref]))), 3),
                    assign(place(10), Value::Ref(place(9), false), 3),
                    assign(place(11), Value::Ref(place(2), false), 3),
                ],
                core(CoreOp::DurationCompare(BinOp::Ge), vec![mv(10), mv(11)], 12, 3),
            ),
            block(
                vec![assign(place(13), Value::Binary(BinOp::Add, copy(3), zero.clone()), 4)],
                core(CoreOp::CheckedShl(Int::U8), vec![mv(13), konst(u32t, vec![int(Int::U32, 1)])], 14, 4),
            ),
            block(
                vec![assign(place(13), Value::Binary(BinOp::Add, copy(3), zero), 5)],
                core(CoreOp::SaturatingAdd(Int::U8), vec![mv(13), konst(u8t, vec![int(Int::U8, 200)])], 15, 5),
            ),
            block(vec![assign(place(0), Value::Aggregate(out, None, vec![mv(5), mv(9), mv(12), mv(14), mv(15)]), 6)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[opt]).expect("a program moving into core operations loads");
    m.set_region(RegionId(0), &[int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit]).unwrap();
    let mut native: Option<Duration> = None;
    for (v, x) in [(Duration::from_millis(700), 3u8), (Duration::from_millis(500), 100), (Duration::from_millis(900), 130)] {
        let none = native.is_none();
        let got = *native.get_or_insert(v);
        let mut want = vec![Scalar::Bool(none)];
        want.extend(dur_r13(got));
        want.push(Scalar::Bool(got >= v));
        match x.checked_shl(1) {
            Some(s) => want.extend([int(Int::Isize, 1), int(Int::U8, s as i128)]),
            None => want.extend([int(Int::Isize, 0), Scalar::Uninit]),
        }
        want.push(int(Int::U8, x.saturating_add(200) as i128));
        let got_m = m.call(f, &[Arg::Ref(RegionId(0)), Arg::Value(&dur_r13(v)), Arg::Value(&[int(Int::U8, x as i128)])]).map(|r| r.to_vec());
        assert_eq!(got_m, Ok(want), "v = {v:?}, x = {x}");
        let mut region = vec![int(Int::Isize, 1)];
        region.extend(dur_r13(Duration::from_millis(700)));
        assert_eq!(m.region(RegionId(0)).unwrap(), region.as_slice(), "the Option filled once, by the first value");
    }
}

/// d01: a downcast to the variant one past the last is no variant.
#[test]
fn a_downcast_one_past_the_last_variant_is_refused() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let opt = b.option(u8t);
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, opt]),
        blocks: vec![block(vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Downcast(2), Projection::Field(0)]))), 1)], ret())],
    });
    assert_eq!(Machine::load(b.program, &[]).map(|_| ()), Err(at_statement("no such variant")));
}

/// d06: `switchInt(const true)` takes the branch its constant names.
#[test]
fn a_switch_on_a_constant_takes_its_constant_s_branch() {
    let mut b = Build::default();
    let (u8t, boolean) = (b.int(Int::U8), b.bool());
    let f = b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[u8t]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: konst(boolean, vec![Scalar::Bool(true)]), targets: vec![(0, BlockId(2))], otherwise: BlockId(1), at: at(1) }),
            block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 1)])), 2)], ret()),
            block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 2)])), 3)], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(m.call(f, &[]).unwrap(), &[int(Int::U8, 1)]);
}

/// d14: the last-listed variant's slots past its fields unwritten too (`E { A = 0, B(u8, bool) = 5, C(i64) = -3 }`).
#[test]
fn the_last_listed_variant_s_slots_past_its_fields_must_be_unwritten() {
    let mut b = Build::default();
    let (u8t, boolean, i64t) = (b.int(Int::U8), b.bool(), b.int(Int::I64));
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant { name: "B".into(), discriminant: 5, fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: boolean }] },
            Variant { name: "C".into(), discriminant: -3, fields: vec![Field { name: "0".into(), ty: i64t }] },
        ],
    });
    let mut m = Machine::load(b.program, &[e]).unwrap();
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, -3), int(Int::I64, -9), Scalar::Uninit]), Ok(()));
    assert_eq!(m.set_region(RegionId(0), &[int(Int::Isize, -3), int(Int::I64, -9), Scalar::Bool(true)]), Err(HostError::Slots), "C(-9) with B's bool left behind");
}
