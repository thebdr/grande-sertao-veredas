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

---

## Q7 - Do the TRISTATE bin-state bits stop a chute?

**Raised** 2026-11-05 by the refute round of C-011: the GENIBELT specification (§4.2.1.4) names
bits 28-31 of a bin state (MAPPING, EMERGENCY_TRISTATE, ALARM_TRISTATE, WARNING_TRISTATE) without
saying what they mean for parcel acceptance. Veredas 1 named none of them.

**Taken (reversible, pending the operator):** they are named (a state word reads as words), and
`accepts_parcels` ignores them, as Veredas 1 did, so no chute that sorted before stops sorting.
Marked `// DECISION PENDING` in `veredas_core::telegram::genibelt::bin_state`.

**Reversible?** Yes: one constant set in `accepts_parcels`, and the test that pins it.
