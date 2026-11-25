//! The loader's half: every index and every instruction's types checked, memory laid out, the call graph walked.
//! A program that passes runs with no allocation and no type it was not loaded with; one that does not is refused
//! by name, before anything runs.
use core::fmt;

use crate::core_ops::same;
use crate::ops::Scalar;
use crate::program::{
    BinOp, Block, Callee, Function, Int, Operand, Place, Program, Projection, Terminator, Type, TypeId, UnOp,
    Value,
};

/// Why a program is not loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// A type the machine cannot lay out: an index outside the program, a type that contains itself with no
    /// reference between, an enum's discriminants that clash or do not fit, a size past the machine's reach.
    Type { ty: u32, reason: &'static str },
    /// A function whose locals or blocks are not as MIR has them.
    Function { function: String, reason: &'static str },
    /// An instruction whose indices or types do not fit; `statement` is `None` for the block's terminator.
    Instruction { function: String, block: u32, statement: Option<u32>, reason: &'static str },
    /// A function that reaches itself through its calls, which has no deepest path to size the stack by
    /// (`docs/questions.md` Q230, taken meanwhile): the functions of the cycle, by name, the first repeated last.
    Recursion { cycle: Vec<String> },
    /// A host region asked for of a type the program does not have, or memory past the machine's reach.
    Region { index: usize, reason: &'static str },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Type { ty, reason } => write!(f, "type {ty}: {reason}"),
            LoadError::Function { function, reason } => write!(f, "`{function}`: {reason}"),
            LoadError::Instruction { function, block, statement: Some(s), reason } => {
                write!(f, "`{function}`, block {block}, statement {s}: {reason}")
            }
            LoadError::Instruction { function, block, statement: None, reason } => {
                write!(f, "`{function}`, block {block}, its terminator: {reason}")
            }
            LoadError::Recursion { cycle } => write!(f, "a function reaches itself: {}", cycle.join(" -> ")),
            LoadError::Region { index, reason } => write!(f, "region {index}: {reason}"),
        }
    }
}

impl core::error::Error for LoadError {}

/// Where each local of a function sits in its frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameLayout {
    pub offsets: Vec<u32>,
    pub size: u32,
}

/// The program's memory, laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub sizes: Sizes,
    pub frames: Vec<FrameLayout>,
    /// For each function, the slots of the deepest path of calls from it, its own frame included.
    pub depth_slots: Vec<u32>,
    /// For each function, the frames on that path.
    pub depth_frames: Vec<u32>,
    /// The largest type, in slots: the scratch a value is built in before it is written.
    pub largest: u32,
}

const NESTING: u32 = 256;

fn add(a: u32, b: u32) -> Option<u32> {
    a.checked_add(b)
}

struct Sizer<'a> {
    types: &'a [Type],
    state: Vec<u8>,
    size: Vec<u32>,
    /// How deep each type nests: 1 for a scalar or a reference, one more than its deepest part otherwise. Kept
    /// whatever order the types come in, so a nesting past [`NESTING`] is refused however a file lists it, and every
    /// walk of a type's parts, a constant's check among them, is as deep as that at most.
    depth: Vec<u32>,
    fields: Vec<Vec<u32>>,
    variants: Vec<Vec<Vec<u32>>>,
}

