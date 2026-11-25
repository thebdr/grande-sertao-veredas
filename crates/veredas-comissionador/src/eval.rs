//! Values and expressions (`docs/test-schema.md`, "Expressions"): the types a test computes with,
//! the arithmetic and the comparisons across them, string interpolation, the built-in calls.
//!
//! Evaluation is pure: plant data and variables come from an [`Env`] the engine hands in, so a
//! script box computes the same in a live run and in its replay.

use std::fmt;
use std::sync::Arc;

use crate::model::{BinOp, CmpOp, Expr, ExprKind, StrPart, UnOp};

/// A row an `await` took: its columns by name, and what portal stores with every row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub table: String,
    /// The row's stamp on the store's axis.
    pub time_ms: i64,
    pub arrival_ms: i64,
    pub source: String,
    pub key: Option<String>,
    pub seq: i64,
    /// The value columns in the table's order, a hole `None`.
    pub columns: Vec<(String, Option<Value>)>,
}

impl Row {
    /// A column by name, or one of the fields every row has.
    pub fn field(&self, name: &str) -> Option<Option<Value>> {
        if let Some((_, v)) = self.columns.iter().find(|(n, _)| n == name) {
            return Some(v.clone());
        }
        Some(Some(match name {
            "time" => Value::Time(self.time_ms),
            "arrival" => Value::Time(self.arrival_ms),
            "source" => Value::Text(Arc::from(self.source.as_str())),
            "key" => return Some(self.key.as_ref().map(|k| Value::Text(Arc::from(k.as_str())))),
            "seq" => Value::Int(self.seq),
            _ => return None,
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Arc<str>),
    /// A moment on the store's axis, in milliseconds.
    Time(i64),
    /// Whole milliseconds.
    Duration(i64),
    Row(Arc<Row>),
}

impl Value {
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "a whole number",
            Value::Float(_) => "a number",
            Value::Bool(_) => "a truth value",
            Value::Text(_) => "a text",
            Value::Time(_) => "a moment",
            Value::Duration(_) => "a duration",
            Value::Row(_) => "a row",
        }
    }
}

/// A duration as the language writes one: the largest unit that divides it exactly.
pub fn duration_text(ms: i64) -> String {
    let (sign, a) = if ms < 0 { ("-", -(ms as i128)) } else { ("", ms as i128) };
    let body = if a != 0 && a % 3_600_000 == 0 {
        format!("{}h", a / 3_600_000)
    } else if a != 0 && a % 60_000 == 0 {
        format!("{}min", a / 60_000)
    } else if a % 1000 == 0 {
        format!("{}s", a / 1000)
    } else {
        format!("{a}ms")
    };
    format!("{sign}{body}")
}

/// A moment as text: the store's axis read as the plant's local clock, to the millisecond.
pub fn time_text(ms: i64) -> String {
    match chrono::DateTime::from_timestamp_millis(ms) {
        Some(t) => t.naive_utc().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        None => format!("{ms} ms"),
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Text(t) => f.write_str(t),
            Value::Time(ms) => f.write_str(&time_text(*ms)),
            Value::Duration(ms) => f.write_str(&duration_text(*ms)),
            Value::Row(r) => write!(f, "a row of `{}` at {}", r.table, time_text(r.time_ms)),
        }
    }
}

/// Why an expression has no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// Plant data with no row yet, a hole, or a name not assigned yet: what had no value.
    NoValue(String),
    /// A fault: two values that do not compare, a division by zero, a type a call does not take.
    Fault(String),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EvalError::NoValue(what) => write!(f, "{what} has no value yet"),
            EvalError::Fault(why) => f.write_str(why),
        }
    }
}

/// Where an expression's names find their values.
pub trait Env {
    /// A variable, `None` when not assigned yet.
    fn var(&self, name: &str) -> Option<Value>;
    /// Plant data: a column's latest value at the run's clock.
    fn plant(&mut self, table: &str, column: &str) -> Result<Value, EvalError>;
    /// The run's clock.
    fn now(&self) -> i64;
}

type E = Result<Value, EvalError>;

fn fault(msg: impl Into<String>) -> EvalError {
    EvalError::Fault(msg.into())
}

