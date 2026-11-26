//! C-293: the tester machine's update command (`veredas-atualizar`, docs/questions.md Q437), its
//! shipped text run here by `python3` as the machine runs it, against a machine's folders of the
//! test's own (`VEREDAS_ROOT`) with a stand-in for `systemctl` first on its PATH: a newer release's
//! bundle, packed by the cut's own code, installed beside the old one, `current` moved, the PATH's
//! links following, the services restarted, the plant's files untouched; every refusal said and
//! leaving the machine as it was. The command run inside the machine itself is tests/maquina.rs's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use alforge::{bundle, cut, maquina};

const TARGET: &str = "x86_64-unknown-linux-gnu";

/// A dynamic executable compiled here, standing in for one a release ships.
fn exe(path: &Path, links: &[&str]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("main.c"), "int main(void) { return 0; }\n").unwrap();
    let out = Command::new("cc").arg("-o").arg(path).arg(src.path().join("main.c")).args(links).output().unwrap();
    assert!(out.status.success(), "cc: {}", String::from_utf8_lossy(&out.stderr));
}

/// A release's folder as the cut assembles it: its manifest, `bin/`, the plant and a README, its
/// sums written.
fn release_folder(parent: &Path, release: &str, target: &str, bins: &[&str]) -> PathBuf {
    release_folder_tested(parent, release, target, bins, cut::SUITE)
}

/// [`release_folder`], its manifest saying `tested` was run (a trial says it was not).
fn release_folder_tested(parent: &Path, release: &str, target: &str, bins: &[&str], tested: &str) -> PathBuf {
    let dir = parent.join(format!("veredas-{release}-{target}"));
    for b in bins {
        exe(&dir.join("bin").join(b), &[]);
    }
    std::fs::create_dir_all(dir.join("planta")).unwrap();
    std::fs::write(dir.join("planta/bench.toml"), format!("# the plant of {release}\n")).unwrap();
    std::fs::write(dir.join("README.md"), "Veredas\n").unwrap();
    let components = vec![("veredas-portal".to_string(), "2.0.0".to_string(), bins.iter().map(|b| b.to_string()).collect())];
    let text = cut::manifest_text(&semver::Version::parse(release).unwrap(), &"ab".repeat(20), target, "rustc 1.100.0", &components, tested);
    std::fs::write(dir.join("release.toml"), text).unwrap();
    bundle::write_sums(&dir).unwrap();
    dir
}

/// A tester machine's folders: release 1.0.0 installed (`veredas-portal` and `veredas-velho`),
/// `current` naming it, its executables linked onto the PATH beside a link of another's, the
/// plant in the home folder.
struct Machine {
    root: tempfile::TempDir,
    stub: tempfile::TempDir,
}

impl Machine {
    fn new() -> Machine {
        Machine::with("1.0.0")
    }

    /// [`Machine::new`] with `release` installed.
    fn with(release: &str) -> Machine {
        let root = tempfile::tempdir().unwrap();
        let opt = root.path().join("opt/veredas");
        release_folder(&opt, release, TARGET, &["veredas-portal", "veredas-velho"]);
        std::os::unix::fs::symlink(format!("veredas-{release}-{TARGET}"), opt.join("current")).unwrap();
        let bin = root.path().join("usr/local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for b in ["veredas-portal", "veredas-velho"] {
            std::os::unix::fs::symlink(format!("/opt/veredas/current/bin/{b}"), bin.join(b)).unwrap();
        }
        std::os::unix::fs::symlink("/usr/bin/true", bin.join("outro")).unwrap();
        let plant = root.path().join("home/veredas/planta");
        std::fs::create_dir_all(&plant).unwrap();
        std::fs::write(plant.join("bench.toml"), "# the tester's own edits\n").unwrap();
        let stub = tempfile::tempdir().unwrap();
        let m = Machine { root, stub };
        m.systemctl(0);
        m
    }

    /// The stand-in `systemctl`: logs a restart and exits `code`; asked to `show` the services, it
    /// says each active with one main process, the same at every look.
    fn systemctl(&self, code: u8) {
        self.systemctl_showing(code, &["ActiveState=active\nExecMainStartTimestampMonotonic=5\n\nActiveState=active\nExecMainStartTimestampMonotonic=6\n"]);
    }

    /// [`Machine::systemctl`], its `show` answering each of `looks` in turn (the last one again).
    fn systemctl_showing(&self, code: u8, looks: &[&str]) {
        use std::os::unix::fs::PermissionsExt;
        let d = self.stub.path();
        for (i, l) in looks.iter().enumerate() {
            std::fs::write(d.join(format!("look-{i}")), l).unwrap();
        }
        let last = looks.len() - 1;
        let p = d.join("systemctl");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nif [ \"$1\" = show ]; then n=$(cat '{d}/looked' 2>/dev/null || echo 0); [ $n -gt {last} ] && n={last}; cat '{d}/look-'$n; echo $((n + 1)) > '{d}/looked'; exit 0; fi\necho \"$*\" >> '{d}/calls'\nexit {code}\n",
                d = d.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_file(d.join("looked"));
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.stub.path().join("calls")).unwrap_or_default()
    }

