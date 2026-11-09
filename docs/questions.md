# Open questions and recorded interpretations

Questions raised mid-build, with the option taken so the work could continue (the CLAUDE.md rule:
take the most reversible option, mark it `// DECISION PENDING` in code, keep going). Answered items
stay as the record of WHY something is shaped the way it is. The decisions of the 2026-11-05 review
are in `docs/veredas-plan.md`, "Decisions log".

---

## Q1 — Where do the suite's laws live?

**Raised** 2026-11-05, laying the base.

The plan's structural rules (members under their roots, naming, versioning, core alone, dependency
direction, plan before code) are checkable from the shape of the workspace. Candidates: a crate
under `tools/`, integration tests inside `veredas-core`, a script.

**Taken (operator):** `tools/leis`, a workspace member that is repo tooling, not a component (no
`veredas-` prefix, like `acerto` and `abaco-oracle` in Veredas 1; exempt from the component laws).
The laws are pure functions over parsed manifests, so a refute round can attack them on synthetic
suites; `cargo test -p leis` is the verdict. Core stays a library that knows nothing of the suite's
layout.

**Reversible?** Yes: a `git mv` and the clause links.

---

## Q2 — The review deck in the repo?

**Raised** 2026-11-05. `docs/` held the deck as `Veredas 2.html` (625 KB) and `Veredas 2.pptx`.

**Taken (operator):** both committed, renamed to ASCII (`veredas-2-review.html`, `.pptx`), as the
deliverable of the 2026-11-05 review. The markdown plan is the authority; the deck is not updated
when the plan changes.

---

## Q3 — A session log, as Veredas 1 kept?

**Raised** 2026-11-05.

**Taken:** none. The contract records what each capability became and when it was proven, the
evidence files hold the research, git holds the rest (the PL5 practice in `_Openn5`).

**Reversible?** Yes: a file.

---

## Q4 — When the plan and the contract disagree?

**Raised** 2026-11-05, seeding the contract from the plan.

**Taken:** the plan wins on intent (what a component is for, the rules); the contract wins on
status (what is proven, by which test). A clause states its rule in one or two sentences and names
the plan section; it never copies the plan's prose, so there is one place to edit. A law in
`tools/leis` changes only with its clause.

**Reversible?** Yes: wording.

---

## Q5 — May maquinar depend on aranha?

**Raised** 2026-11-05 by the refute round of C-005: the law's map lets maquinar use aranha, and
`docs/veredas-plan.md` said nothing about it.

Veredas 1's bench emulates PROFINET devices (DCP Identify, LLDP) with aranha's codecs, pulled with
its service feature off, so the scanner and the bench cannot drift apart on the wire. The
architecture rule (test tools own their sockets, past portal) is not touched by it.

**Taken:** kept. The plan's maquinar rules now say it, and the map's row stays.

**Reversible?** Yes: one row of `MAP` and one line of the plan.

---

## Q6 - The GENIBELT vendor specification

**Raised** 2026-11-05, porting the telegram layer into core (P-001).

Veredas 1 keeps the GENIBELT / Master Sort Control host interface specification machine-locally
at `docs/vendor/genibelt-protocol.txt` there, gitignored because it is proprietary; the code
cites its section numbers. Veredas 2 needs it for reading, verifying and extending the catalog,
not for building.

**Taken (reversible, pending the operator):** not copied. `docs/vendor/` is gitignored here as
there (its README says what belongs in it); the code keeps citing section numbers; the constants
are in code and pinned by the telegram tests, so a machine without the file builds and tests.
To read the spec, use the copy beside Veredas 1 or drop one into `docs/vendor/` here.

**Reversible?** Yes: a file and an ignore rule.

**Since Q8 (2026-11-05):** core names GENIBELT nowhere and cites none of its sections; the
specification is for writing the rules a portal channel is configured with.

**Decided** 2026-11-09 (the operator): not in the repo. `docs/vendor/` stays gitignored with its
README; whoever writes the device's rules file reads a local copy or the one beside Veredas 1.

---

## Q7 - Do the TRISTATE bin-state bits stop a chute?

**Raised** 2026-11-05 by the refute round of C-011: the GENIBELT specification (§4.2.1.4) names
bits 28-31 of a bin state (MAPPING, EMERGENCY_TRISTATE, ALARM_TRISTATE, WARNING_TRISTATE) without
saying what they mean for parcel acceptance. Veredas 1 named none of them.

**Taken (reversible, pending the operator):** they are named (a state word reads as words), and
`accepts_parcels` ignores them, as Veredas 1 did, so no chute that sorted before stops sorting.
Marked `// DECISION PENDING` in `veredas_core::telegram::genibelt::bin_state`.

**Reversible?** Yes: one constant set in `accepts_parcels`, and the test that pins it.

