//! A function's body, from `rustc_public`'s MIR to the interpreter's: one statement for each assignment (storage
//! markers and the borrow checker's reads dropped), one terminator a block, the blocks reached without unwinding
//! alone (the interpreter does not unwind; an ORCL type has no drop glue, so no cleanup is skipped), and every call
//! resolved: to a function of the program's crates, lowered in turn, or to an operation of the closed list.
use std::collections::{HashMap, VecDeque};

use orcl_interp::{
    BinOp, Block, BlockId, Callee, Check, Const, Function, Local, LocalDecl, Operand, Place, Projection, Statement,
    Terminator, Type, TypeId, UnOp, Value,
};
use rustc_public::CrateDef;
use rustc_public::mir::mono::{Instance, InstanceKind};
use rustc_public::mir::{
    self as mir, AggregateKind, AssertMessage, BorrowKind, CastKind, ProjectionElem, Rvalue, StatementKind,
    TerminatorKind, VarDebugInfoContents,
};
use rustc_public::ty::{RigidTy, Span, Ty, TyKind};

use crate::consts::Lit;
use crate::defs::Defs;
use crate::error::{Refusal, Refused};
use crate::lower::Lowerer;

/// The most copies `[x; N]` is lowered to: the interpreter has no repeat, so the front end builds the array from N
/// operands.
const REPEAT: usize = 1 << 16;

/// A body being lowered: its locals, MIR's and the ones the lowering adds, and its blocks' new numbers.
struct Frame<'b> {
    body: &'b mir::Body,
    locals: Vec<LocalDecl>,
    blocks: Vec<Option<u32>>,
}

/// The blocks reached from the first without unwinding, in MIR's order.
fn reached(body: &mir::Body) -> Vec<Option<u32>> {
    let mut seen = vec![false; body.blocks.len()];
    let mut todo = VecDeque::from([0usize]);
    while let Some(b) = todo.pop_front() {
        match seen.get_mut(b) {
            Some(s) if !*s => *s = true,
            _ => continue,
        }
        let Some(block) = body.blocks.get(b) else { continue };
        let next: Vec<usize> = match &block.terminator.kind {
            TerminatorKind::Goto { target } => vec![*target],
            TerminatorKind::SwitchInt { targets, .. } => targets.all_targets(),
            TerminatorKind::Drop { target, .. } | TerminatorKind::Assert { target, .. } => vec![*target],
            TerminatorKind::Call { target, .. } => target.iter().copied().collect(),
            TerminatorKind::InlineAsm { destination, .. } => destination.iter().copied().collect(),
            TerminatorKind::Resume | TerminatorKind::Abort | TerminatorKind::Return | TerminatorKind::Unreachable => {
                Vec::new()
            }
        };
        todo.extend(next);
    }
    let mut n = 0u32;
    seen.iter()
        .map(|s| {
            s.then(|| {
                let id = n;
                n = n.saturating_add(1);
                id
            })
        })
        .collect()
}

const fn bin_op(op: mir::BinOp) -> Option<BinOp> {
    Some(match op {
        mir::BinOp::Add => BinOp::Add,
        mir::BinOp::Sub => BinOp::Sub,
        mir::BinOp::Mul => BinOp::Mul,
        mir::BinOp::Div => BinOp::Div,
        mir::BinOp::Rem => BinOp::Rem,
        mir::BinOp::BitXor => BinOp::BitXor,
        mir::BinOp::BitAnd => BinOp::BitAnd,
        mir::BinOp::BitOr => BinOp::BitOr,
        mir::BinOp::Shl => BinOp::Shl,
        mir::BinOp::Shr => BinOp::Shr,
        mir::BinOp::Eq => BinOp::Eq,
        mir::BinOp::Lt => BinOp::Lt,
        mir::BinOp::Le => BinOp::Le,
        mir::BinOp::Ne => BinOp::Ne,
        mir::BinOp::Ge => BinOp::Ge,
        mir::BinOp::Gt => BinOp::Gt,
        _ => return None,
    })
}

