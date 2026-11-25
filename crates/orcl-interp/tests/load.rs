//! A program the machine cannot lay out or whose instructions do not fit their types is refused when it is loaded,
//! by name and place, before anything runs; so is one whose functions reach themselves (`docs/questions.md` Q230,
//! taken meanwhile).
mod common;
use common::*;
use orcl_interp::*;

fn leaf(name: &str, unit: TypeId, terminator: Terminator) -> Function {
    Function { name: name.into(), params: 0, locals: locals(&[unit]), blocks: vec![block(vec![], terminator)] }
}

fn call(f: u32, next: u32) -> Terminator {
    Terminator::Call { callee: Callee::Function(FunctionId(f)), args: vec![], dest: place(0), next: BlockId(next), at: at(1) }
}

fn refused(program: Program) -> LoadError {
    Machine::load(program, &[]).expect_err("refused")
}

fn instruction(function: &str, block: u32, statement: Option<u32>, reason: &'static str) -> LoadError {
    LoadError::Instruction { function: function.into(), block, statement, reason }
}

#[test]
fn a_function_that_reaches_itself_is_refused_naming_its_cycle() {
    let mut b = Build::default();
    let unit = b.unit();
    b.function(leaf("f", unit, call(0, 0)));
    assert_eq!(refused(b.program), LoadError::Recursion { cycle: vec!["f".into(), "f".into()] });

    let mut b = Build::default();
    let unit = b.unit();
    b.function(leaf("main", unit, call(1, 0)));
    b.function(leaf("a", unit, call(2, 0)));
    b.function(leaf("b", unit, call(1, 0)));
    assert_eq!(refused(b.program), LoadError::Recursion { cycle: vec!["a".into(), "b".into(), "a".into()] });

    // a diamond is no cycle: main calls a and b, both call c
    let mut b = Build::default();
    let unit = b.unit();
    b.function(Function {
        name: "main".into(),
        params: 0,
        locals: locals(&[unit]),
        blocks: vec![block(vec![], call(1, 1)), block(vec![], call(2, 2)), block(vec![], ret())],
    });
    b.function(leaf("a", unit, call(3, 0)));
    b.function(Function { name: "b".into(), params: 0, locals: locals(&[unit]), blocks: vec![block(vec![], call(3, 1)), block(vec![], ret())] });
    b.function(leaf("c", unit, ret()));
    // a's only block calls c and comes back to itself: a loop, not a cycle of calls through a
    assert!(Machine::load(b.program, &[]).is_ok());
}

#[test]
fn a_type_the_machine_cannot_lay_out_is_refused_naming_it() {
    let mut b = Build::default();
    b.program.types.push(Type::Struct { name: "S".into(), fields: vec![Field { name: "s".into(), ty: TypeId(0) }] });
    assert_eq!(refused(b.program), LoadError::Type { ty: 0, reason: "it contains itself with no reference between" });

    // through a reference it is one slot
    let mut b = Build::default();
    b.program.types.push(Type::Ref { to: TypeId(1), mutable: false });
    b.program.types.push(Type::Struct { name: "S".into(), fields: vec![Field { name: "s".into(), ty: TypeId(0) }] });
    assert!(Machine::load(b.program, &[]).is_ok());

    let mut b = Build::default();
    b.program.types.push(Type::Array { element: TypeId(7), len: 2 });
    assert_eq!(refused(b.program), LoadError::Type { ty: 7, reason: "no such type" });

    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::U8,
        variants: vec![
            Variant { name: "A".into(), discriminant: 1, fields: vec![] },
            Variant { name: "B".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: u8t }] },
        ],
    });
    assert_eq!(refused(b.program), LoadError::Type { ty: 1, reason: "two variants with one discriminant" });

    let mut b = Build::default();
    b.ty(Type::Enum {
        name: "E".into(),
        discriminant: Int::U8,
        variants: vec![Variant { name: "A".into(), discriminant: 256, fields: vec![] }],
    });
    assert_eq!(refused(b.program), LoadError::Type { ty: 0, reason: "a discriminant outside its type" });

    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    b.ty(Type::Array { element: u8t, len: u32::MAX });
    let big = b.ty(Type::Array { element: TypeId(1), len: 2 });
    let _ = big;
    assert_eq!(refused(b.program), LoadError::Type { ty: 2, reason: "too large" });
}

