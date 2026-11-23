//! The cut (docs/entrega-plan.md, "How a release is cut"): from HEAD of a clean tree, the whole
//! suite run, the executables built in the builder machine, the bundle assembled and packed, then
//! extracted again, its sums checked and its bench and supervisão run together on its plant; only
//! then is the manifest written, and checked against the repository like any other.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use crate::builder;
use crate::bundle;
use crate::git;
use crate::machine::{Recipe, RecipeError};
use crate::manifest::{self, CheckError, Manifest};
use crate::together::{self, Report};
use crate::vm::{self, VmError};

pub const TOGETHER: &str = "the bundle's bench and supervisão on its plant";
pub const SUITE: &str = "cargo nextest run --workspace";
/// What a trial cut states it did not run: a cut whose manifest is never committed.
pub const NOT_RUN: &str = "not run: a trial cut";

#[derive(Debug, Error)]
pub enum CutError {
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Vm(#[from] VmError),
    #[error(transparent)]
    Recipe(#[from] RecipeError),
    #[error(transparent)]
    Check(#[from] CheckError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Options {
    pub release: semver::Version,
    /// Where the manifest is written (`releases/` of the repository for a real cut).
    pub out: PathBuf,
    /// Where the build, the bundle and the run's copies are made.
    pub work: PathBuf,
    /// Run the whole suite at the commit first (a real cut); a trial cut states it did not.
    pub run_suite: bool,
}

pub struct Cut {
    pub manifest: Manifest,
    pub manifest_path: PathBuf,
    pub bundle: PathBuf,
    /// The bundle extracted, as the run used it.
    pub extracted: PathBuf,
    pub report: Report,
}

fn refused(what: impl Into<String>) -> CutError {
    CutError::Refused(what.into())
}

/// The manifest's text for a cut, keys in the reference's order.
pub fn manifest_text(release: &semver::Version, commit: &str, target: &str, toolchain: &str, components: &[(String, String, Vec<String>)], suite: &str) -> String {
    let mut t = format!(
        "# A suite release (docs/entrega-plan.md), written by `entrega cut`.\nformat = \"veredas-release\"\nversion = 1\nrelease = \"{release}\"\ncommit = \"{commit}\"\ntarget = \"{target}\"\ntoolchain = \"{toolchain}\"\n"
    );
    for (name, version, bins) in components {
        t.push_str(&format!("\n[[component]]\nname = \"{name}\"\nversion = \"{version}\"\n"));
        if !bins.is_empty() {
            let list: Vec<String> = bins.iter().map(|b| format!("\"{b}\"")).collect();
            t.push_str(&format!("bins = [{}]\n", list.join(", ")));
        }
    }
    t.push_str(&format!("\n[tested]\nsuite = \"{suite}\"\ntogether = \"{TOGETHER}\"\n"));
    t
}

/// The suite's command as the cut runs it: `argv` in `root`, its temporary files in `tmp`, and every
/// variable of `env` that would steer what cargo builds or nextest runs removed (`NEXTEST_*`,
/// `CARGO_*` but `CARGO_HOME`, `RUSTFLAGS`, `RUSTDOCFLAGS`, `RUSTC*`, `RUSTUP_TOOLCHAIN`): the
/// whole suite of the default profile, whatever the caller's shell holds.
pub fn suite_command(root: &Path, tmp: &Path, argv: &[&str], env: impl Iterator<Item = (String, String)>) -> Command {
    let mut c = Command::new(argv[0]);
    c.args(&argv[1..]).current_dir(root).env("TMPDIR", tmp);
    for (k, _) in env {
        let steers = k.starts_with("NEXTEST_")
            || (k.starts_with("CARGO_") && k != "CARGO_HOME")
            || k == "RUSTFLAGS"
            || k == "RUSTDOCFLAGS"
            || k.starts_with("RUSTC")
            || k == "RUSTUP_TOOLCHAIN";
        if steers {
            c.env_remove(k);
        }
    }
    c
}

/// Why the working tree at `root` is not exactly `commit`, or `None`: HEAD moved, a tracked file
/// changed, an untracked file stands (whatever `status.showUntrackedFiles` says), a file is ignored
/// by a rule the commit does not hold, or a file is marked so git does not look at it
/// (`--assume-unchanged`, `--skip-worktree`).
pub fn differs_from(root: &Path, commit: &str) -> Result<Option<String>, git::GitError> {
    let head = git::text(root, &["rev-parse", "HEAD"])?.map(|t| t.trim().to_string());
    if head.as_deref() != Some(commit) {
        return Ok(Some(format!("HEAD is {}, not {commit}: a release is cut from one commit", head.unwrap_or_default())));
    }
    let dirty = git::text(root, &["status", "--porcelain", "--untracked-files=all", "--ignore-submodules=none"])?.unwrap_or_default();
    if !dirty.trim().is_empty() {
        return Ok(Some(format!("the tree differs from HEAD ({commit}); a release is cut from a commit:\n{dirty}")));
    }
    // An ignored file the commit does not ignore (by `.git/info/exclude`, a global excludes file or
    // a `.gitignore` git does not track) is a file of this tree the commit is not.
    let ignored = git::text(root, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"])?.unwrap_or_default();
    let mut stray = Vec::new();
    for path in ignored.split('\0').filter(|p| !p.is_empty()) {
        let rule = git::text(root, &["check-ignore", "-v", "--no-index", path])?.unwrap_or_default();
        let source = rule.split(':').next().unwrap_or_default().to_string();
        let tracked = source.ends_with(".gitignore") && git::run(root, &["ls-files", "--error-unmatch", &source])?.is_some();
        if !tracked {
            stray.push(format!("{path} (ignored by {})", if source.is_empty() { "no rule git names".to_string() } else { source }));
        }
    }
    if !stray.is_empty() {
        return Ok(Some(format!("files are ignored by rules the commit does not hold, so the tree is not HEAD's:\n{}", stray.join("\n"))));
    }
    let flagged: Vec<String> = git::text(root, &["ls-files", "-v"])?
        .unwrap_or_default()
        .lines()
        .filter(|l| l.chars().next().is_some_and(|t| t.is_ascii_lowercase() || t == 'S'))
        .map(String::from)
        .collect();
    if !flagged.is_empty() {
        return Ok(Some(format!("files are marked for git not to look at them, so the tree is not known to be HEAD's:\n{}", flagged.join("\n"))));
    }
    Ok(None)
}

pub fn cut(root: &Path, o: &Options) -> Result<Cut, CutError> {
    let git_text = |args: &[&str]| -> Result<String, CutError> {
        git::text(root, args).map_err(|e| refused(e.to_string()))?.ok_or_else(|| refused(format!("git {} answered no", args.join(" "))))
    };
    let commit = git_text(&["rev-parse", "HEAD"])?.trim().to_string();
    if let Some(why) = differs_from(root, &commit).map_err(|e| refused(e.to_string()))? {
        return Err(refused(why));
    }
    let releases = root.join("releases");
    if !o.run_suite && o.out.canonicalize().ok().is_some_and(|out| releases.canonicalize().ok() == Some(out)) {
        return Err(refused(format!("a trial's manifest is never committed: it goes elsewhere than {}", releases.display())));
    }
    let target_file = o.out.join(format!("{}.toml", o.release));
    for f in [target_file.clone(), releases.join(format!("{}.toml", o.release))] {
        if f.exists() {
            return Err(refused(format!("{} exists: release {} is cut already", f.display(), o.release)));
        }
    }
    let recipe_text = git::show(root, &commit, "tools/entrega/maquina/maquina.toml")
        .map_err(|e| refused(e.to_string()))?
        .ok_or_else(|| refused(format!("commit {commit} holds no tools/entrega/maquina/maquina.toml")))?;
    let recipe = Recipe::load(&recipe_text)?;
    let mut refusals = Vec::new();
    let held = manifest::components_at(root, &commit, &mut refusals)?;
    if !refusals.is_empty() {
        return Err(refused(refusals.join("\n")));
    }
    let rustc = vm::run("rustc", &["--version"], None)?.trim().to_string();
    let toolchain = rustc.split_whitespace().nth(1).ok_or_else(|| refused(format!("rustc printed `{rustc}`")))?.to_string();
    let target = "x86_64-unknown-linux-gnu";
    let components: Vec<(String, String, Vec<String>)> =
        held.values().map(|h| (h.name.clone(), h.version.clone(), h.bins.iter().cloned().collect())).collect();
    let text = manifest_text(&o.release, &commit, target, &rustc, &components, if o.run_suite { SUITE } else { NOT_RUN });
    let manifest = manifest::load(&text).map_err(|e| refused(format!("the manifest to be written does not read back: {e}")))?;
    // Checked as any manifest is, against the repository and, for a real cut, against every
    // release already cut (here and in `--out`), before anything is built: a version names one
    // tree. A trial's manifest is never committed, so it is checked against the repository alone.
    let mut against = manifest::check(root, &manifest)?;
    if o.run_suite {
        let mut all = manifest::load_dir(&releases).0;
        if o.out != releases {
            all.extend(manifest::load_dir(&o.out).0);
        }
        all.push((target_file.clone(), manifest.clone()));
        against.extend(manifest::check_together(root, &all)?);
    }
    if !against.is_empty() {
        return Err(refused(format!("release {} would not check:\n{}", o.release, against.join("\n"))));
    }
    if o.run_suite {
        eprintln!("entrega: the whole suite at {commit}, behind the machine's lock");
        // Its temporary files on disk, outside any repository: /tmp is a tmpfs of the machine's
        // memory, and the laws' tests need a folder no git repository holds.
        let tmp = crate::cache::dir()?.join("tmp");
        std::fs::create_dir_all(&tmp)?;
        // Built first, so the machine's lock is held for the tests alone (CLAUDE.md).
        let built = suite_command(root, &tmp, &["cargo", "nextest", "run", "--workspace", "--profile", "default", "--no-run"], std::env::vars())
            .status()
            .map_err(|source| VmError::Spawn { program: "cargo nextest --no-run".into(), source })?;
        if !built.success() {
            return Err(refused(format!("the suite does not build at {commit} ({built}): no release is cut from it")));
        }
        let status = suite_command(root, &tmp, &["flock", "/tmp/veredas-suite.lock", "cargo", "nextest", "run", "--workspace", "--profile", "default"], std::env::vars())
            .status()
            .map_err(|source| VmError::Spawn { program: "flock cargo nextest".into(), source })?;
        if !status.success() {
            return Err(refused(format!("the suite is not green at {commit} ({status}): no release is cut from it")));
        }
        // The suite tested the tree as it stood while it ran: it must still be the commit.
        if let Some(why) = differs_from(root, &commit).map_err(|e| refused(e.to_string()))? {
            return Err(refused(format!("while the suite ran, {why}")));
        }
    }
    let packages: Vec<String> = held.values().filter(|h| !h.bins.is_empty()).map(|h| h.name.clone()).collect();
    let bins: Vec<String> = held.values().flat_map(|h| h.bins.iter().cloned()).collect();
    std::fs::create_dir_all(&o.work)?;
    let built = builder::build(&recipe, root, &commit, &toolchain, &packages, &bins, &o.work)?;
    eprintln!("entrega: built under {} on {}", built.toolchain, built.libc);
    if built.toolchain != rustc {
        return Err(refused(format!("the builder ran `{}`, the suite was tested with `{rustc}`", built.toolchain)));
    }
    let name = format!("veredas-{}-{target}", o.release);
    let names: Vec<String> = held.keys().cloned().collect();
    let packed = bundle::assemble(&recipe, root, &commit, &name, &built.out.join("bin"), &bins, &names, &text, &o.work)?;
    let check = o.work.join("check");
    let _ = std::fs::remove_dir_all(&check);
    std::fs::create_dir_all(&check)?;
    vm::run("tar", &["-xzf", &packed.display().to_string(), "-C", &check.display().to_string()], None)?;
    let extracted = check.join(&name);
    let wrong = bundle::verify_sums(&extracted)?;
    if !wrong.is_empty() {
        return Err(refused(format!("the packed bundle does not match its sums:\n{}", wrong.join("\n"))));
    }
    eprintln!("entrega: running the bundle's bench and supervisão together");
    let report = together::run(&extracted, &o.work.join("together"))?;
    let faults = together::faults(&report);
    if !faults.is_empty() {
        return Err(refused(format!("the bundle did not run together:\n{}", faults.join("\n"))));
    }
    std::fs::create_dir_all(&o.out)?;
    std::fs::write(&target_file, &text)?;
    Ok(Cut { manifest, manifest_path: target_file, bundle: packed, extracted, report })
}
