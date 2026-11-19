# Veredas briefing of 13 to 15 November 2026: what it shall deliver

**A record, kept as it was reviewed.** This is the list of what the briefing of 13 to 15 November
2026 decided, as the assistant wrote it for the operator and as five reviewers attacked it on 15
November (`docs/briefing/review.md`, whose entries cite the item numbers below). It is not updated.
The operator's rulings on the review changed several items (among them B2.2, B2.5, B3.6, B5.4,
B7.3, B7.4 and D1); the current word is in `docs/oficina-plan.md`, `docs/controlador-plan.md`,
`docs/robustness.md` and the component plans, and what is open is in `docs/questions.md`.

Every decision below was made by the operator in interview rounds. Items marked (reading) are the
assistant's interpretation, not the operator's words. Items marked (open) were never asked.

Context for a reader who was not in the briefing. Veredas 2 is a Rust suite for industrial data
logging, supervision and control: core (shared library), portal (the gateway: reads logs, OPC UA and
TCP telegrams into a DuckDB store, serves them, gates writes), maquinar (the bench: simulates
hardware over the real wire protocols), supervisão (the web interface), aranha (network scanner),
comissionador (test platform), ábaco (engineering spreadsheet), controlador (a soft PLC programmed in
ORCL, a subset of Rust). Core, maquinar and portal exist; supervisão is being built; the rest is
planned. One maintainer builds it with AI coding agents under a strict rule: every capability has a
contract clause and a test that drives the real artifact.

## A. Oficina, a new component (veredas-oficina)

The one native application through which a person sets up, programs, monitors and commissions every
component. Supervisão and the future warehouse system stay web and appear inside it.

### A1. Foundations
- A1.1 Shell in Qt Quick (QML); all logic in Rust through the cxx-qt bridge, kept behind one seam so
  the bridge can be replaced; C++ wherever Qt needs it, with no limit.
- A1.2 QML may hold view logic (formatting, animation, selection); never project or plant logic.
- A1.3 Runs on Windows 11 and Linux; Linux through X11 compatibility first. No Windows 10, no ARM
  build. Follows Qt releases every six months.
- A1.4 On hosts without a GPU (virtual machines only: Hyper-V, VMware, VirtualBox, Linux guests)
  everything except 3D works. Oficina detects software drawing, switches off effects, starts the
  embedded browser in software mode, opens 3D only in a reduced mode behind a warning, and says so.
- A1.5 Embedded Chromium shows supervisão's and the warehouse system's pages and hosts the code
  editor (Monaco). Where Chromium cannot draw: a plain native text editor, and web pages open in the
  system browser.
- A1.6 Docking through KDDockWidgets: panes float to other monitors, drag to re-dock, named layouts
  saved and restored. Auto-hide side panes written by us, because the library lacks them in Qt Quick.
- A1.7 The whole interface is translatable from day one. Component state text and logs stay English.
- A1.8 Accent colour lavanda #B9A9E8 (soft #2B2838, outline #494360, text on accent #12101C; hover
  and data-series tint #CFC3F2 proposed). Green #30D158 means a good state and is never the accent.
- A1.9 Remote targets through SSH tunnels: components keep listening on loopback; Oficina starts and
  stops their executables and copies files over plain SSH. No accounts of its own.
- A1.10 Every capability's test drives the real interface: Rust tests through the Spix tool.
- A1.11 Licence GPLv3 for Oficina. The rest of the suite has no licence stated.

### A2. Workbench (first release)
- A2.1 Workbench only, no guided or task view: one project tree covering every component, tabbed
  editors, inspector, messages, a command palette.
- A2.2 Every TOML file opens as a form (built from its schema) and as text, two views of one file;
  edits round-trip and keep comments.
- A2.3 Compare offline against online and offline against offline. No Git integration.
- A2.4 Target selector, and a status bar showing the target, the switch state and the running
  download.

### A3. Portal surfaces (first release)
- A3.1 OPC UA address browser that writes the polling rules.
- A3.2 Capture-rule tester: sample log lines against a rule, live.
- A3.3 Telegram frame designer with a live hex view of real traffic decoded by field.
- A3.4 Certificates (OPC UA trust list) and the project's encrypted secrets file.
- A3.5 Session control and health: start, stop, per-source state, lag, reconnects.
- A3.6 Log trailing, and grep over logs at rest, including very large files.
- A3.7 Database explorer for the DuckDB store: tables and views, SQL, result grid, CSV export,
  snapshot.
- A3.8 One scope for any live or stored signal, later reused for controller variables.

### A4. Bench surfaces (first release)
- A4.1 Bench file editor. A4.2 Traffic monitor with decoded fields. A4.3 Start a bench beside the
  portal session that reads it. Later: the live pressure panel.

