# Contract Items

Governance scope: the **`grande-sertao-veredas`** repo, Veredas 2 — the successor suite to Veredas 1
(`C:\Source\Repos\Veredas`: its own history, never merged here). Authority: `docs/veredas-plan.md`
wins on intent, this contract on status, `tools/leis` enforces the laws (`docs/questions.md` Q4).
The `C-xxx` clauses are the suite's structural laws, each proven by one test file per law under
`tools/leis/tests/` on synthetic suites (exact violations, by message) plus `real_tree.rs` on the
tree as it is; one file per law, so a change to one law re-stales only its clause. A law changes
only with its clause. The eight components and the two suite-wide rules that need code live as
`P-xxx` under **Pending Contract Deltas**, one per component, each citing the plan's section and
the Veredas 1 crates it absorbs; none copies the plan's prose. A member arrives this way: its
`P-xxx` → its `docs/<nome>-plan.md` → its clauses here → `crates/veredas-<nome>` at core's MAJOR →
its simulator in maquinar → `cargo test -p leis` green → reconciled. Runs are recorded under
`.zen/evidence/runs/`, refute verdicts under `.zen/evidence/refutes/`, both committed; what a
refute round found and what it changed is in the clause's evidence note.

## C-001: Members live under crates/ or tools/, each listed by path
- **Date:** 2026-11-05
- **Source:** user (the plan review of 2026-11-05; docs/veredas-plan.md "Adding a component")
- **Description:** Every `Cargo.toml` under `crates/` (a component) or `tools/` (repo tooling), at any depth outside `target/`, `node_modules/` and hidden folders, is a workspace member listed by path in `Cargo.toml` or is covered by an `exclude` entry (which covers everything beneath it, as cargo reads it); every member, and every crate found, lives directly under one of those two roots. A listed path with no manifest there (a glob, a typo) is refused; a crate nested in a folder, or at the root folder itself, is refused unless excluded; a `veredas-*` or `orcl-*` crate anywhere in the repository (by name or by folder, an underscore reading as the hyphen) is a member under a root or nothing, never excluded and never parked elsewhere, so no suite crate dodges the laws by leaving the workspace; a crate elsewhere that is not the suite's is nobody's business. A member is a decision taken through the five conditions, never a folder a glob admits.
- **Verification:** automated → tools/leis/tests/l1_roots.rs, tools/leis/tests/real_tree.rs | synthetic suites for each rule, the reader on planted trees (nested, root-level, hidden and built manifests; suite crates elsewhere), the exclusion as cargo reads it, a suite crate excluded by name, by folder, behind an excluded parent, outside the roots, and under an underscore spelling; the real tree
- **Evidence:** .zen/evidence/C-001.md
- **Status:** pending verification

## C-002: A component is named veredas-<nome>, in a folder of its name
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Naming")
- **Description:** A member under `crates/` is `veredas-<nome>`, `<nome>` a lowercase ASCII letter followed by lowercase letters and digits (no hyphen, underscore, accent or space), in the folder `crates/<name>`, matched exactly whatever the filesystem ignores. The language's crates are `orcl-<nome>` by the same rule. A `veredas-*` crate is a component and an `orcl-*` crate is the language's: both live under `crates/`, never `tools/`, whatever follows the prefix; a tool is not a component; an extension lives inside its host and is not a member (L1 refuses it listed). That the nome is Portuguese and names the role, and that screens show the accented name, is checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/l2_naming.rs, tools/leis/tests/real_tree.rs | the prefix, the nome's character set, the exact folder, the language's names, a tool, a veredas-* crate under tools/, an extension listed; the real tree
- **Evidence:** .zen/evidence/C-002.md
- **Status:** pending verification

## C-003: SemVer per component from 2.0.0, no shared workspace version
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Versioning")
- **Description:** `[workspace.package]` carries no `version`; every component states its own `version`, valid SemVer with MAJOR ≥ 2 (a pre-release such as `2.0.0-alpha.1` counts), never `version.workspace = true`. A tool under `tools/` versions as it likes. That a new component starts at core's current MAJOR is the plan's rule, checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/l3_versioning.rs, tools/leis/tests/real_tree.rs | the shared version, the inherited version, below 2, not SemVer, absent, a pre-release and a tool; the real tree
- **Status:** verified

## C-004: veredas-core depends on no other member
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core: the foundation", rules)
- **Description:** `veredas-core` names no other workspace member in any dependency table: normal, dev, build or target-specific, in cargo's hyphen or pre-2024 underscore spelling, a dependency renamed with `package` or inherited from `[workspace.dependencies]` (named by the root entry alone, as cargo ignores a `package` written beside `workspace = true`) included. Its own self dev-dependency (a test feature) is allowed. The foundation builds and tests alone.
- **Verification:** automated → tools/leis/tests/l4_core.rs, tools/leis/tests/real_tree.rs | every table, the self dev-dependency, a target table, a rename in the member, a rename in the workspace table with a member's `package` beside it ignored, the underscore tables; the real tree
- **Evidence:** .zen/evidence/C-004.md
- **Status:** verified

