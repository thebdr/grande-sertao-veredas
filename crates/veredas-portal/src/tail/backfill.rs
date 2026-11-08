//! Locating a session's start point inside a file family.
//!
//! The order of authority: file-name tokens are a hint (a day-granular prefilter), the first and
//! last line stamps are the truth (buffered peeks), and when no line stamp parses anywhere, the
//! file-name dates are the fallback; with neither, a backfill refuses loudly rather than guess.
//!
//! Peeks and scans read with buffered I/O, never a memory map: a mapped section blocks the
//! file's owner from truncating it, and reading must never block the plant's writer.
//!
//! Offsets are computed on raw bytes per detected encoding (a UTF-16 line start is not a UTF-8
//! line start), so the replay cursor seeks to an exact byte and decodes from there.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use memchr::memchr_iter;

use veredas_core::SourceError;

use crate::encoding::{DetectedEncoding, detect};
use crate::family::{FamilySpec, OrderKey};
use crate::timestamp::TimestampSpec;

/// One file of the replay plan, oldest first; the last entry is the family's active file as of
/// locate time.
#[derive(Clone, Debug)]
pub(crate) struct PlanFile {
    pub path: PathBuf,
    pub name: String,
    pub key: OrderKey,
}

#[derive(Debug)]
pub(crate) struct BackfillPlan {
    pub files: Vec<PlanFile>,
    /// The byte offset into `files[0]` where the replay begins (a line start).
    pub first_offset: u64,
    /// The encoding of `files[0]`, from the peek: a mid-file seek has no byte-order mark.
    pub first_encoding: Option<DetectedEncoding>,
    /// Members older than the plan: pre-start history, never read. (The fixed-mode convergence
    /// seeds its consumed set with these, or a rotation during the backfill would "discover"
    /// and replay them.)
    pub skipped: Vec<String>,
}

/// The first and last parseable line stamps of one file, and its encoding.
#[derive(Debug)]
pub(crate) struct FilePeek {
    pub first_ts: Option<NaiveDateTime>,
    pub last_ts: Option<NaiveDateTime>,
    pub encoding: Option<DetectedEncoding>,
    pub len: u64,
}

/// How many lines from each end a peek scans for a parseable stamp before giving up on that
/// end.
const PEEK_LINE_CAP: usize = 400;

/// The raw line table: (the absolute byte offset of the line's start, the byte range of its
/// content without the terminator). The last entry may be an unterminated partial line.
pub(crate) fn split_lines_raw(bytes: &[u8], enc: DetectedEncoding) -> Vec<(u64, Range<usize>)> {
    match enc {
        DetectedEncoding::Utf8 => {
            let base = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
            split_narrow(bytes, base)
        }
        DetectedEncoding::Cp1252 => split_narrow(bytes, 0),
        DetectedEncoding::Utf16Le => split_wide(bytes, true),
        DetectedEncoding::Utf16Be => split_wide(bytes, false),
    }
}

fn split_narrow(bytes: &[u8], base: usize) -> Vec<(u64, Range<usize>)> {
    let mut out = Vec::new();
    let mut start = base;
    for nl in memchr_iter(b'\n', &bytes[base..]) {
        let abs = base + nl;
        let mut end = abs;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        out.push((start as u64, start..end));
        start = abs + 1;
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        out.push((start as u64, start..end));
    }
    out
}

fn split_wide(bytes: &[u8], le: bool) -> Vec<(u64, Range<usize>)> {
    let bom = if le { [0xFF, 0xFE] } else { [0xFE, 0xFF] };
    let base = if bytes.starts_with(&bom) { 2 } else { 0 };
    let is_unit = |i: usize, ch: u8| -> bool {
        if le { bytes[i] == ch && bytes[i + 1] == 0 } else { bytes[i] == 0 && bytes[i + 1] == ch }
    };
    let mut out = Vec::new();
    let mut start = base;
    let mut i = base;
    // A trailing odd byte (a torn code unit) is left off the table.
    while i + 1 < bytes.len() {
        if is_unit(i, b'\n') {
            let mut end = i;
            if end >= start + 2 && is_unit(end - 2, b'\r') {
                end -= 2;
            }
            out.push((start as u64, start..end));
            start = i + 2;
        }
        i += 2;
    }
    if start < bytes.len() & !1 {
        let mut end = bytes.len() & !1;
        if end >= start + 2 && is_unit(end - 2, b'\r') {
            end -= 2;
        }
        out.push((start as u64, start..end));
    }
    out
}

fn decode_line(bytes: &[u8], enc: DetectedEncoding) -> std::borrow::Cow<'_, str> {
    let (text, _) = enc.encoding().decode_without_bom_handling(bytes);
    text
}

/// Read a file's bytes for a start-point peek or scan. Buffered, never a memory map: a mapped
/// section blocks the owner from truncating the file, and reading must never block the writer.
/// No heavier than a mapping here: the line split below scans the whole buffer anyway, and
/// family files are capped by the rotation model. A concurrent append yields a slightly longer
/// snapshot, a concurrent truncate a slightly shorter one; both fine for a heuristic peek and an
/// inclusive-offset scan.
fn read_whole(path: &Path) -> Result<Option<(Vec<u8>, u64)>, SourceError> {
    let bytes = std::fs::read(path).map_err(|e| SourceError::io(path, e))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let len = bytes.len() as u64;
    Ok(Some((bytes, len)))
}