    /// The command run on `args` as the machine runs it: its exit code, stdout and stderr.
    fn run(&self, args: &[&Path]) -> (Option<i32>, String, String) {
        let out = self.command(args, &[("VEREDAS_SETTLE", "0")]).output().unwrap();
        (out.status.code(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    }

    /// The command on `args` with `env` set (`VEREDAS_SETTLE` left to its default unless given).
    fn command(&self, args: &[&Path], env: &[(&str, &str)]) -> Command {
        let script = self.stub.path().join("veredas-atualizar");
        std::fs::write(&script, maquina::ATUALIZAR).unwrap();
        let mut c = Command::new("python3");
        c.arg(&script).args(args).env("VEREDAS_ROOT", self.root.path()).env("PATH", format!("{}:/usr/bin:/bin", self.stub.path().display())).envs(env.iter().copied());
        c
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    /// Every entry under the machine's root: a folder, a link with what it names, a file with its
    /// bytes' digest and mode.
    fn snapshot(&self) -> BTreeMap<String, String> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
            use std::os::unix::fs::PermissionsExt;
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let p = e.path();
                let rel = p.strip_prefix(base).unwrap().display().to_string();
                let t = e.file_type().unwrap();
                if t.is_symlink() {
                    out.insert(rel, format!("-> {}", std::fs::read_link(&p).unwrap().display()));
                } else if t.is_dir() {
                    out.insert(rel, "folder".into());
                    walk(base, &p, out);
                } else {
                    let mode = std::fs::metadata(&p).unwrap().permissions().mode();
                    out.insert(rel, format!("{} {mode:o}", sha256_file(&p)));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.root.path(), self.root.path(), &mut out);
        out
    }
}

/// A bundle of `release` packed by the cut's own code from a folder `edit` may change after its
/// sums are written: the archive's path.
fn bundle_of(work: &Path, release: &str, target: &str, bins: &[&str], edit: impl FnOnce(&Path)) -> PathBuf {
    let folder = release_folder(work, release, target, bins);
    edit(&folder);
    let name = folder.file_name().unwrap().to_str().unwrap().to_string();
    let out = work.join(format!("{name}.tar.gz"));
    bundle::pack(work, &name, "1795300000", &out).unwrap();
    out
}

/// A child's output once it ends, or a failure after `secs` (a regression must not hang the run).
fn within(mut child: std::process::Child, secs: u64) -> std::process::Output {
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(secs) {
            child.kill().unwrap();
            panic!("the command did not end within {secs} s");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}

fn sha256_file(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))
}

#[test]
fn a_newer_release_is_installed_beside_the_old_and_current_moved_to_it() {
    let m = Machine::new();
    let work = tempfile::tempdir().unwrap();
    let b = bundle_of(work.path(), "1.1.0-alpha.1", TARGET, &["veredas-portal", "veredas-novo"], |_| {});
    let before = m.snapshot();
    let (code, out, err) = m.run(&[&b]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(err, "");
    let opt = m.path("opt/veredas");
    assert_eq!(
        out,
        format!(
            "veredas-atualizar: {} (SHA-256 {})\nveredas-atualizar: its SHA256SUMS {} (what the boot's check says as VEREDAS-BUNDLE)\n\
             veredas-atualizar: release 1.0.0 -> 1.1.0-alpha.1 (commit {})\n\
             veredas-atualizar: installed at {}; the old release kept at {}\n\
             veredas-atualizar: the plant's files in the home folder left as they are; the release's own plant is in /opt/veredas/current/planta\n\
             veredas-atualizar: restarting the bench and supervisão on it, and watching them for 0 s\n\
             veredas-atualizar: the bench and supervisão restarted on it and stayed up\n",
            b.display(),
            sha256_file(&b),
            sha256_file(&m.path(&format!("opt/veredas/veredas-1.1.0-alpha.1-{TARGET}/SHA256SUMS"))),
            "ab".repeat(20),
            opt.join(format!("veredas-1.1.0-alpha.1-{TARGET}")).display(),
            opt.join(format!("veredas-1.0.0-{TARGET}")).display(),
        )
    );
    // `current` names the new release, beside the old one, which is as it was.
    assert_eq!(std::fs::read_link(opt.join("current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-alpha.1-{TARGET}")));
    let after = m.snapshot();
    let old = format!("opt/veredas/veredas-1.0.0-{TARGET}");
    let kept = |s: &BTreeMap<String, String>| -> BTreeMap<String, String> { s.iter().filter(|(k, _)| k.starts_with(&old) || k.starts_with("home/")).map(|(k, v)| (k.clone(), v.clone())).collect() };
    assert_eq!(kept(&after), kept(&before), "the old release and the plant's files untouched");
    // The new release's files are the bundle's, every one matching its sums.
    let new = opt.join(format!("veredas-1.1.0-alpha.1-{TARGET}"));
    assert_eq!(bundle::verify_sums(&new).unwrap(), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(new.join("planta/bench.toml")).unwrap(), "# the plant of 1.1.0-alpha.1\n");
    // The PATH: a link for each of its executables, none for the one it no longer has, another's
    // link kept.
    let link = |b: &str| std::fs::read_link(m.path("usr/local/bin").join(b)).ok().map(|p| p.display().to_string());
    assert_eq!(link("veredas-portal").as_deref(), Some("/opt/veredas/current/bin/veredas-portal"));
    assert_eq!(link("veredas-novo").as_deref(), Some("/opt/veredas/current/bin/veredas-novo"));
    assert_eq!(link("veredas-velho"), None);
    assert_eq!(link("outro").as_deref(), Some("/usr/bin/true"));
    // The services restarted, nothing left of the staging.
    assert_eq!(m.calls(), "restart veredas-bancada.service veredas-supervisao.service\n");
    let left: Vec<String> = std::fs::read_dir(&opt).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.starts_with(".atualizar-")).collect();
    assert_eq!(left, Vec::<String>::new());
    // Installed again: refused, already there and not newer.
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(code, Some(2));
    assert_eq!(
        err,
        format!(
            "veredas-atualizar: not installed, the machine is as it was:\n  release 1.1.0-alpha.1 is not newer than the installed 1.1.0-alpha.1\n  {} is there already\n",
            new.display()
        )
    );
}

#[test]
fn every_bundle_the_command_refuses_leaves_the_machine_as_it_was() {
    let refused = |b: &Path, m: &Machine| -> Vec<String> {
        let before = m.snapshot();
        let (code, out, err) = m.run(&[b]);
        assert_eq!(code, Some(2), "{out}{err}");
        assert_eq!(m.snapshot(), before, "the machine as it was");
        assert_eq!(m.calls(), "", "no service restarted");
        let lines: Vec<String> = err.lines().map(String::from).collect();
        if lines.first().map(String::as_str) == Some("veredas-atualizar: not installed, the machine is as it was:") {
            lines[1..].iter().map(|l| l.trim_start().to_string()).collect()
        } else {
            lines
        }
    };
    let m = Machine::new();
    let work = |_: ()| tempfile::tempdir().unwrap();
    let w = work(());
    let ok = ["veredas-portal"];
    // Off its sums: a file changed after them, one added, one taken out.
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        std::fs::write(d.join("README.md"), "changed\n").unwrap();
        std::fs::write(d.join("extra.txt"), "added\n").unwrap();
        std::fs::remove_file(d.join("planta/bench.toml")).unwrap();
    });
    assert_eq!(refused(&b, &m), ["README.md does not match its sum", "extra.txt is not in SHA256SUMS", "planta/bench.toml is in SHA256SUMS and not in the bundle"]);
    // Not newer: an older release, and the installed release's own pre-release, which ranks below it.
    let w = work(());
    assert_eq!(refused(&bundle_of(w.path(), "0.9.0", TARGET, &ok, |_| {}), &m), ["release 0.9.0 is not newer than the installed 1.0.0"]);
    let w = work(());
    assert_eq!(refused(&bundle_of(w.path(), "1.0.0-rc.1", TARGET, &ok, |_| {}), &m), ["release 1.0.0-rc.1 is not newer than the installed 1.0.0"]);
    // Another target.
    let w = work(());
    assert_eq!(
        refused(&bundle_of(w.path(), "1.1.0", "aarch64-unknown-linux-gnu", &ok, |_| {}), &m),
        ["the bundle is built for aarch64-unknown-linux-gnu, this machine runs x86_64-unknown-linux-gnu"]
    );
    // A folder named after another release than its manifest's.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap().replace("release = \"1.1.0\"", "release = \"1.2.0\"");
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), [format!("the bundle's folder is veredas-1.1.0-{TARGET}, not veredas-1.2.0-{TARGET}")]);
    // Executables the manifest names and the bundle lacks, and one no component ships.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal", "veredas-x"], |d| {
        std::fs::remove_file(d.join("bin/veredas-x")).unwrap();
        exe(&d.join("bin/veredas-y"), &[]);
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), ["bin/veredas-x is named by the manifest and not in the bundle", "bin/veredas-y is in the bundle and no component ships it"]);
    // A manifest of another format.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap().replace("format = \"veredas-release\"", "format = \"veredas-bench\"");
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), ["release.toml is `veredas-bench` version 1, not `veredas-release` version 1"]);
    // An executable that would not start here: a library it was linked against is not on the machine.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("f.c"), "int fantasma(void) { return 0; }\n").unwrap();
        let out = Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(lib.path().join("libfantasma.so")).arg(lib.path().join("f.c")).output().unwrap();
        assert!(out.status.success());
        exe(&d.join("bin/veredas-portal"), &["-Wl,--no-as-needed", &format!("-L{}", lib.path().display()), "-lfantasma"]);
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), ["bin/veredas-portal would not start here: libfantasma.so => not found"]);
    // A link in the archive, and two folders at its top.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| std::os::unix::fs::symlink("/etc/passwd", d.join("README.link")).unwrap());
    assert_eq!(refused(&b, &m), [format!("veredas-1.1.0-{TARGET}/README.link: neither a file nor a folder")]);
    let w = work(());
    let two = w.path().join("two");
    release_folder(&two, "1.1.0", TARGET, &ok);
    std::fs::create_dir_all(two.join("another")).unwrap();
    let b = w.path().join("two.tar.gz");
    alforge::vm::run("tar", &["-czf", &b.display().to_string(), "-C", &two.display().to_string(), &format!("veredas-1.1.0-{TARGET}"), "another"], None).unwrap();
    assert_eq!(refused(&b, &m), ["the bundle holds 2 folders at its top, not one"]);
    // A trial's bundle: never installed, its suite not run.
    let w = work(());
    let trial = release_folder_tested(w.path(), "1.1.0", TARGET, &ok, cut::NOT_RUN);
    let b = w.path().join("trial.tar.gz");
    bundle::pack(w.path(), trial.file_name().unwrap().to_str().unwrap(), "1795300000", &b).unwrap();
    assert_eq!(refused(&b, &m), ["the bundle is a trial cut's (its suite `not run: a trial cut`): only a release cut for real is installed"]);
    // A manifest missing what every manifest states, and an executable two components name.
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap();
        let t: String = t.lines().filter(|l| !l.starts_with("commit = ") && !l.starts_with("toolchain = ")).map(|l| format!("{l}\n")).collect();
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), ["release.toml states no `commit`", "release.toml states no `toolchain`"]);
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap() + "\n[[component]]\nname = \"veredas-maquinar\"\nversion = \"2.0.0\"\nbins = [\"veredas-portal\"]\n";
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(refused(&b, &m), ["bin/veredas-portal is named by more than one component"]);
    // Not the room on the disk (the room it keeps free asked larger than any disk).
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &ok, |_| {});
    let before = m.snapshot();
    let script = m.stub.path().join("veredas-atualizar");
    let out = Command::new("python3")
        .arg(&script)
        .arg(&b)
        .env("VEREDAS_ROOT", m.root.path())
        .env("VEREDAS_RESERVE_MB", "1000000000")
        .env("PATH", format!("{}:/usr/bin:/bin", m.stub.path().display()))
        .output()
        .unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("  the bundle's archive needs ") && err.contains(" MB left free; ") && err.ends_with(" MB are free\n"), "{err}");
    assert_eq!(m.snapshot(), before);
    // An I/O error once the release is moved and `current` pointed at it (the PATH's folder not
    // writable): all of it put back.
    use std::os::unix::fs::PermissionsExt;
    let bin = m.path("usr/local/bin");
    let w = work(());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal", "veredas-novo"], |_| {});
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();
    let before = m.snapshot();
    let (code, out, err) = m.run(&[&b]);
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, Some(2), "{out}{err}");
    // The link the run makes first under a scratch name of its own, refused by the folder's mode.
    let start = format!("veredas-atualizar: not installed, the machine is as it was:\n  {}/.veredas-novo.atualizar-", bin.display());
    assert!(err.starts_with(&start) && err.ends_with(": Permission denied\n"), "{err}");
    assert_eq!(m.snapshot(), before, "current, the links and the folder put back");
    assert_eq!(m.calls(), "");
    // No archive at all, and no file.
    let w = work(());
    let b = w.path().join("notes.tar.gz");
    std::fs::write(&b, "not an archive\n").unwrap();
    let got = refused(&b, &m);
    assert!(got.len() == 1 && got[0].starts_with("the bundle is no release bundle (a gzipped tar): "), "{got:?}");
    let absent = w.path().join("absent.tar.gz");
    assert_eq!(refused(&absent, &m), [format!("veredas-atualizar: {}: No such file or directory", absent.display())]);
}

