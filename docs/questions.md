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
said by its mark and word and the side panel, and a selected level's line grows by 2 so the accent
shows around the level running inside it; a stale line is dashed at its own width; in normal a line
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

**Taken meanwhile**: on a controller the store is the memory window, as ruled. Since 2026-11-22
portal has built three pieces of this area, each on the most reversible answer and none ruled:
- the window in memory has a span and no ceiling in bytes (C-090);
- a recording on the drive stops at its size limit, deletes nothing and counts the rows that came
  after, which is (b) for that record (C-091);
- every fault gets a copy of the store, none is deleted, and nothing limits their number or their
  rate (C-093).

The rest waits: fault records, the state history, earlier downloads and the logs have no limit
and no rule yet.

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

---

## Q432 - Does a release tag its commit in git?

**Raised** 2026-11-22. A manifest names the commit it was cut from; a tag `suite-<release>` on it
would make the commit findable by name and show on GitHub.

Options: (a) a manifest alone, no tag; (b) an annotated tag `suite-<release>`, pushed with the
manifest's commit; (c) a tag per component too (`veredas-portal-2.0.0-alpha.1`).

**Taken meanwhile** (most reversible): (a). A tag pushed is public; nothing is tagged until ruled.

**Reversible?** Yes, while nothing is tagged.

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
