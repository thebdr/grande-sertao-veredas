# The interpreter: its design

**The design of `orcl-interp` and of the program file it runs, 2026-11-25, before the code.** `README.md` ("The
interpreter", "The front end") wins on intent; this file says how the requirements there are met. It stands on the
rulings of 2026-11-25: the front end is `crates/orcl-front` (Q221) on `nightly-2026-11-21` (Q222), a call into core
is an operation of a closed list (Q227), and blocks interrupt each other on one logic thread at the interpreter's
instruction boundaries (Q223, point 4). Where the plan's intent is not settled the point names its question.

## What it runs
A program file (`<name>.orclb`, `docs/controlador/files.md`) that the front end writes while rustc compiles the
program's crates. The file holds the program's own code, the program's crates being the program and `orcl-std`; core
is never in it, only the operations of the closed list. Three hosts load it:
- **controlador**, on a target, under its scheduler (`docs/controlador/execution-model.md`);
- **maquinar's `reflexo`**, which runs an ORCL function block as a device's behaviour (`docs/maquinar-plan.md`);
- **Oficina's simulator**, on virtual time.

So the interpreter runs any function the file names as an entry, `Program::scan` among them, not a program alone.

## The program file
One binary file, little-endian, its own codec and no dependency (a format the suite reads for decades should not
lean on a crate's encoding):
- **A header**: the magic `ORCLB`, the file format's version (from 1), the language's version
  (`orcl_std::LANGUAGE`), the nightly that lowered it, and the source revision the sender gives. A loader refuses a
  file whose format or language version it does not run, naming both; it never guesses, as portal does with a
  TOML file (law L7).
- **The types**: `bool`, the integers of ORCL's sizes, `f32`, `f64`, `Duration` (opaque, its operations on the
  list), structures (fields by name and type), enums (variants with their discriminants and fields), fixed arrays,
  and references, which only a call's parameters hold (O2, O3). `Option<T>` is an enum. A type outside this list
  is refused when the program is lowered, by name.
- **The functions**: each a body as MIR has it at opt-level 0 (`front-end.md`): its parameters, its locals (with the
  source's name where the source gave one), basic blocks of statements, and one terminator a block.
- **The entries**: the functions a host may call, each by its source path (`Conveyor::scan`), with the program's
  inputs, outputs and state types and the functions that build their default and safe values.
- **The source map**: for every instruction its source location, so a fault 502 `program(at)` and a breakpoint
  name a file, a line and a column.

The file carries no hash of its own: a download's integrity is its manifest's, which lists every part with its
SHA-256 and length (Q36, ruled on 2026-11-25).

## The instructions
MIR's own shape, so the front end lowers one to one and a step is a statement of the source:
- **statements**: an assignment of a value to a place, and nothing else (storage markers dropped, since memory is
  laid out at load);
- **values**: a place read, a constant, a binary or unary operation (the checked forms carrying their overflow
  flag, as MIR has them), a cast between ORCL's types (Rust's semantics, a float to an integer saturating), an
  aggregate built, an enum's discriminant read, a reference to a place;
- **places**: a local, then fields, a dereference, a constant index, an index by a local (bounds-checked: past the
  end is the fault 502), an enum variant's fields;
- **terminators**: go to, a switch on an integer, return, a call (to a function of the file or to an operation of
  the list), an assertion (failing it is the fault 502, with MIR's message: overflow, division by zero, an index
  out of bounds), unreachable (the fault 502). A drop of a type without drop glue is nothing, and one with drop
  glue is refused at lowering: no ORCL type has any (O6).

An **instruction boundary** is between two statements, or a statement and a terminator: where a breakpoint stops,
a single step ends, a halt holds every task, and the one logic thread switches to a more urgent task.

## Memory, laid out at load
Everything a program needs is sized when it is loaded, so nothing is allocated during a scan:
- each type has a size in slots (a slot holds one scalar: a `bool`, an integer, a float, half a `Duration`); a
  structure is its fields' slots in order, an enum its discriminant and its largest variant, an array its element's
  slots times its length;
- each function has a frame of its locals' slots;
- the call graph from the entries is walked at load; its deepest path sets the stack's size, the state's types the
  state's. A function that reaches itself has no deepest path: Q230 asks what happens to it; taken meanwhile, the
  file is refused at load, naming the cycle.
- a reference is the index of a slot in that memory; it never outlives its call (O3), so it never dangles.

## Running
- **A machine** holds the loaded program, its memory and its stack. It calls an entry with its arguments and runs it
  one instruction at a time; a host asks for one step, for steps until the entry returns, or for steps until a
  budget of instructions or a flag stops it. Its whole state is data, so a scan halted at an instruction resumes
  where it stopped.
- **A program error** (a failed assertion, an index out of bounds, unreachable code, a core operation's precondition
  broken) ends the entry's run with a typed error that names the instruction's source location; the machine is
  left as it was at that instruction, for the fault record. Never a panic (O11).
- **The engine of a scan** runs an interpreted program as it runs a native one. A native `Program::scan` cannot
  fail, so `orcl-runtime` gains a program whose scan may end in a program error, which the task turns into its
  fault; a native program is that kind of program and never fails (C-151 changes with it).
- **Determinism**: no host time or randomness inside the machine; integer and float operations as Rust's on every
  target (IEEE 754 for the basic float operations; no transcendental function is on the list); the same program,
  inputs and scan times give the same outputs and the same instruction count.

## The closed list of core operations (Q227)
Each operation is a variant of one enum, resolved by the front end from the definition it calls (never from a
printed name, `front-end.md`), with its type arguments where core's function is generic (`Option::get_or_insert` of
the timers' `Duration`). It starts as the study's list, keeping ORCL's types and reaching no panic entry point by
design; `Duration::new`'s slow path and `Ord::clamp` are refused until their preconditions are checked in the
operation itself. Each operation is proven against the native call by conformance programs; one is added with its
programs, never by interpreting core.

## The conformance suite
Programs that run natively and interpreted on the engine of a scan, every scan's outputs compared, the instruction
count recorded. They live with the front end (`crates/orcl-front`), whose own tests run on the pinned nightly by a
command of their own (Q221): the nightly lowers the programs, and the same crate runs them natively. They start
with Veredas 1's conveyor and a probe that reaches every block of `orcl-std` and every operation on the list.

## Built in this order
1. **`orcl-interp`'s program and machine**, its tests on stable with programs built by hand in Rust, as the bench's
   fixtures are (fixture first): every instruction, every place, the faults, memory laid out at load, a scan halted
   at an instruction and resumed. Built on 2026-11-25 (C-153), with `orcl-std`'s on-delay and rising edge lowered by
   hand and run beside the native blocks. The engine of a scan running an interpreted program comes with the
   program kind whose scan may fail ("Running"), a change to C-151.
2. **The program file**: written and read back, its header checked, a file refused by name. Built on 2026-11-25
   (C-154): every file it accepts writes back to the very same bytes, and one small program's bytes are pinned.
3. **`crates/orcl-front`**: the shell on stable, the driver on the nightly lowering the conveyor and the probe. A
   clause, its run the nightly command.
4. **The conformance suite and the closed list** as it grows.

## Open
- **Q230** a program whose functions call themselves: refused at load (taken meanwhile), or run on a stack of a size
  the target sets, a fault when it is exceeded.
