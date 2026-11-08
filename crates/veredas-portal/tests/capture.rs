//! Capture rules and timestamp rules (contract: the capture clause): a line read into typed
//! series by a regex's named groups, one group the identity key, a value refusing its type
//! counted and never killing the line; the stamp a line carries located and parsed by ordered
//! format candidates, the one that succeeds staying preferred for the stream.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDateTime;
use veredas_portal::{CaptureError, CaptureRule, LineEvent, SeriesType, SourceError, SourceId, TimestampExtractor, TimestampSpec, Value, named_groups};

fn ev(source: &str, line: &str) -> LineEvent {
    LineEvent { source: SourceId::new(source), seq: 7, arrival_unix_ms: 1_000, parsed_ts: None, line: line.to_string() }
}

fn types(list: &[(&str, SeriesType)]) -> HashMap<String, SeriesType> {
    list.iter().map(|(n, t)| (n.to_string(), *t)).collect()
}

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

// ---- capture rules ---------------------------------------------------------------------------

#[test]
fn typed_groups_become_samples_carrying_the_key_and_every_ordering_signal() {
    let rule = CaptureRule::new(
        "readings",
        r"S (?P<sensor>\w+) v=(?P<v>-?\d+\.\d+) n=(?P<n>\d+) tag=(?P<tag>\w+)",
        None,
        &types(&[("v", SeriesType::Float), ("n", SeriesType::Int)]),
        Some("sensor"),
    )
    .unwrap();
    assert_eq!(rule.name(), "readings");
    assert_eq!(rule.key_group(), Some("sensor"));
    assert_eq!(rule.scope(), None);
    let series: Vec<(String, SeriesType)> = rule.series().map(|(n, t)| (n.to_string(), t)).collect();
    assert_eq!(
        series,
        vec![("v".into(), SeriesType::Float), ("n".into(), SeriesType::Int), ("tag".into(), SeriesType::Text)],
        "the key group is not a series; an untyped group is text"
    );

    let mut line = ev("press-a", "S temp_3 v=-2.50 n=42 tag=ok");
    line.parsed_ts = Some(dt("2026-07-18 10:00:00"));
    let out = rule.apply(&line, &[]);
    assert!(out.matched);
    assert_eq!(out.coercion_failures, 0);
    assert_eq!(out.key.as_deref(), Some("temp_3"));
    assert_eq!(out.samples.len(), 3);
    for s in &out.samples {
        assert_eq!(s.key.as_deref(), Some("temp_3"));
        assert_eq!(s.seq, 7);
        assert_eq!(s.arrival_unix_ms, 1_000);
        assert_eq!(s.ts, Some(dt("2026-07-18 10:00:00")));
        assert_eq!(s.source.as_str(), "press-a");
    }
    assert_eq!(out.samples[0].series.as_ref(), "v");
    assert_eq!(out.samples[0].value, Value::Float(-2.5));
    assert_eq!(out.samples[1].series.as_ref(), "n");
    assert_eq!(out.samples[1].value, Value::Int(42));
    assert_eq!(out.samples[2].series.as_ref(), "tag");
    assert_eq!(out.samples[2].value, Value::Text(Arc::from("ok")));
}

#[test]
fn the_scope_limits_the_rule_and_a_miss_yields_nothing() {
    let rule = CaptureRule::new("scoped", r"v=(?P<v>\d+)", Some(SourceId::new("only-this")), &types(&[("v", SeriesType::Int)]), None)
        .unwrap();
    assert_eq!(rule.scope(), Some(&SourceId::new("only-this")));
    let other = rule.apply(&ev("other", "v=1"), &[]);
    assert!(!other.matched && other.samples.is_empty() && other.key.is_none());
    assert!(rule.apply(&ev("only-this", "v=1"), &[]).matched);
    let miss = rule.apply(&ev("only-this", "nothing here"), &[]);
    assert!(!miss.matched);
    assert!(miss.samples.is_empty());
    assert_eq!(miss.coercion_failures, 0);
}

