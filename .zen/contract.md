# Contract Items

Governance scope: the **`grande-sertao-veredas`** repo, Veredas 2 — the successor suite to Veredas 1
(`C:\Source\Repos\Veredas`: its own history, never merged here). Authority: `docs/veredas-plan.md`
wins on intent, this contract on status, `tools/leis` enforces the laws (`docs/questions.md` Q4).
The `C-xxx` clauses are the suite's structural laws, each proven by `tools/leis/tests/leis.rs` on
synthetic suites (exact violations, by message) and on the real tree; a law changes only with its
clause. The eight components and the two suite-wide rules that need code live as `P-xxx` under
**Pending Contract Deltas**, one per component, each citing the plan's section and the Veredas 1
crates it absorbs; none copies the plan's prose. A member arrives this way: its `P-xxx` → its
`docs/<nome>-plan.md` → its clauses here → `crates/veredas-<nome>` at core's MAJOR → its simulator
in maquinar → `cargo test -p leis` green → reconciled. Runs are recorded under
`.zen/evidence/runs/`, refute verdicts under `.zen/evidence/refutes/`, both committed.

## C-001: Members live under crates/ or tools/, each listed by path
- **Date:** 2026-11-05
- **Source:** user (the plan review of 2026-11-05; docs/veredas-plan.md "Adding a component")
- **Description:** Every crate under `crates/` (a component) or `tools/` (repo tooling) is a workspace member listed by path in `Cargo.toml`, and every member lives directly under one of those two roots. A listed path with no manifest there (a glob, a typo) is refused; a crate under a root that is neither listed nor in `exclude` is refused. A member is a decision taken through the five conditions, never a folder a glob admits.
- **Verification:** automated → tools/leis/tests/leis.rs | l1_a_member_outside_the_roots_is_named, l1_a_listed_member_without_a_manifest_is_named, l1_a_crate_that_is_not_listed_is_named, l1_an_excluded_crate_is_not_a_member, l1_a_nested_folder_is_not_a_root, the_real_tree_obeys_every_law; the real walker probed on a scratch copy with three bad crates (the evidence note)
- **Evidence:** .zen/evidence/C-001.md
- **Status:** pending verification

## C-002: A component is named veredas-<nome>, in a folder of its name
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Naming")
- **Description:** A member under `crates/` is `veredas-<nome>`, `<nome>` Portuguese in lowercase ASCII letters and digits, and its folder is `crates/<name>`; screens show the accented name. The language's crates (`orcl-*`) keep their names; a member under `tools/` is not a component; an extension lives inside its host and is not a member.
- **Verification:** automated → tools/leis/tests/leis.rs | l2_a_component_without_the_prefix_is_named, l2_an_uppercase_or_accented_nome_is_named, l2_a_bare_prefix_is_named, l2_a_folder_not_named_after_its_crate_is_named, l2_the_language_crates_keep_their_names, l2_a_tool_is_not_a_component, the_real_tree_obeys_every_law
- **Status:** pending verification

## C-003: SemVer per component from 2.0.0, no shared workspace version
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Versioning")
- **Description:** `[workspace.package]` carries no `version`; every component states its own `version`, valid SemVer with MAJOR ≥ 2 (a pre-release such as `2.0.0-alpha.1` counts), never `version.workspace = true`. A tool under `tools/` versions as it likes. That a new component starts at core's current MAJOR is the plan's rule, checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/leis.rs | l3_a_shared_workspace_version_is_named, l3_a_component_inheriting_its_version_is_named, l3_a_version_below_2_is_named, l3_a_version_that_is_not_semver_is_named, l3_a_component_without_a_version_is_named, l3_a_prerelease_from_2_passes_and_a_tool_is_free, the_real_tree_obeys_every_law
- **Status:** pending verification

## C-004: veredas-core depends on no other member
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core: the foundation", rules)
- **Description:** `veredas-core` names no other workspace member in any dependency table: normal, dev, build or target-specific, a renamed dependency (`package = …`) included. Its own self dev-dependency (a test feature) is allowed. The foundation builds and tests alone.
- **Verification:** automated → tools/leis/tests/leis.rs | l4_core_depending_on_a_member_is_named_in_every_table, l4_core_may_depend_on_itself_for_its_tests, l4_a_target_specific_table_counts, l4_a_renamed_dependency_is_seen_by_its_package, the_real_tree_obeys_every_law
- **Status:** pending verification

## C-005: Dependencies point one way, along the map
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Architecture" and each component's rules)
- **Description:** Every edge from a component to another member in its normal or build dependencies is in `MAP` (`tools/leis/src/lib.rs`): anything may use core; portal uses core alone; maquinar uses core and aranha; aranha uses core; the apps (comissionador, supervisao, abaco, wms) use portal; controlador uses portal, aranha and the `orcl-*` crates; `orcl-*` use each other. A component absent from the map may use core alone, and adding its row is a contract change. A dev-dependency is free (a bench is a fixture), a tool may depend on anything, and a crate that is not a member is not the map's business.
- **Verification:** automated → tools/leis/tests/leis.rs | l5_the_planned_suite_breaks_no_law, l5_an_app_depending_on_another_app_is_named, l5_the_bench_never_pulls_portal, l5_portal_never_depends_on_an_app, l5_a_component_outside_the_map_may_use_core_alone, l5_a_build_dependency_counts_and_a_dev_dependency_is_free, l5_a_tool_may_depend_on_anything, l5_an_external_crate_of_a_member_name_shape_is_not_an_edge, the_real_tree_obeys_every_law
- **Status:** pending verification

## C-006: A component's plan exists before its code
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Adding a component", condition 1)
- **Description:** Every component `veredas-<nome>` has a non-empty `docs/<nome>-plan.md` (the one-sentence role nobody covers, its scope and rules); the language's crates share `docs/orcl/README.md`. A tool needs no plan.
- **Verification:** automated → tools/leis/tests/leis.rs | l6_a_component_without_its_plan_is_named, l6_an_empty_plan_is_named, l6_a_plan_with_a_title_passes, l6_the_language_crates_share_docs_orcl, l6_a_tool_needs_no_plan, the_real_tree_obeys_every_law
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
