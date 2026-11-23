//! Search over logs at rest (P-019; Veredas 1's grep tab engine, carried as a library for the
//! workbench that shows it, `docs/oficina-plan.md`).
//!
//! A query is one or more patterns, each a `regex` (the suite's one syntax: Veredas 1's
//! `fancy-regex` fallback is not carried) or a literal, combined per line: every one, any one, or
//! an odd number of them holding, each test negatable. The scope is a folder tree filtered by file
//! name masks, an explicit list of files searched in order, or a log family resolved in its
//! rotation order. Every file is decoded as the live tail decodes it (UTF-8, UTF-16 in either
//! order with or without a mark, CP1252; C-035), so a plant whose tails read right also greps
//! right; a file that looks binary is skipped unless asked for, one past a size limit is skipped
//! and named.
//!
//! Results stream as [`GrepEvent`]s from a worker thread while the search runs: the matching
//! lines with their spans (tagged by the pattern that made them, for a highlighter), the context
//! lines around them and a break between blocks (grep's `--`), one event per file done, and a
//! last [`GrepEvent::Done`] with the totals and why the search stopped (completed, cancelled, or
//! the total cap reached). The [`GrepHandle`] reads the live totals and cancels.
//!
//! **Read-only by design**: every file is opened for reading alone, with full sharing, so a
//! plant's writer is never held up; there is no replace and never will be.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use regex::{Regex, RegexBuilder};
use thiserror::Error;
use veredas_core::SourceError;

use crate::encoding::{DetectedEncoding, TextDecoder, detect};
use crate::family::FamilySpec;

/// A query or a scope refused before anything is read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GrepError {
    #[error("pattern: {0}")]
    Pattern(String),
    #[error("scope: {0}")]
    Scope(String),
}

/// How a query's patterns combine into one verdict per line: a pattern's test is whether it
/// matches the line, inverted when the pattern is negated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Combine {
    /// Every test holds.
    #[default]
    And,
    /// At least one holds.
    Or,
    /// An odd number hold (for two patterns: exactly one).
    Xor,
}

/// One pattern of a query; `negated` inverts its test (NOT).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrepPattern {
    pub pattern: String,
    pub negated: bool,
}

/// What to search for: the patterns, how they combine, and the options every pattern shares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrepQuery {
    pub patterns: Vec<GrepPattern>,
    pub combine: Combine,
    /// Every pattern is literal text, escaped before it is compiled.
    pub literal: bool,
    pub case_insensitive: bool,
    /// A match must stand between word boundaries.
    pub whole_word: bool,
}

impl GrepQuery {
    /// One pattern that must match.
    pub fn single(pattern: impl Into<String>) -> GrepQuery {
        GrepQuery { patterns: vec![GrepPattern { pattern: pattern.into(), negated: false }], ..GrepQuery::default() }
    }
}

/// The knobs of a search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrepOptions {
    /// Context lines before and after each matching line (grep's -B and -A).
    pub context_before: usize,
    pub context_after: usize,
    /// A file longer than this many bytes is skipped and named; none searches every size.
    pub max_file_size: Option<u64>,
    /// Search a file that looks binary (a NUL byte in its head, a UTF-16 file aside); off, it is
    /// skipped and counted.
    pub search_binary: bool,
    /// A file stops being read after this many matching lines (its `FileDone` says so).
    pub max_matches_per_file: Option<u64>,
    /// The search stops after this many matching lines in all (its `Done` says so).
    pub max_total_matches: Option<u64>,
}

/// Where to search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrepScope {
    /// Folders walked in name order; `masks` are file-name globs (`*` any run, `?` one
    /// character), case-insensitive, none for every file; `recursive` off reads a root's own
    /// files alone.
    Tree { roots: Vec<PathBuf>, masks: Vec<String>, recursive: bool },
    /// Files searched in this order.
    Files(Vec<PathBuf>),
}

