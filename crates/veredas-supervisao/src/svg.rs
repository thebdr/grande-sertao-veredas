//! The layout asset: an SVG document read as XML, never by a pattern over its text. What the
//! binding needs of it is which elements carry which ids, and which of them a marker can follow;
//! what the page needs is the text as written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The elements a marker can follow: those whose outline the page can measure along.
pub const FOLLOWABLE: [&str; 7] = ["path", "line", "polyline", "polygon", "rect", "circle", "ellipse"];

/// A layout as read: the file, its text, and every element id with the element's name and how
/// many elements carry it.
#[derive(Clone, Debug)]
pub struct Layout {
    pub file: PathBuf,
    pub text: String,
    /// id → (the element's local name, how many elements carry the id).
    pub ids: BTreeMap<String, (String, usize)>,
}

impl Layout {
    /// The element carrying `id`, when exactly one does.
    pub fn element(&self, id: &str) -> Option<&str> {
        self.ids.get(id).filter(|(_, n)| *n == 1).map(|(name, _)| name.as_str())
    }
}

/// Read a layout file: a well-formed XML document whose root is an `svg` element, holding no
/// script, no `foreignObject` and no event-handler attribute (a layout is a drawing, and the
/// page places it in its own document).
pub fn read(path: &Path) -> Result<Layout, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse(&text, path)
}

/// Judge a layout's text, `path` naming it.
pub fn parse(text: &str, path: &Path) -> Result<Layout, String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| format!("{} is not an XML document: {e}", path.display()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(format!("{} is not an SVG document: its root is <{}>", path.display(), root.tag_name().name()));
    }
    let mut ids: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for node in root.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name == "script" || name == "foreignObject" {
            return Err(format!("{} holds a <{name}> element: a layout is a drawing, with no script and no foreign content", path.display()));
        }
        if let Some(attr) = node.attributes().find(|a| a.name().len() > 2 && a.name()[..2].eq_ignore_ascii_case("on")) {
            return Err(format!("{} holds an event handler (`{}` on <{name}>): a layout is a drawing, with no script", path.display(), attr.name()));
        }
        if let Some(id) = node.attribute("id") {
            let e = ids.entry(id.to_string()).or_insert_with(|| (name.to_string(), 0));
            e.1 += 1;
        }
    }
    Ok(Layout { file: path.to_path_buf(), text: text.to_string(), ids })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_is_an_svg_document_with_no_script() {
        let p = Path::new("l.svg");
        let ok = parse("<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"a\"><path id=\"loop\" d=\"M0 0\"/></g><rect id=\"a\"/></svg>", p).unwrap();
        assert_eq!(ok.element("loop"), Some("path"));
        assert_eq!(ok.element("a"), None, "two elements carry it");
        assert_eq!(ok.ids["a"].1, 2);
        assert_eq!(parse("<html/>", p).unwrap_err(), "l.svg is not an SVG document: its root is <html>");
        assert!(parse("<svg>", p).unwrap_err().starts_with("l.svg is not an XML document: "));
        assert_eq!(parse("<svg><script>x()</script></svg>", p).unwrap_err(), "l.svg holds a <script> element: a layout is a drawing, with no script and no foreign content");
        assert_eq!(parse("<svg><rect onclick=\"x()\"/></svg>", p).unwrap_err(), "l.svg holds an event handler (`onclick` on <rect>): a layout is a drawing, with no script");
        assert_eq!(parse("<svg><rect ONLOAD=\"x()\"/></svg>", p).unwrap_err(), "l.svg holds an event handler (`ONLOAD` on <rect>): a layout is a drawing, with no script");
        assert!(parse("<svg><rect one=\"1\"/></svg>", p).is_err(), "any on* attribute");
        assert!(parse("<svg><rect o=\"1\" on=\"1\"/></svg>", p).is_ok(), "on alone names no event");
        assert!(parse("<svg><foreignObject/></svg>", p).is_err());
    }
}