**Moot (2026-11-05, Q8):** the bin-state words left core with the GENIBELT catalog; whether these
bits stop a chute is a question for the rules a portal channel is configured with.

**Closed** 2026-11-09: moot since Q8 (the operator's decision of 2026-11-05); nothing is left to
decide in the suite, the question belongs to the device's rules file.

---

## Q8 - Does GENIBELT belong in core?

**Raised** 2026-11-05 by the operator, while the telegram layer was being ported into core.

**Taken (operator):** no. "genibelt is a machine my company manufactures and the interface via
TCP socket should only be something veredas-portal manages, using encoding and decoding rules
and storing the data in the database, as per the other protocols. veredas shall be a generic
platform to build on top of, which reliably manages communication with the outside world, while
being content-agnostic. genibelt does not belong in core, a tcp socket configurable for it
does." Core keeps the hardened TCP socket (`telegram::channel`), its framing (`telegram::codec`)
and the rules engine that reads message fields by configuration (`telegram::capture`), named
for no device. The GENIBELT catalog, the sort plan engine, the decision client and the scanner
path of the capture layer leave the crate and are not carried into the suite as code: they
become the rules a portal telegram channel is configured with (plain text, versioned, in a
project folder), and maquinar's simulated controller is driven by the same rules. Veredas 1
keeps its own. Recorded in `docs/veredas-plan.md` (core, portal, maquinar, the crate table and
the decisions log) and in the contract (C-007, P-002, P-004; C-011, C-014 and C-015 shelved).
The refute rounds' fixes to the carved modules stay in this repo's history (commit b7c6d0a) for
whoever ports them to Veredas 1.

**Reversible?** Yes: the modules are in history, and the rules can be compiled back into code.

---

## Q9 - The framing is still one device's shape

**Raised** 2026-11-05, carving GENIBELT out of core (Q8).

The framing core keeps (`telegram::codec`) is generic in what it carries but not in its shape:
the start and end bytes, the 12-byte envelope with its sequence number, the data-area length
and the segment count, the segments of id, length and body, and the 4-byte tail are the layout
of the GENIBELT host interface. The sequence and keep-alive rules the channel applies (sequence
0 on connect then 1..255, a repeated number skipped, a keep-alive probe after silence, a
keep-alive's number counting as the last seen) are that protocol's rules too. Q8 asks for a
framing "driven by configuration (delimiters, envelope, length fields, endianness, sequence
stamping and duplicate skipping as rules)"; designing that is new work, not a split.

**Taken (reversible, pending the operator):** the framing and the channel stay in core as they
are, with the vendor's name and section citations removed and the shape described in their
docs; no other framing exists yet to generalise against. When the first telegram channel's
configuration schema is written (portal, P-002), the framing's constants become that schema's
fields (start/end bytes, envelope layout, length and count fields, byte order, sequence and
keep-alive rules as switches), and the current shape becomes one configuration of it. Open
point for that schema: whether a device's keep-alives consume sequence numbers (the channel
assumes they do: a keep-alive's number counts as the last one seen).

**Reversible?** Yes: a configuration schema over the same code, or removing the framing from
core altogether.

**Answered (operator, 2026-11-05):** "Design configurable framing now". Core keeps no frame
layout of its own. `telegram::framing` is an engine that encodes and decodes any layout a
channel file describes (optional start and end delimiters; envelope, segment-header and tail
fields by size, byte order and role: sequence, length, count, opaque); `telegram::schema` reads
and validates that file (`docs/channel-schema.md`); the channel's sequence, keep-alive and link
rules are sections of the same file. The layout this question was about is the schema's worked
example and the engine's first test fixture, named for its shape (a segmented envelope), not for
its maker: a 12-byte envelope with a sequence, a length and a count is a shape, not a secret.
The device's own file lives with its other rules, outside the suite's code.

---

## Q10 - How a channel file is written

**Raised** 2026-11-05, designing the channel schema (Q9). The plan fixes that every plain-text
file states its schema version and that a reader migrates or refuses; it does not fix the
syntax.

**Taken (reversible, pending the operator):**
- TOML, the suite's existing configuration syntax (the workspace manifests); a file states
  `format = "veredas-channel"` and `version = 1`, and a reader refuses another format or an
  unknown version.
- A field declares its meaning with `role = "sequence" | "length" | "count"`, default
  `"opaque"`; an opaque field may carry a default `value`; sizes are 1 to 8 bytes.
- Delimiters are hex strings (`start = "FD"`), parsed like a typed-in body.
- The behavioural rules are their own sections (`[sequence]`, `[keepalive]`, `[link]`), every
  key with a default; a misspelled key is refused, never ignored.
- The capture rules (`CaptureSpec`) read from the same syntax, their type names spelled as
  Veredas 1 spelled them (`U16`, `Ascii`, `Hex`).

**Reversible?** Yes while no project file exists: a version 2 of the format, migrated by portal.

**Answered (operator, 2026-11-05):** "TOML, suite-wide". The channel file stays as designed, and
the rule is wider than that file: every plain-text config and project file of Veredas 2 is TOML,
stating its `format` and `version` as the channel file does, misspelled keys refused. Veredas 1's
JSON project files are not carried as they are; whatever portal absorbs of them is read into
TOML. Recorded in the plan ("Versioned files"), in P-010, and in `docs/channel-schema.md` as the
pattern the other files follow.

---

## Q11 - May `connect` lie inside the sequence cycle?

**Raised** 2026-11-05, refuting the channel (C-012 round 5, C-017 round 2). A receiver tells the
connect keep-alive from the others by its number alone. When `connect` lies inside
`first..=last`, a keep-alive that takes that number in the cycle is not counted as seen, and in
a two-number cycle the next data frame repeats the last one and is skipped.

**Taken (reversible, pending the operator):** the channel file refuses a `connect` inside
`first..=last`. Every layout seen so far (the worked example: `connect = 0`, cycle 1 to 255)
keeps it outside. A device that needs it inside would need the receiver to exempt only the
first keep-alive after connect instead, a version 2 of the rule.

**Reversible?** Yes: loosening a refusal breaks no file that loads today.

**Decided** 2026-11-09 (the operator): keep refusing. A `connect` inside `first..=last` is refused
by the channel file; the pending note in `veredas_core::telegram::schema` becomes the rule's
comment.

---

## Q12 - Does the Veredas 2 bench have a desktop UI?

**Raised** 2026-11-06, planning maquinar (P-004).

Veredas 1's bench is an egui desktop (`app.rs`, four `ui_*.rs`, about 2 700 lines on `eframe`):
a project bar, tabs for logs, OPC UA, TCP and PROFINET, live previews and monitors, chaos buttons
(every chute full, a random pick), and a JSON project it saves continuously. Whether the Veredas 2
bench has a desktop UI at all, or is a library plus a TOML-driven executable driven from the
command line (with its knobs on the library handle, which is what portal's tests and later
comissionador embed), is the operator's call.

**Taken (reversible, pending the operator):** a library first, with one executable that runs a
bench file (`veredas-maquinar <bench.toml>`) until stopped; the UI is a later phase of
`docs/maquinar-plan.md`, named there with no code. Every operator action the Veredas 1 UI had
(start and stop a part, send a message now, set a destination's state, declare every destination
full, issue a command) is a method on the bench's handles, so a UI, when it comes, drives the
same library the tests do.

**Reversible?** Yes: a UI is a crate or a module on top of the library; nothing below it changes.

**Decided** 2026-11-07 (the operator): library only. The bench is the library and the
TOML-driven executable; no desktop UI is planned, and `docs/maquinar-plan.md` no longer names
one as a later phase.

## Q13 - Should the bench raise the Windows timer resolution?

**Context:** the bench ticks every 20 ms (the OPC server's service loop; the channel's
controller tick). On Windows a timed wait lands at the system timer quantum, 15.6 ms, so a
20 ms tick lands about 31 ms apart (measured: a tag drawn every 20 ms drew 63 times in 2 s on
Windows, where Linux draws 100). One call, `timeBeginPeriod(1)` from `winmm` (an `unsafe` FFI
call, process-wide, a power cost while it holds), would make the tick 20 ms. The rates hold
either way: every due time is scheduled from the last due time, never from the tick that
served it, so a 100 ms draw is 20 in 2 s on both platforms and only the jitter differs
(C-026, round 4; `docs/maquinar-plan.md`, "Traps").

**Taken (reversible, pending the operator):** no FFI. The clause states the quantum, the tests
pin the rate on both platforms and the per-tick figure per platform, and the bench's two tick
loops carry a `DECISION PENDING` note. Raising the resolution is one call at the start of the
executable, behind `cfg(windows)`, when the operator wants 20 ms ticks on the bench.

**Reversible?** Yes: one call, one place, nothing else changes.

**Decided** 2026-11-09 (the operator): yes. The bench executable raises the Windows timer
resolution with `timeBeginPeriod(1)` at its start, behind `cfg(windows)`, paired with
`timeEndPeriod` at exit; the two tick loops lose their pending notes, and the clause and the bench
plan state a 20 ms tick on both platforms.

## Q14 - What is the Veredas 2 desktop shell?

**Raised** 2026-11-07, planning portal (P-002).

Veredas 1's `veredas-desktop` is an egui application of about 21 000 lines (`app.rs`, `boot.rs`,
`style.rs`, fourteen `ui/*.rs` screens: projects, start, trailing, series, grep, viewer, tables,
the OPC explorer, the layout and SQL editors) over the same session, store and server that portal
absorbs. The plan names "the desktop shell" in portal's scope without saying whether Veredas 2's
shell is a port of that desktop, a thinner shell over portal's server (a window that starts a
project and shows health, the operator working in the browser), or no desktop at all, supervisão's
web interface (P-008) being the suite's one interface. The grep tab, the trailing view and the
table browser are operator tooling that goes where the shell goes.

