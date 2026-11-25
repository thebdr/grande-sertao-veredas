//! The program file, `<name>.orclb` (`docs/controlador/files.md`): a header of its own, then the program, in a
//! codec of the interpreter's own, little-endian, with no dependency (`docs/orcl/interpreter.md`, "The program
//! file"). A reader refuses a file whose format or language it does not run, naming what it found, and refuses a
//! malformed one at the byte where it stops making sense; it never guesses and never panics.
use core::fmt;

use crate::core_ops::CoreOp;
use crate::ops::Scalar;
use crate::program::{
    BinOp, Block, BlockId, Callee, Check, Const, Field, Function, FunctionId, Int, Local, LocalDecl, Operand, Place,
    Program, Projection, Span, Statement, Terminator, Type, TypeId, UnOp, Value, Variant,
};

/// The file's first bytes.
pub const MAGIC: [u8; 6] = *b"ORCLB\0";

/// The version of this file's format, from 1; a reader runs this one alone.
pub const FORMAT: u16 = 1;

/// The ORCL versions this interpreter runs (`docs/questions.md` Q220): the language's own number, which `orcl-std`
/// states as `LANGUAGE`.
pub const LANGUAGES: &[u16] = &[1];

/// What the header says beside the format: the language the program is written in, the nightly that lowered it
/// (`docs/questions.md` Q222), and the source revision its sender gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub language: u16,
    pub nightly: String,
    pub revision: String,
}

/// Why a file is not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileError {
    /// It does not start with [`MAGIC`].
    NotAProgram,
    /// A format this reader does not run.
    Format { found: u16 },
    /// A language version this interpreter does not run.
    Language { found: u16 },
    /// A file that stops making sense at byte `at`: cut short, a tag or a value no program has, text that is not
    /// UTF-8, or bytes past the program's end.
    Malformed { at: usize, what: &'static str },
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::NotAProgram => write!(f, "not an ORCL program file"),
            FileError::Format { found } => write!(f, "program file format {found}; this reader runs {FORMAT}"),
            FileError::Language { found } => write!(f, "ORCL {found}; this interpreter runs ORCL {LANGUAGES:?}"),
            FileError::Malformed { at, what } => write!(f, "byte {at}: {what}"),
        }
    }
}

impl core::error::Error for FileError {}

const INTS: [Int; 10] = [Int::I8, Int::I16, Int::I32, Int::I64, Int::Isize, Int::U8, Int::U16, Int::U32, Int::U64, Int::Usize];

