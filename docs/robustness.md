# Robustness: the specification round

A round of decisions the operator called on 2026-11-16, for the whole suite: what a machine built
on Veredas needs in order to run unattended for decades. It began the same day, by the operator
replying to single statements, and was paused on 2026-11-19 when development moved to a Linux
environment. `docs/veredas-plan.md` wins on intent; `.zen/contract.md` (P-017) wins on status.
Nothing of it is built.

This file states what the operator ruled, in the operator's words where they carry the rule, and
the facts the rulings rest on. What the assistant proposed and nobody ruled is not here: it is in
`docs/questions.md` (Q36 to Q42), each with the option taken meanwhile. The rulings bind
controlador first (`docs/controlador-plan.md`), then portal where it is embedded in a controller.

## Why
"these machines are meant to be used for decades left alone, people that program them die sooner."
So the design horizon is decades without attention, and the machine has to explain itself to
whoever comes after. The operator's picture of the trade, given with it: "a plc programmer's nr. 1
goal is to never go back to a plant for debug or repairs", hence logic that people on site and
colleagues can read, and "set up proper remote connectivity, test it before leaving site".

The round was called in these words: "we need parity data and that needs a whole new spec
definition round: robustness (overall, concerning all the suite's critical conponents)".

## The areas
The assistant proposed ten areas. Where each stands:

| Area | Standing |
|---|---|
| 1. What must survive | ruled; its border cases are Q38 |
| 2. Detecting and repairing damage | ruled in principle |
| 3. Power loss at any moment | ruled in principle |
| 4. Wear and filling over decades | opened, three rulings; the rest is Q40 and Q41 |
| 5. Time over decades | ruled; its details are Q39 |
| 6. Running unattended (hardware watchdog, memory errors, slow leaks) | the cycle-time rule alone; Q42 |
| 7. An old controller and a new Oficina (versions, migration, a rescue path) | not opened; Q42 |
| 8. Replacing the hardware (backup, restore, what a stranger needs to rebuild) | not opened; Q42 |
| 9. Which other components count as critical | not opened; Q42 |
| 10. How each rule is proven by injecting faults on the bench | not opened; Q42 |

## Everything that alters the controller
- **A download carries a hash.** "to download a software update, compiled on the sender's machine,
  we need to make it robust, adding a hash or crc so no corrupted program may run. this is
  critical" (2026-11-16).
- **Every altering request is robust**, not the download alone: "every bit of data that alters the
  controller shall be robust, that includes commands such as variable writes, breakpoints setting,
  state change" (2026-11-16).
- **The stored program's integrity is rechecked at power-on**: "great idea, at power on recheck
  the integrity" (2026-11-16).
- **No check of a program's structure before it runs**, for now: "rust as a base gives us an
  optimal linter and compiler. no further checks necessary (for now)" (2026-11-16).
- How a download's parts are hashed and how a request is made robust were proposed and not
  confirmed: Q36 and Q37.

## What must survive
"i'd say everything out of the commissioning scope. picture a cpu doing simple i/o logic that is
left there for years. that is what a plc is, it runs as the day the software engineer left for an
undefined time. all logic is custom made, the code is open [...] what must survive: every piece of
data that is necessary for computing the logic flawlessly in RUN mode" (2026-11-16).

So the rule is that last sentence, and the commissioning scope is outside it. Which data that
makes inside and outside, and the border cases, are Q38.

## Detecting and repairing damage
- **Two copies and a hash per block.** The operator proposed to "store the program and the hash
  twice, and use some corruption reconstruction algorithm", and agreed to its form (2026-11-16): a
  hash per block, so that two copies mend each other even when both are damaged in different
  places, and parity only for the block that is bad in both.
- **A check on a schedule while running**, which repairs and then raises a maintenance request,
  since such a machine may run for years without a power cycle. The operator's answer to it was
  "wow", taken as agreement.
- **Where the copies live is the engineer's parameter**: "we provide a double store, so the choice
  to use two different hard drives is a parameter the engineer sets" (2026-11-16).

## Power loss
- **A cycle's data becomes permanent only after a successful cycle**: "a power loss must be
  controlled, regardless: every cycles saved data is temporary and moves to permanent only on
  successful cycle" (2026-11-16).
- **Controlador on chosen hardware, for everyone**: "controlador on choosen hardware for everyone:
  great".
- **Hardware of our own is a direction, not a decision to start**: "we should design our won
  hardware with: multiple memory chips for different tasks, a small battery that holds 2 cycles
  (to be able to save the state before shutdown), a known processor (multithreading, multi cpu),
  and many more".
- **The operating system is studied before anything is invented** (see "The Linux study").

## Time over decades
- **A dead clock battery is answered by NTP**: "dead clock battery: integrator provides an ntp
  server".
- **Programs avoid absolute dates**: "we PLC programmers are not using absolute dates, at least we
  try not to"; "plc developers avoid absolute time related errors, and an ntp delegates the
  responability".
- **Controlador runs on 64-bit processors only** (2026-11-16). A 32-bit time value ends on 19
  January 2038 and NTP cannot mend that.
- **The suite's certificates expire on the last 32-bit date in 2038**, 19 January 2038 at 03:14:07
  UTC: long-lived like the vendors', and renewing them is a maintenance visit.
- **A maintenance request a year before any certificate expires**, and the security round decides
  whether an expired certificate of a known peer cuts the link or only warns (2026-11-16, "both").
- **The date is unconfirmed until NTP or the clock chip confirms it** ("Yes, until confirmed",
  2026-11-16). Given with the question and not objected to: a program that reads the date gets a
  flag saying whether it is confirmed; timestamps taken meanwhile are marked; durations (the scan,
  timers, delays, hold times) are measured on a clock that only counts forward.
- **Time-of-day blocks wait** ("No, they wait", 2026-11-16): they do not start while the date is
  unconfirmed, and starts that fall before the confirmation do not run afterwards.
- **The clock battery and NTP raise maintenance requests**, by the operator's own formula
  (2026-11-16): "dead battery AND (no ntp set OR ntp down) = maint req urgent / otherwise non
  urgent. a setting flag to ignore dead battery on working ntp may be forseen". With the case the
  formula left out ("Yes, not urgent"), every case is ruled:

| The clock chip | NTP | Maintenance request |
|---|---|---|
| lost its time | not set, or set and silent | urgent |
| lost its time | answering | not urgent; a setting may switch it off |
| kept its time | set and silent | not urgent |
| kept its time | answering, or not set | none |

- **A maximum and a warning cycle time, set by the user**: "the user sets a maximum cycle time and
  a warning cycle time. the warning after years is a hint of possible degrading" (2026-11-16).

## Wear and filling

### What was found
Read in this repository on 2026-11-16 and 2026-11-19:
- **Portal's store has no upper bound.** It is in memory by default, or a database file on the
  drive, pinned by name or one per run in a folder (`docs/project-schema.md`, the `store` key;
  `docs/questions.md` Q16). Nothing is ever deleted, and no limit of size or age exists in
  portal's source.
- **It is written often.** A pass of the session's pump that brought data ends in a write to the
  store, and a pass comes every 50 ms (`crates/veredas-portal/src/session.rs`, `PUMP_IDLE`,
  `pump`, `flush`): up to about twenty writes a second.
- **Not everything portal serves needs the store.** The latest value of each series comes from the
  ingest's cache in memory and the live stream from broadcast frames (`Live.series`,
  `Live.frames`); values over a span of time are read from the store (`serve.rs`,
  `/api/series/data`, `/api/table/data`, `/api/tables`). The store always exists.
