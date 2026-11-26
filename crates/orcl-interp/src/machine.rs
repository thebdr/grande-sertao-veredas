//! The machine: a loaded program, its memory and its stack, run one instruction at a time. Its whole state is data,
//! so a run stopped at an instruction boundary goes on from there; nothing is allocated once it is loaded.
use core::fmt;
use core::time::Duration;

use crate::core_ops::CoreOp;
use crate::layout::{FrameLayout, Layout, LoadError, fits, lay_out};
use crate::ops::{self, Fail, Scalar};
use crate::program::{
    BinOp, BlockId, Callee, Check, Function, FunctionId, Int, Operand, Place, Program, Projection, Span, Terminator,
    Type, TypeId, Value,
};

/// A region of memory the host asked for at load, of one type: a program's state, its inputs, its outputs, a
/// scan's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegionId(pub u32);

/// What a host lets a machine lay out, in slots (`docs/questions.md` Q230: the target's settings give both).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The stack's size: the frames of the calls in progress; as many frames as it has slots, at most.
    pub stack: u32,
    /// The most slots the machine lays out in all: its regions, its stack, and the room for the largest value it
    /// builds and for the largest result. Checked before anything is allocated.
    pub memory: u32,
}

/// An argument the host passes to an entry: a reference to one of its regions, or a value's slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg<'a> {
    Ref(RegionId),
    Value(&'a [Scalar]),
}

/// What one step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// No entry is running.
    Idle,
    /// It ran one instruction, and the entry goes on.
    Ran,
    /// It ran the entry's last instruction: its result is [`Machine::result`].
    Returned,
}

/// A program's failure while it runs: where, and what. What Rust would panic on, never a panic here (ORCL's O11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramError {
    pub function: FunctionId,
    pub block: BlockId,
    /// The statement's index, or `None` for the block's terminator.
    pub statement: Option<u32>,
    pub at: Span,
    pub kind: ErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// One of MIR's run-time checks failed, or a division or remainder met what it checks.
    Check(Check),
    /// Code the compiler marked unreachable was reached.
    Unreachable,
    /// An index past an array's end.
    Bounds { index: i128, len: u32 },
    /// A call past the stack: its callee's frame would end past the stack's slots, or the stack would hold more
    /// frames than it has slots (`docs/questions.md` Q230). The machine stands at the call, nothing of the callee
    /// written.
    Stack,
    /// A value read before anything was written to it.
    Uninit,
    /// An inconsistency that loading should have refused; named, never a panic.
    Internal(&'static str),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "function {} at {}:{}:{}: {:?}", self.function.0, self.at.file, self.at.line, self.at.column, self.kind)
    }
}

impl core::error::Error for ProgramError {}

/// Why an entry was not started, or a region not set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// An entry is running, or one failed and was not aborted.
    Busy,
    NoSuchFunction,
    NoSuchRegion,
    /// Arguments that are not the entry's parameters: too many, too few, or of another type.
    Arguments,
    /// Slots that are not a value of the region's type.
    Slots,
    /// A stack that cannot hold the entry's own frame.
    Stack,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for HostError {}

/// Why [`Machine::call`] gave no result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Host(HostError),
    Program(ProgramError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    function: u32,
    base: u32,
    block: u32,
    statement: u32,
    /// Where the result goes and where the caller goes on: `None` for the entry, whose result is the machine's.
    ret: Option<(u32, u32)>,
}

/// What loading fixed: the program, its layout, the host's regions, where the stack begins and ends, and the room
/// every buffer of the state is given.
#[derive(Clone, Debug)]
struct Code {
    program: Program,
    layout: Layout,
    regions: Vec<(u32, TypeId)>,
    stack: u32,
    stack_end: u32,
    room: Room,
}

/// The most each buffer of the state ever holds: as many frames as the stack has slots, the largest result and the
/// largest value built.
#[derive(Clone, Copy, Debug)]
struct Room {
    frames: usize,
    result: usize,
    scratch: usize,
}

/// What running changes; every buffer sized at load, and a copy sized alike (a `Vec`'s own clone keeps its length
/// alone, and would grow, so allocate, at the copy's first deeper call).
#[derive(Debug)]
struct State {
    memory: Vec<Scalar>,
    frames: Vec<Frame>,
    result: Vec<Scalar>,
    scratch: Vec<Scalar>,
    failed: Option<ProgramError>,
    steps: u64,
}

/// A loaded program and its memory. A copy is a machine of its own, standing where the original stood, with the
/// same room in every buffer.
#[derive(Debug)]
pub struct Machine {
    code: Code,
    state: State,
}

fn with_room<T: Copy>(items: &[T], room: usize) -> Vec<T> {
    let mut v = Vec::with_capacity(room.max(items.len()));
    v.extend_from_slice(items);
    v
}