impl GrepScope {
    /// A log family as it stands now, its members in rotation order, oldest first, as the log
    /// itself reads.
    pub fn from_family(spec: &FamilySpec) -> Result<GrepScope, SourceError> {
        Ok(GrepScope::Files(spec.resolve()?.into_iter().map(|f| f.path).collect()))
    }
}

/// A match's byte range in its line's text, and the index of the pattern that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub pattern: usize,
}

/// A matching line or a context line, in its file's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrepLine {
    pub path: Arc<PathBuf>,
    /// From 1.
    pub line_number: u64,
    /// False for a context line.
    pub is_match: bool,
    pub text: String,
    /// The matches of the patterns that are not negated, sorted, never overlapping; none on a
    /// context line.
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrepEvent {
    Line(GrepLine),
    /// A gap between two blocks of lines in one file (grep's `--`).
    Break { path: Arc<PathBuf> },
    /// A file done that held at least one matching line; `capped` when the per-file cap cut it.
    FileDone { path: Arc<PathBuf>, matches: u64, capped: bool },
    /// A file that could not be read; the search goes on.
    FileError { path: Arc<PathBuf>, error: String },
    /// A file past the size limit, not read.
    SkippedLarge { path: Arc<PathBuf>, size: u64 },
    /// A folder that could not be listed; the search goes on.
    WalkError { error: String },
    /// Always the last event.
    Done(GrepSummary),
}

/// Why a search stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrepStop {
    Completed,
    Cancelled,
    MatchCap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GrepStats {
    pub files_searched: u64,
    pub files_matched: u64,
    pub matches: u64,
    pub errors: u64,
    pub skipped_large: u64,
    pub skipped_binary: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrepSummary {
    pub stats: GrepStats,
    pub elapsed_ms: u64,
    pub stopped: GrepStop,
}

/// Shared by a running search and its owner: the live totals and the cancel switch.
#[derive(Debug, Default)]
pub struct GrepHandle {
    cancel: AtomicBool,
    files_searched: AtomicU64,
    files_matched: AtomicU64,
    matches: AtomicU64,
    errors: AtomicU64,
    skipped_large: AtomicU64,
    skipped_binary: AtomicU64,
}

impl GrepHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> GrepStats {
        GrepStats {
            files_searched: self.files_searched.load(Ordering::Relaxed),
            files_matched: self.files_matched.load(Ordering::Relaxed),
            matches: self.matches.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            skipped_large: self.skipped_large.load(Ordering::Relaxed),
            skipped_binary: self.skipped_binary.load(Ordering::Relaxed),
        }
    }
}

/// A running or finished search: read `events` until [`GrepEvent::Done`]. Dropping it mid-run
/// ends the worker at its next event.
pub struct GrepSearch {
    pub handle: Arc<GrepHandle>,
    pub events: Receiver<GrepEvent>,
    pub worker: JoinHandle<()>,
}

/// A query's patterns compiled once.
struct Compiled {
    patterns: Vec<(Regex, bool)>,
    combine: Combine,
}

impl Compiled {
    fn new(q: &GrepQuery) -> Result<Compiled, GrepError> {
        if q.patterns.is_empty() {
            return Err(GrepError::Pattern("the query has no pattern".into()));
        }
        let mut patterns = Vec::with_capacity(q.patterns.len());
        for (i, p) in q.patterns.iter().enumerate() {
            if p.pattern.is_empty() {
                return Err(GrepError::Pattern(format!("pattern {} is empty", i + 1)));
            }
            let body = if q.literal { regex::escape(&p.pattern) } else { p.pattern.clone() };
            let build = |body: &str| RegexBuilder::new(body).case_insensitive(q.case_insensitive).build();
            let refused = |e: regex::Error| GrepError::Pattern(format!("pattern {} `{}`: {e}", i + 1, p.pattern));
            // Checked bare first: a refusal speaks of the operator's own text, and a group the
            // pattern leaves unbalanced (`a)|(?:b`) cannot escape a wrapper around it.
            let bare = build(&body).map_err(refused)?;
            // A whole word stands apart from word characters on both sides, whatever the pattern's
            // own ends are (`-5` alone in `offset -5 mm`, not glued in `a-5 b`), as Veredas 1's
            // engine read it. A valid pattern wraps whole, but for a comment of its extended mode
            // (`(?x)run # word`), which runs to the line's end and would swallow the wrapper's
            // close: a line end closes the comment first, whitespace in that mode.
            let re = if q.whole_word {
                build(&format!(r"\b{{start-half}}(?:{body})\b{{end-half}}"))
                    .or_else(|_| build(&format!("\\b{{start-half}}(?:{body}\n)\\b{{end-half}}")))
                    .map_err(refused)?
            } else {
                bare
            };
            patterns.push((re, p.negated));
        }
        Ok(Compiled { patterns, combine: q.combine })
    }

