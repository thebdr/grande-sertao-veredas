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
    // the bytes asked for, while watched
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    if WATCHED.with(|w| w.get()) {
        ASKED.with(|a| a.set(a.get() + 1));
    }
}

fn note_bytes(n: usize) {
    if WATCHED.with(|w| w.get()) {
        BYTES.with(|b| b.set(b.get().saturating_add(n)));
    }
}

// SAFETY: every call is handed to the system's allocator unchanged; counting touches no memory it returns.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        note_bytes(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        note_bytes(new_size);
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
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
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
    // a goto, the commonest terminator
    let f = b.function(Function {
        name: "goto".into(),
        params: 0,
        locals: locals(&[u8t]),
        blocks: vec![block(vec![], Terminator::Goto(BlockId(1))), block(vec![assign(place(0), Value::Use(c8(3)), 1)], ret())],
    });
    entries.push((f, vec![]));
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
    let mut m = Machine::load(b.program, &[optd], STACK).unwrap();
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

#[test]
fn loading_a_program_with_a_huge_array_aggregate_asks_for_no_memory_of_the_array_s_size() {
    // `[u8; u32::MAX]` built from no part: refused for its parts' count, never by building a list as long
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let huge = b.ty(Type::Array { element: u8t, len: u32::MAX });
    let unit = b.unit();
    let r = b.reference(huge, true);
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[unit, r]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref]), Value::Aggregate(huge, None, vec![]), 1)], ret())],
    });
    let program = b.program;
    let before = BYTES.with(|b| b.get());
    let (got, _) = watching(|| Machine::load(program, &[], STACK).map(|_| ()));
    let asked = BYTES.with(|b| b.get()) - before;
    assert_eq!(got, Err(LoadError::Instruction { function: "f".into(), block: 0, statement: Some(0), reason: "an aggregate with too many or too few parts" }));
    assert!(asked < 1 << 20, "{asked} bytes asked for while loading");
}

#[test]
fn an_entry_returning_more_slots_than_the_first_function_allocates_nothing_in_a_machine_or_its_copy() {
    // function 0 returns (); function 1 returns (u8, u8, u8): the result's room is the largest return's
    let mut b = Build::default();
    let (unit, u8t) = (b.unit(), b.int(Int::U8));
    let triple = b.tuple(&[u8t, u8t, u8t]);
    b.function(Function { name: "first".into(), params: 0, locals: locals(&[unit]), blocks: vec![block(vec![], ret())] });
    let f = b.function(Function {
        name: "three".into(),
        params: 1,
        locals: locals(&[triple, u8t]),
        blocks: vec![block(vec![assign(place(0), Value::Aggregate(triple, None, vec![copy(1), copy(1), copy(1)]), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    let mut copy = m.clone();
    let arg = [int(Int::U8, 7)];
    let want = [int(Int::U8, 7); 3];
    let (got, asked) = watching(|| m.call(f, &[Arg::Value(&arg)]).map(|r| r == want.as_slice()));
    assert_eq!((got, asked), (Ok(true), 0), "the machine's first return");
    let (got, asked) = watching(|| copy.call(f, &[Arg::Value(&arg)]).map(|r| r == want.as_slice()));
    assert_eq!((got, asked), (Ok(true), 0), "the copy's first return");
}

#[test]
fn a_type_the_program_only_declares_asks_for_no_room_at_load() {
    // [u8; u32::MAX] declared and used by no local: no scratch of its size, which would be 128 GiB
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let _ = b.ty(Type::Array { element: u8t, len: u32::MAX });
    let f = b.function(Function { name: "f".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, 3)])), 1)], ret())] });
    let program = b.program;
    let before = BYTES.with(|b| b.get());
    let (m, _) = watching(|| Machine::load(program, &[], STACK));
    let asked = BYTES.with(|b| b.get()) - before;
    assert!(asked < 1 << 20, "{asked} bytes asked for while loading");
    assert_eq!(m.unwrap().call(f, &[]).unwrap(), &[int(Int::U8, 3)]);
}

