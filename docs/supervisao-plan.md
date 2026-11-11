# veredas-supervisao: the plan

The web interface of Veredas 2; screens say **Supervisão**. `docs/veredas-plan.md`
("veredas-supervisao: the web interface", decision 9) wins on intent; `.zen/contract.md` (P-008,
then the `C-xxx` clauses it became) wins on status. This file says what supervisão holds, what it
took from Veredas 1 and what it left there, in which order it lands, and the traps met on the way.
The look is `docs/design/README.md` and `docs/design/supervisao/`.

## The role nobody covers
**Supervisão is the one place a person watches and acts on a running plant: it starts a plant's
portal sessions, binds their live values to the objects of a 2D layout and of dashboards by one
set of object IDs, and offers the plant's writes only through portal's gated surface.**

No other component answers this: portal reads, stores and serves the plant's data and gates its
writes, but draws nothing (Q14: its executable is its shell, no UI); comissionador runs checks and
reports; ábaco computes on workbooks; the bench and the scanner are test and setup tools. Without
supervisão every app would draw its own plant, as Veredas 1's three screens (the dashboard inside
the gateway, the desktop, the supervision app) each did.

## Decisions it builds on
- **Decision 9** (the plan): 2D layouts in SVG, 3D in glTF, one set of object IDs; configuration is
  plain text, never binary; 3D models are assets it points to.
- **Q18, decided** (the operator, 2026-11-09): Veredas 1's parsing of one company's engineering
  exports and control-software project files is **not carried, at all**. The plant model comes only
  from plain-text TOML a person (or anything else) writes: the binding file, and the SVG layout it
  points to. No export reader is planned in the suite or beside it.
- **Q19, decided** (the operator, 2026-11-09): **supervisão starts the sessions.** It runs each
  plant's portal session in its own process from the plant's project file, through portal's library
  (`project::load_file`, `Session::start`), so one executable serves everything; it owns no plant
  connection of its own (the map, L5: supervisão uses portal; every value arrives through portal).
- **Q20, decided** (the operator, 2026-11-09): **local, no accounts.** The host binds the loopback
  by default; everyone who reaches it sees the same; a write rests on portal's switch (armed by the
  project, never over the wire) and on the actions the binding file declares.
- **Q21, decided** (the operator, 2026-11-09): **3D later**, after the 2D layout, the dashboards,
  the gated writes and the sessions are verified. The binding file's object IDs are the ones a glTF
  model's nodes will carry; no 3D key exists until the 3D view does (a key that does nothing is a
  lie, and adding one later is a MINOR bump of the binding format).

## Scope
- **The host** (`host`, `main.rs`): one executable, `veredas-supervisao <supervisao.toml>`, reads
  the supervisão file (`veredas-supervisao` v1: the address it binds, and the plant sessions, each
  a name, a portal project file and a binding file), checks every file whole before anything
  starts, starts every session through portal, serves them side by side, one path per session
  (`/s/<name>/`), and stops them all when it stops (`--check` reads, checks and prints the plan
  without starting anything).
- **The binding file** (`binding`; `docs/binding-schema.md`, `veredas-binding` v1): a session's
  objects by ID, each bound to a portal series (a source, a series, an optional key) with its unit,
  its state rules, how the layout shows it (a state, a level, a value, a marker moving along a path)
  and the writes it offers; the SVG layout it points to; the headline figures; the dashboards and
  their panels. Checked against the session's project when it loads (every source named is the
  project's; every write an OPC tag a rule marks writable) and against the SVG (every move path is
  an element of it).
- **The object feed** (`feed`): per session, every object's live value, its state (a class with a
  mark and a word, by the first rule that holds), stale when its source is away or its value
  older than the object says, and a marker's place along its path (one per key of a keyed series),
  evaluated in Rust from portal's live frames and caches and served as a stream and a snapshot.
  The browser draws; it decides nothing.
- **The dashboards** (`panel`): each panel computed in Rust from the session's store (the windows
  and pages portal's server serves) into exactly what the page draws: a value tile in four states,
  a trend with its limit and breaches, bars per interval against a target with the running one
  marked, state over time per object, a state grid, levels against ticks, a table with inline bars
  and state words, and charts in cells (per key a median, its sparkline, a distribution on one
  scale for every row, σ and n).
- **Portal's surface, per session**: each session's own portal router (`veredas_portal::serve`)
  nested at `/s/<name>/portal/`, unchanged: the live stream, the windowed REST, the tables, and
  the gated writes. The Tables and Series views read it; the page's write buttons post to it.
