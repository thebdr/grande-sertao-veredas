//! File families: the rotation model of a plant logger, capped files with a new sibling when the
//! cap is reached.
//!
//! A [`FamilySpec`] is a regex over file names with two reserved named groups: `(?P<date>…)`,
//! parsed with a chrono format, and `(?P<idx>…)`, parsed as an integer (a numeric sort, so
//! `_10` follows `_2`). Membership is the pattern matching the whole name; the ordering key is
//! (date, idx).
//!
//! The order of authority: file-name tokens are a hint, line timestamps are the truth (a
//! backfill's peek reads them), a file's modification time is the last resort, breaking ties
//! among keyless files before the name does.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::NaiveDateTime;
use regex::Regex;

use veredas_core::SourceError;

use crate::timestamp::parse_flexible;

/// How the file being written is told from the retired ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveMode {
    /// The member with the greatest (date, idx) key is the active file: schemes that stamp or
    /// index every new file.
    NewestByOrder,
    /// The active file has this constant name (`current.log`); retirees are renamed into the
    /// pattern. A rotation shows as a change of the file's identity under this name.
    Fixed(String),
}

/// The sort key parsed from a member's name tokens. A missing token orders first (`None` before
/// `Some`): a keyless file is taken as older than anything stamped, and keyless files order
/// among themselves by modification time, then name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderKey {
    pub date: Option<NaiveDateTime>,
    pub idx: Option<u64>,
}

impl OrderKey {
    pub const NONE: OrderKey = OrderKey { date: None, idx: None };
}

/// One resolved member of a family, in resolved order.
#[derive(Clone, Debug)]
pub struct FamilyFile {
    pub path: PathBuf,
    /// The plain name in a flat family; the `/`-separated path relative to the family's folder
    /// in a recursive one.
    pub file_name: String,
    pub key: OrderKey,
    pub mtime: Option<SystemTime>,
    pub is_active: bool,
}

/// One source's family description; schemes differ per plant, so all of this is configuration.
#[derive(Clone, Debug)]
pub struct FamilySpec {
    dir: PathBuf,
    pattern: Regex,
    date_format: Option<String>,
    active: ActiveMode,
    recursive: bool,
}

/// A junction or symlink loop must not hang a resolve; a plant's layout is a couple of levels
/// deep at most.
const MAX_RECURSION_DEPTH: usize = 16;

impl FamilySpec {
    /// `pattern` is matched against the whole file name (wrapped in `^(?:…)$`, so no substring
    /// matches). A `(?P<date>…)` group requires `date_format`, and the other way round.
    pub fn new(
        dir: impl Into<PathBuf>,
        pattern: &str,
        date_format: Option<&str>,
        active: ActiveMode,
    ) -> Result<Self, SourceError> {
        let anchored = Regex::new(&format!("^(?:{pattern})$"))?;
        let has_date_group = anchored.capture_names().flatten().any(|name| name == "date");
        match (has_date_group, date_format) {
            (true, None) => {
                return Err(SourceError::Spec(format!(
                    "pattern `{pattern}` captures (?P<date>…) but no date format was given"
                )));
            }
            (false, Some(fmt)) => {
                return Err(SourceError::Spec(format!(
                    "date format `{fmt}` given but pattern `{pattern}` has no (?P<date>…) group"
                )));
            }
            _ => {}
        }
        Ok(FamilySpec {
            dir: dir.into(),
            pattern: anchored,
            date_format: date_format.map(String::from),
            active,
            recursive: false,
        })
    }