#[test]
fn a_value_refusing_its_type_is_counted_and_the_rest_of_the_line_lands() {
    let rule = CaptureRule::new(
        "mixed",
        r"a=(?P<a>\S+) b=(?P<b>\S+) c=(?P<c>\S+)",
        None,
        &types(&[("a", SeriesType::Int), ("b", SeriesType::Float), ("c", SeriesType::Int)]),
        None,
    )
    .unwrap();
    let out = rule.apply(&ev("s", "a=notanint b=3.5 c=1.5"), &[]);
    assert!(out.matched);
    assert_eq!(out.coercion_failures, 2, "a text and a fraction refuse an int");
    assert_eq!(out.samples.len(), 1);
    assert_eq!(out.samples[0].value, Value::Float(3.5));
    // Whitespace around a number is trimmed; a sign and an exponent parse as the type allows.
    let rule = CaptureRule::new("n", r"v=\[(?P<v>[^\]]*)\] f=\[(?P<f>[^\]]*)\]", None, &types(&[("v", SeriesType::Int), ("f", SeriesType::Float)]), None).unwrap();
    let out = rule.apply(&ev("s", "v=[ -7 ] f=[ 1e3 ]"), &[]);
    assert_eq!(out.samples.iter().map(|s| s.value.clone()).collect::<Vec<_>>(), [Value::Int(-7), Value::Float(1000.0)]);
    let out = rule.apply(&ev("s", "v=[] f=[nan]"), &[]);
    assert_eq!(out.coercion_failures, 1, "an empty int refuses");
    assert!(matches!(out.samples[0].value, Value::Float(f) if f.is_nan()), "the float type takes what the parser takes");
}

#[test]
fn time_groups_parse_with_the_source_s_format_list_in_order() {
    let rule = CaptureRule::new("times", r"at=(?P<at>[\d:\-. ]+)$", None, &types(&[("at", SeriesType::Time)]), None).unwrap();
    let formats = vec!["%d.%m.%Y %H:%M".to_string(), "%Y-%m-%d %H:%M".to_string()];
    let out = rule.apply(&ev("s", "at=2026-07-18 10:30"), &formats);
    assert_eq!(out.coercion_failures, 0);
    assert_eq!(out.samples[0].value, Value::Time(dt("2026-07-18 10:30:00")));
    let out = rule.apply(&ev("s", "at=18.07.2026 10:30"), &formats);
    assert_eq!(out.samples[0].value, Value::Time(dt("2026-07-18 10:30:00")));
    // A date alone resolves to midnight.
    let out = rule.apply(&ev("s", "at=2026-07-18"), &["%Y-%m-%d".to_string()]);
    assert_eq!(out.samples[0].value, Value::Time(dt("2026-07-18 00:00:00")));
    // No format configured: a counted failure, never a silent drop.
    let out = rule.apply(&ev("s", "at=2026-07-18 10:30"), &[]);
    assert_eq!(out.coercion_failures, 1);
    assert!(out.samples.is_empty());
    assert!(out.matched);
}

#[test]
fn an_absent_optional_group_emits_nothing_and_counts_no_failure() {
    let rule = CaptureRule::new("opt", r"v=(?P<v>\d+)(?: u=(?P<u>\w+))?", None, &types(&[("v", SeriesType::Int)]), None).unwrap();
    let out = rule.apply(&ev("s", "v=5"), &[]);
    assert_eq!(out.samples.len(), 1);
    assert_eq!(out.coercion_failures, 0);
    let out = rule.apply(&ev("s", "v=5 u=mm"), &[]);
    assert_eq!(out.samples.len(), 2);
    assert_eq!(out.samples[1].value, Value::Text(Arc::from("mm")));
}

