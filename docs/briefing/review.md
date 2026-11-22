# Veredas briefing: the adversarial review, merged

**A record, kept as it was delivered on 15 November 2026.** Five reviewers attacked the briefing's
deliverables list (`docs/briefing/deliverables.md`, items A1.1 to K12), and the assistant merged
their findings into the 55 entries below for the architect. In this text "I" and "my pick" are the
assistant's, "you" and "your choice" the architect's. It is not updated, with one addition: the
table of the architect's rulings on the 14 blockers, placed under "Blockers". The 32 majors and the
9 minors are not ruled (`docs/questions.md` Q31). The current word on everything decided is in
`docs/oficina-plan.md`, `docs/controlador-plan.md` and the component plans.

The review changed nothing in the repository and nothing in the decisions.

## The result in short

Five reviewers attacked the list, one lens each: operating safety, consistency with the suite's own
rules, feasibility and cost, users and product, security and legal exposure. They returned 62
findings. Merged where several said the same thing, and split where one finding needs two
decisions, they come to 55 entries: **14 blockers, 32 majors, 9 minors**.

A blocker here means the plans cannot be written until it is decided. Entry 14 blocks only the
first public build.

| Where the blockers sit | Entries |
|---|---|
| Who may command the controller, and through which gate | 1 to 4 |
| What the controller does when the person or the device goes away | 5 to 9 |
| Where the pieces live and what they stand on | 10 to 13 |
| Before anything is published | 14 |

Four things to know before reading on:

- **One blocker is my own error.** The rule that a fallback "takes over from its own watcher data"
  was my recommendation, and the safety review shows it is unsafe as written (entry 5). A second,
  smaller error of mine is in the state text (entry 18).
- **Twenty-one entries touch a choice you made knowingly.** Each says what the review adds that you
  did not have when you chose. Where it adds nothing, the finding was dropped.
- **I rated three of the users reviewer's blockers as majors** (entries 40 to 42), because each
  attacks a deliberate choice about what the product is. You may weigh them differently.
- **Eleven findings fell away.** Six were already answered by a decision or a rule the reviewers
  could not see, and five rested on a claim that did not hold when I checked it. They are listed
  near the end, with what my list had left out.

How to read an entry:

- **Items**: the numbers in the deliverables list. **From**: the reviews that raised it.
- **Standing**: *gap* (never decided), *contradiction* (two decisions disagree, or one disagrees
  with a rule already in the repository), *your choice* (decided knowingly; the entry says what is
  new), *my error*.
- **Options**, then **My pick**.

---

## Blockers

### The architect's rulings (added to this record)

All fourteen were ruled on 2026-11-15, in six rounds. The entries below are as delivered: "my pick"
in them is what the assistant proposed, not what was ruled.

| Entry | Ruling |
|---|---|
| 1 | The commissioning switch covers every engineering change: download, swap, accept, rollback, cold start, reset of retained data, hardware configuration and device naming join writes, forcing, breakpoints and starting. Only reading and stopping are free. The peer-write setting is the embedded portal's master switch and whitelist; controlador has no second setting. |
| 2 | Falls away: "peers, intended as other controllers we controlador interacts with, shall never be able to change its state". Only a person changes the state, in Oficina or at the target, and the program may stop itself. |
| 3 | The switch is a "file on controlador's OS, not writable from outside". |
| 4 | All three rules: a protocol is bound only when the project names the interface; OPC UA writes need a signed and encrypted channel and a user; Modbus writes come only from listed client addresses. |
| 5 | "live fallback shall be enabled only when feasible. we need to build the specification for what disqualifies the edits for auto takeover. takeover after restart remains possible for all cases". The specification is open (Q32). |
| 6 | (a) and (d): each target has a longest time outputs may stay held at a halt, short by default, after which they go safe and the controller stays halted; a force belongs to the controller until someone removes it. On continue after a halt, "Nothing missed runs". |
| 7 | "Always as before": no start-up parameter, no STOP forced by a commissioning condition, no rule that ends a shadow test. A dry run survives a power cycle; HOLD comes back as STOP, as STOP and FAULT do. |
| 8 | Only (c), outputs wait for release. A loss leaves an error the program acknowledges with a call; a program with no handler for it has the error acknowledged after one full scan on valid inputs. From the device's return until the acknowledge its inputs are live and its outputs withheld. |
| 9 | (a) to (d), as three rules; "only through a target" was not chosen, so the laptop IO check stays. |
| 10 | "Core, rules amended": the masters stay in core, Q9 is reworded, and "testing only" stays a rule in prose that no law checks. Where their tests against the bench live is open (Q34). |
| 11 | "controlador embeds a full version of portal". Controlador owns the real-time communication; its embedded portal reads its memory asynchronously; Oficina embeds a portal too, and the two speak portal's own HTTP interface (ruled 2026-11-16). |
| 12 | "rustc_public is ok as is / proceed as decided": no spike with gates. |
| 13 | Only (b), a smaller first release: no hardware interrupts, and the shadow test only for an application with one cyclic task. Where the execution model is written, and K8 and K10, are open (Q33). |
| 14 | A permissive licence, MIT or Apache 2.0, for every crate Oficina links, and a check of dependency licences in CI. Which expression is open (Q35). |

### Who may command the controller

**1. No table says which gate covers which action, and the three gates disagree**

- Items: B2.5, B7.3, B7.4, B1.6, D1, A6.3. From: consistency, security, safety. Standing:
  contradiction.
- What is wrong: there are three gates. The commissioning switch (B2.5) says starting needs it. The
  peer-write setting (B7.3) is "separate from the commissioning switch", and B7.4 lets a peer start
  the controller under it, so a Modbus client starts a controller whose switch is off. Portal's
  rule, already in the repository, says every write path obeys one master switch (`writes_armed`)
  and a whitelist, yet the embedded portal's new servers would obey the peer-write setting instead.
- The switch's own list names writes, forcing and breakpoints. Nothing gates a download, a swap, an
  accept, a rollback, a cold start, a reset of retained data, a hardware-configuration download,
  the clearing of a maintenance request or the naming of a device. A rollback to one of the last
  ten downloads is then a way to put back a version with a known fault.
- Options:
  - (a) One closed rule: every action that changes the machine needs the armed switch, whatever its
    source, and only stopping is free. The peer-write setting becomes portal's existing master
    switch and whitelist in the embedded project, and a peer's action needs both gates.
  - (b) Three named gates and a table of action against gate; portal's rule is reworded to "at
    least the switch".
- My pick: (a), written as a table in controlador's plan, with "RUN returns as RUN after a power
  cycle" named in it as the one start no person makes.

**2. A peer can start a machine that a person stopped**

- Items: B7.4, B5.4, B2.5. From: safety, security, consistency. Standing: your choice (peers may
  start and stop). New: a stop has no owner.
