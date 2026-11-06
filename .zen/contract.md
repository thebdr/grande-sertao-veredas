# Contract Items

Governance scope: the **`grande-sertao-veredas`** repo, Veredas 2 — the successor suite to Veredas 1
(`C:\Source\Repos\Veredas`: its own history, never merged here). Authority: `docs/veredas-plan.md`
wins on intent, this contract on status, `tools/leis` enforces the laws (`docs/questions.md` Q4).
C-001..C-006 are the suite's structural laws, each proven by one test file per law under
`tools/leis/tests/` on synthetic suites (exact violations, by message) plus `real_tree.rs` on the
tree as it is; one file per law, so a change to one law re-stales only its clause. A law changes
only with its clause. From C-007 the clauses are a component's capabilities, one test file per
capability under the component's `tests/`, run against reality (real sockets, the real server),
never a mock; its plan (`docs/<nome>-plan.md`) says what each absorbed from Veredas 1. The eight
components and the two suite-wide rules that need code live as
`P-xxx` under **Pending Contract Deltas**, one per component, each citing the plan's section and
the Veredas 1 crates it absorbs; none copies the plan's prose. A member arrives this way: its
`P-xxx` → its `docs/<nome>-plan.md` → its clauses here → `crates/veredas-<nome>` at core's MAJOR →
its simulator in maquinar → `cargo test -p leis` green → reconciled. Runs are recorded under
`.zen/evidence/runs/`, refute verdicts under `.zen/evidence/refutes/`, both committed; what a
refute round found and what it changed is in the clause's evidence note.

## C-001: Members live under crates/ or tools/, each listed by path
- **Date:** 2026-11-05
- **Source:** user (the plan review of 2026-11-05; docs/veredas-plan.md "Adding a component")
- **Description:** Every `Cargo.toml` under `crates/` (a component) or `tools/` (repo tooling), at any depth, is a workspace member listed by path in `Cargo.toml` or is covered by an `exclude` entry (which covers everything beneath it, as cargo reads it); every member, and every crate found, lives directly under one of those two roots. A listed path with no manifest there (a glob, a typo) is refused; a crate nested in a folder, or at the root folder itself, is refused unless excluded; a `veredas-*` or `orcl-*` crate anywhere the repository holds it or could (by name or by folder, an underscore reading as the hyphen) is a member under a root or nothing, never excluded and never parked elsewhere, so no suite crate dodges the laws by leaving the workspace; a crate elsewhere that is not the suite's is nobody's business. The reader does not walk the disk by a list of folder names: it asks git what the repository holds or could hold (tracked files, and untracked files git does not ignore), so what git would never take is nobody's business and what it would take is seen wherever it sits, a hidden folder, a nested ignore rule un-ignoring `target/`, or a forced add included; a manifest is `Cargo.toml` in any spelling of its case, as a filesystem that ignores case builds it, and a folder holding it under more than one spelling is refused (which one cargo reads would depend on the filesystem; the exact `Cargo.toml` is the text the laws read meanwhile); a submodule, a folder the repository holds whose files it never lists, is a crate by its folder, with its manifest when checked out; a tree that is no repository is refused. A member is a decision taken through the five conditions, never a folder a glob admits.
- **Verification:** automated → tools/leis/tests/l1_roots.rs, tools/leis/tests/real_tree.rs | synthetic suites for each rule, the reader on planted repositories (nested, root-level, hidden and ignored manifests; suite crates elsewhere, under a dot, under an un-ignored target/, forced past the ignore, spelled `cargo.toml`, spelled both ways in one folder, as a checked-out submodule and as a bare submodule pointer), the exclusion as cargo reads it, a suite crate excluded by name, by folder, behind an excluded parent, outside the roots, and under an underscore spelling, a tree without git refused; the real tree
- **Evidence:** .zen/evidence/C-001.md
- **Status:** verified

