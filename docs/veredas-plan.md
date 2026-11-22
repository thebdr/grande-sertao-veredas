# Veredas 2: the plan

This is the founding plan for **grande-sertao-veredas**, the Veredas 2 suite. It records
what was decided in the review of 2026-11-05. When anything else disagrees with this file,
this file wins; change it in the same commit as the decision that changes it.

Veredas 1 (the `Veredas` repo) keeps its own history and may keep developing; the two
lines never merge.

## Why a second generation
Veredas 1 grew phase by phase into 16 crates sharing one workspace version (0.2.1), with a
crate called `veredas-core` that is really the first app's data plane. Many features are
unfinished. Veredas 2 sets the borders before the suite grows further: one versioning
scheme, one naming rule, and a clear role for each component.

## Versioning

### SemVer, per component
`MAJOR.MINOR.PATCH`. **Every component starts at 2.0.0.**

| Part | Bump when |
|---|---|
| MAJOR | A breaking change to a public API, a wire protocol or a plain-text file format |
| MINOR | A new capability that keeps every existing consumer working |
| PATCH | A fix that changes no interface |
| Pre-release | Work toward a version not yet promised stable (`-alpha.N`, `-beta.N`, `-rc.N`) |

### How releases work
- **Independent versions.** After 2.0.0 each component owns its version; there is no shared
  workspace version.
- **Suite releases.** A suite release is a manifest that pins a tested set of component
  versions, shipped and installed together.