- Scenario: a fitter stops the machine from Oficina to clear a jam, without pressing the emergency
  stop. A line controller or a SCADA panel writes "start" over Modbus or OPC UA. The controller
  goes to RUN with a person inside the guard.
- Options:
  - (a) A stop remembers its source. A stop made from Oficina or at the target is undone only from
    Oficina or at the target; a peer restarts only what a peer stopped.
  - (b) Peers may stop and never start.
  - (c) A peer's start is a request that a person confirms at the target.
- My pick: (a). It keeps what you wanted, a line controller that stops and restarts a station, and
  closes the scenario. The plan should also separate two things the word "start" hides: starting
  the machine is the program's business (a variable marked writable); starting the controller is a
  change of mode, and only the second is this entry.

**3. The switch can be set over the network, through Oficina's own path**

- Items: B2.5, A1.9, A2.2, A2.4. From: security. Standing: gap.
- Scenario: Oficina edits every TOML file on a target (A2.2) and restarts executables over SSH
  (A1.9). So: open the target's configuration, arm the switch, restart controlador, force outputs.
  "Never over the network" holds for controlador's own protocol only; the file that holds the
  switch travels over SSH. The switch guards against a mistaken click, not against a person or a
  program that holds an SSH session.
- Options:
  - (a) The switch is hardware: a key switch or a digital input named in the configuration, as the
    mode switch on a PLC is.
  - (b) The switch lives in a file the SSH user cannot write and Oficina refuses to open or copy.
  - (c) B2.5 is restated as a guard against mistakes, and real access control becomes the first
    item of the security study (J4).
- My pick: (a) where the target has an input for it and (b) elsewhere, the plan saying which a
  target uses and that (b) guards against mistakes only.

**4. The two default protocols are open to anyone who can reach the port**

- Items: B7.1 to B7.5, D1. From: security. Standing: gap inside a choice of yours.
- What is wrong: Modbus/TCP has no authentication. Nothing states OPC UA's security mode, so the
  cheapest build is anonymous and unencrypted. Anyone who can route to port 502 or 4840 then writes
  every writable variable and, with the peer setting on, starts and stops the controller. The fixed
  state block (B7.5) lets one Modbus read identify every reachable controlador and tell what it is
  doing.
- "By default" in your decision named the protocols every controlador speaks. What a fresh
  controller listens on was never asked.
- Options:
  - (a) Both are built in, and a fresh project binds neither to a plant interface until the project
    names one.
  - (b) Over OPC UA, a write or a start needs a signed and encrypted channel and a user; no
    anonymous write.
  - (c) Modbus is read-only unless the project lists the client addresses allowed to write.
  - (d) Modbus is read-only always.
- My pick: (a), (b) and (c) together.

### When the person or the device goes away

**5. The fallback hands the machine to a version whose picture of it may be stale**

- Items: B3.2, B3.6, B4.1, B4.3, B5.5. From: safety, consistency. Standing: my error.
- Scenario: an edit to a sequence is swapped in with the fallback armed. The new version drives for
  twenty minutes and takes the mechanism from step 3 to step 9. The old version computed on the
  same inputs all that time, but its outputs never reached the machine. Where the two behaved
  alike, its steps and latches followed the machine. Where they did not, its timeouts fired and its
  steps went elsewhere. When the new version then faults, the old one takes over "from its own
  watcher data" and commands from whatever it believes.
- The list also contradicts itself here. B3.2 says a version taking over continues from the
  machine's real data; B3.6 says from its own. B4.1 says the whole controller faults and B4.3 that
  leaving a fault takes an acknowledge; the fallback is a fault with neither. The armed flag and
  its running time limit have no term in the state text, and nothing says whether the failed
  version leaves a fault record.
- Options:
  - (a) No live takeover. The controller faults as B4.1 says, outputs safe, and the previous
    version is put back for the next start: acknowledge, then start.
  - (b) Live takeover only while the watcher has stayed in step: from the swap to the fault, the
    comparison showed no difference between its outputs and the driven ones. A first difference
    ends its right to take over (the test goes on and the state says so), and a fault is then
    handled as in (a).
  - (c) As decided.
- My pick: (b) if keeping production through a faulty edit is worth a clause that will take many
  refute rounds, else (a). (b) still covers the case you described, an edit that faults long after
  the swap on a path it had not taken before. With either option: the failed version's fault record
  is written; the state text gains a term for the armed fallback and its time left; and the event
  is named "the edit failed", not a controller fault, so B4.1 stays true.

**6. A halt, a lost laptop and the forces: three rules that pull apart**

- Items: B2.2, B2.3, B2.4, K3, K9. From: safety, users, consistency. Standing: your choices (an
  output may be held at a halt; a halted target stays halted; forces end with the session). New:
  the three were never set against each other.
- What goes wrong:
  - *Held outputs with no clock.* A fill valve is set to hold at a halt. A breakpoint hits in an
    unrelated block and every task halts. The valve stays open, the program's "longest fill time"
    timer is frozen with the program's clock, and the interlock in the cyclic interrupt does not
    run. Nothing bounds the time spent in HOLD.
  - *The laptop goes away.* It sleeps, or walks out of Wi-Fi range. The target stays halted (B2.3),
    outputs held, nobody in charge.
  - *Forces released by a network drop.* "Forces end with the session" (B2.2) makes a lost
    connection a command: a forced "guard closed" snaps back to its real value in the middle of a
    sequence. At a halt it is undecided whether a held forced output drops at that moment.
    "Session" itself is defined nowhere.
  - *What fires on continue* (K9, open): time-of-day and time-delay blocks that came due, and
    hardware interrupts that arrived during the halt.
- Options for held outputs:
  - (a) A longest time an output may stay held at a halt, set per target and short by default. When
    it runs out the held outputs go to safe and the controller stays in HOLD.
  - (b) A breakpoint halts only its own block, and more urgent blocks keep running on the real
    clock.
  - (c) As decided.
- Options for forces:
  - (d) Forces belong to the controller, not to the connection. They stay until someone removes
    them, they show in the state text, and a restart clears them. Siemens and Rockwell both keep
    forces when the programming device disconnects.
  - (e) Forces end a few seconds after the connection is lost.
  - (f) As decided.
- My pick: (a) and (d). With (a) the unattended halt needs no rule of its own. For K9: a
  time-of-day block missed during a halt runs once on continue; cyclic blocks never catch up;
  time-delay blocks count on the program's clock and simply resume; a hardware interrupt keeps the
  newest event per source.

**7. After a power cycle, nothing says which conditions come back**

- Items: B1.6, B1.5, B2.4, B3.6, B5.5, K7. From: safety. Standing: gap.
- Scenario: a controller in `RUN dry()`, every output safe on purpose, loses power. It comes back
  as RUN. If the dry run is not kept, a machine that was deliberately inert drives its outputs on
  power return, from a sequence that advanced while its outputs were safe. The same question stands
  for an unaccepted shadow test (which version loads?), for the fallback flag (reset on shutdown,
  so the new version then runs unguarded) and for forces.
