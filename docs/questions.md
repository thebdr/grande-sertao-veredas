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

**Taken (architect):** `tools/leis`, a workspace member that is repo tooling, not a component (no
`veredas-` prefix, like `acerto` and `abaco-oracle` in Veredas 1; exempt from the component laws).
The laws are pure functions over parsed manifests, so a refute round can attack them on synthetic
suites; `cargo test -p leis` is the verdict. Core stays a library that knows nothing of the suite's
layout.

**Reversible?** Yes: a `git mv` and the clause links.

---

## Q2 — The review deck in the repo?

**Raised** 2026-11-05. `docs/` held the deck as `Veredas 2.html` (625 KB) and `Veredas 2.pptx`.

**Taken (architect):** both committed, renamed to ASCII (`veredas-2-review.html`, `.pptx`), as the
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

**Revisited** 2026-11-14 and 2026-11-15 (the architect, the briefing on controlador): the functions
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

**Taken (reversible, pending the architect):** not copied. `docs/vendor/` is gitignored here as
there (its README says what belongs in it); the code keeps citing section numbers; the constants
are in code and pinned by the telegram tests, so a machine without the file builds and tests.
To read the spec, use the copy beside Veredas 1 or drop one into `docs/vendor/` here.

**Reversible?** Yes: a file and an ignore rule.

**Since Q8 (2026-11-05):** core names GENIBELT nowhere and cites none of its sections; the
specification is for writing the rules a portal channel is configured with.

**Decided** 2026-11-09 (the architect): not in the repo. `docs/vendor/` stays gitignored with its
README; whoever writes the device's rules file reads a local copy or the one beside Veredas 1.

---

## Q7 - Do the TRISTATE bin-state bits stop a chute?

**Raised** 2026-11-05 by the refute round of C-011: the GENIBELT specification (§4.2.1.4) names
bits 28-31 of a bin state (MAPPING, EMERGENCY_TRISTATE, ALARM_TRISTATE, WARNING_TRISTATE) without
saying what they mean for parcel acceptance. Veredas 1 named none of them.

**Taken (reversible, pending the architect):** they are named (a state word reads as words), and
`accepts_parcels` ignores them, as Veredas 1 did, so no chute that sorted before stops sorting.
Marked `// DECISION PENDING` in `veredas_core::telegram::genibelt::bin_state`.

**Reversible?** Yes: one constant set in `accepts_parcels`, and the test that pins it.

**Moot (2026-11-05, Q8):** the bin-state words left core with the GENIBELT catalog; whether these
bits stop a chute is a question for the rules a portal channel is configured with.

**Closed** 2026-11-09: moot since Q8 (the architect's decision of 2026-11-05); nothing is left to
decide in the suite, the question belongs to the device's rules file.

---

## Q8 - Does GENIBELT belong in core?

**Raised** 2026-11-05 by the architect, while the telegram layer was being ported into core.

**Taken (architect):** no. "genibelt is a machine my company manufactures and the interface via
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

**Taken (reversible, pending the architect):** the framing and the channel stay in core as they
are, with the vendor's name and section citations removed and the shape described in their
docs; no other framing exists yet to generalise against. When the first telegram channel's
configuration schema is written (portal, P-002), the framing's constants become that schema's
fields (start/end bytes, envelope layout, length and count fields, byte order, sequence and
keep-alive rules as switches), and the current shape becomes one configuration of it. Open
point for that schema: whether a device's keep-alives consume sequence numbers (the channel
assumes they do: a keep-alive's number counts as the last one seen).

**Reversible?** Yes: a configuration schema over the same code, or removing the framing from
core altogether.

**Answered (architect, 2026-11-05):** "Design configurable framing now". Core keeps no frame
layout of its own. `telegram::framing` is an engine that encodes and decodes any layout a
channel file describes (optional start and end delimiters; envelope, segment-header and tail
fields by size, byte order and role: sequence, length, count, opaque); `telegram::schema` reads
and validates that file (`docs/channel-schema.md`); the channel's sequence, keep-alive and link
rules are sections of the same file. The layout this question was about is the schema's worked
example and the engine's first test fixture, named for its shape (a segmented envelope), not for
its maker: a 12-byte envelope with a sequence, a length and a count is a shape, not a secret.
The device's own file lives with its other rules, outside the suite's code.

**Revisited** 2026-11-15 (the architect, ruling on the review's entry 10 of the briefing: "Core,
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

**Taken (reversible, pending the architect):**
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

**Answered (architect, 2026-11-05):** "TOML, suite-wide". The channel file stays as designed, and
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

**Taken (reversible, pending the architect):** the channel file refuses a `connect` inside
`first..=last`. Every layout seen so far (the worked example: `connect = 0`, cycle 1 to 255)
keeps it outside. A device that needs it inside would need the receiver to exempt only the
first keep-alive after connect instead, a version 2 of the rule.

**Reversible?** Yes: loosening a refusal breaks no file that loads today.

**Decided** 2026-11-09 (the architect): keep refusing. A `connect` inside `first..=last` is refused
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
comissionador embed), is the architect's call.

**Taken (reversible, pending the architect):** a library first, with one executable that runs a
bench file (`veredas-maquinar <bench.toml>`) until stopped; the UI is a later phase of
`docs/maquinar-plan.md`, named there with no code. Every operator action the Veredas 1 UI had
(start and stop a part, send a message now, set a destination's state, declare every destination
full, issue a command) is a method on the bench's handles, so a UI, when it comes, drives the
same library the tests do.

**Reversible?** Yes: a UI is a crate or a module on top of the library; nothing below it changes.

**Decided** 2026-11-07 (the architect): library only. The bench is the library and the
TOML-driven executable; no desktop UI is planned, and `docs/maquinar-plan.md` no longer names
one as a later phase.

**Revisited** 2026-11-13 (the architect): the decision stands for the bench itself, which stays a
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

**Taken (reversible, pending the architect):** no FFI. The clause states the quantum, the tests
pin the rate on both platforms and the per-tick figure per platform, and the bench's two tick
loops carry a `DECISION PENDING` note. Raising the resolution is one call at the start of the
executable, behind `cfg(windows)`, when the architect wants 20 ms ticks on the bench.

**Reversible?** Yes: one call, one place, nothing else changes.

**Decided** 2026-11-09 (the architect): yes. The bench executable raises the Windows timer
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

**Decided (2026-11-09, the architect): the executable stands.** Portal is a library and one executable,
`veredas-portal <project.toml>`, which serves a project from the command line until stopped; no
desktop UI is built. Every operator action the Veredas 1 shell had on a session (start, stop, add
or remove a source, arm the writes, snapshot, export) is a method on the session's handle, so a
shell, if one ever comes (P-008's web interface, or a thin window over the server), drives the same library the tests do; none is built in portal. The grep engine and the skew monitor
wait with it (`docs/portal-plan.md`, "Scope").

**Revisited** 2026-11-13 (the architect): portal still builds no desktop UI. The shell this question
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

**Taken (reversible, pending the architect):** the project file names the user and the name of an
environment variable that holds the password (`password_env`); portal reads the variable when the
session starts and refuses to start a source whose variable is unset, naming the variable, never
the value. No vault, no secrets file.

**Reversible?** Yes: a second source of secrets (a vault, a file outside the project) is one more
way to fill the same field.

**Decided** 2026-11-09 (the architect): an encrypted secrets file in the project folder, committed
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

**Taken (reversible, pending the architect):** refuse by name. The store's open says a database
already stands there and that a session store is fresh; nothing is deleted, nothing merged.

**Reversible?** Yes: a per-session file name or an opening mode is one line in the project's
store section, and the store's check stays as the guard for the fixed-name case.

**Decided** 2026-11-09 (the architect): a file per session. The store's file is named from the
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

**Taken (reversible, pending the architect):** the sniffer stays the first reader, its outcomes
stated and pinned; the padded path is forced on a collapse alone, as C-041 says.

**Reversible?** Yes: forcing the padded path for every linked CSV, or for any file the sniffer
reads ragged, is one branch in the store's CSV scan and one sentence in C-041.

**Decided** 2026-11-09 (the architect): the padded path for any ragged file. The sniffer stays the
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

**Decided** 2026-11-09 (the architect): **not at all.** No export reader is carried, in the suite or
beside it; binding files are written by hand or by anything, and supervisão reads nothing else.

## Q19 - Where do supervisão's sessions come from?

**Raised** 2026-11-09, planning supervisão (P-008).

Either each session is a `veredas-portal` executable already running, which supervisão reaches over
HTTP and owns nothing of, or supervisão runs the portal sessions in its own process from project
files.

**Decided** 2026-11-09 (the architect): **supervisão starts them.** It loads each plant's project
file and starts its session through portal's library, one executable serving every session; the
session's portal router is served under supervisão's address (`/s/<name>/portal/`). Supervisão
still owns no plant connection: every value and every write is portal's.

**Reversible?** Yes: a session reached over HTTP is a second way to fill the same slot.

## Q20 - Who may open supervisão?

**Raised** 2026-11-09, the design's open point "session and permission model for several sessions
on one host".

**Decided** 2026-11-09 (the architect): **local, no accounts.** The host binds the loopback by
default; whoever reaches it sees every session alike; a write rests on portal's switch (armed by
the project, never over the wire) and on the actions the binding file declares.

**Reversible?** Yes: accounts or a read-only remote mode sit in front of the same routes.

**Revisited** 2026-11-13 (the architect): it stays, and it is extended to remote targets: Oficina
reaches a target through SSH and has no accounts of its own. What a login adds is left to the
security round (Q43, P-014).

## Q21 - The 3D view now or later?

**Raised** 2026-11-09, planning supervisão (P-008).

**Decided** 2026-11-09 (the architect): **later**, after the 2D layout, the dashboards, the gated
writes and the several sessions are verified. The binding file's object IDs are the ones a glTF
model's nodes will carry; no 3D key exists in the binding format until the view does.

## Q22 - Which accent does supervisão draw?

**Raised** 2026-11-09, by the architect, looking at the first page.

The design references (`docs/design/README.md`, `docs/design/supervisao/`) gave the web side a
magenta accent, after Veredas 1's dashboard, and the desktop side green.

**Decided** 2026-11-09 (the architect): "veredas accent color shall be green (see veredas-desktop
1)". Supervisão draws Veredas 1's desktop green (`#30D158`, soft `#1C3A27`, outline `#2F5A3D`,
hover and data series `#5FE07F`, on-accent text `#08130B`); the mockups' magenta reads as the
accent wherever it appears. The accent stays what it was: selection, focus, primary actions and
neutral data series, never a plant state.

**Reversed** 2026-11-15 (the architect): "Use Lavanda #B9A9E8 as the suite accent colour". The green
accent "has been an inconvenience": green is to mean a good state, so the accent became a colour
that is memorable and carries no ready meaning (not blue, green, red, yellow or white). Lavanda
`#B9A9E8`, soft `#2B2838`, outline `#494360`, on-accent text `#12101C`; the hover and data-series
tint `#CFC3F2` is the assistant's proposal. Green `#30D158` is freed to mean a good state. The
syntax palette's number colour (`#D2A8FF`) sits too close to the new accent and should move.
Whether "good" is a new state class or the colour of the normal one is open (Q45).

**Its standing** 2026-11-19 (the architect, told that the accent was still applied nowhere):
"lavanda is important". Asked where it should be applied, on the Windows desk before development
moved or first on the Linux machine: "Here, now".

**Applied** 2026-11-19 in supervisão, as a change of its own (C-051). What moved:
- the stylesheet's accent tokens, and the distribution chart's area, the one colour the stylesheet
  spelled beside its tokens; the built page;
- the layout test's pins: the accent `#b9a9e8` and the selection's stroke `rgb(185, 169, 232)`;
- the layout test's band: no state is painted in the accent's four colours nor in any colour within
  35° of its hue, 220° to 290° with a saturation above 0.2 (a blue, a violet or a purple), where
  the band was the green's, 100° to 170°. A state in green is no longer refused; none takes it
  until Q45 is ruled;
- `docs/design/README.md`, which now gives the suite one accent.

Three planted faults, each alone, the page rebuilt and restored after each: a near state's tag
painted in the accent's tint, and the near colour made an indigo (`#5e5ce6`), each failed the
layout test at "a state drawn in the accent"; the near colour made the old green (`#30d158`)
passed it.

Still the assistant's choices, none of them a ruling: the tint `#CFC3F2` (data series, the live
mark, the legend's selection); the band's width, kept as the green's was; the unused token `--pill`
(a dark green), which left the stylesheet with the family it belonged to. The syntax palette's
number colour has not moved: no stylesheet uses it yet.

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

