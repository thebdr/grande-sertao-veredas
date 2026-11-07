//! Log emitters: lines rendered from a template, files named the way a plant names them, written
//! the way a plant logger writes them.
//!
//! An [`EmitterConfig`] says what a line looks like (`{ts}` for the timestamp in a chrono format,
//! `{name}` for a value drawn within its range), how often one is written, and how the files are
//! named and rotated: a pattern of chrono specifiers and `{n}` for the rotation index, a new
//! index after `max_lines` lines, back to index 1 on a new day. The worker ([`spawn`]) writes
//! CRLF, flushed per line, in UTF-8 or UTF-16LE with its byte-order mark, and reports what it
//! does through an [`EmitterStatus`]. A restarted emitter never appends to a file it already
//! wrote: it probes past them.
//!
//! What portal's log families will read back is stated by [`family_shape`]: the regex with its
//! date and index groups, and the date format, that resolve the pattern's files in the order they
//! were written.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Deserialize;
use veredas_core::rng::Pcg32;

/// One emitter, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterConfig {
    pub name: String,
    /// The line: `{ts}` the timestamp, `{name}` a value; anything else verbatim.
    pub template: String,
    /// The chrono format `{ts}` renders with.
    pub ts_format: String,
    /// The values drawn fresh for every line.
    #[serde(default)]
    pub values: Vec<ValueField>,
    /// Milliseconds between lines; a zero writes as fast as it can.
    pub interval_ms: u64,
    /// Chrono specifiers plus `{n}` for the rotation index: `%d%m%Y_Trace{n}.txt`.
    pub file: String,
    /// Rotate to the next index after this many lines; 0 never rotates on count.
    #[serde(default)]
    pub max_lines: u64,
    #[serde(default)]
    pub encoding: Encoding,
    /// A fixed seed replays the same values; absent, the clock seeds the draws.
    #[serde(default)]
    pub seed: Option<u64>,
}

/// A value drawn within `[low, high]`, rendered to `decimals`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueField {
    pub name: String,
    pub low: f64,
    pub high: f64,
    #[serde(default)]
    pub decimals: u8,
}

/// How a file is written. Plants write both; a consumer must detect either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Encoding {
    #[default]
    #[serde(rename = "utf-8")]
    Utf8,
    /// Little-endian UTF-16, a byte-order mark first.
    #[serde(rename = "utf-16le")]
    Utf16le,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16le => "UTF-16LE",
        }
    }
}

/// Format a timestamp with a typed-in chrono format, or say the format is bad.
///
/// `format(..).to_string()` panics on a bad format: chrono yields an error item, `Display`
/// fails, and `ToString` unwraps it. The formats here come from a file an operator wrote, so
/// the failure is a value, never a panic.
pub fn safe_format(dt: &DateTime<Local>, fmt: &str) -> Option<String> {
    let mut out = String::new();
    match write!(out, "{}", dt.format(fmt)) {
        Ok(()) => Some(out),
        Err(_) => None,
    }
}

/// What a bad timestamp format renders as, in a line.
pub const BAD_FORMAT: &str = "(bad datetime format)";

