//! Nothing is allocated once a program is loaded: an allocator that counts what this thread asks for while the
//! machine runs a program's entries, failed ones among them, and stops them midway.
mod common;
use common::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use orcl_interp::*;

struct Counting;

static ASKED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
}

fn note() {
    if WATCHED.with(|w| w.get()) {
        ASKED.fetch_add(1, Ordering::Relaxed);
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
    let before = ASKED.load(Ordering::Relaxed);
    WATCHED.with(|w| w.set(true));
    let out = f();
    WATCHED.with(|w| w.set(false));
    (out, ASKED.load(Ordering::Relaxed) - before)
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