**Decided** 2026-11-15 (the architect, asked "Should the page be barred from loading anything a
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
this ruling; the architect barred it on 2026-11-15 (a site open in the same browser could lay the
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

**Decided** 2026-11-15 (the architect, asked "Keep refusing, or replay such rules?"): "Keep
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

**Decided** 2026-11-15 (the architect, asked "Which tool will draw the layouts?"): "Inkscape": the
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
reader, never by teaching the page a new one. No Inkscape is installed here and the architect
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

**Decided** 2026-11-15 (the architect, asked "Should supervisão's name check guard against accidents
only, not deliberate hiding?"): "Accidents only" (the other choice read "Deliberate hiding too").
The check is for a change made in good faith with ordinary tools: a name written as an editor, a
formatter or the build toolchain writes text; a dependency, a file or a source brought in the ways
cargo, the compiler and git work in ordinary use. It lists the forms it reads. Deliberate
concealment is outside it: a name encoded, split, nested or computed so that it is not read; a
tool's flags or configuration used to keep a file or a source away from the check. The orchestrator
reads the answer as the threat model of the whole clause, both its halves (the name scan, and what
is compiled in or brought into the graph), the examples the architect read coming from both.
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

**Decided** 2026-11-15, by the architect: "Drop the script, load it", (c), against the recommendation.
The reader drops Inkscape's script beside a mesh gradient or a hatch (`svg::POLYFILLS`:
`mesh_polyfill`, `hatch_polyfill`) where and as Inkscape writes it, a `script` of the root's own
writing nothing but its id and a type, with all it holds, as it drops Inkscape's other elements;
the drawing loads, and the shape filled with the mesh or the hatch shows no fill on the page, with
no message. Any other script stays refused: the architect was asked about Inkscape's alone. The
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

**Decided** 2026-11-16, by the architect: "Page's own words only", (b), as the mock does it: "The
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

**Decided** 2026-11-16, by the architect: "Leave it for the security round", (c); and, asked when
portal's own server (run without supervisão) is to be closed, "With the security round". Nothing is
built now: the host's gap stays a limit C-049 states, portal on its own is an open item of the
round, and portal's clauses stay verified. A firewall does not close it: the request comes from a
browser that is allowed to reach the address. The round's open items are P-014, which names both
ways the architect was asked about: a name re-pointed at the address (above); and a write a page of
any site posts as `text/plain`, which a browser sends to another site without asking it first and
which portal's write and send routes parse as JSON whatever its `Content-Type` (`read_body`,
`crates/veredas-portal/src/serve.rs`), while writes are armed, the page unable to read the answer.
The second is read from the code, not yet shown by a test. What the architect was told of browsers
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

**Decided** 2026-11-19, by the architect: "Its line, as drawn", (b), and the line keeps the width it
was drawn with. The line takes the state's colour and keeps the width it was drawn with (never
thinner than the page's own outline); nothing is painted inside, and a level runs along the line.
This is how the design mock shows a stopped induction (`docs/design/supervisao/Main.dc.html`, the
inductions drawn as lines 10 wide, the stopped one the same line in amber). An outline that should
be painted as a body needs a fill in Inkscape. Not chosen: the line redrawn at the page's own thin
width (1.4, which every bound shape's outline takes today, so a conveyor drawn 10 wide would show
its state as a hairline); the enclosed area painted, as now. Read with it (the foundation's
reading, not the architect's words): drawn with no fill is the fill computing to `none` as the
drawing stands before the page paints it, however it comes (an attribute, `style`, a group
above), and a fill of zero opacity or of the background's colour is a fill; no width of the page's
own thins such a line, the alarm's 2 and the selection's 2.5 among them; a level on such a shape
runs along its line, as on a shape with no area; a bound group's shapes each go by their own fill
as drawn.

**Built** 2026-11-22 (C-051, `docs/binding-schema.md`, the `show` key). The page reads each bound
shape's fill as the drawing stands before it paints anything; a shape whose fill is `none`, and a
shape with no area (the design mock's inductions are `<line>`s 10 wide: a line has no fill to
speak of), is drawn as a line: unfilled, its line in the state's colour at the width it was drawn
with, never under 1.4 (2 in alarm, 2.5 selected). A level on such a shape that closes an area runs
along its line: a copy of its shapes over it, clipped to the level, the shape's own line its track.
Readings taken in building it, none of them the architect's words: a text stays a body whatever its
fill (the ruling names shapes); the selection paints the line whole in the accent, the state then
said by its mark and word and the side panel, and a selected level's line is 2 wider than its level
is in alarm, its widest, in every state alike, so the accent shows around the level running inside it; a stale line is dashed at its own width; in normal a line
takes the neutral grey a body takes (`#4A4E5C`), a marker's line the marker's own light grey; a
click inside a shape drawn as a line picks what lies there, the inside being nothing of it; a level
on a group holding such a shape runs along the group's shapes, a group without one still fills its
box.

---

Q31 to Q45 were written on 2026-11-19, when the briefing of 13 to 16 November on Oficina and
controlador, its adversarial review and the robustness round were merged into this repository
(`docs/oficina-plan.md`, `docs/controlador-plan.md`, `docs/robustness.md`, `docs/briefing/`). The
architect: "the pending questions will be resolved later". Each groups the open points of one
subject; "the assistant" is the session that held the briefing.

## Q31 - Which of the review's majors and minors stand?

**Raised** 2026-11-15, by the adversarial review of the briefing (`docs/briefing/review.md`).

The review's 14 blockers were ruled the same day (its table "The architect's rulings"). Its 32
majors (entries 15 to 46) and its 9 minors (47 to 55) were not: each has its options and the
assistant's pick in the review, and the review's last section proposes an order for deciding them.
Some touch what is already built or decided here: entry 22 sets Oficina's starting of portal
sessions against Q19; entry 24 sets a colour computed by an expression against supervisão's rule
that every state has a mark and a word; entry 33 is P-014's ground.

**Taken meanwhile** (the most reversible): none of them is written into a plan as settled. The
plans carry the decisions as the architect made them, and each plan's "Open" names the entries that
touch it. An entry becomes a rule only by the architect's word.

**Ruled in part** 2026-11-25, by the architect, entry by entry (the entries are
`docs/briefing/review.md`'s; 17 to 21 were ruled with Q100 the same day, the fork of entry 27 with
Q102, the file names of entry 25 with Q224):

- Entry 15. Asked "A swap to a version with a different IO mapping leaves the old output channel
  without a writer, and rollback has no stated path. Which of the review's three options hold? (The
  review's pick: all three.)", all three chosen: "(a) Orphaned outputs go safe", "(b) A moved signal
  is one line", "(c) Rollback is a download".

- Entry 16. Asked "A single cycle runs one scan and halts, so every output written in that scan is
  energised for milliseconds (contactors, valves, brakes); and switching a dry run off in RUN makes
  outputs jump to mid-sequence values. Which options hold? (The review's pick: both.)", both chosen:
  "(a) Single cycle guarded" (on a real target only in a dry run, or with outputs held between
  cycles after a confirmation), "(b) Dry run ends through STOP".

- Entry 22. Asked "Q19 has supervisão's host start the portal sessions; if Oficina also starts
  portal for the same project, two sessions poll one PLC and write two stores, and the database lets
  only one process write a file. Who owns the sessions, and how does Oficina run SQL over a store?",
  the architect's own answer, none of the options: "if oficina and supervisao run on the same host
  they share one portal instance". Who starts that instance, and how SQL reaches a store, are put
  next.

- Entry 23. Asked "Vendor description files (GSDML, ESI, EDS) are copied into the project and read
  directly (your choice). They are not TOML, they are the vendors' copyright, and they are XML from
  outside. What are they in a project?", "(a) Inputs, read never written (Recommended)": the TOML
  rule's one named exception, parsed with document type definitions and entities switched off, the
  test fixtures descriptions written here; their readers live in core (Q45's C5, the same day).

**Ruled in part** 2026-11-25, by the architect, four more answers:

- Entry 22, who starts. Asked "Following your answer (\"if oficina and supervisao run on the same
  host they share one portal instance\"): who starts that shared portal instance for a project?",
  "Whichever comes first": the first of Oficina and supervisão to need the project starts the
  instance, the other finds it running and joins, and it stops when the last one leaves. Q19
  (supervisão starts the sessions) is amended by it for a host that runs both.

- Entry 22, SQL. Asked "SQL over a store: the database lets one process write a file. How does
  Oficina run SQL over the store of a running session?", "(c) A read-only route in portal
  (Recommended)".

- Entry 24. Asked "You chose colour, size, position and visibility as expressions, but supervisão's
  rule is that every state has a mark and a word, and an expression that turns an object red
  bypasses both. What may a colour expression do?", "(b) A class or a declared scale (Recommended)":
  a colour expression resolves to a state class, which brings its mark and word, or to a declared
  scale with a legend; the syntax palette's green for strings moves with the number colour.

- Entry 26. Asked "PROFINET first (your choice): only discovery exists, the specification sits
  behind the membership, and no open controller exists to compare with. What holds?", "(a)+(b)
  Modbus proves, real device (Recommended)": Modbus TCP is the bus on which the runtime is first
  proven while the membership runs, PROFINET stays the first fieldbus written, and from its first
  clause the PROFINET work has a real device in the loop and captures of a real controller's traffic
  as fixtures.

**Ruled in part** 2026-11-25, by the architect, four more entries:

- Entry 27. Asked "The bench's EtherCAT device must emulate in software what a real device does in a
  chip (functional, never exact in time), and the young EtherNet/IP crates become ours to keep.
  Which options hold? (The review's pick: (a) and (b).)", chosen: "(a) A partial emulator, said so"
  and "(b) Real terminals in the loop"; not chosen: "(c) EtherNet/IP later".

- Entry 28. Asked "Monaco is an editor only; it has no client for a language server and none for a
  debugger, and Ábaco's Python debugger needs the same interface. Which options hold? (The review's
  pick: (a), decided now so ábaco does not write a second one.)", chosen: "(a) One pair of clients,
  shared", written once on Oficina's side and shared by ORCL and Python; not chosen: a first
  debugger without inline values, and a native editor as fallback.

- Entry 29. Asked "Every clause's test drives the real interface (your choice), but Spix cannot see
  inside the embedded web page (Monaco, supervisão's tabs), and floating panes cannot be exercised
  headless. Which options hold? (The review's pick: (a) and (c).)", chosen: "(a) A gate in the
  spike" and "(b) Two tiers" (logic proven on the Rust model, each surface proven by driving it);
  not chosen: "(c) Multi-monitor docking outside", so docking across monitors is not declared
  outside the suite.

- Entry 30. Asked "Oficina's first release is about fifteen surfaces, and forms are built \"from a
  schema\" that nothing can generate today. Which options hold? (The review's pick: both.)", both
  chosen: "(a) Forms from core's types" and "(b) Surfaces in an order" (shell, tree, form and text,
  target over SSH, session control and health, log trailing and supervisão's tabs first, the rest in
  point releases).

**Ruled in part** 2026-11-25, by the architect, four more entries, three of them the security
round's (Q43):

- Entry 31. Asked "The 3D scene is authored in Qt Quick 3D inside Oficina while bodies, collisions
  and sensors live in the bench's `fisica`, so two scene models must be kept in step, and Qt Quick
  3D brings little for editing. Which options hold? (The review's pick: both.)", both chosen: "(a)
  View-only 3D first", "(b) Ready-made behaviours only".

- Entry 32. Asked "The laptop holds the SSH keys to every plant. Is this the rule? Opening a project
  runs nothing: ORCL projects have no build scripts and no procedural macros; the folder's cargo
  configuration is not honoured; Python runs after a trust prompt per folder; pages not shipped
  inside Oficina get no native bridge.", "Yes, opening runs nothing (Recommended)".

- Entry 33. Asked "Every component listens on loopback with no accounts, so any process on the
  target or the laptop, and a web page by DNS rebinding, reaches the write paths. On Q29 you left
  this for the security round. What closes it?", "(c) Both; token alone for the web (Recommended)":
  a random token per session, handed over SSH and required on every request, with Host and Origin
  checks, for portal and supervisão; and Unix sockets (named pipes on Windows) with the system's
  permissions for controlador's engineering channel. Q29 and P-014's two items are answered by it.

- Entry 34. Asked "A device named `<font color=green>RUN</font>` can paint a false state, a CSV cell
  beginning with `=` becomes a formula, trailed logs pass terminal escapes. Is this the rule? Text
  from outside is always shown as plain text (held by a test over the QML), the CSV export
  neutralises formula cells, and the interface uses the structured state, never parsed state text.",
  "Yes, the whole rule (Recommended)".

**Ruled in part** 2026-11-25, by the architect, four more entries of the security round's list:

- Entry 35. Asked "A fault record holds a snapshot of all data and faults are published to peers;
  and the source of the last ten downloads sits on every machine delivered (your choice). Which
  options hold? (The review's pick: both.)", both chosen: "(a) Peers get cause, number, time" (the
  snapshot read through Oficina only), "(b) Source on target, a setting", on by default.

- Entry 36. Asked "A bench device with a real device's station name, started from a laptop plugged
  into the plant network, can be taken by a real controller, its \"guard closed\" then coming from a
  behaviour model. Which options hold? (The review's pick: all three.)", chosen: "(a) Interfaces
  declared isolated" and "(c) Bench identity refused"; not chosen: "(b) A scan refuses a live
  network".

- Entry 37. Asked "No record says who did what (no accounts, by your choice); forces and breakpoints
  are recorded only at a fault. The Machinery Regulation (EU) 2023/1230 asks a machine builder for
  evidence of interventions from January 2027 (read from secondary sources). Which options hold?
  (The review's pick: both.)", both chosen: "(a) An intervention log" (append-only, on the target,
  each line with the time and who: the SSH user, the peer's address, a name typed once per Oficina
  installation; retention set per target), "(b) Not a safety component".

- Entry 38. Asked "The Cyber Resilience Act (EU) 2024/2847 applies in general from 11 December 2027.
  An unpaid open-source release keeps its author outside \"manufacturer\"; paid support or delivery
  as a product changes that. Which options hold? (The review's pick: all three. No legal advice.)",
  all three chosen: "(a) The plan states the intent", "(b) Security file, component list", "(c) Gap
  list against Annex I". Which intent the plan states is put next.

**Ruled in part** 2026-11-25, by the architect, four more answers:

- Entry 38, the intent. Asked "Following your answer: which intent does the plan state for the suite
  under the Cyber Resilience Act? (No legal advice; the regulation's terms are its own.)",
  "Undecided, say so": the plan says the intent is not decided yet, and what each choice would
  bring.

- Entry 39. Asked "The fieldbus organisations have terms (ODVA's Terms of Usage for EtherNet/IP; the
  EtherCAT group's master agreement; PROFINET certification and its specification licence). Which
  options hold? (The review's pick: all three. Medium confidence, secondary sources.)", chosen: "(c)
  \"Compatible with\" until certified"; not chosen: signing the agreements before each network
  ships, and asking the organisations in writing.

- Entry 40. Asked "In release 1 the only way in is ORCL text (your choice: ladder later, as a
  read-only view). `orcl-std` has the three timers, both edge detectors and the up counter; it lacks
  the down counter and the set and reset latches. Which options hold? (The review's pick: (a) and
  (c) now, (b) when you know.)", chosen: "(a) The standard library, completed" and "(b) Name
  ladder's release"; not chosen: "(c) Release 1 is for text writers". Which release is put next.

- Entry 41. Asked "The first temperature loop has no PID (technology objects are out, by your
  choice), and nothing reads tags or types from another tool. Which options hold? (The review's
  pick: (a) and (b).)", the architect's own words with one option: "technology objects later", and
  "(b) Say what release 1 serves" (discrete IO, no closed loops, no axes). Not chosen: a PID
  function block in the standard library now, and an import from PLCopen XML.

**Ruled in part** 2026-11-26, by the architect, four more answers:

- Entry 40, the release. Asked "Following your answer of yesterday (\"Name ladder's release\"): in
  which release does ladder become editable?", "Release 2": ladder is a read-only view in release 1
  and editable in the release after it.

- Entry 42. Asked "A technician without the project cannot open what is running, though the source
  archive is kept on the controller for exactly that, and connecting needs an SSH key set up first.
  Which options hold? (The review's pick: (a) and (b); (c) can wait.)", chosen: "(b) Pair in one
  step" (Oficina creates the key and installs it); not chosen: "(a) Open the project from the
  target" and "(c) One connection view".

- Entry 43. Asked "Colour and contrast: good, warning and fault collapse into one khaki under the
  most common colour blindness (answered by the mark-and-word rule); the outline `#494360` on the
  soft tone is 1.54 to 1 where a control's only edge needs 3 to 1; and no light theme exists. Which
  hold?", chosen: "Oficina carries mark-and-word" and "A light theme joins the list"; not chosen:
  "Outline becomes #7A7399", so the outline's colour stays as it is.

- Entry 44. Asked "No way to reuse blocks across projects, and nothing says how a structure appears
  in watch tables. Which options hold? (The review's pick: both.)", both chosen: "(a) A library is a
  pinned crate" (named by path or by Git revision, pinned in the project file, no build scripts),
  "(b) Structures expand by member" (in watch tables, forcing and the IO mapping).

**Ruled in part** 2026-11-26, by the architect, four more entries:

- Entry 45. Asked "An acceptance test needs the IO list, the alarm list and the cross-reference on
  paper, and nothing in the list exports anything but comissionador's test report. Which options
  hold? (The review's pick: (a) and (b).)", all three chosen: "(a) Export the three lists" (to CSV
  and PDF, among controlador's surfaces), "(b) An example per network", "(c) Context help".

- Entry 46. Asked "The list does not say what a Rust engineer gets. Does the plan state it: an ORCL
  project is a cargo crate whose logic `cargo test` runs, the checker works outside Oficina and in
  CI, and the allowed subset of Rust is published as one page?", "Yes, the plan says all three".

- Minor 47. Asked "A shadow test is accepted automatically when its time limit runs out (your
  choice). Is that recorded in the download history as \"accepted by timeout\", and as an event?",
  "Yes, recorded as such".

- Minor 48. Asked "A one-value fix is a whole deployment (your choice). Do the documents say that
  \"force once now\" on a variable is the online change of a value (TIA's Modify), and that when the
  doubled scan of a shadow test does not fit, the way is stop and download, the popup saying so?",
  "Yes, both said".

**Ruled in part** 2026-11-26, by the architect, four minors:

- Minor 49. Asked "The three force modes use words a TIA user does not know. Do tables always show
  the mode in words, never the [!] mark alone?", "Yes, always in words".

- Minor 50. Asked "The state strip is English inside a translated interface (your choice; the detail
  view is translated). Which additions hold?", both chosen: "A \"seen\" mark on a request", "Decode
  the shadow term".

- Minor 51. Asked "Oficina stands on four outside projects on the Qt side (cxx-qt at 0.10,
  KDDockWidgets, Spix with one maintainer, Qt releasing every six months), and Qt's own Rust bridge
  is a preview until early 2027. The spike has since passed on cxx-qt. What holds?", "Pin Qt; review
  the bridge in 2027": one Qt version per release, and the bridge looked at again when Qt's own
  leaves preview.

- Minor 52. Asked "On a virtual machine without a GPU, Qt Quick's software backend cannot draw 3D at
  all. Is the single fallback the normal pipeline over a software rasteriser (WARP on Windows,
  llvmpipe on Linux), chosen before the first window, so that switching is a restart?", "No 3D
  without a GPU", against the review's wording: without a GPU Oficina runs with Qt Quick's software
  backend and the 3D view is unavailable.

**Ruled in part** 2026-11-26, by the architect:

- Minor 54. Asked "The components have Portuguese names by law L2, and a foreign user meets
  \"Supervisão\", \"Comissionador\", \"Ábaco\". Do the screens show each component's role as a
  subtitle, the command palette matching unaccented spellings and English aliases?", "Yes, subtitle
  and aliases".

With it every entry of the review is ruled but minor 53 (items that wait on undecided work, to be
marked so in the plans: bookkeeping) and minor 55 (a crash and a power cut look alike: ruled with
Q228's point 8 on 2026-11-25).

## Q32 - What disqualifies an edit from a live takeover?

**Raised** 2026-11-15, by the architect, ruling on the review's entry 5.

"live fallback shall be enabled only when feasible. we need to build the specification for what
disqualifies the edits for auto takeover. takeover after restart remains possible for all cases".
The specification is not built. One candidate criterion, the assistant's and not ruled: the
watching version keeps its right to take over only while the comparison has shown no difference
between its outputs and the driven ones since the swap. Riders of the assistant's that nobody
ruled on: a failed edit leaves a fault record; the state text gains a term for the armed fallback
and its time left; the event is named "the edit failed", not a controller fault.

**Taken meanwhile**: no live takeover is built before the specification. The previous version can
always be put back for the next start.

**Ruled in part** 2026-11-25, by the architect:

- The criterion. Asked "Live takeover (you ruled: \"live fallback shall be enabled only when
  feasible. we need to build the specification for what disqualifies the edits for auto takeover\"):
  is this the first criterion? The watching version keeps its right to take over only while the
  comparison has shown no difference between its outputs and the driven ones since the swap.",
  "Write the specification first": controlador writes the whole specification as a proposal, and the
  architect rules on it as a document. The candidate criterion is neither taken nor refused.

- A failed edit. Asked "Does an edit that failed (the old version took over, or the edit was put
  back) leave a fault record, the event being named \"the edit failed\" and not a controller
  fault?", "A record, named \"the edit failed\"".

**Ruled in part** 2026-11-25, by the architect, the last rider:

- The state text. Asked "While the old version is watching and may still take over after an edit,
  does the state text carry a term for the armed fallback and the time it has left?", "Yes, a term
  with its time left". The term joins core's table of the state text. What stays open in this entry
  is the specification itself, controlador's to write.

**Decided** 2026-11-26, by the architect, on the specification controlador wrote
(`docs/controlador/live-takeover.md`), in four answers:

- The document. Asked "The live-takeover specification (`docs/controlador/live-takeover.md`). Its
  principle: the old version may take over only when everything the plant received since the swap is
  what the old version itself computed from the same inputs. It lists S1 to S4 (the fallback cannot
  be armed), D1 to D7 (the right ends while watching: an output differs, an outside write not taken
  alike, a force, a halt, the watcher in trouble, a device event, the time limit) and T1 to T4 (the
  takeover itself). Is this the specification?", "Yes, this is the specification".

- S3. Asked "Controlador's own reading: the fallback cannot be armed when the edit changes what
  peers see (the published variables, the Modbus addresses, the OPC UA nodes, or which variables are
  writable from outside), since a takeover would change those under a peer. Does S3 belong in the
  list?", "No, S3 out": an edit may change what peers see and still arm the fallback.

- D3. Asked "How strict: a force applies to both versions alike, but a forced plant is not a plant
  the watcher computed for. Which forces end the old version's right to take over?", "Any force (the
  proposal)".

- The time limit. Asked "The plan says the old version watches for a limited time, after which the
  edit is accepted (recorded as accepted by timeout, ruled today). Whose is the limit, and what is
  its default?", "Engineer's, default ten minutes": a setting of each download.

## Q33 - Where is controlador's execution model written, and what does it say?

**Raised** 2026-11-15, by the review's entry 13; the architect chose the smaller first release
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
"Before the code"; no code stands on an assumed model. The assistant's proposal for every point,
written on 2026-11-22, is `docs/controlador/execution-model.md` (Q223); code stands only on the one
case every option runs alike, a single cyclic task.

**Decided** 2026-11-25, by the architect, with Q223, which answered this question with a proposal:
asked "Controlador's execution model is proposed in `docs/controlador/execution-model.md`: tasks
share only the process image, each task its own slice of it, a fieldbus thread above the logic that
is never halted, a program clock paused at a halt, and blocks interrupting each other on ONE logic
thread at instruction boundaries (first release). Accept it?", "Accept as proposed (Recommended)".
The model is written in that file, and the plan carries each decision in a line.

## Q34 - Where do the tests of core's fieldbus masters against the bench live?

**Raised** 2026-11-15, by the ruling on the review's entry 10 ("Core, rules amended").

The masters stay in core. Law L4 stands: core depends on no other member in any table, the test
table included, so core's own tests cannot start maquinar's bench devices. The ruling did not say
where those tests live. And "callable by other crates for testing only" stays a rule in prose that
no law checks.

**Taken meanwhile**: nothing is written. The candidates: the tests of the first consumer
(controlador), or a crate of repo tooling under `tools/`.

**Decided** 2026-11-25, by the architect, with Q101, which asked it for the first master: asked
"Where do the tests of core's fieldbus masters against the bench's devices live? The laws keep the
bench out of core's own tests. Taken meanwhile: a test-only tool crate, `tools/mestres`.",
"tools/mestres (Recommended)": a test-only crate of repo tooling that depends on core and maquinar,
one test file per master, controlador's own tests beside it when it drives real devices.

## Q35 - Which licence expression do the crates carry?

**Raised** 2026-11-15, by the ruling on the review's entry 14.

Oficina is GPLv3. Core, portal, `orcl-std` and every crate Oficina links get a permissive licence
that GPLv3 accepts, and a check of dependency licences runs in CI. Not ruled: whether that is MIT,
Apache 2.0, or the dual expression `MIT OR Apache-2.0` usual for Rust crates. The review also
noted that a licence file at the repository's root would cover the whole repository, where GPLv3
is meant for Oficina alone, and that its reading is no legal advice.

**Taken meanwhile**: no licence is stated in any manifest and no licence file is written. Nothing
is published before this is answered.

**Decided** 2026-11-25, by the architect: asked "Which licence do the suite's permissive crates
carry (core, portal, `orcl-std` and all that Oficina links)? Oficina itself is GPLv3. Taken
meanwhile: no licence is stated anywhere, in a public repository. This is no legal advice.", "MIT OR
Apache-2.0 (Recommended)", against MIT alone, Apache-2.0 alone and "Not yet". The option's text:
"The dual expression usual for Rust crates, and the IronCalc fork's own. Each crate states it in its
manifest and carries its licence files; Oficina states GPL-3.0; no single licence file at the root
claims the whole repository." Not ruled with it: the copyright holder the licence files name, and
whether Oficina's GPL is version 3 only or version 3 or later.

**Decided further** 2026-11-25, by the architect, the two points the first answer left. Asked "The
MIT licence text names a copyright holder (\"Copyright (c) 2026 ...\"), in a public repository. Whom
do the licence files name?", "thebdr", the GitHub account that owns the repository, against "The
Veredas authors" and a name given. Asked "Oficina is GPLv3. Does its manifest say version 3 only, or
version 3 or any later version? This is no legal advice.", "GPL-3.0-or-later".

**Decided further** 2026-11-25, by the architect, on the scope: asked "The ruling of 15 November
gives a permissive licence to every crate Oficina links. Do the crates Oficina does not link
(supervisão, controlador, `orcl-runtime`, and the repo's tools) carry MIT OR Apache-2.0 too?",
"Every crate but Oficina". So every crate of the suite states `MIT OR Apache-2.0` and Oficina alone
`GPL-3.0-or-later`; the IronCalc fork keeps the licence it came with.

## Q36 - How is a download's integrity built?

**Raised** 2026-11-16, by the architect's requirement that a download carry "a hash or crc so no
corrupted program may run".

Ruled so far: the requirement; the recheck at power-on; two copies with a hash per block
(`docs/robustness.md`). The assistant's proposal for the rest, put to the architect on 2026-11-16
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

**Decided** 2026-11-25, by the architect, point by point:

- The manifest. Asked "A download's integrity (you required \"a hash or crc so no corrupted program
  may run\"): is it a manifest listing every part of the download with its SHA-256 and length, the
  SHA-256 of the manifest being the download's identity?", "SHA-256 manifest (Recommended)", against
  a CRC per part.

- Storing. Asked "Does the controller store it to a temporary file, read it back from the drive,
  verify it, and only then rename it into place and answer with the identity it computed?", "Yes,
  read back first (Recommended)", against verifying the bytes as received.

- A failed check. Asked "The stored program is verified again before every load (a swap, a restart,
  a power-on; the power-on recheck is already ruled). When the check fails and the two copies cannot
  mend each other, what happens?", "FAULT, no substitute (Recommended)": the FAULT reason 506
  `load`, never an automatic substitute; against the previous version running by itself.

- Naming. Asked "Do the swap, start and rollback commands name the download's identity, so that a
  command can only ever act on the version its sender meant?", "Yes (Recommended)".

A signature over the same manifest stays with the security round (Q43).

## Q37 - How is every request that alters the controller made robust?

**Raised** 2026-11-16, by the architect: "every bit of data that alters the controller shall be
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
- Read literally, the architect's sentence also covers peers' writes, where secured OPC UA already
  signs and numbers its messages and Modbus/TCP has nothing beyond TCP's checksum. The assistant
  read the scope as the Oficina link.
- The tunnel's end on the laptop is reachable by every program there, a page in a browser
  included, so the server should refuse a `Host` it does not expect and the session value should
  be random (P-014).
- Which gate a command passes. Portal's write surface is "a whitelist over the handles' own
  gates", "never a passthrough" (C-044), and every write path obeys portal's master switch
  (`docs/portal-plan.md`, "Rules"). The ruling of 2026-11-16 has portal carry Oficina's commands
  and controlador check the commissioning switch behind its portal. Whether a command needs
  portal's switch too, the commissioning switch alone, or both is not ruled.
- From the ruling that a write from outside is saved at once: the answer to a write should tell
  its sender how far it got, applied and then saved, so a panel shows a setpoint as kept only when
  it is. A write followed by a power cut inside the shortest gap is still lost.

**Taken meanwhile**: nothing is built; the proposal is the assistant's pick.

**Ruled in part** 2026-11-25, by the architect, point by point, the first four of the proposal:

- The digest. Asked "Every request that alters the controller (a variable write, a breakpoint, a
  state change, a download): does each carry a SHA-256 of its content, as HTTP's Content-Digest (RFC
  9530), which the controller checks before acting?", "Yes, a digest each (Recommended)", against
  downloads only.

- Order and repeats. Asked "Does each altering request carry a session and a strictly increasing
  number, a repeat being answered with the first outcome and not run again, and a gap or an old
  number refused?", "Yes, session and number (Recommended)".

- Addressing. Asked "Does each altering request name the controller's identity, the download's
  identity, and for a state change the state it expects to leave, so that a request reaching the
  wrong controller, the wrong program or the wrong state is refused?", "All three named
  (Recommended)".

- Freshness. Asked "Must each altering request echo the controller's own last clock value, so that
  an old request held back somewhere and delivered late is refused?", "Yes, echo the clock
  (Recommended)".

**Ruled in part** 2026-11-25, by the architect, four more points:

- A lost answer. Asked "When no answer comes back to an altering request, does Oficina show the
  outcome as \"unknown\" and ask the controller for it by the request's number, never assuming it
  failed or succeeded?", "Unknown, asked by number (Recommended)", against treating it as failed.

- Applying. Asked "Is each altering change applied whole, at one point of the scan, never half in
  one scan and half in the next?", "Yes, whole, at one point (Recommended)".

- The answer. Asked "Is the answer to an altering request read back from the controller itself (the
  value now in memory, the new state text, the full breakpoint list), not an echo of what was
  asked?", "Yes, read back (Recommended)", against a bare acknowledgement.

- The scope. Asked "Your sentence \"every bit of data that alters the controller shall be robust\"
  was read as the Oficina link. Does it also bind peers' writes, where secured OPC UA already signs
  and numbers its messages and Modbus/TCP has nothing beyond TCP's checksum?", "The Oficina link
  only", against binding peers where the protocol can and against refusing a protocol that cannot
  protect a write.

**Ruled in part** 2026-11-25, by the architect, the last two points put; with them every point of
the proposal is ruled:

- The gates. Asked "Oficina's commands reach the controller through its embedded portal, and
  controlador checks the commissioning switch behind it. Which gate must a command pass: portal's
  own write switch, the commissioning switch, or both?", "Both switches": a command passes portal's
  master write switch and then the controller's commissioning switch; against the commissioning
  switch alone and portal's switch alone.

- A write's answer. Asked "Since a write from outside is saved at once, held to the shortest gap,
  does the answer tell its sender how far the write got (applied, then saved), so a panel shows a
  setpoint as kept only when it is?", "Yes, both stages told (Recommended)".

Left for the security round (Q43, P-014): a keyed check on altering requests, the refusal of an
unexpected `Host` and the random session value.

## Q38 - What must survive: which data is inside, and the border cases

**Raised** 2026-11-16, in the robustness round (`docs/robustness.md`, "What must survive").

The rule is the architect's: "every piece of data that is necessary for computing the logic
flawlessly in RUN mode", the commissioning scope outside it. The assistant's application of it,
given to the architect and not ruled:
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

**Ruled in part** 2026-11-25, by the architect:

- The list. Asked "What must survive (your rule: \"every piece of data that is necessary for
  computing the logic flawlessly in RUN mode\"): the assistant read as INSIDE the running program,
  the hardware configuration and IO mapping, retained variables, the settings a scan depends on, the
  interpreter, the operating system and what boots them, and the integrity data; as OUTSIDE forces,
  breakpoints, a shadow test, the commissioning switch, earlier downloads, fault records, the state
  history, the store, the logs. Is that the list?", "Yes, as listed (Recommended)", with its rider:
  what is outside may be lost but must never harm what is inside.

- The source. Asked "A border case: the source of the running program. RUN does not need it, but
  \"the code is open\" holds for the next engineer only if it can be read from the machine. Is the
  source inside what must survive?", "Inside (Recommended)".

**Ruled in part** 2026-11-25, by the architect, the last three border cases; with them every point
of this entry is ruled:

- Peers' data. Asked "What peers need to talk to the controller (the Modbus and OPC UA settings and
  the certificates). Are they inside what must survive when the project uses them?", "Inside when
  used (Recommended)".

- A dry run. Asked "A dry run. It is already ruled to survive a power cycle. Does it get the
  protection meant for decades (two copies, hashes, the scheduled check)?", "Survives, unprotected
  (Recommended)".

- The clock. Asked "For a program with time-of-day blocks, is the clock (the date, once confirmed)
  inside what must survive, so that the hardware description of such a target must name a clock chip
  that keeps time?", "Inside for such programs", against leaving it outside, confirmed again after
  each start.

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

**Ruled in part** 2026-11-25, by the architect:

- 64-bit only. Asked "64-bit only (ruled for the processor): does it cover the operating system too,
  the build refusing a 32-bit target? A 32-bit system on a 64-bit processor brings the 32-bit time
  value back. Out would be the 32-bit ARM boards (BeagleBone Black, i.MX 6, STM32MP1, early
  Raspberry Pi).", "Yes, the system too (Recommended)".

**Ruled in part** 2026-11-25, by the architect, four more points:

- When NTP counts as silent. Asked "Only after a waiting time without an answer (at power-on, the
  start-up time), the request being not urgent until then? Otherwise every power-on with a dead
  clock battery sends an urgent request for a few seconds.", "After a waiting time (Recommended)".

- Urgency. Asked "A maintenance request's urgency: does it follow the present situation (back to not
  urgent when NTP returns), while the request itself stays until cleared in Oficina or by a restart,
  by the rule of 14 November?", "Urgency follows, request stays (Recommended)".

- One reason or two. Asked "The clock battery and NTP: one maintenance reason for both, or two? A
  board with no clock chip counts as a dead battery, and the \"ignore\" setting is what such a board
  needs.", "Two reasons": one for the clock chip that lost its time, one for NTP silent or not set.

- The mark of an unconfirmed date. Asked "Which records carry it? Fault records and the state
  history are controlador's; the store's samples are portal's, whose file format would have to
  change.", "Controller's records and the store": fault records, the state history and every sample
  in the embedded portal's store; portal's store format gains the mark by a new version.

**Ruled in part** 2026-11-25, by the architect, the last four points; with them every point of this
entry is ruled:

- The state text. Asked "While the controller's date is unconfirmed, does the state text show it as
  a condition term?", "Yes, a condition term". The term joins core's table of the state text.

- Missed starts. Asked "It is ruled that a time-of-day block whose moment passed while the date was
  unconfirmed never runs. By the same reasoning, does a start that fell while the controller was
  powered off also not run at the next start?", "It does not run (Recommended)".

- The certificates. Asked "The maintenance request a year before any certificate expires is ruled.
  Does \"its certificates\" mean every certificate in the controller's store, its own and its
  peers'?", "Its own and its peers' (Recommended)".

- The cycle-time rule. Asked "The warning cycle time raises a maintenance request, not urgent, with
  a reason of its own beside `overruns`, and the maximum gives a free-running main block its limit.
  Does the controller also record each task's longest cycle per day, so the drift shows before the
  warning?", "Request, and the daily longest cycle".

Left for the security round (Q43): the certificates' end date after 2038, and what a secured link
does while the date is unconfirmed.

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

**Ruled in part** 2026-11-25, by the architect, point by point:

- What a save holds. Asked "Does a save hold the retained variables as they stood at the end of ONE
  successful cycle, never a mix of two?", "One cycle's end, never a mix (Recommended)".

- How it is stored. Asked "Two copies, each with its hash, written one after the other (so one whole
  copy is left when power is cut during a save). With both unreadable at start, the retained
  variables reset and the request `retain_reset` is raised. Is that it?", "Two copies, reset if both
  lost (Recommended)", against a FAULT when both are lost.

- The flag. Asked "The flag \"started from the last save\" (on a target that cannot save at power
  loss): is it raised only when cycles ran after the last save, a power cut while the controller
  sits in STOP raising none?", "Only if cycles ran since (Recommended)".

- No request. Asked "A start from the last save: a power cut being such a target's normal case, does
  it raise no maintenance request, the state text showing it as a condition instead?", "No request,
  a condition (Recommended)"; the term's spelling is core's to propose.

- Who says what a target can do: the target's hardware description, ruled with Q226 the same day
  ("Accept all (Recommended)").

**Ruled in part** 2026-11-25, by the architect, four more points:

- Nothing changed, nothing written. Asked "Is a save skipped when its content equals the last one
  (nothing changed, nothing written), to spare the drive?", "Yes, skipped (Recommended)".

- The period. Asked "Is it the engineer's and never refused, with Oficina showing the years the
  drive has left at that period where the drive reports its wear?", "Engineer's, never refused
  (Recommended)", against refusing a period below a floor.

- The shortest gap. Asked "The shortest gap between saves (ruled for writes from outside): is it the
  engineer's setting with a default, a write that arrives inside the gap being applied at once and
  saved when the gap ends, and Oficina showing the drive's shortest life at that gap?", "Yes, as
  proposed (Recommended)".

- A save on the program's request. Asked "May the program itself ask for a save of the retained
  variables, for example at the end of a batch?", "Yes, held to the shortest gap", against a save at
  once whatever the gap, and against none.

**Ruled in part** 2026-11-25, by the architect, the last point; with it every point of this entry is
ruled:

- The save failing while the machine runs. Asked "RUN goes on, the values being in memory. Is the
  maintenance request not urgent while one copy can still be written, and urgent when none can (the
  next power cut would then reset the retained variables)?", "Not urgent, then urgent
  (Recommended)", against urgent at the first failure and against a FAULT when none can be written.

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

**Taken meanwhile**: on a controller the store is the memory window, as ruled. Since 2026-11-22
portal has built three pieces of this area, each on the most reversible answer and none ruled:
- the window in memory has a span and no ceiling in bytes (C-090);
- a recording on the drive stops at its size limit, deletes nothing and counts the rows that came
  after, which is (b) for that record (C-091);
- every fault gets a copy of the store, none is deleted, and nothing limits their number or their
  rate (C-093).

The rest waits: fault records, the state history, earlier downloads and the logs have no limit
and no rule yet.

**Ruled in part** 2026-11-25, by the architect, the question in hand when the round paused: asked
"When a growing record on a controller's drive (fault records, the state history, earlier downloads,
logs, a recording, the fault's copies of the window) reaches its size limit, what gives way? Portal
built the second option for a recording meanwhile.", "The oldest is deleted", (a), with its two
guards: a fault that repeats is counted on its record and not written again, and the running program
and its fallback are never deleted. So Q16's "nothing is refused, deleted or merged" holds for a
project's store on a PC and not for a record on a controller's drive, and what portal built
meanwhile for a recording (C-091: it stops at its limit) and for the fault's copies (C-093: none
deleted) gives way.

