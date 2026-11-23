//! Just enough XML for a byte-faithful patcher: find tags in a part's text,
//! read and replace attribute values in place, escape and unescape. Nothing
//! is ever parsed into a tree and re-serialised — an untouched byte stays
//! the byte Excel wrote.

use std::borrow::Cow;

/// One tag found in a text: `<name attrs>`, `<name attrs/>` or `</name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag<'a> {
    /// Byte span of the whole tag, `<` to `>` inclusive.
    pub start: usize,
    pub end: usize,
    pub name: &'a str,
    pub closing: bool,
    pub self_closing: bool,
    /// The tag's text from `<` to `>`.
    pub text: &'a str,
}

/// The next tag at or after `from` (comments, processing instructions and
/// declarations are skipped).
pub fn next_tag(s: &str, mut from: usize) -> Option<Tag<'_>> {
    let b = s.as_bytes();
    loop {
        let lt = from + s.get(from..)?.find('<')?;
        if s[lt..].starts_with("<?") || s[lt..].starts_with("<!") {
            let close = if s[lt..].starts_with("<!--") {
                "-->"
            } else {
                ">"
            };
            from = lt + s[lt..].find(close)? + close.len();
            continue;
        }
        let gt = tag_end(b, lt)?;
        let text = &s[lt..=gt];
        let closing = b.get(lt + 1) == Some(&b'/');
        let name_start = if closing { lt + 2 } else { lt + 1 };
        let mut ne = name_start;
        while ne < gt && !b[ne].is_ascii_whitespace() && b[ne] != b'/' && b[ne] != b'>' {
            ne += 1;
        }
        return Some(Tag {
            start: lt,
            end: gt + 1,
            name: &s[name_start..ne],
            closing,
            self_closing: b[gt - 1] == b'/',
            text,
        });
    }
}

/// The `>` that ends the tag starting at `lt` (quoted values may hold `>`).
fn tag_end(b: &[u8], lt: usize) -> Option<usize> {
    let mut i = lt + 1;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The first tag named `name` (opening or empty) at or after `from`.
pub fn find_open<'a>(s: &'a str, name: &str, mut from: usize) -> Option<Tag<'a>> {
    loop {
        let t = next_tag(s, from)?;
        if !t.closing && t.name == name {
            return Some(t);
        }
        from = t.end;
    }
}

/// The closing tag of the element opened by `open` (same name, nesting
/// counted); for a self-closing tag, the tag itself.
pub fn find_close<'a>(s: &'a str, open: &Tag<'_>) -> Option<Tag<'a>> {
    if open.self_closing {
        return next_tag(s, open.start);
    }
    let mut depth = 1;
    let mut from = open.end;
    loop {
        let t = next_tag(s, from)?;
        if t.name == open.name {
            if t.closing {
                depth -= 1;
                if depth == 0 {
                    return Some(t);
                }
            } else if !t.self_closing {
                depth += 1;
            }
        }
        from = t.end;
    }
}

/// The span of the whole element opened by `open`, `<` of the open tag to
/// `>` of the close.
pub fn element_span(s: &str, open: &Tag<'_>) -> Option<(usize, usize)> {
    let close = find_close(s, open)?;
    Some((open.start, close.end))
}

/// The raw (still escaped) value of attribute `name` in a tag's text.
pub fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let (vs, ve) = attr_span(tag, name)?;
    Some(&tag[vs..ve])
}

/// Byte span of attribute `name`'s value inside `tag`.
fn attr_span(tag: &str, name: &str) -> Option<(usize, usize)> {
    let b = tag.as_bytes();
    let mut i = 1;
    // skip the element name
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' || b[i] == b'/' {
            return None;
        }
        let ns = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
        }
        let key = &tag[ns..i];
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'=') {
            i += 1;
        }
        let q = *b.get(i)?;
        if q != b'"' && q != b'\'' {
            return None;
        }
        let vs = i + 1;
        let ve = vs + tag[vs..].find(q as char)?;
        if key == name {
            return Some((vs, ve));
        }
        i = ve + 1;
    }
}

