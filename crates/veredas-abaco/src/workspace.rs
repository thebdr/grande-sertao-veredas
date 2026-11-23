//! The session a `.abaco` file holds besides its workbook (the plan, "The `.abaco` file"): the
//! foreign files it works on, what its tools decided and queued, each tool's own state, the log.
//!
//! Ábaco keeps the shapes generic: a tool decides what its records look like, Ábaco stores, orders
//! and round-trips them, as TOML tables (TOML has no null, so a record holds none).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A foreign file the workspace works on (an IO list, a plant's CSV), with the fingerprint it had
/// when it was recorded, so a tool can tell whether it moved on since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// What the file is to the tool: `io_list`, `inputs_csv`, …
    pub role: String,
    pub path: PathBuf,
    /// Its size in bytes when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// When it was last changed, as recorded: RFC 3339, UTC, whole seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// SHA-256 of its content when recorded, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl Reference {
    /// Reads `path` and records its size, its last change and its SHA-256.
    pub fn record(role: &str, path: &Path) -> std::io::Result<Reference> {
        let meta = std::fs::metadata(path)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| rfc3339(d.as_secs()));
        Ok(Reference {
            role: role.to_string(),
            path: path.to_path_buf(),
            size: Some(meta.len()),
            modified,
            sha256: Some(sha256_of(path)?),
        })
    }

    /// Whether the file's content differs from what was recorded: a file gone or unreadable
    /// differs; a reference recorded without a hash never does.
    pub fn changed(&self) -> bool {
        match &self.sha256 {
            None => false,
            Some(h) => sha256_of(&self.path).map(|now| &now != h).unwrap_or(true),
        }
    }
}

/// SHA-256 of a file's content, lowercase hex. A plain open, so the file's owner may write it
/// meanwhile.
pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Seconds since 1970 as RFC 3339 in UTC (`2026-11-23T10:04:05Z`).
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // the civil date of a day count (Howard Hinnant's days_from_civil, inverted)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// Whether `s` is RFC 3339 in UTC to the second, as [`rfc3339`] writes it: `YYYY-MM-DDThh:mm:ssZ`,
/// a real day of its month.
pub fn is_rfc3339_utc(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let n = |r: std::ops::Range<usize>| s.get(r).and_then(|t| t.parse::<u32>().ok()).unwrap_or(99);
    if b.len() != 20 || !(digits(0..4) && digits(5..7) && digits(8..10) && digits(11..13) && digits(14..16) && digits(17..19)) {
        return false;
    }
    if (b[4], b[7], b[10], b[13], b[16], b[19]) != (b'-', b'-', b'T', b':', b':', b'Z') {
        return false;
    }
    let (year, month, day) = (n(0..4), n(5..7), n(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && n(11..13) < 24 && n(14..16) < 60 && n(17..19) < 60
}

/// What a workspace carries besides its sheets and macros.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    /// The foreign files, in the order they were added, one per role.
    pub references: Vec<Reference>,
    /// What a tool decided, oldest first.
    pub decisions: Vec<toml::Table>,
    /// What a tool queued, in order.
    pub queue: Vec<toml::Table>,
    /// Each tool's own state, by tool name (`acerto`, …): letters, digits and `_`.
    pub state: BTreeMap<String, toml::Table>,
    /// The log, oldest first: link lines, patch reports, a tool's warnings. Plain text in the file
    /// (`workspace/log.txt`), one entry per line.
    pub log: Vec<String>,
}

impl Workspace {
    /// Adds a reference, or replaces the one of the same role.
    pub fn set_reference(&mut self, reference: Reference) {
        match self.references.iter_mut().find(|r| r.role == reference.role) {
            Some(existing) => *existing = reference,
            None => self.references.push(reference),
        }
    }

    pub fn reference(&self, role: &str) -> Option<&Reference> {
        self.references.iter().find(|r| r.role == role)
    }

    pub fn is_empty(&self) -> bool {
        *self == Workspace::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_knows_when_its_file_moved_on() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("inputs.csv");
        std::fs::write(&p, "a;b\n").unwrap();
        let r = Reference::record("inputs_csv", &p).unwrap();
        assert_eq!(r.size, Some(4));
        assert_eq!(r.sha256.as_deref().map(str::len), Some(64));
        assert!(r.modified.as_deref().is_some_and(|m| m.len() == 20 && m.ends_with('Z')));
        assert!(!r.changed());
        std::fs::write(&p, "a;c\n").unwrap();
        assert!(r.changed());
        std::fs::remove_file(&p).unwrap();
        assert!(r.changed());
    }

    #[test]
    fn sha256_and_dates_match_known_values() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, "abc").unwrap();
        assert_eq!(sha256_of(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_795_428_245), "2026-11-23T10:04:05Z");
        assert_eq!(rfc3339(4_107_542_399), "2100-02-28T23:59:59Z");
        for secs in [0, 951_782_400, 1_795_428_245, 4_107_542_399] {
            assert!(is_rfc3339_utc(&rfc3339(secs)));
        }
        for bad in ["2026-11-23 10:04:05Z", "2026-11-23T10:04:05", "2026-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-11-23T24:00:00Z", "2026-11-23T10:04:05+00:00"] {
            assert!(!is_rfc3339_utc(bad), "{bad}");
        }
        assert!(is_rfc3339_utc("2024-02-29T00:00:00Z"));
    }

    #[test]
    fn a_reference_of_a_role_is_replaced() {
        let mut w = Workspace::default();
        let r = |role: &str, p: &str| Reference { role: role.into(), path: p.into(), size: None, modified: None, sha256: None };
        w.set_reference(r("io_list", "a.xlsx"));
        w.set_reference(r("inputs_csv", "i.csv"));
        w.set_reference(r("io_list", "b.xlsx"));
        assert_eq!(w.references.len(), 2);
        assert_eq!(w.reference("io_list").unwrap().path, PathBuf::from("b.xlsx"));
        assert!(!w.is_empty());
        assert!(Workspace::default().is_empty());
    }
}