**Ruled in part** 2026-11-25, by the architect, three of the proposals above:

- Storage in three kinds. Asked "Three kinds kept apart, so wearing or filling one cannot touch
  another: what RUN needs (written only at a download), the retained save (written often), and
  everything that grows (on a data area of its own). Is that the rule?", "Yes, three kinds apart
  (Recommended)".

- Wear is watched. Asked "Where the drive reports its wear, does the controller read it and raise a
  maintenance request (not urgent) at a threshold the engineer sets, with Oficina showing the years
  left at the present write rate?", "Yes (Recommended)".

- The window's ceiling. Asked "The memory window of a controller's embedded portal: it has a span in
  time and, as built, no ceiling in bytes. Does it get a ceiling too, the window being the shorter
  of the two and the controller saying so when the ceiling came first?", "Span and a ceiling in
  bytes (Recommended)", against "Span only" as C-090 has it. The engine's own default memory limit
  is never left as it is on a controller.

**Ruled in part** 2026-11-25, by the architect, four more of the proposals above; with them every
point of this entry that is the architect's is ruled:

- A failed data area. Asked "When the area that holds a controller's growing records fails or cannot
  be written, does RUN go on, recording stopping and a maintenance request being raised?", "RUN goes
  on (Recommended)", against "The controller faults".

- What the window holds. Asked "What the memory window of a controller's embedded portal holds:
  everything that portal reads, unless the project narrows it? Oficina reads the window when it
  connects, so it sees the time before its connection.", "Everything, project may narrow
  (Recommended)", against "Only what the project lists".

- The fault's copies. Asked "The fault's copy of the window (written to the drive with the fault
  record, so whoever comes later sees the minutes before): as built, every fault gets a copy and
  nothing limits their number or rate. What bounds them?", "A number and a rate (Recommended)":
  copies limited in number, the oldest going first, no more than one written per shortest gap, and
  the program may ask for a copy too; against a number only, and against C-093 as built.

- Portals on PCs. Asked "Supervisão's and Oficina's portals have the same unbounded store, where Q16
  ruled \"nothing is refused, deleted or merged\". Do they get a default size limit?", "No limit, as
  Q16 ruled", against a default limit with the oldest deleted, a default limit with recording
  stopped, and leaving it to Q42.

Left with their owners, not rulings: how the window is built so that it hands memory back (portal's
plan to prove), and the operating system's own writing (the Linux study, Q226: volatile logs with
pstore). "Everything that grows has a size limit" follows from the first ruling of this day.

## Q42 - The robustness round: the areas not opened, and the Linux study

**Raised** 2026-11-16, when the round was called.

Not opened:
- running unattended beyond the cycle-time rule: the hardware watchdog, memory errors, slow leaks;
- an old controller and a new Oficina: versions, migration, a rescue path;
- replacing the hardware: backup, restore, what a stranger needs to rebuild;
- which other components count as critical;
- how each rule is proven by injecting faults on the bench.

Owed before the operating-system rules: the study of Linux for real time and of its recovery
methods (`docs/robustness.md`, "The Linux study"). Written as a study on 2026-11-22,
`docs/controlador/linux-study.md`; the choices it leaves are Q226. The assistant's caution, not
ruled: on hardware we do not choose, each cycle's data can be made consistent every cycle but
reaches the drive only at set moments.

**Taken meanwhile**: nothing. The round resumes with the architect.

**Ruled in part** 2026-11-25, by the architect:

- Which other components count as critical. Asked "Which other components count as critical, bound
  by the robustness round beside controlador and the portal embedded in it? Choose all that apply
  (others in the note).", with supervisão and its portal, comissionador and Oficina offered: "None
  beyond the controller". The round binds the controller and its embedded portal alone.

The Linux study is written and its choices ruled (Q226). Still not opened, each waiting for a
proposal before it can be put: running unattended beyond what is ruled (memory errors, slow leaks),
an old controller and a new Oficina, replacing the hardware, and how each rule is proven by
injecting faults on the bench.

**Ruled further** 2026-11-26, by the architect: the four areas this entry left unopened were
proposed by controlador as Q234 to Q237 and ruled there (Q235 in part). With them every area of the
round has been opened.

## Q43 - The security round: the positions going in, and its list

**Raised** 2026-11-16, by the architect: "dedicated round to decide cyber security level
suite-wise, future proof is the best way to design".

The architect's positions going in: default user and password pairs written in a public manual do
not cover it, "most users will leave defaults"; "a password is always better than no password, and
on the network controlador appears as a regular linux machine"; "once secured behind a login,
controlador is just another linux machine. it is open source, and the responsability to keep it
safe belongs to the integrator imo". The briefing had put security out of controlador's first
scope: "none for now, I need to study".

The assistant's inputs, not ruled: no default credential at all, the first commissioning step
setting it (or SSH keys only); the data format is public in an open project, so it is no barrier;
European product rules put duties on whoever supplies a product commercially, so the architect's
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

**Ruled in part** 2026-11-26, by the architect, point by point (the review's security entries 32 to
39 are ruled under Q31, entry 33 answering Q29 and P-014's two items):

- A controller's first credential. Asked "A controller's first credential. Your positions: default
  pairs in a public manual do not cover it (\"most users will leave defaults\"), \"a password is
  always better than no password\", and keeping it safe \"belongs to the integrator\". What does a
  controller ship with?", the architect's own words, none of the options (none set at first
  commissioning by force, SSH keys only, a documented default): "credential-less usage permitted,
  discouraged". How it is discouraged is put next.

- An expired certificate. Asked "A known peer's certificate has passed its end date (the maintenance
  request a year ahead is ruled). Does the secured link to that peer cut, or go on with a warning?",
  "Goes on, urgent request": the link stays up and an urgent maintenance request stands until the
  certificate is renewed.

- An unconfirmed date. Asked "While the controller's date is unconfirmed (dead clock battery, no NTP
  yet) it cannot judge any certificate's dates. What does a secured link do meanwhile?", "Accept
  known peers, dates unjudged": a peer whose certificate the controller already trusts is accepted
  without judging dates, the link marked so; an unknown certificate is refused as always.

**Ruled in part** 2026-11-26, by the architect, four more points; with them the round's list as it
stood is gone through:

- How credential-less use is discouraged. Asked "Following your answer (\"credential-less usage
  permitted, discouraged\"): how does a controller without a credential discourage it?", "Oficina's
  warning only": Oficina warns at each connection to a controller without a credential, and the
  controller itself raises nothing; against a maintenance request, a condition in the state text,
  and the documents alone.

- Certificates after 2038. Asked "The suite's certificates all end on 19 January 2038 (ruled). A
  certificate issued late lives short, and after that date the rule can issue none. Which end date
  do certificates issued from then on carry?", "Decide with the security study".

- A signed download. Asked "A hash guards a download against corruption, not against a person. Does
  the download's manifest carry a signature, which the controller checks before it accepts the
  download?", "With the security study".

- A keyed check. Asked "Yesterday's rule (digest, session, number, identities, freshness) guards
  against transmission faults, not against a person. Does each altering request also carry a keyed
  check (HTTP Message Signatures, RFC 9421)?", "With the security study".

So three points wait for the security study the architect postponed (the briefing's J4): the end
date after 2038, a signature over a download's manifest, a keyed check on altering requests. Its
first output is ruled: the gap list against the Cyber Resilience Act's Annex I (Q31 entry 38).

## Q44 - Conversion to native PLC code: the type table's open points

**Raised** 2026-11-15, by the architect stating the goal: controlador's ORCL program and its
hardware configuration convert to native PLC code, Siemens S7-1500 and S7-1200 first.

The architect began a table mapping ORCL data types to S7 types. In it S7's BYTE, WORD, DWORD and
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

**Taken meanwhile**: nothing is written; the table is the architect's draft and is not in this
repository.

**Decided** 2026-11-25, by the architect, point by point:

- The types. Asked "Conversion to native PLC code (S7 first): below `f64` the left column of your
  type table is not Rust (`char` is 32 bits in Rust; `string`, `time`, `date` have no fixed form;
  `orcl-std` uses Rust's `Duration`, which matches neither TIME nor LTIME). Do these become
  `orcl-std` types of their own with a fixed representation?", "Yes, orcl-std's own types".

- The CPU families. Asked "The classic S7-1200 lacks LWORD, LINT, ULINT, LTIME, LTOD, DT and LDT,
  while the S7-1200 G2 is grouped with the 1500. Does the table get a column per CPU family, a
  program that uses a type its target lacks being refused for that target?", "Yes, a column per
  family", against the S7-1500 only at first.

- A second front end. Asked "Readable SCL: names and comments survive only in a front end that reads
  the source, while the interpreter stands on the compiler's intermediate form (ruled 15 November).
  Do the SCL translators and the ladder view get a second front end that reads the source?", "Not
  yet": decided when the conversion is the work in hand.

The table itself is still the architect's draft, outside this repository.

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
- the "knowledge protection" the architect mentioned in passing is a picture of the trade, not a
  requirement: no such lock exists in ORCL's decisions.

Open, never asked:
- whether "identify, name and address" runs from the laptop or through a target (G2);
- where the readers of GSDML, ESI and EDS files live (C5);
- whether "good" in supervisão is a new state class or the colour of the normal one (K12);
- whether the hardware configuration checks a Modbus module's own timeout in production too;
- whether Npcap's licence allows what raw frames on Windows need.

**Taken meanwhile**: the readings stand as readings, marked so wherever a plan leans on one.

**Ruled in part** 2026-11-25, by the architect, point by point:

- K1. Asked "The state text: are the arguments inside the `force` term always written in the order
  in, out, var (the counts of forced inputs, outputs and variables)?", "Yes: in, out, var".

- K3. Asked "A force is suspended while the program is halted (at a breakpoint). Does it apply again
  by itself when the program continues?", "Yes, it applies again".

- K4. Asked "A shadow test (a new version computing beside the driving one): with the two versions'
  IO mapping allowed to differ, is their output compared per physical channel?", "Per variable
  name", against the assistant's reading (per physical channel): outputs are compared by the
  program's variable names.

- K5. Asked "The state text: the start-up's phases (load, restore, handler, devices) have numbers.
  Does the text show the phase by its word alone, without an `id`?", "Word alone, no id".

- K2 and K6 were ruled with Q100 ("Accept all eight (Recommended)", its points 5 and 6), K7 with
  Q228 (its point 8), and the list of core operations with Q227, all the same day.

**Ruled in part** 2026-11-25, by the architect, four more points, three of them against the
assistant's readings:

- Windows. Asked "Does Windows gain a third use for controlador's code, beside development and
  simulation: the IO check from a laptop (a light controlador session on the laptop driving a remote
  IO module, which you kept on 15 November under three rules)?", "Linux laptops only": the IO check
  from a laptop runs where controlador runs for real; on a Windows laptop it goes through a target.

- A shadow test at a power cycle. Asked "A shadow test not yet accepted when the power is cut. You
  ruled \"always as before\". Does that mean the version that was driving drives again, the test
  stays unaccepted, and the fallback flag resets as decided?", "The shadow test is dropped": the
  driving version drives again and the unaccepted test is gone, started again by a person.

- Knowledge protection. Asked "The \"knowledge protection\" you mentioned in passing (locking a
  block's source, as PLC vendors offer). Is it a picture of the trade and not a requirement, ORCL
  having no such lock?", "A requirement, later": a way to protect a block's source is wanted
  eventually and goes on the list.

- K12. Asked "In supervisão every state has a mark and a word. Is \"good\" (green) a new state class
  of its own, or the colour of the existing normal class?", "The colour of normal".

**Ruled in part** 2026-11-25, by the architect, three of the points never asked before:

- G2. Asked "\"Identify, name and address a device\" (blink a PROFINET device, give it its name and
  IP address). Does it run from the laptop itself, or through a target controller?", "Both": from
  the laptop when it is on the segment, and through a target's embedded aranha when it is not.

- C5. Asked "Where do the readers of vendor description files live (GSDML for PROFINET, ESI for
  EtherCAT, EDS for EtherNet/IP)? Oficina's hardware configuration reads them, and the bench and
  controlador may too.", "In core", against a member of their own and Oficina.

- A Modbus module's own timeout. Asked "The IO check from a laptop refuses to drive outputs on a
  Modbus module that has no timeout of its own or too long a one. Does the hardware configuration
  check a Modbus module's own timeout in production too, for a real controller?", "In production
  too, as a warning": the configuration warns and the engineer may accept it.

Not a ruling but a fact still to check: whether Npcap's licence allows what raw frames on Windows
need. With it this entry is ruled throughout.

---

## Q46 - Line endings on every machine

