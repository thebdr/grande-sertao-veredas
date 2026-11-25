//! Nothing is allocated once a program is loaded: an allocator that counts what this thread asks for while the
//! machine runs a program's entries, failed ones among them, and stops them midway.
mod common;
use common::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use orcl_interp::*;

struct Counting;

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
    // this thread's own count, so a test running beside it on another thread never adds to it
    static ASKED: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    if WATCHED.with(|w| w.get()) {
        ASKED.with(|a| a.set(a.get() + 1));
    }
}

// SAFETY: every call is handed to the system's allocator unchanged; counting touches no memory it returns.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn watching<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = ASKED.with(|a| a.get());
    WATCHED.with(|w| w.set(true));
    let out = f();
    WATCHED.with(|w| w.set(false));
    (out, ASKED.with(|a| a.get()) - before)
}

#[test]
fn running_entries_calls_core_operations_and_failures_allocate_nothing() {
    // fn deep(n: u32) -> u32 { mid(n) }, fn mid(n) -> u32 { leaf(n) }, fn leaf(n) -> u32 { n.saturating_add(n) * 3 }
    // with the multiplication checked, so a large n fails at its assertion three calls down
    let mut b = Build::default();
    let (u32t, boolean) = (b.int(Int::U32), b.bool());
    let pair = b.tuple(&[u32t, boolean]);
    let leaf = b.function(Function {
        name: "leaf".into(),
        params: 1,
        locals: locals(&[u32t, u32t, u32t, pair]),
        blocks: vec![
            block(
                vec![],
                Terminator::Call {
                    callee: Callee::Core(CoreOp::SaturatingAdd(Int::U32)),
                    args: vec![copy(1), copy(1)],
                    dest: place(2),
                    next: BlockId(1),
                    at: at(1),
                },
            ),
            block(
                vec![assign(place(3), Value::Checked(BinOp::Mul, copy(2), konst(u32t, vec![int(Int::U32, 3)])), 2)],
                Terminator::Assert {
                    cond: copy_at(proj(3, vec![Projection::Field(1)])),
                    expected: false,
                    check: Check::Overflow(BinOp::Mul),
                    next: BlockId(2),
                    at: at(2),
                },
            ),
            block(vec![assign(place(0), Value::Use(copy_at(proj(3, vec![Projection::Field(0)]))), 3)], ret()),
        ],
    });
    let pass = |b: &mut Build, name: &str, callee: FunctionId| {
        b.function(Function {
            name: name.into(),
            params: 1,
            locals: locals(&[u32t, u32t]),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Function(callee), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(4) }),
                block(vec![], ret()),
            ],
        })
    };
    let mid = pass(&mut b, "mid", leaf);
    let deep = pass(&mut b, "deep", mid);
    let mut m = Machine::load(b.program, &[]).unwrap();
    let args: Vec<[Scalar; 1]> = (0..2000).map(|n| [int(Int::U32, n)]).collect();
    let big = [int(Int::U32, 1 << 30)];
    let ((), asked) = watching(|| {
        for arg in &args {
            let r = m.call(deep, &[Arg::Value(arg)]).unwrap();
            assert_eq!(r.len(), 1);
        }
        for _ in 0..100 {
            assert!(m.call(deep, &[Arg::Value(&big)]).is_err(), "an overflow three calls down");
            m.abort();
        }
        for arg in args.iter().take(100) {
            m.start(deep, &[Arg::Value(arg)]).unwrap();
            m.run(3).unwrap();
            m.abort();
        }
    });
    assert_eq!(asked, 0, "allocations while running");
    assert_eq!(m.call(deep, &[Arg::Value(&args[7])]).unwrap(), &[int(Int::U32, 42)]);
}

fn core_caller(b: &mut Build, op: CoreOp, result: TypeId, params: &[TypeId]) -> FunctionId {
    let mut types = vec![result];
    types.extend_from_slice(params);
    let args = (1..=params.len() as u32).map(copy).collect();
    b.function(Function {
        name: format!("{op:?}"),
        params: params.len() as u32,
        locals: locals(&types),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(op), args, dest: place(0), next: BlockId(1), at: at(1) }),
            block(vec![], ret()),
        ],
    })
}