#[test]
fn an_instruction_whose_indices_or_types_do_not_fit_is_refused_naming_the_function_block_and_statement() {
    let base = || {
        let mut b = Build::default();
        let (u8t, u16t, boolean, usize_t, unit) = (b.int(Int::U8), b.int(Int::U16), b.bool(), b.int(Int::Usize), b.unit());
        (b, u8t, u16t, boolean, usize_t, unit)
    };
    let one = |f: Function| {
        let (mut b, ..) = base();
        b.function(f);
        refused(b.program)
    };
    let (_, u8t, u16t, boolean, usize_t, unit) = base();
    let f = |locals_: &[TypeId], statements: Vec<Statement>, terminator: Terminator| Function {
        name: "f".into(),
        params: 0,
        locals: locals(locals_),
        blocks: vec![block(statements, terminator)],
    };
    assert_eq!(
        one(f(&[u8t], vec![assign(place(0), Value::Use(konst(u16t, vec![int(Int::U16, 1)])), 1)], ret())),
        instruction("f", 0, Some(0), "a value of another type than its place")
    );
    assert_eq!(
        one(f(&[u8t], vec![assign(place(0), Value::Use(konst(u8t, vec![Scalar::Int(Int::U8, 300)])), 1)], ret())),
        instruction("f", 0, Some(0), "a constant whose slots are not its type's")
    );
    assert_eq!(
        one(f(&[u8t], vec![assign(proj(0, vec![Projection::Field(0)]), Value::Use(copy(0)), 1)], ret())),
        instruction("f", 0, Some(0), "a field of what is no structure")
    );
    assert_eq!(
        one(f(&[u8t], vec![assign(place(4), Value::Use(copy(0)), 1)], ret())),
        instruction("f", 0, Some(0), "no such local")
    );
    let (mut b, ..) = base();
    let arr = b.ty(Type::Array { element: u8t, len: 4 });
    b.function(f(&[u8t, arr, u8t], vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::Index(Local(2))]))), 1)], ret()));
    assert_eq!(refused(b.program), instruction("f", 0, Some(0), "an index that is no usize"));
    let (mut b, ..) = base();
    let arr = b.ty(Type::Array { element: u8t, len: 4 });
    b.function(f(&[u8t, arr, usize_t], vec![assign(place(0), Value::Use(copy_at(proj(1, vec![Projection::ConstantIndex(4)]))), 1)], ret()));
    assert_eq!(refused(b.program), instruction("f", 0, Some(0), "a constant index past the array"));
    assert_eq!(
        one(f(&[u8t], vec![assign(place(0), Value::Binary(BinOp::Lt, copy(0), copy(0)), 1)], ret())),
        instruction("f", 0, Some(0), "a comparison into what is no bool")
    );
    assert_eq!(
        one(f(&[boolean], vec![assign(place(0), Value::Unary(UnOp::Neg, copy(0)), 1)], ret())),
        instruction("f", 0, Some(0), "a unary operation on what it does not take")
    );
    assert_eq!(
        one(f(&[boolean], vec![], Terminator::SwitchInt { value: copy(0), targets: vec![(2, BlockId(0))], otherwise: BlockId(0), at: at(1) })),
        instruction("f", 0, None, "a switch's value outside its operand's type")
    );
    assert_eq!(one(f(&[unit], vec![], Terminator::Goto(BlockId(1)))), instruction("f", 0, None, "no such block"));
    assert_eq!(
        one(f(
            &[u8t, u16t],
            vec![],
            Terminator::Call {
                callee: Callee::Core(CoreOp::SaturatingAdd(Int::U8)),
                args: vec![copy(1), copy(1)],
                dest: place(0),
                next: BlockId(0),
                at: at(1),
            }
        )),
        instruction("f", 0, None, "a core operation called with types it does not take")
    );
    assert_eq!(
        one(f(
            &[unit],
            vec![],
            Terminator::Call { callee: Callee::Function(FunctionId(0)), args: vec![copy(0)], dest: place(0), next: BlockId(0), at: at(1) }
        )),
        instruction("f", 0, None, "a call with too many or too few arguments")
    );
    assert_eq!(
        one(Function { name: "f".into(), params: 0, locals: locals(&[unit]), blocks: vec![] }),
        LoadError::Function { function: "f".into(), reason: "no block" }
    );
    assert_eq!(
        one(Function { name: "f".into(), params: 1, locals: locals(&[unit]), blocks: vec![block(vec![], ret())] }),
        LoadError::Function { function: "f".into(), reason: "no return value, or more parameters than locals" }
    );
    assert_eq!(
        one(Function { name: "f".into(), params: 0, locals: locals(&[TypeId(99)]), blocks: vec![block(vec![], ret())] }),
        LoadError::Function { function: "f".into(), reason: "a local of no such type" }
    );
    let (b, ..) = base();
    assert_eq!(
        Machine::load(b.program, &[TypeId(40)]).unwrap_err(),
        LoadError::Region { index: 0, reason: "a region of no such type" }
    );
}

/// One function `f` over `types` (its locals) with one block, in a program that also holds `g(u8) -> u8`.
struct One {
    b: Build,
}

impl One {
    fn new() -> One {
        let mut b = Build::default();
        let u8t = b.int(Int::U8);
        b.function(Function {
            name: "g".into(),
            params: 1,
            locals: locals(&[u8t, u8t]),
            blocks: vec![block(vec![assign(place(0), Value::Use(copy(1)), 1)], ret())],
        });
        One { b }
    }

    fn ty(&mut self, t: Type) -> TypeId {
        self.b.ty(t)
    }

    fn refused(mut self, locals_: &[TypeId], statements: Vec<Statement>, terminator: Terminator) -> LoadError {
        self.b.function(Function { name: "f".into(), params: 0, locals: locals(locals_), blocks: vec![block(statements, terminator)] });
        refused(self.b.program)
    }

    fn statement(self, locals_: &[TypeId], place_: Place, value: Value) -> LoadError {
        self.refused(locals_, vec![assign(place_, value, 1)], ret())
    }
}

fn at_statement(reason: &'static str) -> LoadError {
    instruction("f", 0, Some(0), reason)
}

fn at_terminator(reason: &'static str) -> LoadError {
    instruction("f", 0, None, reason)
}

