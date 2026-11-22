# veredas-controlador: the plan

The soft PLC of Veredas 2. `docs/veredas-plan.md` ("veredas-controlador: the softPLC") wins on
intent; `.zen/contract.md` (P-003) wins on status. Nothing is built. This file holds what the
architect decided in the briefing of 13 to 16 November 2026, in the rulings of 15 November on its
adversarial review, and in the robustness round begun on 16 November (`docs/robustness.md`). The
list that was reviewed and the review itself are kept as they were delivered
(`docs/briefing/deliverables.md`, `docs/briefing/review.md`); where they differ from this file,
this file is the later word. What is not decided is a question in `docs/questions.md`, named under
"Open" below; nothing open is written here as settled.

## The role nobody covers
**Controlador is the suite's soft PLC: it runs an ORCL program in a deterministic scan on a
real-time Linux target, drives the plant's devices over the fieldbus itself, and keeps running as
it was left, for years, with nobody attending it.**

No other component answers this: portal reads and stores a plant's data and gates writes to it,
and never closes a control loop; the bench plays a plant and never runs in production. The
architect's picture of it (2026-11-16): "picture a cpu doing simple i/o logic that is left there
for years. that is what a plc is, it runs as the day the software engineer left for an undefined
time."

## Decisions it builds on
- The suite's plan (2026-11-05): ORCL programs in a deterministic scan with watchdog and safe
  outputs; direct NIC access on the real-time path, the only production path past portal; the
  hardware configuration on an embedded aranha; everything else through an embedded portal.
- The briefing (2026-11-13 and 14): everything below, in ONE first release.
- The review's rulings (2026-11-15, six rounds): where they changed the briefing, the text below
  carries the ruling and names its date. One of them narrowed the release: the first release
  leaves out hardware interrupts and allows the shadow test only for an application with one
  cyclic task.
- The portal-to-portal protocol (2026-11-16) and the robustness round (from 2026-11-16).

## Scope

### Execution
- **Production on Linux with a real-time kernel only.** Windows is for development and simulation.
  A production target refused real-time scheduling does not run: it stays in STOP.
- **64-bit processors only** (the architect, 2026-11-16).
- **Programs run in our own interpreter**, a crate of the ORCL family that maquinar uses too. Its
  bytecode is made from the Rust compiler's intermediate form, through `rustc_public` as it is
  ("rustc_public is ok as is / proceed as decided", the architect, 2026-11-15), which means one
  pinned nightly toolchain.
- **The interpreter is proven against Rust by a fixed conformance suite.**
- **No check of a program's structure before it runs**, for now: "rust as a base gives us an
  optimal linter and compiler. no further checks necessary (for now)" (the architect, 2026-11-16).
  The compiler and the linter on the sender's machine stand for the source, and the download's
  hash for the program that arrives.
- **The program's clock pauses while the program is halted.**