#[test]
fn a_region_s_part_copied_whole_through_a_reference_allocates_nothing_in_a_machine_or_its_copy() {
    // struct S { prev: [u8; 64], cur: [u8; 64] }  fn scan(s: &mut S) { s.prev = s.cur }: no local of that size;
    // and regions u8 and [u8; 64], `*a = *b` on the second, the larger of two
    let mut b = Build::default();
    let (u8t, unit) = (b.int(Int::U8), b.unit());
    let arr = b.ty(Type::Array { element: u8t, len: 64 });
    let s = b.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "prev".into(), ty: arr }, Field { name: "cur".into(), ty: arr }] });
    let (rs, ra, rb) = (b.reference(s, true), b.reference(arr, true), b.reference(arr, false));
    let scan = b.function(Function {
        name: "scan".into(),
        params: 1,
        locals: locals(&[unit, rs]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref, Projection::Field(0)]), Value::Use(copy_at(proj(1, vec![Projection::Deref, Projection::Field(1)]))), 1)], ret())],
    });
    let whole = b.function(Function {
        name: "whole".into(),
        params: 2,
        locals: locals(&[unit, ra, rb]),
        blocks: vec![block(vec![assign(proj(1, vec![Projection::Deref]), Value::Use(copy_at(proj(2, vec![Projection::Deref]))), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[u8t, s, arr], STACK).unwrap();
    let mut slots = vec![int(Int::U8, 0); 64];
    slots.extend((0..64).map(|i| int(Int::U8, i)));
    m.set_region(RegionId(1), &slots).unwrap();
    m.set_region(RegionId(2), &(0..64).map(|i| int(Int::U8, 100 + i)).collect::<Vec<_>>()).unwrap();
    let mut copy = m.clone();
    for (who, machine) in [("the machine", &mut m), ("its copy", &mut copy)] {
        let (got, asked) = watching(|| {
            let a = machine.call(scan, &[Arg::Ref(RegionId(1))]).map(|r| r.len());
            let b = machine.call(whole, &[Arg::Ref(RegionId(2)), Arg::Ref(RegionId(2))]).map(|r| r.len());
            (a, b)
        });
        assert_eq!((got, asked), ((Ok(0), Ok(0)), 0), "{who}'s first scan");
        let region = machine.region(RegionId(1)).unwrap();
        assert_eq!(&region[..64], &region[64..], "prev = cur in {who}");
    }
}

// The twentieth refute round's tests: a copy of a machine taken mid-run, at every instruction of a chain of calls
// and after a small value was built, allocates nothing as it goes on, as a copy of an idle machine does.

/// A chain of calls `depth` deep, f1 calling f2 ... calling f{depth}, each adding one to what its callee gives.
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

/// A copy taken at every instruction of a chain of calls five deep, run to the end: no allocation, and the
/// result the original gives.
#[test]
fn a_copy_taken_at_any_instruction_of_a_chain_of_calls_allocates_nothing_as_it_goes_on() {
    let (p, entry) = chain(5);
    let mut m = Machine::load(p, &[], STACK).unwrap();
    m.start(entry, &[]).unwrap();
    let mut k = 0;
    loop {
        let mut copy = m.clone();
        let (got, asked) = watching(|| copy.run(u64::MAX));
        assert_eq!((got, asked, copy.result()), (Ok(Step::Returned), 0, &[int(Int::U8, 11)][..]), "a copy taken after {k} instructions, {} calls deep", m.depth());
        k += 1;
        if m.step().unwrap() == Step::Returned {
            break;
        }
    }
    assert!(k > 10, "{k} instructions");
}

/// A copy taken after an instruction built a small value, going on to build a larger one: no allocation.
#[test]
fn a_copy_taken_after_a_small_value_was_built_allocates_nothing_building_a_larger_one() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let arr = b.ty(Type::Array { element: u8t, len: 16 });
    let f = b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[u8t, u8t, arr]),
        blocks: vec![block(
            vec![
                assign(place(1), Value::Use(konst(u8t, vec![int(Int::U8, 3)])), 1),
                assign(place(2), Value::Use(konst(arr, vec![int(Int::U8, 5); 16])), 2),
                assign(place(0), Value::Use(copy_at(proj(2, vec![Projection::ConstantIndex(15)]))), 3),
            ],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    m.start(f, &[]).unwrap();
    for k in 0..4 {
        let mut copy = m.clone();
        let (got, asked) = watching(|| copy.run(u64::MAX));
        assert_eq!((got, asked, copy.result()), (Ok(Step::Returned), 0, &[int(Int::U8, 5)][..]), "a copy taken after {k} instructions");
        m.step().unwrap();
    }
}

// The twenty-first refute round's test: the larger region listed first, copied in part through a reference.

#[test]
fn the_larger_region_listed_first_and_copied_in_part_through_a_reference_allocates_nothing() {
    // regions: S { cur: [u8; 32], prev: [u8; 32] } first, then a u8; fn keep(s: &mut S, n: &mut u8) { (*s).prev = (*s).cur }
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let arr = b.ty(Type::Array { element: u8t, len: 32 });
    let s = b.tuple(&[arr, arr]);
    let unit = b.unit();
    let (rs, rn) = (b.reference(s, true), b.reference(u8t, true));
    let keep = b.function(Function {
        name: "keep".into(),
        params: 2,
        locals: locals(&[unit, rs, rn]),
        blocks: vec![block(
            vec![assign(proj(1, vec![Projection::Deref, Projection::Field(1)]), Value::Use(copy_at(proj(1, vec![Projection::Deref, Projection::Field(0)]))), 1)],
            ret(),
        )],
    });
    let mut m = Machine::load(b.program, &[s, u8t], STACK).unwrap();
    let state: Vec<Scalar> = (0..64).map(|i| int(Int::U8, i % 32)).collect();
    m.set_region(RegionId(0), &state).unwrap();
    m.set_region(RegionId(1), &[int(Int::U8, 0)]).unwrap();
    let mut copy = m.clone();
    let (got, asked) = watching(|| m.call(keep, &[Arg::Ref(RegionId(0)), Arg::Ref(RegionId(1))]).map(|r| r.to_vec()));
    assert_eq!((got, asked), (Ok(vec![]), 0), "the machine");
    let (got, asked) = watching(|| copy.call(keep, &[Arg::Ref(RegionId(0)), Arg::Ref(RegionId(1))]).map(|r| r.to_vec()));
    assert_eq!((got, asked), (Ok(vec![]), 0), "its copy");
    assert_eq!(m.region(RegionId(0)), Some(&state[..32].repeat(2)[..]), "prev is cur");
}

// The twenty-second refute round's tests: the largest of three, listed between the other two, for a callee, an
// entry's result and a region.

/// `main` calls `small`, `large` and `small` again: the largest in slots in the middle of its calls.
#[test]
fn a_function_whose_largest_callee_is_called_between_two_smaller_has_room_for_it() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let big = b.ty(Type::Array { element: u8t, len: 64 });
    let leaf = |b: &mut Build, name: &str, extra: &[TypeId], v: i128| {
        let mut types = vec![u8t];
        types.extend_from_slice(extra);
        b.function(Function { name: name.into(), params: 0, locals: locals(&types), blocks: vec![block(vec![assign(place(0), Value::Use(konst(u8t, vec![int(Int::U8, v)])), 1)], ret())] })
    };
    let small = leaf(&mut b, "small", &[], 1);
    let large = leaf(&mut b, "large", &[big], 2);
    let call = |f, dest, next| Terminator::Call { callee: Callee::Function(f), args: vec![], dest: place(dest), next: BlockId(next), at: at(1) };
    let main = b.function(Function {
        name: "main".into(),
        params: 0,
        locals: locals(&[u8t, u8t, u8t]),
        blocks: vec![block(vec![], call(small, 1, 1)), block(vec![], call(large, 2, 2)), block(vec![], call(small, 1, 3)), block(vec![assign(place(0), Value::Use(copy(2)), 2)], ret())],
    });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    let got = m.call(main, &[]).map(|r| r.to_vec());
    assert_eq!(got, Ok(vec![int(Int::U8, 2)]));
}

/// Three functions, the one returning the most slots in the middle, called as the entry with nothing allocated.
#[test]
fn an_entry_returning_the_most_slots_listed_between_two_others_allocates_nothing() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let wide = b.ty(Type::Array { element: u8t, len: 8 });
    let c8 = |v| konst(u8t, vec![int(Int::U8, v)]);
    b.function(Function { name: "first".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(c8(1)), 1)], ret())] });
    let mid = b.function(Function {
        name: "mid".into(),
        params: 0,
        locals: locals(&[wide]),
        blocks: vec![block(vec![assign(place(0), Value::Aggregate(wide, None, (0..8).map(c8).collect()), 1)], ret())],
    });
    b.function(Function { name: "last".into(), params: 0, locals: locals(&[u8t]), blocks: vec![block(vec![assign(place(0), Value::Use(c8(1)), 1)], ret())] });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    let (got, asked) = watching(|| m.call(mid, &[]).map(|r| r.len()));
    assert_eq!((got, asked), (Ok(8), 0));
}