const BINOPS: [BinOp; 16] = [
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

fn code_of<T: PartialEq>(table: &[T], value: &T) -> u8 {
    table.iter().position(|t| t == value).and_then(|i| u8::try_from(i).ok()).unwrap_or(u8::MAX)
}

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i128(&mut self, v: i128) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.out.extend_from_slice(s.as_bytes());
    }
    fn int(&mut self, t: Int) {
        self.u8(code_of(&INTS, &t));
    }
    fn binop(&mut self, op: BinOp) {
        self.u8(code_of(&BINOPS, &op));
    }
    fn fields(&mut self, fields: &[Field]) {
        self.len(fields.len());
        for f in fields {
            self.str(&f.name);
            self.u32(f.ty.0);
        }
    }
    fn ty(&mut self, ty: &Type) {
        match ty {
            Type::Bool => self.u8(0),
            Type::Int(t) => {
                self.u8(1);
                self.int(*t);
            }
            Type::F32 => self.u8(2),
            Type::F64 => self.u8(3),
            Type::Duration => self.u8(4),
            Type::Struct { name, fields } => {
                self.u8(5);
                self.str(name);
                self.fields(fields);
            }
            Type::Enum { name, discriminant, variants } => {
                self.u8(6);
                self.str(name);
                self.int(*discriminant);
                self.len(variants.len());
                for v in variants {
                    self.str(&v.name);
                    self.i128(v.discriminant);
                    self.fields(&v.fields);
                }
            }
            Type::Array { element, len } => {
                self.u8(7);
                self.u32(element.0);
                self.u32(*len);
            }
            Type::Ref { to, mutable } => {
                self.u8(8);
                self.u32(to.0);
                self.bool(*mutable);
            }
        }
    }
    fn span(&mut self, s: Span) {
        self.u32(s.file);
        self.u32(s.line);
        self.u32(s.column);
    }
    fn place(&mut self, p: &Place) {
        self.u32(p.local.0);
        self.len(p.projections.len());
        for proj in &p.projections {
            match proj {
                Projection::Field(i) => {
                    self.u8(0);
                    self.u32(*i);
                }
                Projection::Deref => self.u8(1),
                Projection::Index(l) => {
                    self.u8(2);
                    self.u32(l.0);
                }
                Projection::ConstantIndex(i) => {
                    self.u8(3);
                    self.u32(*i);
                }
                Projection::Downcast(v) => {
                    self.u8(4);
                    self.u32(*v);
                }
            }
        }
    }
    fn scalar(&mut self, s: Scalar) {
        match s {
            Scalar::Uninit => self.u8(0),
            Scalar::Bool(b) => {
                self.u8(1);
                self.bool(b);
            }
            Scalar::Int(t, v) => {
                self.u8(2);
                self.int(t);
                self.i128(v);
            }
            Scalar::F32(x) => {
                self.u8(3);
                self.u32(x.to_bits());
            }
            Scalar::F64(x) => {
                self.u8(4);
                self.u64(x.to_bits());
            }
            Scalar::Ref(at) => {
                self.u8(5);
                self.u32(at);
            }
        }
    }
    fn operand(&mut self, op: &Operand) {
        match op {
            Operand::Copy(p) => {
                self.u8(0);
                self.place(p);
            }
            Operand::Move(p) => {
                self.u8(1);
                self.place(p);
            }
            Operand::Const(c) => {
                self.u8(2);
                self.u32(c.ty.0);
                self.len(c.slots.len());
                for s in &c.slots {
                    self.scalar(*s);
                }
            }
        }
    }
    fn operands(&mut self, ops: &[Operand]) {
        self.len(ops.len());
        for op in ops {
            self.operand(op);
        }
    }
    fn value(&mut self, v: &Value) {
        match v {
            Value::Use(a) => {
                self.u8(0);
                self.operand(a);
            }
            Value::Binary(op, a, b) => {
                self.u8(1);
                self.binop(*op);
                self.operand(a);
                self.operand(b);
            }
            Value::Checked(op, a, b) => {
                self.u8(2);
                self.binop(*op);
                self.operand(a);
                self.operand(b);
            }
            Value::Unary(op, a) => {
                self.u8(3);
                self.u8(match op {
                    UnOp::Not => 0,
                    UnOp::Neg => 1,
                });
                self.operand(a);
            }
            Value::Cast(a, to) => {
                self.u8(4);
                self.operand(a);
                self.u32(to.0);
            }
            Value::Aggregate(ty, variant, parts) => {
                self.u8(5);
                self.u32(ty.0);
                match variant {
                    None => self.u8(0),
                    Some(v) => {
                        self.u8(1);
                        self.u32(*v);
                    }
                }
                self.operands(parts);
            }
            Value::Discriminant(p) => {
                self.u8(6);
                self.place(p);
            }
            Value::Ref(p, mutable) => {
                self.u8(7);
                self.place(p);
                self.bool(*mutable);
            }
        }
    }
    fn check(&mut self, c: Check) {
        match c {
            Check::Overflow(op) => {
                self.u8(0);
                self.binop(op);
            }
            Check::NegOverflow => self.u8(1),
            Check::DivisionByZero => self.u8(2),
            Check::RemainderByZero => self.u8(3),
            Check::Bounds => self.u8(4),
        }
    }
    fn core(&mut self, op: CoreOp) {
        let with_int = |w: &mut Writer, tag: u8, t: Int| {
            w.u8(tag);
            w.int(t);
        };
        match op {
            CoreOp::SaturatingAdd(t) => with_int(self, 0, t),
            CoreOp::SaturatingSub(t) => with_int(self, 1, t),
            CoreOp::SaturatingMul(t) => with_int(self, 2, t),
            CoreOp::WrappingAdd(t) => with_int(self, 3, t),
            CoreOp::WrappingSub(t) => with_int(self, 4, t),
            CoreOp::WrappingMul(t) => with_int(self, 5, t),
            CoreOp::CheckedShl(t) => with_int(self, 6, t),
            CoreOp::CheckedShr(t) => with_int(self, 7, t),
            CoreOp::Min(t) => with_int(self, 8, t),
            CoreOp::Max(t) => with_int(self, 9, t),
            CoreOp::DurationFromMillis => self.u8(10),
            CoreOp::DurationFromSecs => self.u8(11),
            CoreOp::DurationSaturatingSub => self.u8(12),
            CoreOp::DurationMin => self.u8(13),
            CoreOp::DurationMax => self.u8(14),
            CoreOp::DurationCompare(op) => {
                self.u8(15);
                self.binop(op);
            }
            CoreOp::OptionGetOrInsert => self.u8(16),
            CoreOp::OptionIsNone => self.u8(17),
            CoreOp::OptionIsSome => self.u8(18),
            CoreOp::Default => self.u8(19),
        }
    }
    fn terminator(&mut self, t: &Terminator) {
        match t {
            Terminator::Goto(b) => {
                self.u8(0);
                self.u32(b.0);
            }
            Terminator::SwitchInt { value, targets, otherwise, at } => {
                self.u8(1);
                self.operand(value);
                self.len(targets.len());
                for (v, b) in targets {
                    self.i128(*v);
                    self.u32(b.0);
                }
                self.u32(otherwise.0);
                self.span(*at);
            }
            Terminator::Return { at } => {
                self.u8(2);
                self.span(*at);
            }
            Terminator::Call { callee, args, dest, next, at } => {
                self.u8(3);
                match callee {
                    Callee::Function(f) => {
                        self.u8(0);
                        self.u32(f.0);
                    }
                    Callee::Core(op) => {
                        self.u8(1);
                        self.core(*op);
                    }
                }
                self.operands(args);
                self.place(dest);
                self.u32(next.0);
                self.span(*at);
            }
            Terminator::Assert { cond, expected, check, next, at } => {
                self.u8(4);
                self.operand(cond);
                self.bool(*expected);
                self.check(*check);
                self.u32(next.0);
                self.span(*at);
            }
            Terminator::Unreachable { at } => {
                self.u8(5);
                self.span(*at);
            }
        }
    }
    fn function(&mut self, f: &Function) {
        self.str(&f.name);
        self.u32(f.params);
        self.len(f.locals.len());
        for l in &f.locals {
            self.u32(l.ty.0);
            match &l.name {
                None => self.u8(0),
                Some(n) => {
                    self.u8(1);
                    self.str(n);
                }
            }
        }
        self.len(f.blocks.len());
        for b in &f.blocks {
            self.len(b.statements.len());
            for s in &b.statements {
                self.place(&s.place);
                self.value(&s.value);
                self.span(s.at);
            }
            self.terminator(&b.terminator);
        }
    }
}

