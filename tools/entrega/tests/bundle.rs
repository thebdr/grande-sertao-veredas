//! C-291, the parts of the cut that need no machine: an executable's needs read from `objdump -p`
//! and held to the tester machine's libraries; `SHA256SUMS` written and every disagreement named;
//! the manifest the cut writes read back by the reader; the recipe file read and refused by key;
//! the cut refusing a tree that differs from HEAD and a release already cut; the run together
//! refused for every way the plant is not running, each named. The run of a real bundle is
//! `tests/cut.rs` (feature `corte`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use entrega::bundle;
use entrega::cut::{self, CutError, Options};
use entrega::machine::Recipe;
use entrega::manifest;
use entrega::together::{self, Answer, Report};
use serde_json::{Value, json};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn recipe() -> Recipe {
    Recipe::load_file(&repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap()
}

/// What `objdump -p` prints of a dynamic executable's version references (abridged from portal's,
/// built on the development machine).
const OBJDUMP: &str = "\nVersion References:\n  required from libgcc_s.so.1:\n    0x0b792650 0x00 13 GCC_3.0\n    0x09276fd3 0x00 10 GCC_4.2.0\n  required from libstdc++.so.6:\n    0x0bafd179 0x00 14 CXXABI_1.3.15\n    0x08922974 0x00 09 GLIBCXX_3.4\n    0x02297f89 0x00 07 GLIBCXX_3.4.29\n    0x056bafd3 0x00 05 CXXABI_1.3\n    0x08922979 0x00 04 GLIBCXX_3.4.9\n  required from libm.so.6:\n    0x06969194 0x00 12 GLIBC_2.44\n    0x09691a75 0x00 03 GLIBC_2.2.5\n  required from libc.so.6:\n    0x06969188 0x00 16 GLIBC_2.38\n    0x069691b4 0x00 15 GLIBC_2.34\n    0x09691974 0x00 08 GLIBC_2.3.4\n";

#[test]
fn an_executables_needs_are_its_newest_versions_and_held_to_the_machines() {
    let n = bundle::needs(OBJDUMP);
    let got: Vec<(&str, &str)> = n.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(got, [("CXXABI", "1.3.15"), ("GLIBC", "2.44"), ("GLIBCXX", "3.4.29")]);
    assert!(bundle::newer_than("2.44", "2.41") && bundle::newer_than("3.4.33", "3.4.9") && bundle::newer_than("2.3.4", "2.3"));
    assert!(!bundle::newer_than("2.41", "2.41") && !bundle::newer_than("2.2.5", "2.41") && !bundle::newer_than("1.3.15", "1.3.15"));
    let r = recipe();
    assert_eq!((r.base.glibc.as_str(), r.base.glibcxx.as_str(), r.base.cxxabi.as_str()), ("2.41", "3.4.33", "1.3.15"));
    assert_eq!(
        bundle::check_needs("veredas-portal", &n, &r),
        vec!["veredas-portal needs GLIBC_2.44, the tester machine offers GLIBC_2.41: it would not start there".to_string()]
    );
    let built_there = OBJDUMP.replace("GLIBC_2.44", "GLIBC_2.29");
    assert_eq!(bundle::check_needs("veredas-portal", &bundle::needs(&built_there), &r), Vec::<String>::new());
    let newer_cxx = OBJDUMP.replace("GLIBCXX_3.4.29", "GLIBCXX_3.4.34").replace("CXXABI_1.3.15", "CXXABI_1.3.16").replace("GLIBC_2.44", "GLIBC_2.41");
    assert_eq!(
        bundle::check_needs("x", &bundle::needs(&newer_cxx), &r),
        vec![
            "x needs GLIBCXX_3.4.34, the tester machine offers GLIBCXX_3.4.33: it would not start there".to_string(),
            "x needs CXXABI_1.3.16, the tester machine offers CXXABI_1.3.15: it would not start there".to_string(),
        ]
    );
    // The bench's executable, which links no C++, needs none of the C++ library's versions.
    assert!(!bundle::needs("required from libc.so.6:\n 0x0 0x00 02 GLIBC_2.34\n").contains_key("GLIBCXX"));
}

#[test]
fn sums_cover_every_file_and_name_every_disagreement() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("bin")).unwrap();
    std::fs::write(d.path().join("bin/a"), "a").unwrap();
    std::fs::write(d.path().join("release.toml"), "r").unwrap();
    bundle::write_sums(d.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.path().join("SHA256SUMS")).unwrap(),
        "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb  bin/a\n\
         454349e422f05297191ead13e21d3db520e5abef52055e4964b82fb213f593a1  release.toml\n"
    );
    assert_eq!(bundle::verify_sums(d.path()).unwrap(), Vec::<String>::new());
    std::fs::write(d.path().join("bin/a"), "b").unwrap();
    std::fs::write(d.path().join("extra"), "x").unwrap();
    std::fs::remove_file(d.path().join("release.toml")).unwrap();
    assert_eq!(
        bundle::verify_sums(d.path()).unwrap(),
        ["bin/a does not match its sum", "extra is not in SHA256SUMS", "release.toml is in SHA256SUMS and not in the bundle"]
    );
}