/// Four regions, the two largest in the middle, one copied whole into the other through references: nothing allocated.
#[test]
fn the_largest_of_three_regions_listed_in_the_middle_copied_whole_allocates_nothing() {
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let wide = b.ty(Type::Array { element: u8t, len: 8 });
    let (rw, unit) = (b.reference(wide, true), b.unit());
    // fn copy(a: &mut [u8; 8], b: &mut [u8; 8]) { *b = *a } with no local of the array's size
    let f = b.function(Function {
        name: "copy".into(),
        params: 2,
        locals: locals(&[unit, rw, rw]),
        blocks: vec![block(vec![assign(proj(2, vec![Projection::Deref]), Value::Use(copy_at(proj(1, vec![Projection::Deref]))), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[u8t, wide, wide, u8t], STACK).unwrap();
    m.set_region(RegionId(1), &(0..8).map(|v| int(Int::U8, v)).collect::<Vec<_>>()).unwrap();
    let (got, asked) = watching(|| m.call(f, &[Arg::Ref(RegionId(1)), Arg::Ref(RegionId(2))]).map(|_| ()));
    assert_eq!((got, asked), (Ok(()), 0));
    assert_eq!(m.region(RegionId(2)), m.region(RegionId(1)));
}

/// A function calling itself, run to the stack's edge and past it, and a copy taken at every depth: no allocation
/// once loaded, the stack's frames laid out at load whatever the recursion (Q230).
#[test]
fn a_function_calling_itself_to_the_stack_s_edge_and_past_it_allocates_nothing_nor_does_its_copy() {
    let mut b = Build::default();
    let f = sum(&mut b);
    let mut m = Machine::load(b.program, &[], 50).unwrap();
    let nine = [int(Int::U32, 9)];
    let ten = [int(Int::U32, 10)];
    let (got, asked) = watching(|| m.call(f, &[Arg::Value(&nine)]).map(|r| r.len()));
    assert_eq!((got, asked, m.result()), (Ok(1), 0, &[int(Int::U32, 45)][..]));
    let (got, asked) = watching(|| m.call(f, &[Arg::Value(&ten)]).map(|r| r.len()));
    assert!(matches!(got, Err(CallError::Program(ProgramError { kind: ErrorKind::Stack, .. }))), "{got:?}");
    assert_eq!(asked, 0);
    m.abort();
    m.start(f, &[Arg::Value(&nine)]).unwrap();
    loop {
        let mut copy = m.clone();
        let (got, asked) = watching(|| copy.run(u64::MAX));
        assert_eq!((got, asked, copy.result()), (Ok(Step::Returned), 0, &[int(Int::U32, 45)][..]), "{} calls deep", m.depth());
        if m.step().unwrap() == Step::Returned {
            break;
        }
    }
}
