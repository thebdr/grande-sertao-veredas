//! The shared types (contract: the shared-types clause). Veredas 1's tests of `SourceId`,
//! `Value` and `Sample` come along; the edges are pinned with exact outputs.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::NaiveDateTime;
use veredas_core::{Quality, Sample, SeriesRef, SeriesType, SourceId, Value, now_unix_ms};

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").unwrap()
}

fn sample(ts: Option<NaiveDateTime>, arrival: i64) -> Sample {
    Sample {
        source: SourceId::new("a"),
        series: Arc::from("v"),
        key: None,
        ts,
        seq: 0,
        arrival_unix_ms: arrival,
        value: Value::Float(1.0),
    }
}

#[test]
fn a_source_id_round_trips_and_compares_by_its_text() {
    let a = SourceId::new("system-a");
    let b = a.clone();
    assert_eq!(a, b);
    assert_eq!(a.as_str(), "system-a");
    assert_eq!(a.to_string(), "system-a");
    assert_ne!(a, SourceId::new("system-b"));
    assert_ne!(a, SourceId::new("System-A"), "ids are case-sensitive");
    assert_eq!(SourceId::new(""), SourceId::new(String::new()));
    let ordered: Vec<_> = [SourceId::new("b"), SourceId::new("a"), SourceId::new("B")]
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(ordered, ["B", "a", "b"], "ordered by the bytes of the id");
}

#[test]
fn a_value_has_a_numeric_view_except_text() {
    assert_eq!(Value::Int(7).as_f64(), Some(7.0));
    assert_eq!(Value::Int(-3).as_f64(), Some(-3.0));
    assert_eq!(Value::Float(2.5).as_f64(), Some(2.5));
    assert_eq!(Value::Text("12".into()).as_f64(), None, "text is never parsed into a number");
    assert_eq!(Value::Text("".into()).as_f64(), None);
    // A time reads as epoch milliseconds of the naive stamp taken as UTC.
    assert_eq!(Value::Time(at("1970-01-01 00:00:01.250")).as_f64(), Some(1250.0));
    assert_eq!(Value::Time(at("1969-12-31 23:59:59")).as_f64(), Some(-1000.0));
    assert_eq!(Value::Time(at("2026-07-18 00:00:00")).as_f64(), Some(1_784_332_800_000.0));
    // Below the millisecond the stamp rounds down, before the epoch too.
    assert_eq!(Value::Time(at("1970-01-01 00:00:00.0009")).as_f64(), Some(0.0));
    assert_eq!(Value::Time(at("1969-12-31 23:59:59.9995")).as_f64(), Some(-1.0));
    // An int wider than 32 bits keeps its value.
    assert_eq!(Value::Int(5_000_000_000).as_f64(), Some(5e9));
    // A NaN float stays NaN: no invented number.
    assert!(Value::Float(f64::NAN).as_f64().unwrap().is_nan());
}

#[test]
fn a_value_displays_as_the_operator_reads_it() {
    assert_eq!(Value::Int(-42).to_string(), "-42");
    assert_eq!(Value::Float(2.5).to_string(), "2.5");
    assert_eq!(Value::Float(3.0).to_string(), "3");
    assert_eq!(Value::Text("line 1; ok".into()).to_string(), "line 1; ok");
    assert_eq!(Value::Time(at("2026-07-18 10:00:00.5")).to_string(), "2026-07-18 10:00:00.500");
    assert_eq!(Value::Time(at("2026-07-18 10:00:00")).to_string(), "2026-07-18 10:00:00");
}

#[test]
fn values_compare_by_variant_and_content() {
    assert_eq!(Value::Int(1), Value::Int(1));
    assert_ne!(Value::Int(1), Value::Float(1.0), "an int is not a float of the same number");
    assert_ne!(Value::Text("1".into()), Value::Int(1));
    assert_ne!(Value::Float(f64::NAN), Value::Float(f64::NAN));
}