### Entry points
After Siemens' organisation blocks: the architect went through Siemens' list on 2026-11-14 ("I
think Siemens did a great job in the PLC implementation") and kept every family except three.
- Several main blocks and several start-up blocks, run in order.
- Cyclic interrupts, each with its own period and priority.
- Time-of-day blocks and time-delay blocks.
- Hardware interrupts (a device's process event). Not in the first release (2026-11-15).
- Handlers: time error; device lost or returned; device diagnostics (a device still present that
  reports a problem, such as a wire break); pull or plug of a module; I/O access error; device
  alarms (status, update, manufacturer or profile-specific); program error; stop requested (one
  last scan).
- A more urgent block interrupts a running one.
- Kept out: synchronous cycle, motion control, redundancy errors.
- **On continue after a halt, nothing missed runs** (2026-11-15): whatever came due or arrived
  during the halt is dropped, and the program continues as if no time had passed.
- **Time-of-day blocks wait for a confirmed date** (2026-11-16): they do not start while the
  controller's date is unconfirmed, and starts that fall before the confirmation do not run
  afterwards (`docs/robustness.md`, "Time over decades").

### Start, stop and power
- **Start kinds:** warm (retained variables kept; the normal start, also after a power cycle) and
  cold (everything reinitialised, on command). Start-up shows its phase: load, restore, handler,
  devices. After the start-up time the controller goes to RUN regardless of missing devices and
  shows what is missing.
- **After a power cycle**, as Siemens does: RUN returns as RUN; STOP, HOLD and FAULT all return as
  STOP, the reason saying what it was. "Always as before" (the architect, 2026-11-15): no start-up
  parameter, no STOP forced by a commissioning condition, no rule that ends a shadow test; the
  program handles the rest.
- **A dry run survives a power cycle** (2026-11-15): RUN with the dry run on comes back the same,
  outputs safe until a person ends the dry run.
- **Nobody switches states in normal work**: "we do not switch states in normal working
  conditions, ever" (the architect, 2026-11-15). RUN is the only state with no user interaction
  going on; a fault needs a person, and a triggered stop follows the same logic.

### Monitoring, forcing, debugging
- **Live values** of any variable; watch tables.
- **Forcing** covers inputs, outputs and variables. A variable is forced in one of three ways,
  chosen at the time: once now, before each scan, or always (held at every write, marked `[!]`,
  behind a confirmation).
- **A force belongs to the controller** until someone removes it (2026-11-15): it shows in the
  state text, a restart clears it, and a lost connection changes nothing.
- **Breakpoints and stepping** on the real target and in the simulator. A hit halts every task.
- **Outputs at a halt** follow a per-output setting: safe by default, held if the engineer chose
  it. A forced output follows the same setting. Each target has a longest time outputs may stay
  held at a halt, short by default; when it runs out the held outputs go safe and the controller
  stays halted (2026-11-15).
- **A halted target stays halted** if Oficina disconnects.
- **Pause on command** (the program halts at the end of the scan), **single cycle** (one scan,
  then halted again) and **dry run** (the logic runs on live inputs, every output stays safe).

### The commissioning switch
- **One switch gates every engineering change** (2026-11-15): download, swap, accept, rollback,
  cold start, reset of retained data, hardware configuration and device naming, as well as writes,
  forcing, breakpoints, starting, continuing from HOLD and acknowledging a fault. Only reading and
  stopping are free, so any change to a plant needs someone to arm the switch at the target first.
- **It is a file on the controller's operating system, not writable from outside** (the architect,
  2026-11-15), and it is never set over the network.

### Online change
- **A shadow test on the whole application.** The new version is deployed; both run on the same
  inputs, each on its own data; one drives the outputs. Oficina shows what would differ: the
  outputs, and a watch list.
- **A user action swaps them.** The old version keeps running as the watcher, so going back is
  instant, until the engineer accepts. The version taking over continues from the machine's real
  data, carried by name and type; a retyped variable is reinitialised alone, and Oficina lists
  every added, removed and reinitialised variable before the switch.
- **What may differ:** logic, data and the IO mapping. Hardware configuration and task timing may
  not.
- **Entry check:** a controller enters a shadow test only after a check that the doubled scan fits
  the task's period. In the first release only an application with one cyclic task may enter one
  (2026-11-15).
- **Breakpoints only in the driving version**; a hit halts everything and the watcher pauses in
  step. Trouble in the watching version (an error, or an overrun from the doubled scan) ends the
  test and the machine keeps running.
- **Fallback for a faulted edit.** A controller-wide flag, offered in the download popup, with a
  time limit, reset on shutdown. When the time limit runs out the edit is accepted and the old
  version stops watching. The review showed that taking over "from its own watcher data" is
  unsafe as first written, and the architect ruled (2026-11-15): "live fallback shall be enabled
  only when feasible. we need to build the specification for what disqualifies the edits for auto
  takeover. takeover after restart remains possible for all cases". So the previous version can
  always be put back for the next start; a live takeover happens only for an edit that nothing
  disqualifies; and what disqualifies an edit is Q32. After a fallback the state carries the
  maintenance request `fault_on_update`.
- **The last 10 downloads are kept** on the controller: hash, time, origin, kind, source revision,
  the program itself and the source archive.

### Faults
- **The whole controller faults, never one task alone.** One kind of fault only.
- **In FAULT every output takes its declared safe value**, with no per-output exception, and the
  fieldbus keeps exchanging.
- **Leaving FAULT takes two actions:** acknowledge to STOP, then start.
- **Reasons:** the watchdog; a program error without a handler; a fault raised by the program with
  a code; a fault inside a handler; resources exhausted (call depth, memory); an application that
  cannot be loaded; the runtime's own failure (after an unexpected restart it comes up in FAULT,
  which stops a crash loop).
- **Overruns follow Siemens' OB80:** the time-error handler runs once on the first overrun, and
  the controller faults only when one scan exceeds twice its period.
- **A maximum and a warning cycle time**, both set by the user (the architect, 2026-11-16): "the
  warning after years is a hint of possible degrading". How the two meet the overrun rule is Q39.
