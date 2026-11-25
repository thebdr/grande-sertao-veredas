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
