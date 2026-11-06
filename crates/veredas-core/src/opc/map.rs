//! One poll into rows and samples.
//!
//! Kept apart from the worker so it is testable without a server: given a set of tag reads, this
//! decides what every rule's row looks like. The worker's job is to obtain values; this one's is
//! to be right about them.

use std::collections::HashMap;
use std::sync::Arc;

use opcua_types::{NodeId, Variant};

use crate::opc::spec::{PollSpec, RuleSpec};
use crate::opc::value::{ValueError, bit_field, opc_to_value};
use crate::opc::worker::TagRead;
use crate::{Sample, SourceId, Value};

/// One rule's row for one poll: values positional against [`RuleSpec::columns`], `None` where a
/// tag was bad, absent from the poll, or refused its type.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleRow {
    /// Index of the rule within its [`PollSpec`].
    pub rule_index: usize,
    pub values: Vec<Option<Value>>,
}

/// What one poll produced.
#[derive(Debug, Default)]
pub struct Mapped {
    /// One row per rule, in spec order (a `store_on_change` rule that did not change has none).
    pub rows: Vec<RuleRow>,
    /// Every value as a sample, in row order.
    pub samples: Vec<Sample>,
    /// Values that could not be mapped, as `(column, why)`: reported, never silently dropped.
    pub failures: Vec<(String, String)>,
}

/// The on-change heartbeat: an unchanged `store_on_change` rule still yields one row every
/// this-many polls, so a window over a quiet hour has points in it and a stale source stays
/// distinguishable from a value that never moved.
pub const HEARTBEAT_POLLS: u32 = 60;

/// Per-source memory for `store_on_change` rules: the last kept values per rule, and how many
/// polls each rule has skipped since.
#[derive(Debug, Default)]
pub struct DedupState {
    last: Vec<Option<Vec<Option<Value>>>>,
    skipped: Vec<u32>,
}

/// Map one poll into rule rows and samples, every rule kept.
///
/// `reads` is looked up by node id, not by position, so a rule referencing a shared tag reads
/// the same value without re-fetching; a tag whose node is absent from `reads` is a hole.
pub fn map_poll(spec: &PollSpec, reads: &[TagRead], index_ms: i64, seq: u64) -> Mapped {
    map_poll_dedup(spec, reads, index_ms, seq, None)
}

/// [`map_poll`], keeping transitions: a rule marked `store_on_change` whose values equal its
/// last kept poll yields nothing this time (no row, no samples) until the heartbeat is due.
/// Failures are reported either way: silence must never hide a bad tag. With `dedup` `None`
/// every rule is kept.
pub fn map_poll_dedup(
    spec: &PollSpec,
    reads: &[TagRead],
    index_ms: i64,
    seq: u64,
    mut dedup: Option<&mut DedupState>,
) -> Mapped {
    let by_node: HashMap<&NodeId, &TagRead> = reads.iter().map(|r| (&r.node_id, r)).collect();
    let source = SourceId::new(&spec.source_id);
    let mut out = Mapped::default();
    if let Some(d) = dedup.as_deref_mut() {
        d.last.resize(spec.rules.len(), None);
        d.skipped.resize(spec.rules.len(), 0);
    }

    for (rule_index, rule) in spec.rules.iter().enumerate() {
        let mut values: Vec<Option<Value>> = Vec::new();
        let mut rule_samples: Vec<Sample> = Vec::new();
        for tag in &rule.tags {
            let read = by_node.get(&tag.node_id);
            // An empty variant is no value, whatever the status beside it.
            let raw: Option<&Variant> =
                read.and_then(|r| r.value.as_ref()).filter(|v| !matches!(v, Variant::Empty));

            // A read with no value is information: report it once per tag, on the tag's first
            // column (its own, else its first bit field), so the operator sees WHICH tag the
            // server is unhappy about and why, even for a word polled only for its bits.
            if let (Some(r), None) = (read, raw) {
                let first = if tag.series.is_empty() { tag.bits.first().map(|b| &b.series) } else { Some(&tag.series) };
                if let Some(column) = first {
                    // A good status that carried no value says so, rather than "Good".
                    let why = if r.status.is_good() { ValueError::Empty.to_string() } else { r.status.to_string() };
                    out.failures.push((column.clone(), why));
                }
            }

            // The tag's own column, when it has one.
            if !tag.series.is_empty() {
                let mapped = match raw {
                    None => None,
                    Some(v) => match opc_to_value(v, tag.ty) {
                        Ok(value) => Some(value),
                        Err(e) => {
                            out.failures.push((tag.series.clone(), e.to_string()));
                            None
                        }
                    },
                };
                if let Some(v) = &mapped {
                    push_sample(&mut rule_samples, &source, &tag.series, index_ms, seq, v);
                }
                values.push(mapped);
            }

            // Its bit fields.
            for bit in &tag.bits {
                let mapped = match raw {
                    None => None,
                    Some(v) => match bit_field(v, bit.bit, bit.width) {
                        Ok(value) => Some(value),
                        Err(e) => {
                            out.failures.push((bit.series.clone(), e.to_string()));
                            None
                        }
                    },
                };
                if let Some(v) = &mapped {
                    push_sample(&mut rule_samples, &source, &bit.series, index_ms, seq, v);
                }
                values.push(mapped);
            }
        }
        if rule.store_on_change
            && let Some(d) = dedup.as_deref_mut()
        {
            let unchanged = d.last[rule_index].as_ref().is_some_and(|last| same_row(last, &values));
            if unchanged && d.skipped[rule_index] + 1 < HEARTBEAT_POLLS {
                d.skipped[rule_index] += 1;
                continue; // no row, no samples: the transition is what matters
            }
            d.last[rule_index] = Some(values.clone());
            d.skipped[rule_index] = 0;
        }
        out.samples.append(&mut rule_samples);
        out.rows.push(RuleRow { rule_index, values });
    }
    out
}

/// Two rows hold the same reading: equal values, holes in the same places, and a NaN the same
/// as a NaN (a float stuck at NaN has not changed, though NaN != NaN).
fn same_row(a: &[Option<Value>], b: &[Option<Value>]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Some(Value::Float(x)), Some(Value::Float(y))) => x == y || (x.is_nan() && y.is_nan()),
            (x, y) => x == y,
        })
}

fn push_sample(
    samples: &mut Vec<Sample>,
    source: &SourceId,
    series: &str,
    index_ms: i64,
    seq: u64,
    value: &Value,
) {
    samples.push(Sample {
        source: source.clone(),
        series: Arc::from(series),
        // A tag is identified by its node; a poll measures no separately keyed thing.
        key: None,
        ts: None,
        seq,
        arrival_unix_ms: index_ms,
        value: value.clone(),
    });
}

/// One line for one row, `rule column=value ...`, a hole shown as `?`: an OPC source produces
/// no text of its own, and an operator watching a source should still SEE it doing something.
/// Always one line: a control character or a Unicode line or paragraph separator in a value (a
/// newline in a text tag) reads as a space.
pub fn render_line(rule: &RuleSpec, row: &RuleRow) -> String {
    let mut out = String::with_capacity(64);
    out.push_str(&rule.name);
    for ((name, _), value) in rule.columns().into_iter().zip(&row.values) {
        out.push(' ');
        out.push_str(&name);
        out.push('=');
        match value {
            Some(v) => out.push_str(&v.to_string()),
            None => out.push('?'),
        }
    }
    out.chars().map(crate::opc::flat).collect()
}