- **A fault clears every debug condition** and records what was active.
- **A fault record** holds the cause, the time, the download number, the task, the source
  location, the debug conditions that were active, a snapshot of all data, the last seconds before
  it, and the handler's outcome. Its details are in the popup and in the fault history.
- **The program may** raise a fault with a code, request a stop, raise a maintenance request, or
  record an event only.

### Devices that go away and come back
- **A device lost while running never faults the controller.** Its inputs are invalid and the
  program reacts in its device-lost handler. A loss reaction set per device (carry on, stop or
  fault) was offered on 2026-11-15 and not chosen.
- **While the device is away its inputs read zero or false** (the assistant's default, stated to
  the architect on 2026-11-15 and not objected to).
- **A loss leaves an error that must be acknowledged.** From the device's return until then,
  "inputs can be read when a device is back online but the error was not yet acknowledged, outputs
  may be written by the program but not sent to the io device until the error is acknowledged"
  (the architect, 2026-11-15).
- **The program acknowledges it with a call**, for example on an operator's button. A program with
  no handler for it has the error acknowledged automatically after one full scan on valid inputs.
- **A missing device raises an urgent maintenance request**: "in production a missing device
  requires attention".
- **Device diagnostics** have a handler of their own, so the program can answer a critical device
  event with a maintenance request, a stop or a fault.

### The state
Two layers. Layer 1 is one uppercase word: STOP, STARTUP, RUN, HOLD, FAULT. Layer 2 is
function-style terms after it, lowercase: at most one reason, then every condition that applies,
in a fixed order, with counts and identifiers and never lists.

- **Spelling.** Values follow TOML's own rules for integers and quoted strings ("quoted string for
  parsers", the architect, 2026-11-14): numbers bare, strings quoted.
- **One writer and one reader.** The controller holds the state as structured data; one writer and
  one reader in core produce the text, so every screen, log and test shows the same characters.
  The text stays English; its details are translatable.
- **Numbers.** Every reason is an enum entry with a stable number and a word, and the text shows
  both, the number as the first argument `id`. A reason's hundreds digit is its state. The
  assignment below and the order of the conditions are the assistant's, agreed by the architect
  (2026-11-14).

| State | Number | Reasons |
|---|---|---|
| unknown | 0 | |
| STOP | 1 | 101 `command(source)`, 102 `power_on(was)`, 103 `acknowledged`, 104 `no_program`, 105 `realtime_refused` |
| STARTUP | 2 | 201 `warm`, 202 `cold`; its phases are a small enum of their own: 1 load, 2 restore, 3 handler, 4 `devices(up, of)` |
| RUN | 3 | |
| HOLD | 4 | 401 `breakpoint(at)`, 402 `command(source)`, 403 `step`, 404 `cycle` |
| FAULT | 5 | 501 `watchdog(task)`, 502 `program(at)`, 503 `user(code)`, 504 `handler(event)`, 505 `resource(kind)`, 506 `load(download)`, 507 `internal` |

- **Command sources:** `oficina`, `local` and `program`. The source `peer` was withdrawn on
  2026-11-15, with the rule that no peer changes the state.
- **Conditions, in this order:** `breakpoint(set)`, `force(in, out, var)`, `shadow(drives,
  watches)` (what someone did to the controller); `dry`, `sim`, `no_realtime` (the mode it runs
  in); `devices(missing)`, `overruns(count)`, `retain(reset)` (what it is suffering); then each
  `maintenance_request(id, reason, urgent)`. `breakpoint(at)` appears only on HOLD; `shadow` names
  download numbers.
- **Maintenance reasons:** 1 `fault_on_update` (urgent), 2 `device` (as the device reports), 3
  `device_missing` (urgent), 4 `overruns` (not urgent), 5 `retain_reset` (not urgent), 6 `user`
  (as the program states). `urgent=false` is PROFINET's maintenance required, `urgent=true` its
  maintenance demanded.
- **A maintenance request is cleared only from Oficina or by a restart**, and is raised again if
  its cause persists.
- **Examples:** `RUN`; `RUN breakpoint(set=2) force(in=1, out=2, var=3) shadow(drives=13,
  watches=12) sim()`; `HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)`;
  `STOP command(id=101, source="oficina")`; `FAULT watchdog(id=501, task="main")`; `STARTUP
  warm(id=201) devices(up=3, of=5)`; `RUN maintenance_request(id=3, reason="device_missing",
  urgent=true)`. At a halt a forced output follows its own halt setting, which is why `out=2`
  became `out=1` between the second example and the third.
