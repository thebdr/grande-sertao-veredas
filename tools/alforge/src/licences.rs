//! What a release carries of its licences (docs/alforge-plan.md, "What a release carries of its
//! licences"; docs/questions.md Q35, ruled 2026-11-25): a component or a package its build compiles
//! stating no licence is refused before the build, and the bundle carries `licenses/`. That folder
//! holds every component's licence files and every crate's as they ship them, the texts of Qt and
//! KDDockWidgets with where their sources are, and `THIRD-PARTY.md` listing every package with its
//! licence. All of it is read from the component list (crate::components), so the list and the
//! licences name the same packages.

use std::path::Path;

use serde_json::Value;

use crate::vm::VmError;

/// Whether a file of a package's top folder is one of its licence files, by the names packages give
/// them (`LICENSE`, `LICENSE-MIT`, `COPYING`, `NOTICE`, `UNLICENSE`, `COPYRIGHT`, with any ending).
pub fn is_licence_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE", "COPYRIGHT"].iter().any(|p| upper.starts_with(p))
}

/// The entries of a component list (`sbom`, as crate::components writes it) that state no licence,
/// by their reference: what the cut refuses before it builds.
pub fn unlicensed(sbom: &str) -> Vec<String> {
    let doc: Value = serde_json::from_str(sbom).unwrap_or_default();
    doc["components"].as_array().into_iter().flatten().filter(|c| !c["licenses"].is_array()).filter_map(|c| c["bom-ref"].as_str().map(str::to_string)).collect()
}

/// A licence as the list states it: an expression, or a package's own file.
fn licence_of(c: &Value) -> String {
    let l = &c["licenses"][0];
    l["expression"].as_str().map(str::to_string).or_else(|| l["license"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "none stated".to_string())
}

fn property<'a>(c: &'a Value, name: &str) -> Option<&'a str> {
    c["properties"].as_array().into_iter().flatten().find(|p| p["name"] == name).and_then(|p| p["value"].as_str())
}

/// Each vendored crate's licence files (`vendor/<name>-<version>/`, its top folder), copied into
/// `dest/<name>-<version>/`, before the vendored sources go.
pub fn keep_vendored(vendor: &Path, dest: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    let Ok(entries) = std::fs::read_dir(vendor) else { return Ok(()) };
    for e in entries {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(e.path())? {
            let f = f?;
            let name = f.file_name().to_string_lossy().into_owned();
            if f.file_type()?.is_file() && is_licence_file(&name) {
                let to = dest.join(e.file_name());
                std::fs::create_dir_all(&to)?;
                std::fs::copy(f.path(), to.join(&name))?;
            }
        }
    }
    Ok(())
}

/// Where the sources of what the bundle carries beside the crates lie: Qt's and KDDockWidgets'.
pub fn source_of(name: &str, version: &str) -> String {
    let minor = version.rsplit_once('.').map_or(version, |(m, _)| m);
    match name {
        "qt" => format!("https://download.qt.io/official_releases/qt/{minor}/{version}/single/qt-everywhere-src-{version}.tar.xz"),
        "kddockwidgets" => format!("https://github.com/KDAB/KDDockWidgets/releases/tag/v{version}"),
        _ => String::new(),
    }
}