**Raised** 2026-11-22, taking the work up on the Linux machine (the handoff's first decision).

A run record hashes a test file's bytes as they stand on disk. The Windows desk checked out CRLF
(`core.autocrlf=true`), so its records read stale on a Linux clone, whose files are LF, and the
reverse would hold for records taken on Linux and read in a Windows clone.

Options: every machine checks out LF (`* text=auto eol=lf` in `.gitattributes`; the repository
holds no script that needs CRLF, and the page's folder already worked this way on Windows); or
`.gitattributes` stays as it was, records are taken on Linux only and the contract tool is never
run in a Windows clone.

**Taken meanwhile**: every machine checks out LF, and the runs are recorded again once on the
Linux machine. The Windows VM gets a clone of its own and takes no records.

**Reversible?** Yes: one line of `.gitattributes`, and the records taken again.

**Decided** 2026-11-25, by the architect: asked "Which line endings do the repository's files have
on every machine? Run records hash a test file's bytes, so CRLF on Windows made records stale on
Linux. Taken meanwhile: every machine checks out LF.", "LF everywhere". What was taken stands.

---

## Numbers from Q100 on: one block per member

From 2026-11-22 several member sessions write at once, each in a worktree of its own
(`CLAUDE.md`, "Several sessions at once"). So that no two members claim one number, a member's
new question takes the next free number of its block: core Q100, portal Q130, maquinar Q160,
supervisão Q190, controlador (with ORCL) Q220, aranha Q280, comissionador Q310, ábaco Q340,
oficina Q370, releases Q430; a block ends where the next begins. Suite-wide questions run on
from Q46. A question is appended at the end of this file whatever its number.

---

## Q280 - Does aranha keep an interface of its own?

**Raised** 2026-11-22, planning aranha (P-005; `docs/aranha-plan.md`).

Veredas 1's aranha is an executable serving its own web page (axum, JSON and SSE, a vite and
TypeScript page embedded in the binary, about 1 100 lines of server and the page beside it): the
device table, the drawn topology, the baseline diff, the identify and write buttons. Since then
the architect decided that the bench has no interface of its own and that its surfaces are
Oficina's (Q12, revisited 2026-11-13), and the briefing gave Oficina the topology view and the
hardware configuration's scan, compare and merge, which aranha's scan feeds (`docs/oficina-plan.md`;
the briefing's G1).

Options:
- (a) No interface of its own: a library (what Oficina and controlador embed) and one command-line
  executable (listen on an interface and write the scan file, identify, diff two scans). Aranha's
  surfaces (the table, the topology, the diff, identify, name and address) are Oficina's, as the
  bench's are; until Oficina draws them, the scan file is read as text.
- (b) Keep Veredas 1's page, served by aranha's own executable: a commissioning tool usable today
  in a browser, and a second front end beside Oficina's to keep in step.
- (c) A page in supervisão: supervisão owns no plant connection and starts portal sessions only
  (Q19), so it would show a scan file, never scan.

**Recommended and taken meanwhile**: (a). Nothing of the page is carried; the library's types are
what a surface draws, and the executable writes the same file a surface would read.

**Reversible?** Yes: a page, or Oficina's surfaces, sit on the library; nothing below changes.

**Decided** 2026-11-25, by the architect: asked "Does aranha (the network scanner) keep an interface
of its own, as Veredas 1's had (its own web page)? Taken meanwhile: no, a library and one
command-line executable.", "No interface (Recommended)", (a). What was taken stands: aranha's
surfaces are Oficina's.

---

## Q430 - What is the release member's name?

**Raised** 2026-11-22, planning suite releases and the tester machine (`docs/entrega-plan.md`).

The suite names things in Portuguese by their role. What ships is no component, so it takes no
`veredas-` crate: a plan, `docs/<name>-plan.md`, and a tool, `tools/<name>`.

Options: **entrega** (delivery: what is handed over, the release and the machine alike);
**lancamento** (lançamento, a launch, as a version is "lançada"); **safra** (a harvest's vintage,
one release a year's crop).

**Taken meanwhile** (recommended): `entrega`.

**Reversible?** Yes: a `git mv` of the plan and the tool, and the clauses' links.

**Decided** 2026-11-25, by the architect: asked "What is the release member's name (its plan
`docs/<name>-plan.md` and its tool `tools/<name>`)? Taken meanwhile: `entrega`.", "veredas-alforge",
a name of the architect's own, against `entrega`, `lancamento` and `safra`. Asked then "You named
the release member `veredas-alforge`. Today it is repo tooling (`tools/entrega`, no `veredas-`
crate, exempt from the component laws). Is alforge a component of the suite, or tooling under the
new name?", "Tooling tools/alforge". So the member is alforge (a saddlebag: what the rider carries):
`tools/alforge` and `docs/alforge-plan.md`, repo tooling still, exempt from the component laws, the
crate named `alforge` since the laws keep the `veredas-` prefix for components. The rename is the
release member's to make.

---

## Q431 - What version does a suite release carry?

**Raised** 2026-11-22, planning suite releases. Each component has its own SemVer (the plan,
"Versioning"); P-011's exit names `2.0.0-alpha.1` as the first release.

Options: (a) a SemVer of the release's own, from `2.0.0-alpha.1`, bumped by the largest bump among
its components since the release before (a component's MAJOR makes the release's MAJOR), so the
release's number still says whether something broke; (b) the date, `2026.11` then `2026.11.1`,
which says when and nothing of compatibility; (c) core's version, which says nothing of the apps.

**Taken meanwhile** (recommended): (a), the first release `2.0.0-alpha.1`.

**Reversible?** Yes, until a release is published: the manifest's `release` and its file's name.

**Decided** 2026-11-25, by the architect: asked "What version does a suite release carry, given each
component keeps its own SemVer? Taken meanwhile: a SemVer of the release's own, the first
`2.0.0-alpha.1`.", "Its own SemVer (Recommended)", (a). What was taken stands.

---

## Q432 - Does a release tag its commit in git?

**Raised** 2026-11-22. A manifest names the commit it was cut from; a tag `suite-<release>` on it
would make the commit findable by name and show on GitHub.

Options: (a) a manifest alone, no tag; (b) an annotated tag `suite-<release>`, pushed with the
manifest's commit; (c) a tag per component too (`veredas-portal-2.0.0-alpha.1`).

**Taken meanwhile** (most reversible): (a). A tag pushed is public; nothing is tagged until ruled.

**Reversible?** Yes, while nothing is tagged.

**Decided** 2026-11-25, by the architect: asked "Does a suite release tag its commit in git? Taken
meanwhile (most reversible): no tag, the manifest alone names the commit. A pushed tag is public.",
"No tag", (a). What was taken stands: nothing is tagged.

---

## Q433 - Which light Linux distribution does the tester machine run?

**Raised** 2026-11-22, by the architect's "a light linux distro" (P-022).

Options:
- **Debian 13 "trixie"** (recommended): stable since 2025, supported to 2028 and to 2030 by its
  long-term team; official cloud images in dated builds with their SHA-512 sums, the `generic`
  one carrying the full kernel with VMware's drivers (paravirtual SCSI, VMXNET3, SVGA) and
  cloud-init for an unattended build; glibc 2.41; Chromium and open-vm-tools packaged as plain packages. The base is
  `debian-13-generic-amd64.qcow2` from `cloud.debian.org`, 435 MB, downloaded once.
- **Alpine Linux**: the smallest (a 65 MB installer), but built on musl: the suite's executables
  would need a musl build, and the database engine's C++ needs a musl C++ toolchain this machine
  lacks.
- **Ubuntu 26.04 LTS** (minimal cloud image): newer libraries, but its Chromium and Firefox are
  snaps, which a small appliance carries badly.
- **Fedora**: 13 months of support per release, too short for a machine testers keep.

**Taken meanwhile**: Debian 13, the smallest official image that fits (the `genericcloud` one, 344
MB, carries a kernel without VMware's display driver).

**Reversible?** Yes: one table of the recipe's file (`tools/entrega/maquina/maquina.toml`) and the
provisioning's package names.

**Decided** 2026-11-25, by the architect: asked "Which light Linux distribution does the tester
machine (the VMware image of a release) run? Taken meanwhile: Debian 13.", "Debian 13
(Recommended)". What was taken stands.

---

## Q434 - Which desktop does the tester machine carry, if any?

**Raised** 2026-11-22. Designers look at supervisão in a browser; Oficina (Qt Quick, X11 first)
comes later and needs a desktop.

Options:
- **Xfce** (recommended): light (about 500 MB of memory with the browser open), on X11 as Oficina's
  first Linux target is, and open-vm-tools' screen resizing works with it.
- **LXQt**: lighter still, built on Qt as Oficina is, younger on X11 and Wayland both.
- **No desktop**: the machine serves supervisão to the host's browser and shows a console; the
  smallest image, no Oficina later without adding one.
- **A kiosk**: the browser alone, full screen, on supervisão; no terminal or editor for the plant's
  files, no Oficina.

**Taken meanwhile**: Xfce, logged in automatically, the browser opened on the plant.

**Reversible?** Yes: the recipe's package list and its login settings.

**Decided** 2026-11-25, by the architect: asked "Which desktop does the tester machine carry, if
any? Taken meanwhile: Xfce, logged in automatically, the browser opened on the plant.", "Xfce
(Recommended)". What was taken stands.

---

## Q435 - Which VMware products and versions must the machine open in?

**Raised** 2026-11-22. VMware cannot be run on the development machine; what opens where is
decided by the OVF descriptor's virtual hardware version and devices.

Options for the products: VMware Workstation Pro 17 (Windows and Linux, free for every use since
November 2024), Fusion 13 (Macs with Intel processors), ESXi 7 and 8. Apple-silicon Macs run only
arm64 guests in Fusion: reaching them needs a second, arm64 image (the suite built for aarch64).

Options for the virtual hardware: version 14 (Workstation 14, Fusion 10, ESXi 6.7 and every later
one), version 19 (ESXi 7.0 U2, Workstation 16.2) or version 21 (ESXi 8.0 U2, Workstation 17.5):
a later product opens an older version, never the reverse.

**Taken meanwhile** (recommended): Workstation Pro 17, Fusion 13 on Intel and ESXi 7 and 8, at
hardware version 14, x86_64 alone; a paravirtual SCSI controller, a VMXNET3 card, BIOS firmware.

**Reversible?** Yes: the descriptor's version and devices are the recipe's data.

**Decided** 2026-11-25, by the architect: asked "Which VMware products must the tester machine open
in? Taken meanwhile: Workstation Pro 17, Fusion 13 on Intel Macs, ESXi 7 and 8, at virtual hardware
version 14, x86_64 alone.", "Hardware 14, x86_64 (Recommended)", against a second image for arm64,
hardware version 19 and hardware version 21. What was taken stands: no arm64 image, so Apple-silicon
Macs are not served.

---

## Q436 - How is the tester machine delivered?

**Raised** 2026-11-22.

Options: (a) an **OVA**, one file every VMware product opens (VirtualBox too), 1.09 GB for the
first release (measured on 2026-11-22);
(b) a zipped folder with a `.vmx` and its `.vmdk`, which Workstation and Fusion open directly and
ESXi does not; (c) both. And where: a GitHub release of the repository (public, 2 GB per file, so
a larger OVA would be split), or a share the architect names.

**Taken meanwhile** (recommended): (a), built and kept on the development machine, published
nowhere until ruled (publishing is outward-facing).

**Reversible?** Yes: nothing is published.

**Decided** 2026-11-25, by the architect, in two answers. Asked "In which form is the tester machine
delivered? Taken meanwhile: one OVA file (1.09 GB for the first release).", "OVA (Recommended)",
(a). Asked "Where is the tester machine published? Taken meanwhile: nowhere, it is built and kept on
the development machine.", "Nowhere yet", against a GitHub release and a share of the architect's.
What was taken stands: the image stays on the development machine and the architect hands it over.

---

## Q437 - How does a tester update the machine?

**Raised** 2026-11-22.

Options: (a) a new OVA per release, the old machine thrown away (its plant files with it); (b) an
update command in the machine that fetches a newer release bundle, checks its sums and its
manifest, installs it beside the old one under `/opt/veredas/` and moves `current`, the plant's
files left alone; (c) a package repository of the suite, signed, that the distribution's own
updater reads.

**Taken meanwhile**: (a), since nothing is published yet; (b) recommended once releases are.

**Reversible?** Yes.

**Decided** 2026-11-25, by the architect: asked "How does a tester update the machine to a newer
release? Taken meanwhile: a new OVA per release, the old machine thrown away with its plant files.",
"Update command (Recommended)", (b): a command in the machine fetches a newer release bundle, checks
its sums and its manifest, installs it beside the old one under `/opt/veredas/` and moves `current`,
the plant's files left alone. Where the command fetches from waits on Q436's place, ruled "Nowhere
yet" the same day.

---

## Q438 - What user and password does the machine ship with?

**Raised** 2026-11-22.

Options: (a) user `veredas`, password `veredas`, logged in automatically, `sudo` asking that
password, no SSH server, the password stated on the release's page (recommended: a test machine,
behind VMware's NAT by default); (b) a password drawn per build, printed in the release's notes;
(c) no password at all, `sudo` without one; (d) a password changed at the first login, which ends
the automatic login.

**Taken meanwhile**: (a).

**Reversible?** Yes: the recipe's user table.

**Decided** 2026-11-25, by the architect: asked "What user and password does the tester machine ship
with? Taken meanwhile: user and password both `veredas`, logged in automatically.", "veredas /
veredas (Recommended)", (a). What was taken stands.

---

## Q439 - Where does supervisão listen inside the tester machine?

**Raised** 2026-11-22. Supervisão binds the loopback by default (Q20: local, no accounts); a
tester's or a designer's browser on the host reaches the machine only on its network address.

Options: (a) every interface of the machine, port 8902, writes disarmed in the plant (VMware's
default NAT lets the host alone reach it; a bridged network would show it to the LAN); (b) the
loopback alone, the page seen only in the machine's own browser.

**Taken meanwhile** (recommended): (a), as a plant's own supervisão file (`bind = "0.0.0.0:8902"`):
no code of supervisão changes. Q29's gaps hold as everywhere.

**Reversible?** Yes: one line of the plant's supervisão file.

**Decided** 2026-11-25, by the architect: asked "Where does supervisão listen inside the tester
machine? Taken meanwhile: every interface, port 8902, writes disarmed in the plant.", "Every
interface (Recommended)", (a). What was taken stands.

---

## Q440 - Does the tester machine's test run with the suite?

**Raised** 2026-11-22. The image test downloads a 435 MB base once, needs KVM, and takes minutes:
it installs a desktop from the distribution's mirror.

Options: (a) its own command (`cargo nextest run -p entrega --features maquina`), recorded with its
clause and run at every cut, outside `cargo nextest run --workspace`; (b) in every run of the suite,
GitHub's included.

**Taken meanwhile** (recommended): (a); the cut's run of the bundle (C-291) likewise rides a
feature, since it builds the suite in release mode.

**Reversible?** Yes: a feature in the tool's manifest.

**Decided** 2026-11-25, by the architect: asked "Does the tester machine's image test run with the
suite? It downloads a 435 MB base once, needs KVM and takes minutes. Taken meanwhile: its own
command, run at every release cut.", "Its own command (Recommended)", (a). What was taken stands.

---

## Q340 - Where does the IronCalc fork live, and how do its fixes go upstream?

**Raised** 2026-11-22, planning ábaco (P-007; `docs/abaco-plan.md`, "The fork").

Ábaco's engine is a fork of IronCalc (MIT OR Apache-2.0). Veredas 1 kept it as a git subtree at
`crates/ironcalc` with upstream's 2 222 commits, excluded from its workspace, and made eleven
changes on it, none sent upstream. Here law L1 lets a crate out of the workspace only by an
`exclude` entry, and only if it is not a `veredas-*` or `orcl-*` crate; law L2 makes every member
under `crates/` a `veredas-<nome>`, so the fork's crates cannot be members; law L6 reads every
commit of the history. The suite builds the fork's `base` (the engine) and `xlsx` (the file layer)
only.

Options:
- (a) A copy inside its host: `crates/veredas-abaco/ironcalc/{base,xlsx}`, excluded, upstream's
  files at a recorded commit in one commit and each change a commit of its own, listed in
  `VEREDAS-FORK.md` with an upstream column; upstream's history not imported; a newer upstream
  brought in by diff, measured by the oracle before and after; a fix sent upstream as a patch from a
  clone of upstream. One repository, one push per change, builds offline.
- (b) The same place as a full-history subtree, Veredas 1's way: `git subtree pull` and `push`
  work, and this repository's history takes in upstream's 2 222 commits (L6 walks them; it is
  tested to a bound at 3 000).
- (c) A fork repository of its own on GitHub, the suite depending on it by git URL pinned to a
  commit: pull requests upstream come straight from its branches and this repository holds no
  engine code; every engine change lands in two repositories, and creating the fork is the
  architect's act on their account.
- (d) Where Veredas 1 had it, `crates/ironcalc`, excluded: L1 allows it, but a folder under
  `crates/` that is no component reads as one.

Whichever is chosen, opening a pull request on upstream's repository is public and is the
architect's act.

**Recommended and taken meanwhile**: (a). The fork arrives as Veredas 1 left it (`base` and
`xlsx`, upstream `15ecfec` plus the eleven changes), checked against a clone of upstream.

**Found** 2026-11-22, building on (a): cargo makes a crate beneath a listed member's folder a member
whatever `exclude` says (a member's path that is a prefix of the crate's wins over the exclusion).
With the fork at `crates/veredas-abaco/ironcalc`, cargo built and tested the fork's two crates as
members of the suite, while the laws passed (L1 reads the manifest's lists as written). So (a) is
not to be had as written, and one more option joins the list:
- (e) A copy at the repository's root, `forks/ironcalc`, excluded: outside every member's folder, so
  the exclusion holds; outside `crates/` and `tools/`, where L1 leaves a crate that is not the
  suite's alone; otherwise as (a).

**Recommended now and taken meanwhile**: (e), the fork arriving as (a) described; (c) if the
architect wants upstream pull requests to flow often.

**Reversible?** Yes: a `git mv` and two path dependencies for (b) or (d); (c) replaces the folder
with a git dependency.

**Decided** 2026-11-25, by the architect: asked "Where does ábaco's fork of IronCalc (the
spreadsheet engine) live, and how do its fixes go upstream? Taken meanwhile: a copy at the
repository's root, `forks/ironcalc`, excluded from the workspace.", "forks/ironcalc here
(Recommended)", (e), against a fork repository on GitHub, a full-history subtree and
`crates/ironcalc`. What was taken stands; a fix goes upstream as a patch, and opening a pull request
there stays the architect's act.

---

## Q341 - What becomes of Veredas 1's grid, now that the grid's surface is Oficina's?

**Raised** 2026-11-22, planning ábaco (P-007).

Veredas 1 gave Ábaco its own grid, drawn with egui in the worker's process (its AB16, the
architect's ruling of 2026-11-05: the whole keyboard reaches the grid, no round trip per
keystroke), in a crate of its own, `veredas-abaco-grid`, over a renderer-neutral core inside
`veredas-abaco`: Excel's navigation and selection, the editor's modes, the clipboard's TSV,
filters, and the view the worker sends with the gestures it receives back. The briefing then put
the grid in Oficina, as a tab, with the engine and Python of each workspace in its own process
(A6.1), and Oficina is Qt Quick with its logic in Rust behind one bridge seam.

Options:
- (a) The renderer-neutral core is carried into `veredas-abaco` as a library module; Oficina links
  it in its own process and paints, so no keystroke crosses a process; the worker serves the
  model (cells, values, edits, recalculation) over a local interface agreed with Oficina's plan.
  Oficina's `MAP` row (P-015) then names ábaco.
- (b) The core runs in the worker and Oficina sends every key and gesture across the process
  boundary: the round trip Veredas 1 ruled out when it chose egui over a browser.
- (c) Nothing of the grid is carried: Oficina writes the grid's behaviour itself.
- (d) The egui grid is carried as Ábaco's own window until Oficina's tab exists, then dropped.

**Recommended**: (a). **Taken meanwhile**: nothing of the grid is ported until Oficina's tab is
planned; the egui crate is not carried.

**Reversible?** Yes: nothing is written.

**Decided** 2026-11-25, by the architect: asked "What becomes of Veredas 1's spreadsheet grid, now
that the grid's surface is a tab of Oficina (Qt Quick)? Taken meanwhile: nothing of the grid is
ported until Oficina's tab is planned.", "Carry nothing", (c), against the recommendation (a):
nothing of Veredas 1's grid is carried, neither its egui crate nor its renderer-neutral core, and
Oficina writes the grid's behaviour itself.

---

## Q342 - Is Veredas 1's Excel host carried?

**Raised** 2026-11-22, planning ábaco (P-007).

Veredas 1 drove a controlled, out-of-process Excel over COM as Ábaco's first renderer (its AB4
and AB13): the model pushed into Excel, gestures and the cells a tool declared editable read
back, Windows only. It also backed one of the three answers when a workbook with links opens,
"open and calculate in Excel" (AB7), which was never wired. In Veredas 2 the scope says
byte-faithful edits with no Office on the machine, and Oficina is the one native application.

Options:
- (a) Not carried: Oficina's grid is the one renderer; Excel opens a file Ábaco wrote like any
  other program, and the "open and calculate in Excel" answer is dropped.
- (b) Carried as a Windows-only feature of the worker, a second renderer beside Oficina's tab.
- (c) Carried later, after Oficina's tab, if a plant laptop with Excel and without Oficina needs
  it.

**Recommended and taken meanwhile**: (a); nothing of the host is ported.

**Reversible?** Yes: the host stood on the same view and events as the grid, and can be ported
on them later.

**Decided** 2026-11-25, by the architect: asked "Is Veredas 1's Excel host carried into ábaco (a
controlled Excel driven over COM as a renderer, Windows only)? Taken meanwhile: not carried.", "Not
carried (Recommended)", (a). What was taken stands, and the "open and calculate in Excel" answer is
dropped.

---

## Q343 - Is acerto carried into this public repository, and how?

**Raised** 2026-11-22, planning ábaco (P-007; decision 4 of 2026-11-05).

Decision 4 carries Veredas 1's `python/veredas-acerto` as `acerto`, an Ábaco extension inside its
host. Acerto is a line-for-line port of an older in-house tool: it aligns one company's plant
engineering files (its IO lists, the PLC's plant exports, the SCADA's workbooks) by that company's
conventions, with that tool's tests ported unchanged. Two facts were not on the table on
2026-11-05, or not side by side: this repository is public, while Veredas 1's is private; and on
2026-11-09 the architect ruled that Veredas 1's parsing of one company's engineering exports is
"not at all" carried into supervisão, "in the suite or beside it" (Q18), after the content-agnostic
ruling of Q8. Carrying acerto publishes its logic, and whatever its tests and fixtures hold.

Options:
- (a) Carried whole, as decision 4 says: `crates/veredas-abaco/acerto/`, its logic public, its
  fixtures read first and any customer, site or plant name, address or real file left out.
- (b) Carried as a generic extension: the alignment's mechanics (two statements of one plant
  brought into agreement, the queue, the apply through the patcher) with every company convention
  as a rule file outside the repository, as Q8 and Q9 did for a device's telegrams.
