//! The builder machine (docs/alforge-plan.md, "How a release is cut", step 3): the release's
//! executables built on the oldest system the release targets, the tester machine's own
//! distribution, so they start there and on every newer system. The commit's tree and its
//! dependencies' sources go in on one disk as a tar stream, the build runs offline, the
//! executables come back on another.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cache;
use crate::machine::Recipe;
use crate::vm::{self, DataDisk, Hardware, Spec, Vm, VmError};

/// Bumped when the builder's provisioning changes, so a cached builder of the old kind is not used.
const PROVISIONING: u32 = 1;

/// A cached builder's disk made to name `base` as its backing file. The disk names its base by
/// absolute path, so a cache moved (the member's rename moved `~/.cache/veredas/entrega` to
/// `alforge`) leaves it pointing where the base no longer is; its key holds the base's digest, so
/// the base it was made on is this one, and only the name in its header is rewritten
/// (`qemu-img rebase -u`).
pub fn reuse(disk: &Path, base: &Path) -> Result<(), VmError> {
    let info = vm::run("qemu-img", &["info", "--output=json", &disk.display().to_string()], None)?;
    let backing = serde_json::from_str::<serde_json::Value>(&info).ok().and_then(|v| v["backing-filename"].as_str().map(PathBuf::from));
    if backing.as_deref() != Some(base) {
        vm::run("qemu-img", &["rebase", "-u", "-F", "qcow2", "-b", &base.display().to_string(), &disk.display().to_string()], None)?;
    }
    Ok(())
}

