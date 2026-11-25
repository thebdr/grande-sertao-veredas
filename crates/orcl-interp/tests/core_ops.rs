//! Every operation of the closed list gives what core's own function gives, called from the machine as MIR calls
//! it: by value, or through a reference where core's function takes `&self`.
mod common;
use common::*;
use core::time::Duration;
use orcl_interp::*;

fn caller(b: &mut Build, op: CoreOp, result: TypeId, params: &[TypeId]) -> FunctionId {
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

fn call(m: &mut Machine, f: FunctionId, args: &[Vec<Scalar>]) -> Vec<Scalar> {
    let args: Vec<Arg> = args.iter().map(|a| Arg::Value(a)).collect();
    m.call(f, &args).expect("the operation runs").to_vec()
}

macro_rules! native_int_ops {
    ($t:ty, $op:expr, $a:expr, $b:expr) => {{
        let (a, b) = ($a as $t, $b as $t);
        (match $op {
            CoreOp::SaturatingAdd(_) => a.saturating_add(b),
            CoreOp::SaturatingSub(_) => a.saturating_sub(b),
            CoreOp::SaturatingMul(_) => a.saturating_mul(b),
            CoreOp::WrappingAdd(_) => a.wrapping_add(b),
            CoreOp::WrappingSub(_) => a.wrapping_sub(b),
            CoreOp::WrappingMul(_) => a.wrapping_mul(b),
            CoreOp::Min(_) => Ord::min(a, b),
            CoreOp::Max(_) => Ord::max(a, b),
            _ => unreachable!(),
        }) as i128
    }};
}

fn native(t: Int, op: CoreOp, a: i128, b: i128) -> i128 {
    match t {
        Int::I8 => native_int_ops!(i8, op, a, b),
        Int::I16 => native_int_ops!(i16, op, a, b),
        Int::I32 => native_int_ops!(i32, op, a, b),
        Int::I64 | Int::Isize => native_int_ops!(i64, op, a, b),
        Int::U8 => native_int_ops!(u8, op, a, b),
        Int::U16 => native_int_ops!(u16, op, a, b),
        Int::U32 => native_int_ops!(u32, op, a, b),
        Int::U64 | Int::Usize => native_int_ops!(u64, op, a, b),
    }
}

macro_rules! native_shift {
    ($t:ty, $shl:expr, $a:expr, $n:expr) => {{
        let a = $a as $t;
        (if $shl { a.checked_shl($n) } else { a.checked_shr($n) }).map(|v| v as i128)
    }};
}

fn native_checked_shift(t: Int, shl: bool, a: i128, n: u32) -> Option<i128> {
    match t {
        Int::I8 => native_shift!(i8, shl, a, n),
        Int::I16 => native_shift!(i16, shl, a, n),
        Int::I32 => native_shift!(i32, shl, a, n),
        Int::I64 | Int::Isize => native_shift!(i64, shl, a, n),
        Int::U8 => native_shift!(u8, shl, a, n),
        Int::U16 => native_shift!(u16, shl, a, n),
        Int::U32 => native_shift!(u32, shl, a, n),
        Int::U64 | Int::Usize => native_shift!(u64, shl, a, n),
    }
}

#[test]
fn the_integer_operations_give_what_core_gives_on_every_integer_type() {
    let mut b = Build::default();
    let u32t = b.int(Int::U32);
    let mut fns = vec![];
    for t in INTS {
        let ty = b.int(t);
        let opt = b.option(ty);
        for op in [
            CoreOp::SaturatingAdd(t),
            CoreOp::SaturatingSub(t),
            CoreOp::SaturatingMul(t),
            CoreOp::WrappingAdd(t),
            CoreOp::WrappingSub(t),
            CoreOp::WrappingMul(t),
            CoreOp::Min(t),
            CoreOp::Max(t),
        ] {
            fns.push((t, op, caller(&mut b, op, ty, &[ty, ty])));
        }
        for op in [CoreOp::CheckedShl(t), CoreOp::CheckedShr(t)] {
            fns.push((t, op, caller(&mut b, op, opt, &[ty, u32t])));
        }
    }
    let mut m = Machine::load(b.program, &[]).unwrap();
    let mut rng = Rng(0xC0DE_0000_0000_0001);
    for (t, op, f) in fns {
        let values = samples(t, &mut rng, 10);
        match op {
            CoreOp::CheckedShl(_) | CoreOp::CheckedShr(_) => {
                let mut amounts: Vec<u32> = (0..=t.bits() + 3).collect();
                amounts.extend([100, u32::MAX / 2, u32::MAX]);
                for a in &values {
                    for n in &amounts {
                        let got = call(&mut m, f, &[vec![int(t, *a)], vec![int(Int::U32, *n as i128)]]);
                        let want = match native_checked_shift(t, matches!(op, CoreOp::CheckedShl(_)), *a, *n) {
                            Some(v) => vec![int(Int::Isize, 1), int(t, v)],
                            None => vec![int(Int::Isize, 0), Scalar::Uninit],
                        };
                        assert_eq!(got, want, "{op:?}({a}, {n})");
                    }
                }
            }
            _ => {
                for a in &values {
                    for c in &values {
                        let got = call(&mut m, f, &[vec![int(t, *a)], vec![int(t, *c)]]);
                        assert_eq!(got, vec![int(t, native(t, op, *a, *c))], "{op:?}({a}, {c})");
                    }
                }
            }
        }
    }
}

fn dur(d: Duration) -> Vec<Scalar> {
    vec![int(Int::U64, d.as_secs() as i128), int(Int::U32, d.subsec_nanos() as i128)]
}

#[test]
fn the_duration_operations_give_what_core_gives() {
    let mut b = Build::default();
    let (u64t, d, boolean) = (b.int(Int::U64), b.ty(Type::Duration), b.bool());
    let sd = b.reference(d, false);
    let millis = caller(&mut b, CoreOp::DurationFromMillis, d, &[u64t]);
    let secs = caller(&mut b, CoreOp::DurationFromSecs, d, &[u64t]);
    let sub = caller(&mut b, CoreOp::DurationSaturatingSub, d, &[d, d]);
    let min = caller(&mut b, CoreOp::DurationMin, d, &[d, d]);
    let max = caller(&mut b, CoreOp::DurationMax, d, &[d, d]);
    // a comparison takes references: fn cmp(a: Duration, b: Duration) -> bool { let (x, y) = (&a, &b); op(x, y) }
    let cmps: Vec<(BinOp, FunctionId)> = [BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge]
        .into_iter()
        .map(|op| {
            let f = b.function(Function {
                name: format!("{op:?}"),
                params: 2,
                locals: locals(&[boolean, d, d, sd, sd]),
                blocks: vec![
                    block(
                        vec![assign(place(3), Value::Ref(place(1), false), 1), assign(place(4), Value::Ref(place(2), false), 1)],
                        Terminator::Call {
                            callee: Callee::Core(CoreOp::DurationCompare(op)),
                            args: vec![copy(3), copy(4)],
                            dest: place(0),
                            next: BlockId(1),
                            at: at(1),
                        },
                    ),
                    block(vec![], ret()),
                ],
            });
            (op, f)
        })
        .collect();
    let mut m = Machine::load(b.program, &[]).unwrap();
    let mut rng = Rng(0xC0DE_0000_0000_0002);
    let mut counts: Vec<u64> = vec![0, 1, 999, 1000, 1001, 60_000, u64::MAX / 1000, u64::MAX - 1, u64::MAX];
    counts.extend((0..20).map(|_| rng.next()));
    for v in &counts {
        let arg = vec![int(Int::U64, *v as i128)];
        assert_eq!(call(&mut m, millis, std::slice::from_ref(&arg)), dur(Duration::from_millis(*v)), "from_millis({v})");
        assert_eq!(call(&mut m, secs, std::slice::from_ref(&arg)), dur(Duration::from_secs(*v)), "from_secs({v})");
    }
    let mut durations = vec![
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_nanos(999_999_999),
        Duration::from_secs(1),
        Duration::new(1, 1),
        Duration::from_millis(100),
        Duration::MAX,
        Duration::new(u64::MAX, 0),
    ];
    durations.extend((0..12).map(|_| Duration::new(rng.below(5), rng.below(1_000_000_000) as u32)));
    for x in &durations {
        for y in &durations {
            let args = [dur(*x), dur(*y)];
            assert_eq!(call(&mut m, sub, &args), dur(x.saturating_sub(*y)), "{x:?} - {y:?}");
            assert_eq!(call(&mut m, min, &args), dur(Ord::min(*x, *y)));
            assert_eq!(call(&mut m, max, &args), dur(Ord::max(*x, *y)));
            for (op, f) in &cmps {
                let want = match op {
                    BinOp::Eq => x == y,
                    BinOp::Ne => x != y,
                    BinOp::Lt => x < y,
                    BinOp::Le => x <= y,
                    BinOp::Gt => x > y,
                    _ => x >= y,
                };
                assert_eq!(call(&mut m, *f, &args), vec![Scalar::Bool(want)], "{x:?} {op:?} {y:?}");
            }
        }
    }
}

#[test]
fn an_option_is_filled_once_read_through_its_reference_and_asked_whether_it_holds_a_value() {
    let mut b = Build::default();
    let (d, boolean) = (b.ty(Type::Duration), b.bool());
    let opt = b.option(d);
    let (ropt, sopt, rd) = (b.reference(opt, true), b.reference(opt, false), b.reference(d, true));
    // fn f(o: &mut Option<Duration>, v: Duration) -> (Duration, bool, bool):
    //   let r = o.get_or_insert(v); (*r, (&*o).is_none(), (&*o).is_some())
    let out = b.tuple(&[d, boolean, boolean]);
    let f = b.function(Function {
        name: "f".into(),
        params: 2,
        locals: locals(&[out, ropt, d, rd, sopt, boolean, boolean]),
        blocks: vec![
            block(
                vec![],
                Terminator::Call {
                    callee: Callee::Core(CoreOp::OptionGetOrInsert),
                    args: vec![copy(1), copy(2)],
                    dest: place(3),
                    next: BlockId(1),
                    at: at(1),
                },
            ),
            block(
                vec![assign(place(4), Value::Ref(proj(1, vec![Projection::Deref]), false), 2)],
                Terminator::Call { callee: Callee::Core(CoreOp::OptionIsNone), args: vec![copy(4)], dest: place(5), next: BlockId(2), at: at(2) },
            ),
            block(
                vec![],
                Terminator::Call { callee: Callee::Core(CoreOp::OptionIsSome), args: vec![copy(4)], dest: place(6), next: BlockId(3), at: at(3) },
            ),
            block(
                vec![assign(place(0), Value::Aggregate(out, None, vec![copy_at(proj(3, vec![Projection::Deref])), copy(5), copy(6)]), 4)],
                ret(),
            ),
        ],
    });
    let is_none = b.function(Function {
        name: "is_none".into(),
        params: 1,
        locals: locals(&[boolean, sopt]),
        blocks: vec![
            block(vec![], Terminator::Call { callee: Callee::Core(CoreOp::OptionIsNone), args: vec![copy(1)], dest: place(0), next: BlockId(1), at: at(5) }),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    let region = RegionId(0);
    m.set_region(region, &[int(Int::Isize, 0), Scalar::Uninit, Scalar::Uninit]).unwrap();
    assert_eq!(m.call(is_none, &[Arg::Ref(region)]).unwrap(), &[Scalar::Bool(true)]);
    let mut native: Option<Duration> = None;
    for v in [Duration::from_millis(7), Duration::from_millis(9)] {
        let got = m.call(f, &[Arg::Ref(region), Arg::Value(&dur(v))]).unwrap().to_vec();
        let r = *native.get_or_insert(v);
        let mut want = dur(r);
        want.extend([Scalar::Bool(native.is_none()), Scalar::Bool(native.is_some())]);
        assert_eq!(got, want, "get_or_insert({v:?})");
        let mut slot = vec![int(Int::Isize, 1)];
        slot.extend(dur(Duration::from_millis(7)));
        assert_eq!(m.region(region).unwrap(), slot.as_slice(), "filled once, by the first value");
    }
    assert_eq!(m.call(is_none, &[Arg::Ref(region)]).unwrap(), &[Scalar::Bool(false)]);
}

#[test]
fn a_default_is_false_zero_or_no_time() {
    let mut b = Build::default();
    let mut fns = vec![];
    let boolean = b.bool();
    fns.push((caller(&mut b, CoreOp::Default, boolean, &[]), vec![Scalar::Bool(false)]));
    for t in INTS {
        let ty = b.int(t);
        fns.push((caller(&mut b, CoreOp::Default, ty, &[]), vec![int(t, 0)]));
    }
    let (f32t, f64t, d) = (b.ty(Type::F32), b.ty(Type::F64), b.ty(Type::Duration));
    fns.push((caller(&mut b, CoreOp::Default, f32t, &[]), vec![Scalar::F32(f32::default())]));
    fns.push((caller(&mut b, CoreOp::Default, f64t, &[]), vec![Scalar::F64(f64::default())]));
    fns.push((caller(&mut b, CoreOp::Default, d, &[]), dur(Duration::default())));
    let mut m = Machine::load(b.program, &[]).unwrap();
    for (f, want) in fns {
        let got = call(&mut m, f, &[]);
        // compared slot by slot and, for a float, bit for bit: `-0.0 == 0.0` would let a negative zero pass
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(&want) {
            match (g, w) {
                (Scalar::F32(x), Scalar::F32(y)) => assert_eq!(x.to_bits(), y.to_bits(), "f32's default"),
                (Scalar::F64(x), Scalar::F64(y)) => assert_eq!(x.to_bits(), y.to_bits(), "f64's default"),
                _ => assert_eq!(g, w),
            }
        }
    }
}

#[test]
fn a_checked_shift_s_none_after_a_some_into_the_same_place_leaves_no_payload() {
    // fn f(a: u8) -> Option<u8> { _0 = a.checked_shl(1); _0 = a.checked_shl(9); _0 }, and the same with shr
    let mut b = Build::default();
    let (u8t, u32t) = (b.int(Int::U8), b.int(Int::U32));
    let opt = b.option(u8t);
    let mut fns = vec![];
    for op in [CoreOp::CheckedShl(Int::U8), CoreOp::CheckedShr(Int::U8)] {
        let sh = |n, next| Terminator::Call {
            callee: Callee::Core(op),
            args: vec![copy(1), konst(u32t, vec![int(Int::U32, n)])],
            dest: place(0),
            next: BlockId(next),
            at: at(1),
        };
        fns.push(b.function(Function {
            name: format!("{op:?}"),
            params: 1,
            locals: locals(&[opt, u8t]),
            blocks: vec![block(vec![], sh(1, 1)), block(vec![], sh(9, 2)), block(vec![], ret())],
        }));
    }
    let mut m = Machine::load(b.program, &[]).unwrap();
    for f in fns {
        assert_eq!(call(&mut m, f, &[vec![int(Int::U8, 3)]]), vec![int(Int::Isize, 0), Scalar::Uninit]);
    }
}

#[test]
fn get_or_insert_writes_the_option_s_own_discriminant_type() {
    // an Option whose discriminant the program gives as u8
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let opt = b.ty(Type::Enum {
        name: "Option".into(),
        discriminant: Int::U8,
        variants: vec![
            Variant { name: "None".into(), discriminant: 0, fields: vec![] },
            Variant { name: "Some".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: u8t }] },
        ],
    });
    let (ropt, ru8) = (b.reference(opt, true), b.reference(u8t, true));
    let f = b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[ru8, ropt]),
        blocks: vec![
            block(
                vec![],
                Terminator::Call {
                    callee: Callee::Core(CoreOp::OptionGetOrInsert),
                    args: vec![copy(1), konst(u8t, vec![int(Int::U8, 4)])],
                    dest: place(0),
                    next: BlockId(1),
                    at: at(1),
                },
            ),
            block(vec![], ret()),
        ],
    });
    let mut m = Machine::load(b.program, &[opt]).unwrap();
    m.set_region(RegionId(0), &[int(Int::U8, 0), Scalar::Uninit]).unwrap();
    m.call(f, &[Arg::Ref(RegionId(0))]).unwrap();
    assert_eq!(m.region(RegionId(0)).unwrap(), &[int(Int::U8, 1), int(Int::U8, 4)]);
}

