# Controlador's execution model

**Ruled on 2026-11-25, as proposed** (`docs/questions.md` Q223, which answers Q33): the architect accepted every
point below, the interrupting on one logic thread (5 (b)) for the first release. Open with it: whether O7's publish
at the end of a scan should give way to the prior art's LET (section 3). This file is the assistant's proposal of
2026-11-22, kept as it was ruled; `docs/controlador-plan.md` ("The execution model") carries each decision in a
line and wins on intent. Code stands on one case so far, a program with one cyclic task (the engine of a scan,
`docs/orcl/README.md`).

## What the model must answer
From the review's entry 13 (`docs/briefing/review.md`) and the deliverables K8 to K11:
- what tasks share, and whether only through the process image (K8);
- whether each block has its own slice of the input and output image (K10);
- how a more urgent block interrupts a running one inside the interpreter;
- the fieldbus cycle's own thread, which a breakpoint must not halt;
- the limit of a free-running main block that more urgent blocks starve;
- the shadow test's period check with several blocks that pre-empt one another (K11), put off by the first
  release's limit of one cyclic task (2026-11-15).

## Already decided, and what it implies
- **The entry points** (`docs/controlador-plan.md`, "Entry points"): several main blocks and several start-up
  blocks, run in order; cyclic interrupts with their own period and priority; time-of-day and time-delay blocks;
  handlers; a more urgent block interrupts a running one. No hardware interrupts in the first release.
- **A hit halts every task**; the program's clock pauses while the program is halted; on continue nothing missed
  runs; outputs at a halt follow their halt setting; in FAULT every output takes its safe value and the fieldbus
  keeps exchanging.
- **The whole controller faults, never one task alone.**
- **ORCL's O7, decided by the architect on 2026-10-06 in Veredas 1** (`docs/orcl/language/ownership.md`): "A value
  shared between tasks is an output of exactly one task, its writer, and an input of each task that reads it",
  copied at the scan boundaries, "the writer's values published together at the end of its scan", "a reader takes
  its copy at the start of its scan". The review listed K8 as the assistant's reading; it is in fact the language's
  rule, decided before the briefing. That O7 answers K8 for the controller was Q223's second point, ruled so.

## The proposal

### 1. Where it is written
Here, a document of its own beside the plan, so each point can be ruled apart and the plan stays a statement of
what was decided. Once ruled, the plan's "Execution" carries each decision in a line and points here.