#[test]
fn the_command_says_how_it_was_run_wrong_and_what_it_could_not_do() {
    let m = Machine::new();
    let (code, _, err) = m.run(&[]);
    assert_eq!((code, err.as_str()), (Some(1), "usage: sudo veredas-atualizar <bundle.tar.gz>\n"));
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    // Without the rights to write the release's folder: run it with sudo.
    use std::os::unix::fs::PermissionsExt;
    let opt = m.path("opt/veredas");
    std::fs::set_permissions(&opt, std::fs::Permissions::from_mode(0o555)).unwrap();
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&b]);
    std::fs::set_permissions(&opt, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!((code, err), (Some(2), format!("veredas-atualizar: {} is not writable here: run it with sudo\n", opt.display())));
    assert_eq!(m.snapshot(), before);
    // Another update running: refused at once, nothing left behind.
    // `flock --close`: the lock held by flock alone, never by the sleep it runs.
    let before = m.snapshot();
    let mut holder = Command::new("flock")
        .arg("--close")
        .arg(&opt)
        .args(["sleep", "30"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (code, _, err) = m.run(&[&b]);
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert_eq!((code, err.as_str()), (Some(2), "veredas-atualizar: another update is running\n"));
    assert_eq!(m.snapshot(), before);
    assert_eq!(std::fs::read_link(opt.join("current")).unwrap(), PathBuf::from(format!("veredas-1.0.0-{TARGET}")));
    // The services that do not restart: installed, said, and the way back named.
    let back = format!("sudo ln -sfn veredas-1.0.0-{TARGET} /opt/veredas/current && sudo systemctl restart veredas-bancada.service veredas-supervisao.service");
    m.systemctl(1);
    let (code, out, err) = m.run(&[&b]);
    assert_eq!(code, Some(3), "{out}{err}");
    assert_eq!(m.calls(), "restart veredas-bancada.service veredas-supervisao.service\n");
    assert_eq!(std::fs::read_link(opt.join("current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));
    assert_eq!(err, format!("veredas-atualizar: the bench and supervisão did not stay up on it (systemctl exited 1); to go back: {back}\n"));
    // Restarted, but one does not stay up (the new release refuses the kept plant, exits, and
    // systemd starts it again): exit 3, the way back named.
    for looks in [
        vec!["ActiveState=active\nExecMainStartTimestampMonotonic=5\n\nActiveState=activating\nExecMainStartTimestampMonotonic=6\n"],
        vec![
            "ActiveState=active\nExecMainStartTimestampMonotonic=5\n\nActiveState=active\nExecMainStartTimestampMonotonic=6\n",
            "ActiveState=active\nExecMainStartTimestampMonotonic=5\n\nActiveState=active\nExecMainStartTimestampMonotonic=9\n",
        ],
    ] {
        let m = Machine::new();
        m.systemctl_showing(0, &looks);
        let (code, out, err) = m.run(&[&b]);
        assert_eq!(code, Some(3), "{out}{err}");
        assert!(err.starts_with("veredas-atualizar: the bench and supervisão did not stay up on it (they did not stay active: ") && err.ends_with(&format!("); to go back: {back}\n")), "{err}");
    }
}

/// A plant served as it should be, as the host reads it.
fn served(page: u16) -> alforge::together::Report {
    let snapshot = |place: f64| {
        let objects: Vec<serde_json::Value> = ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"]
            .iter()
            .map(|id| serde_json::json!({ "id": id, "value": 1.0, "stale": false, "place": place }))
            .collect();
        serde_json::json!({ "objects": objects, "session": { "health": { "plc": { "state": "live" }, "sorter": { "state": "live" } } } })
    };
    alforge::together::Report {
        page: alforge::together::Answer { status: page, body: "<html></html>".into() },
        first: snapshot(0.1),
        second: snapshot(0.2),
        apart: std::time::Duration::from_secs(2),
        write: alforge::together::Answer { status: 403, body: r#"{"error":"writes are turned off for this project"}"#.into() },
        bench_exit: None,
        supervisao_exit: None,
        bench_out: String::new(),
        supervisao_out: String::new(),
    }
}

#[test]
fn the_update_inside_the_machine_is_judged_by_what_its_console_and_the_host_say() {
    let old = format!("veredas-1.0.0-{TARGET}");
    let new = format!("veredas-1.1.0-{TARGET}");
    let sum = "c".repeat(64);
    // The console as the proof's commands leave it (seen in the machine, 2026-11-25): each command's
    // echo, its marker split by quotes, then what it printed, its first line after bash's
    // bracketed-paste `ESC[?2004l`.
    let good = format!(
        "veredas login: veredas\nPassword: \nveredas@veredas:~$ echo \"ALFORGE-\"\"UPDATE-EXIT $?\"\n\
         \u{1b}[?2004lALFORGE-PLANT-BEFORE {sum}\nveredas-atualizar: release 1.0.0 -> 1.1.0 (commit ab)\nALFORGE-UPDATE-EXIT 0\n\
         \u{1b}[?2004lALFORGE-CURRENT {new}\nALFORGE-RELEASES current {old} {new} \n\
         \u{1b}[?2004lALFORGE-EXE /opt/veredas/{new}/bin/veredas-maquinar\nALFORGE-EXE /opt/veredas/{new}/bin/veredas-supervisao\n\
         \u{1b}[?2004lALFORGE-CHECK VEREDAS-SUMS-OK 1.1.0\nALFORGE-CHECK VEREDAS-ON-PATH ok\nALFORGE-CHECK VEREDAS-ATUALIZAR ok\n\
         ALFORGE-PLANT-AFTER {sum}\n\u{1b}[?2004lALFORGE-RESTARTED 812 1204 active\nveredas-atualizar: not installed, the machine is as it was:\nALFORGE-AGAIN-EXIT 2\n"
    );
    let reboot = format!("[   9.1] VEREDAS-SUMS-OK 1.1.0\nVEREDAS-BUNDLE {}\nVEREDAS-ON-PATH ok\nVEREDAS-ATUALIZAR ok\n", "e".repeat(64));
    let full = |console: &str, page: u16, again: &str, again_page: u16, after: &str| maquina::UpdateProof {
        console: console.into(),
        plant: served(page),
        reboot_console: again.into(),
        reboot_plant: served(again_page),
        before: "a".into(),
        after: after.into(),
    };
    let proof = |console: &str, page: u16, after: &str| full(console, page, &reboot, 200, after);
    let faults = |console: &str| maquina::update_faults(&proof(console, 200, "a"), "1.0.0", "1.1.0", TARGET);
    assert_eq!(faults(&good), Vec::<String>::new());
    // The boot again on the new release, a line of its check cut by a kernel record (journald's way
    // to the console lets one print inside it): read put back, no fault.
    let cut = reboot.replacen("VEREDAS-ON-PATH ok\n", "VEREDAS-ON-[    9.200000] audit: type=1400 apparmor=\"STATUS\"\nPATH ok\n", 1);
    assert!(!cut.contains("VEREDAS-ON-PATH ok"));
    assert_eq!(maquina::update_faults(&full(&good, 200, &cut, 200, "a"), "1.0.0", "1.1.0", TARGET), Vec::<String>::new());
    for (from, to, want) in [
        ("ALFORGE-UPDATE-EXIT 0", "ALFORGE-UPDATE-EXIT 2", vec!["the update command did not install the bundle (it exited 2)".to_string()]),
        (&format!("ALFORGE-CURRENT {new}") as &str, &format!("ALFORGE-CURRENT {old}") as &str, vec![format!("`current` names {old}, not {new}")]),
        (&format!("current {old} {new}"), &format!("current {new}"), vec![format!("/opt/veredas holds no {old}")]),
        (
            &format!("/opt/veredas/{new}/bin/veredas-supervisao"),
            &format!("/opt/veredas/{old}/bin/veredas-supervisao"),
            vec![
                format!("no running veredas-supervisao is the new release's (/opt/veredas/{new}/bin/veredas-supervisao): [\"/opt/veredas/{new}/bin/veredas-maquinar\", \"/opt/veredas/{old}/bin/veredas-supervisao\"]"),
                format!("an executable of another release still runs: [\"/opt/veredas/{new}/bin/veredas-maquinar\", \"/opt/veredas/{old}/bin/veredas-supervisao\"]"),
            ],
        ),
        ("VEREDAS-SUMS-OK 1.1.0", "VEREDAS-SUMS-FAILED 1.1.0", vec!["the boot's check on the new release does not say `VEREDAS-SUMS-OK 1.1.0`".to_string()]),
        ("ALFORGE-CHECK VEREDAS-ATUALIZAR ok", "ALFORGE-CHECK VEREDAS-ATUALIZAR missing", vec!["the boot's check on the new release does not say `VEREDAS-ATUALIZAR ok`".to_string()]),
        ("ALFORGE-CHECK VEREDAS-ON-PATH ok", "ALFORGE-CHECK VEREDAS-ON-PATH missing veredas-novo", vec!["the boot's check on the new release does not say `VEREDAS-ON-PATH ok`".to_string()]),
        (&format!("ALFORGE-PLANT-AFTER {sum}"), &format!("ALFORGE-PLANT-AFTER {}", "d".repeat(64)), vec![format!("the plant's files changed or were not read ({sum} before, {} after)", "d".repeat(64))]),
        ("ALFORGE-AGAIN-EXIT 2", "ALFORGE-AGAIN-EXIT 0", vec!["the same bundle was not refused a second time (it exited 0)".to_string()]),
        ("ALFORGE-RESTARTED 812 1204 active", "ALFORGE-RESTARTED 812 812 active", vec!["the bench, killed, was not started again by systemd (Some([\"812\", \"812\", \"active\"]))".to_string()]),
        ("ALFORGE-RESTARTED 812 1204 active", "ALFORGE-RESTARTED 812 0 inactive", vec!["the bench, killed, was not started again by systemd (Some([\"812\", \"0\", \"inactive\"]))".to_string()]),
        ("ALFORGE-RESTARTED 812 1204 active", "ALFORGE-RESTARTED 812 1204 activating", vec!["the bench, killed, was not started again by systemd (Some([\"812\", \"1204\", \"activating\"]))".to_string()]),
        (&format!("current {old} {new}"), &format!("current {old}"), vec![format!("/opt/veredas holds no {new}")]),
        (
            &format!("/opt/veredas/{new}/bin/veredas-maquinar"),
            &format!("/opt/veredas/{old}/bin/veredas-maquinar"),
            vec![
                format!("no running veredas-maquinar is the new release's (/opt/veredas/{new}/bin/veredas-maquinar): [\"/opt/veredas/{old}/bin/veredas-maquinar\", \"/opt/veredas/{new}/bin/veredas-supervisao\"]"),
                format!("an executable of another release still runs: [\"/opt/veredas/{old}/bin/veredas-maquinar\", \"/opt/veredas/{new}/bin/veredas-supervisao\"]"),
            ],
        ),
        ("ALFORGE-UPDATE-EXIT 0\n", "", vec!["the update command did not install the bundle (it exited with nothing said)".to_string()]),
    ] {
        assert_eq!(faults(&good.replace(from, to)), want, "{from}");
    }
    // The plant from the host, and the shipped disk, after the update's boot.
    assert_eq!(maquina::update_faults(&proof(&good, 500, "a"), "1.0.0", "1.1.0", TARGET), alforge::together::plant_faults(&served(500)));
    assert!(!alforge::together::plant_faults(&served(500)).is_empty());
    assert_eq!(maquina::update_faults(&proof(&good, 200, "b"), "1.0.0", "1.1.0", TARGET), ["the update's boots changed the shipped disk"]);
    // Booted again after the update: the boot's own check on the new release, and the plant.
    assert_eq!(
        maquina::update_faults(&full(&good, 200, &reboot.replace("VEREDAS-SUMS-OK 1.1.0", "VEREDAS-SUMS-OK 1.0.0"), 200, "a"), "1.0.0", "1.1.0", TARGET),
        ["booted again, the machine's check does not say `VEREDAS-SUMS-OK 1.1.0`"]
    );
    for line in ["VEREDAS-ON-PATH ok", "VEREDAS-ATUALIZAR ok"] {
        assert_eq!(
            maquina::update_faults(&full(&good, 200, &reboot.replace(line, "VEREDAS-OTHER"), 200, "a"), "1.0.0", "1.1.0", TARGET),
            [format!("booted again, the machine's check does not say `{line}`")]
        );
    }
    let got = maquina::update_faults(&full(&good, 200, &reboot, 500, "a"), "1.0.0", "1.1.0", TARGET);
    assert_eq!(got, alforge::together::plant_faults(&served(500)).into_iter().map(|f| format!("booted again: {f}")).collect::<Vec<_>>());
}

#[test]
fn the_command_holds_its_lock_before_it_reads_anything_and_reads_the_bundle_once() {
    // An update reading its bundle from a slow source (a FIFO no one writes yet) holds the lock:
    // another started meanwhile is refused at once, and the first then installs what it read.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let c = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |_| {});
    let fifo = w.path().join("slow.tar.gz");
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let slow = m.command(&[&fifo], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let outside_staging = |s: BTreeMap<String, String>| -> BTreeMap<String, String> { s.into_iter().filter(|(k, _)| !k.contains(".atualizar-")).collect() };
    let before = outside_staging(m.snapshot());
    let (code, _, err) = m.run(&[&c]);
    assert_eq!((code, err.as_str()), (Some(2), "veredas-atualizar: another update is running\n"));
    assert_eq!(outside_staging(m.snapshot()), before, "the refused update changed nothing (the staging is the first one's)");
    std::fs::write(&fifo, std::fs::read(&b).unwrap()).unwrap();
    let out = within(slow, 120);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("(SHA-256 {})", sha256_file(&b))), "the digest of the bytes it read");
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));
}

#[test]
fn a_damaged_or_odd_bundle_and_a_path_not_the_releases_are_refused_with_the_machine_as_it_was() {
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let bytes = std::fs::read(&b).unwrap();
    let before = m.snapshot();
    // A byte changed anywhere in the compressed stream: refused (exit 2) every time, never a
    // traceback, the machine as it was.
    let damaged = w.path().join("damaged.tar.gz");
    let step = ((bytes.len() - 30) / 40).max(1);
    for at in (20..bytes.len() - 10).step_by(step) {
        let mut d = bytes.clone();
        d[at] ^= 0x5a;
        std::fs::write(&damaged, &d).unwrap();
        let (code, out, err) = m.run(&[&damaged]);
        assert_eq!(code, Some(2), "a byte changed at {at}: {out}{err}");
        assert!(!err.contains("Traceback"), "{err}");
    }
    assert_eq!(m.snapshot(), before);
    // Entries the release never makes: a path out of its folder, a FIFO, a hard link.
    let odd = w.path().join("odd.tar.gz");
    let top = format!("veredas-1.1.0-{TARGET}");
    let py = "import io, sys, tarfile\nout, top = sys.argv[1], sys.argv[2]\nwith tarfile.open(out, 'w:gz') as t:\n    \
              for name, kind in [(top + '/../evil', tarfile.REGTYPE), (top + '/fifo', tarfile.FIFOTYPE), (top + '/hard', tarfile.LNKTYPE)]:\n        \
              i = tarfile.TarInfo(name); i.type = kind; i.linkname = top + '/README.md' if kind == tarfile.LNKTYPE else ''\n        \
              t.addfile(i, io.BytesIO(b'') if kind == tarfile.REGTYPE else None)\n";
    assert!(Command::new("python3").args(["-c", py]).arg(&odd).arg(&top).status().unwrap().success());
    let (code, _, err) = m.run(&[&odd]);
    assert_eq!(code, Some(2));
    assert_eq!(
        err,
        format!("veredas-atualizar: not installed, the machine is as it was:\n  {top}/../evil: a path outside the bundle's folder\n  {top}/fifo: neither a file nor a folder\n  {top}/hard: neither a file nor a folder\n")
    );
    // A file on the PATH that is no link a release made, where an executable would go.
    std::fs::write(m.path("usr/local/bin/veredas-novo"), "#!/bin/sh\n").unwrap();
    let n = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal", "veredas-novo"], |_| {});
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&n]);
    assert_eq!(code, Some(2));
    assert_eq!(
        err,
        format!(
            "veredas-atualizar: not installed, the machine is as it was:\n  {} is not a link a release made: an executable of the release would replace it\n",
            m.path("usr/local/bin/veredas-novo").display()
        )
    );
    assert_eq!(m.snapshot(), before);
    // A manifest without `[tested]`: refused by name, never a traceback.
    let t = bundle_of(w.path(), "1.3.0", TARGET, &["veredas-portal"], |d| {
        let text = std::fs::read_to_string(d.join("release.toml")).unwrap();
        std::fs::write(d.join("release.toml"), &text[..text.find("[tested]").unwrap()]).unwrap();
        bundle::write_sums(d).unwrap();
    });
    let (code, _, err) = m.run(&[&t]);
    assert_eq!((code, err), (Some(2), "veredas-atualizar: not installed, the machine is as it was:\n  release.toml states no `tested`\n".to_string()));
}

#[test]
fn newer_is_semvers_precedence_numbers_compared_as_numbers() {
    for (installed, offered, newer) in [("1.0.0-alpha.9", "1.0.0-alpha.10", true), ("1.9.0", "1.10.0", true), ("1.10.0", "1.9.0", false), ("1.0.0-alpha.10", "1.0.0-alpha.9", false)] {
        let m = Machine::with(installed);
        let w = tempfile::tempdir().unwrap();
        let b = bundle_of(w.path(), offered, TARGET, &["veredas-portal"], |_| {});
        let (code, out, err) = m.run(&[&b]);
        assert_eq!(code, Some(if newer { 0 } else { 2 }), "{installed} then {offered}: {out}{err}");
        if !newer {
            assert!(err.contains(&format!("release {offered} is not newer than the installed {installed}")), "{err}");
        }
    }
}

#[test]
fn an_io_error_after_links_changed_puts_them_back_and_the_room_counts_the_bundle() {
    // The links change in name order: `veredas-a` is made and `veredas-portal` pointed again, then
    // the last one's link cannot be written (its name, with the run's scratch prefix, is longer
    // than a name may be): `current`, the links and the folder all put back.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let long = format!("veredas-z{}", "z".repeat(235));
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-a", "veredas-portal", &long], |_| {});
    let before = m.snapshot();
    let (code, out, err) = m.run(&[&b]);
    assert_eq!(code, Some(2), "{out}{err}");
    assert!(err.starts_with("veredas-atualizar: not installed, the machine is as it was:\n  "), "{err}");
    assert_eq!(m.snapshot(), before, "current, veredas-a's link and the release's folder put back");
    assert_eq!(m.calls(), "");
    // The room a bundle needs: its files' size beside the reserve. A sparse file of 1 GB packs
    // small; with the reserve 512 MB under the free room (wide enough for a shared disk's
    // changes meanwhile), its size is what does not fit.
    let m = Machine::new();
    let w2 = tempfile::tempdir().unwrap();
    let big = bundle_of(w2.path(), "1.1.0", TARGET, &["veredas-portal"], |d| {
        std::fs::File::create(d.join("planta/big.bin")).unwrap().set_len(1 << 30).unwrap();
        bundle::write_sums(d).unwrap();
    });
    let avail = String::from_utf8(Command::new("df").args(["--output=avail", "-B1"]).arg(m.path("opt")).output().unwrap().stdout).unwrap();
    let avail: u64 = avail.lines().nth(1).unwrap().trim().parse().unwrap();
    let reserve = ((avail >> 20) - 512).to_string();
    let before = m.snapshot();
    let out = m.command(&[&big], &[("VEREDAS_SETTLE", "0"), ("VEREDAS_RESERVE_MB", &reserve)]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("  the bundle needs 1024 MB under "), "{err}");
    assert_eq!(m.snapshot(), before);
}

#[test]
fn the_services_are_watched_long_enough_to_see_one_restarting() {
    // A service that keeps exiting and coming back: systemd names a new main process each second.
    // With the command's own watch (no VEREDAS_SETTLE), it is seen: exit 3, the way back named.
    use std::os::unix::fs::PermissionsExt;
    let m = Machine::new();
    let stub = m.stub.path().join("systemctl");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nif [ \"$1\" = show ]; then t=$(date +%s); printf 'ActiveState=active\\nExecMainStartTimestampMonotonic=1\\n\\nActiveState=active\\nExecMainStartTimestampMonotonic=%s\\n' \"$t\"; exit 0; fi\necho \"$*\" >> '{}'\n",
            m.stub.path().join("calls").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let start = std::time::Instant::now();
    let out = m.command(&[&b], &[]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(3), "{err}");
    assert!(start.elapsed() >= std::time::Duration::from_secs(9), "watched twice five seconds: {:?}", start.elapsed());
    assert!(err.contains("they did not stay active"), "{err}");
    // systemd that cannot say the services' state: not taken as up.
    let m = Machine::new();
    std::fs::write(m.stub.path().join("systemctl"), "#!/bin/sh\n[ \"$1\" = show ] && exit 1\nexit 0\n").unwrap();
    std::fs::set_permissions(m.stub.path().join("systemctl"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(code, Some(3));
    assert!(err.contains("(systemctl could not say their state)"), "{err}");
}

#[test]
fn round_two_the_lock_holds_the_installed_release_read_and_the_manifest_is_held_to_its_shape() {
    use std::os::unix::fs::PermissionsExt;
    // The installed release's manifest read slowly (a FIFO): read under the lock, so a second
    // update meanwhile is refused at once.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let installed = m.path(&format!("opt/veredas/veredas-1.0.0-{TARGET}/release.toml"));
    let text = std::fs::read(&installed).unwrap();
    std::fs::remove_file(&installed).unwrap();
    assert!(Command::new("mkfifo").arg(&installed).status().unwrap().success());
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let c = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |_| {});
    let slow = m.command(&[&b], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The second bounded too: one that waits instead of refusing fails the test, never hangs it.
    let second = m.command(&[&c], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let out2 = within(second, 30);
    assert_eq!((out2.status.code(), String::from_utf8_lossy(&out2.stderr).into_owned()), (Some(2), "veredas-atualizar: another update is running\n".to_string()));
    std::fs::write(&installed, &text).unwrap();
    let out = within(slow, 120);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));

    // A bundle larger than one read, its gzip CRC changed (the archive whole): refused.
    let m = Machine::new();
    let big = bundle_of(w.path(), "1.3.0", TARGET, &["veredas-portal"], |d| {
        // Bytes no compressor shrinks (xorshift), so the archive is larger than one read.
        let mut x: u64 = 0x9e3779b97f4a7c15;
        let noise: Vec<u8> = (0..(3u32 << 20))
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        std::fs::write(d.join("planta/noise.bin"), noise).unwrap();
        bundle::write_sums(d).unwrap();
    });
    let mut bytes = std::fs::read(&big).unwrap();
    assert!(bytes.len() > 2 << 20, "larger than one read");
    let crc = bytes.len() - 8;
    bytes[crc] ^= 0xff;
    let bad = w.path().join("bad-crc.tar.gz");
    std::fs::write(&bad, &bytes).unwrap();
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&bad]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("  the bundle is no release bundle (a gzipped tar): "), "{err}");
    assert_eq!(m.snapshot(), before);

    // What a manifest's or the sums' damage makes: each refused by name, never a traceback.
    let refused = |b: &Path| -> String {
        let (code, out, err) = m.run(&[b]);
        assert_eq!(code, Some(2), "{out}{err}");
        assert!(!err.contains("Traceback"), "{err}");
        err
    };
    let sums = bundle_of(w.path(), "1.4.0", TARGET, &["veredas-portal"], |d| std::fs::write(d.join("SHA256SUMS"), b"\x81\x82 not utf-8\n").unwrap());
    assert!(refused(&sums).contains("  the bundle holds no readable SHA256SUMS: "));
    type Shape = (&'static str, Box<dyn Fn(String) -> String>, &'static str);
    let shapes: Vec<Shape> = vec![
        ("no suite", Box::new(|t: String| t.lines().filter(|l| !l.starts_with("suite = ")).map(|l| format!("{l}\n")).collect()), "  release.toml's [tested] does not state its `suite` and `together`\n"),
        ("an empty suite", Box::new(|t: String| t.lines().map(|l| if l.starts_with("suite = ") { "suite = \"\"\n".to_string() } else { format!("{l}\n") }).collect()), "  release.toml's [tested] does not state its `suite` and `together`\n"),
        ("a component no table", Box::new(|t: String| t.replace("[[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\n", "component = \"x\"\n[x]\n")), "  release.toml states no `[[component]]`\n"),
        ("a component's version no string", Box::new(|t: String| t.replace("\nversion = \"2.0.0\"\n", "\nversion = 2\n")), "  release.toml's component `veredas-portal` is not a name, a SemVer version and its executables\n"),
    ];
    for (i, (what, edit, want)) in shapes.into_iter().enumerate() {
        let b = bundle_of(w.path(), &format!("1.5.{i}"), TARGET, &["veredas-portal"], |d| {
            let t = std::fs::read_to_string(d.join("release.toml")).unwrap();
            std::fs::write(d.join("release.toml"), edit(t)).unwrap();
            bundle::write_sums(d).unwrap();
        });
        assert!(refused(&b).ends_with(want), "{what}");
    }
    assert_eq!(m.snapshot(), before);

    // Another's link where an executable would go: refused, as a file is.
    std::os::unix::fs::symlink("/usr/bin/true", m.path("usr/local/bin/veredas-novo")).unwrap();
    let n = bundle_of(w.path(), "1.6.0", TARGET, &["veredas-portal", "veredas-novo"], |_| {});
    assert!(refused(&n).contains(&format!("{} is not a link a release made", m.path("usr/local/bin/veredas-novo").display())));
    std::fs::remove_file(m.path("usr/local/bin/veredas-novo")).unwrap();

    // Installed: a tester's file at the name the old command used for its scratch link and a
    // tester's alias through `current` are kept.
    std::fs::write(m.path("usr/local/bin/veredas-novo.new"), "a tester's notes\n").unwrap();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-velho", m.path("usr/local/bin/vv")).unwrap();
    std::fs::set_permissions(m.path("usr/local/bin/veredas-novo.new"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let (code, out, err) = m.run(&[&n]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(std::fs::read_to_string(m.path("usr/local/bin/veredas-novo.new")).unwrap(), "a tester's notes\n", "the old scratch name of a link the run made");
    assert_eq!(std::fs::read_link(m.path("usr/local/bin/vv")).unwrap(), PathBuf::from("/opt/veredas/current/bin/veredas-velho"), "a tester's alias kept");
    assert!(std::fs::read_link(m.path("usr/local/bin/veredas-velho")).is_err(), "the release's own link of an executable it no longer has dropped");
    let scratch: Vec<String> = std::fs::read_dir(m.path("usr/local/bin")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.contains(".atualizar-")).collect();
    assert_eq!(scratch, Vec::<String>::new(), "no scratch link left");
}

/// A library put before libc's (`LD_PRELOAD`) that makes calls fail as a full or failing disk
/// does: every `symlink` after the first `ALFORGE_FAIL_SYMLINK_AFTER` (ENOSPC), and an `unlink`
/// of a path holding `ALFORGE_FAIL_UNLINK` (EIO).
fn failing_disk(dir: &Path) -> PathBuf {
    let c = dir.join("disk.c");
    std::fs::write(
        &c,
        r#"#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
static int made = 0;
static int fail_link(void) { const char *n = getenv("ALFORGE_FAIL_SYMLINK_AFTER"); if (!n) return 0; return ++made > atoi(n); }
static int fail_unlink(const char *p) { const char *m = getenv("ALFORGE_FAIL_UNLINK"); return m && strstr(p, m); }
int symlink(const char *t, const char *l) { if (fail_link()) { errno = ENOSPC; return -1; } int (*f)(const char *, const char *) = dlsym(RTLD_NEXT, "symlink"); return f(t, l); }
int symlinkat(const char *t, int d, const char *l) { if (fail_link()) { errno = ENOSPC; return -1; } int (*f)(const char *, int, const char *) = dlsym(RTLD_NEXT, "symlinkat"); return f(t, d, l); }
int unlink(const char *p) { if (fail_unlink(p)) { errno = EIO; return -1; } int (*f)(const char *) = dlsym(RTLD_NEXT, "unlink"); return f(p); }
int unlinkat(int d, const char *p, int fl) { if (fail_unlink(p)) { errno = EIO; return -1; } int (*f)(int, const char *, int) = dlsym(RTLD_NEXT, "unlinkat"); return f(d, p, fl); }
"#,
    )
    .unwrap();
    let so = dir.join("disk.so");
    let out = Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(&so).arg(&c).arg("-ldl").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    so
}

#[test]
fn round_three_a_rollback_says_what_it_could_not_put_back_and_links_are_the_releases_by_their_own_name() {
    let tools = tempfile::tempdir().unwrap();
    let disk = failing_disk(tools.path());
    let w = tempfile::tempdir().unwrap();
    // `current` moved, then the disk fails: the first link, and `current`'s putting back, cannot
    // be written. Not "as it was": exit 4, what is left said, the folder `current` names kept.
    let m = Machine::new();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-novo", "veredas-portal"], |_| {});
    let out = m.command(&[&b], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_SYMLINK_AFTER", "1")]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.starts_with("veredas-atualizar: not installed, and the machine could not all be put back as it was:\n"), "{err}");
    let new = format!("veredas-1.1.0-{TARGET}");
    assert!(err.contains(&format!("`current` could not be put back on veredas-1.0.0-{TARGET} (No space left on device): it names {new}")), "{err}");
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(&new));
    assert!(m.path("opt/veredas").join(&new).join("bin/veredas-portal").is_file(), "the folder `current` names is kept");
    assert!(err.contains("to go back: sudo ln -sfn veredas-1.0.0-"), "{err}");
    assert_eq!(m.calls(), "");
    // Two of the release's own links dropped in name order, the second failing: the first put back.
    let m = Machine::new();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-antigo", m.path("usr/local/bin/veredas-antigo")).unwrap();
    let only = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |_| {});
    let before = m.snapshot();
    let out = m.command(&[&only], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_UNLINK", "usr/local/bin/veredas-velho")]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.ends_with(&format!("{}: Input/output error\n", m.path("usr/local/bin/veredas-velho").display())), "{err}");
    assert_eq!(m.snapshot(), before, "veredas-antigo's link put back, current and the folder too");
    // A tester's alias through `current` at an executable's name, naming another: refused.
    let m = Machine::new();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-velho", m.path("usr/local/bin/veredas-novo")).unwrap();
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(code, Some(2));
    assert!(err.contains(&format!("{} is not a link a release made", m.path("usr/local/bin/veredas-novo").display())), "{err}");
    assert_eq!(m.snapshot(), before);
}

#[test]
fn round_three_the_manifests_kinds_semvers_identifiers_the_archives_room_first_and_the_umask() {
    use std::os::unix::fs::PermissionsExt;
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let refused = |b: &Path| -> String {
        let (code, out, err) = m.run(&[b]);
        assert_eq!(code, Some(2), "{out}{err}");
        err
    };
    // `[tested]` without `together`, a component's version that is no SemVer.
    let no_together = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |d| {
        let t: String = std::fs::read_to_string(d.join("release.toml")).unwrap().lines().filter(|l| !l.starts_with("together = ")).map(|l| format!("{l}\n")).collect();
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert!(refused(&no_together).ends_with("  release.toml's [tested] does not state its `suite` and `together`\n"));
    let bad_version = bundle_of(w.path(), "1.1.1", TARGET, &["veredas-portal"], |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap().replace("\nversion = \"2.0.0\"\n", "\nversion = \"2.x\"\n");
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    assert!(refused(&bad_version).ends_with("  release.toml's component `veredas-portal` is not a name, a SemVer version and its executables\n"));
    // The archive's room judged before it is copied: one this user cannot read, the reserve above
    // the free room, refused for the room and not for the read.
    let locked = bundle_of(w.path(), "1.1.2", TARGET, &["veredas-portal"], |_| {});
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = m.command(&[&locked], &[("VEREDAS_SETTLE", "0"), ("VEREDAS_RESERVE_MB", "1000000000")]).output().unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(err.contains("  the bundle's archive needs ") && !err.contains("Permission denied"), "{err}");
    // SemVer: a numeric identifier ranks below an alphanumeric one.
    for (installed, offered, newer) in [("1.0.0-alpha.1", "1.0.0-alpha.beta", true), ("1.0.0-alpha.beta", "1.0.0-alpha.1", false)] {
        let m = Machine::with(installed);
        let b = bundle_of(w.path(), offered, TARGET, &["veredas-portal"], |_| {});
        let (code, out, err) = m.run(&[&b]);
        assert_eq!(code, Some(if newer { 0 } else { 2 }), "{installed} then {offered}: {out}{err}");
    }
    // Under a caller's umask of 077 the release's folders are still the services' to read.
    let m = Machine::new();
    let b = bundle_of(w.path(), "1.3.0", TARGET, &["veredas-portal"], |_| {});
    let script = m.stub.path().join("veredas-atualizar");
    std::fs::write(&script, maquina::ATUALIZAR).unwrap();
    let out = Command::new("sh")
        .args(["-c", "umask 077; exec python3 \"$0\" \"$1\""])
        .arg(&script)
        .arg(&b)
        .env("VEREDAS_ROOT", m.root.path())
        .env("VEREDAS_SETTLE", "0")
        .env("PATH", format!("{}:/usr/bin:/bin", m.stub.path().display()))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    for d in ["", "/bin", "/planta"] {
        let mode = std::fs::metadata(m.path(&format!("opt/veredas/veredas-1.3.0-{TARGET}{d}"))).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "{d}");
    }
}

#[test]
fn round_four_a_rollback_names_what_it_left_another_version_and_a_missing_symbol_version_refused() {
    let tools = tempfile::tempdir().unwrap();
    let disk = failing_disk(tools.path());
    let w = tempfile::tempdir().unwrap();
    // The rollback cannot remove the new release's folder (a file of it on a failing disk): not
    // "as it was", exit 4, the folder named.
    let m = Machine::new();
    let b = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |d| {
        std::fs::write(d.join("planta/veredas-velho.txt"), "x\n").unwrap();
        bundle::write_sums(d).unwrap();
    });
    let out = m.command(&[&b], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_UNLINK", "veredas-velho")]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(4), "{err}");
    let dest = m.path(&format!("opt/veredas/veredas-1.2.0-{TARGET}"));
    assert!(err.contains(&format!("the new release's folder {} could not all be removed: ", dest.display())), "{err}");
    assert!(err.contains(&format!("; a later update of this release is refused while it stands (remove it with `sudo rm -rf {}`)\n", dest.display())), "{err}");
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.0.0-{TARGET}")), "current put back");
    // A link that cannot be put back (two of the release's own dropped, the second failing, the
    // first's putting back failing): exit 4, the link and the kept folder named.
    let m = Machine::new();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-antigo", m.path("usr/local/bin/veredas-antigo")).unwrap();
    let only = bundle_of(w.path(), "1.3.0", TARGET, &["veredas-portal"], |_| {});
    let out = m
        .command(&[&only], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_UNLINK", "usr/local/bin/veredas-velho"), ("ALFORGE_FAIL_SYMLINK_AFTER", "2")])
        .output()
        .unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.contains(&format!("{} could not be put back on /opt/veredas/current/bin/veredas-antigo", m.path("usr/local/bin/veredas-antigo").display())), "{err}");
    assert!(err.contains(&format!("the new release's folder {} is kept", m.path(&format!("opt/veredas/veredas-1.3.0-{TARGET}")).display())), "{err}");
    // A manifest of another version than the command reads: refused, never guessed.
    let m = Machine::new();
    let v2 = bundle_of(w.path(), "1.4.0", TARGET, &["veredas-portal"], |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap().replacen("\nversion = 1\n", "\nversion = 2\n", 1);
        std::fs::write(d.join("release.toml"), t).unwrap();
        bundle::write_sums(d).unwrap();
    });
    let (code, _, err) = m.run(&[&v2]);
    assert_eq!((code, err), (Some(2), "veredas-atualizar: not installed, the machine is as it was:\n  release.toml is `veredas-release` version 2, not `veredas-release` version 1\n".to_string()));
    // An executable asking a symbol version its library on this machine does not have (as an
    // executable built against a newer glibc would): refused before anything is switched.
    let lib = tempfile::tempdir().unwrap();
    let l = lib.path();
    std::fs::write(l.join("v.c"), "int f(void) { return 0; }\n").unwrap();
    for (map, vers) in [("v2.map", "VERS_2"), ("v1.map", "VERS_1")] {
        std::fs::write(l.join(map), format!("{vers} {{ global: f; local: *; }};\n")).unwrap();
    }
    let cc = |args: &[&str]| assert!(Command::new("cc").current_dir(l).args(args).status().unwrap().success(), "{args:?}");
    cc(&["-shared", "-fPIC", "-Wl,--version-script=v2.map", "-Wl,-soname,libv.so", "-o", "libv.so", "v.c"]);
    let b = bundle_of(w.path(), "1.5.0", TARGET, &["veredas-portal"], |d| {
        std::fs::write(l.join("main.c"), "int f(void);\nint main(void) { return f(); }\n").unwrap();
        let exe = d.join("bin/veredas-portal");
        let out = Command::new("cc").current_dir(l).arg("-o").arg(&exe).arg("main.c").arg(format!("-Wl,-rpath,{}", l.display())).arg("-L.").arg("-lv").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        bundle::write_sums(d).unwrap();
    });
    cc(&["-shared", "-fPIC", "-Wl,--version-script=v1.map", "-Wl,-soname,libv.so", "-o", "libv.so", "v.c"]);
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("  bin/veredas-portal would not start here: ") && err.contains("version `VERS_2' not found"), "{err}");
    assert_eq!(m.snapshot(), before);
}


#[test]
fn round_five_the_lock_through_the_watch_the_checks_order_and_what_a_run_leaves() {
    use std::io::Write;
    let tools = tempfile::tempdir().unwrap();
    let disk = failing_disk(tools.path());
    let w = tempfile::tempdir().unwrap();
    let staging = |m: &Machine| -> Vec<String> {
        std::fs::read_dir(m.path("opt/veredas")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.starts_with(".atualizar-")).collect()
    };
    // The lock held through the restart and the watch: a second update started while the first
    // watches its services is refused at once, and `current` names the first's release.
    let m = Machine::new();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let c = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |_| {});
    let first = m.command(&[&b], &[("VEREDAS_SETTLE", "2")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let start = std::time::Instant::now();
    while !m.calls().contains("restart") {
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "the first update did not restart the services");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let second = within(m.command(&[&c], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap(), 30);
    assert_eq!((second.status.code(), String::from_utf8_lossy(&second.stderr).into_owned()), (Some(2), "veredas-atualizar: another update is running\n".to_string()));
    let out = within(first, 60);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).ends_with("veredas-atualizar: the bench and supervisão restarted on it and stayed up\n"));
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));
    assert_eq!(m.calls().matches("restart").count(), 1, "the second restarted nothing");

    // The checks in their stated order, the first that fails alone said: the sums before the
    // manifest, the manifest before the libraries, the libraries before the PATH.
    let reasons = |b: &Path, m: &Machine| -> Vec<String> {
        let before = m.snapshot();
        let (code, out, err) = m.run(&[b]);
        assert_eq!(code, Some(2), "{out}{err}");
        assert_eq!(m.snapshot(), before, "the machine as it was");
        let lines: Vec<&str> = err.lines().collect();
        assert_eq!(lines[0], "veredas-atualizar: not installed, the machine is as it was:", "{err}");
        lines[1..].iter().map(|l| l.trim_start().to_string()).collect()
    };
    let missing_library = |d: &Path| {
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("f.c"), "int fantasma(void) { return 0; }\n").unwrap();
        assert!(Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(lib.path().join("libfantasma.so")).arg(lib.path().join("f.c")).status().unwrap().success());
        exe(&d.join("bin/veredas-portal"), &["-Wl,--no-as-needed", &format!("-L{}", lib.path().display()), "-lfantasma"]);
    };
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let older_and_changed = bundle_of(w.path(), "0.9.0", TARGET, &["veredas-portal"], |d| std::fs::write(d.join("README.md"), "changed\n").unwrap());
    assert_eq!(reasons(&older_and_changed, &m), ["README.md does not match its sum"]);
    let w = tempfile::tempdir().unwrap();
    let bad_manifest_and_library = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |d| {
        let t = std::fs::read_to_string(d.join("release.toml")).unwrap().replace("format = \"veredas-release\"", "format = \"veredas-bench\"");
        std::fs::write(d.join("release.toml"), t).unwrap();
        missing_library(d);
        bundle::write_sums(d).unwrap();
    });
    assert_eq!(reasons(&bad_manifest_and_library, &m), ["release.toml is `veredas-bench` version 1, not `veredas-release` version 1"]);
    let w = tempfile::tempdir().unwrap();
    let library_and_path = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal", "veredas-outro"], |d| {
        missing_library(d);
        bundle::write_sums(d).unwrap();
    });
    std::fs::write(m.path("usr/local/bin/veredas-outro"), "a tester's own file\n").unwrap();
    assert_eq!(reasons(&library_and_path, &m), ["bin/veredas-portal would not start here: libfantasma.so => not found"]);
    std::fs::remove_file(m.path("usr/local/bin/veredas-outro")).unwrap();

    // A component's name and executables of their kinds: a number for a name, a string for the
    // list of executables, each refused by its shape.
    for (from, to, shown) in [("name = \"veredas-portal\"", "name = 5", "5"), ("bins = [\"veredas-portal\"]", "bins = \"veredas-portal\"", "veredas-portal")] {
        let w = tempfile::tempdir().unwrap();
        let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |d| {
            let t = std::fs::read_to_string(d.join("release.toml")).unwrap();
            assert!(t.contains(from), "{t}");
            std::fs::write(d.join("release.toml"), t.replacen(from, to, 1)).unwrap();
            bundle::write_sums(d).unwrap();
        });
        assert_eq!(reasons(&b, &m), [format!("release.toml's component `{shown}` is not a name, a SemVer version and its executables")]);
    }

    // The staging folder's removal judged: a refusal that cannot remove it is not "as it was" (exit
    // 4, the folder named, no way back to say: `current` never moved); an install that cannot says
    // so and stays installed. The next update removes it and says so.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let older = bundle_of(w.path(), "0.9.0", TARGET, &["veredas-portal"], |_| {});
    let out = m.command(&[&older], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_UNLINK", "bundle.tar.gz")]).output().unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(4), "{err}");
    let left = staging(&m);
    assert_eq!(left.len(), 1, "{left:?}");
    let stage = m.path(&format!("opt/veredas/{}", left[0]));
    assert_eq!(
        err,
        format!(
            "veredas-atualizar: not installed, and the machine could not all be put back as it was:\n  release 0.9.0 is not newer than the installed 1.0.0\n  its staging folder {s} could not all be removed: {s}/bundle.tar.gz (Input/output error); the next update removes it\n",
            s = stage.display()
        )
    );
    let w = tempfile::tempdir().unwrap();
    let newer = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let out = m.command(&[&newer], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", disk.to_str().unwrap()), ("ALFORGE_FAIL_UNLINK", "bundle.tar.gz")]).output().unwrap();
    let (stdout, err) = (String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap());
    assert_eq!(out.status.code(), Some(0), "{err}");
    // The earlier one could not be removed either (the same failing disk), and is said so; this
    // run's own staging is said, the release installed.
    assert!(err.starts_with(&format!("veredas-atualizar: what an earlier run left could not all be removed: {}/bundle.tar.gz (Input/output error)\n", stage.display())), "{err}");
    let this = err.lines().nth(1).unwrap_or_default();
    assert!(this.starts_with("veredas-atualizar: its staging folder ") && this.ends_with("/bundle.tar.gz (Input/output error); the next update removes it"), "{err}");
    assert_eq!(err.lines().count(), 2, "{err}");
    assert!(stdout.ends_with("veredas-atualizar: the bench and supervisão restarted on it and stayed up\n"), "{stdout}");
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));
    let mut was = staging(&m);
    was.sort();
    assert_eq!(was.len(), 2, "{was:?}");
    let w = tempfile::tempdir().unwrap();
    let next = bundle_of(w.path(), "1.2.0", TARGET, &["veredas-portal"], |_| {});
    let (code, stdout, err) = m.run(&[&next]);
    assert_eq!(code, Some(0), "{err}");
    for n in &was {
        assert!(stdout.contains(&format!("veredas-atualizar: removed what an earlier run left: {}\n", m.path(&format!("opt/veredas/{n}")).display())), "{stdout}");
    }
    assert_eq!(staging(&m), Vec::<String>::new());

    // A run killed while it reads the bundle (SIGKILL, as a power-off leaves it; a SIGTERM is a
    // refusal that removes its staging, round six): its staging folder stays, and
    // the next run removes it, with a scratch link a kill left beside `current` and on the PATH.
    // What is not a run's own (a dot file of another's, a link not ending in a process number) is
    // kept.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let bytes = std::fs::read(&b).unwrap();
    let fifo = w.path().join("slow.tar.gz");
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let child = m.command(&[&fifo], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    writer.write_all(&bytes[..bytes.len() / 2]).unwrap();
    writer.flush().unwrap();
    let start = std::time::Instant::now();
    // The copy's file made (the half written may still sit in its buffer): the run is reading.
    while staging(&m).is_empty() || !m.path(&format!("opt/veredas/{}/bundle.tar.gz", staging(&m)[0])).exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(30), "the run did not start copying");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(Command::new("kill").arg("-KILL").arg(child.id().to_string()).status().unwrap().success());
    drop(writer);
    let killed = within(child, 30);
    assert_eq!(killed.status.code(), None, "killed by its signal");
    let left = staging(&m);
    assert_eq!(left.len(), 1, "a killed run leaves its staging folder");
    std::os::unix::fs::symlink(format!("veredas-1.0.0-{TARGET}"), m.path("opt/veredas/.current.atualizar-4242")).unwrap();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-portal", m.path("usr/local/bin/.veredas-portal.atualizar-4242")).unwrap();
    std::os::unix::fs::symlink("/usr/bin/true", m.path("usr/local/bin/.alias.atualizar-mine")).unwrap();
    std::fs::write(m.path("opt/veredas/.atualizar-notes"), "a tester's own file\n").unwrap();
    let (code, stdout, err) = m.run(&[&b]);
    assert_eq!(code, Some(0), "{err}");
    for p in [format!("opt/veredas/{}", left[0]), "opt/veredas/.current.atualizar-4242".to_string(), "usr/local/bin/.veredas-portal.atualizar-4242".to_string()] {
        assert!(stdout.contains(&format!("veredas-atualizar: removed what an earlier run left: {}\n", m.path(&p).display())), "{p}: {stdout}");
        assert!(!m.path(&p).exists() && std::fs::symlink_metadata(m.path(&p)).is_err(), "{p} removed");
    }
    assert!(std::fs::symlink_metadata(m.path("usr/local/bin/.alias.atualizar-mine")).is_ok() && m.path("opt/veredas/.atualizar-notes").is_file(), "another's kept");
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")));
}

