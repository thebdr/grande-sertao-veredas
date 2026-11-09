//! The state vocabulary every object shares, and the rules that read a value into a state.
//!
//! A state is a class with a mark and a word, so it reads without colour (`docs/design/README.md`,
//! "Every state has a mark and a word"). The class sets the mark and the colour; the rule sets the
//! word ("full", "stopped", "fault"). Normal is the absence of a rule; stale is no rule's, judged
//! on the arrival clock by the feed; selected is the page's own.

use serde::Serialize;

/// The classes of the vocabulary, in the order of their weight on an attention list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateClass {
    Normal,
    Near,
    Warn,
    Alarm,
    Disabled,
    Stale,
}

impl StateClass {
    /// The class a rule names; normal and stale are no rule's.
    pub fn of_rule(name: &str) -> Option<StateClass> {
        match name {
            "near" => Some(StateClass::Near),
            "warn" => Some(StateClass::Warn),
            "alarm" => Some(StateClass::Alarm),
            "disabled" => Some(StateClass::Disabled),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StateClass::Normal => "normal",
            StateClass::Near => "near",
            StateClass::Warn => "warn",
            StateClass::Alarm => "alarm",
            StateClass::Disabled => "disabled",
            StateClass::Stale => "stale",
        }
    }

    /// The mark the class reads by: ● ■ ■ ! ▨ ◌.
    pub fn mark(self) -> &'static str {
        match self {
            StateClass::Normal => "\u{25CF}",
            StateClass::Near | StateClass::Warn => "\u{25A0}",
            StateClass::Alarm => "!",
            StateClass::Disabled => "\u{25A8}",
            StateClass::Stale => "\u{25CC}",
        }
    }
}

/// A value as the feed holds it: a number (an int or a float series), or text.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    Num(f64),
    Text(String),
}

impl Reading {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Reading::Num(v) => Some(*v),
            Reading::Text(_) => None,
        }
    }
}

/// What `equals` compares with: a number compares with numbers, text with text, exactly.
#[derive(Clone, Debug, PartialEq)]
pub enum Comparand {
    Num(f64),
    Text(String),
}

/// One comparison of a rule.
#[derive(Clone, Debug, PartialEq)]
pub enum Compare {
    Equals(Comparand),
    Above(f64),
    Below(f64),
    AtLeast(f64),
    AtMost(f64),
}

impl Compare {
    /// Whether the comparison holds for a reading. A number never equals text and a text reading
    /// is never above or below anything: a rule over the wrong kind of value simply does not hold.
    pub fn holds(&self, r: &Reading) -> bool {
        match (self, r) {
            (Compare::Equals(Comparand::Num(x)), Reading::Num(v)) => v == x,
            (Compare::Equals(Comparand::Text(x)), Reading::Text(v)) => v == x,
            (Compare::Equals(_), _) => false,
            (Compare::Above(x), Reading::Num(v)) => v > x,
            (Compare::Below(x), Reading::Num(v)) => v < x,
            (Compare::AtLeast(x), Reading::Num(v)) => v >= x,
            (Compare::AtMost(x), Reading::Num(v)) => v <= x,
            _ => false,
        }
    }

    /// The comparison as a person reads it, the unit after the number: `below 98 %`.
    pub fn describe(&self, unit: &str) -> String {
        let with_unit = |v: f64| {
            let n = format_number(v, None);
            if unit.is_empty() { n } else { format!("{n} {unit}") }
        };
        match self {
            Compare::Equals(Comparand::Num(x)) => format!("equals {}", with_unit(*x)),
            Compare::Equals(Comparand::Text(x)) => format!("is {x}"),
            Compare::Above(x) => format!("above {}", with_unit(*x)),
            Compare::Below(x) => format!("below {}", with_unit(*x)),
            Compare::AtLeast(x) => format!("at least {}", with_unit(*x)),
            Compare::AtMost(x) => format!("at most {}", with_unit(*x)),
        }
    }
}

/// A number as the screen shows it: with `decimals` places when given, else as it reads with at
/// most three places, trailing zeros and a bare point dropped; never `-0`.
pub fn format_number(v: f64, decimals: Option<u8>) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let text = match decimals {
        Some(d) => format!("{v:.prec$}", prec = d as usize),
        None => {
            let t = format!("{v:.3}");
            let t = t.trim_end_matches('0');
            t.trim_end_matches('.').to_string()
        }
    };
    // A negative value rounded to zero prints as -0, -0.0: shown as zero.
    if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
        text[1..].to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_they_are_shown() {
        assert_eq!(format_number(35.0, None), "35");
        assert_eq!(format_number(2.4, None), "2.4");
        assert_eq!(format_number(1.23456, None), "1.235");
        assert_eq!(format_number(1.5, Some(0)), "2");
        assert_eq!(format_number(97.94, Some(1)), "97.9");
        assert_eq!(format_number(-0.0004, None), "0");
        assert_eq!(format_number(-0.04, Some(1)), "0.0");
        assert_eq!(format_number(-2.5, None), "-2.5");
        assert_eq!(format_number(8640.0, None), "8640");
    }

    #[test]
    fn a_rule_holds_only_over_its_kind_of_value() {
        assert!(Compare::Equals(Comparand::Num(2.0)).holds(&Reading::Num(2.0)));
        assert!(!Compare::Equals(Comparand::Num(2.0)).holds(&Reading::Text("2".into())));
        assert!(Compare::Equals(Comparand::Text("run".into())).holds(&Reading::Text("run".into())));
        assert!(!Compare::Equals(Comparand::Text("run".into())).holds(&Reading::Text("Run".into())));
        assert!(Compare::AtLeast(100.0).holds(&Reading::Num(100.0)));
        assert!(!Compare::Above(100.0).holds(&Reading::Num(100.0)));
        assert!(Compare::Below(98.0).holds(&Reading::Num(97.9)));
        assert!(Compare::AtMost(0.0).holds(&Reading::Num(0.0)));
        assert!(!Compare::Below(98.0).holds(&Reading::Text("97".into())));
        assert_eq!(Compare::Below(98.0).describe("%"), "below 98 %");
        assert_eq!(Compare::Above(2.0).describe(""), "above 2");
    }
}