impl Clone for Machine {
    fn clone(&self) -> Machine {
        let room = self.code.room;
        let state = &self.state;
        Machine {
            code: self.code.clone(),
            state: State {
                memory: state.memory.clone(),
                frames: with_room(&state.frames, room.frames),
                result: with_room(&state.result, room.result),
                scratch: with_room(&state.scratch, room.scratch),
                failed: state.failed,
                steps: state.steps,
            },
        }
    }
}

fn get<T: Copy>(slice: &[T], index: impl TryInto<usize>) -> Result<T, ErrorKind> {
    let i = index.try_into().map_err(|_| ErrorKind::Internal("an index past the machine's reach"))?;
    slice.get(i).copied().ok_or(ErrorKind::Internal("an index outside what was loaded"))
}

fn plus(a: u32, b: u32) -> Result<u32, ErrorKind> {
    a.checked_add(b).ok_or(ErrorKind::Internal("an offset past the machine's reach"))
}

fn fail(f: Fail) -> ErrorKind {
    match f {
        Fail::Check(c) => ErrorKind::Check(c),
        Fail::Uninit => ErrorKind::Uninit,
        Fail::Mismatch => ErrorKind::Internal("operands of types the operation does not take"),
    }
}

fn duration_slots(d: Duration) -> [Scalar; 2] {
    [Scalar::Int(Int::U64, i128::from(d.as_secs())), Scalar::Int(Int::U32, i128::from(d.subsec_nanos()))]
}

fn duration_of(secs: Scalar, nanos: Scalar) -> Result<Duration, ErrorKind> {
    match (secs, nanos) {
        (Scalar::Int(Int::U64, s), Scalar::Int(Int::U32, n)) if n < 1_000_000_000 => {
            let s = u64::try_from(s).map_err(|_| ErrorKind::Internal("a duration's seconds out of range"))?;
            let n = u32::try_from(n).map_err(|_| ErrorKind::Internal("a duration's nanoseconds out of range"))?;
            Ok(Duration::new(s, n))
        }
        (Scalar::Uninit, _) | (_, Scalar::Uninit) => Err(ErrorKind::Uninit),
        _ => Err(ErrorKind::Internal("a duration's slots are not a duration")),
    }
}

impl Code {
    fn function(&self, f: u32) -> Result<&Function, ErrorKind> {
        self.program.functions.get(f as usize).ok_or(ErrorKind::Internal("no such function"))
    }

    fn frame(&self, f: u32) -> Result<&FrameLayout, ErrorKind> {
        self.layout.frames.get(f as usize).ok_or(ErrorKind::Internal("no such frame"))
    }

    fn ty(&self, ty: TypeId) -> Result<&Type, ErrorKind> {
        self.program.types.get(ty.0 as usize).ok_or(ErrorKind::Internal("no such type"))
    }

    fn size(&self, ty: TypeId) -> Result<u32, ErrorKind> {
        get(&self.layout.sizes.size, ty.0)
    }

    /// The first slot of a place in a frame, and its type.
    fn resolve(&self, memory: &[Scalar], frame: &Frame, place: &Place) -> Result<(u32, TypeId), ErrorKind> {
        let function = self.function(frame.function)?;
        let layout = self.frame(frame.function)?;
        let mut ty = function.locals.get(place.local.0 as usize).ok_or(ErrorKind::Internal("no such local"))?.ty;
        let mut at = plus(frame.base, get(&layout.offsets, place.local.0)?)?;
        let mut variant: Option<u32> = None;
        for projection in &place.projections {
            match (projection, variant) {
                (Projection::Field(i), None) => match self.ty(ty)? {
                    Type::Struct { fields, .. } => {
                        let offsets = self.layout.sizes.fields.get(ty.0 as usize).ok_or(ErrorKind::Internal("no fields"))?;
                        at = plus(at, get(offsets, *i)?)?;
                        ty = fields.get(*i as usize).ok_or(ErrorKind::Internal("no such field"))?.ty;
                    }
                    _ => return Err(ErrorKind::Internal("a field of what is no structure")),
                },
                (Projection::Field(i), Some(v)) => match self.ty(ty)? {
                    Type::Enum { variants, .. } => {
                        let offsets = self
                            .layout
                            .sizes
                            .variants
                            .get(ty.0 as usize)
                            .and_then(|all| all.get(v as usize))
                            .ok_or(ErrorKind::Internal("no such variant"))?;
                        at = plus(at, get(offsets, *i)?)?;
                        ty = variants
                            .get(v as usize)
                            .and_then(|variant| variant.fields.get(*i as usize))
                            .ok_or(ErrorKind::Internal("no such field"))?
                            .ty;
                        variant = None;
                    }
                    _ => return Err(ErrorKind::Internal("a variant of what is no enum")),
                },
                (Projection::Downcast(v), None) => variant = Some(*v),
                (Projection::Deref, None) => match (self.ty(ty)?, get(memory, at)?) {
                    (Type::Ref { to, .. }, Scalar::Ref(target)) => {
                        at = target;
                        ty = *to;
                    }
                    (_, Scalar::Uninit) => return Err(ErrorKind::Uninit),
                    _ => return Err(ErrorKind::Internal("a dereference of what is no reference")),
                },
                (Projection::Index(_) | Projection::ConstantIndex(_), None) => {
                    let Type::Array { element, len } = *self.ty(ty)? else {
                        return Err(ErrorKind::Internal("an index into what is no array"));
                    };
                    let index = match projection {
                        Projection::Index(local) => {
                            let slot = plus(frame.base, get(&layout.offsets, local.0)?)?;
                            match get(memory, slot)? {
                                Scalar::Int(Int::Usize, i) => i,
                                Scalar::Uninit => return Err(ErrorKind::Uninit),
                                _ => return Err(ErrorKind::Internal("an index that is no usize")),
                            }
                        }
                        Projection::ConstantIndex(i) => i128::from(*i),
                        _ => return Err(ErrorKind::Internal("no index")),
                    };
                    if index < 0 || index >= i128::from(len) {
                        return Err(ErrorKind::Bounds { index, len });
                    }
                    let index = u32::try_from(index).map_err(|_| ErrorKind::Bounds { index, len })?;
                    let each = self.size(element)?;
                    at = plus(at, index.checked_mul(each).ok_or(ErrorKind::Internal("an index past reach"))?)?;
                    ty = element;
                }
                (_, Some(_)) => return Err(ErrorKind::Internal("a variant read but through a field")),
            }
        }
        if variant.is_some() {
            return Err(ErrorKind::Internal("a place that ends in a variant"));
        }
        Ok((at, ty))
    }
}

