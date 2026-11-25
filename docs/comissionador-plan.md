# veredas-comissionador: the plan

The testing platform of Veredas 2; screens say **Comissionador**. `docs/veredas-plan.md`
("veredas-comissionador: the testing platform", decision 6) wins on intent; `.zen/contract.md`
(P-006, then the `C-2xx` clauses it becomes) wins on status. This file says what comissionador
holds, how a test runs against a plant, what the engine offers Oficina, in which order it lands,
and the traps met on the way. The test file and the function file are `docs/test-schema.md`; the
look is `docs/design/comissionador/`. Phases 1 to 5 are built: the reading and the check (C-200,
C-201), the engine, live and replayed (C-202, C-203), the run record and the report (C-204,
`docs/report-schema.md`), the executable (C-205); phase 6, for Oficina's designer, waits for it.

## The role nobody covers
**Comissionador runs a plant's acceptance tests: a test, written as one plain-text file, drives
the machine through portal's gated writes, waits for and measures what the plant does through
portal's data, live or replayed over a stored window, branches on what it measured, and ends
every run in an acceptance report kept with the evidence it rests on.**

No other component answers this: portal reads, stores and serves the plant and gates its writes,
but judges nothing; supervisão shows a plant to a person and offers its writes one at a time;
the bench plays a plant and judges nothing either; controlador runs a machine's program, where a
test checks the machine from outside; ábaco computes on workbooks. Veredas 1 never had it: its
`comissionador` was a name and a list of questions (`docs/comissionador-plan.md` there, the
seed of 2026-10-24), and the acceptance tests of a plant were run by hand, watching a dashboard.

## Decisions it builds on
- **Decision 6** (the plan, 2026-11-05): one test file, two views (text and flow chart), script
  boxes, functions written in text and reused as blocks, branching on any outcome, with retries;
  an acceptance report for every run; tests in plain text, diff-able in git.
- **Through portal** (the plan, "Adding a component", rule 3; law L5's map: comissionador uses
  portal): it reads plant data only through portal and drives machinery only through portal's
  gated writes. It holds no socket, no OPC UA client and no catalog of its own.