impl Lowerer<'_> {
    /// The function `instance` is, its body lowered, named `name`.
    pub(crate) fn function(&mut self, instance: Instance, name: String) -> Result<Function, Refusal> {
        let at = instance.def.span();
        let body = instance.body().ok_or_else(|| self.refuse(Refused::NoBody, instance.trimmed_name(), at))?;
        if body.spread_arg().is_some() {
            return Err(self.refuse(Refused::Call, instance.trimmed_name(), at));
        }
        let params = u32::try_from(body.arg_locals().len()).unwrap_or(u32::MAX);
        let mut names: HashMap<usize, String> = HashMap::new();
        for info in &body.var_debug_info {
            if let (VarDebugInfoContents::Place(p), None) = (&info.value, &info.composite)
                && p.projection.is_empty()
            {
                names.entry(p.local).or_insert_with(|| info.name.clone());
            }
        }
        let mut locals = Vec::with_capacity(body.locals().len());
        for (i, decl) in body.locals().iter().enumerate() {
            let ty = self.ty(decl.ty, decl.span)?;
            let name = names.get(&i).cloned();
            // ORCL has no `usize` or `isize`: the program's own results, parameters and variables are refused one
            // anywhere in their types, the compiler's temporaries keep theirs (an index, a discriminant)
            if i <= params as usize || name.is_some() {
                self.no_word(ty, decl.span)?;
            }
            locals.push(LocalDecl { ty, name });
        }
        let mut frame = Frame { body: &body, locals, blocks: reached(&body) };
        let mut blocks = Vec::new();
        for (b, block) in body.blocks.iter().enumerate() {
            if frame.blocks.get(b).copied().flatten().is_none() {
                continue;
            }
            let mut statements = Vec::new();
            for statement in &block.statements {
                self.statement(&mut frame, statement, &mut statements)?;
            }
            let terminator = self.terminator(&mut frame, &block.terminator, &mut statements)?;
            blocks.push(Block { statements, terminator });
        }
        Ok(Function { name, params, locals: frame.locals, blocks })
    }

    fn block(&self, frame: &Frame<'_>, b: usize, at: Span) -> Result<BlockId, Refusal> {
        frame
            .blocks
            .get(b)
            .copied()
            .flatten()
            .map(BlockId)
            .ok_or_else(|| self.refuse(Refused::Terminator, "a jump to a block not reached", at))
    }

    fn place_ty(&self, frame: &Frame<'_>, place: &mir::Place, at: Span) -> Result<Ty, Refusal> {
        place.ty(frame.body.locals()).map_err(|_| self.refuse(Refused::Value, "a place of no type", at))
    }

    fn place(&mut self, frame: &Frame<'_>, place: &mir::Place, at: Span) -> Result<Place, Refusal> {
        let mut projections = Vec::with_capacity(place.projection.len());
        let mut ty = frame.body.locals().get(place.local).map(|l| l.ty);
        for elem in &place.projection {
            let here = ty.ok_or_else(|| self.refuse(Refused::Value, "a place of no type", at))?;
            projections.push(match elem {
                ProjectionElem::Deref => Projection::Deref,
                ProjectionElem::Field(i, _) => Projection::Field(u32::try_from(*i).unwrap_or(u32::MAX)),
                ProjectionElem::Index(l) => Projection::Index(Local(u32::try_from(*l).unwrap_or(u32::MAX))),
                ProjectionElem::ConstantIndex { offset, from_end: false, .. } => {
                    Projection::ConstantIndex(u32::try_from(*offset).unwrap_or(u32::MAX))
                }
                ProjectionElem::ConstantIndex { offset, from_end: true, .. } => {
                    let len = match here.kind() {
                        TyKind::RigidTy(RigidTy::Array(_, len)) => len.eval_target_usize().ok(),
                        _ => None,
                    };
                    let index = len.and_then(|n| n.checked_sub(*offset)).and_then(|i| u32::try_from(i).ok());
                    Projection::ConstantIndex(index.ok_or_else(|| self.refuse(Refused::Value, "an index from a slice's end", at))?)
                }
                ProjectionElem::Downcast(v) => {
                    let TyKind::RigidTy(RigidTy::Adt(def, _)) = here.kind() else {
                        return Err(self.refuse(Refused::Value, "a variant of what is no enum", at));
                    };
                    let index = def.variants_iter().position(|x| x.idx() == *v);
                    Projection::Downcast(
                        index
                            .and_then(|i| u32::try_from(i).ok())
                            .ok_or_else(|| self.refuse(Refused::Value, "no such variant", at))?,
                    )
                }
                ProjectionElem::Subslice { .. } => return Err(self.refuse(Refused::Value, "a subslice", at)),
                ProjectionElem::OpaqueCast(_) => return Err(self.refuse(Refused::Value, "an opaque cast", at)),
            });
            ty = elem.ty(here).ok();
        }
        Ok(Place { local: Local(u32::try_from(place.local).unwrap_or(u32::MAX)), projections })
    }

    /// A local the lowering adds to the body.
    fn temporary(frame: &mut Frame<'_>, ty: TypeId) -> Local {
        let local = Local(u32::try_from(frame.locals.len()).unwrap_or(u32::MAX));
        frame.locals.push(LocalDecl { ty, name: None });
        local
    }

    /// An operand. A shared reference to a constant becomes a reference to a local holding the constant, set by the
    /// statements `before` gains, since the interpreter keeps no constant in memory of its own.
    fn operand(
        &mut self,
        frame: &mut Frame<'_>,
        operand: &mir::Operand,
        at: Span,
        before: &mut Vec<Statement>,
    ) -> Result<Operand, Refusal> {
        match operand {
            mir::Operand::Copy(p) => Ok(Operand::Copy(self.place(frame, p, at)?)),
            mir::Operand::Move(p) => Ok(Operand::Move(self.place(frame, p, at)?)),
            mir::Operand::Constant(c) => match self.constant(&c.const_, c.span)? {
                Lit::Value(value) => Ok(Operand::Const(value)),
                Lit::RefTo(value) => {
                    let to = value.ty;
                    let reference = self.reference_to(to);
                    let held = Self::temporary(frame, to);
                    let r = Self::temporary(frame, reference);
                    let span = self.span(at);
                    let place = |l: Local| Place { local: l, projections: Vec::new() };
                    before.push(Statement { place: place(held), value: Value::Use(Operand::Const(value)), at: span });
                    before.push(Statement { place: place(r), value: Value::Ref(place(held), false), at: span });
                    Ok(Operand::Copy(place(r)))
                }
            },
            mir::Operand::RuntimeChecks(check) => Err(self.refuse(Refused::Value, format!("{check:?}"), at)),
        }
    }

    fn reference_to(&mut self, to: TypeId) -> TypeId {
        if let Some(id) = self.keys.get(&crate::types::Key::Ref(to, false)) {
            return *id;
        }
        let id = TypeId(u32::try_from(self.types.len()).unwrap_or(u32::MAX));
        self.types.push(Type::Ref { to, mutable: false });
        self.slots.push(1);
        self.keys.insert(crate::types::Key::Ref(to, false), id);
        id
    }

    fn operands(
        &mut self,
        frame: &mut Frame<'_>,
        operands: &[mir::Operand],
        at: Span,
        before: &mut Vec<Statement>,
    ) -> Result<Vec<Operand>, Refusal> {
        let mut out = Vec::with_capacity(operands.len());
        for op in operands {
            out.push(self.operand(frame, op, at, before)?);
        }
        Ok(out)
    }

    fn statement(&mut self, frame: &mut Frame<'_>, statement: &mir::Statement, out: &mut Vec<Statement>) -> Result<(), Refusal> {
        let at = statement.source_info.span;
        match &statement.kind {
            StatementKind::Assign(place, rvalue) => {
                let place = self.place(frame, place, at)?;
                let value = self.rvalue(frame, rvalue, at, out)?;
                let span = self.span(at);
                out.push(Statement { place, value, at: span });
                Ok(())
            }
            StatementKind::FakeRead(..)
            | StatementKind::StorageLive(_)
            | StatementKind::StorageDead(_)
            | StatementKind::PlaceMention(_)
            | StatementKind::AscribeUserType { .. }
            | StatementKind::Coverage(_)
            | StatementKind::ConstEvalCounter
            | StatementKind::Nop => Ok(()),
            StatementKind::SetDiscriminant { .. } => Err(self.refuse(Refused::Statement, "SetDiscriminant", at)),
            StatementKind::Intrinsic(mir::NonDivergingIntrinsic::Assume(_)) => {
                Err(self.refuse(Refused::Statement, "Intrinsic(Assume)", at))
            }
            StatementKind::Intrinsic(mir::NonDivergingIntrinsic::CopyNonOverlapping(_)) => {
                Err(self.refuse(Refused::Statement, "Intrinsic(CopyNonOverlapping)", at))
            }
        }
    }

    fn rvalue(&mut self, frame: &mut Frame<'_>, rvalue: &Rvalue, at: Span, before: &mut Vec<Statement>) -> Result<Value, Refusal> {
        let refused = |s: &Self, what: &str| Err(s.refuse(Refused::Value, what, at));
        match rvalue {
            Rvalue::Use(op, _) => Ok(Value::Use(self.operand(frame, op, at, before)?)),
            Rvalue::CopyForDeref(p) => Ok(Value::Use(Operand::Copy(self.place(frame, p, at)?))),
            Rvalue::Repeat(op, count) => {
                // `[x; N]` is the array of N copies of x, built as the interpreter builds an array: one operand each,
                // so N is bounded, past which the instruction would outweigh the program
                let ty = rvalue.ty(frame.body.locals()).map_err(|_| self.refuse(Refused::Value, "an array of no type", at))?;
                let ty = self.ty(ty, at)?;
                let n = count.eval_target_usize().map_err(|_| self.refuse(Refused::Value, "an array of no length", at))?;
                let n = usize::try_from(n).ok().filter(|n| *n <= REPEAT);
                let n = n.ok_or_else(|| self.refuse(Refused::Value, format!("an array of one value repeated past {REPEAT}"), at))?;
                let op = self.operand(frame, op, at, before)?;
                Ok(Value::Aggregate(ty, None, vec![op; n]))
            }
            Rvalue::Ref(_, kind, p) => match kind {
                BorrowKind::Shared => Ok(Value::Ref(self.place(frame, p, at)?, false)),
                BorrowKind::Mut { .. } => Ok(Value::Ref(self.place(frame, p, at)?, true)),
                BorrowKind::Fake(_) => refused(self, "a fake borrow"),
            },
            Rvalue::Len(p) => {
                let ty = self.place_ty(frame, p, at)?;
                let TyKind::RigidTy(RigidTy::Array(_, len)) = ty.kind() else { return refused(self, "Len") };
                let n = len.eval_target_usize().map_err(|_| self.refuse(Refused::Value, "Len", at))?;
                let usize_ty = self.ty(Ty::usize_ty(), at)?;
                Ok(Value::Use(Operand::Const(Const {
                    ty: usize_ty,
                    slots: vec![orcl_interp::Scalar::Int(orcl_interp::Int::Usize, i128::from(n))],
                })))
            }
            Rvalue::Cast(kind, op, to) => match kind {
                CastKind::IntToInt | CastKind::FloatToInt | CastKind::FloatToFloat | CastKind::IntToFloat => {
                    let to = self.ty(*to, at)?;
                    Ok(Value::Cast(self.operand(frame, op, at, before)?, to))
                }
                CastKind::Subtype => Ok(Value::Use(self.operand(frame, op, at, before)?)),
                other => refused(self, &format!("Cast({other:?})")),
            },
            Rvalue::BinaryOp(op, a, b) => {
                let Some(op) = bin_op(*op) else { return refused(self, &format!("{op:?}")) };
                Ok(Value::Binary(op, self.operand(frame, a, at, before)?, self.operand(frame, b, at, before)?))
            }
            Rvalue::CheckedBinaryOp(op, a, b) => match bin_op(*op) {
                Some(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul)) => {
                    // the result's type, `(T, bool)`, is the destination's, lowered with the body's locals
                    Ok(Value::Checked(op, self.operand(frame, a, at, before)?, self.operand(frame, b, at, before)?))
                }
                _ => refused(self, &format!("Checked({op:?})")),
            },
            Rvalue::UnaryOp(op, a) => match op {
                mir::UnOp::Not => Ok(Value::Unary(UnOp::Not, self.operand(frame, a, at, before)?)),
                mir::UnOp::Neg => Ok(Value::Unary(UnOp::Neg, self.operand(frame, a, at, before)?)),
                mir::UnOp::PtrMetadata => refused(self, "PtrMetadata"),
            },
            Rvalue::Discriminant(p) => Ok(Value::Discriminant(self.place(frame, p, at)?)),
            Rvalue::Aggregate(kind, ops) => {
                let ty = rvalue.ty(frame.body.locals()).map_err(|_| self.refuse(Refused::Value, "an aggregate of no type", at))?;
                let variant = match kind {
                    AggregateKind::Array(_) | AggregateKind::Tuple => None,
                    AggregateKind::Adt(def, v, _, _, None) => match def.kind() {
                        rustc_public::ty::AdtKind::Enum => Some(
                            def.variants_iter()
                                .position(|x| x.idx() == *v)
                                .and_then(|i| u32::try_from(i).ok())
                                .ok_or_else(|| self.refuse(Refused::Value, "no such variant", at))?,
                        ),
                        _ => None,
                    },
                    AggregateKind::Adt(..) => return refused(self, "a union's field"),
                    AggregateKind::Closure(..) => return Err(self.refuse(Refused::Type, format!("{ty}"), at)),
                    AggregateKind::Coroutine(..) | AggregateKind::CoroutineClosure(..) => {
                        return Err(self.refuse(Refused::Type, format!("{ty}"), at));
                    }
                    AggregateKind::RawPtr(..) => return Err(self.refuse(Refused::Type, format!("{ty}"), at)),
                };
                let ty = self.ty(ty, at)?;
                let ops = self.operands(frame, ops, at, before)?;
                Ok(Value::Aggregate(ty, variant, ops))
            }
            Rvalue::AddressOf(..) => refused(self, "AddressOf"),
            Rvalue::ThreadLocalRef(_) => refused(self, "ThreadLocalRef"),
            Rvalue::Reborrow(..) => refused(self, "Reborrow"),
        }
    }

    fn check(&self, message: &AssertMessage, at: Span) -> Result<Check, Refusal> {
        match message {
            AssertMessage::Overflow(op, ..) => match bin_op(*op) {
                Some(op) => Ok(Check::Overflow(op)),
                None => Err(self.refuse(Refused::Terminator, format!("Assert(Overflow({op:?}))"), at)),
            },
            AssertMessage::OverflowNeg(_) => Ok(Check::NegOverflow),
            AssertMessage::DivisionByZero(_) => Ok(Check::DivisionByZero),
            AssertMessage::RemainderByZero(_) => Ok(Check::RemainderByZero),
            AssertMessage::BoundsCheck { .. } => Ok(Check::Bounds),
            other => Err(self.refuse(Refused::Terminator, format!("Assert({})", other.description().unwrap_or("?")), at)),
        }
    }

    fn terminator(
        &mut self,
        frame: &mut Frame<'_>,
        terminator: &mir::Terminator,
        before: &mut Vec<Statement>,
    ) -> Result<Terminator, Refusal> {
        let at = terminator.source_info.span;
        match &terminator.kind {
            TerminatorKind::Goto { target } => Ok(Terminator::Goto(self.block(frame, *target, at)?)),
            TerminatorKind::SwitchInt { discr, targets } => {
                let value = self.operand(frame, discr, at, before)?;
                let ty = discr.ty(frame.body.locals()).map_err(|_| self.refuse(Refused::Value, "a switch of no type", at))?;
                let ty = self.ty(ty, at)?;
                let wrap = |v: u128| match self.get(ty) {
                    Some(Type::Int(t)) => t.wrap(v as i128),
                    _ => v as i128,
                };
                let mut out = Vec::new();
                for (v, b) in targets.branches() {
                    out.push((wrap(v), self.block(frame, b, at)?));
                }
                let otherwise = self.block(frame, targets.otherwise(), at)?;
                Ok(Terminator::SwitchInt { value, targets: out, otherwise, at: self.span(at) })
            }
            TerminatorKind::Return => Ok(Terminator::Return { at: self.span(at) }),
            TerminatorKind::Unreachable => Ok(Terminator::Unreachable { at: self.span(at) }),
            TerminatorKind::Drop { place, target, .. } => {
                let ty = self.place_ty(frame, place, at)?;
                if Instance::resolve_drop_in_place(ty).is_empty_shim() {
                    Ok(Terminator::Goto(self.block(frame, *target, at)?))
                } else {
                    Err(self.refuse(Refused::DropGlue, format!("{ty}"), at))
                }
            }
            TerminatorKind::Assert { cond, expected, msg, target, .. } => {
                let check = self.check(msg, at)?;
                let cond = self.operand(frame, cond, at, before)?;
                let next = self.block(frame, *target, at)?;
                Ok(Terminator::Assert { cond, expected: *expected, check, next, at: self.span(at) })
            }
            TerminatorKind::Call { func, args, destination, target, .. } => {
                let callee = self.callee(func, at)?;
                let Some(target) = target else {
                    return Err(self.refuse(Refused::Terminator, "a call that does not return", at));
                };
                let args = self.operands(frame, args, at, before)?;
                let dest = self.place(frame, destination, at)?;
                let next = self.block(frame, *target, at)?;
                Ok(Terminator::Call { callee, args, dest, next, at: self.span(at) })
            }
            TerminatorKind::Resume => Err(self.refuse(Refused::Terminator, "UnwindResume", at)),
            TerminatorKind::Abort => Err(self.refuse(Refused::Terminator, "UnwindTerminate", at)),
            TerminatorKind::InlineAsm { .. } => Err(self.refuse(Refused::Terminator, "InlineAsm", at)),
        }
    }

    /// What a call calls: a function of the program's crates, through `Instance::resolve`, or an operation of the
    /// closed list, by the definition it calls.
    fn callee(&mut self, func: &mir::Operand, at: Span) -> Result<Callee, Refusal> {
        let mir::Operand::Constant(c) = func else {
            return Err(self.refuse(Refused::Call, "a call through a function pointer", at));
        };
        let TyKind::RigidTy(RigidTy::FnDef(def, args)) = c.ty().kind() else {
            return Err(self.refuse(Refused::Call, format!("a call through {}", c.ty()), at));
        };
        let Ok(instance) = Instance::resolve(def, &args) else {
            return Err(self.refuse(Refused::Call, self.defs.call_name(def, &args, None), at));
        };
        let name = || self.defs.call_name(def, &args, Some(&instance));
        if Defs::program_crate(&instance.def.krate()) {
            // a foreign function the program declares has no MIR to lower, whatever the flags its crate was built with
            return match instance.kind {
                InstanceKind::Item if !instance.is_foreign_item() => Ok(Callee::Function(self.function_of(instance))),
                _ => Err(self.refuse(Refused::Call, name(), at)),
            };
        }
        if matches!(instance.kind, InstanceKind::Virtual { .. }) || def.krate().name != "core" {
            return Err(self.refuse(Refused::Call, name(), at));
        }
        match self.defs.core_op(def, &args) {
            Some(op) => Ok(Callee::Core(op)),
            None => Err(self.refuse(Refused::CoreCall, name(), at)),
        }
    }
}
