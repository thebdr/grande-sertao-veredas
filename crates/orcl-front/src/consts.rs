//! Constants, decoded from the bytes the compiler evaluated them to, by their type's layout: an integer's bytes in
//! the target's order, a float's bits, a `Duration`'s seconds and nanoseconds, a structure's fields at their
//! offsets, an enum's variant from its tag (direct or in a niche) and that variant's fields. A shared reference to a
//! constant (a promoted `&State::Stopping`) is the constant it refers to, which the body holds in a local of its own.
use std::ops::Range;

use orcl_interp::{Const, Scalar, Type, TypeId};
use rustc_public::abi::{FieldsShape, LayoutShape, Primitive, TagEncoding, VariantsShape};
use rustc_public::mir::Mutability;
use rustc_public::mir::alloc::GlobalAlloc;
use rustc_public::CrateDefType;
use rustc_public::ty::{Allocation, ConstantKind, MirConst, RigidTy, Span, Ty, TyConstKind, TyKind};

use crate::error::{Refusal, Refused};
use crate::lower::Lowerer;

/// A constant operand as the interpreter takes it: a value, or a shared reference to one.
pub(crate) enum Lit {
    Value(Const),
    RefTo(Const),
}

fn mask(bits: usize) -> u128 {
    if bits >= 128 { u128::MAX } else { (1u128 << bits).wrapping_sub(1) }
}

