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
- **Repository changes owed with the first code**, as they stand since the crate began
  (2026-11-25): `crates/veredas-oficina` at core's MAJOR, listed (done, C-250); P-015 becomes
  clauses (C-250 and C-251 so far). Its row in `MAP` (`tools/leis`), a contract change (portal at
  least, and controlador if the laptop IO check's session is linked in), comes with its first edge
  to portal: law L5 lets a component outside the map use core alone, and the first code does. In
  `docs/design/README.md` (which holds the accent's tones since 2026-11-19) the syntax palette's
  number colour (`#D2A8FF`) moves, sitting too close to the accent: with the code editor's clause,
  the first code that draws it. Licence files wait for Q35 (taken meanwhile: none).

## The crate
`crates/veredas-oficina`, from 2026-11-25 (C-250): a library whose model builds on any machine the
suite builds on, and the window behind two features, neither on by default, so no member's build
needs Qt (docs/questions.md Q370, taken meanwhile).
- **The model**, no Qt: `tree` (the project tree) and `formats` (which component reads each
  format). Its members are core alone, without core's default features: the header reader, with
  none of core's plant-facing layers (C-073).
- **`shell`**, the window and the executable `veredas-oficina`: Qt Quick, its logic in Rust through
  cxx-qt, the seam a private module (`src/shell`) with one C++ file beside it (`cpp/shell.cpp`),
  KDDockWidgets docking. It needs `QMAKE` naming Qt 6.12's qmake in cargo's environment (cxx-qt's
  own build scripts read it, not only Oficina's) and `OFICINA_DEPS` (`~/.local/opt/oficina-deps` by
  default) holding KDDockWidgets.
- **`driver`**, the tests' driver: Spix's server inside the window, started only when
  `OFICINA_SPIX_PORT` names a port; Spix and AnyRPC in `OFICINA_DEPS`. Only the tests build it:
  what ships carries no test server.
- **Its tests.** A clause's test drives the window through Spix, on Qt's `offscreen` platform
  unless `OFICINA_TEST_PLATFORM` names another (`xcb` for X11, `windows`):
  `QMAKE=$HOME/.local/opt/Qt/6.12.0/gcc_64/bin/qmake cargo nextest run -p veredas-oficina --features driver`.
  The model's own tests (`tests/tree_model.rs`) run in the suite's default build and prove no
  clause. The whole-suite run holds the window's tests with `--features veredas-oficina/driver`
  and `QMAKE` set.
- **Brought from the spike**, as its verdict kept them: the project tree (its model and its bridge),
  the formats' table, the window's docking and the test driver. The TOML document, the code editor
  and its language-server client, the table, the embedded pages and the drawing probe come over
  with their clauses; the spike's folder stays as its record.

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
  Build Tools 17.14.45, Git 2.56.1, uv 0.13.9 running aqtinstall's master at 4c75985):
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