#[test]
fn every_kind_of_instruction_and_every_core_operation_allocates_nothing_in_a_machine_or_its_copy() {
    let mut b = Build::default();
    let (u8t, u32t, u64t, i16t, boolean, usize_t, f32t, d, unit) = (
        b.int(Int::U8),
        b.int(Int::U32),
        b.int(Int::U64),
        b.int(Int::I16),
        b.bool(),
        b.int(Int::Usize),
        b.ty(Type::F32),
        b.ty(Type::Duration),
        b.unit(),
    );
    let pair = b.tuple(&[u8t, u8t]);
    let arr = b.ty(Type::Array { element: u8t, len: 4 });
    let e = b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::Isize,
        variants: vec![
            Variant { name: "A".into(), discriminant: 0, fields: vec![] },
            Variant { name: "B".into(), discriminant: 3, fields: vec![Field { name: "0".into(), ty: u8t }] },
        ],
    });
    let (optu8, optd, isize_t) = (b.option(u8t), b.option(d), b.int(Int::Isize));
    let (sd, ropt, sopt, rd, ru8) =
        (b.reference(d, false), b.reference(optd, true), b.reference(optd, false), b.reference(d, true), b.reference(u8t, true));
    let c8 = |v| konst(u8t, vec![int(Int::U8, v)]);
    let mut entries: Vec<(FunctionId, Vec<Vec<Scalar>>)> = vec![];
    // switch and aggregate: fn f(a, c, flag) -> (u8, u8) { if flag { (c, a) } else { (a, c) } }
    let f = b.function(Function {
        name: "switch".into(),
        params: 3,
        locals: locals(&[pair, u8t, u8t, boolean]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: copy(3), targets: vec![(0, BlockId(2))], otherwise: BlockId(1), at: at(1) }),
            block(vec![assign(place(0), Value::Aggregate(pair, None, vec![copy(2), copy(1)]), 2)], ret()),
            block(vec![assign(place(0), Value::Aggregate(pair, None, vec![copy(1), copy(2)]), 3)], ret()),
        ],
    });
    entries.push((f, vec![vec![int(Int::U8, 1)], vec![int(Int::U8, 2)], vec![Scalar::Bool(true)]]));
    entries.push((f, vec![vec![int(Int::U8, 1)], vec![int(Int::U8, 2)], vec![Scalar::Bool(false)]]));
    // casts, unary operations, a plain operation and a checked one with its assertion
    let checked = b.tuple(&[u8t, boolean]);
    let f = b.function(Function {
        name: "arith".into(),
        params: 1,
        locals: locals(&[f32t, u8t, f32t, i16t, checked, u8t]),
        blocks: vec![
            block(
                vec![
                    assign(place(2), Value::Cast(copy(1), f32t), 1),
                    assign(place(3), Value::Cast(copy(1), i16t), 1),
                    assign(place(3), Value::Unary(UnOp::Neg, copy(3)), 1),
                    assign(place(5), Value::Unary(UnOp::Not, copy(1)), 1),
                    assign(place(5), Value::Binary(BinOp::Shl, copy(5), konst(u32t, vec![int(Int::U32, 3)])), 1),
                    assign(place(4), Value::Checked(BinOp::Add, copy(1), c8(1)), 2),
                ],
                Terminator::Assert { cond: copy_at(proj(4, vec![Projection::Field(1)])), expected: false, check: Check::Overflow(BinOp::Add), next: BlockId(1), at: at(2) },
            ),
            block(vec![assign(place(0), Value::Unary(UnOp::Neg, copy(2)), 3)], ret()),
        ],
    });
    entries.push((f, vec![vec![int(Int::U8, 9)]]));
    // an enum's variant built, its discriminant read and switched on, its field read; an array indexed two ways;
    // a reference to a local written through by a callee
    let bump = b.function(Function {
        name: "bump".into(),
        params: 1,
        locals: locals(&[unit, ru8]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref]), Value::Binary(BinOp::Add, copy_at(proj(1, vec![Projection::Deref])), c8(1)), 1)], ret())],
    });
    let f = b.function(Function {
        name: "places".into(),
        params: 2,
        locals: locals(&[u8t, u8t, usize_t, e, isize_t, arr, u8t, ru8, unit]),
        blocks: vec![
            block(
                vec![
                    assign(place(3), Value::Aggregate(e, Some(1), vec![copy(1)]), 1),
                    assign(place(4), Value::Discriminant(place(3)), 1),
                    assign(place(5), Value::Aggregate(arr, None, vec![c8(10), c8(20), c8(30), c8(40)]), 2),
                    assign(place(6), Value::Use(copy_at(proj(5, vec![Projection::Index(Local(2))]))), 2),
                    assign(proj(5, vec![Projection::ConstantIndex(0)]), Value::Use(copy(6)), 2),
                    assign(place(7), Value::Ref(place(6), true), 3),
                ],
                Terminator::SwitchInt { value: copy(4), targets: vec![(3, BlockId(1))], otherwise: BlockId(3), at: at(1) },
            ),
            block(vec![], Terminator::Call { callee: Callee::Function(bump), args: vec![copy(7)], dest: place(8), next: BlockId(2), at: at(3) }),
            block(
                vec![assign(place(0), Value::Binary(BinOp::Add, copy_at(proj(3, vec![Projection::Downcast(1), Projection::Field(0)])), copy(6)), 4)],
                ret(),
            ),
            block(vec![], Terminator::Unreachable { at: at(5) }),
        ],
    });
    entries.push((f, vec![vec![int(Int::U8, 5)], vec![int(Int::Usize, 2)]]));
    // every operation of the closed list, by value where core's function takes a value
    for (op, result, params, args) in [
        (CoreOp::SaturatingAdd(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 200)], vec![int(Int::U8, 100)]]),
        (CoreOp::SaturatingSub(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 2)], vec![int(Int::U8, 5)]]),
        (CoreOp::SaturatingMul(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 20)], vec![int(Int::U8, 50)]]),
        (CoreOp::WrappingAdd(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 200)], vec![int(Int::U8, 100)]]),
        (CoreOp::WrappingSub(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 2)], vec![int(Int::U8, 5)]]),
        (CoreOp::WrappingMul(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 20)], vec![int(Int::U8, 50)]]),
        (CoreOp::CheckedShl(Int::U8), optu8, vec![u8t, u32t], vec![vec![int(Int::U8, 3)], vec![int(Int::U32, 2)]]),
        (CoreOp::CheckedShr(Int::U8), optu8, vec![u8t, u32t], vec![vec![int(Int::U8, 3)], vec![int(Int::U32, 9)]]),
        (CoreOp::Min(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 3)], vec![int(Int::U8, 9)]]),
        (CoreOp::Max(Int::U8), u8t, vec![u8t, u8t], vec![vec![int(Int::U8, 3)], vec![int(Int::U8, 9)]]),
        (CoreOp::DurationFromMillis, d, vec![u64t], vec![vec![int(Int::U64, 1500)]]),
        (CoreOp::DurationFromSecs, d, vec![u64t], vec![vec![int(Int::U64, 15)]]),
        (CoreOp::Default, d, vec![], vec![]),
    ] {
        entries.push((core_caller(&mut b, op, result, &params), args));
    }
    let two_d = vec![vec![int(Int::U64, 3), int(Int::U32, 5)], vec![int(Int::U64, 1), int(Int::U32, 900_000_000)]];
    for op in [CoreOp::DurationSaturatingSub, CoreOp::DurationMin, CoreOp::DurationMax] {
        entries.push((core_caller(&mut b, op, d, &[d, d]), two_d.clone()));
    }
    // a comparison through references, and an option filled and asked through its reference to a region
    let cmp = b.function(Function {
        name: "ge".into(),
        params: 2,
        locals: locals(&[boolean, d, d, sd, sd]),
        blocks: vec![
            block(
                vec![assign(place(3), Value::Ref(place(1), false), 1), assign(place(4), Value::Ref(place(2), false), 1)],
                Terminator::Call { callee: Callee::Core(CoreOp::DurationCompare(BinOp::Ge)), args: vec![copy(3), copy(4)], dest: place(0), next: BlockId(1), at: at(1) },
            ),
            block(vec![], ret()),
        ],
    });
    entries.push((cmp, two_d.clone()));
    let opt_fn = b.function(Function {
        name: "opt".into(),
        params: 2,
        locals: locals(&[boolean, ropt, d, rd, sopt, boolean]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionGetOrInsert), args: vec![copy(1), copy(2)], dest: place(3), next: BlockId(1), at: at(1) }),
            block(
                vec![assign(place(4), Value::Ref(proj(1, vec![Projection::Deref]), false), 2)],
                Terminator::Call { callee: Callee::Core(CoreOp::OptionIsNone), args: vec![copy(4)], dest: place(5), next: BlockId(2), at: at(2) },
            ),
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionIsSome), args: vec![copy(4)], dest: place(0), next: BlockId(3), at: at(3) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[optd]).unwrap();
    let region = RegionId(0);
    let none = [int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit];
    let a_duration = [int(Int::U64, 2), int(Int::U32, 0)];
    let run_all = |m: &mut Machine| {
        for _ in 0..50 {
            for (f, args) in &entries {
                let args: [Arg; 3] = [
                    Arg::Value(args.first().map_or(&[][..], |a| a.as_slice())),
                    Arg::Value(args.get(1).map_or(&[][..], |a| a.as_slice())),
                    Arg::Value(args.get(2).map_or(&[][..], |a| a.as_slice())),
                ];
                let n = m.program().functions[f.0 as usize].params as usize;
                assert!(m.call(*f, &args[..n]).is_ok());
            }
            m.set_region(region, &none).unwrap();
            assert_eq!(m.call(opt_fn, &[Arg::Ref(region), Arg::Value(&a_duration)]).unwrap(), &[Scalar::Bool(true)]);
            assert_eq!(m.call(opt_fn, &[Arg::Ref(region), Arg::Value(&a_duration)]).unwrap(), &[Scalar::Bool(true)]);
        }
    };
    let ((), asked) = watching(|| run_all(&mut m));
    assert_eq!(asked, 0, "allocations while every kind of instruction runs");
    let mut copy = m.clone();
    let ((), asked) = watching(|| run_all(&mut copy));
    assert_eq!(asked, 0, "allocations while a copy of the machine runs them");
}