/// A library preloaded before libc's: a rename whose source path holds `ALFORGE_SLOW_RENAME`, or an
/// unlink whose path holds `ALFORGE_SLOW_UNLINK`, first creates the file `ALFORGE_SLOW_MARK`, then
/// takes two seconds (the install's moves, or a removal, to interrupt one while it runs).
fn slow_rename(dir: &Path) -> PathBuf {
    let c = dir.join("slow.c");
    std::fs::write(
        &c,
        r#"#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static void slow(const char *p) {
    const char *m = getenv("ALFORGE_SLOW_RENAME");
    const char *mark = getenv("ALFORGE_SLOW_MARK");
    if (m && strstr(p, m)) { if (mark) { int fd = open(mark, O_CREAT | O_WRONLY, 0644); if (fd >= 0) close(fd); } sleep(2); }
}
int rename(const char *a, const char *b) { slow(a); int (*f)(const char *, const char *) = dlsym(RTLD_NEXT, "rename"); return f(a, b); }
int renameat(int d1, const char *a, int d2, const char *b) { slow(a); int (*f)(int, const char *, int, const char *) = dlsym(RTLD_NEXT, "renameat"); return f(d1, a, d2, b); }
int renameat2(int d1, const char *a, int d2, const char *b, unsigned fl) { slow(a); int (*f)(int, const char *, int, const char *, unsigned) = dlsym(RTLD_NEXT, "renameat2"); return f(d1, a, d2, b, fl); }
static void slow_unlink(const char *p) {
    const char *m = getenv("ALFORGE_SLOW_UNLINK");
    const char *mark = getenv("ALFORGE_SLOW_MARK");
    if (m && strstr(p, m)) { if (mark) { int fd = open(mark, O_CREAT | O_WRONLY, 0644); if (fd >= 0) close(fd); } sleep(2); }
}
int unlink(const char *p) { slow_unlink(p); int (*f)(const char *) = dlsym(RTLD_NEXT, "unlink"); return f(p); }
int unlinkat(int d, const char *p, int fl) { slow_unlink(p); int (*f)(int, const char *, int) = dlsym(RTLD_NEXT, "unlinkat"); return f(d, p, fl); }
"#,
    )
    .unwrap();
    let so = dir.join("slow.so");
    let out = Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(&so).arg(&c).arg("-ldl").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    so
}