- **The web interface** (`web/`, embedded): TypeScript built by vite into `web/dist/`, committed,
  and compiled into the crate, so `cargo build` and CI never need Node. The top bar (wordmark,
  session, view tabs, headline figures, live clock), the layout view (the SVG bound by object ID,
  layer toggles, legend, zoom, the side panel with the selected object and the "needs attention"
  list, the alarm strip), the dashboards, the tables and series views.
- **Later, named, no code now:** the 3D view (Q21); editing the binding in the browser (the
  config file is the editor until then); reloading a binding file when it changes on disk; the
  tracking-accuracy object of the design (its definitions are open, `docs/design/supervisao/`);
  starting, stopping or adding a session from the browser; the grep tool over logs at rest and the
  skew monitor that Q14 left waiting; a session's own `[server]` address (supervisão serves the
  session's surface under its own address and binds no other); the dashboards' charts
  interactive, zoomed in and out on the page, and each chart's data exported as CSV (the
  operator, 2026-11-11, from a demo run).

## Rules
- **Every value arrives through portal; every write rides portal's gated surface.** Supervisão
  holds no plant socket, no OPC client and no store of its own: it reads a session's frames, caches
  and store through portal's library, and its page writes by posting to portal's own write routes
  under `/s/<name>/portal/`, where the switch and the whitelist answer. There is no write path in
  supervisão's code; the switch is armed by the project, never over the wire.
- **The plant model is configuration.** A binding file and an SVG, plain text, written by a person
  or by anything; no export, workbook or vendor project format is read (Q18). The layout's geometry
  is the SVG's, never computed from a plant's data.
- **Content-agnostic** (Q8): no device, vendor, product or customer named in code, comments, tests,
  fixtures or the built bundle; the sample plants are invented and named for their shape. The
  component test reads every file git holds or could hold under the crate, `web/dist/` included.
- **The browser decides nothing.** States, staleness, positions and every panel's numbers are
  computed in Rust and tested there with exact values; the page maps what it receives onto the SVG
  and the panels, and is verified by driving it (`zen-verify`), never by a unit test alone.
- **Every state has a mark and a word** (the design's rule): normal ●, near ■, warn ■, alarm !,
  disabled ▨, stale ◌, selected ◆. A rule names the class and the word ("full", "stopped",
  "fault"); the accent (Veredas' green `#30D158`, Q22) marks selection, focus and primary actions, never a
  state. Writes are shown gated (the warn colour) wherever they appear, and disabled while the
  session's switch is off.
- **Staleness is judged on the arrival clock.** An object is stale when it has no value yet, when its source
  is away (idle past the project's window, retrying, ended or failed, or waiting again once it has
  delivered, as portal reads a reconnect; a source waiting before its first delivery is not away,
  its health lagging its first value by a refresh), or when the object names a
  freshness bound and no sample came within it. A quiet plant
  storing on change is not stale (Veredas 1's lesson: on-change storage makes a healthy plant look
  dead when staleness reads sample age).
- **Every file is TOML stating `format` and `version`** (Q10, L7), read by core's one header reader
  (`veredas_core::schema`); a misspelled key refused; an unknown version refused.
- **Fixture first: the bench is the plant.** Every clause is proven against portal sessions fed by
  `veredas-maquinar` (its OPC UA server, its log emitters) and by stamped logs the test writes, over
  real sockets and real HTTP; plant motion the layout needs (positions along a loop, levels filling)
  is the bench's first (its OPC tags' `ramp`, C-026).
- **One test file per capability** under `tests/`, so each clause links one file.
- **Typed errors in library code**, no `unwrap`/`expect` on an I/O path; tests may unwrap.

## What it absorbs from Veredas 1
Read for this plan (read only): Veredas 1's `crates/veredas-supervisao` (server, proxy, project,
wizard, the model compiler), `supervisao/` (the TypeScript frontend), `dashboard/` (the gateway's
dashboard), and its `docs/supervisao-plan.md`.

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| the server's shape (axum, the embedded `dist/`, the model route) | `host`, `web` | one executable serving the page and its data |
| `proxy.rs` (a hand-rolled HTTP pass-through to the gateway, one whitelisted POST) | not carried (Q19) | the sessions run in the process; portal's router is nested as it is, so its writes are portal's own routes |
| `project.rs` (seven JSON files under `supervisao/`), `prefs.rs`, the launcher-file registration | `config` (the supervisão file), `binding` | one TOML host file and one TOML binding file per session; no per-user preferences file, no registry entries |
| `model/*` export readers, `genrules.rs`, `origins.rs`, `registry::type_dir_for`, the vendor half of `compile.rs`, the drawing correlation | not carried (Q18) | parsing one company's exports is not suite code, nor beside it |
| the model's ideas: stable object IDs, provenance shown, unknown shown as absence, a report with stable codes | `binding`'s checks | a refused binding names its place; an object never bound shows no value, never a guess |
| `live.ts` (frames folded into a last-value map, primed from the listing, health polled) | `feed` (in Rust) | the same feed, evaluated where it can be tested exactly |
| `colours.ts` (condition apart from colour, dropped on stale) | `feed` states, the design's vocabulary | a class with a mark and a word; stale overrides colour |
| `strips.ts` (SVG generated from the model, mutated, never rebuilt) | `web` layout view | the SVG is the operator's file; it is bound, never generated; mutated, never rebuilt |
| the drawer and popup (selected object, live words, documents) | the side panel | the selected object, its value, its sparkline, its gated writes |
| `threed/` (a Canvas2D orthographic renderer of bodies built in code) | later, as glTF (Q21) | the 3D view draws assets the binding points to |
| the wizard (build mode, placements, overrides, bindings) | later (browser editing) | the binding file is the editor for now |
| `dashboard/` (panels `line`, `bar`, `series`, `table`, `stat`, `button`, `input`; a layout JSON posted to the gateway; uPlot; the time span) | `panel` and the dashboards view | panels are TOML in the binding file and their numbers are computed in Rust; the charts are drawn as plain SVG; the `button`/`input` writes are the objects' declared actions |
| the dashboard's layout endpoint (`/api/layout`) and command sequencer | not carried | portal carries neither; a dashboard is the binding file's |