impl Sizer<'_> {
    fn size_of(&mut self, ty: TypeId, nesting: u32) -> Result<u32, LoadError> {
        let index = ty.0 as usize;
        let err = |reason| LoadError::Type { ty: ty.0, reason };
        match self.state.get(index) {
            None => return Err(err("no such type")),
            Some(2) => return self.size.get(index).copied().ok_or(err("no such type")),
            Some(1) => return Err(err("it contains itself with no reference between")),
            Some(_) => {}
        }
        if nesting > NESTING {
            return Err(err("nested too deep"));
        }
        if let Some(s) = self.state.get_mut(index) {
            *s = 1;
        }
        let types = self.types;
        let mut deepest = 0u32;
        let size = match types.get(index) {
            None => return Err(err("no such type")),
            Some(Type::Bool | Type::Int(_) | Type::F32 | Type::F64) => 1,
            Some(Type::Duration) => 2,
            Some(Type::Ref { to, .. }) => {
                if types.get(to.0 as usize).is_none() {
                    return Err(err("a reference to no such type"));
                }
                1
            }
            Some(Type::Struct { fields, .. }) => {
                let mut offsets = Vec::with_capacity(fields.len());
                let mut at = 0u32;
                for field in fields {
                    offsets.push(at);
                    at = add(at, self.size_of(field.ty, nesting.saturating_add(1))?).ok_or(err("too large"))?;
                    deepest = deepest.max(self.depth.get(field.ty.0 as usize).copied().unwrap_or(u32::MAX));
                }
                if let Some(slot) = self.fields.get_mut(index) {
                    *slot = offsets;
                }
                at
            }
            Some(Type::Enum { discriminant, variants, .. }) => {
                let mut largest = 0u32;
                let mut all = Vec::with_capacity(variants.len());
                // sorted, so two variants with one discriminant are found side by side, however many there are
                let mut discriminants: Vec<i128> = variants.iter().map(|v| v.discriminant).collect();
                discriminants.sort_unstable();
                if discriminants.windows(2).any(|w| w.first() == w.get(1)) {
                    return Err(err("two variants with one discriminant"));
                }
                for variant in variants {
                    if !discriminant.holds(variant.discriminant) {
                        return Err(err("a discriminant outside its type"));
                    }
                    let mut offsets = Vec::with_capacity(variant.fields.len());
                    let mut at = 1u32;
                    for field in &variant.fields {
                        offsets.push(at);
                        at = add(at, self.size_of(field.ty, nesting.saturating_add(1))?).ok_or(err("too large"))?;
                        deepest = deepest.max(self.depth.get(field.ty.0 as usize).copied().unwrap_or(u32::MAX));
                    }
                    largest = largest.max(at);
                    all.push(offsets);
                }
                if let Some(slot) = self.variants.get_mut(index) {
                    *slot = all;
                }
                largest.max(1)
            }
            Some(Type::Array { element, len }) => {
                let size = self.size_of(*element, nesting.saturating_add(1))?.checked_mul(*len).ok_or(err("too large"))?;
                deepest = self.depth.get(element.0 as usize).copied().unwrap_or(u32::MAX);
                size
            }
        };
        let depth = deepest.saturating_add(1);
        if depth > NESTING {
            return Err(err("nested too deep"));
        }
        if let Some(d) = self.depth.get_mut(index) {
            *d = depth;
        }
        if let Some(s) = self.size.get_mut(index) {
            *s = size;
        }
        if let Some(s) = self.state.get_mut(index) {
            *s = 2;
        }
        Ok(size)
    }
}

/// What a place is, while its projections are walked: a type, or one variant of an enum.
#[derive(Clone, Copy)]
enum At {
    Type(TypeId),
    Variant(TypeId, u32),
}

struct Checker<'a> {
    program: &'a Program,
    function: &'a Function,
    block: u32,
    statement: Option<u32>,
}