#[test]
fn every_place_that_does_not_fit_its_types_is_refused_by_its_reason() {
    let opt_of = |o: &mut One, t| {
        o.ty(Type::Enum {
            name: "Option".into(),
            discriminant: Int::Isize,
            variants: vec![
                Variant { name: "None".into(), discriminant: 0, fields: vec![] },
                Variant { name: "Some".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: t }] },
            ],
        })
    };
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let s = o.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] });
    assert_eq!(o.statement(&[u8t, s], place(0), Value::Use(copy_at(proj(1, vec![Projection::Field(5)])))), at_statement("no such field"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let opt = opt_of(&mut o, u8t);
    let p = proj(1, vec![Projection::Downcast(1), Projection::Field(9)]);
    assert_eq!(o.statement(&[u8t, opt], place(0), Value::Use(copy_at(p))), at_statement("no such field of the variant"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let opt = opt_of(&mut o, u8t);
    let p = proj(1, vec![Projection::Downcast(9), Projection::Field(0)]);
    assert_eq!(o.statement(&[u8t, opt], place(0), Value::Use(copy_at(p))), at_statement("no such variant"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    assert_eq!(o.statement(&[u8t, u8t], place(0), Value::Use(copy_at(proj(1, vec![Projection::Deref])))), at_statement("a dereference of what is no reference"));
    let mut o = One::new();
    let (u8t, usize_t) = (o.ty(Type::Int(Int::U8)), o.ty(Type::Int(Int::Usize)));
    let p = proj(1, vec![Projection::Index(Local(2))]);
    assert_eq!(o.statement(&[u8t, u8t, usize_t], place(0), Value::Use(copy_at(p))), at_statement("an index into what is no array"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let opt = opt_of(&mut o, u8t);
    let p = proj(1, vec![Projection::Downcast(1), Projection::Deref]);
    assert_eq!(o.statement(&[u8t, opt], place(0), Value::Use(copy_at(p))), at_statement("a variant read but through a field"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let opt = opt_of(&mut o, u8t);
    let p = proj(1, vec![Projection::Downcast(1)]);
    assert_eq!(o.statement(&[u8t, opt], place(0), Value::Use(copy_at(p))), at_statement("a place that ends in a variant"));
}

#[test]
fn every_value_that_does_not_fit_its_types_is_refused_by_its_reason() {
    fn ints(o: &mut One) -> (TypeId, TypeId, TypeId, TypeId) {
        (o.ty(Type::Int(Int::U8)), o.ty(Type::Int(Int::U16)), o.ty(Type::Bool), o.ty(Type::F32))
    }
    type Case = Box<dyn Fn(&mut One) -> (Vec<TypeId>, Value)>;
    let cases: Vec<(&str, Case)> = vec![
        ("a shift of what is no integer", Box::new(|o| { let (_, _, b, _) = ints(o); (vec![b, b], Value::Binary(BinOp::Shl, copy(1), copy(1))) })),
        ("a comparison of values that do not compare", Box::new(|o| { let (u8t, u16t, b, _) = ints(o); (vec![b, u8t, u16t], Value::Binary(BinOp::Eq, copy(1), copy(2))) })),
        ("a comparison of values that do not compare", Box::new(|o| { let (u8t, _, b, _) = ints(o); let s = o.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }); (vec![b, s], Value::Binary(BinOp::Lt, copy(1), copy(1))) })),
        ("bits of what has none", Box::new(|o| { let (_, _, _, f) = ints(o); (vec![f, f], Value::Binary(BinOp::BitAnd, copy(1), copy(1))) })),
        ("arithmetic on what is no number", Box::new(|o| { let (_, _, b, _) = ints(o); (vec![b, b], Value::Binary(BinOp::Add, copy(1), copy(1))) })),
        ("a result of another type than its place", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); (vec![u16t, u8t], Value::Binary(BinOp::Add, copy(1), copy(1))) })),
        ("a result of another type than its place", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); (vec![u16t, u8t, u16t], Value::Binary(BinOp::Shl, copy(1), copy(2))) })),
        ("a checked operation that MIR has no checked form of", Box::new(|o| { let (u8t, _, b, _) = ints(o); let pair = o.ty(Type::Struct { name: "(..)".into(), fields: vec![Field { name: "0".into(), ty: u8t }, Field { name: "1".into(), ty: b }] }); (vec![pair, u8t], Value::Checked(BinOp::Div, copy(1), copy(1))) })),
        ("a checked result into what is no (value, bool)", Box::new(|o| { let (u8t, _, _, _) = ints(o); (vec![u8t, u8t], Value::Checked(BinOp::Add, copy(1), copy(1))) })),
        ("a checked result into what is no (value, bool)", Box::new(|o| { let (u8t, u16t, b, _) = ints(o); let pair = o.ty(Type::Struct { name: "(..)".into(), fields: vec![Field { name: "0".into(), ty: u16t }, Field { name: "1".into(), ty: b }] }); (vec![pair, u8t], Value::Checked(BinOp::Mul, copy(1), copy(1))) })),
        ("a unary operation on what it does not take", Box::new(|o| { let (u8t, _, _, _) = ints(o); (vec![u8t, u8t], Value::Unary(UnOp::Neg, copy(1))) })),
        ("a unary operation on what it does not take", Box::new(|o| { let (_, _, _, f) = ints(o); (vec![f, f], Value::Unary(UnOp::Not, copy(1))) })),
        ("a unary operation on what it does not take", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); (vec![u16t, u8t], Value::Unary(UnOp::Not, copy(1))) })),
        ("a cast `as` does not make", Box::new(|o| { let (u8t, _, b, _) = ints(o); (vec![b, u8t], Value::Cast(copy(1), b)) })),
        ("a cast `as` does not make", Box::new(|o| { let (u8t, _, _, _) = ints(o); let d = o.ty(Type::Duration); (vec![d, u8t], Value::Cast(copy(1), d)) })),
        ("a cast `as` does not make", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); (vec![u8t, u8t], Value::Cast(copy(1), u16t)) })),
        ("an aggregate of what is built otherwise", Box::new(|o| { let (u8t, _, _, _) = ints(o); (vec![u8t], Value::Aggregate(u8t, None, vec![])) })),
        ("no such variant", Box::new(|o| { let (u8t, _, _, _) = ints(o); let e = o.ty(Type::Enum { name: "E".into(), discriminant: Int::U8, variants: vec![Variant { name: "A".into(), discriminant: 0, fields: vec![] }] }); let _ = u8t; (vec![e], Value::Aggregate(e, Some(3), vec![])) })),
        ("an aggregate with too many or too few parts", Box::new(|o| { let (u8t, _, _, _) = ints(o); let s = o.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }); (vec![s, u8t], Value::Aggregate(s, None, vec![copy(1), copy(1)])) })),
        ("a part of another type than its place", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); let s = o.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }); (vec![s, u16t], Value::Aggregate(s, None, vec![copy(1)])) })),
        ("an aggregate of another type than its place", Box::new(|o| { let (u8t, _, _, _) = ints(o); let s = o.ty(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }); let t = o.ty(Type::Struct { name: "T".into(), fields: vec![Field { name: "a".into(), ty: u8t }] }); (vec![t, u8t], Value::Aggregate(s, None, vec![copy(1)])) })),
        ("the discriminant of what is no enum", Box::new(|o| { let (u8t, _, _, _) = ints(o); (vec![u8t, u8t], Value::Discriminant(place(1))) })),
        ("a discriminant into another type", Box::new(|o| { let (u8t, _, _, _) = ints(o); let e = o.ty(Type::Enum { name: "E".into(), discriminant: Int::Isize, variants: vec![Variant { name: "A".into(), discriminant: 0, fields: vec![] }] }); (vec![u8t, e], Value::Discriminant(place(1))) })),
        ("a reference into another type", Box::new(|o| { let (u8t, u16t, _, _) = ints(o); let r = o.ty(Type::Ref { to: u16t, mutable: true }); (vec![r, u8t], Value::Ref(place(1), true)) })),
        ("a reference into another type", Box::new(|o| { let (u8t, _, _, _) = ints(o); let r = o.ty(Type::Ref { to: u8t, mutable: false }); (vec![r, u8t], Value::Ref(place(1), true)) })),
    ];
    for (reason, case) in cases {
        let mut o = One::new();
        let (types, value) = case(&mut o);
        assert_eq!(o.statement(&types, place(0), value), at_statement(reason), "{reason}");
    }
}

