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
1. **HEAD of a clean tree**, cut from a fresh `origin/main`: the cut refuses a tree that differs
   from HEAD and a release whose manifest exists. It runs the whole suite there first (`cargo
   nextest run --workspace`, the gate of `CLAUDE.md`, behind the machine's lock, its temporary
   files in the cache), and refuses a red one. "Tested together" means two runs: that one, which tests the components against each
   other from source, and the cut's own run of the bundle (step 5). A **trial cut** (`--trial`)
   skips the suite, says so in its manifest (`suite = "not run: a trial cut"`) and writes it
   elsewhere: it proves the recipe and is never committed.
2. The components, their versions and their executables read from the commit as C-290 reads
   them (its tree out of git, asked of `cargo metadata`); the recipe's data
   (`tools/entrega/maquina/maquina.toml`) from git too.
3. **Built on the oldest system the release targets**, a builder machine of the tester machine's
   own distribution (Debian 13, glibc 2.41) under QEMU with KVM, eight processors: the commit's
   tree (`git archive`) and its dependencies' sources (`cargo vendor`, as `Cargo.lock` pins them)
   handed in on a disk as a tar stream, `cargo build --release --locked --offline` run inside
   under the toolchain the suite was tested with (the development machine's `rustc`), the
   executables handed back on another disk. Built on the development machine (Ubuntu 26.10,
   glibc 2.44), portal and supervisão need `GLIBC_2.44` and do not start on Debian 13 (the probe
   below); built on the oldest, they run there and on every newer system. The builder's disk is
   the base image with the recipe's packages (a C and C++ compiler) and the toolchain from
   rustup's official source, built once per toolchain and kept in the cache. Each executable's
   symbol versions are then read (`objdump -p`) and held to the guest's glibc, libstdc++ and C++
   ABI as the recipe states them, so a binary that would not start on the tester machine is
   refused at the cut, by name.
4. **The bundle** assembled from git's objects at the commit and the built executables,
   `SHA256SUMS` written, packed with its entries in name order, owned by root and dated at the
   commit (one commit packs to the same bytes), then extracted again and its sums checked.
5. **Run together**: from the extracted bundle, the bench started on `planta/bench.toml`, then
   supervisão on `planta/supervisao.toml`, which starts the plant's portal session through
   portal's library. The plant is copied and only its ports change, to free ones on the loopback
   (the bench's OPC UA port, the project's endpoint, supervisão's address), so the run never meets
   another on the machine. The cut waits until every object of the loop carries a value, reads
   the object feed twice: every object live, both sources live, both parcels moved along the
   loop; posts the write the binding offers and sees portal's switch refuse it (the plant ships
   disarmed); then closes both stdins and sees both exit 0.
6. **The manifest** written into `releases/` only then, naming the commit and what was run, and
   checked like any other (C-290); the person cutting commits it and pushes. The bundle stays under
   `target/entrega/<release>/`. A git tag `suite-<release>` on the commit is Q432's.

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
- **The release**, its bundle installed under `/opt/veredas/<bundle>/`, `/opt/veredas/current`
  pointing at it, the executables on every user's `PATH`; a service checks its files against
  `SHA256SUMS` at every boot and says so on the console and in the journal.
- **A plant from the first boot**: `planta/`, "the loop" of `docs/binding-schema.md` (a loop with
  three outlets, two parcels and an infeed) fed by a bench file of its own: the controller's OPC UA
  server with the parcels' places ramping along the loop, the outlets filling, the speed drawn, and
  a log emitter writing the sorter's lines the project's capture rule reads
  (`tools/entrega/planta/`, copied into the user's home, where the tester edits it). Two services
  start at boot as the user, `veredas-bancada` (the bench) and `veredas-supervisao` (supervisão
  serving the plant), and restart on failure; their output is in the journal. Each executable
  runs until its stdin closes, so a service holds it open with a pipe and its stop ends it
  abruptly, which the bench and an in-memory store bear (a clean stop on a signal is asked of
  maquinar, portal and supervisão). The plant ships disarmed: the binding's "Release" is offered
  and portal refuses it.
- **Supervisão reachable from the host**: the plant's supervisão file binds every interface of
  the machine on port 8902, so the host's browser reaches it at the machine's address (Q439);
  inside, the machine's browser opens it at login.
- **A browser**: Chromium, the engine supervisão's page is tested on (C-051 and its siblings drive
  Chrome for Testing).
- **A desktop**, Xfce (Q434) under LightDM, logged in automatically, its panel set up beforehand
  so no first-start dialog asks, the browser opened on the plant at login, a terminal, a text
  editor for the plant's files, and on the desktop the docs (`/opt/veredas/current/docs`), the
  plant and a short guide (`tools/entrega/maquina/guide.md`).
- **A network that takes any card**: NetworkManager, the build's card-bound configuration removed,
  so VMware's card (its own address, NAT or bridged) comes up by DHCP.
- **The guest tools**: `open-vm-tools` and its desktop part, so VMware sizes the screen, shares
  the clipboard, keeps the clock and shuts the machine down cleanly.
- **Later**: Oficina (Qt Quick on X11 first, its foundations name a virtual machine without a GPU
  as a host, where it switches to software drawing), then each component as it reaches a release.
- **A user**, `veredas` (Q438), the only account that logs in; no SSH server.

### How the image is built (the recipe, `entrega maquina <release>`)
Everything below runs as the user, without sudo, without VMware, packer or docker (what this
machine was found able to do is the next section). The recipe's choices that are the architect's
(the distribution, the desktop, the user, the virtual hardware) are data in one stamped file,
`tools/entrega/maquina/maquina.toml` (`veredas-machine`, version 1), never code, so a ruling is an
edit of that file.
1. **The base**: the distribution's official cloud image (Debian 13, Q433), at a dated build
   named by URL in `maquina.toml` with its SHA-512 copied from the build's own list (served
   over HTTPS, not signed: the pin in the repository is what the recipe trusts),
   downloaded once into a cache outside the repository (`~/.cache/veredas/entrega/`) and checked
   on every use; a mismatch is refused. The image is never committed.
