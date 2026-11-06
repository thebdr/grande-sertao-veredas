//! One poll into rows and samples (C-020). Veredas 1's tests, plus every hole reported once per
//! tag (a bits-only word included), the on-change memory at its heartbeat edge, and the rendered
//! line kept to one line.

use std::sync::Arc;
use std::time::Duration;

use veredas_core::opc::map::{DedupState, HEARTBEAT_POLLS, RuleRow, map_poll, map_poll_dedup, render_line};
use veredas_core::opc::spec::{Auth, BitSpec, PollSpec, RuleSpec, TagSpec, parse_node_id};
use veredas_core::opc::worker::TagRead;
use veredas_core::opc::{StatusCode, Variant};
use veredas_core::{SeriesType, SourceId, Value};

fn read(node: &str, value: Option<Variant>) -> TagRead {
    TagRead {
        node_id: parse_node_id(node).unwrap(),
        status: if value.is_some() { StatusCode::Good } else { StatusCode::BadNodeIdUnknown },
        value,
    }
}

fn tag(node: &str, series: &str, ty: SeriesType, bits: Vec<BitSpec>) -> TagSpec {
    TagSpec { node_id: parse_node_id(node).unwrap(), raw: node.to_string(), series: series.to_string(), ty, writable: false, bits }
}

fn bit(bit: u8, width: u8, series: &str) -> BitSpec {
    BitSpec { bit, width, series: series.into() }
}

fn rule(name: &str, store_on_change: bool, tags: Vec<TagSpec>) -> RuleSpec {
    RuleSpec { name: name.into(), tags, store_on_change }
}

fn spec(rules: Vec<RuleSpec>) -> PollSpec {
    PollSpec {
        source_id: "plc".into(),
        display_name: "PLC".into(),
        endpoint_url: "opc.tcp://x/".into(),
        auth: Auth::Anonymous,
        poll: Duration::from_millis(500),
        rules,
    }
}

fn failures(f: &[(String, String)]) -> Vec<(&str, &str)> {
    f.iter().map(|(c, w)| (c.as_str(), w.as_str())).collect()
}

#[test]
fn a_poll_becomes_one_row_per_rule_with_bits_expanded() {
    let s = spec(vec![rule(
        "line1",
        false,
        vec![
            tag("ns=1;s=t", "temp", SeriesType::Float, vec![]),
            tag("ns=1;s=w", "word", SeriesType::Int, vec![bit(0, 1, "running"), bit(4, 3, "mode")]),
        ],
    )]);
    let reads = vec![
        read("ns=1;s=t", Some(Variant::Double(21.5))),
        read("ns=1;s=w", Some(Variant::UInt16(0b0101_0001))),
    ];

    let m = map_poll(&s, &reads, 1_000, 7);
    assert_eq!(
        m.rows,
        vec![RuleRow {
            rule_index: 0,
            values: vec![
                Some(Value::Float(21.5)),
                Some(Value::Int(0b0101_0001)),
                Some(Value::Int(1)), // bit 0
                Some(Value::Int(5)), // bits 4..6
            ],
        }]
    );
    assert!(m.failures.is_empty());

    // Every mapped value is also a sample, on the poll's clock and sequence, in row order.
    type Seen<'a> = (&'a str, &'a str, Option<&'a str>, Option<chrono::NaiveDateTime>, u64, i64, Value);
    let got: Vec<Seen> = m
        .samples
        .iter()
        .map(|s| (s.source.as_str(), s.series.as_ref(), s.key.as_deref(), s.ts, s.seq, s.arrival_unix_ms, s.value.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("plc", "temp", None, None, 7, 1_000, Value::Float(21.5)),
            ("plc", "word", None, None, 7, 1_000, Value::Int(0b0101_0001)),
            ("plc", "running", None, None, 7, 1_000, Value::Int(1)),
            ("plc", "mode", None, None, 7, 1_000, Value::Int(5)),
        ]
    );
    assert_eq!(m.samples[0].source, SourceId::new("plc"));
    assert_eq!(m.samples[0].index_ms(), 1_000);
}

