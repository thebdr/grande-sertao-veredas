//! A program as the interpreter runs it: its types, its functions in the shape of rustc's MIR at opt-level 0, and
//! the source files its locations name (`docs/orcl/interpreter.md`). The front end builds one while rustc compiles
//! the program's crates; a test builds one by hand.

/// An index into [`Program::types`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

/// An index into [`Program::functions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub u32);

/// A function's local, as MIR numbers them: 0 is the return value, 1 to `params` the parameters, then the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Local(pub u32);

/// An index into a function's [`Function::blocks`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// A program: what the machine loads.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Program {
    pub types: Vec<Type>,
    pub functions: Vec<Function>,
    /// The source files a [`Span`] names, by index.
    pub files: Vec<String>,
}

/// An integer type. ORCL's own are the eight with a bit size; `Isize` and `Usize` are the compiler's, for an enum's
/// discriminant and an array's index, and are 64 bits wide (controlador runs on 64-bit processors only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Int {
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

impl Int {
    /// The width in bits.
    pub const fn bits(self) -> u32 {
        match self {
            Int::I8 | Int::U8 => 8,
            Int::I16 | Int::U16 => 16,
            Int::I32 | Int::U32 => 32,
            Int::I64 | Int::U64 | Int::Isize | Int::Usize => 64,
        }
    }

    pub const fn signed(self) -> bool {
        matches!(self, Int::I8 | Int::I16 | Int::I32 | Int::I64 | Int::Isize)
    }

    /// The least value of the type.
    pub const fn min(self) -> i128 {
        match self {
            Int::I8 => i8::MIN as i128,
            Int::I16 => i16::MIN as i128,
            Int::I32 => i32::MIN as i128,
            Int::I64 | Int::Isize => i64::MIN as i128,
            Int::U8 | Int::U16 | Int::U32 | Int::U64 | Int::Usize => 0,
        }
    }

    /// The greatest value of the type.
    pub const fn max(self) -> i128 {
        match self {
            Int::I8 => i8::MAX as i128,
            Int::I16 => i16::MAX as i128,
            Int::I32 => i32::MAX as i128,
            Int::I64 | Int::Isize => i64::MAX as i128,
            Int::U8 => u8::MAX as i128,
            Int::U16 => u16::MAX as i128,
            Int::U32 => u32::MAX as i128,
            Int::U64 | Int::Usize => u64::MAX as i128,
        }
    }

    /// `value` wrapped into the type, as two's complement does: what Rust's wrapping operations and `as` give.
    pub const fn wrap(self, value: i128) -> i128 {
        match self {
            Int::I8 => value as i8 as i128,
            Int::I16 => value as i16 as i128,
            Int::I32 => value as i32 as i128,
            Int::I64 | Int::Isize => value as i64 as i128,
            Int::U8 => value as u8 as i128,
            Int::U16 => value as u16 as i128,
            Int::U32 => value as u32 as i128,
            Int::U64 | Int::Usize => value as u64 as i128,
        }
    }

    /// Whether `value` is one of the type's.
    pub const fn holds(self, value: i128) -> bool {
        value >= self.min() && value <= self.max()
    }
}

/// A type of the program. A tuple, `()` among them, is a structure whose fields are named 0, 1 and so on; an
/// `Option<T>` is an enum named `Option` whose variant 0, `None`, holds nothing and variant 1, `Some`, one field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Bool,
    Int(Int),
    F32,
    F64,
    /// `core::time::Duration`, opaque: only the operations of the list read or build one. It takes two slots, its
    /// whole seconds (`u64`) and its nanoseconds below a second (`u32`).
    Duration,
    Struct { name: String, fields: Vec<Field> },
    Enum { name: String, discriminant: Int, variants: Vec<Variant> },
    Array { element: TypeId, len: u32 },
    /// Held by a call's parameters and its locals only (ORCL's O2 and O3).
    Ref { to: TypeId, mutable: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: TypeId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub discriminant: i128,
    pub fields: Vec<Field>,
}

/// A function's body.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// Its source path, such as `Conveyor::scan`.
    pub name: String,
    /// How many of its locals after the return value are its parameters.
    pub params: u32,
    /// Local 0 first: its return value.
    pub locals: Vec<LocalDecl>,
    /// Block 0 first: where it starts.
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    pub ty: TypeId,
    /// The source's name, where the source gave one.
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

/// `place = value`: the one statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub place: Place,
    pub value: Value,
    pub at: Span,
}

/// A local, then a path into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub local: Local,
    pub projections: Vec<Projection>,
}

impl Place {
    /// The local itself.
    pub fn local(local: u32) -> Place {
        Place { local: Local(local), projections: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Projection {
    /// A structure's field, or a field of the variant a [`Projection::Downcast`] chose.
    Field(u32),
    /// What a reference refers to.
    Deref,
    /// An array's element at a `Usize` local's value; past the end is a program error.
    Index(Local),
    /// An array's element at a fixed index.
    ConstantIndex(u32),
    /// An enum seen as one of its variants, whose fields a [`Projection::Field`] then reaches.
    Downcast(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Const(Const),
}

/// A constant: a type and its slots, as [`crate::Scalar`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct Const {
    pub ty: TypeId,
    pub slots: Vec<crate::Scalar>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Not,
    Neg,
}

/// What a statement assigns.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Use(Operand),
    /// MIR's plain operation: `+ - *` wrap, a shift takes its amount modulo the width, `/` and `%` a program error
    /// on a zero divisor or an overflow (which MIR's own assertion catches first).
    Binary(BinOp, Operand, Operand),
    /// MIR's checked form of `+ - *`: a tuple of the wrapped result and whether it overflowed.
    Checked(BinOp, Operand, Operand),
    /// `!` on a `bool` or an integer, `-` wrapping on a signed integer or a float.
    Unary(UnOp, Operand),
    /// Rust's `as` between `bool`, the integers and the floats.
    Cast(Operand, TypeId),
    /// A structure, an array, or an enum's variant (`Some(variant)`), from its fields' values.
    Aggregate(TypeId, Option<u32>, Vec<Operand>),
    /// An enum's discriminant.
    Discriminant(Place),
    /// A reference to a place; `true` for a mutable one.
    Ref(Place, bool),
}

/// How a block ends.
#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    Goto(BlockId),
    /// To the target whose value the operand has, else to `otherwise`.
    SwitchInt { value: Operand, targets: Vec<(i128, BlockId)>, otherwise: BlockId, at: Span },
    Return { at: Span },
    Call { callee: Callee, args: Vec<Operand>, dest: Place, next: BlockId, at: Span },
    /// A program error unless the operand is `expected`; MIR's run-time checks.
    Assert { cond: Operand, expected: bool, check: Check, next: BlockId, at: Span },
    Unreachable { at: Span },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callee {
    Function(FunctionId),
    Core(crate::CoreOp),
}

/// What a failed assertion of MIR checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Check {
    Overflow(BinOp),
    NegOverflow,
    DivisionByZero,
    RemainderByZero,
    Bounds,
}

/// A source location: an index into [`Program::files`], a line and a column, each from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: u32,
    pub line: u32,
    pub column: u32,
}