**Decided (2026-11-09, the operator): the executable stands.** Portal is a library and one executable,
`veredas-portal <project.toml>`, which serves a project from the command line until stopped; no
desktop UI is built. Every operator action the Veredas 1 shell had on a session (start, stop, add
or remove a source, arm the writes, snapshot, export) is a method on the session's handle, so a
shell, if one ever comes (P-008's web interface, or a thin window over the server), drives the same library the tests do; none is built in portal. The grep engine and the skew monitor
wait with it (`docs/portal-plan.md`, "Scope").

**Reversible?** Yes: a shell is a crate or a module on top of the library; nothing below it
changes.

---

## Q15 - Where does the suite keep a secret?

**Raised** 2026-11-07, designing portal's project file (P-010).

Veredas 1 kept an OPC UA password in the Windows Credential Manager (`keyring`, keyed by endpoint
and user), so a project folder could be committed and shared. Veredas 2's project file is TOML in
a project folder, meant for git like Veredas 1's, and runs on Linux in CI: a password cannot go in
the file, and a platform vault is one platform's.

**Taken (reversible, pending the operator):** the project file names the user and the name of an
environment variable that holds the password (`password_env`); portal reads the variable when the
session starts and refuses to start a source whose variable is unset, naming the variable, never
the value. No vault, no secrets file.

**Reversible?** Yes: a second source of secrets (a vault, a file outside the project) is one more
way to fill the same field.

**Decided** 2026-11-09 (the operator): an encrypted secrets file in the project folder, committed
with the project, and the key in an environment variable the project file names. Portal reads the
key when the session starts, decrypts the file in memory, and refuses to start a source whose
secret is missing, naming the entry, never a value. `password_env` gives way to the secrets file;
the file's format, the cipher and the tool that writes an entry are portal's design (P-010,
C-046), recorded in `docs/project-schema.md`.

## Q16 - Where does a project's disk store go on a second run?

**Raised** 2026-11-09, the store's first refute round (C-041).

A project fixes its store's disk path (`[store] path`), and a session store is fresh: its tables
are created at open. A second run of the same project finds the first run's database at the
path; opening it would put the new session's rows beside the old ones (or fail on the engine's
table collision, as it did by accident). The choices: refuse by name until the operator moves the
file; open fresh by deleting it; or name the file per session (the project's name and the start
time) so every run leaves its own.