    /// Scan subfolders too. Membership then matches the path relative to the family's folder,
    /// `/`-separated (`01/x_1.log`), so same-named files in different folders stay distinct and
    /// a pattern can confine itself to a subtree. Off by default: one flat folder.
    pub fn with_recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    pub fn recursive(&self) -> bool {
        self.recursive
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn active_mode(&self) -> &ActiveMode {
        &self.active
    }

    /// Membership and the token parse. `Some` means the name belongs to the family. A token that
    /// matches the regex but fails its chrono or integer parse degrades to `None` inside the key
    /// (the file stays a member).
    pub fn parse_key(&self, file_name: &str) -> Option<OrderKey> {
        let caps = self.pattern.captures(file_name)?;
        let date = match (caps.name("date"), &self.date_format) {
            (Some(m), Some(fmt)) => parse_flexible(m.as_str(), fmt),
            _ => None,
        };
        let idx = caps.name("idx").and_then(|m| m.as_str().parse::<u64>().ok());
        Some(OrderKey { date, idx })
    }

    /// Scan the folder (the subtree when recursive) and return the family in resolved order:
    /// retirees ascending by (key, mtime, name), the active file, per the mode, last.
    pub fn resolve(&self) -> Result<Vec<FamilyFile>, SourceError> {
        let fixed_name = match &self.active {
            ActiveMode::Fixed(name) => Some(name.as_str()),
            ActiveMode::NewestByOrder => None,
        };

        let mut members: Vec<FamilyFile> = Vec::new();
        let mut fixed_active: Option<FamilyFile> = None;

        for (name, path, mtime) in self.collect_entries()? {
            let name = name.as_str();
            if Some(name) == fixed_name {
                fixed_active = Some(FamilyFile {
                    path,
                    file_name: name.to_string(),
                    // The tokens may still parse when the fixed name happens to match the
                    // pattern; ordering ignores the active file anyway.
                    key: self.parse_key(name).unwrap_or(OrderKey::NONE),
                    mtime,
                    is_active: true,
                });
                continue;
            }
            if let Some(key) = self.parse_key(name) {
                members.push(FamilyFile { path, file_name: name.to_string(), key, mtime, is_active: false });
            }
        }

        members.sort_by(|a, b| {
            a.key.cmp(&b.key).then_with(|| a.mtime.cmp(&b.mtime)).then_with(|| a.file_name.cmp(&b.file_name))
        });

        match self.active {
            ActiveMode::NewestByOrder => {
                if let Some(last) = members.last_mut() {
                    last.is_active = true;
                }
            }
            ActiveMode::Fixed(_) => {
                if let Some(active) = fixed_active {
                    members.push(active);
                }
            }
        }

        Ok(members)
    }

    /// `(candidate name, absolute path, mtime)` for every file in scope. Flat: one `read_dir`,
    /// the name is the file name. Recursive: a bounded walk, the name the `/`-separated path
    /// relative to the family's folder. An entry or a subfolder that vanishes or errors mid-scan
    /// is skipped (a rotation in progress; the next rescan sees the settled state); only an
    /// unreadable root is an error.
    fn collect_entries(&self) -> Result<Vec<(String, PathBuf, Option<SystemTime>)>, SourceError> {
        let mut out = Vec::new();
        let mut stack: Vec<(PathBuf, usize)> = vec![(self.dir.clone(), 0)];
        let mut first = true;
        while let Some((dir, depth)) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if first => return Err(SourceError::io(&dir, e)),
                Err(_) => continue,
            };
            first = false;
            for entry in entries {
                let Ok(entry) = entry else { continue };
                let Ok(file_type) = entry.file_type() else { continue };
                let path = entry.path();
                if file_type.is_dir() {
                    if self.recursive && depth < MAX_RECURSION_DEPTH {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let name = if self.recursive {
                    let Ok(rel) = path.strip_prefix(&self.dir) else { continue };
                    let mut parts = Vec::new();
                    let mut all_utf8 = true;
                    for comp in rel.components() {
                        match comp.as_os_str().to_str() {
                            Some(s) => parts.push(s),
                            None => {
                                all_utf8 = false;
                                break;
                            }
                        }
                    }
                    if !all_utf8 {
                        continue;
                    }
                    parts.join("/")
                } else {
                    let Some(name) = entry.file_name().to_str().map(String::from) else { continue };
                    name
                };
                let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());
                out.push((name, path, mtime));
            }
        }
        Ok(out)
    }
}