#[test]
fn round_six_leftovers_of_its_own_shapes_alone_and_an_interruption_a_refusal_or_the_way_back() {
    use std::io::Write;
    let tools = tempfile::tempdir().unwrap();
    let slow = slow_rename(tools.path());
    let w = tempfile::tempdir().unwrap();
    let wait_for = |what: &dyn Fn() -> bool, label: &str| {
        let start = std::time::Instant::now();
        while !what() {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "{label}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    let signal = |child: &std::process::Child, sig: &str| assert!(Command::new("kill").arg(format!("-{sig}")).arg(child.id().to_string()).status().unwrap().success());
    // What an earlier run left is removed in its own shapes alone: a staging folder as mkdtemp
    // names it, a scratch link ending in a process number. A dot folder of another name, a folder
    // `.atualizar-` of another shape, and a file or folder named like a scratch link are kept, and
    // nothing is said of them.
    let m = Machine::new();
    let own = [m.path("opt/veredas/.atualizar-ab12_x9z"), m.path("opt/veredas/.current.atualizar-77"), m.path("usr/local/bin/.veredas-portal.atualizar-78")];
    std::fs::create_dir_all(own[0].join("veredas-1.1.0")).unwrap();
    std::fs::write(own[0].join("bundle.tar.gz"), "half\n").unwrap();
    std::os::unix::fs::symlink(format!("veredas-1.0.0-{TARGET}"), &own[1]).unwrap();
    std::os::unix::fs::symlink("/opt/veredas/current/bin/veredas-portal", &own[2]).unwrap();
    let others = ["opt/veredas/.notas", "opt/veredas/.atualizar-x.d", "opt/veredas/.atualizar-mine", "opt/veredas/.atualizar-ab12_x9z0", "opt/veredas/.x.atualizar-2"];
    for o in others {
        std::fs::create_dir_all(m.path(o)).unwrap();
        std::fs::write(m.path(o).join("kept.txt"), "another's\n").unwrap();
    }
    std::fs::write(m.path("opt/veredas/.x.atualizar-1"), "another's\n").unwrap();
    // A file in a staging folder's shape: no run's staging (a run's is a folder), kept.
    std::fs::write(m.path("opt/veredas/.atualizar-zz99zz99"), "another's\n").unwrap();
    std::fs::write(m.path("usr/local/bin/.y.atualizar-3"), "another's\n").unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let (code, out, err) = m.run(&[&b]);
    assert_eq!((code, err.as_str()), (Some(0), ""), "{out}");
    let removed: Vec<&str> = out.lines().filter(|l| l.starts_with("veredas-atualizar: removed what an earlier run left: ")).collect();
    let mut want: Vec<String> = own.iter().map(|p| format!("veredas-atualizar: removed what an earlier run left: {}", p.display())).collect();
    want.sort();
    assert_eq!(removed, want);
    for o in others {
        assert!(m.path(o).join("kept.txt").is_file(), "{o} kept");
    }
    assert!(m.path("opt/veredas/.x.atualizar-1").is_file() && m.path("usr/local/bin/.y.atualizar-3").is_file() && m.path("opt/veredas/.atualizar-zz99zz99").is_file());

    // Interrupted while it reads the bundle (Ctrl-C, a closed terminal, a shutdown's SIGTERM): a
    // refusal, the staging folder removed, the machine as it was.
    for sig in ["INT", "HUP", "TERM"] {
        let m = Machine::new();
        let w = tempfile::tempdir().unwrap();
        let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
        let bytes = std::fs::read(&b).unwrap();
        let fifo = w.path().join("slow.tar.gz");
        assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let before = m.snapshot();
        let child = m.command(&[&fifo], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        writer.write_all(&bytes[..bytes.len() / 2]).unwrap();
        writer.flush().unwrap();
        let opt = m.path("opt/veredas");
        wait_for(&|| std::fs::read_dir(&opt).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".atualizar-")), "the run did not start reading");
        signal(&child, sig);
        // The bundle's end closed at once: a run that missed its signal ends on the cut bundle and
        // says so, rather than waiting on the FIFO.
        drop(writer);
        let out = within(child, 30);
        assert_eq!(
            (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
            (Some(2), format!("veredas-atualizar: not installed, the machine is as it was:\n  stopped by SIG{sig} before anything was installed\n")),
            "SIG{sig}"
        );
        assert_eq!(m.snapshot(), before, "SIG{sig}: the machine as it was, no staging left");
    }

    // Interrupted while the install moves the release (a rename that takes its time): the install
    // runs whole first, then the interruption is no verdict on the services, the way back said.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal", "veredas-novo"], |_| {});
    let mark = tools.path().join("renaming");
    let child = m
        .command(&[&b], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", slow.to_str().unwrap()), ("ALFORGE_SLOW_RENAME", ".atualizar-"), ("ALFORGE_SLOW_MARK", mark.to_str().unwrap())])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&|| mark.exists(), "the install did not start");
    signal(&child, "TERM");
    let out = within(child, 60);
    let back = format!("sudo ln -sfn veredas-1.0.0-{TARGET} /opt/veredas/current && sudo systemctl restart veredas-bancada.service veredas-supervisao.service");
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(3), format!("veredas-atualizar: installed, and stopped by SIGTERM before the bench and supervisão were seen to stay up on it; to go back: {back}\n"))
    );
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.1.0-{TARGET}")), "the install whole");
    for b in ["veredas-portal", "veredas-novo"] {
        assert_eq!(std::fs::read_link(m.path(&format!("usr/local/bin/{b}"))).unwrap(), PathBuf::from(format!("/opt/veredas/current/bin/{b}")), "{b}");
    }
    assert!(std::fs::symlink_metadata(m.path("usr/local/bin/veredas-velho")).is_err(), "the dropped executable's link gone");
    assert_eq!(m.calls().matches("restart").count(), 1, "the services restarted on it before the interruption was taken");
    assert!(!std::fs::read_dir(m.path("opt/veredas")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".")), "no staging or scratch link left");

    // Interrupted while it watches the services: installed, the way back said (exit 3).
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let child = m.command(&[&b], &[("VEREDAS_SETTLE", "3")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    wait_for(&|| m.calls().contains("restart"), "the services were not restarted");
    signal(&child, "INT");
    let out = within(child, 30);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(3), format!("veredas-atualizar: installed, and stopped by SIGINT before the bench and supervisão were seen to stay up on it; to go back: {back}\n"))
    );
    assert!(String::from_utf8_lossy(&out.stdout).ends_with("veredas-atualizar: restarting the bench and supervisão on it, and watching them for 6 s\n"));

    // A folder of another's where an executable would go on the PATH: refused, the machine as it
    // was.
    let m = Machine::new();
    std::fs::create_dir_all(m.path("usr/local/bin/veredas-outro")).unwrap();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal", "veredas-outro"], |_| {});
    let before = m.snapshot();
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(
        (code, err),
        (
            Some(2),
            format!("veredas-atualizar: not installed, the machine is as it was:\n  {} is not a link a release made: an executable of the release would replace it\n", m.path("usr/local/bin/veredas-outro").display())
        )
    );
    assert_eq!(m.snapshot(), before);
}

