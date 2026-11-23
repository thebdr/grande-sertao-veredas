# veredas-aranha: the plan

The network scanner of Veredas 2. `docs/veredas-plan.md` ("veredas-aranha: the network scanner")
wins on intent; `.zen/contract.md` (P-005, then the `C-xxx` clauses it becomes) wins on status.
This file says what aranha does, what it takes from Veredas 1 and what it leaves there, in which
order it lands, and the traps already known. Written 2026-11-22, before any code (law L6).

## The role nobody covers
**Aranha tells who is on an industrial Ethernet segment, how each device names itself and who is
cabled to whom, from what the wire says, with the evidence behind every value, and what changed
since a baseline: for setup and fault-finding, never while the plant produces.**

No other component answers this. Portal reads the plant's data from sources a project names; it
never asks a segment who is there. Maquinar *plays* devices; aranha *finds* them. Controlador's
real-time path exchanges IO with devices its configuration already names; its hardware
configuration needs to be told what is actually on the wire, which is aranha's answer, embedded
there (the plan, "Architecture"). Oficina draws and edits; it owns no plant connection. Without
aranha, the commissioning engineer reaches for a vendor's tool, and the compare and merge of a
controller's hardware configuration has nothing of the suite's own to compare against.

## Scope
- **The PROFINET codecs** (`profinet`): pure code, the frames as bytes and back, with no socket and
  no dependency on the rest of aranha. First the discovery frames: Ethernet II with one 802.1Q tag,
  the MAC address, **DCP** (Identify, Hello, Get, Set and their answers, every option and
  suboption the identify and configure paths use) and **LLDP** (the standard TLVs and the
  organisation TLVs of PROFINET, IEEE 802.1 and IEEE 802.3). Then the frames a PROFINET IO
  controller and a PROFINET device exchange: the **cyclic real-time frame** (frame id, IO data
  with its provider and consumer status, cycle counter, data and transfer status), the **alarm
  frame** (the acyclic real-time protocol's data, acknowledgement and error PDUs and the alarm
  notification), and **PNIO over connectionless DCE/RPC** (the RPC header, the NDR header, the
  PNIO blocks of connect, control, read, write and release, the implicit read, the I&M records and
  the port's neighbour data). These move into core when the IO controller needs them (P-018); the
  bench's PROFINET device speaks through them meanwhile (`docs/questions.md` Q5).
- **The wire** (`wire`): one seam every frame passes through, with three kinds behind it: a raw
  link on a named interface (an `AF_PACKET` socket on Linux), a capture file (classic pcap, read
  and written), and a memory hub for the pure tests.
- **Listening** (`listen`): a worker per interface that hears everything the wire carries before
  anything is sent, keeps every frame verbatim with its interface and time, decodes it into
  **observations** (one thing one frame says about one device, with its witness), and counts what
  it cannot decode, by class, never dropping it unseen.
- **Fusion** (`fusion`): observations into **devices**, the MAC the identity; every field keeps
  every value it was given with the witnesses for each, so a disagreement is a **conflict** on
  record (two devices with one station name, two with one address, a device that changed name or
  address, a device flagging its own address conflict), never a silent overwrite.
- **Asking** (`ask`): DCP Identify (all, by station name, by alias), paced, bounded, and only
  through a listener already running on that interface, so every answer is heard by the same ears
  that heard the wire first.
- **The topology** (`topology`): nodes (devices with their ports, this machine's interfaces,
  chassis named by a neighbour table and not yet identified), links with the confidence their
  evidence earns (heard on our own cable, reported by one side's neighbour table, confirmed by
  both, inferred from a forwarding table), MRP rings, connected components.
- **Scan files and the diff** (`scan`): a scan written as a versioned TOML file (`veredas-scan`,
  law L7) beside the capture of its frames, read back whole; a **baseline** is a saved scan; the
  diff of a scan against a baseline (appeared, vanished, renamed, readdressed, rewired, conflicts
  new and cleared). Its schema document lands with it (`docs/scan-schema.md`).
- **The executable** (`veredas-aranha`): listen on an interface for a while and write the scan;
  identify; diff two scan files. The commissioning tool on its own, from the command line.
- **Later phases**, named, no code now:
  - **deep identity**: the PROFINET implicit read (never an application relation): I&M0 to I&M4,
    the port's neighbour data (where device-to-device links come from), the real identification
    data (the module and submodule layout the hardware configuration compares);
  - **the IP-layer probes**: EtherNet/IP `ListIdentity`, Modbus TCP device identification
    (function 43/14), S7 identity, OPC UA `FindServers` and `GetEndpoints` on core's client, BACnet
    `Who-Is`; and SNMP v2c for managed switches (their LLDP neighbour tables and forwarding
    tables);
  - **writes**: DCP Set of a station name, an IP suite, the identification signal (blink), gated
    (below, "Rules");
  - **Windows**: raw frames there need a packet driver (Npcap in Veredas 1), whose licence for this
    use is open (`docs/questions.md` Q45).

Not in scope, for good: opening an application relation (an IO exchange is the controller's, and
a second controller contending for a device disturbs the first); any frame sent onto a bus that an
active frame can disturb (EtherCAT, POWERLINK: classified and counted, never sent to); watching a
plant in production.

## Rules
- **Setup and fault-finding only**, never while the plant produces (the plan). Nothing in aranha
  runs as a service or feeds portal's store.
- **A test tool that owns its sockets** (the plan, "Architecture"): it opens its raw link and its
  IP sockets itself, past portal.
- **It listens before it asks.** The first thing a scan holds is what the wire already carries. An
  active step is refused on an interface that no listener of this process is hearing.
- **Every active step is bounded and paced.** A rate per interface (Veredas 1's default, 20
  frames a second), a timeout per probe, a cap on what is outstanding. A step that would flood
  (an address sweep wider than a /16) is refused, not throttled into a flood: cheap device
  stacks fall over under scans, and a scanner that stops a line is worse than none.
- **It never guesses identity.** The MAC is the identity, an address is secondary, a name is never
  a key. Where protocols disagree the scan holds every value with its witnesses and names the
  conflict; a chassis key two devices claim resolves to neither, and its links are drawn to an
  ambiguous node with a note.
- **Frames are kept verbatim**, decoded on demand: every observation names the frame it came from,
  and a decoder fixed later reads the same capture again.
- **Writes are a whitelist behind gates, enforced in the library**, below any caller: DCP Set of a
  name, an IP suite and the signal, and nothing else (no factory reset is expressible); one device
  named by its MAC (the codec refuses a multicast or broadcast target); an arming step naming that
  device; a confirmation per action; a running listener. Renaming a device that exchanges data
  with a controller needs the arming step and a confirmation (the architect, 2026-11-15,
  `docs/oficina-plan.md`, "The IO check from a laptop"). Where the step runs, laptop or target, is
  open (Q45); the gates are the same either way.
- **The codecs are written to core's rules from their first line**, so their move into core is a
  move and not a rewrite: they depend on no member and on nothing of aranha; they are
  content-agnostic (no vendor, product or device is named in their code, comments, tests or
  fixtures, and no section sign cites a document, core's `tests/foundation.rs` reader); typed
  errors; total on input (a damaged block is reported with its bytes, never a panic). The rest of
  aranha follows the same rule on names: the vendor a MAC prefix belongs to comes from the IEEE
  registry file read at run time, never from a table in the code.
- **A codec is checked against a decoder that is not ours.** A frame our encoder writes is read
  back by Wireshark's dissectors (`tshark`) and every field compared, besides our own decoder's
  round trip. An encoder and decoder written from one reading of the standard agree with each
  other however wrong that reading is (the review of the briefing, entry 26); the dissectors are a
  second reading, maintained against real devices. Captures of real devices, when a site gives
  them, are fixtures too, kept out of the repository (a plant's MACs and names are plant data).
- **Fixture first.** The scanner's clauses are proven against maquinar's PROFINET bench device on
  a real link, never against frames aranha's own test writes (P-020; the bench uses these codecs,
  so the drift it rules out is between the bench's device and the scanner's reading, and the
  dissectors rule out the drift of both from the standard).
- **The real wire in tests, without privilege.** A test that sends frames runs in a private user
  and network namespace, where it may, with a veth pair between the bench's device and the
  scanner: the kernel's own Ethernet path, nothing of the host's touched, gone when the test ends
  ("Traps", the namespace).
- **Configured, not programmed.** Every plain-text file is TOML stating `format` and `version`
  (law L7), a misspelled key refused; a scan is `veredas-scan`.
- **A library first, one executable on top.** Everything runs in-process behind handles, so
  Oficina and controlador embed it. No interface of its own (`docs/questions.md` Q280, taken
  meanwhile).
- **Light.** The bench depends on aranha for its codecs, and the bench is test-only and light: no
  database, no web stack, no UI crate in aranha's graph.
- **Typed errors on the public surface**; no `unwrap`/`expect` on an I/O path; tests may unwrap.
- **One test file per capability** under `tests/`, so each clause links one file and a change to
  one capability re-stales only its own recorded run.

## What it absorbs from Veredas 1
Read before the port: `docs/aranha-plan.md` and `docs/aranha-usage.md`, all of
`crates/veredas-aranha` (`src/`, its examples and tests), the `aranha/` page, and maquinar's
`pnsim.rs` and `tests/pnsim.rs` that speak through it. What moves where:

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| `mac.rs`, `ether.rs`, `dcp.rs`, `lldp.rs` | `profinet::{mac, ether, dcp, lldp}` | the discovery codecs, checked against the dissectors on the way in ("Traps") |
| `probes/pnrpc.rs` (the RPC header, the implicit read of I&M0) | `profinet::rpc`, the implicit read in deep identity | the RPC framing serves the controller's connect as much as the read; its own header says it was never checked against a real device |
| (new) | `profinet::{rt, alarm}`, the connect, control, write and release blocks | the IO controller and the bench device need them; Veredas 1 had none of the cyclic exchange |
| `wire.rs` (`Wire`, `ReplayWire`, `Hub`), `pcap.rs` | `wire` | the seam and its file and memory kinds; pcap written as well as read |
| `npcap.rs`, `adapters.rs` (Windows, `wpcap.dll` and `GetAdaptersAddresses` bound by hand) | later, the Windows phase | the Linux raw link comes first (`AF_PACKET`, the interface's MAC from the kernel); Npcap waits for Q45 |
| `observe.rs`, `fusion.rs`, `engine.rs` | `observe`, `fusion`, `listen` | as they were, with typed decode errors and the own-machine filter |
| `capture.rs` (a thread per adapter, the one paced sender) | `listen` | the listener is the only sender on its interface, so asking is paced where it is heard |
| `active.rs` (Identify-All, the ARP sweep) | `ask` | the ARP sweep with the IP-layer probes, later |
| `active.rs` (the SNMP survey), `snmp.rs` | later, the IP-layer probes | where device-to-device links come from on managed switches |
| `probes/opcua.rs` | later, on core's OPC UA client | Veredas 1 rode the client crate directly |
| `topology.rs` | `topology` | the confidence tiers, rings, components, the ambiguous chassis |
| `snapshot.rs` (JSON snapshot, baseline, diff, the expected list) | `scan` | TOML (law L7); the expected list is not carried: comparing what the wire shows with what a configuration expects is controlador's hardware configuration, fed by the scan |
| `write.rs` (the gates, `actions.log`) | later, `write` | the gates re-read against the rulings of 2026-11-15 |
| `oui.rs` (thirty vendor prefixes built in) | not carried as a table | content-agnostic: the IEEE registry file is read at run time, and the column says when a prefix is not in it |
| `prefs.rs`, `project.rs`, `files.rs` (`%APPDATA%` JSON, the project's `aranha/` folder) | not carried | a scan file and the executable's arguments; where a project keeps its baselines is Oficina's and controlador's to say |
| `server.rs`, `main.rs` (axum: JSON, SSE), the `aranha/` page (vite, TypeScript) | not carried | Q280: the surfaces are Oficina's; the executable is a command line |
| `examples/fake_switch.rs` (an SNMP agent) | maquinar, with the SNMP phase | a fixture is the bench's |
| `examples/npcap_probe.rs`, `examples/synth_capture.rs` | not carried | a spike and a fixture builder; the codecs' own encoders build fixtures in the tests |
| `tests/offline_capture.rs`, `tests/opcua_probe.rs` | `tests/*.rs`, one per clause | real links and real servers |
| maquinar's `pnsim.rs`, `tests/pnsim.rs` | maquinar's PROFINET device (P-020) | on these codecs; its `Wire` the scanner's seam |

Changed on the way, each recorded in its clause's evidence note:
- edition 2024; the `String` errors of the pcap reader, the SNMP client and the write log typed;
- a damaged DCP block carries a typed reason, not a sentence in a `String`;
- the fixtures name no vendor and no product: neutral station names, the locally administered
  MAC range (`02:…`), made-up vendor and device ids;
- the Windows-only raw binding gives way to the Linux link first;
- JSON files give way to TOML;
- what the dissectors show Veredas 1 read wrongly ("Traps") is fixed with a test that pins the
  dissectors' reading.

## Module map
```
veredas_aranha
  profinet      the codecs; no dependency on the rest of the crate (they move into core: P-018)
    mac           Mac: the identity, its spellings, the multicast addresses DCP and LLDP use
    ether         Ethernet II with one 802.1Q tag; the EtherTypes
    dcp           DCP: the header, the blocks, Identify, Hello, Get, Set and their answers
    lldp          LLDP: the TLVs, the PROFINET, 802.1 and 802.3 organisation TLVs
    rt            the cyclic frame: frame id ranges, IO data, cycle counter, data and transfer status
    alarm         the acyclic real-time PDUs and the alarm notification and acknowledgement
    rpc           connectionless DCE/RPC and NDR headers, the PNIO blocks, the record read
    record        the records a read returns: I&M0, the port's real data with its peers
  wire          the seam; the raw link (Linux), the pcap file, the memory hub
  netns         the private network a test makes: a user and network namespace, veth pairs
  observe       a frame into observations with their witness; the frame's class
  fusion        devices with per-field provenance; conflicts
  listen        the listener: a worker per interface, the kept frames, the paced sender, status
  ask           DCP Identify, paced, through a listener
  topology      nodes, links with confidence, rings, components, notes
  scan          the scan file (`veredas-scan`), the baseline, the diff
  main.rs       veredas-aranha listen | identify | diff
```

## Phases
1. **The plan** (this file) and its questions, before any code (L6).
2. **The discovery codecs.** `crates/veredas-aranha` at `2.0.0-alpha.1`, listed; the component
   clause (P-005 promoted: a library and at most one executable, its plan before its code, core
   its only member dependency, nothing heavy in its graph); `profinet::{mac, ether, dcp, lldp}`
   with Veredas 1's tests, exact bytes, and the dissectors' reading of every frame. Pushed at once:
   the bench's PROFINET device needs them.
3. **The IO codecs.** `profinet::{rt, alarm, rpc}`: what the IO controller (core) and the bench's
   device (maquinar) need of the frames, each pinned by exact bytes and by the dissectors. Their
   order follows what those two sessions ask for first; with no ask yet, the cyclic frame and the
   alarm frame came first, then the RPC's header, arguments and record read (aranha's own need in
   phase 6), and the records I&M0 and the port's data; connect, control, write and release follow
   when core asks.
4. **The wire and listening.** The raw link, the pcap file, the hub; the listener with its kept
   frames, observations and fusion; proven on a veth pair in a private namespace against
   maquinar's PROFINET device (fixture first: until it lands, this phase waits on it).
5. **Asking.** DCP Identify through the listener, paced; the bench device's answers fused with the
   Hello and the LLDP already heard, each value with its witnesses; a staged duplicate name
   flagged.
6. **The topology, the scan file and the diff.** Heard links, MRP rings, the ambiguous chassis;
   `veredas-scan` with `docs/scan-schema.md`; a baseline saved and a rewired, renamed or vanished
   bench device found by the diff. Device-to-device links need a neighbour table (Traps), so
   this phase reads the bench device's port data by the implicit read, the first of deep identity.
7. **The executable.**
8. **Later**: the rest of deep identity, the IP-layer probes one at a time (each with its bench
   responder first), writes, Windows.

Each phase ends with its clauses recorded, refuted and verified, the laws green and every test
the change can reach green, and is pushed.

## Decided in the briefing of November 2026
The architect's decisions of 13 to 16 November 2026 that touch aranha (`docs/briefing/`,
`docs/controlador-plan.md`, `docs/oficina-plan.md`); status P-005.
- **Its scan feeds the compare and merge of controlador's hardware configuration, and the topology
  view** (Oficina's). Aranha offers the scan as library types and as a file; the comparison with a
  configuration, and the drawing, are theirs.
- **Its PROFINET codecs move into core** when the controller needs them (P-018), and the bench then
  takes them from core (Q5, revisited). Until then they live here, written to core's rules.
- **The hardware configuration is built on an embedded aranha** (controlador), so aranha is a
  library that runs inside another process, on Linux, with the raw-socket privilege that process
  holds.
- **Renaming a device that exchanges data with another controller** needs an arming step and a
  confirmation (the rules of the laptop IO check).
- Open: where "identify, name and address" runs, laptop or target (Q45); where the readers of
  GSDML, ESI and EDS files live (Q45); whether Npcap's licence allows what Windows needs (Q45).

## Traps
Known before the code, from reading Veredas 1 against the standard and from this machine; each
is pinned by a test when its phase lands.

- **Raw frames need a privilege this machine does not give, and a namespace gives it.** An
  `AF_PACKET` socket needs `CAP_NET_RAW`; no session has sudo. A process may make a user and
  network namespace of its own, where it holds `CAP_NET_RAW` and `CAP_NET_ADMIN` over that
  namespace only: a veth pair made there carries raw frames through the kernel's real path, and
  vanishes with the process (measured 2026-11-22: a DCP frame sent on one end read back whole on
  the other, no sudo). Two conditions: (1) Ubuntu restricts unprivileged user namespaces through
  AppArmor (`kernel.apparmor_restrict_unprivileged_userns = 1`); the desktop app's profile
  (`/etc/apparmor.d/claude-desktop`) grants `userns`, and every process it starts inherits it, so
  the sessions' test runs may; a test run from a plain terminal may not, and needs, once and by
  root, that setting at 0 or a profile granting `userns` to the test binaries. (2)
  `unshare(CLONE_NEWUSER)` refuses a multi-threaded process, and a test binary is one, so the
  namespace is made in a child process that runs the test's body, with the uid map written, or the
  child loses its capabilities at `exec`. A real interface on a real plant needs the privilege
  itself: `setcap cap_net_raw+ep` on the executable, an install step.
- **Passive LLDP gives nodes, not edges** (Veredas 1, phase A1). LLDP is link-local: what a
  listener hears is the far end of its own cable. An interface hearing several senders is a mirror
  port or a hub, and no cable is drawn from it. Device-to-device links come from neighbour tables:
  the PROFINET port data (implicit read) and the switches' LLDP tables (SNMP).
- **One reading of the standard agrees with itself.** Veredas 1's codecs had places where its
  encoder and decoder agreed with each other and not with the dissectors (tshark 4.6.6, read on
  2026-11-22): a Set's control response is option, suboption and error, three bytes and no block
  info, where Veredas 1 wrote and read five (tshark reads that frame's error as 2); and the status
  word of the MRP port TLV is MRRT's port status (off, configured, up), not a ring port that is up
  or down, so "status 0 is not in a ring" would miss every ring whose MRRT is off: the ring is read
  from the domain. Both are pinned (C-132, C-133).
- **A Hello goes to its own group address**, `01:0e:cf:00:00:01`, not the Identify one
  (`01:0e:cf:00:00:00`); a listener and a bench device that use one address for both miss real
  Hellos (C-132's first refute round).
- **The kernel takes a VLAN tag out of a frame** before a packet socket reads it, keeping it beside
  the frame; a socket bound to one EtherType gets a priority-tagged frame with its tag dropped for
  good. The raw link reads every type, filters in the kernel with a program that looks inside a
  tag, and puts the tag back (C-134). Core's IO controller meets the same on its cyclic frames.
- **The dissector reads the alarm frame more loosely than it is written.** It reads TACK as the
  whole high nibble of AddFlags (reserved bits with it), does not bound a block by the var part's
  length (it reads on into the frame's pad), and breaks on a DATA PDU with no block. The alarm
  codec bounds every read by the var part, keeps the reserved bits apart, and keeps what it does
  not read with the reason (C-136).
- **PNIO's RPC is two byte orders at once.** The connectionless header's integers, its UUIDs' first
  three fields, PNIO's argument counts and a response's PNIO status (a 32-bit value) follow the
  header's data representation (most stacks write little-endian); the blocks are big-endian always.
  A codec that writes the counts big-endian under a little-endian header is read by the dissector
  as a gigabyte of arguments, and a status written byte by byte under it reads as another error
  (C-137, its first refute round).
- **A veth end shows its carrier before it sends what it is given.** Right after a pair comes up,
  a frame sent at once from the end raised first is lost about one time in ten (measured by C-134's
  third critic), though `ip` already shows the end up, its carrier on and its queue set. The
  fixture waits for a numbered probe to cross each pair each way (`netns::wait_pairs`); a test that
  raises a pair again calls it too. The raw link's tests wait on deadlines: they run one at a time
  on the machine, under the suite lock, as maquinar's timed tests do.
- **The kernel takes a frame's outer tag out on the far end, not on the sending end**, so a filter
  that counts the tags left in the frame looks one tag deeper on one end than on the other. The
  raw link's filter counts the tag the kernel took out (C-134).
- **A frame shorter than 60 bytes is padded on the wire, and a capture may or may not hold the
  pad**: DCP is bounded by its own data length and LLDP by its end TLV, never by the frame.
- **This machine announces itself.** An operating system's own LLDP agent and its own ARP are on
  the wire too: observations about the listener's own MACs are counted apart and not shown as
  devices (Veredas 1, phase A2).
- **An answer before the listener is lost.** A Hello sent before a listener opens is gone; the
  bench device's Hello, and every answer to an Identify, must come after the listener is up, so a
  test opens the listener first and the bench second, and an Identify is refused without a
  listener (the rule above).
- **Odd lengths and DCP's padding**: a DCP block is padded to an even length and the pad is not
  counted in its length; an odd station name is where an encoder that forgets it breaks the next
  block.
- **The capture file has its own clock.** A pcap record holds seconds and a fraction, micro- or
  nanoseconds by the file's magic number; a frame's stamp is nanoseconds since the epoch, so the
  wire writes nanosecond pcap and nothing heard on a raw link loses its time. A scan written and
  read back keeps the frame's own time, not the time of the read. tshark reads a record's seconds
  as unsigned (to 2106) but writes them signed (to 2038): its rewrite of a capture fails on a later
  stamp, ours does not.
- **tshark reads a capture more loosely than its header says** (measured, 4.6.6): the snapshot
  length is not held against a record, a fraction past a second and a length on the wire under the
  bytes kept draw a complaint and are read, major version 3 is read and 1 is not, and a file cut in
  its last record gives the frames before it with a note that it was cut short. The reader does
  the same, naming the damage, so a capture still being written is read up to where it stops.
