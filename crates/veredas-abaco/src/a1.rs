//! A1 addresses: `B7`, `$B$7`, `AA10:AC12`. Rows and columns are 1-based,
//! as Excel and the engine count them.

use std::fmt;

/// The engine's limits (Excel's): 1 048 576 rows, 16 384 columns (`XFD`).
pub const LAST_ROW: i32 = 1_048_576;
pub const LAST_COLUMN: i32 = 16_384;

/// One cell's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Addr {
    pub row: i32,
    pub column: i32,
}

/// A rectangle of cells, corners included, `first` top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Area {
    pub first: Addr,
    pub last: Addr,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an A1 address: {0:?}")]
pub struct BadAddress(pub String);

/// `1 → "A"`, `27 → "AA"`, `16384 → "XFD"`.
pub fn column_name(mut column: i32) -> String {
    let mut out = Vec::new();
    while column > 0 {
        let rem = ((column - 1) % 26) as u8;
        out.push(b'A' + rem);
        column = (column - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `"A" → 1`, `"xfd" → 16384`; `None` for anything else or past `XFD`.
pub fn column_number(letters: &str) -> Option<i32> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut n: i32 = 0;
    for c in letters.chars() {
        if !c.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + (c.to_ascii_uppercase() as i32 - 'A' as i32 + 1);
    }
    (1..=LAST_COLUMN).contains(&n).then_some(n)
}

impl Addr {
    pub fn new(row: i32, column: i32) -> Addr {
        Addr { row, column }
    }

    /// `B7` or `$B$7` (the `$` are accepted and ignored).
    pub fn parse(text: &str) -> Result<Addr, BadAddress> {
        let bad = || BadAddress(text.to_string());
        let t = text.trim().replace('$', "");
        let split = t.find(|c: char| c.is_ascii_digit()).ok_or_else(bad)?;
        let (letters, digits) = t.split_at(split);
        let column = column_number(letters).ok_or_else(bad)?;
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        let row: i32 = digits.parse().map_err(|_| bad())?;
        if !(1..=LAST_ROW).contains(&row) {
            return Err(bad());
        }
        Ok(Addr { row, column })
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", column_name(self.column), self.row)
    }
}

impl Area {
    /// `A1:C3`, `C3:A1` (normalised), or a single cell `B2`.
    pub fn parse(text: &str) -> Result<Area, BadAddress> {
        match text.split_once(':') {
            None => {
                let a = Addr::parse(text)?;
                Ok(Area { first: a, last: a })
            }
            Some((a, b)) => {
                let a = Addr::parse(a).map_err(|_| BadAddress(text.to_string()))?;
                let b = Addr::parse(b).map_err(|_| BadAddress(text.to_string()))?;
                Ok(Area::spanning(a, b))
            }
        }
    }

    /// The rectangle with `a` and `b` as opposite corners.
    pub fn spanning(a: Addr, b: Addr) -> Area {
        Area {
            first: Addr::new(a.row.min(b.row), a.column.min(b.column)),
            last: Addr::new(a.row.max(b.row), a.column.max(b.column)),
        }
    }

    pub fn rows(&self) -> i32 {
        self.last.row - self.first.row + 1
    }

    pub fn columns(&self) -> i32 {
        self.last.column - self.first.column + 1
    }

    pub fn is_single(&self) -> bool {
        self.first == self.last
    }
}

impl fmt::Display for Area {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_single() {
            write!(f, "{}", self.first)
        } else {
            write!(f, "{}:{}", self.first, self.last)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_both_ways_at_the_edges() {
        for (n, s) in [(1, "A"), (26, "Z"), (27, "AA"), (52, "AZ"), (703, "AAA"), (16384, "XFD")] {
            assert_eq!(column_name(n), s);
            assert_eq!(column_number(s), Some(n));
        }
        assert_eq!(column_number("xfd"), Some(16384));
        assert_eq!(column_number("XFE"), None);
        assert_eq!(column_number(""), None);
        assert_eq!(column_number("A1"), None);
        assert_eq!(column_name(0), "");
    }

    #[test]
    fn addresses_parse_with_dollars_and_refuse_junk() {
        assert_eq!(Addr::parse("B7").unwrap(), Addr::new(7, 2));
        assert_eq!(Addr::parse(" $AA$10 ").unwrap(), Addr::new(10, 27));
        for bad in ["", "7", "B", "B0", "B1048577", "B7x", "1B", "B-1", "ÄB1"] {
            assert!(Addr::parse(bad).is_err(), "{bad:?} parsed");
        }
        assert_eq!(Addr::new(1_048_576, 16_384).to_string(), "XFD1048576");
    }

    #[test]
    fn areas_normalise_and_print() {
        let a = Area::parse("C3:A1").unwrap();
        assert_eq!(a.to_string(), "A1:C3");
        assert_eq!((a.rows(), a.columns()), (3, 3));
        assert_eq!(Area::parse("B2").unwrap().to_string(), "B2");
        assert!(Area::parse("A1:").is_err());
        assert!(Area::parse("A1:B2:C3").is_err());
    }
}