#[test]
fn one_bad_tag_leaves_the_rest_of_the_row_intact_and_is_reported_once() {
    let s = spec(vec![rule(
        "line1",
        false,
        vec![
            tag("ns=1;s=t", "temp", SeriesType::Float, vec![]),
            tag("ns=1;s=gone", "missing", SeriesType::Int, vec![bit(0, 1, "missing_b0"), bit(1, 1, "missing_b1")]),
            tag("ns=1;s=b", "batch", SeriesType::Text, vec![]),
        ],
    )]);
    let reads = vec![
        read("ns=1;s=t", Some(Variant::Double(21.5))),
        read("ns=1;s=gone", None),
        read("ns=1;s=b", Some(Variant::from("B-1"))),
    ];

    let m = map_poll(&s, &reads, 0, 0);
    assert_eq!(
        m.rows[0].values,
        vec![Some(Value::Float(21.5)), None, None, None, Some(Value::Text(Arc::from("B-1")))],
        "the bad tag is a hole in each of its columns, never a shift"
    );
    // Reported once, on the tag's own column, with the server's reason.
    assert_eq!(failures(&m.failures), vec![("missing", "BadNodeIdUnknown")]);
    // A hole produces no sample: nothing learns a null.
    let names: Vec<&str> = m.samples.iter().map(|s| s.series.as_ref()).collect();
    assert_eq!(names, vec!["temp", "batch"]);
}

#[test]
fn a_bad_word_polled_only_for_its_bits_is_reported_on_its_first_bit() {
    let s = spec(vec![rule(
        "flags",
        false,
        vec![tag("ns=1;s=w", "", SeriesType::Int, vec![bit(0, 1, "running"), bit(1, 1, "fault")])],
    )]);
    let mut r = read("ns=1;s=w", None);
    r.status = StatusCode::BadCommunicationError;
    let m = map_poll(&s, &[r], 0, 0);
    assert_eq!(m.rows[0].values, vec![None, None]);
    assert_eq!(failures(&m.failures), vec![("running", "BadCommunicationError")]);
    assert!(m.samples.is_empty());
}

#[test]
fn a_good_status_without_a_value_is_reported_as_no_value() {
    let s = spec(vec![rule("line1", false, vec![tag("ns=1;s=t", "temp", SeriesType::Float, vec![])])]);
    let r = TagRead { node_id: parse_node_id("ns=1;s=t").unwrap(), value: None, status: StatusCode::Good };
    let m = map_poll(&s, &[r], 0, 0);
    assert_eq!(m.rows[0].values, vec![None]);
    assert_eq!(failures(&m.failures), vec![("temp", "no value")]);
}

#[test]
fn an_empty_variant_beside_a_good_status_is_one_report_per_tag() {
    // A server answering Good with a null value for an uninitialised variable.
    let s = spec(vec![rule(
        "line1",
        false,
        vec![
            tag("ns=1;s=w", "word", SeriesType::Int, vec![bit(0, 1, "b0"), bit(1, 1, "b1")]),
            tag("ns=1;s=f", "", SeriesType::Int, vec![bit(0, 1, "running"), bit(1, 1, "fault")]),
        ],
    )]);
    let empty = |node: &str| TagRead {
        node_id: parse_node_id(node).unwrap(),
        value: Some(Variant::Empty),
        status: StatusCode::Good,
    };
    let m = map_poll(&s, &[empty("ns=1;s=w"), empty("ns=1;s=f")], 0, 0);
    assert_eq!(m.rows[0].values, vec![None, None, None, None, None]);
    assert_eq!(failures(&m.failures), vec![("word", "no value"), ("running", "no value")]);
    assert!(m.samples.is_empty());
}

#[test]
fn a_bad_tag_mapped_by_two_rules_is_reported_in_each() {
    // Once per tag in each rule that maps it: each rule's table has its own hole to explain.
    let s = spec(vec![
        rule("raw", false, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])]),
        rule("flags", false, vec![tag("ns=1;s=w", "", SeriesType::Int, vec![bit(0, 1, "running"), bit(1, 1, "fault")])]),
    ]);
    let m = map_poll(&s, &[read("ns=1;s=w", None)], 0, 0);
    assert_eq!(failures(&m.failures), vec![("word", "BadNodeIdUnknown"), ("running", "BadNodeIdUnknown")]);
}

#[test]
fn a_node_absent_from_the_poll_is_a_hole_with_nothing_to_report() {
    let s = spec(vec![rule(
        "line1",
        false,
        vec![tag("ns=1;s=t", "temp", SeriesType::Float, vec![]), tag("ns=1;s=u", "other", SeriesType::Float, vec![])],
    )]);
    let m = map_poll(&s, &[read("ns=1;s=u", Some(Variant::Double(1.0)))], 0, 0);
    assert_eq!(m.rows[0].values, vec![None, Some(Value::Float(1.0))]);
    assert!(m.failures.is_empty());
}

