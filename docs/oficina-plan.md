# veredas-oficina: the plan

The workshop of Veredas 2; screens say Oficina. `docs/veredas-plan.md` ("veredas-oficina: the
workshop") wins on intent; `.zen/contract.md` (P-015) wins on status. Nothing is built. This file
holds what the operator decided in the briefing of 13 to 16 November 2026 and in the rulings of
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
  was decided). C++ wherever Qt needs it, with no limit.
- **QML may hold view logic** (formatting, animation, selection), never project or plant logic.
- **Windows 11 and Linux**, Linux through X11 compatibility first. No Windows 10 and no ARM build:
  an ARM device runs components only. Oficina follows Qt's releases, one every six months.
- **Hosts without a GPU are virtual machines only** (Hyper-V, VMware, VirtualBox, Linux guests).
  There everything except 3D works: Oficina detects software drawing, switches effects off, starts
  the embedded browser in software mode, opens 3D only in a reduced mode behind a warning, and
  says so.
- **An embedded Chromium** shows supervisão's pages and the warehouse system's, and hosts the code
  editor (Monaco). Where Chromium cannot draw: a plain native text editor, and web pages open in
  the system browser.
- **Docking through KDDockWidgets**: panes float to other monitors, drag to re-dock, named layouts
  are saved and restored. Auto-hide side panes are written here, because the library has none on
  its Qt Quick side.
- **3D in Qt Quick 3D.**
- **Translatable from day one**, the interface alone: a component's state text and its logs stay
  English.
