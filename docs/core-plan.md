# veredas-core: the plan

The foundation of Veredas 2. `docs/veredas-plan.md` ("veredas-core: the foundation") wins on
intent; `.zen/contract.md` (P-001, then the `C-xxx` clauses it became) wins on status. This file
says what core holds, what it took from Veredas 1, in which order, and the traps met on the way.

## The role nobody covers
**Core holds every type and protocol layer that more than one component needs, as a library
that depends on no other member, so the whole suite speaks one wire and one value model.**

No other component answers this: portal is the gateway (it *uses* the layers to reach the
plant), maquinar plays the hardware (it *uses* them to speak the device's wire), aranha scans
the network (it *uses* the OPC client and the fixtures). Without core each would carry its own
copy of the socket and the codec, and the copies would drift; Veredas 1 split the telegram
crate out for exactly that reason. Core is content-agnostic: it moves and reads bytes by
configuration and names no device (`docs/questions.md` Q8).

## Scope
- **Shared types:** the value model (`Value`, `SeriesType`), the sample on its two clocks
  (`Sample`, the parsed or arrival index; `now_unix_ms`), source and series identity
  (`SourceId`, `SeriesRef`: a tag is a series of a source), and `Quality` (good, uncertain,
  bad), the three-way verdict every reading carries whatever its protocol.
- **Worker plumbing:** the stop/join handle of a worker thread (`SourceHandle`, `stop_flag`)
  and its error (`SourceError`), so every source, a tailed log in portal or an OPC poll here,
  has one health model.
- **The TCP telegram layer:** the hardened socket runtime (`Channel`); a framing engine
  (`framing`) that runs whatever layout a versioned channel file describes (`schema`,
  `docs/channel-schema.md`), with no layout of its own (Q9); the capture rules engine that
  reads message fields by configuration, and its inverse, the compose engine (`compose`) that
  writes a body from named fields; and the catalog file (`catalog`, `docs/catalog-schema.md`),
  a channel's messages by name, id and body rules, read and written by those two engines.
  No device's catalog, state words or decision logic: those are the rules a portal channel is
  configured with (Q8), and the bench's controller is driven by the same files.
- **Seeded randomness:** the deterministic PCG32 (`rng`), for weighted draws and fixtures.
- **The OPC UA client layer:** the compiled source shape (`PollSpec`), value conversion, the
  mapping of a poll to rows, the address-space browser, the poll worker with its gated writes.
- **The controller's state:** the structured state of a controller (a PLC) and its one line
  of text, with one writer, one reader and the table of reason numbers and words
  (`docs/state-text.md`; `docs/controlador-plan.md`, "The state").
- **The fieldbus masters:** the functions that drive a plant's devices over a standard fieldbus
  (`fieldbus`), a standard's frames being code here (Q9, revisited): a Modbus/TCP client first.
- **The vendor description files' readers** (proposed, not built; its pending asked of the
  orchestrator): GSDML for PROFINET, ESI for EtherCAT and EDS for EtherNet/IP, read for Oficina's
  hardware configuration, the bench and controlador ("In core", Q45's C5, ruled 2026-11-25,
  against a member of their own and Oficina). They are inputs, read and never written (Q31 entry
  23): the TOML rule's one named exception, copied into a project as their vendor wrote them.
  GSDML and ESI are XML from outside, parsed with document type definitions and entities switched
  off; EDS is ODVA's keyword text. The test fixtures are descriptions written here, never a
  vendor's file, those being their vendors' copyright.
- **Test-fixture plumbing:** the in-process OPC UA demo plant (`opc::demo`, behind the
  `demo-server` feature), seeded randomness.
- **Later phases:** plain-text file schemas with their version stamps (the first being a
  telegram channel's configuration, with portal), logging.

## Rules
- **Designed as if a second consumer already existed.** A type here never names its first
  caller; its docs say what it means on the wire or in the plant, not who uses it.
- **Content-agnostic.** No device, vendor, message catalog or vendor specification is named or
  cited in core (Q8); what a device's telegrams mean is configuration.
- **A library, never an executable.** No `[[bin]]`, no `src/main.rs`, no `src/bin/`, no
  examples: a standing demo plant to drive by hand is maquinar's (the plan, "veredas-maquinar":
  a configurable OPC UA server). `tests/foundation.rs` asks cargo what it would build: one
  library and tests, nothing else.