#[test]
fn every_terminator_that_does_not_fit_is_refused_by_its_reason() {
    let call = |callee, args, next| Terminator::Call { callee, args, dest: place(0), next: BlockId(next), at: at(1) };
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    assert_eq!(o.refused(&[u8t], vec![], Terminator::Assert { cond: copy(0), expected: true, check: Check::Bounds, next: BlockId(0), at: at(1) }), at_terminator("an assertion of what is no bool"));
    let mut o = One::new();
    let b = o.ty(Type::Bool);
    assert_eq!(o.refused(&[b], vec![], Terminator::Assert { cond: copy(0), expected: true, check: Check::Bounds, next: BlockId(9), at: at(1) }), at_terminator("no such block"));
    let mut o = One::new();
    let b = o.ty(Type::Bool);
    assert_eq!(o.refused(&[b], vec![], Terminator::SwitchInt { value: copy(0), targets: vec![(1, BlockId(9))], otherwise: BlockId(0), at: at(1) }), at_terminator("no such block"));
    let mut o = One::new();
    let b = o.ty(Type::Bool);
    assert_eq!(o.refused(&[b], vec![], Terminator::SwitchInt { value: copy(0), targets: vec![], otherwise: BlockId(9), at: at(1) }), at_terminator("no such block"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    let one = konst(u8t, vec![int(Int::U8, 1)]);
    assert_eq!(o.refused(&[u8t], vec![], call(Callee::Function(FunctionId(0)), vec![one], 9)), at_terminator("no such block"), "a call's next");
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    assert_eq!(o.refused(&[u8t], vec![], call(Callee::Function(FunctionId(9)), vec![], 0)), at_terminator("no such function"));
    let mut o = One::new();
    let u8t = o.ty(Type::Int(Int::U8));
    assert_eq!(o.refused(&[u8t], vec![], call(Callee::Function(FunctionId(0)), vec![], 0)), at_terminator("a call with too many or too few arguments"), "too few");
    let mut o = One::new();
    let (u8t, u16t) = (o.ty(Type::Int(Int::U8)), o.ty(Type::Int(Int::U16)));
    let two = konst(u16t, vec![int(Int::U16, 2)]);
    assert_eq!(o.refused(&[u8t], vec![], call(Callee::Function(FunctionId(0)), vec![two], 0)), at_terminator("an argument of another type than its parameter"));
    let mut o = One::new();
    let (u8t, u16t) = (o.ty(Type::Int(Int::U8)), o.ty(Type::Int(Int::U16)));
    let one = konst(u8t, vec![int(Int::U8, 1)]);
    assert_eq!(o.refused(&[u16t], vec![], call(Callee::Function(FunctionId(0)), vec![one], 0)), at_terminator("a call whose result is of another type than its place"));
}

#[test]
fn a_core_operation_called_with_any_type_it_does_not_take_is_refused() {
    // each operation called right, then with each argument and the destination in turn of another type
    let mut b = Build::default();
    let (u8t, u16t, u32t, u64t, boolean, d) =
        (b.int(Int::U8), b.int(Int::U16), b.int(Int::U32), b.int(Int::U64), b.bool(), b.ty(Type::Duration));
    let (optu8, optd) = (b.option(u8t), b.option(d));
    let (sd, ropt, sopt, rd, rd_shared) =
        (b.reference(d, false), b.reference(optd, true), b.reference(optd, false), b.reference(d, true), b.reference(u8t, false));
    let right: Vec<(CoreOp, Vec<TypeId>, TypeId)> = vec![
        (CoreOp::SaturatingAdd(Int::U8), vec![u8t, u8t], u8t),
        (CoreOp::WrappingMul(Int::U8), vec![u8t, u8t], u8t),
        (CoreOp::Max(Int::U8), vec![u8t, u8t], u8t),
        (CoreOp::CheckedShl(Int::U8), vec![u8t, u32t], optu8),
        (CoreOp::DurationFromMillis, vec![u64t], d),
        (CoreOp::DurationSaturatingSub, vec![d, d], d),
        (CoreOp::DurationCompare(BinOp::Ge), vec![sd, sd], boolean),
        (CoreOp::OptionGetOrInsert, vec![ropt, d], rd),
        (CoreOp::OptionIsSome, vec![sopt], boolean),
        (CoreOp::Default, vec![], u16t),
    ];
    let others = [u8t, u16t, u32t, boolean, d, optu8, sd, rd_shared];
    let mut cases = vec![];
    for (op, args, dest) in &right {
        cases.push((*op, args.clone(), *dest, true));
        for i in 0..=args.len() {
            for other in others {
                let mut a = args.clone();
                let mut t = *dest;
                if i < a.len() {
                    if a[i] == other { continue; }
                    a[i] = other;
                } else {
                    // a default is any scalar's or a duration's: only another kind of destination is wrong
                    let scalar = matches!(b.program.types[other.0 as usize], Type::Bool | Type::Int(_) | Type::Duration);
                    if t == other || (*op == CoreOp::Default && scalar) { continue; }
                    t = other;
                }
                cases.push((*op, a, t, false));
            }
        }
        let mut more = args.clone();
        more.push(u8t);
        cases.push((*op, more, *dest, false));
    }
    cases.push((CoreOp::DurationCompare(BinOp::Add), vec![sd, sd], boolean, false));
    cases.push((CoreOp::Default, vec![], optu8, false));
    let (mut accepted, mut refusals) = (0, 0);
    for (op, args, dest, good) in cases {
        let mut p = Build { program: b.program.clone() };
        let mut types = vec![dest];
        types.extend(&args);
        let operands = (1..=args.len() as u32).map(copy).collect();
        p.function(Function {
            name: "f".into(),
            params: args.len() as u32,
            locals: locals(&types),
            blocks: vec![
                block(vec![], Terminator::Call { callee: Callee::Core(op), args: operands, dest: place(0), next: BlockId(1), at: at(1) }),
                block(vec![], ret()),
            ],
        });
        let got = Machine::load(p.program, &[]).map(|_| ());
        if good {
            assert_eq!(got, Ok(()), "{op:?}({args:?}) -> {dest:?}");
            accepted += 1;
        } else {
            assert_eq!(got, Err(at_terminator("a core operation called with types it does not take")), "{op:?}({args:?}) -> {dest:?}");
            refusals += 1;
        }
    }
    assert!(accepted == 10 && refusals > 150, "{accepted} accepted, {refusals} refused");
}

#[test]
fn types_frames_stacks_and_regions_past_reach_are_refused_by_their_reasons() {
    // a chain of 258 structures, each holding the next, the outermost first: nested past what the loader walks
    let mut b = Build::default();
    for k in 0..258u32 {
        b.program.types.push(Type::Struct { name: format!("S{k}"), fields: vec![Field { name: "x".into(), ty: TypeId(k + 1) }] });
    }
    b.program.types.push(Type::Int(Int::U8));
    assert!(matches!(refused(b.program), LoadError::Type { reason: "nested too deep", .. }));
    // and 200 deep loads
    let mut b = Build::default();
    for k in 0..200u32 {
        b.program.types.push(Type::Struct { name: format!("S{k}"), fields: vec![Field { name: "x".into(), ty: TypeId(k + 1) }] });
    }
    b.program.types.push(Type::Int(Int::U8));
    assert!(Machine::load(b.program, &[]).is_ok());
    let mut b = Build::default();
    b.ty(Type::Ref { to: TypeId(5), mutable: false });
    assert_eq!(refused(b.program), LoadError::Type { ty: 0, reason: "a reference to no such type" });
    // two locals of 2^31 slots each: a frame past the machine's reach, nothing allocated to find it out
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let half = b.ty(Type::Array { element: u8t, len: 1 << 31 });
    b.function(Function { name: "f".into(), params: 0, locals: locals(&[half, half]), blocks: vec![block(vec![], ret())] });
    assert_eq!(refused(b.program), LoadError::Function { function: "f".into(), reason: "a frame too large" });
    // a call from a frame of 2^31 slots into another: a stack past reach
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let half = b.ty(Type::Array { element: u8t, len: 1 << 31 });
    let unit = b.unit();
    let g = b.function(Function { name: "g".into(), params: 0, locals: locals(&[unit, half]), blocks: vec![block(vec![], ret())] });
    b.function(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[unit, half]),
        blocks: vec![block(vec![], Terminator::Call { callee: Callee::Function(g), args: vec![], dest: place(0), next: BlockId(1), at: at(1) }), block(vec![], ret())],
    });
    assert_eq!(refused(b.program), LoadError::Region { index: 0, reason: "a stack past the machine's reach" });
    // two regions of 2^31 slots: memory past reach
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    let half = b.ty(Type::Array { element: u8t, len: 1 << 31 });
    assert_eq!(
        Machine::load(b.program, &[half, half]).unwrap_err(),
        LoadError::Region { index: 1, reason: "memory past the machine's reach" }
    );
}

