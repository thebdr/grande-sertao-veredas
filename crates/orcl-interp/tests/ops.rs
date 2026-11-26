//! Every operation MIR has on scalars gives what the compiled program gives: each run in the machine and compared
//! with Rust's own operation on the native type, for every integer type at its bounds, their neighbours, small
//! values and values drawn at random, and for every pair of 8-bit values.
mod common;
use common::*;
use orcl_interp::*;

const BINARY: [BinOp; 16] = [
    BinOp::Add,
    BinOp::Sub,
    BinOp::Mul,
    BinOp::Div,
    BinOp::Rem,
    BinOp::BitAnd,
    BinOp::BitOr,
    BinOp::BitXor,
    BinOp::Shl,
    BinOp::Shr,
    BinOp::Eq,
    BinOp::Ne,
    BinOp::Lt,
    BinOp::Le,
    BinOp::Gt,
    BinOp::Ge,
];

fn comparison(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// What Rust gives for `a op b` on the native type: an integer, a `bool`, or the check MIR fails.
#[derive(Debug, PartialEq)]
enum Native {
    Int(i128),
    Bool(bool),
    Fails(Check),
}

macro_rules! native_binary {
    ($t:ty, $op:expr, $a:expr, $b:expr) => {{
        let (a, b) = ($a as $t, $b as $t);
        match $op {
            BinOp::Add => Native::Int(a.wrapping_add(b) as i128),
            BinOp::Sub => Native::Int(a.wrapping_sub(b) as i128),
            BinOp::Mul => Native::Int(a.wrapping_mul(b) as i128),
            BinOp::Div if b == 0 => Native::Fails(Check::DivisionByZero),
            BinOp::Div => a.checked_div(b).map_or(Native::Fails(Check::Overflow(BinOp::Div)), |v| Native::Int(v as i128)),
            BinOp::Rem if b == 0 => Native::Fails(Check::RemainderByZero),
            BinOp::Rem => a.checked_rem(b).map_or(Native::Fails(Check::Overflow(BinOp::Rem)), |v| Native::Int(v as i128)),
            BinOp::BitAnd => Native::Int((a & b) as i128),
            BinOp::BitOr => Native::Int((a | b) as i128),
            BinOp::BitXor => Native::Int((a ^ b) as i128),
            BinOp::Eq => Native::Bool(a == b),
            BinOp::Ne => Native::Bool(a != b),
            BinOp::Lt => Native::Bool(a < b),
            BinOp::Le => Native::Bool(a <= b),
            BinOp::Gt => Native::Bool(a > b),
            BinOp::Ge => Native::Bool(a >= b),
            BinOp::Shl | BinOp::Shr => unreachable!("shifts are compared on their own"),
        }
    }};
}

fn native(t: Int, op: BinOp, a: i128, b: i128) -> Native {
    match t {
        Int::I8 => native_binary!(i8, op, a, b),
        Int::I16 => native_binary!(i16, op, a, b),
        Int::I32 => native_binary!(i32, op, a, b),
        Int::I64 | Int::Isize => native_binary!(i64, op, a, b),
        Int::U8 => native_binary!(u8, op, a, b),
        Int::U16 => native_binary!(u16, op, a, b),
        Int::U32 => native_binary!(u32, op, a, b),
        Int::U64 | Int::Usize => native_binary!(u64, op, a, b),
    }
}

/// MIR's shift: the amount taken modulo the width, which is what `wrapping_shl` and `wrapping_shr` do with the
/// amount's low 32 bits, the widths being powers of two.
macro_rules! native_shift {
    ($t:ty, $op:expr, $a:expr, $amount:expr) => {{
        let a = $a as $t;
        let amount = $amount as u32;
        if $op == BinOp::Shl { a.wrapping_shl(amount) as i128 } else { a.wrapping_shr(amount) as i128 }
    }};
}

fn native_shift(t: Int, op: BinOp, a: i128, amount: i128) -> i128 {
    match t {
        Int::I8 => native_shift!(i8, op, a, amount),
        Int::I16 => native_shift!(i16, op, a, amount),
        Int::I32 => native_shift!(i32, op, a, amount),
        Int::I64 | Int::Isize => native_shift!(i64, op, a, amount),
        Int::U8 => native_shift!(u8, op, a, amount),
        Int::U16 => native_shift!(u16, op, a, amount),
        Int::U32 => native_shift!(u32, op, a, amount),
        Int::U64 | Int::Usize => native_shift!(u64, op, a, amount),
    }
}

macro_rules! native_overflowing {
    ($t:ty, $op:expr, $a:expr, $b:expr) => {{
        let (a, b) = ($a as $t, $b as $t);
        let (v, o) = match $op {
            BinOp::Add => a.overflowing_add(b),
            BinOp::Sub => a.overflowing_sub(b),
            _ => a.overflowing_mul(b),
        };
        (v as i128, o)
    }};
}

fn native_checked(t: Int, op: BinOp, a: i128, b: i128) -> (i128, bool) {
    match t {
        Int::I8 => native_overflowing!(i8, op, a, b),
        Int::I16 => native_overflowing!(i16, op, a, b),
        Int::I32 => native_overflowing!(i32, op, a, b),
        Int::I64 | Int::Isize => native_overflowing!(i64, op, a, b),
        Int::U8 => native_overflowing!(u8, op, a, b),
        Int::U16 => native_overflowing!(u16, op, a, b),
        Int::U32 => native_overflowing!(u32, op, a, b),
        Int::U64 | Int::Usize => native_overflowing!(u64, op, a, b),
    }
}

macro_rules! cast_to {
    ($a:expr, $to:expr, $from:ty) => {{
        let a = $a as $from;
        match $to {
            Int::I8 => a as i8 as i128,
            Int::I16 => a as i16 as i128,
            Int::I32 => a as i32 as i128,
            Int::I64 | Int::Isize => a as i64 as i128,
            Int::U8 => a as u8 as i128,
            Int::U16 => a as u16 as i128,
            Int::U32 => a as u32 as i128,
            Int::U64 | Int::Usize => a as u64 as i128,
        }
    }};
}

fn native_cast(from: Int, to: Int, a: i128) -> i128 {
    match from {
        Int::I8 => cast_to!(a, to, i8),
        Int::I16 => cast_to!(a, to, i16),
        Int::I32 => cast_to!(a, to, i32),
        Int::I64 | Int::Isize => cast_to!(a, to, i64),
        Int::U8 => cast_to!(a, to, u8),
        Int::U16 => cast_to!(a, to, u16),
        Int::U32 => cast_to!(a, to, u32),
        Int::U64 | Int::Usize => cast_to!(a, to, u64),
    }
}

/// One function per operation: `fn f(a, b) -> r { _0 = value(a, b); return }`.
struct Ops {
    machine: Machine,
    binary: Vec<((Int, BinOp, Int), FunctionId)>,
    checked: Vec<((Int, BinOp), FunctionId)>,
    unary: Vec<((Int, UnOp), FunctionId)>,
    casts: Vec<((Int, Int), FunctionId)>,
}

fn two(b: &mut Build, result: TypeId, a: TypeId, c: TypeId, value: Value) -> FunctionId {
    b.function(Function {
        name: format!("op{}", b.program.functions.len()),
        params: 2,
        locals: locals(&[result, a, c]),
        blocks: vec![block(vec![assign(place(0), value, 1)], ret())],
    })
}

fn ops() -> Ops {
    let mut b = Build::default();
    let boolean = b.bool();
    let (mut binary, mut checked, mut unary, mut casts) = (vec![], vec![], vec![], vec![]);
    for t in INTS {
        let ty = b.int(t);
        for op in BINARY {
            let rhs_types: Vec<Int> = if matches!(op, BinOp::Shl | BinOp::Shr) { INTS.to_vec() } else { vec![t] };
            for r in rhs_types {
                let rty = b.int(r);
                let ret = if comparison(op) { boolean } else { ty };
                let f = two(&mut b, ret, ty, rty, Value::Binary(op, copy(1), copy(2)));
                binary.push(((t, op, r), f));
            }
        }
        for op in [BinOp::Add, BinOp::Sub, BinOp::Mul] {
            let pair = b.tuple(&[ty, boolean]);
            let f = two(&mut b, pair, ty, ty, Value::Checked(op, copy(1), copy(2)));
            checked.push(((t, op), f));
        }
        let ops: &[UnOp] = if t.signed() { &[UnOp::Not, UnOp::Neg] } else { &[UnOp::Not] };
        for op in ops {
            let f = b.function(Function {
                name: format!("un{}", b.program.functions.len()),
                params: 1,
                locals: locals(&[ty, ty]),
                blocks: vec![block(vec![assign(place(0), Value::Unary(*op, copy(1)), 1)], ret())],
            });
            unary.push(((t, *op), f));
        }
        for to in INTS {
            let toty = b.int(to);
            let f = b.function(Function {
                name: format!("cast{}", b.program.functions.len()),
                params: 1,
                locals: locals(&[toty, ty]),
                blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), toty), 1)], ret())],
            });
            casts.push(((t, to), f));
        }
    }
    let machine = Machine::load(b.program, &[], STACK).expect("the operations load");
    Ops { machine, binary, checked, unary, casts }
}

