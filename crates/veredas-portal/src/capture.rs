//! Capture rules: a regex with named groups turns a matching line into typed [`Sample`]s. Each
//! named group is a series, typed by configuration (text, int, float, time); one group may be
//! the identity key (a sensor, an item) that pairing and per-key windows key on.
//!
//! A value that refuses its configured type does not kill the line: it is counted, and the
//! rule's editor shows the count. Time-typed groups parse with the source's ordered chrono
//! format candidates, the same per-source list the line-stamp rule uses.

use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use thiserror::Error;

use veredas_core::{Sample, SeriesType, SourceId, Value};

use crate::source::LineEvent;
use crate::timestamp::parse_flexible;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("rule `{0}`: {1}")]
    Pattern(String, String),
    #[error("rule `{rule}`: key group `{key}` is not a named group of the pattern")]
    KeyGroup { rule: String, key: String },
    #[error("rule `{0}`: the pattern has no named groups, so nothing to capture")]
    NoGroups(String),
}

/// The named groups of a pattern, in pattern order; empty when the pattern is invalid. Lets a
/// rule editor offer group pickers while the pattern is still being typed.
pub fn named_groups(pattern: &str) -> Vec<String> {
    match Regex::new(pattern) {
        Ok(re) => re.capture_names().flatten().map(String::from).collect(),
        Err(_) => Vec::new(),
    }
}

/// A compiled capture rule. The series names are the group names: keep them unique across the
/// rules that share a source.
pub struct CaptureRule {
    name: String,
    /// `None` applies to every source.
    scope: Option<SourceId>,
    regex: Regex,
    /// The value groups in pattern order (the key group is not a value).
    groups: Vec<(Arc<str>, SeriesType)>,
    key_group: Option<String>,
}

/// What one line yielded.
#[derive(Debug, Default)]
pub struct CaptureOutcome {
    pub samples: Vec<Sample>,
    /// The identity key captured from the line, when the rule has a key group: carried out so a
    /// wide row keeps its key even when every value group failed its type.
    pub key: Option<Arc<str>>,
    /// The pattern matched the line (the samples may still be empty: every group failed its
    /// type, or was optional and absent).
    pub matched: bool,
    /// Groups whose text refused their configured type.
    pub coercion_failures: u32,
}

impl CaptureRule {
    /// `types` maps group names to types (a missing one is text); `key_group` names the identity
    /// group, which emits no value.
    pub fn new(
        name: &str,
        pattern: &str,
        scope: Option<SourceId>,
        types: &HashMap<String, SeriesType>,
        key_group: Option<&str>,
    ) -> Result<CaptureRule, CaptureError> {
        let name = name.to_string();
        let regex = Regex::new(pattern).map_err(|e| CaptureError::Pattern(name.clone(), e.to_string()))?;
        let names: Vec<String> = regex.capture_names().flatten().map(String::from).collect();
        if names.is_empty() {
            return Err(CaptureError::NoGroups(name));
        }
        if let Some(key) = key_group
            && !names.iter().any(|n| n == key)
        {
            return Err(CaptureError::KeyGroup { rule: name, key: key.to_string() });
        }
        let groups = names
            .iter()
            .filter(|n| Some(n.as_str()) != key_group)
            .map(|n| (Arc::<str>::from(n.as_str()), types.get(n).copied().unwrap_or(SeriesType::Text)))
            .collect();
        Ok(CaptureRule { name, scope, regex, groups, key_group: key_group.map(String::from) })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn scope(&self) -> Option<&SourceId> {
        self.scope.as_ref()
    }

    /// The value series this rule can emit, in pattern order.
    pub fn series(&self) -> impl Iterator<Item = (&str, SeriesType)> {
        self.groups.iter().map(|(n, t)| (n.as_ref(), *t))
    }

    pub fn key_group(&self) -> Option<&str> {
        self.key_group.as_deref()
    }

    /// Apply to one tagged line. `time_formats` are the source's chrono candidates for
    /// time-typed groups (none means every time coercion fails, counted).
    pub fn apply(&self, ev: &LineEvent, time_formats: &[String]) -> CaptureOutcome {
        let mut out = CaptureOutcome::default();
        if let Some(scope) = &self.scope
            && scope != &ev.source
        {
            return out;
        }
        let Some(caps) = self.regex.captures(&ev.line) else {
            return out;
        };
        out.matched = true;

        let key: Option<Arc<str>> =
            self.key_group.as_deref().and_then(|k| caps.name(k)).map(|m| Arc::from(m.as_str()));
        out.key = key.clone();

        for (series, ty) in &self.groups {
            let Some(text) = caps.name(series).map(|m| m.as_str()) else {
                continue; // an optional group absent on this line
            };
            let value = match ty {
                SeriesType::Text => Some(Value::Text(Arc::from(text))),
                SeriesType::Int => text.trim().parse::<i64>().ok().map(Value::Int),
                SeriesType::Float => text.trim().parse::<f64>().ok().map(Value::Float),
                SeriesType::Time => {
                    time_formats.iter().find_map(|fmt| parse_flexible(text.trim(), fmt)).map(Value::Time)
                }
            };
            match value {
                Some(value) => out.samples.push(Sample {
                    source: ev.source.clone(),
                    series: Arc::clone(series),
                    key: key.clone(),
                    ts: ev.parsed_ts,
                    seq: ev.seq,
                    arrival_unix_ms: ev.arrival_unix_ms,
                    value,
                }),
                None => out.coercion_failures += 1,
            }
        }
        out
    }
}
