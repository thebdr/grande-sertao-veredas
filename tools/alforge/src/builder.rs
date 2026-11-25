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

/// The builder's disk for a toolchain (`1.100.0`), built once and kept in the cache: the base, the
/// recipe's packages, and the toolchain from rustup's official source under `/opt`.
pub fn image(r: &Recipe, toolchain: &str, work: &Path) -> Result<PathBuf, VmError> {
    let base = cache::base_image(r)?;
    let key = cache::sha256_text(&format!("{PROVISIONING}\n{}\n{}\n{toolchain}\n", r.base.sha512, r.builder.packages.join(" ")));
    let path = cache::dir()?.join(format!("builder-{toolchain}-{}.qcow2", &key[..16]));
    if path.exists() {
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

/// The script the builder machine runs on the commit's tree: the toolchain's and the C library's
/// versions written down, each component of `builds` built alone with `cargo build --release
/// --locked --offline -p <component> --bins` (so cargo unifies features within its own inputs, never
/// across components), as many jobs as the builder has processors, and every one of its
/// executables copied out with cargo's dep-info for it; a bin cargo did not build (a required
/// feature the defaults do not turn on) is named, never skipped, and fails the script, so the cut
/// refuses a release that would ship less than its components name.
pub fn script(r: &Recipe, builds: &[(String, Vec<String>)]) -> String {
    let cargo: String = builds
        .iter()
        .map(|(p, _)| format!("cargo build --release --locked --offline --jobs {} -p {p} --bins\n", r.builder.cpus))
        .collect();
    let copies: String = builds
        .iter()
        .flat_map(|(_, bins)| bins)
        .map(|b| format!("if [ -f target/release/{b} ]; then cp target/release/{b} /build/out/bin/; cp target/release/{b}.d /build/out/deps/; else echo \"ALFORGE-MISSING {b}\"; missing=1; fi\n"))
        .collect();
    format!(
        "set -eu\nexport RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\
         cd {SRC}\nmkdir -p /build/out/bin /build/out/deps\nrustc --version > /build/out/toolchain\nldd --version | head -1 > /build/out/libc\n\
         {cargo}missing=\n{copies}\
         [ -z \"$missing\" ]\necho ALFORGE-BUILD-OK\n"
    )
}

/// What a build's [`script`] log (brought back on the output disk, at `log_path`) and the builder's
/// console say: every bin cargo did not build refused by name; a build that did not finish (no
/// `ALFORGE-BUILD-OK` in its log, no `ALFORGE-BUILD-EXIT 0` on the console, or the output disk
/// unread) refused with the log's tail, or the console's when there is no log.
pub fn outcome(log: &str, console: &str, unpacked: bool, log_path: &Path) -> Result<(), VmError> {
    let missing: Vec<&str> = log.lines().filter_map(|l| l.strip_prefix("ALFORGE-MISSING ")).collect();
    if !missing.is_empty() {
        return Err(VmError::Failed {
            program: "the build in the builder machine".into(),
            status: "refused".into(),
            stderr: missing.iter().map(|b| format!("cargo built no `{b}` with the default features (a required feature they do not turn on): the release would not ship it")).collect::<Vec<_>>().join("\n"),
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

/// Builds each of `builds`' components' executables at `commit` of the repository at `root` in a fresh copy of the
/// builder: the commit's tree as git stores it, `cargo vendor` of its lock file, `cargo build --release
/// --locked --offline` inside, the executables named in `bins` handed back.
pub fn build(r: &Recipe, root: &Path, commit: &str, toolchain: &str, builds: &[(String, Vec<String>)], work: &Path) -> Result<Built, VmError> {
    let builder = image(r, toolchain, work)?;
    let input = work.join("in");
    let _ = std::fs::remove_dir_all(&input);
    std::fs::create_dir_all(input.join("src"))?;
    // The commit's tree as git stores it, no attribute or filter applied.
    crate::git::export_tree(root, commit, &[], &input.join("src")).map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?;
    eprintln!("alforge: vendoring the dependencies Cargo.lock pins");
    let config = vm::run("cargo", &["vendor", "--locked", "--versioned-dirs", "../vendor"], Some(&input.join("src")))?;
    std::fs::create_dir_all(input.join("src/.cargo"))?;
    std::fs::write(input.join("src/.cargo/config.toml"), config)?;
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
    let user_data = "#cloud-config\nruncmd:\n  - [ sh, -c, 'mkdir -p /build/out && tar -xf /dev/disk/by-id/virtio-alforge-in -C /build && sh /build/build.sh > /build/out/build.log 2>&1; rc=$?; tail -n 40 /build/out/build.log > /dev/ttyS0; tar -cf /dev/disk/by-id/virtio-alforge-out -C /build/out . && sync; echo \"ALFORGE-BUILD-EXIT $rc\" > /dev/ttyS0' ]\npower_state:\n  mode: poweroff\n  delay: now\n  timeout: 120\n";
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
        data: vec![
            DataDisk { file: in_tar.clone(), serial: "alforge-in".into(), writable: false },
            DataDisk { file: out_raw.clone(), serial: "alforge-out".into(), writable: true },
        ],
        forwards: vec![],
        snapshot: false,
        mac: None,
        qmp: None,
        console: console.clone(),
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
    let _ = std::fs::remove_dir_all(&input);
    Ok(Built { out, toolchain, libc })
}