#[test]
fn round_seven_a_shutdowns_two_signals_and_an_interruption_inside_the_archives_reading() {
    use std::io::{BufRead, Write};
    let wait_for = |what: &dyn Fn() -> bool, label: &str| {
        let start = std::time::Instant::now();
        while !what() {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "{label}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    // A shutdown's SIGTERM and SIGHUP, back to back, as systemd stops a login session.
    let both = |child: &std::process::Child| {
        let p = child.id();
        assert!(Command::new("sh").arg("-c").arg(format!("kill -TERM {p}; kill -HUP {p}")).status().unwrap().success());
    };
    // While it reads the bundle: the first taken, the refusal said whole, the staging removed.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let bytes = std::fs::read(&b).unwrap();
    let fifo = w.path().join("slow.tar.gz");
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let before = m.snapshot();
    let child = m.command(&[&fifo], &[("VEREDAS_SETTLE", "0")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    writer.write_all(&bytes[..bytes.len() / 2]).unwrap();
    writer.flush().unwrap();
    let opt = m.path("opt/veredas");
    wait_for(&|| std::fs::read_dir(&opt).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".atualizar-")), "the run did not start reading");
    both(&child);
    drop(writer);
    let out = within(child, 30);
    // Both pending at once, Python takes them in the signals' order: the first taken is said.
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(
        ["SIGTERM", "SIGHUP"].iter().any(|s| err == format!("veredas-atualizar: not installed, the machine is as it was:\n  stopped by {s} before anything was installed\n")),
        "{err}"
    );
    assert_eq!(m.snapshot(), before, "the machine as it was, no staging left");
    // While it watches the services: installed, the way back said whole.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let child = m.command(&[&b], &[("VEREDAS_SETTLE", "3")]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    wait_for(&|| m.calls().contains("restart"), "the services were not restarted");
    both(&child);
    let out = within(child, 30);
    let back = format!("sudo ln -sfn veredas-1.0.0-{TARGET} /opt/veredas/current && sudo systemctl restart veredas-bancada.service veredas-supervisao.service");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(3), "{err}");
    assert!(
        ["SIGTERM", "SIGHUP"].iter().any(|s| err == format!("veredas-atualizar: installed, and stopped by {s} before the bench and supervisão were seen to stay up on it; to go back: {back}\n")),
        "{err}"
    );

    // Interrupted inside the archive's reading, where every reading error is a damaged bundle: an
    // interruption is said as one, never as the bundle's fault. The bundle holds a sparse GiB of
    // zeros, whose gzip stream takes its time to read; the reserve is set so that the copy fits and
    // the extraction would not, so nothing large is ever written.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |d| {
        std::fs::File::create(d.join("planta/zeros.bin")).unwrap().set_len(1 << 30).unwrap();
        bundle::write_sums(d).unwrap();
    });
    let avail = Command::new("df").args(["--output=avail", "-B1"]).arg(m.path("opt/veredas")).output().unwrap();
    let avail: u64 = String::from_utf8_lossy(&avail.stdout).lines().nth(1).unwrap().trim().parse().unwrap();
    let reserve_mb = ((avail - (512 << 20)) >> 20).to_string();
    let before = m.snapshot();
    let mut child = m
        // Its lines handed over as they are printed, so the test sees the digest when it is said.
        .command(&[&b], &[("VEREDAS_SETTLE", "0"), ("VEREDAS_RESERVE_MB", reserve_mb.as_str()), ("PYTHONUNBUFFERED", "1")])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // The archive's digest said: the copy is done and its reading begins.
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let first = lines.next().unwrap().unwrap();
    assert!(first.contains(" (SHA-256 "), "{first}");
    assert!(Command::new("kill").arg("-INT").arg(child.id().to_string()).status().unwrap().success());
    let out = within(child, 60);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(2), "veredas-atualizar: not installed, the machine is as it was:\n  stopped by SIGINT before anything was installed\n".to_string())
    );
    assert_eq!(m.snapshot(), before);
}

