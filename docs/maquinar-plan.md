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
  within a range on an interval, or a ramp stepping through a range and wrapping, the plant motion
  supervisão's layout shows: a place along a loop, a level filling) served on a real endpoint, anonymous or by user and password;
  values set and read on the server side (write-only tags included); **reactions**, a write of a
  given value into a tag answered by a timed sequence of values, so a consumer's pulse and
  handshake have a plant to land on; a relay (core's `opc::demo::Relay`) for a link that goes
  silent; the stack's limits (nodes per read, sessions) as knobs.
- **Telegram channels** (`channel`): a channel in either role, framed and held by core's channel
  file (`docs/channel-schema.md`), speaking a **catalog** of messages (core's catalog file: a
  name, the id its frame carries, the body's fields) built from named fields and read back into them;
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
- **A Modbus server** (`modbus`, P-020's first device): a device's four tables (coils, discrete
  inputs, input and holding registers) sized by the bench file, named points in them (a bool, or a
  `u16`, `i16`, `u32`, `i32` or `f32` over one or two registers in either word order), every
  public function code of the data model answered on a real TCP socket with the protocol's
  exceptions, a unit it does not play answered as a gateway answers; the plant side by point and
  by address; the knobs a client's failure paths need (every answer late, none at all, every
  connection dropped, a connection limit).
- **An EtherNet/IP adapter** (`ethernetip`, P-020's second device): an identity, assemblies by
  instance, the Class 1 connections it accepts, points in the assemblies' bytes; the encapsulation's
  commands on TCP (ListIdentity on UDP too), the Identity and Assembly objects, the Connection
  Manager's Forward_Open and Forward_Close (directly or inside an Unconnected_Send) with each
  fault's status, Class 1 cyclic I/O over UDP produced at the RPI and consumed, a silent
  originator's connection closed at its timeout; the plant side by point or as bytes; the knobs a
  scanner's failure paths need (production stalled, the sessions silent, the device restarted).
- **A PROFINET device** (`profinet`, P-020's third device, its first slice): discovery and naming
  on a network interface, raw frames through aranha's link and codecs (Linux): a DCP Hello at
  start, an Identify it matches (all, its station name, its alias) answered within the request's
  response delay, a Set of its name, its IP suite or the identification signal applied and each
  block answered with its three-byte control response, a Get answered, LLDP on its port with the
  PROFINET fields; the plant side names it, flags an address conflict, mutes it. The AR and the
  cyclic data come with core's IO controller; the nameplate and port records after.
- **An emulated EtherCAT segment** (`ethercat`, P-020's fourth device): SubDevices answering a
  master's frames on a network interface (Linux, aranha's raw link), each an ESC whose memory
  answers every command with the working counter the standard gives, its AL state machine, its SII
  EEPROM read through the registers, its distributed clock latched and offset, its FMMUs mapping
  the logical commands; the frame sent back from the segment's far end; the plant side reads the
  outputs, sets the inputs, faults a device, silences the segment.
- **The bench file** (`bench`): one versioned TOML file for the whole plant (emitters, the OPC
  and Modbus servers, the channels and the files each points at), started as a library (`Bench::start`,
  for portal's tests and later comissionador) or from the command line (`veredas-maquinar
  <bench.toml>`, until stopped).
- **Later phases**, named, no code now: the rest of P-020 (phase 6 below: the EtherNet/IP
  adapter, the PROFINET device on aranha's codecs, the EtherCAT device, `fisica`, `reflexo`); a
  PKI folder, when the OPC server first needs a secured endpoint. A desktop
  UI is not planned: the architect decided (`docs/questions.md` Q12, 2026-11-07) that the bench
  is the library and its executable.

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
- **Test-only and light**: no DuckDB, never portal's store or desktop, no UI crate (Q12,
  decided: library only).
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
| `app.rs`, `ui_*.rs` (an egui desktop) | not carried | Q12, decided 2026-11-07: the bench is a library and a command-line executable |
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
  modbus      the Modbus/TCP server: four tables, points, the function codes, the knobs
  ethernetip  the EtherNet/IP adapter: its wire (encapsulation, CPF, EPATH, Forward_Open, the Class 1
              frame), its configuration, the objects, the sessions, Class 1 I/O, the knobs
  profinet    the PROFINET device: DCP (Hello, Identify, Set, Get) and LLDP on aranha's codecs and
              raw link, the knobs (Linux)
  ethercat    the emulated EtherCAT segment: the ESC core (registers, commands, AL, SII, DC, FMMUs)
              and the SII image, pure; the segment on a raw link, the knobs (Linux)
  fisica      the scene file (veredas-cena), the World on Rapier stepped at a fixed step, its
              runner in real time and its handle, hooks before each step (C-114)
  reflexo     ready-made models (drive, conveyor, sensor, actuator) between a device's named
              points and a scene, stepped on the scene's thread (C-115)
  bench       the bench file (schema), Bench::start, every part's status
  main.rs     veredas-maquinar <bench.toml> [--check] | --version; stops on stdin's end, `stop`,
              SIGTERM or SIGINT
veredas_core (added for the bench, kept for portal)
  telegram::compose   a body built from named fields: binary fields at offsets, a text template
  telegram::catalog   the catalog file: messages by name, the id their frame carries, body rules,
                      what a message reads, where the ids ride in a channel file
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
   run as a process with a bench file; Q12 asked and decided: library only.

6. **P-020, fixture first** (the briefing of November 2026, below), each device before or alongside
   the master in core that meets it, each a clause of its own proven on the real wire: the Modbus
   server (C-110), then the EtherNet/IP adapter, the PROFINET device on aranha's codecs, the
   EtherCAT device on a raw network card, then `fisica` and `reflexo`'s ready-made models.

Each phase ends with `cargo nextest run --workspace` green bare, `cargo test -p leis` green, and
its clauses recorded, refuted and verified. Push at the end of each phase.

## P-020: the bench devices, as they land
- **The Modbus server** (C-110, 2026-11-22). Written here, not taken from a crate: the protocol is
  small, and what a bench needs of it (the exceptions in the protocol's order, a frame of another
  protocol dropped, a length no frame has closing the stream, answers held or withheld, every
  connection dropped) is the server's own behaviour, which a library would hide. Its proof is
  the protocol specification's own worked examples byte for byte from a raw client, and a client
  library the bench did not write (`tokio-modbus`, a dev-dependency) reading and writing every
  table. No draws, ramps or reactions of its own, unlike the OPC UA server: what makes a device
  move is `reflexo`'s models, driving any bench device's points through its handle, so each
  fieldbus device carries its wire and nothing more. Core's Modbus TCP client meets it: law L4
  keeps core's own tests from starting it (`docs/questions.md` Q34), so a test starts it as the
  library (`modbus::start`) from a crate the map lets use maquinar, or as the executable on a
  bench file with `port = 0`, reading the bound address from the status line it prints at start
  (`modbus <name>: 127.0.0.1:<port> ...`).
- **The EtherNet/IP adapter** (C-111, 2026-11-22). The bench's own code again, for the reason the
  Modbus server is: what a bench must show (each Forward_Open fault's extended status, a stray
  frame, a silent originator's timeout, production stalled) is the adapter's own behaviour. The
  object model is the least a scanner meets: Identity, Assembly, the Connection Manager; Class 1
  cyclic only, point to point both ways (a multicast T→O is refused 0x0108, and is the first thing
  to add when a consumer needs it), no connected explicit messaging (SendUnitData answered 0x0001).
  Its proof has three readers the bench did not write: `ethernetip-connections` (its Forward_Open
  encoder, its reply decoder, its Class 1 frame codec), `ethernetip-core` (its encapsulation and
  common packet format), and Wireshark's dissectors (tshark, in the home folder as aranha's tests
  find it). The young crate is not taken whole: its scanner states a connection's size without the
  16-bit sequence count, and its socket-address item codes are swapped (its originator endpoint
  goes out as 0x8000, the protocol's O→T item); the test drives its codecs with the protocol's sizes
  and codes, and shows its own scanner refused 0x0127 for the sizes. Core's EtherNet/IP scanner, if
  it forks that crate, has both to fix.
  Its first refute round set four borders a scanner's author meets: the adapter routes one level
  (an Unconnected_Send nested in another is refused 0x08, else a thousand levels overflowed the
  thread's stack and killed the bench); produced frames go to the originator's own host, at the
  port its T→O item gives (the item's address is not obeyed, as OpENer does); a frame whose 32-bit
  sequence is not newer than the last taken is dropped as stale; and a ListIdentity broadcast is
  heard only by an adapter bound to `0.0.0.0` (Linux hands a broadcast to the any-address alone),
  whose item then names the address its reply leaves from.
- **The PROFINET device's first slice** (C-112, 2026-11-22): discovery and naming, which aranha's
  scanner and Oficina's engineering tool meet first. It speaks through aranha's codecs and raw
  link (`docs/questions.md` Q5, the map's row: the scanner and the bench cannot drift apart on the
  wire), so C-024's rule that the bench's graph holds no member but core now admits aranha. The
  device and the test's controller share those codecs, so the second reading is Wireshark's
  dissectors over every frame the device sends. aranha's builders refuse a bad station name, so a
  test of the device's refusal writes that one request's bytes itself. Its tests run in aranha's
  `netns::isolated` private network, a veth pair per test. Next: the AR (the DCE/RPC Connect, the
  parameter writes, PrmEnd and ApplicationReady) and the cyclic RT frames on aranha's C-135, with
  core's IO controller; then I&M and the port's neighbour data by the connectionless read.
  Next, before the AR (2026-11-22): the record read aranha's phase 6 needs, fixture first. The
  device answers on its raw link as a real one does, with no host address of its own: ARP for
  the IP suite it holds (from the file or a DCP Set), and connectionless DCE/RPC on UDP 34964
  with aranha's codecs (`profinet::rpc`, `profinet::record`): an implicit read of I&M0 (0xAFF0,
  from the file's identity) and of the port's real data (0x802A at subslot 0x8001: its link, and
  the neighbour its own LLDP reception heard), any other index refused with the PNIO status the
  standard gives; tshark and aranha's own reader as the second readings.
  Landed as C-116 on 2026-11-23. Its test's client is the kernel's own IP stack on the far end of
  the veth pair, so ARP, IPv4 and the UDP checksum are judged by code the bench did not write.
  Trap: a veth leaves a UDP checksum to an offload that never happens, so a raw socket on the
  other end reads only its partial sum and the device (which checks it, as a device does) drops
  the datagram; the test turns transmit offload off on the client's end (`ethtool -K a0 tx off`),
  as a real card writes the whole checksum on the wire.
- **The emulated EtherCAT segment** (C-113, 2026-11-22). Review entry 27 said it plainly: an
  EtherCAT device handles frames in a chip as they pass, and an emulation on an ordinary card is a
  slave controller written in software, functional, never exact in time; its clause says what it
  covers. The ESC is its registers and 8 KB of process RAM, answering every register as stored, with behaviour only where
  the standard gives one: the read-only registers kept, the AL state machine (a transition it
  refuses flagged with code 0x0011 until acknowledged), the SII interface (a read loads 8 bytes at
  once, never busy), the distributed clock (a write to port 0's receive time latches every port's;
  the system time read live, its offset the master's; an FRMW of it sets the difference), the
  FMMUs for the logical commands. The EEPROM image is built from the bench file: the identity, one
  string (the name, given as both order number and name, since a master names a device by its
  order number), General, FMMU and SyncManager categories, the PDOs; no mailbox, so a master maps
  the process data from the EEPROM. The frame goes back on the wire it came from with the source's
  locally administered bit set, as a first device sets it: ethercrab drops a frame still carrying
  its own source. Proof: ethercrab 0.7.1 (the crate core's master stands on) finds the devices,
  reads them, addresses them, takes them to OP and exchanges process data; hand-built frames pin
  each command's data and counter; tshark reads the frame as sent and as returned.
  Its first refute round (2026-11-22) corrected what a master other than ethercrab would meet: an
  LWR counts 1 a device (2 only for an LRW's write); FMMUs map bit by bit, as the features
  register says they do, so masters that pack 1-bit terminals into shared bytes are served; the
  system time difference is sign and magnitude, as an ESC states it; the registers EtherCAT only
  reads are kept (the alias, each SyncManager's status, the receive times); the alias addresses a
  device once the master sets DL control's bit 24; a fault never raises a state; a PDO holds 255
  entries a way, the EEPROM counting them in a byte. The ESC core is pure, so its clock's values
  are pinned to the nanosecond by calling it directly.
- **Raw frames without a privilege.** The EtherCAT and PROFINET devices answer raw Ethernet
  frames, which need `CAP_NET_RAW`, and the machines here have no sudo. Checked on the Linux
  machine (2026-11-22): `unshare -Urn` (an unprivileged user namespace with its own network
  namespace) gives a process root inside it; there it creates a veth pair, brings it up and sends
  an EtherCAT frame (EtherType 88A4) from one end to an `AF_PACKET` socket on the other. So those
  tests run the device and the master inside such a namespace, on a veth pair, and the suite
  needs no privilege and no setting on the host; a real network card is the commissioning case,
  not the suite's.
- **`fisica`, as designed before its code** (2026-11-22). The bodies of a plant's scene and their
  motion, headless, at a fixed step, on Rapier (`docs/questions.md` Q160, taken meanwhile), which
  only `fisica`'s own module names. A scene is a file of its own, `veredas-cena` version 1, read by
  the bench file's `[[fisica]]` part, so that Oficina's view (review entry 31, option a) draws what
  the bench moves from the same file. Metres, kilograms, seconds; z up, x along a line; a pose is
  a centre and three angles in degrees (yaw about z, then pitch about the new y, then roll about
  the new x). Its parts, boxes all in this first slice: fixed shapes (a floor, guides, a chute's
  sides), belts (a fixed box whose top surface moves along its own x at a speed the plant side
  sets: the solver's contacts with it given that tangent velocity, its sides left still), bodies
  (cartons, parts: dynamic, a mass, a friction), movers (kinematic boxes the plant side moves, a
  pusher's blade or a lift's table, which push bodies and are not pushed) and sensors (a box or a
  ball that reports the bodies inside it, the step each entered and left: a light barrier's beam,
  a proximity switch's field). A `World` is pure and stepped by its caller, so the tests pin the
  physics without sleeping; `fisica::start` runs one on its own thread in real time, with a handle
  that reads every pose and sensor, sets belts' speeds and movers' poses, adds and removes bodies.
  The same scene and the same inputs give the same run, bit for bit (Rapier's
  `enhanced-determinism`). Checked first with Rapier 0.36.1 (spike, 2026-11-22): a 0.3 m carton on
  a belt at 0.5 m/s reached the belt's speed within 0.05 s and broke a 1 cm beam at 2.0 m from
  x = 1.849 to x = 2.157 (its edges, to the step), and two runs agreed bit for bit; the tangent
  velocity's sign follows which collider of the pair the belt is. `reflexo`'s models (a drive, a
  sensor, a conveyor, an actuator) bind these to a bench device's points through their handles.
  Landed as C-114 the same day. What its test taught: a sensor sees a body one step after the
  first step whose poses overlap, since Rapier detects contacts before it moves the bodies (the
  clause states it; 4 ms at the default step, a real sensor's own delay is longer); a part's
  `size` is along its own axes, so a belt yawed 90° is still long in its own x.
- **`reflexo`'s first half, as designed before its code** (2026-11-22). Ready-made models that make
  a bench device behave as the machine behind it: a model reads what a controller wrote to the
  device (its outputs) and writes what the controller reads (its inputs), and moves a `fisica`
  scene's parts. A model names a device's point as `<part>.<point>`, of the bench's Modbus servers
  and EtherNet/IP adapters (their points are named); a bool point carries 0 or 1, a number is
  converted to the point's type (an integer rounded and held to its range). The models step on
  the scene's thread, before each step and at its fixed step, so a model and the bodies it moves
  share one clock and a run repeats. Four kinds:
  - a **drive**: `run` and an optional `reverse` read, a `setpoint` read in the drive's own unit;
    its speed ramps toward the setpoint (0 when not running) at its `accel` and `decel` (units a
    second); it writes its `speed`, `running` (speed not 0) and `at_speed` (within its
    `tolerance` of a running setpoint) where the file names those points;
  - a **conveyor**: a drive whose speed moves a belt of the scene, `metres_per_unit` of surface
    speed per unit of the drive's;
  - a **sensor**: a scene sensor written to a bool point, occupied as 1 (0 when `normally_closed`),
    after its `on_delay_ms` and `off_delay_ms`;
  - an **actuator**: a cylinder; its `extend` read, a mover travelling between its `retracted` and
    `extended` poses in `stroke_ms`, its end switches written when it is at either end.
  The bench file gains `[[fisica]]` (a scene file run in real time) and `[[reflexo]]` (a scene
  part's models). The second half, ORCL function blocks, waits for controlador's interpreter;
  EtherCAT devices, whose process data has no named points, are bound by byte and bit then.
  Landed as C-115 the same day, without a `step_ms` key (models without a scene step every 4 ms:
  a key that would be refused beside `scene` waits for a consumer that needs another step). Its
  test found a ramp summed in floating point landing a step late (0.4 added 125 times is
  49.9999999999999). Its first refute round (2026-11-23) found the same drift in long ramps and in
  the actuator's stroke: a ramp is now worked out from a count of steps per stretch, an actuator's
  travel and a sensor's delay counted in whole nanoseconds, so each lands at the first step at or
  after its time. It also found a scene running before its models were hooked in while another
  scene was built: the bench builds every scene paused (`fisica::paused`), hooks the models in,
  then runs them.

## Decided in the briefing of November 2026, not built
The architect's decisions of 13 to 16 November 2026 that touch the bench
(`docs/controlador-plan.md`, `docs/oficina-plan.md`); status P-020.
- **Bench devices for all four fieldbuses**, in Rust, inside maquinar: a PROFINET device, an
  emulated EtherCAT device answering on a raw network card (built: C-113), an EtherNet/IP adapter
  (built: C-111),
  a Modbus server (built: C-110).
- **Two extensions inside maquinar**, called by their bare names as `acerto` is in its host:
  `fisica` (the bodies, their motion, which sensor an object reached) and `reflexo` (what makes
  simulated devices behave: ready-made models for drive, sensor, conveyor and actuator, and beyond
  them ORCL function blocks run by the same interpreter as controlador). Both compute headless, at
  a fixed step; Oficina's 3D view only draws the positions it is sent.
- **Two new dependencies**, allowed by the architect: the interpreter and a physics engine.
- **The codecs it shares with aranha move into core** with the fieldbus masters (Q5, revisited).
- **Q12 stands:** the bench has no interface of its own. Its surfaces are Oficina's.
- Open, all under Q31: the review's entries 25 (the two extensions can only be modules here, since
  laws L1 and L2 refuse them as crates), 27 (the EtherCAT bench device; the EtherNet/IP fork), 31
  (two scene models) and 36 (a bench device on a real network passing for a real one).

## Traps
Recorded as the port meets them.

- **Check the disk before the first build.** Core's OPC phase filled C: twice (`docs/core-plan.md`,
  "Traps"); each OPC test binary carries a PDB of about 200 MB, and this crate adds a server
  and a client per test file. 52 GB were free when this plan was written, the target at 6 GB;
  42 GB after phase 3, the target at 12.6 GB (the debug PDBs), without a clean.
- **The OPC namespace must not be the application URI** (inherited from Veredas 1's `opcsim`
  and core's demo): the diagnostics node manager registers first and takes a colliding index,
  and every read then answers `BadNodeIdUnknown` with the nodes plainly present.
- **Bind first, advertise the bound port**: with port 0 the server's advertised endpoint must
  carry the port the OS gave, or a client's endpoint match finds nothing.
- **The stack takes any scalar into any tag.** async-opcua's write validation refuses arrays and
  byte strings only; an Int32 lands in a float tag and `get` returns it. A write callback is
  called *instead of* the write, under the address-space lock, so it cannot store a good value
  itself: the bench answers `BadTypeMismatch` in the callback and hands a right value to a task
  on the server's runtime, which lands it the moment the lock is free (C-026, rounds 2 and 3).
- **A due time taken from the tick's clock drifts by a tick.** `Instant::now()` after a tick is
  a little past it, so the next due time is just past a boundary and waits one more tick; every
  interval stretched by 20 ms. Schedule from the last due instant and skip what a stall missed
  (C-026, round 4). The tick itself lands at the host's timer quantum, which the bench holds at a
  millisecond while any of its parts runs (`timer::FineTimer`: on Windows the process opts out
  of power throttling's timer coarsening, then `timeBeginPeriod(1)`, paired with its release;
  `docs/questions.md` Q13, decided 2026-11-09): 20 ms on both platforms, where Windows ticked
  about every 31 ms before. The request alone still ticked every 30 ms on Windows 11, which
  ignores it from a process without a foreground window; the opt-out is what makes it count.
- **A timing test measures the host, so it runs alone.** With the timer held fine, the 20 ms
  tag still drew 65 times in 2 s under the whole suite's sixteen parallel processes (a 31 ms
  tick, the scheduler's doing, not the timer's), and 95 alone or beside the bench's own tests.
  `.config/nextest.toml` gives `draws_come_at_their_interval` the machine to itself
  (`threads-required = 'num-cpus'`); law L7 leaves that file unstamped, as a tool's own. CI on
  Linux ran `cargo test --workspace` then, which that file does not reach, and a shared runner drew
  81 in 2 s (the run for c9f39cc; CI runs nextest since 2026-11-09, but a shared runner's load is
  not the suite's alone, so the bound stays): the tick's `MissedTickBehavior::Delay` turns every late
  wake-up into a longer period, so the 20 ms tag's count is a bound against the quantum, not a
  pace: at least 70, where a 31 ms tick (64) cannot reach, at most 102, where two draws a tick
  would; the 100 ms tag's 19 to 21 stays exact, a draw being due from its due time, not from
  the tick that serves it.
- **Which of the bench's tests go behind the machine's lock** (the architect's rule as the
  orchestrator relayed it, 2026-11-22: the lock is for a test that measures time or drives a
  browser). Behind `flock /tmp/veredas-suite.lock`, built first: a test that measures the host's
  clock, a rate, a tick or an upper bound on how soon something happens. Free: a test that reads a
  file, checks a refusal, or talks over a socket with a deadline that only bounds a wait, and a
  test whose only time check is a lower bound (a delay can only grow under load; not so against a
  runner that takes its late steps at once, whose own steps are then the measure). The timed ones,
  as a filter a run (a critic's mutant run too) leaves out to go free:
  `test(draws_come_at_their_interval) | test(ramps_come_at_their_interval_counted_apart_from_draws)
  | test(a_stall_skips_the_draws_and_steps_it_missed) | test(a_draw_or_a_step_is_next_due_an_interval_after_it_was_due)
  | test(a_reaction_answers_a_write_with_a_timed_sequence) | test(a_stop_is_prompt_whatever_the_interval)
  | test(the_machine_announces_awaits_the_answer_and_reports_the_outcome_tick_by_tick)
  | test(a_destination_s_state_forces_the_outcome_and_the_bench_s_knobs_set_it)
  | test(faults_mark_their_share_of_items_and_the_rate_is_the_interval_s_whatever_the_tick)
  | test(the_machine_and_the_unit_converse_over_a_real_socket_and_the_knobs_turn_live)
  | binary(pressure) | test(the_knobs_delay_silence_and_drop_every_connection)
  | test(a_class_1_connection_produces_at_its_rpi_and_consumes_what_its_originator_runs)
  | test(a_connection_whose_originator_falls_silent_is_closed_at_its_timeout)
  | test(the_timeout_counts_the_consumed_rpi_and_the_multiplier_and_the_first_frame_waits_ten_seconds)
  | test(a_connection_s_first_frame_comes_at_once_whatever_the_others_wait_for)
  | test(the_runner_steps_in_real_time_and_its_handle_reads_and_moves_the_scene)
  | test(an_identify_is_answered_within_its_response_delay)
  | test(lldp_names_the_station_and_the_port_and_says_when_it_leaves)
  | test(round_1::the_delay_follows_the_factor) | test(round_1::lldp_comes_on_its_interval)
  | test(round_2::a_multiplier_of_two_is_sixteen_rpis) | test(round_2::the_first_frame_comes_at_once_at_a_long_rpi)
  | test(round_2::stale_frames_do_not_feed_the_watchdog)
  | test(round_1::a_stalled_runner_catches_up_and_counts_its_late_steps) | binary(reflexo)
  | test(round_4::an_answer_to_a_client_that_closed_during_its_delay_is_counted_as_none)
  | test(round_4::the_same_client_closing_with_a_reset_counts_none_either)
  | test(round_5::a_client_that_closes_or_resets_during_the_delay_is_gone_at_once_whatever_it_pipelined)`
  (the Modbus server's "gone at once" bounds, the first two added late: 2026-11-23).
  Free, by that rule: the bench file, the channels, the controller's rules, the component, the
  EtherCAT segment (ethercrab's own timeouts are deadlines), `fisica` but its runner, and every
  other test of the files above. A new test that measures time joins this list in its commit, and
  `.config/nextest.toml`'s exclusive block with it. A run record of a file that holds one goes
  through `cargo nextest run`, which honours that block; `cargo test` runs a file's tests as threads
  of one process, the timed ones beside the rest (C-111's record failed so on 2026-11-23).
- **A peek sees only the first byte.** The Modbus server watched a delayed answer's client with a
  one-byte peek; a request pipelined behind made the peek ready for good, and a close behind it went
  unseen till the delay ended. Read ahead into the connection's own buffer instead: the end of the
  stream comes after whatever was sent (C-110, round 5).
- **The accept loop runs before what woke with it.** On a current-thread runtime the `block_on`
  future is polled before the tasks woken in the same tick, so a client that closes and connects
  again at once finds its old connection still counted against the limit. Yield once at the limit
  before refusing (C-110, round 5).
- **A thread spawned is not a line written.** The emitter's loop reads its stop flag before its
  first line, so a part that fails right after it (a server on a taken port) can stop and join
  it before its thread's first tick, and its file is not there: CI on Linux did exactly that
  (the run for 4dcdbf8). A test of "stopped and joined before the error came back" asserts the
  file's state unchanged over two and a half intervals, a first line or none, never that the
  first line is on disk the moment `Bench::start` returns.
- **Dep-info is one compilation's view.** The content check reads what rustc read, so it sees
  only what the `cfg`s of that run compiled in: it builds debug and checks release on the
  platform of the run (Windows here, Linux in CI); another OS, `cfg(doc)` and `cfg(doctest)`
  are outside it, and the clause says so rather than claiming more (C-024, rounds 4 and 5).
- **Never open a log file by name.** A reopen after a failed write appends to whoever took the
  name meanwhile, or sticks on a file that is gone: leave the file and take a fresh name with
  `create_new` (C-025, round 4).
- **A knob is answered after the status mirrors it.** The channel's service loop answered a
  knob as it applied it and mirrored the controller's status at the end of the pass, so a caller
  reading the status right after `set_bit` saw the pass before; the reply now follows the mirror
  (C-031's socket test).
- **Peer ids are core's, counted across the process.** Two channels in one test see ids 1 and 3,
  never 1 and 1: read the id from the status's peers, never assume it.
- **An item at no chute gets a drawn outcome.** A destination outside the list (the unit's
  reject, 0) forces nothing, so "every chute full" leaves it alone; a test of FULL outcomes
  counts the outcomes at a chute (C-032).
- **`cargo metadata` spans every platform; `--offline` does not.** The dependency walk resolves
  the graph for every platform (a banned crate behind another platform's `cfg` must be seen),
  and a registry cache holds only what this platform fetched to build: on a fresh Linux runner
  the walk failed offline on chrono's Android crate. The walk's metadata call is online; the
  nested build and the `--no-deps` metadata calls stay offline.
- **serde's `flatten` disarms `deny_unknown_fields`.** The bench's OPC table flattened the
  server's config under the part's name and accepted a misspelled key; a table that adds a key
  to another struct's spells that struct's keys out (C-033).
- **Record on CRLF.** `core.autocrlf=true` makes a freshly written LF file show as modified at
  the next checkout, which stales a record. Before `zen-record-run`: `git ls-files --eol` lists
  `i/lf w/lf` files, `perl -pi -e 's/\r?\n/\r\n/'` converts them, and `git add -u` must then
  show nothing.
- **One stream, many purposes, is timing in disguise.** The controller drew items, outcomes,
  cyclic messages and answers from one seeded stream; a tick long enough to carry an outcome and
  the next announce in the other order (a starved host) shifted every later item's draw, and two
  runs under one seed disagreed on CI. A stream per purpose keeps a seed meaning "the same run".
  Found with a status read that could not see travelling items and a count taken before the
  list it was read from had caught up: a test that reads a snapshot waits for what it asserts.
- **A randomized tag holds its initial value until its first draw.** The draws test read a tag
  right after connecting and, on a slow runner, got the configured 0 outside the range it drew
  in (C-026, found by CI twice). A test of what a clock moves waits for the move it asserts: the
  status counts the draws, so the read waits for the first.
- **A Modbus frame holds 253 bytes of PDU, and a request past it is no request.** A write of 124
  registers with its 248 bytes needs a PDU of 254: its length field says 255, which no frame has,
  and the server closes the stream rather than answer 03. A test of a quantity one past the limit
  asks it inside the largest frame there is (C-110).
