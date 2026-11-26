//! # orcl-interp - ORCL's interpreter
//!
//! Runs a program in the shape of rustc's MIR at opt-level 0, one instruction at a time, on memory laid out when the
//! program is loaded (`docs/orcl/interpreter.md`):
//! - [`Program`]: the program's types, its functions and the source files its locations name;
//! - [`Machine`]: the loaded program, its memory and its stack; a host starts an entry and steps it, and the whole
//!   state is data, so a run stopped at an instruction boundary goes on from there;
//! - [`CoreOp`]: the closed list of core-library operations a program may call (`docs/questions.md` Q227);
//! - [`file`]: the program file, `<name>.orclb`, its header naming its format, its language and its nightly.
//!
//! A program whose indices or types do not fit is refused when it is loaded, by name ([`LoadError`]). It runs on a
//! stack of the size its host gives, in slots, one whose functions call themselves among them, a call past the stack
//! being the program's own error (`docs/questions.md` Q230). A program's failure while it runs is a [`ProgramError`]
//! naming its source location, never a panic (ORCL's O11).
#![forbid(unsafe_code)]
#![deny(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used
)]

mod core_ops;
pub mod file;
mod layout;
mod machine;
mod ops;
mod program;

pub use core_ops::CoreOp;
pub use layout::LoadError;
pub use machine::{Arg, CallError, ErrorKind, HostError, Machine, ProgramError, RegionId, Step};
pub use ops::Scalar;
pub use program::{
    BinOp, Block, BlockId, Callee, Check, Const, Field, Function, FunctionId, Int, Local, LocalDecl, Operand, Place,
    Program, Projection, Span, Statement, Terminator, Type, TypeId, UnOp, Value, Variant,
};
