# Ownership and borrowing

> **Status: decided, 2026-10-06, in Veredas 1.** The assistant proposed the rules from a comparison of PLC
> platforms (Veredas 1's `docs/orcl/research/plc-ownership-comparison.md`, not carried here). The architect confirmed
> them on 2026-10-06, with the assistant's recommended answer to each open question. Two rules came out of that
> review: O13 (no interior mutability) and O14 (function blocks are not copied). Carried into Veredas 2 on
> 2026-11-22 as decided; what changed with the briefing of November 2026 is said where it applies (O11, and the end
> of this page). The end of this page says which rules rustc and the lints check today.

ORCL code is real Rust, so the borrow checker already proves two things about every program: a `&mut` borrow is the
only live path to its data, and no reference outlives what it points to. ORCL code must also translate to PLC
languages, so the question is which part of Rust's model to keep. The comparison found:
- every platform can pass a variable to a call by reference (an in-out), and none was found to check aliasing;
- no platform can safely keep a reference beyond the call:
  - four have no data references at all;
  - Siemens keeps references out of static data;
  - an online change can leave CODESYS pointers stale;
- only CODESYS, TwinCAT and B&R have a heap. It is opt-in and conflicts with online change;
- only Omron enforces one writer task per variable.

So ORCL keeps what every target can represent: owned data, and borrows that last one call. rustc then proves
what no PLC language proves: no aliasing, checked at compile time.

The rules apply to ORCL programs, the `no_std` logic crates. Host code, such as the runtime or a demo's simulated
plant, is ordinary Rust.

## The rules

### O1. Data is owned statically

A program's data is a tree of owned values fixed at build time. The program struct owns its function-block
instances, which own theirs. Nothing is created or destroyed at run time, and a move compiles to a copy or to
nothing.

*On the targets:* instance data blocks and multi-instances on Siemens, FB instances everywhere else.

### O2. Borrows are call parameters

A function or method takes `&mut T` or `&T`; `&mut self` and `&self` are the instance being called. rustc proves
that a `&mut` parameter has no alias for the length of the call, which is the check no PLC makes. Once aliasing is
excluded, passing by reference and copying in and out give the same result, so every target's in-out works.
- `&mut T` becomes an in-out parameter.
- `&T` becomes a read-only in-out where the target has one: IN_OUT CONSTANT on CODESYS and TwinCAT, a Constant
  InOut on Rockwell (v18+). Elsewhere:
  - a structure or an array becomes an in-out;
  - an elementary value becomes an input (a copy).
- A literal or a packed `bool` passed by reference goes through a temporary.

Nothing writes a `&T` during the call: rustc rejects writes through it, O13 removes the types that would allow
them, and O8 keeps outside writers away. So a copy and a reference give the same result, and the translator may
choose either per target, for speed (OQ6).

### O3. No borrow outlives its call

ORCL types hold no references:
- no reference in a struct field, a `static` or a return value;
- no lifetime parameters on ORCL types.

With no stored pointer, nothing can go stale. That is also what makes online change safe on every target.

### O4. Arrays are borrowed whole

- `&[T; N]` and `&mut [T; N]` become an array in-out: `ARRAY[*]` on Siemens and in IEC, an array InOut on
  Rockwell.
- A sub-slice becomes three parameters: the array, the start and the length.

### O5. A local borrow names one place

A borrow bound inside one function body is allowed when it names one place. Examples:
- `let m = &mut self.motor;`
- `if let Some(m) = self.motors.get_mut(i) { … }`, the normal way to index without a panic path.

The translator writes the place itself, with the bounds check of `get_mut` and the index captured at the borrow.
A borrow that can name either of two places, such as `if c { &mut a } else { &mut b }`, is not allowed in v1 (OQ2).

### O6. No heap

`no_std` without `alloc`: no `Box`, `Vec`, `String`, `Rc` or `Arc`. Fixed-capacity types from `orcl-std` take
their place when they are needed.

### O7. Tasks share data through the process image

A value shared between tasks is an output of exactly one task, its writer, and an input of each task that reads it.
The runtime, or the translated code, copies it at the scan boundaries inside a critical section:
- the writer's values are published together at the end of its scan, so a reader never sees half an update;
- a reader takes its copy at the start of its scan, as it does for every input.

One writer holds by construction: a field of one task's outputs has only that task as its writer. Programs stay
"inputs in, outputs out", so their tests need no concurrency. A handshake between two tasks takes two variables, a
request and an acknowledge, as is usual on PLCs.

This is O8 applied to tasks: to a reading task, another task is outside data. There is no `Shared<T>` type (OQ4).

*On the targets:*
- Omron enforces one writer natively (the refreshing task).
- Elsewhere the copy is taken in a critical section: IEC Ed.4 MUTEX, compare-and-swap on CODESYS,
  `FB_IecCriticalSection` on TwinCAT, DIS_AIRT / EN_AIRT or UMOVE_BLK on Siemens, UID / UIE or CPS on Rockwell,
  AsSem semaphores on B&R. Mitsubishi is unconfirmed.
- ORCL promises no atomics: the platforms document atomicity differently.

### O8. Data written from outside is never borrowed

Some values can be written by an HMI, a communication partner or asynchronous I/O. They are never passed by
reference:
- they are read as a snapshot, through the process image;
- they are written through the outputs.

`Program::scan(&mut self, &Scan, &Inputs, &mut Outputs)` already works this way: the runtime reads `Inputs` once
per scan and applies `Outputs` after it. The reason: a Siemens InOut's copy-back can overwrite what an HMI wrote
during the call, and Rockwell warns that InOut values can change during the call.

### O9. No threads; no closures in v1

Tasks are configuration (`TaskConfig`), not spawned at run time, so ORCL has no threads and no `Send` closures.

Closures of any kind are out of v1: `for` loops cover the need and translate one to one to FOR loops. A closure
that does not escape its call only borrows for that call, so the model can allow it later, as in
`motors.iter().any(|m| m.fault)`, once the translator knows each iterator method (OQ3).

### O10. No optional references

`Option<&T>` is never a parameter, a field or a return type, because Rockwell and Omron in-outs must always be
connected. Pass a value and a flag instead.
- `Option<T>` of a value is fine. It becomes the value plus a flag: `Ton` keeps an `Option<Duration>`.
- The `Option<&mut T>` that `get_mut` returns is matched on the spot (O5) and never stored.

### O11. No panics; a fault if one happens anyway

The denied lints remove the usual panic sources: arithmetic overflow, indexing, `unwrap`, `expect` and `panic!`.

If a panic happens anyway, it maps to the platform's fault: a Siemens STOP, a Rockwell major fault. On
controlador the whole controller faults, never one task alone, and every output takes its declared safe value: a
program error without a handler is the reason 502 `program(at)` (`docs/controlador-plan.md`, "Faults"). In the
interpreter a failed check is that fault, never the end of the process. Veredas 1's `orcl-runtime` ended the
process; that gap is not carried.

### O12. Safety code passes references only as in-outs

Safety programs take references only as in-out parameters:
- Siemens F lists no pointer or reference types;
- GuardLogix safety AOIs cannot take standard tags.

### O13. No interior mutability

`Cell`, `RefCell`, `OnceCell`, `LazyCell`, `UnsafeCell` and the atomics are in `core`, so `no_std` does not exclude
them. ORCL code uses none of them:
- a `&T` parameter is then truly read-only, which O2 relies on;
- a `static` is then truly a constant, so no task can write a global outside the process image (O7).

The review found the gap with a probe crate that carried the ORCL crate header. Through a `&Settings` parameter,
`settings.limit.set(7)` wrote a "read-only" value, and a `static AtomicBool` gave every task a writable global.
Both passed `no_std`, `forbid(unsafe_code)` and the denied lints. A translator that passed that `&T` as a copy
would lose the write, so the PLC would behave differently from the softPLC.

### O14. Function blocks are not copied; values are

- A function block, state with an identity such as a timer, an edge, a counter or a machine, is neither `Copy`
  nor `Clone`.
- A value, such as the inputs, the outputs, a data structure or an enum, is `Copy`.
- A function block is reset through its own method, as `RESET_TIMER` resets a timer on Siemens, never by
  assigning a new instance.

With a `Copy` timer, `let mut delay = self.delay; delay.scan(…)` compiled cleanly and scanned a copy: the
instance never timed out. Without `Copy`, rustc rejects that line. `Copy` also tells the translator what is a
value type (a UDT) and what is a function block (OQ5).

## On the targets, at a glance

From the comparison; a dash means not available.

| ORCL | Siemens S7-1500 | CODESYS / TwinCAT | Rockwell Logix | Omron NJ/NX | Mitsubishi iQ-R |
|---|---|---|---|---|---|
| `&mut T` | InOut: a structure by reference, an elementary value copied in and out | VAR_IN_OUT | InOut | in-out | VAR_IN_OUT, copied in and out |
| `&T` | InOut for a structure or an array; Input for an elementary value | VAR_IN_OUT CONSTANT | Constant InOut (v18+) | input (a copy), or an in-out | input (a copy) |
| `&[T; N]`, `&mut [T; N]` | InOut `ARRAY[*]` | array in-out | array InOut | array in-out | array argument, copied |
| shared value (O7), copied at the scan boundaries | DIS_AIRT / EN_AIRT, UMOVE_BLK | compare-and-swap, `FB_IecCriticalSection` | UID / UIE, CPS | one refreshing task, native | unconfirmed |
| stored reference (O3) | Temp and FC parameters only | REFERENCE TO, POINTER TO (unchecked) | – | – | – |
| heap (O6) | – | `__NEW` / `__DELETE`, opt-in | – | – | – |

## The open questions, decided

The architect confirmed the assistant's recommendation on each, 2026-10-06. They are numbered OQ1 to OQ6 here so that they are never read as the suite's questions in `docs/questions.md`.

- **OQ1. Are O1–O12 ORCL's ownership model?** Yes, with two rules added, O13 and O14, and O7 reworded for OQ4.
- **OQ2. Is "a borrow that names one place" the right line for local borrows (O5)?** Yes, for v1. The principle
  behind OQ2 and OQ3 is to start strict: loosening a rule later breaks no program, tightening one can.
- **OQ3. Closures (O9)?** Out of v1. A closure that does not escape its call may come later.
- **OQ4. `Shared<T>` (O7): a type or a checker rule?** Neither: data shared between tasks travels through the
  process image.
- **OQ5. Should function blocks stay `Copy`?** No: function blocks are not copied, values are (O14).
- **OQ6. `&T` on targets without a read-only in-out (O2)?** A reference for structures and arrays, a copy for
  elementary values. With O8 and O13 both give the same result, so the translator chooses per target.

## How the rules are checked

Today:
- rustc enforces the aliasing half of O2 and that no reference outlives its data. It also rejects copying an
  `orcl-std` function block (O14); a `compile_fail` doctest in `orcl-std` pins that.
- `no_std` enforces O6, as long as a crate does not declare `extern crate alloc`.
- clippy, denied in each program crate:
  - most of O11, through the panic lints;
  - O13, through `disallowed_types` and the crate's `clippy.toml`.
- O7 needs no checker: it holds by construction once the runtime runs several tasks.

Veredas 1 planned a subset checker for the rest. The briefing of November 2026 set it aside: "rust as a base gives
us an optimal linter and compiler. no further checks necessary (for now)" (the architect, 2026-11-16). So these are
kept by convention, and by the interpreter where it cannot run what breaks them (`../front-end.md`):
- no lifetime parameters on types (O3);
- local borrows that name one place (O5);
- no closures (O9);
- no `Option<&T>` parameters, fields or return types (O10);
- function blocks that derive neither `Copy` nor `Clone` and are never assigned whole (O14).
