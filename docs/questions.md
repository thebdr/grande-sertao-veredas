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
