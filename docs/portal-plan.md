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
- **The shell and the embeddable build**: decided (`docs/questions.md` Q14, 2026-11-09): portal
  is a library and one executable, `veredas-portal <project.toml>`, serving a project from the
  command line, and that executable is its shell; no desktop UI is built in portal, Veredas 1's
  `veredas-desktop` screens being supervisão's (P-008) to take up or leave. A build without the
  default features leaves the store and the server out; no consumer is named today (controlador
  embeds the full build, P-019).
- **The grep engine** (Veredas 1's grep tab over logs at rest): built as a library (C-094,
  P-019). **Later, named, no code now:** the clock-skew monitor as a derived source; the library
  of templates and recents; bundles.

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
| core `grep` | `veredas_portal::grep` (C-094, P-019) | the grep engine over logs at rest, on Oficina's first-release list, as a library |
| core `capture` (the grep tab's ad-hoc tables, CSV and Markdown), `highlight` | later, with the workbench (Q14 decided: no shell in portal) | operator tooling over the grep's results |
| core `store` (`SeriesStore`, `Point`, `SeriesInfo`), `DERIVED_SOURCE` | `veredas_portal::store` | the store's own surface, beside its one implementation |
| core `skew` | later | a derived source of its own once the store takes derived producers |
| core `tcp` (the D5 stub) | not carried | a telegram channel is the TCP source |
| core `testkit` (the synthetic family writer) | not carried | the bench's emitters write the files (C-025); a test that needs a torn write uses the bench's writer or writes the file itself |
| `veredas-store` (`DuckStore`, `schema`, `error`, the capability probes) | `veredas_portal::store` | the session store, every protocol's data alike |
| `veredas-server` (`feed`, `api`, `lttb`; the SSE stream) | `veredas_portal::serve` | SSE, windowed REST, the write surface |
| `veredas-server` the embedded dashboard (`rust-embed`, `dashboard/dist`) | not carried | the web interface is supervisão's (P-008); portal serves data, not pages |
| desktop `session` (the pump, health, add/remove/reconnect), the OPC and telegram runtimes | `veredas_portal::session` | the ingest, UI-free |
| desktop `telegram` (the vendor's decision client and lifecycle events) | not carried (Q8) | a channel's replies are rules; its data is rows |
| desktop `config` (the string-level project), `project` (the JSON files, bundles, the legacy migration), `secrets` | `veredas_portal::project` (TOML, P-010) | one project file; passwords sealed in a secrets file beside it, the key in the environment (Q15); Veredas 1's JSON read into TOML |
| desktop `app`, `boot`, `style`, `icon`, `ui/*`, `tables` (the SQL builder), `main` | Q14 decided: not portal's | the executable is the shell; the screens are supervisão's (P-008) to take up or leave |

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
- the OPC password is a project secret: the project file names the user and an entry of the
  sealed secrets file beside it (`veredas-secrets`, ChaCha20-Poly1305 under a key held in the
  environment variable the project names, written by `veredas-portal --secret`; `docs/questions.md`
  Q15 decided);
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
  grep        search over logs at rest: patterns combined per line, a tree, a list or a family
  channel     a portal telegram channel: channel file + catalog + reply rules, rows per message
  opc         an OPC source compiled from the project, core's worker polled into rows
  store       the DuckDB session store: base tables, samples, views, windows, snapshots, CSV
  session     a project's sources running, pumped into the store and the feed; health
  serve       SSE, windowed REST, the gated write surface (feature "serve")
  peer        another portal read over its HTTP interface: stream, snapshot, windows (feature "peer")
  project     the project file (veredas-project), migration, Veredas 1's folder read in
  main.rs     veredas-portal <project.toml> [--check]
veredas_core (added for portal, kept for every member)
  schema      one header reader: format and version read before the body, refused or migrated
```

Features: `store`, `serve` and `peer`, all on by default; without them the crate holds the data
plane, the channels, the OPC sources and the project file, nothing persisted and nothing served
(no consumer named today; controlador embeds the full build, P-019). `peer` needs no store: a
light portal reads a full one.

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
7. **The shell and the embeddable build.** Q14 decided (the architect, 2026-11-09): the executable
   stands as the shell, no desktop UI in portal; the build without default features for
   controlador's embedding.

Each phase ends with `cargo nextest run --workspace` green bare, `cargo test -p leis` green, and
its clauses recorded, refuted and verified. Push at the end of each phase.

## Decided in the briefing of November 2026, not built
The architect's decisions of 13 to 19 November 2026 that touch portal (`docs/oficina-plan.md`,
`docs/controlador-plan.md`, `docs/robustness.md`); status P-019. They override the lines above
where they differ.
- **Controlador embeds the full portal**, with its store and its server: "controlador embeds a
  full version of portal" (2026-11-15). The build without the default features stays a build of
  this crate, with no consumer named today; the lines above that say "for controlador's embedding"
  are overtaken.
- **Inside a controller, portal reads controlador's memory** (the variables list, the CPU's
  states) asynchronously, outside the scan; controlador owns the real-time communication
  (2026-11-15).
- **Oficina embeds a portal too, and two portals speak portal's own HTTP interface**, "http
  (https, later)": the event stream, the windowed reads, the gated write surface, on the
  controller's loopback behind SSH. Forcing, breakpoints, downloads, start, stop and acknowledge
  travel on the same link: portal carries them, and controlador handles them behind its portal
  (2026-11-16). **Built: the reading side (C-095)**, the module `peer`: `PeerClient` reads
  another portal's snapshot, listing, series windows, table pages and charts, a tag on demand,
  and follows its event stream (hello, data, lag), every call a connection of its own speaking
  HTTP/1.1 itself over a tokio socket, bounded in time (ten seconds), in size (32 MiB an answer,
  64 KiB a head) and, for the stream, in silence (15 seconds, three of the server's keep-alive
  comments missed). An error is the peer's own text with its status. A stream that ends stays
  ended: the reader opens a new one, whose hello says where the peer is, and reads the gap again
  by window. Not built: the commands, which wait for Q37; storing what is read in the reader's
  own store (a peer as a source), until Oficina says what it keeps.
- **The embedded portal serves OPC UA and Modbus/TCP** to other controllers and higher-level
  systems: the controller's state, its faults and the variables the program marks. Its master
  switch is the gate for writes from outside, and its whitelist holds exactly the variables the
  program marks writable. A protocol is bound only when the project names the interface; OPC UA
  writes need a signed and encrypted channel and a user; Modbus writes come only from listed
  client addresses (2026-11-15).
- **Controller state changes and faults are stored like any other data**; the history view reads
  them. **Built (C-092)**: a controller source fed in process by the embedding
  (`Session::controller_feed()`, `push_state` with core's `ControllerState`, `push_fault`), rows
  of `controller_states` and `controller_faults`, series, frames and health like any other source's.
- **On a controller the store is a window in memory by default**: the last stretch of values, as
  long as the project sets, nothing written to the drive. Recording to the drive is the project's
  choice, in files with a size limit; at a fault the window is written to the drive with the fault
  record (2026-11-19; `docs/robustness.md`, "Wear and filling"). **Built: the window (C-090)**,
  `store = { window_s = N }` or `SessionConfig::store_window`: the rows that arrived within the
  span (the arrival clock, not the plant's stamp), the older let go once a second by the pump
  through the ingest connection with a plain DELETE (an in-memory store hands the space back,
  probed and pinned in C-042), the listing still counting what was seen. **Built: the recording
  (C-091)**, `record = { dir, max_mb, every_s }`: a second store on the drive with the same
  tables and views, written in one batch every few seconds and at the stop, stopping at its size
  limit with nothing deleted (the answer taken while Q41 was open; ruled since, below: the
  oldest goes); refused beside a store on the drive, which already holds every row. **Built: the
  copy at a fault (C-093)**, `SessionConfig::fault_copy_dir`: at every fault record the store is copied
  to a file of its own through the ingest connection, the record in it.
- **What gives way at a size limit, and the window's bounds** (Q41, ruled 2026-11-25; not built):
  a growing record on a controller's drive deletes its oldest when it reaches its limit, a fault
  that repeats counted on its record and not written again, the running program and its fallback
  never deleted. So the recording (C-091) lets its oldest batches go instead of stopping, and the
  fault's copies (C-093) are limited in number, the oldest going first, at most one written per
  shortest gap, the program able to ask for one. The window (C-090) gets a ceiling in bytes beside
  its span, the shorter of the two ruling and the controller told when the ceiling came first;
  the engine's default memory limit is never left as it is on a controller. The window holds
  everything portal reads unless the project narrows it. A data area that fails or cannot be
  written stops the recording with its reason, and RUN goes on (controlador raises the
  maintenance request). A store on a PC keeps no limit (Q16).
- **The mark of an unconfirmed date in every sample** (Q39, ruled 2026-11-25; not built): every
  sample of the embedded portal's store carries the controller's mark, and the store's files gain
  it by a new format version, read by migration (law L7: migrate or refuse).
- **A command passes both switches, and its answer tells both stages** (Q37, ruled 2026-11-25; not
  built): portal's master write switch, then the controller's commissioning switch; the answer
  says applied, then saved. The numbered, digested requests of Q37 are the Oficina link's, which
  portal carries.
- **One portal instance per project on a host that runs Oficina and supervisão** (Q31 entry 22,
  ruled 2026-11-25; not built): the first of them to need the project starts it, the other joins,
  it stops when the last leaves (Q19 amended for such a host); SQL over a running session's store
  goes through a read-only route of portal's.
- **A token on every request** (Q31 entry 33, ruled 2026-11-25; not built): a random token per
  session, handed over SSH and required on every request to the HTTP interface, with Host and
  Origin checks, for portal and supervisão; controlador's engineering channel is its own (Unix
  sockets, named pipes on Windows). Q29 and P-014's two items are answered by it.
- **The grep engine** over logs at rest is on Oficina's first-release list. **Built (C-094)** as
  the library module `grep` (the light build keeps it): Veredas 1's engine without `fancy-regex`
  and without ripgrep's crates, every file decoded by portal's own decoder in the tail's own slices
  (as a tail reading it whole from its start; a live tail may have settled UTF-8 or CP1252 otherwise
  on a file mixing them), no link followed, a line past 4 MiB searched in its first 4 MiB (the
  cut its end to the patterns), a cap of 0 giving no line, a cancel heard while a file is read
  (at every slice; a line in hand is scanned whole first).
- **A log says when its start's history is in the store** (Q319, 2026-11-25; **built, C-096**):
  each log's health carries `history_read`, said once the lines its worker had sent by the end of
  every file as it stood at the start are stored; a comissionador replay and supervisão's host
  wait for it instead of guessing from stillness. A history that cannot be whole (a member
  there and not readable when the backfill reaches it, a flush that failed, a worker that failed)
  is never said read, so a waiter needs a deadline of its own.
- **Every gated write and send kept in the store with the moment it was answered** (Q318,
  2026-11-25; not built, C-097 proposed): a table of the session's own, `writes`, one row per
  write or send: when it was asked and when answered (the answer's arrival its stamp on the
  store's axis), the source, what (an OPC tag and its value; a channel's message and fields),
  and the answer (each tag's status; the send's outcome; a refusal by the switch, so a refused
  write is a row too). Kept off the narrow surface (its columns are no series of the source).
  Portal records what passes through it: `ChannelHandle::send` already does, and an OPC write
  needs a call of portal's (`Live::write_tags`, core's handle inside it), the HTTP surface's
  write moving to it and comissionador's `plant.rs` write with it (through the orchestrator).
  Its columns follow what comissionador states it reads.
- **Q14 stands:** portal builds no desktop UI. Oficina is a component of its own.
- **The licence** (Q35, ruled 2026-11-25; done): `MIT OR Apache-2.0`, the two texts beside the
  manifest, the copyright holder thebdr.
- Open: the rest of the security round (Q43, P-014); how the window is built so that it hands
  memory back, portal's to prove (Q41).

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
- **A derived default is not the file's default** (C-046). The session table absent, `Default::default()`
  gave the idle window 0 and the project refused itself; a table whose keys have documented
  defaults implements `Default` by hand, with those defaults.
- **A fixed active absent at the start is new content** (C-036, round 2). Waiting for it held the
  start five seconds and then attached at its end, losing the first line; a listing with no active
  signals the attach, and the file read from its start when it comes.
- **A fixed active born after the start may retire before it is opened** (C-036, round 3). Marking the
  listing known at the open lost every generation born meanwhile; such a family is followed as one
  born after the start, the pre-start retirees seeded as history.
- **Two validators of one configuration drift** (reconcile of C-043 and C-046). The file and the
  session each checked names in their own way; one type (`project::Claims`) and one channel check
  (`channel::check`) now serve both.
- **A grace tuned on an idle machine loses under the suite** (C-036). The straggler scenarios drained
  a retired file for 120 ms while the straggler lands after 15; the whole suite in parallel held
  the writer past the grace once and the line was lost. Test graces carry headroom for load.
- **An open that fails is not a file that is absent** (C-035, round 7). A held file read as absent
  was later read whole, its history flooding the stream; only NotFound is absence, any other
  failure keeps the configured start for the open that succeeds, and the length and creation time
  recorded at the look tell the same file (read from where it stood) from a replacement (read whole).
  Round 8 found the start applied to the replacement, losing its first lines.
- **A family's held active is a file's held active** (C-098, from C-096's rounds 3 to 6). The
  family started at its end reopened its held active at its end on every retry, losing what was
  written while it was held; it keeps the first look's length and identity as the file does, and
  knows the held file by its identity, since a fixed name never changes. A held open of the fixed
  path sweeps what retired since its listing, as the adopt does.
- **A held file is in no listing while it is renamed in two steps** (C-098, round 3): a rotation
  through a name outside the pattern leaves it in no member while a new active stands at its path,
  and the live loop takes it for a member dropped in once it lands. It is looked for in the folder
  by its identity and read where it is, its identity kept so the landing reads nothing twice. A
  held file cannot be read, so a copy-and-truncate beside it is told by its length alone: watched
  while it stands at its path, a file shorter than the most of it seen was cut. Its bytes, which
  would tell a copy from a file dropped in, are the one thing a held file never shows.
- **A fixed active the sweep's open finds gone may have been renamed after the listing** (C-096,
  round 3): a rename is atomic, so a listing taken after that open shows it, and only then is
  the history read.
- **A test build right after clippy may not find an rlib.** Clippy leaves check-mode artifacts beside the
  build ones; a test build (nextest above all) started right after it failed four times with "crate
  `libduckdb_sys` required to be available in rlib format" and compiler internal errors, and a plain
  `cargo build --workspace --tests` healed it each time. Clippy runs after the suite, never before it.
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
- **An unknown identity is no identity.** A hold that refuses every read still lets a zero-access
  open through, but an access-control list that denies the reader its attributes refuses even
  that, while the length stays readable from the directory entry. Taking "no identity read" for
  "the same file" attached a successor at the old length and lost its first lines (C-035, round
  9). Unknown matches nothing; on Unix the device and inode come from the directory entry. A look that cannot read even the length records zero and no identity, so whatever opens is read whole (round 10).
- **The live snapshots are refreshed every 500 ms.** The series, health, vitals and counters a
  session publishes are the pump's last refresh, not the store: a test that reads one right after
  the store shows a row may read the refresh before it (CI, Linux, the counters one pass behind).
  Every read of a published snapshot waits for its condition, as the health and series reads do;
  the server test read its listing a refresh too early on CI as well (the channel's series).
- **A retired file may have shrunk.** The drain of a file renamed away took "shorter than what
  was read" for "emptied": a truncation and a rotation landing between a poll's read and its
  look at the path lost the torn line the truncation left (CI on Linux, where no watcher woke the
  poll between the two). A retiring file found shorter is reread from its top to its stable end.
  The deterministic test shrinks the retired file during a long drain grace; operations that
  merely share a poll are seen by the read before the look and never reach that path.
- **A boundary is where the critic looks.** The backfill's first refute round found its gaps at
  the edges every fixture avoided: two lines on one second across a seam (10 ms steps never made
  one), a peek of an empty file "detecting" UTF-8, a plan computed on a file the listing did not
  name (the active recreated between the listing and the peek), a family of one empty file
  refused as unparseable. A fixture set with distinct stamps, non-empty files and a still family
  proves the happy path alone; the round-1 tests of C-037 hold the edges.
- **A test file that imports a feature's module declares the feature.** `cargo test
  --no-default-features -p veredas-portal` compiled every integration test, and the five bound to
  the store or the server (`store`, `store_engine`, `session`, `project`, `serve`) failed to
  build, so the light build had no runnable test set. Each is a `[[test]]` in the manifest with
  `required-features`; the light set is the data plane's eight binaries, 130 tests.
- **A fix is a change to refute.** Two of the backfill's round-1 fixes broke cases the code before
  them handled (a shortcut over the kept files forgot the prefilter's drops; a strict comparison
  lost the start equal to the first stamp), and the existing tests held neither case. A round's
  fixes go back to the critic as a round of their own, and the test a gap earns asserts the
  neighbouring cases too (the start on, before and after the edge), not the gap alone.
- **A preference falls back to the order, not around it.** The extractor's hinted loop went on
  from the preferred format and wrapped, so with three formats a format that had never matched
  could win over the first configured one and then stick; the two-format test could not tell a
  rotation from the order (C-038, round 1). A test of "first this, then the order" needs three.
- **Two things a capture rule's author meets that no clause promises away:** a timezone in a
  format (`%z`, `%:z`) is parsed and dropped, the stamp staying naive local; and a bare carriage
  return the tail keeps (C-035) sits before the end of the line, so a `$`-anchored pattern misses
  it: match `\r?$`, or do not anchor the end.
- **A refusal the handle makes alone is still the status's.** The switch stopped a send in the
  handle, before the worker, so the refusal never reached the shared status and the last refusal
  stayed an older one; and the peer was asked for before the catalog, so a misspelled message
  with nobody connected read as "no peer" (C-039, round 1). Every refusal path writes the status,
  and the order of refusals in the clause is the order of the code, tested with nobody connected.
- **What compile accepts, the poll must read.** Compile allowed bits on a float tag while core
  carves integer words alone, so such a tag polled a hole per bit forever after compile said yes
  (C-040, round 1). A compile-time check mirrors the reader it guards, and a test of "misspelled
  keys refused" misspells optional keys, since a misspelled required key fails for the missing
  one with or without the unknown-field check.
- **A surface built from the whole catalog fails whole.** The table listing described every
  view before answering any question, so one linked CSV missing for a moment hid every base
  table from the server (C-041, round 1). List what can be described and mark the rest; answer a
  question about one table from that table alone.
- **A cast in SQL is the engine's display.** `CAST(26.0 AS VARCHAR)` is "26.0" where the suite
  shows "26", and the test had pinned both; read a value typed and display it in Rust.
- **Ties need a tiebreaker, and an attach needs its detach.** Newest-first pages over equal
  stamps repeated and skipped rows until the sequence broke the tie; a failed `COPY FROM
  DATABASE` left its `ATTACH` in place and every later snapshot refused. A statement that must
  run whatever happened before it is its own statement.
- **A sweep stops at the file the cursor holds.** The pre-read sweep of a freshly adopted active
  skipped the cursor's own retiree (no duplicate) but read the retirees newer than it, so two
  rotations inside the sweep's drain grace put a generation before the cursor's own (C-036,
  found by CI, reproduced deterministically with a long grace). Order needs the sweep to end at
  the cursor's file; the next sweep, after the cursor has read it, takes the rest.
- **Stop the producers before the consumer.** The session joined its pump before its workers, so
  what the workers sent after the pump's last pass, and any backlog past one pass's cap, was lost
  on a stop, and no refresh ran after the workers ended, so a stopped source read live for ever
  (C-043, round 1). The workers end first; the consumer drains dry and publishes the end.
- **An idle stream is one nobody feeds.** The keep-alive comment was asserted on a stream whose
  session polled the plant every 50 ms: a frame every 60 ms, never five idle seconds, and the wait
  timed out (C-044, round 1). Stop the producer first, wait for its source to read retrying, then
  wait for the comment; a disarmed body answered 403 only once the handler took the body as bytes
  and parsed it after the switch, the framework's extractor having answered 422 before it.
- **An extractor answers before the handler.** `Bytes` buffered the body under the framework's
  2 MB limit and refused a larger one 413 in plain text before the switch was consulted
  (C-044, round 2). The handler takes the request, checks the switch, then reads the body itself
  under its own limit, every refusal in the error form.
- **A lag is counted, never "once".** How many lag events a flood raises is the scheduler's: the
  receiver reads between the sender's pushes and lags each time it falls the channel's depth
  behind (thirteen and sixteen events in two runs of one test). Pin the sum, frames received
  plus counts carried, against the frames sent. And stop every live source before a flood: one
  broadcasting during the drain lagged a slow client a second time and took the awaited frame
  with it (CI on Linux, C-044's first round). Scan a growing buffer from where the last scan
  stopped.
- **An untagged enum swallows the key.** `start = { last_hourz = 4 }` read through
  `#[serde(untagged)]` gave "did not match any variant of untagged enum StartFile", and an extra
  key passed unseen (C-046, round 1). A key that takes a word or an inline table is read as a
  `toml::Value` and judged key by key, with serde's own `unknown_field` and `missing_field` so
  the words and the parser's place stay those of every other refusal.
- **An import that does not load its own output guesses.** `--from-v1` printed a start it had
  not read and a server with no rule, and `--check` refused the file it had just written (C-046,
  round 1). The import loads what it is about to print and refuses the folder, with its notes,
  when that fails.
- **What is not carried takes its dependents with it.** The import dropped a disabled source but
  carried the rule scoped to it, and its own self-check then refused the whole folder (C-046,
  round 2). Every part left out is followed to what names it; each is left out and named, so the
  file printed loads and the operator reads why.
- **A reading of SQL is the wrong tool.** Five rounds of the project file's import tried to tell
  the tables a view reads from the words of its SQL, and each round found another FROM (a
  quoted name, a comma join, EXTRACT, IS DISTINCT FROM, a CTE's column list). The engine that
  will run the file judges it: the import compiles the tables as the session does and opens an
  in-memory store, and what the engine refuses is left out with the engine's reason (C-046,
  round 5). What the engine cannot read where the import runs (a CSV on the plant's machine) is
  no fault of the file: carried as written, named.
- **A share mounted on Linux reads as a local path.** `tail::pathkind` knows a share by its
  Windows shapes alone, a UNC path or a drive letter `GetDriveTypeW` calls remote; on Linux an
  absolute path has no prefix and is local, so a tail of a CIFS or NFS mount (`/mnt/...`) arms a
  watcher that no write from another machine wakes. Nothing is lost: the poll reads every line
  at its interval, as on a Windows share, and `use_notify = Some(false)` turns the watcher off by
  hand. No poll interval follows the kind on either system; `poll_interval` is the operator's.
  The fix reads the mount's filesystem type (P-013; found by the station-hardware research,
  2026-11-10).
- **A port a test picks is anybody's until it binds it** (C-044, the flaky wait, diagnosed on the
  Linux machine on 2026-11-22). `free_port()` bound port 0, read the number and closed the socket;
  the bench bound that number later, on its own thread, after its start had returned. With ten
  sessions binding and dialling at once another socket took the port between the two about once
  in five hundred runs: the bench's listener failed (`Address already in use`) with the bench
  reporting a start, and the session waited for a peer that was never there. A listener binds
  port 0 itself and the test reads its port back (`tests/common/ports.rs`, from the channel's
  `listening on <addr>`); a port where nobody may answer is held bound and listening for nobody.
  Played under a churner in a private network namespace (`unshare --user --map-root-user --net`,
  no sudo), the old fixture failed 9 runs in 12 and the new one none.
- **The pump reads through its own connection** (C-043, found by the same diagnostic). Its
  refresh read the store's memory figure through the server's query connection, so a client
  polling the store (or one heavy window) held every pass behind that lock: 16 passes in 16
  seconds. Whatever the pump reads, it reads through the ingest connection, which nobody else
  holds; a reader blocked for good (an export into a pipe nobody opens) proves it.
- **A sniffer that collapses a ragged file hides its dialect** (C-041, round 4). On a ragged file
  quoted with `'`, the engine's sniffer settles on `|` and one column; asked to pad
  (`sniff_csv(..., null_padding=true)`) it names `,` and `'`. And the padded reader refuses a quoted
  line break unless it runs on one thread (`parallel=false`), and a forced reader left to find its
  own quote may find none and refuse what it reads whole when given one: the sniffer's quote and
  escape are always written out, empty for one it did not see (round 5). The raggedness count
  reads the file in that dialect as the engine does: a quote opens only at a field's start, a row
  ends at LF, CRLF or a bare CR, the window counts rows and a blank line moves it by none; a count
  that fails is the read's refusal, never a total of nothing.
- **The sniffer's delimiter is not the file's when its rows all hold commas** (C-041, round 8).
  A `;` file with commas in its text, its lists, a column name or its decimal numbers is named `,`
  by the padded sniff, and the pieces of `1,5;2,5` type as many numbers on `,` as the decimals do
  on `;`. The header tells: on the wrong delimiter it splits into one field, or fewer than its
  rows. A dialect is read only where the header fits the rows; raggedness is still read in the
  sniffer's own dialect (a file regular on `;` loses its header line to the sniffer on `,`). Title
  lines are not detected: one over a table with a short row stays the sniffer's one text column.
  And the header alone can lie (round 9): a `;` file whose units hold as many commas as its
  decimals fits `,` too, so a fit never replaces a dialect with one typing fewer numbers; and
  where the rows' counts tie (round 10), the dialects that fit are weighed by their numbers. The
  numbers can lie too: a list `10,20,30` cut on `,` types its items, wherever it stands and
  whatever its numbers, and a `;` export with one decimal column whose name holds a comma ties
  with `,`. Rounds 11 to 13 tried to tell a list from a cut (a column cut out of another
  reading's list of numbers no number); each refinement broke `;` exports of decimal commas, far
  commoner (ids glued to a decimal's integer part pass for a list), so the weighing is round
  10's again and those files are stated limits (round 14 gave their real reach). The weighing
  proposes; a view names its dialect (`CsvOptions` written into its reader), and a link's page
  shows every reading `store::csv_readings` gives, each with its first rows, for the person to pick
  (Q131, ruled 2026-11-25).
- **A transaction does not freeze a file** (C-041, round 11). The engine reads a CSV again at
  every statement, so a page's count and rows over a view of a log still being written were two
  reads of it, though one transaction held them. A view's window is read once into a table of the
  page's own transaction; a table's page reads in place, its snapshot the transaction's.
- **A public name of portal is supervisão's business too.** Supervisão's policy test reads portal's
  public functions and fields by name and refuses one its own sources hold that it has not
  reviewed: `CsvOptions.escape` and a session's `window` failed it (supervisão names an HTML
  `escape` and its time windows). New public names are checked against supervisão's sources, or
  run its `writes` test, before they land.
- **serde_json reads about one double in seven a unit off** (C-095, round 6). Without its
  `float_roundtrip` feature it rounds a decimal twice: `1.4000000000000001` (14 × 0.1) reads as
  1.4. The feature would rebuild the database engine (serde_json is a build dependency of its
  crate) and change every member's reading, so portal's peer reader parses JSON itself, each
  number through Rust's correctly rounded parse. A test of exact values needs values that are
  not exact in binary.
- **A feature turned on in a shared crate moves other members' builds** (C-095). Portal's first
  reader used hyper's client side (`client`, `http1`): every crate sharing hyper was built anew,
  the database engine's build script among them (six minutes and the memory that kills a
  machine), and supervisão's graph without portal gained two crates its review had not read
  (`try-lock`, `want`). The reader speaks HTTP/1.1 itself over tokio instead; a new feature on a
  crate the workspace shares is weighed before it lands.
