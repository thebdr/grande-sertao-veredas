# veredas-abaco: the plan

The engineering spreadsheet of Veredas 2; screens say Ábaco. `docs/veredas-plan.md`
("veredas-abaco: the engineering spreadsheet", decision 4) wins on intent; `.zen/contract.md`
(P-007, then the `C-220`… clauses it becomes) wins on status. This file says what Ábaco holds,
what it takes from Veredas 1 and what it leaves there, how the laws shape the port, in which order
it is built, and the traps met on the way. Written 2026-11-22, before any code (law L6).

## The role nobody covers
**Ábaco computes and edits the engineering workbooks the suite works from, with no Office on the
machine: a formula engine measured against Excel's own results, edits that leave every untouched
byte of a workbook as it was, and Python for what a formula cannot do.**

No other component answers this. Portal reads the plant and stores what it reads; it never opens
a workbook. Supervisão draws a plant from its binding file and reads no engineering export (Q18).
Comissionador checks plant data against tests. Oficina shows the grid as a tab (A6.1 of
`docs/briefing/deliverables.md`) but computes nothing: the engine and the Python of each workspace
run in a process of Ábaco's own. Without Ábaco every workbook the engineering of a plant lives in
goes back to Excel on a Windows desk.

## Scope
- **The engine:** a fork of IronCalc (Rust, MIT OR Apache-2.0) modified for engineering work:
  references to other workbooks kept as written and resolved by a policy, `TEXT` over numbers held
  as text, user-defined functions through a host hook, and the file-layer fixes below. Accuracy is
  measured against the values Excel cached in workbooks it saved, never claimed (the oracle).
- **Links to other workbooks:** resolved from the cache the file carries (the default, Excel's own
  behaviour with the linked book closed), from the linked workbook opened beside it, or asked of
  the person; under a linked resolution, one log line per cell whose live value differs from the
  cache.
- **Byte-faithful edits** of a workbook Ábaco did not write: values, formulas, rows and columns,
  sheets, names, widths and outlines; references re-spelled wherever they point; every part no edit
  touched comes out byte for byte; a three-way check against what was read, an atomic write with a
  backup, the source read again before the rename.
- **The `.abaco` workspace file:** a workbook born in Ábaco, and at the same time the session that
  works on other files: one zip of plain-text parts (sheets, macros, the references, decisions,
  queue and log of its tools), readable with `unzip` and an editor; its export to `.xlsx`.
