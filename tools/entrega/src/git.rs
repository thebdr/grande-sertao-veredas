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

/// git at `root`, with what is not history turned off: replace refs (`refs/replace/*`, which a
/// clone does not fetch) and the grafts file both rewrite objects and ancestry, so every git this
/// tool runs reads the objects and parents as committed.
pub fn command(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).env("GIT_NO_REPLACE_OBJECTS", "1").env("GIT_GRAFT_FILE", "/dev/null");
    c
}

/// Runs git at `root` ([`command`]): `Some(stdout)` when it succeeded, `None` when it answered no
/// (an object that is not there, a commit that is no ancestor).
pub fn run(root: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, GitError> {
    let out = command(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| GitError { dir: root.display().to_string(), source })?;
    Ok(out.status.success().then_some(out.stdout))
}

/// Whether `id` is a commit's own full id here: an object of type commit that resolves to exactly
/// itself (a ref named like an id, or a prefix of a longer id, resolves to another).
pub fn is_commit_id(root: &Path, id: &str) -> Result<bool, GitError> {
    let kind = text(root, &["cat-file", "-t", id])?.map(|t| t.trim().to_string());
    if kind.as_deref() != Some("commit") {
        return Ok(false);
    }
    let spec = format!("{id}^{{commit}}");
    Ok(text(root, &["rev-parse", "--verify", "--quiet", &spec])?.is_some_and(|t| t.trim() == id))
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

/// Why a tree was not exported: git could not run, or the tree holds what the export refuses.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("{0}")]
    Refused(String),
}

/// Writes the tree of `commit` into `dest` from git's objects as they are stored, no attribute,
/// filter or line-ending rule applied (`git archive` applies `export-ignore`, `export-subst` and
/// smudge filters from settings outside the history, so two clones could export one commit two
/// ways). Only the paths under one of `prefixes` are written (every path when it is empty); an
/// executable keeps its mode, a submodule is left out. A path with a `..`, `.` or empty part (a
/// tree `git mktree` can make) is refused before anything is written. A release's tree holds no
/// link and no path that is not UTF-8: a link, judged as text, can still reach outside through another link or make
/// a member's folder somewhere else, and a name that is not UTF-8 cannot be written as stored; each
/// is refused by its path, wherever it sits. Returns how many files were written.
pub fn export_tree(root: &Path, commit: &str, prefixes: &[&str], dest: &Path) -> Result<usize, ExportError> {
    use std::io::{BufRead, BufReader, Read, Write};
    let io = |source: std::io::Error| GitError { dir: root.display().to_string(), source };
    let listing = run(root, &["ls-tree", "-r", "-z", "--full-tree", commit])?
        .ok_or_else(|| io(std::io::Error::other(format!("git cannot list the tree of {commit}"))))?;
    // (mode, object id, path) of every blob wanted.
    let mut wanted: Vec<(String, String, String)> = Vec::new();
    for raw in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Ok(rec) = std::str::from_utf8(raw) else {
            let lossy = String::from_utf8_lossy(raw);
            let path = lossy.split_once('\t').map_or(lossy.as_ref(), |(_, p)| p);
            return Err(ExportError::Refused(format!("the path `{path}` is not UTF-8: a release's tree names its files in UTF-8")));
        };
        let Some((info, path)) = rec.split_once('\t') else { continue };
        if path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            return Err(ExportError::Refused(format!("the path `{path}` has a `..`, `.` or empty part: it would be written outside its place")));
        }
        let mut f = info.split_whitespace();
        let (Some(mode), Some(kind), Some(oid)) = (f.next(), f.next(), f.next()) else { continue };
        if mode == "120000" {
            return Err(ExportError::Refused(format!(
                "the link `{path}`: a release's tree holds no link, which could lead outside the history"
            )));
        }
        if kind == "commit" {
            return Err(ExportError::Refused(format!(
                "the submodule `{path}`: a release's tree holds no submodule, whose code its history does not hold"
            )));
        }
        if kind != "blob" {
            continue;
        }
        if !prefixes.is_empty() && !prefixes.iter().any(|p| path == *p || path.starts_with(&format!("{}/", p.trim_end_matches('/')))) {
            continue;
        }
        wanted.push((mode.to_string(), oid.to_string(), path.to_string()));
    }
    let mut child = command(root)
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
        std::fs::write(&target, &body).map_err(io)?;
        if mode == "100755" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
        }
    }
    let _ = feeder.join();
    let _ = child.wait();
    Ok(wanted.len())
}
