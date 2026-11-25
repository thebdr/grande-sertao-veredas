//! The closed list of core-library operations a program may call (`docs/questions.md` Q227): each is the
//! interpreter's own, computed natively, and a call into core outside the list is refused when the program is
//! lowered. The list starts with what `orcl-std`'s blocks and the programs studied reach (`docs/orcl/front-end.md`)
//! and grows one operation at a time, each with the conformance programs that prove it against core's.
use crate::program::{BinOp, Int, Type, TypeId};

/// One operation of the list, with the types it is called at where core's function is generic or defined for each
/// integer type. Arguments are passed as MIR passes them to core's function: by value, or by reference where the
/// function takes `&self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoreOp {
    /// `<int>::saturating_add(self, rhs)`.
    SaturatingAdd(Int),
    /// `<int>::saturating_sub(self, rhs)`.
    SaturatingSub(Int),
    /// `<int>::saturating_mul(self, rhs)`.
    SaturatingMul(Int),
    /// `<int>::wrapping_add(self, rhs)`.
    WrappingAdd(Int),
    /// `<int>::wrapping_sub(self, rhs)`.
    WrappingSub(Int),
    /// `<int>::wrapping_mul(self, rhs)`.
    WrappingMul(Int),
    /// `<int>::checked_shl(self, rhs: u32) -> Option<int>`.
    CheckedShl(Int),
    /// `<int>::checked_shr(self, rhs: u32) -> Option<int>`.
    CheckedShr(Int),
    /// `Ord::min(self, other)` on an integer.
    Min(Int),
    /// `Ord::max(self, other)` on an integer.
    Max(Int),
    /// `Duration::from_millis(u64)`.
    DurationFromMillis,
    /// `Duration::from_secs(u64)`.
    DurationFromSecs,
    /// `Duration::saturating_sub(self, rhs)`.
    DurationSaturatingSub,
    /// `Ord::min(self, other)` on `Duration`.
    DurationMin,
    /// `Ord::max(self, other)` on `Duration`.
    DurationMax,
    /// `PartialEq::eq` and `ne`, `PartialOrd::lt`, `le`, `gt` and `ge` on `Duration`: `(&self, &other) -> bool`.
    DurationCompare(BinOp),
    /// `Option::<T>::get_or_insert(&mut self, value: T) -> &mut T`.
    OptionGetOrInsert,
    /// `Option::<T>::is_none(&self) -> bool`.
    OptionIsNone,
    /// `Option::<T>::is_some(&self) -> bool`.
    OptionIsSome,
    /// `Default::default()` of `bool`, an integer, a float or `Duration`: false, zero, zero, zero.
    Default,
}

/// The shape an `Option<T>` must have: an enum named `Option`, variant 0 holding nothing and variant 1 one field
/// of `T`. Its `T`, if so.
pub(crate) fn option_of(types: &[Type], ty: TypeId) -> Option<TypeId> {
    match types.get(ty.0 as usize)? {
        Type::Enum { name, variants, .. } if name == "Option" => match variants.as_slice() {
            [none, some] if none.fields.is_empty() && some.fields.len() == 1 && none.discriminant == 0 && some.discriminant == 1 => {
                some.fields.first().map(|f| f.ty)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether two types are one: the same index, or two scalar types alike (`bool`, one integer type, one float type,
/// `Duration`), which a program may list more than once.
pub(crate) fn same(types: &[Type], a: TypeId, b: TypeId) -> bool {
    a == b
        || match (types.get(a.0 as usize), types.get(b.0 as usize)) {
            (Some(x @ (Type::Bool | Type::Int(_) | Type::F32 | Type::F64 | Type::Duration)), Some(y)) => x == y,
            _ => false,
        }
}

fn is(types: &[Type], ty: TypeId, want: &Type) -> bool {
    types.get(ty.0 as usize) == Some(want)
}

fn reference(types: &[Type], ty: TypeId, mutable: bool) -> Option<TypeId> {
    match types.get(ty.0 as usize)? {
        Type::Ref { to, mutable: m } if *m == mutable => Some(*to),
        _ => None,
    }
}

impl CoreOp {
    /// Whether a call with arguments of `args`' types and a destination of `dest`'s matches the operation.
    pub(crate) fn accepts(self, types: &[Type], args: &[TypeId], dest: TypeId) -> bool {
        let int = |t: Int| Type::Int(t);
        match (self, args) {
            (
                CoreOp::SaturatingAdd(t)
                | CoreOp::SaturatingSub(t)
                | CoreOp::SaturatingMul(t)
                | CoreOp::WrappingAdd(t)
                | CoreOp::WrappingSub(t)
                | CoreOp::WrappingMul(t)
                | CoreOp::Min(t)
                | CoreOp::Max(t),
                [a, b],
            ) => is(types, *a, &int(t)) && is(types, *b, &int(t)) && is(types, dest, &int(t)),
            (CoreOp::CheckedShl(t) | CoreOp::CheckedShr(t), [a, b]) => {
                is(types, *a, &int(t))
                    && is(types, *b, &int(Int::U32))
                    && option_of(types, dest).is_some_and(|inner| is(types, inner, &int(t)))
            }
            (CoreOp::DurationFromMillis | CoreOp::DurationFromSecs, [a]) => {
                is(types, *a, &int(Int::U64)) && is(types, dest, &Type::Duration)
            }
            (CoreOp::DurationSaturatingSub | CoreOp::DurationMin | CoreOp::DurationMax, [a, b]) => {
                is(types, *a, &Type::Duration) && is(types, *b, &Type::Duration) && is(types, dest, &Type::Duration)
            }
            (CoreOp::DurationCompare(op), [a, b]) => {
                matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
                    && reference(types, *a, false).is_some_and(|t| is(types, t, &Type::Duration))
                    && reference(types, *b, false).is_some_and(|t| is(types, t, &Type::Duration))
                    && is(types, dest, &Type::Bool)
            }
            (CoreOp::OptionGetOrInsert, [a, b]) => match reference(types, *a, true).and_then(|o| option_of(types, o)) {
                Some(inner) => same(types, *b, inner) && reference(types, dest, true).is_some_and(|t| same(types, t, inner)),
                None => false,
            },
            (CoreOp::OptionIsNone | CoreOp::OptionIsSome, [a]) => {
                reference(types, *a, false).and_then(|o| option_of(types, o)).is_some() && is(types, dest, &Type::Bool)
            }
            (CoreOp::Default, []) => matches!(
                types.get(dest.0 as usize),
                Some(Type::Bool | Type::Int(_) | Type::F32 | Type::F64 | Type::Duration)
            ),
            _ => false,
        }
    }
}