- **The accent is lavanda `#B9A9E8`** (soft `#2B2838`, outline `#494360`, text on the accent
  `#12101C`; the hover and data-series tint `#CFC3F2` is the assistant's proposal). Green
  `#30D158` means a good state and is never the accent (`docs/questions.md` Q22, reversed on
  2026-11-15).
- **Licence GPLv3.** Every crate Oficina links carries a permissive licence that GPLv3 accepts,
  MIT or Apache 2.0, and a check of dependency licences runs in CI (the operator, 2026-11-15;
  which expression is Q35).
- **Every capability's test drives the real interface**: Rust tests through the Spix tool.
- Qt Creator was dropped as a base and is kept as a reference.

### How it reaches a target
- **Over SSH.** The components on a target keep listening on the loopback. Oficina logs in by SSH,
  over the same LAN or a VPN, starts and stops the components' executables, copies files, and
  carries their interfaces inside the connection. No accounts of its own (Q20 stands).
- **Portal to portal.** Oficina embeds a portal, and that portal talks to the portal embedded in
  controlador (the operator, 2026-11-15) over portal's own HTTP interface, "http (https, later)"
  (2026-11-16): the event stream, the windowed reads, the gated write surface. The interface
  answers on the controller's loopback only.
- **Commands travel on the same link** (2026-11-16): forcing, breakpoints, downloads, start, stop
  and acknowledge. Portal carries them and controlador handles them behind its portal, where the
  commissioning switch is checked (`docs/controlador-plan.md`).
- **Everything that alters the controller is robust**: "every bit of data that alters the
  controller shall be robust, that includes commands such as variable writes, breakpoints setting,
  state change" (the operator, 2026-11-16). A download carries a hash, "so no corrupted program
  may run". How both are built is Q36 and Q37.
- **Only Oficina and the target change a controller's state** (2026-11-15): a person does it in
  Oficina or at the target, and the program may stop itself. Supervisão, SCADA, MES and other
  controllers read the state and never change it.
- **The commissioning switch is not Oficina's to set.** It is a file on the controller's operating
  system that is not writable from outside; a person arms it at the target. Every engineering
  change needs it armed; reading and stopping are free.

### The first release
The workbench, with portal's and the bench's surfaces and supervisão embedded. It does not wait for
controlador.

- **Workbench only**, no guided or task view: one project tree covering every component, tabbed
  editors, an inspector, messages and a command palette; a target selector, and a status bar
  showing the target, the switch's state and the running download.
- **Every TOML file opens as a form and as text**, two views of one file. The form is built from
  the file's schema; edits round-trip and keep comments.
- **Compare** offline against online and offline against offline. No Git integration.
- **Portal's surfaces:**
  - an OPC UA address browser that writes the polling rules;
  - a capture-rule tester: sample log lines against a rule, live;
  - a telegram frame designer with a live hex view of real traffic, decoded by field;
  - certificates (the OPC UA trust list) and the project's sealed secrets file;
  - session control and health: start, stop, per-source state, lag, reconnects;
  - log trailing, and grep over logs at rest, very large files included;
  - a database explorer for the DuckDB store: tables and views, SQL, a result grid, CSV export,
    snapshot;
  - one scope for any live or stored signal, reused later for controller variables.
- **The bench's surfaces:** the bench file editor; a traffic monitor with decoded fields; starting
  a bench beside the portal session that reads it. Later: the live pressure panel.
- **Supervisão's pages** embedded as tabs.

### Surfaces that arrive with their components
- **Ábaco:** the grid as a tab, with the engine and Python of each workspace in its own process; a
  Python editor with completion; run with stop-on-error; a full debugger; forms as controls on the
  sheet and as designed dialogs. Later: a macro recorder.
- **Comissionador:** the flow-chart designer, the text editor on the same file, and the
  run-and-report screen (PDF, sign-off), delivered together.
- **Controlador:**
  - the ORCL editor with a language server, and a quick switch between ORCL and a ladder view of
    the same source (ladder read-only at first);
  - inline live values; cross-reference and call tree; the subset checker's messages; static
    analysis and naming rules;
  - watch tables; forcing, with its three modes for variables;
  - breakpoints, stepping, single cycle, pause on command;
  - the shadow test: what would differ (outputs and a watch list), swap, accept, and the fallback
    flag in the download popup;
  - hardware configuration: the device tree and its parameters; scan, compare and merge; topology;
    the IO mapping with live values, forcing and the per-output halt setting;
  - simulator controls: virtual time, snapshots;
  - project data editors: multilingual text lists, alarms and events, recipes, units and limits;
  - the download history (the last 10) and rollback;
  - the controller's state as a strip; a click opens a detail view with the state's history and
    the fault records, which are also shown in a loud popup.
- **Commissioning:** IO check without a program; identify, name and address a device. Later: drive
  parameters, a control panel, tuning, a drive scope.
- **The 3D plant simulator** as an authoring environment in Qt Quick 3D: place equipment from a
  library, set behaviours, collisions and physics. The physics and the devices' replies are
  computed in maquinar, headless, at a fixed step; Oficina's view draws the positions it is sent
  (`docs/maquinar-plan.md`).
- **The HMI creator** edits the screens of a separate panel HMI, a component still to be planned
  (`docs/veredas-plan.md`, "A panel HMI").

### The IO check from a laptop
With no controlador target set, a light controlador session holding only the logic to drive a
remote IO module runs on the laptop itself, with timeouts never above 2 s and with known devices
only. The operator kept it on 2026-11-15, under three rules:
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
- **A test that does not drive the real interface proves no clause.**

## Before the code
- **One own-shell spike before this plan commits to its framework** (decided 2026-11-13): open a
  project folder and show the tree; one TOML file as form and text with a round trip that keeps
  comments; a million-row table from a Rust model; a Monaco tab with a language server on an ORCL
  file; a supervisão page embedded and read back by a test; a run on each GPU-less virtual machine
  (Windows on Hyper-V, VMware and VirtualBox, and Linux guests); one headless test that drives the
  interface.
- **Repository changes owed with the first code:** `crates/veredas-oficina` at core's MAJOR,
  listed; its row in `MAP` (`tools/leis`), a contract change written with the crate (portal at
  least, and controlador if the laptop IO check's session is linked in);
  in `docs/design/README.md` (which holds the accent's tones since 2026-11-19) the syntax
  palette's number colour
  (`#D2A8FF`) moves, sitting too close to the accent; licence files (Q35); P-015 becomes clauses.

## Open
- **Q31, the review's majors and minors**, none ruled. Those that touch Oficina: 22 (Oficina
  starting portal sessions against Q19, and SQL beside a session that writes the store), 28
  (Monaco has neither a language-server client nor a debugger client), 29 (Spix cannot see inside
  the web page; docking across monitors on a headless screen), 30 (no schema a form can be built
  from; the order of the surfaces), 31 (two scene models kept in step), 32 (opening a project must
  run nothing), 33 (loopback and a tunnel are not a boundary), 34 (text from the plant shown as
  plain text), 42 (opening the project from the target; pairing in one step), 43 (the outline's
  contrast; no light theme), 45 (exports, examples, help), and the minors 51, 52 and 54.
- **Q35** the licence expression. **Q36, Q37** the download's integrity and the robust link.
  **Q43** the security round. **Q45** the briefing's readings, among them where "identify, name
  and address" runs, on the laptop or through a target.
- The research behind these decisions (five PLC suites' manuals; three toolkits, Qt 6.12, Avalonia
  12.1 and Slint 1.18; studies of Qt without a GPU, of cxx-qt and KDDockWidgets, of Qt Creator as
  a platform, of softPLC debug and online-change mechanisms and of fieldbus master stacks) was
  read in the assistant's research runs. Their reports are not in this repository.
