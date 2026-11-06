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
  `docs/channel-schema.md`), with no layout of its own (Q9); and the capture rules
  engine that reads message fields by configuration. No device's catalog, state words or
  decision logic: those are the rules a portal channel is configured with (Q8).
- **Seeded randomness:** the deterministic PCG32 (`rng`), for weighted draws and fixtures.
- **The OPC UA client layer:** the compiled source shape (`PollSpec`), value conversion, the
  mapping of a poll to rows, the address-space browser, the poll worker with its gated writes.
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
- typed errors: `OpcError` (naming the service `Call`), `NodeIdError`, `WriteRefused`, `DemoError`;
- a write or read asked of a worker with no session is answered at once (`NotConnected`).
  Veredas 1 queued it and fired a command pulse whenever the plant came back;
- the worker checks the master switch again just before the wire. Veredas 1 checked only in the
  handle, so a write already queued fired after a disarm;
- every connection attempt is bounded (`CONNECT_TIMEOUT`, 10 s) and raced against stop, in the
  worker and the browser. Veredas 1 could hang on a peer that accepts TCP and never answers.
  Dropping the browser is prompt, and an abandoned session's event loop is aborted;
- a zero poll interval is refused at spawn (Veredas 1's worker thread panicked in tokio);
- bits are read at the wire word's own width. Veredas 1 sign-extended, so bit 20 of an Int16 −1
  read 1, and gave a UInt64 above `i64::MAX` no bits. A 64-bit field with its top bit set is
  refused rather than wrapped;
- a bad read of a word polled only for its bits is reported (Veredas 1 reported nothing), and a
  good status with no value is reported as "no value";
- the one-line renderings are one line (a control character reads as a space), and the browser's
  value brief is cut by characters, not bytes;
- the demo plant caps a Read below its bulk folder, so the worker's chunking is proven rather than
  assumed. It also has a slow tag, holds its namespace index, and reports why it did not start;
- comments that named callers, vendors, products or field sites now say what the code means on
  the wire or in the plant.

## Module map
```
veredas_core
  types       SourceId, SeriesRef, Value, SeriesType, Sample, Quality, now_unix_ms
  source      SourceHandle, stop_flag, SourceError
  rng         Pcg32
  telegram    Endian; schema, framing, channel, capture
  opc         spec, value, map, browse, worker, error; demo (feature "demo-server")
```

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
   lands (portal's project files): the stamp, the reader that migrates or refuses.
5. **Logging**, when the first consumer needs one shape for it (Veredas 1's crates here log
   nothing; their workers report through events and status).

Each phase ends with `cargo nextest run --workspace` green bare, the laws green, and its
clauses recorded, refuted and verified. Push at the end of phase 2 and of phase 3.

## Traps
Recorded as the port meets them.

- **A full disk corrupts the incremental cache.** On 2026-11-05 the C: drive filled mid-build
  ("no space on device"); the next build failed with "cached cgu ... should have an object file".
  `cargo clean -p veredas-core` and removing its `target/debug/incremental/` entries fixed it. The
  OPC layer's dependencies are large: check free space before phase 3.
- **A rule about what cargo builds is checked by asking cargo.** Re-implementing its target rules
  missed edition 2021's `crate_type` and `autolib = false` (C-007's first refute round);
  `cargo metadata --no-deps` sees them all and builds nothing.
- **LNK1140 ("limit exceeded for program database") on one test binary** after a link was cut
  short: the half-written PDB is reused. Delete that test's `.pdb` under `target/debug/deps` and
  link again. Each OPC test binary carries a PDB of about 200 MB, and `-p veredas-core` and
  `--workspace` builds hash apart, so phase 3 took the target from 4.6 to about 10 GB.
- **async-opcua 0.19 reports any service fault as `BadDecodingError: Request returned
  ServiceFault`.** The server's own status (`BadTooManyOperations` for a Read past its cap) is
  lost. In a failure text, it means the server refused the whole call.
- **The demo stack is lax.** It browses an unknown node as Good and empty, and takes a String
  written into an Int32 tag. A test that needs a refusal constructs the result, or writes a node
  the plant holds read-only.
- **A read callback blocks a server thread.** The demo plant's slow tag holds one thread for
  `SLOW_READ`, so the tests that read it run the plant on a multi-threaded runtime.
