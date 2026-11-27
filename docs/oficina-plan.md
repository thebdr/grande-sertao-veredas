# veredas-oficina: the plan

The workshop of Veredas 2; screens say Oficina. `docs/veredas-plan.md` ("veredas-oficina: the
workshop") wins on intent; `.zen/contract.md` (P-015, and C-250 on) wins on status. Its crate began
on 2026-11-25 ("The crate", below), after the own-shell spike's verdict. This file
holds what the architect decided in the briefing of 13 to 16 November 2026 and in the rulings of
15 November on its adversarial review. The list that was reviewed and the review itself are kept as
they were delivered (`docs/briefing/deliverables.md`, `docs/briefing/review.md`); where they differ
from this file, this file is the later word. What is not decided is a question in
`docs/questions.md`, named under "Open" below; nothing open is written here as settled.

## The role nobody covers
**Oficina is the one native application through which a person sets up, programs, monitors and
commissions every component of the suite.**

No other component answers this. Portal and the bench are a library and an executable each, with
no desktop interface (`docs/questions.md` Q14, Q12). Supervisão is the web interface a plant is
watched through; it stays web, and so will the warehouse system. Oficina shows their pages inside
its own window and adds what a browser page is not asked to be: the engineering tool of its three
users, the PLC programmer, the commissioning technician and the software engineer.

## Scope

### Foundations
- **Shell in Qt Quick (QML), logic in Rust** through the cxx-qt bridge, kept behind one seam so
  the bridge can be replaced (The Qt Company's own Rust bridge was a technology preview when this
  was decided). C++ wherever Qt needs it, with no limit. The bridge is looked at again when Qt's
  own leaves preview, in 2027 (minor 51, Q31, ruled 2026-11-26).
- **QML may hold view logic** (formatting, animation, selection), never project or plant logic.
- **Windows 11 and Linux**, Linux through X11 compatibility first. No Windows 10 and no ARM build:
  an ARM device runs components only. Each Oficina release pins one Qt version (minor 51), Qt
  6.12.0 today. On a Wayland desktop the window runs on Wayland by Qt's default when no platform
  is named (the architect's first run, 2026-11-26), where every test so far ran on `offscreen` or
  on X11 (`xcb`, this desktop's XWayland among them): whether Oficina claims Wayland or chooses
  X11 itself is Q372.
- **Hosts without a GPU are virtual machines only** (Hyper-V, VMware, VirtualBox, Linux guests).
  There everything except 3D works: Oficina detects software drawing before its first window, runs
  with Qt Quick's software backend, switches effects off, starts the embedded browser in software
  mode and says so; the 3D view is unavailable (minor 52, Q31, ruled 2026-11-26, against the
  review's wording, which kept the normal pipeline over a software rasteriser). The spike measured
  both: the normal pipeline over llvmpipe and WARP, and the software backend on `offscreen`, where
  points 1, 2, 3, 5 and 7 passed (point 6, below). The software backend on a display is for the
  drawing probe's clause to run.
- **An embedded Chromium** shows supervisão's pages and the warehouse system's, and hosts the code
  editor (Monaco). Where Chromium cannot draw: a plain native text editor, and web pages open in
  the system browser.
- **Docking through KDDockWidgets**: panes float to other monitors, drag to re-dock, named layouts
  are saved and restored. Auto-hide side panes are written here, because the library has none on
  its Qt Quick side. Docking across monitors is not declared outside the suite (entry 29's third
  option was not chosen, Q31): it is proven where a display is, not headless.
- **3D in Qt Quick 3D**, view-only first, its behaviours chosen from ready-made ones only (entry
  31, Q31); on a host with a GPU only (minor 52).
- **Translatable from day one**, the interface alone: a component's state text and its logs stay
  English.
- **The components' names stay Portuguese on screen** (law L2): where a screen presents a
  component, its role shows as a subtitle, and the command palette matches unaccented spellings and
  English aliases (minor 54, Q31, ruled 2026-11-26).
- **The accent is lavanda `#B9A9E8`** (soft `#2B2838`, outline `#494360`, text on the accent
  `#12101C`; the hover and data-series tint `#CFC3F2` is the assistant's proposal). Green
  `#30D158` means a good state and is never the accent (`docs/questions.md` Q22, reversed on
  2026-11-15). Every state has a mark and a word, so it reads without colour (the design's rule,
  `docs/design/README.md`; entry 43, Q31, ruled 2026-11-26): good, warning and fault are one khaki
  under the most common colour blindness. The outline keeps its tone (the review's `#7A7399` not
  chosen). A light theme joins the list (entry 43), its tones set with the first code that draws
  it.
- **Licence GPLv3.** Every crate Oficina links carries a permissive licence that GPLv3 accepts, and
  a check of dependency licences runs in CI (the architect, 2026-11-15). The suite's permissive
  crates state `MIT OR Apache-2.0` and Oficina `GPL-3.0-or-later`, the GPL version 3 text beside
  its manifest (`COPYING`), the licence files' copyright holder thebdr (Q35, ruled 2026-11-25).
- **Two tiers of proof** (entry 29, Q31, ruled 2026-11-25): logic is proven on the Rust model, and
  each surface by a Rust test that drives the real interface through the Spix tool, the own-shell
  spike being the gate both stand on.
- Qt Creator was dropped as a base and is kept as a reference.

### How it reaches a target
- **Over SSH.** The components on a target keep listening on the loopback. Oficina logs in by SSH,
  over the same LAN or a VPN, starts and stops the components' executables, copies files, and
  carries their interfaces inside the connection. No accounts of its own (Q20 stands). Loopback is
  no boundary (entry 33, Q31, ruled 2026-11-25): portal and supervisão require on every request a
  random token per session, handed over SSH, and check `Host` and `Origin`; controlador's
  engineering channel is a Unix socket (a named pipe on Windows) under the system's permissions.
- **Pairing is one step** (entry 42, Q31, ruled 2026-11-26): Oficina creates the key and installs
  it on the target. Not chosen: opening the project from the target, and one connection view.
- **A controller may run without a credential**, permitted and discouraged (Q43, ruled
  2026-11-26): Oficina warns at each connection to a controller without a credential; the
  controller itself raises nothing.
- **An older controller** (Q235, ruled 2026-11-26): Oficina speaks its versions for everything
  short of a download (connecting, reading the state and the records, watching, forcing), writing
  only what that controller reads. A program download to a controller older than Oficina's release
  is refused, the controller's update offered first. No interface is frozen as a rescue path. How
  far back Oficina speaks is Q371.
- **Portal to portal.** Oficina embeds a portal, and that portal talks to the portal embedded in
  controlador (the architect, 2026-11-15) over portal's own HTTP interface, "http (https, later)"
  (2026-11-16): the event stream, the windowed reads, the gated write surface. The interface
  answers on the controller's loopback only. On a host that runs supervisão too, Oficina and
  supervisão share one portal instance for a project: whichever needs it first starts it, the other
  joins, and it stops when the last leaves (entry 22, Q31, ruled 2026-11-25; Q19 amended for such a
  host). SQL over a running session's store goes through a read-only route in portal.
- **Commands travel on the same link** (2026-11-16): forcing, breakpoints, downloads, start, stop
  and acknowledge. Portal carries them and controlador handles them behind its portal, where the
  commissioning switch is checked (`docs/controlador-plan.md`).
- **Everything that alters the controller is robust**: "every bit of data that alters the
  controller shall be robust, that includes commands such as variable writes, breakpoints setting,
  state change" (the architect, 2026-11-16). A download carries a hash, "so no corrupted program
  may run". Ruled of Q37 on 2026-11-25: each altering request carries a SHA-256 of its content
  (HTTP's `Content-Digest`, RFC 9530) that the controller checks before acting; a change is
  applied whole, at one point of the scan; and a request whose answer is lost is shown as
  "unknown", its outcome asked of the controller by the request's number, never assumed. What
  Oficina shows as done is read back from the controller (the value now in memory, the new state
  text, the whole breakpoint list), never an echo of what was asked, and a write's answer says how
  far it got, applied and then saved. A command passes portal's write switch and then the
  commissioning switch (Q37). A signature over a download's manifest and a keyed check on each
  altering request wait for the security study (Q43, 2026-11-26).
- **Only Oficina and the target change a controller's state** (2026-11-15): a person does it in
  Oficina or at the target, and the program may stop itself. Supervisão, SCADA, MES and other
  controllers read the state and never change it.
- **The commissioning switch is not Oficina's to set.** It is a file on the controller's operating
  system that is not writable from outside; a person arms it at the target. Every engineering
  change needs it armed; reading and stopping are free.

### The first release
The workbench, with portal's and the bench's surfaces and supervisão embedded. It does not wait for
controlador. Its surfaces come in an order (entry 30, Q31, ruled 2026-11-25): the shell, the tree,
form and text, the target over SSH, session control and health, log trailing and supervisão's tabs
first, the rest in point releases.

- **Workbench only**, no guided or task view: one project tree covering every component, tabbed
  editors, an inspector, messages and a command palette; a target selector, and a status bar
  showing the target, the switch's state and the running download.
- **Every TOML file opens as a form and as text**, two views of one file. The form is built from
  core's types (entry 30, Q31, ruled 2026-11-25); edits round-trip and keep comments.
- **Compare** offline against online and offline against offline. No Git integration.
- **Portal's surfaces:**
  - an OPC UA address browser that writes the polling rules;
  - a capture-rule tester: sample log lines against a rule, live;
  - a telegram frame designer with a live hex view of real traffic, decoded by field;
  - certificates (the OPC UA trust list) and the project's sealed secrets file;
  - session control and health: start, stop, per-source state, lag, reconnects;
  - log trailing, and grep over logs at rest, very large files included;
  - a database explorer for the DuckDB store: tables and views, SQL through portal's read-only
    route, a result grid, CSV export (a cell that would read as a formula neutralised, entry 34),
    snapshot;
  - one scope for any live or stored signal, reused later for controller variables.
- **The bench's surfaces:** the bench file editor; a traffic monitor with decoded fields; starting
  a bench beside the portal session that reads it. Later: the live pressure panel.
- **Supervisão's pages** embedded as tabs.

### Surfaces that arrive with their components
- **Aranha**, which keeps no interface of its own (Q280, ruled 2026-11-25): the device table, the
  topology, the diff, identify, name and address, as the bench's surfaces are Oficina's.
- **Ábaco:** the grid as a tab, with the engine and Python of each workspace in its own process; a
  Python editor with completion; run with stop-on-error; a full debugger; forms as controls on the
  sheet and as designed dialogs. Later: a macro recorder. The Python editor and debugger use the
  one language-server client and the one debugger client Oficina writes for ORCL too (entry 28,
  Q31, ruled 2026-11-25). The grid's behaviour (Excel's navigation
  and selection, the editor's modes, the clipboard, the filters) is Oficina's own code, written new:
  nothing of Veredas 1's grid is carried, neither its egui crate nor its renderer-neutral core,
  which may be read as a description of the behaviour (Q341, ruled 2026-11-25). Ábaco's worker
  serves the model (cells, values, edits, recalculation) over a local interface the two plans agree
  when the tab is planned.
- **Comissionador:** the flow-chart designer, the text editor on the same file, and the
  run-and-report screen, delivered together. The report is comissionador's TOML record of a run,
  its evidence beside it, which the screen draws as PDF (Q312, ruled 2026-11-25); anyone signs by a
  typed name, a test may name the roles that must sign (the report reads "awaiting witness" until
  they have), and a signed report is frozen. A run started from Oficina first shows the test's gated
  writes for the person to confirm, then arms portal's write switch at its start and disarms it at
  its end (Q314, ruled 2026-11-25): that confirming step is a screen of Oficina's.
- **Controlador:**
  - the ORCL editor with a language server and context help, and a quick switch between ORCL and
    a ladder view of the same source, ladder read-only in release 1 and editable from release 2
    (entries 40 and 45, Q31, ruled 2026-11-26); one language-server client and one debugger
    client on Oficina's side, shared by ORCL and ábaco's Python (entry 28, Q31);
  - a library of blocks shared across projects is a crate named by path or by Git revision,
    pinned in the project file, with no build scripts (entry 44, Q31; entry 32's rule);
  - inline live values; cross-reference and call tree; the subset checker's messages; static
    analysis and naming rules;
  - the IO list, the alarm list and the cross-reference exported to CSV and PDF (entry 45), and
    an example project per network shipped with Oficina (entry 45);
  - watch tables; forcing, with its three modes for variables, each mode shown in words in a
    table, never by the [!] mark alone (minor 49); a structure expands member by member in watch
    tables, forcing and the IO mapping (entry 44). "Force once now" on a variable is the online
    change of a value, TIA's Modify (minor 48);
  - breakpoints, stepping, single cycle, pause on command;
  - the shadow test: what would differ (outputs and a watch list), swap, accept, and in the
    download popup the fallback flag and its time limit, the engineer's, ten minutes by default
    (Q32, ruled 2026-11-26). The popup offers the fallback only where the live-takeover
    specification allows it (`docs/controlador/live-takeover.md`, its S list), and Oficina says
    when the right to take over ends. Where the doubled scan does not fit, the way is stop and
    download, the popup saying so (minor 48). A download to a controller older than Oficina's
    release offers the controller's update first (Q235);
  - hardware configuration: the device tree and its parameters; scan, compare and merge; topology;
    the IO mapping with live values, forcing and the per-output halt setting. Vendor description
    files (GSDML, ESI, EDS) are inputs, read and never written, parsed with document type
    definitions and entities off, their readers in core (entry 23, Q31; Q45's C5). A Modbus module
    with no timeout of its own, or too long a one, is warned of in production too, the engineer
    free to accept it (Q45);
  - simulator controls: virtual time, snapshots;
  - project data editors: multilingual text lists, alarms and events, recipes, units and limits;
  - the download history (the last 10) and rollback; a shadow test accepted when its time limit
    ran out is recorded there as "accepted by timeout", and as an event (minor 47);
  - the controller's state as a strip; a click opens a detail view with the state's history and
    the fault records, which are also shown in a loud popup. A maintenance request carries a
    "seen" mark, so clearing one whose cause persists does not look like a dead button, and the
    detail view decodes the shadow term (`shadow(drives=13, watches=12)`) (minor 50). A fault
    record's snapshot of all data is read through Oficina only, peers getting the cause, the
    number and the time (entry 35, Q31);
  - a backup is one file with a rebuild sheet (Q236, ruled 2026-11-26): its manifest lists the
    program and its source, the hardware configuration, the IO mapping, the target's settings and
    the retained data at that moment, each with its SHA-256, and the sheet says in plain text, for
    someone who never met Veredas, the board and drive needed, the system image and where to get
    it, the network names and addresses, what to install and in what order. Oficina makes it from
    the controller and restores it to any target that meets the hardware requirements, checked by
    its manifest, asking whether the retained data is restored or the program starts cold;
  - the intervention log the target keeps (entry 37, Q31): each line with the time and who, the
    SSH user, the peer's address and a name typed once per Oficina installation;
  - the retained variables' saves: where the drive reports its wear, Oficina shows the years it
    has left at the save's period and its shortest life at the shortest gap (Q40);
  - what a Rust engineer gets (entry 46, Q31, ruled 2026-11-26): an ORCL project is a cargo crate
    whose logic `cargo test` runs, the subset checker works outside Oficina and in CI, and the
    allowed subset of Rust is published as one page (ORCL's, `docs/orcl/README.md`). Oficina is
    one way into such a project, never the only one.
- **Commissioning:** IO check without a program; identify, name and address a device, from the
  laptop when it is on the segment and through a target's embedded aranha when it is not (Q45's G2,
  ruled 2026-11-25). Later: drive parameters, a control panel, tuning, a drive scope.
- **The 3D plant simulator** as an authoring environment in Qt Quick 3D: place equipment from a
  library, set behaviours, collisions and physics; view-only first, behaviours chosen from
  ready-made ones only (entry 31, Q31); unavailable on a host without a GPU (minor 52). The
  physics and the devices' replies are
  computed in maquinar, headless, at a fixed step; Oficina's view draws the positions it is sent
  (`docs/maquinar-plan.md`).
- **The HMI creator** edits the screens of a separate panel HMI, a component still to be planned
  (`docs/veredas-plan.md`, "A panel HMI").

### The IO check from a laptop
With no controlador target set, a light controlador session holding only the logic to drive a
remote IO module runs on the laptop itself, with timeouts never above 2 s and with known devices
only, on Linux laptops only: on a Windows laptop it goes through a target (Q45, ruled
2026-11-25). The architect kept it on 2026-11-15, under three rules:
- the laptop session has an arming step of its own in Oficina, per session, naming the device;
- driving outputs is refused for a device that exchanges data with another controller, and on
  Modbus when the module has no timeout of its own or too long a one; renaming such a device needs
  the arming step and a confirmation;
- outputs are momentary, on only while the control is held; latching one is a separate, explicit
  choice.

## Rules
- **One seam for the bridge.** Nothing outside it knows which bridge joins QML and Rust.
- **No project or plant logic in QML.**
- **Through portal.** Oficina owns no plant connection: it reaches a plant through the portal it
  embeds, and a target through SSH. The one exception is the IO check from a laptop: the light
  controlador session Oficina starts there drives a module itself, on controlador's real-time
  path, behind Oficina's own arming step. It embeds supervisão's pages, never supervisão's code.
- **Oficina changes a plant only through gates it does not own:** the commissioning switch armed
  at the target, portal's master switch and whitelist, and its own arming step for the laptop IO
  check.
- **The state text is never translated and never rewritten**: the same characters on every screen,
  log and test, produced by core's one writer (`docs/core-plan.md`).
- **Green is a state, never the accent.**
- **Two tiers of proof** (entry 29, Q31, ruled 2026-11-25): a clause of logic is proven on the Rust
  model; a clause of a surface only by a test that drives the real interface.
- **Opening a project runs nothing** (entry 32, Q31): ORCL projects have no build scripts and no
  procedural macros; the folder's cargo configuration is not honoured; Python runs after a trust
  prompt per folder; pages not shipped inside Oficina get no native bridge.
- **Text from outside is plain text** (entry 34, Q31): every text from a plant, a file or a peer is
  drawn as plain text, held by a test over the QML; the CSV export neutralises a cell that would
  read as a formula; the interface reads the structured state, never parsed state text.

## Before the code
- **One own-shell spike before this plan commits to its framework** (decided 2026-11-13): open a
  project folder and show the tree; one TOML file as form and text with a round trip that keeps
  comments; a million-row table from a Rust model; a Monaco tab with a language server on an ORCL
  file; a supervisão page embedded and read back by a test; a run on each GPU-less virtual machine
  (Windows on Hyper-V, VMware and VirtualBox, and Linux guests); one headless test that drives the
  interface. How each point is judged, and the spike's verdict, are below ("The own-shell
  spike").
- **Repository changes owed with the first code**, as they stand since the crate began
  (2026-11-25): `crates/veredas-oficina` at core's MAJOR, listed (done, C-250); P-015 becomes
  clauses (C-250 and C-251 so far). Its row in `MAP` (`tools/leis`), a contract change (portal at
  least, and controlador if the laptop IO check's session is linked in), comes with its first edge
  to portal: law L5 lets a component outside the map use core alone, and the first code does. In
  `docs/design/README.md` (which holds the accent's tones since 2026-11-19) the syntax palette's
  number colour (`#D2A8FF`) moves, sitting too close to the accent, and its string green with it,
  green being a state's (entry 24, Q31): with the code editor's clause, the first code that draws
  it. The licence: `GPL-3.0-or-later` and `COPYING` (done, Q35).

## The crate
`crates/veredas-oficina`, from 2026-11-25 (C-250): a library whose model builds on any machine the
suite builds on, the window its feature `shell`, on by default since 2026-11-26 (Q370, ruled: every
machine that builds the suite carries Qt 6.12 and KDDockWidgets, this machine's cargo configuration
naming `QMAKE`), and the tests' driver a feature of its own, never on by default.
- **The model**, no Qt: `tree` (the project tree) and `formats` (which component reads each
  format). Its members are core alone, without core's default features: the header reader, with
  none of core's plant-facing layers (C-073).
- **`shell`**, on by default, the window and the executable `veredas-oficina`: Qt Quick, its logic
  in Rust through cxx-qt, the seam a private module (`src/shell`) with one C++ file beside it
  (`cpp/shell.cpp`), KDDockWidgets docking; `--no-default-features` builds the model alone. It
  needs `QMAKE` naming Qt 6.12's qmake in cargo's environment (cxx-qt's own build scripts read it,
  not only Oficina's) and `OFICINA_DEPS` (`~/.local/opt/oficina-deps` by default) holding
  KDDockWidgets. In the workspace's unified build its crates give two of Ábaco's crates a feature
  each, `span-locations` on proc-macro2 and `std` on foldhash, which Ábaco reviewed (C-220,
  2026-11-26).
- **`driver`**, the tests' driver: Spix's server inside the window, started only when
  `OFICINA_SPIX_PORT` names a port; Spix and AnyRPC in `OFICINA_DEPS`. Only the tests build it:
  what ships carries no test server.
- **Its tests.** A clause's test drives the window through Spix, on Qt's `offscreen` platform
  unless `OFICINA_TEST_PLATFORM` names another (`xcb` for X11, `windows`):
  `QMAKE=$HOME/.local/opt/Qt/6.12.0/gcc_64/bin/qmake cargo nextest run -p veredas-oficina --features driver`.
  The model's own tests (`tests/tree_model.rs`) run in the suite's default build and prove no
  clause. The whole-suite run holds the window's tests with `--features veredas-oficina/driver`.
  On Linux the component test builds the model inside bubblewrap (`bwrap`, no root needed), Qt and
  the C++ libraries hidden, as a machine without them would; GitHub's run builds it before Qt is
  installed.
- **Brought from the spike**, as its verdict kept them: the project tree (its model and its bridge),
  the formats' table, the window's docking and the test driver. The TOML document, the code editor
  and its language-server client, the table, the embedded pages and the drawing probe come over
  with their clauses; the spike's folder stays as its record.
- **What its terminal shows is Oficina's.** The architect's first run (2026-11-26, on Wayland)
  printed two kinds of others' lines: Qt's warning category `qt.qpa.wayland.textinput` (its
  text-input surface "Trying to disable" one surface while none is focused), and `qml: -2x`, a
  `console.log` left in KDDockWidgets 2.5.1's own title bar (`TitleBar.qml`, at each click of a
  dock's float button: the image suffix for a display at twice the density). A release keeps them
  out at the window's start: a Qt category known as noise by a logging rule set before the
  application, so that a person's `QT_LOGGING_RULES` still turns it back on, and a library's own
  print by a message handler that drops what that library's QML file sends, until a release of
  the library drops it. Each lands with a clause whose test reads what the window writes.
  Reporting the print to KDDockWidgets' project is an outward step, the architect's to take or
  allow.
- **What a release ships** (Q443 and Q446, ruled 2026-11-26): the release builder turns on the
  features the executable needs, `shell` and never `driver`, whatever the defaults; the bundle
  carries Qt 6.12's libraries and KDDockWidgets in `lib/` beside `bin/`, with their licence texts
  and where their sources are, the executable finding them by its `RUNPATH`, `$ORIGIN/../lib`.
  The build script writes only this machine's folders into the `RUNPATH` today: `$ORIGIN/../lib`
  joins them when the release member's bundle first carries the window.

## The own-shell spike
The spike answers one question before `crates/veredas-oficina` exists: does Qt Quick, with its
logic in Rust through cxx-qt behind one seam, KDDockWidgets for docking, Qt WebEngine for Monaco
and the embedded pages, and Spix for the tests, carry the workbench on Windows 11 and Linux, GPU or
none? Its verdict, written at the end of this section, is the architect's to read before the crate
begins. It is not the crate: it proves no clause (its tests are evidence for P-015,
`.zen/evidence/P-015.md`), and what it builds is kept only where the verdict says so.

### Where its code lives
- `spikes/oficina/`, a cargo workspace of its own, outside the suite's. Its packages are named
  `oficina-spike-*`: no `veredas-*` or `orcl-*` name and no folder under `crates/` or `tools/`, so
  it is no member and the laws leave it be (L1: a crate elsewhere that is not the suite's is
  nobody's business; L2 and L6 judge members). L7 still holds any TOML file it keeps to the stamp.
  The suite's `cargo build --workspace` never builds it, so no machine needs Qt to build the suite.
- `spikes/oficina/model`: the Rust side the bridge seam hides, with no Qt: the project tree
  (`tree`), the TOML document behind the form and the text (`document`), the formats' components
  (`formats`). It uses core's one header reader by path, and the laws' own reader of the suite's
  TOML texts in its tests. It builds anywhere:
  `cargo nextest run --manifest-path spikes/oficina/Cargo.toml -p oficina-spike-model`.
- `spikes/oficina/shell` (with Qt): the window, the QML and the bridge, a library with a thin
  `main` so its tests link what it runs. Only its `bridge` module names cxx-qt, and one C++ file
  inside the seam starts what Qt wants started in C++ (WebEngine, the docking library, the test
  server); the model's types never cross the seam as Qt types. It builds where `QMAKE` names Qt's
  qmake and `OFICINA_DEPS` (by default `~/.local/opt/oficina-deps`) holds KDDockWidgets, Spix and
  AnyRPC. Its tests start the window on Qt's `offscreen` platform and drive it through Spix; a
  timing is judged on a release build, alone on the machine.
- Pinned for the spike: Qt 6.12.0 (6.12.1's WebEngine is not published yet), cxx-qt 0.10.0,
  KDDockWidgets 2.5.1 (its Qt Quick side), Spix 0.14 over AnyRPC's master at b1949b3 (its 1.0.2
  forces AddressSanitizer on), Monaco 0.57.0,
  rust-analyzer from rustup, CMake 4.5.1 for the C++ libraries. How the machine gets Qt is the
  architect's (the measured routes are in the evidence). Everything is built on Linux here, and
  on Windows in the `win11` guest on this machine (below).

### The seven points
Each point is judged by a Rust test that drives the shell through Spix with no display, unless
the point says otherwise; numbers are taken on this machine and on each virtual machine run.
Where a point leans on one of the review's entries, the entry is not ruled (Q31): the spike
measures it so the answer exists whichever way it is ruled.

1. **A project folder opened, its tree shown.** Proves that a Rust model reaches a QML `TreeView`
   through the bridge as a lazy item model, its ids kept across a re-read. Rust side, built: a
   folder is read when it opens and once; folders first, then by name with case folded; each TOML
   file named by its stamp through core's reader and by the component whose reader states the
   format, a refusal in the reader's own words; a link shown and never followed; nothing in the
   folder run or honoured. Judged by: the planted project of the model's tests shown row for row;
   a folder read only when the view opens it; a refresh that keeps the selection and the open
   folders; a name holding markup drawn as its characters (entry 34); a folder of 100 000 files
   opening in under one second. Failing it: if the bridge cannot express a lazy item model, its
   C++ side grows inside the seam (allowed, "C++ wherever Qt needs it"); if no glue makes it
   lazy, the bridge itself is in question.
2. **One TOML file as form and text, comments kept.** Proves two views of one document. Rust side,
   built: a form edit changes the characters of one value and nothing else (its comment, spacing,
   quoting and radix kept), proven on every TOML text the suite holds (544 edits, each changing
   one line); a text that does not parse is refused at its line and UTF-16 column, the document
   left as it was; CR LF and a byte-order mark, which the TOML editing library drops, put back on
   writing. The form is built from the document's own values, not from a schema (entry 30).
   Judged by: an edit in the form appearing in the text as exactly the expected text; a typed
   change in the text appearing in the form's field; an invalid text marked where it fails while
   the form keeps its last state; the saved bytes equal to the model's. The text view here is a
   native QML editor, the plan's fallback where Chromium cannot draw, so this point proves that
   fallback too. Failing it: if two views cannot stay in step through the bridge, the form and
   the text become two modes of one view.
3. **A million rows from a Rust model.** Proves that a QML `TableView` over a table model written
   in Rust draws only what is visible. Rust side, to build with the point: cells computed on
   demand from a deterministic source (core's PCG32), nothing materialised. Judged by: 1 000 000
   rows of 8 columns open in under one second; the last row reached and its cell read back; the
   process's memory with a million rows within 20 MB of its memory with a thousand; scrolling
   frame times recorded with a GPU and without. Failing it: the cell path moves into C++ inside
   the seam; if it is still too slow, the bridge is in question.
4. **Monaco with a language server on an ORCL file.** Proves that Monaco runs in WebEngine from
   files Oficina ships (no network), reaches Rust through a web channel, and that a
   language-server client on Oficina's side (entry 28) drives rust-analyzer on an ORCL file using
   `orcl-std`. Judged by: a planted type error marked at its line and column within 10 seconds of
   opening, within 2 seconds of an edit; completion offering the timer's methods; and nothing
   run on opening (entry 32): the test plants a build script, a procedural macro and a cargo
   configuration naming a compiler wrapper, each writing a marker file if run, and no marker
   appears. Failing it: a native code editor replaces Monaco (entry 28's third option); if
   rust-analyzer cannot be held to running nothing, the ORCL subset checker is the only check.
5. **A supervisão page embedded and read back.** Proves that a page served by a real supervisão
   host, started by the test on supervisão's own fixtures, shows in a WebEngine tab and that a
   test reads it back through a hook on Oficina's side, never through a script the page carries.
   Judged by: the value of a known object, driven by the bench through portal, read from the page
   equal to what the bench drove; the served page finding no native bridge object (entry 32:
   pages not shipped with Oficina get none). Failing it: on hosts where WebEngine cannot draw,
   web pages open in the system browser, as the plan already says; if it fails everywhere, the
   embedded browser is in question.
6. **Without a GPU.** Proves the plan's promise for virtual machines: everything but 3D works,
   Oficina detects software drawing, switches effects off, starts WebEngine in software mode and
   says so. Here: this machine with Mesa's software rasteriser forced, a Linux guest on QEMU/KVM
   without a virtual GPU, and Windows 11 on QEMU/KVM (the orchestrator's machine) on WARP. Judged
   by: points 1, 2, 3, 5 and 7 passing in each, the detection naming the software adapter, and
   point 3's frame times recorded. **Stated remainder:** Windows on Hyper-V, VMware and
   VirtualBox need machines this one does not have; they wait for the architect. Failing it: Qt
   Quick's one fallback is the normal pipeline over a software rasteriser, chosen before the first
   window (the review's minor 52); if that cannot carry the workbench, the plan's promise for
   virtual machines changes.
7. **One headless test that drives the interface.** Proves that a Rust test, run by cargo
   nextest, starts the shell with Spix's server, drives QML items by name (click, type, read a
   property) with no display, and that its exit code is the verdict. Judged by: one run that
   drives a QML control and an edit inside Monaco, the page read through Oficina's hook (entry
   29's first option); green 20 times in a row here; Qt's `offscreen` platform first, Xvfb only if
   WebEngine needs a display server. The window is a KDDockWidgets main window from point 1 on:
   its panes docked, and a named layout saved and restored inside the headless test; panes floated
   to another monitor are not judged headless (entry 29's third option). Failing it: another
   driver takes Spix's place (Qt's own test library from C++, or the accessibility tree), at the
   cost entry 29 counts.

### The runs elsewhere
Each is one command whose result is one file, so a run on another machine is a command to start
and a file to read back.
- **A Linux guest with no GPU** (point 6): `spikes/oficina/run-guest.sh` on this machine. It boots
  Ubuntu 26.10's own cloud image (this machine's release: the spike runs there as built here) on
  QEMU/KVM with 4 cores, 4 GB and no display device, this machine's folders mounted at their own
  paths, runs the spike's tests from nextest archives on `offscreen` and on X11 under Xvfb, where
  Mesa draws with llvmpipe, and shuts the guest down. Result:
  `spikes/oficina/target/guest/guest-run.txt`; the image and the guest's disk in
  `~/.cache/veredas/oficina/`.
- **Windows**, at a clone of the repository on the Windows machine:
  `powershell -ExecutionPolicy Bypass -File spikes\oficina\run-windows.ps1 -Expect software
  -ExpectAdapter "Microsoft Basic Render"` (no GPU) or `-Expect gpu`. It builds what is missing
  (AnyRPC, Spix and KDDockWidgets as here, Monaco from npm, supervisão's host), runs the suite on
  `offscreen` and on `windows`, point 7 twenty times, and the release timings. Result:
  `spikes\oficina\target\windows-run.txt`, its last lines each step's exit code. Run in the
  `win11` guest on 2026-11-24, started in the signed-in user's desktop session (the `windows`
  platform needs a desktop, which an SSH logon has not). What it needs installed first,
  in the versions this machine used where they apply, is installed by one command run elevated,
  `powershell -ExecutionPolicy Bypass -File spikes\oficina\install-windows-tools.ps1`, each piece
  from its publisher and checked before it runs (in the `win11` guest on 2026-11-24: 49 minutes,
  Build Tools 17.14.45, Git 2.56.1, uv 0.13.9 running aqtinstall's master at 4c75985). It also
  sets, for the user, `QMAKE` and `PATH` with Qt's `bin` and the C++ libraries' `bin`: any cargo
  build of Oficina's window needs the first (MSVC is found without a developer shell), and any test
  of its package the second, its window linked in (measured in the guest, 2026-11-25):
  - Visual Studio 2022 Build Tools 17.14 or later, workload "Desktop development with C++" (MSVC
    v143 for x64 and a Windows 11 SDK): Qt's `msvc2022_64` build is made with it.
  - Qt 6.12.0 for MSVC 2022 64-bit in `C:\Qt\6.12.0\msvc2022_64` (`-Qt` names another place),
    with Qt WebEngine, Qt WebChannel, Qt Positioning, Qt ShaderTools and Qt Quick 3D: through
    Qt's online installer (an account), or aqtinstall from its repository after 2026-11-22, since
    its last release, 3.3.0, cannot read Qt 6.12's Windows repository.
  - Rust 1.100.0 through rustup, its host `x86_64-pc-windows-msvc`, with the components
    `rust-src` and `rust-analyzer`; cargo-nextest 0.9.152.
  - CMake 4.5.1 and Ninja 1.13.2 on `PATH`; Git for Windows; Node.js 26.13.0 with npm 11.20.0.
  - Windows PowerShell 5.1, Windows' own; network to github.com, registry.npmjs.org and
    crates.io during the run; about 40 GB free and 16 GB of memory (the database engine's C++,
    built for supervisão's host, is the largest part).

### Results
On this machine, Qt 6.12.0 on its `offscreen` platform, through Spix, and again on X11 and in
Windows 11 (below); the numbers and what was learnt on the way are in the evidence. A test run
names its platform with `OFICINA_TEST_PLATFORM` (`offscreen` when unnamed).

1. **Passed** (2026-11-22). The planted project drawn row for row; a folder read when it opens
   and not before; a refresh keeps the open folders and the selection; a name holding markup drawn
   as its characters; a folder of 100 000 files opened in 157 to 343 ms from the click to its first
   row drawn (a release build, three runs, the machine's load about 30 on 24 cores). The bridge
   expresses the lazy item model in Rust alone; one C++ file starts the application.
2. **Passed** (2026-11-22). A TOML file opened from the tree shows as a form and as text. A value
   typed into the form came back in the text as exactly the expected text (its radix and its
   comment kept); a change typed into the text showed in its field; a value of the wrong type was
   refused by name; a text that does not parse was marked at its line and column while the form
   kept its last state and saving was refused, and once mended the form followed; saved, the file's
   bytes were the CR LF and the byte-order mark it was read with. The text view is Qt Quick's own
   text editor, so the plan's fallback where Chromium cannot draw passed with it.
3. **Passed** (2026-11-22; its frame times recorded with point 6). A table of 1 000 000 rows of 8
   columns, each cell computed from its row when the view asks: open in about 40 ms, its last row
   drawn in 22 to 28 ms after the jump, every cell of it the model's own, and the process's memory
   with a million rows within 0.2 MB of its memory with a thousand (release, three runs). The
   scrolling frame times, with a GPU and without, are under point 6, where the drawing paths are
   set.
4. **Passed** (2026-11-22). An ORCL file opened from the tree shows in Monaco, in a page Oficina
   ships, joined by Qt's web channel to a language-server client of Oficina's own (the review's
   entry 28, its first option, written in the model so ábaco's Python can share it). rust-analyzer
   marked a planted error at its place 3.8 to 6.1 s after the file was opened; Ctrl+Space offered
   the timer's methods; a correction typed into Monaco removed the mark 0.13 to 0.20 s later; and
   nothing of the project's folder ran (entry 32): a planted build script, a cargo configuration
   naming a compiler wrapper and a toolchain file naming a toolchain of scripts stayed unrun, where
   rust-analyzer started as editors commonly start it ran them. What holds them off: the project
   given to rust-analyzer as an inline description of its crates, build scripts and procedural
   macros off, the toolchain's own binary started in a folder of Oficina's. Spix cannot type into the
   page; a typing hook of Oficina's does, by the keyboard's path.
5. **Passed** (2026-11-23). Supervisão's own host, started by the test on the reference's worked
   example with maquinar's bench playing the plant, served its page into Oficina's web pane; the
   loop's speed, set by the bench and read by portal, was read back from the page through
   Oficina's hook (in a script world of Oficina's own, the page carrying no script for it) equal
   to what the bench held, 0.16 to 0.34 s after each change; the served page found no bridge
   object, where the page Oficina ships found its own.
6. **Passed** (2026-11-23, Windows 2026-11-24), but for its stated remainder. With Mesa's
   software rasteriser forced here (`LIBGL_ALWAYS_SOFTWARE=1`, on X11), and by itself in a Linux
   guest on QEMU/KVM with no display device (Ubuntu 26.10's cloud image, 4 cores, 4 GB, on X11
   under Xvfb), Oficina named llvmpipe as what draws it before its first window, switched its
   effects off, started WebEngine without the GPU (pages get no WebGL) and said so in its footer;
   Qt Quick kept its normal pipeline (OpenGL) over llvmpipe, the plan's fallback. Points 1, 2, 3,
   5 and 7 passed in both, and on `offscreen` in the guest, where Qt Quick draws with its own
   software renderer and Oficina counts that as software too (here in release, 100 000 files in
   174 to 227 ms and a million rows in 51 to 63 ms; in the guest 139 and 50 ms). Point 3's frame
   times, scrolling 500 rows a second for 3 s (median, 95th percentile and worst interval, ms): on
   this machine's GPU 17, 29 to 40, 41 to 105; over llvmpipe here 25 to 31, 42 to 54, 54 to 67;
   on `offscreen` 16, 17 to 24, 18 to 45; in the guest over llvmpipe under Xvfb, which holds no
   frame to a display's rate, 12, 20, 31. In Windows 11 on QEMU/KVM with no GPU (the `win11`
   guest), Oficina asked Direct3D 11 for its adapter before the first window and named WARP, the
   Microsoft Basic Render Driver; Qt Quick drew through Direct3D 11 over it, effects off, pages
   without WebGL, and the footer said so; points 1, 2, 3, 5 and 7 passed there on `windows` and on
   `offscreen`; the frame times over WARP 31 to 32, 49 to 51, 56 to 67. **Not run:** the stated
   remainder, Hyper-V, VMware and VirtualBox.
7. **Passed** (2026-11-23). One test, run by cargo nextest with no display, its exit code the
   verdict: a value typed into the form and the text following it, an edit typed into Monaco and
   read back through Oficina's hook, and a named layout saved by KDDockWidgets' own saver, the
   panes laid out anew (one closed, one floated into a window of its own) and the layout restored,
   every pane back in the main window at its place and size, the page that floated and came back
   still holding its edit (moved, not reloaded). Green 20 times in a row, about 2 s a run. Panes
   floated to another monitor are not judged headless.

On X11 too (2026-11-22, once the architect installed the seven libraries Qt's X11 plugin needs):
the same tests passed on Qt's `xcb` platform, through this machine's display (the desktop's
XWayland, OpenGL 4.6 on its Intel GPU), and the timings held in release (a million rows open in
47 to 63 ms, 100 000 files in 154 to 226 ms); on 2026-11-23 the spike's 40 tests, points 4, 5 and
7 among them, passed there with the GPU and over llvmpipe.

In Windows 11 (2026-11-24, the `win11` guest on this machine: QEMU/KVM, 8 cores, 16 GB, no GPU, so
Direct3D 11 over WARP), Qt 6.12.0 for MSVC 2022: the spike's 40 tests (Linux's 43 but for three of
Unix's own: an unreadable folder, a symbolic link, a name that is not UTF-8) passed on Qt's
`windows` platform in the signed-in desktop and on `offscreen`, every point among them, from one
command (`run-windows.ps1`) that built the C++ libraries from nothing; point 7 20 times in a row,
about 2.3 s a run. Release, three runs: a million rows open in 46 to 88 ms, the last row drawn in
23 to 50 ms, its memory within 2.3 to 7.1 MB of a thousand rows'; 100 000 files in 160 to 228 ms.
In the run of record point 4 marked the planted error in 5.0 and 5.1 s and cleared it 0.13 and 0.16
s after the edit; once, with this machine loaded by other sessions, the mark came after its 10 s,
and five runs straight after marked in 3.6 to 5.2 s. Point 5's page read 0.11 to 0.31 s after each
change. "Nothing of the folder ran" says less there of the planted toolchain file, whose scripts
Windows does not run.

### Verdict
Given 2026-11-25, on the seven points' evidence: **Qt Quick, its logic in Rust through cxx-qt
behind one seam, KDDockWidgets, Qt WebEngine for Monaco and the embedded pages, and Spix for the
tests carry the workbench, on Linux with a GPU and without, and on Windows 11 without one.** Every
point passed on Linux (`offscreen`, X11 with this machine's GPU and over llvmpipe, a Linux guest on
QEMU/KVM with no GPU) and in Windows 11 on QEMU/KVM over WARP (`windows` and `offscreen`), each
from one command whose exit code is the verdict, and no failing branch of any point was taken: the
bridge expresses the lazy models in Rust alone, the C++ stays one file inside the seam, Monaco is
kept, rust-analyzer is held to running nothing of the folder, and the embedded browser stays.
- **What it does not cover:** the stated remainder, Windows on Hyper-V, VMware and VirtualBox,
  needs machines this one has not (the architect's); Windows with a GPU was not run, the guest
  having none, and no point asked for it (Qt Quick takes Direct3D 11 there too, on the GPU's
  adapter instead of WARP); panes floated to another monitor are not judged headless; on Windows
  the planted toolchain file proves less (its scripts do not run there). The review's entries the
  spike measured (28, 29, 32, 34 among them) stay unruled under Q31: the spike gives their answers,
  not their rulings.
- **What is kept:** the model (`tree`, `document`, `formats`, `lsp`) and the bridge's shape (one
  seam, one C++ file) are the crate's starting point, each part entering as its clause is
  written; the test driver (Spix through the tests' `common`) and the runs elsewhere
  (`run-guest.sh`, `install-windows-tools.ps1`, `run-windows.ps1`) carry on as the crate's. The
  QML panes are the spike's and are rewritten under the design system. P-015 becomes clauses
  with the crate (Before the code).

## Open
- **Q31, the review's majors and minors**: ruled on 2026-11-25 and 2026-11-26, those that touch
  Oficina written above where each applies (22, 23, 24, 28 to 35, 37, 40, 42 to 46, and the minors
  47 to 52 and 54). Left: minor 53, bookkeeping (what waits on undecided work, marked so: the HMI
  creator waits for the panel HMI's plan).
- **Q371**, Oficina's: how far back it speaks to an older controller (Q235's floor).
- **Q372**, Oficina's: on a Wayland desktop, Wayland claimed or X11 chosen by Oficina itself.
- **Q36, Q37 and Q43**: ruled but for what waits on the security study (Q43, 2026-11-26): a
  signature over a download's manifest, a keyed check on each altering request, and the end date
  of certificates issued after 2038. **Q45**'s remaining readings (its points on Windows, G2, C5
  and a Modbus module's own timeout are ruled and written above).
- The research behind these decisions (five PLC suites' manuals; three toolkits, Qt 6.12, Avalonia
  12.1 and Slint 1.18; studies of Qt without a GPU, of cxx-qt and KDDockWidgets, of Qt Creator as
  a platform, of softPLC debug and online-change mechanisms and of fieldbus master stacks) was
  read in the assistant's research runs. Their reports are not in this repository.