- **Health.** The standard health value (NE 107, OPC UA's DeviceHealth) is derived from the state:
  FAULT gives failure; STOP, STARTUP, HOLD and any debug condition give function check; a
  maintenance request gives maintenance required; a clean RUN gives normal.
- **Not ruled:** the review's entries 17 to 21 on this text (one name meaning two things, the
  spelling rules, the `id` of a maintenance request, a version for the text, what a debug
  condition is), Q31. The terms and numbers the robustness round owes are not assigned either: an
  unconfirmed date, the clock battery and NTP, a certificate near its end, the warning cycle time,
  a start from the last save (Q39, Q40).

### Networks and devices
- **Four networks in the first release**, built in this order: PROFINET IO controller (written
  here on aranha's codecs, since no open controller exists; join PROFINET International and
  certify later), then EtherCAT master (on the ethercrab crate), EtherNet/IP scanner (fork a young
  Rust crate, test on real devices), Modbus TCP client (existing crates).
- **The functions that drive hardware live in `veredas-core`**: safe in controlador on Linux, and
  callable by other crates for testing only ("testers should be near a kill switch or estop
  anyway"). The architect kept this on 2026-11-15 against the review, with core's rules amended
  (`docs/core-plan.md`; `docs/questions.md` Q9).
- **Vendor description files** (GSDML, ESI, EDS) are copied into the project and read directly;
  the chosen configuration is written as Veredas TOML.
- **The hardware configuration** stands on an embedded aranha, whose scan feeds the compare and
  merge of the configuration and the topology view.
- **IO check** runs through a controlador target; with none set, through the laptop session
  (`docs/oficina-plan.md`, "The IO check from a laptop").
- **Fixture first:** the bench devices for all four networks live in maquinar
  (`docs/maquinar-plan.md`).

### Talking to other systems
- **Modbus/TCP and OPC UA by default**, with other controllers and higher-level systems, both
  served by controlador's embedded portal.
- **What they publish:** the controller's state, its faults, and only the variables the ORCL
  program marks. The standard health value is published too.
- **Writes.** A variable is writable from outside only if the program marks it so. The gate is the
  embedded portal's master switch, and its whitelist holds exactly the variables the program marks
  writable; controlador has no second setting (2026-11-15).
- **Nobody outside changes the state.** "peers, intended as other controllers we controlador
  interacts with, shall never be able to change its state" (the architect, 2026-11-15). The same
  holds for supervisão, SCADA and MES: they read the state.
- **Bound by the project, secured by the protocol** (2026-11-15): a protocol is bound only when
  the project names the interface; OPC UA writes need a signed and encrypted channel and a user;
  Modbus writes come only from listed client addresses.
- **Modbus addresses** are declared by the engineer in the project file. The state sits in a fixed
  block of registers, laid out identically on every controlador, before the engineer's own.

### The engineering link
- **Controlador owns the real-time communication, and nothing else on the wire.** "controlador
  shall own the RT communication / embedded portal in controlador shall get access to
  controlador's memory: variables list, cpu states, etc; it reads values asynchronously" (the
  architect, 2026-11-15).
- **The embedded portal is the full one**, with its store and its server: "controlador embeds a
  full version of portal" (2026-11-15).
