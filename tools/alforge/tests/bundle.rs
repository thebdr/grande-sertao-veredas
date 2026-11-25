//! C-291, the parts of the cut that need no machine: an executable's needs read from `objdump -p`
//! and held to the tester machine's libraries; `SHA256SUMS` written and every disagreement named;
//! the manifest the cut writes read back by the reader; the recipe file read and refused by key;
//! the cut refusing a tree that differs from HEAD and a release already cut; the run together
//! refused for every way the plant is not running, each named; a real cut's suite run seen through
//! stand-ins for `cargo nextest` and `flock`. The run of a real bundle is `tests/cut.rs` (feature
//! `corte`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use alforge::builder;
use alforge::bundle;
use alforge::cut::{self, CutError, Options};
use alforge::machine::{Recipe, RecipeError};
use alforge::manifest;
use alforge::together::{self, Answer, Report};
use alforge::vm::{self, VmError};
use serde_json::{Value, json};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn recipe() -> Recipe {
    Recipe::load_file(&repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap()
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
    assert_eq!(
        (m.release.to_string(), m.commit.as_str(), m.target.as_str(), m.toolchain.as_str()),
        ("2.0.0-alpha.1".to_string(), commit, "x86_64-unknown-linux-gnu", "rustc 1.100.0 (0ef1bc386 2026-11-09)")
    );
    let got: Vec<(String, String, Vec<String>)> = m.components.iter().map(|c| (c.name.clone(), c.version.to_string(), c.bins.clone())).collect();
    assert_eq!(got, components);
    assert_eq!((m.tested.suite.as_str(), m.tested.together.as_str()), ("cargo nextest run --workspace", "the bundle's bench and supervisão on its plant"));
    let trial = manifest::load(&cut::manifest_text(&release, commit, "x86_64-unknown-linux-gnu", "rustc 1", &components, cut::NOT_RUN)).unwrap();
    assert_eq!(trial.tested.suite, "not run: a trial cut");
}

#[test]
fn the_recipe_is_read_through_cores_header_and_refused_by_key() {
    let text = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
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
        work: c.path().join("target/alforge").join(release),
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
    let text = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
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
        work: c.path().join("target/alforge").join(release),
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
            .args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"])
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
    r.write(".gitignore", "/target/\n");
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("tools/alforge/maquina/maquina.toml", &recipe_with("cpus = 4", "cpus = 4"));
    let c1 = r.commit("core 2.0.0");
    let components = vec![("veredas-core".to_string(), "2.0.0".to_string(), vec![])];
    let first = semver::Version::parse("2.0.0").unwrap();
    r.write("releases/2.0.0.toml", &cut::manifest_text(&first, &c1, "x86_64-unknown-linux-gnu", "rustc 1", &components, cut::SUITE));
    r.commit("release 2.0.0");
    r.write("crates/veredas-core/src/lib.rs", "pub fn changed() {}\n");
    r.commit("core changed, its version not bumped");
    // A real cut (the suite is to run, through the stand-ins): the one-tree check comes first, so
    // nothing runs or builds; a cut that let it through would stop at once, its suite "not building".
    let cache = tempfile::tempdir().unwrap();
    let (got, log) = cut_with_suite(&r, cache.path(), 1, "exit 1", &[]);
    assert_eq!(got.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&got.stderr),
        "release 2.0.1 would not check:\nveredas-core 2.0.0 is pinned by releases 2.0.0 and 2.0.1 at different build inputs (crates/veredas-core/): a version names everything its build reads\n"
    );
    assert_eq!(log, Vec::<String>::new(), "refused before the suite");
    assert!(!r.dir.path().join("target/alforge/2.0.1").exists(), "refused before anything was run or built");
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
        work: c.path().join("target/alforge/9.9.9-trial.1"),
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
    let tar = alforge::vm::clean_command("tar");
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

/// A real cut (`alforge cut 2.0.1`) of the repository `r`, its cache `cache`, seen through
/// stand-ins for `cargo nextest` and `flock` first on its `PATH` (every other cargo command goes to
/// the real one): the stand-in's build exits `build_exit`, its suite runs `run`. The caller's shell
/// holds `NEXTEST_PROFILE`, `CARGO_TARGET_DIR` and a `RUSTUP_TOOLCHAIN` naming no toolchain, which
/// the cut, reading its toolchain as the suite runs, never meets. Hands back the cut's output and the stand-ins'
/// log, a line a call: its arguments, and what the suite's environment held.
fn cut_with_suite(r: &Repo, cache: &Path, build_exit: u8, run: &str, args: &[&str]) -> (std::process::Output, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    let stub = tempfile::tempdir().unwrap();
    let log = stub.path().join("log");
    let cargo = format!(
        "#!/bin/sh\nif [ \"$1\" = nextest ]; then\n  printf 'cargo %s | TMPDIR=%s NEXTEST_PROFILE=%s CARGO_TARGET_DIR=%s\\n' \"$*\" \"$TMPDIR\" \"${{NEXTEST_PROFILE-unset}}\" \"${{CARGO_TARGET_DIR-unset}}\" >> '{log}'\n  case \" $* \" in *' --no-run '*) exit {build_exit};; esac\n  {run}\nfi\nexec '{real}' \"$@\"\n",
        log = log.display(),
        real = env!("CARGO"),
    );
    let flock = format!("#!/bin/sh\nprintf 'flock %s\\n' \"$*\" >> '{}'\nshift\nexec \"$@\"\n", log.display());
    for (name, text) in [("cargo", cargo), ("flock", flock)] {
        let p = stub.path().join(name);
        std::fs::write(&p, text).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::join_paths(std::iter::once(stub.path().to_path_buf()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()))).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_alforge"))
        .args(["cut", "2.0.1"])
        .args(args)
        .current_dir(r.dir.path())
        .env("PATH", path)
        .env("ALFORGE_CACHE", cache)
        .env("NEXTEST_PROFILE", "no-browser")
        .env("CARGO_TARGET_DIR", stub.path().join("elsewhere"))
        .env("RUSTUP_TOOLCHAIN", "no-such-toolchain")
        .output()
        .unwrap();
    let lines = std::fs::read_to_string(&log).unwrap_or_default().lines().map(String::from).collect();
    (out, lines)
}

#[test]
fn a_real_cuts_suite_is_the_whole_default_profile_behind_the_lock_and_the_tree_judged_after_it() {
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write(".gitignore", "/target/\n");
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    // The recipe's base at an address nothing answers: a cut whose suite passes stops at the
    // builder's first download, so nothing is built here.
    let recipe = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    assert_eq!(recipe.lines().filter(|l| l.starts_with("url = ")).count(), 1);
    let base = "https://127.0.0.1:9/alforge-stub.qcow2";
    let recipe: String = recipe.lines().map(|l| if l.starts_with("url = ") { format!("url = \"{base}\"\n") } else { format!("{l}\n") }).collect();
    r.write("tools/alforge/maquina/maquina.toml", &recipe);
    let commit = r.commit("core 2.0.0");
    let cache = tempfile::tempdir().unwrap();
    let work = r.dir.path().join("target/alforge/2.0.1");
    let manifest = r.dir.path().join("releases/2.0.1.toml");
    // The suite the manifest names is the one run: built first, then run behind the machine's lock,
    // the default profile both times, none of the shell's steering, its temporary files in the cache.
    let env = format!(" | TMPDIR={} NEXTEST_PROFILE=unset CARGO_TARGET_DIR=unset", cache.path().join("tmp").display());
    let built = format!("{} --profile default --no-run{env}", cut::SUITE);
    let locked = format!("flock /tmp/veredas-suite.lock {} --profile default", cut::SUITE);
    let ran = format!("{} --profile default{env}", cut::SUITE);
    let stderr = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).into_owned();
    let start = |at: &str| format!("alforge: the whole suite at {at}, behind the machine's lock\n");

    // A suite that does not build: refused, the lock never taken, nothing built, no manifest.
    let (out, log) = cut_with_suite(&r, cache.path(), 1, "exit 0", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), format!("{}the suite does not build at {commit} (exit status: 1): no release is cut from it\n", start(&commit)));
    assert_eq!(log, std::slice::from_ref(&built));
    assert!(!work.exists() && !manifest.exists());

    // A red suite.
    let (out, log) = cut_with_suite(&r, cache.path(), 0, "exit 1", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), format!("{}the suite is not green at {commit} (exit status: 1): no release is cut from it\n", start(&commit)));
    assert_eq!(log, [built.clone(), locked.clone(), ran.clone()]);
    assert!(!work.exists() && !manifest.exists());

    // A green suite while a file was added: the tree it tested is not the commit's.
    let (out, log) = cut_with_suite(&r, cache.path(), 0, "echo x > stray; exit 0", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr(&out),
        format!("{}while the suite ran, the tree differs from HEAD ({commit}); a release is cut from a commit:\n?? stray\n\n", start(&commit))
    );
    assert_eq!(log, [built.clone(), locked.clone(), ran.clone()]);
    assert!(!work.exists() && !manifest.exists());
    std::fs::remove_file(r.dir.path().join("stray")).unwrap();

    // A green suite while a commit was made.
    let (out, log) = cut_with_suite(&r, cache.path(), 0, "git -c user.name=e -c user.email=e@example.invalid -c commit.gpgsign=false commit -q --allow-empty -m moved; exit 0", &[]);
    let moved = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(moved, commit);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), format!("{}while the suite ran, HEAD is {moved}, not {commit}: a release is cut from one commit\n", start(&commit)));
    assert_eq!(log, [built.clone(), locked.clone(), ran.clone()]);
    assert!(!work.exists() && !manifest.exists());

    // A green suite, the tree untouched: the cut goes on to the builder (whose first download, from
    // an address nothing answers, fails here), still without a manifest.
    let (out, log) = cut_with_suite(&r, cache.path(), 0, "exit 0", &[]);
    assert_eq!(log, [built, locked, ran]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr(&out);
    assert!(text.starts_with(&start(&moved)), "{text}");
    assert!(text.contains(&format!("alforge: downloading {base} once into {}\n", cache.path().join("alforge-stub.qcow2").display())), "{text}");
    assert!(work.exists() && !manifest.exists());
}