- **The design** (`docs/design/comissionador/`): the block kinds and their colours, the flow
  chart's top-to-bottom reading with branch exits to the right, functions as blocks with input
  ports left and output ports right, a `reads` line and no writes in a function, the layout
  written by the flow view as the file's last block, gated writes counted before a run, runs
  kept under `runs/<test>/<n>` with their step results, the portal data they used and their
  report, a failed live run replayable from its own window, a run that times out on a wait
  shown as aborted (the run history's "replay · aborted · wait_pe timeout").
- **The designer is Oficina's** (`docs/oficina-plan.md`, "Surfaces that arrive with their
  components"; the briefing's A6.2): the flow-chart designer, the text editor on the same file
  and the run-and-report screen (PDF, sign-off) are delivered together in Oficina. Comissionador
  is the engine they stand on: a library and one executable, no interface of its own, like
  portal and the bench (Q12, Q14).
- **Local, no accounts** (Q20): whoever runs a test types the name the report carries.
- **Sessions in process** (Q19, supervisão's ruling, followed here meanwhile): comissionador's
  executable starts the plant's portal session from its project file in its own process, as
  supervisão does (Q313 asks whether it should reach a portal over HTTP instead, or as well).

## Scope
- **The test language** (`lex`, `parse`, `model`; `docs/test-schema.md`): the test file
  (`veredas-test`) and the function file (`veredas-functions`), stamped in TOML and read by
  core's header reader, every node of the syntax tree carrying its place in the text.
- **The check** (`check`): a test and the function files it uses checked whole before a run,
  every problem named with its file, line and column; the counts the designer shows (steps,
  branches, functions, gated writes); the flow graph the flow view draws.
- **The engine** (`plant`, `eval`, `run`): a checked test bound to a portal session and run,
  step by step, on a clock that is the store's axis, live against the plant or replayed over a
  stored window; writes through portal's gated handles alone; branches, `on` routes and retries;
  functions and script boxes; a stream of events while it runs; a stop by the tester.
- **The run record and the acceptance report** (`record`): every run kept under
  `runs/<id>/<nnnn>/` with the exact text that ran, its steps, its criteria, the values it
  reported, its writes, the rows it read and its verdict; a tester's note and signatures; read
  back for the run history and for a replay of the run's own window. Its form is Q312's.
- **The executable** (`main.rs`): `veredas-comissionador check <test>`, `run <test> --project
  <project.toml>` and `replay <test> --project <project.toml> --from <t> --to <t>`, the verdict
  in the exit code, for a commissioning laptop and for a pipeline with no Oficina.
- **For Oficina, later**: the edits the flow view makes, written back into the text with
  everything else kept byte for byte (below, "What the engine offers Oficina").
- **Later, named, no code now**: a reach to a portal over HTTP (Q313); a replay from a run's kept
  evidence once its session is gone (it needs a stored window portal can open read only); a
  `sign` setting for safety tests that need a witness (Q312).

## Rules
- **Through portal, both ways.** Every value a test reads is a row of portal's store, read through
  portal's library; every write is portal's OPC UA handle or channel handle, behind portal's
  master switch and whitelist (C-043, C-044). Comissionador never sets the switch: its sources
  name no call that arms it, and a test holds that by reading them (Q314).
- **The test file is the program.** What a run does is what the file says; the flow chart is a
  view of it, and positions are the one thing the view adds (the `layout` block). Nothing about a
  test lives elsewhere: no hidden settings, no state between runs but the runs folder.
- **A function only reads; a script box never touches the plant.** A block that is a function
  can be placed anywhere without a review of what it might write; a script box computes the same
  in a live run and in its replay.
- **Live and replay are one engine.** The same interpreter judges every step on the rows' own
  stamps; the modes differ in where the rows come from (arriving, or stored) and in what a write
  does (sent, or noted). A replay of a live run's window reads the rows the live run read.
- **Every run ends in a report**, whatever its verdict, aborted ones included; a run that could
  not start (refused at its check or at binding) is no run and leaves nothing.
- **Fixture first: the bench is the plant.** Every clause of the engine is proven against a
  portal session fed by maquinar (`Bench::start`: its OPC UA server with reactions, its
  controller reference over a real socket), never a mock of portal; comissionador's tests
  dev-depend on the bench (law L5 ignores dev tables).
- **Content-agnostic**, like the rest of the suite (Q8): no device, vendor or product in code,
  comments, tests or fixtures; the fixtures are the schemas' worked examples.
- **Typed errors on the public surface**; no `unwrap`/`expect` on an I/O path; tests may unwrap.
- **A library first, one executable on top**, depending on core and portal alone (`MAP`:
  `veredas-comissionador → [veredas-portal]`).
- **One test file per capability** under `tests/`, so a clause links one file and a change to one
  capability re-stales only its own recorded run.

## What it absorbs from Veredas 1
Nothing of code: Veredas 1 has no comissionador crate. Its seed plan (2026-10-24) asked five
questions, and this plan answers them as follows:

| The seed's question | Here |
|---|---|
| The plant I/O boundary: a consumer of veredas's gated surfaces, or a direct connector? | Through portal, reads and writes (the plan's rule; decision 6) |
| The authoring surface: egui, web, or its own app? | Oficina's designer (A6.2); the engine has none |
| What a procedure is on disk | One plain-text test file (`docs/test-schema.md`, Q310) |
| Execution semantics: steps, assertions, timeouts, retries; manual entries | The test language; a step for a person's entry is not designed yet (below, "Open") |
| The report: format, contents, where it lands | `runs/<id>/<nnnn>/`, its form Q312 |

## How a run meets the plant

**The door.** The engine reaches portal through one interface, `Plant`, so a second way to reach
it (Q313) is a second implementation, not a second engine. It asks four things: the store's
catalog (every table with its columns and types); the rows of a table in a window of the store's
axis (`DuckStore::query_rows`, newest first, which the engine pages and orders); a write of one
OPC UA tag (the session's `OpcHandle::write_tags`, the tag found by its table and column in the
session's compiled spec, the value coerced to the tag's type by `serve::coerce`); a send of one
catalog message (`ChannelHandle::send`). It reads the switch's position; it never sets it. The
in-process implementation holds a session's `Live`, all of it public today: nothing is needed
from portal for the phases below.

**The clock.** A run keeps a moment on the store's axis (`docs/test-schema.md`, "Time"). A wait
gathers the rows of every table its condition reads, from the clock on, in stamp order, applies
each to the values it knows and evaluates the condition at that row's stamp; the first stamp at
which it holds is the step's moment, and the clock moves there. Live, the engine polls the store
every 50 ms and treats a moment as read once it is 250 ms past (the session's pump flushes every
50 ms), and a write moves the clock to the plant's answer. Replayed, the rows are all there, and
the stored window's end is the run's horizon. The same rows give the same verdict both ways.

**Bound before it runs.** Before its first step a run checks the test against the session: the
session's name against `portal`, the mode against `modes`, every table and column the test and
its functions read (a column read through a name no table it may hold a row of has among them, an `await`'s and through every copy), every `write` target an OPC UA tag marked writable, every `send` a channel
and a message of the session, and, live, the switch armed when the test writes. A refusal there
starts nothing and writes no run.

**What a write is.** Live, a `write` or a `send` goes to the plant through portal's handle and the
step's detail keeps portal's answer word for word; a refusal (the switch turned off mid-run, a
server away, a write that may or may not have reached the plant) ends the step in `error`, which
aborts the run unless the test routes it. Replayed, nothing is sent; the step passes with the
note "not sent: replay" (Q315).

## What the engine offers Oficina
Oficina's designer and run-and-report screen (A6.2) stand on this, all of it headless and tested
here, so Oficina's tests drive its interface and not the engine's logic:
- **Parse with places**: the syntax tree of a test or a function file, every step, action, arm,
  route and position with its byte range, line and column; problems never a panic, each with its
  place, so the text view underlines them.
- **The check and its counts**: problems; steps, branches, functions and gated writes (the
  inspector's "3 gated writes in this test"); a function's signature, the block's ports.
- **The flow graph**: a node per step with its block kind (the kind of its first action, `stop
  fail` drawn as a fail end) and its actions as text; edges for the fall-through, each `on` route
  (labelled with its outcomes), each branch arm (labelled with its pattern), each retry (with its
  `max`), each `stop`, and the start and end nodes; the positions from `layout`.
- **Edits as text**: a step's position set (the `layout` block written, every other byte kept),
  then, as the flow view needs them, a step added, removed, renamed (its routes renamed with it),
  an edge moved; each returns the new text, so the text view and git see what the flow view did.
- **Binding against a session**: the same checks a run makes before its first step, without a
  run, so the inspector can say a target is not writable or the switch is off.
- **A run with its events**: every step's start and end (outcome, detail, attempt), every
  criterion as it is judged, every value reported, every write and its answer, the verdict; the
  tester's stop.
- **The runs of a test**: listed newest first with mode, verdict and duration; one read back
  whole; a note added; a report signed, and frozen once signed.

## Module map
```
veredas_comissionador
  lex       the tokens of a test or a function file, each with its place; the header cut off
  parse     the syntax tree, every node with its place; the stamp read by core's header reader
  model     a test and its functions as the engine runs them: steps, actions, routes, expressions
  check     the whole check: names, routes, retries, calls, reads; the counts; the flow graph
  eval      values and expressions: the types, the comparisons across them, interpolation
  plant     the door to portal: the catalog, rows in a window, the gated write and send (Live)
  run       the interpreter: the clock, steps, outcomes, branches, retries, functions, events
  record    the runs folder: the record and the report written and read back, note, signature
  edit      the flow view's edits as text (later, as Oficina's designer needs them)
  main.rs   veredas-comissionador check | run | replay
```

## Phases
1. **The grammar and the check.** The crate at `2.0.0-alpha.1`, listed in the workspace;
   `lex`, `parse`, `model`, `check`; the reference's worked examples loaded from
   `docs/test-schema.md` and every statement of it held against the reader; every refusal by its
   text and place. Clauses: C-200, the component (P-006 promoted: a library and one executable,
   core and portal alone, its plan before its code, the switch never armed by its sources);
   C-201, the test file and the function file read and checked.
2. **The engine, live.** `eval`, `plant`, `run` against a portal session fed by the bench: the
   worked example against the controller reference's machine, and tests of every action, route,
   outcome and verdict against the bench's OPC UA server and its reactions (a gated write
   refused while the switch is off, then landing armed; a wait on a reaction; a branch and a
   retry; a function's timeout; a script's fault). Clause C-202.
3. **Replay.** The same tests replayed over a stored window of the session; a live run's own
   window replayed to the same verdict and values; the window's end aborting a run. Clause C-203.
4. **The run record and the report.** `record`: written under `runs/<id>/<nnnn>/`, read back,
   listed; the note; the signature, frozen after (as Q312 is answered). Clause C-204, its
   reference `docs/report-schema.md`.
5. **The executable.** `check`, `run`, `replay` from the command line, the verdict in the exit
   code. Clause C-205.
6. **For the designer.** The flow graph and the edits as text, when Oficina's designer starts
   (P-015); clauses then.

Each phase ends with `cargo nextest run --workspace -E 'rdeps(veredas-comissionador) |
package(leis)'` green, its clauses recorded, refuted and verified, and a push.

## Questions
Asked in `docs/questions.md`, each with the option taken meanwhile:
- **Q310** the test file's syntax: the design's language, refined, with the suite's stamp
  (taken), or TOML throughout, or a subset of Rust.
- **Q311** the script box's language: the test language's own statements (taken), an embedded
  scripting language, or Python.
- **Q312** what an acceptance report holds, its form, and who signs it: a TOML record per run with
  the evidence beside it and the PDF drawn by Oficina (taken).
- **Q313** how the engine reaches portal: in process (taken first), over portal's HTTP interface,
  or both behind one interface.
- **Q314** who arms portal's write switch for a run: never comissionador (taken).
- **Q315** what a write does in a replay: not sent, noted (taken).
- **Q316** how retries count and how they show: per route, per run, never reset (taken).
- **Q317** how a test names plant data: portal's own `table.column` (taken), aliases, or the
  plant's symbolic names.

Open and not asked yet, because nothing waits on them: a step in which a person enters a
measurement or confirms what they saw (the seed's "minimal manual work"); how many ports a
function block shows before it collapses (the design's open point, Oficina's to draw).

## Traps
Recorded as the build meets them.

- **A replay cannot see a session start's read-in end** (C-205, Q319). Portal shows a log's line
  count and last stamp only. A log that has read a line stamped at or after the session's start
  has read its history (files read whole, the newest last, lines stored before health is shown);
  a stamp past the window is not enough, a rotated member's late line being one. A log with no
  stamps holds nothing of a past window. For the rest, lines holding still across two refreshes
  are a guess that a family's empty members beat: portal waits for each member to hold still
  before the next.

- **A word the language reads by its place is no reserved word** (C-201). `max` was reserved for
  `retry <step> max <n>` and is also a built-in call, so `max(a, b)` could not be written. The
  route reads `max` by its place; the reserved list is the reader's and the reference's at once,
  held equal by the grammar test.
- **A read waits for its moment to be stored** (C-202). The session's pump flushes every 50 ms,
  so a live run reads a moment only once it is 250 ms past; read sooner, the rows stamped just
  before it are not there yet, and a live run and its replay would disagree.
- **A fixture must reply to the destination the test follows** (C-202). The worked example's
  function counts outcomes for destination 4; a bench whose replies name another destination makes
  it wait out its 60 s bound, which is right and reads like a hang.
- **A column that is always empty is read back to the table's first row** (C-202, named, not
  built): the latest value passes holes, so a rule's optional group that never matches makes every
  read of it a scan of the table.
- **A step is not named after an action.** The design names a step `report`; the reader refuses
  the language's words as names, so the reference's step is `summary`. A name that is an action
  would make a step's first line read two ways.
- **A bench channel says where it listens only until a peer has come and gone** (C-205). Its wire
  state reads `listening on <address>` while it waits for its first peer and `<n> peer(s)` after,
  so every project text that needs the machine's port is written before the first run dials it.
- **A hand-written TOML scan keeps losing to TOML** (C-201, rounds 1, 3, 4 and 5): an escape, a
  quote run, a character past ASCII each hid a header key. The header is read by the `toml`
  crate's own document reader (`toml::de::DeTable`), its keys with their spans, its values left
  as text.
- **A replay's write moves no clock** (C-203, Q318). Live, a write moves the clock to the plant's
  answer, at least the read settle after a step that read the plant; a replay sends nothing and
  stays. Plant data read right after a write, before that lag has passed, is the plant before the
  write in a replay: a tight wait times out, an await takes an earlier row, a read the old value.
  Until portal keeps each write's moment, a test meant to replay reads nothing right after a
  write.
- **The run's bound is the run's clock's** (C-202, C-203). Judged on the machine's clock live, a
  read's settle put a run past its bound that its own window, replayed, never reached; on the
  run's clock in both modes a script box meets it never, its statement budget and the tester's
  stop ending a runaway loop.
- **A view's rows carry no source or sequence** (C-202). An await told the last row it took by
  its source and sequence, so over a view it retook a row of a shared stamp; a stamp's rows now
  stand in one order whatever the store's, and an await counts what it passed.
- **The store's latest breaks a tie its own way** (C-202). Two rows of a table at one stamp (two
  keys, two sources): a wait applying them in sequence disagreed with a read; a tied stamp is
  read back through `latest` before the wait judges it.
- **Rows share stamps** (C-202). A log stamped to the second, or two messages in one millisecond,
  give rows of one stamp: a wait judges a stamp once all its rows are applied, and an await at the
  stamp of the row the last one took takes the next row there, or a counting loop misses events.