- **Python:** macros that drive the model through the `abaco` module, custom functions callable
  from a formula, CPython embedded in the executable, macros found nearest first (the workspace,
  the person's library, the built-ins); one worker process per workspace, so a hung macro takes
  down its workspace and nothing else (A6.1).
- **Extensions**, inside their host and no member: `acerto` first, if it is carried (Q343).
- **What Oficina draws** (A6.1: the grid as a tab, the Python editor, run with stop-on-error, a
  debugger, forms): Oficina's surfaces, standing on what Ábaco's worker serves. What of the grid's
  behaviour Ábaco carries is Q341.

Not in scope: drawing anything (Oficina's); a plant connection (a sheet that reads plant data does
so through portal, the row L5's map already holds); Excel as a renderer (Q342).

## Rules
- **Excel is the reference.** A formula's right value is the one Excel cached for it. Parity is a
  number the oracle measures over workbooks Excel saved; a gap is counted and classified, never
  waved through.
- **A foreign workbook is patched, never rewritten.** The fork's writer regenerates every part
  (Veredas 1 measured it: no part of five real workbooks survived its save); it is the `.abaco`
  exporter only, never the save path of a file Ábaco did not write.
- **A formula is spelled for the file** (`_xlfn.`, `_xlpm.`). Veredas 1's rule that a new formula
  shape is done only once a real Excel opened a file Ábaco wrote stands; no Excel runs on this
  machine, so that acceptance is owed on one that has it and each clause says which it owes.
- **Everything visible.** The workspace file is plain text inside a zip; a failure names its cell,
  row or file; the log is text a sheet can show.
- **Plain text is TOML and states its schema** (the plan, "Versioning"; law L7): the `.abaco`
  file's parts, the preferences, an edit file. Read through core's header reader
  (`veredas_core::schema`), a newer version refused, never guessed; while core carries its
  network layers, Ábaco compiles that reader's own source by path (Q345). A key nothing reads is
  refused at any depth.
- **The engine's changes live in the fork**, one upstream-shaped commit each, listed in
  `forks/ironcalc/VEREDAS-FORK.md`; Ábaco's own crate adds policy, never engine
  behaviour, so each change can go upstream on its own.
- **Opening runs nothing.** Opening a workbook or a workspace runs no Python. Veredas 1 loaded a
  home's `functions` macro at open; here that waits for the security round's entry 32 (Q43): until
  it is ruled, custom functions load on the person's command.
- **No customer, site or vendor named.** This repository is public and Veredas 1's is private:
  nothing carried names a plant, a customer or a company's file kinds, and no workbook of a real
  plant is committed. The real workbooks are measured from a path outside the repository and only
  their numbers and hashes are recorded.
- **Typed errors** on the public surface; no `unwrap`/`expect` on an I/O path; tests may unwrap.
- **Evidence is reality:** real workbooks Excel saved, opened, evaluated, patched, saved and read
  again; a mock is never the evidence of a clause. One test file per capability, so a change to one
  re-stales only its own recorded run.

## How the laws shape the port
- **L1, roots.** One member, `crates/veredas-abaco`, and one tool, `tools/abaco-oracle`. The fork's
  two crates (`ironcalc_base`, `ironcalc`) sit at the repository's root, in `forks/ironcalc`, and are
  excluded by the root manifest: they are not the suite's crates, so L1 lets them out of the
  workspace, and they are no `veredas-*` or `orcl-*` crate, so nothing of the suite dodges the laws
  that way. They cannot sit inside the host's folder, as this plan first said: cargo makes a crate
  beneath a listed member's folder a member whatever `exclude` says (Traps). Where the fork lives
  for good is Q340.
- **L2, naming.** Veredas 1's `veredas-abaco-py` is no component name (its `nome` would hold a
  hyphen): the Python half folds into `veredas-abaco`, behind a cargo feature, with the executable.
  Veredas 1's `veredas-abaco-grid` is not carried as a crate (Q341). `acerto` is a Python package
  inside its host, with no cargo manifest, so it is no member (L2's rule for an extension).
- **L3, versions.** `veredas-abaco` starts at `2.0.0-alpha.1`, core's MAJOR. The tool and the fork
  version as they like; the fork keeps upstream's numbers (`0.9.1`), so a diff against upstream
  shows only the changes.
- **L5, direction.** The map already lets Ábaco use core and portal. It depends on no member yet:
  core arrives once its header reader can be had without its network layers (asked of core,
  2026-11-23, Q345); until then the `.abaco` file compiles that reader's source by path. Portal
  arrives only with a sheet that reads plant data. The oracle, a tool, may depend on anything, and reads its baseline through
  core's header reader.
- **L6, plan first.** This file, committed before the crate.
- **L7, stamps.** Every TOML part Ábaco writes states `format` (`veredas-abaco` and its parts'
  `veredas-abaco-<part>`, `docs/abaco-file.md`; `veredas-abaco-prefs`, `veredas-abaco-edits`)
  and `version` from 1. The fork's two crates hold no
  TOML file but their manifests; upstream's bindings, which do, are not carried.

## What it absorbs from Veredas 1
Veredas 1's Ábaco (`/home/thebdr/code/Veredas`, branch `supervisao-plan`, its
`docs/abaco-plan.md` and `docs/abaco-oracle.md`) was read in full before this plan. Its rulings AB1
to AB16 were the architect's; those that stand are carried as rules above, those the suite changed
are named below.

| Veredas 1 | Veredas 2 | Why |
|---|---|---|
| `crates/ironcalc`: `base` and `xlsx`, upstream `15ecfec` (2026-11-02) plus fork changes 1 to 11 | `forks/ironcalc/{base,xlsx}`, excluded from the workspace (Q340, taken meanwhile) | the engine |
| `crates/ironcalc`: the Python, WebAssembly and Node bindings, the web app, upstream's docs, the language and locale generators | not carried | the suite builds the engine and its file layer only; Ábaco's Python is its own (AB2) |
| `veredas-abaco` `links`, `linkdiff` | `veredas_abaco`, phase 1 | the link policy the oracle opens every workbook through |
| `veredas-abaco` `a1`, `workbook` (its model half) | `veredas_abaco::a1`, `book`, phase 2 | the model the patcher and every later phase stand on; its workspace half comes with the `.abaco` file |
| `veredas-abaco` `prefs` (the link policy and the renderer as a preference) | phase 4, the link policy alone, in stamped TOML | a preference is the worker's; the renderer is Q341's and Q342's |
| `veredas-abaco` `patch` | `veredas_abaco::patch`, phase 2; edits through the API (an edit file arrives with the command line) | byte-faithful edits |
| `veredas-abaco` `abacofile`, `workspace` | phase 3, its JSON parts read into TOML | the workspace file |
| `veredas-abaco` `homes`; `veredas-abaco-py` (module `abaco._core`, the `abaco` executable, its environment, console and command line) | phase 4, inside `veredas-abaco` behind a feature; the executable named `veredas-abaco` | L2; the plan names binaries `veredas-<nome>` |
| `veredas-abaco` `view`, `grid` (the renderer-neutral core: navigation, selection, the editor's modes, the clipboard's TSV, filters) | Q341 | the grid's surface is Oficina's |
| `veredas-abaco-grid` (egui) | not carried (Q341) | Oficina is the one native application |
| `veredas-abaco` `excel` (the COM host, Windows only) | Q342 | a renderer beside Oficina |
| `veredas-abaco` examples (the grid spike, cell and cache diffs, round trips) | not carried; what a phase needs becomes a subcommand of the oracle or a test | throwaway spikes and probes |
| `abaco-oracle` | `tools/abaco-oracle` | test tooling (P-007) |
| `python/veredas-acerto` | `crates/veredas-abaco/acerto/` if carried (Q343) | an extension inside its host (decision 4) |

Changed on the way, each recorded in its clause's evidence note:
- edition 2024 for Ábaco's crate and the oracle (the fork keeps upstream's 2021);
- JSON becomes stamped TOML: the preferences (`abaco.json` becomes `abaco.toml`, in
  `$XDG_CONFIG_HOME/veredas/` on Linux and `%APPDATA%\veredas\` on Windows), the `.abaco` file's
  manifest, workbook and workspace parts, the patcher's edit list (Veredas 1's JSON lines);
- the executable `abaco` becomes `veredas-abaco`; the Python import name stays `abaco`, the name
  macros and extensions already use;
- Veredas 1's window title (`abaco v…`, its brand rule) is not carried: Ábaco has no window, and
  Oficina's tab says Ábaco;
- custom functions are not loaded at open (Rules, "Opening runs nothing");
- the person who decides is the architect; Veredas 1's documents said "the operator".

## The fork
Veredas 1 imported IronCalc as a git subtree with its 2 222 upstream commits, at upstream `15ecfec`,
and made eleven changes on it, each one commit (its `VEREDAS-FORK.md`): external references
`[n]Sheet!A1` kept as written and resolved through a pluggable resolver (1); external links read and
written by the file layer (2); the linked resolver that opens the companion workbook (3); `TEXT`
over numeric text (4); the external cells a workbook reads, listed (5); serde on the model's
structure types (6); user-defined functions (7); inline strings decoding `_xHHHH_` (8);
`Sheet!#REF!` lexed as the error (9); a package without `xl/styles.xml` (10); column outline groups
(11). None was sent upstream.

Taken meanwhile (Q340): the fork arrives as Veredas 1 left it, `base` and `xlsx` only, as a copy,
not a subtree, at `forks/ironcalc`: one commit with upstream's files at `15ecfec`, checked against
a fetch of upstream at that commit, then each change as a commit of its own (twelve: change 1 had a
refinement), the result checked equal to Veredas 1's fork once its test data's real names are
replaced by neutral ones. Three changes of Ábaco's own followed, each found by a refute round: a
link's target read as the URI Excel writes (12), the statistical functions on one float path in
every build (13) and the chi-squared and gamma inverses to full precision (14), and a clock a host
can pin at run time (15), random functions a host can seed (16); and four found by the patcher's
refute round, where the engine's model disagreed with what Excel makes of a move: a range a
deletion cuts into shrinks (17), a table a host moves is read where it is (18), an external
reference keeps its address when its cell moves (19), a moved formula keeps its kind (20), and
the engine's static analysis open to a host (21). The
record lists each, with its upstream column. Upstream's own tests and the changes' (in the fork's folder, with
upstream's workbooks) run in the fork's own workspace,
`cargo test --locked --manifest-path forks/ironcalc/Cargo.toml`, and Ábaco's component test runs
them, so they run with Ábaco's tests. Upstream has moved since (`main` at `f936620` on 2026-11-22): bringing it in is
a step of its own, after parity, measured by the oracle before and after. A change goes upstream as
a patch from a clone of upstream; opening the pull request is the architect's act.

## The oracle and parity
The oracle reads, for every formula cell, the value Excel cached beside it straight from the sheet
XML (shared strings, rich text, shared formulas, `t="str"` handled), so the engine's own reader is
never trusted for the expected side; it cross-checks that reading against an independent reader;
then it opens the workbook through Ábaco under a link policy, evaluates it, and classifies every
difference (external link, unknown function, error for value, value for error, error type, number
beyond 1e-9 relative, text, boolean, empty against a value, number against text, spill, missing
sheet). Its report is plain text.

Veredas 1's numbers: on five real workbooks of a plant's engineering (two of about a million and a
hundred thousand formulas, each linked to a configuration workbook, the two configuration
workbooks, and an IO list), 1 217 533 formula cells, every one equal to Excel's under the cached
resolution and under the linked one (its `docs/abaco-oracle.md`, "Phase B1b"). Those workbooks
were never in Veredas 1's repository (they were copied from a cloud folder on the Windows desk)
and are not on this machine: Q344.

Taken meanwhile (Q344), the measure that can be made here: the corpus is every workbook among the
fork's own test files that Microsoft Excel saved, whose cached values are Excel's. The package says
so three ways, all required: `docProps/app.xml` names Excel on Windows, on the Mac, online or on
Android exactly, at version 14 or later, and `xl/workbook.xml` carries Excel's own file version
(openpyxl calls itself "Microsoft Excel Compatible", XlsxWriter claims an Excel of version 12). That
is 257 workbooks. The baseline is Veredas 1's own oracle and fork, built and run on this machine over that
corpus (`tools/abaco-oracle/baseline/veredas-1.toml`); Veredas 2 reaches parity when its numbers
are equal or better, file by file, under the cached and the linked resolution. Veredas 1 measured
256 of them, 75 743 of 76 245 formula cells equal to Excel's (99.34 %; the rest volatile dates,
numerical tails and functions the engine lacks); its oracle aborted on the 257th before the engine
ran. The corpus's three workbooks with links name workbooks that are not here, so it shows no
parity against linked workbooks read live: that waits for the plant's own. The five workbooks are run from a path outside the repository when the architect gives them,
and their numbers join the evidence.

## The module map
`crates/veredas-abaco` (a library; the executable arrives with the worker in phase 4):
- `links`, `linkdiff`: the link policy and the cache-against-live log (phase 1);
- `book`: a workbook opened, evaluated and read, cell by cell, value and formula (Veredas 1's
  `workbook`); `a1`: addresses and areas; `patch`: the byte-faithful patcher (phase 2);
- `abacofile`, `workspace`: the `.abaco` file and the session it holds (phase 3);
- `prefs`, `homes`, `py`, the worker and its command line (phase 4, the `python` feature);
- `acerto/`: the extension, if carried.

`forks/ironcalc` (not a member, excluded): the engine and its file layer.

`crates/veredas-abaco/tests/fixtures/no-styles.xlsx`: upstream's `arithmetic.xlsx` without its
styles part (Veredas 1's fixture, carried as it was).

`tools/abaco-oracle` (a library and its executable): the expected side, the comparison and its
classes, the report, the corpus list, and the parity test.

## The `.abaco` file (phase 3)
A zip of plain-text parts, readable with `unzip` and an editor; the reference, its keys and a
worked example, is `docs/abaco-file.md`, written with the code.

```text
manifest.toml                 who wrote it, the workbook's name, its sheets in order, its macros
workbook.toml                 styles, defined names, metadata, settings, theme, tables, links
sheets/01-<name>.toml         a sheet: its structure, then its cells, one line per cell
macros/<name>.py              the macros the workspace carries (never run at open)
workspace/references.toml     the foreign files it works on, each fingerprinted
workspace/decisions.toml      what its tools decided, oldest first
workspace/queue.toml          what its tools queued, in order
workspace/state/<tool>.toml   a tool's own state
workspace/log.txt             the log, one escaped entry per line
```

- **Every TOML part states its schema**: `format` (`veredas-abaco` for the manifest,
  `veredas-abaco-<part>` for the others, so a part in the wrong place is refused by name) and
  `version` from 1, read through core's header reader before the body; a newer version refused, an
  older one migrated (none yet).
- **One line per cell**, row-major, in the sheet's `[cells]` table, its key the address:
  `A1 = { n = 42 }`, `B1 = { s = "text", style = 3 }`, `C1 = { f = "A1*2", v = 84 }` (the value is
  for the reader, recomputed at open), `D1 = { array = "A1:A3*2", size = [1, 3] }`,
  `E1 = { dynamic = "SEQUENCE(3)" }`, `F1 = { style = 5 }`; a boolean `b`, an error `e`. A cell an
  array spills into is not written (it is computed), only its style. TOML's own strings escape
  text and formulas: Veredas 1's tab-separated lines and their escapes are not carried.
- **Numbers read back exactly**: a whole number under 10^15 written as an integer, any other as
  the shortest text that reads back as the same double.
- **Tools' records are TOML tables**: a decision, a queue entry or a state holds no null (TOML has
  none).
- **The engine's structures pass through JSON** (styles, theme, names, a sheet's structure): their
  none is JSON's null, written `nan`, which no JSON value holds; their keys come sorted.