/// The first and last parseable line stamps (a buffered read); the scan is capped at
/// `PEEK_LINE_CAP` lines from each end, except that a head carrying no stamp within the cap
/// under a tail that does is scanned forward to its first stamp: a long unstamped header must
/// not hide the file from the locate. No encoding for a file with no bytes to detect it from.
pub(crate) fn peek_span(path: &Path, ts: &TimestampSpec) -> Result<FilePeek, SourceError> {
    let Some((bytes, len)) = read_whole(path)? else {
        return Ok(FilePeek { first_ts: None, last_ts: None, encoding: None, len: 0 });
    };
    let encoding = detect(&bytes[..bytes.len().min(4096)]);
    let lines = split_lines_raw(&bytes, encoding);
    let stamp = |r: &Range<usize>| ts.extract(&decode_line(&bytes[r.clone()], encoding));
    let last_ts = lines.iter().rev().take(PEEK_LINE_CAP).find_map(|(_, r)| stamp(r));
    let mut first_ts = lines.iter().take(PEEK_LINE_CAP).find_map(|(_, r)| stamp(r));
    if first_ts.is_none() && last_ts.is_some() {
        first_ts = lines.iter().skip(PEEK_LINE_CAP).find_map(|(_, r)| stamp(r));
    }
    Ok(FilePeek { first_ts, last_ts, encoding: Some(encoding), len })
}

/// The byte offset of the first line whose stamp is at or after `start`. Unstamped lines before
/// it belong to the preceding timed region and are skipped; when no line qualifies, the file's
/// length is returned (nothing here: begin at the next file, or live-only when this is the
/// active one).
pub(crate) fn scan_start_offset(
    path: &Path,
    enc: DetectedEncoding,
    ts: &TimestampSpec,
    start: NaiveDateTime,
) -> Result<u64, SourceError> {
    let Some((bytes, len)) = read_whole(path)? else {
        return Ok(0);
    };
    for (off, range) in split_lines_raw(&bytes, enc) {
        if let Some(t) = ts.extract(&decode_line(&bytes[range], enc))
            && t >= start
        {
            return Ok(off);
        }
    }
    Ok(len)
}

/// How long a locate keeps listing again through rotations before it gives up.
const LOCATE_PATIENCE: Duration = Duration::from_millis(500);

/// Build the replay plan for `start`. `None` when the family has no members, or none with a
/// line in it (nothing to backfill; live tailing decides what to do with whatever appears).
pub(crate) fn locate_start(
    spec: &FamilySpec,
    ts: &TimestampSpec,
    start: NaiveDateTime,
) -> Result<Option<BackfillPlan>, SourceError> {
    locate_start_with(spec, ts, start, &mut |_| {})
}

