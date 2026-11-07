//! The stamp a line carries: a search pattern locating it in the line, and ordered chrono
//! format candidates to parse it with.
//!
//! A per-source rule rather than a capture rule's: ordering, the clock-skew monitor and a
//! backfill's start point all need line stamps before any capture rule exists. A parse failure
//! is never an error: the line still flows, ordered by its sequence number.

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;

use veredas_core::SourceError;

/// A chrono format may describe a whole date and time or a date alone (a file name's stamp
/// often carries no time). A date alone resolves to midnight, so everything orders on one axis.
pub(crate) fn parse_flexible(s: &str, fmt: &str) -> Option<NaiveDateTime> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
        return Some(dt);
    }
    // A name carries what it carries: a year alone is its first day, a month its first, an hour
    // without minutes the start of the hour. The missing fields are filled with the first value,
    // never guessed beyond that, so names of every granularity order on one axis.
    let has = |f: &str, c: char| f.contains(&format!("%{c}"));
    let (mut text, mut f) = (s.to_string(), fmt.to_string());
    if (has(&f, 'Y') || has(&f, 'y')) && !has(&f, 'm') {
        text.push_str(" 01");
        f.push_str(" %m");
    }
    if has(&f, 'm') && !has(&f, 'd') {
        text.push_str(" 01");
        f.push_str(" %d");
    }
    if has(&f, 'H') && !has(&f, 'M') {
        text.push_str(" 00");
        f.push_str(" %M");
    }
    if f != fmt {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&text, &f) {
            return Some(dt);
        }
        if let Ok(d) = NaiveDate::parse_from_str(&text, &f) {
            return d.and_hms_opt(0, 0, 0);
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        return d.and_hms_opt(0, 0, 0);
    }
    None
}

/// The configuration half: the pattern and the format candidates. Cheap to clone into workers.
#[derive(Clone, Debug)]
pub struct TimestampSpec {
    pattern: Regex,
    formats: Vec<String>,
}

impl TimestampSpec {
    /// `pattern` locates the stamp: its `(?P<ts>…)` group when it has one, else the whole match.
    /// `formats` are chrono formats tried in order; a date-only format resolves to midnight.
    pub fn new(
        pattern: &str,
        formats: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, SourceError> {
        let pattern = Regex::new(pattern)?;
        let formats: Vec<String> = formats.into_iter().map(Into::into).collect();
        if formats.is_empty() {
            return Err(SourceError::Spec("timestamp spec needs at least one chrono format".into()));
        }
        Ok(TimestampSpec { pattern, formats })
    }

    /// The formats, in order.
    pub fn formats(&self) -> &[String] {
        &self.formats
    }

    /// Stateless extraction, for peeks and one-off parses. A worker's hot path goes through a
    /// [`TimestampExtractor`].
    pub fn extract(&self, line: &str) -> Option<NaiveDateTime> {
        self.extract_with_hint(line, &mut 0)
    }

    /// Try the hinted format first, then the rest in configured order. Once a format succeeds it
    /// stays preferred: a source's stream is format-consistent, and this keeps an ambiguous
    /// stamp (a day and month that swap) resolving the same way for the whole stream.
    fn extract_with_hint(&self, line: &str, hint: &mut usize) -> Option<NaiveDateTime> {
        let caps = self.pattern.captures(line)?;
        let text = match caps.name("ts") {
            Some(m) => m.as_str(),
            None => caps.get(0)?.as_str(),
        };
        let n = self.formats.len();
        for off in 0..n {
            let i = (*hint + off) % n;
            if let Some(dt) = parse_flexible(text, &self.formats[i]) {
                *hint = i;
                return Some(dt);
            }
        }
        None
    }
}

/// The runtime half: a spec and its moving format hint. One per source worker.
pub struct TimestampExtractor {
    spec: TimestampSpec,
    hint: usize,
}

impl TimestampExtractor {
    pub fn new(spec: TimestampSpec) -> Self {
        TimestampExtractor { spec, hint: 0 }
    }

    pub fn extract(&mut self, line: &str) -> Option<NaiveDateTime> {
        self.spec.extract_with_hint(line, &mut self.hint)
    }
}