- **Depends on no other member** (law L4). Its tests too: core cannot dev-depend on maquinar, so
  the fixtures its own tests run against (the OPC demo plant) live here, as fixture plumbing,
  and maquinar builds on them rather than the other way round.
- **Typed errors on the public surface.** Veredas 1 returned `Result<_, String>` from the
  hex parser, the OPC browser and the worker; the port gives each a typed error whose
  `Display` is Veredas 1's text, so the operator reads the same words. No `unwrap`/`expect`
  on an I/O path; tests may unwrap.
- **Evidence is reality.** The telegram channel is tested over real loopback sockets; the OPC layer against the in-process demo plant speaking real OPC UA over
  TCP. A mock is never the evidence of a clause.
- **One test file per capability** under `tests/`, so each contract clause links one file and
  a change to one capability re-stales only its own recorded run.

## What it absorbs from Veredas 1
The generic part of `crates/veredas-telegram`, all of `crates/veredas-opc`, and the slice of
Veredas 1's `veredas-core` the OPC layer stood on. Read in full before the port; what moves
where:

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| `veredas-telegram` `channel`, `Endian`, `capture` (its `CaptureSpec` rules); its `codec` generalised into `framing` and `schema` (Q9) | `veredas_core::telegram` | portal and maquinar both hold telegram links, framed and read by rules |
| `veredas-telegram` `genibelt`, `sortplan`, `decision_client`, `capture::scanner_parcel` | not carried (Q8) | one vendor's catalog and sorting logic: rules a portal channel is configured with, not suite code; ported, refuted and fixed here first (commits up to b7c6d0a) |
| `veredas-telegram` `rng` (`Pcg32`) | `veredas_core::rng` | maquinar's simulators seed it; weighted draws scale it |
| `veredas-opc` `spec`, `value`, `map`, `browse`, `worker` | `veredas_core::opc` | portal polls, browses and writes; aranha discovers OPC servers |
| `veredas-opc` `demo` (feature `demo-server`) | `veredas_core::opc::demo`, same feature | the fixture core's own tests and aranha's run against |
| `veredas-opc` `examples/demo_server.rs` | not carried | an executable; the standing demo plant is maquinar's |
| Veredas 1 core `SourceId`, `Value`, `SeriesType`, `Sample`, `SeriesRef`, `now_unix_ms` | `veredas_core::types` | the OPC layer produces them, portal's data plane will too |
| Veredas 1 core `SourceHandle`, `stop_flag`, `SourceError` | `veredas_core::source` | the OPC worker returns the handle portal's tail workers will return |

Not carried into core: the rest of Veredas 1's `veredas-core` (tailing, log families, grep,
regex capture, the store trait, `LineEvent`, the `Source` trait and `SourceSet`), which is the
data plane and goes to portal (P-002); `DERIVED_SOURCE`, a store concept, with it.

