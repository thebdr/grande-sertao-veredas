# veredas-controlador: the plan

The soft PLC of Veredas 2. `docs/veredas-plan.md` ("veredas-controlador: the softPLC") wins on
intent; `.zen/contract.md` (P-003) wins on status. Built so far: ORCL's standard library carried
from Veredas 1 (`orcl-std`, C-150), the engine of a scan (`orcl-runtime`, C-151), the controller's
states and commands over one cyclic task (C-152), the interpreter's machine
on programs built by hand and its program file (`orcl-interp`, C-153 and C-154, 2026-11-25), and an
interpreted program on the engine, its program errors faulting the task (C-151, C-155) and the
controller as 502 `program(at)` (C-152). This
file holds what the architect decided in the briefing of 13 to 16 November 2026, in the rulings of
15 November on its adversarial review, in the robustness round begun on 16 November
(`docs/robustness.md`), and in the rulings of 25 November on the questions its first build raised
(Q220 to Q229, and Q33 with Q223), on the robustness round (Q35 to Q42) and on the review's entries and
the briefing's readings (Q31 up to entry 41, Q32 in part, Q44, Q45), and in the rulings of 26 November
(Q31's remaining entries, Q32's specification, Q43 in part, Q230 to Q237). The list that was reviewed and the review itself are kept as they were
delivered (`docs/briefing/deliverables.md`, `docs/briefing/review.md`); where they differ from this
file, this file is the later word. What is not decided is a question in `docs/questions.md`, named
under "Open" below; nothing open is written here as settled.

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
- **Production on Linux with a real-time kernel only.** Windows is for development and simulation,
  and gains no third use: the IO check from a laptop runs on a Linux laptop, where controlador runs
  for real, and a Windows laptop's goes through a target ("Linux laptops only", 2026-11-25, Q45). A
  production target refused real-time scheduling does not run: it stays in STOP.
- **64-bit processors only** (the architect, 2026-11-16), **and 64-bit systems**: the build refuses a
  32-bit target, whose 32-bit time value would come back (2026-11-25, Q39).
- **Programs run in our own interpreter**, a crate of the ORCL family that maquinar uses too. Its
  bytecode is made from the Rust compiler's intermediate form, through `rustc_public` as it is
  ("rustc_public is ok as is / proceed as decided", the architect, 2026-11-15), which means one
  pinned nightly toolchain.
- **The front end and the cut at core** (2026-11-25): the front end is `crates/orcl-front`, built on
  stable, its driver behind a cargo feature only the pinned nightly turns on (Q221); the nightly is
  `nightly-2026-11-21`, moved on purpose a few times a year at most, never for a release being
  commissioned, each move gated by the conformance suite (Q222); a program's calls into core are a
  closed list of the interpreter's operations, a call outside it refused by name and place when the
  program is lowered (Q227). `docs/orcl/README.md`, "The front end".
- **The interpreter is proven against Rust by a fixed conformance suite.**
- **What an interpreted program runs on** (2026-11-26): a stack of the size the target's settings give,
  laid out at load, on which every program Rust compiles runs, one whose functions call themselves
  among them, a call past it being the fault 502 `program(at)` (Q230); its first state and its safe
  outputs computed when the front end builds it and written into its file, a program whose
  `safe_outputs` reads its state refused (Q231); no instruction budget, the entry running to its
  return and a scan that never returns waiting for the host's watchdog, as a native one does (Q232).
  Until the target's settings (`veredas-alvo`) and the program file's format past 1 are built, the
  controller's host hands in all three, as it hands in the program (C-152, C-155).
- **No check of a program's structure before it runs**, for now: "rust as a base gives us an
  optimal linter and compiler. no further checks necessary (for now)" (the architect, 2026-11-16).
  The compiler and the linter on the sender's machine stand for the source, and the download's
  hash for the program that arrives.
- **The program's clock pauses while the program is halted.**