2. **The disk**: a copy-on-write overlay of the base, grown to the size `maquina.toml` gives.
3. **Provisioning, unattended**: the overlay booted under QEMU with KVM, headless, the serial
   console written to a log; a NoCloud seed (an ISO labelled `cidata`, written by `xorriso`)
   carries the cloud-init user data, and a data disk carries the release's bundle, the guide and
   the provisioning script as a tar stream. The first boot installs the recipe's packages (the
   desktop, the browser, the guest tools, NetworkManager) from the distribution's mirror, removes
   the SSH server, creates the user, installs the bundle and the plant, the services and the
   autologin, hands the network to NetworkManager, disables cloud-init, empties the package cache,
   blanks the machine's identity (each import draws its own), trims the free space and powers off;
   the recipe waits for the power-off within a bound and refuses a log without the provisioning's
   last line. The release's recipe and guide are read at the commit its manifest names.
4. **The shipped disk**: the overlay flattened by `qemu-img convert` into a stream-optimized VMDK,
   VMware's compressed format for an OVA.
5. **The package**: an OVF descriptor for VMware's virtual hardware (Q435's version; a
   paravirtual SCSI controller, a VMXNET3 network card on a network named `nat`, a video card,
   BIOS firmware, 2 processors and 4 GB by default), the VMDK and a `.mf` of their SHA-256 sums,
   tarred in that order (ustar, owned by root) into `veredas-<release>.ova`.

### How a build is proven
- **The shipped disk boots on VMware's devices**: the VMDK taken out of the OVA, never the
  overlay, is booted under QEMU with KVM on VMware's own disk and network devices (QEMU emulates
  `pvscsi` and `vmxnet3`), copy-on-write so the proof changes nothing shipped. QEMU's
  `vmware-svga` is older than today's `vmwgfx` driver accepts, so the display is VMware's to
  prove; the proof boot draws on QEMU's standard VGA.
- **The network takes VMware's card**: the proof boot gives the card an address of VMware's
  prefix that the provisioning never saw.
- **The plant runs from the first boot**: from the host, through a port QEMU forwards to the
  machine's 8902, the test reads supervisão's page and the plant's object feed and sees every
  object live, the parcels move and the sorter's lines arrive; a write the binding offers is
  refused by portal's switch; the release's files are the bundle's (the boot's check of their
  sums, on the console).
- **The desktop draws the page**: a screen dump over QEMU's monitor shows a screen of many
  colours, not a console.
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
- **QEMU emulates VMware's devices**: `pvscsi`, `vmxnet3` (and `mptsas1068`, VMware's "LSI Logic
  SAS"), so the boot proof runs on VMware's disk and network hardware; its `vmware-svga` is
  refused by Debian 13's `vmwgfx` ("unsupported hypervisor").
- **The whole path, probed end to end** (2026-11-22): Debian 13's `generic` image of build
  20261116-2664 (435 MB, its SHA-512 checked), a copy-on-write overlay grown to 8 GB, booted under
  KVM on `pvscsi` and `vmxnet3` with a NoCloud seed on an IDE CD and a data disk holding a tar
  stream: root found on the paravirtual disk, the network up through QEMU's NAT, the mirror
  answering, cloud-init's last stage done 8 s after power-on and the machine off at 14 s. The disk
  flattened by `qemu-img convert` into a stream-optimized VMDK (442 MiB for 852 MB used, 34 s),
  which boots under QEMU copy-on-write on the same devices, its bytes unchanged after.
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
  executable needs glibc 2.34 and runs in Debian 13; portal's and supervisão's need `GLIBC_2.44`
  (`sinh` and `cosh` took new symbol versions in glibc 2.44) and stop at once there, "version
  `GLIBC_2.44' not found": hence the builder machine (step 3 of the cut).

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
- **SeaBIOS boots the first disk it likes**: with a data disk attached and no `bootindex` on the
  system disk, the probe hung ten minutes with nothing on the serial console. The system disk
  always carries `bootindex=0`, and every boot runs under a bound.
- **A Unix socket's path holds at most 107 bytes**, and the scratch folder's paths are longer:
  QEMU's monitor (QMP) listens on the loopback by TCP.
- **Debian's generic image ships an SSH server and a network configuration bound to the build's
  card**: cloud-init writes a netplan matched to the card it saw, which VMware's card (another
  address) would not match once cloud-init is disabled. The provisioning removes the server and
  hands the network to a configuration that takes any wired card.
- **agetty hangs up the serial line when it starts**: a build whose output went to `/dev/ttyS0`
  from cloud-init's first stage lost every line once the login prompt came up, and only the exit
  code, written by a later `echo`, arrived. A guest's long job logs to a file, which comes back on
  a disk, and its tail is echoed to the console after.
- **`/tmp` is a tmpfs of the machine's memory, shared by every session**, and its quota for the
  user ran out at 20 GB ("Disk quota exceeded"). Disks, bundles and OVAs go under `target/entrega/`
  or the cache, never in `/tmp`. A `TMPDIR` moved elsewhere must lie outside every git repository:
  under the worktree's `target/`, a law's test of "a tree without git is refused" found the
  worktree's repository around its tree and failed the cut's suite. The cut's suite takes
  `~/.cache/veredas/entrega/tmp`.
- **Debian's image has no `strings`** (binutils is not installed): what the guest is asked is
  answered with its own tools, or read on the host.