impl Checker<'_> {
    fn ty(&self, ty: TypeId) -> Option<&Type> {
        self.program.types.get(ty.0 as usize)
    }

    fn fail(&self, reason: &'static str) -> LoadError {
        LoadError::Instruction {
            function: self.function.name.clone(),
            block: self.block,
            statement: self.statement,
            reason,
        }
    }

    fn local(&self, local: u32) -> Result<TypeId, LoadError> {
        self.function.locals.get(local as usize).map(|l| l.ty).ok_or_else(|| self.fail("no such local"))
    }

    fn place(&self, place: &Place) -> Result<TypeId, LoadError> {
        let mut at = At::Type(self.local(place.local.0)?);
        for projection in &place.projections {
            at = match (at, projection) {
                (At::Type(ty), Projection::Field(i)) => match self.ty(ty) {
                    Some(Type::Struct { fields, .. }) => {
                        At::Type(fields.get(*i as usize).map(|f| f.ty).ok_or_else(|| self.fail("no such field"))?)
                    }
                    _ => return Err(self.fail("a field of what is no structure")),
                },
                (At::Variant(ty, v), Projection::Field(i)) => match self.ty(ty) {
                    Some(Type::Enum { variants, .. }) => At::Type(
                        variants
                            .get(v as usize)
                            .and_then(|variant| variant.fields.get(*i as usize))
                            .map(|f| f.ty)
                            .ok_or_else(|| self.fail("no such field of the variant"))?,
                    ),
                    _ => return Err(self.fail("a variant of what is no enum")),
                },
                (At::Type(ty), Projection::Downcast(v)) => match self.ty(ty) {
                    Some(Type::Enum { variants, .. }) if (*v as usize) < variants.len() => At::Variant(ty, *v),
                    _ => return Err(self.fail("no such variant")),
                },
                (At::Type(ty), Projection::Deref) => match self.ty(ty) {
                    Some(Type::Ref { to, .. }) => At::Type(*to),
                    _ => return Err(self.fail("a dereference of what is no reference")),
                },
                (At::Type(ty), Projection::Index(local)) => {
                    if !matches!(self.ty(self.local(local.0)?), Some(Type::Int(Int::Usize))) {
                        return Err(self.fail("an index that is no usize"));
                    }
                    match self.ty(ty) {
                        Some(Type::Array { element, .. }) => At::Type(*element),
                        _ => return Err(self.fail("an index into what is no array")),
                    }
                }
                (At::Type(ty), Projection::ConstantIndex(i)) => match self.ty(ty) {
                    Some(Type::Array { element, len }) if i < len => At::Type(*element),
                    _ => return Err(self.fail("a constant index past the array")),
                },
                (At::Variant(..), _) => return Err(self.fail("a variant read but through a field")),
            };
        }
        match at {
            At::Type(ty) => Ok(ty),
            At::Variant(..) => Err(self.fail("a place that ends in a variant")),
        }
    }

    fn operand(&self, operand: &Operand, layout: &Sizes) -> Result<TypeId, LoadError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.place(place),
            Operand::Const(c) => {
                if fits(&self.program.types, layout, c.ty, &c.slots) {
                    Ok(c.ty)
                } else {
                    Err(self.fail("a constant whose slots are not its type's"))
                }
            }
        }
    }

    fn same(&self, a: TypeId, b: TypeId) -> bool {
        same(&self.program.types, a, b)
    }

    fn value(&self, value: &Value, dest: TypeId, layout: &Sizes) -> Result<(), LoadError> {
        let ok = |good: bool, reason| if good { Ok(()) } else { Err(self.fail(reason)) };
        match value {
            Value::Use(op) => ok(self.same(self.operand(op, layout)?, dest), "a value of another type than its place"),
            Value::Binary(op, a, b) => {
                let (ta, tb) = (self.operand(a, layout)?, self.operand(b, layout)?);
                let (xa, xb) = (self.ty(ta), self.ty(tb));
                let result = match op {
                    BinOp::Shl | BinOp::Shr => {
                        ok(matches!((xa, xb), (Some(Type::Int(_)), Some(Type::Int(_)))), "a shift of what is no integer")?;
                        ta
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        ok(
                            self.same(ta, tb) && matches!(xa, Some(Type::Bool | Type::Int(_) | Type::F32 | Type::F64)),
                            "a comparison of values that do not compare",
                        )?;
                        return ok(matches!(self.ty(dest), Some(Type::Bool)), "a comparison into what is no bool");
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                        ok(self.same(ta, tb) && matches!(xa, Some(Type::Bool | Type::Int(_))), "bits of what has none")?;
                        ta
                    }
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                        ok(
                            self.same(ta, tb) && matches!(xa, Some(Type::Int(_) | Type::F32 | Type::F64)),
                            "arithmetic on what is no number",
                        )?;
                        ta
                    }
                };
                ok(self.same(result, dest), "a result of another type than its place")
            }
            Value::Checked(op, a, b) => {
                let (ta, tb) = (self.operand(a, layout)?, self.operand(b, layout)?);
                ok(
                    matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul)
                        && self.same(ta, tb)
                        && matches!(self.ty(ta), Some(Type::Int(_))),
                    "a checked operation that MIR has no checked form of",
                )?;
                let pair = match self.ty(dest) {
                    Some(Type::Struct { fields, .. }) => match fields.as_slice() {
                        [r, o] => self.same(r.ty, ta) && matches!(self.ty(o.ty), Some(Type::Bool)),
                        _ => false,
                    },
                    _ => false,
                };
                ok(pair, "a checked result into what is no (value, bool)")
            }
            Value::Unary(op, a) => {
                let ta = self.operand(a, layout)?;
                let good = match (op, self.ty(ta)) {
                    (UnOp::Not, Some(Type::Bool | Type::Int(_))) => true,
                    (UnOp::Neg, Some(Type::Int(t))) => t.signed(),
                    (UnOp::Neg, Some(Type::F32 | Type::F64)) => true,
                    _ => false,
                };
                ok(good && self.same(ta, dest), "a unary operation on what it does not take")
            }
            Value::Cast(a, to) => {
                let ta = self.operand(a, layout)?;
                let good = matches!(
                    (self.ty(ta), self.ty(*to)),
                    (Some(Type::Bool), Some(Type::Bool | Type::Int(_)))
                        | (Some(Type::Int(_) | Type::F32 | Type::F64), Some(Type::Int(_) | Type::F32 | Type::F64))
                );
                ok(good && self.same(*to, dest), "a cast `as` does not make")
            }
            Value::Aggregate(ty, variant, ops) => {
                // the parts' types, read where they stand: an array's element is one type for every part, so
                // nothing as long as the array is built to check it
                let fields: Option<&[crate::program::Field]> = match (self.ty(*ty), variant) {
                    (Some(Type::Struct { fields, .. }), None) => Some(fields),
                    (Some(Type::Array { .. }), None) => None,
                    (Some(Type::Enum { variants, .. }), Some(v)) => match variants.get(*v as usize) {
                        Some(variant) => Some(&variant.fields),
                        None => return Err(self.fail("no such variant")),
                    },
                    _ => return Err(self.fail("an aggregate of what is built otherwise")),
                };
                let (count, element) = match (fields, self.ty(*ty)) {
                    (Some(fields), _) => (fields.len(), None),
                    (None, Some(Type::Array { element, len })) => (*len as usize, Some(*element)),
                    _ => return Err(self.fail("an aggregate of what is built otherwise")),
                };
                ok(count == ops.len(), "an aggregate with too many or too few parts")?;
                for (i, op) in ops.iter().enumerate() {
                    let want = match (fields, element) {
                        (Some(fields), _) => fields.get(i).map(|f| f.ty),
                        (None, element) => element,
                    };
                    let part = self.operand(op, layout)?;
                    ok(want.is_some_and(|w| self.same(w, part)), "a part of another type than its place")?;
                }
                ok(*ty == dest, "an aggregate of another type than its place")
            }
            Value::Discriminant(place) => match self.ty(self.place(place)?) {
                Some(Type::Enum { discriminant, .. }) => {
                    ok(self.ty(dest) == Some(&Type::Int(*discriminant)), "a discriminant into another type")
                }
                _ => Err(self.fail("the discriminant of what is no enum")),
            },
            Value::Ref(place, mutable) => {
                let to = self.place(place)?;
                match self.ty(dest) {
                    Some(Type::Ref { to: t, mutable: m }) if m == mutable && self.same(*t, to) => Ok(()),
                    _ => Err(self.fail("a reference into another type")),
                }
            }
        }
    }

    fn terminator(&self, block: &Block, layout: &Sizes) -> Result<(), LoadError> {
        let blocks = self.function.blocks.len();
        let target = |b: u32| if (b as usize) < blocks { Ok(()) } else { Err(self.fail("no such block")) };
        match &block.terminator {
            Terminator::Goto(b) => target(b.0),
            Terminator::SwitchInt { value, targets, otherwise, .. } => {
                let ty = self.operand(value, layout)?;
                let fits_value = |v: i128| match self.ty(ty) {
                    Some(Type::Bool) => v == 0 || v == 1,
                    Some(Type::Int(t)) => t.holds(v),
                    _ => false,
                };
                for (v, b) in targets {
                    if !fits_value(*v) {
                        return Err(self.fail("a switch's value outside its operand's type"));
                    }
                    target(b.0)?;
                }
                target(otherwise.0)
            }
            Terminator::Return { .. } | Terminator::Unreachable { .. } => Ok(()),
            Terminator::Assert { cond, next, .. } => {
                if !matches!(self.ty(self.operand(cond, layout)?), Some(Type::Bool)) {
                    return Err(self.fail("an assertion of what is no bool"));
                }
                target(next.0)
            }
            Terminator::Call { callee, args, dest, next, .. } => {
                target(next.0)?;
                let dest = self.place(dest)?;
                let mut types = Vec::with_capacity(args.len());
                for arg in args {
                    types.push(self.operand(arg, layout)?);
                }
                match callee {
                    Callee::Function(g) => {
                        let g = self.program.functions.get(g.0 as usize).ok_or_else(|| self.fail("no such function"))?;
                        if types.len() != g.params as usize {
                            return Err(self.fail("a call with too many or too few arguments"));
                        }
                        for (i, ty) in types.iter().enumerate() {
                            let param = g.locals.get(i.saturating_add(1)).map(|l| l.ty);
                            if !param.is_some_and(|p| self.same(p, *ty)) {
                                return Err(self.fail("an argument of another type than its parameter"));
                            }
                        }
                        match g.locals.first() {
                            Some(ret) if self.same(ret.ty, dest) => Ok(()),
                            _ => Err(self.fail("a call whose result is of another type than its place")),
                        }
                    }
                    Callee::Core(op) => {
                        if op.accepts(&self.program.types, &types, dest) {
                            Ok(())
                        } else {
                            Err(self.fail("a core operation called with types it does not take"))
                        }
                    }
                }
            }
        }
    }
}