/// Reasons no program can meet: more than 2^32 types, functions, blocks or statements; and a variant's field of
/// what is no enum, which the downcast before it already refuses.
const UNREACHABLE: [&str; 5] = ["too many types", "too many functions", "too many blocks", "too many statements", "a variant of what is no enum"];

#[test]
fn every_reason_the_loader_gives_is_met_by_a_test_here() {
    // every reason literal of the loader's two sources: `layout.rs`'s, and the ones `Machine::load` gives in
    // `machine.rs`, each read from its `reason: "..."` (the tests above this one, so a reason named only here is
    // not met)
    let here = include_str!("load.rs").split("/// Reasons no program can meet").next().unwrap_or_default();
    let layout = include_str!("../src/layout.rs");
    let machine = include_str!("../src/machine.rs");
    let mut reasons: Vec<&str> = layout
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && !s.contains('{'))
        .collect();
    for text in [layout, machine] {
        reasons.extend(text.split("reason: \"").skip(1).filter_map(|part| part.split('"').next()));
    }
    reasons.sort();
    reasons.dedup();
    assert!(reasons.len() > 50, "{} reasons read", reasons.len());
    assert!(reasons.contains(&"a region of no such type") && reasons.contains(&"memory past the machine's reach"));
    for reason in reasons {
        if UNREACHABLE.contains(&reason) {
            continue;
        }
        assert!(here.contains(&format!("\"{reason}\"")), "no test here meets \"{reason}\"");
    }
    for reason in UNREACHABLE {
        assert!(layout.contains(&format!("\"{reason}\"")), "\"{reason}\" is still the loader's");
    }
}

#[test]
fn a_core_operation_at_another_integer_type_or_into_another_option_is_refused() {
    let call = |b: &mut Build, op: CoreOp, dest: TypeId, args: &[TypeId]| {
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
    };
    let refused_core = || at_terminator("a core operation called with types it does not take");
    let mut b = Build::default();
    let u16t = b.int(Int::U16);
    call(&mut b, CoreOp::SaturatingAdd(Int::U8), u16t, &[u16t, u16t]);
    assert_eq!(refused(b.program), refused_core(), "the whole call at another integer type");
    let mut b = Build::default();
    let (u8t, u16t, u32t) = (b.int(Int::U8), b.int(Int::U16), b.int(Int::U32));
    let opt16 = b.option(u16t);
    call(&mut b, CoreOp::CheckedShl(Int::U8), opt16, &[u8t, u32t]);
    assert_eq!(refused(b.program), refused_core(), "a checked shift into an Option of another integer");
    let mut b = Build::default();
    let (u8t, d) = (b.int(Int::U8), b.ty(Type::Duration));
    let optu8 = b.option(u8t);
    let (ropt, rd) = (b.reference(optu8, true), b.reference(d, true));
    call(&mut b, CoreOp::OptionGetOrInsert, rd, &[ropt, d]);
    assert_eq!(refused(b.program), refused_core(), "a value of another type than the Option's");
}