fn run(m: &mut Machine, f: FunctionId, args: &[Scalar]) -> Result<Vec<Scalar>, ErrorKind> {
    let args: Vec<Arg> = args.iter().map(|s| Arg::Value(std::slice::from_ref(s))).collect();
    match m.call(f, &args) {
        Ok(r) => Ok(r.to_vec()),
        Err(CallError::Program(e)) => {
            m.abort();
            Err(e.kind)
        }
        Err(CallError::Host(e)) => panic!("the host's call refused: {e:?}"),
    }
}

fn check_binary(m: &mut Machine, t: Int, op: BinOp, r: Int, f: FunctionId, a: i128, b: i128) {
    let got = run(m, f, &[int(t, a), int(r, b)]);
    let want = if matches!(op, BinOp::Shl | BinOp::Shr) {
        Native::Int(native_shift(t, op, a, b))
    } else {
        native(t, op, a, b)
    };
    let want = match want {
        Native::Int(v) => Ok(vec![int(t, v)]),
        Native::Bool(v) => Ok(vec![Scalar::Bool(v)]),
        Native::Fails(c) => Err(ErrorKind::Check(c)),
    };
    assert_eq!(got, want, "{a} {op:?} {b} on {t:?} (rhs {r:?})");
}

#[test]
fn every_binary_operation_on_every_integer_type_gives_what_rust_gives() {
    let mut o = ops();
    let mut rng = Rng(0x0BC1_D5EE_D000_0001);
    let mut tried = 0usize;
    for ((t, op, r), f) in o.binary.clone() {
        let lhs = samples(t, &mut rng, 12);
        let rhs = if matches!(op, BinOp::Shl | BinOp::Shr) {
            // every amount up to past the widest width, then the rhs type's own samples
            let mut v: Vec<i128> = (0..=130).filter(|x| r.holds(*x)).collect();
            v.extend(samples(r, &mut rng, 6));
            v
        } else {
            samples(r, &mut rng, 12)
        };
        for a in &lhs {
            for b in &rhs {
                check_binary(&mut o.machine, t, op, r, f, *a, *b);
                tried += 1;
            }
        }
    }
    assert!(tried > 100_000, "{tried} pairs tried");
}