#[test]
fn round_eight_an_interruption_inside_a_refusals_cleanup_or_the_leftovers_and_a_current_ending_in_a_slash() {
    let tools = tempfile::tempdir().unwrap();
    let slow = slow_rename(tools.path());
    let wait_for = |what: &dyn Fn() -> bool, label: &str| {
        let start = std::time::Instant::now();
        while !what() {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "{label}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    let signal = |child: &std::process::Child, sig: &str| assert!(Command::new("kill").arg(format!("-{sig}")).arg(child.id().to_string()).status().unwrap().success());
    let back = format!("sudo ln -sfn veredas-1.0.0-{TARGET} /opt/veredas/current && sudo systemctl restart veredas-bancada.service veredas-supervisao.service");
    // Interrupted while a refusal removes its staging folder: held, the refusal said whole, the
    // machine as it was.
    let m = Machine::new();
    let w = tempfile::tempdir().unwrap();
    let older = bundle_of(w.path(), "0.9.0", TARGET, &["veredas-portal"], |_| {});
    let mark = tools.path().join("removing");
    let before = m.snapshot();
    let child = m
        .command(&[&older], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", slow.to_str().unwrap()), ("ALFORGE_SLOW_UNLINK", "bundle.tar.gz"), ("ALFORGE_SLOW_MARK", mark.to_str().unwrap())])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&|| mark.exists(), "the staging folder's removal did not start");
    signal(&child, "INT");
    let out = within(child, 60);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(2), "veredas-atualizar: not installed, the machine is as it was:\n  release 0.9.0 is not newer than the installed 1.0.0\n".to_string())
    );
    assert_eq!(m.snapshot(), before);
    // Interrupted while it removes what a killed run left, before the bundle is read: said, and the
    // next run removes the rest.
    let m = Machine::new();
    let leftover = m.path("opt/veredas/.atualizar-ab12cd34");
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("bundle.tar.gz"), "half\n").unwrap();
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let mark = tools.path().join("leftovers");
    let child = m
        .command(&[&b], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", slow.to_str().unwrap()), ("ALFORGE_SLOW_UNLINK", "bundle.tar.gz"), ("ALFORGE_SLOW_MARK", mark.to_str().unwrap())])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&|| mark.exists(), "the leftovers' removal did not start");
    signal(&child, "TERM");
    let out = within(child, 60);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(2), "veredas-atualizar: not installed, stopped by SIGTERM before the bundle was read; what this run or an earlier one left in /opt/veredas the next update removes\n".to_string())
    );
    assert_eq!(std::fs::read_link(m.path("opt/veredas/current")).unwrap(), PathBuf::from(format!("veredas-1.0.0-{TARGET}")));
    let (code, _, err) = m.run(&[&b]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!leftover.exists() && std::fs::read_dir(m.path("opt/veredas")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".atualizar-")));
    // A restart that fails while an interruption is held (one held since the install): the failure
    // is said, the way back with it.
    let m = Machine::new();
    m.systemctl(1);
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let mark = tools.path().join("renaming");
    let child = m
        .command(&[&b], &[("VEREDAS_SETTLE", "0"), ("LD_PRELOAD", slow.to_str().unwrap()), ("ALFORGE_SLOW_RENAME", ".atualizar-"), ("ALFORGE_SLOW_MARK", mark.to_str().unwrap())])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&|| mark.exists(), "the install did not start");
    signal(&child, "TERM");
    let out = within(child, 60);
    assert_eq!(
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned()),
        (Some(3), format!("veredas-atualizar: the bench and supervisão did not stay up on it (systemctl exited 1); to go back: {back}\n"))
    );
    // `current` ending in a slash, as a going back by tab completion leaves it: the old release and
    // the way back named by its folder.
    let m = Machine::new();
    std::fs::remove_file(m.path("opt/veredas/current")).unwrap();
    std::os::unix::fs::symlink(format!("veredas-1.0.0-{TARGET}/"), m.path("opt/veredas/current")).unwrap();
    m.systemctl(1);
    let w = tempfile::tempdir().unwrap();
    let b = bundle_of(w.path(), "1.1.0", TARGET, &["veredas-portal"], |_| {});
    let (code, out, err) = m.run(&[&b]);
    assert_eq!((code, err), (Some(3), format!("veredas-atualizar: the bench and supervisão did not stay up on it (systemctl exited 1); to go back: {back}\n")));
    assert!(out.contains(&format!("the old release kept at {}\n", m.path(&format!("opt/veredas/veredas-1.0.0-{TARGET}")).display())), "{out}");
}
