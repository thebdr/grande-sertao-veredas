# The ORCL language

ORCL, the Open Rust Control Language, is the architect's own control language. ORCL code is Rust that rustc
compiles:
- `cargo test` runs it on simulated time;
- controlador's interpreter runs it in a deterministic scan (`docs/controlador-plan.md`);
- translators turn it into native PLC code, Siemens S7-1500 and S7-1200 first (the architect, 2026-11-15).

ORCL supplies the platform functions (ports or mock-ups) and the special data types itself, in `orcl-std`.

ORCL started as OPCL, a Pascal-like language for Siemens, and was restarted as Rust-like on 2026-10-06. Nothing from
the Pascal era carries over: not its syntax, not IEC as the global rule, not chained assignment, not its type list.
This page and [`ownership.md`](ownership.md) were written in Veredas 1 and carried into Veredas 2 on 2026-11-22;
the decisions below are the architect's of 2026-10-06, and the briefing of November 2026 is named where it added to
them.

## Decided (the architect, 2026-10-06)

- **Rust as much as possible:** its syntax, its scopes and its style.
- **Every elementary type carries its bit size**, because types must translate to many PLC languages: `bool`,
  `u8` `u16` `u32` `u64`, `i8` `i16` `i32` `i64`, `f32` `f64`. So there is no `usize` or `isize`.
- **Operators:** Rust's `&&`, `||` and `!`.
- **Naming:** Rust style, snake_case.
- **Strongly typed:** every conversion is explicit.
- **The parts of a value are reached through methods,** such as `status.get_bit(3)`. `orcl-std` implements
  `get_bit`, `set_bit` and `with_bit`.
- **Ownership and borrowing:** the rules O1–O14 in [`ownership.md`](ownership.md). In short: data is owned
  statically, borrows are call parameters that never outlive the call, there is no heap, tasks share data through
  the process image, there is no interior mutability, and function blocks are not copied.

## Decided in the briefing of November 2026

- **No check of a program's structure before it runs**, for now: "rust as a base gives us an optimal linter and
  compiler. no further checks necessary (for now)" (the architect, 2026-11-16). The subset checker Veredas 1 put on
  its roadmap is not built.
- **ORCL text first; ladder later**, as a view of the same source.
- **ORCL stays a translatable subset of real Rust, with its own SemVer**; controlador states which ORCL versions it
  runs (the suite's plan, 2026-11-05). How the language's version relates to its crates' versions is open
  (`docs/questions.md` Q220).

## Kept by convention

rustc, `no_std` and the lints check part of the rules; the rest is kept by convention, as `orcl-std` does:
- **No heap:** the logic crates are `no_std`.
- **No `unsafe`:** `#![forbid(unsafe_code)]`.
- **No panic paths.** clippy denies `arithmetic_side_effects`, `indexing_slicing`, `panic`, `unwrap_used` and
  `expect_used`. Time arithmetic saturates, and a bit index past the width reads `false` and writes nothing.
- **Time comes from the scan,** never from a clock.
- **No interior mutability** (O13): each program crate's `clippy.toml` lists `Cell`, the other cells and the
  atomics, and the crate denies `clippy::disallowed_types`.
- **Function blocks are not copied** (O14): `orcl-std`'s timers, edges and counter derive neither `Copy` nor
  `Clone`, and a `compile_fail` doctest pins it.

## Not settled yet

Open since Veredas 1, none ruled:
- **Indices:** Rust indexes with `usize`, which ORCL excludes. So O5's `self.motors.get_mut(i)` needs a
  conversion from the index's own type, or an `orcl-std` helper.
- **Text:** strings and characters. Rust's `char` is a 32-bit Unicode scalar, while a PLC CHAR is 8 bits
  (`docs/questions.md` Q44).
- **Time:** `orcl-std` uses `core::time::Duration`, which matches neither S7's TIME nor its LTIME (Q44).
- **128-bit integers:** no PLC has them.
- **The prior art's proposals** (Veredas 1's `docs/orcl/research/prior-art.md`, not carried): publishing a task's
  shared values at the end of its logical period (LET) rather than at the end of its scan; only `for` loops over
  ranges and arrays in v1; edition 2021 for program crates, to keep a qualified compiler's safety profile open; the
  first test oracle for a translator.
- The rest of the language, to be decided topic by topic.
