# entrega: the plan

What Veredas 2 ships: the suite release and the tester machine. `docs/veredas-plan.md` ("How
releases work") wins on intent; `.zen/contract.md` (P-011, P-022, then the `C-xxx` clauses from
C-290 they become) wins on status. This file says what a release is made of and how it is cut,
what the tester machine holds and how it is built and proven, what was found of this machine's
means, and the questions the architect has not ruled (`docs/questions.md`, from Q430).

**Entrega** (delivery) is no component. It adds no `veredas-*` crate and edits no component's
crate; its code is a tool, `tools/entrega` (repository tooling, like `tools/leis`: exempt from the
component laws, free to depend on any member, L5). Its name is the architect's to confirm (Q430).

## The role nobody covers
**Entrega turns the repository into what a person runs without building it: a suite release, a
manifest that pins a tested set of component versions with the executables built from them, and
the tester machine, a virtual machine holding that release with a plant already running.**

Every component builds and tests itself; none says which versions of the others it was run with,
none ships an executable to someone without Rust, and none owns the plant a tester or a designer
looks at on the first day. Without entrega a tester builds the suite from a commit and writes a
bench, a project and a binding before seeing anything.

## Part one: the suite release

### What a release is made of
- **The manifest**, `releases/<release>.toml`, committed: a `veredas-release` file (version 1,
  read by core's one header reader, a misspelled key refused like every file of the suite, L7). It
  names the release, the commit it was cut from, the target and the toolchain, every component at
  that commit with its version and the executables it ships, and what was run to call the set
  tested. A manifest is checkable forever: at its commit every component of the workspace is
  listed, at the version its own `Cargo.toml` states there, and nothing else is.
- **The bundle**, built, never committed: `veredas-<release>-<target>.tar.gz`, holding `bin/` (the
  executables, built in release mode), `planta/` (the plant of the tester machine, below), `docs/`
  (the suite's README, its plan, every file reference `docs/*-schema.md`, the shipped components'
  plans and `docs/design/`), `release.toml` (the manifest) and `SHA256SUMS` over every file.
- **The tester machine**, part two: built from a bundle, never from a tree.

The worked example, the first manifest's shape (`tools/entrega/tests/manifest.rs` loads this block
and checks every statement of the keys below against what it read):

```toml
format = "veredas-release"
version = 1
release = "2.0.0-alpha.1"
commit = "0123456789abcdef0123456789abcdef01234567"
target = "x86_64-unknown-linux-gnu"
toolchain = "rustc 1.100.0 (0ef1bc386 2026-11-09)"

[[component]]
name = "veredas-core"
version = "2.0.0-alpha.1"

[[component]]
name = "veredas-maquinar"
version = "2.0.0-alpha.1"
bins = ["veredas-maquinar"]

[[component]]
name = "veredas-portal"
version = "2.0.0-alpha.1"
bins = ["veredas-portal"]

[[component]]
name = "veredas-supervisao"
version = "2.0.0-alpha.1"
bins = ["veredas-supervisao"]

[tested]
suite = "cargo nextest run --workspace"
together = "the bundle's bench and supervisão on its plant"
```

Keys (each required unless a default is given):
- `format` = `"veredas-release"`, `version` = 1; another format or version is refused by name.
- `release`: the suite release's own SemVer (Q431), unique across `releases/`, the file's name.
- `commit`: the full 40-hexadecimal id of the commit the bundle was built from, an ancestor of the
  commit that adds the manifest.
- `target`: the Rust target triple the executables were built for.
- `toolchain`: what `rustc --version` printed at the cut, stated, not enforced.
- `[[component]]`: `name` (a `veredas-*` or `orcl-*` member at the commit), `version` (SemVer, the
  one its manifest states at the commit), `bins` (default none: a library such as core ships
  none; each a `[[bin]]` of the component, built with its default features).
- `[tested]`: `suite`, the command whose green run at the commit made the set tested; `together`,
  what the cut ran from the bundle itself. Statements of what was run, written by the cut.