/// The tag with attribute `name` set to `value` (already escaped): replaced
/// in place when present, appended before the end otherwise.
pub fn set_attr(tag: &str, name: &str, value: &str) -> String {
    if let Some((vs, ve)) = attr_span(tag, name) {
        return format!("{}{}{}", &tag[..vs], value, &tag[ve..]);
    }
    let cut = if tag.ends_with("/>") {
        tag.len() - 2
    } else {
        tag.len() - 1
    };
    format!("{} {}=\"{}\"{}", &tag[..cut], name, value, &tag[cut..])
}

/// The tag without attribute `name`.
pub fn remove_attr(tag: &str, name: &str) -> String {
    let Some((vs, ve)) = attr_span(tag, name) else {
        return tag.to_string();
    };
    // back up over `name="` and the whitespace before it
    let b = tag.as_bytes();
    let mut s = vs - 1; // the opening quote
    while s > 0 && b[s - 1] != b'=' {
        s -= 1;
    }
    s -= 1; // '='
    while s > 0 && !b[s - 1].is_ascii_whitespace() {
        s -= 1;
    }
    while s > 0 && b[s - 1].is_ascii_whitespace() {
        s -= 1;
    }
    format!("{}{}", &tag[..s], &tag[ve + 1..])
}

/// XML text → string: the XML 1.0 end-of-line normalisation first (a raw
/// CR LF or lone CR in the source reads as LF, §2.11 — what Excel and every
/// XML parser see), then the entities (`&#13;` stays a real CR).
pub fn unescape(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        let n = s.replace("\r\n", "\n").replace('\r', "\n");
        return Cow::Owned(unescape(&n).into_owned());
    }
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';') else {
            out.push_str(rest);
            return Cow::Owned(out);
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=semi]),
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// String → XML text (element content). Control characters Excel cannot
/// carry raw are written as `_xHHHH_`, the OOXML escape Excel reads back.
pub fn escape_text(s: &str) -> String {
    escape(s, false)
}