/// Evaluate an expression.
pub fn eval(e: &Expr, env: &mut dyn Env) -> E {
    match &e.kind {
        ExprKind::Int(i) => Ok(Value::Int(*i)),
        ExprKind::Float(x) => Ok(Value::Float(*x)),
        ExprKind::Bool(b) => Ok(Value::Bool(*b)),
        ExprKind::Duration(ms) => Ok(Value::Duration(*ms)),
        ExprKind::Str(parts) => {
            let mut s = String::new();
            for p in parts {
                match p {
                    StrPart::Text(t) => s.push_str(t),
                    StrPart::Expr(x) => s.push_str(&eval(x, env)?.to_string()),
                }
            }
            Ok(Value::Text(Arc::from(s)))
        }
        ExprKind::Name(n) => env.var(n).ok_or_else(|| EvalError::NoValue(format!("`{n}`"))),
        ExprKind::Field { base, field } => match env.var(&base.value) {
            Some(Value::Row(row)) => match row.field(&field.value) {
                Some(Some(v)) => Ok(v),
                Some(None) => Err(EvalError::NoValue(format!("`{}.{}`", base.value, field.value))),
                None => Err(fault(format!("a row of `{}` has no column `{}`", row.table, field.value))),
            },
            Some(other) => Err(fault(format!("`{}` is {}, not a row: it has no column `{}`", base.value, other.kind(), field.value))),
            None => Err(EvalError::NoValue(format!("`{}`", base.value))),
        },
        ExprKind::Plant { table, column } => env.plant(&table.value, &column.value),
        ExprKind::Call { name, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                vals.push(eval(a, env)?);
            }
            builtin(&name.value, &vals, env.now())
        }
        ExprKind::Unary { op: UnOp::Not, expr } => match eval(expr, env)? {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            v => Err(fault(format!("`not` takes a truth value, not {}", v.kind()))),
        },
        ExprKind::Unary { op: UnOp::Neg, expr } => match eval(expr, env)? {
            Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(|| fault("a whole number past its range")),
            Value::Float(x) => Ok(Value::Float(-x)),
            Value::Duration(d) => d.checked_neg().map(Value::Duration).ok_or_else(|| fault("a duration past its range")),
            v => Err(fault(format!("`-` takes a number or a duration, not {}", v.kind()))),
        },
        ExprKind::Binary { op: BinOp::And, left, right } => {
            if !truth(&eval(left, env)?, "and")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(truth(&eval(right, env)?, "and")?))
        }
        ExprKind::Binary { op: BinOp::Or, left, right } => {
            if truth(&eval(left, env)?, "or")? {
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(truth(&eval(right, env)?, "or")?))
        }
        ExprKind::Binary { op: BinOp::Cmp(c), left, right } => {
            let (l, r) = (eval(left, env)?, eval(right, env)?);
            compare(*c, &l, &r).map(Value::Bool)
        }
        ExprKind::Binary { op, left, right } => {
            let (l, r) = (eval(left, env)?, eval(right, env)?);
            arith(*op, &l, &r)
        }
    }
}

fn truth(v: &Value, op: &str) -> Result<bool, EvalError> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(fault(format!("`{op}` takes truth values, not {}", other.kind()))),
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// An arithmetic operator over two values, as an expression applies it.
pub fn arith(op: BinOp, l: &Value, r: &Value) -> E {
    use Value::*;
    let sym = match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        _ => "?",
    };
    let bad = || fault(format!("`{sym}` does not take {} and {}", l.kind(), r.kind()));
    let range = || fault("a whole number past its range");
    let span = || fault("a duration or a moment past its range");
    match (op, l, r) {
        (BinOp::Div, _, _) if num(r) == Some(0.0) && num(l).is_some() => Err(fault("a division by zero")),
        (BinOp::Rem, Int(_), Int(0)) => Err(fault("a division by zero")),
        (BinOp::Add, Int(a), Int(b)) => a.checked_add(*b).map(Int).ok_or_else(range),
        (BinOp::Sub, Int(a), Int(b)) => a.checked_sub(*b).map(Int).ok_or_else(range),
        (BinOp::Mul, Int(a), Int(b)) => a.checked_mul(*b).map(Int).ok_or_else(range),
        (BinOp::Rem, Int(a), Int(b)) => Ok(Int(a.wrapping_rem(*b))),
        (BinOp::Div, Int(_) | Float(_), Int(_) | Float(_)) => Ok(Float(num(l).unwrap_or(0.0) / num(r).unwrap_or(1.0))),
        (BinOp::Add | BinOp::Sub | BinOp::Mul, Int(_) | Float(_), Int(_) | Float(_)) => {
            let (a, b) = (num(l).unwrap_or(0.0), num(r).unwrap_or(0.0));
            Ok(Float(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                _ => a * b,
            }))
        }
        (BinOp::Sub, Time(a), Time(b)) => a.checked_sub(*b).map(Duration).ok_or_else(span),
        (BinOp::Add, Time(a), Duration(d)) | (BinOp::Add, Duration(d), Time(a)) => a.checked_add(*d).map(Time).ok_or_else(span),
        (BinOp::Sub, Time(a), Duration(d)) => a.checked_sub(*d).map(Time).ok_or_else(span),
        (BinOp::Add, Duration(a), Duration(b)) => a.checked_add(*b).map(Duration).ok_or_else(span),
        (BinOp::Sub, Duration(a), Duration(b)) => a.checked_sub(*b).map(Duration).ok_or_else(span),
        (BinOp::Mul, Duration(d), Int(n)) | (BinOp::Mul, Int(n), Duration(d)) => d.checked_mul(*n).map(Duration).ok_or_else(span),
        (BinOp::Mul, Duration(d), Float(x)) | (BinOp::Mul, Float(x), Duration(d)) => times(*d, *x).map(Duration).ok_or_else(span),
        (BinOp::Div, Duration(_), Duration(0)) => Err(fault("a division by zero")),
        (BinOp::Div, Duration(a), Duration(b)) => Ok(Float(*a as f64 / *b as f64)),
        _ => Err(bad()),
    }
}