- **The tester machine.** A suite release also ships as a virtual machine for VMware, on a light
  Linux distribution, holding the whole release suite: testers and designers run the suite
  without building it (the architect, 2026-11-22). Both are planned in `docs/entrega-plan.md`
  (the contract's P-011 and P-022).
- **Core ripples outward.** A MAJOR bump of `veredas-core` means every dependent is rebuilt
  and retested before its own next release.
- **Versioned files.** Every plain-text config and project file is TOML and states its
  schema as `format` and `version`; a misspelled key is refused. Portal migrates it or
  refuses it, and never guesses. Veredas 1's JSON project files are not carried as they are:
  what portal absorbs of them is read into TOML (`docs/questions.md` Q10; the pattern is in
  each `docs/*-schema.md`, from the channel file on: the worked example loaded by the test,
  every key statement the reader's, and, from the catalog on, the text held as the test's copy).
  Law L7 (`tools/leis`) holds every TOML file that is not a cargo manifest and every TOML block of
  a reference to the stamp; one reader in core (`veredas_core::schema`) reads it for every file.
- **New components** start at core's current MAJOR (3.0.0 if core is on 3.x).
- **ORCL** has its own SemVer as a language; controlador states which ORCL versions it runs.

## Naming
`veredas-<nome>`:
- **Portuguese, and it names the role.** Aranha, the spider, crawls the network. Ábaco, the
  abacus, computes. Portal is the gateway. Oficina is the workshop.
- **ASCII ids, accented display names.** Crates and binaries stay plain
  (`veredas-abaco`, `veredas-supervisao`); screens say Ábaco and Supervisão.
- **Exceptions.** `veredas-core` keeps its name. ORCL is a language, not a component, and
  keeps its own crate names (`orcl-std`, `orcl-runtime`). `veredas-wms` keeps the industry's
  acronym. Extensions live inside their host and drop the `veredas-` prefix (`acerto`, an
  Ábaco extension).

## Adding a component
1. **A role nobody covers.** One sentence no existing component already answers, written
   in `docs/<nome>-plan.md` before the code.
2. **Generalization first.** Anything a second component could use is designed for
   `veredas-core` from the start.
3. **Through portal.** All exchange with the outside world goes through `veredas-portal`.
   Exempt: the test tools (maquinar, aranha) and controlador's real-time path.
4. **Fixture first.** Any hardware it talks to has a simulator in `veredas-maquinar`, built
   before or alongside it.
5. **Named and versioned.** A Portuguese `veredas-<nome>`, starting at core's current MAJOR.

## Architecture
Every component meets the outside world (PLCs and OPC UA servers, TCP telegrams, log files,
browsers) through **portal**. Two kinds of exception:
- **Test tools own their sockets.** Maquinar plays the hardware and aranha scans the
  network; neither runs during production.
- **Controlador's real-time path** goes straight to the fieldbus through its NIC. Everything
  else it does rides an embedded portal, the full one with its store and its server (the
  architect, 2026-11-15; this plan first said "portal-lite"); its hardware configuration embeds
  aranha.

`veredas-core` sits underneath everything as a library, not a service.

**Oficina**, the one native application, reaches every component: a local one directly, a remote
target over SSH, where it starts and stops the components' executables, copies files and carries
their loopback interfaces inside the connection. Plant data and commands still go through portal:
Oficina embeds a portal, which talks to the portal embedded in controlador over portal's own HTTP
interface (the architect, 2026-11-13 to 2026-11-16; `docs/oficina-plan.md`).

## Components

### veredas-core: the foundation
Every class and library more than one component needs.
- Scope: shared types (tags, values, timestamps, quality); shared protocol layers (a
  hardened TCP socket; a framing engine driven by a channel configuration schema, with no
  frame layout of its own; a rules engine that reads message fields by configuration; the
  OPC UA client); plain-text file schemas with their version stamps, the channel file being
  the first (`docs/channel-schema.md`); errors, logging and test-fixture plumbing.
- Rules: new functionality is designed as if a second consumer already existed; component
  code moves here once a second component needs it; a library container, never an
  executable, with no limit on what it holds; depends on no other component;
  content-agnostic: names no device, vendor or message catalog, because what a device's
  telegrams mean is configuration (decisions, 2026-11-05).
- From Veredas 1: `veredas-telegram` (its socket, framing and capture only), `veredas-opc`.
- Decided in the briefing of November 2026, not built (`docs/core-plan.md`): the controller's
  state text, one writer and one reader, with the table of reason numbers; the functions that
  drive hardware, the four fieldbus masters, safe in controlador on Linux and callable by other
  crates for testing only; aranha's PROFINET codecs and the OPC UA server code move here. The
  masters' frames are fixed by their standards; the rule that core's telegram framing holds no
  layout of its own stands (`docs/questions.md` Q9, revisited).

### veredas-portal: the gateway
The single point of information exchange between Veredas and the outside world.
- Scope: the data plane (tailing, log families, regex capture; telegram channels configured
  by rules); the DuckDB session store; SSE, windowed REST and the gated write surface; one
  executable, `veredas-portal <project.toml>`, as its shell (Q14, decided 2026-11-09: no desktop
  UI in portal; the operator works through the executable and supervisão's web interface).
- Rules: every component exchanges data through portal, except the test tools and
  controlador's real-time path; writes stay gated (whitelisted tags, master switch off by
  default); a device spoken over TCP is a telegram channel configured by encoding and
  decoding rules (plain text, versioned, in a project folder, as an OPC rule maps tags and a
  log rule captures fields), its data stored like any other protocol's, never vendor code;
  ships a light, embeddable build (controlador embeds the full portal, not the light build: the
  architect, 2026-11-15).
- From Veredas 1: `veredas-core`, `veredas-store`, `veredas-server`, `veredas-desktop`.
- Decided in the briefing of November 2026, not built (`docs/portal-plan.md`): Oficina embeds a
  portal too, and two portals speak portal's own HTTP interface (HTTPS later), commands on the
  same link; the portal embedded in a controller serves OPC UA and Modbus/TCP to other systems
  and keeps its recorded values in memory, as a fixed window.

### veredas-controlador: the softPLC
Programmed in ORCL (Open Rust Control Language), and in ladder later.
- Scope: ORCL programs in a deterministic scan cycle with watchdog and safe outputs; direct
  NIC access for real-time communication; PLC Hardware Configuration built on an embedded
  aranha; an embedded portal for logging and non-real-time protocols (the full portal: the
  architect, 2026-11-15).
- Rules: the only production component allowed past portal, and only on the real-time path;
  everything else goes through its embedded portal; ORCL stays a translatable subset of real
  Rust, with its own SemVer; controlador states which ORCL versions it runs.
- Builds on: `orcl-std`, `orcl-runtime`, `demos/conveyor`.
- Plan: `docs/controlador-plan.md`, from the briefing of November 2026: the state model, faults,
  forcing and debugging, the shadow test, four fieldbuses, what other systems may read and write,
  and what keeps a machine running for decades (`docs/robustness.md`).

### veredas-maquinar: the bench
Tools that simulate real hardware, so every consumer meets its plant on a bench first.
- Scope: log emitters and a configurable OPC UA server; TCP telegram channels up to a full
  simulated controller, driven by the same rules a portal telegram channel is configured
  with, never by vendor code; automated replies to incoming data, random or rule-based;
  controllable pressure (rates, no-reads, full chutes); later, a 2D/3D plant simulator.
- Rules: fixtures come before the consumer that needs them; a test tool that owns its
  sockets; speaks the real device's wire, so a consumer cannot tell the difference;
  test-only and light: no DuckDB, never portal's store or desktop; may use aranha's codecs
  to emulate the devices aranha scans, so the two never drift apart on the wire
  (`docs/questions.md` Q5).
- From Veredas 1: `veredas-maquinar`, `veredas-trajetorias` (the automated replies only).
- Decided in the briefing of November 2026, not built (`docs/maquinar-plan.md`): bench devices
  for the four fieldbuses, and two extensions inside maquinar, `fisica` and `reflexo`, that
  compute the plant simulator's physics and device behaviour headless.

### veredas-aranha: the network scanner
For setup and troubleshooting, never for production.
- Scope: PROFINET DCP and LLDP first, then EtherNet/IP, Modbus, S7, OPC UA, BACnet; listens
  before it asks and fuses every protocol's answer with provenance; draws the topology and
  diffs it against a baseline.
- Rules: used for setup and fault-finding only, not while the plant produces; a test tool
  that owns its sockets; embedded by controlador as its PLC Hardware Configuration; also
  runs on its own for commissioning.
- From Veredas 1: `veredas-aranha` and its plan.
- Decided in the briefing of November 2026: its scan feeds the compare and merge of controlador's
  hardware configuration and the topology view; its PROFINET codecs move into core when the
  controller needs them (`docs/core-plan.md`).

### veredas-comissionador: the testing platform
Predefined checks on portal data, live or replayed over history.
- Scope: checks run in real time or replay historical windows; its own designer with one
  test file and two views, text or flow chart; script boxes, and custom functions written
  in text then reused as blocks (like ladder function blocks); branching on any outcome,
  with retries; an acceptance report for every run.
- Rules: reads plant data only through portal; drives machinery only through portal's
  gated writes; tests are plain text, diff-able in git.
- Design: `design/comissionador/`.

### veredas-abaco: the engineering spreadsheet
For engineering work where Excel gets too tight.
- Scope: a fork of IronCalc modified for engineering; Python macros and custom functions;
  byte-faithful `.xlsx` edits with no Office on the machine; its own `.abaco` workspace
  file; extensions, starting with `acerto` (plant alignment).
- Rules: edits the same engineering workbooks the other components read; fork fixes go
  back upstream to IronCalc where they fit; accuracy is measured against Excel's own
  cached values.
- From Veredas 1: `veredas-abaco`, `veredas-abaco-py`, the IronCalc fork, `abaco-oracle`
  (as test tooling), `python/veredas-acerto` (as the `acerto` extension).

### veredas-supervisao: the web interface
The suite's web interface: a plant watched and operated from a browser. Oficina shows its pages
inside the workbench (the architect, 2026-11-13; this plan first said "the unified web interface
for the whole suite").
- Scope: layouts in 2D (SVG) and 3D (glTF) sharing one set of object IDs; objects that move
  live across the layout; dashboards, charts and tables, absorbing Veredas 1's dashboard;
  several sessions served side by side from one machine; editing in the browser or in
  config files.
- Rules: configuration is plain text, never binary, and 3D models are assets it points to;
  owns no plant connection, every value arrives through portal; every write rides portal's
  gated surface; it starts the plants' portal sessions in its own process from their project
  files (Q19); its plant model is a binding file and an SVG, never a plant's engineering exports
  (Q18); local, no accounts (Q20); 3D after the 2D views are verified (Q21).
- From Veredas 1: `veredas-supervisao` (not its export readers), `dashboard/`.
- Design: `design/supervisao/`.
- Decided in the briefing of November 2026, not built (`docs/supervisao-plan.md`): colour, size,
  position and visibility as expressions of values, computed in Rust; a behaviour editor with
  object types; translatable from day one; green as the colour of a good state. Built on
  2026-11-19: the accent moved from green to lavanda (`docs/questions.md` Q22, reversed).

### veredas-oficina: the workshop
The one native application through which a person sets up, programs, monitors and commissions
every component.
- Scope: a workbench (one project tree for every component, tabbed editors, inspector, docking,
  command palette); every TOML file as a form and as text; portal's and the bench's surfaces
  first, with supervisão's pages embedded; then each component's surfaces as it lands,
  controlador's above all (the ORCL editor, forcing, breakpoints, the shadow test, hardware
  configuration).
- Rules: Qt Quick with its logic in Rust; Windows 11 and Linux; owns no plant connection, it
  embeds a portal and reaches a remote target over SSH; only Oficina and the target change a
  controller's state; translatable from day one; GPLv3, the crates it links under a permissive
  licence; every test drives the real interface.
- New in Veredas 2 (the architect, 2026-11-13). Plan: `docs/oficina-plan.md`.

### A panel HMI: still to be planned
A separate, lighter runtime for small operator panels, whose screens Oficina's HMI creator edits.
It has no name, no technology and no plan yet (the briefing of November 2026).

### veredas-wms: later
A production warehouse management system, structured for the job. It arrives after the
components above, with a dedicated study, then its own plan and development phase. Production
storage lives here, never in maquinar; its structure comes from the study, not from
`veredas-trajetorias`.

## From Veredas 1 crates to Veredas 2

| Veredas 1 | Veredas 2 |
|---|---|
| `veredas-core`, `veredas-store`, `veredas-server`, `veredas-desktop` | portal |
| `veredas-maquinar` | maquinar |
| `veredas-trajetorias` | maquinar (automated replies); its DuckDB store is not carried |
| `veredas-aranha` | aranha |
| `veredas-supervisao`, `dashboard/` | supervisao |
| `veredas-abaco`, `veredas-abaco-py`, IronCalc fork | abaco |
| `abaco-oracle` | abaco, as test tooling |
| `python/veredas-acerto` | `acerto`, an Ábaco extension |
| `orcl-std`, `orcl-runtime`, `demos/conveyor` | ORCL, used by controlador |
| `veredas-telegram` (socket, framing, capture), `veredas-opc` | core |
| `veredas-telegram` (GENIBELT catalog, sort plan, decision client) | not carried: rules a portal channel and maquinar's controller are configured with |
| (new) | comissionador, controlador app, oficina, a panel HMI (to be planned), wms (later) |

## Decisions log
2026-11-05, plan review:
1. Test tools (maquinar, aranha) own their sockets, past portal.
2. Trajetorias' automated replies move to maquinar, with no DuckDB.
3. The telegram and OPC layers move into core, a container with no limits.
4. Acerto becomes an Ábaco extension, without the `veredas-` prefix.
5. Core keeps its name; WMS keeps the industry's acronym.
6. Comissionador: one test file, two views, with script and function blocks and branching.
7. ORCL has its own SemVer; controlador holds the compatibility requirements.
8. A new component starts at core's current MAJOR.
9. Supervisão: 2D layouts in SVG, 3D in glTF, one set of object IDs.

2026-11-05, laying the base (`docs/questions.md` Q1–Q5): the laws live in `tools/leis`; the
review deck is committed; no session log; the plan wins on intent, the contract on status;
maquinar may use aranha's codecs.

2026-11-05, porting core (`docs/questions.md` Q8, Q9): GENIBELT does not belong in core. The
architect: "genibelt is a machine my company manufactures and the interface via TCP socket
should only be something veredas-portal manages, using encoding and decoding rules and storing
the data in the database, as per the other protocols. veredas shall be a generic platform to
build on top of, which reliably manages communication with the outside world, while being
content-agnostic. genibelt does not belong in core, a tcp socket configurable for it does."
Core keeps the socket, its framing and the capture rules engine; the GENIBELT catalog, the sort
plan and the decision client are not carried into the suite as code. Then, on Q9: "Design
configurable framing now": core holds no frame layout; a framing engine runs whatever layout a
versioned channel file describes (`docs/channel-schema.md`), and GENIBELT's layout is such a
file, outside the suite's code.

2026-11-09, planning supervisão (`docs/questions.md` Q18–Q21, the architect): Veredas 1's parsing
of one company's engineering exports is not carried at all; supervisão starts the portal sessions
in its own process; local, no accounts; the 3D view comes later.

2026-11-13 to 2026-11-19, the briefing on Oficina and controlador (the architect;
`docs/oficina-plan.md`, `docs/controlador-plan.md`, `docs/robustness.md`, `docs/questions.md`
Q31–Q45; the list that was reviewed and its review are kept in `docs/briefing/`):
1. A new component, `veredas-oficina`: the one native application, for every component.
   Supervisão and the warehouse system stay web and appear inside it.
2. Controlador's scope is set: its state model, faults, forcing and debugging, the shadow test,
   four fieldbuses with their masters in core, Modbus/TCP and OPC UA to other systems.
3. Only a person, in Oficina or at the target, changes a controller's state (the program may
   stop itself); every
   engineering change needs a switch armed at the target.