impl State {
    fn slot(&self, at: u32) -> Result<Scalar, ErrorKind> {
        get(&self.memory, at)
    }

    /// One slot of an operand whose type has one slot.
    fn scalar(&self, code: &Code, frame: &Frame, operand: &Operand) -> Result<Scalar, ErrorKind> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.slot(code.resolve(&self.memory, frame, place)?.0),
            Operand::Const(c) => c.slots.first().copied().ok_or(ErrorKind::Internal("an empty constant")),
        }
    }

    fn int(&self, code: &Code, frame: &Frame, operand: &Operand) -> Result<i128, ErrorKind> {
        match self.scalar(code, frame, operand)? {
            Scalar::Int(_, v) => Ok(v),
            Scalar::Uninit => Err(ErrorKind::Uninit),
            _ => Err(ErrorKind::Internal("an integer that is none")),
        }
    }

    /// A reference argument's target slot.
    fn target(&self, code: &Code, frame: &Frame, operand: &Operand) -> Result<u32, ErrorKind> {
        match self.scalar(code, frame, operand)? {
            Scalar::Ref(at) => Ok(at),
            Scalar::Uninit => Err(ErrorKind::Uninit),
            _ => Err(ErrorKind::Internal("a reference that is none")),
        }
    }

    fn duration_at(&self, at: u32) -> Result<Duration, ErrorKind> {
        duration_of(self.slot(at)?, self.slot(plus(at, 1)?)?)
    }

    fn duration(&self, code: &Code, frame: &Frame, operand: &Operand) -> Result<Duration, ErrorKind> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.duration_at(code.resolve(&self.memory, frame, place)?.0),
            Operand::Const(c) => match c.slots.as_slice() {
                [s, n] => duration_of(*s, *n),
                _ => Err(ErrorKind::Internal("a duration constant that is none")),
            },
        }
    }

    /// Appends an operand's slots to the scratch as they are: an enum's unwritten slots past its variant included.
    fn push_operand(&mut self, code: &Code, frame: &Frame, operand: &Operand) -> Result<(), ErrorKind> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let (at, ty) = code.resolve(&self.memory, frame, place)?;
                let end = plus(at, code.size(ty)?)?;
                let slots = self.memory.get(at as usize..end as usize).ok_or(ErrorKind::Internal("a place past memory"))?;
                self.scratch.extend_from_slice(slots);
            }
            Operand::Const(c) => self.scratch.extend_from_slice(&c.slots),
        }
        Ok(())
    }

    /// Writes the scratch to the slots from `at`.
    fn store(&mut self, at: u32) -> Result<(), ErrorKind> {
        let len = u32::try_from(self.scratch.len()).map_err(|_| ErrorKind::Internal("a value past reach"))?;
        let end = plus(at, len)?;
        let target = self.memory.get_mut(at as usize..end as usize).ok_or(ErrorKind::Internal("a place past memory"))?;
        target.copy_from_slice(&self.scratch);
        Ok(())
    }

    fn top(&mut self) -> Result<&mut Frame, ErrorKind> {
        self.frames.last_mut().ok_or(ErrorKind::Internal("no frame"))
    }

    fn goto(&mut self, block: u32) -> Result<Step, ErrorKind> {
        let top = self.top()?;
        top.block = block;
        top.statement = 0;
        Ok(Step::Ran)
    }

    /// A value, built in the scratch.
    fn evaluate(&mut self, code: &Code, frame: &Frame, value: &Value) -> Result<(), ErrorKind> {
        self.scratch.clear();
        match value {
            Value::Use(op) => self.push_operand(code, frame, op)?,
            Value::Binary(op, a, b) => {
                let r = ops::binary(*op, self.scalar(code, frame, a)?, self.scalar(code, frame, b)?).map_err(fail)?;
                self.scratch.push(r);
            }
            Value::Checked(op, a, b) => {
                let (r, overflowed) =
                    ops::checked(*op, self.scalar(code, frame, a)?, self.scalar(code, frame, b)?).map_err(fail)?;
                self.scratch.push(r);
                self.scratch.push(Scalar::Bool(overflowed));
            }
            Value::Unary(op, a) => {
                let r = ops::unary(*op, self.scalar(code, frame, a)?).map_err(fail)?;
                self.scratch.push(r);
            }
            Value::Cast(a, to) => {
                let r = ops::cast(self.scalar(code, frame, a)?, code.ty(*to)?).map_err(fail)?;
                self.scratch.push(r);
            }
            Value::Aggregate(ty, variant, parts) => {
                match (code.ty(*ty)?, variant) {
                    (Type::Enum { discriminant, variants, .. }, Some(v)) => {
                        let d = variants.get(*v as usize).ok_or(ErrorKind::Internal("no such variant"))?.discriminant;
                        self.scratch.push(Scalar::Int(*discriminant, d));
                    }
                    (Type::Struct { .. } | Type::Array { .. }, None) => {}
                    _ => return Err(ErrorKind::Internal("an aggregate of what is built otherwise")),
                }
                for part in parts {
                    self.push_operand(code, frame, part)?;
                }
                self.scratch.resize(code.size(*ty)? as usize, Scalar::Uninit);
            }
            Value::Discriminant(place) => {
                let (at, _) = code.resolve(&self.memory, frame, place)?;
                match self.slot(at)? {
                    s @ Scalar::Int(..) => self.scratch.push(s),
                    Scalar::Uninit => return Err(ErrorKind::Uninit),
                    _ => return Err(ErrorKind::Internal("a discriminant that is no integer")),
                }
            }
            Value::Ref(place, _) => {
                let (at, _) = code.resolve(&self.memory, frame, place)?;
                self.scratch.push(Scalar::Ref(at));
            }
        }
        Ok(())
    }

    /// Runs the instruction `frame` stands at.
    fn execute(&mut self, code: &Code, frame: &Frame) -> Result<Step, (Option<u32>, Span, ErrorKind)> {
        let function = code.function(frame.function).map_err(|k| (None, Span::default(), k))?;
        let block =
            function.blocks.get(frame.block as usize).ok_or((None, Span::default(), ErrorKind::Internal("no block")))?;
        if let Some(statement) = block.statements.get(frame.statement as usize) {
            let mut run = || -> Result<(), ErrorKind> {
                let (dest, _) = code.resolve(&self.memory, frame, &statement.place)?;
                self.evaluate(code, frame, &statement.value)?;
                self.store(dest)?;
                let top = self.top()?;
                top.statement = top.statement.saturating_add(1);
                Ok(())
            };
            return run().map(|()| Step::Ran).map_err(|k| (Some(frame.statement), statement.at, k));
        }
        let at = match &block.terminator {
            Terminator::Goto(_) => Span::default(),
            Terminator::SwitchInt { at, .. }
            | Terminator::Return { at }
            | Terminator::Call { at, .. }
            | Terminator::Assert { at, .. }
            | Terminator::Unreachable { at } => *at,
        };
        self.terminate(code, frame, &block.terminator).map_err(|k| (None, at, k))
    }

    fn terminate(&mut self, code: &Code, frame: &Frame, terminator: &Terminator) -> Result<Step, ErrorKind> {
        match terminator {
            Terminator::Goto(b) => self.goto(b.0),
            Terminator::SwitchInt { value, targets, otherwise, .. } => {
                let v = match self.scalar(code, frame, value)? {
                    Scalar::Bool(b) => i128::from(b),
                    Scalar::Int(_, v) => v,
                    Scalar::Uninit => return Err(ErrorKind::Uninit),
                    _ => return Err(ErrorKind::Internal("a switch on what is no integer")),
                };
                let target = targets.iter().find(|(t, _)| *t == v).map_or(otherwise.0, |(_, b)| b.0);
                self.goto(target)
            }
            Terminator::Assert { cond, expected, check, next, .. } => match self.scalar(code, frame, cond)? {
                Scalar::Bool(b) if b == *expected => self.goto(next.0),
                Scalar::Bool(_) => Err(ErrorKind::Check(*check)),
                Scalar::Uninit => Err(ErrorKind::Uninit),
                _ => Err(ErrorKind::Internal("an assertion of what is no bool")),
            },
            Terminator::Unreachable { .. } => Err(ErrorKind::Unreachable),
            Terminator::Return { .. } => {
                let function = code.function(frame.function)?;
                let ret = function.locals.first().ok_or(ErrorKind::Internal("no return value"))?.ty;
                let from = plus(frame.base, get(&code.frame(frame.function)?.offsets, 0u32)?)?;
                let end = plus(from, code.size(ret)?)?;
                let slots = self.memory.get(from as usize..end as usize).ok_or(ErrorKind::Internal("a frame past memory"))?;
                self.scratch.clear();
                self.scratch.extend_from_slice(slots);
                self.frames.pop();
                match frame.ret {
                    None => {
                        self.result.clear();
                        self.result.extend_from_slice(&self.scratch);
                        Ok(Step::Returned)
                    }
                    Some((dest, next)) => {
                        self.store(dest)?;
                        self.goto(next)
                    }
                }
            }
            Terminator::Call { callee: Callee::Function(g), args, dest, next, .. } => {
                let (dest, _) = code.resolve(&self.memory, frame, dest)?;
                let base = plus(frame.base, code.frame(frame.function)?.size)?;
                let callee = code.frame(g.0)?;
                let end = plus(base, callee.size)?;
                // Q230: the stack is the host's size; a call past it, in slots or in frames, is the program's error
                if end > code.stack_end || self.frames.len() >= code.room.frames {
                    return Err(ErrorKind::Stack);
                }
                self.memory
                    .get_mut(base as usize..end as usize)
                    .ok_or(ErrorKind::Internal("a frame past memory"))?
                    .fill(Scalar::Uninit);
                for (i, arg) in args.iter().enumerate() {
                    self.scratch.clear();
                    self.push_operand(code, frame, arg)?;
                    let offset = callee.offsets.get(i.saturating_add(1)).copied().ok_or(ErrorKind::Internal("no parameter"))?;
                    self.store(plus(base, offset)?)?;
                }
                self.frames.push(Frame { function: g.0, base, block: 0, statement: 0, ret: Some((dest, next.0)) });
                Ok(Step::Ran)
            }
            Terminator::Call { callee: Callee::Core(op), args, dest, next, .. } => {
                let (dest_at, dest_ty) = code.resolve(&self.memory, frame, dest)?;
                self.core(code, frame, *op, args, dest_ty)?;
                self.store(dest_at)?;
                self.goto(next.0)
            }
        }
    }

    /// Runs a core operation, its result left in the scratch.
    fn core(&mut self, code: &Code, frame: &Frame, op: CoreOp, args: &[Operand], dest: TypeId) -> Result<(), ErrorKind> {
        let arg = |i: usize| args.get(i).ok_or(ErrorKind::Internal("a missing argument"));
        let mut result: [Scalar; 2] = [Scalar::Uninit; 2];
        let mut len = 1usize;
        match op {
            CoreOp::SaturatingAdd(t) | CoreOp::SaturatingSub(t) | CoreOp::SaturatingMul(t) => {
                let (a, b) = (self.int(code, frame, arg(0)?)?, self.int(code, frame, arg(1)?)?);
                let exact = match op {
                    CoreOp::SaturatingAdd(_) => a.checked_add(b),
                    CoreOp::SaturatingSub(_) => a.checked_sub(b),
                    _ => a.checked_mul(b),
                };
                // only a product of two 64-bit values can pass i128, and then it passes every type's bounds too,
                // on the side its operands' signs give
                let value = exact.unwrap_or(if (a < 0) == (b < 0) { i128::MAX } else { i128::MIN });
                result[0] = Scalar::Int(t, value.clamp(t.min(), t.max()));
            }
            CoreOp::WrappingAdd(t) | CoreOp::WrappingSub(t) | CoreOp::WrappingMul(t) => {
                let bin = match op {
                    CoreOp::WrappingAdd(_) => BinOp::Add,
                    CoreOp::WrappingSub(_) => BinOp::Sub,
                    _ => BinOp::Mul,
                };
                let a = Scalar::Int(t, self.int(code, frame, arg(0)?)?);
                let b = Scalar::Int(t, self.int(code, frame, arg(1)?)?);
                result[0] = ops::binary(bin, a, b).map_err(fail)?;
            }
            CoreOp::CheckedShl(t) | CoreOp::CheckedShr(t) => {
                let (value, amount) = (self.int(code, frame, arg(0)?)?, self.int(code, frame, arg(1)?)?);
                let Type::Enum { discriminant, .. } = code.ty(dest)? else {
                    return Err(ErrorKind::Internal("an option that is no enum"));
                };
                if amount >= i128::from(t.bits()) {
                    result[0] = Scalar::Int(*discriminant, 0);
                } else {
                    let bin = if matches!(op, CoreOp::CheckedShl(_)) { BinOp::Shl } else { BinOp::Shr };
                    let shifted = ops::binary(bin, Scalar::Int(t, value), Scalar::Int(Int::U32, amount)).map_err(fail)?;
                    result = [Scalar::Int(*discriminant, 1), shifted];
                    len = 2;
                }
            }
            CoreOp::Min(t) | CoreOp::Max(t) => {
                let (a, b) = (self.int(code, frame, arg(0)?)?, self.int(code, frame, arg(1)?)?);
                result[0] = Scalar::Int(t, if matches!(op, CoreOp::Min(_)) { a.min(b) } else { a.max(b) });
            }
            CoreOp::DurationFromMillis | CoreOp::DurationFromSecs => {
                let v = u64::try_from(self.int(code, frame, arg(0)?)?).map_err(|_| ErrorKind::Internal("no u64"))?;
                let d = if op == CoreOp::DurationFromMillis { Duration::from_millis(v) } else { Duration::from_secs(v) };
                result = duration_slots(d);
                len = 2;
            }
            CoreOp::DurationSaturatingSub | CoreOp::DurationMin | CoreOp::DurationMax => {
                let (a, b) = (self.duration(code, frame, arg(0)?)?, self.duration(code, frame, arg(1)?)?);
                let d = match op {
                    CoreOp::DurationSaturatingSub => a.saturating_sub(b),
                    CoreOp::DurationMin => a.min(b),
                    _ => a.max(b),
                };
                result = duration_slots(d);
                len = 2;
            }
            CoreOp::DurationCompare(cmp) => {
                let a = self.duration_at(self.target(code, frame, arg(0)?)?)?;
                let b = self.duration_at(self.target(code, frame, arg(1)?)?)?;
                result[0] = Scalar::Bool(match cmp {
                    BinOp::Eq => a == b,
                    BinOp::Ne => a != b,
                    BinOp::Lt => a < b,
                    BinOp::Le => a <= b,
                    BinOp::Gt => a > b,
                    BinOp::Ge => a >= b,
                    _ => return Err(ErrorKind::Internal("no comparison")),
                });
            }
            CoreOp::OptionIsNone | CoreOp::OptionIsSome => {
                let some = match self.slot(self.target(code, frame, arg(0)?)?)? {
                    Scalar::Int(_, d) => d == 1,
                    Scalar::Uninit => return Err(ErrorKind::Uninit),
                    _ => return Err(ErrorKind::Internal("an option's discriminant that is none")),
                };
                result[0] = Scalar::Bool(if op == CoreOp::OptionIsSome { some } else { !some });
            }
            CoreOp::OptionGetOrInsert => {
                let at = self.target(code, frame, arg(0)?)?;
                let (dty, some) = match self.slot(at)? {
                    Scalar::Int(t, d) => (t, d == 1),
                    Scalar::Uninit => return Err(ErrorKind::Uninit),
                    _ => return Err(ErrorKind::Internal("an option's discriminant that is none")),
                };
                let field = plus(at, 1)?;
                if !some {
                    self.scratch.clear();
                    self.push_operand(code, frame, arg(1)?)?;
                    self.store(field)?;
                    *self.memory.get_mut(at as usize).ok_or(ErrorKind::Internal("an option past memory"))? =
                        Scalar::Int(dty, 1);
                }
                result[0] = Scalar::Ref(field);
            }
            CoreOp::Default => match code.ty(dest)? {
                Type::Bool => result[0] = Scalar::Bool(false),
                Type::Int(t) => result[0] = Scalar::Int(*t, 0),
                Type::F32 => result[0] = Scalar::F32(0.0),
                Type::F64 => result[0] = Scalar::F64(0.0),
                Type::Duration => {
                    result = duration_slots(Duration::ZERO);
                    len = 2;
                }
                // an `Option`, as the load checked: `None`, its discriminant 0, the rest unwritten
                Type::Enum { discriminant, .. } => result[0] = Scalar::Int(*discriminant, 0),
                _ => return Err(ErrorKind::Internal("no default")),
            },
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(result.get(..len).ok_or(ErrorKind::Internal("a result past its slots"))?);
        self.scratch.resize(code.size(dest)? as usize, Scalar::Uninit);
        Ok(())
    }
}