#[test]
fn a_type_mismatch_is_reported_not_truncated() {
    let s = spec(vec![rule("line1", false, vec![tag("ns=1;s=t", "count", SeriesType::Int, vec![])])]);
    // A Double into an int series: refused rather than rounded.
    let m = map_poll(&s, &[read("ns=1;s=t", Some(Variant::Double(21.7)))], 0, 0);
    assert_eq!(m.rows[0].values, vec![None]);
    assert_eq!(failures(&m.failures), vec![("count", "Double value does not fit a int series")]);
    assert!(m.samples.is_empty());
}

#[test]
fn bits_of_a_non_integer_are_reported_per_bit_while_its_own_column_maps() {
    let s = spec(vec![rule(
        "line1",
        false,
        vec![tag("ns=1;s=t", "temp", SeriesType::Float, vec![bit(0, 1, "b0"), bit(1, 1, "b1")])],
    )]);
    let m = map_poll(&s, &[read("ns=1;s=t", Some(Variant::Double(3.0)))], 0, 0);
    assert_eq!(m.rows[0].values, vec![Some(Value::Float(3.0)), None, None]);
    assert_eq!(
        failures(&m.failures),
        vec![
            ("b0", "bit extraction needs an integer value, got Double"),
            ("b1", "bit extraction needs an integer value, got Double"),
        ]
    );
}

#[test]
fn a_tag_shared_by_two_rules_is_read_once_and_lands_in_both() {
    let s = spec(vec![
        rule("raw", false, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])]),
        rule("flags", false, vec![tag("ns=1;s=w", "", SeriesType::Int, vec![bit(1, 1, "fault")])]),
    ]);
    // ONE read entry: exactly what poll_nodes would have asked for.
    let m = map_poll(&s, &[read("ns=1;s=w", Some(Variant::UInt16(0b10)))], 0, 0);
    assert_eq!(
        m.rows,
        vec![
            RuleRow { rule_index: 0, values: vec![Some(Value::Int(2))] },
            // The bits-only rule has no raw column, just the flag.
            RuleRow { rule_index: 1, values: vec![Some(Value::Int(1))] },
        ]
    );
}

#[test]
fn the_rendered_line_shows_values_and_holes_on_one_line() {
    let r = rule(
        "line1",
        false,
        vec![
            tag("ns=1;s=t", "temp", SeriesType::Float, vec![]),
            tag("ns=1;s=g", "gone", SeriesType::Float, vec![]),
            tag("ns=1;s=b", "batch", SeriesType::Text, vec![]),
        ],
    );
    let row = RuleRow {
        rule_index: 0,
        values: vec![Some(Value::Float(21.5)), None, Some(Value::Text(Arc::from("B-1\r\nnext\tcol\u{85}a\u{2028}b\u{2029}c")))],
    };
    assert_eq!(render_line(&r, &row), "line1 temp=21.5 gone=? batch=B-1  next col a b c");
    let empty = rule("quiet", false, vec![]);
    assert_eq!(render_line(&empty, &RuleRow { rule_index: 0, values: vec![] }), "quiet");
}

#[test]
fn store_on_change_skips_identical_polls_until_the_heartbeat() {
    let s = spec(vec![rule("words", true, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])])]);
    let reads = |v: i32| vec![read("ns=1;s=w", Some(Variant::Int32(v)))];
    let mut d = DedupState::default();

    // The first poll keeps; an identical second yields NOTHING.
    let m = map_poll_dedup(&s, &reads(7), 1000, 1, Some(&mut d));
    assert_eq!((m.rows.len(), m.samples.len()), (1, 1));
    let m = map_poll_dedup(&s, &reads(7), 2000, 2, Some(&mut d));
    assert_eq!((m.rows.len(), m.samples.len()), (0, 0), "no row, no samples");

    // A transition keeps again.
    let m = map_poll_dedup(&s, &reads(8), 3000, 3, Some(&mut d));
    assert_eq!(m.rows.len(), 1);

    // A quiet stretch heartbeats on exactly every HEARTBEAT_POLLS-th poll after the last kept.
    let mut kept_at = Vec::new();
    for i in 1..=(HEARTBEAT_POLLS * 2) {
        let m = map_poll_dedup(&s, &reads(8), 4000 + i64::from(i), 4 + u64::from(i), Some(&mut d));
        if !m.rows.is_empty() {
            assert_eq!(m.samples.len(), 1);
            kept_at.push(i);
        }
    }
    assert_eq!(kept_at, vec![HEARTBEAT_POLLS, HEARTBEAT_POLLS * 2]);

    // Without the memory, every poll keeps.
    let a = map_poll_dedup(&s, &reads(7), 1000, 1, None);
    let b = map_poll(&s, &reads(7), 2000, 2);
    assert_eq!(a.rows.len() + b.rows.len(), 2);
    // And a rule without the flag keeps every poll even with the memory.
    let plain = spec(vec![rule("words", false, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])])]);
    let mut d = DedupState::default();
    let kept: usize = (0..5).map(|i| map_poll_dedup(&plain, &reads(7), i, i as u64, Some(&mut d)).rows.len()).sum();
    assert_eq!(kept, 5);
}