- (c) Not carried here: acerto stays a package of the private repository, installed into Ábaco
  through the extension point (Veredas 1's tools found by entry point), and this repository holds
  the extension point and a small public example.

**Recommended**: (c), or (b) if acerto is to ship with the suite; (a) only if the architect is
content for the in-house tool's logic to be public. **Taken meanwhile**: nothing of acerto is
carried; it is the last phase of the plan, so nothing waits on the answer yet.

**Reversible?** Only before: a publication is not undone by a revert.

**Decided** 2026-11-25, by the architect: asked "Is acerto (Veredas 1's port of an in-house tool
that aligns one company's plant engineering files) carried into this public repository? Publishing
is not undone by a revert. Taken meanwhile: nothing of acerto is carried.", "Not carried here
(Recommended)", (c): acerto stays a package of the private repository, installed into Ábaco through
the extension point; this repository holds the extension point and a small public example. Decision
4 of 2026-11-05 gives way in that.

---

## Q344 - What is ábaco's parity measured on while Veredas 1's workbooks are not here?

**Raised** 2026-11-22, planning ábaco (P-007's exit: "at parity with Veredas 1's oracle numbers").

Veredas 1's oracle numbers come from five real workbooks of one plant (1 217 533 formula cells, all
equal to Excel's cached values). They were copied from a cloud folder on the Windows desk, never
committed (Veredas 1's repository has only test workbooks), and they are not on this machine: not
on its disk, not in Veredas 1's checkout, not in the Google Drive mount (searched by name
2026-11-22). They hold a customer's engineering data and never enter this public repository.

Options:
- (a) The architect puts the five workbooks on this machine, outside any repository; the oracle
  runs on them from that path and only the numbers and the files' hashes are recorded.
- (b) Parity is measured on the workbooks Excel saved among the fork's own test files (about 250,
  from Excel on Windows, on the Mac and online, their cached values Excel's): Veredas 1's oracle
  and fork run on this machine give the baseline, and Veredas 2 must be equal or better file by
  file.
- (c) Both: (b) verifies the clause now, (a) is added to its evidence when the files come.

**Recommended**: (c). **Taken meanwhile**: (b).

**Reversible?** Yes: a corpus added to the evidence.

**Decided** 2026-11-25, by the architect: asked "What is ábaco's parity with Excel measured on,
while Veredas 1's five real workbooks (a customer's data, 1.2 million formula cells) are not on this
machine? Taken meanwhile: the about 250 Excel-saved workbooks among the fork's own tests.", "Both
(Recommended)", (c): the fork's Excel-saved workbooks verify the clause now, and the five real
workbooks join the evidence when the architect puts them on this machine, outside any repository,
only the numbers and the files' hashes recorded.

---

## Q345 - How does ábaco read its stamps while core carries its network layers?

**Raised** 2026-11-23, building ábaco's `.abaco` file (C-224; `docs/abaco-plan.md` phase 3).

Every stamp of the suite is read by core's one header reader (C-045, `veredas_core::schema`), and
the `.abaco` file's TOML parts are stamped. `veredas-core` pulls its OPC UA client, tokio, socket2,
regex and chrono whatever features are asked, while its `schema` needs only serde, toml and
thiserror; Ábaco's component clause (C-220) holds no plant connection in its graph. Asked of core
through the orchestrator on 2026-11-23 (the shared notes' `abaco-asks-core-for-the-header-reader-alone.md`).

Options:
- (a) Core puts its network layers behind a default feature, so `default-features = false` gives
  `schema` alone; Ábaco depends on core then, as L5's map lets it.
- (b) Ábaco depends on core whole: its graph gains the plant stack C-220 refuses.
- (c) Ábaco compiles core's own `src/schema.rs` by path (`#[path]`): the one reader's very code,
  its order and texts, no copy to drift; the day (a) lands it becomes `use veredas_core::schema`.
- (d) Ábaco writes a reader of its own: a second source of truth, which drifted at once (C-224's
  confrontation, 2026-11-23: a version past 2^63 refused before the format was judged).

**Recommended**: (a). **Taken meanwhile**: (c), marked `// DECISION PENDING (Q345)` in
`crates/veredas-abaco/src/abacofile.rs`.

**Reversible?** Yes: one line, once core's feature lands.

**Measured** 2026-11-23, once core's `net` feature landed (C-073; `docs/core-plan.md`
"Features"): with `veredas-core = { path = "../veredas-core", default-features = false }`, Ábaco
built alone carries `schema` on serde, toml and thiserror, as asked. A workspace build unifies a
package's features across its members, though: `cargo tree --workspace` shows Ábaco's subtree
holding `veredas-core [default, demo-server, net]`, and under it tokio, socket2 and the OPC UA
client, the plant stack C-220's workspace-subtree check refuses (every suite run, and the
workspace's own `cargo test`, builds Ábaco that way). So (a) brings that stack into every test
build of Ábaco, while (c) keeps the reader's very code without it. A further option:
- (e) The header reader in a crate of its own (`schema` alone, on serde, toml and thiserror),
  which core and Ábaco both depend on: no unification can bring the plant stack in. A new member
  under L1 to L6 (its plan, its name), core's to propose.

**Recommended now**: (c) stays, or (e) if core takes it; (a) only if the architect accepts the plant
stack in Ábaco's workspace builds (its release build, made alone, would not carry it).

**Decided** 2026-11-25, by the architect: asked "How does ábaco read the suite's file stamps without
pulling core's network stack into its builds? Taken meanwhile: it compiles core's own `schema.rs` by
path. Core's `net` feature landed, but a workspace build still unifies the stack in.", "Compile
core's file by path (Recommended)", (c), against a crate of the reader's own (e) and core without
its default features (a). What was taken stands.

---

## Q346 - How does Excel spell PERCENTOF in a file?

**Raised** 2026-11-23, answering C-223's eighth refute round (fork change 25, `forks/ironcalc/VEREDAS-FORK.md`).

A function Excel added after its 2007 file format is written in a file with `_xlfn.` (`_xlfn.DAYS`),
and Excel reads one spelled otherwise as `#NAME?`. The fork now writes every function as Microsoft's
grammar, rust_xlsxwriter's list and the Excel-saved workbooks of its tests spell it. PERCENTOF
(Excel 365, 2024) is in none of the three.

Options:
- (a) `_xlfn.PERCENTOF`, as every function Excel added after 2010 is written.
- (b) `PERCENTOF`, plain.

The architect could settle it with one workbook saved by Excel 365 whose cell holds `=PERCENTOF(1,{1,2})`:
its sheet XML shows the spelling, and the workbook can join the fork's tests.

**Recommended**: (a). **Taken meanwhile**: (a), marked `// DECISION PENDING` in
`forks/ironcalc/base/src/functions/mod.rs` (`to_xlsx_string`).

**Reversible?** Yes: one line, and the base test's list.

**Decided** 2026-11-25, by the architect: asked "How does Excel spell PERCENTOF (Excel 365, 2024) in
a file? No source ábaco has shows it; a wrong spelling reads as #NAME? in Excel. Taken meanwhile:
`_xlfn.PERCENTOF`.", "_xlfn.PERCENTOF (Recommended)", (a), against saving a workbook in Excel 365 to
show it. What was taken stands, a reasoned spelling no file from Excel has shown.

---

## Q310 - The test file's syntax

**Raised** 2026-11-22, planning comissionador (P-006; `docs/comissionador-plan.md`).

Decision 6 settles one plain-text test file with two views; the design draws a language of its own
(`docs/design/comissionador/Text.dc.html`, "a sketch to make the idea concrete, not a settled
grammar"), while the plan says every plain-text config and project file is TOML. A test is a
program (steps, conditions, branches, scripts, loops in functions), so the choice is the
architect's. Options:
- (a) The design's language, refined (`docs/test-schema.md`): line by line, `step <name>
  <action>`, `branch` with arms, `on <outcome> -> <route>`, functions in `.fn` files, the
  `layout` block last; the suite's stamp kept as two TOML lines read by core's one header
  reader, so a wrong format or version is refused by the texts every file shares.
- (b) TOML throughout: settings as keys, steps as `[[step]]` tables, every condition, script and
  function body a string inside them, the flow view's positions a table.
- (c) A subset of Rust, as ORCL is: a test a Rust function over a test library, the flow chart
  drawn from Rust's syntax.

The assistant recommends (a): it reads as the design does, for testers who are not developers;
the conditions and scripts need a language in any case, and (b) only moves that language into
strings, which neither the text view nor a diff shows well; (c) suits a developer and makes the
flow view a reading of arbitrary Rust.

**Taken meanwhile**: (a).

**Reversible?** Yes, while the files are few: the engine runs a model that does not depend on the
syntax, and the syntax is one front end (`lex`, `parse`); the worked examples would be rewritten.

**Decided** 2026-11-25, by the architect: asked "In which syntax is a comissionador test file
written? A test is a program (steps, conditions, branches, scripts). Taken meanwhile: the design's
own language, refined.", "The design's language (Recommended)", (a), against TOML throughout and a
subset of Rust. What was taken stands.

---

## Q311 - The script box's language

**Raised** 2026-11-22, planning comissionador (P-006).

A script box holds a step with no clean graphical form, a calculation say; a function's body is
written in text too. Which language they are written in is open. Options:
- (a) The test language's own statements, the same for script boxes and functions: assignments,
  `if`, `for` over a range, arithmetic, comparisons, a few calls (`abs`, `min`, `round`, ...);
  in a function also waits and awaits over portal's data; no other input or output.
- (b) An embedded scripting language: Rhai (Rust-like, pure Rust, MIT or Apache) or Lua (through
  a C library), behind the same `script` keyword.
- (c) Python, as ábaco's macros are: an interpreter on every machine that runs a test, and a
  sandbox to build around it.

The assistant recommends (a): one language for the whole file, which the check can type and the
flow view can show; deterministic, so a replay computes what the live run computed; and a script
box that cannot reach the plant needs no review of what it might do. (b) is the way to a general
language later, beside (a), if tests outgrow it.

**Taken meanwhile**: (a). A script box reads no plant data and does not wait; a function reads
and waits but never writes.

**Reversible?** Yes: (b) can be added beside (a) under a tagged `script` block; replacing (a)
rewrites the scripts and functions written meanwhile.

**Decided** 2026-11-25, by the architect: asked "In which language are comissionador's script boxes
and function bodies written? Taken meanwhile: the test language's own statements.", "The test
language (Recommended)", (a), against Rhai or Lua embedded and Python. What was taken stands.

---

## Q312 - What an acceptance report holds, in which form, and who signs it

**Raised** 2026-11-22, planning comissionador (P-006). The design leaves open who signs a report,
whether a safety test needs a witness, and the PDF's layout.

Taken as its contents, every run's: the test (its id, title, file, the exact text that ran and
the function files with it); the run (its number, mode, the portal session's name, the window on
the store's axis, its start and end, its verdict and the step that decided it); every executed
step in order (attempt, outcome, detail, times); every criterion (each `check`, each `branch`'s
decision: the expression, the values measured, the result); the values `report` recorded; every
gated write made and the plant's answer; the evidence, the rows of every table the run read over
its window; a tester's note; the signatures. Options for its form:
- (a) A TOML record per run, `veredas-report` version 1, in `runs/<id>/<nnnn>/`, the evidence as
  CSV files beside it; the PDF drawn from the record by Oficina's run-and-report screen.
- (b) The engine writes the PDF itself, beside the record.
- (c) An HTML page per run.

And for signing:
- (d) Anyone signs, by a name typed (no accounts, Q20); a signed report is frozen (a note or a
  second signature is refused, a correction is a new run).
- (e) As (d), and a test may name the roles that must sign (`sign tester, witness`); the report
  reads "awaiting witness" until the roles have signed.

The assistant recommends (a) and (e): the record is plain text, versioned and diff-able like
every file of the suite, any drawing of it reads the record, and the PDF's layout stays with the
screen the design gives it to (A6.2).

**Taken meanwhile**: (a) and (d); (e) is not built until the answer.

**Reversible?** Yes: the record is the source; a PDF or a page is a drawing of it. Freezing a
signed report is the one thing a later answer could not undo for reports already signed.

**Decided** 2026-11-25, by the architect, in two answers. Asked "In which form does comissionador
keep a run's acceptance report? Taken meanwhile: a TOML record per run with the evidence as CSV
files beside it.", "TOML record (Recommended)", (a). Asked "Who signs an acceptance report? There
are no accounts (Q20). Taken meanwhile: anyone signs by a typed name, and a signed report is
frozen.", "Roles named by the test (Recommended)", (e): anyone signs by a typed name and a signed
report is frozen, and a test may name the roles that must sign, the report reading "awaiting
witness" until they have. (e) is now to be built.

---

## Q313 - How comissionador reaches portal

**Raised** 2026-11-22, planning comissionador (P-006).

A run reads the plant and writes to it through portal. Options:
- (a) In process: the engine is handed a portal session (its `Live`), and comissionador's
  executable starts the session from the plant's project file, as supervisão does (Q19).
- (b) Over portal's own HTTP interface, the one two portals speak (P-019): windowed reads and the
  write surface; the engine then runs against any portal it can reach (one it did not start, the
  one embedded in a controller, through Oficina's SSH) without running a session itself.
- (c) Both, behind one interface.

The assistant recommends (c), built in the order (a) then (b): (a) needs nothing new from portal
and gives the tests a session they control; a test run from Oficina against a controller will
come through (b) once portal's link between two portals exists.

**Taken meanwhile**: (a), behind the interface (`Plant`) that (b) would implement.

**Reversible?** Yes: the interface is the engine's one door to portal.

**Decided** 2026-11-25, by the architect: asked "How does comissionador reach portal to read and
write the plant? Taken meanwhile: in process, behind an interface a second way could implement.",
"Both, in process first (Recommended)", (c): in process now, then over portal's HTTP interface once
the link between two portals exists (P-019).

---

## Q314 - Who arms portal's write switch for a run

**Raised** 2026-11-22, planning comissionador (P-006).

The design's inspector says a test's writes need "portal's write switch armed when the run
starts", and its run screen reads "write switch: off (disarmed after run)". Options:
- (a) Never comissionador: the switch is the project's and the person's (set in the project file
  or by the process that owns the session); a live run whose test writes is refused while the
  switch is off; a write the switch refuses mid-run ends its step in `error`.
- (b) The run arms the switch at its start, once the person has confirmed the list of the test's
  gated writes, and disarms it at its end.
- (c) As (a), and a run disarms at its end a switch it found armed.

The assistant recommends (a): portal sets the switch in process and never over the wire, and
supervisão never arms it either; with (b) the engine would hold the power to arm, and the
confirming step belongs to Oficina (as its own arming step for the laptop's IO check does). (c)
can surprise another user of the same session, a supervisão page among them.

**Taken meanwhile**: (a). The crate's sources name no call that arms the switch, and a test
holds that by reading them as tokens, as supervisão's does.

**Reversible?** Yes: (b) or (c) is a call added at the run's start or end.

**Decided** 2026-11-25, by the architect: asked "Who arms portal's write switch for a comissionador
run whose test writes to the plant? Taken meanwhile: never comissionador; a live run that writes is
refused while the switch is off.", "The run arms and disarms", (b), against the recommendation: the
run arms the switch at its start, once the person has confirmed the list of the test's gated writes,
and disarms it at its end. What was taken meanwhile (no call that arms the switch, and the test that
reads the sources for one) gives way.

---

## Q315 - What a write does in a replay

**Raised** 2026-11-22, planning comissionador (P-006).

A replay runs a test over a stored window, where the plant cannot be written. The design shows a
test with writes run in replay. Options:
- (a) Not sent: the step passes with the note "not sent: replay" and the clock does not move.
- (b) Matched against the window: the clock moves to where the stored rows show the tag take the
  written value (the live run's own write), the step failing when they never do.
- (c) A test that writes is refused in replay.

The assistant recommends (a): a replay of a live run's own window starts where that run started,
so the waits after a write meet what the write caused; (b) fails on a tag that is written but
never polled, and a message sent is not a row of the store.

**Taken meanwhile**: (a).

**Reversible?** Yes: one action's behaviour in one mode.

**Decided** 2026-11-25, by the architect: asked "What does a write do when comissionador replays a
test over a stored window, where the plant cannot be written? Taken meanwhile: not sent, the step
passes with a note.", "Not sent, step passes (Recommended)", (a). What was taken stands; where the
clock stands after it is Q318's.

---

## Q316 - How retries count, and how they show in the report

**Raised** 2026-11-22, planning comissionador (P-006). The design leaves open how retries count
across nested branches and how they appear in the report.

Options:
- (a) Each retry route (a `retry <step> max <n>` in an arm or an `on` line) counts its own uses
  over the whole run, never reset; exhausted, the step that took it ends in `fail`, "retry of
  `<step>` exhausted after <n>".
- (b) A route's count starts again whenever the run reaches the route's step by any other way
  (a fresh pass through an outer loop).
- (c) One budget of retries for the whole run.

In the report, every attempt of a step is a row with its number (`route #1`, `route #2`), and the
row of the step that retried says which retry it was ("retry 1 of 1").

The assistant recommends (a): a reader counts it from the file alone, and a test that wants a
fresh budget per outer pass writes a second route.

**Taken meanwhile**: (a), with the report rows as above.

**Reversible?** Yes: a counter's scope.

**Decided** 2026-11-25, by the architect: asked "How do retries count in a comissionador run? Taken
meanwhile: each retry route counts its own uses over the whole run, never reset.", "Per route, whole
run (Recommended)", (a), with the report's rows as the entry draws them. What was taken stands.

---

## Q317 - How a test names plant data

**Raised** 2026-11-22, planning comissionador (P-006).

The design writes `Chute[12].ForceFull` and `Parcel.Announced`, a plant's own symbolic names.
Portal knows the plant by its tables and columns: a capture rule's, an OPC UA rule's or a
message's table, its fields or tags the columns. Options:
- (a) Portal's own names, `<table>.<column>`, as the store and the server list them; a write's
  column is an OPC UA tag the rule marks writable, a send names a channel and a message.
- (b) As (a), and aliases declared in a test or in a shared file (`tag ForceFull =
  plc_main.chute12_force`), with an index form for families of objects (`Chute[n].ForceFull`)
  mapped by a pattern.
- (c) The plant's symbolic names (OPC UA browse paths, a controller's variables), resolved
  through portal.

The assistant recommends (a) now and (b) when tests over many alike objects need it: (a) is what
a person sees in portal's tables and in supervisão, and needs nothing new; (c) reaches past what
portal stores.

**Taken meanwhile**: (a).

**Reversible?** Yes: (b) adds a resolution step before (a); the files written with (a) keep
working.

**Decided** 2026-11-25, by the architect: asked "How does a comissionador test name plant data? The
design writes `Chute[12].ForceFull`; portal knows tables and columns. Taken meanwhile: portal's own
names, `<table>.<column>`.", "Portal's names (Recommended)", (a), aliases (b) to be added when tests
over many alike objects need them. What was taken stands.

---

## Q318 - Where a replay's clock stands after a write

**Raised** 2026-11-23, by the comissionador session, from C-203's first refute round.

Live, a `write` or a `send` moves the run's clock to the moment the plant answered. A replay sends
nothing, and its clock stays where it was. After a step that read the plant, the live clock sits
at least the read's settle (250 ms) behind the machine. So a wait right after a write starts that
much, plus the plant's answer, earlier in a replay of the live run's own window. A wait whose
bound is tighter than that lag plus the plant's reaction holds live and times out replayed: a test
that writes `run := 1` after reading the plant, then waits 300 ms for a reaction 150 ms after the
write, passes live and aborts replayed. Options:
- (a) Portal keeps every gated write and send in the store with the moment it was answered, and a
  replay's write moves its clock to that moment when the window holds it. Needs portal (a
  table of writes), and replays every live run exactly.
- (b) A replay's write moves its clock by the read's settle, 250 ms. Closer, never exact: the
  lag depends on what the step before did, and the answer's time is not stored.
- (c) The claim narrowed: a replay gives a live run's verdict and values when no wait after a
  write needs a bound tighter than that lag and the reaction, and the reference says so.

The assistant recommends (a): portal already holds every write's answer in the live run, and a
store row per write would also show supervisão and the record who wrote what, when.

**Taken meanwhile**: (c). `docs/test-schema.md` ("Time") states the limit, C-203 states it, and
`crates/veredas-comissionador/tests/replay.rs` pins it.

**Reversible?** Yes: (a) or (b) changes where a replay's write leaves the clock, in one place of
the engine, and the test that pins the limit turns into one that asserts the same verdict.

**Widened** 2026-11-24, from C-203's second refute round: not only a wait's timeout but every
plant read right after a write (a read, a check, a report, a wait, an await) finds the plant as
it stood before the write in a replay; an await with a loose bound takes an earlier row. The
claim and the reference now exclude them all, and a second test pins it. (a) closes both.

**Decided** 2026-11-25, by the architect: asked "In a replay, where does the clock stand after a
write? Live it moves to the plant's answer; replayed it stays, so a read or a tight wait right after
a write can pass live and fail replayed. Taken meanwhile: the limit is stated.", "Portal stores each
write (Recommended)", (a): portal keeps every gated write and send in the store with the moment it
was answered, and a replay's write moves its clock to that moment when the window holds it. The
table of writes is portal's to build; until it lands the stated limit (c) holds.

---

## Q100 - The controller's state text: the readings its writer and reader take

**Raised** 2026-11-22, building the state text's writer and reader in core (P-018;
`docs/state-text.md`; `docs/controlador-plan.md`, "The state").

The plan fixes the words, the numbers, the order and the examples; the review's entries 17 to 21
on this text are not ruled (Q31). The writer and the reader needed an answer to each point below.
The plan's seven examples read and write back byte for byte under every reading taken.

1. **The unknown state's word.** The plan numbers it 0 and gives it no word. (a) `UNKNOWN`;
   (b) no text at all: a reader with no state shows nothing. Taken: (a). Its health: none.