impl Machine {
    /// Loads `program`, with a region of memory for each of `regions`' types and a stack of `limits.stack` slots
    /// after them, or says why it cannot. The stack's size and the memory's ceiling are the host's, as the target's
    /// settings give them (`docs/questions.md` Q230); in slots, not bytes, so a program reaches the same depth
    /// wherever it runs. Every buffer is sized here, its size checked against `limits.memory` before anything is
    /// allocated, so nothing is allocated while it runs and a size the program or the host names never reaches the
    /// allocator past the ceiling. A call past the stack is the program's error [`ErrorKind::Stack`].
    pub fn load(program: Program, regions: &[TypeId], limits: Limits) -> Result<Machine, LoadError> {
        let stack = limits.stack;
        let layout = lay_out(&program)?;
        let too_large = |index| LoadError::Region { index, reason: "memory past the machine's reach" };
        let mut placed = Vec::with_capacity(regions.len());
        let mut at = 0u32;
        for (index, ty) in regions.iter().enumerate() {
            let size = layout
                .sizes
                .size
                .get(ty.0 as usize)
                .copied()
                .ok_or(LoadError::Region { index, reason: "a region of no such type" })?;
            placed.push((at, *ty));
            at = at.checked_add(size).ok_or(too_large(index))?;
        }
        let total = at.checked_add(stack).ok_or(too_large(regions.len()))?;
        let frames = stack as usize;
        let returns = program
            .functions
            .iter()
            .filter_map(|f| f.locals.first())
            .filter_map(|l| layout.sizes.size.get(l.ty.0 as usize).copied())
            .max()
            .unwrap_or(0);
        let regions_largest = regions.iter().filter_map(|t| layout.sizes.size.get(t.0 as usize).copied()).max().unwrap_or(0);
        let largest = layout.largest.max(regions_largest);
        // the ceiling, before any buffer is allocated: the regions and the stack (which a slot's index must reach),
        // the largest value built and the largest result, in slots, summed where no sum of three `u32`s overflows
        let laid_out = u64::from(total).saturating_add(u64::from(largest)).saturating_add(u64::from(returns));
        if laid_out > u64::from(limits.memory) {
            return Err(LoadError::Region { index: regions.len(), reason: "memory past the host's limit" });
        }
        let (largest, returns) = (largest as usize, returns as usize);
        Ok(Machine {
            code: Code {
                program,
                layout,
                regions: placed,
                stack: at,
                stack_end: total,
                room: Room { frames, result: returns, scratch: largest },
            },
            state: State {
                memory: vec![Scalar::Uninit; total as usize],
                frames: Vec::with_capacity(frames),
                result: Vec::with_capacity(returns),
                scratch: Vec::with_capacity(largest),
                failed: None,
                steps: 0,
            },
        })
    }