#[test]
fn the_key_is_carried_even_when_every_value_fails_and_an_absent_key_is_none() {
    let rule = CaptureRule::new("k", r"(?P<id>\w+): (?P<v>\S+)", None, &types(&[("v", SeriesType::Int)]), Some("id")).unwrap();
    let out = rule.apply(&ev("s", "pump7: broken"), &[]);
    assert!(out.matched);
    assert_eq!(out.key.as_deref(), Some("pump7"));
    assert!(out.samples.is_empty());
    assert_eq!(out.coercion_failures, 1);
    let rule = CaptureRule::new("k", r"(?:(?P<id>\w+): )?v=(?P<v>\d+)", None, &types(&[("v", SeriesType::Int)]), Some("id")).unwrap();
    let out = rule.apply(&ev("s", "v=3"), &[]);
    assert_eq!(out.key, None);
    assert_eq!(out.samples[0].key, None);
}

#[test]
fn named_groups_are_listed_in_pattern_order_and_empty_for_a_bad_or_groupless_pattern() {
    assert_eq!(named_groups(r"(?P<a>\d+) then (?P<b>\w+)"), vec!["a", "b"]);
    assert_eq!(named_groups(r"(?P<b>\d+)(x)(?P<a>\w+)"), vec!["b", "a"], "an unnamed group is not listed");
    assert!(named_groups("(broken").is_empty());
    assert!(named_groups(r"\d+").is_empty());
}

#[test]
fn construction_refuses_a_bad_pattern_no_groups_and_an_unknown_key_group_by_name() {
    match CaptureRule::new("r", "(broken", None, &HashMap::new(), None) {
        Err(CaptureError::Pattern(rule, msg)) => {
            assert_eq!(rule, "r");
            assert!(msg.contains("unclosed group"), "{msg}");
        }
        other => panic!("{:?}", other.err()),
    }
    assert_eq!(
        CaptureRule::new("r", r"\d+", None, &HashMap::new(), None).err(),
        Some(CaptureError::NoGroups("r".into()))
    );
    assert_eq!(
        CaptureRule::new("r", r"(\d+)", None, &HashMap::new(), None).err(),
        Some(CaptureError::NoGroups("r".into())),
        "an unnamed group is no series"
    );
    assert_eq!(
        CaptureRule::new("r", r"(?P<v>\d+)", None, &HashMap::new(), Some("ghost")).err(),
        Some(CaptureError::KeyGroup { rule: "r".into(), key: "ghost".into() })
    );
    assert_eq!(
        CaptureError::KeyGroup { rule: "r".into(), key: "ghost".into() }.to_string(),
        "rule `r`: key group `ghost` is not a named group of the pattern"
    );
    assert_eq!(CaptureError::NoGroups("r".into()).to_string(), "rule `r`: the pattern has no named groups, so nothing to capture");
    // Lookaround is not this engine's syntax: refused as a bad pattern, never a second engine.
    assert!(matches!(CaptureRule::new("r", r"(?P<v>\d+)(?= OK)", None, &HashMap::new(), None), Err(CaptureError::Pattern(..))));
}

// ---- timestamp rules -------------------------------------------------------------------------

#[test]
fn a_named_group_locates_the_stamp_and_the_whole_match_is_the_fallback() {
    let spec = TimestampSpec::new(r"^\[(?P<ts>[^\]]+)\]", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    assert_eq!(spec.extract("[2026-07-17 10:30:00] INFO sensor ok"), Some(dt("2026-07-17 10:30:00")));
    assert_eq!(spec.extract("no stamp here"), None);
    assert_eq!(spec.extract("[garbage] INFO"), None);
    assert_eq!(spec.formats(), ["%Y-%m-%d %H:%M:%S"]);

    let spec = TimestampSpec::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}", ["%Y-%m-%d %H:%M:%S"]).unwrap();
    assert_eq!(spec.extract("prefix 2026-01-02 03:04:05 suffix"), Some(dt("2026-01-02 03:04:05")));
}

