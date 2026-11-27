# alforge: the plan

What Veredas 2 ships: the suite release and the tester machine. `docs/veredas-plan.md` ("How
releases work") wins on intent; `.zen/contract.md` (the `C-xxx` clauses from
C-290, P-011 promoted into them, and P-022) wins on status. This file says what a release is made of and how it is cut,
what the tester machine holds and how it is built and proven, what was found of this machine's
means, and the questions put to the architect (`docs/questions.md`, from Q430; Q430 to Q441 ruled
2026-11-25).

**Alforge** (a saddlebag: what the rider carries; the architect's name, Q430) is no component. It
adds no `veredas-*` crate and edits no component's crate; its code is a tool, `tools/alforge`, the
crate `alforge` (repository tooling, like `tools/leis`: exempt from the component laws, free to
depend on any member, L5). Until 2026-11-25 it was `entrega`, in `tools/entrega`: a release cut
before then (2.0.0-alpha.1) holds its recipe, guide and plant there, and the tool reads them there.

## The role nobody covers
**Alforge turns the repository into what a person runs without building it: a suite release, a
manifest that pins a tested set of component versions with the executables built from them, and
the tester machine, a virtual machine holding that release with a plant already running.**

Every component builds and tests itself; none says which versions of the others it was run with,
none ships an executable to someone without Rust, and none owns the plant a tester or a designer
looks at on the first day. Without alforge a tester builds the suite from a commit and writes a
bench, a project and a binding before seeing anything.

## Part one: the suite release

### What a release is made of
- **The manifest**, `releases/<release>.toml`, committed: a `veredas-release` file (version 1,
  its header read by core's one header reader and held to its stamp by law L7; a misspelled key
  refused by the manifest's own reader, C-290). It
  names the release, the commit it was cut from, the target and the toolchain, every component at
  that commit with its version and the executables it ships, and what was run to call the set
  tested. A manifest is checkable forever: at its commit every component of the workspace is
  listed, at the version its own `Cargo.toml` states there, and nothing else is.
- **The bundle**, built, never committed: `veredas-<release>-<target>.tar.gz`, holding `bin/` (the
  executables, built in release mode), `planta/` (the plant of the tester machine, below), `docs/`
  (the suite's README, its plan, every file reference `docs/*-schema.md`, the shipped components'
  plans and `docs/design/`), `release.toml` (the manifest), `sbom.cdx.json` (the component list,
  below) and `SHA256SUMS` over every file.
- **The tester machine**, part two: built from a bundle, never from a tree.

The worked example, the first manifest's shape (`tools/alforge/tests/manifest.rs` loads this block
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
- `release`: the suite release's own SemVer (Q431, ruled), unique across `releases/`, the file's
  name.
- `commit`: the full 40-hexadecimal id of the commit the bundle was built from, an ancestor of the
  commit that adds the manifest.
- `target`: the Rust target triple the executables were built for.
- `toolchain`: what `rustc --version` printed at the cut, stated, not enforced.
- `[[component]]`: `name` (a `veredas-*` or `orcl-*` member at the commit), `version` (SemVer, the
  one cargo reads at the commit), `bins` (default none: a library such as core ships none; every
  `bin` target cargo names for the component, each built with the features its executables need,
  whatever its default features are, or the cut refuses it: Q443, ruled (c). What they need is the
  union of its bins' `required-features`, as cargo states them, turned on in the component's one
  build, so Oficina's window ships with `shell` on). A bin whose features only the toolchain its
  package's folder names builds (`rust-toolchain`) is not shipped, and the cut says so: ORCL's front
  end's driver needs its pinned nightly and `rustc-dev` (Q449, raised 2026-11-27; taken meanwhile).
  Nor is a bin whose required features cargo can meet for no build of the bins, which cargo builds
  never: a feature its metadata does not name for the package, a dev-dependency's, one a
  dependency does not have (C-290's rounds 31 and 32).
- `[tested]`: `suite`, the commands whose green runs at the commit made the set tested (the whole
  suite, then each component's own tests built alone: Q444); `together`, what the cut ran from the
  bundle itself. Statements of what was run, written by the cut.

### How a release is cut (the recipe, `alforge cut <release>`)
1. **HEAD of a clean tree**, cut from a fresh `origin/main`: the cut refuses a tree that differs
   from HEAD (an untracked file included) and a release whose manifest exists. It plans the
   manifest first and checks it against the repository and every release already cut, so a
   version another release pinned at other build inputs is refused before anything is built.
   **A version names everything its build reads but the toolchain** (Q441, ruled "All build
   inputs"):
   - the component's folder, but for its top `tests/`, `benches/` and `examples/` (Q445, ruled
     "Tests left out"): what only the tests read never goes into an executable, and a file there
     that a shipped target starts from or a shipped source names is charged as a file of its own.
     None is left out when a module rustc reads could reach it: in a package with a build script
     (which may read anything in its folder, as IronCalc's xlsx lists its `tests/` to write code),
     or one whose top holds a file rustc may read as Rust (a crate root, a `mod.rs`, a `#[path]` or
     `include!` file, whose `mod examples;` is `examples/mod.rs`), nor one holding a package a
     build uses; and a package nested in another's folder is that folder's no more, its own input;
   - the folder of every member it uses, alike (its normal and build dependencies by path,
     transitively; what its tests alone use never goes into an executable), and of every package
     by path outside the members (a fork kept in the tree, `forks/ironcalc`, with its own
     workspace's manifest whole, which cargo reads for inherited keys);
   - every file outside those folders their sources name by a literal `#[path]`, `include_str!`,
     `include_bytes!` or `include!` (Ábaco's `#[path]` to core's `schema.rs`);
   - the lock file's entries for its external dependencies, transitively, each by its source and
     checksum;
   - what cargo reads of the workspace's manifest to build it (Q442, ruled "Only what cargo reads
     for it"): its `resolver`, its profiles, its `[patch]` and `[replace]`, and each
     `[workspace.package]` key, `[workspace.dependencies]` entry and `[workspace.lints]` table that
     the component or a member it uses inherits (`x.workspace = true` in its package, its normal
     and build dependencies or its lints), each named `Cargo.toml: <key>` and identified by its
     value, so the member list, comments, layout and what other members alone inherit charge
     nothing. The resolver named is the one cargo uses: the workspace's, else the root package's
     own, else its edition's. A workspace resolving with 1 is refused: under it a test's
     dependencies and a root package turn features on in what ships, which no version names;
   - and the root's cargo configuration.

   Each is identified as git stores it at the commit (a part of the manifest by its value), so two
   commits compare without a build, and a refusal names each input that differs. So a change to
   core bumps core and every component using it, a `Cargo.lock` update every component it reaches,
   a profile's setting every component, and a new member or a test nothing else. It then
   runs the whole suite there (`cargo nextest run --workspace`, the gate of `CLAUDE.md`, built
   first and run behind the machine's lock, its temporary files in the cache), and refuses a red
   one; then **each component's own tests, built alone** (Q444, ruled "Alone, and tested alone"):
   `cargo nextest run -p <component>@<version>` with the features its executables need (Q443), each built
   first and run behind the machine's lock, on the development machine, where the suite runs. Built
   alone, a component's features unify within its own inputs and its tests' dependencies, as its
   executables are built, so what ships is tested as it is built, not only with every member's
   features turned on together. A red one refuses the cut, naming the component. "Tested together"
   means two runs: the whole suite, which tests the components against each other from source, and
   the cut's own run of the bundle (step 5). A **trial cut** (`--trial`)
   skips the suite and the runs alone, says so in its manifest (`suite = "not run: a trial cut"`), is checked against
   the repository alone and writes it beside its bundle, never into `releases/`: it proves the
   recipe and is never committed. After a real cut's suite, the tree is checked again: a commit or
   an edit made while it ran refuses the cut.
2. The components, their versions and their executables read from the commit as C-290 reads
   them (its tree out of git, asked of `cargo metadata`); the recipe's data
   (`tools/alforge/maquina/maquina.toml`) from git too.
3. **Built on the oldest system the release targets**, a builder machine of the tester machine's
   own distribution (Debian 13, glibc 2.41) under QEMU with KVM, four processors and 6 GB, its
   build four jobs, holding the machine's lock for the database engine's compilation: the
   commit's tree as git stores it (its objects, no attribute or filter applied) and its
   dependencies' sources (`cargo vendor`, as `Cargo.lock` pins them) handed in on a disk as a tar
   stream, `cargo build --release --locked --offline -p <component>@<version> --bins --features <what its
   bins require>` run inside for each component alone (in one build cargo unifies features across
   the packages it builds, so one component's manifest would reach another's executables) under
   the toolchain the suite was tested with (the development machine's `rustc`), the executables
   handed back on another disk with cargo's dep-info for each (`target/release/<bin>.d`: every
   file rustc read and every file a build script declared with `rerun-if-changed`). Each file it
   names lies in what its component's version names, or the cut refuses it by name: the sources'
   scan of step 1 is proven against the compiler at every cut.

   **Each build runs traced** (Q448, ruled "Trace each build"): `strace -f` (one of the recipe's
   packages) logs every file each process of the build opens, as the kernel's own path of what it
   opened, and every program it starts. The trace comes back on the output disk as `trace/<component>.trace`, and the
   cut judges every file a process other than the builder's cargo (a cargo a build script runs is
   judged) opened to read under the tree or the vendored
   sources as it judges the dep-info's paths. So a build script's undeclared read, a header the C
   compiler it runs includes and a file a proc-macro reads while it expands are judged too.
   - Cargo's own reads are its resolution, pinned by `--locked`: every member's manifest to load
     the workspace, the lock, the vendored sums.
   - What lies outside the tree is the builder's, not a version's: the toolchain, the system's
     headers, Qt.
   - A file read without being opened is not judged: one a kernel ring reads (`io_uring`), and a
     file whose existence alone a build looks at.

   Built on the development machine (Ubuntu 26.10,
   glibc 2.44), portal and supervisão need `GLIBC_2.44` and do not start on Debian 13 (the probe
   below); built on the oldest, they run there and on every newer system. The builder's disk is
   the base image with the recipe's packages (a C and C++ compiler, `strace`, `patchelf`) and the toolchain from
   rustup's official source, built once per toolchain and kept in the cache. Each executable's
   symbol versions are then read (`objdump -p`) and held to the guest's glibc, libstdc++ and C++
   ABI as the recipe states them, so a binary that would not start on the tester machine is
   refused at the cut, by name.
4. **The bundle** assembled from git's objects at the commit and the built executables,
   `SHA256SUMS` written, packed with its entries in name order, owned by root, dated at the
   commit and every mode read as 0644 or 0755 (one commit packs to the same bytes whatever the
   umask), then extracted again and its sums checked.
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
   `target/alforge/<release>/`. No git tag marks the commit (Q432, ruled): the manifest alone
   names it.

Core ripples outward (the plan), and since Q441 any change to what a component's build reads does:
a change to core charges core and every component using it, a lock file update every component it
reaches, a part of the workspace's manifest every component that inherits it (Q442). The cut refuses a release that
pins such a component at its earlier version, before anything runs, naming each input that moved;
each member bumps its own version (alforge edits no component's crate), asked through the
orchestrator before a real cut. A bump an input forces is a PATCH by SemVer's own rule when the
component's interface does not change.

### What a release carries of its licences (Q35)
Every crate but Oficina states `MIT OR Apache-2.0` and carries `LICENSE-MIT` and `LICENSE-APACHE`
beside its manifest, the copyright holder thebdr; Oficina is `GPL-3.0-or-later` (Q35, ruled
2026-11-25). A release ships that code and what it links (no legal advice):
- **The cut refuses**, before it runs or builds anything, a release whose component list (C-295)
  holds an entry stating no `license` (nor a `license-file`; an empty one states none): a
  component, a package by path, a crate cargo resolves for them; and a package by path whose
  `license-file` the commit does not hold as a file, or which lies outside the repository. Every member states its licence since 2026-11-26, and every
  package the list holds at HEAD states one (C-294).
- **The bundle carries `licenses/`**, made from the component list:
  - for each component and each package by path, its licence files and notices at any depth of its
    folder (any name holding `LICENSE`, `LICENCE`, `COPYING`, `NOTICE` or `ATTRIBUTION`,
    `THIRD-PARTY*`, `ACKNOWLEDG*`, `AUTHORS*`, a font's `OFL*`, REUSE's `.reuse/dep5` and
    `REUSE.toml` and the like, and every file of a `LICENSES/` folder) as git stores them, in `components/<name>/`, a
    package nested in its folder left to that package; and the file its `license-file` names,
    whatever its name, wherever it lies in the repository (outside its folder, by its name at
    `components/<name>/`, as `cargo package` would put it);
  - for each crate, the same of what its vendored sources ship, in `crates/<name>-<version>/`, the
    file its `license-file` names among them;
  - with Oficina's window, Qt's texts (Debian's own LGPL-3, GPL-2 and GPL-3) and Qt's own SPDX
    documents of the modules the bundle carries, which name the third-party code inside Qt (PCRE2,
    double-conversion, libpng, libjpeg, HarfBuzz, md4c) and its licences, and KDDockWidgets' (its
    `LICENSE.txt` and every file of its `LICENSES/`), and where their sources are (Q446). Whether a
    bundle carries the texts of the licences Qt's documents name, and ICU 73's, is Q450;
  - `THIRD-PARTY.md`: every entry with its version, its licence, its source and the files kept.
- **The tester machine carries the bundle whole:** `/opt/veredas/current/licenses/`, beside the docs
  on the desktop. Its Debian system carries Debian's own (`/usr/share/doc/*/copyright`).
- **Oficina's source:** a bundle holding Oficina that is handed out goes with its corresponding
  source, namely the commit's tree and the vendored crates the builder built it from (the cut's
  builder input). Nothing is published yet (Q436, "Nowhere yet"); this binds the first place named.

### Oficina's window and Qt (Q370, Q443, Q446)
Oficina's one executable, its window, requires the `shell` feature. Under Q443 (c) the cut builds it
with that feature on, whatever Oficina's defaults say, so a release ships the window from the cut
that lands this, before or after Oficina's switch to `default = ["shell"]` (Q370).
- **The builder builds it** with Qt 6.12 and KDDockWidgets from the development machine, the paths
  `~/.cargo/config.toml` names (`~/.local/opt/Qt/6.12.0/gcc_64`, 2.4 GB, and
  `~/.local/opt/oficina-deps`). They are copied once per Qt version into a builder disk of their own,
  cached as the toolchain is, and the build's environment names `QMAKE` and `OFICINA_DEPS` there.
  They lie outside the tree: like the toolchain, they are the builder's, named by the component
  list (C-295), never by a version.
- **Both run on Debian 13:** Qt is qt.io's build (glibc 2.34, GLIBCXX 3.4.29) and KDDockWidgets needs
  glibc 2.38 and GLIBCXX 3.4.32, within the recipe's 2.41 and 3.4.33. The cut's ABI check holds the
  window and every library it ships with it to them.
- **The bundle carries them** (Q446, ruled "The bundle carries them"): `lib/qt/`, laid out as Qt
  lays itself out, holds what the window loads at run time. That is every library of Qt and
  KDDockWidgets it, its plugins and its QML modules load, as the builder's `ldd` resolves them, each
  under the name it is loaded by (`lib/qt/lib/`); the `xcb` and `offscreen` platform plugins and X's
  OpenGL integrations (`lib/qt/plugins/`); and the QML modules Oficina's folder imports, as Qt's
  own `qmlimportscanner` reads them (`lib/qt/qml/`). The window's `RUNPATH` becomes
  `$ORIGIN/../lib/qt/lib` (`patchelf` in the builder), so it loads the bundle's Qt and no other, and
  Qt finds its plugins and modules from where its own library lies, with no `qt.conf`. The builder
  then starts the window from that layout offscreen: it must still be up after ten seconds, with
  no module or library said unloaded. About 115 MB extracted (33 libraries); each release carries
  its own Qt, and an update brings a new Qt with it. `licenses/` gains Qt's and KDDockWidgets' texts
  and where their sources are (C-294). KDDockWidgets is `GPL-2.0-only OR GPL-3.0-only` (its own
  `LICENSE.txt`), not LGPLv3 as Q446's text had it; Oficina's GPLv3 or later takes it. Qt's open
  source licence is `LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only`. This copy of Qt carries no
  licence text: Debian's `/usr/share/common-licenses` holds LGPL-3, GPL-2 and GPL-3.
- **The tester machine provides** what any Qt program takes from its system (the X client
  libraries, OpenGL, fontconfig, D-Bus, GLib), from Debian's packages in the recipe. Its boot check
  says that every library the release's executables and `lib/qt/` load is found
  (`VEREDAS-LIBRARIES ok`), and that the window starts offscreen and is still up after ten seconds
  (`VEREDAS-WINDOW ok`); the proof requires both.
- **The real cut's suite run** builds the workspace on the development machine, where cargo's
  configuration names Qt, and Oficina's own tests built alone run with `shell` on.

### The component list (Q31 entry 38, "(b) Security file, component list")
Each release says what it is made of in a CycloneDX 1.6 JSON document, the form vulnerability
scanners read by package URL: `sbom.cdx.json` in the bundle, under its sums, and
`veredas-<release>-<target>.cdx.json` beside the bundle. It is not committed: `alforge components
<commit> --release <release>` gives it again, byte for byte, from the commit the manifest names.
It holds:
- the release itself: the suite, its release (or the commit alone, without `--release`), the
  commit, dated at the commit;
- one entry per component at the commit: name, version, licence expression, its folder, the
  executables it ships;
- one per package the components' builds compile for the release's target with every feature of
  theirs on, as cargo's own feature resolver has it (`cargo tree -e normal,build --target
  --no-dedupe`, offline, locked): what cargo's unit graph for the components holds, package by
  package and edge by edge, and a subset of the lock entries their versions name (C-290). A
  dependency of another platform alone, a feature only a test's dependency turns on and a weak
  feature (`dep?/f`) nothing else turns on list nothing; `cargo metadata`'s resolve would list them
  (C-295's first refute round). Nor does what `cargo tree` prints and no build compiles, and what
  lies below it: a build-dependency of a package with no build script, a dependency on a package
  with no library (rounds 2 and 3). The caller's cargo configuration steers it in nothing: its
  rustflags and its rustc are set aside, as the builder's build has none (round 4). A package by path outside the components (the IronCalc
  fork) by its folder; a crate by `pkg:cargo/<name>@<version>` (with `vcs_url` for a git one), the
  lock's checksum as its SHA-256, and its licence as its manifest states it;
- with Oficina's window, Qt and KDDockWidgets at their versions, as the recipe names them (Q446);
- the dependencies between them, as cargo resolves them.

`alforge components <commit>` prints the same list from a commit alone, so CI can produce it on
every push, as the ruling asks; the cut's list and `alforge components`'s of one commit and release
are the same bytes, and the cut's own test holds them to it. CycloneDX is the form scanners read
by purl; an SPDX document would carry the same facts. The security file, `SECURITY.md` at the root,
names GitHub's private vulnerability reporting and no address (Q447, ruled); turning it on in the
repository's settings is the architect's act. Which intent the suite states under the Cyber
Resilience Act is undecided (entry 38, ruled "Undecided, say so"), and is the suite plan's to say.

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
  (`tools/alforge/planta/`, copied into the user's home, where the tester edits it). Two services
  start at boot as the user, `veredas-bancada` (the bench) and `veredas-supervisao` (supervisão
  serving the plant), and are started again however they end (`Restart=always`; `systemctl stop` stops them); their output is in the journal. Each executable
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
  plant and a short guide (`tools/alforge/maquina/guide.md`).
- **A network that takes any card**: NetworkManager, the build's card-bound configuration removed,
  so VMware's card (its own address, NAT or bridged) comes up by DHCP.
- **The guest tools**: `open-vm-tools` and its desktop part, so VMware sizes the screen, shares
  the clipboard, keeps the clock and shuts the machine down cleanly.
- **Later**: Oficina (Qt Quick on X11 first, its foundations name a virtual machine without a GPU
  as a host, where it switches to software drawing), then each component as it reaches a release.
- **A user**, `veredas` (Q438), the only account that logs in; no SSH server.

### How the image is built (the recipe, `alforge maquina <release>`)
Everything below runs as the user, without sudo, without VMware, packer or docker (what this
machine was found able to do is the next section). The recipe's choices that are the architect's
(the distribution, the desktop, the user, the virtual hardware) are data in one stamped file,
`tools/alforge/maquina/maquina.toml` (`veredas-machine`, version 1), never code, so a ruling is an
edit of that file. A machine is built only from a cut that ran to its end: the release's manifest
written where the cut writes it (`releases/` for a real cut, the trial's folder beside its bundle)
and the bundle's own `release.toml` byte for byte (a real cut's decides over a trial's of the same
release left beside the bundle); a bundle a refused cut left behind is refused.
1. **The base**: the distribution's official cloud image (Debian 13, Q433), at a dated build
   named by URL in `maquina.toml` with its SHA-512 copied from the build's own list (served
   over HTTPS, not signed: the pin in the repository is what the recipe trusts),
   downloaded once into a cache outside the repository (`~/.cache/veredas/alforge/`) and checked
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

### How a build is proven (`alforge prova <release>`, and the machine's test)
- **The shipped disk boots on VMware's devices**: the VMDK taken out of the OVA, never the
  overlay, is booted under QEMU with KVM on VMware's own disk and network devices (QEMU emulates
  `pvscsi` and `vmxnet3`), copy-on-write so the proof changes nothing shipped. QEMU's
  `vmware-svga` is older than today's `vmwgfx` driver accepts, so the display is VMware's to
  prove; the proof boot draws on QEMU's standard VGA.
- **The network takes VMware's card**: the proof boot gives the card an address of VMware's
  prefix that the provisioning never saw, and the boot's check names the cards it sees.
- **The machine is what the provisioning promised**: the boot's check says the recipe's packages are
  installed (the guest tools among them: their working is VMware's to show), the bench and supervisão
  are services of the recipe's user, there is no SSH server,
  cloud-init is disabled and every executable of the release is on the `PATH`, and systemd says it
  initializes a machine identity (the shipped disk's is blank); the build itself refuses a
  provisioning whose log does not end finished with the root's free space trimmed.
- **The plant runs from the first boot**: from the host, through a port QEMU forwards to the
  machine's 8902, the test reads supervisão's page and the plant's object feed and sees every
  object live, the parcels move and the sorter's lines arrive; a write the binding offers is
  refused by portal's switch; the release's files are the bundle's (the boot's check of their
  sums, on the console).
- **The desktop draws the page**: screen dumps over QEMU's monitor until supervisão's own canvas
  colour (read from the release's page) covers 15 % of the screen; a desktop without the page, its
  wallpaper and panel, holds none of it, however many colours it has.
- **The OVA is what VMware reads**: `alforge prova` checks the OVA's members, descriptor and sums
  before it boots the disk, as the machine's test does, with the recipe and supervisão's page canvas
  at the release's own commit, so an older release is proven by its own; every raw header is walked,
  three plain ustar files in order, so a pax or GNU extension header is refused.
- **The machine is this cut's**: the boot's check says the digest of the bundle's `SHA256SUMS` it
  holds, and the proof requires the digest of the bundle of the cut that ran to its end, so a
  machine left from another cut of the same release is never proven as this one.
- **The package is what it says**: the OVA's members in order, the descriptor naming the disk with
  its size and capacity, the `.mf` sums matching the members.
- What only VMware proves is listed below, and stays the architect's to try.

The image test downloads the base once and needs KVM; it runs by its own command, recorded with its
clause and run at every cut, not in every run of the suite (Q440, ruled).

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
- To try it, the architect follows "The architect's trial" below. Broadcom's `ovftool` (a free
  download behind a Broadcom account) also validates the descriptor without running it.
  Apple-silicon Macs run only arm64 guests in Fusion: this image is x86_64 (Q435).

### The architect's trial (what P-022 waits on)
The OVA is `/home/thebdr/veredas-entrega/veredas-2.0.0-alpha.1.ova` (1.09 GB, never committed),
its SHA-256 beside it in `veredas-2.0.0-alpha.1.ova.sha256`; the same machine `alforge prova`
passed on 2026-11-23.
1. **Check the copy**: `sha256sum -c veredas-2.0.0-alpha.1.ova.sha256` (Linux), or on Windows
   `certutil -hashfile veredas-2.0.0-alpha.1.ova SHA256` against the file's first word.
2. **Open it**: VMware Workstation Pro 17 on Windows or Linux (free), File > Open, the OVA, a name
   and a folder, Import; the network adapter stays NAT. Fusion 13 on an Intel Mac: File > Import.
   ESXi 7 or 8: Create / Register VM > Deploy a virtual machine from an OVF or OVA file. A warning
   at import is worth its exact text even when Retry gets past it.
3. **First boot** (Power on): within two minutes Debian logs in by itself as `veredas` (password
   `veredas`, asked only by `sudo`) to an Xfce desktop, and Chromium opens full-window on the plant,
   `http://127.0.0.1:8902/s/loop/`: parcels moving along the loop, outlets filling; "Release" is
   offered and refused (writes are off).
4. **Look at** (a terminal: the Applications menu > Terminal Emulator):
   - the display: resizing VMware's window (or View > Fit Guest Now) resizes the desktop, and
     `lsmod | grep vmwgfx` names VMware's display driver;
   - the devices: `lsmod | grep -E 'vmw_pvscsi|vmxnet3'` names both;
   - the guest tools: VMware's summary of the machine shows its tools running and its IP address;
   - the boot's check: `journalctl -b -u veredas-verifica --no-pager` says `VEREDAS-SUMS-OK
     2.0.0-alpha.1` and `ok`, `absent` or `disabled` on each of its other lines;
   - the network: in the host's own browser, `http://<the machine's address>:8902/s/loop/` (the
     address from the summary, or `ip -4 addr` in the machine) shows the same plant, live;
   - the shutdown: VM > Power > Shut Down Guest powers the machine off cleanly.
5. **If a step fails, send back**: the product and its version (Help > About), the step, a
   screenshot, and the output of `journalctl -b --no-pager -u veredas-verifica -u veredas-bancada
   -u veredas-supervisao`, `lsmod | grep -E 'vmwgfx|vmw_pvscsi|vmxnet3'` and `ip -4 addr`; for an
   import or a boot that fails, the `vmware.log` in the machine's folder.

### Delivery and updates
- **An OVA**, one file every VMware product opens, VirtualBox too: 1.09 GB for 2.0.0-alpha.1
  (the disk 16 GB to the guest, 1.09 GB stream-optimized; provisioned in about 12 minutes).
  Published nowhere yet (Q436, ruled "OVA" and "Nowhere yet"): it is built and kept on this
  machine and the architect hands it over, the one to try in `/home/thebdr/veredas-entrega/`,
  outside the repository and every build folder.
- **Updates** (Q437, ruled "Update command"): `sudo veredas-atualizar <bundle.tar.gz>` in the
  machine (C-293), a Python 3 script the provisioning installs in `/usr/local/sbin`. Nothing is
  published (Q436), so it fetches nothing: it takes a bundle the tester has copied into the machine
  (dragged onto the desktop, or downloaded in Chromium: VMware's to try) and prints the archive's
  SHA-256 to compare with the one handed over, and its `SHA256SUMS`' digest, which the boot's check
  says as `VEREDAS-BUNDLE`; once a place is named it can fetch from it. Bundles are handed over as
  the OVA is, when a newer release exists and the orchestrator calls it (none is kept beside the OVA). It installs the bundle only if
  every file matches `SHA256SUMS`, its manifest is a `veredas-release` of the machine's target,
  newer than the installed release (SemVer's precedence) and naming exactly the executables in
  `bin/`, and every executable finds its libraries on the machine (`ldd`); then it moves it beside
  the old one under `/opt/veredas/`, points `current` at it in one step (a relative link, renamed
  over the old one), relinks the PATH (a link for each new executable, none left for one it
  dropped) and restarts the bench and supervisão on it. Its files are synced to the disk before
  `current` names them, and the switch is synced before it is said, so a power-off leaves one
  release or the other whole. The plant's files in the home folder are
  left as they are; the new release's own plant is in `/opt/veredas/current/planta`. Anything
  refused leaves the machine as it was, one update runs at a time (the release's folder locked
  to its end, the services' watch included), and a restart that fails says how to point `current`
  back. What a run killed midway leaves (a reboot or a closed terminal during the read: its
  staging folder `/opt/veredas/.atualizar-*`, a scratch link) is removed and said by the next run,
  under the lock. An interruption (Ctrl-C, a closed terminal, a shutdown) before the install is a
  refusal; the install's few renames hold it until they are done; during the services' watch it is
  answered with the way back. The boot's check names the release
  `current` names, and says the command is there (`VEREDAS-ATUALIZAR ok`). A trial's bundle is
  never installed. The command watches the services after the restart (active, one main process,
  at two looks): a release that refuses the kept plant exits and is restarted by systemd, and the
  command says so with the way back (exit 3). Each old release stays (about 160 MB extracted on a
  16 GB disk); nothing removes them. What a bundle cannot carry (the command itself, the services,
  the packages, the desktop) still comes with a new OVA.

### What a cut and a machine cost in disk, and what is kept
Measured on release 2.0.0-alpha.1 (2026-11-22):
- **Kept across cuts, in the cache** (`~/.cache/veredas/alforge/`, outside the repository): the base
  image (435 MB, once) and the builder machine's disk (1.6 GB, once per toolchain).
- **During a cut**, under `target/alforge/<release>/`: the commit's tree and its vendored
  dependencies (about 0.7 GB, and their tar stream as much), the builder's copy-on-write disk while
  it builds and the output disk (sparse); all removed once the executables are out.
- **Kept after a cut**: the bundle (48 MB), its extracted check (161 MB) and the executables as
  built (160 MB), the run's copy of the plant (small).
- **During a machine's build**: its copy-on-write disk (about 2.8 GB) and the VMDK, removed once
  the OVA is packed; **kept**: the OVA alone (1.09 GB). The proof's copy of the disk taken out of
  the OVA (1.09 GB) is removed after `alforge prova` reports. The proven OVA the architect tries
  is copied, with its SHA-256, to `/home/thebdr/veredas-entrega/` (1.09 GB), which no reset of a
  build folder touches.
- A superseded or failed cut's folder is removed whole; a trial's, by its test when it passes.

## Rules
- **A release pins a tree.** One release, one commit; a manifest checkable at any later commit
  against git's history, as L6 judges history: the tree read as git stores it, replace refs and
  grafts off, and holding no link and no path that is not UTF-8.
  The check keeps the repository's own record honest: a manifest the cut wrote, in the suite's
  repository with its laws green. A hostile history or environment is outside its promise, though
  every one a critic built has been refused by name (`.zen/evidence/C-290.md`).
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
  `tools/alforge/tests/`, one file per capability: no mock of QEMU, of the guest or of the bundle.
- **Typed errors** in the tool, no `unwrap`/`expect` on an I/O path; tests may unwrap.

## Clauses ahead
- **C-290**, the manifest: the format and its refusals, and a manifest checked against the
  repository's history (`tests/manifest.rs`).
- **C-291**, the cut: the bundle built in release mode from a commit, its sums, and its
  executables run together against the bench on the plant it ships (`tests/cut.rs`).
- **C-292**, the tester machine: the OVA built by the recipe from a bundle, its disk booted under
  QEMU on VMware's devices, the plant live from the first boot and reached from the host
  (`tests/maquina.rs`).
- **C-293**, the update command: a newer release's bundle checked and installed beside the old one
  (`tests/atualizar.rs`), and run inside the machine C-292 builds (`tests/maquina.rs`: it rides on
  that machine rather than building a second, so a change to that file re-stales both clauses).
- **C-294**, a release's licences: the cut's refusal of a component or crate stating none, and the
  bundle's `licenses/` (Q35). Every member states its licence since 2026-11-26.
- **C-295**, the component list: the cut's CycloneDX document and `alforge components <commit>`,
  which agree on one commit (Q31 entry 38).

The rulings of 2026-11-26 (Q442 to Q448) change what four verified clauses state, and each such
clause takes a fresh record and a refute pass before it is verified again:
- C-290: a version's folders without their tests (Q445), the workspace manifest's parts (Q442),
  every bin shipped with the features it requires (Q443);
- C-291: each build with those features (Q443) and traced (Q448), each component's tests run alone
  (Q444), Oficina's window built with Qt and shipped with its `lib/` (Q446);
- C-292: the window's libraries loading on the tester machine (Q446);
- C-293 states nothing these change, its record held to its own files.

## What a component is asked for
Through the orchestrator: before every real cut, a version bump of each component whose build
inputs moved since the release before (Q441; the cut names them, and for 2.0.0-alpha.2 that is every
component 2.0.0-alpha.1 pins). None was needed for the first image:
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
  a disk, and its tail is echoed to the console after. The same race lost the tester machine's
  boot check (`veredas-verifica`, output to journal and console) on 2026-11-23, after five proofs
  that won it: a unit that speaks on the console at boot is ordered before
  `serial-getty@ttyS0.service`.
- **`/tmp` is a tmpfs of the machine's memory, shared by every session**, and its quota for the
  user ran out at 20 GB ("Disk quota exceeded"). Disks, bundles and OVAs go under `target/alforge/`
  or the cache, never in `/tmp`. A `TMPDIR` moved elsewhere must lie outside every git repository:
  under the worktree's `target/`, a law's test of "a tree without git is refused" found the
  worktree's repository around its tree and failed the cut's suite. The cut's suite takes
  `~/.cache/veredas/alforge/tmp`.
- **Debian's image has no `strings`** (binutils is not installed): what the guest is asked is
  answered with its own tools, or read on the host.
- **A clone never builds into the worktree's `target/`.** Cargo names a member's artifacts by its
  path inside the workspace, so a mutant built in a clone with `CARGO_TARGET_DIR` set to the
  worktree's folder leaves its library and executable there under the worktree's own names, and
  the worktree's next run tests the mutant (2026-11-23: two of `tests/bundle.rs` red on correct
  code until `cargo clean -p alforge`). A clone or a critic gets a target folder of its own, seeded
  from `~/.cache/veredas/target-seed`.
- **A cached disk names its base by absolute path.** Moving the cache (the rename moved
  `~/.cache/veredas/entrega` to `alforge`, 2026-11-25) left the builder's disk pointing at a base
  that was no longer there, and the cut stopped at `qemu-img create`. A cached builder now has its
  backing file rewritten to the base the cache holds (`builder::reuse`, `qemu-img rebase -u`: its
  key holds the base's digest, so the base is the one it was made on).
- **Typing into a machine with no SSH**: the serial console as a socket QEMU waits on
  (`server=on,wait=on`, under the cache's `tmp/`: a socket's path holds at most 107 bytes), a
  login as the recipe's user, and each marker typed split by quotes so the echo of a command never
  reads as its output (C-293's proof).