    pub fn program(&self) -> &Program {
        &self.code.program
    }

    /// The slots a value of `ty` occupies.
    pub fn size_of(&self, ty: TypeId) -> Option<u32> {
        self.code.layout.sizes.size.get(ty.0 as usize).copied()
    }

    /// The slots of the machine's memory, regions first, then the stack: what a test reads to see that nothing
    /// else was written.
    pub fn memory(&self) -> &[Scalar] {
        &self.state.memory
    }

    /// A region's slots.
    pub fn region(&self, region: RegionId) -> Option<&[Scalar]> {
        let (base, ty) = *self.code.regions.get(region.0 as usize)?;
        let size = self.size_of(ty)?;
        self.state.memory.get(base as usize..base.checked_add(size)? as usize)
    }

    /// Sets a region to a value of its type, refused unless `slots` are one.
    pub fn set_region(&mut self, region: RegionId, slots: &[Scalar]) -> Result<(), HostError> {
        let (base, ty) = *self.code.regions.get(region.0 as usize).ok_or(HostError::NoSuchRegion)?;
        if !fits(&self.code.program.types, &self.code.layout.sizes, ty, slots) {
            return Err(HostError::Slots);
        }
        let end = base.checked_add(self.size_of(ty).ok_or(HostError::NoSuchRegion)?).ok_or(HostError::NoSuchRegion)?;
        let target = self.state.memory.get_mut(base as usize..end as usize).ok_or(HostError::NoSuchRegion)?;
        // `fits` holds the length to the type's; said again here, so a copy of another length is a refusal and never
        // the slice copy's panic
        if target.len() != slots.len() {
            return Err(HostError::Slots);
        }
        target.copy_from_slice(slots);
        Ok(())
    }