### A5. Supervisão inside Oficina (first release)
- A5.1 Supervisão's pages embedded as tabs.

### A6. Surfaces that arrive with their components
- A6.1 Ábaco: the grid as a tab, with the engine and Python of each workspace in its own process;
  Python editor with completion; run with stop-on-error; a full debugger; forms as controls on the
  sheet and as designed dialogs. Later: macro recorder.
- A6.2 Comissionador: flow-chart designer, text editor on the same file, and the run-and-report
  screen (PDF, sign-off), delivered together.
- A6.3 Controlador:
  - ORCL editor with a language server; quick switch between ORCL and a ladder view of the same
    source (ladder read-only at first).
  - Inline live values; cross-reference and call tree; subset checker messages; static analysis and
    naming rules.
  - Watch tables; forcing with three modes for variables.
  - Breakpoints, stepping, single cycle, pause on command.
  - Shadow test: differences (outputs and a watch list), swap, accept, the fallback flag in the
    download popup.
  - Hardware configuration: device tree and parameters, scan/compare/merge, topology, IO mapping
    with live values, forcing and the per-output halt setting.
  - Simulator controls: virtual time, snapshots.
  - Project data editors: multilingual text lists, alarms and events, recipes, units and limits.
  - Download history (last 10) and rollback.
  - The controller state as a strip; a click opens a detail view with the state's history and the
    fault records (also shown in a loud popup).
- A6.4 Commissioning: IO check without a program; identify, name and address a device. Later: drive
  parameters, control panel, tuning, drive scope.
- A6.5 3D plant simulator as an authoring environment in Qt Quick 3D: place equipment from a
  library, set behaviours, collisions and physics.
- A6.6 HMI creator: edits the screens of a separate panel HMI (see H).

## B. Controlador, the soft PLC. Everything below ships in ONE first release.

### B1. Execution
- B1.1 Production on Linux with a real-time kernel only. Windows for development and simulation. A
  production target refused real-time scheduling stays in STOP.
- B1.2 Programs run in our own interpreter (a crate of the ORCL family, also used by maquinar). Its
  bytecode is made from the Rust compiler's intermediate form, through a front end the planned
  translators to Siemens SCL can share. One pinned compiler version.
