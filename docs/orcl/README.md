# ORCL: the plan the language's crates share

ORCL, the Open Rust Control Language, and the crates that carry it. `docs/veredas-plan.md` ("Naming",
"veredas-controlador: the softPLC", "From Veredas 1 crates to Veredas 2") wins on intent; `.zen/contract.md` (P-003)
wins on status. Law L6 reads this file as the plan of every `orcl-*` crate. The softPLC that runs ORCL has its own
plan, `docs/controlador-plan.md`; this file says what the language and its crates are, whoever runs them. What is
not decided is a question in `docs/questions.md`, named under "Open" below; nothing open is written here as settled.

## The role nobody covers
**ORCL is control logic written in a translatable subset of real Rust: rustc compiles and checks it, `cargo test`
runs it on simulated time, an interpreter of our own runs it in a deterministic scan, and translators turn it into
native PLC code.**

No component answers this. Controlador runs a program and owns the scan's place in a plant: its states, its faults,
its fieldbus and its gates. Maquinar plays a plant, and its `reflexo` extension runs ORCL function blocks as device
behaviour (`docs/maquinar-plan.md`). Oficina edits and simulates programs. ORCL is what all of them run: the
language, its standard library, the engine of a scan and the interpreter. It is a language, not a component (the
suite's plan, "Naming"), so its crates keep the names `orcl-*`, live under `crates/` and share this one plan.

## Decisions it builds on
- **Veredas 1, 2026-10-06** (the architect): ORCL restarted as Rust-like; Rust as much as possible; elementary
  types with their bit size; the ownership rules O1 to O14 (`language/README.md`, `language/ownership.md`, carried).
- **Veredas 1, 2026-10-24**: ORCL became part of the Veredas suite.
- **The suite's plan, 2026-11-05**: ORCL has its own SemVer as a language; controlador states which ORCL versions it
  runs; `orcl-std`, `orcl-runtime` and `demos/conveyor` are carried into ORCL, used by controlador.
- **The briefing, 2026-11-13 to 16** (`docs/controlador-plan.md`, "Execution"): programs run in our own
  interpreter, a crate of the ORCL family that maquinar uses too; its bytecode is made from the Rust compiler's
  intermediate form through `rustc_public` as it is ("rustc_public is ok as is / proceed as decided", 2026-11-15),
  which means one pinned nightly toolchain; the interpreter is proven against Rust by a fixed conformance suite; no
  check of a program's structure before it runs, for now (2026-11-16); ORCL text first, ladder later as a view of
  the same source; a program and its hardware configuration convert to native PLC code, Siemens S7-1500 and S7-1200
  first (2026-11-15).

## The crates

| Crate | What it is | Uses | Stands |
|---|---|---|---|
| `orcl-std` | The standard control library, `no_std`: the scan model (`Scan`, `Program`), IEC-style timers (`Ton`, `Tof`, `Tp`), edges (`RTrig`, `FTrig`), a counter (`Ctu`), bit access (`Bits`) | nothing | carried from Veredas 1 |
| `orcl-runtime` | The engine of a scan: a task's cycle over a process image, on a clock it is handed (virtual or real), the time-error rule, safe outputs, cycle statistics; no operating-system code | `orcl-std` | carried from Veredas 1, reworked |
| `orcl-interp` | The interpreter and its program format: bytecode made from the compiler's intermediate form, run one instruction at a time, suspendable at every instruction boundary | `orcl-std` | planned, after the front end's study |
| `orcl-front` | The front end: reads an ORCL crate through `rustc_public` on the pinned nightly and writes the program `orcl-interp` runs | `orcl-interp` | planned; where it builds is Q221 |

A crate's name and its row in law L5's map (`MAP` in `tools/leis`: `orcl-*` use each other and core) are settled
when the crate lands. The two planned crates are names for the study to argue with, not decisions.

## Scope

### The language
The rules of `language/`: Rust's syntax and style, elementary types with their bit size, explicit conversions, no
heap, no `unsafe`, no panic paths, time from the scan, data owned statically, borrows that last one call, tasks
sharing data through the process image, no interior mutability, function blocks never copied. Open since Veredas 1:
indices, text, time types, 128-bit integers, the prior art's proposals (`language/README.md`, "Not settled yet").

### The standard library
What a program builds on, `no_std`, every operation with a defined result. Checked on 2026-11-15: the on-delay,
off-delay and pulse timers, both edge detectors and the up counter, and nothing else. Technology objects (axes, PID,
cams, counters as hardware objects) are out of scope (`docs/controlador-plan.md`, "Out of scope").

### The engine of a scan
What every host of a program shares, so that a scan means the same thing in `cargo test`, in maquinar's `reflexo`,
in Oficina's simulator and on a controller:
- a task's inputs read once at the start of its scan, its outputs applied once after it;
- time read once per scan, from a clock the host hands in: virtual in a test or a simulation, monotonic on a target;
- absolute deadlines, so a cycle never drifts; late cycles counted, never caught up;
- the time-error rule of `docs/controlador-plan.md` ("Faults"): the handler on the first overrun, a fault when one
  scan exceeds twice its period; safe outputs once faulted;
- cycle statistics against the absolute deadline: lateness, execution time, overruns, skipped cycles.

How several tasks pre-empt one another, what they share and each one's slice of the process image is controlador's
execution model, open (`docs/questions.md` Q33; the proposal is `docs/controlador/execution-model.md`). Until it is
ruled, the engine runs one cyclic task, which stands on no assumed model.

### The interpreter
A program compiled by rustc runs, on a controller, in an interpreter of our own (2026-11-13), for what native code
cannot give: breakpoints and stepping on the real target, the shadow test and the online change, a scan halted at
an instruction and resumed, every variable read and forced by name. Its requirements, the first and the last the
briefing's and the rest what its debugging, its faults and its online change need (the assistant's reading):
- **bytecode from the compiler's intermediate form** through `rustc_public`, one pinned nightly (2026-11-15);
- **suspendable at every instruction boundary**, which breakpoints, single cycle, a halt that holds every task and
  the fault at twice the period all need;