/// The bundle's `licenses/` at `dest`, from the component list `sbom` of `commit` in the
/// repository at `root`: each component's and each package by path's licence files of its top
/// folder as git stores them (`components/<name>/`), each crate's from `vendored` (as
/// [`keep_vendored`] kept them, `crates/<name>-<version>/`), Qt's and KDDockWidgets' texts when the
/// list names them (`qt/`, `kddockwidgets/`: Debian's own copies of the LGPL and GPL, and
/// KDDockWidgets' own files from `kdd_doc`), and `THIRD-PARTY.md` naming every entry, its source,
/// its licence and the files kept for it.
pub fn assemble(sbom: &str, root: &Path, commit: &str, vendored: &Path, kdd_doc: Option<&Path>, licences_dir: &Path, dest: &Path) -> Result<(), VmError> {
    let io = |e: String| VmError::Io(std::io::Error::other(e));
    let doc: Value = serde_json::from_str(sbom).map_err(|e| io(format!("the component list does not read: {e}")))?;
    std::fs::create_dir_all(dest)?;
    let mut rows: Vec<String> = Vec::new();
    for c in doc["components"].as_array().into_iter().flatten() {
        let (name, version) = (c["name"].as_str().unwrap_or_default(), c["version"].as_str().unwrap_or_default());
        let mut files: Vec<String> = Vec::new();
        let source;
        if let Some(dir) = property(c, "veredas:path") {
            // A component or a package by path: its folder's own files at the commit.
            source = format!("this repository, {dir}");
            let to = format!("components/{name}");
            for f in crate::git::list(root, commit, dir).map_err(|e| io(e.to_string()))?.unwrap_or_default().into_iter().filter(|f| is_licence_file(f)) {
                if let Some(text) = crate::git::run(root, &["cat-file", "blob", &format!("{commit}:{dir}/{f}")]).map_err(|e| io(e.to_string()))? {
                    std::fs::create_dir_all(dest.join(&to))?;
                    std::fs::write(dest.join(&to).join(&f), text)?;
                    files.push(format!("{to}/{f}"));
                }
            }
        } else if c["purl"].as_str().is_some_and(|p| p.starts_with("pkg:cargo/")) {
            source = c["purl"].as_str().unwrap_or_default().to_string();
            let folder = format!("{name}-{version}");
            let from = vendored.join(&folder);
            if let Ok(entries) = std::fs::read_dir(&from) {
                let mut names: Vec<String> = entries.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
                names.sort();
                for f in names {
                    std::fs::create_dir_all(dest.join("crates").join(&folder))?;
                    std::fs::copy(from.join(&f), dest.join("crates").join(&folder).join(&f))?;
                    files.push(format!("crates/{folder}/{f}"));
                }
            }
        } else {
            // Qt or KDDockWidgets, which the bundle carries in `lib/qt/`.
            source = source_of(name, version);
            let texts: Vec<(String, std::path::PathBuf)> = match name {
                "qt" => ["LGPL-3", "GPL-2", "GPL-3"].iter().map(|t| (t.to_string(), licences_dir.join(t))).collect(),
                _ => kdd_doc.map(|d| vec![("LICENSE.txt".to_string(), d.join("LICENSE.txt")), ("GPL-2.0-only.txt".to_string(), d.join("LICENSES/GPL-2.0-only.txt")), ("GPL-3.0-only.txt".to_string(), d.join("LICENSES/GPL-3.0-only.txt"))]).unwrap_or_default(),
            };
            for (f, from) in texts {
                std::fs::create_dir_all(dest.join(name))?;
                std::fs::copy(&from, dest.join(name).join(&f)).map_err(|e| io(format!("{name}'s licence text {}: {e}", from.display())))?;
                files.push(format!("{name}/{f}"));
            }
        }
        rows.push(format!("| {name} | {version} | {} | {source} | {} |", licence_of(c), if files.is_empty() { "none shipped".to_string() } else { files.join(", ") }));
    }
    let release = doc["metadata"]["component"]["version"].as_str().unwrap_or_default();
    let text = format!(
        "# What this release carries, and under which licences\n\n\
         Release {release}, cut from commit {commit}. Every component of the suite and every package its \
         components' builds may compile, with the licence its own manifest states, where it comes \
         from, and the licence files it ships, kept beside this file. `sbom.cdx.json` at the bundle's \
         root names the same packages for scanners.\n\n\
         Oficina (`veredas-oficina`) is GPL-3.0-or-later: a bundle holding it that is handed out goes \
         with its corresponding source, this repository at the commit above and the vendored crates \
         its build compiled.\n\n\
         | Package | Version | Licence | Source | Files |\n|---|---|---|---|---|\n{}\n",
        rows.join("\n")
    );
    std::fs::write(dest.join("THIRD-PARTY.md"), text)?;
    Ok(())
}