- Also missing: Siemens makes this a parameter of the CPU, "Startup after POWER ON", with three
  choices: stay in STOP, warm restart to RUN, warm restart to the mode before power off. Your
  decision is its default; the parameter itself is not in the list. Automatic RUN on power return
  is then the only behaviour, and the rule against unexpected restart (EN 60204-1) rests on the
  program alone.
- Options:
  - (a) The Siemens parameter with its three choices, default "the mode before power off".
  - (b) A controller that had any commissioning condition when power went (dry run, forces,
    breakpoints, an unaccepted shadow test) comes back in STOP, the reason saying what it was.
  - (c) An unaccepted shadow test always restores the last accepted download.
- My pick: all three. K7 stays a reading: a kernel crash that reboots the machine looks like a
  power cut, and telling them apart depends on the hardware.

**8. A device that comes back is driven at once from outputs computed while it was away**

- Items: B1.5, B4.6, B1.4. From: safety. Standing: gap.
- Scenario: a remote IO station is missing at start-up and the controller goes to RUN regardless
  (B1.5). The program, reading that station's inputs as invalid, sets its outputs all the same:
  extend the cylinder. A technician is at the station plugging the cable back in. On the first
  exchange the waiting output image is applied and the cylinder moves under their hands.
- Stated nowhere: what an invalid input reads as (zero, its last value, a substitute), and what a
  returning device receives when the program has no handler for it.
- Options:
  - (a) An invalid input reads as zero or false, and each device has a validity flag the program
    can read.
  - (b) Each device says what its loss does: carry on (the default, as decided), stop, or fault.
  - (c) A returning device's outputs stay safe until the program releases the device from its
    device-returned handler; a program without that handler gets them back after one full scan on
    valid inputs.
- My pick: all three.

**9. The IO check from a laptop drives real outputs through no gate, and naming a device is an
ungated write**

- Items: B6.3, A6.4, G2, B1.1, B2.5. From: safety, consistency, security. Standing: gap inside a
  design of yours.
- What is wrong:
  - With no target set, the session runs on the laptop, and the switch "in the target's own
    configuration" is then the laptop's own: the person who wants the output on is the person who
    sets it.
  - The laptop runs Windows, without real time, driving a real module. B1.1 gives Windows two uses,
    development and simulation; this is a third.
  - If the laptop freezes with an output on, a PROFINET device falls back when its watchdog runs
    out. Modbus TCP has no watchdog in the protocol: the coil stays on unless the module has a
    timeout of its own. "Never above 2 s" is a promise about the laptop, not about the device.
  - Modbus accepts several clients, so the laptop's writes can fight a production controller's on
    the same module.
  - "Identify, name and address" is a PROFINET DCP Set. Renaming a device that a running controller
    holds drops its connection and can stop a line.
- Options:
  - (a) The laptop session has an arming step of its own in Oficina, per session, naming the
    device.
  - (b) It refuses a device that is exchanging data with another controller; on Modbus it reads
    back the module's own timeout and refuses when there is none or it is too long.
  - (c) Outputs are momentary, on only while the control is held; latching one is a separate,
    explicit choice.
  - (d) Naming and addressing are refused for a device with a live connection unless armed and
    confirmed.
  - (e) IO check and naming run only through a Linux target.
- My pick: (a) to (d), and B1.1 gains the laptop IO check as Windows' third use. Two side notes.
  The Modbus point holds in production too: a crashed controller leaves Modbus outputs as they
  were, so the hardware configuration should check the module's timeout. And on Windows raw frames
  need Npcap, whose free licence I believe forbids bundling it; to check.

### Where the pieces live and what they stand on

**10. The network masters in core break three of the repository's rules**

- Items: C2, C3, C5, B6.1, E2. From: consistency, feasibility, security. Standing: your choice
  ("the hardware driving functions can live in veredas core"). New: the three conflicts.
- What is wrong:
  - Q9, decided: "core holds no frame layout". PROFINET, EtherCAT and CIP codecs are fixed layouts
    in code.
  - "Callable by other crates for testing only" cannot be enforced. Law L5 lets every member use
    core, so ábaco or supervisão would carry a fieldbus master and pass every law.
  - Law L4 forbids core from depending on another member in any table, the test table included.
    Core's masters therefore cannot be tested against the bench devices in maquinar (E2). Either
    the device emulations are written twice, or the masters are proven from another crate.
  - Cost: every member compiles the raw-socket dependencies, core's version follows two crates that
    are not yet at 1.0, and every run recorded against core goes stale.
- Options:
  - (a) The fieldbus stacks get a member of their own (a name such as `veredas-campo`), with a row
    in the dependency map naming who may use it: controlador, maquinar, aranha. "Testing only" is
    then law L5, its tests may use the bench as portal's do, and core stays as it is.
  - (b) They stay in core behind a cargo feature, with a new law that only named members switch it
    on; the tests still live outside core.
  - (c) They stay in core and Q9 is amended.
- My pick: (a). It gives what you asked for, one place that is safe in controlador and usable by
  the others to test, and it makes the rule one the laws can check. Aranha's codecs and the readers
  of description files (C5, open) go there instead of into core.

**11. Controlador's embedded portal has no store and no server, and Oficina's channel to the
controller belongs to nobody**

- Items: D1, D2, B7.1, A1.9, B2.1 to B2.4. From: consistency. Standing: contradiction with the
  portal plan.
- What is wrong: the portal plan defines the build that controlador embeds as the one "without the
  default features", which "leaves the store and the server out". D2 stores state changes and
  faults "like any other data", and D1 and B7.1 serve OPC UA and Modbus from that same embedded
  portal.
- Separately, no item delivers the protocol through which Oficina reads live values, forces, sets
  breakpoints and downloads. It is not portal's HTTP interface, which serves stored series, and not
  OPC UA, which publishes marked variables only.
- Options:
  - (a) The embedded build gains the store and the server, and the plan says what "light" still
    means.
  - (b) The embedded build stays light. The controller keeps its own bounded records as files
    (fault records, download history, a ring of state changes), and the long history is an ordinary
    portal session that reads the controller over OPC UA as it reads any plant device.
  - (c) The engineering channel is controlador's own, named in the plan as a third exception to
    "every component exchanges data through portal", beside the test tools and the real-time path.
- My pick: (b) and (c), with the two plant-protocol servers as a feature of the light build that is
  separate from portal's HTTP server.

**12. The interpreter's front end stands on an interface the compiler does not publish**