/// [`escape_text`] for element text (`attr` false) or a double-quoted attribute value (`attr`
/// true): one pass, so an escape written never meets a later replacement.
fn escape(s: &str, attr: bool) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 8);
    for (i, c) in s.char_indices() {
        match c {
            // a literal "_xHHHH_" would read back as the character it names: its underscore is
            // escaped itself ("_x005F_")
            '_' if is_escape_at(b, i) => out.push_str("_x005F_"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            // an attribute's tab, line feed and carriage return read back as spaces unless they
            // are character references
            '\t' if attr => out.push_str("&#9;"),
            '\n' if attr => out.push_str("&#10;"),
            '\r' if attr => out.push_str("&#13;"),
            // XML reads a raw carriage return as a line feed: in text, written as Excel writes it
            '\r' => out.push_str("_x000D_"),
            '\t' | '\n' => out.push(c),
            // what XML cannot carry: control characters, and the two non-characters U+FFFE and U+FFFF
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push_str(&format!("_x{:04X}_", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn is_escape_at(b: &[u8], i: usize) -> bool {
    b.len() >= i + 7
        && b[i] == b'_'
        && b[i + 1] == b'x'
        && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit)
        && b[i + 6] == b'_'
}

/// Element text as Excel means it: XML entities, then the OOXML
/// "_xHHHH_" escapes.
pub fn decode_text(s: &str) -> String {
    let u = unescape(s);
    if !u.contains("_x") {
        return u.into_owned();
    }
    let b = u.as_bytes();
    let mut out = String::with_capacity(u.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if is_escape_at(b, i)
            && let Some(c) = u32::from_str_radix(&u[i + 2..i + 6], 16)
                .ok()
                .and_then(char::from_u32)
            {
                out.push_str(&u[last..i]);
                out.push(c);
                i += 7;
                last = i;
                continue;
            }
        i += 1;
    }
    out.push_str(&u[last..]);
    out
}

/// String → attribute value (double-quoted).
pub fn escape_attr(s: &str) -> String {
    escape(s, true)
}

/// The inner text of the first `<name …>…</name>` in `s` (raw, escaped).
pub fn inner<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = find_open(s, name, 0)?;
    if open.self_closing {
        return Some("");
    }
    let close = find_close(s, &open)?;
    Some(&s[open.end..close.start])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ends_normalise_as_xml_says_but_char_refs_stay() {
        // a raw CR LF / lone CR in the source is LF to every XML reader
        // (Excel included); &#13; and _x000D_ are real CRs
        assert_eq!(decode_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(decode_text("a&#13;\nb"), "a\r\nb");
        assert_eq!(decode_text("a_x000D_\r\nb &amp; c"), "a\r\nb & c");
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn tags_attributes_in_place() {
        let s = r#"<?xml version="1.0"?><!-- x --><row r="5" spans="1:3" x14ac:dyDescent="0.2"><c r="A5" s="2"/></row>"#;
        let t = next_tag(s, 0).unwrap();
        assert_eq!((t.name, t.closing, t.self_closing), ("row", false, false));
        assert_eq!(attr(t.text, "spans"), Some("1:3"));
        assert_eq!(attr(t.text, "s"), None);
        assert_eq!(
            set_attr(t.text, "r", "6"),
            r#"<row r="6" spans="1:3" x14ac:dyDescent="0.2">"#
        );
        assert_eq!(
            remove_attr(t.text, "spans"),
            r#"<row r="5" x14ac:dyDescent="0.2">"#
        );
        let c = next_tag(s, t.end).unwrap();
        assert!(c.self_closing);
        assert_eq!(
            set_attr(c.text, "t", "inlineStr"),
            r#"<c r="A5" s="2" t="inlineStr"/>"#
        );
        assert_eq!(find_close(s, &t).unwrap().name, "row");
    }

    #[test]
    fn nesting_and_quoted_gt() {
        let s = r#"<a k="x>y"><a></a><b/></a><c/>"#;
        let a = next_tag(s, 0).unwrap();
        assert_eq!(attr(a.text, "k"), Some("x>y"));
        let (st, en) = element_span(s, &a).unwrap();
        assert_eq!(&s[st..en], r#"<a k="x>y"><a></a><b/></a>"#);
    }

    #[test]
    fn escaping_round_trips() {
        let raw = "a<b & \"c\" \u{1}";
        let esc = escape_attr(raw);
        assert_eq!(esc, "a&lt;b &amp; &quot;c&quot; _x0001_");
        assert_eq!(unescape("x &amp; &#233;&#xE9; &lt;"), "x & éé <");
        assert_eq!(escape_text("_x0041_"), "_x005F_x0041_");
        assert_eq!(decode_text("a_x000D_b"), "a\rb");
        assert_eq!(decode_text(&escape_text("_x0041_ \u{2}")), "_x0041_ \u{2}");
        // a carriage return in text is Excel's escape, in an attribute a character reference; a
        // literal `_x000D_` keeps its underscore escaped either way
        assert_eq!(escape_text("a\rb _x000D_"), "a_x000D_b _x005F_x000D_");
        assert_eq!(decode_text(&escape_text("a\rb _x000D_")), "a\rb _x000D_");
        assert_eq!(escape_attr("a\r\n\tb\" _x000D_"), "a&#13;&#10;&#9;b&quot; _x005F_x000D_");
        assert_eq!(unescape(&escape_attr("a\r\n\tb\"")), "a\r\n\tb\"");
        // a literal escape of a control character stays literal too
        assert_eq!(escape_text("_x0001_\u{1}"), "_x005F_x0001__x0001_");
        assert_eq!(decode_text(&escape_text("_x0001_\u{1}\u{FFFF}")), "_x0001_\u{1}\u{FFFF}");
        assert_eq!(inner("<v xml:space=\"preserve\"> a </v>", "v"), Some(" a "));
    }
}