## C-005: Dependencies point one way, along the map
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Architecture" and each component's rules; docs/questions.md Q5)
- **Description:** Every edge from a component to another member in its normal or build dependencies, however declared (a path, a `package` rename, a `workspace = true` inheritance), is in `MAP` (`tools/leis/src/lib.rs`): anything may use core; portal uses core alone; maquinar uses core and aranha (its codecs, to emulate the devices aranha scans); aranha uses core; the apps (comissionador, supervisao, abaco, wms) use portal; controlador uses portal, aranha and the `orcl-*` crates; `orcl-*` use each other. A component absent from the map may use core alone, and adding its row is a contract change. A dev-dependency is free (a bench is a fixture), a tool may depend on anything, and a crate that is not a member is not the map's business. The tests carry their own copy of this sentence and walk every pair of the planned suite against it, so a row of `MAP` cannot drift from the clause unnoticed.
- **Verification:** automated → tools/leis/tests/l5_direction.rs, tools/leis/tests/real_tree.rs | the planned suite, app→app, bench→portal, portal→app, a component outside the map, build vs dev, a tool, a non-member, a workspace-table rename, every pair of the planned suite against the clause's copy of the map; the real tree
- **Evidence:** .zen/evidence/C-005.md
- **Status:** verified

## C-006: No component's code lands without its plan
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Adding a component", condition 1)
- **Description:** Every component `veredas-<nome>` has `docs/<nome>-plan.md`, not blank (whitespace, the byte-order mark and the zero-width characters U+200B–U+200D and U+2060 are not text), and so does every commit reachable from HEAD that holds the component's manifest: a law over git history, every commit judged (merges, re-additions and a plan deleted then restored included), the earliest offending commit named, so no component's code ever landed without its plan, whatever the tree says today. The language's crates share `docs/orcl/README.md`; the suite's own `docs/veredas-plan.md` is no component's plan; a tool needs no plan; a component not yet committed is the tree law's business; a shallow clone, its history cut, is refused rather than judged. The suite runs on every push (`.github/workflows/leis.yml`, full history) and before every commit by the repo's rule. What the plan must say (the one-sentence role nobody covers, its scope and rules) is checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/l6_plans.rs, tools/leis/tests/real_tree.rs | absent, blank, a byte-order mark, a title, the language's shared plan, a tool, a veredas-* crate under tools/ through the laws together, the reader leaving the suite's plan to nobody, throwaway git repositories where the plan comes late, together or blank, where the code lands on a merged branch or in the merge commit itself, is removed and re-added, or its plan is deleted and restored, and where the component is uncommitted, there is no repository, or the clone is shallow; the real tree and its history
- **Evidence:** .zen/evidence/C-006.md
- **Status:** pending verification

## Pending Contract Deltas

### P-001: veredas-core — the foundation
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core: the foundation")
- **Description:** Every class and library more than one component needs, as a library that depends on no other component: shared types (tags, values, timestamps, quality), the TCP telegram and OPC UA client layers, plain-text file schemas with their version stamps, errors, logging and test-fixture plumbing; new functionality designed as if a second consumer already existed. Absorbs Veredas 1's `veredas-telegram` and `veredas-opc`. Exit: `docs/core-plan.md`, `crates/veredas-core` at 2.0.0-alpha.1 building and tested, the laws green, its capabilities as clauses here.
- **Status:** pending

### P-002: veredas-portal — the gateway
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-portal: the gateway")
- **Description:** The single point of exchange between the suite and the outside world: the data plane (tailing, log families, regex capture), the DuckDB session store, SSE, windowed REST and the gated write surface (whitelisted tags, master switch off by default), the desktop shell, and a light embeddable build for controlador. Absorbs Veredas 1's `veredas-core` (the data plane), `veredas-store`, `veredas-server`, `veredas-desktop`. Exit: `docs/portal-plan.md`, `crates/veredas-portal` at 2.0.0-alpha.1 building and tested against maquinar, the laws green, its capabilities as clauses.
- **Status:** pending

### P-003: veredas-controlador — the softPLC
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-controlador: the softPLC")
- **Description:** ORCL programs in a deterministic scan cycle with watchdog and safe outputs; direct NIC access on the real-time path (the only production path past portal); PLC Hardware Configuration on an embedded aranha; an embedded portal-lite for logging and non-real-time protocols; states which ORCL versions it runs. Builds on Veredas 1's `orcl-std`, `orcl-runtime`, `demos/conveyor` (ORCL keeps its own SemVer and `docs/orcl/`). Exit: `docs/controlador-plan.md`, the ORCL crates and `crates/veredas-controlador` at 2.0.0-alpha.1 with a simulated fieldbus in maquinar, the laws green.
- **Status:** pending