#[test]
fn an_unchanged_rule_is_skipped_alone_and_still_reports_its_failures() {
    let s = spec(vec![
        rule(
            "words",
            true,
            vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![]), tag("ns=1;s=gone", "gone", SeriesType::Int, vec![])],
        ),
        rule("always", false, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])]),
    ]);
    let reads = vec![read("ns=1;s=w", Some(Variant::Int32(1))), read("ns=1;s=gone", None)];
    let mut d = DedupState::default();
    let first = map_poll_dedup(&s, &reads, 0, 0, Some(&mut d));
    assert_eq!(first.rows.iter().map(|r| r.rule_index).collect::<Vec<_>>(), vec![0, 1]);
    let second = map_poll_dedup(&s, &reads, 1, 1, Some(&mut d));
    // The unchanged on-change rule is skipped; the other rule keeps its row.
    assert_eq!(second.rows, vec![RuleRow { rule_index: 1, values: vec![Some(Value::Int(1))] }]);
    assert_eq!(second.samples.len(), 1);
    // The skipped rule's bad tag is still reported.
    assert_eq!(failures(&second.failures), vec![("gone", "BadNodeIdUnknown")]);
}

#[test]
fn a_float_stuck_at_nan_is_unchanged() {
    // NaN != NaN, but a reading frozen at NaN (a faulted sensor) has not changed.
    let s = spec(vec![rule("temps", true, vec![tag("ns=1;s=t", "temp", SeriesType::Float, vec![])])]);
    let nan = vec![read("ns=1;s=t", Some(Variant::Double(f64::NAN)))];
    let mut d = DedupState::default();
    let kept: Vec<u32> =
        (1..=HEARTBEAT_POLLS + 1).filter(|&i| !map_poll_dedup(&s, &nan, i64::from(i), 0, Some(&mut d)).rows.is_empty()).collect();
    assert_eq!(kept, vec![1, HEARTBEAT_POLLS + 1]);
    // A real number after the NaN is a transition, and back again.
    assert_eq!(map_poll_dedup(&s, &[read("ns=1;s=t", Some(Variant::Double(1.0)))], 0, 0, Some(&mut d)).rows.len(), 1);
    assert_eq!(map_poll_dedup(&s, &nan, 0, 0, Some(&mut d)).rows.len(), 1);
}

#[test]
fn a_hole_appearing_or_healing_is_a_transition() {
    let s = spec(vec![rule("words", true, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])])]);
    let mut d = DedupState::default();
    let good = vec![read("ns=1;s=w", Some(Variant::Int32(1)))];
    let bad = vec![read("ns=1;s=w", None)];
    let kept = |reads: &Vec<TagRead>, d: &mut DedupState| map_poll_dedup(&s, reads, 0, 0, Some(d)).rows.len();
    assert_eq!(kept(&good, &mut d), 1);
    assert_eq!(kept(&bad, &mut d), 1, "a value turning into a hole is kept");
    assert_eq!(kept(&bad, &mut d), 0, "an unchanged hole is skipped");
    assert_eq!(kept(&good, &mut d), 1, "the hole healing is kept");
}

#[test]
fn the_memory_follows_a_spec_that_gains_rules() {
    let one = spec(vec![rule("a", true, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])])]);
    let two = spec(vec![
        rule("a", true, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])]),
        rule("b", true, vec![tag("ns=1;s=w", "word", SeriesType::Int, vec![])]),
    ]);
    let reads = vec![read("ns=1;s=w", Some(Variant::Int32(3)))];
    let mut d = DedupState::default();
    assert_eq!(map_poll_dedup(&one, &reads, 0, 0, Some(&mut d)).rows.len(), 1);
    let m = map_poll_dedup(&two, &reads, 1, 1, Some(&mut d));
    // Rule a is unchanged and skipped; rule b is new and kept.
    assert_eq!(m.rows.iter().map(|r| r.rule_index).collect::<Vec<_>>(), vec![1]);
}