### How a release is cut (the recipe, `entrega cut <release>`)
1. A clean tree at a commit of `main` that the whole suite passed (`cargo nextest run
   --workspace`, the gate of `CLAUDE.md`). "Tested together" means two runs: that one, which
   tests the components against each other from source, and the cut's own run of the bundle.
2. The components and their versions read from the commit (git, not the working tree); a version
   already shipped under another release with other code is refused (a release pins a version to
   one tree).
3. `cargo build --release --locked` of every shipped executable, for the target; each executable's
   needs of the system's libraries read from its symbol versions and held to the guest's (part
   two), so a binary that would not start on the tester machine is refused at the cut, by name.
4. The bundle assembled, `SHA256SUMS` written, the bundle extracted again into a scratch folder.
5. **Run together**: from the extracted bundle, the bench started on `planta/bench.toml`, then
   supervisão on `planta/supervisao.toml`, which starts the plant's portal session through
   portal's library; the cut waits until the session's objects carry live values from both the
   bench's OPC UA server and its log emitter, the layout's markers move, and a write the binding
   offers is refused by portal's switch (the plant ships disarmed); then both are stopped by
   closing their stdin and must exit 0.
6. The manifest written, naming the commit and what was run; the person cutting commits it under
   `releases/` and pushes. A git tag `suite-<release>` on the commit is Q432's.

A MAJOR bump of `veredas-core` ripples (the plan): every dependent's next version is cut only once
it has been rebuilt and retested against it, which the cut sees as the suite green at the commit.

## Part two: the tester machine

### Who uses it, and for what
- **Testers** run a named suite release without building it: the plant is running when the
  machine boots; they change the bench's pressure, write their own bench, project and binding
  files from the references in `docs/`, run the executables from a terminal, and report against
  the release's name.
- **Designers** look at supervisão's pages on a live plant (the layout, the dashboards, the
  tables, a write offered and refused), edit a binding or the layout's SVG and see the result, and
  later work in Oficina.

Neither needs Rust, Node, git or a network beyond the machine.

### What it holds
- **The release**, its bundle installed under `/opt/veredas/<release>/`, `/opt/veredas/current`
  pointing at it, the executables on every user's `PATH`.
- **A plant from the first boot**: `planta/`, "the loop" of `docs/binding-schema.md` (a loop with
  three outlets, two parcels and an infeed) fed by a bench file of its own: the controller's OPC UA
  server with the parcels' places ramping along the loop, the outlets filling, the speed drawn, and
  a log emitter writing the sorter's lines the project's capture rule reads. Two services start at
  boot, `veredas-bancada` (the bench) and `veredas-supervisao` (supervisão serving the plant), and
  restart on failure; their output is in the journal. The plant ships disarmed: the binding's
  "Release" is offered and portal refuses it.
- **Supervisão reachable from the host**: the plant's supervisão file binds every interface of
  the machine on port 8902, so the host's browser reaches it at the machine's address (Q439);
  inside, the machine's browser opens it at login.
- **A browser**: Chromium, the engine supervisão's page is tested on (C-051 and its siblings drive
  Chrome for Testing).
- **A desktop**, Xfce (Q434), logged in automatically, with the browser on the plant, a terminal,
  a text editor for the plant's files, and the docs one click away (`/opt/veredas/current/docs`).
- **The guest tools**: `open-vm-tools` and its desktop part, so VMware sizes the screen, shares
  the clipboard, keeps the clock and shuts the machine down cleanly.
- **Later**: Oficina (Qt Quick on X11 first, its foundations name a virtual machine without a GPU
  as a host, where it switches to software drawing), then each component as it reaches a release.
- **A user**, `veredas` (Q438), the only account that logs in; no SSH server.

