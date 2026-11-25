//! What one slot holds, and Rust's operations on it: MIR's binary and unary operations and `as`, each as the
//! compiled program computes it.
use crate::program::{BinOp, Check, Int, Type, UnOp};

/// One slot of the machine's memory: a scalar, a reference to a slot, or nothing written yet. An integer carries
/// its type and its value, always one of the type's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Uninit,
    Bool(bool),
    Int(Int, i128),
    F32(f32),
    F64(f64),
    /// The index of the slot a reference refers to.
    Ref(u32),
}

impl Scalar {
    /// An integer of type `ty`, or `None` when `value` is not one of the type's.
    pub const fn int(ty: Int, value: i128) -> Option<Scalar> {
        if ty.holds(value) { Some(Scalar::Int(ty, value)) } else { None }
    }
}

/// Why an operation gave no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    /// The program's own failure: what Rust would panic on.
    Check(Check),
    /// A slot read before anything was written to it.
    Uninit,
    /// Operands of types the operation does not take: a program the loader should have refused.
    Mismatch,
}

fn int_of(a: Scalar) -> Result<(Int, i128), Fail> {
    match a {
        Scalar::Int(t, v) => Ok((t, v)),
        Scalar::Uninit => Err(Fail::Uninit),
        _ => Err(Fail::Mismatch),
    }
}

/// The exact result of `+ - *` on two values of one integer type, or `None` past `i128`.
fn exact(op: BinOp, a: i128, b: i128) -> Option<i128> {
    match op {
        BinOp::Add => a.checked_add(b),
        BinOp::Sub => a.checked_sub(b),
        BinOp::Mul => a.checked_mul(b),
        _ => None,
    }
}

/// `+ - *` wrapped into `ty`, as two's complement gives it.
fn wrapping(op: BinOp, ty: Int, a: i128, b: i128) -> Result<i128, Fail> {
    let (x, y) = (a as u128, b as u128);
    let low = match op {
        BinOp::Add => x.wrapping_add(y),
        BinOp::Sub => x.wrapping_sub(y),
        BinOp::Mul => x.wrapping_mul(y),
        _ => return Err(Fail::Mismatch),
    };
    Ok(ty.wrap(low as i128))
}

/// MIR's checked form: the wrapped result and whether the exact one is outside the type.
pub(crate) fn checked(op: BinOp, a: Scalar, b: Scalar) -> Result<(Scalar, bool), Fail> {
    let ((ta, x), (tb, y)) = (int_of(a)?, int_of(b)?);
    if ta != tb || !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
        return Err(Fail::Mismatch);
    }
    let wrapped = wrapping(op, ta, x, y)?;
    let overflowed = exact(op, x, y).is_none_or(|v| !ta.holds(v));
    Ok((Scalar::Int(ta, wrapped), overflowed))
}

fn compare<T: PartialOrd>(op: BinOp, a: T, b: T) -> Result<bool, Fail> {
    Ok(match op {
        BinOp::Eq => a == b,
        BinOp::Ne => a != b,
        BinOp::Lt => a < b,
        BinOp::Le => a <= b,
        BinOp::Gt => a > b,
        BinOp::Ge => a >= b,
        _ => return Err(Fail::Mismatch),
    })
}