- Items: B1.2, B1.3, J2, A6.3, B8.1. From: feasibility, consistency. Standing: your choice (an
  interpreter fed from the compiler's intermediate form, against my recommendation). New: the facts
  below.
- What is wrong:
  - The public interface to that intermediate form, `rustc_public`, is not on crates.io (checked
    today). "One pinned compiler version" therefore means a pinned nightly using the compiler's
    internal crates, on every user's machine and in CI. The workspace pins no toolchain today.
  - The core library ships no intermediate form for most of its functions. The interpreter needs
    its own build of the standard library, or a hand-written list of core operations. J2's "list of
    core-library operations" is that work, not a study.
  - That form also carries generics to be resolved, drops, panics, trait objects, closures and the
    overflow checks; and the conformance suite goes stale each time the pin moves.
  - A ladder view of the same source needs the source's structure, so it needs a second,
    source-level front end whatever is decided.
- Options:
  - (a) One source-level front end shared by the interpreter, the ladder view and the SCL
    translators, proven by differential tests against natively compiled runs.
  - (b) The intermediate form is kept, with the pinned nightly and an own build of the standard
    library; ladder gets a front end of its own.
  - (c) Programs are compiled natively in production and interpreted only in simulation, which
    gives up breakpoints and the shadow test on the real target.
- My pick: keep your choice as the hypothesis and turn J2 from a study into a spike with pass or
  fail gates, before the plan commits. The gates: the pinned toolchain builds on both systems and
  in CI; a small program using orcl-std's timers lowers with nothing from core beyond a written
  list; type and layout data for the carry by name and type comes out; one move of the pin is
  timed. (a) is the named fallback if a gate fails.

**13. The controller's execution model is not written, and four decisions multiply on it**

- Items: B1.4, B2.3, B3.1 to B3.6, B4.5, K8, K10, K11. From: feasibility, safety. Standing: gap (K8
  is my reading, K10 and K11 were listed open), under your choice of one release.
- What is wrong:
  - How one block interrupts another inside an interpreter is not chosen: one real-time thread per
    priority with an interpreter that can be re-entered, or switching at instruction boundaries on
    one thread.
  - Whether a value of several words can be read half-written depends on K8 and K10, which nobody
    decided.
  - The shadow test doubles every block at every priority. "The doubled scan fits the period"
    (B3.4) has no meaning yet for blocks that interrupt each other (K11).
  - A breakpoint halts every task while the fieldbus keeps exchanging, so the bus cycle needs a
    real-time thread of its own outside the interpreter.
  - For overruns, a free-running main block starved by more urgent ones has no stated period.
- Options:
  - (a) The execution model is the first section of controlador's plan, written before any other,
    and answers K8, K10 and K11. For K11: at each priority the two versions' measured worst-case
    times together fit the period, otherwise the shadow test is refused.
  - (b) The first release drops hardware interrupts and allows the shadow test only for an
    application with one cyclic task.
  - (c) One release is kept, built and proven on hardware in this order: execution and faults,
    debugging, a first bus, then the shadow test.
- My pick: (a) and (c).

### Before anything is published

**14. A GPLv3 Oficina cannot be given to anyone while the crates it links have no licence**

- Items: A1.11, I6, B6.1, E3. From: security and legal, consistency. Standing: contradiction inside
  one decision.
- What is wrong: KDDockWidgets is offered under GPL 2 only, GPL 3 only or a commercial licence, and
  Qt Quick 3D under GPL 3 or a commercial licence. Oficina and everything linked into it must then
  be conveyed under GPL 3, with source. Core with "no licence stated" is all rights reserved and
  cannot be part of that. The licences of the EtherNet/IP crate to be forked and of the physics
  engine were never checked. And I6's "a licence file" at the root would cover the whole
  repository, where A1.11 means Oficina alone.
- Options:
  - (a) Every crate Oficina links gets a GPL 3 compatible licence before the first build leaves
    your machine: a permissive one (MIT or Apache 2.0), which also lets a machine builder embed
    controlador, or GPL 3 itself.
  - (b) Oficina reaches core only across a process boundary; weaker, and it needs qualified advice.
  - (c) A check of dependency licences runs in CI.
- My pick: (a) and (c). This is a reading, not legal advice; an hour with someone qualified before
  the first public build is worth it.

---

## Majors

### Online change

**15. A swap with a different IO mapping leaves outputs without an owner, and rollback has no
stated path**

- Items: B3.2, B3.3, K4, A6.3. From: safety, security. Standing: gap.
- The new version moves a valve from one output to another, which B3.3 allows. At the swap the old
  channel has no writer: nothing says whether it keeps its last value or goes to safe. The
  comparison per physical channel (K4) showed two unrelated differences instead of "one signal
  moved". Rollback to an earlier download is the same change in reverse and is not said to go
  through the shadow test at all.
- Options: (a) An output that loses its writer at a swap goes to safe, and the popup before the
  swap lists each one for confirmation. (b) The comparison shows a moved signal as one line. (c) A
  rollback is a download like any other, through the shadow test or through a stop, never a direct
  replacement.
- My pick: all three.

**16. A single cycle pulses every output, and leaving a dry run makes outputs jump**

- Items: B2.4, B2.3. From: safety. Standing: gap.
- A single cycle runs one scan and halts, and outputs at a halt default to safe. Each click
  therefore energises every output written in that scan for a few milliseconds: contactors, valves,
  drive enables, brakes. And when a dry run is switched off in RUN, every output takes the
  program's value from the middle of a sequence at once.
- Options: (a) On a real target a single cycle runs only in a dry run, or with outputs held between
  cycles after a confirmation that says so. (b) Leaving a dry run goes through STOP and a start.
- My pick: both.

### The state text

**17. One name means two things depending on where it stands**

- Items: B5.1, B5.4, B5.5, B5.7, K5. From: consistency, users. Standing: gap in terms I proposed;
  two of the clashes are in your own examples.
- `devices(up, of)` is a start-up phase and `devices(missing)` a condition, so a start-up with a
  device missing writes two `devices` terms. `breakpoint(id, at)` is a reason and `breakpoint(set)`
  a condition, so a halt at one breakpoint with two set writes the name twice, or drops the count
  as the example in B5.7 does. `handler` is a phase and a fault reason. Phases are a third kind of
  term with no place in "one reason, then conditions".
- Options: (a) Two changes: the condition becomes `breakpoints(set=2)`, and the phase becomes
  arguments of the start kind, `STARTUP warm(id=201, phase="devices", up=3, of=5)`. Then every name
  has one meaning. (b) The names stay, with the rule that the term carrying `id` is the reason.
- My pick: (a).

**18. The spelling rules are incomplete, and one of my readings is wrong**

- Items: B5.2, B5.5, B5.7, K2. From: consistency. Standing: gap, and my error in K2.
- K2 says every value that is not a number is quoted, yet the examples write `urgent=true`. B5.5
  lists `dry` and `sim` bare where the examples write `sim()`. "TOML's rules" also allow
  single-quoted strings, escapes and spaces around `=`, and nothing says which of those are valid
  here. A strict reader refuses one of each pair.
- Proposal, to be pinned by a test in core: integers and booleans bare; strings in double quotes
  with TOML's escapes; every term has parentheses, empty or not; a comma and a space between
  arguments and no space around `=`; one space between terms.

**19. `maintenance_request(id=…)` cannot tell two requests apart**

- Items: B5.5, B5.6, K6. From: consistency, users. Standing: gap.
- Everywhere else `id` is a reason's number; here it is the maintenance reason's number, 1 to 6,
  which collides with the state numbers 1 to 5. Two missing devices give two identical terms, and
  "cleared from Oficina" cannot say which one.
- Options: (a) The maintenance reasons become 601 to 606, so every `id` in the text is unique. One
  term per reason carries a count, which keeps your rule of counts and never lists:
  `maintenance_request(id=603, reason="device_missing", urgent=true, count=2)`. The detail view
  lists the single requests, each clearable. (b) Each request gets a number of its own in the text.
- My pick: (a).

**20. The state text is a wire format with no version and no rule for a term a reader does not
know**

- Items: B5.1, B5.3, B7.5, D2. From: consistency. Standing: gap.
- The text is published over OPC UA and Modbus and stored by portal, so by the suite's own
  versioning rule a change to it is a breaking change. It carries no version. A condition added
  later (entry 5 adds one) shifts the fixed order for every older reader, and "refuse, never guess"
  would make an older Oficina refuse a newer controller. The fixed register block (B7.5) cannot
  hold strings or several requests.
- Options: (a) A version number is published beside the text, not inside it. A reader keeps a term
  it does not know as opaque text and says so. (b) The register block gets a written layout: state
  number, reason number, one bit per condition, the counts, the number of maintenance requests and
  whether any is urgent. Strings travel over OPC UA only.
- My pick: both.

**21. "Debug condition" is never defined, and the health value hides urgent requests behind it**

- Items: B4.5, B4.7, B5.5, B5.8. From: consistency, safety. Standing: gap.
- B5.8 gives "function check" for any debug condition and B4.7 clears every debug condition at a
  fault, but no list says which conditions those are: is `sim()` one? During weeks of commissioning
  one force or one breakpoint is always there, so health reads "function check" throughout and an
  urgent `device_missing` never reaches supervisão or a peer as a maintenance signal.
- Three smaller points on overruns. The list reads as if the time-error handler ran once ever,
  where Siemens runs it at each scan that overruns. Nothing says what a first overrun does when the
  program has no time-error handler; on Siemens the CPU stops. And a scan may sit between once and
  twice its period for ever with only a non-urgent request.
- Options: (a) Each condition is tagged in the table: commissioning (breakpoint, force, shadow,
  dry, sim, no_realtime) or trouble (devices, overruns, retain). A fault clears breakpoints and
  forces and ends a shadow test. (b) The single health value keeps NE 107's order, and the count of
  maintenance requests and an "urgent" flag are published beside it, so nothing is hidden. (c) A
  number of overrunning scans in a row, set per target, raises an urgent request.
- My pick: all three; the handler runs at every overrunning scan; an overrun with no handler faults
  the controller, as on Siemens.

### Where the pieces live, continued

**22. Oficina starting portal sessions collides with Q19, and SQL over the store has no route**

- Items: A1.9, A3.5, A3.7, A4.3. From: consistency. Standing: contradiction with Q19.
- Q19 decided that supervisão's host starts the sessions, one executable serving all of them. If
  Oficina also starts `veredas-portal` for the same project, two sessions poll one PLC and write
  two stores. And DuckDB lets one process write a file or several read it, never both, so Oficina
  cannot open a store beside the session that writes it.
- Options: (a) Oficina asks supervisão's host to start and stop sessions, which moves the host's
  start and stop interface forward. (b) Oficina owns the sessions and supervisão reaches them over
  HTTP, Q19's reversible branch. (c) SQL goes through a read-only route in portal, or runs on a
  snapshot copy.
- My pick: (a) and (c).

**23. Vendor description files in the project folder**

- Items: B6.2, C5. From: consistency, security. Standing: your choice (copied into the project,
  read directly). New: four loose ends.
- They are plain-text project files that are not TOML, against the plan's rule. Where their readers
  live is open (C5), and core names no vendor. The files are the vendors' copyright, so they cannot
  sit in a public repository or its fixtures. And they are XML from outside: a careless parser
  reads local files through external entities.
- Options: (a) They stay in the project as inputs that are read and never written, and the plan's
  TOML rule names them as its one exception. (b) They are imported once into a Veredas device file
  and not kept.
- My pick: (a), with the readers beside the fieldbus stacks (entry 10), parsing with document type
  definitions and entities switched off, and test fixtures that are descriptions we write
  ourselves.

**24. A colour computed by an expression can show a state with no mark and no word**

- Items: F1, F2, F3, F5, K12. From: consistency. Standing: your choice (colour, size, position and
  visibility as expressions). New: the clash with the design rule.
- Supervisão's rule is that every state has a mark and a word, so it reads without colour, within
  four state classes. An expression that turns an object red bypasses both. Nothing says where an
  expression ends and a script begins. F3's edit mode in the browser moves a pending item (P-012,
  "later") forward without saying so. And the syntax palette's colour for strings is a green,
  `#7EE787`, while green now means a good state; I5 moves only the number colour.