### P-004: veredas-maquinar — the bench
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-maquinar: the bench"; decisions 1, 2)
- **Description:** Simulated hardware so every consumer meets its plant on a bench first: log emitters, a configurable OPC UA server, TCP telegram channels up to a full simulated sort controller, automated replies (random or rule-based), controllable pressure (rates, no-reads, full chutes); later a 2D/3D plant simulator. Owns its sockets; speaks the real device's wire; test-only and light, no DuckDB, never portal's store or desktop. Absorbs Veredas 1's `veredas-maquinar` and `veredas-trajetorias`' automated replies (its store is not carried). Exit: `docs/maquinar-plan.md`, `crates/veredas-maquinar` at 2.0.0-alpha.1, the laws green.
- **Status:** pending

### P-005: veredas-aranha — the network scanner
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-aranha: the network scanner")
- **Description:** The industrial network scanner for setup and fault-finding, never for production: PROFINET DCP and LLDP first, then EtherNet/IP, Modbus, S7, OPC UA, BACnet; listens before it asks, fuses every protocol's answer with provenance, draws the topology and diffs it against a baseline. Owns its sockets; embedded by controlador as its PLC Hardware Configuration; runs on its own for commissioning. Absorbs Veredas 1's `veredas-aranha` and its plan. Exit: `docs/aranha-plan.md`, `crates/veredas-aranha` at 2.0.0-alpha.1 against maquinar's emulated devices, the laws green.
- **Status:** pending

### P-006: veredas-comissionador — the testing platform
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-comissionador: the testing platform"; decision 6; design docs/design/comissionador/)
- **Description:** Predefined checks on portal data, live or replayed over a stored window, ending in an acceptance report per run; its own designer with one plain-text test file and two views (text, flow chart), script boxes, functions written in text and reused as blocks, branching on any outcome with retries. Reads plant data only through portal; drives machinery only through portal's gated writes; tests are diff-able in git. New in Veredas 2. Exit: `docs/comissionador-plan.md` (the grammar settled), `crates/veredas-comissionador` at 2.0.0-alpha.1 running a test against maquinar through portal, the laws green.
- **Status:** pending

### P-007: veredas-abaco — the engineering spreadsheet, with acerto
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-abaco: the engineering spreadsheet"; decision 4)
- **Description:** The spreadsheet for engineering work where Excel gets too tight: a fork of IronCalc modified for engineering, Python macros and custom functions, byte-faithful `.xlsx` edits with no Office on the machine, its own `.abaco` workspace file, and extensions starting with `acerto` (plant alignment, no `veredas-` prefix, inside its host). Edits the same workbooks the other components read; fork fixes go upstream where they fit; accuracy is measured against Excel's own cached values. Absorbs Veredas 1's `veredas-abaco`, `veredas-abaco-py`, the IronCalc fork, `abaco-oracle` (as test tooling under `tools/`) and `python/veredas-acerto`. Exit: `docs/abaco-plan.md`, `crates/veredas-abaco` at 2.0.0-alpha.1 at parity with Veredas 1's oracle numbers, the laws green.
- **Status:** pending

### P-008: veredas-supervisao — the web interface
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-supervisao: the web interface"; decision 9; design docs/design/supervisao/)
- **Description:** The unified web interface for the suite: layouts in 2D (SVG) and 3D (glTF) sharing one set of object IDs, objects that move live, dashboards, charts and tables absorbing Veredas 1's dashboard, several sessions served side by side from one machine, editing in the browser or in plain-text config files. Owns no plant connection; every value arrives through portal and every write rides portal's gated surface; 3D models are assets the config points to. Absorbs Veredas 1's `veredas-supervisao` and `dashboard/`. Exit: `docs/supervisao-plan.md` (the binding file format settled), `crates/veredas-supervisao` at 2.0.0-alpha.1 fed by portal from maquinar, the laws green.
- **Status:** pending

### P-009: veredas-wms — later, after its study
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-wms: later"; decision 5)
- **Description:** A production warehouse management system, structured for the job: a dedicated study first, then its own plan and development phase, after the eight components above. Its structure comes from the study, not from `veredas-trajetorias`; production storage lives here, never in maquinar; it follows the suite's rules (portal, core, SemVer; the `wms` acronym kept). Exit: the study, then `docs/wms-plan.md`.
- **Status:** pending

### P-010: Versioned plain-text files
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Versioning", "Versioned files")
- **Description:** Every plain-text config and project file of the suite states its schema version; portal migrates it or refuses it, and never guesses. A portal capability (P-002), parked apart because every component's files obey it and the law that checks the stamp belongs to core's file schemas.
- **Status:** pending

### P-011: Suite releases
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "How releases work")
- **Description:** A suite release is a manifest that pins a tested set of component versions, shipped and installed together; a MAJOR bump of `veredas-core` means every dependent is rebuilt and retested before its own next release. Exit: the manifest format, the release recipe, and `2.0.0-alpha.1` cut with every component on the scheme.
- **Status:** pending