/// Type sizes and offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    /// Slots of each type.
    pub size: Vec<u32>,
    /// Each structure's fields' offsets; empty for any other type.
    pub fields: Vec<Vec<u32>>,
    /// Each enum's variants' fields' offsets from the enum's first slot, its discriminant; empty otherwise.
    pub variants: Vec<Vec<Vec<u32>>>,
}

/// Whether `slots` are a value of `ty`: every scalar of the type's kind and range, a `Duration`'s nanoseconds
/// below a second, an enum's discriminant one of its variants' and the slots past that variant's fields unwritten.
/// A constant holds no reference.
pub(crate) fn fits(types: &[Type], sizes: &Sizes, ty: TypeId, slots: &[Scalar]) -> bool {
    let size = sizes.size.get(ty.0 as usize).copied().unwrap_or(u32::MAX);
    if slots.len() != size as usize {
        return false;
    }
    match (types.get(ty.0 as usize), slots) {
        (Some(Type::Bool), [Scalar::Bool(_)]) | (Some(Type::F32), [Scalar::F32(_)]) | (Some(Type::F64), [Scalar::F64(_)]) => {
            true
        }
        (Some(Type::Int(t)), [Scalar::Int(u, v)]) => t == u && t.holds(*v),
        (Some(Type::Duration), [Scalar::Int(Int::U64, _), Scalar::Int(Int::U32, n)]) => *n < 1_000_000_000,
        (Some(Type::Struct { fields, .. }), _) => {
            let offsets = sizes.fields.get(ty.0 as usize);
            fields.iter().enumerate().all(|(i, field)| {
                let at = offsets.and_then(|o| o.get(i)).copied().unwrap_or(u32::MAX) as usize;
                let len = sizes.size.get(field.ty.0 as usize).copied().unwrap_or(u32::MAX) as usize;
                slots.get(at..at.saturating_add(len)).is_some_and(|part| fits(types, sizes, field.ty, part))
            })
        }
        (Some(Type::Array { element, len }), _) => {
            let each = sizes.size.get(element.0 as usize).copied().unwrap_or(u32::MAX) as usize;
            if each == 0 {
                // elements of no slot are all alike: one stands for every one of them, however long the array
                return *len == 0 || fits(types, sizes, *element, &[]);
            }
            (0..*len as usize).all(|i| {
                let at = i.saturating_mul(each);
                slots.get(at..at.saturating_add(each)).is_some_and(|part| fits(types, sizes, *element, part))
            })
        }
        (Some(Type::Enum { discriminant, variants, .. }), [Scalar::Int(u, d), rest @ ..]) => {
            let Some(v) = variants.iter().position(|v| v.discriminant == *d) else { return false };
            let (Some(variant), Some(offsets)) =
                (variants.get(v), sizes.variants.get(ty.0 as usize).and_then(|all| all.get(v)))
            else {
                return false;
            };
            let mut end = 1usize;
            let fields_fit = variant.fields.iter().zip(offsets).all(|(field, at)| {
                let at = *at as usize;
                let len = sizes.size.get(field.ty.0 as usize).copied().unwrap_or(u32::MAX) as usize;
                end = end.max(at.saturating_add(len));
                slots.get(at..at.saturating_add(len)).is_some_and(|part| fits(types, sizes, field.ty, part))
            });
            u == discriminant && fields_fit && rest.iter().skip(end.saturating_sub(1)).all(|s| *s == Scalar::Uninit)
        }
        _ => false,
    }
}

