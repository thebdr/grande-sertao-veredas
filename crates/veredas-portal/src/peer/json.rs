//! JSON read into a `serde_json::Value` with every number read as its text says (C-095).
//!
//! serde_json's own reader, without its `float_roundtrip` feature, rounds a decimal twice and
//! reads about one double in seven a unit in the last place off (`1.4000000000000001`, the
//! double 14 × 0.1 is, read as 1.4). The feature is not portal's to turn on: serde_json is a
//! build dependency of the database engine, so the feature would rebuild the engine in every
//! build folder, and it would change every member's reading. So the reader reads its answers
//! here, each number through Rust's correctly rounded parse, and hands the value to serde.

use serde_json::{Map, Number, Value};

/// The deepest nesting read, serde_json's own limit.
const DEPTH: usize = 128;

/// The JSON text as a value; a text that is no JSON refused with what broke and where.
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut r = Reader { s: text.as_bytes(), i: 0, depth: 0 };
    r.space();
    let v = r.value()?;
    r.space();
    if r.i < r.s.len() {
        return Err(r.at("trailing characters"));
    }
    Ok(v)
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Reader<'_> {
    fn at(&self, what: &str) -> String {
        format!("{what} at byte {}", self.i)
    }

    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        let here = self.s.get(self.i) == Some(&c);
        if here {
            self.i += 1;
        }
        here
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while let Some(b'0'..=b'9') = self.s.get(self.i) {
            self.i += 1;
        }
        self.i - start
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.s.get(self.i) {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.at("expected a value")),
            None => Err(self.at("the text ended before a value")),
        }
    }

    fn nested(&mut self, read: fn(&mut Self) -> Result<Value, String>) -> Result<Value, String> {
        self.depth += 1;
        if self.depth > DEPTH {
            return Err(self.at("nested past 128"));
        }
        let v = read(self);
        self.depth -= 1;
        v
    }

    fn word(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.at("expected a value"))
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut map = Map::new();
        self.space();
        if self.eat(b'}') {
            return Ok(Value::Object(map));
        }
        loop {
            self.space();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.at("expected a key"));
            }
            let key = self.string()?;
            self.space();
            if !self.eat(b':') {
                return Err(self.at("expected `:`"));
            }
            self.space();
            let v = self.value()?;
            map.insert(key, v);
            self.space();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Value::Object(map));
            }
            return Err(self.at("expected `,` or `}`"));
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.space();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            self.space();
            items.push(self.value()?);
            self.space();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Value::Array(items));
            }
            return Err(self.at("expected `,` or `]`"));
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(&c) = self.s.get(self.i) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // The text is UTF-8 and the run stops at an ASCII byte: whole characters.
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.at("no UTF-8"))?);
            match self.s.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    self.escape(&mut out)?;
                }
                Some(_) => return Err(self.at("a control character in a string")),
                None => return Err(self.at("the text ended inside a string")),
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), String> {
        let Some(&c) = self.s.get(self.i) else { return Err(self.at("the text ended inside a string")) };
        self.i += 1;
        match c {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let high = self.hex4()?;
                let code = if (0xD800..0xDC00).contains(&high) {
                    if !(self.eat(b'\\') && self.eat(b'u')) {
                        return Err(self.at("a lone leading surrogate"));
                    }
                    let low = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return Err(self.at("a lone leading surrogate"));
                    }
                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                } else {
                    high
                };
                out.push(char::from_u32(code).ok_or_else(|| self.at("a lone trailing surrogate"))?);
            }
            _ => return Err(self.at("an escape that is none")),
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(digits) = self.s.get(self.i..self.i + 4) else { return Err(self.at("the text ended inside an escape")) };
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(self.at("an escape that is none"));
        }
        let n = digits.iter().fold(0u32, |n, d| n * 16 + (*d as char).to_digit(16).unwrap_or(0));
        self.i += 4;
        Ok(n)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        self.eat(b'-');
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.at("a number that is none")),
        }
        let mut whole = true;
        if self.eat(b'.') {
            whole = false;
            if self.digits() == 0 {
                return Err(self.at("a number that is none"));
            }
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            whole = false;
            self.i += 1;
            if let Some(b'+' | b'-') = self.s.get(self.i) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.at("a number that is none"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.at("a number that is none"))?;
        if whole {
            if let Ok(n) = text.parse::<u64>() {
                return Ok(Value::Number(n.into()));
            }
            // `-0` is the double -0, as serde_json reads it; an integer past 64 bits a double.
            if let Ok(n) = text.parse::<i64>()
                && n != 0
            {
                return Ok(Value::Number(n.into()));
            }
        }
        let x: f64 = text.parse().map_err(|_| self.at("a number that is none"))?;
        Number::from_f64(x).map(Value::Number).ok_or_else(|| self.at("a number out of range"))
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    /// Every double read as its shortest text says, where serde_json's default reading is a unit
    /// off for about one in seven: a hundred thousand of them, scaled readings and roots.
    #[test]
    fn every_number_reads_as_its_text_says() {
        for i in 0..100_000u32 {
            for x in [f64::from(i) * 0.1, 20.0 + f64::from(i) * 0.01, f64::from(i).sqrt(), -f64::from(i) * 0.3, f64::from(i) * 1e-300, f64::from(i) * 1e300] {
                if !x.is_finite() {
                    continue;
                }
                let text = serde_json::to_string(&x).unwrap();
                let read = parse(&text).unwrap().as_f64().unwrap();
                assert_eq!(read.to_bits(), x.to_bits(), "{text}");
            }
        }
        // 14 × 0.1, which serde_json's default reading takes for 1.4.
        assert_eq!(parse("1.4000000000000001").unwrap().as_f64().unwrap().to_bits(), 0x3ff6_6666_6666_6667);
        // Integers as integers, past 64 bits a double, `-0` the double -0.
        assert_eq!(parse("[0, 18446744073709551615, -9223372036854775808, 18446744073709551616, -0, 1e2, 1E-2, 0.5e+1]").unwrap(), json!([0u64, u64::MAX, i64::MIN, 18446744073709551616f64, -0.0, 100.0, 0.01, 5.0]));
        assert!(parse("-0").unwrap().as_f64().unwrap().is_sign_negative());
    }

    /// Strings, objects and arrays as JSON has them: every escape, a pair of surrogates, nesting
    /// to 128 and no deeper, white space anywhere between; and what is no JSON refused.
    #[test]
    fn the_rest_of_json_and_what_is_none() {
        assert_eq!(
            parse(" { \"a\" : [ 1 , true , false , null , \"x\\\"\\\\\\/\\b\\f\\n\\r\\t\\u00e7\\ud83d\\ude00\" ] , \"b\" : { } , \"a\" : [ ] } ").unwrap(),
            json!({ "a": [], "b": {} })
        );
        assert_eq!(parse("[\"x\\\"\\\\\\/\\b\\f\\n\\r\\t\\u00e7\\ud83d\\ude00ção\"]").unwrap(), json!(["x\"\\/\u{8}\u{c}\n\r\t\u{e7}\u{1f600}ção"]));
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&deep(128)).is_ok());
        assert!(parse(&deep(129)).unwrap_err().starts_with("nested past 128"));
        for bad in ["", "[1,]", "{\"a\" 1}", "{1:2}", "01", "1.", ".5", "1e", "-", "+1", "[1 2]", "\"a", "\"\u{1}\"", "\"\\x\"", "\"\\ud83d\"", "\"\\ude00\"", "\"\\u12\"", "\"\\u+123\"", "tru", "nul", "1e400", "[] x"] {
            assert!(parse(bad).is_err(), "{bad:?} read");
        }
    }
}
