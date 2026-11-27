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

/// Whether a file is one of a package's licence files or notices, by the names packages give them
/// (`LICENSE`, `LICENSE-MIT`, `COPYING`, `NOTICE`, `UNLICENSE`, `COPYRIGHT`, `THIRD-PARTY-NOTICES.md`,
/// `3RDPARTY.md`, `ATTRIBUTION.md`, `ACKNOWLEDGMENTS`, `AUTHORS`, which a licence names as its
/// copyright holders, a font's `OFL.txt`, `OFL-Inter.txt` or `UFL.txt`, REUSE's `REUSE.toml`, with
/// any ending but a Rust source's `.rs`; any name holding `LICENSE`, `LICENCE`, `COPYING`, `NOTICE`
/// or `ATTRIBUTION`, as a font's `OFL-LICENSE.txt`).
pub fn is_licence_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    // A source of Rust (`licenses.rs`) is code, never a licence's text.
    if upper.ends_with(".RS") {
        return false;
    }
    // A font licence's own word, alone or before a `.`, `-` or `_` (`OFL.txt`, `OFL-Inter.txt`).
    let font = ["OFL", "UFL"].iter().any(|p| upper.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '-', '_'])));
    font || upper == "REUSE.TOML"
        || ["COPYRIGHT", "THIRD-PARTY", "THIRD_PARTY", "THIRDPARTY", "3RDPARTY", "ACKNOWLEDG", "AUTHORS"].iter().any(|p| upper.starts_with(p))
        || ["LICENSE", "LICENCE", "COPYING", "NOTICE", "ATTRIBUTION"].iter().any(|p| upper.contains(p))
}

/// Whether a path inside a package (`src/x/LICENSE-MIT`, `LICENSES/MIT.txt`) is one of its licence
/// files: a licence file's name at any depth, any file of a folder named `LICENSES` or `LICENCES`,
/// or REUSE's copyright file `.reuse/dep5` (Debian's machine-readable copyright, half's only
/// notice of its holder).
pub fn is_licence_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.last().is_some_and(|f| is_licence_file(f))
        || parts[..parts.len().saturating_sub(1)].iter().any(|d| ["LICENSES", "LICENCES"].contains(&d.to_ascii_uppercase().as_str()))
        || parts.ends_with(&[".reuse", "dep5"])
}

