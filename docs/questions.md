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

**Revisited** 2026-11-14 and 2026-11-15 (the operator, the briefing on controlador): the functions
that drive hardware live in `veredas-core`, so aranha's PROFINET codecs move into core when the
controller needs them, and the bench then takes them from core. Aranha's row for maquinar in `MAP`
stays until that move is made (`docs/core-plan.md`, "Decided in the briefing of November 2026").

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

**Revisited** 2026-11-15 (the operator, ruling on the review's entry 10 of the briefing: "Core,
rules amended"): the fieldbus masters stay in core, as decided on 2026-11-14, with the codecs of
their standards (PROFINET, EtherCAT, EtherNet/IP, Modbus TCP). What this question settled is the
telegram framing: core's framing engine holds no frame layout of its own, and a device spoken over
a TCP telegram channel is still configuration. A standard fieldbus's frames are fixed by its
standard and are code in core's masters; no vendor's device or catalog is named there either. The
ruling asked for this rewording and gave none: the wording is the assistant's, written on
2026-11-19 with `docs/veredas-plan.md` and `docs/core-plan.md`.

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

**Revisited** 2026-11-13 (the operator): the decision stands for the bench itself, which stays a
library and an executable. The bench's surfaces (its file editor, a traffic monitor, starting a
bench beside a session, later the pressure panel) are Oficina's (`docs/oficina-plan.md`), driving
the same handles the tests do.

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

**Revisited** 2026-11-13 (the operator): portal still builds no desktop UI. The shell this question
left open has come as a component of its own, Oficina, the suite's one native application
(`docs/oficina-plan.md`): it embeds a portal and drives the same session handle. The grep engine is
on Oficina's first-release list.

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

## Q18 - How is Veredas 1's export parsing carried?

**Raised** 2026-11-09, planning supervisão (P-008).

Veredas 1's supervision app compiles its plant model by parsing one company's engineering exports
and control-software project files (its "Layer P": the sorter tables, the line and conveyor
exports, the tag and alarm dictionaries, a mechanical workbook). That collides with the
content-agnostic rule (Q8): such parsing is not suite code. The choices: not at all; an outside
tool that writes the binding file; or a generic importer inside supervisão driven by rule files.

**Taken while asking:** the plant model comes only from plain-text TOML a person or an outside tool
writes (the binding file, `docs/binding-schema.md`) and the SVG layout it points to.

**Decided** 2026-11-09 (the operator): **not at all.** No export reader is carried, in the suite or
beside it; binding files are written by hand or by anything, and supervisão reads nothing else.

## Q19 - Where do supervisão's sessions come from?

**Raised** 2026-11-09, planning supervisão (P-008).

Either each session is a `veredas-portal` executable already running, which supervisão reaches over
HTTP and owns nothing of, or supervisão runs the portal sessions in its own process from project
files.

**Decided** 2026-11-09 (the operator): **supervisão starts them.** It loads each plant's project
file and starts its session through portal's library, one executable serving every session; the
session's portal router is served under supervisão's address (`/s/<name>/portal/`). Supervisão
still owns no plant connection: every value and every write is portal's.

**Reversible?** Yes: a session reached over HTTP is a second way to fill the same slot.

## Q20 - Who may open supervisão?

**Raised** 2026-11-09, the design's open point "session and permission model for several sessions
on one host".

**Decided** 2026-11-09 (the operator): **local, no accounts.** The host binds the loopback by
default; whoever reaches it sees every session alike; a write rests on portal's switch (armed by
the project, never over the wire) and on the actions the binding file declares.

**Reversible?** Yes: accounts or a read-only remote mode sit in front of the same routes.

**Revisited** 2026-11-13 (the operator): it stays, and it is extended to remote targets: Oficina
reaches a target through SSH and has no accounts of its own. What a login adds is left to the
security round (Q43, P-014).

## Q21 - The 3D view now or later?

**Raised** 2026-11-09, planning supervisão (P-008).

**Decided** 2026-11-09 (the operator): **later**, after the 2D layout, the dashboards, the gated
writes and the several sessions are verified. The binding file's object IDs are the ones a glTF
model's nodes will carry; no 3D key exists in the binding format until the view does.

## Q22 - Which accent does supervisão draw?

**Raised** 2026-11-09, by the operator, looking at the first page.

The design references (`docs/design/README.md`, `docs/design/supervisao/`) gave the web side a
magenta accent, after Veredas 1's dashboard, and the desktop side green.

**Decided** 2026-11-09 (the operator): "veredas accent color shall be green (see veredas-desktop
1)". Supervisão draws Veredas 1's desktop green (`#30D158`, soft `#1C3A27`, outline `#2F5A3D`,
hover and data series `#5FE07F`, on-accent text `#08130B`); the mockups' magenta reads as the
accent wherever it appears. The accent stays what it was: selection, focus, primary actions and
neutral data series, never a plant state.

**Reversed** 2026-11-15 (the operator): "Use Lavanda #B9A9E8 as the suite accent colour". The green
accent "has been an inconvenience": green is to mean a good state, so the accent became a colour
that is memorable and carries no ready meaning (not blue, green, red, yellow or white). Lavanda
`#B9A9E8`, soft `#2B2838`, outline `#494360`, on-accent text `#12101C`; the hover and data-series
tint `#CFC3F2` is the assistant's proposal. Green `#30D158` is freed to mean a good state. NOT
applied yet: supervisão's stylesheet, its built page, `docs/design/README.md` and any test that
names the colour still carry the green (P-021). The syntax palette's number colour (`#D2A8FF`) sits
too close to the new accent and should move. Whether "good" is a new state class or the colour of
the normal one is open (Q45).