#[test]
fn a_reference_to_the_type_just_past_the_last_is_refused() {
    let mut b = Build::default();
    b.ty(Type::Ref { to: TypeId(1), mutable: false });
    assert_eq!(refused(b.program), LoadError::Type { ty: 0, reason: "a reference to no such type" });
    let mut b = Build::default();
    b.ty(Type::Ref { to: TypeId(0), mutable: false });
    assert!(Machine::load(b.program, &[]).is_ok(), "a reference to itself is a slot");
}

#[test]
fn an_array_built_from_a_part_of_another_type_and_a_switch_value_outside_an_integer_s_type_are_refused() {
    let mut b = Build::default();
    let (u8t, u16t) = (b.int(Int::U8), b.int(Int::U16));
    let arr = b.ty(Type::Array { element: u8t, len: 2 });
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[arr, u16t]),
        blocks: vec![block(vec![assign(place(0), Value::Aggregate(arr, None, vec![konst(u8t, vec![int(Int::U8, 1)]), copy(1)]), 1)], ret())],
    });
    assert_eq!(refused(b.program), at_statement("a part of another type than its place"));
    let mut b = Build::default();
    let u8t = b.int(Int::U8);
    b.function(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[u8t, u8t]),
        blocks: vec![
            block(vec![], Terminator::SwitchInt { value: copy(1), targets: vec![(256, BlockId(1))], otherwise: BlockId(1), at: at(1) }),
            block(vec![assign(place(0), Value::Use(copy(1)), 2)], ret()),
        ],
    });
    assert_eq!(refused(b.program), at_terminator("a switch's value outside its operand's type"));
}

/// `u8`, then `n` structures each holding the one before: the order an interner gives, the innermost first.
fn chain_inner_first(n: u32) -> (Program, TypeId) {
    let mut p = Program::default();
    p.types.push(Type::Int(Int::U8));
    for k in 0..n {
        p.types.push(Type::Struct { name: String::new(), fields: vec![Field { name: String::new(), ty: TypeId(k) }] });
    }
    (p, TypeId(n))
}

#[test]
fn a_nesting_past_the_limit_is_refused_whatever_order_the_types_come_in_and_never_by_a_crash() {
    let (p, _) = chain_inner_first(258);
    assert!(matches!(refused(p), LoadError::Type { reason: "nested too deep", .. }), "the innermost first");
    let (p, _) = chain_inner_first(255);
    assert!(Machine::load(p, &[]).is_ok(), "255 structures around a u8: 256 deep, at the limit");
    let (p, _) = chain_inner_first(256);
    assert!(matches!(refused(p), LoadError::Type { reason: "nested too deep", .. }), "one past it");
    // a constant of a type nested 100 000 deep, read from a file, loaded on a thread of 8 MiB: refused by its
    // reason, where the check of the constant's slots once recursed as deep and overflowed the stack
    let (mut p, outer) = chain_inner_first(100_000);
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[outer]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(outer, vec![int(Int::U8, 1)])), 1)], ret())],
    });
    let header = orcl_interp::file::Header { language: 1, nightly: "n".into(), revision: "r".into() };
    let (_, back) = orcl_interp::file::read(&orcl_interp::file::write(&header, &p)).unwrap();
    let got = std::thread::Builder::new().stack_size(8 << 20).spawn(move || Machine::load(back, &[]).map(|_| ())).unwrap().join();
    assert!(matches!(got, Ok(Err(LoadError::Type { reason: "nested too deep", .. }))), "{got:?}");
}

#[test]
fn an_array_of_elements_of_no_slot_is_checked_once_however_long() {
    // a constant of `[[(); u32::MAX]; u32::MAX]`: no slot at all. Checked element by element it took hours; the
    // thread's 60 seconds are a guard against a hang, not a measure
    let mut p = Program::default();
    p.types.push(Type::Struct { name: "()".into(), fields: vec![] });
    p.types.push(Type::Array { element: TypeId(0), len: u32::MAX });
    p.types.push(Type::Array { element: TypeId(1), len: u32::MAX });
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[TypeId(2)]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(TypeId(2), vec![])), 1)], ret())],
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Machine::load(p, &[]).map(|m| m.memory().len()));
    });
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(60)), Ok(Ok(0)));
}

#[test]
fn an_enum_of_many_variants_is_checked_for_two_with_one_discriminant_without_comparing_every_pair() {
    let variants = |n: i128, clash: bool| {
        let mut v: Vec<Variant> = (0..n).map(|d| Variant { name: String::new(), discriminant: d, fields: vec![] }).collect();
        if clash {
            v.push(Variant { name: String::new(), discriminant: n / 2, fields: vec![] });
        }
        v
    };
    // 200 000 variants: every pair compared is 2 * 10^10 comparisons; sorted, a few million
    let mut b = Build::default();
    b.ty(Type::Enum { name: "E".into(), discriminant: Int::I32, variants: variants(200_000, true) });
    assert_eq!(refused(b.program), LoadError::Type { ty: 0, reason: "two variants with one discriminant" });
    let mut b = Build::default();
    b.ty(Type::Enum { name: "E".into(), discriminant: Int::I32, variants: variants(200_000, false) });
    assert!(Machine::load(b.program, &[]).is_ok());
}

fn on_a_thread_of_8_mib(p: Program) -> Result<(), LoadError> {
    std::thread::Builder::new().stack_size(8 << 20).spawn(move || Machine::load(p, &[]).map(|_| ())).unwrap().join().unwrap()
}

/// `u8`, then `n` types each holding the one before through `wrap`, the innermost first; a function holding a
/// constant of the outermost, its slots `slots`.
fn wrapped_inner_first(n: u32, wrap: impl Fn(TypeId) -> Type, slots: Vec<Scalar>) -> Program {
    let mut p = Program::default();
    p.types.push(Type::Int(Int::U8));
    for k in 0..n {
        p.types.push(wrap(TypeId(k)));
    }
    let outer = TypeId(n);
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[outer]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(outer, slots)), 1)], ret())],
    });
    p
}