    /// Whether a line matches the query.
    fn verdict(&self, line: &str) -> bool {
        let mut tests = self.patterns.iter().map(|(re, negated)| re.is_match(line) != *negated);
        match self.combine {
            Combine::And => tests.all(|t| t),
            Combine::Or => tests.any(|t| t),
            Combine::Xor => tests.filter(|t| *t).count() % 2 == 1,
        }
    }

    /// The spans of a matching line: every match of a pattern that is not negated, sorted by
    /// start (the longer first on a tie), an overlap with an earlier span dropped.
    fn spans(&self, line: &str) -> Vec<Span> {
        let mut all: Vec<Span> = Vec::new();
        for (i, (re, negated)) in self.patterns.iter().enumerate() {
            if !negated {
                all.extend(re.find_iter(line).filter(|m| m.end() > m.start()).map(|m| Span { start: m.start(), end: m.end(), pattern: i }));
            }
        }
        all.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut out: Vec<Span> = Vec::with_capacity(all.len());
        for s in all {
            if out.last().is_none_or(|l| s.start >= l.end) {
                out.push(s);
            }
        }
        out
    }
}

/// Start a search on a worker thread. The query and the scope are checked first: a pattern that
/// is empty or no `regex`, a query with none, a tree with no root, are refused before anything
/// is read.
pub fn search(scope: GrepScope, query: &GrepQuery, options: GrepOptions) -> Result<GrepSearch, GrepError> {
    let compiled = Compiled::new(query)?;
    if let GrepScope::Tree { roots, .. } = &scope
        && roots.is_empty()
    {
        return Err(GrepError::Scope("a tree with no root".into()));
    }
    let handle = Arc::new(GrepHandle::default());
    let (tx, events) = channel();
    let h = Arc::clone(&handle);
    let worker = std::thread::Builder::new()
        .name("portal-grep".into())
        .spawn(move || run(scope, compiled, options, h, tx))
        .map_err(|e| GrepError::Scope(format!("the search's thread did not start: {e}")))?;
    Ok(GrepSearch { handle, events, worker })
}

/// The bytes read of a file's head to tell its encoding and whether it is binary.
const HEAD: usize = 8 * 1024;

fn run(scope: GrepScope, q: Compiled, o: GrepOptions, h: Arc<GrepHandle>, tx: Sender<GrepEvent>) {
    let started = Instant::now();
    let mut stopped = GrepStop::Completed;
    let files: Box<dyn Iterator<Item = Result<PathBuf, String>>> = match scope {
        GrepScope::Files(list) => Box::new(list.into_iter().map(Ok)),
        GrepScope::Tree { roots, masks, recursive } => Box::new(walk(roots, masks, recursive).into_iter()),
    };
    for item in files {
        if h.is_cancelled() {
            stopped = GrepStop::Cancelled;
            break;
        }
        let path = match item {
            Ok(p) => Arc::new(p),
            Err(error) => {
                h.errors.fetch_add(1, Ordering::Relaxed);
                if tx.send(GrepEvent::WalkError { error }).is_err() {
                    return;
                }
                continue;
            }
        };
        match search_file(&path, &q, &o, &h, &tx) {
            Ok(FileOutcome::Searched) => {}
            Ok(FileOutcome::TotalCap) => {
                stopped = GrepStop::MatchCap;
                break;
            }
            Ok(FileOutcome::Cancelled) => {
                stopped = GrepStop::Cancelled;
                break;
            }
            Err(Gone) => return,
        }
    }
    let _ = tx.send(GrepEvent::Done(GrepSummary { stats: h.stats(), elapsed_ms: started.elapsed().as_millis() as u64, stopped }));
}