#[test]
fn every_pair_of_8_bit_values_gives_what_rust_gives() {
    let mut o = ops();
    for ((t, op, r), f) in o.binary.clone() {
        if !(matches!(t, Int::I8 | Int::U8) && r == t) {
            continue;
        }
        for a in t.min()..=t.max() {
            for b in t.min()..=t.max() {
                check_binary(&mut o.machine, t, op, r, f, a, b);
            }
        }
    }
}

#[test]
fn checked_arithmetic_wraps_and_says_whether_it_overflowed_as_rust_does() {
    let mut o = ops();
    let mut rng = Rng(0x0BC1_D5EE_D000_0002);
    for ((t, op), f) in o.checked.clone() {
        let values = samples(t, &mut rng, 16);
        for a in &values {
            for b in &values {
                let (v, overflowed) = native_checked(t, op, *a, *b);
                let got = run(&mut o.machine, f, &[int(t, *a), int(t, *b)]);
                assert_eq!(got, Ok(vec![int(t, v), Scalar::Bool(overflowed)]), "{a} {op:?} {b} on {t:?}");
            }
        }
        if matches!(t, Int::I8 | Int::U8) {
            for a in t.min()..=t.max() {
                for b in t.min()..=t.max() {
                    let (v, overflowed) = native_checked(t, op, a, b);
                    let got = run(&mut o.machine, f, &[int(t, a), int(t, b)]);
                    assert_eq!(got, Ok(vec![int(t, v), Scalar::Bool(overflowed)]), "{a} {op:?} {b} on {t:?}");
                }
            }
        }
    }
}