## The web interface
- `web/` holds the TypeScript sources, `index.html`, `package.json`, the vite and TypeScript
  configuration, and `dist/`, the built bundle, committed. The build writes fixed names
  (`dist/index.html`, `dist/assets/app.js`, `dist/assets/app.css`) and two hashes (line endings
  normalised): `dist/source.hash` over every source the bundle is built from, `dist/output.hash`
  over the three files as written; the crate compiles the three files in with `include_str!`, so
  no embedding crate and no build script is needed. The hashes are written once the bundle is on
  disk, never by a failed build.
- The build reads nothing beside its sources (no `.env`, no `public/`, no PostCSS file; a module,
  or a file a stylesheet pulls in, outside `src/`, the hashed files and `node_modules/` refused)
  and emits the three files alone (a second chunk or an asset refused, none inlined):
  `vite.config.ts` holds the guards, and the component test holds `vite.config.ts` word for word.
  The hashes are over bytes (CR LF read as LF), so a source that is not text counts byte for byte.
- The component test recomputes both hashes, the first from the sources in the tree, the second
  from the page compiled in, and holds every file under the crate to Rust, a hashed source or the
  page: an edit to `web/src` without a rebuild, a page edited by hand, or a new file the build
  could read is red on every machine, Node or not.
- Node and npm are needed only to change the page (`npm ci`, `npm run build` in `web/`); the crate,
  its tests and CI never run them. `web/node_modules/` is never committed.
- The tests run the page itself in a headless Chromium (Edge on Windows, Chrome or Chromium
  elsewhere, the one `VEREDAS_BROWSER` names first), driven over the DevTools protocol through a
  WebSocket the tests write in a few lines, and read back what it drew. The suite needs that
  browser as it needs a socket: a missing one fails, never skips. GitHub's Ubuntu runner carries
  Chrome.
- What the page adds to the drawing for an object carries the object's id in `data-for` (a
  marker's tag its key in `data-key`), so the drawing is read by id, as it is bound.
- The charts are SVG drawn by the page's own code: the objects are small and specific (dashed
  limits, breach dots, state bars, distributions on one scale), and a chart library would be one
  more thing in the bundle to keep content-agnostic.

## Module map
```
veredas_supervisao
  config      the supervisão file (veredas-supervisao v1): bind, sessions (name, project, binding)
  binding     the binding file (veredas-binding v1): objects, rules, layout, headline, dashboards;
              checked against the project and the SVG
  svg         the layout asset read as XML: its element ids, the geometry elements a marker can follow
  state       the state vocabulary and the rules: a value into a class, a mark and a word
  feed        a session's objects live: values from portal's frames and caches, states, staleness,
              markers along paths; a snapshot and a stream
  panel       the dashboards' panels computed from the session's store
  host        sessions started from the supervisão file, served side by side under /s/<name>/
  web         the embedded page (index.html, app.js, app.css) and its source hash
  main.rs     veredas-supervisao <supervisao.toml> [--check]
web/          the page: TypeScript sources, vite configuration, dist/ committed
```

Routes (all under one address; `<name>` a session):