const fn is_comparison(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// MIR's plain binary operation.
pub(crate) fn binary(op: BinOp, a: Scalar, b: Scalar) -> Result<Scalar, Fail> {
    match (a, b) {
        (Scalar::Uninit, _) | (_, Scalar::Uninit) => Err(Fail::Uninit),
        (Scalar::Int(ta, x), Scalar::Int(tb, y)) => int_binary(op, ta, x, tb, y),
        (Scalar::Bool(x), Scalar::Bool(y)) => match op {
            BinOp::BitAnd => Ok(Scalar::Bool(x & y)),
            BinOp::BitOr => Ok(Scalar::Bool(x | y)),
            BinOp::BitXor => Ok(Scalar::Bool(x ^ y)),
            _ => compare(op, x, y).map(Scalar::Bool),
        },
        (Scalar::F32(x), Scalar::F32(y)) => match op {
            BinOp::Add => Ok(Scalar::F32(x + y)),
            BinOp::Sub => Ok(Scalar::F32(x - y)),
            BinOp::Mul => Ok(Scalar::F32(x * y)),
            BinOp::Div => Ok(Scalar::F32(x / y)),
            BinOp::Rem => Ok(Scalar::F32(x % y)),
            _ => compare(op, x, y).map(Scalar::Bool),
        },
        (Scalar::F64(x), Scalar::F64(y)) => match op {
            BinOp::Add => Ok(Scalar::F64(x + y)),
            BinOp::Sub => Ok(Scalar::F64(x - y)),
            BinOp::Mul => Ok(Scalar::F64(x * y)),
            BinOp::Div => Ok(Scalar::F64(x / y)),
            BinOp::Rem => Ok(Scalar::F64(x % y)),
            _ => compare(op, x, y).map(Scalar::Bool),
        },
        _ => Err(Fail::Mismatch),
    }
}

fn int_binary(op: BinOp, ta: Int, x: i128, tb: Int, y: i128) -> Result<Scalar, Fail> {
    if matches!(op, BinOp::Shl | BinOp::Shr) {
        // MIR's shift: the amount is the right operand modulo the left operand's width, whatever its type.
        let amount = y.checked_rem_euclid(i128::from(ta.bits())).ok_or(Fail::Mismatch)? as u32;
        let value = match op {
            BinOp::Shl => ta.wrap((x as u128).wrapping_shl(amount) as i128),
            _ => x.wrapping_shr(amount),
        };
        return Ok(Scalar::Int(ta, value));
    }
    if ta != tb {
        return Err(Fail::Mismatch);
    }
    if is_comparison(op) {
        return compare(op, x, y).map(Scalar::Bool);
    }
    let value = match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul => wrapping(op, ta, x, y)?,
        BinOp::Div | BinOp::Rem => {
            let zero = if op == BinOp::Div { Check::DivisionByZero } else { Check::RemainderByZero };
            if y == 0 {
                return Err(Fail::Check(zero));
            }
            if ta.signed() && x == ta.min() && y == -1 {
                return Err(Fail::Check(Check::Overflow(op)));
            }
            let exact = if op == BinOp::Div { x.checked_div(y) } else { x.checked_rem(y) };
            exact.ok_or(Fail::Mismatch)?
        }
        BinOp::BitAnd => ta.wrap(x & y),
        BinOp::BitOr => ta.wrap(x | y),
        BinOp::BitXor => ta.wrap(x ^ y),
        _ => return Err(Fail::Mismatch),
    };
    Ok(Scalar::Int(ta, value))
}

/// MIR's unary operation: `!` on a `bool` or within an integer's width, `-` wrapping on a signed integer, and on a
/// float.
pub(crate) fn unary(op: UnOp, a: Scalar) -> Result<Scalar, Fail> {
    match (op, a) {
        (_, Scalar::Uninit) => Err(Fail::Uninit),
        (UnOp::Not, Scalar::Bool(x)) => Ok(Scalar::Bool(!x)),
        (UnOp::Not, Scalar::Int(t, x)) => Ok(Scalar::Int(t, t.wrap(!x))),
        (UnOp::Neg, Scalar::Int(t, x)) if t.signed() => {
            Ok(Scalar::Int(t, t.wrap(x.checked_neg().ok_or(Fail::Mismatch)?)))
        }
        (UnOp::Neg, Scalar::F32(x)) => Ok(Scalar::F32(-x)),
        (UnOp::Neg, Scalar::F64(x)) => Ok(Scalar::F64(-x)),
        _ => Err(Fail::Mismatch),
    }
}

/// A float's value as Rust's `as` gives it to an integer type: toward zero, saturating at the type's bounds, NaN
/// as zero.
fn float_to_int(value: f64, to: Int) -> i128 {
    if value.is_nan() {
        0
    } else {
        (value as i128).clamp(to.min(), to.max())
    }
}

/// Rust's `as` between `bool`, the integers and the floats.
pub(crate) fn cast(a: Scalar, to: &Type) -> Result<Scalar, Fail> {
    match (a, to) {
        (Scalar::Uninit, _) => Err(Fail::Uninit),
        (Scalar::Bool(x), Type::Bool) => Ok(Scalar::Bool(x)),
        (Scalar::Bool(x), Type::Int(t)) => Ok(Scalar::Int(*t, i128::from(x))),
        (Scalar::Int(_, x), Type::Int(t)) => Ok(Scalar::Int(*t, t.wrap(x))),
        (Scalar::Int(_, x), Type::F32) => Ok(Scalar::F32(x as f32)),
        (Scalar::Int(_, x), Type::F64) => Ok(Scalar::F64(x as f64)),
        // an `f32` widened to `f64` is exact, so the conversion below is the `f32`'s own
        (Scalar::F32(x), Type::Int(t)) => Ok(Scalar::Int(*t, float_to_int(f64::from(x), *t))),
        (Scalar::F64(x), Type::Int(t)) => Ok(Scalar::Int(*t, float_to_int(x, *t))),
        (Scalar::F32(x), Type::F32) => Ok(Scalar::F32(x)),
        (Scalar::F32(x), Type::F64) => Ok(Scalar::F64(f64::from(x))),
        (Scalar::F64(x), Type::F32) => Ok(Scalar::F32(x as f32)),
        (Scalar::F64(x), Type::F64) => Ok(Scalar::F64(x)),
        _ => Err(Fail::Mismatch),
    }
}