4. Controlador embeds the full portal and Oficina embeds a portal; the two speak portal's own
   HTTP interface, commands on the same link.
5. Core gains the fieldbus masters; its telegram framing still holds no layout of its own
   (`docs/questions.md` Q9, revisited).
6. The accent is lavanda; green means a good state (`docs/questions.md` Q22, reversed).
7. Oficina is GPLv3; the crates it links carry a permissive licence.
8. A robustness round for the whole suite, for machines that run unattended for decades: begun,
   not finished. A security round is owed.
9. A panel HMI is a component still to be planned.

2026-11-22, the move to Linux (the architect):
1. Development is on a Linux machine, with a Windows 11 virtual machine on it for testing on
   Windows. The suite green on that machine is the gate; GitHub's run on every push stays as a
   clean-checkout check no push waits for.
2. One session per component, its main developer, for the nine components planned (core,
   maquinar, portal, supervisão, controlador, aranha, comissionador, ábaco, oficina), and one for
   suite releases; an orchestrating session checks their results and carries the questions. The
   warehouse system and the panel HMI stay parked. This overtakes "one member at a time" and
   aranha's wait for the architect's word (`CLAUDE.md`, "Several sessions at once").
3. A new product: the tester machine, a VMware virtual machine on a light Linux distribution
   holding the whole release suite, for testers and designers ("How releases work").
4. Every machine checks out LF (`docs/questions.md` Q46, taken meanwhile).

## Next steps
1. Record each decision above where it applies (component plans, `docs/questions.md`).
2. Reshape the workspace: move and rename crates per the table; drop the shared version.
3. Cut `2.0.0-alpha.1`: every component on the new scheme, building and tested.

Borders first, then features.