| Route | What |
|---|---|
| `GET /` | the page (it lists the sessions and opens the first) |
| `GET /assets/app.js`, `/assets/app.css` | the page's bundle |
| `GET /api/sessions` | the sessions: name, title, path |
| `GET /s/<name>/` | the page, opened on that session |
| `GET /s/<name>/binding` | the binding as the page needs it (objects, headline, dashboards, the layout's name) |
| `GET /s/<name>/layout.svg` | the SVG asset, as written |
| `GET /s/<name>/objects` | the object feed: an SSE stream, a snapshot then changes, the session's switch and health |
| `GET /s/<name>/objects.json` | the object feed's snapshot |
| `GET /s/<name>/dashboard/<index>` | a dashboard's panels, computed (`to` optional: the window's right edge on the store's axis) |
| `/s/<name>/portal/...` | the session's portal router, nested unchanged (C-044) |

## Phases
1. **The plan and the files.** This plan, `docs/binding-schema.md` (the supervisão file and the
   binding file, each worked example loaded by its test), Q18–Q21 recorded, P-008 promoted.
2. **The crate and the host.** `crates/veredas-supervisao` at `2.0.0-alpha.1`, listed; the
   component clause (a library and one executable, content-agnostic over every file git holds or
   could hold under it, `dist/` matching its sources, core and portal alone); the host starting
   sessions from the supervisão file and serving each under its name, portal's surface nested;
   several side by side.
3. **The bench's motion.** The OPC server's `ramp` (C-026 grows): a value stepping through a range
   on an interval and wrapping, for markers that move and levels that fill.
4. **The object feed.** States, staleness, markers, against a session fed by the bench and by a
   log the test writes.
5. **The 2D layout.** The page binds the SVG by object ID: states with a mark and a word, levels,
   markers moving, the side panel, the attention list, the alarm strip; verified in the browser.
6. **The dashboards.** Every panel kind computed exactly in Rust and drawn by the page; verified
   in the browser.
7. **Tables and series.** The views over portal's nested REST.
8. **Writes, gated.** Actions declared in the binding, shown gated and disabled while the switch
   is off, refused 403 through portal's surface on the wire with nothing reaching the plant; armed
   by the project, landing at the bench.
9. **Later:** the 3D view (Q21), browser editing, live reload of the binding file.

Each phase ends with `cargo nextest run --workspace` green bare, `cargo test -p leis` green, and
its clauses recorded, refuted and verified. Push at the end of a phase green end to end.

## Traps
Recorded as the port meets them. Carried from Veredas 1's supervision app, where they apply here:

- **Staleness by sample age makes a quiet plant look dead.** Storage on change sends nothing while
  nothing changes; judge staleness by the source's health on the arrival clock, and drop the colour
  of a stale object rather than hold it.
- **Mutate, never rebuild, the SVG.** Rebuilding the layout on every frame loses the selection and
  the zoom and costs a frame per update; the page binds the elements once and changes attributes.
- **Priming at scale freezes a page.** The feed is seeded from portal's series cache on the server,
  never by the browser walking every series.
- **Naming literals shared across components must be pinned by tests, or nothing lights up.** A
  binding's sources and writable tags are checked against the project when it loads.
- **Scope creep in 3D**, and the pressure to make a screen look complete: a screen shows what its
  bindings carry, and an unbound object shows no value.

Met during the port:

- **Waiting is also a reconnect.** Portal reads an OPC source reconnecting and a channel whose peer
  left as waiting, the state of a source before its first value; staleness reads waiting as away
  only once the source has delivered (C-050, found in the browser with the plant stopped).
- **An arrival is not a change.** Every poll refreshes an object's arrival; a change published on
  it would send every polled object every tick. A change is anything but the arrival (C-050).
- **The health lags the first value** by one refresh (500 ms): an object whose source still reads
  waiting while its value arrived is not stale (C-050).
- **An element replaced between a press and its release loses the click.** The top bar and the side
  panel are built once and updated in place; the attention list is rebuilt only when it changes.
- **A transition is for a step, not a first place.** Markers slid in from the drawing's origin and
  levels grew from empty on load until the first paint was made instant.
- **The workspace's resolve is not a package's.** Once supervisão used portal's default features,
  `cargo metadata` unified them into portal's node and portal's light-build test found the engine;
  a build of one package alone is asked of `cargo tree -p` (C-034).
- **A running demo holds its executable.** Cargo cannot relink a binary a bench or supervisão demo
  is running; stop the demo before recording a clause whose test builds it.
- **Verify a page at the browser pane's own size.** Pointer clicks were mis-mapped under an
  emulated viewport scaled down to fit; screenshots also lag the page by a frame or two, so read
  the page back by script as well.
- **The refute target's path must be short.** The bundled database engine's build overflows
  Windows path lengths under the session's scratch folder; the critic's target lives at
  `%TEMP%\vs-target`, and its first build takes over an hour.
