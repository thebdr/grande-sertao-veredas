# The interpreter's front end, and the core-library operations ORCL allows: a study

**A study, 2026-11-22.** The briefing owed it before the code (`docs/briefing/deliverables.md`, J2; the review's
entry 12): how a program compiled by rustc becomes the bytecode controlador's interpreter runs, and which
operations of Rust's core library an ORCL program may reach. It rules nothing. The architect's ruling it stands on:
the bytecode is made from the compiler's intermediate form (MIR) through `rustc_public`, as it is, which means one
pinned nightly toolchain ("rustc_public is ok as is / proceed as decided", 2026-11-15). What it leaves open is
`docs/questions.md` Q221, Q222 and Q227.

Everything under "What ran here" was run on the Linux machine on 2026-11-22, in a scratch folder outside the
repository: a driver of about 300 lines, `orcl-std` and the conveyor copied from Veredas 1 (commit `671cdc43`), and
two probe programs, one reaching every block of `orcl-std` and one every arithmetic form. The driver and the
scripts that reproduce every figure are not committed; this text carries what they showed.

## What the review said, and what holds
The review (2026-11-15) said three things. Checked again:
- **`rustc_public` is not on crates.io.** Still true: the crates.io API answers that no crate `rustc_public`,
  `stable_mir` or any variant exists (https://crates.io/api/v1/crates/rustc_public). The crate is
  `0.1.0-preview` inside the compiler's tree; its release to crates.io is tracked in an issue open since
  2023-09-01 (https://github.com/rust-lang/rustc_public/issues/30). Using it needs a nightly, the `rustc-dev` and
  `llvm-tools` components and `#![feature(rustc_private)]` (the project's book,
  https://github.com/rust-lang/rustc_public/blob/main/book/src/initial.md). On stable 1.100.0 a driver fails to
  build (E0463, no `rustc_driver`).
- **"The core library ships no MIR for most of its functions."** Overstated for ORCL. With the toolchain's own
  library every core *function* an ORCL program reached had a body. What had none: one intrinsic with no fallback
  (`saturating_add::<i32>`) and the panic entry points. It is `orcl-std` that needs care: built without
  `-Zalways-encode-mir`, 6 of the conveyor's 10 program functions had no body (`Ton::scan`, `RTrig::scan`,
  `Ctu::scan`, `Ctu::cv`, `ms`, `s`), because a library's MIR is kept only for generic or inlinable functions.
- **"The interpreter needs its own build of the standard library, or a hand-written list of core operations."**
  The list is needed either way. A core rebuilt with `-Zalways-encode-mir` (through `-Zbuild-std=core`, 16 to 21
  seconds) gives the panic entry points bodies, but they end in `panic_impl` and the `caller_location` intrinsic,
  which have none in any build.

## Facts about `rustc_public` (sources opened 2026-11-22)
- **The interface moves under one version number.** Within a year, at `0.1.0-preview` throughout, MIR's enums
  changed six times (an operation replaced, a run-time check added, a variant removed, a cast kind added), read
  from the history of `compiler/rustc_public/src/mir/body.rs` in rust-lang/rust. The project's stated aim is "a
  public, SemVer compliant interface rather than a completely stable API" (https://github.com/rust-lang/rustc_public).
- **A driver starts through a macro**, `rustc_public::run!`, that runs after analysis and before code generation;
  the book says "there's currently no stable way" to start the compiler for it (issue #69, open since 2024-03-11).
- **MIR cannot leave the compiler as it is.** Its types serialise but never deserialise, and every type,
  definition and span is written as an index into the compiling session's own tables: the conveyor's root body
  becomes 63 KB of JSON whose call targets read `"ty": 0`. So the bytecode is made inside the driver, while the
  compiler runs; nothing of `rustc_public`'s own form is the program file.
- **The driver is tied to one nightly at run time too.** It links that nightly's `librustc_driver` (158 MB)
  dynamically; whatever ships the front end ships that exact library.

## Prior art
- **Miri** interprets MIR through the compiler's internal interfaces, not `rustc_public`, and builds its own
  standard library with `-Zalways-encode-mir -Zmir-opt-level=0` among its flags; intrinsics and foreign functions
  are hand-written shims (https://github.com/rust-lang/miri, `src/lib.rs`, `cargo-miri/src/setup.rs`).
- **Charon** exports a crate's MIR to JSON that its own library reads back, pinned to a nightly and built on the
  compiler's internals; it forces `always_encode_mir` and MIR opt-level 0, and builds a Miri sysroot because the
  standard library otherwise keeps MIR only for "const items and generic or inlineable functions"
  (https://github.com/AeneasVerif/charon, `get_mir.rs`).
- **Eurydice** compiles a subset of Rust to C from Charon's output, and implements the core operations it meets as
  a hand-written list (`include/eurydice_glue.h`, e.g. `core_num__u8__wrapping_add`): the list approach, in
  production (https://github.com/AeneasVerif/eurydice).
- **Kani**, a model checker, uses `rustc_public` "as is, despite them not being stable yet", pins the same
  `nightly-2026-11-21` used here, and builds its own standard library with `-Z always-encode-mir`
  (https://github.com/model-checking/kani, `docs/src/stable-mir.md`).

## What ran here
The toolchain: `nightly-2026-11-21` (rustc 1.102.0-nightly c7d5ba59d), with `rustc-dev`, `llvm-tools` and
`rust-src`: 293 MB to download (`rustc-dev` alone 119 MB), 1.5 GB installed, four and a half minutes to install,
in the home folder through rustup.

The driver found each local `Program::scan`, walked every function instance it reaches (each call resolved through
`Instance::resolve`, each drop through its drop glue) and wrote every body as `rustc_public` MIR, with the calls
that leave the program's crates. It built clean in 2.6 s, with no crate from crates.io. Lowering a program crate
took 0.04 to 0.07 s in all, against 0.03 to 0.04 s for plain rustc; about 100 MB of memory.

| Build | Program | Functions reached | Program functions without a body | Core functions without a body |
|---|---|---|---|---|
| `orcl-std` without `-Zalways-encode-mir` | conveyor | 10 | 6 | (the walk stops) |
| the toolchain's own core, program crates with `-Zalways-encode-mir` | conveyor | 31 | 0 | 3 |
| the same | every block of `orcl-std` | 87 | 0 | 3 |
| core rebuilt with `-Zalways-encode-mir`, dev profile | conveyor | 76 | 0 | 4 |
| the same, release profile | conveyor | 22 | 0 (3 left of the program's own) | 4 |

Three findings beside the table:
- **An optimised build loses the program's shape.** In release the MIR inliner folds `RTrig::scan`, `Ctu::scan`,
  `ms` and the rest into `Conveyor::scan`; breakpoints and single steps need every function kept, so the program's
  crates are compiled at MIR opt-level 0, as Miri and Charon do.
- **Overflow checks lower as the interpreter needs them**: `+` becomes a checked addition and an `Assert`; `/`
  asserts a divisor that is not zero and a quotient that does not overflow; unary `-` and `<<` assert too. A failed
  `Assert` is the fault 502 `program(at)`, its span the source location (`docs/orcl/README.md`).
- **Two quirks.** `Instance::has_body()` answers false for compiler shims that `body()` returns, so the driver asks
  `body()`. Printed names follow re-exports (core's `Duration` prints as `orcl_std::Duration`), so the driver keys
  the operations below on the definitions it resolves, never on names.

Not run: anything on Windows (`rustc-dev` exists for `x86_64-pc-windows-msvc`); a full standard library or a Miri
sysroot; an interpreter. The spike lowers and lists; it executes nothing.

## The core-library operations ORCL programs reach
The program's crates are the program itself and `orcl-std`. Across the conveyor and a probe that reaches every
block of `orcl-std`, their code calls into core at 27 distinct functions, and core's own bodies use operations far
outside ORCL's subset: `isize` and `usize`, raw pointers, `Transmute` into niche-carrying types, closures,
`fmt::Arguments` and function pointers on the panic paths. Interpreting core's MIR would need a value model as wide
as Miri's. Cutting at the boundary instead, each call from the program's crates into core is one operation of the
interpreter, implemented natively, as Eurydice does.

What the programs reached, grouped:
- **Integer arithmetic**: `i32::saturating_add` (the counter); in the probe also `wrapping_mul`, `checked_sub`,
  `saturating_mul`; and in program code the MIR operations themselves: checked `+ - *` with their `Assert`,
  `/` and `%` with theirs, unary `-`.
- **Bits**: `checked_shl` and `checked_shr` on `u8` to `u64` (`Bits`); `&`, `|` and `!` on those and on `bool`.
- **Comparison**: `Ord::min`, `PartialOrd::ge` and `PartialEq::eq` on `Duration`; `<`, `>=`, `==` on the integers;
  an enum's discriminant read and compared (an `isize` the interpreter keeps for itself); in the probe `Ord::max`
  and `Ord::clamp` on `i32` (`clamp` can panic).
- **`Option`** of a value: `get_or_insert` (the timers), `is_none`, `is_some_and` (`Bits::get_bit`, through a
  closure); in the probe `unwrap_or`, and `Result::unwrap_or`.
- **`Duration`**: `from_millis`, `from_secs` (`ms`, `s`), `saturating_sub`; inside them `checked_sub` and `new`,
  whose slow path calls `expect_failed`, a panic entry point.
- **Conversions**: in the probe `From<u8> for u16`, `TryFrom<i32> for u8`, `as` between integers and from an
  integer to `f32`; `f32` addition and multiplication.
- **`Default`** of `bool` and every integer type, through `#[derive(Default)]` and `Program::safe_outputs`.
- **Panic paths**: `expect_failed`, `panic_nounwind_fmt` (every precondition check behind the compiler's run-time
  checks), `panic_fmt` (`clamp`); `Unreachable` on a `match`'s impossible arm.

The assistant's proposal for "the list of core-library operations ORCL allows" (J2), not ruled (Q227): the
operations above that keep ORCL's types and never reach a panic entry point by design, each implemented natively
by the interpreter and proven against the native call by the conformance suite; a call into core outside the list
refused when the program is lowered, naming the function and the place it is called from. That refusal is the
interpreter saying what it cannot run, not a check of the program's structure: the ruling of 2026-11-16 ("no
further checks necessary") stands for the program. The list grows by adding an operation and its conformance
programs, never by interpreting core.

## What follows for the design
The assistant's reading, for the interpreter's plan; each point is open where it names a question.
- **The front end is a driver on the pinned nightly**, run as the compiler of the program's crates; it writes the
  program file (`<name>.orclb`, `docs/controlador/files.md`) while the compiler runs, keyed on resolved definitions.
  Where it builds beside a workspace on stable is Q221; which nightly and when it moves is Q222.
- **The program's crates are compiled with `-Zalways-encode-mir` and MIR opt-level 0**: every function keeps a body
  and the shape of its source.
- **The cut at core** (Q227): the operations above, native, in a closed list; core's MIR is never interpreted, so
  the toolchain's own core library serves and no standard library is rebuilt.
- **The interpreter's values** are ORCL's elementary types, `Duration`, enums with their discriminant, structures,
  fixed arrays and an `Option` of a value; anything else in a body is refused at lowering, by name.
- **The conformance suite** runs each of its programs natively and interpreted on the engine of a scan
  (`orcl-runtime`, which runs any `Program`) and compares every scan's outputs; it is rerun at every move of the
  pin, the interface moving as the facts above show.
- **On the sender's machine** the front end needs the pinned nightly with `rustc-dev` (293 MB, 1.5 GB installed) and
  the driver built against it. Oficina installs both, which is Oficina's plan to carry.

## Found in passing
- **`orcl-std` used a closure**: `Bits::get_bit` called `is_some_and(|v| ...)`. ORCL keeps closures out of v1 (O9);
  whether that binds `orcl-std` too is not written down, and the closure would cost the interpreter a closure value
  it otherwise never needs. Since 2026-11-25 `orcl-std` says the same with a `match` (C-150), and its library code
  holds no closure.
- **Core has an unstable `set_bit`** on unsigned integers (feature `get_set_bit`, rust-lang/rust#147702), by
  value, returning the new integer. Were it stabilised as it is, `status.set_bit(n, v)` would resolve to core's
  method, an inherent method winning over `orcl-std`'s trait, and become a statement that changes nothing.
  `orcl-std`'s own test of `set_bit` would then fail, so the change cannot pass unseen; the name the architect
  chose (2026-10-06) stands until then.