/// `d` times `x`, exactly, rounded once to the whole number nearest it (half away from zero, as
/// `round` does), when one holds it: a float is a whole mantissa times a power of two, so the
/// product is a whole number in 128 bits shifted, never a float's rounding of `d`.
fn times(d: i64, x: f64) -> Option<i64> {
    if !x.is_finite() {
        return None;
    }
    let bits = x.to_bits();
    let (field, frac) = (((bits >> 52) & 0x7ff) as i32, (bits & ((1u64 << 52) - 1)) as i128);
    let (mantissa, exp) = if field == 0 { (frac, -1074) } else { (frac | (1 << 52), field - 1075) };
    let mantissa = if bits >> 63 == 1 { -mantissa } else { mantissa };
    // |d| < 2^63 and the mantissa < 2^53: the product fits 128 bits.
    let n = d as i128 * mantissa;
    let exact = if exp >= 0 {
        if exp >= 64 {
            if n == 0 { 0 } else { return None }
        } else {
            n.checked_mul(1i128 << exp)?
        }
    } else {
        let k = (-exp) as u32;
        let a = n.unsigned_abs();
        let q = if k >= 120 {
            // |n| < 2^116: under a sixteenth.
            0
        } else {
            let (q, r) = (a >> k, a & ((1u128 << k) - 1));
            if r >= 1u128 << (k - 1) { q + 1 } else { q }
        };
        if n < 0 { -(q as i128) } else { q as i128 }
    };
    i64::try_from(exact).ok()
}

/// A float rounded to the whole number nearest it, when one holds it: never saturated.
fn whole(x: f64) -> Option<i64> {
    let r = x.round();
    // 2^63 is the first float past the range; every float below it in magnitude fits.
    (r.is_finite() && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&r)).then_some(r as i64)
}

/// A text read as a number, trimmed.
fn text_number(t: &str) -> Option<f64> {
    t.trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

/// Compare two values as the reference says.
pub fn compare(op: CmpOp, l: &Value, r: &Value) -> Result<bool, EvalError> {
    use std::cmp::Ordering;
    use Value::*;
    let ord: Option<Ordering> = match (l, r) {
        (Int(a), Int(b)) => Some(a.cmp(b)),
        (Int(_) | Float(_), Int(_) | Float(_)) => num_cmp(l, r),
        (Bool(a), Bool(b)) => Some(a.cmp(b)),
        (Bool(b), Int(i)) => Some((*b as i64).cmp(i)),
        (Int(i), Bool(b)) => Some(i.cmp(&(*b as i64))),
        (Text(a), Text(b)) => Some(a.cmp(b)),
        (Text(t), n @ (Int(_) | Float(_))) | (n @ (Int(_) | Float(_)), Text(t)) => {
            let Some(x) = text_numeric(t) else {
                return match op {
                    CmpOp::Eq => Ok(false),
                    CmpOp::Ne => Ok(true),
                    _ => Err(fault(format!("\"{t}\" is no number to order against {n}"))),
                };
            };
            // Keep the sides as written.
            if matches!(l, Text(_)) { num_cmp(&x, n) } else { num_cmp(n, &x) }
        }
        (Time(a), Time(b)) => Some(a.cmp(b)),
        (Duration(a), Duration(b)) => Some(a.cmp(b)),
        _ => return Err(fault(format!("{} and {} do not compare", l.kind(), r.kind()))),
    };
    let Some(o) = ord else {
        // A NaN: equal to nothing, ordered against nothing.
        return Ok(op == CmpOp::Ne);
    };
    Ok(match op {
        CmpOp::Eq => o == Ordering::Equal,
        CmpOp::Ne => o != Ordering::Equal,
        CmpOp::Lt => o == Ordering::Less,
        CmpOp::Le => o != Ordering::Greater,
        CmpOp::Gt => o == Ordering::Greater,
        CmpOp::Ge => o != Ordering::Less,
    })
}

/// Two numbers ordered exactly: a whole number against a float never through the float's 53 bits.
fn num_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use Value::*;
    match (a, b) {
        (Int(x), Int(y)) => Some(x.cmp(y)),
        (Float(x), Float(y)) => x.partial_cmp(y),
        (Int(x), Float(y)) => int_float(*x, *y),
        (Float(x), Int(y)) => int_float(*y, *x).map(std::cmp::Ordering::reverse),
        _ => None,
    }
}