/// Render one line: `{ts}` becomes the formatted timestamp, `{name}` that value to its decimals.
///
/// Anything unrecognised (an unknown name, an unclosed brace, a stray `}`) is written as typed:
/// the line then shows the mistake where a consumer reads it, instead of hiding it.
pub fn render_line(
    template: &str,
    ts_format: &str,
    now: &DateTime<Local>,
    values: &[(String, f64, u8)],
) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        if name == "ts" {
            out.push_str(&safe_format(now, ts_format).unwrap_or_else(|| BAD_FORMAT.to_string()));
        } else if let Some((_, v, decimals)) = values.iter().find(|(n, _, _)| n == name) {
            let _ = write!(out, "{v:.*}", *decimals as usize);
        } else {
            out.push('{');
            out.push_str(name);
            out.push('}');
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Draw this emitter's values for one line.
pub fn draw_values(fields: &[ValueField], rng: &mut Pcg32) -> Vec<(String, f64, u8)> {
    fields.iter().map(|f| (f.name.clone(), rng.uniform(f.low, f.high, f.decimals), f.decimals)).collect()
}

/// Why a file pattern cannot name a file.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("the file pattern is not a valid datetime format")]
    BadFormat,
    #[error("the file pattern is empty")]
    Empty,
    /// The rendered name holds a character a file name cannot (`%H:%M` renders a `:`), which
    /// would otherwise fail at open time with a baffling message.
    #[error("a file name cannot contain {0}")]
    InvalidChars(String),
    /// Windows drops a trailing dot or space, so the file on disk would not be the name reported.
    #[error("a file name cannot end with `.` or a space")]
    BadEnding,
    /// A name Windows reserves whatever its extension (`CON`, `NUL`, `COM1`...): writing it
    /// opens a device, not a file.
    #[error("`{0}` is a name the system reserves")]
    Reserved(String),
}

/// Characters no file name may hold (Windows forbids them; a name without them opens anywhere).
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Names Windows reserves whatever the extension, by their stem.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The one place the naming scheme lives.
///
/// A family is read as a regex with a date group and an index group, ordered by (date, index):
/// plant files look like `28072026_Trace1.txt`. So a pattern carries chrono specifiers **and**
/// `{n}` for the index: `%d%m%Y_Trace{n}.txt`. `{n}` is substituted in the pattern before the
/// date is rendered (splitting a rendered name is ambiguous once a date format has dots). A
/// pattern without `{n}` names index 1 bare, and later indices gain `_<idx>` before the
/// pattern's extension.
pub fn file_name_for(pattern: &str, now: &DateTime<Local>, idx: u32) -> Result<String, NameError> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(NameError::Empty);
    }
    let with_idx = if pattern.contains("{n}") {
        pattern.replace("{n}", &idx.to_string())
    } else if idx <= 1 {
        pattern.to_string()
    } else {
        match extension_dot(pattern) {
            Some(dot) => format!("{}_{idx}{}", &pattern[..dot], &pattern[dot..]),
            None => format!("{pattern}_{idx}"),
        }
    };
    let rendered = safe_format(now, &with_idx).ok_or(NameError::BadFormat)?;
    if rendered.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if let Some(bad) = rendered.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        return Err(NameError::InvalidChars(format!("`{bad}`")));
    }
    if rendered.ends_with('.') || rendered.ends_with(' ') {
        return Err(NameError::BadEnding);
    }
    let stem = rendered.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(NameError::Reserved(rendered));
    }
    Ok(rendered)
}

/// Where a pattern's extension starts, if it has one.
///
/// Not simply the last dot: a dotted date format (`log_%d.%m.%Y`) has dots that belong to the
/// date, and an index inserted at the last one would land inside the timestamp
/// (`log_28.07_2.2026`), which no family regex reads. A trailing segment is an extension only
/// when it holds no specifier and looks like one: short and alphanumeric.
fn extension_dot(pattern: &str) -> Option<usize> {
    let dot = pattern.rfind('.')?;
    let ext = &pattern[dot + 1..];
    let plausible = !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    plausible.then_some(dot)
}

/// Rotation bookkeeping, pure, so it is pinned without waiting for midnight.
///
/// Two triggers: the date part of the name changed (a new day starts at index 1), or the line
/// count reached `max_lines` (the index advances).
pub struct Rotor {
    /// The index-1 rendering of the current name: the "still the same day" key. `None` until the
    /// first line decides it: an empty string would read as a name that just changed and reset
    /// the index a restart deliberately probed for.
    base: Option<String>,
    idx: u32,
    lines_in_file: u64,
}

impl Rotor {
    /// Start at `idx` (at least 1), having written nothing.
    pub fn new(idx: u32) -> Rotor {
        Rotor { base: None, idx: idx.max(1), lines_in_file: 0 }
    }

    pub fn idx(&self) -> u32 {
        self.idx
    }

    pub fn lines_in_file(&self) -> u64 {
        self.lines_in_file
    }