### The execution model
Ruled on 2026-11-25 as proposed (Q223, which answers Q33), point by point:
1. **Where it is written:** `docs/controlador/execution-model.md`, this list carrying each point.
2. **Tasks share only the process image** (K8, ORCL's O7): a value two tasks share is one task's
   output and the others' input, copied at scan boundaries; a force, a write through the embedded
   portal and a device event reach a task at its scan boundary, never inside its scan. Not ruled
   with it: whether O7's publish at the end of the writer's scan gives way to the prior art's LET.
3. **Each task its own slice of the process image** (K10): an output in one task's slice only, an
   input latched by every task that reads it, a slice copied whole.
4. **One logic thread** in the first release, itself under the kernel's real-time scheduling,
   switching between tasks at the interpreter's instruction boundaries: a hit halts every task, and
   a field incident replays on the simulator.
5. **The fieldbus has its own thread**, above the logic and never halted: halt values at a halt,
   safe values in FAULT, and the logic's watchdog at twice a scan's period (fault 501).
6. **Time:** the program clock, paused at a halt; time-of-day blocks on the confirmed date alone; a
   start that fell during a halt dropped (K9); the main cycle held to the maximum cycle time,
   pre-emptions included.
7. **Start-up, handlers and stop:** start-up blocks before the first main cycle, with no cyclic
   interrupt; a handler is a task its event releases; a program error ends its block's scan, then
   the program-error handler runs or the controller faults (502); a stop runs the stop handler's
   one last scan.
8. **The shadow test's period check with several tasks** (K11) is put off by the first release's
   one cyclic task.

### On the target
The Linux study's recommendations, all accepted on 2026-11-25 (Q226;
`docs/controlador/linux-study.md`):
- **The kernel:** the Civil Infrastructure Platform's ten-year line (6.12-cip to 2035).
- **Updates with fallback:** SWUpdate with EFI Boot Guard, CIP's own reference; "booted well"
  means the controller's process up with its program loaded and its first cycle within time.
- **The system partition:** read-only under dm-verity with correction (2 parity bytes, about
  0.8 %); uncorrectable damage restarts into the other slot, and the count of corrected blocks is a
  maintenance request.
- **The program's two copies:** Btrfs (RAID1, or DUP on one drive), repairing on read as well as on
  scrub.
- **Time:** chrony, a clock chip that reports (`RTC_VL_READ`), systemd's 15-year rewind built out
  of the system image.
- **The watchdog chain:** firmware or bootloader, kernel, PID 1, and the controller's own process
  owning the last ping.
- **Logs** volatile, with pstore keeping a crash's record.
- **What a target must have:** ECC memory, a drive that reports its wear (eMMC or NVMe, not SD),
  memory that keeps its content where retained data is saved every cycle, a power-fail line where
  there is hold-up power; the target's hardware description, which Q40's "who says what a target
  can do" reads.

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
  afterwards (`docs/robustness.md`, "Time over decades"); nor does a start that fell while the
  controller was powered off (2026-11-25, Q39). For a program with such blocks the confirmed date is
  inside what must survive, so its target's hardware description names a clock chip that keeps
  time (Q38).

### Start, stop and power
- **Start kinds:** warm (retained variables kept; the normal start, also after a power cycle) and
  cold (everything reinitialised, on command). Start-up shows its phase: load, restore, handler,
  devices. After the start-up time the controller goes to RUN regardless of missing devices and
  shows what is missing.
- **After a power cycle**, as Siemens does: RUN returns as RUN; STOP, HOLD and FAULT all return as
  STOP, the reason saying what it was. "Always as before" (the architect, 2026-11-15): no start-up
  parameter, no STOP forced by a commissioning condition, no rule that ends a shadow test; the
  program handles the rest.
- **A program that cannot be made, until downloads exist** (2026-11-26, Q233): a controller whose
  program, handed in by its host, cannot be made comes up in STOP 104 `no_program`, at power-on
  whatever state it would come back to and at a start's load alike; a person's start tries it again,
  nothing by itself. A stored program failing its check is the FAULT 506 `load(download)` once
  downloads exist (Q36, "Everything that alters the controller is robust" below).
- **A dry run survives a power cycle** (2026-11-15): RUN with the dry run on comes back the same,
  outputs safe until a person ends the dry run.
- **Nobody switches states in normal work**: "we do not switch states in normal working
  conditions, ever" (the architect, 2026-11-15). RUN is the only state with no user interaction
  going on; a fault needs a person, and a triggered stop follows the same logic.
- **Where this plan was silent, the first build's nine readings stand** (Q228, 2026-11-25): in
  STOP the outputs take their safe values and the plant's exchange goes on; a controller that never
  ran comes up in STOP with no reason; a start-up a power cut interrupted returns through a warm
  start; warm and cold starts both begin from the initial values until ORCL can mark a variable
  retained; the start-up's phases pass at once with nothing to restore, no handler and no device;
  the condition `overruns` counts the time errors since the last start; a command that would
  change nothing is refused, naming the state; a runtime restarted while the machine was not comes
  up in `FAULT internal`; a stop from STARTUP is allowed.