2. **Which conditions are debug conditions** (health gives function check for "any debug
   condition", and a fault clears them; the plan never lists them; the review's entry 21).
   (a) What someone did and the mode it runs in: `breakpoint`, `force`, `shadow`, `dry`, `sim`,
   `no_realtime`; (b) only what someone did: `breakpoint`, `force`, `shadow`. Taken: (a), so a
   simulated or dry-run controller never reads normal.
3. **What `retain(reset)` holds.** (a) The number of retained variables reset; (b) a flag,
   `retain(reset=true)`. Taken: (a), the plan's "counts and identifiers".
4. **One name, two terms** (the review's entry 17). (a) The names stay and are told apart by
   their arguments: `breakpoint(id, at)` the reason, `breakpoint(set)` the condition; `devices(up,
   of)` the phase, `devices(missing)` the condition; a `HOLD` at a breakpoint also writes
   `breakpoint(set=n)` while breakpoints are set; (b) as (a), but a `HOLD` at a breakpoint leaves
   the count out, as the plan's third example reads; (c) the review's renames,
   `breakpoints(set)` and the phase as arguments of the start kind. Taken: (a); the plan's third
   example then reads as a halt whose breakpoint was removed while halted.
5. **The spelling the review's entry 18 asked to pin.** Taken, as the review proposed it:
   integers and booleans bare (`urgent=true`), strings in double quotes with TOML's escapes,
   every term with parentheses (`sim()`), `, ` between arguments, no space around `=`, one space
   between terms. Added: each value has one spelling (decimal only; a control character, a line
   separator or an invisible or reordering format character escaped by number, uppercase), and
   the reader refuses every other spelling, so a text and a state are one thing.
6. **Several maintenance requests** (K6, and the review's entry 19). The plan's `id` 1 to 6
   stands; requests stand by number, one not urgent before an urgent one, the same one twice
   allowed. The review's 601 to 606 with a count is not taken.
7. **A version, and a term a reader does not know** (the review's entry 20). Taken: no version,
   and the reader refuses a term it does not know, by name. The terms the robustness round owes
   (an unconfirmed date, the clock battery, a certificate near its end, the warning cycle time;
   Q39, Q40) are therefore refused until they are added to the table.
8. **The arguments' types.** `code` any 64-bit integer; `download`, the phase's counts and every
   condition's count 0 to 4294967295; `was` a layer-1 word; `kind`, `task`, `event`, `at` free
   strings. `STARTUP devices(up, of)` does not check `up <= of`.

**Recommended**: as taken, and the review's entries 17 to 21 ruled with them. **Taken meanwhile**:
as above, each marked `DECISION PENDING` in `crates/veredas-core/src/state.rs` where the code
leans on it.

**Reversible?** Yes while nothing stores the text: each point is a change to the writer, the
reader and their test. Once controllers publish it and portal stores it, a change is a new
version of the text, which point 7 does not yet give.

**Decided** 2026-11-25, by the architect: asked "Core's writer and reader of the controller's state
text took eight readings the plan left open (the unknown state's word `UNKNOWN`, which conditions
count as debug, what `retain(reset)` holds, one name for two terms, the spelling, maintenance
requests, no version, argument types). Accept them as taken?", "Accept all eight (Recommended)",
against going through them one by one. The eight points stand as taken above, and the review's
entries 17 to 21 on this text are ruled with them (Q31).

---

## Q101 - Where core's Modbus client meets the bench's Modbus server

**Raised** 2026-11-22, building the first of core's fieldbus masters (P-018, C-071). Q34 asks the
same for every master and was left with nothing written; the first master now needs an answer.

Law L4 keeps every member out of core's dependency tables, the test table included, so core's own
tests cannot start the bench's Modbus server. Core's test holds the client to the standard's
example frames over real TCP, and to every way a link fails; the whole device, the bench's
server, has to be met from a crate the laws allow.

Options:
- (a) A crate of repo tooling, `tools/mestres` ("the masters"), test-only: it depends on core and
  maquinar and holds one test file per master against the bench device that plays its devices.
  Core's session writes it; a tool may depend on anything (L5), needs no plan (L6) and keeps a
  version of its own.
- (b) The bench's own tests drive core's client against its server; the test is maquinar's file
  and the clause core's.
- (c) Controlador's tests, once controlador exists: the first consumer proves what it uses.

**Recommended**: (a) now, and (c) beside it when controlador drives real devices. **Taken
meanwhile**: (a). `tools/mestres` holds `tests/modbus.rs` since the bench's server (C-110) reached
`main` the same day.

**Reversible?** Yes: a tool crate removed or its tests moved.

**Decided** 2026-11-25, by the architect: asked "Where do the tests of core's fieldbus masters
against the bench's devices live? The laws keep the bench out of core's own tests. Taken meanwhile:
a test-only tool crate, `tools/mestres`.", "tools/mestres (Recommended)", (a), and (c) beside it
when controlador drives real devices. What was taken stands; Q34 is answered with it.

## Q220 - ORCL's version as a language, beside its crates' versions

**Raised** 2026-11-22, by the controlador session, carrying the ORCL crates in
(`docs/orcl/README.md`).

The suite's plan: "ORCL has its own SemVer as a language; controlador states which ORCL versions it
runs." Law L3 holds every crate under `crates/` to a version from 2.0.0, and it reads the language's
crates as it reads a component, so `orcl-std` and `orcl-runtime` start at `2.0.0-alpha.1`. In
Veredas 1 they were `0.1.0`, and the language had no number of its own.

Options:
- (a) The language's version is `orcl-std`'s. A program is written against an `orcl-std` version,
  and controlador states the `orcl-std` versions its interpreter runs. One number; a change to the
  library that leaves the language alone moves it too.
- (b) The language has a number of its own, ORCL 1, 2 and so on, as Rust has editions: stated by
  `orcl-std`, written in a program's compiled header, and named by controlador. The crates keep
  their SemVer from 2.0.0. Two numbers, each for its own reader.
- (c) Law L3 leaves the language's crates out, and they version from 1.0.0 as the language does. A
  change to a law and its clause (C-003).

The assistant recommends (b): a program left running for decades names the language it was written
in, which a library's version does not say.

**Taken meanwhile**: (a), which adds nothing to undo: the crates start at `2.0.0-alpha.1` and
nothing yet records a language version.

**Reversible?** Yes, until a compiled program records one.

**Decided** 2026-11-25, by the architect: asked "How is ORCL versioned as a language, beside its
crates (`orcl-std`, `orcl-runtime`, at 2.0.0-alpha.1 under law L3)? Taken meanwhile: the language's
version is `orcl-std`'s.", "A number of its own (Recommended)", (b): ORCL 1, 2 and so on, stated by
`orcl-std`, written in a compiled program's header and named by controlador; the crates keep their
SemVer from 2.0.0.

## Q223 - Controlador's execution model, as proposed

**Raised** 2026-11-22, by the controlador session, answering Q33 with a proposal.

`docs/controlador/execution-model.md` proposes, point by point, for the architect to rule:
1. **Where it is written:** that file, beside the plan; once ruled, the plan carries each decision
   in a line.
2. **What tasks share (K8):** only the process image. That is ORCL's O7, decided by the architect on
   2026-10-06 in Veredas 1 (`docs/orcl/language/ownership.md`), which the review read as an open
   reading. The controller applies forces, writes through its embedded portal and device events at a
   task's scan boundary, never inside its scan. Whether O7's publish at the end of a scan should
   give way to the prior art's LET (at the end of the logical period, the same values on every
   platform, up to one period later) stays open with it.
3. **Each task its own slice (K10):** yes; an output in one task's slice only, an input latched by
   every task that reads it, a slice copied whole.
4. **How one block interrupts another:** (a) a real-time thread per priority, the kernel
   pre-empting, or (b) one logic thread switching at instruction boundaries. The rule "existing,
   tested Linux methods before our own" (2026-11-16) speaks for (a); a hit halting every task and
   the replay of a field incident speak for (b). The assistant's pick: (b) for the first release.
5. **The fieldbus's own thread**, above the logic, never halted, sending halt or safe values, and
   the logic's watchdog at twice a scan's period.
6. **Time:** the program clock, paused at a halt; time-of-day blocks on the confirmed date alone; a
   start that fell during a halt dropped (K9); the main cycle held to the maximum cycle time,
   pre-emptions included.
7. **Start-up, handlers and stop:** start-up blocks before the first main cycle with no cyclic
   interrupt; a handler a task released by its event; a program error ending its block's scan, then
   the handler or the fault 502; the stop handler's one last scan.
8. **K11** put off by the first release's one cyclic task.

**Taken meanwhile**: Q33's: no code stands on an assumed model. The engine of a scan runs one cyclic
task, which every option runs alike.

**Reversible?** Yes: nothing is built on points 2 to 8.

**Decided** 2026-11-25, by the architect: asked "Controlador's execution model is proposed in
`docs/controlador/execution-model.md`: tasks share only the process image, each task its own slice
of it, a fieldbus thread above the logic that is never halted, a program clock paused at a halt, and
blocks interrupting each other on ONE logic thread at instruction boundaries (first release). Accept
it?", "Accept as proposed (Recommended)", against a real-time thread per priority for point 4 and
going through the points one by one. The eight points stand as proposed, point 4 as (b) for the
first release; what point 2 leaves open (O7's publish at the end of a scan, or LET) stays open. Q33
is answered with it.

## Q224 - Names for controlador's own files

**Raised** 2026-11-22, by the controlador session (`docs/controlador-plan.md`, "Before the code").

`docs/controlador/files.md` proposes, none of them confusable with maquinar's `veredas-controller`:
`veredas-aplicacao` (the application: tasks, cycle times, halt settings, the retained save, the IO
mapping, the Modbus addresses), `veredas-rede` (the hardware configuration), `veredas-alvo` (the
target's own settings, written at the target and never downloaded), `veredas-carga` (a download's
manifest, Q36); the compiled program a binary `<name>.orclb` with its own header; the commissioning
switch a file whose presence arms it, so a lost switch reads as off.

Options: those names; English ones (`veredas-application`, `veredas-network`, `veredas-target`,
`veredas-download`), against the suite's rule that names are Portuguese; or one prefix for all
(`veredas-controlador-*`), which brings back the near-clash with `veredas-controller`.

**Taken meanwhile**: the names proposed, in documents only; no file of any of them exists.

**Reversible?** Yes, until a file of the format exists anywhere.

**Decided** 2026-11-25, by the architect: asked "What are controlador's own file formats called?
Taken meanwhile, in documents only: `veredas-aplicacao` (application), `veredas-rede` (hardware
configuration), `veredas-alvo` (target settings), `veredas-carga` (a download's manifest), and
`<name>.orclb` for a compiled program.", "The Portuguese names", against English names and one
`veredas-controlador-*` prefix. What was taken stands.

## Q225 - May a `clippy.toml` sit in the repository?

**Raised** 2026-11-22, by the controlador session, carrying `orcl-std` in.

ORCL's O13 (no interior mutability) is checked by clippy's `disallowed_types`, whose list of types
every ORCL program crate carries in its `clippy.toml` (`docs/orcl/language/README.md`). Law L7 holds
every TOML file the repository holds, but a cargo manifest and the test runner's
`.config/nextest.toml`, to a `veredas-*` stamp. Clippy refuses a key it does not know: a
`clippy.toml` with `format` and `version` first stops the build with "unknown field name" (tried on
2026-11-22, with the clippy of Rust 1.100.0). So `orcl-std`'s `clippy.toml` cannot be carried as it
is.

Options:
- (a) L7 treats `clippy.toml` as a tool's file, as it treats nextest's. A change to the law and to
  the clause that states it (C-046, portal's), so the orchestrator carries it.
- (b) The repository holds no `clippy.toml`. O13 is kept by convention in the repository's own
  crates, or by a test that reads their sources; a program crate in a project, outside the
  repository, still carries the file.
- (c) Clippy's configuration is kept outside the repository and named through `CLIPPY_CONF_DIR`:
  every run depends on a setting nobody sees.

The assistant recommends (a).

**Taken meanwhile**: (b), without the test: `orcl-std` and the conveyor fixture carry their lints,
`disallowed_types` among them, with no list of types (DECISION PENDING in
`crates/orcl-std/src/lib.rs`).

**Reversible?** Yes: the file comes back as Veredas 1 had it.

**Decided** 2026-11-25, by the architect: asked "May a `clippy.toml` sit in the repository? ORCL's
rule against interior mutability is checked by a clippy list kept in that file, but law L7 demands a
`veredas-*` stamp in every TOML file and clippy refuses unknown keys. Taken meanwhile: no
`clippy.toml`, the rule kept by convention.", "L7 exempts it (Recommended)", (a): law L7 treats
`clippy.toml` as a tool's file, as it treats nextest's. The law and the clause that states it
(C-046) change together, in one commit.

## Q221 - Where does ORCL's front end build, beside a workspace on stable?

**Raised** 2026-11-22, by the controlador session, in the study of the interpreter's front end
(`docs/orcl/front-end.md`).

The front end is a `rustc_public` driver: it builds only on the pinned nightly, with `rustc-dev` and
`#![feature(rustc_private)]`, and fails on stable (E0463). The workspace builds on stable, and law
L1 holds every `orcl-*` crate to be a workspace member under `crates/`, never excluded.

Options:
- (a) `crates/orcl-front`, a member that builds on stable as an empty shell; its driver sits behind
  a cargo feature that only the pinned nightly turns on (`cargo +<nightly> build -p orcl-front
  --features driver`), so `cargo build --workspace` and the suite stay on stable, and the front
  end's own tests run under the nightly by a command of their own.
- (b) The whole workspace moves to the pinned nightly (`rust-toolchain.toml` at the root): one
  toolchain, every member's build and every recorded run tied to a nightly that moves.
- (c) A repository of its own, as Charon is; the interpreter here reads the program files it writes.
- (d) A crate under `tools/` with a name outside `orcl-*`, excluded from the workspace with its own
  toolchain file: L1 allows it, but it is part of the product, which Oficina ships, not repository
  tooling.

The assistant recommends (a).

**Taken meanwhile**: nothing of the front end is built.

**Reversible?** Yes: nothing is built.

**Decided** 2026-11-25, by the architect: asked "Where does ORCL's front end build? It is a compiler
driver that builds only on a pinned nightly Rust, while the workspace builds on stable. Taken
meanwhile: nothing of it is built.", "Member, driver by feature (Recommended)", (a):
`crates/orcl-front`, a member that builds on stable as an empty shell, its driver behind a cargo
feature only the pinned nightly turns on, its own tests run under the nightly by a command of their
own.

## Q222 - Which nightly does ORCL pin, and when does it move?

**Raised** 2026-11-22, with Q221.

`rustc_public` changes under one version number (six changes to MIR's enums in a year,
`docs/orcl/front-end.md`), and the driver links its nightly's own compiler library at run time. So
the pin decides what every sender's machine installs (293 MB, 1.5 GB installed), and each move means
adapting the driver and running the conformance suite again.

The assistant's proposal: pin `nightly-2026-11-21`, the one the study ran on (Kani pins the same);
write it in the front end's toolchain file and in every program file's header; move it on purpose,
at most a few times a year and never for a release that is being commissioned, each move a change of
its own with the conformance suite as its gate; a controller runs program files from the nightlies
its interpreter was proven against.

**Taken meanwhile**: `nightly-2026-11-21`, for studies only, installed in the home folder of the
development machine.

**Reversible?** Yes, until a program file records its nightly.

**Decided** 2026-11-25, by the architect: asked "Which nightly Rust does ORCL's front end pin, and
when does it move? Each move means adapting the driver and rerunning the conformance suite. Taken
meanwhile: `nightly-2026-11-21`, for studies only.", "As proposed (Recommended)":
`nightly-2026-11-21`, written in the front end's toolchain file and in every program file's header,
moved on purpose, a few times a year at most and never for a release being commissioned, each move a
change of its own gated by the conformance suite.

## Q226 - The Linux study's choices

**Raised** 2026-11-22, by the controlador session, in the study of Linux for real time and recovery
(`docs/controlador/linux-study.md`, owed by `docs/robustness.md` and Q42).

The study found a tested method for every recovery mechanism the round asked about, corrected the
list it started from (no `/sys/kernel/realtime` in mainline; systemd-timesyncd does not cross 2036;
systemd rewinds a clock more than 15 years past its epoch) and leaves these choices, each with the
assistant's recommendation:
- **The kernel line**: kernel.org's long-term kernels (about four years) or the Civil Infrastructure
  Platform's (ten years, 6.12-cip to 2035), and how a controller moves to the next line.
  Recommended: CIP's.
- **The update framework with fallback**: RAUC, SWUpdate with EFI Boot Guard, Mender,
  systemd-sysupdate (UEFI only) or OSTree with greenboot; and what "booted well" means for a
  controller. Recommended: SWUpdate with EFI Boot Guard, CIP's own reference, if CIP's kernel is
  chosen; "booted well" meaning the controller's process up with its program loaded and its first
  cycle within time.
- **The system partition**: read-only under dm-verity, with correction or without, and what
  uncorrectable damage does. Recommended: with correction (2 parity bytes, about 0.8 %),
  uncorrectable damage restarting into the other slot, the count of corrected blocks a maintenance
  request.
- **The program's two copies**: Btrfs RAID1 (or DUP on one drive), a ZFS mirror (outside the kernel)
  or dm-integrity under an LVM RAID1; the scrub's period and how its result becomes a maintenance
  request. Recommended: Btrfs, in the kernel and repairing on read as well as on scrub.
- **Time**: the NTP client (chrony or ntpd, not systemd-timesyncd); clock chips whose drivers answer
  `RTC_VL_READ`, which an x86 PC's own clock does not; how systemd's 15-year rewind is switched off
  in the system image. Recommended: chrony, a clock chip that reports, the rewind built out.
- **The watchdog chain**: firmware or bootloader, kernel, PID 1, and the controller's own process
  owning the last ping. Recommended: that chain, ending in the controller.
- **Logs**: volatile, with pstore keeping a crash's record. Recommended.
- **What a target must have**: ECC memory, a drive that reports its wear (eMMC or NVMe, not SD),
  memory that keeps its content where retained data is saved every cycle, a power-fail line where
  there is hold-up power. Recommended as the target's hardware description, read by Q40's "who says
  what a target can do".

**Taken meanwhile**: nothing is built on any of them. When controlador's check of real-time
scheduling is built, it reads what mainline offers: the policy and priority the process got, its
locked memory and `PREEMPT_RT` in the kernel's version string, never `/sys/kernel/realtime`.

**Reversible?** Yes: nothing is built.

**Decided** 2026-11-25, by the architect: asked "The Linux study for the controller recommends: the
Civil Infrastructure Platform's 10-year kernel; SWUpdate with EFI Boot Guard for updates with
fallback; a read-only system under dm-verity with correction; Btrfs for the program's two copies;
chrony; a watchdog chain ending in the controller; volatile logs with pstore; ECC memory and a
wear-reporting drive. Accept them?", "Accept all (Recommended)", against going through them one by
one and "Not yet". The eight recommendations above are the plan's choices.

## Q227 - The list of core-library operations ORCL allows: a closed list, or core's MIR interpreted?

**Raised** 2026-11-22, by the controlador session, in the study of the interpreter's front end (the
briefing's J2).

The programs studied call into core at 27 distinct functions; core's own bodies use what ORCL never
does (`isize`, `usize`, raw pointers, `Transmute`, closures, the formatting machinery of panics),
and two leaves have no body in any build (`panic_impl`, `caller_location`).

Options:
- (a) A closed list, the cut at core: each allowed call from the program's crates into core is an
  operation of the interpreter, implemented natively and proven against the native call by the
  conformance suite; a call outside the list is refused when the program is lowered, by name and
  place. The list grows one operation at a time. Eurydice does this.
- (b) Core's MIR interpreted, as Miri does: a value model as wide as Rust's own (pointers, `usize`,
  niches), a rebuilt standard library, the native list kept for the leaves anyway.

The assistant recommends (a), with the list in `docs/orcl/front-end.md` as its start: what keeps
ORCL's types and reaches no panic entry point by design. The refusal at lowering is what the
interpreter cannot run, not a check of the program's structure, which the ruling of 2026-11-16
leaves out.

**Taken meanwhile**: nothing is built.

**Reversible?** Yes: nothing is built.

**Decided** 2026-11-25, by the architect: asked "How does ORCL's interpreter handle a program's
calls into Rust's core library (27 distinct functions in the programs studied)? Taken meanwhile:
nothing is built.", "A closed list (Recommended)", (a): each allowed call into core is an operation
of the interpreter, proven against the native call by the conformance suite, and a call outside the
list is refused by name and place when the program is lowered.

## Q228 - Controlador's states: the readings its first clause takes

**Raised** 2026-11-22, by the controlador session, building the controller's states over one cyclic
task (C-152).

`docs/controlador-plan.md` decides the states, the commands, their gate and the return after a power
cycle. Where it is silent the first build takes a reading, each marked `DECISION PENDING (Q228)` in
`crates/veredas-controlador/src/controller.rs`:
1. **The outputs in STOP** take the program's safe values, and the plant's exchange goes on, as the
   plan rules for FAULT. The other reading: outputs held at their last values in STOP.
2. **A controller that never ran** comes up in STOP with no reason. The other reading: `STOP
   power_on(was="STOP")`.
3. **A start-up a power cut interrupted** returns through a warm start, as RUN does, its aim having
   been RUN: a flicker of the supply during a warm start would otherwise leave a plant stopped until
   a person comes. The other reading: STOP `power_on(was="STARTUP")`.
4. **Warm and cold starts** both start the program from its initial values, since ORCL marks no
   variable as retained yet; how a program marks one is a language decision still to come.
5. **The start-up's phases** (load, restore, handler, devices), with nothing to restore, no handler
   and no device, pass in the cycle after the command, and the program's first scan runs in that
   cycle.
6. **The condition `overruns`** counts the time errors since the last start; the maintenance request
   `overruns` waits for Q39.
7. **A command that would change nothing** (a start outside STOP, a stop in STOP or FAULT, an
   acknowledge outside FAULT) is refused, naming the state, and changes nothing.
8. **A runtime that restarted while the machine did not** comes up in `FAULT internal`, the
   briefing's reading K7 (Q45).
9. **A stop from STARTUP** is allowed, as from RUN.

**Taken meanwhile**: the readings above, as readings.

**Reversible?** Yes: each is a line of the controller and an assertion of its test.

**Decided** 2026-11-25, by the architect: asked "Controlador's first build took nine readings where
the plan is silent: outputs take safe values in STOP; a controller that never ran comes up in STOP;
an interrupted start-up returns through a warm start; no retained variables yet; a command that
would change nothing is refused; a runtime restarted alone comes up in FAULT internal; a stop from
STARTUP is allowed. Accept them?", "Accept all nine (Recommended)", against going through them one
by one. The nine readings stand as taken.

---

## Q102 - What "fork a young Rust crate" means for the EtherNet/IP scanner

**Raised** 2026-11-22, building the EtherNet/IP scanner (P-018, C-072; the briefing: "EtherNet/IP
scanner (fork a young Rust crate, test on real devices)"). The review's entry 27 on the fork is
not ruled (Q31).

The young crates are `ethernetip-core` and `ethernetip-connections` 0.1.1 (Apache-2.0), the ones
the bench's adapter test already drives it with: encapsulation, Common Packet Format, the
Connection Manager's requests, Class 1 framing, and a scanner of their own. Read before choosing:
their codecs check every length before they read; their scanner does not fit a controller (no
bound on connecting or on an answer, the 16-bit sequence count left out of a connection's sizes,
the Sockaddr Info items numbered the other way round from the standard, no watch on a silent
adapter). Law L4 keeps any workspace member out of core's tables, so a fork cannot be a member core
uses.

Options:
- (a) Core depends on the two crates, pinned exactly, for their codecs, and writes the scanner
  itself; a codec that needs changing is then vendored into core with its licence notice.
- (b) The two crates are vendored into core now, trimmed to what the scanner uses, and fixed in
  place.
- (c) A fork published under the architect's own account, as a git dependency.

**Recommended**: (a). **Taken meanwhile**: (a): `fieldbus::enip` uses the codecs and none of the
crates' constants for the Sockaddr items. A licence check of the dependencies (Q35) has to accept
Apache-2.0.

**Reversible?** Yes: (b) is the same code moved in; (c) needs the architect's account.

**Decided** 2026-11-25, by the architect: asked "What does \"fork a young Rust crate\" mean for
core's EtherNet/IP scanner? The young crates' codecs are sound, their scanner does not fit a
controller. Taken meanwhile: depend on the crates for their codecs, write the scanner in core.",
"Depend, own scanner (Recommended)", (a), against vendoring the crates into core now and a fork on
the architect's account. What was taken stands; the review's entry 27 is answered with it.

## Q160 - Which physics engine `fisica` stands on

**Raised** 2026-11-22, starting `fisica` (P-020; `docs/maquinar-plan.md`, "Decided in the briefing
of November 2026, not built"). The architect allowed a physics engine as a new dependency; which
one is this question.

What `fisica` has to do: bodies in a plant's scene (a carton on a belt, a part pushed off it, a
part dropped into a chute), their motion at a fixed step, headless, the same run for the same
inputs, and which sensor an object reached (a light barrier's beam, a proximity switch's field).
Oficina's 3D view only draws the positions it is sent.

Options:
- (a) Rapier (`rapier3d`, Dimforge, Apache-2.0): rigid bodies, colliders, sensors reported as
  intersection events, a pipeline stepped by the caller at a fixed step, determinism across
  machines with its `enhanced-determinism` feature; pure Rust, no engine or ECS around it.
- (b) Avian (`avian3d`): an ECS physics engine built on Bevy; it brings Bevy's ECS and app model
  into the bench, which has neither.
- (c) Parry (`parry3d`, Rapier's geometry half) with `fisica`'s own kinematics: shapes and sensor
  queries without dynamics, conveyors moving parts as kinematic bodies along a path. Smaller, but
  no contact is resolved (a part pushed off a belt, a stack, a chute) unless `fisica` writes it.
- (d) No dependency: `fisica`'s own one-dimensional motion along conveyor paths, sensors as
  points on them. Enough for a conveyor line, not for a scene.

**Recommended**: (a), behind `fisica`'s own types (a body, a sensor, a step), so the engine is
named in one module and a change of engine touches only that module. **Taken meanwhile**: (a),
pinned to one version with `enhanced-determinism` on.

**Reversible?** Yes: one dependency and one module; no file format names the engine.

**Decided** 2026-11-25, by the architect: asked "Which physics engine does the bench's `fisica`
(cartons on belts, parts in chutes, sensors reached) stand on? Taken meanwhile: Rapier, pinned, with
its determinism feature.", "Rapier (Recommended)", (a), against Avian, Parry with `fisica`'s own
kinematics and no dependency. What was taken stands. Since C-114's fourth refute round
(2026-11-25) the bench uses Rapier's double-precision build of the same version, `rapier3d-f64`
(in single precision a slow belt far from the origin carried nothing): the same engine.

---

## Q441 - What code does a component's version name in a release?

**Raised** 2026-11-23, by the orchestrator's implications check on C-290.

A release pins each component at a version, and "one version, one tree" (C-290) refuses two
releases that pin one version at different trees of the component's folder. What the executable
under that version is built from is wider: the members it uses (core, at their own versions), the
lock file (every external crate's exact version), the workspace's manifest (profiles, shared
dependency versions) and the toolchain the manifest states. Portal could keep its version and its
folder while core's code or `Cargo.lock` changes, and a second release would ship another
executable under the same version.

Options:
- (a) The folder alone (taken meanwhile): each member is judged by its own version; the lock file
  and the workspace's manifest are not. A dependency's change ships under the same version.
- (b) The folder and every member it uses, transitively: a core change without a portal bump is
  refused for portal too. The lock file still free.
- (c) Everything the build reads but the toolchain: the folder, the members it uses, the lock
  file's entries for its dependency closure and the workspace's manifest. Any change to what goes
  into an executable bumps its version; a lock file update bumps every component it reaches.

**Recommended**: (c), so a version names one executable's inputs, as testers and the robustness
rulings (a download's identity) expect; (a) meanwhile, stated as a limit in C-290.

**Reversible?** Yes: the check widens without changing the manifest's format.

**Decided** 2026-11-25, by the architect: asked "In a release, what code does a component's version
name? A release refuses one version pinned at two different trees; today only the component's own
folder is judged (taken meanwhile).", "All build inputs (Recommended)", (c): the folder, the members
it uses, the lock file's entries for its dependency closure and the workspace's manifest, everything
the build reads but the toolchain.

---

## Q442 - Does any edit of the workspace's manifest bump every component?

**Raised** 2026-11-25, by the release member, implementing Q441's ruling ("All build inputs").

The workspace's manifest (`Cargo.toml` at the root) is one of the inputs Q441 names. Read whole, any
edit of it charges every component: a member added to `members`, a comment, a workspace dependency
one member alone uses. Members edit it often (each new member, each dependency's comment), so every
such edit would bump every component's version at the next release.

Options:
- (a) The whole file as git stores it (taken meanwhile): any byte changed bumps every component.
- (b) What cargo reads of it to build that component: the resolver, the profiles, `[patch]` and
  `[replace]`, and the `[workspace.package]`, `[workspace.dependencies]` and `[workspace.lints]`
  entries the component and the members it uses inherit; `members`, `exclude`, comments and the
  entries other members alone inherit left out.

**Recommended**: (b): a version names what goes into its executables and nothing else (features no
longer unify across components: the cut builds each alone). Under (a), every small commit to the
root manifest (14 since 2.0.0-alpha.1) charges every component, and only each member may bump its
own version: every cut then needs all members to bump in step, a shared cadence the plan's
"Independent versions" set out to end.

**Taken meanwhile** (most reversible): (a), the stricter reading: a narrower one later lets through
every set of releases the whole file let through.

**Reversible?** Yes: the check narrows without changing the manifest's format.

**Decided** 2026-11-26, by the architect: asked "The workspace's root `Cargo.toml` is one of them,
and members edit it often. Does any edit of it bump every component? Taken meanwhile: yes, the whole
file.", "Only what cargo reads for it (Recommended)", (b): the resolver, the profiles, `[patch]` and
`[replace]`, and the workspace entries the component and the members it uses inherit; the member
list, comments and other members' entries left out.

---

## Q443 - Does a release ship an executable its component's default features do not build?

**Raised** 2026-11-25, by the release member: Oficina's one executable, `veredas-oficina`, needs its
`shell` feature (Qt 6.12 and KDDockWidgets), which its default features leave off so that the
suite's build needs no Qt (Q370). The cut builds every component with its default features and
refused a release that would ship less than the manifest names; since Oficina landed, every cut of
HEAD would stop at the builder, "cargo built no `veredas-oficina`".

Options:
- (a) A release ships what the default features build (taken meanwhile): cargo's own build with
  them decides, at the cut (alforge models no feature), so Oficina is pinned at its version with no
  executable and the cut says so.
- (b) No release until Oficina's window builds by default (the suite's build needing Qt, against
  Q370).
- (c) The builder builds each component with the features its executables need, and the tester
  machine carries their libraries: Oficina's window in the machine, for designers. Debian 13 holds
  Qt 6.8, so Qt 6.12 and KDDockWidgets would be built for the builder and shipped with the release.

**Recommended**: (a) now; (c) once Oficina's window is something a tester or a designer uses.

**Taken meanwhile** (most reversible): (a): a later release ships the window by naming it, and
nothing shipped is taken back.

**Reversible?** Yes: the manifest's `bins` say what each release ships.

**Decided** 2026-11-26, by the architect: asked "Does a release ship an executable that its
component's default features do not build? It arose because Oficina's window sat behind a feature;
you have since ruled the window built by default (Q370), so today the rule would bind no component.
Taken meanwhile: a release ships what the default features build.", "With the features its
executables need", (c), against the recommendation (a): the builder turns on, per component, the
features its executables need, whatever its defaults are.

---

## Q444 - Is a release's component built alone, as its version names it, or with the others, as the suite tests it?

**Raised** 2026-11-25, by the release member, from the implications check on C-290 after Q441.

One `cargo build` of several packages unifies their features: built with the others, a component's
executable carries features another member turns on (today the shipped portal carried core's
`demo-server`, which the bench turns on), so another member's manifest reaches it and its version
would not name it (Q441). The cut now builds each component alone. The suite (`cargo nextest run
--workspace`) still tests them built together, with features unified across every member and the
tests' own dependencies: at HEAD, portal and supervisão built alone lack features in 9 packages
against the workspace's build, the bench in 17. Only the bench and supervisão run again from the
bundle as built.

Options:
- (a) Built alone (taken meanwhile): a version names its executable's inputs; what ships differs
  from what the suite built, the run together the one check of what ships.
- (b) Built together, as the suite builds: what ships is what was tested, and every member's
  manifest becomes an input of every component's version (any manifest edit bumps all).
- (c) Built alone, and the cut also runs each component's own tests built alone (`cargo nextest run
  -p <component>`), so what ships is tested as it is built; a longer cut.

**Recommended**: (c).

**Taken meanwhile** (most reversible): (a): nothing is published, and the next cut may build either
way.

**Reversible?** Yes: the builder's script.

**Decided** 2026-11-26, by the architect: asked "Is a release's component built alone, as its
version names it, or together with the others, as the suite tests it? Built together, cargo unifies
features, so another member's manifest reaches the executable (the shipped portal carried a feature
the bench turns on). Taken meanwhile: built alone.", "Alone, and tested alone (Recommended)", (c):
the cut builds each component alone and also runs each component's own tests built alone.

---

## Q445 - Does a test-only change in a component's folder bump its version?

**Raised** 2026-11-25, by the release member, from the same check.

Q441 named the component's folder whole: an edit of its `tests/`, `benches/` or `examples/` charges
it and every component using it, though nothing of it goes into an executable (refute rounds edit
core's tests daily). Test-only dependencies already charge nothing.

Options:
- (a) The folder whole (taken meanwhile).
- (b) The folder but its top `tests/`, `benches/` and `examples/` (and what only they read), the
  cut's dep-info check still holding what the build read.

**Recommended**: (b), a version naming what goes into an executable.

**Taken meanwhile** (most reversible): (a), the stricter reading.

**Reversible?** Yes: the check narrows without changing the manifest's format.

**Decided** 2026-11-26, by the architect: asked "Does a test-only change in a component's folder
(its `tests/`, `benches/`, `examples/`) bump its version, though nothing of it goes into an
executable? Refute rounds edit tests daily. Taken meanwhile: yes, the folder whole.", "Tests left
out (Recommended)", (b).

## Q446 - Does a bundle that holds Oficina's window carry Qt 6.12's libraries, or does the tester machine provide them?

**Raised** 2026-11-26, by the release member, from Q370's ruling ("Built by default").

Once Oficina's window is built by default, a release ships it, and the executable loads Qt 6.12 and
KDDockWidgets at run time. Debian 13, the tester machine's distribution (Q433), offers Qt 6.8, so the
machine cannot take Qt 6.12 from its packages. Both libraries as the development machine holds them
run on Debian 13: Qt is qt.io's build (gcc 11.5, GLIBCXX 3.4.29), and KDDockWidgets needs
GLIBCXX 3.4.32, under Debian 13's 3.4.33. What Oficina loads is about 120 MB extracted, about 45 MB
compressed: Qt's core libraries (52 MB), ICU (38 MB), the platform plugin and QML modules (about
20 MB), and KDDockWidgets (7 MB). A release extracts to 160 MB today, and the machine's disk is
16 GB. Either way the builder takes Qt from the development machine's `~/.local/opt/Qt/6.12.0`
(2.4 GB, cached once per Qt version) to build the window; that part is the release member's.

Options:
- (a) The bundle carries what Oficina's window loads, in `lib/` beside `bin/` (the executable finds it
  by its `RUNPATH`, `$ORIGIN/../lib`), with Qt's and KDDockWidgets' licence texts (both LGPLv3 here)
  and where their sources are. Each release carries its own Qt; an update brings a new Qt with it.
- (b) The tester machine provides it: the provisioning installs Qt 6.12 and KDDockWidgets under
  `/opt/qt` from the development machine's copy. The bundle stays small, but a release built on
  another Qt needs a new OVA.
- (c) A release ships no Oficina window for now: the cut builds and judges it, and the manifest
  says it is not shipped, as a bin with required features is today (Q443).

**Recommended**: (a): a release stays whole in itself, an update keeps working across a Qt change,
and the machine grows by about 120 MB per release kept.

**Taken meanwhile** (most reversible): (c) until the switch lands, then (a) as the plan states it,
nothing shipped before a ruling: no cut is due until alpha.2.

**Reversible?** Yes: where the libraries live is the bundle's layout and the provisioning, never a
component's code.

**Decided** 2026-11-26, by the architect: asked "Once Oficina's window is built by default a release
ships it, and it loads Qt 6.12 and KDDockWidgets, which Debian 13 (the tester machine) does not
offer. Where do those libraries come from? Taken meanwhile: no window shipped until the switch
lands.", "The bundle carries them (Recommended)", (a): in `lib/` beside `bin/`, with Qt's and
KDDockWidgets' licence texts and where their sources are.

## Q447 - Which reporting channel does the security file name?

**Raised** 2026-11-26, by the release member, from Q31 entry 38, ruled "(b) Security file,
component list".

Before the first public release, the repository holds a security file (`SECURITY.md`) saying how to
report a vulnerability, and each release carries a component list. The component list is the
release member's (the cut produces it, `docs/alforge-plan.md`). The channel is a person's or an
account's, so it is the architect's to name.

Options:
- (a) GitHub's private vulnerability reporting on the repository (a report stays private between the
  reporter and the repository's owners; nothing is published by the report itself).
- (b) An e-mail address the architect names, published in the file.
- (c) Both: GitHub's private reporting first, an address beside it.

**Recommended**: (a): it needs no published address and keeps reports beside the code.

**Taken meanwhile** (most reversible): no security file yet; nothing is public that asks for one
until the first public release (Q436, "Nowhere yet").

**Reversible?** Yes: a file of a few lines.

**Decided** 2026-11-26, by the architect: asked "Which channel does `SECURITY.md` name for reporting
a vulnerability? An address in it is published.", "GitHub's private reporting (Recommended)", (a).
No address is published; turning private reporting on in the repository's settings is the
architect's act.

## Q448 - Does the cut hold every file a build opens to what its version names, or what rustc reads?

**Raised** 2026-11-26, by the release member, from C-291's refute round 21.

A version names everything its build reads (Q441). The cut proves it with cargo's dep-info, which
holds what rustc read and what a build script declared (`rerun-if-changed`). Three ordinary forms
read files the dep-info never names:
- a build script reading a shared data file without declaring it (code generated from it);
- a header the C compiler a build script runs includes (`cc` with a shared include folder);
- a file a proc-macro by path reads while it expands.

On a real workspace the critic built, changing only such files changed two executables under one
version, with `alforge check` and the cut both passing. No shipped component does this today.

Options:
- (a) A stated limit: the cut judges what rustc reads and a build script declares (taken
  meanwhile).
- (b) Each component built from a tree holding only what its version names, so any other read
  fails the build. Cargo needs every member's manifest and targets, so the other members are
  stubbed. Each component's build compiles its own dependencies, and a real cut takes several
  times longer (today about 15 minutes).
- (c) Each build traced in the builder (`strace -f` on the file calls), and every path opened
  under the tree judged against the version's inputs. A cut takes longer by the tracing alone,
  and the builder disk gains `strace`.

**Recommended**: (c): it judges exactly what the build opened, at a moderate cost, and it reaches
what (b) does without stubbing the workspace.

**Taken meanwhile** (most reversible): (a), the limit stated in C-290 and C-291 and shown by a test
built with the suite's own cargo.

**Reversible?** Yes: (b) and (c) add to the cut's judgement and change no manifest.

**Decided** 2026-11-26, by the architect: asked "A version names everything its build reads (ruled),
and the release cut proves it from what the compiler reports. Three ordinary forms escape that
report (a build script reading an undeclared file, a C header, a file a macro reads); no shipped
component does this today. How far does the cut go? Taken meanwhile: a stated limit.", "Trace each
build (Recommended)", (c): each build traced in the builder and every path opened under the tree
judged against the version's inputs.

---

## Q229 - A pulse that comes back with no new edge, and which preset a timer is judged against

**Raised** 2026-11-23, by the controlador session, from C-150's refute rounds 11 and 12.

`orcl-std`'s timers, carried from Veredas 1, judge their elapsed time on every scan against the `pt`
that scan gives, as the counter judges its count against each scan's preset. While a timer measures,
its elapsed time is the scan's time less its start. The pulse `Tp` goes on measuring after its pulse
has ended for as long as its input stays on. So, with the input still on, a `pt` raised past the
elapsed time turns the pulse on again with no new rising edge. So does a scan whose time goes back,
which none of the runtime's clocks gives. IEC 61131-3's TP gives one pulse per rising edge.
- (a) Keep the timers as carried: each scan's `pt`, and a pulse that can come back.
- (b) Each scan's `pt`, but a pulse that has ended stays off until the next rising edge, whatever
  `pt` or the time does. This changes the carried `Tp`.
- (c) Every timer judged against the `pt` it started with, its switching time fixed at its start.
  This changes all three timers.

**Taken meanwhile**: (a), as carried. C-150 states it, and `crates/orcl-std/src/timers.rs` marks it
`DECISION PENDING (Q229)`.

**Reversible?** Yes: a few lines of `Tp`, or of all three timers, and the assertions that pin them.

**Decided** 2026-11-25, by the architect: asked "ORCL's timers judge elapsed time against the preset
each scan gives. So a pulse timer (TP) whose input stays on can fire again with no new rising edge
when its preset is raised; IEC 61131-3's TP gives one pulse per rising edge. Taken meanwhile: kept
as carried from Veredas 1.", "Pulse needs a new edge", (b): each scan's `pt` still counts, and a
pulse that has ended stays off until the next rising edge, whatever `pt` or the time does. The
carried `Tp` changes; the other two timers stay.

---

## Q230 - A program whose functions call themselves

**Raised** 2026-11-25, by the controlador session, designing the interpreter
(`docs/orcl/interpreter.md`).

The interpreter lays a program's memory out when it loads it, so that nothing is allocated during a
scan (`docs/orcl/README.md`, "The interpreter"): it walks the calls from the program's entries and
sizes its stack by the deepest path. A function that reaches itself, directly or through others, has
no deepest path. Rust compiles such a program, the ruling of 2026-11-16 ("no further checks
necessary") leaves the program unchecked, and ORCL's rules say nothing of recursion yet. Whether
every translator's target can represent it is not studied.

Options:
- (a) The interpreter refuses such a program when it loads it, naming the cycle of calls, as it
  refuses a call into core outside its list: what it cannot lay out. The language's own principle,
  "start strict, since loosening a rule breaks no program and tightening one can"
  (`docs/orcl/language/ownership.md`, OQ2), speaks for it.
- (b) It runs on a stack of a size the target's settings give, laid out at load, and a call past it
  is the fault 502 `program(at)`: every program Rust compiles runs, and a deep one faults in
  production.
- (c) The language forbids recursion as a rule of its own, which the translators can rely on, and the
  interpreter refuses as in (a).

The assistant recommends (a) now, and (c) when the language's rules are next reviewed.

**Taken meanwhile**: (a). No program file exists yet.

**Reversible?** Yes: loosening to (b) breaks no program.

**Decided** 2026-11-26, by the architect: asked "ORCL's interpreter sizes a program's stack at load,
by the deepest path of calls, so nothing is allocated during a scan. A function that calls itself
(directly or through others) has no deepest path. What happens to such a program? Taken meanwhile:
refused at load.", "Runs on a fixed stack", (b), against the recommendation (a): it runs on a stack
of a size the target's settings give, laid out at load, and a call past it is the fault 502
`program(at)`. What was taken meanwhile (refusing such a program at load) gives way.

---

## Q231 - Where an interpreted program's first state and safe outputs come from

**Raised** 2026-11-25, by the controlador session, putting an interpreted program on the engine of a
scan (`docs/orcl/interpreter.md`, "Running").

A native program starts from its `Default` (controlador makes it so) and the engine asks it for its
safe outputs on the scan that faults and on every faulted scan; `safe_outputs(&self)` may read the
program's state, as the engine's own model test makes it do. An interpreted program's state is a
region of the machine, laid out at load and unwritten until something writes it, and its safe
outputs are a value of its outputs' type. Who gives the two?
- (a) The host hands both at load, as slots: the front end computes them when it lowers the program
  (calling `Default::default` and `safe_outputs` on the native crate on the nightly) and writes them
  into the program file. Nothing of the program runs after a fault, but safe outputs that depend on
  the state are those of the first state, so the front end refuses a program whose `safe_outputs`
  reads its state, or the conformance suite finds it.
- (b) The front end lowers `Default::default` and `safe_outputs` as entries of their own: the machine
  calls the first at load and the second on the faulting scan and every faulted one, its state as
  the fault left it (the failed call aborted first). Natively faithful, but program code runs on a
  faulted machine, and a `safe_outputs` that fails needs an answer of its own (all-default outputs).
- (c) As (b) for the first state, and safe outputs computed once at load.

The assistant recommends (a): the safe state is what a fault falls back on, so it is best fixed
before anything runs, and checked when the program is built; ORCL may state that safe outputs are a
constant.

**Taken meanwhile**: (a), without the file's part: `orcl_runtime::Interpreted` is handed the first
state as slots and the safe outputs as a value when it is made (DECISION PENDING in
`crates/orcl-runtime/src/interpreted.rs`), and the controller's `Loaded` (C-152) takes both from its
host, the program made afresh from them at every start (DECISION PENDING in
`crates/veredas-controlador/src/controller.rs`). The engine's `TaskProgram::safe_outputs` takes `&mut self`,
so (b) needs no change to the engine.

Both (a) and (b) need a program file of a format past 1, which holds neither the values nor a table
of entries (C-154: such a table comes with a new format version). And (b)'s "the failed call aborted
first" throws away the machine's position at the failure, which the fault record keeps
(`docs/orcl/interpreter.md`, "Running"): the position would have to be copied out before.

**Reversible?** Yes: the host's hand-over is one constructor; (b) or (c) adds entries the adapter
calls, and a format version to the file.

**Decided** 2026-11-26, by the architect: asked "Where do an interpreted ORCL program's first state
and its safe outputs come from? Natively the program computes both (`Default`, `safe_outputs`).
Taken meanwhile: the host hands both at load.", "Computed at build, in the file (Recommended)", (a):
the front end computes both when it lowers the program and writes them into the program file, and a
program whose `safe_outputs` reads its state is refused; against lowering them as entries the
machine runs.

---

## Q232 - An instruction budget per scan for an interpreted program

**Raised** 2026-11-25, by the controlador session, putting an interpreted program on the engine of a
scan (`docs/orcl/interpreter.md`, "Running").

The engine measures a scan when it ends; a scan that never ends is the host's watchdog's to catch
(`docs/controlador/execution-model.md`, point 6: the bus thread faults at twice the period and sends
the safe values). An interpreted scan runs one instruction at a time, so the engine could stop it
itself after a number of instructions, and the controller could then acknowledge and restart it on
the same thread, which a scan stuck in a native loop does not allow. Options:
- (a) No budget: the entry is called to its return, and a scan that never returns waits for the
  host's watchdog, as a native one does.
- (b) A budget per scan, set with the task (a number of instructions, or the instructions a target
  runs in twice the period, measured): past it the scan ends as the watchdog's fault, and the
  machine stands at the instruction it reached, for the fault record.
- (c) No budget, but the logic thread switches at instruction boundaries (Q223 with Q33): the bus
  thread's watchdog marks the task faulted and the logic thread stops it at its next instruction.

The assistant recommends (c) once several tasks are built, with (b) as its stand-in for one task: it
is what makes a stuck interpreted scan recoverable without killing the thread.

**Taken meanwhile**: (a). `orcl_runtime::Interpreted` calls its entry to its return (C-155 says so).

**Reversible?** Yes: a budget is a field of the adapter, read by one loop.

**Decided** 2026-11-26, by the architect: asked "An interpreted scan runs one instruction at a time,
so the engine could stop a scan that never ends by itself, where a native one waits for the
watchdog. Does a scan get an instruction budget? Taken meanwhile: none.", "No budget", (a), against
the recommendation ((b) now, (c) later): the entry runs to its return, and a scan that never returns
waits for the host's watchdog, as a native one does. What was taken stands.

---

## Q233 - The download a FAULT 506 `load` names before there are downloads

**Raised** 2026-11-25, by the controlador session, making the controller's program at power-on and at
every start (C-152).

Q36 rules that a stored program failing its check before a load (a swap, a restart, a power-on) is
the FAULT 506 `load(download)`, never a substitute, and the plan keeps the last 10 downloads, each
with its identity, the SHA-256 of its manifest. Downloads are not built yet: the controller's program
is handed in by its host (Q231), and one that cannot be made (the machine refuses it, a first state of
another type) now leaves the controller in FAULT 506 too. Core's `FaultReason::Load` carries the
download as a `u32`, so it needs a number when there is no download to name.
- (a) 0, read as "no download": the state reads `FAULT load(id=506, download=0)`; the first download
  is 1.
- (b) The number of the download the program came from, which a program handed in by its host does
  not have until downloads land: until then the controller comes up in STOP 104 `no_program`
  instead, which the plan has for a controller with no program.
- (c) A reason of its own for a program handed in that cannot be made, which core's list would gain.

The assistant recommends (a): 506 is the fault the ruling names for a program that does not load, and
the download's number joins it when downloads land; whether `download` is a counter or the identity's
short form is the download's own question then.

**Taken meanwhile**: (a). `NO_DOWNLOAD` in `crates/veredas-controlador/src/controller.rs` (DECISION
PENDING); the adapter's reason is kept for the host (`Controller::load_error`).

**Reversible?** Yes: one constant and the text the state carries.

**Decided** 2026-11-26, by the architect: asked "A stored program that fails its check is the FAULT
506 `load(download)` (ruled). Downloads are not built yet, and a program handed in by its host that
cannot be made also ends in FAULT 506. Which download does the fault name then? Taken meanwhile:
0.", "STOP no_program instead", (b), against the recommendation (a): until downloads exist, a
controller whose handed-in program cannot be made comes up in STOP 104 `no_program`. What was taken
meanwhile (`download=0`) gives way.

---

## Q234 - Running unattended: memory errors and slow leaks

**Raised** 2026-11-26, by the controlador session, proposing the first of the four areas Q42 left
unopened (`docs/robustness.md`, area 6).

What is ruled covers a scan that runs late (the cycle-time rule), the watchdog chain, and the
hardware a target must have, ECC memory among it (Q226). What a controller left for years meets
besides: memory errors, which EDAC reports, corrected or not
(https://docs.kernel.org/admin-guide/RAS/main.html); and slow leaks, a process whose memory, open
files or threads grow by a little a day until something gives. The scan itself allocates nothing
once a program is loaded (C-153, C-155), but the embedded portal, its store, the logs and the
operating system do.
- (a) Count and say so. A corrected memory error is counted, and a rate past a threshold raises a
  maintenance request; an uncorrected one the kernel reports in the controller's memory is the
  runtime's failure, FAULT 507, since the program's data can no longer be trusted. The controller
  reads its own resident memory, open files and threads against limits set when it starts: a
  rising trend over days raises a maintenance request, a limit reached is FAULT 505 `resource`.
- (b) As (a), and the non-real-time parts (the embedded portal) restart on a schedule the engineer
  sets, the logic never.
- (c) Nothing past the hardware watchdog: a leak ends as a reboot through the watchdog chain.

The assistant recommends (a): it says what will fail before it fails, where (c) turns a slow leak
into an unplanned stop and (b) restarts parts of a running plant on a calendar.

**Taken meanwhile**: nothing; the area is not built.

**Reversible?** Yes until built: each is a reading of the controller's own counters.

**Decided** 2026-11-26, by the architect: asked "Running unattended for years: beyond the cycle-time
rule, the watchdog chain and ECC memory (ruled), what does a controller do about memory errors and
slow leaks (memory, open files, threads growing a little a day)?", "Count and say so (Recommended)",
(a), against restarting the non-real-time parts on a schedule and the watchdog alone.

---

## Q235 - An old controller and a new Oficina: versions, migration, a rescue path

**Raised** 2026-11-26, by the controlador session, proposing the second of the four areas Q42 left
unopened (`docs/robustness.md`, area 7).

A controller is left as the engineer left it, for years, while Oficina moves on. What versions
there are: the controller's release, the ORCL language versions it runs (`orcl_interp::LANGUAGES`,
C-154), the program file's format (C-154) and each TOML file's `format` and `version` (law L7,
portal migrating or refusing).
- (a) Oficina speaks to every controller back to a floor it states (a number of releases or of
  years): it reads the controller's versions when it connects and writes only what that controller
  reads, a program for its ORCL version and files in its formats.
- (b) The controller first: Oficina refuses to download to a controller older than its own release
  and offers the controller's update (with its own rollback) before the program.
- (c) A rescue path always, whatever the versions: Oficina can read the state, the fault records,
  the stored program and its source, and put the previous version back, through a small interface
  frozen from release 1.

The assistant recommends (a) with (c): updating the controller to change a program on an old
machine is a risk its owner should choose, not one Oficina imposes; and (c) is cheap only if it is
frozen before the first release.

**Taken meanwhile**: nothing; the area is not built.

**Reversible?** (a) and (b) yes; (c) only before the first release, since the frozen interface is
what later releases keep.

**Decided** 2026-11-26, by the architect, in two answers:

- Asked "An old controller and a new Oficina: a controller stays as the engineer left it for years
  while Oficina moves on. Which options hold? (Controlador's pick: (a) with (c).)", chosen: "(a)
  Oficina speaks old versions" and "(b) Controller updated first"; not chosen: "(c) A rescue path,
  frozen".

- Asked "You chose both \"(a) Oficina speaks old versions\" and \"(b) Controller updated first\".
  They pull apart on one case: a program download to a controller older than Oficina's release. What
  holds there?", "Update first to download": Oficina refuses to download to an older controller and
  offers its update first, and speaks the old versions for everything short of that (connecting,
  reading the state and records, watching, forcing). No interface is frozen from release 1 as a
  rescue path.

---

## Q236 - Replacing the hardware: backup, restore, what a stranger needs

**Raised** 2026-11-26, by the controlador session, proposing the third of the four areas Q42 left
unopened (`docs/robustness.md`, area 8).

The board fails after ten years; someone who never met the engineer has the cabinet, a new board
that meets the hardware requirements (Q226) and whatever was kept.
- (a) A backup is one file, a manifest listing its parts with their SHA-256 as a download's does
  (Q36): the program and its source, the hardware configuration, the IO mapping, the target's own
  settings and the retained data at that moment. Oficina makes it from the controller and restores
  it to any target that meets the requirements, checked by its manifest; a restore asks a person
  whether the retained data is restored or the program starts cold.
- (b) As (a), and the backup carries a rebuild sheet in plain text: the board and drive it needs,
  the system image's version and where to get it, the network names and addresses, what to install
  and in what order, written for someone with no knowledge of Veredas.
- (c) The project folder is the backup, kept by the engineer; the controller holds nothing the
  project lacks but the retained data, saved on request.

The assistant recommends (b): what fails years later is the knowledge around the program, and the
sheet is what a stranger reads first; (c) loses the retained data and every setting made on the
target.

**Taken meanwhile**: nothing; the area is not built.

**Reversible?** Yes: the backup's format is a `veredas-*` format with a version (law L7).

**Decided** 2026-11-26, by the architect: asked "Replacing the hardware: the board fails after ten
years and someone who never met the engineer has the cabinet and a new board. What is a backup?",
"One file with a rebuild sheet (Recommended)", (b), against the file without the sheet and the
project folder as the backup.

---

## Q237 - How each rule is proven by injecting faults on the bench

**Raised** 2026-11-26, by the controlador session, proposing the fourth of the four areas Q42 left
unopened (`docs/robustness.md`, area 10).

Every robustness rule is a sentence of a plan; it holds only when a test shows the controller
keeping it under the fault it guards against. Maquinar's bench plays the plant on virtual time
(fixture first).
- (a) A fault catalogue: for every rule a named fault the bench injects (a power cut at a chosen
  instruction, a drive that fails a write or reports wear, a clock that jumps or an NTP that lies, a
  device lost mid-exchange, memory full), each entry linking the test of the clause that builds the
  rule.
- (b) As (a), and a long soak: weeks of virtual time with faults drawn at random from the
  catalogue, the controller's invariants checked throughout (no output but a computed or a safe
  one, a state that reads back, nothing that grows), run before each release.
- (c) Faults proven on real hardware in a laboratory before each release.

The assistant recommends (b), with a few of (c)'s power cuts on real boards before a release: (a)
proves each rule alone, the soak their combination, and virtual time makes years of it cheap,
while real hardware alone is too slow to repeat.

**Taken meanwhile**: nothing; the area is not built.

**Reversible?** Yes: a catalogue grows entry by entry.

**Decided** 2026-11-26, by the architect: asked "Proving the robustness rules: each holds only when
a test shows the controller keeping it under the fault it guards against, the bench playing the
plant on virtual time. How are they proven?", "Catalogue, soak, some real cuts (Recommended)", (b)
with a few of (c)'s power cuts on real boards before a release.

---

## Q47 - How much debug information do development builds carry?

**Raised** 2026-11-23, by the orchestrating session, on the home disk's use with ten sessions.

The workspace manifest compiles the database engine without debug information and seven OPC UA
crates, eleven Arrow crates and `duckdb` with line tables only (1549cb9, 2026-11-10), and says why
it stops there: "A backtrace still names file and line; the suite's own crates keep full debug
info". Since then every test binary still carries a full copy of the debug information of what
it links, and a session's `target` grew to between 23 and 84 GB in a few hours; the home disk
(935 GB) fell by up to 100 GB an hour on 2026-11-22.

Measured on 2026-11-23 at 8ce94c9, a full test build (`cargo nextest run --workspace --no-run`)
into a folder seeded with the same 11 GB of compiled dependencies each time:

| Development profile | Folder after | portal's part | maquinar's | supervisão's | core's |
|---|---|---|---|---|---|
| (a) as the manifest stands | 34 GB | 5.6 GB | 3.3 GB | 3.2 GB | 2.1 GB |
| (b) line tables only for every crate | 25 GB | 3.2 GB | 1.6 GB | 1.9 GB | 1.0 GB |
| (c) debug information stripped at link | 21 GB | 1.8 GB | 0.9 GB | 1.1 GB | 0.4 GB |

(b) and (c) also recompiled every dependency, which (a) did not, so their folders hold about 5 GB
that a seed built with the same profile would have spared. Under (c) the whole suite ran green,
1415 of 1415.

Options:
- (a) Leave it. Full debug information for the suite's own crates; the disk is kept by resetting
  build folders.
- (b) `debug = "line-tables-only"` for the whole development profile. A panic names its place and
  a backtrace still names file and line, as the manifest's comment asks; a debugger no longer
  shows variables. About half the size.
- (c) `strip = "debuginfo"` for the development profile. A panic still names its place; a
  backtrace names functions without file or line. About a third of the size.

(b) and (c) each cost one full rebuild in every build folder, the database engine included, and
change no release build.

**Recommended**: (b), which keeps what the earlier commit set out to keep. **Taken meanwhile**:
(a), nothing changed.

**Reversible?** Yes: one line of the workspace manifest, and one more rebuild.

**Decided** 2026-11-24, by the architect: asked "Build development code with line tables only, to
halve the build folders (Q47)?", "Yes, line tables", (b). The workspace manifest sets
`debug = "line-tables-only"` for the whole development profile; the crates named one by one since
1549cb9 no longer need their own lines, and the engine keeps its own setting (optimised, no debug
information). Every build folder rebuilds once.

## Q130 - A time of day in nanoseconds on a page: a panic, a refusal by name, or read?

**Raised** 2026-11-23, by portal, answering the orchestrator after 98acf3b1 turned supervisão's
`dashboards` red.

The engine's driver cannot read a `TIME_NS` value: `row.get` panics on it ("internal error:
entered unreachable code: invalid value: 1, Time64(ns)"). A page of rows (`DuckStore::query_rows`,
C-041) over a view holding one has always panicked there. Supervisão catches the panic and says
in the panel's place "the panel could not be computed: …", and its test pins that text
(`crates/veredas-supervisao/tests/dashboards.rs`, panel 22 of the reference). 98acf3b1 made the
page one transaction; the panic skipped its `ROLLBACK` and left the server's connection inside
it, which turned the test red. The guard that ends the transaction on the way out, a panic
included, is the fix. The value itself can go three ways:
- (a) The panic, as before: the caller's to catch. Supervisão's test holds as written.
- (b) Refused by name: the page fails with "table `nanos`: the column `at` of type TIME_NS
  cannot be read", and the panel shows that text. Supervisão's assertion at panel 22 changes to
  that text.
- (c) Read: portal casts the column to `TIME` (microseconds, the nanoseconds' last three digits
  dropped), and the value is written as a time of day like the others. Supervisão's panel 22
  reads, and the limit C-052 states for it goes.

**Recommended**: (b): no panic crosses a library's surface, and the panel still says why. Or (c),
if the panel should show the time. **Taken meanwhile**: (a), the one that keeps `main` green
without editing another member's test; (b) and (c) are each a few lines in `store.rs`, both tried
and green in portal's own tests.

**Reversible?** Yes: portal's change and supervisão's assertion, in one push each.

**Decided** 2026-11-25, by the architect: asked "When a page of rows holds a time of day in
nanoseconds (TIME_NS), which the database driver cannot read, what does portal do? Taken meanwhile:
the driver's panic, which supervisão catches and shows as \"the panel could not be computed\".",
"Read it as microseconds", (c), against the recommendation (b): portal casts the column to `TIME`,
the nanoseconds' last three digits dropped, and the value is written as a time of day like the
others. Supervisão's panel 22 then reads, and the limit C-052 states for it goes.

## Q131 - A CSV that two dialects fit: guess, or show both?

**Raised** 2026-11-24, by portal, after fourteen refute rounds of C-041's CSV weighing.

When a linked CSV is ragged or the engine's sniffer collapses it, portal proposes a dialect: the
delimiters whose header fits the rows are weighed by the columns they type as numbers, then the
dialect given, the widest, the order (`store::csv_fitting_dialect`, round 10's weighing). Some
files fit two delimiters and the numbers cannot tell which is right: a `;` file whose last column
is a list `10,20,30` named `x,y,z` reads on `,`; a `,` file with a list of `;` between a stamp and
a temperature reads on `;`, the stamp glued to the list as a decimal; a `;` export of one decimal
column whose name holds a comma (`Temperatur, C`) ties with `,`, and a cut last line leaves it on
`,` with every decimal cut in two. Rounds 11 to 13 added rules to tell a list from a cut; each
broke `;` exports of decimal commas, which are far commoner, and was withdrawn.

A view can always name its dialect (`CsvOptions`: the delimiter and the decimal comma written
into its reader), and then reads right.
- (a) Keep the weighing as it is, its limits stated in C-041; the operator names the dialect when
  a preview reads wrong.
- (b) Keep refining the weighing. Fourteen rounds suggest each rule moves the misreads elsewhere.
- (c) When two dialects fit, portal returns both readings (each dialect with its first rows), and
  the link's page (Oficina's, supervisão's) shows them and lets the operator pick; the pick is
  written into the view. Portal's share is small: the candidates already exist in `fitting`.

**Recommended**: (c), with (a) as its fallback for a file no person looks at. **Taken
meanwhile**: (a).

**Reversible?** Yes: (c) adds a call beside `csv_fitting_dialect` and changes no reading.

**Decided** 2026-11-25, by the architect: asked "When a linked CSV fits two dialects (`;` or `,`)
and the numbers cannot tell which is right, what does portal do? Fourteen refute rounds moved the
misreads around without ending them. Taken meanwhile: keep the weighing, limits stated.", "Show
both, operator picks (Recommended)", (c): when two dialects fit, portal returns both readings, each
with its first rows, the link's page shows them and the pick is written into the view; the weighing
(a) stays for a file no person looks at.

---

## Q319 - How a replay knows that what a session's start reads into the store has been read

**Raised** 2026-11-24, by the comissionador session, from C-205's second refute round.

`veredas-comissionador replay` starts a session from the project file and replays a window of its
store, so it must wait until the start has read the logs' history into the store. Portal shows a
log's count of lines and the last stamp it read, nothing about whether its start is done. The
executable now takes a log that has read a line stamped after the window's end as done (portal
reads a log in its stamps' order), and the others as done when their lines hold still across two
of portal's refreshes (about a second). That second is a guess: portal reads each older member of
a family whole and waits for it to hold still before the next, so a start whose family holds four
or more empty members in a row (or members that took long to open) pauses longer, the replay
starts before the newest member is read, and the run is kept aborted with no row of its window
(the critic: 5 empty members aborted 2 runs of 3, 8 all of them). Options:
- (a) Portal publishes, per log, that its start's read-in has reached the end of every file as it
  was when the start opened it (a flag in `SourceHealth`, or a count of the files the start
  planned and the ones read). Needs portal; the executable then waits for that flag, exactly.
- (b) The executable waits longer for stillness (five seconds, say). Still a guess: it moves the
  failure to more empty members, and every replay pays it.
- (c) The claim narrowed: the limit stated, a family's empty members named, nothing else changed.

The assistant recommends (a): the start's plan is portal's (`tail::plan_for`), so portal alone
knows when it is done, and supervisão's host and any replay of a fresh session want the same.

**Taken meanwhile**: (c), with the line past the window: C-205 states the limit, and `main.rs`
marks the stillness `DECISION PENDING (Q319)`.

**Reversible?** Yes: (a) replaces the stillness in one loop of `main.rs`; the line past the window
stays as a shortcut.

**Decided** 2026-11-25, by the architect: asked "How does a comissionador replay know that a fresh
portal session has finished reading the logs' history into the store? Today it guesses from the
lines holding still for a second, and a family with several empty log files defeats the guess. Taken
meanwhile: the limit is stated.", "Portal says so (Recommended)", (a): portal publishes, per log,
that its start's read-in has reached the end of every file as it was when the start opened it, and
the replay waits for that. The flag is portal's to build; until it lands the stated limit (c) holds.

---

## Q370 - Does the suite's build need Qt for Oficina's window?

**Raised** 2026-11-25, by the oficina session, starting `crates/veredas-oficina` on the
architect's word ("start the oficina crate").

Oficina's window links Qt 6.12 (Qt Quick, through cxx-qt) and KDDockWidgets, and its tests Spix
and AnyRPC: Qt from its installer and three C++ libraries built from source. Built by default,
every member's `cargo build --workspace` and pre-push run, the orchestrator's whole-suite run and
GitHub's run would need all of them installed and `QMAKE` naming Qt's qmake, and would compile the
bridge's C++ (about a minute on this machine). Options:
- (a) The window behind a cargo feature of Oficina's, `shell`, off by default, and the tests'
  driver behind `driver`. The suite's default build compiles Oficina's model alone, with no Qt;
  Oficina's window tests, the ones that prove its clauses, run where Qt is, with
  `--features veredas-oficina/driver` and `QMAKE` set. The orchestrator's whole-suite run adds
  both; GitHub's run does not, until it has Qt.
- (b) The window built by default: every machine that builds the suite installs Qt 6.12,
  KDDockWidgets, Spix and AnyRPC (the Windows guest has them through
  `spikes/oficina/install-windows-tools.ps1`), and GitHub's run installs them too.
- (c) Oficina outside the suite's workspace, as its spike is: refused by L1 (a `veredas-*` crate is
  a member under a root or nothing).

The assistant recommends (a) now, and (b) once Oficina has several surfaces and the suite's runs
should hold its tests without being asked.

**Taken meanwhile**: (a). The tests' driver stays a feature of its own whichever is ruled: what
ships carries no test server. With (a), the release tool (`tools/alforge`, which builds each
component with `cargo build --release --locked --offline -p <component> --bins` and no feature)
must name `--features shell` for Oficina, or it builds no executable of it: the releases member's,
when Oficina first ships.

**Reversible?** Yes: (b) is `default = ["shell"]` in Oficina's manifest, and Qt on the machines.

**Decided** 2026-11-25, by the architect: asked "Does the suite's default build need Qt for
Oficina's window (Qt 6.12, KDDockWidgets, and Spix and AnyRPC for its tests)? Taken meanwhile: the
window sits behind a cargo feature, off by default.", "Built by default", (b), against the
recommendation: the window is built by default, so every machine that builds the suite carries Qt
6.12, KDDockWidgets, Spix and AnyRPC, GitHub's run among them. The switch waits until every member's
build on the development machine finds them.

---

## Q371 - How far back does Oficina speak to an older controller?

**Raised** 2026-11-26, by the oficina session, from Q235's ruling: "(a) Oficina speaks old
versions", whose option read "back to a floor it states (a number of releases or of years)", the
floor itself not ruled.

Oficina reads a controller's versions when it connects (its release, the ORCL language versions
it runs, the program file's format, each TOML file's `format` and `version`) and writes only what
that controller reads, for everything short of a download (Q235). Each version it speaks is code
kept and a test kept.
- (a) No floor yet: every release from the first, a floor stated when a version is first dropped,
  with the release that drops it.
- (b) A number of releases: the last N releases of controlador.
- (c) A number of years: every controller release of the last N years, so a machine left as
  delivered stays reachable for that long (Q236's case is a board that fails after ten years).

**Recommended**: (a): nothing is dropped before release 1 is out, and the choice is easier with a
first real version to drop.

**Taken meanwhile** (most reversible): (a), nothing built: the area lands with the target's
clauses.

**Reversible?** Yes until a release drops a version; a dropped version's machines then need the
controller's update to be reached.

---

## Q320 - How a person confirms a run's gated writes, and a switch a run finds armed

**Raised** 2026-11-25, by the comissionador session, from the architect's ruling on Q314 (b): the
run arms portal's switch at its start once the person has confirmed the list of the test's gated
writes, and disarms it at its end. Two things the ruling leaves open:

1. **How the person confirms, with no screen** (Oficina's designer is not built). Options:
   - (a) A fingerprint of the list: the executable, refusing a run whose switch is off, prints the
     test's gated writes as written and their fingerprint (32 hex digits: 128 bits of SHA-256 over the list, the test's whole text and the function files it uses, since C-202's round 4 found two tests made to share a 64-bit FNV-1a one); the person runs again
     with `--arm <fingerprint>`, which confirms that list and no other: an edited test asks again.
     Oficina's screen later shows the list and confirms it the same way, through the library.
   - (b) An interactive prompt in the executable (`arm the switch for these 3 writes? [y/N]`).
     Natural on a laptop, but nothing can confirm it in a pipeline.
   - (c) A flag with no list (`--arm`): the person confirms what they have not been shown.
2. **A switch the run finds armed** (another user of the same session armed it). Options:
   - (a) Left as found: the run uses it and never disarms a switch it did not arm.
   - (b) Disarmed at the run's end anyway, as option (c) of Q314 read: surprises the other user.

The assistant recommends 1 (a) and 2 (a): a fingerprint binds the confirmation to the list and works
headless, and leaving a switch as found changes nothing for anyone else.

**Taken meanwhile**: 1 (a) and 2 (a). C-202 and C-205 state them, the tests hold them.

**Reversible?** Yes: the fingerprint is one function (`run::writes_fingerprint`) and one option
(`--arm`); a prompt or another form of confirmation would call the same `RunOptions::confirming`.

**Decided** 2026-11-26, by the architect, in two answers:

- How the person confirms. Asked "With no screen yet, how does the person confirm? Taken meanwhile:
  a fingerprint of the list.", "A fingerprint of the list (Recommended)", 1 (a), against an
  interactive prompt and a flag with no list.

- A switch found armed. Asked "When a comissionador run finds portal's write switch already armed by
  another user of the same session, what does it do at its end? Taken meanwhile: leaves it as
  found.", "Left as found (Recommended)", 2 (a). What was taken stands in both.

---

## Q281 - How does a Windows laptop send and hear raw frames?

**Raised** 2026-11-26, after reading Npcap's licence (`docs/aranha-plan.md`, "Npcap's licence";
Q45's G2: identify, name and address runs from the laptop on the segment too).

On Linux aranha sends and hears raw frames itself (`AF_PACKET`). Windows has no such socket; Veredas
1 used Npcap. Npcap's free licence lets the engineer install it on up to five computers (a virtual
machine counting as one) and recommends that software ask its users to install it themselves; it
forbids bundling it, has no silent installer, and its opening line says it "may not be
redistributed or used in other software without special permission", which it leaves undefined.

Options:
- (a) The engineer installs the free Npcap from npcap.com; aranha neither ships nor starts its
  installer, and says so where it needs it. Up to five machines per engineer; a note to the
  publisher would settle what "used in other software" means for software that only drives it.
- (b) The suite buys Npcap OEM: the redistribution licence (from $39,980 perpetual, or $6,980 a
  quarter) to ship it and install it silently, or the internal-use licence (from $4,980 for 100
  installs) for an organisation's own laptops.
- (c) No raw frames on Windows: a Windows laptop identifies, names and addresses through a target's
  embedded aranha, as its IO check goes through a target (Q45, ruled 2026-11-25); the laptop path
  of G2 is Linux only.

**Recommended and taken meanwhile**: (c), the Windows phase being later: nothing of Windows is
built until it is ruled, and a target serves a Windows laptop already.

**Reversible?** Yes: a Windows wire is one more implementation of the wire seam (C-139); nothing
above it changes, whichever of (a) or (b) comes.

**Decided** 2026-11-26, by the architect: asked "How does a Windows laptop send and hear raw frames
for aranha (identify, name and address from the laptop)? Windows has no raw socket; Npcap's free
licence allows the engineer to install it (up to five machines) and forbids bundling it. Taken
meanwhile: no raw frames on Windows.", "The engineer installs Npcap", (a), against the
recommendation (c): the engineer installs the free Npcap from its publisher, and aranha neither
ships nor starts its installer and says so where it needs it. So the laptop's own path of G2 runs on
Windows too, once its wire is built.

---

## Q190 - A database path the engine reads otherwise than as a local file: read it as the engine does, or refuse it?

**Raised** 2026-11-25, by the supervisão session, from C-048's refute rounds 22 to 25. The host
refuses two sessions that would open one database file (C-048), so it must know which file the
engine opens for each path. DuckDB rewrites a path before the drive reads it, in more ways each
round has found: a leading `~` (the home folder), a `file:` URL in three forms, a type before the
first colon (`duckdb:` its own, `sqlite:` and any other an extension's), another file system before
`://` (`s3://`, `http://`, `hf://`, `az://` and more, opened through httpfs or azure or not at
all), a second expansion inside its local file system, and a write-ahead log (`<file>.wal`) beside
every database. Such a path reaches the engine only when the supervisão file itself is given by a
relative path whose first part is a folder named so (`~`, `duckdb:`, `s3:`), which no project
writes on purpose. Options:

- (a) **Read it as the engine does**, as rounds 23 and 24 built: the reader copies DuckDB 1.5.4's
  rules by hand, each round finding the next, and an engine upgrade can change them unseen.
- (b) **Refuse it by name**: a path whose first part holds a colon (a one-letter drive on Windows
  apart) or begins with `~` is refused at the check, "a path the engine may read otherwise than as
  a file here"; the reader then places plain paths alone, and only the write-ahead log is the
  engine's own rule left to follow.
- (c) **Hand the question to portal**: portal opens the database and could say which file it opened
  (its guard against a database already standing, C-041, checks the path as written today).

The assistant recommends (b): it fails closed, needs no copy of the engine's rules, survives an
upgrade, and costs nothing a project does on purpose; (c) is right in the long run but is portal's.

**Taken meanwhile**: (b), with the write-ahead log compared beside each database. C-048 states it,
`config.rs` marks it `DECISION PENDING`.

**Reversible?** Yes: the refusal is one function (`config::engine_reads_otherwise`); reading the
forms as the engine does again would put round 24's `as_the_engine_reads` back in its place.

**Decided** 2026-11-26, by the architect: asked "The host refuses two sessions that would open one
database file, so it must know which file the engine opens for a path; DuckDB rewrites some paths
(`~`, `file:`, `s3://`, a type before a colon), which no project writes on purpose. What does
supervisão do with such a path? Taken meanwhile: refuses it.", "Refuse it by name (Recommended)",
(b), against reading it as the engine does and handing it to portal. What was taken stands.

---

## Q321 - What a run's confirmation covers when the switch it arms lets the session's replies through

**Raised** 2026-11-26, by the comissionador session, from C-205's fourth refute round.

Portal's write switch is the whole session's: a channel's automatic replies (a project's
`replies`, answering each announcement) wait on it as a test's writes do (`docs/portal-plan.md`).
So when a run arms it for the gated writes the person confirmed (Q314 (b)), the project's replies
reach the plant too: the critic's run of the worked example, confirmed for its three sends, let 74
messages through, its replies among them, none of them shown. Options:
- (a) The confirmation covers both: the refusal lists the test's gated writes and the session's
  replies, and the fingerprint is of both (with the texts they depend on), so editing a reply rule
  asks again.
- (b) A run whose session has replies is never armed by the run: the person arms the switch
  themselves, or the project drops its replies. A test that needs the replies (the worked example)
  then cannot be confirmed by its run.
- (c) Portal gives a gate narrower than the session's, arming a run's writes and leaving the
  replies as the project's switch has them. Needs portal, and a test that needs the replies still
  needs them armed.

The assistant recommends (a): the person sees and confirms everything the armed switch lets reach
the plant, and the tests that rely on replies still run.

**Taken meanwhile**: (a). C-202 and C-205 state it, the tests hold it.

**Reversible?** Yes: the replies are handed to the plant beside the arming (`LivePlant::with_arming`)
and read by one method (`Plant::also_armed`); refusing instead is one branch of the binding.

**Decided** 2026-11-26, by the architect: asked "Portal's write switch is the whole session's, so a
run that arms it for its confirmed writes also lets the project's automatic replies reach the plant
(a critic's run confirmed for 3 sends let 74 messages through). What does the confirmation cover?
Taken meanwhile: both.", "Writes and replies (Recommended)", (a), against never arming a session
that has replies and a narrower gate in portal. What was taken stands.