- **The store can already write itself to a file while running** (`DuckStore::save_to`).

Read elsewhere:
- DuckDB's manual says that its single-file format limits how space is handed back after rows are
  deleted, and that compacting a database means copying it into a fresh one
  (https://duckdb.org/docs/2.0/operations_manual/footprint_of_duckdb/reclaiming_space.html).
- Siemens' S7-1500 memory manual says that its memory card allows only a limited number of delete
  and write operations, and that the card's life depends on its size and on the number and type of
  writes.

### The arithmetic of wear
A drive's life is a budget: the bytes it can take, divided by the bytes written per day. The
figures are assumed and typical, not measured.

| Drive | Takes in total | Life with a 16 KB save every 10 ms cycle |
|---|---|---|
| 8 GB eMMC, 3,000 erase cycles | about 24 TB | about half a year |
| 256 GB industrial SSD, rated 600 TB | 600 TB | about 12 years |
| FRAM or MRAM | no practical limit | decades |

A save every 10 ms is 8.6 million saves a day, about 138 GB a day. One save a second is about 1.4
GB a day, 47 years on the eMMC counting the save's own bytes alone. Two copies on that one drive
halve it, and a flash drive writes several times the bytes it is handed for a small write that
must reach the drive at once (the file system's own records, whole flash pages). That factor is
not measured: with an assumed factor of 5 the 47 years become under 5, and a save every ten
seconds brings them back. So flash cannot carry a save every cycle for decades, and the period a
target can afford is measured on the target, never taken from arithmetic.

### Retained variables, by what the target can do
- **Memory that keeps its content without power:** a save every cycle, by the rule on temporary
  and permanent data above.
- **Hold-up power with a power-fail signal:** a save at power loss, as in the operator's "small
  battery that holds 2 cycles (to be able to save the state before shutdown)".
- **Neither** (most small boards), ruled on 2026-11-16, "Last save, flagged":
  - the controller saves the retained variables at an orderly stop and at a period the engineer
    sets;
  - a save is never left half-written;
  - after a power cut the controller starts from the last save and tells the program through a
    flag in the start-up handler; setpoints and counters survive, and the engineer's logic decides
    what to re-initialise, the step of a sequence for one;
  - such a target is accepted: resetting every retained variable, or refusing the target for a
    program with retained variables, were the answers not taken.

### A write from outside is saved at once
Ruled on 2026-11-16 ("Yes, saved at once"), for a target that starts from the last save:
- the controller saves at the end of the cycle that took a write from outside the program to a
  retained variable;
- never sooner than the shortest gap after the previous save, so a system that writes every second
  is held to that gap;
- it covers every write that passes the switch and whitelist of the controller's embedded portal:
  Oficina's variable write, an operator's panel, a supervisory system.

### The store of a controller's embedded portal
Ruled on 2026-11-19 ("Memory window"):
- by default the portal embedded in a controller keeps its recorded values in memory, as a fixed
  window: the last stretch of values, as long as the project sets;
- it writes nothing to the drive, so recording causes no wear; a power cut empties the window.

Given with that answer as the assistant's two additions, and not objected to:
- the project can turn on recording to the drive, in files with a size limit;
- at a fault the controller writes the window to the drive with the fault record, so whoever comes
  later sees the minutes before the fault.

What gives way when a record on a controller's drive reaches its size limit was the question in
hand when the round paused. The assistant's "oldest deleted first" departs from Q16, where the
operator decided for a project's disk store that "nothing is refused, deleted or merged"; it is
Q41.

## The Linux study
Owed before any rule on the operating system is written: "we need to study linux for rt
applications, i suppose recovery mechanisms exist. otherwise we build our own: 2 partitions? whole
os saved on successful boot? idk, but: if existing tested methods exist, we use them. and I'm
positive there's plenty" (2026-11-16). An existing, tested method wins over one of our own.

Its list so far, named by the assistant from memory and not verified:
- two system partitions with boot counting and automatic fall-back (RAUC, SWUpdate, Mender);
- a read-only system partition under dm-verity, with its error-correcting data;
- Btrfs or ZFS mirroring, with a checksum per block and a scheduled repair run, which is the
  program store ruled above;
- real time in the mainline kernel since 6.12;
- MRAM or FRAM for data written every cycle, and capacitors in place of a battery;
- file-system timestamps that reach past 2038 (ext4 with small inodes, and XFS without its
  larger-timestamp option, stop there);
- NTP's own 32-bit count of seconds, which starts over on 7 February 2036: the client chosen must
  handle it;
- the hardware watchdog, since a frozen operating system reports no cycle time;
- how the clock chip's power-lost mark, and a low-battery report where the chip has one, are read
  from Linux;
- the operating system's own writing to the drive (its logs, its temporary files);
- how a drive's wear is read (eMMC and SSDs report it; most SD cards do not).

## What is owed beside this round
- **A suite-wide cyber-security round**, called by the operator on 2026-11-16: "dedicated round to
  decide cyber security level suite-wise, future proof is the best way to design". Its open items
  are P-014 and Q43. A hash guards against corruption, not against a person: signatures and keyed
  checks belong to that round.
- **The specification of what disqualifies an edit from a live takeover** (Q32).