Changed on the way, each recorded in the clause's evidence note:
- edition 2024 (Veredas 1 is on 2021);
- the public `String` errors typed (OPC browse, worker, node-id parsing, hex parsing, the
  demo plant's start);
- `Quality` added: Veredas 1 judged an OPC read good or not from its `StatusCode`; the
  three-way severity becomes a shared type and `TagRead::quality` reports it;
- the vendor's name and specification citations removed from what stayed (Q6, Q8): the
  codec's fixed layout became the channel schema's worked example (Q9), pinned by golden frames.

The OPC layer, on the way in (phase 3; each is pinned by the clause's test):
- typed errors: `OpcError` (naming the service `Call`), `NodeIdError`, `WriteError`, `DemoError`;
- a write says what the plant may have seen. Disarmed, not writable, and not sent (no session, or
  still queued when the worker ended) mean the plant saw nothing. `Unconfirmed` means the call
  failed or timed out, or the worker stopped mid-call: the write may still land, because a link
  that stalled unnoticed delivers it when it recovers, and no client can recall bytes on the
  wire. Veredas 1 called both kinds a refusal;
- a write or read asked of a worker with no session is answered at once (`NotConnected`).
  Veredas 1 queued it and fired a command pulse whenever the plant came back;
- a call that fails or times out ends the session, so a wedged session is not reused;
- a request whose caller stopped waiting before the worker reached it is never sent, so a pulse
  an operator gave up on, and pressed again, does not land twice later;
- the browser treats a view as worth expanding (Veredas 1 showed it as a leaf), and runs every
  browse page, the first and each continuation, through one refusal check;
- the worker checks the master switch again just before the wire. Veredas 1 checked only in the
  handle, so a write already queued fired after a disarm;
- every connection attempt is bounded (`CONNECT_TIMEOUT`, 10 s) and raced against stop, in the
  worker and the browser. Veredas 1 could hang on a peer that accepts TCP and never answers;
- a stop of the worker, or a drop of the browser, takes a fraction of a second, whatever the
  peer does. Its CloseSession is bounded at 100 ms (250 ms when a failed session is dropped).
  Both threads run on `opc::run_on_own_runtime`, which shuts the runtime down without waiting
  for a host name still resolving, and an abandoned session's event loop is aborted. Veredas 1 waited 2 s for a silent peer's close and joined
  whatever the browser was doing;
- the browser reads at most 64 pages under one node, counting the first (Veredas 1 read 65), and
  drops a child listed twice wherever it falls;
- a zero poll interval is refused at spawn (Veredas 1's worker thread panicked in tokio);
- bits are read at the wire word's own width. Veredas 1 sign-extended, so bit 20 of an Int16 −1
  read 1, and gave a UInt64 above `i64::MAX` no bits. A 64-bit field with its top bit set is
  refused rather than wrapped;
- a bad read of a word polled only for its bits is reported (Veredas 1 reported nothing), and a
  good status with no value, or with an empty one, is reported once as "no value";
- a float stuck at NaN counts as unchanged, so storing on change still skips it;
- every refused wire type is named: Veredas 1 called five of them "unsupported";
- the identity's password shows in neither its label nor its debug form;
- the one-line renderings are one line (a control character or a Unicode line separator reads
  as a space), and the browser's value brief is cut by characters, not bytes;
- the demo plant caps a Read at 800, below its bulk folder and the stack's array limit, so the
  worker's chunking is proven rather than assumed. It also has:
  - folders of 64 and 65 children, for a server set to one reference per page;
  - a child linked twice;
  - a slow tag;
  - a freezable relay, to make a link go silent;

  and it holds its namespace index and reports why it did not start;
- comments that named callers, vendors, products or field sites now say what the code means on
  the wire or in the plant.

## Features
Core's layers come by cargo feature, so a component carries what it uses (C-073):
- `net`, on by default: `types` (chrono), `source` (regex), `telegram` (tokio, socket2, regex) and
  `opc` (the OPC UA client). Every component that reads a plant keeps the default and sees no
  change.
- Without the default features (`default-features = false`): `schema`, `rng` and `state`, on
  serde, toml and thiserror alone, for a component that reads the suite's stamped files and never a
  plant (ábaco's `.abaco` workspace file, 2026-11-23).
- `demo-server` (the OPC UA demo plant, with `net`), `modbus` and `enip` (the fieldbus masters,
  each standing alone on its framing and a runtime), all off by default.

A form is what a crate built alone gets (`cargo build -p <member>`). A build of the whole
workspace unifies features across members, so there a member asking for core without its default
features gets the plant-facing layers too when another member keeps them (ábaco's Q345, measured
after C-073 landed). That is cargo's doing, and C-073's promise is stated for the crate built alone.

**`src/schema.rs` is compiled by ábaco too, by `#[path]`** (Q345, ruled 2026-11-25, its (c)): ábaco
reads its `.abaco` stamps with core's own file, not through core as a dependency. So the file keeps
its path and its name, stands alone on serde, toml and thiserror, and uses no other module of
core. Moving it, splitting it, or giving it a `use crate::..` breaks ábaco's build; such a change
goes through the orchestrator to ábaco first.

## Module map
```
veredas_core
  types       SourceId, SeriesRef, Value, SeriesType, Sample, Quality, now_unix_ms
  source      SourceHandle, stop_flag, SourceError
  schema      stamp, check, Stamp, HeaderError: the one header reader (C-045)
  rng         Pcg32
  telegram    Endian; schema, framing, channel, capture, compose, catalog
  opc         spec, value, map, browse, worker, error; demo (feature "demo-server")
  fieldbus    modbus (feature "modbus": Client, Target, Timeouts, ModbusError);
              enip (feature "enip": Scanner, Spec, Header, LinkState, EnipError)
  state       ControllerState, State, the reasons, Conditions, MaintenanceRequest, Health;
              REASONS, PHASES, MAINTENANCE_REASONS; the writer (Display), the reader (FromStr)
```

Added for the bench (2026-11-06, `docs/maquinar-plan.md` phase 3), designed for portal's
channels as much as for the bench's: `telegram::compose` (the inverse of capture: a text
template and binary fields at offsets, the body padded to a length, every write bounded and
refused rather than truncated) and `telegram::catalog` (the second plain-text file schema after
the channel file: `veredas-catalog` version 1, each message holding one field list that both
engines use, so what a channel sends is what the same catalog reads back).

## Phases of the port
1. **Foundation and shared types.** The crate at `2.0.0-alpha.1`, `types` and `source` with
   their tests, `tests/foundation.rs` on what cargo builds. Clauses: the foundation (P-001
   promoted), shared types, worker plumbing.
2. **The telegram layer.** Every generic module with its Veredas 1 tests moved to
   `tests/telegram_*.rs`, the channel over loopback. One clause per module.
3. **The OPC UA client layer.** Every module with its Veredas 1 tests, `capabilities.rs` (the
   API assumptions pinned against a real server, run first after a version bump), the browse
   and worker end-to-end tests against the demo plant. One clause per module.
4. **File schemas with version stamps**, with P-010, when the first plain-text file format
   lands (portal's project files): the stamp, the reader that migrates or refuses. The channel
   file and the catalog file (phase 3 of the bench) are the first two, each refusing an unknown
   version; migration waits for portal.
5. **Logging**, when the first consumer needs one shape for it (Veredas 1's crates here log
   nothing; their workers report through events and status).

Each phase ends with `cargo nextest run --workspace` green bare, the laws green, and its
clauses recorded, refuted and verified. Push at the end of phase 2 and of phase 3.

## Decided in the briefing of November 2026, not built
The architect's decisions of 13 to 16 November 2026 that touch core (`docs/controlador-plan.md`);
status P-018.
- **The controller's state text has one writer and one reader, here**, with the table of reason
  numbers and words, so every screen, log and test shows the same characters
  (`docs/controlador-plan.md`, "The state"). Built 2026-11-22 (C-070, `docs/state-text.md`): the
  reader accepts exactly what the writer writes; the eight readings it took are Q100's, accepted
  as taken (2026-11-25), the review's entries 17 to 21 ruled with them.
- **The functions that drive hardware live in core**: the four fieldbus masters (PROFINET IO
  controller, EtherCAT master, EtherNet/IP scanner, Modbus TCP client), safe in controlador on
  Linux and callable by other crates for testing only (2026-11-14). The Modbus/TCP client was
  built first (2026-11-22, C-071, `fieldbus::modbus`), on the tokio-modbus crate's framing with
  every answer checked here, and met against the bench's Modbus server from the test-only tool
  `tools/mestres` (Q101, ruled 2026-11-25 with Q34). The review set this against
  Q9 and laws L4 and L5, and the architect kept it: "Core, rules amended" (2026-11-15).
- **What is amended:** core's telegram framing still holds no frame layout of its own, and a
  device spoken over a TCP telegram channel is still configuration. A standard fieldbus's frames
  are fixed by its standard and are code in the masters; no vendor's device or catalog is named
  there (`docs/questions.md` Q9, revisited). "Testing only" is a rule in prose that no law checks.
- **Aranha's PROFINET codecs move into core** when the controller needs them, and the adopted
  network libraries become core's dependencies.
- **The OPC UA server code moves from maquinar into core**, since controlador's embedded portal
  serves OPC UA. Maquinar agreed to the cut on 2026-11-22; the design is "The OPC UA server", below.
- Ruled 2026-11-25: the masters' tests against the bench live in `tools/mestres`, a test-only
  crate of repo tooling that depends on core and maquinar, law L4 standing (Q101 and Q34).
- Open: where the readers of GSDML, ESI and EDS files live (Q45); the review's entries 23 and 26
  (Q31).

## The fieldbus masters: how each is built (P-018)
In the briefing's order for core (`docs/controlador-plan.md`, "Networks and devices"); each is a
module of `fieldbus`, a clause of its own, proven against its bench device in maquinar (P-020)
from a crate law L4 allows (Q34, Q101), its frames held to the standard's own examples in core's
test.
- **Modbus/TCP client** (`fieldbus::modbus`, C-071): built, on tokio-modbus's framing, and met
  against the bench's Modbus server (C-110) from `tools/mestres` (Q101). A Modbus module's own
  timeout is checked by the hardware configuration in production too, as a warning the engineer
  may accept (Q45, ruled 2026-11-25); the check is the configuration's, and what it needs of the
  client is asked when the configuration is built.

Each master is a cargo feature of core, off by default (`modbus` the first), as the OPC demo
plant is: a component that only polls or serves (portal, supervisão) carries no fieldbus code and
no fieldbus crate, and its pinned dependency graph does not move when a master lands (supervisão's
`tests/component.rs` pins its own). Controlador switches on the masters it drives; core's own
tests switch them on through the self dev-dependency.
- **EtherNet/IP scanner** (`fieldbus::enip`, C-072): built 2026-11-22. The young crates are
  `ethernetip-core` and `ethernetip-connections` 0.1.1 (Apache-2.0), which the bench's adapter
  test already used and which hold the encapsulation, the Common Packet Format, the Connection
  Manager's requests and the Class 1 framing; rseip 0.3.1 has no implicit I/O. Core takes their
  codecs, pinned exactly, and writes the scanner itself (Q102, ruled 2026-11-25): their own scanner opens with no
  bound in time, leaves the 16-bit sequence count out of a connection's sizes, names the Sockaddr
  Info items the other way round (see "Traps"), and watches no silence. Met against maquinar's
  EtherNet/IP adapter (C-111) from `tools/mestres`. The first input is awaited the standard's
  initial watchdog, 10 s (or the timeout when longer), as the bench's adapter awaits its first
  output; `Spec::first_input` shortens it for a test.
- **PROFINET IO controller** (`fieldbus::profinet`): written here on aranha's codecs (DCP, the
  real-time frames), moved into core when the controller needs them (the cut asked of aranha
  through the orchestrator). Raw Ethernet frames. Modbus TCP is the bus on which controlador's
  runtime is first proven, and PROFINET stays the first fieldbus written; from its first clause
  the PROFINET work has a real device in the loop and captures of a real controller's traffic as
  fixtures, beside maquinar's device (Q31 entry 26, ruled 2026-11-25).
- **EtherCAT master** (`fieldbus::ethercat`): on ethercrab 0.7.1 (MIT or Apache-2.0), its
  MainDevice over a raw socket. Raw Ethernet frames; its bench device is maquinar's emulated
  SubDevice, a partial emulator and said to be one (functional, never exact in time), with real
  terminals in the loop besides (Q31 entry 27, ruled 2026-11-25). EtherNet/IP is not moved to a
  later release ("later" not chosen, the same entry).

Raw frames in tests need no privilege from the sessions the desktop app starts: an unprivileged
user and network namespace (`unshare -Urn`) holds a veth pair whose ends take raw sockets, the
host's links untouched (measured 2026-11-22; aranha's note on the shared folder has the two
traps: a multi-threaded process cannot enter a new user namespace, so the test re-runs itself in
a child, and the child needs its uid map). Outside the app (a plain terminal, CI) the host's
AppArmor rule refuses it unless `kernel.apparmor_restrict_unprivileged_userns` is 0. A real
network card needs `CAP_NET_RAW` on the controller's executable, an install step.

## The OPC UA server (P-018): the cut from maquinar
Controlador's embedded portal serves OPC UA, so the server leaves the bench for core as
`opc::server`, behind a feature of its own, `opc-server` (with `net` and async-opcua's server; the
demo plant then stands on it). Maquinar agreed to the cut on 2026-11-22 with five corrections,
taken whole (`maquinar-answers-2026-11-22.md` on the shared notes folder). Into core:
- **The start:** on a thread and current-thread runtime of its own, bound first and advertising the
  port it got (port 0 included). The application URI and name, the data namespace and the folder
  (node id and browse name) are the caller's, and core names no component. A namespace equal to the
  application URI is refused by name: the diagnostics node manager would claim its index, and every
  read would come back `BadNodeIdUnknown`. Anonymous access is always accepted, plus optionally one
  user by name and password, the password hidden in `Debug`. `max_sessions` and
  `max_nodes_per_read` are optional knobs.
- **A variable:** a string id, a browse name, a type (int as Int32, float as Double, bool, text),
  an access (read, write-only, read-write) and an initial value; `parse_value`, `render_value`, and
  the check of a value's wire type.
- **The write guard:** a client's write of the wrong wire type is answered `BadTypeMismatch`, and
  one with an index range `BadIndexRangeInvalid`. A right one lands as soon as the address space's
  lock is free, and is handed to the caller as `(id, value)` in the order the writes landed. A set
  through core's handle is never handed as a client write, so the bench's reactions tell their own
  sets from a client's. The guard counts the mismatches (a client's refused, a caller's set
  dropped) and keeps the last; the bench's status composes them.
- **The handle:** set, get and snapshot from the caller's side (write-only variables included);
  the client writes; the status; a handle on the server's runtime, so a caller's loop runs on the
  server's one thread and keeps the bench's timing margins; stop; and drop as the teardown that
  closes every connection and frees the port.

Stays in maquinar, on core's server: `OpcConfig` and its reading from the bench file, the draws,
ramps and reactions and their loop, `stall`, `TICK` and the timer guard, and the relay's start.
Once core's module is on `main`, maquinar switches `opc.rs` onto it in one commit of its own.
C-026's test file stays unchanged; its run is recorded again and refuted on the switch. Core's
own clause tests the server against core's OPC UA client.

## Traps
Recorded as the port meets them.

- **`ethernetip-core` swaps the Sockaddr Info items.** Its `SOCKADDR_INFO_T_TO_O` is 0x8000 and
  carries the originator's own address in a Forward_Open's request; the standard, the bench's
  adapter and Wireshark have 0x8001 for where T→O data goes (the scanner's address, in the request)
  and 0x8000 for where O→T data goes (the adapter's, in the reply). `fieldbus::enip` names the
  items by the standard's numbers and never uses the crate's constants; the dissectors check it.
- **A test that counts the wire against the clock is a timed test.** Under the whole suite (1 300
  tests at once) a 20 ms exchange came round less often than its interval and a 1 s bound was
  passed by 0.2 s. A property that order or a count can judge is judged so (a blocking task still
  running when the call returned, C-021); one that is a rate or a timeout (the masters' intervals,
  bounds and windows) runs with the machine to itself, named in the runner's file
  (`.config/nextest.toml`): `enip_scanner`, `modbus_client` and `tools/mestres`.
- **A connection's size counts the sequence count.** Each direction of a Class 1 Forward_Open
  carries its data plus the 2-byte sequence count plus the 4-byte run/idle header where the
  direction has one; the crate's own scanner leaves the count out and an adapter refuses it
  (0x0127). The 16-bit Forward_Open carries 511 bytes at most.

- **tokio-modbus's typed reads trust the device.** Its `Reader` methods only `debug_assert!` that
  an answer holds the quantity asked: a short answer panics a debug build and comes back short in
  a release one, and a write's echo is checked the same way. `fieldbus::modbus` goes through the
  crate's `call` and checks every answer itself (C-071).
- **A tokio timer is made inside its runtime.** `runtime.block_on(tokio::time::timeout(..))`
  builds the timer outside the runtime and panics ("no reactor running"); wrap it in an `async`
  block.

- **The crate is not formatted by rustfmt as a whole.** `cargo fmt -p veredas-core --check`
  differs on files written before the Linux machine; format a new file alone
  (`rustfmt --edition 2024 <file>`), so a commit carries no reformatting of files it did not
  change. `rustfmt` on a file that declares modules (`lib.rs`, a `mod.rs`) formats every module
  it declares too: edit those by hand, or restore the others after.

- **A full disk corrupts the incremental cache.** On 2026-11-05 the C: drive filled mid-build
  ("no space on device"); the next build failed with "cached cgu ... should have an object file".
  `cargo clean -p veredas-core` and removing its `target/debug/incremental/` entries fixed it. The
  OPC layer's dependencies are large: check free space before phase 3.
- **A rule about what cargo builds is checked by asking cargo.** Re-implementing its target rules
  missed edition 2021's `crate_type` and `autolib = false` (C-007's first refute round);
  `cargo metadata --no-deps` sees them all and builds nothing.
- **Where a feature puts an item is asked of rustdoc, not read in the text.** A reading of the
  source misses a gate with a comment inside, a line break, a macro, an `include!`d file or a
  `#[path]` module (C-073, round 5). rustdoc's pages and their `id` anchors, built per form with
  `--document-private-items`, show every item wherever its gate is written. Two things on a page
  are not core's: the blanket impls (a master's framing brings tracing's `Instrument` onto every
  type) and the `notable-traits-data` tooltip script; leave both out of the comparison. Names
  are not enough: compare what each anchor renders (a field's type, a `pub(crate)`), and check
  the view against the build with a consumer per form, since rustdoc sets `cfg(doc)` (round 6).
- **Rust is read by a lexer that is Rust's, and a profile is measured, not argued.** C-073's gate
  audit read core's sources with a lexer of the test's own, and rounds 15 to 18 each beat it with
  another spelling: a gate split over two lines, a raw byte string or a `'\''` it misread, a
  built-in macro imported under another name, a `#[path]` inside a `cfg_attr`. The sources are now
  read as tokens by proc-macro2, as supervisão's own test reads its sources, and every rule is a
  rule on tokens: `feature` stands inside an attribute only, and the names of `include!`, `env!`
  and their kin appear nowhere. The audit once also had to argue that a release build relates the
  forms as the development build does. Now the probe builds and runs every form in both profiles,
  each consumer carrying the workspace's profiles, and rustc's dep-info says which files it read.
  The whole test took 6.4 minutes on 2026-11-25 with the probe's build folder emptied first.
  A consumer built alone stands in for a member's build but is not that build: core's identity
  hash (a `TypeId` reads it) and its files' paths differ there. Code that reads them can tell
  the test from a member, and no test measures that; the clause names it as a limit (round 19).
  rustdoc reading a crate's source has `debug_assertions` on whatever the profile:
  `cargo doc --profile release` and `RUSTDOCFLAGS="-C debug-assertions=off"` change nothing. A
  facade's pages, rendered from the crate's metadata compiled in that profile, do follow it, so
  the release view of what a form holds is the facade's (round 21).
- **A copy of the tree never builds into the worktree's target.** Cargo keys a workspace
  member's artifacts by its path relative to the workspace, and judges a path package fresh by
  file times alone. A critic's copy built with `CARGO_TARGET_DIR` set to the worktree's target
  overwrites the worktree's core with the copy's mutant. The worktree's older files then look
  fresh, so its next run links the mutant and only a replayed warning shows it (C-073, round 6).
  Give a copy a target of its own (a fresh copy of the seed is fine: the copy's files are newer
  than the seed's artifacts). After such a mix-up, reset the target and `touch` the crate's
  files.
- **LNK1140 ("limit exceeded for program database") on one test binary** after a link was cut
  short: the half-written PDB is reused. Delete that test's `.pdb` under `target/debug/deps` and
  link again. Each OPC test binary carries a PDB of about 200 MB, and `-p veredas-core` and
  `--workspace` builds hash apart, so phase 3 took the target from 4.6 to about 10 GB.
- **async-opcua 0.19 cannot encode or decode an array of more than 1 000 elements.** A Read of
  more nodes fails at the server's decoder, as `BadDecodingError: Request returned ServiceFault`,
  before any per-call cap is checked. That is the text the field's 3,048-tag source showed. A
  Read past a lower cap fails with the server's own status (`BadTooManyOperations: ...`). The
  demo's cap therefore sits at 800: a cap of 1 000 made the first refute round's pin test the
  decoder, not the cap. Reads are chunked at 500. The client decodes no array over 1 000 either:
  a browse answer of 1 001 references kills the connection, so the browser asks for 1 000.
- **The demo stack is lax.** It browses an unknown node as Good and empty (only the null node
  `i=0` is refused), and takes a String written into an Int32 tag. A test that needs a refusal
  browses `i=0`, constructs the result, or writes a node the plant holds read-only.
- **A silent link is a relay, not a mock.** `demo::Relay` listens on `127.0.0.2` at the plant's
  port. The client's endpoint match compares port and path, not host, so the session goes
  through it. Frozen, it holds what it reads, which is how a stalled link delivers late.
- **A test of DNS timing bites only where a failed lookup is slow.** About 1.2 s on Windows; a
  Linux resolver may answer at once. So the property is pinned apart from DNS: a 3 s blocking task
  left on `run_on_own_runtime` must not hold its thread.
- **A read callback blocks a server thread.** The demo plant's slow tag holds one thread for
  `SLOW_READ`, so the tests that read it run the plant on a multi-threaded runtime.
- **A header reader that materializes the body judges it.** The first fix of the header reader's
  order (C-045, round 1) parsed the whole text into a `toml::Table`: every body value became an
  `i64`, and the catalog's reference carries an offset of 2^64-1 that the stamp then refused for
  a body it never reads. Deserialize a struct of the two keys as `toml::Value`s and let serde skip
  the rest unread; judge each key's type yourself, the format whole before the version.