#[test]
fn the_manifest_a_cut_writes_reads_back_as_written() {
    let components = vec![
        ("orcl-std".to_string(), "2.0.0-alpha.1".to_string(), vec![]),
        ("veredas-portal".to_string(), "2.0.0-alpha.1".to_string(), vec!["veredas-portal".to_string()]),
    ];
    let release = semver::Version::parse("2.0.0-alpha.1").unwrap();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let text = cut::manifest_text(&release, commit, "x86_64-unknown-linux-gnu", "rustc 1.100.0 (0ef1bc386 2026-11-09)", &components, cut::SUITE);
    let m = manifest::load(&text).unwrap();
    assert_eq!((m.release.to_string(), m.commit.as_str(), m.toolchain.as_str()), ("2.0.0-alpha.1".to_string(), commit, "rustc 1.100.0 (0ef1bc386 2026-11-09)"));
    let got: Vec<(String, String, Vec<String>)> = m.components.iter().map(|c| (c.name.clone(), c.version.to_string(), c.bins.clone())).collect();
    assert_eq!(got, components);
    assert_eq!((m.tested.suite.as_str(), m.tested.together.as_str()), ("cargo nextest run --workspace", "the bundle's bench and supervisão on its plant"));
    let trial = manifest::load(&cut::manifest_text(&release, commit, "x86_64-unknown-linux-gnu", "rustc 1", &components, cut::NOT_RUN)).unwrap();
    assert_eq!(trial.tested.suite, "not run: a trial cut");
}

