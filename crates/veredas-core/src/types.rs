//! The shared types: what a value is, which series it belongs to, when it happened, and how
//! far it can be trusted.
//!
//! A *tag* in plant terms is a [`SeriesRef`]: a named series of one source. A reading of it is
//! a [`Sample`], carrying a [`Value`] and every ordering signal there is: the stamp parsed from
//! the source (source wall clock, no zone), a per-source dense counter, and the arrival clock.
//! Cross-source math never trusts a parsed stamp alone (clock skew), so arrival is kept beside
//! it forever.

use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::NaiveDateTime;

/// Identifies one configured source (a file family, a TCP feed, an OPC endpoint).
/// Cheap to clone; compares and orders by the id string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(Arc<str>);

impl SourceId {
    pub fn new(id: impl AsRef<str>) -> Self {
        SourceId(Arc::from(id.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One typed value. Numeric views read `Int` as `f64` and `Time` as epoch milliseconds (the
/// naive stamp read as if it were UTC, the same axis [`Sample::index_ms`] uses); `Text` has
/// none.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Text(Arc<str>),
    Time(NaiveDateTime),
}

impl Value {
    /// Numeric view for math and charts; `Text` has none.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Time(t) => Some(t.and_utc().timestamp_millis() as f64),
            Value::Text(_) => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Text(s) => f.write_str(s),
            Value::Time(t) => write!(f, "{t}"),
        }
    }
}

/// The declared type of a series: what every value on it must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeriesType {
    Int,
    Float,
    Text,
    Time,
}

impl fmt::Display for SeriesType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SeriesType::Int => "int",
            SeriesType::Float => "float",
            SeriesType::Text => "text",
            SeriesType::Time => "time",
        })
    }
}

/// Points at one series, a tag: `(source, series name)`. Displays as `source/series`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeriesRef {
    pub source: SourceId,
    pub series: Arc<str>,
}

impl SeriesRef {
    pub fn new(source: impl AsRef<str>, series: impl AsRef<str>) -> Self {
        SeriesRef {
            source: SourceId::new(source),
            series: Arc::from(series.as_ref()),
        }
    }
}

impl fmt::Display for SeriesRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.source, self.series)
    }
}

/// One value on one series.
///
/// `ts` is the stamp parsed from the source (source wall clock, no zone); `seq` is the
/// per-source dense counter, the ordering fallback when no stamp parses; `arrival_unix_ms` is
/// the common reference clock. [`Sample::index_ms`] puts every sample on one axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub source: SourceId,
    pub series: Arc<str>,
    /// The identity of the thing measured, when the source names one (a sensor, a parcel):
    /// pairing and per-key windows key on it.
    pub key: Option<Arc<str>>,
    pub ts: Option<NaiveDateTime>,
    pub seq: u64,
    pub arrival_unix_ms: i64,
    pub value: Value,
}

impl Sample {
    /// The time index: the parsed stamp when present (read as if UTC), arrival otherwise.
    pub fn index_ms(&self) -> i64 {
        match self.ts {
            Some(t) => t.and_utc().timestamp_millis(),
            None => self.arrival_unix_ms,
        }
    }

    pub fn series_ref(&self) -> SeriesRef {
        SeriesRef {
            source: self.source.clone(),
            series: Arc::clone(&self.series),
        }
    }
}

/// How far a reading can be trusted, whatever its protocol: the three-way severity OPC UA
/// puts in the top bits of every status code, and the verdict any other source can give.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Good,
    Uncertain,
    Bad,
}

impl Quality {
    pub fn is_good(self) -> bool {
        self == Quality::Good
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Good => "good",
            Quality::Uncertain => "uncertain",
            Quality::Bad => "bad",
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Wall-clock arrival stamp: milliseconds since the unix epoch. A clock set before 1970
/// reports the negative offset rather than lying.
pub fn now_unix_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}