/// Every file under `dir` (paths from it, `/` apart, in name order).
fn files_under(dir: &Path) -> std::io::Result<Vec<String>> {
    fn inner(base: &Path, rel: &str, out: &mut Vec<String>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(base.join(rel))?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if e.file_type()?.is_dir() {
                inner(base, &path, out)?;
            } else if e.file_type()?.is_file() {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.is_dir() {
        inner(dir, "", &mut out)?;
    }
    Ok(out)
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

/// The file an entry's licence is, as its manifest's `license-file` names it (crate::components
/// writes it from the repository's root for a package by path, from the crate's folder for a crate).
fn licence_file(c: &Value) -> Option<&str> {
    c["licenses"][0]["license"]["name"].as_str()?.strip_prefix("the package's own file ")
}

/// A relative path with its `.` and `..` parts read away (`crates/x/../../LICENSE` is `LICENSE`):
/// `None` for an absolute one or one that climbs above where it starts.
fn within(path: &str) -> Option<String> {
    if path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The entries by path of a component list (`sbom`) whose `license-file` names a file the commit
/// `commit` of the repository at `root` does not hold as a file (or one outside it), each said: what
/// the cut refuses before it builds, as it refuses an entry stating no licence.
pub fn missing_texts(sbom: &str, root: &Path, commit: &str) -> Result<Vec<String>, crate::git::GitError> {
    let doc: Value = serde_json::from_str(sbom).unwrap_or_default();
    let mut out = Vec::new();
    for c in doc["components"].as_array().into_iter().flatten() {
        let (Some(_), Some(file)) = (property(c, "veredas:path"), licence_file(c)) else { continue };
        let r = c["bom-ref"].as_str().unwrap_or_default();
        match within(file) {
            None => out.push(format!("{r} names its licence file {file}, outside the repository: a release carries the text its licence is")),
            Some(p) => {
                if crate::git::run(root, &["cat-file", "blob", &format!("{commit}:{p}")])?.is_none() {
                    out.push(format!("{r} names its licence file {p}, which commit {commit} does not hold: a release carries the text its licence is"));
                }
            }
        }
    }
    Ok(out)
}

/// Each vendored crate's licence files (`vendor/<name>-<version>/`, at any depth, a `LICENSES/`
/// folder whole, and the file its manifest's `license-file` names, whatever its name), copied into
/// `dest/<name>-<version>/` at their own paths, before the vendored sources go. A `license-file`
/// outside the crate's folder is the file of its name at the crate's top, where `cargo package`
/// put it.
pub fn keep_vendored(vendor: &Path, dest: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    let Ok(entries) = std::fs::read_dir(vendor) else { return Ok(()) };
    for e in entries {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        let mut kept: Vec<String> = files_under(&e.path())?.into_iter().filter(|f| is_licence_path(f)).collect();
        let stated = std::fs::read_to_string(e.path().join("Cargo.toml")).ok().and_then(|t| t.parse::<toml::Table>().ok()).and_then(|m| m.get("package").and_then(|p| p.get("license-file")).and_then(|f| f.as_str()).map(str::to_string));
        if let Some(f) = stated {
            let named = within(&f).filter(|p| e.path().join(p).is_file());
            let at_top = Path::new(&f).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| e.path().join(n).is_file());
            if let Some(p) = named.or(at_top).filter(|p| !kept.contains(p)) {
                kept.push(p);
            }
        }
        for f in kept {
            let to = dest.join(e.file_name()).join(&f);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(e.path().join(&f), to)?;
        }
    }
    Ok(())
}

/// Qt's own SPDX documents (`<module>-<version>.spdx.json` of the install's `sbom/`) of the modules
/// whose files the bundle's `lib/qt/` (`staged`) holds, matched by file name (a library by its
/// name up to `.so`, a plugin or a QML module's file by its path), in name order.
pub fn qt_documents(sbom_dir: &Path, staged: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let key = |p: &str| -> String {
        let p = p.trim_start_matches("./");
        match p.strip_prefix("lib/") {
            Some(name) => format!("lib/{}", name.split(".so").next().unwrap_or(name)),
            None => p.to_string(),
        }
    };
    let held: std::collections::BTreeSet<String> = files_under(staged)?.iter().map(|f| key(f)).collect();
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(sbom_dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.to_string_lossy().ends_with(".spdx.json")).collect();
    names.sort();
    for doc in names {
        let text = std::fs::read_to_string(&doc)?;
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        if v["files"].as_array().into_iter().flatten().filter_map(|f| f["fileName"].as_str()).any(|f| held.contains(&key(f))) {
            out.push(doc);
        }
    }
    Ok(out)
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
/// list names them (`qt/`: Debian's own copies of the LGPL and GPL, and Qt's own SPDX documents of
/// the modules `qt_docs` names, which name the third-party code inside them and its licences,
/// docs/questions.md Q450; `kddockwidgets/`: its `LICENSE.txt`, every file of its `LICENSES/` and
/// its `3RDPARTY.md` from `kdd_doc`), and `THIRD-PARTY.md` naming every entry, its source, its
/// licence and the files kept for it. A component's or a package by path's licence files are
/// taken at any depth of its folder, a `LICENSES/` folder whole.
#[allow(clippy::too_many_arguments)]
pub fn assemble(sbom: &str, root: &Path, commit: &str, vendored: &Path, kdd_doc: Option<&Path>, licences_dir: &Path, qt_docs: &[std::path::PathBuf], dest: &Path) -> Result<(), VmError> {
    let io = |e: String| VmError::Io(std::io::Error::other(e));
    let doc: Value = serde_json::from_str(sbom).map_err(|e| io(format!("the component list does not read: {e}")))?;
    std::fs::create_dir_all(dest)?;
    let mut rows: Vec<String> = Vec::new();
    for c in doc["components"].as_array().into_iter().flatten() {
        let (name, version) = (c["name"].as_str().unwrap_or_default(), c["version"].as_str().unwrap_or_default());
        let mut files: Vec<String> = Vec::new();
        let source;
        if let Some(dir) = property(c, "veredas:path") {
            // A component or a package by path: its folder's own files at the commit, a package's
            // nested in it (a folder holding a manifest with `[package]`) left to that package.
            source = if dir.is_empty() { "this repository, its root".to_string() } else { format!("this repository, {dir}") };
            let to = format!("components/{name}");
            let blob = |path: &str| crate::git::run(root, &["cat-file", "blob", &format!("{commit}:{path}")]).map_err(|e| io(e.to_string()));
            let from_root = |f: &str| if dir.is_empty() { f.to_string() } else { format!("{dir}/{f}") };
            // Its files alone (`<mode> blob <id>\t<path>`): a submodule's entry is no file of it.
            let listing = crate::git::run(root, &["ls-tree", "-r", "-z", &format!("{commit}:{dir}")]).map_err(|e| io(e.to_string()))?.unwrap_or_default();
            let all: Vec<String> = listing
                .split(|b| *b == 0)
                .filter_map(|e| {
                    let e = String::from_utf8_lossy(e).into_owned();
                    let (head, path) = e.split_once('\t')?;
                    (head.split(' ').nth(1) == Some("blob")).then(|| path.to_string())
                })
                .collect();
            let mut nested: Vec<String> = Vec::new();
            for m in all.iter().filter_map(|p| p.strip_suffix("/Cargo.toml")) {
                let text = blob(&from_root(&format!("{m}/Cargo.toml")))?.unwrap_or_default();
                let manifest = String::from_utf8_lossy(&text).parse::<toml::Table>().unwrap_or_default();
                if manifest.contains_key("package") || manifest.contains_key("project") {
                    nested.push(format!("{m}/"));
                }
            }
            let mut paths: Vec<String> = all.into_iter().filter(|p| is_licence_path(p) && !nested.iter().any(|n| p.starts_with(n.as_str()))).collect();
            // The file its `license-file` names, wherever it lies in the repository: at its own
            // path inside the folder, by its name at the folder's top outside it (as `cargo
            // package` puts it), under `license-file/` when a file of the folder has that name.
            let mut stated: Option<(String, String)> = None;
            if let Some(file) = licence_file(c) {
                let p = within(file).ok_or_else(|| io(format!("{name} names its licence file {file}, outside the repository")))?;
                let inside = if dir.is_empty() { Some(p.as_str()) } else { p.strip_prefix(&format!("{dir}/")) };
                match inside {
                    Some(rel) => {
                        if !paths.iter().any(|x| x == rel) {
                            paths.push(rel.to_string());
                        }
                    }
                    None => {
                        let base = Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let at = if paths.contains(&base) { format!("license-file/{base}") } else { base };
                        stated = Some((at, p.clone()));
                    }
                }
            }
            let mut carried: Vec<(String, String)> = paths.into_iter().map(|f| (f.clone(), from_root(&f))).collect();
            carried.extend(stated);
            for (f, from) in carried {
                let text = blob(&from)?.ok_or_else(|| io(format!("{name}'s licence file {from} is not a file of commit {commit}")))?;
                let at = dest.join(&to).join(&f);
                if let Some(parent) = at.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(at, text)?;
                files.push(format!("{to}/{f}"));
            }
            files.sort();
        } else if c["purl"].as_str().is_some_and(|p| p.starts_with("pkg:cargo/")) {
            source = c["purl"].as_str().unwrap_or_default().to_string();
            let folder = format!("{name}-{version}");
            let from = vendored.join(&folder);
            for f in files_under(&from)? {
                let at = dest.join("crates").join(&folder).join(&f);
                if let Some(parent) = at.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from.join(&f), at)?;
                files.push(format!("crates/{folder}/{f}"));
            }
        } else {
            // Qt or KDDockWidgets, which the bundle carries in `lib/qt/`.
            source = source_of(name, version);
            let texts: Vec<(String, std::path::PathBuf)> = match name {
                "qt" => ["LGPL-3", "GPL-2", "GPL-3"]
                    .iter()
                    .map(|t| (t.to_string(), licences_dir.join(t)))
                    .chain(qt_docs.iter().filter_map(|d| d.file_name().map(|n| (format!("sbom/{}", n.to_string_lossy()), d.clone()))))
                    .collect(),
                _ => match kdd_doc {
                    Some(d) => {
                        let mut t = vec![("LICENSE.txt".to_string(), d.join("LICENSE.txt"))];
                        if d.join("3RDPARTY.md").is_file() {
                            t.push(("3RDPARTY.md".to_string(), d.join("3RDPARTY.md")));
                        }
                        t.extend(files_under(&d.join("LICENSES"))?.into_iter().map(|f| (format!("LICENSES/{f}"), d.join("LICENSES").join(&f))));
                        t
                    }
                    None => Vec::new(),
                },
            };
            for (f, from) in texts {
                let at = dest.join(name).join(&f);
                if let Some(parent) = at.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&from, &at).map_err(|e| io(format!("{name}'s licence text {}: {e}", from.display())))?;
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
