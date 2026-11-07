# veredas-portal: the plan

The gateway of Veredas 2. `docs/veredas-plan.md` ("veredas-portal: the gateway") wins on intent;
`.zen/contract.md` (P-002 and P-010, then the `C-xxx` clauses they became) wins on status. This
file says what portal holds, what it took from Veredas 1 and what it left there, in which order
it lands, and the traps met on the way.

## The role nobody covers
**Portal is the one point where the suite exchanges data with the outside world: it reads the
plant's logs, telegrams and OPC UA servers by configuration, stores every protocol's data alike in
one session store, and serves it, and its gated writes, to every other component.**

No other component answers this: core moves and reads bytes but holds no connection and stores
nothing; maquinar plays the plant and never runs in production; aranha scans a network for setup;
the apps (comissionador, supervisão, ábaco) read plant data and drive machinery only through
portal; controlador's real-time path is the one production exception, and everything else it does
rides an embedded portal. Without portal each app would grow a socket to the plant of its own,
and Veredas 1 shows where that ends: its first app's data plane was called `veredas-core`, and
its sort brain, its supervision app and its desktop each spoke to the plant on their own terms.

## Scope
- **The data plane for logs** (`tail`, `family`, `capture`, `encoding`): one growing file or a
  rotating file family tailed `tail -F` style, zero lines lost or duplicated across rotation
  (drain-then-handoff, catch-up through generations born inside one rescan), a start point in the
  past replayed from the located line and handed off live at the exact byte (backfill), text
  decoded from UTF-8, UTF-16 and CP1252 with lazy detection, lines stamped by a per-source
  timestamp rule and a dense sequence, and a line read into typed series by a regex capture rule
  (named groups, typed per group, an identity key). Reading never blocks the plant's writer.
- **Telegram channels configured by rules** (`channel`): a portal channel is core's channel file
  (`docs/channel-schema.md`) and a catalog (`docs/catalog-schema.md`) in a project folder. It
  asks `Catalog::check_framing` before it binds, places and reads ids by `frame`, `messages_of`
  and `Message::reads`, captures each received message's fields by the catalog's rules into a
  row of a table named for the message, and composes replies by `compose` from named fields,
  when a reply rule says so. No vendor is named: a device spoken over TCP is such a pair of files
  (`docs/questions.md` Q8).
- **OPC UA sources** (`opc`): core's `opc::worker` polled on an interval, each rule a table of
  tags and bit fields, `store_on_change` kept, the write whitelist core's.
- **The session store** (`store`): the DuckDB session store from Veredas 1, every protocol's data
  stored alike: one wide base table per rule (a capture rule, an OPC rule, a telegram message),
  the narrow `samples` table, user-defined SQL views in one namespace, windowed queries,
  snapshots and CSV export, in RAM or on disk.