#[test]
fn not_negation_and_every_cast_between_integers_give_what_rust_gives() {
    let mut o = ops();
    let mut rng = Rng(0x0BC1_D5EE_D000_0003);
    for ((t, op), f) in o.unary.clone() {
        for a in samples(t, &mut rng, 64) {
            let want = match op {
                UnOp::Not => native_cast(t, t, !a),
                UnOp::Neg => native_cast(t, t, native_checked(t, BinOp::Sub, 0, a).0),
            };
            assert_eq!(run(&mut o.machine, f, &[int(t, a)]), Ok(vec![int(t, want)]), "{op:?} {a} on {t:?}");
        }
    }
    for ((from, to), f) in o.casts.clone() {
        for a in samples(from, &mut rng, 64) {
            let want = native_cast(from, to, a);
            assert_eq!(run(&mut o.machine, f, &[int(from, a)]), Ok(vec![int(to, want)]), "{a} as {to:?} from {from:?}");
        }
    }
}

#[test]
fn bool_operations_give_what_rust_gives() {
    let mut b = Build::default();
    let boolean = b.bool();
    let ops = [BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge];
    let fs: Vec<FunctionId> =
        ops.iter().map(|op| two(&mut b, boolean, boolean, boolean, Value::Binary(*op, copy(1), copy(2)))).collect();
    let not = b.function(Function {
        name: "not".into(),
        params: 1,
        locals: locals(&[boolean, boolean]),
        blocks: vec![block(vec![assign(place(0), Value::Unary(UnOp::Not, copy(1)), 1)], ret())],
    });
    let u8t = b.int(Int::U8);
    let as_u8 = b.function(Function {
        name: "as_u8".into(),
        params: 1,
        locals: locals(&[u8t, boolean]),
        blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), u8t), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    for x in [false, true] {
        for y in [false, true] {
            let want = [x & y, x | y, x ^ y, x == y, x != y, !x & y, x <= y, x & !y, x >= y];
            for ((op, f), w) in ops.iter().zip(&fs).zip(want) {
                assert_eq!(run(&mut m, *f, &[Scalar::Bool(x), Scalar::Bool(y)]), Ok(vec![Scalar::Bool(w)]), "{x} {op:?} {y}");
            }
        }
        assert_eq!(run(&mut m, not, &[Scalar::Bool(x)]), Ok(vec![Scalar::Bool(!x)]));
        assert_eq!(run(&mut m, as_u8, &[Scalar::Bool(x)]), Ok(vec![int(Int::U8, x as i128)]));
    }
}

fn floats_f64() -> Vec<f64> {
    vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -2.5,
        1e-300,
        3.7e19,
        -9.3e18,
        1.8446744073709552e19,
        // 2^63 either side: i64's bounds as the nearest floats
        9.223_372_036_854_776e18,
        -9.223_372_036_854_776e18,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        255.9,
        -128.5,
        65535.99,
    ]
}