#[test]
fn a_series_type_displays_its_lowercase_name() {
    let names: Vec<String> = [SeriesType::Int, SeriesType::Float, SeriesType::Text, SeriesType::Time]
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(names, ["int", "float", "text", "time"]);
}

#[test]
fn a_series_ref_is_a_tag_of_one_source() {
    let r = SeriesRef::new("plc", "line1.temperature");
    assert_eq!(r.source, SourceId::new("plc"));
    assert_eq!(&*r.series, "line1.temperature");
    assert_eq!(r.to_string(), "plc/line1.temperature");
    assert_ne!(r, SeriesRef::new("plc2", "line1.temperature"));
    assert_ne!(r, SeriesRef::new("plc", "line1.pressure"));
    // Ordered by source first, then series.
    let mut refs = [SeriesRef::new("b", "a"), SeriesRef::new("a", "z"), SeriesRef::new("a", "b")];
    refs.sort();
    let shown: Vec<String> = refs.iter().map(|r| r.to_string()).collect();
    assert_eq!(shown, ["a/b", "a/z", "b/a"]);
    // Not by the shown text: "plc" sorts before "plc-2" as a source, though "plc-2/temp" sorts
    // before "plc/temp" as a string ('-' is below '/').
    let mut refs = [SeriesRef::new("plc-2", "temp"), SeriesRef::new("plc", "temp")];
    refs.sort();
    assert_eq!(refs.map(|r| r.to_string()), ["plc/temp", "plc-2/temp"]);
    let mut as_text = ["plc-2/temp".to_string(), "plc/temp".to_string()];
    as_text.sort();
    assert_eq!(as_text, ["plc-2/temp", "plc/temp"], "the fixture tells the two orders apart");
    let mut refs = [SeriesRef::new("a.b", "c"), SeriesRef::new("a", "z")];
    refs.sort();
    assert_eq!(refs.map(|r| r.to_string()), ["a/z", "a.b/c"]);
}

#[test]
fn a_sample_indexes_on_its_parsed_stamp_then_on_arrival() {
    let t = at("2026-07-18 10:00:00");
    let mut s = sample(Some(t), 123);
    assert_eq!(s.index_ms(), 1_784_368_800_000);
    s.ts = None;
    assert_eq!(s.index_ms(), 123);
    // A stamp before the epoch is a negative index, not arrival.
    assert_eq!(sample(Some(at("1969-12-31 23:59:59.999")), 5).index_ms(), -1);
    // A clock set before 1970 gives a negative arrival, which is the index when no stamp parsed.
    assert_eq!(sample(None, -250).index_ms(), -250);
    // The index rounds a sub-millisecond stamp down, as the value's numeric view does.
    assert_eq!(sample(Some(at("1969-12-31 23:59:59.9995")), 0).index_ms(), -1);
    assert_eq!(sample(Some(at("2026-07-18 10:00:00.0009")), 0).index_ms(), 1_784_368_800_000);
}

#[test]
fn a_sample_names_its_series() {
    let mut s = sample(None, 0);
    s.key = Some(Arc::from("sensor-7"));
    assert_eq!(s.series_ref(), SeriesRef::new("a", "v"));
    assert_eq!(s.series_ref().to_string(), "a/v");
}

#[test]
fn quality_is_three_way_and_only_good_is_good() {
    assert!(Quality::Good.is_good());
    assert!(!Quality::Uncertain.is_good());
    assert!(!Quality::Bad.is_good());
    let shown: Vec<String> =
        [Quality::Good, Quality::Uncertain, Quality::Bad].iter().map(|q| q.to_string()).collect();
    assert_eq!(shown, ["good", "uncertain", "bad"]);
    assert_eq!(Quality::Uncertain.label(), "uncertain");
}

#[test]
fn arrival_is_the_unix_clock_in_milliseconds() {
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let now = now_unix_ms();
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    assert!(before <= now && now <= after, "{before} <= {now} <= {after}");
}