/// `x.min(K)`, `x.max(K)`, `x.saturating_sub(K)` and `K.saturating_sub(x)` with `K` a constant `Duration` operand,
/// as MIR passes a `const` item or `Duration::ZERO`: against core's.
#[test]
fn a_duration_constant_passed_to_a_core_operation_gives_what_core_gives() {
    let mut b = Build::default();
    let d = b.ty(Type::Duration);
    let k = Duration::new(1, 500_000_000);
    let kc = konst(d, dur(k));
    let mut fns = vec![];
    for (op, const_first) in [
        (CoreOp::DurationMin, false),
        (CoreOp::DurationMax, false),
        (CoreOp::DurationSaturatingSub, false),
        (CoreOp::DurationSaturatingSub, true),
    ] {
        let args = if const_first { vec![kc.clone(), copy(1)] } else { vec![copy(1), kc.clone()] };
        let f = b.function(Function {
            name: format!("{op:?}{const_first}"),
            params: 1,
            locals: locals(&[d, d]),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Core(op), args, dest: place(0), next: BlockId(1), at: at(1) }),
                block(vec![], ret()),
            ],
        });
        fns.push((op, const_first, f));
    }
    let mut m = Machine::load(b.program, &[]).unwrap();
    for x in [Duration::ZERO, Duration::from_millis(700), k, Duration::new(1, 500_000_001), Duration::new(9, 1)] {
        for (op, const_first, f) in &fns {
            let want = match (op, const_first) {
                (CoreOp::DurationMin, _) => x.min(k),
                (CoreOp::DurationMax, _) => x.max(k),
                (_, false) => x.saturating_sub(k),
                _ => k.saturating_sub(x),
            };
            let got = m.call(*f, &[Arg::Value(&dur(x))]).map(|r| r.to_vec());
            assert_eq!(got, Ok(dur(want)), "{op:?} const_first={const_first} x={x:?}");
        }
    }
}

#[test]
fn a_checked_shift_into_an_option_whose_discriminant_is_a_u8_writes_a_u8() {
    let mut b = Build::default();
    let (u8t, u32t) = (b.int(Int::U8), b.int(Int::U32));
    let opt = b.ty(Type::Enum {
        name: "Option".into(),
        discriminant: Int::U8,
        variants: vec![
            Variant { name: "None".into(), discriminant: 0, fields: vec![] },
            Variant { name: "Some".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: u8t }] },
        ],
    });
    let f = caller(&mut b, CoreOp::CheckedShl(Int::U8), opt, &[u8t, u32t]);
    let mut m = Machine::load(b.program, &[]).unwrap();
    assert_eq!(call(&mut m, f, &[vec![int(Int::U8, 3)], vec![int(Int::U32, 2)]]), vec![int(Int::U8, 1), int(Int::U8, 12)]);
    assert_eq!(call(&mut m, f, &[vec![int(Int::U8, 3)], vec![int(Int::U32, 8)]]), vec![int(Int::U8, 0), Scalar::Uninit]);
}