### 2. Terms
- A **block** is an entry point of the program: a function the program marks with its kind (how a program marks
  it is the language's, open).
- A **task** is where blocks run: the main cycle (every main block, in order, then again), each cyclic interrupt,
  each time-of-day block, each time-delay block, each handler, and start-up (every start-up block, in order, once).
- Every task has a **priority**, the engineer's, with defaults in Siemens' order: the main cycle lowest, then
  time-of-day and time-delay, cyclic interrupts, the device handlers, and the time-error and program-error
  handlers highest. A task interrupts only a task of lower priority; tasks of one priority run one after another
  in the order they came due.

### 3. What tasks share (K8): only the process image
O7 as decided: a task's data is its own (every function-block instance belongs to one task); a value two tasks
share is the output of one and an input of the others, copied at scan boundaries. The controller adds the same
rule for everything that reaches a task from outside: a force, a write through the embedded portal and the device
events are applied to a task's image at its scan boundary (O8), never in the middle of its scan.

What it buys: no lock inside a program, a program's tests with no concurrency, and a task's outputs that depend only
on its latched inputs, its own state and its scan's time.

What it costs, said plainly: what a reader sees depends on when the writer's scan ended, so the order in which two
tasks' scans end can change a value a third reads. The prior art's LET (publishing at the end of the writer's
logical period) removes that at the cost of up to one period of latency; it was proposed in Veredas 1 and not
decided (`docs/orcl/language/README.md`, "Not settled yet"). This proposal keeps O7 as decided; so did the ruling,
and whether LET should replace it stays open.

### 4. Each task its own slice (K10): yes
- Each task has its own **input slice**, latched whole at the start of its scan, and its own **output slice**,
  published whole at its end.
- A physical output is in exactly one task's slice: one writer, checked when the configuration is built.
- A physical input may be in several tasks' slices; each latches its own copy at its own start, as Siemens'
  process image partitions do. Two tasks may therefore read one input in two states within one bus cycle.
- A slice is copied whole, so no task ever reads a value of several words half-written: the bus thread and a task
  exchange a slice through a buffer that hands over whole copies (a latest-value handoff), never field by field.

### 5. How one block interrupts another
Two ways, both kept possible by points 3 and 4 (each task's context is independent of the others'):
- **(a) A real-time thread per priority.** Each priority level is a `SCHED_FIFO` thread at its own level, each task
  an interpreter context; the kernel pre-empts. A halt or a breakpoint sets a flag that every context reads at
  each instruction boundary, so every task stops within one instruction.
- **(b) One logic thread, switching at instruction boundaries.** One `SCHED_FIFO` thread runs every task's context;
  when a more urgent task comes due, the interpreter saves the running context at the next instruction boundary and
  runs the urgent one. The latency of a pre-emption is the longest single instruction.

| | (a) a thread per priority | (b) one logic thread |
|---|---|---|
| Who pre-empts | the kernel, a tested method | our own scheduler inside the interpreter |
| A hit halts every task | a flag read by every thread | by construction: one thread |
| A run replayed in the simulator | the kernel's interleaving is not recorded | the same switch points can be recorded and replayed |
| The shadow test's doubled scan | measured per thread | measured on one timeline |
| Cores used by the logic | several | one; the others serve the bus and the embedded portal |
| Copies of the image between tasks | across threads, under a lock with priority inheritance or a lock-free handoff | on one thread, no lock |

The architect's rule "existing, tested Linux methods before our own" (2026-11-16) speaks for (a). The ruling that a
hit halts every task, and record and replay of a field incident on the simulator, speak for (b). Either way the
simulator schedules every task on one thread on virtual time, since a simulation must replay alike.

**The assistant's pick, ruled on 2026-11-25: (b) for the first release**, its one logic thread itself under the
kernel's real-time scheduling, with (a) reachable later without changing a program.

### 6. The fieldbus has its own thread, above the logic
- The bus cycle runs on its own real-time thread, at a priority above the logic, and keeps the latest input and
  output values of every device.
- A breakpoint, a halt or a fault never stops it: at a halt it sends each output's halt value (safe, or held within
  the target's longest hold time); in FAULT it sends the safe values; a device lost or returned becomes an event
  queued for its handler task.
- It is also the watchdog of the logic: if a scan passes twice its period without ending (an interpreter that
  cannot reach an instruction boundary, a bug), the bus thread sends the safe values itself and the controller
  faults with 501 `watchdog(task)`. Below it, the hardware watchdog answers a frozen operating system
  (`docs/controlador/linux-study.md`).

### 7. Time
- **The program clock** gives every scan its time (`Scan::now`): the controller's running time, paused while the
  program is halted. Periods and deadlines are on this clock, so after a halt every due time moves by the halt's
  length, and whatever came due during the halt is dropped (2026-11-15).
- **The date** is a second clock, read by time-of-day blocks only, which wait until it is confirmed (2026-11-16). A
  time-of-day start that fell during a halt or before the confirmation does not run (K9, read with the ruling of
  2026-11-15).
- **The main cycle's limit** is the maximum cycle time the user sets, counted from the start of the cycle to its
  end, pre-emptions included; cyclic interrupts are held to their period. The time-error rule then applies to both
  alike: the handler on the first overrun, a fault when one scan exceeds twice its limit (Q39's reading).

### 8. Start-up, handlers and stop
- Start-up blocks run in order in STARTUP (phase 3, `handler`), before the first main cycle; no cyclic interrupt runs
  during start-up.
- A handler is a task released by its event, its input slice the event's data and its outputs its own.
- A program error in a block ends that block's scan at the failed check; the block's slice keeps its last published
  outputs; the program-error handler runs if the program has one, else the controller faults with 502
  `program(at)`.
- A stop requested runs the stop handler for one last scan, then the controller goes to STOP.

### 9. The shadow test (K11)
Put off by the first release's limit of one cyclic task. With one cyclic task the check is the review's: both
versions' measured worst-case scans together fit the period.

## What code stands on so far
The engine of a scan (`orcl-runtime`) runs one cyclic task: inputs latched at the start of its scan, outputs
published at its end, absolute deadlines on the program clock, the time-error rule, safe outputs in FAULT. Several
tasks, pre-emption on one logic thread, handlers and the bus thread are ruled and not built yet.