enum FileOutcome {
    Searched,
    TotalCap,
    Cancelled,
}

/// The receiver is gone: the owner dropped the search.
struct Gone;

fn send(tx: &Sender<GrepEvent>, e: GrepEvent) -> Result<(), Gone> {
    tx.send(e).map_err(|_| Gone)
}

/// As many bytes as `buf` holds, or the file's end: a slice of the tail's size, whatever short
/// reads the drive gives.
fn read_slice(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn search_file(path: &Arc<PathBuf>, q: &Compiled, o: &GrepOptions, h: &GrepHandle, tx: &Sender<GrepEvent>) -> Result<FileOutcome, Gone> {
    let failed = |e: std::io::Error| -> Result<FileOutcome, Gone> {
        h.errors.fetch_add(1, Ordering::Relaxed);
        send(tx, GrepEvent::FileError { path: Arc::clone(path), error: e.to_string() })?;
        Ok(FileOutcome::Searched)
    };
    let opened = std::fs::File::open(path.as_path()).and_then(|f| f.metadata().map(|m| (f, m.len())));
    let (mut f, size) = match opened {
        Ok(r) => r,
        Err(e) => return failed(e),
    };
    if o.max_file_size.is_some_and(|max| size > max) {
        h.skipped_large.fetch_add(1, Ordering::Relaxed);
        send(tx, GrepEvent::SkippedLarge { path: Arc::clone(path), size })?;
        return Ok(FileOutcome::Searched);
    }
    // Read and decoded in the tail's own slices (C-035): the decoder settles UTF-8 or CP1252 on
    // the slice that holds the first byte past ASCII, so a file at rest decodes here as a tail
    // reading it whole from its start decodes it. A cancel is heard at every slice.
    let mut buf = vec![0u8; crate::tail::READ_CHUNK];
    let first = match read_slice(&mut f, &mut buf) {
        Ok(n) => n,
        Err(e) => return failed(e),
    };
    let head = &buf[..first.min(HEAD)];
    let utf16 = matches!(detect(head), DetectedEncoding::Utf16Le | DetectedEncoding::Utf16Be);
    if !utf16 && !o.search_binary && head.contains(&0) {
        h.skipped_binary.fetch_add(1, Ordering::Relaxed);
        return Ok(FileOutcome::Searched);
    }
    h.files_searched.fetch_add(1, Ordering::Relaxed);
    let mut text = String::with_capacity(size as usize);
    let mut decoder = TextDecoder::new();
    decoder.feed(&buf[..first], &mut text);
    let mut more = first == buf.len();
    while more {
        if h.is_cancelled() {
            return Ok(FileOutcome::Cancelled);
        }
        let n = match read_slice(&mut f, &mut buf) {
            Ok(n) => n,
            Err(e) => return failed(e),
        };
        decoder.feed(&buf[..n], &mut text);
        more = n == buf.len();
    }
    decoder.finish(&mut text);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

    let mut matches = 0u64;
    let mut before: VecDeque<(u64, &str)> = VecDeque::with_capacity(o.context_before + 1);
    let mut after_left = 0usize;
    let mut last_emitted: Option<u64> = None;
    let mut capped = false;
    let mut total_cap = false;
    let mut lines = text.split('\n').peekable();
    let mut number = 0u64;
    while let Some(raw) = lines.next() {
        if lines.peek().is_none() && raw.is_empty() {
            break; // the text ended on a line feed: no line after it
        }
        number += 1;
        if number.is_multiple_of(4096) && h.is_cancelled() {
            return Ok(FileOutcome::Cancelled);
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let hit = !capped && !total_cap && q.verdict(line);
        if hit {
            let first = before.front().map(|(n, _)| *n).unwrap_or(number);
            // A break between two blocks of a file, when context is asked (grep's `--`).
            if (o.context_before > 0 || o.context_after > 0) && last_emitted.is_some_and(|l| first > l + 1) {
                send(tx, GrepEvent::Break { path: Arc::clone(path) })?;
            }
            for (n, t) in before.drain(..) {
                send(tx, GrepEvent::Line(GrepLine { path: Arc::clone(path), line_number: n, is_match: false, text: t.to_string(), spans: Vec::new() }))?;
            }
            send(tx, GrepEvent::Line(GrepLine { path: Arc::clone(path), line_number: number, is_match: true, text: line.to_string(), spans: q.spans(line) }))?;
            last_emitted = Some(number);
            after_left = o.context_after;
            matches += 1;
            let total = h.matches.fetch_add(1, Ordering::Relaxed) + 1;
            if o.max_matches_per_file.is_some_and(|cap| matches >= cap) {
                capped = true;
            }
            if o.max_total_matches.is_some_and(|cap| total >= cap) {
                total_cap = true;
            }
        } else if after_left > 0 {
            send(tx, GrepEvent::Line(GrepLine { path: Arc::clone(path), line_number: number, is_match: false, text: line.to_string(), spans: Vec::new() }))?;
            last_emitted = Some(number);
            after_left -= 1;
        } else if capped || total_cap {
            break;
        } else if o.context_before > 0 {
            if before.len() == o.context_before {
                before.pop_front();
            }
            before.push_back((number, line));
        }
    }
    if matches > 0 {
        h.files_matched.fetch_add(1, Ordering::Relaxed);
        send(tx, GrepEvent::FileDone { path: Arc::clone(path), matches, capped })?;
    }
    Ok(if total_cap { FileOutcome::TotalCap } else { FileOutcome::Searched })
}

/// The files of a tree, in name order, each root in turn; a folder that cannot be listed is an
/// error in its place.
fn walk(roots: Vec<PathBuf>, masks: Vec<String>, recursive: bool) -> Vec<Result<PathBuf, String>> {
    let masks: Vec<String> = masks.iter().map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty()).collect();
    let mut out = Vec::new();
    fn visit(dir: &Path, masks: &[String], recursive: bool, out: &mut Vec<Result<PathBuf, String>>) {
        // A link, to a folder or to a file, is not followed (Veredas 1's walk, `follow_links(false)`):
        // a `loop -> .` would search the tree again and again, a `latest -> 2026-11` twice.
        let mut entries: Vec<(PathBuf, std::fs::FileType)> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.flatten().filter_map(|e| e.file_type().ok().map(|t| (e.path(), t))).collect(),
            Err(e) => {
                out.push(Err(format!("{}: {e}", dir.display())));
                return;
            }
        };
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (p, kind) in entries {
            if kind.is_dir() {
                if recursive {
                    visit(&p, masks, recursive, out);
                }
            } else if kind.is_file() {
                let name: Vec<char> = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default().chars().collect();
                if masks.is_empty() || masks.iter().any(|m| glob(&m.chars().collect::<Vec<char>>(), &name)) {
                    out.push(Ok(p));
                }
            }
        }
    }
    for root in &roots {
        if root.is_file() {
            out.push(Ok(root.clone()));
        } else {
            visit(root, &masks, recursive, &mut out);
        }
    }
    out
}

/// A file-name glob over characters: `*` any run of them, `?` one (an accented letter one), anything
/// else itself.
fn glob(pattern: &[char], name: &[char]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some('*'), _) => glob(&pattern[1..], name) || (!name.is_empty() && glob(pattern, &name[1..])),
        (Some('?'), Some(_)) => glob(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => glob(&pattern[1..], &name[1..]),
        _ => false,
    }
}