- **deterministic**: the same program, inputs and scan times give the same outputs on every target;
- **no allocation during a scan**: everything a program needs is laid out when it is loaded;
- **a fault, never a panic**: a failed check (an overflow, an index out of range) is the controller's fault 502
  `program(at)` with its source location;
- **every variable by name and type**, from the type and layout data of the intermediate form, which is also what
  carries data by name and type across a swap (`docs/controlador-plan.md`, "Online change");
- **proven against Rust by a fixed conformance suite**: each of its programs runs natively and interpreted, and the
  outputs are compared scan by scan.

How the intermediate form is reached, what it holds, and the list of core-library operations ORCL allows are the
study `front-end.md`, owed before the code.

### Translators
To native PLC code, Siemens first (S7-1500 and S7-1200), program and hardware configuration (2026-11-15). Not
started. The type table the architect began is open (Q44), and so is the reading that readable SCL and the ladder
view need a second front end that reads the source (Q44, the assistant's reading).

## Rules
- **A translatable subset of real Rust.** A rule enters the language only if every target in view can represent it;
  start strict, since loosening a rule breaks no program and tightening one can (`language/ownership.md`, OQ2).
- **Its own SemVer as a language**; controlador states which ORCL versions it runs. Law L3 holds the crates under
  `crates/` to a version from 2.0.0, so they start at `2.0.0-alpha.1`; what the language's own version is beside
  them is Q220.
- **Time comes from the scan.** No ORCL crate reads a clock inside a program's scan; the host hands the time in.
- **No operating-system code in the language's crates.** Real-time scheduling, memory locking, threads, sockets and
  the fieldbus belong to the host that runs a program (controlador on Linux; maquinar on the bench). So the same
  crates build and test on every machine, and a test on simulated time is the same scan as a target's.
- **The library is `no_std`, with no `unsafe` and no panic path**, under the denied lints and the `clippy.toml` of
  `language/README.md` (the file itself waits for Q225). The engine and the interpreter are ordinary Rust held to
  the suite's rules: typed errors, no `unwrap` or `expect` outside tests.
- **Every failure inside a scan is a typed outcome**, never the end of the process (O11, as amended).
- **Fixture first.** A program's plant in a test is a simulated process image, then maquinar's bench.
- **One test file per capability** under the crate's `tests/`, so each contract clause links one file.
- **No licence is stated** in any manifest until Q35 is answered; nothing is published (`publish = false`).

## What it absorbs from Veredas 1
Read on 2026-11-22 from Veredas 1's branch `supervisao-plan` (2026-11-11): `crates/orcl-std`, `crates/orcl-runtime`,
`demos/conveyor`, `docs/orcl/`. Veredas 1 called them demo stage; they had 17 tests and a doctest.
- **`orcl-std`** comes whole: its five modules, their tests, its `compile_fail` doctest for O14 and its lints.
  Changed: the version, `0.1.0` there, starts at `2.0.0-alpha.1` (law L3). Left out for now: its `clippy.toml`, the
  list of types O13 forbids. Law L7 refuses a TOML file without the suite's stamp and clippy refuses any key it does
  not know, so the file cannot sit in this repository until L7 treats it as a tool's file, as it treats
  `.config/nextest.toml` (Q225). Meanwhile O13 is kept by convention in the repository's own crates; a program
  crate in a project, outside the repository, still carries the file.
- **`orcl-runtime`** comes as the engine of a scan. Kept: the process image, absolute deadlines, skipped cycles,
  safe outputs once faulted, the statistics. Changed: the clock is handed in, so the same engine runs on virtual
  time; a scan is stepped by its host rather than looped for a fixed duration; the watchdog follows the plan's
  time-error rule in place of Veredas 1's count of consecutive overruns, and the scan that faults never writes
  its own outputs (Veredas 1 wrote them, then the safe ones); a deadline falling exactly at a scan's end runs
  rather than being skipped; a period of zero is refused by name rather than raised to 100 µs; jitter is named
  lateness, a scan's start against its absolute deadline, which is what it measured. Moved out: the real-time setup of a Linux
  process (`mlockall`, `SCHED_FIFO` by hand-declared foreign functions) goes to controlador, the one production
  host, by the rule above and on an existing, tested library (`docs/controlador-plan.md`, "Rules").
- **`demos/conveyor`** comes as a fixture: its program (a conveyor with a guarded door and a start, stop and fault
  machine) and its tests on simulated time join `orcl-runtime`'s tests, and the engine runs it against a simulated
  plant. Not carried: its host binary, since a program runs in controlador, Oficina's simulator or a test, and a
  standing demo is the bench's.
- **`docs/orcl/language/`** comes as decided, "the user" read as the architect and its own questions renumbered OQ1
  to OQ6; O11 now says what controlador does with a failed check, and the subset checker gives way to the ruling of
  2026-11-16.
- **Not carried:** `HANDOFF.md` (a session note; the contract and git are the record here) and `research/` (about
  200 KB of reports with their sources, cited where a decision leans on them as Veredas 1's
  `docs/orcl/research/`).

## Before the code
- **This plan** (law L6), with the language's rules carried.
- **The study of the interpreter's front end and of the core-library operations ORCL allows** (`front-end.md`).
- **Controlador's execution model**, proposed for the architect's ruling (`docs/controlador/execution-model.md`,
  Q33). The engine runs one cyclic task until it is ruled.

## Open
- **Q33** controlador's execution model, which decides how the engine runs several tasks.
- **Q35** the licence. **Q44** the type table, and a second front end for the translators.
- **Q45** the reading that the conformance suite is only as good as the list of core operations ORCL allows.
- **Q220** the language's version beside its crates'. **Q221** where the front end builds. **Q222** the pinned nightly
  and its moves. **Q225** a `clippy.toml` under law L7. **Q227** the list of core-library operations ORCL allows.
  **Q229** a pulse that comes back with no new edge, and which preset a timer is judged against.