    /// The file the next line belongs in, and whether this call moved to a different file than
    /// the previous one.
    pub fn file_for(&mut self, pattern: &str, now: &DateTime<Local>, max_lines: u64) -> Result<(String, bool), NameError> {
        let base = file_name_for(pattern, now, 1)?;
        let mut rotated = false;
        if self.base.is_none() {
            self.base = Some(base);
            rotated = true;
        } else if self.base.as_deref() != Some(base.as_str()) {
            self.base = Some(base);
            self.idx = 1;
            self.lines_in_file = 0;
            rotated = true;
        } else if max_lines > 0 && self.lines_in_file >= max_lines {
            self.idx += 1;
            self.lines_in_file = 0;
            rotated = true;
        }
        Ok((file_name_for(pattern, now, self.idx)?, rotated))
    }

    /// Record that a line was written to the current file.
    pub fn note_line(&mut self) {
        self.lines_in_file += 1;
    }

    /// Move to the next index without writing: how a file that already exists is skipped.
    pub fn skip(&mut self, pattern: &str, now: &DateTime<Local>) -> Result<String, NameError> {
        self.idx += 1;
        self.lines_in_file = 0;
        file_name_for(pattern, now, self.idx)
    }
}

/// The shape a family reader needs to resolve the files a pattern writes: a regex with a `date`
/// group and an `idx` group, and the date format of the `date` group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyShape {
    pub regex: String,
    pub date_format: String,
}

/// One piece of a file pattern, as the family shape reads it.
enum Piece {
    /// A literal character.
    Lit(char),
    /// A date specifier and the digits it renders.
    Spec(char, usize),
    /// The rotation index, `{n}`.
    Idx,
    /// Where a pattern without `{n}` takes its `_<idx>` for indices past 1.
    Ext,
}

/// The family shape of a pattern, or `None` for a pattern with no date, with a specifier beyond
/// the common ones (`%Y %y %m %d %H %M %S`), with a placeholder other than `{n}`, or with a
/// character no file name holds: a shape that might match the wrong files is never guessed.
///
/// The `date` group runs from the first specifier to the last one reached through single
/// literal separators (`%d.%m.%Y`, `%Y-%m-%d_%H%M`), and the date format holds exactly what the
/// group covers, separators included, so a name's date parses back with it. A specifier beyond a
/// longer literal (`%Y_T%m`) stays outside the group as digits of the name. A pattern without
/// `{n}` gets an optional `_<idx>` group where [`file_name_for`] inserts the index (before the
/// extension, else at the end), so index 1's bare name and its rotated siblings resolve together;
/// a reader takes a missing index as 1.
pub fn family_shape(pattern: &str) -> Option<FamilyShape> {
    let pattern = pattern.trim();
    let mut pieces = Vec::new();
    let mut chars = pattern.char_indices();
    // Where an index would be inserted for a pattern without `{n}`: before the extension's dot.
    let ext_at = extension_dot(pattern);
    let mut has_idx = false;
    while let Some((at, c)) = chars.next() {
        if Some(at) == ext_at {
            pieces.push(Piece::Ext);
        }
        match c {
            '{' => {
                if chars.next().map(|(_, c)| c) == Some('n') && chars.next().map(|(_, c)| c) == Some('}') {
                    pieces.push(Piece::Idx);
                    has_idx = true;
                } else {
                    return None;
                }
            }
            '%' => {
                let (_, spec) = chars.next()?;
                let digits = match spec {
                    'Y' => 4,
                    'm' | 'd' | 'H' | 'M' | 'S' | 'y' => 2,
                    _ => return None,
                };
                pieces.push(Piece::Spec(spec, digits));
            }
            c if FORBIDDEN.contains(&c) || c.is_control() => return None,
            c => pieces.push(Piece::Lit(c)),
        }
    }
    if ext_at.is_none() {
        pieces.push(Piece::Ext);
    }
    if has_idx {
        pieces.retain(|p| !matches!(p, Piece::Ext));
    }
    let first = pieces.iter().position(|p| matches!(p, Piece::Spec(..)))?;
    let mut last = first;
    let mut i = first + 1;
    loop {
        match (pieces.get(i), pieces.get(i + 1)) {
            (Some(Piece::Spec(..)), _) => {
                last = i;
                i += 1;
            }
            (Some(Piece::Lit(_)), Some(Piece::Spec(..))) => i += 1,
            _ => break,
        }
    }
    // The date is one run of specifiers (a separator between two is part of it) that carries a
    // year: a second run, or a run with no year, could not be read back as one date the reader
    // orders by, so such a pattern has no shape.
    if pieces.iter().skip(last + 1).any(|p| matches!(p, Piece::Spec(..))) {
        return None;
    }
    if !pieces[first..=last].iter().any(|p| matches!(p, Piece::Spec('Y' | 'y', _))) {
        return None;
    }
    let mut regex = String::new();
    let mut date_format = String::new();
    for (at, piece) in pieces.iter().enumerate() {
        if at == first {
            regex.push_str("(?P<date>");
        }
        match piece {
            Piece::Lit(c) => {
                // Only a regex metacharacter is escaped (the set `regex::escape` escapes, bar
                // the hyphen, literal outside a class); any other character, ASCII or not,
                // stands for itself.
                if "\\.+*?()|[]{}^$#&~".contains(*c) {
                    regex.push('\\');
                }
                regex.push(*c);
                if (first..=last).contains(&at) {
                    date_format.push(*c);
                }
            }
            Piece::Spec(spec, digits) => {
                let _ = write!(regex, "\\d{{{digits}}}");
                if (first..=last).contains(&at) {
                    let _ = write!(date_format, "%{spec}");
                }
            }
            // The first `{n}` is the index group; a second renders the same number and is
            // matched as digits (a group name may be used once).
            Piece::Idx if !regex.contains("(?P<idx>") => regex.push_str("(?P<idx>\\d+)"),
            Piece::Idx => regex.push_str("\\d+"),
            Piece::Ext => regex.push_str("(?:_(?P<idx>\\d+))?"),
        }
        if at == last {
            regex.push(')');
        }
    }
    Some(FamilyShape { regex, date_format })
}