- Options: (a) Expressions compute size, position and visibility; colour stays with the state
  classes. (b) A colour expression must resolve either to a state class, which brings its mark and
  word, or to a declared scale with a legend, such as a temperature.
- My pick: (b), and the string colour moves with the number colour. K12 (is "good" a new class or
  the normal one's colour) still needs your word.

**25. Section I of the list misses repository changes the laws require**

- Items: I1, I3, I4, E1, H1, A6.6, B1.2. From: consistency. Standing: gap; bookkeeping, with one
  decision.
- The dependency map holds members only: core's outside dependencies are not rows, and Oficina's
  own row was never written out.
- Law L6 wants a plan before code. `docs/orcl/README.md` for the interpreter's family and
  `docs/controlador-plan.md` are not listed, nor the edits to the plans of core, portal, maquinar
  and aranha.
- `fisica` and `reflexo` can only be modules inside maquinar: as crates, L1 refuses them nested and
  L2 refuses them without the prefix.
- The HMI creator (A6.6) is code for a member that has no name and no plan (H1).
- The decision: `veredas-controller` is already the format name of maquinar's simulated-controller
  file, so controlador's own files need names that do not clash.

### Feasibility and order of work

**26. PROFINET first**

- Items: B6.1, C3, E2, J3. From: feasibility. Standing: your choice. New: what exists, and what
  would prove it.
- Veredas 1's aranha holds discovery (DCP and LLDP) and a read of a device's nameplate that, by its
  own header, was never checked against a real device. The cyclic exchange, the connection
  sequence, alarms and record data are all new work, the specification sits behind the membership,
  and no open controller exists to compare with. A bench device written from the same reading of
  the specification proves little.
- Options: (a) Modbus TCP becomes the bus on which the runtime is first proven, while the
  membership runs; PROFINET stays the first fieldbus written. (b) From its first clause the
  PROFINET work has a real device in the loop and captures of a real controller's traffic as
  fixtures. (c) PROFINET moves to a second release.
- My pick: (a) and (b).

**27. The EtherCAT bench device is a slave controller written in software, and the EtherNet/IP fork
becomes ours to keep**

- Items: E2, B6.1. From: feasibility, legal. Standing: gap.
- A real EtherCAT device handles frames in a chip as they pass. Emulating one on an ordinary
  network card means writing that chip's behaviour: register map, sync managers, the state machine,
  the mailbox, the counter rules. It can be functional, never exact in time, and no open emulator
  exists. Forking a young EtherNet/IP crate means owning it. (ethercrab itself is at 0.7.1,
  released in March 2026: before 1.0, with one main maintainer.)
- Options: (a) The emulator covers only what the master exercises, and its clause says so; the
  repository's fixture-first rule needs some emulator. (b) Real terminals are in the loop from the
  start. (c) EtherNet/IP moves to a later release.
- My pick: (a) and (b).

**28. The ORCL editor needs two clients Monaco does not have**

- Items: A6.3, A6.1, A1.5. From: feasibility. Standing: gap.
- Monaco is an editor only. It has no client for a language server and none for a debugger; both
  belong to VS Code. Oficina has to supply them, with the debugger's interface (breakpoints in the
  margin, call stack, variables, stepping). Ábaco's Python debugger needs the same interface. And
  the plain editor that replaces Monaco where Chromium cannot draw loses all of it, on exactly the
  virtual machines A1.4 promises.