impl Lowerer<'_> {
    fn bad_const(&self, what: &str, at: Span) -> Refusal {
        self.refuse(Refused::Value, what, at)
    }

    pub(crate) fn constant(&mut self, c: &MirConst, at: Span) -> Result<Lit, Refusal> {
        let ty = c.ty();
        match c.kind() {
            ConstantKind::ZeroSized => {
                let id = self.ty(ty, at)?;
                Ok(Lit::Value(Const { ty: id, slots: Vec::new() }))
            }
            ConstantKind::Allocated(alloc) => self.allocated(ty, alloc, at),
            ConstantKind::Ty(tc) => match tc.kind() {
                TyConstKind::Value(t, alloc) => self.allocated(*t, alloc, at),
                TyConstKind::ZSTValue(t) => {
                    let id = self.ty(*t, at)?;
                    Ok(Lit::Value(Const { ty: id, slots: Vec::new() }))
                }
                _ => Err(self.bad_const("a constant not evaluated", at)),
            },
            ConstantKind::Unevaluated(_) | ConstantKind::Param(_) => Err(self.bad_const("a constant not evaluated", at)),
        }
    }

    fn allocated(&mut self, ty: Ty, alloc: &Allocation, at: Span) -> Result<Lit, Refusal> {
        if let TyKind::RigidTy(RigidTy::Ref(_, to, mutability)) = ty.kind() {
            let (Mutability::Not, [(0, prov)]) = (mutability, alloc.provenance.ptrs.as_slice()) else {
                return Err(self.bad_const(&format!("a constant {ty}"), at));
            };
            let offset = alloc.read_uint().ok().and_then(|o| usize::try_from(o).ok());
            let (Some(offset), GlobalAlloc::Memory(target)) = (offset, GlobalAlloc::from(prov.0)) else {
                return Err(self.bad_const(&format!("a reference to a static, {ty}"), at));
            };
            let id = self.ty(to, at)?;
            let slots = self.decode(to, id, &target, offset, at)?;
            return Ok(Lit::RefTo(Const { ty: id, slots }));
        }
        let id = self.ty(ty, at)?;
        if !alloc.provenance.ptrs.is_empty() {
            return Err(self.bad_const(&format!("a constant holding a reference, {ty}"), at));
        }
        let slots = self.decode(ty, id, alloc, 0, at)?;
        Ok(Lit::Value(Const { ty: id, slots }))
    }

    fn layout(&self, ty: Ty, at: Span) -> Result<LayoutShape, Refusal> {
        ty.layout().map(|l| l.shape()).map_err(|_| self.bad_const(&format!("a constant {ty} with no layout"), at))
    }

    fn read(&self, alloc: &Allocation, range: Range<usize>, at: Span) -> Result<u128, Refusal> {
        alloc.read_partial_uint(range).map_err(|_| self.bad_const("a constant's bytes past its allocation", at))
    }

    /// The slots of a value of `ty` (lowered as `id`) whose bytes start at `base` in `alloc`.
    fn decode(&mut self, ty: Ty, id: TypeId, alloc: &Allocation, base: usize, at: Span) -> Result<Vec<Scalar>, Refusal> {
        let mut out = Vec::new();
        self.decode_into(ty, id, alloc, base, at, &mut out)?;
        Ok(out)
    }

    fn offsets(&self, shape: &LayoutShape, ty: Ty, at: Span) -> Result<Vec<usize>, Refusal> {
        match &shape.fields {
            FieldsShape::Arbitrary { offsets } => Ok(offsets.iter().map(|o| o.bytes()).collect()),
            _ => Err(self.bad_const(&format!("a constant {ty} laid out otherwise"), at)),
        }
    }

    fn decode_into(
        &mut self,
        ty: Ty,
        id: TypeId,
        alloc: &Allocation,
        base: usize,
        at: Span,
        out: &mut Vec<Scalar>,
    ) -> Result<(), Refusal> {
        let lowered = self.get(id).cloned().ok_or_else(|| self.bad_const("a constant of no type", at))?;
        let end = |n: usize| base.saturating_add(n);
        match lowered {
            Type::Bool => match self.read(alloc, base..end(1), at)? {
                0 => out.push(Scalar::Bool(false)),
                1 => out.push(Scalar::Bool(true)),
                _ => return Err(self.bad_const("a bool that is neither", at)),
            },
            Type::Int(t) => {
                let bits = t.bits() as usize;
                let raw = self.read(alloc, base..end(bits / 8), at)?;
                let value = if t.signed() { t.wrap(raw as i128) } else { raw as i128 };
                out.push(Scalar::Int(t, value));
            }
            Type::F32 => out.push(Scalar::F32(f32::from_bits(self.read(alloc, base..end(4), at)? as u32))),
            Type::F64 => out.push(Scalar::F64(f64::from_bits(self.read(alloc, base..end(8), at)? as u64))),
            Type::Duration => {
                let shape = self.layout(ty, at)?;
                let offsets = self.offsets(&shape, ty, at)?;
                let (Some(secs), Some(nanos)) = (offsets.first(), offsets.get(1)) else {
                    return Err(self.bad_const("a Duration laid out otherwise", at));
                };
                let secs = self.read(alloc, base.saturating_add(*secs)..end(*secs + 8), at)?;
                let nanos = self.read(alloc, base.saturating_add(*nanos)..end(*nanos + 4), at)?;
                out.push(Scalar::Int(orcl_interp::Int::U64, secs as i128));
                out.push(Scalar::Int(orcl_interp::Int::U32, nanos as i128));
            }
            Type::Ref { .. } => return Err(self.bad_const(&format!("a constant holding a reference, {ty}"), at)),
            Type::Array { element, len } => {
                let TyKind::RigidTy(RigidTy::Array(element_ty, _)) = ty.kind() else {
                    return Err(self.bad_const(&format!("a constant {ty}"), at));
                };
                let shape = self.layout(ty, at)?;
                let FieldsShape::Array { stride, .. } = shape.fields else {
                    return Err(self.bad_const(&format!("a constant {ty} laid out otherwise"), at));
                };
                for i in 0..len as usize {
                    let at_byte = base.saturating_add(i.saturating_mul(stride.bytes()));
                    self.decode_into(element_ty, element, alloc, at_byte, at, out)?;
                }
            }
            Type::Struct { fields, .. } => {
                let parts = self.parts(ty, 0, at)?;
                let shape = self.layout(ty, at)?;
                let offsets = self.offsets(&shape, ty, at)?;
                for ((field, part), offset) in fields.iter().zip(parts).zip(offsets) {
                    self.decode_into(part, field.ty, alloc, base.saturating_add(offset), at, out)?;
                }
            }
            Type::Enum { discriminant, variants, .. } => {
                let shape = self.layout(ty, at)?;
                let (v, offsets) = self.variant_of(ty, &shape, alloc, base, at)?;
                let variant = variants.get(v).ok_or_else(|| self.bad_const("an enum's variant past its variants", at))?;
                out.push(Scalar::Int(discriminant, variant.discriminant));
                let start = out.len();
                let parts = self.parts(ty, v, at)?;
                for ((field, part), offset) in variant.fields.iter().zip(parts).zip(offsets) {
                    self.decode_into(part, field.ty, alloc, base.saturating_add(offset), at, out)?;
                }
                // the slots past the variant's fields are unwritten, as the interpreter holds an enum
                let size = self.slots_of(id) as usize;
                let written = out.len().saturating_sub(start).saturating_add(1);
                out.extend(std::iter::repeat_n(Scalar::Uninit, size.saturating_sub(written)));
            }
        }
        Ok(())
    }

    /// The types of a structure's or a tuple's fields, or of an enum's variant `v`'s.
    fn parts(&self, ty: Ty, v: usize, at: Span) -> Result<Vec<Ty>, Refusal> {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Tuple(parts)) => Ok(parts),
            TyKind::RigidTy(RigidTy::Adt(def, args)) => {
                let variant = def.variants_iter().nth(v).ok_or_else(|| self.bad_const("no such variant", at))?;
                Ok(variant.fields().iter().map(|f| f.ty_with_args(&args)).collect())
            }
            _ => Err(self.bad_const(&format!("a constant {ty}"), at)),
        }
    }

    /// The variant an enum's bytes hold, by its index, and its fields' offsets.
    fn variant_of(
        &self,
        ty: Ty,
        shape: &LayoutShape,
        alloc: &Allocation,
        base: usize,
        at: Span,
    ) -> Result<(usize, Vec<usize>), Refusal> {
        let TyKind::RigidTy(RigidTy::Adt(def, _)) = ty.kind() else {
            return Err(self.bad_const(&format!("a constant {ty}"), at));
        };
        let position = |idx| def.variants_iter().position(|v| v.idx() == idx);
        match &shape.variants {
            VariantsShape::Single { index } => {
                let v = position(*index).ok_or_else(|| self.bad_const("no such variant", at))?;
                Ok((v, self.offsets(shape, ty, at)?))
            }
            VariantsShape::Multiple { tag, tag_encoding, tag_field, variants } => {
                let tag_offset = *self.offsets(shape, ty, at)?.get(*tag_field).ok_or_else(|| self.bad_const("no tag", at))?;
                let rustc_public::abi::Scalar::Initialized { value: Primitive::Int { length, .. }, .. } = tag else {
                    return Err(self.bad_const("a tag that is no integer", at));
                };
                let bits = length.bits();
                let from = base.saturating_add(tag_offset);
                let raw = self.read(alloc, from..from.saturating_add(bits / 8), at)?;
                let v = match tag_encoding {
                    TagEncoding::Direct => def
                        .variants_iter()
                        .position(|v| def.discriminant_for_variant(v.idx()).val & mask(bits) == raw),
                    TagEncoding::Niche { untagged_variant, niche_variants, niche_start } => {
                        let (Some(start), Some(last), Some(untagged)) =
                            (position(*niche_variants.start()), position(*niche_variants.end()), position(*untagged_variant))
                        else {
                            return Err(self.bad_const("no such variant", at));
                        };
                        let relative = raw.wrapping_sub(*niche_start) & mask(bits);
                        let span = last.saturating_sub(start) as u128;
                        Some(if relative <= span { start.saturating_add(relative as usize) } else { untagged })
                    }
                };
                let v = v.ok_or_else(|| self.bad_const("an enum's tag of no variant", at))?;
                let offsets = variants
                    .get(v)
                    .map(|f| f.offsets.iter().map(|o| o.bytes()).collect())
                    .ok_or_else(|| self.bad_const("no such variant", at))?;
                Ok((v, offsets))
            }
            VariantsShape::Empty => Err(self.bad_const(&format!("a constant {ty} of no variant"), at)),
        }
    }
}