fn same_float(a: Scalar, b: Scalar) -> bool {
    match (a, b) {
        (Scalar::F32(x), Scalar::F32(y)) => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        (Scalar::F64(x), Scalar::F64(y)) => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

#[test]
fn float_arithmetic_comparison_and_casts_give_what_rust_gives() {
    let mut b = Build::default();
    let (f32t, f64t, boolean) = (b.ty(Type::F32), b.ty(Type::F64), b.bool());
    let arith = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem];
    let cmp = [BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge];
    let mut fns = vec![];
    for (ty, wide) in [(f32t, false), (f64t, true)] {
        for op in arith {
            fns.push((wide, op, two(&mut b, ty, ty, ty, Value::Binary(op, copy(1), copy(2)))));
        }
        for op in cmp {
            fns.push((wide, op, two(&mut b, boolean, ty, ty, Value::Binary(op, copy(1), copy(2)))));
        }
    }
    let neg64 = b.function(Function {
        name: "neg".into(),
        params: 1,
        locals: locals(&[f64t, f64t]),
        blocks: vec![block(vec![assign(place(0), Value::Unary(UnOp::Neg, copy(1)), 1)], ret())],
    });
    let mut to_int = vec![];
    let mut from_int = vec![];
    for t in INTS {
        let ty = b.int(t);
        for (fty, wide) in [(f32t, false), (f64t, true)] {
            let f = b.function(Function {
                name: format!("f2i{}", b.program.functions.len()),
                params: 1,
                locals: locals(&[ty, fty]),
                blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), ty), 1)], ret())],
            });
            to_int.push((t, wide, f));
            let g = b.function(Function {
                name: format!("i2f{}", b.program.functions.len()),
                params: 1,
                locals: locals(&[fty, ty]),
                blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), fty), 1)], ret())],
            });
            from_int.push((t, wide, g));
        }
    }
    let narrow = b.function(Function {
        name: "narrow".into(),
        params: 1,
        locals: locals(&[f32t, f64t]),
        blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), f32t), 1)], ret())],
    });
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    let values = floats_f64();
    for (wide, op, f) in &fns {
        for x in &values {
            for y in &values {
                let (sx, sy, want) = if *wide {
                    let w = match op {
                        BinOp::Add => Scalar::F64(x + y),
                        BinOp::Sub => Scalar::F64(x - y),
                        BinOp::Mul => Scalar::F64(x * y),
                        BinOp::Div => Scalar::F64(x / y),
                        BinOp::Rem => Scalar::F64(x % y),
                        BinOp::Eq => Scalar::Bool(x == y),
                        BinOp::Ne => Scalar::Bool(x != y),
                        BinOp::Lt => Scalar::Bool(x < y),
                        BinOp::Le => Scalar::Bool(x <= y),
                        BinOp::Gt => Scalar::Bool(x > y),
                        _ => Scalar::Bool(x >= y),
                    };
                    (Scalar::F64(*x), Scalar::F64(*y), w)
                } else {
                    let (x, y) = (*x as f32, *y as f32);
                    let w = match op {
                        BinOp::Add => Scalar::F32(x + y),
                        BinOp::Sub => Scalar::F32(x - y),
                        BinOp::Mul => Scalar::F32(x * y),
                        BinOp::Div => Scalar::F32(x / y),
                        BinOp::Rem => Scalar::F32(x % y),
                        BinOp::Eq => Scalar::Bool(x == y),
                        BinOp::Ne => Scalar::Bool(x != y),
                        BinOp::Lt => Scalar::Bool(x < y),
                        BinOp::Le => Scalar::Bool(x <= y),
                        BinOp::Gt => Scalar::Bool(x > y),
                        _ => Scalar::Bool(x >= y),
                    };
                    (Scalar::F32(x), Scalar::F32(y), w)
                };
                let got = run(&mut m, *f, &[sx, sy]).unwrap();
                assert!(got.len() == 1 && same_float(got[0], want), "{x} {op:?} {y}: {got:?} against {want:?}");
            }
        }
    }
    for x in &values {
        let got = run(&mut m, neg64, &[Scalar::F64(*x)]).unwrap();
        assert!(same_float(got[0], Scalar::F64(-x)));
        let got = run(&mut m, narrow, &[Scalar::F64(*x)]).unwrap();
        assert!(same_float(got[0], Scalar::F32(*x as f32)), "{x} as f32");
        for (t, wide, f) in &to_int {
            let arg = if *wide { Scalar::F64(*x) } else { Scalar::F32(*x as f32) };
            let v = if *wide { *x } else { (*x as f32) as f64 };
            let want = match t {
                Int::I8 => v as i8 as i128,
                Int::I16 => v as i16 as i128,
                Int::I32 => v as i32 as i128,
                Int::I64 | Int::Isize => v as i64 as i128,
                Int::U8 => v as u8 as i128,
                Int::U16 => v as u16 as i128,
                Int::U32 => v as u32 as i128,
                Int::U64 | Int::Usize => v as u64 as i128,
            };
            assert_eq!(run(&mut m, *f, &[arg]), Ok(vec![int(*t, want)]), "{v} as {t:?}");
        }
    }
    let mut rng = Rng(0x0BC1_D5EE_D000_0004);
    for (t, wide, f) in &from_int {
        for a in samples(*t, &mut rng, 64) {
            let want = match (t, wide) {
                (Int::I8 | Int::I16 | Int::I32 | Int::I64 | Int::Isize, true) => Scalar::F64(a as i64 as f64),
                (_, true) => Scalar::F64(a as u64 as f64),
                (Int::I8 | Int::I16 | Int::I32 | Int::I64 | Int::Isize, false) => Scalar::F32(a as i64 as f32),
                (_, false) => Scalar::F32(a as u64 as f32),
            };
            let got = run(&mut m, *f, &[int(*t, a)]).unwrap();
            assert!(same_float(got[0], want), "{a} of {t:?} as a float: {got:?} against {want:?}");
        }
    }
}