/// Lays out and checks a program, or says why it cannot be loaded.
pub(crate) fn lay_out(program: &Program) -> Result<Layout, LoadError> {
    let n = program.types.len();
    let mut sizer = Sizer {
        types: &program.types,
        state: vec![0; n],
        size: vec![0; n],
        depth: vec![0; n],
        fields: vec![Vec::new(); n],
        variants: vec![Vec::new(); n],
    };
    for i in 0..n {
        sizer.size_of(TypeId(u32::try_from(i).map_err(|_| LoadError::Type { ty: u32::MAX, reason: "too many types" })?), 0)?;
    }
    let sizes = Sizes { size: sizer.size, fields: sizer.fields, variants: sizer.variants };
    let largest = sizes.size.iter().copied().max().unwrap_or(0);

    let mut frames = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let fail = |reason| LoadError::Function { function: function.name.clone(), reason };
        if function.locals.is_empty() || function.params as usize >= function.locals.len() {
            return Err(fail("no return value, or more parameters than locals"));
        }
        if function.blocks.is_empty() {
            return Err(fail("no block"));
        }
        let mut offsets = Vec::with_capacity(function.locals.len());
        let mut at = 0u32;
        for local in &function.locals {
            let size = sizes.size.get(local.ty.0 as usize).copied().ok_or(fail("a local of no such type"))?;
            offsets.push(at);
            at = add(at, size).ok_or(fail("a frame too large"))?;
        }
        frames.push(FrameLayout { offsets, size: at });
        for (b, block) in function.blocks.iter().enumerate() {
            let block_index = u32::try_from(b).map_err(|_| fail("too many blocks"))?;
            for (s, statement) in block.statements.iter().enumerate() {
                let checker = Checker {
                    program,
                    function,
                    block: block_index,
                    statement: Some(u32::try_from(s).map_err(|_| fail("too many statements"))?),
                };
                let dest = checker.place(&statement.place)?;
                checker.value(&statement.value, dest, &sizes)?;
            }
            Checker { program, function, block: block_index, statement: None }.terminator(block, &sizes)?;
        }
    }

    let (depth_slots, depth_frames) = depths(program, &frames)?;
    Ok(Layout {
        sizes,
        frames,
        depth_slots,
        depth_frames,
        largest,
    })
}