/// Why an emitter could not write. Carried in its status, so a stopped share or a bad pattern is
/// read where the emitter is watched, and typed, so a test matches the kind.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmitError {
    #[error(transparent)]
    Name(#[from] NameError),
    /// An I/O failure, with what was being done and where.
    #[error("{action} {path}: {message}")]
    Io { action: &'static str, path: PathBuf, message: String },
    /// Every name tried already exists: nothing is written, since appending is never done.
    #[error("no free file name after {tried} names that exist")]
    NoFreeName { tried: u32 },
}

impl EmitError {
    fn io(action: &'static str, path: &Path, e: &std::io::Error) -> EmitError {
        EmitError::Io { action, path: path.to_path_buf(), message: e.to_string() }
    }
}

/// What a running emitter reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmitterStatus {
    pub total_lines: u64,
    pub lines_in_file: u64,
    pub current_file: Option<String>,
    pub last_error: Option<EmitError>,
}

/// A running emitter. Dropping it asks the worker to stop and waits for it.
pub struct EmitterHandle {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<EmitterStatus>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl EmitterHandle {
    /// Ask the worker to stop; it notices within a slice ([`SLICE`]).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.join.as_ref().map(|j| j.is_finished()).unwrap_or(true)
    }

    pub fn status(&self) -> EmitterStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Stop and wait for the worker.
    pub fn join(mut self) {
        self.stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for EmitterHandle {
    fn drop(&mut self) {
        self.stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// How long a sleep slice may be: the stop flag is checked between slices, so this bounds how long
/// stopping takes.
pub const SLICE: Duration = Duration::from_millis(100);

/// Why an emitter could not be spawned.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("spawn the emitter thread: {0}")]
    Thread(std::io::Error),
}

/// Start emitting into `out_dir`. The worker owns a copy of `cfg`; `stream` tells the draws of
/// two emitters apart when the clock seeds them (a config with a `seed` replays the same lines).
pub fn spawn(cfg: EmitterConfig, out_dir: PathBuf, stream: u64) -> Result<EmitterHandle, SpawnError> {
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(EmitterStatus::default()));
    let worker_stop = stop.clone();
    let worker_status = status.clone();
    let join = std::thread::Builder::new()
        .name(format!("maquinar-emit-{}", cfg.name))
        .spawn(move || run(cfg, out_dir, worker_stop, worker_status, stream))
        .map_err(SpawnError::Thread)?;
    Ok(EmitterHandle { stop, status, join: Some(join) })
}

/// The most files skipped on one rotation before the emitter gives up looking for a free name.
pub const MAX_SKIPS: u32 = 10_000;

/// The emitter apart from its thread: one line per [`Writer::write_at`], at the clock given.
///
/// The worker calls it with the wall clock; a test calls it with any instant, so a line written
/// at 23:59:59.999 is seen to land in the file of the date it carries. One `now` decides the
/// file and stamps the line: read twice, a line at midnight could land in the previous day's
/// file with the next day's timestamp.
pub struct Writer {
    cfg: EmitterConfig,
    out_dir: PathBuf,
    rng: Pcg32,
    rotor: Rotor,
    open: Option<(String, std::fs::File)>,
    max_skips: u32,
    status: EmitterStatus,
}

impl Writer {
    pub fn new(cfg: EmitterConfig, out_dir: PathBuf, stream: u64) -> Writer {
        let rng = match cfg.seed {
            Some(seed) => Pcg32::new(seed, stream),
            None => Pcg32::from_entropy(stream),
        };
        Writer {
            cfg,
            out_dir,
            rng,
            rotor: Rotor::new(1),
            open: None,
            max_skips: MAX_SKIPS,
            status: EmitterStatus::default(),
        }
    }

    /// How many existing names are skipped before the writer gives up ([`MAX_SKIPS`] unless
    /// said otherwise; a test lowers it).
    pub fn set_max_skips(&mut self, max: u32) {
        self.max_skips = max;
    }

    pub fn status(&self) -> &EmitterStatus {
        &self.status
    }

    /// Write one line as of `now`: decide the file (never one that already exists, unless this
    /// run opened it itself), open it if needed, write and flush. Trouble goes into the status
    /// and the next call tries again.
    pub fn write_at(&mut self, now: &DateTime<Local>) {
        let (mut name, rotated) = match self.rotor.file_for(&self.cfg.file, now, self.cfg.max_lines) {
            Ok(v) => v,
            Err(e) => {
                self.status.last_error = Some(e.into());
                return;
            }
        };
        if rotated {
            self.open = None;
        }
        if self.open.as_ref().map(|(n, _)| n != &name).unwrap_or(true) {
            // No file open under this name: one is created new, never opened by name. Whether
            // this run wrote the name before a day change, an earlier run left it, a folder
            // that was away came back with it, another writer took it this very instant, or
            // this run's own file was lost after a failed write, a file that exists is not this
            // run's to append to: the name is skipped. The creation and the claim are one step,
            // and nothing is written when no name is free.
            let mut skips = 0;
            let file = loop {
                match open_file(&self.out_dir, &name, self.cfg.encoding) {
                    Ok(f) => break f,
                    Err(EmitError::Io { message, .. }) if message.starts_with(EXISTS) => {
                        if skips >= self.max_skips {
                            self.open = None;
                            self.status.current_file = None;
                            self.status.lines_in_file = 0;
                            self.status.last_error = Some(EmitError::NoFreeName { tried: skips + 1 });
                            return;
                        }
                        name = match self.rotor.skip(&self.cfg.file, now) {
                            Ok(n) => n,
                            Err(e) => {
                                self.status.last_error = Some(e.into());
                                return;
                            }
                        };
                        skips += 1;
                    }
                    Err(e) => {
                        self.open = None;
                        self.status.last_error = Some(e);
                        return;
                    }
                }
            };
            self.open = Some((name.clone(), file));
            self.status.current_file = Some(name);
            self.status.lines_in_file = 0;
            self.status.last_error = None;
        }
        if let Some((name, file)) = self.open.as_mut() {
            let values = draw_values(&self.cfg.values, &mut self.rng);
            let line = render_line(&self.cfg.template, &self.cfg.ts_format, now, &values);
            match write_line(file, &line, self.cfg.encoding) {
                Ok(()) => {
                    self.rotor.note_line();
                    self.status.total_lines += 1;
                    self.status.lines_in_file = self.rotor.lines_in_file();
                    self.status.last_error = None;
                }
                Err(e) => {
                    // The file is left: the next line takes a fresh name, so a file that was
                    // lost, replaced or taken by another writer meanwhile is never appended to.
                    let path = self.out_dir.join(name.as_str());
                    self.release();
                    self.status.last_error = Some(EmitError::io("writing", &path, &e));
                }
            }
        }
    }

    /// Leave the current file: the next line is written to a fresh name, the index moved on. What
    /// a failed write does, and what a bench may ask for to rotate on demand.
    pub fn release(&mut self) {
        if self.open.take().is_some() {
            // The rotor keeps naming this file until a count or a day moves it on; the next
            // name must be a new one.
            let _ = self.rotor.skip(&self.cfg.file, &Local::now());
        }
    }
}

fn run(cfg: EmitterConfig, out_dir: PathBuf, stop: Arc<AtomicBool>, status: Arc<Mutex<EmitterStatus>>, stream: u64) {
    let interval = Duration::from_millis(cfg.interval_ms);
    let mut writer = Writer::new(cfg, out_dir, stream);
    while !stop.load(Ordering::Relaxed) {
        writer.write_at(&Local::now());
        if let Ok(mut s) = status.lock() {
            *s = writer.status().clone();
        }
        // Sleep the interval in slices so stopping is prompt.
        let mut left = interval;
        while !left.is_zero() && !stop.load(Ordering::Relaxed) {
            let slice = left.min(SLICE);
            std::thread::sleep(slice);
            left -= slice;
        }
    }
}

/// How an open of a file that must not exist yet reports that it does.
const EXISTS: &str = "already exists: ";

/// Create a file to append to. It must not exist yet: it is created new, so two writers racing
/// for one name cannot both take it, and one that exists is reported with [`EXISTS`] first in
/// the message. A file is never opened by name.
fn open_file(dir: &Path, name: &str, encoding: Encoding) -> Result<std::fs::File, EmitError> {
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(dir).map_err(|e| EmitError::io("creating", dir, &e))?;
    }
    let path = dir.join(name);
    let mut file = std::fs::OpenOptions::new().append(true).create_new(true).open(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            EmitError::Io { action: "creating", path: path.clone(), message: format!("{EXISTS}{e}") }
        } else {
            EmitError::io("creating", &path, &e)
        }
    })?;
    if encoding == Encoding::Utf16le {
        // A UTF-16 plant file carries a byte-order mark; a reader sniffs it.
        file.write_all(&[0xFF, 0xFE]).map_err(|e| EmitError::io("writing the BOM to", &path, &e))?;
    }
    Ok(file)
}

/// CRLF, flushed per line, as the plant loggers this stands in for write: the line and its
/// ending in one write, so no other writer's bytes can land between them.
fn write_line(file: &mut std::fs::File, line: &str, encoding: Encoding) -> std::io::Result<()> {
    let bytes = match encoding {
        Encoding::Utf8 => {
            let mut b = Vec::with_capacity(line.len() + 2);
            b.extend_from_slice(line.as_bytes());
            b.extend_from_slice(b"\r\n");
            b
        }
        Encoding::Utf16le => {
            let mut b = Vec::with_capacity(line.len() * 2 + 4);
            for unit in line.encode_utf16().chain([0x000D, 0x000A]) {
                b.extend_from_slice(&unit.to_le_bytes());
            }
            b
        }
    };
    file.write_all(&bytes)?;
    file.flush()
}