#[test]
fn formats_are_tried_in_order_and_fractions_and_dates_alone_parse() {
    let spec = TimestampSpec::new(r"^(?P<ts>\S+ [\d:]+)", ["%Y-%m-%d %H:%M:%S", "%d.%m.%Y %H:%M:%S"]).unwrap();
    assert_eq!(spec.extract("2026-07-17 10:00:00 a"), Some(dt("2026-07-17 10:00:00")));
    assert_eq!(spec.extract("17.07.2026 10:00:00 a"), Some(dt("2026-07-17 10:00:00")));

    let spec = TimestampSpec::new(r"^(?P<ts>\S+ \S+)", ["%Y-%m-%d %H:%M:%S%.3f"]).unwrap();
    let got = spec.extract("2026-07-17 10:30:00.123 x").unwrap();
    assert_eq!(got.format("%H:%M:%S%.3f").to_string(), "10:30:00.123");

    let spec = TimestampSpec::new(r"^(?P<ts>\d{8})", ["%Y%m%d"]).unwrap();
    assert_eq!(spec.extract("20260717 a date alone"), Some(dt("2026-07-17 00:00:00")));
    assert_eq!(spec.extract("99999999 not a date"), None);
}

#[test]
fn a_successful_format_stays_preferred_for_the_stream() {
    // "01.02.2026" is ambiguous between day.month and month.day; the extractor resolves it with
    // whichever format last succeeded, so a stream reads consistently.
    let spec = TimestampSpec::new(r"^(?P<ts>[\d.]+ [\d:]+)", ["%d.%m.%Y %H:%M:%S", "%m.%d.%Y %H:%M:%S"]).unwrap();
    let mut x = TimestampExtractor::new(spec.clone());
    assert_eq!(x.extract("01.02.2026 10:00:00 a"), Some(dt("2026-02-01 10:00:00")), "configured order first");
    assert_eq!(x.extract("02.13.2026 10:00:00 a"), Some(dt("2026-02-13 10:00:00")), "only month.day parses a 13");
    assert_eq!(x.extract("01.02.2026 10:00:00 a"), Some(dt("2026-01-02 10:00:00")), "now resolved as month.day");
    // The stateless spec has no memory: configured order every time.
    assert_eq!(spec.extract("01.02.2026 10:00:00 a"), Some(dt("2026-02-01 10:00:00")));
}

#[test]
fn an_empty_format_list_and_a_bad_pattern_are_refused() {
    let formats: [&str; 0] = [];
    match TimestampSpec::new(r"x", formats) {
        Err(SourceError::Spec(msg)) => assert_eq!(msg, "timestamp spec needs at least one chrono format"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(TimestampSpec::new(r"(unclosed", ["%Y"]), Err(SourceError::Pattern(_))));
}

/// A preferred format that misses falls back in the configured order, never by rotation from
/// the preference: a format that never succeeded must not beat an earlier configured one, or the
/// stream would read an ambiguous stamp the other way round from the stateless spec, and keep
/// reading it so.
#[test]
fn a_failed_preference_falls_back_in_configured_order_not_by_rotation() {
    let spec = TimestampSpec::new(r"^(?P<ts>\S+)", ["%d.%m.%Y", "%Y-%m-%d", "%m.%d.%Y"]).unwrap();
    let mut x = TimestampExtractor::new(spec.clone());
    assert_eq!(x.extract("2026-01-05"), Some(dt("2026-01-05 00:00:00")), "the ISO format now preferred");
    // The preferred format misses here: the configured order decides, as the stateless spec does.
    assert_eq!(spec.extract("01.02.2026"), Some(dt("2026-02-01 00:00:00")));
    assert_eq!(x.extract("01.02.2026"), Some(dt("2026-02-01 00:00:00")), "day.month, first in the configured order");
    // And day.month is what now stays preferred.
    assert_eq!(x.extract("03.04.2026"), Some(dt("2026-04-03 00:00:00")));
}