fn array1(t: TypeId) -> Type {
    Type::Array { element: t, len: 1 }
}

fn enum1(t: TypeId) -> Type {
    Type::Enum { name: "E".into(), discriminant: Int::U8, variants: vec![Variant { name: "A".into(), discriminant: 0, fields: vec![Field { name: "0".into(), ty: t }] }] }
}

fn struct1(t: TypeId) -> Type {
    Type::Struct { name: String::new(), fields: vec![Field { name: String::new(), ty: t }] }
}

fn too_deep(r: Result<(), LoadError>) -> bool {
    matches!(r, Err(LoadError::Type { reason: "nested too deep", .. }))
}

#[test]
fn arrays_and_enums_nested_past_the_limit_innermost_first_are_refused_and_a_structure_counts_its_deepest_field() {
    let one = || vec![int(Int::U8, 1)];
    assert!(too_deep(on_a_thread_of_8_mib(wrapped_inner_first(256, array1, one()))), "256 arrays around a u8");
    assert!(on_a_thread_of_8_mib(wrapped_inner_first(255, array1, one())).is_ok(), "255 arrays");
    assert!(too_deep(on_a_thread_of_8_mib(wrapped_inner_first(100_000, array1, one()))), "100 000 arrays");
    // a constant of n enums around a u8: n discriminants of 0, then the u8
    let slots = |n: usize| {
        let mut s = vec![int(Int::U8, 0); n];
        s.push(int(Int::U8, 1));
        s
    };
    assert!(too_deep(on_a_thread_of_8_mib(wrapped_inner_first(256, enum1, slots(256)))), "256 enums around a u8");
    assert!(on_a_thread_of_8_mib(wrapped_inner_first(255, enum1, slots(255))).is_ok(), "255 enums");
    assert!(too_deep(on_a_thread_of_8_mib(wrapped_inner_first(100_000, enum1, slots(100_000)))), "100 000 enums");
    // 255 structures around a u8 (256 deep, at the limit), then one holding (that, u8): 257 deep by its first field
    let mut p = Program::default();
    p.types.push(Type::Int(Int::U8));
    for k in 0..255u32 {
        p.types.push(struct1(TypeId(k)));
    }
    p.types.push(Type::Struct { name: String::new(), fields: vec![Field { name: "a".into(), ty: TypeId(255) }, Field { name: "b".into(), ty: TypeId(0) }] });
    assert!(too_deep(on_a_thread_of_8_mib(p)), "a structure whose first field is the deep one");
}

#[test]
fn a_nesting_of_100_000_outermost_first_is_refused_through_every_kind_of_part_and_names_a_type_nested_too_deep() {
    for wrap in [array1 as fn(TypeId) -> Type, enum1, struct1] {
        let mut p = Program::default();
        for k in 0..100_000u32 {
            p.types.push(wrap(TypeId(k + 1)));
        }
        p.types.push(Type::Int(Int::U8));
        assert!(too_deep(on_a_thread_of_8_mib(p)));
    }
    // S0 holds S1 ... S257 holds a u8: S0 is 259 deep, S257 two; the type named is one past the limit
    let mut p = Program::default();
    for k in 0..258u32 {
        p.types.push(struct1(TypeId(k + 1)));
    }
    p.types.push(Type::Int(Int::U8));
    match refused(p) {
        LoadError::Type { ty, reason: "nested too deep" } => assert!(259 - ty > 256, "type {ty} named, {} deep", 259 - ty),
        other => panic!("{other:?}"),
    }
}

/// `f` on a thread of its own, given 60 seconds: a guard against a hang, not a measure.
fn within_a_minute<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(60)).ok()
}

/// T0 = (), Tk = (T(k-1), T(k-1)): types of no slot, `k + 1` deep, each twice as wide as the one before.
fn wide_of_no_slot(k: u32) -> (Program, TypeId) {
    let mut p = Program::default();
    p.types.push(Type::Struct { name: "()".into(), fields: vec![] });
    for i in 1..=k {
        p.types.push(Type::Struct { name: String::new(), fields: vec![Field { name: String::new(), ty: TypeId(i - 1) }, Field { name: String::new(), ty: TypeId(i - 1) }] });
    }
    (p, TypeId(k))
}

#[test]
fn a_constant_an_argument_and_a_region_of_a_wide_type_of_no_slot_are_checked_at_once() {
    // 65 deep and 2^64 leaves wide, a file of under 2 kB: walked as a tree, its check would never end
    let (mut p, top) = wide_of_no_slot(64);
    p.functions.push(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[TypeId(0), top, top]),
        blocks: vec![block(vec![assign(place(2), Value::Use(konst(top, vec![])), 1)], ret())],
    });
    let got = within_a_minute(move || {
        let mut m = Machine::load(p, &[top]).unwrap();
        (m.set_region(RegionId(0), &[]), m.start(FunctionId(0), &[Arg::Value(&[])]), m.run(u64::MAX))
    });
    assert_eq!(got, Some((Ok(()), Ok(()), Ok(Step::Returned))));
}

#[test]
fn a_constant_array_of_an_enum_of_many_variants_finds_each_variant_by_its_discriminant() {
    // 200 000 variants and 200 000 elements, each the last variant: 4 * 10^10 comparisons if searched one by one
    let n = 200_000u32;
    let mut p = Program::default();
    let variants = (0..n).map(|d| Variant { name: String::new(), discriminant: i128::from(d), fields: vec![] }).collect();
    p.types.push(Type::Enum { name: "E".into(), discriminant: Int::I32, variants });
    p.types.push(Type::Array { element: TypeId(0), len: n });
    let slots = vec![Scalar::Int(Int::I32, i128::from(n) - 1); n as usize];
    let mut wrong = slots.clone();
    wrong[n as usize / 2] = Scalar::Int(Int::I32, i128::from(n));
    for (s, good) in [(slots, true), (wrong, false)] {
        let mut q = p.clone();
        q.functions.push(Function {
            name: "f".into(),
            params: 0,
            locals: locals(&[TypeId(1)]),
            blocks: vec![block(vec![assign(place(0), Value::Use(konst(TypeId(1), s)), 1)], ret())],
        });
        let got = within_a_minute(move || Machine::load(q, &[]).map(|_| ()));
        if good {
            assert_eq!(got, Some(Ok(())));
        } else {
            assert_eq!(got, Some(Err(at_statement("a constant whose slots are not its type's"))), "a discriminant no variant has");
        }
    }
}

