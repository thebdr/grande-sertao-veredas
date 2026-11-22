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