- **Oficina's portal talks to it over portal's own HTTP interface**, on the controller's loopback
  behind SSH, commands on the same link (2026-11-16; `docs/oficina-plan.md`, "How it reaches a
  target"). Controlador handles the commands behind its portal and checks the switch there.
- **Everything that alters the controller is robust** (2026-11-16), a download above all: it
  carries a hash, "so no corrupted program may run" (Q36, Q37).
- **State changes and faults are stored like any other data**, and the history view reads them.
- **By default the embedded portal keeps its recorded values in memory, as a fixed window**, and
  writes nothing to the drive (2026-11-19; `docs/robustness.md`, "Wear and filling").

### Language and project data
- **ORCL text first; ladder later**, as a view of the same source.
- **Project data:** multilingual text lists, alarm and event definitions, recipes, units and
  limits on variables.
- **A simulator** with the bench as plant, virtual time and snapshots.
- Checked on 2026-11-15: Veredas 1's `orcl-std` holds the on-delay, off-delay and pulse timers,
  both edge detectors and the up counter, and nothing else.

### Conversion to native PLC code
The goal, stated outright by the architect on 2026-11-15: controlador's ORCL program **and its
hardware configuration** convert to native PLC code, Siemens S7-1500 and S7-1200 first. Veredas
1's ORCL notes already planned the program translators, Siemens first; the hardware configuration
is new. The architect began a table mapping ORCL data types to S7 types; its open points are Q44.

### What keeps it running for decades
The robustness round's rulings, stated in `docs/robustness.md`, bind controlador first:
- what must survive is "every piece of data that is necessary for computing the logic flawlessly
  in RUN mode"; the commissioning scope is outside it;
- the program is stored twice with a hash per block, the two copies mend each other, the
  integrity is rechecked at power-on and on a schedule while running;
- "every cycles saved data is temporary and moves to permanent only on successful cycle";
- the date is unconfirmed until NTP or the clock chip confirms it, with the maintenance requests
  the architect ruled for the clock battery and NTP;
- a maintenance request a year before any certificate expires;
- retained variables by what the target can do, the last save flagged where it cannot save at
  power loss, and a write from outside saved at once.

### Out of scope
Technology objects entirely (axes, PID, cams, counters). Security features, until the security
round (Q43). Certified functional safety. Know-how protection. Synchronous cycle, motion control
and redundancy blocks.

## Rules
- **The only production component past portal, and only on the real-time path.** Everything else
  it does goes through its embedded portal.
- **Nobody changes the controller's state but a person, in Oficina or at the target**; the program
  may stop itself.
- **Every engineering change needs the switch armed at the target.** Stopping and reading are
  free.
- **The whole controller faults, and a fault always needs a person.**
- **A device's trouble is the program's to handle**; it never faults the controller by itself.
- **The state is structured data;** its text has one writer and one reader, in core.
- **ORCL stays a translatable subset of real Rust**, with its own SemVer; controlador states which
  ORCL versions it runs.
- **Fixture first:** every network and every device it drives has its bench device in maquinar
  before or alongside.
- **Existing, tested Linux methods before our own** (the architect, 2026-11-16).

## Before the code
Where each item stands on 2026-11-22:
- **`docs/orcl/README.md`**, the plan the language's crates share (law L6), and the ORCL crates: the
  plan written, with the language's rules carried from Veredas 1 (`docs/orcl/language/`); `orcl-std`
  and `orcl-runtime` carried.
- **The execution model**: how one block interrupts another inside the interpreter, what tasks
  share, and each block's slice of the process image. Where it is written and what it says is Q33;
  the assistant's proposal is `docs/controlador/execution-model.md`, for the architect to rule point
  by point (Q223). No code stands on it but the one case every option runs alike, one cyclic task.
- **A study of the interpreter's front end** and of the list of core-library operations ORCL allows:
  `docs/orcl/front-end.md` (Q221, Q222).
- **The study of Linux for real time and of its recovery methods** (`docs/robustness.md`, "The Linux
  study"): `docs/controlador/linux-study.md`, its choices for the architect in Q226.
- **PROFINET International membership**: the architect's.
- **Names for controlador's own files** that do not clash with `veredas-controller`, already the
  format of maquinar's simulated-controller file: proposed in `docs/controlador/files.md` (Q224).
- **Rows in `MAP`** (`tools/leis`) for what controlador and the interpreter may depend on, each a
  contract change: the rows stand since 2026-11-05 (C-005: controlador uses portal, aranha and the
  `orcl-*` crates; `orcl-*` use each other). The interpreter in maquinar is P-020's change.

## Open
- **Q31**, the review's majors and minors, none ruled. Those that touch controlador: 15 (outputs
  without an owner at a swap; rollback's path), 16 (a single cycle pulses every output; leaving a
  dry run), 17 to 21 (the state text), 23 (vendor description files), 26 and 27 (PROFINET first;
  the EtherCAT bench device; the EtherNet/IP fork), 35 (what the controller keeps and publishes),
  36 (a bench device passing for a real one), 37 (no record of who did what), 38 and 39 (the Cyber
  Resilience Act; the fieldbus organisations' terms), 40 and 41 (no language a PLC programmer
  knows; no closed loop), 44 (libraries; structures), 46 (what a Rust engineer gets), and the
  minors 47 to 50, 53 and 55.
- **Q32** what disqualifies an edit from a live takeover. **Q33** the execution model. **Q34**
  where core's masters are tested against the bench. **Q36, Q37** the download's integrity and
  robust requests. **Q38 to Q42** the robustness round's open points. **Q43** the security round.
  **Q44** the conversion's type table. **Q45** the briefing's readings.
- **Q220** ORCL's version beside its crates'. **Q221, Q222** the front end's build and its pinned
  toolchain. **Q223** the execution model as proposed. **Q224** controlador's file names. **Q225**
  a `clippy.toml` under law L7. **Q226** the Linux study's choices.
