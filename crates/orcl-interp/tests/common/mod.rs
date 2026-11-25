//! Programs built by hand, as the front end would lower them: a few helpers so a test reads as MIR does.
#![allow(dead_code)]
use orcl_interp::*;

/// A program under construction, its types interned so one type has one index.
#[derive(Default)]
pub struct Build {
    pub program: Program,
}

impl Build {
    pub fn ty(&mut self, ty: Type) -> TypeId {
        if let Some(i) = self.program.types.iter().position(|t| *t == ty) {
            return TypeId(i as u32);
        }
        self.program.types.push(ty);
        TypeId(self.program.types.len() as u32 - 1)
    }

    pub fn int(&mut self, t: Int) -> TypeId {
        self.ty(Type::Int(t))
    }

    pub fn bool(&mut self) -> TypeId {
        self.ty(Type::Bool)
    }

    pub fn unit(&mut self) -> TypeId {
        self.ty(Type::Struct { name: "()".into(), fields: vec![] })
    }

    pub fn tuple(&mut self, parts: &[TypeId]) -> TypeId {
        let fields = parts.iter().enumerate().map(|(i, t)| Field { name: i.to_string(), ty: *t }).collect();
        self.ty(Type::Struct { name: "(..)".into(), fields })
    }

    pub fn reference(&mut self, to: TypeId, mutable: bool) -> TypeId {
        self.ty(Type::Ref { to, mutable })
    }

    pub fn option(&mut self, of: TypeId) -> TypeId {
        self.ty(Type::Enum {
            name: "Option".into(),
            discriminant: Int::Isize,
            variants: vec![
                Variant { name: "None".into(), discriminant: 0, fields: vec![] },
                Variant { name: "Some".into(), discriminant: 1, fields: vec![Field { name: "0".into(), ty: of }] },
            ],
        })
    }

    pub fn function(&mut self, f: Function) -> FunctionId {
        self.program.functions.push(f);
        FunctionId(self.program.functions.len() as u32 - 1)
    }
}

pub fn locals(types: &[TypeId]) -> Vec<LocalDecl> {
    types.iter().map(|t| LocalDecl { ty: *t, name: None }).collect()
}

pub fn at(line: u32) -> Span {
    Span { file: 0, line, column: 1 }
}

pub fn place(local: u32) -> Place {
    Place::local(local)
}

pub fn proj(local: u32, projections: Vec<Projection>) -> Place {
    Place { local: Local(local), projections }
}

pub fn copy(local: u32) -> Operand {
    Operand::Copy(place(local))
}

pub fn copy_at(p: Place) -> Operand {
    Operand::Copy(p)
}

pub fn konst(ty: TypeId, slots: Vec<Scalar>) -> Operand {
    Operand::Const(Const { ty, slots })
}

pub fn assign(p: Place, value: Value, line: u32) -> Statement {
    Statement { place: p, value, at: at(line) }
}

pub fn block(statements: Vec<Statement>, terminator: Terminator) -> Block {
    Block { statements, terminator }
}

pub fn ret() -> Terminator {
    Terminator::Return { at: at(99) }
}

pub fn int(t: Int, v: i128) -> Scalar {
    Scalar::int(t, v).expect("a value of its type")
}

/// The eight integer types of ORCL, and the compiler's two.
pub const INTS: [Int; 10] =
    [Int::I8, Int::I16, Int::I32, Int::I64, Int::Isize, Int::U8, Int::U16, Int::U32, Int::U64, Int::Usize];

/// A small deterministic generator, so every run draws the same values.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Values of `t` worth trying: its bounds and their neighbours, small values, and `extra` drawn at random.
pub fn samples(t: Int, rng: &mut Rng, extra: usize) -> Vec<i128> {
    let (lo, hi) = (t.min(), t.max());
    let mut v = vec![lo, lo + 1, lo + 2, hi, hi - 1, hi - 2, 0, 1, 2, 3, 7, 8, 63, 64, 100];
    if t.signed() {
        v.extend([-1, -2, -3, -7, -8, -64, -100]);
    }
    for _ in 0..extra {
        v.push(t.wrap(rng.next() as i128 | ((rng.next() as i128) << 64)));
    }
    v.retain(|x| t.holds(*x));
    v.sort();
    v.dedup();
    v
}