- Options: (a) One language-server client and one debugger client, written once on Oficina's side
  and shared by ORCL and Python. (b) The first debugger ships without inline values. (c) A native
  code editor replaces Monaco if the spike fails.
- My pick: (a), decided now so that ábaco does not write a second one.

**29. The test rule meets an embedded browser, and each interface clause is expensive**

- Items: A1.10, A1.5, A1.6, J1. From: feasibility. Standing: your choice (every clause's test
  drives the real interface). New: the second driver, and the cost on record.
- Spix drives QML items. It cannot see inside the web page, where Monaco and supervisão's tabs
  live. Panes floated to a second monitor cannot be exercised on a headless screen. The
  repository's record gives the price: about 205 of its 560 commits are refute rounds, and
  supervisão's interface and timing clauses took up to 12 rounds each.
- Options: (a) The spike gains a gate: one test drives a QML control and an edit inside Monaco in
  the same run, the page read through a hook on Oficina's side. (b) Two tiers: logic is proven on
  the Rust model, and each surface is proven by driving it. (c) Docking across monitors is declared
  outside the headless suite.
- My pick: (a) and (c); (b) only if the spike shows the cost cannot be carried.

**30. Oficina's first release is about fifteen surfaces, and forms need a schema nobody has**

- Items: A2, A3, A4, A5, D3. From: feasibility. Standing: your choice of scope. New: the missing
  schema.
- A2.2 builds each form "from its schema". The repository's schemas are prose documents and Rust
  types; nothing a form can be generated from exists. The release also includes the grep engine
  portal deferred, a grid for a million rows, the scope and the frame designer, each at the cost
  entry 29 describes.
- Options: (a) The form schema is generated from core's Rust types, one source for reader and form.
  (b) The plan orders the surfaces: shell, tree, form and text, target over SSH, session control
  and health, log trailing and supervisão's tabs first, the rest in point releases.
- My pick: both.

**31. 3D authoring keeps two scene models in step**

- Items: A6.5, E1, E3. From: feasibility. Standing: gap.
- The scene is authored in Qt Quick 3D inside Oficina, while bodies, collisions and sensors live in
  `fisica` inside maquinar. Shapes, transforms and units must match across a socket. Qt Quick 3D
  brings little for editing: placing, picking, snapping and the equipment library are ours to
  write. Behaviours as ORCL blocks wait for the interpreter.
- Options: (a) A view-only 3D comes first: a scene described in a file, played back from maquinar's
  positions. Authoring follows. (b) Only the ready-made behaviour models until the interpreter
  exists.
- My pick: both.

### Security

**32. Opening a project, or connecting to a target, can run someone else's code on the laptop**

- Items: A6.3, A6.1, B6.2, A5.1, B3.7, J1. From: security. Standing: gap.
- The laptop holds the SSH keys to every plant the engineer serves. Four ways in: cargo's build
  scripts, procedural macros and a `.cargo/config.toml` in the project folder, all run by a Rust
  language server on open; Python macros and a debugger port in ábaco; vendor XML; and a native
  bridge exposed to a page that a target serves.
- Rule to adopt: opening a project runs nothing. ORCL projects have no build scripts and no
  procedural macros; the folder's cargo configuration is not honoured; Python runs after a trust
  prompt per folder; pages not shipped inside Oficina get no native bridge. The language-server
  part belongs in the spike.

**33. Loopback and an SSH tunnel are not a boundary**

- Items: A1.9, A5.1, I3 (Q20). From: security. Standing: gap. It concerns code already built.
- Every component listens on loopback with no accounts. Any process on the target reaches
  controlador's force and download interface and portal's write path. On the laptop, any process
  reaches the plant through the forwarded port, and so does a web page in a browser, by DNS
  rebinding, unless the server checks the Host header or wants a token. A quick search of today's
  crates found no check of the Host or Origin header; I did not prove the exposure.
- Options: (a) A random token per session, handed over SSH and required on every request, with Host
  and Origin checks. (b) Unix sockets, and named pipes on Windows, with the system's permissions
  for controlador's engineering channel. (c) Both.
- My pick: (c), and (a) alone for portal and supervisão.

**34. Text from the plant is shown and exported with no rule**

- Items: A6.3, A3.3, A3.6, A3.7, G1. From: security. Standing: gap.
- QML's text element treats a string that looks like markup as rich text unless told otherwise. A
  device named `<font color=green>RUN</font>` can paint a false state into the strip, and an image
  tag makes Oficina fetch an address. A CSV export carries a cell beginning with `=` into a
  spreadsheet as a formula. Trailed logs pass terminal escape sequences.
- Rule to adopt: text from outside is always shown as plain text, held by a law-style test over the
  QML; the CSV export neutralises formula cells; the interface never parses state text from a
  target, it uses the structured state.

**35. What the controller keeps and what it publishes**

- Items: B3.7, B4.8, B7.2, D2, B9. From: security. Standing: one gap, one choice of yours.
- A fault record holds a snapshot of all data, and B7.2 publishes faults to peers without saying
  whether the snapshot goes with them; it would carry variables the program never marked. And the
  source archive of the last ten downloads sits on every machine delivered, which you chose, with
  know-how protection out of scope.
