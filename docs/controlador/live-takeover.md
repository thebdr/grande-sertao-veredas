# Live takeover: what disqualifies an edit

**A proposal, 2026-11-25, for the architect to rule as a document** (`docs/questions.md` Q32: "controlador writes
the whole specification as a proposal, and the architect rules on it as a document"). Nothing here is settled
until then, and no live takeover is built before it is ruled. `docs/controlador-plan.md` ("Online change") wins on
intent; this file is the specification it owes.

## What a live takeover is
An edit reaches the plant through a shadow test: the new version is deployed, both versions run on the same inputs,
each on its own data, and after a person's swap the new version drives the outputs while the old one keeps running
as the watcher (the plan, "Online change"). With the fallback armed (a controller-wide flag offered in the download
popup, with a time limit, reset on shutdown), a failure of the new version while the old one watches can hand the
outputs back to the old version without a stop: a **live takeover**. Without it, the whole controller faults as it
always does, its outputs safe, and the previous version is put back for the next start: "takeover after restart
remains possible for all cases" (the architect, 2026-11-15).

What was ruled with Q32 stands around this proposal: an edit that failed leaves a fault record named "the edit
failed", not a controller fault; and while the old version watches and may still take over, the state text
carries a term with the time it has left.

## The principle
The old version may take over only when its picture of the plant is the plant's own: when everything the plant
received since the swap is what the old version computed for it, from the same inputs. Then, seen from the plant,
the old version has been driving all along, and its steps, latches and timers are where the plant is. The review's
scenario (entry 5, `docs/briefing/review.md`) is the case this excludes: a sequence the new version took from step 3
to step 9 by commands the old one never sent, the old one's timeouts firing meanwhile.

Each disqualifier below breaks that equality, or makes it unknowable. A disqualifier found before the swap keeps the
fallback from being armed; one met while the old version watches ends its right to take over for the rest of the
test. The test itself goes on either way: a lost right is not a fault.

## Disqualified before the swap: the fallback cannot be armed
The download popup offers the fallback only when none of these holds, and the controller refuses a swap with the
fallback armed when one does (Oficina's check is a convenience; the controller's is the rule).
- **S1. The IO mapping differs**: an output moved, added or removed, an input mapped elsewhere. Outputs are then not
  comparable channel by channel, and an output may lose its writer at a takeover (the review's entry 15).
- **S2. What may never differ in a swap differs**: the hardware configuration or the task timing (the plan, "What
  may differ"), refused for any swap already and named here for completeness.
- **S3. What peers see differs**: the published variables, the Modbus addresses, the OPC UA nodes or which variables
  are writable from outside. A takeover would change under a peer what it reads or writes, which the comparison of
  outputs does not cover. (The assistant's reading; the architect may rule it out of the list.)
- **S4. The test cannot compare**: an application the shadow test cannot run whole, or a comparison the controller
  cannot keep up with in the doubled scan. The entry check already refuses the first; the second disqualifies the
  fallback, never the test.

## Disqualified while the old version watches: the right to take over ends
From the swap on, the controller checks every scan. The first of these ends the right to take over; the state text
drops the armed term, the state history records which one and when, and Oficina says so.
- **D1. An output differs.** At the end of any scan since the swap, a physical output of the task's slice that the
  new version wrote differs from what the old version computed for it: its value (floats compared bit for bit, the
  interpreter being deterministic), or whether it was written at all. This is the candidate criterion of the
  review's option (b).
- **D2. A write from outside that the old version did not take alike.** A write through the embedded portal (a peer
  or Oficina) reaches both versions at their scan boundary, by name and type. A write to a variable the old version
  lacks, holds under another type, or refuses (its limits) leaves the two versions with different data the outputs
  may not show until later; it disqualifies at once.
- **D3. A force.** A force belongs to the controller and applies to both versions alike, but a forced plant is not a
  plant the watcher computed for, and a force on what the old version lacks cannot apply to it. Any force set while
  the fallback is armed disqualifies. (A narrower rule, only forces the old version cannot take, is possible; the
  assistant proposes the strict one, by the language's own principle that loosening breaks nothing.)
- **D4. A halt.** A breakpoint hit, a pause or a single cycle. Breakpoints sit in the driving version only and the
  watcher pauses in step (the plan), but outputs at a halt follow the halt settings and the plant then moves on its
  own; debugging is not the production a live takeover protects.
- **D5. The watcher in trouble.** An error or an overrun in the old version ends the shadow test itself (the plan);
  so does its own fault. With no watcher there is no takeover.
- **D6. A device event the two did not handle alike.** A device lost, returned or reporting a diagnosis reaches both
  versions' handlers; if one has a handler and the other none, or their outputs then differ, D1 or this rule ends
  the right. (Kept separate so the state history says why.)
- **D7. The time limit ran out.** The edit is then accepted and the old version stops watching (the plan); there is
  nothing left to take over.

## The takeover itself
When the new version fails and none of the above has happened since the swap:
- **T1. Which failures hand over.** A program error in the new version (the fault 502 `program(at)`) or a fault it
  raised itself, in a scan whose outputs it therefore never wrote (C-151: a faulting scan writes no outputs of its
  own). Not handed over, and handled as faults: the watchdog (501), since the scan did not end and the bus thread has
  already sent the safe values; resources exhausted; the runtime's own failure; a check of the stored program that
  fails (506 `load`, Q36); and a power cycle, after which the previous version can be put back as always.
- **T2. In the failing scan.** The old version's outputs for that very scan are written in place of the new
  version's, so the plant sees no gap and no safe value. The old version becomes the only version and drives on;
  the new one is unloaded.
- **T3. What is recorded.** A fault record named "the edit failed", with the new version's error, its source
  location, its download's identity and the swap's time; the maintenance request `fault_on_update`; the download
  history marking the failed download, the running one being the last accepted. The controller stays in RUN and the
  state text says what happened.
- **T4. Retained data.** The old version's retained variables are the ones it computed itself while watching, which
  D1 to D3 keep equal in effect to the plant's; the next retained save is the old version's.

## How it will be proven
On the bench (maquinar), each case a test of the clause that builds it, on virtual time:
- an edit to a sequence that faults after many scans with every output equal: the old version takes over in the
  failing scan, the plant's outputs continuous, no safe value written, the fault record and the request present;
- the same edit with one output different once, long before the fault: the right ends at that scan, the state says
  so, and the fault is a controller FAULT with safe outputs;
- each of S1 to S4 refusing the armed swap, and each of D2 to D7 ending the right, by name;
- a failure in each class of T1, handed over or not as listed.

## What the architect is asked
- Is this the specification: the principle, S1 to S4, D1 to D7 and T1 to T4?
- Two of its points are the assistant's own readings, each marked above: S3 (what peers see) and D3's strictness.
- The fallback's time limit: a default, and whether it is the engineer's to set (the plan says it has one, not
  what it is).
