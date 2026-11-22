//! The bundle (docs/entrega-plan.md, "What a release is made of"): `bin/` (the executables built in
//! the builder), `planta/` (the tester machine's plant), the suite's README and `docs/` (its plan,
//! every file reference, the shipped components' plans, the design references), `release.toml`
//! (the manifest) and `SHA256SUMS` over every other file; packed as
//! `veredas-<release>-<target>.tar.gz`, its entries in name order, owned by root, dated at the
//! commit, so one commit packs to the same bytes. Every file but the executables comes from git's
//! objects at the commit, never the working tree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cache;
use crate::git;
use crate::machine::Recipe;
use crate::vm::{self, VmError};

/// The newest version of each library the executable asks for, by `objdump -p`'s "required from"
/// lines: (`GLIBC`, `2.34`), (`GLIBCXX`, `3.4.29`), (`CXXABI`, `1.3.15`).
pub fn needs(objdump: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for word in objdump.split_whitespace() {
        let Some((lib, ver)) = word.split_once('_') else { continue };
        if !matches!(lib, "GLIBC" | "GLIBCXX" | "CXXABI") || !ver.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let newer = out.get(lib).is_none_or(|cur| newer_than(ver, cur));
        if newer {
            out.insert(lib.to_string(), ver.to_string());
        }
    }
    out
}

fn parts(v: &str) -> Vec<u32> {
    v.split('.').filter_map(|p| p.parse().ok()).collect()
}

/// Whether dotted version `a` is newer than `b`.
pub fn newer_than(a: &str, b: &str) -> bool {
    parts(a) > parts(b)
}

/// Refusals for an executable whose needs pass what the guest's libraries offer.
pub fn check_needs(bin: &str, needs: &BTreeMap<String, String>, r: &Recipe) -> Vec<String> {
    let limits = [("GLIBC", &r.base.glibc), ("GLIBCXX", &r.base.glibcxx), ("CXXABI", &r.base.cxxabi)];
    let mut out = Vec::new();
    for (lib, limit) in limits {
        if let Some(v) = needs.get(lib)
            && newer_than(v, limit)
        {
            out.push(format!("{bin} needs {lib}_{v}, the tester machine offers {lib}_{limit}: it would not start there"));
        }
    }
    out
}

/// The files of the repository at `commit` the bundle's docs are: the README, the suite's plan,
/// every `docs/*-schema.md`, the plans of `components` (where one exists) and `docs/design/`.
pub fn doc_paths(root: &Path, commit: &str, components: &[String]) -> Result<Vec<String>, VmError> {
    let all = git::text(root, &["ls-tree", "-r", "--name-only", commit])
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .unwrap_or_default();
    let plans: Vec<String> = components.iter().filter_map(|c| c.strip_prefix("veredas-")).map(|n| format!("docs/{n}-plan.md")).collect();
    Ok(all
        .lines()
        .filter(|p| {
            *p == "README.md"
                || *p == "docs/veredas-plan.md"
                || (p.starts_with("docs/") && p.ends_with("-schema.md") && !p["docs/".len()..].contains('/'))
                || plans.iter().any(|x| x == p)
                || p.starts_with("docs/design/")
        })
        .map(String::from)
        .collect())
}

/// Every file under `dir`, as paths relative to it with `/`, in name order.
pub fn files(dir: &Path) -> Result<Vec<String>, std::io::Error> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) -> Result<(), std::io::Error> {
        for e in std::fs::read_dir(d)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                walk(base, &p, out)?;
            } else {
                out.push(p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// `SHA256SUMS` over every file of `dir` but itself, in `sha256sum`'s own form.
pub fn write_sums(dir: &Path) -> Result<(), std::io::Error> {
    let mut text = String::new();
    for f in files(dir)? {
        if f == "SHA256SUMS" {
            continue;
        }
        text.push_str(&format!("{}  {f}\n", cache::sha256(&dir.join(&f))?));
    }
    std::fs::write(dir.join("SHA256SUMS"), text)
}

/// Refusals for a bundle folder whose files and `SHA256SUMS` disagree: a file changed, missing
/// or not listed.
pub fn verify_sums(dir: &Path) -> Result<Vec<String>, std::io::Error> {
    let text = std::fs::read_to_string(dir.join("SHA256SUMS"))?;
    let mut listed = BTreeMap::new();
    for line in text.lines() {
        if let Some((h, f)) = line.split_once("  ") {
            listed.insert(f.to_string(), h.to_string());
        }
    }
    let mut out = Vec::new();
    let present = files(dir)?;
    for f in &present {
        if f == "SHA256SUMS" {
            continue;
        }
        match listed.get(f) {
            None => out.push(format!("{f} is not in SHA256SUMS")),
            Some(h) if *h != cache::sha256(&dir.join(f))? => out.push(format!("{f} does not match its sum")),
            Some(_) => {}
        }
    }
    for f in listed.keys() {
        if !present.contains(f) {
            out.push(format!("{f} is in SHA256SUMS and not in the bundle"));
        }
    }
    Ok(out)
}

/// Assembles the bundle folder `work/<name>` and packs it beside as `<name>.tar.gz`; returns the
/// archive's path. Refused, by name, when an executable would not start on the tester machine.
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    r: &Recipe,
    root: &Path,
    commit: &str,
    name: &str,
    built_bins: &Path,
    bins: &[String],
    components: &[String],
    manifest_text: &str,
    work: &Path,
) -> Result<PathBuf, VmError> {
    let dir = work.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin"))?;
    let mut refusals = Vec::new();
    for b in bins {
        let from = built_bins.join(b);
        let to = dir.join("bin").join(b);
        std::fs::copy(&from, &to)?;
        let dump = vm::run("objdump", &["-p", &to.display().to_string()], None)?;
        refusals.extend(check_needs(b, &needs(&dump), r));
    }
    if !refusals.is_empty() {
        return Err(VmError::Failed { program: "the executables' check".into(), status: "refused".into(), stderr: refusals.join("\n") });
    }
    // The docs and the plant as git stores them at the commit, no attribute or filter applied.
    let mut paths = doc_paths(root, commit, components)?;
    paths.push("tools/entrega/planta".into());
    let p: Vec<&str> = paths.iter().map(String::as_str).collect();
    crate::git::export_tree(root, commit, &p, &dir).map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::rename(dir.join("tools/entrega/planta"), dir.join("planta"))?;
    std::fs::remove_dir_all(dir.join("tools"))?;
    std::fs::write(dir.join("release.toml"), manifest_text)?;
    write_sums(&dir)?;
    let epoch = git::text(root, &["show", "-s", "--format=%ct", commit])
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .ok_or_else(|| VmError::Io(std::io::Error::other(format!("git cannot show {commit}"))))?;
    let out = work.join(format!("{name}.tar.gz"));
    pack(work, name, epoch.trim(), &out)?;
    Ok(out)
}

/// Packs `work/<name>` into `out` so that the same files give the same bytes whatever made them:
/// entries in name order, owned by root, dated at `epoch` (seconds), every mode read as `0644`
/// or, for a folder or an executable, `0755` (the umask that made the files plays no part).
pub fn pack(work: &Path, name: &str, epoch: &str, out: &Path) -> Result<(), VmError> {
    vm::run(
        "tar",
        &[
            "--sort=name",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "--mode=a+rX,u+w,go-w,ug-s",
            &format!("--mtime=@{epoch}"),
            "--use-compress-program=gzip -n",
            "-cf",
            &out.display().to_string(),
            "-C",
            &work.display().to_string(),
            name,
        ],
        None,
    )?;
    Ok(())
}