- Options: (a) Peers get a fault's cause, number and time; the snapshot is read through Oficina
  only. (b) Keeping the source on the target is a project setting, on by default as you decided.
- My pick: both.

**36. A bench device on a real network can pass for a real one**

- Items: E2, A4.3, G1. From: safety. Standing: gap.
- A bench PROFINET device with the station name of a real one, started from a laptop plugged into
  the plant network, can be taken by a real controller after a restart. Its "guard closed" and
  "level OK" then come from a behaviour model. An emulated EtherCAT device on a live segment joins
  the ring.
- Options: (a) Each interface a bench uses is declared isolated in the bench file. (b) Before
  starting, a scan refuses an interface where a real controller or unknown devices answer. (c)
  Bench devices carry an identity that controlador refuses outside simulation.
- My pick: all three.

### Records and regulation

**37. There is no record of who did what**

- Items: B2.2, B3.7, B4.7, B7.4, A2.3, A1.9. From: users, security. Standing: your choices (no
  accounts, no Git integration). New: the regulation, and how little is recorded.
- Forces and breakpoints are recorded only when a fault happens. The download history is a ring of
  ten, and its "origin" is not a person. A peer's start or stop carries no identity. The Machinery
  Regulation (EU) 2023/1230 applies from 20 January 2027 and asks a machine builder for evidence of
  interventions and of software changes. That is read from secondary sources; the annex's exact
  wording, and whether a requested postponement was adopted, are to be checked.
- Options: (a) An append-only log of interventions on the target: download, swap, accept, rollback,
  force set and removed, breakpoint, start and stop with their source, switch change. Each line has
  the time and who: the SSH user, the peer's address, and a name the engineer types once per
  Oficina installation. Its retention is set per target. (b) The documents state that controlador
  is not a safety component.
- My pick: both.

**38. The Cyber Resilience Act sets the security study's deadline**

- Items: B9, J4, B7. From: legal. Standing: gap.
- Regulation (EU) 2024/2847 applies in general from 11 December 2027, and its reporting duties
  since 11 September 2026. An unpaid open-source release keeps its author outside the regulation's
  "manufacturer". Paid support, a paid build or delivery as a product changes that. The first
  machine builder to ship controlador in a product becomes that product's manufacturer and will ask
  upstream for secure defaults, a list of components, a way to report vulnerabilities and years of
  support. Entries 1 to 4 would then count against it.
- Options: (a) The plan states the intent: no commercial activity, steward, or manufacturer. (b)
  Before the first public release: a security file with a reporting channel, and a component list
  produced in CI. (c) The security study's first output is the gap list against the regulation's
  Annex I.
- My pick: all three. The reviewer read a reproduction of the regulation, because the official site
  returned empty pages; the dates match what I know.

**39. The fieldbus organisations have terms the list does not mention**

- Items: B6.1, B6.2, E2, J3. From: legal. Standing: gap (J3 covers PROFINET International's
  membership only).
- ODVA grants the right to make or sell an EtherNet/IP implementation under its Terms of Usage
  agreement, and the name needs conformance. The EtherCAT group's master licence is free but is an
  agreement to sign; its public answers do not cover a device emulated in software, without the
  group's chip. A product sold as "PROFINET" must be certified, and whether PROFINET
  International's specification licence allows publishing source written from it sits in
  members-only text.
- Options: (a) Sign the ODVA and EtherCAT agreements before each network ships. (b) Ask the
  EtherCAT group in writing about the emulated device, and PROFINET International about published
  source. (c) Write "compatible with" until certified.
- My pick: all three. Medium confidence: secondary sources.

### Users and product

**40. In release 1 a PLC programmer has no language they know**

- Items: B8.1, A6.3, B9. From: users. Standing: your choice (ORCL text first, ladder later as a
  view). The reviewer rated it a blocker.
- The only door is Rust text. The ladder view is read-only, and there is no function block diagram
  and no sequence chart. The list never said a standard library exists. It does: Veredas 1's
  orcl-std has the on-delay, off-delay and pulse timers, both edge detectors and the up counter. It
  lacks the down counter and the set and reset latches. B9's "counters" meant high-speed counter
  objects, which the list should say.
- Options: (a) The standard library, with the IEC names and behaviour, is a named first-release
  item and is completed. (b) The plan names the release in which ladder becomes editable. (c) The
  plan says who release 1 is for: people who write text.
- My pick: (a) and (c) now, (b) when you know.

**41. No closed loop, and no way in for an existing project**

- Items: B9, A6.4. From: users. Standing: your choice (technology objects out entirely). The
  reviewer rated it a blocker.
- The first temperature loop has no PID. Nothing reads tags or types from another tool, so an
  existing machine is retyped by hand.
- Options: (a) A PID function block with anti-windup and manual and automatic modes joins the
  standard library. It is a block, not a technology object. (b) The plan says which machines
  release 1 serves: discrete IO, no closed loops, no axes. (c) Later, an import of tags and types
  only, from PLCopen XML.
- My pick: (a) and (b).

**42. A technician without the project cannot open what is running, and needs SSH set up first**

- Items: A2.1, A1.9, A6.4, B3.7. From: users. Standing: your choices (workbench only; SSH). The
  reviewer rated it a blocker.
- "Go online and upload from the device" is the first thing a site technician does in every tool
  they know. B3.7 keeps the source archive on the controller for exactly that, and no Oficina item
  opens it. Connecting needs an SSH key on a Linux target before anything else.
- Options: (a) "Open the project from the target" joins controlador's surfaces. (b) Oficina pairs
  with a target in one step: it creates the key and installs it. (c) One connection view inside the
  workbench: find targets with aranha, connect, see the state, check IO, see forces.
- My pick: (a) and (b); (c) is a view, not the guided mode you declined, and can wait.

**43. Colour and contrast**

- Items: A1.8, F5, K12. From: users. Standing: partly answered, one fix, one gap.
- I recomputed two of the reviewer's figures and they hold. Lavanda and the good green have the
  same lightness (1.05 to 1), and good, warning and fault collapse into one khaki under the most
  common colour blindness. The suite's rule answers that: every state has a mark and a word. It is
  written for supervisão, and Oficina's plan must carry it.
- The fix: the outline `#494360` on the soft tone `#2B2838` is 1.54 to 1, where the only edge of a
  control needs 3 to 1. `#7A7399` gives 3.2 to 1 on the same tone.
- The gap: no light theme anywhere in the list. A dark screen at a sunlit dock is hard to read. It
  was never asked.

**44. No way to reuse blocks across projects, and nothing says how a structure appears**

- Items: B8.2, A6.3, B2.1. From: users. Standing: gap.
- Every tool these users know has a library of versioned blocks. The list names none, and does not
  say whether one member of a structure can be watched, forced or mapped.
- Options: (a) A library is a crate named by path or by Git revision and pinned in the project
  file, with no build scripts (entry 32). (b) Structures expand member by member in watch tables,
  forcing and the IO mapping.