fn callees(function: &Function) -> impl Iterator<Item = u32> + '_ {
    function.blocks.iter().filter_map(|block| match &block.terminator {
        Terminator::Call { callee: Callee::Function(g), .. } => Some(g.0),
        _ => None,
    })
}

/// The deepest path of calls from every function, in slots and in frames; a cycle refused, by its functions'
/// names.
fn depths(program: &Program, frames: &[FrameLayout]) -> Result<(Vec<u32>, Vec<u32>), LoadError> {
    let n = program.functions.len();
    let mut state = vec![0u8; n];
    let mut slots = vec![0u32; n];
    let mut count = vec![0u32; n];
    let name = |f: u32| program.functions.get(f as usize).map(|g| g.name.clone()).unwrap_or_default();
    for root in 0..n {
        // an explicit stack: (function, its callees still to visit), so a deep call graph does not exhaust ours
        let root = u32::try_from(root).map_err(|_| LoadError::Region { index: 0, reason: "too many functions" })?;
        if state.get(root as usize) == Some(&2) {
            continue;
        }
        let mut path: Vec<(u32, Vec<u32>)> = Vec::new();
        let first = program.functions.get(root as usize).map(|f| callees(f).collect()).unwrap_or_default();
        path.push((root, first));
        if let Some(s) = state.get_mut(root as usize) {
            *s = 1;
        }
        while let Some((f, pending)) = path.last_mut() {
            let f = *f;
            if let Some(g) = pending.pop() {
                match state.get(g as usize) {
                    Some(1) => {
                        let start = path.iter().position(|(h, _)| *h == g).unwrap_or(0);
                        let mut cycle: Vec<String> = path.iter().skip(start).map(|(h, _)| name(*h)).collect();
                        cycle.push(name(g));
                        return Err(LoadError::Recursion { cycle });
                    }
                    Some(0) => {
                        if let Some(s) = state.get_mut(g as usize) {
                            *s = 1;
                        }
                        let next = program.functions.get(g as usize).map(|h| callees(h).collect()).unwrap_or_default();
                        path.push((g, next));
                    }
                    _ => {}
                }
            } else {
                let function = program.functions.get(f as usize);
                let (mut deepest_slots, mut deepest_frames) = (0u32, 0u32);
                for g in function.into_iter().flat_map(callees) {
                    deepest_slots = deepest_slots.max(slots.get(g as usize).copied().unwrap_or(0));
                    deepest_frames = deepest_frames.max(count.get(g as usize).copied().unwrap_or(0));
                }
                let own = frames.get(f as usize).map(|fr| fr.size).unwrap_or(0);
                let too_deep = LoadError::Region { index: 0, reason: "a stack past the machine's reach" };
                if let Some(s) = slots.get_mut(f as usize) {
                    *s = add(own, deepest_slots).ok_or(too_deep.clone())?;
                }
                if let Some(c) = count.get_mut(f as usize) {
                    *c = add(1, deepest_frames).ok_or(too_deep)?;
                }
                if let Some(s) = state.get_mut(f as usize) {
                    *s = 2;
                }
                path.pop();
            }
        }
    }
    Ok((slots, count))
}