### How the image is built (the recipe, `entrega maquina <bundle>`)
Everything below runs as the user, without sudo, without VMware, packer or docker (what this
machine was found able to do is the next section). The recipe's choices that are the architect's
(the distribution, the desktop, the user, the virtual hardware) are data in one stamped file,
`tools/entrega/maquina/maquina.toml` (`veredas-machine`, version 1), never code, so a ruling is an
edit of that file.
1. **The base**: the distribution's official cloud image (Debian 13, Q433), at a dated build
   named by URL in `maquina.toml` with its SHA-512 from the distribution's signed list,
   downloaded once into a cache outside the repository (`~/.cache/veredas/entrega/`) and checked
   on every use; a mismatch is refused. The image is never committed.
2. **The disk**: a copy-on-write overlay of the base, grown to the size `maquina.toml` gives.
3. **Provisioning, unattended**: the overlay booted under QEMU with KVM, headless, the serial
   console written to a log; a NoCloud seed (an ISO labelled `cidata`, written by `xorriso`)
   carries the cloud-init user data and the bundle. The first boot installs the desktop, the
   browser and the guest tools from the distribution's mirror, installs the bundle and the plant,
   the services and the autologin, creates the user, then disables cloud-init, empties the package
   cache, zeroes the free space and powers off; the recipe waits for the power-off within a bound
   and refuses a log without the provisioning's last line.
4. **The shipped disk**: the overlay flattened by `qemu-img convert` into a stream-optimized VMDK,
   VMware's compressed format for an OVA.
5. **The package**: an OVF descriptor for VMware's virtual hardware (Q435's version; a
   paravirtual SCSI controller, a VMXNET3 network card, VMware's SVGA display, BIOS firmware, 2
   processors and 4 GB by default), the VMDK and a `.mf` of their SHA-256 sums, tarred in that
   order into `veredas-<release>.ova`.

### How a build is proven
- **The shipped disk boots on VMware's devices**: the VMDK taken out of the OVA, never the
  overlay, is booted under QEMU with KVM on the same device models VMware presents (QEMU emulates
  `pvscsi`, `vmxnet3` and `vmware-svga`), copy-on-write so the proof changes nothing shipped.
- **The plant runs from the first boot**, with nobody logged in: from the host, through a port
  QEMU forwards to the machine's 8902, the test reads supervisão's page and the plant's object
  feed and sees the parcels move and the sorter's lines arrive; a write the binding offers is
  refused by portal's switch; the executables in the machine are the bundle's (their sums).
- **The package is what it says**: the OVA's members in order, the descriptor naming the disk with
  its size and capacity, the `.mf` sums matching the members.
- What only VMware proves is listed below, and stays the architect's to try.

The image test downloads the base once and needs KVM; it runs by its own command, recorded with its
clause and run at every cut, not in every run of the suite (Q440).

### What this machine can build (found 2026-11-22)
Verified by running each, not assumed:
- **KVM works without sudo**: `/dev/kvm` carries an ACL for the user; QEMU 11.0.3 starts with
  `-accel kvm`.
- **QEMU's user network** (slirp) is built in: the guest reaches the distribution's mirror and the
  host forwards a port in, no bridge or tap needed.
- **QEMU emulates VMware's devices**: `pvscsi`, `vmxnet3`, `vmware-svga` (and `mptsas1068`,
  VMware's "LSI Logic SAS"), so the boot proof runs on VMware's virtual hardware.
- **Images**: `qemu-img` 11.0.3 converts to VMDK, `xorriso` writes the seed ISO, `tar` packs the
  OVA, `sha256sum`, `curl` and `gpg` fetch and check the base.
- **libvirt** answers on `qemu:///session` and the user is in the `libvirt` group, but nothing
  here needs it: the recipe runs QEMU directly, so it runs the same on any Linux with KVM.
- **Unprivileged user namespaces** work through `unshare -r` (AppArmor restricts them for other
  programs), which a later recipe without a VM (a root filesystem built by `mmdebstrap`) could use;
  `mmdebstrap`, `debootstrap`, `virt-customize` and `guestfish` are not installed.
- **Not here, and not needed**: VMware, `ovftool`, packer, docker, podman, `genisoimage`,
  `cloud-localds`, mtools.