/// The program file's bytes for `program` under `header`.
pub fn write(header: &Header, program: &Program) -> Vec<u8> {
    let mut w = Writer { out: Vec::new() };
    w.out.extend_from_slice(&MAGIC);
    w.u16(FORMAT);
    w.u16(header.language);
    w.str(&header.nightly);
    w.str(&header.revision);
    w.len(program.files.len());
    for f in &program.files {
        w.str(f);
    }
    w.len(program.types.len());
    for t in &program.types {
        w.ty(t);
    }
    w.len(program.functions.len());
    for f in &program.functions {
        w.function(f);
    }
    w.out
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

type Read<T> = Result<T, FileError>;

impl Reader<'_> {
    fn bad(&self, what: &'static str) -> FileError {
        FileError::Malformed { at: self.at, what }
    }

    fn take(&mut self, n: usize) -> Read<&[u8]> {
        let end = self.at.checked_add(n).ok_or(self.bad("cut short"))?;
        let slice = self.bytes.get(self.at..end).ok_or(self.bad("cut short"))?;
        self.at = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Read<[u8; N]> {
        let slice = self.take(N)?;
        <[u8; N]>::try_from(slice).map_err(|_| FileError::Malformed { at: 0, what: "cut short" })
    }

    fn u8(&mut self) -> Read<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }
    fn u16(&mut self) -> Read<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Read<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Read<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn i128(&mut self) -> Read<i128> {
        Ok(i128::from_le_bytes(self.array()?))
    }

    fn bool(&mut self) -> Read<bool> {
        let at = self.at;
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(FileError::Malformed { at, what: "a bool that is neither 0 nor 1" }),
        }
    }

    /// A count of things each at least a byte long: never more than the bytes left, so a damaged count cannot ask
    /// for more memory than the file has.
    fn len(&mut self) -> Read<usize> {
        let at = self.at;
        let n = self.u32()? as usize;
        if n > self.bytes.len().saturating_sub(self.at) {
            return Err(FileError::Malformed { at, what: "a count past the file's end" });
        }
        Ok(n)
    }

    fn str(&mut self) -> Read<String> {
        let n = self.len()?;
        let at = self.at;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| FileError::Malformed { at, what: "text that is not UTF-8" })
    }

    fn tag(&mut self, what: &'static str) -> Read<(u8, FileError)> {
        let err = self.bad(what);
        Ok((self.u8()?, err))
    }

    fn int(&mut self) -> Read<Int> {
        let (code, err) = self.tag("no such integer type")?;
        INTS.get(code as usize).copied().ok_or(err)
    }

    fn binop(&mut self) -> Read<BinOp> {
        let (code, err) = self.tag("no such operation")?;
        BINOPS.get(code as usize).copied().ok_or(err)
    }

    fn fields(&mut self) -> Read<Vec<Field>> {
        let n = self.len()?;
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(Field { name: self.str()?, ty: TypeId(self.u32()?) });
        }
        Ok(out)
    }

    fn ty(&mut self) -> Read<Type> {
        let (tag, err) = self.tag("no such type kind")?;
        Ok(match tag {
            0 => Type::Bool,
            1 => Type::Int(self.int()?),
            2 => Type::F32,
            3 => Type::F64,
            4 => Type::Duration,
            5 => Type::Struct { name: self.str()?, fields: self.fields()? },
            6 => {
                let name = self.str()?;
                let discriminant = self.int()?;
                let n = self.len()?;
                let mut variants = Vec::new();
                for _ in 0..n {
                    variants.push(Variant { name: self.str()?, discriminant: self.i128()?, fields: self.fields()? });
                }
                Type::Enum { name, discriminant, variants }
            }
            7 => Type::Array { element: TypeId(self.u32()?), len: self.u32()? },
            8 => Type::Ref { to: TypeId(self.u32()?), mutable: self.bool()? },
            _ => return Err(err),
        })
    }

    fn span(&mut self) -> Read<Span> {
        Ok(Span { file: self.u32()?, line: self.u32()?, column: self.u32()? })
    }

    fn place(&mut self) -> Read<Place> {
        let local = Local(self.u32()?);
        let n = self.len()?;
        let mut projections = Vec::new();
        for _ in 0..n {
            let (tag, err) = self.tag("no such projection")?;
            projections.push(match tag {
                0 => Projection::Field(self.u32()?),
                1 => Projection::Deref,
                2 => Projection::Index(Local(self.u32()?)),
                3 => Projection::ConstantIndex(self.u32()?),
                4 => Projection::Downcast(self.u32()?),
                _ => return Err(err),
            });
        }
        Ok(Place { local, projections })
    }

    fn scalar(&mut self) -> Read<Scalar> {
        let (tag, err) = self.tag("no such scalar kind")?;
        Ok(match tag {
            0 => Scalar::Uninit,
            1 => Scalar::Bool(self.bool()?),
            2 => {
                let t = self.int()?;
                let at = self.at;
                let v = self.i128()?;
                Scalar::int(t, v).ok_or(FileError::Malformed { at, what: "an integer outside its type" })?
            }
            3 => Scalar::F32(f32::from_bits(self.u32()?)),
            4 => Scalar::F64(f64::from_bits(self.u64()?)),
            5 => Scalar::Ref(self.u32()?),
            _ => return Err(err),
        })
    }

    fn operand(&mut self) -> Read<Operand> {
        let (tag, err) = self.tag("no such operand kind")?;
        Ok(match tag {
            0 => Operand::Copy(self.place()?),
            1 => Operand::Move(self.place()?),
            2 => {
                let ty = TypeId(self.u32()?);
                let n = self.len()?;
                let mut slots = Vec::new();
                for _ in 0..n {
                    slots.push(self.scalar()?);
                }
                Operand::Const(Const { ty, slots })
            }
            _ => return Err(err),
        })
    }

    fn operands(&mut self) -> Read<Vec<Operand>> {
        let n = self.len()?;
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(self.operand()?);
        }
        Ok(out)
    }

    fn value(&mut self) -> Read<Value> {
        let (tag, err) = self.tag("no such value kind")?;
        Ok(match tag {
            0 => Value::Use(self.operand()?),
            1 => Value::Binary(self.binop()?, self.operand()?, self.operand()?),
            2 => Value::Checked(self.binop()?, self.operand()?, self.operand()?),
            3 => {
                let (op, err) = self.tag("no such unary operation")?;
                let op = match op {
                    0 => UnOp::Not,
                    1 => UnOp::Neg,
                    _ => return Err(err),
                };
                Value::Unary(op, self.operand()?)
            }
            4 => Value::Cast(self.operand()?, TypeId(self.u32()?)),
            5 => {
                let ty = TypeId(self.u32()?);
                let (has, err) = self.tag("an option that is neither 0 nor 1")?;
                let variant = match has {
                    0 => None,
                    1 => Some(self.u32()?),
                    _ => return Err(err),
                };
                Value::Aggregate(ty, variant, self.operands()?)
            }
            6 => Value::Discriminant(self.place()?),
            7 => Value::Ref(self.place()?, self.bool()?),
            _ => return Err(err),
        })
    }

    fn check(&mut self) -> Read<Check> {
        let (tag, err) = self.tag("no such check")?;
        Ok(match tag {
            0 => Check::Overflow(self.binop()?),
            1 => Check::NegOverflow,
            2 => Check::DivisionByZero,
            3 => Check::RemainderByZero,
            4 => Check::Bounds,
            _ => return Err(err),
        })
    }

    fn core(&mut self) -> Read<CoreOp> {
        let (tag, err) = self.tag("no such core operation")?;
        Ok(match tag {
            0 => CoreOp::SaturatingAdd(self.int()?),
            1 => CoreOp::SaturatingSub(self.int()?),
            2 => CoreOp::SaturatingMul(self.int()?),
            3 => CoreOp::WrappingAdd(self.int()?),
            4 => CoreOp::WrappingSub(self.int()?),
            5 => CoreOp::WrappingMul(self.int()?),
            6 => CoreOp::CheckedShl(self.int()?),
            7 => CoreOp::CheckedShr(self.int()?),
            8 => CoreOp::Min(self.int()?),
            9 => CoreOp::Max(self.int()?),
            10 => CoreOp::DurationFromMillis,
            11 => CoreOp::DurationFromSecs,
            12 => CoreOp::DurationSaturatingSub,
            13 => CoreOp::DurationMin,
            14 => CoreOp::DurationMax,
            15 => CoreOp::DurationCompare(self.binop()?),
            16 => CoreOp::OptionGetOrInsert,
            17 => CoreOp::OptionIsNone,
            18 => CoreOp::OptionIsSome,
            19 => CoreOp::Default,
            _ => return Err(err),
        })
    }

    fn terminator(&mut self) -> Read<Terminator> {
        let (tag, err) = self.tag("no such terminator")?;
        Ok(match tag {
            0 => Terminator::Goto(BlockId(self.u32()?)),
            1 => {
                let value = self.operand()?;
                let n = self.len()?;
                let mut targets = Vec::new();
                for _ in 0..n {
                    targets.push((self.i128()?, BlockId(self.u32()?)));
                }
                Terminator::SwitchInt { value, targets, otherwise: BlockId(self.u32()?), at: self.span()? }
            }
            2 => Terminator::Return { at: self.span()? },
            3 => {
                let (kind, err) = self.tag("no such callee")?;
                let callee = match kind {
                    0 => Callee::Function(FunctionId(self.u32()?)),
                    1 => Callee::Core(self.core()?),
                    _ => return Err(err),
                };
                Terminator::Call {
                    callee,
                    args: self.operands()?,
                    dest: self.place()?,
                    next: BlockId(self.u32()?),
                    at: self.span()?,
                }
            }
            4 => Terminator::Assert {
                cond: self.operand()?,
                expected: self.bool()?,
                check: self.check()?,
                next: BlockId(self.u32()?),
                at: self.span()?,
            },
            5 => Terminator::Unreachable { at: self.span()? },
            _ => return Err(err),
        })
    }

    fn function(&mut self) -> Read<Function> {
        let name = self.str()?;
        let params = self.u32()?;
        let n = self.len()?;
        let mut locals = Vec::new();
        for _ in 0..n {
            let ty = TypeId(self.u32()?);
            let (has, err) = self.tag("an option that is neither 0 nor 1")?;
            let name = match has {
                0 => None,
                1 => Some(self.str()?),
                _ => return Err(err),
            };
            locals.push(LocalDecl { ty, name });
        }
        let n = self.len()?;
        let mut blocks = Vec::new();
        for _ in 0..n {
            let m = self.len()?;
            let mut statements = Vec::new();
            for _ in 0..m {
                statements.push(Statement { place: self.place()?, value: self.value()?, at: self.span()? });
            }
            blocks.push(Block { statements, terminator: self.terminator()? });
        }
        Ok(Function { name, params, locals, blocks })
    }
}

/// Reads a program file: its header, then its program, refusing a format or a language this interpreter does not
/// run before reading anything else, and a malformed file at the byte where it stops making sense. Whether the
/// program itself fits its types is the loader's to say ([`crate::Machine::load`]).
pub fn read(bytes: &[u8]) -> Result<(Header, Program), FileError> {
    let mut r = Reader { bytes, at: 0 };
    if bytes.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return Err(FileError::NotAProgram);
    }
    r.at = MAGIC.len();
    let format = r.u16()?;
    if format != FORMAT {
        return Err(FileError::Format { found: format });
    }
    let language = r.u16()?;
    if !LANGUAGES.contains(&language) {
        return Err(FileError::Language { found: language });
    }
    let header = Header { language, nightly: r.str()?, revision: r.str()? };
    let mut program = Program::default();
    for _ in 0..r.len()? {
        program.files.push(r.str()?);
    }
    for _ in 0..r.len()? {
        program.types.push(r.ty()?);
    }
    for _ in 0..r.len()? {
        program.functions.push(r.function()?);
    }
    if r.at != bytes.len() {
        return Err(r.bad("bytes past the program's end"));
    }
    Ok((header, program))
}