- **The server** (`serve`): SSE (`/events`: a `hello` snapshot, then `data` frames, `lag` when a
  client falls behind), windowed REST with LTTB downsampling (`/api/session`, `/api/series`,
  `/api/series/data`, `/api/tables`, `/api/table/data`), and the gated write surface
  (`/api/opc/read`, `/api/opc/write`, `/api/commands`, `/api/commands/run`, and the telegram
  reply as the same gate's second path): whitelisted tags, the master switch off by default, a
  write refused by default and proven refused on the wire.
- **The session** (`session`): a project's sources started as workers, their events pumped into
  the store and the feed, per-source health on the arrival clock (live, waiting, idle, retrying,
  ended, failed), the series listing kept by ingest and never by a store scan.
- **The project file** (`project`, P-010): one TOML project per plant stating `format` and
  `version`, portal migrating an older version or refusing an unknown one; what Veredas 1's JSON
  project carried is read into TOML, never carried as JSON; one header reader in core for every
  versioned file of the suite, and the test that holds every file to the stamp.
- **The desktop shell and the embeddable build**: what Veredas 1's `veredas-desktop` becomes is
  the operator's call (`docs/questions.md` Q14): a port of the egui desktop, a thinner shell over
  the server, or the web interface of supervisão. Until answered, portal is a library and one
  executable, `veredas-portal <project.toml>`, serving a project from the command line; a build
  without the default features leaves the store and the server out, for controlador's embedding.
- **Later, named, no code now:** the grep engine (Veredas 1's grep tab over logs at rest), a
  shell's concern once Q14 is answered; the clock-skew monitor as a derived source; the
  library of templates and recents; bundles.

## Rules
- **Every component exchanges data through portal**, except the test tools and controlador's
  real-time path (the plan, "Architecture"). Portal itself reaches the plant only through core's
  layers: it holds no codec, no catalog and no device of its own.
- **Writes stay gated**: the master switch off by default, a whitelist of writable tags, both
  enforced in core's handle below the server, so an in-process caller is gated like a browser;
  every write path (the OPC endpoints, the commands, a telegram reply) obeys the same switch.
- **A device spoken over TCP is a channel file, a catalog and portal's own rules file**, plain
  text, versioned, in a project folder, as an OPC rule maps tags and a log rule captures fields.
  Its data is stored like any other protocol's.
- **Content-agnostic**, like core and the bench (Q8): no device, vendor, message catalog or
  product named in code, comments, tests or fixtures. The fixtures use the schemas' worked
  examples, run by the bench.
- **Fixture first: the bench is the plant.** Every clause is proven against maquinar, started
  in-process through `Bench::start` from a bench file in the test's fixtures or as the
  executable, over real files, real sockets and a real OPC UA endpoint, and against a real
  DuckDB file; a mock is never the evidence of a clause. Portal's tests dev-depend on the bench
  (law L5 ignores dev tables); portal itself depends on core alone (`MAP`: `portal → []`).
- **Every plain-text file is TOML stating `format` and `version`**, a misspelled key refused,
  an unknown version refused, an older version migrated (P-010): the project file is the first
  file migrated; the channel file, the catalog, the controller file and the bench file are read
  by core's one header reader.
- **Reading a plant file never blocks its owner from writing**: every handle on a log file is
  `File::open` (share-all on Windows), never a stricter share mode and never a memory map.
- **Two clocks, not one**: a sample indexes on the stamp parsed from the plant (the store's axis)
  with its arrival on the unix clock beside it; health is judged on arrival.
- **A library first, one executable on top**; the light build is the same crate without its
  default features (`store`, `serve`), never a second crate; cargo,
  asked what it would build, names one library, at most one binary named `veredas-portal`, and
  tests.
- **Typed errors on the public surface**; no `unwrap`/`expect` on an I/O path; tests may unwrap.
- **One test file per capability** under `tests/`, so each clause links one file and a change to
  one capability re-stales only its own recorded run.

## What it absorbs from Veredas 1
Read in full before the port: Veredas 1's `veredas-core` (the data plane), `veredas-store`,
`veredas-server`, `veredas-desktop` (its structure and its project format,
`docs/project-format.md` there), with Veredas 1's plan (phases 1, 3, 4, 6, 10, 11, 12) and
`CLAUDE.md`. What moves where:

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| core `tail/{single,family,cursor,backfill}`, `family`, `encoding`, `pathkind`, `timestamp`, `timeparse`, `ring` | `veredas_portal::tail`, `family`, `encoding` | the data plane, as it was, with typed errors |
| core `source` (`Source`, `SourceSet`, `Emitter`, `WorkerReport`), `LineEvent` | `veredas_portal::source` | the line-producing side of one worker model; the handle itself is core's `SourceHandle` |
| core `extract` (`ExtractRule`, `GroupType`) | `veredas_portal::capture` | a line into typed series by rules; `GroupType` is core's `SeriesType` |
| core `capture` (the grep tab's ad-hoc tables, CSV and Markdown), `grep`, `highlight` | later, with the shell (Q14) | operator tooling over logs at rest, not the gateway |
| core `store` (`SeriesStore`, `Point`, `SeriesInfo`), `DERIVED_SOURCE` | `veredas_portal::store` | the store's own surface, beside its one implementation |
| core `skew` | later | a derived source of its own once the store takes derived producers |
| core `tcp` (the D5 stub) | not carried | a telegram channel is the TCP source |
| core `testkit` (the synthetic family writer) | not carried | the bench's emitters write the files (C-025); a test that needs a torn write uses the bench's writer or writes the file itself |
| `veredas-store` (`DuckStore`, `schema`, `error`, the capability probes) | `veredas_portal::store` | the session store, every protocol's data alike |
| `veredas-server` (`feed`, `api`, `lttb`; the SSE stream) | `veredas_portal::serve` | SSE, windowed REST, the write surface |
| `veredas-server` the embedded dashboard (`rust-embed`, `dashboard/dist`) | not carried | the web interface is supervisão's (P-008); portal serves data, not pages |
| desktop `session` (the pump, health, add/remove/reconnect), the OPC and telegram runtimes | `veredas_portal::session` | the ingest, UI-free |
| desktop `telegram` (the vendor's decision client and lifecycle events) | not carried (Q8) | a channel's replies are rules; its data is rows |
| desktop `config` (the string-level project), `project` (the JSON files, bundles, the legacy migration), `secrets` | `veredas_portal::project` (TOML, P-010) | one project file; passwords stay out of it; Veredas 1's JSON read into TOML |
| desktop `app`, `boot`, `style`, `icon`, `ui/*`, `tables` (the SQL builder), `main` | Q14 | the shell is the operator's call |

Changed on the way, each recorded in the clause's evidence note:
- edition 2024; typed errors where Veredas 1 returned `String` (the session's start, the project
  reader, the server's refusals as status codes with a typed body);
- patterns are `regex` syntax alone, as core's capture rules are; Veredas 1's `fancy-regex`
  fallback for lookaround is not carried (one engine, one syntax, across the suite's rules);
- `Value`, `SeriesType`, `Sample`, `SeriesRef`, `SourceId`, `SourceHandle`, `now_unix_ms` are
  core's, not redefined; `GroupType` folds into `SeriesType`;
- a telegram channel captures by a catalog, never by a `CaptureSpec` typed into a source: the
  catalog closes C-013's warning-on-a-bad-pattern gap (the bench note's follow-up), and portal
  captures outside a catalog nowhere;
- the OPC password is a project secret: the project file names the user, the password comes from
  the environment or a prompt (`docs/questions.md` Q15 asks where the suite keeps secrets);
- the project is one TOML file, not a folder of JSON files; a Veredas 1 folder is read into it
  once, by `project::from_v1`, with every field it carries mapped and every field it drops named.

## Module map
```
veredas_portal
  encoding    detection and streaming decode (UTF-8, UTF-16, CP1252)
  family      FamilySpec: a rotating file family by a name pattern, resolved in order
  timestamp   TimestampSpec: the stamp a line carries, located and parsed by ordered formats
  tail        FileTailSource, FamilyTailSource, backfill: lines from files, zero lost
  source      Source, SourceSet, Emitter, LineEvent: the line-producing worker model
  capture     CaptureRule: a line into typed series by a regex's named groups
  channel     a portal telegram channel: channel file + catalog + reply rules, rows per message
  opc         an OPC source compiled from the project, core's worker polled into rows
  store       the DuckDB session store: base tables, samples, views, windows, snapshots, CSV
  session     a project's sources running, pumped into the store and the feed; health
  serve       SSE, windowed REST, the gated write surface (feature "serve")
  project     the project file (veredas-project), migration, Veredas 1's folder read in
  main.rs     veredas-portal <project.toml> [--check]
veredas_core (added for portal, kept for every member)
  schema      one header reader: format and version read before the body, refused or migrated
```

Features: `store` and `serve`, both on by default; without them (controlador's embedding) the
crate holds the data plane, the channels, the OPC sources and the project file, nothing persisted
and nothing served.

## Phases
1. **The crate and the data plane.** `crates/veredas-portal` at `2.0.0-alpha.1`, listed; the
   component clause (P-002 promoted: a library and one executable, content-agnostic, core alone,
   its plan before its code); `encoding`, `family`, `tail`, `source`, `capture` with Veredas 1's
   tests against real directories and the bench's emitters (rotation, reopen, families).
2. **Telegram channels by rules.** `channel` against the bench channel with its controller and
   pressure: the catalog's check before binding, every message a row, replies composed by rules.
3. **OPC sources.** `opc` against the bench's OPC UA server: polls into rows, bits, on-change,
   the handle's whitelist.
4. **The session store.** `store` from `veredas-store`, a real DuckDB file, every protocol's
   rows in it alike; the capability probes pinned against the bundled engine.
5. **The server.** `serve`: SSE and REST over the session, the write surface refused by default
   and proven refused on the wire against the bench.
6. **The project file (P-010).** `project`: TOML with `format` and `version`, migration and
   refusal, Veredas 1's JSON read in; core's header reader replacing the four copies; the test
   over every file of the suite.
7. **The shell and the embeddable build.** Q14 asked; the most reversible option taken meanwhile:
   the executable and the build without default features.

Each phase ends with `cargo nextest run --workspace` green bare, `cargo test -p leis` green, and
its clauses recorded, refuted and verified. Push at the end of each phase.

## Traps
Recorded as the port meets them.

- **Check the disk before the first build.** 45 GB were free when this plan was written, this
  repo's target at 19 GB. The bundled DuckDB compiles its C++ once (about six minutes), and its
  debug static library is about 2.2 GB, mapped into every test binary that links it: Veredas 1's
  workspace profile compiles `libduckdb-sys` at `opt-level = 2, debug = false` so the linker does
  not exhaust the page file; this workspace does the same. Phases 1 to 3 took the free space
  from 45 to 33 GB (the OPC test binaries and a scratch copy's target for the refute rounds).
- **Plain ASCII tells nothing about an encoding** (C-035, round 1). Veredas 1 committed the
  decoder at the first four bytes, so a log that opens with an ASCII header was UTF-8 for good
  and its first degree sign read as U+FFFD. ASCII streams through undecided; the first byte
  beyond it decides between UTF-8 and CP1252, however far in.
- **A UTF-16 line feed is two bytes.** A tail attached at the end judged "mid-line" by the last
  byte alone, so a UTF-16 LE file ending cleanly (its last byte 0x00) lost its first new line.
  The attach reads the file's head to decide the encoding and compares the last code unit.
- **A file holding its mark alone, and an attach inside a code unit** (C-035, round 2). A logger
  writes the mark when it creates the file; a tail attached right after saw a file not ending on
  a line feed and skipped the first line written. And an attach whose raw length fell inside a
  UTF-16 code unit decoded every unit after a byte out of step, silent for good. A start at or
  before the mark is clean; the start offset is rounded to the unit boundary from the mark.
- **A torn lead in an ASCII-opened file is U+FFFD, not a CP1252 character** (C-035, round 2).
  The ASCII state decided CP1252 for any high byte held when the stream ended; a valid UTF-8
  prefix cut short is UTF-8 whether or not the stream is ending, and the decoder finishes it as
  U+FFFD. A byte no sequence starts stays CP1252.
- **A skipped fragment must not vote on the encoding** (C-035, round 3). An attach at the end
  inside a UTF-8 character of an ASCII-headed file fed the decoder a continuation byte alone,
  which no sequence starts, and the file read as CP1252 for good. While the head leaves the
  encoding undecided the torn fragment is dropped raw, before the decoder sees it.
- **A head under four bytes decides nothing alone** (C-035, round 4). A start from the beginning
  waits for four bytes or a line feed before deciding; an attach at the end of a one-to-three-byte
  file decided on what it held, and a torn mark or a lone UTF-16 character chose wrong for good.
  Such a head is pre-fed to a fresh decoder and votes with the bytes that follow it.
- **A short head is text until it proves a mark** (C-035, round 5). The byte a mark begins with
  is also a CP1252 letter; taking it for a torn mark let text from before the attach open the
  first line. The skip is decided once the encoding is, and a NUL beside a line feed in the first
  bytes waits for the UTF-16 pattern instead of taking the line-feed shortcut.
- **A locate must survive a rotation** (C-037, found by the suite). The backfill listed the family,
  then peeked each member; the fixed-name writer retired the active in between, the peek hit a
  file no longer there and the worker ended in error with no line emitted (one run in six). The
  locate lists again on a not-found peek, with patience.
- **A line written before the attach is history** (C-035, C-036, found by the suite). A live start
  attached on the worker thread; a test (or a plant) writing right after the start saw its lines
  skipped, one run in three under load. The start returns once the path has been looked at.
- **A family is followed by its keys** (C-036, round 1). A keyless pattern cannot be followed: no
  rank holds against a straggler; it is listed by time and refused for following.
- **An hour in a file name needs its minutes filled** (C-036, round 1). The date read back without
  minutes was midnight for every hour; the reader fills them, and a shape is one run of specifiers.
- **A wake channel with no sender spins the loop.** The watcher owned the wake channel's only
  sender; where the watcher could not be set up (a parent folder not there yet) the receiver
  read as disconnected and the wait returned at once, every tick, a full core. The loop keeps a
  sender of its own.
- **A same-size rewrite is not a truncation to a length check.** A rewrite in place that is back
  at or past the read offset by the next poll is read as a continuation; every poll-based tail
  shares the limit, and the clause states it rather than claim past it.
- **Planted folders of two tests in one binary must not share a name.** The component test's
  target check and graph walk each planted packages under the process id plus a case name, and
  one case name was common: run in parallel, one test removed the other's folder.
