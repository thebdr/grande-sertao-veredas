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

pub fn cut(root: &Path, o: &Options) -> Result<Cut, CutError> {
    let git_text = |args: &[&str]| -> Result<String, CutError> {
        git::text(root, args).map_err(|e| refused(e.to_string()))?.ok_or_else(|| refused(format!("git {} answered no", args.join(" "))))
    };
    let commit = git_text(&["rev-parse", "HEAD"])?.trim().to_string();
    // Untracked files count: the suite runs in this tree, and a file git does not hold would test
    // a tree the commit is not.
    let dirty = git_text(&["status", "--porcelain"])?;
    if !dirty.trim().is_empty() {
        return Err(refused(format!("the tree differs from HEAD ({commit}); a release is cut from a commit:\n{dirty}")));
    }
    let releases = root.join("releases");
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
        let built = Command::new("cargo")
            .args(["nextest", "run", "--workspace", "--no-run"])
            .env("TMPDIR", &tmp)
            .current_dir(root)
            .status()
            .map_err(|source| VmError::Spawn { program: "cargo nextest --no-run".into(), source })?;
        if !built.success() {
            return Err(refused(format!("the suite does not build at {commit} ({built}): no release is cut from it")));
        }
        let status = Command::new("flock")
            .args(["/tmp/veredas-suite.lock", "cargo", "nextest", "run", "--workspace"])
            .env("TMPDIR", &tmp)
            .current_dir(root)
            .status()
            .map_err(|source| VmError::Spawn { program: "flock cargo nextest".into(), source })?;
        if !status.success() {
            return Err(refused(format!("the suite is not green at {commit} ({status}): no release is cut from it")));
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
