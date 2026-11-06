# veredas-maquinar: the plan

The bench of Veredas 2. `docs/veredas-plan.md` ("veredas-maquinar: the bench") wins on intent;
`.zen/contract.md` (P-004, then the `C-xxx` clauses it became) wins on status. This file says what
the bench plays, what it took from Veredas 1 and what it left there, in which order it lands, and
the traps met on the way.

## The role nobody covers
**Maquinar plays the plant's hardware on the real wire, by configuration, so every consumer meets
its plant on a bench before it meets the plant.**

No other component answers this: core moves and reads bytes but plays nobody; portal is the
gateway that *talks to* the plant; aranha scans a network; comissionador checks what portal
reports. Without the bench each consumer would be tested against the field, or against a mock of
its own that drifts from the wire. The bench is the fixture every other member is built against
(the plan, "Fixture first"), so it comes before the consumer that needs it, and it is test-only:
it never runs in production, owns its sockets, and carries no store and no desktop.

## Scope
- **Log emitters** (`emit`): lines rendered from a template (`{ts}` and `{name}` values drawn
  within ranges), written the way a plant logger writes them (CRLF, flushed per line, UTF-8 or
  UTF-16LE with its BOM) into files named by a date pattern and a rotation index, rotating on a
  line count and on the day; a restart never appends to a file it already wrote; the family
  shape (regex and date format) that reads the files back, for portal's log families.