**Taken (reversible, pending the operator):** refuse by name. The store's open says a database
already stands there and that a session store is fresh; nothing is deleted, nothing merged.

**Reversible?** Yes: a per-session file name or an opening mode is one line in the project's
store section, and the store's check stays as the guard for the fixed-name case.

**Decided** 2026-11-09 (the operator): a file per session. The store's file is named from the
project's name and the session's start time, so every run leaves its own database and nothing is
refused, deleted or merged; the refusal stays as the guard when a project pins a fixed name and a
database already stands there.

## Q17 - Should a linked CSV with ragged rows be read through the padded path?

**Raised** 2026-11-09, the engine pins' refute rounds (C-042, rounds 4 to 10).

A linked CSV is read live through a view the store builds from the engine's own sniffer
(`read_csv_auto`), and the store forces the padded reader (`read_csv` with a delimiter and
`null_padding`) only where the sniffer collapsed the file to one column. On a ragged file the
sniffer has more outcomes than a sentence lists (C-042 pins one of each: a collapse, a row kept
alone, a run kept as data, a run's first row taken as the header, the sniff refused, the read
refused past the sample), and two of them lose or rename an operator's rows in silence. The
padded path's behaviour is general: every row kept, a short one padded, a long one widening the
schema within 2 048 lines and refusing the read by its line past them.

**Taken (reversible, pending the operator):** the sniffer stays the first reader, its outcomes
stated and pinned; the padded path is forced on a collapse alone, as C-041 says.

**Reversible?** Yes: forcing the padded path for every linked CSV, or for any file the sniffer
reads ragged, is one branch in the store's CSV scan and one sentence in C-041.

**Decided** 2026-11-09 (the operator): the padded path for any ragged file. The sniffer stays the
first reader; whenever it reads a file ragged (any outcome but a regular table), the padded reader
takes the file: every row kept, a short one padded, a long one widening the schema within 2 048
lines, the read refused by its line past them. No row is lost or renamed in silence.