### Monitoring, forcing, debugging
- **Live values** of any variable; watch tables.
- **Forcing** covers inputs, outputs and variables. A variable is forced in one of three ways,
  chosen at the time: once now, before each scan, or always (held at every write, marked `[!]`,
  behind a confirmation). Tables always show the mode in words, never the mark alone (the review's
  minor 49, Q31). "Force once now" on a variable is the online change of a value, TIA's Modify: the
  documents say so, since a one-value fix is otherwise a whole deployment (the review's minor 48).
- **A structure expands member by member** in watch tables, forcing and the IO mapping (2026-11-26,
  the review's entry 44, Q31).
- **A force belongs to the controller** until someone removes it (2026-11-15): it shows in the
  state text, a restart clears it, and a lost connection changes nothing. A force suspended while
  the program is halted applies again by itself when the program continues (2026-11-25, Q45's K3).
- **Breakpoints and stepping** on the real target and in the simulator. A hit halts every task.
- **Outputs at a halt** follow a per-output setting: safe by default, held if the engineer chose
  it. A forced output follows the same setting. Each target has a longest time outputs may stay
  held at a halt, short by default; when it runs out the held outputs go safe and the controller
  stays halted (2026-11-15).
- **A halted target stays halted** if Oficina disconnects.
- **Pause on command** (the program halts at the end of the scan), **single cycle** (one scan,
  then halted again) and **dry run** (the logic runs on live inputs, every output stays safe).
  Since a single cycle would energise every output its scan writes for milliseconds, on a real
  target it runs only in a dry run, or with outputs held between cycles after a confirmation that
  says so; and leaving a dry run goes through STOP and a start, so no output jumps to a value from
  the middle of a sequence (2026-11-25, the review's entry 16, Q31).

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
  not. With the mapping allowed to differ, the comparison is per variable name, not per physical
  channel (2026-11-25, Q45's K4), so a signal moved to another channel shows as one line; an output
  channel that loses its writer at the swap goes to its safe value, the popup before the swap
  listing each one for confirmation; and a rollback is a download like any other, through the shadow
  test or through a stop, never a direct replacement (the review's entry 15, Q31).
- **An unaccepted shadow test at a power cycle is dropped**: the version that was driving drives
  again, and a person starts the test again (2026-11-25, Q45).
- **Entry check:** a controller enters a shadow test only after a check that the doubled scan fits
  the task's period. In the first release only an application with one cyclic task may enter one
  (2026-11-15). When the doubled scan does not fit, the way is a stop and a download, and the popup
  says so (the review's minor 48, Q31).
- **Breakpoints only in the driving version**; a hit halts everything and the watcher pauses in
  step. Trouble in the watching version (an error, or an overrun from the doubled scan) ends the
  test and the machine keeps running.
- **Fallback for a faulted edit.** A controller-wide flag, offered in the download popup, with a
  time limit, reset on shutdown. When the time limit runs out the edit is accepted and the old
  version stops watching; the limit is the engineer's, a setting of each download, ten minutes by
  default (2026-11-26, Q32), and an edit so accepted is recorded as "accepted by timeout" in the
  download history, and as an event (the review's minor 47, Q31). The review showed that taking
  over "from its own watcher data" is unsafe as first written, and the architect ruled (2026-11-15): "live fallback shall be enabled
  only when feasible. we need to build the specification for what disqualifies the edits for auto
  takeover. takeover after restart remains possible for all cases". So the previous version can
  always be put back for the next start; a live takeover happens only for an edit that nothing
  disqualifies; and what disqualifies an edit is a specification controlador wrote whole, as a
  proposal the architect ruled as a document (Q32): `docs/controlador/live-takeover.md`, written on
  2026-11-25 and ruled on 2026-11-26 ("Yes, this is the specification"), what peers see left out of
  its disqualifiers (its S3, "No, S3 out") and any force ending the old version's right to take over
  (its D3). After a fallback the state carries
  the maintenance request `fault_on_update`; an edit that failed (the old version took over, or the
  edit was put back) leaves a fault record named "the edit failed", not a controller fault; and
  while the old version watches and may still take over, the state text carries a term with the
  time it has left (Q32).
- **The last 10 downloads are kept** on the controller: hash, time, origin, kind, source revision,
  the program itself and the source archive. Keeping the source on the target is a project setting,
  on by default (2026-11-25, the review's entry 35, Q31).

### Faults
- **The whole controller faults, never one task alone.** One kind of fault only.
- **In FAULT every output takes its declared safe value**, with no per-output exception, and the
  fieldbus keeps exchanging.
- **Leaving FAULT takes two actions:** acknowledge to STOP, then start.
- **Reasons:** the watchdog; a program error without a handler; a fault raised by the program with
  a code; a fault inside a handler; resources exhausted (memory; a call past an interpreted program's
  stack is the program's own error, 502, Q230); an application that cannot be loaded; the runtime's own failure (after an unexpected restart it comes up in FAULT,
  which stops a crash loop).
- **Overruns follow Siemens' OB80:** the time-error handler runs once on the first overrun, and
  the controller faults only when one scan exceeds twice its period.
- **A maximum and a warning cycle time**, both set by the user (the architect, 2026-11-16): "the
  warning after years is a hint of possible degrading". The warning raises a maintenance request,
  not urgent, with a reason of its own beside `overruns`; the maximum gives a free-running main
  block its limit; and the controller records each task's longest cycle per day, so a drift shows
  before the warning (2026-11-25, Q39).
- **A fault clears every debug condition** and records what was active.
- **A fault record** holds the cause, the time, the download number, the task, the source
  location, the debug conditions that were active, a snapshot of the data, the last seconds before
  it, and the handler's outcome. Its details are in the popup and in the fault history. On `main`
  the snapshot is portal's copy of its store at the fault (C-093), which holds what the store
  holds. It is all the controller's data only once portal reads the controller's memory, which
  is not built yet (see "The engineering link"). Peers get a fault's cause, number and time; the
  snapshot is read through Oficina only (2026-11-25, the review's entry 35, Q31).
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
| STARTUP | 2 | 201 `warm`, 202 `cold`; its phases are a small enum of their own: 1 load, 2 restore, 3 handler, 4 `devices(up, of)`, each shown by its word alone, no `id` (Q45's K5) |
| RUN | 3 | |
| HOLD | 4 | 401 `breakpoint(at)`, 402 `command(source)`, 403 `step`, 404 `cycle` |
| FAULT | 5 | 501 `watchdog(task)`, 502 `program(at)`, 503 `user(code)`, 504 `handler(event)`, 505 `resource(kind)`, 506 `load(download)`, 507 `internal` |

- **Command sources:** `oficina`, `local` and `program`. The source `peer` was withdrawn on
  2026-11-15, with the rule that no peer changes the state.
- **Conditions, in this order:** `breakpoint(set)`, `force(in, out, var)` (its arguments always in
  that order, Q45's K1), `shadow(drives,
  watches)` (what someone did to the controller); `dry`, `sim`, `no_realtime` (the mode it runs
  in); `devices(missing)`, `overruns(count)`, `retain(reset)` (what it is suffering); then each
  `maintenance_request(id, reason, urgent)`. `breakpoint(at)` appears only on HOLD; `shadow` names
  download numbers.
- **Maintenance reasons:** 1 `fault_on_update` (urgent), 2 `device` (as the device reports), 3
  `device_missing` (urgent), 4 `overruns` (not urgent), 5 `retain_reset` (not urgent), 6 `user`
  (as the program states). `urgent=false` is PROFINET's maintenance required, `urgent=true` its
  maintenance demanded.
- **A maintenance request is cleared only from Oficina or by a restart**, and is raised again if
  its cause persists. A request carries a "seen" mark, so clearing one whose cause persists does not
  look like a dead button, and the detail view decodes the `shadow` term in words (2026-11-26, the
  review's minor 50, Q31; the text itself stays English, its details translated).
- **Examples:** `RUN`; `RUN breakpoint(set=2) force(in=1, out=2, var=3) shadow(drives=13,
  watches=12) sim()`; `HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)`;
  `STOP command(id=101, source="oficina")`; `FAULT watchdog(id=501, task="main")`; `STARTUP
  warm(id=201) devices(up=3, of=5)`; `RUN maintenance_request(id=3, reason="device_missing",
  urgent=true)`. At a halt a forced output follows its own halt setting, which is why `out=2`
  became `out=1` between the second example and the third.
- **Health.** The standard health value (NE 107, OPC UA's DeviceHealth) is derived from the state:
  FAULT gives failure; STOP, STARTUP, HOLD and any debug condition give function check; a
  maintenance request gives maintenance required; a clean RUN gives normal.
- **Ruled with Q100** (2026-11-25): the review's entries 17 to 21 on this text (one name meaning
  two things, the spelling rules, the `id` of a maintenance request, a version for the text, what a
  debug condition is), as core's writer and reader took them. The terms the rulings of 2026-11-25 add join core's table of the state text,
  their spelling and numbers core's to propose: conditions for an unconfirmed date (Q39), a start
  from the last save (Q40, a condition and no request) and an armed fallback with its time left
  (Q32); maintenance reasons for the clock chip that lost its time and for NTP silent or not set
  (two, an NTP silent only after a waiting time, the urgency following the present situation while
  the request stays until cleared, Q39), a certificate a year from its end (its own and its peers',
  Q39) and a known peer's certificate past its end, urgent until it is renewed (Q43), the warning cycle time (Q39), a drive's wear past its threshold and a data area that cannot
  be written (Q41), and a retained save that fails (Q40).

### Networks and devices
- **Four networks in the first release**, built in this order: PROFINET IO controller (written
  here on aranha's codecs, since no open controller exists; join PROFINET International and
  certify later), then EtherCAT master (on the ethercrab crate), EtherNet/IP scanner (fork a young
  Rust crate, test on real devices), Modbus TCP client (existing crates).
- **Modbus TCP proves the runtime first** (2026-11-25, the review's entry 26, Q31): it is the bus on
  which the runtime is first proven while the membership runs; PROFINET stays the first fieldbus
  written, and from its first clause the PROFINET work has a real device in the loop and captures of
  a real controller's traffic as fixtures. Until a network is certified, controlador is written
  "compatible with" it (the review's entry 39).
- **The functions that drive hardware live in `veredas-core`**: safe in controlador on Linux, and
  callable by other crates for testing only ("testers should be near a kill switch or estop
  anyway"). The architect kept this on 2026-11-15 against the review, with core's rules amended
  (`docs/core-plan.md`; `docs/questions.md` Q9).
- **Vendor description files** (GSDML, ESI, EDS) are copied into the project and read directly;
  the chosen configuration is written as Veredas TOML. They are inputs, read and never written, the
  TOML rule's one named exception, parsed with document type definitions and entities switched off;
  their readers live in core (2026-11-25, the review's entry 23 and Q45's C5).
- **The hardware configuration** stands on an embedded aranha, whose scan feeds the compare and
  merge of the configuration and the topology view. Identifying, naming and addressing a device runs
  from the laptop when it is on the segment, and through a target's embedded aranha when it is not
  (2026-11-25, Q45's G2); a Windows laptop sends and hears raw frames through Npcap, which the
  engineer installs from its publisher, aranha neither shipping nor starting its installer
  (2026-11-26, aranha's Q281).
- **IO check** runs through a controlador target; with none set, through the laptop session
  (`docs/oficina-plan.md`, "The IO check from a laptop"), on a Linux laptop only (Q45). It refuses
  to drive outputs on a Modbus module with no timeout of its own or too long a one; the hardware
  configuration checks that timeout in production too, as a warning the engineer may accept
  (2026-11-25, Q45).
- **A bench device never passes for a real one** (2026-11-25, the review's entry 36): bench devices
  carry an identity that controlador refuses outside simulation, and each interface a bench uses is
  declared isolated in its bench file (the bench's side).
- **Fixture first:** the bench devices for all four networks live in maquinar
  (`docs/maquinar-plan.md`). Core's masters are tested against them in the test-only tool
  `tools/mestres`, and controlador's own tests sit beside them when it drives real devices (Q101
  with Q34, 2026-11-25).

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
- **Pairing is one step** (2026-11-26, the review's entry 42, Q31): Oficina creates the SSH key and
  installs it on the target. Not chosen: opening the project from the target's kept source archive
  as one of controlador's surfaces, and one connection view.
- **A controller's first credential** (2026-11-26, Q43): "credential-less usage permitted,
  discouraged", discouraged by Oficina's warning at each connection to a controller without one; the
  controller itself raises nothing.
- **Certificates of known peers** (2026-11-26, Q43): a secured link to a known peer whose
  certificate has passed its end goes on, an urgent maintenance request standing until it is
  renewed; while the controller's date is unconfirmed a peer whose certificate it already trusts is
  accepted without judging dates, the link marked so, and an unknown certificate is refused as
  always. The end date of certificates issued after 2038 waits for the security study.
- **Loopback is no boundary** (2026-11-25, the review's entry 33): controlador's engineering channel
  is a Unix socket (a named pipe on Windows) with the system's permissions, and portal's HTTP
  interface takes a random token per session, handed over SSH and required on every request, with
  Host and Origin checks.
- **An intervention log** (2026-11-25, the review's entry 37): append-only, on the target: download,
  swap, accept, rollback, force set and removed, breakpoint, start and stop with their source, switch
  change, each line with the time and who (the SSH user, the peer's address, a name the engineer
  types once per Oficina installation), its retention set per target. The documents state that
  controlador is not a safety component.
- **Everything that alters the controller is robust** (2026-11-16), a download above all: it
  carries a hash, "so no corrupted program may run". Ruled point by point on 2026-11-25:
  - **a download** (Q36) is a manifest listing every part with its SHA-256 and length, the
    manifest's SHA-256 being the download's identity. The controller stores it to a temporary
    file, reads it back from the drive and verifies it, and only then renames it into place,
    answering with the identity it computed. The stored program is verified again before every
    load (a swap, a restart, a power-on); a check that fails where the two copies cannot mend each
    other is the FAULT 506 `load`, never a substitute. Swap, start and rollback name the identity
    they act on. A signature over the manifest waits for the security study (Q43, 2026-11-26);
  - **every request that alters the controller** (Q37), a variable write, a breakpoint, a state
    change and a download alike, carries a SHA-256 of its content (as HTTP's Content-Digest, RFC
    9530), checked before anything is done; a session and a strictly increasing number, a repeat
    answered with its first outcome and not run again, a gap or an old number refused; the
    controller's identity, the download's, and for a state change the state it expects to leave;
    and the controller's own last clock value, so a request held back and delivered late is
    refused. A change is applied whole at one point of the scan; its answer is read back from the
    controller, a write's telling applied and saved apart; a lost answer is shown in Oficina as
    unknown and asked for by the request's number. A command passes portal's master write switch
    and then the commissioning switch. This binds the Oficina link, not peers' writes. A keyed
    check (HTTP Message Signatures, RFC 9421) waits for the security study (Q43, 2026-11-26); the
    refusal of an unexpected `Host` and the random session value are the engineering link's (entry
    33, above).
- **State changes and faults are stored like any other data**, and the history view reads them.
- **By default the embedded portal keeps its recorded values in memory, as a fixed window**, and
  writes nothing to the drive (2026-11-19; `docs/robustness.md`, "Wear and filling"). Two
  additions came with that ruling and were not objected to: the project may turn on recording to
  the drive (C-091), and at a fault portal writes its store to the drive with the fault record
  (C-093) when the embedding names a fault folder. Portal built them and the window on three
  answers it took meanwhile; the architect ruled them on 2026-11-25 (Q41), and what portal built
  gives way where the rulings differ:
  - a growing record on the controller's drive (fault records, the state history, earlier
    downloads, logs, a recording, the fault's copies) loses its oldest at its size limit, a fault
    that repeats being counted on its record and not written again, the running program and its
    fallback never deleted (C-091 as built stops at its limit instead);
  - the fault's copies of the window are limited in number, the oldest going first, no more than
    one written per shortest gap, and the program may ask for one (C-093 as built keeps them all);
  - the window has a span and a ceiling in bytes, the shorter of the two, the controller saying
    so when the ceiling came first (C-090 as built has a span only); it holds everything portal
    reads unless the project narrows it.
- **Controlador owes the embedding**, none of it built yet: portal as a dependency, its controller
  source fed with every state and fault record (C-092), and the fault folder named (C-093).

### Language and project data
- **ORCL text first; ladder later**, as a read-only view of the same source in release 1, editable in
  release 2 (2026-11-26, the review's entry 40, Q31).
- **What a Rust engineer gets** (2026-11-26, the review's entry 46): an ORCL project is a cargo crate
  whose logic `cargo test` runs; the checker works outside Oficina and in CI; the allowed subset of
  Rust is published as one page.
- **A library is a pinned crate** (2026-11-26, the review's entry 44): reused blocks come from a
  crate named by path or by Git revision, pinned in the project file, with no build scripts.
- **What is handed over** (2026-11-26, the review's entry 45): the IO list, the alarm list and the
  cross-reference export to CSV and PDF, among controlador's surfaces; one example project per
  network; context help in the editor.
- **Project data:** multilingual text lists, alarm and event definitions, recipes, units and
  limits on variables.
- **A simulator** with the bench as plant, virtual time and snapshots.
- Checked on 2026-11-15: Veredas 1's `orcl-std` holds the on-delay, off-delay and pulse timers,
  both edge detectors and the up counter, and nothing else. **The standard library is a named
  first-release item and is completed**, with the IEC names and behaviour: the down counter and the
  set and reset latches join it (2026-11-25, the review's entry 40, Q31).

### Conversion to native PLC code
The goal, stated outright by the architect on 2026-11-15: controlador's ORCL program **and its
hardware configuration** convert to native PLC code, Siemens S7-1500 and S7-1200 first. Veredas
1's ORCL notes already planned the program translators, Siemens first; the hardware configuration
is new. The architect began a table mapping ORCL data types to S7 types, still a draft outside this
repository. Decided on 2026-11-25 (Q44): the types below `f64` (`char`, text, time, date) become
`orcl-std`'s own types with a fixed representation; the table has a column per CPU family, a program
using a type its target lacks being refused for that target; and a second front end that reads the
source, for readable SCL and the ladder view, is "not yet", decided when the conversion is the work
in hand.

### What keeps it running for decades
The robustness round's rulings, stated in `docs/robustness.md`, bind controlador and its embedded
portal, and no other component (2026-11-25, Q42):
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

Ruled on 2026-11-25:
- **What must survive** (Q38): the running program and its source, the hardware configuration and
  IO mapping, the retained variables, the settings a scan depends on, the interpreter, the
  operating system and what boots them, the integrity data, what peers need to talk to the
  controller when the project uses them, and for a program with time-of-day blocks the confirmed
  date. Outside: forces, breakpoints, a shadow test, the commissioning switch, earlier downloads,
  fault records, the state history, the store and the logs, which may be lost but must never harm
  what is inside. A dry run survives a power cycle without that protection.
- **The retained save** (Q40) holds the retained variables as they stood at the end of one
  successful cycle, never a mix of two, in two copies each with its hash, written one after the
  other; with both unreadable at start the retained variables reset and `retain_reset` is raised.
  A save equal to the last is skipped. Its period is the engineer's and never refused, Oficina
  showing the years the drive has left at it; a write from outside is applied at once and saved
  when the shortest gap (the engineer's, with a default) ends; the program may ask for a save,
  held to that gap. On a target that cannot save at power loss, the flag "started from the last
  save" is raised only when cycles ran since that save. A save failing while the controller runs
  leaves RUN going, its maintenance request not urgent while one copy can still be written and
  urgent when none can.
- **The unconfirmed date** (Q39) marks the controller's fault records, its state history and every
  sample of the embedded portal's store, whose format gains the mark by a new version.
- **Storage in three kinds** (Q41), kept apart so wearing or filling one cannot touch another: what
  RUN needs (written only at a download), the retained save, and everything that grows, on a data
  area of its own. Where the drive reports its wear the controller reads it and raises a request,
  not urgent, at a threshold the engineer sets. When the data area fails, RUN goes on, recording
  stops and a request is raised.

Ruled on 2026-11-26, the four areas Q42 left unopened, proposed by controlador:
- **Running unattended** (Q234): a corrected memory error is counted, and a rate past a threshold
  raises a maintenance request; an uncorrected one the kernel reports in the controller's memory is
  the runtime's failure, FAULT 507, the program's data no longer trusted. The controller reads its
  own resident memory, open files and threads against limits set when it starts: a trend rising over
  days raises a maintenance request, a limit reached is FAULT 505 `resource`. Nothing restarts on a
  schedule.
- **An old controller and a new Oficina** (Q235): Oficina speaks every older controller's versions
  for everything short of a download (connecting, reading the state and records, watching,
  forcing), reading the controller's versions when it connects and writing only what that controller
  reads; it refuses to download a program to a controller older than its own release and offers the
  controller's update first, with its own rollback. No interface is frozen from release 1 as a
  rescue path.
- **Replacing the hardware** (Q236): a backup is one file, a manifest listing its parts with their
  SHA-256 as a download's does: the program and its source, the hardware configuration, the IO
  mapping, the target's own settings, the retained data at that moment, and a rebuild sheet in plain
  text for someone with no knowledge of Veredas (the board and drive it needs, the system image's
  version and where to get it, the network names and addresses, what to install and in what order).
  Oficina makes it from the controller and restores it to any target that meets the requirements,
  checked by its manifest, a person choosing whether the retained data is restored or the program
  starts cold. Its format is a `veredas-*` format with a version (law L7).
- **Proving the rules** (Q237): a fault catalogue, a named fault the bench injects for every rule (a
  power cut at a chosen instruction, a drive that fails a write or reports wear, a clock that jumps
  or an NTP that lies, a device lost mid-exchange, memory full), each linking the test of the clause
  that builds the rule; a long soak before each release, weeks of virtual time with faults drawn at
  random from the catalogue and the controller's invariants checked throughout (no output but a
  computed or a safe one, a state that reads back, nothing that grows); and a few power cuts on real
  boards before a release. The catalogue and the soak are the bench's to carry too
  (`docs/maquinar-plan.md`).

### Out of scope
Technology objects entirely (axes, PID, cams, counters): "technology objects later", and release 1
says which machines it serves, discrete IO, no closed loops, no axes (the architect, 2026-11-25, the
review's entry 41; no PID block in the standard library now, no import from PLCopen XML). Security
features beyond what the security round ruled on 2026-11-26 (Q43; "The engineering link" above), until
the security study the architect postponed: the end date of certificates after 2038, a signature over
a download's manifest and a keyed check on altering requests wait for it; under the Cyber Resilience Act the suite's intent is not
decided yet, and before the first public release come a security file with a reporting channel and
a component list, the security study's first output being the gap list against its Annex I (the
review's entry 38). Certified functional safety. Know-how protection for now: a way to protect a
block's source is a requirement, later (2026-11-25, Q45). Synchronous cycle, motion control and
redundancy blocks.

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
- **ORCL stays a translatable subset of real Rust**, with a version of its own as a language,
  ORCL 1, 2 and so on, as Rust has editions: `orcl-std` states it, a compiled program's header
  records it, and controlador names the versions it runs; the crates keep their SemVer (Q220,
  2026-11-25).
- **Fixture first:** every network and every device it drives has its bench device in maquinar
  before or alongside.
- **Existing, tested Linux methods before our own** (the architect, 2026-11-16).

## Before the code
Where each item stands on 2026-11-25:
- **`docs/orcl/README.md`**, the plan the language's crates share (law L6), and the ORCL crates: the
  plan written, with the language's rules carried from Veredas 1 (`docs/orcl/language/`); `orcl-std`
  and `orcl-runtime` carried.
- **The execution model**: ruled, "The execution model" above; code stands on one cyclic task so
  far.
- **A study of the interpreter's front end** and of the list of core-library operations ORCL allows:
  `docs/orcl/front-end.md`, its choices ruled (Q221, Q222, Q227). The front end's plan is a section
  of `docs/orcl/README.md`, which law L6 reads for every `orcl-*` crate; the interpreter's design is
  `docs/orcl/interpreter.md` (Q230, Q231 and Q232 ruled on 2026-11-26).
- **The study of Linux for real time and of its recovery methods** (`docs/robustness.md`, "The Linux
  study"): `docs/controlador/linux-study.md`, its choices ruled ("On the target" above).
- **PROFINET International membership**: the architect's.
- **Names for controlador's own files** that do not clash with `veredas-controller`, already the
  format of maquinar's simulated-controller file: `veredas-aplicacao` (the application),
  `veredas-rede` (the hardware configuration), `veredas-alvo` (the target's own settings),
  `veredas-carga` (a download's manifest) and `<name>.orclb` for a compiled program, ruled
  (Q224; `docs/controlador/files.md`).
- **Rows in `MAP`** (`tools/leis`) for what controlador and the interpreter may depend on, each a
  contract change: the rows stand since 2026-11-05 (C-005: controlador uses portal, aranha and the
  `orcl-*` crates; `orcl-*` use each other). The interpreter in maquinar is P-020's change.

## Open
- **Q31**, the review's majors and minors: every entry is ruled, each written above where it binds
  controlador, but minor 53 (items that wait on undecided work, marked so in the plans) and minor
  55 (ruled with Q228's point 8).
- **Q43** the security round: its list as it stood is gone through (2026-11-26); the end date of
  certificates after 2038, a signature over a download's manifest and a keyed check on altering
  requests wait for the security study.
- Left open by Q223's ruling: whether O7's publish at the end of a writer's scan gives way to the
  prior art's LET.