/// The builder's disk for a toolchain (`1.100.0`), built once and kept in the cache: the base, the
/// recipe's packages, and the toolchain from rustup's official source under `/opt`.
pub fn image(r: &Recipe, toolchain: &str, work: &Path) -> Result<PathBuf, VmError> {
    let base = cache::base_image(r)?;
    let key = cache::sha256_text(&format!("{PROVISIONING}\n{}\n{}\n{toolchain}\n", r.base.sha512, r.builder.packages.join(" ")));
    let path = cache::dir()?.join(format!("builder-{toolchain}-{}.qcow2", &key[..16]));
    if path.exists() {
        reuse(&path, &base)?;
        return Ok(path);
    }
    eprintln!("alforge: building the builder machine for rustc {toolchain} (once)");
    std::fs::create_dir_all(work)?;
    let part = path.with_extension("part.qcow2");
    vm::overlay(&base, &part, r.builder.disk_gb)?;
    let packages: String = r.builder.packages.iter().map(|p| format!("  - {p}\n")).collect();
    let user_data = format!(
        "#cloud-config\npackage_update: true\npackages:\n{packages}runcmd:\n  - [ sh, -c, 'set -e; export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo; curl -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh; sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain {toolchain} > /dev/ttyS0 2>&1; /opt/cargo/bin/rustc --version > /dev/ttyS0; /opt/cargo/bin/cargo --version > /dev/ttyS0; echo ALFORGE-BUILDER-OK > /dev/ttyS0' ]\npower_state:\n  mode: poweroff\n  delay: now\n  timeout: 120\n"
    );
    let seed = work.join("builder-seed.iso");
    vm::seed(&work.join("builder-seed"), &seed, &user_data, &format!("alforge-builder-{}", &key[..16]))?;
    let console = work.join("builder-console.log");
    let mut m = Vm::start(&Spec {
        disk: part.clone(),
        hardware: Hardware::Virtio,
        cpus: r.builder.cpus,
        memory_mb: r.builder.memory_mb,
        seed: Some(seed),
        data: vec![],
        forwards: vec![],
        snapshot: false,
        mac: None,
        qmp: None,
        console: console.clone(),
        serial_in: false,
    })?;
    m.wait_off(Duration::from_secs(45 * 60))?;
    let text = vm::read_console(&console);
    if !text.contains("ALFORGE-BUILDER-OK") || !text.contains(&format!("rustc {toolchain}")) {
        return Err(VmError::Failed {
            program: "the builder's provisioning".into(),
            status: "no ALFORGE-BUILDER-OK".into(),
            stderr: vm::tail(&text, 40),
        });
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// Where the builder holds the commit's tree, and the vendored sources of its lock file: the
/// folders a dep-info file names (`crate::inputs::read_outside`).
pub const SRC: &str = "/build/src";
pub const VENDOR: &str = "/build/vendor";

/// What the builder builds of one component, alone: its executables, or, for a component that ships
/// none, its library, so every component's build is held to its inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub package: String,
    /// Its version, which names it to cargo with its name (`-p veredas-core@2.1.0`): a package by
    /// path of the same name at another version may share the graph.
    pub version: String,
    /// Its folder in the tree (`crates/veredas-oficina`), whose QML a Qt program imports.
    pub dir: String,
    pub bins: Vec<String>,
    /// Its library target's name, built (`--lib`) when it ships no executable.
    pub lib: Option<String>,
    /// The features its executables need, turned on in its build whatever its defaults are
    /// (docs/questions.md Q443).
    pub features: Vec<String>,
}

/// A word quoted for the shell, whatever it holds.
fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// What turns `package`'s required `features` on in its build, run from the workspace's root: each
/// of its own features named `<package>/<feature>` (a root package of the workspace would take a
/// bare one as its own), a dependency's (`dep/feature`) as cargo states it.
pub fn feature_args(package: &str, features: &[String]) -> Vec<String> {
    features.iter().flat_map(|f| ["--features".to_string(), if f.contains('/') { f.clone() } else { format!("{package}/{f}") }]).collect()
}

/// The script the builder machine runs on the commit's tree: the toolchain's and the C library's
/// versions written down, each component of `builds` built alone, traced ([`crate::trace::STRACE`],
/// its log `/build/out/trace/<component>.trace`), with `cargo build --release
/// --locked --offline -p <component> --bins`, each feature its executables need turned on
/// (`--features`, docs/questions.md Q443; `--lib` for one that ships no executable), so cargo
/// unifies features within its own inputs, never across components, as many jobs as the builder
/// has processors, no name under `/build` holding a control character once they are built
/// ([`CONTROL_CHECK`]), and every executable copied out with cargo's dep-info for it (a library's
/// dep-info alone); a bin cargo did not build is named, never skipped, and fails the script, so the
/// cut refuses a release that would ship less than its components name.
pub fn script(r: &Recipe, builds: &[Build]) -> String {
    // A component with executables builds them; one without builds its library, whose dep-info
    // then holds the build to its inputs.
    let cargo: String = builds
        .iter()
        .filter_map(|b| {
            let what = match (b.bins.is_empty(), b.lib.is_some()) {
                (false, _) => format!("--bins{}", feature_args(&b.package, &b.features).iter().map(|w| format!(" {}", if w == "--features" { w.clone() } else { quoted(w) })).collect::<String>()),
                (true, true) => "--lib".to_string(),
                (true, false) => return None,
            };
            // Traced (docs/questions.md Q448): the log is the component's, beside its executables.
            Some(format!("{} /build/out/trace/{}.trace cargo build --release --locked --offline --jobs {} -p {}@{} {what}\n", crate::trace::STRACE, b.package, r.builder.cpus, b.package, b.version))
        })
        .collect();
    let copies: String = builds
        .iter()
        .flat_map(|b| {
            let lib = b.lib.iter().filter(|_| b.bins.is_empty()).map(|l| format!("cp target/release/lib{l}.d /build/out/deps/\n"));
            b.bins
                .iter()
                .map(|b| format!("if [ -f target/release/{b} ]; then cp target/release/{b} /build/out/bin/; cp target/release/{b}.d /build/out/deps/; else echo \"ALFORGE-MISSING {b}\"; missing=1; fi\n"))
                .chain(lib)
                .collect::<Vec<_>>()
        })
        .collect();
    let qt: String = builds.iter().flat_map(|b| b.bins.iter().map(move |bin| format!("stage_qt {} {}\n", quoted(bin), quoted(&b.dir)))).collect();
    format!(
        "set -eu\nexport RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\
         cd {SRC}\nmkdir -p /build/out/bin /build/out/deps /build/out/trace\nrustc --version > /build/out/toolchain\nldd --version | head -1 > /build/out/libc\n\
         {QT_ENV}\n{QT_STAGE}\n{cargo}{CONTROL_CHECK}\nmissing=\n{copies}\
         [ -z \"$missing\" ]\n{qt}echo ALFORGE-BUILD-OK\n"
    )
}

/// The builder script's line naming Qt and KDDockWidgets for Oficina's window, when their disks are
/// mounted ([`qt_disks`]): `QMAKE` and `OFICINA_DEPS`, which its build script and cxx-qt's read.
pub const QT_ENV: &str = "if [ -x /opt/qt/qt/bin/qmake ]; then export QMAKE=/opt/qt/qt/bin/qmake OFICINA_DEPS=/opt/qt/deps; fi";

/// The builder script's function run, once every executable is copied out, on each (`stage_qt <bin>
/// <its component's folder>`) and doing its work for one that loads Qt (its `ldd` naming
/// `/opt/qt`): what it loads at run time staged in `/build/out/qt/` as Qt lays itself
/// out (`lib/`, `plugins/`, `qml/`), so the bundle's `lib/qt/` holds it (docs/questions.md Q446):
/// the X and offscreen platform plugins and X's OpenGL integrations; the QML modules its
/// component's folder imports, as Qt's own `qmlimportscanner` reads them, each module's own files and its
/// folders that are no module; every library of Qt and KDDockWidgets it, those plugins and those
/// modules load, under the name each is loaded by (KDDockWidgets' looking beside itself). The
/// libraries they take from the system are listed (`/build/out/qt-system-needs`), and the
/// executable's `RUNPATH` becomes `$ORIGIN/../lib/qt/lib`, so it loads the bundle's Qt and no
/// other; Qt finds its plugins and modules from where its own library lies. Then the executable is
/// started from that layout, offscreen, for ten seconds: what it says and how it ended are logged
/// (`ALFORGE-QT-SAYS`, `ALFORGE-QT-RUN <name> <code>`), for [`outcome`] to judge.
pub const QT_STAGE: &str = r#"stage_qt() {
exe=/build/out/bin/$1
[ -d /opt/qt/qt/lib ] || return 0
ldd "$exe" 2>/dev/null | grep -q '=> /opt/qt/' || return 0
q=/build/out/qt
name=$1
mkdir -p "$q/lib" "$q/plugins/platforms" "$q/plugins/xcbglintegrations" "$q/qml"
cp /opt/qt/qt/plugins/platforms/libqxcb.so /opt/qt/qt/plugins/platforms/libqoffscreen.so "$q/plugins/platforms/"
cp /opt/qt/qt/plugins/xcbglintegrations/*.so "$q/plugins/xcbglintegrations/"
/opt/qt/qt/libexec/qmlimportscanner -rootPath "/build/src/$2" -importPath /opt/qt/qt/qml > /build/out/qt-imports.json
for m in $(grep -o '"path": *"/opt/qt/qt/qml/[^"]*"' /build/out/qt-imports.json | sed 's/^"path": *"//; s/"$//' | sort -u); do
rel=${m#/opt/qt/qt/qml/}
mkdir -p "$q/qml/$rel"
find "$m" -maxdepth 1 -type f -exec cp {} "$q/qml/$rel/" \;
for d in "$m"/*/; do if [ -d "$d" ] && [ ! -f "$d/qmldir" ]; then cp -r "$d" "$q/qml/$rel/"; fi; done
done
loaded=$(cd "$q" && find plugins qml -name '*.so' | sed 's#^#/opt/qt/qt/#')
for f in "$exe" $loaded; do ldd "$f" | awk '$2 == "=>" && $3 ~ /^\/opt\/qt\// {print $1, $3}'; done | sort -u > /build/out/qt-libs
while read -r so path; do cp -L "$path" "$q/lib/$so"; case "$path" in /opt/qt/deps/*) patchelf --set-rpath '$ORIGIN' "$q/lib/$so";; esac; done < /build/out/qt-libs
for f in "$exe" $loaded; do ldd "$f" | awk '$2 == "=>" && $3 !~ /^\/opt\/qt\// {print $1}'; done | sort -u > /build/out/qt-system-needs
patchelf --set-rpath '$ORIGIN/../lib/qt/lib' "$exe"
rm -rf /build/try && mkdir -p /build/try/bin /build/try/lib && cp "$exe" /build/try/bin/ && cp -r "$q" /build/try/lib/qt
rc=0
QT_QPA_PLATFORM=offscreen timeout 10 /build/try/bin/"$name" > /build/out/qt-run.log 2>&1 || rc=$?
sed 's/^/ALFORGE-QT-SAYS /' /build/out/qt-run.log
echo "ALFORGE-QT-RUN $name $rc"
}"#;

/// The builder script's line run after the builds: a name under `/build` that cargo's reading of
/// rustc's dep-info misreads fails it. That is a control character, C1's included (a newline,
/// which a build script may write, ends the reading), a backslash or a final space (a name ending
/// in either joins the next path), or a whitespace other than a space (Unicode's, U+00A0 among
/// them, splits a name).
/// Each is matched as UTF-8's bytes.
pub const CONTROL_CHECK: &str = "if [ -n \"$(LC_ALL=C find /build \\( -name \"$(printf '*[\\001-\\037\\177]*')\" -o -name '*\\\\*' -o -name '* ' -o -name \"$(printf '*\\302[\\200-\\240]*')\" -o -name \"$(printf '*\\341\\232\\200*')\" -o -name \"$(printf '*\\342\\200[\\200-\\212\\250\\251\\257]*')\" -o -name \"$(printf '*\\342\\201\\237*')\" -o -name \"$(printf '*\\343\\200\\200*')\" \\) -print -quit)\" ]; then echo ALFORGE-CONTROL-NAME; exit 1; fi";

/// What a build's [`script`] log (brought back on the output disk, at `log_path`) and the builder's
/// console say: every bin cargo did not build refused by name; a build that did not finish (no
/// `ALFORGE-BUILD-OK` in its log, no `ALFORGE-BUILD-EXIT 0` on the console, or the output disk
/// unread) refused with the log's tail, or the console's when there is no log.
pub fn outcome(log: &str, console: &str, unpacked: bool, log_path: &Path) -> Result<(), VmError> {
    if log.lines().any(|l| l == "ALFORGE-CONTROL-NAME") {
        return Err(VmError::Failed {
            program: "the build in the builder machine".into(),
            status: "refused".into(),
            stderr: "a name under /build holds a control character, a backslash or a whitespace other than a space, or ends in a space, after the build: cargo's dep-info would misread it and hide reads after it".into(),
        });
    }
    // A window that did not stay up from the bundle's layout, or said a module or a library did
    // not load (`timeout`'s 124 is a window still running at its end).
    let said: Vec<&str> = log.lines().filter_map(|l| l.strip_prefix("ALFORGE-QT-SAYS ")).collect();
    let faults: Vec<&&str> = said
        .iter()
        .filter(|l| {
            let l = l.to_lowercase();
            ["is not installed", "cannot load library", "could not be loaded", "failed to load", "no such file", "cannot open shared object"].iter().any(|w| l.contains(w))
        })
        .collect();
    for run in log.lines().filter_map(|l| l.strip_prefix("ALFORGE-QT-RUN ")) {
        let (name, code) = run.rsplit_once(' ').unwrap_or((run, ""));
        if code != "124" || !faults.is_empty() {
            return Err(VmError::Failed {
                program: "the window's run in the builder machine".into(),
                status: format!("`{name}` ended with {code} within ten seconds, offscreen, from the bundle's layout"),
                stderr: said.join("\n"),
            });
        }
    }
    let missing: Vec<&str> = log.lines().filter_map(|l| l.strip_prefix("ALFORGE-MISSING ")).collect();
    if !missing.is_empty() {
        return Err(VmError::Failed {
            program: "the build in the builder machine".into(),
            status: "refused".into(),
            stderr: missing.iter().map(|b| format!("cargo built no `{b}` with the features its executables need: the release would not ship it")).collect::<Vec<_>>().join("\n"),
        });
    }
    if !unpacked || !log.contains("ALFORGE-BUILD-OK") || !console.contains("ALFORGE-BUILD-EXIT 0") {
        return Err(VmError::Failed {
            program: "the build in the builder machine".into(),
            status: format!("no ALFORGE-BUILD-OK (its log: {})", log_path.display()),
            stderr: if log.is_empty() { vm::tail(console, 60) } else { vm::tail(log, 60) },
        });
    }
    Ok(())
}

/// Bumped when the Qt disks' making changes, so a cached disk of the old kind is not used.
const QT_DISKS: u32 = 1;

/// Where the development machine holds Qt and KDDockWidgets: the folder of the qmake cargo's
/// configuration names (`QMAKE`, its `bin/` parent) and `OFICINA_DEPS`, else their defaults
/// (`~/.local/opt/Qt/<version>/gcc_64`, `~/.local/opt/oficina-deps`), each held to the recipe's
/// version: Qt's by its qmake, KDDockWidgets' by its CMake package.
pub fn qt_folders(q: &crate::machine::Qt) -> Result<(PathBuf, PathBuf), VmError> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let qt = std::env::var_os("QMAKE").map(PathBuf::from).and_then(|m| m.parent().and_then(Path::parent).map(Path::to_path_buf)).unwrap_or_else(|| home.join(format!(".local/opt/Qt/{}/gcc_64", q.version)));
    let deps = std::env::var_os("OFICINA_DEPS").map(PathBuf::from).unwrap_or_else(|| home.join(".local/opt/oficina-deps"));
    let found = vm::run(&qt.join("bin/qmake").display().to_string(), &["-query", "QT_VERSION"], None)?;
    if found.trim() != q.version {
        return Err(VmError::Failed { program: "the builder's Qt".into(), status: "another version".into(), stderr: format!("{} is Qt {}, the recipe names {}", qt.display(), found.trim(), q.version) });
    }
    let cmake = deps.join("lib/cmake/KDDockWidgets-qt6/KDDockWidgets-qt6ConfigVersion.cmake");
    let text = std::fs::read_to_string(&cmake)?;
    let kdd = text.lines().find_map(|l| l.trim().strip_prefix("set(PACKAGE_VERSION \"").and_then(|r| r.strip_suffix("\")"))).unwrap_or_default();
    if kdd != q.kddockwidgets {
        return Err(VmError::Failed { program: "the builder's KDDockWidgets".into(), status: "another version".into(), stderr: format!("{} is KDDockWidgets {kdd}, the recipe names {}", deps.display(), q.kddockwidgets) });
    }
    Ok((qt, deps))
}

/// The builder's disks of Qt and KDDockWidgets, when the recipe names them: each folder made into a
/// read-only ext4 disk (`mke2fs -d`, no privilege), once per version, kept in the cache; the
/// builder mounts them at `/opt/qt/qt` and `/opt/qt/deps`. None when the recipe names no Qt.
pub fn qt_disks(r: &Recipe) -> Result<Vec<DataDisk>, VmError> {
    let Some(q) = &r.builder.qt else { return Ok(Vec::new()) };
    let (qt, deps) = qt_folders(q)?;
    let mut disks = Vec::new();
    for (what, dir, serial) in [("qt", &qt, "alforge-qt"), ("kddockwidgets", &deps, "alforge-qtdeps")] {
        let key = cache::sha256_text(&format!("{QT_DISKS}\n{what}\n{}\n{}\n", q.version, q.kddockwidgets));
        let path = cache::dir()?.join(format!("{what}-{}-{}.ext4", if what == "qt" { &q.version } else { &q.kddockwidgets }, &key[..16]));
        if !path.exists() {
            eprintln!("alforge: making the builder's {what} disk from {} (once)", dir.display());
            let used = vm::run("du", &["-sk", &dir.display().to_string()], None)?;
            let kib: u64 = used.split_whitespace().next().and_then(|k| k.parse().ok()).unwrap_or(0);
            let part = path.with_extension("part");
            let _ = std::fs::remove_file(&part);
            vm::run("mke2fs", &["-q", "-t", "ext4", "-m", "0", "-E", "root_owner=0:0", "-d", &dir.display().to_string(), &part.display().to_string(), &format!("{}k", kib + kib / 4 + 131_072)], None)?;
            std::fs::rename(&part, &path)?;
        }
        disks.push(DataDisk { file: path, serial: serial.into(), writable: false });
    }
    Ok(disks)
}

/// What a build handed back.
#[derive(Debug)]
pub struct Built {
    /// The folder holding `bin/`, `toolchain` and `libc`.
    pub out: PathBuf,
    /// What `rustc --version` printed in the builder.
    pub toolchain: String,
    /// What `ldd --version` printed first in the builder.
    pub libc: String,
}

/// The vendoring of the lock file: one folder a crate at its version (`vendor/<name>-<version>/`),
/// which the judgement of a build's reads matches exactly (`crate::inputs::Packages::vendored`).
pub const VENDOR_ARGS: [&str; 4] = ["vendor", "--locked", "--versioned-dirs", "../vendor"];

/// `cargo vendor`'s stanza (written from the tree's folder, `directory = "../vendor"`) as it reads
/// from the folder above the tree, where the builder writes it: cargo takes a configuration's
/// relative directory from the folder holding its `.cargo`.
pub fn vendor_config(stanza: &str) -> String {
    stanza.replace("directory = \"../vendor\"", "directory = \"vendor\"")
}

/// The builder's input at `input` (emptied first): the commit's tree as git stores it in `src/`,
/// the `cargo vendor` of its lock file in `vendor/`, and the vendored sources' stanza in
/// `.cargo/config.toml` beside the tree, never over a configuration the commit holds (cargo merges
/// the folders' configurations upward from where it runs).
pub fn prepare(root: &Path, commit: &str, input: &Path) -> Result<(), VmError> {
    let _ = std::fs::remove_dir_all(input);
    std::fs::create_dir_all(input.join("src"))?;
    crate::git::export_tree(root, commit, &[], &input.join("src")).map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?;
    eprintln!("alforge: vendoring the dependencies Cargo.lock pins");
    let config = vm::run("cargo", &VENDOR_ARGS, Some(&input.join("src")))?;
    std::fs::create_dir_all(input.join(".cargo"))?;
    std::fs::write(input.join(".cargo/config.toml"), vendor_config(&config))?;
    Ok(())
}

/// Builds each of `builds`' components' executables at `commit` of the repository at `root` in a fresh copy of the
/// builder: the commit's tree as git stores it, `cargo vendor` of its lock file, `cargo build --release
/// --locked --offline` inside, the executables named in `bins` handed back.
pub fn build(r: &Recipe, root: &Path, commit: &str, toolchain: &str, builds: &[Build], work: &Path) -> Result<Built, VmError> {
    let builder = image(r, toolchain, work)?;
    let input = work.join("in");
    prepare(root, commit, &input)?;
    std::fs::write(input.join("build.sh"), script(r, builds))?;
    let in_tar = work.join("in.tar");
    vm::run("tar", &["-cf", &in_tar.display().to_string(), "-C", &input.display().to_string(), "."], None)?;
    let out_raw = work.join("out.raw");
    let f = std::fs::File::create(&out_raw)?;
    f.set_len(4 << 30)?;
    drop(f);
    let disk = work.join("build.qcow2");
    vm::overlay(&builder, &disk, r.builder.disk_gb)?;
    // The build logs to a file, which goes back on the output disk whatever the outcome, and its
    // tail is echoed to the console afterwards: agetty hangs up ttyS0 when it starts, so a
    // descriptor opened on it before then writes nothing (docs/alforge-plan.md, "Traps").
    // Qt's disks, when the recipe names Qt, mounted read-only where the script looks for them.
    let qt = qt_disks(r)?;
    let user_data = "#cloud-config\nruncmd:\n  - [ sh, -c, 'mkdir -p /build/out && if [ -e /dev/disk/by-id/virtio-alforge-qt ]; then mkdir -p /opt/qt/qt /opt/qt/deps && mount -o ro /dev/disk/by-id/virtio-alforge-qt /opt/qt/qt && mount -o ro /dev/disk/by-id/virtio-alforge-qtdeps /opt/qt/deps; fi; tar -xf /dev/disk/by-id/virtio-alforge-in -C /build && sh /build/build.sh > /build/out/build.log 2>&1; rc=$?; tail -n 40 /build/out/build.log > /dev/ttyS0; tar -cf /dev/disk/by-id/virtio-alforge-out -C /build/out . && sync; echo \"ALFORGE-BUILD-EXIT $rc\" > /dev/ttyS0' ]\npower_state:\n  mode: poweroff\n  delay: now\n  timeout: 120\n";
    let nonce = cache::sha256_text(&format!("{commit}{:?}", std::time::SystemTime::now()));
    let seed = work.join("build-seed.iso");
    vm::seed(&work.join("build-seed"), &seed, user_data, &format!("alforge-build-{}", &nonce[..16]))?;
    let console = work.join("build-console.log");
    // The build compiles the database engine: one such build at a time on the host.
    let _engine = vm::engine_lock()?;
    eprintln!("alforge: building in the builder machine ({} processors); its console is {}", r.builder.cpus, console.display());
    let mut m = Vm::start(&Spec {
        disk: disk.clone(),
        hardware: Hardware::Virtio,
        cpus: r.builder.cpus,
        memory_mb: r.builder.memory_mb,
        seed: Some(seed),
        data: [DataDisk { file: in_tar.clone(), serial: "alforge-in".into(), writable: false }, DataDisk { file: out_raw.clone(), serial: "alforge-out".into(), writable: true }].into_iter().chain(qt).collect(),
        forwards: vec![],
        snapshot: false,
        mac: None,
        qmp: None,
        console: console.clone(),
        serial_in: false,
    })?;
    m.wait_off(Duration::from_secs(90 * 60))?;
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    let unpacked = vm::run("tar", &["-xf", &out_raw.display().to_string(), "-C", &out.display().to_string()], None);
    let log = std::fs::read_to_string(out.join("build.log")).unwrap_or_default();
    let text = vm::read_console(&console);
    outcome(&log, &text, unpacked.is_ok(), &out.join("build.log"))?;
    let toolchain = std::fs::read_to_string(out.join("toolchain"))?.trim().to_string();
    let libc = std::fs::read_to_string(out.join("libc"))?.trim().to_string();
    for f in [&disk, &out_raw, &in_tar] {
        let _ = std::fs::remove_file(f);
    }
    // The vendored crates' licence files, which the bundle carries (crate::licences), kept before
    // their sources go.
    crate::licences::keep_vendored(&input.join("vendor"), &work.join("vendored-licences"))?;
    let _ = std::fs::remove_dir_all(&input);
    Ok(Built { out, toolchain, libc })
}