- **The executables' needs**: built on this machine (Ubuntu 26.10, glibc 2.44), the bench's
  executable needs glibc 2.34 at most; portal's and supervisão's, which link the database engine's
  C++, are measured at the cut against the guest's glibc and libstdc++ (step 3 above).

### What only VMware proves
- That the OVA imports into each product named by Q435 (Workstation Pro, Fusion, ESXi) and the
  machine boots there; that VMware's own SCSI and network devices behave as QEMU's models of them
  do; that open-vm-tools resizes the screen and shuts down cleanly.
- To try it, the architect installs VMware Workstation Pro 17 (free for every use since November
  2024) on a Windows or Linux computer, or uses an existing ESXi or Fusion on an Intel Mac, opens
  the OVA (File > Open), boots it, and opens `http://<machine>:8902/` from the host. Broadcom's
  `ovftool` (a free download behind a Broadcom account) also validates the descriptor without
  running it. Apple-silicon Macs run only arm64 guests in Fusion: this image is x86_64 (Q435).

### Delivery and updates
- **An OVA**, one file every VMware product opens, VirtualBox too; about 1.5 to 2.5 GB (measured
  on the first image). Where it is published is Q436: a GitHub release of the repository (2 GB per
  file) or a share the architect names. Until ruled, it is built and kept on this machine.
- **Updates** (Q437): meanwhile a new OVA per release, the machine disposable; then an update
  command in the machine that fetches a newer bundle, checks its sums and its manifest, installs it
  beside the old one under `/opt/veredas/` and moves `current`, leaving the plant's files alone.

## Rules
- **A release pins a tree.** One release, one commit; a manifest checkable at any later commit
  against git's history, as L6 judges history.
- **Built from the repository by a recipe.** The bundle from a commit, the machine from a bundle;
  no step by hand, no file of the image made anywhere but by the recipe.
- **Large files are never committed.** The base image, the seed, the disk, the OVA and the bundle
  live in the cache or under `target/`; the recipe fetches and checks what it needs.
- **The plant is the bench's.** The tester machine's plant is configuration (a bench file, a
  project, a binding, an SVG), content-agnostic like the suite (Q8): invented, named for its shape.
- **The machine is disarmed and local by default.** Writes off in the plant's project; supervisão
  is reachable from the machine's own network only, which VMware's default (NAT) limits to the
  host; Q29's gaps (a request's `Host` and `Origin` unread) hold here as everywhere.
- **Each claim a clause, proven by a test that builds or boots the real thing**, under
  `tools/entrega/tests/`, one file per capability: no mock of QEMU, of the guest or of the bundle.
- **Typed errors** in the tool, no `unwrap`/`expect` on an I/O path; tests may unwrap.

## Clauses ahead
- **C-290**, the manifest: the format and its refusals, and a manifest checked against the
  repository's history (`tests/manifest.rs`).
- **C-291**, the cut: the bundle built in release mode from a commit, its sums, and its
  executables run together against the bench on the plant it ships (`tests/cut.rs`).
- **C-292**, the tester machine: the OVA built by the recipe from a bundle, its disk booted under
  QEMU on VMware's devices, the plant live from the first boot and reached from the host
  (`tests/maquina.rs`).

## What a component is asked for
Through the orchestrator, none needed for the first image:
- the executables stop when stdin closes; a service manager stops them with SIGTERM, which ends
  them abruptly. The bench and an in-memory store lose nothing by it, so the services keep stdin
  open meanwhile; a SIGTERM handled as stdin's close (maquinar, portal, supervisão) would make the
  stop clean once a plant keeps a store on disk;
- a `--version` flag on each executable, so a tester reads what runs without the manifest.

## Traps
Recorded as the work meets them.
- **A background shell may not load the profile**: `cargo` was not on `PATH` in a background
  command, which failed in a pipe that hid the exit code. Name `~/.cargo/bin/cargo` or export
  `PATH`, and never pipe a build.