- My pick: both.

**45. No help, no examples, no documents to hand over**

- Items: A2, A6.3. From: users. Standing: gap.
- An acceptance test needs the IO list, the alarm list and the cross-reference on paper. Every
  reference tool prints them. Nothing in the list exports anything but comissionador's test report.
- Options: (a) Export of the three lists to CSV and PDF joins controlador's surfaces. (b) One
  example project per network. (c) Context help in the editor.
- My pick: (a) and (b).

**46. What a Rust engineer gets is not stated**

- Items: B1.2, A6.3, J2. From: users. Standing: gap in the list, probably not in the design.
- The list does not say whether an ORCL project is a cargo crate whose logic `cargo test` runs,
  whether the checker works outside Oficina and in CI, or where the allowed subset is written. In
  Veredas 1 ORCL builds natively as plain Rust, so the answers are probably yes. The plan should
  say so and publish the subset as one page, which is J2's output.

---

## Minors

| No. | Entry | Standing | What to do |
|---|---|---|---|
| 47 | Automatic acceptance when the time limit runs out (B3.6) | your choice | Record it in the download history as accepted by timeout, and as an event. |
| 48 | A one-value fix is a whole deployment (B3.1, B3.4) | your choice | Say that "force once now" on a variable is the online change of a value, TIA's Modify. Say what happens when the doubled scan does not fit: stop and download, the popup saying so. |
| 49 | The three force modes use words a TIA user does not know (B2.2) | your wording | Tables show the mode in words, never the [!] mark alone. |
| 50 | The strip is English inside a translated interface (B5.3, A1.7) | your choice | The detail view is translated, as decided. Add a "seen" mark on a maintenance request, so that clearing one whose cause persists does not look like a dead button. Decode `shadow(drives=13, watches=12)` in the detail view. |
| 51 | Four outside projects on the Qt side, and a Qt release every six months (A1.1, A1.3, A1.6, A1.10) | your choice | cxx-qt is at 0.10, Qt's own Rust bridge is a preview until early 2027, KDDockWidgets' Qt Quick side has no auto-hide, Spix has one maintainer. Choose the bridge after the spike; pin one Qt version per Oficina release. |
| 52 | Virtual machines without a GPU (A1.4) | gap in wording | Qt Quick's software backend cannot draw 3D at all. The single fallback is the normal pipeline over a software rasteriser (WARP on Windows, llvmpipe on Linux), chosen before the first window, so switching is a restart. Six environments to test, none on hosted CI. |
| 53 | Items that wait on undecided work (A6.6, A6.4, B7.4) | gap | The HMI creator waits for the panel HMI's plan (H1), device naming for G2, peer start for the security study. Mark them so in the plan. |
| 54 | Portuguese names for a foreign user (law L2) | the repository's law | Show the role as a subtitle; let the command palette match unaccented and English aliases. |
| 55 | A crash and a power cut look alike (K7) | my reading | Stays open; see entry 7. |

---

## Findings that fell away

**Answered by a decision or a rule the reviewers could not see.** The first four are omissions in
my list.

1. *Variables that lose their value at a swap.* The option you chose says Oficina lists every
   added, removed and reinitialised variable before the switch. My list left it out.
2. *Whether the watcher halts at a breakpoint.* The decision says a hit halts everything and the
   watcher pauses in step. My list left it out.
3. *"No timer to be found."* The standard library exists (entry 40). My list did not mention it.
4. *"No online change of a value."* "Force once now" is that (entry 48). My list did not say so.
5. *State shown by colour alone.* The design rule of a mark and a word answers it (entry 43).
6. *A test cannot read the embedded page.* The spike already has "a supervisão page embedded and
   read back by a test"; entry 29 makes the combined run explicit.

**Claims that did not hold when checked.**

1. *"ethercrab's newest release is 0.4, from 2024."* crates.io shows 0.7.1 of 23 March 2026. The
   list was right.
2. *"SCL cannot be produced from the compiler's intermediate form."* Too strong. Structured code
   can be rebuilt from a control-flow graph, losing comments and the shape of expressions. The
   claim does hold for a ladder view that stays tied to the source (entry 12).
3. *"rust-analyzer cannot be limited to a subset, so ORCL needs an analysis front end of its own."*
   ORCL is valid Rust, so rust-analyzer serves completion, cross-reference and the call tree as it
   is, and the subset checker plugs in as its check command. What remains is entry 28 (the clients)
   and entry 32 (it must run nothing on open).
4. *"Aranha's PROFINET code is two files."* It also has the nameplate read. The conclusion stands
   (entry 26).
5. *"Vendor description files break Q18."* Q18 refused readers of one company's engineering exports
   for supervisão. Device descriptions are a standard. The clash that remains is with the TOML rule
   (entry 23).

## What no reviewer could assess

- The interpreter's timing on a real-time kernel: jitter and worst-case time decide whether the
  overrun rule and the shadow test's period check can hold at all.
- Whether a crash of the runtime, as distinct from a FAULT, stops the cyclic exchange so that
  devices fall back on their own. It depends on master code not yet written.
- The exact text of the Machinery Regulation's annex, and whether its requested postponement was
  adopted.
- PROFINET International's specification licence, and the EtherCAT group's position on an emulated
  device.
- The licences of the EtherNet/IP crate and of the physics engine: neither is chosen yet.
- Which verified contract clauses would go stale. Entries 1, 11, 22, 24 and 33 touch portal's write
  surface, the project file and supervisão.
- What Siemens and Beckhoff do when the programming device is lost during a halt: stated from
  memory, not from their documents.
- Whether one test can drive QML and the embedded page together: a question for the spike.
- The market: new machines or existing ones, one programmer per site or several. It decides how
  heavy entries 37, 41 and 42 are.

## What I checked myself, today

- crates.io: ethercrab is at 0.7.1; `rustc_public` is not published.
- Veredas 1: orcl-std's blocks (three timers, two edge detectors, one up counter) and aranha's
  files (DCP, LLDP, a nameplate read, nothing of the cyclic exchange).
- This repository: the rules the reviewers quoted (the portal plan's light build and its single
  write switch, "core holds no frame layout", Q18, Q19, the mark-and-word rule), the commit counts
  (560, about 205 of them refute rounds), no pinned toolchain file, and no Host or Origin check
  found in the crates by a quick search.
- Your interview answer on carrying data at a swap, read back from the session record.
- Two contrast figures, recomputed by hand.

## A possible order for deciding

1. Command and gates: entries 1 to 4, then 9.
2. Halts, forces, power and devices: 6, 7, 8, 16.
3. Online change: 5, 15, 13.
4. Where the pieces live: 10, 11, 22, 23, 25.
5. The interpreter: 12, with the spike's gates.
6. The state text: 17 to 21.
7. Security rules, records and licences: 32 to 39, and 14.
8. Product and order of work: 40 to 46, 24, and 26 to 31.