## Q23 - May a layout's stylesheet load from another host?

**Raised** 2026-11-11, by C-048's fourth refute round.

The layout reader refuses script in every form it found (elements, handlers, `javascript:` links,
entities expanding to markup, `xml:base`), but a `<style>` inside the SVG may `@import url(...)` a
stylesheet from another host, and a layout's style applies to the whole page once mounted. The
page itself loads nothing but its own files (C-047); a layout is the plant engineer's own file,
served by the host. Neither the clause nor `docs/binding-schema.md` promises to refuse outside
content in a layout.

**Taken meanwhile** (the most reversible): nothing refused; the reader stays as it is. If the
answer is "refuse", it is a new refusal in the reference (an `@import` or a `url()` outside the
document in a layout's style) with its exact text, and C-048 states it.

**Decided** 2026-11-15 (the operator, asked "Should the page be barred from loading anything a
layout points to on another host?"): "Yes, bar it" (the other choice read "Leave it open"). Two
layers, one job each. The reader, for the engineer: a picture is embedded (an image's `href`,
plain or `xlink:`, a `data:image/…` URI, Inkscape's "Embed"), any other refused with what to do
in Inkscape; the reader's rule stays that small, no list of every syntax that can fetch. The host,
for the guarantee: it serves the page under a Content-Security-Policy that lets the browser load
and run the page's own files alone (its bundle the one script, no inline script or handler, its
own sheet and the drawing's `style` attributes, pictures its own or embedded, connections to its
own address, fonts its own, no base address moved, a form sent nowhere but here), and the
drawing's own route under one that runs and loads nothing but its embedded pictures, sandboxed;
every answer `nosniff`. The claim that nothing a drawing writes makes the page load from another
host or run script rests on the header, proven in the browser by putting every kind of load and
script straight into the page and the drawing (C-049). Limits: a link navigates on a click and
loads nothing by itself; the browser may open a bare connection to a frame's host (its preconnect)
before its policy refuses the frame, asking nothing; portal's nested surface answers with its own
headers; the policy is the browser's to enforce. Another site framing the page was not barred by
this ruling; the operator barred it on 2026-11-15 (a site open in the same browser could lay the
page under its own clicks): both policies say `frame-ancestors 'none'`, so no page shows the page or
the drawing inside a frame, and a page meant to frame supervisão's would have to be named there.
(Before the ruling, while Q25 stood, the question had no effect on a sheet: the reader refuses a
layout's `<style>` sheet altogether, and Inkscape writes none.)

## Q24 - Does a timeline replay a rule over another series?

**Raised** 2026-11-14, by C-052's ninth refute round.

An object's state may come from a rule over another series (`source`, `series`, `key` on the
rule); the feed reads that series' latest value, so a tile or a grid cell shows it. A timeline
replays the object's history from the store, its own series alone: such a rule's states were left
out, the row drawn "normal" where the cell beside it said "alarm". Replaying it means merging the
histories of every series the rules read, and a text or a time series has no numeric window the
store serves today.

**Taken meanwhile** (the most reversible): an object with a rule over another series is refused on
a timeline, by name, and the reference says so; a rule that writes the object's own series out in
full reads what the timeline replays, and stands. If the answer is "replay it", the refusal goes and
the timeline merges the series' histories (numeric ones first), the test driving both kinds of rule.

**Decided** 2026-11-15 (the operator, asked "Keep refusing, or replay such rules?"): "Keep
refusing": a timeline shows objects whose rules read their own series; replay can come later. The
refusal stands as built and tested (C-048, C-052), its `DECISION PENDING` mark gone.

## Q25 - What drawing does the layout draw objects on?

**Raised** 2026-11-14, by C-051's tenth refute round.

The reader took any element of the SVG namespace (bar a script and foreign content) as an object's
element, with any paint syntax and any coordinate construct. The page could not draw all of them as
the reference says: paint written in a `style` attribute, a `<style>` sheet or a `use`d symbol
outranked the state's; a `tspan` was never filled; an element inside a nested `svg` was measured in
the wrong frame; an object's element inside another took the outer one's stroke and selection. A
drawing's sheet also styled the page's every screen.

**Taken meanwhile** (the most reversible): the page draws on a named subset and the reader refuses
the rest by name. An object's element is a `g`, one of the seven shapes or a `text`, inside no
other object's element, with nothing but groups between it and the drawing's root (a link would
take its click), holding nothing but groups, shapes, texts and their spans (C-051's eleventh round:
a link inside takes the click, a nested viewport keeps its own paint); a marker's path stands inside
nothing but groups either. The drawing holds no `class` (with no sheet of its own it meets only the
page's rules), no id the page names its own, and no CSS that moves, times or unpins what the page
writes (a transform property, a transition, an animation, a motion offset, `pointer-events`; an
element's own `transform` attribute is kept, the page reading it, but not the root's, which the
page lays its own frame over). Paint is
written on the elements (attributes or `style`, never `!important` on or inside an object's
element, however CSS spells it), and the state's paint outranks it at any depth of a group. A
`<style>` sheet is refused, and so is any animation (C-048's tenth round: an animated class or
transform undoes what the page writes): the drawing is still, the page animates it. If the answer is
"draw more", each refusal goes with the page learning that construct (a sheet scoped to the drawing,
a `use` painted through its shadow tree, a nested viewport measured through its own frame), the test
driving it.

**Decided** 2026-11-15 (the operator, asked "Which tool will draw the layouts?"): "Inkscape": the
reader takes Inkscape files as saved, its editor-only elements dropped on read; style attributes,
text spans and matrix transforms kept. The drawing language is what Inkscape saves: its own
elements (`sodipodi:*`, `inkscape:*` beside the drawing or in its `defs`, the `rdf` block in
`metadata`, by namespace, `svg::EDITOR`) are dropped on read by the reader and the page, anywhere
else refused; its attributes are kept and read by nothing; every other foreign namespace stays
refused. The subset above stands (Inkscape writes no sheet, class, animation, script, moving CSS or
`pointer-events` unless its user goes out of the way). An object's element or a marker's path in a
hidden layer (`display: none`) is refused, the hiding element named; a clone or an image where an
object's element is refused with what to do in Inkscape (unlink the clone; draw the image as
shapes, or move it out of the group). A construct Inkscape does not write is answered at the
reader, never by teaching the page a new one. No Inkscape is installed here and the operator
supplies no saved file: the fixture is a drawing written by hand to Inkscape 1.3's format, and no
file saved by Inkscape has gone through the reader or the page (C-048 and C-051 state it).

## Q26 - Does supervisão's name check guard against deliberate hiding?

**Raised** 2026-11-15, by the orchestrator, from the trend of C-047's refute rounds.

After fourteen rounds the component check (no device, vendor, product or customer named in any file
under the crate; nothing compiled in from outside it; nothing brought into its graph but through
core and portal) was refuted only by constructions made to evade it: a name split by a character
entity nested eight levels deep, a file hidden from the check by a git index flag, a cargo
configuration placed in a folder between the crate and the repository's root. Each was answered,
and each answer invited the next.

**Taken meanwhile**: every construction answered as it came.

**Decided** 2026-11-15 (the operator, asked "Should supervisão's name check guard against accidents
only, not deliberate hiding?"): "Accidents only" (the other choice read "Deliberate hiding too").
The check is for a change made in good faith with ordinary tools: a name written as an editor, a
formatter or the build toolchain writes text; a dependency, a file or a source brought in the ways
cargo, the compiler and git work in ordinary use. It lists the forms it reads. Deliberate
concealment is outside it: a name encoded, split, nested or computed so that it is not read; a
tool's flags or configuration used to keep a file or a source away from the check. The orchestrator
reads the answer as the threat model of the whole clause, both its halves (the name scan, and what
is compiled in or brought into the graph), the examples the operator read coming from both.
Nothing built comes out: every check and test stays, the ruling ends the escalation. A refutation of
C-047 is a way a good-faith change gets past the check, named with the tool or the workflow that
produces it; a construction whose only purpose is to evade is no finding. The ruling is C-047's
alone: the reader's refusals of script and foreign content (C-048) keep their adversary, a layout
being a file from outside the repository.

## Q27 - What does the reader do with a mesh gradient Inkscape saves with its script?

**Raised** 2026-11-15, by C-048's thirteenth refute round (from Inkscape's source, not run here: no
Inkscape is installed).

When a drawing's `defs` hold a mesh gradient (Fill and Stroke offers one in a click), Inkscape 1.x
saves a script beside it, `<script id="mesh_polyfill">` as the root's last child, which draws the
mesh in a browser that cannot (Chromium among them). A preference governs it (Edit › Preferences ›
Input/Output › SVG export, "Insert JavaScript code for mesh gradients", on by default, as the
critic read the source); an SVG2 hatch gets a `hatch_polyfill` the same way. The reader refuses the
script as any script ("holds a <script> element: a layout is a drawing, with no script and no
foreign content"), naming a script the engineer never wrote and giving no step in Inkscape. Without
the script the mesh loads, and Chromium draws it as nothing (the host's policy would bar the
polyfill anyway, Q23).

The choices: (a) refuse a mesh gradient and a hatch by name, with the step in Inkscape (a flat,
linear or radial fill instead, or the preference unticked); (b) keep refusing the script as now and
state it as a limit; (c) drop Inkscape's polyfill script on read like its other editor elements and
let a mesh draw as the browser draws it (nothing).

**Taken meanwhile** (the most reversible): (b), nothing changed; the clause states the limit.

**Decided** 2026-11-15, by the operator: "Drop the script, load it", (c), against the recommendation.
The reader drops Inkscape's script beside a mesh gradient or a hatch (`svg::POLYFILLS`:
`mesh_polyfill`, `hatch_polyfill`) where and as Inkscape writes it, a `script` of the root's own
writing nothing but its id and a type, with all it holds, as it drops Inkscape's other elements;
the drawing loads, and the shape filled with the mesh or the hatch shows no fill on the page, with
no message. Any other script stays refused: the operator was asked about Inkscape's alone. The
script runs nowhere: the page removes every script before it draws, and neither of the host's
policies runs one (C-048, C-049).

## Q28 - Are a table's column headings drawn in capitals?

**Raised** 2026-11-16, by C-052's fifteenth refute round (the census reading text as drawn).

`docs/design/README.md` sets column headers in mono 600, uppercase, and the page drew every table
heading through `text-transform: uppercase` (`.dt th`). A heading drawn so says something else than
it was written: the cells' σ column (the standard deviation) is drawn Σ, the sign of a sum; a
binding's label carrying a unit loses its case (`Rate (kW)` as RATE (KW), `I (mA)` as I (MA), a
megaampere), and a Tables view column name is drawn in other letters than the project wrote. The
design's own mock does not capitalise by style: `docs/design/supervisao/Objects.dc.html` types its
cells headings in capitals in the markup (CHUTE, MEDIAN, MEDIAN, 60 MIN, DISTRIBUTION, N, NOTE),
with no `text-transform` in the file, and draws σ small.

The choices: (a) headings as written, the binding's labels and the page's own words alike (the
design's capitals dropped); (b) as the mock: the page's own words written in capitals in its source
(KEY, MEDIAN, DISTRIBUTION, N, STATE, OBJECT), σ small, a binding's label and a column's name as
written; (c) every heading in capitals by style, σ drawn Σ and a unit's case lost.

**Taken meanwhile** (the most reversible): (a), the one CSS property removed (`web/src/style.css`,
`DECISION PENDING (Q28)` beside it); the census holds every text a panel shows to be drawn as written.

**Decided** 2026-11-16, by the operator: "Page's own words only", (b), as the mock does it: "The
page's own headings are written in capitals (STATE, OBJECT, KEY, MEDIAN, N); the symbol σ, an
engineer's label and a view's column name are drawn exactly as written." An engineer who wants a
label in capitals writes it so. The page types its own headings in capitals in its source (the
cells table's KEY, MEDIAN, MEDIAN, WINDOW, DISTRIBUTION, σ, N; the alarms table's STATE, OBJECT,
VALUE, SINCE), the Tables view's type word as it writes it; no heading goes through
`text-transform` (C-052's census refuses any). `docs/design/README.md` says the same of column
headers.

## Q29 - Does the host answer a request named for another host?

**Raised** 2026-11-16, by C-049's fifteenth refute round (outside the clause; for the suite's owed
cyber-security round).

Neither supervisão's host (`host.rs`) nor portal's nested router (`serve.rs`) reads a request's
`Host` or `Origin`. A site whose own name an attacker points at 127.0.0.1 after the page has loaded
(DNS rebinding) becomes, to the browser, the same origin as the host: its script reads every
session's data and presses an armed write, with no frame at all, which neither content policy can
govern and which the framing ruling meant to bar. The page and portal answer any name that reaches
the address.

The choices: (a) answer only a request whose `Host` is the bound address or a name the supervisão
file lists (`hosts = [...]`), and refuse a write whose `Origin` is not the page's own; (b) the same,
the names taken from the machine's own (its host name, `localhost`); (c) leave it to the network
(a firewall, a reverse proxy) and state it as a limit.

**Taken meanwhile** (the most reversible): (c), nothing changed; C-049 states it as a limit.

**Decided** 2026-11-16, by the operator: "Leave it for the security round", (c); and, asked when
portal's own server (run without supervisão) is to be closed, "With the security round". Nothing is
built now: the host's gap stays a limit C-049 states, portal on its own is an open item of the
round, and portal's clauses stay verified. A firewall does not close it: the request comes from a
browser that is allowed to reach the address. The round's open items are P-014, which names both
ways the operator was asked about: a name re-pointed at the address (above); and a write a page of
any site posts as `text/plain`, which a browser sends to another site without asking it first and
which portal's write and send routes parse as JSON whatever its `Content-Type` (`read_body`,
`crates/veredas-portal/src/serve.rs`), while writes are armed, the page unable to read the answer.
The second is read from the code, not yet shown by a test. What the operator was told of browsers
is Chrome's own account of local network access: from Chrome 142 a page from the public internet
asks the user before it reaches a local or loopback address; a page on the plant's own network is
not covered.

## Q30 - Does a state paint the inside of a drawing's stroke-only shape?

**Raised** 2026-11-16, by C-051's eighteenth refute round.

A shape Inkscape saves with `fill:none` (a pipe drawn with the pen or the connector tool, an open
bent path; a ring drawn as a stroke) and bound to an object: the page outranks the drawing's paint
with the state's, `fill` among it, so in a warn state the pipe's bend is painted as a yellow
triangle the drawing never had, and a ring's inside is painted whole. A level on such a shape rises
inside the area its outline closes. (A shape with no area at all, a straight line, two walls Inkscape
combined, an L of separate runs, is filled over its strokes alone, C-051.)

The choices: (a) as now: the state's colour paints the area the outline closes, the clause and the
reference saying so for open paths and rings; (b) a shape the drawing writes `fill:none` on takes
its state on its stroke alone, and a level on it fills over its stroke (as a shape with no area does).

**Taken meanwhile** (the most reversible): (a), nothing changed; C-051 states it as pending on Q30.

**Decided** 2026-11-19, by the operator: "Its line, as drawn", (b), and the line keeps the width it
was drawn with. The line takes the state's colour and keeps the width it was drawn with (never
thinner than the page's own outline); nothing is painted inside, and a level runs along the line.
This is how the design mock shows a stopped induction (`docs/design/supervisao/Main.dc.html`, the
inductions drawn as lines 10 wide, the stopped one the same line in amber). An outline that should
be painted as a body needs a fill in Inkscape. Not chosen: the line redrawn at the page's own thin
width (1.4, which every bound shape's outline takes today, so a conveyor drawn 10 wide would show
its state as a hairline); the enclosed area painted, as now. Read with it (the foundation's
reading, not the operator's words): drawn with no fill is the fill computing to `none` as the
drawing stands before the page paints it, however it comes (an attribute, `style`, a group
above), and a fill of zero opacity or of the background's colour is a fill; no width of the page's
own thins such a line, the alarm's 2 and the selection's 2.5 among them; a level on such a shape
runs along its line, as on a shape with no area; a bound group's shapes each go by their own fill
as drawn.

**Not yet built**: the page still paints (a). The suite's development moved to another machine on
2026-11-19 before it was built; C-051 states the ruling as not built.

---

Q31 to Q45 were written on 2026-11-19, when the briefing of 13 to 16 November on Oficina and
controlador, its adversarial review and the robustness round were merged into this repository
(`docs/oficina-plan.md`, `docs/controlador-plan.md`, `docs/robustness.md`, `docs/briefing/`). The
operator: "the pending questions will be resolved later". Each groups the open points of one
subject; "the assistant" is the session that held the briefing.

## Q31 - Which of the review's majors and minors stand?

**Raised** 2026-11-15, by the adversarial review of the briefing (`docs/briefing/review.md`).

The review's 14 blockers were ruled the same day (its table "The operator's rulings"). Its 32
majors (entries 15 to 46) and its 9 minors (47 to 55) were not: each has its options and the
assistant's pick in the review, and the review's last section proposes an order for deciding them.
Some touch what is already built or decided here: entry 22 sets Oficina's starting of portal
sessions against Q19; entry 24 sets a colour computed by an expression against supervisão's rule
that every state has a mark and a word; entry 33 is P-014's ground.

**Taken meanwhile** (the most reversible): none of them is written into a plan as settled. The
plans carry the decisions as the operator made them, and each plan's "Open" names the entries that
touch it. An entry becomes a rule only by the operator's word.

## Q32 - What disqualifies an edit from a live takeover?

**Raised** 2026-11-15, by the operator, ruling on the review's entry 5.

"live fallback shall be enabled only when feasible. we need to build the specification for what
disqualifies the edits for auto takeover. takeover after restart remains possible for all cases".
The specification is not built. One candidate criterion, the assistant's and not ruled: the
watching version keeps its right to take over only while the comparison has shown no difference
between its outputs and the driven ones since the swap. Riders of the assistant's that nobody
ruled on: a failed edit leaves a fault record; the state text gains a term for the armed fallback
and its time left; the event is named "the edit failed", not a controller fault.

**Taken meanwhile**: no live takeover is built before the specification. The previous version can
always be put back for the next start.

## Q33 - Where is controlador's execution model written, and what does it say?

**Raised** 2026-11-15, by the review's entry 13; the operator chose the smaller first release
alone.

Not chosen: writing the execution model as the first section of controlador's plan, and a fixed
build order. So these stay open:
- where the model is written;
- whether tasks share data only through the process image (K8, the assistant's reading, which is
  what would make pre-emption safe without locks);
- whether each block gets its own slice of the input and output image (K10);
- how one block interrupts another inside the interpreter: a real-time thread per priority with an
  interpreter that can be re-entered, or switching at instruction boundaries on one thread;
- the fieldbus cycle's own real-time thread, which a breakpoint must not halt.

K11, the period check of a shadow test with several pre-emptive blocks, is put off by the first
release's limit of one cyclic task.

**Taken meanwhile**: `docs/controlador-plan.md` says what was decided and names the model under
"Before the code"; no code stands on an assumed model.

## Q34 - Where do the tests of core's fieldbus masters against the bench live?

**Raised** 2026-11-15, by the ruling on the review's entry 10 ("Core, rules amended").

The masters stay in core. Law L4 stands: core depends on no other member in any table, the test
table included, so core's own tests cannot start maquinar's bench devices. The ruling did not say
where those tests live. And "callable by other crates for testing only" stays a rule in prose that
no law checks.

**Taken meanwhile**: nothing is written. The candidates: the tests of the first consumer
(controlador), or a crate of repo tooling under `tools/`.

## Q35 - Which licence expression do the crates carry?

**Raised** 2026-11-15, by the ruling on the review's entry 14.

Oficina is GPLv3. Core, portal, `orcl-std` and every crate Oficina links get a permissive licence
that GPLv3 accepts, and a check of dependency licences runs in CI. Not ruled: whether that is MIT,
Apache 2.0, or the dual expression `MIT OR Apache-2.0` usual for Rust crates. The review also
noted that a licence file at the repository's root would cover the whole repository, where GPLv3
is meant for Oficina alone, and that its reading is no legal advice.

**Taken meanwhile**: no licence is stated in any manifest and no licence file is written. Nothing
is published before this is answered.

## Q36 - How is a download's integrity built?

**Raised** 2026-11-16, by the operator's requirement that a download carry "a hash or crc so no
corrupted program may run".

Ruled so far: the requirement; the recheck at power-on; two copies with a hash per block
(`docs/robustness.md`). The assistant's proposal for the rest, put to the operator on 2026-11-16
and not confirmed:
- a manifest lists every part of a download with its SHA-256 and length, and the SHA-256 of the
  manifest is the download's identity (the download history already keeps a hash);
- the controller stores to a temporary file, reads it back from the drive, verifies, and only then
  renames it into place and answers with the identity it computed;
- it verifies again before every load (a swap, a restart, a power-on); a failed check is the FAULT
  reason 506 `load`, never an automatic substitute;
- swap, start and rollback name the identity;
- a hash guards against corruption, not against tampering, which needs a signature over the same
  manifest and belongs to the security round (Q43).

**Taken meanwhile**: nothing is built; the proposal is the assistant's pick.

## Q37 - How is every request that alters the controller made robust?

**Raised** 2026-11-16, by the operator: "every bit of data that alters the controller shall be
robust, that includes commands such as variable writes, breakpoints setting, state change".

The assistant's proposal, put the same day and not confirmed: one rule for every altering request,
the download being its largest case, against the transmission faults listed in IEC 61784-3
(borrowed as a checklist; no safety rating is claimed):
- a SHA-256 of the content with every request (HTTP's Content-Digest, RFC 9530);
- a session and a strictly increasing number; a repeat is answered with the first outcome and not
  run again; a gap or an old number is refused;
- the controller's identity named in the request;
- the download's identity, and for a state change the state it expects to leave, named in the
  request;
- freshness, by echoing the controller's own last clock value;
- no answer is shown as "unknown", and the outcome is asked for by number;
- the change is applied whole, at one point of the scan;
- the answer is read back from the controller: the value in memory, the new state text, the full
  breakpoint list.

Its limits and loose ends:
- It guards against faults, not against a person; a keyed check (RFC 9421) belongs to the security
  round.
- Read literally, the operator's sentence also covers peers' writes, where secured OPC UA already
  signs and numbers its messages and Modbus/TCP has nothing beyond TCP's checksum. The assistant
  read the scope as the Oficina link.
- The tunnel's end on the laptop is reachable by every program there, a page in a browser
  included, so the server should refuse a `Host` it does not expect and the session value should
  be random (P-014).
- From the ruling that a write from outside is saved at once: the answer to a write should tell
  its sender how far it got, applied and then saved, so a panel shows a setpoint as kept only when
  it is. A write followed by a power cut inside the shortest gap is still lost.

**Taken meanwhile**: nothing is built; the proposal is the assistant's pick.

## Q38 - What must survive: which data is inside, and the border cases

**Raised** 2026-11-16, in the robustness round (`docs/robustness.md`, "What must survive").

The rule is the operator's: "every piece of data that is necessary for computing the logic
flawlessly in RUN mode", the commissioning scope outside it. The assistant's application of it,
given to the operator and not ruled:
- inside: the running program; the hardware configuration and IO mapping; retained variables; the
  settings a scan depends on; the interpreter, the operating system and what boots them; the
  integrity data itself;
- outside: forces, breakpoints, a shadow test, the commissioning switch (lost reads as off),
  earlier downloads, fault records, the state history, the store, the logs, and everything on the
  laptop;
- what is outside may be lost, but must never harm what is inside: a size limit, and storage apart
  from the program's.

Border cases, each with the assistant's reading:
- the source of the running program: RUN does not need it, but "the code is open" holds for the
  next engineer only if it can be read from the machine, so inside;
- what peers need, the Modbus and OPC UA settings and certificates: inside when the project uses
  them;
- a dry run: it keeps its ruling to survive a power cycle, without the protection meant for
  decades;
- the clock, for a program with time-of-day blocks.

**Taken meanwhile**: the readings above, as readings.

## Q39 - Time over decades: the details the rulings left

**Raised** 2026-11-16, in the robustness round (`docs/robustness.md`, "Time over decades").

Not ruled, each with the assistant's reading or proposal:
- **64-bit only** has to cover the operating system too (x86-64 and 64-bit ARM under a 64-bit
  Linux), since a 32-bit system on a 64-bit processor brings the 32-bit time value back; the build
  refuses a 32-bit target. Out are the 32-bit ARM boards (BeagleBone Black, i.MX 6, STM32MP1, the
  early Raspberry Pi models).
- **When NTP counts as silent:** only after a waiting time without an answer (at power-on, the
  start-up time), the request being not urgent until then. Otherwise every power-on with a dead
  battery sends an urgent request for a few seconds.
- **The urgency follows the present situation** (back to not urgent when NTP returns), while the
  request itself stays until cleared in Oficina or a restart, by the rule of 2026-11-14.
- **One maintenance reason or two** for the battery and NTP. A board with no clock chip counts as
  a dead battery, and the "ignore" setting is what such a board needs.
- **Which records carry the mark of an unconfirmed date:** fault records and the state history are
  controlador's; the store's samples are portal's, whose format would have to change (law L7).
  And whether a passing unconfirmed date shows as a condition term in the state text.
- **Starts that were missed:** a time-of-day block set to start once, whose moment passed while
  the date was unconfirmed, never runs; by the same reasoning a start that fell while the
  controller was powered off would not run either. A function that must run at an interval
  whatever the date belongs in a cyclic block.
- **The certificates' one end date:** every certificate the suite issues ends at the same second,
  so the year-ahead request comes up on every controller on 19 January 2037; a certificate issued
  late lives short, and after the end date the rule can issue none. "Its certificates" is read as
  every certificate in the controller's store, its own and its peers'. While its date is
  unconfirmed a controller can judge no certificate's dates and cannot time the year-ahead
  request. The end date that follows 2038, and what a secured link does meanwhile, are for the
  security round (Q43).
- **The cycle-time rule:** the maximum is the time error already decided (the handler on the first
  overrun, a fault at twice the period) and gives a free-running main block the limit the review's
  entry 13 missed; the warning is a maintenance request, not urgent, with a reason of its own
  beside `overruns`; optionally the controller records each task's longest cycle per day, so the
  drift shows before the warning.

**Taken meanwhile**: the readings above, as readings; no reason number is assigned.

## Q40 - The retained save: the details the rulings left

**Raised** 2026-11-16, in the robustness round (`docs/robustness.md`, "Retained variables, by what
the target can do").

Ruled: on a target that cannot save at power loss, the last save, flagged; a write from outside
saved at once, held to a shortest gap. Not ruled, each the assistant's reading or proposal:
- **What a save holds:** the retained variables as they stood at the end of one successful cycle,
  never a mix of two (the rule on temporary and permanent data, applied to each save).
- **How it is stored:** as everything RUN needs, two copies, each with its hash, written one after
  the other, which also leaves one whole copy when power is cut during a save. With both
  unreadable at start, the retained variables reset with the request `retain_reset`.
- **When the flag is raised:** only when cycles ran after the last save; a power cut while the
  controller sits in STOP raises none. `warm` then means "as of the last save" on such a target.
- **No maintenance request for a start from the last save**, a power cut being this target's
  normal case; the state text shows it as a condition beside `retain(reset=true)`, spelling open.
- **Who says what a target can do:** the target's hardware description, since hold-up power cannot
  be detected.
- **Nothing changed, nothing written:** a save whose content equals the last one is skipped.
- **The period** is the engineer's and is never refused; where the drive reports its wear, Oficina
  shows the years it has left at that period.
- **The shortest gap** is the engineer's setting, with a default. It bounds the wear, the drive
  seeing at most one save per gap, and Oficina shows the drive's shortest life at that gap. A
  write that arrives inside the gap is applied at once and saved when the gap ends.
- **A save on the program's request**, for example at the end of a batch.
- **The place of the save failing while the machine runs** does not stop RUN, the values being in
  memory. It raises a maintenance request, not urgent while one copy can still be written and
  urgent when none can, since the next power cut would then reset the retained variables.

**Taken meanwhile**: the readings above, as readings.

## Q41 - Wear and filling: what gives way at a limit, and the proposals not ruled

**Raised** 2026-11-16 and 2026-11-19, in the robustness round (`docs/robustness.md`, "Wear and
filling").

The question in hand when the round paused: when a growing record on a controller's drive reaches
its size limit, what gives way? It comes up for a recording on the drive, fault records, the state
history, earlier downloads, logs and the fault's copies of the window. The drive is finite, so
either old data goes or new data is not written.
- (a) The oldest is deleted (the assistant's pick), with two guards: a fault that repeats is
  counted on its record and not written again, so it cannot push the history out; and the running
  program and its fallback are never deleted.
- (b) Recording stops and the controller raises a maintenance request. Nothing is ever deleted, as
  Q16 decided for a project's disk store on a PC ("nothing is refused, deleted or merged").

The assistant's other proposals in this area, none ruled:
- **Storage in three kinds:** what RUN needs, written only at a download; the retained save,
  written often and needed at the next start; everything that grows, on a data area apart, so that
  wearing or filling it cannot touch the program.
- **Wear is watched:** where the drive reports it, the controller reads it and raises a
  maintenance request, not urgent, at a threshold the engineer sets; Oficina shows the years left
  at the present write rate.
- **Everything that grows has a size limit.**
- **The window in memory needs a ceiling in bytes** beside its span, since the memory an hour of
  values takes depends on how many variables change: the window is the shorter of the two, and the
  controller says so when the ceiling came first. `memory_limit_mb = 0` means the engine's own
  default (from memory and unverified: 80 % of the machine's memory), which a controller must not
  leave as it is.
- **What the window holds:** everything the embedded portal reads; the project may narrow it.
  Oficina reads it over portal's HTTP interface when it connects, so it sees the time before its
  connection.
- **The fault's copy of the window** can use `DuckStore::save_to`. Copies are limited in number,
  and no more than one is written per shortest gap. The program may ask for a copy too. A power
  cut, or a fault that takes the runtime itself down, leaves no window: only the fault record
  remains.
- **How the window is built** is for portal's plan to prove: DuckDB's manual says its file hands
  space back poorly after deletes; for its in-memory form nothing was checked. Dropping whole
  slices of time is the form the assistant expects to hold.
- **A full or failed data area never stops RUN:** recording stops and a maintenance request is
  raised.
- **The operating system's own writing** goes to the Linux study.
- **Portals on PCs:** supervisão's and Oficina's portals have the same unbounded store. Whether
  they get a default limit belongs to "which other components count as critical" (Q42).

From memory and unverified: an S7-1500 reports its memory card's used life in percent and raises a
maintenance alarm at a configurable threshold; DuckDB confirms each commit to the drive.

**Taken meanwhile**: nothing is built. On a controller the store is the memory window, as ruled;
the rest waits.

## Q42 - The robustness round: the areas not opened, and the Linux study

**Raised** 2026-11-16, when the round was called.

Not opened:
- running unattended beyond the cycle-time rule: the hardware watchdog, memory errors, slow leaks;
- an old controller and a new Oficina: versions, migration, a rescue path;
- replacing the hardware: backup, restore, what a stranger needs to rebuild;
- which other components count as critical;
- how each rule is proven by injecting faults on the bench.

Owed before the operating-system rules: the study of Linux for real time and of its recovery
methods (`docs/robustness.md`, "The Linux study"). The assistant's caution, not ruled: on hardware
we do not choose, each cycle's data can be made consistent every cycle but reaches the drive only
at set moments.

**Taken meanwhile**: nothing. The round resumes with the operator.

## Q43 - The security round: the positions going in, and its list

**Raised** 2026-11-16, by the operator: "dedicated round to decide cyber security level
suite-wise, future proof is the best way to design".

The operator's positions going in: default user and password pairs written in a public manual do
not cover it, "most users will leave defaults"; "a password is always better than no password, and
on the network controlador appears as a regular linux machine"; "once secured behind a login,
controlador is just another linux machine. it is open source, and the responsability to keep it
safe belongs to the integrator imo". The briefing had put security out of controlador's first
scope: "none for now, I need to study".

The assistant's inputs, not ruled: no default credential at all, the first commissioning step
setting it (or SSH keys only); the data format is public in an open project, so it is no barrier;
European product rules put duties on whoever supplies a product commercially, so the operator's
view holds while the suite itself is not sold (a reading, not legal advice).

The round's list so far, beside P-014's two items (Q29):
- whether an expired certificate of a known peer cuts a secured link or only warns; the
  certificates' end date after 2038; what a secured link does while the controller's date is
  unconfirmed (Q39);
- a signature over a download's manifest, and a keyed check on altering requests (Q36, Q37);
- the review's security entries, none ruled: 32 (opening a project must run nothing), 33 (loopback
  and a tunnel are not a boundary), 34 (text from the plant shown as plain text), 35 (what the
  controller keeps and publishes), 36 (a bench device passing for a real one), 37 (no record of
  who did what), 38 (the Cyber Resilience Act) and 39 (the fieldbus organisations' terms).

**Taken meanwhile**: nothing is built before the round (P-014).

## Q44 - Conversion to native PLC code: the type table's open points

**Raised** 2026-11-15, by the operator stating the goal: controlador's ORCL program and its
hardware configuration convert to native PLC code, Siemens S7-1500 and S7-1200 first.

The operator began a table mapping ORCL data types to S7 types. In it S7's BYTE, WORD, DWORD and
LWORD map onto the unsigned integers, with no bit-string types of ORCL's own (the assistant's
reading of the table). Checked on Siemens' TIA V21 pages that day: DATE runs to D#2169-06-06
(2168-12-31 is the older S7-300/400 figure); DTL is 12 bytes, up to
DTL#2262-04-11-23:47:16.854775807, weekday 1 being Sunday; the classic S7-1200 lacks LWORD, LINT,
ULINT, S5TIME, LTIME, LTOD, DT and LDT, while the S7-1200 G2 is grouped with the 1500; references
and named value types are S7-1500 only.

Points the assistant raised, not decided:
- below `f64` the table's left column is not Rust: `char` is 32 bits in Rust, and `string`,
  `time`, `date` and the rest must be `orcl-std` types with a fixed representation (`orcl-std`
  uses `core::time::Duration` today, which matches neither TIME nor LTIME);
- the table needs a column per CPU family;
- readable SCL needs names and comments, which ORCL's own notes say only a front end that reads
  the source keeps. The interpreter stands on the compiler's intermediate form (ruled 2026-11-15),
  so the SCL translators and the ladder view still need a second front end that reads the source
  (the assistant's reading, not ruled).

**Taken meanwhile**: nothing is written; the table is the operator's draft and is not in this
repository.

## Q45 - The briefing's readings and small open points

**Raised** 2026-11-15, in the deliverables list (`docs/briefing/deliverables.md`, section K) and
in the rulings that followed.

Readings of the assistant's, none ruled:
- the arguments inside `force` are always in the order in, out, var (K1);
- every string value in the state text is quoted, and booleans stay bare, as `urgent=true` shows
  (K2, corrected: "every non-numeric value" was wrong);
- a force suspended at a halt applies again when the program continues (K3);
- with the IO mapping allowed to differ, the shadow comparison is per physical channel (K4);
- start-up phases have numbers but show no `id` in the text (K5);
- several maintenance requests can be active, each as its own term (K6);
- a crash is told from a power cycle by whether the machine itself rebooted (K7);
- the conformance suite is only as good as the list of core operations ORCL allows;
- Windows gains a third use beside development and simulation, the IO check from a laptop;
- for an unaccepted shadow test at a power cycle, "always as before" means the version that was
  driving drives again and the test stays unaccepted, the fallback flag resetting as decided;
- the "knowledge protection" the operator mentioned in passing is a picture of the trade, not a
  requirement: no such lock exists in ORCL's decisions.

Open, never asked:
- whether "identify, name and address" runs from the laptop or through a target (G2);
- where the readers of GSDML, ESI and EDS files live (C5);
- whether "good" in supervisão is a new state class or the colour of the normal one (K12);
- whether the hardware configuration checks a Modbus module's own timeout in production too;
- whether Npcap's licence allows what raw frames on Windows need.

**Taken meanwhile**: the readings stand as readings, marked so wherever a plan leans on one.
