# Controlador's own files: names

**Ruled on 2026-11-25** (`docs/questions.md` Q224): the Portuguese names proposed below stand, against English names
and one `veredas-controlador-*` prefix. `docs/controlador-plan.md` ("Before the code") owed names for
controlador's own files that do not clash with `veredas-controller`, the format of maquinar's simulated-controller
file (`docs/controller-schema.md`). This file proposes them, on 2026-11-22. A name is cheap to change until a file
of that format exists anywhere; none does yet, and no key of any of them is designed here.

## The clash to avoid
`veredas-controller` already names the bench's file: a simulated controller's behaviour as rules against a catalog.
A file of the soft PLC called `veredas-controlador` would differ from it by three letters, in two languages, for two
different things. So no format of controlador's is named after the component: each is named for what it holds, in
Portuguese as the suite's names are, plain ASCII as law L7 and the reader in core expect.

## The files

| Format | What it holds | Where it lives | Who writes it |
|---|---|---|---|
| `veredas-aplicacao` | A controller's application: the program it runs, its tasks with their periods and priorities, the maximum and warning cycle times, each output's halt setting (its safe value is declared by the program, `Program::safe_outputs` in `orcl-std` today), the retained save's period and shortest gap, the IO mapping, the Modbus addresses the engineer declares | the project, beside the program's source | the engineer, in Oficina or by hand |
| `veredas-rede` | The hardware configuration: each network, its devices and their modules, names and addresses, the vendor description file each one comes from | the project | Oficina, from the engineer's choices and aranha's scan |
| `veredas-alvo` | The target itself: where the program's two copies live, the network card of each network, the real-time priority and cores, what the hardware can do for the retained save (memory that keeps its content, hold-up power, neither) | the controller's own disk, never in a download | a person at the target |
| `veredas-carga` | A download's manifest: every part with its length and SHA-256, the identity being the manifest's own hash (Q36's proposal) | inside each download, and kept with the last ten | Oficina, when it sends |

Not TOML, and so outside law L7:
- **The compiled program**, the bytecode `orcl-interp` runs: a binary file with a header of its own (a magic number,
  its format's version, the ORCL version it needs), named `<name>.orclb`. Its format belongs to ORCL
  (`docs/orcl/README.md`).
- **The commissioning switch**: a file on the controller's operating system, not writable from outside
  (2026-11-15). Proposed as a file whose presence arms the switch and whose absence leaves it off, so that a lost or
  damaged switch reads as off (Q38's reading), in a folder only the target's own accounts may write. It holds no
  setting and has no format.
- **The records the controller keeps**: the download history, fault records, the state history and the retained
  save. Their names come with their capabilities; Q36, Q40 and Q41 decide what they hold.

## Why this split
- **What may change in an online change is one file.** Logic, data and the IO mapping may differ between two versions
  in a shadow test; the hardware configuration and task timing may not (`docs/controlador-plan.md`, "Online
  change"). So the IO mapping sits in `veredas-aplicacao`, beside the program, and the hardware in `veredas-rede`,
  which a swap leaves alone. Task timing sits in `veredas-aplicacao` too, and a shadow test refuses a version whose
  timing differs.
- **What belongs to the machine never travels.** The disks and network cards of one target are not the project's:
  a download that carried them would make every controller of a project the same machine. So `veredas-alvo` is
  written at the target, as the commissioning switch is.
- **A download names its parts.** Q36's proposal hashes every part through one manifest; `veredas-carga` is that
  manifest's format, so the identity of a download is a file the controller can read back.