## C-002: A component is named veredas-<nome>, in a folder of its name
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "Naming")
- **Description:** A member under `crates/` is `veredas-<nome>`, `<nome>` a lowercase ASCII letter followed by lowercase letters and digits (no hyphen, underscore, accent or space), in the folder `crates/<name>`, matched exactly whatever the filesystem ignores. The language's crates are `orcl-<nome>` by the same rule. A `veredas-*` crate is a component and an `orcl-*` crate is the language's: both live under `crates/`, never `tools/`, whatever follows the prefix, by name or by folder, an underscore reading as the hyphen (as L1 reads a suite crate); a tool is not a component; an extension lives inside its host and is not a member (L1 refuses it listed). That the nome is Portuguese and names the role, and that screens show the accented name, is checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/l2_naming.rs, tools/leis/tests/real_tree.rs | the prefix, the nome's character set, the exact folder, the language's names, a tool, a veredas-* crate under tools/, an extension listed; the real tree
- **Evidence:** .zen/evidence/C-002.md
- **Status:** verified

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
- **Description:** Every component `veredas-<nome>` has `docs/<nome>-plan.md`, not blank (whitespace, the byte-order mark and the zero-width characters U+200B–U+200D and U+2060 are not text), and so does every commit reachable from HEAD that holds a component under `crates/` (`veredas-*` or `orcl-*` by name, a folder, a submodule or a link, present at HEAD or not: a renamed or deleted component's landing is judged too): a law over git's trees, every commit judged (merges, re-additions and a plan deleted then restored included), the plan a regular file of text (a folder or a symlink in its place is no plan; bytes that are not UTF-8 are blank), the earliest offending commit named once per folder, so no component's code ever landed without its plan, whatever the tree says today. The language's crates share `docs/orcl/README.md`; the suite's own `docs/veredas-plan.md` is no component's plan, in history as in the tree; a tool needs no plan; a component not yet committed is the tree law's business; a shallow clone, its history cut, is refused rather than judged. The law answers in a fixed number of git conversations whatever the history's length. The suite runs on every push (`.github/workflows/leis.yml`, full history) and before every commit by the repo's rule. What the plan must say (the one-sentence role nobody covers, its scope and rules) is checked at the component's review, not by the law.
- **Verification:** automated → tools/leis/tests/l6_plans.rs, tools/leis/tests/real_tree.rs | absent, blank, a byte-order mark, a title, the language's shared plan, a tool, a veredas-* crate under tools/ through the laws together, the reader leaving the suite's plan to nobody, throwaway git repositories where the plan comes late, together or blank, where the code lands on a merged branch or in the merge commit itself, is removed and re-added, or its plan is deleted and restored, where the component was renamed or deleted since, where a folder or a symlink stands for the plan, where the plan is UTF-16, where the suite's plan is the only plan, where the component is a submodule or a link, a history of three thousand commits answered within a bound, and where the component is uncommitted, there is no repository, or the clone is shallow; the real tree and its history
- **Evidence:** .zen/evidence/C-006.md
- **Status:** verified

## C-007: veredas-core — the foundation
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core: the foundation"; promoted from P-001)
- **Description:** `crates/veredas-core` holds what more than one component needs (the plan's scope; `docs/core-plan.md` maps what it absorbed from Veredas 1's `veredas-telegram` (its socket, framing and capture only), `veredas-opc` and core, and the phases still ahead: file schemas, logging), content-agnostic (no source or test file names the GENIBELT protocol or cites a vendor document: what a device's telegrams mean is configuration; `docs/questions.md` Q8), as a library that builds no executable: cargo, asked what it would build, names exactly one library target (crate types `lib`/`rlib`) and tests, nothing else. It depends on no other member, its plan came before its code, and it versions on its own from 2.0.0 (L3; it started at 2.0.0-alpha.1). Each capability is a clause of its own below.
- **Verification:** automated → crates/veredas-core/tests/foundation.rs, tools/leis/tests/real_tree.rs | `cargo metadata --no-deps` on core, and on planted packages that each make an executable another way (`[[bin]]`, `[[example]]`, `src/main.rs`, `src/bin/`, `examples/`, `crate-type` and edition 2021's `crate_type` naming `bin`, a `cdylib`, an empty `crate-type`, a bench, a build script, `autolib = false`), all refused; every file of the crate, whatever its extension, the manifest included, read whole, normalised to letters and digits across line breaks, for the vendor's name and protocol terms and for vendor-document and section citations, the reader checked on planted spellings; the laws on the real tree with core in it (name, version, no member dependency, plan in its history)
- **Evidence:** .zen/evidence/C-007.md
- **Status:** verified

## C-008: The shared types: one value model, a tag, a sample on two clocks, quality
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core: the foundation", scope; docs/core-plan.md "Scope")
- **Description:** A `Value` is an int, float, text or naive time, with a numeric view for all but text (a time as the epoch milliseconds of its stamp read as UTC, rounded down), and displays as the operator reads it; `SeriesType` names the four; a tag is a `SeriesRef`, a series of one source, shown `source/series`, ordered by source then series (not by the shown text); a `Sample` indexes on its parsed stamp, else on arrival, on the value's millisecond axis; `Quality` is good, uncertain or bad, and only good is good; `now_unix_ms` is the unix clock in milliseconds.
- **Verification:** automated → crates/veredas-core/tests/types.rs | Veredas 1's tests plus the edges: case and ordering of ids, tags ordered apart from their shown text (`plc` before `plc-2`), negative and wide ints, text never parsed, times before the epoch and below the millisecond, NaN, display of each variant and of fractional seconds, int vs float equality, both index paths with negative clocks, the quality labels, arrival bracketed by the system clock
- **Evidence:** .zen/evidence/C-008.md
- **Status:** verified

## C-009: One stop/join handle for every source worker
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md "Scope", worker plumbing)
- **Description:** A source worker is a thread ending in `Result<(), SourceError>`, held by a `SourceHandle`: polling it never blocks a live worker, reads finished once the worker has ended, keeps the outcome (its error, or `Panicked` for a panic, whatever the panic's payload) and gives `join` that same outcome without joining twice; stopping raises the worker's own flag, and dropping the handle raises it without waiting. A `SourceError` names an I/O failure with its path.
- **Verification:** automated → crates/veredas-core/tests/source_handle.rs | real threads: a live worker polled without blocking, stop reaching a worker, an error kept across polls and join, join alone, an ended worker finished before any poll, a panic as `Panicked` (a payload whose drop panics too included), a drop that does not wait, every error's text and its source chain
- **Evidence:** .zen/evidence/C-009.md
- **Status:** verified


## C-010: The framing engine: any configured layout, frames that never lie about their size
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-core", scope: a framing engine driven by a channel configuration schema; docs/questions.md Q8, Q9; docs/channel-schema.md)
- **Description:** Core holds no frame layout: a `Framing` compiled from a channel file encodes and decodes whatever it describes (optional start and end delimiters; envelope, segment-header and tail fields of 1 to 8 bytes in either byte order, each opaque, or the envelope's sequence, data-area length or segment count, or a segment's body length), a frame with no segments being a keep-alive. Encoding computes every length and count, stamps the sequence given, fills opaque fields from the frame or their configured defaults, and refuses a value its field cannot hold or a frame over the limit, never clamping; decoding returns the frame and the bytes it used, asks for more only while the delimiter, envelope and whole segment headers that have arrived could still be a valid frame, and refuses a frame whose lengths disagree once the envelope or the segment header that shows it has arrived whole (never holding the real frames behind it); every length or count a peer declares is bounded by the frame limit before any arithmetic, so a field 8 bytes wide never wraps, and a data area, body or segment count that can never fit the limit, or a body that leaves no room for the segment headers still to come, is refused at once rather than waited for, as is a data area that its declared count of segments could not fill (each at most its header and the largest body its length field can declare); a refusal says how far to skip to the next occurrence of the start delimiter's first byte (one byte where there is no start delimiter). Hex text parses as two ASCII hex digits per byte; previews and dumps render bodies for an operator.
- **Verification:** automated → crates/veredas-core/tests/telegram_framing.rs | on synthetic layouts read from channel files: the segmented envelope (Veredas 1's codec tests ported: golden frames byte for byte in both orders, every prefix waiting, exact resync distances, inconsistent lengths refused early, the remaining-headers boundary on both sides, a wrong byte order read exactly, the frame limit at its edge both ways), opaque fields and their defaults, the sequence stamp, values that do not fit; a length-prefixed layout with no delimiter or segments; a delimited layout counted by segments; 8-byte envelope lengths, segment lengths and counts at `u64::MAX` and 2^63 refused without wrapping, and each limit (data area, body, count, headers still to come) pinned on both sides with the frame at its edge decoded whole; a data area too large for its count of one-byte-length segments refused at the envelope and after a header, both sides of each bound, real frames behind it never held; a hand-built framing past the ceiling waiting rather than wrapping; hex, previews and dumps
- **Evidence:** .zen/evidence/C-010.md
- **Status:** verified

## C-012: One TCP telegram link, held correctly in either role
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md; docs/questions.md Q8, Q9: the rules are the channel file's; docs/channel-schema.md)
- **Description:** A `Channel` listens or dials on its own thread, framing by its channel file and holding the link by that file's rules: with `[sequence]` rules, on connect each side sends a keep-alive stamped `connect`, then stamps `first..=last` at send time, a number taken only by a frame that encodes (a send the encoder refuses takes none), and the receiver skips a frame repeating the last number it saw, keep-alives counting as seen when `keepalive_counts` says so (bar the connect keep-alive's, `connect` lying outside the cycle); a keep-alive, the channel's probe or one its owner sends (any frame the peer will read as one), takes the next number when keep-alives count, and carries the last number stamped, taking none, when they do not, so data frames never come back round to the last one; a keep-alive (a frame with no segments; in a segmentless layout, an empty body) is delivered as a keep-alive event carrying its fields; a keep-alive on connect when `on_connect`; silence for `idle` sends one keep-alive probe per window, never while the peer talks (zero never probes); a dialler retries until the listener appears; replies are unicast by a peer id never reused, so a reply for a departed peer is dropped, never delivered to its successor; frames go out as soon as the socket takes them and every queued command is handled at once, so a peer that reads keeps up with any burst its outbox bound holds, whatever other peers are connected, and the channel keeps reading meanwhile; a peer whose socket takes nothing for the write timeout is dropped, the socket itself asked (never a readiness cache, so a slow but reading peer is never judged deaf), judged without waiting, so it stalls no other peer and no shutdown; garbage is resynced past and counted; a bind failure is reported and the channel still stops.
- **Verification:** automated → crates/veredas-core/tests/telegram_channel.rs | over real loopback TCP: two channels both ways, the sequence and its wrap from 255 to 1, the connect keep-alive's 0 in both roles, a duplicate skipped, a keep-alive between two frames on a raw socket and a collision after 254 probes between two channels, the idle probe counted and silent while the peer talks, the departed peer, a 300-frame burst to a reading peer alone and beside three others, a 3 s stream, a deaf peer fed until the write timeout judges it (with a healthy peer never unread for 400 ms meanwhile, and a new peer served after), stop behind a backlog to four deaf peers, a peer reading 100 KB/s under an 8 MB burst never dropped while most of it is still queued, uncounted probes read off a raw socket carrying the last number, two uncounted channels across a whole cycle of probes never skipping, 254 keep-alives sent by the owner between two frames (uncounted and counted) and 254 empty messages of a segmentless layout between two data messages never costing one, two refused sends between two frames in a 1..=3 cycle (unicast and to all) taking no number, a segmentless layout's empty body delivered as a keep-alive with its field, a late listener, garbage before a frame, a port already taken; every channel configured by a channel file (the segmented envelope with sequence rules, or a bare layout), message ids neutral
- **Evidence:** .zen/evidence/C-012.md
- **Status:** verified

## C-013: Message bodies read into named fields by rules
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md; docs/questions.md Q8: encoding and decoding by rules)
- **Description:** A `CaptureSpec`, configuration rather than code (read from TOML text, optional keys taking their defaults, a misspelled key or type refused), reads a message body into named fields: a regex's named groups, binary fields at byte offsets in either order, and repeated patterns as `name1`, `name2`... up to a cap; a field past the body's end does not match and never panics; a binary field wins a name collision; a bad pattern is a warning that drops that one capture.
- **Verification:** automated → crates/veredas-core/tests/telegram_capture.rs | Veredas 1's tests, plus every field type's exact value in both orders, fields running past the end by one byte, a repeat's own cap and a repeat without a group, one bad pattern among good captures, a binary field over a repeat, a spec read from TOML with its defaults and equal to the literal one, all seven type names read and wrong spellings refused, a misspelled key refused in each of the three tables (the scanner-request tests left with the GENIBELT path, Q8)
- **Evidence:** .zen/evidence/C-013.md
- **Status:** verified

## C-016: A deterministic PCG32
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md, test-fixture plumbing)
- **Description:** `Pcg32` is the reference PCG32 (XSH-RR 64/32): it replays the same sequence from the same seed and stream, and diverges across streams, a stream id taken modulo 2^63 as in the reference; `next_f64` is in [0, 1); `uniform` stays within any finite range at its rounding, never NaN; `int_in` covers both ends of any `i64` range, never overflowing; `flip` gives both faces.
- **Verification:** automated → crates/veredas-core/tests/rng.rs | Veredas 1's tests, plus the reference demo's output for seed 42 stream 54, a thousand mixed draws replayed, streams 0 and 2^63 equal, ranges wider than `i64`, both ends at the extremes, `uniform` over (-f64::MAX, f64::MAX) from a zero draw, `next_f64` exact on the reference draws, 0 from the zero state and under 1 over a million draws
- **Evidence:** .zen/evidence/C-016.md
- **Status:** verified

## C-017: The channel file: a telegram channel as versioned, validated plain text
- **Date:** 2026-11-05
- **Source:** user (docs/questions.md Q9, Q10; docs/veredas-plan.md "Versioned files"; P-010)
- **Description:** A telegram channel is configured by a TOML file stating `format = "veredas-channel"` and `version = 1` (`docs/channel-schema.md`): its framing (delimiters, envelope, segment header, tail, byte order, frame limit), its sequence rules, its keep-alive rules and its link rules, every optional key with its documented default. `schema::load` refuses another format, an unknown or missing version, a misspelled key, an unknown role, a size out of 1 to 8, a role in the wrong part or twice, a computed field given any value (0 included), a default too wide for its field, a name used twice, a layout that cannot end, an empty or non-hex delimiter, a frame limit below the fixed bytes or over 16 MiB, a sequence field without `[sequence]` rules (an empty `[sequence]` takes the defaults), sequence rules with no sequence field, out of order, a cycle of one number that skips duplicates, or a `connect` inside the cycle (`docs/questions.md` Q11), a zero write timeout, an outbox smaller than one largest frame, and no peers, naming the place; it never guesses. The worked example in the documentation loads and frames as its prose describes it.
- **Verification:** automated → crates/veredas-core/tests/telegram_schema.rs | the documentation's own example block loaded and checked field by field (envelope, segment header and tail as name, size and role), its 11-byte envelope, 3-byte tail, 16-byte keep-alive and 22-byte data frame checked against the prose that states them; every default pinned, a per-field byte order, every limit accepted at its edge (`connect` just outside the cycle on either side, the frame limit at exactly the fixed bytes, one peer), the keep-alive and sequence keys read away from their defaults, a segmented layout ended by its length, its count or both, the framing byte order reaching the segment header and the tail byte for byte, every width at its top (8-byte fields, the widest value of a byte, a 2-byte cycle to 65535), the header (format, version) refused four ways, misspelled keys in every section (`[sequence]` included), and every invalid layout with its exact message (the fixed bytes counted with a tail, a `connect` too wide for its field, every role in every part that does not take it, a count given a value, a default too wide for a 2-byte field included); each of 143 one-line mutations killed by this test alone: of the reader (every check in every part, every role list, every copied field, default and inheritance, every `deny_unknown_fields`, the header compared exactly, multi-byte delimiters) and of the documentation (every number of the worked example and its segment id, every default, floor, limit, range, the format and the version of the key reference, each read from the text and compared with the reader)
- **Evidence:** .zen/evidence/C-017.md
- **Status:** pending

## Pending Contract Deltas

### P-002: veredas-portal — the gateway
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-portal: the gateway")
- **Description:** The single point of exchange between the suite and the outside world: the data plane (tailing, log families, regex capture, and telegram channels configured by encoding and decoding rules over core's socket, never vendor code: a device such as GENIBELT is a versioned plain-text rules file in a project; `docs/questions.md` Q8), the DuckDB session store, SSE, windowed REST and the gated write surface (whitelisted tags, master switch off by default), the desktop shell, and a light embeddable build for controlador. Absorbs Veredas 1's `veredas-core` (the data plane), `veredas-store`, `veredas-server`, `veredas-desktop`. Exit: `docs/portal-plan.md`, `crates/veredas-portal` at 2.0.0-alpha.1 building and tested against maquinar, the laws green, its capabilities as clauses.
- **Status:** pending

### P-003: veredas-controlador — the softPLC
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-controlador: the softPLC")
- **Description:** ORCL programs in a deterministic scan cycle with watchdog and safe outputs; direct NIC access on the real-time path (the only production path past portal); PLC Hardware Configuration on an embedded aranha; an embedded portal-lite for logging and non-real-time protocols; states which ORCL versions it runs. Builds on Veredas 1's `orcl-std`, `orcl-runtime`, `demos/conveyor` (ORCL keeps its own SemVer and `docs/orcl/`). Exit: `docs/controlador-plan.md`, the ORCL crates and `crates/veredas-controlador` at 2.0.0-alpha.1 with a simulated fieldbus in maquinar, the laws green.
- **Status:** pending

### P-004: veredas-maquinar — the bench
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "veredas-maquinar: the bench"; decisions 1, 2)
- **Description:** Simulated hardware so every consumer meets its plant on a bench first: log emitters, a configurable OPC UA server, TCP telegram channels up to a full simulated controller driven by the same rules a portal channel is configured with (never vendor code; `docs/questions.md` Q8), automated replies (random or rule-based), controllable pressure (rates, no-reads, full chutes); later a 2D/3D plant simulator. Owns its sockets; speaks the real device's wire; test-only and light, no DuckDB, never portal's store or desktop. Absorbs Veredas 1's `veredas-maquinar` and `veredas-trajetorias`' automated replies (its store is not carried). Exit: `docs/maquinar-plan.md`, `crates/veredas-maquinar` at 2.0.0-alpha.1, the laws green.
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
- **Description:** Every plain-text config and project file of the suite is TOML stating its `format` and `version` (misspelled keys refused; the operator, `docs/questions.md` Q10: "TOML, suite-wide"); portal migrates it or refuses it, and never guesses; Veredas 1's JSON project files are not carried as they are, what portal absorbs of them is read into TOML. A portal capability (P-002), parked apart because every component's files obey it and the law that checks the stamp belongs to core's file schemas. First instance: the channel file (`veredas-channel` version 1, C-017, `docs/channel-schema.md`, the pattern the others follow), read by core, which refuses an unknown format or version; migration of an older version and the law over every file wait for portal.
- **Status:** pending

### P-011: Suite releases
- **Date:** 2026-11-05
- **Source:** user (docs/veredas-plan.md "How releases work")
- **Description:** A suite release is a manifest that pins a tested set of component versions, shipped and installed together; a MAJOR bump of `veredas-core` means every dependent is rebuilt and retested before its own next release. Exit: the manifest format, the release recipe, and `2.0.0-alpha.1` cut with every component on the scheme.
- **Status:** pending