    /// Whether an entry is running, a failed one not yet aborted among them.
    pub fn running(&self) -> bool {
        !self.state.frames.is_empty()
    }

    /// Instructions the running entry, or the last, has run.
    pub fn steps(&self) -> u64 {
        self.state.steps
    }

    /// The last entry's result: the slots of its return value.
    pub fn result(&self) -> &[Scalar] {
        &self.state.result
    }

    /// Where the running entry stands: the function, the block and the statement its next step runs (the block's
    /// terminator once the statement is past the last).
    pub fn position(&self) -> Option<(FunctionId, BlockId, u32)> {
        self.state.frames.last().map(|f| (FunctionId(f.function), BlockId(f.block), f.statement))
    }

    /// How many calls deep the running entry is: 1 in the entry itself.
    pub fn depth(&self) -> usize {
        self.state.frames.len()
    }

    /// Ends the running entry where it stands, a failed one among them; memory keeps what it was written.
    pub fn abort(&mut self) {
        self.state.frames.clear();
        self.state.failed = None;
    }

    /// Starts `function` with `args`; its first instruction runs on the next [`Machine::step`]. Refused, before
    /// anything is written, when an entry runs, when `args` are not its parameters, or when the stack cannot hold its
    /// own frame ([`HostError::Stack`]).
    pub fn start(&mut self, function: FunctionId, args: &[Arg<'_>]) -> Result<(), HostError> {
        let (code, state) = (&self.code, &mut self.state);
        if !state.frames.is_empty() || state.failed.is_some() {
            return Err(HostError::Busy);
        }
        let f = code.program.functions.get(function.0 as usize).ok_or(HostError::NoSuchFunction)?;
        let frame = code.layout.frames.get(function.0 as usize).ok_or(HostError::NoSuchFunction)?;
        if args.len() != f.params as usize {
            return Err(HostError::Arguments);
        }
        let base = code.stack;
        let end = base.checked_add(frame.size).ok_or(HostError::Stack)?;
        if end > code.stack_end || code.room.frames == 0 {
            return Err(HostError::Stack);
        }
        let end = end as usize;
        state.memory.get_mut(base as usize..end).ok_or(HostError::Arguments)?.fill(Scalar::Uninit);
        for (i, arg) in args.iter().enumerate() {
            let local = f.locals.get(i.saturating_add(1)).ok_or(HostError::Arguments)?;
            let offset = *frame.offsets.get(i.saturating_add(1)).ok_or(HostError::Arguments)?;
            let at = base.checked_add(offset).ok_or(HostError::Arguments)? as usize;
            match arg {
                Arg::Ref(region) => {
                    let (rbase, rty) = *code.regions.get(region.0 as usize).ok_or(HostError::NoSuchRegion)?;
                    let good = matches!(
                        code.program.types.get(local.ty.0 as usize),
                        Some(Type::Ref { to, .. }) if crate::core_ops::same(&code.program.types, *to, rty)
                    );
                    if !good {
                        return Err(HostError::Arguments);
                    }
                    *state.memory.get_mut(at).ok_or(HostError::Arguments)? = Scalar::Ref(rbase);
                }
                Arg::Value(slots) => {
                    if !fits(&code.program.types, &code.layout.sizes, local.ty, slots) {
                        return Err(HostError::Arguments);
                    }
                    let target = state.memory.get_mut(at..at.saturating_add(slots.len())).ok_or(HostError::Arguments)?;
                    target.copy_from_slice(slots);
                }
            }
        }
        state.steps = 0;
        state.result.clear();
        state.frames.push(Frame { function: function.0, base, block: 0, statement: 0, ret: None });
        Ok(())
    }

    /// Runs one instruction of the running entry. After a program error every step gives that error again, until
    /// [`Machine::abort`].
    pub fn step(&mut self) -> Result<Step, ProgramError> {
        let (code, state) = (&self.code, &mut self.state);
        if let Some(e) = state.failed {
            return Err(e);
        }
        let Some(frame) = state.frames.last().copied() else { return Ok(Step::Idle) };
        match state.execute(code, &frame) {
            Ok(step) => {
                state.steps = state.steps.saturating_add(1);
                Ok(step)
            }
            Err((statement, at, kind)) => {
                let e = ProgramError { function: FunctionId(frame.function), block: BlockId(frame.block), statement, at, kind };
                state.failed = Some(e);
                Err(e)
            }
        }
    }

    /// Runs up to `budget` instructions: [`Step::Returned`] once the entry has returned, [`Step::Ran`] when the
    /// budget ran out first (a budget of zero included, the entry standing where it stood), [`Step::Idle`] when
    /// nothing was running; a failed entry's error, as [`Machine::step`] gives it.
    pub fn run(&mut self, budget: u64) -> Result<Step, ProgramError> {
        if let Some(e) = self.state.failed {
            return Err(e);
        }
        let mut last = if self.running() { Step::Ran } else { Step::Idle };
        for _ in 0..budget {
            last = self.step()?;
            if last != Step::Ran {
                return Ok(last);
            }
        }
        Ok(last)
    }

    /// Starts `function` with `args` and runs it to its return.
    pub fn call(&mut self, function: FunctionId, args: &[Arg<'_>]) -> Result<&[Scalar], CallError> {
        self.start(function, args).map_err(CallError::Host)?;
        loop {
            match self.step().map_err(CallError::Program)? {
                Step::Ran => {}
                Step::Returned | Step::Idle => return Ok(&self.state.result),
            }
        }
    }
}