/// A whole number against a float: the float's whole part compared as a whole number, then its
/// fraction; past the 64 bits, the float's side.
fn int_float(i: i64, f: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    if f.is_nan() {
        return None;
    }
    if f >= 9_223_372_036_854_775_808.0 {
        return Some(Less);
    }
    if f < -9_223_372_036_854_775_808.0 {
        return Some(Greater);
    }
    let whole = f.trunc();
    Some(i.cmp(&(whole as i64)).then(if f > whole { Less } else if f < whole { Greater } else { Equal }))
}

/// A text read as a number: a whole number when it is one, else a float.
fn text_numeric(t: &str) -> Option<Value> {
    match t.trim().parse::<i64>() {
        Ok(i) => Some(Value::Int(i)),
        Err(_) => text_number(t).map(Value::Float),
    }
}

fn builtin(name: &str, v: &[Value], now: i64) -> E {
    use Value::*;
    let want = |what: &str| fault(format!("`{name}` takes {what}"));
    match (name, v) {
        ("abs", [Int(i)]) => i.checked_abs().map(Int).ok_or_else(|| fault("a whole number past its range")),
        ("abs", [Float(x)]) => Ok(Float(x.abs())),
        ("abs", [Duration(d)]) => d.checked_abs().map(Duration).ok_or_else(|| fault("a duration past its range")),
        ("abs", _) => Err(want("a number or a duration")),
        ("min" | "max", vals) if !vals.is_empty() => {
            let mut best = vals[0].clone();
            for x in &vals[1..] {
                let better = if name == "min" { compare(CmpOp::Lt, x, &best)? } else { compare(CmpOp::Gt, x, &best)? };
                if better {
                    best = x.clone();
                }
            }
            Ok(best)
        }
        ("round", [Int(i)]) => Ok(Int(*i)),
        ("round", [x]) => match num(x) {
            Some(f) => whole(f).map(Int).ok_or_else(|| fault("`round` gives a whole number past its range")),
            None => Err(want("a number")),
        },
        ("round", [x, Int(p)]) => match num(x) {
            Some(f) if (0..=15).contains(p) => {
                let m = 10f64.powi(*p as i32);
                Ok(Float((f * m).round() / m))
            }
            _ => Err(want("a number and a whole number of places from 0 to 15")),
        },
        ("seconds", [Duration(d)]) => Ok(Float(*d as f64 / 1000.0)),
        ("seconds", _) => Err(want("a duration")),
        ("now", []) => Ok(Time(now)),
        ("int", [Int(i)]) => Ok(Int(*i)),
        ("int", [Float(x)]) if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x.trunc()) => Ok(Int(x.trunc() as i64)),
        ("int", [Float(_)]) => Err(fault("`int` gives a whole number past its range")),
        ("int", [Bool(b)]) => Ok(Int(*b as i64)),
        ("int", [Text(t)]) => t.trim().parse::<i64>().map(Int).map_err(|_| fault(format!("\"{t}\" is no whole number"))),
        ("int", _) => Err(want("a number, a truth value or a text")),
        ("float", [x @ (Int(_) | Float(_))]) => Ok(Float(num(x).unwrap_or(0.0))),
        ("float", [Text(t)]) => text_number(t).map(Float).ok_or_else(|| fault(format!("\"{t}\" is no number"))),
        ("float", _) => Err(want("a number or a text")),
        ("text", [x]) => Ok(Text(Arc::from(x.to_string()))),
        _ => Err(fault(format!("`{name}` does not take these values"))),
    }
}