- B1.3 The interpreter is proven against Rust by a fixed conformance suite.
- B1.4 Entry points, after Siemens' organisation blocks: several main blocks and several start-up
  blocks run in order; cyclic interrupts with their own period and priority; time-of-day blocks;
  time-delay blocks; hardware interrupts (a device's process event); handlers for time error, device
  lost or returned, device diagnostics, pull or plug of a module, I/O access error, device alarms
  (status, update, manufacturer or profile-specific), program error, and stop requested (one last
  scan). A more urgent block interrupts a running one.
- B1.5 Start kinds: warm (retained variables kept; also after a power cycle) and cold (everything
  reinitialised, on command). Start-up shows its phase: load, restore, handler, devices. After the
  start-up time it goes to RUN regardless of missing devices and shows what is missing.
- B1.6 After a power cycle: RUN returns as RUN; STOP, HOLD and FAULT all return as STOP, with the
  reason saying what it was.

### B2. Monitoring, forcing, debugging
- B2.1 Live values of any variable; watch tables.
- B2.2 Forcing of inputs, outputs and variables. A variable is forced once now, before each scan, or
  always (held at every write; marked [!]; confirmation). Forces end with the session.
- B2.3 Breakpoints and stepping on the real target and in the simulator. A hit halts every task.
  Outputs at a halt follow a per-output setting, default safe, hold if the engineer chose it; a
  forced output follows the same setting. The program's clock pauses. A halted target stays halted
  if Oficina disconnects.
- B2.4 Pause on command (halts at the end of the scan), single cycle, and dry run (logic on live
  inputs, every output safe).
- B2.5 One switch, set in the target's own configuration and never over the network, gates writes,
  forcing and breakpoints. Stopping is free; starting, continuing from HOLD and acknowledging a
  fault need the armed switch.

### B3. Online change
- B3.1 A shadow test on the whole application: the new version is deployed, both run on the same
  inputs each on its own data, one drives the outputs. Oficina shows what would differ.
- B3.2 A user action swaps them; the old one keeps running as the watcher until the engineer
  accepts. The version taking over continues from the machine's real data, carried by name and type.
- B3.3 Logic, data and the IO mapping may differ between the two; hardware configuration and task
  timing may not.
- B3.4 A controller enters a shadow test only after a check that the doubled scan fits the period.
- B3.5 Breakpoints only in the driving version. Trouble in the watching version (an error, or an
  overrun) ends the test and the machine keeps running.
- B3.6 Fallback for a faulted edit: armed by a controller-wide flag offered in the download popup,
  with a time limit, reset on shutdown. If the swapped-in version faults, the old one takes over
  from its own watcher data, with a loud popup, and the state gains a maintenance request. When the
  time limit runs out the edit is accepted and the old version stops watching.
- B3.7 The controller keeps its last 10 downloads: hash, time, origin, kind, source revision, the
  program itself and the source archive.

### B4. Faults
- B4.1 The whole controller faults, never one task alone. One kind of fault only.
- B4.2 In FAULT every output takes its declared safe value and the fieldbus keeps exchanging.
- B4.3 Leaving FAULT: acknowledge to STOP, then start.
- B4.4 Fault reasons: watchdog, program error without a handler, raised by the program with a code,
  a fault inside a handler, resources exhausted, an application that cannot be loaded, the runtime's
  own failure (after an unexpected restart it comes up in FAULT).
- B4.5 Overruns: the time-error handler runs once on the first overrun; the controller faults only
  when one scan exceeds twice its period.
- B4.6 A device lost while running never faults the controller: its inputs are invalid and the
  program reacts.
- B4.7 A fault clears every debug condition and records what was active.
- B4.8 A fault record holds cause, time, download number, task, source location, the active debug
  conditions, a snapshot of all data, the last seconds before it, and the handler's outcome.
- B4.9 The program may raise a fault with a code, request a stop, raise a maintenance request, or
  record an event only.

### B5. State model
- B5.1 Two layers. Layer 1 is one word: STOP, STARTUP, RUN, HOLD, FAULT. Layer 2 is function-style
  terms: at most one reason, then conditions in a fixed order; counts and identifiers, never lists.
- B5.2 Values follow TOML's rules: integers bare, strings quoted.
- B5.3 The controller holds the state as structured data; one writer and one reader in core produce
  the text. The text stays English; details are translatable.
- B5.4 Every reason is an enum entry with a stable number and a word; the text shows both, the
  number as the first argument `id`. States: 0 unknown, 1 STOP, 2 STARTUP, 3 RUN, 4 HOLD, 5 FAULT.
  Reasons: STOP 101 command(source), 102 power_on(was), 103 acknowledged, 104 no_program, 105
  realtime_refused; STARTUP 201 warm, 202 cold, with phases load, restore, handler, devices(up, of);
  HOLD 401 breakpoint(at), 402 command(source), 403 step, 404 cycle; FAULT 501 watchdog(task), 502
  program(at), 503 user(code), 504 handler(event), 505 resource(kind), 506 load(download), 507
  internal. Command sources: oficina, local, peer, program.
- B5.5 Conditions, in this order: breakpoint(set), force(in, out, var), shadow(drives, watches),
  dry, sim, no_realtime, devices(missing), overruns(count), retain(reset), then each
  maintenance_request(id, reason, urgent).
- B5.6 Maintenance reasons: 1 fault_on_update (urgent), 2 device (as the device reports), 3
  device_missing (urgent), 4 overruns (not urgent), 5 retain_reset (not urgent), 6 user (as the
  program states). Cleared only from Oficina or by a restart; raised again if the cause persists.
- B5.7 Examples: `RUN`; `RUN breakpoint(set=2) force(in=1, out=2, var=3) shadow(drives=13,
  watches=12) sim()`; `HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)`;
  `STOP command(id=101, source="oficina")`; `FAULT watchdog(id=501, task="main")`; `STARTUP
  warm(id=201) devices(up=3, of=5)`; `RUN maintenance_request(id=3, reason="device_missing",
  urgent=true)`.
- B5.8 The standard health value is derived: FAULT gives failure; STOP, STARTUP, HOLD and any debug
  condition give function check; a maintenance request gives maintenance required; clean RUN gives
  normal.

### B6. Networks and devices
- B6.1 Four networks in the first release, built in this order: PROFINET IO controller (written by
  us on aranha's codecs; join PROFINET International, certify later), then EtherCAT master (on the
  ethercrab crate), EtherNet/IP scanner (fork a young Rust crate, test on real devices), Modbus TCP
  client (existing crates).
- B6.2 Vendor description files (GSDML, ESI, EDS) are copied into the project and read directly;
  the chosen configuration is written as Veredas TOML.
- B6.3 IO check runs through a controlador target. With none set, a light controlador session with
  only the logic to drive the remote IO module runs on the laptop, timeouts never above 2 s, known
  devices only.

### B7. Talking to other systems
- B7.1 Modbus/TCP and OPC UA by default, served by controlador's embedded portal.
- B7.2 They publish the controller's state, its faults and only the variables the program marks.
- B7.3 A variable is writable from outside only if marked so, under a project setting of its own,
  separate from the commissioning switch.
- B7.4 Peers may also start and stop the controller, under that same setting.
- B7.5 Modbus addresses are declared by the engineer. The state sits in a fixed block of registers,
  identical on every controlador.

### B8. Language and project data
- B8.1 ORCL text first; ladder later as a view of the same source.
- B8.2 Multilingual text lists, alarm and event definitions, recipes, units and limits on variables.

### B9. Out of scope
- Technology objects entirely (axes, PID, cams, counters). Security features, until studied.
  Certified functional safety. Know-how protection. Synchronous cycle, motion control and redundancy
  blocks.

## C. Core
- C1 The state text's writer and reader, and the table of reason numbers and words.
- C2 The hardware driving functions (the four network masters) live in core: safe in controlador on
  Linux, callable by other crates for testing only.
- C3 Aranha's PROFINET codecs move into core; the adopted network libraries become core's
  dependencies.
- C4 The OPC UA server code moves from maquinar into core.
- C5 (open) Where the readers of GSDML, ESI and EDS files live.

## D. Portal
- D1 The embedded portal gains OPC UA and Modbus/TCP server sides.
- D2 Controller state changes and faults are stored like any other data; the history view reads
  them.
- D3 The grep engine over logs at rest, which the portal plan left waiting.

## E. Maquinar
- E1 Two extensions inside maquinar, called by their bare names as acerto is: `fisica` (bodies,
  motion, which sensor an object reached) and `reflexo` (what makes simulated devices behave:
  ready-made models for drive, sensor, conveyor, actuator, and beyond them ORCL function blocks run
  by the interpreter). Both compute headless at a fixed step.
- E2 Bench devices for all four networks, in Rust, inside maquinar: a PROFINET device, an emulated
  EtherCAT device answering on a raw network card, an EtherNet/IP adapter, a Modbus server.
- E3 Two new dependencies: the interpreter and a physics engine.

## F. Supervisão
- F1 Stays web. The browser decides nothing; no scripts.
- F2 The binding file's rules grow so colour, size, position and visibility can be expressions of
  values, computed and tested in Rust.
- F3 Edit mode gets a behaviour editor. The binding file gains object types: a type holds the
  behaviour, objects may override it. Apply to all objects of a type, or to the selected objects,
  where the first selected is the master and only compatible parameters are applied.
- F4 Translatable from day one.
- F5 Accent changes from green to lavanda; green means a good state.

## G. Aranha
- G1 Its scan feeds the compare and merge of the hardware configuration and the topology view.
- G2 (open) Whether "identify, name and address" runs from the laptop or through a target.

## H. A new component still to be planned
- H1 A separate panel HMI: a lighter runtime for small operator panels, with its own plan. Its
  technology is not chosen.

## I. Repository changes when this is written down
- I1 `docs/oficina-plan.md` before any code; new entries in `docs/questions.md`.
- I2 The plan's line calling supervisão "the unified web interface for the whole suite" reworded.
- I3 Earlier decisions revisited: Q5 (maquinar may use aranha's codecs), Q12 (the bench has no UI),
  Q14 (portal has no desktop UI), Q20 (local, no accounts: extended for SSH), Q22 (green accent).
- I4 New rows in the dependency map of `tools/leis` (oficina; maquinar on the interpreter; core's
  new dependencies).
- I5 `docs/design/README.md`: the accent's tones; the number colour in the syntax palette moved.
- I6 A licence file.

## J. Work that is not code
- J1 One own-shell spike before the plan commits: open a project folder and show the tree; one TOML
  file as form and text with a round trip that keeps comments; a million-row table from a Rust
  model; a Monaco tab with a language server on an ORCL file; a supervisão page embedded and read
  back by a test; a run on each GPU-less virtual machine; one headless test that drives the UI.
- J2 A study of the interpreter's front end and of the list of core-library operations ORCL allows.
- J3 PROFINET International membership.
- J4 The security study the operator postponed.

## K. Readings and open points (to be attacked, not assumed)
- K1 (reading) Arguments inside `force` are always in the order in, out, var.
- K2 (reading) Every non-numeric value in the state text is quoted.
- K3 (reading) A force suspended at a halt applies again when the program continues.
- K4 (reading) With the IO mapping allowed to differ, the shadow comparison is per physical channel.
- K5 (reading) Start-up phases have numbers but show no id in the text.
- K6 (reading) Several maintenance requests can be active, each as its own term.
- K7 (reading) A crash is told from a power cycle by whether the machine itself rebooted.
- K8 (reading) Tasks share data only through the process image, which is what makes pre-emption
  safe without locks.
- K9 (open) What a time-of-day block does when the program's clock was paused during a halt.
- K10 (open) Whether each block gets its own slice of the input and output image.
- K11 (open) How a shadow test's period check works with several pre-emptive blocks.
- K12 (open) Whether "good" in supervisão is a new state class or the colour of the normal one.