/// `locate_start` with a hook before every peek, for a test that rotates the family mid-locate.
pub(crate) fn locate_start_with(
    spec: &FamilySpec,
    ts: &TimestampSpec,
    start: NaiveDateTime,
    before_peek: &mut dyn FnMut(&Path),
) -> Result<Option<BackfillPlan>, SourceError> {
    let deadline = Instant::now() + LOCATE_PATIENCE;
    loop {
        match locate_once(spec, ts, start, before_peek) {
            Ok(Located::Plan(plan)) => return Ok(plan),
            // A rotation during the locate: a member renamed away between the listing and its
            // peek (the fixed-name mode above all, whose active retires under a new name), or the
            // members changed under the peeks, so what was read may not be what was listed. The
            // listing is taken again; a family that never holds still for one listing runs the
            // patience out.
            Ok(Located::Changed) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(Located::Changed) => {
                return Err(SourceError::Backfill(
                    "cannot locate the start point: the family kept changing under every listing for 500 ms".into(),
                ));
            }
            Err(SourceError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
}

/// One pass of the locate: a plan, or the finding that the family changed under it.
enum Located {
    Plan(Option<BackfillPlan>),
    Changed,
}

fn locate_once(
    spec: &FamilySpec,
    ts: &TimestampSpec,
    start: NaiveDateTime,
    before_peek: &mut dyn FnMut(&Path),
) -> Result<Located, SourceError> {
    let files = spec.resolve()?;
    if files.is_empty() {
        return Ok(Located::Plan(None));
    }

    // The file-name token prefilter (a hint): when a file's successor carries a date from a day
    // strictly before the start's day, the file ended before that day began and is droppable
    // without a peek. Day granularity is the strongest safe inference from date-only tokens.
    let day_floor = start.date().and_hms_opt(0, 0, 0).unwrap_or(start);
    let mut candidates: Vec<usize> = Vec::new();
    for i in 0..files.len() {
        let ended_before_start_day =
            files.get(i + 1).and_then(|next| next.key.date).map(|d| d < day_floor).unwrap_or(false);
        if !ended_before_start_day {
            candidates.push(i);
        }
    }

    let mut peeks: Vec<FilePeek> = Vec::with_capacity(candidates.len());
    for &i in &candidates {
        before_peek(&files[i].path);
        peeks.push(peek_span(&files[i].path, ts)?);
    }

    // No line anywhere yet: nothing to locate in. The live attach reads what is born whole.
    if peeks.iter().all(|p| p.len == 0) {
        return Ok(Located::Plan(None));
    }

    let any_line_ts = peeks.iter().any(|p| p.first_ts.is_some() || p.last_ts.is_some());
    let enc_of = |p: &FilePeek| p.encoding.unwrap_or(DetectedEncoding::Utf8);

    // The index into `candidates` where the replay begins, and the byte offset into that file.
    let (begin, first_offset) = if any_line_ts {
        // The newest file whose first stamp is strictly before the start: a file opening on the
        // start's own stamp is reached through its predecessor's scan, so a stamp shared across
        // a seam begins in the earlier file.
        let best = (0..candidates.len()).rev().find(|&i| peeks[i].first_ts.map(|t| t < start).unwrap_or(false));
        match best {
            // The start predates everything visible: replay from the top.
            None => (0, 0),
            Some(i) => {
                let p = &peeks[i];
                if p.last_ts.map(|t| t < start).unwrap_or(false) {
                    if i + 1 < candidates.len() {
                        // The start falls in the gap between file i and i+1: the next file's
                        // first stamped line, its unstamped opening lines belonging to the
                        // earlier file's last stamp. A next file with no stamp at all is taken
                        // from its top.
                        let next = &peeks[i + 1];
                        let off = if next.first_ts.is_some() {
                            scan_start_offset(&files[candidates[i + 1]].path, enc_of(next), ts, start)?
                        } else {
                            0
                        };
                        (i + 1, off)
                    } else {
                        // The start is after everything: live-only from the end.
                        (i, p.len)
                    }
                } else {
                    let file = &files[candidates[i]];
                    (i, scan_start_offset(&file.path, enc_of(p), ts, start)?)
                }
            }
        }
    } else {
        // No line stamp parsed anywhere: fall back to the file-name dates (day granularity,
        // replaying from the start of the best-matching day).
        if candidates.iter().all(|&i| files[i].key.date.is_none()) {
            return Err(SourceError::Backfill(
                "cannot locate the start point: no line timestamps parse and the file names carry no dates; check the timestamp spec"
                    .into(),
            ));
        }
        let dated_best =
            (0..candidates.len()).rev().find(|&i| files[candidates[i]].key.date.map(|d| d <= start).unwrap_or(false));
        (dated_best.unwrap_or(0), 0)
    };

    // The peeks and the scan read files by the names the listing gave; a rotation meanwhile
    // (every rotation adds a name) may have put another file under a listed name. The listing
    // is taken again, and a plan stands only when the members are the ones it was computed for.
    let after = spec.resolve()?;
    if after.len() != files.len() || after.iter().zip(&files).any(|(a, b)| a.file_name != b.file_name) {
        return Ok(Located::Changed);
    }

    let plan_files: Vec<PlanFile> = candidates[begin..]
        .iter()
        .map(|&i| PlanFile { path: files[i].path.clone(), name: files[i].file_name.clone(), key: files[i].key })
        .collect();
    let plan_names: std::collections::HashSet<&str> = plan_files.iter().map(|f| f.name.as_str()).collect();
    let skipped =
        files.iter().filter(|f| !plan_names.contains(f.file_name.as_str())).map(|f| f.file_name.clone()).collect();

    Ok(Located::Plan(Some(BackfillPlan { files: plan_files, first_offset, first_encoding: peeks[begin].encoding, skipped })))
}

/// The plan as a test reads it: the file names in order, the first offset, the skipped names.
pub type PlanView = (Vec<String>, u64, Vec<String>);

/// The plan, for a test.
#[doc(hidden)]
pub fn plan_for(spec: &FamilySpec, ts: &TimestampSpec, start: NaiveDateTime) -> Result<Option<PlanView>, SourceError> {
    Ok(locate_start(spec, ts, start)?.map(|p| (p.files.iter().map(|f| f.name.clone()).collect(), p.first_offset, p.skipped)))
}

/// The plan, for a test that rotates the family before a peek.
#[doc(hidden)]
pub fn plan_for_with(spec: &FamilySpec, ts: &TimestampSpec, start: NaiveDateTime, before_peek: &mut dyn FnMut(&Path)) -> Result<Option<PlanView>, SourceError> {
    Ok(locate_start_with(spec, ts, start, before_peek)?.map(|p| (p.files.iter().map(|f| f.name.clone()).collect(), p.first_offset, p.skipped)))
}

/// The raw line table, for a test: (offset, content) per line.
#[doc(hidden)]
pub fn line_table(bytes: &[u8], enc: DetectedEncoding) -> Vec<(u64, String)> {
    split_lines_raw(bytes, enc).into_iter().map(|(off, r)| (off, decode_line(&bytes[r], enc).into_owned())).collect()
}