/// A dynamic executable compiled here with the C compiler, standing in for one the builder made:
/// it needs the host's C library at the versions the host's compiler links against.
fn stand_in(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let src = tempfile::tempdir().unwrap();
    let c = src.path().join("main.c");
    std::fs::write(&c, "int main(void) { return 0; }\n").unwrap();
    let exe = dir.join(name);
    let out = Command::new("cc").arg("-o").arg(&exe).arg(&c).output().unwrap();
    assert!(out.status.success(), "cc: {}", String::from_utf8_lossy(&out.stderr));
    exe
}

#[test]
fn after_the_build_the_cut_stops_on_another_toolchain_an_executable_too_new_and_a_bundle_off_its_sums() {
    let c = clone();
    let commit = String::from_utf8(Command::new("git").arg("-C").arg(c.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap().trim().to_string();
    let built_dir = tempfile::tempdir().unwrap();
    let bin = built_dir.path().join("bin");
    stand_in(&bin, "primeiro");
    stand_in(&bin, "segundo");
    let built = |toolchain: &str| builder::Built { out: built_dir.path().to_path_buf(), toolchain: toolchain.into(), libc: "ldd (Debian GLIBC 2.41-12) 2.41".into() };
    let bins = vec!["primeiro".to_string(), "segundo".to_string()];
    // The component that builds them, its version naming its folder and one file outside it.
    let inputs = alforge::inputs::Inputs([("crates/veredas-core/", "1"), ("docs/shared.rs", "2")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
    // And a component that ships no executable, held to its inputs by its library's build.
    let lib_inputs = alforge::inputs::Inputs([("crates/veredas-aranha/", "3")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
    let held: std::collections::BTreeMap<String, alforge::manifest::Held> = [
        (
            "veredas-aranha".to_string(),
            alforge::manifest::Held {
                dir: "crates/veredas-aranha".into(),
                name: "veredas-aranha".into(),
                version: "2.0.0".into(),
                bins: Default::default(),
                optional: Default::default(),
                lib: Some("veredas_aranha".into()),
                inputs: Ok(lib_inputs),
            },
        ),
        (
            "veredas-core".to_string(),
            alforge::manifest::Held {
                dir: "crates/veredas-core".into(),
                name: "veredas-core".into(),
                version: "2.0.0".into(),
                bins: bins.iter().cloned().collect(),
                optional: Default::default(),
                lib: Some("veredas_core".into()),
                inputs: Ok(inputs),
            },
        ),
    ]
    .into_iter()
    .collect();
    // Cargo's dep-info for each, as the builder hands it back: what each compilation read.
    let deps = built_dir.path().join("deps");
    std::fs::create_dir_all(&deps).unwrap();
    let dep_info = |extra: &str, lib_extra: &str| {
        for b in &bins {
            let more = if b == "segundo" { extra } else { "" };
            std::fs::write(
                deps.join(format!("{b}.d")),
                format!("/build/src/target/release/{b}: /build/src/crates/veredas-core/src/main.rs /build/src/docs/shared.rs /build/src/target/release/build/x/out/gen.rs{more}\n"),
            )
            .unwrap();
        }
        std::fs::write(deps.join("libveredas_aranha.d"), format!("/build/src/target/release/libveredas_aranha.rlib: /build/src/crates/veredas-aranha/src/lib.rs{lib_extra}\n")).unwrap();
    };
    let name = "veredas-9.9.9-x86_64-unknown-linux-gnu";
    let rustc = "rustc 1.100.0 (0ef1bc386 2026-11-09)";
    let text = "# the manifest's text\n";
    let work = tempfile::tempdir().unwrap();
    let packed = work.path().join(format!("{name}.tar.gz"));

    // The builder ran another toolchain than the suite: refused before anything is assembled.
    dep_info("", "");
    match cut::after_build(&recipe(), c.path(), &commit, rustc, &built("rustc 1.99.0 (5a1b2c3d4 2026-10-01)"), name, &held, text, work.path()) {
        Err(CutError::Refused(e)) => assert_eq!(e, format!("the builder ran `rustc 1.99.0 (5a1b2c3d4 2026-10-01)`, the suite was tested with `{rustc}`")),
        other => panic!("not refused: {other:?}"),
    }
    assert!(!work.path().join(name).exists() && !packed.exists());

    // A compilation that read a file of the tree its version does not name, and one outside the
    // tree: each refused by name before anything is assembled.
    dep_info(" /build/src/docs/fora.rs /etc/fora.conf", " /build/src/crates/veredas-core/src/schema.rs");
    match cut::after_build(&recipe(), c.path(), &commit, rustc, &built(rustc), name, &held, text, work.path()) {
        Err(CutError::Refused(e)) => assert_eq!(
            e,
            "the library of veredas-aranha read crates/veredas-core/src/schema.rs, which its version does not name\n\
             `segundo` of veredas-core read /etc/fora.conf, which its version does not name\n`segundo` of veredas-core read docs/fora.rs, which its version does not name"
        ),
        other => panic!("not refused: {other:?}"),
    }
    assert!(!work.path().join(name).exists() && !packed.exists());
    // No dep-info for one, an executable's or a library's: refused, naming where it was looked for.
    for (file, what) in [("primeiro.d", "`primeiro` of veredas-core"), ("libveredas_aranha.d", "the library of veredas-aranha")] {
        dep_info("", "");
        std::fs::remove_file(deps.join(file)).unwrap();
        match cut::after_build(&recipe(), c.path(), &commit, rustc, &built(rustc), name, &held, text, work.path()) {
            Err(CutError::Refused(e)) => assert!(e.starts_with(&format!("the build left no dep-info for {what} ({}: ", deps.join(file).display())), "{e}"),
            other => panic!("not refused: {other:?}"),
        }
        assert!(!work.path().join(name).exists() && !packed.exists());
    }
    dep_info("", "");

    // Executables needing more of the C library than the tester machine offers: each refused by
    // name, nothing packed.
    let glibc = bundle::needs(&vm::run("objdump", &["-p", &bin.join("primeiro").display().to_string()], None).unwrap())["GLIBC"].clone();
    let mut old = recipe();
    old.base.glibc = "2.2".into();
    match cut::after_build(&old, c.path(), &commit, rustc, &built(rustc), name, &held, text, work.path()) {
        Err(CutError::Vm(VmError::Failed { program, status, stderr })) => {
            assert_eq!((program.as_str(), status.as_str()), ("the executables' check", "refused"));
            assert_eq!(
                stderr,
                format!(
                    "primeiro needs GLIBC_{glibc}, the tester machine offers GLIBC_2.2: it would not start there\nsegundo needs GLIBC_{glibc}, the tester machine offers GLIBC_2.2: it would not start there"
                )
            );
        }
        other => panic!("not refused: {other:?}"),
    }
    assert!(!packed.exists());

    // Right: packed, extracted again into `check`, matching its sums, the executables as built.
    let (p, extracted) = cut::after_build(&recipe(), c.path(), &commit, rustc, &built(rustc), name, &held, text, work.path()).unwrap();
    assert_eq!((p, extracted.clone()), (packed.clone(), work.path().join("check").join(name)));
    let epoch = String::from_utf8(Command::new("git").arg("-C").arg(c.path()).args(["show", "-s", "--format=%ct", &commit]).output().unwrap().stdout).unwrap();
    let entries = listing(&packed);
    assert!(entries.len() > 10, "{entries:?}");
    let want_date = utc(epoch.trim().parse().unwrap());
    for e in &entries {
        assert_eq!((e.owner.as_str(), &e.date), ("0/0", &want_date), "{}", e.name);
    }
    assert_in_name_order(&entries);
    assert_eq!(bundle::verify_sums(&extracted).unwrap(), Vec::<String>::new());
    for b in &bins {
        assert_eq!(std::fs::read(extracted.join("bin").join(b)).unwrap(), std::fs::read(bin.join(b)).unwrap());
    }
    assert_eq!(std::fs::read_to_string(extracted.join("release.toml")).unwrap(), text);
    assert!(extracted.join("planta/bench.toml").is_file());

    // The archive changed after it was packed (a file changed, one added, one taken out): its
    // extraction is refused, each file named.
    let again = tempfile::tempdir().unwrap();
    vm::run("tar", &["-xzf", &packed.display().to_string(), "-C", &again.path().display().to_string()], None).unwrap();
    let inner = again.path().join(name);
    std::fs::write(inner.join("release.toml"), "# another text\n").unwrap();
    std::fs::write(inner.join("extra.txt"), "not packed by the cut\n").unwrap();
    std::fs::remove_file(inner.join("bin/segundo")).unwrap();
    let tampered = again.path().join("tampered.tar.gz");
    bundle::pack(again.path(), name, "1", &tampered).unwrap();
    match cut::unpack_checked(&tampered, &work.path().join("check"), name) {
        Err(CutError::Refused(e)) => assert_eq!(
            e,
            "the packed bundle does not match its sums:\nextra.txt is not in SHA256SUMS\nrelease.toml does not match its sum\nbin/segundo is in SHA256SUMS and not in the bundle"
        ),
        other => panic!("not refused: {other:?}"),
    }
    assert!(!work.path().join("check").join(name).join("bin/primeiro").exists() || work.path().join("check").join(name).join("extra.txt").exists(), "the check folder holds the tampered archive alone");
}

#[test]
fn the_builder_builds_locked_offline_on_its_processors_and_refuses_a_bin_cargo_did_not_build() {
    let r = recipe();
    assert_eq!(r.builder.cpus, 4);
    // Each component built alone, so cargo unifies features within its own inputs.
    // A component that ships no executable has its library built alone, for its dep-info.
    let build = |p: &str, bins: &[&str], lib: &str| builder::Build { package: p.into(), bins: bins.iter().map(|b| b.to_string()).collect(), lib: Some(lib.into()), optional: vec![] };
    let mut portal = build("veredas-portal", &["veredas-portal"], "veredas_portal");
    portal.optional = vec!["porteiro".into()];
    let builds = vec![build("veredas-aranha", &[], "veredas_aranha"), build("veredas-maquinar", &["veredas-bancada"], "veredas_maquinar"), portal];
    let script = builder::script(&r, &builds);
    assert_eq!(
        script,
        "set -eu\nexport RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\
         cd /build/src\nmkdir -p /build/out/bin /build/out/deps\nrustc --version > /build/out/toolchain\nldd --version | head -1 > /build/out/libc\n\
         cargo build --release --locked --offline --jobs 4 -p veredas-aranha --lib\n\
         cargo build --release --locked --offline --jobs 4 -p veredas-maquinar --bins\n\
         cargo build --release --locked --offline --jobs 4 -p veredas-portal --bins\nmissing=\n\
         cp target/release/libveredas_aranha.d /build/out/deps/\n\
         if [ -f target/release/veredas-bancada ]; then cp target/release/veredas-bancada /build/out/bin/; cp target/release/veredas-bancada.d /build/out/deps/; else echo \"ALFORGE-MISSING veredas-bancada\"; missing=1; fi\n\
         if [ -f target/release/veredas-portal ]; then cp target/release/veredas-portal /build/out/bin/; cp target/release/veredas-portal.d /build/out/deps/; else echo \"ALFORGE-MISSING veredas-portal\"; missing=1; fi\n\
         if [ -f target/release/porteiro ]; then cp target/release/porteiro /build/out/bin/; cp target/release/porteiro.d /build/out/deps/; fi\n\
         [ -z \"$missing\" ]\necho ALFORGE-BUILD-OK\n"
    );
    // The script run here as the builder runs it (its `/build` a folder of the test's, stand-ins
    // for `cargo` and `rustc` first on its PATH), once with every bin built and once with one not.
    use std::os::unix::fs::PermissionsExt;
    // A third time cargo also builds `porteiro`, which has required features: copied out with its
    // dep-info, shipped as cargo made it (docs/questions.md Q443).
    for built in [vec!["veredas-bancada", "veredas-portal"], vec!["veredas-bancada"], vec!["veredas-bancada", "veredas-portal", "porteiro"]] {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("build");
        let stub = d.path().join("stub");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::create_dir_all(&stub).unwrap();
        let touch: String = built.iter().map(|b| format!(" target/release/{b} target/release/{b}.d")).collect::<String>() + " target/release/libveredas_aranha.d";
        for (name, text) in [
            ("cargo", format!("#!/bin/sh\necho \"cargo $*\" >> '{}'\nmkdir -p target/release\ntouch{touch}\n", d.path().join("calls").display())),
            ("rustc", "#!/bin/sh\necho 'rustc 1.100.0 (0ef1bc386 2026-11-09)'\n".to_string()),
        ] {
            std::fs::write(stub.join(name), text).unwrap();
            std::fs::set_permissions(stub.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let here = script.replace("/build", &root.display().to_string()).replace("PATH=/opt/cargo/bin:", &format!("PATH={}:", stub.display()));
        let log_path = root.join("out/build.log");
        let status = Command::new("sh").arg("-c").arg(format!("sh -c \"$1\" > '{}' 2>&1", log_path.display())).arg("sh").arg(&here).status().unwrap();
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.path().join("calls")).unwrap(),
            "cargo build --release --locked --offline --jobs 4 -p veredas-aranha --lib\ncargo build --release --locked --offline --jobs 4 -p veredas-maquinar --bins\ncargo build --release --locked --offline --jobs 4 -p veredas-portal --bins\n"
        );
        assert_eq!(std::fs::read_to_string(root.join("out/toolchain")).unwrap(), "rustc 1.100.0 (0ef1bc386 2026-11-09)\n");
        let mut shipped: Vec<String> = std::fs::read_dir(root.join("out/bin")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        shipped.sort();
        let mut built = built.clone();
        built.sort();
        assert_eq!(shipped, built);
        let mut infos: Vec<String> = std::fs::read_dir(root.join("out/deps")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        infos.sort();
        let mut want: Vec<String> = built.iter().map(|b| format!("{b}.d")).chain(["libveredas_aranha.d".to_string()]).collect();
        want.sort();
        assert_eq!(infos, want, "each executable's dep-info beside it, and the library's of the component that ships none");
        let console = format!("ALFORGE-BUILD-EXIT {}", status.code().unwrap());
        let judged = builder::outcome(&log, &console, true, &log_path);
        if built.contains(&"veredas-portal") {
            assert!(status.success() && log.ends_with("ALFORGE-BUILD-OK\n"), "{log}");
            assert!(judged.is_ok(), "{judged:?}");
        } else {
            assert_eq!(status.code(), Some(1), "{log}");
            assert_eq!(log, "ALFORGE-MISSING veredas-portal\n");
            match judged {
                Err(VmError::Failed { program, status, stderr }) => assert_eq!(
                    (program.as_str(), status.as_str(), stderr.as_str()),
                    (
                        "the build in the builder machine",
                        "refused",
                        "cargo built no `veredas-portal` with the default features: the release would not ship it"
                    )
                ),
                other => panic!("not refused: {other:?}"),
            }
        }
    }
    // A build that did not finish: no OK in its log, another exit on the console, its disk unread.
    let log_path = Path::new("/build/out/build.log");
    for (log, console, unpacked) in [("cargo: error\n", "ALFORGE-BUILD-EXIT 0", true), ("ALFORGE-BUILD-OK\n", "ALFORGE-BUILD-EXIT 101", true), ("ALFORGE-BUILD-OK\n", "ALFORGE-BUILD-EXIT 0", false)] {
        match builder::outcome(log, console, unpacked, log_path) {
            Err(VmError::Failed { program, status, .. }) => {
                assert_eq!((program.as_str(), status.as_str()), ("the build in the builder machine", "no ALFORGE-BUILD-OK (its log: /build/out/build.log)"))
            }
            other => panic!("not refused: {other:?}"),
        }
    }
}

#[test]
fn a_machine_starts_with_12_gb_available_and_the_engines_lock_is_the_one_flock_takes() {
    assert_eq!(vm::AVAILABLE_BEFORE_A_MACHINE, 12 * 1024 * 1024 * 1024);
    assert_eq!(vm::memory_available_in("MemTotal:       65000000 kB\nMemFree:         9000000 kB\nMemAvailable:   12582912 kB\n"), Some(12 << 30));
    assert_eq!(vm::memory_available_in("MemTotal:       65000000 kB\n"), None);
    // Under the rule a machine is refused once the wait is over, the two amounts named.
    let now = vm::available_memory().unwrap();
    assert!(vm::wait_for_memory(now / 2, std::time::Duration::ZERO).is_ok());
    match vm::wait_for_memory(u64::MAX, std::time::Duration::ZERO) {
        Err(VmError::Failed { program, status, stderr }) => {
            assert_eq!((program.as_str(), status.as_str()), ("the machine's start", "refused"));
            assert!(stderr.starts_with("the host has ") && stderr.ends_with(&format!(" MB available, a machine waits for {} MB", u64::MAX >> 20)), "{stderr}");
        }
        other => panic!("not refused: {other:?}"),
    }
    // The engine's lock is the file `flock` locks, held exclusively until it is dropped.
    assert_eq!(vm::ENGINE_LOCK, "/tmp/veredas-engine.lock");
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("engine.lock");
    let free = || Command::new("flock").arg("-n").arg(&f).arg("true").status().unwrap().success();
    let held = vm::lock(&f).unwrap();
    assert!(!free(), "flock took the lock while alforge held it");
    drop(held);
    assert!(free(), "the lock outlived its holder");
}

/// A throwaway repository a trial cut can read up to its build: one component, `veredas-core`
/// 2.0.0, its root manifest `workspace`, and the recipe `recipe` (none when `None`) committed.
fn repo_for_cut(workspace: &str, recipe: Option<&str>) -> (Repo, String) {
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write(".gitignore", "/target/\n");
    r.write("Cargo.toml", workspace);
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    if let Some(text) = recipe {
        r.write("tools/alforge/maquina/maquina.toml", text);
    }
    let commit = r.commit("core 2.0.0");
    (r, commit)
}

/// The repository's recipe with its base at an address nothing answers (a cut that got as far as
/// its build would stop at the first download) and `from` replaced by `to`.
fn recipe_with(from: &str, to: &str) -> String {
    let text = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    assert_eq!(text.matches(from).count(), 1, "{from}");
    text.lines()
        .map(|l| if l.starts_with("url = ") { "url = \"https://127.0.0.1:9/alforge-stub.qcow2\"\n".to_string() } else { format!("{l}\n") })
        .collect::<String>()
        .replace(from, to)
}

/// A trial cut of release 9.9.9 of the repository at `root`: its outcome, and whether it left
/// anything (its work folder, a manifest where it writes).
fn trial_cut(root: &Path) -> (Result<cut::Cut, CutError>, bool) {
    let out = tempfile::tempdir().unwrap();
    let work = root.join("target/alforge/9.9.9");
    let o = Options { release: semver::Version::parse("9.9.9").unwrap(), out: out.path().to_path_buf(), work: work.clone(), run_suite: false };
    let got = cut::cut(root, &o);
    let left = work.exists() || out.path().join("9.9.9.toml").exists();
    (got, left)
}

#[test]
fn every_refusal_before_the_build_stops_the_cut_with_nothing_built() {
    let workspace = "[workspace]\nmembers = [\"crates/veredas-core\"]\n";
    let message = |got: Result<cut::Cut, CutError>| match got {
        Err(e) => e.to_string(),
        Ok(c) => panic!("cut: {:?}", c.manifest),
    };

    // The commit holds no recipe.
    let (r, c) = repo_for_cut(workspace, None);
    let (got, left) = trial_cut(r.dir.path());
    assert_eq!(message(got), format!("commit {c} holds no tools/alforge/maquina/maquina.toml"));
    assert!(!left);

    // The commit's recipe refused by core's header reader, by a misspelled key and by a value.
    for (from, to, want) in [
        ("\nversion = 1\n", "\nversion = 2\n", None),
        ("glibc = \"2.41\"", "glibcc = \"2.41\"", Some("a machine recipe: unknown field `glibcc`")),
        ("glibc = \"2.41\"", "glibc = \"v2.41\"", Some("a machine recipe: `base.glibc` is `v2.41`, not a version of dotted digits")),
    ] {
        let (r, _) = repo_for_cut(workspace, Some(&recipe_with(from, to)));
        let (got, left) = trial_cut(r.dir.path());
        match (got, want) {
            (Err(CutError::Recipe(RecipeError::Header(e))), None) => assert!(e.to_string().contains("version"), "{e}"),
            (Err(CutError::Recipe(RecipeError::Invalid(e))), Some(w)) => assert!(format!("a machine recipe: {e}").starts_with(w), "{e}"),
            (other, _) => panic!("{to}: {:?}", other.map(|c| c.manifest)),
        }
        assert!(!left, "{to}");
    }

    // The commit's components refused (its root manifest no workspace).
    let (r, c) = repo_for_cut("[package]\nname = \"veredas-raiz\"\nversion = \"2.0.0\"\nedition = \"2024\"\n", Some(&recipe_with("cpus = 4", "cpus = 4")));
    let (got, left) = trial_cut(r.dir.path());
    assert_eq!(message(got), format!("commit {c}: its Cargo.toml is no workspace (`[workspace]`): cargo would look for one above the tree"));
    assert!(!left);

    // A component whose build inputs do not read (it depends on a crate and the commit holds no
    // lock file to pin it): what its version names cannot be held to its build.
    let (r, _) = repo_for_cut(workspace, Some(&recipe_with("cpus = 4", "cpus = 4")));
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = \"1\"\n");
    r.commit("core uses serde, nothing locked");
    let (got, left) = trial_cut(r.dir.path());
    assert_eq!(message(got), "veredas-core's build inputs do not read: veredas-core depends on `serde` and the commit holds no Cargo.lock to pin it");
    assert!(!left);

    // The manifest it would write refused against the repository (a shallow clone's history is
    // cut), before anything runs.
    let (r, _) = repo_for_cut(workspace, Some(&recipe_with("cpus = 4", "cpus = 4")));
    let shallow = tempfile::tempdir().unwrap();
    let out = Command::new("git").args(["clone", "-q", "--depth", "1"]).arg(format!("file://{}", r.dir.path().display())).arg(shallow.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (got, left) = trial_cut(shallow.path());
    assert_eq!(message(got), "release 9.9.9 would not check:\nthe repository is a shallow clone: its history is cut, and a release is checked against the whole of it");
    assert!(!left);
}

/// One entry of an archive's listing (`tar --numeric-owner --utc --full-time -tv`).
#[derive(Debug)]
struct Entry {
    mode: String,
    owner: String,
    date: (String, String),
    name: String,
}

/// The entries of a gzipped archive as GNU tar lists them, its owners by number and its dates in
/// UTC, whatever the shell's `TAR_OPTIONS`.
fn listing(archive: &Path) -> Vec<Entry> {
    let out = Command::new("tar").args(["--numeric-owner", "--utc", "--full-time", "--quoting-style=literal", "-tvzf"]).arg(archive).env_remove("TAR_OPTIONS").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            // The mode, the owner, the size, the date, the time, the name (none holds a space).
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 6, "{l}");
            Entry { mode: f[0].into(), owner: f[1].into(), date: (f[3].into(), f[4].into()), name: f[5].into() }
        })
        .collect()
}

/// `seconds` since the epoch as tar's `--utc --full-time` prints them: the date, the time.
fn utc(seconds: i64) -> (String, String) {
    let out = Command::new("date").args(["-u", "-d"]).arg(format!("@{seconds}")).arg("+%Y-%m-%d %H:%M:%S").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let (d, t) = text.trim().split_once(' ').unwrap();
    (d.into(), t.into())
}

/// The entries depth first, every folder's children in the byte order of their names (what
/// `--sort=name` gives): each entry's path, read part by part, after the one before it.
fn assert_in_name_order(entries: &[Entry]) {
    let parts = |e: &Entry| e.name.trim_end_matches('/').split('/').map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>();
    for w in entries.windows(2) {
        assert!(parts(&w[0]) < parts(&w[1]), "{} before {}", w[0].name, w[1].name);
    }
}

#[test]
fn a_bundles_entries_are_in_name_order_owned_by_root_by_number_and_dated_at_its_epoch() {
    // Files made out of name order (twenty in a shuffled order, then `b` before `a`), so the order
    // the folder hands them back in is not their names'.
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("veredas-y");
    std::fs::create_dir_all(dir.join("many")).unwrap();
    for i in 0..20 {
        std::fs::write(dir.join("many").join(format!("f{:02}", (i * 7) % 20)), format!("{i}\n")).unwrap();
    }
    std::fs::write(dir.join("b"), "b\n").unwrap();
    std::fs::write(dir.join("a"), "a\n").unwrap();
    let out = work.path().join("y.tar.gz");
    bundle::pack(work.path(), "veredas-y", "1795300000", &out).unwrap();
    let entries = listing(&out);
    let mut want = vec!["veredas-y/".to_string(), "veredas-y/a".into(), "veredas-y/b".into(), "veredas-y/many/".into()];
    want.extend((0..20).map(|n| format!("veredas-y/many/f{n:02}")));
    assert_eq!(entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), want);
    assert_in_name_order(&entries);
    for e in &entries {
        assert_eq!((e.owner.as_str(), &e.date.0, &e.date.1), ("0/0", &"2026-11-21".to_string(), &"22:26:40".to_string()), "{}", e.name);
        assert!(e.mode == "drwxr-xr-x" || e.mode == "-rw-r--r--", "{e:?}");
    }
}

#[test]
fn an_executable_name_two_components_build_is_refused_before_anything_runs() {
    // Two components each building `ferramenta` (cargo builds one file of that name, warning only),
    // and a third building `outra`.
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write(".gitignore", "/target/\n");
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-a\", \"crates/veredas-b\", \"crates/veredas-c\"]\nresolver = \"3\"\n");
    for (c, bins) in [("veredas-a", ["ferramenta", "a"]), ("veredas-b", ["ferramenta", "outra"]), ("veredas-c", ["outra", "c"])] {
        let targets: String = bins.iter().map(|b| format!("\n[[bin]]\nname = \"{b}\"\npath = \"src/{b}.rs\"\n")).collect();
        r.write(&format!("crates/{c}/Cargo.toml"), &format!("[package]\nname = \"{c}\"\nversion = \"2.0.0\"\nedition = \"2024\"\n{targets}"));
        for b in bins {
            r.write(&format!("crates/{c}/src/{b}.rs"), "fn main() {}\n");
        }
    }
    r.write("tools/alforge/maquina/maquina.toml", &recipe_with("cpus = 4", "cpus = 4"));
    r.commit("two executables named alike");
    let (got, left) = trial_cut(r.dir.path());
    match got {
        Err(CutError::Refused(e)) => assert_eq!(
            e,
            "`ferramenta` is an executable of veredas-a and veredas-b: a bundle holds one executable a name\n\
             `outra` is an executable of veredas-b and veredas-c: a bundle holds one executable a name"
        ),
        other => panic!("not refused: {:?}", other.map(|c| c.manifest)),
    }
    assert!(!left, "refused before anything ran or was built");
}

#[test]
fn a_trials_manifest_goes_beside_its_bundle() {
    // `alforge cut <release> --trial` with no `--out`: where its manifest goes is where the cut
    // looks for one already there, the release's own work folder (and where `alforge maquina`
    // looks for it).
    let (r, _) = repo_for_cut("[workspace]\nmembers = [\"crates/veredas-core\"]\n", Some(&recipe_with("cpus = 4", "cpus = 4")));
    let beside = r.dir.path().join("target/alforge/9.9.9/9.9.9.toml");
    std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
    std::fs::write(&beside, "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).args(["cut", "9.9.9", "--trial"]).current_dir(r.dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stderr), format!("{} exists: release 9.9.9 is cut already\n", beside.display()));
}

#[test]
fn a_release_already_cut_where_the_cut_writes_takes_part_in_its_check() {
    // Release 2.0.0, cut into a folder of its own (`--out`) and not committed, pins veredas-core
    // 2.0.0 at its commit; HEAD changes core's code without its version. The cut runs with the
    // stand-ins, so a cut that let this through would stop at once, its suite "not building".
    let r = Repo { dir: tempfile::tempdir().unwrap() };
    r.git(&["init", "-q", "-b", "main"]);
    r.write(".gitignore", "/target/\n");
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-core\"]\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("tools/alforge/maquina/maquina.toml", &recipe_with("cpus = 4", "cpus = 4"));
    let c1 = r.commit("core 2.0.0");
    let out = tempfile::tempdir().unwrap();
    let components = vec![("veredas-core".to_string(), "2.0.0".to_string(), vec![])];
    let first = semver::Version::parse("2.0.0").unwrap();
    std::fs::write(out.path().join("2.0.0.toml"), cut::manifest_text(&first, &c1, "x86_64-unknown-linux-gnu", "rustc 1", &components, cut::SUITE)).unwrap();
    r.write("crates/veredas-core/src/lib.rs", "pub fn changed() {}\n");
    r.commit("core changed, its version not bumped");
    let cache = tempfile::tempdir().unwrap();
    let (got, log) = cut_with_suite(&r, cache.path(), 1, "exit 1", &["--out", out.path().to_str().unwrap()]);
    assert_eq!(got.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&got.stderr),
        "release 2.0.1 would not check:\nveredas-core 2.0.0 is pinned by releases 2.0.0 and 2.0.1 at different build inputs (crates/veredas-core/): a version names everything its build reads\n"
    );
    assert_eq!(log, Vec::<String>::new(), "refused before the suite");
    assert!(!r.dir.path().join("target/alforge/2.0.1").exists() && !out.path().join("2.0.1.toml").exists());
}

/// The name of the test below, which runs its own binary again for it.
const PACK_UNDER_THE_SHELL: &str = "a_bundle_packs_to_the_same_bytes_whatever_the_shells_tar_and_gzip_variables_say";

#[test]
fn a_bundle_packs_to_the_same_bytes_whatever_the_shells_tar_and_gzip_variables_say() {
    // The child: this test run again with the shell's `TAR_OPTIONS`, `GZIP` and `GZIP_OPT` set; it
    // packs the folder it is given and leaves.
    if let Some(dir) = std::env::var_os("ALFORGE_PACK_UNDER_THE_SHELL") {
        let dir = PathBuf::from(dir);
        bundle::pack(&dir, "veredas-z", "1795300000", &dir.join("child.tar.gz")).unwrap();
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("veredas-z");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin/tool"), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.join("README.md"), "z\n".repeat(4096)).unwrap();
    let parent = work.path().join("parent.tar.gz");
    bundle::pack(work.path(), "veredas-z", "1795300000", &parent).unwrap();
    let shell = [("TAR_OPTIONS", "--format=posix"), ("GZIP", "-1"), ("GZIP_OPT", "-1")];
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", PACK_UNDER_THE_SHELL])
        .env("ALFORGE_PACK_UNDER_THE_SHELL", work.path())
        .envs(shell)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let child = work.path().join("child.tar.gz");
    assert_eq!(std::fs::read(&child).unwrap(), std::fs::read(&parent).unwrap(), "the shell's variables reached the packing");
    // Where they do reach tar and gzip, the same files pack to other bytes.
    let reached = work.path().join("reached.tar.gz");
    let status = Command::new("tar")
        .args(["--sort=name", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+rX,u+w,go-w,ug-s", "--mtime=@1795300000", "--use-compress-program=gzip -n", "-cf"])
        .arg(&reached)
        .arg("-C")
        .arg(work.path())
        .arg("veredas-z")
        .envs(shell)
        .status()
        .unwrap();
    assert!(status.success());
    assert_ne!(std::fs::read(&reached).unwrap(), std::fs::read(&parent).unwrap());
}

#[test]
fn a_dep_info_is_read_as_make_reads_it_and_held_to_what_a_version_names() {
    let inputs = alforge::inputs::Inputs(
        [("crates/veredas-portal/", "1"), ("crates/veredas-core/", "2"), ("docs/shared.rs", "3"), ("Cargo.toml", "4")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    );
    // Cargo's own form: the rule's target, then what it read; a space in a name escaped, a `$`
    // doubled, a line continued; a rule with no prerequisites; the build's own output and the
    // vendored sources left out.
    let info = "/build/src/target/release/veredas-portal: /build/src/crates/veredas-portal/src/main.rs /build/src/crates/veredas-core/src/lib.rs \\\n \
                /build/src/docs/a\\ b.rs /build/src/docs/shared.rs /build/src/docs/cost$$.txt /build/src/target/release/build/x-1/out/gen.rs \
                /build/vendor/serde-1.0.200/src/lib.rs /etc/fora.conf /build/src/crates/veredas-portalx/src/lib.rs \
                /build/src/crates/veredas-portal/src/../../../docs/unix.rs /build/src/crates/veredas-core/../../docs/shared.rs \
                /build/src/crates/../../etc/climbed.rs /build/src/crates/veredas-portal/../../target/release/build/y/out/z.rs\n\n\
                /build/src/crates/veredas-portal/src/main.rs:\n";
    // A path that climbs back (as rustc writes a `#[path]` module's) is judged where it lands: in a
    // named file, in another folder, out of the tree, in the build's own output.
    assert_eq!(
        alforge::inputs::read_outside(info, builder::SRC, builder::VENDOR, &inputs),
        ["/build/src/crates/../../etc/climbed.rs", "/etc/fora.conf", "crates/veredas-portalx/src/lib.rs", "docs/a b.rs", "docs/cost$.txt", "docs/unix.rs"],
        "a folder's name is no prefix of another's"
    );
    assert_eq!((builder::SRC, builder::VENDOR), ("/build/src", "/build/vendor"));
    assert_eq!(alforge::inputs::read_outside("/build/src/target/release/x: /build/src/crates/veredas-core/src/lib.rs\n", builder::SRC, builder::VENDOR, &inputs), Vec::<String>::new());
}

#[test]
fn a_cached_builder_disk_follows_its_base_where_the_cache_now_holds_it() {
    // A base and the builder's overlay on it, then the cache moved (as the member's rename moved
    // it): the overlay names the old place until it is reused.
    let d = tempfile::tempdir().unwrap();
    let old = d.path().join("entrega");
    std::fs::create_dir_all(&old).unwrap();
    let base = old.join("base.qcow2");
    let disk = old.join("builder.qcow2");
    vm::run("qemu-img", &["create", "-q", "-f", "qcow2", &base.display().to_string(), "64M"], None).unwrap();
    vm::overlay(&base, &disk, 1).unwrap();
    let moved = d.path().join("alforge");
    std::fs::rename(&old, &moved).unwrap();
    let backing = |p: &Path| -> String {
        let info: serde_json::Value = serde_json::from_str(&vm::run("qemu-img", &["info", "--output=json", &p.display().to_string()], None).unwrap()).unwrap();
        info["backing-filename"].as_str().unwrap_or_default().to_string()
    };
    let disk = moved.join("builder.qcow2");
    assert_eq!(backing(&disk), base.display().to_string());
    builder::reuse(&disk, &moved.join("base.qcow2")).unwrap();
    assert_eq!(backing(&disk), moved.join("base.qcow2").display().to_string());
    vm::run("qemu-img", &["check", "-q", &disk.display().to_string()], None).unwrap();
    // Reused again where it already names its base: nothing to rewrite.
    let before = std::fs::read(&disk).unwrap();
    builder::reuse(&disk, &moved.join("base.qcow2")).unwrap();
    assert_eq!(std::fs::read(&disk).unwrap(), before);
}