#[test]
fn every_float_negation_and_cast_between_floats_and_from_bool_gives_what_rust_gives() {
    let mut b = Build::default();
    let (f32t, f64t, boolean) = (b.ty(Type::F32), b.ty(Type::F64), b.bool());
    let one = |b: &mut Build, result: TypeId, arg: TypeId, value: Value| {
        b.function(Function {
            name: format!("f{}", b.program.functions.len()),
            params: 1,
            locals: locals(&[result, arg]),
            blocks: vec![block(vec![assign(place(0), value, 1)], ret())],
        })
    };
    let neg32 = one(&mut b, f32t, f32t, Value::Unary(UnOp::Neg, copy(1)));
    let casts = [(f32t, f32t), (f32t, f64t), (f64t, f64t), (f64t, f32t)];
    let cast_fns: Vec<(TypeId, TypeId, FunctionId)> =
        casts.iter().map(|(from, to)| (*from, *to, one(&mut b, *to, *from, Value::Cast(copy(1), *to)))).collect();
    let from_bool: Vec<(Int, FunctionId)> = INTS
        .iter()
        .map(|t| {
            let ty = b.int(*t);
            (*t, one(&mut b, ty, boolean, Value::Cast(copy(1), ty)))
        })
        .collect();
    let bool_bool = one(&mut b, boolean, boolean, Value::Cast(copy(1), boolean));
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    for x in floats_f64() {
        let narrow = x as f32;
        let got = run(&mut m, neg32, &[Scalar::F32(narrow)]).unwrap();
        assert!(same_float(got[0], Scalar::F32(-narrow)), "-{narrow}");
        for (from, to, f) in &cast_fns {
            let arg = if *from == f32t { Scalar::F32(narrow) } else { Scalar::F64(x) };
            let want = match (*from == f32t, *to == f32t) {
                (true, true) => Scalar::F32(narrow),
                (true, false) => Scalar::F64(f64::from(narrow)),
                (false, false) => Scalar::F64(x),
                (false, true) => Scalar::F32(x as f32),
            };
            let got = run(&mut m, *f, &[arg]).unwrap();
            assert!(same_float(got[0], want), "{arg:?} as another float: {got:?} against {want:?}");
        }
    }
    for flag in [false, true] {
        for (t, f) in &from_bool {
            assert_eq!(run(&mut m, *f, &[Scalar::Bool(flag)]), Ok(vec![int(*t, flag as i128)]), "{flag} as {t:?}");
        }
        assert_eq!(run(&mut m, bool_bool, &[Scalar::Bool(flag)]), Ok(vec![Scalar::Bool(flag)]));
    }
}

#[test]
fn a_wide_integer_becomes_the_nearest_float_in_one_rounding() {
    // values whose nearest f32 differs from the f32 nearest to their nearest f64: rounding twice gets them wrong
    let mut b = Build::default();
    let (f32t, f64t) = (b.ty(Type::F32), b.ty(Type::F64));
    let mut fns = vec![];
    for t in [Int::U64, Int::I64, Int::Usize, Int::Isize, Int::U32, Int::I32] {
        let ty = b.int(t);
        for fty in [f32t, f64t] {
            let f = b.function(Function {
                name: format!("w{}", b.program.functions.len()),
                params: 1,
                locals: locals(&[fty, ty]),
                blocks: vec![block(vec![assign(place(0), Value::Cast(copy(1), fty), 1)], ret())],
            });
            fns.push((t, fty == f32t, f));
        }
    }
    let mut m = Machine::load(b.program, &[], STACK).unwrap();
    let mut values: Vec<i128> = vec![];
    for k in 25..64 {
        // 2^k + 2^(k-24) + 1: halfway between two f32s, plus one below the f64's precision
        values.push((1i128 << k) + (1i128 << (k - 24)) + 1);
        values.push((1i128 << k) + (1i128 << (k - 24)) - 1);
        values.push((1i128 << k) + (1i128 << (k - 24)));
        values.push(-((1i128 << k) + (1i128 << (k - 24)) + 1));
    }
    let mut checked = 0;
    for (t, narrow, f) in &fns {
        for v in &values {
            if !t.holds(*v) {
                continue;
            }
            let want = match (t.signed(), narrow) {
                (true, true) => Scalar::F32(*v as i64 as f32),
                (false, true) => Scalar::F32(*v as u64 as f32),
                (true, false) => Scalar::F64(*v as i64 as f64),
                (false, false) => Scalar::F64(*v as u64 as f64),
            };
            let got = run(&mut m, *f, &[int(*t, *v)]).unwrap();
            assert!(same_float(got[0], want), "{v} of {t:?}: {got:?} against {want:?}");
            checked += 1;
        }
    }
    assert!(checked > 200, "{checked}");
    assert_eq!((((1u64 << 60) + (1 << 36) + 1) as f32).to_bits(), 0x5d80_0001, "the case rounds up once, down twice");
}