#[test]
fn a_wide_part_of_no_slot_inside_a_type_with_a_slot_is_checked_at_once() {
    // T0 = (), Tk = (Tk-1, Tk-1) to T64; then S = (bool, T64): one slot, beside a part 2^64 leaves wide
    let (mut p, wide) = wide_of_no_slot(64);
    p.types.push(Type::Bool);
    let boolean = TypeId(p.types.len() as u32 - 1);
    p.types.push(Type::Struct { name: "S".into(), fields: vec![Field { name: "a".into(), ty: boolean }, Field { name: "w".into(), ty: wide }] });
    let s = TypeId(p.types.len() as u32 - 1);
    let one = vec![Scalar::Bool(true)];
    p.functions.push(Function {
        name: "f".into(),
        params: 1,
        locals: locals(&[TypeId(0), s, s]),
        blocks: vec![block(vec![assign(place(2), Value::Use(konst(s, one.clone())), 1)], ret())],
    });
    let got = within_a_minute(move || {
        let mut m = Machine::load(p, &[s]).unwrap();
        (m.set_region(RegionId(0), &one), m.start(FunctionId(0), &[Arg::Value(&one)]), m.run(u64::MAX))
    });
    assert_eq!(got, Some((Ok(()), Ok(()), Ok(Step::Returned))));
}

#[test]
fn a_structure_of_one_slot_beside_many_fields_of_no_slot_is_checked_by_its_one_slot() {
    // S = { bool, 100 000 x () }, and a constant [S; 100 000]: walked field by field, 10^10 checks; by the fields
    // that hold a slot, 100 000
    let mut p = Program::default();
    p.types.push(Type::Struct { name: "()".into(), fields: vec![] });
    p.types.push(Type::Bool);
    let mut fields = vec![Field { name: "b".into(), ty: TypeId(1) }];
    fields.extend((0..100_000).map(|_| Field { name: String::new(), ty: TypeId(0) }));
    p.types.push(Type::Struct { name: "S".into(), fields });
    p.types.push(Type::Array { element: TypeId(2), len: 100_000 });
    let slots = vec![Scalar::Bool(true); 100_000];
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[TypeId(3)]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(TypeId(3), slots)), 1)], ret())],
    });
    let mut bad = p.clone();
    if let Value::Use(Operand::Const(c)) = &mut bad.functions[0].blocks[0].statements[0].value {
        c.slots[77_777] = int(Int::U8, 1);
    }
    assert_eq!(within_a_minute(move || Machine::load(p, &[]).map(|_| ())), Some(Ok(())));
    assert_eq!(within_a_minute(move || Machine::load(bad, &[]).map(|_| ())), Some(Err(at_statement("a constant whose slots are not its type's"))), "one slot of another kind");
}

#[test]
fn a_nesting_past_the_limit_names_a_type_nested_past_it_wherever_the_chain_starts_and_walks_on_a_small_stack() {
    // type 0 a u8; types 1 to 258 each holding the next, 258 holding type 0: type k is 260 - k deep
    let mut p = Program::default();
    p.types.push(Type::Int(Int::U8));
    for k in 1..=258u32 {
        p.types.push(struct1(TypeId(if k == 258 { 0 } else { k + 1 })));
    }
    match refused(p) {
        LoadError::Type { ty, reason: "nested too deep" } => assert!(ty >= 1 && 260 - ty > 256, "type {ty} named"),
        other => panic!("{other:?}"),
    }
    // 100 000 structures outermost first, on Rust's default thread stack of 2 MiB: the walk stops at the limit
    let mut p = Program::default();
    for k in 0..100_000u32 {
        p.types.push(struct1(TypeId(k + 1)));
    }
    p.types.push(Type::Int(Int::U8));
    let got = std::thread::Builder::new().stack_size(2 << 20).spawn(move || Machine::load(p, &[]).map(|_| ())).unwrap().join().ok();
    assert!(matches!(got, Some(Err(LoadError::Type { reason: "nested too deep", .. }))), "{got:?}");
}

#[test]
fn a_variant_of_one_slot_beside_many_fields_of_no_slot_is_checked_by_its_one_slot() {
    // enum E { A(bool, 100 000 x ()) }, and a constant [E; 100 000]: walked field by field, 10^10 checks
    let mut p = Program::default();
    p.types.push(Type::Struct { name: "()".into(), fields: vec![] });
    p.types.push(Type::Bool);
    let mut fields = vec![Field { name: "0".into(), ty: TypeId(1) }];
    fields.extend((0..100_000).map(|_| Field { name: String::new(), ty: TypeId(0) }));
    p.types.push(Type::Enum { name: "E".into(), discriminant: Int::U8, variants: vec![Variant { name: "A".into(), discriminant: 0, fields }] });
    p.types.push(Type::Array { element: TypeId(2), len: 100_000 });
    let mut slots = Vec::new();
    for _ in 0..100_000 {
        slots.push(int(Int::U8, 0));
        slots.push(Scalar::Bool(true));
    }
    p.functions.push(Function {
        name: "f".into(),
        params: 0,
        locals: locals(&[TypeId(3)]),
        blocks: vec![block(vec![assign(place(0), Value::Use(konst(TypeId(3), slots)), 1)], ret())],
    });
    let mut bad = p.clone();
    if let Value::Use(Operand::Const(c)) = &mut bad.functions[0].blocks[0].statements[0].value {
        c.slots[77_777] = int(Int::U8, 1);
    }
    assert_eq!(within_a_minute(move || Machine::load(p, &[]).map(|_| ())), Some(Ok(())));
    assert_eq!(within_a_minute(move || Machine::load(bad, &[]).map(|_| ())), Some(Err(at_statement("a constant whose slots are not its type's"))), "one slot of another kind");
}