#[test]
fn the_recipe_is_read_through_cores_header_and_refused_by_key() {
    let text = std::fs::read_to_string(repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    let r = Recipe::load(&text).unwrap();
    assert_eq!(r.base_name(), "debian-13-generic-amd64-20261116-2664.qcow2");
    assert_eq!((r.builder.cpus, r.builder.memory_mb), (4, 6144), "one machine at a time takes at most 4 cores and 6 GB here");
    assert_eq!((r.machine.cpus, r.machine.memory_mb), (2, 4096));
    assert_eq!((r.machine.user.as_str(), r.machine.supervisao_port, r.machine.hardware_version), ("veredas", 8902, 14));
    let refused = |from: &str, to: &str| {
        assert_eq!(text.matches(from).count(), 1, "{from}");
        Recipe::load(&text.replacen(from, to, 1)).unwrap_err().to_string()
    };
    assert_eq!(refused("format = \"veredas-machine\"", "format = \"veredas-bench\""), "`format` is `veredas-bench`, not `veredas-machine`");
    assert!(refused("cpus = 4", "cpu = 4").starts_with("a machine recipe: unknown field `cpu`"));
    assert_eq!(refused("url = \"https://", "url = \"http://"), "a machine recipe: `base.url` is `http://cloud.debian.org/images/cloud/trixie/20261116-2664/debian-13-generic-amd64-20261116-2664.qcow2`, not an https address");
    assert_eq!(refused("sha512 = \"e53a", "sha512 = \"E53a"), "a machine recipe: `base.sha512` is not 128 lowercase hexadecimal characters");
    assert_eq!(refused("\"chromium\"", "\"chromium; rm\""), "a machine recipe: `chromium; rm` is not a package's name");
    assert_eq!(refused("user = \"veredas\"", "user = \"Veredas\""), "a machine recipe: `machine.user` is `Veredas`, not a lowercase login");
    assert_eq!(refused("password = \"veredas\"", "password = \"a\\\"b\""), "a machine recipe: `machine.password` is empty or holds a quote, a backslash, a colon or a line break");
}

/// A throwaway clone of this repository at HEAD, so the cut's refusals are seen without touching
/// the working tree.
fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let out = Command::new("git").args(["clone", "-q", "--no-local", "--depth", "1"]).arg(format!("file://{}", repo_root().display())).arg(d.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    d
}

#[test]
fn the_cut_refuses_a_tree_that_differs_from_head_and_a_release_already_cut() {
    let c = clone();
    let out = tempfile::tempdir().unwrap();
    let options = |release: &str| Options {
        release: semver::Version::parse(release).unwrap(),
        out: out.path().to_path_buf(),
        work: c.path().join("target/entrega").join(release),
        run_suite: false,
    };
    std::fs::write(out.path().join("2.0.0.toml"), "").unwrap();
    match cut::cut(c.path(), &options("2.0.0")) {
        Err(CutError::Refused(e)) => assert_eq!(e, format!("{} exists: release 2.0.0 is cut already", out.path().join("2.0.0.toml").display())),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
    std::fs::write(c.path().join("README.md"), "changed\n").unwrap();
    match cut::cut(c.path(), &options("2.0.1")) {
        Err(CutError::Refused(e)) => assert!(e.contains("the tree differs from HEAD") && e.contains(" M README.md"), "{e}"),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
}

#[test]
fn a_bundle_packs_to_the_same_bytes_whatever_the_modes_its_files_were_made_with() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("veredas-x");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("bin/tool"), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.join("docs/a.md"), "a\n").unwrap();
    let set = |path: &str, mode: u32| std::fs::set_permissions(dir.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
    // As a umask of 0077 would leave them, then as 0002 would.
    for (p, m) in [("bin/tool", 0o700), ("docs/a.md", 0o600), ("docs", 0o700), ("bin", 0o700)] {
        set(p, m);
    }
    let first = work.path().join("first.tar.gz");
    bundle::pack(work.path(), "veredas-x", "1795300000", &first).unwrap();
    // As a umask of 0002 would leave them, under a folder whose setgid bit its children inherit.
    for (p, m) in [("bin/tool", 0o775), ("docs/a.md", 0o664), ("docs", 0o2775), ("bin", 0o2775)] {
        set(p, m);
    }
    let second = work.path().join("second.tar.gz");
    bundle::pack(work.path(), "veredas-x", "1795300000", &second).unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
    let listing = Command::new("tar").arg("-tvzf").arg(&first).output().unwrap();
    let modes: Vec<String> = String::from_utf8(listing.stdout).unwrap().lines().map(|l| l.split_whitespace().next().unwrap().to_string()).collect();
    assert_eq!(modes, ["drwxr-xr-x", "drwxr-xr-x", "-rwxr-xr-x", "drwxr-xr-x", "-rw-r--r--"]);
}

#[test]
fn the_recipes_library_limits_are_versions_of_dotted_digits() {
    let text = std::fs::read_to_string(repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    for (from, to, key, value) in [
        ("glibc = \"2.41\"", "glibc = \"v2.41\"", "base.glibc", "v2.41"),
        ("glibcxx = \"3.4.33\"", "glibcxx = \"3.4.\"", "base.glibcxx", "3.4."),
        ("cxxabi = \"1.3.15\"", "cxxabi = \"\"", "base.cxxabi", ""),
    ] {
        assert_eq!(text.matches(from).count(), 1, "{from}");
        assert_eq!(
            Recipe::load(&text.replacen(from, to, 1)).unwrap_err().to_string(),
            format!("a machine recipe: `{key}` is `{value}`, not a version of dotted digits")
        );
    }
}

#[test]
fn the_cut_refuses_an_untracked_file_and_a_release_already_in_releases() {
    let c = clone();
    let out = tempfile::tempdir().unwrap();
    let options = |release: &str| Options {
        release: semver::Version::parse(release).unwrap(),
        out: out.path().to_path_buf(),
        work: c.path().join("target/entrega").join(release),
        run_suite: false,
    };
    match cut::cut(c.path(), &options("2.0.0-alpha.1")) {
        Err(CutError::Refused(e)) => assert_eq!(e, format!("{} exists: release 2.0.0-alpha.1 is cut already", c.path().join("releases/2.0.0-alpha.1.toml").display())),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
    std::fs::write(c.path().join("notes.txt"), "never committed\n").unwrap();
    match cut::cut(c.path(), &options("9.9.9")) {
        Err(CutError::Refused(e)) => assert!(e.contains("the tree differs from HEAD") && e.contains("?? notes.txt"), "{e}"),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
}

/// A throwaway repository holding the recipe and one component, committed on `main`.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.dir.path())
            .args(["-c", "user.name=entrega", "-c", "user.email=entrega@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
        self.git(&["rev-parse", "HEAD"])
    }
}

#[test]
fn the_cut_refuses_a_version_another_release_pinned_at_other_code_before_building_anything() {
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("tools/entrega/maquina/maquina.toml", &std::fs::read_to_string(repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap());
    let c1 = r.commit("core 2.0.0");
    let components = vec![("veredas-core".to_string(), "2.0.0".to_string(), vec![])];
    let first = semver::Version::parse("2.0.0").unwrap();
    r.write("releases/2.0.0.toml", &cut::manifest_text(&first, &c1, "x86_64-unknown-linux-gnu", "rustc 1", &components, cut::SUITE));
    r.commit("release 2.0.0");
    r.write("crates/veredas-core/src/lib.rs", "pub fn changed() {}\n");
    r.commit("core changed, its version not bumped");
    let work = r.dir.path().join("target/entrega/2.0.1");
    // A real cut (the suite is to run): the one-tree check comes first, so nothing runs or builds.
    let options = Options { release: semver::Version::parse("2.0.1").unwrap(), out: r.dir.path().join("releases"), work: work.clone(), run_suite: true };
    match cut::cut(r.dir.path(), &options) {
        Err(CutError::Refused(e)) => assert_eq!(
            e,
            "release 2.0.1 would not check:\nveredas-core 2.0.0 is pinned by releases 2.0.0 and 2.0.1 at different code: a version names one tree"
        ),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
    assert!(!work.exists(), "refused before anything was run or built");
    assert!(!r.dir.path().join("releases/2.0.1.toml").exists());
}

#[test]
fn the_tree_is_hers_only_when_git_looks_at_every_file() {
    // Untracked files counted whatever `status.showUntrackedFiles` says; a file git is told not to
    // look at, either way, makes the tree unknown; HEAD moved is said as such.
    let c = clone();
    let commit = Command::new("git").arg("-C").arg(c.path()).args(["rev-parse", "HEAD"]).output().unwrap();
    let commit = String::from_utf8(commit.stdout).unwrap().trim().to_string();
    let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(c.path()).args(args).status().unwrap().success(), "{args:?}");
    assert_eq!(cut::differs_from(c.path(), &commit).unwrap(), None);
    git(&["config", "status.showUntrackedFiles", "no"]);
    std::fs::write(c.path().join("notes.txt"), "never committed\n").unwrap();
    let why = cut::differs_from(c.path(), &commit).unwrap().unwrap();
    assert!(why.contains("?? notes.txt"), "{why}");
    std::fs::remove_file(c.path().join("notes.txt")).unwrap();
    for flag in ["--skip-worktree", "--assume-unchanged"] {
        git(&["update-index", flag, "README.md"]);
        std::fs::write(c.path().join("README.md"), "changed behind git's back\n").unwrap();
        let why = cut::differs_from(c.path(), &commit).unwrap().unwrap();
        assert!(why.starts_with("files are marked for git not to look at them") && why.contains("README.md"), "{flag}: {why}");
        git(&["update-index", &flag.replace("--", "--no-"), "README.md"]);
        git(&["checkout", "--", "README.md"]);
    }
    assert_eq!(cut::differs_from(c.path(), &commit).unwrap(), None);
    std::fs::write(c.path().join("docs/x.md"), "x\n").unwrap();
    git(&["add", "docs/x.md"]);
    git(&["-c", "user.name=e", "-c", "user.email=e@example.invalid", "commit", "-q", "-m", "moved on"]);
    let why = cut::differs_from(c.path(), &commit).unwrap().unwrap();
    assert!(why.starts_with("HEAD is ") && why.ends_with(&format!("not {commit}: a release is cut from one commit")), "{why}");
}

#[test]
fn a_trials_manifest_never_goes_into_releases() {
    let c = clone();
    let options = Options {
        release: semver::Version::parse("9.9.9-trial.1").unwrap(),
        out: c.path().join("releases"),
        work: c.path().join("target/entrega/9.9.9-trial.1"),
        run_suite: false,
    };
    match cut::cut(c.path(), &options) {
        Err(CutError::Refused(e)) => assert_eq!(e, format!("a trial's manifest is never committed: it goes elsewhere than {}", c.path().join("releases").display())),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
}

#[test]
fn a_file_ignored_by_a_rule_the_commit_does_not_hold_makes_the_tree_another() {
    let c = clone();
    let commit = String::from_utf8(Command::new("git").arg("-C").arg(c.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap().trim().to_string();
    // What the commit's own `.gitignore` ignores is no difference (a build folder).
    std::fs::create_dir_all(c.path().join("target/debug")).unwrap();
    std::fs::write(c.path().join("target/debug/x"), "built\n").unwrap();
    assert_eq!(cut::differs_from(c.path(), &commit).unwrap(), None);
    // A file hidden by an untracked `.gitignore` that ignores itself too.
    std::fs::write(c.path().join("crates/veredas-core/build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(c.path().join("crates/veredas-core/.gitignore"), ".gitignore\nbuild.rs\n").unwrap();
    let why = cut::differs_from(c.path(), &commit).unwrap().unwrap();
    assert!(why.starts_with("files are ignored by rules the commit does not hold") && why.contains("crates/veredas-core/build.rs"), "{why}");
    std::fs::remove_file(c.path().join("crates/veredas-core/.gitignore")).unwrap();
    // A file hidden by `.git/info/exclude`.
    std::fs::write(c.path().join(".git/info/exclude"), "/crates/veredas-core/build.rs\n").unwrap();
    let why = cut::differs_from(c.path(), &commit).unwrap().unwrap();
    assert!(why.contains("crates/veredas-core/build.rs (ignored by .git/info/exclude)"), "{why}");
    std::fs::remove_file(c.path().join("crates/veredas-core/build.rs")).unwrap();
    assert_eq!(cut::differs_from(c.path(), &commit).unwrap(), None);
}

#[test]
fn the_suite_runs_the_default_profile_whatever_the_shell_holds() {
    let env = [
        ("NEXTEST_PROFILE", "no-browser"),
        ("NEXTEST_RETRIES", "5"),
        ("CARGO_BUILD_JOBS", "16"),
        ("CARGO_TARGET_DIR", "/elsewhere"),
        ("RUSTFLAGS", "--cfg probe"),
        ("RUSTC_WRAPPER", "sccache"),
        ("RUSTUP_TOOLCHAIN", "nightly"),
        ("CARGO_HOME", "/home/x/.cargo"),
        ("PATH", "/usr/bin"),
    ]
    .map(|(k, v)| (std::ffi::OsString::from(k), std::ffi::OsString::from(v)));
    let c = cut::suite_command(Path::new("/repo"), Path::new("/tmpdir"), &["cargo", "nextest", "run", "--workspace", "--profile", "default"], env.into_iter());
    let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(args, ["nextest", "run", "--workspace", "--profile", "default"]);
    // The child's environment is the given one alone, the steering variables left out.
    let envs: std::collections::BTreeMap<String, Option<String>> =
        c.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
    let set: Vec<(&str, &str)> = envs.iter().filter_map(|(k, v)| v.as_deref().map(|v| (k.as_str(), v))).collect();
    assert_eq!(set, [("CARGO_HOME", "/home/x/.cargo"), ("PATH", "/usr/bin"), ("TMPDIR", "/tmpdir")]);
    assert_eq!(c.get_current_dir(), Some(Path::new("/repo")));
}

#[test]
fn the_toolchain_named_is_the_one_the_suite_runs_with() {
    let root = repo_root();
    let tmp = tempfile::tempdir().unwrap();
    let env: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().filter(|(k, _)| k != "RUSTUP_TOOLCHAIN").collect();
    let plain = cut::suite_rustc(&root, tmp.path(), env.clone().into_iter()).unwrap();
    assert!(plain.starts_with("rustc "), "{plain}");
    // A toolchain set in the caller's shell (here one that is not installed, so it would fail)
    // steers neither the suite nor the version the manifest names.
    let mut steered = env.clone();
    steered.push(("RUSTUP_TOOLCHAIN".into(), "no-such-toolchain".into()));
    assert_eq!(cut::suite_rustc(&root, tmp.path(), steered.clone().into_iter()).unwrap(), plain);
    // The variable does steer rustc when it reaches it: the cleaning is what keeps it out.
    let mut raw = Command::new("rustc");
    raw.arg("--version").env_clear().envs(steered).current_dir(&root);
    assert!(!raw.output().unwrap().status.success(), "rustup refuses a toolchain that is not installed");
}

#[test]
fn a_rule_in_a_tracked_gitignore_under_a_folder_with_letters_outside_ascii_is_the_commits() {
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write("docs/design/ábaco/.gitignore", "out/\n");
    r.write("docs/design/ábaco/README.md", "x\n");
    let c = r.commit("a tracked .gitignore under an accented folder");
    r.write("docs/design/ábaco/out/f", "built\n");
    assert_eq!(cut::differs_from(r.dir.path(), &c).unwrap(), None);
}

#[test]
fn the_bundles_docs_hold_names_with_letters_outside_ascii() {
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    for f in ["README.md", "docs/veredas-plan.md", "docs/visão-schema.md", "docs/design/README.md", "docs/design/ábaco/README.md", "docs/ábaco-plan.md", "docs/other.md"] {
        r.write(f, "x\n");
    }
    let c = r.commit("docs in Portuguese");
    let paths = bundle::doc_paths(r.dir.path(), &c, &["veredas-ábaco".to_string()]).unwrap();
    assert_eq!(paths, ["README.md", "docs/design/README.md", "docs/design/ábaco/README.md", "docs/veredas-plan.md", "docs/visão-schema.md", "docs/ábaco-plan.md"]);
}

#[test]
fn round_five_the_orcl_plan_a_mode_change_ignored_folders_and_the_packers_environment() {
    // The language's crates' plan travels with them.
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    for f in ["README.md", "docs/orcl/README.md", "docs/core-plan.md"] {
        r.write(f, "x\n");
    }
    let c = r.commit("plans");
    let paths = bundle::doc_paths(r.dir.path(), &c, &["orcl-std".to_string(), "veredas-core".to_string()]).unwrap();
    assert_eq!(paths, ["README.md", "docs/core-plan.md", "docs/orcl/README.md"]);
    // A mode the commit does not hold counts, whatever `core.fileMode` says.
    let cl = clone();
    let commit = String::from_utf8(Command::new("git").arg("-C").arg(cl.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap().trim().to_string();
    assert!(Command::new("git").arg("-C").arg(cl.path()).args(["config", "core.fileMode", "false"]).status().unwrap().success());
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(cl.path().join("README.md"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let why = cut::differs_from(cl.path(), &commit).unwrap().unwrap();
    assert!(why.contains(" M README.md"), "{why}");
    std::fs::set_permissions(cl.path().join("README.md"), std::fs::Permissions::from_mode(0o644)).unwrap();
    // A folder of files the commit's own rules ignore one by one (`*.user`) is no difference.
    std::fs::create_dir_all(cl.path().join("ide")).unwrap();
    std::fs::write(cl.path().join("ide/proj.user"), "x\n").unwrap();
    assert_eq!(cut::differs_from(cl.path(), &commit).unwrap(), None);
    // tar and gzip run without the variables that change what they write.
    let tar = entrega::vm::clean_command("tar");
    let removed: Vec<String> = tar.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
    assert_eq!(removed, ["GZIP", "GZIP_OPT", "TAR_OPTIONS"]);
    // A variable that is not UTF-8 is passed on as it is, never a panic.
    use std::os::unix::ffi::OsStringExt;
    let odd = (std::ffi::OsString::from("LANG_X"), std::ffi::OsString::from_vec(b"caf\xe9".to_vec()));
    let cmd = cut::suite_command(Path::new("/repo"), Path::new("/tmpdir"), &["true"], vec![odd.clone()].into_iter());
    assert!(cmd.get_envs().any(|(k, v)| k == odd.0.as_os_str() && v == Some(odd.1.as_os_str())));
}

/// The loop's objects the run together reads (the binding's eight).
const LOOP: [&str; 8] = ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"];

/// A run of the plant as the cut reads one that is right: the page, every object of the loop live,
/// both sources live, both parcels moved between the snapshots, the write refused by the switch,
/// both executables ended with 0.
fn running() -> Report {
    let snapshot = |place: f64| {
        let objects: Vec<Value> = LOOP.iter().map(|id| json!({ "id": id, "value": 1.0, "stale": false, "place": place })).collect();
        json!({ "objects": objects, "session": { "health": { "plc": { "state": "live" }, "sorter": { "state": "live" } } } })
    };
    Report {
        page: Answer { status: 200, body: "<!doctype html><html><body></body></html>".into() },
        first: snapshot(0.1),
        second: snapshot(0.2),
        apart: Duration::from_secs(2),
        write: Answer { status: 403, body: r#"{"error":"writes are turned off for this project"}"#.into() },
        bench_exit: Some(0),
        supervisao_exit: Some(0),
        bench_out: String::new(),
        supervisao_out: String::new(),
    }
}

fn objects(snapshot: &mut Value) -> &mut Vec<Value> {
    snapshot["objects"].as_array_mut().unwrap()
}

fn set(snapshot: &mut Value, id: &str, key: &str, v: Value) {
    objects(snapshot).iter_mut().find(|o| o["id"] == id).unwrap()[key] = v;
}

fn faults_of(change: impl FnOnce(&mut Report)) -> Vec<String> {
    let mut r = running();
    change(&mut r);
    together::faults(&r)
}

#[test]
fn the_run_together_is_refused_for_every_way_the_plant_is_not_running_each_named() {
    assert_eq!(together::faults(&running()), Vec::<String>::new());
    // The page.
    assert_eq!(faults_of(|r| r.page.status = 404), ["the page answered 404 without a page"]);
    assert_eq!(faults_of(|r| r.page.body = "Not Found".into()), ["the page answered 200 without a page"]);
    // Each object of the loop: stale, without a value, absent from the feed.
    for id in LOOP {
        for (key, v) in [("stale", Value::Bool(true)), ("value", Value::Null)] {
            let mut r = running();
            set(&mut r.second, id, key, v);
            let o = together::object(&r.second, id).unwrap().clone();
            assert_eq!(together::faults(&r), [format!("object `{id}` shows no live value: {o}")], "{id} {key}");
        }
        let mut want = vec![format!("object `{id}` is not in the feed")];
        if id.starts_with("parcel-") {
            want.push(format!("`{id}` did not move along the loop in 2000 ms (Some(Number(0.1)) then None)"));
        }
        assert_eq!(faults_of(|r| objects(&mut r.second).retain(|o| o["id"] != id)), want);
    }
    // Each source: in another state, absent from the session's health.
    for source in ["plc", "sorter"] {
        assert_eq!(
            faults_of(|r| r.second["session"]["health"][source]["state"] = "stale".into()),
            [format!(r#"source `{source}` is "stale", not live"#)]
        );
        assert_eq!(
            faults_of(|r| {
                r.second["session"]["health"].as_object_mut().unwrap().remove(source);
            }),
            [format!("source `{source}` is null, not live")]
        );
    }
    // Each parcel: where it was, at no place, not in the first snapshot.
    for id in ["parcel-1", "parcel-2"] {
        let still = format!("`{id}` did not move along the loop in 2000 ms");
        assert_eq!(faults_of(|r| set(&mut r.second, id, "place", 0.1.into())), [format!("{still} (Some(Number(0.1)) then Some(Number(0.1)))")]);
        assert_eq!(faults_of(|r| set(&mut r.second, id, "place", Value::Null)), [format!("{still} (Some(Number(0.1)) then Some(Null))")]);
        assert_eq!(faults_of(|r| objects(&mut r.first).retain(|o| o["id"] != id)), [format!("{still} (None then Some(Number(0.2)))")]);
    }
    // The offered write: carried out, refused by something else than the switch, unanswered.
    assert_eq!(
        faults_of(|r| r.write = Answer { status: 200, body: "{}".into() }),
        ["the offered write was not refused by the switch: 200 {}"]
    );
    assert_eq!(
        faults_of(|r| r.write.body = r#"{"error":"forbidden"}"#.into()),
        [r#"the offered write was not refused by the switch: 403 {"error":"forbidden"}"#]
    );
    assert_eq!(
        faults_of(|r| r.write = Answer { status: 0, body: String::new() }),
        ["the offered write was not refused by the switch: 0 "]
    );
    // Each executable: ended otherwise than with 0, or not ended.
    assert_eq!(
        faults_of(|r| {
            r.bench_exit = Some(1);
            r.bench_out = "panicked".into();
        }),
        ["the bench ended with Some(1): panicked"]
    );
    assert_eq!(faults_of(|r| r.bench_exit = None), ["the bench ended with None: "]);
    assert_eq!(
        faults_of(|r| {
            r.supervisao_exit = Some(101);
            r.supervisao_out = "boom".into();
        }),
        ["supervisão ended with Some(101): boom"]
    );
    assert_eq!(faults_of(|r| r.supervisao_exit = None), ["supervisão ended with None: "]);
    // The machine's check of its plant reads no exit (its services are not stopped to be judged);
    // the cut's reads both.
    let mut r = running();
    r.bench_exit = None;
    r.supervisao_exit = None;
    assert_eq!(together::plant_faults(&r), Vec::<String>::new());
    assert_eq!(together::faults(&r), ["the bench ended with None: ", "supervisão ended with None: "]);
    // A plant that answered nothing: the page, the eight objects, the two sources, the two parcels,
    // the write and the two exits, each named.
    let silent = Report {
        page: Answer { status: 0, body: String::new() },
        first: Value::Null,
        second: Value::Null,
        apart: Duration::from_secs(2),
        write: Answer { status: 0, body: String::new() },
        bench_exit: None,
        supervisao_exit: None,
        bench_out: String::new(),
        supervisao_out: String::new(),
    };
    assert_eq!(together::faults(&silent).len(), 16, "{:#?}", together::faults(&silent));
}
