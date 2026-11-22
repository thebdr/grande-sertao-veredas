//! git, asked as a program: what the repository held at a commit, read from its objects, never
//! from a checkout (a release is judged against history, as law L6 judges a plan).

use std::path::Path;
use std::process::{Command, Stdio};

use thiserror::Error;

/// git could not be run at all (absent, or the folder unreadable).
#[derive(Debug, Error)]
#[error("git could not run in {dir}: {source}")]
pub struct GitError {
    pub dir: String,
    #[source]
    pub source: std::io::Error,
}

/// Runs git at `root`: `Some(stdout)` when it succeeded, `None` when it answered no (an object
/// that is not there, a commit that is no ancestor).
pub fn run(root: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| GitError { dir: root.display().to_string(), source })?;
    Ok(out.status.success().then_some(out.stdout))
}

/// [`run`], its output read as text.
pub fn text(root: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    Ok(run(root, args)?.map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// A file's text at a commit (`git show <commit>:<path>`), `None` when the commit holds no such
/// file.
pub fn show(root: &Path, commit: &str, path: &str) -> Result<Option<String>, GitError> {
    text(root, &["show", &format!("{commit}:{path}")])
}

/// The names directly under a folder at a commit, `None` when the commit holds no such folder.
pub fn list(root: &Path, commit: &str, dir: &str) -> Result<Option<Vec<String>>, GitError> {
    let spec = format!("{commit}:{dir}");
    Ok(text(root, &["ls-tree", "--name-only", &spec])?.map(|t| t.lines().map(str::to_string).collect()))
}

/// The object id a path names at a commit (a folder's tree, a file's blob).
pub fn object(root: &Path, commit: &str, path: &str) -> Result<Option<String>, GitError> {
    let spec = format!("{commit}:{path}");
    Ok(text(root, &["rev-parse", "--verify", "--quiet", &spec])?.map(|t| t.trim().to_string()))
}

/// Writes the tree of `commit` into `dest` from git's objects as they are stored, no attribute,
/// filter or line-ending rule applied (`git archive` applies `export-ignore`, `export-subst` and
/// smudge filters from settings outside the history, so two clones could export one commit two
/// ways). Only the paths under one of `prefixes` are written (every path when it is empty); an
/// executable keeps its mode, a link is made a link, a submodule is left out. Returns how many
/// files were written.
pub fn export_tree(root: &Path, commit: &str, prefixes: &[&str], dest: &Path) -> Result<usize, GitError> {
    use std::io::{BufRead, BufReader, Read, Write};
    let io = |source: std::io::Error| GitError { dir: root.display().to_string(), source };
    let listing = run(root, &["ls-tree", "-r", "-z", "--full-tree", commit])?
        .ok_or_else(|| io(std::io::Error::other(format!("git cannot list the tree of {commit}"))))?;
    // (mode, object id, path) of every blob wanted.
    let mut wanted: Vec<(String, String, String)> = Vec::new();
    for rec in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let Some((info, path)) = rec.split_once('\t') else { continue };
        let mut f = info.split_whitespace();
        let (Some(mode), Some(kind), Some(oid)) = (f.next(), f.next(), f.next()) else { continue };
        if kind != "blob" {
            continue;
        }
        if !prefixes.is_empty() && !prefixes.iter().any(|p| path == *p || path.starts_with(&format!("{}/", p.trim_end_matches('/')))) {
            continue;
        }
        wanted.push((mode.to_string(), oid.to_string(), path.to_string()));
    }
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(io)?;
    let mut stdin = child.stdin.take().ok_or_else(|| io(std::io::Error::other("no stdin")))?;
    let ids: String = wanted.iter().map(|(_, oid, _)| format!("{oid}\n")).collect();
    let feeder = std::thread::spawn(move || {
        let r = stdin.write_all(ids.as_bytes());
        drop(stdin);
        r
    });
    let mut out = BufReader::new(child.stdout.take().ok_or_else(|| io(std::io::Error::other("no stdout")))?);
    for (mode, oid, path) in &wanted {
        let mut header = String::new();
        out.read_line(&mut header).map_err(io)?;
        let size: usize = header
            .split_whitespace()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io(std::io::Error::other(format!("git cat-file answered `{}` for {oid}", header.trim()))))?;
        let mut body = vec![0u8; size];
        out.read_exact(&mut body).map_err(io)?;
        let mut newline = [0u8; 1];
        out.read_exact(&mut newline).map_err(io)?;
        let target = dest.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        if mode == "120000" {
            std::os::unix::fs::symlink(String::from_utf8_lossy(&body).as_ref(), &target).map_err(io)?;
        } else {
            std::fs::write(&target, &body).map_err(io)?;
            if mode == "100755" {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
            }
        }
    }
    let _ = feeder.join();
    let _ = child.wait();
    Ok(wanted.len())
}