- **What it does not know it keeps**: a part this Ábaco does not read is written back as it came,
  so a newer Ábaco's additions survive an older one's save.
- **Written through a file beside the destination**, then renamed over it, the previous file kept
  as `.bak` (the patcher's commit step); the same workbook writes the same bytes (each zip entry
  dated at the format's epoch).
- **Proof:** every workbook of the corpus opened, written as `.abaco`, read back: every cell's
  content, style and value after recalculation, and every part the engine holds (styles, names,
  metadata, settings, theme, tables, links, each sheet's structure) equal; written again to the
  same bytes; exported to `.xlsx` through the fork's writer and opened again to the same values
  (but a workbook with tables: the fork's writer writes none).
  The workspace's parts round trip on their own tests; a wrong format, a newer version, a missing
  part and a broken line are each refused by name.

## Phases
1. **The engine at parity** (P-007's exit). The fork in (above); `crates/veredas-abaco` at
   `2.0.0-alpha.1` with `links` and `linkdiff`; `tools/abaco-oracle`; the
   baseline taken with Veredas 1's oracle, then Veredas 2's run, equal or better. Clauses: the
   component, the link policy, the engine at parity.
2. **Byte-faithful edits.** The patcher: untouched parts byte for byte, caches rewritten only where
   an edit moved the engine's value (a spilled cell's too), references rippled through every sheet,
   name, validation, conditional format, comment, drawing, chart (a chart sheet's included) and
   table; the output read back through the engine beside the destination before it replaces it.
   Proven on workbooks built for each structure and on every workbook of the corpus (with nothing
   to do, with one number changed, and with each row and column move and a name set, verified or
   refused as pinned); Excel's acceptance owed where Excel runs. Not carried yet, refused before
   anything is written: a move that cuts into a table (resizing it), a move in a workbook with
   pivot tables, writing a formula that spills; a structured reference in a formula an edit writes
   is written as the cells it reads (the engine parses it to them). Its edits come
   through the API; an edit file, stamped TOML, comes with the command line, once core's header
   reader can be had without core's network layers (asked of core, Q345).
3. **The `.abaco` file** (above; C-224). Lossless round trip of every workbook of the corpus through the
   zip and back; the workspace's parts; the `.xlsx` export. Its stamps are read by core's header
   reader, its own source compiled by path until core's network layers sit behind a feature and
   Ábaco can depend on core (asked of core, 2026-11-23; Q345, `// DECISION PENDING`).
4. **Python.** CPython embedded (python-build-standalone, pinned, as Veredas 1 did: a release
   carries the runtime beside the executable); the `abaco` module; custom functions; the macro
   homes; the worker, one per workspace, and its command line; stop at the failing line with the
   model inspectable. The interface Oficina's tab, editor and debugger speak to the worker is agreed
   with Oficina's session (the review's entry 28, `docs/briefing/review.md`, puts one debugger
   client on Oficina's side, shared with ORCL).
5. **Acerto**, as Q343 rules.
6. **The grid's model** for Oficina's tab, as Q341 rules.

## On this machine
- No Excel, no Office: Excel's acceptance of a written file is owed elsewhere (Rules).
- Python 3.14.8 is installed with its shared library, but without `libpython3.14-dev`'s link name
  `libpython3.14.so`, `pip`, `venv`, `ensurepip` or `pytest`. Embedding works against it once the
  linker is shown the library (a link of one's own to `libpython3.14.so.1.0`, tried 2026-11-22).
  Veredas 1's runtime, python-build-standalone, installs in the home folder from its official
  source and brings all of these; phase 4 fetches it.

## Open questions
- Q340: where the IronCalc fork lives, and how its fixes go upstream.
- Q341: what becomes of Veredas 1's grid, now that the grid's surface is Oficina's.
- Q342: whether Veredas 1's Excel host is carried.
- Q343: whether `acerto` is carried into this public repository, and how.
- Q344: what parity is measured on while Veredas 1's five workbooks are not on this machine.

## Traps
- **Cargo makes a crate beneath a listed member's folder a member, whatever `exclude` says**: a
  member's path that is a prefix of the crate's wins over an exclusion. Met 2026-11-22 with the
  fork at `crates/veredas-abaco/ironcalc`: the laws passed (they read the manifest's lists as
  written), and cargo built and tested the fork's crates as members of the suite. The component
  test asks cargo itself (`workspace_members`), and the fork moved to `forks/ironcalc`.
- **Calamine 0.26 gathers a sheet's whole declared range**: on a workbook whose sheet declares an
  enormous dimension it asked for 512 GB and aborted Veredas 1's oracle. The cross-check streams
  calamine's cells instead.
- **Not every package that says "Microsoft Excel" was saved by Excel**: openpyxl writes "Microsoft
  Excel Compatible / Openpyxl" and caches no value; XlsxWriter writes "Microsoft Excel" with version
  12. And Excel on Android calls itself "Excel Android". The corpus's rule reads the exact name,
  the version and the workbook's file version.
- **A link's target is a URI**: Excel percent-encodes it (`Zone%20list.xlsx`). Veredas 1's linked
  resolver decoded only URLs; fork change 12, found by C-221's refute round.
- **A measure on `TODAY()` expires**: two calendar templates of the corpus build a year from
  `YEAR(TODAY())`; unpinned, parity would have failed on 2027-01-01. The engine's clock is pinned at
  run time (fork change 15) to the baseline's day; the compile-time `mock_time` would have changed
  every other build of the engine in the same cargo run.
- **One float path**: `statrs` without its `std` feature does its float math through `libm` unless
  another crate of the build turns `num-traits`' `std` on, so the engine computed differently
  alone and in the suite's workspace build (fork changes 13 and 14).
- **A cached `inf` equals every infinity**: an oracle tolerance written `|a-b| <= e` lets a
  non-finite cached value match; Veredas 1's had it. A cached number must be finite.
- **The patcher on real files** (phase 2's corpus test, 2026-11-22): a row with no `r` attribute
  (rows may leave it implicit) made Veredas 1's patcher rewrite a sheet no edit touched; a chart
  sheet (`chartsheet`, no cells) made it refuse the whole workbook, and a local name's
  `localSheetId` was written from the engine's order, which skips chart sheets; a random array's
  spilled cells were not seen as volatile, so the patcher's own verification failed, and a random
  draw that repeats by chance hid a volatile cell from a two-evaluation measure. Each is answered in
  `src/patch` and pinned by `tests/patch_corpus.rs`.
- **The patcher's verification ran after the rename** (C-223's refute round, 2026-11-22): a file it
  could not vouch for had already replaced the destination, an in-place patch's source with it. It
  now verifies the new file beside the destination, under the destination's own name (the engine
  names a workbook after its file), and renames only then.
- **The engine's model and Excel part ways on moves**: a deleted end of a range made `A1:#REF!`
  where Excel shrinks the range (fork change 17); an external reference's relative part kept its
  offset, so its address moved with its cell (19); a reference turned `#REF!` made a plain formula
  a dynamic array (20); a structured reference is cells to the engine and a table's range never
  moved (18). The patcher's verification found each, on 49 to 51 of the corpus's 266 workbooks
  with a row or a column deleted.
- **A cell an array spilled into is a value with no formula**: left as it was when its anchor goes,
  Excel reads it as a constant; rewritten as an inline string, it blocks the array (`#SPILL!`).
  Excel writes a spilled text as `t="str"`, and so does the patcher.
- **A text test of volatility reads names where there are none**: the engine stores formulas in
  R1C1, where `R[0]C[-2]` is a name `R`, and `RAND` is a substring of `GRANDTOTAL(`; a false
  positive keeps a stale cache. Volatile calls and names are read as whole tokens of each formula's
  A1 spelling, outside strings and quoted names, and a name is resolved as Excel does (the sheet
  that qualifies it or the formula's own, its local name before the workbook's; a LAMBDA name is
  read when called).
- **A random draw moves what reads it, by chance** (C-223's second round): a measure over a few
  evaluations lets a two-valued draw repeat. Every evaluation the patcher compares is made with
  one seed and one pinned time, so a draw is the same before, after and in verification; the
  measure compares with other seeds on other days.
- **Empty is neither 0 nor `""`**: a comparison that let them meet skipped a 0 written into an
  empty cell and hid an array's cell left as a constant. Values are compared as Excel tells them
  apart, numbers to 15 significant digits; only a formula's cache may hold an empty value as
  `<v></v>`.
- **An array grows and shrinks with what it reads**: its anchor's `ref` and its spilled cells
  must follow, and the engine keeps a spill's size as (width, height). Verification recomputes
  the array from the file, so a stale cache does not show there: it reads back the written cache
  of every value the edits moved.
- **A format's or a validation's formula is relative to its range's top-left cell**: a move that
  deletes that cell re-anchors the formula on the new one before shifting it, or it turns `#REF!`.
- **XML reads a carriage return as a line feed**: a text holding one is written `_x000D_`.
- **Excel writes a volatile array's spilled cells with an empty `<f ca="1"/>`**: a placeholder,
  not a formula; and a move can leave the engine a spilled cell outside its array, read empty.
- **What reads a random cell is unstable only while it does** (C-223's third round): exempt a
  cell from "moved" only if it is unstable before the edits and after them, or a formula the
  edits made deterministic keeps a stale cache.
- **References hide outside the formulas the engine reads**: a table's totals row and sort, a
  chartEx (`cx:f`, its own relationship type), a shape's `textlink`, an autoFilter's `colId`
  (counted from its first column) and its sort. Verification cannot see them: each has its test.
- **Excel writes `#SPILL!` and `#CALC!` as `#VALUE!`** with value metadata in rich-data parts;
  the patcher writes `#VALUE!` alone.
- **The engine names a workbook after its file** (`CELL("filename")`): the model is named after
  the destination before the first evaluation the patch compares; it spells every file `.xlsx`.
- **The engine's reading of a plain formula is not Excel's** (C-223's fourth round): it computes
  `SUM(A1:A3*B1:B3)` as an array (140) where Excel reads a plain `<f>` with implicit intersection
  (10). A formula an edit writes with a part that may not be single under an operator is written
  as Excel 365's dynamic array of one cell (`t="array"`, `cm`, the metadata part's dynamic-array
  entry, added when missing); fork change 21 opens the engine's static analysis for the judgement.
- **A table's header is its column's name**: editing it without renaming the table column is a
  repair in Excel; such an edit is refused.
- **A plain formula in the file is not the array an edit writes** (C-223's fifth round): the
  engine reads both alike, so "already there" and an expectation compare the formula's kind too.
  A one-value function given a range (`SUM(ABS(A1:A3))`) is lifted like an operator's part.
- **A form control lives twice**: its `ctrlProp` attributes and the VML shape's `x:FmlaLink`,
  `x:FmlaRange`, `x:FmlaTxbx` and `x:Anchor`; both copies move.
- **Several edits of one cell**: only the last computes its content, or an overwritten formula is
  looked for in a model that no longer holds it.
- **XML cannot carry U+FFFE and U+FFFF**: written `_xFFFE_`, `_xFFFF_`; a literal `_xHHHH_` in a
  text gets its underscore escaped (`_x005F_`), whatever character it names.
- **Part names differ in more than case** (OPC): a new sheet's part takes a name free in any case.
- **A metadata part may hold rich values only**: the dynamic-array type, its future-metadata list
  and a cell entry are appended among the root's own children, never inside an entry's `extLst`.
- **The engine's value setters leave a dynamic array's spill** (C-223's sixth round): a value over
  an anchor clears the array first (`Workbook::set_value`), or its cells stay as constants.
- **Excel 365's `@` is the judge of a plain formula's kind** (C-223's eighth round): a plain
  formula of the file is the array an edit writes where Excel 365 shows it with no implicit
  intersection (`=@INDEX(A1:A10,B1)` has one): its result, every operator's operands and every
  one-value argument single, a name of one cell read as the value it is. A cache cannot judge it:
  the intersected and the array value coincide on some inputs, and the edits change them.
- **The engine stores a plain formula with its own `@`** where it cannot see a name's extent
  (`IF(Flag,1,0)` as `@IF(Flag,1,0)`): the kind is judged on the formula's text parsed afresh.
- **The engine has errors no file holds** (`#CIRC!`, `#N/IMPL`, `#ERROR!`): a cache that would
  hold one refuses the patch.
- **`cm` is an index**, into the metadata part's cell metadata (fork change 22); `1` is Excel 365's
  usual, not the rule.
- **A formula's text is escaped as a cell's** (`_x005F_`; fork change 23).
- **A shared group is expanded only where a move changes what a member derives to**; moved whole,
  it keeps its sharing and its master's `ref` moves.
- **The kind is judged on the formula as the edits left it** (C-223's seventh and eighth rounds):
  a deletion that shrinks `SUM(A1:A3*B1:B3)` to `SUM(A1:A1*B1:B1)` leaves a formula Excel reads
  as written, and one an earlier edit of the list wrote is the patcher's own, whatever its text.
- **A name local to a chart sheet is the file's alone** (C-223's eighth round): the engine holds
  no chart sheet, so it keeps such a name with a sheet id no worksheet has and lists it as global.
  Ábaco's name list leaves it out; the patcher keeps its element where it is, moves its references
  as text, and refuses an edit of it.
- **A one-cell merge is no merge**; an empty list (`<hyperlinks/>`, `<mergeCells/>`) is no list.
- **A carriage return is `_x000D_` in text and `&#13;` in an attribute**, written in one pass.
- **SpreadsheetML under a namespace prefix is refused** (`<x:worksheet xmlns:x="…main">`, the
  .NET Open XML SDK's default; C-223's eighth and ninth rounds): the patcher reads tags by their
  literal, unprefixed names, as Excel writes them, so a part with any element under a prefix bound
  to the main namespace, transitional or strict (the root, or `<x:definedNames>` inside a plain
  root) refuses the patch,
  whatever its edits, naming the part and the element. A prefix declared and never used is none.
- **A prefix is read as XML reads it, and only Excel's own are followed** (C-223's eleventh
  round): a namespace declaration's value is decoded (`&#109;ain` is `main`); a chart's, a
  chartEx's, a drawing's, an extension's and a form control's VML elements are matched under
  Excel's prefixes (`c:`, `cx:`, `c15:`, `xdr:`, `x14:`, `xm:`, `x:`; a form control's properties
  x14's in the default namespace), and a part writing them under another is refused; so are
  threaded comments (Excel's default namespace), VML (`v:`) and the strict chart and drawing
  namespaces (`c:`, `xdr:`) (twelfth round). A part declaring another encoding than UTF-8, or
  holding CDATA, an XML comment or a processing instruction, is refused.
- **Attributes and texts are read as XML reads them** (C-223's twelfth round): `xml::attr` gives a
  value with its white space normalized and its references decoded (`A&#53;` is A5), `raw_attr`
  the bytes for a reader that decodes OOXML's `_xHHHH_` too; a value written back from it is
  escaped for XML. The texts holding a number or an area (anchors, a note's row and column, an
  `xm:sqref`, a cached value) are decoded before they are read. `<f ca="1"></f>` is the
  placeholder `<f ca="1"/>` is. The thirteenth round took the rest: a shared group is found
  whatever its `t` spelling, and a child written with an end tag is a child; `remove_attr` removes
  an attribute with spaces around its `=`; the metadata part's `fDynamic` and the content types'
  `PartName` are read as XML reads them; an end tag with white space before `>` is refused; the
  worksheet's tail moves its cell watches, scenarios, consolidation, smart tags, custom views and
  publishing (a deleted changing cell, source or published range refused); names compare in
  Unicode case, as the engine compares them.
- **Verification runs on a copy at a short UTF-8 path** (the engine's loader takes no other): under
  the system's temporary folder, the model renamed to the destination's stem before comparing;
  the new file then goes beside the destination under a short name of its own.
- **The engine reads `IFNA(…)` and `_xlfn.IFNA(…)` alike; Excel does not** (C-223's ninth and
  tenth rounds): "already there" and a name's rewrite compare the function spellings of the file's
  own text too (`refs::function_calls`: each call's prefix, each LET or LAMBDA parameter's
  `_xlpm.`, bracketed table columns passed over), a cell's found back in the source through the
  moves (`backward`).
- **A function the engine does not compute is refused in an edit** (`AGGREGATE`, `IMAGE`,
  `WEBSERVICE`): it would be cached as `#NAME?` and printed lowercased without its prefix. A LAMBDA
  name the formula sees (global, or local to its own sheet or scope) and a parameter a LET or
  LAMBDA binds are the workbook's own.
- **A new cell takes its row's format, else its column's** (C-223's tenth round): a row formatted
  whole (`s` with `customFormat`) gives a cell typed into it that format, as Excel does. The engine keeps its
  function list and a LAMBDA parameter's name private; the patcher tells LET and the name by their
  debug text.
- **A legacy array is the edit's kind only at the edit's size**: `{=A1:A3*2}` over one cell shows
  one value where the edit's formula spills three; `{=SUM(…)}` over one cell is the same.
- **An attribute is rewritten between double quotes**, whatever quotes it had: a value escaped for
  `"` may hold a raw `'`.
- **An attribute's raw whitespace is a space** (XML's attribute-value normalization; C-223's
  eighth round): a moved `cfvo` formula or text link is read as any reader reads it, a raw tab,
  line feed or CR LF one space, `&#9;` a tab.
- **TOML has no null, and the engine's `Color::None` serialises as one** (an untagged enum's unit
  variant): the `.abaco` file writes the engine's structures through JSON, null as `nan`.
- **The zip reader keeps one entry of a name**: a file holding two cannot be told apart from one.
- **The fork's writer writes no tables**: an `.xlsx` export of a workbook with tables reads its
  structured references otherwise.
- **A read-only destination**: its `.bak` is read-only too after the first patch, so the next
  removes it before making a new one, and the destination keeps its own permissions.
- **Calamine 0.26 does not build with zip 2.6**: the lock holds zip 2.4.2, the version Veredas 1's
  lock had.
