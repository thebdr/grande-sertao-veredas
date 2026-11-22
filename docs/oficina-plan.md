# veredas-oficina: the plan

The workshop of Veredas 2; screens say Oficina. `docs/veredas-plan.md` ("veredas-oficina: the
workshop") wins on intent; `.zen/contract.md` (P-015) wins on status. Nothing is built. This file
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
  MIT or Apache 2.0, and a check of dependency licences runs in CI (the architect, 2026-11-15;
  which expression is Q35).
- **Every capability's test drives the real interface**: Rust tests through the Spix tool.
- Qt Creator was dropped as a base and is kept as a reference.

### How it reaches a target
- **Over SSH.** The components on a target keep listening on the loopback. Oficina logs in by SSH,
  over the same LAN or a VPN, starts and stops the components' executables, copies files, and
  carries their interfaces inside the connection. No accounts of its own (Q20 stands).
- **Portal to portal.** Oficina embeds a portal, and that portal talks to the portal embedded in
  controlador (the architect, 2026-11-15) over portal's own HTTP interface, "http (https, later)"
  (2026-11-16): the event stream, the windowed reads, the gated write surface. The interface
  answers on the controller's loopback only.
- **Commands travel on the same link** (2026-11-16): forcing, breakpoints, downloads, start, stop
  and acknowledge. Portal carries them and controlador handles them behind its portal, where the
  commissioning switch is checked (`docs/controlador-plan.md`).
- **Everything that alters the controller is robust**: "every bit of data that alters the
  controller shall be robust, that includes commands such as variable writes, breakpoints setting,
  state change" (the architect, 2026-11-16). A download carries a hash, "so no corrupted program
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
only. The architect kept it on 2026-11-15, under three rules:
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
  interface. How each point is judged, and the spike's verdict, are below ("The own-shell
  spike").
- **Repository changes owed with the first code:** `crates/veredas-oficina` at core's MAJOR,
  listed; its row in `MAP` (`tools/leis`), a contract change written with the crate (portal at
  least, and controlador if the laptop IO check's session is linked in);
  in `docs/design/README.md` (which holds the accent's tones since 2026-11-19) the syntax
  palette's number colour
  (`#D2A8FF`) moves, sitting too close to the accent; licence files (Q35); P-015 becomes clauses.

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
  KDDockWidgets 2.5.1 (its Qt Quick side), Spix 0.14 over AnyRPC 1.0.2, Monaco 0.57.0,
  rust-analyzer from rustup, CMake 4.5.1 for the C++ libraries. How the machine gets Qt is the
  architect's (the measured routes are in the evidence). Everything is built on Linux here; the
  Windows runs are asked for through the orchestrator.

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

### Results so far
On this machine, Qt 6.12.0 on its `offscreen` platform, through Spix, and again on X11 (below);
the numbers and what was learnt on the way are in the evidence. A test run names its platform with
`OFICINA_TEST_PLATFORM` (`offscreen` when unnamed).

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
3. **Passed** (2026-11-22), but for its frame times. A table of 1 000 000 rows of 8 columns, each
   cell computed from its row when the view asks: open in about 40 ms, its last row drawn in 22 to
   28 ms after the jump, every cell of it the model's own, and the process's memory with a million
   rows within 0.2 MB of its memory with a thousand (release, three runs). The scrolling frame
   times, with a GPU and without, are taken with point 6, where the drawing paths are set.

On X11 too (2026-11-22, once the architect installed the seven libraries Qt's X11 plugin needs):
the same tests passed on Qt's `xcb` platform, through this machine's display (the desktop's
XWayland, OpenGL 4.6 on its Intel GPU), and the timings held in release (a million rows open in
47 to 63 ms, 100 000 files in 154 to 226 ms). Windows waits for its runs.

### Verdict
Not given: points 4 to 7 remain, and point 3's frame times.

## Open
- **Q31, the review's majors and minors**, none ruled. Those that touch Oficina: 22 (Oficina
  starting portal sessions against Q19, and SQL beside a session that writes the store), 28
  (Monaco has neither a language-server client nor a debugger client), 29 (Spix cannot see inside
  the web page; docking across monitors on a headless screen), 30 (no schema a form can be built
  from; the order of the surfaces), 31 (two scene models kept in step), 32 (opening a project must
  run nothing), 33 (loopback and a tunnel are not a boundary), 34 (text from the plant shown as
  plain text), 42 (opening the project from the target; pairing in one step), 43 (the outline's
  contrast; no light theme), 45 (exports, examples, help), and the minors 51, 52 and 54.
- **Q341** (ábaco's, not ruled): its recommendation that Oficina link the grid's
  drawing-independent logic in its own process and paint it, the worker serving the model, which
  would put ábaco in Oficina's `MAP` row. **Q280** (aranha's, not ruled): aranha keeps no interface
  of its own, its surfaces (the device table, the topology, the diff, identify, name and address)
  being Oficina's, as the bench's are.
- **Q35** the licence expression. **Q36, Q37** the download's integrity and the robust link.
  **Q43** the security round. **Q45** the briefing's readings, among them where "identify, name
  and address" runs, on the laptop or through a target.
- The research behind these decisions (five PLC suites' manuals; three toolkits, Qt 6.12, Avalonia
  12.1 and Slint 1.18; studies of Qt without a GPU, of cxx-qt and KDDockWidgets, of Qt Creator as
  a platform, of softPLC debug and online-change mechanisms and of fieldbus master stacks) was
  read in the assistant's research runs. Their reports are not in this repository.