- **A configurable OPC UA server** (`opc`): a tag list (type, access, initial value, a draw
  within a range on an interval) served on a real endpoint, anonymous or by user and password;
  values set and read on the server side (write-only tags included); **reactions**, a write of a
  given value into a tag answered by a timed sequence of values, so a consumer's pulse and
  handshake have a plant to land on; a relay (core's `opc::demo::Relay`) for a link that goes
  silent; the stack's limits (nodes per read, sessions) as knobs.
- **Telegram channels** (`channel`): a channel in either role, framed and held by core's channel
  file (`docs/channel-schema.md`), speaking a **catalog** of messages (core's catalog file: a
  name, a segment id, the body's fields) built from named fields and read back into them;
  cyclic and on-demand messages; a monitor of what arrived, by name and field.
- **A simulated controller** (`controller`): the controller's behaviour as a versioned rules file
  of its own: **items** it announces at a rate and awaits an answer for (an announcement, a
  reply keyed by a field, an outcome after a travel time, drawn from a weighted mix or forced
  by the destination's state), **answers** it gives to incoming messages (fields copied,
  constant, drawn, or decided by rules: first-match patterns, a weighted spread, a reject
  value, sticky per key), **registers** cyclic messages read and **commands** set, and
  **destinations** with named state bits that block. The same file plays either side of a
  conversation: the machine, or the stand-in for the unit that answers it.
- **Controllable pressure**: the announcement rate, named faults as a percentage of items
  (a no-read: the fields a fault blanks or sets), destination states (a full chute), every one
  live on the handle while the bench runs.
- **The bench file** (`bench`): one versioned TOML file for the whole plant (emitters, the OPC
  server, the channels and the files each points at), started as a library (`Bench::start`,
  for portal's tests and later comissionador) or from the command line (`veredas-maquinar
  <bench.toml>`, until stopped).
- **Later phases**, named, no code now: the PROFINET device emulation (Veredas 1's `pnsim`,
  which speaks through aranha's codecs and waits for P-005; `docs/questions.md` Q5); the 2D/3D
  plant simulator; a PKI folder, when the OPC server first needs a secured endpoint; a desktop
  UI (`docs/questions.md` Q12).

## Rules
- **Fixtures first.** A consumer's hardware is simulated here before or alongside the consumer;
  portal (P-002) is tested against this bench, never against a mock of its own.
- **A test tool that owns its sockets** (the plan, "Architecture"): it binds, listens and dials
  itself, past portal, and never runs in production.
- **Speaks the real device's wire**, so a consumer cannot tell the difference: a real OPC UA
  server on a real endpoint, real TCP by core's channel, files on a real disk. A mock is never
  the evidence of a clause.
- **Content-agnostic**, like core (`docs/questions.md` Q8): no device, vendor, message catalog
  or product is named in its code, comments, tests or fixtures. What a controller's telegrams
  mean is configuration: a channel file, a catalog, a rules file. The fixtures use the channel
  schema's worked example, a segmented envelope, with message names that say what a message
  does (an announcement, an answer, an outcome), never whose it is. The bench's own test reads
  every file of the crate for the vendor terms core bans.
- **Generalisation first** (the plan, "Adding a component", 2). What portal would also need is
  designed for core and lands there with its own clauses: the compose engine (a body built from
  named fields, the inverse of `telegram::capture`) and the catalog file. Core still depends on
  nothing (L4); the bench depends on core, and on nothing else of the suite until aranha exists.
- **Configured, not programmed.** Every plain-text file is TOML stating `format` and `version`,
  a misspelled key refused, an unknown version refused (`docs/questions.md` Q10; P-010): the
  bench file (`veredas-bench`), the controller rules (`veredas-controller`), and core's channel
  and catalog files. Each format has its schema document under `docs/`, and its test loads the
  document's own example.
- **A library first, one executable on top.** Everything runs in-process behind handles, so a
  consumer's tests embed a bench; `src/main.rs` runs a bench file from the command line. No
  other executable, no examples: cargo, asked what it would build, names one library, one
  binary and tests.
- **Test-only and light**: no DuckDB, never portal's store or desktop, no UI crate (Q12).
- **Every worker owns a thread and stops promptly**; a handle's drop stops its worker; a
  status is read without blocking; every knob of pressure is live on the handle.
- **Typed errors on the public surface**; no `unwrap`/`expect` on an I/O path; tests may unwrap.
- **One test file per capability** under `tests/`, so each clause links one file and a change
  to one capability re-stales only its own recorded run.

## What it absorbs from Veredas 1
Read in full before the port: all of `crates/veredas-maquinar` (`config.rs`, `logsim.rs`,
`opcsim.rs`, `tcpsim.rs`, `mscsim.rs`, `pnsim.rs`, `app.rs`, the four `ui_*.rs`, `tests/`,
`examples/fake_eu.rs`, `pki/`) and the automated replies of `crates/veredas-trajetorias`
(`plan.rs`, `service.rs`; its `store.rs` and DuckDB are not carried, decision 2). What moves where:

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| `logsim.rs` (templates, file naming, the rotor, the emitter worker, `family_spec_for`) | `emit` | the log emitters, as they were, with typed errors |
| `opcsim.rs` (the tag list served, draws, server-side set and snapshot) | `opc` | the configurable server; its vendor command handshake (a trigger written 100, read 99 then 1) becomes a configured **reaction** |
| `tcpsim.rs` (a channel in either role, cyclic and on-demand telegrams, the monitor) | `channel` | the framing and link rules are core's channel file; an ASCII or hex body becomes a catalog message composed from fields |
| `mscsim.rs` (the parcel factory: announce, destination, outcome, chute states, commands, status) | `controller` | generalised into the rules file: items, answers, outcomes, destinations with named bits, registers, commands, cyclic messages; no message, field or state is named in code |
| trajetorias `plan.rs`, `service.rs` (routing rules first match, weighted spread, reject, sticky) | `controller` answers (`decide`) | the automated replies, with no HTTP, no CSV and no DuckDB: rules in the controller file |
| `config.rs` (the JSON project, preferences, recents) | `bench` (`veredas-bench`, TOML) | Q10; no preferences: a bench file is handed to the executable |
| `examples/fake_eu.rs` | a channel with an answer rule | the stand-in unit is the controller file played from the other side |
| `tests/logsim.rs`, `opcsim.rs`, `tcpsim.rs`, `mscsim_e2e.rs` | `tests/*.rs`, one per clause | real directories, a real client, real sockets |
| `tests/family_contract.rs` | portal's, when its log families land | the scanner it pins is portal's (P-002) |
| `pnsim.rs`, `tests/pnsim.rs` | a later phase | waits for aranha's codecs (P-005, Q5) |
| `app.rs`, `ui_*.rs` (an egui desktop) | not carried now | Q12: the bench is a library and a command-line executable until the operator decides |
| `pki/` | a later phase | the server serves an unsecured endpoint, as Veredas 1's did |

Changed on the way, each recorded in the clause's evidence note:
- edition 2024; typed errors (`EmitError`, `OpcError`, `ChannelError`, `RulesError`, `BenchError`)
  where Veredas 1 returned `String`;
- the telegram channel's envelope node ids, byte order and keep-alive are the channel file's,
  not fields of the bench's own;
- the message id is a catalog entry, not a number typed per telegram, and a body is fields;
- the sorter's catalog, state-word bits, commands and outcome codes left the code: they are the
  rules file's, and the worked fixtures use neutral names and small numbers;
- the no-read is a named fault with a percentage, one of any number the file may declare;
- the GUI-only pieces (previews, the "restart to apply" snapshot comparison, recents) are gone;
  a config applies at start, and a running part is stopped and started to apply a change.

## Module map
```
veredas_maquinar
  emit        template rendering, file naming and rotation, the emitter worker, the family shape
  opc         the configurable OPC UA server, its reactions, the relay (core's demo)
  channel     a bench channel: channel file + catalog, cyclic and on-demand messages, the monitor
  controller  the rules file (schema), the engine (items, answers, registers, commands, cyclic,
              destinations), the pressure knobs
  bench       the bench file (schema), Bench::start, every part's status
  main.rs     veredas-maquinar <bench.toml> [--check]
veredas_core (added for the bench, kept for portal)
  telegram::compose   a body built from named fields: binary fields at offsets, a text template
  telegram::catalog   the catalog file: messages by name, their segment id and body rules
```

## Phases
1. **The crate and the emitters.** `crates/veredas-maquinar` at `2.0.0-alpha.1`, listed; the
   component clause (P-004 promoted: a library and one executable, content-agnostic, its plan
   before its code); `emit` with Veredas 1's tests against a real directory.
2. **The OPC UA server**, driven by the same client library portal will use: reads, draws,
   writes landing server-side, write-only tags, reactions, a port in use, stop freeing the port.
3. **Core: compose and the catalog.** `telegram::compose` (the inverse of capture, round-tripped
   through it) and `telegram::catalog` (`docs/catalog-schema.md`), each its own clause and test
   file in core; core's plan and module map updated.
4. **Channels and the controller.** `channel` over real loopback sockets in both roles; the
   controller rules file (`docs/controller-schema.md`); the engine, pure and tick-driven, pinned
   without sleeping, then the whole conversation over a real socket; pressure live.
5. **The bench file and the executable.** `docs/bench-schema.md`; `Bench::start`; `main.rs`
   run as a process with a bench file; Q12 asked.

Each phase ends with `cargo nextest run --workspace` green bare, `cargo test -p leis` green, and
its clauses recorded, refuted and verified. Push at the end of each phase.

## Traps
Recorded as the port meets them.

- **Check the disk before the first build.** Core's OPC phase filled C: twice (`docs/core-plan.md`,
  "Traps"); each OPC test binary carries a PDB of about 200 MB, and this crate adds a server
  and a client per test file. 52 GB were free when this plan was written, the target at 6 GB.
- **The OPC namespace must not be the application URI** (inherited from Veredas 1's `opcsim`
  and core's demo): the diagnostics node manager registers first and takes a colliding index,
  and every read then answers `BadNodeIdUnknown` with the nodes plainly present.
- **Bind first, advertise the bound port**: with port 0 the server's advertised endpoint must
  carry the port the OS gave, or a client's endpoint match finds nothing.
